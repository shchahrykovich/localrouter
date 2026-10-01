# 7. Test plan

**Status:** The automated tests T1 to T14 and E1d exist and pass (2026-10-01).
The manual tests M1 to M6 have not been run yet.

## What the repository can run today

| Kind | Tool | Where |
|---|---|---|
| Rust unit tests | `cargo test` | `#[cfg(test)]` modules in `libs/core/src`, `apps/daemon/src` |
| Rust integration, core | `cargo test -p localrouter-core --test …` | `libs/core/tests/` (`proxy.rs` has a harness with echo, TLS and WebSocket upstreams) |
| Real daemon over the socket | `cargo test -p localrouterd --test api` | `apps/daemon/tests/api.rs` |
| Real binaries, CLI and MCP | `cargo test -p localrouter --test …` | `apps/cli/tests/` (`cli.rs`, `mcp.rs`, `e2e.rs`; `reqwest` and `rmcp` client are dev dependencies) |
| Swift | `swift test --package-path apps/menubar` | `ApiContractTests.swift` walks `api/examples/` |
| Browser, Chrome, Claude Code | none | manual tests only |

Rules from `CLAUDE.md` that shape the plan: tests set `LOCALROUTER_HOME` to a
short temp folder and ports `0`; they never touch `~/Library`; keep a probe
socket instead of re-binding a freed port.

## Overview

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| T1 | unit + daemon | proxy port binds loopback pair only (I1) | sockets | port `0` instead of 8877 | CI |
| T2 | core integration | absolute form, 400, 508, `.localhost` to route table, proxy headers removed (I8, I9, I10) | hyper, route table, echo upstream | internet → local echo server; DNS → a resolver that panics | CI |
| T3 | core integration | tunnel copies bytes unchanged (I7) | TCP | internet → local TCP echo | CI |
| T4 | unit | host patterns and the inspect set | - | - | CI |
| T5 | unit + core integration | inspection CA: made on need, `0600`, not replaced, leaf only for inspect-set names, never on 443 (I3, I4, I5) | rcgen, rustls | - | CI |
| T6 | core integration | upstream certificate checked; self-signed upstream gives 502 (I6) | rustls | macOS trust store → a root store with a test CA | CI |
| T7 | daemon | on, off, port change without restart; failed bind changes nothing (I2, I12) | real daemon, sockets | - | CI |
| T8 | unit | config fields, instance defaults, downgrade shape | - | - | CI |
| T9 | contract | examples decode in Rust and Swift; 13 methods; no private key in replies; old log shape decodes (I4, I13, I16) | serde, Codable | - | CI |
| T10 | CLI | `proxy` commands, `env` output, `inspect add` | real binaries | `security` → not called (trust is M-tested) | CI |
| T11 | MCP | seven tools; `get_proxy` reply (I14) | real MCP shim and daemon | - | CI |
| T12 | texts | help, note, MCP text describe the proxy; no fixed names (I15) | - | - | CI |
| T13 | core integration | proxy log entries have no query, header, body (I11) | - | - | CI |
| T14 | Swift unit | Chrome launcher: hidden without Chrome; arguments equal `chrome_args`; turns the proxy on first (I17, I18) | argument building | `NSWorkspace` → a fake that records calls | CI |
| E1 | end-to-end | the journey through the real binaries | daemon, CLI, MCP, reqwest through the proxy | internet → local TLS server with a test CA; keychain → CA file passed as root | CI |
| M1 | manual | Chrome through the proxy, tunnel and inspect | everything | - | by hand |
| M2 | manual | Claude Code through the proxy | everything | - | by hand, uses the user's Claude account |
| M3 | manual | menu bar Proxy section and Logs | app | - | by hand |
| M4 | manual | failure checks a user will meet | everything | - | by hand |
| M5 | manual | agent acceptance: an agent sets up a Chrome run through the proxy | agent, MCP | - | by hand |
| M6 | manual | post-release smoke test | release build | - | by hand |

### Replaced parts and where they run for real

| Replaced in CI | Hides | Real in |
|---|---|---|
| the internet (local echo and TLS servers) | real DNS, CDNs, HTTP/2 servers, slow and large responses | M1, M2 |
| macOS trust store (test root store) | that `rustls-platform-verifier` accepts real public certificates and a company root | M1, M2 |
| keychain trust (CA file given to reqwest) | that Chrome accepts leaves from a trusted inspection CA | M1 |
| `NODE_EXTRA_CA_CERTS` | that Claude Code reads it | M2 (gap if it does not: see manifest risk) |
| `NSWorkspace` (fake in T14) | that Chrome really starts a new instance and honours `--proxy-server` | M3 |
| port 8877 | a real conflict with another program on 8877 | M4 |

