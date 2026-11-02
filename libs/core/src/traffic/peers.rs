//! The peer table (ADR 11, 1.4): for each open client connection of the
//! router and the forward proxy, keyed by the client's port, the host it
//! asked for and the server the proxy connected to. nettop knows the app of
//! a connection but, for proxy traffic, only the connection to the proxy
//! port; the client's port joins the two.
//!
//! In memory only. It is not the HAR log and is filled with `proxy_log` off.
//! Bounded (4,096 entries, oldest dropped) and behind a plain mutex that is
//! never held across an `.await` (I13). A closed connection stays 30 s, so a
//! sample taken just after the close still finds it.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::attribute::PeerInfo;

pub const CAPACITY: usize = 4096;
pub const KEEP_CLOSED: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct PeerTable {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    map: HashMap<u16, Entry>,
    /// Insertion order, for dropping the oldest. An item whose `seq` no
    /// longer matches its entry is stale and skipped.
    order: VecDeque<(u16, u64)>,
    seq: u64,
}

#[derive(Debug)]
struct Entry {
    info: PeerInfo,
    seq: u64,
    closed_at: Option<Instant>,
}

impl PeerTable {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A client connected from `port`. The guard marks it closed when the
    /// connection ends.
    pub fn open(self: &Arc<Self>, port: u16, client: Option<&str>) -> PeerGuard {
        let mut inner = self.inner.lock().unwrap();
        inner.insert(port, PeerInfo { client: client.map(str::to_string), host: None, upstream: None });
        PeerGuard { table: self.clone(), port }
    }

    /// The host the connection from `port` asked for, and the server the
    /// proxy reached for it, when known. Adds the entry when it is missing.
    pub fn set_target(&self, port: u16, host: &str, upstream: Option<SocketAddr>) {
        let mut inner = self.inner.lock().unwrap();
        match inner.map.get_mut(&port) {
            Some(e) => {
                e.info.host = Some(host.to_string());
                if upstream.is_some() {
                    e.info.upstream = upstream;
                }
            }
            None => inner.insert(port, PeerInfo { client: None, host: Some(host.to_string()), upstream }),
        }
    }

    fn close(&self, port: u16) {
        if let Some(e) = self.inner.lock().unwrap().map.get_mut(&port) {
            e.closed_at = Some(Instant::now());
        }
    }

    /// The entry of the client port, unless it closed more than 30 s ago.
    pub fn get(&self, port: u16) -> Option<PeerInfo> {
        let inner = self.inner.lock().unwrap();
        let e = inner.map.get(&port)?;
        if e.closed_at.is_some_and(|t| t.elapsed() > KEEP_CLOSED) {
            return None;
        }
        Some(e.info.clone())
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Inner {
    fn insert(&mut self, port: u16, info: PeerInfo) {
        self.prune();
        self.seq += 1;
        let seq = self.seq;
        self.map.insert(port, Entry { info, seq, closed_at: None });
        self.order.push_back((port, seq));
        while self.map.len() > CAPACITY {
            let Some((old, s)) = self.order.pop_front() else { break };
            if self.map.get(&old).is_some_and(|e| e.seq == s) {
                self.map.remove(&old);
            }
        }
        // Stale order items: never more than a few times the map.
        if self.order.len() > CAPACITY * 4 {
            let map = &self.map;
            self.order.retain(|(p, s)| map.get(p).is_some_and(|e| e.seq == *s));
        }
    }

    /// Drop entries closed more than 30 s ago from the front of the order.
    fn prune(&mut self) {
        while let Some(&(port, seq)) = self.order.front() {
            match self.map.get(&port) {
                Some(e) if e.seq == seq => {
                    if e.closed_at.is_some_and(|t| t.elapsed() > KEEP_CLOSED) {
                        self.map.remove(&port);
                        self.order.pop_front();
                    } else {
                        break;
                    }
                }
                _ => {
                    self.order.pop_front();
                }
            }
        }
    }
}

/// Marks a connection closed when dropped.
#[derive(Debug)]
pub struct PeerGuard {
    table: Arc<PeerTable>,
    port: u16,
}

impl Drop for PeerGuard {
    fn drop(&mut self) {
        self.table.close(self.port);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_target_close_and_lookup() {
        let t = PeerTable::new();
        let guard = t.open(61002, Some("chrome"));
        t.set_target(61002, "github.com", Some("203.0.113.4:443".parse().unwrap()));
        let info = t.get(61002).unwrap();
        assert_eq!(info.client.as_deref(), Some("chrome"));
        assert_eq!(info.host.as_deref(), Some("github.com"));
        // A later request on the same connection without an address keeps it.
        t.set_target(61002, "api.github.com", None);
        assert_eq!(t.get(61002).unwrap().upstream, Some("203.0.113.4:443".parse().unwrap()));
        drop(guard);
        // Closed, but still there for a sample taken just after.
        assert!(t.get(61002).is_some());
        assert!(t.get(1).is_none());
    }

    #[test]
    fn the_table_is_bounded() {
        let t = PeerTable::new();
        let guards: Vec<_> = (0..5000u32).map(|i| t.open(i as u16, None)).collect();
        assert_eq!(t.len(), CAPACITY);
        // The oldest were dropped, the newest are there.
        assert!(t.get(0).is_none());
        assert!(t.get(4999).is_some());
        drop(guards);
    }
}
