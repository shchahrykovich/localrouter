//! The forward proxy (ADR 06): one connection from a program that uses
//! LocalRouter as its HTTP proxy, on `127.0.0.1:<proxy_port>`.
//!
//! - `GET http://host/a` (absolute form): sent to the real server.
//! - `CONNECT host:443`: a tunnel that reads nothing (I7), or, when the host
//!   is in the inspect set, inspected with a leaf of the inspection CA (I3).
//! - A `.localhost` name never leaves the Mac: the router answers it (I8).
//! - `GET /a` (no full URL): 400. The proxy's own address: 508 (I9).
//!
//! `Proxy-Authorization` and `Proxy-Connection` never reach the server, and
//! nothing is added: no `Via`, no `X-Forwarded-*` (I10).
//!
//! Every proxy port is served here: the main one and one per proxy client
//! (ADR 09). A connection knows its port ([`ClientPort`]), and the log marks
//! its requests with the client's name.

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::upgrade::Upgraded;
use hyper::{Method, Request, Response, StatusCode, Uri, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::har::HarRecord;
use crate::har::capture::Recording;
use crate::har::websocket::{self, WsTap};
use crate::inspect::bare_host;
use crate::instance::Instance;
use crate::logs::{LogEntry, ProxyMode, RequestLog};
use crate::phone::{self, LanClient, PhoneEvent, Setup, SetupFile, Trust};
use crate::proxy::{Body, ClientScheme, FromProxy, Proxy, escape, is_upgrade, page, remove_hop_headers};
use crate::scripts::Ctx;
use crate::routes::host_key;
use crate::tls::CertStore;
use crate::upstream::Upstream;

/// A 1×1 transparent GIF: the answer to the trust check.
const PIXEL: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";

/// Time a client gets for its TLS handshake inside a `CONNECT`.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ForwardProxy {
    /// Names the CLI in the pages the proxy writes.
    pub instance: Instance,
    /// Answers `.localhost` names, as on ports 80 and 443.
    pub router: Arc<Proxy>,
    pub upstream: Arc<Upstream>,
    pub log: Arc<RequestLog>,
    /// The local CA: a `CONNECT shop.localhost:443` gets its leaf here.
    pub local_certs: Arc<CertStore>,
    /// The inspection CA. It gives a leaf only for a name in the inspect set,
    /// so a leaf from it is the decision to inspect.
    pub inspect_certs: Arc<CertStore>,
    /// Every bound proxy port, the main one and the clients', for the loop
    /// check. Empty while the proxy is off.
    pub own_ports: Arc<RwLock<Vec<u16>>>,
    /// A target another machine may not reach through a phone port (ADR 10,
    /// I5). The daemon passes [`phone::is_local_target`] with this Mac's
    /// addresses; a test can narrow it to reach a test server on loopback.
    pub local_target: Arc<dyn Fn(IpAddr) -> bool + Send + Sync>,
    /// The inspection CA (DER and name) for a phone's setup page, while
    /// HTTPS is inspected; `None` otherwise.
    pub setup_ca: Arc<dyn Fn() -> Option<(Vec<u8>, String)> + Send + Sync>,
    /// Tells the daemon that a device opened the setup page (it is allowed
    /// now) or asked to use a phone port (the user decides).
    pub phone_events: Arc<dyn Fn(PhoneEvent) + Send + Sync>,
}

/// The type of a configuration profile.
const MOBILECONFIG: &str = "application/x-apple-aspen-config";

/// The proxy port a connection came in on (ADR 09).
#[derive(Debug, Clone)]
pub struct ClientPort {
    /// The proxy client's name; `None` for the main port.
    pub client: Option<Arc<str>>,
    pub port: u16,
    /// A phone client (ADR 10): its password; the port then serves the setup
    /// paths and checks every other machine.
    pub lan: Option<Arc<LanClient>>,
}