## 1. Automated tests (CI)

### Unit tests, `libs/core/src`

| Test file | Case |
|---|---|
| `inspect.rs` tests (T4) | `api.example.com` matches itself, not `x.api.example.com`; `*.example.com` matches `a.example.com` and `a.b.example.com`, not `example.com`; case and port ignored; bad patterns refused (`*.com`? see note, `a*.b`, `*.*`, empty, a label with `_`); `*` matches every host outside `.localhost` (added 2026-10-01) |
| `tls.rs` tests (T5) | inspection CA created in `inspect-ca.tmp-<pid>/` then renamed; `ca.key` mode `0600`; a damaged `inspect-ca/` is reported, not replaced; inspection `CertStore` refuses a name outside the set and a `.localhost` name |
| `config.rs` tests (T8) | new fields default per instance (8877 release, 7877 `-dev`); a file without them parses; a file with them round-trips |
| `logs.rs` tests (T13) | a proxy entry built from `GET /a?token=x` stores `/a`; tunnel entry has `CONNECT`, empty path, bytes |

Note on `*.com`: the plan refuses a pattern with fewer than two labels after
`*.`, so `*.com` is refused. This is a guard against inspecting a whole
top-level domain by mistake.

### Core integration, `libs/core/tests/forward.rs` (new)

A harness like `proxy.rs`: a `forward::serve` task on a random port, an echo
upstream, a TLS upstream with a test CA, and an `upstream.rs` client built
with that test CA as its only root.

| Test file | Case |
|---|---|
| `forward.rs` (T2) | `GET http://127.0.0.1:<echo>/a?x=1` reaches the echo with `GET /a?x=1`, `Host` unchanged, no `Proxy-Connection`, no `Proxy-Authorization`, no `Via`, no `X-Forwarded-For` |
| `forward.rs` (T2) | `GET /a` (not absolute) gives 400 and the "this port is a proxy" page |
| `forward.rs` (T2) | `GET http://127.0.0.1:<own port>/` gives 508 |
| `forward.rs` (T2) | `GET http://shop.localhost/` with a route gives the dev server's answer; the resolver given to the harness panics if called |
| `forward.rs` (T3) | `CONNECT 127.0.0.1:<tcp echo>`, then 1 MB of random bytes, comes back equal; log entry has bytes |
| `forward.rs` (T5) | `CONNECT test.example:443` with `test.example` in the set: the client sees a leaf for `test.example` signed by the inspection CA; not in the set: the client's TLS reaches the upstream's own certificate |
| `forward.rs` (T5) | the 443 router `CertStore` with `test.example` in the inspect set still refuses `test.example` |
| `forward.rs` (T6) | inspected host whose upstream has a self-signed certificate: 502 page names the certificate problem; the upstream receives no HTTP bytes |
| `forward.rs` (T6) | absolute-form `https://` is not accepted (clients use CONNECT); 400 |
| `forward.rs` (T13) | after the above, every proxy log entry has no `?`, no header names, no body |

### Real daemon, `apps/daemon/tests/api.rs`

| Test file | Case |
|---|---|
| `api.rs` (T1) | `set_config proxy_enabled true, proxy_port 0`: `status.proxy.bound` is exactly `127.0.0.1:p` and `[::1]:p`, even with `allow_lan` true |
| `api.rs` (T7) | proxy off: connect to the port fails; on: works; off again: an open tunnel is closed |
| `api.rs` (T7) | port taken on `::1` only (probe socket kept): `set_config` fails `port_in_use`, `get_config` unchanged, `127.0.0.1` port free again |
| `api.rs` (T7) | routes before and after on, off, port change are equal |
| `api.rs` (T5) | fresh daemon, proxy on, no inspect host: no `inspect-ca/`; add one host: folder exists; remove it: folder stays; `reset_inspect_ca` makes a different common name |
| `api.rs` (T9) | no reply in the whole test contains `PRIVATE KEY` |

### Contract, `libs/core/tests/api_examples.rs` and `ApiContractTests.swift` (T9)

