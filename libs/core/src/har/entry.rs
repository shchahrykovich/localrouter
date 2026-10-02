//! One HAR 1.2 entry (ADR 08, change 1, "What one entry holds").
//!
//! The network task copies what it saw into a [`HarRecord`]; the writer
//! thread turns it into JSON with [`HarRecord::to_entry`]. Bodies are copied
//! up to a limit (`capture.rs`) and decoded here, on the writer thread. The
//! URL keeps its query (U2): the record is not built from a `LogEntry`, which
//! drops it (ADR 01, I10).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use hyper::header::{self, HeaderMap};
use hyper::{StatusCode, Version};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};

use super::capture::{Copied, DECODED_LIMIT};
use super::websocket::WsMessage;
use crate::logs::ProxyMode;
use crate::scripts::bodies::{BodyClass, Decoder, Encoding, LimitedVec, Reservation};
use crate::scripts::lua_api::base64_encode;

/// Record ids: unique in one daemon run, so the viewer can match an open
/// request with its entry.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// What the proxy saw of one request: copied by the network task, boxed, and
/// freed after one write.
#[derive(Debug, Clone)]
pub struct HarRecord {
    /// `_id` in the entry.
    pub id: u64,
    /// When the proxy got the request.
    pub started: SystemTime,
    /// Milliseconds until the response headers (or until a tunnel closed).
    pub wait_ms: u64,
    pub method: String,
    /// The full URL, query included.
    pub url: String,
    pub request_version: Version,
    /// As the client sent them, before script rules.
    pub request_headers: HeaderMap,
    /// As the client got it, after script rules.
    pub status: u16,
    pub response_version: Version,
    pub response_headers: HeaderMap,
    pub mode: ProxyMode,
    /// The proxy client whose port carried the request (ADR 09); `None` for
    /// the main port. Written as `_client`.
    pub client: Option<Arc<str>>,
    /// The route that answered a `.localhost` name.
    pub route: Option<String>,
    /// Ids of the script rules that ran, and the one that failed (ADR 07).
    pub scripts: Vec<String>,
    pub script_error: Option<String>,
    /// Tunnels only.
    pub bytes_in: Option<u64>,
    pub bytes_out: Option<u64>,
    /// What passed of the bodies, when the log copied them.
    pub request_body: Option<Copied>,
    pub response_body: Option<Copied>,
    /// Milliseconds from the response headers to the end of the body, or
    /// how long a WebSocket stayed open.
    pub receive_ms: u64,
    /// The response body did not end normally.
    pub body_error: Option<String>,
    /// The messages of a WebSocket, and how many were not kept.
    pub ws_messages: Option<Vec<WsMessage>>,
    pub ws_dropped: u64,
    /// The memory the copies take, given back when the record is freed.
    pub held: Vec<Arc<Reservation>>,
}

