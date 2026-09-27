//! T7: `localrouter mcp` with a real MCP client over stdio.

mod common;

use std::borrow::Cow;

use common::Daemon;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use tokio::process::Command;

async fn mcp_client(home: &std::path::Path) -> RunningService<rmcp::RoleClient, ()> {
    mcp_client_as(std::path::Path::new(env!("CARGO_BIN_EXE_localrouter")), home).await
}

async fn mcp_client_as(program: &std::path::Path, home: &std::path::Path) -> RunningService<rmcp::RoleClient, ()> {
    let mut cmd = Command::new(program);
    cmd.arg("mcp").env("LOCALROUTER_HOME", home);
    let transport = TokioChildProcess::new(cmd).unwrap();
    ().serve(transport).await.unwrap()
}

async fn call(client: &RunningService<rmcp::RoleClient, ()>, name: &'static str, args: Value) -> CallToolResult {
    let params = CallToolRequestParams::new(Cow::Borrowed(name)).with_arguments(args.as_object().unwrap().clone());
    client.call_tool(params).await.unwrap()
}

fn text(result: &CallToolResult) -> String {
    serde_json::to_value(&result.content).unwrap()[0]["text"].as_str().unwrap_or_default().to_string()
}

fn is_error(result: &CallToolResult) -> bool {
    result.is_error == Some(true)
}

