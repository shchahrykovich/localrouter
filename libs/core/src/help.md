# LocalRouter

LocalRouter {{VERSION}} runs on this Mac. It gives local dev servers names
instead of ports:

- HTTP: `https://shop.localhost` goes to `http://127.0.0.1:5173`
- TCP: `db.shop.localhost:15432` goes to `127.0.0.1:5432` (databases, caches)

macOS sends every `*.localhost` name to this Mac by itself. There is no DNS
setup and no `/etc/hosts` change.

This page is for a coding agent. If the user sent you here, add LocalRouter to
the project in the current folder: follow the steps below, then tell the user
the new URLs.

Read this page again at any time: `curl -s http://router.localhost`

## Status

{{STATUS}}

If a part is off, "no" or broken, see "Turn on HTTP, HTTPS and trust" below.

## Routes on this Mac now

{{ROUTES}}

## Step 1: find a way to talk to LocalRouter

Use the first one that works:

1. MCP tools `register_route`, `list_routes` and the others below. If you have
   them, use them.
2. The command line tool: run `localrouter status`. It is usually
   `~/.local/bin/localrouter`.
3. Neither works: ask the user to open the LocalRouter menu bar app and choose
   **⋯ > Install Command Line Tool…**. Then Claude Code gets the MCP tools with
   `claude mcp add localrouter -- ~/.local/bin/localrouter mcp`

| MCP tool | Command line | What it does |
|---|---|---|
| `register_route` | `localrouter add <host> <port>` | Create or replace a route |
| `unregister_route` | `localrouter rm <host>` | Remove a route |
| `list_routes` | `localrouter list` | All routes, their URLs, and whether each target is up |
| `find_free_port` | - | A free TCP port on 127.0.0.1 |
| `get_logs` | `localrouter logs [host]` | Recent requests and connections |
| `status` | `localrouter status` | Daemon version, ports, local CA |

## Step 2: choose names

- A host is the name without `.localhost`: `shop`, `api.shop`, `feat-login.shop`.
- A label (the part between dots) uses `a-z`, `0-9` and `-`, 1 to 63
  characters. Turn `feat/login` into `feat-login`.
- Use the project folder name for the main server: `shop`.
- Put the other servers of the project under it: `api.shop`, `db.shop`.
- For a git branch or worktree, put the branch first: `feat-login.shop`.
- A name without its own route uses its parent: `other.shop` goes to `shop`.
- `router` is taken by this page.
- Run `localrouter list` first. Adding a host that exists replaces it.

## Step 3: find the dev servers

Look for the ports the project uses:

- `package.json` scripts and framework config (Vite 5173, Next.js 3000)
- `docker-compose.yml` or `compose.yaml`, the `ports:` lists
- `Procfile`, `Makefile`, `.env` files, `README`

The target must be on this Mac: `127.0.0.1`, `localhost` or `[::1]`.

## Step 4: add routes

HTTP server on port 5173:

```
localrouter add shop 5173 --note "main dev server"
```

With MCP: `register_route` with `host: "shop"`, `port: 5173`,
`note: "main dev server"`, `persistent: true`.

Other cases:

- The server speaks HTTPS itself: `--target https://127.0.0.1:8443`
- Always send the browser to HTTPS: add `--https-only`
- A database or another TCP server on port 5432:
  `localrouter add db.shop 5432 --tcp --listen 15432`.
  Clients connect to `db.shop.localhost:15432`. A TCP route is chosen by its
  listen port only; the name is for people.

How long a route lives:

| Kind | Command line | MCP | Ends when |
|---|---|---|---|
| persistent | default | `persistent: true` | You remove it |
| session | `--session` | default | LocalRouter restarts |
| owned | `--owner-pid <pid>` | `owner_pid: <pid>` | That process exits |

## Step 5: a server for a branch or worktree

1. Get a free port: `find_free_port`.
2. Start the dev server on that port.
3. Register `<branch>.<project>` with `owner_pid` set to the dev server's pid.
   The route goes away when the server stops.
4. Give the user `https://<branch>.<project>.localhost`.

## Step 6: check that it works

```
curl -s -o /dev/null -w "%{http_code}\n" http://shop.localhost/
```

- `502` and a LocalRouter page: the dev server does not run, or it listens on
  another port.
- `403`, "Invalid Host header" or "Blocked request": the dev server checks the
  `Host` header. Allow `.localhost`: Vite `server.allowedHosts`,
  webpack-dev-server `allowedHosts`, Django `ALLOWED_HOSTS`, Rails
  `config.hosts`.
- HTTPS errors: see "Turn on HTTP, HTTPS and trust" below.
- `localrouter logs shop` shows what reached LocalRouter.

## Turn on HTTP, HTTPS and trust

The user does this in the LocalRouter menu bar app: click its icon in the menu
bar, then open the **Settings** tab. Ask the user first; do not change their
keychain on your own.

- **CA trusted by macOS: no.** Browsers warn on `https://` URLs. In
  **Settings > HTTPS certificate authority**, click **Trust…**. macOS asks for
  the user's password once. From a terminal: `localrouter trust`
- **HTTP or HTTPS: off.** Another program uses port 80 or 443.
  **Settings > Daemon** shows "not listening" and the error in red. Find the
  program with `sudo lsof -nP -iTCP:443 -sTCP:LISTEN` (or `:80`), stop it, then
  restart the daemon:
  `launchctl kickstart -k gui/$(id -u)/dev.localrouter.app.daemon`
- **Local CA: broken.** HTTPS is off. `localrouter ca reset --yes` makes a new
  CA; after that, trust it again. The old CA stops working.
- **Node.js, Python and other tools** do not read the macOS keychain. Point
  them to the CA file:
  `export NODE_EXTRA_CA_CERTS="$(localrouter ca-path)"` or
  `export REQUESTS_CA_BUNDLE="$(localrouter ca-path)"`
- **Firefox:** open `about:config` and set
  `security.enterprise_roots.enabled` to `true`.

## Step 7: write it down in the project

Add a short section to the project's agent instructions (`CLAUDE.md` or
`AGENTS.md`) or to the README. Use the real names and ports:

```
## Local URLs (LocalRouter)

- https://shop.localhost - dev server, port 5173
- db.shop.localhost:15432 - Postgres, port 5432

Set up once: localrouter add shop 5173 && localrouter add db.shop 5432 --tcp --listen 15432
Branch or worktree: register <branch>.shop with owner_pid of the dev server.
Instructions: curl -s http://router.localhost
```

Do not make the project need LocalRouter. If you add `localrouter add` to a dev
script, skip it when the tool is missing:
`command -v localrouter >/dev/null && localrouter add shop 5173 --session`
