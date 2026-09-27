# 9. Tasks

| # | Task | Depends on |
|---|---|---|
| 1 | Rust `Instance`, the names table, folders | — |
| 2 | Daemon: instance at start, config defaults, CA name, `set_config` from the file | 1 |
| 3 | Texts: templates, 404 link, CLI `guide` and `status`, source scan | 1 |
| 4 | MCP: server name and instructions | 3 |
| 5 | Swift `Instance`, bundle layout, paths, installers, uninstall, updater gate | 1 (the table) |
| 6 | App UI: Daemon section, names in header and tooltip, texts | 5 |
| 7 | Scripts: `--suffix`, templates, `install.sh` default, `publish.sh` guard | 1, 5 |
| 8 | End-to-end journey E1c and manual tests M1 to M5 | 2, 3, 4, 6, 7 |
| 9 | Docs, release, M6, status to as-built | 8 |

Tasks 2, 3 and 5 can run in parallel after task 1.

## 1. Rust Instance, the names table, folders

**Deps:** none. **Tests:** T1, T2.

Add `libs/core/src/instance.rs`: the suffix rule, `Instance::from_exe_name`,
and every derived name from [01](01-instance-suffix.md) (bundle id, label,
program names, folders, links, MCP name, CA name prefix, default ports, help URL
and URL with a port). Add `api/instance-names.json` with the rows from the test
plan. Change `libs/core/src/paths.rs` to take the instance; `LOCALROUTER_HOME`
keeps winning. Done when T1 and T2 pass and every existing test still passes
with the empty suffix.

## 2. Daemon

**Deps:** 1. **Tests:** T3, T4, T13.

`apps/daemon/src/main.rs` builds the instance from its resolved executable name
and stops with a clear error on a bad suffix before it opens any folder (I3).
`libs/core/src/config.rs` gets `Config::defaults_for(&Instance)`; the daemon
uses it for a missing file, a file that does not parse and each missing field
(I9). `tls.rs` puts the app name in a new CA's common name. `set_config` in
`daemon.rs` reads `config.json` under the write lock, changes only the given
fields, writes, and replies `restart_needed` when the file's ports differ from
the bound ports (I8). Done when T3, T4 and T13 pass.

## 3. Texts

**Deps:** 1. **Tests:** T5, T6, T7.

Turn `help.md`, the 404 page (`proxy.rs`), the CLI texts (`main.rs`) and
`scripts/LocalRouter.md` into templates with the placeholders from
[03](03-texts.md). The 404 page links on the request's port. Add `<cli> guide`
(renders `help.md` with `status` and `list_routes`) and the instance and folder
lines in `status`. The suffixed note gets the "when to use it" sentence (G1).
Add `libs/core/tests/no_fixed_names.rs` with its allow list, and extend
`agent_texts.rs` (I4). Done when T5, T6 and T7 pass and ADR 03's text checks
still pass.

## 4. MCP

**Deps:** 3. **Tests:** T8.

`apps/cli/src/mcp.rs`: server name from the instance; `INSTRUCTIONS` rendered
with the CLI name, the `guide` command and, for a suffixed instance, the "when
to use it" sentence. The tool list stays six. Done when T8 passes.

## 5. Swift kit

**Deps:** 1 (the table). **Tests:** T9, T10, T11, T6 (Swift part).

Add `LocalRouterKit/Instance.swift`, reading `LRInstanceSuffix` from
`Info.plist`. Derive `BundleLayout`, `DaemonClient.Paths`, the daemon plist name
(`AppModel.daemonPlist`), `CLIInstaller` and `ClaudeInstaller` names, and the
agent help URL from it. Installers replace only links with their own instance's
name (I2). Move the uninstall steps into a testable helper that touches only the
instance's folders and links (I10). The updater gate refuses with a suffix
before any network call (I5). Add `NoFixedNamesTests.swift`. Done when T9, T10,
T11 and the Swift part of T6 pass.

## 6. App UI

**Deps:** 5. **Tests:** T12.

A pure function in the kit decides whether Settings shows the Daemon section
(I12); the section shows version, ports, the `config.json` path with "Show in
Finder", and the restart line when the file's ports differ from the bound ports.
The header and tooltip show the app name; the icon is orange for a suffix or an
ad-hoc build. The Updates section says "Updates are off in LocalRouter-dev" with
a suffix. `HelpView`, `DomainsView`, "Copy Prompt" and "Copy MCP Command" use the
instance's names. At start, the app checks that its daemon and CLI carry its
suffix and shows an error if not. Done when T12 passes and M4 looks right.

## 7. Scripts

**Deps:** 1, 5. **Tests:** M1, M6 (no shell test tool).

`release-lib.sh`: `lr_bundle_id`, `lr_daemon_label`, `lr_app_name` as functions
of the suffix. `build-app.sh --suffix`: checks the rule, names the bundle and
programs, renders `Info.plist.in` (with `LRInstanceSuffix`), `daemon.plist.in`
and the note template, signs with the instance's identifiers, and fails if the
suffixed programs are missing. `install.sh --suffix` with default `-dev`: quits,
replaces and restarts only that instance (I6). `publish.sh`: stops on any suffix
(I7). Done when M1 and the first check of M6 pass.

## 8. Verify

**Deps:** 2, 3, 4, 6, 7. **Tests:** E1c, M1, M2, M3, M4, M5.

Write journey E1c in `apps/cli/tests/e2e.rs`. Run M1 to M5 on the maintainer's
Mac with the real release installed, and record the results in the test plan.
M3 is the acceptance test for gap G1: if the agent picks the wrong CLI, change
the note and MCP texts before release.

## 9. Docs and release

**Deps:** 8. **Tests:** M6.

Update `docs/dictionary.md` (instance, suffix, the name table), the project
`CLAUDE.md` (bundle layout, `install.sh` builds `-dev`, `LOCALROUTER_HOME` versus
the suffix), and the help page's line on cookies shared across ports. Release
with `publish.sh`, run M6, then set this ADR's status to built and append the
Actual Change Manifest with any drift.

## Invariants to tasks

| Invariant | Task |
|---|---|
| I1 release names unchanged | 1, 5 |
| I2 no shared names | 1, 5 |
| I3 suffix only from the bundle | 2, 3 |
| I4 texts name their instance | 3, 4, 6 |
| I5 updater off with a suffix | 5 |
| I6 install.sh keeps the release | 7 |
| I7 publish.sh refuses a suffix | 7 |
| I8 set_config keeps hand edits | 2 |
| I9 no fallback to 80/443 | 2 |
| I10 uninstall is per instance | 5 |
| I11 Rust and Swift agree | 1, 5 |
| I12 Daemon section rule | 6 |
