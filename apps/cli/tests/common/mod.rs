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
        Self::start_program(&build_daemon(), env)
    }

    /// Start `program`, for example the daemon copied as `localrouterd-dev`.
    pub fn start_program(daemon: &Path, env: &[(&str, &str)]) -> Self {
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
        self.cli_as(Path::new(env!("CARGO_BIN_EXE_localrouter")), args)
    }

    /// Run `program` (for example a CLI copied as `localrouter-dev`) against this daemon.
    pub fn cli_as(&self, program: &Path, args: &[&str]) -> (bool, String, String) {
        let out = Command::new(program)
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

/// ADR 04: the CLI copied under a suffixed name. The instance comes from the
/// program's own file name, so `localrouter-dev` is the `-dev` instance.
pub fn renamed_cli(suffix: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new().prefix("lrbin").tempdir().unwrap();
    let program = dir.path().join(format!("localrouter{suffix}"));
    std::fs::copy(env!("CARGO_BIN_EXE_localrouter"), &program).unwrap();
    (dir, program)
}

/// ADR 04: the daemon copied under a suffixed name (`localrouterd-dev`).
pub fn renamed_daemon(suffix: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new().prefix("lrbin").tempdir().unwrap();
    let program = dir.path().join(format!("localrouterd{suffix}"));
    std::fs::copy(build_daemon(), &program).unwrap();
    (dir, program)
}
