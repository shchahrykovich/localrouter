//! What a script sees (ADR 07, change 1): the `req`, `res` and `ex` tables
//! and their write-back, and the modules `json`, `sse`, `base64`, `url`,
//! `multipart`, `log` and, for log rules, `capture`.
//!
//! Secret headers are shown as `[redacted]`. Redaction is a view: a value the
//! script leaves as `[redacted]` keeps its original bytes (I8).

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use bytes::Bytes;
use mlua::{Lua, Table, Value};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::scripts::bodies::{self, BodyClass};
use crate::scripts::events::{self, Event, Format};

/// The value a script sees for a secret header.
pub const REDACTED: &str = "[redacted]";

/// Secret by default (ADR 07, change 4); `secret_headers` in the config adds names.
pub const DEFAULT_SECRET_HEADERS: [&str; 7] =
    ["authorization", "proxy-authorization", "cookie", "set-cookie", "x-api-key", "api-key", "x-auth-token"];

/// Lines a rule may write to the daemon log per minute.
pub const LOG_LINES_PER_MINUTE: u32 = 200;

/// How a body reached the script, and why there is none (ADR 07, change 4,
/// "Body fields a script sees").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BodyMeta {
    pub content_type: Option<String>,
    pub class: Option<BodyClass>,
    /// Bytes on the wire. `None` while unknown.
    pub size: Option<u64>,
    pub truncated: bool,
    /// `class`, `too_large`, `budget`, `streamed`, `slow_disk`, `quota`,
    /// `save_error`, `upgrade`, `error`.
    pub skipped: Option<&'static str>,
    /// Path inside `output_dir` of a saved body.
    pub file: Option<String>,
    pub file_truncated: bool,
    /// The original `Content-Encoding`.
    pub encoding: Option<String>,
    /// `Range` on a request, `Content-Range` on a response.
    pub range: Option<String>,
    pub charset: Option<String>,
    pub events_skipped: u64,
}

impl BodyMeta {
    /// The fields that come from the headers.
    pub fn from_headers(headers: &[(String, Bytes)], range_header: &str, class: BodyClass) -> Self {
        let get = |name: &str| {
            headers.iter().find(|(n, _)| n == name).map(|(_, v)| String::from_utf8_lossy(v).into_owned())
        };
        let content_type = get("content-type");
        let charset = content_type.as_deref().and_then(|ct| {
            ct.split(';').skip(1).find_map(|p| {
                let (k, v) = p.split_once('=')?;
                k.trim().eq_ignore_ascii_case("charset").then(|| v.trim().trim_matches('"').to_ascii_lowercase())
            })
        });
        BodyMeta {
            size: get("content-length").and_then(|v| v.trim().parse().ok()),
            encoding: get("content-encoding").filter(|e| !e.trim().eq_ignore_ascii_case("identity")),
            range: get(range_header),
            content_type,
            charset,
            class: Some(class),
            ..Default::default()
        }
    }
}

/// One side of an exchange: headers (lower-case names, in order) and body.
#[derive(Debug, Clone, Default)]
pub struct Message {
    pub headers: Vec<(String, Bytes)>,
    /// Decoded. `None` when not held or copied.
    pub body: Option<Bytes>,
    pub meta: BodyMeta,
}

impl Message {
    pub fn header(&self, name: &str) -> Option<&Bytes> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

/// The request as a script sees it.
#[derive(Debug, Clone, Default)]
pub struct RequestInfo {
    pub id: String,
    /// `proxy` or `router`.
    pub source: &'static str,
    pub route: Option<String>,
    pub method: String,
    pub scheme: &'static str,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub query: String,
    pub msg: Message,
}

#[derive(Debug, Clone, Default)]
pub struct ResponseInfo {
    pub status: u16,
    pub msg: Message,
}

/// A copy of a finished exchange, for a log script.
#[derive(Debug, Clone, Default)]
pub struct Exchange {
    pub request: RequestInfo,
    pub response: ResponseInfo,
    pub time_ms: u64,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub answered_by: Option<String>,
    pub upgraded: bool,
}

/// A response an intercept script returned from `on_request`.
#[derive(Debug, Clone, Default)]
pub struct ScriptResponse {
    pub status: u16,
    pub headers: Vec<(String, Bytes)>,
    pub body: Bytes,
}

/// What a body change is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyRule {
    /// The script may replace the body.
    Free,
    /// The body was not held because it is too large: a change is an error.
    TooLarge,
    /// A partial body (`206`, `Content-Range`): a change is an error (I24).
    Partial,
}

/// What the script did to a message: the new headers, and a new body if it
/// set one.
#[derive(Debug, Clone, Default)]
pub struct MessageChange {
    pub headers: Vec<(String, Bytes)>,
    pub body: Option<Bytes>,
}

/// Everything a state needs to know about its rule.
pub struct StateCtx {
    pub rule_id: String,
    pub log: bool,
    pub capture: Option<Arc<CaptureTarget>>,
    pub lines: Arc<LineLimiter>,
}

