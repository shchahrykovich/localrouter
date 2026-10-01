# 8. Semantic Change Manifest

## 1. Manifest status

```text
Manifest status: PLANNED
```

This file describes intended behaviour. Nothing here is built. It assumes
ADR 06 is built first.

## 2. Semantic change summary

```text
Artifacts
+ 1 embedded language runtime (Lua 5.4 through mlua, vendored)
+ 1 rule table (memory) and 1 rule file (script-rules.json)
+ 3 socket API methods (set_script_rule, remove_script_rule, list_script_rules)
+ 2 MCP tools (set_script_rule, remove_script_rule)
+ 1 CLI command group (rules ...)
+ 1 served page (router.localhost/scripts) and its template (scripts.md)
+ 2 crate dependencies (mlua, async-compression)
~ get_proxy reply (+ script_rules), log entry (+ rules, script_error), config (+ secret_headers)
~ inspect set (+ hosts of enabled rules)

Persistent data
+ script-rules.json (persistent rules only)
+ capture files: one or more per log rule, in each rule's output_dir,
  count and size chosen by the script, up to max_capture_bytes per rule per run
+ saved body files: one per saved body, in output_dir/bodies/, within the same quota

Runtime effects
+ user code (Lua) runs inside the daemon on matching exchanges
+ requests and responses can be changed or answered by a script
+ the daemon writes files outside its data folder (capture files)
+ bodies held in memory (intercept, up to 8 MB) and copied (log, up to 16 MB),
  only for the body classes a rule lists
+ bodies written to files while they stream (log rules with save)
+ event streams split into events, each passed through on_event

Modified
~ router and forward proxy request path (a rule lookup on every HTTP request)

External effects
~ requests sent to servers may differ from what the client sent (intercept)

Destructive operations
1 (capture.write replaces a file in output_dir)

Unresolved effects
7
```

## 3. Source of truth

```text
CANONICAL
  script files (.lua)       owned by the user or agent, anywhere on disk; the daemon only reads them
  script-rules.json         persistent rules; written only by the daemon
  capture files             the output; written only by the daemon, on behalf of one rule

DERIVED (memory only)
  loaded script version     compiled from the file; rebuilt on change
  inspect set               config inspect_hosts + hosts of enabled rules (ADR 06)
  rule counters             since the rule was set or the daemon started

TEMPORARY
  held bodies and copies    until the exchange ends or the log call returns
```

## 4. Artifacts

```text
Persistent artifacts
+ <data folder>/script-rules.json       {"version": 1, "rules": [...]}
+ <output_dir>/<name>                   mode 0600, per log rule
+ <output_dir>/bodies/                  mode 0700, created on the first saved body
+ <output_dir>/bodies/<id>-req|res[.part].<ext>   mode 0600, one per saved body

Code
+ libs/core/src/scripts/rules.rs, engine.rs, lua_api.rs, bodies.rs, events.rs
+ libs/core/src/scripts.md
~ libs/core/src/proxy.rs, forward.rs, inspect.rs, logs.rs, api.rs, config.rs, help.rs, help.md, note.md, mcp.md
~ apps/daemon/src/daemon.rs, socket.rs, store.rs
~ apps/cli/src/main.rs, mcp.rs
~ apps/menubar Api.swift, Settings (Scripts list)

Interfaces
+ socket set_script_rule, remove_script_rule, list_script_rules
+ MCP set_script_rule, remove_script_rule
+ CLI rules, rules add|check|rm|enable|disable|api
+ HTTP GET router.localhost/scripts
+ Lua API: kind, on_request, on_response, on_event, on_exchange; request_body,
  response_body, copy, save; json, sse, base64, url, multipart, log, capture

Zeroes
New programs:        0
New processes:       0  (script threads live in the daemon)
New ports:           0
Network calls made by scripts: 0 (no network API)
```

## 5. Runtime effects

