# 10. Tasks

**Status:** Tasks 1 to 11 are built, with the end-to-end test of task 12
(E1e). Open: the manual tests of task 12 and task 13 (release). What changed
from this plan is at the end: [Plan vs actual](#plan-vs-actual).

| # | Task | Depends on |
|---|---|---|
| 1 | Rule model and matching | ADR 06 |
| 2 | Lua engine: sandbox, limits, script threads, load and reload | - |
| 3 | Lua API: `req`, `res`, `ex`, modules, `capture`, secrets view | 2 |
| 4 | Bodies: classes, hold, copy, save to file, event streams, decode, budget | - |
| 5 | Hook in `proxy.rs` and `forward.rs`; errors; disable after 20 | 1, 3, 4 |
| 6 | Daemon: rule methods, `script-rules.json`, owners, inspect set | 1, 2, 5 |
| 7 | Socket API 1.4, examples, Swift types | 6 |
| 8 | CLI `rules` commands | 7 |
| 9 | MCP tools `set_script_rule`, `remove_script_rule` | 7 |
| 10 | Script reference, `/scripts` page, agent texts, dictionary | 3, 8, 9 |
| 11 | Menu bar: Scripts list, rule ids in Logs | 7 |
| 12 | End-to-end test and manual tests | 6, 8, 9, 10, 11 |
| 13 | Release, smoke test, ADR status | 12 |

Tasks 1, 2 and 4 can run in parallel. Tasks 8, 9 and 11 can run in parallel.

## 1. Rule model and matching

**Deps:** ADR 06 (host patterns in `inspect.rs`). **Tests:** T1, T2.

`libs/core/src/scripts/rules.rs`: `ScriptRule` with the fields of
[02](02-rules.md), `validate` (label id, pattern, path normalized like
routes, absolute paths, `output_dir` not `/` and not in the data folder, at
most 64 rules, persistent with owner refused), and `RuleTable::matching(host,
path, method)` returning intercept rules in order and log rules. Invariants
I3, I10 (validation part).

## 2. Lua engine: sandbox, limits, script threads, load and reload

**Deps:** none. **Tests:** T3, T4, T11.

Add `mlua` (`lua54`, `vendored`, `send`, `serialize`). `engine.rs`: build a
state with only the allowed libraries; text-mode load; top level under
100 ms; check the returned table; memory limit 64 MB; instruction hook every
1,000 instructions with the call's deadline; script thread pool (cores, at
most 4); up to 4 states per intercept rule; one state and one ordered queue
per log rule; `check_only`; reload by modification time, size and inode at
most once a second, keeping the old version on failure. Invariants I4, I5,
I12, I16.

## 3. Lua API: `req`, `res`, `ex`, modules, `capture`, secrets view

**Deps:** 2. **Tests:** T9, T10 (unit part), T25.

