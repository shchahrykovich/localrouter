//! ADR 06: the forward proxy against local servers that play the internet.
//!
//! T2 (absolute form, 400, 508, `.localhost`, proxy headers), T3 (tunnel),
//! T5 (inspection: leaf only for the inspect set, never on 443), T6 (the
//! upstream certificate is checked) and T13 (no query in the log).
//!
//! Replaced parts: the internet is local echo and TLS servers; DNS is a
//! resolver that records every name and refuses `.localhost`; the macOS trust
//! store is a root store with a test CA (real in manual tests M1 and M2).

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use localrouter_core::forward::ForwardProxy;
use localrouter_core::inspect::InspectSet;
use localrouter_core::instance::Instance;
use localrouter_core::logs::{LogEntry, ProxyMode, RequestLog, Via};
use localrouter_core::paths::Paths;
use localrouter_core::proxy::{Proxy, RouteSource};
use localrouter_core::routes::{Protocol, Route, RouteTable};
use localrouter_core::tls::{self, CaKind, CaLoad, CertStore, LocalCa};
use localrouter_core::upstream::{BoxFuture, Resolve, Upstream};
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

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

/// Test DNS: names in the map, IP literals as they are. Every name asked is
/// recorded, so a test can prove `.localhost` never got here (I8).
struct TestResolver {
    names: HashMap<String, SocketAddr>,
    asked: Mutex<Vec<String>>,
}

impl Resolve for TestResolver {
    fn resolve(&self, host: &str, port: u16) -> BoxFuture<io::Result<Vec<SocketAddr>>> {
        self.asked.lock().unwrap().push(host.to_string());
        let answer = if host.ends_with("localhost") {
            Err(io::Error::other(format!("I8: {host} reached the resolver")))
        } else if let Ok(ip) = host.parse() {
            Ok(vec![SocketAddr::new(ip, port)])
        } else {
            self.names.get(host).map(|a| vec![*a]).ok_or_else(|| io::Error::other(format!("unknown test name {host}")))
        };
        Box::pin(async move { answer })
    }
}

/// HTTP server that answers with what it received, as JSON.
async fn serve_echo<IO>(io: IO, served: Arc<AtomicUsize>)
where
    IO: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
        served.fetch_add(1, Ordering::SeqCst);
        async move {
            let header = |n: &str| req.headers().get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let json = serde_json::json!({
                "path": req.uri().to_string(),
                "host": header("host"),
                "proxy_authorization": header("proxy-authorization"),
                "proxy_connection": header("proxy-connection"),
                "via": header("via"),
                "xff": header("x-forwarded-for"),
                "xfh": header("x-forwarded-host"),
            });
            Ok::<_, std::convert::Infallible>(Response::new(Full::new(Bytes::from(json.to_string()))))
        }
    });
    let _ = hyper::server::conn::http1::Builder::new().serve_connection(io, svc).await;
}

async fn echo_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(serve_echo(TokioIo::new(stream), Arc::new(AtomicUsize::new(0))));
        }
    });
    addr
}

/// A TCP server that sends back every byte.
async fn tcp_echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let (mut r, mut w) = stream.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

/// A test CA, as an rcgen issuer and its certificate.
fn test_ca() -> (rcgen::Issuer<'static, rcgen::KeyPair>, CertificateDer<'static>) {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(vec![]).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.distinguished_name.push(rcgen::DnType::CommonName, "Test Internet CA");
    let cert = params.self_signed(&key).unwrap();
    (rcgen::Issuer::new(params, key), cert.der().clone())
}

/// HTTPS echo server for `names`, signed by `issuer` (or self-signed).
async fn tls_server(names: &[&str], issuer: Option<&rcgen::Issuer<'static, rcgen::KeyPair>>) -> (SocketAddr, Arc<AtomicUsize>) {
    let key = rcgen::KeyPair::generate().unwrap();
    let params = rcgen::CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).unwrap();
    let cert = match issuer {
        Some(issuer) => params.signed_by(&key, issuer).unwrap(),
        None => params.self_signed(&key).unwrap(),
    };
    let config = rustls::ServerConfig::builder_with_provider(tls::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()))
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = Arc::new(AtomicUsize::new(0));
    let count = served.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            let count = count.clone();
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(stream).await {
                    serve_echo(TokioIo::new(tls), count).await;
                }
            });
        }
    });
    (addr, served)
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

fn ready(load: CaLoad) -> LocalCa {
    match load {
        CaLoad::Ready(ca) => *ca,
        CaLoad::Broken(why) => panic!("{why}"),
    }
}

