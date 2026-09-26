//! HTTP proxy for HTTP routes: shared ports 80 and 443, chosen by name.
//!
//! One incoming connection may be HTTP/1.1 or HTTP/2. Each request is sent to
//! the target over a new HTTP/1.1 connection (plain or TLS). WebSocket
//! upgrades are passed through. See ADR 01, changes 6 and 7.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Request, Response, StatusCode, Uri, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::logs::{LogEntry, RequestLog};
use crate::routes::{Route, Scheme, host_key};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

type Body = BoxBody<Bytes, hyper::Error>;

/// What the proxy needs from the daemon.
pub trait RouteSource: Send + Sync {
    /// The HTTP route for a `Host` header (longest match, fallback as configured).
    fn lookup(&self, host: &str) -> Option<Route>;
    /// All routes, for the 404 page.
    fn all(&self) -> Vec<Route>;
    /// The HTTPS port actually bound, for `https_only` redirects.
    fn https_port(&self) -> Option<u16>;
}

pub struct Proxy {
    pub routes: Arc<dyn RouteSource>,
    pub log: Arc<RequestLog>,
    pub tls_client: Arc<rustls::ClientConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientScheme {
    Http,
    Https,
}

impl ClientScheme {
    fn as_str(self) -> &'static str {
        match self {
            ClientScheme::Http => "http",
            ClientScheme::Https => "https",
        }
    }
}

impl Proxy {
    /// Serve one accepted client connection until it closes.
    pub async fn serve<IO>(self: Arc<Self>, io: IO, scheme: ClientScheme, peer: SocketAddr)
    where
        IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let service = hyper::service::service_fn(move |req| {
            let proxy = self.clone();
            async move { Ok::<_, Infallible>(proxy.handle(req, scheme, peer).await) }
        });
        let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
        if let Err(e) = builder.serve_connection_with_upgrades(TokioIo::new(io), service).await {
            tracing::debug!("connection from {peer} ended: {e}");
        }
    }

    async fn handle(&self, req: Request<Incoming>, scheme: ClientScheme, peer: SocketAddr) -> Response<Body> {
        let start = Instant::now();
        let host = request_host(&req).unwrap_or_default();
        let method = req.method().to_string();
        let path = req.uri().path_and_query().map_or("/".to_string(), |p| p.to_string());

        let response = match self.routes.lookup(&host) {
            None => not_found(&host, &self.routes.all()),
            Some(route) if scheme == ClientScheme::Http && route.https_only => {
                redirect_to_https(&host, &path, self.routes.https_port())
            }
            Some(route) => match self.forward(req, &route, &host, scheme, peer).await {
                Ok(resp) => resp,
                Err(e) => bad_gateway(&host, &route, &e),
            },
        };
        let millis = start.elapsed().as_millis() as u64;
        self.log.push(LogEntry::http(&method, &host, &path, response.status().as_u16(), millis));
        response
    }

    async fn forward(
        &self,
        mut req: Request<Incoming>,
        route: &Route,
        host: &str,
        scheme: ClientScheme,
        peer: SocketAddr,
    ) -> Result<Response<Body>, String> {
        let target = route.target_addr().map_err(|e| e.to_string())?;
        let addr = target.socket_addr();
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| format!("{} did not answer within {} s", route.target, CONNECT_TIMEOUT.as_secs()))?
            .map_err(|e| format!("cannot connect to {}: {e}", route.target))?;
        let _ = tcp.set_nodelay(true);

        let wants_upgrade = is_upgrade(req.headers());
        let client_upgrade = wants_upgrade.then(|| hyper::upgrade::on(&mut req));
        let out = outgoing_request(req, host, scheme, peer, wants_upgrade)?;

        let mut resp = if target.scheme == Scheme::Https {
            let connector = tokio_rustls::TlsConnector::from(self.tls_client.clone());
            let name = if target.host == "localhost" {
                rustls::pki_types::ServerName::try_from("localhost").map_err(|e| e.to_string())?
            } else {
                rustls::pki_types::ServerName::IpAddress(addr.ip().into())
            };
            let tls = connector
                .connect(name.to_owned(), tcp)
                .await
                .map_err(|e| format!("TLS to {} failed: {e}", route.target))?;
            send(TokioIo::new(tls), out).await?
        } else {
            send(TokioIo::new(tcp), out).await?
        };

        if resp.status() == StatusCode::SWITCHING_PROTOCOLS {
            if let Some(client_upgrade) = client_upgrade {
                let server_upgrade = hyper::upgrade::on(&mut resp);
                tokio::spawn(async move {
                    match tokio::try_join!(client_upgrade, server_upgrade) {
                        Ok((client, server)) => {
                            let mut client = TokioIo::new(client);
                            let mut server = TokioIo::new(server);
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
                        }
                        Err(e) => tracing::debug!("upgrade failed: {e}"),
                    }
                });
            }
            let (parts, body) = resp.into_parts();
            return Ok(Response::from_parts(parts, body.boxed()));
        }

        let (mut parts, body) = resp.into_parts();
        remove_hop_headers(&mut parts.headers, false);
        Ok(Response::from_parts(parts, body.boxed()))
    }
}

