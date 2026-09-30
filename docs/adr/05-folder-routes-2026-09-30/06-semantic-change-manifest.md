# 6. Semantic Change Manifest

This ADR was written after the build. Sections 1 to 14 describe the system as
built. The plan it is compared with (at the end) is the design stated before
any code was written, in the working session of 2026-09-30:

1. a folder route is an HTTP route whose target is `file://` plus an absolute
   folder, with no new route field;
2. the daemon serves the files itself, with tower-http's `ServeFile`;
3. LocalRouter's own code resolves the path: no `..`, no dotfiles, no symlinks
   out of the folder; a folder answers with `index.html`, else a file list;
4. the CLI gets `--folder`, MCP `register_route` gets `folder`.

Invariant IDs (I1..) and test IDs (T1..) are local to this ADR.

## 1. Manifest status

```text
Manifest status: IMPLEMENTED_WITH_DRIFT
```

The drift is small and every item is `ACCEPTED`; see Plan vs Actual.

## 2. Semantic change summary

```text
Artifacts
+ 1 target kind: file:// (folder route)
+ 1 library module: libs/core/src/folder.rs
+ 1 dependency: tower-http (feature fs), already in Cargo.lock through rmcp
+ 1 CLI option: add --folder
+ 1 MCP argument: register_route.folder
+ 2 API example files
~ socket API version 1.1 -> 1.2 (no new method, no new field)
~ 3 agent texts (help.md, note.md, mcp.md)

Persistent data
+ 0 new files
~ routes.json may hold file:// targets (same shape, same version rule)

Runtime effects
+ 1 new read path: the proxy reads files and folders on disk
+ 1 new read at register: metadata of the folder
~ 1 changed read: the up check stats the folder instead of a TCP connect
+ 0 new writes

External effects
0

Destructive operations
0

Unresolved effects
2
```

## 3. Source of truth

```text
Source of truth: unchanged for routes (routes.json and the daemon's memory).

NEW, EXTERNAL
  the served folder
    role:      EXTERNAL (owned by the user or the agent that writes it)
    authority: the disk; LocalRouter never copies or caches its files
    rebuilt by: whatever made it (a build, a test run, an agent)
```

## 4. Artifacts

```text
Code
+ libs/core/src/folder.rs            resolve, serve, serve_file, listing
~ libs/core/src/routes.rs            FOLDER_SCHEME, Route::folder, normalize_folder_target,
                                     RouteError::BadFolder, RouteError::FolderOnlyForHttp
~ libs/core/src/proxy.rs             folder branch; Body = UnsyncBoxBody
~ apps/daemon/src/daemon.rs          folder check at register, folder up check
~ apps/cli/src/main.rs, mcp.rs       --folder, folder

Dependencies
+ tower-http 0.6 (default features off, fs on)
+ percent-encoding 2 (direct; was already indirect)

Interfaces
~ API_VERSION 1.2 (Rust), apiVersion 1.2 (Swift)
+ api/examples/register_route_folder.request.json, .reply.json

Texts and docs
~ libs/core/src/help.md, note.md, mcp.md
~ docs/dictionary.md, CLAUDE.md

Programs, processes, ports:   0 new
Socket methods:               0 new
MCP tools:                    0 new (still six)
Data-folder files:            0 new
```

## 5. Runtime effects

```text
READ
type:                   file system read
target:                 the folder of a folder route, and files under it
operation:              canonicalize, metadata, read_dir, open, read
trigger:                a GET or HEAD request whose route has a file:// target
cardinality:            one path resolution and at most one file per request;
                        read_dir only for a folder without index.html
write_idempotent:       n/a (no write)
producer_deterministic: yes for a given state of the folder
retention:              none (nothing is cached; the browser revalidates)
destructive:            no
reversible:             n/a

READ
type:                   file system metadata
target:                 the folder of a folder route
trigger:                register_route with a file:// target; list_routes
cardinality:            one per register call; one per folder route per list call
write_idempotent:       n/a
producer_deterministic: yes for a given state of the disk
destructive:            no

WRITE (unchanged path, listed because the flow uses it)
type:                   memory ring buffer
target:                 request log
trigger:                every answered request, folder routes included
cardinality:            one entry per request
write_idempotent:       no (each request adds an entry)
producer_deterministic: yes
retention:              ring of log_size entries
destructive:            drops the oldest entry, as before

EMIT
type:                   HTTP answers new to LocalRouter
values:                 206, 304, 308 (add /), 403, 405, 404 "No such file",
                        502 "The folder is not there", file list page
```

## 6. Reads and writes

