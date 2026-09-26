//! T5: the HTTP proxy against real upstream servers on random ports.

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
    fn lookup(&self, host: &str) -> Option<Route> {
        self.routes.read().unwrap().lookup(host, true).cloned()
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
        protocol: Protocol::Http,
        target,
        listen_port: None,
        https_only: false,
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
        routes: source.clone(),
        log: log.clone(),
        tls_client: tls::insecure_loopback_client_config(),
        status: Some(Arc::new(|| Box::pin(async { Some(fixed_status()) }))),
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
    let ca = match LocalCa::load_or_create(&Paths::under(dir.path().to_path_buf())) {
        CaLoad::Ready(ca) => *ca,
        CaLoad::Broken(why) => panic!("{why}"),
    };
    let ca_der = ca.cert_der().clone();
    let src = source.clone();
    let store = Arc::new(CertStore::new(Some(ca), Arc::new(move |name: &str| src.lookup(name).is_some())));
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
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let req = Request::get(path).header("host", host).header("keep-alive", "timeout=5").body(Empty::<Bytes>::new()).unwrap();
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
