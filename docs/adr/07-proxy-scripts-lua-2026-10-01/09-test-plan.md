# 9. Test plan

**Status:** The automated tests are written and pass (`cargo test --workspace`,
`swift test`). The manual tests M1 to M8 are not run yet; an M2 and M7 smoke run
by hand passed on 2026-10-01 (a log rule on `example.com` and an intercept rule
on `httpbin.org`, through the proxy, plain and inspected).

Where each group lives:

| Tests | File |
|---|---|
| T1, T2 | `libs/core/src/scripts/rules.rs` |
| T3, T4, T11, I6 | `libs/core/src/scripts/engine.rs` |
| T9, T10 (unit), T25 | `libs/core/src/scripts/lua_api.rs` |
| T20 (classes), T22 (links), T24 (unit), event splitter | `libs/core/src/scripts/bodies.rs`, `events.rs` |
| T5 to T8, T10, T12, T18 to T24 | `libs/core/tests/scripts.rs` |
| T13 | `apps/daemon/tests/api.rs`, `apps/daemon/src/store.rs` |
| T14 | `libs/core/tests/api_examples.rs`, `ApiContractTests.swift` |
| T15 | `apps/cli/tests/cli.rs` (the pseudo-terminal case uses `/usr/bin/script`) |
| T16 | `apps/cli/tests/mcp.rs` |
| T17 | `libs/core/tests/agent_texts.rs`, `libs/core/tests/proxy.rs` |
| E1e | `apps/cli/tests/e2e.rs` (`script_rules_journey`) |

Two cases differ from the plan: T19's "a TCP route runs no script" holds by
construction (TCP routes never reach the HTTP hook) and has no test of its
own; T22's slow disk is a writer that waits 30 ms per part.

## What the repository can run today

