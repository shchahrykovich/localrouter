//! Request log: a fixed-size ring buffer in memory, plus a live stream.
//!
//! Entries never hold query strings, headers or bodies (invariant I10).
//! Traffic of the forward proxy uses the same `http` entry with optional
//! fields, so older clients still decode `get_logs` (ADR 06, I16).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LogEntry {
    /// One HTTP request, written after the response headers are sent.
    Http {
        time_ms: u64,
        method: String,
        /// Full name the client asked for, without the port.
        host: String,
        /// Path without the query string.
        path: String,
        status: u16,
        duration_ms: u64,
        /// Key of the route that answered, for example `shop/blog`. Absent for
        /// the 404 page and the help page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route: Option<String>,
        /// `proxy` for traffic of the forward proxy; absent for the router.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<Via>,
        /// How the forward proxy carried the request.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<ProxyMode>,
        /// Tunnels only: bytes from the client to the server.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes_in: Option<u64>,
        /// Tunnels only: bytes from the server to the client.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes_out: Option<u64>,
    },
    /// One TCP connection, written when it closes.
    Tcp {
        time_ms: u64,
        /// Host key of the route.
        host: String,
        listen_port: u16,
        bytes_in: u64,
        bytes_out: u64,
        duration_ms: u64,
        /// The target did not answer.
        failed: bool,
    },
}

/// Which way in a request came (ADR 06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Proxy,
}

/// How the forward proxy carried a request (ADR 06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// An absolute-form `http://` request.
    Http,
    /// One HTTP request read inside an inspected `CONNECT`.
    Inspect,
    /// A `CONNECT` whose bytes were copied, not read.
    Tunnel,
}

impl LogEntry {
    /// An HTTP entry. The query string is removed here, so no caller can store it.
    pub fn http(method: &str, host: &str, path_and_query: &str, status: u16, duration_ms: u64) -> Self {
        let path = path_and_query.split(['?', '#']).next().unwrap_or("/");
        let host = host.rsplit_once(':').map_or(host, |(h, p)| if p.bytes().all(|b| b.is_ascii_digit()) { h } else { host });
        LogEntry::Http {
            time_ms: now_ms(),
            method: method.to_string(),
            host: host.to_ascii_lowercase(),
            path: if path.is_empty() { "/".into() } else { path.to_string() },
            status,
            duration_ms,
            route: None,
            via: None,
            mode: None,
            bytes_in: None,
            bytes_out: None,
        }
    }

    /// A closed tunnel of the forward proxy: method `CONNECT`, empty path,
    /// and the bytes each way. It takes no path, so it cannot store a query.
    pub fn tunnel(host: &str, status: u16, duration_ms: u64, bytes_in: u64, bytes_out: u64) -> Self {
        let mut entry = Self::http("CONNECT", host, "", status, duration_ms).via_proxy(ProxyMode::Tunnel);
        if let LogEntry::Http { path, bytes_in: i, bytes_out: o, .. } = &mut entry {
            path.clear();
            *i = Some(bytes_in);
            *o = Some(bytes_out);
        }
        entry
    }

    /// Mark an HTTP entry as forward proxy traffic.
    pub fn via_proxy(mut self, how: ProxyMode) -> Self {
        if let LogEntry::Http { via, mode, .. } = &mut self {
            *via = Some(Via::Proxy);
            *mode = Some(how);
        }
        self
    }

    /// Set the key of the route that answered (HTTP entries only).
    pub fn with_route(mut self, key: Option<String>) -> Self {
        if let LogEntry::Http { route, .. } = &mut self {
            *route = key;
        }
        self
    }

    /// The route host key (TCP) or the full name (HTTP).
    pub fn host(&self) -> &str {
        match self {
            LogEntry::Http { host, .. } | LogEntry::Tcp { host, .. } => host,
        }
    }