impl ForwardProxy {
    /// Serve one accepted connection until it closes or `cancel` fires (the
    /// proxy was turned off, I2).
    pub async fn serve<IO>(self: Arc<Self>, io: IO, peer: SocketAddr, at: ClientPort, cancel: CancellationToken)
    where
        IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let this = self.clone();
        let child = cancel.clone();
        let service = hyper::service::service_fn(move |req| {
            let this = this.clone();
            let cancel = child.clone();
            let at = at.clone();
            async move { Ok::<_, Infallible>(this.handle(req, peer, &at, cancel).await) }
        });
        let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
        let conn = builder.serve_connection_with_upgrades(TokioIo::new(io), service);
        tokio::select! {
            r = conn => if let Err(e) = r { tracing::debug!("proxy connection from {peer} ended: {e}") },
            _ = cancel.cancelled() => {}
        }
    }

    async fn handle(self: &Arc<Self>, mut req: Request<Incoming>, peer: SocketAddr, at: &ClientPort, cancel: CancellationToken) -> Response<Body> {
        if let Some(lan) = &at.lan
            && let Some(resp) = self.phone_gate(&req, peer, at, lan).await
        {
            return resp;
        }
        if req.method() == Method::CONNECT {
            return self.connect(req, peer, at, cancel).await;
        }
        let start = Instant::now();
        let method = req.method().to_string();
        let path = req.uri().path_and_query().map_or("/".to_string(), |p| p.to_string());
        let Some(authority) = req.uri().authority().cloned() else {
            // A setup path holds a phone's setup token, a PAC path its PAC
            // key: never in a log (I9).
            let path = if path.starts_with("/setup/") {
                "/setup/…".to_string()
            } else if path.starts_with("/pac/") {
                "/pac/…".to_string()
            } else {
                path
            };
            let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let seen = self.seen(&req, || if host.is_empty() { path.clone() } else { format!("http://{host}{path}") }, ProxyMode::Http, at);
            let resp = self.not_a_proxy_request(at.port);
            self.log_http(&method, &host, &path, &resp, start, ProxyMode::Http, Default::default(), seen);
            return resp;
        };
        if req.uri().scheme_str() != Some("http") {
            let seen = self.seen(&req, || req.uri().to_string(), ProxyMode::Http, at);
            let resp = https_needs_connect(authority.as_str());
            self.log_http(&method, authority.as_str(), &path, &resp, start, ProxyMode::Http, Default::default(), seen);
            return resp;
        }
        let host = bare_host(authority.as_str());
        let port = authority.port_u16().unwrap_or(80);

        // A .localhost name goes to the route table, never to DNS (I8). The
        // router writes its HAR entry, as for every request with `via`.
        if host_key(&host).is_some() {
            *req.uri_mut() = path.parse::<Uri>().unwrap_or_else(|_| Uri::from_static("/"));
            if let Ok(value) = HeaderValue::from_str(authority.as_str()) {
                req.headers_mut().insert(header::HOST, value);
            }
            req.headers_mut().remove(header::PROXY_AUTHORIZATION);
            let via = FromProxy { mode: ProxyMode::Http, client: at.client.clone() };
            return self.router.handle(req, ClientScheme::Http, peer, Some(via)).await;
        }
        // What the client sent, before script rules (ADR 08).
        let seen = self.seen(&req, || req.uri().to_string(), ProxyMode::Http, at);
        if self.is_own_address(&host, port) {
            let resp = loop_detected(&host, port);
            self.log_http(&method, &host, &path, &resp, start, ProxyMode::Http, Default::default(), seen);
            return resp;
        }
        let req = req.map(|b| b.map_err(Into::into).boxed_unsync());
        let (req, rec) = self.recording(seen, req);
        let base = format!("http://{authority}");
        let ctx = Ctx { source: "proxy", route: None, scheme: "http", host: host.clone(), port };
        let (resp, scripts) = self.through_scripts(req, &base, &host, ctx).await;
        self.log_sent(&method, &host, &path, resp, start, ProxyMode::Http, scripts, rec)
    }

    /// A phone port (ADR 10). First the setup paths, for any peer, never
    /// logged (I9): opening the page with the token allows that device.
    /// Then, for another machine, a proxy request needs an allowed device
    /// (I3) and may not reach this Mac's own addresses (I5). `None` lets the
    /// request go on as on any proxy port.
    async fn phone_gate<B>(&self, req: &Request<B>, peer: SocketAddr, at: &ClientPort, lan: &Arc<LanClient>) -> Option<Response<Body>> {
        let authority = req.uri().authority().cloned();
        let ip = peer.ip().to_canonical();
        let local = ip.is_loopback();
        // The page is asked for directly, or through this same port by a
        // phone whose proxy is already set: Safari then sends
        // `GET http://<mac ip>:<port>/setup/…`. Only an IP address, so no
        // name is resolved for it.
        let to_this_port = authority.as_ref().is_none_or(|a| {
            a.port_u16() == Some(at.port) && bare_host(a.as_str()).parse::<IpAddr>().is_ok()
        });
        // The PAC file of a phone set to Automatic: any device may read it
        // (the phone's system fetches it, maybe before it is allowed); it
        // only names this port. Never logged.
        if to_this_port
            && matches!(*req.method(), Method::GET | Method::HEAD)
            && let Some(key) = phone::parse_pac_path(req.uri().path())
            && lan.is_pac(key)
        {
            let server = match req.uri().authority() {
                Some(a) => bare_host(a.as_str()),
                None => req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).map(bare_host).unwrap_or_default(),
            };
            let server = if server.contains(':') { format!("[{server}]") } else { server };
            return Some(
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/x-ns-proxy-autoconfig")
                    .header(header::CACHE_CONTROL, "no-store")
                    .body(Full::new(Bytes::from(phone::pac_file(&server, at.port, lan.paused()))).map_err(|never| match never {}).boxed_unsync())
                    .expect("static response parts are valid"),
            );
        }
        if to_this_port
            && matches!(*req.method(), Method::GET | Method::HEAD)
            && let Some((token, file)) = phone::parse_setup_path(req.uri().path_and_query().map_or("/", |p| p.as_str()))
            && lan.is_token(token)
        {
            let allowed = (!local).then_some(ip);
            if allowed.is_some() && lan.allow(ip) {
                (self.phone_events)(PhoneEvent::Scanned { client: lan.name.clone(), ip });
            }
            return Some(self.setup_answer(req, at, lan, file, allowed, ip));
        }
        if local {
            return None;
        }
        // Not a proxy request: the 400 page, as on every proxy port.
        let authority = authority?;
        let connect = req.method() == Method::CONNECT;
        let host = bare_host(authority.as_str());
        let port = authority.port_u16().unwrap_or(if connect { 443 } else { 80 });
        let start = Instant::now();
        let app = escape(&self.instance.app_name());
        let refusal = if !lan.allows(ip) {
            (self.phone_events)(PhoneEvent::Asked { client: lan.name.clone(), ip, host: host.clone() });
            page(
                StatusCode::FORBIDDEN,
                "Waiting for the Mac",
                format!(
                    "<p>This device ({ip}) is not allowed to use the {app} proxy yet. On the Mac, press <b>Allow</b> \
                     in the Proxy tab, or scan the QR code of Proxy tab, Set up iPhone.</p>"
                ),
            )
        } else if connect && host == phone::CHECK_HOST {
            // The setup page's trust check: the port answers it itself.
            return None;
        } else if host_key(&host).is_none() && self.reaches_this_mac(&host, port).await {
            page(
                StatusCode::FORBIDDEN,
                "Not through the phone port",
                format!(
                    "<p><code>{}</code> is this Mac. Through the phone port a phone reaches the routes \
                     (<code>.localhost</code> names), not the Mac's own servers.</p>",
                    escape(&host)
                ),
            )
        } else {
            return None;
        };
        let status = refusal.status().as_u16();
        let millis = start.elapsed().as_millis() as u64;
        if connect {
            self.log.push(LogEntry::tunnel(&host, status, millis, 0, 0));
            self.record_tunnel(at, SystemTime::now(), &host, port, status, millis, (0, 0));
        } else {
            let path = req.uri().path_and_query().map_or("/".to_string(), |p| p.to_string());
            let seen = self.seen(req, || req.uri().to_string(), ProxyMode::Http, at);
            self.log_http(req.method().as_str(), &host, &path, &refusal, start, ProxyMode::Http, Default::default(), seen);
        }
        Some(refusal)
    }

    /// The target resolves to this Mac: loopback, unspecified, or one of
    /// its own addresses (I5). A name that does not resolve is not refused
    /// here; sending it gives the 502.
    async fn reaches_this_mac(&self, host: &str, port: u16) -> bool {
        if host.eq_ignore_ascii_case("localhost") {
            return true;
        }
        let addrs: Vec<IpAddr> = match host.parse::<IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) => match self.upstream.resolve(host, port).await {
                Ok(list) => list.iter().map(SocketAddr::ip).collect(),
                Err(_) => return false,
            },
        };
        addrs.into_iter().any(|ip| (self.local_target)(ip))
    }

    /// The setup page or the CA profile. The server on the page is the host
    /// the phone used to reach this port.
    fn setup_answer<B>(&self, req: &Request<B>, at: &ClientPort, lan: &LanClient, file: SetupFile, allowed: Option<IpAddr>, ip: IpAddr) -> Response<Body> {
        let server = match req.uri().authority() {
            Some(a) => bare_host(a.as_str()),
            None => req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).map(bare_host).unwrap_or_default(),
        };
        let setup = Setup {
            app_name: self.instance.app_name(),
            bundle_id: self.instance.bundle_id(),
            token: lan.token(),
            server,
            port: at.port,
            ca: (self.setup_ca)(),
            allowed,
            via_proxy: req.uri().authority().is_some(),
            trust: lan.trust(ip),
            nonce: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            pac: lan.pac(),
            paused: lan.paused(),
        };
        let (body, content_type) = match file {
            SetupFile::Page => (Some(phone::setup_page(&setup)), "text/html; charset=utf-8"),
            SetupFile::CaProfile => (phone::ca_profile(&setup), MOBILECONFIG),
        };
        let Some(body) = body else { return self.not_a_proxy_request(at.port) };
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::CONTENT_SECURITY_POLICY, phone::setup_csp())
            .header(header::REFERRER_POLICY, "no-referrer")
            .body(Full::new(Bytes::from(body)).map_err(|never| match never {}).boxed_unsync())
            .expect("static response parts are valid")
    }

    /// Follow a request the proxy sends on: its body, and its WebSocket
    /// messages after an upgrade.
    fn recording(&self, seen: Option<HarRecord>, req: Request<Body>) -> (Request<Body>, Option<Recording>) {
        match (seen, self.router.har.as_ref()) {
            (Some(record), Some(har)) => {
                let (req, rec) = har.start_recording(record, req);
                (req, Some(rec))
            }
            _ => (req, None),
        }
    }

    /// A HAR record of what the client sent, only while the log is on: with
    /// it off nothing is copied (ADR 08, I1).
    fn seen<B>(&self, req: &Request<B>, url: impl FnOnce() -> String, mode: ProxyMode, at: &ClientPort) -> Option<HarRecord> {
        let har = self.router.har.as_ref()?;
        har.enabled().then(|| {
            let mut record = HarRecord::start(req, url(), mode);
            record.client = at.client.clone();
            record
        })
    }

    /// One closed tunnel, or a `CONNECT` the proxy answered itself.
    #[allow(clippy::too_many_arguments)]
    fn record_tunnel(&self, at: &ClientPort, started: SystemTime, host: &str, port: u16, status: u16, millis: u64, bytes: (u64, u64)) {
        if let Some(har) = self.router.har.as_ref().filter(|h| h.enabled()) {
            let mut record = HarRecord::tunnel(started, host, port, status, millis, bytes);
            record.client = at.client.clone();
            har.record(record);
        }
    }

    /// An inspected `CONNECT` whose client ended the TLS handshake: it sends
    /// no request, so this is its only entry.
    fn record_refused_inspection(&self, at: &ClientPort, started: SystemTime, host: &str, port: u16, millis: u64, why: &str) {
        if let Some(har) = self.router.har.as_ref().filter(|h| h.enabled()) {
            let mut record = HarRecord::tunnel(started, host, port, 200, millis, (0, 0));
            record.mode = ProxyMode::Inspect;
            record.bytes_in = None;
            record.bytes_out = None;
            record.client = at.client.clone();
            record.tls_error = Some(inspection_refused(why));
            har.record(record);
        }
    }

    /// Send a request to `base` (scheme and authority) plus its own path,
    /// through the matching script rules (ADR 07). Returns the response and
    /// the rules that ran and failed, for the log.
    async fn through_scripts(
        &self,
        req: Request<Body>,
        base: &str,
        host: &str,
        ctx: Ctx,
    ) -> (Response<Body>, (Vec<String>, Option<String>)) {
        let send = |req: Request<Body>| async move {
            let pq = req.uri().path_and_query().map_or("/".to_string(), |p| p.to_string());
            let sent = match format!("{base}{pq}").parse::<Uri>() {
                Ok(uri) => self.send(req, uri).await,
                Err(e) => Err(e.to_string()),
            };
            match sent {
                Ok(resp) => (resp, None),
                Err(e) => (bad_gateway(host, &e), Some(e)),
            }
        };
        let scripts = self.router.scripts.clone();
        match scripts.matching(host, req.uri().path(), req.method().as_str()) {
            None => (send(req).await.0, (vec![], None)),
            Some(m) => {
                let out = scripts.exchange(m, ctx, req, send).await;
                (out.response, (out.rules, out.script_error))
            }
        }
    }

    async fn connect(self: &Arc<Self>, req: Request<Incoming>, peer: SocketAddr, at: &ClientPort, cancel: CancellationToken) -> Response<Body> {
        let start = Instant::now();
        let started = SystemTime::now();
        let Some(authority) = req.uri().authority().cloned() else {
            return page(StatusCode::BAD_REQUEST, "Bad CONNECT", "<p>CONNECT needs a host and a port.</p>".into());
        };
        let host = bare_host(authority.as_str());
        let port = authority.port_u16().unwrap_or(443);
        if let Some(lan) = &at.lan
            && host == phone::CHECK_HOST
        {
            return self.trust_check(req, peer, lan.clone(), cancel);
        }
        if self.is_own_address(&host, port) {
            let resp = loop_detected(&host, port);
            self.log.push(LogEntry::tunnel(&host, resp.status().as_u16(), 0, 0, 0));
            self.record_tunnel(at, started, &host, port, resp.status().as_u16(), 0, (0, 0));
            return resp;
        }

        // A .localhost name: the router answers inside the tunnel, with a
        // leaf of the local CA (it must have a route, ADR 01 I5).
        if host_key(&host).is_some() {
            let upgrade = hyper::upgrade::on(req);
            let this = self.clone();
            let via = FromProxy { mode: ProxyMode::Inspect, client: at.client.clone() };
            tokio::spawn(async move {
                let Ok(upgraded) = upgrade.await else { return };
                let io = TokioIo::new(upgraded);
                let router = this.router.clone();
                let serve = async move {
                    if port == 80 {
                        router.serve_as(io, ClientScheme::Http, peer, Some(via)).await;
                        return;
                    }
                    let Some(cert) = this.local_certs.cert_for(&host) else {
                        tracing::debug!("CONNECT {host}: no route, so no certificate");
                        return;
                    };
                    if let Ok(tls) = accept_tls(io, cert).await {
                        router.serve_as(tls, ClientScheme::Https, peer, Some(via)).await;
                    }
                };
                tokio::select! { _ = serve => {}, _ = cancel.cancelled() => {} }
            });
            return empty(StatusCode::OK);
        }

        // In the inspect set: answer the client's TLS here and read each
        // request (ADR 06, change 2).
        if let Some(cert) = self.inspect_certs.cert_for(&host) {
            let upgrade = hyper::upgrade::on(req);
            let this = self.clone();
            let at = at.clone();
            tokio::spawn(async move {
                let Ok(upgraded) = upgrade.await else { return };
                let tls = match accept_tls(TokioIo::new(upgraded), cert).await {
                    Ok(tls) => {
                        note_trust(&at, peer, Trust::Trusted);
                        tls
                    }
                    Err(fail) => {
                        if fail.cert_refused {
                            note_trust(&at, peer, Trust::Refused);
                        }
                        this.record_refused_inspection(&at, started, &host, port, start.elapsed().as_millis() as u64, &fail.why);
                        return;
                    }
                };
                let session = this.clone();
                let service = hyper::service::service_fn(move |req| {
                    let session = session.clone();
                    let host = host.clone();
                    let at = at.clone();
                    async move { Ok::<_, Infallible>(session.inspected(req, &host, port, &at).await) }
                });
                let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
                let conn = builder.serve_connection_with_upgrades(TokioIo::new(tls), service);
                tokio::select! {
                    r = conn => if let Err(e) = r { tracing::debug!("inspected connection ended: {e}") },
                    _ = cancel.cancelled() => {}
                }
            });
            return empty(StatusCode::OK);
        }

        // A tunnel: connect first, so a server that does not answer is a 502
        // the client can show.
        let server = match self.upstream.connect(&host, port).await {
            Ok(server) => server,
            Err(e) => {
                let resp = bad_gateway(&host, &e);
                let millis = start.elapsed().as_millis() as u64;
                self.log.push(LogEntry::tunnel(&host, resp.status().as_u16(), millis, 0, 0));
                self.record_tunnel(at, started, &host, port, resp.status().as_u16(), millis, (0, 0));
                return resp;
            }
        };
        let upgrade = hyper::upgrade::on(req);
        let this = self.clone();
        let at = at.clone();
        tokio::spawn(async move {
            let (bytes_in, bytes_out) = match upgrade.await {
                Ok(client) => tunnel(client, server, cancel).await,
                Err(_) => (0, 0),
            };
            let millis = start.elapsed().as_millis() as u64;
            this.log.push(LogEntry::tunnel(&host, 200, millis, bytes_in, bytes_out));
            this.record_tunnel(&at, started, &host, port, 200, millis, (bytes_in, bytes_out));
        });
        empty(StatusCode::OK)
    }

    /// The setup page's trust check (`CONNECT trust-check.invalid:443`): a
    /// handshake with an inspection leaf, then a 1×1 GIF for any request.
    /// It notes whether the device trusts the CA and is never logged.
    fn trust_check(self: &Arc<Self>, req: Request<Incoming>, peer: SocketAddr, lan: Arc<LanClient>, cancel: CancellationToken) -> Response<Body> {
        let Some(cert) = self.inspect_certs.cert_for_own_name(phone::CHECK_HOST) else {
            return page(StatusCode::NOT_FOUND, "No inspection CA", "<p>HTTPS is not inspected, so there is nothing to trust.</p>".into());
        };
        let upgrade = hyper::upgrade::on(req);
        tokio::spawn(async move {
            let Ok(upgraded) = upgrade.await else { return };
            let tls = match accept_tls(TokioIo::new(upgraded), cert).await {
                Ok(tls) => tls,
                Err(fail) => {
                    if fail.cert_refused {
                        lan.set_trust(peer.ip(), Trust::Refused);
                    }
                    return;
                }
            };
            lan.set_trust(peer.ip(), Trust::Trusted);
            let service = hyper::service::service_fn(|_req| async {
                Ok::<_, Infallible>(
                    Response::builder()
                        .header(header::CONTENT_TYPE, "image/gif")
                        .header(header::CACHE_CONTROL, "no-store")
                        .body(Full::new(Bytes::from_static(PIXEL)).map_err(|never| match never {}).boxed_unsync())
                        .expect("static response parts are valid"),
                )
            });
            let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            let conn = builder.serve_connection(TokioIo::new(tls), service);
            tokio::select! { _ = conn => {}, _ = cancel.cancelled() => {} }
        });
        empty(StatusCode::OK)
    }

    /// One request inside an inspected `CONNECT`: sent to the same host and
    /// port over a new, verified TLS connection.
    async fn inspected(&self, req: Request<Incoming>, host: &str, port: u16, at: &ClientPort) -> Response<Body> {
        let start = Instant::now();
        let method = req.method().to_string();
        let path = req.uri().path_and_query().map_or("/".to_string(), |p| p.to_string());
        let name = if host.contains(':') { format!("[{host}]") } else { host.to_string() };
        let authority = if port == 443 { name } else { format!("{name}:{port}") };
        let seen = self.seen(&req, || format!("https://{authority}{path}"), ProxyMode::Inspect, at);
        let req = req.map(|b| b.map_err(Into::into).boxed_unsync());
        let (req, rec) = self.recording(seen, req);
        let ctx = Ctx { source: "proxy", route: None, scheme: "https", host: host.to_string(), port };
        let (resp, scripts) = self.through_scripts(req, &format!("https://{authority}"), host, ctx).await;
        self.log_sent(&method, host, &path, resp, start, ProxyMode::Inspect, scripts, rec)
    }

    /// Send a request to the real server at `uri`, as the client wrote it:
    /// hop-by-hop and proxy headers removed, nothing added (I10).
    async fn send(&self, mut req: Request<Body>, uri: Uri) -> Result<Response<Body>, String> {
        let tap = req.extensions_mut().remove::<WsTap>();
        let wants_upgrade = is_upgrade(req.headers());
        let client_upgrade = wants_upgrade.then(|| hyper::upgrade::on(&mut req));
        let (mut parts, body) = req.into_parts();
        // `Host` from an HTTP/1.1 client: the upstream client writes it
        // again from the URI for an HTTP/1.1 server, and an HTTP/2 server
        // gets `:authority` only. Some servers reset an HTTP/2 stream
        // that has both, and the client got a 502.
        if parts.headers.get(header::HOST).is_some_and(|h| same_authority(h, &uri)) {
            parts.headers.remove(header::HOST);
        }
        parts.uri = uri;
        parts.version = Version::HTTP_11;
        remove_hop_headers(&mut parts.headers, wants_upgrade);
        parts.headers.remove(header::PROXY_AUTHORIZATION);
        let out = Request::from_parts(parts, body);

        let mut resp = self.upstream.send(out, wants_upgrade).await?;
        if resp.status() == StatusCode::SWITCHING_PROTOCOLS
            && let Some(client_upgrade) = client_upgrade
        {
            let server_upgrade = hyper::upgrade::on(&mut resp);
            let extensions = resp.headers().get(header::SEC_WEBSOCKET_EXTENSIONS).and_then(|v| v.to_str().ok()).map(str::to_string);
            if let Some(tap) = &tap {
                tap.mark_started();
            }
            tokio::spawn(async move {
                if let Ok((client, server)) = tokio::try_join!(client_upgrade, server_upgrade) {
                    match tap {
                        Some(tap) => websocket::pump(TokioIo::new(client), TokioIo::new(server), tap, extensions).await,
                        None => {
                            let _ = tokio::io::copy_bidirectional(&mut TokioIo::new(client), &mut TokioIo::new(server)).await;
                        }
                    }
                }
            });
            let (parts, body) = resp.into_parts();
            return Ok(Response::from_parts(parts, body.map_err(Into::into).boxed_unsync()));
        }
        let (mut parts, body) = resp.into_parts();
        remove_hop_headers(&mut parts.headers, false);
        Ok(Response::from_parts(parts, body.map_err(Into::into).boxed_unsync()))
    }

    /// The destination is this proxy itself: a loopback address or
    /// `localhost`, on any of its ports (the main one or a client's).
    fn is_own_address(&self, host: &str, port: u16) -> bool {
        let local = host == "localhost"
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified() || ip.to_canonical().is_loopback());
        local && self.own_ports.read().unwrap().contains(&port)
    }

    #[allow(clippy::too_many_arguments)]
    fn log_http(
        &self,
        method: &str,
        host: &str,
        path: &str,
        resp: &Response<Body>,
        start: Instant,
        mode: ProxyMode,
        scripts: (Vec<String>, Option<String>),
        seen: Option<HarRecord>,
    ) {
        let millis = start.elapsed().as_millis() as u64;
        if let (Some(record), Some(har)) = (seen, self.router.har.as_ref()) {
            har.record(record.finish(resp, millis).with_scripts(&scripts));
        }
        let entry = LogEntry::http(method, host, path, resp.status().as_u16(), millis).via_proxy(mode);
        self.log.push(entry.with_scripts(scripts.0, scripts.1));
    }

    /// Log a request the proxy sent on. With a recording, the entry is
    /// written when the response body ends or the WebSocket closes.
    #[allow(clippy::too_many_arguments)]
    fn log_sent(
        &self,
        method: &str,
        host: &str,
        path: &str,
        resp: Response<Body>,
        start: Instant,
        mode: ProxyMode,
        scripts: (Vec<String>, Option<String>),
        rec: Option<Recording>,
    ) -> Response<Body> {
        let millis = start.elapsed().as_millis() as u64;
        let entry = LogEntry::http(method, host, path, resp.status().as_u16(), millis).via_proxy(mode);
        self.log.push(entry.with_scripts(scripts.0.clone(), scripts.1.clone()));
        match (rec, self.router.har.as_ref()) {
            (Some(mut rec), Some(har)) => {
                rec.record = rec.record.finish(&resp, millis).with_scripts(&scripts);
                har.end_recording(rec, resp)
            }
            _ => resp,
        }
    }

    fn not_a_proxy_request(&self, port: u16) -> Response<Body> {
        let cli = escape(&self.instance.cli());
        page(
            StatusCode::BAD_REQUEST,
            "This port is a proxy",
            format!(
                "<p>This is the {app} forward proxy. Set <code>http://127.0.0.1:{port}</code> as the HTTP proxy of a \
                 program; do not open it in the browser.</p><p>Settings for a shell: <code>{cli} proxy env</code>. \
                 Chrome with the proxy: <code>{cli} proxy chrome</code>.</p>",
                app = escape(&self.instance.app_name()),
            ),
        )
    }
}