These tests already walk `api/examples/` and fail on a method without an
example, so the new methods are covered once their files exist (this test
grows by itself). New files: `get_proxy.request.json`, `get_proxy.reply.json`,
`reset_inspect_ca.request.json`, `.reply.json`, `status.reply.json` with
`proxy`, `log.event.json` variant with `via` and `mode`. One added case: the
new log example decodes with the 1.2 Rust type (I16).

### CLI, `apps/cli/tests/cli.rs` (T10)

| Case |
|---|
| `proxy on` then `proxy` prints the URL and "not trusted" note |
| `proxy env` prints exactly four `export` lines before the CA exists (`HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY` with `.localhost`, `NODE_USE_ENV_PROXY`), and five after (`NODE_EXTRA_CA_CERTS`) |
| `proxy off` prints the "programs started with HTTPS_PROXY now fail" line |
| `proxy inspect add api.example.com` twice: listed once; `rm` removes it; `add '*.com'` fails with a message |
| `proxy chrome --print` (prints the command instead of running it) shows `--proxy-server` and a profile under `Library/Caches` |

### MCP, `apps/cli/tests/mcp.rs` (T11)

| Case |
|---|
| `tools/list` has exactly the seven names (snapshot changed on purpose) |
| `get_proxy` with an unknown argument fails (`deny_unknown_fields`) |
| `get_proxy` with the proxy off returns `enabled: false` and a note; `get_config` before and after equal |

### Swift, `apps/menubar/Tests/LocalRouterKitTests/ChromeLauncherTests.swift` (T14, new)

`ChromeLauncher` takes a small protocol for "find app by bundle id" and
"open app with arguments", so the test passes a fake.

| Case |
|---|
| fake finds no `com.google.Chrome`: the menu model has no "Open Chrome via Proxy" item |
| fake finds Chrome, proxy off: the launcher calls `set_config proxy_enabled true` first, then opens Chrome |
| `set_config` fails: Chrome is not opened; the error text is returned for the popover |
| the arguments passed equal `chrome_args` of `api/examples/get_proxy.reply.json`; `createsNewApplicationInstance` is true |
| `--user-data-dir` is never Chrome's default profile folder (`Library/Application Support/Google/Chrome`) |

### Agent texts, `libs/core/tests/agent_texts.rs`, `no_fixed_names.rs` (T12)

| Case |
|---|
| `help.md`, `note.md`, `mcp.md` mention `get_proxy`, the `env` command, the trust step, "cannot change the proxy of a program that is already running", and "never write proxy settings into project files" |
| the `-dev` render names `localrouter-dev proxy env` and port 7877 |

## 2. Automated end-to-end test, `apps/cli/tests/e2e.rs` (E1d)

Through the real binaries, with checks after each step:

1. Start the daemon with a temp home and ports `0`. Start a local TLS server
   for `test.example` with a test CA, and a local plain HTTP echo.
2. `localrouter proxy on` → `localrouter proxy` shows `enabled` and a port.
   Check: `status.proxy.bound` has two loopback addresses.
3. reqwest with `Proxy::all(url)`: `GET http://127.0.0.1:<echo>/x` → 200,
   echo saw `/x`. Check: `localrouter logs` shows `via proxy`, `mode http`.
4. reqwest `GET https://test.example/` (resolved to the TLS server through a
   test-only upstream resolver), no inspect host → tunnel; the client trusts
   the test CA and gets 200. Check: one tunnel entry.
5. `localrouter proxy inspect add test.example` → `inspect-ca/` exists.
   reqwest with the inspection `ca.pem` as root: 200. Check: an `inspect`
   entry with path `/`.
6. MCP `get_proxy` → `inspect_set` contains `test.example`; `env` has
   `NODE_EXTRA_CA_CERTS` equal to the file used in step 5.
7. `localrouter proxy off` → the reqwest call fails with connection refused.

The test-only upstream resolver is a replaced part; M1 and M2 use real DNS.

## 3. Manual tests

### M1. Chrome through the proxy

| Check | Expected |
|---|---|
| `localrouter proxy on && localrouter proxy chrome` | a new Chrome window with a separate profile |
| open `https://example.com` | page loads; Logs shows one `CONNECT example.com` tunnel |
| `proxy inspect add example.com`, reload, before trust | Chrome shows a certificate error for example.com only; other sites load |
| `localrouter proxy trust` (password), reload | page loads; Logs shows `GET /` with mode inspect |
| open `https://shop.localhost` in that window | works as before (Chrome bypasses the proxy for loopback names) |
| `proxy untrust` | example.com fails again; `shop.localhost` still works (local CA untouched) |

