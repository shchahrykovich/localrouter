//! Routes: validation, the route table and name lookup.
//!
//! See ADR 01, changes 6 (route model) and 7 (protocols).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::TLD;

pub const MAX_NOTE_CHARS: usize = 500;
pub const MAX_LABEL_LEN: usize = 63;
/// A full DNS name is at most 253 characters; the key leaves room for `.localhost`.
pub const MAX_HOST_LEN: usize = 253 - TLD.len() - 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// Served on the shared HTTP and HTTPS ports, chosen by name.
    #[default]
    Http,
    /// Served on its own loopback port, chosen by that port.
    Tcp,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Protocol::Http => "http",
            Protocol::Tcp => "tcp",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    /// Host key: the name without `.localhost`, for example `feat-login.shop`.
    pub host: String,
    #[serde(default)]
    pub protocol: Protocol,
    /// `http://`, `https://` or `tcp://` plus a loopback host and a port.
    pub target: String,
    /// TCP routes only: the loopback port clients connect to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_port: Option<u16>,
    /// HTTP routes only: plain HTTP is redirected to HTTPS.
    #[serde(default, skip_serializing_if = "is_false")]
    pub https_only: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The route is removed when this process exits. Never saved to disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
    /// Saved in `routes.json`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub persistent: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Route {
    pub fn target_addr(&self) -> Result<TargetAddr, RouteError> {
        TargetAddr::parse(&self.target)
    }

    /// The full name clients use, for example `feat-login.shop.localhost`.
    pub fn full_name(&self) -> String {
        format!("{}.{TLD}", self.host)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
    Tcp,
}

/// A parsed route target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetAddr {
    pub scheme: Scheme,
    /// As written: `127.0.0.1`, `localhost` or `::1`.
    pub host: String,
    pub port: u16,
}

impl TargetAddr {
    pub fn parse(target: &str) -> Result<Self, RouteError> {
        let bad = || RouteError::BadTarget(target.to_string());
        let (scheme, rest) = target.split_once("://").ok_or_else(bad)?;
        let scheme = match scheme {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            "tcp" => Scheme::Tcp,
            _ => return Err(bad()),
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
            let (host, after) = v6.split_once(']').ok_or_else(bad)?;
            (host, after.strip_prefix(':').ok_or_else(bad)?)
        } else {
            rest.rsplit_once(':').ok_or_else(bad)?
        };
        let port: u16 = port.parse().map_err(|_| bad())?;
        if port == 0 {
            return Err(bad());
        }
        let host = host.to_ascii_lowercase();
        if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
            return Err(RouteError::NotLoopback(target.to_string()));
        }
        Ok(Self { scheme, host, port })
    }

    /// Address to connect to. `localhost` is sent to `127.0.0.1`.
    pub fn socket_addr(&self) -> std::net::SocketAddr {
        let ip: std::net::IpAddr = if self.host == "::1" {
            std::net::Ipv6Addr::LOCALHOST.into()
        } else {
            std::net::Ipv4Addr::LOCALHOST.into()
        };
        (ip, self.port).into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RouteError {
    #[error("host is empty")]
    EmptyHost,
    #[error("host is longer than {MAX_HOST_LEN} characters")]
    HostTooLong,
    #[error("\"{label}\" is not a valid label: use a-z, 0-9 and '-', 1 to 63 characters, no '-' at the start or end. Try \"{suggestion}\"")]
    BadLabel { label: String, suggestion: String },
    #[error("host must not end in .{TLD}: write \"{0}\"")]
    HasTld(String),
    #[error("target \"{0}\" is not valid: use http://, https:// or tcp:// plus 127.0.0.1, localhost or [::1] and a port")]
    BadTarget(String),
    #[error("target \"{0}\" is not a loopback address: only 127.0.0.1, localhost and [::1] are allowed")]
    NotLoopback(String),
    #[error("an http route needs an http:// or https:// target")]
    HttpNeedsHttpTarget,
    #[error("a tcp route needs a tcp:// target")]
    TcpNeedsTcpTarget,
    #[error("a tcp route needs listen_port")]
    TcpNeedsListenPort,
    #[error("listen_port is only for tcp routes")]
    ListenPortOnlyForTcp,
    #[error("https_only is only for http routes")]
    HttpsOnlyOnlyForHttp,
    #[error("listen_port {0} is below 1024; use 1024 to 65535, or 0 to pick a free port")]
    ListenPortPrivileged(u16),
    #[error("listen_port {0} is a LocalRouter HTTP port")]
    ListenPortIsHttpPort(u16),
    #[error("listen_port {port} is already used by route \"{host}\"")]
    ListenPortTaken { port: u16, host: String },
    #[error("listen_port {0} equals the target port; the route would connect to itself")]
    ListenPortIsTarget(u16),
    #[error("note is longer than {MAX_NOTE_CHARS} characters")]
    NoteTooLong,
    #[error("a route cannot be both persistent and owned by a process (owner_pid)")]
    PersistentWithOwner,
}

/// Turn any text into a valid label: `feat/login` → `feat-login`.
pub fn slugify(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    let out: String = out.chars().take(MAX_LABEL_LEN).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "x".into() } else { out }
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_LABEL_LEN
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Lower-case and check a host key.
pub fn normalize_host(host: &str) -> Result<String, RouteError> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return Err(RouteError::EmptyHost);
    }
    if let Some(stripped) = host.strip_suffix(&format!(".{TLD}")) {
        return Err(RouteError::HasTld(stripped.to_string()));
    }
    if host.len() > MAX_HOST_LEN {
        return Err(RouteError::HostTooLong);
    }
    // A slash splits a branch name like feat/login into two "labels"; suggest
    // one label for the part before the first dot instead.
    for label in host.split('.') {
        if !valid_label(label) {
            let suggestion = host.split('.').map(slugify).collect::<Vec<_>>().join(".");
            return Err(RouteError::BadLabel { label: label.to_string(), suggestion });
        }
    }
    Ok(host)
}

