//! `localrouter mcp`: MCP server over stdio for coding agents. Its name and
//! instructions come from the instance (ADR 04): `localrouter-dev mcp`.
//!
//! Exactly nine tools (invariant I13; ADR 06 added `get_proxy`, I14; ADR 07
//! added `set_script_rule` and `remove_script_rule`, I13). Each call opens the
//! daemon socket, so the shim holds no state and never starts a daemon (ADR
//! 01, change 5).

use std::path::PathBuf;

use localrouter_core::api::{self, FindFreePortParams, GetLogsParams, HostParams, IdParams, SetScriptRuleParams};
use localrouter_core::help;
use localrouter_core::instance::Instance;
use localrouter_core::routes::{FOLDER_SCHEME, Protocol, Route};
use localrouter_core::scripts::rules::{DEFAULT_ORDER, OnError, ScriptRule};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::client::Client;

// Unknown arguments are refused, not dropped: an argument this server does not
// know would otherwise be lost without a word (ADR 03, manifest B2).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterArgs {
    /// Name without .localhost, for example "shop" or "feat-login.shop".
    pub host: String,
    /// HTTP only: path prefix on this name, for example "/blog". Requests for /blog and /blog/... go to this server; other paths go to the route without a path.
    #[serde(default)]
    pub path: Option<String>,
    /// With path: remove the path before the request reaches the server. For servers that answer at /.
    #[serde(default)]
    pub strip_path: Option<bool>,
    /// "http" (default) for web servers, "tcp" for databases, caches and other TCP services.
    #[serde(default)]
    pub protocol: Option<String>,
    /// Full target: http://127.0.0.1:5173, https://127.0.0.1:8443 or tcp://127.0.0.1:5432. Loopback only.
    #[serde(default)]
    pub target: Option<String>,
    /// Shorthand for target: the local port of the server (127.0.0.1).
    #[serde(default)]
    pub port: Option<u16>,
    /// Serve the files of this folder instead of a server, for example "/Users/me/shop/dist". Absolute path. A folder answers with its index.html, else a file list. Instead of target and port.
    #[serde(default)]
    pub folder: Option<String>,
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
        let target = match (self.target, self.port, self.folder) {
            (Some(t), _, None) => t,
            (None, Some(port), None) => match protocol {
                Protocol::Http => format!("http://127.0.0.1:{port}"),
                Protocol::Tcp => format!("tcp://127.0.0.1:{port}"),
            },
            // The daemon checks the path: absolute, and a folder that exists.
            (None, None, Some(folder)) => format!("{FOLDER_SCHEME}{folder}"),
            (None, None, None) => return Err("give target, port or folder".into()),
            _ => return Err("give folder without target and port".into()),
        };
        Ok(Route {
            host: self.host,
            path: self.path,
            protocol,
            target,
            listen_port: match protocol {
                Protocol::Tcp => Some(self.listen_port.unwrap_or(0)),
                Protocol::Http => self.listen_port,
            },
            https_only: self.https_only.unwrap_or(false),
            strip_path: self.strip_path.unwrap_or(false),
            note: self.note.unwrap_or_default(),
            owner_pid: self.owner_pid,
            persistent: self.persistent.unwrap_or(false),
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostArgs {
    /// Name without .localhost, for example "feat-login.shop".
    pub host: String,
    /// The path of a path route, for example "/blog". Without it, only the route without a path is removed.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FreePortArgs {
    /// Try this port first, then the ports above it.
    #[serde(default)]
    pub near: Option<u16>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogsArgs {
    /// Only entries for this name and its subdomains.
    #[serde(default)]
    pub host: Option<String>,
    /// How many entries, newest last. Default 50.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

// There is no reveal_secrets argument: only the user, at a terminal, can let a
// rule see API keys and cookies, and unknown arguments are refused (ADR 07, I9).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptRuleArgs {
    /// The rule's name, a-z, 0-9 and '-', for example "claude-capture". An existing id is replaced.
    pub id: String,
    /// "api.example.com", "*.example.com", "*" (every host outside .localhost), or a route such as "shop.localhost".
    pub host: String,
    /// Absolute path of the .lua file.
    pub script: String,
    /// Only this path prefix: "/v1" matches /v1 and /v1/..., never /v1x.
    #[serde(default)]
    pub path: Option<String>,
    /// Only these methods, for example ["POST"].
    #[serde(default)]
    pub methods: Option<Vec<String>>,
    /// Log rules: absolute path of an existing folder, the only one the script writes. Keep it out of git.
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Intercept rules run from low to high (default 100).
    #[serde(default)]
    pub order: Option<i64>,
    /// Intercept rules: "fail" (default, the client gets a 502 page) or "pass" (traffic goes on unchanged).
    #[serde(default)]
    pub on_error: Option<String>,
    /// Log rules: bytes the rule may write (default 1 GiB).
    #[serde(default)]
    pub max_capture_bytes: Option<u64>,
    /// false sets the rule turned off.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Why the rule exists.
    #[serde(default)]
    pub note: Option<String>,
    /// Remove the rule when this process exits. Cannot be combined with persistent.
    #[serde(default)]
    pub owner_pid: Option<u32>,
    /// Keep the rule after a daemon restart. Only when the user asks.
    #[serde(default)]
    pub persistent: Option<bool>,
    /// Check the fields, the folder and the script, and store nothing.
    #[serde(default)]
    pub check_only: Option<bool>,
}

impl ScriptRuleArgs {
    pub fn into_params(self) -> Result<SetScriptRuleParams, String> {
        let on_error = match self.on_error.as_deref() {
            None => None,
            Some("fail") => Some(OnError::Fail),
            Some("pass") => Some(OnError::Pass),
            Some(other) => return Err(format!("on_error \"{other}\": use fail or pass")),
        };
        let rule = ScriptRule {
            id: self.id,
            host: self.host,
            path: self.path,
            methods: self.methods.unwrap_or_default(),
            script: self.script,
            output_dir: self.output_dir,
            order: self.order.unwrap_or(DEFAULT_ORDER),
            on_error,
            reveal_secrets: false,
            max_capture_bytes: self.max_capture_bytes,
            enabled: self.enabled.unwrap_or(true),
            note: self.note.unwrap_or_default(),
            owner_pid: self.owner_pid,
            persistent: self.persistent.unwrap_or(false),
        };
        Ok(SetScriptRuleParams { rule, check_only: self.check_only.unwrap_or(false) })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdArgs {
    /// The id of the script rule.
    pub id: String,
}

#[derive(Clone)]
pub struct LocalRouterMcp {
    socket: PathBuf,
    instance: Instance,
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
    pub fn new(socket: PathBuf, instance: Instance) -> Self {
        Self { socket, instance, tool_router: Self::tool_router() }
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

    #[tool(description = "Create or replace a route: a name under .localhost that forwards to a local server, \
or serves the files of a folder (folder). Give path to send only that part of the name to this route. Returns the URLs. Replacing an existing name and \
path returns replaced=true and the old target.")]
    async fn register_route(&self, Parameters(args): Parameters<RegisterArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(match args.into_route() {
            Ok(route) => self.call("register_route", route).await,
            Err(e) => error_result(e),
        })
    }

    #[tool(description = "Remove a route by name, and by path for a path route. Without path it removes only \
the route without a path.")]
    async fn unregister_route(&self, Parameters(args): Parameters<HostArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(self.call("unregister_route", HostParams { host: args.host, path: args.path }).await)
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

    // Read only: turning the proxy on and choosing inspected hosts are CLI
    // commands, which the user sees in the transcript (ADR 06, change 3).
    #[tool(description = "How to send traffic through the LocalRouter proxy: proxy URL, environment variables for \
Claude Code and Node.js, Chrome flags, which hosts are inspected, and whether the inspection CA is trusted. A program \
reads the proxy settings when it starts: pass env to a program you start, never write them into project files. Also \
lists every script rule with its counters and last error (script_rules), and the HAR log of proxy traffic: log.folder \
holds the files (read them with jq, never whole), log.url is the viewer for the user.")]
    async fn get_proxy(&self, Parameters(_): Parameters<NoArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(self.call("get_proxy", api::Empty {}).await)
    }

    #[tool(description = "Run a Lua script on the HTTP traffic of a host: an intercept script changes or answers \
requests while the client waits; a log script gets a copy of each finished exchange and writes files in output_dir. \
The kind comes from the script. Read the script reference first: the /scripts page of the help site, or the `rules api` \
command. Test with check_only: true, then set the rule before the job starts, with owner_pid of a process you \
started, or remove it when done. A host outside .localhost becomes inspected by the proxy. Secret headers reach \
scripts as [redacted]; only the user can change that.")]
    async fn set_script_rule(&self, Parameters(args): Parameters<ScriptRuleArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(match args.into_params() {
            Ok(params) => self.call("set_script_rule", params).await,
            Err(e) => error_result(e),
        })
    }

    #[tool(description = "Remove a script rule by id. Returns removed=false when there is none. Capture files stay.")]
    async fn remove_script_rule(&self, Parameters(args): Parameters<IdArgs>) -> Result<CallToolResult, ErrorData> {
        Ok(self.call("remove_script_rule", IdParams { id: args.id }).await)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LocalRouterMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(self.instance.cli(), env!("CARGO_PKG_VERSION")))
            .with_instructions(help::mcp_instructions(&self.instance))
    }
}

pub async fn run(socket: PathBuf, instance: Instance) -> anyhow::Result<()> {
    let service = LocalRouterMcp::new(socket, instance).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
