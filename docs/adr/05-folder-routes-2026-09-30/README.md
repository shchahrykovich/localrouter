# ADR 05. Folder routes: serve a folder with no dev server

**Status:** Built on 2026-09-30 with small drift (see the manifest's
[Plan vs Actual](06-semantic-change-manifest.md#plan-vs-actual)). Automated
tests pass (200 Rust, the Swift suite with 0 failures). Manual test M1 is done;
M2 to M5 are open, and the change is not released yet.

## Summary

**In one sentence.** A route's target may be `file://` plus an absolute
folder, and then the daemon serves that folder's files itself
(`libs/core/src/folder.rs`), so an agent can show the user an HTML page, a
build or a report at a `.localhost` name without starting a server.

**In three sentences.**

To show a file in a browser, an agent had to start a throwaway server on a
free port and register it.

Now `localrouter add report --folder ./coverage`, or MCP `register_route` with
`folder`, makes a folder route: an ordinary HTTP route whose target is
`file:///…/coverage`.

The daemon resolves each request path itself (no dotfiles, no `..`, no
symlinks out of the folder) and hands one checked file to tower-http's
`ServeFile`, which adds content types, `Range` and `304`.

**In seven sentences.**

Coding agents often make something to look at (a page, `dist/`, a coverage
report), and before this ADR LocalRouter could only name a running server.

A folder route is an HTTP route whose `target` is `file://` plus an absolute
path; there is no new field, so `routes.json`, the Swift types and the six MCP
tools keep their shape, and the socket API only moves from 1.1 to 1.2.

The proxy branches after the route lookup: a folder route goes to
`folder::serve`, which answers GET and HEAD with the file, the folder's
`index.html`, a file list, a `308` to add the slash, or a `404`, `403`, `405`
or `502` page.

Safety lives in one short function: a request part that starts with `.` after
percent-decoding is a 404, and so is any real path (after symlinks) outside the
folder, so `.git`, `.env` and `../../.ssh` are never served.

The CLI makes a relative `--folder` absolute, because the daemon has no working
folder; MCP must send an absolute path; the daemon refuses a folder that does
not exist at registration but keeps a saved one whose disk is gone.

The trade-off is a small loss on downgrade: an older daemon skips a saved
folder route, and its next write to `routes.json` drops it.

Tests cover the rules at every layer (unit, proxy, real daemon, CLI, MCP, agent
texts, contract) with no new test dependency; a browser, the LaunchAgent and
macOS privacy rules are left to manual tests.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 1 | A folder is a new kind of target | feature | [01-folder-target.md](01-folder-target.md) |
| 2 | The daemon serves the folder itself | feature | [02-serving-files.md](02-serving-files.md) |
| 3 | Clients, the daemon check and the agent texts | feature | [03-clients-and-texts.md](03-clients-and-texts.md) |
| 4 | Components: where the change lives | overview | [04-components.md](04-components.md) |
| 5 | Data flow | data flow | [05-data-flow.md](05-data-flow.md) |
| 6 | Semantic Change Manifest | manifest | [06-semantic-change-manifest.md](06-semantic-change-manifest.md) |
| 7 | Test plan | test plan | [07-test-plan.md](07-test-plan.md) |

There is no working-backwards file and no tasks file: this ADR records work
that is already built.

## The through-line

**A folder route is a route, not a second server.** Because it is an ordinary
HTTP route, names, paths, TLS, owner processes and the request log work for it
with no change. Because the daemon reads the disk as the user, every request
path is checked by LocalRouter's own code before any file is opened. And
because the daemon has no working folder, the client that knows the user's
folder must make the path absolute.

## Why read the manifest

[06-semantic-change-manifest.md](06-semantic-change-manifest.md) holds three
things no change file argues:

1. **A silent loss on downgrade.** An older daemon skips a saved folder route
   at start. Its next write to `routes.json` writes only the routes it loaded,
   so the folder route is gone for good. Remove folder routes, or keep a copy
   of `routes.json`, before a downgrade.
2. **Two open points.** Whether macOS privacy rules let the LaunchAgent read
   Desktop, Documents and Downloads (U1, manual test M3), and whether pages the
   LAN can see should show local folder paths when `allow_lan` is on (U2).
3. **What is exposed.** With `allow_lan` on, every folder route is readable
   from the LAN, and nothing refuses a broad folder such as the home folder
   (only `/` is refused).

## Why read the test plan

[07-test-plan.md](07-test-plan.md) lists 9 automated test groups, one
end-to-end test and five manual tests. It notes one thing the automated tests
cannot prove: that `folder.rs` never writes (I4) is checked by code review
only. The browser, the installed LaunchAgent and macOS privacy are manual
tests M2 and M3, still open.

## Notable artifacts

- Target form `file:///<absolute folder>`; `RouteError::BadFolder` and
  `RouteError::FolderOnlyForHttp`.
- New module `libs/core/src/folder.rs`; new dependency tower-http (feature
  `fs`, only `ServeFile`).
- CLI `add --folder`; MCP `register_route` argument `folder`.
- Socket API 1.2; `api/examples/register_route_folder.request.json` and
  `.reply.json`.
- No new program, process, port, socket method, MCP tool or data-folder file.

## Not in this ADR

- Live reload, or watching the folder for changes.
- A single-page-app fallback (a missing path answering `index.html`).
- Serving one file instead of a folder.
- Writing to the folder (PUT, uploads) or running scripts.
- `/.well-known/` and other names that start with a dot.

## Diagrams

All diagrams are D2 sources in [diagrams/](diagrams/) with rendered `.svg`
files. `diagrams/_palette.d2` is the shared colour palette, copied from ADR 03;
it has no render of its own. Render one with:
`d2 --theme 0 --pad 20 diagrams/02-resolve.d2 diagrams/02-resolve.svg`
