# 0. Working backwards: what users might say

**Status:** Simulated. No user has seen this feature. Every quote below is
written by us to test the design before it is built. None of it is a real user
statement.

"Working backwards" means: write the announcement and the user reactions first,
then check that the design answers them. Each reaction below is checked against
the ADR and marked:

| Mark | Meaning |
|---|---|
| **covered** | the ADR decides it; the link says where |
| **accepted** | the ADR knows about it and chose to leave it; the link says why |
| **gap, fixed** | the first draft of the ADR did not answer it; this simulation found it, and the ADR now does. The link says where. |

## The announcement

> **LocalRouter 0.2: one name, several apps**
>
> Many sites are more than one app. The shop is one app, the blog is another,
> the admin screens are a third, and in production one proxy puts them all on
> one domain by path. Until now LocalRouter gave each of them its own name, so
> sign-in cookies, links and requests between the apps did not work locally the
> way they work in production.
>
> Now one name can serve several dev servers:
>
> ```
> localrouter add shop 5173
> localrouter add shop 3001 --path /blog
> ```
>
> `https://shop.localhost/blog` goes to the blog on port 3001, and every other
> path of `https://shop.localhost` goes to the shop on port 5173. Coding agents
> do the same through the `register_route` tool with `path: "/blog"`. A git
> worktree can take over one path on its own branch name, for example
> `feat-x.shop.localhost/blog`, and keep the rest of the site from the main
> checkout.
>
> Each app must know its path, as it does in production: Next.js `basePath`,
> Vite `base`. For an API that answers at `/`, add `--strip-path`.

## Who reacts

| Role | Setup | What they want |
|---|---|---|
| R1. Full-stack developer | three Next.js or Vite apps that share one production domain | the production shape on the laptop |
| R2. Developer who works through Claude Code | agent sessions in several worktrees | "run the blog branch" without breaking the main site |
| R3. API developer | a frontend at `/` and an API server that answers at `/`, mounted at `/api` in production | no CORS, one origin |
| R4. Developer with one app | one dev server per project | nothing to change |
| R5. Team lead | a team where not everyone has LocalRouter | nothing in the repository that needs LocalRouter |

## Positive reactions

**R1: "Finally I can sign in once and move between the shop and the blog."**
The apps share one origin, so they share cookies, as in production.
**covered**, [03](03-forwarding.md), "What is never rewritten".

**R2: "My agent started the blog branch on `feat-x.shop.localhost/blog`, and the
product pages on the same name still came from my main checkout."**
This is the fallback from a branch host to the project host, per path.
**covered**, [02](02-path-lookup.md), Scenario A.

**R3: "The frontend calls `/api/users` and there is no CORS setup any more."**
`--path /api --strip-path` sends `/users` to the API and adds
`X-Forwarded-Prefix: /api`. **covered**, [03](03-forwarding.md).

**R4: "I updated and nothing changed."**
A table without path routes answers exactly as before. **covered**, invariant
I24, with a regression test.

**R5: "Nothing about LocalRouter is in our repository."**
The routes live on each Mac. The only repository change is the base path in the
app, which production needs anyway. A teammate without LocalRouter opens
`http://localhost:3001/blog` directly. **covered**, the texts keep the rule "do
not make a project need LocalRouter".

## Confused or negative reactions

### About the apps themselves

**R1: "The blog page loads, but it has no styles and no scripts."**
The blog app has no base path, so it asks for `/_next/static/...`, and the
default route answers that. **accepted** with help: the log shows
`route shop` on those requests, and the help page names the cause.
[03](03-forwarding.md), "The problem LocalRouter cannot fix". A warning in the
daemon is open point U6.

**R1: "I set `basePath`, and now my `<img src="/logo.png">` is broken."**
In Next.js, `basePath` is added by `next/link` and `next/image`, but not by a
plain `<img>` tag. The same break happens in production. **gap, fixed (G4)**:
this is app work, and the help page now says it in Step 6.
[03](03-forwarding.md), [04](04-clients-and-agent-texts.md).

**R1: "My app does not use a base path in production. It owns `/products` and
`/brands`, and it moves only its files under `/shop-assets`, which the
production proxy strips."**
The ADR can express this: one route per prefix to the same port, and one
`strip_path` route for the file prefix:

```
localrouter add shop 3001 --path /products
localrouter add shop 3001 --path /brands
localrouter add shop 3001 --path /shop-assets --strip-path
```

**gap, fixed (G1)**: this is pattern B in [03](03-forwarding.md), and the help
page now shows both patterns. **gap, fixed (G2)**: the ADR now decides one
route per path, with the same `owner_pid` for all routes of one dev server, and
says why a route does not get a list of paths. [01](01-route-key.md), "One app
with several paths".

**R2: "Claude added `basePath` to our `next.config` without asking, and the
production build changed."**
`basePath` is not only for local development. It changes the production build.
**gap, fixed (G3)**: the note, the MCP instructions and the help page now all
tell the agent to ask before it changes `basePath`, `base` or `assetPrefix`,
and to use the pattern production already uses. Invariant I32, test T11, and
manual test M3. [04](04-clients-and-agent-texts.md).

**R3: "My API redirects to `/login`, and the browser leaves `/api`."**
`strip_path` does not rewrite the `Location` header. **accepted**,
[03](03-forwarding.md): rewriting would make local differ from production. The
API should read `X-Forwarded-Prefix` or return full paths.

