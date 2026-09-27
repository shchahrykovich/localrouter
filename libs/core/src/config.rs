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
}

impl Config {
    /// A new instance's settings: the release gets ports 80 and 443, any
    /// other instance 7080 and 7443 (ADR 04).
    pub fn defaults_for(instance: &Instance) -> Self {
        let (http_port, https_port) = instance.default_ports();
        Self { http_port, https_port, ..Self::default() }
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
