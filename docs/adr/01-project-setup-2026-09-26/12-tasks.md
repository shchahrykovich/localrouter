# 12. Tasks

| # | Task | Depends on |
|---|---|---|
| 1 | Workspace scaffold | none |
| 2 | Route model and lookup | 1 |
| 3 | Local CA and leaf certificates | 1 |
| 4 | Request log | 1 |
| 5 | API types and examples | 1 |
| 6 | HTTP proxy | 2, 3, 4 |
| 7 | TCP byte copy | 2, 4 |
| 8 | Daemon: shared HTTP listeners and peer check | 6 |
| 9 | Daemon: store, socket API, TCP listeners, lock, pid watch | 2, 5, 7, 8 |
| 10 | CLI and MCP shim | 5, 9 |
| 11 | macOS menu bar app and packaging | 5, 9 (and decisions U2, U4) |
| 12 | Verify: E1 and manual tests | 10, 11 |
| 13 | Docs and status flip | 12 |

Tasks 2, 3, 4 and 5 can run in parallel after task 1. Tasks 6 and 7 can run in
parallel. Task 11 can run in parallel with task 10.

## 1. Workspace scaffold

**Deps:** none. **Tests:** none of its own; `cargo test --workspace` runs and passes with zero tests.

Create `Cargo.toml` (workspace, members listed one by one: `apps/daemon`,
`apps/cli`, `libs/core`), `rust-toolchain.toml`, `.gitignore`, and the three
crates with empty `lib.rs` / `main.rs`, plus the empty folder `api/examples/`.
`apps/menubar/` is created in task 11. Add `paths.rs` with the
`LOCALROUTER_HOME` override and `config.rs` with defaults (ports 80 and 443,
fallback on, LAN access off, log size 1,000). Get approval for the test
dev-dependencies listed in the test plan first. Done when `cargo build` and
`cargo test` pass on a clean checkout.

## 2. Route model and lookup

**Deps:** 1. **Tests:** T1.

`libs/core/src/routes.rs`: `Route` with `protocol` (`http`, `tcp`), label
validation with the slug hint, loopback-only targets (I9), schemes and fields
valid only for their protocol (I19), `listen_port` rules (I17), longest-match
lookup for HTTP routes with fallback on and off, and the route list for the 404
page. Done when T1 passes.

## 3. Local CA and leaf certificates

**Deps:** 1. **Tests:** T2.

`libs/core/src/tls.rs`: create the CA into `ca.tmp-<pid>/` then rename to
`ca/` (key mode `0600` at creation, I6); load; refuse to replace a damaged CA
(I7); delete leftover `ca.tmp-*`; leaf certificates for routed `.localhost`
names only, 90 days, SAN and EKU (I5); in-memory cache; a `rustls` SNI resolver.
Keep name constraints behind a build flag until M7 decides U1. Done when T2
passes.

## 4. Request log

**Deps:** 1. **Tests:** T8.

`libs/core/src/logs.rs`: fixed-size ring buffer with two entry kinds (HTTP
request, TCP connection), entries without query strings, headers or bodies
(I10), and a broadcast channel for `subscribe_logs`. Done when T8 passes.

## 5. API types and examples

**Deps:** 1. **Tests:** T9 (Rust half).

`libs/core/src/api.rs`: request and reply types for all 11 socket methods,
including the `protocol`, `listen_port` and `https_only` fields of
`register_route`; `api_version` 1.0; error type. One example file per type in
`api/examples/`, with one HTTP and one TCP example of `register_route`.
`libs/core/tests/api_examples.rs` walks the folder (I11). Done when T9 Rust half
passes.

## 6. HTTP proxy

**Deps:** 2, 3, 4. **Tests:** T5.

`libs/core/src/proxy.rs`: `hyper` server side (HTTP/1.1, HTTP/2 via ALPN),
forward to `http://` and `https://` targets with `Host` unchanged (I12) and
`X-Forwarded-*` added; for `https://` targets, TLS to the dev server without
certificate check; WebSocket upgrade; `https_only` redirect with `308`; 2-second
connect timeout; 502 and 404 pages; one log entry per request. Done when T5
passes.

## 7. TCP byte copy

**Deps:** 2, 4. **Tests:** T12 (copy cases).

`libs/core/src/tcp.rs`: for one accepted connection, connect to the target
within 2 seconds, copy bytes both ways with half-close, write one log entry at
close (bytes each way, duration, `failed`), and stop at once when the route's
cancel signal fires (I18). Done when the `tcp.rs` cases of T12 pass.

## 8. Daemon: shared HTTP listeners and peer check

**Deps:** 6. **Tests:** T4.

`apps/daemon/src/listen.rs`: bind `0.0.0.0` and `[::]` (with `IPV6_V6ONLY`)
on the configured HTTP ports; peer check before reading (I2); report each failed
bind in status instead of exiting; at most one "refused peer" log line per
second. Done when T4 passes and a manual run on ports 80/443 serves a route.

