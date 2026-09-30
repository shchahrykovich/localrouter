# 7. Test plan

The tests that exist for this as-built ADR, the manual checks with their
results, and the gaps. Test IDs are local to this ADR.

## What the repository can run today

| Kind | Tool | Where |
|---|---|---|
| Rust unit tests | `cargo test` | `mod tests` in `libs/core/src/*.rs`, `apps/cli/src/main.rs` |
| Rust integration tests | `cargo test` | `libs/core/tests/proxy.rs` (proxy with a test route table), `apps/daemon/tests/api.rs` (real `localrouterd` over the socket), `apps/cli/tests/cli.rs`, `apps/cli/tests/mcp.rs` (rmcp client), `libs/core/tests/agent_texts.rs` |
| Contract tests | `cargo test`, XCTest | `libs/core/tests/api_examples.rs`, `ApiContractTests.swift`; both read every file in `api/examples/` |
| Browser tests | none | checked by hand |
| LaunchAgent, signed bundle | none | checked by hand |

Every test uses a temp `LOCALROUTER_HOME` and ports `0`, as `CLAUDE.md`
requires. No test dependency was added: `tempfile` was already there.

## Overview

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| T1 | unit | folder target form, refusals, protocol rule (I1, I8) | `RouteTable::validate` | none | `cargo test` |
| T2 | unit | path resolution: files, index, list, 308, hidden names, `..` in every spelling, symlinks out (I2, I3) | `folder::resolve`, `listing`, real temp folders and symlinks | none | `cargo test` |
| T3 | integration | answers through the real proxy: types, headers, Range, HEAD, 304, 404, 405, 308, list, path route, 502 (I2, I3, I4, I7) | `Proxy`, `folder.rs`, `ServeFile`, hyper client | route table: a test `RouteSource` without `validate`; no daemon, no TLS for these cases | `cargo test` |
| T4 | integration | daemon refuses a missing folder, a file, a relative path; saves version 1; up check; serves over HTTP; loads a saved route whose folder is gone (I5, I6, I8) | `localrouterd` binary, socket, HTTP listener | none | `cargo test` |
| T5 | unit | the CLI makes a path absolute and refuses a file or a missing path (I8) | `folder_target` | none | `cargo test` |
| T6 | integration | MCP `folder` argument, its refusals, schema lists `folder`, still six tools (I8, I10) | CLI MCP server, daemon | the agent: rmcp client | `cargo test` |
| T7 | unit | every agent text of both instances names folder routes and the absolute path rule (I9) | `help::render`, note, MCP instructions | none | `cargo test` |
| T8 | contract | the new example pair round-trips in Rust and Swift; version 1.2 | both decoders | none | `cargo test`, `swift test` |
| T9 | regression | forwarding still works after the body type change | the existing 17 proxy tests, all daemon and CLI tests | as before | `cargo test` |
| E1 | end-to-end | relative `--folder dist` from another working folder, `list` shows up, GET through the daemon, CLI refusals | CLI binary, daemon binary, HTTP | browser: reqwest | `cargo test` |
| M1 | manual | a daemon run by hand, `curl` on list, file, `.env`, folder without `/` | daemon, CLI, curl | LaunchAgent: a terminal process | by hand, **done** |
| M2 | manual | installed dev app, browser over HTTPS: page, list, reload after edit | everything | none | by hand, open |
| M3 | manual | a folder in `~/Documents` through the LaunchAgent (U1) | macOS privacy rules | none | by hand, open |
| M4 | manual | a Claude Code session asked for an HTML report with a URL uses `folder` | agent, MCP, texts | none | by hand, open |
| M5 | manual | downgrade: an older daemon skips the folder route and says so | older release | none | by hand, open |

**Replaced parts and who covers them.**

- T3's route table skips `validate`, so its targets are not normalized (the
  temp folder is `/var/...`, a symlink to `/private/var/...`). This is a real
  case: `resolve` canonicalizes the root, so the test also proves that a
  folder reached through a symlink works. The normalized form is covered by T1
  and T4.
- No automated test runs a browser, TLS with a folder route, or the
  LaunchAgent. M2 and M3 cover them.
- I4's "no write call" is checked only by reading `folder.rs`; no test can
  prove an absence of writes cheaply.

## Automated tests (CI)

Run all: `cargo test --workspace && swift test --package-path apps/menubar`

### Route model, `libs/core/src/routes.rs`

| Test | Case |
|---|---|
| `folder_targets_are_normalized` | trailing `/`, `//`, `/./` give one form; `Route::folder` returns the path |
| `bad_folder_targets_are_refused_with_the_rule` | relative, `./site`, `..`, `/`, `//////`, a newline |
| `folder_routes_are_http_routes_and_may_have_a_path` | TCP + `file://` refused; `shop/docs` folder route wins its path; `serves` is true |
| `bad_target_message_names_the_folder_form` | the error text names `file://` |

### Path resolution, `libs/core/src/folder.rs`

| Test | Case |
|---|---|
| `files_folders_and_index_html` | `/` gives `index.html`, `%20` decoded, a link inside is served, `/sub` and `""` give 308, missing and `a.txt/x` give 404 |
| `a_folder_without_index_html_is_listed_without_hidden_names` | list order, hidden names and links out left out |
| `hidden_names_dot_dot_and_links_out_of_the_folder_are_not_found` | `.env`, `.git/config`, `%2egit`, `..`, `%2e%2e`, `%2f` inside a part, link out, link to `.git`, NUL, bad UTF-8 |
| `a_missing_folder_is_reported_as_gone` | missing folder, a file as the folder |
| `listing_links_are_encoded_and_escaped` | `a b#<c>.html` is encoded in `href` and escaped in text; `../`; title decoded |