/// Where a log rule writes, and its quota.
#[derive(Debug)]
pub struct CaptureTarget {
    pub dir: PathBuf,
    pub quota: u64,
    /// Bytes written by this rule, capture files and saved bodies together.
    pub written: Arc<AtomicU64>,
}

impl CaptureTarget {
    /// Take `n` bytes of the quota, or fail with "capture quota reached".
    pub fn take(&self, n: u64) -> Result<(), String> {
        let mut cur = self.written.load(Ordering::Relaxed);
        loop {
            let next = cur + n;
            if next > self.quota {
                return Err(format!("capture quota reached ({} bytes, max_capture_bytes)", self.quota));
            }
            match self.written.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => return Ok(()),
                Err(actual) => cur = actual,
            }
        }
    }

    /// Give back bytes taken but not written.
    pub fn give_back(&self, n: u64) {
        self.written.fetch_sub(n, Ordering::AcqRel);
    }
}

/// At most 200 log lines a minute per rule.
#[derive(Debug)]
pub struct LineLimiter {
    window: Mutex<(Instant, u32)>,
}

impl Default for LineLimiter {
    fn default() -> Self {
        Self { window: Mutex::new((Instant::now(), 0)) }
    }
}

impl LineLimiter {
    fn allow(&self) -> bool {
        let mut w = self.window.lock().unwrap();
        if w.0.elapsed().as_secs() >= 60 {
            *w = (Instant::now(), 0);
        }
        w.1 += 1;
        w.1 <= LOG_LINES_PER_MINUTE
    }
}

/// Set while an `on_*` function runs: `capture` works only then, not while
/// the file loads.
pub struct InCall(pub bool);

// ---- the sandbox's modules

const ARRAY_MT: &str = "localrouter.json.array";

/// Put the modules into a new state's globals.
pub fn install(lua: &Lua, ctx: &StateCtx) -> mlua::Result<()> {
    let g = lua.globals();
    let array_mt = lua.create_table()?;
    lua.set_named_registry_value(ARRAY_MT, array_mt)?;

    let json = lua.create_table()?;
    json.set("null", Value::NULL)?;
    json.set(
        "encode",
        lua.create_function(|lua, v: Value| {
            let mut path = vec![];
            let j = to_json(lua, &v, &mut path, 0).map_err(mlua::Error::runtime)?;
            serde_json::to_string(&j).map_err(mlua::Error::runtime)
        })?,
    )?;
    json.set(
        "decode",
        lua.create_function(|lua, s: mlua::String| {
            let v: serde_json::Value = serde_json::from_slice(&s.as_bytes()).map_err(|e| mlua::Error::runtime(format!("json.decode: {e}")))?;
            from_json(lua, &v)
        })?,
    )?;
    g.set("json", json)?;

    let sse = lua.create_table()?;
    sse.set(
        "parse",
        lua.create_function(|lua, s: mlua::String| {
            let list = lua.create_table()?;
            for (i, e) in events::parse_all(Format::Sse, &s.as_bytes()).iter().enumerate() {
                list.raw_set(i + 1, event_fields(lua, e, None)?)?;
            }
            Ok(list)
        })?,
    )?;
    g.set("sse", sse)?;

    let b64 = lua.create_table()?;
    b64.set("encode", lua.create_function(|_, s: mlua::String| Ok(base64_encode(&s.as_bytes())))?)?;
    b64.set(
        "decode",
        lua.create_function(|lua, s: mlua::String| {
            let bytes = base64_decode(&s.as_bytes()).ok_or_else(|| mlua::Error::runtime("base64.decode: not base64"))?;
            lua.create_string(bytes)
        })?,
    )?;
    g.set("base64", b64)?;

    let url = lua.create_table()?;
    url.set("encode", lua.create_function(|_, s: String| Ok(utf8_percent_encode(&s, COMPONENT).to_string()))?)?;
    url.set(
        "decode",
        lua.create_function(|lua, s: String| lua.create_string(percent_decode_str(&s).collect::<Vec<u8>>()))?,
    )?;
    url.set(
        "parse_query",
        lua.create_function(|lua, s: String| {
            let t = lua.create_table()?;
            for pair in s.trim_start_matches('?').split('&').filter(|p| !p.is_empty()) {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                let dec = |x: &str| -> Vec<u8> { percent_decode_str(&x.replace('+', " ")).collect() };
                let (k, v) = (lua.create_string(dec(k))?, lua.create_string(dec(v))?);
                match t.raw_get::<Value>(&k)? {
                    Value::Nil => t.raw_set(k, v)?,
                    Value::Table(list) => list.raw_set(list.raw_len() + 1, v)?,
                    first => {
                        let list = lua.create_table()?;
                        list.raw_set(1, first)?;
                        list.raw_set(2, v)?;
                        t.raw_set(k, list)?;
                    }
                }
            }
            Ok(t)
        })?,
    )?;
    g.set("url", url)?;

    let multipart = lua.create_table()?;
    multipart.set(
        "parse",
        lua.create_function(|lua, (body, content_type): (mlua::String, String)| {
            let parts = parse_multipart(&body.as_bytes(), &content_type).map_err(mlua::Error::runtime)?;
            let list = lua.create_table()?;
            for (i, p) in parts.into_iter().enumerate() {
                let t = lua.create_table()?;
                t.set("name", p.name)?;
                t.set("filename", p.filename)?;
                t.set("content_type", p.content_type)?;
                let h = lua.create_table()?;
                for (n, v) in p.headers {
                    h.set(n, v)?;
                }
                t.set("headers", h)?;
                t.set("data", lua.create_string(&p.data)?)?;
                list.raw_set(i + 1, t)?;
            }
            Ok(list)
        })?,
    )?;
    g.set("multipart", multipart)?;

    let log = lua.create_table()?;
    for (name, warn) in [("info", false), ("warn", true)] {
        let id = ctx.rule_id.clone();
        let lines = ctx.lines.clone();
        log.set(
            name,
            lua.create_function(move |_, msg: mlua::String| {
                write_line(&id, &lines, warn, &String::from_utf8_lossy(&msg.as_bytes()));
                Ok(())
            })?,
        )?;
    }
    g.set("log", log)?;
    let id = ctx.rule_id.clone();
    let lines = ctx.lines.clone();
    g.set(
        "print",
        lua.create_function(move |_, args: mlua::MultiValue| {
            let parts: Vec<String> = args
                .iter()
                .map(|v| match v {
                    Value::String(s) => String::from_utf8_lossy(&s.as_bytes()).into_owned(),
                    other => other.to_string().unwrap_or_else(|_| other.type_name().to_string()),
                })
                .collect();
            write_line(&id, &lines, false, &parts.join("\t"));
            Ok(())
        })?,
    )?;

    if ctx.log {
        let capture = lua.create_table()?;
        for (name, append) in [("write", false), ("append", true)] {
            let target = ctx.capture.clone();
            capture.set(
                name,
                lua.create_function(move |lua, (file, data): (String, mlua::String)| {
                    if !lua.app_data_ref::<InCall>().is_some_and(|c| c.0) {
                        return Err(mlua::Error::runtime("capture works inside on_exchange and on_event, not while the file loads"));
                    }
                    let target = target.as_ref().ok_or_else(|| mlua::Error::runtime("this rule has no output_dir"))?;
                    capture_write(target, &file, &data.as_bytes(), append).map_err(mlua::Error::runtime)
                })?,
            )?;
        }
        g.set("capture", capture)?;
    }
    Ok(())
}

