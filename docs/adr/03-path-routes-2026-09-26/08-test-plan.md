# 8. Test plan

Tests to write for this proposed ADR. Test IDs are local to this ADR; where a
test extends an ADR 01 test, the file says so.

## What the repository can run today

| Kind | Tool | Where |
|---|---|---|
| Rust unit tests | `cargo test` | `mod tests` inside `libs/core/src/*.rs`, `apps/daemon/src/*.rs` |
| Rust integration tests | `cargo test` | `libs/core/tests/proxy.rs` (in-process proxy harness with echo, TLS and WebSocket upstreams), `apps/daemon/tests/api.rs` (real daemon, socket calls), `apps/cli/tests/cli.rs`, `apps/cli/tests/mcp.rs` (rmcp client), `apps/cli/tests/e2e.rs` (journey E1) |
| Contract tests | `cargo test`, XCTest | `libs/core/tests/api_examples.rs`, `apps/menubar/Tests/LocalRouterKitTests/ApiContractTests.swift`, both walk `api/examples/` |
| Swift unit tests | XCTest | `LocalRouterKitTests` only. `AppModel` and the views are in the app target and have no tests. |
| Browser tests | none | there is no browser test tool; real frameworks are checked by hand (M1, M2) |

All tests use `LOCALROUTER_HOME` in a short temp dir and ports `0`, as the
project `CLAUDE.md` requires. Nothing here needs a new dependency.

## Overview

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| T1 | unit | path rules and the route key (I20, I21) | `RouteTable::validate` | none | `cargo test` |
| T2 | unit | protocol rules (I22) | `validate` | none | `cargo test` |
| T3 | unit | match rule, lookup order, regression, explain agrees with lookup (I23, I24, I33) | `RouteTable::lookup`, `explain` | none | `cargo test` |
| T4 | integration | forwarding, strip, headers, 502, TLS, WebSocket (I25, I26, I27) | proxy, TLS, hyper | the daemon: a fixed `RouteSource` | `cargo test` |
| T5 | integration | log entry `route` | proxy, request log | the daemon | `cargo test` |
| T6 | integration | register, replace, remove by key, owner watch (I20, I28) | daemon, socket, kqueue | none | `cargo test` |
| T7 | unit | `routes.json` version 1 or 2 (I29) | `store.rs` | none | `cargo test` |
| T8 | contract | Rust and Swift agree on the new fields (I30); Swift id and remove call | both type sets | the daemon (files, not a socket) | `cargo test`, `swift test` |
| T9 | integration | CLI flags and output, `which` | CLI binary, daemon | none | `cargo test` |
| T10 | integration | MCP arguments, unknown arguments, six tools (I31) | MCP server, daemon | the agent: rmcp client | `cargo test` |
| T11 | unit | help page and texts mention path routes, both patterns, the ask-first rule, `which` (I32) | `help.rs`, text files | none | `cargo test` |
| E1b | end-to-end | the path journey through real binaries | daemon, CLI, MCP shim | dev servers: echo servers | `cargo test` |
| M1 | manual | real Next.js and Vite apps under a path (pattern A) and behind a stripped file prefix (pattern B), hot reload | everything | none | by hand |
| M2 | manual | the failure without a base path, and how it looks | everything | none | by hand |
| M3 | manual | a Claude Code session sets up path routes from the texts | agent, texts, MCP | none | by hand |
| M4 | manual | menu bar app rows and remove | app | none | by hand |
| M5 | manual | downgrade with a saved path route | two builds | none | by hand |
| M6 | manual | post-release smoke test | release build | none | by hand |

**Replaced parts and who covers them.**

- T4 and T5 replace the daemon with a fixed `RouteSource`. The real daemon's
  certificate hook (`serves`) and lookup wiring are covered by E1b over HTTPS.
- T8 replaces the socket with files. The real Swift remove call against a real
  daemon is covered only by M4: there is no Swift test that talks to a daemon.
  That is a gap for B1; T8 narrows it by testing the request the Swift client
  builds.
- Every automated test uses echo servers instead of real frameworks. The
  framework behaviour that makes path routes work or fail (base path, hot
  reload socket paths) is covered only by M1 and M2.

## 1. Automated tests (CI)

### Core: route key and rules

`libs/core/src/routes.rs`, `mod tests`.

