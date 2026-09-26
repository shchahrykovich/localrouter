//! Shared HTTP listeners: ports 80 and 443 on `0.0.0.0` and `[::]`, with the
//! peer check before any byte is read (invariant I2). See ADR 01, change 3.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};

use localrouter_core::api::PortStatus;
use socket2::{Domain, Socket, Type};

/// Loopback peers: `127.0.0.0/8`, `::1` and `::ffff:127.x.x.x`.
pub fn is_loopback_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    }
}

/// Accept the connection? Always yes for loopback; otherwise only with `allow_lan`.
pub fn peer_allowed(ip: IpAddr, allow_lan: bool) -> bool {
    allow_lan || is_loopback_peer(ip)
}

/// Bind one listening socket. IPv6 sockets are IPv6-only, so the two families never overlap.
pub fn bind(addr: SocketAddr) -> io::Result<std::net::TcpListener> {
    let domain = if addr.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
    let socket = Socket::new(domain, Type::STREAM, None)?;
    if addr.is_ipv6() {
        socket.set_only_v6(true)?;
    }
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    Ok(socket.into())
}

/// Bind `port` on all IPv4 and all IPv6 interfaces. Port `0` picks one free
/// port for IPv4 and uses the same number for IPv6.
pub fn bind_all(port: u16) -> (Vec<std::net::TcpListener>, PortStatus) {
    let mut status = PortStatus { configured: port, port: None, bound: vec![], errors: vec![] };
    let mut listeners = vec![];
    let mut actual = port;
    for ip in [IpAddr::V4(Ipv4Addr::UNSPECIFIED), IpAddr::V6(Ipv6Addr::UNSPECIFIED)] {
        let addr = SocketAddr::new(ip, actual);
        match bind(addr) {
            Ok(l) => {
                let local = l.local_addr().map(|a| a.port()).unwrap_or(actual);
                actual = local;
                status.port = Some(local);
                status.bound.push(SocketAddr::new(ip, local).to_string());
                listeners.push(l);
            }
            Err(e) => status.errors.push(describe_bind_error(addr, &e)),
        }
    }
    (listeners, status)
}

pub fn describe_bind_error(addr: SocketAddr, e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::AddrInUse => format!("{addr}: port {} is in use by another program", addr.port()),
        io::ErrorKind::PermissionDenied => format!("{addr}: permission denied"),
        _ => format!("{addr}: {e}"),
    }
}

/// Log at most one "refused peer" line per second.
pub struct RefusalLog {
    last_ms: AtomicU64,
}

impl RefusalLog {
    pub const fn new() -> Self {
        Self { last_ms: AtomicU64::new(0) }
    }

    pub fn refused(&self, peer: SocketAddr) {
        let now = localrouter_core::logs::now_ms();
        let last = self.last_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= 1000
            && self.last_ms.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
        {
            tracing::warn!("refused a connection from {peer}: not a loopback address (LAN access is off)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    // T4, I2
    #[test]
    fn loopback_peers_are_accepted() {
        for s in ["127.0.0.1", "127.8.8.8", "::1", "::ffff:127.0.0.1"] {
            assert!(peer_allowed(ip(s), false), "{s}");
        }
    }

    // T4, I2
    #[test]
    fn other_peers_are_refused_unless_lan_is_allowed() {
        for s in ["192.168.1.5", "10.0.0.1", "::ffff:10.0.0.1", "fe80::1", "2001:db8::1", "0.0.0.0"] {
            assert!(!peer_allowed(ip(s), false), "{s}");
            assert!(peer_allowed(ip(s), true), "{s}");
        }
    }

    #[test]
    fn port_zero_gives_the_same_port_on_both_families() {
        let (listeners, status) = bind_all(0);
        assert_eq!(listeners.len(), 2, "{status:?}");
        let ports: Vec<_> = listeners.iter().map(|l| l.local_addr().unwrap().port()).collect();
        assert_eq!(ports[0], ports[1]);
        assert!(status.errors.is_empty());
    }

    #[test]
    fn port_in_use_is_reported() {
        let (_held, status) = bind_all(0);
        let port = status.port.unwrap();
        let (listeners, again) = bind_all(port);
        assert!(listeners.is_empty() || !again.errors.is_empty(), "{again:?}");
    }
}
