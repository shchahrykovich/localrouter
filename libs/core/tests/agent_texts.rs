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
}

/// The help page follows the ports the daemon bound, not only the defaults.
#[test]
fn the_dev_help_page_uses_the_bound_ports() {
    let page = render(&dev(), &[], Some(7081), Some(7444), None);
    assert!(page.contains("curl -s http://router.localhost:7081`"));
    assert!(page.contains("https://shop.localhost:7444` goes to"));
}