impl HarRecord {
    /// A record with no headers: the caller fills what it knows.
    pub fn new(started: SystemTime, method: &str, url: String, mode: ProxyMode) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            started,
            wait_ms: 0,
            method: method.to_string(),
            url,
            request_version: Version::HTTP_11,
            request_headers: HeaderMap::new(),
            status: 0,
            response_version: Version::HTTP_11,
            response_headers: HeaderMap::new(),
            mode,
            client: None,
            route: None,
            scripts: vec![],
            script_error: None,
            bytes_in: None,
            bytes_out: None,
            request_body: None,
            response_body: None,
            receive_ms: 0,
            body_error: None,
            ws_messages: None,
            ws_dropped: 0,
            held: vec![],
        }
    }

    /// What the client sent: called before script rules run, and only when
    /// the log is on (I1).
    pub fn start<B>(req: &hyper::Request<B>, url: String, mode: ProxyMode) -> Self {
        let mut r = Self::new(SystemTime::now(), req.method().as_str(), url, mode);
        r.request_version = req.version();
        r.request_headers = req.headers().clone();
        r
    }

    /// What the client got: the response after script rules.
    pub fn finish<B>(mut self, resp: &hyper::Response<B>, wait_ms: u64) -> Self {
        self.status = resp.status().as_u16();
        self.response_version = resp.version();
        self.response_headers = resp.headers().clone();
        self.wait_ms = wait_ms;
        self
    }

    /// The script rules that ran, and the one that failed.
    pub fn with_scripts(mut self, scripts: &(Vec<String>, Option<String>)) -> Self {
        self.scripts = scripts.0.clone();
        self.script_error = scripts.1.clone();
        self
    }

    /// One closed tunnel: `CONNECT https://host:port`, no headers.
    pub fn tunnel(started: SystemTime, host: &str, port: u16, status: u16, wait_ms: u64, bytes: (u64, u64)) -> Self {
        let name = if host.contains(':') { format!("[{host}]") } else { host.to_string() };
        let mut r = Self::new(started, "CONNECT", format!("https://{name}:{port}"), ProxyMode::Tunnel);
        r.status = status;
        r.wait_ms = wait_ms;
        r.bytes_in = Some(bytes.0);
        r.bytes_out = Some(bytes.1);
        r
    }

    /// The HAR `entries[]` object. Every header is written as it is;
    /// `Proxy-Authorization`, a header for the proxy, is left out (I4).
    pub fn to_entry(&self) -> Value {
        let request_headers: Vec<Value> = self
            .request_headers
            .iter()
            .filter(|(name, _)| *name != header::PROXY_AUTHORIZATION)
            .map(|(name, value)| header_json(name.as_str(), value.as_bytes()))
            .collect();
        let response_headers: Vec<Value> =
            self.response_headers.iter().map(|(name, value)| header_json(name.as_str(), value.as_bytes())).collect();
        let request_size = self.request_body.as_ref().map(|c| c.size).or_else(|| content_length(&self.request_headers));
        let response_size = self.response_body.as_ref().map(|c| c.size).or_else(|| content_length(&self.response_headers));
        let mime = header_text(&self.response_headers, header::CONTENT_TYPE).unwrap_or_default();
        let redirect = self
            .response_headers
            .get(header::LOCATION)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .unwrap_or_default();
        let status_text = StatusCode::from_u16(self.status).ok().and_then(|s| s.canonical_reason()).unwrap_or("");

        let mut content = json!({ "size": response_size.unwrap_or(0), "mimeType": mime });
        if let Some(copied) = self.response_body.as_ref().filter(|c| c.size > 0) {
            body_fields(&mut content, copied, Some(&mime), header_text(&self.response_headers, header::CONTENT_ENCODING).as_deref());
        }
        let mut request = json!({
            "method": self.method,
            "url": self.url,
            "httpVersion": version(self.request_version),
            "cookies": [],
            "headers": request_headers,
            "queryString": query_string(&self.url),
            "headersSize": -1,
            "bodySize": request_size.map_or(-1, |n| n as i64),
        });
        if let Some(copied) = self.request_body.as_ref().filter(|c| c.size > 0) {
            let ctype = header_text(&self.request_headers, header::CONTENT_TYPE).unwrap_or_default();
            let mut post = json!({ "mimeType": ctype });
            body_fields(&mut post, copied, Some(&ctype), header_text(&self.request_headers, header::CONTENT_ENCODING).as_deref());
            if let Some(o) = post.as_object_mut() {
                o.remove("size");
            }
            request["postData"] = post;
        }

        let mut entry = Map::new();
        entry.insert("_id".into(), self.id.into());
        entry.insert("startedDateTime".into(), iso_ms(self.started).into());
        entry.insert("time".into(), (self.wait_ms + self.receive_ms).into());
        entry.insert("request".into(), request);
        entry.insert(
            "response".into(),
            json!({
                "status": self.status,
                "statusText": status_text,
                "httpVersion": version(self.response_version),
                "cookies": [],
                "headers": response_headers,
                "content": content,
                "redirectURL": redirect,
                "headersSize": -1,
                "bodySize": response_size.map_or(-1, |n| n as i64),
            }),
        );
        entry.insert("cache".into(), json!({}));
        entry.insert("timings".into(), json!({ "send": 0, "wait": self.wait_ms, "receive": self.receive_ms }));
        entry.insert("_mode".into(), mode(self.mode).into());
        if let Some(client) = &self.client {
            entry.insert("_client".into(), client.as_ref().into());
        }
        if let Some(messages) = &self.ws_messages {
            entry.insert("_resourceType".into(), "websocket".into());
            entry.insert("_webSocketMessages".into(), serde_json::to_value(messages).unwrap_or_default());
            if self.ws_dropped > 0 {
                entry.insert("_webSocketDropped".into(), self.ws_dropped.into());
            }
        }
        if let Some(e) = &self.body_error {
            entry.insert("_bodyError".into(), e.clone().into());
        }
        if let Some(route) = &self.route {
            entry.insert("_route".into(), route.clone().into());
        }
        if !self.scripts.is_empty() {
            entry.insert("_scripts".into(), self.scripts.clone().into());
        }
        if let Some(e) = &self.script_error {
            entry.insert("_scriptError".into(), e.clone().into());
        }
        if let Some(n) = self.bytes_in {
            entry.insert("_bytesIn".into(), n.into());
        }
        if let Some(n) = self.bytes_out {
            entry.insert("_bytesOut".into(), n.into());
        }
        Value::Object(entry)
    }
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers.get(name).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
}