## 9. Daemon: store, socket API, TCP listeners, lock, pid watch

**Deps:** 2, 5, 7, 8. **Tests:** T3, T6, T12 (listener cases).

`store.rs`: load, atomic save, rollback of memory on a failed save (I4), owned
routes never saved (I3), broken file moved aside. `tcp_listen.rs`: bind
`127.0.0.1` and `::1` only for each TCP route (I15), bind before storing and
close again on any later failure (I16), `listen_port: 0`, close the listener and
all connections on removal (I18), `listen_failed` for a persistent route whose
port is taken at start. `socket.rs`: all 11 methods, `hello` version (I14), no
key material in any reply (I6), clear error when the socket path is longer than
104 bytes. `lock.rs`: `flock` single instance (I8). `pidwatch.rs`: kqueue
`NOTE_EXIT`. `main.rs`: start-up order (lock, config, CA, routes, shared
listeners, TCP listeners, socket) and `daemon.log` with 5 MB rotation. Done
when T3, T6 and the `tcp_listen.rs` cases of T12 pass.

## 10. CLI and MCP shim

**Deps:** 5, 9. **Tests:** T7, T10, T11.

`apps/cli`: the 10 commands, including `trust`, `untrust`, `ca-path` and
`ca reset` (untrust the old root, call `reset_ca`, offer trust again), and the
`add` options `--tcp`, `--listen <port>`, `--target <url>` and `--https-only`.
`mcp.rs`: `rmcp` stdio server with exactly six tools (I13), "not running"
error, version check (I14). The `register_route` tool description tells agents
when to use `tcp`. T11 checks that the CLI crate does not depend on the daemon
crate (I1). Done when T7, T10 and T11 pass.

## 11. macOS menu bar app and packaging

**Deps:** 5, 9, and decisions U2 (bundle id, signing) and U4 (CLI link
location). **Tests:** T9 (Swift half), M3.

`apps/menubar/`: `MenuBarExtra` with Domains (HTTP and TCP routes, with the
listen port shown for TCP), Logs, Settings, Help; `DaemonClient` with log
subscription; `Api.swift` plus `ApiContractTests.swift` walking `api/examples/`
(I11); `SMAppService` registration of the bundled LaunchAgent; Trust, Untrust
and Uninstall buttons (Uninstall runs untrust and deletes the data folders, see
the manifest's Rollback). `scripts/build-app.sh` copies both Rust binaries into
`Contents/MacOS/`. Done when `xcodebuild test` passes and M3 passes.

## 12. Verify

**Deps:** 10, 11. **Tests:** E1, M1, M2, M4, M5, M6, M7.

Write `apps/cli/tests/e2e.rs` (E1, including the TCP steps). Then run every
manual test in the test plan on a real Mac and record the results in the test
plan file. M1 includes `psql` and `redis-cli` through TCP routes. M7 decides
U1: write the outcome into [04](04-https-local-ca.md) and the manifest.

## 13. Docs and status flip

**Deps:** 12. **Tests:** M8.

Update the top-level `README.md`: the final CLI command list (adds `status`,
`untrust`, `ca-path`, `ca reset`, and the TCP options of `add`), `.localhost`
only in version 1, and the project layout link to
[08](08-components.md). Run M8 on the release build. Flip this ADR to
"Accepted, as built", append the Actual Change Manifest and the Plan vs Actual
table to the manifest file, and note any drift.

## Invariants to tasks

| Invariant | Task |
|---|---|
| I1. Only the daemon writes the data folder | 9 (store in daemon crate), 10 (T11) |
| I2. Non-loopback peers closed first | 8 |
| I3. Owned routes never saved | 9 |
| I4. Atomic replace, failed write keeps old state | 9 |
| I5. Leaf certs only for routed `.localhost` names | 3 |
| I6. `ca.key` 0600, never in replies or logs | 3, 9 |
| I7. Existing CA never replaced except by reset | 3, 10 |
| I8. One daemon per data folder | 9 |
| I9. Targets are loopback only | 2 |
| I10. Log capped, no query, headers or bodies | 4 |
| I11. Rust and Swift decode the same JSON | 5, 11 |
| I12. `Host` unchanged | 6 |
| I13. Exactly six MCP tools | 10 |
| I14. Clients refuse a different major `api_version` | 9, 10 |
| I15. TCP listeners bind loopback only | 9 |
| I16. A failed register leaves no listener | 9 |
| I17. `listen_port` unique, not HTTP ports, not the target port | 2 |
| I18. Removing a TCP route closes its connections | 7, 9 |
| I19. Fields valid only for their protocol | 2 |

Every test ID in the test plan appears above: T1 (2), T2 (3), T3 (9), T4 (8),
T5 (6), T6 (9), T7 (10), T8 (4), T9 (5, 11), T10 (10), T11 (10), T12 (7, 9),
E1 (12), M1 to M7 (11, 12), M8 (13).
