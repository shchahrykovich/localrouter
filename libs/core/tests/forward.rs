//! ADR 06: the forward proxy against local servers that play the internet.
//!
//! T2 (absolute form, 400, 508, `.localhost`, proxy headers), T3 (tunnel),
//! T5 (inspection: leaf only for the inspect set, never on 443), T6 (the
//! upstream certificate is checked) and T13 (no query in the log).
//! ADR 08: T4 (the HAR log records each mode; off records nothing) and T5
//! (a stalled writer never delays traffic).
//!
//! Replaced parts: the internet is local echo and TLS servers; DNS is a
//! resolver that records every name and refuses `.localhost`; the macOS trust
//! store is a root store with a test CA (real in manual tests M1 and M2).

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use localrouter_core::forward::{ClientPort, ForwardProxy};
use localrouter_core::har::{HarLog, HarSettings};
use localrouter_core::inspect::InspectSet;
use localrouter_core::instance::Instance;
use localrouter_core::logs::{LogEntry, ProxyMode, RequestLog, Via};
use localrouter_core::paths::Paths;
use localrouter_core::phone::{self, LanClient, PhoneEvent};
use localrouter_core::proxy::{Proxy, RouteSource};
use localrouter_core::routes::{Protocol, Route, RouteTable};
use localrouter_core::scripts::Scripts;
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

/// A TLS server that speaks HTTP/2 (ALPN `h2`) and, like some real servers,
/// resets an HTTP/2 stream that has a `host` header next to `:authority`. It
/// answers the HTTP version and the `host` header it got.
async fn h2_server(name: &str, issuer: &rcgen::Issuer<'static, rcgen::KeyPair>) -> SocketAddr {
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec![name.to_string()]).unwrap().signed_by(&key, issuer).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(tls::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()))
        .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(stream).await else { return };
                let svc = hyper::service::service_fn(|req: Request<Incoming>| async move {
                    let host = req.headers().get("host").map(|v| v.to_str().unwrap_or("").to_string());
                    if req.version() == hyper::Version::HTTP_2 && host.is_some() {
                        return Err(io::Error::other("host header in an HTTP/2 request"));
                    }
                    let json = serde_json::json!({ "version": format!("{:?}", req.version()), "host": host });
                    Ok(Response::new(Full::new(Bytes::from(json.to_string()))))
                });
                let builder = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
                let _ = builder.serve_connection(TokioIo::new(tls), svc).await;
            });
        }
    });
    addr
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
    /// A second port of the same proxy, for the proxy client `agent` (ADR 09).
    agent: SocketAddr,
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
    /// The proxy log (ADR 08), on, in the temp folder.
    har: Arc<HarLog>,
    scripts: Arc<Scripts>,
    /// A phone port (ADR 10) whose every connection looks like it came from
    /// 192.168.0.23, another machine.
    phone: SocketAddr,
    /// The same phone client on a second port, with the real peer (this Mac).
    phone_local: SocketAddr,
    phone_client: Arc<LanClient>,
    /// What the phone ports told the daemon.
    events: Arc<Mutex<Vec<PhoneEvent>>>,
    /// Off: the local-target rule lets the test servers on 127.0.0.1 through,
    /// so a phone request can succeed. On: the real rule (I5).
    strict: Arc<AtomicBool>,
    _dir: tempfile::TempDir,
}

/// The peer every connection to `Harness::phone` gets.
const PHONE_PEER: &str = "192.168.0.23:50000";
const PHONE_TOKEN: &str = "k7mq-2xph-9tdw-r4nc";

