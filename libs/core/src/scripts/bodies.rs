//! Bodies for scripts (ADR 07, change 4): the class of a body from its
//! Content-Type, decoding that stops at a limit, the one memory budget for
//! every held body, copy and queued event, and files for saved bodies.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

/// Held for an intercept script: 8 MB after decoding.
pub const HOLD_LIMIT: usize = 8 << 20;
/// Copied for a log script: 16 MB after decoding.
pub const COPY_LIMIT: usize = 16 << 20;
/// All held bodies, copies and queued events together: 512 MB.
pub const BUDGET: usize = 512 << 20;
/// One event given to `on_event`: 1 MB.
pub const EVENT_LIMIT: usize = 1 << 20;
/// A log rule's queue: 1,000 items or 256 MB.
pub const QUEUE_ITEMS: usize = 1000;
pub const QUEUE_BYTES: usize = 256 << 20;
/// The writer queue of one saved body: 4 MB. ADR 07 planned 256 KB; on
/// loopback that fills before the writer thread starts, so a local dev
/// server's images were cut as "slow_disk" (see the ADR's actual manifest).
pub const SAVE_QUEUE: usize = 4 << 20;

/// The class of a body (ADR 07, change 4, section 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyClass {
    Text,
    Events,
    Media,
    Multipart,
    Binary,
    None,
}

impl BodyClass {
    pub const ALL: [BodyClass; 5] = [BodyClass::Text, BodyClass::Events, BodyClass::Media, BodyClass::Multipart, BodyClass::Binary];

    pub fn as_str(self) -> &'static str {
        match self {
            BodyClass::Text => "text",
            BodyClass::Events => "events",
            BodyClass::Media => "media",
            BodyClass::Multipart => "multipart",
            BodyClass::Binary => "binary",
            BodyClass::None => "none",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == name)
    }

    /// The class from the Content-Type header alone (I26). The same header
    /// always gives the same class; the bytes are never looked at.
    pub fn of(content_type: Option<&str>) -> Self {
        let Some(ct) = content_type else { return BodyClass::Binary };
        let essence = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        let (top, sub) = essence.split_once('/').unwrap_or((essence.as_str(), ""));
        if crate::scripts::events::Format::of(Some(&essence)).is_some() {
            return BodyClass::Events;
        }
        match top {
            "text" => BodyClass::Text,
            "image" | "audio" | "video" | "font" => BodyClass::Media,
            "multipart" => BodyClass::Multipart,
            "application" => match sub {
                "json" | "xml" | "javascript" | "x-www-form-urlencoded" | "graphql" | "ecmascript" | "x-javascript" => {
                    BodyClass::Text
                }
                s if s.ends_with("+json") || s.ends_with("+xml") => BodyClass::Text,
                _ => BodyClass::Binary,
            },
            _ => BodyClass::Binary,
        }
    }
}

/// A set of classes, as a script lists them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Classes(u8);

impl Classes {
    pub const NONE: Classes = Classes(0);

    pub fn of(list: &[BodyClass]) -> Self {
        Classes(list.iter().fold(0, |acc, c| acc | Self::bit(*c)))
    }

    fn bit(c: BodyClass) -> u8 {
        match c {
            BodyClass::Text => 1,
            BodyClass::Events => 2,
            BodyClass::Media => 4,
            BodyClass::Multipart => 8,
            BodyClass::Binary => 16,
            BodyClass::None => 0,
        }
    }

    pub fn contains(self, c: BodyClass) -> bool {
        self.0 & Self::bit(c) != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn union(self, other: Classes) -> Classes {
        Classes(self.0 | other.0)
    }

    pub fn overlaps(self, other: Classes) -> bool {
        self.0 & other.0 != 0
    }

    pub fn names(self) -> Vec<&'static str> {
        BodyClass::ALL.into_iter().filter(|c| self.contains(*c)).map(BodyClass::as_str).collect()
    }
}

/// The global memory budget (I17): a counter of reserved bytes.
#[derive(Debug)]
pub struct Budget {
    used: AtomicUsize,
    limit: usize,
}

