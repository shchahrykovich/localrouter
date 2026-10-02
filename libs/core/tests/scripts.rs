//! ADR 07: script rules on real traffic, through the router (`Proxy`) and
//! the forward proxy, against local servers.
//!
//! T5 (intercept on_request), T6 (on_response), T7 (holding bodies), T8 (log
//! copies), T10 (secrets), T12 (on_error, disable after 20), T18 (no rule, no
//! cost), T19 (where scripts run), T20 (body classes), T21 (event streams),
//! T22 (saved bodies), T23 (Range and 206), T24 (decompression bombs).
//!
//! Replaced parts: the internet is local servers; the real limits are
//! smaller test limits (`Scripts::with_limits`), and a slow disk is a writer
//! that waits on purpose (`save_delay`).

use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use localrouter_core::instance::Instance;
use localrouter_core::logs::{LogEntry, RequestLog};
use localrouter_core::proxy::{ClientScheme, Proxy, RouteSource};
use localrouter_core::routes::{Protocol, Route, RouteTable};
use localrouter_core::scripts::engine::load_file;
use localrouter_core::scripts::rules::ScriptRule;
use localrouter_core::scripts::{Limits, Scripts};
use localrouter_core::tls;
use tokio::net::{TcpListener, TcpStream};

type UpBody = BoxBody<Bytes, Infallible>;

struct Table(RwLock<RouteTable>);

impl RouteSource for Table {
    fn lookup(&self, host: &str, path: &str) -> Option<Route> {
        self.0.read().unwrap().lookup(host, path, true).cloned()
    }
    fn all(&self) -> Vec<Route> {
        self.0.read().unwrap().iter().cloned().collect()
    }
    fn https_port(&self) -> Option<u16> {
        None
    }
}

fn full(b: impl Into<Bytes>) -> UpBody {
    Full::new(b.into()).boxed()
}

fn pseudo_random(n: usize) -> Vec<u8> {
    let mut seed: u64 = 0x9e3779b97f4a7c15;
    (0..n)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

const PNG_SIZE: usize = 5 << 20;
const VIDEO_SIZE: usize = 4 << 20;

/// Times the stream upstream sent each event, so a test can say the client
/// had event 1 before event 2 was sent.
type Sent = Arc<Mutex<Vec<Instant>>>;

/// The test internet: one server with a path per case.
async fn upstream(sent: Sent) -> SocketAddr {
    let png = Bytes::from(pseudo_random(PNG_SIZE));
    let video = Bytes::from(pseudo_random(VIDEO_SIZE));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let (png, video, sent) = (png.clone(), video.clone(), sent.clone());
            tokio::spawn(async move {
                let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
                    let (png, video, sent) = (png.clone(), video.clone(), sent.clone());
                    async move { Ok::<_, Infallible>(answer(req, png, video, sent).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), svc).await;
            });
        }
    });
    addr
}

fn typed(ct: &str, body: impl Into<Bytes>) -> Response<UpBody> {
    Response::builder().header("content-type", ct).body(full(body)).unwrap()
}

