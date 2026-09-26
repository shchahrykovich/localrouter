//! ADR 03, T11, I32: the texts a coding agent reads all describe path routes,
//! and all tell it to ask before it changes the production build. This checks
//! that the words are there, not that they are well written; a real agent
//! session checks that (manual test M3).

const HELP: &str = include_str!("../src/help.md");
const NOTE: &str = include_str!("../../../scripts/LocalRouter.md");
const MCP: &str = include_str!("../../../apps/cli/src/mcp.rs");

/// The MCP server instructions: the `INSTRUCTIONS` constant in mcp.rs.
fn mcp_instructions() -> &'static str {
    let start = MCP.find("const INSTRUCTIONS").expect("INSTRUCTIONS in mcp.rs");
    let end = MCP[start..].find("\";").expect("end of INSTRUCTIONS") + start;
    &MCP[start..end]
}

#[test]
fn every_agent_text_describes_path_routes_and_asks_before_changing_the_build() {
    for (name, text) in [("help.md", HELP), ("LocalRouter.md", NOTE), ("MCP instructions", mcp_instructions())] {
        assert!(text.contains("path"), "{name} does not mention path routes");
        assert!(text.contains("production build"), "{name} lacks the ask-first rule");
        assert!(text.contains("basePath"), "{name} does not name basePath");
    }
    assert!(NOTE.contains("--path"), "LocalRouter.md lacks the --path command");
}

#[test]
fn the_help_page_names_both_patterns_and_the_which_command() {
    for word in ["--path", "--strip-path", "assetPrefix", "localrouter which", "Step 4b"] {
        assert!(HELP.contains(word), "help.md lacks {word}");
    }
}
