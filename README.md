# LocalRouter

A small macOS menu bar app that gives each local project a name instead of a port.

```
before:  http://localhost:5173        http://localhost:3001       http://localhost:8080
after:   https://shop.localhost       https://api.shop.localhost  https://feat-login.shop.localhost
```

LocalRouter is local DNS plus a reverse proxy, with an MCP interface. MCP (Model
Context Protocol) is the protocol that coding agents such as Claude Code use to
call tools. So when an agent starts a new project, it can register a domain for
that project by itself, and write a note that says what the domain is for.

> **Status: design stage.** Nothing is built yet. This README describes the
> planned behavior. Commands and tool names can still change.

## What it does

1. **Names instead of ports.** Map `shop.localhost` to `127.0.0.1:5173`.
2. **Subdomains for branches and worktrees.** Map `feat-login.shop.localhost` to
   the dev server of the `feat-login` worktree, next to the main one.
3. **HTTPS.** Every name also works over `https://`, with a certificate from a
   local certificate authority that the app creates on first run.
4. **Three protocols.** HTTP and HTTPS routes share ports 80 and 443 and are
   chosen by name. TCP routes (Postgres, Redis, any TCP service) each get their
   own loopback port, for example `db.shop.localhost:15432`.
5. **MCP server.** Agents can register, list and remove routes, find a free port,
   and read request logs.
6. **Menu bar only.** No Dock icon and no main window. The icon sits next to the
   clock. The menu has four sections: Domains, Logs, Settings, Help.

## How it works

```
  Browser / curl / agent
        │  https://feat-login.shop.localhost
        ▼
  ┌──────────────────────┐   1. name lookup
  │ macOS resolver       │──────────────────►  *.localhost = 127.0.0.1
  └──────────────────────┘                     (built into macOS, no DNS server)
        │  connect 127.0.0.1:443
        ▼
  ┌──────────────────────────────────────────────┐
  │ localrouterd (background process)            │
  │  2. TLS: pick certificate for the SNI name   │
  │  3. read Host header: feat-login.shop        │
  │  4. look up route table                      │
  │  5. forward request (HTTP/1.1, HTTP/2, WS)   │
  └──────────────────────────────────────────────┘
        │  http://127.0.0.1:5174
        ▼
  Dev server of the feat-login worktree (Vite, Next.js, Rails, ...)
```

WebSocket upgrade is forwarded too, so hot module reload in Vite and Next.js keeps
working.

## Domain names: why `.localhost` and not `.local`

The top-level domain (TLD) is the last part of the name. The choice of TLD
decides whether LocalRouter must run its own DNS server.

| TLD | How the name resolves on macOS | Setup | Default |
|---|---|---|---|
| `.localhost` | macOS resolves `*.localhost` and `*.*.localhost` to `127.0.0.1` itself. Browsers also treat it as a secure context. | None | ✓ |
| `.test` | Reserved for testing (RFC 2606). Needs a file in `/etc/resolver/test` that points to a small DNS server inside LocalRouter. | Admin password once | optional |
| `.local` | Reserved for Bonjour / mDNS (RFC 6762). macOS sends these names to multicast DNS, not to normal DNS. | Not supported | ✗ |

Checked on macOS 27.0 with `socket.getaddrinfo`:

- `feat.projectx.localhost` resolves to `127.0.0.1` at once.
- `projectx.local` fails after **5.0 seconds** (the mDNS timeout). A browser
  would wait this long on each new connection.

The TLD is a setting. The default is `.localhost`.

## Routes

A route maps one host name to one target.

| Field | Example | Meaning |
|---|---|---|
| `host` | `feat-login.shop` | Name without the TLD. |
| `protocol` | `http` or `tcp` | Default `http`. |
| `target` | `http://127.0.0.1:5174`, `https://127.0.0.1:5174`, `tcp://127.0.0.1:55001` | Where to forward. Always a loopback address. |
| `listen_port` | `15432` | TCP routes only: the port clients connect to. `0` picks a free one. |
| `https_only` | `true` | HTTP routes only: redirect `http://` to `https://`. |
| `note` | `Login redesign, worktree ../shop-feat-login` | Free text. Agents write why the route exists. |
| `owner_pid` | `48121` | Optional. The route is removed when this process exits. |
| `persistent` | `true` | Keep the route after a restart. |

**Subdomain fallback.** If `feat-x.shop` has no route of its own, the request
goes to the `shop` route. You can turn this off in Settings. This applies to HTTP
routes only.

**Why TCP routes need their own port.** Plain TCP carries no host name, and
every `*.localhost` name resolves to `127.0.0.1`. So the port is the only thing
that tells two TCP routes apart. The name is for people: it is shown in the
app and the logs. Details: [ADR 01, change 7](docs/adr/01-project-setup-2026-09-26/07-protocols.md).

## MCP interface

Add LocalRouter to Claude Code:

