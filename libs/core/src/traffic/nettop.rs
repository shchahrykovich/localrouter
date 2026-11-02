//! The text `nettop` prints in logging mode (ADR 11, change 1):
//!
//! ```text
//! nettop -L 1 -x -n -J bytes_in,bytes_out
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
//!
//! `-L 1` prints one sample and exits. The counters are totals since each
//! socket opened, and [`Totals`] turns them into the bytes since the
//! previous run. A long-running `nettop -L 0 -s 5 -d` would print the change
//! itself, but on macOS 27 it uses about 1.5 CPU cores all the time, at any
//! interval; one `-L 1` run uses about 0.02 s. The cost: a socket that
//! opens and closes between two runs is not seen, and the last bytes of a
//! socket that closes are lost.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

/// The first line of every sample.
pub const HEADER: &str = ",bytes_in,bytes_out,";

/// The arguments the sampler runs `/usr/bin/nettop` with: one sample of
/// totals, then exit.
pub fn args() -> Vec<String> {
    ["-L", "1", "-x", "-n", "-J", "bytes_in,bytes_out"].iter().map(|s| s.to_string()).collect()
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

/// A socket, as far as nettop shows it: the process, the protocol and both
/// ends.
type ConnKey = (u32, Proto, Option<IpAddr>, Option<u16>, Option<SocketAddr>);

fn conn_key(pid: u32, c: &Conn) -> ConnKey {
    (pid, c.proto, c.local.ip, c.local.port, c.remote)
}

/// Turns the totals of `nettop -L 1` runs into the bytes since the
/// previous run.
#[derive(Debug, Default)]
pub struct Totals {
    last: Option<HashMap<ConnKey, (u64, u64)>>,
}

impl Totals {
    /// The bytes of each socket since the previous run, as a sample of the
    /// same shape. `None` for the first run, which is only the starting
    /// point. A socket first seen now counts all its bytes: it opened since
    /// the previous run. A socket whose counter went down is a new socket
    /// with the same ends and counts all its bytes too. Sockets with the
    /// same key (two unconnected UDP sockets on one port) are added up.
    /// Sockets and processes with no new bytes are left out.
    pub fn next(&mut self, sample: &Sample) -> Option<Sample> {
        let mut now: HashMap<ConnKey, (u64, u64)> = HashMap::new();
        for p in &sample.procs {
            for c in &p.conns {
                let total = now.entry(conn_key(p.pid, c)).or_default();
                total.0 += c.bytes_in;
                total.1 += c.bytes_out;
            }
        }
        let last = self.last.replace(now)?;
        let now = self.last.as_ref().expect("just set");
        let mut out = Sample::default();
        let mut done = std::collections::HashSet::new();
        for p in &sample.procs {
            let mut conns = vec![];
            for c in &p.conns {
                let key = conn_key(p.pid, c);
                if !done.insert(key) {
                    continue;
                }
                let (cur_in, cur_out) = now[&key];
                let (bytes_in, bytes_out) = match last.get(&key) {
                    Some(&(in0, out0)) if cur_in >= in0 && cur_out >= out0 => (cur_in - in0, cur_out - out0),
                    _ => (cur_in, cur_out),
                };
                if bytes_in > 0 || bytes_out > 0 {
                    conns.push(Conn { bytes_in, bytes_out, ..c.clone() });
                }
            }
            if !conns.is_empty() {
                out.procs.push(Proc { name: p.name.clone(), pid: p.pid, conns });
            }
        }
        Some(out)
    }
}

/// The samples of a text with several, split at each header line: the
/// fixture holds two.
#[cfg(test)]
pub(crate) fn split_samples(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if line.trim() == HEADER {
            out.push(String::new());
        }
        if let Some(cur) = out.last_mut() {
            cur.push_str(line);
            cur.push('\n');
        }
    }
    out
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
        split_samples(FIXTURE)
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
    fn args_ask_for_one_sample_of_totals() {
        // `-L 0` (run until killed) uses about 1.5 CPU cores on macOS 27.
        assert_eq!(args().join(" "), "-L 1 -x -n -J bytes_in,bytes_out");
    }

    fn totals(lines: &str) -> Sample {
        parse_sample(&format!("{HEADER}\n{lines}")).unwrap()
    }

    fn bytes(s: &Sample, pid: u32, port: u16) -> Option<(u64, u64)> {
        let p = s.procs.iter().find(|p| p.pid == pid)?;
        p.conns.iter().find(|c| c.local.port == Some(port)).map(|c| (c.bytes_in, c.bytes_out))
    }

    #[test]
    fn the_first_run_is_only_the_starting_point() {
        let mut t = Totals::default();
        assert_eq!(t.next(&totals("a.1,10,20,\ntcp4 192.0.2.1:1000<->192.0.2.2:443,10,20,\n")), None);
    }

    #[test]
    fn later_runs_give_the_bytes_since_the_previous_run() {
        let mut t = Totals::default();
        t.next(&totals("a.1,0,0,\ntcp4 192.0.2.1:1000<->192.0.2.2:443,100,20,\ntcp4 192.0.2.1:1001<->192.0.2.2:443,5,5,\n"));
        let d = t
            .next(&totals(concat!(
                "a.1,0,0,\n",
                // grew
                "tcp4 192.0.2.1:1000<->192.0.2.2:443,150,25,\n",
                // 1001 closed: nothing
                // opened since the previous run: all its bytes
                "tcp4 192.0.2.1:1002<->192.0.2.2:443,7,3,\n",
                "b.2,0,0,\n",
                // the same ends in another process: its own socket
                "tcp4 192.0.2.1:1000<->192.0.2.2:443,1,1,\n",
            )))
            .unwrap();
        assert_eq!(bytes(&d, 1, 1000), Some((50, 5)));
        assert_eq!(bytes(&d, 1, 1001), None);
        assert_eq!(bytes(&d, 1, 1002), Some((7, 3)));
        assert_eq!(bytes(&d, 2, 1000), Some((1, 1)));
        // The next run is measured from this one.
        let d = t.next(&totals("a.1,0,0,\ntcp4 192.0.2.1:1000<->192.0.2.2:443,160,25,\n")).unwrap();
        assert_eq!(bytes(&d, 1, 1000), Some((10, 0)));
    }

    #[test]
    fn a_counter_that_went_down_is_a_new_socket() {
        let mut t = Totals::default();
        t.next(&totals("a.1,0,0,\ntcp4 192.0.2.1:1000<->192.0.2.2:443,100,100,\n"));
        let d = t.next(&totals("a.1,0,0,\ntcp4 192.0.2.1:1000<->192.0.2.2:443,30,200,\n")).unwrap();
        assert_eq!(bytes(&d, 1, 1000), Some((30, 200)));
    }

    #[test]
    fn idle_sockets_and_processes_are_left_out() {
        let mut t = Totals::default();
        let s = totals("a.1,0,0,\ntcp4 192.0.2.1:1000<->192.0.2.2:443,100,100,\nb.2,0,0,\ntcp4 192.0.2.1:2000<->192.0.2.2:443,1,1,\n");
        t.next(&s);
        let d = t.next(&s).unwrap();
        assert_eq!(d, Sample::default());
    }

    #[test]
    fn sockets_with_the_same_ends_are_added_up() {
        let mut t = Totals::default();
        let two = |a: u64, b: u64| totals(&format!("dns.5,0,0,\nudp4 *:5353<->*:*,{a},0,\nudp4 *:5353<->*:*,{b},0,\n"));
        t.next(&two(10, 20));
        let d = t.next(&two(15, 30)).unwrap();
        assert_eq!(d.procs[0].conns.len(), 1);
        assert_eq!(bytes(&d, 5, 5353), Some((15, 0)));
    }
}
