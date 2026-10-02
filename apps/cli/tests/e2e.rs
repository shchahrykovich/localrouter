//! E1: the main journey through the real binaries, with checks along the way.
//! E1b (ADR 03): the path-route journey, the same way.
//! E1c (ADR 04): a dev instance next to the release, the same way.
//! ADR 08: E1 `proxy_log_journey` (a request in a HAR file and in the
//! viewer) and E2 `agent_loop` (operate, run, inspect, as an agent would).
//!
//! Replaced in CI (covered by manual tests M1, M3, M6): ports 80/443 (random
//! ports), the macOS `*.localhost` resolver (reqwest `resolve`), keychain trust
//! (ca.pem passed as the only root), and SMAppService (daemon spawned directly).

mod common;

use std::borrow::Cow;
use std::io::{Read, Write};
use std::net::SocketAddr;

use common::Daemon;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// HTTP upstream that answers with the Host header and path it saw.
async fn echo_upstream() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let host = req
                    .lines()
                    .find_map(|l| l.strip_prefix("host: ").or_else(|| l.strip_prefix("Host: ")))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let body = json!({"host": host, "path": path}).to_string();
                let resp = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    port
}

fn tcp_echo() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            std::thread::spawn(move || {
                let mut buf = [0u8; 256];
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

#[tokio::test(flavor = "multi_thread")]
async fn main_journey() {
    // 1-2. Daemon on random ports. Check: both ports bound, CA created.
    let d = Daemon::start();
    let status = d.status();
    let http = status["http"]["port"].as_u64().unwrap() as u16;
    let https = status["https"]["port"].as_u64().unwrap() as u16;
    assert_eq!(status["ca"]["state"], "ok");

    // 3. Upstream on a random port.
    let up = echo_upstream().await;

    // 4. Register through MCP. Check: URL and replaced=false.
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_localrouter"));
    cmd.arg("mcp").env("LOCALROUTER_HOME", d.home());
    let mcp = ().serve(TokioChildProcess::new(cmd).unwrap()).await.unwrap();
    let args = json!({"host": "shop", "port": up, "note": "e2e"});
    let r = mcp
        .call_tool(CallToolRequestParams::new(Cow::Borrowed("register_route")).with_arguments(args.as_object().unwrap().clone()))
        .await
        .unwrap();
    let text = serde_json::to_value(&r.content).unwrap()[0]["text"].as_str().unwrap().to_string();
    let reg: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reg["replaced"], false);
    assert_eq!(reg["route"]["urls"][0], format!("https://shop.localhost:{https}"));

    // 5. List through the CLI. Check: upstream up.
    let (ok, out, _) = d.cli(&["list", "--json"]);
    assert!(ok);
    let list: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["routes"][0]["upstream_up"], true);

    // 6. HTTPS to a subdomain, trusting only ca.pem. Check: fallback to shop, Host unchanged.
    let pem = std::fs::read(d.home().join("ca/ca.pem")).unwrap();
    let client = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(&pem).unwrap()])
        .resolve("feat-login.shop.localhost", SocketAddr::from(([127, 0, 0, 1], https)))
        .build()
        .unwrap();
    let url = format!("https://feat-login.shop.localhost:{https}/hello?x=1");
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert_eq!(body["host"], format!("feat-login.shop.localhost:{https}"));
    assert_eq!(body["path"], "/hello?x=1");

    // 7. Logs. Check: one entry, path without the query.
    let (_, out, _) = d.cli(&["logs"]);
    assert!(out.contains("feat-login.shop.localhost/hello") && !out.contains("x=1"), "{out}");

    // 8. Unregister, then HTTPS again. Check: no route, no certificate, handshake refused.
    assert!(d.cli(&["rm", "shop"]).0);
    let fresh = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(&pem).unwrap()])
        .resolve("feat-login.shop.localhost", SocketAddr::from(([127, 0, 0, 1], https)))
        .build()
        .unwrap();
    assert!(fresh.get(&url).send().await.is_err());

    // 9. Plain HTTP. Check: 404 page.
    let plain = reqwest::Client::builder()
        .resolve("shop.localhost", SocketAddr::from(([127, 0, 0, 1], http)))
        .build()
        .unwrap();
    let resp = plain.get(format!("http://shop.localhost:{http}/")).send().await.unwrap();
    assert_eq!(resp.status(), 404);

    // 10. TCP route with listen_port 0. Check: the reply holds the real port.
    let target = tcp_echo();
    let (ok, out, err) = d.cli(&["add", "db.shop", &target.to_string(), "--tcp", "--session"]);
    assert!(ok, "{err}");
    let listen: u16 = out.lines().nth(1).unwrap().trim().rsplit(':').next().unwrap().parse().unwrap();

    // 11. Send ping. Check: ping comes back; one TCP log entry after close.
    {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", listen)).unwrap();
        s.write_all(b"ping").unwrap();
        let mut buf = [0u8; 4];
        s.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
    }
    let mut seen = false;
    for _ in 0..50 {
        let (_, out, _) = d.cli(&["logs", "db.shop"]);
        if out.contains(&format!("tcp db.shop:{listen} in 4 B, out 4 B")) {
            seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(seen, "no TCP log entry");

    // 12. Unregister. Check: new connections are refused.
    assert!(d.cli(&["rm", "db.shop"]).0);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(std::net::TcpStream::connect(("127.0.0.1", listen)).is_err());
    mcp.cancel().await.unwrap();
}

/// HTTP upstream that answers with its tag, the path and X-Forwarded-Prefix.
/// Aborting the handle closes the port.
async fn tagged_upstream(tag: &'static str) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let prefix = req
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("x-forwarded-prefix: ").map(str::to_string))
                    .unwrap_or_default();
                let body = json!({"tag": tag, "path": path, "prefix": prefix.trim()}).to_string();
                let resp = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    (port, task)
}

