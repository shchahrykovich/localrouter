//! Which network a connection came in on (ADR 08, change 4).
//!
//! A network is known by its router's MAC address: `mac:18:35:d1:15:d1:a8`.
//! That needs no Location Services permission, unlike the Wi-Fi name.
//!
//! 1. The local address of the accepted socket gives the interface
//!    (`getifaddrs`).
//! 2. The System Configuration store gives that interface's router and the
//!    router's MAC address: the `IPv4` state of each network service has
//!    `InterfaceName`, `ARPResolvedIPAddress` and `ARPResolvedHardwareAddress`
//!    (the values of macOS's own `NetworkSignature`).
//!
//! The ARP table (`sysctl NET_RT_FLAGS`) is not used: on macOS 26 it is
//! empty for a program without Local Network access. The store is read over
//! configd's Mach port: no program runs, no permission is asked, and nothing
//! is cached or watched (I20).

use std::ffi::{CStr, CString, c_char, c_void};
use std::net::{IpAddr, Ipv4Addr};

/// The network of one interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    /// `mac:18:35:d1:15:d1:a8`.
    pub id: String,
    pub router: Ipv4Addr,
    pub interface: String,
}

/// What the store says about one network service's IPv4 state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceIpv4 {
    pub interface: Option<String>,
    pub router: Option<String>,
    pub router_mac: Option<String>,
}

/// Interfaces that never identify a network: VPN and tunnels.
const UNKNOWN_KINDS: [&str; 5] = ["utun", "ipsec", "ppp", "gif", "stf"];

/// The network of the interface that owns `local` (the local address of an
/// accepted socket). `None` when it cannot be recognised: a VPN, no router,
/// no resolved router address.
pub fn of_local_ip(local: IpAddr) -> Option<Network> {
    let interface = interface_of(local.to_canonical())?;
    network_of_interface(&interface, &sc::ipv4_services())
}

/// The network of the primary interface: what status shows as "this network".
pub fn current() -> Option<Network> {
    let interface = sc::primary_interface()?;
    network_of_interface(&interface, &sc::ipv4_services())
}

/// Pick the interface's service and read its router. Pure, for tests.
pub fn network_of_interface(interface: &str, services: &[ServiceIpv4]) -> Option<Network> {
    if UNKNOWN_KINDS.iter().any(|k| interface.starts_with(k)) {
        return None;
    }
    let service = services.iter().find(|s| s.interface.as_deref() == Some(interface))?;
    let router: Ipv4Addr = service.router.as_deref()?.parse().ok()?;
    let mac = parse_mac(service.router_mac.as_deref()?)?;
    Some(Network { id: mac_id(&mac), router, interface: interface.to_string() })
}

/// `18:35:d1:15:d1:a8`, also with one-digit parts (`0:1b:…`).
pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 2 {
            return None;
        }
        mac[i] = u8::from_str_radix(p, 16).ok()?;
    }
    (mac != [0; 6] && mac != [0xff; 6]).then_some(mac)
}

pub fn mac_id(mac: &[u8; 6]) -> String {
    let hex: Vec<String> = mac.iter().map(|b| format!("{b:02x}")).collect();
    format!("mac:{}", hex.join(":"))
}

