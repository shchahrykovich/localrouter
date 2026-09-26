# 7. Semantic Change Manifest

What will exist in the system after this ADR is built, what runs that does not
run today, what must stay true, and what a rollback cannot undo. Invariant and
open-point numbers continue those of ADR 01 (`I1` to `I19`, `U1` to `U5`).

## 1. Manifest status

```text
Manifest status: IMPLEMENTED_WITH_DRIFT   (planned text below is unchanged;
                                           see Actual Change Manifest at the end)
```

## 2. Semantic change summary

```text
Artifacts
+ 2 route fields: path, strip_path
+ 1 route identity: the route key (host, path)
+ 1 request header sent to dev servers: X-Forwarded-Prefix (strip_path only)
+ 1 log entry field: route
+ 3 API example files
~ 1 socket API: 1.0 -> 1.1, 2 request types and 2 reply types grow
~ 2 MCP tools: register_route, unregister_route (arguments)
~ 4 CLI commands: add, rm, list, logs
+ 1 CLI command: which (read-only)
~ 1 persistent file format: routes.json version 2 (version 1 still written when possible)
~ 3 agent texts: Claude Code note, MCP instructions, help page

Persistent data
~ routes.json: a saved route may carry path and strip_path

Runtime effects
~ route lookup reads the request path
+ the CLI runs the same lookup on a copy of the table (which)
~ certificate decision reads "does the name have any route"
~ request path rewritten for strip_path routes
+ X-Forwarded-Prefix set for strip_path routes

New programs, processes, ports, data-folder files, MCP tools, socket methods: 0
External effects: 0
Destructive operations: 0 new
Unresolved effects: 2 (U6, U7)
```

## 3. Source of truth

```text
Source of truth: unchanged.
  The daemon's in-memory route table stays CANONICAL while it runs.
  routes.json stays the CANONICAL store of persistent routes; the daemon is
  still its only writer (I1).
  The route key (host, path) replaces the host as the identity inside both.
```

## 4. Artifacts

```text
Domain objects
~ Route           + path: Option<String>, + strip_path: bool
+ RouteKey        (host, path), "" = default route
~ RouteTable      keyed by RouteKey; + serves(name, fallback);
                  + explain(name, path, fallback), which lookup is built on
~ RouteError      + BadPath, + PathOnlyForHttp, + StripPathNeedsPath,
                  + HostHasTcpRoute, + HostHasPathRoutes
~ LogEntry::Http  + route: Option<String>

Interfaces
~ socket API 1.1  Route, HostParams, RouteView, LogEntry grow; 11 methods, unchanged
~ MCP             register_route + path, strip_path; unregister_route + path;
                  unknown arguments refused; still 6 tools
~ CLI             add --path --strip-path; rm --path; list and logs columns;
                  + which <url>

Persistent artifacts
~ routes.json     version 2 when a saved route has path or strip_path

Texts
~ scripts/LocalRouter.md (bundled as Contents/Resources/LocalRouter.md)
~ apps/cli/src/mcp.rs INSTRUCTIONS and tool descriptions
~ libs/core/src/help.md and the {{ROUTES}} rendering in help.rs
~ docs/dictionary.md, CLAUDE.md

Test fixtures
+ api/examples/register_route_path.request.json
+ api/examples/register_route_path.reply.json
+ api/examples/unregister_route_path.request.json
~ api/examples/list_routes.reply.json, get_logs.reply.json, log.event.json

Database entities: 0      Events: 0      Workers: 0
Infrastructure:    0      Processes: 0   Ports: 0
```

## 5. Runtime effects

