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
        "set_script_rule" => check::<SetScriptRuleParams>(params, file),
        "remove_script_rule" => check::<IdParams>(params, file),
        "status" | "list_script_rules" | "list_routes" | "get_config" | "reset_ca" | "get_proxy" | "reset_inspect_ca" => {
            check::<Empty>(params, file)
        }
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
        "reset_ca" | "reset_inspect_ca" => check::<ResetCaResult>(result, file),
        "get_proxy" => check::<GetProxyResult>(result, file),
        "set_script_rule" => check::<SetScriptRuleResult>(result, file),
        "remove_script_rule" => check::<RemoveScriptRuleResult>(result, file),
        "list_script_rules" => check::<ListScriptRulesResult>(result, file),
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

/// ADR 03, I30: the examples carry the new fields, so both contract tests
/// (this one and the Swift one) check that they survive a round trip.
#[test]
fn examples_cover_path_routes() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../api/examples");
    let read = |file: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))).unwrap()
    };
    let has_path_route = |route: &Value| route["path"].is_string() && route["strip_path"] == Value::Bool(true);

    assert!(has_path_route(&read("register_route_path.request.json")["params"]), "request needs path and strip_path");
    assert!(has_path_route(&read("register_route_path.reply.json")["result"]["route"]), "reply needs path and strip_path");
    let listed = read("list_routes.reply.json");
    assert!(listed["result"]["routes"].as_array().unwrap().iter().any(has_path_route), "list_routes needs a path route");
    assert!(read("unregister_route_path.request.json")["params"]["path"].is_string());
    assert!(read("log.event.json")["entry"]["route"].is_string(), "a log entry needs route");
    assert!(read("get_logs.reply.json")["result"]["entries"][0]["route"].is_string());
}

/// ADR 06: the examples carry the proxy fields, so both contract tests check
/// that they survive a round trip.
#[test]
fn examples_cover_the_forward_proxy() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../api/examples");
    let read = |file: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))).unwrap()
    };
    assert!(read("status.reply.json")["result"]["proxy"]["inspect_ca"].is_object());
    assert!(read("get_config.reply.json")["result"]["inspect_hosts"].is_array());
    assert!(read("set_config_proxy.request.json")["params"]["inspect_hosts"].is_array());
    let proxy = read("get_proxy.reply.json");
    assert!(proxy["result"]["chrome_args"].as_array().unwrap().len() >= 2);
    assert!(proxy["result"]["env"]["NODE_EXTRA_CA_CERTS"].is_string());
    assert_eq!(read("log_proxy.event.json")["entry"]["via"], "proxy");
    let entries = read("get_logs.reply.json")["result"]["entries"].clone();
    assert!(entries.as_array().unwrap().iter().any(|e| e["mode"] == "tunnel" && e["bytes_in"].is_u64()));
}

/// The `http` log entry as API 1.2 knew it, before ADR 06.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[allow(dead_code)]
enum LogEntry12 {
    Http { time_ms: u64, method: String, host: String, path: String, status: u16, duration_ms: u64, route: Option<String> },
    Tcp { time_ms: u64, host: String, listen_port: u16, bytes_in: u64, bytes_out: u64, duration_ms: u64, failed: bool },
}

/// ADR 06, I16: an older client decodes proxy entries from a newer daemon,
/// because they keep `kind: "http"` and only add optional fields.
#[test]
fn proxy_log_entries_decode_with_the_1_2_shape() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../api/examples");
    let event: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("log_proxy.event.json")).unwrap()).unwrap();
    let old: LogEntry12 = serde_json::from_value(event["entry"].clone()).expect("1.2 decodes a proxy entry");
    assert!(matches!(old, LogEntry12::Http { status: 200, .. }));
    let logs: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("get_logs.reply.json")).unwrap()).unwrap();
    for entry in logs["result"]["entries"].as_array().unwrap() {
        serde_json::from_value::<LogEntry12>(entry.clone()).unwrap_or_else(|e| panic!("{entry}: {e}"));
    }
}

/// ADR 07, T14: the examples carry the script rule fields, so both contract
/// tests check that they survive a round trip.
#[test]
fn examples_cover_script_rules() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../api/examples");
    let read = |file: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))).unwrap()
    };
    let rules = read("get_proxy.reply.json")["result"]["script_rules"].clone();
    assert!(rules.as_array().unwrap().iter().any(|r| r["kind"] == "log" && r["last_error"]["message"].is_string()));
    assert!(rules.as_array().unwrap().iter().any(|r| r["kind"] == "intercept" && r["enabled"] == false));
    assert_eq!(read("set_script_rule_check.request.json")["params"]["check_only"], true);
    assert!(read("set_script_rule.reply.json")["result"]["inspect"]["ca_created"].is_boolean());
    let entry = read("log.event.json")["entry"].clone();
    assert!(entry["rules"].is_array() && entry["script_error"].is_string());
    assert!(read("get_config.reply.json")["result"].get("secret_headers").is_none());
}

/// `get_proxy` as API 1.3 knew it, before ADR 07.
#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct GetProxy13 {
    enabled: bool,
    url: String,
    port: u16,
    inspect_set: Vec<String>,
    notes: Vec<String>,
}

/// ADR 07, I19: an older client decodes `get_proxy` and log entries from a
/// newer daemon, because the new fields are optional.
#[test]
fn script_fields_decode_with_the_1_3_shapes() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../api/examples");
    let read = |file: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(dir.join(file)).unwrap()).unwrap() };
    serde_json::from_value::<GetProxy13>(read("get_proxy.reply.json")["result"].clone()).expect("1.3 decodes get_proxy");
    serde_json::from_value::<LogEntry12>(read("log.event.json")["entry"].clone()).expect("1.3 decodes a log entry with rules");
    // And a 1.3 daemon's get_proxy, without script_rules, decodes here.
    let mut old = read("get_proxy.reply.json")["result"].clone();
    old.as_object_mut().unwrap().remove("script_rules");
    let new: GetProxyResult = serde_json::from_value(old).unwrap();
    assert!(new.script_rules.is_empty());
}
