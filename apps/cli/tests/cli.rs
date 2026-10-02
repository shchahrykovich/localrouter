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

// ADR 04, T7: a CLI named localrouter-dev is the -dev instance.

#[test]
fn a_dev_cli_names_itself_in_help() {
    let (_bin, dev) = common::renamed_cli("-dev");
    let out = Command::new(&dev).arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("Usage: localrouter-dev"), "{help}");
}

#[test]
fn a_dev_cli_status_names_the_instance_and_its_folder() {
    let d = Daemon::start();
    let (_bin, dev) = common::renamed_cli("-dev");
    let (ok, out, err) = d.cli_as(&dev, &["status"]);
    assert!(ok, "{err}");
    let first = out.lines().next().unwrap();
    assert!(first.starts_with("LocalRouter-dev "), "{out}");
    assert!(first.contains(&d.home().display().to_string()), "{out}");
    assert!(first.ends_with("(from LOCALROUTER_HOME)"), "{out}");
}

#[test]
fn guide_prints_the_help_page_with_the_bound_ports_and_routes() {
    let d = Daemon::start();
    let (_bin, dev) = common::renamed_cli("-dev");
    let (ok, _, err) = d.cli_as(&dev, &["add", "shop", "5173"]);
    assert!(ok, "{err}");
    let http = d.status()["http"]["port"].as_u64().unwrap();
    let (ok, guide, err) = d.cli_as(&dev, &["guide"]);
    assert!(ok, "{err}");
    assert!(guide.contains(&format!("curl -s http://router.localhost:{http}`")), "{guide}");
    assert!(guide.contains("localrouter-dev add shop 5173"), "{guide}");
    assert!(guide.contains("shop.localhost"), "the route list: {guide}");
}

