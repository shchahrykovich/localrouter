# 8. Tasks

Each task merges with its tests. Test IDs are from the
[test plan](08-test-plan.md).

| # | Task | Depends on |
|---|---|---|
| 1 | Shared secret header list (`secrets.rs`) | none |
| 2 | HAR entries (`har/entry.rs`) | 1 |
| 3 | HAR writer (`har/writer.rs`, `har/mod.rs`) | 2 |
| 4 | Config fields and socket API 1.5 (Rust, Swift, examples) | none |
| 5 | Record in the forward proxy and the router | 3 |
| 6 | Reserved host `proxy`, certificate, saved route kept | none |
| 7 | The viewer (`har/viewer.rs` and its files) | 3, 6 |
| 8 | Daemon wiring | 3, 4, 5, 7 |
| 9 | CLI `proxy log` and the MCP description | 8 |
| 10 | Agent texts and fixed names | 8 |
| 11 | Menu bar app | 4, 8 |
| 12 | End-to-end tests and manual tests | 9, 10, 11, 14, 15 |
| 13 | Docs, release, ADR status | 12 |
| 14 | LAN access per network | 4 |
| 15 | CA bundle and the new env names | 4 |

Tracks that can run side by side: 1 → 2 → 3; 4 then 14 and 15; 6.

## 1. Shared secret header list

**Deps:** none. **Tests:** T3.

Move `DEFAULT_SECRET_HEADERS` and the merge with `secret_headers` from
`scripts/lua_api.rs` and `Scripts::secrets` into `libs/core/src/secrets.rs`.
Scripts read it from there. Done when the existing scripts tests pass
unchanged and T3's list cases pass.

## 2. HAR entries

**Deps:** 1. **Tests:** T1, T3.

