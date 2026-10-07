# {{APP}}

{{INSTANCE_NOTE}}{{APP}} runs on this Mac. It gives local dev servers names instead of
ports. macOS sends every `*.localhost` name to this Mac: no DNS, no `/etc/hosts`.
The {{APP}} menu bar app installed this note.

- HTTP: `https://shop.localhost{{HTTPS}}` goes to `http://127.0.0.1:5173`
- By path: `https://shop.localhost{{HTTPS}}/blog` goes to another dev server
- TCP: `db.shop.localhost:15432` goes to `127.0.0.1:5432`
- Folder: `https://coverage.shop.localhost{{HTTPS}}` serves a folder, no server

## Read the guide first

Before you add the first route in a project, read the full guide. It has the
current status and routes, and covers names, path and TCP routes, worktrees,
HTTPS trust, the proxy, scripts and what to write in the project:
`{{CLI}} guide`. No CLI: `curl -s {{DEFAULT_HELP_URL}}`. Neither works: ask the
user to choose **Install Command Line Tool…** in the {{APP}} menu.

Use the MCP tools (`register_route`, `list_routes`, `find_free_port`, …) or the
CLI: `{{CLI}} list`, `{{CLI}} add <host> <port> [--path /blog]`,
`{{CLI}} add <host> --folder <dir>`, `{{CLI}} which <url>`.

## When to use it

- You start a dev server or give the user a URL: give a name, not a port.
- Main server: the project folder name (`shop`). Others go under it: `api.shop`.
- Branch or worktree: `feat-login.shop`, with `owner_pid` of its dev server, so
  the route goes away when the server stops.
- Several apps on one name by path: `{{CLI}} add shop 3001 --path /blog`. The
  app must live under that path (Next.js `basePath`, Vite `base`), or use
  `--strip-path`. Ask the user before you change `basePath`, `base` or
  `assetPrefix`: they change the production build too.
- HTML or a report the user should open: serve its folder (`--folder`, or MCP
  `folder` with an absolute path).
- The project's `AGENTS.md`, `CLAUDE.md` or README has a "Local URLs (LocalRouter)"
  section: use those names.
- Run `{{CLI}} list` first: adding a host and path that exist replaces that route.
- Do not make a project need LocalRouter: other people may not have it.

## Proxy

Off by default. When on, `127.0.0.1:{{PROXY_PORT}}` is the HTTP proxy of Chrome
or a program. Every request it carries goes to HAR files in
`{{PROXY_LOG_FOLDER}}`, with cookies and API keys as they are; the user sees
them at {{PROXY_LOG_URL}}.

- MCP `get_proxy` or `{{CLI}} proxy` gives the URL, the environment variables
  and the Chrome flags. Run a command through it:
  `eval "$({{CLI}} proxy env)" && npm test`.
- HTTPS is read only after the user trusts the inspection CA (`{{CLI}} proxy trust`).
- You cannot change the proxy of a program that is already running, and that
  includes yourself.
- A phone cannot use `127.0.0.1`: the user sets it up in the app (Proxy tab,
  Set up iPhone). You cannot do it for them.
- Never write proxy settings into project files (`.env`,
  `.claude/settings.json`, test configs).
- A HAR file can be 100 MB: do not read a whole file, use `jq` on the newest
  file in `{{CLI}} proxy log path`.
- `{{CLI}} proxy env` also sets `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` and
  `CURL_CA_BUNDLE`, so Python and `curl` trust inspected hosts.

## Scripts

A script rule runs Lua on the traffic of a route or a proxied host. An
intercept script may change or answer requests; a log script writes a copy of
each exchange to its `output_dir`. Read `{{CLI}} rules api` (or
`curl -s {{DEFAULT_HELP_URL}}/scripts`) first, test with `check_only`, and give
the rule `owner_pid` of your process. Use `on_event` for streams;
`response_body` in an intercept rule stops streaming. Scripts see every header
as it is. Do not commit captures.