### Proxy, `libs/core/tests/proxy.rs`

| Test | Case |
|---|---|
| `folder_route_serves_files_with_type_and_no_cache` | `text/html; charset=utf-8`, `no-cache`, `Last-Modified`, `text/javascript`, log entry with the route |
| `folder_route_answers_range_head_and_if_modified_since` | 206 with `bytes=2-4`, HEAD with length and no body, 304 |
| `folder_route_hides_dot_files_and_refuses_other_methods` | five 404 cases never show the secret; POST gives 405 with `Allow` |
| `folder_path_route_maps_its_path_to_the_folder_and_adds_the_slash` | `/docs/sub/page.html`, 308 keeps the query, list, other paths reach the server |
| `folder_route_whose_folder_is_gone_gets_a_502_with_the_folder` | 502 page names the folder and the note |

### Daemon, `apps/daemon/tests/api.rs`

| Test | Case |
|---|---|
| `folder_route_is_checked_saved_served_and_reported_up` | refusals (missing, file, relative), trailing `/` removed, `routes.json` version 1, `upstream_up` true then false, a real HTTP GET, restart with the folder gone keeps the route |

### CLI and MCP

| Test | Case |
|---|---|
| `apps/cli/src/main.rs` `folder_target_is_absolute_and_resolved` | absolute result, a file refused, a missing path named |
| `apps/cli/tests/mcp.rs` `folder_routes_through_mcp` | target built from `folder`; relative, missing, with `port`, with `tcp` refused |
| `apps/cli/tests/mcp.rs` `exactly_six_tools_are_listed` | the schema has `folder`; six tools |

### Texts and contract

| Test | Case |
|---|---|
| `libs/core/tests/agent_texts.rs` `every_agent_text_describes_folder_routes` | release and dev: "folder", "absolute path", `--folder` in the note and the help page |
| `libs/core/tests/api_examples.rs`, `ApiContractTests.swift` | the new example pair decodes and round-trips; both walk the folder, so no test change was needed |

## Automated end-to-end test

`apps/cli/tests/cli.rs` `folder_route_from_a_relative_path`:

1. Make `dist/index.html` in a temp working folder.
2. Run the real CLI there: `add docs --folder dist --session`. Check: the
   output names `file://` plus the real absolute path.
3. `list`. Check: the line starts with `up`.
4. GET `/` with `Host: docs.localhost` through the daemon's HTTP port. Check:
   200 and the page.
5. `--folder` with `--tcp`, a missing folder, and no target at all. Check: each
   fails with a message that names the problem.

## Manual tests

### M1. Daemon by hand (done, 2026-09-30)

`H=$(mktemp -d /tmp/lrXXXX) && printf '{"version":1,"http_port":0,"https_port":0}' > $H/config.json && LOCALROUTER_HOME=$H cargo run -p localrouterd`

| Check | Expected | Result |
|---|---|---|
| `localrouter add report --folder <site> --session` | prints `file://` plus the absolute path and two URLs | ✓ |
| `curl` `/` on a folder without `index.html` | file list: `css/`, `report.html`; `.env` not listed | ✓ |
| `curl -i /report.html` | 200, `text/html; charset=utf-8`, `no-cache`, the page | ✓ |
| `curl /.env` | 404 | ✓ |
| `curl /css` | 308 to `/css/` | ✓ |
| `localrouter list` | `up` and the `file://` target | ✓ |

### M2. Installed dev app and a browser (open)

`scripts/install.sh --user --launch`, then `localrouter-dev add coverage.shop --folder <dir>`.

| Check | Expected |
|---|---|
| open `https://coverage.shop.localhost:7443/` | the page, a trusted certificate |
| a folder without `index.html` | the file list; links open files and folders |
| edit the HTML, reload | the new content (304 only when unchanged) |
| a video or a large file | plays and seeks (Range) |
| the menu bar app | the row shows `→ file:///…` and a green dot; delete the folder and it turns red |

### M3. macOS privacy (open, U1)

| Check | Expected |
|---|---|
| folder route to `~/Documents/x` through the LaunchAgent | one of: works; a macOS prompt that names the app; the 403 page. Record which, and update U1 and `help.md` |
| same folder under `~/Projects` | works |

### M4. Claude Code acceptance (open)

| Check | Expected |
|---|---|
| ask a session with the MCP server: "make an HTML page with the test results and give me a URL" | the agent writes the file, calls `register_route` with an absolute `folder`, and gives an `https://….localhost` URL that works |
| the agent gives a relative folder | the error says "absolute path"; the agent retries with one |

### M5. Downgrade (open)

| Check | Expected |
|---|---|
| save a persistent folder route, install the previous release | status names the skipped route; other routes work |
| add any persistent route on the old release, then upgrade | the folder route is gone from `routes.json` (the known loss in the manifest's blast radius) |

## Not in this plan without approval

- **Browser tests** (for example Playwright): would cover M2's page, list and
  reload checks in CI. Costs a new test tool and a browser in CI.

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| I1 folder target form | T1 | |
| I2 nothing outside the folder | T2, T3 | |
| I3 no hidden part served or listed | T2, T3 | M1 |
| I4 read only, GET and HEAD | T3 (405) | code review for "no write" |
| I5 register refuses, load keeps | T4 | |
| I6 version unchanged | T4 | |
| I7 own 502, no fall-through | T3 | |
| I8 absolute path only | T1, T4, T5, T6, E1 | M4 |
| I9 agent texts | T7 | M4 |
| I10 six MCP tools | T6 | |
