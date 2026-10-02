//! The viewer at `proxy.localhost` and `router.localhost/proxy-log/` (ADR 08,
//! change 2).
//!
//! Everything comes from the daemon binary: the page, its script and its
//! style are compiled in, and the data paths read the folder at each request.
//! Three rules: loopback peers only (I8), `GET` and `HEAD` only, no CORS
//! headers (I10); only our HAR files are served, never a link (I9). Nothing
//! is kept between requests, and no request holds a whole file (I22).

use std::convert::Infallible;
use std::error::Error;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::net::IpAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Frame;
use hyper::header::{self, HeaderValue};
use hyper::{Method, Response, StatusCode, Uri};
use tokio::io::AsyncReadExt;
use tokio::sync::{broadcast, mpsc};

use super::{CLOSING, HarLog, KEEP_FILES, LiveEvent, file_key};
use crate::proxy::Body;

/// Files and `/api/entries` are read in chunks of this size.
pub const CHUNK: usize = 64 * 1024;
/// Entries `/api/entries` returns at most.
pub const MAX_ENTRIES: usize = 2000;
/// A comment line on `/api/live` this often keeps the connection open.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

const HTML: &str = include_str!("viewer.html");
const JS: &str = include_str!("viewer.js");
const CSS: &str = include_str!("viewer.css");

const CSP: &str = "default-src 'self'; img-src 'self' data:; frame-ancestors 'none'";

/// What the page shows besides the log: the CLI name, for "how to turn it on".
pub struct ViewerContext<'a> {
    pub cli: &'a str,
}

/// Answer one request to the viewer. `path` is relative to the viewer: `/`,
/// `/api/files`, … (the router strips `/proxy-log` for the second address).
pub async fn serve(log: &Arc<HarLog>, ctx: ViewerContext<'_>, method: &Method, uri: &Uri, peer: IpAddr) -> Response<Body> {
    // This Mac only, also with allow_lan on (I8).
    if !peer.to_canonical().is_loopback() {
        return text(StatusCode::FORBIDDEN, "The proxy log is for this Mac only.\n");
    }
    // Read only (I10).
    if method != Method::GET && method != Method::HEAD {
        let mut r = text(StatusCode::METHOD_NOT_ALLOWED, "The proxy log is read only.\n");
        r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        return r;
    }
    let path = uri.path();
    let query = uri.query().unwrap_or("");
    match path {
        "/" | "/index.html" => fixed(HTML, "text/html; charset=utf-8"),
        "/viewer.js" => fixed(JS, "text/javascript; charset=utf-8"),
        "/viewer.css" => fixed(CSS, "text/css; charset=utf-8"),
        "/api/files" => json(files_json(log, &ctx)),
        "/api/entries" => entries(log, query).await,
        "/api/live" => live(log),
        _ => match path.strip_prefix("/files/") {
            Some(name) => file(log, name, query.split('&').any(|p| p == "download=1")).await,
            None => text(StatusCode::NOT_FOUND, "Not found.\n"),
        },
    }
}

/// The security headers every answer carries.
fn secure(mut r: Response<Body>) -> Response<Body> {
    let h = r.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    r
}

fn full(status: StatusCode, content_type: &'static str, body: Bytes) -> Response<Body> {
    let r = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Full::new(body).map_err(|never: Infallible| match never {}).boxed_unsync())
        .expect("static response parts are valid");
    secure(r)
}

fn text(status: StatusCode, body: &'static str) -> Response<Body> {
    full(status, "text/plain; charset=utf-8", Bytes::from_static(body.as_bytes()))
}

/// The page files are read-only data in the binary: no heap (I22).
fn fixed(body: &'static str, content_type: &'static str) -> Response<Body> {
    full(StatusCode::OK, content_type, Bytes::from_static(body.as_bytes()))
}

fn json(value: serde_json::Value) -> Response<Body> {
    full(StatusCode::OK, "application/json", Bytes::from(value.to_string()))
}

fn files_json(log: &HarLog, ctx: &ViewerContext<'_>) -> serde_json::Value {
    let (file_mb, file_requests) = log.limits();
    serde_json::json!({
        "proxy": log.proxy_on(),
        "log": log.is_on(),
        "folder": log.folder().display().to_string(),
        "file_mb": file_mb,
        "file_requests": file_requests,
        "keep_files": KEEP_FILES,
        "written": log.written(),
        "dropped": log.dropped(),
        "error": log.error(),
        "cli": ctx.cli,
        "files": log.files(),
    })
}

/// A name the viewer may serve: ours, a regular file directly in the
/// folder, not a link (I9).
fn checked(log: &HarLog, name: &str) -> Option<PathBuf> {
    file_key(name)?;
    let path = log.folder().join(name);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    meta.is_file().then_some(path)
}

/// The bytes of a file to serve, and whether the closing line is added. The
/// current file is cut before its closing line, read under the lock, so a
/// write in progress is never seen (I3).
fn readable(log: &HarLog, name: &str, path: &std::path::Path) -> Option<(u64, bool)> {
    match log.current_snapshot() {
        Some((current, len)) if current == name => Some((len, true)),
        _ => Some((std::fs::metadata(path).ok()?.len(), false)),
    }
}

