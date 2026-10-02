//! The texts a coding agent reads: the help page, the Claude Code note and the
//! MCP instructions, rendered for the release and for a suffixed instance.
//!
//! ADR 03, T11, I32: they describe path routes and tell the agent to ask
//! before it changes the production build.
//! ADR 04, T5, I4: each names its own instance's CLI, help URL and ports.
//!
//! This checks that the words are there, not that they are well written; a
//! real agent session checks that (manual tests).

use localrouter_core::help::{instance_note, mcp_instructions, render, render_note};
use localrouter_core::instance::Instance;

fn dev() -> Instance {
    Instance::new("-dev").unwrap()
}

/// Every agent text of an instance, with the ports the daemon has bound.
fn texts(instance: &Instance, http: u16, https: u16) -> Vec<(&'static str, String)> {
    vec![
        ("help page", render(instance, &[], Some(http), Some(https), None)),
        ("note", render_note(instance)),
        ("MCP instructions", mcp_instructions(instance)),
    ]
}

#[test]
fn every_agent_text_describes_path_routes_and_asks_before_changing_the_build() {
    for instance in [Instance::release(), dev()] {
        for (name, text) in texts(&instance, 80, 443) {
            assert!(text.contains("path"), "{name} does not mention path routes");
            assert!(text.contains("production build"), "{name} lacks the ask-first rule");
            assert!(text.contains("basePath"), "{name} does not name basePath");
        }
    }
    assert!(render_note(&Instance::release()).contains("--path"), "the note lacks the --path command");
}

#[test]
fn every_agent_text_describes_folder_routes() {
    for instance in [Instance::release(), dev()] {
        for (name, text) in texts(&instance, 80, 443) {
            assert!(text.contains("folder"), "{name} does not mention folder routes");
            assert!(text.contains("absolute path"), "{name} does not say MCP needs an absolute path");
        }
        assert!(render_note(&instance).contains("--folder"), "the note lacks the --folder option");
        assert!(render(&instance, &[], Some(80), Some(443), None).contains("--folder"), "the help page lacks --folder");
    }
}

#[test]
fn the_help_page_names_both_patterns_and_the_which_command() {
    let page = render(&Instance::release(), &[], Some(80), Some(443), None);
    for word in ["--path", "--strip-path", "assetPrefix", "localrouter which", "Step 4b"] {
        assert!(page.contains(word), "help page lacks {word}");
    }
}

#[test]
fn no_placeholder_is_left() {
    for instance in [Instance::release(), dev()] {
        for (name, text) in texts(&instance, 7080, 7443) {
            assert!(!text.contains("{{"), "{name} for {instance} has a placeholder left: {text}");
        }
    }
}

/// The release texts keep the names they always had.
#[test]
fn the_release_texts_name_the_release() {
    let [(_, page), (_, note), (_, mcp)] = <[_; 3]>::try_from(texts(&Instance::release(), 80, 443)).unwrap();
    assert!(page.contains("claude mcp add localrouter -- ~/.local/bin/localrouter mcp"));
    assert!(page.contains("curl -s http://router.localhost`"));
    assert!(page.contains("https://shop.localhost` goes to"));
    assert!(page.starts_with("# LocalRouter\n"));
    assert!(note.contains("localrouter guide"));
    assert!(note.contains("curl -s http://router.localhost`"), "the note keeps a fallback when the CLI is missing (G5)");
    assert!(mcp.contains("`localrouter guide`"));
    for (name, text) in [("page", &page), ("note", &note), ("mcp", &mcp)] {
        assert!(!text.contains("localrouter-"), "{name} names a suffixed instance");
        assert!(!text.contains("development build"), "{name} has the instance note");
    }
}

/// What a dev text must not contain, outside the parts that name the release
/// on purpose: the instance note and Step 7 (project files are shared with
/// people who run the release).
fn release_names_in(text: &str, https: bool) -> Vec<String> {
    let mut text = text.replace(&instance_note(&dev(), Some(7443)), "");
    if let Some(step7) = text.find("## Step 7") {
        text.truncate(step7);
    }
    let text = text.replace(&dev().daemon_label(), "");
    let mut found = vec![];
    for (i, _) in text.match_indices("localrouter") {
        if !text[i..].starts_with("localrouter-dev") {
            found.push(text[i..].chars().take(40).collect());
        }
    }
    for (i, _) in text.match_indices("router.localhost") {
        let after = &text[i + "router.localhost".len()..];
        if !(after.starts_with(":7080") || after.starts_with(":7443")) {
            found.push(text[i..].chars().take(40).collect());
        }
    }
    if https {
        for (i, _) in text.match_indices("https://") {
            let url: String = text[i..].chars().take_while(|c| !c.is_whitespace() && *c != '`').collect();
            if url.contains(".localhost") && !url.contains(".localhost:7443") {
                found.push(url);
            }
        }
    }
    found
}

