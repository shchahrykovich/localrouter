# 3. Forwarding: the path is kept unless `strip_path` is set

**Context.** Once a route is chosen, the proxy sends the request to the target
with the `Host` header unchanged and three `X-Forwarded-*` headers added
(`outgoing_request`, `libs/core/src/proxy.rs:189`). A path route adds one
question: does the dev server see `/blog/post-1` or `/post-1`?

![What the dev server sees, with and without strip_path](diagrams/03-strip-path.svg)

## Decision

### Default: the path is kept

The request path and query reach the dev server byte for byte. This is right for
an app that is built to live under its path, which is how the production proxy
of such a site works too:

| Framework | Setting | What it changes |
|---|---|---|
| Next.js | `basePath: "/blog"` in `next.config` | pages, links, `/_next/` files and the hot reload socket all move under `/blog` |
| Vite | `base: "/admin/"` in `vite.config` | built file URLs, the dev client and the hot reload socket move under `/admin/` |
| Astro | `base: "/docs"` | the same for Astro pages and files |

The exact paths of each framework's hot reload socket under a base path are to
be checked in manual test M1, not assumed.

### `strip_path: true`: the prefix is removed

For a dev server that answers at `/` and does not know its prefix, for example an
API server mounted at `/api`:

1. The route path is removed from the front of the request path:
   `/api/users?x=1` becomes `/users?x=1`, and `/api` becomes `/`.
2. The query string is kept.
3. `X-Forwarded-Prefix: /api` is set. A value sent by the client is removed
   first, so the dev server can trust it. FastAPI's `root_path`, Spring and
   others read this header to build correct links.
4. The `Host` header is still unchanged (invariant I12 of ADR 01).

### What is never rewritten

The proxy does not rewrite response headers or bodies, with or without
`strip_path`. That means:

- A `Location: /login` redirect from a stripped app sends the browser to
  `/login`, outside the app's prefix.
- A `Set-Cookie` with `Path=/` from any app is sent to all apps of the host,
  because they share one origin. This is the same as in production, and it is
  usually the reason to use one name.

Rewriting either would make the local setup differ from production in a way no
test in the app can see. So `strip_path` is recommended only for servers that
answer data (APIs) or read `X-Forwarded-Prefix`. For a web app, set the base
path in the app.

### The problem LocalRouter cannot fix: shared framework paths

![Two apps on one name ask for the same /_next/ files](diagrams/04-shared-asset-paths.svg)

Two Next.js apps without a base path both put their scripts at
`/_next/static/...`. On one name, those requests match no path route, so they
go to the default route. The main app answers 404 for the blog app's chunk
names, and the blog page renders without scripts and styles. No error names the
cause: every request got an answer, only from the wrong app. Vite apps have the
same problem with `/@vite/client` and `/src/...`.

A router cannot solve this, because the request for `/_next/static/a.js`
carries nothing that says which app's page asked for it. (The `Referer` header
could, but it is missing for some requests and not sent by every client, so a
guess from it would work most of the time and fail without a message the rest.)
The fix is in the app: a base path, as in the table above.

LocalRouter helps in two places:

1. **The request log names the route that answered.** HTTP log entries get a
   `route` field with the route key, for example `shop` or `shop/blog`.
   `localrouter logs shop` then shows `/_next/static/a.js 404 route shop`, which
   points at the cause.
2. **The texts that agents read say it.** The help page and the Claude Code
   note tell the agent to set the base path when it adds a path route (see
   [04](04-clients-and-agent-texts.md)).

A warning in the daemon itself (for example "a 404 from the default route for
`/_next/` while the host has path routes") is open point U6.

### WebSocket upgrades

Upgrades use the same lookup and the same path rule. A hot reload socket at
`/blog/_next/webpack-hmr` goes to the blog route; one at `/_next/webpack-hmr`
goes to the default route, which is the same failure as above and has the same
fix.

## Tests

- `T4`: path kept byte for byte; `strip_path` removes exactly the prefix, keeps
  the query, maps `/api` to `/`; `X-Forwarded-Prefix` set and a client value
  replaced; `Host` unchanged; WebSocket upgrade under a path route.
- `T5`: the log entry carries `route`.
- `M1`, `M2`: real Next.js and Vite apps, with and without a base path.