/// `Host` names the URI's host and port (the scheme's port when absent),
/// case aside.
fn same_authority(host: &HeaderValue, uri: &Uri) -> bool {
    let (Ok(host), Some(authority)) = (host.to_str(), uri.authority()) else { return false };
    let Ok(host) = host.parse::<hyper::http::uri::Authority>() else { return false };
    let default = if uri.scheme_str() == Some("https") { 443 } else { 80 };
    host.host().eq_ignore_ascii_case(authority.host()) && host.port_u16().unwrap_or(default) == authority.port_u16().unwrap_or(default)
}

/// Why a client's TLS handshake failed.
struct HandshakeFailed {
    /// For the log.
    why: String,
    /// The client said it does not accept the certificate: it does not
    /// trust the CA. Closing without a word (an app that pins) is not this.
    cert_refused: bool,
}

impl HandshakeFailed {
    fn other(why: String) -> Self {
        Self { why, cert_refused: false }
    }
}

/// A TLS server handshake with one fixed leaf, ALPN `h2` and `http/1.1`.
async fn accept_tls<IO>(io: IO, cert: Arc<CertifiedKey>) -> Result<tokio_rustls::server::TlsStream<IO>, HandshakeFailed>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    let mut config = rustls::ServerConfig::builder_with_provider(crate::tls::provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| HandshakeFailed::other(e.to_string()))?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(OneCert(cert)));
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(io)).await {
        Ok(Ok(tls)) => Ok(tls),
        Ok(Err(e)) => {
            tracing::debug!("TLS handshake inside CONNECT failed: {e}");
            let cert_refused = matches!(
                e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()),
                Some(rustls::Error::AlertReceived(
                    rustls::AlertDescription::CertificateUnknown
                        | rustls::AlertDescription::UnknownCA
                        | rustls::AlertDescription::BadCertificate
                ))
            );
            Err(HandshakeFailed { why: e.to_string(), cert_refused })
        }
        Err(_) => Err(HandshakeFailed::other(format!("the client sent no TLS handshake in {} s", HANDSHAKE_TIMEOUT.as_secs()))),
    }
}