```
claude mcp add localrouter -- localrouter mcp
```

`localrouter mcp` is a small stdio process. It forwards each tool call to the
running daemon over a Unix socket.

| Tool | What it does |
|---|---|
| `register_route` | Create or update a route (`host`, `target`, `note`, `owner_pid`, `persistent`). Returns the full URLs. |
| `unregister_route` | Remove a route by host. |
| `list_routes` | All routes, with an "upstream is up / down" flag for each. |
| `find_free_port` | Return a free local port. Useful when many worktrees run at once. |
| `get_logs` | Last N requests, optionally for one host: time, method, path, status, duration. |
| `status` | Daemon version, TLD, HTTPS state, CA trust state. |

Example of what an agent does when it starts a worktree:

1. Call `find_free_port` and get `5174`.
2. Start the dev server on port `5174`.
3. Call `register_route` with `host = "feat-login.shop"`, `target = "http://127.0.0.1:5174"`, `note = "worktree for branch feat/login"`.
4. Tell the user: open `https://feat-login.shop.localhost`.

## Command line

```
localrouter add shop 5173 --note "main dev server"
localrouter add feat-login.shop 5174
localrouter add api.shop --target https://127.0.0.1:8443 --https-only
localrouter add db.shop 55001 --tcp --listen 15432
localrouter list
localrouter rm feat-login.shop
localrouter logs shop
localrouter status
localrouter trust          # install the local CA into the keychain
localrouter untrust        # remove it again
localrouter ca-path        # print the path of ca.pem
localrouter ca reset       # make a new CA (breaks the old trust)
localrouter mcp            # MCP server over stdio
```

## HTTPS

1. On first run, LocalRouter creates a root certificate authority (CA) in
   `~/Library/Application Support/LocalRouter/ca/`. The private key never leaves
   this folder.
2. `localrouter trust` (or a button in Settings) adds the CA to the login
   keychain. macOS asks for your password once.
3. When a TLS connection arrives, the daemon reads the requested name (SNI) and
   signs a certificate for it. Certificates are cached.

Some clients do not use the macOS keychain:

| Client | What to do |
|---|---|
| Firefox | Set `security.enterprise_roots.enabled` to `true`. |
| Node.js | `export NODE_EXTRA_CA_CERTS="$(localrouter ca-path)"` |
| Python `requests` | `export REQUESTS_CA_BUNDLE="$(localrouter ca-path)"` |

## Ports 80 and 443

Checked on macOS 27.0 without root:

| Bind address | Port 80 |
|---|---|
| `0.0.0.0` | ✓ allowed |
| `127.0.0.1` | ✗ permission denied |

So the daemon binds `0.0.0.0:80` and `0.0.0.0:443` and **closes every
connection that does not come from a loopback address**. Your dev servers are not
visible on the local network. The setting "Allow LAN access" turns this check
off. If the macOS firewall is on, it can ask once whether to accept incoming
connections.

## Tech stack

Goal: very small memory use, native look, no web view.

| Part | Language | Main libraries |
|---|---|---|
| `localrouterd`: proxy, TLS, route table, logs, optional DNS | Rust | `tokio`, `hyper`, `rustls`, `rcgen`, `hickory-server` (only for `.test`) |
| `localrouter`: CLI and MCP stdio server | Rust | `clap`, `rmcp` (official Rust MCP SDK) |
| Menu bar app | Swift | SwiftUI `MenuBarExtra`, `SMAppService` to start the daemon at login |
| Link between app and daemon | JSON over a Unix socket | |

Why this split:

1. The hard parts are network code: proxy, TLS, WebSocket, DNS. Rust has mature
   libraries for each of them.
2. The daemon runs without the UI. Routes keep working when the menu bar app is
   closed. The daemon can be tested on its own.
3. SwiftUI gives a real native menu bar item with little code and little memory.
   A Tauri or Electron UI would start a web view process for a few menus.

## Planned layout

```
localrouter/
├── apps/
│   ├── daemon/      Rust, localrouterd: listeners, proxy, Unix socket API
│   ├── cli/         Rust, localrouter: commands and the MCP stdio server
│   └── menubar/     Swift, LocalRouter.app (Xcode project)
├── libs/
│   └── core/        Rust library: route table, proxy, TLS, CA, logs
├── api/examples/    JSON examples shared by the Rust and Swift tests
└── docs/
```

The full tree is in [ADR 01, components](docs/adr/01-project-setup-2026-09-26/08-components.md#project-structure-on-disk).

## Menu bar UI

| Section | Contents |
|---|---|
| Domains | List of routes with status dot (upstream up / down), note, "open in browser", "copy URL", remove. |
| Logs | Live request log, filter by host. |
| Settings | TLD, HTTPS on/off, trust CA, subdomain fallback, allow LAN access, start at login. |
| Help | How to add the MCP server to Claude Code, how to trust the CA in Firefox and Node. |

## License

Not chosen yet.
