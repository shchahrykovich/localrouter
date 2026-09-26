# 9. Tasks

**Status on 2026-09-26:** tasks 1 to 8 done. Task 9: E1b, M1 (Next.js) and M2
done; M3, M4, M5 and the Vite and Turbopack parts of M1 open. Task 10: the ADR
status is flipped with the Actual Change Manifest; the release and M6 are open.

| # | Task | Depends on |
|---|---|---|
| 1 | Route key and path rules in core | - |
| 2 | Path lookup, `explain` and `serves` | 1 |
| 3 | Proxy: path, `strip_path`, headers, log `route` | 2 |
| 4 | Daemon: keys, exact removal, certificate hook, `routes.json` version | 2 |
| 5 | Socket API 1.1, examples, Swift types and remove | 1, 4 |
| 6 | CLI flags, output and `which` | 5 |
| 7 | MCP arguments, instructions, unknown arguments | 5 |
| 8 | Agent texts and repository docs | 6, 7 |
| 9 | End-to-end journey and manual tests | 3, 4, 6, 7, 8 |
| 10 | Release, smoke test, ADR status | 9 |

Tasks 6 and 7 can run in parallel. Tasks 3 and 4 can run in parallel.

## 1. Route key and path rules in core

**Deps:** none. **Tests:** T1, T2.

In `libs/core/src/routes.rs`: add `Route.path: Option<String>` and
`Route.strip_path: bool` with the serde attributes of the other optional
fields; add `RouteKey { host, path }`; key the `BTreeMap` by it; change `get`,
`insert`, `remove`, `owned_by` to keys. Add the path normalization and the
`RouteError` variants of the manifest. Add the protocol rules to `validate`. Add
the second hint to `BadLabel` when the `/` is in the last label. Done when T1
and T2 pass and every existing `routes.rs` test passes unchanged.

## 2. Path lookup, `explain` and `serves`

**Deps:** 1. **Tests:** T3.

Write `explain(name, path, fallback)` as in [02](02-path-lookup.md), using a
range scan over one host key and recording each step. Replace
`lookup(name, fallback)` with `lookup(name, path, fallback)`, built on
`explain` so the two cannot disagree (I33). Add `serves(name, fallback)`. Write
the regression test that runs the old lookup cases through the new function,
and the test that `explain` and `lookup` agree on every case. Done when T3
passes.

## 3. Proxy: path, `strip_path`, headers, log `route`

**Deps:** 2. **Tests:** T4, T5.

In `libs/core/src/proxy.rs`: `RouteSource::lookup` takes the path; `handle`
passes `req.uri().path()`; `outgoing_request` strips and sets
`X-Forwarded-Prefix` for `strip_path` routes; the 404 page lists paths; the
502 page names the route key. In `libs/core/src/logs.rs`: `LogEntry::Http.route`.
Done when T4 and T5 pass, including the WebSocket and TLS cases.

## 4. Daemon: keys, exact removal, certificate hook, `routes.json` version

**Deps:** 2. **Tests:** T6, T7.

In `apps/daemon/src/daemon.rs`: every table access by key; `unregister_route`
by key (I28); `remove_owned_by` by key; the certificate hook calls `serves`;
`view` builds URLs with the path; `RouteSource for Shared` passes the path. In
`apps/daemon/src/store.rs`: write version 1 or 2 (I29), read both. Done when T6
and T7 pass and every existing `api.rs` test passes.

## 5. Socket API 1.1, examples, Swift types and remove

**Deps:** 1, 4. **Tests:** T8.

`API_VERSION = "1.1"`; `HostParams.path`. Add and change the `api/examples/`
files listed in [04](04-clients-and-agent-texts.md), plus the Rust check for
I30. Swift: `Route` and `RouteView` fields and `id`, `HostParams.path`,
`LogEntry` `route`, `DaemonClient.unregister(_ route:)`, `AppModel.remove` uses
it, Domains rows show the path, Logs rows show the route key. Add
`RouteKeyTests.swift`. Done when both contract tests and T8 pass. This task must
not be split from task 4 in a release (B1).

## 6. CLI flags, output and `which`

**Deps:** 5. **Tests:** T9.

`apps/cli/src/main.rs`: `add --path --strip-path`, `rm --path` with the
"remain" line, `list` and `logs` columns, local errors before a socket call.
New command `which <url>`: parse a URL or `name/path`, call `list_routes` and
`get_config`, build a `RouteTable`, print `RouteTable::explain` as in
[04](04-clients-and-agent-texts.md). Done when T9 passes, including the
pattern B routes.

## 7. MCP arguments, instructions, unknown arguments

**Deps:** 5. **Tests:** T10.

`apps/cli/src/mcp.rs`: `path`, `strip_path` on `RegisterArgs` and into
`into_route`; `path` on `HostArgs` for `unregister_route`;
`#[serde(deny_unknown_fields)]` on every argument struct; the new tool
descriptions and `INSTRUCTIONS` paragraph, including the ask-first rule and
`localrouter which`. Update
`exactly_six_tools_are_listed`. Done when T10 passes and the tool list is
still six (I31).

## 8. Agent texts and repository docs

**Deps:** 6, 7. **Tests:** T11.

Change `libs/core/src/help.md` in the seven places listed in
[04](04-clients-and-agent-texts.md), including both patterns, the ask-first
rule, the `<img>` line and `localrouter which`; the `{{ROUTES}}` rendering in
`help.rs`; `scripts/LocalRouter.md` in its five places, including the
ask-first rule; `docs/dictionary.md`, and the
project `CLAUDE.md` line. Add `libs/core/tests/agent_texts.rs`. Done when T11
passes and a reader who knows only the note and the help page can add a path
route (checked in task 9, M3).

## 9. End-to-end journey and manual tests

**Deps:** 3, 4, 6, 7, 8. **Tests:** E1b, M1, M2, M3, M4, M5.

Write E1b in `apps/cli/tests/e2e.rs`. Build with `scripts/install.sh --user
--launch` and run M1 to M5 by hand; record the results and the real hot reload
socket paths in [03](03-forwarding.md) and the M2 answer in the manifest's U6.
Done when E1b passes and every manual table is ticked or a failure is written
down.

## 10. Release, smoke test, ADR status

**Deps:** 9. **Tests:** M6.

Run `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `swift
test --package-path apps/menubar`. Release with `scripts/publish.sh --minor`
(new feature, API minor bump). Run M6 on the updated app. Then set this ADR's
status to built, append the Actual Change Manifest to
[07](07-semantic-change-manifest.md) with a Plan vs Actual table, and do not
edit the planned half.

## Invariants to tasks

| Invariant | Task |
|---|---|
| I20 route key | 1, 4 |
| I21 path rules | 1 |
| I22 protocol rules | 1 |
| I23 match rule | 2 |
| I24 lookup order, regression | 2 |
| I25 no fallback on error | 3 |
| I26 path kept or stripped | 3 |
| I27 certificate by name | 2, 4 |
| I28 exact removal | 4, 5 |
| I29 file version | 4 |
| I30 examples cover new fields | 5 |
| I31 MCP six tools, arguments | 7 |
| I32 texts mention path routes, both patterns, ask first | 7, 8 |
| I33 `which` agrees with the proxy | 2, 6 |

Every test ID of the [test plan](08-test-plan.md) appears above: T1 and T2 in
task 1, T3 in 2, T4 and T5 in 3, T6 and T7 in 4, T8 in 5, T9 in 6, T10 in 7,
T11 in 8, E1b and M1 to M5 in 9, M6 in 10.
