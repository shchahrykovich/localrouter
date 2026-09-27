# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

LocalRouter is a macOS menu bar app that gives local dev servers names instead
of ports: `https://feat-login.shop.localhost` for HTTP, `db.shop.localhost:15432`
for TCP. Coding agents manage routes through an MCP server. Design and
decisions: `docs/adr/01-project-setup-2026-09-26/` (architecture),
`docs/adr/02-distribution-and-self-update-2026-09-26/` (release and updater) and
`docs/adr/03-path-routes-2026-09-26/` (several dev servers on one name, by path).
Use the words defined in `docs/dictionary.md` (route, host key, target, listen
port, owned/session/persistent route) in code, docs and UI text.

## Commands

```
cargo test --workspace                                   # all Rust tests
cargo clippy --workspace --all-targets                   # expected: no warnings
swift test --package-path apps/menubar                   # Swift tests
swift build --package-path apps/menubar                  # build the menu bar app only
scripts/build-app.sh                                     # build/LocalRouter.app, ad-hoc signed
scripts/install.sh --user --launch                       # build and install to ~/Applications
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
  proxy on hyper (`proxy.rs`), TCP byte copy (`tcp.rs`), local CA and per-name
  leaf certificates chosen by SNI (`tls.rs`), request log ring buffer
  (`logs.rs`), socket API types (`api.rs`). No I/O at start; testable alone.
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

The MCP server exposes exactly six tools; `apps/cli/tests/mcp.rs` checks the
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
  version-mismatch test); release builds ignore it.

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
- The version has one source: `[workspace.package] version` in `Cargo.toml`;
  `build-app.sh` renders it into `Info.plist`.
- Releases follow VibeViewer's scripts: Developer ID signing with hardened
  runtime, notarized and stapled DMG, `gh release create` on
  `shchahrykovich/localrouter`. The updater accepts only https GitHub URLs,
  images `spctl` accepts, and the same Team ID, so ad-hoc builds never update
  themselves. `.env.notarize` holds credentials; it is git-ignored and sourced
  without `export`.
- Only `aarch64-apple-darwin` is built (Apple Silicon, macOS 14+).
