//! `localrouter`: command-line tool and MCP server for LocalRouter.

mod client;
mod lan;
mod mcp;
mod proxy;
mod rules;
mod trust;

use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use localrouter_core::api::{
    self, GetLogsParams, GetLogsResult, HostParams, ListRoutesResult, RegisterRouteResult, ResetCaResult,
    StatusResult, SubscribeLogsParams, UnregisterRouteResult,
};
use localrouter_core::config::Config;
use localrouter_core::help;
use localrouter_core::logs::{LogEntry, ProxyMode};
use localrouter_core::instance::Instance;
use localrouter_core::paths::Paths;
use localrouter_core::routes::{Explanation, FOLDER_SCHEME, Protocol, Route, RouteTable, Step};

use crate::client::{Client, ClientError};

#[derive(Parser)]
#[command(name = "localrouter", version, about = "Names instead of ports for local dev servers")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add or replace a route. HTTP: `add shop 5173`. Folder: `add docs --folder ./dist`. TCP: `add db.shop 55001 --tcp --listen 15432`.
    Add {
        /// Name without .localhost, for example shop or feat-login.shop.
        host: String,
        /// HTTP only: the path prefix this route answers, for example /blog. Other paths of the name go to its route without a path.
        #[arg(long)]
        path: Option<String>,
        /// With --path: remove the path before the request reaches the server (for servers that answer at /).
        #[arg(long)]
        strip_path: bool,
        /// Local port of the server (127.0.0.1). Use --target for anything else.
        port: Option<u16>,
        /// Full target URL: http://, https:// or tcp:// plus a loopback host and a port.
        #[arg(long)]
        target: Option<String>,
        /// Serve the files of this folder, with no dev server. index.html for a folder, else a file list.
        #[arg(long, conflicts_with_all = ["port", "target", "tcp", "listen"])]
        folder: Option<std::path::PathBuf>,
        /// A TCP route (databases, caches) instead of an HTTP route.
        #[arg(long)]
        tcp: bool,
        /// TCP only: the port clients connect to. Default: a free port.
        #[arg(long)]
        listen: Option<u16>,
        /// HTTP only: redirect http:// to https://.
        #[arg(long)]
        https_only: bool,
        /// Why this route exists.
        #[arg(long, default_value = "")]
        note: String,
        /// Do not save the route; it ends when the daemon stops.
        #[arg(long)]
        session: bool,
        /// Remove the route when this process exits (implies --session).
        #[arg(long)]
        owner_pid: Option<u32>,
    },
    /// Remove a route. Without --path, only the route without a path.
    Rm {
        host: String,
        /// The path of a path route, for example /blog.
        #[arg(long)]
        path: Option<String>,
    },
    /// Show which route answers a URL, and why. Makes no request.
    Which {
        /// A URL or a name and path: https://feat-x.shop.localhost/blog, shop.localhost/blog, shop/blog.
        url: String,
    },
    /// List routes.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show recent requests and connections.
    Logs {
        /// Only this name and its subdomains.
        host: Option<String>,
        #[arg(short = 'n', long, default_value_t = 50)]
        limit: usize,
        /// Keep printing new entries.
        #[arg(short, long)]
        follow: bool,
    },
    /// Daemon, ports and CA status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Trust the local CA in the login keychain (macOS asks for your password).
    Trust,
    /// Remove the local CA from the login keychain.
    Untrust,
    /// Print the path of ca.pem (for NODE_EXTRA_CA_CERTS and similar).
    CaPath,
    /// Local CA commands.
    Ca {
        #[command(subcommand)]
        command: CaCommand,
    },
    /// The forward proxy: send Chrome, Claude Code or a test run through LocalRouter and see their requests.
    Proxy {
        #[command(subcommand)]
        command: Option<proxy::ProxyCommand>,
    },
    /// LAN access per network: which networks may reach ports 80 and 443. Without a subcommand: the state.
    Lan {
        #[command(subcommand)]
        command: Option<lan::LanCommand>,
    },
    /// Script rules: Lua scripts that change (intercept) or record (log) HTTP traffic. Without a subcommand: the list.
    Rules {
        /// The list as JSON.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<rules::RulesCommand>,
    },
    /// Print the full guide for coding agents, with the current ports, status and routes.
    Guide,
    /// Run the MCP server over stdio (for coding agents).
    Mcp,
    /// Print the Claude Code note of this instance (used when the app is built).
    #[command(hide = true)]
    Note,
}

