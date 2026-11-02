//! From one nettop sample to bytes per app (ADR 11, change 1):
//!
//! 1. The daemon's own pid is dropped (I2): every route and proxy byte would
//!    otherwise count twice, once for the app and once for the daemon.
//! 2. Each connection gets its `via` (I3): `router` for a loopback connection
//!    to the HTTP or HTTPS port or a TCP route's listen port, `proxy` for one
//!    to a proxy port, `direct` for everything else.
//! 3. A `proxy` connection is joined to the forward proxy's peer table on its
//!    local port, which gives the host and the upstream address.
//! 4. Processes are grouped by app: the outermost `.app`, the executable, or
//!    "macOS services".

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::Via;
use super::nettop::{Proto, Sample};

/// The key of the row that holds the programs of macOS itself.
pub const SERVICES_KEY: &str = "macOS services";

/// The ports that make a loopback connection `router` or `proxy`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ports {
    /// The bound HTTP and HTTPS ports.
    pub http: HashSet<u16>,
    /// TCP routes: listen port → the route's name, `db.shop.localhost`.
    pub tcp_routes: HashMap<u16, String>,
    /// The main proxy port and every proxy client's port (ADR 09).
    pub proxy: HashSet<u16>,
}

/// What the router or the forward proxy knows about one client connection,
/// found by the client's port (ADR 11, 1.4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerInfo {
    /// The proxy client's name; `None` for the main port or the router.
    pub client: Option<String>,
    /// The last host asked for on this connection.
    pub host: Option<String>,
    /// The server the forward proxy connected to for it.
    pub upstream: Option<SocketAddr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppKind {
    /// An `.app` bundle.
    App,
    /// An executable outside a bundle: `/opt/homebrew/bin/node`.
    Program,
    /// The programs of macOS itself, in one row.
    Services,
    /// The process ended before its path was read (gap G4).
    Exited,
}

/// The row a process belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppId {
    /// The app path, the executable path, [`SERVICES_KEY`], or
    /// `exited:<nettop name>`.
    pub key: String,
    pub name: String,
    pub kind: AppKind,
}

/// The row of an executable path, or of a process whose path is unknown.
pub fn app_of(exe: Option<&Path>, nettop_name: &str) -> AppId {
    let Some(exe) = exe else {
        return AppId { key: format!("exited:{nettop_name}"), name: nettop_name.to_string(), kind: AppKind::Exited };
    };
    let text = exe.to_string_lossy();
    // The apps that come with macOS are apps; the rest of the system is one row.
    let system_app = ["/System/Applications/", "/System/Cryptexes/App/System/Applications/"].iter().any(|p| text.starts_with(p));
    if !system_app && ["/System/", "/usr/libexec/", "/usr/sbin/", "/sbin/"].iter().any(|p| text.starts_with(p)) {
        return AppId { key: SERVICES_KEY.to_string(), name: SERVICES_KEY.to_string(), kind: AppKind::Services };
    }
    if let Some(app) = outermost_app(&text) {
        let name = app.rsplit('/').next().unwrap_or(app).trim_end_matches(".app").to_string();
        return AppId { key: app.to_string(), name, kind: AppKind::App };
    }
    let name = exe.file_name().map_or_else(|| text.to_string(), |n| n.to_string_lossy().into_owned());
    AppId { key: text.into_owned(), name, kind: AppKind::Program }
}

/// `/Applications/Google Chrome.app` for any path inside it, also from a
/// helper app inside the bundle.
fn outermost_app(path: &str) -> Option<&str> {
    let mut at = 0;
    while let Some(i) = path[at..].find(".app") {
        let end = at + i + 4;
        if end == path.len() || path.as_bytes()[end] == b'/' {
            return Some(&path[..end]);
        }
        at = end;
    }
    None
}

