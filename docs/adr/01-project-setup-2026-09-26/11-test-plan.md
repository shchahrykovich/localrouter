# 11. Test plan

## What the repository can run today

Nothing. The repository has only `README.md` and this ADR. There is no test
framework, no CI and no build.

This plan uses only the test tools that come with the two toolchains:

| Tool | Comes with | Used for |
|---|---|---|
| `cargo test` | Rust | all Rust unit, integration and end-to-end tests |
| XCTest (`xcodebuild test`) | Xcode | the Swift half of the contract test |

It also needs these Rust **dev-dependencies** (test-only libraries). They are
new, so they need your approval before task 1:

| Crate | Why |
|---|---|
| `tempfile` | a fresh data folder per test (`LOCALROUTER_HOME`) |
| `reqwest` (rustls, http2) | HTTP client with a custom root CA and custom name lookup |
| `tokio-tungstenite` | WebSocket client for the upgrade test |
| `rmcp` client feature | a real MCP client for the MCP tests |

## Overview

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| T1 | unit | route validation, longest match, fallback, 404, loopback-only targets | `routes.rs` | nothing | `cargo test` |
| T2 | unit | CA creation, key mode, leaf fields, name refusal, damaged CA left alone | `tls.rs`, real files in a temp folder | keychain (not touched) | `cargo test` |
| T3 | integration | atomic save, failed save keeps old state, owned routes never saved, bad file moved aside | `store.rs`, real files | disk-full (simulated by a read-only folder) | `cargo test` |
| T4 | unit | peer filter for IPv4, IPv6, IPv4-mapped IPv6, `allow_lan` | `listen.rs` filter function | real LAN peer (covered by M4) | `cargo test` |
| T5 | integration | forwarding, `Host` unchanged, `X-Forwarded-*`, WebSocket, 502, HTTP/2 over TLS | `proxy.rs`, real upstream server | ports 80/443 (random high ports) | `cargo test` |
| T6 | integration | socket API, single instance, owned route removal on process exit, version check, no key in replies | real `localrouterd` binary | ports 80/443, SMAppService launch | `cargo test` |
| T7 | integration | MCP tools list, each tool, error when daemon is down, version check | real `localrouter mcp` + real daemon | Claude Code (covered by M2) | `cargo test` |
| T8 | unit | request log cap, no query strings, no headers | `logs.rs` | nothing | `cargo test` |
| T9 | contract | Rust and Swift decode every file in `api/examples` | both type sets | the socket (covered by T6, M3) | `cargo test`, `xcodebuild test` |
| T10 | integration | CLI commands and their output | real `localrouter` + daemon | nothing | `cargo test` |
| T12 | integration | TCP routes: byte copy, loopback-only bind, port taken, removal closes connections | `tcp.rs`, `tcp_listen.rs`, real echo server | nothing | `cargo test` |
| T11 | architecture | `apps/cli` does not depend on `apps/daemon` | `cargo metadata` | nothing | `cargo test` |
| E1 | end-to-end | full journey: register by MCP, list by CLI, HTTPS request, TCP route, log, remove, 404 | both binaries, real TLS, real upstream | macOS resolver, keychain trust, ports 80/443, SMAppService | `cargo test --test e2e` |
| M1 | manual | real names in real browsers and tools | everything | nothing | by hand |
| M2 | manual | acceptance with Claude Code | everything | nothing | by hand |
| M3 | manual | menu bar UI | app + daemon | nothing | by hand |
| M4 | manual | failure cases a user will meet | everything | nothing | by hand |
| M5 | manual | idle memory | release builds | nothing | by hand |
| M6 | manual | uninstall leaves nothing behind | everything | nothing | by hand |
| M7 | manual | name-constrained CA (U1) | everything | nothing | by hand |
| M8 | manual | release smoke test | release app | nothing | by hand |

**Replaced parts and who covers them for real:**

| Replaced in CI | Why | Covered for real by |
|---|---|---|
| Ports 80/443 → random high ports (`0` in config) | tests must run in parallel and not clash with the user's own daemon | M1, M4 |
| macOS `*.localhost` resolution → `reqwest` `resolve()` override | keeps the test independent of OS behaviour | M1 |
| Keychain trust → pass `ca.pem` to the client as its root | CI must not change the keychain | M1, M6 |
| SMAppService launch → spawn `localrouterd` directly | no app bundle in `cargo test` | M3, M8 |
| Claude Code → `rmcp` client | no agent in CI | M2 |

## Automated tests (run by `cargo test`)

All Rust tests run with one command from the repository root:
`cargo test --workspace`

