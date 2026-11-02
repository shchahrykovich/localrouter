//! Socket API types: newline-delimited JSON over `daemon.sock`.
//!
//! One line in: `{"id": 1, "method": "list_routes", "params": {}}`.
//! One line out: `{"id": 1, "result": {...}}` or `{"id": 1, "error": {...}}`.
//! After `subscribe_logs` the daemon also sends `{"event": "log", "entry": {...}}`
//! lines until the client disconnects.
//!
//! The Swift app keeps its own copy of these shapes. Every file in
//! `api/examples/` must decode on both sides (invariant I11).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;
use crate::logs::LogEntry;
use crate::routes::Route;
use crate::scripts::engine::{LastError, ScriptKind};
use crate::scripts::rules::ScriptRule;
use crate::traffic::Via;
use crate::traffic::attribute::AppKind;
use crate::traffic::nettop::Proto;

/// Major.minor. A client stops when the major number differs (invariant I14).
pub const API_VERSION: &str = "1.9";

pub fn api_major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

pub const METHODS: [&str; 21] = [
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
    "get_proxy",
    "reset_inspect_ca",
    "set_script_rule",
    "remove_script_rule",
    "list_script_rules",
    "new_setup_code",
    "set_phone_device",
    "get_app_traffic",
    "update_geo_db",
    "clear_traffic_history",
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
    /// A script rule's fields, its script or its `output_dir` (ADR 07).
    InvalidScriptRule,
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
    /// The forward proxy (ADR 06). Absent from daemons before API 1.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyStatus>,
    /// The network of the default route, for LAN access per network (ADR 08).
    /// `None` when it cannot be recognised, and from daemons before API 1.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkStatus>,
    /// Plain sentences for things the user should know (ADR 08): a saved
    /// route that hides a built-in name, LAN access that now works per network.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// The traffic sampler (ADR 11). Absent from daemons before API 1.9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<SamplerState>,
    /// Why the sampler is restarting: nettop failed or its output was not
    /// understood.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_error: Option<String>,
    /// Bytes of history on disk, `<data>/traffic/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_history_bytes: Option<u64>,
    /// The places databases, when they are on this Mac.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geo_db: Option<GeoDbInfo>,
}

/// The network the Mac is on, as LAN access sees it (ADR 08, change 4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkStatus {
    /// `mac:18:35:d1:15:d1:a8`.
    pub id: String,
    /// The name in `lan_networks`, or empty when the network is not there.
    #[serde(default)]
    pub name: String,
    /// The router's IP address.
    pub router: String,
    /// `en0`.
    pub interface: String,
    /// `allow_lan` is on and this network is in `lan_networks`.
    pub lan_allowed: bool,
}

/// The forward proxy port and the inspection CA (ADR 06).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyStatus {
    /// `proxy_enabled` in the config.
    pub enabled: bool,
    /// Port from the config (`0` = any free port).
    pub configured: u16,
    /// Port actually bound, if bound.
    pub port: Option<u16>,
    /// Addresses bound: `127.0.0.1:8877` and `[::1]:8877`, or none.
    pub bound: Vec<String>,
    /// Why the port is not bound, when it should be.
    pub errors: Vec<String>,
    /// `None` until the inspection CA exists (it is made on first need).
    pub inspect_ca: Option<CaStatus>,
    /// The proxy clients' ports (ADR 09). Absent before API 1.6.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clients: Vec<ProxyClientStatus>,
}

/// One proxy client's port (ADR 09).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyClientStatus {
    pub name: String,
    /// Port from the config (`0` = any free port).
    pub configured: u16,
    /// Port actually bound, if bound.
    pub port: Option<u16>,
    /// `127.0.0.1:8878` and `[::1]:8878`, or `0.0.0.0:8878` and
    /// `[::]:8878` for a phone client, or none.
    pub bound: Vec<String>,
    /// Why the port is not bound while the proxy is on.
    pub errors: Vec<String>,
    /// A phone client: its port listens on the LAN (ADR 10).
    #[serde(default, skip_serializing_if = "is_false")]
    pub lan: bool,
    /// Devices that asked to use a phone client and wait for Allow or Deny.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<PendingDevice>,
}

