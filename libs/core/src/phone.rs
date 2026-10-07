//! A phone on the forward proxy (ADR 10), the way Charles does it.
//!
//! A phone client's port listens on the LAN. The phone sets it as its Wi-Fi
//! proxy by hand (two values: server and port). There is no proxy password:
//! a device is allowed by its address. Opening the setup page from the QR
//! code allows the device that opened it; any other device waits until the
//! user presses Allow on the Mac. This module holds the parts that need no
//! socket: the setup token, the allowed devices, the rule that keeps another
//! machine away from this Mac's own servers (I5), the setup page and the CA
//! profile. The daemon keeps the tokens and the devices.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::RwLock;

use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};

/// 32 lowercase letters and digits without look-alikes: no `0`, `o`, `1`, `l`.
const ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";

/// `k7mq-2xph-9tdw-r4nc`: 80 bits from the system random source. The QR
/// code holds it; whoever opens the setup page with it is allowed.
pub fn new_token() -> String {
    let mut bytes = [0u8; 16];
    SystemRandom::new().fill(&mut bytes).expect("the system random source works");
    let mut out = String::with_capacity(19);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(ALPHABET[usize::from(b & 31)] as char);
    }
    out
}

/// `a == b`, looking at every byte whatever the first difference is (I4).
pub fn same_token(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The name the setup page loads a tiny image from over HTTPS. A phone port
/// answers it itself with a leaf of the inspection CA, so the handshake
/// tells whether the iPhone trusts the CA. `.invalid` is reserved: no real
/// host has it (RFC 6761).
pub const CHECK_HOST: &str = "trust-check.invalid";

/// Whether a device trusts the inspection CA, from its last TLS handshake
/// with an inspection leaf. Memory only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trust {
    /// No handshake that tells yet.
    #[default]
    Unknown,
    /// The device refused the certificate (`CertificateUnknown`, `UnknownCA`,
    /// `BadCertificate`): the CA is missing or not turned on.
    Refused,
    /// A handshake with an inspection leaf succeeded.
    Trusted,
}

/// A phone client as its port sees it: the name, the setup token and the
/// allowed devices. The daemon changes both; the port reads them per request.
/// The port also notes whether each device trusts the inspection CA.
#[derive(Debug)]
pub struct LanClient {
    pub name: String,
    token: RwLock<String>,
    devices: RwLock<Vec<IpAddr>>,
    trust: RwLock<HashMap<IpAddr, Trust>>,
}

impl LanClient {
    pub fn new(name: &str, token: &str, devices: Vec<IpAddr>) -> Self {
        Self {
            name: name.to_string(),
            token: RwLock::new(token.to_string()),
            devices: RwLock::new(devices),
            trust: RwLock::new(HashMap::new()),
        }
    }

    pub fn trust(&self, ip: IpAddr) -> Trust {
        self.trust.read().unwrap().get(&ip.to_canonical()).copied().unwrap_or_default()
    }

    pub fn set_trust(&self, ip: IpAddr, trust: Trust) {
        self.trust.write().unwrap().insert(ip.to_canonical(), trust);
    }

    pub fn token(&self) -> String {
        self.token.read().unwrap().clone()
    }

    pub fn set_token(&self, token: &str) {
        *self.token.write().unwrap() = token.to_string();
    }

    /// The token of a setup path.
    pub fn is_token(&self, token: &str) -> bool {
        same_token(token.as_bytes(), self.token.read().unwrap().as_bytes())
    }

    /// This device may use the port. IPv4-mapped IPv6 counts as IPv4.
    pub fn allows(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        self.devices.read().unwrap().iter().any(|d| d.to_canonical() == ip)
    }

    /// Allow a device. Returns false when it was allowed already.
    pub fn allow(&self, ip: IpAddr) -> bool {
        if self.allows(ip) {
            return false;
        }
        self.devices.write().unwrap().push(ip.to_canonical());
        true
    }

    pub fn forget(&self, ip: IpAddr) {
        let ip = ip.to_canonical();
        self.devices.write().unwrap().retain(|d| d.to_canonical() != ip);
    }
}

/// What the phone port tells the daemon, which keeps the devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneEvent {
    /// The device opened the setup page with the right token: it is allowed.
    Scanned { client: String, ip: IpAddr },
    /// A device that is not allowed sent a proxy request: the user decides.
    Asked { client: String, ip: IpAddr, host: String },
}

