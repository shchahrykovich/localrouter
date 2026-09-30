# {{APP}}

{{INSTANCE_NOTE}}{{APP}} {{VERSION}} runs on this Mac. It gives local dev servers names
instead of ports:

- HTTP: `https://shop.localhost{{HTTPS}}` goes to `http://127.0.0.1:5173`
- HTTP by path: `https://shop.localhost{{HTTPS}}/blog` goes to `http://127.0.0.1:3001`,
  and every other path of `shop.localhost` to 5173
- TCP: `db.shop.localhost:15432` goes to `127.0.0.1:5432` (databases, caches)
- Folder: `https://coverage.shop.localhost{{HTTPS}}` serves the files of a folder,
  with no dev server

macOS sends every `*.localhost` name to this Mac by itself. There is no DNS
setup and no `/etc/hosts` change.
{{COOKIE_NOTE}}
This page is for a coding agent. If the user sent you here, add LocalRouter to
the project in the current folder: follow the steps below, then tell the user
the new URLs.

Read this page again at any time: `{{CLI}} guide`, or `curl -s {{HELP_URL}}`

## Status

{{STATUS}}

If a part is off, "no" or broken, see "Turn on HTTP, HTTPS and trust" below.

## Routes on this Mac now

{{ROUTES}}

## Step 1: find a way to talk to LocalRouter

Use the first one that works:

1. MCP tools `register_route`, `list_routes` and the others below. If you have
   them, use them.
2. The command line tool: run `{{CLI}} status`. It is usually
   `~/.local/bin/{{CLI}}`.
3. Neither works: ask the user to open the {{APP}} menu bar app and choose
   **⋯ > Install Command Line Tool…**. Then Claude Code gets the MCP tools with
   `claude mcp add {{CLI}} -- ~/.local/bin/{{CLI}} mcp`

| MCP tool | Command line | What it does |
|---|---|---|
| `register_route` | `{{CLI}} add <host> <port> [--path /blog]`, `{{CLI}} add <host> --folder <dir>` | Create or replace a route |
| `unregister_route` | `{{CLI}} rm <host> [--path /blog]` | Remove a route |
| `list_routes` | `{{CLI}} list` | All routes, their URLs, and whether each target is up |
| `find_free_port` | - | A free TCP port on 127.0.0.1 |
| `get_logs` | `{{CLI}} logs [host]` | Recent requests and connections |
| `status` | `{{CLI}} status` | Daemon version, ports, local CA |
| - | `{{CLI}} which <url>` | Which route answers a URL, and why (no MCP tool) |

## Step 2: choose names

- A host is the name without `.localhost`: `shop`, `api.shop`, `feat-login.shop`.
- A label (the part between dots) uses `a-z`, `0-9` and `-`, 1 to 63
  characters. Turn `feat/login` into `feat-login`.
- Use the project folder name for the main server: `shop`.
- Put the other servers of the project under it: `api.shop`, `db.shop`.
- For a git branch or worktree, put the branch first: `feat-login.shop`.
- A name without its own route uses its parent: `other.shop` goes to `shop`.
- One site that is split by path in production (`/blog`, `/admin`): one host,
  one route per path (Step 4b). Separate sites: separate hosts.
- `router` is taken by this page.
- Run `{{CLI}} list` first. Adding a host and path that exist replaces that
  route.

## Step 3: find the dev servers

Look for the ports the project uses:

- `package.json` scripts and framework config (Vite 5173, Next.js 3000)
- `docker-compose.yml` or `compose.yaml`, the `ports:` lists
- `Procfile`, `Makefile`, `.env` files, `README`

The target must be on this Mac: `127.0.0.1`, `localhost` or `[::1]`.

## Step 4: add routes

HTTP server on port 5173:

```
{{CLI}} add shop 5173 --note "main dev server"
```

With MCP: `register_route` with `host: "shop"`, `port: 5173`,
`note: "main dev server"`, `persistent: true`.

Other cases:

