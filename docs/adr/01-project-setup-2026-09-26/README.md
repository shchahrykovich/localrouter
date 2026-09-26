# ADR 01. Project setup: stack and architecture of LocalRouter

**Status:** Accepted, built on 2026-09-26 with drift (see the manifest's Actual
Change Manifest). Automated tests pass (86 Rust, 13 Swift); manual tests M1 to
M4 and M6 to M8 are open. Distribution and self-update:
[ADR 02](../02-distribution-and-self-update-2026-09-26/README.md).

## Summary

**In one sentence.** LocalRouter is a Rust daemon (`localrouterd`) plus a Rust CLI
and MCP shim (`localrouter`) plus a Swift menu bar app, which gives each local
dev server a `https://<name>.localhost` address, and each local database or
cache a stable `<name>.localhost:<port>` TCP address.

**In three sentences.**

Developers and coding agents start many dev servers, and each one hides behind a
different `localhost` port.

A per-user Rust daemon listens on ports 80 and 443 for HTTP and HTTPS routes and
on one loopback port per TCP route, keeps the table of routes, and signs a
certificate for each HTTPS name with a local certificate authority.
Agents, the CLI and the menu bar app all change that table through one JSON
socket API, and browsers see `https://feat-login.shop.localhost` served by the
right worktree.

**In seven sentences.**

The first idea used `.local` names, but macOS sends `.local` to Bonjour and a
failed lookup takes 5 seconds. We measured that macOS resolves every
`*.localhost` name to `127.0.0.1` and `::1` by itself, so version 1 uses
`.localhost` and needs no DNS server.

The daemon `localrouterd` binds ports 80 and 443 on all interfaces, which macOS
allows without root, closes every connection that does not come from loopback,
and picks an HTTP route by longest match on the `Host` header, so `feat-x.shop`
falls back to `shop`. Plain TCP carries no name, so each TCP route (Postgres,
Redis) gets its own loopback listen port, while HTTPS uses a 90-day certificate
per routed name, signed by a CA the user trusts once.

Coding agents register routes through `localrouter mcp`, a stdio MCP shim, and
the CLI and the Swift app use the same socket API; only the daemon writes
`routes.json`, and routes tied to a process ID vanish when that process exits.

The main trade-offs are sockets that are reachable at TCP level (guarded by a
tested peer check) and a local root CA whose key is a strong secret. Tests use
`cargo test` for everything except one Swift contract test, and an end-to-end
test runs the real binaries on random ports.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 1 | Domain ending: `.localhost`, not `.local` | feature | [01-domain-tld.md](01-domain-tld.md) |
| 2 | Rust daemon and CLI, Swift menu bar app | feature | [02-language-split.md](02-language-split.md) |
| 3 | Ports 80 and 443 without root | feature | [03-listen-ports.md](03-listen-ports.md) |
| 4 | HTTPS with a local certificate authority | feature | [04-https-local-ca.md](04-https-local-ca.md) |
| 5 | One daemon API for MCP, CLI and app | feature | [05-one-daemon-api.md](05-one-daemon-api.md) |
| 6 | Route model: names, fallback, lifetime | feature | [06-route-model.md](06-route-model.md) |
| 7 | Protocols: HTTP, HTTPS and TCP | feature | [07-protocols.md](07-protocols.md) |
| 8 | Components and project structure on disk | proposed | [08-components.md](08-components.md) |
| 9 | Data flow | data flow | [09-data-flow.md](09-data-flow.md) |
| 10 | Semantic Change Manifest | manifest | [10-semantic-change-manifest.md](10-semantic-change-manifest.md) |
| 11 | Test plan | test plan | [11-test-plan.md](11-test-plan.md) |
| 12 | Tasks | build plan | [12-tasks.md](12-tasks.md) |

**Where the files go:** the source tree, the app bundle, and the files on the
user's Mac are all in [08-components.md, "Project structure on
disk"](08-components.md#project-structure-on-disk).

## The through-line

Every decision here comes from one rule: **no root, no system changes, and one
owner for each piece of state.** `.localhost` avoids a DNS server and
`/etc/resolver`. Binding all interfaces with a peer check avoids a root helper.
The daemon is the only writer of routes and the CA, and every client goes
through its socket. The same rule shapes TCP routes: without root there is only
one loopback address, so a TCP route is chosen by its port, not by its name.

## Why read the manifest

[10-semantic-change-manifest.md](10-semantic-change-manifest.md) holds four
things that no change file argues:

1. **One silent failure.** When two agents register the same host, the second
   one replaces the first. Only the second agent is told. The first agent's URL
   now shows the other worktree.
2. **What uninstall leaves behind.** If the user deletes the app without
   "Untrust", a trusted root CA stays in the keychain, with its key still on
   disk. This produced the Uninstall button in task 11.
3. **Five unresolved effects.** CA name constraints (U1), signing and bundle id
   (U2), `.test` support (U3), where the CLI is linked (U4), and TCP routing by
   TLS SNI (U5). U2 and U4 block the app packaging task.
4. **HTTPS targets are not verified.** The daemon accepts any certificate from
   an `https://` dev server. This is safe only because targets must be loopback.

Writing the manifest also changed the design: `ca reset` and the rule "the
daemon never replaces an existing CA by itself" came from it, and so did the
single-rename CA folder in the data flow.

## Why read the test plan

[11-test-plan.md](11-test-plan.md) found that the repository has **no test setup
at all**. The plan uses only `cargo test` and XCTest, but it needs four new Rust
test-only libraries (`tempfile`, `reqwest`, `tokio-tungstenite`, the `rmcp`
client). They need approval before task 1. CI on macOS runners is listed as
"not without approval" because it is billed.

## Notable artifacts this ADR plans

- Binaries: `localrouterd`, `localrouter`, `LocalRouter.app`.
- State files: `config.json`, `routes.json`, `ca/ca.key`, `ca/ca.pem`.
- Route protocols: `http` (shared ports 80 and 443) and `tcp` (one loopback port per route).
- Interfaces: 11 socket methods, 6 MCP tools, 10 CLI commands.
- No database, no migrations, no network traffic leaving the Mac.

## Diagrams

All diagrams are D2 sources in [diagrams/](diagrams/) with rendered `.svg`
files. `diagrams/_palette.d2` is the shared colour palette that each diagram
imports; it has no render of its own. Render one with:
`d2 --theme 0 --pad 20 diagrams/01-domain-tld.d2 diagrams/01-domain-tld.svg`
