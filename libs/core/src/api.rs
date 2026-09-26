//! Socket API types: newline-delimited JSON over `daemon.sock`.
//!
//! One line in: `{"id": 1, "method": "list_routes", "params": {}}`.
//! One line out: `{"id": 1, "result": {...}}` or `{"id": 1, "error": {...}}`.
//! After `subscribe_logs` the daemon also sends `{"event": "log", "entry": {...}}`
//! lines until the client disconnects.
//!
//! The Swift app keeps its own copy of these shapes. Every file in
//! `api/examples/` must decode on both sides (invariant I11).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;
use crate::logs::LogEntry;
use crate::routes::Route;

/// Major.minor. A client stops when the major number differs (invariant I14).
pub const API_VERSION: &str = "1.0";

pub fn api_major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

pub const METHODS: [&str; 11] = [
    "hello",
    "status",
    "register_route",
    "unregister_route",
    "list_routes",
    "find_free_port",
    "get_logs",
    "subscribe_logs",
    "get_config",
    "set_config",
    "reset_ca",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl Response {
    pub fn ok(id: u64, result: impl Serialize) -> Self {
        Self { id, result: Some(serde_json::to_value(result).unwrap_or(Value::Null)), error: None }
    }
    pub fn err(id: u64, error: ApiError) -> Self {
        Self { id, result: None, error: Some(error) }
    }
}

/// A line the daemon pushes without a request (after `subscribe_logs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    Log { entry: LogEntry },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    UnknownMethod,
    InvalidRoute,
    NotFound,
    PortInUse,
    ProcessNotFound,
    Io,
    CaUnavailable,
    VersionMismatch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

// ---- hello

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloParams {
    /// Free text, for the daemon log: "cli", "mcp", "menubar".
    pub client: String,
    pub api_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloResult {
    pub api_version: String,
    pub daemon_version: String,
}

// ---- status

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortStatus {
    /// Port from the config (`0` = any free port).
    pub configured: u16,
    /// Port actually bound, if any socket is bound.
    pub port: Option<u16>,
    /// Addresses bound, for example `0.0.0.0:443`.
    pub bound: Vec<String>,
    /// One message per address that could not be bound.
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaState {
    /// CA loaded; HTTPS works.
    Ok,
    /// CA files damaged; HTTPS is off until `localrouter ca reset`.
    Broken,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaStatus {
    pub state: CaState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// Path of `ca.pem`.
    pub pem_path: String,
    /// Common name of the CA certificate (used to find it in the keychain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub common_name: Option<String>,
    /// Whether macOS trusts the CA for TLS. `None` if the check could not run.
    pub trusted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusResult {
    pub daemon_version: String,
    pub api_version: String,
    pub pid: u32,
    pub data_dir: String,
    pub http: PortStatus,
    pub https: PortStatus,
    pub ca: CaStatus,
    pub routes: usize,
    /// Set when `routes.json` failed to load and was moved aside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routes_file_problem: Option<String>,
    /// Hosts of TCP routes whose listen port could not be bound.
    #[serde(default)]
    pub listen_failed: Vec<String>,
}

// ---- routes

/// A route as clients see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteView {
    #[serde(flatten)]
    pub route: Route,
    /// `http://` and `https://` URLs, or `host:port` for a TCP route.
    pub urls: Vec<String>,
    /// The target accepted a TCP connection within 200 ms. Only in `list_routes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_up: Option<bool>,
    /// TCP route whose listen port could not be bound.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub listen_failed: bool,
}

/// Parameters of `register_route` are a [`Route`].
pub type RegisterRouteParams = Route;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterRouteResult {
    pub route: RouteView,
    /// True when a route with the same host was replaced.
    pub replaced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostParams {
    pub host: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnregisterRouteResult {
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListRoutesResult {
    pub routes: Vec<RouteView>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FindFreePortParams {
    /// Try this port first, then the ports above it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindFreePortResult {
    pub port: u16,
}

// ---- logs

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GetLogsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetLogsResult {
    pub entries: Vec<LogEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubscribeLogsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribeLogsResult {
    pub subscribed: bool,
}

// ---- config

pub type GetConfigResult = Config;

/// Only the fields that are present change.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SetConfigParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub https_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_lan: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_size: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetConfigResult {
    pub config: Config,
    /// True when a port changed and the daemon must be restarted to use it.
    pub restart_needed: bool,
}

// ---- CA

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResetCaResult {
    pub pem_path: String,
    pub common_name: String,
}

/// Parameters for methods that take none.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Empty {}
