//! T9 (Rust half), I11: every file in `api/examples/` decodes into its Rust type.
//!
//! The test walks the folder, so a new example is covered without an edit here;
//! an example with an unknown name fails, so it cannot be skipped silently.
//! The Swift app has the same test over the same folder.

use std::path::PathBuf;

use localrouter_core::api::*;
use localrouter_core::config::Config;
use localrouter_core::routes::Route;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn check<T: DeserializeOwned + serde::Serialize>(value: Value, file: &str) {
    let typed: T = serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{file}: {e}"));
    // Round trip: nothing the example says is lost by the Rust type.
    let back = serde_json::to_value(&typed).unwrap();
    assert_eq!(strip_nulls(value), strip_nulls(back), "{file}: round trip differs");
}

/// `null` and a missing field mean the same; empty `note` is not written.
fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, v)| !v.is_null() && !(k == "listen_failed" && v == &Value::Bool(false)))
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(strip_nulls).collect()),
        other => other,
    }
}

fn params(method: &str, params: Value, file: &str) {
    match method {
        "hello" => check::<HelloParams>(params, file),
        "register_route" => check::<Route>(params, file),
        "unregister_route" => check::<HostParams>(params, file),
        "find_free_port" => check::<FindFreePortParams>(params, file),
        "get_logs" => check::<GetLogsParams>(params, file),
        "subscribe_logs" => check::<SubscribeLogsParams>(params, file),
        "set_config" => check::<SetConfigParams>(params, file),
        "status" | "list_routes" | "get_config" | "reset_ca" => check::<Empty>(params, file),
        other => panic!("{file}: unknown method {other}"),
    }
}

fn result(method: &str, result: Value, file: &str) {
    match method {
        "hello" => check::<HelloResult>(result, file),
        "status" => check::<StatusResult>(result, file),
        "register_route" => check::<RegisterRouteResult>(result, file),
        "unregister_route" => check::<UnregisterRouteResult>(result, file),
        "list_routes" => check::<ListRoutesResult>(result, file),
        "find_free_port" => check::<FindFreePortResult>(result, file),
        "get_logs" => check::<GetLogsResult>(result, file),
        "subscribe_logs" => check::<SubscribeLogsResult>(result, file),
        "get_config" => check::<Config>(result, file),
        "set_config" => check::<SetConfigResult>(result, file),
        "reset_ca" => check::<ResetCaResult>(result, file),
        other => panic!("{file}: unknown method {other}"),
    }
}

#[test]
fn every_example_decodes() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../api/examples");
    let mut methods_seen = std::collections::BTreeSet::new();
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        if !file.ends_with(".json") {
            continue;
        }
        count += 1;
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stem = file.trim_end_matches(".json");
        let (name, kind) = stem.rsplit_once('.').unwrap_or_else(|| panic!("{file}: name it <method>.<kind>.json"));
        match kind {
            "request" => {
                let req: Request = serde_json::from_value(value).unwrap_or_else(|e| panic!("{file}: {e}"));
                methods_seen.insert(req.method.clone());
                params(&req.method, req.params, &file);
            }
            "reply" if name == "error" => {
                let resp: Response = serde_json::from_value(value).unwrap();
                assert!(resp.error.is_some() && resp.result.is_none(), "{file}");
            }
            "reply" => {
                let resp: Response = serde_json::from_value(value).unwrap_or_else(|e| panic!("{file}: {e}"));
                // register_route_tcp → register_route
                let method = METHODS
                    .iter()
                    .find(|m| name == **m || name.starts_with(&format!("{m}_")))
                    .unwrap_or_else(|| panic!("{file}: unknown method {name}"));
                result(method, resp.result.unwrap(), &file);
            }
            "event" => check::<Event>(value, &file),
            other => panic!("{file}: unknown kind {other}"),
        }
    }
    assert!(count >= 20, "only {count} examples");
    for m in METHODS {
        assert!(methods_seen.contains(m), "no request example for {m}");
    }
}
