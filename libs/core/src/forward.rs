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

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
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
use crate::inspect::bare_host;
use crate::instance::Instance;
use crate::logs::{LogEntry, ProxyMode, RequestLog};
use crate::proxy::{Body, ClientScheme, Proxy, escape, is_upgrade, page, remove_hop_headers};
use crate::scripts::Ctx;
use crate::routes::host_key;
use crate::tls::CertStore;
use crate::upstream::Upstream;

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
    /// The proxy port, for the loop check. `0` until bound.
    pub own_port: Arc<AtomicU16>,
}

impl ForwardProxy {
    /// Serve one accepted connection until it closes or `cancel` fires (the
    /// proxy was turned off, I2).
    pub async fn serve<IO>(self: Arc<Self>, io: IO, peer: SocketAddr, cancel: CancellationToken)
    where
        IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let this = self.clone();
        let child = cancel.clone();
        let service = hyper::service::service_fn(move |req| {
            let this = this.clone();
            let cancel = child.clone();
            async move { Ok::<_, Infallible>(this.handle(req, peer, cancel).await) }
        });
        let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
        let conn = builder.serve_connection_with_upgrades(TokioIo::new(io), service);
        tokio::select! {
            r = conn => if let Err(e) = r { tracing::debug!("proxy connection from {peer} ended: {e}") },
            _ = cancel.cancelled() => {}
        }
    }

    async fn handle(self: &Arc<Self>, mut req: Request<Incoming>, peer: SocketAddr, cancel: CancellationToken) -> Response<Body> {
        if req.method() == Method::CONNECT {
            return self.connect(req, peer, cancel).await;
        }
        let start = Instant::now();
        let method = req.method().to_string();
        let path = req.uri().path_and_query().map_or("/".to_string(), |p| p.to_string());
        let Some(authority) = req.uri().authority().cloned() else {
            let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let seen = self.seen(&req, || if host.is_empty() { path.clone() } else { format!("http://{host}{path}") }, ProxyMode::Http);
            let resp = self.not_a_proxy_request();
            self.log_http(&method, &host, &path, &resp, start, ProxyMode::Http, Default::default(), seen);
            return resp;
        };
        if req.uri().scheme_str() != Some("http") {
            let seen = self.seen(&req, || req.uri().to_string(), ProxyMode::Http);
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
            return self.router.handle(req, ClientScheme::Http, peer, Some(ProxyMode::Http)).await;
        }
        // What the client sent, before script rules (ADR 08).
        let seen = self.seen(&req, || req.uri().to_string(), ProxyMode::Http);
        if self.is_own_address(&host, port) {
            let resp = loop_detected(&host, port);
            self.log_http(&method, &host, &path, &resp, start, ProxyMode::Http, Default::default(), seen);
            return resp;
        }
        let req = req.map(|b| b.map_err(Into::into).boxed_unsync());
        let base = format!("http://{authority}");
        let ctx = Ctx { source: "proxy", route: None, scheme: "http", host: host.clone(), port };
        let (resp, scripts) = self.through_scripts(req, &base, &host, ctx).await;
        self.log_http(&method, &host, &path, &resp, start, ProxyMode::Http, scripts, seen);
        resp
    }

    /// A HAR record of what the client sent, only while the log is on: with
    /// it off nothing is copied (ADR 08, I1).
    fn seen<B>(&self, req: &Request<B>, url: impl FnOnce() -> String, mode: ProxyMode) -> Option<HarRecord> {
        let har = self.router.har.as_ref()?;
        har.enabled().then(|| HarRecord::start(req, url(), mode))
    }

    /// One closed tunnel, or a `CONNECT` the proxy answered itself.
    fn record_tunnel(&self, started: SystemTime, host: &str, port: u16, status: u16, millis: u64, bytes: (u64, u64)) {
        if let Some(har) = self.router.har.as_ref().filter(|h| h.enabled()) {
            har.record(HarRecord::tunnel(started, host, port, status, millis, bytes));
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

    async fn connect(self: &Arc<Self>, req: Request<Incoming>, peer: SocketAddr, cancel: CancellationToken) -> Response<Body> {
        let start = Instant::now();
        let started = SystemTime::now();
        let Some(authority) = req.uri().authority().cloned() else {
            return page(StatusCode::BAD_REQUEST, "Bad CONNECT", "<p>CONNECT needs a host and a port.</p>".into());
        };
        let host = bare_host(authority.as_str());
        let port = authority.port_u16().unwrap_or(443);
        if self.is_own_address(&host, port) {
            let resp = loop_detected(&host, port);
            self.log.push(LogEntry::tunnel(&host, resp.status().as_u16(), 0, 0, 0));
            self.record_tunnel(started, &host, port, resp.status().as_u16(), 0, (0, 0));
            return resp;
        }

        // A .localhost name: the router answers inside the tunnel, with a
        // leaf of the local CA (it must have a route, ADR 01 I5).
        if host_key(&host).is_some() {
            let upgrade = hyper::upgrade::on(req);
            let this = self.clone();
            tokio::spawn(async move {
                let Ok(upgraded) = upgrade.await else { return };
                let io = TokioIo::new(upgraded);
                let router = this.router.clone();
                let serve = async move {
                    if port == 80 {
                        router.serve_as(io, ClientScheme::Http, peer, Some(ProxyMode::Inspect)).await;
                        return;
                    }
                    let Some(cert) = this.local_certs.cert_for(&host) else {
                        tracing::debug!("CONNECT {host}: no route, so no certificate");
                        return;
                    };
                    if let Some(tls) = accept_tls(io, cert).await {
                        router.serve_as(tls, ClientScheme::Https, peer, Some(ProxyMode::Inspect)).await;
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
            tokio::spawn(async move {
                let Ok(upgraded) = upgrade.await else { return };
                let Some(tls) = accept_tls(TokioIo::new(upgraded), cert).await else { return };
                let session = this.clone();
                let service = hyper::service::service_fn(move |req| {
                    let session = session.clone();
                    let host = host.clone();
                    async move { Ok::<_, Infallible>(session.inspected(req, &host, port).await) }
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
                self.record_tunnel(started, &host, port, resp.status().as_u16(), millis, (0, 0));
                return resp;
            }
        };
        let upgrade = hyper::upgrade::on(req);
        let this = self.clone();
        tokio::spawn(async move {
            let (bytes_in, bytes_out) = match upgrade.await {
                Ok(client) => tunnel(client, server, cancel).await,
                Err(_) => (0, 0),
            };
            let millis = start.elapsed().as_millis() as u64;
            this.log.push(LogEntry::tunnel(&host, 200, millis, bytes_in, bytes_out));
            this.record_tunnel(started, &host, port, 200, millis, (bytes_in, bytes_out));
        });
        empty(StatusCode::OK)
    }

    /// One request inside an inspected `CONNECT`: sent to the same host and
    /// port over a new, verified TLS connection.
    async fn inspected(&self, req: Request<Incoming>, host: &str, port: u16) -> Response<Body> {
        let start = Instant::now();
        let method = req.method().to_string();
        let path = req.uri().path_and_query().map_or("/".to_string(), |p| p.to_string());
        let name = if host.contains(':') { format!("[{host}]") } else { host.to_string() };
        let authority = if port == 443 { name } else { format!("{name}:{port}") };
        let seen = self.seen(&req, || format!("https://{authority}{path}"), ProxyMode::Inspect);
        let req = req.map(|b| b.map_err(Into::into).boxed_unsync());
        let ctx = Ctx { source: "proxy", route: None, scheme: "https", host: host.to_string(), port };
        let (resp, scripts) = self.through_scripts(req, &format!("https://{authority}"), host, ctx).await;
        self.log_http(&method, host, &path, &resp, start, ProxyMode::Inspect, scripts, seen);
        resp
    }

    /// Send a request to the real server at `uri`, as the client wrote it:
    /// hop-by-hop and proxy headers removed, nothing added (I10).
    async fn send(&self, mut req: Request<Body>, uri: Uri) -> Result<Response<Body>, String> {
        let wants_upgrade = is_upgrade(req.headers());
        let client_upgrade = wants_upgrade.then(|| hyper::upgrade::on(&mut req));
        let (mut parts, body) = req.into_parts();
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
            tokio::spawn(async move {
                if let Ok((client, server)) = tokio::try_join!(client_upgrade, server_upgrade) {
                    let _ = tokio::io::copy_bidirectional(&mut TokioIo::new(client), &mut TokioIo::new(server)).await;
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
    /// `localhost`, on the proxy port.
    fn is_own_address(&self, host: &str, port: u16) -> bool {
        let own = self.own_port.load(Ordering::Relaxed);
        if own == 0 || port != own {
            return false;
        }
        host == "localhost"
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified() || ip.to_canonical().is_loopback())
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

    fn not_a_proxy_request(&self) -> Response<Body> {
        let port = self.own_port.load(Ordering::Relaxed);
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

/// A TLS server handshake with one fixed leaf, ALPN `h2` and `http/1.1`.
async fn accept_tls<IO>(io: IO, cert: Arc<CertifiedKey>) -> Option<tokio_rustls::server::TlsStream<IO>>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    let mut config = rustls::ServerConfig::builder_with_provider(crate::tls::provider())
        .with_safe_default_protocol_versions()
        .ok()?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(OneCert(cert)));
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(io)).await {
        Ok(Ok(tls)) => Some(tls),
        Ok(Err(e)) => {
            tracing::debug!("TLS handshake inside CONNECT failed: {e}");
            None
        }
        Err(_) => None,
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
    fn loop_and_gateway_pages_name_the_host() {
        let r = loop_detected("127.0.0.1", 8877);
        assert_eq!(r.status(), StatusCode::LOOP_DETECTED);
        let r = bad_gateway("a<b", "TLS failed");
        assert_eq!(r.status(), StatusCode::BAD_GATEWAY);
    }
}