### Core library: `libs/core/src/*.rs` (unit, in `#[cfg(test)]` modules)

| Test file | Case |
|---|---|
| `routes.rs` | T1: valid and invalid labels (`feat/login` refused, error proposes `feat-login`); upper case stored lower |
| `routes.rs` | T1: exact match; fallback `feat-other.shop` → `shop`; fallback off → none; `blog` → none |
| `routes.rs` | T1: target `http://192.168.1.5:3000` refused; `http://[::1]:3000` accepted |
| `routes.rs` | T1: `https://127.0.0.1:5173` accepted for http; `tcp://127.0.0.1:5432` accepted for tcp; each scheme refused for the other protocol |
| `routes.rs` | T1: tcp route without `listen_port` refused; `listen_port` 80, 443, a port used by another route, or equal to the target port refused; `https_only` on a tcp route refused |
| `tls.rs` | T2: new CA → `ca.key` mode is `0600` right after creation |
| `tls.rs` | T2: leaf for `feat.shop.localhost` has SAN = that name, EKU serverAuth, validity 90 days |
| `tls.rs` | T2: leaf for `example.com` refused; leaf for a `.localhost` name without a route refused |
| `tls.rs` | T2: `ca/` with a missing `ca.key` → HTTPS off, files unchanged, no new CA |
| `tls.rs` | T2: leftover `ca.tmp-123/` is deleted on load |
| `logs.rs` | T8: 1,001 entries into a 1,000 buffer keep the newest 1,000 |
| `logs.rs` | T8: request `/cb?token=abc` is stored as path `/cb` |

### Core library integration: `libs/core/tests/`

| Test file | Case |
|---|---|
| `proxy.rs` | T5: GET forwarded, body and status unchanged |
| `proxy.rs` | T5: upstream sees `Host: feat.shop.localhost` and `X-Forwarded-For`, `-Proto`, `-Host` |
| `proxy.rs` | T5: WebSocket upgrade, echo one message both ways |
| `proxy.rs` | T5: upstream port closed → 502 page contains target and note |
| `proxy.rs` | T5: TLS client offering `h2` gets HTTP/2 |
| `proxy.rs` | T5: `https://` target with a self-signed certificate is forwarded |
| `proxy.rs` | T5: `https_only` route: plain HTTP gets `308` to the `https://` URL |
| `tcp.rs` | T12: bytes copied both ways; half-close from the client reaches the target |
| `tcp.rs` | T12: target port closed → client connection closed within 2 s, log entry marked `failed` |
| `api_examples.rs` | T9: decode every file in `api/examples/` into its Rust type; the test walks the folder, so a new example is covered without an edit |

### Daemon: `apps/daemon/`

| Test file | Case |
|---|---|
| `src/listen.rs` | T4: `127.0.0.1`, `127.8.8.8`, `::1`, `::ffff:127.0.0.1` accepted; `192.168.1.5`, `::ffff:10.0.0.1`, `fe80::1` refused; all accepted with `allow_lan` |
| `src/store.rs` | T3: save writes `routes.json` via temp + rename; file is valid JSON sorted by host |
| `src/store.rs` | T3: data folder made read-only → register fails, memory table unchanged, old file unchanged |
| `src/store.rs` | T3: owned route is not in `routes.json`; `persistent` + `owner_pid` refused |
| `src/store.rs` | T3: broken `routes.json` → moved to `routes.json.bad-<time>`, daemon starts with no persistent routes, `status` reports it |
| `src/tcp_listen.rs` | T12: every TCP route socket is bound to `127.0.0.1` or `::1`, also with `allow_lan` on |
| `src/tcp_listen.rs` | T12: port already taken on `::1` only → register fails and the `127.0.0.1` socket is closed again |
| `src/tcp_listen.rs` | T12: `unregister_route` closes the listener and an open client connection |
| `src/tcp_listen.rs` | T12: `listen_port: 0` → reply holds the real port, and it accepts connections |
| `tests/api.rs` | T6: `hello` returns `api_version` 1.x |
| `tests/api.rs` | T6: register, list, unregister round trip |
| `tests/api.rs` | T6: second daemon on the same folder exits non-zero with "already running" |
| `tests/api.rs` | T6: spawn `sleep 30`, register with its pid, kill it, route gone within 1 s |
| `tests/api.rs` | T6: no reply of any method contains `PRIVATE KEY` |
| `tests/api.rs` | T6: `LOCALROUTER_HOME` path longer than 104 bytes → clear error naming the limit |

### CLI and MCP: `apps/cli/tests/`

