//! An instance is one complete copy of LocalRouter: app, daemon, CLI, data
//! folder, CA, links and ports (ADR 04). Its suffix is fixed when the bundle is
//! built: empty for the release, `-dev` for a local build. Every name is the
//! release name plus the suffix, so the empty suffix gives the names
//! LocalRouter always had.
//!
//! The Swift app has a copy of these rules (`Instance.swift`). Both are
//! checked against `api/instance-names.json`.

use std::fmt;

/// Longest suffix, dash included. Keeps the socket path under the macOS limit.
pub const MAX_SUFFIX_LEN: usize = 16;

const DAEMON_PROGRAM: &str = "localrouterd";
const CLI_PROGRAM: &str = "localrouter";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Instance {
    suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstanceError {
    #[error("invalid instance suffix {0:?}: use \"\" or \"-\" followed by 1 to 15 of a-z and 0-9, like \"-dev\"")]
    BadSuffix(String),
    #[error("cannot tell the instance from the program name {0:?}: it must start with localrouterd or localrouter")]
    BadProgramName(String),
    #[error("cannot find this program's own path: {0}")]
    NoExePath(String),
}

impl Instance {
    /// The release: no suffix.
    pub fn release() -> Self {
        Self { suffix: String::new() }
    }

    /// An instance with this suffix, or an error when the suffix breaks the rule.
    pub fn new(suffix: &str) -> Result<Self, InstanceError> {
        if valid_suffix(suffix) {
            Ok(Self { suffix: suffix.to_string() })
        } else {
            Err(InstanceError::BadSuffix(suffix.to_string()))
        }
    }

    /// The instance a program belongs to, from its file name:
    /// `localrouterd-dev` and `localrouter-dev` are `-dev`.
    pub fn from_program_name(name: &str) -> Result<Self, InstanceError> {
        let suffix = name
            .strip_prefix(DAEMON_PROGRAM)
            .or_else(|| name.strip_prefix(CLI_PROGRAM))
            .ok_or_else(|| InstanceError::BadProgramName(name.to_string()))?;
        Self::new(suffix)
    }

    /// The instance of the running program. Links are resolved first, so
    /// `~/.local/bin/localrouter-dev` counts as the program in the bundle.
    pub fn of_this_program() -> Result<Self, InstanceError> {
        let exe = std::env::current_exe().map_err(|e| InstanceError::NoExePath(e.to_string()))?;
        let exe = exe.canonicalize().unwrap_or(exe);
        let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        Self::from_program_name(name)
    }

    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    pub fn is_release(&self) -> bool {
        self.suffix.is_empty()
    }

    /// `LocalRouter-dev`: the app's name, and the name of its folders.
    pub fn app_name(&self) -> String {
        format!("LocalRouter{}", self.suffix)
    }

    pub fn bundle_id(&self) -> String {
        format!("dev.localrouter.app{}", self.suffix)
    }

    pub fn daemon_label(&self) -> String {
        format!("{}.daemon", self.bundle_id())
    }

    pub fn daemon_program(&self) -> String {
        format!("{DAEMON_PROGRAM}{}", self.suffix)
    }

    /// The CLI's name, also the MCP server's name.
    pub fn cli(&self) -> String {
        format!("{CLI_PROGRAM}{}", self.suffix)
    }

    /// The data folder, relative to the home folder.
    pub fn data_folder(&self) -> String {
        format!("Library/Application Support/{}", self.app_name())
    }

    /// The logs folder, relative to the home folder.
    pub fn logs_folder(&self) -> String {
        format!("Library/Logs/{}", self.app_name())
    }

    /// The Claude Code note, linked into `~/.claude`.
    pub fn note(&self) -> String {
        format!("{}.md", self.app_name())
    }

    /// The common name of a new CA starts with this.
    pub fn ca_name_prefix(&self) -> String {
        format!("{} CA", self.app_name())
    }

    /// The common name of a new inspection CA starts with this (ADR 06), so
    /// the keychain shows which CA is which.
    pub fn inspect_ca_name_prefix(&self) -> String {
        format!("{} Inspection", self.app_name())
    }

    /// The caches folder, relative to the home folder. Chrome writes the
    /// proxy profile there, not the daemon (ADR 06).
    pub fn caches_folder(&self) -> String {
        format!("Library/Caches/{}", self.app_name())
    }

    /// Ports a new `config.json` gets: 80 and 443 for the release, 7080 and
    /// 7443 otherwise. 8080 is avoided: many dev servers use it.
    pub fn default_ports(&self) -> (u16, u16) {
        if self.is_release() { (80, 443) } else { (7080, 7443) }
    }

    /// The forward proxy port a new `config.json` gets (ADR 06): 8877 for the
    /// release, 7877 otherwise, so two instances can both run a proxy.
    pub fn default_proxy_port(&self) -> u16 {
        if self.is_release() { 8877 } else { 7877 }
    }

    /// The help page at the default HTTP port; right until the user changes
    /// the ports in `config.json`.
    pub fn default_help_url(&self) -> String {
        help_url(Some(self.default_ports().0), None)
    }
}

impl fmt::Display for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.app_name())
    }
}

/// The help page for the ports the daemon has bound. Plain HTTP first: curl
/// reads it without trusting the local CA.
pub fn help_url(http_port: Option<u16>, https_port: Option<u16>) -> String {
    let host = format!("{}.{}", crate::routes::HELP_HOST, crate::TLD);
    match (http_port, https_port) {
        (Some(80), _) => format!("http://{host}"),
        (Some(p), _) => format!("http://{host}:{p}"),
        (None, Some(443)) => format!("https://{host}"),
        (None, Some(p)) => format!("https://{host}:{p}"),
        (None, None) => format!("http://{host}"),
    }
}