struct PathJourney {
    d: Daemon,
    http: u16,
    https: u16,
    mcp: rmcp::service::RunningService<rmcp::RoleClient, ()>,
}

impl PathJourney {
    async fn mcp(&self, tool: &'static str, args: Value) -> Value {
        let r = self
            .mcp
            .call_tool(CallToolRequestParams::new(Cow::Borrowed(tool)).with_arguments(args.as_object().unwrap().clone()))
            .await
            .unwrap();
        let text = serde_json::to_value(&r.content).unwrap()[0]["text"].as_str().unwrap().to_string();
        assert_ne!(r.is_error, Some(true), "{tool}: {text}");
        serde_json::from_str(&text).unwrap()
    }

    /// GET http://<host>.localhost<path>; the status and the body.
    async fn get(&self, host: &str, path: &str) -> (u16, String) {
        let url = format!("http://127.0.0.1:{}{path}", self.http);
        let r = reqwest::Client::new().get(url).header("host", format!("{host}.localhost")).send().await.unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    /// The request reaches `tag` and the log names `route`; `localrouter which`
    /// names the same route before any request (I33).
    async fn check(&self, host: &str, path: &str, tag: &str, route: &str) -> Value {
        let (_, which, err) = self.d.cli(&["which", &format!("{host}.localhost{path}")]);
        let asked = format!("{host}.localhost{}", path.split('?').next().unwrap());
        assert!(which.starts_with(&format!("{asked} -> {route} -> ")), "which {asked}: {which}{err}");
        let (status, body) = self.get(host, path).await;
        assert_eq!(status, 200, "{host}{path}: {body}");
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["tag"], tag, "{host}{path}");
        let logs = self.mcp("get_logs", json!({"limit": 1})).await;
        assert_eq!(logs["entries"][0]["route"], route, "log route for {host}{path}");
        body
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn path_journey() {
    // 1. Daemon and four upstreams: main, blog, api, and a branch of the blog.
    let d = Daemon::start();
    let status = d.status();
    let http = status["http"]["port"].as_u64().unwrap() as u16;
    let https = status["https"]["port"].as_u64().unwrap() as u16;
    let (main, _m) = tagged_upstream("main").await;
    let (blog, blog_task) = tagged_upstream("blog").await;
    let (api, _a) = tagged_upstream("api").await;
    let (branch, _b) = tagged_upstream("branch").await;
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_localrouter"));
    cmd.arg("mcp").env("LOCALROUTER_HOME", d.home());
    let mcp = ().serve(TokioChildProcess::new(cmd).unwrap()).await.unwrap();
    let j = PathJourney { d, http, https, mcp };

    // 2. CLI: the main app. Check: /blog still reaches it.
    assert!(j.d.cli(&["add", "shop", &main.to_string()]).0);
    j.check("shop", "/blog", "main", "shop").await;

    // 3. MCP: the blog on /blog. Check: /blog/x reaches the blog with its path, /x the main app.
    let reg = j.mcp("register_route", json!({"host": "shop", "path": "/blog", "port": blog})).await;
    assert_eq!(reg["route"]["path"], "/blog");
    assert_eq!(j.check("shop", "/blog/x", "blog", "shop/blog").await["path"], "/blog/x");
    j.check("shop", "/x", "main", "shop").await;
    j.check("shop", "/blogger", "main", "shop").await;

    // 4. CLI: an API that answers at /. Check: prefix removed, sent as a header.
    let (ok, _, err) = j.d.cli(&["add", "shop", &api.to_string(), "--path", "/api", "--strip-path"]);
    assert!(ok, "{err}");
    let body = j.check("shop", "/api/users?x=1", "api", "shop/api").await;
    assert_eq!(body["path"], "/users?x=1");
    assert_eq!(body["prefix"], "/api");

    // 5. HTTPS through the daemon's own certificate hook, also for a name that
    //    has only a path route (I27).
    assert!(j.d.cli(&["add", "docs", &main.to_string(), "--path", "/guide", "--session"]).0);
    let pem = std::fs::read(j.d.home().join("ca/ca.pem")).unwrap();
    for (name, path, tag) in [("shop.localhost", "/blog/1", "blog"), ("docs.localhost", "/guide", "main")] {
        let client = reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(&pem).unwrap()])
            .resolve(name, SocketAddr::from(([127, 0, 0, 1], j.https)))
            .build()
            .unwrap();
        let resp = client.get(format!("https://{name}:{}{path}", j.https)).send().await.unwrap();
        assert_eq!(resp.status(), 200, "{name}{path}");
        let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
        assert_eq!(body["tag"], tag, "{name}{path}");
    }