```text
READ   route lookup by host key and request path
type:                   in-memory read
target:                 route table
trigger:                every HTTP request and every WebSocket upgrade
cardinality:            one lookup per request
write_idempotent:       n/a (read)
producer_deterministic: yes (same table, name and path give the same route)
destructive:            no

READ   certificate decision
type:                   in-memory read
target:                 route table, RouteTable::serves
trigger:                TLS handshake for a name without a cached leaf
cardinality:            one per new name per 90 days, as today
producer_deterministic: yes
destructive:            no

CALL   forward with a stripped path
type:                   HTTP request to a loopback dev server
target:                 route target
trigger:                request matched by a route with strip_path
effect:                 request path loses the route path; query kept;
                        X-Forwarded-Prefix set, client value dropped
write_idempotent:       as idempotent as the request itself (unchanged)
producer_deterministic: yes (a pure string operation)
destructive:            no

READ   explain one URL (localrouter which)
type:                   socket calls list_routes and get_config, then an
                        in-memory lookup in the CLI process
trigger:                a person or an agent runs localrouter which <url>
cardinality:            one of each call per command
write_idempotent:       n/a (read)
producer_deterministic: yes for a given table; the table can change between
                        the command and a later request
side effect:            list_routes makes one 200 ms connection attempt per
                        route target (upstream_up), as it does today
destructive:            no

WRITE  routes.json
type:                   file REPLACE (temp file, fsync, rename)
target:                 <data folder>/routes.json
trigger:                register_route or unregister_route of a persistent route
cardinality:            one file write per call, as today
write_idempotent:       yes (the same table writes the same bytes)
producer_deterministic: yes (routes sorted by key)
change:                 version 2 when any saved route has path or strip_path
retention:              until the next write
destructive:            replaces the previous file, as today
reversible:             by the next write

WRITE  request log entry
type:                   in-memory ring buffer append
trigger:                every HTTP request
change:                 + route (the key that answered; absent for 404 and
                        the help page)
destructive:            pushes out the oldest entry, as today
```

## 6. Reads and writes

```text
READ
- route table (lookup by key range, serves)
- request path (new input to the lookup)
- routes.json version 1 or 2 at start

WRITE
- route table (insert, replace, remove by key)
- routes.json (version 1 or 2)
- request log (with route)
- the CLI's own copy of the table, for which (memory only, discarded)

EXTERNAL READ / WRITE
- none

HIDDEN DEPENDENCIES THAT EXIST ONLY AFTER THIS CHANGE
- A path app works only if it is built with a base path (Next.js basePath,
  Vite base). LocalRouter cannot check this. See U6.
- Every client that removes a route must send the path. A client that sends
  only the host removes the default route instead of the one it meant. See B1.
```

## 7. Interfaces and events

```text
~ SOCKET  register_route    params: + path, + strip_path
~ SOCKET  unregister_route  params: + path; without it, only the default route
~ SOCKET  list_routes       reply:  routes carry path, strip_path; urls with path
~ SOCKET  get_logs, subscribe_logs  entries: + route
~ SOCKET  hello             api_version "1.1"
~ MCP     register_route    + path, + strip_path
~ MCP     unregister_route  + path
~ MCP     all tools         unknown arguments refused
~ CLI     localrouter add   + --path, + --strip-path
~ CLI     localrouter rm    + --path
~ CLI     localrouter list  path shown
~ CLI     localrouter logs  route column
+ CLI     localrouter which <url>  read-only, no new socket method
~ HTTP    to dev servers    + X-Forwarded-Prefix (strip_path routes only)
~ HTTP    404 page          lists path routes with their paths
  Socket methods, MCP tool names, events: unchanged
```

## 8. External side effects

```text
External side effects: none. Targets stay loopback only (I9).
```

## 9. Invariants

