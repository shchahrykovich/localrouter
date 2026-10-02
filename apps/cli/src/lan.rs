//! `localrouter lan …`: LAN access per network (ADR 08, change 4). Not an
//! MCP action: an agent must not open the user's dev servers to a network.

use anyhow::bail;
use clap::Subcommand;
use localrouter_core::api::{self, NetworkStatus, SetConfigParams, SetConfigResult, StatusResult};
use localrouter_core::config::{Config, LanNetwork};
use localrouter_core::instance::Instance;
use localrouter_core::paths::Paths;

use crate::client::Client;

#[derive(Subcommand)]
pub enum LanCommand {
    /// Allow LAN access on the network the Mac is on now.
    Allow {
        /// A name for it, for example Home.
        #[arg(long)]
        name: Option<String>,
    },
    /// Stop allowing LAN access on the network the Mac is on now.
    Deny,
    /// Remove an allowed network by its name or id.
    Forget { network: String },
}

pub async fn run(command: Option<LanCommand>, paths: &Paths, instance: &Instance) -> anyhow::Result<()> {
    let mut c = Client::connect(&paths.socket(), "cli").await?;
    let status: StatusResult = c.call("status", api::Empty {}).await?;
    let config: Config = c.call("get_config", api::Empty {}).await?;
    let mut list = config.lan_networks.clone();
    match command {
        None => {
            print!("{}", describe(&config, status.network.as_ref(), instance));
            return Ok(());
        }
        Some(LanCommand::Allow { name }) => {
            let Some(n) = status.network.as_ref() else {
                bail!("this network cannot be recognised (no router, or a VPN), so it cannot be allowed");
            };
            let name = name.unwrap_or_default();
            match list.iter_mut().find(|l| l.id == n.id) {
                Some(l) if !name.is_empty() => l.name = name,
                Some(_) => {}
                None => list.push(LanNetwork { id: n.id.clone(), name, router: n.router.clone() }),
            }
        }
        Some(LanCommand::Deny) => {
            let Some(n) = status.network.as_ref() else { bail!("this network cannot be recognised") };
            let before = list.len();
            list.retain(|l| l.id != n.id);
            if list.len() == before {
                println!("This network was not allowed.");
                return Ok(());
            }
        }
        Some(LanCommand::Forget { network }) => {
            let wanted = network.trim();
            let before = list.len();
            list.retain(|l| !(l.id.eq_ignore_ascii_case(wanted) || (!l.name.is_empty() && l.name == wanted)));
            if list.len() == before {
                bail!("no allowed network named {wanted}");
            }
        }
    }
    let r: SetConfigResult = c.call("set_config", SetConfigParams { lan_networks: Some(list), ..Default::default() }).await?;
    let status: StatusResult = c.call("status", api::Empty {}).await?;
    print!("{}", describe(&r.config, status.network.as_ref(), instance));
    Ok(())
}

fn label(n: &LanNetwork) -> String {
    let name = if n.name.is_empty() { "(no name)".to_string() } else { n.name.clone() };
    format!("{name}: router {} ({})", n.router, n.id)
}

/// The state as text: the main switch, this network, the list.
pub fn describe(config: &Config, here: Option<&NetworkStatus>, instance: &Instance) -> String {
    let mut out = String::new();
    if config.allow_lan {
        out.push_str("LAN access     on, for the networks below only\n");
    } else {
        out.push_str("LAN access     off: only this Mac reaches ports 80 and 443 (turn it on in Settings > Routing)\n");
    }
    match here {
        None => out.push_str("This network   cannot be recognised (no router, or a VPN): never allowed\n"),
        Some(n) => {
            let name = if n.name.is_empty() { String::new() } else { format!("{}, ", n.name) };
            let state = if n.lan_allowed {
                "allowed"
            } else if config.lan_networks.iter().any(|l| l.id == n.id) {
                "in the list, but LAN access is off"
            } else {
                "not allowed"
            };
            out.push_str(&format!("This network   {name}router {} on {} ({}): {state}\n", n.router, n.interface, n.id));
        }
    }
    if config.lan_networks.is_empty() {
        out.push_str("Allowed        none\n");
    } else {
        for (i, n) in config.lan_networks.iter().enumerate() {
            let head = if i == 0 { "Allowed" } else { "" };
            out.push_str(&format!("{head:<15}{}\n", label(n)));
        }
    }
    out.push_str(&format!(
        "\nOn other networks only this Mac can reach ports 80 and 443.\nAllow this one: {cli} lan allow --name Home\n",
        cli = instance.cli()
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_names_the_network_and_its_state() {
        let config = Config {
            allow_lan: true,
            lan_networks: vec![LanNetwork { id: "mac:18:35:d1:15:d1:a8".into(), name: "Home".into(), router: "192.168.0.1".into() }],
            ..Default::default()
        };
        let here = NetworkStatus {
            id: "mac:18:35:d1:15:d1:a8".into(),
            name: "Home".into(),
            router: "192.168.0.1".into(),
            interface: "en0".into(),
            lan_allowed: true,
        };
        let text = describe(&config, Some(&here), &Instance::release());
        assert!(text.contains("This network   Home, router 192.168.0.1 on en0 (mac:18:35:d1:15:d1:a8): allowed"), "{text}");
        assert!(text.contains("Allowed        Home: router 192.168.0.1"), "{text}");
        let text = describe(&config, None, &Instance::release());
        assert!(text.contains("cannot be recognised"), "{text}");
    }
}
