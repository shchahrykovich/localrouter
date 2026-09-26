//! User settings, stored in `config.json` and written only by the daemon.

use serde::{Deserialize, Serialize};

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
