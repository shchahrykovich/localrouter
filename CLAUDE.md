# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

LocalRouter is a macOS menu bar app that gives local dev servers names instead
of ports: `https://feat-login.shop.localhost` for HTTP, `db.shop.localhost:15432`
for TCP. Coding agents manage routes through an MCP server. Design and
decisions: `docs/adr/01-project-setup-2026-09-26/` (architecture),
`docs/adr/02-distribution-and-self-update-2026-09-26/` (release and updater),
`docs/adr/03-path-routes-2026-09-26/` (several dev servers on one name, by path),
`docs/adr/05-folder-routes-2026-09-30/` (a folder served with no dev server),
`docs/adr/06-forward-proxy-2026-10-01/` (a forward proxy for Chrome and Claude Code),
`docs/adr/07-proxy-scripts-lua-2026-10-01/` (Lua scripts that change or record traffic) and
`docs/adr/08-proxy-har-log-2026-10-02/` (the proxy log in HAR files, its viewer at
`proxy.localhost`, LAN access per network) and
`docs/adr/09-proxy-clients-2026-10-02/` (one more proxy port per client, `_client`
in the log, `proxy.localhost/<client>`) and
`docs/adr/10-phone-proxy-qr-2026-10-06/` (proposed, not built: a LAN proxy client
for an iPhone, with a password and a QR code in the Proxy tab).
Use the words defined in `docs/dictionary.md` (route, host key, target, listen
port, owned/session/persistent route) in code, docs and UI text.

## Commands

```
cargo test --workspace                                   # all Rust tests
cargo clippy --workspace --all-targets                   # expected: no warnings
swift test --package-path apps/menubar                   # Swift tests
swift build --package-path apps/menubar                  # build the menu bar app only
scripts/build-app.sh                                     # build/LocalRouter.app, ad-hoc signed
scripts/install.sh --user --launch                       # build and install LocalRouter-dev.app to ~/Applications
scripts/publish.sh [--patch|--minor|--major|--set X.Y.Z] # release to GitHub (needs .env.notarize)
```

Single tests:

```
cargo test -p localrouter-core --lib routes::tests::lookup_exact_fallback_and_none
cargo test -p localrouter-core --test proxy websocket
cargo test -p localrouterd --bin localrouterd listen::tests     # daemon unit tests live in the binary crate
cargo test -p localrouterd --test api tcp_route
cargo test -p localrouter --test e2e                             # end-to-end journey E1
swift test --package-path apps/menubar --filter UpdaterTests
```

Run the daemon by hand without touching `~/Library`:

```
H=$(mktemp -d) && printf '{"version":1,"http_port":0,"https_port":0}' > $H/config.json && LOCALROUTER_HOME=$H cargo run -p localrouterd
LOCALROUTER_HOME=$H cargo run -p localrouter -- status
```

## Architecture

Three programs, one owner of state:

```
Claude Code ──MCP stdio──► localrouter mcp ─┐
localrouter CLI ────────────────────────────┼─ JSON lines, Unix socket daemon.sock ─► localrouterd ─► routes.json, config.json, ca/
LocalRouter.app (Swift) ────────────────────┘                                          │
browser :80/:443 (by name), TCP clients :<listen_port> (by port) ──────────────────────┘──► dev servers on loopback
```

- `libs/core` (Rust library): route model and lookup (`routes.rs`), HTTP/HTTPS
  proxy on hyper (`proxy.rs`), folder routes served from disk with
  tower-http's `ServeFile` (`folder.rs`), TCP byte copy (`tcp.rs`), local CA and per-name
  leaf certificates chosen by SNI (`tls.rs`), request log ring buffer
  (`logs.rs`), socket API types (`api.rs`). The forward proxy (ADR 06):
  `forward.rs` (absolute form, `CONNECT` tunnel or inspect), `upstream.rs`
  (the only code that connects to other machines), `inspect.rs` (host
  patterns). No I/O at start; testable alone.
- `apps/daemon` (`localrouterd`): the **only writer** of the data folder. Binds
  the shared HTTP ports, one loopback listener pair per TCP route, serves the
  socket API (`socket.rs` dispatch → `daemon.rs` methods), watches owner pids
  with kqueue. Every mutation takes the `write` lock and rolls back memory if
  the file write fails.
- `apps/cli` (`localrouter`): stateless client. `localrouter mcp` is the MCP
  server (rmcp); it opens a new socket connection per tool call and never
  starts a daemon. `apps/cli` must not depend on `apps/daemon` (a test checks
  `cargo metadata`).
- `apps/menubar`: Swift package. `LocalRouterKit` holds the Swift copy of the
  API types, the socket client, the GitHub self-updater, and the CLI and Claude
  Code installers; `LocalRouter` is the menu bar app, which registers the daemon
  as an `SMAppService` LaunchAgent. It is an AppKit `NSStatusItem` with an
  `NSPopover` of SwiftUI views (`StatusItemController`), not a `MenuBarExtra`:
  `MenuBarExtra` cannot open its window from code, and the right-click menu
  commands open it to show their result.