async fn send<IO>(io: IO, req: Request<Incoming>) -> Result<Response<Incoming>, String>
where
    IO: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
        .handshake(io)
        .await
        .map_err(|e| format!("HTTP handshake with the target failed: {e}"))?;
    tokio::spawn(async move {
        if let Err(e) = conn.with_upgrades().await {
            tracing::debug!("upstream connection ended: {e}");
        }
    });
    sender.send_request(req).await.map_err(|e| format!("the target closed the connection: {e}"))
}

/// The request as the dev server sees it: HTTP/1.1, origin-form URI, `Host`
/// unchanged (I12), `X-Forwarded-*` added.
fn outgoing_request(
    req: Request<Incoming>,
    host: &str,
    scheme: ClientScheme,
    peer: SocketAddr,
    keep_upgrade: bool,
) -> Result<Request<Incoming>, String> {
    let (mut parts, body) = req.into_parts();
    let path = parts.uri.path_and_query().map_or("/", |p| p.as_str()).to_string();
    parts.uri = path.parse::<Uri>().map_err(|e| e.to_string())?;
    parts.version = Version::HTTP_11;
    remove_hop_headers(&mut parts.headers, keep_upgrade);
    let h = &mut parts.headers;
    h.insert(header::HOST, HeaderValue::from_str(host).map_err(|e| e.to_string())?);

    let peer_ip = peer.ip().to_canonical().to_string();
    let xff = match h.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        Some(prev) => format!("{prev}, {peer_ip}"),
        None => peer_ip,
    };
    h.insert(HeaderName::from_static("x-forwarded-for"), HeaderValue::from_str(&xff).map_err(|e| e.to_string())?);
    h.insert(HeaderName::from_static("x-forwarded-proto"), HeaderValue::from_static(scheme.as_str()));
    h.insert(HeaderName::from_static("x-forwarded-host"), HeaderValue::from_str(host).map_err(|e| e.to_string())?);
    Ok(Request::from_parts(parts, body))
}

fn is_upgrade(headers: &HeaderMap) -> bool {
    headers.contains_key(header::UPGRADE)
        && headers
            .get_all(header::CONNECTION)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .any(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case("upgrade")))
}

const HOP_HEADERS: [&str; 7] =
    ["connection", "keep-alive", "proxy-connection", "te", "trailer", "transfer-encoding", "upgrade"];

fn remove_hop_headers(headers: &mut HeaderMap, keep_upgrade: bool) {
    // Headers named in Connection are hop-by-hop too.
    let named: Vec<String> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty() && t != "upgrade")
        .collect();
    for name in named {
        headers.remove(name.as_str());
    }
    for name in HOP_HEADERS {
        if keep_upgrade && (name == "connection" || name == "upgrade") {
            continue;
        }
        headers.remove(name);
    }
    if keep_upgrade {
        headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    }
}

/// `Host` header (HTTP/1.1) or `:authority` (HTTP/2).
fn request_host<B>(req: &Request<B>) -> Option<String> {
    req.headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| req.uri().authority().map(|a| a.to_string()))
}

