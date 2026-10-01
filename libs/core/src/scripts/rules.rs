//! Script rules (ADR 07, change 2): which script runs on which traffic.
//!
//! A rule names a host pattern, an optional path prefix and methods, and a
//! `.lua` file. Its kind (intercept or log) comes from the script, not from a
//! field. Lifetimes are the ones of routes: persistent, owned, session.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::inspect::{HostPattern, bare_host};
use crate::routes::{HELP_HOST, MAX_LABEL_LEN, MAX_NOTE_CHARS, host_key, normalize_path, path_matches};

/// At most this many rules (ADR 07, change 2).
pub const MAX_RULES: usize = 64;
/// The intercept order of a rule that gives none.
pub const DEFAULT_ORDER: i64 = 100;
/// The write quota of a log rule that gives none: 1 GiB.
pub const DEFAULT_MAX_CAPTURE_BYTES: u64 = 1 << 30;

/// What an intercept rule does when its script fails.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnError {
    /// The client gets a 502 page that names the rule (the default).
    #[default]
    Fail,
    /// The request or response goes on unchanged.
    Pass,
}

/// One script rule, as clients send it and as `script-rules.json` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptRule {
    /// The rule's name: label rules, for example `claude-capture`.
    pub id: String,
    /// `api.example.com`, `*.example.com`, or a `.localhost` name.
    pub host: String,
    /// A path prefix with the route path match: `/v1` matches `/v1/x`, not `/v1x`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Upper-case methods; empty means every method.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    /// Absolute path of the `.lua` file.
    pub script: String,
    /// Log rules only: the one folder the rule writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    /// Intercept rules run from low to high, then by id.
    #[serde(default = "default_order")]
    pub order: i64,
    /// Intercept rules only. `None` means `fail`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_error: Option<OnError>,
    /// Show secret headers to the script. Never settable through MCP.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reveal_secrets: bool,
    /// Log rules only: bytes the rule may write. `None` means 1 GiB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_capture_bytes: Option<u64>,
    /// A disabled rule matches nothing and keeps its counters.
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The rule is removed when this process exits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
    /// Saved in `script-rules.json`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub persistent: bool,
}

fn default_order() -> i64 {
    DEFAULT_ORDER
}

fn yes() -> bool {
    true
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl ScriptRule {
    pub fn on_error(&self) -> OnError {
        self.on_error.unwrap_or_default()
    }

    pub fn max_capture_bytes(&self) -> u64 {
        self.max_capture_bytes.unwrap_or(DEFAULT_MAX_CAPTURE_BYTES)
    }

    /// The host pattern, parsed. Valid for a rule that passed [`validate`].
    pub fn pattern(&self) -> RuleHost {
        RuleHost::parse(&self.host).unwrap_or(RuleHost::Local { name: String::new(), wildcard: false })
    }

    /// Does this rule match a request? Disabled rules match nothing.
    pub fn matches(&self, host: &str, path: &str, method: &str) -> bool {
        self.enabled
            && self.pattern().matches(host)
            && path_matches(self.path.as_deref(), path)
            && (self.methods.is_empty() || self.methods.iter().any(|m| m.eq_ignore_ascii_case(method)))
    }
}

/// The host of a rule: an internet pattern (ADR 06) or a `.localhost` name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleHost {
    /// Proxy traffic: `api.example.com` or `*.example.com`.
    Internet(HostPattern),
    /// Router traffic: the host key `shop` of `shop.localhost`, or a
    /// wildcard `*.shop.localhost` (one or more labels in front of `shop`).
    Local { name: String, wildcard: bool },
}

impl RuleHost {
    pub fn parse(text: &str) -> Result<Self, String> {
        let lower = text.trim().trim_end_matches('.').to_ascii_lowercase();
        let (wildcard, name) = match lower.strip_prefix("*.") {
            Some(rest) => (true, rest),
            None => (false, lower.as_str()),
        };
        if let Some(key) = host_key(name) {
            if key.split('.').any(|l| l.is_empty() || l.len() > MAX_LABEL_LEN || l.starts_with('-') || l.ends_with('-'))
                || !key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            {
                return Err(format!("{text:?}: use a-z, 0-9 and '-' in each part of a .localhost name"));
            }
            if key == HELP_HOST {
                return Err(format!("{text:?} is the built-in help page; scripts never run on it"));
            }
            return Ok(RuleHost::Local { name: key, wildcard });
        }
        if name == crate::TLD {
            return Err(format!("{text:?}: name a route, for example shop.localhost or *.shop.localhost"));
        }
        let pattern = HostPattern::parse(text)?;
        if !wildcard && !name.contains('.') {
            return Err(format!(
                "{text:?} is one word: for a route use {name}.localhost, for an internet host its full name"
            ));
        }
        Ok(RuleHost::Internet(pattern))
    }

    /// A host outside `.localhost`: its rule adds it to the inspect set.
    pub fn is_internet(&self) -> bool {
        matches!(self, RuleHost::Internet(_))
    }

