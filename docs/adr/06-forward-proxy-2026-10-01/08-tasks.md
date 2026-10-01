# 8. Tasks

**Status:** Tasks 1 to 10 are done (2026-10-01). Task 11: E1d is done, the
manual tests are not. Task 12 is open.

| # | Task | Depends on |
|---|---|---|
| 1 | Config fields, host patterns, log entry fields in core | - |
| 2 | Inspection CA in `tls.rs` | - |
| 3 | Upstream client `upstream.rs` | - |
| 4 | Forward proxy `forward.rs` | 1, 2, 3 |
| 5 | Daemon: listener, live bind, `get_proxy`, `reset_inspect_ca`, `status.proxy` | 4 |
| 6 | Socket API 1.3, examples, Swift types | 1, 5 |
| 7 | CLI `proxy` commands and trust for the second CA | 6 |
| 8 | MCP tool `get_proxy` | 6 |
| 9 | Agent texts and dictionary | 7, 8 |
| 10 | Menu bar: Proxy settings, Logs label, Open Chrome via Proxy | 6 |
| 11 | End-to-end test and manual tests | 5, 7, 8, 9, 10 |
| 12 | Decide U2, release, smoke test, ADR status | 11 |

Tasks 1, 2 and 3 can run in parallel. Tasks 7, 8 and 10 can run in parallel.

## 1. Config fields, host patterns, log entry fields in core

**Deps:** none. **Tests:** T4, T8, T13 (unit part).

`config.rs`: `proxy_enabled`, `proxy_port`, `inspect_hosts`, with
`Instance::default_proxy_port()` (8877 release, 7877 suffixed) in
`instance.rs` and the Swift `Instance.swift` rule, checked against
`api/instance-names.json`. New `inspect.rs`: `HostPattern` (exact or `*.` plus
at least two labels), `InspectSet::matches(host)`. `logs.rs`: optional `via`,
`mode`, `bytes_in`, `bytes_out` on the `Http` entry, and a constructor for a
tunnel entry that cannot take a query. Done when T4, T8 and the unit part of
T13 pass and every existing test passes unchanged.

## 2. Inspection CA in `tls.rs`

**Deps:** none. **Tests:** T5 (unit part).

`InspectCa::load_or_create` reuses the create-in-temp-then-rename code of
`LocalCa`, in `inspect-ca/`, with the common name prefix from the instance.
A second `CertStore` built with an allow function that asks the inspect set.
No change to the local CA's allow function (ADR 01 I5). Invariants I3, I4, I5.

## 3. Upstream client `upstream.rs`

**Deps:** none. **Tests:** T6.

A hyper-util client with a pool (90 s idle), 10 s connect timeout, ALPN `h2`
and `http/1.1`, and a `rustls::ClientConfig` passed in. Production builds it
with `rustls-platform-verifier`; tests pass a root store with a test CA. The
type takes no "accept any certificate" option (I6). A resolver trait, so T2
can prove `.localhost` never reaches it (I8). Add the dependency to the
workspace.

## 4. Forward proxy `forward.rs`

**Deps:** 1, 2, 3. **Tests:** T2, T3, T5 (integration), T6, T13.

`forward::serve(io, peer)`: read the first request; absolute form → send
through `upstream`; `CONNECT` → tunnel with `copy_bidirectional`, or inspect
(TLS server with the inspection `CertStore`, then hyper server with upgrades,
each request through `upstream`); `.localhost` names → `Proxy::handle`; other
forms → 400 page; own address → 508 (I9). Remove `Proxy-Authorization` and
`Proxy-Connection`; add nothing (I10). One log entry per request or tunnel
(I11). The empty hook seam for ADR 07 at the two points named in
[04](04-components.md). New `libs/core/tests/forward.rs` with the harness.

## 5. Daemon: listener, live bind, `get_proxy`, `reset_inspect_ca`, `status.proxy`

**Deps:** 4. **Tests:** T1, T5 (daemon part), T7.

`apps/daemon/src/proxy_listen.rs`: bind `127.0.0.1` and `::1` (both or
neither, like `tcp_listen.rs`), peer check, accept into `forward::serve`.
`set_config` binds before it writes and rolls back on failure (I2, I12). Create
the inspection CA when `inspect_hosts` becomes non-empty (I5). `get_proxy`
builds `env`, `chrome_args` (`--user-data-dir` under
`~/Library/Caches/LocalRouter<suffix>/chrome-proxy`, `--proxy-server`,
`--no-first-run`, `--no-default-browser-check`) and `notes`. Load the
inspection CA at start if the folder exists.

## 6. Socket API 1.3, examples, Swift types

**Deps:** 1, 5. **Tests:** T9.

`api.rs`: `API_VERSION` `1.3`, `METHODS` 13, `GetProxyResult`, `ProxyStatus`,
`InspectCaStatus`, new config fields in `SetConfigParams`. `api/examples/`:
`get_proxy.*`, `reset_inspect_ca.*`, `status.reply.json` with `proxy`,
`log.event.json` with `via` and `mode`. `Api.swift` mirrors them. The 1.2
decode check for the new log example (I16).

## 7. CLI `proxy` commands and trust for the second CA

**Deps:** 6. **Tests:** T10.

`proxy`, `on`, `off`, `port`, `env`, `chrome` (and `--print`), `inspect add|rm|list`,
`trust`, `untrust`, `ca-path`, `ca reset`. `trust.rs` takes the PEM path and
common name, so it serves both CAs. `proxy chrome` uses `chrome_args` from
`get_proxy` only (I18).

## 8. MCP tool `get_proxy`

**Deps:** 6. **Tests:** T11.