```text
I20. The route key is (host, path). At most one route exists per key. A route
     without a path and a route with path "/" have the same key.
     enforced by: RouteTable keyed by RouteKey; T1, T6.

I21. A stored path starts with "/", has no trailing "/", no empty, "." or ".."
     segment, only RFC 3986 unreserved characters, and at most 200 characters.
     enforced by: validation in routes.rs; T1.

I22. path and strip_path exist only on HTTP routes; strip_path needs a path. A
     host key with a TCP route has no other route; a host key with a path route
     has no TCP route.
     enforced by: validation in routes.rs; T2.

I23. A route path P matches a request path R only when R == P or R starts
     with P + "/". Matching is case-sensitive and uses the raw path without
     the query.
     enforced by: T3 (the match table in 02-path-lookup.md).

I24. Lookup picks the nearest host key that has a matching route, then the
     longest matching path on that key. A table without path routes returns,
     for every name and path, the same route as the lookup before this ADR.
     enforced by: T3, including a regression test that runs the ADR 01 lookup
     cases through the new function.

I25. When the chosen route's target fails, the reply is that route's 502 page.
     The request is never sent to another route.
     enforced by: T4.

I26. Without strip_path, the path and query reach the dev server byte for
     byte. With strip_path, exactly the route path is removed (empty becomes
     "/"), the query is kept, and X-Forwarded-Prefix equals the route path,
     replacing any client value. The Host header is unchanged (I12).
     enforced by: T4.

I27. A leaf certificate is issued for a name when any HTTP route serves that
     name, with or without a path. The path plays no part in TLS. (I5 still
     holds: only .localhost names, only names with a route.)
     enforced by: T4 (host with only a path route gets a certificate; a name
     with no route still gets none).

I28. unregister_route without path removes only the default route of the
     host. No call removes a route whose key it did not name, except the owner
     watch, which removes exactly the keys that process owns.
     enforced by: T6.

I29. routes.json is written as version 1 when no saved route has path or
     strip_path, and as version 2 otherwise. The daemon reads both.
     enforced by: T7.

I30. api/examples contains a route with path and strip_path in a
     register_route request and reply and in the list_routes reply, an
     unregister_route request with path, and a log entry with route. So I11
     (both sides decode every example) covers the new fields.
     enforced by: T8 (a Rust test that fails when one of these is missing).

I31. The MCP server still exposes exactly six tools (I13). register_route
     lists path and strip_path, unregister_route lists path, and every tool
     refuses an unknown argument.
     enforced by: T10.

I32. The help page, the Claude Code note and the MCP instructions all
     mention path routes and all three tell the agent to ask the user before
     it changes basePath, base or assetPrefix, because these change the
     production build. The help page names both patterns (base path; page
     prefixes plus a stripped file prefix) and localrouter which.
     enforced by: T11 (presence of the words; wording by M3).

I33. localrouter which and the proxy choose the same route for the same
     table, name and path: lookup is explain without the record, in one
     function in libs/core.
     enforced by: T3 (explain and lookup agree on every lookup case), E1b
     (which names the same route as the log entry of a real request).
```

## 10. Data impact

```text
Migration          none required. Every routes.json version 1 file is a valid
                   input; its routes become default routes.
Backfill           none
Derived rebuild    route table and leaf certificates, on every start, as today
Destructive        none new
Compatibility      a daemon from before this ADR refuses routes.json version 2:
                   it moves the file aside to routes.json.bad-<time> and starts
                   with no persistent routes (store.rs, load). This happens
                   only after a downgrade, and only when a path route was
                   saved. See Rollback.
Expected growth    about 40 bytes per saved path route
```

## 11. Blast radius

```text
B1. LocalRouter.app, remove button
    dependency:   AppModel.remove sends unregister_route with host only
                  (apps/menubar/Sources/LocalRouter/AppModel.swift:140)
    impact:       if the Swift change is left out, the new daemon reads the
                  call as "remove the default route"
    failure mode: SILENT. The user clicks the row shop.localhost/blog; the
                  daemon removes shop, the main app. The /blog row stays. No
                  error is shown.
```

Scenario A - works (Swift change built):

1. Routes: `shop` → 5173, `shop/blog` → 3001.
2. The user clicks remove on the row `shop.localhost/blog`.
3. The app sends `{host: "shop", path: "/blog"}`.
4. The daemon removes `shop/blog`. `shop` still serves the main app.

Scenario B - broken (Swift change forgotten):

1. Same routes.
2. The user clicks remove on the row `shop.localhost/blog`.
3. The app sends `{host: "shop"}`.
4. The daemon removes the default route `shop`.
5. `https://shop.localhost/` now answers 404, and the blog route the user wanted
   gone still runs. The user sees the opposite of what they clicked.