/// `/files/<name>`: the HAR file, streamed in chunks (I22).
async fn file(log: &HarLog, name: &str, download: bool) -> Response<Body> {
    let Some(path) = checked(log, name) else { return text(StatusCode::NOT_FOUND, "No such proxy log file.\n") };
    let Some((len, closing)) = readable(log, name, &path) else {
        return text(StatusCode::NOT_FOUND, "No such proxy log file.\n");
    };
    let mut f = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return text(StatusCode::NOT_FOUND, "No such proxy log file.\n"),
    };
    let total = len + if closing { CLOSING.len() as u64 } else { 0 };
    let (tx, rx) = mpsc::channel::<Result<Bytes, Box<dyn Error + Send + Sync>>>(2);
    tokio::spawn(async move {
        let mut left = len;
        let mut buf = vec![0u8; CHUNK];
        while left > 0 {
            let want = (left as usize).min(CHUNK);
            match f.read(&mut buf[..want]).await {
                Ok(0) => {
                    let _ = tx.send(Err("the file got shorter while it was read".into())).await;
                    return;
                }
                Ok(n) => {
                    left -= n as u64;
                    if tx.send(Ok(Bytes::copy_from_slice(&buf[..n]))).await.is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.into())).await;
                    return;
                }
            }
        }
        if closing {
            let _ = tx.send(Ok(Bytes::from_static(CLOSING))).await;
        }
    });
    let mut r = Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, total)
        .body(ChannelBody(rx).boxed_unsync())
        .expect("static response parts are valid");
    if download {
        let value = format!("attachment; filename=\"{name}\"");
        if let Ok(v) = HeaderValue::from_str(&value) {
            r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    secure(r)
}

/// `/api/entries?file=<name>&before=<offset>&limit=<n>`: the newest entries
/// before byte `before`, newest first, read backwards from the end in chunks
/// (I22). `before` in the answer is the cursor for the next page, `null` at
/// the start of the file.
async fn entries(log: &HarLog, query: &str) -> Response<Body> {
    let mut name = None;
    let mut before = None;
    let mut limit = 500usize;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("file", v)) => name = Some(v.to_string()),
            Some(("before", v)) => before = v.parse::<u64>().ok(),
            Some(("limit", v)) => limit = v.parse::<usize>().unwrap_or(500).clamp(1, MAX_ENTRIES),
            _ => {}
        }
    }
    let Some(name) = name else { return text(StatusCode::BAD_REQUEST, "Give file=<name>.\n") };
    let Some(path) = checked(log, &name) else { return text(StatusCode::NOT_FOUND, "No such proxy log file.\n") };
    let Some((len, current)) = readable(log, &name, &path) else {
        return text(StatusCode::NOT_FOUND, "No such proxy log file.\n");
    };
    let end = before.map_or(len, |b| b.min(len));
    let read = tokio::task::spawn_blocking(move || read_backwards(&path, end, limit)).await;
    let Ok(Ok((lines, next))) = read else { return text(StatusCode::NOT_FOUND, "The file could not be read.\n") };
    // The lines are entries exactly as written: no parse, no copy into a tree.
    let mut out = Vec::with_capacity(lines.iter().map(|l| l.len() + 1).sum::<usize>() + 128);
    out.extend_from_slice(b"{\"file\":");
    out.extend_from_slice(serde_json::to_string(&name).unwrap_or_default().as_bytes());
    out.extend_from_slice(format!(",\"current\":{current},\"before\":").as_bytes());
    out.extend_from_slice(next.map_or("null".to_string(), |n| n.to_string()).as_bytes());
    out.extend_from_slice(b",\"entries\":[");
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.extend_from_slice(line);
    }
    out.extend_from_slice(b"]}");
    full(StatusCode::OK, "application/json", Bytes::from(out))
}

/// Up to `limit` entry lines that end before byte `end`, newest first, and
/// the offset where the oldest of them starts (`None` once the header is
/// reached). Memory: one chunk, one partial line and the lines returned.
pub fn read_backwards(path: &std::path::Path, end: u64, limit: usize) -> std::io::Result<(Vec<Vec<u8>>, Option<u64>)> {
    let mut f = File::open(path)?;
    let mut lines: Vec<Vec<u8>> = vec![];
    let mut carry: Vec<u8> = vec![];
    let mut pos = end;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let start = pos.saturating_sub(CHUNK as u64);
        let n = (pos - start) as usize;
        f.seek(SeekFrom::Start(start))?;
        f.read_exact(&mut buf[..n])?;
        let mut chunk = buf[..n].to_vec();
        chunk.extend_from_slice(&carry);
        carry.clear();
        // Complete lines are those after a newline in this chunk; the part
        // before the first newline may continue in the previous chunk.
        let mut line_end = chunk.len();
        let mut i = chunk.len();
        while i > 0 {
            i -= 1;
            if chunk[i] != b'\n' {
                continue;
            }
            let line = &chunk[i + 1..line_end];
            let offset = start + i as u64 + 1;
            if let Some(done) = take_line(line, offset, &mut lines, limit) {
                return Ok((lines, done));
            }
            line_end = i;
        }
        if start == 0 {
            // The first line of the file is the header.
            return Ok((lines, None));
        }
        carry.extend_from_slice(&chunk[..line_end]);
        pos = start;
    }
}