#[test]
fn guide_works_without_a_daemon() {
    let home = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_localrouter"))
        .arg("guide")
        .env("LOCALROUTER_HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let guide = String::from_utf8_lossy(&out.stdout);
    assert!(guide.contains("Status is not available."), "{guide}");
    assert!(guide.contains("curl -s http://router.localhost`"), "{guide}");
}

#[test]
fn note_prints_the_claude_note_of_the_instance() {
    let (_bin, dev) = common::renamed_cli("-dev");
    let out = Command::new(&dev).arg("note").output().unwrap();
    let note = String::from_utf8_lossy(&out.stdout);
    assert!(note.starts_with("# LocalRouter-dev"), "{note}");
    assert!(note.contains("localrouter-dev guide"), "{note}");
}

#[test]
fn a_cli_with_a_bad_suffix_stops_before_it_makes_a_folder() {
    let (_bin, bad) = common::renamed_cli("-Dev");
    let home = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    let data = home.path().join("data");
    let out = Command::new(&bad).args(["status"]).env("LOCALROUTER_HOME", &data).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid instance suffix"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!data.exists());
}

// Folder routes from the command line.

#[test]
fn folder_route_from_a_relative_path() {
    let d = Daemon::start();
    let work = tempfile::Builder::new().prefix("lr-work").tempdir().unwrap();
    std::fs::create_dir(work.path().join("dist")).unwrap();
    std::fs::write(work.path().join("dist/index.html"), "<h1>built</h1>").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_localrouter"))
        .args(["add", "docs", "--folder", "dist", "--session"])
        .current_dir(work.path())
        .env("LOCALROUTER_HOME", d.home())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let real = work.path().join("dist").canonicalize().unwrap();
    assert!(text.contains(&format!("docs.localhost -> file://{}", real.display())), "the CLI sends an absolute path: {text}");

    let (ok, out, _) = d.cli(&["list"]);
    assert!(ok && out.starts_with("up"), "{out}");
    assert_eq!(get_body(&d, "docs.localhost", "/"), (200, "<h1>built</h1>".to_string()));

    let (ok, _, err) = d.cli(&["add", "docs", "--folder", "/tmp", "--tcp"]);
    assert!(!ok && err.contains("cannot be used with"), "{err}");
    let (ok, _, err) = d.cli(&["add", "docs", "--folder", "/no/such/folder"]);
    assert!(!ok && err.contains("/no/such/folder"), "{err}");
    let (ok, _, err) = d.cli(&["add", "docs"]);
    assert!(!ok && err.contains("--folder"), "{err}");
}

// ---- ADR 06, T10: proxy commands. Port 0 first: tests never take 8877.

fn proxy_on(d: &Daemon) -> String {
    let (ok, _, err) = d.cli(&["proxy", "port", "0"]);
    assert!(ok, "{err}");
    let (ok, out, err) = d.cli(&["proxy", "on"]);
    assert!(ok, "{err}");
    assert!(out.contains("The proxy is on: http://127.0.0.1:"), "{out}");
    out
}

#[test]
fn proxy_on_shows_the_url_and_off_warns_about_running_programs() {
    let d = Daemon::start();
    let (ok, out, _) = d.cli(&["proxy"]);
    assert!(ok);
    assert!(out.contains("off (port 8877)"), "the release default port: {out}");
    proxy_on(&d);
    let (ok, out, _) = d.cli(&["proxy"]);
    assert!(ok);
    assert!(out.contains("on: http://127.0.0.1:"), "{out}");
    assert!(out.contains("none: every CONNECT is a tunnel"), "{out}");

    let (ok, out, err) = d.cli(&["proxy", "inspect", "add", "api.example.com"]);
    assert!(ok, "{err}");
    let (_, out2, _) = d.cli(&["proxy"]);
    assert!(out2.contains("Inspected      api.example.com"), "{out2}");
    // The temp CA is not in the keychain.
    assert!(out.contains("not trusted") || out2.contains("NOT trusted"), "{out}{out2}");

    let (ok, out, _) = d.cli(&["proxy", "off"]);
    assert!(ok);
    assert!(out.contains("now fail to connect. Restart them without it."), "{out}");
}

#[test]
fn proxy_env_prints_the_exports_then_the_ca_names() {
    let d = Daemon::start();
    proxy_on(&d);
    let (ok, out, _) = d.cli(&["proxy", "env"]);
    assert!(ok);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 7, "{out}");
    assert!(lines[0].starts_with("export HTTPS_PROXY=http://127.0.0.1:"), "{out}");
    assert!(lines[1].starts_with("export HTTP_PROXY="));
    assert!(lines[2].contains("NO_PROXY=") && lines[2].contains(".localhost"), "{out}");
    assert_eq!(lines[3], "export NODE_USE_ENV_PROXY=1");
    // ADR 08: curl reads a plain-http proxy only from the lowercase name.
    assert!(lines[4].starts_with("export https_proxy=http://127.0.0.1:"), "{out}");
    assert!(lines[5].starts_with("export http_proxy="));
    assert!(lines[6].starts_with("export no_proxy="));

    assert!(d.cli(&["proxy", "inspect", "add", "api.example.com"]).0);
    let (_, out, _) = d.cli(&["proxy", "env"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 11, "{out}");
    let (_, ca_path, _) = d.cli(&["proxy", "ca-path"]);
    assert_eq!(lines[4], format!("export NODE_EXTRA_CA_CERTS={}", ca_path.trim()));
    // ADR 08, I23: Python and curl get the bundle, never the CA alone.
    let bundle = d.home().join("inspect-ca/bundle.pem");
    for (i, name) in ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"].iter().enumerate() {
        assert_eq!(lines[5 + i], format!("export {name}={}", bundle.display()), "{out}");
    }
}