#[derive(Subcommand)]
enum CaCommand {
    /// Make a new CA. The old one stops working and is untrusted.
    Reset {
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> ExitCode {
    // Before any folder is opened: a bad suffix must not create one (ADR 04, I3).
    let instance = match Instance::of_this_program() {
        Ok(instance) => instance,
        Err(e) => {
            eprintln!("localrouter: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Usage lines and errors name this instance's command: localrouter-dev.
    // clap wants a static name; one short string per process is leaked.
    let name: &'static str = Box::leak(instance.cli().into_boxed_str());
    let matches = Cli::command().name(name).bin_name(name).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
    match runtime.block_on(run(cli.command, instance)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{name}: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn connect(paths: &Paths) -> Result<Client, ClientError> {
    Client::connect(&paths.socket(), "cli").await
}

async fn run(command: Command, instance: Instance) -> anyhow::Result<()> {
    let paths = Paths::from_env(&instance);
    match command {
        Command::Mcp => mcp::run(paths.socket(), instance).await,
        Command::Proxy { command } => proxy::run(command, &paths, &instance).await,
        Command::Rules { json, command } => rules::run(command, json, &paths, &instance).await,
        Command::Lan { command } => lan::run(command, &paths, &instance).await,
        Command::Note => {
            print!("{}", help::render_note(&instance));
            Ok(())
        }
        Command::Guide => {
            // Without a daemon the guide still helps: it says so in its status.
            let (routes, status) = match connect(&paths).await {
                Ok(mut c) => {
                    let list: ListRoutesResult = c.call("list_routes", api::Empty {}).await?;
                    let status: StatusResult = c.call("status", api::Empty {}).await?;
                    (list.routes.into_iter().map(|v| v.route).collect(), Some(status))
                }
                Err(_) => (vec![], None),
            };
            let (http, https) = status.as_ref().map_or((None, None), |s| (s.http.port, s.https.port));
            print!("{}", help::render(&instance, &routes, http, https, status.as_ref()));
            Ok(())
        }
        Command::CaPath => {
            println!("{}", paths.ca_pem().display());
            Ok(())
        }
        Command::Add { host, path, strip_path, port, target, folder, tcp, listen, https_only, note, session, owner_pid } => {
            let protocol = if tcp { Protocol::Tcp } else { Protocol::Http };
            let target = match (target, port, folder) {
                (Some(t), _, _) => t,
                (None, Some(p), _) if tcp => format!("tcp://127.0.0.1:{p}"),
                (None, Some(p), _) => format!("http://127.0.0.1:{p}"),
                (None, None, Some(f)) => folder_target(&f)?,
                (None, None, None) => bail!("give a port, --target or --folder"),
            };
            if listen.is_some() && !tcp {
                bail!("--listen is only for TCP routes; add --tcp");
            }
            if path.is_some() && tcp {
                bail!("--path is only for HTTP routes; remove --tcp");
            }
            if strip_path && path.is_none() {
                bail!("--strip-path needs --path");
            }
            let route = Route {
                host,
                path,
                protocol,
                target,
                listen_port: tcp.then_some(listen.unwrap_or(0)),
                https_only,
                strip_path,
                note,
                owner_pid,
                persistent: !session && owner_pid.is_none(),
            };
            let r: RegisterRouteResult = connect(&paths).await?.call("register_route", route).await?;
            let rt = &r.route.route;
            println!("{} -> {}", rt.full_name_and_path(), rt.target);
            for url in &r.route.urls {
                println!("  {url}");
            }
            if let Some(old) = r.old_target.filter(|_| r.replaced) {
                println!("  (replaced {old})");
            }
            Ok(())
        }
        Command::Rm { host, path } => {
            let mut c = connect(&paths).await?;
            let r: UnregisterRouteResult =
                c.call("unregister_route", HostParams { host: host.clone(), path: path.clone() }).await?;
            let key = format!("{host}{}", path.as_deref().unwrap_or(""));
            if !r.removed {
                bail!("no route named {key}");
            }
            // Removing the route without a path leaves the path routes (I28); say so.
            let left: Vec<String> = if path.is_none() {
                let list: ListRoutesResult = c.call("list_routes", api::Empty {}).await?;
                let host_key = host.trim().trim_end_matches(".localhost").to_ascii_lowercase();
                list.routes.iter().filter(|v| v.route.host == host_key).map(|v| v.route.key().to_string()).collect()
            } else {
                vec![]
            };
            match left.as_slice() {
                [] => println!("removed {key}"),
                [one] => println!("removed {key}; {one} remains"),
                many => println!("removed {key}; {} remain", many.join(", ")),
            }
            Ok(())
        }
        Command::Which { url } => {
            let mut c = connect(&paths).await?;
            let list: ListRoutesResult = c.call("list_routes", api::Empty {}).await?;
            let config: Config = c.call("get_config", api::Empty {}).await?;
            let routes: Vec<Route> = list.routes.into_iter().map(|v| v.route).collect();
            print!("{}", which(&routes, config.fallback, &url));
            Ok(())
        }
        Command::List { json } => {
            let r: ListRoutesResult = connect(&paths).await?.call("list_routes", api::Empty {}).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
                return Ok(());
            }
            if r.routes.is_empty() {
                println!("No routes. Add one with: {} add shop 5173", instance.cli());
            }
            for v in &r.routes {
                let up = match v.upstream_up {
                    Some(true) => "up  ",
                    Some(false) => "down",
                    None => "?   ",
                };
                let failed = if v.listen_failed { "  [listen failed]" } else { "" };
                let kind = if v.route.persistent { "" } else if v.route.owner_pid.is_some() { "  (owned)" } else { "  (session)" };
                let strip = if v.route.strip_path { "  (strip)" } else { "" };
                let name = v.urls.first().cloned().unwrap_or_else(|| v.route.full_name_and_path());
                println!("{up}  {name:<32} -> {}{strip}{kind}{failed}", v.route.target);
                if !v.route.note.is_empty() {
                    println!("      {}", v.route.note);
                }
            }
            Ok(())
        }
        Command::Logs { host, limit, follow } => {
            let mut c = connect(&paths).await?;
            let r: GetLogsResult = c.call("get_logs", GetLogsParams { host: host.clone(), limit: Some(limit) }).await?;
            for e in &r.entries {
                println!("{}", format_entry(e));
            }
            if follow {
                let _: serde_json::Value = c.call("subscribe_logs", SubscribeLogsParams { host }).await?;
                loop {
                    println!("{}", format_entry(&c.next_log().await?));
                }
            }
            Ok(())
        }
        Command::Status { json } => {
            let s: StatusResult = connect(&paths).await?.call("status", api::Empty {}).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
                return Ok(());
            }
            print_status(&instance, &s);
            Ok(())
        }
        Command::Trust => {
            trust::trust(&paths.ca_pem())?;
            println!("Trusted {}.", paths.ca_pem().display());
            Ok(())
        }
        Command::Untrust => {
            let cn = match connect(&paths).await {
                Ok(mut c) => c.call::<StatusResult>("status", api::Empty {}).await.ok().and_then(|s| s.ca.common_name),
                Err(_) => None,
            };
            trust::untrust(&paths.ca_pem(), cn.as_deref())?;
            println!("Removed the {} CA from the login keychain.", instance.app_name());
            Ok(())
        }
        Command::Ca { command: CaCommand::Reset { yes } } => {
            if !yes {
                eprintln!("This makes a new CA. Browsers stop trusting the old one. Run again with --yes to continue.");
                bail!("not confirmed");
            }
            let mut c = connect(&paths).await?;
            let s: StatusResult = c.call("status", api::Empty {}).await?;
            trust::untrust(&paths.ca_pem(), s.ca.common_name.as_deref()).context("could not remove the old CA")?;
            let r: ResetCaResult = c.call("reset_ca", api::Empty {}).await?;
            println!("New CA: {}. Run `{} trust` to trust it.", r.common_name, instance.cli());
            Ok(())
        }
    }
}

/// `./dist` → `file:///Users/me/shop/dist`. The daemon has no working
/// folder, so the path is made absolute here.
fn folder_target(folder: &std::path::Path) -> anyhow::Result<String> {
    let abs = std::fs::canonicalize(folder).with_context(|| format!("folder {}", folder.display()))?;
    if !abs.is_dir() {
        bail!("{} is not a folder", abs.display());
    }
    let abs = abs.to_str().with_context(|| format!("{} is not UTF-8", abs.display()))?;
    Ok(format!("{FOLDER_SCHEME}{abs}"))
}

fn print_status(instance: &Instance, s: &StatusResult) {
    // The instance and its folder, so LOCALROUTER_HOME in a shell cannot hide
    // which daemon answered (ADR 04, gap G4).
    let home = if std::env::var_os(localrouter_core::paths::HOME_ENV).is_some_and(|h| !h.is_empty()) {
        " (from LOCALROUTER_HOME)"
    } else {
        ""
    };
    println!("{} {} (pid {}), data in {}{home}", instance.app_name(), s.daemon_version, s.pid, s.data_dir);
    for (name, p) in [("HTTP ", &s.http), ("HTTPS", &s.https)] {
        match p.port {
            Some(port) => println!("{name}  port {port}"),
            None => println!("{name}  not listening"),
        }
        for e in &p.errors {
            println!("       {e}");
        }
    }
    let trusted = match s.ca.trusted {
        Some(true) => "trusted",
        Some(false) => &format!("NOT trusted: run `{} trust`", instance.cli()),
        None => "trust unknown",
    };
    match s.ca.state {
        api::CaState::Ok => println!("CA     {} ({trusted})", s.ca.common_name.as_deref().unwrap_or("?")),
        api::CaState::Broken => println!("CA     broken: {}", s.ca.problem.as_deref().unwrap_or("?")),
    }
    println!("Routes {}", s.routes);
    if let Some(p) = &s.routes_file_problem {
        println!("       {p}");
    }
    for h in &s.listen_failed {
        println!("       TCP route {h}: listen port is taken");
    }
    for note in &s.notes {
        println!("Note   {note}");
    }
}

fn format_entry(e: &LogEntry) -> String {
    match e {
        LogEntry::Http {
            time_ms,
            method,
            host,
            path,
            status,
            duration_ms,
            route,
            via: _,
            mode,
            bytes_in,
            bytes_out,
            rules,
            script_error,
        } => {
            let route = route.as_deref().map(|r| format!(" route {r}")).unwrap_or_default();
            let via = match mode {
                Some(ProxyMode::Http) => " via proxy (http)",
                Some(ProxyMode::Inspect) => " via proxy (inspect)",
                Some(ProxyMode::Tunnel) => " via proxy (tunnel)",
                None => "",
            };
            let bytes = match (bytes_in, bytes_out) {
                (Some(i), Some(o)) => format!(" in {i} B, out {o} B,"),
                _ => String::new(),
            };
            let rules = if rules.is_empty() { String::new() } else { format!(" rules {}", rules.join(",")) };
            let failed = script_error.as_deref().map(|r| format!(" (rule {r} failed)")).unwrap_or_default();
            format!("{} {status} {method:<6} {host}{path}{bytes} {duration_ms} ms{route}{via}{rules}{failed}", clock(*time_ms))
        }
        LogEntry::Tcp { time_ms, host, listen_port, bytes_in, bytes_out, duration_ms, failed } => {
            let state = if *failed { "FAILED" } else { "tcp" };
            format!("{} {state} {host}:{listen_port} in {bytes_in} B, out {bytes_out} B, {duration_ms} ms", clock(*time_ms))
        }
    }
}

/// Split `https://shop.localhost:8443/blog/x?y=1`, `shop.localhost/blog` or
/// `shop/blog` into a name under `.localhost` (with its port, if any) and a path
/// without the query.
fn split_url(url: &str) -> (String, String) {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let (name, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let path = path.split(['?', '#']).next().unwrap_or("/");
    let bare = name.rsplit_once(':').map_or(name, |(h, p)| if p.bytes().all(|b| b.is_ascii_digit()) { h } else { name });
    let name = if bare.to_ascii_lowercase().ends_with(".localhost") || bare.eq_ignore_ascii_case("localhost") {
        name.to_string()
    } else {
        name.replacen(bare, &format!("{bare}.localhost"), 1)
    };
    (name, if path.is_empty() { "/".into() } else { path.to_string() })
}

/// The `localrouter which` report: the same lookup the proxy runs (I33),
/// with each step.
fn which(routes: &[Route], fallback: bool, url: &str) -> String {
    let (name, path) = split_url(url);
    let mut out = String::new();
    if let Some((_, port)) = name.rsplit_once(':')
        && let Ok(port) = port.parse::<u16>()
        && let Some(r) = routes.iter().find(|r| r.protocol == Protocol::Tcp && r.listen_port == Some(port))
    {
        out.push_str(&format!("{name} -> tcp route {} -> {}\n", r.host, r.target));
        out.push_str("  TCP routes are chosen by listen port; the name is not checked.\n");
        return out;
    }
    let mut table = RouteTable::new();
    for r in routes {
        table.insert(r.clone());
    }
    let Explanation { route, steps } = table.explain(&name, &path, fallback);
    let asked = format!("{name}{path}");
    match &route {
        Some(r) => out.push_str(&format!("{asked} -> {} -> {}\n", r.key(), r.target)),
        None => out.push_str(&format!("{asked} -> no route (the 404 page)\n")),
    }
    let show = |p: &Option<String>| p.clone().unwrap_or_else(|| "(no path)".into());
    for (i, step) in steps.iter().enumerate() {
        let line = match step {
            Step::NotLocalhost => "not a .localhost name".to_string(),
            Step::Key { host, paths, matched: _ } if paths.is_empty() => format!("{host}: no routes"),
            Step::Key { host, paths, matched: None } => {
                let list: Vec<String> = paths.iter().map(show).collect();
                format!("{host}: routes {}; none matches {path}", list.join(", "))
            }
            Step::Key { host, matched: Some(None), .. } => format!("{host}: the route without a path matches"),
            Step::Key { host, paths, matched: Some(Some(p)) } if paths.len() > 1 => {
                let list: Vec<String> = paths.iter().map(show).collect();
                format!("{host}: {p} matches, the longest of {}", list.join(", "))
            }
            Step::Key { host, matched: Some(Some(p)), .. } => format!("{host}: {p} matches"),
            Step::Fallback { parent } => format!("fallback is on: try {parent}"),
            Step::FallbackOff => "fallback is off: stop".to_string(),
        };
        out.push_str(&format!("  {}. {line}\n", i + 1));
    }
    out
}

fn clock(time_ms: u64) -> String {
    let secs = time_ms / 1000;
    // UTC HH:MM:SS; enough for a live log.
    format!("{:02}:{:02}:{:02}", secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(host: &str, path: Option<&str>, port: u16) -> Route {
        Route {
            host: host.into(),
            path: path.map(str::to_string),
            protocol: Protocol::Http,
            target: format!("http://127.0.0.1:{port}"),
            listen_port: None,
            https_only: false,
            strip_path: false,
            note: String::new(),
            owner_pid: None,
            persistent: false,
        }
    }

    #[test]
    fn urls_are_split_into_name_and_path() {
        for (url, name, path) in [
            ("https://feat-x.shop.localhost/blog/1?x=2", "feat-x.shop.localhost", "/blog/1"),
            ("http://shop.localhost:8080/", "shop.localhost:8080", "/"),
            ("shop.localhost/blog", "shop.localhost", "/blog"),
            ("shop/blog", "shop.localhost", "/blog"),
            ("shop", "shop.localhost", "/"),
            ("db.shop.localhost:15432", "db.shop.localhost:15432", "/"),
        ] {
            assert_eq!(split_url(url), (name.to_string(), path.to_string()), "{url}");
        }
    }

    #[test]
    fn which_explains_a_fallback() {
        let routes = [route("shop", None, 5173), route("feat-x.shop", Some("/blog"), 3002)];
        let out = which(&routes, true, "https://feat-x.shop.localhost/products");
        assert_eq!(
            out,
            "feat-x.shop.localhost/products -> shop -> http://127.0.0.1:5173\n  \
             1. feat-x.shop: routes /blog; none matches /products\n  \
             2. fallback is on: try shop\n  \
             3. shop: the route without a path matches\n"
        );
    }

    #[test]
    fn which_names_the_longest_path_and_the_missing_route() {
        let routes = [route("shop", None, 1), route("shop", Some("/blog"), 2), route("shop", Some("/blog/admin"), 3)];
        let out = which(&routes, true, "shop.localhost/blog/admin/x");
        assert!(out.starts_with("shop.localhost/blog/admin/x -> shop/blog/admin -> http://127.0.0.1:3\n"), "{out}");
        assert!(out.contains("/blog/admin matches, the longest of (no path), /blog, /blog/admin"), "{out}");
        let out = which(&routes, false, "blog.localhost/");
        assert!(out.starts_with("blog.localhost/ -> no route (the 404 page)"), "{out}");
        assert!(out.contains("fallback is off"), "{out}");
    }

    #[test]
    fn which_on_a_tcp_listen_port_names_the_tcp_route() {
        let mut db = route("db.shop", None, 1);
        db.protocol = Protocol::Tcp;
        db.target = "tcp://127.0.0.1:5432".into();
        db.listen_port = Some(15432);
        let out = which(&[db], true, "db.shop.localhost:15432");
        assert!(out.starts_with("db.shop.localhost:15432 -> tcp route db.shop -> tcp://127.0.0.1:5432"), "{out}");
    }

    #[test]
    fn folder_target_is_absolute_and_resolved() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("dist")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        let real = dir.path().canonicalize().unwrap();
        let t = folder_target(&dir.path().join("x/../dist")).unwrap_err();
        assert!(t.to_string().contains("folder"), "{t:#}");
        assert_eq!(folder_target(&dir.path().join("dist/")).unwrap(), format!("file://{}/dist", real.display()));
        assert!(format!("{:#}", folder_target(&dir.path().join("a.txt")).unwrap_err()).contains("is not a folder"));
        assert!(format!("{:#}", folder_target(&dir.path().join("nope")).unwrap_err()).contains("nope"));
    }
}
