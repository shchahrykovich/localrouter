# ADR 08. Proxy log, a viewer at proxy.localhost, LAN access per network

**Status:** Built, 2026-10-02, not released. What differs from the plan and
the test results: [Plan versus actual](10-plan-vs-actual.md). Builds on
[ADR 06](../06-forward-proxy-2026-10-01/README.md) (the forward proxy) and
[ADR 07](../07-proxy-scripts-lua-2026-10-01/README.md) (script rules, the
secret header list).

## Summary

**In one sentence.** The forward proxy writes every request it carries to
rolling HAR files in the instance's logs folder (`libs/core/src/har/`), the
daemon serves a read-only viewer for them at `proxy.localhost`, and "Allow LAN
access" applies only on the networks the user allowed.

**In three sentences.**

Today proxy traffic is only in the in-memory log, without headers or query,
and lost at restart, and "Allow LAN access" opens the dev servers on every
network the Mac joins.

A writer thread appends one HAR 1.2 entry per proxied request with secret
headers redacted, rolls files at a size or a request limit and keeps the 5
newest, and a page compiled into the daemon shows them at `proxy.localhost`,
while LAN access becomes a list of networks known by their router's MAC
address.

Claude Code can turn the proxy on, run programs through it and read what they
sent with CLI commands, `proxy env` and `jq`, MCP keeps its nine tools, and
nothing of this stays in memory while it is idle.

**In seven sentences.**

The proxy (ADR 06) keeps its traffic only in the in-memory request log, with
no headers and no query (ADR 01, I10), and the user asked for every request
on disk, on by default, in a format Chrome opens, with two limits and a switch.

The format is HAR 1.2, which Chrome DevTools saves and imports; because Chrome
cannot show a `.har` file as a page, the daemon also serves a viewer at
`proxy.localhost`, for this Mac only and read only.

The network task hands each record to a bounded queue with `try_send`, so
traffic never waits, and a `har-writer` thread that exists only while there is
traffic redacts secret headers, keeps the file valid JSON after every entry,
rolls it at `proxy_log_file_mb` or `proxy_log_file_requests`, and deletes files
past the 5th.

The viewer's HTML, JavaScript and CSS are compiled into the daemon, and its
JSON paths stream files in 64 KB chunks, read the newest entries from the end
of a file and push new ones live, with no state kept between requests.

"Allow LAN access" now applies only to networks in `lan_networks`, each known
by its router's MAC address read from the routing and ARP tables when another
machine connects, and an old `allow_lan: true` becomes "the current network
only" at the update.

Claude Code operates the proxy through CLI commands the user sees, runs
programs with `proxy env`, which now also points Python and `curl` at a bundle
of the system roots plus the inspection CA, and reads the files with `jq`;
`get_proxy` gains a `log` block and the MCP server still lists nine tools.

The open decisions are bodies (U1) and query-string redaction (U2); the tests
cover the writer, the viewer's security rules, the LAN check, both API sides
and two end-to-end journeys, and manual tests cover the Chrome import, real
networks, memory and an agent session.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 0 | Working backwards: what users might say | simulation | [00-working-backwards.md](00-working-backwards.md) |
| 1 | HAR files: every proxied request on disk | feature | [01-har-files.md](01-har-files.md) |
| 2 | The viewer at proxy.localhost | feature, security | [02-viewer.md](02-viewer.md) |
| 3 | Clients and agents: settings, MCP, the menu bar app | feature | [03-clients-and-agents.md](03-clients-and-agents.md) |
| 4 | LAN access per network | feature, security | [04-lan-per-network.md](04-lan-per-network.md) |
| 5 | Components: where the log lives | overview | [05-components.md](05-components.md) |
| 6 | Data flow | data flow | [06-data-flow.md](06-data-flow.md) |
| 7 | Semantic Change Manifest | manifest | [07-semantic-change-manifest.md](07-semantic-change-manifest.md) |
| 8 | Test plan | test plan | [08-test-plan.md](08-test-plan.md) |
| 9 | Tasks | build plan | [09-tasks.md](09-tasks.md) |
| 10 | Plan versus actual | as built | [10-plan-vs-actual.md](10-plan-vs-actual.md) |

