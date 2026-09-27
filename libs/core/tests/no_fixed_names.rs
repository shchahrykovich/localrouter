//! ADR 04, T6, I4: no Rust source writes a fixed CLI name, help URL or note
//! name into a text. Those come from the instance, so a dev build never tells
//! a person or an agent to use the release. The scan covers every source file,
//! so a new text is checked without a new test case.
//!
//! Skipped: comments, except `///` in the CLI's main.rs, which clap shows as
//! help; test modules (after `#[cfg(test)]`); and the templates, which the
//! render test checks.

use std::path::{Path, PathBuf};

const WORDS: [&str; 5] = ["localrouter ", "localrouterd", "router.localhost", "LocalRouter.md", "LocalRouter.app"];

/// Lines that may name the release on purpose, with the reason.
const ALLOWED: [(&str, &str); 3] = [
    // An internal function's doc comment, not shown to anyone.
    ("apps/cli/src/main.rs", "/// The `localrouter which` report"),
    // The rule itself: the release's program names.
    ("libs/core/src/instance.rs", "const DAEMON_PROGRAM: &str = \"localrouterd\";"),
    ("libs/core/src/instance.rs", "#[error(\"cannot tell the instance from the program name"),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = vec![];
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn no_source_writes_a_fixed_instance_name() {
    let mut found = vec![];
    let mut scanned = 0;
    for dir in ["apps/cli/src", "apps/daemon/src", "libs/core/src"] {
        for file in rust_files(&root().join(dir)) {
            scanned += 1;
            let rel = file.strip_prefix(root()).unwrap().to_string_lossy().replace("../", "");
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("#[cfg(test)]") {
                    break;
                }
                let code = line.trim_start();
                // Doc comments are help text only where clap reads them.
                let help_text = rel.ends_with("apps/cli/src/main.rs") && code.starts_with("///");
                if code.starts_with("//") && !help_text {
                    continue;
                }
                if !WORDS.iter().any(|w| line.contains(w)) {
                    continue;
                }
                if ALLOWED.iter().any(|(f, start)| rel.ends_with(f) && code.starts_with(start)) {
                    continue;
                }
                found.push(format!("{rel}:{}: {code}", n + 1));
            }
        }
    }
    assert!(scanned > 20, "the scan found only {scanned} files");
    assert!(found.is_empty(), "fixed names; use the instance instead:\n{}", found.join("\n"));
}
