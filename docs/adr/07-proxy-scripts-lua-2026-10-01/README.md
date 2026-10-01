# ADR 07. Lua scripts for traffic: intercept and log

**Status:** Accepted. Built on 2026-10-01: tasks 1 to 11 and the automated
end-to-end test E1e. Open: manual tests M1 to M8 (only an M2 and M7 smoke run
by hand so far), the decisions U2 to U7, and the release (task 13). Builds on
[ADR 06](../06-forward-proxy-2026-10-01/README.md). Differences from the plan:
[10-tasks.md](10-tasks.md#plan-vs-actual).

## Summary

**In one sentence.** The daemon embeds Lua 5.4 (`libs/core/src/scripts/`)
and runs user scripts on HTTP traffic of the router and the proxy, chosen by
**script rules** (host, path, method) that agents set through MCP: an
**intercept** script may change or answer a request, a **log** script gets a
copy of the finished exchange and writes it to files.

**In three sentences.**

With ADR 06 LocalRouter sees the traffic, but the user cannot act on it:
change a request, fake a reply, or save bodies for later.

A script rule names a host pattern, an optional path and methods, a `.lua`
file and, for log scripts, an output folder; the daemon keeps the rules (in
`script-rules.json` when persistent) and runs each script in a sandbox with
time and memory limits.

Intercept scripts run while the client waits and may change anything but the
destination; log scripts run after the response is delivered, from a copy,
so they can never slow or break traffic.

**In seven sentences.**

The user wants two things from traffic: to change it, and to record it with
bodies, and these have opposite costs, so they are two kinds of script.

A script is a Lua file that returns a table with `kind` and its functions
(`on_request`, `on_response` for intercept; `on_exchange` for log), and gets
`req`, `res` and `ex` tables plus `json`, `sse`, `base64`, `url`, `log` and,
for log scripts only, `capture`.

Rules use the route model's ideas (path match, owned, session and persistent
lifetimes), apply to router routes and to proxy traffic, and a rule on an
internet host makes that host inspected (ADR 06).

The runtime is `mlua` with vendored Lua 5.4 on a few script threads, with no
`io`, `os`, `require` or `load`, 50 ms per intercept call, 2 s per log call,
64 MB per state, and a rule that fails 20 times in a row is turned off.

Every body has a class from its Content-Type (text, events, media,
multipart, binary): intercept scripts hold only the classes they list (up to
8 MB), log scripts copy text and events by default (up to 16 MB, one 512 MB
budget) and can save media, downloads and uploads straight to files while
they stream; event streams are never held whole and pass event by event
through `on_event`.

Secret headers reach scripts as `[redacted]` unless the user, at a terminal,
allows a rule to see them; MCP cannot.

Agents get two tools (nine in total), a `check_only` mode, a reference page at
`router.localhost/scripts`, and rule state in `get_proxy`; the tests check
each rule at every layer, and real Claude Code and Chrome in manual tests.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 0 | Working backwards: what users might say | simulation | [00-working-backwards.md](00-working-backwards.md) |
| 1 | Two kinds of script: intercept and log | feature | [01-script-kinds.md](01-script-kinds.md) |
| 2 | Script rules: which script runs on which traffic | feature | [02-rules.md](02-rules.md) |
| 3 | The Lua runtime: sandbox, limits, threads, reload | feature, security | [03-lua-runtime.md](03-lua-runtime.md) |
| 4 | Bodies: text, streams, media and binary; capture files; secrets | feature, security | [04-bodies-and-captures.md](04-bodies-and-captures.md) |
| 5 | Clients and agents: set rules, read results, learn the API | feature | [05-clients-and-agents.md](05-clients-and-agents.md) |
| 6 | Components: where scripts live | overview | [06-components.md](06-components.md) |
| 7 | Data flow | data flow | [07-data-flow.md](07-data-flow.md) |
| 8 | Semantic Change Manifest | manifest | [08-semantic-change-manifest.md](08-semantic-change-manifest.md) |
| 9 | Test plan | test plan | [09-test-plan.md](09-test-plan.md) |
| 10 | Tasks | build plan | [10-tasks.md](10-tasks.md) |

## The through-line

**Recording must never cost the traffic; changing it always costs, and says
so.** That one rule splits scripts into two kinds, decides when bodies are
held, puts log work behind a queue that drops instead of waiting, and gives
every limit a place where it fails the script, not the request or the daemon.

## Why read the working-backwards file

[00-working-backwards.md](00-working-backwards.md) found six gaps, now
fixed. Three came from the review of streaming, media and binary bodies:
streams that never end (G4), images and video that cost memory and could not
be kept (G5), and partial `206` bodies and decompression bombs (G6). The one that changed the design most: an agent whose own API traffic
goes through the proxy can break that traffic with a bad intercept rule and
then cannot call the tool that removes it. Rules are now turned off after 20
failures in a row (I6). It also added one memory budget for all log queues
(I17).

## Why read the manifest

[08-semantic-change-manifest.md](08-semantic-change-manifest.md) holds three
things no change file argues:

1. **The daemon writes outside its data folder for the first time** (capture
   files), and no rollback removes them.
2. **The Lua API becomes a public interface.** Scripts live outside this
   repository; renaming a field later breaks them.
3. **Seven open decisions:** rerouting (U1, rejected for now), shared state
   (U2), WebSocket frames (U3), trailers and gRPC (U4), a quota that
   restarts with the daemon (U5), one file per uploaded part (U6), and
   other streaming formats such as gRPC streams and MJPEG (U7).

## Why read the test plan

[09-test-plan.md](09-test-plan.md) lists 25 automated test groups, one
end-to-end test and eight manual tests. The hard claims are timing claims
("a log rule does not delay streaming"), so the core tests use an upstream
that streams one event every 100 ms and check that the client gets the first
before the second is sent. Speed with real traffic is manual test M6.

## Notable artifacts

- New module `libs/core/src/scripts/` (`rules.rs`, `engine.rs`,
  `lua_api.rs`, `bodies.rs`, `events.rs`) and template
  `libs/core/src/scripts.md`.
- New data: `script-rules.json`; capture files in each log rule's
  `output_dir`, and saved bodies in `output_dir/bodies/`.
- Socket API 1.4: `set_script_rule`, `remove_script_rule`,
  `list_script_rules`; `get_proxy.script_rules`; log fields `rules`,
  `script_error`; config `secret_headers`.
- MCP: `set_script_rule`, `remove_script_rule` (nine tools).
- New dependencies: `mlua` (Lua 5.4, vendored), `async-compression`.
- No new program, process or port.

## Not in this ADR

- Sending a request to a different address (U1).
- State shared between calls or rules (U2).
- WebSocket messages, HTTP trailers, gRPC framing (U3, U4).
- Splitting other streaming formats into events (U7); one file per uploaded
  part (U6); guessing a body's type from its bytes.
- Network or file access from scripts other than `capture`.
- Scripts on TCP routes or on tunnels.

## Diagrams

All diagrams are D2 sources in [diagrams/](diagrams/) with rendered `.svg`
files. `diagrams/_palette.d2` is the shared colour palette, copied from ADR 05.
Render one with:
`d2 --theme 0 --pad 20 diagrams/01-hooks.d2 diagrams/01-hooks.svg`
