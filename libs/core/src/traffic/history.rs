//! Traffic history (ADR 11, 1.6). The daemon is its only writer.
//!
//! | Store | Shape |
//! |---|---|
//! | memory | the last 60 minutes, one entry per minute per active pair, and the open minute, which each 5-second sample adds to |
//! | `<data>/traffic/YYYY-MM-DD.ndjson` | one line per minute per active pair (app, executable, `via`, remote, port, protocol, host), appended when the minute ends |
//!
//! A crash loses at most the open minute. A half-written last line is
//! skipped when read. Day files are named by the UTC date. Files are kept
//! `traffic_history_days` days and at most 500 MB together, the oldest
//! deleted first (I4). The folder is `0700`, the files `0600`: they list
//! every address every app talked to.
//!
//! A range longer than an hour reads the day files once and keeps the
//! result until the next minute is written (gap G5).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Via;
use super::attribute::{AppBytes, AppKind};
use super::nettop::Proto;
use crate::api::{AppTraffic, AppTrafficResult, ByVia, InOut, RemoteTraffic, SamplerState, TrafficBucket, TrafficRange, TrafficTotals};

/// The day files together stay under this.
pub const MAX_BYTES: u64 = 500 * 1024 * 1024;
/// At most this many remotes in a reply.
pub const MAX_REMOTES: usize = 2000;
/// A process counts as running when it was in a sample this recently.
const PID_FRESH_SECS: u64 = 30;
/// Different pairs kept in one minute: a flood of new connections cannot
/// grow memory without end; extra pairs are counted under no remote.
const MAX_PAIRS_PER_MINUTE: usize = 20_000;

/// One line of a day file: one pair's bytes in one minute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// The minute's start, Unix seconds.
    pub t: u64,
    pub app: String,
    pub name: String,
    pub kind: AppKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    pub via: Via,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub proto: Proto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(rename = "in")]
    pub bytes_in: u64,
    #[serde(rename = "out")]
    pub bytes_out: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct PairKey {
    app: String,
    exe: Option<String>,
    via: Via,
    remote: Option<String>,
    port: Option<u16>,
    proto: Proto,
    host: Option<String>,
}

#[derive(Debug, Clone)]
struct Minute {
    t: u64,
    pairs: HashMap<PairKey, InOut>,
    names: HashMap<String, (String, AppKind)>,
}

impl Minute {
    fn new(t: u64) -> Self {
        Self { t, pairs: HashMap::new(), names: HashMap::new() }
    }

    fn lines(&self) -> Vec<Line> {
        let mut lines: Vec<Line> = self
            .pairs
            .iter()
            .map(|(k, v)| {
                let (name, kind) = self.names.get(&k.app).cloned().unwrap_or_else(|| (k.app.clone(), AppKind::Program));
                Line {
                    t: self.t,
                    app: k.app.clone(),
                    name,
                    kind,
                    exe: k.exe.clone(),
                    via: k.via,
                    remote: k.remote.clone(),
                    port: k.port,
                    proto: k.proto,
                    host: k.host.clone(),
                    bytes_in: v.bytes_in,
                    bytes_out: v.bytes_out,
                }
            })
            .collect();
        lines.sort_by(|a, b| (&a.app, &a.exe, a.via, &a.remote, a.port).cmp(&(&b.app, &b.exe, b.via, &b.remote, b.port)));
        lines
    }

    fn add_line(&mut self, l: &Line) {
        self.names.insert(l.app.clone(), (l.name.clone(), l.kind));
        let key = PairKey {
            app: l.app.clone(),
            exe: l.exe.clone(),
            via: l.via,
            remote: l.remote.clone(),
            port: l.port,
            proto: l.proto,
            host: l.host.clone(),
        };
        self.pairs.entry(key).or_default().add(l.bytes_in, l.bytes_out);
    }
}

/// The sums a reply is made from, for one range, before the app filter.
#[derive(Debug, Clone, Default)]
struct Agg {
    start: u64,
    bucket_secs: u64,
    n: usize,
    apps: BTreeMap<String, AppAgg>,
    remotes: HashMap<RemoteKey, InOut>,
}