/// `text`, `encoding` (`base64` for bytes that are not text), `size` after
/// decoding, and `_truncated` when only the first bytes were kept.
fn body_fields(target: &mut Value, copied: &Copied, content_type: Option<&str>, content_encoding: Option<&str>) {
    let Some(o) = target.as_object_mut() else { return };
    let (bytes, cut) = match decode(copied, Encoding::of(content_encoding)) {
        Ok(v) => v,
        Err(why) => {
            o.insert("_decodeError".into(), why.into());
            (copied.data.clone(), copied.truncated)
        }
    };
    let ct = content_type.filter(|c| !c.is_empty());
    let text_like = ct.is_none() || matches!(BodyClass::of(ct), BodyClass::Text | BodyClass::Events);
    let text = if text_like {
        match std::str::from_utf8(&bytes) {
            Ok(t) => Some(t.to_string()),
            // A cut body may end inside a character.
            Err(e) if cut && e.error_len().is_none() => Some(String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()),
            Err(_) => None,
        }
    } else {
        None
    };
    match text {
        Some(t) => {
            o.insert("text".into(), t.into());
        }
        None => {
            o.insert("text".into(), base64_encode(&bytes).into());
            o.insert("encoding".into(), "base64".into());
        }
    }
    if cut {
        o.insert("_truncated".into(), true.into());
    } else {
        o.insert("size".into(), bytes.len().into());
    }
}

/// The body without its Content-Encoding, and whether it is cut. A cut body
/// gives what decodes of its first bytes.
fn decode(copied: &Copied, encoding: Encoding) -> Result<(Vec<u8>, bool), String> {
    if matches!(encoding, Encoding::Identity | Encoding::Unknown) {
        return Ok((copied.data.clone(), copied.truncated));
    }
    let mut d = Decoder::new(encoding, LimitedVec::new(DECODED_LIMIT)).map_err(|e| e.to_string())?;
    match d.write_all(&copied.data) {
        Ok(()) => {}
        Err(_) if d.sink().truncated || copied.truncated => return Ok((std::mem::take(&mut d.sink_mut().data), true)),
        Err(e) => return Err(format!("the body does not decode: {e}")),
    }
    if copied.truncated {
        return Ok((std::mem::take(&mut d.sink_mut().data), true));
    }
    match d.finish() {
        Ok(v) => Ok((v.data, v.truncated)),
        Err(e) => Err(format!("the body does not decode: {e}")),
    }
}

fn header_json(name: &str, value: &[u8]) -> Value {
    json!({ "name": name, "value": String::from_utf8_lossy(value) })
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers.get(header::CONTENT_LENGTH)?.to_str().ok()?.trim().parse().ok()
}

fn version(v: Version) -> &'static str {
    match v {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_2 => "HTTP/2.0",
        Version::HTTP_3 => "HTTP/3.0",
        _ => "HTTP/1.1",
    }
}

pub(crate) fn mode(m: ProxyMode) -> &'static str {
    match m {
        ProxyMode::Http => "http",
        ProxyMode::Inspect => "inspect",
        ProxyMode::Tunnel => "tunnel",
    }
}

/// `?q=1&b=a%20b` as HAR `queryString`, decoded; `+` is a space.
fn query_string(url: &str) -> Vec<Value> {
    let Some((_, query)) = url.split_once('?') else { return vec![] };
    let query = query.split('#').next().unwrap_or("");
    let decode = |s: &str| percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned();
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            json!({ "name": decode(name), "value": decode(value) })
        })
        .collect()
}