/// `:7443` for a port a URL must name, empty for the scheme's own port.
pub fn port_part(port: u16, default: u16) -> String {
    if port == default { String::new() } else { format!(":{port}") }
}

fn valid_suffix(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let Some(rest) = s.strip_prefix('-') else { return false };
    !rest.is_empty() && s.len() <= MAX_SUFFIX_LEN && rest.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashSet;

    const TABLE: &str = include_str!("../../../api/instance-names.json");

    fn table() -> Value {
        serde_json::from_str(TABLE).unwrap()
    }

    fn names(i: &Instance) -> Vec<(&'static str, Value)> {
        let (http, https) = i.default_ports();
        vec![
            ("app_name", i.app_name().into()),
            ("bundle_id", i.bundle_id().into()),
            ("daemon_label", i.daemon_label().into()),
            ("daemon_program", i.daemon_program().into()),
            ("cli", i.cli().into()),
            ("data_folder", i.data_folder().into()),
            ("logs_folder", i.logs_folder().into()),
            ("note", i.note().into()),
            ("mcp_name", i.cli().into()),
            ("ca_name_prefix", i.ca_name_prefix().into()),
            ("inspect_ca_name_prefix", i.inspect_ca_name_prefix().into()),
            ("caches_folder", i.caches_folder().into()),
            ("http_port", http.into()),
            ("https_port", https.into()),
            ("proxy_port", i.default_proxy_port().into()),
            ("default_help_url", i.default_help_url().into()),
        ]
    }

    #[test]
    fn every_row_of_the_table_gives_its_names() {
        let t = table();
        let rows = t["rows"].as_array().unwrap();
        assert!(rows.len() >= 3);
        for row in rows {
            let suffix = row["suffix"].as_str().unwrap();
            let i = Instance::new(suffix).unwrap();
            for (key, value) in names(&i) {
                assert_eq!(row[key], value, "suffix {suffix:?}, {key}");
            }
            // Every field of the row is checked: a new column needs a new name here.
            assert_eq!(row.as_object().unwrap().len(), names(&i).len() + 1, "suffix {suffix:?}: unchecked column");
        }
    }

    #[test]
    fn the_release_keeps_the_names_it_always_had() {
        let r = Instance::release();
        assert_eq!(r, Instance::new("").unwrap());
        assert_eq!(r.bundle_id(), "dev.localrouter.app");
        assert_eq!(r.daemon_label(), "dev.localrouter.app.daemon");
        assert_eq!(r.cli(), "localrouter");
        assert_eq!(r.daemon_program(), "localrouterd");
        assert_eq!(r.data_folder(), "Library/Application Support/LocalRouter");
        assert_eq!(r.logs_folder(), "Library/Logs/LocalRouter");
        assert_eq!(r.note(), "LocalRouter.md");
        assert_eq!(r.default_ports(), (80, 443));
        assert_eq!(r.default_help_url(), "http://router.localhost");
    }

    #[test]
    fn two_instances_never_share_a_name() {
        let t = table();
        let instances: Vec<Instance> =
            t["rows"].as_array().unwrap().iter().map(|r| Instance::new(r["suffix"].as_str().unwrap()).unwrap()).collect();
        for key in
            ["app_name", "bundle_id", "daemon_label", "daemon_program", "cli", "data_folder", "logs_folder", "caches_folder", "note"]
        {
            let values: HashSet<String> = instances
                .iter()
                .map(|i| names(i).into_iter().find(|(k, _)| *k == key).unwrap().1.as_str().unwrap().to_string())
                .collect();
            assert_eq!(values.len(), instances.len(), "{key} is shared by two instances");
        }
    }

    #[test]
    fn suffix_rule() {
        for bad in table()["invalid"].as_array().unwrap() {
            let bad = bad.as_str().unwrap();
            assert_eq!(Instance::new(bad), Err(InstanceError::BadSuffix(bad.into())), "{bad:?} must be refused");
        }
        assert!(Instance::new("-dev").is_ok());
        assert!(Instance::new("-abcdefghijklmno").is_ok(), "16 characters is the maximum");
    }

    #[test]
    fn instance_from_program_name() {
        assert_eq!(Instance::from_program_name("localrouterd-dev").unwrap().suffix(), "-dev");
        assert_eq!(Instance::from_program_name("localrouter-dev").unwrap().suffix(), "-dev");
        assert_eq!(Instance::from_program_name("localrouterd").unwrap(), Instance::release());
        assert_eq!(Instance::from_program_name("localrouter").unwrap(), Instance::release());
        assert!(matches!(Instance::from_program_name("localrouter-Dev"), Err(InstanceError::BadSuffix(_))));
        assert!(matches!(Instance::from_program_name("lr"), Err(InstanceError::BadProgramName(_))));
    }

    #[test]
    fn help_url_names_ports_other_than_80_and_443() {
        assert_eq!(help_url(Some(80), Some(443)), "http://router.localhost");
        assert_eq!(help_url(Some(7080), Some(7443)), "http://router.localhost:7080");
        assert_eq!(help_url(None, Some(443)), "https://router.localhost");
        assert_eq!(help_url(None, Some(7443)), "https://router.localhost:7443");
        assert_eq!(port_part(443, 443), "");
        assert_eq!(port_part(7443, 443), ":7443");
    }
}