- The server speaks HTTPS itself: `--target https://127.0.0.1:8443`
- Always send the browser to HTTPS: add `--https-only`
- A database or another TCP server on port 5432:
  `{{CLI}} add db.shop 5432 --tcp --listen 15432`.
  Clients connect to `db.shop.localhost:15432`. A TCP route is chosen by its
  listen port only; the name is for people.

Files with no server: a build folder, a test report, HTML you just wrote:

```
{{CLI}} add coverage.shop --folder ./coverage --session
```

With MCP: `register_route` with `host: "coverage.shop"` and
`folder: "/Users/me/shop/coverage"`. MCP needs an absolute path; the command
line makes a relative one absolute.

- A folder answers with its `index.html`, or else with a list of its files.
- Names that start with `.` (`.git`, `.env`) are never served, and neither are
  links that lead out of the folder.
- Files are read on every request: reload the page to see a change. There is
  no live reload.
- With `--path /docs`, `/docs/a.html` is `a.html` in the folder.
- Only GET and HEAD. For anything more, start a dev server.
- macOS may keep {{APP}} out of Desktop, Documents, Downloads and iCloud
  Drive (a `403` page). Use a folder outside them, or ask the user to allow
  {{APP}} in System Settings > Privacy & Security > Files and Folders.

How long a route lives:

| Kind | Command line | MCP | Ends when |
|---|---|---|---|
| persistent | default | `persistent: true` | You remove it |
| session | `--session` | default | LocalRouter restarts |
| owned | `--owner-pid <pid>` | `owner_pid: <pid>` | That process exits |

## Step 4b: several apps on one name

Use this when the apps of one site share a domain in production and a proxy
there picks the app by path. Locally they then share cookies and sign-in, as in
production.

```
{{CLI}} add shop 5173
{{CLI}} add shop 3001 --path /blog
```

With MCP: `register_route` with `host: "shop"`, `path: "/blog"`, `port: 3001`.

- `/blog` matches `/blog` and `/blog/...`, never `/blogger`. Case matters.
- The route without a path gets every path no other route of the name matches.
- The longest matching path wins: `/blog/admin` before `/blog`.
- A path route whose server is down answers 502. The request never goes to
  another route.
- Remove a path route with `{{CLI}} rm shop --path /blog`. Without
  `--path`, only the route without a path is removed.

**The app must know its path.** LocalRouter sends the path unchanged and
rewrites nothing in the answer. First look at how production splits the site,
and use the same pattern:

| Pattern | The app's pages | The app's files | App setting | Routes |
|---|---|---|---|---|
| A: base path | under `/blog/...` | under `/blog/_next/...` | Next.js `basePath: "/blog"`, Vite `base: "/blog/"`, Astro `base: "/blog"` | `--path /blog` |
| B: own pages, file prefix | `/products/...`, `/brands/...` | under `/shop-assets/_next/...` | Next.js `assetPrefix: "/shop-assets"` | one route per page prefix, and one for the file prefix |

Pattern B, one app on port 3002:

```
{{CLI}} add shop 3002 --path /products
{{CLI}} add shop 3002 --path /brands
{{CLI}} add shop 3002 --path /shop-assets
```

Do not add `--strip-path` to the file prefix for `next dev`: the dev server
serves its files under the prefix itself, and its hot reload socket
(`/shop-assets/_next/hmr`) works only with the prefix. A production proxy may
strip the prefix; the dev server does not need it.

Give all routes of one dev server the same `owner_pid`, so they go away
together.

**Ask the user before you change `basePath`, `base` or `assetPrefix`.** These
settings change the production build too, not only the dev server.

`--strip-path` (MCP `strip_path: true`) removes the path before the request
reaches the server: `/api/users` arrives as `/users`, with the header
`X-Forwarded-Prefix: /api`. Use it for an API or another server that answers
at `/` and does not know its prefix. Redirects and cookies from a stripped
server are not rewritten, so do not use it for the pages of a web app.

If `register_route` has no `path` argument, your MCP server is older than
LocalRouter: restart the session, or use the command line.

## Step 5: a server for a branch or worktree

1. Get a free port: `find_free_port`.
2. Start the dev server on that port.
3. Register `<branch>.<project>` with `owner_pid` set to the dev server's pid.
   The route goes away when the server stops.
