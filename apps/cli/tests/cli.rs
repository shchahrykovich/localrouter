//! T10 (CLI commands) and T11 (the CLI does not depend on the daemon crate).

mod common;

use std::process::Command;

use common::Daemon;

#[test]
fn add_list_rm_round_trip() {
    let d = Daemon::start();
    let (ok, out, err) = d.cli(&["add", "shop", "5173", "--note", "main dev server"]);
    assert!(ok, "{err}");
    assert!(out.contains("shop.localhost -> http://127.0.0.1:5173"), "{out}");
    assert!(out.contains("https://shop.localhost:"), "{out}");

    let (ok, out, _) = d.cli(&["list"]);
    assert!(ok);
    assert!(out.contains("down") && out.contains("shop.localhost") && out.contains("main dev server"), "{out}");

    let (ok, out, _) = d.cli(&["add", "shop", "5174"]);
    assert!(ok);
    assert!(out.contains("(replaced http://127.0.0.1:5173)"), "{out}");

    let (ok, out, _) = d.cli(&["rm", "shop"]);
    assert!(ok, "{out}");
    let (ok, _, err) = d.cli(&["rm", "shop"]);
    assert!(!ok);
    assert!(err.contains("no route named shop"), "{err}");
}

#[test]
fn cli_routes_are_persistent_unless_session() {
    let d = Daemon::start();
    assert!(d.cli(&["add", "a", "5000"]).0);
    assert!(d.cli(&["add", "b", "5001", "--session"]).0);
    let saved = std::fs::read_to_string(d.home().join("routes.json")).unwrap();
    assert!(saved.contains("\"a\"") && !saved.contains("\"b\""), "{saved}");
}

#[test]
fn tcp_route_from_the_cli() {
    let d = Daemon::start();
    let (ok, out, err) = d.cli(&["add", "db.shop", "55001", "--tcp"]);
    assert!(ok, "{err}");
    assert!(out.contains("db.shop.localhost -> tcp://127.0.0.1:55001"), "{out}");
    assert!(out.contains("db.shop.localhost:"), "{out}");
    let (ok, _, err) = d.cli(&["add", "web", "5173", "--listen", "15000"]);
    assert!(!ok);
    assert!(err.contains("--tcp"), "{err}");
}

#[test]
fn invalid_name_shows_the_slug_hint() {
    let d = Daemon::start();
    let (ok, _, err) = d.cli(&["add", "feat/login.shop", "5174"]);
    assert!(!ok);
    assert!(err.contains("feat-login.shop"), "{err}");
}

#[test]
fn status_and_ca_path() {
    let d = Daemon::start();
    let (ok, out, _) = d.cli(&["status"]);
    assert!(ok);
    assert!(out.contains("HTTPS  port") && out.contains("LocalRouter CA"), "{out}");
    let (ok, out, _) = d.cli(&["ca-path"]);
    assert!(ok);
    assert_eq!(out.trim(), d.home().join("ca/ca.pem").display().to_string());
}

#[test]
fn ca_reset_needs_confirmation() {
    let d = Daemon::start();
    let (ok, _, err) = d.cli(&["ca", "reset"]);
    assert!(!ok);
    assert!(err.contains("--yes"), "{err}");
}

#[test]
fn not_running_is_a_clear_error() {
    let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_localrouter")).arg("list").env("LOCALROUTER_HOME", dir.path()).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("LocalRouter is not running"));
}

#[test]
fn logs_show_requests() {
    let d = Daemon::start();
    let s = d.status();
    let http = s["http"]["port"].as_u64().unwrap();
    // A request for an unknown name still gets logged (404).
    let _ = reqwest_get(&format!("http://127.0.0.1:{http}/hello?secret=1"), "nothing.localhost");
    let (ok, out, _) = d.cli(&["logs"]);
    assert!(ok);
    assert!(out.contains("404") && out.contains("nothing.localhost/hello") && !out.contains("secret"), "{out}");
}

fn reqwest_get(url: &str, host: &str) -> u16 {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let client = reqwest::Client::builder().build().unwrap();
        client.get(url).header("host", host).send().await.map(|r| r.status().as_u16()).unwrap_or(0)
    })
}

// T11, I1: the CLI crate must not link the daemon crate (the daemon is the only writer).
#[test]
fn cli_does_not_depend_on_the_daemon_crate() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let cli = meta["packages"].as_array().unwrap().iter().find(|p| p["name"] == "localrouter").unwrap();
    let deps: Vec<&str> = cli["dependencies"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap()).collect();
    assert!(!deps.contains(&"localrouterd"), "{deps:?}");
}
