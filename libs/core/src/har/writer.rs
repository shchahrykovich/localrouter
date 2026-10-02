//! The current HAR file: create, append, roll, prune, repair (ADR 08,
//! change 1).
//!
//! A file is always valid HAR (I3):
//!
//! ```text
//! {"log":{"version":"1.2","creator":{…},"pages":[],"entries":[
//! {…entry 1…},
//! {…entry 2…}
//! ]}}
//! ```
//!
//! To add an entry the writer writes `,\n{entry}\n]}}\n` over the closing
//! `\n]}}\n` with one positioned write. Every byte before the closing never
//! changes once written, so a reader that knows the length can stream them
//! and add the closing itself.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::KEEP_FILES;

/// The end of every file. The bytes before it are final.
pub const CLOSING: &[u8] = b"\n]}}\n";

/// Sort key of a HAR file name: date, time, and the `-2` suffix.
pub type FileKey = (u64, u64, u64);

/// `proxy-YYYYMMDD-HHMMSS.har` or `proxy-YYYYMMDD-HHMMSS-N.har`; any other
/// name is not ours and is never touched (I5) or served (I9).
pub fn file_key(name: &str) -> Option<FileKey> {
    let rest = name.strip_prefix("proxy-")?.strip_suffix(".har")?;
    let digits = |s: &str, n: usize| (s.len() == n && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse::<u64>().ok()).flatten();
    let mut parts = rest.split('-');
    let date = digits(parts.next()?, 8)?;
    let time = digits(parts.next()?, 6)?;
    let n = match parts.next() {
        None => 1,
        Some(s) if !s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit()) => s.parse().ok()?,
        Some(_) => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((date, time, n))
}

/// When a file is full.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub bytes: u64,
    pub entries: u64,
}

struct Current {
    name: String,
    path: PathBuf,
    /// The file's length, closing included.
    size: u64,
    entries: u64,
}

pub struct State {
    folder: PathBuf,
    current: Option<Current>,
    file: Option<File>,
    pub error: Option<String>,
    /// Entry counts of the files this run wrote, at most [`KEEP_FILES`].
    counts: Vec<(String, u64)>,
}

impl State {
    pub fn new(folder: PathBuf) -> Self {
        Self { folder, current: None, file: None, error: None, counts: vec![] }
    }

    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }

    /// Close the file; the next entry reopens it.
    pub fn close(&mut self) {
        self.file = None;
    }

    /// Forget the current file: the next entry starts a new one.
    pub fn end_file(&mut self) {
        self.file = None;
        self.current = None;
    }

    pub fn current_name(&self) -> Option<String> {
        self.current.as_ref().map(|c| c.name.clone())
    }

    pub fn snapshot(&self) -> Option<(String, u64)> {
        self.current.as_ref().map(|c| (c.name.clone(), c.size - CLOSING.len() as u64))
    }

    pub fn run_counts(&self) -> Vec<(String, u64)> {
        self.counts.clone()
    }

    /// Append one entry line. Returns the name of a file it started.
    pub fn append(&mut self, line: &[u8], started: SystemTime, limits: Limits, header: &str) -> Result<Option<String>, String> {
        // The user may have deleted the file in Finder (G9).
        if self.current.as_ref().is_some_and(|c| !c.path.is_file()) {
            self.end_file();
        }
        let full = self.current.as_ref().is_some_and(|c| {
            c.entries > 0 && (c.entries >= limits.entries || c.size + 2 + line.len() as u64 > limits.bytes)
        });
        let mut started_file = None;
        if self.current.is_none() || full {
            self.end_file();
            started_file = Some(self.start_file(started, header)?);
        }
        if self.file.is_none() {
            let path = &self.current.as_ref().expect("a current file").path;
            let file = OpenOptions::new().write(true).open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
            self.file = Some(file);
        }
        let current = self.current.as_mut().expect("a current file");
        let mut buf = Vec::with_capacity(line.len() + 2 + CLOSING.len());
        buf.extend_from_slice(if current.entries == 0 { b"\n" } else { b",\n" });
        buf.extend_from_slice(line);
        buf.extend_from_slice(CLOSING);
        let at = current.size - CLOSING.len() as u64;
        let file = self.file.as_ref().expect("an open file");
        file.write_all_at(&buf, at).map_err(|e| format!("cannot write {}: {e}", current.path.display()))?;
        current.size = at + buf.len() as u64;
        current.entries += 1;
        let (name, entries) = (current.name.clone(), current.entries);
        match self.counts.iter_mut().find(|(n, _)| *n == name) {
            Some((_, c)) => *c = entries,
            None => {
                self.counts.push((name, entries));
                if self.counts.len() > KEEP_FILES {
                    self.counts.remove(0);
                }
            }
        }
        Ok(started_file)
    }

    /// A new file named after the local time of its first entry, then prune.
    fn start_file(&mut self, started: SystemTime, header: &str) -> Result<String, String> {
        fs::create_dir_all(&self.folder).map_err(|e| format!("cannot create {}: {e}", self.folder.display()))?;
        let stamp = local_stamp(started);
        let mut content = Vec::with_capacity(header.len() + CLOSING.len());
        content.extend_from_slice(header.as_bytes());
        content.extend_from_slice(CLOSING);
        for n in 1u32..1000 {
            let name = if n == 1 { format!("proxy-{stamp}.har") } else { format!("proxy-{stamp}-{n}.har") };
            let path = self.folder.join(&name);
            let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("cannot create {}: {e}", path.display())),
            };
            file.write_all(&content).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            self.current = Some(Current { name: name.clone(), path, size: content.len() as u64, entries: 0 });
            self.file = Some(file);
            prune(&self.folder);
            return Ok(name);
        }
        Err(format!("too many files named proxy-{stamp}-N.har in {}", self.folder.display()))
    }
}