// ADR 08, T10: `proxy log` prints the state; on, off, limits, path.
#[test]
fn proxy_log_commands() {
    let d = Daemon::start();
    let folder = d.home().join("logs/proxy");
    let (ok, out, _) = d.cli(&["proxy", "log"]);
    assert!(ok);
    assert!(out.contains(&format!("Log        on, writes every proxied request to {}", folder.display())), "{out}");
    assert!(out.contains("The proxy is off, so nothing is written"), "{out}");
    assert!(out.contains("Viewer     http://proxy.localhost:"), "{out}");
    assert!(out.contains("20 MB or 5000 requests per file"), "{out}");

    let (ok, out, _) = d.cli(&["proxy", "log", "off"]);
    assert!(ok && out.contains("Log        off"), "{out}");
    let (ok, out, _) = d.cli(&["proxy", "log", "on"]);
    assert!(ok && out.contains("Log        on"), "{out}");
    let (ok, out, _) = d.cli(&["proxy", "log", "limits", "--mb", "5", "--requests", "200"]);
    assert!(ok && out.contains("5 MB or 200 requests per file"), "{out}");
    let (ok, _, err) = d.cli(&["proxy", "log", "limits", "--mb", "500"]);
    assert!(!ok && err.contains("1 to 200"), "{err}");
    let (ok, _, err) = d.cli(&["proxy", "log", "limits"]);
    assert!(!ok && err.contains("--mb"), "{err}");

    let (ok, out, _) = d.cli(&["proxy", "log", "path"]);
    assert!(ok);
    assert_eq!(out.trim(), folder.display().to_string());
}

// ADR 08, T10, G2: `proxy on` and `proxy` say where the log goes.
#[test]
fn proxy_on_prints_the_log_line() {
    let d = Daemon::start();
    let out = proxy_on(&d);
    let folder = d.home().join("logs/proxy");
    assert!(out.contains(&format!("Log: on, writes every request to {}", folder.display())), "{out}");
    assert!(out.contains("proxy log off to stop"), "{out}");
    let (_, out, _) = d.cli(&["proxy"]);
    assert!(out.contains("Log: on, writes every request to"), "{out}");
}

#[test]
fn proxy_inspect_add_rm_and_a_bad_pattern() {
    let d = Daemon::start();
    assert!(d.cli(&["proxy", "inspect", "add", "api.example.com"]).0);
    assert!(d.cli(&["proxy", "inspect", "add", "API.example.com"]).0);
    let (_, out, _) = d.cli(&["proxy", "inspect", "list"]);
    assert_eq!(out, "api.example.com\n", "listed once");
    let (ok, _, err) = d.cli(&["proxy", "inspect", "add", "*.com"]);
    assert!(!ok);
    assert!(err.contains("top-level domain"), "{err}");
    assert!(d.cli(&["proxy", "inspect", "rm", "api.example.com"]).0);
    let (_, out, _) = d.cli(&["proxy", "inspect", "list"]);
    assert!(out.contains("No inspected hosts"), "{out}");
    let (ok, _, err) = d.cli(&["proxy", "inspect", "rm", "api.example.com"]);
    assert!(!ok);
    assert!(err.contains("not in the inspect list"), "{err}");
}

// I18: the printed Chrome command uses exactly the daemon's chrome_args.
#[test]
fn proxy_chrome_print_shows_the_proxy_flag_and_a_separate_profile() {
    let d = Daemon::start();
    let (ok, out, err) = d.cli(&["proxy", "chrome", "--print"]);
    assert!(ok, "{err}");
    assert!(out.starts_with("open -na 'Google Chrome' --args --user-data-dir="), "{out}");
    assert!(out.contains("--proxy-server=http://127.0.0.1:8877"), "{out}");
    assert!(out.contains("chrome-proxy"), "{out}");
    assert!(!out.contains("Google/Chrome"), "never the default profile: {out}");
    assert!(!d.status()["proxy"]["enabled"].as_bool().unwrap(), "--print changes nothing");
}

#[test]
fn proxy_trust_before_the_ca_exists_says_how_to_make_it() {
    let d = Daemon::start();
    let (ok, _, err) = d.cli(&["proxy", "trust"]);
    assert!(!ok);
    assert!(err.contains("proxy inspect add"), "{err}");
}

// ---- ADR 07: script rules (T15)