/// A target another machine must not reach through the phone port: this
/// Mac's loopback, the unspecified address, or one of its own addresses
/// (I5). IPv4-mapped IPv6 counts as its IPv4 address.
pub fn is_local_target(ip: IpAddr, own: &[IpAddr]) -> bool {
    let ip = ip.to_canonical();
    ip.is_loopback() || ip.is_unspecified() || own.iter().any(|o| o.to_canonical() == ip)
}

/// Every interface address of this Mac, with its interface name
/// (`getifaddrs`).
pub fn interface_addresses() -> Vec<(String, IpAddr)> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list`; it is freed below with freeifaddrs.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return vec![];
    }
    let mut out = vec![];
    let mut cur = list;
    while !cur.is_null() {
        // SAFETY: `cur` is a node of the list getifaddrs returned.
        let ifa = unsafe { &*cur };
        if !ifa.ifa_addr.is_null() {
            // SAFETY: ifa_addr points to a sockaddr of its family's size.
            let ip = unsafe {
                match i32::from((*ifa.ifa_addr).sa_family) {
                    libc::AF_INET => {
                        let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                        Some(IpAddr::V4(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr))))
                    }
                    libc::AF_INET6 => {
                        let sin6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                        Some(IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.s6_addr)))
                    }
                    _ => None,
                }
            };
            if let Some(ip) = ip {
                // SAFETY: ifa_name is a NUL-terminated string in the list.
                let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
                out.push((name, ip));
            }
        }
        cur = ifa.ifa_next;
    }
    // SAFETY: `list` came from getifaddrs and is freed once.
    unsafe { libc::freeifaddrs(list) };
    out
}

/// Every address of this Mac's interfaces, for [`is_local_target`].
pub fn own_addresses() -> Vec<IpAddr> {
    interface_addresses().into_iter().map(|(_, ip)| ip).collect()
}

/// The first IPv4 address of an interface: the server in the setup URL.
pub fn ipv4_of(interface: &str) -> Option<Ipv4Addr> {
    interface_addresses().into_iter().find_map(|(name, ip)| match ip {
        IpAddr::V4(v4) if name == interface => Some(v4),
        _ => None,
    })
}

/// What a setup path asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupFile {
    Page,
    CaProfile,
}

/// `/setup/<token>` and `/setup/<token>/ca.mobileconfig`. Anything else is
/// `None`.
pub fn parse_setup_path(path: &str) -> Option<(&str, SetupFile)> {
    let rest = path.strip_prefix("/setup/")?;
    let rest = rest.split('?').next().unwrap_or(rest);
    let (token, file) = match rest.split_once('/') {
        None => (rest, SetupFile::Page),
        Some((token, "")) => (token, SetupFile::Page),
        Some((token, "ca.mobileconfig")) => (token, SetupFile::CaProfile),
        Some(_) => return None,
    };
    (!token.is_empty()).then_some((token, file))
}

/// Everything the setup page and the CA profile show.
#[derive(Debug, Clone)]
pub struct Setup {
    pub app_name: String,
    pub bundle_id: String,
    pub token: String,
    /// The host the phone used to reach the port: the proxy server to type.
    pub server: String,
    pub port: u16,
    /// The inspection CA (DER) and its name, when HTTPS is inspected.
    pub ca: Option<(Vec<u8>, String)>,
    /// The device this visit allowed; `None` for this Mac.
    pub allowed: Option<IpAddr>,
    /// The page came through the proxy: the iPhone has set it.
    pub via_proxy: bool,
    /// Whether this device trusts the inspection CA.
    pub trust: Trust,
    /// Makes each check image URL new, so Safari never takes it from a cache.
    pub nonce: u64,
}

fn html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The setup page (ADR 10, change 2): only the next step the iPhone needs,
/// from what this request shows. No script: until the last step it reloads
/// itself every 3 s, and a Check Again button does the same by hand.
///
/// 1. Opened directly, not through the proxy: set the proxy.
/// 2. Through the proxy, and HTTPS is inspected, but the CA is not trusted
///    (or not known yet): install and trust it. The page loads an image from
///    [`CHECK_HOST`]; the port answers it with an inspection leaf, so the
///    handshake tells whether the iPhone trusts the CA.
/// 3. Through the proxy and trusted (or nothing to trust): connected.
pub fn setup_page(s: &Setup) -> String {
    let app = html(&s.app_name);
    let server = html(&s.server);
    let port = s.port;
    let ca_name = s.ca.as_ref().map(|(_, name)| html(name));
    let ok_mark = "<span class=\"badge done\">✓</span>";
    let mut body = String::new();
    let mut reload = true;
    let path = |parts: &[&str]| parts.iter().map(|p| format!("<b>{p}</b>")).collect::<Vec<_>>().join("<span class=\"arrow\">›</span>");
    if !s.via_proxy {
        body.push_str(&format!(
            "<div class=\"step\"><div class=\"row\"><span class=\"badge now\">1</span><span class=\"title\">Send traffic to the Mac</span></div>\
             <div class=\"body\"><p class=\"path\">{p}</p>\
             <div class=\"ios\"><div class=\"r\"><span>Server</span><span class=\"val\">{server}</span></div>\
             <div class=\"r\"><span>Port</span><span class=\"val\">{port}</span></div>\
             <div class=\"r\"><span>Authentication</span><span class=\"toggle off\"></span></div></div>\
             <p class=\"hint\">Tap <b>Save</b>, then come back here.</p></div></div>",
            p = path(&["Settings", "Wi-Fi", "ⓘ", "Configure Proxy", "Manual"]),
        ));
        if ca_name.is_some() {
            body.push_str("<div class=\"step later\"><div class=\"row\"><span class=\"badge later\">2</span><span class=\"title\">Trust the certificate</span></div></div>");
        }
        body.push_str("<p class=\"status\"><span class=\"pulse\"></span>Waiting for the proxy…</p>");
    } else if let Some(ca) = ca_name.as_ref().filter(|_| s.trust != Trust::Trusted) {
        body.push_str(&format!(
            "<div class=\"step\"><div class=\"row\">{ok_mark}<span class=\"title\">Traffic goes to the Mac<small>{server} : {port}</small></span></div></div>\
             <div class=\"step\"><div class=\"row\"><span class=\"badge now\">2</span><span class=\"title\">Trust the certificate<small>So the Mac can read HTTPS</small></span></div>\
             <div class=\"body\"><div class=\"sub\"><span class=\"n\">a</span><span class=\"t\">Download the profile.</span></div>\
             <a class=\"button\" href=\"/setup/{token}/ca.mobileconfig\">Install Certificate</a>\
             <div class=\"sub\"><span class=\"n\">b</span><span class=\"t\">{b}</span></div>\
             <div class=\"sub\"><span class=\"n\">c</span><span class=\"t\">{c1}<span class=\"arrow\">›</span><br>{c2}, turn this on:</span></div>\
             <div class=\"ios\"><div class=\"r\"><span>{ca}</span><span class=\"toggle\"></span></div></div>\
             <p class=\"hint\">Step c is easy to miss: installing the profile is not enough.</p></div></div>\
             <img class=\"check\" src=\"https://{CHECK_HOST}/{nonce}.gif\" alt=\"\">",
            token = html(&s.token),
            b = path(&["Settings", "Profile Downloaded", "Install"]),
            c1 = path(&["Settings", "General", "About"]),
            c2 = path(&["Certificate Trust Settings"]),
            nonce = s.nonce,
        ));
        body.push_str(if s.trust == Trust::Refused {
            "<p class=\"status\"><span class=\"pulse\"></span>The certificate is not trusted yet.</p>"
        } else {
            "<p class=\"status\"><span class=\"pulse\"></span>Checking the certificate…</p>"
        });
    } else {
        reload = false;
        let ca_row = match &ca_name {
            Some(_) => "<div class=\"r\"><span>Certificate</span><span>trusted ✓</span></div>",
            None => "<div class=\"r\"><span>HTTPS</span><span>passed on unread</span></div>",
        };
        body.push_str(&format!(
            "<div class=\"hero\"><div class=\"big\">✓</div><h3>This iPhone is connected</h3>\
             <p>Its requests appear on the Mac, in the Proxy tab.</p></div>\
             <div class=\"info\"><div class=\"r\"><span>Proxy</span><span>{server} : {port} ✓</span></div>{ca_row}</div>\
             <div class=\"info\"><div class=\"r\"><span><b>To stop</b></span><span>Wi-Fi › ⓘ › Configure Proxy › Off</span></div></div>\
             <details><summary>Good to know</summary>\
             <p>While the proxy is on, this iPhone has no internet on this Wi-Fi when the Mac sleeps or the proxy is off.</p>\
             <p>The Mac keeps this iPhone's requests in its log, cookies included.</p>\
             <p>Apps that pin their certificates (iCloud, banks) fail while the proxy reads their host.</p></details>"
        ));
    }
    if reload {
        body.push_str("<a class=\"button plain\" href=\"\">Check Again</a>");
    }
    if let Some(ip) = s.allowed {
        body.push_str(&format!("<p class=\"foot\">This iPhone ({ip}) is allowed.</p>"));
    }
    let refresh = if reload { "<meta http-equiv=\"refresh\" content=\"3\">" } else { "" };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{refresh}\
         <meta name=\"color-scheme\" content=\"light dark\"><title>{app}: connect this iPhone</title><style>{SETUP_CSS}</style></head>\
         <body><div class=\"top\"><span class=\"logo\"></span>{top}</div><h1>Connect this iPhone</h1>{body}</body></html>",
        top = html(&s.app_name.to_uppercase()),
    )
}

/// The look of the setup page: iOS grouped lists, light and dark.
const SETUP_CSS: &str = ":root{--bg:#f2f2f7;--card:#fff;--ink:#1c1c1e;--mute:#6e6e73;--line:#e5e5ea;--blue:#007aff;--green:#34c759;--field:#f2f2f7;--orange:#ff9500}\
@media (prefers-color-scheme:dark){:root{--bg:#000;--card:#1c1c1e;--ink:#f2f2f7;--mute:#98989f;--line:#38383a;--blue:#0a84ff;--green:#30d158;--field:#2c2c2e;--orange:#ff9f0a}}\
*{box-sizing:border-box}body{margin:0 auto;max-width:30em;padding:24px 16px;background:var(--bg);color:var(--ink);font:16px/1.4 -apple-system,sans-serif}\
.top{display:flex;align-items:center;gap:8px;color:var(--mute);font-size:13px;font-weight:600;letter-spacing:.02em}\
.logo{width:22px;height:22px;border-radius:6px;background:linear-gradient(135deg,#2e9e63,#1f6f8b)}\
h1{font-size:30px;line-height:1.1;margin:10px 0 18px}\
.step{background:var(--card);border-radius:14px;margin:0 0 10px;overflow:hidden}.row{display:flex;align-items:center;gap:12px;padding:13px 14px}\
.badge{width:26px;height:26px;border-radius:50%;flex:none;display:grid;place-items:center;font-size:14px;font-weight:700}\
.done{background:var(--green);color:#fff}.now{background:var(--blue);color:#fff}.badge.later{border:1.5px solid var(--line);color:var(--mute)}\
.title{font-weight:600;flex:1}.title small{display:block;font-weight:400;color:var(--mute);font-size:13px}.step.later .title{color:var(--mute)}\
.body{padding:0 14px 16px 52px}.path{font-size:15px;margin:0 0 12px}.arrow{color:var(--mute);margin:0 3px}\
.ios{background:var(--field);border-radius:10px;margin:0 0 12px}.ios .r{display:flex;align-items:center;justify-content:space-between;gap:10px;padding:11px 12px;border-bottom:1px solid var(--line)}\
.ios .r:last-child{border-bottom:0}.val{font:600 22px/1 ui-monospace,Menlo,monospace;user-select:all;-webkit-user-select:all}\
.toggle{width:46px;height:28px;border-radius:14px;background:var(--green);position:relative;flex:none}\
.toggle::after{content:\"\";position:absolute;right:2px;top:2px;width:24px;height:24px;border-radius:50%;background:#fff;box-shadow:0 1px 2px rgba(0,0,0,.25)}\
.toggle.off{background:var(--line)}.toggle.off::after{right:auto;left:2px}\
.button{display:block;text-align:center;background:var(--blue);color:#fff;font-weight:600;font-size:17px;padding:13px;border-radius:12px;text-decoration:none;margin:0 0 12px}\
.button.plain{background:var(--card);color:var(--blue);margin-top:16px}\
.sub{display:flex;gap:10px;margin:0 0 12px}.sub .n{width:20px;height:20px;border-radius:50%;background:var(--field);color:var(--mute);font-size:12px;font-weight:700;display:grid;place-items:center;flex:none;margin-top:1px}\
.sub .t{font-size:15px;flex:1;min-width:0}.hint{font-size:13px;color:var(--mute);margin:4px 0 0}\
.status{display:flex;align-items:center;gap:8px;justify-content:center;color:var(--mute);font-size:13px;margin:16px 0 0}\
.pulse{width:8px;height:8px;border-radius:50%;background:var(--orange);animation:p 1.6s ease-in-out infinite}@keyframes p{50%{opacity:.25}}\
@media (prefers-reduced-motion:reduce){.pulse{animation:none}}.check{width:1px;height:1px;position:absolute;opacity:0}\
.hero{background:var(--card);border-radius:14px;padding:24px 16px;text-align:center;margin:0 0 10px}\
.big{width:64px;height:64px;border-radius:50%;background:var(--green);color:#fff;display:grid;place-items:center;font-size:34px;font-weight:700;margin:0 auto 12px}\
.hero h3{margin:0 0 6px;font-size:21px}.hero p{margin:0;color:var(--mute);font-size:15px}\
.info{background:var(--card);border-radius:14px;padding:4px 14px;margin:0 0 10px}.info .r{display:flex;justify-content:space-between;gap:12px;padding:11px 0;border-bottom:1px solid var(--line);font-size:15px}\
.info .r:last-child{border-bottom:0}.info .r span:last-child{color:var(--mute);text-align:right}\
details{background:var(--card);border-radius:14px;padding:12px 14px;font-size:14px}summary{font-weight:600}details p{color:var(--mute);margin:8px 0 0}\
.foot{text-align:center;color:var(--mute);font-size:12px;margin-top:16px}";

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A stable UUID from a name, so the same profile gets the same UUID.
fn uuid_for(name: &str) -> String {
    let d = digest(&SHA256, name.as_bytes());
    let hex: String = d.as_ref()[..16].iter().map(|x| format!("{x:02X}")).collect();
    format!("{}-{}-5{}-{}-{}", &hex[0..8], &hex[8..12], &hex[13..16], &hex[16..20], &hex[20..32])
}

fn fingerprint(der: &[u8]) -> String {
    digest(&SHA256, der).as_ref()[..8].iter().map(|x| format!("{x:02x}")).collect()
}

/// The inspection CA as a configuration profile: one root payload, nothing
/// else (I10). It does not touch the Wi-Fi network, so iOS asks for no Wi-Fi
/// password. `None` without a CA.
pub fn ca_profile(s: &Setup) -> Option<String> {
    let (der, name) = s.ca.as_ref()?;
    let id = format!("{}.inspection-ca.{}", s.bundle_id, fingerprint(der));
    let payload_id = format!("{id}.root");
    Some(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\"><dict>\
         <key>PayloadType</key><string>Configuration</string>\
         <key>PayloadVersion</key><integer>1</integer>\
         <key>PayloadIdentifier</key><string>{id}</string>\
         <key>PayloadUUID</key><string>{uuid}</string>\
         <key>PayloadDisplayName</key><string>{name}</string>\
         <key>PayloadDescription</key><string>{description}</string>\
         <key>PayloadOrganization</key><string>{app}</string>\
         <key>PayloadRemovalDisallowed</key><false/>\
         <key>PayloadContent</key><array><dict>\
         <key>PayloadType</key><string>com.apple.security.root</string>\
         <key>PayloadVersion</key><integer>1</integer>\
         <key>PayloadIdentifier</key><string>{payload_id}</string>\
         <key>PayloadUUID</key><string>{payload_uuid}</string>\
         <key>PayloadDisplayName</key><string>{name}</string>\
         <key>PayloadCertificateFileName</key><string>inspection-ca.cer</string>\
         <key>PayloadContent</key><data>{data}</data></dict></array>\
         </dict></plist>\n",
        id = xml(&id),
        uuid = uuid_for(&id),
        name = xml(name),
        description = xml(&format!("The CA {} uses to read HTTPS through its proxy. Remove this profile to stop.", s.app_name)),
        app = xml(&s.app_name),
        payload_id = xml(&payload_id),
        payload_uuid = uuid_for(&payload_id),
        data = base64(der),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Setup {
        Setup {
            app_name: "LocalRouter-dev".into(),
            bundle_id: "dev.localrouter.app-dev".into(),
            token: "k7mq-2xph-9tdw-r4nc".into(),
            server: "192.168.0.10".into(),
            port: 7878,
            ca: Some((vec![0x30, 0x82, 0x01, 0x0a, 0xff], "LocalRouter-dev Inspection CA 1234".into())),
            allowed: Some("192.168.0.23".parse().unwrap()),
            via_proxy: true,
            trust: Trust::Unknown,
            nonce: 7,
        }
    }

    /// One field of a profile, read by macOS's own plist parser:
    /// `PayloadContent.0.PayloadType`. `None` when the key is absent.
    fn field(text: &str, key: &str) -> Option<String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.mobileconfig");
        std::fs::write(&path, text).unwrap();
        let lint = std::process::Command::new("/usr/bin/plutil").arg("-lint").arg(&path).output().unwrap();
        assert!(lint.status.success(), "plutil: {}", String::from_utf8_lossy(&lint.stdout));
        let out = std::process::Command::new("/usr/bin/plutil").args(["-extract", key, "raw", "-o", "-"]).arg(&path).output().unwrap();
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    #[test]
    fn token_format() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let t = new_token();
            assert_eq!(t.len(), 19, "{t}");
            let groups: Vec<&str> = t.split('-').collect();
            assert_eq!(groups.len(), 4);
            assert!(groups.iter().all(|g| g.len() == 4 && g.bytes().all(|b| ALPHABET.contains(&b))), "{t}");
            assert!(seen.insert(t));
        }
        for look_alike in *b"0o1l" {
            assert!(!ALPHABET.contains(&look_alike));
        }
    }

    #[test]
    fn same_token_compares_everything() {
        assert!(same_token(b"k7mq-2xph", b"k7mq-2xph"));
        assert!(!same_token(b"k7mq-2xph", b"k7mq-2xpj"));
        assert!(!same_token(b"k7mq-2xph", b"k7mq-2xp"));
        assert!(!same_token(b"k7mq-2xph", b"k7mq-2xphh"));
        assert!(!same_token(b"", b"a"));
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
    }

    #[test]
    fn devices_and_token() {
        let c = LanClient::new("iphone", "k7mq", vec![]);
        let phone: IpAddr = "192.168.0.23".parse().unwrap();
        assert!(!c.allows(phone));
        assert!(c.allow(phone));
        assert!(!c.allow("::ffff:192.168.0.23".parse().unwrap()), "the same device as IPv4-mapped IPv6");
        assert!(c.allows("::ffff:192.168.0.23".parse().unwrap()));
        assert!(!c.allows("192.168.0.24".parse().unwrap()));
        c.forget(phone);
        assert!(!c.allows(phone));
        assert!(c.is_token("k7mq") && !c.is_token("k7mr"));
        c.set_token("new");
        assert!(c.is_token("new") && !c.is_token("k7mq"));
    }

    #[test]
    fn local_targets() {
        let own: Vec<IpAddr> = vec!["192.168.0.10".parse().unwrap(), "fe80::1".parse().unwrap()];
        for local in ["127.0.0.1", "127.5.5.5", "::1", "::ffff:127.0.0.1", "0.0.0.0", "::", "192.168.0.10", "::ffff:192.168.0.10", "fe80::1"] {
            assert!(is_local_target(local.parse().unwrap(), &own), "{local}");
        }
        for remote in ["192.168.0.5", "1.1.1.1", "2606:4700::1111"] {
            assert!(!is_local_target(remote.parse().unwrap(), &own), "{remote}");
        }
    }

    #[test]
    fn interface_addresses_include_loopback() {
        assert!(interface_addresses().iter().any(|(_, ip)| ip.is_loopback()));
        assert_eq!(ipv4_of("lo0"), Some(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn setup_paths() {
        assert_eq!(parse_setup_path("/setup/abc"), Some(("abc", SetupFile::Page)));
        assert_eq!(parse_setup_path("/setup/abc/"), Some(("abc", SetupFile::Page)));
        assert_eq!(parse_setup_path("/setup/abc?x=1"), Some(("abc", SetupFile::Page)));
        assert_eq!(parse_setup_path("/setup/abc/ca.mobileconfig"), Some(("abc", SetupFile::CaProfile)));
        assert_eq!(parse_setup_path("/setup/abc/phone.mobileconfig"), None);
        assert_eq!(parse_setup_path("/setup/"), None);
        assert_eq!(parse_setup_path("/other"), None);
    }

    // Each state shows only its step; the last one stops reloading.
    #[test]
    fn setup_page_shows_only_the_next_step() {
        let s = setup();
        let direct = setup_page(&Setup { via_proxy: false, ..s.clone() });
        assert!(direct.contains("<span class=\"val\">192.168.0.10</span>") && direct.contains("<span class=\"val\">7878</span>"));
        assert!(direct.contains("Waiting for the proxy") && direct.contains("http-equiv=\"refresh\""));
        assert!(!direct.contains("ca.mobileconfig") && !direct.contains(CHECK_HOST));

        let untrusted = setup_page(&s);
        assert!(untrusted.contains("Traffic goes to the Mac") && !untrusted.contains("class=\"val\""));
        assert!(untrusted.contains("href=\"/setup/k7mq-2xph-9tdw-r4nc/ca.mobileconfig\""));
        assert!(untrusted.contains("Certificate Trust Settings") && untrusted.contains("LocalRouter-dev Inspection CA 1234"));
        assert!(untrusted.contains(&format!("src=\"https://{CHECK_HOST}/7.gif\"")), "the check image");
        assert!(untrusted.contains("Checking the certificate") && untrusted.contains("Check Again"));
        let refused = setup_page(&Setup { trust: Trust::Refused, ..s.clone() });
        assert!(refused.contains("not trusted yet"));

        let done = setup_page(&Setup { trust: Trust::Trusted, ..s.clone() });
        assert!(done.contains("This iPhone is connected") && done.contains("Configure Proxy › Off"));
        assert!(!done.contains("http-equiv=\"refresh\"") && !done.contains(CHECK_HOST) && !done.contains("Check Again"));
        // Without inspection nothing needs trust: connected at once.
        let plain = setup_page(&Setup { ca: None, ..s.clone() });
        assert!(plain.contains("This iPhone is connected") && plain.contains("passed on unread"));

        for page in [&direct, &untrusted, &done] {
            assert!(page.contains("This iPhone (192.168.0.23) is allowed."));
            assert!(!page.contains("<script"));
        }
        let escaped = setup_page(&Setup { allowed: None, via_proxy: false, server: "<b>".into(), ..s });
        assert!(escaped.contains("&lt;b&gt;") && !escaped.contains("is allowed"));
    }

    #[test]
    fn trust_per_device() {
        let c = LanClient::new("iphone", "k7mq", vec![]);
        let phone: IpAddr = "192.168.0.23".parse().unwrap();
        assert_eq!(c.trust(phone), Trust::Unknown);
        c.set_trust("::ffff:192.168.0.23".parse().unwrap(), Trust::Refused);
        assert_eq!(c.trust(phone), Trust::Refused);
        c.set_trust(phone, Trust::Trusted);
        assert_eq!(c.trust(phone), Trust::Trusted);
        assert_eq!(c.trust("192.168.0.24".parse().unwrap()), Trust::Unknown);
    }

    #[test]
    fn ca_profile_holds_only_the_ca() {
        let s = setup();
        let p = ca_profile(&s).unwrap();
        assert_eq!(field(&p, "PayloadType").as_deref(), Some("Configuration"));
        assert_eq!(field(&p, "PayloadContent").as_deref(), Some("1"));
        assert_eq!(field(&p, "PayloadContent.0.PayloadType").as_deref(), Some("com.apple.security.root"));
        assert_eq!(field(&p, "PayloadContent.0.PayloadContent"), Some(base64(&[0x30, 0x82, 0x01, 0x0a, 0xff])));
        assert!(field(&p, "PayloadIdentifier").unwrap().starts_with("dev.localrouter.app-dev.inspection-ca."));
        assert!(!p.contains("wifi") && !p.contains("PRIVATE KEY"));
        assert!(ca_profile(&Setup { ca: None, ..s }).is_none());
    }
}
