# 10. Plan versus actual

**Status:** built on branch `settings-window`, 2026-10-02. Not released.
Tasks 1 to 12 and the docs of task 13 are done. The release and the
post-release smoke test are not done: they wait for the user.

## Where it differs from the plan

| # | Plan | Actual | Why |
|---|---|---|---|
| D1 | The network id comes from the routing table and the ARP table, by `sysctl` ([change 4](04-lan-per-network.md)). | The interface comes from `getifaddrs`; the router and its MAC address come from the macOS System Configuration store: `State:/Network/Service/*/IPv4` keys `InterfaceName`, `ARPResolvedIPAddress`, `ARPResolvedHardwareAddress`. | Checked on this Mac (macOS 26): `sysctl NET_RT_FLAGS` with `RTF_LLINFO` returns **0 bytes** to a new program, and `NET_RT_DUMP` has no MAC address. A program that has Local Network access (here, an older Python) gets the full table. The store gives the same values to an unsigned program, with no permission. I20 still holds: no program runs, no permission, no cache. |
| D2 | The network lookup runs in the accept loop. | It runs in a blocking task per connection from another machine; loopback is accepted at once, as before. | The first store lookup in a process takes about 0.7 s (the framework loads); later ones take about 0.4 ms. The accept loop must not wait. |
| D3 | `/api/entries?before=<n>`: entries before entry number `n`. | `before` is a byte offset: the answer gives the cursor of its oldest entry, and `null` at the start of the file. | The viewer reads the file backwards from its end and does not know the entry number without reading the whole file (I22). |
| D4 | Requests to `router.localhost` keep going to the in-memory log. | Requests to `router.localhost/proxy-log/…` (the viewer's second address) go to neither log, as for `proxy.localhost`. Other `router.localhost` requests are logged as before. | The second address is the viewer: its live feed would fill the Logs tab. |
| D5 | `status` reports the saved-route problem and the LAN update note. | A new optional field `status.notes` (a list of sentences). The CLI prints them as `Note` lines; Settings → Daemon shows them. | The ADR named the texts, not the field. |
| D6 | `proxy log path` asks the daemon. | It prints the folder from the instance's paths, with no daemon, like `proxy ca-path`. | The `jq` recipes work even when the daemon is down. |
| D7 | T5 sends 10,000 requests with the writer paused. | 5,000 requests (50 tasks × 100). | More than the 4,096-record queue, so drops are proven; the test stays under a second. |
| D8 | T16 checks a read-only `config.json` during the LAN update. | Not automated. | The daemon writes `config.json` by rename in its data folder, which the test cannot make read-only without stopping the daemon from starting. The code keeps the list in memory when the write fails (`Daemon::load`), and `set_config` takes the list from memory. |
| D9 | `secrets.rs` is a list. | `SecretHeaders`, one object the scripts and the HAR log share (`Scripts::secrets`). | One object cannot drift; `set_config` changes both at once. |

## Automated tests: results

All run on 2026-10-02 on this Mac.

| Suite | Result |
|---|---|
| `cargo test --workspace` | 400 passed, 0 failed |
| `cargo clippy --workspace --all-targets` | no warnings |
| `swift test --package-path apps/menubar` | 127 passed, 0 failed |

| Test plan ID | Where | Result |
|---|---|---|
| T1, T3 | `har/entry.rs`, `secrets.rs` | passed |
| T2 | `har/writer.rs` (roll, prune, collision, repair, deleted file, read-only folder) | passed |
| T4, T5 | `libs/core/tests/forward.rs` `har_*` | passed |
| T6, T17 | `libs/core/tests/proxy.rs` `proxy_log::*`, `har/mod.rs` | passed |
| T7 | `routes.rs`, `tls.rs`, `api.rs` `proxy_route_saved_before_is_kept`, `proxy.rs` `a_saved_proxy_route_wins_over_the_viewer` | passed |
| T8 | `apps/daemon/tests/api.rs` `proxy_log_*`, `proxy_env_bundle` | passed |
| T9 | `api_examples.rs`, `ApiContractTests.swift` | passed |
| T10 | `apps/cli/tests/cli.rs` `proxy_log_commands`, `proxy_on_prints_the_log_line` | passed |
| T11 | `apps/cli/tests/mcp.rs` (nine tools; `log` block; description) | passed |
| T12 | `agent_texts.rs`, `no_fixed_names.rs`, `NoFixedNamesTests.swift` | passed |
| T13 | `ProxyLogTests.swift`, `SettingsTreeTests.swift` | passed |
| T14 | `apps/daemon/src/listen.rs` tests | passed |
| T15 | `apps/daemon/src/network.rs` tests (store values; the real store read on this Mac gave `mac:18:35:d1:15:d1:a8`) | passed |
| T16 | `api.rs` `lan_networks_and_status_network`, `lan_update_from_allow_lan_true` | passed (D8) |
| E1 | `e2e.rs` `proxy_log_journey` | passed |
| E2 | `e2e.rs` `agent_loop`, with the real macOS `curl` and `python3` and `jq` | passed |

## Manual tests

M1 to M11 are not run yet. They need the app installed
(`scripts/install.sh --user --launch`), real Chrome, a second device and a
second network. M9 and M10 matter most after D1: they are the only check of
the System Configuration store on Wi-Fi, Ethernet and a VPN.
