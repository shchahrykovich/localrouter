# 7. Test plan

Proposed: these tests are to be written. The commands are the repo's own
(`CLAUDE.md`).

## What the repo can run today

| Kind | Tool | Where |
|---|---|---|
| Rust unit tests | `cargo test` | `#[cfg(test)]` modules in `libs/core/src/*.rs`; daemon unit tests in the binary crate |
| Rust integration tests | `cargo test --test …` | `libs/core/tests/` (`forward.rs` has a `harness()` with echo servers and a test CA), `apps/daemon/tests/api.rs`, `apps/cli/tests/` (`cli.rs`, `mcp.rs`, `e2e.rs`; `common/mod.rs` builds and starts `localrouterd`) |
| Contract tests | both sides walk `api/examples/` | `libs/core/tests/api_examples.rs`, `ApiContractTests.swift` |
| Swift unit tests | `swift test --package-path apps/menubar` | `LocalRouterKit` only; the app target has no tests |
| Browser tests | none | no browser test tool in the repo |

So the viewer's JavaScript and the app's views have no automated tests. They
are covered by manual tests M2 and M3. A browser test tool is listed under
"Not in this plan without approval".

## Overview

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| T1 | unit | a record becomes the right HAR entry | `har/entry.rs` | records built by hand | `cargo test -p localrouter-core --lib har::entry` |
| T2 | unit | the writer: valid JSON, roll, prune, repair, deleted file, errors | `har/writer.rs`, a temp folder | the clock (a fixed start time) | `cargo test -p localrouter-core --lib har::writer` |
| T3 | unit | secret headers are redacted; scripts still see the same list | `secrets.rs`, `entry.rs`, `lua_api.rs` | none | `cargo test -p localrouter-core --lib secrets` |
| T4 | integration | the forward proxy records each mode; off records nothing | `ForwardProxy`, router, upstream with test CA | internet servers (echo servers), DNS (`LOCALROUTER_TEST_RESOLVE`) | `cargo test -p localrouter-core --test forward har` |
| T5 | integration | a stalled writer never delays traffic | `ForwardProxy`, `HarLog` | the disk (a writer paused by a test hook) | `cargo test -p localrouter-core --test forward har_backpressure` |
| T6 | integration | the viewer: paths, peers, names, methods, headers, live feed, not logged | router `Proxy::handle`, `har/viewer.rs` | the peer address (passed as an argument) | `cargo test -p localrouter-core --test proxy proxy_log` |
| T7 | unit and integration | the reserved name and the saved `proxy` route | `routes.rs`, `Daemon::load` | none | `cargo test -p localrouter-core --lib routes::tests` and `cargo test -p localrouterd --test api proxy_route` |
| T8 | integration | daemon API: set the 3 fields, `get_proxy` log block, live off and on | `localrouterd` over the socket | none | `cargo test -p localrouterd --test api proxy_log` |
| T9 | contract | the new example files decode on both sides | Rust and Swift types | none | `cargo test -p localrouter-core --test api_examples` and `swift test --package-path apps/menubar --filter ApiContractTests` |
| T10 | integration | `proxy log …` commands | the CLI binary and daemon | none | `cargo test -p localrouter --test cli proxy_log` |
| T11 | integration | MCP: nine tools; `get_proxy` has `log`; description names it | the MCP server and daemon | none | `cargo test -p localrouter --test mcp` |
| T12 | text | agent texts name the log, jq recipes, no fixed `proxy.localhost` | `help.md`, `note.md`, `mcp.md`, all sources | none | `cargo test -p localrouter-core --test agent_texts --test no_fixed_names` and `swift test --package-path apps/menubar --filter NoFixedNamesTests` |
| T13 | Swift unit | `ProxyLog`: browser choice, state text; settings search finds the log | `LocalRouterKit` | the installed-app check (an injected function) | `swift test --package-path apps/menubar --filter "ProxyLogTests\|SettingsTreeTests"` |
| T14 | unit | the LAN decision for every case | `listen::peer_allowed` | the network id (passed in) | `cargo test -p localrouterd --bin localrouterd listen::tests` |
| T15 | unit | network id from interface, routing and ARP data | `network.rs` parsing | the sysctl bytes (recorded fixtures) | `cargo test -p localrouterd --bin localrouterd network::tests` |
| T16 | integration | `lan_networks`, `status.network`, the update from `allow_lan: true` | `localrouterd` over the socket | the current network (`LOCALROUTER_TEST_NETWORK`, debug builds only) | `cargo test -p localrouterd --test api lan` |
| T17 | integration | idle: no writer thread, no open file, no queue, no live channel | `HarLog`, writer, viewer | the 30 s idle time (shortened by a test setting) | `cargo test -p localrouter-core --test proxy har_idle` |
| E1 | end-to-end | a request through the proxy shows up in a HAR file and in the viewer | daemon, CLI, router, proxy, writer, viewer, files | the internet server (an echo upstream) | `cargo test -p localrouter --test e2e proxy_log_journey` |
| E2 | end-to-end | the agent loop through the CLI: operate, run, inspect | daemon, CLI, `curl` and `python3` started with `proxy env`, `jq` | the internet server (a TLS echo upstream with the test CA) | `cargo test -p localrouter --test e2e agent_loop` |
| M1 | manual | real Chrome imports the file | Chrome DevTools | none | by hand |
| M2 | manual | the viewer page in real Chrome | Chrome, the page's JavaScript | none | by hand |
| M3 | manual | the app UI | the menu bar app | none | by hand |
| M4 | manual | failures: disk full, folder not writable, file deleted | the real disk | none | by hand |
| M5 | manual | another machine with `allow_lan` gets 403 | a second device on the LAN | none | by hand |
| M6 | manual | agent acceptance: Claude Code finds a failed request | Claude Code, MCP, jq | none | by hand |
| M7 | manual | update with a saved route `proxy` | a real `routes.json` | none | by hand |
| M8 | manual | load: a heavy page through the proxy, no visible delay | Chrome, real sites | none | by hand |
| M9 | manual | LAN at home works; on a phone hotspot it does not | a phone, home Wi-Fi, a hotspot | none | by hand |
| M10 | manual | Wi-Fi and Ethernet at once: each interface is checked on its own | two networks | none | by hand |
| M11 | manual | memory: idle and after load the daemon returns to its idle size | `footprint`, Activity Monitor | none | by hand |