fn write_line(rule: &str, lines: &LineLimiter, warn: bool, msg: &str) {
    if !lines.allow() {
        return;
    }
    let msg: String = msg.chars().take(2000).collect();
    if warn {
        tracing::warn!("script rule {rule}: {msg}");
    } else {
        tracing::info!("script rule {rule}: {msg}");
    }
}

/// Characters `url.encode` keeps as they are: letters, digits and `-._~`.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

/// A capture file name: flat, no `/`, no `..`, not hidden.
pub fn valid_capture_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 128
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// `capture.write` (replace through a temp file and a rename) and
/// `capture.append` (`O_APPEND`). Both open with `O_NOFOLLOW`, mode 0600,
/// and count against the quota (I7).
pub fn capture_write(target: &CaptureTarget, name: &str, data: &[u8], append: bool) -> Result<(), String> {
    if !valid_capture_name(name) {
        return Err(format!(
            "capture name {name:?}: use letters, digits, '.', '_' and '-', starting with a letter or digit (no folders)"
        ));
    }
    target.take(data.len() as u64)?;
    let path = target.dir.join(name);
    let result = if append {
        bodies::open_append(&path).and_then(|mut f| f.write_all(data))
    } else {
        let tmp = target.dir.join(format!(".lr-tmp-{}-{}", std::process::id(), crate::scripts::new_id()));
        let r = bodies::create_new(&tmp).and_then(|mut f| f.write_all(data)).and_then(|_| std::fs::rename(&tmp, &path));
        if r.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        r
    };
    result.map_err(|e| {
        target.give_back(data.len() as u64);
        format!("capture.{} {name}: {e}", if append { "append" } else { "write" })
    })
}

// ---- JSON

fn path_text(path: &[String]) -> String {
    if path.is_empty() { "the value".into() } else { format!("field {}", path.join(".")) }
}

