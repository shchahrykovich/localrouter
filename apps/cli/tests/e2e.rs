//! E1: the main journey through the real binaries, with checks along the way.
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
