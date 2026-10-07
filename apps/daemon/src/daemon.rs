//! Daemon state and the socket API methods.
//!
//! Every change to routes or settings goes through [`Daemon`], one at a time
//! (`write` lock), so memory and disk never disagree (invariant I4).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use localrouter_core::api::{
    self, ApiError, CaState, CaStatus, ErrorCode, FindFreePortParams, FindFreePortResult, GetLogsParams, GetLogsResult,
    GetProxyParams, GetProxyResult, HelloParams, HelloResult, HostParams, IdParams, InspectChange, ListRoutesResult,
    LanProxyInfo, ListScriptRulesResult, NetworkStatus, NewSetupCodeParams, NewSetupCodeResult, PendingDevice, PortStatus, ProxyClientStatus, ProxyLogStatus, ProxyStatus, RegisterRouteResult, RemoveScriptRuleResult, ResetCaResult, RouteView, SetConfigParams, SetPhoneDeviceParams, SetPhoneDeviceResult,
    SetConfigResult, SetScriptRuleParams, SetScriptRuleResult, StatusResult, UnregisterRouteResult,
};
use localrouter_core::config::{Config, DEFAULT_CLIENT, LanNetwork, ProxyClient};
use localrouter_core::forward::{ClientPort, ForwardProxy};
use localrouter_core::har::{self, ClientInfo, HarLog, HarSettings};
use localrouter_core::inspect::{self, InspectSet};
use localrouter_core::logs::RequestLog;
use localrouter_core::instance::Instance;
use localrouter_core::paths::Paths;
use localrouter_core::phone::{self, LanClient, PhoneEvent};
use localrouter_core::proxy::{ClientScheme, Proxy, RouteSource};
use localrouter_core::routes::{PROXY_LOG_HOST, Protocol, Reserved, Route, RouteError, RouteKey, RouteTable, normalize_path};
use localrouter_core::scripts::engine::{ScriptKind, load_file};
use localrouter_core::scripts::rules::{self as script_rules, RuleHost};
use localrouter_core::scripts::{Scripts, bodies};
use localrouter_core::tcp::{self, TcpRouteInfo};
use localrouter_core::tls::{CaKind, CaLoad, CertStore, LocalCa};
use localrouter_core::upstream::Upstream;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::listen::{self, RefusalLog};
use crate::network::{self, Network};
use crate::pidwatch::{self, PidWatch};
use crate::{store, tcp_listen};

pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Route table and settings that the proxy and the TLS resolver read.
pub struct Shared {
    pub routes: RwLock<RouteTable>,
    pub config: RwLock<Config>,
    pub http_port: AtomicU16,
    pub https_port: AtomicU16,
    /// Hosts the forward proxy inspects (ADR 06), from `inspect_hosts`.
    pub inspect: RwLock<InspectSet>,
    /// Every bound proxy port, the main one and the clients' (ADR 09). The
    /// proxy reads it for its loop check (I9).
    pub proxy_ports: Arc<RwLock<Vec<u16>>>,
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

/// A bound forward proxy port (ADR 06), the main one or a client's (ADR 09).
/// Cancelling closes the listeners and every open proxy connection (I2).
struct ProxyHandle {
    /// The port in the config when it was bound (`0` = any port).
    configured: u16,
    port: u16,
    cancel: CancellationToken,
    /// A phone client (ADR 10): its port listens on the LAN.
    lan: Option<LanHandle>,
}

/// A phone client's token and devices, shared with its port, and the token
/// its open connections hang on. Denying a device cancels that token and
/// makes a new one under the listener's, so the listener stays bound (I11).
struct LanHandle {
    client: Arc<LanClient>,
    conns: Arc<Mutex<CancellationToken>>,
}

#[derive(Default)]
struct Problems {
    ca: Option<String>,
    routes_file: Option<String>,
    http: Option<PortStatus>,
    https: Option<PortStatus>,
    /// TCP routes whose port could not be bound at start.
    listen_failed: Vec<String>,
    /// Why the inspection CA could not be loaded or made.
    inspect_ca: Option<String>,
    /// Why the proxy port is not bound while it should be.
    proxy: Vec<String>,
    /// Why a proxy client's port is not bound while the proxy is on.
    proxy_clients: HashMap<String, String>,
    /// The one-time update of `allow_lan` to a list of networks (ADR 08).
    lan_note: Option<String>,
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
    /// The inspection CA's leaves, only for names in the inspect set (I3).
    pub inspect_certs: Arc<CertStore>,
    pub proxy: Arc<Proxy>,
    pub forward: Arc<ForwardProxy>,
    /// Script rules (ADR 07); the router and the forward proxy share them.
    pub scripts: Arc<Scripts>,
    /// The proxy log (ADR 08); the router and the forward proxy share it.
    pub har: Arc<HarLog>,
    pub shutdown: CancellationToken,
    pids: PidWatch,
    tcp: Mutex<HashMap<String, TcpHandle>>,
    proxy_listener: Mutex<Option<ProxyHandle>>,
    /// The proxy clients' ports, by name (ADR 09).
    client_listeners: Mutex<HashMap<String, ProxyHandle>>,
    /// The phone clients' setup tokens and allowed devices, as in
    /// `proxy-phones.json` (ADR 10).
    phones: Mutex<BTreeMap<String, store::PhoneRecord>>,
    /// Devices that asked to use a phone client, by client. Memory only.
    pending: Mutex<HashMap<String, Vec<PendingDevice>>>,
    /// Devices the user denied, until the daemon stops: they are not asked
    /// about again.
    denied: Mutex<HashSet<(String, std::net::IpAddr)>>,
    problems: Mutex<Problems>,
    trust_cache: TrustCache,
    inspect_trust_cache: TrustCache,
    write: tokio::sync::Mutex<()>,
    refusals: RefusalLog,
}

type TrustCache = Mutex<Option<(Instant, Option<bool>)>>;

fn err(code: ErrorCode, message: impl Into<String>) -> ApiError {
    ApiError::new(code, message)
}

fn route_err(e: RouteError) -> ApiError {
    err(ErrorCode::InvalidRoute, e.to_string())
}

fn rule_err(message: impl Into<String>) -> ApiError {
    err(ErrorCode::InvalidScriptRule, message)
}

impl Daemon {
    /// Load config, CA and routes. Does not bind anything yet.
    pub fn load(paths: Paths, instance: Instance, pids: PidWatch, ca: Option<LocalCa>, ca_problem: Option<String>) -> Arc<Self> {
        let mut config = store::load_config(&paths.config(), &instance);
        // ADR 08, I18: an old `allow_lan: true` meant every network. It
        // becomes the network the Mac is on now, never "every network".
        let mut lan_note = None;
        if config.value.allow_lan && !store::config_has_field(&paths.config(), "lan_networks") {
            lan_note = Some(match current_network() {
                Some(n) => {
                    let note = format!(
                        "LAN access now works per network. Allowed on: {} (router {}). Name it or add other networks in Settings > Routing.",
                        n.router,
                        n.id.trim_start_matches("mac:")
                    );
                    config.value.lan_networks = vec![LanNetwork { id: n.id, name: String::new(), router: n.router.to_string() }];
                    note
                }
                None => "LAN access now works per network. This network could not be recognised, so no network is allowed yet: \
                         allow one in Settings > Routing."
                    .to_string(),
            });
            if let Err(e) = store::save_config(&paths.config(), &config.value) {
                tracing::warn!("could not write config.json: {e}; the allowed network is kept in memory");
            }
        }
        let start_ports = (config.value.http_port, config.value.https_port);
        let shared = Arc::new(Shared {
            routes: RwLock::new(RouteTable::new()),
            config: RwLock::new(config.value.clone()),
            http_port: AtomicU16::new(0),
            https_port: AtomicU16::new(0),
            inspect: RwLock::new(InspectSet::new(&config.value.inspect_hosts)),
            proxy_ports: Arc::new(RwLock::new(vec![])),
        });
        if (config.problem.is_some() || !paths.config().exists())
            && let Err(e) = store::save_config(&paths.config(), &config.value)
        {
            tracing::warn!("could not write config.json: {e}");
        }
        let phones = reconcile_phones(&paths, &config.value);
        let log = Arc::new(RequestLog::new(config.value.log_size));
        let scripts = Scripts::new();
        load_saved_script_rules(&paths, &scripts);
        // A file a crash cut is repaired before anything can read it (I16).
        let har = HarLog::new(HarSettings {
            folder: paths.proxy_log_dir(),
            creator: instance.app_name(),
            version: DAEMON_VERSION.to_string(),
            enabled: config.value.proxy_log,
            file_mb: config.value.proxy_log_file_mb,
            file_requests: config.value.proxy_log_file_requests,
        });
        har.set_proxy_on(config.value.proxy_enabled);
        har.repair_at_start();
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

        // The inspection CA is loaded when it exists, never made at start (I5).
        let (inspect_ca, inspect_problem) = match LocalCa::load_existing(&paths, CaKind::Inspection) {
            None => (None, None),
            Some(CaLoad::Ready(ca)) => (Some(*ca), None),
            Some(CaLoad::Broken(why)) => {
                tracing::error!("the inspection CA cannot be used: {why}");
                (None, Some(why))
            }
        };
        let set = shared.clone();
        let inspect_certs =
            Arc::new(CertStore::inspection(inspect_ca, Arc::new(move |name: &str| set.inspect.read().unwrap().matches(name))));

        let loaded = store::load_routes(&paths.routes());
        let mut problems =
            Problems { ca: ca_problem, routes_file: loaded.problem, inspect_ca: inspect_problem, lan_note, ..Default::default() };
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
                // A route `proxy` saved before ADR 08 is kept (I11).
                match table.validate_saved(&mut route, reserved) {
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
        let daemon = Arc::new_cyclic(|this: &Weak<Self>| {
            let this_for_scripts = this.clone();
            let this_for_setup = this.clone();
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
                scripts: scripts.clone(),
                har: Some(har.clone()),
            });
            let forward = Arc::new(ForwardProxy {
                instance: instance.clone(),
                router: proxy.clone(),
                upstream: Arc::new(upstream()),
                log: log.clone(),
                local_certs: certs.clone(),
                inspect_certs: inspect_certs.clone(),
                own_ports: shared.proxy_ports.clone(),
                local_target: Arc::new(|ip| phone::is_local_target(ip, &phone::own_addresses())),
                setup_ca: {
                    let this = this_for_setup.clone();
                    Arc::new(move || this.upgrade().and_then(|d| d.setup_ca()))
                },
                phone_events: Arc::new(move |event| {
                    if let Some(d) = this_for_setup.upgrade() {
                        d.phone_event(event);
                    }
                }),
            });
            let weak = this_for_scripts.clone();
            scripts.set_on_disable(Arc::new(move |id: String| {
                let Some(this) = weak.upgrade() else { return };
                tokio::spawn(async move { this.rule_turned_off(&id).await });
            }));
            Self {
                paths,
                instance,
                start_ports,
                shared,
                log,
                certs,
                inspect_certs,
                proxy,
                forward,
                scripts,
                har,
                shutdown: CancellationToken::new(),
                pids,
                tcp: Mutex::new(HashMap::new()),
                proxy_listener: Mutex::new(None),
                client_listeners: Mutex::new(HashMap::new()),
                phones: Mutex::new(phones),
                pending: Mutex::new(HashMap::new()),
                denied: Mutex::new(HashSet::new()),
                problems: Mutex::new(problems),
                trust_cache: Mutex::new(None),
                inspect_trust_cache: Mutex::new(None),
                write: tokio::sync::Mutex::new(()),
                refusals: RefusalLog::new(),
            }
        });
        // Hosts of saved rules join the inspect set.
        daemon.refresh_inspect();
        // The viewer's client menu knows the clients before any port binds.
        daemon.refresh_proxy_ports();
        daemon
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
            // I2: decide before reading any byte. Loopback at once; another
            // machine after a network lookup off the accept loop (ADR 08).
            if listen::is_loopback_peer(peer.ip()) {
                self.serve_accepted(stream, peer, scheme, &acceptor);
                continue;
            }
            let this = self.clone();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let local = stream.local_addr().ok();
                let check = this.clone();
                let allowed =
                    tokio::task::spawn_blocking(move || check.lan_peer_allowed(peer.ip(), local)).await.unwrap_or(false);
                if allowed {
                    this.serve_accepted(stream, peer, scheme, &acceptor);
                } else {
                    this.refusals.refused(peer);
                }
            });
        }
    }

    /// Serve a connection the peer check accepted.
    fn serve_accepted(&self, stream: tokio::net::TcpStream, peer: SocketAddr, scheme: ClientScheme, acceptor: &tokio_rustls::TlsAcceptor) {
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

    /// Ports 80 and 443: loopback always; another machine only with
    /// `allow_lan` on an allowed network (ADR 08, I17). The network is
    /// looked up only for such a peer, and never cached (I20).
    fn lan_peer_allowed(&self, peer: std::net::IpAddr, local: Option<SocketAddr>) -> bool {
        if listen::is_loopback_peer(peer) {
            return true;
        }
        let (allow_lan, networks) = {
            let c = self.shared.config.read().unwrap();
            (c.allow_lan, if c.allow_lan { c.lan_networks.clone() } else { vec![] })
        };
        listen::peer_allowed(peer, allow_lan, &networks, || network_of(local?.ip()).map(|n| n.id))
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

    // ---- forward proxy port (ADR 06)

    /// Bind the proxy port at start when the config says so. A taken port is
    /// reported in `status.proxy`, not fatal.
    pub fn start_saved_proxy(self: &Arc<Self>) {
        let config = self.config();
        if !config.proxy_enabled {
            return;
        }
        match bind_proxy(config.proxy_port) {
            Ok((listeners, port)) => self.start_proxy(listeners, config.proxy_port, port),
            Err(e) => {
                tracing::error!("the forward proxy is off: {}", e.message);
                self.problems.lock().unwrap().proxy = vec![e.message];
            }
        }
        // A client's taken port does not stop the others (ADR 09).
        for client in &config.proxy_clients {
            match bind_client(client) {
                Ok((listeners, port)) => self.start_client(&client.name, listeners, client.port, port, client.lan),
                Err(e) => self.client_failed(&client.name, e.message),
            }
        }
        self.refresh_proxy_ports();
    }

    fn start_proxy(self: &Arc<Self>, listeners: Vec<std::net::TcpListener>, configured: u16, port: u16) {
        let cancel = self.shutdown.child_token();
        self.accept_proxy(listeners, ClientPort { client: None, port, lan: None }, cancel.clone(), None);
        self.har.set_proxy_on(true);
        self.problems.lock().unwrap().proxy.clear();
        if let Some(old) = self.proxy_listener.lock().unwrap().replace(ProxyHandle { configured, port, cancel, lan: None }) {
            old.cancel.cancel();
        }
        tracing::info!("forward proxy on 127.0.0.1:{port} and [::1]:{port}");
    }

    /// Start one proxy client's port (ADR 09). A handle with the same name
    /// is replaced, and its connections closed.
    fn start_client(self: &Arc<Self>, name: &str, listeners: Vec<std::net::TcpListener>, configured: u16, port: u16, lan: bool) {
        let cancel = self.shutdown.child_token();
        let lan = lan.then(|| {
            let record = self.phones.lock().unwrap().get(name).cloned().unwrap_or_default();
            let devices = record.devices.iter().filter_map(|d| d.parse().ok()).collect();
            LanHandle { client: Arc::new(LanClient::new(name, &record.token, devices)), conns: Arc::new(Mutex::new(cancel.child_token())) }
        });
        let at = ClientPort { client: Some(Arc::from(name)), port, lan: lan.as_ref().map(|l| l.client.clone()) };
        self.accept_proxy(listeners, at, cancel.clone(), lan.as_ref().map(|l| l.conns.clone()));
        self.problems.lock().unwrap().proxy_clients.remove(name);
        let place = if lan.is_some() { format!("0.0.0.0:{port} and [::]:{port} (phone)") } else { format!("127.0.0.1:{port} and [::1]:{port}") };
        if let Some(old) = self.client_listeners.lock().unwrap().insert(name.to_string(), ProxyHandle { configured, port, cancel, lan }) {
            old.cancel.cancel();
        }
        tracing::info!("proxy client {name} on {place}");
    }

    fn client_failed(&self, name: &str, why: String) {
        tracing::error!("proxy client {name} is not listening: {why}");
        self.problems.lock().unwrap().proxy_clients.insert(name.to_string(), why);
    }

    /// Close one proxy client's port and its open connections.
    fn stop_client(&self, name: &str) {
        if let Some(handle) = self.client_listeners.lock().unwrap().remove(name) {
            handle.cancel.cancel();
            tracing::info!("proxy client {name} off");
        }
        self.problems.lock().unwrap().proxy_clients.remove(name);
    }

    /// The ports the loop check knows, and the viewer's client menu: from
    /// the config and the bound handles. Called after every change.
    fn refresh_proxy_ports(&self) {
        let main = self.proxy_listener.lock().unwrap().as_ref().map(|h| h.port);
        let clients: Vec<(String, Option<u16>)> = {
            let bound = self.client_listeners.lock().unwrap();
            self.config().proxy_clients.iter().map(|c| (c.name.clone(), bound.get(&c.name).map(|h| h.port))).collect()
        };
        *self.shared.proxy_ports.write().unwrap() = main.into_iter().chain(clients.iter().filter_map(|c| c.1)).collect();
        let mut menu = vec![ClientInfo { name: DEFAULT_CLIENT.to_string(), port: main }];
        menu.extend(clients.into_iter().map(|(name, port)| ClientInfo { name, port }));
        self.har.set_clients(menu);
    }

    /// Accept on one proxy port until `cancel` is cancelled. A phone
    /// client's connections hang on `conns` (ADR 10, I11).
    fn accept_proxy(
        self: &Arc<Self>,
        listeners: Vec<std::net::TcpListener>,
        at: ClientPort,
        cancel: CancellationToken,
        conns: Option<Arc<Mutex<CancellationToken>>>,
    ) {
        fn token(cancel: &CancellationToken, conns: &Option<Arc<Mutex<CancellationToken>>>) -> CancellationToken {
            match conns {
                Some(c) => c.lock().unwrap().child_token(),
                None => cancel.child_token(),
            }
        }
        for l in listeners {
            let Ok(listener) = TcpListener::from_std(l) else { continue };
            let this = self.clone();
            let cancel = cancel.clone();
            let at = at.clone();
            let conns = conns.clone();
            tokio::spawn(async move {
                loop {
                    let (stream, peer) = tokio::select! {
                        r = listener.accept() => match r {
                            Ok(x) => x,
                            Err(_) => { tokio::time::sleep(Duration::from_millis(50)).await; continue; }
                        },
                        _ = cancel.cancelled() => return,
                    };
                    // Loopback only, whatever allow_lan says (I1). The
                    // sockets are bound to loopback; this is the second check.
                    // A phone client takes another machine on the same terms
                    // as ports 80 and 443, decided off the accept loop (ADR 10, I2).
                    if !listen::is_loopback_peer(peer.ip()) {
                        if at.lan.is_none() {
                            this.refusals.refused(peer);
                            continue;
                        }
                        let (this, at, cancel, conns) = (this.clone(), at.clone(), cancel.clone(), conns.clone());
                        tokio::spawn(async move {
                            let local = stream.local_addr().ok();
                            let check = this.clone();
                            let allowed =
                                tokio::task::spawn_blocking(move || check.lan_peer_allowed(peer.ip(), local)).await.unwrap_or(false);
                            if allowed {
                                let _ = stream.set_nodelay(true);
                                tokio::spawn(this.forward.clone().serve(stream, peer, at, token(&cancel, &conns)));
                            } else {
                                this.refusals.refused(peer);
                            }
                        });
                        continue;
                    }
                    let _ = stream.set_nodelay(true);
                    tokio::spawn(this.forward.clone().serve(stream, peer, at.clone(), token(&cancel, &conns)));
                }
            });
        }
    }

    /// Close the proxy listeners, the clients' too, and every open proxy
    /// connection (I2).
    fn stop_proxy(&self) {
        if let Some(handle) = self.proxy_listener.lock().unwrap().take() {
            handle.cancel.cancel();
            tracing::info!("forward proxy off");
        }
        let names: Vec<String> = self.client_listeners.lock().unwrap().keys().cloned().collect();
        for name in names {
            self.stop_client(&name);
        }
        self.har.set_proxy_on(false);
        let mut problems = self.problems.lock().unwrap();
        problems.proxy.clear();
        problems.proxy_clients.clear();
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
        let ids = self.scripts.owned_by(pid);
        for id in &ids {
            self.scripts.remove(id);
            tracing::info!("removed script rule {id}: owner process {pid} exited");
        }
        if !ids.is_empty() {
            self.refresh_inspect();
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
        let proxy = self.proxy_status().await;
        let network = self.network_status().await;
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
        let mut notes: Vec<String> = problems.lan_note.iter().cloned().collect();
        drop(problems);
        // A route `proxy` saved before ADR 08 wins over the viewer (I11).
        if self.shared.routes.read().unwrap().routes_of(PROXY_LOG_HOST).next().is_some() {
            let help = localrouter_core::instance::help_url(self.routes_http_port(), self.routes_https_port());
            notes.push(format!("The route {PROXY_LOG_HOST} hides the proxy log viewer. It is also at {help}/proxy-log/."));
        }

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
            proxy: Some(proxy),
            network,
            notes,
        }
    }

    /// The network of the default route, read now (ADR 08, change 4).
    async fn network_status(&self) -> Option<NetworkStatus> {
        let n = tokio::task::spawn_blocking(current_network).await.ok().flatten()?;
        let config = self.config();
        let saved = config.lan_networks.iter().find(|l| l.id == n.id);
        Some(NetworkStatus {
            lan_allowed: config.allow_lan && saved.is_some(),
            name: saved.map(|l| l.name.clone()).unwrap_or_default(),
            id: n.id,
            router: n.router.to_string(),
            interface: n.interface,
        })
    }

    fn routes_http_port(&self) -> Option<u16> {
        Some(self.shared.http_port.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }

    fn routes_https_port(&self) -> Option<u16> {
        Some(self.shared.https_port.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }

    async fn proxy_status(&self) -> ProxyStatus {
        let config = self.config();
        let bound_port = self.proxy_listener.lock().unwrap().as_ref().map(|h| h.port);
        let bound = bound_port.map_or_else(Vec::new, |p| vec![format!("127.0.0.1:{p}"), format!("[::1]:{p}")]);
        let errors = self.problems.lock().unwrap().proxy.clone();
        ProxyStatus {
            enabled: config.proxy_enabled,
            configured: config.proxy_port,
            port: bound_port,
            bound,
            errors,
            inspect_ca: self.inspect_ca_status().await,
            clients: self.client_statuses(&config),
        }
    }

    /// Each proxy client of the config with its bound port (ADR 09).
    fn client_statuses(&self, config: &Config) -> Vec<ProxyClientStatus> {
        let bound = self.client_listeners.lock().unwrap();
        let problems = self.problems.lock().unwrap();
        let pending = self.pending.lock().unwrap();
        config
            .proxy_clients
            .iter()
            .map(|c| {
                let handle = bound.get(&c.name);
                let port = handle.map(|h| h.port);
                let on_lan = handle.is_some_and(|h| h.lan.is_some());
                ProxyClientStatus {
                    name: c.name.clone(),
                    configured: c.port,
                    port,
                    bound: match (port, on_lan) {
                        (None, _) => vec![],
                        (Some(p), false) => vec![format!("127.0.0.1:{p}"), format!("[::1]:{p}")],
                        (Some(p), true) => vec![format!("0.0.0.0:{p}"), format!("[::]:{p}")],
                    },
                    errors: problems.proxy_clients.get(&c.name).cloned().into_iter().collect(),
                    lan: c.lan,
                    pending: pending.get(&c.name).cloned().unwrap_or_default(),
                }
            })
            .collect()
    }

    /// `None` until the inspection CA exists (I5).
    async fn inspect_ca_status(&self) -> Option<CaStatus> {
        let pem_path = self.paths.inspect_ca_pem().display().to_string();
        if self.inspect_certs.has_ca() {
            let trusted = check_trust(self.paths.inspect_ca_pem(), &self.inspect_trust_cache).await;
            return Some(CaStatus { state: CaState::Ok, problem: None, pem_path, common_name: self.inspect_certs.common_name(), trusted });
        }
        let problem = self.problems.lock().unwrap().inspect_ca.clone()?;
        Some(CaStatus { state: CaState::Broken, problem: Some(problem), pem_path, common_name: None, trusted: None })
    }

    /// Ask macOS whether `ca.pem` is trusted for TLS. Cached for 10 seconds.
    async fn ca_trusted(&self) -> Option<bool> {
        check_trust(self.paths.ca_pem(), &self.trust_cache).await
    }

    pub fn forget_trust_cache(&self) {
        *self.trust_cache.lock().unwrap() = None;
        *self.inspect_trust_cache.lock().unwrap() = None;
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

    pub async fn set_config(self: &Arc<Self>, p: SetConfigParams) -> Result<SetConfigResult, ApiError> {
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
        if let Some(v) = p.proxy_enabled {
            new.proxy_enabled = v;
        }
        if let Some(v) = p.proxy_port {
            new.proxy_port = v;
        }
        if let Some(list) = &p.inspect_hosts {
            new.inspect_hosts = inspect::normalize(list).map_err(|why| err(ErrorCode::InvalidRequest, why))?;
        }
        if let Some(v) = p.proxy_log {
            new.proxy_log = v;
        }
        if let Some(v) = p.proxy_log_file_mb {
            new.proxy_log_file_mb = v;
        }
        if let Some(v) = p.proxy_log_file_requests {
            new.proxy_log_file_requests = v;
        }
        if p.proxy_log_file_mb.is_some() || p.proxy_log_file_requests.is_some() {
            Config::check_proxy_log_limits(new.proxy_log_file_mb, new.proxy_log_file_requests)
                .map_err(|why| err(ErrorCode::InvalidRequest, why))?;
        }
        // The list changes only here, so memory is right even when the
        // one-time update could not be written (I18).
        new.lan_networks = match &p.lan_networks {
            Some(list) => normalize_networks(list).map_err(|why| err(ErrorCode::InvalidRequest, why))?,
            None => self.config().lan_networks,
        };
        if new.http_port != 0 && new.http_port == new.https_port {
            return Err(err(ErrorCode::InvalidRequest, "http_port and https_port must differ"));
        }
        // On macOS a loopback socket can bind next to the wildcard one of
        // port 80 and take its loopback traffic: refuse the shared ports.
        if new.proxy_port != 0 && (new.proxy_port == new.http_port || new.proxy_port == new.https_port) {
            return Err(err(ErrorCode::InvalidRequest, "proxy_port must differ from http_port and https_port"));
        }
        if let Some(list) = &p.proxy_clients {
            new.proxy_clients = list.clone();
        }
        check_clients(&new).map_err(|why| err(ErrorCode::InvalidRequest, why))?;

        // The proxy port: bind the new pair before anything is written, so a
        // taken port changes nothing (I12). Only when the call names a proxy
        // field: another switch must not fail on a port taken at start.
        let current = self.proxy_listener.lock().unwrap().as_ref().map(|h| h.configured);
        let mut new_listener = None;
        let mut stop_listener = false;
        if p.proxy_enabled.is_some() || p.proxy_port.is_some() {
            if !new.proxy_enabled {
                stop_listener = current.is_some();
            } else if current != Some(new.proxy_port) {
                new_listener = Some(bind_proxy(new.proxy_port)?);
            }
        }
        // The clients' ports (ADR 09), only when the call names the proxy or
        // the clients. A client this call adds or moves must bind, as above;
        // the others bind if they can, and a failure is reported in status.
        let mut client_binds: Vec<(String, u16, bool, Result<Bound, ApiError>)> = vec![];
        let mut client_stops: Vec<String> = vec![];
        if p.proxy_enabled.is_some() || p.proxy_clients.is_some() {
            let old = self.config().proxy_clients;
            // A client is bound as wanted when its port and its LAN switch
            // match: turning `lan` on binds the wildcard pair (ADR 10).
            let bound: HashMap<String, (u16, bool)> =
                self.client_listeners.lock().unwrap().iter().map(|(n, h)| (n.clone(), (h.configured, h.lan.is_some()))).collect();
            let wanted: &[ProxyClient] = if new.proxy_enabled { &new.proxy_clients } else { &[] };
            for name in bound.keys() {
                if !wanted.iter().any(|c| c.name == *name && bound.get(name) == Some(&(c.port, c.lan))) {
                    client_stops.push(name.clone());
                }
            }
            for c in wanted {
                if bound.get(&c.name) == Some(&(c.port, c.lan)) {
                    continue;
                }
                let named = p.proxy_clients.is_some() && !old.contains(c);
                // A client that only switches `lan` keeps its port: its old
                // listener must close before the new pair can bind.
                if bound.get(&c.name).is_some_and(|(port, _)| *port == c.port && c.port != 0) {
                    self.stop_client(&c.name);
                    client_stops.retain(|n| n != &c.name);
                }
                let bind = bind_client(c);
                if named && let Err(e) = bind {
                    return Err(err(e.code, format!("proxy client {}: {}", c.name, e.message)));
                }
                client_binds.push((c.name.clone(), c.port, c.lan, bind));
            }
        }

        // The inspection CA is made the first time the inspect set is not
        // empty (I5), and never replaced here (I4). The set is `*` by
        // default, so turning the proxy on makes it too; a CA that cannot
        // be made then leaves every CONNECT a tunnel instead of failing.
        if p.inspect_hosts.is_some() && !new.inspect_hosts.is_empty() {
            self.ensure_inspect_ca().map_err(|why| err(ErrorCode::CaUnavailable, why))?;
        } else if p.proxy_enabled == Some(true)
            && !new.inspect_hosts.is_empty()
            && let Err(why) = self.ensure_inspect_ca()
        {
            tracing::warn!("{why}; CONNECTs stay tunnels");
        }

        // A phone client's token is written before the config names the
        // client, so a phone client never lacks one (ADR 10, I12).
        let old_phones = self.phones.lock().unwrap().clone();
        let new_phones = phones_for(&new, &old_phones);
        if new_phones != old_phones {
            store::save_phones(&self.paths.proxy_phones(), &new_phones)
                .map_err(|e| err(ErrorCode::Io, format!("could not save the phone clients: {e}")))?;
        }
        // A failed write drops the new listeners unused (I4).
        if let Err(e) = store::save_config(&self.paths.config(), &new) {
            if new_phones != old_phones {
                let _ = store::save_phones(&self.paths.proxy_phones(), &old_phones);
            }
            return Err(err(ErrorCode::Io, format!("could not save config.json: {e}")));
        }
        self.pending.lock().unwrap().retain(|name, _| new_phones.contains_key(name));
        *self.phones.lock().unwrap() = new_phones;
        *self.shared.config.write().unwrap() = new.clone();
        // New CONNECTs use the new set; open tunnels stay tunnels.
        self.refresh_inspect();
        self.log.set_capacity(new.log_size);
        self.har.set_limits(new.proxy_log_file_mb, new.proxy_log_file_requests);
        self.har.set_enabled(new.proxy_log);
        if let Some((listeners, port)) = new_listener {
            self.start_proxy(listeners, new.proxy_port, port);
        } else if stop_listener {
            self.stop_proxy();
        }
        for name in client_stops {
            self.stop_client(&name);
        }
        for (name, configured, lan, bind) in client_binds {
            match bind {
                Ok((listeners, port)) => self.start_client(&name, listeners, configured, port, lan),
                Err(e) => self.client_failed(&name, e.message),
            }
        }
        self.refresh_proxy_ports();
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

    /// Everything a client needs to use the forward proxy (ADR 06, change 3).
    pub async fn get_proxy(&self, p: GetProxyParams) -> Result<GetProxyResult, ApiError> {
        let config = self.config();
        let status = self.proxy_status().await;
        // A client's reply has its port, its own Chrome profile and its page
        // of the viewer (ADR 09). `default` is the main port.
        let client = match p.client.as_deref() {
            None | Some(DEFAULT_CLIENT) => None,
            Some(name) => Some(status.clients.iter().find(|c| c.name == name).cloned().ok_or_else(|| {
                err(ErrorCode::NotFound, format!("no proxy client {name}; add it: {} proxy client add {name}", self.instance.cli()))
            })?),
        };
        let (port, bound, errors) = match &client {
            None => (status.port.unwrap_or(config.proxy_port), status.bound.clone(), status.errors.clone()),
            Some(c) => (c.port.unwrap_or(c.configured), c.bound.clone(), c.errors.clone()),
        };
        let name = client.as_ref().map(|c| c.name.clone());
        let url = format!("http://127.0.0.1:{port}");
        let cli = self.instance.cli();
        let mut env = std::collections::BTreeMap::new();
        env.insert("HTTPS_PROXY".to_string(), url.clone());
        env.insert("HTTP_PROXY".to_string(), url.clone());
        // Dev servers are reached directly, as without the proxy.
        env.insert("NO_PROXY".to_string(), "localhost,127.0.0.1,::1,.localhost".to_string());
        // The built-in fetch of recent Node.js reads HTTPS_PROXY only with this.
        env.insert("NODE_USE_ENV_PROXY".to_string(), "1".to_string());
        // curl reads a plain-http proxy only from the lowercase name.
        env.insert("http_proxy".to_string(), url.clone());
        env.insert("https_proxy".to_string(), url.clone());
        env.insert("no_proxy".to_string(), "localhost,127.0.0.1,::1,.localhost".to_string());
        let ca_ready = status.inspect_ca.as_ref().is_some_and(|ca| ca.state == CaState::Ok);
        if ca_ready {
            env.insert("NODE_EXTRA_CA_CERTS".to_string(), self.paths.inspect_ca_pem().display().to_string());
            // Python and curl read one CA file that replaces the default
            // roots: the bundle holds the system roots too (ADR 08, I23).
            let bundle = self.paths.inspect_ca_bundle();
            if bundle.is_file() {
                for name in ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"] {
                    env.insert(name.to_string(), bundle.display().to_string());
                }
            }
        }
        let chrome_args = vec![
            format!("--user-data-dir={}", self.paths.chrome_profile_for(name.as_deref()).display()),
            format!("--proxy-server={url}"),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
        ];
        let (inspect_set, inspects_all) = {
            let set = self.shared.inspect.read().unwrap();
            (set.patterns(), set.inspects_all())
        };
        let mut notes = vec![];
        if !config.proxy_enabled {
            notes.push(format!("The proxy is off. Programs that use it cannot connect. Turn it on: {cli} proxy on"));
        } else if bound.is_empty() {
            notes.push(format!("The proxy port is not bound: {}", errors.join("; ")));
        }
        match &status.inspect_ca {
            Some(ca) if ca.state == CaState::Broken => notes.push(format!(
                "The inspection CA cannot be used: {}. Inspected hosts fail. Run: {cli} proxy ca reset --yes",
                ca.problem.as_deref().unwrap_or("unknown problem")
            )),
            Some(ca) if !inspect_set.is_empty() && ca.trusted == Some(false) => notes.push(format!(
                "The inspection CA is not trusted in the login keychain: Chrome will refuse inspected hosts. Run: {cli} proxy trust"
            )),
            _ => {}
        }
        if inspects_all {
            notes.push(format!(
                "The inspect set holds '*': every HTTPS host is inspected, none is tunnelled. Programs that do not trust the inspection CA, and apps that pin certificates, fail on every host. It comes from inspect_hosts ({cli} proxy inspect rm '*') or a script rule with host '*'."
            ));
        }
        let mut log = self.proxy_log_status();
        if let Some(name) = &name {
            log.url = format!("{}/{name}", log.url);
        }
        let lan = match &client {
            Some(c) if c.lan => Some(self.lan_info(&c.name, port, !bound.is_empty()).await),
            _ => None,
        };
        Ok(GetProxyResult {
            enabled: config.proxy_enabled,
            client: name,
            url,
            port,
            bound,
            errors,
            inspect_hosts: config.inspect_hosts,
            inspect_set,
            inspect_ca: status.inspect_ca,
            env,
            chrome_args,
            notes,
            script_rules: self.scripts.views(),
            log: Some(log),
            clients: status.clients,
            lan,
        })
    }

    /// What a phone needs to use its client (ADR 10): this Mac's address on
    /// the current network, the setup URL, the devices, and why it cannot
    /// work now.
    async fn lan_info(&self, name: &str, port: u16, bound: bool) -> LanProxyInfo {
        let config = self.config();
        let cli = self.instance.cli();
        let network = tokio::task::spawn_blocking(current_network).await.ok().flatten();
        let address = match network.clone() {
            Some(n) => tokio::task::spawn_blocking(move || lan_address(&n)).await.ok().flatten(),
            None => None,
        };
        let saved = network.as_ref().and_then(|n| config.lan_networks.iter().find(|l| l.id == n.id));
        let record = self.phones.lock().unwrap().get(name).cloned().unwrap_or_default();
        let mut problems = vec![];
        if !config.proxy_enabled {
            problems.push(format!("The proxy is off. Turn it on: {cli} proxy on"));
        } else if !bound {
            problems.push("The phone port is not listening; see the proxy client's errors.".to_string());
        }
        if !config.allow_lan {
            problems.push("LAN access is off, so no other machine can connect.".to_string());
        }
        match (&network, saved) {
            (None, _) => problems.push("This network cannot be recognised (no router, or a VPN), so LAN access does not work on it.".to_string()),
            (Some(_), None) => problems.push(format!("This network is not allowed for LAN access. Allow it: {cli} lan allow")),
            _ => {}
        }
        if let (Some(n), None) = (&network, address) {
            problems.push(format!("This Mac has no IPv4 address on {}.", n.interface));
        }
        LanProxyInfo {
            address: address.map(|a| a.to_string()),
            port,
            setup_url: address.map(|a| format!("http://{a}:{port}/setup/{}", record.token)),
            devices: record.devices,
            pending: self.pending.lock().unwrap().get(name).cloned().unwrap_or_default(),
            problems,
        }
    }

    /// The phone client `name`, or the error a call about it gets.
    fn phone_client(&self, name: &str) -> Result<ProxyClient, ApiError> {
        let config = self.config();
        let Some(client) = config.proxy_clients.iter().find(|c| c.name == name) else {
            return Err(err(ErrorCode::NotFound, format!("no proxy client {name}")));
        };
        if !client.lan {
            return Err(err(
                ErrorCode::InvalidRequest,
                format!("proxy client {name} is not a phone client; add one: {} proxy client add <name> --lan", self.instance.cli()),
            ));
        }
        Ok(client.clone())
    }

    /// A new setup token: the old QR code stops allowing devices; allowed
    /// devices stay.
    pub async fn new_setup_code(&self, p: NewSetupCodeParams) -> Result<NewSetupCodeResult, ApiError> {
        let _guard = self.write.lock().await;
        let client = self.phone_client(&p.client)?;
        let token = phone::new_token();
        let mut map = self.phones.lock().unwrap().clone();
        map.entry(client.name.clone()).or_default().token = token.clone();
        store::save_phones(&self.paths.proxy_phones(), &map)
            .map_err(|e| err(ErrorCode::Io, format!("could not save the phone clients: {e}")))?;
        *self.phones.lock().unwrap() = map;
        let port = {
            let handles = self.client_listeners.lock().unwrap();
            let handle = handles.get(&client.name);
            if let Some(lan) = handle.and_then(|h| h.lan.as_ref()) {
                lan.client.set_token(&token);
            }
            handle.map(|h| h.port)
        };
        let info = self.lan_info(&client.name, port.unwrap_or(client.port), port.is_some()).await;
        tracing::info!("proxy client {}: new setup code", client.name);
        Ok(NewSetupCodeResult { setup_url: info.setup_url })
    }

    /// Allow a device on a phone client, or deny it. Denying removes it from
    /// the allowed list, closes the client's open connections, and stops
    /// asking about it until the daemon restarts (I11).
    pub async fn set_phone_device(&self, p: SetPhoneDeviceParams) -> Result<SetPhoneDeviceResult, ApiError> {
        let _guard = self.write.lock().await;
        let client = self.phone_client(&p.client)?;
        let ip: std::net::IpAddr =
            p.address.trim().parse().map_err(|_| err(ErrorCode::InvalidRequest, format!("{} is not an IP address", p.address)))?;
        let ip = ip.to_canonical();
        let mut map = self.phones.lock().unwrap().clone();
        let record = map.entry(client.name.clone()).or_default();
        record.devices.retain(|d| d.parse::<std::net::IpAddr>().map(|x| x.to_canonical()) != Ok(ip));
        if p.allow {
            record.devices.push(ip.to_string());
        }
        let devices = record.devices.clone();
        store::save_phones(&self.paths.proxy_phones(), &map)
            .map_err(|e| err(ErrorCode::Io, format!("could not save the phone clients: {e}")))?;
        *self.phones.lock().unwrap() = map;
        if let Some(list) = self.pending.lock().unwrap().get_mut(&client.name) {
            list.retain(|d| d.address != ip.to_string());
        }
        {
            let mut denied = self.denied.lock().unwrap();
            if p.allow {
                denied.remove(&(client.name.clone(), ip));
            } else {
                denied.insert((client.name.clone(), ip));
            }
        }
        if let Some(h) = self.client_listeners.lock().unwrap().get(&client.name)
            && let Some(lan) = &h.lan
        {
            if p.allow {
                lan.client.allow(ip);
            } else {
                lan.client.forget(ip);
                let mut conns = lan.conns.lock().unwrap();
                conns.cancel();
                *conns = h.cancel.child_token();
            }
        }
        tracing::info!("proxy client {}: {} {ip}", client.name, if p.allow { "allowed" } else { "denied" });
        Ok(SetPhoneDeviceResult { devices })
    }

    /// The inspection CA for a phone's setup page, while HTTPS is inspected.
    fn setup_ca(&self) -> Option<(Vec<u8>, String)> {
        let inspecting = !self.shared.inspect.read().unwrap().patterns().is_empty();
        if inspecting { self.inspect_certs.ca_certificate() } else { None }
    }

    /// From a phone port: a device opened the setup page (it is allowed now;
    /// save it), or a device that is not allowed asked (the user decides).
    fn phone_event(self: &Arc<Self>, event: PhoneEvent) {
        match event {
            PhoneEvent::Scanned { client, ip } => {
                let address = ip.to_canonical().to_string();
                {
                    let mut phones = self.phones.lock().unwrap();
                    let Some(record) = phones.get_mut(&client) else { return };
                    if !record.devices.contains(&address) {
                        record.devices.push(address.clone());
                    }
                }
                if let Some(list) = self.pending.lock().unwrap().get_mut(&client) {
                    list.retain(|d| d.address != address);
                }
                self.denied.lock().unwrap().remove(&(client.clone(), ip.to_canonical()));
                tracing::info!("proxy client {client}: {address} opened the setup page and is allowed");
                let this = self.clone();
                tokio::spawn(async move {
                    let _guard = this.write.lock().await;
                    let phones = this.phones.lock().unwrap().clone();
                    if let Err(e) = store::save_phones(&this.paths.proxy_phones(), &phones) {
                        tracing::warn!("could not save the phone clients: {e}");
                    }
                });
            }
            PhoneEvent::Asked { client, ip, host } => {
                if self.denied.lock().unwrap().contains(&(client.clone(), ip.to_canonical())) {
                    return;
                }
                let address = ip.to_canonical().to_string();
                let at_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                let mut pending = self.pending.lock().unwrap();
                let list = pending.entry(client).or_default();
                list.retain(|d| d.address != address);
                list.push(PendingDevice { address, host, at_ms });
                // A device that keeps asking stays; the oldest of many goes.
                if list.len() > 10 {
                    list.remove(0);
                }
            }
        }
    }

    /// The `log` block of `get_proxy` (ADR 08, change 3).
    fn proxy_log_status(&self) -> ProxyLogStatus {
        let (file_mb, file_requests) = self.har.limits();
        ProxyLogStatus {
            enabled: self.har.is_on(),
            folder: self.har.folder().display().to_string(),
            url: localrouter_core::instance::proxy_log_url(self.routes_http_port(), self.routes_https_port()),
            file_mb,
            file_requests,
            keep_files: har::KEEP_FILES,
            current: self.har.current(),
            files: self.har.files().len(),
            written: self.har.written(),
            dropped: self.har.dropped(),
            error: self.har.error(),
        }
    }

    // ---- script rules (ADR 07)

    /// The inspect set: `inspect_hosts` of the config and the hosts of
    /// enabled script rules outside `.localhost` (I11).
    fn refresh_inspect(&self) {
        let mut hosts = self.config().inspect_hosts;
        for h in self.scripts.inspected_hosts() {
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
        *self.shared.inspect.write().unwrap() = InspectSet::new(&hosts);
    }

    /// Make the inspection CA when it does not exist yet. `Ok(true)` when it
    /// was made now. Never replaces one (ADR 06, I4).
    fn ensure_inspect_ca(&self) -> Result<bool, String> {
        if self.inspect_certs.has_ca() {
            return Ok(false);
        }
        if let Some(problem) = self.problems.lock().unwrap().inspect_ca.clone() {
            return Err(format!("the inspection CA cannot be used: {problem}. Run `{} proxy ca reset --yes`", self.instance.cli()));
        }
        match LocalCa::load_or_create_kind(&self.paths, &self.instance, CaKind::Inspection) {
            CaLoad::Ready(ca) => {
                tracing::info!("made the inspection CA {}", ca.common_name());
                self.inspect_certs.set_ca(Some(*ca));
                self.forget_trust_cache();
                if let Err(e) = write_ca_bundle(&self.paths) {
                    tracing::warn!("could not write the CA bundle: {e}");
                }
                Ok(true)
            }
            CaLoad::Broken(why) => Err(why),
        }
    }

    /// Create and delete a file in `output_dir`, so a folder macOS keeps the
    /// daemon out of is refused now, not at the first write.
    fn probe_output_dir(&self, dir: &str) -> Result<(), ApiError> {
        let path = std::path::Path::new(dir);
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(rule_err(format!("output_dir {dir} is not a folder"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(rule_err(format!("output_dir {dir} does not exist; create it first")));
            }
            Err(e) => return Err(rule_err(self.privacy_hint(dir, &e))),
        }
        let probe = path.join(format!(".lr-probe-{}-{}", std::process::id(), localrouter_core::scripts::new_id()));
        match bodies::create_new(&probe) {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                Ok(())
            }
            Err(e) => Err(rule_err(self.privacy_hint(dir, &e))),
        }
    }

    fn privacy_hint(&self, dir: &str, e: &std::io::Error) -> String {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "cannot write in output_dir {dir}: {e}. macOS keeps apps out of Desktop, Documents, Downloads and iCloud \
                 Drive: use a folder outside them, or allow {} in System Settings > Privacy & Security",
                self.instance.app_name()
            )
        } else {
            format!("cannot write in output_dir {dir}: {e}")
        }
    }

    /// `set_script_rule`: check the fields, the folder and the script; then,
    /// unless `check_only`, store the rule (ADR 07, flow F3).
    pub async fn set_script_rule(self: &Arc<Self>, p: SetScriptRuleParams) -> Result<SetScriptRuleResult, ApiError> {
        let _guard = self.write.lock().await;
        let mut rule = p.rule;
        let id = rule.id.trim().to_ascii_lowercase();
        let others = self.scripts.rules().iter().filter(|r| r.id != id).count();
        script_rules::validate(&mut rule, &self.paths.data, others).map_err(rule_err)?;
        let path = std::path::PathBuf::from(&rule.script);
        let loaded = tokio::task::spawn_blocking(move || load_file(&path))
            .await
            .map_err(|e| err(ErrorCode::Io, e.to_string()))?
            .map_err(rule_err)?;
        match loaded.info.kind {
            ScriptKind::Log => {
                if rule.on_error.is_some() {
                    return Err(rule_err("on_error is for intercept rules; a log rule never affects traffic"));
                }
                match rule.output_dir.clone() {
                    Some(dir) => self.probe_output_dir(&dir)?,
                    // `check_only` may test a script before its folder exists.
                    None if p.check_only => {}
                    None => {
                        return Err(rule_err(format!("{} is a log script: give output_dir, the folder it writes", loaded.name)));
                    }
                }
            }
            ScriptKind::Intercept => {
                if rule.output_dir.is_some() {
                    return Err(rule_err(format!("{} is an intercept script: output_dir is for log rules", loaded.name)));
                }
                if rule.max_capture_bytes.is_some() {
                    return Err(rule_err("max_capture_bytes is for log rules"));
                }
            }
        }
        let host = RuleHost::parse(&rule.host).map_err(rule_err)?;
        let config = self.config();
        let mut notes = vec![];
        if p.check_only {
            if loaded.info.kind == ScriptKind::Log && rule.output_dir.is_none() {
                notes.push(format!("{} is a log script: give output_dir when you set the rule", loaded.name));
            }
            if host.is_internet() && !config.proxy_enabled {
                notes.push(self.proxy_off_note());
            }
            return Ok(SetScriptRuleResult {
                rule: Scripts::preview(rule, loaded),
                replaced: false,
                check_only: true,
                inspect: None,
                notes,
            });
        }
        if let Some(pid) = rule.owner_pid
            && !pidwatch::alive(pid)
        {
            return Err(err(ErrorCode::ProcessNotFound, format!("process {pid} is not running")));
        }
        let pattern = localrouter_core::scripts::rule_host(&rule);
        let was_inspected = pattern.as_ref().is_some_and(|p| self.shared.inspect.read().unwrap().patterns().contains(p));
        let old = self.scripts.put(rule.clone(), Ok(loaded));
        let replaced = old.is_some();
        let old_persistent = old.as_ref().is_some_and(|o| o.active.rule.persistent);
        if (rule.persistent || old_persistent)
            && let Err(e) = store::save_script_rules(&self.paths.script_rules(), &self.scripts.rules())
        {
            self.scripts.restore(&rule.id, old);
            return Err(err(ErrorCode::Io, format!("could not save script-rules.json: {e}")));
        }
        if let Some(pid) = rule.owner_pid
            && let Err(e) = self.pids.watch(pid)
        {
            self.scripts.restore(&rule.id, old);
            return Err(err(ErrorCode::ProcessNotFound, format!("cannot watch process {pid}: {e}")));
        }
        let mut inspect = None;
        if host.is_internet() {
            // A failure to make the CA is not a rollback: the rule stays, and
            // the reply says why its host cannot be inspected yet.
            let ca_created = match self.ensure_inspect_ca() {
                Ok(made) => made,
                Err(why) => {
                    notes.push(format!("{} cannot be inspected: {why}", rule.host));
                    false
                }
            };
            self.refresh_inspect();
            let ca_trusted = if self.inspect_certs.has_ca() {
                check_trust(self.paths.inspect_ca_pem(), &self.inspect_trust_cache).await
            } else {
                None
            };
            let host_added = !was_inspected && rule.enabled;
            if rule.enabled && self.inspect_certs.has_ca() && ca_trusted != Some(true) {
                notes.push(format!(
                    "{} is now inspected. Clients must trust the inspection CA: NODE_EXTRA_CA_CERTS={}, or ask the user to run: {} proxy trust",
                    rule.host,
                    self.paths.inspect_ca_pem().display(),
                    self.instance.cli()
                ));
            }
            if !config.proxy_enabled {
                notes.push(self.proxy_off_note());
            }
            inspect = Some(InspectChange { host_added, ca_created, ca_trusted });
        } else {
            // A rule moved from an internet host to a route takes the old
            // host out of the inspect set (I11).
            self.refresh_inspect();
        }
        tracing::info!("set script rule {} on {}{}", rule.id, rule.host, rule.path.as_deref().unwrap_or(""));
        let view = self.scripts.view(&rule.id).ok_or_else(|| err(ErrorCode::Io, "the rule vanished"))?;
        Ok(SetScriptRuleResult { rule: view, replaced, check_only: false, inspect, notes })
    }

    fn proxy_off_note(&self) -> String {
        format!(
            "The proxy is off. This rule sees only router traffic until: {} proxy on",
            self.instance.cli()
        )
    }

    pub async fn remove_script_rule(&self, p: IdParams) -> Result<RemoveScriptRuleResult, ApiError> {
        let _guard = self.write.lock().await;
        let id = p.id.trim().to_ascii_lowercase();
        let Some(old) = self.scripts.remove(&id) else {
            return Ok(RemoveScriptRuleResult { removed: false });
        };
        if old.active.rule.persistent
            && let Err(e) = store::save_script_rules(&self.paths.script_rules(), &self.scripts.rules())
        {
            self.scripts.restore(&id, Some(old));
            return Err(err(ErrorCode::Io, format!("could not save script-rules.json: {e}")));
        }
        self.refresh_inspect();
        tracing::info!("removed script rule {id}");
        Ok(RemoveScriptRuleResult { removed: true })
    }

    pub fn list_script_rules(&self) -> ListScriptRulesResult {
        ListScriptRulesResult { rules: self.scripts.views() }
    }

    /// A rule was turned off after 20 failures: save that when it is
    /// persistent, and take its host out of the inspect set.
    async fn rule_turned_off(&self, id: &str) {
        let _guard = self.write.lock().await;
        if self.scripts.get(id).is_some_and(|e| e.active.rule.persistent)
            && let Err(e) = store::save_script_rules(&self.paths.script_rules(), &self.scripts.rules())
        {
            tracing::warn!("could not save script-rules.json: {e}");
        }
        self.refresh_inspect();
    }

    /// Delete the inspection CA and make a new one. A user action, not an
    /// MCP tool (ADR 06, change 2).
    pub async fn reset_inspect_ca(&self) -> Result<ResetCaResult, ApiError> {
        let _guard = self.write.lock().await;
        let ca = LocalCa::reset_kind(&self.paths, &self.instance, CaKind::Inspection)
            .map_err(|e| err(ErrorCode::CaUnavailable, format!("could not make a new inspection CA: {e}")))?;
        let common_name = ca.common_name().to_string();
        self.inspect_certs.set_ca(Some(ca));
        self.problems.lock().unwrap().inspect_ca = None;
        self.forget_trust_cache();
        if let Err(e) = write_ca_bundle(&self.paths) {
            tracing::warn!("could not write the CA bundle: {e}");
        }
        tracing::warn!("inspection CA reset: new CA {common_name}");
        Ok(ResetCaResult { pem_path: self.paths.inspect_ca_pem().display().to_string(), common_name })
    }
}

/// How old `bundle.pem` may get: the system roots change with macOS updates.
const BUNDLE_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

impl Daemon {
    /// At start: write `bundle.pem` when the inspection CA exists and the
    /// bundle is missing or older than 30 days. Runs off the network tasks.
    pub fn refresh_ca_bundle_if_old(self: &Arc<Self>) {
        if !self.inspect_certs.has_ca() {
            return;
        }
        let old = std::fs::metadata(self.paths.inspect_ca_bundle())
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().unwrap_or_default() > BUNDLE_MAX_AGE)
            .unwrap_or(true);
        if old {
            let paths = self.paths.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = write_ca_bundle(&paths) {
                    tracing::warn!("could not write the CA bundle: {e}");
                }
            });
        }
    }
}