The contract test does not catch this: the request is valid in both cases.
Guard: task 5 moves the call into `DaemonClient.unregister(_ route:)` in
`LocalRouterKit`, which has tests, and T8 checks that it sends the path.

```text
B2. MCP servers started before the update
    dependency:   a Claude Code session keeps running the localrouter mcp
                  process it started; the old code has no path argument and
                  drops unknown arguments without an error
    impact:       an agent that read the new help page passes path to
                  register_route; the old server drops it
    failure mode: SILENT. register_route with host "shop", path "/blog",
                  port 3001 becomes a default route shop -> 3001, which
                  REPLACES the main app's route. The reply says
                  replaced: true, old_target http://127.0.0.1:5173; an agent
                  that reads it can notice, one that does not cannot.
    mitigation:   (1) the old server's tool schema has no path, so a careful
                  agent does not send it; (2) help.md Step 4b says "if
                  register_route has no path argument, restart the session or
                  use the command line"; (3) deny_unknown_fields from this ADR
                  on, so the next change of this kind fails loudly.
                  The gap stays for servers older than this ADR.

B3. LocalRouter.app, route list
    dependency:   RouteView.id is the host (Api.swift:107)
    impact:       two rows with the same id in one SwiftUI list
    failure mode: rows drawn from the wrong data or not updated after a
                  change. Guard: id becomes the key, T8.

B4. Scripts that parse `localrouter list --json`
    dependency:   assume one entry per host
    impact:       see two entries with host "shop"
    failure mode: a script that builds a map by host keeps one of them.
                  Only after the user adds a path route; no such script is in
                  this repository.

B5. Downgrade to a build from before this ADR
    dependency:   store.rs refuses an unknown routes.json version
    failure mode: persistent routes are gone until the file is moved back;
                  status shows routes_file_problem. Only when a path route was
                  saved (I29 keeps version 1 otherwise).

B6. Dev servers behind a path route
    dependency:   framework asset and hot reload paths
    failure mode: SILENT in LocalRouter. Without a base path, a page renders
                  without scripts or styles; every request gets an answer, from
                  the default route. Visible through the route field of the
                  request log and through localrouter which. See U6.
```

**Confirmed unaffected.** TCP routes and their listeners, the local CA and
`ca.key`, trust in the keychain, the peer check, `config.json`, the updater, the
Claude Code installer and `~/.claude/CLAUDE.md`, and every socket method name.

## 12. Unresolved effects

```text
? U6. Warning for framework paths that reach the default route

status:  UNRESOLVED
effect:  the daemon notices a pattern like "the default route answered 404 for
         /_next/... or /@vite/... while this host has path routes" and says so
         in the log entry, the status, or the app
reason:  the pattern is a guess from framework names; a list of framework
         paths goes stale; a false warning on a site that serves /_next/ from
         its main app on purpose is noise. Undecided: where the warning shows,
         and whether it is worth a list of framework paths in the daemon.
blocks:  nothing. The texts (I32), the route field in the log and
         localrouter which (I33) cover the case without it.
outcome: a decision after M2 shows how hard the failure is to diagnose with
         the log alone

? U7. Remove every route of a host at once

status:  UNRESOLVED
effect:  unregister_route with a flag, or localrouter rm --all-paths
reason:  I28 makes removal exact on purpose. A "remove all" is convenient but
         reopens the path to removing routes the caller did not name; an agent
         that owns only shop/blog must not remove shop.
blocks:  nothing
outcome: decide when a user asks for it
```

## 13. Risks

- **Mixed `https_only` on one host.** `https_only` is per route, so
  `http://shop.localhost/blog` can redirect while `http://shop.localhost/`
  does not. The help page recommends the same value on every route of a host;
  nothing enforces it.
- **`strip_path` apps redirect outside their prefix.** No `Location` or cookie
  rewriting (decided in [03](03-forwarding.md)). An app that redirects to
  `/login` leaves its prefix. The risk is that a user reads this as a
  LocalRouter bug.
