//! Learn when an owner process exits: kqueue `EVFILT_PROC` + `NOTE_EXIT`.

use std::io;
use std::os::fd::RawFd;
use std::ptr;

use tokio::sync::mpsc;

pub struct PidWatch {
    kq: RawFd,
}

impl PidWatch {
    /// Start the watcher thread. Exited pids arrive on the returned channel.
    pub fn start() -> io::Result<(Self, mpsc::UnboundedReceiver<u32>)> {
        // SAFETY: kqueue() has no preconditions; a negative result is an error.
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return Err(io::Error::last_os_error());
        }
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::Builder::new().name("pidwatch".into()).spawn(move || wait_loop(kq, tx))?;
        Ok((Self { kq }, rx))
    }

    /// Watch `pid`. Fails with `ESRCH` when the process does not exist.
    pub fn watch(&self, pid: u32) -> io::Result<()> {
        let change = libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: ptr::null_mut(),
        };
        // SAFETY: one valid change record, no output events, no timeout.
        let r = unsafe { libc::kevent(self.kq, &change, 1, ptr::null_mut(), 0, ptr::null()) };
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
}

fn wait_loop(kq: RawFd, tx: mpsc::UnboundedSender<u32>) {
    let mut events: [libc::kevent; 16] = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: `events` has room for 16 records; no timeout (block).
        let n = unsafe { libc::kevent(kq, ptr::null(), 0, events.as_mut_ptr(), 16, ptr::null()) };
        if n < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            tracing::error!("kqueue failed: {}", io::Error::last_os_error());
            return;
        }
        for ev in &events[..n as usize] {
            if ev.filter == libc::EVFILT_PROC && ev.fflags & libc::NOTE_EXIT != 0 && tx.send(ev.ident as u32).is_err() {
                return;
            }
        }
    }
}

/// True when a process with this pid exists (it may belong to another user).
pub fn alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else { return false };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 only checks for existence and permission.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn exit_of_a_watched_process_is_reported() {
        let (watch, mut rx) = PidWatch::start().unwrap();
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        assert!(alive(pid));
        watch.watch(pid).unwrap();
        child.kill().unwrap();
        let got = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        assert_eq!(got, pid);
        let _ = child.wait();
    }

    #[test]
    fn watching_a_dead_pid_fails() {
        let (watch, _rx) = PidWatch::start().unwrap();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(watch.watch(pid).is_err());
        assert!(!alive(pid));
    }
}