struct Harness {
    proxy: SocketAddr,
    log: Arc<RequestLog>,
    resolver: Arc<TestResolver>,
    /// Certificate of the inspection CA: the client trusts it.
    inspection_ca: CertificateDer<'static>,
    /// Certificate of the test internet CA.
    internet_ca: CertificateDer<'static>,
    local_store: Arc<CertStore>,
    echo: SocketAddr,
    /// Requests the self-signed server answered.
    self_signed_served: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

async fn harness() -> Harness {
    let echo = echo_server().await;
    let (internet_issuer, internet_ca) = test_ca();
    let (secure, _) = tls_server(&["test.example", "plain.example"], Some(&internet_issuer)).await;
    let (self_signed, self_signed_served) = tls_server(&["selfsigned.example"], None).await;
    let resolver = Arc::new(TestResolver {
        names: HashMap::from([
            ("test.example".to_string(), secure),
            ("plain.example".to_string(), secure),
            ("selfsigned.example".to_string(), self_signed),
        ]),
        asked: Mutex::new(vec![]),
    });

    let mut table = RouteTable::new();
    table.insert(route("shop", format!("http://127.0.0.1:{}", echo.port())));
    let routes = Arc::new(Table(RwLock::new(table)));
    let log = Arc::new(RequestLog::new(100));
    let router = Arc::new(Proxy {
        instance: Instance::release(),
        routes: routes.clone(),
        log: log.clone(),
        tls_client: tls::insecure_loopback_client_config(),
        status: None,
        scripts: localrouter_core::scripts::Scripts::new(),
    });

    let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    let paths = Paths::under(dir.path().to_path_buf());
    let local = ready(LocalCa::load_or_create(&paths, &Instance::release()));
    let r = routes.clone();
    let local_store = Arc::new(CertStore::new(Some(local), Arc::new(move |n: &str| r.0.read().unwrap().serves(n, true))));
    let inspection = ready(LocalCa::load_or_create_kind(&paths, &Instance::release(), CaKind::Inspection));
    let inspection_ca = inspection.cert_der().clone();
    let set = InspectSet::new(&["test.example".into(), "selfsigned.example".into()]);
    let inspect_store = Arc::new(CertStore::inspection(Some(inspection), Arc::new(move |n: &str| set.matches(n))));

    let mut roots = rustls::RootCertStore::empty();
    roots.add(internet_ca.clone()).unwrap();
    let client_tls = rustls::ClientConfig::builder_with_provider(tls::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let upstream = Arc::new(Upstream::new(Arc::new(client_tls), resolver.clone()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = listener.local_addr().unwrap();
    let forward = Arc::new(ForwardProxy {
        instance: Instance::release(),
        router,
        upstream,
        log: log.clone(),
        local_certs: local_store.clone(),
        inspect_certs: inspect_store,
        own_port: Arc::new(AtomicU16::new(proxy.port())),
    });
    tokio::spawn(async move {
        loop {
            let (stream, peer) = listener.accept().await.unwrap();
            tokio::spawn(forward.clone().serve(stream, peer, CancellationToken::new()));
        }
    });
    Harness { proxy, log, resolver, inspection_ca, internet_ca, local_store, echo, self_signed_served, _dir: dir }
}

/// Send one request to the proxy as written: `uri` may be absolute.
async fn send(proxy: SocketAddr, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, String) {
    let stream = TcpStream::connect(proxy).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let mut req = Request::get(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = sender.send_request(req.body(Empty::<Bytes>::new()).unwrap()).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// `CONNECT target`; returns the stream after a 200, or the status line.
async fn connect(proxy: SocketAddr, target: &str) -> Result<TcpStream, String> {
    let mut stream = TcpStream::connect(proxy).await.unwrap();
    stream.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes()).await.unwrap();
    let mut head = vec![];
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    if head.starts_with("HTTP/1.1 200") { Ok(stream) } else { Err(head) }
}

/// TLS over a CONNECT stream, trusting only `root`.
async fn tls_over(stream: TcpStream, name: &str, root: &CertificateDer<'static>) -> io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(root.clone()).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(tls::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(config)).connect(ServerName::try_from(name.to_string()).unwrap(), stream).await
}

async fn get_over<IO>(io: IO, host: &str, path: &str) -> (StatusCode, String)
where
    IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io)).await.unwrap();
    tokio::spawn(conn);
    let req = Request::get(path).header("host", host).body(Empty::<Bytes>::new()).unwrap();
    let resp = sender.send_request(req).await.unwrap();
    let status = resp.status();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"))
}

fn issuer_cn(der: &CertificateDer<'_>) -> String {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(der).unwrap();
    cert.issuer().iter_common_name().next().unwrap().as_str().unwrap().to_string()
}

// T2, I10: absolute form reaches the server with Host unchanged, proxy
// headers removed and nothing added.
#[tokio::test]
async fn absolute_form_is_sent_on_without_proxy_headers() {
    let h = harness().await;
    let url = format!("http://127.0.0.1:{}/a?x=1", h.echo.port());
    let host = format!("127.0.0.1:{}", h.echo.port());
    let (status, body) =
        send(h.proxy, &url, &[("host", &host), ("proxy-authorization", "Basic eDp5"), ("proxy-connection", "keep-alive")]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let j = json(&body);
    assert_eq!(j["path"], "/a?x=1");
    assert_eq!(j["host"], host, "Host unchanged");
    for key in ["proxy_authorization", "proxy_connection", "via", "xff", "xfh"] {
        assert_eq!(j[key], "", "{key} must not reach the server");
    }
    let entry = h.log.recent(None, 1).pop().unwrap();
    assert!(matches!(&entry, LogEntry::Http { path, via: Some(Via::Proxy), mode: Some(ProxyMode::Http), .. } if path == "/a"), "{entry:?}");
}

// T2: a request without a full URL is not proxied.
#[tokio::test]
async fn origin_form_gets_the_this_is_a_proxy_page() {
    let h = harness().await;
    let (status, body) = send(h.proxy, "/a", &[("host", "example.com")]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("This port is a proxy"), "{body}");
    assert!(body.contains("localrouter proxy env"), "{body}");
}

// T2, I9: the proxy never sends a request to itself.
#[tokio::test]
async fn own_address_is_a_loop() {
    let h = harness().await;
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let (status, _) = send(h.proxy, &format!("http://{host}:{}/", h.proxy.port()), &[]).await;
        assert_eq!(status, StatusCode::LOOP_DETECTED, "{host}");
    }
    let refused = connect(h.proxy, &format!("127.0.0.1:{}", h.proxy.port())).await.unwrap_err();
    assert!(refused.starts_with("HTTP/1.1 508"), "{refused}");
}

// T2, I8: a .localhost name goes to the route table, never to DNS.
#[tokio::test]
async fn localhost_names_are_answered_by_the_route_table() {
    let h = harness().await;
    let (status, body) = send(h.proxy, "http://shop.localhost/x", &[("host", "shop.localhost")]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json(&body)["host"], "shop.localhost");
    let (status, _) = send(h.proxy, "http://blog.localhost/", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no route: the router's 404, not DNS");

    // CONNECT shop.localhost:443: TLS with the local CA, then the router.
    let stream = connect(h.proxy, "shop.localhost:443").await.unwrap();
    let local_ca = h.local_store.cert_for("shop.localhost").unwrap().cert[1].clone();
    let tls = tls_over(stream, "shop.localhost", &local_ca).await.unwrap();
    let (status, body) = get_over(tls, "shop.localhost", "/y").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let asked = h.resolver.asked.lock().unwrap().clone();
    assert!(asked.iter().all(|n| !n.ends_with("localhost")), "resolver asked for {asked:?}");
    let entries = h.log.recent(None, 10);
    assert!(entries.iter().all(|e| matches!(e, LogEntry::Http { via: Some(Via::Proxy), .. })), "{entries:?}");
}

// T3, I7: a tunnel copies bytes unchanged both ways, and logs the bytes.
#[tokio::test]
async fn a_tunnel_copies_bytes_unchanged() {
    let h = harness().await;
    let echo = tcp_echo().await;
    let stream = connect(h.proxy, &format!("127.0.0.1:{}", echo.port())).await.unwrap();
    let mut seed: u64 = 0x9e3779b97f4a7c15;
    let data: Vec<u8> = (0..1_000_000)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect();
    let (mut r, mut w) = stream.into_split();
    let sent = data.clone();
    let writer = tokio::spawn(async move {
        w.write_all(&sent).await.unwrap();
        w.shutdown().await.unwrap();
    });
    let mut back = vec![];
    r.read_to_end(&mut back).await.unwrap();
    writer.await.unwrap();
    assert!(back == data, "bytes changed in the tunnel");

    // The entry is written when the tunnel closes.
    let mut entry = None;
    for _ in 0..100 {
        entry = h.log.recent(None, 10).into_iter().find(|e| matches!(e, LogEntry::Http { mode: Some(ProxyMode::Tunnel), .. }));
        if entry.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let entry = entry.expect("a tunnel entry");
    assert!(
        matches!(&entry, LogEntry::Http { method, path, status: 200, bytes_in: Some(1_000_000), bytes_out: Some(1_000_000), .. } if method == "CONNECT" && path.is_empty()),
        "{entry:?}"
    );
}

// T3: a server that does not answer is a 502 for the CONNECT.
#[tokio::test]
async fn a_tunnel_to_a_closed_port_is_a_502() {
    let h = harness().await;
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let refused = connect(h.proxy, &format!("127.0.0.1:{port}")).await.unwrap_err();
    assert!(refused.starts_with("HTTP/1.1 502"), "{refused}");
}

// T5, I3: a host in the inspect set gets a leaf of the inspection CA, and the
// request reaches the real server; any other host is a tunnel to the
// server's own certificate.
#[tokio::test]
async fn inspect_set_hosts_are_inspected_and_others_tunnelled() {
    let h = harness().await;

    let stream = connect(h.proxy, "test.example:443").await.unwrap();
    let tls = tls_over(stream, "test.example", &h.inspection_ca).await.expect("the inspection CA signs test.example");
    let leaf = tls.get_ref().1.peer_certificates().unwrap()[0].clone();
    assert!(issuer_cn(&leaf).starts_with("LocalRouter Inspection "), "{}", issuer_cn(&leaf));
    let (status, body) = get_over(tls, "test.example", "/in?token=x").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json(&body)["path"], "/in?token=x");
    assert_eq!(json(&body)["host"], "test.example", "Host unchanged");
    let entry = h.log.recent(None, 1).pop().unwrap();
    assert!(matches!(&entry, LogEntry::Http { path, host, mode: Some(ProxyMode::Inspect), .. } if path == "/in" && host == "test.example"), "{entry:?}");

    let stream = connect(h.proxy, "plain.example:443").await.unwrap();
    let tls = tls_over(stream, "plain.example", &h.internet_ca).await.expect("a tunnel reaches the real certificate");
    let leaf = tls.get_ref().1.peer_certificates().unwrap()[0].clone();
    assert_eq!(issuer_cn(&leaf), "Test Internet CA");
    let (status, _) = get_over(tls, "plain.example", "/").await;
    assert_eq!(status, StatusCode::OK);
}

// T5, I3: the 443 router store never signs an inspect-set name.
#[tokio::test]
async fn the_router_store_refuses_inspect_set_names() {
    let h = harness().await;
    assert!(h.local_store.cert_for("test.example").is_none());
}

// T6, I6: an upstream certificate that macOS (here: the test root store)
// does not trust gives a 502 that names the problem, and no request bytes.
#[tokio::test]
async fn an_untrusted_upstream_certificate_is_a_502() {
    let h = harness().await;
    let stream = connect(h.proxy, "selfsigned.example:443").await.unwrap();
    let tls = tls_over(stream, "selfsigned.example", &h.inspection_ca).await.unwrap();
    let (status, body) = get_over(tls, "selfsigned.example", "/secret").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body.contains("certificate is not trusted"), "{body}");
    assert!(body.contains("invalid peer certificate"), "the page names the certificate problem: {body}");
    assert_eq!(h.self_signed_served.load(Ordering::SeqCst), 0, "the server got no request");
}

// T6: https:// in absolute form is refused; clients use CONNECT.
#[tokio::test]
async fn absolute_https_is_refused() {
    let h = harness().await;
    let (status, body) = send(h.proxy, "https://test.example/", &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("CONNECT"), "{body}");
}

// T13, I11: no proxy entry holds a query string.
#[tokio::test]
async fn proxy_log_entries_hold_no_query() {
    let h = harness().await;
    send(h.proxy, &format!("http://127.0.0.1:{}/q?secret=1", h.echo.port()), &[]).await;
    let stream = connect(h.proxy, "test.example:443").await.unwrap();
    let tls = tls_over(stream, "test.example", &h.inspection_ca).await.unwrap();
    get_over(tls, "test.example", "/q?secret=2").await;
    let entries = h.log.recent(None, 10);
    assert!(entries.len() >= 2);
    let text = serde_json::to_string(&entries).unwrap();
    assert!(!text.contains("secret") && !text.contains('?'), "{text}");
}
