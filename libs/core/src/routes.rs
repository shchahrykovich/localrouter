//! Routes: validation, the route table and name lookup.
//!
//! See ADR 01, changes 6 (route model) and 7 (protocols), and ADR 03 (path
//! routes: the route key is host plus path).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::TLD;

/// Target prefix of a folder route: `file://` plus an absolute path, written
/// as is (no percent-encoding): `file:///Users/me/site`.
pub const FOLDER_SCHEME: &str = "file://";
pub const MAX_NOTE_CHARS: usize = 500;
pub const MAX_LABEL_LEN: usize = 63;
/// A full DNS name is at most 253 characters; the key leaves room for `.localhost`.
pub const MAX_HOST_LEN: usize = 253 - TLD.len() - 1;
pub const MAX_PATH_LEN: usize = 200;
/// Host key of the help page the daemon serves itself (`router.localhost`).
/// No route can take it.
pub const HELP_HOST: &str = "router";
/// Host key of the proxy log viewer (ADR 08). New routes cannot take it; a
/// route saved before it was reserved still loads and wins (I11).
pub const PROXY_LOG_HOST: &str = "proxy";
/// The viewer's second address, always served: `router.localhost/proxy-log/`.
pub const PROXY_LOG_PATH: &str = "/proxy-log";

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
    /// HTTP routes only: the path prefix this route answers, for example
    /// `/blog` (it answers `/blog` and `/blog/...`). `None` is the default
    /// route of the host: every path no other route of the host matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub protocol: Protocol,
    /// `http://`, `https://` or `tcp://` plus a loopback host and a port, or
    /// `file://` plus the absolute path of a folder (a folder route).
    pub target: String,
    /// TCP routes only: the loopback port clients connect to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_port: Option<u16>,
    /// HTTP routes only: plain HTTP is redirected to HTTPS.
    #[serde(default, skip_serializing_if = "is_false")]
    pub https_only: bool,
    /// Path routes only: remove the path before the request reaches the target.
    #[serde(default, skip_serializing_if = "is_false")]
    pub strip_path: bool,
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

    /// The folder a folder route serves. `None` for a route to a server.
    pub fn folder(&self) -> Option<&Path> {
        self.target.strip_prefix(FOLDER_SCHEME).map(Path::new)
    }

    /// The full name clients use, for example `feat-login.shop.localhost`.
    pub fn full_name(&self) -> String {
        format!("{}.{TLD}", self.host)
    }

    /// The name plus the path, for example `shop.localhost/blog`.
    pub fn full_name_and_path(&self) -> String {
        format!("{}{}", self.full_name(), self.path.as_deref().unwrap_or(""))
    }

    pub fn key(&self) -> RouteKey {
        RouteKey { host: self.host.clone(), path: self.path.clone().unwrap_or_default() }
    }
}

/// Identity of a route: host plus path. `path` is empty for the default route.
/// Written as the two joined: `shop`, `shop/blog`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteKey {
    pub host: String,
    pub path: String,
}

impl RouteKey {
    pub fn new(host: &str, path: Option<&str>) -> Self {
        Self { host: host.to_string(), path: path.unwrap_or("").to_string() }
    }
}

impl fmt::Display for RouteKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.host, self.path)
    }
}