    /// True when the entry belongs to `filter`, given as a host key or a full name.
    pub fn matches_host(&self, filter: &str) -> bool {
        let filter = filter.trim_end_matches(".localhost");
        let host = self.host().trim_end_matches(".localhost");
        host == filter || host.ends_with(&format!(".{filter}"))
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub struct RequestLog {
    entries: Mutex<VecDeque<LogEntry>>,
    capacity: Mutex<usize>,
    live: broadcast::Sender<LogEntry>,
}

impl RequestLog {
    pub fn new(capacity: usize) -> Self {
        let (live, _) = broadcast::channel(256);
        Self {
            entries: Mutex::new(VecDeque::with_capacity(capacity.min(4096))),
            capacity: Mutex::new(capacity.max(1)),
            live,
        }
    }

    pub fn set_capacity(&self, capacity: usize) {
        let capacity = capacity.max(1);
        *self.capacity.lock().unwrap() = capacity;
        let mut entries = self.entries.lock().unwrap();
        while entries.len() > capacity {
            entries.pop_front();
        }
    }

    pub fn push(&self, entry: LogEntry) {
        let capacity = *self.capacity.lock().unwrap();
        {
            let mut entries = self.entries.lock().unwrap();
            while entries.len() >= capacity {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
        }
        // No subscribers is normal.
        let _ = self.live.send(entry);
    }

    /// Up to `limit` newest entries, oldest first, optionally for one host.
    pub fn recent(&self, host: Option<&str>, limit: usize) -> Vec<LogEntry> {
        let entries = self.entries.lock().unwrap();
        let mut out: Vec<LogEntry> = entries
            .iter()
            .rev()
            .filter(|e| host.is_none_or(|h| e.matches_host(h)))
            .take(limit)
            .cloned()
            .collect();
        out.reverse();
        out
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.live.subscribe()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T8
    #[test]
    fn keeps_only_the_newest_entries() {
        let log = RequestLog::new(1000);
        for i in 0..1001 {
            log.push(LogEntry::http("GET", "shop.localhost", &format!("/{i}"), 200, 1));
        }
        assert_eq!(log.len(), 1000);
        let all = log.recent(None, usize::MAX);
        assert!(matches!(&all[0], LogEntry::Http { path, .. } if path == "/1"));
        assert!(matches!(&all[999], LogEntry::Http { path, .. } if path == "/1000"));
    }

    // T8, I10
    #[test]
    fn query_string_is_never_stored() {
        let e = LogEntry::http("GET", "shop.localhost:443", "/cb?token=abc", 200, 1);
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("token"), "{json}");
        assert!(matches!(e, LogEntry::Http { ref path, ref host, .. } if path == "/cb" && host == "shop.localhost"));
    }

    // ADR 06, T13, I11: a proxy entry stores no query either.
    #[test]
    fn proxy_entries_keep_no_query_and_mark_the_way_in() {
        let e = LogEntry::http("GET", "api.example.com:443", "/a?token=x", 200, 3).via_proxy(ProxyMode::Inspect);
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["path"], "/a");
        assert_eq!(json["host"], "api.example.com");
        assert_eq!(json["via"], "proxy");
        assert_eq!(json["mode"], "inspect");
        assert!(json.get("bytes_in").is_none());
        assert!(!json.to_string().contains("token"));
    }

    #[test]
    fn a_tunnel_entry_has_connect_an_empty_path_and_bytes() {
        let e = LogEntry::tunnel("example.com:443", 200, 50, 10, 20);
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["kind"], "http");
        assert_eq!(json["method"], "CONNECT");
        assert_eq!(json["path"], "");
        assert_eq!(json["host"], "example.com");
        assert_eq!(json["mode"], "tunnel");
        assert_eq!((json["bytes_in"].as_u64(), json["bytes_out"].as_u64()), (Some(10), Some(20)));
    }

    // ADR 06, I16: a router entry looks exactly as before.
    #[test]
    fn a_router_entry_has_no_proxy_fields() {
        let json = serde_json::to_value(LogEntry::http("GET", "shop.localhost", "/", 200, 1)).unwrap();
        for key in ["via", "mode", "bytes_in", "bytes_out"] {
            assert!(json.get(key).is_none(), "{key}");
        }
    }

    #[test]
    fn filter_by_host_includes_subdomains() {
        let log = RequestLog::new(10);
        log.push(LogEntry::http("GET", "shop.localhost", "/", 200, 1));
        log.push(LogEntry::http("GET", "feat.shop.localhost", "/", 200, 1));
        log.push(LogEntry::http("GET", "blog.localhost", "/", 200, 1));
        assert_eq!(log.recent(Some("shop"), 10).len(), 2);
        assert_eq!(log.recent(Some("feat.shop.localhost"), 10).len(), 1);
        assert_eq!(log.recent(None, 2).len(), 2);
    }

    #[test]
    fn shrinking_capacity_drops_old_entries() {
        let log = RequestLog::new(10);
        for _ in 0..10 {
            log.push(LogEntry::http("GET", "a.localhost", "/", 200, 1));
        }
        log.set_capacity(3);
        assert_eq!(log.len(), 3);
    }

    #[tokio::test]
    async fn subscribers_get_new_entries() {
        let log = RequestLog::new(10);
        let mut rx = log.subscribe();
        log.push(LogEntry::http("GET", "a.localhost", "/x", 200, 1));
        assert_eq!(rx.recv().await.unwrap().host(), "a.localhost");
    }
}