async fn harness() -> Harness {
    let echo = echo_server().await;
    let (internet_issuer, internet_ca) = test_ca();
    let (secure, _) = tls_server(&["test.example", "plain.example"], Some(&internet_issuer)).await;
    let (self_signed, self_signed_served) = tls_server(&["selfsigned.example"], None).await;
    let h2 = h2_server("h2.example", &internet_issuer).await;
    let resolver = Arc::new(TestResolver {
        names: HashMap::from([
            ("h2.example".to_string(), h2),
            ("test.example".to_string(), secure),
            ("plain.example".to_string(), secure),
            ("selfsigned.example".to_string(), self_signed),
        ]),
        asked: Mutex::new(vec![]),
    });

    let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    // A folder route: its files have a Content-Length the body ends at.
    let site = dir.path().join("site");
    std::fs::create_dir_all(&site).unwrap();
    std::fs::write(site.join("big.txt"), "x".repeat(300_000)).unwrap();
    std::fs::write(site.join("small.txt"), "hi\n").unwrap();
    let mut table = RouteTable::new();
    table.insert(route("shop", format!("http://127.0.0.1:{}", echo.port())));
    table.insert(route("site", format!("file://{}", site.display())));
    let routes = Arc::new(Table(RwLock::new(table)));
    let log = Arc::new(RequestLog::new(100));
    let scripts = Scripts::new();
    let har = HarLog::new(HarSettings {
        folder: dir.path().join("logs/proxy"),
        creator: "LocalRouter".into(),
        version: "test".into(),
        enabled: true,
        file_mb: 20,
        file_requests: 5000,
    });
    let router = Arc::new(Proxy {
        instance: Instance::release(),
        routes: routes.clone(),
        log: log.clone(),
        tls_client: tls::insecure_loopback_client_config(),
        status: None,
        scripts: scripts.clone(),
        har: Some(har.clone()),
    });

    let paths = Paths::under(dir.path().to_path_buf());
    let local = ready(LocalCa::load_or_create(&paths, &Instance::release()));
    let r = routes.clone();
    let local_store = Arc::new(CertStore::new(Some(local), Arc::new(move |n: &str| r.0.read().unwrap().serves(n, true))));
    let inspection = ready(LocalCa::load_or_create_kind(&paths, &Instance::release(), CaKind::Inspection));
    let inspection_ca = inspection.cert_der().clone();
    let set = InspectSet::new(&["test.example".into(), "selfsigned.example".into(), "h2.example".into()]);
    let inspect_store = Arc::new(CertStore::inspection(Some(inspection), Arc::new(move |n: &str| set.matches(n))));
    let extras_store = inspect_store.clone();
    let strict = Arc::new(AtomicBool::new(false));
    let rule = strict.clone();

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
    let agent_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let agent = agent_listener.local_addr().unwrap();
    let phone_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let phone = phone_listener.local_addr().unwrap();
    let phone_local_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let phone_local = phone_local_listener.local_addr().unwrap();
    let phone_client = Arc::new(LanClient::new("iphone", PHONE_TOKEN, vec![]));
    let events = Arc::new(Mutex::new(vec![]));
    let sink = events.clone();
    let forward = Arc::new(ForwardProxy {
        instance: Instance::release(),
        router,
        upstream,
        log: log.clone(),
        local_certs: local_store.clone(),
        inspect_certs: inspect_store,
        own_ports: Arc::new(RwLock::new(vec![proxy.port(), agent.port(), phone.port(), phone_local.port()])),
        local_target: Arc::new(move |ip| rule.load(Ordering::SeqCst) && phone::is_local_target(ip, &[])),
        setup_ca: Arc::new(move || extras_store.ca_certificate()),
        phone_events: Arc::new(move |e| sink.lock().unwrap().push(e)),
    });
    let fake: SocketAddr = PHONE_PEER.parse().unwrap();
    let listeners = [
        (listener, None, None, None),
        (agent_listener, Some(Arc::from("agent")), None, None),
        (phone_listener, Some(Arc::from("iphone")), Some(phone_client.clone()), Some(fake)),
        (phone_local_listener, Some(Arc::from("iphone")), Some(phone_client.clone()), None),
    ];
    for (listener, client, lan, fake_peer) in listeners {
        let forward = forward.clone();
        let at = ClientPort { client, port: listener.local_addr().unwrap().port(), lan };
        tokio::spawn(async move {
            loop {
                let (stream, peer) = listener.accept().await.unwrap();
                let peer = fake_peer.unwrap_or(peer);
                tokio::spawn(forward.clone().serve(stream, peer, at.clone(), CancellationToken::new()));
            }
        });
    }
    Harness {
        proxy,
        agent,
        log,
        resolver,
        inspection_ca,
        internet_ca,
        local_store,
        echo,
        self_signed_served,
        har,
        scripts,
        phone,
        phone_local,
        phone_client,
        events,
        strict,
        _dir: dir,
    }
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

// ---- ADR 08: the HAR log

/// The entries of the current HAR file once it has `n`, as written.
async fn har_entries(h: &Harness, n: usize) -> Vec<serde_json::Value> {
    for _ in 0..500 {
        if h.har.written() >= n as u64
            && let Some(name) = h.har.current()
        {
            let text = std::fs::read_to_string(h.har.folder().join(name)).unwrap();
            let har: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("not HAR ({e}): {text}"));
            return har["log"]["entries"].as_array().unwrap().clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the HAR file did not get {n} entries; written {}", h.har.written());
}

fn header<'a>(list: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    list.as_array()?.iter().find(|h| h["name"] == name).and_then(|h| h["value"].as_str())
}

// An HTTP/1.1 client sends `Host`; the proxy talks HTTP/2 to the server.
// The server must get `:authority` only: some servers reset a stream
// that also has `host`, and the client got a 502. An HTTP/1.1 server still
// gets its `Host`.
#[tokio::test]
async fn an_http1_client_reaches_an_http2_server_without_a_host_header() {
    let h = harness().await;
    let stream = connect(h.proxy, "h2.example:443").await.unwrap();
    let tls = tls_over(stream, "h2.example", &h.inspection_ca).await.unwrap();
    let (status, body) = get_over(tls, "h2.example", "/in").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json(&body), serde_json::json!({"version": "HTTP/2.0", "host": null}));

    let stream = connect(h.proxy, "test.example:443").await.unwrap();
    let tls = tls_over(stream, "test.example", &h.inspection_ca).await.unwrap();
    let (status, body) = get_over(tls, "test.example", "/in").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json(&body)["host"], "test.example");
}