/// app, executable, remote, port, protocol, via, host.
type RemoteKey = (String, Option<String>, Option<String>, Option<u16>, Proto, Via, Option<String>);

#[derive(Debug, Clone, Default)]
struct AppAgg {
    name: String,
    kind: Option<AppKind>,
    total: InOut,
    by_via: ByVia,
    exes: BTreeSet<String>,
    remotes: HashSet<String>,
    buckets: Vec<InOut>,
    spark: [u64; 12],
}

impl Agg {
    fn new(range: TrafficRange, now: u64) -> Self {
        let n = range.buckets();
        let bucket_secs = range.secs() / n;
        let end = now.div_ceil(bucket_secs).max(1) * bucket_secs;
        // `now` exactly on a bucket edge still has a bucket for it.
        let end = if end == now { end + bucket_secs } else { end };
        Self { start: end - n * bucket_secs, bucket_secs, n: n as usize, ..Default::default() }
    }

    fn end(&self) -> u64 {
        self.start + self.bucket_secs * self.n as u64
    }

    fn add(&mut self, l: &Line) {
        if l.t < self.start || l.t >= self.end() {
            return;
        }
        let i = ((l.t - self.start) / self.bucket_secs) as usize;
        let spark = ((l.t - self.start) * 12 / (self.bucket_secs * self.n as u64)) as usize;
        let n = self.n;
        let a = self.apps.entry(l.app.clone()).or_insert_with(|| AppAgg { buckets: vec![InOut::default(); n], ..Default::default() });
        a.name.clone_from(&l.name);
        a.kind = Some(l.kind);
        a.total.add(l.bytes_in, l.bytes_out);
        a.by_via.get_mut(l.via).add(l.bytes_in, l.bytes_out);
        if let Some(exe) = &l.exe
            && !a.exes.contains(exe)
        {
            a.exes.insert(exe.clone());
        }
        if let Some(r) = &l.remote
            && !a.remotes.contains(r)
        {
            a.remotes.insert(r.clone());
        }
        a.buckets[i].add(l.bytes_in, l.bytes_out);
        a.spark[spark.min(11)] += l.bytes_in + l.bytes_out;
        self.remotes
            .entry((l.app.clone(), l.exe.clone(), l.remote.clone(), l.port, l.proto, l.via, l.host.clone()))
            .or_default()
            .add(l.bytes_in, l.bytes_out);
    }
}

/// The history: memory, day files, and the cache of long ranges.
pub struct History {
    dir: PathBuf,
    /// Closed minutes of the last hour, oldest first.
    minutes: VecDeque<Minute>,
    open: Option<Minute>,
    /// Pids per app, with the time each was last in a sample.
    pids: HashMap<String, BTreeMap<u32, u64>>,
    /// Counts written minutes: a cached long range is good while it holds.
    generation: u64,
    cache: HashMap<TrafficRange, (u64, u64, Agg)>,
    /// Day files opened for reading, for the test of the cache.
    pub files_read: u64,
}