```text
CALL  run an intercept script
trigger:                a request matches an enabled intercept rule
cardinality:            one on_request and at most one on_response per matching rule per exchange
write_idempotent:       n/a
producer_deterministic: not guaranteed (a script may use os.time, or globals)
blocking:               yes, up to 50 ms per call
destructive:            may change or replace what reaches the server or the client

CALL  run a log script
trigger:                an exchange that matched an enabled log rule ends
cardinality:            one per exchange per rule, unless dropped
blocking:               no
producer_deterministic: not guaranteed

WRITE capture file (APPEND)
target:                 <output_dir>/<name>
trigger:                capture.append in a log script
cardinality:            as many as the script calls; up to max_capture_bytes per rule per run
write_idempotent:       no (each call adds)
producer_deterministic: depends on the script
retention:              until the user deletes it
destructive:            no
reversible:             yes (delete the file)

CALL  run on_event (intercept)
trigger:                one event of an events-class body matched by a rule with on_event
cardinality:            one per event per matching intercept rule
blocking:               that event only, up to 50 ms per call
destructive:            may change or drop the event the client receives

WRITE saved body file (CREATE, then APPEND while the body streams)
target:                 <output_dir>/bodies/<exchange id>-req|res[.part].<ext>
trigger:                a body whose class is in a matching log rule's save
cardinality:            one file per saved body; a page with 80 images makes 80 files
write_idempotent:       no (a new file per exchange id)
producer_deterministic: yes (the decoded body as received)
retention:              until the user deletes it
destructive:            no
reversible:             yes (delete the file)

WRITE capture file (REPLACE)
trigger:                capture.write
write_idempotent:       yes for the same data
destructive:            yes: the old content of that file is gone
reversible:             no

WRITE script-rules.json (REPLACE)
trigger:                set or remove of a persistent rule; disable after 20 failures of a persistent rule
write_idempotent:       yes
producer_deterministic: yes

WRITE inspect set (memory), and inspect-ca/ on first need (ADR 06)
trigger:                a rule with a host outside .localhost becomes enabled

WRITE daemon log line
trigger:                log.info / log.warn / print (at most 200 a minute per rule); rule disabled
```

## 6. Reads and writes

```text
READ
- script files (on set, then at most once a second on use)
- script-rules.json (at start)
- request and response headers and bodies of matching exchanges
- config.json (secret_headers)

WRITE
- script-rules.json
- capture files in each log rule's output_dir
- request log fields rules, script_error (memory)
- daemon log lines

EXTERNAL WRITE
- changed requests reach internet servers and dev servers (intercept)
```

Hidden dependency created by this change: a persistent rule depends on a
script file that lives outside LocalRouter, often inside a git worktree. When
the worktree is deleted, the rule keeps the last loaded version until the
daemon restarts; after a restart the rule cannot load and stays in the list
with `last_error` and matches nothing.

## 7. Interfaces and events

```text
+ SOCKET set_script_rule (with check_only), remove_script_rule, list_script_rules
~ SOCKET get_proxy                + script_rules
~ SOCKET get_config / set_config  + secret_headers
~ EVENT  log                      + optional rules, script_error
~ API_VERSION 1.3 → 1.4           (minor: additive); METHODS 13 → 16
+ MCP    set_script_rule, remove_script_rule   (tool count 7 → 9)
+ CLI    rules ...
+ HTTP   router.localhost/scripts
+ Lua    the script API of 01-script-kinds.md (a public interface from now on:
         changing it breaks users' scripts)
```

## 8. External side effects

```text
~ Requests that reach servers can differ from what the client sent, and
  responses that reach clients can differ from what the server sent, for
  matching exchanges of enabled intercept rules.
+ Disk use in output_dir: up to max_capture_bytes per log rule per daemon run
  (default 1 GiB).
No network call is made by a script.
```

## 9. Invariants