/// Facts about the running daemon that validation needs.
#[derive(Debug, Clone, Copy)]
pub struct Reserved {
    pub http_port: u16,
    pub https_port: u16,
}

#[derive(Debug, Clone, Default)]
pub struct RouteTable {
    routes: BTreeMap<String, Route>,
}

impl RouteTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, host: &str) -> Option<&Route> {
        self.routes.get(host)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Route> {
        self.routes.values()
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Insert or replace. Returns the old route with the same host.
    pub fn insert(&mut self, route: Route) -> Option<Route> {
        self.routes.insert(route.host.clone(), route)
    }

    pub fn remove(&mut self, host: &str) -> Option<Route> {
        self.routes.remove(host)
    }

    /// Hosts of routes owned by `pid`.
    pub fn owned_by(&self, pid: u32) -> Vec<String> {
        self.routes.values().filter(|r| r.owner_pid == Some(pid)).map(|r| r.host.clone()).collect()
    }

    /// The TCP route listening on `port`.
    pub fn by_listen_port(&self, port: u16) -> Option<&Route> {
        self.routes
            .values()
            .find(|r| r.protocol == Protocol::Tcp && r.listen_port == Some(port))
    }

    /// Persistent routes, sorted by host (what `routes.json` holds).
    pub fn persistent(&self) -> Vec<Route> {
        self.routes.values().filter(|r| r.persistent).cloned().collect()
    }

    /// Check and normalize a route before it is inserted. A route with the same
    /// host is ignored in the uniqueness checks, because it will be replaced.
    pub fn validate(&self, route: &mut Route, reserved: Reserved) -> Result<(), RouteError> {
        route.host = normalize_host(&route.host)?;
        let target = route.target_addr()?;
        if route.note.chars().count() > MAX_NOTE_CHARS {
            return Err(RouteError::NoteTooLong);
        }
        if route.persistent && route.owner_pid.is_some() {
            return Err(RouteError::PersistentWithOwner);
        }
        match route.protocol {
            Protocol::Http => {
                if target.scheme == Scheme::Tcp {
                    return Err(RouteError::HttpNeedsHttpTarget);
                }
                if route.listen_port.is_some() {
                    return Err(RouteError::ListenPortOnlyForTcp);
                }
            }
            Protocol::Tcp => {
                if target.scheme != Scheme::Tcp {
                    return Err(RouteError::TcpNeedsTcpTarget);
                }
                if route.https_only {
                    return Err(RouteError::HttpsOnlyOnlyForHttp);
                }
                let port = route.listen_port.ok_or(RouteError::TcpNeedsListenPort)?;
                if port != 0 {
                    if port < 1024 {
                        return Err(RouteError::ListenPortPrivileged(port));
                    }
                    if port == reserved.http_port || port == reserved.https_port {
                        return Err(RouteError::ListenPortIsHttpPort(port));
                    }
                    if port == target.port {
                        return Err(RouteError::ListenPortIsTarget(port));
                    }
                    if let Some(other) = self.by_listen_port(port).filter(|o| o.host != route.host) {
                        return Err(RouteError::ListenPortTaken { port, host: other.host.clone() });
                    }
                }
            }
        }
        Ok(())
    }

    /// Find the HTTP route for a `Host` header or TLS SNI name.
    ///
    /// Longest match: `feat-x.shop` falls back to `shop` when `fallback` is on.
    pub fn lookup(&self, name: &str, fallback: bool) -> Option<&Route> {
        let full = host_key(name)?;
        let mut key = full.as_str();
        loop {
            if let Some(route) = self.routes.get(key).filter(|r| r.protocol == Protocol::Http) {
                return Some(route);
            }
            if !fallback {
                return None;
            }
            key = key.split_once('.')?.1;
        }
    }
}

/// `Feat.Shop.localhost:443` → `feat.shop`. `None` if the name is not under `.localhost`.
pub fn host_key(name: &str) -> Option<String> {
    const SUFFIX: &str = ".localhost";
    let name = name.trim_end_matches('.');
    let name = match name.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => name,
    };
    let name = name.trim_end_matches('.');
    let at = name.len().checked_sub(SUFFIX.len()).filter(|&at| at > 0 && name.is_char_boundary(at))?;
    let (key, tail) = name.split_at(at);
    tail.eq_ignore_ascii_case(SUFFIX).then(|| key.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: Reserved = Reserved { http_port: 80, https_port: 443 };

    fn http(host: &str, port: u16) -> Route {
        Route {
            host: host.into(),
            protocol: Protocol::Http,
            target: format!("http://127.0.0.1:{port}"),
            listen_port: None,
            https_only: false,
            note: String::new(),
            owner_pid: None,
            persistent: false,
        }
    }

    fn tcp(host: &str, listen: u16, target: u16) -> Route {
        Route {
            protocol: Protocol::Tcp,
            target: format!("tcp://127.0.0.1:{target}"),
            listen_port: Some(listen),
            ..http(host, 1)
        }
    }

    fn table(routes: &[Route]) -> RouteTable {
        let mut t = RouteTable::new();
        for r in routes {
            t.insert(r.clone());
        }
        t
    }

    // T1: labels

    #[test]
    fn valid_hosts_are_lower_cased() {
        let mut r = http("Feat-Login.Shop", 5173);
        RouteTable::new().validate(&mut r, RES).unwrap();
        assert_eq!(r.host, "feat-login.shop");
    }

    #[test]
    fn branch_name_with_slash_is_refused_with_a_slug_hint() {
        let mut r = http("feat/login.shop", 5173);
        let err = RouteTable::new().validate(&mut r, RES).unwrap_err();
        assert_eq!(
            err,
            RouteError::BadLabel { label: "feat/login".into(), suggestion: "feat-login.shop".into() }
        );
        assert!(err.to_string().contains("feat-login.shop"));
    }

    #[test]
    fn bad_labels_are_refused() {
        for host in ["-shop", "shop-", "sh_op", "", "a..b", &"x".repeat(64)] {
            let mut r = http(host, 5173);
            assert!(RouteTable::new().validate(&mut r, RES).is_err(), "{host:?} was accepted");
        }
    }

    #[test]
    fn host_with_tld_is_refused() {
        let mut r = http("shop.localhost", 5173);
        assert_eq!(
            RouteTable::new().validate(&mut r, RES).unwrap_err(),
            RouteError::HasTld("shop".into())
        );
    }

    #[test]
    fn slugify_examples() {
        assert_eq!(slugify("feat/login"), "feat-login");
        assert_eq!(slugify("Fix: The Bug!"), "fix-the-bug");
        assert_eq!(slugify("///"), "x");
    }

    // T1: targets (I9) and protocol fields (I19)

    #[test]
    fn only_loopback_targets_are_accepted() {
        let mut lan = http("shop", 1);
        lan.target = "http://192.168.1.5:3000".into();
        assert!(matches!(RouteTable::new().validate(&mut lan, RES), Err(RouteError::NotLoopback(_))));

        for ok in ["http://[::1]:3000", "http://localhost:3000", "https://127.0.0.1:8443", "http://127.0.0.1:3000/"] {
            let mut r = http("shop", 1);
            r.target = ok.into();
            RouteTable::new().validate(&mut r, RES).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
    }

    #[test]
    fn target_needs_a_port() {
        let mut r = http("shop", 1);
        r.target = "http://127.0.0.1".into();
        assert!(matches!(RouteTable::new().validate(&mut r, RES), Err(RouteError::BadTarget(_))));
    }

    #[test]
    fn schemes_must_match_the_protocol() {
        let mut r = http("shop", 1);
        r.target = "tcp://127.0.0.1:5432".into();
        assert_eq!(RouteTable::new().validate(&mut r, RES), Err(RouteError::HttpNeedsHttpTarget));

        let mut t = tcp("db", 15432, 5432);
        t.target = "http://127.0.0.1:5432".into();
        assert_eq!(RouteTable::new().validate(&mut t, RES), Err(RouteError::TcpNeedsTcpTarget));

        let mut ok = tcp("db", 15432, 5432);
        RouteTable::new().validate(&mut ok, RES).unwrap();
    }

    #[test]
    fn protocol_only_fields_are_checked() {
        let mut r = http("shop", 5173);
        r.listen_port = Some(15000);
        assert_eq!(RouteTable::new().validate(&mut r, RES), Err(RouteError::ListenPortOnlyForTcp));

        let mut t = tcp("db", 15432, 5432);
        t.https_only = true;
        assert_eq!(RouteTable::new().validate(&mut t, RES), Err(RouteError::HttpsOnlyOnlyForHttp));

        let mut t = tcp("db", 15432, 5432);
        t.listen_port = None;
        assert_eq!(RouteTable::new().validate(&mut t, RES), Err(RouteError::TcpNeedsListenPort));
    }

    // T1: listen_port rules (I17)

    #[test]
    fn listen_port_rules() {
        let existing = table(&[tcp("db.shop", 15432, 55001)]);
        let cases = [
            (tcp("cache", 80, 6379), RouteError::ListenPortPrivileged(80)),
            (tcp("cache", 1023, 6379), RouteError::ListenPortPrivileged(1023)),
            (tcp("cache", 5432, 5432), RouteError::ListenPortIsTarget(5432)),
            (tcp("cache", 15432, 6379), RouteError::ListenPortTaken { port: 15432, host: "db.shop".into() }),
        ];
        for (mut route, want) in cases {
            assert_eq!(existing.validate(&mut route, RES), Err(want));
        }
        let custom = Reserved { http_port: 8080, https_port: 8443 };
        let mut r = tcp("cache", 8443, 6379);
        assert_eq!(RouteTable::new().validate(&mut r, custom), Err(RouteError::ListenPortIsHttpPort(8443)));
    }

    #[test]
    fn same_host_may_keep_its_listen_port() {
        let existing = table(&[tcp("db.shop", 15432, 55001)]);
        let mut again = tcp("db.shop", 15432, 55002);
        existing.validate(&mut again, RES).unwrap();
    }

    #[test]
    fn listen_port_zero_means_pick_one() {
        let mut r = tcp("db", 0, 5432);
        RouteTable::new().validate(&mut r, RES).unwrap();
    }

    #[test]
    fn persistent_and_owner_pid_are_refused() {
        let mut r = http("shop", 5173);
        r.persistent = true;
        r.owner_pid = Some(42);
        assert_eq!(RouteTable::new().validate(&mut r, RES), Err(RouteError::PersistentWithOwner));
    }

    #[test]
    fn long_note_is_refused() {
        let mut r = http("shop", 5173);
        r.note = "é".repeat(MAX_NOTE_CHARS + 1);
        assert_eq!(RouteTable::new().validate(&mut r, RES), Err(RouteError::NoteTooLong));
    }

    // T1: lookup

    #[test]
    fn lookup_exact_fallback_and_none() {
        let t = table(&[http("shop", 5173), http("feat-login.shop", 5174)]);
        let port = |name: &str, fb| t.lookup(name, fb).map(|r| r.target_addr().unwrap().port);
        assert_eq!(port("shop.localhost", true), Some(5173));
        assert_eq!(port("feat-login.shop.localhost", true), Some(5174));
        assert_eq!(port("feat-other.shop.localhost", true), Some(5173));
        assert_eq!(port("feat-other.shop.localhost", false), None);
        assert_eq!(port("blog.localhost", true), None);
        assert_eq!(port("shop.localhost:8443", true), Some(5173));
        assert_eq!(port("shop.localhost.", true), Some(5173));
        assert_eq!(port("localhost", true), None);
        assert_eq!(port("shop.example.com", true), None);
        assert_eq!(port("Shop.LocalHost", true), Some(5173));
    }

    #[test]
    fn lookup_ignores_tcp_routes() {
        let t = table(&[tcp("db.shop", 15432, 5432)]);
        assert!(t.lookup("db.shop.localhost", true).is_none());
        assert_eq!(t.by_listen_port(15432).unwrap().host, "db.shop");
    }

    #[test]
    fn persistent_list_is_sorted_and_skips_others() {
        let mut a = http("b", 1);
        a.persistent = true;
        let mut b = http("a", 2);
        b.persistent = true;
        let t = table(&[a, b, http("c", 3)]);
        let hosts: Vec<_> = t.persistent().into_iter().map(|r| r.host).collect();
        assert_eq!(hosts, ["a", "b"]);
    }
}
