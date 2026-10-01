//! `localrouter rules …`: script rules (ADR 07, change 5). Lua scripts that
//! change (intercept) or record (log) the HTTP traffic of routes and of the
//! forward proxy.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Subcommand;
use localrouter_core::api::{
    self, IdParams, ListScriptRulesResult, RemoveScriptRuleResult, ScriptRuleView, SetScriptRuleParams, SetScriptRuleResult,
};
use localrouter_core::help;
use localrouter_core::instance::Instance;
use localrouter_core::paths::Paths;
use localrouter_core::scripts::rules::{OnError, ScriptRule};

use crate::client::Client;

#[derive(Subcommand)]
pub enum RulesCommand {
    /// Set or replace a rule: run a .lua script on the traffic of a host.
    Add {
        /// The rule's name: a-z, 0-9 and '-'.
        id: String,
        /// api.example.com, *.example.com, or a route such as shop.localhost.
        #[arg(long)]
        host: String,
        /// The .lua file. A relative path is made absolute.
        #[arg(long)]
        script: PathBuf,
        /// Only this path prefix: /v1 matches /v1 and /v1/..., never /v1x.
        #[arg(long)]
        path: Option<String>,
        /// Only these methods (repeat the option for more).
        #[arg(long = "method")]
        methods: Vec<String>,
        /// Log rules: the folder the script writes. A relative path is made absolute.
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Intercept rules run from low to high.
        #[arg(long)]
        order: Option<i64>,
        /// Intercept rules: `pass` sends the traffic on unchanged when the script fails (default `fail`: a 502 page).
        #[arg(long, value_parser = ["fail", "pass"])]
        on_error: Option<String>,
        /// Log rules: bytes the rule may write (default 1 GiB).
        #[arg(long)]
        max_capture_bytes: Option<u64>,
        /// Let the script see API keys and cookies. Asks you to type yes; refused without a terminal.
        #[arg(long)]
        reveal_secrets: bool,
        /// Keep the rule after a daemon restart.
        #[arg(long, conflicts_with = "owner_pid")]
        persistent: bool,
        /// Remove the rule when this process exits.
        #[arg(long)]
        owner_pid: Option<u32>,
        /// Set the rule turned off.
        #[arg(long)]
        disabled: bool,
        /// Why the rule exists.
        #[arg(long, default_value = "")]
        note: String,
    },
    /// Check a script without setting a rule: syntax, kind and fields.
    Check {
        /// The .lua file.
        script: PathBuf,
        /// A log script's folder, to check that the daemon can write it.
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// The host the rule would have (default: a route name).
        #[arg(long)]
        host: Option<String>,
    },
    /// Remove a rule. Its capture files stay.
    Rm { id: String },
    /// Turn a rule on again (for example after it was turned off by 20 failures).
    Enable { id: String },
    /// Turn a rule off; it keeps its counters.
    Disable { id: String },
    /// Print the script reference: the Lua API, body classes, limits.
    Api,
}

pub async fn run(command: Option<RulesCommand>, json: bool, paths: &Paths, instance: &Instance) -> anyhow::Result<()> {
    if let Some(RulesCommand::Api) = command {
        print!("{}", help::render_scripts(instance, None, None));
        return Ok(());
    }
    let mut c = Client::connect(&paths.socket(), "cli").await?;
    match command {
        None => {
            let r: ListScriptRulesResult = c.call("list_script_rules", api::Empty {}).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
                return Ok(());
            }
            if r.rules.is_empty() {
                println!("No script rules. Set one with: {} rules add <id> --host <host> --script <file.lua>", instance.cli());
                println!("The script reference: {} rules api", instance.cli());
            }
            for v in &r.rules {
                print_rule(v);
            }
            Ok(())
        }
        Some(RulesCommand::Add {
            id,
            host,
            script,
            path,
            methods,
            output_dir,
            order,
            on_error,
            max_capture_bytes,
            reveal_secrets,
            persistent,
            owner_pid,
            disabled,
            note,
        }) => {
            let rule = ScriptRule {
                id,
                host: host.clone(),
                path,
                methods,
                script: absolute(&script)?,
                output_dir: output_dir.as_deref().map(absolute).transpose()?,
                order: order.unwrap_or(localrouter_core::scripts::rules::DEFAULT_ORDER),
                on_error: on_error.map(|e| if e == "pass" { OnError::Pass } else { OnError::Fail }),
                reveal_secrets,
                max_capture_bytes,
                enabled: !disabled,
                note,
                owner_pid,
                persistent,
            };
            if reveal_secrets {
                confirm_reveal(&host)?;
            }
            let r: SetScriptRuleResult = c.call("set_script_rule", SetScriptRuleParams { rule, check_only: false }).await?;
            print_set(&r);
            Ok(())
        }
        Some(RulesCommand::Check { script, output_dir, host }) => {
            let rule = ScriptRule {
                id: "check".into(),
                host: host.unwrap_or_else(|| "check.localhost".into()),
                path: None,
                methods: vec![],
                script: absolute(&script)?,
                output_dir: output_dir.as_deref().map(absolute).transpose()?,
                order: localrouter_core::scripts::rules::DEFAULT_ORDER,
                on_error: None,
                reveal_secrets: false,
                max_capture_bytes: None,
                enabled: true,
                note: String::new(),
                owner_pid: None,
                persistent: false,
            };
            let r: SetScriptRuleResult = c.call("set_script_rule", SetScriptRuleParams { rule, check_only: true }).await?;
            let kind = r.rule.kind.map_or("?".to_string(), |k| format!("{k:?}").to_lowercase());
            println!("{}: ok, a {kind} script", script.display());
            for n in &r.notes {
                println!("  {n}");
            }
            Ok(())
        }
        Some(RulesCommand::Rm { id }) => {
            let r: RemoveScriptRuleResult = c.call("remove_script_rule", IdParams { id: id.clone() }).await?;
            if !r.removed {
                bail!("no script rule named {id}");
            }
            println!("removed {id}");
            Ok(())
        }
        Some(RulesCommand::Enable { id }) => set_enabled(&mut c, &id, true).await,
        Some(RulesCommand::Disable { id }) => set_enabled(&mut c, &id, false).await,
        Some(RulesCommand::Api) => unreachable!("handled above"),
    }
}