/// The `_tlsError` of a `CONNECT` whose client ended the handshake with the
/// inspection leaf: a phone or a program that does not trust the CA, or an
/// app that pins its certificates. Without it the log shows nothing at all.
fn inspection_refused(why: &str) -> String {
    format!(
        "The client ended the TLS handshake ({why}). It most likely does not trust the inspection CA: install the CA \
         and trust it, or take this host out of the inspected hosts. An app that pins its certificates always fails here."
    )
}

/// On a phone port, what a handshake with an inspection leaf says about the
/// device's trust in the CA (the setup page shows it).
fn note_trust(at: &ClientPort, peer: SocketAddr, trust: Trust) {
    if let Some(lan) = &at.lan {
        lan.set_trust(peer.ip(), trust);
    }
}

/// The leaf for the `CONNECT` host, whatever the client sends as SNI.
#[derive(Debug)]
struct OneCert(Arc<CertifiedKey>);

impl ResolvesServerCert for OneCert {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// Copy bytes both ways until both sides close or `cancel` fires. Returns the
/// bytes from the client and the bytes to the client. Nothing is read (I7).
async fn tunnel(client: Upgraded, server: TcpStream, cancel: CancellationToken) -> (u64, u64) {
    let bytes_in = AtomicU64::new(0);
    let bytes_out = AtomicU64::new(0);
    let (client_r, client_w) = tokio::io::split(TokioIo::new(client));
    let (server_r, server_w) = server.into_split();
    tokio::select! {
        _ = async {
            tokio::join!(
                crate::tcp::pipe(client_r, server_w, &bytes_in),
                crate::tcp::pipe(server_r, client_w, &bytes_out),
            )
        } => {}
        _ = cancel.cancelled() => {}
    }
    (bytes_in.load(Ordering::Relaxed), bytes_out.load(Ordering::Relaxed))
}

fn empty(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()).map_err(|never| match never {}).boxed_unsync())
        .expect("static response parts are valid")
}