#[test]
fn every_dev_text_names_the_dev_instance() {
    for (name, text) in texts(&dev(), 7080, 7443) {
        // The MCP text names no port on purpose: the reply has the exact URL.
        let found = release_names_in(&text, name != "MCP instructions");
        assert!(found.is_empty(), "{name} names the release: {found:?}");
    }
}

#[test]
fn dev_texts_say_when_to_use_the_dev_instance() {
    for (name, text) in texts(&dev(), 7080, 7443) {
        assert!(text.contains("only when the user asks"), "{name} lacks the when-to-use sentence (G1)");
    }
    let note = render_note(&dev());
    assert!(note.contains("localrouter-dev guide"));
    assert!(note.contains("curl -s http://router.localhost:7080`"), "fallback with the default help URL (G5)");
    let page = render(&dev(), &[], Some(7080), Some(7443), None);
    assert!(page.contains("Project files are shared with people who run the release"), "Step 7 says why it keeps the release names");
    assert!(page.contains("launchctl kickstart -k gui/$(id -u)/dev.localrouter.app-dev.daemon"));
    assert!(page.contains("Browsers share cookies between ports"), "the dev page warns about shared cookies");
    assert!(page.starts_with("# LocalRouter-dev\n"), "the title names the instance");
    assert!(render_note(&dev()).starts_with("# LocalRouter-dev\n"));
    let release = render(&Instance::release(), &[], Some(80), Some(443), None);
    assert!(!release.contains("Browsers share cookies between ports"));
}

/// The help page follows the ports the daemon bound, not only the defaults.
#[test]
fn the_dev_help_page_uses_the_bound_ports() {
    let page = render(&dev(), &[], Some(7081), Some(7444), None);
    assert!(page.contains("curl -s http://router.localhost:7081`"));
    assert!(page.contains("https://shop.localhost:7444` goes to"));
}

/// ADR 06, T12, I15: every agent text describes the proxy, the get_proxy
/// tool, the env command, the trust step, that a running program keeps its
/// proxy, and the rule never to write proxy settings into project files.
#[test]
fn every_agent_text_describes_the_proxy_and_its_rules() {
    for instance in [Instance::release(), dev()] {
        let cli = instance.cli();
        for (name, text) in texts(&instance, 80, 443) {
            // One line, lower case: the templates wrap sentences.
            let lower = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
            assert!(text.contains("get_proxy"), "{name} does not name get_proxy");
            assert!(text.contains(&format!("{cli} proxy env")), "{name} lacks the env command");
            assert!(text.contains(&format!("{cli} proxy trust")), "{name} lacks the trust step");
            assert!(lower.contains("cannot change the proxy of a program that is already running"), "{name}");
            assert!(lower.contains("never write proxy settings into project files"), "{name}");
        }
    }
    let dev_page = render(&dev(), &[], Some(7080), Some(7443), None);
    assert!(dev_page.contains("127.0.0.1:7877"), "the dev page names the dev proxy port");
    assert!(render_note(&Instance::release()).contains("127.0.0.1:8877"));
}

/// ADR 07, T17, I14: every agent text describes both kinds of script, the
/// reference page, check_only, owner_pid, response_body and streaming,
/// that scripts see every header as it is, and not committing captures.
#[test]
fn every_agent_text_describes_scripts_and_their_rules() {
    for instance in [Instance::release(), dev()] {
        let cli = instance.cli();
        for (name, text) in texts(&instance, 80, 443) {
            let lower = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
            for word in ["intercept", "log script", "check_only", "owner_pid", "response_body", "on_event", "output_dir"] {
                assert!(lower.contains(word), "{name} lacks {word}");
            }
            assert!(lower.contains("stops streaming") || lower.contains("streaming stops"), "{name}: response_body and streaming");
            assert!(lower.contains("every header as it is"), "{name}: headers");
            assert!(!text.contains("[redacted]"), "{name}: nothing is redacted");
            assert!(lower.contains("do not commit captures"), "{name}: captures");
            assert!(text.contains(&format!("{cli} rules api")), "{name} lacks the rules api command");
            assert!(text.contains("/scripts"), "{name} lacks the reference page");
        }
    }
    let page = render(&Instance::release(), &[], Some(80), Some(443), None);
    for word in ["set_script_rule", "remove_script_rule", "curl -s http://router.localhost/scripts"] {
        assert!(page.contains(word), "help page lacks {word}");
    }
}

