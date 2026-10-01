//! `routes.json` and `config.json`: load, and replace atomically.
//!
//! Only the daemon writes these files (invariant I1). A write goes to a temp
//! file, is fsynced, then renamed over the old file (I4).

use std::fs;
use std::io::Write;
use std::path::Path;

use localrouter_core::config::Config;
use localrouter_core::instance::Instance;
use localrouter_core::routes::Route;
use localrouter_core::scripts::rules::ScriptRule;
use serde::{Deserialize, Serialize};

/// Version 1 has routes without a path. Version 2 may have `path` and
/// `strip_path` (ADR 03). A file is written as version 1 whenever it can be, so
/// a daemon from before ADR 03 still reads it (I29).
pub const ROUTES_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
struct RoutesFile {
    version: u32,
    routes: Vec<Route>,
}

/// `script-rules.json` (ADR 07): persistent script rules only.
pub const SCRIPT_RULES_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct ScriptRulesFile {
    version: u32,
    rules: Vec<ScriptRule>,
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
        if !(1..=ROUTES_VERSION).contains(&file.version) {
            return Err(format!("unknown version {}", file.version));
        }
        Ok(file.routes)
    })
}

/// A broken file is moved aside, as for `routes.json`.
pub fn load_script_rules(path: &Path) -> Loaded<Vec<ScriptRule>> {
    load(path, |text| {
        let file: ScriptRulesFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if file.version != SCRIPT_RULES_VERSION {
            return Err(format!("unknown version {}", file.version));
        }
        Ok(file.rules)
    })
}

/// Persistent script rules, sorted by id, replaced atomically (ADR 01, I4).
pub fn save_script_rules(path: &Path, rules: &[ScriptRule]) -> std::io::Result<()> {
    let mut rules: Vec<ScriptRule> = rules.iter().filter(|r| r.persistent).cloned().collect();
    rules.sort_by(|a, b| a.id.cmp(&b.id));
    replace(path, &serde_json::to_vec_pretty(&ScriptRulesFile { version: SCRIPT_RULES_VERSION, rules })?)
}

/// The instance's defaults stand in for a missing or broken file, so a
/// suffixed daemon never falls back to ports 80 and 443 (ADR 04, I9).
pub fn load_config(path: &Path, instance: &Instance) -> Loaded<Config> {
    load_or(path, || Config::defaults_for(instance), |text| Config::parse(text, instance))
}

/// `config.json` as it is on disk now, for a change that must keep hand
/// edits (ADR 04, I8). `Ok(None)` when there is no file. Unlike
/// `load_config`, a file that does not parse is left where it is.
pub fn read_config(path: &Path, instance: &Instance) -> Result<Option<Config>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Config::parse(&text, instance).map(Some).map_err(|why| format!("{} does not parse: {why}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

fn load<T: Default>(path: &Path, parse: impl Fn(&str) -> Result<T, String>) -> Loaded<T> {
    load_or(path, T::default, parse)
}

fn load_or<T>(path: &Path, default: impl Fn() -> T, parse: impl Fn(&str) -> Result<T, String>) -> Loaded<T> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded { value: default(), problem: None },
        Err(e) => return Loaded { value: default(), problem: Some(format!("cannot read {}: {e}", path.display())) },
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
            Loaded { value: default(), problem: Some(problem) }
        }
    }
}