/// Whether a route with `route_path` answers `request_path`: equal, or
/// followed by `/`. `/blog` matches `/blog/x`, never `/blogger`. A route
/// without a path matches every path.
pub fn path_matches(route_path: Option<&str>, request_path: &str) -> bool {
    match route_path {
        None => true,
        Some(p) => request_path.strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with('/')),
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
    #[error("\"{given}\" is not a valid host: a label cannot contain '/'. Try \"{suggestion}\", or host \"{host}\" with path \"{path}\"")]
    HostLooksLikePath { given: String, suggestion: String, host: String, path: String },
    #[error("path \"{path}\" is not valid: {reason}")]
    BadPath { path: String, reason: &'static str },
    #[error("path and strip_path are only for http routes")]
    PathOnlyForHttp,
    #[error("strip_path needs a path")]
    StripPathNeedsPath,
    #[error("host \"{0}\" has a tcp route; a path route needs a host without one")]
    HostHasTcpRoute(String),
    #[error("host \"{0}\" has path routes; remove them before adding a tcp route")]
    HostHasPathRoutes(String),
    #[error("host must not end in .{TLD}: write \"{0}\"")]
    HasTld(String),
    #[error("\"{0}\" is reserved: {0}.{TLD} is LocalRouter's help page. Use another name")]
    ReservedHost(String),
    #[error("\"{0}\" is reserved: {0}.{TLD} is LocalRouter's proxy log viewer. Use another name")]
    ReservedForProxyLog(String),
    #[error("target \"{0}\" is not valid: use http://, https:// or tcp:// plus 127.0.0.1, localhost or [::1] and a port, or file:// plus the absolute path of a folder")]
    BadTarget(String),
    #[error("folder \"{folder}\" is not valid: {reason}")]
    BadFolder { folder: String, reason: &'static str },
    #[error("a folder target (file://) is only for http routes")]
    FolderOnlyForHttp,
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
            // `shop/blog` may be a host plus a path (ADR 03). The slash is in
            // the last label only then; `feat/login.shop` is a branch name.
            if let Some((h, rest)) = host.split_once('/')
                && host.rsplit('.').next().is_some_and(|last| last.contains('/'))
                && h.split('.').all(valid_label)
                && let Ok(Some(path)) = normalize_path(&format!("/{rest}"))
            {
                return Err(RouteError::HostLooksLikePath { given: host.clone(), suggestion, host: h.to_string(), path });
            }
            return Err(RouteError::BadLabel { label: label.to_string(), suggestion });
        }
    }
    Ok(host)
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// Check a route path and remove a trailing `/`. `/` and the empty string
/// mean "no path" (the default route), so they return `None`.
pub fn normalize_path(path: &str) -> Result<Option<String>, RouteError> {
    let bad = |reason| RouteError::BadPath { path: path.to_string(), reason };
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Ok(None);
    }
    let Some(rest) = trimmed.strip_prefix('/') else {
        return Err(bad("it must start with /"));
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.len() + 1 > MAX_PATH_LEN {
        return Err(bad("it is longer than 200 characters"));
    }
    if !rest.split('/').all(valid_segment) {
        return Err(bad(
            "use segments of A-Z, a-z, 0-9, '-', '.', '_' and '~' separated by one '/', and no '.' or '..' segment",
        ));
    }
    Ok(Some(format!("/{rest}")))
}

/// Check the folder of a folder target and write it in one form:
/// `file:///Users/me//site/` becomes `file:///Users/me/site`. Does not look
/// at the disk; the daemon checks that the folder exists.
pub fn normalize_folder_target(target: &str) -> Result<String, RouteError> {
    let folder = target.strip_prefix(FOLDER_SCHEME).unwrap_or(target);
    let bad = |reason| RouteError::BadFolder { folder: folder.to_string(), reason };
    let path = Path::new(folder);
    if !path.is_absolute() {
        return Err(bad("use an absolute path, for example file:///Users/me/site"));
    }
    if folder.contains(['\0', '\n', '\r']) {
        return Err(bad("it contains a control character"));
    }
    let mut clean = PathBuf::new();
    for c in path.components() {
        match c {
            Component::RootDir | Component::Normal(_) => clean.push(c),
            // `..` would make the path the route shows differ from the one
            // it serves. (`components` already drops a `.` inside the path.)
            _ => return Err(bad("use a path without '..' parts")),
        }
    }
    if clean.parent().is_none() {
        return Err(bad("the whole disk cannot be served; name a folder"));
    }
    Ok(format!("{FOLDER_SCHEME}{}", clean.display()))
}

/// One step of a lookup, for `explain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The name does not end in `.localhost`.
    NotLocalhost,
    /// The HTTP routes of this host key (by path, `None` = default route) and
    /// the one whose path matched, if any.
    Key { host: String, paths: Vec<Option<String>>, matched: Option<Option<String>> },
    /// Nothing matched; fallback is on, so the parent host key is tried next.
    Fallback { parent: String },
    /// Nothing matched and fallback is off.
    FallbackOff,
}

/// The route a name and path lead to, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub route: Option<Route>,
    pub steps: Vec<Step>,
}

/// Facts about the running daemon that validation needs.
#[derive(Debug, Clone, Copy)]
pub struct Reserved {
    pub http_port: u16,
    pub https_port: u16,
}

#[derive(Debug, Clone, Default)]
pub struct RouteTable {
    /// Sorted by host, then path, so the routes of one host are one range.
    routes: BTreeMap<RouteKey, Route>,
}