/// A Lua value as JSON. A string that is not UTF-8 is an error with the hint
/// of ADR 07, change 4, section 10.
pub fn to_json(lua: &Lua, v: &Value, path: &mut Vec<String>, depth: usize) -> Result<serde_json::Value, String> {
    if depth > 100 {
        return Err(format!("{} is nested too deep (or refers to itself)", path_text(path)));
    }
    Ok(match v {
        Value::Nil => serde_json::Value::Null,
        Value::LightUserData(p) if p.0.is_null() => serde_json::Value::Null,
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Integer(i) => serde_json::Value::from(*i),
        Value::Number(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| format!("{} is {n}, which JSON cannot hold", path_text(path)))?,
        Value::String(s) => match std::str::from_utf8(&s.as_bytes()) {
            Ok(text) => serde_json::Value::String(text.to_string()),
            Err(_) => {
                return Err(format!(
                    "{} is not text: use base64.encode, or save the body to a file",
                    path_text(path)
                ));
            }
        },
        Value::Table(t) => {
            let is_array_mt = match (t.metatable(), lua.named_registry_value::<Table>(ARRAY_MT)) {
                (Some(mt), Ok(arr)) => mt == arr,
                _ => false,
            };
            let len = t.raw_len();
            let mut count = 0;
            for pair in t.pairs::<Value, Value>() {
                pair.map_err(|e| e.to_string())?;
                count += 1;
            }
            if is_array_mt || (len > 0 && count == len) {
                let mut out = Vec::with_capacity(len);
                for i in 1..=len {
                    path.push(i.to_string());
                    out.push(to_json(lua, &t.raw_get::<Value>(i).map_err(|e| e.to_string())?, path, depth + 1)?);
                    path.pop();
                }
                serde_json::Value::Array(out)
            } else {
                let mut map = serde_json::Map::new();
                for pair in t.pairs::<Value, Value>() {
                    let (k, v) = pair.map_err(|e| e.to_string())?;
                    let key = match &k {
                        Value::String(s) => String::from_utf8_lossy(&s.as_bytes()).into_owned(),
                        Value::Integer(i) => i.to_string(),
                        Value::Number(n) => n.to_string(),
                        other => return Err(format!("{} has a {} key; JSON keys are strings", path_text(path), other.type_name())),
                    };
                    path.push(key.clone());
                    let j = to_json(lua, &v, path, depth + 1)?;
                    path.pop();
                    map.insert(key, j);
                }
                serde_json::Value::Object(map)
            }
        }
        other => return Err(format!("{} is a {}, which JSON cannot hold", path_text(path), other.type_name())),
    })
}

/// JSON as a Lua value. Arrays get a marker, so an empty array stays one.
pub fn from_json(lua: &Lua, v: &serde_json::Value) -> mlua::Result<Value> {
    Ok(match v {
        serde_json::Value::Null => Value::NULL,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        serde_json::Value::String(s) => Value::String(lua.create_string(s)?),
        serde_json::Value::Array(items) => {
            let t = lua.create_table_with_capacity(items.len(), 0)?;
            for (i, item) in items.iter().enumerate() {
                t.raw_set(i + 1, from_json(lua, item)?)?;
            }
            t.set_metatable(Some(lua.named_registry_value::<Table>(ARRAY_MT)?))?;
            Value::Table(t)
        }
        serde_json::Value::Object(map) => {
            let t = lua.create_table_with_capacity(0, map.len())?;
            for (k, item) in map {
                t.raw_set(k.as_str(), from_json(lua, item)?)?;
            }
            Value::Table(t)
        }
    })
}

// ---- base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, &b)| acc | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard or URL-safe alphabet, with or without padding; spaces and line
/// ends are ignored.
pub fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ---- multipart

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Part {
    pub name: Option<String>,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub headers: Vec<(String, String)>,
    pub data: Vec<u8>,
}

/// `multipart.parse(body, content_type)`.
pub fn parse_multipart(body: &[u8], content_type: &str) -> Result<Vec<Part>, String> {
    let boundary = content_type
        .split(';')
        .skip(1)
        .find_map(|p| {
            let (k, v) = p.split_once('=')?;
            k.trim().eq_ignore_ascii_case("boundary").then(|| v.trim().trim_matches('"').to_string())
        })
        .ok_or("multipart.parse: the content type has no boundary")?;
    let delim = format!("--{boundary}");
    let mut parts = vec![];
    let mut rest = body;
    // Skip the preamble up to the first delimiter.
    let first = find(rest, delim.as_bytes()).ok_or("multipart.parse: no part found")?;
    rest = &rest[first + delim.len()..];
    loop {
        if rest.starts_with(b"--") {
            break;
        }
        rest = rest.strip_prefix(b"\r\n").or_else(|| rest.strip_prefix(b"\n")).ok_or("multipart.parse: a delimiter without a line end")?;
        let end = find(rest, format!("\r\n{delim}").as_bytes())
            .map(|i| (i, i + 2 + delim.len()))
            .or_else(|| find(rest, format!("\n{delim}").as_bytes()).map(|i| (i, i + 1 + delim.len())))
            .ok_or("multipart.parse: a part has no end")?;
        let part = &rest[..end.0];
        rest = &rest[end.1..];
        let (head, data) = match find(part, b"\r\n\r\n") {
            Some(i) => (&part[..i], &part[i + 4..]),
            None => match find(part, b"\n\n") {
                Some(i) => (&part[..i], &part[i + 2..]),
                None => (&part[..0], part),
            },
        };
        let mut p = Part { data: data.to_vec(), ..Part::default() };
        for line in String::from_utf8_lossy(head).lines() {
            let Some((name, value)) = line.split_once(':') else { continue };
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim().to_string());
            if name == "content-disposition" {
                for param in value.split(';').skip(1) {
                    if let Some((k, v)) = param.split_once('=') {
                        let v = v.trim().trim_matches('"').to_string();
                        match k.trim() {
                            "name" => p.name = Some(v),
                            "filename" => p.filename = Some(v),
                            _ => {}
                        }
                    }
                }
            }
            if name == "content-type" {
                p.content_type = Some(value.clone());
            }
            p.headers.push((name, value));
        }
        parts.push(p);
    }
    Ok(parts)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// ---- the req, res and ex tables

