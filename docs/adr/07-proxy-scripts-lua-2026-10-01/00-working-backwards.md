# 0. Working backwards: what users might say

**Status:** Mixed. Quotes marked (real) are the project owner's own requests,
from the design conversation on 2026-10-01. The GitHub repository has no
issues yet (checked on 2026-10-01). Every other quote is simulated: written by
us to test the design before it is built.

| Mark | Meaning |
|---|---|
| **covered** | the ADR decides it; the link says where |
| **accepted** | the ADR knows about it and chose to leave it; the link says why |
| **gap, fixed** | the first draft did not answer it; the ADR now does |

## Real feedback

> "we should use lua to add ability to intercept requests. two types of
> scripts: intercept change in/out request; log in this case we do not stop
> processing of request but allow to parse body req/res and save to a file
> for instance." (real)

> "so agent should be able to get params of proxy. assign lua scripts based
> on rules." (real)

> "in adr 7 think of streaming, media image, video, binary and etc" (real,
> review of the first draft, 2026-10-01)

Answered by the two kinds ([01](01-script-kinds.md)), log rules that never
stop the request ([04](04-bodies-and-captures.md)), and rules an agent sets
through MCP ([02](02-rules.md), [05](05-clients-and-agents.md)). The review
of streaming and media found gaps G4 to G6 below; body classes, `on_event`
and `save` in [04](04-bodies-and-captures.md) are the answer.

## The announcement

> **LocalRouter 0.3: scripts for your traffic**
>
> Write a few lines of Lua and LocalRouter runs them on the requests of your
> dev servers and of anything you send through its proxy.
>
> An **intercept** script changes a request or a response, or answers by
> itself: add a header, block a call, return a fake reply. A **log** script
> gets a copy of each request and response, with bodies, after they are done,
> and writes them to files. It cannot slow or break the traffic.
>
> ```
> localrouter rules add claude --host api.anthropic.com --path /v1/messages --script capture.lua --output-dir ./captures
> ```
>
> Agents do the same with the `set_script_rule` tool, and read the script
> reference at `router.localhost/scripts`. API keys and cookies are hidden
> from scripts unless you allow it yourself.

## Who reacts

| Role | Setup | What they want |
|---|---|---|
| R1. Developer of an LLM app | calls a model API from a dev server and from tests | every prompt and reply in a file |
| R2. Frontend developer | Chrome, a backend that is not ready | fake replies for some endpoints |
| R3. Automated client: a coding agent | reads tool descriptions literally, writes scripts fast | capture or change traffic for a task, then clean up |
| R4. User who does not want the feature | routes only | no slower, nothing recorded |
| R5. Team member without LocalRouter | same repository | nothing breaks for them |
| R6. Security-minded user or operator | shared laptop, company rules | know what is on disk and who can see keys |
| R7. Developer of a media or file app | images, video with seeking, uploads, downloads | see and keep these bodies without breaking playback |

## Positive reactions

**R1: "My capture file has every prompt and every full streamed reply, and
Claude Code still streams."** Log rules copy and never hold.
**covered**, [04](04-bodies-and-captures.md).

**R2: "I return a fake JSON for /api/billing until the backend is ready."**
**covered**, [01](01-script-kinds.md), returning a response.

**R4: "Nothing changed in speed."** No rule, no Lua, no copy.
**covered**, manifest I1, T18, M6.

## Negative reactions: the user's own script or setup

**R1: "My log script crashed on json.decode: the body is gzip."** Bodies are
decoded for scripts. **covered**, [04](04-bodies-and-captures.md) "Decoding".

**R1: "I count requests in a global and the number jumps around."** Each
intercept rule has up to 4 states, and a reload makes new ones.
**accepted**, [03](03-lua-runtime.md) "Threads"; shared state is manifest U2.

**R2: "After I added my script, the reply from the model arrives all at
once."** The script asks for `response_body`. **covered**,
[04](04-bodies-and-captures.md); the reference and agent texts say it.

**R1: "I edited my script and nothing changed."** A broken edit keeps the
old version; the error is in `last_error`, the app shows it in red.
**covered**, [03](03-lua-runtime.md) "Loading and reloading".

**R1: "output_dir on my Desktop is refused."** macOS privacy rules keep the
daemon out of Desktop and Documents (as for folder routes in ADR 05). The
probe finds it at once. **covered**, [04](04-bodies-and-captures.md).

**R1: "My log rule crashed in json.encode on an image body."** Binary is
not text. The error says to use `base64.encode` or `save`.
**covered**, [04](04-bodies-and-captures.md) section 10, T25.

**R7: "The server sends JSON as application/octet-stream, so my rule never
got the body."** The class comes from the header; the rule must list
`binary`. **accepted**, [04](04-bodies-and-captures.md) trade-offs, manifest I26.

**R7: "I listed media in response_body and now my video starts late."**
Holding a body means waiting for it. **accepted**,
[04](04-bodies-and-captures.md) section 4; manifest blast radius, "Video and
audio players". The default does not hold media.

## Negative reactions: the feature's own behaviour

**R1: "My log rule never wrote anything for the notification stream. The
stream stays open all day."** In the first draft a log script ran only when
the exchange ended. **gap, fixed (G4)**: `on_event` gets each event as it
passes, for intercept and log; event streams are never held whole.
[04](04-bodies-and-captures.md) section 3, manifest I21, T21.