```text
I1.  With no enabled rule that matches, an exchange runs no Lua, holds no body
     and copies no body.
     enforced by: the hook returns before any allocation; T18 counts calls and buffers.

I2.  A log rule never delays, changes or fails an exchange: the client gets the
     same bytes, and each part is sent before the copy is queued.
     enforced by: tee design in bodies.rs; T8 (byte-equal, and a slow log script
     does not delay the next streamed part).

I3.  Only intercept scripts change an exchange, and only exchanges whose host,
     path and method match their rule.
     enforced by: T2, T5.

I4.  A script has no io, package, debug, require, load, loadfile, dofile,
     string.dump, or os function other than time, date, clock; bytecode is refused.
     enforced by: engine.rs builds the state; T3 tries each one.

I5.  A call that exceeds its time or memory limit fails that call only; no script
     can stop the daemon or the network threads.
     enforced by: instruction hook, set_memory_limit, script threads; T4.

I6.  An intercept failure follows the rule's on_error. After 20 failed calls in a
     row, the rule is disabled and says why.
     enforced by: T12.

I7.  capture writes go only to a flat name directly in the rule's output_dir,
     opened with O_NOFOLLOW, created 0600, within max_capture_bytes. output_dir is
     absolute, exists, is not / and is not in the data folder.
     enforced by: lua_api.rs; T9.

I8.  A script sees secret headers as "[redacted]" unless reveal_secrets; redaction
     never changes the bytes sent to the server or the client.
     enforced by: T10.

I9.  reveal_secrets cannot be set through MCP; the CLI sets it only when standard
     input is a terminal and the user types yes.
     enforced by: T15, T16.

I10. A rule with owner_pid is never written to script-rules.json; persistent with
     owner_pid is refused; the rule goes when its owner exits.
     enforced by: T13.

I11. The inspect set holds the host of every enabled rule outside .localhost, and
     not the host of a removed or disabled rule (unless inspect_hosts lists it).
     enforced by: T13.

I12. A script that does not compile, or whose top level does not return a valid
     kind, is refused at set_script_rule. A broken edit keeps the last good version.
     enforced by: T11.

I13. The MCP server lists exactly nine tools.
     enforced by: T16.

I14. help.md, note.md, mcp.md and scripts.md describe the two kinds, check_only,
     owner_pid for rules, streaming and response_body, secrets, and not committing
     captures; they name their instance through templates.
     enforced by: T17.

I15. A body over the hold limit (8 MB) passes unchanged; the intercept script sees
     body = nil and body_truncated = true. A log copy keeps at most 16 MB per body.
     enforced by: T7.

I16. A log rule's script sees exchanges one at a time, in the order they ended.
     enforced by: T8.

I17. All held bodies and log copies together use at most 512 MB; beyond that, new
     copies are dropped and counted.
     enforced by: bodies.rs budget; T8.

I18. Every API example decodes in Rust and Swift; METHODS lists 16 methods, each
     with an example.
     enforced by: T14.

I19. An older client decodes get_logs and get_proxy from a newer daemon: new
     fields are optional.
     enforced by: T14.

I20. Scripts never run on router.localhost, the router's 404 page, TCP routes or
     tunnels.
     enforced by: T19.

I21. An event stream is never held whole. Each event reaches the client after
     only its own intercept on_event calls; earlier events are already sent.
     enforced by: events.rs; T21 (the client gets event 1 before the server
     sends event 2, with an intercept on_event and a log on_event).

I22. A body is held, copied or saved only when a matching rule lists its class.
     Defaults: intercept holds none; log copies text and events; media,
     multipart and binary are only counted.
     enforced by: T20 (a 5 MB image under a default log rule: 0 bytes held or
     copied, body_size = 5 MB).

I23. Saving a body to a file never slows the client. A full writer queue or the
     quota stops the save and marks the file truncated.
     enforced by: T22 (a writer that blocks: the client still gets every byte
     at full speed; the file is marked truncated).

I24. A script cannot change the body of a 206 response or of a request with
     Content-Range; it may change their headers.
     enforced by: T23.

I25. Decoding stops at the limit of its use (8 MB held, 16 MB copied, the quota
     for a saved file), so a small compressed body cannot expand past it.
     enforced by: T24 (a 10 KB gzip body that expands to 1 GB).

I26. The body class comes from the Content-Type header only; the same headers
     always give the same class.
     enforced by: T20 (a PNG sent as application/octet-stream is binary).
```