/// The script's view of headers: lower-case names; a repeated header is a
/// list; a secret header's value is `[redacted]` unless `reveal`.
fn headers_table(lua: &Lua, headers: &[(String, Bytes)], secret: &dyn Fn(&str) -> bool, reveal: bool) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (name, value) in headers {
        let shown: &[u8] = if !reveal && secret(name) { REDACTED.as_bytes() } else { value };
        let v = lua.create_string(shown)?;
        match t.raw_get::<Value>(name.as_str())? {
            Value::Nil => t.raw_set(name.as_str(), v)?,
            Value::Table(list) => list.raw_set(list.raw_len() + 1, v)?,
            first => {
                let list = lua.create_table()?;
                list.raw_set(1, first)?;
                list.raw_set(2, v)?;
                t.raw_set(name.as_str(), list)?;
            }
        }
    }
    Ok(t)
}

/// Read headers back. An untouched `[redacted]` keeps the original value.
fn read_headers(t: &Table, original: &[(String, Bytes)], secret: &dyn Fn(&str) -> bool) -> Result<Vec<(String, Bytes)>, String> {
    let mut out = vec![];
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair.map_err(|e| e.to_string())?;
        let name = match k {
            Value::String(s) => String::from_utf8_lossy(&s.as_bytes()).to_ascii_lowercase(),
            other => return Err(format!("a header name must be a string, not a {}", other.type_name())),
        };
        if hyper::header::HeaderName::from_bytes(name.as_bytes()).is_err() {
            return Err(format!("{name:?} is not a valid header name"));
        }
        let values: Vec<Bytes> = match v {
            Value::String(s) => vec![Bytes::copy_from_slice(&s.as_bytes())],
            Value::Integer(i) => vec![Bytes::from(i.to_string())],
            Value::Number(n) => vec![Bytes::from(n.to_string())],
            Value::Table(list) => {
                let mut vs = vec![];
                for item in list.sequence_values::<Value>() {
                    match item.map_err(|e| e.to_string())? {
                        Value::String(s) => vs.push(Bytes::copy_from_slice(&s.as_bytes())),
                        other => return Err(format!("header {name}: a value must be a string, not a {}", other.type_name())),
                    }
                }
                vs
            }
            other => return Err(format!("header {name}: a value must be a string or a list, not a {}", other.type_name())),
        };
        let originals: Vec<&Bytes> = original.iter().filter(|(n, _)| *n == name).map(|(_, v)| v).collect();
        for (i, value) in values.into_iter().enumerate() {
            let value = if secret(&name) && value == REDACTED.as_bytes() {
                match originals.get(i) {
                    Some(orig) => (*orig).clone(),
                    None => continue,
                }
            } else {
                value
            };
            if hyper::header::HeaderValue::from_maybe_shared(value.clone()).is_err() {
                return Err(format!("header {name}: the value has a line end or another character headers cannot hold"));
            }
            out.push((name.clone(), value));
        }
    }
    // Keep the original order of names that are still there.
    let rank = |n: &str| original.iter().position(|(o, _)| o == n).unwrap_or(usize::MAX);
    out.sort_by_key(|(n, _)| rank(n));
    Ok(out)
}

fn set_body_fields(lua: &Lua, t: &Table, m: &Message) -> mlua::Result<()> {
    let meta = &m.meta;
    match &m.body {
        Some(b) => t.set("body", lua.create_string(b)?)?,
        None => t.set("body", Value::Nil)?,
    }
    t.set("content_type", meta.content_type.clone())?;
    t.set("body_class", meta.class.map(BodyClass::as_str))?;
    t.set("body_size", meta.size)?;
    t.set("body_truncated", meta.truncated)?;
    t.set("body_skipped", meta.skipped)?;
    t.set("body_file", meta.file.clone())?;
    t.set("body_file_truncated", meta.file_truncated)?;
    t.set("body_encoding", meta.encoding.clone())?;
    t.set("range", meta.range.clone())?;
    t.set("charset", meta.charset.clone())?;
    t.set("events_skipped", meta.events_skipped)?;
    Ok(())
}

