//! `localrouter`: command-line tool and MCP server for LocalRouter.

mod client;
mod mcp;
mod trust;

use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use localrouter_core::api::{
    self, GetLogsParams, GetLogsResult, HostParams, ListRoutesResult, RegisterRouteResult, ResetCaResult,
    StatusResult, SubscribeLogsParams, UnregisterRouteResult,
};
use localrouter_core::logs::LogEntry;
use localrouter_core::paths::Paths;
use localrouter_core::routes::{Protocol, Route};

use crate::client::{Client, ClientError};

#[derive(Parser)]
#[command(name = "localrouter", version, about = "Names instead of ports for local dev servers")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add or replace a route. HTTP: `add shop 5173`. TCP: `add db.shop 55001 --tcp --listen 15432`.
    Add {
        /// Name without .localhost, for example shop or feat-login.shop.
        host: String,
        /// Local port of the server (127.0.0.1). Use --target for anything else.
        port: Option<u16>,
        /// Full target URL: http://, https:// or tcp:// plus a loopback host and a port.
        #[arg(long)]
        target: Option<String>,
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
    /// Remove a route.
    Rm { host: String },
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
    /// Run the MCP server over stdio (for coding agents).
    Mcp,
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
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
    match runtime.block_on(run(cli.command)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("localrouter: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn connect(paths: &Paths) -> Result<Client, ClientError> {
    Client::connect(&paths.socket(), "cli").await
}

async fn run(command: Command) -> anyhow::Result<()> {
    let paths = Paths::from_env();
    match command {
        Command::Mcp => mcp::run(paths.socket()).await,
        Command::CaPath => {
            println!("{}", paths.ca_pem().display());
            Ok(())
        }
        Command::Add { host, port, target, tcp, listen, https_only, note, session, owner_pid } => {
            let protocol = if tcp { Protocol::Tcp } else { Protocol::Http };
            let target = match (target, port) {
                (Some(t), _) => t,
                (None, Some(p)) if tcp => format!("tcp://127.0.0.1:{p}"),
                (None, Some(p)) => format!("http://127.0.0.1:{p}"),
                (None, None) => bail!("give a port or --target"),
            };
            if listen.is_some() && !tcp {
                bail!("--listen is only for TCP routes; add --tcp");
            }
            let route = Route {
                host,
                protocol,
                target,
                listen_port: tcp.then_some(listen.unwrap_or(0)),
                https_only,
                note,
                owner_pid,
                persistent: !session && owner_pid.is_none(),
            };
            let r: RegisterRouteResult = connect(&paths).await?.call("register_route", route).await?;
            let rt = &r.route.route;
            println!("{} -> {}", rt.full_name(), rt.target);
            for url in &r.route.urls {
                println!("  {url}");
            }
            if let Some(old) = r.old_target.filter(|_| r.replaced) {
                println!("  (replaced {old})");
            }
            Ok(())
        }
        Command::Rm { host } => {
            let r: UnregisterRouteResult = connect(&paths).await?.call("unregister_route", HostParams { host: host.clone() }).await?;
            if !r.removed {
                bail!("no route named {host}");
            }
            println!("removed {host}");
            Ok(())
        }
        Command::List { json } => {
            let r: ListRoutesResult = connect(&paths).await?.call("list_routes", api::Empty {}).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
                return Ok(());
            }
            if r.routes.is_empty() {
                println!("No routes. Add one with: localrouter add shop 5173");
            }
            for v in &r.routes {
                let up = match v.upstream_up {
                    Some(true) => "up  ",
                    Some(false) => "down",
                    None => "?   ",
                };
                let failed = if v.listen_failed { "  [listen failed]" } else { "" };
                let kind = if v.route.persistent { "" } else if v.route.owner_pid.is_some() { "  (owned)" } else { "  (session)" };
                println!("{up}  {:<32} -> {}{kind}{failed}", v.urls.first().cloned().unwrap_or_else(|| v.route.full_name()), v.route.target);
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
            print_status(&s);
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
            println!("Removed the LocalRouter CA from the login keychain.");
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
            println!("New CA: {}. Run `localrouter trust` to trust it.", r.common_name);
            Ok(())
        }
    }
}

fn print_status(s: &StatusResult) {
    println!("LocalRouter {} (pid {}), data in {}", s.daemon_version, s.pid, s.data_dir);
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
        Some(false) => "NOT trusted: run `localrouter trust`",
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
}

fn format_entry(e: &LogEntry) -> String {
    match e {
        LogEntry::Http { time_ms, method, host, path, status, duration_ms } => {
            format!("{} {status} {method:<6} {host}{path} {duration_ms} ms", clock(*time_ms))
        }
        LogEntry::Tcp { time_ms, host, listen_port, bytes_in, bytes_out, duration_ms, failed } => {
            let state = if *failed { "FAILED" } else { "tcp" };
            format!("{} {state} {host}:{listen_port} in {bytes_in} B, out {bytes_out} B, {duration_ms} ms", clock(*time_ms))
        }
    }
}

fn clock(time_ms: u64) -> String {
    let secs = time_ms / 1000;
    // UTC HH:MM:SS; enough for a live log.
    format!("{:02}:{:02}:{:02}", secs / 3600 % 24, secs / 60 % 60, secs % 60)
}