// T4: one absolute-form GET, one inspected request, one .localhost GET and
// one tunnel: four entries, each with its mode, the query kept (U2).
#[tokio::test]
async fn har_records_each_mode() {
    let h = harness().await;
    let url = format!("http://127.0.0.1:{}/a?x=1", h.echo.port());
    let (status, _) = send(h.proxy, &url, &[("authorization", "Bearer t")]).await;
    assert_eq!(status, StatusCode::OK);
    let entries = har_entries(&h, 1).await;
    assert_eq!(entries[0]["request"]["url"], url);
    assert_eq!(entries[0]["_mode"], "http");
    assert_eq!(header(&entries[0]["request"]["headers"], "authorization"), Some("Bearer t"), "every header as it is");

    let stream = connect(h.proxy, "test.example:443").await.unwrap();
    let tls = tls_over(stream, "test.example", &h.inspection_ca).await.unwrap();
    let (status, _) = get_over(tls, "test.example", "/in?token=x").await;
    assert_eq!(status, StatusCode::OK);
    let entries = har_entries(&h, 2).await;
    assert_eq!(entries[1]["request"]["url"], "https://test.example/in?token=x");
    assert_eq!(entries[1]["_mode"], "inspect");

    let (status, _) = send(h.proxy, "http://shop.localhost/x?y=2", &[("host", "shop.localhost")]).await;
    assert_eq!(status, StatusCode::OK);
    let entries = har_entries(&h, 3).await;
    assert_eq!(entries[2]["request"]["url"], "http://shop.localhost/x?y=2");
    assert_eq!(entries[2]["_mode"], "http");
    assert_eq!(entries[2]["_route"], "shop");

    let echo = tcp_echo().await;
    let mut stream = connect(h.proxy, &format!("127.0.0.1:{}", echo.port())).await.unwrap();
    stream.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    stream.read_exact(&mut back).await.unwrap();
    drop(stream);
    let entries = har_entries(&h, 4).await;
    assert_eq!(entries[3]["request"]["method"], "CONNECT");
    assert_eq!(entries[3]["request"]["url"], format!("https://127.0.0.1:{}", echo.port()));
    assert_eq!(entries[3]["_mode"], "tunnel");
    assert_eq!(entries[3]["_bytesIn"], 4);
    assert!(entries.iter().all(|e| e.get("_client").is_none()), "the main port writes no _client (ADR 09)");
}

// A client that does not trust the inspection CA (an iPhone before Install
// CA) ends the handshake inside the CONNECT. The log must show it: it
// sent no request, so nothing else is written, and the user saw nothing.
#[tokio::test]
async fn a_refused_inspection_certificate_is_logged() {
    let h = harness().await;
    let stream = connect(h.agent, "test.example:443").await.unwrap();
    assert!(tls_over(stream, "test.example", &h.internet_ca).await.is_err(), "the client refuses the inspection leaf");
    let entries = har_entries(&h, 1).await;
    let e = &entries[0];
    assert_eq!(e["request"]["method"], "CONNECT");
    assert_eq!(e["request"]["url"], "https://test.example:443");
    assert_eq!(e["_mode"], "inspect");
    assert_eq!(e["_client"], "agent");
    let why = e["_tlsError"].as_str().unwrap_or_else(|| panic!("no _tlsError: {e}"));
    assert!(why.contains("does not trust the inspection CA"), "{why}");
}

