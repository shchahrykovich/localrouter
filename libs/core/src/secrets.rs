//! The secret header list (ADR 07, change 4; ADR 08, change 1).
//!
//! Script rules and the HAR writer read the same list, so what a script sees
//! as `[redacted]` and what a HAR file holds as `[redacted]` cannot drift
//! apart. A rule's `reveal_secrets` changes only what that rule sees.

use std::sync::{Arc, RwLock};

/// The value written in place of a secret header.
pub const REDACTED: &str = "[redacted]";

/// Secret by default; `secret_headers` in the config adds names.
pub const DEFAULT_SECRET_HEADERS: [&str; 7] =
    ["authorization", "proxy-authorization", "cookie", "set-cookie", "x-api-key", "api-key", "x-auth-token"];

/// The defaults plus `secret_headers` of the config, lower case. One per
/// daemon; the script engine and the HAR writer share it.
pub struct SecretHeaders {
    list: RwLock<Arc<Vec<String>>>,
}

impl Default for SecretHeaders {
    fn default() -> Self {
        Self { list: RwLock::new(Arc::new(merge(&[]))) }
    }
}

impl SecretHeaders {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Replace the user's names; the defaults always stay.
    pub fn set(&self, extra: &[String]) {
        *self.list.write().unwrap() = Arc::new(merge(extra));
    }

    /// The whole list at this moment.
    pub fn get(&self) -> Arc<Vec<String>> {
        self.list.read().unwrap().clone()
    }

    /// `name` in any case.
    pub fn is_secret(&self, name: &str) -> bool {
        self.list.read().unwrap().iter().any(|s| s.eq_ignore_ascii_case(name))
    }
}

fn merge(extra: &[String]) -> Vec<String> {
    let mut all: Vec<String> = DEFAULT_SECRET_HEADERS.iter().map(|s| s.to_string()).collect();
    for h in extra {
        let h = h.trim().to_ascii_lowercase();
        if !h.is_empty() && !all.contains(&h) {
            all.push(h);
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    // ADR 08, T3: the default list plus secret_headers; case does not matter.
    #[test]
    fn defaults_plus_the_users_names_in_any_case() {
        let s = SecretHeaders::new();
        assert!(s.is_secret("Authorization"));
        assert!(s.is_secret("COOKIE"));
        assert!(!s.is_secret("x-session"));
        s.set(&[" X-Session ".into(), "authorization".into(), String::new()]);
        assert!(s.is_secret("x-session"));
        assert_eq!(s.get().len(), DEFAULT_SECRET_HEADERS.len() + 1, "no duplicate, no empty name");
        s.set(&[]);
        assert!(!s.is_secret("x-session"), "set replaces the user's names");
        assert!(s.is_secret("x-api-key"), "the defaults always stay");
    }
}
