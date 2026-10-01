# {{APP}}

{{INSTANCE_NOTE}}{{APP}} runs on this Mac. It gives local dev servers names instead of
ports:

- HTTP: `https://shop.localhost{{HTTPS}}` goes to `http://127.0.0.1:5173`
- HTTP by path: `https://shop.localhost{{HTTPS}}/blog` goes to one dev server, and the
  rest of `shop.localhost` to another
- TCP: `db.shop.localhost:15432` goes to `127.0.0.1:5432` (databases, caches)
- Folder: `https://coverage.shop.localhost{{HTTPS}}` serves the files of a folder,
  with no dev server

macOS sends every `*.localhost` name to this Mac by itself. There is no DNS
setup and no `/etc/hosts` change.

You are reading this because the user installed agent instructions from
the {{APP}} menu bar app (Claude Code or Codex).

## When to use it

- You start a dev server, or the user asks for a URL: give a name, not a port.
- You work in a git branch or worktree: give its server its own name, so it
  does not take the main one's port or URL.
- The project is one site made of several apps split by path (`/blog`,
  `/admin`): one name, one route per path.
- You wrote HTML, built a site or made a report, and the user should open it:
  serve its folder (`--folder`, or MCP `folder` with an absolute path). No
  server is needed.
- The project's `AGENTS.md`, `CLAUDE.md` or README has a "Local URLs (LocalRouter)" section:
  use the names listed there.

Do not make a project need LocalRouter. Other people on the project may not
have it.

## How to talk to it

1. MCP tools `register_route`, `list_routes`, `find_free_port` and the others,
   if you have them.
2. The command line tool: `{{CLI}} status`, `{{CLI}} list`,
   `{{CLI}} add <host> <port> [--path /blog]`,
   `{{CLI}} add <host> --folder <dir>`, `{{CLI}} which <url>`.
   It is usually `~/.local/bin/{{CLI}}`.
3. Neither works: ask the user to choose **Install Command Line Tool…** in the
   {{APP}} menu.

## Full instructions

The command line tool prints the full guide, with the current ports, status
and routes:

```
{{CLI}} guide
```

If `{{CLI}}` is not found, run `curl -s {{DEFAULT_HELP_URL}}`. If that fails
too, ask the user to choose **Install Command Line Tool…** in the {{APP}} menu.

Read it before you add the first route in a project. It covers names, path
routes, TCP routes, branches and worktrees, HTTPS trust, and what to write in
the project.

## Names in short

- A host is the name without `.localhost`: `shop`, `api.shop`, `feat-login.shop`.
- Main server: the project folder name. Other servers go under it: `api.shop`.
- Branch or worktree: the branch first, `feat-login.shop`, registered with
  `owner_pid` of the dev server so the route goes away when the server stops.
- Path routes: `{{CLI}} add shop 3001 --path /blog`. The app must live
  under that path (Next.js `basePath`, Vite `base`), or use `--strip-path` for
  a server that answers at `/`.
- Ask the user before you change `basePath`, `base` or `assetPrefix`: they
  change the production build too.
- Run `{{CLI}} list` first. Adding a host and path that exist replaces that
  route.

## Proxy

{{APP}} can be the HTTP proxy of Chrome or a program, on
`127.0.0.1:{{PROXY_PORT}}`, so the user sees which servers it calls. Off by
default.

- `{{CLI}} proxy on`, then MCP `get_proxy` or `{{CLI}} proxy` gives the URL,
  the environment variables and the Chrome flags.
- Start a command through it: `eval "$({{CLI}} proxy env)" && npm test`.
  Chrome: `{{CLI}} proxy chrome`.
- To read the HTTPS requests of a host: `{{CLI}} proxy inspect add <host>`.
  The user must trust the inspection CA (`{{CLI}} proxy trust`; macOS asks for
  their password).
- You cannot change the proxy of a program that is already running, and that
  includes yourself: a program reads the proxy settings when it starts.
- Never write proxy settings into project files (`.env`,
  `.claude/settings.json`, test configs): other people on the project may not
  have {{APP}}.

## Scripts

A script rule runs a Lua file on the HTTP traffic of a route or of a host
the proxy carries. An **intercept script** may change or answer requests
while the client waits; a **log script** gets a copy of each finished
exchange and writes files in its `output_dir`.

- Read the reference first: `{{CLI}} rules api`, or
  `curl -s {{DEFAULT_HELP_URL}}/scripts`.
- Test with `check_only` (`{{CLI}} rules check <file>`), then set the rule
  before the job starts: MCP `set_script_rule`, or `{{CLI}} rules add`.
- Give it `owner_pid` of a process you started, or remove it when done
  (`remove_script_rule`). `persistent` only when the user asks.
- A log rule copies text and events by default; use `on_event` for streams
  and `save` for images, video, downloads and uploads. An intercept rule with
  `response_body` stops streaming for that body.
- Secret headers reach scripts as `[redacted]`; only the user can reveal them.
- Do not commit captures: keep `output_dir` out of git. They hold prompts and
  API replies.