**R1: "Vite hot reload does not connect under `/admin`."**
Not known yet. The real socket paths are checked in manual test M1.
**covered by a test**, not by a decision: [08](08-test-plan.md), M1.

### About LocalRouter's own behaviour

**R1: "I clicked remove on the `/blog` row, and my main site went down."**
This happens only if the app keeps sending the host alone. **covered**: the app
change is in the same task as the daemon change, and the Swift request is
tested. [07](07-semantic-change-manifest.md), B1.

**R2: "I asked Claude to put the blog on `/blog`, and it replaced my main
route."**
This happens when an MCP server started before the update drops the `path`
argument. **accepted with mitigation**, [07](07-semantic-change-manifest.md), B2.
The help page tells the agent to restart the session.

**R1: "`localrouter add shop/blog 3001` says `Try shop-blog`."**
**covered**: the message gains the second hint, `or host "shop" with --path
/blog`. [04](04-clients-and-agent-texts.md), "A path typed into the host".

**R1: "The blog dev server is not running, and `/blog` shows a LocalRouter 502
instead of the shop's page."**
**covered**, by design: a broken app must look broken. Invariant I25.

**R2: "`feat-x.shop.localhost/blog` shows my branch's shop, not the blog."**
The branch host has its own default route, and a nearer host key always wins.
**accepted**, [02](02-path-lookup.md). **gap, fixed (G5)**: the user needed a
fast way to see which route answers a URL. `localrouter which
https://feat-x.shop.localhost/blog` now prints the route and each step of the
lookup. [02](02-path-lookup.md), "Explaining a lookup"; invariant I33.

**R1: "`http://shop.localhost/blog` goes to HTTPS, but `http://shop.localhost/`
does not."**
`https_only` is per route. **accepted**, [07](07-semantic-change-manifest.md),
risks.

**R1: "I want to remove everything under `shop` at once."**
**accepted for now**: open point U7.

**R5: "I went back to the old version, and all my saved routes are gone."**
Only when a path route was saved; the file is moved aside, not deleted.
**covered**, [07](07-semantic-change-manifest.md), rollback, and manual test M5.

**R3: "Can I route `/api/v2` to another server than `/api`?"**
Yes: the longest path wins. **covered**, [02](02-path-lookup.md).

**R3: "Can I route by a pattern, like `/*.php`?"**
No. **accepted**, README, "Not in this ADR".

## A simulated agent session

This is how we expect Claude Code to act with the texts from
[04](04-clients-and-agent-texts.md). It is the script for manual test M3.

1. The user says: "Run the blog on shop.localhost/blog, next to the main app."
2. Claude reads `~/.claude/LocalRouter.md`. It finds "Path routes:
   `localrouter add shop 3001 --path /blog`" and the base path rule.
3. Claude runs `curl -s http://router.localhost` and reads Step 4b.
4. Claude opens `apps/blog/next.config.ts` and finds no `basePath`.
5. Claude asks: "The blog needs `basePath: "/blog"`. This also changes the
   production build. Add it?" The note says so, so this holds even if Claude
   has not read the help page yet (G3).
6. Claude calls `find_free_port`, starts the blog on that port, and calls
   `register_route` with `host: "shop"`, `path: "/blog"`, `owner_pid` of the
   dev server.
7. Claude checks `curl -s -o /dev/null -w "%{http_code}\n"
   https://shop.localhost/blog/` and `localrouter logs shop`, and sees
   `route shop/blog` on the page and on its `/blog/_next/...` files. If a file
   is missing, it runs `localrouter which` on that URL (G5).
8. Claude writes the "Local URLs (LocalRouter)" section with the `/blog` line.

## Gaps found by this simulation, and how they were fixed

The first draft of the ADR did not answer these five. All five are now fixed
in the change files.

| # | Gap | Fix | Where |
|---|---|---|---|
| G1 | The help page showed only the base-path pattern. The "several page prefixes plus a stripped file prefix" pattern worked but was not described. | The ADR names two patterns, A and B, and the help page shows both, with the commands. | [03](03-forwarding.md), "The two patterns a production proxy uses"; [04](04-clients-and-agent-texts.md), Step 4b; M1 |
| G2 | One app with several prefixes needs one route per prefix. | Decided: one route per path, no list of paths in a route. All routes of one dev server share its `owner_pid`. A new risk in the manifest covers a half-removed app. | [01](01-route-key.md), "One app with several paths"; [07](07-semantic-change-manifest.md), risks |
| G3 | The texts told the agent to set `basePath`, but not to ask first, although it changes the production build. | All three agent texts carry the ask-first rule, and the note carries it too, because every session reads the note. | [04](04-clients-and-agent-texts.md); invariant I32; T11, M3 |
| G4 | A plain `<img src="/...">` breaks under a base path. | One line in help page Step 6, and a check in M1. | [03](03-forwarding.md); [04](04-clients-and-agent-texts.md) |
| G5 | No way to ask "which route answers this URL?" without a request. | New read-only command `localrouter which <url>`. It runs `RouteTable::explain` in the CLI, the same code the proxy runs. No new socket method and no new MCP tool. | [02](02-path-lookup.md), "Explaining a lookup"; [04](04-clients-and-agent-texts.md); [06](06-data-flow.md), flow C; invariant I33; T3, T9, E1b |

## What this file does not do

It does not replace manual test M3. A real agent session and real users may
say things none of the roles above say. After the feature ships, compare real
reactions with this list and record the difference here.