fn events(list: Vec<&'static str>, gap: Duration, sent: Sent, ct: &'static str, keep_open: bool) -> Response<UpBody> {
    let stream = futures_util::stream::unfold(0usize, move |i| {
        let list = list.clone();
        let sent = sent.clone();
        async move {
            if i >= list.len() {
                if keep_open {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
                return None;
            }
            if i > 0 {
                tokio::time::sleep(gap).await;
            }
            sent.lock().unwrap().push(Instant::now());
            Some((Ok::<_, Infallible>(Frame::data(Bytes::from_static(list[i].as_bytes()))), i + 1))
        }
    });
    Response::builder().header("content-type", ct).body(BodyExt::boxed(StreamBody::new(stream))).unwrap()
}

async fn answer(req: Request<Incoming>, png: Bytes, video: Bytes, sent: Sent) -> Response<UpBody> {
    let path = req.uri().path().to_string();
    let header = |n: &str| req.headers().get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    match path.as_str() {
        "/sse" => events(vec!["data: one\n\n", "event: ping\ndata: x\n\n", "data: two\n\n", "data: three\n\n"], Duration::from_millis(100), sent, "text/event-stream", false),
        "/sse-open" => events(vec!["data: a\n\n", "data: b\n\n"], Duration::from_millis(50), sent, "text/event-stream", true),
        "/ndjson" => events(vec!["{\"n\":1}\n{\"n\"", ":2}\n", "{\"n\":3}\n"], Duration::from_millis(20), sent, "application/x-ndjson", false),
        "/sse-big" => {
            let big: &'static str = Box::leak(format!("data: {}\n\n", "x".repeat(2 << 20)).into_boxed_str());
            events(vec!["data: small\n\n", big, "data: after\n\n"], Duration::from_millis(10), sent, "text/event-stream", false)
        }
        "/ndjson-big" => {
            // One part ends a small event and starts a 1.5 MB one.
            let first: &'static str = Box::leak(format!("{{\"a\":1}}\n{{\"big\":\"{}", "x".repeat(3 << 19)).into_boxed_str());
            events(vec![first, "\"}\n{\"c\":3}\n"], Duration::from_millis(20), sent, "application/x-ndjson", false)
        }
        "/ndjson-tail" => events(vec!["{\"a\":1}\n", "{\"last\":2}"], Duration::from_millis(20), sent, "application/x-ndjson", false),
        "/sse-two-encodings" => {
            let mut r = events(vec!["data: one\n\n"], Duration::from_millis(1), sent, "text/event-stream", false);
            r.headers_mut().insert("content-encoding", "gzip, br".parse().unwrap());
            r
        }
        "/png" => typed("image/png", png),
        "/png-octet" => typed("application/octet-stream", png),
        "/big" => typed("text/plain", vec![b'a'; 2 << 20]),
        "/gzip-json" => Response::builder()
            .header("content-type", "application/json")
            .header("content-encoding", "gzip")
            .body(full(gzip(br#"{"hello":"world","n":1}"#)))
            .unwrap(),
        "/svg-gz" => Response::builder()
            .header("content-type", "image/svg+xml")
            .header("content-encoding", "gzip")
            .body(full(gzip(b"<svg xmlns='http://www.w3.org/2000/svg'/>")))
            .unwrap(),
        "/bomb" => Response::builder()
            .header("content-type", "text/plain")
            .header("content-encoding", "gzip")
            .body(full(gzip(&vec![b'z'; 64 << 20])))
            .unwrap(),
        "/video" => {
            let range = header("range");
            if let Some(spec) = range.strip_prefix("bytes=") {
                let (a, b) = spec.split_once('-').unwrap();
                let a: usize = a.parse().unwrap();
                let b: usize = if b.is_empty() { VIDEO_SIZE - 1 } else { b.parse().unwrap() };
                Response::builder()
                    .status(206)
                    .header("content-type", "video/mp4")
                    .header("content-range", format!("bytes {a}-{b}/{VIDEO_SIZE}"))
                    .body(full(video.slice(a..=b)))
                    .unwrap()
            } else {
                typed("video/mp4", video)
            }
        }
        _ => {
            // Echo: what the server got, as JSON.
            let method = req.method().to_string();
            let uri = req.uri().to_string();
            let headers: serde_json::Map<String, serde_json::Value> = req
                .headers()
                .iter()
                .map(|(n, v)| (n.to_string(), serde_json::Value::String(String::from_utf8_lossy(v.as_bytes()).into_owned())))
                .collect();
            let body = req.into_body().collect().await.unwrap().to_bytes();
            let json = serde_json::json!({"method": method, "path": uri, "headers": headers, "body": String::from_utf8_lossy(&body)});
            typed("application/json", json.to_string())
        }
    }
}

struct Harness {
    http: SocketAddr,
    log: Arc<RequestLog>,
    scripts: Arc<Scripts>,
    dir: tempfile::TempDir,
    out: PathBuf,
    sent: Sent,
}

fn route(host: &str, target: String) -> Route {
    Route {
        host: host.into(),
        path: None,
        protocol: Protocol::Http,
        target,
        listen_port: None,
        https_only: false,
        strip_path: false,
        note: String::new(),
        owner_pid: None,
        persistent: false,
    }
}

fn test_limits() -> Limits {
    Limits { hold: 1 << 20, copy: 1 << 20, budget: 64 << 20, ..Limits::default() }
}

async fn harness_with(limits: Limits) -> Harness {
    let sent: Sent = Arc::default();
    let up = upstream(sent.clone()).await;
    let mut table = RouteTable::new();
    table.insert(route("shop", format!("http://127.0.0.1:{}", up.port())));
    table.insert(route("other", format!("http://127.0.0.1:{}", up.port())));
    let log = Arc::new(RequestLog::new(100));
    let scripts = Scripts::with_limits(limits);
    let proxy = Arc::new(Proxy {
        instance: Instance::release(),
        routes: Arc::new(Table(RwLock::new(table))),
        log: log.clone(),
        tls_client: tls::insecure_loopback_client_config(),
        status: None,
        scripts: scripts.clone(),
        har: None,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, peer) = listener.accept().await.unwrap();
            tokio::spawn(proxy.clone().serve(stream, ClientScheme::Http, peer));
        }
    });
    let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    Harness { http, log, scripts, dir, out, sent }
}

async fn harness() -> Harness {
    harness_with(test_limits()).await
}

impl Harness {
    /// Write `name.lua` and set a rule for it on `host`.
    fn rule(&self, id: &str, host: &str, lua: &str, edit: impl FnOnce(&mut ScriptRule)) {
        let script = self.dir.path().join(format!("{id}.lua"));
        std::fs::write(&script, lua).unwrap();
        let mut rule: ScriptRule = serde_json::from_value(serde_json::json!({
            "id": id, "host": host, "script": script, "output_dir": self.out,
        }))
        .unwrap();
        edit(&mut rule);
        let loaded = load_file(&script);
        if let Ok(l) = &loaded
            && l.info.kind == localrouter_core::scripts::engine::ScriptKind::Intercept
        {
            rule.output_dir = None;
        }
        self.scripts.put(rule, Ok(loaded.unwrap_or_else(|e| panic!("{id}: {e}"))));
    }

    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.out.join(name)).unwrap_or_default()
    }

    /// The lines of a capture file, once it has `n`.
    async fn lines(&self, name: &str, n: usize) -> Vec<serde_json::Value> {
        for _ in 0..300 {
            let text = self.file(name);
            let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
            if lines.len() >= n {
                return lines;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{name} never got {n} lines: {:?}; rules: {:?}", self.file(name), self.scripts.views());
    }
}

struct Got {
    status: StatusCode,
    headers: hyper::HeaderMap,
    body: Bytes,
}

impl Got {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text()))
    }
}