impl History {
    /// The history in `dir`. The last hour comes back from the day files at
    /// minute grain. Nothing is created until the first minute is written.
    pub fn open(dir: PathBuf, now: u64) -> Self {
        let mut h = Self {
            dir,
            minutes: VecDeque::new(),
            open: None,
            pids: HashMap::new(),
            generation: 0,
            cache: HashMap::new(),
            files_read: 0,
        };
        let since = now.saturating_sub(3600);
        let mut by_t: BTreeMap<u64, Minute> = BTreeMap::new();
        for l in h.read_lines(since) {
            if l.t >= since && l.t < minute_of(now) {
                by_t.entry(l.t).or_insert_with(|| Minute::new(l.t)).add_line(&l);
            }
        }
        h.minutes = by_t.into_values().collect();
        h
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Add one sample's rows, taken at `now`. Closes the open minute first
    /// when `now` is in a later one, and writes it to its day file.
    pub fn add(&mut self, now: u64, rows: &[AppBytes]) -> io::Result<()> {
        let flushed = self.roll(now);
        let t = minute_of(now);
        let open = self.open.get_or_insert_with(|| Minute::new(t));
        for r in rows {
            open.names.insert(r.app.key.clone(), (r.app.name.clone(), r.app.kind));
            let mut key = PairKey {
                app: r.app.key.clone(),
                exe: r.exe.clone(),
                via: r.via,
                remote: r.remote.map(|ip| ip.to_string()),
                port: r.port,
                proto: r.proto,
                host: r.host.clone(),
            };
            if open.pairs.len() >= MAX_PAIRS_PER_MINUTE && !open.pairs.contains_key(&key) {
                key.remote = None;
                key.port = None;
                key.host = None;
            }
            open.pairs.entry(key).or_default().add(r.bytes_in, r.bytes_out);
            self.pids.entry(r.app.key.clone()).or_default().insert(r.pid, now);
        }
        flushed
    }

    /// Close the open minute when `now` is past it: keep it in memory and
    /// append its lines to the day file.
    pub fn roll(&mut self, now: u64) -> io::Result<()> {
        let t = minute_of(now);
        let Some(open) = self.open.take_if(|m| m.t < t) else { return Ok(()) };
        let result = if open.pairs.is_empty() { Ok(()) } else { self.append(&open) };
        self.minutes.push_back(open);
        while self.minutes.front().is_some_and(|m| m.t + 3600 <= t) {
            self.minutes.pop_front();
        }
        self.generation += 1;
        for apps in self.pids.values_mut() {
            apps.retain(|_, seen| *seen + PID_FRESH_SECS >= now);
        }
        self.pids.retain(|_, p| !p.is_empty());
        result
    }

    fn append(&self, minute: &Minute) -> io::Result<()> {
        ensure_dir(&self.dir)?;
        let path = self.dir.join(day_file_name(minute.t));
        let mut file = OpenOptions::new().create(true).append(true).mode(0o600).open(&path)?;
        let mut text = String::new();
        for line in minute.lines() {
            text.push_str(&serde_json::to_string(&line).map_err(io::Error::other)?);
            text.push('\n');
        }
        // One write per minute: a crash cuts at most the last line.
        file.write_all(text.as_bytes())
    }

    /// The reply for a range, without the sampler state and places, which
    /// the daemon adds.
    pub fn query(&mut self, now: u64, range: TrafficRange, app: Option<&str>) -> AppTrafficResult {
        let mut agg = if range == TrafficRange::Hour {
            let mut agg = Agg::new(range, now);
            for m in &self.minutes {
                for l in m.lines() {
                    agg.add(&l);
                }
            }
            agg
        } else {
            let fresh = Agg::new(range, now);
            match self.cache.get(&range) {
                Some((generation, start, agg)) if *generation == self.generation && *start == fresh.start => agg.clone(),
                _ => {
                    let mut agg = fresh;
                    for l in self.read_lines(agg.start) {
                        agg.add(&l);
                    }
                    self.cache.insert(range, (self.generation, agg.start, agg.clone()));
                    agg
                }
            }
        };
        if let Some(open) = &self.open {
            for l in open.lines() {
                agg.add(&l);
            }
        }
        self.result(agg, range, app, now)
    }

    fn result(&self, agg: Agg, range: TrafficRange, app: Option<&str>, now: u64) -> AppTrafficResult {
        let wanted = |key: &str| app.is_none_or(|a| a == key);
        let mut buckets: Vec<TrafficBucket> = (0..agg.n)
            .map(|i| TrafficBucket { t: agg.start + i as u64 * agg.bucket_secs, bytes_in: 0, bytes_out: 0 })
            .collect();
        let mut totals = TrafficTotals::default();
        let mut apps = vec![];
        for (key, a) in &agg.apps {
            if !wanted(key) {
                continue;
            }
            for (b, v) in buckets.iter_mut().zip(&a.buckets) {
                b.bytes_in += v.bytes_in;
                b.bytes_out += v.bytes_out;
            }
            totals.bytes_in += a.total.bytes_in;
            totals.bytes_out += a.total.bytes_out;
            for via in [Via::Router, Via::Proxy, Via::Direct] {
                let v = *a.by_via.clone().get_mut(via);
                totals.by_via.get_mut(via).add(v.bytes_in, v.bytes_out);
            }
            let pids = self
                .pids
                .get(key)
                .map(|p| p.iter().filter(|(_, seen)| **seen + PID_FRESH_SECS >= now).map(|(pid, _)| *pid).collect())
                .unwrap_or_default();
            apps.push(AppTraffic {
                app: key.clone(),
                name: a.name.clone(),
                kind: a.kind.unwrap_or(AppKind::Program),
                paths: a.exes.iter().cloned().collect(),
                pids,
                bytes_in: a.total.bytes_in,
                bytes_out: a.total.bytes_out,
                by_via: a.by_via,
                addresses: a.remotes.len(),
                spark: a.spark.to_vec(),
            });
        }
        apps.sort_by(|a, b| b.bytes_in.cmp(&a.bytes_in).then(b.bytes_out.cmp(&a.bytes_out)).then(a.name.cmp(&b.name)));
        let mut remotes: Vec<RemoteTraffic> = agg
            .remotes
            .into_iter()
            .filter(|((app, ..), _)| wanted(app))
            .map(|((app, exe, address, port, proto, via, host), v)| RemoteTraffic {
                app,
                exe,
                address,
                port,
                proto,
                host,
                asn_org: None,
                server_name: None,
                place: None,
                via,
                bytes_in: v.bytes_in,
                bytes_out: v.bytes_out,
            })
            .collect();
        remotes.sort_by(|a, b| {
            (b.bytes_in + b.bytes_out).cmp(&(a.bytes_in + a.bytes_out)).then_with(|| (&a.app, &a.address).cmp(&(&b.app, &b.address)))
        });
        remotes.truncate(MAX_REMOTES);
        AppTrafficResult {
            range,
            sampler: SamplerState::Off,
            error: None,
            bucket_secs: agg.bucket_secs,
            buckets,
            apps,
            remotes,
            totals,
            places: false,
        }
    }

    /// Every line of the day files from the day of `since` on. Lines that do
    /// not parse (a half-written last line) are skipped.
    fn read_lines(&mut self, since: u64) -> Vec<Line> {
        let first = day_file_name(since);
        let mut files: Vec<PathBuf> = day_files(&self.dir).into_iter().filter(|(name, _, _)| *name >= first).map(|(_, p, _)| p).collect();
        files.sort();
        let mut out = vec![];
        for path in files {
            let Ok(file) = fs::File::open(&path) else { continue };
            self.files_read += 1;
            for line in BufReader::new(file).lines() {
                let Ok(line) = line else { break };
                if let Ok(l) = serde_json::from_str::<Line>(&line)
                    && l.t >= since
                {
                    out.push(l);
                }
            }
        }
        out
    }

    /// Delete every day file and forget the memory.
    pub fn clear(&mut self) -> usize {
        let mut removed = 0;
        for (_, path, _) in day_files(&self.dir) {
            if fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        self.minutes.clear();
        self.open = None;
        self.cache.clear();
        self.generation += 1;
        removed
    }

    /// Forget the cached long ranges, after retention deleted files.
    pub fn forget_cache(&mut self) {
        self.cache.clear();
    }
}

/// The start of the minute of `t`.
pub fn minute_of(t: u64) -> u64 {
    t - t % 60
}

/// `2026-11-02.ndjson`: the UTC date of `t`.
pub fn day_file_name(t: u64) -> String {
    let date = time::OffsetDateTime::from_unix_timestamp(t as i64).map(|d| d.date()).unwrap_or(time::Date::MIN);
    format!("{:04}-{:02}-{:02}.ndjson", date.year(), date.month() as u8, date.day())
}

/// The day files in `dir`: name, path and size, oldest first.
fn day_files(dir: &Path) -> Vec<(String, PathBuf, u64)> {
    let Ok(entries) = fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<(String, PathBuf, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let ok = name.len() == 17
                && name.ends_with(".ndjson")
                && name[..10].bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            ok.then(|| (name, e.path(), meta.len()))
        })
        .collect();
    out.sort();
    out
}

/// Bytes of history on disk.
pub fn size_on_disk(dir: &Path) -> u64 {
    day_files(dir).iter().map(|(_, _, size)| size).sum()
}

/// I4: delete the day files older than `days` days, then the oldest while
/// all of them together are above `max_bytes`. Today's file stays. Returns
/// the number deleted.
pub fn retain(dir: &Path, now: u64, days: u32, max_bytes: u64) -> usize {
    let today = day_file_name(now);
    // With `days` = 1 only today's file stays.
    let keep_from = day_file_name(now.saturating_sub((days.max(1) as u64 - 1) * 86_400));
    let mut removed = 0;
    let mut files = day_files(dir);
    files.retain(|(name, path, _)| {
        if *name < keep_from && fs::remove_file(path).is_ok() {
            removed += 1;
            return false;
        }
        true
    });
    let mut total: u64 = files.iter().map(|(_, _, s)| s).sum();
    for (name, path, size) in &files {
        if total <= max_bytes || *name == today {
            break;
        }
        if fs::remove_file(path).is_ok() {
            removed += 1;
            total -= size;
        }
    }
    removed
}

fn ensure_dir(dir: &Path) -> io::Result<()> {
    if !dir.is_dir() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let mode = fs::metadata(dir)?.permissions().mode() & 0o777;
    if mode != 0o700 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::super::attribute::AppId;
    use super::*;

    /// 2026-11-02 10:00:00 UTC.
    const T0: u64 = 1_793_613_600;

    fn dir() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("lr").tempdir().unwrap()
    }

