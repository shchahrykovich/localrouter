//! `localrouter proxy …`: the forward proxy (ADR 06, change 3). Every value
//! comes from the daemon's `get_proxy`, so the CLI, the MCP tool and the app
//! give the same settings (I18).

use anyhow::{Context, bail};
use clap::Subcommand;
use localrouter_core::api::{
    self, CaState, FindFreePortParams, FindFreePortResult, GetProxyParams, GetProxyResult, LanProxyInfo, NewSetupCodeParams,
    NewSetupCodeResult, ResetCaResult, SetConfigParams, SetConfigResult, SetPhoneDeviceParams, SetPhoneDeviceResult,
};
use localrouter_core::config::{Config, DEFAULT_CLIENT, ProxyClient};
use localrouter_core::instance::Instance;
use localrouter_core::paths::Paths;

use crate::client::Client;
use crate::trust;

#[derive(Subcommand)]
pub enum ProxyCommand {
    /// Turn the proxy on (binds 127.0.0.1 and ::1 at once).
    On,
    /// Turn the proxy off and close its connections.
    Off,
    /// Move the proxy to another port.
    Port { port: u16 },
    /// Print `export` lines: eval "$(<this command>)" before starting a program.
    Env {
        /// For this proxy client's port instead of the main one.
        #[arg(long)]
        client: Option<String>,
    },
    /// Open a separate Chrome window that uses the proxy (its own profile).
    Chrome {
        /// Print the command instead of running it.
        #[arg(long)]
        print: bool,
        /// Use this proxy client's port, with a Chrome profile of its own.
        #[arg(long)]
        client: Option<String>,
    },
    /// Proxy clients: one more proxy port per program, so the log shows which one sent what. Without a subcommand: the list.
    Client {
        #[command(subcommand)]
        command: Option<ClientCommand>,
    },
    /// Hosts whose HTTPS is read (inspected) instead of passed through.
    Inspect {
        #[command(subcommand)]
        command: InspectCommand,
    },
    /// Trust the inspection CA in the login keychain (macOS asks for your password).
    Trust,
    /// Remove the inspection CA from the login keychain.
    Untrust,
    /// Print the path of the inspection CA's ca.pem (for NODE_EXTRA_CA_CERTS).
    CaPath,
    /// Inspection CA commands.
    Ca {
        #[command(subcommand)]
        command: ProxyCaCommand,
    },
    /// The proxy log: every proxied request in HAR files. Without a subcommand: its state.
    Log {
        #[command(subcommand)]
        command: Option<LogCommand>,
    },
}

#[derive(Subcommand)]
pub enum LogCommand {
    /// Write every proxied request to a HAR file.
    On,
    /// Stop writing. The files stay.
    Off,
    /// When a file is full: MB, requests, or both. The first reached starts a new file.
    Limits {
        /// 1 to 200.
        #[arg(long)]
        mb: Option<u64>,
        /// 100 to 1000000.
        #[arg(long)]
        requests: Option<u64>,
    },
    /// Open the log viewer in the browser.
    Open {
        /// Open the page of one proxy client.
        #[arg(long)]
        client: Option<String>,
    },
    /// Print the folder of the HAR files, for scripts.
    Path,
}

#[derive(Subcommand)]
pub enum ClientCommand {
    /// Add a proxy client with its own port: chrome, agent-1 (a-z, 0-9, '-').
    Add {
        name: String,
        /// The port; without it, the next free port after the proxy port.
        #[arg(long)]
        port: Option<u16>,
        /// A phone client: its port listens on the LAN and takes the allowed
        /// devices (ADR 10). Needs LAN access on this network.
        #[arg(long)]
        lan: bool,
    },
    /// Remove a proxy client and close its port. Its log entries stay.
    Rm { name: String },
    /// Print what a phone needs: the server, the port, and the setup URL
    /// (the app shows it as a QR code; opening it allows the device).
    Setup {
        name: String,
        /// A new setup URL: the old QR code stops allowing devices.
        #[arg(long)]
        new_code: bool,
    },
    /// Allow a device (its IP address) on a phone client.
    Allow { name: String, address: String },
    /// Remove a device from a phone client, or refuse one that asks.
    Deny { name: String, address: String },
    /// Send a phone set to Automatic direct: its PAC file says DIRECT.
    Pause { name: String },
    /// Send a phone set to Automatic through the proxy again.
    Resume { name: String },
    /// List the proxy clients and their ports.
    List,
}