- **Nearest host key wins.** A branch host with a default route hides the
  project's path routes for that name. This is intended (I24), but a user who
  expected `feat-x.shop.localhost/blog` to reach the main blog app sees the
  branch's main app instead. The 404 and log `route` field make it visible.
- **One app, several routes.** An app with several page prefixes and a file
  prefix (pattern B in [03](03-forwarding.md)) needs one route per prefix. If
  a user removes one of them by hand, the app is half routed: some pages come
  from the default route. Routes registered with the same `owner_pid` go
  together; persistent and session routes do not. `localrouter list` shows the
  routes of one host together, so the gap is visible.
- **An agent changes the production build.** Making a path route work can
  need `basePath`, `base` or `assetPrefix` in the app, and those change the
  production build. The texts tell the agent to ask first (I32), but a text
  is a request to the agent, not a check. M3 tests it with a real session.
- **Replace across agents, now per key.** ADR 01 names the silent replace when
  two agents register the same host. With paths, two agents that register
  `shop/blog` for two worktrees replace each other the same way. The help page
  keeps telling agents to use a branch host for a worktree.

## 14. Rollback

```text
Code rollback
  Reinstall the previous release. Sufficient on its own when no path route
  was ever saved (routes.json stayed version 1, I29).

Data rollback
  When a path route was saved, the old daemon moves routes.json aside to
  routes.json.bad-<time>. To keep the default routes: before the downgrade,
  remove the persistent path routes (localrouter rm <host> --path <p>), so the
  file is written as version 1. After the fact: edit the .bad file, delete
  the path routes and set "version": 1, then move it back while the daemon
  is stopped.

Schema, infrastructure
  Nothing to reverse.

What a rollback cannot undo
  Agent text read by running sessions. A Claude Code session that read the new
  note or help page keeps the idea of path routes until it ends. Against an
  old daemon its register_route calls with path go through an old MCP server
  that drops path (B2). Restart the sessions after a downgrade.
```

---

# Actual Change Manifest

Re-read from the built code on 2026-09-26, not copied from the plan above.

## A1. Summary

```text
Artifacts
+ Route.path, Route.strip_path                     libs/core/src/routes.rs
+ RouteKey (host, path), Display "shop/blog"       libs/core/src/routes.rs
+ RouteTable::routes_of, lookup(name, path, fb),
  explain, serves; one private walk behind lookup
  and explain                                      libs/core/src/routes.rs
+ path_matches, normalize_path (public helpers)    libs/core/src/routes.rs
+ RouteError: HostLooksLikePath, BadPath,
  PathOnlyForHttp, StripPathNeedsPath,
  HostHasTcpRoute, HostHasPathRoutes               libs/core/src/routes.rs
+ LogEntry::Http.route, LogEntry::with_route       libs/core/src/logs.rs
+ X-Forwarded-Prefix on strip_path routes          libs/core/src/proxy.rs
~ API_VERSION "1.1"; HostParams.path               libs/core/src/api.rs
~ routes.json version 1 or 2 on write; 1..=2 read  apps/daemon/src/store.rs
~ daemon: every table access by RouteKey;
  certificate hook calls serves                    apps/daemon/src/daemon.rs
+ CLI: add --path --strip-path, rm --path,
  which <url>, route column in logs, (strip) in list  apps/cli/src/main.rs
~ MCP: path, strip_path, deny_unknown_fields on
  every argument struct, INSTRUCTIONS paragraph    apps/cli/src/mcp.rs
~ Swift: Route.path, stripPath, id = key,
  HostParams(route:), LogEntry.http route,
  DaemonClient.unregister(_ route:)                apps/menubar/Sources/LocalRouterKit/
~ App: remove by key, strip badge, route in logs   apps/menubar/Sources/LocalRouter/
+ api/examples: register_route_path.request/reply,
  unregister_route_path.request; list_routes,
  get_logs, log.event, status, hello changed
~ help.md, help.rs, scripts/LocalRouter.md,
  docs/dictionary.md, CLAUDE.md

New programs, processes, ports, data-folder files, MCP tools, socket methods: 0
Unresolved effects: 2 (U6, U7)
```

