//! The texts people and coding agents read: the page at `router.localhost`
//! (with the daemon status and the routes filled in), the Claude Code note,
//! and the MCP server instructions. Each names its own instance: its CLI, its
//! help URL and its ports (ADR 04, I4).

use crate::api::{CaState, PortStatus, StatusResult};
use crate::instance::{Instance, help_url, port_part};
use crate::routes::{Protocol, Route};

const TEMPLATE: &str = include_str!("help.md");
const NOTE: &str = include_str!("note.md");
const MCP_INSTRUCTIONS: &str = include_str!("mcp.md");

/// The help page text. The ports are the bound ones, if any.
pub fn render(
    instance: &Instance,
    routes: &[Route],
    http_port: Option<u16>,
    https_port: Option<u16>,
    status: Option<&StatusResult>,
) -> String {
    // The proxy port the daemon uses, else the instance's default (in fill).
    let proxy_port = status.and_then(|s| s.proxy.as_ref()).map(|p| p.port.unwrap_or(p.configured)).filter(|&p| p != 0);
    let template = match proxy_port {
        Some(port) => TEMPLATE.replace("{{PROXY_PORT}}", &port.to_string()),
        None => TEMPLATE.to_string(),
    };
    fill(&template, instance, http_port, https_port)
        .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
        .replace("{{STATUS}}", &status_table(status))
        .replace("{{ROUTES}}", &route_list(routes, https_port))
}

/// The Claude Code note, linked into `~/.claude`. It is made when the bundle
/// is built, so it names only the default ports and points to `<cli> guide`
/// for the real ones.
pub fn render_note(instance: &Instance) -> String {
    fill(NOTE, instance, None, None)
}

/// The MCP server's instructions. They name no port: the reply of
/// `register_route` has the exact URL.
pub fn mcp_instructions(instance: &Instance) -> String {
    fill(MCP_INSTRUCTIONS, instance, None, None).trim_end().to_string()
}

/// The sentence a suffixed instance puts first, so an agent that reads the
/// notes of two instances knows which one to use (ADR 04, gap G1). Empty for
/// the release.
pub fn instance_note(instance: &Instance, https_port: Option<u16>) -> String {
    if instance.is_release() {
        return String::new();
    }
    let release = Instance::release();
    let https = https_port.unwrap_or(instance.default_ports().1);
    format!(
        "> This is **{}**, a development build of LocalRouter next to the release. Use `{}` only when the user asks for \
         the dev build or gives a URL with port `:{https}`. For everything else use the release: `{}`.\n\n",
        instance.app_name(),
        instance.cli(),
        release.cli(),
    )
}