    // 6. A worktree of the blog on a branch name, owned by its dev server.
    //    Check: its /blog reaches the branch, every other path the main app.
    let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
    j.mcp("register_route", json!({"host": "feat-x.shop", "path": "/blog", "port": branch, "owner_pid": child.id()})).await;
    j.check("feat-x.shop", "/blog/post-1", "branch", "feat-x.shop/blog").await;
    j.check("feat-x.shop", "/products", "main", "shop").await;

    // 7. The branch's dev server exits. Check within 1 s: the branch name's
    //    /blog falls back to the main blog, and shop/blog is still there.
    child.kill().unwrap();
    child.wait().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let list = j.mcp("list_routes", json!({})).await;
        let hosts: Vec<String> = list["routes"].as_array().unwrap().iter().map(|r| r["host"].as_str().unwrap().to_string()).collect();
        if !hosts.contains(&"feat-x.shop".to_string()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "owned path route still there after 1 s");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    j.check("feat-x.shop", "/blog/post-1", "blog", "shop/blog").await;

    // 8. The blog stops. Check: /blog answers 502 naming the blog, never the main app (I25).
    blog_task.abort();
    let _ = blog_task.await;
    let (status, body) = j.get("shop", "/blog/x").await;
    assert_eq!(status, 502, "{body}");
    assert!(body.contains(&blog.to_string()), "{body}");

    // 9. Logs name the routes.
    let (_, out, _) = j.d.cli(&["logs", "shop"]);
    for route in ["route shop/blog", "route shop\n", "route shop/api"] {
        assert!(out.contains(route), "{route}: {out}");
    }

    // 10. rm shop without a path. Check: the path routes remain; /x has no route.
    let (ok, out, _) = j.d.cli(&["rm", "shop"]);
    assert!(ok);
    assert_eq!(out.trim(), "removed shop; shop/api, shop/blog remain");
    assert_eq!(j.get("shop", "/x").await.0, 404);
    j.mcp.cancel().await.unwrap();
}

/// GET https://<name>:<port><path>, trusting only `ca`.
async fn https_get(ca: &std::path::Path, name: &str, port: u16, path: &str) -> Value {
    let pem = std::fs::read(ca).unwrap();
    let client = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(&pem).unwrap()])
        .resolve(name, SocketAddr::from(([127, 0, 0, 1], port)))
        .build()
        .unwrap();
    let resp = client.get(format!("https://{name}:{port}{path}")).send().await.unwrap();
    assert_eq!(resp.status(), 200, "{name}:{port}{path}");
    serde_json::from_str(&resp.text().await.unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn instances_journey() {
    // 1. Two dev servers with the same name in both instances.
    let (release_server, _r) = tagged_upstream("release").await;
    let (dev_server, _d) = tagged_upstream("dev").await;

    // 2. The release daemon and a dev daemon, each with its own folder.
    //    Check: both run at once; the dev one does not exit "already running".
    let release = Daemon::start();
    let (_bin, dev_daemon) = common::renamed_daemon("-dev");
    let dev = Daemon::start_program(&dev_daemon, &[]);
    let (_cli_bin, dev_cli) = common::renamed_cli("-dev");

    // 3. The same name in each. Check: each list shows only its own route.
    assert!(release.cli(&["add", "shop", &release_server.to_string()]).0);
    let (ok, _, err) = dev.cli_as(&dev_cli, &["add", "shop", &dev_server.to_string()]);
    assert!(ok, "{err}");
    let (_, release_list, _) = release.cli(&["list", "--json"]);
    let (_, dev_list, _) = dev.cli_as(&dev_cli, &["list", "--json"]);
    let target = |list: &str| serde_json::from_str::<Value>(list).unwrap()["routes"][0]["target"].clone();
    assert_eq!(target(&release_list), json!(format!("http://127.0.0.1:{release_server}")));
    assert_eq!(target(&dev_list), json!(format!("http://127.0.0.1:{dev_server}")));

    // 4. One name, two ports: the port picks the instance.
    let port = |d: &Daemon, scheme: &str| d.status()[scheme]["port"].as_u64().unwrap() as u16;
    let release_https = port(&release, "https");
    let dev_https = port(&dev, "https");
    assert_eq!(https_get(&release.home().join("ca/ca.pem"), "shop.localhost", release_https, "/").await["tag"], "release");
    assert_eq!(https_get(&dev.home().join("ca/ca.pem"), "shop.localhost", dev_https, "/").await["tag"], "dev");

    // 5. The dev help page names the dev CLI and its own port.
    let dev_http = port(&dev, "http");
    let page = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{dev_http}/"))
        .header("host", "router.localhost")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("`localrouter-dev list`"), "{page}");
    assert!(page.contains(&format!("curl -s http://router.localhost:{dev_http}`")), "{page}");
    assert!(page.contains("LocalRouter-dev CA "), "the dev CA carries the instance name: {page}");

    // 6. The release stops. Check: the dev instance still answers.
    drop(release);
    assert_eq!(https_get(&dev.home().join("ca/ca.pem"), "shop.localhost", dev_https, "/x").await["path"], "/x");
}