I6 (disable after 20 failures) and I17 (the 512 MB budget) came from the
working-backwards file. I21 to I26 came from the review of streaming, media
and binary bodies on 2026-10-01 (working-backwards G4 to G6). I1 is written as an invariant, not left as a hope, so
that the path without rules is tested (T18): a hook that copies every body "in
case a log rule matches" would cost every request even when no rule exists.

## 10. Data impact

```text
New persistent data
+ script-rules.json (small)
+ capture files, outside the data folder; size bounded per rule per run

Migration         none
Backfill          none
Derived rebuild   loaded scripts are rebuilt from files at start
Destructive       capture.write replaces a file; remove_script_rule never deletes captures
Expected growth   capture files grow with traffic, up to max_capture_bytes per rule per run.
                  A rule that saves media reaches 1 GiB fast: one 10-minute 1080p video is
                  about 500 MB, and a video player that seeks saves every part it fetched.
                  bodies/ can hold many small files (one per image).
Retention         captures: until the user deletes them; counters: memory

Compatibility with older readers
- an older daemon does not read script-rules.json and does not delete it: rules
  return after an upgrade
- older clients ignore get_proxy.script_rules and the new log fields (I19)
```

## 11. Blast radius

```text
Every HTTP request through the router or the proxy
dependency:   the hook runs a rule lookup
impact:       a few microseconds with rules; nothing else without a match (I1)
failure mode: a bug in the hook breaks all HTTP traffic, loud. T19 runs the
              existing proxy tests with an empty rule table.

Dev servers behind a router route matched by an intercept rule
dependency:   receive what the script sends
failure mode: a script that changes a body wrongly gives the dev server a bad
              request. Silent from the dev server's view: it sees a request the
              browser never sent. The request log names the rule in rules.

Streaming clients (Claude Code, server-sent events in a browser)
dependency:   receive responses as they stream
failure mode: an intercept rule with response_body = true makes the reply arrive
              at the end. Silent: no error, only a long wait.

The agent whose own API traffic goes through the proxy
dependency:   needs api host requests to work to keep working
failure mode: a broken intercept rule on that host makes every call fail with
              the 502 page; the agent cannot call the tool to remove it.
              Ends after 20 failures (I6).

Disk of the folder in output_dir
failure mode: fills up to max_capture_bytes per rule per run; a full disk makes
              writes fail and is counted, the traffic is not affected.

Git repositories
dependency:   output_dir inside a project
failure mode: captures with prompts and API replies get committed. Silent.
              Only the agent texts (I14) and the user's .gitignore stop it.

Video and audio players
dependency:   stream media and ask for parts with Range when they seek
impact:       an intercept rule that lists media holds each part until complete
failure mode: playback starts late and seeking is slow (each part up to 8 MB
              waits). Larger parts pass unchanged. Silent: the video plays,
              only slower. The default (media not held) avoids it.

Pages with many images
dependency:   a log rule with save = { "media" } on the page's hosts
failure mode: thousands of small files in bodies/; Finder and git become slow
              in that folder. Silent.

Servers that send the wrong Content-Type
dependency:   the class comes from the header (I26)
failure mode: binary sent as text/plain is copied as text and json.encode of it
              fails; JSON sent as application/octet-stream is not copied by
              default. The script sees body_class and body_skipped, so the
              reason is visible, but nothing fixes the server.

Event streams with a slow intercept on_event
failure mode: each event is late by the script's run time; tokens of a model
              reply appear slower. Silent apart from the timing.

Daemon memory
dependency:   held bodies and log queues
failure mode: up to 512 MB more memory (I17), then copies are dropped, counted.

Users with no rules
impact:       none. Confirmed unaffected: routes, TCP routes, folder routes, the
              proxy of ADR 06, the updater, the installers.
```

