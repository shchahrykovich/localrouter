//! Shared HTTP listeners: ports 80 and 443 on `0.0.0.0` and `[::]`, with the
//! peer check before any byte is read (invariant I2). See ADR 01, change 3.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};

use localrouter_core::api::PortStatus;
use localrouter_core::config::LanNetwork;
use socket2::{Domain, Socket, Type};

/// Loopback peers: `127.0.0.0/8`, `::1` and `::ffff:127.x.x.x`.
pub fn is_loopback_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    }
}

/// Accept a connection to ports 80 and 443? Always yes for loopback, with
/// no lookup. Another machine only with `allow_lan` on and the network it
/// came in on in `lan_networks` (ADR 08, I17); an unknown network is refused.
/// `network_of` gives the network id and runs only for such a peer. The
/// proxy port, TCP routes and the viewer never use this (I19).
pub fn peer_allowed(
    ip: IpAddr,
    allow_lan: bool,
    networks: &[LanNetwork],
    network_of: impl FnOnce() -> Option<String>,
) -> bool {
    if is_loopback_peer(ip) {
        return true;
    }
    if !allow_lan || networks.is_empty() {
        return false;
    }
    let Some(id) = network_of() else { return false };
    networks.iter().any(|n| n.id == id)
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
///
/// A wildcard socket is kept only if a connection to the loopback address of
/// its family really reaches it. `SO_REUSEADDR` lets `0.0.0.0:80` bind next to
/// another program's `127.0.0.1:80`, and that program would then get every
/// request while the status said "bound".
pub fn bind_all(port: u16) -> (Vec<std::net::TcpListener>, PortStatus) {
    let mut status = PortStatus { configured: port, port: None, bound: vec![], errors: vec![] };
    let mut listeners = vec![];
    let mut actual = port;
    for (ip, loopback) in [
        (IpAddr::V4(Ipv4Addr::UNSPECIFIED), IpAddr::V4(Ipv4Addr::LOCALHOST)),
        (IpAddr::V6(Ipv6Addr::UNSPECIFIED), IpAddr::V6(Ipv6Addr::LOCALHOST)),
    ] {
        let addr = SocketAddr::new(ip, actual);
        match bind(addr) {
            Ok(l) => {
                let local = l.local_addr().map(|a| a.port()).unwrap_or(actual);
                actual = local;
                let probe = SocketAddr::new(loopback, local);
                if !receives(&l, probe) {
                    status.errors.push(format!(
                        "{probe}: port {local} is held by another program, so loopback requests would not reach LocalRouter"
                    ));
                    continue;
                }
                status.port = Some(local);
                status.bound.push(SocketAddr::new(ip, local).to_string());
                listeners.push(l);
            }
            Err(e) => status.errors.push(describe_bind_error(addr, &e)),
        }
    }
    (listeners, status)
}

/// Does a connection to `probe` arrive at `listener`? The listener is
/// non-blocking, so poll accept() for a short while.
fn receives(listener: &std::net::TcpListener, probe: SocketAddr) -> bool {
    let Ok(stream) = std::net::TcpStream::connect_timeout(&probe, std::time::Duration::from_millis(300)) else {
        return false;
    };
    let Ok(ours) = stream.local_addr() else { return false };
    for _ in 0..30 {
        match listener.accept() {
            Ok((_, peer)) if peer == ours => return true,
            Ok(_) => continue,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(_) => return false,
        }
    }
    false
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
            tracing::warn!("refused a connection from {peer}: LAN access is off, or not allowed on this network");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn home() -> Vec<LanNetwork> {
        vec![LanNetwork { id: "mac:18:35:d1:15:d1:a8".into(), name: "Home".into(), router: "192.168.0.1".into() }]
    }

    fn no_lookup() -> Option<String> {
        panic!("the network must not be looked up")
    }

    // T4, I2; ADR 08, T14: loopback is accepted with no lookup.
    #[test]
    fn loopback_peers_are_accepted() {
        for s in ["127.0.0.1", "127.8.8.8", "::1", "::ffff:127.0.0.1"] {
            assert!(peer_allowed(ip(s), false, &[], no_lookup), "{s}");
            assert!(peer_allowed(ip(s), true, &home(), no_lookup), "{s}");
        }
    }

    // T4, I2; ADR 08, T14, I17: another machine only on an allowed network.
    #[test]
    fn other_peers_are_refused_unless_lan_is_allowed_on_this_network() {
        let home_id = || Some("mac:18:35:d1:15:d1:a8".to_string());
        let cafe_id = || Some("mac:aa:bb:cc:dd:ee:ff".to_string());
        for s in ["192.168.1.5", "10.0.0.1", "::ffff:10.0.0.1", "fe80::1", "2001:db8::1", "0.0.0.0"] {
            assert!(!peer_allowed(ip(s), false, &home(), no_lookup), "{s}: allow_lan off");
            assert!(!peer_allowed(ip(s), true, &[], no_lookup), "{s}: no allowed network");
            assert!(peer_allowed(ip(s), true, &home(), home_id), "{s}: home");
            assert!(!peer_allowed(ip(s), true, &home(), cafe_id), "{s}: another network");
            assert!(!peer_allowed(ip(s), true, &home(), || None), "{s}: unknown network");
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

    // A program that already listens on 127.0.0.1:<port> would silently get
    // every loopback request, because SO_REUSEADDR lets the wildcard bind
    // succeed next to it. bind_all must notice and report it.
    #[test]
    fn loopback_port_held_by_another_program_is_reported() {
        let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = other.local_addr().unwrap().port();
        let (listeners, status) = bind_all(port);
        let v4: Vec<_> = listeners.iter().filter(|l| l.local_addr().unwrap().is_ipv4()).collect();
        assert!(v4.is_empty(), "0.0.0.0:{port} must not stay bound: {status:?}");
        assert!(
            status.errors.iter().any(|e| e.contains("127.0.0.1") && e.contains("another program")),
            "{status:?}"
        );
    }

    #[test]
    fn port_in_use_is_reported() {
        let (_held, status) = bind_all(0);
        let port = status.port.unwrap();
        let (listeners, again) = bind_all(port);
        assert!(listeners.is_empty() || !again.errors.is_empty(), "{again:?}");
    }
}