`HarRecord` (what the network task copies: times, method, URL with query,
versions, both header maps, status, mode, route, scripts, tunnel bytes) and
`HarRecord::to_entry(&secrets) -> serde_json::Value` with the fields of
[change 1](01-har-files.md#what-one-entry-holds). `Proxy-Authorization` is
left out; secret headers are `[redacted]`. Done when T1 and T3 pass.

## 3. HAR writer

**Deps:** 2. **Tests:** T2, T5 (the queue part), T17.

`HarLog` in `har/mod.rs`: the atomic on flag, limits, a `sync_channel(4096)`
sender, `dropped`, a `broadcast` live feed, `record()` that does nothing when
off. `har/writer.rs`: the `har-writer` thread; file header and closing line;
the positioned write under the file lock; roll at either limit; prune to 5 by
the name pattern; repair of the newest earlier file at start; a new file when
the current one is gone; stop on a write error and keep the text. Resource
rule: the thread and its queue start at the first record and end after 30 s
without records, closing the file; records are boxed. The repair runs once at
daemon start, outside the thread. A test hook pauses the thread (for T5) and
shortens the idle time (for T17). Done when T2 and T17 pass, including the
foreign files, the cut file, the deleted file and the read-only folder.

## 4. Config fields and socket API 1.5

**Deps:** none. **Tests:** T8 (fields), T9.

`config.rs`: three fields with defaults; `SetConfigParams` with range checks
(1 to 200 MB, 100 to 1,000,000 requests); `GetProxyResult.log`;
`API_VERSION` "1.5". `api/examples/`: `set_config_proxy_log.request.json`,
updated `get_config.reply.json` and `get_proxy.reply.json`. `Api.swift`: the
same, every new field optional. Done when both contract tests pass.

## 5. Record in the forward proxy and the router

**Deps:** 3. **Tests:** T4, T5.

`forward.rs`: copy the request headers before `through_scripts`, only when
the log is on; record in `log_http`, for tunnels when they close, and for the
proxy's own error answers. `proxy.rs`: record requests with `via`; skip
`router` and `proxy` hosts. Done when T4 and T5 pass.

## 6. Reserved host `proxy`, certificate, saved route kept

**Deps:** none. **Tests:** T7.

`PROXY_LOG_HOST` in `routes.rs`; `validate` refuses it for new routes and
`script_rules::validate` for rules; the local `CertStore` always issues
`proxy.localhost`. `Daemon::load` keeps a saved route `proxy` and adds the
problem text to status. Done when T7 passes.

## 7. The viewer

**Deps:** 3, 6. **Tests:** T6, T17 (live channel), M2.

`har/viewer.rs`: the paths of [change 2](02-viewer.md#how-the-content-is-served)
under `proxy.localhost` and `router.localhost/proxy-log/`, including
`/api/entries` (read from the end); files streamed in 64 KB chunks; the
current file by length under the lock plus the closing line; the live channel
only while a page is open; loopback only;
`GET`/`HEAD` only; name and link checks; the security headers; the live feed
with `entry`, `file`, `off`, `lagged` and keep-alive. `viewer.html`,
`viewer.js`, `viewer.css` with `include_str!`: state line, file list, table,
filter, errors only, details, live rows, Download HAR with the import hint;
relative URLs only. `proxy.rs`: answer both addresses before the route
lookup; never push them to `RequestLog`. Done when T6 passes and M2 is ticked.

## 8. Daemon wiring

**Deps:** 3, 4, 5, 7. **Tests:** T8.

`daemon.rs`: make the folder from `Paths.logs`, start `HarLog` and the writer
from `config.json`; give it to `ForwardProxy` and `Proxy`; apply `set_config`
live (flag, limits, `secret_headers`); fill `get_proxy.log`. Done when T8
passes.

## 9. CLI `proxy log` and the MCP description

**Deps:** 8. **Tests:** T10, T11.

`apps/cli/src/proxy.rs`: `proxy log`, `on`, `off`, `limits`, `open`, `path`;
the `Log:` line in `proxy` and `proxy on`. `mcp.rs`: the `get_proxy`
description. No new tool. Done when T10 and T11 pass.

## 10. Agent texts and fixed names

**Deps:** 8. **Tests:** T12.

`help.md`, `note.md`, `mcp.md`: the "Proxy log" part with `{{PROXY_LOG_FOLDER}}`,
`{{PROXY_LOG_URL}}` and the `jq` recipes; `help.rs` fills them. Add
`proxy.localhost` to both fixed-name word lists. Done when T12 passes.

## 11. Menu bar app

**Deps:** 4, 8. **Tests:** T13, M3.

`ProxyLog.swift` (browser choice with an injected installed check, state
text). Settings → Proxy: the "Proxy log" section. `SettingsTree` keywords.
Proxy tab: the log line and two buttons. Right-click menu: Open Proxy Log,
Show Proxy Log Folder. Controls hidden when `log` is missing. Done when T13
passes and M3 is ticked.

## 12. End-to-end test and manual tests

**Deps:** 9, 10, 11, 14, 15. **Tests:** E1, E2, M1, M2, M3, M4, M5, M6, M7, M8, M9, M10, M11.

Write `proxy_log_journey` and `agent_loop` in `apps/cli/tests/e2e.rs`. Run every manual test on
the dev instance (`scripts/install.sh --user --launch`) and write the results
into the test plan.

## 13. Docs, release, ADR status

**Deps:** 12. **Tests:** the post-release smoke test.

`README.md` (Proxy tab, Settings, Proxy log), `docs/dictionary.md` (proxy log,
HAR file, viewer), `CLAUDE.md` (the ADR list and one "easy to get wrong" line:
the viewer is loopback only and read only). Release with `scripts/publish.sh`
after the user agrees, then the smoke test. Set this ADR's status to as-built,
append the Actual Change Manifest and the Plan vs Actual table.

## 14. LAN access per network

**Deps:** 4. **Tests:** T14, T15, T16, M9, M10.

`apps/daemon/src/network.rs`: the network id of an interface from
`getifaddrs`, the routing table and the ARP table by `sysctl`, with pure
parsing functions and recorded fixtures; no program, no cache. `listen.rs`:
`peer_allowed(peer, local_addr, &config)` looks up the network only for a
non-loopback peer with `allow_lan` on. `config.rs` and `api.rs`:
`lan_networks`, `status.network`. `daemon.rs`: the one-time update from
`allow_lan: true`, kept in memory if the write fails; the status note.
`LOCALROUTER_TEST_NETWORK` in debug builds. `apps/cli/src/lan.rs`: `lan`,
`allow`, `deny`, `forget`. App: the Routing page list. Done when T14, T15 and
T16 pass and M9 is ticked.

## 15. CA bundle and the new env names

**Deps:** 4. **Tests:** T8 (`proxy_env_bundle`), E2.

`inspect-ca/bundle.pem` from `/usr/bin/security find-certificate -a -p
/System/Library/Keychains/SystemRootCertificates.keychain` plus the inspection
CA, written with the CA, when it changes, and at start when older than 30
days. `get_proxy.env` gains `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`,
`CURL_CA_BUNDLE`, `http_proxy`, `https_proxy`, `no_proxy`. The agent texts
list the agent loop of [change 3](03-clients-and-agents.md#the-agent-loop-operate-run-inspect).
Done when `proxy_env_bundle` and E2 pass.

## Invariants to tasks

| Invariant | Task |
|---|---|
| I1 log off: no record | 3, 5 |
| I2 never delays traffic | 3, 5 |
| I3 valid JSON always | 3, 7 |
| I4 secrets redacted | 1, 2 |
| I5 prune only own files | 3 |
| I6 file limits | 3, 4 |
| I7 viewer not logged | 5, 7 |
| I8 loopback only | 7 |
| I9 only HAR files served | 7 |
| I10 read only, no CORS, CSP | 7 |
| I11 reserved name, old route kept | 6 |
| I12 nine MCP tools | 9 |
| I13 no fixed viewer name | 10 |
| I14 tests in temp homes | 3, 8 |
| I15 write error stops cleanly | 3 |
| I16 new file per run, repair | 3 |
| I17 LAN only on allowed networks | 14 |
| I18 update never means every network | 14 |
| I19 proxy port, TCP, viewer this Mac only | 7, 14 |
| I20 no program, no permission, no cache for the network id | 14 |
| I21 idle holds nothing | 3, 7 |
| I22 no whole file in memory | 7 |
| I23 the CA bundle adds, never replaces | 15 |
