//! The text `nettop` prints in logging mode (ADR 11, change 1):
//!
//! ```text
//! nettop -L 0 -s 5 -x -n -d -J bytes_in,bytes_out
//! ```
//!
//! ```text
//! ,bytes_in,bytes_out,
//! apsd.401,732854,2812087,
//! tcp4 192.0.2.155:57853<->203.0.113.10:5223,732854,2812087,
//! localrouterd-de.913,7915,45252,
//! tcp6 ::1.80<->::1.49368,699,620,
//! tcp4 *:443<->*:*,,,
//! ```
//!
//! A sample starts with the header line. A process line is `<name>.<pid>`,
//! and the name is cut to about 15 characters, so it is only a label. A
//! connection line follows its process: IPv4 writes `addr:port`, IPv6
//! `addr.port`, and a scope can follow an IPv6 address (`fe80::1%en0.55893`).
//! nettop is a tool, not an API: a sample of which no line parses is an
//! error, so a changed format is reported instead of showing no traffic.

use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

/// The first line of every sample.
pub const HEADER: &str = ",bytes_in,bytes_out,";

/// The arguments the sampler runs `/usr/bin/nettop` with.
pub fn args(interval_secs: u64) -> Vec<String> {
    ["-L", "0", "-s", &interval_secs.to_string(), "-x", "-n", "-d", "-J", "bytes_in,bytes_out"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Proto {
    Tcp,
    Udp,
}

/// One socket of a process, with the bytes of this sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conn {
    pub proto: Proto,
    /// `None` when nettop writes `*` for the address (any address); the port
    /// is still known then.
    pub local: Endpoint,
    /// `None` for a socket with no peer: an unconnected UDP socket.
    pub remote: Option<SocketAddr>,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// One side of a socket: an address, or `*`, and a port, or `*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub ip: Option<IpAddr>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    /// The name nettop prints, cut to about 15 characters.
    pub name: String,
    pub pid: u32,
    pub conns: Vec<Conn>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sample {
    pub procs: Vec<Proc>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("nettop output not understood: the sample does not start with {HEADER:?} but with {0:?}")]
    Header(String),
    #[error("nettop output not understood: none of the {0} lines of the sample could be read, first: {1:?}")]
    NoLineRead(usize, String),
}

/// Parse one sample: the header line and the lines after it, up to the next
/// header. Lines that cannot be read are skipped, unless no line can be read.
pub fn parse_sample(text: &str) -> Result<Sample, ParseError> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(first) = lines.next() else { return Ok(Sample::default()) };
    if first.trim() != HEADER {
        return Err(ParseError::Header(first.chars().take(80).collect()));
    }
    let mut sample = Sample::default();
    let mut total = 0;
    let mut read = 0;
    let mut first_bad: Option<String> = None;
    for line in lines {
        total += 1;
        match parse_line(line) {
            Some(Line::Proc(name, pid)) => {
                read += 1;
                sample.procs.push(Proc { name, pid, conns: vec![] });
            }
            Some(Line::Conn(conn)) => {
                read += 1;
                // A listening socket: no peer and empty counters.
                let Some(conn) = conn else { continue };
                if let Some(p) = sample.procs.last_mut() {
                    p.conns.push(conn);
                }
            }
            None => {
                first_bad.get_or_insert_with(|| line.chars().take(80).collect());
            }
        }
    }
    if total > 0 && read == 0 {
        return Err(ParseError::NoLineRead(total, first_bad.unwrap_or_default()));
    }
    Ok(sample)
}

/// Splits the endless output of `nettop -L 0` into samples, line by line.
///
/// A sample ends only where the next one starts. nettop writes to a pipe
/// with full buffering, so a pause in the output is not the end of a
/// sample: its last lines often come with the next write.
#[derive(Debug, Default)]
pub struct Splitter {
    current: String,
    seen_header: bool,
}

impl Splitter {
    /// Feed one line. Returns the previous sample when this line starts a
    /// new one.
    pub fn push(&mut self, line: &str) -> Option<String> {
        let line = line.trim_end_matches(['\n', '\r']);
        let mut done = None;
        if line.trim() == HEADER {
            if self.seen_header {
                done = Some(std::mem::take(&mut self.current));
            }
            self.current.clear();
            self.seen_header = true;
        }
        if !line.trim().is_empty() {
            self.current.push_str(line);
            self.current.push('\n');
        }
        done
    }

