//! Server names of remote addresses by reverse DNS (a PTR lookup), for
//! direct traffic, which nettop shows only as addresses.
//!
//! A server name is how the server calls itself, not the domain the app
//! asked for: `34.149.66.165` gives
//! `165.66.149.34.bc.googleusercontent.com`, many addresses give nothing.
//! Each new address sends one query to the Mac's DNS resolver, so this runs
//! only while `traffic_names` is on.
//!
//! A lookup never runs on the request path: [`ServerNames::names`] answers
//! from the cache and queues the addresses it does not know. Answers, also
//! "no name", are kept an hour; the cache holds at most 10,000 addresses.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const CAPACITY: usize = 10_000;
pub const KEEP: Duration = Duration::from_secs(3600);
/// Lookups running at once.
pub const PARALLEL: usize = 8;

/// The lookup itself: blocking, may take seconds. `None` when the address
/// has no name.
pub type Lookup = Arc<dyn Fn(IpAddr) -> Option<String> + Send + Sync>;

pub struct ServerNames {
    lookup: Lookup,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    known: HashMap<IpAddr, (Option<String>, Instant)>,
    order: VecDeque<IpAddr>,
    /// Asked for and not answered yet.
    pending: HashSet<IpAddr>,
    queue: VecDeque<IpAddr>,
    running: usize,
}

impl ServerNames {
    pub fn new(lookup: Lookup) -> Arc<Self> {
        Arc::new(Self { lookup, inner: Mutex::new(Inner::default()) })
    }