    /// `host` is a `Host` header or a URI host, with or without a port.
    pub fn matches(&self, host: &str) -> bool {
        match self {
            RuleHost::Internet(p) => host_key(host).is_none() && p.matches(host),
            RuleHost::Local { name, wildcard } => {
                let Some(key) = host_key(&bare_host(host)) else { return false };
                if *wildcard {
                    key.len() > name.len() + 1 && key.ends_with(name.as_str()) && key.as_bytes()[key.len() - name.len() - 1] == b'.'
                } else {
                    key == *name
                }
            }
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_LABEL_LEN
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// An absolute path that is not `/`, written without `.` and `..` parts.
fn absolute(what: &str, text: &str) -> Result<String, String> {
    let path = Path::new(text.trim());
    if !path.is_absolute() {
        return Err(format!("{what} must be an absolute path, for example /Users/me/{}", if what == "script" { "x.lua" } else { "captures" }));
    }
    if text.contains(['\0', '\n', '\r']) {
        return Err(format!("{what} contains a control character"));
    }
    let mut clean = std::path::PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::RootDir | std::path::Component::Normal(_) => clean.push(c),
            std::path::Component::CurDir => {}
            _ => return Err(format!("{what} must be a path without '..' parts")),
        }
    }
    if clean.parent().is_none() {
        return Err(format!("{what} cannot be /"));
    }
    clean.to_str().map(str::to_string).ok_or_else(|| format!("{what} is not UTF-8"))
}

/// Check and normalize a rule's fields. Checks that need the script (kind,
/// `output_dir` for log rules) and the disk come later. `data_dir` is the
/// daemon's data folder: `output_dir` may not be in it. `others` is the
/// number of rules already in the table without this id.
pub fn validate(rule: &mut ScriptRule, data_dir: &Path, others: usize) -> Result<(), String> {
    rule.id = rule.id.trim().to_ascii_lowercase();
    if !valid_id(&rule.id) {
        return Err(format!(
            "id {:?}: use 1 to {MAX_LABEL_LEN} characters of a-z, 0-9 and '-', not starting or ending with '-'",
            rule.id
        ));
    }
    if others >= MAX_RULES {
        return Err(format!("there are already {MAX_RULES} script rules; remove one first"));
    }
    RuleHost::parse(&rule.host)?;
    rule.host = rule.host.trim().trim_end_matches('.').to_ascii_lowercase();
    rule.path = match rule.path.take() {
        Some(p) => normalize_path(&p).map_err(|e| e.to_string())?,
        None => None,
    };
    let mut methods = vec![];
    for m in &rule.methods {
        let m = m.trim().to_ascii_uppercase();
        if m.is_empty() || m.len() > 20 || !m.bytes().all(|b| b.is_ascii_uppercase() || b == b'-' || b == b'_') {
            return Err(format!("method {m:?} is not an HTTP method"));
        }
        if !methods.contains(&m) {
            methods.push(m);
        }
    }
    rule.methods = methods;
    rule.script = absolute("script", &rule.script)?;
    if let Some(dir) = rule.output_dir.take() {
        let dir = absolute("output_dir", &dir)?;
        let data = data_dir.to_string_lossy();
        let data = data.trim_end_matches('/');
        if dir == data || dir.starts_with(&format!("{data}/")) {
            return Err(format!("output_dir {dir} is in the data folder {data}, which holds only the daemon's state"));
        }
        rule.output_dir = Some(dir);
    }
    if rule.note.chars().count() > MAX_NOTE_CHARS {
        return Err(format!("note is longer than {MAX_NOTE_CHARS} characters"));
    }
    if rule.persistent && rule.owner_pid.is_some() {
        return Err("a rule cannot be both persistent and owned by a process (owner_pid)".into());
    }
    if rule.max_capture_bytes == Some(0) {
        return Err("max_capture_bytes must be at least 1".into());
    }
    Ok(())
}

/// Intercept rules in the order they run: by `order`, then by id.
pub fn sort_intercept<T>(rules: &mut [T], key: impl Fn(&T) -> (i64, String)) {
    rules.sort_by_key(|r| key(r));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, host: &str) -> ScriptRule {
        serde_json::from_value(serde_json::json!({"id": id, "host": host, "script": "/tmp/x.lua"})).unwrap()
    }

    fn check(r: &mut ScriptRule) -> Result<(), String> {
        validate(r, Path::new("/Users/me/Library/Application Support/LocalRouter"), 0)
    }

    // T1
    #[test]
    fn defaults_are_the_documented_ones() {
        let r = rule("cap", "api.example.com");
        assert_eq!((r.order, r.enabled, r.on_error(), r.max_capture_bytes()), (100, true, OnError::Fail, 1 << 30));
        let json = serde_json::to_value(&r).unwrap();
        for absent in ["path", "methods", "output_dir", "on_error", "reveal_secrets", "owner_pid", "persistent", "note"] {
            assert!(json.get(absent).is_none(), "{absent}");
        }
    }