Seventh tool, no arguments, `deny_unknown_fields`. Change the `tools/list`
snapshot in `apps/cli/tests/mcp.rs` on purpose, and the MCP server doc
comment "Exactly six tools" (I14).

## 9. Agent texts and dictionary

**Deps:** 7, 8. **Tests:** T12.

`help.md`, `note.md`, `mcp.md`: a short Proxy part (I15). `docs/dictionary.md`:
forward proxy, proxy port, tunnel, inspect, inspect set, inspection CA,
upstream server; the MCP tool count; the socket method count; the log entry
fields. `CLAUDE.md`: the seven tools and ADR 06 in the design list.

## 10. Menu bar: Proxy settings, Logs label, Open Chrome via Proxy

**Deps:** 6. **Tests:** T14, M3.

Settings: Proxy section (on, port, inspect list, Trust or Untrust inspection).
Logs: a `proxy` label on `via: "proxy"` entries. New
`LocalRouterKit/ChromeLauncher.swift`: finds `com.google.Chrome` with
`NSWorkspace`, turns the proxy on if needed, opens Chrome with
`createsNewApplicationInstance` and `get_proxy.chrome_args`. Behind a small
protocol so T14 can fake `NSWorkspace`. `StatusItemController`: the
right-click item "Open Chrome via Proxy", added only when Chrome is found,
checked each time the menu opens; the popover opens with the result (I17,
I18).

## 11. End-to-end test and manual tests

**Deps:** 5, 7, 8, 9, 10. **Tests:** E1, M1, M2, M3, M4, M5.

Write E1d in `apps/cli/tests/e2e.rs` as in the test plan. Run M1 to M5 on a
real Mac and record the results in the test plan, especially whether Claude
Code reads `NODE_EXTRA_CA_CERTS` (M2).

## 12. Decide U2, release, smoke test, ADR status

**Deps:** 11. **Tests:** M6.

Decide unresolved effect U2 (who may use the proxy port) or accept it with a
note in the help text. Release with `scripts/publish.sh --minor` (0.1.x to 0.2.0). Run M6 on
the installed release. Append the Actual Change Manifest and the Plan vs
Actual table, and flip the README status.

## Invariants to tasks

| Invariant | Task |
|---|---|
| I1 loopback only | 5 |
| I2 off means no socket | 5 |
| I3 leaves only for the inspect set | 2, 4 |
| I4 key `0600`, never in a reply, not replaced | 2, 6 |
| I5 CA only on need | 2, 5 |
| I6 upstream certificate checked | 3 |
| I7 tunnel unchanged | 4 |
| I8 `.localhost` never leaves | 3, 4 |
| I9 no loop | 4 |
| I10 proxy headers removed, none added | 4 |
| I11 no query, header, body in the log | 1, 4 |
| I12 no restart; failed bind changes nothing | 5 |
| I13 examples decode both sides | 6 |
| I14 seven MCP tools | 8 |
| I15 agent texts | 9 |
| I16 older clients decode logs | 1, 6 |
| I17 Chrome item only with Chrome; own profile | 10 |
| I18 one Chrome argument list | 5, 7, 10 |

## Plan vs actual

What the build did differently from this plan, and why.

| # | Planned | Built | Why |
|---|---|---|---|
| 1 | An empty hook function for ADR 07 in `forward.rs` and `proxy.rs` | Not added | A function that does nothing cannot be tested. ADR 07 adds it with its first rule. |
| 2 | Nothing about the proxy port and ports 80/443 | `proxy_port` must differ from `http_port` and `https_port` (`invalid_request`) | On macOS a socket on `127.0.0.1:80` can bind next to the router's `0.0.0.0:80` and take its loopback traffic. |
| 3 | `set_config` binds the proxy port | It binds only when the call names `proxy_enabled` or `proxy_port` | A Settings switch (fallback, LAN) must not fail because the proxy port was taken at start. |
| 4 | The inspection leaf is chosen by SNI | It is chosen by the `CONNECT` host | The upstream connection goes to the `CONNECT` host, so the client sees a certificate for the host it is really connected to. |
| 5 | Not said | A host in the inspect set while the inspection CA is missing or broken is tunnelled | Never less safe: nothing is read, and `get_proxy` has a note. |
| 6 | Not said | `CONNECT shop.localhost:443` is logged with mode `inspect` | The router reads those requests, so they are not a tunnel. |
| 7 | `proxy chrome` opens Chrome | It turns the proxy on first, as the menu item does; `--print` changes nothing | One behaviour for the CLI and the app. |
| 8 | The Chrome profile is in `~/Library/Caches/LocalRouter<suffix>/chrome-proxy` | The same, and `<LOCALROUTER_HOME>/caches/chrome-proxy` when `LOCALROUTER_HOME` is set | Tests never touch `~/Library`. |
| 9 | E1 uses "a test-only upstream resolver" | Debug builds of the daemon read `LOCALROUTER_TEST_RESOLVE` and `LOCALROUTER_TEST_UPSTREAM_CA` | Like `LOCALROUTER_TEST_API_VERSION`: release builds ignore them. |
| 10 | A 502 page for every upstream failure | A certificate failure has its own title: "The server's certificate is not trusted" | It is not an outage, and the user should not look for one. |
| 11 | `instance-names.json` unchanged | It gained `caches_folder`, `inspect_ca_name_prefix` and `proxy_port` | The Rust and Swift copies of these names are checked against it. |
| 12 | A new file `apps/daemon/src/proxy_listen.rs` | The listener is in `daemon.rs` (`start_proxy`, `stop_proxy`) and binds with `tcp_listen::bind_loopback` | The loopback pair, both or neither, already existed for TCP routes; a second copy would drift. |