// ADR 09: every mode on a proxy client's port names the client: absolute
// form, inspected, .localhost by absolute form and by CONNECT, a tunnel, and
// the proxy's own 508.
#[tokio::test]
async fn har_marks_every_mode_on_a_client_port() {
    let h = harness().await;
    let (status, _) = send(h.agent, &format!("http://127.0.0.1:{}/a", h.echo.port()), &[]).await;
    assert_eq!(status, StatusCode::OK);

    let stream = connect(h.agent, "test.example:443").await.unwrap();
    let tls = tls_over(stream, "test.example", &h.inspection_ca).await.unwrap();
    assert_eq!(get_over(tls, "test.example", "/in").await.0, StatusCode::OK);

    let (status, _) = send(h.agent, "http://shop.localhost/x", &[("host", "shop.localhost")]).await;
    assert_eq!(status, StatusCode::OK);

    let stream = connect(h.agent, "shop.localhost:443").await.unwrap();
    let local_ca = h.local_store.cert_for("shop.localhost").unwrap().cert[1].clone();
    let tls = tls_over(stream, "shop.localhost", &local_ca).await.unwrap();
    assert_eq!(get_over(tls, "shop.localhost", "/y").await.0, StatusCode::OK);

    let echo = tcp_echo().await;
    let mut stream = connect(h.agent, &format!("127.0.0.1:{}", echo.port())).await.unwrap();
    stream.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    stream.read_exact(&mut back).await.unwrap();
    drop(stream);

    // The main port is an address of the same proxy: a loop.
    let (status, _) = send(h.agent, &format!("http://127.0.0.1:{}/", h.proxy.port()), &[]).await;
    assert_eq!(status, StatusCode::LOOP_DETECTED);

    let entries = har_entries(&h, 6).await;
    let modes: Vec<&str> = entries.iter().map(|e| e["_mode"].as_str().unwrap()).collect();
    assert_eq!(modes.len(), 6, "{entries:?}");
    for e in &entries {
        assert_eq!(e["_client"], "agent", "{e}");
    }
}

// T4, I1: with the log off nothing is recorded and no writer starts.
#[tokio::test]
async fn har_off_records_nothing() {
    let h = harness().await;
    h.har.set_enabled(false);
    for i in 0..10 {
        send(h.proxy, &format!("http://127.0.0.1:{}/{i}", h.echo.port()), &[]).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(h.har.written(), 0);
    let r = h.har.resources();
    assert!(!r.writer_thread && !r.queue && !r.file_open, "{r:?}");
    assert!(!h.har.folder().exists(), "no file, no folder");
}

// T4: the request headers are the client's, before scripts; the response
// headers are what the client got, after scripts.
#[tokio::test]
async fn har_records_client_headers_before_scripts() {
    let h = harness().await;
    let script = h._dir.path().join("add.lua");
    std::fs::write(
        &script,
        r#"return { kind = "intercept",
             on_request = function(req) req.headers["x-from-script"] = "1" end,
             on_response = function(req, res) res.headers["x-res-from-script"] = "1" end }"#,
    )
    .unwrap();
    let rule: localrouter_core::scripts::rules::ScriptRule =
        serde_json::from_value(serde_json::json!({ "id": "add", "host": "127.0.0.1", "script": script })).unwrap();
    h.scripts.put(rule, Ok(localrouter_core::scripts::engine::load_file(&script).unwrap()));
    let (status, _) = send(h.proxy, &format!("http://127.0.0.1:{}/h", h.echo.port()), &[("x-client", "c")]).await;
    assert_eq!(status, StatusCode::OK);
    let e = &har_entries(&h, 1).await[0];
    assert_eq!(header(&e["request"]["headers"], "x-client"), Some("c"));
    assert_eq!(header(&e["request"]["headers"], "x-from-script"), None, "before scripts");
    assert_eq!(header(&e["response"]["headers"], "x-res-from-script"), Some("1"), "after scripts");
    assert_eq!(e["_scripts"], serde_json::json!(["add"]));
}

// T5, I2: a stalled writer never delays traffic: every request completes,
// and the records that did not fit are counted.
#[tokio::test]
async fn har_backpressure() {
    let h = harness().await;
    h.har.pause_for_tests(true);
    let url = format!("http://127.0.0.1:{}/b", h.echo.port());
    let mut tasks = vec![];
    for _ in 0..50 {
        let (proxy, url) = (h.proxy, url.clone());
        tasks.push(tokio::spawn(async move {
            for _ in 0..100 {
                let (status, _) = send(proxy, &url, &[]).await;
                assert_eq!(status, StatusCode::OK);
            }
        }));
    }
    let start = std::time::Instant::now();
    for t in tasks {
        t.await.unwrap();
    }
    assert!(start.elapsed() < std::time::Duration::from_secs(60), "traffic waited for the writer");
    assert!(h.har.dropped() > 0, "5000 records into a queue of 4096 with the writer paused");
    h.har.pause_for_tests(false);
}

