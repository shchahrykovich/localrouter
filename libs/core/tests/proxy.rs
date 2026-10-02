//! T5: the HTTP proxy against real upstream servers on random ports.
//! ADR 03, T4 and T5: path routes, strip_path, and the route in the log.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use localrouter_core::api::{CaState, CaStatus, PortStatus, StatusResult};
use localrouter_core::logs::{LogEntry, RequestLog};
use localrouter_core::instance::Instance;
use localrouter_core::paths::Paths;
use localrouter_core::proxy::{ClientScheme, Proxy, RouteSource};
use localrouter_core::routes::{Protocol, Route, RouteTable};
use localrouter_core::tls::{self, CaLoad, CertStore, LocalCa};
use rustls::pki_types::ServerName;
use tokio::net::{TcpListener, TcpStream};

struct Table {
    routes: RwLock<RouteTable>,
    https_port: Option<u16>,
}

impl RouteSource for Table {
    fn lookup(&self, host: &str, path: &str) -> Option<Route> {
        self.routes.read().unwrap().lookup(host, path, true).cloned()
    }
    fn all(&self) -> Vec<Route> {
        self.routes.read().unwrap().iter().cloned().collect()
    }
    fn https_port(&self) -> Option<u16> {
        self.https_port
    }
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
        note: "test note".into(),
        owner_pid: None,
        persistent: false,
    }
}

/// Upstream that answers with what it received, as JSON.
async fn echo_upstream() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(serve_echo(TokioIo::new(stream)));
        }
    });
    addr
}

async fn serve_echo<IO>(io: IO)
where
    IO: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let svc = hyper::service::service_fn(|req: Request<Incoming>| async move {
        let header = |n: &str| req.headers().get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let json = serde_json::json!({
            "method": req.method().as_str(),
            "path": req.uri().to_string(),
            "host": header("host"),
            "xff": header("x-forwarded-for"),
            "xfp": header("x-forwarded-proto"),
            "xfh": header("x-forwarded-host"),
            "keep_alive": header("keep-alive"),
            "prefix": header("x-forwarded-prefix"),
        });
        Ok::<_, std::convert::Infallible>(Response::new(Full::new(Bytes::from(json.to_string()))))
    });
    let _ = hyper::server::conn::http1::Builder::new().serve_connection(io, svc).await;
}

/// HTTPS upstream with a self-signed certificate.
async fn tls_echo_upstream() -> SocketAddr {
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap().self_signed(&key).unwrap();
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into());
    let config = rustls::ServerConfig::builder_with_provider(tls::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], key_der)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(stream).await {
                    serve_echo(TokioIo::new(tls)).await;
                }
            });
        }
    });
    addr
}