    fn row(app: &str, via: Via, remote: Option<&str>, bytes_in: u64, bytes_out: u64) -> AppBytes {
        AppBytes {
            app: AppId { key: app.into(), name: app.trim_start_matches('/').into(), kind: AppKind::Program },
            exe: Some(app.into()),
            pid: 42,
            via,
            remote: remote.map(|r| r.parse::<IpAddr>().unwrap()),
            port: remote.map(|_| 443),
            proto: Proto::Tcp,
            host: None,
            bytes_in,
            bytes_out,
        }
    }

    #[test]
    fn the_samples_of_a_minute_add_up_to_its_line() {
        let d = dir();
        let traffic = d.path().join("traffic");
        let mut h = History::open(traffic.clone(), T0);
        // Twelve samples of 5 s in one minute.
        for i in 0..12 {
            h.add(T0 + i * 5, &[row("/bin/a", Via::Direct, Some("203.0.113.4"), 10, 1)]).unwrap();
        }
        assert!(!traffic.exists(), "nothing is written before the minute ends");
        h.add(T0 + 60, &[]).unwrap();
        let text = fs::read_to_string(traffic.join("2026-11-02.ndjson")).unwrap();
        let lines: Vec<Line> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 1);
        assert_eq!((lines[0].t, lines[0].bytes_in, lines[0].bytes_out), (T0, 120, 12));
        assert_eq!(lines[0].remote.as_deref(), Some("203.0.113.4"));
        // Folder 0700, file 0600.
        assert_eq!(fs::metadata(&traffic).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(traffic.join("2026-11-02.ndjson")).unwrap().permissions().mode() & 0o777, 0o600);
        // The hour view has it, with the open minute.
        h.add(T0 + 65, &[row("/bin/a", Via::Router, Some("::1"), 5, 5)]).unwrap();
        let r = h.query(T0 + 70, TrafficRange::Hour, None);
        assert_eq!(r.buckets.len(), 60);
        assert_eq!((r.totals.bytes_in, r.totals.by_via.router.bytes_in), (125, 5));
        assert_eq!(r.apps[0].addresses, 2);
        assert_eq!(r.apps[0].pids, vec![42]);
        assert_eq!(r.apps[0].spark.iter().sum::<u64>(), 142);
        assert_eq!(r.buckets.iter().map(|b| b.bytes_in).sum::<u64>(), 125);
        // Each remote names its executable, so an app splits by process.
        assert!(r.remotes.iter().all(|x| x.exe.as_deref() == Some("/bin/a")));
    }

    #[test]
    fn a_day_file_with_a_half_last_line_reads_back_without_it() {
        let d = dir();
        let traffic = d.path().join("traffic");
        let mut h = History::open(traffic.clone(), T0);
        h.add(T0, &[row("/bin/a", Via::Direct, Some("203.0.113.4"), 10, 1)]).unwrap();
        h.add(T0 + 60, &[row("/bin/b", Via::Direct, Some("203.0.113.5"), 20, 2)]).unwrap();
        h.roll(T0 + 120).unwrap();
        let path = traffic.join("2026-11-02.ndjson");
        let mut text = fs::read_to_string(&path).unwrap();
        text.truncate(text.len() - 20);
        fs::write(&path, text).unwrap();
        // A restart reads the last hour back from the file.
        let mut again = History::open(traffic, T0 + 130);
        let r = again.query(T0 + 130, TrafficRange::Hour, None);
        assert_eq!(r.apps.len(), 1);
        assert_eq!(r.apps[0].app, "/bin/a");
    }

    #[test]
    fn two_long_reads_in_the_same_minute_open_the_files_once() {
        let d = dir();
        let traffic = d.path().join("traffic");
        let mut h = History::open(traffic, T0);
        h.add(T0, &[row("/bin/a", Via::Direct, Some("203.0.113.4"), 10, 1)]).unwrap();
        h.roll(T0 + 60).unwrap();
        let before = h.files_read;
        let r1 = h.query(T0 + 61, TrafficRange::Week, None);
        let r2 = h.query(T0 + 65, TrafficRange::Week, Some("/bin/a"));
        assert_eq!(h.files_read, before + 1);
        assert_eq!((r1.totals.bytes_in, r2.totals.bytes_in), (10, 10));
        assert_eq!(r1.buckets.len(), 96);
        // The next minute line makes the next read open the files again.
        h.add(T0 + 70, &[row("/bin/a", Via::Direct, Some("203.0.113.4"), 1, 1)]).unwrap();
        h.roll(T0 + 125).unwrap();
        let r3 = h.query(T0 + 126, TrafficRange::Week, None);
        assert_eq!(h.files_read, before + 2);
        assert_eq!(r3.totals.bytes_in, 11);
        // The app filter.
        assert!(h.query(T0 + 126, TrafficRange::Week, Some("/bin/none")).apps.is_empty());
    }

    #[test]
    fn retention_by_days_and_by_size() {
        let d = dir();
        let traffic = d.path();
        let day = 86_400;
        for (t, size) in [(T0 - 3 * day, 300), (T0 - 2 * day, 300), (T0 - day, 300), (T0, 300)] {
            fs::write(traffic.join(day_file_name(t)), vec![b'x'; size]).unwrap();
        }
        fs::write(traffic.join("notes.txt"), "kept").unwrap();
        // days = 1 at midnight + 1 day: only today's file stays.
        assert_eq!(retain(traffic, T0, 7, u64::MAX), 0);
        assert_eq!(retain(traffic, T0, 1, u64::MAX), 3);
        assert_eq!(day_files(traffic).len(), 1);
        assert!(traffic.join("notes.txt").exists());
        // The size cap deletes the oldest first, never today's.
        for t in [T0 - 2 * day, T0 - day] {
            fs::write(traffic.join(day_file_name(t)), vec![b'x'; 300]).unwrap();
        }
        assert_eq!(retain(traffic, T0, 30, 650), 1);
        let left: Vec<String> = day_files(traffic).into_iter().map(|(n, _, _)| n).collect();
        assert_eq!(left, vec![day_file_name(T0 - day), day_file_name(T0)]);
        assert!(size_on_disk(traffic) <= 650);
        assert_eq!(retain(traffic, T0, 30, 10), 1);
        assert_eq!(day_files(traffic).len(), 1, "today's file stays");
    }

    #[test]
    fn clear_deletes_every_day_file_and_the_memory() {
        let d = dir();
        let traffic = d.path().join("traffic");
        let mut h = History::open(traffic.clone(), T0);
        h.add(T0, &[row("/bin/a", Via::Direct, Some("203.0.113.4"), 10, 1)]).unwrap();
        h.roll(T0 + 60).unwrap();
        assert_eq!(h.clear(), 1);
        assert_eq!(size_on_disk(&traffic), 0);
        assert!(h.query(T0 + 61, TrafficRange::Hour, None).apps.is_empty());
    }

    #[test]
    fn day_file_names_are_utc_dates() {
        assert_eq!(day_file_name(T0), "2026-11-02.ndjson");
        assert_eq!(day_file_name(T0 + 14 * 3600), "2026-11-03.ndjson");
        assert_eq!(minute_of(T0 + 59), T0);
    }
}