/// A device that sent a proxy request to a phone port and is not allowed
/// yet (ADR 10). Kept in memory only; the newest request wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDevice {
    /// `192.168.0.23`.
    pub address: String,
    /// The host it asked for, to help the user recognise it.
    pub host: String,
    /// Unix time in milliseconds of its last request.
    pub at_ms: u64,
}

fn is_false(v: &bool) -> bool {
    !*v
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
    /// The path of a path route. Without it, `unregister_route` removes only
    /// the route without a path (ADR 03, I28).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
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

// ---- traffic by app (ADR 11)

/// The time a `get_app_traffic` reply covers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrafficRange {
    #[default]
    #[serde(rename = "1h")]
    Hour,
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}

impl TrafficRange {
    pub fn secs(self) -> u64 {
        match self {
            Self::Hour => 3600,
            Self::Day => 86_400,
            Self::Week => 7 * 86_400,
            Self::Month => 30 * 86_400,
        }
    }

    /// 60 buckets of a minute for an hour, 96 for the longer ranges.
    pub fn buckets(self) -> u64 {
        if self == Self::Hour { 60 } else { 96 }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GetAppTrafficParams {
    #[serde(default)]
    pub range: TrafficRange,
    /// Only this app: its key, as in [`AppTraffic::app`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SamplerState {
    /// `traffic` is off: no nettop process runs.
    Off,
    Running,
    /// nettop exited or its output was not understood; it starts again
    /// after 1, 2, 4 … up to 60 seconds.
    Restarting,
}

/// Bytes received and sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InOut {
    #[serde(rename = "in")]
    pub bytes_in: u64,
    #[serde(rename = "out")]
    pub bytes_out: u64,
}

impl InOut {
    pub fn add(&mut self, bytes_in: u64, bytes_out: u64) {
        self.bytes_in += bytes_in;
        self.bytes_out += bytes_out;
    }
}

/// Bytes by how they reached the network.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByVia {
    pub router: InOut,
    pub proxy: InOut,
    pub direct: InOut,
}

impl ByVia {
    pub fn get_mut(&mut self, via: Via) -> &mut InOut {
        match via {
            Via::Router => &mut self.router,
            Via::Proxy => &mut self.proxy,
            Via::Direct => &mut self.direct,
        }
    }
}

/// One bar of the histogram: the bytes from `t` (Unix seconds) for
/// `bucket_secs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficBucket {
    pub t: u64,
    #[serde(rename = "in")]
    pub bytes_in: u64,
    #[serde(rename = "out")]
    pub bytes_out: u64,
}

/// One row of the Apps view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppTraffic {
    /// The key: the app path (`/Applications/Google Chrome.app`), the
    /// executable path, `macOS services`, or `exited:<name>`.
    pub app: String,
    pub name: String,
    pub kind: AppKind,
    /// The executables seen under this app, for the app checks.
    pub paths: Vec<String>,
    /// Its processes seen in the last 30 seconds.
    pub pids: Vec<u32>,
    #[serde(rename = "in")]
    pub bytes_in: u64,
    #[serde(rename = "out")]
    pub bytes_out: u64,
    pub by_via: ByVia,
    /// Different remote addresses.
    pub addresses: usize,
    /// In and out together, in 12 equal parts of the range, oldest first.
    pub spark: Vec<u64>,
}

/// Where a place lookup put an address. City-level guesses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// `GB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    pub lat: f64,
    pub lon: f64,
}