#[derive(Subcommand)]
pub enum InspectCommand {
    /// Inspect a host: api.example.com, *.example.com for every name under it, or '*' for every host.
    Add { pattern: String },
    /// Stop inspecting a host.
    Rm { pattern: String },
    /// List the inspected hosts.
    List,
}

#[derive(Subcommand)]
pub enum ProxyCaCommand {
    /// Make a new inspection CA. The old one stops working and is untrusted.
    Reset {
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
}

/// The order `proxy env` prints; other variables follow.
const ENV_ORDER: [&str; 11] = [
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "NODE_USE_ENV_PROXY",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "https_proxy",
    "http_proxy",
    "no_proxy",
];

pub async fn run(command: Option<ProxyCommand>, paths: &Paths, instance: &Instance) -> anyhow::Result<()> {
    let cli = instance.cli();
    // `ca-path` works without a daemon, like the local CA's.
    if let Some(ProxyCommand::CaPath) = command {
        println!("{}", paths.inspect_ca_pem().display());
        return Ok(());
    }
    // `log path` too: `ls -t "$(… proxy log path)"` works with no daemon.
    if let Some(ProxyCommand::Log { command: Some(LogCommand::Path) }) = command {
        println!("{}", paths.proxy_log_dir().display());
        return Ok(());
    }
    let mut c = Client::connect(&paths.socket(), "cli").await?;
    match command {
        None => {
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            print!("{}", describe(&p, instance));
        }
        Some(ProxyCommand::Client { command }) => clients(&mut c, command.unwrap_or(ClientCommand::List), instance).await?,
        Some(ProxyCommand::On) => {
            set(&mut c, SetConfigParams { proxy_enabled: Some(true), ..Default::default() }).await?;
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            println!("The proxy is on: {}", p.url);
            println!("Use it: eval \"$({cli} proxy env)\" && claude, or {cli} proxy chrome");
            // Headers reach the disk from now on: say where (gap G2).
            if let Some(line) = log_line(&p, instance) {
                println!("{line}");
            }
        }
        Some(ProxyCommand::Off) => {
            let before: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            set(&mut c, SetConfigParams { proxy_enabled: Some(false), ..Default::default() }).await?;
            println!("The proxy is off.");
            println!(
                "Programs started with HTTPS_PROXY={} now fail to connect. Restart them without it.",
                before.url
            );
        }
        Some(ProxyCommand::Port { port }) => {
            set(&mut c, SetConfigParams { proxy_port: Some(port), ..Default::default() }).await?;
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            println!("The proxy port is {}. Programs that use the old port must be started again.", p.port);
        }
        Some(ProxyCommand::Env { client }) => {
            let p = get_proxy(&mut c, client).await?;
            for line in env_lines(&p) {
                println!("{line}");
            }
            // To stderr, so `eval "$(… proxy env)"` reads only the exports.
            for note in &p.notes {
                eprintln!("# {note}");
            }
        }
        Some(ProxyCommand::Chrome { print, client }) => {
            let mut p = get_proxy(&mut c, client.clone()).await?;
            if print {
                println!("{}", shell_line(&chrome_command(&p.chrome_args)));
                return Ok(());
            }
            if !p.enabled {
                set(&mut c, SetConfigParams { proxy_enabled: Some(true), ..Default::default() }).await?;
                p = get_proxy(&mut c, client).await?;
                println!("Turned the proxy on.");
            }
            let command = chrome_command(&p.chrome_args);
            let out = std::process::Command::new(&command[0]).args(&command[1..]).output().context("cannot run open")?;
            if !out.status.success() {
                bail!("could not start Google Chrome: {}", String::from_utf8_lossy(&out.stderr).trim());
            }
            println!("Chrome started with the proxy at {}.", p.url.trim_start_matches("http://"));
            for note in &p.notes {
                println!("Note: {note}");
            }
        }
        Some(ProxyCommand::Inspect { command }) => {
            let config: Config = c.call("get_config", api::Empty {}).await?;
            let mut list = config.inspect_hosts;
            match command {
                InspectCommand::List => {
                    let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
                    if p.inspect_set.is_empty() {
                        println!("No inspected hosts: every CONNECT is a tunnel.");
                    }
                    for host in &p.inspect_set {
                        println!("{host}");
                    }
                    return Ok(());
                }
                InspectCommand::Add { pattern } => {
                    list.push(pattern);
                }
                InspectCommand::Rm { pattern } => {
                    let wanted = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
                    let before = list.len();
                    list.retain(|p| *p != wanted);
                    if list.len() == before {
                        bail!("{pattern} is not in the inspect list");
                    }
                }
            }
            set(&mut c, SetConfigParams { inspect_hosts: Some(list), ..Default::default() }).await?;
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            if p.inspect_hosts.is_empty() {
                println!("No inspected hosts: every CONNECT is a tunnel.");
            } else {
                println!("Inspected: {}", p.inspect_hosts.join(", "));
            }
            for note in &p.notes {
                println!("Note: {note}");
            }
        }
        Some(ProxyCommand::Trust) => {
            let pem = paths.inspect_ca_pem();
            if !pem.exists() {
                bail!("the inspection CA does not exist yet; it is made by `{cli} proxy inspect add <host>`");
            }
            trust::trust(&pem)?;
            println!("Trusted {}. It lets {} read HTTPS traffic for the hosts you list.", pem.display(), instance.app_name());
        }
        Some(ProxyCommand::Untrust) => {
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            let cn = p.inspect_ca.and_then(|ca| ca.common_name);
            trust::untrust(&paths.inspect_ca_pem(), cn.as_deref())?;
            println!("Removed the {} inspection CA from the login keychain.", instance.app_name());
        }
        Some(ProxyCommand::CaPath) => unreachable!("answered above"),
        Some(ProxyCommand::Log { command }) => log(&mut c, command, instance).await?,
        Some(ProxyCommand::Ca { command: ProxyCaCommand::Reset { yes } }) => {
            if !yes {
                eprintln!("This makes a new inspection CA. Programs stop trusting the old one. Run again with --yes to continue.");
                bail!("not confirmed");
            }
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            let cn = p.inspect_ca.and_then(|ca| ca.common_name);
            trust::untrust(&paths.inspect_ca_pem(), cn.as_deref()).context("could not remove the old inspection CA")?;
            let r: ResetCaResult = c.call("reset_inspect_ca", api::Empty {}).await?;
            println!("New inspection CA: {}. Run `{cli} proxy trust` to trust it.", r.common_name);
        }
    }
    Ok(())
}

/// `proxy log …` (ADR 08, change 3).
async fn log(c: &mut Client, command: Option<LogCommand>, instance: &Instance) -> anyhow::Result<()> {
    let params = match command {
        None => None,
        Some(LogCommand::On) => Some(SetConfigParams { proxy_log: Some(true), ..Default::default() }),
        Some(LogCommand::Off) => Some(SetConfigParams { proxy_log: Some(false), ..Default::default() }),
        Some(LogCommand::Limits { mb, requests }) => {
            if mb.is_none() && requests.is_none() {
                bail!("give --mb, --requests or both");
            }
            Some(SetConfigParams { proxy_log_file_mb: mb, proxy_log_file_requests: requests, ..Default::default() })
        }
        Some(LogCommand::Open { client }) => {
            let p = get_proxy(c, client).await?;
            let Some(log) = p.log else { bail!("this daemon has no proxy log; update it") };
            let out = std::process::Command::new("open").arg(&log.url).output().context("cannot run open")?;
            if !out.status.success() {
                bail!("could not open {}: {}", log.url, String::from_utf8_lossy(&out.stderr).trim());
            }
            println!("Opened {}", log.url);
            return Ok(());
        }
        Some(LogCommand::Path) => unreachable!("answered above"),
    };
    if let Some(params) = params {
        set(c, params).await?;
    }
    let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
    print!("{}", describe_log(&p, instance));
    Ok(())
}

/// The `Log:` line of `proxy` and `proxy on`.
fn log_line(p: &GetProxyResult, instance: &Instance) -> Option<String> {
    let log = p.log.as_ref()?;
    let cli = instance.cli();
    Some(if log.enabled {
        format!("Log: on, writes every request to {} ({cli} proxy log off to stop)", log.folder)
    } else {
        format!("Log: off ({cli} proxy log on to write every request to {})", log.folder)
    })
}

/// The state of the proxy log as text.
pub fn describe_log(p: &GetProxyResult, instance: &Instance) -> String {
    let Some(log) = &p.log else { return "This daemon has no proxy log; update it.\n".to_string() };
    let cli = instance.cli();
    let mut out = String::new();
    if log.enabled {
        out.push_str(&format!("Log        on, writes every proxied request to {}\n", log.folder));
    } else {
        out.push_str(&format!("Log        off: {cli} proxy log on. The files in {} stay.\n", log.folder));
    }
    if !p.enabled {
        out.push_str(&format!("           The proxy is off, so nothing is written: {cli} proxy on\n"));
    }
    out.push_str(&format!("Viewer     {}\n", log.url));
    out.push_str(&format!(
        "Limits     {} MB or {} requests per file; the {} newest files are kept\n",
        log.file_mb, log.file_requests, log.keep_files
    ));
    match &log.current {
        Some(name) => out.push_str(&format!("Current    {name}\n")),
        None => out.push_str("Current    none yet: the next proxied request starts a file\n"),
    }
    out.push_str(&format!("Written    {} since the daemon started, {} files in the folder\n", log.written, log.files));
    if log.dropped > 0 {
        out.push_str(&format!("Dropped    {}: the writer was behind\n", log.dropped));
    }
    if let Some(e) = &log.error {
        out.push_str(&format!("Error      {e}. Nothing is written until: {cli} proxy log off && {cli} proxy log on\n"));
    }
    out.push_str("\nHeaders and URLs are written as they are, cookies and API keys too, and the first 1 MB of each body.\n");
    out
}

async fn set(c: &mut Client, params: SetConfigParams) -> anyhow::Result<SetConfigResult> {
    Ok(c.call("set_config", params).await?)
}

/// `get_proxy` for the main port or one proxy client's. A daemon before API
/// 1.6 ignores `client` and answers for the main port: refuse that.
async fn get_proxy(c: &mut Client, client: Option<String>) -> anyhow::Result<GetProxyResult> {
    let client = client.filter(|n| n != DEFAULT_CLIENT);
    let p: GetProxyResult = c.call("get_proxy", GetProxyParams { client: client.clone() }).await?;
    if client.is_some() && p.client != client {
        bail!("the daemon does not know proxy clients; update it");
    }
    Ok(p)
}

/// `proxy client …` (ADR 09).
async fn clients(c: &mut Client, command: ClientCommand, instance: &Instance) -> anyhow::Result<()> {
    let cli = instance.cli();
    let config: Config = c.call("get_config", api::Empty {}).await?;
    let mut list = config.proxy_clients.clone();
    let pause = matches!(command, ClientCommand::Pause { .. });
    match command {
        ClientCommand::List => {
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            print!("{}", describe_clients(&p, &config, instance));
            return Ok(());
        }
        ClientCommand::Add { name, port, lan } => {
            ProxyClient::check_name(&name).map_err(anyhow::Error::msg)?;
            if list.iter().any(|x| x.name == name) {
                bail!("proxy client {name} exists; remove it first: {cli} proxy client rm {name}");
            }
            let port = match port {
                Some(port) => port,
                None => next_port(c, &config).await?,
            };
            list.push(ProxyClient { name: name.clone(), port, lan, paused: false });
            set(c, SetConfigParams { proxy_clients: Some(list), ..Default::default() }).await?;
            let p = get_proxy(c, Some(name.clone())).await?;
            if let Some(info) = &p.lan {
                println!("Phone client {name}: port {} on the LAN, for allowed devices.", info.port);
                print!("{}", describe_lan(info));
                println!("The app shows the setup URL as a QR code: Proxy tab, Phone.");
                return Ok(());
            }
            println!("Proxy client {name}: {}", p.url);
            println!("Use it: eval \"$({cli} proxy env --client {name})\" && claude, or {cli} proxy chrome --client {name}");
            if let Some(log) = &p.log {
                println!("Its requests: {}", log.url);
            }
            for note in &p.notes {
                println!("Note: {note}");
            }
        }
        ClientCommand::Rm { name } => {
            let before = list.len();
            list.retain(|x| x.name != name);
            if list.len() == before {
                bail!("there is no proxy client {name}");
            }
            set(c, SetConfigParams { proxy_clients: Some(list), ..Default::default() }).await?;
            println!("Removed proxy client {name}. Its port is closed; its log entries stay.");
        }
        ClientCommand::Setup { name, new_code } => {
            if new_code {
                let _: NewSetupCodeResult = c.call("new_setup_code", NewSetupCodeParams { client: name.clone() }).await?;
            }
            let p = get_proxy(c, Some(name.clone())).await?;
            let Some(info) = &p.lan else { bail!("proxy client {name} is not a phone client; add one: {cli} proxy client add <name> --lan") };
            print!("{}", describe_lan(info));
        }
        ClientCommand::Allow { name, address } => {
            let r: SetPhoneDeviceResult =
                c.call("set_phone_device", SetPhoneDeviceParams { client: name.clone(), address: address.clone(), allow: true }).await?;
            println!("Allowed {address} on {name}. Allowed devices: {}", r.devices.join(", "));
        }
        ClientCommand::Pause { name } | ClientCommand::Resume { name } => {
            let paused = pause;
            let Some(client) = list.iter_mut().find(|x| x.name == name) else { bail!("there is no proxy client {name}") };
            if !client.lan {
                bail!("proxy client {name} is not a phone client; add one: {cli} proxy client add <name> --lan");
            }
            client.paused = paused;
            set(c, SetConfigParams { proxy_clients: Some(list), ..Default::default() }).await?;
            if paused {
                println!("Paused {name}: a phone set to Automatic goes direct (it may take until it rejoins the Wi-Fi). Manual settings do not follow.");
            } else {
                println!("Resumed {name}: a phone set to Automatic goes through the proxy again.");
            }
        }
        ClientCommand::Deny { name, address } => {
            let r: SetPhoneDeviceResult =
                c.call("set_phone_device", SetPhoneDeviceParams { client: name.clone(), address: address.clone(), allow: false }).await?;
            let left = if r.devices.is_empty() { "none".to_string() } else { r.devices.join(", ") };
            println!("Denied {address} on {name}. Allowed devices: {left}");
        }
    }
    Ok(())
}

/// The first free port after the proxy port and the clients' ports. A port
/// in the config is skipped even when it is not bound (the proxy is off).
async fn next_port(c: &mut Client, config: &Config) -> anyhow::Result<u16> {
    if config.proxy_port == 0 {
        // Tests: the main port is "any free port", so is the client's.
        return Ok(0);
    }
    let used: Vec<u16> =
        [config.http_port, config.https_port, config.proxy_port].into_iter().chain(config.proxy_clients.iter().map(|x| x.port)).collect();
    let mut near = config.proxy_clients.iter().map(|x| x.port).chain([config.proxy_port]).max().unwrap_or(config.proxy_port);
    for _ in 0..50 {
        near = near.checked_add(1).context("no free port above the proxy port")?;
        let free: FindFreePortResult = c.call("find_free_port", FindFreePortParams { near: Some(near) }).await?;
        if !used.contains(&free.port) {
            return Ok(free.port);
        }
        near = free.port;
    }
    bail!("no free port found; give one with --port")
}

/// The proxy clients as text: name, port, state.
pub fn describe_clients(p: &GetProxyResult, config: &Config, instance: &Instance) -> String {
    let cli = instance.cli();
    if p.clients.is_empty() {
        return format!(
            "No proxy clients. Every program uses the main port {}.\nAdd one: {cli} proxy client add <name> (a port of its own, and its own page in the log viewer)\n",
            p.port
        );
    }
    let mut out = format!("{:<20} {}\n", DEFAULT_CLIENT, describe_port(p.enabled, &p.bound, p.port, &p.errors));
    for client in &p.clients {
        let port = client.port.unwrap_or(client.configured);
        let phone = if client.lan { "  (phone, on the LAN; devices allowed by address)" } else { "" };
        out.push_str(&format!("{:<20} {}{phone}\n", client.name, describe_port(config.proxy_enabled, &client.bound, port, &client.errors)));
    }
    out.push_str(&format!("\nA shell:   eval \"$({cli} proxy env --client <name>)\" && claude\nChrome:    {cli} proxy chrome --client <name>\n"));
    out
}

/// What a phone needs, as text (ADR 10).
pub fn describe_lan(info: &LanProxyInfo) -> String {
    let mut out = format!(
        "Server     {}\nPort       {}\n",
        info.address.as_deref().unwrap_or("(unknown: no IPv4 address on this network)"),
        info.port
    );
    if let Some(url) = &info.setup_url {
        out.push_str(&format!("Setup URL  {url}  (opening it allows the device)\n"));
    }
    if let Some(url) = &info.pac_url {
        out.push_str(&format!("PAC URL    {url}  (Configure Proxy → Automatic)\n"));
    }
    let devices = if info.devices.is_empty() { "none".to_string() } else { info.devices.join(", ") };
    out.push_str(&format!("Allowed    {devices}\n"));
    for d in &info.pending {
        out.push_str(&format!("Waiting    {} (asked for {}): allow it with proxy client allow\n", d.address, d.host));
    }
    for problem in &info.problems {
        out.push_str(&format!("Problem: {problem}\n"));
    }
    out
}

fn describe_port(enabled: bool, bound: &[String], port: u16, errors: &[String]) -> String {
    match (enabled, bound.is_empty()) {
        (true, false) => format!("http://127.0.0.1:{port}"),
        (true, true) => format!("port {port}, not listening: {}", errors.join("; ")),
        (false, _) => format!("port {port} (the proxy is off)"),
    }
}

/// `export NAME='value'` lines in a fixed order.
pub fn env_lines(p: &GetProxyResult) -> Vec<String> {
    let mut names: Vec<&String> = p.env.keys().collect();
    names.sort_by_key(|n| ENV_ORDER.iter().position(|o| o == n).unwrap_or(ENV_ORDER.len()));
    names.into_iter().map(|n| format!("export {n}={}", quote(&p.env[n]))).collect()
}

/// `open -na "Google Chrome" --args …`: a new Chrome instance, so the flags
/// are read even when the user's Chrome is running.
pub fn chrome_command(chrome_args: &[String]) -> Vec<String> {
    let mut command: Vec<String> = ["open", "-na", "Google Chrome", "--args"].iter().map(|s| s.to_string()).collect();
    command.extend(chrome_args.iter().cloned());
    command
}

fn shell_line(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

/// Quote for a POSIX shell when needed.
fn quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./:=,@%+".contains(&b)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The `get_proxy` reply as text.
pub fn describe(p: &GetProxyResult, instance: &Instance) -> String {
    let cli = instance.cli();
    let mut out = String::new();
    if p.enabled && !p.bound.is_empty() {
        out.push_str(&format!("Proxy          on: {} ({})\n", p.url, p.bound.join(", ")));
    } else if p.enabled {
        out.push_str(&format!("Proxy          on, but not listening on port {}\n", p.port));
    } else {
        out.push_str(&format!("Proxy          off (port {}). Turn it on: {cli} proxy on\n", p.port));
    }
    if p.inspect_set.is_empty() {
        out.push_str("Inspected      none: every CONNECT is a tunnel\n");
    } else {
        out.push_str(&format!("Inspected      {}\n", p.inspect_set.join(", ")));
    }
    match &p.inspect_ca {
        None => out.push_str("Inspection CA  not made yet (made by the first inspect add)\n"),
        Some(ca) if ca.state == CaState::Broken => {
            out.push_str(&format!("Inspection CA  broken: {}\n", ca.problem.as_deref().unwrap_or("?")))
        }
        Some(ca) => {
            let trusted = match ca.trusted {
                Some(true) => "trusted".to_string(),
                Some(false) => format!("NOT trusted: run `{cli} proxy trust`"),
                None => "trust unknown".to_string(),
            };
            out.push_str(&format!("Inspection CA  {} ({trusted})\n", ca.common_name.as_deref().unwrap_or("?")));
        }
    }
    if let Some(log) = &p.log {
        out.push_str(&format!("Log            {} ({})\n", if log.enabled { "on" } else { "off" }, log.url));
    }
    if !p.clients.is_empty() {
        let names: Vec<String> = p
            .clients
            .iter()
            .map(|c| match c.port {
                Some(port) => format!("{} ({port})", c.name),
                None => format!("{} ({}, not listening)", c.name, c.configured),
            })
            .collect();
        out.push_str(&format!("Clients        {}\n", names.join(", ")));
    }
    out.push_str(&format!("\nA shell:   eval \"$({cli} proxy env)\" && claude\nChrome:    {cli} proxy chrome\n"));
    if let Some(line) = log_line(p, instance) {
        out.push_str(&line);
        out.push('\n');
    }
    if !p.notes.is_empty() {
        out.push('\n');
        for note in &p.notes {
            out.push_str(&format!("Note: {note}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> GetProxyResult {
        let text = include_str!("../../../api/examples/get_proxy.reply.json");
        let reply: serde_json::Value = serde_json::from_str(text).unwrap();
        serde_json::from_value(reply["result"].clone()).unwrap()
    }

    #[test]
    fn env_lines_come_in_a_fixed_order_and_are_quoted() {
        let lines = env_lines(&example());
        assert!(lines[0].starts_with("export HTTPS_PROXY=http://127.0.0.1:8877"), "{lines:?}");
        assert!(lines[1].starts_with("export HTTP_PROXY="));
        assert!(lines[2].starts_with("export NO_PROXY="));
        assert!(lines[3].starts_with("export NODE_USE_ENV_PROXY=1"));
        // The path has spaces (Application Support).
        assert!(lines[4].starts_with("export NODE_EXTRA_CA_CERTS='/Users/"), "{lines:?}");
        // ADR 08: the CA bundle for Python and curl, then the lowercase names.
        assert!(lines[5].starts_with("export SSL_CERT_FILE='/Users/") && lines[5].ends_with("bundle.pem'"), "{lines:?}");
        assert!(lines[6].starts_with("export REQUESTS_CA_BUNDLE="));
        assert!(lines[7].starts_with("export CURL_CA_BUNDLE="));
        assert_eq!(lines[8], "export https_proxy=http://127.0.0.1:8877");
        assert_eq!(lines[9], "export http_proxy=http://127.0.0.1:8877");
        assert!(lines[10].starts_with("export no_proxy="));
        assert_eq!(lines.len(), 11);
    }

    // ADR 08, T10: the log state names the folder, the viewer and the limits.
    #[test]
    fn the_log_state_names_the_folder_and_the_viewer() {
        let p = example();
        let text = describe_log(&p, &Instance::release());
        assert!(text.contains("Log        on, writes every proxied request to /Users/me/Library/Logs/LocalRouter/proxy"), "{text}");
        assert!(text.contains("Viewer     http://proxy.localhost"), "{text}");
        assert!(text.contains("20 MB or 5000 requests per file; the 5 newest files are kept"), "{text}");
        assert!(text.contains("cookies and API keys too"), "{text}");
        let line = log_line(&p, &Instance::release()).unwrap();
        assert!(line.starts_with("Log: on, writes every request to /Users/me/Library/Logs/LocalRouter/proxy"), "{line}");
    }

    // I18: the command is `open` plus exactly the daemon's chrome_args.
    #[test]
    fn the_chrome_command_uses_the_daemon_arguments() {
        let p = example();
        let command = chrome_command(&p.chrome_args);
        assert_eq!(&command[..4], ["open", "-na", "Google Chrome", "--args"]);
        assert_eq!(&command[4..], p.chrome_args.as_slice());
        assert!(shell_line(&command).starts_with("open -na 'Google Chrome' --args --user-data-dir="));
    }

    #[test]
    fn quote_keeps_plain_words_and_quotes_the_rest() {
        assert_eq!(quote("http://127.0.0.1:8877"), "http://127.0.0.1:8877");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
    }
}