### M2. Claude Code through the proxy

Uses the user's Claude account; a few short requests.

| Check | Expected |
|---|---|
| `eval "$(localrouter proxy env)" && claude -p "say hi"` with no inspect host | answer arrives; Logs shows tunnels to the API host |
| `proxy inspect add api.anthropic.com`, repeat | answer arrives; Logs shows `POST /v1/messages` with mode inspect. If Claude Code reports a certificate error, `NODE_EXTRA_CA_CERTS` is not read: record it as a finding |
| streamed reply in an interactive session | text appears as it streams, not all at the end |

### M3. Menu bar app

| Check | Expected |
|---|---|
| Settings → Proxy: turn on | status shows the port bound |
| add and remove an inspect host | list updates; `localrouter proxy inspect list` agrees |
| Trust inspection | macOS asks for the password; state becomes trusted |
| Logs | proxy entries show a `proxy` label; filter by host works |
| right-click the icon, Chrome installed | the menu has "Open Chrome via Proxy" |
| click it with the proxy off | proxy turns on; a new Chrome window opens; popover says "Chrome started with the proxy at 127.0.0.1:8877" |
| in that window open `https://example.com` | Logs shows a `CONNECT example.com` entry |
| the user's normal Chrome, open `https://example.com` | no new Logs entry: the normal Chrome does not use the proxy |
| click the item again | a new window in the same proxy Chrome, not a third Chrome process |
| in the proxy window open `https://shop.localhost` | works; no proxy entry (Chrome bypasses loopback names) |
| rename `Google Chrome.app` away (or test on a Mac without it), right-click | no "Open Chrome via Proxy" item |

### M4. Failure checks

| Check | Expected |
|---|---|
| another program on 8877, `proxy on` | error names port 8877 and says it is taken; setting stays off |
| `HTTPS_PROXY` set, daemon stopped, `curl https://example.com` | curl fails "connection refused"; `localrouter status` explains the daemon is down |
| inspected host with an expired certificate (`expired.badssl.com`) | 502 page from LocalRouter that says the certificate expired |
| corporate VPN with its own DNS | inspected and tunnelled hosts resolve as without the proxy |

### M5. Agent acceptance

Follow the session in [00-working-backwards.md](00-working-backwards.md):
ask Claude Code "run my Playwright test through the LocalRouter proxy and
show me the API calls". Expected: the agent calls `get_proxy`, turns the
proxy on with the CLI, starts the browser with `chrome_args` or `env`, and
reads `get_logs`. It does not try to change its own proxy, and it writes no
proxy setting into the project. Also check one Node.js test runner with
`NODE_USE_ENV_PROXY=1`: its `fetch` calls appear in Logs.

### M6. Post-release smoke test

No paid calls. On the installed release: `localrouter proxy on`,
`curl -x http://127.0.0.1:8877 http://example.com -o /dev/null -w '%{http_code}'`
prints `200`; `localrouter proxy off`.

## Not in this plan without approval

| Tool | Would cover | Cost |
|---|---|---|
| Browser automation in CI (Playwright) | Chrome with the proxy flag and keychain trust | a new dev dependency and a browser download in CI; trust needs a keychain the CI user can write |

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| I1 loopback only | T1 | - |
| I2 off means no socket | T7 | M3 |
| I3 leaves only for the inspect set; 443 never uses it | T5 | M1 |
| I4 key `0600`, never in a reply, not replaced | T5, T9 | - |
| I5 CA only on need | T5 | - |
| I6 upstream certificate checked | T6 | M4 |
| I7 tunnel unchanged | T3 | M1 |
| I8 `.localhost` never leaves | T2 | M1 |
| I9 no loop | T2 | - |
| I10 proxy headers removed, none added | T2 | - |
| I11 no query, header, body in log | T13 | - |
| I12 no restart; failed bind changes nothing | T7 | M4 |
| I13 examples decode both sides | T9 | - |
| I14 seven MCP tools | T11 | M5 |
| I15 agent texts | T12 | M5 |
| I16 old clients decode logs | T9 | - |
| I17 menu item only with Chrome; own profile | T14 | M3 |
| I18 one Chrome argument list | T10, T14 | M3 |
