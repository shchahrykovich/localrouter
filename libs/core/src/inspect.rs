//! Host patterns and the inspect set of the forward proxy (ADR 06, change 2).
//!
//! A `CONNECT` is a tunnel unless its host matches a pattern of the inspect
//! set. A pattern is an exact name (`api.example.com`) or `*.` plus a name of
//! at least two labels (`*.example.com`), which matches one or more labels in
//! front of that name, never the name itself. `*` alone matches every host
//! outside `.localhost`. Matching ignores case, a final dot and the port.

/// One parsed pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPattern {
    /// The name after `*.`, or the whole name. Lower case. Empty for `*`.
    name: String,
    kind: PatternKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternKind {
    /// `api.example.com`: this name only.
    Exact,
    /// `*.example.com`: one or more labels in front of the name.
    Under,
    /// `*`: every host outside `.localhost`.
    All,
}

impl HostPattern {
    /// Parse and check one pattern. The error says what is wrong.
    pub fn parse(text: &str) -> Result<Self, String> {
        let lower = text.trim().trim_end_matches('.').to_ascii_lowercase();
        if lower == "*" {
            return Ok(Self { name: String::new(), kind: PatternKind::All });
        }
        let (kind, name) = match lower.strip_prefix("*.") {
            Some(rest) => (PatternKind::Under, rest.to_string()),
            None => (PatternKind::Exact, lower.clone()),
        };
        if name.is_empty() {
            return Err(format!("{text:?} is not a host name"));
        }
        if name.len() > 253 {
            return Err(format!("{text:?} is longer than 253 characters"));
        }
        for label in name.split('.') {
            if label.is_empty() || label.len() > 63 {
                return Err(format!("{text:?}: each part between dots must have 1 to 63 characters"));
            }
            if !label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
                return Err(format!(
                    "{text:?}: use a-z, 0-9 and '-' in a name, and '*.' only at the start (\"*.example.com\")"
                ));
            }
        }
        if kind == PatternKind::Under && name.split('.').count() < 2 {
            return Err(format!(
                "{text:?} would inspect a whole top-level domain; name at least two parts after '*.', or use '*' for every host"
            ));
        }
        if is_local(&name) {
            return Err(format!("{text:?}: .{} names never leave this Mac, so there is nothing to inspect", crate::TLD));
        }
        Ok(Self { name, kind })
    }

    /// `*`: the pattern of every host outside `.localhost`.
    pub fn is_all(&self) -> bool {
        self.kind == PatternKind::All
    }

    /// Does `host` (a name, maybe with a port) match this pattern?
    pub fn matches(&self, host: &str) -> bool {
        let host = bare_host(host);
        match self.kind {
            PatternKind::Exact => host == self.name,
            PatternKind::Under => {
                host.len() > self.name.len() + 1
                    && host.ends_with(&self.name)
                    && host.as_bytes()[host.len() - self.name.len() - 1] == b'.'
            }
            PatternKind::All => !host.is_empty() && !is_local(&host),
        }
    }
}

impl std::fmt::Display for HostPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            PatternKind::Exact => f.write_str(&self.name),
            PatternKind::Under => write!(f, "*.{}", self.name),
            PatternKind::All => f.write_str("*"),
        }
    }
}

/// `localhost` or a name under it, in lower case and without a port.
fn is_local(name: &str) -> bool {
    name == crate::TLD || name.ends_with(&format!(".{}", crate::TLD))
}

