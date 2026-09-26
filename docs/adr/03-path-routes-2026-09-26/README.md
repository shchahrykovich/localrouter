# ADR 03. Path routes: several dev servers on one name, chosen by path

**Status:** Proposed, 2026-09-26. Nothing is built. Changes
[ADR 01, change 6](../01-project-setup-2026-09-26/06-route-model.md): the host
is no longer unique across routes; the route key (host plus path) is.

## Summary

**In one sentence.** An HTTP route gets an optional path prefix, so
`https://shop.localhost/blog` can go to one dev server and the rest of
`shop.localhost` to another, the way a production reverse proxy splits one
domain between several apps.

**In three sentences.**

Many sites are several apps behind one domain, split by path, and LocalRouter
today can give each app only its own name.

The daemon's route table is keyed by host plus path, and the proxy picks the
nearest host key first and then the longest matching path, keeping the path or
stripping it per route.

Agents, the CLI and the app pass the new `path` and `strip_path` fields through
the same socket API, now version 1.1, and the texts Claude Code reads explain
path routes and the base path each app needs.

**In seven sentences.**

A site that is built as a main app plus a blog app and an admin app, joined by
a proxy that reads the path, cannot be reproduced locally today: one host key
has at most one route (`RouteTable`, `libs/core/src/routes.rs:229`).

A route gets two optional fields, `path` and `strip_path`, and its identity
becomes the route key `(host, path)`; a route without a path is the host's
default route, the only kind that exists today.

On each request the proxy walks the host key from the full name to its parents,
as today, and on the first host key with a matching route it takes the longest
path, where `/blog` matches `/blog` and `/blog/...` but never `/blogger`; a
failing target gives its own 502 and never falls through to another route.

The path reaches the dev server unchanged unless `strip_path` is set, which
removes the prefix and adds `X-Forwarded-Prefix`; nothing else is rewritten, so
each app must be built to live under its path (Next.js `basePath`, Vite
`base`), and LocalRouter cannot fix two apps that both ask for `/_next/...`.

The socket API becomes 1.1 with additive fields only, the MCP server keeps its
six tools and gains `path` and `strip_path` arguments, the CLI gains `--path`
and `--strip-path`, and the app keys its rows and its remove call by host plus
path.

`routes.json` stays version 1 until a path route is saved, so a downgrade only
loses anything when path routes were in use.

Tests extend the existing Rust and Swift suites (no new dependency), add a
second end-to-end journey, and leave the real frameworks to manual checks with
Next.js, Vite and a Claude Code session.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 1 | The route key becomes host plus path | feature | [01-route-key.md](01-route-key.md) |
| 2 | Lookup: nearest host key first, then the longest path | feature | [02-path-lookup.md](02-path-lookup.md) |
| 3 | Forwarding: the path is kept unless `strip_path` is set | feature | [03-forwarding.md](03-forwarding.md) |
| 4 | Clients and agent texts: socket API, MCP, CLI, app, Claude Code docs | feature | [04-clients-and-agent-texts.md](04-clients-and-agent-texts.md) |
| 5 | Components: where the change lives | proposed | [05-components.md](05-components.md) |
| 6 | Data flow | data flow | [06-data-flow.md](06-data-flow.md) |
| 7 | Semantic Change Manifest | manifest | [07-semantic-change-manifest.md](07-semantic-change-manifest.md) |
| 8 | Test plan | test plan | [08-test-plan.md](08-test-plan.md) |
| 9 | Tasks | build plan | [09-tasks.md](09-tasks.md) |

## What changes for MCP and the Claude Code texts, in short

Full detail in [04](04-clients-and-agent-texts.md).

| Place | Change |
|---|---|
| MCP `register_route` | new arguments `path`, `strip_path` |
| MCP `unregister_route` | new argument `path`; without it only the route without a path is removed |
| MCP tool count | still six |
| MCP arguments | unknown arguments are refused instead of dropped |
| MCP `INSTRUCTIONS` | one paragraph on path routes and base paths |
| `scripts/LocalRouter.md` (`~/.claude/LocalRouter.md`) | four lines: what a path route is, when to use it, the command, "host and path" replace rule |
| `libs/core/src/help.md` (`router.localhost`) | seven places, including a new "Step 4b: several apps on one name" |
| `~/.claude/CLAUDE.md`, the installer | no change: the note is a link into the bundle |

## The through-line

Everything in this ADR follows from one choice: **a path route is a full route,
not an option of a host.** Because each path route has its own owner process,
note and lifetime, a worktree can own `feat-x.shop/blog` alone. Because the key
is exact, removal must be exact, and every client that removed "by host" must
now remove by key. And because LocalRouter forwards and does not rewrite, the
apps must know their own paths, which is why the texts agents read matter as
much as the code.

## Why read the manifest

[07-semantic-change-manifest.md](07-semantic-change-manifest.md) holds three
things no change file argues:

1. **Two silent failures.** If the app's remove button keeps sending only the
   host, clicking the `/blog` row removes the main app's route instead (B1, with
   both scenarios). An MCP server started before the update drops the `path`
   argument, so an agent that asks for `shop/blog` replaces `shop` (B2).
2. **What a downgrade does.** An older daemon moves a version 2 `routes.json`
   aside and starts with no persistent routes. The version is written as 1
   whenever possible to keep this rare (I29).
3. **Two open points.** A daemon warning for framework paths that reach the
   default route (U6), and a way to remove every route of a host (U7). Neither
   blocks the build.

Writing the manifest changed the design: `deny_unknown_fields` on the MCP
arguments, the move of the Swift remove call into the tested kit, and the
version 1 or 2 rule for `routes.json` all came from it.

## Why read the test plan

[08-test-plan.md](08-test-plan.md) found one gap it cannot close with the tools
the repository has: every automated test uses echo servers, so the framework
behaviour that makes path routes work (base paths, hot reload socket paths) is
covered only by manual tests M1 and M2. A browser test tool would close it and
is listed as "not without approval".

## Notable artifacts this ADR plans

- Route fields `path` and `strip_path`; the route key `(host, path)`.
- Request header `X-Forwarded-Prefix` for `strip_path` routes.
- Log entry field `route`.
- Socket API 1.1; `routes.json` version 2 (version 1 still written when
  possible).
- Three new files in `api/examples/`.
- No new program, process, port, data-folder file, socket method or MCP tool.

## Not in this ADR

- Rewriting a path to another path (only removing the prefix).
- Redirecting one name to another name.
- Matching by pattern (`*`, regular expressions), by header or by method.
- Path routes for TCP routes (plain TCP has no path).
- Rewriting `Location` or `Set-Cookie` in responses.

## Diagrams

All diagrams are D2 sources in [diagrams/](diagrams/) with rendered `.svg`
files. `diagrams/_palette.d2` is the shared colour palette, copied from ADR 01;
it has no render of its own. Render one with:
`d2 --theme 0 --pad 20 diagrams/02-path-lookup.d2 diagrams/02-path-lookup.svg`
