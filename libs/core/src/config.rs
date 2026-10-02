//! User settings, stored in `config.json` and written only by the daemon.

use serde::{Deserialize, Serialize};

use crate::instance::Instance;

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    /// Port for plain HTTP. `0` means "any free port" (tests).
    pub http_port: u16,
    /// Port for HTTPS. `0` means "any free port" (tests).
    pub https_port: u16,
    /// A host key with no route uses the route of its parent.
    pub fallback: bool,
    /// Accept connections to the shared HTTP ports from other machines.
    pub allow_lan: bool,
    /// Entries kept in the request log.
    pub log_size: usize,
    /// Bind the forward proxy port on loopback (ADR 06).
    pub proxy_enabled: bool,
    /// The forward proxy port. `0` means "any free port" (tests).
    pub proxy_port: u16,
    /// Host patterns whose `CONNECT`s are inspected instead of tunnelled:
    /// `api.example.com`, `*.example.com`, or `*` for every host outside
    /// `.localhost`.
    pub inspect_hosts: Vec<String>,
    /// Header names scripts see as `[redacted]`, besides the defaults
    /// (`authorization`, `cookie`, ...; ADR 07, change 4). Not written while
    /// empty, so the file stays as an older daemon wrote it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secret_headers: Vec<String>,
    /// Write every request the forward proxy carries to rolling HAR files
    /// in `<logs>/proxy/` (ADR 08).
    pub proxy_log: bool,
    /// A HAR file is full at this many MB (1 to 200).
    pub proxy_log_file_mb: u64,
    /// A HAR file is full at this many requests (100 to 1,000,000).
    pub proxy_log_file_requests: u64,
    /// The networks where `allow_lan` applies, each known by its router's
    /// MAC address (ADR 08, change 4). On any other network only this Mac
    /// reaches ports 80 and 443.
    pub lan_networks: Vec<LanNetwork>,
}

/// One network where LAN access is allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanNetwork {
    /// `mac:18:35:d1:15:d1:a8`: the router's MAC address.
    pub id: String,
    /// The user's name for it: "Home", "Office".
    #[serde(default)]
    pub name: String,
    /// The router's IP address when the network was allowed, for people.
    #[serde(default)]
    pub router: String,
}

impl Config {
    /// Range checks for the proxy log limits; the message names the range.
    pub fn check_proxy_log_limits(file_mb: u64, file_requests: u64) -> Result<(), String> {
        use crate::har::{FILE_MB_MAX, FILE_MB_MIN, FILE_REQUESTS_MAX, FILE_REQUESTS_MIN};
        if !(FILE_MB_MIN..=FILE_MB_MAX).contains(&file_mb) {
            return Err(format!("proxy_log_file_mb must be {FILE_MB_MIN} to {FILE_MB_MAX}, not {file_mb}"));
        }
        if !(FILE_REQUESTS_MIN..=FILE_REQUESTS_MAX).contains(&file_requests) {
            return Err(format!(
                "proxy_log_file_requests must be {FILE_REQUESTS_MIN} to {FILE_REQUESTS_MAX}, not {file_requests}"
            ));
        }
        Ok(())
    }
}

impl Config {
    /// A new instance's settings: the release gets ports 80 and 443, any
    /// other instance 7080 and 7443 (ADR 04).
    pub fn defaults_for(instance: &Instance) -> Self {
        let (http_port, https_port) = instance.default_ports();
        Self { http_port, https_port, proxy_port: instance.default_proxy_port(), ..Self::default() }
    }

