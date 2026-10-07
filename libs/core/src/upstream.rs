//! The forward proxy's client for real servers ("upstream"): resolve, connect,
//! pool, TLS that always checks the server's certificate, HTTP/1.1 and HTTP/2.
//! See ADR 06, change 1 (decision 8) and change 2 (decision 5).
//!
//! Until ADR 06 the daemon only ever connected to loopback. Everything that
//! reaches another machine goes through this file.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::body::Incoming;
use hyper::{Request, Response, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

use crate::proxy::Body;

/// Connect timeout for one upstream server, all its addresses together.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long an idle upstream connection stays in the pool.
pub const POOL_IDLE: Duration = Duration::from_secs(90);

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Turns a name into addresses. A trait so tests can prove that a
/// `.localhost` name never reaches it (I8) and send test names to local
/// servers.
pub trait Resolve: Send + Sync + 'static {
    fn resolve(&self, host: &str, port: u16) -> BoxFuture<io::Result<Vec<SocketAddr>>>;
}

/// The macOS resolver (`getaddrinfo`), so VPN DNS and `/etc/hosts` work.
pub struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> BoxFuture<io::Result<Vec<SocketAddr>>> {
        let host = host.to_string();
        Box::pin(async move { Ok(tokio::net::lookup_host((host.as_str(), port)).await?.collect()) })
    }
}

/// TLS settings that check server certificates with the macOS trust store, so
/// a company root the user installed works. There is no switch that accepts
/// any certificate (I6).
pub fn platform_tls() -> Result<Arc<rustls::ClientConfig>, rustls::Error> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let config = rustls::ClientConfig::builder_with_provider(crate::tls::provider())
        .with_safe_default_protocol_versions()?
        .with_platform_verifier()?
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// The upstream client. One per daemon; cheap to share.
pub struct Upstream {
    /// ALPN `h2` and `http/1.1`.
    client: Client<Connector, Body>,
    /// ALPN `http/1.1` only, for WebSocket and other upgrades.
    h1: Client<Connector, Body>,
    connector: Connector,
}

impl Upstream {
    /// `tls` must verify server certificates: production passes
    /// [`platform_tls`], tests a root store with a test CA.
    pub fn new(tls: Arc<rustls::ClientConfig>, resolver: Arc<dyn Resolve>) -> Self {
        let with_alpn = |alpn: &[&[u8]]| {
            let mut config = (*tls).clone();
            config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
            Connector { resolver: resolver.clone(), tls: Arc::new(config) }
        };
        let connector = with_alpn(&[b"h2", b"http/1.1"]);
        let h1_connector = with_alpn(&[b"http/1.1"]);
        let build = |c: Connector| {
            Client::builder(TokioExecutor::new()).pool_idle_timeout(POOL_IDLE).pool_timer(TokioTimer::new()).build(c)
        };
        Self { client: build(connector.clone()), h1: build(h1_connector), connector }
    }

    /// Send one request. Its URI is absolute (`https://host/path`). With
    /// `upgrade` the connection is HTTP/1.1, so a `101` answer can follow.
    pub async fn send(&self, req: Request<Body>, upgrade: bool) -> Result<Response<Incoming>, String> {
        let client = if upgrade { &self.h1 } else { &self.client };
        client.request(req).await.map_err(|e| describe(&e))
    }

    /// The addresses a name resolves to, with the resolver the connections
    /// use: the phone port checks them before it sends (ADR 10, I5).
    pub async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        self.connector.resolver.resolve(host, port).await
    }

    /// A TCP connection for a tunnel.
    pub async fn connect(&self, host: &str, port: u16) -> Result<TcpStream, String> {
        self.connector.tcp(host, port).await.map_err(|e| e.to_string())
    }
}