// T6, I7: the viewer and the help page reached through the proxy are not
// HAR entries; the viewer is not in the request log either.
#[tokio::test]
async fn proxy_log_through_the_proxy_is_not_logged() {
    let h = harness().await;
    let (status, body) = send(h.proxy, "http://proxy.localhost/api/files", &[("host", "proxy.localhost")]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json(&body)["log"], true);
    send(h.proxy, "http://router.localhost/", &[("host", "router.localhost")]).await;
    send(h.proxy, "http://shop.localhost/", &[("host", "shop.localhost")]).await;
    let entries = har_entries(&h, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(h.har.written(), 1, "only shop.localhost: {entries:?}");
    let logged: Vec<String> = h.log.recent(None, 10).iter().map(|e| e.host().to_string()).collect();
    assert_eq!(logged, ["router.localhost", "shop.localhost"], "the viewer is not in the request log");
}


// ---- bodies and WebSocket messages in the proxy log

/// A raw request through the proxy, so a test controls the body.
async fn raw(proxy: SocketAddr, head: &str, body: &[u8]) -> String {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut out = vec![];
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), s.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).into_owned()
}

// The request and response bodies are in the entry, written when the
// response body ended.
#[tokio::test]
async fn har_records_request_and_response_bodies() {
    let h = harness().await;
    let port = h.echo.port();
    let body = br#"{"prompt":"hello"}"#;
    let head = format!(
        "POST http://127.0.0.1:{port}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let answer = raw(h.proxy, &head, body).await;
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    let entries = har_entries(&h, 1).await;
    let e = &entries[0];
    assert_eq!(e["request"]["postData"]["text"], r#"{"prompt":"hello"}"#);
    assert_eq!(e["request"]["postData"]["mimeType"], "application/json");
    assert_eq!(e["request"]["bodySize"], body.len());
    let text = e["response"]["content"]["text"].as_str().unwrap();
    assert!(text.contains("/v1/messages"), "the echo body: {text}");
    assert!(answer.ends_with(text), "the entry holds what the client got");
    assert!(e["_id"].as_u64().is_some());
}

// A file of a folder route that the client read to the end is a whole
// response: no `_bodyError`, every byte counted.
#[tokio::test]
async fn har_folder_route_bodies_read_to_the_end_have_no_error() {
    let h = harness().await;
    for (path, len) in [("/big.txt", 300_000), ("/small.txt", 3)] {
        let (status, body) = send(h.proxy, &format!("http://site.localhost{path}"), &[]).await;
        assert_eq!((status, body.len()), (StatusCode::OK, len), "{path}");
    }
    let entries = har_entries(&h, 2).await;
    for e in &entries {
        let url = e["request"]["url"].as_str().unwrap();
        assert!(e.get("_bodyError").is_none(), "{url}: {}", e["_bodyError"]);
        let len = if url.ends_with("/big.txt") { 300_000 } else { 3 };
        assert_eq!(e["response"]["bodySize"], len, "{url}");
    }
}

// A client that goes away in the middle of a file: the entry says so.
#[tokio::test]
async fn har_folder_route_body_cut_by_the_client_has_an_error() {
    let h = harness().await;
    std::fs::write(h._dir.path().join("site/huge.txt"), vec![b'x'; 32 << 20]).unwrap();
    let mut s = TcpStream::connect(h.proxy).await.unwrap();
    s.write_all(b"GET http://site.localhost/huge.txt HTTP/1.1\r\nHost: site.localhost\r\n\r\n").await.unwrap();
    let mut buf = vec![0u8; 64 << 10];
    s.read_exact(&mut buf).await.unwrap();
    drop(s);
    let entries = har_entries(&h, 1).await;
    assert_eq!(entries[0]["_bodyError"], "the client closed the connection before the body ended");
}

/// A WebSocket server that sends back every text and binary message.
async fn ws_echo() -> SocketAddr {
    use futures_util::{SinkExt, StreamExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                while let Some(Ok(msg)) = ws.next().await {
                    if (msg.is_text() || msg.is_binary()) && ws.send(msg).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

// A WebSocket through the proxy: its entry is written when it closes, with
// every message in both directions.
#[tokio::test]
async fn har_records_websocket_messages() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::protocol::Role;
    let h = harness().await;
    let up = ws_echo().await;
    let mut s = TcpStream::connect(h.proxy).await.unwrap();
    let port = up.port();
    s.write_all(
        format!(
            "GET http://127.0.0.1:{port}/live HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut head = vec![];
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head));
    let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(s, Role::Client, None).await;
    ws.send(Message::text("hello")).await.unwrap();
    assert_eq!(ws.next().await.unwrap().unwrap().into_text().unwrap(), "hello");
    ws.send(Message::binary(vec![1u8, 2, 3])).await.unwrap();
    assert_eq!(ws.next().await.unwrap().unwrap().into_data().to_vec(), vec![1u8, 2, 3]);
    assert_eq!(h.har.written(), 0, "an open WebSocket is not written yet");
    ws.close(None).await.unwrap();
    while ws.next().await.is_some() {}
    drop(ws);

    let entries = har_entries(&h, 1).await;
    let e = &entries[0];
    assert_eq!(e["response"]["status"], 101);
    assert_eq!(e["_resourceType"], "websocket");
    let messages: Vec<(String, u64, String)> = e["_webSocketMessages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["type"].as_str().unwrap().to_string(), m["opcode"].as_u64().unwrap(), m["data"].as_str().unwrap().to_string()))
        .collect();
    let want = |kind: &str, opcode: u64, data: &str| (kind.to_string(), opcode, data.to_string());
    assert_eq!(&messages[..4], &[want("send", 1, "hello"), want("receive", 1, "hello"), want("send", 2, "AQID"), want("receive", 2, "AQID")]);
    assert!(messages[4..].iter().any(|m| m.0 == "send" && m.1 == 8), "the close frame: {messages:?}");
}

// ---- ADR 10: a phone port

fn phone_ip() -> std::net::IpAddr {
    PHONE_PEER.parse::<SocketAddr>().unwrap().ip()
}

/// One request as written, with its response headers.
async fn request(proxy: SocketAddr, uri: &str, headers: &[(&str, &str)]) -> (StatusCode, hyper::HeaderMap, String) {
    let stream = TcpStream::connect(proxy).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    let mut req = Request::get(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = sender.send_request(req.body(Empty::<Bytes>::new()).unwrap()).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

// T3, I3: a device that is not allowed is refused for every kind of request,
// nothing is sent on, and the daemon hears that it asked.
#[tokio::test]
async fn an_unknown_device_waits_for_the_mac() {
    let h = harness().await;
    let echo = format!("http://127.0.0.1:{}/a", h.echo.port());
    let (status, _, body) = request(h.phone, &echo, &[]).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("Allow"), "{body}");
    assert_eq!(request(h.phone, "http://shop.localhost/", &[]).await.0, StatusCode::FORBIDDEN);
    let refused = connect(h.phone, "plain.example:443").await.unwrap_err();
    assert!(refused.starts_with("HTTP/1.1 403"), "{refused}");
    assert!(h.resolver.asked.lock().unwrap().is_empty(), "nothing was resolved or sent");
    let events = h.events.lock().unwrap().clone();
    assert_eq!(events.len(), 3);
    assert_eq!(events[2], PhoneEvent::Asked { client: "iphone".into(), ip: phone_ip(), host: "plain.example".into() });
    let entries = har_entries(&h, 3).await;
    assert!(entries.iter().all(|e| e["_client"] == "iphone"));
}

// T3: an allowed device uses every kind of request.
#[tokio::test]
async fn an_allowed_device_works() {
    let h = harness().await;
    h.phone_client.allow(phone_ip());
    assert_eq!(request(h.phone, &format!("http://127.0.0.1:{}/a", h.echo.port()), &[]).await.0, StatusCode::OK);
    assert_eq!(request(h.phone, "http://shop.localhost/", &[]).await.0, StatusCode::OK);
    connect(h.phone, "plain.example:443").await.unwrap();
    assert!(h.events.lock().unwrap().is_empty());
}

// T3: this Mac needs nothing on a phone port (ADR 09 behaviour).
#[tokio::test]
async fn this_mac_needs_no_allowing() {
    let h = harness().await;
    assert_eq!(request(h.phone_local, &format!("http://127.0.0.1:{}/a", h.echo.port()), &[]).await.0, StatusCode::OK);
}

// T4, I5: with the real rule an allowed device still cannot reach this
// Mac's loopback, by address or by a name that resolves to it; routes work.
#[tokio::test]
async fn local_target_is_refused_for_a_lan_peer() {
    let h = harness().await;
    h.strict.store(true, Ordering::SeqCst);
    h.phone_client.allow(phone_ip());
    let (status, _, body) = request(h.phone, &format!("http://127.0.0.1:{}/a", h.echo.port()), &[]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("is this Mac"), "{body}");
    assert_eq!(request(h.phone, "http://localhost:5432/", &[]).await.0, StatusCode::FORBIDDEN);
    assert_eq!(request(h.phone, "http://plain.example/", &[]).await.0, StatusCode::FORBIDDEN);
    let refused = connect(h.phone, &format!("[::ffff:127.0.0.1]:{}", h.echo.port())).await.unwrap_err();
    assert!(refused.starts_with("HTTP/1.1 403"), "{refused}");
    assert_eq!(request(h.phone, "http://shop.localhost/", &[]).await.0, StatusCode::OK);
    assert_eq!(request(h.phone_local, &format!("http://127.0.0.1:{}/a", h.echo.port()), &[]).await.0, StatusCode::OK);
}

// T8, I13: the setup page allows the device that opens it, shows the two
// values, and has its headers.
#[tokio::test]
async fn the_setup_page_allows_the_device() {
    let h = harness().await;
    let host = format!("192.168.0.10:{}", h.phone.port());
    let (status, headers, body) = request(h.phone, &format!("/setup/{PHONE_TOKEN}"), &[("host", &host)]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");
    assert_eq!(
        headers.get("content-security-policy").unwrap().to_str().unwrap(),
        format!("default-src 'none'; style-src 'unsafe-inline'; img-src https://{}", phone::CHECK_HOST)
    );
    assert_eq!(headers.get("referrer-policy").unwrap(), "no-referrer");
    assert!(body.contains("<span class=\"val\">192.168.0.10</span>") && body.contains(&format!("<span class=\"val\">{}</span>", h.phone.port())));
    assert!(body.contains("This iPhone (192.168.0.23) is allowed."), "{body}");
    assert!(body.contains("Waiting for the proxy"), "opened directly: the proxy is not set yet");
    assert!(h.phone_client.allows(phone_ip()));
    assert_eq!(h.events.lock().unwrap().clone(), vec![PhoneEvent::Scanned { client: "iphone".into(), ip: phone_ip() }]);
    // Now its requests go through.
    assert_eq!(request(h.phone, &format!("http://127.0.0.1:{}/a", h.echo.port()), &[]).await.0, StatusCode::OK);
    // This Mac may open the page too; it is not added as a device.
    let (status, _, body) = request(h.phone_local, &format!("/setup/{PHONE_TOKEN}"), &[("host", &host)]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("is allowed"));
    assert_eq!(h.events.lock().unwrap().len(), 1);
}

// An iPhone whose proxy is already set opens the QR page through the proxy:
// Safari sends `GET http://<mac>:<port>/setup/<token>` in absolute form, to
// this same port. It got "Not through the phone port", and the token went
// into the log.
#[tokio::test]
async fn the_setup_page_through_the_proxy_itself() {
    let h = harness().await;
    h.strict.store(true, Ordering::SeqCst);
    let base = format!("http://192.168.0.10:{}/setup/{PHONE_TOKEN}", h.phone.port());
    let host = format!("192.168.0.10:{}", h.phone.port());
    let (status, _, body) = request(h.phone, &base, &[("host", &host)]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains(&format!("192.168.0.10 : {}", h.phone.port())), "{body}");
    assert!(body.contains("This iPhone (192.168.0.23) is allowed."), "{body}");
    assert!(body.contains("ca.mobileconfig") && body.contains("Checking the certificate"), "through the proxy: the CA step");
    assert!(h.phone_client.allows(phone_ip()));
    let (status, headers, _) = request(h.phone, &format!("{base}/ca.mobileconfig"), &[("host", &host)]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("content-type").unwrap(), "application/x-apple-aspen-config");
    assert!(h.log.recent(None, 10).is_empty(), "the setup page is not logged");
    // Another port is an ordinary request, sent on.
    let other = format!("http://127.0.0.1:{}/setup/{PHONE_TOKEN}", h.echo.port());
    h.strict.store(false, Ordering::SeqCst);
    let (status, _, body) = request(h.phone, &other, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["path"], format!("/setup/{PHONE_TOKEN}"), "the echo server got it");
}

/// The phone's trust once the server side of the handshake has noted it:
/// the client can finish its side first.
async fn trust_becomes(h: &Harness, want: phone::Trust) -> phone::Trust {
    for _ in 0..200 {
        if h.phone_client.trust(phone_ip()) == want {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    h.phone_client.trust(phone_ip())
}

/// `CONNECT trust-check.invalid:443` on the phone port, then TLS that trusts
/// only `root`. The handshake result, and the status of a GET when it worked.
async fn trust_check(h: &Harness, root: &CertificateDer<'static>) -> Option<StatusCode> {
    let stream = connect(h.phone, &format!("{}:443", phone::CHECK_HOST)).await.unwrap();
    let tls = tls_over(stream, phone::CHECK_HOST, root).await.ok()?;
    Some(get_over(tls, phone::CHECK_HOST, "/1.gif").await.0)
}

// The setup page's trust check: the port answers it itself with an
// inspection leaf. A device that refuses the leaf is noted as not trusting
// the CA, one that accepts it as trusting; the page follows; nothing is
// logged and nothing is resolved.
#[tokio::test]
async fn the_trust_check_tells_the_setup_page() {
    let h = harness().await;
    h.phone_client.allow(phone_ip());
    let page = || async {
        let base = format!("http://192.168.0.10:{}/setup/{PHONE_TOKEN}", h.phone.port());
        request(h.phone, &base, &[]).await.2
    };
    assert_eq!(h.phone_client.trust(phone_ip()), phone::Trust::Unknown);

    assert_eq!(trust_check(&h, &h.internet_ca).await, None, "the client does not trust the inspection CA");
    assert_eq!(trust_becomes(&h, phone::Trust::Refused).await, phone::Trust::Refused);
    assert!(page().await.contains("not trusted yet"));

    assert_eq!(trust_check(&h, &h.inspection_ca).await, Some(StatusCode::OK));
    assert_eq!(trust_becomes(&h, phone::Trust::Trusted).await, phone::Trust::Trusted);
    let done = page().await;
    assert!(done.contains("This iPhone is connected") && !done.contains("http-equiv=\"refresh\""), "{done}");

    assert!(h.resolver.asked.lock().unwrap().is_empty(), "the check host is never resolved");
    assert!(h.log.recent(None, 10).is_empty(), "the check is not logged");
}

// Real traffic tells too: a refused inspected handshake marks the device,
// a working one clears it. Closing without a word (pinning) says nothing.
#[tokio::test]
async fn inspected_handshakes_note_the_trust() {
    let h = harness().await;
    h.phone_client.allow(phone_ip());
    let stream = connect(h.phone, "test.example:443").await.unwrap();
    assert!(tls_over(stream, "test.example", &h.internet_ca).await.is_err());
    assert_eq!(trust_becomes(&h, phone::Trust::Refused).await, phone::Trust::Refused);
    let mut stream = connect(h.phone, "test.example:443").await.unwrap();
    stream.shutdown().await.unwrap();
    drop(stream);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(h.phone_client.trust(phone_ip()), phone::Trust::Refused, "a silent close changes nothing");
    let stream = connect(h.phone, "test.example:443").await.unwrap();
    tls_over(stream, "test.example", &h.inspection_ca).await.unwrap();
    assert_eq!(trust_becomes(&h, phone::Trust::Trusted).await, phone::Trust::Trusted);
}

// An unknown device gets no trust check: it waits for the Mac like any
// request.
#[tokio::test]
async fn the_trust_check_needs_an_allowed_device() {
    let h = harness().await;
    let refused = connect(h.phone, &format!("{}:443", phone::CHECK_HOST)).await.unwrap_err();
    assert!(refused.starts_with("HTTP/1.1 403"), "{refused}");
}

// T8, I13: a wrong or old token, and every other port, get the 400 page and
// allow nobody.
#[tokio::test]
async fn setup_page_refusals() {
    let h = harness().await;
    let host = [("host", "192.168.0.10")];
    assert_eq!(request(h.phone, "/setup/wrong", &host).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(request(h.phone, "/setup/", &host).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(request(h.proxy, &format!("/setup/{PHONE_TOKEN}"), &host).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(request(h.agent, &format!("/setup/{PHONE_TOKEN}"), &host).await.0, StatusCode::BAD_REQUEST);
    h.phone_client.set_token("new-code");
    assert_eq!(request(h.phone, &format!("/setup/{PHONE_TOKEN}"), &host).await.0, StatusCode::BAD_REQUEST);
    assert!(!h.phone_client.allows(phone_ip()));
    assert_eq!(request(h.phone, "/setup/new-code", &host).await.0, StatusCode::OK);
    assert!(h.phone_client.allows(phone_ip()));
}

// T9: the CA profile, as iOS downloads it: the CA only, no Wi-Fi.
#[tokio::test]
async fn the_ca_profile() {
    let h = harness().await;
    let (status, headers, body) = request(h.phone, &format!("/setup/{PHONE_TOKEN}/ca.mobileconfig"), &[("host", "192.168.0.10")]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("content-type").unwrap(), "application/x-apple-aspen-config");
    assert!(body.contains("com.apple.security.root"));
    assert!(!body.contains("com.apple.wifi"));
}

// T10, I9: setup requests are in no log; a wrong one is logged without the
// path; no entry holds the token.
#[tokio::test]
async fn setup_not_logged() {
    let h = harness().await;
    let host = [("host", "192.168.0.10")];
    request(h.phone, &format!("/setup/{PHONE_TOKEN}"), &host).await;
    request(h.phone, &format!("/setup/{PHONE_TOKEN}/ca.mobileconfig"), &host).await;
    request(h.phone, "/setup/a-wrong-token", &host).await;
    let entries = h.log.recent(None, 10);
    assert_eq!(entries.len(), 1, "only the wrong one");
    assert_eq!(serde_json::to_value(&entries[0]).unwrap()["path"], "/setup/…");
    let har = har_entries(&h, 1).await;
    let text = serde_json::to_string(&har).unwrap() + &serde_json::to_string(&entries).unwrap();
    assert!(!text.contains(PHONE_TOKEN) && !text.contains("a-wrong-token"), "{text}");
}