/// The bytes of one app through one `via` to one remote, in one sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBytes {
    pub app: AppId,
    /// The executable, when its path is known.
    pub exe: Option<String>,
    pub pid: u32,
    pub via: Via,
    /// `None` for an unconnected UDP socket.
    pub remote: Option<IpAddr>,
    pub port: Option<u16>,
    pub proto: Proto,
    /// The route's name for `router`; the proxy's host for `proxy`.
    pub host: Option<String>,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// app, executable, pid, via, remote, port, protocol, host.
type RowKey = (String, Option<String>, u32, Via, Option<IpAddr>, Option<u16>, Proto, Option<String>);

/// Apply the rules of the module text to one sample. `path_of` reads a
/// process's executable path (`proc_pidpath`); `peer` looks up the router's
/// and the forward proxy's table by the client's port.
pub fn apply(
    sample: &Sample,
    ports: &Ports,
    daemon_pid: u32,
    peer: &dyn Fn(u16) -> Option<PeerInfo>,
    path_of: &dyn Fn(u32) -> Option<std::path::PathBuf>,
) -> Vec<AppBytes> {
    let mut out: Vec<AppBytes> = vec![];
    let mut index: HashMap<RowKey, usize> = HashMap::new();
    for proc in &sample.procs {
        // I2: the daemon's own sockets are never an app.
        if proc.pid == daemon_pid {
            continue;
        }
        let used: Vec<_> = proc.conns.iter().filter(|c| c.bytes_in > 0 || c.bytes_out > 0).collect();
        if used.is_empty() {
            continue;
        }
        let exe = path_of(proc.pid);
        let app = app_of(exe.as_deref(), &proc.name);
        let exe = exe.map(|p| p.to_string_lossy().into_owned());
        for conn in used {
            let (via, remote, port, host) = classify(conn.remote, conn.local.port, ports, peer);
            let key = (app.key.clone(), exe.clone(), proc.pid, via, remote, port, conn.proto, host.clone());
            match index.get(&key) {
                Some(&i) => {
                    out[i].bytes_in += conn.bytes_in;
                    out[i].bytes_out += conn.bytes_out;
                }
                None => {
                    index.insert(key, out.len());
                    out.push(AppBytes {
                        app: app.clone(),
                        exe: exe.clone(),
                        pid: proc.pid,
                        via,
                        remote,
                        port,
                        proto: conn.proto,
                        host,
                        bytes_in: conn.bytes_in,
                        bytes_out: conn.bytes_out,
                    });
                }
            }
        }
    }
    out
}

