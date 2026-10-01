//! `localrouter proxy …`: the forward proxy (ADR 06, change 3). Every value
//! comes from the daemon's `get_proxy`, so the CLI, the MCP tool and the app
//! give the same settings (I18).

use anyhow::{Context, bail};
use clap::Subcommand;
use localrouter_core::api::{self, CaState, GetProxyResult, ResetCaResult, SetConfigParams, SetConfigResult};
use localrouter_core::config::Config;
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
    Env,
    /// Open a separate Chrome window that uses the proxy (its own profile).
    Chrome {
        /// Print the command instead of running it.
        #[arg(long)]
        print: bool,
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
}

#[derive(Subcommand)]
pub enum InspectCommand {
    /// Inspect a host: api.example.com, or *.example.com for every name under it.
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
const ENV_ORDER: [&str; 5] = ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "NODE_USE_ENV_PROXY", "NODE_EXTRA_CA_CERTS"];

pub async fn run(command: Option<ProxyCommand>, paths: &Paths, instance: &Instance) -> anyhow::Result<()> {
    let cli = instance.cli();
    // `ca-path` works without a daemon, like the local CA's.
    if let Some(ProxyCommand::CaPath) = command {
        println!("{}", paths.inspect_ca_pem().display());
        return Ok(());
    }
    let mut c = Client::connect(&paths.socket(), "cli").await?;
    match command {
        None => {
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            print!("{}", describe(&p, instance));
        }
        Some(ProxyCommand::On) => {
            set(&mut c, SetConfigParams { proxy_enabled: Some(true), ..Default::default() }).await?;
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            println!("The proxy is on: {}", p.url);
            println!("Use it: eval \"$({cli} proxy env)\" && claude, or {cli} proxy chrome");
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
        Some(ProxyCommand::Env) => {
            let p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            for line in env_lines(&p) {
                println!("{line}");
            }
            // To stderr, so `eval "$(… proxy env)"` reads only the exports.
            for note in &p.notes {
                eprintln!("# {note}");
            }
        }
        Some(ProxyCommand::Chrome { print }) => {
            let mut p: GetProxyResult = c.call("get_proxy", api::Empty {}).await?;
            if print {
                println!("{}", shell_line(&chrome_command(&p.chrome_args)));
                return Ok(());
            }
            if !p.enabled {
                set(&mut c, SetConfigParams { proxy_enabled: Some(true), ..Default::default() }).await?;
                p = c.call("get_proxy", api::Empty {}).await?;
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

async fn set(c: &mut Client, params: SetConfigParams) -> anyhow::Result<SetConfigResult> {
    Ok(c.call("set_config", params).await?)
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
    out.push_str(&format!("\nA shell:   eval \"$({cli} proxy env)\" && claude\nChrome:    {cli} proxy chrome\n"));
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