/// ADR 07, T17: the reference names every field of `req`, `res` and `ex`
/// and every module. The field names come from the tables scripts get, so a
/// new field without a line in the reference fails here.
#[test]
fn the_script_reference_names_every_field_and_module() {
    use localrouter_core::scripts::lua_api::{self, Exchange, StateCtx};
    let reference = localrouter_core::help::render_scripts(&Instance::release(), Some(80), Some(443));
    let lua = mlua::Lua::new();
    lua_api::install(&lua, &StateCtx { rule_id: "t".into(), log: true, capture: None, lines: Default::default() }).unwrap();
    let ex = Exchange { error: Some("e".into()), answered_by: Some("a".into()), ..Default::default() };
    let mut fields: Vec<String> = vec![];
    let ex_table = lua_api::exchange_table(&lua, &ex).unwrap();
    for pair in ex_table.pairs::<String, mlua::Value>() {
        fields.push(pair.unwrap().0);
    }
    for key in ["request", "response"] {
        let t: mlua::Table = ex_table.get(key).unwrap();
        // Fields that are nil in this example still have a line in the table builders.
        for pair in t.pairs::<String, mlua::Value>() {
            fields.push(pair.unwrap().0);
        }
    }
    let ev = lua_api::event_fields(&lua, &Default::default(), Some(1)).unwrap();
    for pair in ev.pairs::<String, mlua::Value>() {
        fields.push(pair.unwrap().0);
    }
    for f in ["route", "content_type", "body", "body_skipped", "body_file", "body_encoding", "range", "charset", "event", "id", "retry"] {
        fields.push(f.to_string());
    }
    for f in &fields {
        assert!(reference.contains(&format!("`{f}`")), "the reference lacks the field {f}");
    }
    for module in ["json", "sse", "base64", "url", "multipart", "log", "capture"] {
        assert!(reference.contains(&format!("`{module}`")), "the reference lacks the module {module}");
        let present: bool = lua.load(format!("return {module} ~= nil")).eval().unwrap();
        assert!(present, "{module} is in the reference but not in the sandbox");
    }
    assert!(!reference.contains("{{"), "a placeholder is left");
    let dev = localrouter_core::help::render_scripts(&dev(), Some(7080), Some(7443));
    assert!(dev.contains("localrouter-dev rules api") && dev.contains("router.localhost:7080/scripts"));
}

/// ADR 08, T12: every agent text names the proxy log: its folder and viewer
/// for this instance, that cookies and API keys are written as they are,
/// "do not read a whole file", the jq
/// recipe, and the CA bundle names.
#[test]
fn every_agent_text_describes_the_proxy_log() {
    for (instance, http, url, folder) in [
        (Instance::release(), 80, "http://proxy.localhost", "~/Library/Logs/LocalRouter/proxy"),
        (dev(), 7080, "http://proxy.localhost:7080", "~/Library/Logs/LocalRouter-dev/proxy"),
    ] {
        let cli = instance.cli();
        for (name, text) in texts(&instance, http, http + 363) {
            let lower = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
            assert!(text.contains(folder), "{name} lacks the folder {folder}");
            assert!(text.contains(url), "{name} lacks the viewer {url}");
            assert!(lower.contains("cookies and api keys"), "{name}: headers are written as they are");
            assert!(lower.contains("do not read a whole file"), "{name}");
            assert!(text.contains(&format!("{cli} proxy log path")), "{name} lacks the folder command");
            assert!(text.contains("jq"), "{name} lacks jq");
            assert!(text.contains("SSL_CERT_FILE") && text.contains("CURL_CA_BUNDLE"), "{name} lacks the CA bundle");
            assert!(!text.contains("{{"), "{name}: a placeholder is left");
        }
    }
    // The note is made when the bundle is built: it names the default ports.
    assert!(render_note(&dev()).contains("http://proxy.localhost:7080"));
    assert!(!render_note(&Instance::release()).contains("http://proxy.localhost:"), "release viewer without a port");
}