`lua_api.rs`: build `req`, `res`, `ex` from the exchange with secret headers
shown as `[redacted]`, and write changes back so that an untouched
`[redacted]` keeps the original value. Modules `json` (with `null`), `sse`,
`base64`, `url`, `multipart`, `log` (200 lines a minute per rule), `print`.
`json.encode` refuses strings that are not valid UTF-8 with the hint to use
`base64` or `save`. The body fields of [04](04-bodies-and-captures.md#body-fields-a-script-sees). `capture` for
log rules only: name check, `O_NOFOLLOW`, mode `0600`, append with
`O_APPEND`, write through temp file and rename, quota. `secret_headers` in
`config.rs`. Invariants I7, I8.

## 4. Bodies: classes, hold, copy, save to file, event streams, decode, budget

**Deps:** none. **Tests:** T7, T8 (budget part), T20, T21 (splitter part), T22, T23, T24.

Add `async-compression`. `bodies.rs`: the class of a body from its
Content-Type (I26); a body wrapper that holds up to 8 MB (and passes the rest
unchanged when over); a tee that sends each part to the client first and
copies up to 16 MB; a file writer behind a 256 KB queue that stops instead of
waiting (I23) and writes `output_dir/bodies/<id>-req|res[.part].<ext>`;
`Range` and `Content-Range` passed to scripts, and partial bodies marked
read-only (I24); decoding of gzip, deflate, br and zstd that stops at the
limit of its use (I25); one global byte budget (512 MB, smaller in tests)
shared by all holds, copies and queued events. `events.rs`: split
server-sent events and NDJSON into events, write changed events back in the
same format, skip events over 1 MB (I21). Refuse invalid class lists at load
(I22). Invariants I2, I15, I17, I21 to I26.

## 5. Hook in `proxy.rs` and `forward.rs`; errors; disable after 20

**Deps:** 1, 3, 4. **Tests:** T5, T6, T7, T8, T10, T12, T18, T19, T21 (hook part).

Fill the empty seam of ADR 06 at the two points, and add the end-of-exchange
copy. Return before any allocation when no rule matches (I1). Skip the help
page, the 404 page, TCP routes and tunnels (I20). `on_error` and the 502 page
naming the rule; count failures; disable after 20 in a row, with a daemon
log line (I6). Call intercept `on_event` for each event from `events.rs`
before it goes to the client, and queue a copy for log `on_event` after it
(I21). Request log fields `rules`, `script_error`. New
`libs/core/tests/scripts.rs`.

## 6. Daemon: rule methods, `script-rules.json`, owners, inspect set

**Deps:** 1, 2, 5. **Tests:** T13.

`set_script_rule` (with the `output_dir` probe and `check_only`),
`remove_script_rule`, `list_script_rules` in `daemon.rs` and `socket.rs`.
`script-rules.json` in `store.rs` with atomic replace and rollback. Owned
rules through the existing pid watcher. The inspect set includes enabled rule
hosts outside `.localhost`; create the inspection CA on first need. Persist a
rule disabled after 20 failures when it is persistent. `get_proxy` gains
`script_rules`. Invariants I10, I11.

## 7. Socket API 1.4, examples, Swift types

**Deps:** 6. **Tests:** T14.

`API_VERSION` `1.4`, `METHODS` 16. Examples listed in the test plan. Swift
`Api.swift` types. The 1.3 decode check (I18, I19).

## 8. CLI `rules` commands

**Deps:** 7. **Tests:** T15.

`rules`, `rules add`, `check`, `rm`, `enable`, `disable`, `api`. Make
`--script` and `--output-dir` absolute. `--reveal-secrets` only with a
terminal on standard input and a typed `yes` (I9).

## 9. MCP tools `set_script_rule`, `remove_script_rule`

**Deps:** 7. **Tests:** T16.

Two tools with `deny_unknown_fields` and no `reveal_secrets`. Nine-tool
snapshot. Tool descriptions point to `router.localhost/scripts` (I9, I13).

## 10. Script reference, `/scripts` page, agent texts, dictionary

**Deps:** 3, 8, 9. **Tests:** T17.

`libs/core/src/scripts.md` template, served by `help.rs` at `/scripts` and
printed by `rules api`. The six points of [05](05-clients-and-agents.md) in
`help.md`, `note.md`, `mcp.md`. `docs/dictionary.md`: script rule, intercept
script, log script, exchange, capture file, `output_dir`, secret header.
`CLAUDE.md`: nine tools, ADR 07 (I14).

## 11. Menu bar: Scripts list, rule ids in Logs

**Deps:** 7. **Tests:** M3.

Settings Scripts list with counters, last error, Enabled switch, Remove, and
a confirmation dialog for revealing secrets. Logs show `rules`.

## 12. End-to-end test and manual tests

**Deps:** 6, 8, 9, 10, 11. **Tests:** E1, M1, M2, M3, M4, M5, M6, M8.

Write E1e in `apps/cli/tests/e2e.rs`. Run M1 to M6 and M8 and record the
results in the test plan.

## 13. Release, smoke test, ADR status

**Deps:** 12. **Tests:** M7.

Release with ADR 06 or after it. Run M7 on the installed release. Append the
Actual Change Manifest and the Plan vs Actual table; flip the README status.

## Invariants to tasks

| Invariant | Task |
|---|---|
| I1 no rule, no cost | 5 |
| I2 log never delays or changes | 4 |
| I3 only matching intercepts change | 1, 5 |
| I4 sandbox | 2 |
| I5 limits fail the call only | 2 |
| I6 `on_error`; disable after 20 | 5, 6 |
| I7 capture writes confined | 1, 3 |
| I8 secrets hidden, wire unchanged | 3 |
| I9 no `reveal_secrets` through MCP; terminal only | 8, 9 |
| I10 owned rules never saved | 1, 6 |
| I11 inspect set follows rules | 6 |
| I12 compile errors refused; old version kept | 2 |
| I13 nine MCP tools | 9 |
| I14 agent texts | 10 |
| I15 body limits | 4 |
| I16 log order | 2 |
| I17 512 MB budget | 4 |
| I18 examples decode | 7 |
| I19 old clients decode | 7 |
| I20 no scripts on help, 404, TCP, tunnels | 5 |
| I21 event streams never held whole | 4, 5 |
| I22 only listed classes held, copied or saved | 4 |
| I23 saving never slows the client | 4 |
| I24 partial bodies cannot be changed | 4 |
| I25 decoding stops at each limit | 4 |
| I26 class from Content-Type only | 4 |

## Plan vs actual

What the build did differently from this plan, and why.

| # | Planned | Built | Why |
|---|---|---|---|
| 1 | The writer queue of a saved body is 256 KB | 4 MB (`bodies::SAVE_QUEUE`) | On loopback 256 KB fills before the writer thread starts, so every image of a local dev server was cut as `slow_disk` (found by T22). The client is still never slowed; only the stop comes later. |
| 2 | `async-compression` decodes bodies | `flate2`, `brotli` and `zstd` write-decoders | Decoding happens in the tap as parts pass, with a sink that refuses bytes past the limit (I25). A push decoder fits that; an async reader does not. |
| 3 | `mlua` with the `serialize` feature | Without it; `json` is written in `lua_api.rs` | The "is not text: use base64.encode" error needs the field path, and empty arrays must stay arrays. |
| 4 | The hook in `proxy.rs` and `forward.rs` | The hook is `Scripts::exchange` in `scripts/mod.rs`; `proxy.rs` and `forward.rs` call it with a `send` function | One flow for router and proxy traffic, testable without a daemon. |
| 5 | `body_skipped`: `class`, `too_large`, `budget`, `streamed`, `slow_disk`, `quota`, `upgrade` | Also `save_error` (the file could not be made, for example `bodies/` is a link) and `error` (the body broke while held) | A script must be able to tell these from a full disk. |
| 6 | `check_only` checks `output_dir` | A log script may be checked without `output_dir`; the reply has a note | `rules check <file>` tests a script before its folder exists. |
| 7 | Errors use the existing codes | A new code `invalid_script_rule` | An agent can tell a bad rule from a bad route. |
| 8 | Setting a rule again resets its counters | Counters carry over when `script` and `output_dir` stay the same | `rules enable` and `rules disable` set the rule again; the ADR says a disabled rule keeps its counters. |
| 9 | Not said | A saved rule whose file does not load at start stays in the list, matches nothing, and is not loaded again until it is set again | Its fields were never checked against a script of that kind. |
| 10 | The Swift `LogEntry.http` case | It gained a ninth value, `scripts: ScriptRun?` | Logs shows the rule ids that ran, red when one failed. |
| 11 | The app sets `reveal_secrets` with a confirmation dialog | "Let It See Secrets…" in Settings > Scripts, with a dialog that names the host | As planned; the CLI and the app are the only ways. |
| 12 | Not said | `rules add` makes a session rule unless `--persistent` | Rules are for a job; the ADR asks for persistent rules only when the user asks. |
| 13 | One pool of script threads (the number of cores, at most 4) | Two pools of that size: one for intercept calls, one for log calls | Found in review: four slow log scripts took every thread, so intercept calls failed "busy" and the client got a 502. That broke I2. |
| 14 | Not said | A held or script-set body is copied and saved in the task that ends the exchange, with no writer-queue limit | Found in review: saving it before `send` made the client wait for the disk, and a held body over 4 MB was saved empty. |
