//! Request log: a fixed-size ring buffer in memory, plus a live stream.
//!
//! Entries never hold query strings, headers or bodies (invariant I10).

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
        }
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