/// One line read backwards. `Some(next)` when the walk is over.
fn take_line(line: &[u8], offset: u64, lines: &mut Vec<Vec<u8>>, limit: usize) -> Option<Option<u64>> {
    let line = line.strip_suffix(b",").unwrap_or(line);
    if line.starts_with(b"{\"log\"") {
        return Some(None);
    }
    if line.first() != Some(&b'{') {
        return None;
    }
    if serde_json::from_slice::<serde::de::IgnoredAny>(line).is_err() {
        return None;
    }
    lines.push(line.to_vec());
    (lines.len() >= limit).then_some(Some(offset))
}

/// `/api/live`: server-sent events, one per new entry, while the page is
/// open. The receiver is dropped with the connection, and the writer drops
/// the channel at its next record (I21).
fn live(log: &HarLog) -> Response<Body> {
    let mut events = log.subscribe();
    let (tx, rx) = mpsc::channel::<Result<Bytes, Box<dyn Error + Send + Sync>>>(16);
    tokio::spawn(async move {
        if tx.send(Ok(Bytes::from_static(b": live\n\n"))).await.is_err() {
            return;
        }
        loop {
            let chunk = tokio::select! {
                ev = events.recv() => match ev {
                    Ok(LiveEvent::Entry(line)) => format!("event: entry\ndata: {line}\n\n"),
                    Ok(LiveEvent::File(name)) => format!("event: file\ndata: {}\n\n", serde_json::to_string(&name).unwrap_or_default()),
                    Ok(LiveEvent::Off) => "event: off\ndata: {}\n\n".to_string(),
                    Err(broadcast::error::RecvError::Lagged(_)) => "event: lagged\ndata: {}\n\n".to_string(),
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = tokio::time::sleep(KEEP_ALIVE) => ": keep-alive\n\n".to_string(),
                _ = tx.closed() => return,
            };
            if tx.send(Ok(Bytes::from(chunk))).await.is_err() {
                return;
            }
        }
    });
    let r = Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(ChannelBody(rx).boxed_unsync())
        .expect("static response parts are valid");
    secure(r)
}

/// A body fed by a task, one chunk at a time.
struct ChannelBody(mpsc::Receiver<Result<Bytes, Box<dyn Error + Send + Sync>>>);

impl hyper::body::Body for ChannelBody {
    type Data = Bytes;
    type Error = Box<dyn Error + Send + Sync>;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        self.0.poll_recv(cx).map(|item| item.map(|r| r.map(Frame::data)))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn har(entries: usize, pad: usize) -> Vec<u8> {
        let mut out = br#"{"log":{"version":"1.2","creator":{"name":"x","version":"1"},"pages":[],"entries":["#.to_vec();
        for i in 0..entries {
            out.extend_from_slice(if i == 0 { b"\n" } else { b",\n" });
            out.extend_from_slice(format!("{{\"i\":{i},\"pad\":\"{}\"}}", "x".repeat(pad)).as_bytes());
        }
        out.extend_from_slice(CLOSING);
        out
    }

    fn index(line: &[u8]) -> u64 {
        serde_json::from_slice::<serde_json::Value>(line).unwrap()["i"].as_u64().unwrap()
    }

    // T6, I22: entries from the end, newest first, in pages, with a cursor.
    #[test]
    fn entries_are_read_from_the_end_in_pages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxy-20261002-000000.har");
        std::fs::File::create(&path).unwrap().write_all(&har(3000, 100)).unwrap();
        let len = std::fs::metadata(&path).unwrap().len();
        let (lines, next) = read_backwards(&path, len, 500).unwrap();
        assert_eq!(lines.len(), 500);
        assert_eq!(index(&lines[0]), 2999);
        assert_eq!(index(&lines[499]), 2500);
        let (lines, next) = read_backwards(&path, next.unwrap(), 2000).unwrap();
        assert_eq!((index(&lines[0]), index(&lines[1999])), (2499, 500));
        let (lines, next) = read_backwards(&path, next.unwrap(), 2000).unwrap();
        assert_eq!(lines.len(), 500);
        assert_eq!(index(&lines[499]), 0);
        assert_eq!(next, None, "the header ends the walk");
    }

    // A line longer than one chunk is read whole.
    #[test]
    fn a_line_longer_than_a_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxy-20261002-000000.har");
        std::fs::File::create(&path).unwrap().write_all(&har(4, CHUNK * 2)).unwrap();
        let len = std::fs::metadata(&path).unwrap().len();
        let (lines, next) = read_backwards(&path, len, 10).unwrap();
        assert_eq!(lines.iter().map(|l| index(l)).collect::<Vec<_>>(), vec![3, 2, 1, 0]);
        assert_eq!(next, None);
    }
}