4. Give the user `https://<branch>.<project>.localhost{{HTTPS}}`.

A branch of one path app: register `<branch>.<project>` with the same path and
`owner_pid`, for example `feat-x.shop` with path `/blog`. Other paths of the
branch name use the project's routes, so the rest of the site still works.

## Step 6: check that it works

```
curl -s -o /dev/null -w "%{http_code}\n" http://shop.localhost{{HTTP}}/
```

- `502` and a LocalRouter page: the dev server does not run, or it listens on
  another port. For a folder route: the folder is gone.
- `403`, "Invalid Host header" or "Blocked request": the dev server checks the
  `Host` header. Allow `.localhost`: Vite `server.allowedHosts`,
  webpack-dev-server `allowedHosts`, Django `ALLOWED_HOSTS`, Rails
  `config.hosts`.
- HTTPS errors: see "Turn on HTTP, HTTPS and trust" below.
- `{{CLI}} logs shop` shows what reached LocalRouter, and which route
  answered each request (`route shop/blog`).
- `{{CLI}} which https://shop.localhost{{HTTPS}}/blog/x` shows which route answers
  a URL, and why, without a request.
- A page under `/blog` with no styles, or with the styles and scripts of
  another app: the app asks for `/_next/...` or `/@vite/...` outside its path,
  and the route without a path answers that. Dev file names such as
  `main-app.js` are the same in every Next.js app, so the answer is often 200
  with the wrong file, not an error. `{{CLI}} logs <host>` shows `route
  shop` on those files. Set the base path in the app (Step 4b).
- A wrong or missing image under a base path: a plain `<img src="/logo.png">`
  does not get the base path, so another app may answer it. Use `next/image`,
  or write the base path into the URL.

## Turn on HTTP, HTTPS and trust

The user does this in the {{APP}} menu bar app: click its icon in the menu
bar, then open the **Settings** tab. Ask the user first; do not change their
keychain on your own.

- **CA trusted by macOS: no.** Browsers warn on `https://` URLs. In
  **Settings > HTTPS certificate authority**, click **Trust…**. macOS asks for
  the user's password once. From a terminal: `{{CLI}} trust`
- **HTTP or HTTPS: off.** Another program uses port {{HTTP_PORT}} or {{HTTPS_PORT}}.
  **Settings > Daemon** shows "not listening" and the error in red. Find the
  program with `sudo lsof -nP -iTCP:{{HTTPS_PORT}} -sTCP:LISTEN` (or `:{{HTTP_PORT}}`), stop it, then
  restart the daemon:
  `launchctl kickstart -k gui/$(id -u)/{{DAEMON_LABEL}}`
- **Local CA: broken.** HTTPS is off. `{{CLI}} ca reset --yes` makes a new
  CA; after that, trust it again. The old CA stops working.
- **Node.js, Python and other tools** do not read the macOS keychain. Point
  them to the CA file:
  `export NODE_EXTRA_CA_CERTS="$({{CLI}} ca-path)"` or
  `export REQUESTS_CA_BUNDLE="$({{CLI}} ca-path)"`
- **Firefox:** open `about:config` and set
  `security.enterprise_roots.enabled` to `true`.

## Step 7: write it down in the project

Add a short section to the project's agent instructions (`CLAUDE.md` or
`AGENTS.md`) or to the README. Use the real names and ports:

{{STEP7_NOTE}}
```
## Local URLs (LocalRouter)

- https://shop.localhost - dev server, port 5173
- https://shop.localhost/blog - blog app, port 3001 (Next.js basePath /blog)
- db.shop.localhost:15432 - Postgres, port 5432

Set up once: localrouter add shop 5173 && localrouter add shop 3001 --path /blog && localrouter add db.shop 5432 --tcp --listen 15432
Branch or worktree: register <branch>.shop with owner_pid of the dev server.
Instructions: curl -s http://router.localhost
```

Do not make the project need LocalRouter. If you add `localrouter add` to a dev
script, skip it when the tool is missing:
`command -v localrouter >/dev/null && localrouter add shop 5173 --session`
