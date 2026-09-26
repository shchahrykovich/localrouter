//! The page at `router.localhost`: instructions for a coding agent, in
//! Markdown, with the daemon status and the current routes filled in.

use crate::api::{CaState, PortStatus, StatusResult};
use crate::routes::{Protocol, Route};

const TEMPLATE: &str = include_str!("help.md");

/// The help page text. `https_port` is the bound HTTPS port, if any.
pub fn render(routes: &[Route], https_port: Option<u16>, status: Option<&StatusResult>) -> String {
    TEMPLATE
        .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
        .replace("{{STATUS}}", &status_table(status))
        .replace("{{ROUTES}}", &route_list(routes, https_port))
}

fn status_table(status: Option<&StatusResult>) -> String {
    let Some(s) = status else {
        return "Status is not available.".into();
    };
    let port = |p: &PortStatus| {
        let errors = p.errors.join("; ");
        match (p.port, errors.is_empty()) {
            (Some(port), true) => format!("on, port {port}"),
            (Some(port), false) => format!("on, port {port} ({errors})"),
            (None, true) => "off".into(),
            (None, false) => format!("off: {errors}"),
        }
    };
    let ca = match s.ca.state {
        CaState::Ok => format!("ready, {}", s.ca.common_name.as_deref().unwrap_or("no name")),
        CaState::Broken => format!("broken: {}", s.ca.problem.as_deref().unwrap_or("unknown problem")),
    };
    let trusted = match s.ca.trusted {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    };
    format!(
        "| Part | State |\n|---|---|\n| HTTP | {} |\n| HTTPS | {} |\n| Local CA | {} |\n| CA trusted by macOS | {trusted} |",
        port(&s.http).replace('|', "/"),
        port(&s.https).replace('|', "/"),
        ca.replace('|', "/"),
    )
}

fn route_list(routes: &[Route], https_port: Option<u16>) -> String {
    if routes.is_empty() {
        return "No routes yet.".into();
    }
    let lines: Vec<String> = routes
        .iter()
        .map(|r| {
            let name = r.full_name();
            let (address, kind) = match (r.protocol, r.listen_port, https_port) {
                (Protocol::Tcp, Some(port), _) => (format!("{name}:{port}"), "tcp, "),
                (Protocol::Tcp, None, _) => (name, "tcp, "),
                (Protocol::Http, _, Some(443)) => (format!("https://{name}"), ""),
                (Protocol::Http, _, Some(port)) => (format!("https://{name}:{port}"), ""),
                (Protocol::Http, _, None) => (name, ""),
            };
            let life = if r.owner_pid.is_some() {
                "owned"
            } else if r.persistent {
                "persistent"
            } else {
                "session"
            };
            let note = if r.note.is_empty() { String::new() } else { format!(" - {}", r.note.replace(['\r', '\n'], " ")) };
            format!("- `{address}` goes to `{}` ({kind}{life}){note}", r.target)
        })
        .collect();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CaState, CaStatus, PortStatus};

    fn route(host: &str, protocol: Protocol, target: &str, listen_port: Option<u16>) -> Route {
        Route {
            host: host.into(),
            protocol,
            target: target.into(),
            listen_port,
            https_only: false,
            note: String::new(),
            owner_pid: None,
            persistent: true,
        }
    }

    fn status(http: Option<u16>, https_errors: &[&str], trusted: Option<bool>) -> StatusResult {
        let port = |configured, port: Option<u16>, errors: &[&str]| PortStatus {
            configured,
            port,
            bound: vec![],
            errors: errors.iter().map(|e| e.to_string()).collect(),
        };
        StatusResult {
            daemon_version: "0.1.2".into(),
            api_version: "1.0".into(),
            pid: 1,
            data_dir: "/tmp".into(),
            http: port(80, http, &[]),
            https: port(443, None, https_errors),
            ca: CaStatus {
                state: CaState::Ok,
                problem: None,
                pem_path: "/tmp/ca.pem".into(),
                common_name: Some("LocalRouter CA 1234".into()),
                trusted,
            },
            routes: 0,
            routes_file_problem: None,
            listen_failed: vec![],
        }
    }

    #[test]
    fn status_shows_ports_ca_and_trust() {
        let s = status(Some(80), &["cannot bind 0.0.0.0:443: address in use"], Some(false));
        let page = render(&[], None, Some(&s));
        assert!(page.contains("| HTTP | on, port 80 |"), "{page}");
        assert!(page.contains("| HTTPS | off: cannot bind 0.0.0.0:443: address in use |"), "{page}");
        assert!(page.contains("| Local CA | ready, LocalRouter CA 1234 |"), "{page}");
        assert!(page.contains("| CA trusted by macOS | no |"), "{page}");
        assert!(page.contains("Trust…"), "the page says how to fix it in the app: {page}");
    }

    #[test]
    fn broken_ca_and_unknown_trust_are_shown() {
        let mut s = status(Some(80), &[], None);
        s.https.port = Some(443);
        s.ca.state = CaState::Broken;
        s.ca.problem = Some("ca.key is missing".into());
        let page = render(&[], Some(443), Some(&s));
        assert!(page.contains("| HTTPS | on, port 443 |"), "{page}");
        assert!(page.contains("| Local CA | broken: ca.key is missing |"), "{page}");
        assert!(page.contains("| CA trusted by macOS | unknown |"), "{page}");
    }

    #[test]
    fn missing_status_says_so() {
        let page = render(&[], Some(443), None);
        assert!(page.contains("Status is not available."), "{page}");
    }

    #[test]
    fn no_routes_leaves_no_placeholder() {
        let page = render(&[], Some(443), None);
        assert!(page.starts_with("# LocalRouter"));
        assert!(page.contains("No routes yet."));
        assert!(!page.contains("{{"), "{page}");
        assert!(page.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn routes_are_listed_with_address_target_kind_and_note() {
        let mut shop = route("shop", Protocol::Http, "http://127.0.0.1:5173", None);
        shop.note = "main\ndev server".into();
        let mut db = route("db.shop", Protocol::Tcp, "tcp://127.0.0.1:5432", Some(15432));
        db.persistent = false;
        let mut feat = route("feat.shop", Protocol::Http, "http://127.0.0.1:5174", None);
        feat.persistent = false;
        feat.owner_pid = Some(42);

        let page = render(&[shop, db, feat], Some(443), None);
        assert!(page.contains("- `https://shop.localhost` goes to `http://127.0.0.1:5173` (persistent) - main dev server"), "{page}");
        assert!(page.contains("- `db.shop.localhost:15432` goes to `tcp://127.0.0.1:5432` (tcp, session)"), "{page}");
        assert!(page.contains("- `https://feat.shop.localhost` goes to `http://127.0.0.1:5174` (owned)"), "{page}");
    }

    #[test]
    fn https_port_other_than_443_is_in_the_url() {
        let page = render(&[route("shop", Protocol::Http, "http://127.0.0.1:5173", None)], Some(8443), None);
        assert!(page.contains("`https://shop.localhost:8443`"), "{page}");
    }
}