| Test file | Case |
|---|---|
| `routes.rs` | T1: `/blog`, `/blog/`, `/docs/v2`, `/api_v2`, `/~me` are accepted; the trailing `/` is removed |
| `routes.rs` | T1: `/` and a missing path give the same key; registering both leaves one route |
| `routes.rs` | T1: `blog`, `/a//b`, `/a/../b`, `/a/./b`, `/a%20b`, `/a?b`, `/a#b`, a 201-character path are refused, each with its own message |
| `routes.rs` | T1: `/Blog` and `/blog` are two keys |
| `routes.rs` | T2: TCP route with `path` refused; `strip_path` without `path` refused |
| `routes.rs` | T2: path route on a host with a TCP route refused; TCP route on a host with a path route refused; TCP route replacing a plain default HTTP route still allowed (ADR 01 behaviour) |
| `routes.rs` | T1: `normalize_host("shop/blog")` error mentions both `shop-blog` and host `shop` with path `/blog` |

### Core: lookup

`libs/core/src/routes.rs`, `mod tests`.

| Test file | Case |
|---|---|
| `routes.rs` | T3: the match table in [02](02-path-lookup.md), row by row, including `/blogger`, `/Blog`, `/blog%2Fx` |
| `routes.rs` | T3: `/blog/admin` beats `/blog`, `/blog` beats the default route |
| `routes.rs` | T3: nearest host key wins: `feat-x.shop` default route answers `/blog` although `shop/blog` exists |
| `routes.rs` | T3: Scenario A and B of [02](02-path-lookup.md), fallback on and off |
| `routes.rs` | T3 regression (grows by itself): every lookup case of the existing `lookup_exact_fallback_and_none` test, run through the new `lookup` with path `/` and with a random path, gives the same route |
| `routes.rs` | T3: `serves`: a host with only a path route is served; a name with no route is not; fallback respected |
| `routes.rs` | T3 (grows by itself): for every case in this section, `explain(...).route` equals `lookup(...)` (I33) |
| `routes.rs` | T3: `explain` records each host key tried, the paths it has, the match or its absence, and the fallback step |

### Core: proxy

`libs/core/tests/proxy.rs`, with the existing `harness`, `echo_upstream`,
`ws_upstream` and `tls_client` helpers. The echo upstream returns the request it
saw as JSON, so path and headers can be checked.

| Test file | Case |
|---|---|
| `proxy.rs` | T4: `/blog/x?y=1` reaches the `/blog` upstream as `/blog/x?y=1`; `/x` reaches the default upstream |
| `proxy.rs` | T4: `strip_path`: `/api/users?x=1` arrives as `/users?x=1`, `/api` as `/`; `X-Forwarded-Prefix: /api`; a client `X-Forwarded-Prefix: /evil` is replaced |
| `proxy.rs` | T4: `Host` is unchanged with and without `strip_path` (extends the ADR 01 T5 check) |
| `proxy.rs` | T4: the `/blog` upstream is closed; `/blog/x` answers 502 naming the `/blog` target, while the default upstream is up |
| `proxy.rs` | T4: WebSocket upgrade on `/blog/_next/webpack-hmr` reaches the `/blog` upstream |
| `proxy.rs` | T4: TLS handshake for a name whose only route has a path succeeds; a name with no route is still refused (extends `tls_handshake_is_refused_for_names_without_route`) |
| `proxy.rs` | T4: 404 page lists `shop.localhost/blog` |
| `proxy.rs` | T5: log entries carry `route: "shop/blog"`, `route: "shop"`, and no `route` for a 404 and for `router.localhost` |

### Daemon

`apps/daemon/tests/api.rs` (real daemon) and `apps/daemon/src/store.rs`
`mod tests`.

| Test file | Case |
|---|---|
| `api.rs` | T6: register `shop` and `shop/blog`; `list_routes` has both, with URLs `https://shop.localhost/blog` |
| `api.rs` | T6: register `shop/blog` again with another port: `replaced: true`, `old_target` is the first port, `shop` untouched |
| `api.rs` | T6: `unregister_route {host: "shop"}` removes only `shop`; `shop/blog` stays (I28) |
| `api.rs` | T6: `unregister_route {host: "shop", path: "/blog"}` removes only `shop/blog` |
| `api.rs` | T6: an owned `feat-x.shop/blog` and a persistent `shop`: kill the owner, only `feat-x.shop/blog` goes (extends `owned_route_disappears_when_its_process_exits`) |
| `api.rs` | T6: a persistent path route survives a daemon restart |
| `store.rs` | T7: only default routes saved: file has `"version": 1` |
| `store.rs` | T7: one saved path route: file has `"version": 2`; removing it writes version 1 again |
| `store.rs` | T7: version 1 and version 2 files both load; version 3 is moved aside as today |

### Contract

