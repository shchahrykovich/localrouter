# LocalRouter

LocalRouter runs on this Mac. It gives local dev servers names instead of
ports:

- HTTP: `https://shop.localhost` goes to `http://127.0.0.1:5173`
- TCP: `db.shop.localhost:15432` goes to `127.0.0.1:5432` (databases, caches)

macOS sends every `*.localhost` name to this Mac by itself. There is no DNS
setup and no `/etc/hosts` change.

You are reading this because the user chose **Install Claude Code
Instructions…** in the LocalRouter menu bar app.

## When to use it

- You start a dev server, or the user asks for a URL: give a name, not a port.
- You work in a git branch or worktree: give its server its own name, so it
  does not take the main one's port or URL.
- The project's `CLAUDE.md` or README has a "Local URLs (LocalRouter)" section:
  use the names listed there.

Do not make a project need LocalRouter. Other people on the project may not
have it.

## How to talk to it

1. MCP tools `register_route`, `list_routes`, `find_free_port` and the others,
   if you have them.
2. The command line tool: `localrouter status`, `localrouter list`,
   `localrouter add <host> <port>`. It is usually `~/.local/bin/localrouter`.
3. Neither works: ask the user to choose **Install Command Line Tool…** in the
   LocalRouter menu.

## Full instructions

The running app serves the full guide, with its current status and routes:

```
curl -s http://router.localhost
```

Read it before you add the first route in a project. It covers names, TCP
routes, branches and worktrees, HTTPS trust, and what to write in the project.

## Names in short

- A host is the name without `.localhost`: `shop`, `api.shop`, `feat-login.shop`.
- Main server: the project folder name. Other servers go under it: `api.shop`.
- Branch or worktree: the branch first, `feat-login.shop`, registered with
  `owner_pid` of the dev server so the route goes away when the server stops.
- Run `localrouter list` first. Adding a host that exists replaces it.