/// The `req` table (intercept) or `ex.request` (log).
pub fn request_table(lua: &Lua, r: &RequestInfo, secret: &dyn Fn(&str) -> bool, reveal: bool) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("id", r.id.as_str())?;
    t.set("source", r.source)?;
    t.set("route", r.route.clone())?;
    t.set("method", r.method.as_str())?;
    t.set("scheme", r.scheme)?;
    t.set("host", r.host.as_str())?;
    t.set("port", r.port)?;
    t.set("path", r.path.as_str())?;
    t.set("query", r.query.as_str())?;
    t.set("headers", headers_table(lua, &r.msg.headers, secret, reveal)?)?;
    set_body_fields(lua, &t, &r.msg)?;
    Ok(t)
}

/// The `res` table (intercept) or `ex.response` (log).
pub fn response_table(lua: &Lua, r: &ResponseInfo, secret: &dyn Fn(&str) -> bool, reveal: bool) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("status", r.status)?;
    t.set("headers", headers_table(lua, &r.msg.headers, secret, reveal)?)?;
    set_body_fields(lua, &t, &r.msg)?;
    Ok(t)
}

/// The `ex` table of a log script.
pub fn exchange_table(lua: &Lua, ex: &Exchange, secret: &dyn Fn(&str) -> bool, reveal: bool) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("request", request_table(lua, &ex.request, secret, reveal)?)?;
    t.set("response", response_table(lua, &ex.response, secret, reveal)?)?;
    t.set("time_ms", ex.time_ms)?;
    t.set("duration_ms", ex.duration_ms)?;
    t.set("error", ex.error.clone())?;
    t.set("answered_by", ex.answered_by.clone())?;
    t.set("upgraded", ex.upgraded)?;
    Ok(t)
}

/// One event as a table: `event`, `data`, `id`, `retry` and, for log
/// scripts, `index`.
pub fn event_fields(lua: &Lua, e: &Event, index: Option<u64>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("event", e.event.clone())?;
    t.set("data", lua.create_string(&e.data)?)?;
    t.set("id", e.id.clone())?;
    t.set("retry", e.retry.clone())?;
    t.set("index", index)?;
    Ok(t)
}

/// Read an event back after an intercept `on_event`.
pub fn read_event(t: &Table, original: &Event) -> Result<Event, String> {
    let s = |key: &str| -> Result<Option<String>, String> {
        match t.raw_get::<Value>(key).map_err(|e| e.to_string())? {
            Value::Nil => Ok(None),
            Value::String(s) => Ok(Some(String::from_utf8_lossy(&s.as_bytes()).into_owned())),
            Value::Integer(i) => Ok(Some(i.to_string())),
            other => Err(format!("event.{key} must be a string, not a {}", other.type_name())),
        }
    };
    let changed = Event { event: s("event")?, data: s("data")?.unwrap_or_default(), id: s("id")?, retry: s("retry")?, raw: Bytes::new(), comment: false };
    if changed.event == original.event && changed.data == original.data && changed.id == original.id && changed.retry == original.retry {
        return Ok(original.clone());
    }
    Ok(changed)
}

fn body_from(v: Value, what: &str) -> Result<Option<Bytes>, String> {
    match v {
        Value::Nil => Ok(None),
        Value::String(s) => Ok(Some(Bytes::copy_from_slice(&s.as_bytes()))),
        other => Err(format!("{what}.body must be a string or nil, not a {}", other.type_name())),
    }
}

/// Read the body field back: `Some` only when the script changed it.
fn read_body(t: &Table, m: &Message, rule: BodyRule, what: &str) -> Result<Option<Bytes>, String> {
    let now = body_from(t.raw_get::<Value>("body").map_err(|e| e.to_string())?, what)?;
    let changed = match (&now, &m.body) {
        (None, None) => false,
        (Some(a), Some(b)) => a != b,
        _ => true,
    };
    if !changed {
        return Ok(None);
    }
    match rule {
        BodyRule::Free => Ok(Some(now.unwrap_or_default())),
        BodyRule::TooLarge => Err(format!("cannot change {what}.body: the body was too large to hold, so it was sent as it came")),
        BodyRule::Partial => Err(format!("cannot change a partial body ({what}.range is set); change its headers only")),
    }
}