```text
READ
- the route table (memory), as before
- the folder of each folder route, and the files under it (new)

WRITE
- route table and routes.json, as before (a folder route is saved like any route)
- request log (memory), as before

EXTERNAL READ / WRITE
- none
```

Hidden dependency created by this ADR: **the daemon's file access rights**.
What a folder route can serve depends on what macOS lets the `localrouterd`
LaunchAgent read. See U1.

## 7. Interfaces and events

```text
~ SOCKET  register_route   target may be file:///<absolute folder>
~ SOCKET  list_routes      upstream_up means "folder exists" for a folder route
~ SOCKET  API version      1.1 -> 1.2
~ MCP     register_route   + argument folder (absolute path); still six tools
~ CLI     localrouter add  + --folder <dir>; conflicts with port, --target, --tcp, --listen
~ HTTP    folder routes    GET and HEAD only; answers in 02-serving-files.md
  EVENTS  none
```

## 8. External side effects

```text
External side effects: none
```

## 9. Invariants

```text
I1. A folder target MUST be file:// plus an absolute path with no '..' part,
    and MUST NOT be the root folder.
    enforced by: normalize_folder_target, called from RouteTable::validate
    checked by:  T1

I2. A folder route MUST NOT answer with a file whose real path (after
    symlinks) is outside the folder.
    enforced by: folder::inside (canonicalize, then strip_prefix of the real root)
    checked by:  T2, T3

I3. A folder route MUST NOT answer with, or list, a path that has a part
    starting with '.' below the folder, before or after symlinks and
    percent-decoding.
    enforced by: folder::hidden on the decoded request parts and on the real path
    checked by:  T2, T3

I4. A folder route MUST only read. It answers GET and HEAD; nothing in
    folder.rs writes to disk.
    enforced by: the method check in folder::serve; no write call in the module
    checked by:  T3 (405); code review for "no write" (no automated check)

I5. register_route MUST refuse a folder that does not exist or is not a
    folder. Loading routes.json at start MUST NOT refuse it.
    enforced by: the metadata check in Daemon::register_route only
    checked by:  T4

I6. Saving a folder route alone MUST NOT raise the routes.json version.
    enforced by: save_routes raises the version only for path or strip_path
    checked by:  T4

I7. A folder route whose folder is gone MUST answer its own 502 and MUST NOT
    fall through to another route (as ADR 03, I25, for servers).
    enforced by: the lookup picks the route before the folder is read
    checked by:  T3

I8. The daemon MUST NOT resolve a relative folder against its own working
    folder. The CLI MUST send an absolute path.
    enforced by: I1 (daemon); canonicalize in the CLI's folder_target
    checked by:  T1, T4, T5, T6, E1

I9. Every agent text (help page, note, MCP instructions) MUST describe folder
    routes and say that MCP needs an absolute path.
    enforced by: the texts
    checked by:  T7

I10. The MCP server MUST still list exactly six tools.
    enforced by: the tool router
    checked by:  T6
```

## 10. Data impact

```text
New persistent data
  none; a persistent folder route is one more entry in routes.json

Migration     none
Backfill      none
Derived rebuild none
Destructive   none

Compatibility with existing readers
  a daemon from before ADR 05 skips a saved folder route at start (BadTarget),
  loads the others, and names the skipped route in status. See the blast
  radius for what its next write does.
```

## 11. Blast radius

```text
Older daemon after a downgrade
    ↓ because of
  it cannot parse a file:// target
    ↓ creates
  the folder route is skipped at start, with "skipped report: target ... is
  not valid" in status (routes_file_problem)
    failure mode: SILENT DATA LOSS LATER. The skipped route is not in the
    route table, and save_routes writes only the table. The next change to any
    persistent route (daemon.rs:471, :509 before ADR 05) rewrites routes.json
    without the folder route. Upgrading again does not bring it back.

MCP server started before the update (an agent session left open)
    ↓ because of
  deny_unknown_fields on RegisterArgs
    ↓ creates
  register_route with folder fails with "unknown field folder"
    failure mode: visible error; the agent restarts the session or uses the CLI

CLI from before the update
    failure mode: visible; clap refuses --folder. --target file:///... works
    against a new daemon.

Menu bar app 1.1 with daemon 1.2
    failure mode: none; same major version. The row shows "→ file:///..." and
    the status dot follows upstream_up.

404 page, 502 page and router.localhost
    ↓ because of
  they print each route's target
    ↓ creates
  the local folder path (with the user name) is shown to whoever reaches the
  page: loopback only, or the LAN when allow_lan is on. See U2.

allow_lan
    ↓ because of
  folder routes are served on the same listeners as other HTTP routes
    ↓ creates
  with allow_lan on, every folder route is readable from the LAN
    failure mode: silent; nothing on the route says it is reachable from the LAN

Confirmed unaffected
  - forwarding to servers: the body type changed, the 22 proxy tests pass
  - TLS: a folder route's name gets a certificate through RouteTable::serves
  - TCP routes, listen ports, find_free_port
  - owner_pid removal, session and persistent kinds
  - routes.json version rule and the Swift route model
```