| Test file | Case |
|---|---|
| `mcp.rs` | T7: `tools/list` returns exactly the six tools (snapshot of names and input schemas) |
| `mcp.rs` | T7: each tool called once against a test daemon |
| `mcp.rs` | T7: no daemon → every tool returns "LocalRouter is not running" |
| `mcp.rs` | T7: daemon with api_version 2.0 (test flag) → shim refuses with an update message |
| `cli.rs` | T10: `add`, `list`, `rm`, `status`, `ca-path` output |
| `cli.rs` | T11: `cargo metadata` shows no dependency from `localrouter-cli` to `localrouterd` |

### Swift: `apps/menubar/LocalRouterTests/`

| Test file | Case |
|---|---|
| `ApiContractTests.swift` | T9: decode every file in `../api/examples/` into its Swift type |

Command: `xcodebuild test -project apps/menubar/LocalRouter.xcodeproj -scheme LocalRouter`

### Test data

| Fixture | Used by | Why |
|---|---|---|
| Fresh temp data folder per test | T2, T3, T6, T7, T10, E1 | tests run in parallel and never touch the real `~/Library` |
| Routes `shop` and `feat-login.shop` | T1, T5, E1 | proves both the exact match and the fallback |
| A third name `feat-other.shop` with **no** route | T1, E1 | the fallback case must return `shop`, not 404 |
| `sleep 30` child process | T6 | a real pid that the test can kill |
| Tiny upstream server (hyper) that echoes the request headers | T5, E1 | lets the test see exactly what the dev server receives |

## Automated end-to-end test: E1

File: `apps/cli/tests/e2e.rs`. Command: `cargo test --test e2e`

1. Build nothing extra: use the test binaries `localrouterd` and `localrouter`.
2. Start `localrouterd` with a temp `LOCALROUTER_HOME` and ports `0`.
   **Check:** `localrouter status` shows both ports and "CA created".
3. Start the echo upstream on a random port.
4. Through an `rmcp` client on `localrouter mcp`, call `register_route` with
   `host = "shop"` and the upstream port.
   **Check:** the reply holds `https://shop.localhost` and `replaced: false`.
5. Run `localrouter list`. **Check:** `shop` is listed with `upstream_up: true`.
6. Send `GET https://feat-login.shop.localhost:<port>/hello?x=1` with `reqwest`,
   `ca.pem` as the only root, and name lookup forced to `127.0.0.1`.
   **Check:** status 200, and the echoed `Host` is `feat-login.shop.localhost`
   (fallback to `shop`).
7. Call `get_logs`. **Check:** one entry, path `/hello`, no `x=1`.
8. Call `unregister_route` for `shop`, then repeat step 6.
   **Check:** the TLS handshake is refused (no route, so no certificate).
9. Repeat step 6 over plain HTTP. **Check:** 404 page.
10. Start a TCP echo server and register `db.shop` with `protocol = "tcp"`
    and `listen_port = 0`. **Check:** the reply holds the real listen port.
11. Connect to `127.0.0.1:<listen port>`, send `ping`. **Check:** `ping` comes
    back, and `get_logs` shows one TCP entry after the connection closes.
12. Unregister `db.shop`. **Check:** a new connection to the port is refused.

## Manual tests

Build first: `cargo build --release && ./scripts/build-app.sh`

### M1. Real names on the real Mac

| Check | Expected |
|---|---|
| Open the app, click Trust, enter password | `localrouter status` says "CA trusted" |
| `npm create vite@latest demo` then `npm run dev`, then `localrouter add demo 5173` | route listed, status dot green |
| Open `https://demo.localhost` in Safari, Chrome | page loads, lock icon, no warning |
| Firefox with `security.enterprise_roots.enabled` | page loads, no warning |
| Edit a file in the Vite project | page updates without reload (WebSocket through the proxy works) |
| `curl https://demo.localhost` | 200 |
| `docker run -d -p 55001:5432 -e POSTGRES_PASSWORD=x postgres` then `localrouter add db.demo 55001 --tcp --listen 15432`, then `psql -h db.demo.localhost -p 15432 -U postgres` | `psql` connects |
| `redis-server --port 56379` then a TCP route on 16379, then `redis-cli -h cache.demo.localhost -p 16379 ping` | `PONG` |
| Vite with `@vitejs/plugin-basic-ssl` on 5174, route with target `https://127.0.0.1:5174` | page loads over `https://`, no warning from LocalRouter's certificate |
| `NODE_EXTRA_CA_CERTS="$(localrouter ca-path)" node -e "fetch('https://demo.localhost').then(r=>console.log(r.status))"` | `200` |