/// Persistent routes, sorted by host and path.
pub fn save_routes(path: &Path, routes: &[Route]) -> std::io::Result<()> {
    let mut routes = routes.to_vec();
    routes.sort_by_key(Route::key);
    let version = if routes.iter().any(|r| r.path.is_some() || r.strip_path) { 2 } else { 1 };
    let file = RoutesFile { version, routes };
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
            path: None,
            protocol: Protocol::Http,
            target: "http://127.0.0.1:5173".into(),
            listen_port: None,
            https_only: false,
            strip_path: false,
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

    fn version(path: &Path) -> serde_json::Value {
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path).unwrap()).unwrap()["version"].clone()
    }

    // ADR 03, T7, I29
    #[test]
    fn version_is_1_without_path_routes_and_2_with_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routes.json");
        save_routes(&path, &[route("shop")]).unwrap();
        assert_eq!(version(&path), 1);

        let blog = Route { path: Some("/blog".into()), ..route("shop") };
        save_routes(&path, &[blog.clone(), route("shop")]).unwrap();
        assert_eq!(version(&path), 2);
        let loaded = load_routes(&path);
        assert!(loaded.problem.is_none());
        assert_eq!(loaded.value, [route("shop"), blog], "sorted by host, then path");

        let strip = Route { path: Some("/api".into()), strip_path: true, ..route("shop") };
        save_routes(&path, &[strip]).unwrap();
        assert_eq!(version(&path), 2);

        save_routes(&path, &[route("shop")]).unwrap();
        assert_eq!(version(&path), 1, "back to version 1 when no path route is saved");
    }

    // ADR 03, T7
    #[test]
    fn versions_1_and_2_load_and_3_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routes.json");
        for v in [1, 2] {
            fs::write(&path, format!(r#"{{"version":{v},"routes":[{{"host":"shop","target":"http://127.0.0.1:1"}}]}}"#)).unwrap();
            let loaded = load_routes(&path);
            assert!(loaded.problem.is_none(), "version {v}");
            assert_eq!(loaded.value.len(), 1);
        }
        fs::write(&path, r#"{"version":3,"routes":[]}"#).unwrap();
        let loaded = load_routes(&path);
        assert!(loaded.problem.unwrap().contains("unknown version 3"));
        assert!(!path.exists());
    }

    // ADR 07, T13: persistent rules only, sorted, and back after a load.
    #[test]
    fn script_rules_round_trip_and_only_persistent_ones_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script-rules.json");
        let rule = |id: &str, persistent: bool| -> ScriptRule {
            serde_json::from_value(serde_json::json!({"id": id, "host": "a.example.com", "script": "/x.lua", "persistent": persistent}))
                .unwrap()
        };
        save_script_rules(&path, &[rule("b", true), rule("s", false), rule("a", true)]).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        let loaded = load_script_rules(&path);
        assert!(loaded.problem.is_none());
        assert_eq!(loaded.value.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        fs::write(&path, r#"{"version":2,"rules":[]}"#).unwrap();
        assert!(load_script_rules(&path).problem.unwrap().contains("unknown version 2"));
    }

    #[test]
    fn missing_file_is_empty_without_problem() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_config(&dir.path().join("config.json"), &Instance::release());
        assert_eq!(loaded.value, Config::default());
        assert!(loaded.problem.is_none());
    }

    fn dev() -> Instance {
        Instance::new("-dev").unwrap()
    }

    // ADR 04, T3, I9: no path gives a suffixed instance ports 80 and 443.
    #[test]
    fn a_suffixed_instance_gets_its_own_ports_when_the_file_is_missing_or_broken() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let missing = load_config(&path, &dev());
        assert_eq!((missing.value.http_port, missing.value.https_port), (7080, 7443));

        fs::write(&path, "{ not json").unwrap();
        let broken = load_config(&path, &dev());
        assert_eq!((broken.value.http_port, broken.value.https_port), (7080, 7443));
        assert!(broken.problem.unwrap().contains("moved to"));
        assert!(!path.exists(), "a broken file is moved aside, as before");

        fs::write(&path, r#"{"https_port": 7444}"#).unwrap();
        let partial = load_config(&path, &dev());
        assert_eq!((partial.value.http_port, partial.value.https_port), (7080, 7444));
    }

    // ADR 04, I8: set_config reads the file without moving a broken one aside.
    #[test]
    fn read_config_leaves_a_broken_file_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(read_config(&path, &dev()), Ok(None));
        fs::write(&path, "{ not json").unwrap();
        assert!(read_config(&path, &dev()).unwrap_err().contains("does not parse"));
        assert!(path.exists());
        fs::write(&path, r#"{"allow_lan": true}"#).unwrap();
        let c = read_config(&path, &dev()).unwrap().unwrap();
        assert!(c.allow_lan);
        assert_eq!(c.http_port, 7080);
    }
}