/// `2026-10-02T09:35:12.345Z`.
pub(crate) fn iso_ms(t: SystemTime) -> String {
    let t = time::OffsetDateTime::from(t);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond()
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use hyper::header::HeaderValue;

    use super::*;

    fn at() -> SystemTime {
        // 2026-10-02T09:35:12.345Z
        UNIX_EPOCH + Duration::from_millis(1_790_933_712_345)
    }

    fn get() -> HarRecord {
        let mut r = HarRecord::new(at(), "GET", "http://api.example.com/a?q=1&api_key=abc&x=a%20b+c".into(), ProxyMode::Http);
        r.wait_ms = 12;
        r.status = 200;
        r.request_headers.insert("accept", HeaderValue::from_static("*/*"));
        r.response_headers.insert("content-type", HeaderValue::from_static("application/json"));
        r.response_headers.insert("content-length", HeaderValue::from_static("42"));
        r
    }

    // T1: absolute-form GET: method, full URL with query, queryString,
    // status, statusText, times, _mode http.
    #[test]
    fn an_absolute_form_get() {
        let e = get().to_entry();
        assert_eq!(e["startedDateTime"], "2026-10-02T09:35:12.345Z");
        assert_eq!(e["time"], 12);
        assert_eq!(e["timings"], json!({"send": 0, "wait": 12, "receive": 0}));
        assert_eq!(e["request"]["method"], "GET");
        assert_eq!(e["request"]["url"], "http://api.example.com/a?q=1&api_key=abc&x=a%20b+c", "U2: the query is written as it is");
        assert_eq!(
            e["request"]["queryString"],
            json!([{"name": "q", "value": "1"}, {"name": "api_key", "value": "abc"}, {"name": "x", "value": "a b c"}])
        );
        assert_eq!(e["request"]["httpVersion"], "HTTP/1.1");
        assert_eq!(e["request"]["bodySize"], -1);
        assert_eq!(e["response"]["status"], 200);
        assert_eq!(e["response"]["statusText"], "OK");
        assert_eq!(e["response"]["bodySize"], 42);
        assert_eq!(e["response"]["content"], json!({"size": 42, "mimeType": "application/json"}));
        assert_eq!(e["response"]["redirectURL"], "");
        assert_eq!(e["_mode"], "http");
        assert!(e.get("_route").is_none() && e.get("_scripts").is_none() && e.get("_bytesIn").is_none());
        assert!(e.get("_client").is_none(), "the main port writes no _client (ADR 09)");
        assert!(e["response"].get("content").unwrap().get("text").is_none(), "U1: no bodies");
    }

    // ADR 09: a request on a client's port names the client.
    #[test]
    fn a_client_port_names_its_client() {
        let mut r = get();
        r.client = Some(Arc::from("chrome"));
        assert_eq!(r.to_entry()["_client"], "chrome");
    }

    // T1: inspect and .localhost through the proxy.
    #[test]
    fn inspect_mode_and_a_route() {
        let mut r = get();
        r.mode = ProxyMode::Inspect;
        r.route = Some("shop/blog".into());
        let e = r.to_entry();
        assert_eq!(e["_mode"], "inspect");
        assert_eq!(e["_route"], "shop/blog");
    }

    // T1: a tunnel is one CONNECT with its bytes.
    #[test]
    fn a_tunnel() {
        let e = HarRecord::tunnel(at(), "example.com", 443, 200, 900, (10, 20)).to_entry();
        assert_eq!(e["request"]["method"], "CONNECT");
        assert_eq!(e["request"]["url"], "https://example.com:443");
        assert_eq!(e["_mode"], "tunnel");
        assert_eq!((e["_bytesIn"].as_u64(), e["_bytesOut"].as_u64()), (Some(10), Some(20)));
        assert_eq!(e["time"], 900);
    }

    // T1: sizes, mime type and redirect from the headers.
    #[test]
    fn sizes_mime_and_redirect() {
        let mut r = get();
        r.status = 302;
        r.request_headers.insert("content-length", HeaderValue::from_static("7"));
        r.response_headers.remove("content-length");
        r.response_headers.insert("location", HeaderValue::from_static("/next"));
        let e = r.to_entry();
        assert_eq!(e["request"]["bodySize"], 7);
        assert_eq!(e["response"]["bodySize"], -1);
        assert_eq!(e["response"]["content"]["size"], 0);
        assert_eq!(e["response"]["redirectURL"], "/next");
        assert_eq!(e["response"]["statusText"], "Found");
    }

    // T1: the script rules that ran.
    #[test]
    fn scripts_and_script_error() {
        let mut r = get();
        r.scripts = vec!["a".into(), "b".into()];
        r.script_error = Some("b".into());
        let e = r.to_entry();
        assert_eq!(e["_scripts"], json!(["a", "b"]));
        assert_eq!(e["_scriptError"], "b");
    }

    // T3: every header is written as it is, cookies and keys too;
    // Proxy-Authorization is a header for the proxy and is absent.
    #[test]
    fn headers_are_written_as_they_are() {
        let mut r = get();
        for (name, value) in [
            ("authorization", "Bearer t"),
            ("cookie", "a=1"),
            ("x-api-key", "k"),
            ("proxy-authorization", "Basic p"),
        ] {
            r.request_headers.insert(name, HeaderValue::from_static(value));
        }
        r.response_headers.insert("set-cookie", HeaderValue::from_static("b=2"));
        let e = r.to_entry();
        let names: Vec<&str> = e["request"]["headers"].as_array().unwrap().iter().map(|h| h["name"].as_str().unwrap()).collect();
        assert!(!names.contains(&"proxy-authorization"), "{names:?}");
        let value = |list: &Value, name: &str| {
            list.as_array().unwrap().iter().find(|h| h["name"] == name).map(|h| h["value"].clone()).unwrap()
        };
        assert_eq!(value(&e["request"]["headers"], "authorization"), "Bearer t");
        assert_eq!(value(&e["request"]["headers"], "cookie"), "a=1");
        assert_eq!(value(&e["request"]["headers"], "x-api-key"), "k");
        assert_eq!(value(&e["request"]["headers"], "accept"), "*/*");
        assert_eq!(value(&e["response"]["headers"], "set-cookie"), "b=2");
    }

    fn copied(data: &[u8], size: u64, truncated: bool) -> Copied {
        Copied { data: data.to_vec(), size, truncated }
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    // Bodies: text as it is, sizes from the bytes that passed, the time to
    // the end of the body.
    #[test]
    fn text_bodies_are_written_with_their_sizes() {
        let mut r = get();
        r.method = "POST".into();
        r.request_headers.insert("content-type", HeaderValue::from_static("application/json"));
        r.request_body = Some(copied(br#"{"q":1}"#, 7, false));
        r.response_body = Some(copied(br#"{"ok":true}"#, 11, false));
        r.receive_ms = 5;
        let e = r.to_entry();
        assert_eq!(e["request"]["postData"], json!({"mimeType": "application/json", "text": "{\"q\":1}"}));
        assert_eq!(e["request"]["bodySize"], 7);
        assert_eq!(e["response"]["content"], json!({"size": 11, "mimeType": "application/json", "text": "{\"ok\":true}"}));
        assert_eq!(e["response"]["bodySize"], 11);
        assert_eq!((e["time"].as_u64(), e["timings"]["receive"].as_u64()), (Some(17), Some(5)));
        assert!(e["_id"].as_u64().unwrap() > 0);
    }

    // A gzip body is written decoded; bodySize stays the bytes on the wire.
    #[test]
    fn an_encoded_body_is_decoded() {
        let plain = "hello ".repeat(1000);
        let wire = gzip(plain.as_bytes());
        let mut r = get();
        r.response_headers.insert("content-type", HeaderValue::from_static("text/plain; charset=utf-8"));
        r.response_headers.insert("content-encoding", HeaderValue::from_static("gzip"));
        r.response_body = Some(copied(&wire, wire.len() as u64, false));
        let e = r.to_entry();
        assert_eq!(e["response"]["content"]["text"], plain);
        assert_eq!(e["response"]["content"]["size"], plain.len());
        assert_eq!(e["response"]["bodySize"], wire.len());
    }

    // Bytes that are not text are base64; a cut body says so.
    #[test]
    fn binary_and_cut_bodies() {
        let mut r = get();
        r.response_headers.insert("content-type", HeaderValue::from_static("image/png"));
        r.response_body = Some(copied(&[0x89, b'P', b'N', b'G'], 9000, true));
        let e = r.to_entry();
        let c = &e["response"]["content"];
        assert_eq!((c["text"].as_str(), c["encoding"].as_str()), (Some("iVBORw=="), Some("base64")));
        assert_eq!(c["_truncated"], true);
        assert_eq!(e["response"]["bodySize"], 9000);

        let mut r = get();
        r.response_headers.insert("content-type", HeaderValue::from_static("text/plain"));
        r.response_body = Some(copied("aé".as_bytes().split_at(2).0, 3, true));
        assert_eq!(r.to_entry()["response"]["content"]["text"], "a", "a cut character is left out");
    }

    // A WebSocket keeps its messages in Chrome's field.
    #[test]
    fn websocket_messages() {
        let mut r = get();
        r.status = 101;
        r.ws_messages = Some(vec![WsMessage { kind: "send", time: 1.5, opcode: 1, data: "hi".into(), size: 2, truncated: false }]);
        r.ws_dropped = 3;
        r.receive_ms = 4000;
        let e = r.to_entry();
        assert_eq!(e["_resourceType"], "websocket");
        assert_eq!(e["_webSocketMessages"], json!([{"type": "send", "time": 1.5, "opcode": 1, "data": "hi", "_size": 2}]));
        assert_eq!(e["_webSocketDropped"], 3);
        assert_eq!(e["timings"]["receive"], 4000);
    }
}