### Changing the socket API

The Swift app does not link Rust; the JSON shapes exist twice. To add or change
a method, update together: `libs/core/src/api.rs` (types and `METHODS`),
`apps/daemon/src/socket.rs` + `daemon.rs`, `api/examples/<method>.request.json`
and `.reply.json`, and `apps/menubar/Sources/LocalRouterKit/Api.swift`. Both
contract tests (`libs/core/tests/api_examples.rs`, `ApiContractTests.swift`)
walk `api/examples/`, round-trip every file, and fail on a method without an
example. Bump the major of `API_VERSION` only for breaking changes; clients
refuse a different major.

The MCP server exposes exactly nine tools; `apps/cli/tests/mcp.rs` checks the
list, so a new tool needs that test changed on purpose.

## Things that are easy to get wrong

- **Only `.localhost` names.** macOS resolves every `*.localhost` to
  127.0.0.1 and ::1 itself; there is no DNS server. `.local` is Bonjour and
  stalls 5 s.
- **Ports 80/443 without root** work only on `0.0.0.0`/`[::]`, so every accepted
  connection is peer-checked before any byte is read (`listen.rs`). After
  binding, `bind_all` connects to 127.0.0.1 and ::1 to prove its own socket
  gets the traffic: `SO_REUSEADDR` otherwise lets another program's
  `127.0.0.1:80` silently win.
- **TCP routes are chosen by port only**; the host name is for people. Their
  listeners bind 127.0.0.1 and ::1 only, both or neither.
- **`router.localhost` is built in**: the proxy answers it before the route
  lookup with `libs/core/src/help.md` (status and routes filled in by
  `help.rs`), and `CertStore` always issues its certificate. Validation refuses
  the host key `router`. Keep `help.md` in step with the CLI and MCP tools.
- **Texts name their instance** (ADR 04): `help.md`, `note.md` and `mcp.md` in
  `libs/core/src` are templates (`{{CLI}}`, `{{HELP_URL}}`, `{{HTTPS}}` …)
  filled by `help.rs`. Never write `localrouter `, `router.localhost` or
  `LocalRouter.md` into a string; `libs/core/tests/no_fixed_names.rs` fails on it.
- **Owned routes (`owner_pid`) are never saved**; persistent + owner_pid is
  refused.
- **A route is keyed by host plus path** (ADR 03). Never look up, replace or
  remove a route by host alone: `unregister_route` without `path` removes only
  the route without a path. `RouteTable::lookup` (the proxy) and
  `RouteTable::explain` (`localrouter which`) share one walk, so they cannot
  disagree. `routes.json` is written as
  version 1 unless a saved route has a path, so older daemons can still read it.
- **MCP arguments refuse unknown fields** (`deny_unknown_fields`): an argument
  the server does not know must fail, not vanish.
- **The daemon never replaces an existing CA**; only `reset_ca` (user action,
  not an MCP tool) does. `ca.key` is created with mode 0600 and never appears in
  a reply or log.
- **Tests never touch `~/Library`**: they set `LOCALROUTER_HOME` to a temp dir
  and write a `config.json` with ports `0`. macOS limits socket paths to 103
  bytes, so keep temp dirs short (`tempfile::Builder::new().prefix("lr")`).
  CLI tests build `localrouterd` themselves (`apps/cli/tests/common/mod.rs`).
- **Fork inheritance makes socket tests flaky** if a test frees a port and
  binds it again while another test spawns a child: macOS marks sockets
  close-on-exec only after creation. Keep a probe socket instead of re-binding
  a freed port (see `tcp_listen.rs` tests).
- Debug builds of the daemon read `LOCALROUTER_TEST_API_VERSION` (used by the
  version-mismatch test), `LOCALROUTER_TEST_RESOLVE` and
  `LOCALROUTER_TEST_UPSTREAM_CA` (the proxy end-to-end test); release builds
  ignore them.
- **The forward proxy (ADR 06)**: its port binds 127.0.0.1 and ::1 only, never
  with `allow_lan`, and never the HTTP or HTTPS port. A `.localhost` name that
  reaches it goes to the route table, never to DNS. Upstream TLS always uses a
  verifying `ClientConfig` (`upstream::platform_tls`); never add an
  accept-any switch there. `inspect_hosts` is `["*"]` by default. The
  inspection CA (`inspect-ca/`) is never made at start: it is made when
  `inspect_hosts` is set non-empty or the proxy is turned on with a non-empty
  list; until it exists every `CONNECT` is a tunnel. Its `CertStore` never
  signs a `.localhost` name. Tests set `proxy_port` 0 before turning the proxy
  on, and their `config.json` has `"inspect_hosts":[]`.
