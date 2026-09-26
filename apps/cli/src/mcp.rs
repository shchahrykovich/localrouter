//! `localrouter mcp`: MCP server over stdio for coding agents.
//!
//! Exactly six tools (invariant I13). Each call opens the daemon socket, so the
//! shim holds no state and never starts a daemon (ADR 01, change 5).

use std::path::PathBuf;

use localrouter_core::api::{self, FindFreePortParams, GetLogsParams, HostParams};
use localrouter_core::routes::{Protocol, Route};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::client::Client;

const INSTRUCTIONS: &str = "LocalRouter gives local servers stable names instead of ports.\n\
HTTP dev servers get https://<name>.localhost (ports 80 and 443 are shared and chosen by name).\n\
Databases and other TCP services get <name>.localhost:<listen_port> (one loopback port per route).\n\
Typical flow: call find_free_port, start the dev server on that port, then register_route with a note \
that says what the route is for. For a git branch or worktree use one label in front of the project: \
feat-login.shop (labels are a-z, 0-9 and '-'). Set owner_pid to the dev server's process id to remove the \
route automatically when it exits. Tell the user the URL from the reply.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RegisterArgs {
    /// Name without .localhost, for example "shop" or "feat-login.shop".
    pub host: String,
    /// "http" (default) for web servers, "tcp" for databases, caches and other TCP services.
    #[serde(default)]
    pub protocol: Option<String>,
    /// Full target: http://127.0.0.1:5173, https://127.0.0.1:8443 or tcp://127.0.0.1:5432. Loopback only.
    #[serde(default)]
    pub target: Option<String>,
    /// Shorthand for target: the local port of the server (127.0.0.1).
    #[serde(default)]
    pub port: Option<u16>,
    /// TCP only: the port clients connect to. 0 or missing picks a free port.
    #[serde(default)]
    pub listen_port: Option<u16>,
    /// HTTP only: redirect http:// to https://.
    #[serde(default)]
    pub https_only: Option<bool>,
    /// Why this route exists, for example "worktree for branch feat/login".
    #[serde(default)]
    pub note: Option<String>,
    /// Remove the route when this process exits. Cannot be combined with persistent.
    #[serde(default)]
    pub owner_pid: Option<u32>,
    /// Keep the route after a daemon restart.
    #[serde(default)]
    pub persistent: Option<bool>,
}

impl RegisterArgs {
    pub fn into_route(self) -> Result<Route, String> {
        let protocol = match self.protocol.as_deref().unwrap_or("http") {
            "http" | "https" => Protocol::Http,
            "tcp" => Protocol::Tcp,
            other => return Err(format!("unknown protocol \"{other}\": use http or tcp")),
        };
        let target = match (self.target, self.port) {
            (Some(t), _) => t,
            (None, Some(port)) => match protocol {
                Protocol::Http => format!("http://127.0.0.1:{port}"),
                Protocol::Tcp => format!("tcp://127.0.0.1:{port}"),
            },
            (None, None) => return Err("give target or port".into()),
        };
        Ok(Route {
            host: self.host,
            protocol,
            target,
            listen_port: match protocol {
                Protocol::Tcp => Some(self.listen_port.unwrap_or(0)),
                Protocol::Http => self.listen_port,
            },
            https_only: self.https_only.unwrap_or(false),
            note: self.note.unwrap_or_default(),
            owner_pid: self.owner_pid,
            persistent: self.persistent.unwrap_or(false),
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HostArgs {
    /// Name without .localhost, for example "feat-login.shop".
    pub host: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct FreePortArgs {
    /// Try this port first, then the ports above it.
    #[serde(default)]
    pub near: Option<u16>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct LogsArgs {
    /// Only entries for this name and its subdomains.
    #[serde(default)]
    pub host: Option<String>,
    /// How many entries, newest last. Default 50.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct NoArgs {}

#[derive(Clone)]
pub struct LocalRouterMcp {
    socket: PathBuf,
    tool_router: ToolRouter<Self>,
}

fn text_result(value: serde_json::Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(serde_json::to_string_pretty(&value).unwrap_or_default())])
}

fn error_result(message: impl ToString) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_string())])
}

#[tool_router]
impl LocalRouterMcp {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket, tool_router: Self::tool_router() }
    }

    async fn call(&self, method: &str, params: impl serde::Serialize) -> CallToolResult {
        match Client::connect(&self.socket, "mcp").await {
            Ok(mut c) => match c.call_value(method, params).await {
                Ok(v) => text_result(v),
                Err(e) => error_result(e),
            },
            Err(e) => error_result(e),
        }
    }

    #[tool(description = "Create or replace a route: a name under .localhost that forwards to a local server. \
Returns the URLs. Replacing an existing name returns replaced=true and the old target.")]
    async fn register_route(&self, Parameters(args): Parameters<RegisterArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(match args.into_route() {
            Ok(route) => self.call("register_route", route).await,
            Err(e) => error_result(e),
        })
    }

    #[tool(description = "Remove a route by name.")]
    async fn unregister_route(&self, Parameters(args): Parameters<HostArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(self.call("unregister_route", HostParams { host: args.host }).await)
    }

    #[tool(description = "List all routes with their URLs, notes, and whether each target is up.")]
    async fn list_routes(&self, Parameters(_): Parameters<NoArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(self.call("list_routes", api::Empty {}).await)
    }

    #[tool(description = "Return a free TCP port on 127.0.0.1 to start a dev server or to use as a TCP listen_port.")]
    async fn find_free_port(&self, Parameters(args): Parameters<FreePortArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(self.call("find_free_port", FindFreePortParams { near: args.near }).await)
    }

    #[tool(description = "Recent requests (HTTP) and connections (TCP): time, host, path without query, status, duration.")]
    async fn get_logs(&self, Parameters(args): Parameters<LogsArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(self.call("get_logs", GetLogsParams { host: args.host, limit: Some(args.limit.unwrap_or(50)) }).await)
    }

    #[tool(description = "Daemon version, ports bound or failed, and whether the local CA exists and is trusted.")]
    async fn status(&self, Parameters(_): Parameters<NoArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(self.call("status", api::Empty {}).await)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LocalRouterMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("localrouter", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

pub async fn run(socket: PathBuf) -> anyhow::Result<()> {
    let service = LocalRouterMcp::new(socket).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
