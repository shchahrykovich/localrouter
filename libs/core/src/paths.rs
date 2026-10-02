//! Where LocalRouter keeps its files.
//!
//! `LOCALROUTER_HOME` replaces both folders, so tests never touch `~/Library`.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::instance::Instance;

pub const HOME_ENV: &str = "LOCALROUTER_HOME";

/// macOS limits a Unix socket path to 104 bytes including the final NUL.
pub const MAX_SOCKET_PATH: usize = 103;

#[derive(Debug, Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub logs: PathBuf,
    /// Files other programs write for this instance: the Chrome profile of
    /// the proxy (ADR 06). Never written by the daemon.
    pub caches: PathBuf,
}

impl Paths {
    /// Paths from `LOCALROUTER_HOME`, or the macOS folders of this instance.
    pub fn from_env(instance: &Instance) -> Self {
        Self::resolve(instance, std::env::var_os(HOME_ENV), std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default())
    }

    /// `LOCALROUTER_HOME` wins over the instance, so tests and `cargo run`
    /// never touch `~/Library` (ADR 04). Otherwise each instance has its own
    /// folders: `LocalRouter` for the release, `LocalRouter-dev` for `-dev`.
    pub fn resolve(instance: &Instance, local_home: Option<OsString>, home: PathBuf) -> Self {
        match local_home {
            Some(dir) if !dir.is_empty() => Self::under(PathBuf::from(dir)),
            _ => Self {
                data: home.join(instance.data_folder()),
                logs: home.join(instance.logs_folder()),
                caches: home.join(instance.caches_folder()),
            },
        }
    }

    /// All files under one folder (used for `LOCALROUTER_HOME` and tests).
    pub fn under(root: PathBuf) -> Self {
        Self { logs: root.join("logs"), caches: root.join("caches"), data: root }
    }

    pub fn config(&self) -> PathBuf {
        self.data.join("config.json")
    }
    pub fn routes(&self) -> PathBuf {
        self.data.join("routes.json")
    }
    /// Persistent script rules (ADR 07).
    pub fn script_rules(&self) -> PathBuf {
        self.data.join("script-rules.json")
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
    /// The inspection CA of the forward proxy (ADR 06), made on first need.
    pub fn inspect_ca_dir(&self) -> PathBuf {
        self.data.join("inspect-ca")
    }
    pub fn inspect_ca_pem(&self) -> PathBuf {
        self.inspect_ca_dir().join("ca.pem")
    }
    pub fn inspect_ca_key(&self) -> PathBuf {
        self.inspect_ca_dir().join("ca.key")
    }
    /// The macOS system roots plus the inspection CA, for programs whose CA
    /// setting replaces the default roots (ADR 08, change 3). Public only.
    pub fn inspect_ca_bundle(&self) -> PathBuf {
        self.inspect_ca_dir().join("bundle.pem")
    }
    /// The HAR files of the proxy log (ADR 08).
    pub fn proxy_log_dir(&self) -> PathBuf {
        self.logs.join("proxy")
    }
    /// The separate Chrome profile that uses the proxy (ADR 06).
    pub fn chrome_profile(&self) -> PathBuf {
        self.caches.join("chrome-proxy")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_instance_has_its_own_folders() {
        let home = PathBuf::from("/Users/u");
        let dev = Paths::resolve(&Instance::new("-dev").unwrap(), None, home.clone());
        assert_eq!(dev.data, PathBuf::from("/Users/u/Library/Application Support/LocalRouter-dev"));
        assert_eq!(dev.logs, PathBuf::from("/Users/u/Library/Logs/LocalRouter-dev"));
        assert_eq!(dev.chrome_profile(), PathBuf::from("/Users/u/Library/Caches/LocalRouter-dev/chrome-proxy"));
        let release = Paths::resolve(&Instance::release(), None, home);
        assert_eq!(release.data, PathBuf::from("/Users/u/Library/Application Support/LocalRouter"));
        assert_eq!(release.logs, PathBuf::from("/Users/u/Library/Logs/LocalRouter"));
    }

    #[test]
    fn localrouter_home_wins_over_the_instance() {
        for suffix in ["", "-dev"] {
            let p = Paths::resolve(&Instance::new(suffix).unwrap(), Some("/tmp/lr1".into()), PathBuf::from("/Users/u"));
            assert_eq!(p.data, PathBuf::from("/tmp/lr1"));
            assert_eq!(p.logs, PathBuf::from("/tmp/lr1/logs"));
        }
        // An empty LOCALROUTER_HOME counts as not set, as before.
        let p = Paths::resolve(&Instance::release(), Some("".into()), PathBuf::from("/Users/u"));
        assert_eq!(p.data, PathBuf::from("/Users/u/Library/Application Support/LocalRouter"));
    }

    /// With the longest suffix the socket path fits for user names up to 28
    /// characters; a longer one is refused at start with a clear message.
    #[test]
    fn the_longest_suffix_fits_user_names_up_to_28_characters() {
        let longest = Instance::new("-abcdefghijklmno").unwrap();
        let at = |n: usize| Paths::resolve(&longest, None, PathBuf::from(format!("/Users/{}", "u".repeat(n))));
        assert_eq!(at(28).socket_path_problem(), None, "{}", at(28).socket().display());
        assert!(at(29).socket_path_problem().is_some());
    }
}