/// Patterns as the user wrote them, checked, lower case and without
/// duplicates, in their first order. The first bad pattern is the error.
pub fn normalize(patterns: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = vec![];
    for p in patterns {
        let p = HostPattern::parse(p)?.to_string();
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

/// The hosts the proxy inspects. Today these are the `inspect_hosts` of
/// `config.json`; ADR 07 adds the hosts of script rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InspectSet {
    patterns: Vec<HostPattern>,
}

impl InspectSet {
    /// A set from patterns. A pattern that does not parse is left out (the
    /// daemon checks patterns before it saves them).
    pub fn new(patterns: &[String]) -> Self {
        Self { patterns: patterns.iter().filter_map(|p| HostPattern::parse(p).ok()).collect() }
    }

    pub fn matches(&self, host: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(host))
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// The set holds `*`: every host outside `.localhost` is inspected.
    pub fn inspects_all(&self) -> bool {
        self.patterns.iter().any(HostPattern::is_all)
    }

    /// The patterns, as `get_proxy` shows them.
    pub fn patterns(&self) -> Vec<String> {
        self.patterns.iter().map(|p| p.to_string()).collect()
    }
}

/// `API.Example.com.:443` → `api.example.com`; `[::1]:443` → `::1`.
pub fn bare_host(host: &str) -> String {
    let host = host.trim();
    let host = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        match host.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && !h.contains(':') => h,
            _ => host,
        }
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> HostPattern {
        HostPattern::parse(s).unwrap_or_else(|e| panic!("{e}"))
    }

    // T4
    #[test]
    fn an_exact_name_matches_only_itself() {
        let exact = p("api.example.com");
        assert!(exact.matches("api.example.com"));
        assert!(!exact.matches("x.api.example.com"));
        assert!(!exact.matches("example.com"));
    }

    // T4
    #[test]
    fn a_wildcard_matches_one_or_more_labels_in_front() {
        let w = p("*.example.com");
        assert!(w.matches("a.example.com"));
        assert!(w.matches("a.b.example.com"));
        assert!(!w.matches("example.com"));
        assert!(!w.matches("aexample.com"), "a label boundary is needed");
        assert!(!w.matches("example.com.evil.net"));
    }

    // T4
    #[test]
    fn a_star_matches_every_host_outside_localhost() {
        let all = p("*");
        assert!(all.is_all());
        assert_eq!(all.to_string(), "*");
        for host in ["api.example.com", "example.com:443", "com", "1.2.3.4", "[2001:db8::1]:443", "API.Example.com."] {
            assert!(all.matches(host), "{host}");
        }
        for host in ["shop.localhost", "feat.shop.localhost:7443", "localhost", "Shop.LOCALHOST.", ""] {
            assert!(!all.matches(host), "{host:?} must stay with the route table");
        }
        assert_eq!(p(" * ").to_string(), "*");
        assert_eq!(p("*.").to_string(), "*", "a final dot is ignored, as for every pattern");
    }

    #[test]
    fn case_final_dot_and_port_are_ignored() {
        assert!(p("API.Example.com").matches("api.EXAMPLE.com:443"));
        assert!(p("*.example.com.").matches("A.Example.Com.:8443"));
        assert_eq!(p("API.Example.com").to_string(), "api.example.com");
    }

    // T4
    #[test]
    fn bad_patterns_are_refused() {
        for bad in ["", "**", "*.*", "*.com", "a*.b.com", "a.*.com", "under_score.com", "a..b", "shop.localhost", "*.shop.localhost", "localhost", "é.com"] {
            assert!(HostPattern::parse(bad).is_err(), "{bad:?} must be refused");
        }
        let why = HostPattern::parse("*.com").unwrap_err();
        assert!(why.contains("top-level domain"), "{why}");
    }

    #[test]
    fn normalize_lowers_and_removes_duplicates() {
        let list = normalize(&["API.example.com".into(), "api.example.com".into(), "*.Example.org".into()]).unwrap();
        assert_eq!(list, ["api.example.com", "*.example.org"]);
        assert!(normalize(&["ok.com".into(), "*.com".into()]).is_err());
        assert_eq!(normalize(&["*".into(), " * ".into(), "api.example.com".into()]).unwrap(), ["*", "api.example.com"]);
    }

    #[test]
    fn the_set_matches_any_pattern() {
        let set = InspectSet::new(&["api.example.com".into(), "*.example.org".into(), "*.com".into()]);
        assert!(set.matches("api.example.com:443"));
        assert!(set.matches("a.example.org"));
        assert!(!set.matches("other.com"), "a bad pattern is left out");
        assert_eq!(set.patterns(), ["api.example.com", "*.example.org"]);
        assert!(InspectSet::default().is_empty());
        assert!(!set.inspects_all());

        let all = InspectSet::new(&["api.example.com".into(), "*".into()]);
        assert!(all.inspects_all());
        assert!(all.matches("other.net:443"));
        assert!(!all.matches("shop.localhost"));
    }

    #[test]
    fn bare_host_handles_ipv6_and_ports() {
        assert_eq!(bare_host("[::1]:8877"), "::1");
        assert_eq!(bare_host("::1"), "::1");
        assert_eq!(bare_host("Example.com:80"), "example.com");
        assert_eq!(bare_host("example.com"), "example.com");
    }
}