// I13
#[tokio::test]
async fn exactly_six_tools_are_listed() {
    let d = Daemon::start();
    let client = mcp_client(d.home()).await;
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(names, ["find_free_port", "get_logs", "list_routes", "register_route", "status", "unregister_route"]);
    let register = tools.iter().find(|t| t.name == "register_route").unwrap();
    let schema = serde_json::to_value(&register.input_schema).unwrap();
    for field in ["host", "path", "strip_path", "protocol", "target", "port", "listen_port", "note", "owner_pid", "persistent"] {
        assert!(schema["properties"].get(field).is_some(), "register_route has no {field}: {schema}");
    }
    // ADR 03, I31: unregister_route names a route by host and path.
    let unregister = tools.iter().find(|t| t.name == "unregister_route").unwrap();
    let schema = serde_json::to_value(&unregister.input_schema).unwrap();
    assert!(schema["properties"].get("path").is_some(), "unregister_route has no path: {schema}");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn each_tool_works_against_a_daemon() {
    let d = Daemon::start();
    let client = mcp_client(d.home()).await;

    let r = call(&client, "find_free_port", json!({})).await;
    assert!(!is_error(&r), "{}", text(&r));
    let port = serde_json::from_str::<Value>(&text(&r)).unwrap()["port"].as_u64().unwrap();

    let r = call(&client, "register_route", json!({"host": "shop", "port": port, "note": "agent test"})).await;
    assert!(!is_error(&r), "{}", text(&r));
    let reg: Value = serde_json::from_str(&text(&r)).unwrap();
    assert_eq!(reg["route"]["target"], format!("http://127.0.0.1:{port}"));
    assert!(reg["route"]["urls"][0].as_str().unwrap().starts_with("https://shop.localhost"));

    let r = call(&client, "register_route", json!({"host": "db.shop", "protocol": "tcp", "port": 55001})).await;
    assert!(!is_error(&r), "{}", text(&r));
    let reg: Value = serde_json::from_str(&text(&r)).unwrap();
    assert!(reg["route"]["listen_port"].as_u64().unwrap() >= 1024);

    let r = call(&client, "list_routes", json!({})).await;
    assert_eq!(serde_json::from_str::<Value>(&text(&r)).unwrap()["routes"].as_array().unwrap().len(), 2);

    let r = call(&client, "get_logs", json!({"limit": 5})).await;
    assert!(!is_error(&r));

    let r = call(&client, "status", json!({})).await;
    assert_eq!(serde_json::from_str::<Value>(&text(&r)).unwrap()["ca"]["state"], "ok");

    let r = call(&client, "unregister_route", json!({"host": "shop"})).await;
    assert_eq!(serde_json::from_str::<Value>(&text(&r)).unwrap()["removed"], true);

    let r = call(&client, "register_route", json!({"host": "feat/login.shop", "port": 5174})).await;
    assert!(is_error(&r));
    assert!(text(&r).contains("feat-login.shop"), "{}", text(&r));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn every_tool_says_not_running_without_a_daemon() {
    let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
    let client = mcp_client(dir.path()).await;
    for (name, args) in [
        ("register_route", json!({"host": "a", "port": 5000})),
        ("unregister_route", json!({"host": "a"})),
        ("list_routes", json!({})),
        ("find_free_port", json!({})),
        ("get_logs", json!({})),
        ("status", json!({})),
    ] {
        let r = call(&client, name, args).await;
        assert!(is_error(&r), "{name}");
        assert!(text(&r).contains("LocalRouter is not running"), "{name}: {}", text(&r));
    }
    // The shim never starts a daemon.
    assert!(!dir.path().join("daemon.sock").exists());
    client.cancel().await.unwrap();
}

// I14
#[tokio::test]
async fn a_daemon_with_another_major_version_is_refused() {
    let d = Daemon::start_env(&[("LOCALROUTER_TEST_API_VERSION", "2.0")]);
    let client = mcp_client(d.home()).await;
    let r = call(&client, "list_routes", json!({})).await;
    assert!(is_error(&r));
    assert!(text(&r).contains("update the older one"), "{}", text(&r));
    client.cancel().await.unwrap();
}

// ADR 03, T10: path routes and unknown arguments.
#[tokio::test]
async fn path_routes_through_mcp_and_unknown_arguments_are_refused() {
    let d = Daemon::start();
    let client = mcp_client(d.home()).await;

    let r = call(&client, "register_route", json!({"host": "shop", "path": "/blog", "port": 3001})).await;
    assert!(!is_error(&r), "{}", text(&r));
    let reg: Value = serde_json::from_str(&text(&r)).unwrap();
    assert_eq!(reg["route"]["path"], "/blog");

    // A misspelled argument must not be dropped: dropping "pth" would register
    // the route without a path and replace the main app's route.
    let params = CallToolRequestParams::new(Cow::Borrowed("register_route"))
        .with_arguments(json!({"host": "shop", "pth": "/api", "port": 8000}).as_object().unwrap().clone());
    match client.call_tool(params).await {
        Ok(r) => assert!(is_error(&r), "an unknown argument was accepted: {}", text(&r)),
        Err(e) => assert!(e.to_string().contains("pth"), "{e}"),
    }
    let r = call(&client, "list_routes", json!({})).await;
    let routes = serde_json::from_str::<Value>(&text(&r)).unwrap()["routes"].as_array().unwrap().clone();
    assert_eq!(routes.len(), 1, "nothing but shop/blog: {routes:?}");

    let r = call(&client, "unregister_route", json!({"host": "shop"})).await;
    assert_eq!(serde_json::from_str::<Value>(&text(&r)).unwrap()["removed"], false, "I28");
    let r = call(&client, "unregister_route", json!({"host": "shop", "path": "/blog"})).await;
    assert_eq!(serde_json::from_str::<Value>(&text(&r)).unwrap()["removed"], true);
    client.cancel().await.unwrap();
}

// ADR 04, T8: the MCP server of localrouter-dev names the dev instance.
#[tokio::test]
async fn a_dev_mcp_server_names_the_dev_instance() {
    let d = Daemon::start();
    let (_bin, dev) = common::renamed_cli("-dev");
    let client = mcp_client_as(&dev, d.home()).await;
    let info = client.peer_info().expect("server info");
    assert_eq!(info.server_info.as_ref().map(|s| s.name.as_str()), Some("localrouter-dev"));
    let instructions = info.instructions.clone().unwrap_or_default();
    assert!(instructions.contains("`localrouter-dev guide`"), "{instructions}");
    assert!(instructions.contains("only when the user asks"), "{instructions}");
    assert_eq!(client.list_all_tools().await.unwrap().len(), 6);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn the_release_mcp_server_keeps_its_name() {
    let d = Daemon::start();
    let client = mcp_client(d.home()).await;
    let info = client.peer_info().expect("server info");
    assert_eq!(info.server_info.as_ref().map(|s| s.name.as_str()), Some("localrouter"));
    assert!(!info.instructions.clone().unwrap_or_default().contains("development build"));
    client.cancel().await.unwrap();
}