**R7: "I wanted every image and video the page loads. The first draft copied
each one into memory up to 16 MB, and I had to write them out from Lua."**
And a log rule on a page of images cost memory for every image, even when
the script did not look at bodies. **gap, fixed (G5)**: body classes with
defaults (media, binary and multipart are only counted), and `save`, which
writes files while the body streams, outside the memory budget.
[04](04-bodies-and-captures.md) sections 1, 2 and 6, manifest I22, I23,
T20, T22, M8.

**R7: "An intercept script that rewrote a video part broke seeking. And a
10 KB gzip reply turned into a gigabyte in memory."** The first draft did
not say what happens to a `206` body, or where decoding stops. **gap, fixed
(G6)**: a partial body can be read, never changed; decoding stops at the
limit of its use. [04](04-bodies-and-captures.md) sections 7 and 9,
manifest I24, I25, T23, T24.

**R7: "My 50 MB upload is in the capture as one .multipart file, not as the
photo I uploaded."** **accepted**, manifest U6; `multipart.parse` works on
copies up to 16 MB.

**R3, then the user: "The agent added an intercept rule on api.anthropic.com
to test something, the script had a typo, and now Claude Code gets a 502 on
every call. It cannot even call the tool to remove the rule."** The agent's
own traffic went through the rule it broke. **gap, fixed (G1)**: after 20
failed calls in a row the rule is disabled and says why.
[03](03-lua-runtime.md) "Errors", manifest I6, T12, M4.

**R6: "I added log rules on five busy hosts and the daemon took 10 GB of
memory."** Each queue had its own limit, and nothing limited all of them
together. **gap, fixed (G2)**: one budget of 512 MB for all held bodies and
copies. [04](04-bodies-and-captures.md), manifest I17, T8.

**R3: "Which pid do I give as owner_pid for a rule?"** For routes it is the
dev server. For a rule, the first draft said nothing. **gap, fixed (G3)**:
the agent texts say: the pid of a process you started for the job (the test
run), or remove the rule when done. [05](05-clients-and-agents.md) point 3,
manifest I14, T17.

**R6: "The capture file has the user's API key in it."** Secret headers are
hidden from scripts. **covered**, [04](04-bodies-and-captures.md). Keys
*inside bodies* are not hidden. **accepted**, manifest risks.

**R3: "I set reveal_secrets in the MCP call and got an error."** On purpose.
**covered**, [04](04-bodies-and-captures.md) point 4, manifest I9.

**R5: "Someone committed captures/ with a hundred prompts and the replies."**
**covered** by the agent texts (point 5) and **accepted** for humans: the user's
`.gitignore` decides. Manifest blast radius, "Git repositories".

**R1: "My log rule stopped capturing after I deleted the worktree and
restarted the Mac."** The script file is gone; the rule stays and matches
nothing, with `last_error`. **accepted**, manifest risks.

**R6: "My captures folder grew past 1 GB over a week."** The quota restarts
with the daemon. **accepted**, manifest U5.

**R2: "I want to send /api to my local mock server instead of the real
one."** Rerouting is not in this ADR. **accepted**, manifest U1.

**R2: "I want to change WebSocket messages."** **accepted**, manifest U3.

## A simulated session: the agent

The user says: "Record every request my integration tests send to the model
API, then tell me which prompts are longest."

1. The agent calls `get_proxy`. The proxy is on; no rules; `env` given.
2. It reads `router.localhost/scripts` (the `set_script_rule` description
   points there).
3. It writes `/tmp/lr-cap/capture.lua`: a log script that appends
   `{path, model, input_chars, status}` as one JSON line.
   *With the texts of the first draft, it would write the captures into the
   project's `tests/` folder. Point 5 of the agent texts stops this.*
4. It calls `set_script_rule` with `check_only: true` → ok.
5. It starts the test run in the background with the `env` values and gets
   its pid.
6. It calls `set_script_rule` with `owner_pid` of that run (G3),
   `output_dir: "/tmp/lr-cap/out"`. The reply notes that the host is now
   inspected and the CA is not trusted; the agent already passes
   `NODE_EXTRA_CA_CERTS` to the run.
   *Order matters: a request sent between steps 5 and 6 is not captured. The
   agent text says to set the rule first when every request counts, and use
   a session rule removed at the end.*
7. The run ends; the rule goes away with it.
8. The agent reads `/tmp/lr-cap/out/messages.jsonl` and reports.
9. It does not ask for `reveal_secrets` and does not copy the file into the
   project.

This is the script of manual test M5.

## Gaps

| # | Gap | Fix | Where |
|---|---|---|---|
| G1 | a broken intercept rule on the agent's own API host locks the agent out | disable a rule after 20 failures in a row | [03](03-lua-runtime.md), manifest I6, T12, task 5 |
| G2 | many log rules together can hold many GB | one 512 MB budget for all held bodies and copies | [04](04-bodies-and-captures.md), manifest I17, T8, task 4 |
| G3 | agents do not know which pid owns a rule | agent texts explain owner_pid for rules, and set-first for full capture | [05](05-clients-and-agents.md), manifest I14, T17, task 10 |
| G4 | a stream that never ends is never logged; a held event stream never arrives | `on_event` for intercept and log; event streams never held whole | [04](04-bodies-and-captures.md) section 3, manifest I21, T21, tasks 4 and 5 |
| G5 | every image and video costs memory; large media cannot be kept | body classes with defaults; `save` to files while streaming | [04](04-bodies-and-captures.md) sections 1, 2, 6, manifest I22, I23, T20, T22, task 4 |
| G6 | a changed `206` body breaks players; decoding has no stop | partial bodies read-only; decoding stops at each limit | [04](04-bodies-and-captures.md) sections 7, 9, manifest I24, I25, T23, T24, task 4 |
