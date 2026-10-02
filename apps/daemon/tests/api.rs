//! T6 and T12 (listener cases): the real `localrouterd` binary in a temp folder.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
}

impl Daemon {
    fn start() -> Self {
        Self::start_with(|_| {})
    }

    /// Start with ports 0 and nothing inspected (the default is `*`); `prepare`
    /// may write files into the folder first.
    fn start_with(prepare: impl FnOnce(&Path)) -> Self {
        Self::start_program(Path::new(env!("CARGO_BIN_EXE_localrouterd")), |dir| {
            std::fs::write(
                dir.join("config.json"),
                json!({"version":1,"http_port":0,"https_port":0,"fallback":true,"allow_lan":false,"log_size":100,"inspect_hosts":[]})
                    .to_string(),
            )
            .unwrap();
            prepare(dir);
        })
    }

    /// Start `program` with a fresh LOCALROUTER_HOME that `prepare` fills.
    fn start_program(program: &Path, prepare: impl FnOnce(&Path)) -> Self {
        Self::start_program_env(program, prepare, &[])
    }

    /// [`Daemon::start_program`] with extra environment variables.
    fn start_program_env(program: &Path, prepare: impl FnOnce(&Path), env: &[(&str, &str)]) -> Self {
        let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
        prepare(dir.path());
        let mut cmd = Command::new(program);
        cmd.env("LOCALROUTER_HOME", dir.path()).env("LOCALROUTER_LOG", "warn").stderr(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().unwrap();
        let d = Daemon { child, dir };
        let deadline = Instant::now() + Duration::from_secs(10);
        while UnixStream::connect(d.socket()).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        d
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    fn client(&self) -> Client {
        let stream = UnixStream::connect(self.socket()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        Client { reader: BufReader::new(stream.try_clone().unwrap()), stream, next: 1 }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next: u64,
}

impl Client {
    /// Full reply line as JSON.
    fn raw(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string() + "\n";
        self.stream.write_all(line.as_bytes()).unwrap();
        loop {
            let mut reply = String::new();
            self.reader.read_line(&mut reply).unwrap();
            let v: Value = serde_json::from_str(&reply).unwrap();
            if v.get("event").is_none() {
                assert_eq!(v["id"], id);
                return v;
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let v = self.raw(method, params);
        assert!(v.get("error").is_none(), "{method} failed: {v}");
        v["result"].clone()
    }

    fn error_code(&mut self, method: &str, params: Value) -> String {
        let v = self.raw(method, params);
        v["error"]["code"].as_str().unwrap_or_else(|| panic!("{method} did not fail: {v}")).to_string()
    }
}

fn http_route(host: &str, port: u16) -> Value {
    json!({"host": host, "target": format!("http://127.0.0.1:{port}")})
}

#[test]
fn hello_returns_api_version_and_refuses_a_different_major() {
    let d = Daemon::start();
    let mut c = d.client();
    let r = c.call("hello", json!({"client": "test", "api_version": "1.0"}));
    assert!(r["api_version"].as_str().unwrap().starts_with("1."));
    assert_eq!(c.error_code("hello", json!({"client": "test", "api_version": "2.0"})), "version_mismatch");
}

#[test]
fn register_list_unregister_round_trip() {
    let d = Daemon::start();
    let mut c = d.client();
    let mut r = http_route("Shop", 5173);
    r["persistent"] = json!(true);
    r["note"] = json!("main dev server");
    let reg = c.call("register_route", r);
    assert_eq!(reg["replaced"], false);
    assert_eq!(reg["route"]["host"], "shop");
    assert!(reg["route"]["urls"][0].as_str().unwrap().starts_with("https://shop.localhost:"));

    let saved: Value = serde_json::from_str(&std::fs::read_to_string(d.dir.path().join("routes.json")).unwrap()).unwrap();
    assert_eq!(saved["routes"][0]["host"], "shop");

    let again = c.call("register_route", http_route("shop", 5174));
    assert_eq!(again["replaced"], true);
    assert_eq!(again["old_target"], "http://127.0.0.1:5173");

    let list = c.call("list_routes", json!({}));
    assert_eq!(list["routes"].as_array().unwrap().len(), 1);
    assert_eq!(list["routes"][0]["upstream_up"], false);

    assert_eq!(c.call("unregister_route", json!({"host": "shop.localhost"}))["removed"], true);
    assert_eq!(c.call("unregister_route", json!({"host": "shop"}))["removed"], false);
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(d.dir.path().join("routes.json")).unwrap()).unwrap();
    assert_eq!(saved["routes"].as_array().unwrap().len(), 0);
}

#[test]
fn invalid_routes_are_refused_and_nothing_is_stored() {
    let d = Daemon::start();
    let mut c = d.client();
    assert_eq!(c.error_code("register_route", http_route("feat/login.shop", 5173)), "invalid_route");
    let mut r = http_route("shop", 5173);
    r["persistent"] = json!(true);
    r["owner_pid"] = json!(std::process::id());
    assert_eq!(c.error_code("register_route", r), "invalid_route");
    assert_eq!(c.call("list_routes", json!({}))["routes"].as_array().unwrap().len(), 0);
    assert_eq!(c.error_code("no_such_method", json!({})), "unknown_method");
}

// I8
#[test]
fn second_daemon_on_the_same_folder_exits() {
    let d = Daemon::start();
    let out = Command::new(env!("CARGO_BIN_EXE_localrouterd"))
        .env("LOCALROUTER_HOME", d.dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("already running"));
}

// I3
#[test]
fn owned_route_disappears_when_its_process_exits() {
    let d = Daemon::start();
    let mut c = d.client();
    let mut child = Command::new("sleep").arg("30").spawn().unwrap();
    let mut r = http_route("wt", 5175);
    r["owner_pid"] = json!(child.id());
    c.call("register_route", r);
    assert!(!d.dir.path().join("routes.json").exists() || !std::fs::read_to_string(d.dir.path().join("routes.json")).unwrap().contains("wt"));
    child.kill().unwrap();
    child.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let n = c.call("list_routes", json!({}))["routes"].as_array().unwrap().len();
        if n == 0 {
            break;
        }
        assert!(Instant::now() < deadline, "owned route still there after 1 s");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn dead_owner_pid_is_refused() {
    let d = Daemon::start();
    let mut c = d.client();
    let mut child = Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    let mut r = http_route("wt", 5175);
    r["owner_pid"] = json!(pid);
    assert_eq!(c.error_code("register_route", r), "process_not_found");
}

// I6
#[test]
fn no_reply_contains_key_material() {
    let d = Daemon::start();
    let mut c = d.client();
    let mut all = String::new();
    for (m, p) in [
        ("hello", json!({"client":"t","api_version":"1.0"})),
        ("status", json!({})),
        ("list_routes", json!({})),
        ("get_logs", json!({})),
        ("get_config", json!({})),
        ("find_free_port", json!({})),
    ] {
        all.push_str(&c.raw(m, p).to_string());
    }
    assert!(!all.contains("PRIVATE KEY"));
    let key = std::fs::read_to_string(d.dir.path().join("ca/ca.key")).unwrap();
    let body: String = key.lines().filter(|l| !l.starts_with("-----")).collect();
    assert!(!all.contains(&body[..20]));
}

#[test]
fn status_reports_ports_ca_and_counts() {
    let d = Daemon::start();
    let mut c = d.client();
    let s = c.call("status", json!({}));
    assert!(s["http"]["port"].as_u64().unwrap() > 0);
    assert!(s["https"]["port"].as_u64().unwrap() > 0);
    assert_eq!(s["ca"]["state"], "ok");
    assert!(s["ca"]["common_name"].as_str().unwrap().starts_with("LocalRouter CA"));
    assert_eq!(s["routes"], 0);
}

#[test]
fn long_home_path_gives_a_clear_error() {
    let long = std::env::temp_dir().join("x".repeat(120));
    let out = Command::new(env!("CARGO_BIN_EXE_localrouterd")).env("LOCALROUTER_HOME", &long).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("at most 103"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn set_config_is_saved_and_applied() {
    let d = Daemon::start();
    let mut c = d.client();
    let r = c.call("set_config", json!({"fallback": false, "log_size": 5}));
    assert_eq!(r["config"]["fallback"], false);
    assert_eq!(r["restart_needed"], false);
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(d.dir.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["log_size"], 5);
    assert_eq!(c.call("set_config", json!({"https_port": 8443}))["restart_needed"], true);
}

fn read_config(d: &Daemon) -> Value {
    serde_json::from_str(&std::fs::read_to_string(d.dir.path().join("config.json")).unwrap()).unwrap()
}

/// ADR 04, T4, I8: ports are edited only by hand in config.json. A later
/// Settings switch must not put the old ports back.
#[test]
fn set_config_keeps_a_hand_edit_of_config_json() {
    let d = Daemon::start();
    let mut c = d.client();
    let mut edited = read_config(&d);
    edited["https_port"] = json!(7444);
    std::fs::write(d.dir.path().join("config.json"), edited.to_string()).unwrap();

    let r = c.call("set_config", json!({"allow_lan": true}));
    let saved = read_config(&d);
    assert_eq!(saved["https_port"], 7444, "the hand edit was undone: {saved}");
    assert_eq!(saved["allow_lan"], true);
    assert_eq!(r["restart_needed"], true, "the file's ports differ from the ports the daemon started with");
}

/// ADR 04, T4: a switch that touches no port needs no restart, also after
/// an earlier call changed a port (the daemon still runs on its start ports).
#[test]
fn restart_needed_compares_with_the_start_ports() {
    let d = Daemon::start();
    let mut c = d.client();
    assert_eq!(c.call("set_config", json!({"fallback": false}))["restart_needed"], false);
    assert_eq!(c.call("set_config", json!({"https_port": 8443}))["restart_needed"], true);
    assert_eq!(c.call("set_config", json!({"fallback": true}))["restart_needed"], true, "8443 is still not bound");
    assert_eq!(c.call("set_config", json!({"https_port": 0}))["restart_needed"], false);
}

// T12: TCP route listeners

fn echo_server() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = s.read(&mut buf) {
                    if n == 0 || s.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

#[test]
fn tcp_route_listens_on_loopback_and_removal_closes_connections() {
    let d = Daemon::start();
    let mut c = d.client();
    let target = echo_server();
    let reg = c.call(
        "register_route",
        json!({"host": "db.shop", "protocol": "tcp", "target": format!("tcp://127.0.0.1:{target}"), "listen_port": 0}),
    );
    let port = reg["route"]["listen_port"].as_u64().unwrap() as u16;
    assert!(port >= 1024);
    assert_eq!(reg["route"]["urls"][0], format!("db.shop.localhost:{port}"));

    for addr in [format!("127.0.0.1:{port}"), format!("[::1]:{port}")] {
        let mut s = TcpStream::connect(&addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        s.write_all(b"ping").unwrap();
        let mut buf = [0u8; 4];
        s.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping", "{addr}");
    }

    let mut open = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    open.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    open.write_all(b"x").unwrap();
    let mut one = [0u8; 1];
    open.read_exact(&mut one).unwrap();

    c.call("unregister_route", json!({"host": "db.shop"}));
    let n = open.read(&mut one).unwrap_or(0);
    assert_eq!(n, 0, "open connection closed");
    std::thread::sleep(Duration::from_millis(100));
    assert!(TcpStream::connect(format!("127.0.0.1:{port}")).is_err(), "listener closed");
}

#[test]
fn tcp_route_with_a_taken_port_fails_and_stores_nothing() {
    let d = Daemon::start();
    let mut c = d.client();
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let r = json!({"host": "db", "protocol": "tcp", "target": "tcp://127.0.0.1:5432", "listen_port": port, "persistent": true});
    assert_eq!(c.error_code("register_route", r), "port_in_use");
    assert_eq!(c.call("list_routes", json!({}))["routes"].as_array().unwrap().len(), 0);
    assert!(!d.dir.path().join("routes.json").exists());
}

#[test]
fn saved_tcp_route_with_a_taken_port_is_marked_listen_failed() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let d = Daemon::start_with(|dir| {
        let routes = json!({"version": 1, "routes": [
            {"host": "db", "protocol": "tcp", "target": "tcp://127.0.0.1:5432", "listen_port": port, "persistent": true}
        ]});
        std::fs::write(dir.join("routes.json"), routes.to_string()).unwrap();
    });
    let mut c = d.client();
    let s = c.call("status", json!({}));
    assert_eq!(s["listen_failed"], json!(["db"]));
    let list = c.call("list_routes", json!({}));
    assert_eq!(list["routes"][0]["listen_failed"], true);
}

#[test]
fn broken_routes_file_is_reported_in_status() {
    let d = Daemon::start_with(|dir| std::fs::write(dir.join("routes.json"), "{oops").unwrap());
    let mut c = d.client();
    let s = c.call("status", json!({}));
    assert!(s["routes_file_problem"].as_str().unwrap().contains("moved"));
}

#[test]
fn router_host_is_refused_because_the_help_page_owns_it() {
    let d = Daemon::start();
    let mut c = d.client();
    let v = c.raw("register_route", http_route("router", 5173));
    assert_eq!(v["error"]["code"], "invalid_route", "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("help page"), "{v}");
    c.call("register_route", http_route("feat.router", 5173));
}

#[test]
fn saved_router_route_is_skipped_and_reported() {
    let d = Daemon::start_with(|dir| {
        let routes = json!({"version": 1, "routes": [
            {"host": "router", "target": "http://127.0.0.1:5173", "persistent": true},
            {"host": "shop", "target": "http://127.0.0.1:5174", "persistent": true}
        ]});
        std::fs::write(dir.join("routes.json"), routes.to_string()).unwrap();
    });
    let mut c = d.client();
    let s = c.call("status", json!({}));
    assert_eq!(s["routes"], 1);
    assert!(s["routes_file_problem"].as_str().unwrap().contains("skipped router"), "{s}");
}

#[test]
fn router_localhost_shows_the_daemon_status() {
    let d = Daemon::start();
    let mut c = d.client();
    let port = c.call("status", json!({}))["http"]["port"].as_u64().unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", port as u16)).unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: router.localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut page = String::new();
    s.read_to_string(&mut page).unwrap();
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.contains(&format!("| HTTP | on, port {port} |")), "{page}");
    assert!(page.contains("| Local CA | ready, "), "{page}");
}

// ADR 03, T6: path routes through the real daemon.

fn path_route(host: &str, path: &str, port: u16) -> Value {
    json!({"host": host, "path": path, "target": format!("http://127.0.0.1:{port}")})
}

fn keys(c: &mut Client) -> Vec<String> {
    c.call("list_routes", json!({}))["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| format!("{}{}", r["host"].as_str().unwrap(), r["path"].as_str().unwrap_or("")))
        .collect()
}

#[test]
fn path_routes_are_registered_replaced_and_removed_by_key() {
    let d = Daemon::start();
    let mut c = d.client();
    c.call("register_route", http_route("shop", 5173));
    let reg = c.call("register_route", path_route("shop", "/blog/", 3001));
    assert_eq!(reg["replaced"], false);
    assert_eq!(reg["route"]["path"], "/blog", "trailing slash removed");
    assert!(reg["route"]["urls"][0].as_str().unwrap().ends_with("/blog"), "{reg}");
    assert_eq!(keys(&mut c), ["shop", "shop/blog"]);

    let again = c.call("register_route", path_route("shop", "/blog", 3002));
    assert_eq!(again["replaced"], true);
    assert_eq!(again["old_target"], "http://127.0.0.1:3001");
    assert_eq!(keys(&mut c), ["shop", "shop/blog"], "the default route is untouched");

    // I28: without a path only the default route goes.
    assert_eq!(c.call("unregister_route", json!({"host": "shop"}))["removed"], true);
    assert_eq!(keys(&mut c), ["shop/blog"]);
    assert_eq!(c.call("unregister_route", json!({"host": "shop", "path": "/blog"}))["removed"], true);
    assert!(keys(&mut c).is_empty());
    assert_eq!(c.error_code("unregister_route", json!({"host": "shop", "path": "blog"})), "invalid_route");
}

#[test]
fn path_route_rules_are_enforced_by_the_daemon() {
    let d = Daemon::start();
    let mut c = d.client();
    assert_eq!(c.error_code("register_route", path_route("shop", "/a/../b", 1)), "invalid_route");
    let mut strip = http_route("shop", 1);
    strip["strip_path"] = json!(true);
    assert_eq!(c.error_code("register_route", strip), "invalid_route");
    c.call("register_route", json!({"host": "db", "protocol": "tcp", "target": "tcp://127.0.0.1:5432", "listen_port": 0}));
    assert_eq!(c.error_code("register_route", path_route("db", "/x", 1)), "invalid_route");
    assert_eq!(keys(&mut c), ["db"]);
}

#[test]
fn owner_exit_removes_only_its_path_route() {
    let d = Daemon::start();
    let mut c = d.client();
    let mut shop = http_route("shop", 5173);
    shop["persistent"] = json!(true);
    c.call("register_route", shop);
    let mut child = Command::new("sleep").arg("30").spawn().unwrap();
    let mut owned = path_route("shop", "/blog", 3002);
    owned["owner_pid"] = json!(child.id());
    c.call("register_route", owned);
    assert_eq!(keys(&mut c), ["shop", "shop/blog"]);
    child.kill().unwrap();
    child.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while keys(&mut c) != ["shop"] {
        assert!(Instant::now() < deadline, "owned path route still there after 1 s: {:?}", keys(&mut c));
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn persistent_path_route_survives_a_restart() {
    let saved = {
        let d = Daemon::start();
        let mut c = d.client();
        let mut r = path_route("shop", "/blog", 3001);
        r["persistent"] = json!(true);
        r["strip_path"] = json!(true);
        c.call("register_route", r);
        std::fs::read_to_string(d.dir.path().join("routes.json")).unwrap()
    };
    assert!(saved.contains("\"version\": 2"), "{saved}");
    let d = Daemon::start_with(|dir| std::fs::write(dir.join("routes.json"), &saved).unwrap());
    let mut c = d.client();
    let list = c.call("list_routes", json!({}));
    assert_eq!(list["routes"][0]["path"], "/blog");
    assert_eq!(list["routes"][0]["strip_path"], true);
    assert!(c.call("status", json!({})).get("routes_file_problem").is_none());
}

#[test]
fn a_request_reaches_the_path_route_and_the_log_names_it() {
    // A one-shot HTTP upstream per route, so the answer says which one it was.
    fn upstream(body: &'static str) -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            }
        });
        port
    }
    let d = Daemon::start();
    let mut c = d.client();
    c.call("register_route", http_route("shop", upstream("main")));
    c.call("register_route", path_route("shop", "/blog", upstream("blog")));
    let http = c.call("status", json!({}))["http"]["port"].as_u64().unwrap() as u16;
    let get = |path: &str| {
        let mut s = TcpStream::connect(("127.0.0.1", http)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nhost: shop.localhost\r\nconnection: close\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out.rsplit("\r\n\r\n").next().unwrap().to_string()
    };
    assert_eq!(get("/blog/x"), "blog");
    assert_eq!(get("/x"), "main");
    let entries = c.call("get_logs", json!({"host": "shop", "limit": 10}))["entries"].clone();
    let routes: Vec<_> = entries.as_array().unwrap().iter().map(|e| e["route"].as_str().unwrap().to_string()).collect();
    assert_eq!(routes, ["shop/blog", "shop"]);
}

// ADR 04, T3: a daemon named localrouterd-dev is the -dev instance.

/// The daemon binary copied under the name of a suffixed instance. The
/// instance comes from the program's own file name.
fn renamed_daemon(suffix: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new().prefix("lrbin").tempdir().unwrap();
    let program = dir.path().join(format!("localrouterd{suffix}"));
    std::fs::copy(env!("CARGO_BIN_EXE_localrouterd"), &program).unwrap();
    (dir, program)
}

#[test]
fn a_dev_daemon_writes_its_own_default_ports() {
    let (_bin, program) = renamed_daemon("-dev");
    let d = Daemon::start_program(&program, |_| {});
    let saved = read_config(&d);
    assert_eq!((saved["http_port"].clone(), saved["https_port"].clone()), (json!(7080), json!(7443)));
    let r = d.client().call("get_config", json!({}));
    assert_eq!(r["http_port"], 7080, "{r}");
}

#[test]
fn a_dev_daemon_keeps_an_existing_config_as_written() {
    let (_bin, program) = renamed_daemon("-dev");
    let written = "{\"version\":1,\"http_port\":0,\"https_port\":0,\"fallback\":false,\"allow_lan\":false,\"log_size\":7}\n";
    let d = Daemon::start_program(&program, |dir| std::fs::write(dir.join("config.json"), written).unwrap());
    assert_eq!(std::fs::read_to_string(d.dir.path().join("config.json")).unwrap(), written);
}

#[test]
fn a_daemon_with_a_bad_suffix_stops_before_it_makes_a_folder() {
    let (_bin, program) = renamed_daemon("-Dev");
    let home = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    let data = home.path().join("data");
    let out = Command::new(&program).env("LOCALROUTER_HOME", &data).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid instance suffix"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!data.exists());
}

// Folder routes: target file:// plus a folder the daemon serves itself.

#[test]
fn folder_route_is_checked_saved_served_and_reported_up() {
    let site = tempfile::Builder::new().prefix("lr-site").tempdir().unwrap();
    std::fs::write(site.path().join("index.html"), "<h1>report</h1>").unwrap();
    let d = Daemon::start();
    let mut c = d.client();

    let missing = json!({"host": "docs", "target": format!("file://{}/nope", site.path().display())});
    let v = c.raw("register_route", missing);
    assert_eq!(v["error"]["code"], "invalid_route", "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("cannot open folder"), "{v}");
    let file = json!({"host": "docs", "target": format!("file://{}/index.html", site.path().display())});
    assert!(c.raw("register_route", file)["error"]["message"].as_str().unwrap().contains("is not a folder"));
    // A folder the system refuses (macOS privacy gives the same error kind):
    // the message says what to do.
    use std::os::unix::fs::PermissionsExt;
    let locked = tempfile::Builder::new().prefix("lr-locked").tempdir().unwrap();
    std::fs::create_dir(locked.path().join("site")).unwrap();
    std::fs::set_permissions(locked.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
    let denied = c.raw("register_route", json!({"host": "docs", "target": format!("file://{}/site", locked.path().display())}));
    std::fs::set_permissions(locked.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(denied["error"]["message"].as_str().unwrap().contains("Privacy & Security"), "{denied}");
    let relative = json!({"host": "docs", "target": "file://site"});
    assert!(c.raw("register_route", relative)["error"]["message"].as_str().unwrap().contains("absolute path"));

    let target = format!("file://{}/", site.path().display());
    let reg = c.call("register_route", json!({"host": "docs", "target": target, "persistent": true}));
    assert_eq!(reg["route"]["target"], format!("file://{}", site.path().display()), "trailing slash removed");
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(d.dir.path().join("routes.json")).unwrap()).unwrap();
    assert_eq!(saved["version"], 1, "a folder route alone keeps version 1, so older daemons skip only that route");
    assert_eq!(c.call("list_routes", json!({}))["routes"][0]["upstream_up"], true);

    let port = c.call("status", json!({}))["http"]["port"].as_u64().unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", port as u16)).unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: docs.localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut reply = String::new();
    s.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with("<h1>report</h1>"), "{reply}");

    let gone = site.path().to_path_buf();
    drop(site);
    assert_eq!(c.call("list_routes", json!({}))["routes"][0]["upstream_up"], false);

    // A saved folder route whose folder is gone still loads after a restart.
    drop(c);
    let routes = std::fs::read_to_string(d.dir.path().join("routes.json")).unwrap();
    drop(d);
    let d = Daemon::start_with(|dir| std::fs::write(dir.join("routes.json"), &routes).unwrap());
    let list = d.client().call("list_routes", json!({}));
    assert_eq!(list["routes"][0]["target"], format!("file://{}", gone.display()));
    assert_eq!(list["routes"][0]["upstream_up"], false);
}

// ---- ADR 06: the forward proxy port

fn proxy_on(c: &mut Client) -> u16 {
    c.call("set_config", json!({"proxy_enabled": true, "proxy_port": 0}));
    c.call("status", json!({}))["proxy"]["port"].as_u64().expect("the proxy port is bound") as u16
}

/// `CONNECT target` through the proxy; the stream after the 200.
fn connect_through(proxy: u16, target: &str) -> TcpStream {
    let mut s = TcpStream::connect(("127.0.0.1", proxy)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes()).unwrap();
    let mut head = vec![];
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 200"), "{}", String::from_utf8_lossy(&head));
    s
}

// T1, I1: the proxy port is 127.0.0.1 and ::1 only, even with allow_lan.
#[test]
fn proxy_port_binds_loopback_only_even_with_allow_lan() {
    let d = Daemon::start();
    let mut c = d.client();
    c.call("set_config", json!({"allow_lan": true}));
    let port = proxy_on(&mut c);
    let s = c.call("status", json!({}));
    assert_eq!(s["proxy"]["bound"], json!([format!("127.0.0.1:{port}"), format!("[::1]:{port}")]));
    assert_eq!(s["proxy"]["enabled"], true);
    assert_eq!(s["proxy"]["errors"], json!([]));
    let p = c.call("get_proxy", json!({}));
    assert_eq!(p["url"], format!("http://127.0.0.1:{port}"));
    assert_eq!(p["env"]["HTTPS_PROXY"], format!("http://127.0.0.1:{port}"));
}

// T7, I2, I12: on and off without a restart; off closes open tunnels; routes
// are not touched.
#[test]
fn proxy_on_and_off_without_restart_closes_open_tunnels() {
    let d = Daemon::start();
    let mut c = d.client();
    c.call("register_route", http_route("shop", 5173));
    let routes_before = c.call("list_routes", json!({}))["routes"].clone();
    assert!(c.call("status", json!({}))["proxy"]["port"].is_null(), "off by default");

    let port = proxy_on(&mut c);
    let target = echo_server();
    let mut tunnel = connect_through(port, &format!("127.0.0.1:{target}"));
    tunnel.write_all(b"ping").unwrap();
    let mut buf = [0u8; 4];
    tunnel.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"ping");

    c.call("set_config", json!({"proxy_enabled": false}));
    assert_eq!(tunnel.read(&mut buf).unwrap_or(0), 0, "the open tunnel is closed");
    std::thread::sleep(Duration::from_millis(100));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "the listener is closed");
    assert!(c.call("status", json!({}))["proxy"]["port"].is_null());
    assert_eq!(c.call("list_routes", json!({}))["routes"], routes_before);
    assert_eq!(read_config(&d)["proxy_enabled"], false);

    // The tunnel left one log entry, via the proxy.
    let logs = c.call("get_logs", json!({}))["entries"].clone();
    assert!(logs.as_array().unwrap().iter().any(|e| e["method"] == "CONNECT" && e["mode"] == "tunnel"), "{logs}");
}

// T7, I12: a port taken on ::1 only fails the call and changes nothing.
#[test]
fn a_taken_proxy_port_changes_nothing() {
    // A port below the ephemeral range, free on both addresses (see the
    // tcp_listen tests): keep its ::1 socket as the probe, free 127.0.0.1.
    let (port, _held_v6) = (20000 + (std::process::id() % 5000) as u16..30000)
        .find_map(|p| {
            let v4 = std::net::TcpListener::bind(("127.0.0.1", p)).ok()?;
            let v6 = std::net::TcpListener::bind(("::1", p)).ok()?;
            drop(v4);
            Some((p, v6))
        })
        .expect("a free port");
    let d = Daemon::start();
    let mut c = d.client();
    let file_before = read_config(&d);
    assert_eq!(c.error_code("set_config", json!({"proxy_enabled": true, "proxy_port": port})), "port_in_use");
    let config = c.call("get_config", json!({}));
    assert_eq!(config["proxy_enabled"], false);
    assert_ne!(config["proxy_port"], port);
    assert_eq!(read_config(&d), file_before, "config.json is not written");
    assert!(c.call("status", json!({}))["proxy"]["port"].is_null());
    let mut v4 = std::net::TcpListener::bind(("127.0.0.1", port));
    for _ in 0..40 {
        if v4.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        v4 = std::net::TcpListener::bind(("127.0.0.1", port));
    }
    assert!(v4.is_ok(), "127.0.0.1:{port} still held after the failed call");
}

// On macOS a loopback socket can bind next to the wildcard one of the HTTP
// port and take its traffic: the proxy may not share a port with the router.
#[test]
fn proxy_port_may_not_be_the_http_port() {
    let d = Daemon::start();
    let mut c = d.client();
    let http = c.call("status", json!({}))["http"]["port"].as_u64().unwrap();
    c.call("set_config", json!({"http_port": http}));
    assert_eq!(c.error_code("set_config", json!({"proxy_enabled": true, "proxy_port": http})), "invalid_request");
}

// T7: a saved proxy_enabled binds at start.
#[test]
fn a_saved_proxy_is_bound_at_start() {
    let d = Daemon::start_with(|dir| {
        let mut config: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
        config["proxy_enabled"] = json!(true);
        config["proxy_port"] = json!(0);
        std::fs::write(dir.join("config.json"), config.to_string()).unwrap();
    });
    let mut c = d.client();
    let port = c.call("status", json!({}))["proxy"]["port"].as_u64().expect("bound at start");
    assert!(TcpStream::connect(("127.0.0.1", port as u16)).is_ok());
}

// T5, I4, I5: the inspection CA is made on first need, kept when the list
// empties, replaced only by reset_inspect_ca, and its key never leaves.
#[test]
fn the_default_inspect_list_is_every_host_and_turning_the_proxy_on_makes_the_ca() {
    let d = Daemon::start_program(Path::new(env!("CARGO_BIN_EXE_localrouterd")), |dir| {
        std::fs::write(dir.join("config.json"), json!({"version":1,"http_port":0,"https_port":0}).to_string()).unwrap();
    });
    let mut c = d.client();
    assert_eq!(c.call("get_config", json!({}))["inspect_hosts"], json!(["*"]));
    let ca_dir = d.dir.path().join("inspect-ca");
    assert!(!ca_dir.exists(), "never made at start (I5)");
    proxy_on(&mut c);
    assert!(ca_dir.join("ca.pem").exists(), "the proxy turned on with '*' makes the CA");
    let p = c.call("get_proxy", json!({}));
    assert_eq!(p["inspect_set"], json!(["*"]));
    assert!(p["inspect_ca"]["common_name"].as_str().unwrap().starts_with("LocalRouter Inspection "));
}

#[test]
fn the_inspection_ca_is_made_on_need_and_kept() {
    let d = Daemon::start();
    let mut c = d.client();
    proxy_on(&mut c);
    let ca_dir = d.dir.path().join("inspect-ca");
    assert!(!ca_dir.exists(), "no inspect host, no CA (I5)");
    assert!(c.call("status", json!({}))["proxy"]["inspect_ca"].is_null());
    assert!(c.call("get_proxy", json!({}))["env"].get("NODE_EXTRA_CA_CERTS").is_none());

    assert_eq!(c.error_code("set_config", json!({"inspect_hosts": ["*.com"]})), "invalid_request");
    assert!(!ca_dir.exists());

    let r = c.call("set_config", json!({"inspect_hosts": ["API.example.com", "api.example.com", "*.example.org"]}));
    assert_eq!(r["config"]["inspect_hosts"], json!(["api.example.com", "*.example.org"]), "checked, lower case, no duplicates");
    assert!(ca_dir.join("ca.pem").exists());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(ca_dir.join("ca.key")).unwrap().permissions().mode() & 0o777, 0o600);
    let p = c.call("get_proxy", json!({}));
    assert_eq!(p["inspect_set"], json!(["api.example.com", "*.example.org"]));
    let first = p["inspect_ca"]["common_name"].as_str().unwrap().to_string();
    assert!(first.starts_with("LocalRouter Inspection "), "{first}");
    assert_eq!(p["env"]["NODE_EXTRA_CA_CERTS"], ca_dir.join("ca.pem").display().to_string());

    assert!(!p["notes"].to_string().contains("'*'"), "no note without '*'");

    let r = c.call("set_config", json!({"inspect_hosts": [" * ", "api.example.com"]}));
    assert_eq!(r["config"]["inspect_hosts"], json!(["*", "api.example.com"]), "'*' is every host outside .localhost");
    let p = c.call("get_proxy", json!({}));
    assert_eq!(p["inspect_set"], json!(["*", "api.example.com"]));
    let notes = p["notes"].to_string();
    assert!(notes.contains("every HTTPS host is inspected"), "{notes}");

    c.call("set_config", json!({"inspect_hosts": []}));
    assert!(ca_dir.exists(), "an empty list keeps the CA");
    assert_eq!(c.call("get_proxy", json!({}))["inspect_ca"]["common_name"], first.as_str());

    let mut replies = String::new();
    let reset = c.raw("reset_inspect_ca", json!({}));
    replies.push_str(&reset.to_string());
    assert_ne!(reset["result"]["common_name"], first.as_str());
    replies.push_str(&c.raw("get_proxy", json!({})).to_string());
    replies.push_str(&c.raw("status", json!({})).to_string());
    replies.push_str(&c.raw("get_config", json!({})).to_string());
    assert!(!replies.contains("PRIVATE KEY"), "I4");
    let key = std::fs::read_to_string(ca_dir.join("ca.key")).unwrap();
    let body: String = key.lines().filter(|l| !l.starts_with("-----")).collect();
    assert!(!replies.contains(&body[..20]));
}

// T9: the two new methods are known; get_proxy names the Chrome profile.
#[test]
fn get_proxy_gives_chrome_args_with_their_own_profile() {
    let d = Daemon::start();
    let mut c = d.client();
    let p = c.call("get_proxy", json!({}));
    let args: Vec<String> = serde_json::from_value(p["chrome_args"].clone()).unwrap();
    let profile = d.dir.path().join("caches/chrome-proxy").display().to_string();
    assert_eq!(args[0], format!("--user-data-dir={profile}"));
    assert!(args[1].starts_with("--proxy-server=http://127.0.0.1:"));
    assert!(!profile.contains("Google/Chrome"), "never Chrome's own profile (I17)");
    assert!(!d.dir.path().join("caches").exists(), "the daemon writes nothing there");
}

// ---- ADR 07: script rules (T13)

const LOG_LUA: &str = r#"return { kind = "log", on_exchange = function(ex) capture.append("x.jsonl", ex.request.path .. "\n") end }"#;
const INTERCEPT_LUA: &str = r#"return { kind = "intercept", on_request = function(req) req.headers["x-lr"] = "1" end }"#;

/// A folder outside the daemon's data folder, with a script and `out/`.
fn script_dir(name: &str, lua: &str) -> (tempfile::TempDir, String, String) {
    let dir = tempfile::Builder::new().prefix("lrs").tempdir().unwrap();
    let script = dir.path().join(name);
    std::fs::write(&script, lua).unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    (dir, script.display().to_string(), out.display().to_string())
}

fn rule_ids(c: &mut Client) -> Vec<String> {
    let list = c.call("list_script_rules", json!({}));
    list["rules"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn a_persistent_script_rule_survives_a_restart_and_an_owned_one_never_reaches_the_file() {
    let (_s, script, out) = script_dir("cap.lua", LOG_LUA);
    let saved = {
        let d = Daemon::start();
        let mut c = d.client();
        let r = c.call("set_script_rule", json!({"id": "cap", "host": "shop.localhost", "script": script, "output_dir": out, "persistent": true}));
        assert_eq!((r["rule"]["kind"].as_str(), r["replaced"].as_bool()), (Some("log"), Some(false)));

        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        c.call("set_script_rule", json!({"id": "mine", "host": "shop.localhost", "script": script, "output_dir": out, "owner_pid": child.id()}));
        let file = std::fs::read_to_string(d.dir.path().join("script-rules.json")).unwrap();
        assert!(file.contains("\"cap\"") && !file.contains("mine"), "I10: {file}");
        child.kill().unwrap();
        child.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while rule_ids(&mut c).contains(&"mine".to_string()) {
            assert!(Instant::now() < deadline, "the owned rule is still there");
            std::thread::sleep(Duration::from_millis(20));
        }
        file
    };
    let d = Daemon::start_with(|dir| std::fs::write(dir.join("script-rules.json"), &saved).unwrap());
    let mut c = d.client();
    let list = c.call("list_script_rules", json!({}));
    assert_eq!(list["rules"][0]["id"], "cap");
    assert_eq!(list["rules"][0]["kind"], "log", "the script loaded again at start");
    assert_eq!(c.error_code("set_script_rule", json!({"id": "x", "host": "a.example.com", "script": script, "output_dir": out, "persistent": true, "owner_pid": 1})), "invalid_script_rule");
}

#[test]
fn a_failed_write_of_script_rules_json_changes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let (_s, script, out) = script_dir("cap.lua", LOG_LUA);
    let d = Daemon::start();
    let mut c = d.client();
    std::fs::set_permissions(d.dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let code = c.error_code("set_script_rule", json!({"id": "cap", "host": "shop.localhost", "script": script, "output_dir": out, "persistent": true}));
    std::fs::set_permissions(d.dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(code, "io");
    assert!(rule_ids(&mut c).is_empty(), "I4: memory rolled back");
}

#[test]
fn a_rule_on_an_internet_host_makes_it_inspected_until_removed() {
    let (_s, script, _) = script_dir("add.lua", INTERCEPT_LUA);
    let d = Daemon::start();
    let mut c = d.client();
    assert!(!d.dir.path().join("inspect-ca").exists());
    let r = c.call("set_script_rule", json!({"id": "api", "host": "api.test.example", "script": script}));
    assert_eq!(r["inspect"]["host_added"], true);
    assert_eq!(r["inspect"]["ca_created"], true);
    let notes = r["notes"].to_string();
    assert!(notes.contains("The proxy is off"), "{notes}");
    assert!(d.dir.path().join("inspect-ca/ca.pem").exists());
    let p = c.call("get_proxy", json!({}));
    assert_eq!(p["inspect_set"], json!(["api.test.example"]));
    assert_eq!(p["script_rules"][0]["id"], "api");

    assert_eq!(c.call("remove_script_rule", json!({"id": "api"}))["removed"], true);
    assert_eq!(c.call("get_proxy", json!({}))["inspect_set"], json!([]), "I11: gone with the rule");
    assert_eq!(c.call("remove_script_rule", json!({"id": "api"}))["removed"], false);

    c.call("set_config", json!({"inspect_hosts": ["api.test.example"]}));
    c.call("set_script_rule", json!({"id": "api", "host": "api.test.example", "script": script}));
    c.call("remove_script_rule", json!({"id": "api"}));
    assert_eq!(c.call("get_proxy", json!({}))["inspect_set"], json!(["api.test.example"]), "inspect_hosts still lists it");
}

#[test]
fn a_rule_on_every_host_inspects_every_host_until_removed() {
    let (_s, script, _) = script_dir("add.lua", INTERCEPT_LUA);
    let d = Daemon::start();
    let mut c = d.client();
    let r = c.call("set_script_rule", json!({"id": "all", "host": "*", "script": script}));
    assert_eq!(r["rule"]["host"], "*");
    assert_eq!(r["inspect"]["host_added"], true);
    let p = c.call("get_proxy", json!({}));
    assert_eq!(p["inspect_set"], json!(["*"]));
    assert!(p["notes"].to_string().contains("every HTTPS host is inspected"), "{}", p["notes"]);

    c.call("remove_script_rule", json!({"id": "all"}));
    let p = c.call("get_proxy", json!({}));
    assert_eq!(p["inspect_set"], json!([]));
    assert!(!p["notes"].to_string().contains("'*'"));
}

#[test]
fn check_only_stores_nothing_and_bad_rules_are_refused_with_the_reason() {
    let (s, script, out) = script_dir("cap.lua", LOG_LUA);
    let d = Daemon::start();
    let mut c = d.client();
    let r = c.call("set_script_rule", json!({"id": "cap", "host": "shop.localhost", "script": script, "output_dir": out, "check_only": true}));
    assert_eq!((r["check_only"].as_bool(), r["rule"]["kind"].as_str()), (Some(true), Some("log")));
    assert!(rule_ids(&mut c).is_empty());

    let bad = s.path().join("bad.lua");
    std::fs::write(&bad, "return {\n  kind = 'log',\n  on_exchange = function(ex) x = end }").unwrap();
    let v = c.raw("set_script_rule", json!({"id": "bad", "host": "shop.localhost", "script": bad, "output_dir": out, "check_only": true}));
    assert_eq!(v["error"]["code"], "invalid_script_rule");
    assert!(v["error"]["message"].as_str().unwrap().contains("bad.lua:3:"), "{v}");

    let v = c.raw("set_script_rule", json!({"id": "cap", "host": "shop.localhost", "script": script}));
    assert!(v["error"]["message"].as_str().unwrap().contains("give output_dir"), "{v}");
    let data_out = d.dir.path().join("out");
    let v = c.raw("set_script_rule", json!({"id": "cap", "host": "shop.localhost", "script": script, "output_dir": data_out}));
    assert!(v["error"]["message"].as_str().unwrap().contains("data folder"), "{v}");
    let v = c.raw("set_script_rule", json!({"id": "cap", "host": "router.localhost", "script": script, "output_dir": out}));
    assert!(v["error"]["message"].as_str().unwrap().contains("help page"), "{v}");
}

#[test]
fn an_output_dir_the_daemon_cannot_write_is_refused_at_once() {
    use std::os::unix::fs::PermissionsExt;
    let (_s, script, out) = script_dir("cap.lua", LOG_LUA);
    let d = Daemon::start();
    let mut c = d.client();
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o500)).unwrap();
    let v = c.raw("set_script_rule", json!({"id": "cap", "host": "shop.localhost", "script": script, "output_dir": out}));
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o700)).unwrap();
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("cannot write in output_dir") && msg.contains("Privacy & Security"), "{msg}");
    assert!(std::fs::read_dir(&out).unwrap().next().is_none(), "the probe left nothing");
}

#[test]
fn rule_counters_move_with_traffic_and_the_log_names_the_rules() {
    let (_s, script, out) = script_dir("cap.lua", LOG_LUA);
    let d = Daemon::start();
    let mut c = d.client();
    c.call("register_route", http_route("shop", echo_server()));
    c.call("set_script_rule", json!({"id": "cap", "host": "shop.localhost", "script": script, "output_dir": out}));
    let http = c.call("status", json!({}))["http"]["port"].as_u64().unwrap() as u16;
    let mut s = TcpStream::connect(("127.0.0.1", http)).unwrap();
    write!(s, "GET /hello HTTP/1.1\r\nhost: shop.localhost\r\nconnection: close\r\n\r\n").unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while std::fs::read_to_string(Path::new(&out).join("x.jsonl")).unwrap_or_default() != "/hello\n" {
        assert!(Instant::now() < deadline, "the capture file never got the line");
        std::thread::sleep(Duration::from_millis(20));
    }
    let rule = &c.call("list_script_rules", json!({}))["rules"][0];
    assert_eq!(rule["matched"], 1);
    assert_eq!(rule["bytes_written"], 7);
    assert_eq!(rule["script_sha256"].as_str().unwrap().len(), 64);
    let entry = c.call("get_logs", json!({"limit": 1}))["entries"][0].clone();
    assert_eq!(entry["rules"], json!(["cap"]));
}

// I11: a rule moved from an internet host to a route takes its old host out
// of the inspect set at once.
#[test]
fn a_rule_moved_to_a_route_leaves_the_inspect_set() {
    let (_s, script, _) = script_dir("add.lua", INTERCEPT_LUA);
    let d = Daemon::start();
    let mut c = d.client();
    c.call("set_script_rule", json!({"id": "x", "host": "api.test.example", "script": script}));
    assert_eq!(c.call("get_proxy", json!({}))["inspect_set"], json!(["api.test.example"]));
    c.call("set_script_rule", json!({"id": "x", "host": "shop.localhost", "script": script}));
    assert_eq!(c.call("get_proxy", json!({}))["inspect_set"], json!([]));
}

// ---- ADR 08: the proxy log, the reserved name, the CA bundle, LAN per network

/// One absolute-form GET through the proxy to the daemon's own HTTP port:
/// an upstream request the proxy records.
fn get_through(proxy: u16, http: u16, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", proxy)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(format!("GET http://127.0.0.1:{http}{path} HTTP/1.1\r\nHost: 127.0.0.1:{http}\r\nAuthorization: Bearer t\r\nConnection: close\r\n\r\n").as_bytes())
        .unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn wait_written(c: &mut Client, n: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let log = c.call("get_proxy", json!({}))["log"].clone();
        if log["written"].as_u64() >= Some(n) {
            return log;
        }
        assert!(Instant::now() < deadline, "the log did not get {n} entries: {log}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// T8: a fresh home gets the log on, 100 MB, 5000 requests; get_proxy has the
// log block with the folder under the temp home.
#[test]
fn proxy_log_defaults() {
    let d = Daemon::start();
    let mut c = d.client();
    let config = c.call("get_config", json!({}));
    assert_eq!((config["proxy_log"].clone(), config["proxy_log_file_mb"].clone(), config["proxy_log_file_requests"].clone()), (json!(true), json!(100), json!(5000)));
    assert_eq!(config["lan_networks"], json!([]));
    let log = c.call("get_proxy", json!({}))["log"].clone();
    assert_eq!(log["folder"], d.dir.path().join("logs/proxy").display().to_string());
    assert_eq!(log["enabled"], true);
    assert_eq!(log["keep_files"], 5);
    let http = c.call("status", json!({}))["http"]["port"].as_u64().unwrap();
    assert_eq!(log["url"], format!("http://proxy.localhost:{http}"));
}

// T8: each field changes; an out-of-range value is refused with the range.
#[test]
fn proxy_log_set_config() {
    let d = Daemon::start();
    let mut c = d.client();
    let r = c.call("set_config", json!({"proxy_log": false, "proxy_log_file_mb": 5, "proxy_log_file_requests": 200}));
    assert_eq!((r["config"]["proxy_log"].clone(), r["config"]["proxy_log_file_mb"].clone(), r["config"]["proxy_log_file_requests"].clone()), (json!(false), json!(5), json!(200)));
    let log = c.call("get_proxy", json!({}))["log"].clone();
    assert_eq!((log["enabled"].clone(), log["file_mb"].clone(), log["file_requests"].clone()), (json!(false), json!(5), json!(200)));
    for (params, range) in [
        (json!({"proxy_log_file_mb": 0}), "1 to 200"),
        (json!({"proxy_log_file_mb": 201}), "1 to 200"),
        (json!({"proxy_log_file_requests": 99}), "100 to 1000000"),
    ] {
        let v = c.raw("set_config", params.clone());
        assert_eq!(v["error"]["code"], "invalid_request", "{params}");
        assert!(v["error"]["message"].as_str().unwrap().contains(range), "{v}");
    }
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(d.dir.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["proxy_log_file_mb"], 5, "a refused value is not written");
}

// T8, the mock-versus-real check: the daemon makes the log from config.json
// and records proxy traffic; off stops records at once; on starts a new file.
#[test]
fn proxy_log_off_then_on() {
    let d = Daemon::start();
    let mut c = d.client();
    let proxy = proxy_on(&mut c);
    let http = c.call("status", json!({}))["http"]["port"].as_u64().unwrap() as u16;
    get_through(proxy, http, "/a?x=1");
    let log = wait_written(&mut c, 1);
    let first = log["current"].as_str().unwrap().to_string();
    let text = std::fs::read_to_string(d.dir.path().join("logs/proxy").join(&first)).unwrap();
    let har: Value = serde_json::from_str(&text).unwrap();
    let entry = &har["log"]["entries"][0];
    assert_eq!(entry["request"]["url"], format!("http://127.0.0.1:{http}/a?x=1"));
    assert!(text.contains("Bearer t"), "every header as it is: {text}");
    assert_eq!(har["log"]["creator"]["name"], "LocalRouter");

    c.call("set_config", json!({"proxy_log": false}));
    get_through(proxy, http, "/b");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(c.call("get_proxy", json!({}))["log"]["written"], 1, "off: no record");

    c.call("set_config", json!({"proxy_log": true}));
    get_through(proxy, http, "/c");
    let log = wait_written(&mut c, 2);
    assert_ne!(log["current"].as_str().unwrap(), first, "on after off: a new file");
}

// T7, I11: a saved route `proxy` loads and answers proxy.localhost; a new one
// is refused; status names the second address.
#[test]
fn proxy_route_saved_before_is_kept() {
    let d = Daemon::start_with(|dir| {
        std::fs::write(
            dir.join("routes.json"),
            json!({"version":1,"routes":[{"host":"proxy","target":"http://127.0.0.1:9","persistent":true}]}).to_string(),
        )
        .unwrap();
    });
    let mut c = d.client();
    let status = c.call("status", json!({}));
    assert_eq!(status["routes"], 1, "{status}");
    let notes = status["notes"].to_string();
    assert!(notes.contains("The route proxy hides the proxy log viewer") && notes.contains("/proxy-log/"), "{notes}");
    let http = status["http"]["port"].as_u64().unwrap() as u16;
    let mut s = TcpStream::connect(("127.0.0.1", http)).unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: proxy.localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 502"), "the route answered, not the viewer: {out}");

    assert_eq!(c.error_code("register_route", http_route("proxy", 5173)), "invalid_route");
    assert_eq!(c.error_code("register_route", http_route("router", 5173)), "invalid_route");
}

// T8, I23: the bundle holds the system roots then the inspection CA, never a
// key; env points the three CA names at it; the lowercase names are set.
#[test]
fn proxy_env_bundle() {
    let d = Daemon::start();
    let mut c = d.client();
    let p = c.call("get_proxy", json!({}));
    assert!(p["env"].get("SSL_CERT_FILE").is_none(), "no CA, no bundle");
    assert_eq!(p["env"]["no_proxy"], "localhost,127.0.0.1,::1,.localhost");
    assert_eq!(p["env"]["http_proxy"], p["env"]["HTTP_PROXY"]);
    assert_eq!(p["env"]["https_proxy"], p["env"]["HTTPS_PROXY"]);

    c.call("set_config", json!({"inspect_hosts": ["api.example.com"]}));
    let p = c.call("get_proxy", json!({}));
    let bundle = d.dir.path().join("inspect-ca/bundle.pem");
    for name in ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"] {
        assert_eq!(p["env"][name], bundle.display().to_string(), "{name}");
    }
    let text = std::fs::read_to_string(&bundle).unwrap();
    let certs = text.matches("-----BEGIN CERTIFICATE-----").count();
    assert!(certs > 100, "the system roots: {certs} certificates");
    let ca = std::fs::read_to_string(d.dir.path().join("inspect-ca/ca.pem")).unwrap();
    assert!(text.trim_end().ends_with(ca.trim_end()), "the inspection CA comes last");
    assert!(!text.contains("PRIVATE KEY"), "no key in the bundle");
}

const HOME_NET: &str = "mac:18:35:d1:15:d1:a8,192.168.0.1,en0";

fn start_on_network(config: Value, network: &str) -> Daemon {
    Daemon::start_program_env(
        Path::new(env!("CARGO_BIN_EXE_localrouterd")),
        |dir| std::fs::write(dir.join("config.json"), config.to_string()).unwrap(),
        &[("LOCALROUTER_TEST_NETWORK", network)],
    )
}

fn base_config(allow_lan: bool) -> Value {
    json!({"version":1,"http_port":0,"https_port":0,"fallback":true,"allow_lan":allow_lan,"log_size":100,"inspect_hosts":[]})
}

// T16: lan_networks is set and read; status.network says whether LAN access
// applies here.
#[test]
fn lan_networks_and_status_network() {
    let d = start_on_network(base_config(false), HOME_NET);
    let mut c = d.client();
    let net = c.call("status", json!({}))["network"].clone();
    assert_eq!(net, json!({"id": "mac:18:35:d1:15:d1:a8", "name": "", "router": "192.168.0.1", "interface": "en0", "lan_allowed": false}));
    let r = c.call(
        "set_config",
        json!({"allow_lan": true, "lan_networks": [{"id": "MAC:18:35:D1:15:D1:A8", "name": " Home ", "router": "192.168.0.1"}]}),
    );
    assert_eq!(r["config"]["lan_networks"], json!([{"id": "mac:18:35:d1:15:d1:a8", "name": "Home", "router": "192.168.0.1"}]));
    let net = c.call("status", json!({}))["network"].clone();
    assert_eq!((net["name"].clone(), net["lan_allowed"].clone()), (json!("Home"), json!(true)));
    let v = c.raw("set_config", json!({"lan_networks": [{"id": "192.168.0.1"}]}));
    assert_eq!(v["error"]["code"], "invalid_request", "{v}");
    // Another switch keeps the list.
    let r = c.call("set_config", json!({"fallback": false}));
    assert_eq!(r["config"]["lan_networks"][0]["name"], "Home");
}

// T16, I18: an old allow_lan true becomes the current network, never every network.
#[test]
fn lan_update_from_allow_lan_true() {
    let d = start_on_network(base_config(true), HOME_NET);
    let mut c = d.client();
    let config = c.call("get_config", json!({}));
    assert_eq!(config["lan_networks"], json!([{"id": "mac:18:35:d1:15:d1:a8", "name": "", "router": "192.168.0.1"}]));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(d.dir.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["lan_networks"], config["lan_networks"], "written once");
    let status = c.call("status", json!({}));
    assert!(status["notes"].to_string().contains("LAN access now works per network. Allowed on: 192.168.0.1"), "{status}");
    assert_eq!(status["network"]["lan_allowed"], true);

    // An unknown network: the list stays empty, the note says so.
    let d = start_on_network(base_config(true), "none");
    let mut c = d.client();
    assert_eq!(c.call("get_config", json!({}))["lan_networks"], json!([]));
    let status = c.call("status", json!({}));
    assert!(status["notes"].to_string().contains("could not be recognised"), "{status}");
    assert!(status["network"].is_null());

    // A file that already has the field is left as it is.
    let mut config = base_config(true);
    config["lan_networks"] = json!([]);
    let d = start_on_network(config, HOME_NET);
    let mut c = d.client();
    assert_eq!(c.call("get_config", json!({}))["lan_networks"], json!([]));
    assert!(c.call("status", json!({})).get("notes").is_none());
}


// ---- ADR 09: proxy clients, one more proxy port per client

/// The bound port of proxy client `name`, from status.
fn client_port(c: &mut Client, name: &str) -> Option<u16> {
    let s = c.call("status", json!({}));
    let clients = s["proxy"]["clients"].as_array().cloned().unwrap_or_default();
    clients.iter().find(|x| x["name"] == name)?["port"].as_u64().map(|p| p as u16)
}

/// Every entry of the proxy log files, oldest first.
fn har_entries(d: &Daemon) -> Vec<Value> {
    let dir = d.dir.path().join("logs/proxy");
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    names
        .iter()
        .flat_map(|p| {
            let har: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
            har["log"]["entries"].as_array().unwrap().clone()
        })
        .collect()
}

/// GET a path of the viewer at proxy.localhost on the daemon's HTTP port.
fn viewer_get(http: u16, path: &str) -> (String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", http)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: proxy.localhost\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
    (head.to_string(), body.to_string())
}

// A client's port binds with the proxy, its requests carry `_client`, the
// main port's do not, and get_proxy gives the client's own settings.
#[test]
fn a_proxy_client_has_its_own_port_and_its_name_in_the_log() {
    let d = Daemon::start();
    let mut c = d.client();
    let main = proxy_on(&mut c);
    c.call("set_config", json!({"proxy_clients": [{"name": "chrome", "port": 0}]}));
    let chrome = client_port(&mut c, "chrome").expect("the client's port is bound");
    assert_ne!(chrome, main);
    let s = c.call("status", json!({}));
    assert_eq!(s["proxy"]["clients"][0]["bound"], json!([format!("127.0.0.1:{chrome}"), format!("[::1]:{chrome}")]));
    assert_eq!(read_config(&d)["proxy_clients"], json!([{"name": "chrome", "port": 0}]));

    let http = s["http"]["port"].as_u64().unwrap() as u16;
    get_through(main, http, "/from-main");
    get_through(chrome, http, "/from-chrome");
    wait_written(&mut c, 2);
    let entries = har_entries(&d);
    let by_path = |p: &str| entries.iter().find(|e| e["request"]["url"].as_str().unwrap().ends_with(p)).unwrap().clone();
    assert_eq!(by_path("/from-chrome")["_client"], "chrome");
    assert!(by_path("/from-main").get("_client").is_none(), "the main port writes no _client");

    let p = c.call("get_proxy", json!({"client": "chrome"}));
    assert_eq!(p["client"], "chrome");
    assert_eq!(p["url"], format!("http://127.0.0.1:{chrome}"));
    assert_eq!(p["env"]["HTTPS_PROXY"], format!("http://127.0.0.1:{chrome}"));
    let profile = p["chrome_args"][0].as_str().unwrap();
    assert!(profile.starts_with("--user-data-dir=") && profile.ends_with("/chrome-proxy-chrome"), "{profile}");
    assert!(p["log"]["url"].as_str().unwrap().ends_with("/chrome"), "{p}");
    let main_reply = c.call("get_proxy", json!({}));
    assert!(main_reply.get("client").is_none());
    assert_eq!(main_reply["url"], format!("http://127.0.0.1:{main}"));
    assert_eq!(c.call("get_proxy", json!({"client": "default"}))["url"], main_reply["url"]);
    assert_eq!(c.error_code("get_proxy", json!({"client": "nobody"})), "not_found");
}

// The loop check knows every proxy port: a request on one port for another
// port of the same proxy is answered 508 and not sent on.
#[test]
fn a_request_for_another_proxy_port_is_a_loop() {
    let d = Daemon::start();
    let mut c = d.client();
    let main = proxy_on(&mut c);
    c.call("set_config", json!({"proxy_clients": [{"name": "agent", "port": 0}]}));
    let agent = client_port(&mut c, "agent").unwrap();
    assert!(get_through(main, agent, "/x").starts_with("HTTP/1.1 508"));
    assert!(get_through(agent, main, "/x").starts_with("HTTP/1.1 508"));
}

// Off closes the clients' ports and their connections; on binds them again;
// removing a client closes its port and keeps the others.
#[test]
fn client_ports_follow_the_proxy_switch_and_the_list() {
    let d = Daemon::start();
    let mut c = d.client();
    proxy_on(&mut c);
    c.call("set_config", json!({"proxy_clients": [{"name": "a", "port": 0}, {"name": "b", "port": 0}]}));
    let a = client_port(&mut c, "a").unwrap();
    let target = echo_server();
    let mut tunnel = connect_through(a, &format!("127.0.0.1:{target}"));

    c.call("set_config", json!({"proxy_enabled": false}));
    let mut buf = [0u8; 4];
    assert_eq!(tunnel.read(&mut buf).unwrap_or(0), 0, "the open tunnel is closed");
    std::thread::sleep(Duration::from_millis(100));
    assert!(TcpStream::connect(("127.0.0.1", a)).is_err(), "the client's listener is closed");
    assert_eq!(client_port(&mut c, "a"), None);
    assert_eq!(c.call("status", json!({}))["proxy"]["clients"][0]["errors"], json!([]));

    c.call("set_config", json!({"proxy_enabled": true}));
    let a = client_port(&mut c, "a").expect("bound again");
    let b = client_port(&mut c, "b").expect("bound again");
    c.call("set_config", json!({"proxy_clients": [{"name": "b", "port": 0}]}));
    std::thread::sleep(Duration::from_millis(100));
    assert!(TcpStream::connect(("127.0.0.1", a)).is_err(), "a removed client's port is closed");
    assert_eq!(client_port(&mut c, "b"), Some(b), "the other client keeps its port");
}

// A client the call adds on a taken port fails the call and changes nothing,
// as the main port does (I12).
#[test]
fn a_taken_client_port_changes_nothing() {
    let (port, _held) = (20000 + (std::process::id() % 5000) as u16..30000)
        .find_map(|p| {
            let v4 = std::net::TcpListener::bind(("127.0.0.1", p)).ok()?;
            let v6 = std::net::TcpListener::bind(("::1", p)).ok()?;
            drop(v4);
            Some((p, v6))
        })
        .expect("a free port");
    let d = Daemon::start();
    let mut c = d.client();
    proxy_on(&mut c);
    let before = read_config(&d);
    let v = c.raw("set_config", json!({"proxy_clients": [{"name": "chrome", "port": port}]}));
    assert_eq!(v["error"]["code"], "port_in_use", "{v}");
    assert!(v["error"]["message"].as_str().unwrap().starts_with("proxy client chrome: "), "{v}");
    assert_eq!(read_config(&d), before, "config.json is not written");
    assert_eq!(c.call("get_config", json!({}))["proxy_clients"], json!([]));
}

// Names and ports are checked before anything is bound or written.
#[test]
fn proxy_client_names_and_ports_are_checked() {
    let d = Daemon::start();
    let mut c = d.client();
    let http = c.call("status", json!({}))["http"]["port"].as_u64().unwrap();
    c.call("set_config", json!({"http_port": http, "proxy_port": 18877}));
    for (list, says) in [
        (json!([{"name": "Chrome", "port": 0}]), "a-z, 0-9"),
        (json!([{"name": "default", "port": 0}]), "reserved"),
        (json!([{"name": "api", "port": 0}]), "reserved"),
        (json!([{"name": "a", "port": 0}, {"name": "a", "port": 0}]), "twice"),
        (json!([{"name": "a", "port": 18900}, {"name": "b", "port": 18900}]), "both have port 18900"),
        (json!([{"name": "a", "port": http}]), "http_port"),
        (json!([{"name": "a", "port": 18877}]), "proxy_port"),
    ] {
        let v = c.raw("set_config", json!({"proxy_clients": list}));
        assert_eq!(v["error"]["code"], "invalid_request", "{list}");
        assert!(v["error"]["message"].as_str().unwrap().contains(says), "{list}: {v}");
    }
    c.call("set_config", json!({"proxy_clients": [{"name": "a", "port": 18900}]}));
    assert_eq!(c.raw("set_config", json!({"proxy_port": 18900}))["error"]["code"], "invalid_request", "the main port may not take a client's");
}

// Saved clients bind at start, with the saved proxy.
#[test]
fn saved_proxy_clients_bind_at_start() {
    let d = Daemon::start_with(|dir| {
        let mut config: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
        config["proxy_enabled"] = json!(true);
        config["proxy_port"] = json!(0);
        config["proxy_clients"] = json!([{"name": "agent-1", "port": 0}]);
        std::fs::write(dir.join("config.json"), config.to_string()).unwrap();
    });
    let mut c = d.client();
    let port = client_port(&mut c, "agent-1").expect("bound at start");
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
}

// The viewer lists the clients, filters entries by client, and serves the
// page at /<client>.
#[test]
fn the_viewer_lists_and_filters_by_client() {
    let d = Daemon::start();
    let mut c = d.client();
    let main = proxy_on(&mut c);
    c.call("set_config", json!({"proxy_clients": [{"name": "chrome", "port": 0}]}));
    let chrome = client_port(&mut c, "chrome").unwrap();
    let http = c.call("status", json!({}))["http"]["port"].as_u64().unwrap() as u16;
    get_through(main, http, "/m1");
    get_through(chrome, http, "/c1");
    get_through(main, http, "/m2");
    let log = wait_written(&mut c, 3);
    let file = log["current"].as_str().unwrap().to_string();

    let (_, body) = viewer_get(http, "/api/files");
    let files: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(files["clients"], json!([{"name": "default", "port": main}, {"name": "chrome", "port": chrome}]));

    let paths = |client: &str| -> Vec<String> {
        let (_, body) = viewer_get(http, &format!("/api/entries?file={file}&client={client}"));
        let page: Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("{e}: {body}"));
        page["entries"].as_array().unwrap().iter().map(|e| e["request"]["url"].as_str().unwrap().rsplit('/').next().unwrap().to_string()).collect()
    };
    assert_eq!(paths("chrome"), ["c1"]);
    assert_eq!(paths("default"), ["m2", "m1"]);
    let (head, _) = viewer_get(http, &format!("/api/entries?file={file}&client=No%20such"));
    assert!(head.starts_with("HTTP/1.1 400"), "{head}");

    let (head, body) = viewer_get(http, "/chrome");
    assert!(head.starts_with("HTTP/1.1 200") && body.contains("viewer.js"), "{head}");
    let (head, _) = viewer_get(http, "/chrome/");
    assert!(head.starts_with("HTTP/1.1 302") && head.to_ascii_lowercase().contains("location: ../chrome"), "{head}");
}