**Replaced parts and who covers them.**

| Replaced part | Hidden | Covered for real by |
|---|---|---|
| echo servers instead of internet servers | real TLS servers, HTTP/2, real header sets | M2, M8 (real sites through the proxy) |
| the peer address passed as an argument (T6) | that `listen.rs` gives the real peer to the router | M5 |
| the paused writer (T5) | a real slow disk | M8, M4 |
| the clock in T2 | file names from the real time, collisions | E1 (real clock), M4 |
| the network id passed in (T14) and `LOCALROUTER_TEST_NETWORK` (T16) | that `network.rs` reads the real tables, on Wi-Fi, Ethernet, VPN | M9, M10 |
| recorded sysctl bytes (T15) | a macOS version that changes the table format | M9 on each new macOS |
| the shortened idle time (T17) | the real 30 s timer | M11 |
| the installed-app check (T13) | whether Chrome is really found | M3 |

## Mock-versus-real check

`libs/core/tests/forward.rs` builds a `ForwardProxy` by hand in `harness()`.
It does not go through `Daemon::load`. A T4 test that passes a `HarLog` to
the harness proves the proxy records, but not that the daemon makes the
`HarLog` from `config.json` with the right flag and limits. T8 covers that
through the socket, and E1 covers the whole path. Without T8, T4 would pass
with a daemon that never turns the log on.

The in-memory log has a test that the query is never stored
(`logs.rs`, `query_string_is_never_stored`). The HAR record must not be built
from `LogEntry`, or it loses the query too. T1 checks a URL with a query, and
E1 checks it on the real path.

## Test data

| Data | Used by | Why |
|---|---|---|
| a request with `Authorization`, `Cookie`, `X-Api-Key`, `Proxy-Authorization` and one header from `secret_headers` | T3, E1 | every kind of secret, including a user-added name |
| a URL with a query `?q=1&api_key=abc` | T1, E1 | the query is written as it is (U2 is open) |
| a script rule with `reveal_secrets: true` on the same host | T3 | the rule sees the value; the file does not |
| a folder with `notes.txt`, `proxy-old.har` (bad name) and six valid files | T2 | prune deletes only the oldest valid one |
| a file cut in the middle of an entry line | T2 | repair at start |
| a symbolic link `proxy-20260101-000000.har` → `ca.key` | T6 | the viewer refuses links |
| a saved `routes.json` with a route `proxy` | T7, M7 | the route survives the update |

