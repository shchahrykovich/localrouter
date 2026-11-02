//! Traffic by app (ADR 11): what every program on this Mac sends and
//! receives, read from `nettop`, which comes with macOS.
//!
//! - [`nettop`]: the CSV text of one sample. No I/O.
//! - [`attribute`]: the `via` of each connection, the daemon's own pid
//!   dropped, the proxy join, the app of each process. No I/O except the
//!   path callback.
//! - [`history`]: memory for the last hour, one NDJSON line per minute per
//!   active pair in `<data>/traffic/`, reading back, retention.
//! - [`geo`]: places from DB-IP Lite databases on this Mac; the download
//!   rule.
//! - [`names`]: server names by reverse DNS, in the background, cached.
//! - [`peers`]: what the router and the forward proxy know about a client
//!   connection, by its port.

pub mod attribute;
pub mod geo;
pub mod history;
pub mod names;
pub mod nettop;
pub mod peers;

use serde::{Deserialize, Serialize};

/// How an app's connection reached the network (ADR 11, 1.3). The user chose
/// the words; `router` here means "through a `.localhost` route".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Router,
    Proxy,
    Direct,
}

/// One sample every this many seconds.
pub const SAMPLE_SECS: u64 = 5;