/// HTTPS echo server for `test.example`, signed by a fresh test CA. Returns
/// its port and the CA certificate as PEM.
async fn test_internet_server() -> (u16, String) {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.distinguished_name.push(rcgen::DnType::CommonName, "E1 Internet CA");
    let ca_pem = ca_params.self_signed(&ca_key).unwrap().pem();
    let issuer = rcgen::Issuer::new(ca_params, ca_key);
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["test.example".into()]).unwrap().signed_by(&key, &issuer).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()))
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (s, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut s) = acceptor.accept(s).await else { return };
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let body = json!({"server": "test.example", "path": path}).to_string();
                let resp = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    (port, ca_pem)
}

fn via_proxy(proxy: &str, ca_pem: &[u8]) -> reqwest::Client {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy).unwrap())
        .tls_certs_only([reqwest::Certificate::from_pem(ca_pem).unwrap()])
        .build()
        .unwrap()
}

/// Wait until `localrouter logs` shows a line with every word.
async fn wait_for_log(d: &Daemon, words: &[&str]) -> String {
    for _ in 0..50 {
        let (_, out, _) = d.cli(&["logs"]);
        if let Some(line) = out.lines().find(|l| words.iter().all(|w| l.contains(w))) {
            return line.to_string();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("no log line with {words:?}: {}", d.cli(&["logs"]).1);
}

/// E1d (ADR 06): the forward proxy through the real binaries.
///
/// Replaced: the internet is a local HTTPS server with a test CA; DNS and the
/// macOS trust store are replaced by the daemon's debug-only test settings;
/// keychain trust is the inspection ca.pem given to reqwest. M1 and M2 use
/// the real ones.
#[tokio::test(flavor = "multi_thread")]
async fn proxy_journey() {
    // 1. A test internet server, an echo server, and the daemon that sends
    //    test.example to the test server and trusts only the test CA.
    let (server, internet_ca) = test_internet_server().await;
    let echo = echo_upstream().await;
    let ca_file = tempfile::Builder::new().prefix("lrca").suffix(".pem").tempfile().unwrap();
    std::fs::write(ca_file.path(), &internet_ca).unwrap();
    let resolve = format!("test.example=127.0.0.1:{server}");
    let d = Daemon::start_env(&[
        ("LOCALROUTER_TEST_RESOLVE", &resolve),
        ("LOCALROUTER_TEST_UPSTREAM_CA", ca_file.path().to_str().unwrap()),
    ]);

    // 2. Proxy on. Check: two loopback addresses.
    assert!(d.cli(&["proxy", "port", "0"]).0);
    let (ok, _, err) = d.cli(&["proxy", "on"]);
    assert!(ok, "{err}");
    let status = d.status();
    let port = status["proxy"]["port"].as_u64().unwrap();
    assert_eq!(status["proxy"]["bound"], json!([format!("127.0.0.1:{port}"), format!("[::1]:{port}")]));
    let (_, out, _) = d.cli(&["proxy"]);
    assert!(out.contains(&format!("on: http://127.0.0.1:{port}")), "{out}");
    let proxy = format!("http://127.0.0.1:{port}");

    // 3. Plain HTTP through the proxy. Check: the echo saw /x; the log says via proxy.
    let client = via_proxy(&proxy, internet_ca.as_bytes());
    let resp = client.get(format!("http://127.0.0.1:{echo}/x?q=1")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert_eq!(body["path"], "/x?q=1");
    let line = wait_for_log(&d, &["GET", "/x", "via proxy (http)"]).await;
    assert!(!line.contains("q=1"), "{line}");

    // 4. HTTPS, no inspect host: a tunnel. The client trusts the test CA and
    //    gets the real server. Check: one tunnel entry.
    let resp = client.get("https://test.example/t").send().await.unwrap();
    assert_eq!(resp.status(), 200);
    wait_for_log(&d, &["CONNECT", "test.example", "via proxy (tunnel)"]).await;

    // 5. Inspect test.example. Check: inspect-ca/ exists; a client that trusts
    //    only the inspection CA gets 200; an inspect entry with path /i.
    let (ok, _, err) = d.cli(&["proxy", "inspect", "add", "test.example"]);
    assert!(ok, "{err}");
    let inspection_pem = std::fs::read(d.home().join("inspect-ca/ca.pem")).unwrap();
    let inspected = via_proxy(&proxy, &inspection_pem);
    let resp = inspected.get("https://test.example/i?secret=1").send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert_eq!(body["server"], "test.example", "the real server answered");
    assert_eq!(body["path"], "/i?secret=1");
    let line = wait_for_log(&d, &["GET", "test.example/i", "via proxy (inspect)"]).await;
    assert!(!line.contains("secret"), "{line}");

    // 6. MCP get_proxy. Check: test.example is inspected, and
    //    NODE_EXTRA_CA_CERTS is the file used in step 5.
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_localrouter"));
    cmd.arg("mcp").env("LOCALROUTER_HOME", d.home());
    let mcp = ().serve(TokioChildProcess::new(cmd).unwrap()).await.unwrap();
    let r = mcp.call_tool(CallToolRequestParams::new(Cow::Borrowed("get_proxy"))).await.unwrap();
    let text = serde_json::to_value(&r.content).unwrap()[0]["text"].as_str().unwrap().to_string();
    let p: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(p["inspect_set"], json!(["test.example"]));
    assert_eq!(p["env"]["NODE_EXTRA_CA_CERTS"], d.home().join("inspect-ca/ca.pem").display().to_string());
    mcp.cancel().await.unwrap();

    // 7. Proxy off. Check: the next request through it fails to connect.
    assert!(d.cli(&["proxy", "off"]).0);
    let fresh = via_proxy(&proxy, internet_ca.as_bytes());
    let e = fresh.get(format!("http://127.0.0.1:{echo}/after")).send().await.unwrap_err();
    assert!(e.is_connect(), "{e:?}");
}

/// HTTPS server for `api.test.example`, signed by a fresh test CA, that reads
/// one request with its body and answers with three server-sent events.
/// Returns its port and the CA certificate as PEM.
async fn streaming_api_server() -> (u16, String) {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.distinguished_name.push(rcgen::DnType::CommonName, "E1e Internet CA");
    let ca_pem = ca_params.self_signed(&ca_key).unwrap().pem();
    let issuer = rcgen::Issuer::new(ca_params, ca_key);
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["api.test.example".into()]).unwrap().signed_by(&key, &issuer).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()))
        .unwrap();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (s, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut s) = acceptor.accept(s).await else { return };
                // Read the head, then Content-Length bytes of body.
                let mut buf = vec![];
                let mut byte = [0u8; 1];
                while !buf.ends_with(b"\r\n\r\n") {
                    if s.read(&mut byte).await.unwrap_or(0) == 0 {
                        return;
                    }
                    buf.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                let len: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                    .unwrap_or(0);
                let mut body = vec![0u8; len];
                let _ = s.read_exact(&mut body).await;
                let _ = s
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n")
                    .await;
                for i in 1..=3 {
                    let event = format!("data: token {i}\n\n");
                    let _ = s.write_all(format!("{:x}\r\n{event}\r\n", event.len()).as_bytes()).await;
                    let _ = s.flush().await;
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                let _ = s.write_all(b"0\r\n\r\n").await;
                let _ = s.shutdown().await;
            });
        }
    });
    (port, ca_pem)
}