/// WebSocket upstream that echoes every message.
async fn ws_upstream() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                while let Some(Ok(msg)) = ws.next().await {
                    if msg.is_text() && ws.send(msg).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

struct Harness {
    http: SocketAddr,
    log: Arc<RequestLog>,
    _dir: tempfile::TempDir,
    https: SocketAddr,
    ca_der: rustls::pki_types::CertificateDer<'static>,
}

fn fixed_status() -> StatusResult {
    let port = |p: u16| PortStatus { configured: p, port: Some(p), bound: vec![], errors: vec![] };
    StatusResult {
        daemon_version: "test".into(),
        api_version: "1.0".into(),
        pid: 1,
        data_dir: "/tmp".into(),
        http: port(80),
        https: port(443),
        ca: CaStatus { state: CaState::Ok, problem: None, pem_path: "ca.pem".into(), common_name: None, trusted: Some(true) },
        routes: 1,
        routes_file_problem: None,
        listen_failed: vec![],
        proxy: None,
        network: None,
        notes: vec![],
    }
}

async fn harness(routes: Vec<Route>) -> Harness {
    let mut table = RouteTable::new();
    for r in routes {
        table.insert(r);
    }
    let https_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let https = https_listener.local_addr().unwrap();
    let source = Arc::new(Table { routes: RwLock::new(table), https_port: Some(https.port()) });
    let log = Arc::new(RequestLog::new(100));
    let proxy = Arc::new(Proxy {
        instance: Instance::release(),
        routes: source.clone(),
        log: log.clone(),
        tls_client: tls::insecure_loopback_client_config(),
        status: Some(Arc::new(|| Box::pin(async { Some(fixed_status()) }))),
        scripts: localrouter_core::scripts::Scripts::new(),
        har: None,
    });

    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = http_listener.local_addr().unwrap();
    let p = proxy.clone();
    tokio::spawn(async move {
        loop {
            let (stream, peer) = http_listener.accept().await.unwrap();
            tokio::spawn(p.clone().serve(stream, ClientScheme::Http, peer));
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let ca = match LocalCa::load_or_create(&Paths::under(dir.path().to_path_buf()), &Instance::release()) {
        CaLoad::Ready(ca) => *ca,
        CaLoad::Broken(why) => panic!("{why}"),
    };
    let ca_der = ca.cert_der().clone();
    let src = source.clone();
    let store = Arc::new(CertStore::new(Some(ca), Arc::new(move |name: &str| src.routes.read().unwrap().serves(name, true))));
    let acceptor = tokio_rustls::TlsAcceptor::from(tls::server_config(store).unwrap());
    tokio::spawn(async move {
        loop {
            let (stream, peer) = https_listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            let p = proxy.clone();
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(stream).await {
                    p.serve(tls, ClientScheme::Https, peer).await;
                }
            });
        }
    });
    Harness { http, log, _dir: dir, https, ca_der }
}

async fn get(addr: SocketAddr, host: &str, path: &str) -> (StatusCode, hyper::HeaderMap, String) {
    get_with(addr, host, path, &[]).await
}

async fn get_with(addr: SocketAddr, host: &str, path: &str, extra: &[(&str, &str)]) -> (StatusCode, hyper::HeaderMap, String) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let mut req = Request::get(path).header("host", host).header("keep-alive", "timeout=5");
    for (name, value) in extra {
        req = req.header(*name, *value);
    }
    let req = req.body(Empty::<Bytes>::new()).unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"))
}

#[tokio::test]
async fn forwards_get_with_host_unchanged_and_forwarded_headers() {
    let up = echo_upstream().await;
    let h = harness(vec![route("shop", format!("http://127.0.0.1:{}", up.port()))]).await;
    let (status, _, body) = get(h.http, "feat.shop.localhost", "/a/b?x=1").await;
    assert_eq!(status, StatusCode::OK);
    let j = json(&body);
    assert_eq!(j["path"], "/a/b?x=1");
    assert_eq!(j["host"], "feat.shop.localhost", "I12: Host unchanged");
    assert_eq!(j["xff"], "127.0.0.1");
    assert_eq!(j["xfp"], "http");
    assert_eq!(j["xfh"], "feat.shop.localhost");
    assert_eq!(j["keep_alive"], "", "hop-by-hop header removed");

    let entries = h.log.recent(None, 10);
    assert!(matches!(&entries[0], LogEntry::Http { path, status: 200, .. } if path == "/a/b"));
}

#[tokio::test]
async fn unknown_name_gets_404_page_listing_routes() {
    let h = harness(vec![route("shop", "http://127.0.0.1:9".into())]).await;
    let (status, _, body) = get(h.http, "blog.localhost", "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("shop.localhost"), "{body}");
}

#[tokio::test]
async fn closed_target_gets_502_with_target_and_note() {
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let h = harness(vec![route("shop", format!("http://127.0.0.1:{}", closed.port()))]).await;
    let (status, _, body) = get(h.http, "shop.localhost", "/").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body.contains(&closed.port().to_string()), "{body}");
    assert!(body.contains("test note"), "{body}");
}

#[tokio::test]
async fn https_target_with_self_signed_cert_is_forwarded() {
    let up = tls_echo_upstream().await;
    let h = harness(vec![route("api", format!("https://localhost:{}", up.port()))]).await;
    let (status, _, body) = get(h.http, "api.localhost", "/x").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json(&body)["host"], "api.localhost");
}

#[tokio::test]
async fn https_only_route_redirects_plain_http() {
    let up = echo_upstream().await;
    let mut r = route("shop", format!("http://127.0.0.1:{}", up.port()));
    r.https_only = true;
    let h = harness(vec![r]).await;
    let (status, headers, _) = get(h.http, "shop.localhost", "/p?q=1").await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(headers["location"], format!("https://shop.localhost:{}/p?q=1", h.https.port()));
}

#[tokio::test]
async fn websocket_upgrade_passes_through() {
    let up = ws_upstream().await;
    let h = harness(vec![route("ws", format!("http://127.0.0.1:{}", up.port()))]).await;
    let stream = TcpStream::connect(h.http).await.unwrap();
    let req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request("ws://ws.localhost/hmr").unwrap();
    let (mut ws, resp) = tokio_tungstenite::client_async(req, stream).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SWITCHING_PROTOCOLS);
    ws.send("ping".into()).await.unwrap();
    let back = ws.next().await.unwrap().unwrap();
    assert_eq!(back.into_text().unwrap(), "ping");
}

async fn tls_client(h: &Harness, name: &str, alpn: &[&[u8]]) -> Result<tokio_rustls::client::TlsStream<TcpStream>, std::io::Error> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(h.ca_der.clone()).unwrap();
    let mut config = rustls::ClientConfig::builder_with_provider(tls::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let stream = TcpStream::connect(h.https).await.unwrap();
    connector.connect(ServerName::try_from(name.to_string()).unwrap(), stream).await
}

#[tokio::test]
async fn https_client_offering_h2_gets_http2_and_a_trusted_cert() {
    let up = echo_upstream().await;
    let h = harness(vec![route("shop", format!("http://127.0.0.1:{}", up.port()))]).await;
    let tls = tls_client(&h, "feat.shop.localhost", &[b"h2", b"http/1.1"]).await.expect("certificate trusted by the local CA");
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
    let (mut sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls)).await.unwrap();
    tokio::spawn(conn);
    let req = Request::get("https://feat.shop.localhost/h2").body(Empty::<Bytes>::new()).unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.version(), Version::HTTP_2);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let j = json(&String::from_utf8_lossy(&body));
    assert_eq!(j["host"], "feat.shop.localhost");
    assert_eq!(j["xfp"], "https");
    assert_eq!(j["path"], "/h2");
}