    // T1
    #[test]
    fn ids_follow_label_rules() {
        for good in ["cap", "claude-capture", "a1"] {
            assert!(check(&mut rule(good, "api.example.com")).is_ok(), "{good}");
        }
        for bad in ["", "-a", "a-", "a_b", "a.b", &"x".repeat(64)] {
            assert!(check(&mut rule(bad, "api.example.com")).is_err(), "{bad:?}");
        }
        let mut upper = rule("Cap", "api.example.com");
        check(&mut upper).unwrap();
        assert_eq!(upper.id, "cap");
    }

    // T1
    #[test]
    fn hosts_are_patterns_or_localhost_names() {
        for good in ["api.example.com", "*.example.com", "shop.localhost", "*.shop.localhost", "API.Example.com."] {
            assert!(check(&mut rule("a", good)).is_ok(), "{good}");
        }
        for bad in ["*.com", "router.localhost", "localhost", "shop", "a b.com", "", "*.localhost"] {
            assert!(check(&mut rule("a", bad)).is_err(), "{bad:?}");
        }
        assert!(RuleHost::parse("router.localhost").unwrap_err().contains("help page"));
    }

    // T1
    #[test]
    fn path_methods_and_paths_are_normalized() {
        let mut r = rule("a", "shop.localhost");
        r.path = Some("/v1/".into());
        r.methods = vec!["post".into(), "POST".into(), "get".into()];
        r.output_dir = Some("/tmp/./out".into());
        check(&mut r).unwrap();
        assert_eq!(r.path.as_deref(), Some("/v1"));
        assert_eq!(r.methods, ["POST", "GET"]);
        assert_eq!(r.output_dir.as_deref(), Some("/tmp/out"));

        let mut r = rule("a", "shop.localhost");
        r.script = "cap.lua".into();
        assert!(check(&mut r).unwrap_err().contains("absolute"));
        let mut r = rule("a", "shop.localhost");
        r.methods = vec!["GET /".into()];
        assert!(check(&mut r).is_err());
    }

    // T1
    #[test]
    fn output_dir_may_not_be_root_or_in_the_data_folder() {
        for bad in ["/", "/Users/me/Library/Application Support/LocalRouter", "/Users/me/Library/Application Support/LocalRouter/out", "/tmp/../etc", "out"] {
            let mut r = rule("a", "api.example.com");
            r.output_dir = Some(bad.into());
            assert!(check(&mut r).is_err(), "{bad}");
        }
        let mut r = rule("a", "api.example.com");
        r.output_dir = Some("/Users/me/Library/Application Support/LocalRouter-dev".into());
        assert!(check(&mut r).is_ok(), "another instance's folder is not this one's data folder");
    }

    // T1
    #[test]
    fn limits_and_lifetimes() {
        let mut r = rule("a", "api.example.com");
        assert!(validate(&mut r, Path::new("/data"), MAX_RULES).unwrap_err().contains("64"));
        let mut r = rule("a", "api.example.com");
        r.persistent = true;
        r.owner_pid = Some(1);
        assert!(check(&mut r).unwrap_err().contains("persistent"));
        let mut r = rule("a", "api.example.com");
        r.note = "x".repeat(501);
        assert!(check(&mut r).is_err());
    }

    // T2
    #[test]
    fn matching_uses_host_path_and_method() {
        let mut r = rule("a", "*.example.com");
        r.path = Some("/v1".into());
        r.methods = vec!["POST".into()];
        assert!(r.matches("a.example.com:443", "/v1/messages", "POST"));
        assert!(r.matches("a.example.com", "/v1", "post"));
        assert!(!r.matches("a.example.com", "/v1x", "POST"));
        assert!(!r.matches("a.example.com", "/v1", "GET"));
        assert!(!r.matches("example.com", "/v1", "POST"));
        r.enabled = false;
        assert!(!r.matches("a.example.com", "/v1", "POST"), "a disabled rule matches nothing");
    }

    // T2
    #[test]
    fn localhost_rules_match_router_names_only() {
        let shop = rule("a", "shop.localhost");
        assert!(shop.matches("shop.localhost", "/", "GET"));
        assert!(shop.matches("Shop.localhost:7443", "/", "GET"));
        assert!(!shop.matches("feat.shop.localhost", "/", "GET"), "an exact name is exact");
        let any = rule("a", "*.shop.localhost");
        assert!(any.matches("feat.shop.localhost", "/", "GET"));
        assert!(!any.matches("shop.localhost", "/", "GET"));
        let internet = rule("a", "*.example.com");
        assert!(!internet.matches("x.example.com.localhost", "/", "GET"));
    }

    // T2
    #[test]
    fn intercept_rules_sort_by_order_then_id() {
        let mut list = vec![(20, "b"), (10, "z"), (20, "a")];
        sort_intercept(&mut list, |(o, id)| (*o, id.to_string()));
        assert_eq!(list, [(10, "z"), (20, "a"), (20, "b")]);
    }
}