/// What an intercept `on_request` changed. The request info is updated.
pub fn read_request(t: &Table, r: &mut RequestInfo, secret: &dyn Fn(&str) -> bool, rule: BodyRule) -> Result<Option<Bytes>, String> {
    let get_str = |key: &str| -> Result<String, String> {
        match t.raw_get::<Value>(key).map_err(|e| e.to_string())? {
            Value::String(s) => Ok(String::from_utf8_lossy(&s.as_bytes()).into_owned()),
            Value::Nil => Ok(String::new()),
            other => Err(format!("req.{key} must be a string, not a {}", other.type_name())),
        }
    };
    let method = get_str("method")?.to_ascii_uppercase();
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-' || b == b'_') {
        return Err(format!("req.method {method:?} is not an HTTP method"));
    }
    let path = get_str("path")?;
    if !path.starts_with('/') || path.contains(['?', '#', ' ', '\r', '\n']) {
        return Err(format!("req.path {path:?} must start with / and hold no '?', '#' or spaces (use req.query)"));
    }
    let query = get_str("query")?;
    let query = query.trim_start_matches('?').to_string();
    if query.contains(['#', ' ', '\r', '\n']) {
        return Err(format!("req.query {query:?} holds a '#' or a space"));
    }
    for (key, now) in [("host", &r.host), ("scheme", &r.scheme.to_string())] {
        if get_str(key)? != *now {
            return Err(format!("req.{key} cannot be changed: sending a request somewhere else is not possible"));
        }
    }
    let headers = match t.raw_get::<Value>("headers").map_err(|e| e.to_string())? {
        Value::Table(h) => read_headers(&h, &r.msg.headers, secret)?,
        Value::Nil => vec![],
        other => return Err(format!("req.headers must be a table, not a {}", other.type_name())),
    };
    let body = read_body(t, &r.msg, rule, "req")?;
    r.method = method;
    r.path = path;
    r.query = query;
    r.msg.headers = headers;
    if let Some(b) = &body {
        r.msg.body = Some(b.clone());
    }
    Ok(body)
}

/// What an intercept `on_response` changed.
pub fn read_response(t: &Table, r: &mut ResponseInfo, secret: &dyn Fn(&str) -> bool, rule: BodyRule) -> Result<Option<Bytes>, String> {
    let status = match t.raw_get::<Value>("status").map_err(|e| e.to_string())? {
        Value::Integer(i) if (100..=999).contains(&i) => i as u16,
        Value::Number(n) if (100.0..=999.0).contains(&n) && n.fract() == 0.0 => n as u16,
        other => return Err(format!("res.status must be a number from 100 to 999, not {other:?}")),
    };
    let headers = match t.raw_get::<Value>("headers").map_err(|e| e.to_string())? {
        Value::Table(h) => read_headers(&h, &r.msg.headers, secret)?,
        Value::Nil => vec![],
        other => return Err(format!("res.headers must be a table, not a {}", other.type_name())),
    };
    let body = read_body(t, &r.msg, rule, "res")?;
    r.status = status;
    r.msg.headers = headers;
    if let Some(b) = &body {
        r.msg.body = Some(b.clone());
    }
    Ok(body)
}

