# 0. Working backwards: what users might say

**Status:** Mixed. Quotes marked (real) are the user's own messages in the
design session of 2026-10-02, copied word for word. The GitHub repository has
no issues about this (checked 2026-10-02). Every other quote is simulated: we
wrote it to test the design before it is built.

## Real feedback

1. (real) "proxy by default should write log for all requests. log is written
   in apps' folder. in setting two params. rolling log by size or number of
   requests. log in format that can be opened in chrome. this can be turned
   off. add in menu - open logs in this case open with local chrome if
   installed. allso add button for open log folder."
2. (real) "in chrome you can save logs in har format … the idea is to open
   this har in chrome later"
3. (real) "actually your idea is much better. i like https://proxy.localhost
   where i can see my logs. prepare adr first. how to server content? changes
   in mcp?"
4. (real, translated from Russian) "allow lan access must be per LAN. If I
   allow it at home, that does not mean the café. Save the networks and in
   which ones it is allowed. Add it to the ADR."
5. (real) "one of the requirement of this app is to consume as minimum
   resources as possible including mem. so if nodobody make requests to proxy
   viewer unload what is not used" and "or event better - never store for
   long"
6. (real) "after the implemnetation claude code should be able to operate
   proxy, run apps using it, inspect traffic which was sent"

| Real request | Where the ADR answers it |
|---|---|
| on by default, all requests | [01](01-har-files.md#what-is-recorded), [settings](01-har-files.md#settings) |
| in the app's folder | the instance's logs folder, [01](01-har-files.md#files-and-rolling) |
| two settings, by size or number of requests | both limits, the first one reached rolls ([01](01-har-files.md#files-and-rolling)); read as "both", see R6 |
| a format Chrome opens | HAR 1.2 ([01](01-har-files.md#context)) |
| can be turned off | `proxy_log` ([01](01-har-files.md#settings)) |
| menu item, local Chrome if installed | [02](02-viewer.md#opening-it), [03](03-clients-and-agents.md#the-menu-bar-app) |
| button for the folder | Show Proxy Log Folder ([03](03-clients-and-agents.md#the-menu-bar-app)) |
| a page at proxy.localhost | [02](02-viewer.md) |
| how to serve the content | [02](02-viewer.md#how-the-content-is-served) |
| changes in MCP | [03](03-clients-and-agents.md#mcp-no-new-tool) |
| LAN access per network | [04](04-lan-per-network.md) |
| minimum memory, nothing kept for long | [01](01-har-files.md#resource-use), [02](02-viewer.md#resource-use), [04](04-lan-per-network.md#how-a-network-is-recognised) |
| Claude Code operates, runs, inspects | [03](03-clients-and-agents.md#the-agent-loop-operate-run-inspect) |

## The announcement

> **LocalRouter: see everything your proxy carried**
>
> When the forward proxy is on, LocalRouter now writes every request it
> carries to a HAR file: the same format Chrome DevTools saves with "Save all
> as HAR". Headers that hold secrets are written as `[redacted]`.
>
> Open **Proxy Log** from the menu bar icon. Your Chrome opens
> `http://proxy.localhost`, which lists the files and shows new requests as
> they happen. Filter, click a row for its headers, or download a file and
> drop it onto the DevTools Network panel.
>
> Settings → Proxy → Proxy log: turn it off, or set the size of a file (20 MB)
> and the number of requests in a file (5,000). LocalRouter keeps the 5 newest
> files in `~/Library/Logs/LocalRouter/proxy`.
>
> Agents: `get_proxy` now tells them where the files are. They read them with
> `jq`.
>
> One thing to know: HTTPS to a host that is not inspected is a tunnel, so the
> log shows only its host name.

## Roles

| Role | Setup | Wants |
|---|---|---|
| Main user | runs tests and Chrome through the proxy | to see later what was sent and what came back |
| Coding agent user | Claude Code with the MCP server | the agent to find the failed request by itself |
| User who does not want it | turned the proxy on once for a test | nothing on disk they did not ask for |
| Team member without LocalRouter | works in the same repository | nothing changes in shared files |
| Operator | runs the release and the dev instance, updates | no lost routes, bounded disk use, a clean rollback |

## Reactions

### Positive

**R1: "I found the 401 from last night's test run in two clicks."**
Files survive restarts; the viewer lists them by time. **covered**,
[01](01-har-files.md#files-and-rolling), [02](02-viewer.md#the-page).

**R2: "The rows show up while the page loads. I don't refresh anything."**
**covered** by the live feed, [02](02-viewer.md#how-the-content-is-served).

**R3: "I dragged the file into DevTools and got my usual Network panel."**
**covered**, [02](02-viewer.md#the-page), test M1.

### Negative: the user's own setup

**R4: "I opened proxy.localhost and it is empty."**
The proxy is off, or the log is off, or the program does not use the proxy.
Before the fix the page showed an empty table and no reason. **gap, fixed**
(G1): the state line says what is off and how to turn it on.
[02](02-viewer.md#the-page).

**R5: "My HTTPS site shows one CONNECT row and no paths."**
The host is not inspected, so it is a tunnel. **accepted** with help: the
entry has `_mode: tunnel`, and the help text says how to inspect a host
(ADR 06). [01](01-har-files.md#what-is-recorded).

**R6: "I wanted to roll by size only, not by count."**
The ADR reads the request as both limits at once. **accepted**: set the other
limit high (200 MB or 1,000,000 requests). [01](01-har-files.md#files-and-rolling).

**R7: "jq: command not found."** (macOS 14)
**accepted** with help: the texts say macOS 15 includes `jq`, else
`brew install jq` or any JSON tool. [03](03-clients-and-agents.md#agent-texts).

### Negative: the feature's own behaviour

**R8: "I turned the proxy on for one test. Now my headers are on disk, and
nobody told me."**
On by default is the user's request, but the moment it starts writing must be
visible. **gap, fixed** (G2): `proxy on` prints the `Log:` line with the
folder; Settings shows the log switch beside the proxy switch.
[01](01-har-files.md#settings), [03](03-clients-and-agents.md#cli).

**R9: "My agent read the whole 20 MB file and ran out of context."**
**gap, fixed** (G3): the texts say not to read a whole file and give `jq`
recipes; there is no MCP tool that returns a file.
[03](03-clients-and-agents.md#agent-texts).

**R10: "The Mac crashed. Now DevTools says the file is not valid."**
A write cut in the middle leaves a broken last line. **gap, fixed** (G4):
repair at the next start. [01](01-har-files.md#the-file-is-valid-har-after-every-entry), invariant I16.

**R11: "After the update my route proxy.localhost is gone."**
Saved routes are validated again at load, and a reserved name would be
skipped without warning. This is the silent failure in the
[blast radius](07-semantic-change-manifest.md#11-blast-radius). **gap, fixed**
(G5): the saved route is kept and wins; the viewer is also at
`router.localhost/proxy-log/`. [02](02-viewer.md#the-name), invariant I11.

**R12: "With LAN access on, a colleague opened my proxy log from their
laptop."**
**gap, fixed** (G6): loopback peers only. [02](02-viewer.md#who-can-read-it), invariant I8.

**R13: "The Logs tab is full of requests to proxy.localhost, and the HAR has
the viewer reading itself."**
**gap, fixed** (G7): the viewer is never logged. [02](02-viewer.md#the-viewer-is-never-logged), invariant I7.

**R14: "My OpenAI key was in a query string, and now it is in the HAR file,
and in my Time Machine backup."**
Silent failure from the blast radius. **gap, not now** (G8): it becomes
unresolved effect U2; Settings and help say URLs are written as they are.
[manifest §12](07-semantic-change-manifest.md#12-unresolved-effects).

**R15: "I deleted the log file in Finder and new requests went nowhere."**
On macOS a deleted file that is still open accepts writes and is lost. **gap,
fixed** (G9): the writer checks its file before each entry and starts a new
one. [01](01-har-files.md#the-file-is-valid-har-after-every-entry).

**R16: "The Response tab in DevTools is empty."**
Bodies are not written. **accepted** for now: U1; a log script rule (ADR 07)
records bodies. [01](01-har-files.md#what-one-entry-holds).

**R17: "DevTools says every request spent all its time 'waiting'."**
The entry is written at the response headers. **accepted**, a named risk in
the [manifest](07-semantic-change-manifest.md#13-risks).

**R18: "A page with 3,000 requests: the file has gaps."**
The queue dropped records. **accepted**: `dropped` is shown in the viewer,
`proxy log` and Settings. [01](01-har-files.md#traffic-is-never-delayed).

**R19: (team member) "Did LocalRouter change anything in our repo?"**
No: the files are in the user's logs folder, and no project file changes.
**covered**, [01](01-har-files.md#files-and-rolling).

**R20: (operator) "Rollback to the old version. Where did 100 MB go?"**
The files stay; a rollback does not delete them. **accepted**, in the
[rollback section](07-semantic-change-manifest.md#14-rollback).

### Added with changes 4 and 5 of the session (LAN, memory, agent loop)

**R21: "I can show my site on my phone at home, and in the café nobody can
reach it."** **covered**, [04](04-lan-per-network.md).

**R22: "After the update my phone at the office cannot reach the Mac any
more."** The update allowed only the network the Mac was on at that moment.
The silent failure from the blast radius. **covered** with help: status and
the Routing page show the note and "Allow on this network".
[04](04-lan-per-network.md#settings).

**R23: "With my VPN on, LAN access does not work."** A VPN interface has no
router MAC. **accepted**: Settings says the network cannot be recognised.
[04](04-lan-per-network.md#how-a-network-is-recognised).

**R24: "We rolled back to the old version, and the Mac was open to the café
Wi-Fi again."** An older daemon reads `allow_lan: true` as every network.
**gap, not now** (G11): written in the
[rollback section](07-semantic-change-manifest.md#14-rollback): turn
`allow_lan` off before a rollback.

**R25: "LocalRouter uses more memory since the proxy log came."**
**covered**: idle holds nothing (I21), no whole file in memory (I22), M11
measures it. [01](01-har-files.md#resource-use).

**R26: (Claude Code user) "I asked it to run my tests through the proxy and
look at what they sent. It did it all, and I only saw the commands."**
**covered**, [03](03-clients-and-agents.md#the-agent-loop-operate-run-inspect).

**R27: (Claude Code user) "My Python tests fail with
CERTIFICATE_VERIFY_FAILED on the inspected host, but the Node tests work."**
`proxy env` gave a CA file only to Node. **gap, fixed** (G10): the CA bundle
and three more env names. [03](03-clients-and-agents.md#the-agent-loop-operate-run-inspect), I23, E2.

Count: 5 positive, 22 negative or confused.

## A simulated session with a coding agent

The user says: "My e2e test runs through the proxy and gets 401 from
api.example.com. Find out why." The proxy is off.

0. The agent calls MCP `get_proxy`, sees the proxy is off, runs
   `localrouter proxy on` and `localrouter proxy inspect add api.example.com`
   in the terminal, then the tests: `eval "$(localrouter proxy env)" && npm
   run e2e`. A Python helper in the tests also calls the host; **before G10 it
   failed** with a certificate error, now it uses the CA bundle.
1. The agent calls MCP `get_proxy` again. It reads `log.enabled: true`,
   `log.folder`, `log.current`.
2. **Before the fix it ran `cat` on the current file** (20 MB) because nothing
   told it the size. The help text now says not to (G3). With the fix:
3. It runs `ls -t "<folder>"/proxy-*.har | head -1`.
4. It runs the `jq` recipe for status ≥ 400 and gets
   `401 POST https://api.example.com/v1/items`.
5. It runs the second recipe for that URL and reads the response headers:
   `www-authenticate: Bearer error="invalid_token"`. The request's
   `authorization` is `[redacted]`, so it cannot see the token, and says so.
6. It tells the user: the token is rejected as invalid; it checks the test's
   environment for the token variable.
7. The user says "turn the log off". The agent runs `localrouter proxy log
   off` in the terminal. There is no MCP tool for it.

This is the script of manual test M6.

## Gaps

| # | Gap | Fix | Where |
|---|---|---|---|
| G1 | the viewer is empty with no reason | state line with what is off and how to turn it on | [02](02-viewer.md#the-page), M2 |
| G2 | the user is not told headers go to disk | `Log:` line in `proxy on`; switch beside the proxy switch | [01](01-har-files.md#settings), [03](03-clients-and-agents.md#cli), T10 |
| G3 | an agent reads a whole file | texts: size warning, `jq` recipes | [03](03-clients-and-agents.md#agent-texts), T12, M6 |
| G4 | a crash leaves an invalid file | repair at start; new file per run | [01](01-har-files.md#the-file-is-valid-har-after-every-entry), I16, T2 |
| G5 | a saved route `proxy` disappears | kept at load; second address | [02](02-viewer.md#the-name), I11, T7, M7 |
| G6 | LAN peers read the log | loopback only | [02](02-viewer.md#who-can-read-it), I8, T6, M5 |
| G7 | the viewer logs itself | never logged | [02](02-viewer.md#the-viewer-is-never-logged), I7, T6 |
| G8 | tokens in query strings reach the file | not now: U2, and the Settings text says it | [manifest §12](07-semantic-change-manifest.md#12-unresolved-effects) |
| G10 | Python and `curl` started with `proxy env` reject the inspection CA | `inspect-ca/bundle.pem` (system roots + inspection CA); 3 CA names and lowercase proxy names in `proxy env` | [03](03-clients-and-agents.md#the-agent-loop-operate-run-inspect), I23, T8, E2, task 15 |
| G11 | a rollback opens LAN access on every network again | not now: written in the rollback section | [manifest §14](07-semantic-change-manifest.md#14-rollback) |
| G9 | a deleted file swallows entries | check before each entry, start a new file | [01](01-har-files.md#the-file-is-valid-har-after-every-entry), T2, M4 |

Each fixed gap is in its change file, the manifest (invariant or risk), the
test plan and the tasks.
