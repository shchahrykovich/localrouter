//! The traffic sampler (ADR 11, change 1): `/usr/bin/nettop` as a child
//! process while `traffic` is on, the history in `<data>/traffic/`, and the
//! places databases in `<data>/geo/`.
//!
//! - I1: with `traffic` off there is no nettop child and nothing is written.
//! - I2: the daemon's own pid is dropped from every sample.
//! - I5: a failing nettop never touches routing or the proxy: it starts
//!   again after 1, 2, 4 … up to 60 seconds, and `status` says why.
//!
//! nettop is a tool, not an API: a sample of which no line parses stops the
//! child and reports "nettop output not understood".

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use localrouter_core::api::{AppTrafficResult, GeoDbInfo, GetAppTrafficParams, SamplerState};
use localrouter_core::traffic::attribute::{self, PeerInfo, Ports};
use localrouter_core::traffic::geo::{self, Places};
use localrouter_core::traffic::history::{self, History};
use localrouter_core::traffic::nettop::{self, Splitter};
use localrouter_core::traffic::peers::PeerTable;
use localrouter_core::traffic::SAMPLE_SECS;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_util::sync::CancellationToken;

const NETTOP: &str = "/usr/bin/nettop";
/// The waits before nettop starts again, in seconds.
const BACKOFF: [u64; 7] = [1, 2, 4, 8, 16, 32, 60];
/// A child that ran this long resets the wait to 1 second.
const HEALTHY: Duration = Duration::from_secs(60);
/// How long to wait for a line before closing the minute anyway.
const QUIET: Duration = Duration::from_millis(1000);

/// The ports that make a connection `router` or `proxy`, read at each sample.
pub type PortsFn = Arc<dyn Fn() -> Ports + Send + Sync>;

pub struct Traffic {
    dir: PathBuf,
    geo_dir: PathBuf,
    peers: Arc<PeerTable>,
    state: Mutex<(SamplerState, Option<String>)>,
    history: Arc<Mutex<History>>,
    places: RwLock<Option<Arc<Places>>>,
    sampler: Mutex<Option<CancellationToken>>,
    /// `traffic_history_days`, for the hourly retention pass.
    days: AtomicU32,
}

impl Traffic {
    pub fn new(data: &Path, peers: Arc<PeerTable>) -> Arc<Self> {
        let dir = data.join("traffic");
        let history = History::open(dir.clone(), unix_now());
        Arc::new(Self {
            dir,
            geo_dir: data.join("geo"),
            peers,
            state: Mutex::new((SamplerState::Off, None)),
            history: Arc::new(Mutex::new(history)),
            places: RwLock::new(None),
            sampler: Mutex::new(None),
            // 0 is never a setting: the first `apply` always runs retention.
            days: AtomicU32::new(0),
        })
    }

    pub fn state(&self) -> (SamplerState, Option<String>) {
        self.state.lock().unwrap().clone()
    }

    fn set_state(&self, state: SamplerState, error: Option<String>) {
        *self.state.lock().unwrap() = (state, error);
    }

    /// Start or stop the sampler and open or close the places databases to
    /// match the config. `parent` is the daemon's shutdown token.
    pub fn apply(self: &Arc<Self>, traffic: bool, places: bool, days: u32, ports: PortsFn, parent: &CancellationToken) {
        let old_days = self.days.swap(days, Ordering::Relaxed);
        if old_days != days {
            self.retain(days);
        }
        {
            let mut sampler = self.sampler.lock().unwrap();
            match (traffic, sampler.is_some()) {
                (true, false) => {
                    let cancel = parent.child_token();
                    *sampler = Some(cancel.clone());
                    self.set_state(SamplerState::Running, None);
                    let this = self.clone();
                    tokio::spawn(async move { this.run(ports, cancel).await });
                }
                (false, true) => {
                    if let Some(cancel) = sampler.take() {
                        cancel.cancel();
                    }
                    self.set_state(SamplerState::Off, None);
                }
                _ => {}
            }
        }
        let open = self.places.read().unwrap().is_some();
        if places && !open {
            self.reload_places();
        } else if !places && open {
            *self.places.write().unwrap() = None;
        }
    }

    /// Delete old day files: at start, every hour and when the setting
    /// changes (I4).
    pub fn retain(&self, days: u32) {
        if !self.dir.is_dir() {
            return;
        }
        let removed = history::retain(&self.dir, unix_now(), days, history::MAX_BYTES);
        if removed > 0 {
            tracing::info!("traffic history: removed {removed} old day files");
            self.history.lock().unwrap().forget_cache();
        }
    }

    pub fn clear(&self) -> usize {
        self.history.lock().unwrap().clear()
    }

    pub fn history_bytes(&self) -> Option<u64> {
        self.dir.is_dir().then(|| history::size_on_disk(&self.dir))
    }

    pub fn geo_info(&self) -> Option<GeoDbInfo> {
        geo::info(&self.geo_dir)
    }

    fn reload_places(&self) {
        let places = Places::open(&self.geo_dir);
        *self.places.write().unwrap() = (!places.is_empty()).then(|| Arc::new(places));
    }

    /// Download the DB-IP databases of this month (I11: only here, and only
    /// with the verifying TLS settings of `upstream`).
    pub async fn update_geo_db(&self, upstream: &localrouter_core::upstream::Upstream, places_on: bool) -> Result<GeoDbInfo, String> {
        let (year, month) = geo::current_month();
        let info = geo::install(&self.geo_dir, year, month, |url| geo::fetch(upstream, url)).await?;
        if places_on {
            self.reload_places();
        }
        Ok(info)
    }

