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

> **Status: version 0.1, Apple Silicon Macs, macOS 14 or later.** The design is
> in [ADR 01](docs/adr/01-project-setup-2026-09-26/README.md); distribution and
> self-update are in [ADR 02](docs/adr/02-distribution-and-self-update-2026-09-26/README.md).

## Install

1. Download `localrouter-<version>.dmg` from
   [Releases](https://github.com/shchahrykovich/localrouter/releases/latest).
2. Open it and drag **LocalRouter** to **Applications**. Start it from there.
3. The icon appears next to the clock. macOS may say that a background item was
   added: that is the daemon. If the menu says "not running", allow LocalRouter
   in **System Settings → General → Login Items**.
4. **Settings → Trust…** (the gear at the top right) trusts the local CA (macOS asks for your password), so
   `https://` names show no warning.
5. **⋯ → Install Command Line Tool…** links `~/.local/bin/localrouter`. Then give
   your coding agent access:

   ```
   claude mcp add localrouter -- ~/.local/bin/localrouter mcp
   ```

6. **⋯ → Install Claude Code Instructions…** links `~/.claude/LocalRouter.md`
   to a note in the app and adds `@LocalRouter.md` at the top of
   `~/.claude/CLAUDE.md`. Every new Claude Code session then knows that
   LocalRouter is here and how to use it.
7. **⋯ → Install Codex Instructions…** links `~/.codex/LocalRouter.md` to
   the same note and adds an instruction to read it to `~/.codex/AGENTS.md`
   (or a non-empty `AGENTS.override.md`). Existing instructions are preserved;
   repeated installs do not add duplicates. Start a new Codex session afterward.
   If `CODEX_HOME` is set in the app's environment, that directory is used.
   Run Codex once before installing so its home directory exists.

The app updates itself from GitHub Releases: it checks 30 seconds after start
and then every 6 hours, and **⋯ → Check for Updates…** checks at once. The app
must be in `/Applications` or `~/Applications` to replace itself.

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
   clock. The menu has three tabs: Router, Logs, Proxy. The gear at the top
   right, or **Settings…** in the right-click menu, opens the Settings window,
   which also holds Help.

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

Version 1 supports `.localhost` only.

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

Add LocalRouter to Claude Code (after **Install Command Line Tool**):

```
claude mcp add localrouter -- ~/.local/bin/localrouter mcp
```

`localrouter mcp` is a small stdio process. It forwards each tool call to the
running daemon over a Unix socket.

| Tool | What it does |
|---|---|
| `register_route` | Create or update a route (`host`, `protocol`, `target` or `port`, `listen_port`, `https_only`, `note`, `owner_pid`, `persistent`). Returns the URLs. |
| `unregister_route` | Remove a route by host. |
| `list_routes` | All routes, with an "upstream is up / down" flag for each. |
| `find_free_port` | Return a free local port. Useful when many worktrees run at once. |
| `get_logs` | Last N requests and TCP connections, optionally for one host. Paths never include the query string. |
| `status` | Daemon version, ports bound or failed, CA state and trust. |
| `get_proxy` | The forward proxy: URL, environment variables, Chrome flags, inspected hosts, inspection CA trust, and the script rules with their counters. Read only. |
| `set_script_rule` | Run a Lua script on a host's HTTP traffic (`id`, `host`, `path`, `methods`, `script`, `output_dir`, `owner_pid`, `persistent`, `check_only`, ...). |
| `remove_script_rule` | Remove a script rule by id. Capture files stay. |

To set up a project, tell the agent in the project folder:

```
Run curl -s http://router.localhost and follow it to add LocalRouter to this project.
```

`router.localhost` is a page the daemon serves itself. It is Markdown for
agents: setup steps, the status of HTTP, HTTPS and the local CA, how to turn
each on in the app, and the current routes. The host key `router` is reserved
for it. The app copies this prompt from the Router tab and the Help page of
Settings, and from the menu that opens on a right-click of the menu bar icon.

Example of what an agent does when it starts a worktree:

1. Call `find_free_port` and get `5174`.
2. Start the dev server on port `5174`.
3. Call `register_route` with `host = "feat-login.shop"`, `target = "http://127.0.0.1:5174"`, `note = "worktree for branch feat/login"`.
4. Tell the user: open `https://feat-login.shop.localhost`.

## Command line

```
localrouter add shop 5173 --note "main dev server"   # saved; --session to not save
localrouter add feat-login.shop 5174 --owner-pid 4242  # removed when process 4242 exits
localrouter add api.shop --target https://127.0.0.1:8443 --https-only
localrouter add db.shop 55001 --tcp --listen 15432
localrouter list
localrouter rm feat-login.shop
localrouter logs shop -f   # follow
localrouter status
localrouter trust          # install the local CA into the keychain
localrouter untrust        # remove it again
localrouter ca-path        # print the path of ca.pem
localrouter ca reset --yes # make a new CA (breaks the old trust)
localrouter mcp            # MCP server over stdio
```

## Forward proxy

LocalRouter can also be the HTTP proxy of Chrome or of a program, so you see
which servers it calls ([ADR 06](docs/adr/06-forward-proxy-2026-10-01/README.md)).
It is off by default and listens on `127.0.0.1:8877` only.

```
localrouter proxy on                          # bind 127.0.0.1:8877 and [::1]:8877
eval "$(localrouter proxy env)" && npm test   # one command through the proxy
localrouter proxy chrome                      # a separate Chrome window that uses it
localrouter logs                              # requests marked "via proxy"
localrouter proxy trust                       # trust the inspection CA (password)
localrouter proxy inspect rm '*'              # stop reading every host's HTTPS requests
localrouter proxy inspect add api.example.com # read only this host's HTTPS requests
localrouter proxy off
```

The inspect list is `*` by default: LocalRouter reads the HTTPS requests of
every host outside `.localhost`, with a second CA, the inspection CA, which you
trust separately. A host that is not in the list is a tunnel: LocalRouter sees
only its name. The real server's certificate is always checked with the macOS
trust store. The menu bar icon's right-click menu has **Open Chrome via
Proxy** when Google Chrome is installed.

### Proxy log

Every request the proxy carries is written to HAR files, the format Chrome
DevTools imports ([ADR 08](docs/adr/08-proxy-har-log-2026-10-02/README.md)). The
log is on by default and writes only while the proxy is on.

```
localrouter proxy log                          # state: folder, viewer, limits, errors
localrouter proxy log off                      # or on
localrouter proxy log limits --mb 20 --requests 5000
localrouter proxy log open                     # the viewer: http://proxy.localhost
ls -t "$(localrouter proxy log path)"          # ~/Library/Logs/LocalRouter/proxy
```

- A new file starts at 20 MB or 5000 requests, whichever comes first; the 5
  newest files are kept.
- Headers are written as they are, cookies and API keys too. URLs are written
  as they are, query included. Bodies are not written.
- The viewer at `http://proxy.localhost` (also `router.localhost/proxy-log/`)
  lists the files, shows the requests live, and downloads a file for
  DevTools → Network → Import HAR file. It answers this Mac only and is read
  only.
- `proxy env` also sets `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` and
  `CURL_CA_BUNDLE` to a bundle of the macOS roots plus the inspection CA, so
  Python and `curl` work with inspected hosts.

## Scripts

A script rule runs a Lua 5.4 file on the HTTP traffic of a route or of a host
the proxy carries ([ADR 07](docs/adr/07-proxy-scripts-lua-2026-10-01/README.md)).
An **intercept** script runs while the client waits and may change or answer a
request, change a response, or change each event of a stream. A **log**
script gets a copy of each finished request and response and writes files in
its `output_dir`; it never slows or changes traffic.

```
localrouter rules api                           # the reference: Lua API, body classes, limits
localrouter rules check ./capture.lua           # test a script, store nothing
localrouter rules add claude --host api.anthropic.com --script ./capture.lua --output-dir ./captures
localrouter rules                               # rules with matched, errors, last error
localrouter rules disable claude                # or enable, rm
```

Scripts run in a sandbox (no files, no network, no `os.execute`) with 50 ms
per intercept call, 2 s per log call and 64 MB per Lua state. A rule that
fails 20 times in a row is turned off. Scripts see every header as it is,
cookies and API keys too. The
reference is also at `http://router.localhost/scripts`. Keep `output_dir` out
of git: captures hold prompts and API replies.

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

So the daemon binds `0.0.0.0` and `[::]` on ports 80 and 443 and **closes every
connection that does not come from a loopback address**. Your dev servers are not
visible on the local network. The setting "Allow LAN access" lets other
machines in, but only on the networks you allowed (ADR 08): a network is
known by its router's MAC address, so LAN access at home does not open your
dev servers in a café. `localrouter lan` shows this network and the list;
`localrouter lan allow --name Home` adds it. If the macOS firewall is on, it
can ask once whether to accept incoming connections.

## Tech stack

Goal: very small memory use, native look, no web view.

| Part | Language | Main libraries |
|---|---|---|
| `localrouterd`: proxy, TLS, route table, logs, scripts | Rust | `tokio`, `hyper`, `rustls`, `rcgen`, `mlua` (Lua 5.4, built in) |
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

## Layout

```
localrouter/
├── apps/
│   ├── daemon/      Rust, localrouterd: listeners, Unix socket API, route store
│   ├── cli/         Rust, localrouter: commands and the MCP stdio server
│   └── menubar/     Swift package, LocalRouter.app: menu bar UI and updater
├── libs/
│   └── core/        Rust library: routes, proxy, TCP copy, TLS, CA, logs, API types
├── api/examples/    JSON examples shared by the Rust and Swift tests
├── scripts/         build, release, notarize, publish, install
└── docs/            ADRs and the dictionary
```

Inside `LocalRouter.app`: `Contents/MacOS/LocalRouter` (Swift),
`Contents/MacOS/localrouterd` (daemon, started by the LaunchAgent in
`Contents/Library/LaunchAgents`) and `Contents/Helpers/localrouter` (CLI).

The full tree is in [ADR 01, components](docs/adr/01-project-setup-2026-09-26/08-components.md#project-structure-on-disk).

## Menu bar UI

| Section | Contents |
|---|---|
| Router (tab) | List of routes with status dot (upstream up / down), note, "open in browser", "copy URL", remove. |
| Logs (tab) | Live request log, filter by host. |
| Proxy (tab) | Forward proxy on or off, its address, the command to start a program through it, Open Chrome via Proxy, the log line ("Log: on, 912 requests in …"), Open Proxy Log, Show Log Folder, the requests it carried. |
| Settings (window: gear button, or **Settings…** in the right-click menu) | A tree of pages with a search field, as in JetBrains IDEs: General (open at login, uninstall), Routing (subdomain fallback, LAN access and its networks), HTTPS Certificates, Proxy (proxy log; Inspection, Scripts), Daemon, Updates, Help. |
| Right-click menu | Open LocalRouter, Open Chrome via Proxy, Open Proxy Log, Show Proxy Log Folder, agent instructions and installers, Settings, Quit. |
| Help (Settings page, or **Help** in the right-click menu) | How to add the MCP server to a coding agent, how to trust the CA in Firefox and Node. **Help with Claude** and **Help with Codex** (only when the app is installed) open a new agent session with a prompt that reads the help page first. |

## Development

```
cargo test --workspace
swift test --package-path apps/menubar
scripts/install.sh --user --launch
```

`scripts/install.sh` builds an ad-hoc signed app into `~/Applications`. Such a
build does not update itself (the updater only accepts notarized images from
GitHub). Tests never touch `~/Library`: they set `LOCALROUTER_HOME` to a temp
folder and use random ports.

## Releasing

Same approach as VibeViewer. One-time setup: a "Developer ID Application"
certificate in the keychain, and notary credentials in `.env.notarize` (see
`scripts/notarize.env.example`; the file is ignored by git).

```
scripts/publish.sh            # bump patch, build, sign, notarize, release on GitHub
scripts/publish.sh --minor    # or --major, --set X.Y.Z, --no-bump
```

`publish.sh` runs `release.sh --notarize` (version bump in `Cargo.toml`,
Developer ID signing with the hardened runtime, DMG, `notarytool`, staple),
then `gh release create vX.Y.Z` on `shchahrykovich/localrouter` and checks the
uploaded size. Commit the version bump afterwards.

## License

MIT. See [LICENSE](LICENSE).