    /// Parse `config.json`. A field the file does not have gets the
    /// instance's default, so a suffixed daemon never falls back to 80 or
    /// 443 (ADR 04, I9).
    pub fn parse(text: &str, instance: &Instance) -> Result<Self, String> {
        let file: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let serde_json::Value::Object(fields) = file else { return Err("config.json is not a JSON object".into()) };
        let mut merged = serde_json::to_value(Self::defaults_for(instance)).map_err(|e| e.to_string())?;
        for (key, value) in fields {
            merged[key] = value;
        }
        serde_json::from_value(merged).map_err(|e| e.to_string())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            http_port: 80,
            https_port: 443,
            fallback: true,
            allow_lan: false,
            log_size: 1000,
            proxy_enabled: false,
            proxy_port: Instance::release().default_proxy_port(),
            inspect_hosts: vec![],
            secret_headers: vec![],
            proxy_log: true,
            proxy_log_file_mb: crate::har::DEFAULT_FILE_MB,
            proxy_log_file_requests: crate::har::DEFAULT_FILE_REQUESTS,
            lan_networks: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev() -> Instance {
        Instance::new("-dev").unwrap()
    }

    #[test]
    fn defaults_depend_on_the_instance() {
        let r = Config::defaults_for(&Instance::release());
        assert_eq!((r.http_port, r.https_port), (80, 443));
        assert_eq!(r, Config::default());
        let d = Config::defaults_for(&dev());
        assert_eq!((d.http_port, d.https_port), (7080, 7443));
        assert_eq!(d.log_size, r.log_size);
    }

    // ADR 06, T8: the proxy is off by default, on the instance's own port.
    #[test]
    fn proxy_fields_default_per_instance() {
        let r = Config::defaults_for(&Instance::release());
        assert_eq!((r.proxy_enabled, r.proxy_port, r.inspect_hosts.len()), (false, 8877, 0));
        let d = Config::defaults_for(&dev());
        assert_eq!((d.proxy_enabled, d.proxy_port), (false, 7877));
        // A file written before ADR 06 has none of the fields.
        let old = Config::parse(r#"{"version":1,"http_port":0,"https_port":0,"fallback":true,"allow_lan":false,"log_size":5}"#, &dev())
            .unwrap();
        assert_eq!((old.proxy_enabled, old.proxy_port, old.inspect_hosts.len()), (false, 7877, 0));
    }

    #[test]
    fn proxy_fields_round_trip() {
        let mut c = Config::defaults_for(&dev());
        c.proxy_enabled = true;
        c.proxy_port = 9000;
        c.inspect_hosts = vec!["api.example.com".into(), "*.example.org".into()];
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(Config::parse(&text, &dev()).unwrap(), c);
    }

    // ADR 08, T8: the log is on by default, 20 MB or 5000 requests per file;
    // a file written before ADR 08 gets the same.
    #[test]
    fn proxy_log_fields_default() {
        let old = Config::parse(r#"{"version":1,"allow_lan":true}"#, &dev()).unwrap();
        assert_eq!((old.proxy_log, old.proxy_log_file_mb, old.proxy_log_file_requests), (true, 20, 5000));
        assert!(old.lan_networks.is_empty());
        let mut c = Config::defaults_for(&dev());
        c.lan_networks = vec![LanNetwork { id: "mac:18:35:d1:15:d1:a8".into(), name: "Home".into(), router: "192.168.0.1".into() }];
        c.proxy_log = false;
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(Config::parse(&text, &dev()).unwrap(), c);
    }

    #[test]
    fn proxy_log_limits_have_ranges() {
        assert!(Config::check_proxy_log_limits(1, 100).is_ok());
        assert!(Config::check_proxy_log_limits(200, 1_000_000).is_ok());
        assert!(Config::check_proxy_log_limits(0, 5000).unwrap_err().contains("1 to 200"));
        assert!(Config::check_proxy_log_limits(201, 5000).is_err());
        assert!(Config::check_proxy_log_limits(20, 99).unwrap_err().contains("100 to 1000000"));
    }

    #[test]
    fn a_missing_field_gets_the_instance_default() {
        let c = Config::parse(r#"{"https_port": 7444}"#, &dev()).unwrap();
        assert_eq!((c.http_port, c.https_port), (7080, 7444));
        let c = Config::parse(r#"{"https_port": 8443}"#, &Instance::release()).unwrap();
        assert_eq!((c.http_port, c.https_port), (80, 8443));
    }

    #[test]
    fn a_whole_file_is_read_as_written() {
        let c = Config::parse(
            r#"{"version":1,"http_port":0,"https_port":0,"fallback":false,"allow_lan":true,"log_size":5}"#,
            &dev(),
        )
        .unwrap();
        assert_eq!((c.http_port, c.https_port, c.fallback, c.allow_lan, c.log_size), (0, 0, false, true, 5));
    }

    #[test]
    fn not_an_object_or_a_wrong_type_is_an_error() {
        assert!(Config::parse("[]", &dev()).is_err());
        assert!(Config::parse(r#"{"http_port": "80"}"#, &dev()).is_err());
        assert!(Config::parse("{", &dev()).is_err());
    }
}