/// HAR files in `folder`, newest first.
fn ours(folder: &Path) -> Vec<(FileKey, PathBuf)> {
    let Ok(dir) = fs::read_dir(folder) else { return vec![] };
    let mut found: Vec<(FileKey, PathBuf)> = dir
        .flatten()
        .filter_map(|item| {
            let key = file_key(&item.file_name().to_string_lossy())?;
            let meta = fs::symlink_metadata(item.path()).ok()?;
            meta.is_file().then(|| (key, item.path()))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found
}

/// Keep the [`KEEP_FILES`] newest of our files; never touch another name.
fn prune(folder: &Path) {
    for (_, path) in ours(folder).into_iter().skip(KEEP_FILES) {
        if let Err(e) = fs::remove_file(&path) {
            tracing::warn!("could not delete {}: {e}", path.display());
        }
    }
}

/// Repair the newest file of an earlier run when a crash cut it: keep the
/// header line and every entry line that parses, and close it again.
pub fn repair_newest(folder: &Path) -> io::Result<bool> {
    let Some((_, path)) = ours(folder).into_iter().next() else { return Ok(false) };
    if is_whole(&path)? {
        return Ok(false);
    }
    let tmp = path.with_file_name(format!(".{}.repair", path.file_name().unwrap_or_default().to_string_lossy()));
    {
        let reader = BufReader::new(File::open(&path)?);
        let mut out = io::BufWriter::new(File::create(&tmp)?);
        let mut lines = reader.split(b'\n');
        let header = match lines.next() {
            Some(Ok(h)) if h.starts_with(b"{\"log\"") && h.ends_with(b"[") => h,
            // Not even a header: not something the writer can repair.
            _ => {
                drop(out);
                let _ = fs::remove_file(&tmp);
                return Ok(false);
            }
        };
        out.write_all(&header)?;
        let mut kept = 0u64;
        for line in lines {
            let line = line?;
            let line = line.strip_suffix(b",").unwrap_or(&line);
            if line.first() != Some(&b'{') || serde_json::from_slice::<serde::de::IgnoredAny>(line).is_err() {
                continue;
            }
            out.write_all(if kept == 0 { b"\n" } else { b",\n" })?;
            out.write_all(line)?;
            kept += 1;
        }
        out.write_all(CLOSING)?;
        out.flush()?;
    }
    fs::rename(&tmp, &path)?;
    tracing::warn!("repaired {}: its last entry was cut", path.display());
    Ok(true)
}

/// The file ends with the closing line, and the line before it is the
/// header or an entry that parses.
fn is_whole(path: &Path) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let take = len.min(4 * 1024 * 1024);
    file.seek(SeekFrom::Start(len - take))?;
    let mut tail = Vec::with_capacity(take as usize);
    file.take(take).read_to_end(&mut tail)?;
    let Some(body) = tail.strip_suffix(CLOSING) else { return Ok(false) };
    let last = body.rsplit(|b| *b == b'\n').next().unwrap_or(&[]);
    if last.starts_with(b"{\"log\"") && last.ends_with(b"[") {
        return Ok(true);
    }
    Ok(serde_json::from_slice::<serde::de::IgnoredAny>(last).is_ok())
}

/// `20261002-093512` in local time.
fn local_stamp(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, which lives on this stack.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&secs, &mut tm).is_null() };
    if !ok {
        let t = time::OffsetDateTime::from(t);
        return format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.year(), u8::from(t.month()), t.day(), t.hour(), t.minute(), t.second());
    }
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const HEADER: &str = r#"{"log":{"version":"1.2","creator":{"name":"LocalRouter-dev","version":"9.9.9"},"pages":[],"entries":["#;
    const BIG: Limits = Limits { bytes: 20 * 1024 * 1024, entries: 5000 };

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_791_020_112 + secs)
    }

    fn line(i: usize) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"request": {"url": format!("http://a.example/{i}")}})).unwrap()
    }

    fn parse(path: &Path) -> serde_json::Value {
        let text = fs::read_to_string(path).unwrap();
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON ({e}): {text}", path.display()))
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut n: Vec<String> = fs::read_dir(folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        n.sort();
        n
    }

    #[test]
    fn file_names_are_ours_or_not() {
        assert_eq!(file_key("proxy-20261002-093512.har"), Some((20261002, 93512, 1)));
        assert_eq!(file_key("proxy-20261002-093512-2.har"), Some((20261002, 93512, 2)));
        for bad in ["proxy-old.har", "notes.txt", "proxy-2026100-093512.har", "proxy-20261002-093512-.har", "proxy-20261002-093512.har.bak", "../proxy-20261002-093512.har", "proxy-20261002-093512-2-3.har"] {
            assert_eq!(file_key(bad), None, "{bad}");
        }
        assert!(file_key("proxy-20261002-093512-2.har") > file_key("proxy-20261002-093512.har"), "-2 is newer");
    }

    // T2, I3: after each of 100 entries the file parses as HAR 1.2.
    #[test]
    fn valid_json_after_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = State::new(dir.path().to_path_buf());
        for i in 0..100 {
            let started = s.append(&line(i), at(0), BIG, HEADER).unwrap();
            assert_eq!(started.is_some(), i == 0);
            let path = dir.path().join(s.current_name().unwrap());
            let har = parse(&path);
            assert_eq!(har["log"]["version"], "1.2");
            assert_eq!(har["log"]["creator"]["name"], "LocalRouter-dev");
            assert_eq!(har["log"]["entries"].as_array().unwrap().len(), i + 1);
            let (_, len) = s.snapshot().unwrap();
            assert_eq!(len + CLOSING.len() as u64, fs::metadata(&path).unwrap().len());
        }
        let text = fs::read_to_string(dir.path().join(s.current_name().unwrap())).unwrap();
        assert_eq!(text.lines().count(), 102, "one entry per line");
    }

    // T2, I6: roll at file_requests, at file_mb; an entry bigger than the
    // limit is alone in its file.
    #[test]
    fn roll_at_either_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = State::new(dir.path().to_path_buf());
        let limits = Limits { bytes: 1 << 20, entries: 3 };
        for i in 0..7 {
            s.append(&line(i), at(i as u64), limits, HEADER).unwrap();
        }
        let files = names(dir.path());
        assert_eq!(files.len(), 3, "{files:?}");
        assert_eq!(parse(&dir.path().join(&files[0]))["log"]["entries"].as_array().unwrap().len(), 3);

        let dir = tempfile::tempdir().unwrap();
        let mut s = State::new(dir.path().to_path_buf());
        let limits = Limits { bytes: 400, entries: 5000 };
        let big = vec![b'"'; 1].into_iter().chain(vec![b'x'; 1000]).chain(vec![b'"']).collect::<Vec<u8>>();
        s.append(&line(0), at(0), limits, HEADER).unwrap();
        s.append(&big, at(1), limits, HEADER).unwrap();
        s.append(&line(2), at(2), limits, HEADER).unwrap();
        let files = names(dir.path());
        assert_eq!(files.len(), 3, "the big entry is alone: {files:?}");
        for f in &files {
            let len = fs::metadata(dir.path().join(f)).unwrap().len();
            let entries = parse(&dir.path().join(f))["log"]["entries"].as_array().unwrap().len();
            assert!(len <= 400 || entries == 1, "{f}: {len} bytes, {entries} entries");
        }
    }

    // T2, I5: prune keeps the 5 newest; other names stay.
    #[test]
    fn prune_keeps_five_and_never_touches_other_names() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("notes.txt"), "mine").unwrap();
        fs::write(dir.path().join("proxy-old.har"), "mine").unwrap();
        for i in 1..=6 {
            fs::write(dir.path().join(format!("proxy-20250101-00000{i}.har")), "x").unwrap();
        }
        let mut s = State::new(dir.path().to_path_buf());
        s.append(&line(0), at(0), BIG, HEADER).unwrap();
        let files = names(dir.path());
        assert!(files.contains(&"notes.txt".to_string()) && files.contains(&"proxy-old.har".to_string()), "{files:?}");
        let ours: Vec<&String> = files.iter().filter(|n| file_key(n).is_some()).collect();
        assert_eq!(ours.len(), 5, "{ours:?}");
        for gone in ["proxy-20250101-000001.har", "proxy-20250101-000002.har"] {
            assert!(!files.contains(&gone.to_string()), "{gone} kept: {files:?}");
        }
    }

    // T2: a name that exists gets -2.
    #[test]
    fn a_name_collision_gets_a_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = State::new(dir.path().to_path_buf());
        a.append(&line(0), at(0), BIG, HEADER).unwrap();
        let mut b = State::new(dir.path().to_path_buf());
        b.append(&line(0), at(0), BIG, HEADER).unwrap();
        let (na, nb) = (a.current_name().unwrap(), b.current_name().unwrap());
        assert_eq!(nb, na.replace(".har", "-2.har"));
    }

    // T2, I16: a file cut mid-line is repaired; a whole file is left as is.
    #[test]
    fn repair_a_cut_file_and_leave_a_whole_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = State::new(dir.path().to_path_buf());
        for i in 0..3 {
            s.append(&line(i), at(0), BIG, HEADER).unwrap();
        }
        let path = dir.path().join(s.current_name().unwrap());
        let whole = fs::read(&path).unwrap();
        assert!(!repair_newest(dir.path()).unwrap(), "a whole file is left as it is");
        assert_eq!(fs::read(&path).unwrap(), whole);

        // A crash in the middle of the 4th write.
        let cut = [&whole[..whole.len() - CLOSING.len()], b",\n{\"request\":{\"ur"].concat();
        fs::write(&path, &cut).unwrap();
        assert!(repair_newest(dir.path()).unwrap());
        let har = parse(&path);
        assert_eq!(har["log"]["entries"].as_array().unwrap().len(), 3);
        assert_eq!(fs::read(&path).unwrap(), whole, "the same bytes as before the cut");
        // A file with the header only is whole too.
        let empty = dir.path().join("proxy-20990101-000000.har");
        fs::write(&empty, [HEADER.as_bytes(), CLOSING].concat()).unwrap();
        assert!(!repair_newest(dir.path()).unwrap());
    }

    // T2: a new run never appends to an earlier file.
    #[test]
    fn a_new_run_starts_a_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = State::new(dir.path().to_path_buf());
        s.append(&line(0), at(0), BIG, HEADER).unwrap();
        let first = s.current_name().unwrap();
        let mut again = State::new(dir.path().to_path_buf());
        again.append(&line(1), at(0), BIG, HEADER).unwrap();
        assert_ne!(again.current_name().unwrap(), first);
        assert_eq!(parse(&dir.path().join(&first))["log"]["entries"].as_array().unwrap().len(), 1);
    }

    // T2, G9: the current file deleted: the next entry starts a new file.
    #[test]
    fn a_deleted_file_starts_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = State::new(dir.path().to_path_buf());
        s.append(&line(0), at(0), BIG, HEADER).unwrap();
        fs::remove_file(dir.path().join(s.current_name().unwrap())).unwrap();
        let started = s.append(&line(1), at(5), BIG, HEADER).unwrap();
        assert!(started.is_some());
        let har = parse(&dir.path().join(s.current_name().unwrap()));
        assert_eq!(har["log"]["entries"].as_array().unwrap().len(), 1);
    }

    // T2, I15: a folder that cannot be written gives an error, not a loop.
    #[test]
    fn a_read_only_folder_is_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("proxy");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o500)).unwrap();
        let mut s = State::new(folder.clone());
        let e = s.append(&line(0), at(0), BIG, HEADER).unwrap_err();
        assert!(e.contains("cannot create"), "{e}");
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).unwrap();
    }
}