fn https_needs_connect(authority: &str) -> Response<Body> {
    page(
        StatusCode::BAD_REQUEST,
        "Use CONNECT for https",
        format!(
            "<p>The request named <code>https://{}</code> in full. A proxy client sends <code>CONNECT</code> for \
             https:// URLs; this proxy does not take them in this form.</p>",
            escape(authority)
        ),
    )
}

fn loop_detected(host: &str, port: u16) -> Response<Body> {
    page(
        StatusCode::LOOP_DETECTED,
        "The proxy cannot proxy itself",
        format!("<p><code>{}:{port}</code> is this proxy. The request was not sent on.</p>", escape(host)),
    )
}

fn bad_gateway(host: &str, error: &str) -> Response<Body> {
    // The macOS trust store refused the server: say so, it is not an outage.
    let title = if error.contains("certificate") { "The server's certificate is not trusted" } else { "The server did not answer" };
    page(
        StatusCode::BAD_GATEWAY,
        title,
        format!("<p>The proxy could not reach <code>{}</code>: {}.</p>", escape(host), escape(error)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_authority_compares_host_and_port() {
        let yes = |host: &str, uri: &str| same_authority(&HeaderValue::from_str(host).unwrap(), &uri.parse().unwrap());
        assert!(yes("api.example.com", "https://api.example.com/a"));
        assert!(yes("Example.COM:443", "https://example.com/"));
        assert!(yes("example.com", "http://example.com:80/"));
        assert!(yes("example.com:8443", "https://example.com:8443/"));
        assert!(yes("[::1]:8080", "http://[::1]:8080/"));
        assert!(!yes("example.com", "https://example.com:8443/"));
        assert!(!yes("other.example", "https://example.com/"));
        assert!(!yes("not a host", "https://example.com/"));
    }

    #[test]
    fn loop_and_gateway_pages_name_the_host() {
        let r = loop_detected("127.0.0.1", 8877);
        assert_eq!(r.status(), StatusCode::LOOP_DETECTED);
        let r = bad_gateway("a<b", "TLS failed");
        assert_eq!(r.status(), StatusCode::BAD_GATEWAY);
    }
}