async fn send(addr: SocketAddr, method: &str, host: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Got {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let mut req = Request::builder().method(method).uri(path).header("host", host);
    for (n, v) in headers {
        req = req.header(*n, *v);
    }
    let resp = sender.send_request(req.body(Full::new(Bytes::copy_from_slice(body))).unwrap()).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    Got { status, headers, body }
}

async fn get(addr: SocketAddr, path: &str) -> Got {
    send(addr, "GET", "shop.localhost", path, &[], b"").await
}

/// GET a stream; returns each data frame with the time it arrived.
async fn get_stream(addr: SocketAddr, path: &str) -> Vec<(Instant, Bytes)> {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let req = Request::get(path).header("host", "shop.localhost").body(Full::new(Bytes::new())).unwrap();
    let mut body = sender.send_request(req).await.unwrap().into_body();
    let mut frames = vec![];
    while let Some(Ok(frame)) = body.frame().await {
        if let Ok(data) = frame.into_data() {
            frames.push((Instant::now(), data));
        }
    }
    frames
}

fn joined(frames: &[(Instant, Bytes)]) -> String {
    frames.iter().map(|(_, b)| String::from_utf8_lossy(b).into_owned()).collect()
}

// ---- T5: on_request

#[tokio::test]
async fn on_request_changes_the_request_and_can_answer() {
    let h = harness().await;
    h.rule("a-trace", "shop.localhost", r#"
        return {
          kind = "intercept",
          request_body = { "text" },
          on_request = function(req)
            req.headers["x-trace"] = "lr-" .. #req.id
            if req.path == "/v1/admin" then
              return { status = 403, headers = { ["content-type"] = "text/plain" }, body = "blocked by a-trace.lua" }
            end
            if req.path == "/old" then req.path = "/new"; req.query = "x=2" end
            if req.body then
              local data = json.decode(req.body)
              data.max_tokens = 256
              req.body = json.encode(data)
            end
          end,
        }"#, |r| r.order = 10);
    h.rule("b-second", "shop.localhost", r#"
        return { kind = "intercept", on_request = function(req)
          req.headers["x-order"] = (req.headers["x-trace"] and "after-trace") or "first"
        end }"#, |r| r.order = 20);

    let echo = get(h.http, "/old?x=1").await.json();
    assert_eq!(echo["path"], "/new?x=2");
    assert_eq!(echo["headers"]["x-trace"], "lr-26");
    assert_eq!(echo["headers"]["x-order"], "after-trace", "order 10 runs before order 20");

    let body = br#"{"model":"m","max_tokens":4096}"#;
    let gz = gzip(body);
    let got = send(h.http, "POST", "shop.localhost", "/v1/messages", &[("content-type", "application/json"), ("content-encoding", "gzip")], &gz).await;
    let echo = got.json();
    let sent: serde_json::Value = serde_json::from_str(echo["body"].as_str().unwrap()).unwrap();
    assert_eq!(sent["max_tokens"], 256, "the script got the body decoded and changed it");
    assert_eq!(echo["headers"]["content-length"], sent.to_string().len().to_string());
    assert!(echo["headers"].get("content-encoding").is_none(), "a new body goes without Content-Encoding");

    let got = get(h.http, "/v1/admin").await;
    assert_eq!(got.status, StatusCode::FORBIDDEN);
    assert_eq!(got.text(), "blocked by a-trace.lua");
    let entry = h.log.recent(None, 1).pop().unwrap();
    assert!(matches!(&entry, LogEntry::Http { rules, status: 403, .. } if rules == &["a-trace", "b-second"]), "{entry:?}");
    let views = h.scripts.views();
    assert_eq!(views[0].answered, 1);
    assert_eq!(views[0].matched, 3);
}

// ---- T6: on_response

#[tokio::test]
async fn on_response_changes_status_headers_and_a_held_body() {
    let h = harness().await;
    h.rule("res", "shop.localhost", r#"
        return {
          kind = "intercept",
          response_body = true,
          on_response = function(req, res)
            res.status = 299
            res.headers["x-seen-by"] = "localrouter"
            if req.path == "/gzip-json" then
              local data = json.decode(res.body)
              data.hello = "script"
              res.body = json.encode(data)
            end
          end,
        }"#, |_| {});
    let got = get(h.http, "/gzip-json").await;
    assert_eq!(got.status.as_u16(), 299);
    assert_eq!(got.headers["x-seen-by"], "localrouter");
    assert!(got.headers.get("content-encoding").is_none());
    assert_eq!(got.json()["hello"], "script", "the script read the gzip body decoded");
}

// ---- T7: holding

#[tokio::test]
async fn streaming_is_kept_unless_a_rule_holds_the_body() {
    let h = harness().await;
    // A rule that matches but holds nothing: events still stream.
    h.rule("hdr", "shop.localhost", r#"return { kind = "intercept", on_response = function(req, res) res.headers["x-a"] = "1" end }"#, |_| {});
    h.sent.lock().unwrap().clear();
    let frames = get_stream(h.http, "/sse").await;
    let sent = h.sent.lock().unwrap().clone();
    assert!(frames[0].0 < sent[1], "the client had event 1 before the server sent event 2");

    // With response_body = true, a text body is held; an events body is
    // never held (I21), so test it on a text body: the 2 MB body is over the
    // 1 MB test limit and passes unchanged, with body = nil.
    h.rule("hdr", "shop.localhost", r#"
        return { kind = "intercept", response_body = { "text" }, on_response = function(req, res)
          res.headers["x-body"] = res.body and "held" or ("nil " .. tostring(res.body_skipped) .. " " .. tostring(res.body_truncated))
        end }"#, |_| {});
    let got = get(h.http, "/big").await;
    assert_eq!(got.headers["x-body"], "nil too_large true");
    assert_eq!(got.body.len(), 2 << 20, "the client got every byte");
    let got = get(h.http, "/gzip-json").await;
    assert_eq!(got.headers["x-body"], "held");
    assert_eq!(got.headers["content-encoding"], "gzip", "an unchanged body is sent as it came");
}

// ---- T8: log rules

const CAPTURE: &str = r#"
    return {
      kind = "log",
      on_exchange = function(ex)
        capture.append("x.jsonl", json.encode({
          path = ex.request.path, status = ex.response.status, class = ex.response.body_class,
          size = ex.response.body_size, skipped = ex.response.body_skipped, file = ex.response.body_file,
          len = ex.response.body and #ex.response.body or json.null, truncated = ex.response.body_truncated,
          range = ex.response.range, request = ex.request.body, auth = ex.request.headers.authorization,
          id = ex.request.id, answered_by = ex.answered_by, file_truncated = ex.response.body_file_truncated,
        }) .. "\n")
      end,
    }"#;

#[tokio::test]
async fn a_log_rule_never_delays_or_changes_the_stream() {
    let h = harness().await;
    h.rule("cap", "shop.localhost", CAPTURE, |_| {});
    h.sent.lock().unwrap().clear();
    let frames = get_stream(h.http, "/sse").await;
    let sent = h.sent.lock().unwrap().clone();
    assert!(frames[0].0 < sent[1], "I2: event 1 arrived before event 2 was sent");
    let text = joined(&frames);
    assert_eq!(text, "data: one\n\nevent: ping\ndata: x\n\ndata: two\n\ndata: three\n\n", "byte-equal");
    let lines = h.lines("x.jsonl", 1).await;
    assert_eq!(lines[0]["len"], text.len(), "the copy holds the whole stream");
    assert_eq!(lines[0]["class"], "events");
}

#[tokio::test]
async fn a_slow_log_script_does_not_delay_the_next_request_and_sees_exchanges_in_order() {
    let h = harness().await;
    h.rule("slow", "shop.localhost", r#"
        return { kind = "log", on_exchange = function(ex)
          local t = os.clock() while os.clock() - t < 0.3 do end
          capture.append("order.txt", ex.request.path .. "\n")
        end }"#, |_| {});
    let started = Instant::now();
    for i in 0..5 {
        get(h.http, &format!("/n{i}")).await;
    }
    assert!(started.elapsed() < Duration::from_millis(600), "requests waited for the log script: {:?}", started.elapsed());
    for _ in 0..200 {
        if h.file("order.txt").lines().count() == 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(h.file("order.txt"), "/n0\n/n1\n/n2\n/n3\n/n4\n", "I16: in the order they ended");
}

#[tokio::test]
async fn copies_past_the_budget_are_dropped_and_counted() {
    let h = harness_with(Limits { budget: 1 << 20, copy: 4 << 20, ..test_limits() }).await;
    h.rule("cap", "shop.localhost", CAPTURE, |_| {});
    let got = get(h.http, "/big").await;
    assert_eq!(got.body.len(), 2 << 20, "the exchange is not affected");
    let lines = h.lines("x.jsonl", 1).await;
    assert_eq!(lines[0]["skipped"], "budget");
    assert_eq!(lines[0]["len"], serde_json::Value::Null);
    assert_eq!(h.scripts.budget.used(), 0, "every reserved byte came back");
}

// ---- T10: headers

#[tokio::test]
async fn scripts_see_every_header_as_it_is() {
    let h = harness().await;
    h.rule("cap", "shop.localhost", CAPTURE, |_| {});
    h.rule("swap", "shop.localhost", r#"
        return { kind = "intercept", on_request = function(req)
          if req.path == "/swap" then req.headers.authorization = "Bearer new" end
          req.headers["x-saw"] = req.headers.authorization
        end }"#, |_| {});
    let echo = send(h.http, "GET", "shop.localhost", "/keep", &[("authorization", "Bearer real")], b"").await.json();
    assert_eq!(echo["headers"]["authorization"], "Bearer real");
    assert_eq!(echo["headers"]["x-saw"], "Bearer real", "the script sees the value");
    let echo = send(h.http, "GET", "shop.localhost", "/swap", &[("authorization", "Bearer real")], b"").await.json();
    assert_eq!(echo["headers"]["authorization"], "Bearer new");
    let lines = h.lines("x.jsonl", 2).await;
    assert_eq!(lines[0]["auth"], "Bearer real", "a log script sees the value");
}

// ---- T12: errors

#[tokio::test]
async fn on_error_fail_gives_a_502_and_pass_goes_on_and_20_failures_turn_the_rule_off() {
    let h = harness().await;
    h.rule("boom", "shop.localhost", "return { kind = 'intercept', on_request = function(req)\n  error('x')\nend }", |_| {});
    let got = get(h.http, "/a").await;
    assert_eq!(got.status, StatusCode::BAD_GATEWAY);
    assert!(got.text().contains("boom") && got.text().contains("boom.lua:2: x"), "{}", got.text());
    let entry = h.log.recent(None, 1).pop().unwrap();
    assert!(matches!(&entry, LogEntry::Http { script_error: Some(id), .. } if id == "boom"), "{entry:?}");

    h.rule("boom", "shop.localhost", "return { kind = 'intercept', on_request = function(req) error('x') end }", |r| {
        r.on_error = Some(localrouter_core::scripts::rules::OnError::Pass)
    });
    let got = get(h.http, "/a").await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(got.json()["path"], "/a", "the request went on unchanged");

    for _ in 0..19 {
        get(h.http, "/a").await;
    }
    let v = h.scripts.view("boom").unwrap();
    assert!(!v.rule.enabled, "I6: off after 20 failures in a row");
    assert!(v.last_error.unwrap().message.contains("disabled after 20"));
    let calls = h.scripts.stats.lua_calls.load(Ordering::Relaxed);
    get(h.http, "/a").await;
    assert_eq!(h.scripts.stats.lua_calls.load(Ordering::Relaxed), calls, "request 21 runs no script");
}

// ---- T18: no rule, no cost

#[tokio::test]
async fn without_a_matching_rule_nothing_is_held_copied_or_run() {
    let h = harness().await;
    for _ in 0..20 {
        get(h.http, "/x").await;
    }
    h.rule("cap", "other.localhost", CAPTURE, |_| {});
    h.rule("int", "*.example.com", r#"return { kind = "intercept", request_body = true, on_request = function() end }"#, |_| {});
    for _ in 0..80 {
        get(h.http, "/x").await;
    }
    let s = &h.scripts.stats;
    assert_eq!((s.lua_calls.load(Ordering::Relaxed), s.held_bytes.load(Ordering::Relaxed), s.copied_bytes.load(Ordering::Relaxed)), (0, 0, 0));
    let entry = h.log.recent(None, 1).pop().unwrap();
    assert!(matches!(&entry, LogEntry::Http { rules, .. } if rules.is_empty()));
}

// ---- T19: where scripts run

#[tokio::test]
async fn scripts_run_on_routes_and_never_on_the_help_page_or_the_404_page() {
    let h = harness().await;
    h.rule("nope", "nope.localhost", r#"return { kind = "intercept", on_request = function() return { status = 418 } end }"#, |_| {});
    h.rule("shop", "shop.localhost", r#"return { kind = "intercept", on_request = function(req) req.headers["x-src"] = req.source .. " " .. req.route end }"#, |_| {});
    assert_eq!(send(h.http, "GET", "nope.localhost", "/", &[], b"").await.status, StatusCode::NOT_FOUND);
    assert_eq!(send(h.http, "GET", "router.localhost", "/", &[], b"").await.status, StatusCode::OK);
    assert_eq!(get(h.http, "/").await.json()["headers"]["x-src"], "router shop");
}

// ---- T20: classes

#[tokio::test]
async fn a_default_log_rule_counts_media_and_copies_it_only_when_asked() {
    let h = harness_with(Limits { copy: 8 << 20, ..test_limits() }).await;
    h.rule("cap", "shop.localhost", CAPTURE, |_| {});
    let before = h.scripts.stats.copied_bytes.load(Ordering::Relaxed);
    assert_eq!(get(h.http, "/png").await.body.len(), PNG_SIZE);
    let lines = h.lines("x.jsonl", 1).await;
    assert_eq!((lines[0]["class"].as_str(), lines[0]["skipped"].as_str(), lines[0]["size"].as_u64()), (Some("media"), Some("class"), Some(PNG_SIZE as u64)));
    assert_eq!(h.scripts.stats.copied_bytes.load(Ordering::Relaxed), before, "I22: 0 bytes copied");

    get(h.http, "/png-octet").await;
    let lines = h.lines("x.jsonl", 2).await;
    assert_eq!(lines[1]["class"], "binary", "I26: the header decides");

    h.rule("cap", "shop.localhost", &CAPTURE.replace("kind = \"log\",", "kind = \"log\", copy = { \"media\" },"), |_| {});
    get(h.http, "/png").await;
    let lines = h.lines("x.jsonl", 3).await;
    assert_eq!(lines[2]["len"], PNG_SIZE);
}

// ---- T21: event streams

#[tokio::test]
async fn intercept_on_event_changes_and_drops_events_one_at_a_time() {
    let h = harness().await;
    h.rule("up", "shop.localhost", r#"
        return { kind = "intercept", on_event = function(req, res, event)
          if event.event == "ping" then return false end
          event.data = string.upper(event.data)
        end }"#, |_| {});
    h.sent.lock().unwrap().clear();
    let frames = get_stream(h.http, "/sse").await;
    let sent = h.sent.lock().unwrap().clone();
    assert_eq!(joined(&frames), "data: ONE\n\ndata: TWO\n\ndata: THREE\n\n");
    assert!(frames[0].0 < sent[1], "I21: event 1 reached the client before the server sent event 2");
}

#[tokio::test]
async fn log_on_event_sees_each_event_in_order_and_on_exchange_says_streamed() {
    let h = harness().await;
    h.rule("ev", "shop.localhost", r#"
        return {
          kind = "log",
          on_event = function(ex, event) capture.append("ev.jsonl", json.encode({ i = event.index, data = event.data }) .. "\n") end,
          on_exchange = function(ex) capture.append("ex.jsonl", json.encode({ skipped = ex.response.body_skipped }) .. "\n") end,
        }"#, |_| {});
    get_stream(h.http, "/ndjson").await;
    let ev = h.lines("ev.jsonl", 3).await;
    assert_eq!(ev.iter().map(|e| e["data"].as_str().unwrap().to_string()).collect::<Vec<_>>(), [r#"{"n":1}"#, r#"{"n":2}"#, r#"{"n":3}"#], "a line split across two parts is one event");
    assert_eq!(ev.iter().map(|e| e["i"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(h.lines("ex.jsonl", 1).await[0]["skipped"], "streamed");

    // A stream left open: events arrive, on_exchange has not run.
    std::fs::remove_file(h.out.join("ev.jsonl")).unwrap();
    std::fs::remove_file(h.out.join("ex.jsonl")).unwrap();
    let addr = h.http;
    let reader = tokio::spawn(async move { get_stream(addr, "/sse-open").await });
    let ev = h.lines("ev.jsonl", 2).await;
    assert_eq!(ev[1]["data"], "b");
    assert_eq!(h.file("ex.jsonl"), "", "the stream has not ended");
    reader.abort();
}

#[tokio::test]
async fn an_event_over_the_limit_passes_unchanged_and_is_counted() {
    let h = harness_with(Limits { event: 1 << 20, ..test_limits() }).await;
    h.rule("up", "shop.localhost", r#"return { kind = "intercept", on_event = function(req, res, e) e.data = "seen " .. e.data end }"#, |_| {});
    let text = joined(&get_stream(h.http, "/sse-big").await);
    assert!(text.starts_with("data: seen small\n\n"), "{}", &text[..40]);
    assert!(text.ends_with("data: seen after\n\n"));
    assert!(text.contains(&"x".repeat(2 << 20)), "the big event passed unchanged");
}

// ---- T22: saving bodies

const SAVE_MEDIA: &str = r#"
    return { kind = "log", copy = { "text" }, save = { "media" }, on_exchange = function(ex)
      capture.append("idx.jsonl", json.encode({ file = ex.response.body_file, truncated = ex.response.body_file_truncated,
        skipped = ex.response.body_skipped, range = ex.response.range, size = ex.response.body_size }) .. "\n")
    end }"#;

#[tokio::test]
async fn saved_bodies_are_files_with_the_bytes_decoded_and_private() {
    let h = harness().await;
    h.rule("save", "shop.localhost", SAVE_MEDIA, |_| {});
    let png = get(h.http, "/png").await.body;
    let idx = h.lines("idx.jsonl", 1).await;
    let file = idx[0]["file"].as_str().unwrap().to_string();
    assert!(file.starts_with("bodies/") && file.ends_with("-res.png"), "{file}");
    let path = h.out.join(&file);
    assert_eq!(std::fs::read(&path).unwrap(), png, "the file equals what the client got");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(h.out.join("bodies")).unwrap().permissions().mode() & 0o777, 0o700);

    get(h.http, "/svg-gz").await;
    let idx = h.lines("idx.jsonl", 2).await;
    let svg = std::fs::read_to_string(h.out.join(idx[1]["file"].as_str().unwrap())).unwrap();
    assert!(svg.starts_with("<svg"), "saved decoded: {svg}");
}

#[tokio::test]
async fn a_slow_disk_or_the_quota_stops_the_save_and_never_the_client() {
    let h = harness_with(Limits { save_queue: 256 << 10, save_delay: Some(Duration::from_millis(30)), ..test_limits() }).await;
    h.rule("save", "shop.localhost", SAVE_MEDIA, |_| {});
    let started = Instant::now();
    assert_eq!(get(h.http, "/png").await.body.len(), PNG_SIZE);
    assert!(started.elapsed() < Duration::from_secs(3), "the client waited for the disk: {:?}", started.elapsed());
    let idx = h.lines("idx.jsonl", 1).await;
    assert_eq!((idx[0]["truncated"].as_bool(), idx[0]["skipped"].as_str()), (Some(true), Some("slow_disk")));
    let saved = std::fs::metadata(h.out.join(idx[0]["file"].as_str().unwrap())).unwrap().len();
    assert!(saved < PNG_SIZE as u64, "{saved}");

    // The quota covers saved bodies and capture files together: the body
    // fills it, so the script's own index line is refused too.
    let h = harness().await;
    h.rule("save", "shop.localhost", SAVE_MEDIA, |r| r.max_capture_bytes = Some(1 << 20));
    assert_eq!(get(h.http, "/png").await.body.len(), PNG_SIZE);
    let mut view = h.scripts.view("save").unwrap();
    for _ in 0..200 {
        if view.last_error.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        view = h.scripts.view("save").unwrap();
    }
    assert!(view.last_error.unwrap().message.contains("capture quota reached"));
    assert_eq!(view.bytes_written, 1 << 20);
    let files: Vec<_> = std::fs::read_dir(h.out.join("bodies")).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].metadata().unwrap().len(), 1 << 20, "the file stops at the quota");
}

#[tokio::test]
async fn a_bodies_link_to_another_folder_is_never_written() {
    let h = harness().await;
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), h.out.join("bodies")).unwrap();
    h.rule("save", "shop.localhost", SAVE_MEDIA, |_| {});
    assert_eq!(get(h.http, "/png").await.body.len(), PNG_SIZE);
    let idx = h.lines("idx.jsonl", 1).await;
    assert_eq!(idx[0]["skipped"], "save_error");
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

// ---- T23: Range and 206

#[tokio::test]
async fn partial_bodies_are_copied_as_parts_and_cannot_be_changed() {
    let h = harness().await;
    h.rule("save", "shop.localhost", SAVE_MEDIA, |_| {});
    let got = send(h.http, "GET", "shop.localhost", "/video", &[("range", "bytes=1000-1999")], b"").await;
    assert_eq!((got.status, got.body.len()), (StatusCode::PARTIAL_CONTENT, 1000));
    let idx = h.lines("idx.jsonl", 1).await;
    assert_eq!(idx[0]["range"], format!("bytes 1000-1999/{VIDEO_SIZE}"));
    assert!(idx[0]["file"].as_str().unwrap().ends_with("-res.part.mp4"));
    assert_eq!(std::fs::read(h.out.join(idx[0]["file"].as_str().unwrap())).unwrap(), got.body);

    h.rule("change", "shop.localhost", r#"
        return { kind = "intercept", response_body = { "media" }, on_response = function(req, res)
          res.headers["x-range"] = res.range
          if req.query == "body" then res.body = "nope" end
        end }"#, |r| r.on_error = Some(localrouter_core::scripts::rules::OnError::Pass));
    let got = send(h.http, "GET", "shop.localhost", "/video", &[("range", "bytes=0-99")], b"").await;
    assert_eq!(got.headers["x-range"], format!("bytes 0-99/{VIDEO_SIZE}"), "a header change works");
    let got = send(h.http, "GET", "shop.localhost", "/video?body", &[("range", "bytes=0-99")], b"").await;
    assert_eq!(got.body.len(), 100, "the client got the original part");
    assert!(h.scripts.view("change").unwrap().last_error.unwrap().message.contains("cannot change a partial body"));
}

// ---- T24: decompression bombs

#[tokio::test]
async fn a_small_gzip_body_stops_at_each_limit() {
    let h = harness().await;
    h.rule("cap", "shop.localhost", CAPTURE, |_| {});
    h.rule("hold", "shop.localhost", r#"
        return { kind = "intercept", response_body = true, on_response = function(req, res)
          res.headers["x-held"] = tostring(res.body ~= nil) .. " " .. tostring(res.body_skipped)
        end }"#, |_| {});
    let got = get(h.http, "/bomb").await;
    assert_eq!(got.headers["x-held"], "false too_large", "not held past the 1 MB test limit");
    let lines = h.lines("x.jsonl", 1).await;
    assert_eq!((lines[0]["len"].as_u64(), lines[0]["truncated"].as_bool()), (Some(1 << 20), Some(true)), "the copy stops at the copy limit");
    assert!(h.scripts.budget.used() < 8 << 20);
}

// ---- the forward proxy

#[tokio::test]
async fn proxy_traffic_in_absolute_form_runs_rules_for_its_host() {
    use localrouter_core::forward::ForwardProxy;
    use localrouter_core::upstream::{BoxFuture, Resolve, Upstream};
    struct To(SocketAddr);
    impl Resolve for To {
        fn resolve(&self, _: &str, _: u16) -> BoxFuture<std::io::Result<Vec<SocketAddr>>> {
            let a = self.0;
            Box::pin(async move { Ok(vec![a]) })
        }
    }
    let h = harness().await;
    let up = upstream(Arc::default()).await;
    let router = Arc::new(Proxy {
        instance: Instance::release(),
        routes: Arc::new(Table(RwLock::new(RouteTable::new()))),
        log: h.log.clone(),
        tls_client: tls::insecure_loopback_client_config(),
        status: None,
        scripts: h.scripts.clone(),
        har: None,
    });
    let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    let paths = localrouter_core::paths::Paths::under(dir.path().to_path_buf());
    let ca = match tls::LocalCa::load_or_create(&paths, &Instance::release()) {
        tls::CaLoad::Ready(ca) => *ca,
        tls::CaLoad::Broken(w) => panic!("{w}"),
    };
    let forward = Arc::new(ForwardProxy {
        instance: Instance::release(),
        router,
        upstream: Arc::new(Upstream::new(localrouter_core::upstream::platform_tls().unwrap(), Arc::new(To(up)))),
        log: h.log.clone(),
        local_certs: Arc::new(tls::CertStore::new(Some(ca), Arc::new(|_: &str| false))),
        inspect_certs: Arc::new(tls::CertStore::inspection(None, Arc::new(|_: &str| false))),
        own_ports: Arc::new(std::sync::RwLock::new(vec![])),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, peer) = listener.accept().await.unwrap();
            let at = localrouter_core::forward::ClientPort { client: None, port: addr.port() };
            tokio::spawn(forward.clone().serve(stream, peer, at, tokio_util::sync::CancellationToken::new()));
        }
    });
    h.rule("api", "api.test.example", r#"
        return { kind = "intercept", on_request = function(req)
          req.headers["x-src"] = req.source .. " " .. req.scheme .. " " .. req.host .. " " .. req.port
          req.path = "/changed"
        end }"#, |_| {});
    let echo = send(addr, "GET", "api.test.example", "http://api.test.example/v1?x=1", &[], b"").await.json();
    assert_eq!(echo["headers"]["x-src"], "proxy http api.test.example 80");
    assert_eq!(echo["path"], "/changed?x=1");
    let echo = send(addr, "GET", "other.test.example", "http://other.test.example/v1", &[], b"").await.json();
    assert!(echo["headers"].get("x-src").is_none(), "a rule acts only on its host");
}


// ---- fixes after review (each test failed before its fix)

/// GET a stream; true when the body ended with an error (the stream was cut).
async fn stream_was_cut(addr: SocketAddr, path: &str) -> bool {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let req = Request::get(path).header("host", "shop.localhost").body(Full::new(Bytes::new())).unwrap();
    let mut body = sender.send_request(req).await.unwrap().into_body();
    loop {
        match body.frame().await {
            None => return false,
            Some(Err(_)) => return true,
            Some(Ok(_)) => {}
        }
    }
}

// A held body that a log rule saves: the file has every byte, also past the
// 4 MB writer queue, because the bytes are already in memory.
#[tokio::test]
async fn a_held_body_is_saved_whole() {
    let h = harness_with(Limits { hold: 8 << 20, ..test_limits() }).await;
    h.rule("hold", "shop.localhost", r#"return { kind = "intercept", response_body = { "media" }, on_response = function() end }"#, |_| {});
    h.rule("save", "shop.localhost", SAVE_MEDIA, |_| {});
    let png = get(h.http, "/png").await.body;
    let idx = h.lines("idx.jsonl", 1).await;
    assert_eq!(idx[0]["truncated"], false, "{:?}", idx[0]);
    assert_eq!(std::fs::read(h.out.join(idx[0]["file"].as_str().unwrap())).unwrap(), png);
}

// A client that goes away while the server is quiet ends the exchange at
// once: the log rule sees it, and the budget gets its bytes back.
#[tokio::test]
async fn a_client_that_goes_away_ends_a_quiet_stream() {
    let h = harness().await;
    h.rule("cap", "shop.localhost", CAPTURE, |_| {});
    let addr = h.http;
    let reader = tokio::spawn(async move { get_stream(addr, "/sse-open").await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    reader.abort();
    let lines = h.lines("x.jsonl", 1).await;
    assert_eq!(lines[0]["path"], "/sse-open");
    // The script writes its line before the queue item that holds the copy
    // is dropped: wait for the bytes to come back.
    for _ in 0..200 {
        if h.scripts.budget.used() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(h.scripts.budget.used(), 0);
}

// The part of an event over the limit marks that event only: the small event
// before it in the same part still reaches the script, and the tail of the
// big one does not.
#[tokio::test]
async fn only_the_event_over_the_limit_is_skipped() {
    let h = harness_with(Limits { event: 1 << 20, ..test_limits() }).await;
    h.rule("up", "shop.localhost", r#"return { kind = "intercept", on_event = function(req, res, e) e.data = string.upper(e.data) end }"#, |_| {});
    let text = joined(&get_stream(h.http, "/ndjson-big").await);
    assert!(text.starts_with("{\"A\":1}\n"), "{}", &text[..20]);
    assert!(text.ends_with("x\"}\n{\"C\":3}\n"), "{}", &text[text.len() - 20..]);
}

// A failed on_request with on_error "pass" lets the request go on, so the
// response rules still run.
#[tokio::test]
async fn a_passed_failure_still_runs_on_response() {
    let h = harness().await;
    h.rule("a-fails", "shop.localhost", "return { kind = 'intercept', on_request = function() error('x') end }", |r| {
        r.on_error = Some(localrouter_core::scripts::rules::OnError::Pass)
    });
    h.rule("b-marks", "shop.localhost", r#"return { kind = "intercept", on_response = function(req, res) res.headers["x-b"] = "ran" end }"#, |_| {});
    let got = get(h.http, "/x").await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(got.headers.get("x-b").map(|v| v.to_str().unwrap()), Some("ran"));
}

// Slow log scripts do not take the threads intercept scripts need.
#[tokio::test(flavor = "multi_thread")]
async fn slow_log_scripts_do_not_starve_intercept_scripts() {
    let h = harness().await;
    for i in 0..6 {
        h.rule(&format!("slow{i}"), "shop.localhost", r#"
            return { kind = "log", on_exchange = function(ex)
              local t = os.clock() while os.clock() - t < 1.5 do end
            end }"#, |r| r.path = Some("/first".into()));
    }
    h.rule("fast", "shop.localhost", r#"return { kind = "intercept", on_request = function(req) req.headers["x-fast"] = "1" end }"#, |r| {
        r.path = Some("/second".into())
    });
    get(h.http, "/first").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let got = get(h.http, "/second").await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text());
    assert_eq!(got.json()["headers"]["x-fast"], "1");
}

// The last event, with no line end after it, is cut like any other when its
// on_event fails with on_error "fail".
#[tokio::test]
async fn a_failure_on_the_last_event_cuts_the_stream() {
    let h = harness().await;
    h.rule("boom", "shop.localhost", r#"return { kind = "intercept", on_event = function(req, res, e) if e.data:find("last") then error("x") end end }"#, |_| {});
    assert!(stream_was_cut(h.http, "/ndjson-tail").await);
}

// A body with an encoding the daemon cannot decode keeps its header and its
// bytes, also when an on_event rule matches.
#[tokio::test]
async fn an_unknown_encoding_is_passed_as_it_came() {
    let h = harness().await;
    h.rule("up", "shop.localhost", r#"return { kind = "intercept", on_event = function(req, res, e) e.data = "changed" end }"#, |_| {});
    let got = get(h.http, "/sse-two-encodings").await;
    assert_eq!(got.headers.get("content-encoding").map(|v| v.to_str().unwrap()), Some("gzip, br"));
    assert_eq!(got.text(), "data: one\n\n");
}

// A request body held for an intercept script stays in the budget while the
// exchange goes on, because the log copy still holds it.
#[tokio::test]
async fn a_held_request_body_stays_in_the_budget_until_the_exchange_ends() {
    let h = harness().await;
    h.rule("hold", "shop.localhost", r#"return { kind = "intercept", request_body = true, on_request = function() end }"#, |_| {});
    h.rule("cap", "shop.localhost", r#"return { kind = "log", copy = {}, on_exchange = function() end }"#, |_| {});
    let addr = h.http;
    let reader = tokio::spawn(async move {
        let stream = TcpStream::connect(addr).await.unwrap();
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
        tokio::spawn(conn);
        let req = Request::post("/sse-open")
            .header("host", "shop.localhost")
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(vec![b'1'; 100_000])))
            .unwrap();
        let mut body = sender.send_request(req).await.unwrap().into_body();
        while let Some(Ok(_)) = body.frame().await {}
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(h.scripts.budget.used() >= 100_000, "used {}", h.scripts.budget.used());
    reader.abort();
}
