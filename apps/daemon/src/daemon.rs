//! Daemon state and the socket API methods.
//!
//! Every change to routes or settings goes through [`Daemon`], one at a time
//! (`write` lock), so memory and disk never disagree (invariant I4).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use localrouter_core::api::{
    self, ApiError, CaState, CaStatus, ErrorCode, FindFreePortParams, FindFreePortResult, GetLogsParams, GetLogsResult,
    HelloParams, HelloResult, HostParams, ListRoutesResult, PortStatus, RegisterRouteResult, ResetCaResult, RouteView,
    SetConfigParams, SetConfigResult, StatusResult, UnregisterRouteResult,
};
use localrouter_core::config::Config;
use localrouter_core::logs::RequestLog;
use localrouter_core::instance::Instance;
use localrouter_core::paths::Paths;
use localrouter_core::proxy::{ClientScheme, Proxy, RouteSource};
use localrouter_core::routes::{Protocol, Reserved, Route, RouteError, RouteKey, RouteTable, normalize_path};
use localrouter_core::tcp::{self, TcpRouteInfo};
use localrouter_core::tls::{CertStore, LocalCa};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::listen::{self, RefusalLog};
use crate::pidwatch::{self, PidWatch};
use crate::{store, tcp_listen};

pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Route table and settings that the proxy and the TLS resolver read.
pub struct Shared {
    pub routes: RwLock<RouteTable>,
    pub config: RwLock<Config>,
    pub http_port: AtomicU16,
    pub https_port: AtomicU16,
}