#[tokio::test]
async fn tls_handshake_is_refused_for_names_without_route() {
    let h = harness(vec![route("shop", "http://127.0.0.1:9".into())]).await;
    assert!(tls_client(&h, "blog.localhost", &[b"http/1.1"]).await.is_err());
}

// router.localhost: instructions for coding agents, built into the daemon.

#[tokio::test]
async fn router_name_serves_agent_instructions_as_plain_text() {
    let h = harness(vec![route("shop", "http://127.0.0.1:9".into())]).await;
    let (status, headers, body) = get(h.http, "router.localhost", "/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/plain; charset=utf-8");
    assert!(body.starts_with("# LocalRouter"), "{body}");
    assert!(body.contains("register_route"), "{body}");
    assert!(body.contains("shop.localhost"), "the page lists current routes: {body}");
    assert!(body.contains("| HTTP | on, port 80 |"), "the page shows the daemon status: {body}");
}

#[tokio::test]
async fn router_name_wins_over_a_route_with_the_same_host_key() {
    let h = harness(vec![route("router", "http://127.0.0.1:9".into())]).await;
    let (status, _, body) = get(h.http, "Router.localhost:80", "/anything").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("# LocalRouter"), "{body}");
}

#[tokio::test]
async fn router_name_is_served_over_https_without_a_route() {
    let h = harness(vec![]).await;
    let tls = tls_client(&h, "router.localhost", &[b"http/1.1"]).await.expect("router.localhost gets a certificate");
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
    tokio::spawn(conn);
    let req = Request::get("/").header("host", "router.localhost").body(Empty::<Bytes>::new()).unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).starts_with("# LocalRouter"));
}

// ADR 03: path routes.

fn at(host: &str, path: &str, target: String) -> Route {
    Route { path: Some(path.into()), ..route(host, target) }
}

fn url(addr: SocketAddr) -> String {
    format!("http://127.0.0.1:{}", addr.port())
}

#[tokio::test]
async fn path_route_gets_its_path_unchanged_and_others_go_to_the_default() {
    let main = echo_upstream().await;
    let blog = echo_upstream().await;
    let h = harness(vec![route("shop", url(main)), at("shop", "/blog", url(blog))]).await;

    let (status, _, body) = get(h.http, "shop.localhost", "/blog/x?y=1").await;
    assert_eq!(status, StatusCode::OK);
    let j = json(&body);
    assert_eq!(j["path"], "/blog/x?y=1", "I26: path and query unchanged");
    assert_eq!(j["host"], "shop.localhost", "I12: Host unchanged");
    assert_eq!(j["prefix"], "", "no X-Forwarded-Prefix without strip_path");

    // Both upstreams echo the same JSON; the log says which route answered.
    for (path, want) in [("/blog", "shop/blog"), ("/x", "shop"), ("/blogger", "shop"), ("/", "shop")] {
        let (status, _, body) = get(h.http, "shop.localhost", path).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["path"], path);
        let last = h.log.recent(None, 1).pop().unwrap();
        assert!(matches!(&last, LogEntry::Http { route: Some(r), .. } if r == want), "{path}: {last:?}");
    }
}

#[tokio::test]
async fn strip_path_removes_the_prefix_and_sets_forwarded_prefix() {
    let api = echo_upstream().await;
    let mut r = at("shop", "/api", url(api));
    r.strip_path = true;
    let h = harness(vec![r]).await;

    let (_, _, body) = get_with(h.http, "shop.localhost", "/api/users?x=1", &[("x-forwarded-prefix", "/evil")]).await;
    let j = json(&body);
    assert_eq!(j["path"], "/users?x=1");
    assert_eq!(j["prefix"], "/api", "I26: the client value is replaced");
    assert_eq!(j["host"], "shop.localhost");

    let (_, _, body) = get(h.http, "shop.localhost", "/api").await;
    assert_eq!(json(&body)["path"], "/");
}

#[tokio::test]
async fn closed_path_target_gets_its_502_and_never_the_default_route() {
    let main = echo_upstream().await;
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let h = harness(vec![route("shop", url(main)), at("shop", "/blog", url(closed))]).await;
    let (status, _, body) = get(h.http, "shop.localhost", "/blog/x").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "I25: {body}");
    assert!(body.contains(&closed.port().to_string()), "{body}");
    assert!(body.contains("shop.localhost/blog"), "{body}");
    let (status, _, _) = get(h.http, "shop.localhost", "/x").await;
    assert_eq!(status, StatusCode::OK, "the default route still works");
}