/// The bytes of one app to one remote through one `via`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteTraffic {
    pub app: String,
    /// The executable that made the connection; an app with several
    /// processes (macOS services, a browser's helpers) splits by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    /// `None` for an unconnected UDP socket, or a proxy connection the
    /// proxy has no record of.
    pub address: Option<String>,
    pub port: Option<u16>,
    pub proto: Proto,
    /// The route's name for `router`, the host for `proxy`. nettop has no
    /// host names for `direct`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The network owner from the ASN database: `GitHub, Inc.`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asn_org: Option<String>,
    /// How the server calls itself, by reverse DNS, while `traffic_names`
    /// is on: `165.66.149.34.bc.googleusercontent.com`. Not the domain the
    /// app asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    pub via: Via,
    #[serde(rename = "in")]
    pub bytes_in: u64,
    #[serde(rename = "out")]
    pub bytes_out: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficTotals {
    #[serde(rename = "in")]
    pub bytes_in: u64,
    #[serde(rename = "out")]
    pub bytes_out: u64,
    pub by_via: ByVia,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppTrafficResult {
    pub range: TrafficRange,
    pub sampler: SamplerState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub bucket_secs: u64,
    pub buckets: Vec<TrafficBucket>,
    /// Sorted by bytes in, most first.
    pub apps: Vec<AppTraffic>,
    /// Sorted by bytes in and out, most first; at most 2,000.
    pub remotes: Vec<RemoteTraffic>,
    pub totals: TrafficTotals,
    /// `traffic_places` is on and the databases are on this Mac.
    pub places: bool,
}

/// The DB-IP Lite databases on this Mac.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoDbInfo {
    /// The month of the databases: `2026-11`.
    pub date: String,
    /// Both files together.
    pub bytes: u64,
}

pub type UpdateGeoDbResult = GeoDbInfo;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClearTrafficHistoryResult {
    /// Day files deleted.
    pub removed: usize,
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
    /// Bind or close the forward proxy port, at once (ADR 06).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_port: Option<u16>,
    /// Replaces the whole list. The first need makes the inspection CA.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspect_hosts: Option<Vec<String>>,
    /// The proxy log (ADR 08). Out-of-range limits are refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_log: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_log_file_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_log_file_requests: Option<u64>,
    /// Replaces the whole list of networks where LAN access applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan_networks: Option<Vec<crate::config::LanNetwork>>,
    /// Replaces the whole list of proxy clients (ADR 09). While the proxy is
    /// on, a new or moved port binds before anything is written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_clients: Option<Vec<crate::config::ProxyClient>>,
    /// Run the traffic sampler (ADR 11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<bool>,
    /// 1, 7 or 30.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_history_days: Option<u32>,
    /// Look up places; turning it on does not download: `update_geo_db` does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_places: Option<bool>,
    /// Look up server names by reverse DNS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic_names: Option<bool>,
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

// ---- forward proxy (ADR 06)

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GetProxyParams {
    /// A proxy client's name (ADR 09): `url`, `port`, `env` and
    /// `chrome_args` are then for its port. Without it, the main port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

/// Everything a client needs to use the forward proxy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetProxyResult {
    pub enabled: bool,
    /// The proxy client this reply is for; absent for the main port. A
    /// daemon before API 1.6 never sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// `http://127.0.0.1:<port>`, the value for `HTTPS_PROXY`.
    pub url: String,
    /// The bound port, or the configured one while not bound.
    pub port: u16,
    pub bound: Vec<String>,
    pub errors: Vec<String>,
    /// `inspect_hosts` from the config.
    pub inspect_hosts: Vec<String>,
    /// Every host pattern that is inspected: `inspect_hosts`, and later the
    /// hosts of script rules (ADR 07).
    pub inspect_set: Vec<String>,
    /// `None` before the inspection CA exists.
    pub inspect_ca: Option<CaStatus>,
    /// Environment variables for programs started through the proxy.
    pub env: BTreeMap<String, String>,
    /// Arguments for a separate Chrome instance that uses the proxy. Every
    /// client uses these, none builds its own (I18).
    pub chrome_args: Vec<String>,
    /// Plain sentences for each thing that will not work yet.
    pub notes: Vec<String>,
    /// Every script rule with its state (ADR 07). Absent before API 1.4.
    #[serde(default)]
    pub script_rules: Vec<ScriptRuleView>,
    /// The HAR log of proxy traffic (ADR 08). Absent before API 1.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<ProxyLogStatus>,
    /// Every proxy client's port (ADR 09). Absent before API 1.6.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clients: Vec<ProxyClientStatus>,
    /// For a phone client (ADR 10): what the phone needs. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan: Option<LanProxyInfo>,
}

/// How a phone reaches its proxy client (ADR 10). The MCP tool removes
/// `setup_url`: its token allows a device (I8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanProxyInfo {
    /// This Mac's IPv4 address on the current network; `None` when unknown.
    pub address: Option<String>,
    pub port: u16,
    /// `http://<address>:<port>/setup/<token>`: what the QR code holds.
    /// Opening it allows the device that opens it.
    pub setup_url: Option<String>,
    /// `http://<address>:<port>/pac/<key>.pac`: the URL for Configure Proxy
    /// → Automatic. The phone goes through the proxy, or direct when the
    /// port does not answer or the client is paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pac_url: Option<String>,
    /// The allowed devices.
    pub devices: Vec<String>,
    /// Devices waiting for Allow or Deny.
    pub pending: Vec<PendingDevice>,
    /// Why a phone cannot use it now, in plain sentences.
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NewSetupCodeParams {
    pub client: String,
}

/// A new setup token: the old QR code stops allowing devices. Devices
/// already allowed stay allowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSetupCodeResult {
    pub setup_url: Option<String>,
}

