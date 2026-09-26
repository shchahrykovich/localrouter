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

// ADR 03, T9: path routes from the command line.

fn http_port(d: &Daemon) -> u64 {
    d.status()["http"]["port"].as_u64().unwrap()
}

/// GET through the daemon; returns the status and the body.
fn get_body(d: &Daemon, host: &str, path: &str) -> (u16, String) {
    let url = format!("http://127.0.0.1:{}{path}", http_port(d));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let r = reqwest::Client::new().get(url).header("host", host).send().await.unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    })
}

/// An upstream that answers every request with the path it received.
fn path_echo() -> u16 {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut s in l.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("?").to_string();
            let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{path}", path.len());
        }
    });
    port
}

#[test]
fn path_route_add_list_rm() {
    let d = Daemon::start();
    assert!(d.cli(&["add", "shop", "5173"]).0);
    let (ok, out, err) = d.cli(&["add", "shop", "3001", "--path", "/blog", "--note", "blog app"]);
    assert!(ok, "{err}");
    assert!(out.contains("shop.localhost/blog -> http://127.0.0.1:3001"), "{out}");
    assert!(out.contains("https://shop.localhost:") && out.contains("/blog"), "{out}");

    let (ok, out, _) = d.cli(&["list"]);
    assert!(ok);
    assert!(out.contains("/blog") && out.contains("blog app"), "{out}");

    // I28: removing the host alone leaves the path route, and says so.
    let (ok, out, err) = d.cli(&["rm", "shop"]);
    assert!(ok, "{err}");
    assert_eq!(out.trim(), "removed shop; shop/blog remains");
    let (ok, out, _) = d.cli(&["rm", "shop", "--path", "/blog"]);
    assert!(ok);
    assert_eq!(out.trim(), "removed shop/blog");
    let (ok, _, err) = d.cli(&["rm", "shop", "--path", "/blog"]);
    assert!(!ok);
    assert!(err.contains("no route named shop/blog"), "{err}");
}

#[test]
fn path_flag_errors_come_before_any_socket_call() {
    // No daemon runs here: these must fail on their own, not with "not running".
    let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_localrouter")).args(args).env("LOCALROUTER_HOME", dir.path()).output().unwrap();
        (out.status.success(), String::from_utf8_lossy(&out.stderr).to_string())
    };
    let (ok, err) = run(&["add", "shop", "8000", "--strip-path"]);
    assert!(!ok && err.contains("--strip-path needs --path"), "{err}");
    let (ok, err) = run(&["add", "db", "5432", "--tcp", "--path", "/x"]);
    assert!(!ok && err.contains("--path is only for HTTP routes"), "{err}");
}

#[test]
fn host_with_a_slash_gets_both_hints() {
    let d = Daemon::start();
    let (ok, _, err) = d.cli(&["add", "shop/blog", "3001"]);
    assert!(!ok);
    assert!(err.contains("\"shop-blog\"") && err.contains("host \"shop\" with path \"/blog\""), "{err}");
}

#[test]
fn which_and_logs_name_the_route() {
    let d = Daemon::start();
    let echo = path_echo();
    assert!(d.cli(&["add", "shop", &echo.to_string()]).0);
    assert!(d.cli(&["add", "feat-x.shop", &echo.to_string(), "--path", "/blog", "--session"]).0);

    let (ok, out, err) = d.cli(&["which", "https://feat-x.shop.localhost/products"]);
    assert!(ok, "{err}");
    assert!(out.starts_with(&format!("feat-x.shop.localhost/products -> shop -> http://127.0.0.1:{echo}")), "{out}");
    assert!(out.contains("fallback is on: try shop"), "{out}");
    let (_, out, _) = d.cli(&["which", "feat-x.shop/blog/1"]);
    assert!(out.starts_with("feat-x.shop.localhost/blog/1 -> feat-x.shop/blog"), "{out}");

    assert_eq!(get_body(&d, "feat-x.shop.localhost", "/blog/1"), (200, "/blog/1".into()));
    assert_eq!(get_body(&d, "feat-x.shop.localhost", "/products"), (200, "/products".into()));
    let (ok, out, _) = d.cli(&["logs", "shop"]);
    assert!(ok);
    assert!(out.contains("feat-x.shop.localhost/blog/1") && out.contains("route feat-x.shop/blog"), "{out}");
    assert!(out.contains("feat-x.shop.localhost/products") && out.contains("route shop\n"), "{out}");
}

#[test]
fn one_app_with_page_prefixes_and_a_file_prefix() {
    // Pattern B of ADR 03, change 3, as `next dev` needs it (manual test M1):
    // page prefixes and the file prefix all go to one app, path unchanged.
    let d = Daemon::start();
    let main = path_echo();
    let app = path_echo().to_string();
    assert!(d.cli(&["add", "shop", &main.to_string()]).0);
    for prefix in ["/products", "/brands", "/shop-assets"] {
        let (ok, _, err) = d.cli(&["add", "shop", &app, "--path", prefix]);
        assert!(ok, "{err}");
    }
    assert_eq!(get_body(&d, "shop.localhost", "/products/x"), (200, "/products/x".into()));
    assert_eq!(get_body(&d, "shop.localhost", "/brands"), (200, "/brands".into()));
    assert_eq!(get_body(&d, "shop.localhost", "/shop-assets/_next/hmr?id=1"), (200, "/shop-assets/_next/hmr?id=1".into()));
    let (_, out, _) = d.cli(&["which", "shop.localhost/shop-assets/_next/static/a.js"]);
    assert!(out.starts_with("shop.localhost/shop-assets/_next/static/a.js -> shop/shop-assets"), "{out}");
}

#[test]
fn strip_path_serves_a_server_that_answers_at_the_root() {
    let d = Daemon::start();
    let api = path_echo().to_string();
    let (ok, _, err) = d.cli(&["add", "shop", &api, "--path", "/api", "--strip-path"]);
    assert!(ok, "{err}");
    assert_eq!(get_body(&d, "shop.localhost", "/api/users?v=1"), (200, "/users?v=1".into()));
    assert_eq!(get_body(&d, "shop.localhost", "/api"), (200, "/".into()));
    let (_, out, _) = d.cli(&["list"]);
    assert!(out.contains("(strip)"), "{out}");
}