/// Set the rule again with `enabled` changed; its other fields stay.
async fn set_enabled(c: &mut Client, id: &str, enabled: bool) -> anyhow::Result<()> {
    let list: ListScriptRulesResult = c.call("list_script_rules", api::Empty {}).await?;
    let Some(view) = list.rules.into_iter().find(|v| v.rule.id == id) else { bail!("no script rule named {id}") };
    let rule = ScriptRule { enabled, ..view.rule };
    let r: SetScriptRuleResult = c.call("set_script_rule", SetScriptRuleParams { rule, check_only: false }).await?;
    println!("{id} is {}", if r.rule.rule.enabled { "on" } else { "off" });
    for n in &r.notes {
        println!("  {n}");
    }
    Ok(())
}

/// `./cap.lua` → `/Users/me/shop/cap.lua`. The daemon has no working folder.
fn absolute(path: &Path) -> anyhow::Result<String> {
    let abs = std::path::absolute(path).with_context(|| format!("{}", path.display()))?;
    abs.to_str().map(str::to_string).with_context(|| format!("{} is not UTF-8", abs.display()))
}

/// `--reveal-secrets` needs a person at a terminal who types yes (ADR 07, I9).
/// This stops an agent that runs the command; it is not a security boundary.
fn confirm_reveal(host: &str) -> anyhow::Result<()> {
    if !std::io::stdin().is_terminal() {
        eprintln!("--reveal-secrets needs a terminal: a person must confirm it. Nothing was set.");
        std::process::exit(2);
    }
    eprint!("This rule will see API keys and cookies of {host}. Type yes: ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if answer.trim() != "yes" {
        eprintln!("Not confirmed. Nothing was set.");
        std::process::exit(2);
    }
    Ok(())
}

fn print_set(r: &SetScriptRuleResult) {
    let v = &r.rule;
    let kind = v.kind.map_or("?".to_string(), |k| format!("{k:?}").to_lowercase());
    let what = if r.replaced { "replaced" } else { "set" };
    println!("{what} {} ({kind}) on {}{}", v.rule.id, v.rule.host, v.rule.path.as_deref().unwrap_or(""));
    for n in &r.notes {
        println!("  {n}");
    }
}

fn print_rule(v: &ScriptRuleView) {
    let r = &v.rule;
    let state = if r.enabled { "on " } else { "off" };
    let kind = v.kind.map_or("?".to_string(), |k| format!("{k:?}").to_lowercase());
    let life = if r.owner_pid.is_some() {
        "  (owned)"
    } else if r.persistent {
        ""
    } else {
        "  (session)"
    };
    let methods = if r.methods.is_empty() { String::new() } else { format!(" {}", r.methods.join(",")) };
    let at = format!("{}{}{methods}", r.host, r.path.as_deref().unwrap_or(""));
    println!("{state}  {kind:<9} {:<20} {at}{life}", r.id);
    let mut counts = format!("matched {}, errors {}", v.matched, v.errors);
    if v.answered > 0 {
        counts.push_str(&format!(", answered {}", v.answered));
    }
    if v.dropped > 0 {
        counts.push_str(&format!(", dropped {}", v.dropped));
    }
    if r.output_dir.is_some() {
        counts.push_str(&format!(", written {} B", v.bytes_written));
    }
    println!("      {counts}");
    println!("      script {}", r.script);
    if let Some(dir) = &r.output_dir {
        println!("      output {dir}");
    }
    if r.reveal_secrets {
        println!("      sees secret headers");
    }
    if let Some(e) = &v.last_error {
        println!("      last error: {}", e.message);
    }
    if !r.note.is_empty() {
        println!("      {}", r.note);
    }
}