/// `inspect-ca/bundle.pem`: the macOS system root certificates followed by
/// the inspection CA. Public certificates only, never a key (I23).
fn write_ca_bundle(paths: &Paths) -> Result<(), String> {
    let out = std::process::Command::new("/usr/bin/security")
        .args(["find-certificate", "-a", "-p", "/System/Library/Keychains/SystemRootCertificates.keychain"])
        .output()
        .map_err(|e| format!("cannot run security: {e}"))?;
    if !out.status.success() {
        return Err(format!("security find-certificate failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let mut pem = out.stdout;
    // Debug builds: the end-to-end test adds its test internet CA as a root.
    #[cfg(debug_assertions)]
    if let Ok(extra) = std::env::var("LOCALROUTER_TEST_EXTRA_ROOTS") {
        pem.extend(std::fs::read(&extra).map_err(|e| format!("cannot read {extra}: {e}"))?);
    }
    if !pem.ends_with(b"\n") {
        pem.push(b'\n');
    }
    let ca = std::fs::read(paths.inspect_ca_pem()).map_err(|e| format!("cannot read the inspection CA: {e}"))?;
    if !ca.starts_with(b"-----BEGIN CERTIFICATE-----") {
        return Err("the inspection CA file is not one PEM certificate".into());
    }
    pem.extend(ca);
    store::replace(&paths.inspect_ca_bundle(), &pem).map_err(|e| format!("cannot write the bundle: {e}"))
}

/// The network a local address belongs to. Debug builds accept
/// `LOCALROUTER_TEST_NETWORK=mac:…,192.168.0.1,en0` (or `none`), so tests
/// need no real network.
fn network_of(local: std::net::IpAddr) -> Option<Network> {
    #[cfg(debug_assertions)]
    if let Some(test) = test_network() {
        return test;
    }
    network::of_local_ip(local)
}

/// The network of the default route.
fn current_network() -> Option<Network> {
    #[cfg(debug_assertions)]
    if let Some(test) = test_network() {
        return test;
    }
    network::current()
}

#[cfg(debug_assertions)]
fn test_network() -> Option<Option<Network>> {
    let value = std::env::var("LOCALROUTER_TEST_NETWORK").ok()?;
    let mut parts = value.split(',');
    let id = parts.next()?.to_string();
    if id == "none" {
        return Some(None);
    }
    let router = parts.next()?.parse().ok()?;
    let interface = parts.next().unwrap_or("en0").to_string();
    Some(Some(Network { id, router, interface }))
}

/// `lan_networks` from a client: ids are `mac:` plus six lower-case hex
/// pairs; names are trimmed; one entry per id.
fn normalize_networks(list: &[LanNetwork]) -> Result<Vec<LanNetwork>, String> {
    let mut out: Vec<LanNetwork> = vec![];
    for n in list {
        let id = n.id.trim().to_ascii_lowercase();
        let mac = id.strip_prefix("mac:").unwrap_or("");
        let ok = mac.split(':').count() == 6 && mac.split(':').all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()));
        if !ok {
            return Err(format!("lan_networks: {:?} is not a network id; use mac: and the router's MAC address", n.id));
        }
        if n.name.chars().count() > 100 {
            return Err("lan_networks: a name has at most 100 characters".into());
        }
        if !out.iter().any(|o| o.id == id) {
            out.push(LanNetwork { id, name: n.name.trim().to_string(), router: n.router.trim().to_string() });
        }
    }
    Ok(out)
}

/// Ask macOS whether a CA file is trusted for TLS. Cached for 10 seconds.
async fn check_trust(pem: std::path::PathBuf, cache: &TrustCache) -> Option<bool> {
    if let Some((at, value)) = *cache.lock().unwrap()
        && at.elapsed() < Duration::from_secs(10)
    {
        return value;
    }
    let out = tokio::process::Command::new("/usr/bin/security")
        .args(["verify-cert", "-L", "-q", "-p", "ssl", "-c"])
        .arg(&pem)
        .output()
        .await;
    let value = out.ok().map(|o| o.status.success());
    *cache.lock().unwrap() = Some((Instant::now(), value));
    value
}

/// The proxy clients of a new config (ADR 09): valid names, one entry per
/// name, and no port shared with another client or another listener of the
/// daemon (`0`, any free port, is for tests).
fn check_clients(config: &Config) -> Result<(), String> {
    let others = [("http_port", config.http_port), ("https_port", config.https_port), ("proxy_port", config.proxy_port)];
    for (i, c) in config.proxy_clients.iter().enumerate() {
        ProxyClient::check_name(&c.name)?;
        let earlier = &config.proxy_clients[..i];
        if earlier.iter().any(|e| e.name == c.name) {
            return Err(format!("proxy client {} is in the list twice", c.name));
        }
        if c.port == 0 {
            continue;
        }
        if let Some(e) = earlier.iter().find(|e| e.port == c.port) {
            return Err(format!("proxy clients {} and {} both have port {}", e.name, c.name, c.port));
        }
        if let Some((field, _)) = others.iter().find(|(_, p)| *p == c.port) {
            return Err(format!("proxy client {} has port {}, which is the {field}", c.name, c.port));
        }
    }
    Ok(())
}

/// The listeners of one proxy port and the port they got.
type Bound = (Vec<std::net::TcpListener>, u16);

/// A client's port: the wildcard pair for a phone client (ADR 10), else
/// loopback (I1).
fn bind_client(c: &ProxyClient) -> Result<Bound, ApiError> {
    if c.lan { bind_lan(c.port) } else { bind_proxy(c.port) }
}

/// A phone client's port on `0.0.0.0` and `[::]`, with the check that
/// loopback traffic really reaches it (`bind_all`, as ports 80 and 443).
fn bind_lan(port: u16) -> Result<Bound, ApiError> {
    let (listeners, status) = listen::bind_all(port);
    match status.port {
        Some(actual) if !listeners.is_empty() => Ok((listeners, actual)),
        _ => Err(err(ErrorCode::PortInUse, format!("cannot listen on 0.0.0.0:{port} and [::]:{port}: {}", status.errors.join("; ")))),
    }
}

/// The phone clients a config needs: the kept record of each, a new token
/// and no device for a new one, nothing for any other name (ADR 10, I12).
fn phones_for(config: &Config, old: &BTreeMap<String, store::PhoneRecord>) -> BTreeMap<String, store::PhoneRecord> {
    config
        .proxy_clients
        .iter()
        .filter(|c| c.lan)
        .map(|c| {
            let record = match old.get(&c.name) {
                Some(r) if !r.token.is_empty() => r.clone(),
                _ => store::PhoneRecord { token: phone::new_token(), devices: vec![] },
            };
            (c.name.clone(), record)
        })
        .collect()
}

/// At start: every phone client of the config gets a token before its port
/// binds, and a removed client's record goes (I12).
fn reconcile_phones(paths: &Paths, config: &Config) -> BTreeMap<String, store::PhoneRecord> {
    let old = store::load_phones(&paths.proxy_phones());
    let new = phones_for(config, &old);
    if new != old
        && let Err(e) = store::save_phones(&paths.proxy_phones(), &new)
    {
        tracing::warn!("could not write the phone clients: {e}; they are kept in memory");
    }
    new
}

/// This Mac's IPv4 address on a network's interface: the server in a
/// phone's setup URL. Debug builds take a 4th field of
/// `LOCALROUTER_TEST_NETWORK`.
fn lan_address(n: &Network) -> Option<std::net::Ipv4Addr> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("LOCALROUTER_TEST_NETWORK")
        && let Some(address) = value.split(',').nth(3)
    {
        return address.parse().ok();
    }
    phone::ipv4_of(&n.interface)
}


