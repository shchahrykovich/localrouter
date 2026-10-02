//! The proxy log (ADR 08): every request the forward proxy carries, written to
//! rolling HAR 1.2 files in `<logs>/proxy/`, and a viewer for them at
//! `proxy.localhost`.
//!
//! The network task checks [`HarLog::enabled`] (one atomic load, I1), copies
//! what it saw into a [`HarRecord`] and calls [`HarLog::record`], which only
//! does `try_send` on a bounded queue (I2). The `har-writer` thread exists
//! only while there are records: it starts at the first one and exits after
//! 30 seconds without any, closing its file and dropping its queue (I21).

pub mod entry;
pub mod viewer;
mod writer;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::broadcast;

pub use entry::HarRecord;
pub use writer::{CLOSING, file_key};

use crate::secrets::SecretHeaders;

/// Files kept; older ones are deleted at each new file (I5).
pub const KEEP_FILES: usize = 5;
/// Records the queue holds while the writer is behind; more are dropped.
pub const QUEUE_RECORDS: usize = 4096;
/// The writer thread exits after this long without a record.
pub const IDLE: Duration = Duration::from_secs(30);
/// `proxy_log_file_mb`: the viewer parses a whole file in the browser.
pub const FILE_MB_MIN: u64 = 1;
pub const FILE_MB_MAX: u64 = 200;
pub const FILE_REQUESTS_MIN: u64 = 100;
pub const FILE_REQUESTS_MAX: u64 = 1_000_000;
pub const DEFAULT_FILE_MB: u64 = 20;
pub const DEFAULT_FILE_REQUESTS: u64 = 5000;

/// What the live feed of the viewer sends (`/api/live`).
#[derive(Debug, Clone, PartialEq)]
pub enum LiveEvent {
    /// One entry, as written to the file: JSON on one line.
    Entry(Arc<str>),
    /// A new file started.
    File(String),
    /// The log was turned off.
    Off,
}

/// The settings the log starts with; `set_*` change them later.
#[derive(Debug, Clone)]
pub struct HarSettings {
    /// `<logs>/proxy`.
    pub folder: PathBuf,
    /// `creator.name`: the instance's app name.
    pub creator: String,
    /// `creator.version`: the daemon version.
    pub version: String,
    pub enabled: bool,
    pub file_mb: u64,
    pub file_requests: u64,
}

/// One HAR file in the folder, for the viewer and `get_proxy`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    /// Known for files this daemon run wrote; `None` for older ones.
    pub entries: Option<u64>,
    pub current: bool,
}

/// What the log holds right now. Tests use it to prove the idle state (T17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resources {
    pub writer_thread: bool,
    pub queue: bool,
    pub file_open: bool,
    pub live_channel: bool,
}

pub struct HarLog {
    folder: PathBuf,
    creator: String,
    version: String,
    secrets: Arc<SecretHeaders>,
    on: AtomicBool,
    /// A write failed: nothing more is written until off and on (I15).
    failed: AtomicBool,
    /// `proxy_enabled`, for the viewer's state line.
    proxy_on: AtomicBool,
    file_mb: AtomicU64,
    file_requests: AtomicU64,
    dropped: AtomicU64,
    written: AtomicU64,
    /// The queue's sending side while the writer thread runs.
    sender: Mutex<Option<SyncSender<Box<HarRecord>>>>,
    writer_running: AtomicBool,
    /// The current file. Its lock is the "file lock": the viewer reads the
    /// length under it (I3).
    state: Mutex<writer::State>,
    /// Exists only while a viewer page is open (I21).
    live: Mutex<Option<broadcast::Sender<LiveEvent>>>,
    idle_ms: AtomicU64,
    paused: AtomicBool,
    this: Weak<HarLog>,
}

