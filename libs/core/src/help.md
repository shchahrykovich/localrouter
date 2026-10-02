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
| `get_proxy` | `{{CLI}} proxy` | Proxy URL, environment variables, Chrome flags, script rules (see "Proxy" below) |
| `set_script_rule` | `{{CLI}} rules add <id> --host <host> --script <file.lua>` | Run a Lua script on a host's traffic (see "Scripts" below) |
| `remove_script_rule` | `{{CLI}} rules rm <id>` | Remove a script rule |
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
- macOS keeps {{APP}} out of Desktop, Documents, Downloads and iCloud
  Drive: a folder there answers `403`. Use a folder outside them, for example
  in the project, or ask the user to allow {{APP}} in System Settings >
  Privacy & Security.

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

## Proxy: see what a browser or a program sends

{{APP}} can also be the HTTP proxy of a program, so the user sees which
servers it calls. It is off by default and listens on
`127.0.0.1:{{PROXY_PORT}}` only.

1. Turn it on: `{{CLI}} proxy on`. Then `{{CLI}} proxy` (MCP `get_proxy`)
   shows the URL, the environment variables and the Chrome flags.
2. Start a program through it:
   - a command: `eval "$({{CLI}} proxy env)" && npm test`
   - Chrome: `{{CLI}} proxy chrome` opens a separate Chrome window with its
     own profile. The user's normal Chrome does not change.
3. `{{CLI}} logs` (MCP `get_logs`) shows each request, marked `via proxy`.

- HTTPS is inspected by default: the inspect list is `*`, so the proxy reads
  each request of every host outside `.localhost`. Apps that pin
  certificates fail. To read only some hosts:
  `{{CLI}} proxy inspect rm '*'`, then
  `{{CLI}} proxy inspect add api.example.com` (or `*.example.com`). A host
  that is not inspected is a tunnel: the log shows the host, not the
  requests. Quote the `*` in a shell.
- An inspected host works only in a program that trusts the inspection CA.
  Only the user can trust it, because macOS asks for their password: ask them
  to run `{{CLI}} proxy trust`. Node.js and Claude Code read
  `NODE_EXTRA_CA_CERTS` instead; `{{CLI}} proxy env` sets it.
- A program reads the proxy settings when it starts. You cannot change the
  proxy of a program that is already running, and that includes yourself.
  Start other programs with the `env` values, or ask the user to restart
  Claude Code with `eval "$({{CLI}} proxy env)" && claude`.
- **Never write proxy settings into project files** (`.env`,
  `.claude/settings.json`, test configs). Other people on the project may not
  have {{APP}}, and every request of theirs would fail. Pass the values to a
  command, or let the user put them in their own shell.
- `.localhost` names do not go through the proxy (`NO_PROXY`); they work as
  before.
- `{{CLI}} proxy off` closes the port. Programs started with the proxy
  settings then fail to connect until they are started again without them.

### Proxy clients: one port per program

To tell programs apart in the log, give each one a **proxy client**: one
more proxy port with a name. The log marks each request with the name of the
port that carried it (`_client`; the main port is `default` and writes no
`_client`).

1. `{{CLI}} proxy client add agent-1` (names: `a-z`, `0-9`, `-`). It takes
   the next free port after the main one; `--port 8890` picks one.
2. Start the program on it: `eval "$({{CLI}} proxy env --client agent-1)" && npm test`,
   or `{{CLI}} proxy chrome --client agent-1` (a Chrome profile of its own).
   MCP `get_proxy` with `client` gives the same values.
3. The user sees only its requests at {{PROXY_LOG_URL}}/agent-1, or picks it
   in the "Proxy" menu of the log viewer.

`{{CLI}} proxy client` lists the clients, `{{CLI}} proxy client rm agent-1`
closes the port; the entries stay in the log. Client ports open and close with
`{{CLI}} proxy on` and `off`.

## Proxy log: what a program sent, in HAR files

