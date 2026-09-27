# 8. Test plan

Tests to write for this proposed ADR. Test IDs are local to this ADR.

## What the repository can run today

| Kind | Tool | Where |
|---|---|---|
| Rust unit tests | `cargo test` | `mod tests` in `libs/core/src/*.rs`, `apps/daemon/src/*.rs` |
| Rust integration tests | `cargo test` | `libs/core/tests/agent_texts.rs` (text checks), `apps/daemon/tests/api.rs` (real daemon over the socket), `apps/cli/tests/cli.rs`, `apps/cli/tests/mcp.rs` (rmcp client), `apps/cli/tests/e2e.rs` (journeys) |
| Contract tests | `cargo test`, XCTest | `libs/core/tests/api_examples.rs`, `ApiContractTests.swift`; both read every `.json` in `api/examples/` |
| Swift unit tests | XCTest | `LocalRouterKitTests` only; the app target has no tests |
| Shell script tests | none | `build-app.sh`, `install.sh`, `publish.sh` are checked by hand |
| UI tests | none | the Settings and menu bar changes are checked by hand |

Every test sets `LOCALROUTER_HOME` to a short temp dir and uses ports `0`, as the
project `CLAUDE.md` requires. Nothing here needs a new dependency.

**How a test gets a suffixed program without building a bundle.** The suffix of
a Rust program comes from its own file name. A test copies the built
`localrouter` or `localrouterd` binary into its temp dir as `localrouter-dev` or
`localrouterd-dev` and runs the copy. `apps/cli/tests/common/mod.rs` already
builds `localrouterd`; it gets a helper `renamed(binary, suffix)`.

## Overview

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| T1 | unit | suffix rule; every name for every row of the table; rows pairwise distinct (I1, I2, I3, I11) | `Instance` (Rust) | none | `cargo test` |
| T2 | unit | folders by instance; `LOCALROUTER_HOME` wins; socket path under 103 bytes for the longest suffix | `Paths` | `HOME` given as a value | `cargo test` |
| T3 | unit + integration | default ports by instance: missing file, file that does not parse, missing field; no rewrite at start (I9) | `Config`, `store.rs`, daemon start | none | `cargo test` |
| T4 | integration | `set_config` keeps a hand edit; `restart_needed` when file ports differ from bound ports (I8) | daemon, socket | none | `cargo test` |
| T5 | unit | every text rendered for `-dev`, 7080, 7443 names only the dev instance (I4) | `help::render`, 404 page, note template, MCP instructions, CLI texts | none | `cargo test` |
| T6 | unit | no fixed CLI name, help URL or note name in a string literal (I4); grows by itself | source files | none | `cargo test`, `swift test` |
| T7 | integration | a binary named `localrouter-dev`: `--help`, `status` (instance, folder), `guide`; a binary named `localrouter-Dev` exits with an error (I3) | CLI, daemon | none | `cargo test` |
| T8 | integration | MCP server name `localrouter-dev`, instructions with the dev CLI and the "when to use" sentence; still six tools | MCP server, daemon | the agent: rmcp client | `cargo test` |
| T9 | unit | Swift `Instance` against the same table (I1, I2, I11) | `Instance` (Swift) | none | `swift test` |
| T10 | unit | installers and uninstall by instance: link names, "only own links", folders (I2, I10) | `CLIInstaller`, `ClaudeInstaller`, uninstall helper | home and `~/.claude` as temp dirs | `swift test` |
| T11 | unit | the updater refuses with a suffix, even with a Team ID (I5) | `Updater` gate | the bundle: a value | `swift test` |
| T12 | unit | the Daemon section rule (I12) | pure function in the kit | status: a value | `swift test` |
| T13 | unit | CA common name carries the app name | `tls.rs` | none | `cargo test` |
| E1c | end-to-end | two instances side by side through real binaries | two daemons, two CLIs, proxy, HTTPS | dev servers: echo servers; folders: `LOCALROUTER_HOME` | `cargo test` |
| M1 | manual | `install.sh` puts `LocalRouter-dev.app` next to the release; both run (I6) | everything | none | by hand |
| M2 | manual | trust the dev CA; browser opens `https://shop.localhost:7443` | browser, keychain | none | by hand |
| M3 | manual | a Claude Code session with both notes uses the right CLI | agent, notes, MCP | none | by hand |
| M4 | manual | edit ports in `config.json`, restart, Settings shows the section | app, daemon | none | by hand |
| M5 | manual | uninstall the dev instance; the release keeps working (I10) | app, keychain, launchd | none | by hand |
| M6 | manual | release smoke test after `publish.sh`; `publish.sh --suffix -dev` stops (I7) | release build | none | by hand |