## 12. Unresolved effects

```text
? U1. Send a request to a different address (rerouting)
status:  REJECTED for this ADR
effect:  on_request sets another upstream, for example a local mock server
reason:  it crosses the loopback rule for router traffic (ADR 01 I9) and makes
         a rule able to send data anywhere. Returning a response covers mocks.

? U2. State shared between calls and rules
status:  REQUIRES_DECISION
effect:  a key-value store that every state of a rule, or every rule, can use
reason:  counting, sampling and "first request only" need it; globals are per state.
outcome: a later ADR

? U3. WebSocket frames
status:  REQUIRES_DECISION
effect:  scripts see or change WebSocket messages
reason:  frames are not request/response pairs; a different hook shape
blocks:  nothing in this ADR

? U4. HTTP trailers and gRPC
status:  REQUIRES_DECISION
effect:  scripts see trailers; a changed gRPC body keeps its framing
reason:  not exposed in req/res; an intercept with body on gRPC may break it

? U5. Capture quota across restarts
status:  REQUIRES_DECISION
effect:  max_capture_bytes counts what is in output_dir, not what was written
         in this run
reason:  counting needs a scan of output_dir at start, or a saved counter

? U6. Save each uploaded file of a multipart body as its own file
status:  REQUIRES_DECISION
effect:  save writes one file per part (with its filename) instead of one .multipart file
reason:  names come from the client and must be checked; one upload can have
         hundreds of parts

? U7. Other streaming formats
status:  REQUIRES_DECISION
effect:  split gRPC streams, chunked JSON arrays and multipart/x-mixed-replace
         (MJPEG camera streams) into messages for on_event
reason:  each needs its own parser; today they are one body of class binary,
         multipart or text, and are never held by default

Inherited: ADR 06 U2 (other accounts on the Mac can use the proxy). With log
rules, their traffic can end up in this user's capture files.
```

## 13. Risks

```text
- mlua and the vendored Lua C code run inside the daemon. A bug in the C code is
  a crash of the daemon, not a failed call. Lua 5.4 is old and widely used; the
  sandbox removes the libraries with known escape paths.
- A script author reads the Lua API as stable. Changing a field name later breaks
  scripts that live outside this repository.
- 50 ms is measured in Lua instructions and clock checks; a LocalRouter function
  that takes long (json.decode of 8 MB) runs to its end before the check.
- reveal_secrets through the terminal check is a speed bump, not a boundary.
- A persistent rule whose file sits in a deleted worktree matches nothing after a
  restart; the user may not notice that capture stopped.
- Captures can hold credentials inside bodies (OAuth refresh tokens in a JSON
  body) even with header redaction on. Saved bodies hold whatever the user saw:
  private photos, documents, downloads.
- The event splitter must follow the server-sent events format exactly (CRLF
  and LF line ends, comment lines, multi-line data). A difference changes what
  the client receives when an intercept on_event rewrites events.
```

## 14. Rollback

```text
Code rollback
  revert the commits. Sufficient: no route or config shape depends on rules.

Schema rollback
  script-rules.json stays; an older daemon ignores it.

Data rollback
  delete script-rules.json to forget rules. Capture files and saved bodies in
  bodies/ are NOT removed by any rollback: the user deletes them.

Infrastructure rollback
  nothing.

External side effects
  requests that intercept scripts changed have reached servers; not undone.
  capture files copied elsewhere (committed, uploaded, read by an agent into a
  model conversation) cannot be called back.
```