The proxy writes every request it carries to HAR files (the format Chrome
DevTools imports) in `{{PROXY_LOG_FOLDER}}`. The user sees them at
{{PROXY_LOG_URL}}. The log is on by default and writes only while the proxy
is on; `{{CLI}} proxy log` shows its state, `{{CLI}} proxy log off` stops it.

- Headers are written as they are, cookies and API keys too. URLs are
  written as they are, query included. Request and response bodies are
  written too, the first 1 MB of each, decoded from gzip, br or zstd
  (`content.text`; base64 when they are not text). A WebSocket is written
  when it closes, with its messages in `_webSocketMessages`.
- **Do not read a whole file**: one can be 100 MB. Use `jq` and ask for the
  rows you need.
- The current file can be in the middle of a write. If `jq` fails on it, run
  it again.
- `jq` is part of macOS 15 and later. On macOS 14 use any JSON tool, or
  `brew install jq`.

```sh
f=$(ls -t "$({{CLI}} proxy log path)"/proxy-*.har | head -1)
jq -r '.log.entries[] | select(.response.status >= 400 or .response.status == 0) | "\(.response.status) \(.request.method) \(.request.url)"' "$f"
jq '.log.entries[] | select(.request.url | contains("api.example.com")) | {url: .request.url, status: .response.status, headers: .response.headers}' "$f" | head -c 20000
jq -r '.log.entries[] | select(._client == "agent-1") | "\(.response.status) \(.request.url)"' "$f"
```

The whole loop, on your own:

1. Operate: `{{CLI}} proxy on`, `{{CLI}} proxy inspect add api.example.com`.
   The user sees each command.
2. Check: MCP `get_proxy` (port, env, inspected hosts, CA trust, `log`).
3. Run: `eval "$({{CLI}} proxy env)" && npm test`. `proxy env` also points
   Python and `curl` at a CA bundle (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`,
   `CURL_CA_BUNDLE`): the macOS roots plus the inspection CA.
4. Inspect: `jq` on the newest file, as above.

Ask the user to run `{{CLI}} proxy trust` only when Chrome or Safari must
read an inspected host.

## Scripts: change or record traffic

A script rule runs a Lua file on the HTTP traffic of a route
(`shop.localhost`) or of a host the proxy carries (`api.example.com`).

- An **intercept script** runs while the client waits: it may change a
  request, answer it without the server, change a response, and change or
  drop each event of a stream.
- A **log script** gets a copy of each finished request and response and
  writes files in its `output_dir`. It never slows or changes traffic.

The reference, with the fields, the modules and the limits:
`{{CLI}} rules api`, or `curl -s {{HELP_URL}}/scripts`. Read it before you write a
script.

1. Test the script first, with nothing stored: `{{CLI}} rules check ./capture.lua`
   (MCP `set_script_rule` with `check_only: true`).
2. Set the rule **before** you start the job, so it sees the first requests:
   `{{CLI}} rules add claude --host api.anthropic.com --path /v1/messages --script ./capture.lua --output-dir ./captures`
3. Give the rule `owner_pid` of a process you started for the job, or remove
   it when you are done: `{{CLI}} rules rm claude`. Make a rule `persistent`
   only when the user asks.
4. `{{CLI}} rules` (MCP `get_proxy`, `script_rules`) shows `matched`,
   `errors` and `last_error`. A rule is turned off after 20 failed calls in a
   row.

- A log rule never needs `response_body`. An intercept rule with
  `response_body` holds the whole body, so **streaming stops**; use
  `on_event` for event streams.
- Bodies come in classes: text, events, media, multipart, binary. A log rule
  copies text and events by default. For streams use `on_event`; to keep
  images, video, downloads or uploads use `save`, which writes files, not
  `copy`.
- Scripts see every header as it is, cookies and API keys too.
- Scripts may live in the project, but **do not commit captures**: keep
  `output_dir` out of git. Captures hold prompts and API replies.
- A rule on a host outside `.localhost` makes the proxy inspect that host:
  the program must use the proxy and trust the inspection CA (see "Proxy").

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