impl RouteSource for Shared {
    fn lookup(&self, host: &str, path: &str) -> Option<Route> {
        let fallback = self.config.read().unwrap().fallback;
        self.routes.read().unwrap().lookup(host, path, fallback).cloned()
    }
    fn all(&self) -> Vec<Route> {
        self.routes.read().unwrap().iter().cloned().collect()
    }
    fn https_port(&self) -> Option<u16> {
        Some(self.https_port.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }
    fn http_port(&self) -> Option<u16> {
        Some(self.http_port.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }
}

struct TcpHandle {
    port: u16,
    cancel: CancellationToken,
}

#[derive(Default)]
struct Problems {
    ca: Option<String>,
    routes_file: Option<String>,
    http: Option<PortStatus>,
    https: Option<PortStatus>,
    /// TCP routes whose port could not be bound at start.
    listen_failed: Vec<String>,
}

pub struct Daemon {
    pub paths: Paths,
    pub instance: Instance,
    /// The ports in config.json when the daemon started. Ports are bound
    /// once, so a restart is needed while config.json names other ports.
    start_ports: (u16, u16),
    pub shared: Arc<Shared>,
    pub log: Arc<RequestLog>,
    pub certs: Arc<CertStore>,
    pub proxy: Arc<Proxy>,
    pub shutdown: CancellationToken,
    pids: PidWatch,
    tcp: Mutex<HashMap<String, TcpHandle>>,
    problems: Mutex<Problems>,
    trust_cache: Mutex<Option<(Instant, Option<bool>)>>,
    write: tokio::sync::Mutex<()>,
    refusals: RefusalLog,
}

fn err(code: ErrorCode, message: impl Into<String>) -> ApiError {
    ApiError::new(code, message)
}

fn route_err(e: RouteError) -> ApiError {
    err(ErrorCode::InvalidRoute, e.to_string())
}

impl Daemon {
    /// Load config, CA and routes. Does not bind anything yet.
    pub fn load(paths: Paths, instance: Instance, pids: PidWatch, ca: Option<LocalCa>, ca_problem: Option<String>) -> Arc<Self> {
        let config = store::load_config(&paths.config(), &instance);
        let start_ports = (config.value.http_port, config.value.https_port);
        let shared = Arc::new(Shared {
            routes: RwLock::new(RouteTable::new()),
            config: RwLock::new(config.value.clone()),
            http_port: AtomicU16::new(0),
            https_port: AtomicU16::new(0),
        });
        if (config.problem.is_some() || !paths.config().exists())
            && let Err(e) = store::save_config(&paths.config(), &config.value)
        {
            tracing::warn!("could not write config.json: {e}");
        }
        let log = Arc::new(RequestLog::new(config.value.log_size));
        let lookup = shared.clone();
        // A name gets a certificate when any HTTP route serves it, with or
        // without a path: TLS comes before the path is known (I27).
        let certs = Arc::new(CertStore::new(
            ca,
            Arc::new(move |name: &str| {
                let fallback = lookup.config.read().unwrap().fallback;
                lookup.routes.read().unwrap().serves(name, fallback)
            }),
        ));

        let loaded = store::load_routes(&paths.routes());
        let mut problems = Problems { ca: ca_problem, routes_file: loaded.problem, ..Default::default() };
        if let Some(p) = &config.problem {
            tracing::warn!("{p}");
        }
        {
            let mut table = shared.routes.write().unwrap();
            let reserved = reserved(&config.value);
            for mut route in loaded.value {
                // Saved routes are persistent by definition and never owned (I3).
                route.persistent = true;
                route.owner_pid = None;
                match table.validate(&mut route, reserved) {
                    Ok(()) => {
                        table.insert(route);
                    }
                    Err(e) => {
                        let key = route.key();
                        tracing::warn!("skipped saved route {key}: {e}");
                        problems.routes_file.get_or_insert_with(String::new).push_str(&format!("skipped {key}: {e}. "));
                    }
                }
            }
        }
        Arc::new_cyclic(|this: &Weak<Self>| {
            let this = this.clone();
            let proxy = Arc::new(Proxy {
                instance: instance.clone(),
                routes: shared.clone(),
                log: log.clone(),
                tls_client: localrouter_core::tls::insecure_loopback_client_config(),
                // router.localhost shows the same status as `localrouter status`.
                status: Some(Arc::new(move || {
                    let this = this.clone();
                    Box::pin(async move { Some(this.upgrade()?.status().await) })
                })),
            });
            Self {
                paths,
                instance,
                start_ports,
                shared,
                log,
                certs,
                proxy,
                shutdown: CancellationToken::new(),
                pids,
                tcp: Mutex::new(HashMap::new()),
                problems: Mutex::new(problems),
                trust_cache: Mutex::new(None),
                write: tokio::sync::Mutex::new(()),
                refusals: RefusalLog::new(),
            }
        })
    }

    fn config(&self) -> Config {
        self.shared.config.read().unwrap().clone()
    }

    // ---- listeners

    /// Bind the shared HTTP and HTTPS ports and start accepting.
    pub fn start_http_listeners(self: &Arc<Self>) -> anyhow::Result<()> {
        let config = self.config();
        let tls = localrouter_core::tls::server_config(self.certs.clone())?;
        let acceptor = tokio_rustls::TlsAcceptor::from(tls);
        for (port, scheme) in [(config.http_port, ClientScheme::Http), (config.https_port, ClientScheme::Https)] {
            let (listeners, status) = listen::bind_all(port);
            for e in &status.errors {
                tracing::error!("{e}");
            }
            let bound = status.port.unwrap_or(0);
            match scheme {
                ClientScheme::Http => {
                    self.shared.http_port.store(bound, Ordering::Relaxed);
                    self.problems.lock().unwrap().http = Some(status);
                }
                ClientScheme::Https => {
                    self.shared.https_port.store(bound, Ordering::Relaxed);
                    self.problems.lock().unwrap().https = Some(status);
                }
            }
            for l in listeners {
                let listener = TcpListener::from_std(l)?;
                let this = self.clone();
                let acceptor = acceptor.clone();
                tokio::spawn(async move { this.accept_http(listener, scheme, acceptor).await });
            }
        }
        Ok(())
    }

    async fn accept_http(self: Arc<Self>, listener: TcpListener, scheme: ClientScheme, acceptor: tokio_rustls::TlsAcceptor) {
        loop {
            let (stream, peer) = tokio::select! {
                r = listener.accept() => match r {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::warn!("accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                },
                _ = self.shutdown.cancelled() => return,
            };
            // I2: decide before reading any byte.
            let allow_lan = self.shared.config.read().unwrap().allow_lan;
            if !listen::peer_allowed(peer.ip(), allow_lan) {
                self.refusals.refused(peer);
                drop(stream);
                continue;
            }
            let _ = stream.set_nodelay(true);
            let proxy = self.proxy.clone();
            match scheme {
                ClientScheme::Http => {
                    tokio::spawn(proxy.serve(stream, scheme, peer));
                }
                ClientScheme::Https => {
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)).await {
                            Ok(Ok(tls)) => proxy.serve(tls, scheme, peer).await,
                            Ok(Err(e)) => tracing::debug!("TLS handshake with {peer} failed: {e}"),
                            Err(_) => tracing::debug!("TLS handshake with {peer} timed out"),
                        }
                    });
                }
            }
        }
    }