### M2. Acceptance with Claude Code

| Check | Expected |
|---|---|
| `claude mcp add localrouter -- localrouter mcp`, then ask: "start a Vite app and give it a local domain" | agent calls `find_free_port`, starts the server, calls `register_route` with a note, and prints the URL |
| Ask: "make a worktree for branch feat/login and give it its own domain" | agent registers `feat-login.<project>`; both URLs work at the same time |
| Look at Domains in the menu bar | both routes, each with the agent's note |
| Stop the worktree's dev server started with `owner_pid` | its route disappears within about 1 s |

### M3. Menu bar UI

| Check | Expected |
|---|---|
| Launch the app | icon next to the clock, no Dock icon, no window |
| Domains | every route with status dot, note, "Open", "Copy URL", "Remove" |
| Logs | new requests appear live; filter by host works |
| Settings | toggles for fallback and LAN access take effect without restart; Trust and Uninstall buttons work |
| Help | shows the `claude mcp add` line and the Firefox and Node steps |
| Quit the app | routes keep working (daemon still runs) |

### M4. Failure cases

| Check | Expected |
|---|---|
| `python3 -m http.server 80` running before LocalRouter starts | status and menu bar show "port 80 in use", HTTPS on 443 still works |
| Request from a phone on the same Wi-Fi to `http://<mac-ip>/` | connection closed, one line in `daemon.log` |
| macOS firewall on, answer "Deny" to the prompt | loopback requests still work |
| Dev server stopped | 502 page with the target and note |
| `localrouter add feat/login.shop 5174` | error that proposes `feat-login.shop` |
| Quit the daemon, then call an MCP tool | "LocalRouter is not running..." |
| Open a route before clicking Trust | browser certificate warning (expected) |

### M5. Idle memory

| Check | Expected |
|---|---|
| `ps -o rss= -p $(pgrep localrouterd)` after 10 minutes idle with 5 routes | under 20 MB (target) |
| same for `LocalRouter` app, menu closed | under 50 MB (target) |

### M6. Uninstall

| Check | Expected |
|---|---|
| Settings → Uninstall, then delete the app | `security find-certificate -c "LocalRouter" ~/Library/Keychains/login.keychain-db` finds nothing trusted; data folders gone; `pgrep localrouterd` empty |

### M7. Name-constrained CA (decides U1)

| Check | Expected |
|---|---|
| Build with name constraints on, `localrouter ca reset`, Trust | M1 passes in Safari, Chrome, Firefox, curl, Node |
| Try to use the CA to sign `example.com` (test tool) and open it | every client rejects it |

### M8. Release smoke test

No deploy exists; this runs on each release build. It makes no network calls.

| Check | Expected |
|---|---|
| Fresh user account, install app, open it | icon appears, `localrouter status` OK |
| Register one route, open it over HTTPS after Trust | page loads |

## Not in this plan without approval

| Tool | Would cover | Cost |
|---|---|---|
| GitHub Actions on macOS runners | runs `cargo test` and `xcodebuild test` on every push | macOS runner minutes are billed for private repositories |
| Playwright | automates the browser part of M1 | a Node toolchain in a Rust/Swift repo |
| XcodeGen | generates the Xcode project from YAML, so the project file is readable in reviews | one more tool to install |

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| I1. Only the daemon writes the data folder | T3, T11 | |
| I2. Non-loopback peers closed first | T4 | M4 |
| I3. Owned routes never saved; persistent + pid refused | T3 | M2 |
| I4. Atomic replace, failed write keeps old state | T3 | |
| I5. Leaf certs only for routed `.localhost` names, correct fields | T2, E1 | M1 |
| I6. `ca.key` mode 0600, never in replies or logs | T2, T6 | |
| I7. Existing CA never replaced except by reset | T2 | M7 |
| I8. One daemon per data folder | T6 | |
| I9. Targets are loopback only | T1 | |
| I10. Log capped, no query, headers or bodies | T8, E1 | |
| I11. Rust and Swift decode the same JSON | T9 | M3 |
| I12. `Host` unchanged | T5, E1 | M1 |
| I13. Exactly six MCP tools | T7 | M2 |
| I14. Clients refuse a different major `api_version` | T6, T7 | |
| I15. TCP listeners bind loopback only | T12 | |
| I16. A failed register leaves no listener | T12 | |
| I17. `listen_port` unique, not HTTP ports, not the target port | T1 | |
| I18. Removing a TCP route closes its connections | T12, E1 | |
| I19. Fields valid only for their protocol | T1 | |