| Test file | Case |
|---|---|
| `libs/core/tests/api_examples.rs` | T8: the new and changed examples decode and re-encode (automatic: the test walks the folder) |
| `libs/core/tests/api_examples.rs` | T8: a new check fails when no example has `path`, `strip_path`, a `route` log field, or an `unregister_route` request with `path` (I30) |
| `ApiContractTests.swift` | T8: the same examples round-trip in Swift (automatic) |
| `LocalRouterKitTests/RouteKeyTests.swift` (new) | T8: two `RouteView`s with host `shop` and paths none and `/blog` have different `id`s |
| `LocalRouterKitTests/RouteKeyTests.swift` (new) | T8: the params `DaemonClient.unregister(_ route:)` sends for `shop/blog` encode to `{"host":"shop","path":"/blog"}` |

### Command line and MCP

| Test file | Case |
|---|---|
| `apps/cli/tests/cli.rs` | T9: `add shop 3001 --path /blog`, `list` shows `shop.localhost/blog`, `rm shop --path /blog` removes it |
| `apps/cli/tests/cli.rs` | T9: `rm shop` with path routes left prints `shop/blog remain` |
| `apps/cli/tests/cli.rs` | T9: `--strip-path` without `--path` and `--path` with `--tcp` fail before any socket call (work without a daemon) |
| `apps/cli/tests/cli.rs` | T9: `add shop/blog 3001` shows both hints (extends `invalid_name_shows_the_slug_hint`) |
| `apps/cli/tests/cli.rs` | T9: `logs shop` shows the route column |
| `apps/cli/tests/cli.rs` | T9: `which https://feat-x.shop.localhost/products` prints `shop` and the fallback step; `which` for a name with no route prints the 404 case; `which` on a TCP listen port prints the TCP route line |
| `apps/cli/tests/cli.rs` | T9: pattern B: three routes to one port, `/products/x` and `/brands` kept, `/shop-assets/_next/a.js` arrives as `/_next/a.js` |
| `apps/cli/tests/mcp.rs` | T10: `exactly_six_tools_are_listed` also expects `path`, `strip_path` on `register_route` and `path` on `unregister_route` |
| `apps/cli/tests/mcp.rs` | T10: `register_route` with an unknown argument `pth` returns a tool error and creates no route |
| `apps/cli/tests/mcp.rs` | T10: `register_route` with `path` then `unregister_route` with `path` round-trip |

### Agent texts

| Test file | Case |
|---|---|
| `libs/core/src/help.rs` `mod tests` | T11: `{{ROUTES}}` lists `https://shop.localhost/blog → http://127.0.0.1:3001` and marks strip routes (extends `routes_are_listed_with_address_target_kind_and_note`) |
| `libs/core/tests/agent_texts.rs` (new) | T11: `help.md` contains `--path`, `basePath`, `assetPrefix`, `strip` and `localrouter which`; `scripts/LocalRouter.md` contains `--path`; all three texts (help page, note, MCP `INSTRUCTIONS`) contain `production build` (the ask-first rule) (files read with `include_str!`) |

## 2. Automated end-to-end test: E1b

`apps/cli/tests/e2e.rs`, a second journey beside `main_journey`, with the same
helpers (`echo_upstream`, real `localrouterd`, real `localrouter`, real
`localrouter mcp` over rmcp).

1. Start the daemon and three echo upstreams: A (main), B (blog), C (api).
2. CLI: `add shop A`. Check: `GET http://shop.localhost/blog` reaches A.
3. MCP: `register_route` with `host: shop`, `path: /blog`, `port: B`. Check:
   `/blog/x` reaches B, `/x` still reaches A.
4. CLI: `add shop C --path /api --strip-path`. Check: `/api/users` reaches C as
   `/users` with `X-Forwarded-Prefix: /api`.
5. HTTPS: `GET https://shop.localhost/blog` over TLS through the real
   certificate hook. Check: 200 from B.
6. Start a child process; MCP: `register_route` `feat-x.shop`, `/blog`, a
   fourth upstream D, `owner_pid` of the child. Check:
   `feat-x.shop.localhost/blog` reaches D, `feat-x.shop.localhost/x` reaches A
   (Scenario A).
7. Kill the child. Check within 1 second: `feat-x.shop.localhost/blog` reaches
   B, `shop/blog` still in `list_routes`.
8. Stop B. Check: `/blog/x` answers 502 naming B, not A.
9. `localrouter logs shop`: entries carry the route keys `shop/blog`, `shop`,
   `shop/api`.
10. For each request made in steps 2 to 8, `localrouter which <same url>`
   prints the same route key as that request's log entry (I33, against the
   real daemon's table).
11. CLI: `rm shop`. Check: `shop/blog` and `shop/api` remain; `/x` answers 404.

## 3. Manual tests

### M1. Real frameworks under a path

Needs Node.js. No cost.