/// Allow a device on a phone client, or deny it: a denied device leaves
/// the allowed list and the waiting list, and its connections close.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SetPhoneDeviceParams {
    pub client: String,
    pub address: String,
    pub allow: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPhoneDeviceResult {
    pub devices: Vec<String>,
}

/// The proxy log: where the HAR files are and how the writer is doing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyLogStatus {
    /// `proxy_log` from the config.
    pub enabled: bool,
    /// The folder of the HAR files.
    pub folder: String,
    /// The viewer, for the user: `http://proxy.localhost` on this instance,
    /// or `http://proxy.localhost/<client>` in a reply for a client.
    pub url: String,
    pub file_mb: u64,
    pub file_requests: u64,
    /// Files kept; older ones are deleted.
    pub keep_files: usize,
    /// The file the next entry goes to, if one is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    /// HAR files in the folder.
    pub files: usize,
    /// Entries written since the daemon started.
    pub written: u64,
    /// Records dropped because the writer was behind.
    pub dropped: u64,
    /// Why the writer stopped, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---- script rules (ADR 07)

/// A script rule and its state, as `list_script_rules` and `get_proxy` show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptRuleView {
    #[serde(flatten)]
    pub rule: ScriptRule,
    /// `intercept` or `log`, from the script. `None` when it never loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ScriptKind>,
    /// Exchanges that matched since the rule was set or the daemon started.
    pub matched: u64,
    /// Intercept: responses returned by `on_request`.
    pub answered: u64,
    pub errors: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<LastError>,
    /// Log: copies dropped because a limit was reached.
    pub dropped: u64,
    /// Log: bytes written to capture files and saved bodies.
    pub bytes_written: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_loaded_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_sha256: Option<String>,
}

/// Parameters of `set_script_rule`: the rule, and `check_only`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetScriptRuleParams {
    #[serde(flatten)]
    pub rule: ScriptRule,
    /// Check everything (fields, `output_dir`, the script) and store nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub check_only: bool,
}

/// What setting a rule did to the inspect set (ADR 07, change 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectChange {
    /// The rule's host was not inspected before.
    pub host_added: bool,
    /// The inspection CA was made for this rule.
    pub ca_created: bool,
    /// Whether macOS trusts the inspection CA. `None` if unknown.
    pub ca_trusted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetScriptRuleResult {
    pub rule: ScriptRuleView,
    /// A rule with this id was replaced.
    pub replaced: bool,
    /// Nothing was stored: the rule and its script passed every check.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub check_only: bool,
    /// For a host outside `.localhost`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspect: Option<InspectChange>,
    /// Plain sentences for each thing that will not work yet.
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IdParams {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoveScriptRuleResult {
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListScriptRulesResult {
    pub rules: Vec<ScriptRuleView>,
}

/// Parameters for methods that take none.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Empty {}