## Automated tests (CI)

### Core unit tests: `libs/core/src/har/`, `secrets.rs`

| Test file | Case |
|---|---|
| `har/entry.rs` | absolute-form GET: method, full URL with query, `queryString`, status, `statusText`, times, `_mode` `http` |
| `har/entry.rs` | inspected request: `_mode` `inspect`; `.localhost` via proxy: `_route` set |
| `har/entry.rs` | tunnel: method `CONNECT`, URL `https://host:port`, `_bytesIn`, `_bytesOut` |
| `har/entry.rs` | `bodySize` from `Content-Length`, `-1` without it; `content.mimeType`; `redirectURL` from `Location` |
| `har/entry.rs` | `_scripts` and `_scriptError` from the script run |
| `har/writer.rs` | after each of 100 entries the file parses as HAR 1.2 with `creator.name` = the instance name |
| `har/writer.rs` | roll at `file_requests`; roll at `file_mb`; one entry bigger than the limit is alone in its file |
| `har/writer.rs` | prune keeps the 5 newest; `notes.txt` and `proxy-old.har` stay |
| `har/writer.rs` | a name collision gets `-2` |
| `har/writer.rs` | a file cut mid-line is repaired at start; a whole file is left as it is; the new run starts a new file |
| `har/writer.rs` | the current file deleted: the next entry starts a new file |
| `har/writer.rs` | a read-only folder: the writer stops, keeps the error, does not loop |
| `secrets.rs` | the default list plus `secret_headers`; case does not matter |
| `har/entry.rs` | secret headers are `[redacted]`; `Proxy-Authorization` is absent |

### Core integration tests: `libs/core/tests/forward.rs`

Uses the existing `harness()`, `echo_server()`, `tls_server()` and `send()`.

| Test file | Case |
|---|---|
| `forward.rs` | `har_records_each_mode`: one absolute-form GET, one inspected request, one tunnel, one `.localhost` GET through the proxy: four entries in this order of completion |
| `forward.rs` | `har_off_records_nothing`: the flag off, ten requests, zero records, zero queue sends |
| `forward.rs` | `har_records_client_headers_before_scripts`: an intercept rule adds a header; the HAR request headers do not have it, the response headers are what the client got |
| `forward.rs` | `har_backpressure`: the writer paused, 10,000 requests all complete with 200, `dropped` > 0, no request slower than with the log off by more than a set margin |

### Core integration tests: `libs/core/tests/proxy.rs` (the viewer)

| Test file | Case |
|---|---|
| `proxy.rs` | `proxy_log_paths`: `/`, `/viewer.js`, `/viewer.css`, `/api/files`, `/files/<name>`, `?download=1` sets `Content-Disposition` |
| `proxy.rs` | `proxy_log_second_address`: the same answers under `router.localhost/proxy-log/` |
| `proxy.rs` | `proxy_log_loopback_only`: peer `192.168.1.5` gets 403; `127.0.0.1` and `::1` get 200 |
| `proxy.rs` | `proxy_log_methods`: `POST`, `PUT`, `DELETE` get 405 |
| `proxy.rs` | `proxy_log_names`: `..`, `%2e%2e`, `config.json`, a symbolic link get 404 |
| `proxy.rs` | `proxy_log_headers`: CSP, `nosniff`, `no-store` on every answer; no `Access-Control-*` header |
| `proxy.rs` | `proxy_log_current_file_is_whole`: 1,000 reads while entries are written; each parses |
| `proxy.rs` | `proxy_log_live`: an entry written after the page connects arrives as `event: entry`; turning off sends `event: off` |
| `proxy.rs` | `proxy_log_not_logged`: requests to `proxy.localhost` reach neither the HAR nor `RequestLog`; `router.localhost` does not reach the HAR |
| `proxy.rs` | `proxy_log_no_scripts`: a script rule cannot be set for host `proxy`, and none runs there |

### Routes and the daemon: `routes.rs`, `apps/daemon/tests/api.rs`

