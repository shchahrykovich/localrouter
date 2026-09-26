//! Test helpers: a real daemon in a temp folder, on random ports.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Once;
use std::time::{Duration, Instant};

pub fn bin(name: &str) -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_localrouter")).with_file_name(name)
}

/// The CLI crate cannot depend on the daemon binary, so build it once here.
pub fn build_daemon() -> PathBuf {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = Command::new(cargo)
            .args(["build", "--quiet", "-p", "localrouterd"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("cargo build");
        assert!(status.success(), "building localrouterd failed");
    });
    bin("localrouterd")
}

pub struct Daemon {
    pub child: Child,
    pub dir: tempfile::TempDir,
}

impl Daemon {
    pub fn start() -> Self {
        Self::start_env(&[])
    }

    pub fn start_env(env: &[(&str, &str)]) -> Self {
        let daemon = build_daemon();
        let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"version":1,"http_port":0,"https_port":0,"fallback":true,"allow_lan":false,"log_size":100}"#,
        )
        .unwrap();
        let mut cmd = Command::new(daemon);
        cmd.env("LOCALROUTER_HOME", dir.path()).env("LOCALROUTER_LOG", "warn").stderr(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().unwrap();
        let d = Self { child, dir };
        let deadline = Instant::now() + Duration::from_secs(10);
        while std::os::unix::net::UnixStream::connect(d.home().join("daemon.sock")).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        d
    }

    pub fn home(&self) -> &Path {
        self.dir.path()
    }

    /// Run the CLI against this daemon.
    pub fn cli(&self, args: &[&str]) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_localrouter"))
            .args(args)
            .env("LOCALROUTER_HOME", self.home())
            .output()
            .unwrap();
        (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    }

    pub fn status(&self) -> serde_json::Value {
        let (ok, out, err) = self.cli(&["status", "--json"]);
        assert!(ok, "{err}");
        serde_json::from_str(&out).unwrap()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