**Replaced parts and who covers them.**

- E1c sets `LOCALROUTER_HOME` for each daemon, so it does not use the real
  `~/Library` folders chosen by the suffix. T2 checks the folder rule as a
  function; M1 checks the real folders.
- T10 uses temp dirs for home and `~/.claude`. The real links and the
  `CLAUDE.md` line are checked in M1 and M3.
- T11 gives the bundle as a value. A real Developer ID signed dev build is not
  made in any test; the gate is a plain condition on the suffix, checked before
  any signature check.
- No test runs the shell scripts. `build-app.sh` checks its own output (the
  programs with the suffix must exist in the bundle); M1 and M6 run the scripts.
  This is a gap (risk R1 in the manifest). A shell test tool would close it; see
  "Not in this plan without approval".

## 1. Automated tests (CI)

### Core: the instance

`libs/core/src/instance.rs`, `mod tests`, reading `api/instance-names.json`.

| Test file | Case |
|---|---|
| `instance.rs` | T1: for every row of `api/instance-names.json`, every derived name equals the expected one |
| `instance.rs` | T1: the `""` row equals today's constants: `dev.localrouter.app`, `dev.localrouter.app.daemon`, `LocalRouter`, `localrouter`, `LocalRouter.md`, ports 80 and 443 (I1) |
| `instance.rs` | T1: over all rows, data folder, logs folder, bundle id, label, CLI link and note link are pairwise distinct (I2) |
| `instance.rs` | T1: `-dev`, `-test2`, a 16-character suffix (`-` plus 15) are accepted; `dev`, `-`, `-Dev`, `-dev.1`, `-my_build`, a 17-character suffix are refused |
| `instance.rs` | T1: from a file name: `localrouterd-dev` → `-dev`, `localrouter` → `""`, `localrouter-dev` → `-dev`, `localrouterd` → `""`, `something` → error |
| `paths.rs` | T2: suffix `-dev` and `HOME=/Users/u` → `/Users/u/Library/Application Support/LocalRouter-dev`, logs `/Users/u/Library/Logs/LocalRouter-dev` |
| `paths.rs` | T2: `LOCALROUTER_HOME` set → that folder for every suffix |
| `paths.rs` | T2: with the longest allowed suffix, a 28-character user name fits the 103-byte socket limit and a 29-character one is reported by `socket_path_problem` |
| `tls.rs` | T13: a CA made for `-dev` has a common name starting `LocalRouter-dev CA `; for `""` it starts `LocalRouter CA ` (today's test at `tls.rs:375` stays) |

### Core: config defaults

| Test file | Case |
|---|---|
| `config.rs` | T3: `Config::defaults_for(-dev)` has 7080 and 7443; for `""` it has 80 and 443 |
| `config.rs` | T3: parsing `{"https_port": 7444}` for `-dev` gives `http_port` 7080, not 80 |
| `apps/daemon/src/store.rs` | T3: a file that does not parse, for `-dev`, is moved aside and 7080, 7443 are used |
| `apps/daemon/tests/api.rs` | T3: a daemon binary named `localrouterd-dev` with no `config.json` writes one with 7080 and 7443 (ports then set to 0 by the test before binding is checked; see Test data) |
| `apps/daemon/tests/api.rs` | T3: an existing `config.json` with other values is byte-for-byte the same after start |

### Daemon: set_config

`apps/daemon/tests/api.rs`.

| Test file | Case |
|---|---|
| `api.rs` | T4: start the daemon; write `http_port: 0` to `config.json` by hand; call `set_config {allow_lan: true}`; the file still has the hand-edited value and `allow_lan: true` |
| `api.rs` | T4: after a hand edit of the ports, `set_config` replies `restart_needed: true`; without an edit it replies `false` |
| `api.rs` | T4 regression: `set_config {http_port: X}` still saves X and replies `restart_needed: true` (today's behaviour) |

### Texts

`libs/core/tests/agent_texts.rs` (extended) and a new `libs/core/tests/no_fixed_names.rs`.

| Test file | Case |
|---|---|
| `agent_texts.rs` | T5: `help::render` for `-dev`, 7080, 7443: no `localrouter` not followed by `-dev`; no `router.localhost` not followed by `:7080`; every `https://…localhost` example has `:7443` |
| `agent_texts.rs` | T5: the same render for `""`, 80, 443 equals today's page except the `guide` line (regression) |
| `agent_texts.rs` | T5: the 404 page for a request on port 7443 links to `//router.localhost:7443/` |
| `agent_texts.rs` | T5: the note template rendered for `-dev` names `localrouter-dev guide` and has the "when to use it" sentence; rendered for `""` it has neither the suffix nor the sentence |
| `agent_texts.rs` | T5: both rendered notes have the fallback line with their default help URL: `http://router.localhost` for `""`, `http://router.localhost:7080` for `-dev` (G5) |
| `agent_texts.rs` | T5: ADR 03's checks (path routes, "production build", `basePath`) still pass for every rendered text |
| `no_fixed_names.rs` | T6: every string literal in `apps/cli/src/*.rs` and `libs/core/src/*.rs` that contains `localrouter `, `router.localhost` or `LocalRouter.md` is on the allow list; the list names each line and why |
| `NoFixedNamesTests.swift` | T6: the same scan over `apps/menubar/Sources/**/*.swift` |

### CLI and MCP

`apps/cli/tests/cli.rs`, `apps/cli/tests/mcp.rs`, using `renamed(binary, "-dev")`.

| Test file | Case |
|---|---|
| `cli.rs` | T7: `localrouter-dev --help` names `localrouter-dev` in the usage line |
| `cli.rs` | T7: `localrouter-dev status` prints `instance: -dev` and the data folder |
| `cli.rs` | T7: `localrouter-dev guide` prints the help page with the daemon's bound ports and the routes |
| `cli.rs` | T7: `localrouter-Dev status` exits non-zero with "invalid instance suffix" and creates no folder |
| `mcp.rs` | T8: server info name is `localrouter-dev`; instructions contain `localrouter-dev guide` and the "when to use it" sentence |
| `mcp.rs` | T8: the tool list is still the same six tools (existing check) |

### Swift kit

`apps/menubar/Tests/LocalRouterKitTests/`.

| Test file | Case |
|---|---|
| `InstanceTests.swift` | T9: every row of `api/instance-names.json` gives the same names as in Rust; the `""` row equals today's `BundleLayout` and installer constants |
| `InstanceTests.swift` | T9: `Instance(infoPlistSuffix: nil)` (for `swift run`) is the empty suffix |
| `ClaudeInstallerTests.swift` | T10: the dev installer makes `LocalRouter-dev.md` and the line `@LocalRouter-dev.md`; it refuses to replace `LocalRouter.md` pointing into a release bundle, and the release installer refuses the dev link |
| `CLIInstallerTests.swift` (new) | T10: the dev installer links `localrouter-dev`; neither installer replaces the other's link |
| `UninstallTests.swift` (new) | T10: the uninstall helper for `-dev` in a temp home removes the dev folders and links and leaves the release folders and links in place |
| `UpdaterTests.swift` | T11: with a suffix, the gate refuses before any network call, also when a Team ID is given |
| `DaemonSectionTests.swift` | T12: 80/443 and no problem → hidden; 7080/7443 → shown; 80/443 with a listen error → shown; file ports differ from bound ports → shown with the restart line |

## 2. Automated end-to-end test

`apps/cli/tests/e2e.rs`, new journey **E1c: two instances side by side**.

1. Start echo server A and echo server B.
2. Start `localrouterd` with home H1 and `localrouterd-dev` (a renamed copy)
   with home H2, both with ports `0`. Check: both are running at once; neither
   exits with "already running".
3. `localrouter add shop <A>` (home H1) and `localrouter-dev add shop <B>`
   (home H2). Check: each CLI's `list` shows only its own route.
4. Request `https://shop.localhost:<H1 https port>/` and
   `https://shop.localhost:<H2 https port>/`. Check: A answers the first, B
   the second.
5. Request `http://router.localhost:<H2 http port>/`. Check: the page names
   `localrouter-dev` and the H2 ports; it does not contain `localrouter add`.
6. Stop `localrouterd`. Check: the dev instance still answers step 4.

## 3. Manual tests

### M1. Install next to the release

| Check | Expected |
|---|---|
| Install the GitHub release; run `scripts/install.sh --user --launch` | two apps: `/Applications/LocalRouter.app`, `~/Applications/LocalRouter-dev.app` |
| Menu bar | two icons; the dev icon is orange; tooltips `LocalRouter` and `LocalRouter-dev` |
| `ps -axo comm \| grep localrouterd` | `localrouterd` and `localrouterd-dev` |
| `ls ~/Library/Application\ Support/` | `LocalRouter` and `LocalRouter-dev` |
| System Settings → Login Items | two entries |
| Release Domains tab | release routes unchanged |
| `scripts/install.sh --user` again | only the dev app is replaced; the release keeps running |

### M2. Dev HTTPS

| Check | Expected |
|---|---|
| Dev Settings → Trust… | the keychain shows `LocalRouter-dev CA …` next to `LocalRouter CA …` |
| `localrouter-dev add shop 5173`, open `https://shop.localhost:7443` | the dev server page, no certificate warning |
| Open `https://shop.localhost` | the release route (or its 404 page), not the dev one |

### M3. Claude Code with both notes

Script from the working-backwards session ([00](00-working-backwards.md)).

| Check | Expected |
|---|---|
| Install both notes; start a Claude Code session in a project | `CLAUDE.md` has `@LocalRouter.md` and `@LocalRouter-dev.md` |
| "Start the dev server and give it a name" | the agent uses `localrouter` |
| "Register it on the dev build of LocalRouter" | the agent uses `localrouter-dev`, reports a `:7443` URL |
| Ask the agent how it chose | it quotes the "when to use it" sentence |

### M4. Ports by hand

| Check | Expected |
|---|---|
| Dev Settings | Daemon section shows version, 7080, 7443 and the `config.json` path |
| Edit `config.json` to 7081, 7444; switch "Allow LAN access" | the file keeps 7081, 7444; Settings shows the restart line |
| Run the copied restart command | Settings shows 7081, 7444; routes show `:7444` URLs |
| Release Settings | no Daemon section (80, 443, no problem) |

### M5. Uninstall the dev instance

| Check | Expected |
|---|---|
| Dev Settings → Uninstall | dev folders, links, CA and login item are gone |
| Release | still running, routes and trust unchanged, `~/.local/bin/localrouter` still links to it |

### M6. Release smoke test

| Check | Expected |
|---|---|
| `scripts/publish.sh --suffix -dev` | stops with an error before building |
| Update an older release to the new one | same folders, same routes, same CA, no new files; Settings has no Daemon section |
| `localrouter guide` | the help page with ports 80 and 443 |

## Test data

| Fixture | Used by | Why |
|---|---|---|
| `api/instance-names.json` with rows `""`, `-dev`, `-test2`, and the 16-character maximum | T1, T9 | the empty row pins today's names; two non-empty rows prove distinctness; the longest row checks limits |
| A daemon binary copied as `localrouterd-dev` | T3, E1c | the suffix comes from the file name |
| A `config.json` with only `https_port` | T3 | the missing-field case that today falls back to 80 |
| A `config.json` that is not JSON | T3 | the move-aside case that today falls back to 80 and 443 |
| Two homes, H1 and H2, with the same route name `shop` | E1c | same names in both instances, so a leak between them gives a wrong answer instead of none |
| A release link and a dev link in one temp home | T10 | each installer and uninstall must leave the other's link alone |

T3's daemon test cannot bind 7080 and 7443 in CI. It starts the daemon, reads
the written `config.json`, and stops it; binding is tested with ports `0`, as
everywhere else.

## Not in this plan without approval

| Tool | Would cover | Cost |
|---|---|---|
| A shell test tool (for example `bats`) | `build-app.sh --suffix`, `install.sh` default, `publish.sh` refusal (I6, I7, R1) | a new dev dependency and CI step |
| A UI test target for the app | Settings Daemon section, header and tooltip | a new Xcode test target; the package has none today |

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| I1 release names unchanged | T1, T9, T5 (regression) | M6 |
| I2 no shared names | T1, T9, T10 | M1 |
| I3 suffix only from the bundle | T1, T7 | — |
| I4 texts name their instance | T5, T6, T8, E1c | M3 |
| I5 updater off with a suffix | T11 | — |
| I6 install.sh keeps the release | none (no shell test tool) | M1 |
| I7 publish.sh refuses a suffix | none (no shell test tool) | M6 |
| I8 set_config keeps hand edits | T4 | M4 |
| I9 no fallback to 80/443 | T3 | M4 |
| I10 uninstall is per instance | T10 | M5 |
| I11 Rust and Swift agree | T1, T9 | — |
| I12 Daemon section rule | T12 | M4 |