async fn mcp_call(mcp: &rmcp::service::RunningService<rmcp::RoleClient, ()>, name: &'static str, args: Value) -> (bool, String) {
    let params = CallToolRequestParams::new(Cow::Borrowed(name)).with_arguments(args.as_object().unwrap().clone());
    let r = mcp.call_tool(params).await.unwrap();
    let text = serde_json::to_value(&r.content).unwrap()[0]["text"].as_str().unwrap_or_default().to_string();
    (r.is_error != Some(true), text)
}

/// E1e (ADR 07): an agent-like journey with script rules: check, set, traffic,
/// capture, remove.
///
/// Replaced: the internet is a local HTTPS server that streams three events;
/// DNS and the macOS trust store are the daemon's debug-only test settings;
/// keychain trust is the inspection ca.pem given to reqwest. M1 uses Claude
/// Code and the real API.
#[tokio::test(flavor = "multi_thread")]
async fn script_rules_journey() {
    // 1. The daemon, with the proxy on, and the test server for api.test.example.
    let (server, internet_ca) = streaming_api_server().await;
    let ca_file = tempfile::Builder::new().prefix("lrca").suffix(".pem").tempfile().unwrap();
    std::fs::write(ca_file.path(), &internet_ca).unwrap();
    let resolve = format!("api.test.example=127.0.0.1:{server}");
    let d = Daemon::start_env(&[
        ("LOCALROUTER_TEST_RESOLVE", &resolve),
        ("LOCALROUTER_TEST_UPSTREAM_CA", ca_file.path().to_str().unwrap()),
    ]);
    assert!(d.cli(&["proxy", "port", "0"]).0);
    assert!(d.cli(&["proxy", "on"]).0);
    let proxy = format!("http://127.0.0.1:{}", d.status()["proxy"]["port"].as_u64().unwrap());

    // 2. A log script and a broken one, in a folder of their own.
    let work = tempfile::Builder::new().prefix("lrw").tempdir().unwrap();
    let cap = work.path().join("cap.lua");
    std::fs::write(
        &cap,
        r#"return { kind = "log", on_exchange = function(ex)
            capture.append("calls.jsonl", json.encode({
              path = ex.request.path, request = ex.request.body, response = ex.response.body,
              auth = ex.request.headers.authorization, status = ex.response.status,
            }) .. "\n")
          end }"#,
    )
    .unwrap();
    let bad = work.path().join("bad.lua");
    std::fs::write(&bad, "return {\n  kind = 'log',\n  on_exchange = function(ex) capture.append( end }").unwrap();
    let out = work.path().join("out");
    std::fs::create_dir(&out).unwrap();

    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_localrouter"));
    cmd.arg("mcp").env("LOCALROUTER_HOME", d.home());
    let mcp = ().serve(TokioChildProcess::new(cmd).unwrap()).await.unwrap();

    // 3. check_only with the broken script. Check: the error has the line; no rule.
    let (ok, text) =
        mcp_call(&mcp, "set_script_rule", json!({"id": "cap", "host": "api.test.example", "script": bad, "output_dir": out, "check_only": true})).await;
    assert!(!ok && text.contains("bad.lua:3:"), "{text}");
    assert!(d.cli(&["rules", "--json"]).1.contains("\"rules\": []"));

    // 4. The real rule, owned by a helper process. Check: the host is now
    //    inspected and the inspection CA was made for it.
    let mut helper = std::process::Command::new("sleep").arg("60").spawn().unwrap();
    let (ok, text) = mcp_call(
        &mcp,
        "set_script_rule",
        json!({"id": "cap", "host": "api.test.example", "path": "/v1", "script": cap, "output_dir": out, "owner_pid": helper.id()}),
    )
    .await;
    assert!(ok, "{text}");
    let set: Value = serde_json::from_str(&text).unwrap();
    assert_eq!((set["inspect"]["host_added"].as_bool(), set["inspect"]["ca_created"].as_bool()), (Some(true), Some(true)));

    // 5. POST through the proxy, trusting the inspection CA. Check: the three
    //    events arrive unchanged.
    let inspection_pem = std::fs::read(d.home().join("inspect-ca/ca.pem")).unwrap();
    let client = via_proxy(&proxy, &inspection_pem);
    let resp = client
        .post("https://api.test.example/v1/messages")
        .header("authorization", "Bearer sk-test-123")
        .header("content-type", "application/json")
        .body(r#"{"prompt":"hello"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "data: token 1\n\ndata: token 2\n\ndata: token 3\n\n");

    // 6. The capture file: the request body, the decoded events, and the
    //    authorization header redacted.
    let file = out.join("calls.jsonl");
    let mut line = String::new();
    for _ in 0..150 {
        line = std::fs::read_to_string(&file).unwrap_or_default();
        if !line.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let call: Value = serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("{e}: {line:?}"));
    assert_eq!(call["request"], r#"{"prompt":"hello"}"#);
    assert_eq!(call["response"], "data: token 1\n\ndata: token 2\n\ndata: token 3\n\n");
    assert_eq!(call["auth"], "[redacted]");
    assert!(!line.contains("sk-test-123"));
    let rules: Value = serde_json::from_str(&d.cli(&["rules", "--json"]).1).unwrap();
    assert_eq!(rules["rules"][0]["matched"], 1);

    // 7. The log names the rule.
    wait_for_log(&d, &["POST", "api.test.example/v1/messages", "rules cap"]).await;

    // 8. The helper exits. Check: the rule is gone, its host left the inspect
    //    set, the capture file stays.
    helper.kill().unwrap();
    helper.wait().unwrap();
    let mut gone = false;
    for _ in 0..100 {
        let p: Value = serde_json::from_str(&mcp_call(&mcp, "get_proxy", json!({})).await.1).unwrap();
        if p["script_rules"] == json!([]) && p["inspect_set"] == json!([]) {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(gone, "the owned rule and its inspected host are still there");
    assert!(file.exists(), "captures are never removed");
    mcp.cancel().await.unwrap();
}

// ---- ADR 08

/// GET a path of the viewer on the daemon's HTTP port, from this Mac.
fn viewer_get(http: u16, path: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", http)).unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: proxy.localhost\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut raw = vec![];
    s.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_string())
}

/// E1 (ADR 08): a request through the proxy shows up in a HAR file and in
/// the viewer, with its query and with Authorization redacted.
///
/// Replaced: the internet server is a local echo server reached through the
/// daemon's debug-only resolver. M1 and M2 use real sites and real Chrome.
#[tokio::test(flavor = "multi_thread")]
async fn proxy_log_journey() {
    let echo = echo_upstream().await;
    let resolve = format!("example.test=127.0.0.1:{echo}");
    let d = Daemon::start_env(&[("LOCALROUTER_TEST_RESOLVE", &resolve)]);
    let folder = d.home().join("logs/proxy");

    // 1. Proxy on. Check: the Log: line names the temp folder.
    assert!(d.cli(&["proxy", "port", "0"]).0);
    let (ok, out, err) = d.cli(&["proxy", "on"]);
    assert!(ok, "{err}");
    assert!(out.contains(&format!("Log: on, writes every request to {}", folder.display())), "{out}");
    let status = d.status();
    let proxy = format!("http://127.0.0.1:{}", status["proxy"]["port"]);
    let http = status["http"]["port"].as_u64().unwrap() as u16;

    // 2. A request with a secret header and a query. Check: 200.
    let client = reqwest::Client::builder().proxy(reqwest::Proxy::all(&proxy).unwrap()).build().unwrap();
    let resp = client.get("http://example.test/a?x=1").header("authorization", "Bearer t").send().await.unwrap();
    assert_eq!(resp.status(), 200);

    // 3. The viewer's file list. Check: one current file with one entry.
    let mut files = Value::Null;
    for _ in 0..100 {
        files = serde_json::from_str(&viewer_get(http, "/api/files").1).unwrap();
        if files["files"][0]["entries"] == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(files["files"].as_array().unwrap().len(), 1, "{files}");
    assert_eq!(files["files"][0]["current"], true);
    let name = files["files"][0]["name"].as_str().unwrap().to_string();

    // 4. The file from the viewer. Check: HAR 1.2, the query, [redacted].
    let (status, body) = viewer_get(http, &format!("/files/{name}"));
    assert_eq!(status, 200);
    let har: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(har["log"]["version"], "1.2");
    let entry = &har["log"]["entries"][0];
    assert_eq!(entry["request"]["url"], "http://example.test/a?x=1");
    let auth = entry["request"]["headers"].as_array().unwrap().iter().find(|h| h["name"] == "authorization").unwrap();
    assert_eq!(auth["value"], "[redacted]");
    assert!(!body.contains("Bearer t"));

    // 5. The same bytes on disk.
    assert_eq!(std::fs::read_to_string(folder.join(&name)).unwrap(), body);

    // 6. Log off, one more request. Check: still one entry, still valid.
    assert!(d.cli(&["proxy", "log", "off"]).0);
    client.get("http://example.test/b").send().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let har: Value = serde_json::from_str(&std::fs::read_to_string(folder.join(&name)).unwrap()).unwrap();
    assert_eq!(har["log"]["entries"].as_array().unwrap().len(), 1);

    // 7. MCP get_proxy. Check: log.enabled false, the temp folder.
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_localrouter"));
    cmd.arg("mcp").env("LOCALROUTER_HOME", d.home());
    let mcp = ().serve(TokioChildProcess::new(cmd).unwrap()).await.unwrap();
    let (ok, text) = mcp_call(&mcp, "get_proxy", json!({})).await;
    assert!(ok, "{text}");
    let p: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(p["log"]["enabled"], false);
    assert_eq!(p["log"]["folder"], folder.display().to_string());
    mcp.cancel().await.unwrap();
}

/// HTTPS echo server for `names`, signed by a fresh test CA. Returns its
/// port and the CA as PEM.
async fn tls_server_for(names: &[&str], ca_name: &str) -> (u16, String) {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(vec![]).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.distinguished_name.push(rcgen::DnType::CommonName, ca_name);
    let ca_pem = ca_params.self_signed(&ca_key).unwrap().pem();
    let issuer = rcgen::Issuer::new(ca_params, ca_key);
    let key = rcgen::KeyPair::generate().unwrap();
    let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    let cert = rcgen::CertificateParams::new(names).unwrap().signed_by(&key, &issuer).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()))
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (s, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut s) = acceptor.accept(s).await else { return };
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let body = json!({"path": path}).to_string();
                let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    (port, ca_pem)
}

fn has(program: &str) -> bool {
    std::process::Command::new("/bin/sh").args(["-c", &format!("command -v {program}")]).output().is_ok_and(|o| o.status.success())
}

/// E2 (ADR 08): the agent loop through the CLI, as the agent texts give it:
/// operate (`proxy on`, `inspect add`), run (`curl` and `python3` after
/// `eval "$(… proxy env)"`), inspect (`jq` on the newest file).
///
/// Replaced: the internet servers are local HTTPS servers with a test CA,
/// which the daemon's debug-only settings trust and add to the bundle's roots.
#[tokio::test(flavor = "multi_thread")]
async fn agent_loop() {
    for program in ["curl", "python3", "jq"] {
        if !has(program) {
            eprintln!("agent_loop skipped: {program} is not installed");
            return;
        }
    }
    let (inspected, ca_pem) = tls_server_for(&["example.test"], "E2 Internet CA").await;
    let (tunnelled, ca2_pem) = tls_server_for(&["plain.test"], "E2 Second CA").await;
    let roots = tempfile::Builder::new().prefix("lrca").suffix(".pem").tempfile().unwrap();
    std::fs::write(roots.path(), format!("{ca_pem}{ca2_pem}")).unwrap();
    let resolve = format!("example.test=127.0.0.1:{inspected},plain.test=127.0.0.1:{tunnelled}");
    let roots_path = roots.path().to_str().unwrap();
    let d = Daemon::start_env(&[
        ("LOCALROUTER_TEST_RESOLVE", &resolve),
        ("LOCALROUTER_TEST_UPSTREAM_CA", roots_path),
        ("LOCALROUTER_TEST_EXTRA_ROOTS", roots_path),
    ]);
    let cli = env!("CARGO_BIN_EXE_localrouter");
    let sh = |script: &str| {
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", script])
            .env("LOCALROUTER_HOME", d.home())
            .env("CLI", cli)
            .env_remove("HTTPS_PROXY")
            .env_remove("HTTP_PROXY")
            .env_remove("https_proxy")
            .env_remove("http_proxy")
            .output()
            .unwrap();
        (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
    };

    // 1. Operate. Check: both exit 0.
    let (ok, _, err) = sh(r#""$CLI" proxy port 0 && "$CLI" proxy on && "$CLI" proxy inspect add example.test"#);
    assert!(ok, "{err}");

    // 2. curl to the inspected host: it trusts the inspection CA through
    //    CURL_CA_BUNDLE. Check: 200 from the server.
    let (ok, out, err) = sh(r#"eval "$("$CLI" proxy env)" && curl -sS https://example.test/curl?k=1"#);
    assert!(ok, "curl failed: {err}");
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["path"], "/curl?k=1", "{out}");

    // 3. Python through SSL_CERT_FILE. Check: 200.
    let (ok, out, err) = sh(
        r#"eval "$("$CLI" proxy env)" && python3 -c 'import urllib.request; r = urllib.request.urlopen("https://example.test/py"); print(r.status)'"#,
    );
    assert!(ok, "python3 failed: {err}");
    assert_eq!(out.trim(), "200");

    // 4. curl to a host that is not inspected: a tunnel to the real server,
    //    whose CA is in the bundle's root part. Check: 200.
    let (ok, out, err) = sh(r#"eval "$("$CLI" proxy env)" && curl -sS https://plain.test/tunnel"#);
    assert!(ok, "curl through the tunnel failed: {err}");
    assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["path"], "/tunnel");

    // 5. Inspect with jq, as the texts say. Check: the two inspected
    //    requests with their paths, and the tunnel as one CONNECT.
    let mut lines = String::new();
    for _ in 0..100 {
        let (ok, out, err) = sh(
            r#"f=$(ls -t "$("$CLI" proxy log path)"/proxy-*.har | head -1) && jq -r '.log.entries[] | "\(._mode) \(.request.method) \(.request.url)"' "$f""#,
        );
        assert!(ok || err.contains("parse error"), "{err}");
        lines = out;
        if lines.lines().count() >= 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(lines.contains("inspect GET https://example.test/curl?k=1"), "{lines}");
    assert!(lines.contains("inspect GET https://example.test/py"), "{lines}");
    assert!(lines.contains("tunnel CONNECT https://plain.test:443"), "{lines}");
}