#[tokio::test]
async fn websocket_upgrade_reaches_the_path_route() {
    let main = echo_upstream().await;
    let ws = ws_upstream().await;
    let h = harness(vec![route("shop", url(main)), at("shop", "/blog", url(ws))]).await;
    let stream = TcpStream::connect(h.http).await.unwrap();
    let req = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        "ws://shop.localhost/blog/_next/webpack-hmr",
    )
    .unwrap();
    let (mut sock, resp) = tokio_tungstenite::client_async(req, stream).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SWITCHING_PROTOCOLS);
    sock.send("ping".into()).await.unwrap();
    assert_eq!(sock.next().await.unwrap().unwrap().into_text().unwrap(), "ping");
}

#[tokio::test]
async fn name_with_only_a_path_route_gets_a_certificate() {
    let up = echo_upstream().await;
    let h = harness(vec![at("shop", "/blog", url(up))]).await;
    let tls = tls_client(&h, "shop.localhost", &[b"http/1.1"]).await.expect("I27: certificate for a path-only name");
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
    tokio::spawn(conn);
    let req = Request::get("/blog/x").header("host", "shop.localhost").body(Empty::<Bytes>::new()).unwrap();
    let resp = sender.send_request(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(tls_client(&h, "blog.localhost", &[b"http/1.1"]).await.is_err(), "I5: no route, no certificate");
}

#[tokio::test]
async fn not_found_lists_path_routes_and_logs_no_route() {
    let h = harness(vec![at("shop", "/blog", "http://127.0.0.1:9".into())]).await;
    let (status, _, body) = get(h.http, "shop.localhost", "/other").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("shop.localhost/blog"), "{body}");
    let entries = h.log.recent(None, 10);
    assert!(matches!(&entries[0], LogEntry::Http { status: 404, route: None, .. }), "{entries:?}");
    let _ = get(h.http, "router.localhost", "/").await;
    let entries = h.log.recent(None, 10);
    assert!(matches!(entries.last().unwrap(), LogEntry::Http { status: 200, route: None, .. }), "{entries:?}");
}

// Folder routes: the proxy answers from a folder, with no dev server.

fn site() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("index.html"), "<h1>home</h1>").unwrap();
    std::fs::write(dir.path().join("app.js"), "console.log(1)").unwrap();
    std::fs::write(dir.path().join("sub/page.html"), "0123456789").unwrap();
    std::fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
    dir
}

fn folder_target(dir: &tempfile::TempDir) -> String {
    format!("file://{}", dir.path().display())
}

async fn send(addr: SocketAddr, req: Request<Empty<Bytes>>) -> (StatusCode, hyper::HeaderMap, String) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let resp = sender.send_request(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn folder_route_serves_files_with_type_and_no_cache() {
    let dir = site();
    let h = harness(vec![route("docs", folder_target(&dir))]).await;

    let (status, headers, body) = get(h.http, "docs.localhost", "/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "<h1>home</h1>", "index.html for the folder");
    assert_eq!(headers["content-type"], "text/html; charset=utf-8");
    assert_eq!(headers["cache-control"], "no-cache");
    assert!(headers.contains_key("last-modified"));

    let (status, headers, _) = get(h.http, "docs.localhost", "/app.js?v=2").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-type"].to_str().unwrap().starts_with("text/javascript"), "{headers:?}");

    let entries = h.log.recent(None, 10);
    assert!(matches!(&entries[0], LogEntry::Http { status: 200, route: Some(r), .. } if r == "docs"), "{entries:?}");
}

#[tokio::test]
async fn folder_route_answers_range_head_and_if_modified_since() {
    let dir = site();
    let h = harness(vec![route("docs", folder_target(&dir))]).await;

    let (status, _, body) = get_with(h.http, "docs.localhost", "/sub/page.html", &[("range", "bytes=2-4")]).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, "234");

    let head = Request::head("/sub/page.html").header("host", "docs.localhost").body(Empty::new()).unwrap();
    let (status, headers, body) = send(h.http, head).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-length"], "10");
    assert_eq!(body, "");

    let (_, headers, _) = get(h.http, "docs.localhost", "/sub/page.html").await;
    let modified = headers["last-modified"].to_str().unwrap().to_string();
    let (status, _, _) = get_with(h.http, "docs.localhost", "/sub/page.html", &[("if-modified-since", &modified)]).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn folder_route_hides_dot_files_and_refuses_other_methods() {
    let dir = site();
    let h = harness(vec![route("docs", folder_target(&dir))]).await;
    for path in ["/.env", "/%2eenv", "/../etc/passwd", "/sub/%2e%2e/%2e%2e/etc/passwd", "/nope.html"] {
        let (status, _, body) = get(h.http, "docs.localhost", path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(!body.contains("SECRET"), "{path}: {body}");
    }
    let post = Request::post("/index.html").header("host", "docs.localhost").body(Empty::new()).unwrap();
    let (status, headers, _) = send(h.http, post).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(headers["allow"], "GET, HEAD");
}

#[tokio::test]
async fn folder_path_route_maps_its_path_to_the_folder_and_adds_the_slash() {
    let dir = site();
    let up = echo_upstream().await;
    let h = harness(vec![route("shop", url(up)), at("shop", "/docs", folder_target(&dir))]).await;

    let (status, _, body) = get(h.http, "shop.localhost", "/docs/sub/page.html").await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "0123456789"));

    let (status, headers, _) = get(h.http, "shop.localhost", "/docs?x=1").await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(headers["location"], "/docs/?x=1");

    let (status, headers, body) = get(h.http, "shop.localhost", "/docs/sub/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/html; charset=utf-8");
    assert!(body.contains("<a href=\"page.html\">page.html</a>"), "a listing without index.html: {body}");
    assert!(body.contains("<a href=\"../\">"), "{body}");

    let (status, _, body) = get(h.http, "shop.localhost", "/other").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["path"], "/other", "other paths still go to the dev server");
}