/// Bind the proxy port on 127.0.0.1 and ::1, both or neither (I1).
fn bind_proxy(port: u16) -> Result<Bound, ApiError> {
    tcp_listen::bind_loopback(port).map_err(|e| {
        let why = if e.kind() == std::io::ErrorKind::AddrInUse {
            format!("port {port} is in use by another program")
        } else {
            e.to_string()
        };
        err(ErrorCode::PortInUse, format!("cannot listen on 127.0.0.1:{port} and [::1]:{port}: {why}"))
    })
}

/// The upstream client: the macOS trust store and resolver. Debug builds
/// accept two test settings for the end-to-end test, as they accept
/// `LOCALROUTER_TEST_API_VERSION`; release builds ignore them.
fn upstream() -> Upstream {
    use localrouter_core::upstream::{SystemResolver, platform_tls};
    #[cfg(debug_assertions)]
    if let Some(test) = test_upstream::from_env() {
        return test;
    }
    let tls = platform_tls().unwrap_or_else(|e| {
        // Never less safe: without the platform verifier, trust nothing.
        tracing::error!("cannot use the macOS trust store: {e}; the proxy refuses every TLS server");
        let config = rustls::ClientConfig::builder_with_provider(localrouter_core::tls::provider())
            .with_safe_default_protocol_versions()
            .expect("ring supports the default versions")
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        Arc::new(config)
    });
    Upstream::new(tls, Arc::new(SystemResolver))
}

