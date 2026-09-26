//! One loopback listener pair per TCP route: `127.0.0.1` and `::1` only
//! (invariant I15), whatever `allow_lan` says. See ADR 01, change 7.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::listen;

/// Bind `port` (or a free port for `0`) on both loopback addresses. Either
/// both sockets are bound or none is (invariant I16).
pub fn bind_loopback(port: u16) -> io::Result<(Vec<std::net::TcpListener>, u16)> {
    let v4 = listen::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))?;
    let actual = v4.local_addr()?.port();
    let v6 = listen::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), actual))?;
    Ok((vec![v4, v6], actual))
}

#[cfg(test)]
mod tests {
    use super::*;

    // T12, I15
    #[test]
    fn sockets_are_loopback_only() {
        let (listeners, port) = bind_loopback(0).unwrap();
        assert!(port >= 1024);
        for l in &listeners {
            let addr = l.local_addr().unwrap();
            assert!(addr.ip().is_loopback(), "{addr}");
            assert_eq!(addr.port(), port);
        }
    }

    // T12, I16
    #[test]
    fn port_taken_on_ipv6_only_leaves_ipv4_free() {
        // Find a port free on both, then hold it on ::1 only.
        let (probe, port) = bind_loopback(0).unwrap();
        drop(probe);
        let _held = listen::bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)).unwrap();
        assert!(bind_loopback(port).is_err());
        // The IPv4 socket bound during the failed call must be closed again.
        let v4 = listen::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
        assert!(v4.is_ok(), "127.0.0.1:{port} still held: {v4:?}");
    }
}