/// I3: the `via`, and the remote and host the bytes are counted under.
fn classify(
    remote: Option<SocketAddr>,
    local_port: Option<u16>,
    ports: &Ports,
    peer: &dyn Fn(u16) -> Option<PeerInfo>,
) -> (Via, Option<IpAddr>, Option<u16>, Option<String>) {
    let Some(remote) = remote else { return (Via::Direct, None, None, None) };
    let ip = remote.ip().to_canonical();
    if !ip.is_loopback() {
        return (Via::Direct, Some(ip), Some(remote.port()), None);
    }
    let port = remote.port();
    let from_peer = || local_port.and_then(peer).unwrap_or_default();
    if ports.http.contains(&port) {
        return (Via::Router, Some(ip), Some(port), from_peer().host);
    }
    if let Some(name) = ports.tcp_routes.get(&port) {
        return (Via::Router, Some(ip), Some(port), Some(name.clone()));
    }
    if ports.proxy.contains(&port) {
        let info = from_peer();
        // The bytes go to where the proxy sent them; with no match they
        // stay with the proxy port and have no place.
        return match info.upstream {
            Some(up) => (Via::Proxy, Some(up.ip().to_canonical()), Some(up.port()), info.host),
            None => (Via::Proxy, None, None, info.host),
        };
    }
    // A dev server reached by its port: direct, and never on the map.
    (Via::Direct, Some(ip), Some(port), None)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::nettop::{Splitter, parse_sample};
    use super::*;

    const FIXTURE: &str = include_str!("testdata/nettop-two-samples.csv");

    fn sample(n: usize) -> Sample {
        let mut s = Splitter::default();
        let mut parts: Vec<String> = FIXTURE.lines().filter_map(|l| s.push(l)).collect();
        parts.extend(s.finish());
        parse_sample(&parts[n]).unwrap()
    }

    fn ports() -> Ports {
        Ports {
            http: [80, 443].into(),
            tcp_routes: [(15432, "db.shop.localhost".to_string())].into(),
            proxy: [8877, 8878].into(),
        }
    }

    fn paths(pid: u32) -> Option<PathBuf> {
        match pid {
            5120 => Some("/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper".into()),
            401 => Some("/System/Library/PrivateFrameworks/ApplePushService.framework/apsd".into()),
            404 => Some("/usr/libexec/usbmuxd".into()),
            521 => Some("/usr/sbin/mDNSResponder".into()),
            7001 => Some("/opt/homebrew/bin/node".into()),
            913 => Some("/Applications/LocalRouter-dev.app/Contents/MacOS/localrouterd-dev".into()),
            _ => None,
        }
    }

    fn no_peer(_: u16) -> Option<PeerInfo> {
        None
    }

    fn one(sample: &Sample, local_port: u16, remote: &str, p: &Ports) -> (Via, Option<IpAddr>, Option<u16>, Option<String>) {
        let _ = sample;
        classify(Some(remote.parse().unwrap()), Some(local_port), p, &no_peer)
    }

    #[test]
    fn loopback_to_the_http_https_and_tcp_route_ports_is_router() {
        let s = sample(0);
        let p = ports();
        assert_eq!(one(&s, 1, "[::1]:443", &p).0, Via::Router);
        assert_eq!(one(&s, 1, "127.0.0.1:80", &p).0, Via::Router);
        let (via, _, _, host) = one(&s, 1, "127.0.0.1:15432", &p);
        assert_eq!((via, host.as_deref()), (Via::Router, Some("db.shop.localhost")));
    }

    #[test]
    fn loopback_to_a_proxy_port_is_proxy() {
        let s = sample(0);
        let p = ports();
        assert_eq!(one(&s, 1, "127.0.0.1:8877", &p).0, Via::Proxy);
        assert_eq!(one(&s, 1, "[::1]:8878", &p).0, Via::Proxy);
        // An IPv4-mapped loopback address is loopback too.
        assert_eq!(one(&s, 1, "[::ffff:127.0.0.1]:8877", &p).0, Via::Proxy);
    }

    #[test]
    fn other_loopback_ports_and_real_remotes_are_direct() {
        let s = sample(0);
        let p = ports();
        let (via, ip, port, _) = one(&s, 1, "127.0.0.1:5173", &p);
        assert_eq!((via, ip, port), (Via::Direct, Some("127.0.0.1".parse().unwrap()), Some(5173)));
        assert_eq!(one(&s, 1, "192.0.2.7:22", &p).0, Via::Direct);
        assert_eq!(one(&s, 1, "203.0.113.4:443", &p).0, Via::Direct);
        assert_eq!(classify(None, Some(5353), &p, &no_peer), (Via::Direct, None, None, None));
    }

    #[test]
    fn the_daemon_is_dropped_and_the_rest_adds_up() {
        let s = sample(0);
        let rows = apply(&s, &ports(), 913, &no_peer, &paths);
        assert!(rows.iter().all(|r| r.pid != 913));
        assert!(!rows.iter().any(|r| r.app.name.contains("localrouterd")));
        let total: u64 = s.procs.iter().flat_map(|p| &p.conns).map(|c| c.bytes_in).sum();
        let daemon: u64 = s.procs.iter().filter(|p| p.pid == 913).flat_map(|p| &p.conns).map(|c| c.bytes_in).sum();
        let apps: u64 = rows.iter().map(|r| r.bytes_in).sum();
        assert_eq!(apps, total - daemon);
        // Chrome: router 3.4 KB in, proxy 96 KB in (the example in 01, 1.3).
        let chrome: Vec<_> = rows.iter().filter(|r| r.pid == 5120).collect();
        let by = |v: Via| chrome.iter().filter(|r| r.via == v).map(|r| r.bytes_in).sum::<u64>();
        assert_eq!((by(Via::Router), by(Via::Proxy)), (3400, 96000));
    }

    #[test]
    fn a_proxy_connection_is_joined_on_its_local_port() {
        let s = sample(1);
        let peer = |port: u16| {
            (port == 61002).then(|| PeerInfo {
                client: None,
                host: Some("github.com".into()),
                upstream: Some("203.0.113.4:443".parse().unwrap()),
            })
        };
        let rows = apply(&s, &ports(), 913, &peer, &paths);
        let proxy = rows.iter().find(|r| r.via == Via::Proxy).unwrap();
        assert_eq!(proxy.host.as_deref(), Some("github.com"));
        assert_eq!(proxy.remote, Some("203.0.113.4".parse().unwrap()));
        assert_eq!(proxy.port, Some(443));
        assert_eq!((proxy.bytes_in, proxy.bytes_out), (1500, 200));
        // Without a match: the host is unknown and there is no place.
        let rows = apply(&s, &ports(), 913, &no_peer, &paths);
        let proxy = rows.iter().find(|r| r.via == Via::Proxy).unwrap();
        assert_eq!((proxy.host.as_deref(), proxy.remote), (None, None));
        // A router connection gets the route host the router saw.
        let peer = |port: u16| (port == 61001).then(|| PeerInfo { host: Some("shop.localhost".into()), ..Default::default() });
        let rows = apply(&s, &ports(), 913, &peer, &paths);
        let router = rows.iter().find(|r| r.via == Via::Router).unwrap();
        assert_eq!(router.host.as_deref(), Some("shop.localhost"));
    }

    #[test]
    fn app_paths() {
        let rows = apply(&sample(0), &ports(), 913, &no_peer, &paths);
        let chrome = rows.iter().find(|r| r.pid == 5120).unwrap();
        assert_eq!(chrome.app, AppId { key: "/Applications/Google Chrome.app".into(), name: "Google Chrome".into(), kind: AppKind::App });
        assert!(chrome.exe.as_deref().unwrap().ends_with("Google Chrome Helper"));
        let usb = rows.iter().find(|r| r.pid == 404).unwrap();
        assert_eq!((usb.app.key.as_str(), usb.app.kind), (SERVICES_KEY, AppKind::Services));
        let apsd = rows.iter().find(|r| r.pid == 401).unwrap();
        assert_eq!(apsd.app.kind, AppKind::Services);
        let node = apply(&sample(1), &ports(), 913, &no_peer, &paths).into_iter().find(|r| r.pid == 7001).unwrap();
        assert_eq!((node.app.key.as_str(), node.app.name.as_str(), node.app.kind), ("/opt/homebrew/bin/node", "node", AppKind::Program));
        // No path: the nettop name, marked exited.
        let gone = app_of(None, "curl");
        assert_eq!((gone.key.as_str(), gone.name.as_str(), gone.kind), ("exited:curl", "curl", AppKind::Exited));
        // An app that ships with macOS stays an app; a .application folder is not an app.
        assert_eq!(app_of(Some(Path::new("/System/Applications/Mail.app/Contents/MacOS/Mail")), "Mail").kind, AppKind::App);
        assert_eq!(app_of(Some(Path::new("/opt/x.application/bin/y")), "y").kind, AppKind::Program);
        assert_eq!(app_of(Some(Path::new("/usr/bin/curl")), "curl").kind, AppKind::Program);
    }

    #[test]
    fn connections_without_bytes_are_left_out() {
        let s = sample(0);
        let rows = apply(&s, &ports(), 913, &no_peer, &paths);
        assert!(rows.iter().all(|r| r.bytes_in + r.bytes_out > 0));
        assert!(!rows.iter().any(|r| r.remote == Some("2001:db8:1::443".parse().unwrap())));
    }
}