/// A response returned by `on_request`: `{ status, headers, body }`.
pub fn read_script_response(t: &Table) -> Result<ScriptResponse, String> {
    let status = match t.raw_get::<Value>("status").map_err(|e| e.to_string())? {
        Value::Nil => 200,
        Value::Integer(i) if (200..=999).contains(&i) => i as u16,
        Value::Number(n) if (200.0..=999.0).contains(&n) && n.fract() == 0.0 => n as u16,
        other => return Err(format!("the returned status must be a number from 200 to 999, not {other:?}")),
    };
    let headers = match t.raw_get::<Value>("headers").map_err(|e| e.to_string())? {
        Value::Table(h) => read_headers(&h, &[], &|_| false)?,
        Value::Nil => vec![],
        other => return Err(format!("the returned headers must be a table, not a {}", other.type_name())),
    };
    let body = body_from(t.raw_get::<Value>("body").map_err(|e| e.to_string())?, "the returned")?.unwrap_or_default();
    Ok(ScriptResponse { status, headers, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> Lua {
        let lua = Lua::new();
        install(&lua, &StateCtx { rule_id: "t".into(), log: false, capture: None, lines: Arc::default() }).unwrap();
        lua
    }

    // T25
    #[test]
    fn multipart_parse_returns_the_parts_with_names_and_bytes() {
        let png = [0x89u8, b'P', b'N', b'G', 0, 0xff, 13, 10];
        let mut body = b"--XyZ\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nhello\r\n--XyZ\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\n"
            .to_vec();
        body.extend_from_slice(&png);
        body.extend_from_slice(b"\r\n--XyZ--\r\n");
        let parts = parse_multipart(&body, "multipart/form-data; boundary=XyZ").unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0].name.as_deref(), parts[0].data.as_slice()), (Some("title"), &b"hello"[..]));
        assert_eq!(parts[1].filename.as_deref(), Some("a.png"));
        assert_eq!(parts[1].content_type.as_deref(), Some("image/png"));
        assert_eq!(parts[1].data, png);
        assert!(parse_multipart(b"x", "multipart/form-data").is_err());
    }

    // T25
    #[test]
    fn binary_strings_survive_base64_and_json_refuses_them_with_the_hint() {
        let lua = lua();
        let ok: bool = lua.load(r#"local s = "\0\255\254abc" return base64.decode(base64.encode(s)) == s"#).eval().unwrap();
        assert!(ok);
        let err = lua.load(r#"return json.encode({ response = { body = "\xff\xfe" } })"#).eval::<String>().unwrap_err().to_string();
        assert!(err.contains("field response.body is not text: use base64.encode, or save the body to a file"), "{err}");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_decode(b"aGVsbG8").unwrap(), b"hello");
    }

    #[test]
    fn json_round_trips_and_keeps_empty_arrays() {
        let lua = lua();
        let out: String = lua
            .load(r#"local v = json.decode('{"a":[],"b":[1,2],"c":null,"d":{"e":"f"}}') v.x = 1 return json.encode(v)"#)
            .eval()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, serde_json::json!({"a":[],"b":[1,2],"c":null,"d":{"e":"f"},"x":1}));
        let null: bool = lua.load("return json.decode('null') == json.null").eval().unwrap();
        assert!(null);
    }

    #[test]
    fn url_and_sse_helpers() {
        let lua = lua();
        let (a, b, c): (String, String, String) = lua
            .load(r#"local q = url.parse_query("a=1&b=x%20y&a=2&c=d+e") return q.a[2], q.b, q.c"#)
            .eval()
            .unwrap();
        assert_eq!((a.as_str(), b.as_str(), c.as_str()), ("2", "x y", "d e"));
        let enc: String = lua.load(r#"return url.encode("a b/c")"#).eval().unwrap();
        assert_eq!(enc, "a%20b%2Fc");
        let n: i64 = lua.load(r#"local e = sse.parse("event: x\ndata: 1\n\ndata: 2\n\n") return #e"#).eval().unwrap();
        assert_eq!(n, 2);
    }

    // T10 (unit part), I8: redaction is a view.
    #[test]
    fn redacted_headers_keep_their_value_unless_changed() {
        let lua = lua();
        let secret = |n: &str| DEFAULT_SECRET_HEADERS.contains(&n);
        let original = vec![
            ("authorization".to_string(), Bytes::from_static(b"Bearer real")),
            ("cookie".to_string(), Bytes::from_static(b"a=1")),
            ("x-app".to_string(), Bytes::from_static(b"1")),
        ];
        let t = headers_table(&lua, &original, &secret, false).unwrap();
        assert_eq!(t.get::<String>("authorization").unwrap(), REDACTED);
        t.set("cookie", "b=2").unwrap();
        t.set("x-app", Value::Nil).unwrap();
        t.set("X-New", "yes").unwrap();
        let back = read_headers(&t, &original, &secret).unwrap();
        let get = |n: &str| back.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
        assert_eq!(get("authorization").unwrap(), "Bearer real");
        assert_eq!(get("cookie").unwrap(), "b=2");
        assert!(get("x-app").is_none());
        assert_eq!(get("x-new").unwrap(), "yes");
        let shown = headers_table(&lua, &original, &secret, true).unwrap();
        assert_eq!(shown.get::<String>("authorization").unwrap(), "Bearer real");
    }

    // T9
    #[test]
    fn capture_files_are_flat_0600_never_through_a_link_and_within_the_quota() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = CaptureTarget { dir: dir.path().to_path_buf(), quota: 1024, written: Arc::default() };
        capture_write(&target, "a.jsonl", b"one\n", true).unwrap();
        capture_write(&target, "a.jsonl", b"two\n", true).unwrap();
        let path = dir.path().join("a.jsonl");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        capture_write(&target, "b.json", b"{}", false).unwrap();
        capture_write(&target, "b.json", b"[]", false).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("b.json")).unwrap(), "[]");
        for bad in ["../x", "a/b", ".hidden", "", "-a"] {
            assert!(capture_write(&target, bad, b"x", true).is_err(), "{bad:?}");
        }
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path().join("x"), dir.path().join("link.jsonl")).unwrap();
        assert!(capture_write(&target, "link.jsonl", b"x", true).is_err());
        assert!(!elsewhere.path().join("x").exists());
        let err = capture_write(&target, "big", &[0u8; 2000], true).unwrap_err();
        assert!(err.contains("capture quota reached"), "{err}");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(".lr-tmp")).collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn a_request_change_cannot_move_the_request_or_change_a_partial_body() {
        let lua = lua();
        let secret = |_: &str| false;
        let mut r = RequestInfo { method: "GET".into(), scheme: "https", host: "a.example".into(), path: "/".into(), ..Default::default() };
        let t = request_table(&lua, &r, &secret, false).unwrap();
        t.set("host", "evil.example").unwrap();
        assert!(read_request(&t, &mut r.clone(), &secret, BodyRule::Free).unwrap_err().contains("cannot be changed"));
        let t = request_table(&lua, &r, &secret, false).unwrap();
        t.set("body", "x").unwrap();
        assert!(read_request(&t, &mut r.clone(), &secret, BodyRule::Partial).unwrap_err().contains("partial body"));
        assert!(read_request(&t, &mut r.clone(), &secret, BodyRule::TooLarge).unwrap_err().contains("too large"));
        t.set("path", "/new").unwrap();
        t.set("method", "post").unwrap();
        let body = read_request(&t, &mut r, &secret, BodyRule::Free).unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str(), body.unwrap()), ("POST", "/new", Bytes::from_static(b"x")));
    }
}