    /// Text that came before any header line: nettop's format changed. The
    /// caller parses it to get the error.
    pub fn take_headerless(&mut self) -> Option<String> {
        (!self.seen_header && !self.current.is_empty()).then(|| std::mem::take(&mut self.current))
    }

    /// The sample read so far, at the end of the output.
    pub fn finish(&mut self) -> Option<String> {
        if !self.seen_header || self.current.is_empty() {
            return None;
        }
        self.seen_header = false;
        Some(std::mem::take(&mut self.current))
    }
}

enum Line {
    Proc(String, u32),
    /// `None`: a listening socket, which is skipped.
    Conn(Option<Conn>),
}

fn parse_line(line: &str) -> Option<Line> {
    // `<first>,<bytes_in>,<bytes_out>,`: the first field has no comma.
    let line = line.trim_end();
    let mut fields = line.strip_suffix(',').unwrap_or(line).rsplitn(3, ',');
    let bytes_out = fields.next()?;
    let bytes_in = fields.next()?;
    let first = fields.next()?;
    let counter = |s: &str| if s.is_empty() { Some(None) } else { s.parse::<u64>().ok().map(Some) };
    let (bytes_in, bytes_out) = (counter(bytes_in)?, counter(bytes_out)?);
    if let Some((proto, rest)) = first.split_once(' ')
        && let Some(proto) = parse_proto(proto)
    {
        let (local, remote) = rest.split_once("<->")?;
        let v6 = proto.1;
        let local = parse_endpoint(local, v6)?;
        let remote = parse_endpoint(remote, v6)?;
        let remote = match (remote.ip, remote.port) {
            (Some(ip), Some(port)) => Some(SocketAddr::new(ip, port)),
            _ => None,
        };
        if remote.is_none() && bytes_in.is_none() && bytes_out.is_none() {
            return Some(Line::Conn(None));
        }
        return Some(Line::Conn(Some(Conn {
            proto: proto.0,
            local,
            remote,
            bytes_in: bytes_in.unwrap_or(0),
            bytes_out: bytes_out.unwrap_or(0),
        })));
    }
    let (name, pid) = first.rsplit_once('.')?;
    let pid = pid.parse::<u32>().ok()?;
    if name.is_empty() {
        return None;
    }
    Some(Line::Proc(name.to_string(), pid))
}

/// `tcp4` → (Tcp, false); `udp6` → (Udp, true).
fn parse_proto(s: &str) -> Option<(Proto, bool)> {
    match s {
        "tcp4" => Some((Proto::Tcp, false)),
        "tcp6" => Some((Proto::Tcp, true)),
        "udp4" => Some((Proto::Udp, false)),
        "udp6" => Some((Proto::Udp, true)),
        _ => None,
    }
}