#[tokio::test]
async fn folder_route_whose_folder_is_gone_gets_a_502_with_the_folder() {
    let dir = site();
    let target = folder_target(&dir);
    let gone = dir.path().display().to_string();
    drop(dir);
    let h = harness(vec![route("docs", target)]).await;
    let (status, _, body) = get(h.http, "docs.localhost", "/").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body.contains(&gone), "{body}");
    assert!(body.contains("test note"), "{body}");
}

#[tokio::test]
async fn folder_route_answers_403_with_the_privacy_hint_when_a_file_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;
    let dir = site();
    let locked = dir.path().join("sub/page.html");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let h = harness(vec![route("docs", folder_target(&dir))]).await;
    let (status, _, body) = get(h.http, "docs.localhost", "/sub/page.html").await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("Privacy &amp; Security"), "{body}");
}

// ADR 07, T17: router.localhost/scripts is the script reference.
#[tokio::test]
async fn router_name_serves_the_script_reference_at_scripts() {
    let h = harness(vec![]).await;
    let (status, headers, body) = get(h.http, "router.localhost", "/scripts").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/plain; charset=utf-8");
    assert!(body.starts_with("# LocalRouter scripts"), "{body}");
    assert!(body.contains("on_exchange"), "{body}");
}

// ---- ADR 08, T6 and T17: the proxy log viewer at proxy.localhost

mod proxy_log {
    use std::time::{Duration, SystemTime};

    use localrouter_core::har::viewer::{self, ViewerContext};
    use localrouter_core::har::{HarLog, HarRecord, HarSettings};
    use localrouter_core::logs::ProxyMode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    pub struct Viewer {
        pub http: SocketAddr,
        /// Connections here are served as if from 192.168.1.5 (I8).
        pub lan: SocketAddr,
        pub log: Arc<RequestLog>,
        pub har: Arc<HarLog>,
        pub dir: tempfile::TempDir,
    }

    pub async fn viewer(routes: Vec<Route>) -> Viewer {
        let mut table = RouteTable::new();
        for r in routes {
            table.insert(r);
        }
        let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
        let har = HarLog::new(HarSettings {
            folder: dir.path().join("logs/proxy"),
            creator: "LocalRouter".into(),
            version: "test".into(),
            enabled: true,
            file_mb: 20,
            file_requests: 5000,
        });
        let log = Arc::new(RequestLog::new(100));
        let proxy = Arc::new(Proxy {
            instance: Instance::release(),
            routes: Arc::new(Table { routes: RwLock::new(table), https_port: None }),
            log: log.clone(),
            tls_client: tls::insecure_loopback_client_config(),
            status: None,
            scripts: localrouter_core::scripts::Scripts::new(),
            har: Some(har.clone()),
        });
        let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http = http_listener.local_addr().unwrap();
        let lan_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let lan = lan_listener.local_addr().unwrap();
        let p = proxy.clone();
        tokio::spawn(async move {
            loop {
                let (stream, peer) = http_listener.accept().await.unwrap();
                tokio::spawn(p.clone().serve(stream, ClientScheme::Http, peer));
            }
        });
        tokio::spawn(async move {
            loop {
                let (stream, _) = lan_listener.accept().await.unwrap();
                let peer: SocketAddr = "192.168.1.5:50000".parse().unwrap();
                tokio::spawn(proxy.clone().serve(stream, ClientScheme::Http, peer));
            }
        });
        Viewer { http, lan, log, har, dir }
    }

    pub fn record(har: &HarLog, i: usize) {
        let mut r = HarRecord::new(SystemTime::now(), "GET", format!("http://api.example.com/{i}?k=v"), ProxyMode::Http);
        r.status = if i.is_multiple_of(2) { 200 } else { 500 };
        r.request_headers.insert("authorization", "Bearer secret".parse().unwrap());
        har.record(r);
    }