| Check | Expected |
|---|---|
| Create a Next.js app with `basePath: "/blog"`, run `next dev -p 3001`; a second Next.js app without base path on 3000; `localrouter add shop 3000 && localrouter add shop 3001 --path /blog` | both routes listed |
| Open `https://shop.localhost/blog` | blog page with styles; no failed requests in the browser network tab |
| Edit a blog page | page updates without reload (hot reload socket under `/blog`) |
| Open `https://shop.localhost/` and edit a page of the main app | same |
| A Vite app with `base: "/admin/"` on 5174, `--path /admin` | page, styles and hot reload work |
| Pattern B: a Next.js app with `assetPrefix: "/shop-assets"` and pages `/products`, `/brands` on 3002; three routes as in [01](01-route-key.md) | pages load with styles; hot reload works, or the socket path it uses is recorded |
| A plain `<img src="/logo.png">` in the `basePath` app | image missing, as the help page says; `localrouter which https://shop.localhost/logo.png` prints `shop` |
| Record the hot reload socket paths seen in `localrouter logs shop` | noted in this ADR for [03](03-forwarding.md) |

### M2. The failure without a base path

| Check | Expected |
|---|---|
| Remove `basePath` from the blog app, restart it | |
| Open `https://shop.localhost/blog` | page renders without styles (the documented failure) |
| `localrouter logs shop` | `/_next/static/...` entries with `404` and route `shop` |
| Could a developer find the cause from the log alone, in under 5 minutes? | record the answer; it decides U6 |

### M3. Claude Code acceptance

A new Claude Code session in a project with two Next.js apps. No path routes
exist.

| Check | Expected |
|---|---|
| Ask: "run the blog on shop.localhost/blog and the main app on shop.localhost" | the agent reads the note, fetches `router.localhost`, uses `path` |
| Before it changes `basePath` in the blog app, the agent asks and says that it changes the production build | yes; a change without asking is a failure of I32's text |
| The agent checks the production split first (for example a proxy config in the repository) and picks pattern A or B to match | yes |
| Ask: "why does feat-x.shop.localhost/products show the main app?" | the agent runs `localrouter which` and explains the fallback |
| The agent writes the "Local URLs" section with the path line | yes |
| Ask the agent to start a worktree of the blog | it registers `<branch>.shop` with path `/blog` and `owner_pid` |
| Close the session, start a new one with an MCP server from before the update still running elsewhere | not required; B2 is documented, not tested |

### M4. Menu bar app

| Check | Expected |
|---|---|
| With `shop` and `shop/blog`, open Domains | two rows, `shop.localhost` and `shop.localhost/blog` |
| Remove the `/blog` row | only `/blog` goes; `shop.localhost` still opens (B1, Scenario A) |
| Logs tab | entries show the route key |

### M5. Downgrade

| Check | Expected |
|---|---|
| Save a persistent path route, install the previous release | status shows `routes_file_problem`; `routes.json.bad-<time>` exists |
| Reinstall the new release, move the `.bad` file back while the daemon is stopped | routes come back |
| Remove path routes first, then downgrade | no problem; default routes kept (I29) |

### M6. Post-release smoke test

Free, no data written outside the test route.

| Check | Expected |
|---|---|
| After the update: `localrouter status` | API version 1.1 |
| `localrouter add smoke 8000 --path /x --session` with `python3 -m http.server 8000` running | `curl -s -o /dev/null -w "%{http_code}\n" https://smoke.localhost/x/` prints `404` from the Python server (it has no `/x`), and `localrouter logs smoke` shows route `smoke/x` |
| `localrouter rm smoke --path /x` | removed |

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| I20 route key | T1, T6 | M4 |
| I21 path rules | T1 | |
| I22 protocol rules | T2 | |
| I23 match rule | T3 | |
| I24 lookup order, regression | T3, E1b | M1 |
| I25 no fallback on error | T4, E1b | |
| I26 path kept or stripped | T4, E1b | M1 |
| I27 certificate by name | T4, E1b | M1 |
| I28 exact removal | T6, E1b | M4 |
| I29 file version | T7 | M5 |
| I30 examples cover new fields | T8 | |
| I31 MCP six tools, arguments | T10 | M3 |
| I32 texts mention path routes, both patterns, ask first | T11 | M3 (wording, behaviour) |
| I33 `which` agrees with the proxy | T3, E1b | M1 |

## Not in this plan without approval

- **A browser test tool** (for example Playwright) to automate M1 and M2 with real
  Next.js and Vite apps. It would turn the only real-framework coverage into a
  repeatable test, at the cost of a Node.js toolchain in the test setup and
  minutes per run.