/// Placeholders every text shares. Ports missing from `http_port` and
/// `https_port` are the instance's defaults.
fn fill(template: &str, instance: &Instance, http_port: Option<u16>, https_port: Option<u16>) -> String {
    let (default_http, default_https) = instance.default_ports();
    let http = http_port.unwrap_or(default_http);
    let https = https_port.unwrap_or(default_https);
    let step7 = if instance.is_release() {
        String::new()
    } else {
        format!(
            "Project files are shared with people who run the release, so write the release names there: `{}`, and \
             URLs without `:{https}`. `{}` is only for you, on this Mac.\n\n",
            Instance::release().cli(),
            instance.cli()
        )
    };
    // Browsers keep cookies by name, not by port (ADR 04, manifest blast radius).
    let cookies = if instance.is_release() {
        String::new()
    } else {
        format!(
            "\nBrowsers share cookies between ports: a login on `shop.localhost` also shows up on \
             `shop.localhost{}`, and the other way round.\n",
            port_part(https, 443)
        )
    };
    template
        .replace("{{COOKIE_NOTE}}", &cookies)
        .replace("{{INSTANCE_NOTE}}", &instance_note(instance, Some(https)))
        .replace("{{STEP7_NOTE}}", &step7)
        .replace("{{APP}}", &instance.app_name())
        .replace("{{CLI}}", &instance.cli())
        .replace("{{DAEMON_LABEL}}", &instance.daemon_label())
        .replace("{{HELP_URL}}", &help_url(Some(http), Some(https)))
        .replace("{{DEFAULT_HELP_URL}}", &instance.default_help_url())
        .replace("{{HTTP_PORT}}", &http.to_string())
        .replace("{{HTTPS_PORT}}", &https.to_string())
        .replace("{{PROXY_PORT}}", &instance.default_proxy_port().to_string())
        .replace("{{HTTP}}", &port_part(http, 80))
        .replace("{{HTTPS}}", &port_part(https, 443))
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
    let proxy = match &s.proxy {
        None => String::new(),
        Some(p) => {
            let state = match (p.enabled, p.port) {
                (true, Some(port)) => format!("on, 127.0.0.1:{port}"),
                (true, None) => format!("not listening: {}", p.errors.join("; ")),
                (false, _) => "off".into(),
            };
            format!("\n| Proxy | {} |", state.replace('|', "/"))
        }
    };
    format!(
        "| Part | State |\n|---|---|\n| HTTP | {} |\n| HTTPS | {} |\n| Local CA | {} |\n| CA trusted by macOS | {trusted} |{proxy}",
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
            let path = r.path.as_deref().unwrap_or("");
            let (address, kind) = match (r.protocol, r.listen_port, https_port) {
                (Protocol::Tcp, Some(port), _) => (format!("{name}:{port}"), "tcp, "),
                (Protocol::Tcp, None, _) => (name, "tcp, "),
                (Protocol::Http, _, Some(443)) => (format!("https://{name}{path}"), ""),
                (Protocol::Http, _, Some(port)) => (format!("https://{name}:{port}{path}"), ""),
                (Protocol::Http, _, None) => (format!("{name}{path}"), ""),
            };
            let kind = if r.strip_path { "strip, " } else { kind };
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
            path: None,
            protocol,
            target: target.into(),
            listen_port,
            https_only: false,
            strip_path: false,
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
            proxy: None,
        }
    }

    #[test]
    fn status_shows_ports_ca_and_trust() {
        let s = status(Some(80), &["cannot bind 0.0.0.0:443: address in use"], Some(false));
        let page = render(&Instance::release(), &[], None, None, Some(&s));
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
        let page = render(&Instance::release(), &[], None, Some(443), Some(&s));
        assert!(page.contains("| HTTPS | on, port 443 |"), "{page}");
        assert!(page.contains("| Local CA | broken: ca.key is missing |"), "{page}");
        assert!(page.contains("| CA trusted by macOS | unknown |"), "{page}");
    }

    #[test]
    fn missing_status_says_so() {
        let page = render(&Instance::release(), &[], None, Some(443), None);
        assert!(page.contains("Status is not available."), "{page}");
    }

    #[test]
    fn no_routes_leaves_no_placeholder() {
        let page = render(&Instance::release(), &[], None, Some(443), None);
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

        let page = render(&Instance::release(), &[shop, db, feat], None, Some(443), None);
        assert!(page.contains("- `https://shop.localhost` goes to `http://127.0.0.1:5173` (persistent) - main dev server"), "{page}");
        assert!(page.contains("- `db.shop.localhost:15432` goes to `tcp://127.0.0.1:5432` (tcp, session)"), "{page}");
        assert!(page.contains("- `https://feat.shop.localhost` goes to `http://127.0.0.1:5174` (owned)"), "{page}");
    }

    #[test]
    fn path_routes_are_listed_with_their_path() {
        let mut blog = route("shop", Protocol::Http, "http://127.0.0.1:3001", None);
        blog.path = Some("/blog".into());
        let mut api = route("shop", Protocol::Http, "http://127.0.0.1:8000", None);
        api.path = Some("/api".into());
        api.strip_path = true;
        let page = render(&Instance::release(), &[blog, api], None, Some(443), None);
        assert!(page.contains("- `https://shop.localhost/blog` goes to `http://127.0.0.1:3001` (persistent)"), "{page}");
        assert!(page.contains("- `https://shop.localhost/api` goes to `http://127.0.0.1:8000` (strip, persistent)"), "{page}");
    }

    #[test]
    fn https_port_other_than_443_is_in_the_url() {
        let page = render(&Instance::release(), &[route("shop", Protocol::Http, "http://127.0.0.1:5173", None)], None, Some(8443), None);
        assert!(page.contains("`https://shop.localhost:8443`"), "{page}");
    }
}