    pub async fn written(har: &HarLog, n: u64) {
        for _ in 0..500 {
            if har.written() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("only {} written", har.written());
    }

    pub async fn request(addr: SocketAddr, method: &str, host: &str, path: &str) -> (StatusCode, hyper::HeaderMap, String) {
        let stream = TcpStream::connect(addr).await.unwrap();
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
        tokio::spawn(conn);
        let req = Request::builder().method(method).uri(path).header("host", host).body(Empty::<Bytes>::new()).unwrap();
        let resp = sender.send_request(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    async fn files(v: &Viewer) -> serde_json::Value {
        json(&get(v.http, "proxy.localhost", "/api/files").await.2)
    }

    // T6: every path, and ?download=1 sets Content-Disposition.
    #[tokio::test]
    async fn proxy_log_paths() {
        let v = viewer(vec![]).await;
        record(&v.har, 0);
        written(&v.har, 1).await;
        let (status, headers, page) = get(v.http, "proxy.localhost", "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers["content-type"].to_str().unwrap().starts_with("text/html"));
        assert!(page.contains("src=\"viewer.js\"") && page.contains("href=\"viewer.css\""), "relative URLs only: {page}");
        for (path, kind) in [("/viewer.js", "text/javascript"), ("/viewer.css", "text/css")] {
            let (status, headers, _) = get(v.http, "proxy.localhost", path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(headers["content-type"].to_str().unwrap().starts_with(kind), "{path}");
        }
        let f = files(&v).await;
        assert_eq!((f["proxy"].clone(), f["log"].clone(), f["keep_files"].clone()), (json!(false), json!(true), json!(5)));
        assert_eq!(f["files"].as_array().unwrap().len(), 1);
        assert_eq!(f["files"][0]["entries"], 1);
        assert_eq!(f["files"][0]["current"], true);
        let name = f["files"][0]["name"].as_str().unwrap().to_string();

        let (status, headers, body) = get(v.http, "proxy.localhost", &format!("/files/{name}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers.get("content-disposition").is_none());
        let har = json(&body);
        assert_eq!(har["log"]["entries"][0]["request"]["url"], "http://api.example.com/0?k=v");
        assert_eq!(har["log"]["entries"][0]["request"]["headers"][0]["value"], "Bearer secret");
        let on_disk = std::fs::read_to_string(v.har.folder().join(&name)).unwrap();
        assert_eq!(body, on_disk, "the same bytes as the file");
        let (_, headers, _) = get(v.http, "proxy.localhost", &format!("/files/{name}?download=1")).await;
        assert_eq!(headers["content-disposition"], format!("attachment; filename=\"{name}\""));

        let (status, _, body) = get(v.http, "proxy.localhost", &format!("/api/entries?file={name}&limit=10")).await;
        assert_eq!(status, StatusCode::OK);
        let page = json(&body);
        assert_eq!(page["entries"].as_array().unwrap().len(), 1);
        assert_eq!(page["before"], serde_json::Value::Null);
        let (status, _, _) = get(v.http, "proxy.localhost", "/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // T6: the second address, router.localhost/proxy-log/, gives the same.
    #[tokio::test]
    async fn proxy_log_second_address() {
        let v = viewer(vec![]).await;
        let (status, headers, _) = get(v.http, "router.localhost", "/proxy-log").await;
        assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
        assert_eq!(headers["location"], "/proxy-log/");
        let (status, _, page) = get(v.http, "router.localhost", "/proxy-log/").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page, get(v.http, "proxy.localhost", "/").await.2);
        let (status, _, body) = get(v.http, "router.localhost", "/proxy-log/api/files").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["log"], true);
        let (status, _, _) = get(v.http, "router.localhost", "/proxy-logs").await;
        assert_eq!(status, StatusCode::OK, "another path is the help page");
    }

    // T6, I8: another machine gets 403 at both addresses.
    #[tokio::test]
    async fn proxy_log_loopback_only() {
        let v = viewer(vec![]).await;
        for (host, path) in [("proxy.localhost", "/"), ("proxy.localhost", "/api/files"), ("router.localhost", "/proxy-log/")] {
            let (status, headers, _) = get(v.lan, host, path).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{host}{path}");
            assert!(headers.contains_key("content-security-policy"));
            assert_eq!(get(v.http, host, path).await.0, StatusCode::OK, "{host}{path} from this Mac");
        }
    }

    // T6, I10: only GET and HEAD.
    #[tokio::test]
    async fn proxy_log_methods() {
        let v = viewer(vec![]).await;
        for method in ["POST", "PUT", "DELETE", "PATCH"] {
            let (status, headers, _) = request(v.http, method, "proxy.localhost", "/api/files").await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
            assert_eq!(headers["allow"], "GET, HEAD");
        }
        let (status, _, body) = request(v.http, "HEAD", "proxy.localhost", "/").await;
        assert_eq!((status, body.as_str()), (StatusCode::OK, ""));
    }

    // T6, I9: only our files, never a link, never another name.
    #[tokio::test]
    async fn proxy_log_names() {
        let v = viewer(vec![]).await;
        let folder = v.har.folder().to_path_buf();
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(v.dir.path().join("ca.key"), "PRIVATE KEY").unwrap();
        std::os::unix::fs::symlink(v.dir.path().join("ca.key"), folder.join("proxy-20260101-000000.har")).unwrap();
        std::fs::write(folder.join("notes.txt"), "mine").unwrap();
        std::fs::write(v.dir.path().join("config.json"), "{}").unwrap();
        for path in [
            "/files/proxy-20260101-000000.har",
            "/files/notes.txt",
            "/files/../config.json",
            "/files/%2e%2e/config.json",
            "/files/..%2fconfig.json",
            "/files/",
            "/api/entries?file=proxy-20260101-000000.har",
        ] {
            let (status, _, body) = get(v.http, "proxy.localhost", path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
            assert!(!body.contains("PRIVATE KEY"), "{path}");
        }
        let f = files(&v).await;
        assert_eq!(f["files"].as_array().unwrap().len(), 0, "a link is not listed: {f}");
    }

    // T6, I10: the security headers on every answer; no CORS header.
    #[tokio::test]
    async fn proxy_log_headers() {
        let v = viewer(vec![]).await;
        for (method, path) in [("GET", "/"), ("GET", "/viewer.js"), ("GET", "/api/files"), ("GET", "/nope"), ("POST", "/")] {
            let (_, headers, _) = request(v.http, method, "proxy.localhost", path).await;
            assert_eq!(headers["content-security-policy"], "default-src 'self'; img-src 'self' data:; frame-ancestors 'none'", "{path}");
            assert_eq!(headers["x-content-type-options"], "nosniff", "{path}");
            assert_eq!(headers["cache-control"], "no-store", "{path}");
            assert_eq!(headers["referrer-policy"], "no-referrer", "{path}");
            assert!(!headers.keys().any(|k| k.as_str().starts_with("access-control-")), "{path}: {headers:?}");
        }
    }

    // T6, I3: the current file read during writes is always complete JSON.
    #[tokio::test]
    async fn proxy_log_current_file_is_whole() {
        let v = viewer(vec![]).await;
        record(&v.har, 0);
        written(&v.har, 1).await;
        let name = v.har.current().unwrap();
        let har = v.har.clone();
        let writer = tokio::spawn(async move {
            for i in 1..2000 {
                record(&har, i);
                if i % 50 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        });
        for _ in 0..300 {
            let (status, _, body) = get(v.http, "proxy.localhost", &format!("/files/{name}")).await;
            assert_eq!(status, StatusCode::OK);
            let har: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("cut JSON ({e})"));
            assert!(!har["log"]["entries"].as_array().unwrap().is_empty());
        }
        writer.await.unwrap();
    }

    // T6, T17: the live feed sends entries and `off`; the channel exists
    // only while a page listens.
    #[tokio::test]
    async fn proxy_log_live() {
        let v = viewer(vec![]).await;
        assert!(!v.har.resources().live_channel);
        let mut stream = TcpStream::connect(v.http).await.unwrap();
        stream.write_all(b"GET /api/live HTTP/1.1\r\nHost: proxy.localhost\r\n\r\n").await.unwrap();
        let mut seen = String::new();
        let mut buf = [0u8; 4096];
        while !seen.contains(": live") {
            let n = stream.read(&mut buf).await.unwrap();
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        assert!(seen.contains("text/event-stream"), "{seen}");
        assert!(v.har.resources().live_channel);
        record(&v.har, 7);
        while !seen.contains("event: entry") {
            let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await.unwrap().unwrap();
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        assert!(seen.contains("event: file"), "a new file: {seen}");
        assert!(seen.contains("api.example.com/7"), "{seen}");
        v.har.set_enabled(false);
        while !seen.contains("event: off") {
            let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await.unwrap().unwrap();
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        drop(stream);
        v.har.set_enabled(true);
        tokio::time::sleep(Duration::from_millis(100)).await;
        record(&v.har, 8);
        written(&v.har, 2).await;
        for _ in 0..100 {
            if !v.har.resources().live_channel {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            record(&v.har, 9);
        }
        assert!(!v.har.resources().live_channel, "the channel goes with the last page");
    }

    // T6, I7: the viewer is never logged and never a HAR entry.
    #[tokio::test]
    async fn proxy_log_not_logged() {
        let v = viewer(vec![]).await;
        for path in ["/", "/api/files", "/viewer.js"] {
            get(v.http, "proxy.localhost", path).await;
        }
        get(v.http, "router.localhost", "/proxy-log/").await;
        assert!(v.log.is_empty(), "{:?}", v.log.recent(None, 10));
        assert_eq!(v.har.written(), 0);
    }

    // T7, I11: a route `proxy` (saved before ADR 08) wins over the viewer;
    // the second address still works.
    #[tokio::test]
    async fn a_saved_proxy_route_wins_over_the_viewer() {
        let up = echo_upstream().await;
        let v = viewer(vec![route("proxy", format!("http://127.0.0.1:{}", up.port()))]).await;
        let (status, _, body) = get(v.http, "proxy.localhost", "/x").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json(&body)["path"], "/x", "the dev server answered");
        let (status, _, page) = get(v.http, "router.localhost", "/proxy-log/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(page.contains("viewer.js"));
    }

    // T6, I22: a 5 MB file arrives in chunks of at most 64 KB.
    #[tokio::test]
    async fn proxy_log_streams() {
        let v = viewer(vec![]).await;
        let folder = v.har.folder().to_path_buf();
        std::fs::create_dir_all(&folder).unwrap();
        let mut content = br#"{"log":{"version":"1.2","creator":{"name":"x","version":"1"},"pages":[],"entries":["#.to_vec();
        let line = format!("{{\"pad\":\"{}\"}}", "x".repeat(1000));
        for i in 0..5000 {
            content.extend_from_slice(if i == 0 { b"\n" } else { b",\n" });
            content.extend_from_slice(line.as_bytes());
        }
        content.extend_from_slice(localrouter_core::har::CLOSING);
        std::fs::write(folder.join("proxy-20260101-000000.har"), &content).unwrap();
        let uri: hyper::Uri = "/files/proxy-20260101-000000.har".parse().unwrap();
        let ctx = ViewerContext { cli: "localrouter" };
        let resp = viewer::serve(&v.har, ctx, &hyper::Method::GET, &uri, "127.0.0.1".parse().unwrap()).await;
        let mut body = resp.into_body();
        let (mut total, mut biggest, mut frames) = (0usize, 0usize, 0usize);
        while let Some(frame) = body.frame().await {
            let data = frame.unwrap().into_data().unwrap();
            total += data.len();
            biggest = biggest.max(data.len());
            frames += 1;
        }
        assert_eq!(total, content.len());
        assert!(biggest <= viewer::CHUNK, "a chunk of {biggest} bytes");
        assert!(frames >= content.len() / viewer::CHUNK, "{frames} frames");
    }

    // T6, I22: /api/entries on a large file returns the newest entries first,
    // and the cursor walks back to the start.
    #[tokio::test]
    async fn proxy_log_entries_from_the_end() {
        let v = viewer(vec![]).await;
        let folder = v.har.folder().to_path_buf();
        std::fs::create_dir_all(&folder).unwrap();
        let mut content = br#"{"log":{"version":"1.2","creator":{"name":"x","version":"1"},"pages":[],"entries":["#.to_vec();
        for i in 0..20000 {
            content.extend_from_slice(if i == 0 { b"\n" } else { b",\n" });
            content.extend_from_slice(format!("{{\"i\":{i},\"pad\":\"{}\"}}", "x".repeat(2500)).as_bytes());
        }
        content.extend_from_slice(localrouter_core::har::CLOSING);
        assert!(content.len() > 50_000_000);
        std::fs::write(folder.join("proxy-20260101-000000.har"), &content).unwrap();
        let (_, _, body) = get(v.http, "proxy.localhost", "/api/entries?file=proxy-20260101-000000.har&limit=500").await;
        let page = json(&body);
        let entries = page["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 500);
        assert_eq!((entries[0]["i"].as_u64(), entries[499]["i"].as_u64()), (Some(19999), Some(19500)));
        let before = page["before"].as_u64().unwrap();
        let (_, _, body) =
            get(v.http, "proxy.localhost", &format!("/api/entries?file=proxy-20260101-000000.har&limit=500&before={before}")).await;
        assert_eq!(json(&body)["entries"][0]["i"], 19499);
    }

    // The list has no bodies, only that they exist; /api/entry gives one
    // whole entry at the offset the list names.
    #[tokio::test]
    async fn proxy_log_list_without_bodies_and_one_whole_entry() {
        let v = viewer(vec![]).await;
        let mut r = HarRecord::new(SystemTime::now(), "POST", "http://api.example.com/v1".into(), ProxyMode::Http);
        r.status = 200;
        r.response_headers.insert("content-type", "application/json".parse().unwrap());
        r.request_body = Some(localrouter_core::har::capture::Copied { data: b"{}".to_vec(), size: 2, truncated: false });
        r.response_body = Some(localrouter_core::har::capture::Copied { data: br#"{"ok":1}"#.to_vec(), size: 8, truncated: false });
        v.har.record(r);
        written(&v.har, 1).await;
        let name = files(&v).await["files"][0]["name"].as_str().unwrap().to_string();
        let (_, _, body) = get(v.http, "proxy.localhost", &format!("/api/entries?file={name}")).await;
        let e = &json(&body)["entries"][0];
        assert!(e["response"]["content"].get("text").is_none(), "no body in the list: {e}");
        assert_eq!(e["response"]["content"]["_body"], true);
        assert_eq!(e["request"]["postData"]["_body"], true);
        let at = e["_at"].as_u64().unwrap();
        let (status, _, body) = get(v.http, "proxy.localhost", &format!("/api/entry?file={name}&at={at}")).await;
        assert_eq!(status, StatusCode::OK);
        let whole = json(&body);
        assert_eq!(whole["response"]["content"]["text"], r#"{"ok":1}"#);
        assert_eq!(whole["request"]["postData"]["text"], "{}");
        let (status, _, _) = get(v.http, "proxy.localhost", &format!("/api/entry?file={name}&at={}", at + 1)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    use serde_json::json;
}
