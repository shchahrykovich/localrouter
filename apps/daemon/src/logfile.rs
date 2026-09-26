//! `daemon.log`: daemon events (not requests), rotated at 5 MB to `daemon.log.1`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const MAX_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone)]
pub struct RotatingFile {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    path: PathBuf,
    file: File,
    size: u64,
}

impl RotatingFile {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        Ok(Self { inner: Arc::new(Mutex::new(Inner { path, file, size })) })
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self.inner.lock().unwrap();
        if inner.size + buf.len() as u64 > MAX_BYTES {
            let old = inner.path.with_extension("log.1");
            let _ = fs::rename(&inner.path, old);
            inner.file = OpenOptions::new().create(true).append(true).open(&inner.path)?;
            inner.size = 0;
        }
        let n = inner.file.write(buf)?;
        inner.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.lock().unwrap().file.flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RotatingFile {
    type Writer = RotatingFile;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