| Test file | Case |
|---|---|
| `routes.rs` | `validate` refuses `proxy` and `Proxy` for a new route |
| `api.rs` | `proxy_route_saved_before_is_kept`: a `routes.json` with `proxy` loads; the route answers `proxy.localhost`; status has the problem text |
| `api.rs` | `proxy_log_defaults`: a fresh home: `proxy_log` true, 20, 5000; `get_proxy` has `log` with the folder under the temp home |
| `api.rs` | `proxy_log_set_config`: each field changes; out-of-range values are refused with the range in the message |
| `api.rs` | `proxy_log_off_then_on`: off stops records at once; on starts a new file |
| `api.rs` | `proxy_env_bundle`: with an inspection CA, `env` has the 3 CA names pointing at `bundle.pem`; the bundle has more than 100 certificates, ends with the inspection CA, and has no private key; the lowercase proxy names are set |

### Contract: `api/examples/`

| Test file | Case |
|---|---|
| `api_examples.rs`, `ApiContractTests.swift` | walk the folder: the new `set_config_proxy_log.request.json` and the changed replies round-trip on both sides. This test grows by itself: a new example file is checked without a code change. |

### CLI and MCP: `apps/cli/tests/`

| Test file | Case |
|---|---|
| `cli.rs` | `proxy_log` prints the state; `on`, `off`, `limits --mb 5 --requests 200`, `path` |
| `cli.rs` | `proxy on` prints the `Log:` line |
| `mcp.rs` | `exactly_nine_tools_are_listed` stays as it is and passes |
| `mcp.rs` | `get_proxy` result has `log.folder` and `log.url`; the description mentions the HAR log |

### Texts: `libs/core/tests/`

| Test file | Case |
|---|---|
| `agent_texts.rs` | `help.md`, `note.md`, `mcp.md` mention the log folder template, the viewer URL, `[redacted]` and "do not read a whole file" |
| `no_fixed_names.rs`, `NoFixedNamesTests.swift` | the word list gains `proxy.localhost`; the scan covers every source and text file, so it grows by itself |

### Swift: `apps/menubar/Tests/LocalRouterKitTests/`

| Test file | Case |
|---|---|
| `ProxyLogTests.swift` | Chrome installed: open with Chrome; not installed: the default browser; the state line for on, off, error, dropped |
| `SettingsTreeTests.swift` | "har" and "requests per file" find the Proxy page |

### LAN per network: `apps/daemon/src/listen.rs`, `network.rs`, `apps/daemon/tests/api.rs`

| Test file | Case |
|---|---|
| `listen.rs` | loopback peer: accepted with `allow_lan` off and on, no lookup made |
| `listen.rs` | LAN peer, `allow_lan` off: refused |
| `listen.rs` | LAN peer, on, network in the list: accepted; not in the list: refused; unknown: refused |
| `listen.rs` | the proxy port and TCP route listeners never call the LAN check |
| `network.rs` | recorded routing and ARP bytes give `mac:18:35:d1:15:d1:a8` for `en0`; a `utun` interface gives unknown; no ARP entry gives unknown |
| `api.rs` | `lan_networks` set and read; `status.network` with `LOCALROUTER_TEST_NETWORK` |
| `api.rs` | update: `allow_lan: true` and no list becomes the current network; with an unknown network, an empty list; a read-only `config.json` still gives the list in memory |

### Resource use: `libs/core/tests/proxy.rs`

| Test file | Case |
|---|---|
| `proxy.rs` | `har_idle`: after the idle time the writer thread has exited, the file is closed, the queue is gone; the next request starts them again and appends to the same file |
| `proxy.rs` | `har_idle`: with no `/api/live` page there is no live channel; one page makes it; closing it drops it at the next record |
| `proxy.rs` | `proxy_log_streams`: a 5 MB file arrives in chunks of at most 64 KB |
| `proxy.rs` | `proxy_log_entries_from_the_end`: a 50 MB file, `limit=500`, returns the newest 500 in order |

## Automated end-to-end test

`apps/cli/tests/e2e.rs`, `proxy_log_journey`. It starts a real
`localrouterd` with a temp home and ports 0 (`common/mod.rs`).