impl Budget {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self { used: AtomicUsize::new(0), limit })
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// Reserve `n` bytes, or nothing when that would pass the limit.
    pub fn try_reserve(self: &Arc<Self>, n: usize) -> Option<Reservation> {
        let mut cur = self.used.load(Ordering::Relaxed);
        loop {
            let next = cur.checked_add(n).filter(|&v| v <= self.limit)?;
            match self.used.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => return Some(Reservation { budget: self.clone(), bytes: n }),
                Err(actual) => cur = actual,
            }
        }
    }
}

/// Bytes reserved in the budget, given back when dropped.
#[derive(Debug)]
pub struct Reservation {
    budget: Arc<Budget>,
    bytes: usize,
}

impl Reservation {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Reserve `n` more bytes in the same reservation.
    pub fn grow(&mut self, n: usize) -> bool {
        match self.budget.try_reserve(n) {
            Some(more) => {
                self.bytes += more.bytes;
                std::mem::forget(more);
                true
            }
            None => false,
        }
    }

    /// Merge another reservation into this one.
    pub fn absorb(&mut self, other: Reservation) {
        self.bytes += other.bytes;
        std::mem::forget(other);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// A sink that keeps up to `limit` bytes and reports when it is full.
pub struct LimitedVec {
    pub data: Vec<u8>,
    limit: usize,
    pub truncated: bool,
}

impl LimitedVec {
    pub fn new(limit: usize) -> Self {
        Self { data: Vec::new(), limit, truncated: false }
    }
}

impl Write for LimitedVec {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = self.limit - self.data.len();
        if buf.len() > room {
            self.data.extend_from_slice(&buf[..room]);
            self.truncated = true;
            return Err(io::Error::new(io::ErrorKind::StorageFull, "limit"));
        }
        self.data.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A `Content-Encoding` the daemon decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Identity,
    Gzip,
    Deflate,
    Brotli,
    Zstd,
    /// Not known: the raw bytes are given, with `body_encoding` set.
    Unknown,
}

impl Encoding {
    pub fn of(header: Option<&str>) -> Self {
        let Some(h) = header else { return Encoding::Identity };
        // Several encodings in a row are rare; only one is decoded.
        let parts: Vec<String> = h.split(',').map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()).collect();
        match parts.as_slice() {
            [] => Encoding::Identity,
            [one] => match one.as_str() {
                "identity" => Encoding::Identity,
                "gzip" | "x-gzip" => Encoding::Gzip,
                "deflate" => Encoding::Deflate,
                "br" => Encoding::Brotli,
                "zstd" => Encoding::Zstd,
                _ => Encoding::Unknown,
            },
            _ => Encoding::Unknown,
        }
    }
}

/// A streaming decoder that writes into `W`. Writing past what `W` accepts
/// fails, so decoding stops at the limit of its use (I25).
pub enum Decoder<W: Write> {
    Plain(W),
    Gzip(flate2::write::MultiGzDecoder<W>),
    Deflate(flate2::write::ZlibDecoder<W>),
    Brotli(Box<brotli::DecompressorWriter<W>>),
    Zstd(Box<zstd::stream::write::Decoder<'static, W>>),
}

impl<W: Write> Decoder<W> {
    pub fn new(encoding: Encoding, sink: W) -> io::Result<Self> {
        Ok(match encoding {
            Encoding::Identity | Encoding::Unknown => Decoder::Plain(sink),
            Encoding::Gzip => Decoder::Gzip(flate2::write::MultiGzDecoder::new(sink)),
            Encoding::Deflate => Decoder::Deflate(flate2::write::ZlibDecoder::new(sink)),
            Encoding::Brotli => Decoder::Brotli(Box::new(brotli::DecompressorWriter::new(sink, 8192))),
            Encoding::Zstd => Decoder::Zstd(Box::new(zstd::stream::write::Decoder::new(sink)?)),
        })
    }

    pub fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            Decoder::Plain(w) => w.write_all(data),
            Decoder::Gzip(d) => d.write_all(data),
            Decoder::Deflate(d) => d.write_all(data),
            Decoder::Brotli(d) => d.write_all(data),
            Decoder::Zstd(d) => d.write_all(data),
        }
    }