/// The error and its causes in one line, without the client's own wrapper
/// ("client error (Connect)") when a cause says more.
fn describe(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts: Vec<String> = vec![];
    let mut next = e.source();
    while let Some(cause) = next {
        let text = cause.to_string();
        if !parts.iter().any(|p| p.contains(&text)) {
            parts.push(text);
        }
        next = cause.source();
    }
    if parts.is_empty() { e.to_string() } else { parts.join(": ") }
}

#[derive(Clone)]
struct Connector {
    resolver: Arc<dyn Resolve>,
    tls: Arc<rustls::ClientConfig>,
}

impl Connector {
    async fn tcp(&self, host: &str, port: u16) -> io::Result<TcpStream> {
        let attempt = async {
            let addrs = self
                .resolver
                .resolve(host, port)
                .await
                .map_err(|e| io::Error::new(e.kind(), format!("cannot resolve {host}: {e}")))?;
            let mut last = io::Error::new(io::ErrorKind::NotFound, format!("{host} has no address"));
            for addr in addrs {
                match TcpStream::connect(addr).await {
                    Ok(tcp) => {
                        let _ = tcp.set_nodelay(true);
                        return Ok(tcp);
                    }
                    Err(e) => last = io::Error::new(e.kind(), format!("cannot connect to {host}:{port}: {e}")),
                }
            }
            Err(last)
        };
        tokio::time::timeout(CONNECT_TIMEOUT, attempt).await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{host}:{port} did not answer within {} s", CONNECT_TIMEOUT.as_secs()),
            )
        })?
    }
}

impl tower_service::Service<Uri> for Connector {
    type Response = Conn;
    type Error = io::Error;
    type Future = BoxFuture<io::Result<Conn>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            let host = uri.host().ok_or_else(|| io::Error::other("the URL has no host"))?;
            let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
            let https = uri.scheme_str() == Some("https");
            let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
            let tcp = this.tcp(&host, port).await?;
            if !https {
                return Ok(Conn { io: TokioIo::new(Box::new(tcp)), h2: false });
            }
            let name = ServerName::try_from(host.clone())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("{host} is not a TLS name: {e}")))?;
            let tls = tokio_rustls::TlsConnector::from(this.tls.clone())
                .connect(name, tcp)
                .await
                .map_err(|e| io::Error::new(e.kind(), format!("TLS to {host} failed: {e}")))?;
            let h2 = tls.get_ref().1.alpn_protocol() == Some(b"h2");
            Ok(Conn { io: TokioIo::new(Box::new(tls)), h2 })
        })
    }
}

trait Io: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Io for T {}

/// One upstream connection, plain or TLS, and whether TLS chose HTTP/2.
struct Conn {
    io: TokioIo<Box<dyn Io>>,
    h2: bool,
}

impl Connection for Conn {
    fn connected(&self) -> Connected {
        if self.h2 { Connected::new().negotiated_h2() } else { Connected::new() }
    }
}

impl hyper::rt::Read for Conn {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: hyper::rt::ReadBufCursor<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl hyper::rt::Write for Conn {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_drops_the_wrapper_and_keeps_the_cause() {
        #[derive(Debug)]
        struct Outer(io::Error);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("client error (Connect)")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let e = Outer(io::Error::other("TLS to x failed: invalid peer certificate: UnknownIssuer"));
        assert_eq!(describe(&e), "TLS to x failed: invalid peer certificate: UnknownIssuer");
    }

    #[tokio::test]
    async fn a_name_without_an_answer_is_a_clear_error() {
        struct Nothing;
        impl Resolve for Nothing {
            fn resolve(&self, _: &str, _: u16) -> BoxFuture<io::Result<Vec<SocketAddr>>> {
                Box::pin(async { Ok(vec![]) })
            }
        }
        let tls = platform_tls().unwrap();
        let up = Upstream::new(tls, Arc::new(Nothing));
        let e = up.connect("nowhere.example", 443).await.unwrap_err();
        assert!(e.contains("nowhere.example has no address"), "{e}");
    }

    #[test]
    fn platform_tls_builds() {
        assert!(platform_tls().is_ok());
    }
}
