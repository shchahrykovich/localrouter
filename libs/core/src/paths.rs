//! Where LocalRouter keeps its files.
//!
//! `LOCALROUTER_HOME` replaces both folders, so tests never touch `~/Library`.

use std::path::PathBuf;

pub const HOME_ENV: &str = "LOCALROUTER_HOME";

/// macOS limits a Unix socket path to 104 bytes including the final NUL.
pub const MAX_SOCKET_PATH: usize = 103;

#[derive(Debug, Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub logs: PathBuf,
}

impl Paths {
    /// Paths from `LOCALROUTER_HOME`, or the standard macOS folders.
    pub fn from_env() -> Self {
        match std::env::var_os(HOME_ENV) {
            Some(home) if !home.is_empty() => Self::under(PathBuf::from(home)),
            _ => {
                let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
                Self {
                    data: home.join("Library/Application Support/LocalRouter"),
                    logs: home.join("Library/Logs/LocalRouter"),
                }
            }
        }
    }

    /// All files under one folder (used for `LOCALROUTER_HOME` and tests).
    pub fn under(root: PathBuf) -> Self {
        Self { logs: root.join("logs"), data: root }
    }

    pub fn config(&self) -> PathBuf {
        self.data.join("config.json")
    }
    pub fn routes(&self) -> PathBuf {
        self.data.join("routes.json")
    }
    pub fn socket(&self) -> PathBuf {
        self.data.join("daemon.sock")
    }
    pub fn lock(&self) -> PathBuf {
        self.data.join("daemon.lock")
    }
    pub fn ca_dir(&self) -> PathBuf {
        self.data.join("ca")
    }
    pub fn ca_pem(&self) -> PathBuf {
        self.ca_dir().join("ca.pem")
    }
    pub fn ca_key(&self) -> PathBuf {
        self.ca_dir().join("ca.key")
    }
    pub fn daemon_log(&self) -> PathBuf {
        self.logs.join("daemon.log")
    }

    /// Error text when the socket path is too long for macOS, else `None`.
    pub fn socket_path_problem(&self) -> Option<String> {
        let len = self.socket().as_os_str().len();
        (len > MAX_SOCKET_PATH).then(|| {
            format!(
                "socket path {} is {len} bytes; macOS allows at most {MAX_SOCKET_PATH}. Use a shorter {HOME_ENV}.",
                self.socket().display()
            )
        })
    }
}