    /// Bind listeners for the persistent TCP routes loaded at start. A port
    /// that is taken marks the route `listen_failed`; the route stays.
    pub fn start_saved_tcp_routes(self: &Arc<Self>) {
        let routes: Vec<Route> =
            self.shared.routes.read().unwrap().iter().filter(|r| r.protocol == Protocol::Tcp).cloned().collect();
        for route in routes {
            let port = route.listen_port.unwrap_or(0);
            match tcp_listen::bind_loopback(port) {
                Ok((listeners, port)) => self.start_tcp(&route.host, listeners, port),
                Err(e) => {
                    tracing::error!("TCP route {} cannot listen on {port}: {e}", route.host);
                    self.problems.lock().unwrap().listen_failed.push(route.host.clone());
                }
            }
        }
    }

    fn start_tcp(self: &Arc<Self>, host: &str, listeners: Vec<std::net::TcpListener>, port: u16) {
        let cancel = self.shutdown.child_token();
        for l in listeners {
            let Ok(listener) = TcpListener::from_std(l) else { continue };
            let this = self.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                loop {
                    let (stream, _) = tokio::select! {
                        r = listener.accept() => match r {
                            Ok(x) => x,
                            Err(_) => { tokio::time::sleep(Duration::from_millis(50)).await; continue; }
                        },
                        _ = cancel.cancelled() => return,
                    };
                    // Read the target at accept time: a re-registered route may
                    // keep its port and change its target.
                    let info = this.shared.routes.read().unwrap().by_listen_port(port).and_then(|r| {
                        Some(TcpRouteInfo { host: r.host.clone(), listen_port: port, target: r.target_addr().ok()?.socket_addr() })
                    });
                    let Some(info) = info else { continue };
                    tokio::spawn(tcp::serve(stream, info, this.log.clone(), cancel.clone()));
                }
            });
        }
        self.problems.lock().unwrap().listen_failed.retain(|h| h != host);
        self.tcp.lock().unwrap().insert(host.to_string(), TcpHandle { port, cancel });
    }

    /// Close a TCP route's listeners and open connections (I18).
    fn stop_tcp(&self, host: &str) {
        if let Some(handle) = self.tcp.lock().unwrap().remove(host) {
            handle.cancel.cancel();
        }
        self.problems.lock().unwrap().listen_failed.retain(|h| h != host);
    }

    // ---- owner processes

    pub async fn remove_owned_by(self: &Arc<Self>, pid: u32) {
        let _guard = self.write.lock().await;
        // Exactly the keys this process owns (I28): an owned shop/blog goes,
        // a persistent shop stays.
        let keys = self.shared.routes.read().unwrap().owned_by(pid);
        for key in keys {
            self.shared.routes.write().unwrap().remove(&key);
            if key.path.is_empty() {
                self.stop_tcp(&key.host);
            }
            tracing::info!("removed route {key}: owner process {pid} exited");
        }
    }

    // ---- API methods

    pub fn hello(&self, p: HelloParams) -> Result<HelloResult, ApiError> {
        let ours = api_version();
        if api::api_major(&p.api_version) != api::api_major(&ours) {
            return Err(err(
                ErrorCode::VersionMismatch,
                format!(
                    "the {} speaks API {} but this daemon speaks {}; update the older one",
                    p.client,
                    p.api_version,
                    ours
                ),
            ));
        }
        tracing::debug!("hello from {}", p.client);
        Ok(HelloResult { api_version: ours, daemon_version: DAEMON_VERSION.into() })
    }

    pub async fn status(&self) -> StatusResult {
        let trusted = if self.certs.has_ca() { self.ca_trusted().await } else { None };
        let problems = self.problems.lock().unwrap();
        let empty = |configured| PortStatus { configured, port: None, bound: vec![], errors: vec![] };
        let config = self.config();
        let (state, problem) = match (&problems.ca, self.certs.has_ca()) {
            (_, true) => (CaState::Ok, None),
            (p, false) => (CaState::Broken, p.clone().or(Some("no CA loaded".into()))),
        };
        let http = problems.http.clone().unwrap_or_else(|| empty(config.http_port));
        let https = problems.https.clone().unwrap_or_else(|| empty(config.https_port));
        let routes_file_problem = problems.routes_file.clone();
        let listen_failed = problems.listen_failed.clone();
        drop(problems);
        StatusResult {
            daemon_version: DAEMON_VERSION.into(),
            api_version: api_version(),
            pid: std::process::id(),
            data_dir: self.paths.data.display().to_string(),
            http,
            https,
            ca: CaStatus {
                state,
                problem,
                pem_path: self.paths.ca_pem().display().to_string(),
                common_name: self.certs.common_name(),
                trusted,
            },
            routes: self.shared.routes.read().unwrap().len(),
            routes_file_problem,
            listen_failed,
        }
    }

    /// Ask macOS whether `ca.pem` is trusted for TLS. Cached for 10 seconds.
    async fn ca_trusted(&self) -> Option<bool> {
        if let Some((at, value)) = *self.trust_cache.lock().unwrap()
            && at.elapsed() < Duration::from_secs(10)
        {
            return value;
        }
        let pem = self.paths.ca_pem();
        let out = tokio::process::Command::new("/usr/bin/security")
            .args(["verify-cert", "-L", "-q", "-p", "ssl", "-c"])
            .arg(&pem)
            .output()
            .await;
        let value = out.ok().map(|o| o.status.success());
        *self.trust_cache.lock().unwrap() = Some((Instant::now(), value));
        value
    }

    pub fn forget_trust_cache(&self) {
        *self.trust_cache.lock().unwrap() = None;
    }

    fn view(&self, route: &Route) -> RouteView {
        let http = self.shared.http_port.load(Ordering::Relaxed);
        let https = self.shared.https_port.load(Ordering::Relaxed);
        let name = route.full_name();
        let path = route.path.as_deref().unwrap_or("");
        let urls = match route.protocol {
            Protocol::Tcp => vec![format!("{name}:{}", route.listen_port.unwrap_or(0))],
            Protocol::Http => {
                let mut urls = vec![];
                if https != 0 {
                    urls.push(if https == 443 { format!("https://{name}{path}") } else { format!("https://{name}:{https}{path}") });
                }
                if http != 0 && !route.https_only {
                    urls.push(if http == 80 { format!("http://{name}{path}") } else { format!("http://{name}:{http}{path}") });
                }
                urls
            }
        };
        let listen_failed = self.problems.lock().unwrap().listen_failed.contains(&route.host);
        RouteView { route: route.clone(), urls, upstream_up: None, listen_failed }
    }

    pub async fn register_route(self: &Arc<Self>, mut route: Route) -> Result<RegisterRouteResult, ApiError> {
        let _guard = self.write.lock().await;
        let config = self.config();
        self.shared.routes.read().unwrap().validate(&mut route, reserved(&config)).map_err(route_err)?;
        // A typo in a folder fails now, not as a 502 later. A saved route
        // whose folder is gone still loads: the disk may come back.
        if let Some(folder) = route.folder() {
            match std::fs::metadata(folder) {
                Ok(m) if m.is_dir() => {}
                Ok(_) => return Err(err(ErrorCode::InvalidRoute, format!("{} is not a folder", folder.display()))),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    return Err(err(
                        ErrorCode::InvalidRoute,
                        format!(
                            "cannot open folder {}: {e}. macOS keeps apps out of Desktop, Documents, Downloads and \
                             iCloud Drive: use a folder outside them, or allow {} in System Settings > Privacy & Security",
                            folder.display(),
                            self.instance.app_name()
                        ),
                    ));
                }
                Err(e) => return Err(err(ErrorCode::InvalidRoute, format!("cannot open folder {}: {e}", folder.display()))),
            }
        }
        if let Some(pid) = route.owner_pid
            && !pidwatch::alive(pid)
        {
            return Err(err(ErrorCode::ProcessNotFound, format!("process {pid} is not running")));
        }
        let key = route.key();
        let old = self.shared.routes.read().unwrap().get(&key).cloned();

        // TCP: keep the listener when the port stays, else bind first (I16).
        let mut new_listener = None;
        if route.protocol == Protocol::Tcp {
            let wanted = route.listen_port.unwrap_or(0);
            let current = self.tcp.lock().unwrap().get(&route.host).map(|h| h.port);
            match current {
                Some(port) if wanted == port || wanted == 0 && old.as_ref().is_some_and(|o| o.protocol == Protocol::Tcp) => {
                    route.listen_port = Some(port);
                }
                _ => {
                    let (listeners, port) = tcp_listen::bind_loopback(wanted).map_err(|e| {
                        err(ErrorCode::PortInUse, format!("cannot listen on 127.0.0.1:{wanted} and [::1]:{wanted}: {e}"))
                    })?;
                    route.listen_port = Some(port);
                    new_listener = Some((listeners, port));
                }
            }
        }

        let replaced = old.is_some();
        let old_target = old.as_ref().map(|o| o.target.clone());
        self.shared.routes.write().unwrap().insert(route.clone());

        let undo = |this: &Self| {
            let mut table = this.shared.routes.write().unwrap();
            match &old {
                Some(o) => table.insert(o.clone()),
                None => table.remove(&key),
            };
        };
        if route.persistent || old.as_ref().is_some_and(|o| o.persistent) {
            let persistent = self.shared.routes.read().unwrap().persistent();
            if let Err(e) = store::save_routes(&self.paths.routes(), &persistent) {
                undo(self);
                return Err(err(ErrorCode::Io, format!("could not save routes.json: {e}")));
            }
        }
        if let Some(pid) = route.owner_pid
            && let Err(e) = self.pids.watch(pid)
        {
            undo(self);
            return Err(err(ErrorCode::ProcessNotFound, format!("cannot watch process {pid}: {e}")));
        }

        // Success: switch listeners.
        if let Some((listeners, port)) = new_listener {
            self.stop_tcp(&route.host);
            self.start_tcp(&route.host, listeners, port);
        } else if route.protocol == Protocol::Http && route.path.is_none() {
            // A default HTTP route replaced a TCP route of the same host.
            self.stop_tcp(&route.host);
        }
        tracing::info!("registered {key} -> {}", route.target);
        Ok(RegisterRouteResult { route: self.view(&route), replaced, old_target })
    }

    pub async fn unregister_route(&self, p: HostParams) -> Result<UnregisterRouteResult, ApiError> {
        let _guard = self.write.lock().await;
        let host = p.host.trim().trim_end_matches(".localhost").to_ascii_lowercase();
        let path = match p.path.as_deref().map(normalize_path).transpose() {
            Ok(path) => path.flatten(),
            Err(e) => return Err(route_err(e)),
        };
        // Exactly one key; without a path, the default route only (I28).
        let key = RouteKey { host: host.clone(), path: path.unwrap_or_default() };
        let Some(old) = self.shared.routes.write().unwrap().remove(&key) else {
            return Ok(UnregisterRouteResult { removed: false });
        };
        if old.persistent {
            let persistent = self.shared.routes.read().unwrap().persistent();
            if let Err(e) = store::save_routes(&self.paths.routes(), &persistent) {
                self.shared.routes.write().unwrap().insert(old);
                return Err(err(ErrorCode::Io, format!("could not save routes.json: {e}")));
            }
        }
        if key.path.is_empty() {
            self.stop_tcp(&host);
        }
        tracing::info!("unregistered {key}");
        Ok(UnregisterRouteResult { removed: true })
    }

    pub async fn list_routes(&self) -> ListRoutesResult {
        let routes: Vec<Route> = self.shared.routes.read().unwrap().iter().cloned().collect();
        // Check every target at the same time, 200 ms each.
        let checks: Vec<_> = routes
            .iter()
            .map(|r| {
                let folder = r.folder().map(|f| f.to_path_buf());
                let addr = r.target_addr().ok().map(|t| t.socket_addr());
                tokio::spawn(async move {
                    // A folder route is up while its folder is there.
                    if let Some(folder) = folder {
                        return tokio::fs::metadata(folder).await.is_ok_and(|m| m.is_dir());
                    }
                    let Some(addr) = addr else { return false };
                    let connect = tokio::net::TcpStream::connect(addr);
                    matches!(tokio::time::timeout(Duration::from_millis(200), connect).await, Ok(Ok(_)))
                })
            })
            .collect();
        let mut views = Vec::with_capacity(routes.len());
        for (route, check) in routes.iter().zip(checks) {
            let up = check.await.unwrap_or(false);
            views.push(RouteView { upstream_up: Some(up), ..self.view(route) });
        }
        ListRoutesResult { routes: views }
    }

    pub fn find_free_port(&self, p: FindFreePortParams) -> Result<FindFreePortResult, ApiError> {
        let taken: Vec<u16> = self.shared.routes.read().unwrap().iter().filter_map(|r| r.listen_port).collect();
        let free = |port: u16| {
            !taken.contains(&port)
                && std::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).is_ok()
                && std::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port))).is_ok()
        };
        if let Some(near) = p.near.filter(|&n| n >= 1024) {
            for port in near..=near.saturating_add(200) {
                if free(port) {
                    return Ok(FindFreePortResult { port });
                }
            }
        }
        for _ in 0..20 {
            let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| err(ErrorCode::Io, e.to_string()))?;
            let port = l.local_addr().map_err(|e| err(ErrorCode::Io, e.to_string()))?.port();
            drop(l);
            if free(port) {
                return Ok(FindFreePortResult { port });
            }
        }
        Err(err(ErrorCode::PortInUse, "no free port found"))
    }

    pub fn get_logs(&self, p: GetLogsParams) -> GetLogsResult {
        GetLogsResult { entries: self.log.recent(p.host.as_deref(), p.limit.unwrap_or(100).min(10_000)) }
    }

    pub fn get_config(&self) -> Config {
        self.config()
    }

    pub async fn set_config(&self, p: SetConfigParams) -> Result<SetConfigResult, ApiError> {
        let _guard = self.write.lock().await;
        // Start from the file, not from memory: ports are edited there by
        // hand, and a Settings switch must not put the old ones back (ADR 04,
        // I8). A file that does not parse is refused rather than replaced.
        let on_disk = store::read_config(&self.paths.config(), &self.instance).map_err(|why| err(ErrorCode::Io, why))?;
        let mut new = on_disk.unwrap_or_else(|| self.config());
        if let Some(v) = p.http_port {
            new.http_port = v;
        }
        if let Some(v) = p.https_port {
            new.https_port = v;
        }
        if let Some(v) = p.fallback {
            new.fallback = v;
        }
        if let Some(v) = p.allow_lan {
            new.allow_lan = v;
        }
        if let Some(v) = p.log_size {
            if v == 0 {
                return Err(err(ErrorCode::InvalidRequest, "log_size must be at least 1"));
            }
            new.log_size = v;
        }
        if new.http_port != 0 && new.http_port == new.https_port {
            return Err(err(ErrorCode::InvalidRequest, "http_port and https_port must differ"));
        }
        store::save_config(&self.paths.config(), &new)
            .map_err(|e| err(ErrorCode::Io, format!("could not save config.json: {e}")))?;
        *self.shared.config.write().unwrap() = new.clone();
        self.log.set_capacity(new.log_size);
        let restart_needed = (new.http_port, new.https_port) != self.start_ports;
        Ok(SetConfigResult { config: new, restart_needed })
    }

    pub async fn reset_ca(&self) -> Result<ResetCaResult, ApiError> {
        let _guard = self.write.lock().await;
        let ca = LocalCa::reset(&self.paths, &self.instance).map_err(|e| err(ErrorCode::CaUnavailable, format!("could not make a new CA: {e}")))?;
        let common_name = ca.common_name().to_string();
        self.certs.set_ca(Some(ca));
        self.problems.lock().unwrap().ca = None;
        self.forget_trust_cache();
        tracing::warn!("CA reset: new CA {common_name}");
        Ok(ResetCaResult { pem_path: self.paths.ca_pem().display().to_string(), common_name })
    }
}

/// The API version this daemon speaks. Debug builds accept
/// `LOCALROUTER_TEST_API_VERSION`, so tests can play a newer daemon.
fn api_version() -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("LOCALROUTER_TEST_API_VERSION") {
        return v;
    }
    api::API_VERSION.to_string()
}

fn reserved(config: &Config) -> Reserved {
    Reserved { http_port: config.http_port, https_port: config.https_port }
}