- **Script rules (ADR 07)** live in `libs/core/src/scripts/`: `rules.rs`
  (fields, matching), `engine.rs` (Lua 5.4 through `mlua`, sandbox, limits,
  script threads, reload), `lua_api.rs` (`req`/`res`/`ex` and the modules),
  `bodies.rs` (classes, decoding with limits, the 512 MB budget, saved files),
  `events.rs` (server-sent events and NDJSON), `mod.rs` (the hook the router
  and the forward proxy call). With no matching rule the hook returns before
  any allocation (I1); keep it that way. Network tasks never run Lua: they
  send a job to the script threads. A log rule must never delay or change
  traffic: each part goes to the client first, and its queue drops instead of
  waiting. Scripts see every header as it is. The reference text is
  `libs/core/src/scripts.md`, served at `router.localhost/scripts`; keep it in
  step with `lua_api.rs` (a test reads the field names from the tables).
- **The proxy log (ADR 08)** lives in `libs/core/src/har/`: `entry.rs` (a
  record becomes one HAR entry; bodies are decoded here, on the writer
  thread), `capture.rs` (bodies: a copy of the first 1 MB of each, inside a
  256 MB budget; the entry is written when the response body ends or is
  dropped), `websocket.rs` (after a 101 the copy loops read the frames,
  `permessage-deflate` included, into `_webSocketMessages`; the entry is
  written at close), `writer.rs` (the file, roll, prune, repair), `mod.rs`
  (`HarLog`: the on flag, the queue, the `har-writer` thread, live events),
  `viewer.rs` and its `viewer.html/.js/.css`. A copy never delays traffic: it
  is cut at the limit or the budget. `/api/entries` and the live feed give
  summaries without bodies (`summarize`), each with `_at`, its offset;
  `/api/entry?file=&at=` gives one whole entry. The network task checks
  `enabled()` before it copies anything and only calls `try_send`; the writer
  thread exists only while there are records. A file is valid HAR after every
  entry: only the closing `\n]}}\n` is ever rewritten. The viewer at
  `proxy.localhost` and `router.localhost/proxy-log/` is **loopback only and
  read only** (`GET`/`HEAD`, no CORS, CSP), serves only `proxy-*.har` names that
  are regular files, and is never logged. `proxy` is reserved for new routes,
  but a saved route `proxy` still loads and wins (`validate_saved`). Headers
  are written as they are, cookies and API keys too; only
  `Proxy-Authorization` is left out.
- **LAN access per network (ADR 08)**: `allow_lan` lets another machine in
  only when its network id (the router's MAC address, `apps/daemon/src/network.rs`)
  is in `lan_networks`. The id comes from the System Configuration store, not
  the ARP table: macOS 26 hides the ARP table from programs without Local
  Network access. The lookup runs off the accept loop, only for a non-loopback
  peer; the proxy port, TCP routes and the viewer never use it. Debug builds
  read `LOCALROUTER_TEST_NETWORK` (`mac:…,192.168.0.1,en0` or `none`) and
  `LOCALROUTER_TEST_EXTRA_ROOTS` (roots added to `bundle.pem`).

## Bundle and release

- `LocalRouter.app/Contents/MacOS/LocalRouter` (Swift), `Contents/MacOS/localrouterd`
  (LaunchAgent `BundleProgram`), `Contents/Helpers/localrouter` (CLI),
  `Contents/Resources/LocalRouter.md` (the Claude Code note, printed by the
  bundled CLI's hidden `note` command from `libs/core/src/note.md`; keep it in
  step with `help.md`). The CLI
  is not in `MacOS/` because `localrouter` and `LocalRouter` are one file on a
  case-insensitive disk; paths live in `BundleLayout` (Swift) and
  `scripts/build-app.sh`.
- Bundle id `dev.localrouter.app`, LaunchAgent label `dev.localrouter.app.daemon`.
- **Instances** (ADR 04): `scripts/build-app.sh --suffix -dev` builds
  `LocalRouter-dev.app`, which runs next to the release with its own names,
  folders and ports (7080/7443). Every name is the release name plus the
  suffix; `libs/core/src/instance.rs`, `Instance.swift` and `release-lib.sh`
  hold the rule, checked against `api/instance-names.json`. The daemon and CLI
  read the suffix from their own file names, so never rename them in the
  bundle. `scripts/install.sh` installs the `-dev` instance by default and
  never touches `LocalRouter.app` unless given `--suffix ""`. A suffixed
  instance never updates itself; `release.sh` refuses a suffix.
- The version has one source: `[workspace.package] version` in `Cargo.toml`;
  `build-app.sh` renders it into `Info.plist`.
- Releases follow VibeViewer's scripts: Developer ID signing with hardened
  runtime, notarized and stapled DMG, `gh release create` on
  `shchahrykovich/localrouter`. The updater accepts only https GitHub URLs,
  images `spctl` accepts, and the same Team ID, so ad-hoc builds never update
  themselves. `.env.notarize` holds credentials; it is git-ignored and sourced
  without `export`.
- Only `aarch64-apple-darwin` is built (Apple Silicon, macOS 14+).