## A2. Invariants and what checks them

| Invariant | Checked by |
|---|---|
| I20 | `routes.rs` tests `slash_path_and_no_path_are_one_key`, `path_routes_have_their_own_keys`; `api.rs` `path_routes_are_registered_replaced_and_removed_by_key` |
| I21 | `valid_paths_are_normalized`, `bad_paths_are_refused_with_the_rule` |
| I22 | `path_is_only_for_http_routes`, `tcp_routes_do_not_share_a_host_with_path_routes`, `path_route_rules_are_enforced_by_the_daemon` |
| I23 | `match_rule_table` |
| I24 | `longest_path_wins_on_one_host`, `nearest_host_key_wins_over_a_longer_path`, `branch_path_route_falls_back_for_other_paths`, `without_path_routes_the_lookup_is_the_old_one` |
| I25 | proxy `closed_path_target_gets_its_502_and_never_the_default_route`; E1b step 8 |
| I26 | proxy `path_route_gets_its_path_unchanged...`, `strip_path_removes_the_prefix_and_sets_forwarded_prefix`; E1b step 4 |
| I27 | proxy `name_with_only_a_path_route_gets_a_certificate`; E1b step 5 through the daemon's own hook |
| I28 | `api.rs` path tests, `owner_exit_removes_only_its_path_route`; CLI `path_route_add_list_rm`; E1b step 10 |
| I29 | `store.rs` `version_is_1_without_path_routes_and_2_with_them`, `versions_1_and_2_load_and_3_is_moved_aside` |
| I30 | `api_examples.rs` `examples_cover_path_routes`, plus both round-trip tests |
| I31 | `mcp.rs` `exactly_six_tools_are_listed`, `path_routes_through_mcp_and_unknown_arguments_are_refused` |
| I32 | `agent_texts.rs` (presence); wording by M3, open |
| I33 | `answer()` helper in every `routes.rs` lookup test compares `lookup` and `explain`; CLI `which_*` tests; E1b `check()` at every step |

## A3. Plan vs Actual

| # | Kind | Difference | Status |
|---|---|---|---|
| D1 | `~` | Pattern B for `next dev` must not strip the file prefix: hot reload needs it (M1). The plan, the help page draft and the working-backwards commands used `--strip-path`. | ACCEPTED: help page and CLI test corrected in this change |
| D2 | `~` | Without a base path the failure is `200` with another app's files, not `404` (M2). U6's idea, a warning on 404s from the default route, would not see it. | ACCEPTED for the texts; U6 stays UNRESOLVED with this new fact |
| D3 | `~` | Next.js 16 hot reload path is `/_next/hmr`, not `/_next/webpack-hmr`. | ACCEPTED: no code depends on the name |
| D4 | `+` | The path hint is its own error, `HostLooksLikePath`, instead of a second hint inside `BadLabel`, so the old `BadLabel` test did not change. | ACCEPTED |
| D5 | `+` | `routes_of`, `path_matches`, `normalize_path` are public; the daemon uses `normalize_path` for `unregister_route`. | ACCEPTED |
| D6 | `+` | The `hello` and `status` examples now say `1.1`. | ACCEPTED |
| D7 | `-` | The app's Domains list does not indent path routes under their host; rows come in daemon order (host, then path, default first) and show `shop.localhost/blog`. | ACCEPTED |
| D8 | `-` | Manual tests M3 (Claude Code session), M4 (menu bar app), M5 (downgrade), M6 (post-release smoke) not run; not released. | FIX_REQUIRED before release |
| D9 | `~` | A WebSocket request whose upstream never answers writes no log entry at all, so the log does not show it. Seen in M1 with the stripped socket. | ACCEPTED: the same holds for any request the upstream never answers; not new in this ADR |