## The two questions of the design session

**How is the content served?** From inside the daemon. The router answers
`proxy.localhost` before the route lookup, as it answers `router.localhost`.
The page, its script and its style are compiled in with `include_str!`; the
data comes from three paths: `/api/files`, `/files/<name>` and `/api/live`.
No web folder, no build step, nothing from the internet.
[Change 2](02-viewer.md#how-the-content-is-served).

**What changes in MCP?** No new tool: the server still lists nine. `get_proxy`
returns a `log` block (folder, viewer URL, limits, counters, error) and its
description says so. Changing the log is a CLI command the user sees, the rule
ADR 06 set for the proxy. Agents read the files with `jq`.
[Change 3](03-clients-and-agents.md#mcp-no-new-tool).

**Can Claude Code use the proxy on its own?** Yes: it operates it with CLI
commands the user sees, runs programs with `proxy env`, and reads the HAR
files with `jq`. The one missing piece was a CA file for programs that are not
Node.js; the daemon now writes a bundle of the system roots plus the
inspection CA. [Change 3](03-clients-and-agents.md#the-agent-loop-operate-run-inspect).

## Why read the working-backwards file

It found eleven gaps: nine are fixed in this ADR, two are written down as
"not now". The two that changed the
design most:

- **G5.** A user's saved route named `proxy` would vanish after the update,
  because saved routes are validated again at load. The route is now kept,
  and the viewer has a second address.
- **G2.** On by default means headers reach the disk the first time the user
  turns the proxy on for one test. The CLI and Settings now say so at that
  moment.

## Why read the manifest

- **A new kind of data on disk.** The HAR files are the only place where
  headers (redacted) and full URLs of proxy traffic are kept, and
  `~/Library/Logs` goes into Time Machine backups. A rollback does not delete
  them. [§10, §11, §14](07-semantic-change-manifest.md#10-data-impact).
- **Two unresolved effects**: bodies (U1) and query-string redaction (U2).
  Neither blocks this ADR. [§12](07-semantic-change-manifest.md#12-unresolved-effects).
- **One destructive path**: prune deletes HAR files past the 5th, and only
  files whose names the writer makes (I5).
- **A rollback opens LAN access on every network again**: an older daemon
  reads `allow_lan: true` as before. [§14](07-semantic-change-manifest.md#14-rollback).

## Why read the test plan

The forward proxy tests build the proxy by hand, not through the daemon, so
they cannot show that the daemon turns the log on from `config.json`. The
plan adds a daemon API test for that. The viewer's JavaScript and the app
views have no automated tests: the repo has no browser or UI test tool, and
adding one needs approval. [Test plan](08-test-plan.md#mock-versus-real-check).

## Notable artifacts

- `libs/core/src/har/` (new): `mod.rs`, `entry.rs`, `writer.rs`, `viewer.rs`,
  `viewer.html`, `viewer.js`, `viewer.css`.
- `libs/core/src/secrets.rs` (new, moved from the script engine).
- `config.json`: `proxy_log`, `proxy_log_file_mb`, `proxy_log_file_requests`.
- Socket API 1.5; `api/examples/set_config_proxy_log.request.json`.
- Files: `~/Library/Logs/LocalRouter/proxy/proxy-YYYYMMDD-HHMMSS.har`,
  `inspect-ca/bundle.pem`.
- `apps/daemon/src/network.rs`, `apps/cli/src/lan.rs` (new); `config.json`
  `lan_networks`.

## The through-line

The proxy already sees every request; this ADR keeps what it saw, and lets
Claude Code use it end to end. Every decision follows from three rules: the
record must never slow or change the traffic (ADR 06, ADR 07); what
LocalRouter keeps and serves stays on this Mac, on networks the user chose,
with secrets hidden; and nothing stays in memory longer than one request
needs it.