/// `192.0.2.1:443`, `*:443`, `*:*` for IPv4; `::1.443`, `fe80::1%en0.55893`,
/// `*.443`, `*.*` for IPv6.
fn parse_endpoint(s: &str, v6: bool) -> Option<Endpoint> {
    let sep = if v6 { '.' } else { ':' };
    let (addr, port) = s.rsplit_once(sep)?;
    let port = if port == "*" { None } else { Some(port.parse::<u16>().ok()?) };
    let ip = if addr == "*" {
        None
    } else {
        // The scope (`%en0`) names the interface; the address is enough here.
        let addr = addr.split('%').next()?;
        let ip: IpAddr = addr.parse().ok()?;
        if ip.is_ipv6() != v6 {
            return None;
        }
        Some(ip)
    };
    Some(Endpoint { ip, port })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("testdata/nettop-two-samples.csv");

    fn samples() -> Vec<String> {
        let mut s = Splitter::default();
        let mut out: Vec<String> = FIXTURE.lines().filter_map(|l| s.push(l)).collect();
        out.extend(s.finish());
        out
    }

    #[test]
    fn two_samples_split_at_the_header_line() {
        let parts = samples();
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.starts_with(HEADER)));
        let first = parse_sample(&parts[0]).unwrap();
        let second = parse_sample(&parts[1]).unwrap();
        assert_eq!(first.procs.len(), 6);
        assert_eq!(second.procs.len(), 5);
        // The first sample is cumulative, the second has only the change.
        let apsd = |s: &Sample| s.procs.iter().find(|p| p.pid == 401).unwrap().conns[0].bytes_in;
        assert_eq!((apsd(&first), apsd(&second)), (732854, 120));
    }

    #[test]
    fn ipv4_ipv6_and_scoped_addresses() {
        let first = parse_sample(&samples()[0]).unwrap();
        let apsd = first.procs.iter().find(|p| p.name == "apsd").unwrap();
        assert_eq!(apsd.conns[0].remote, Some("203.0.113.10:5223".parse().unwrap()));
        assert_eq!(apsd.conns[0].local.port, Some(57853));
        let usb = first.procs.iter().find(|p| p.name == "usbmuxd").unwrap();
        assert_eq!(usb.conns[0].remote, Some("[fe80::10f5:1a0:b89a:4135]:60445".parse().unwrap()));
        assert_eq!(usb.conns[0].local.port, Some(55893));
        let chrome = first.procs.iter().find(|p| p.pid == 5120).unwrap();
        assert_eq!(chrome.name, "Google Chrome H");
        assert_eq!(chrome.conns[0].remote, Some("[::1]:443".parse().unwrap()));
        assert_eq!(chrome.conns[0].local.port, Some(61001));
        assert_eq!(chrome.conns[2].remote, Some("[2001:db8:1::443]:443".parse().unwrap()));
        assert_eq!((chrome.conns[1].bytes_in, chrome.conns[1].bytes_out), (96000, 9100));
    }

    #[test]
    fn listening_sockets_are_skipped_and_unconnected_udp_is_kept() {
        let first = parse_sample(&samples()[0]).unwrap();
        let launchd = first.procs.iter().find(|p| p.pid == 1).unwrap();
        assert!(launchd.conns.is_empty());
        let dns = first.procs.iter().find(|p| p.name == "mDNSResponder").unwrap();
        // `udp4 *:5353<->*:*` with counters stays; `udp6 *.5353<->*.*,,,` goes.
        assert_eq!(dns.conns.len(), 2);
        assert_eq!(dns.conns[1].remote, None);
        assert_eq!(dns.conns[1].local, Endpoint { ip: None, port: Some(5353) });
        assert_eq!(dns.conns[1].bytes_in, 82149271);
    }

    #[test]
    fn a_cut_name_keeps_its_pid() {
        let first = parse_sample(&samples()[0]).unwrap();
        let d = first.procs.iter().find(|p| p.pid == 913).unwrap();
        assert_eq!(d.name, "localrouterd-de");
        // A name with dots: the pid is after the last one.
        let s = parse_sample(&format!("{HEADER}\ncom.apple.WebKi.77,1,2,\n")).unwrap();
        assert_eq!((s.procs[0].name.as_str(), s.procs[0].pid), ("com.apple.WebKi", 77));
    }

    #[test]
    fn a_sample_no_line_of_which_parses_is_an_error() {
        let e = parse_sample(&format!("{HEADER}\nsomething new|1|2\nand more|3|4\n")).unwrap_err();
        assert!(e.to_string().starts_with("nettop output not understood"), "{e}");
        let e = parse_sample("time,bytes_in,bytes_out,rx_dupe\nx.1,1,2,3\n").unwrap_err();
        assert!(matches!(e, ParseError::Header(_)));
        // One bad line among good ones is skipped.
        let s = parse_sample(&format!("{HEADER}\na.1,1,2,\nnot a line\ntcp4 192.0.2.1:1<->192.0.2.2:2,3,4,\n")).unwrap();
        assert_eq!(s.procs[0].conns.len(), 1);
        // An empty sample is not an error: nothing used the network.
        assert_eq!(parse_sample(&format!("{HEADER}\n")).unwrap(), Sample::default());
    }

    #[test]
    fn a_sample_cut_by_a_pause_is_joined_with_its_tail() {
        let mut s = Splitter::default();
        assert_eq!(s.push(HEADER), None);
        assert_eq!(s.push("a.1,1,2,"), None);
        // A pause here is not the end: the tail comes with the next write.
        assert_eq!(s.take_headerless(), None);
        assert_eq!(s.push("tcp4 192.0.2.1:1<->192.0.2.2:2,3,4,"), None);
        let done = s.push(HEADER).unwrap();
        assert_eq!(parse_sample(&done).unwrap().procs[0].conns.len(), 1);
        // Text with no header at all is handed back to be reported.
        let mut s = Splitter::default();
        s.push("time;bytes");
        assert!(parse_sample(&s.take_headerless().unwrap()).is_err());
    }

    #[test]
    fn args_match_the_adr() {
        assert_eq!(args(5).join(" "), "-L 0 -s 5 -x -n -d -J bytes_in,bytes_out");
    }
}