impl RouteTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &RouteKey) -> Option<&Route> {
        self.routes.get(key)
    }

    /// All routes of one host key, default route first, then by path.
    pub fn routes_of<'a>(&'a self, host: &str) -> impl Iterator<Item = &'a Route> + use<'a> {
        let host = host.to_string();
        self.routes
            .range(RouteKey { host: host.clone(), path: String::new() }..)
            .take_while(move |(k, _)| k.host == host)
            .map(|(_, r)| r)
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

    /// Insert or replace. Returns the old route with the same key.
    pub fn insert(&mut self, route: Route) -> Option<Route> {
        self.routes.insert(route.key(), route)
    }

    pub fn remove(&mut self, key: &RouteKey) -> Option<Route> {
        self.routes.remove(key)
    }

    /// Keys of routes owned by `pid`.
    pub fn owned_by(&self, pid: u32) -> Vec<RouteKey> {
        self.routes.iter().filter(|(_, r)| r.owner_pid == Some(pid)).map(|(k, _)| k.clone()).collect()
    }

    /// The TCP route listening on `port`.
    pub fn by_listen_port(&self, port: u16) -> Option<&Route> {
        self.routes
            .values()
            .find(|r| r.protocol == Protocol::Tcp && r.listen_port == Some(port))
    }

    /// Persistent routes, sorted by host and path (what `routes.json` holds).
    pub fn persistent(&self) -> Vec<Route> {
        self.routes.values().filter(|r| r.persistent).cloned().collect()
    }

    /// Check and normalize a route before it is inserted. A route with the same
    /// host is ignored in the uniqueness checks, because it will be replaced.
    pub fn validate(&self, route: &mut Route, reserved: Reserved) -> Result<(), RouteError> {
        if normalize_host(&route.host)? == PROXY_LOG_HOST {
            return Err(RouteError::ReservedForProxyLog(PROXY_LOG_HOST.into()));
        }
        self.validate_saved(route, reserved)
    }

    /// [`RouteTable::validate`] for a route loaded from `routes.json`: a
    /// saved route `proxy` from before ADR 08 is kept, so an update never
    /// makes a working route vanish (I11).
    pub fn validate_saved(&self, route: &mut Route, reserved: Reserved) -> Result<(), RouteError> {
        route.host = normalize_host(&route.host)?;
        if route.host == HELP_HOST {
            return Err(RouteError::ReservedHost(route.host.clone()));
        }
        route.path = match route.path.take() {
            Some(p) => normalize_path(&p)?,
            None => None,
        };
        // `None`: a folder route, which has no address.
        let target = if route.folder().is_some() {
            route.target = normalize_folder_target(&route.target)?;
            None
        } else {
            Some(route.target_addr()?)
        };
        if route.note.chars().count() > MAX_NOTE_CHARS {
            return Err(RouteError::NoteTooLong);
        }
        if route.persistent && route.owner_pid.is_some() {
            return Err(RouteError::PersistentWithOwner);
        }
        match route.protocol {
            Protocol::Http => {
                if target.as_ref().is_some_and(|t| t.scheme == Scheme::Tcp) {
                    return Err(RouteError::HttpNeedsHttpTarget);
                }
                if route.listen_port.is_some() {
                    return Err(RouteError::ListenPortOnlyForTcp);
                }
                if route.strip_path && route.path.is_none() {
                    return Err(RouteError::StripPathNeedsPath);
                }
                // A TCP route keeps its host to itself (I22). Its key is the
                // default key, so a default HTTP route simply replaces it.
                if route.path.is_some() && self.routes_of(&route.host).any(|r| r.protocol == Protocol::Tcp) {
                    return Err(RouteError::HostHasTcpRoute(route.host.clone()));
                }
            }
            Protocol::Tcp => {
                let Some(target) = target else {
                    return Err(RouteError::FolderOnlyForHttp);
                };
                if target.scheme != Scheme::Tcp {
                    return Err(RouteError::TcpNeedsTcpTarget);
                }
                if route.https_only {
                    return Err(RouteError::HttpsOnlyOnlyForHttp);
                }
                if route.path.is_some() || route.strip_path {
                    return Err(RouteError::PathOnlyForHttp);
                }
                if self.routes_of(&route.host).any(|r| r.path.is_some()) {
                    return Err(RouteError::HostHasPathRoutes(route.host.clone()));
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

    /// Find the HTTP route for a `Host` header and a request path (ADR 03).
    ///
    /// The nearest host key with a matching route wins, then the longest path
    /// on it. `feat-x.shop` falls back to `shop` when `fallback` is on and no
    /// route of `feat-x.shop` matches the path.
    pub fn lookup(&self, name: &str, path: &str, fallback: bool) -> Option<&Route> {
        self.walk(name, path, fallback, None)
    }

    /// `lookup`, with each step recorded. The two share one walk, so they
    /// always choose the same route (I33).
    pub fn explain(&self, name: &str, path: &str, fallback: bool) -> Explanation {
        let mut steps = Vec::new();
        let route = self.walk(name, path, fallback, Some(&mut steps)).cloned();
        Explanation { route, steps }
    }

    /// Whether some HTTP route serves this name, whatever the path. Decides
    /// whether the name gets a TLS certificate (I27).
    pub fn serves(&self, name: &str, fallback: bool) -> bool {
        let Some(full) = host_key(name) else { return false };
        let mut key = full.as_str();
        loop {
            if self.routes_of(key).any(|r| r.protocol == Protocol::Http) {
                return true;
            }
            if !fallback {
                return false;
            }
            match key.split_once('.') {
                Some((_, parent)) => key = parent,
                None => return false,
            }
        }
    }

    fn walk(&self, name: &str, path: &str, fallback: bool, mut steps: Option<&mut Vec<Step>>) -> Option<&Route> {
        let Some(full) = host_key(name) else {
            if let Some(steps) = steps.as_mut() {
                steps.push(Step::NotLocalhost);
            }
            return None;
        };
        let mut key = full.as_str();
        loop {
            let mut best: Option<&Route> = None;
            for route in self.routes_of(key).filter(|r| r.protocol == Protocol::Http) {
                if path_matches(route.path.as_deref(), path)
                    && best.is_none_or(|b| route.path.as_ref().map_or(0, String::len) > b.path.as_ref().map_or(0, String::len))
                {
                    best = Some(route);
                }
            }
            if let Some(steps) = steps.as_mut() {
                steps.push(Step::Key {
                    host: key.to_string(),
                    paths: self.routes_of(key).filter(|r| r.protocol == Protocol::Http).map(|r| r.path.clone()).collect(),
                    matched: best.map(|r| r.path.clone()),
                });
            }
            if best.is_some() {
                return best;
            }
            if !fallback {
                if let Some(steps) = steps.as_mut() {
                    steps.push(Step::FallbackOff);
                }
                return None;
            }
            key = key.split_once('.')?.1;
            if let Some(steps) = steps.as_mut() {
                steps.push(Step::Fallback { parent: key.to_string() });
            }
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
            path: None,
            protocol: Protocol::Http,
            target: format!("http://127.0.0.1:{port}"),
            listen_port: None,
            https_only: false,
            strip_path: false,
            note: String::new(),
            owner_pid: None,
            persistent: false,
        }
    }

    fn at(host: &str, path: &str, port: u16) -> Route {
        Route { path: Some(path.into()), ..http(host, port) }
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
    fn router_host_is_reserved_for_the_help_page() {
        for host in ["router", "Router"] {
            let mut r = http(host, 5173);
            assert_eq!(RouteTable::new().validate(&mut r, RES).unwrap_err(), RouteError::ReservedHost("router".into()));
        }
        let mut sub = http("feat.router", 5173);
        RouteTable::new().validate(&mut sub, RES).unwrap();
    }

    // ADR 08, T7, I11: `proxy` is refused for a new route, kept for a saved one.
    #[test]
    fn proxy_host_is_reserved_for_new_routes_only() {
        for host in ["proxy", "Proxy"] {
            let mut r = http(host, 5173);
            assert_eq!(
                RouteTable::new().validate(&mut r, RES).unwrap_err(),
                RouteError::ReservedForProxyLog("proxy".into())
            );
        }
        let mut saved = http("proxy", 5173);
        RouteTable::new().validate_saved(&mut saved, RES).unwrap();
        let mut saved = http("router", 5173);
        assert!(RouteTable::new().validate_saved(&mut saved, RES).is_err(), "router stays refused");
        let mut sub = http("feat.proxy", 5173);
        RouteTable::new().validate(&mut sub, RES).unwrap();
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
        let port = |name: &str, fb| t.lookup(name, "/", fb).map(|r| r.target_addr().unwrap().port);
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
        assert!(t.lookup("db.shop.localhost", "/", true).is_none());
        assert!(!t.serves("db.shop.localhost", true));
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

    // ADR 03, T1: path rules and the route key (I20, I21)

    #[test]
    fn valid_paths_are_normalized() {
        for (given, want) in [
            ("/blog", Some("/blog")),
            ("/blog/", Some("/blog")),
            ("/docs/v2", Some("/docs/v2")),
            ("/api_v2", Some("/api_v2")),
            ("/~me", Some("/~me")),
            ("/Blog", Some("/Blog")),
            ("/", None),
            ("", None),
        ] {
            assert_eq!(normalize_path(given).unwrap().as_deref(), want, "{given:?}");
        }
    }

    #[test]
    fn bad_paths_are_refused_with_the_rule() {
        let long = format!("/{}", "a".repeat(MAX_PATH_LEN));
        for (given, reason) in [
            ("blog", "start with /"),
            ("/a//b", "segments"),
            ("/a/../b", "segments"),
            ("/a/./b", "segments"),
            ("/a%20b", "segments"),
            ("/a?b", "segments"),
            ("/a#b", "segments"),
            ("/a*", "segments"),
            (long.as_str(), "200 characters"),
        ] {
            let err = normalize_path(given).unwrap_err();
            assert!(matches!(err, RouteError::BadPath { .. }), "{given:?}: {err}");
            assert!(err.to_string().contains(reason), "{given:?}: {err}");
        }
    }

    #[test]
    fn slash_path_and_no_path_are_one_key() {
        let mut slash = at("shop", "/", 1);
        RouteTable::new().validate(&mut slash, RES).unwrap();
        assert_eq!(slash.path, None);
        let mut t = table(&[http("shop", 5173)]);
        assert!(t.insert(slash).is_some(), "the default route was not replaced");
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn path_routes_have_their_own_keys() {
        let mut t = table(&[http("shop", 1), at("shop", "/blog", 2), at("shop", "/Blog", 3)]);
        assert_eq!(t.len(), 3);
        assert_eq!(t.get(&RouteKey::new("shop", Some("/blog"))).unwrap().target, "http://127.0.0.1:2");
        assert_eq!(RouteKey::new("shop", Some("/blog")).to_string(), "shop/blog");
        assert_eq!(RouteKey::new("shop", None).to_string(), "shop");
        let old = t.insert(at("shop", "/blog", 4)).unwrap();
        assert_eq!(old.target, "http://127.0.0.1:2");
        assert_eq!(t.remove(&RouteKey::new("shop", None)).unwrap().target, "http://127.0.0.1:1");
        let left: Vec<_> = t.routes_of("shop").map(|r| r.key().to_string()).collect();
        assert_eq!(left, ["shop/Blog", "shop/blog"]);
    }

    #[test]
    fn host_with_a_path_gets_a_path_hint() {
        let err = normalize_host("shop/blog").unwrap_err();
        assert_eq!(
            err,
            RouteError::HostLooksLikePath {
                given: "shop/blog".into(),
                suggestion: "shop-blog".into(),
                host: "shop".into(),
                path: "/blog".into()
            }
        );
        let text = err.to_string();
        assert!(text.contains("\"shop-blog\"") && text.contains("path \"/blog\""), "{text}");
        // One label, so it may be a branch name too: both hints.
        assert!(matches!(normalize_host("feat/login"), Err(RouteError::HostLooksLikePath { .. })));
        // The slash is not in the last label: a branch name, the old hint.
        assert!(matches!(normalize_host("feat/login.shop"), Err(RouteError::BadLabel { .. })));
    }

    // ADR 03, T2: protocol rules (I22)

    #[test]
    fn path_is_only_for_http_routes() {
        let mut t = tcp("db", 15432, 5432);
        t.path = Some("/x".into());
        assert_eq!(RouteTable::new().validate(&mut t, RES), Err(RouteError::PathOnlyForHttp));
        let mut t = tcp("db", 15432, 5432);
        t.strip_path = true;
        assert_eq!(RouteTable::new().validate(&mut t, RES), Err(RouteError::PathOnlyForHttp));
        let mut r = http("shop", 1);
        r.strip_path = true;
        assert_eq!(RouteTable::new().validate(&mut r, RES), Err(RouteError::StripPathNeedsPath));
    }

    #[test]
    fn tcp_routes_do_not_share_a_host_with_path_routes() {
        let with_tcp = table(&[tcp("db", 15432, 5432)]);
        let mut p = at("db", "/x", 1);
        assert_eq!(with_tcp.validate(&mut p, RES), Err(RouteError::HostHasTcpRoute("db".into())));

        let with_path = table(&[at("shop", "/blog", 1)]);
        let mut t = tcp("shop", 15432, 5432);
        assert_eq!(with_path.validate(&mut t, RES), Err(RouteError::HostHasPathRoutes("shop".into())));

        // ADR 01 behaviour stays: a TCP route may replace a default HTTP route.
        let with_default = table(&[http("shop", 1)]);
        let mut t = tcp("shop", 15432, 5432);
        with_default.validate(&mut t, RES).unwrap();
    }

    // ADR 03, T3: the match rule, the lookup order and explain (I23, I24, I33)

    #[test]
    fn match_rule_table() {
        for (req, blog, default) in [
            ("/blog", true, true),
            ("/blog/post-1", true, true),
            ("/blogger", false, true),
            ("/Blog", false, true),
            ("/", false, true),
            ("/blog%2Fx", false, true),
        ] {
            assert_eq!(path_matches(Some("/blog"), req), blog, "/blog vs {req}");
            assert_eq!(path_matches(None, req), default, "default vs {req}");
        }
    }

    fn answer(t: &RouteTable, name: &str, path: &str, fallback: bool) -> Option<String> {
        let got = t.lookup(name, path, fallback).map(|r| r.key().to_string());
        let explained = t.explain(name, path, fallback).route.map(|r| r.key().to_string());
        assert_eq!(got, explained, "explain and lookup disagree on {name}{path}");
        got
    }

    #[test]
    fn longest_path_wins_on_one_host() {
        let t = table(&[http("shop", 1), at("shop", "/blog", 2), at("shop", "/blog/admin", 3)]);
        assert_eq!(answer(&t, "shop.localhost", "/blog/admin/x", true).as_deref(), Some("shop/blog/admin"));
        assert_eq!(answer(&t, "shop.localhost", "/blog/x", true).as_deref(), Some("shop/blog"));
        assert_eq!(answer(&t, "shop.localhost", "/blogger", true).as_deref(), Some("shop"));
        assert_eq!(answer(&t, "shop.localhost", "/", true).as_deref(), Some("shop"));
    }

    #[test]
    fn nearest_host_key_wins_over_a_longer_path() {
        let t = table(&[http("shop", 1), at("shop", "/blog", 2), http("feat-x.shop", 3)]);
        assert_eq!(answer(&t, "feat-x.shop.localhost", "/blog", true).as_deref(), Some("feat-x.shop"));
    }

    #[test]
    fn branch_path_route_falls_back_for_other_paths() {
        // Scenario A and B of ADR 03, change 2.
        let t = table(&[http("shop", 5173), at("shop", "/blog", 3001), at("feat-x.shop", "/blog", 3002)]);
        assert_eq!(answer(&t, "feat-x.shop.localhost", "/blog/post-1", true).as_deref(), Some("feat-x.shop/blog"));
        assert_eq!(answer(&t, "feat-x.shop.localhost", "/products", true).as_deref(), Some("shop"));
        assert_eq!(answer(&t, "feat-x.shop.localhost", "/products", false), None);
        assert_eq!(answer(&t, "feat-x.shop.localhost", "/blog", false).as_deref(), Some("feat-x.shop/blog"));
    }

    #[test]
    fn host_with_only_path_routes() {
        let t = table(&[at("shop", "/blog", 1)]);
        assert_eq!(answer(&t, "shop.localhost", "/", true), None);
        assert!(t.serves("shop.localhost", true), "a path route must still get a certificate");
        assert!(t.serves("x.shop.localhost", true));
        assert!(!t.serves("x.shop.localhost", false));
        assert!(!t.serves("blog.localhost", true));
    }

    #[test]
    fn without_path_routes_the_lookup_is_the_old_one() {
        // I24 regression: every case of lookup_exact_fallback_and_none, with
        // several request paths, gives the route the old lookup gave.
        let t = table(&[http("shop", 5173), http("feat-login.shop", 5174)]);
        let cases = [
            ("shop.localhost", true, Some(5173)),
            ("feat-login.shop.localhost", true, Some(5174)),
            ("feat-other.shop.localhost", true, Some(5173)),
            ("feat-other.shop.localhost", false, None),
            ("blog.localhost", true, None),
            ("shop.localhost:8443", true, Some(5173)),
            ("Shop.LocalHost", true, Some(5173)),
            ("shop.example.com", true, None),
        ];
        for (name, fb, want) in cases {
            for path in ["/", "/blog", "/a/b?c", "*"] {
                let port = t.lookup(name, path, fb).map(|r| r.target_addr().unwrap().port);
                assert_eq!(port, want, "{name} {path} fallback={fb}");
                assert_eq!(t.serves(name, fb), want.is_some(), "serves {name}");
            }
        }
    }

    #[test]
    fn explain_records_each_step() {
        let t = table(&[http("shop", 5173), at("feat-x.shop", "/blog", 3002)]);
        let e = t.explain("feat-x.shop.localhost", "/products", true);
        assert_eq!(e.route.unwrap().key().to_string(), "shop");
        assert_eq!(
            e.steps,
            [
                Step::Key { host: "feat-x.shop".into(), paths: vec![Some("/blog".into())], matched: None },
                Step::Fallback { parent: "shop".into() },
                Step::Key { host: "shop".into(), paths: vec![None], matched: Some(None) },
            ]
        );
        let off = t.explain("feat-x.shop.localhost", "/products", false);
        assert_eq!(off.steps.last(), Some(&Step::FallbackOff));
        assert_eq!(t.explain("shop.example.com", "/", true).steps, [Step::NotLocalhost]);
    }

    // Folder routes: target file:// plus an absolute folder.

    fn folder(host: &str, target: &str) -> Route {
        Route { target: target.into(), ..http(host, 1) }
    }

    #[test]
    fn folder_targets_are_normalized() {
        for (given, want) in [
            ("file:///Users/me/site", "file:///Users/me/site"),
            ("file:///Users/me/site/", "file:///Users/me/site"),
            ("file:///Users/me//my site", "file:///Users/me/my site"),
            ("file:///Users/./me", "file:///Users/me"),
        ] {
            let mut r = folder("docs", given);
            RouteTable::new().validate(&mut r, RES).unwrap_or_else(|e| panic!("{given}: {e}"));
            assert_eq!(r.target, want);
            assert_eq!(r.folder(), Some(Path::new(want.strip_prefix("file://").unwrap())));
        }
        assert_eq!(http("shop", 5173).folder(), None);
    }

    #[test]
    fn bad_folder_targets_are_refused_with_the_rule() {
        for (given, reason) in [
            ("file://site", "absolute path"),
            ("file://./site", "absolute path"),
            ("file:///Users/me/../other", "'..'"),
            ("file:///", "whole disk"),
            ("file://////", "whole disk"),
            ("file:///a\nb", "control character"),
        ] {
            let err = RouteTable::new().validate(&mut folder("docs", given), RES).unwrap_err();
            assert!(matches!(err, RouteError::BadFolder { .. }), "{given:?}: {err}");
            assert!(err.to_string().contains(reason), "{given:?}: {err}");
        }
    }

    #[test]
    fn folder_routes_are_http_routes_and_may_have_a_path() {
        let mut t = folder("db", "file:///srv/db");
        t.protocol = Protocol::Tcp;
        t.listen_port = Some(15432);
        assert_eq!(RouteTable::new().validate(&mut t, RES), Err(RouteError::FolderOnlyForHttp));

        let mut docs = Route { path: Some("/docs".into()), ..folder("shop", "file:///srv/docs") };
        RouteTable::new().validate(&mut docs, RES).unwrap();
        let t = table(&[http("shop", 5173), docs]);
        assert_eq!(answer(&t, "shop.localhost", "/docs/a.html", true).as_deref(), Some("shop/docs"));
        assert!(t.serves("shop.localhost", true));
    }

    #[test]
    fn bad_target_message_names_the_folder_form() {
        let mut r = http("shop", 1);
        r.target = "ftp://x".into();
        let err = RouteTable::new().validate(&mut r, RES).unwrap_err();
        assert!(err.to_string().contains("file://"), "{err}");
    }
}