## 12. Unresolved effects

```text
? U1. macOS privacy for the LaunchAgent

status:  REQUIRES_TEST (manual M3)
effect:  a folder route in Desktop, Documents, Downloads or iCloud Drive may
         get EPERM (403 page), may make macOS show a prompt naming the app,
         or may work
reason:  only the daemon run from a terminal was tested; a LaunchAgent inside
         a signed bundle is treated by macOS privacy rules differently
blocks:  nothing in the code; the wording of the 403 page and of help.md
outcome: a note in this ADR after M3; a new decision only if macOS blocks
         common folders with no way to allow them

? U2. Folder paths on pages that the LAN can see

status:  REQUIRES_DECISION
effect:  with allow_lan on, the 404 page and router.localhost show
         /Users/<name>/... to LAN clients
reason:  pages already show server targets (ports only); a folder target
         carries the user name and the folder layout
blocks:  nothing
outcome: hide file:// targets from non-loopback peers, or accept it
```

## 13. Risks

```text
- A symlink swapped between the path check and ServeFile's open can lead out
  of the folder (check, then use). Only a local process with the user's rights
  can do it, and such a process can read the file directly.
- A file list of a folder with very many entries is built in memory in one
  page; there is no limit.
- The content type comes from the file extension (mime_guess). A file with an
  unknown extension is sent as application/octet-stream, and a browser may
  download it instead of showing it.
- A folder route to a large folder such as the home folder serves every file
  in it that has no hidden part: Documents, Downloads, keys outside dot
  folders. Nothing refuses a broad folder except "/".
- Names starting with '.' are never served, so a site that needs
  /.well-known/... cannot be served from a folder route.
```

## 14. Rollback

```text
Code rollback
  revert the commit. Enough on its own only when no folder route is saved:
  a saved folder route is skipped by the older daemon, and the next write to
  routes.json drops it for good (see the blast radius).

Data rollback
  before downgrading: localrouter rm <host> for each folder route, or keep a
  copy of routes.json. Nothing else to undo.

Schema rollback        nothing: routes.json shape and version rule unchanged
Infrastructure         nothing
External side effects  none
```

## Plan vs Actual

Compared with the four points of the design stated before the build.

```text
                                      Planned   Actual
Target form file://, no new field         +1       +1   ✓
New module serving files                  +1       +1   ✓
tower-http ServeFile                      +1       +1   ✓
Own path checks (.., dotfiles, links)     +1       +1   ✓
index.html, else file list                +1       +1   ✓
CLI --folder, MCP folder                  +2       +2   ✓
Socket API version change                  0       +1   ⚠
Changed up check in list_routes            0       +1   ⚠
Proxy body type change                     0       +1   ⚠
Extra response headers                     0       +2   ⚠
New MCP tools                              0        0   ✓
New writes                                 0        0   ✓
```

## Architectural drift

```text
+ API version 1.1 -> 1.2
  status: ACCEPTED
  reason: clients can tell whether a daemon knows folder routes; the major
          stays, so no client refuses the other.

+ upstream_up means "folder exists" for a folder route
  status: ACCEPTED
  reason: a TCP connect has no meaning for a folder; without it every folder
          route would show as down.

+ Proxy body type UnsyncBoxBody<Bytes, Box<dyn Error + Send + Sync>>
  status: ACCEPTED
  reason: ServeFile's body is not Sync and fails with io::Error. Internal;
          hyper needs only Send.

+ Cache-Control: no-cache on files and lists; charset=utf-8 on text types
  status: ACCEPTED
  reason: a reload after an edit must show the new file; UTF-8 text must not
          be read as Latin-1.

+ The file list leaves out links out of the folder and links to hidden names
  status: ACCEPTED
  reason: the list must not show a link that answers 404.

+ A '.' inside a folder target is dropped, not refused
  status: ACCEPTED
  reason: Rust's Path::components drops it; the result names the same folder.

~ MCP: target and port together still work (target wins); only folder with
  either is refused
  status: ACCEPTED
  reason: keeps the old behaviour for existing agent calls.

+ 403 page with a hint about macOS privacy
  status: ACCEPTED, not verified (U1)
```