The same tools as [ADR 06's test plan](../06-forward-proxy-2026-10-01/07-test-plan.md):
Rust unit tests, `libs/core/tests/` with the proxy harness (echo, TLS,
WebSocket upstreams), the real daemon in `apps/daemon/tests/api.rs`, the
real binaries in `apps/cli/tests/` (`reqwest`, `rmcp`), and Swift contract
tests. ADR 06 adds `libs/core/tests/forward.rs`. Nothing runs Chrome or
Claude Code in CI.

Scripts used by tests are small `.lua` files written into the test's temp
folder, so each test shows its script next to its checks.

## Overview

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| T1 | unit | rule fields: id, host pattern, path, methods, absolute paths, `output_dir` rules, limits | - | - | CI |
| T2 | unit | matching and order (I3) | - | - | CI |
| T3 | unit | sandbox: each removed function is absent; bytecode refused (I4) | Lua 5.4 | - | CI |
| T4 | unit | time and memory limits fail the call, not the process (I5) | Lua 5.4 | - | CI |
| T5 | core integration | `on_request` changes method, path, headers, body; returns a response; order (I3) | proxy harness, echo upstream | the internet → echo | CI |
| T6 | core integration | `on_response` changes status, headers, body | proxy harness | - | CI |
| T7 | core integration | bodies: held only when asked, 8 MB limit, decoding, streaming kept otherwise (I15) | proxy harness, a streaming upstream | - | CI |
| T8 | core integration | log copy: byte-equal, not delayed, order, drop, 512 MB budget (I2, I16, I17) | proxy harness | - | CI |
| T9 | unit + integration | capture files: names, `0600`, `O_NOFOLLOW`, quota, `output_dir` checks (I7) | file system | - | CI |
| T10 | core integration | secrets hidden from scripts; bytes on the wire unchanged (I8) | proxy harness | - | CI |
| T11 | unit | load, `check_only`, reload, broken edit keeps old, file gone (I12) | file system | - | CI |
| T12 | core integration | `on_error` fail and pass; disabled after 20 failures (I6) | proxy harness | - | CI |
| T13 | daemon | set, remove, list; lifetimes; `script-rules.json` and its rollback; inspect set (I10, I11) | real daemon | - | CI |
| T14 | contract | examples decode in Rust and Swift; 16 methods; old shapes decode (I18, I19) | serde, Codable | - | CI |
| T15 | CLI | `rules` commands; `--reveal-secrets` refused without a terminal (I9) | real binaries | terminal → a pseudo-terminal in one case | CI |
| T16 | MCP | nine tools; `reveal_secrets` refused; `check_only` (I9, I13) | real MCP shim and daemon | - | CI |
| T17 | texts | agent texts and `scripts.md`; `/scripts` page; no fixed names (I14) | - | - | CI |
| T18 | core integration | no rule: no Lua, no buffer, no copy (I1) | proxy harness | - | CI |
| T19 | core integration | router traffic gets scripts; help page, 404, TCP, tunnels do not (I20) | proxy and forward harnesses | - | CI |
| T20 | unit + core integration | body classes from Content-Type; defaults; only listed classes held, copied or saved (I22, I26) | proxy harness | - | CI |
| T21 | core integration | event streams: split, intercept `on_event` changes and drops, log `on_event` order, never held (I21) | proxy harness, a streaming upstream | - | CI |
| T22 | core integration | saving bodies to files: decoded, names, `0600`, truncated on a slow writer or quota, client never slowed (I23) | proxy harness, file system | slow disk → a writer that blocks on purpose | CI |
| T23 | core integration | `Range` and `206`: partial copies and files, body change refused (I24) | proxy harness | - | CI |
| T24 | core integration | decompression bombs stop at each limit (I25) | proxy harness | - | CI |
| T25 | unit | `multipart.parse`; binary strings; `json.encode` refuses non-UTF-8 with the hint | Lua 5.4 | - | CI |
| E1 | end-to-end | an agent-like journey: check, set, traffic, capture, remove | daemon, CLI, MCP, reqwest through the proxy | the internet → local TLS server | CI |
| M1 | manual | capture Claude Code's API calls with a log rule | everything | - | by hand, uses the user's Claude account |
| M2 | manual | intercept in Chrome: add a header, block a path, fake a reply | everything | - | by hand |
| M3 | manual | app Scripts list | app | - | by hand |
| M4 | manual | failure checks | everything | - | by hand |
| M5 | manual | agent acceptance (the session of the working-backwards file) | agent, MCP | - | by hand, uses the user's Claude account |
| M6 | manual | speed: throughput and streaming with and without rules | everything | - | by hand |
| M7 | manual | post-release smoke test | release build | - | by hand |
| M8 | manual | media and binary in Chrome: images, video with seeking, a download, an upload | everything | - | by hand |

### Replaced parts and where they run for real

| Replaced in CI | Hides | Real in |
|---|---|---|
| the internet (local servers) | real compressed, chunked and streamed bodies from real APIs | M1, M2 |
| a short synthetic stream | a model reply that streams for 30 seconds | M1, M6 |
| a pseudo-terminal in T15 | a real user typing `yes` | M4 |
| test-sized limits (a 1 MB budget instead of 512 MB, set through a test-only constructor) | the real limits under load | M6 |
| a writer that blocks on purpose (T22) | a real slow or full disk | M8 |
| synthetic images, video parts and gzip bombs | real players that seek, real servers that mislabel types | M8 |

## 1. Automated tests (CI)

### Unit tests, `libs/core/src/scripts/`

| Test file | Case |
|---|---|
| `rules.rs` (T1) | id label rules; `*.com` refused; `path` normalized like routes; `methods` upper-cased; relative `script` refused; `output_dir` of `/`, the data folder, or inside it refused; more than 64 rules refused; `persistent` with `owner_pid` refused |
| `rules.rs` (T2) | `*.example.com` matches `a.example.com`; `/v1` matches `/v1/messages`, not `/v1x`; method filter; intercept order by `order`, then id; a disabled rule matches nothing |
| `engine.rs` (T3) | for each of `io`, `os.execute`, `os.remove`, `os.getenv`, `require`, `package`, `debug`, `load`, `loadfile`, `dofile`, `string.dump`: calling it fails with "attempt to call a nil value" or "attempt to index a nil value"; a file with the bytecode signature `\27Lua` is refused |
| `engine.rs` (T4) | `while true do end` fails "time limit" within 100 ms; a table that grows to 100 MB fails "memory limit"; the next call on the same rule works |
| `engine.rs` (T11) | syntax error refused with file and line; no `kind` refused; `kind = "log"` without `on_exchange` refused; `check_only` stores nothing; edit the file: the next call after 1 s uses the new code; a broken edit: old code runs, `last_error` set; delete the file: old code runs, `last_error` "gone" |
| `lua_api.rs` (T9) | `capture.append("a.jsonl", …)` twice: two lines, mode `0600`; names `../x`, `a/b`, `.hidden`, `""` refused; a symlink `out/a.jsonl → /tmp/x` makes the write fail; quota 1 KB: the write after it fails "capture quota reached" |

### Core integration, `libs/core/tests/scripts.rs` (new)

Uses the `proxy.rs` harness with a rule table, plus a streaming upstream that
sends one server-sent event every 100 ms.

| Test file | Case |
|---|---|
| `scripts.rs` (T5) | `on_request` adds `x-trace`: the echo sees it; changes the path: echo sees the new path; sets the body: echo sees it with a correct `Content-Length` and no `Content-Encoding`; returns `{status=403}`: echo gets nothing, client gets 403; two rules with order 10 and 20 run in that order |
| `scripts.rs` (T6) | `on_response` sets status 299 and a header; with `response_body`, replaces the body |
| `scripts.rs` (T7) | no rule asks for a body: the client receives the first event before the upstream sends the second (streaming kept); a rule with `response_body`: the client receives all events at the end; a gzip body is given to the script decoded; a 9 MB body: script sees `body = nil`, `body_truncated = true`, client gets all 9 MB unchanged |
| `scripts.rs` (T8) | a log rule on the streaming upstream: the client still gets the first event before the second is sent; the bytes the client gets equal the upstream's bytes; a log script that sleeps 500 ms (a busy loop under the 2 s limit) does not delay the next request; 5 exchanges reach the script in end order; with a test budget of 1 MB, the copy past it is dropped and `dropped` counts it |
| `scripts.rs` (T10) | a log script writes `ex.request.headers.authorization` to a file: the file has `[redacted]`; the echo got the real value; an intercept script that sets `authorization = "Bearer new"`: echo gets `new`; one that does not touch it: echo gets the original; `reveal_secrets` true: the file has the real value |
| `scripts.rs` (T12) | `on_request` calls `error("x")`: with `fail`, client gets the 502 page naming the rule and line; with `pass`, echo gets the request unchanged; 20 failures in a row: rule `enabled` false, `last_error` says why, request 21 runs no script |
| `scripts.rs` (T18) | with an empty rule table, a counting hook shows 0 Lua calls, 0 held bytes and 0 copied bytes for 100 requests; the same with rules that match a different host |
| `scripts.rs` (T19) | a rule for `shop.localhost` runs on a router route; a rule with host `router.localhost` is refused; the 404 page and a TCP route run no script; through `forward.rs`, a tunnel to a host with no rule runs no script, and a rule on that host makes it inspected |

### Real daemon, `apps/daemon/tests/api.rs` (T13)

| Case |
|---|
| `set_script_rule` persistent, restart the daemon: the rule is back; owned with the pid of a child that then exits: the rule is gone and was never in `script-rules.json` |
| data folder read-only: setting a persistent rule fails `io` and `list_script_rules` is unchanged |
| a rule for `api.test.example`: `get_proxy.inspect_set` has it and `inspect-ca/` exists; remove it: the host is gone from the set; `inspect_hosts` listing it too: still in the set |
| `set_script_rule` with an `output_dir` that is read-only: refused with the probe error |
| `list_script_rules` counters move after traffic through the proxy |

### Contract, `api_examples.rs` and `ApiContractTests.swift` (T14)

New examples: `set_script_rule.request.json` and `.reply.json` (a log rule),
`set_script_rule_check.request.json` (`check_only`), `remove_script_rule.*`,
`list_script_rules.*`, `get_proxy.reply.json` with `script_rules`,
`log.event.json` with `rules` and `script_error`. These tests walk the folder,
so they grow by themselves. One added case: the new `get_proxy` and log
examples decode with the 1.3 types (I19).

### CLI, `apps/cli/tests/cli.rs` (T15)

| Case |
|---|
| `rules add cap --host api.test.example --script ./cap.lua --output-dir ./out` makes both paths absolute and lists the rule |
| `rules check ./broken.lua` prints the file and line and exits 1 |
| `rules add … --reveal-secrets` with standard input not a terminal: refused, exit 2, nothing set |
| the same in a pseudo-terminal, typing `yes`: the rule has `reveal_secrets` |
| `rules disable cap`, `rules enable cap`, `rules rm cap` |
| `rules api` prints the reference with this instance's names |

### MCP, `apps/cli/tests/mcp.rs` (T16)

| Case |
|---|
| `tools/list` has exactly the nine names |
| `set_script_rule` with `reveal_secrets` fails "unknown field" |
| `set_script_rule` with `check_only` returns ok and `get_proxy.script_rules` stays empty |
| `remove_script_rule` of an unknown id returns `removed: false` |

### Agent texts, `agent_texts.rs`, `no_fixed_names.rs` (T17)

| Case |
|---|
| `help.md`, `note.md`, `mcp.md` mention both kinds, `router.localhost/scripts`, `check_only`, `owner_pid` for rules, `response_body` and streaming, "do not commit captures", and that only the user can reveal secrets |
| `scripts.md` has every field of `req`, `res`, `ex` and every module (the test reads the field list from the Rust type, so a new field without a doc line fails) |
| `GET router.localhost/scripts` on a `-dev` instance names `localrouter-dev` |

### Body classes, streams, media and binary, `libs/core/tests/scripts.rs` (T20 to T24)

The harness gets three more upstreams: one that serves a 5 MB PNG, a 40 MB
"video" with `Range` support, and a 10 KB gzip body that expands to 1 GB.

| Test file | Case |
|---|---|
| `scripts.rs` (T20) | class table: `application/json` → text, `application/problem+json` → text, `text/event-stream` → events, `image/png` → media, `multipart/form-data` → multipart, `application/octet-stream` and no Content-Type → binary, HEAD and 304 → none |
| `scripts.rs` (T20) | a default log rule on the PNG: 0 bytes held or copied (budget counter), `body_skipped = "class"`, `body_size` = 5 MB; with `copy = { "media" }`: the script gets the 5 MB as a string |
| `scripts.rs` (T20) | a PNG sent as `application/octet-stream`: class binary, not media (I26) |
| `scripts.rs` (T20) | a script with `media` in both `copy` and `save`, or `events` in `response_body`: refused at set with the reason |
| `scripts.rs` (T21) | an intercept `on_event` that uppercases `data` and drops events with `event: ping`: the client gets the changed events in order, no ping, and gets event 1 before the server sends event 2 |
| `scripts.rs` (T21) | NDJSON: one event per line, a line split across two network parts is one event |
| `scripts.rs` (T21) | a log `on_event` gets index 1, 2, 3 in order; with `on_event`, `on_exchange` has `body_skipped = "streamed"`; a stream left open: events reach `on_event` while `on_exchange` has not run |
| `scripts.rs` (T21) | an event of 2 MB passes unchanged and counts in `events_skipped` |
| `scripts.rs` (T22) | `save = { "media" }` on the PNG: `bodies/<id>-res.png` equals the upstream bytes, mode `0600`, folder `0700`; `on_exchange` gets `body_file` |
| `scripts.rs` (T22) | a gzip-encoded SVG is saved decoded |
| `scripts.rs` (T22) | the writer blocks on purpose: the client still gets the 40 MB at the speed of the run without a rule (within 20 %), the file is shorter and `body_file_truncated = true`, `body_skipped = "slow_disk"` |
| `scripts.rs` (T22) | quota 1 MB: the saved file stops at 1 MB, `body_skipped = "quota"` |
| `scripts.rs` (T22) | `bodies/` replaced by a symlink to another folder: the save fails, nothing is written there |
| `scripts.rs` (T23) | `Range: bytes=1000-1999` → 206; the copy has 1,000 bytes and `range = "bytes 1000-1999/…"`; the saved file name has `.part` |
| `scripts.rs` (T23) | an intercept that sets `res.body` on the 206: error "cannot change a partial body", the client gets the original part; a header change works |
| `scripts.rs` (T24) | the bomb under a log rule with `copy = { "text" }`: the copy stops at 16 MB, `body_truncated`; under an intercept with `response_body`: stops at 8 MB and is not held; under `save`: stops at the quota. Memory stays under the test budget throughout |

### Lua helpers, `libs/core/src/scripts/lua_api.rs` tests (T25)

| Case |
|---|
| `multipart.parse` of a form with a text field and a PNG file returns two parts with the right `filename` and bytes |
| a binary string survives `base64.encode` then `base64.decode` |
| `json.encode({ body = "\xff\xfe" })` fails with "is not text: use base64.encode, or save the body to a file" |

## 2. Automated end-to-end test, `apps/cli/tests/e2e.rs` (E1e)

Through the real binaries, as an agent would do it:

1. Daemon with a temp home; proxy on (ADR 06); a local TLS server for
   `api.test.example` that streams three server-sent events.
2. Write `cap.lua` (log, appends one JSON line) and `bad.lua` (syntax error)
   in the temp folder.
3. MCP `set_script_rule` with `bad.lua` and `check_only` → error with the line.
   Check: no rule.
4. MCP `set_script_rule` with `cap.lua`, `output_dir`, `owner_pid` of a
   helper process → reply `inspect.host_added` and `ca_created`.
5. reqwest through the proxy, trusting the inspection CA: POST a JSON body.
   Check: the three events arrive; the client's bytes equal the server's.
6. Wait for `list_script_rules` to show `matched: 1`. Check: `out/` holds one
   line with the request body and the decoded events; the `authorization`
   header in it is `[redacted]`.
7. `localrouter logs` shows the entry with `rules: ["cap"]`.
8. Kill the helper process. Check: the rule is gone and the host left the
   inspect set; the capture file stays.

## 3. Manual tests

### M1. Capture Claude Code's API calls

Uses the user's Claude account; a few short prompts.

| Check | Expected |
|---|---|
| write `capture.lua` from [01](01-script-kinds.md); `localrouter rules add claude --host api.anthropic.com --path /v1/messages --script capture.lua --output-dir ~/lr-captures` | rule listed, kind log, host in the inspect set |
| `eval "$(localrouter proxy env)" && claude` and ask a question | the answer streams as usual |
| `~/lr-captures/messages.jsonl` | one line per call, request and the full streamed reply; no API key or token in the headers |
| `sse.parse` of the reply in a second script | events parsed |

### M2. Intercept in Chrome

| Check | Expected |
|---|---|
| rule on `example.com` that adds `x-lr: 1` to requests | visible in the server's echo (use `httpbin.org/headers`) |
| `on_request` returns 403 for `/blocked` | Chrome shows the script's text |
| `on_response` with `response_body` replaces a word on a page | the page shows the new word |

### M3. App Scripts list

| Check | Expected |
|---|---|
| Settings → Scripts after M1 | the rule with counters that grow |
| break the script file | red last error with the line; traffic still captured by the old version |
| disable, enable, remove | the list and `localrouter rules` agree |

### M4. Failure checks

| Check | Expected |
|---|---|
| `output_dir` on the Desktop | refused at once with the macOS privacy message |
| an intercept rule with `error("x")` on the API host Claude Code uses | 20 failed calls, then the rule is disabled and Claude Code works again |
| `--reveal-secrets` in a terminal | the question appears; `yes` sets it |
| a 300 MB download through a host with a log rule | download completes at normal speed; `body_truncated` in the capture |

### M5. Agent acceptance

Run the session in [00-working-backwards.md](00-working-backwards.md) with
Claude Code. Expected: the agent reads the reference, uses `check_only`,
assigns an owned or removed-after rule, does not ask for `reveal_secrets`,
does not commit the captures, and reports from the capture file.

### M6. Speed

| Check | Expected |
|---|---|
| 1,000 small requests through the router, no rules, before and after this ADR | no visible difference (within noise) |
| the same with one log rule on that host | under 1 ms more per request on average |
| a streamed model reply with a log rule | first token appears at the same time as without the rule |

### M8. Media and binary in Chrome

Uses "Open Chrome via Proxy" (ADR 06). Public test pages only.

| Check | Expected |
|---|---|
| log rule with defaults on a host with a page of 50 images | the page loads at normal speed; the rule shows 50 `matched`, nothing in `bodies/` |
| change the rule to `save = { "media" }`, reload | 50 files in `bodies/`, each opens in Preview |
| a video page; play and seek three times | playback and seeking feel the same as without the rule; `.part` files for each fetched part |
| an intercept rule with `response_body = { "media" }` on the same video | playback starts later (expected), seeking still works; parts over 8 MB pass |
| download a 1 GB file with `save = { "binary" }` and the default quota | the download completes at normal speed; the saved file stops at 1 GiB minus earlier writes, marked truncated |
| upload a 20 MB file through a form with `copy = { "multipart" }` | the upload works; `multipart.parse` in the script lists the file part |
| a chat page that streams with server-sent events, with a log `on_event` | tokens appear as fast as without the rule; `on_event` lines in the capture file in order |

### M7. Post-release smoke test

No paid calls. On the release: a log rule on `example.com` with
`output_dir` in a temp folder; `curl -x http://127.0.0.1:8877 http://example.com`;
the capture file has one line; remove the rule.

## Not in this plan without approval

| Tool | Would cover | Cost |
|---|---|---|
| a fuzzer for the Lua API (cargo-fuzz) | odd values from scripts (huge strings, cycles in tables) | a nightly toolchain in CI |
| a benchmark harness (criterion) | M6 in CI | a dev dependency and noisy CI timings |

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| I1 no rule, no cost | T18 | M6 |
| I2 log never delays or changes | T8 | M1, M6 |
| I3 only matching intercepts change | T2, T5 | M2 |
| I4 sandbox | T3 | - |
| I5 limits fail the call only | T4 | M4 |
| I6 `on_error`; disable after 20 | T12 | M4 |
| I7 capture writes confined | T9, T13 | M4 |
| I8 secrets hidden, wire unchanged | T10 | M1 |
| I9 no `reveal_secrets` through MCP; terminal only | T15, T16 | M4 |
| I10 owned rules never saved | T13 | - |
| I11 inspect set follows rules | T13, E1 | M1 |
| I12 compile errors refused; old version kept | T11 | M3 |
| I13 nine MCP tools | T16 | M5 |
| I14 agent texts | T17 | M5 |
| I15 body limits | T7 | M4 |
| I16 log order | T8 | - |
| I17 512 MB budget | T8 (test-sized budget) | M4 |
| I18 examples decode | T14 | - |
| I19 old clients decode | T14 | - |
| I20 no scripts on help, 404, TCP, tunnels | T19 | - |
| I21 event streams never held whole | T21 | M8 |
| I22 only listed classes held, copied or saved | T20 | M8 |
| I23 saving never slows the client | T22 | M8 |
| I24 partial bodies cannot be changed | T23 | M8 |
| I25 decoding stops at each limit | T24 | - |
| I26 class from Content-Type only | T20 | - |