    /// The reply of `get_app_traffic`. Long ranges read files, so this runs
    /// off the network tasks.
    pub async fn query(&self, p: GetAppTrafficParams) -> AppTrafficResult {
        let history = self.history.clone();
        let places = self.places.read().unwrap().clone();
        let (state, error) = self.state();
        let mut result = tokio::task::spawn_blocking(move || {
            let mut r = history.lock().unwrap().query(unix_now(), p.range, p.app.as_deref());
            if let Some(places) = &places {
                for remote in &mut r.remotes {
                    let Some(ip) = remote.address.as_deref().and_then(|a| a.parse::<IpAddr>().ok()) else { continue };
                    if ip.is_loopback() {
                        continue;
                    }
                    let (place, owner) = places.lookup(ip);
                    remote.place = place;
                    remote.asn_org = owner;
                }
            }
            r
        })
        .await
        .unwrap_or_else(|e| panic!("traffic query failed: {e}"));
        result.sampler = state;
        result.error = error;
        result.places = self.places.read().unwrap().is_some();
        result
    }

    /// Run nettop until `cancel`, starting it again when it fails.
    async fn run(self: Arc<Self>, ports: PortsFn, cancel: CancellationToken) {
        self.retain(self.days.load(Ordering::Relaxed));
        let mut attempt = 0usize;
        let mut last_retain = Instant::now();
        loop {
            let started = Instant::now();
            let why = tokio::select! {
                why = self.run_once(&ports, &mut last_retain) => why,
                _ = cancel.cancelled() => return,
            };
            if started.elapsed() >= HEALTHY {
                attempt = 0;
            }
            let wait = BACKOFF[attempt.min(BACKOFF.len() - 1)];
            attempt += 1;
            tracing::warn!("traffic sampler: {why}; starting nettop again in {wait} s");
            self.set_state(SamplerState::Restarting, Some(why));
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                _ = cancel.cancelled() => return,
            }
        }
    }

    /// One nettop child, until it exits or prints something not understood.
    /// Returns why it stopped. The child is killed when this is dropped.
    async fn run_once(&self, ports: &PortsFn, last_retain: &mut Instant) -> String {
        let program = nettop_program();
        let mut child = match tokio::process::Command::new(&program)
            .args(nettop::args(SAMPLE_SECS))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return format!("cannot start {}: {e}", program.display()),
        };
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(mut err) = child.stderr.take() {
            let sink = stderr.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                while let Ok(n) = err.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let mut s = sink.lock().unwrap();
                    s.push_str(&String::from_utf8_lossy(&buf[..n]));
                    let len = s.len();
                    if len > 500 {
                        let cut = s.char_indices().map(|(i, _)| i).find(|&i| i >= len - 500).unwrap_or(0);
                        s.drain(..cut);
                    }
                }
            });
        }
        let Some(stdout) = child.stdout.take() else { return "nettop has no output".into() };
        let mut lines = BufReader::new(stdout).lines();
        let mut splitter = Splitter::default();
        let mut baseline = true;
        let me = std::process::id();
        loop {
            let ready = match tokio::time::timeout(QUIET, lines.next_line()).await {
                Ok(Ok(Some(line))) => splitter.push(&line),
                Ok(Ok(None)) | Ok(Err(_)) => {
                    let status = child.wait().await.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                    let err = stderr.lock().unwrap().trim().to_string();
                    return if err.is_empty() { format!("nettop exited ({status})") } else { format!("nettop exited ({status}): {err}") };
                }
                // A pause: only text that never had a header is a sample now,
                // so that a changed format is reported.
                Err(_) => splitter.take_headerless(),
            };
            let now = unix_now();
            let Some(text) = ready else {
                // Close the minute on time even when nothing is read.
                if let Err(e) = self.history.lock().unwrap().roll(now) {
                    tracing::warn!("cannot write traffic history: {e}");
                }
                continue;
            };
            let sample = match nettop::parse_sample(&text) {
                Ok(s) => s,
                Err(e) => return e.to_string(),
            };
            self.set_state(SamplerState::Running, None);
            // The first sample counts from each socket's start: it is only
            // the starting point.
            if std::mem::take(&mut baseline) {
                continue;
            }
            let ports = ports();
            let peers = self.peers.clone();
            let peer = move |port: u16| -> Option<PeerInfo> { peers.get(port) };
            let rows = attribute::apply(&sample, &ports, me, &peer, &pid_path);
            if let Err(e) = self.history.lock().unwrap().add(now, &rows) {
                tracing::warn!("cannot write traffic history: {e}");
            }
            if last_retain.elapsed() >= Duration::from_secs(3600) {
                *last_retain = Instant::now();
                self.retain(self.days.load(Ordering::Relaxed));
            }
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// `/usr/bin/nettop`. Debug builds read `LOCALROUTER_TEST_NETTOP`: a program
/// that prints nettop-like text, so tests need no real traffic.
fn nettop_program() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Ok(p) = std::env::var("LOCALROUTER_TEST_NETTOP") {
        return PathBuf::from(p);
    }
    PathBuf::from(NETTOP)
}

/// The executable of a process (`proc_pidpath`), or `None` when it ended.
fn pid_path(pid: u32) -> Option<PathBuf> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is as large as the size passed.
    let n = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(buf)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_path_reads_this_process() {
        let me = pid_path(std::process::id()).unwrap();
        assert_eq!(me, std::env::current_exe().unwrap().canonicalize().unwrap());
        // A pid that cannot exist.
        assert_eq!(pid_path(u32::MAX >> 1), None);
    }
}