/// A folder with `cap.lua`, `bad.lua` and `out/`; the CLI runs inside it, so
/// relative paths are relative to it.
fn script_folder() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("lrs").tempdir().unwrap();
    std::fs::write(dir.path().join("cap.lua"), r#"return { kind = "log", on_exchange = function(ex) end }"#).unwrap();
    std::fs::write(dir.path().join("bad.lua"), "return {\n  kind = 'log',\n  on_exchange = function(ex) x = end }").unwrap();
    std::fs::create_dir(dir.path().join("out")).unwrap();
    dir
}

fn cli_in(d: &Daemon, cwd: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_localrouter"))
        .args(args)
        .current_dir(cwd)
        .env("LOCALROUTER_HOME", d.home())
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn rules_add_makes_paths_absolute_and_lists_the_rule() {
    let d = Daemon::start();
    let dir = script_folder();
    let real = dir.path().canonicalize().unwrap();
    let (code, out, err) = cli_in(&d, &real, &["rules", "add", "cap", "--host", "api.test.example", "--script", "./cap.lua", "--output-dir", "./out"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("set cap (log) on api.test.example"), "{out}");
    assert!(out.contains("is now inspected"), "{out}");
    let (_, json, _) = cli_in(&d, &real, &["rules", "--json"]);
    let list: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(list["rules"][0]["script"], real.join("cap.lua").display().to_string());
    assert_eq!(list["rules"][0]["output_dir"], real.join("out").display().to_string());
    let (_, out, _) = cli_in(&d, &real, &["rules"]);
    assert!(out.contains("on   log") && out.contains("cap") && out.contains("(session)"), "{out}");
}

#[test]
fn rules_check_prints_the_file_and_line_of_a_mistake() {
    let d = Daemon::start();
    let dir = script_folder();
    let (code, _, err) = cli_in(&d, dir.path(), &["rules", "check", "./bad.lua"]);
    assert_eq!(code, 1);
    assert!(err.contains("bad.lua:3:"), "{err}");
    let (code, out, err) = cli_in(&d, dir.path(), &["rules", "check", "./cap.lua"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("ok, a log script"), "{out}");
    assert!(out.contains("give output_dir"), "{out}");
    let (_, json, _) = cli_in(&d, dir.path(), &["rules", "--json"]);
    assert!(json.contains("\"rules\": []"), "check stores nothing: {json}");
}

#[test]
fn rules_disable_enable_and_rm() {
    let d = Daemon::start();
    let dir = script_folder();
    assert_eq!(cli_in(&d, dir.path(), &["rules", "add", "cap", "--host", "shop.localhost", "--script", "cap.lua", "--output-dir", "out"]).0, 0);
    let (code, out, _) = cli_in(&d, dir.path(), &["rules", "disable", "cap"]);
    assert_eq!((code, out.trim()), (0, "cap is off"));
    let (_, out, _) = cli_in(&d, dir.path(), &["rules"]);
    assert!(out.starts_with("off"), "{out}");
    assert_eq!(cli_in(&d, dir.path(), &["rules", "enable", "cap"]).1.trim(), "cap is on");
    assert_eq!(cli_in(&d, dir.path(), &["rules", "rm", "cap"]).1.trim(), "removed cap");
    let (code, _, err) = cli_in(&d, dir.path(), &["rules", "rm", "cap"]);
    assert_eq!(code, 1);
    assert!(err.contains("no script rule named cap"), "{err}");
}

#[test]
fn rules_api_prints_the_reference_with_this_instances_names() {
    let d = Daemon::start();
    let (ok, out, _) = d.cli(&["rules", "api"]);
    assert!(ok);
    assert!(out.starts_with("# LocalRouter scripts"), "{out}");
    assert!(out.contains("localrouter rules check"));
    let (_bin, dev) = common::renamed_cli("-dev");
    let (ok, out, _) = d.cli_as(&dev, &["rules", "api"]);
    assert!(ok);
    assert!(out.contains("localrouter-dev rules check") && out.contains("router.localhost:7080/scripts"), "{out}");
}
