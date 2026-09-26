//! `routes.json` and `config.json`: load, and replace atomically.
//!
//! Only the daemon writes these files (invariant I1). A write goes to a temp
//! file, is fsynced, then renamed over the old file (I4).

use std::fs;
use std::io::Write;
use std::path::Path;

use localrouter_core::config::Config;
use localrouter_core::routes::Route;
use serde::{Deserialize, Serialize};

pub const ROUTES_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct RoutesFile {
    version: u32,
    routes: Vec<Route>,
}

/// What a load found.
#[derive(Debug, Default)]
pub struct Loaded<T> {
    pub value: T,
    /// Set when the file did not parse and was moved aside.
    pub problem: Option<String>,
}

pub fn load_routes(path: &Path) -> Loaded<Vec<Route>> {
    load(path, |text| {
        let file: RoutesFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if file.version != ROUTES_VERSION {
            return Err(format!("unknown version {}", file.version));
        }
        Ok(file.routes)
    })
}

pub fn load_config(path: &Path) -> Loaded<Config> {
    load(path, |text| serde_json::from_str(text).map_err(|e| e.to_string()))
}

fn load<T: Default>(path: &Path, parse: impl Fn(&str) -> Result<T, String>) -> Loaded<T> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::default(),
        Err(e) => return Loaded { value: T::default(), problem: Some(format!("cannot read {}: {e}", path.display())) },
    };
    match parse(&text) {
        Ok(value) => Loaded { value, problem: None },
        Err(why) => {
            let aside = path.with_extension(format!("json.bad-{}", localrouter_core::logs::now_ms() / 1000));
            let moved = fs::rename(path, &aside).is_ok();
            let problem = if moved {
                format!("{} did not load ({why}); it was moved to {}", path.display(), aside.display())
            } else {
                format!("{} did not load ({why})", path.display())
            };
            Loaded { value: T::default(), problem: Some(problem) }
        }
    }
}

/// Persistent routes, sorted by host.
pub fn save_routes(path: &Path, routes: &[Route]) -> std::io::Result<()> {
    let mut routes = routes.to_vec();
    routes.sort_by(|a, b| a.host.cmp(&b.host));
    let file = RoutesFile { version: ROUTES_VERSION, routes };
    replace(path, &serde_json::to_vec_pretty(&file)?)
}

pub fn save_config(path: &Path, config: &Config) -> std::io::Result<()> {
    replace(path, &serde_json::to_vec_pretty(config)?)
}

/// Write `data` to `path.tmp`, fsync, rename over `path`, fsync the folder.
pub fn replace(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        if let Some(dir) = path.parent() {
            fs::File::open(dir)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use localrouter_core::routes::Protocol;

    use super::*;

    fn route(host: &str) -> Route {
        Route {
            host: host.into(),
            protocol: Protocol::Http,
            target: "http://127.0.0.1:5173".into(),
            listen_port: None,
            https_only: false,
            note: String::new(),
            owner_pid: None,
            persistent: true,
        }
    }

    // T3
    #[test]
    fn save_writes_sorted_json_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routes.json");
        save_routes(&path, &[route("b"), route("a")]).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["routes"][0]["host"], "a");
        assert!(!dir.path().join("routes.json.tmp").exists());
        let loaded = load_routes(&path);
        assert_eq!(loaded.value.len(), 2);
        assert!(loaded.problem.is_none());
    }

    // T3, I4
    #[test]
    fn failed_save_keeps_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routes.json");
        save_routes(&path, &[route("a")]).unwrap();
        let before = fs::read(&path).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let result = save_routes(&path, &[route("a"), route("b")]);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(before, fs::read(&path).unwrap());
    }

    // T3
    #[test]
    fn broken_file_is_moved_aside_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routes.json");
        fs::write(&path, "{ not json").unwrap();
        let loaded = load_routes(&path);
        assert!(loaded.value.is_empty());
        assert!(loaded.problem.unwrap().contains("moved"));
        assert!(!path.exists());
        let aside: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("routes.json.bad-"))
            .collect();
        assert_eq!(aside.len(), 1);
    }

    #[test]
    fn missing_file_is_empty_without_problem() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_config(&dir.path().join("config.json"));
        assert_eq!(loaded.value, Config::default());
        assert!(loaded.problem.is_none());
    }
}
