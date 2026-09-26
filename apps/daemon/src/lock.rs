//! One daemon per data folder (invariant I8): `flock` on `daemon.lock`.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

/// Held for the life of the daemon; the OS releases it when the process ends.
pub struct InstanceLock {
    _file: File,
}

pub enum LockError {
    AlreadyRunning,
    Io(io::Error),
}

pub fn acquire(path: &Path) -> Result<InstanceLock, LockError> {
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(path).map_err(LockError::Io)?;
    // SAFETY: a valid open file descriptor.
    let r = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if r == 0 {
        return Ok(InstanceLock { _file: file });
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Err(LockError::AlreadyRunning)
    } else {
        Err(LockError::Io(err))
    }
}