| Step | Check |
|---|---|
| 1. `proxy on` through the CLI | the reply has the `Log:` line with the temp folder |
| 2. Send `GET http://example.test/a?x=1` with `Authorization: Bearer t` through the proxy to an echo upstream (`LOCALROUTER_TEST_RESOLVE`) | 200 |
| 3. `GET /api/files` from the viewer | one current file, `entries` 1 |
| 4. `GET /files/<name>` | parses as HAR 1.2; the URL has `?x=1`; `Authorization` is `[redacted]` |
| 5. Read the same file from disk | the same bytes as step 4 |
| 6. `proxy log off`, send one more request | the file still has 1 entry and still parses |
| 7. MCP `get_proxy` | `log.enabled` false, `log.folder` is the temp folder |

## Automated end-to-end test of the agent loop

`apps/cli/tests/e2e.rs`, `agent_loop`. The steps are the commands the
[agent texts](03-clients-and-agents.md#the-agent-loop-operate-run-inspect)
give, run as a shell would run them. Skipped when `python3` or `jq` is
missing on the CI machine, and the test says so.

| Step | Check |
|---|---|
| 1. `proxy on`, `proxy inspect add example.test` | both exit 0 |
| 2. `curl -s https://example.test/a` in a shell after `eval "$(… proxy env)"` | 200 from the TLS echo upstream: `curl` trusted the inspection CA through `CURL_CA_BUNDLE` |
| 3. the same with `python3 -c 'import urllib.request…'` | 200 through `SSL_CERT_FILE` |
| 4. `curl` to a host that is not inspected | the tunnel works: the bundle still holds the roots (the test adds a second test CA to the bundle's root part) |
| 5. `jq` on the newest file from `proxy log path` | the two inspected requests, with their paths; the tunnel as one `CONNECT` |

## Manual tests

### M1. Chrome imports the file

| Check | Expected |
|---|---|
| Turn the proxy on, `proxy chrome`, open three sites, one with an inspected host | the viewer shows the requests |
| Download HAR from the viewer; DevTools → Network → Import HAR file | Chrome shows the requests with methods, statuses, headers |
| Drag the file onto the Network panel | the same |
| Look at an inspected request's headers in DevTools | `authorization` shows `[redacted]` |

### M2. The viewer in Chrome

| Check | Expected |
|---|---|
| Right-click menu → Open Proxy Log | the user's normal Chrome opens `http://proxy.localhost` (dev: `:7080`) |
| Proxy off | the page says the proxy is off and how to turn it on |
| Browse with the proxied Chrome | new rows appear at the top without a reload |
| Filter `api`, "Errors only" | only matching rows |
| Click a row | headers, timings, `_mode` |
| Choose an older file | its rows load |
| DevTools Console on the viewer | no errors, no CSP reports |
| `https://proxy.localhost` with the CA trusted | the same page |

### M3. The app

| Check | Expected |
|---|---|
| Settings → search "har" | the Proxy page, with the "Proxy log" section |
| Turn the log off and on, change both limits | `proxy log` in a terminal shows the new values |
| Proxy tab | the log line and both buttons |
| Show Log Folder | Finder opens the folder |
| Chrome not installed (rename it for the test) | Open Proxy Log uses the default browser |
| A 1.4 daemon (an older build) | the app shows no log controls and no error |

### M4. Failures

| Check | Expected |
|---|---|
| `chmod 500` the folder, send requests | the writer stops; Settings and `proxy log` show the error; pages still load |
| Delete the current file in Finder, send a request | a new file appears |
| Kill the daemon during heavy traffic, start it | the newest old file parses; a new file starts |
| Fill the disk (a small disk image as `LOCALROUTER_HOME`) | the error says the disk is full; traffic continues |

### M5. Another machine

| Check | Expected |
|---|---|
| `allow_lan` on; from a phone open `http://<mac-ip>/proxy-log/` with Host `proxy.localhost` | 403 |
| The same for `router.localhost/proxy-log/` | 403 |

### M6. Agent acceptance

The script is the simulated session in the
[working-backwards file](00-working-backwards.md#a-simulated-session-with-a-coding-agent).

| Check | Expected |
|---|---|
| Start with the proxy off. Ask Claude Code: "Run the e2e tests through the proxy, inspect api.example.com, and tell me which requests failed." | it runs `proxy on` and `proxy inspect add` in the terminal, starts the tests with `proxy env`, then reads the log with `jq` |
| A Python script in the tests calls the inspected host | it works without the user doing anything (the CA bundle) |
| Ask Claude Code: "My test through the proxy got 401 from api.example.com. Why?" | it calls `get_proxy`, uses `jq` on the newest file, does not read the whole file |
| It reports the status and the `www-authenticate` header | correct |
| Ask it to turn the log off | it runs `proxy log off` in the terminal, not an MCP tool |

### M7. Update with a saved `proxy` route

| Check | Expected |
|---|---|
| With the old build, add a persistent route `proxy` → a dev server; update | the route still answers `proxy.localhost` |
| Settings → Daemon, and `status` | the problem text names `router.localhost/proxy-log/` |
| Open `router.localhost/proxy-log/` | the viewer |

### M8. Load

| Check | Expected |
|---|---|
| Open a heavy site (hundreds of requests) in the proxied Chrome, log on, then off | no difference you can see in load time; `dropped` 0 on an SSD |

### M9. LAN at home and on a hotspot

| Check | Expected |
|---|---|
| At home: Settings → Routing → Allow LAN access, "Allow on this network", name it "Home" | the phone on the same Wi-Fi opens a route by the Mac's IP |
| Join the phone's hotspot with the Mac; a second device on the hotspot opens the same address | refused; Settings shows "This network: not allowed" |
| Back home | the phone works again without any change |
| `localrouter lan` in each place | shows the network, the router and "allowed" or "not allowed" |

### M10. Two networks at once

| Check | Expected |
|---|---|
| Wi-Fi (allowed) and Ethernet (not allowed) at once | a device on Wi-Fi gets in; a device on Ethernet is refused |
| A VPN on | a peer through the VPN interface is refused; Settings says the VPN network cannot be recognised |

### M11. Memory

| Check | Expected |
|---|---|
| `footprint localrouterd` with the proxy and log on, no traffic, no page, after 1 minute | the same as with the log off, within 1 MB |
| Load a heavy site, open the viewer, close it, wait 1 minute | back to the idle size within 1 MB; `ps -M` shows no `har-writer` thread |

### Post-release smoke test

| Check | Expected |
|---|---|
| Install the release DMG; turn the proxy on; open one site; Open Proxy Log | the request is in the viewer; the folder is `~/Library/Logs/LocalRouter/proxy` |

None of these tests costs money. M1, M2 and M8 send requests to real sites
through the user's network.

## Not in this plan without approval

| Tool | Would cover | Cost |
|---|---|---|
| A browser test tool (Playwright) | the viewer's JavaScript: filter, live rows, file choice (today M2) | a Node toolchain in a Rust and Swift repo; CI time |
| XCUITest or a UI test target | the app's Settings section and menu items (today M3) | a new test target and signing in CI |

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| I1 log off: no record | T4 `har_off_records_nothing` | |
| I2 never delays traffic | T5 | M8 |
| I3 valid JSON always | T2, T6 `proxy_log_current_file_is_whole`, E1 | M1, M4 |
| I4 secrets redacted | T3, E1 | M1 |
| I5 prune only own files, keep 5 | T2 | |
| I6 file limits | T2 | |
| I7 viewer not logged | T6 `proxy_log_not_logged` | M2 |
| I8 loopback only | T6 `proxy_log_loopback_only` | M5 |
| I9 only HAR files served | T6 `proxy_log_names` | |
| I10 read only, no CORS, CSP | T6 `proxy_log_methods`, `proxy_log_headers` | M2 |
| I11 reserved name, old route kept | T7 | M7 |
| I12 nine MCP tools | T11 | M6 |
| I13 no fixed viewer name | T12 | |
| I14 tests stay in temp homes | T2, T8 (temp homes by construction) | |
| I15 write error stops cleanly | T2 | M4 |
| I16 new file per run, repair | T2 | M4 |
| I17 LAN only on allowed networks | T14, T16 | M9, M10 |
| I18 update never means every network | T16 | |
| I19 proxy port, TCP, viewer this Mac only | T6, T14 | M5 |
| I20 no program, no permission, no cache for the network id | T15, code review | M9 |
| I21 idle holds nothing | T17 | M11 |
| I23 the CA bundle adds, never replaces | T8 `proxy_env_bundle`, E2 | M6 |
| I22 no whole file in memory | T6 `proxy_log_streams`, `proxy_log_entries_from_the_end` | M11 |