    /// Flush what is left and give the sink back.
    pub fn finish(self) -> io::Result<W> {
        match self {
            Decoder::Plain(w) => Ok(w),
            Decoder::Gzip(d) => d.finish(),
            Decoder::Deflate(d) => d.finish(),
            Decoder::Brotli(d) => d.into_inner().map_err(|_| io::Error::other("the brotli body ended early")),
            Decoder::Zstd(mut d) => {
                d.flush()?;
                Ok(d.into_inner())
            }
        }
    }

    pub fn sink_mut(&mut self) -> &mut W {
        match self {
            Decoder::Plain(w) => w,
            Decoder::Gzip(d) => d.get_mut(),
            Decoder::Deflate(d) => d.get_mut(),
            Decoder::Brotli(d) => d.get_mut(),
            Decoder::Zstd(d) => d.get_mut(),
        }
    }

    pub fn sink(&self) -> &W {
        match self {
            Decoder::Plain(w) => w,
            Decoder::Gzip(d) => d.get_ref(),
            Decoder::Deflate(d) => d.get_ref(),
            Decoder::Brotli(d) => d.get_ref(),
            Decoder::Zstd(d) => d.get_ref(),
        }
    }
}

/// Decode a whole body up to `limit` bytes. Returns the bytes and whether
/// they were cut at the limit. A body that does not decode is an error.
pub fn decode_all(encoding: Encoding, data: &[u8], limit: usize) -> Result<(Vec<u8>, bool), String> {
    let mut d = Decoder::new(encoding, LimitedVec::new(limit)).map_err(|e| e.to_string())?;
    match d.write_all(data) {
        Ok(()) => {}
        // Full: keep what fits, without finishing the decoder (that would
        // write again).
        Err(_) if d.sink().truncated => return Ok((std::mem::take(&mut d.sink_mut().data), true)),
        Err(e) => return Err(format!("the body does not decode: {e}")),
    }
    match d.finish() {
        Ok(v) => {
            let cut = v.truncated;
            Ok((v.data, cut))
        }
        Err(e) => Err(format!("the body does not decode: {e}")),
    }
}

/// A file extension for a saved body, from its Content-Type.
pub fn extension(content_type: Option<&str>, class: BodyClass) -> &'static str {
    let essence = content_type.and_then(|c| c.split(';').next()).map(|c| c.trim().to_ascii_lowercase()).unwrap_or_default();
    match essence.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "image/avif" => "avif",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "font/woff2" => "woff2",
        "font/woff" => "woff",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/json" => "json",
        "text/html" => "html",
        "text/plain" => "txt",
        "text/css" => "css",
        _ if class == BodyClass::Multipart => "multipart",
        _ => "bin",
    }
}

/// Create `output_dir/bodies/` (mode 0700) when missing and refuse a
/// `bodies` that is a link, so a saved body never lands elsewhere.
pub fn bodies_dir(output_dir: &Path) -> io::Result<PathBuf> {
    let dir = output_dir.join("bodies");
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "bodies/ is a symbolic link"));
        }
        Ok(m) if !m.is_dir() => return Err(io::Error::new(io::ErrorKind::AlreadyExists, "bodies is not a folder")),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        }
        Err(e) => return Err(e),
    }
    Ok(dir)
}

/// Open a new file with mode 0600, never through a link (`O_NOFOLLOW`).
pub fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)
}