    /// Whether an address is worth a lookup: not loopback, not unspecified,
    /// not link-local (those are this Mac and its own network cable).
    pub fn wanted(ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
            return false;
        }
        match ip {
            IpAddr::V4(v4) => !v4.is_link_local() && !v4.is_broadcast(),
            IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
        }
    }

    /// The known names of `ips`. Unknown ones are queued and looked up on
    /// threads of their own; a later call gets their names.
    pub fn names(self: &Arc<Self>, ips: impl IntoIterator<Item = IpAddr>) -> HashMap<IpAddr, String> {
        let mut out = HashMap::new();
        let mut inner = self.inner.lock().unwrap();
        for ip in ips {
            if !Self::wanted(ip) {
                continue;
            }
            match inner.known.get(&ip) {
                Some((name, at)) if at.elapsed() < KEEP => {
                    if let Some(name) = name {
                        out.insert(ip, name.clone());
                    }
                }
                _ => {
                    if inner.pending.insert(ip) {
                        inner.queue.push_back(ip);
                    }
                }
            }
        }
        let start = PARALLEL.saturating_sub(inner.running).min(inner.queue.len());
        inner.running += start;
        drop(inner);
        for _ in 0..start {
            let this = self.clone();
            std::thread::Builder::new()
                .name("reverse-dns".into())
                .spawn(move || this.work())
                .map_err(|e| tracing::warn!("cannot start a reverse DNS thread: {e}"))
                .ok();
        }
        out
    }

    /// Take addresses from the queue until it is empty.
    fn work(&self) {
        loop {
            let Some(ip) = ({
                let mut inner = self.inner.lock().unwrap();
                let next = inner.queue.pop_front();
                if next.is_none() {
                    inner.running -= 1;
                }
                next
            }) else {
                return;
            };
            let name = (self.lookup)(ip).map(|n| n.trim_end_matches('.').to_ascii_lowercase()).filter(|n| !n.is_empty());
            let mut inner = self.inner.lock().unwrap();
            inner.pending.remove(&ip);
            if inner.known.insert(ip, (name, Instant::now())).is_none() {
                inner.order.push_back(ip);
            }
            while inner.known.len() > CAPACITY {
                let Some(old) = inner.order.pop_front() else { break };
                inner.known.remove(&old);
            }
        }
    }

    /// Wait until no lookup is queued or running (tests).
    pub fn settle(&self, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        while Instant::now() < until {
            let inner = self.inner.lock().unwrap();
            if inner.running == 0 && inner.queue.is_empty() {
                return true;
            }
            drop(inner);
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().known.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The part of a server name that names the network: the last two labels,
/// or three after a short second-level label (`co.uk`, `com.au`).
/// `165.66.149.34.bc.googleusercontent.com` gives `googleusercontent.com`.
pub fn base_domain(name: &str) -> &str {
    let labels: Vec<&str> = name.split('.').collect();
    if labels.len() <= 2 {
        return name;
    }
    let second = labels[labels.len() - 2];
    let keep = if labels.len() >= 3 && second.len() <= 3 && labels[labels.len() - 1].len() == 2 { 3 } else { 2 };
    let skip: usize = labels[..labels.len() - keep].iter().map(|l| l.len() + 1).sum();
    &name[skip..]
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn fake(calls: Arc<AtomicUsize>) -> Lookup {
        Arc::new(move |ip: IpAddr| {
            calls.fetch_add(1, Ordering::SeqCst);
            match ip.to_string().as_str() {
                "34.149.66.165" => Some("165.66.149.34.bc.googleusercontent.com.".into()),
                _ => None,
            }
        })
    }

    #[test]
    fn a_name_comes_on_a_later_call_and_is_cached() {
        let calls = Arc::new(AtomicUsize::new(0));
        let names = ServerNames::new(fake(calls.clone()));
        let ip: IpAddr = "34.149.66.165".parse().unwrap();
        let none: IpAddr = "203.0.113.4".parse().unwrap();
        // The first call never waits for a lookup.
        assert!(names.names([ip, none]).is_empty());
        assert!(names.settle(Duration::from_secs(5)));
        let got = names.names([ip, none]);
        assert_eq!(got.get(&ip).map(String::as_str), Some("165.66.149.34.bc.googleusercontent.com"));
        assert!(!got.contains_key(&none));
        // Both answers, also "no name", are cached: no second lookup.
        names.names([ip, none, ip]);
        assert!(names.settle(Duration::from_secs(5)));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn this_mac_and_its_cable_are_never_looked_up() {
        let calls = Arc::new(AtomicUsize::new(0));
        let names = ServerNames::new(fake(calls.clone()));
        let local = ["127.0.0.1", "::1", "0.0.0.0", "169.254.1.2", "fe80::1", "224.0.0.251", "::ffff:127.0.0.1"];
        names.names(local.iter().map(|s| s.parse::<IpAddr>().unwrap()));
        assert!(names.settle(Duration::from_secs(5)));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(ServerNames::wanted("192.168.0.10".parse().unwrap()));
    }

    #[test]
    fn many_addresses_are_looked_up_by_few_threads_and_the_cache_is_bounded() {
        let calls = Arc::new(AtomicUsize::new(0));
        let names = ServerNames::new(fake(calls.clone()));
        let ips: Vec<IpAddr> = (0..(CAPACITY as u32 + 500)).map(|i| IpAddr::from((0x0a00_0000u32 + i).to_be_bytes())).collect();
        names.names(ips.iter().copied());
        assert!(names.inner.lock().unwrap().running <= PARALLEL);
        assert!(names.settle(Duration::from_secs(30)));
        assert_eq!(calls.load(Ordering::SeqCst), ips.len());
        assert_eq!(names.len(), CAPACITY);
    }

    #[test]
    fn base_domains() {
        assert_eq!(base_domain("165.66.149.34.bc.googleusercontent.com"), "googleusercontent.com");
        assert_eq!(base_domain("lhr25s34-in-f14.1e100.net"), "1e100.net");
        assert_eq!(base_domain("server-1.cdn.example.co.uk"), "example.co.uk");
        assert_eq!(base_domain("example.com"), "example.com");
        assert_eq!(base_domain("localhost"), "localhost");
    }
}