impl HarLog {
    pub fn new(settings: HarSettings, secrets: Arc<SecretHeaders>) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            state: Mutex::new(writer::State::new(settings.folder.clone())),
            folder: settings.folder,
            creator: settings.creator,
            version: settings.version,
            secrets,
            on: AtomicBool::new(settings.enabled),
            failed: AtomicBool::new(false),
            proxy_on: AtomicBool::new(false),
            file_mb: AtomicU64::new(settings.file_mb),
            file_requests: AtomicU64::new(settings.file_requests),
            dropped: AtomicU64::new(0),
            written: AtomicU64::new(0),
            sender: Mutex::new(None),
            writer_running: AtomicBool::new(false),
            live: Mutex::new(None),
            idle_ms: AtomicU64::new(IDLE.as_millis() as u64),
            paused: AtomicBool::new(false),
            this: this.clone(),
        })
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// True when a record would be written. The network task checks this
    /// before it copies anything (I1).
    pub fn enabled(&self) -> bool {
        self.on.load(Ordering::Relaxed) && !self.failed.load(Ordering::Relaxed)
    }

    /// `proxy_log` from the config.
    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// Hand one record to the writer. Never waits: a full queue drops the
    /// record and counts it (I2).
    pub fn record(&self, record: HarRecord) {
        if !self.enabled() {
            return;
        }
        let record = Box::new(record);
        let mut sender = self.sender.lock().unwrap();
        if let Some(tx) = sender.as_ref() {
            match tx.try_send(record) {
                Ok(()) => return,
                Err(TrySendError::Full(_)) => {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                // The thread is gone (it panicked): start a new one below.
                Err(TrySendError::Disconnected(r)) => return self.start_writer(&mut sender, r),
            }
        }
        self.start_writer(&mut sender, record);
    }

    fn start_writer(&self, sender: &mut Option<SyncSender<Box<HarRecord>>>, first: Box<HarRecord>) {
        let Some(this) = self.this.upgrade() else { return };
        let (tx, rx) = mpsc::sync_channel(QUEUE_RECORDS);
        let _ = tx.try_send(first);
        self.writer_running.store(true, Ordering::SeqCst);
        let spawned = std::thread::Builder::new().name("har-writer".into()).spawn(move || this.run(rx));
        match spawned {
            Ok(_) => *sender = Some(tx),
            Err(e) => {
                self.writer_running.store(false, Ordering::SeqCst);
                self.fail(format!("cannot start the HAR writer: {e}"));
            }
        }
    }

    /// The `har-writer` thread.
    fn run(self: Arc<Self>, rx: mpsc::Receiver<Box<HarRecord>>) {
        loop {
            if self.paused.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            let idle = Duration::from_millis(self.idle_ms.load(Ordering::Relaxed));
            match rx.recv_timeout(idle) {
                Ok(record) => self.write_one(&record),
                Err(RecvTimeoutError::Timeout) => {
                    // Under the sender lock no record can arrive, so none is lost.
                    let mut sender = self.sender.lock().unwrap();
                    if let Ok(record) = rx.try_recv() {
                        drop(sender);
                        self.write_one(&record);
                        continue;
                    }
                    *sender = None;
                    self.state.lock().unwrap().close();
                    self.writer_running.store(false, Ordering::SeqCst);
                    return;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.state.lock().unwrap().close();
                    self.writer_running.store(false, Ordering::SeqCst);
                    return;
                }
            }
        }
    }

    fn write_one(&self, record: &HarRecord) {
        if self.failed.load(Ordering::Relaxed) {
            return;
        }
        let entry = record.to_entry(&self.secrets);
        let line = match serde_json::to_string(&entry) {
            Ok(line) => line,
            Err(_) => return,
        };
        let limits = writer::Limits {
            bytes: self.file_mb.load(Ordering::Relaxed).saturating_mul(1024 * 1024),
            entries: self.file_requests.load(Ordering::Relaxed),
        };
        let result = {
            let mut state = self.state.lock().unwrap();
            state.append(line.as_bytes(), record.started, limits, &self.header())
        };
        match result {
            Ok(new_file) => {
                self.written.fetch_add(1, Ordering::Relaxed);
                if let Some(name) = new_file {
                    self.send_live(|| LiveEvent::File(name));
                }
                self.send_live(|| LiveEvent::Entry(Arc::from(line)));
            }
            Err(e) => self.fail(e),
        }
    }

    fn fail(&self, why: String) {
        tracing::error!("the proxy log stopped: {why}");
        let mut state = self.state.lock().unwrap();
        state.error = Some(why);
        state.close();
        self.failed.store(true, Ordering::Relaxed);
    }

    /// Send to open viewer pages. The event is built only when a page is
    /// open; with none left, the channel is dropped here (I21).
    fn send_live(&self, event: impl FnOnce() -> LiveEvent) {
        let mut live = self.live.lock().unwrap();
        if let Some(tx) = live.as_ref() {
            if tx.receiver_count() == 0 {
                *live = None;
            } else {
                let _ = tx.send(event());
            }
        }
    }

    /// The first line of every file.
    fn header(&self) -> String {
        let creator = serde_json::json!({ "name": self.creator, "version": self.version });
        format!("{{\"log\":{{\"version\":\"1.2\",\"creator\":{creator},\"pages\":[],\"entries\":[")
    }

    /// Turn the log on or off, at once. Off: no new record; the current file
    /// stays valid. On after off: a new file at the next record, and a
    /// write error is forgotten.
    pub fn set_enabled(&self, on: bool) {
        let was = self.on.swap(on, Ordering::SeqCst);
        if was == on {
            return;
        }
        if on {
            let mut state = self.state.lock().unwrap();
            state.error = None;
            state.end_file();
            self.failed.store(false, Ordering::SeqCst);
        } else {
            self.send_live(|| LiveEvent::Off);
        }
    }

    /// New limits apply at the next "full" check.
    pub fn set_limits(&self, file_mb: u64, file_requests: u64) {
        self.file_mb.store(file_mb, Ordering::Relaxed);
        self.file_requests.store(file_requests, Ordering::Relaxed);
    }

    pub fn set_proxy_on(&self, on: bool) {
        self.proxy_on.store(on, Ordering::Relaxed);
    }

    pub fn proxy_on(&self) -> bool {
        self.proxy_on.load(Ordering::Relaxed)
    }

    pub fn limits(&self) -> (u64, u64) {
        (self.file_mb.load(Ordering::Relaxed), self.file_requests.load(Ordering::Relaxed))
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Entries written since the daemon started.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    pub fn error(&self) -> Option<String> {
        self.state.lock().unwrap().error.clone()
    }

    /// The name of the file the next entry goes to, if one is open.
    pub fn current(&self) -> Option<String> {
        self.state.lock().unwrap().current_name()
    }

    /// The current file's name and its length without the closing line,
    /// read under the file lock. Every byte before that length is final.
    pub fn current_snapshot(&self) -> Option<(String, u64)> {
        self.state.lock().unwrap().snapshot()
    }

    /// The HAR files in the folder, newest first. Read at each call (no
    /// index in memory).
    pub fn files(&self) -> Vec<FileInfo> {
        let (current, counts) = {
            let state = self.state.lock().unwrap();
            (state.snapshot(), state.run_counts())
        };
        let mut found: Vec<(writer::FileKey, FileInfo)> = vec![];
        let Ok(dir) = std::fs::read_dir(&self.folder) else { return vec![] };
        for item in dir.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(key) = file_key(&name) else { continue };
            let Ok(meta) = std::fs::symlink_metadata(item.path()) else { continue };
            if !meta.is_file() {
                continue;
            }
            let is_current = current.as_ref().is_some_and(|(n, _)| *n == name);
            let size = match &current {
                Some((n, len)) if *n == name => len + CLOSING.len() as u64,
                _ => meta.len(),
            };
            let entries = counts.iter().find(|(n, _)| *n == name).map(|(_, c)| *c);
            found.push((key, FileInfo { name, size, entries, current: is_current }));
        }
        found.sort_by_key(|f| std::cmp::Reverse(f.0));
        found.into_iter().map(|(_, f)| f).collect()
    }

    /// Once, when the daemon starts and before the viewer answers: make the
    /// newest file of an earlier run valid again if a crash cut it (I16).
    pub fn repair_at_start(&self) {
        if let Err(e) = writer::repair_newest(&self.folder) {
            tracing::warn!("could not repair the newest proxy log file: {e}");
        }
    }

    /// A receiver for the live feed; the first one makes the channel.
    pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        let mut live = self.live.lock().unwrap();
        match live.as_ref() {
            Some(tx) => tx.subscribe(),
            None => {
                let (tx, rx) = broadcast::channel(64);
                *live = Some(tx);
                rx
            }
        }
    }

    /// What the log holds right now (T17).
    pub fn resources(&self) -> Resources {
        Resources {
            writer_thread: self.writer_running.load(Ordering::SeqCst),
            queue: self.sender.lock().unwrap().is_some(),
            file_open: self.state.lock().unwrap().is_open(),
            live_channel: self.live.lock().unwrap().is_some(),
        }
    }

    /// Tests only: hold the writer, so the queue fills (T5).
    pub fn pause_for_tests(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Tests only: a shorter idle time than 30 seconds (T17).
    pub fn set_idle_for_tests(&self, idle: Duration) {
        self.idle_ms.store(idle.as_millis() as u64, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Instant, SystemTime};

    use super::*;
    use crate::logs::ProxyMode;

    fn log(dir: &Path) -> Arc<HarLog> {
        HarLog::new(
            HarSettings {
                folder: dir.join("proxy"),
                creator: "LocalRouter-dev".into(),
                version: "9.9.9".into(),
                enabled: true,
                file_mb: 20,
                file_requests: 5000,
            },
            SecretHeaders::new(),
        )
    }

    fn rec(i: usize) -> HarRecord {
        let mut r = HarRecord::new(SystemTime::now(), "GET", format!("http://a.example/{i}"), ProxyMode::Http);
        r.status = 200;
        r
    }

    fn wait_until(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // I1: off records nothing and starts nothing.
    #[test]
    fn off_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        log.set_enabled(false);
        for i in 0..10 {
            log.record(rec(i));
        }
        assert_eq!(log.resources(), Resources { writer_thread: false, queue: false, file_open: false, live_channel: false });
        assert!(!dir.path().join("proxy").exists());
    }

    // T17, I21: the thread, its queue and its file go after the idle time,
    // and the next record appends to the same file.
    #[test]
    fn idle_writer_exits_and_comes_back_to_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        log.set_idle_for_tests(Duration::from_millis(100));
        log.record(rec(0));
        wait_until("one entry", || log.written() == 1);
        let first = log.current().unwrap();
        wait_until("idle exit", || !log.resources().writer_thread);
        assert_eq!(log.resources(), Resources { writer_thread: false, queue: false, file_open: false, live_channel: false });
        log.record(rec(1));
        wait_until("second entry", || log.written() == 2);
        assert_eq!(log.current().unwrap(), first, "the same current file");
        assert_eq!(log.files()[0].entries, Some(2));
    }

    // T5 (queue part), I2: a stalled writer drops and never blocks.
    #[test]
    fn a_full_queue_drops_and_does_not_wait() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        log.pause_for_tests(true);
        let start = Instant::now();
        for i in 0..QUEUE_RECORDS + 100 {
            log.record(rec(i));
        }
        assert!(start.elapsed() < Duration::from_secs(2), "record() waited");
        assert!(log.dropped() >= 99, "dropped {}", log.dropped());
        log.pause_for_tests(false);
        wait_until("queue drained", || log.written() + log.dropped() >= (QUEUE_RECORDS + 100) as u64);
    }

    // T17: the live channel exists only while a page listens.
    #[test]
    fn live_channel_only_while_a_page_listens() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        assert!(!log.resources().live_channel);
        let mut rx = log.subscribe();
        assert!(log.resources().live_channel);
        log.record(rec(0));
        wait_until("entry", || log.written() == 1);
        let mut events = vec![];
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        assert!(matches!(events[0], LiveEvent::File(_)), "{events:?}");
        assert!(matches!(&events[1], LiveEvent::Entry(line) if line.contains("a.example/0")), "{events:?}");
        drop(rx);
        log.record(rec(1));
        wait_until("entry", || log.written() == 2);
        assert!(!log.resources().live_channel, "dropped at the next record");
    }

    // Off sends `off` to open pages; on after off starts a new file.
    #[test]
    fn off_then_on_starts_a_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = log(dir.path());
        log.record(rec(0));
        wait_until("entry", || log.written() == 1);
        let first = log.current().unwrap();
        let mut rx = log.subscribe();
        log.set_enabled(false);
        assert_eq!(rx.try_recv().unwrap(), LiveEvent::Off);
        log.record(rec(1));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(log.written(), 1);
        log.set_enabled(true);
        log.record(rec(2));
        wait_until("entry", || log.written() == 2);
        assert_ne!(log.current().unwrap(), first);
    }
}