/// Open a file to append, creating it with mode 0600, never through a link.
pub fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().append(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // T20
    #[test]
    fn class_comes_from_the_content_type_only() {
        for (ct, class) in [
            (Some("application/json"), BodyClass::Text),
            (Some("application/problem+json; charset=utf-8"), BodyClass::Text),
            (Some("text/html"), BodyClass::Text),
            (Some("application/x-www-form-urlencoded"), BodyClass::Text),
            (Some("text/event-stream"), BodyClass::Events),
            (Some("application/x-ndjson"), BodyClass::Events),
            (Some("image/png"), BodyClass::Media),
            (Some("video/mp4"), BodyClass::Media),
            (Some("font/woff2"), BodyClass::Media),
            (Some("multipart/form-data; boundary=x"), BodyClass::Multipart),
            (Some("multipart/byteranges; boundary=x"), BodyClass::Multipart),
            (Some("application/octet-stream"), BodyClass::Binary),
            (Some("application/pdf"), BodyClass::Binary),
            (Some("application/grpc"), BodyClass::Binary),
            (None, BodyClass::Binary),
        ] {
            assert_eq!(BodyClass::of(ct), class, "{ct:?}");
        }
    }

    #[test]
    fn class_sets() {
        let s = Classes::of(&[BodyClass::Text, BodyClass::Media]);
        assert!(s.contains(BodyClass::Text) && s.contains(BodyClass::Media) && !s.contains(BodyClass::Binary));
        assert_eq!(s.names(), ["text", "media"]);
        assert!(s.overlaps(Classes::of(&[BodyClass::Media])));
        assert!(Classes::NONE.is_empty());
    }

    // I17
    #[test]
    fn the_budget_refuses_past_its_limit_and_gets_bytes_back() {
        let b = Budget::new(100);
        let a = b.try_reserve(60).unwrap();
        assert!(b.try_reserve(50).is_none());
        let mut c = b.try_reserve(40).unwrap();
        assert!(!c.grow(1));
        drop(a);
        assert!(c.grow(10));
        assert_eq!(b.used(), 50);
        drop(c);
        assert_eq!(b.used(), 0);
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    // T24, I25: a small gzip body that expands far stops at the limit.
    #[test]
    fn decoding_stops_at_the_limit() {
        let bomb = gzip(&vec![0u8; 50 << 20]);
        assert!(bomb.len() < 100 << 10, "{}", bomb.len());
        let (data, cut) = decode_all(Encoding::Gzip, &bomb, 1 << 20).unwrap();
        assert_eq!((data.len(), cut), (1 << 20, true));
        let (data, cut) = decode_all(Encoding::Gzip, &gzip(b"hello"), 100).unwrap();
        assert_eq!((data.as_slice(), cut), (&b"hello"[..], false));
        assert!(decode_all(Encoding::Gzip, b"not gzip", 100).is_err());
    }

    #[test]
    fn every_known_encoding_decodes() {
        let text = b"the same text in every encoding".repeat(20);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&text).unwrap();
        let deflate = z.finish().unwrap();
        let mut br = Vec::new();
        brotli::BrotliCompress(&mut &text[..], &mut br, &Default::default()).unwrap();
        let zstd = zstd::encode_all(&text[..], 3).unwrap();
        for (enc, data) in [(Encoding::Gzip, gzip(&text)), (Encoding::Deflate, deflate), (Encoding::Brotli, br), (Encoding::Zstd, zstd), (Encoding::Identity, text.clone())] {
            assert_eq!(decode_all(enc, &data, 1 << 20).unwrap().0, text, "{enc:?}");
        }
        assert_eq!(Encoding::of(Some("GZIP")), Encoding::Gzip);
        assert_eq!(Encoding::of(Some("gzip, br")), Encoding::Unknown);
        assert_eq!(Encoding::of(Some("compress")), Encoding::Unknown);
    }

    #[test]
    fn extensions_come_from_the_content_type() {
        assert_eq!(extension(Some("image/png"), BodyClass::Media), "png");
        assert_eq!(extension(Some("video/mp4; codecs=x"), BodyClass::Media), "mp4");
        assert_eq!(extension(Some("multipart/form-data; boundary=x"), BodyClass::Multipart), "multipart");
        assert_eq!(extension(None, BodyClass::Binary), "bin");
    }

    // T22: a bodies/ that is a link is refused; files are 0600 and new.
    #[test]
    fn saved_files_never_follow_links() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("bodies")).unwrap();
        assert!(bodies_dir(dir.path()).is_err());
        std::fs::remove_file(dir.path().join("bodies")).unwrap();
        let bodies = bodies_dir(dir.path()).unwrap();
        assert_eq!(std::fs::metadata(&bodies).unwrap().permissions().mode() & 0o777, 0o700);
        let f = bodies.join("a.bin");
        create_new(&f).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(create_new(&f).is_err(), "never replaces a file");
        std::os::unix::fs::symlink(elsewhere.path().join("x"), bodies.join("link")).unwrap();
        assert!(open_append(&bodies.join("link")).is_err());
        assert!(!elsewhere.path().join("x").exists());
    }
}