/// The interface that owns a local address.
fn interface_of(local: IpAddr) -> Option<String> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list`; it is freed below with freeifaddrs.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut cur = list;
    while !cur.is_null() {
        // SAFETY: `cur` is a node of the list getifaddrs returned.
        let ifa = unsafe { &*cur };
        if !ifa.ifa_addr.is_null() {
            // SAFETY: ifa_addr points to a sockaddr of its family's size.
            let ip = unsafe {
                match (*ifa.ifa_addr).sa_family as i32 {
                    libc::AF_INET => {
                        let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                        Some(IpAddr::V4(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr))))
                    }
                    libc::AF_INET6 => {
                        let sin6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                        Some(IpAddr::V6(std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr)))
                    }
                    _ => None,
                }
            };
            if ip == Some(local) {
                // SAFETY: ifa_name is a NUL-terminated string in the list.
                found = Some(unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned());
                break;
            }
        }
        cur = ifa.ifa_next;
    }
    // SAFETY: `list` came from getifaddrs and is freed once.
    unsafe { libc::freeifaddrs(list) };
    found
}

/// A few calls of SystemConfiguration and CoreFoundation, by hand: the
/// daemon has no crate for them.
mod sc {
    use super::*;

    type CFTypeRef = *const c_void;
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFArrayGetTypeID() -> usize;
        fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, encoding: u32) -> CFTypeRef;
        fn CFStringGetCString(s: CFTypeRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFArrayGetCount(a: CFTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(a: CFTypeRef, i: isize) -> CFTypeRef;
        fn CFDictionaryGetValue(d: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    }

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCDynamicStoreCreate(alloc: CFTypeRef, name: CFTypeRef, callout: CFTypeRef, ctx: CFTypeRef) -> CFTypeRef;
        fn SCDynamicStoreCopyKeyList(store: CFTypeRef, pattern: CFTypeRef) -> CFTypeRef;
        fn SCDynamicStoreCopyValue(store: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    }

    /// An owned CoreFoundation object, released on drop.
    struct Owned(CFTypeRef);

    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the object came from a Create or Copy call.
                unsafe { CFRelease(self.0) };
            }
        }
    }

    impl Owned {
        fn some(self) -> Option<Self> {
            (!self.0.is_null()).then_some(self)
        }
    }

    fn cfstr(s: &str) -> Option<Owned> {
        let c = CString::new(s).ok()?;
        // SAFETY: `c` is a NUL-terminated UTF-8 string.
        Owned(unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) }).some()
    }

    /// A CFString as a Rust string; `None` for any other type.
    fn string(v: CFTypeRef) -> Option<String> {
        // SAFETY: `v` is a live CF object; its type is checked first.
        unsafe {
            if v.is_null() || CFGetTypeID(v) != CFStringGetTypeID() {
                return None;
            }
            let mut buf = [0 as c_char; 256];
            if CFStringGetCString(v, buf.as_mut_ptr(), buf.len() as isize, UTF8) == 0 {
                return None;
            }
            Some(CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
        }
    }

    /// `dict[key]` as a string.
    fn get(dict: CFTypeRef, key: &str) -> Option<String> {
        let k = cfstr(key)?;
        // SAFETY: `dict` is a CFDictionary (checked by the caller); the
        // value is borrowed, not owned.
        string(unsafe { CFDictionaryGetValue(dict, k.0) })
    }

    fn store() -> Option<Owned> {
        let name = cfstr("lan-network-check")?;
        // SAFETY: no callout, no context: a store for reads only.
        Owned(unsafe { SCDynamicStoreCreate(std::ptr::null(), name.0, std::ptr::null(), std::ptr::null()) }).some()
    }

    /// The value of one key, when it is a dictionary.
    fn dict(store: &Owned, key: CFTypeRef) -> Option<Owned> {
        // SAFETY: `store` is live; the copy is owned and released on drop.
        let d = Owned(unsafe { SCDynamicStoreCopyValue(store.0, key) }).some()?;
        // SAFETY: `d` is live.
        (unsafe { CFGetTypeID(d.0) == CFDictionaryGetTypeID() }).then_some(d)
    }

    /// `State:/Network/Service/<id>/IPv4` of every service.
    pub fn ipv4_services() -> Vec<ServiceIpv4> {
        let Some(store) = store() else { return vec![] };
        let Some(pattern) = cfstr("State:/Network/Service/[^/]+/IPv4") else { return vec![] };
        // SAFETY: `store` and `pattern` are live; the list is owned.
        let Some(keys) = Owned(unsafe { SCDynamicStoreCopyKeyList(store.0, pattern.0) }).some() else { return vec![] };
        // SAFETY: `keys` is live; its type is checked.
        if unsafe { CFGetTypeID(keys.0) != CFArrayGetTypeID() } {
            return vec![];
        }
        // SAFETY: `keys` is a CFArray.
        let count = unsafe { CFArrayGetCount(keys.0) };
        let mut out = vec![];
        for i in 0..count {
            // SAFETY: `i` is in range; the key is borrowed from the array.
            let key = unsafe { CFArrayGetValueAtIndex(keys.0, i) };
            let Some(d) = dict(&store, key) else { continue };
            out.push(ServiceIpv4 {
                interface: get(d.0, "InterfaceName"),
                router: get(d.0, "ARPResolvedIPAddress").or_else(|| get(d.0, "Router")),
                router_mac: get(d.0, "ARPResolvedHardwareAddress"),
            });
        }
        out
    }

    /// `PrimaryInterface` of `State:/Network/Global/IPv4`.
    pub fn primary_interface() -> Option<String> {
        let store = store()?;
        let key = cfstr("State:/Network/Global/IPv4")?;
        let d = dict(&store, key.0)?;
        get(d.0, "PrimaryInterface")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(interface: &str, router: &str, mac: Option<&str>) -> ServiceIpv4 {
        ServiceIpv4 { interface: Some(interface.into()), router: Some(router.into()), router_mac: mac.map(str::to_string) }
    }

    // ADR 08, T15: the store's values of the Mac in change 4 give its id.
    #[test]
    fn the_router_mac_of_the_interface() {
        let services = vec![
            service("en7", "10.0.0.1", Some("0:1b:2c:3d:4e:5f")),
            service("en0", "192.168.0.1", Some("18:35:d1:15:d1:a8")),
        ];
        let n = network_of_interface("en0", &services).unwrap();
        assert_eq!(n, Network { id: "mac:18:35:d1:15:d1:a8".into(), router: Ipv4Addr::new(192, 168, 0, 1), interface: "en0".into() });
        assert_eq!(network_of_interface("en7", &services).unwrap().id, "mac:00:1b:2c:3d:4e:5f", "one-digit parts");
    }

    // T15: no service, no resolved router, a VPN: unknown.
    #[test]
    fn unknown_networks() {
        let services = vec![service("en0", "192.168.0.1", None), service("utun3", "10.8.0.1", Some("aa:bb:cc:dd:ee:ff"))];
        assert_eq!(network_of_interface("en0", &services), None, "no ARP entry for the router");
        assert_eq!(network_of_interface("utun3", &services), None, "a VPN");
        assert_eq!(network_of_interface("en5", &services), None, "no service");
        for bad in ["", "18:35:d1:15:d1", "18:35:d1:15:d1:a8:00", "0:0:0:0:0:0", "zz:35:d1:15:d1:a8", "183:5:d1:15:d1:a8"] {
            assert_eq!(parse_mac(bad), None, "{bad}");
        }
    }

    // Reads this Mac's real store; prints what it finds (M9).
    #[test]
    fn the_real_store_is_readable() {
        eprintln!("services: {:?}; primary: {:?}; current: {:?}", sc::ipv4_services(), sc::primary_interface(), current());
    }
}
