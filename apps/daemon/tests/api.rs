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

    /// Start with ports 0; `prepare` may write files into the folder first.
    fn start_with(prepare: impl FnOnce(&Path)) -> Self {
        let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.json"),
            json!({"version":1,"http_port":0,"https_port":0,"fallback":true,"allow_lan":false,"log_size":100}).to_string(),
        )
        .unwrap();
        prepare(dir.path());
        let child = Command::new(env!("CARGO_BIN_EXE_localrouterd"))
            .env("LOCALROUTER_HOME", dir.path())
            .env("LOCALROUTER_LOG", "warn")
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
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