fn page(status: StatusCode, title: &str, body_html: String) -> Response<Body> {
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
         <style>body{{font:15px -apple-system,sans-serif;max-width:44em;margin:3em auto;padding:0 1em;color:#222}}\
         code{{background:#f2f2f2;padding:1px 4px;border-radius:3px}}li{{margin:.3em 0}}</style></head>\
         <body><h1>{title}</h1>{body_html}<p style=\"color:#888\">LocalRouter</p></body></html>"
    );
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from(html)).map_err(|never| match never {}).boxed())
        .expect("static response parts are valid")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn not_found(host: &str, routes: &[Route]) -> Response<Body> {
    let mut list = String::new();
    for r in routes {
        let name = escape(&r.full_name());
        let item = match r.listen_port {
            Some(port) => format!("<li><code>{name}:{port}</code> (tcp) → <code>{}</code></li>", escape(&r.target)),
            None => format!("<li><a href=\"//{name}/\">{name}</a> → <code>{}</code></li>", escape(&r.target)),
        };
        list.push_str(&item);
    }
    if list.is_empty() {
        list = "<li>No routes yet. Add one with <code>localrouter add shop 5173</code>.</li>".into();
    }
    let known = if host_key(host).is_some() { "" } else { " It is not a .localhost name." };
    page(
        StatusCode::NOT_FOUND,
        "No route for this name",
        format!("<p>No route matches <code>{}</code>.{known}</p><h2>Routes</h2><ul>{list}</ul>", escape(host)),
    )
}

fn bad_gateway(host: &str, route: &Route, error: &str) -> Response<Body> {
    let note = if route.note.is_empty() { String::new() } else { format!("<p>Note: {}</p>", escape(&route.note)) };
    page(
        StatusCode::BAD_GATEWAY,
        "The dev server did not answer",
        format!(
            "<p><code>{}</code> goes to <code>{}</code>, but {}.</p>{note}<p>Is the dev server running?</p>",
            escape(host),
            escape(&route.target),
            escape(error)
        ),
    )
}

fn redirect_to_https(host: &str, path: &str, https_port: Option<u16>) -> Response<Body> {
    let name = host.rsplit_once(':').map_or(host, |(h, p)| if p.bytes().all(|b| b.is_ascii_digit()) { h } else { host });
    let port = match https_port {
        Some(443) | None => String::new(),
        Some(p) => format!(":{p}"),
    };
    let location = format!("https://{name}{port}{path}");
    Response::builder()
        .status(StatusCode::PERMANENT_REDIRECT)
        .header(header::LOCATION, location)
        .body(Full::new(Bytes::new()).map_err(|never| match never {}).boxed())
        .expect("static response parts are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hop_headers_are_removed_and_named_ones_too() {
        let mut h = HeaderMap::new();
        h.insert("connection", HeaderValue::from_static("keep-alive, x-secret"));
        h.insert("x-secret", HeaderValue::from_static("1"));
        h.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        h.insert("x-app", HeaderValue::from_static("1"));
        remove_hop_headers(&mut h, false);
        assert!(h.get("x-secret").is_none());
        assert!(h.get("connection").is_none());
        assert!(h.get("x-app").is_some());
    }

    #[test]
    fn upgrade_headers_survive_when_asked() {
        let mut h = HeaderMap::new();
        h.insert("connection", HeaderValue::from_static("Upgrade"));
        h.insert("upgrade", HeaderValue::from_static("websocket"));
        assert!(is_upgrade(&h));
        remove_hop_headers(&mut h, true);
        assert_eq!(h.get("upgrade").unwrap(), "websocket");
        assert_eq!(h.get("connection").unwrap(), "upgrade");
    }

    #[test]
    fn redirect_keeps_path_and_uses_https_port() {
        let r = redirect_to_https("shop.localhost:8080", "/a?b=1", Some(8443));
        assert_eq!(r.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(r.headers()[header::LOCATION], "https://shop.localhost:8443/a?b=1");
        let r = redirect_to_https("shop.localhost", "/", Some(443));
        assert_eq!(r.headers()[header::LOCATION], "https://shop.localhost/");
    }
}