/// `LOCALROUTER_TEST_RESOLVE=test.example=127.0.0.1:4443` sends a name to a
/// local server; `LOCALROUTER_TEST_UPSTREAM_CA=<pem>` trusts only that CA.
#[cfg(debug_assertions)]
mod test_upstream {
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::Arc;

    use localrouter_core::upstream::{BoxFuture, Resolve, SystemResolver, Upstream, platform_tls};

    struct MapResolver(HashMap<String, SocketAddr>);

    impl Resolve for MapResolver {
        fn resolve(&self, host: &str, port: u16) -> BoxFuture<std::io::Result<Vec<SocketAddr>>> {
            match self.0.get(host) {
                Some(addr) => {
                    let addr = *addr;
                    Box::pin(async move { Ok(vec![addr]) })
                }
                None => SystemResolver.resolve(host, port),
            }
        }
    }

    pub fn from_env() -> Option<Upstream> {
        let names = std::env::var("LOCALROUTER_TEST_RESOLVE").ok();
        let ca = std::env::var("LOCALROUTER_TEST_UPSTREAM_CA").ok();
        if names.is_none() && ca.is_none() {
            return None;
        }
        let map = names
            .unwrap_or_default()
            .split(',')
            .filter_map(|pair| {
                let (name, addr) = pair.split_once('=')?;
                Some((name.to_string(), addr.parse().ok()?))
            })
            .collect();
        let tls = match ca {
            Some(path) => {
                use rustls::pki_types::pem::PemObject;
                let mut roots = rustls::RootCertStore::empty();
                for cert in rustls::pki_types::CertificateDer::pem_file_iter(&path).ok()?.flatten() {
                    roots.add(cert).ok()?;
                }
                let config = rustls::ClientConfig::builder_with_provider(localrouter_core::tls::provider())
                    .with_safe_default_protocol_versions()
                    .ok()?
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                Arc::new(config)
            }
            None => platform_tls().ok()?,
        };
        tracing::warn!("test upstream settings in use");
        Some(Upstream::new(tls, Arc::new(MapResolver(map))))
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

/// Persistent rules from `script-rules.json`. A rule whose script does not
/// load stays in the list with its error and matches nothing.
fn load_saved_script_rules(paths: &Paths, scripts: &Scripts) {
    let loaded = store::load_script_rules(&paths.script_rules());
    if let Some(p) = &loaded.problem {
        tracing::warn!("{p}");
    }
    for mut rule in loaded.value {
        rule.persistent = true;
        rule.owner_pid = None;
        let others = scripts.len();
        if let Err(e) = script_rules::validate(&mut rule, &paths.data, others) {
            tracing::warn!("skipped saved script rule {}: {e}", rule.id);
            continue;
        }
        let script = load_file(std::path::Path::new(&rule.script));
        if let Err(e) = &script {
            tracing::warn!("script rule {} does not load: {e}", rule.id);
        }
        scripts.put(rule, script);
    }
}
