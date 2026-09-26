# Dictionary

The words this project uses, and what each one means. Use these words in code,
docs, UI text, MCP tool descriptions and commit messages. If two words could
mean the same thing, this file says which one we use.

Status: the project is in the design stage. Field names and file paths below are
the planned ones from [ADR 01](adr/01-project-setup-2026-09-26/README.md). When
the code exists, each entry also points to the file that defines it.

## Words we use, and words we avoid

| Use | Do not use | Why |
|---|---|---|
| **route** | router, mapping, domain, binding | A route is one name-to-port entry. "Router" is the whole app. |
| **host** (of a route) | domain, subdomain, name | The route field is `host`. "Domain" is fine in UI text for users. |
| **target** | upstream, backend, port | The route field is `target`. "Upstream" is fine when talking about the proxy. |
| **daemon** | server, service, backend | "Server" is ambiguous: we also talk about dev servers. |
| **dev server** | app, upstream server | The user's own HTTP server, for example Vite on port 5173. |
| **client** (of the daemon) | frontend, UI | The CLI, the MCP shim and the app are all clients. |
| **local CA** | root cert, our certificate | The CA is the key pair plus its root certificate. |
| **listen port** / **target port** | port | Always say which one. A TCP route has both: the port clients connect to, and the port of the real server. |

## Domain entities

These are the things the system stores or passes around.

### Route

One entry in the route table: "requests for this host go to this target".
Every route has a protocol: `http` or `tcp`.
Defined in `libs/core/src/routes.rs` (planned). Decided in
[ADR 01, change 6](adr/01-project-setup-2026-09-26/06-route-model.md).

| Field | Type | Meaning |
|---|---|---|
| `host` | string | The host key, see below. Example: `feat-login.shop`. |
| `protocol` | `http` or `tcp` | Default `http`. See Route protocol. |
| `target` | URL | Where requests go, always a loopback address. `http://` or `https://` for HTTP routes, `tcp://` for TCP routes. Example: `http://127.0.0.1:5174`. |
| `listen_port` | integer | TCP routes only. The loopback port clients connect to. `0` means "pick a free one". |
| `https_only` | boolean, default `false` | HTTP routes only. Plain HTTP gets a `308` redirect to `https://`. |
| `note` | string, up to 500 characters | Free text. Agents write why the route exists. Example: "worktree for branch feat/login". |
| `owner_pid` | integer, optional | Process ID. The route is removed when this process exits. |
| `persistent` | boolean, default `false` | Save the route in `routes.json` so it survives a restart. |

A route cannot be both `persistent` and have an `owner_pid`.

### Route protocol

Decided in [ADR 01, change 7](adr/01-project-setup-2026-09-26/07-protocols.md).

| Protocol | Clients connect to | Route chosen by | Target schemes |
|---|---|---|---|
| **`http`** (HTTP route) | shared ports 80 (HTTP) and 443 (HTTPS) | the name: TLS SNI and `Host` header | `http://`, `https://` |
| **`tcp`** (TCP route) | the route's own `listen_port` on `127.0.0.1` and `::1` | the listen port only; the name is not checked | `tcp://` |

Plain TCP (Postgres, Redis) carries no host name, and every `*.localhost` name
resolves to `127.0.0.1`. So the port is the only thing that can tell two TCP
routes apart.

Example: `db.shop` with listen port 15432 and target `tcp://127.0.0.1:55001`.
Clients use `db.shop.localhost:15432`.

### Route kind

Every route has exactly one kind. The kind decides how long the route lives.

| Kind | How it is created | Saved to disk | Removed when |
|---|---|---|---|
| **persistent** | `persistent: true` | yes, in `routes.json` | someone calls `unregister_route` |
| **owned** | `owner_pid` is set | never | the owner process exits, or `unregister_route` |
| **session** | neither is set | no | the daemon stops, or `unregister_route` |

### Host key

The route's `host` field: the full name **without** `.localhost` and without a
port. It has one or more **labels** separated by dots.

| Full name in the browser | Host key |
|---|---|
| `shop.localhost` | `shop` |
| `feat-login.shop.localhost:443` | `feat-login.shop` |

### Label

One part of a host key between dots. Rules: `a-z`, `0-9` and `-`, 1 to 63
characters, no `-` at the start or end, always lower case. `feat/login` is not a
valid label. The daemon proposes `feat-login` instead.

### Project host and branch host

Not stored as separate types. They are names for a common pattern:

| Term | Example | Meaning |
|---|---|---|
| **project host** | `shop` | the main dev server of a project |
| **branch host** | `feat-login.shop` | a dev server for a branch or worktree, one label in front of the project host |

### Target

The URL a route forwards to: a scheme, then `127.0.0.1`, `localhost` or `[::1]`,
then a port. HTTP routes use `http://` or `https://` (the certificate of an
`https://` dev server is not checked). TCP routes use `tcp://`. Other addresses are refused, so the daemon can never
forward to another machine.

### Route table

All routes the daemon knows at the moment. It lives in the daemon's memory. At
start it is filled from `routes.json` (persistent routes only). It changes only
through the socket API.

### Request log entry

One line in the request log. There are two kinds.

**HTTP entry**, for HTTP routes. It is created after the response headers are sent.

| Field | Meaning |
|---|---|
| time | when the request arrived |
| method | `GET`, `POST`, ... |
| host | the full host name the client asked for |
| path | the path **without** the query string: `/cb?token=abc` is stored as `/cb` |
| status | HTTP status code |
| duration | time to the response headers |
| bytes | response size |

Headers and bodies are never stored.

**TCP entry**, for TCP routes. It is created when the connection closes: time,
route host, listen port, bytes in each direction, duration, and `failed` if the
target did not answer within 2 seconds. The bytes themselves are never stored.

### Request log

A **ring buffer** (a list with a fixed size, where a new entry pushes out the
oldest) of request log entries. Default size: 1,000. It lives in memory only.
Clients read it with `get_logs`, or follow it live with `subscribe_logs`.

### Config

The user's settings, stored in `config.json` and written only by the daemon.

| Setting | Default | Meaning |
|---|---|---|
| HTTP port | `80` | tests use `0`, which means "any free port" |
| HTTPS port | `443` | same |
| fallback | on | a host key with no route uses the route of its parent (see Fallback) |
| `allow_lan` | off | accept connections from other machines |
| log size | 1,000 | entries in the request log |

### Local CA

The **certificate authority** (CA) that this Mac's LocalRouter owns. A CA is a
key pair plus a certificate that says "I sign other certificates". It has two
files in `ca/`:

| File | Meaning |
|---|---|
| `ca.key` | The private key. Mode `0600`. Never leaves the folder. Anyone who has it can make certificates this user's browser trusts. |
| `ca.pem` | The root certificate (ECDSA P-256, 10 years). Safe to share with Firefox, Node, `curl`. |

The daemon creates the local CA on first start and never replaces it by itself.
Only `localrouter ca reset` makes a new one.

### Leaf certificate

A certificate for one exact name, for example `feat-login.shop.localhost`,
signed by the local CA. Made on demand when a TLS connection asks for that name,
valid for 90 days, kept in memory only. It is made only for names that end in
`.localhost` and have a route.

### Trust

The setting in the macOS **login keychain** that says "certificates signed by
`ca.pem` are valid". The user sets it with the Trust button or
`localrouter trust`, and removes it with `localrouter untrust`. macOS asks for
the password. The daemon never sets it.

## Programs and parts

| Term | Meaning |
|---|---|
| **LocalRouter** | The whole product. |
| **daemon**, `localrouterd` | The background program, written in Rust. One per user. It listens on ports 80 and 443, holds the route table, and is the only program that writes the data folder. |
| **CLI**, `localrouter` | The command-line tool, written in Rust. Holds no state. |
| **MCP shim**, `localrouter mcp` | The same binary, started by a coding agent. It turns MCP tool calls into socket calls. Holds no state. It never starts a daemon. |
| **app**, `LocalRouter.app` | The menu bar app, written in Swift. Shows Domains, Logs, Settings and Help. Starts the daemon at login. Holds no state. |
| **core**, `localrouter-core` | The Rust library in `libs/core` with routes, proxy, TLS, request log and API types. |
| **client** | Any program that talks to the daemon over the socket: the CLI, the MCP shim, the app. |
| **data folder** | `~/Library/Application Support/LocalRouter/`, or `$LOCALROUTER_HOME` when that is set (tests use it). |

## Interfaces

| Term | Meaning |
|---|---|
| **socket API** | The only way clients talk to the daemon. Newline-delimited JSON over the Unix socket `daemon.sock`. Shaped like JSON-RPC 2.0. Has 11 methods. |
| **method** | One call of the socket API, for example `register_route`. |
| **`api_version`** | Version of the socket API, returned by `hello`. Starts at `1.0`. A client stops when the major number differs from its own. |
| **MCP tool** | One function an agent can call through the MCP shim. There are six: `register_route`, `unregister_route`, `list_routes`, `find_free_port`, `get_logs`, `status`. |
| **API example** | One JSON file in `api/examples/`. Both the Rust and the Swift tests decode every example, so the two type sets stay equal. |

## Behaviour

| Term | Meaning |
|---|---|
| **byte copy** | What the daemon does for a TCP route: pass bytes both ways without reading them. No TLS, no parsing. |
| **`listen_failed`** | Status of a persistent TCP route whose listen port was taken when the daemon started. The route stays, but serves nothing until the port is free. |
| **lookup** | Finding the HTTP route for a request: take the `Host` header, remove `.localhost` and the port, then look for the host key. |
| **longest match** | If several routes could serve a host, the one with the most labels wins. `feat-login.shop` beats `shop`. |
| **fallback** | If the host key has no route, drop the left label and try again. `feat-other.shop` falls back to `shop`. Can be turned off in the config. |
| **forward** | Send the request to the route's target, with the `Host` header unchanged and `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Host` added. |
| **peer check** | On each new connection, the daemon checks the address of the other side before it reads any byte. It closes the connection unless the address is loopback or `allow_lan` is on. |
| **upstream up** | The target port accepts a TCP connection within 200 ms. Shown as the status dot in the app and `upstream_up` in `list_routes`. |
| **help page** | The page at `router.localhost`, served by the daemon itself. Markdown sent as plain text, for coding agents: setup steps, the status of HTTP, HTTPS and the local CA, and the current routes. No route can use the host key `router`. |
| **404 page** | The daemon's answer when no route matches. It lists all routes. |
| **502 page** | The daemon's answer when the target of an HTTP route does not answer within 2 seconds. It shows the target and the note. |
| **308 redirect** | The answer on port 80 for an HTTP route with `https_only`. It sends the client to the same URL with `https://`. |
| **atomic replace** | Writing a file by writing a temp file, calling `fsync`, then renaming it over the old file. A crash leaves the old file or the new file, never half a file. |
| **CA reset** | `localrouter ca reset`: remove trust for the old CA, delete it, create a new one. Cannot be undone. |

## Technical terms

| Term | Meaning |
|---|---|
| **TLD** | Top-level domain: the last part of a name, for example `localhost` in `shop.localhost`. |
| **`.localhost`** | A TLD reserved by RFC 6761 for the local machine. macOS resolves every `*.localhost` name to `127.0.0.1` and `::1` by itself. LocalRouter version 1 uses only this TLD. |
| **`.local`** | A TLD reserved for mDNS. LocalRouter does not use it: on macOS a failed lookup waits 5 seconds. |
| **`.test`** | A TLD reserved for testing (RFC 2606). Would need a DNS server and `/etc/resolver/test`. Deferred. |
| **mDNS** | Multicast DNS (RFC 6762). The protocol Bonjour uses to find devices on the local network. |
| **loopback** | Addresses that always mean "this machine": `127.0.0.0/8` for IPv4, `::1` for IPv6. |
| **IPv4-mapped IPv6 address** | An IPv4 address written in IPv6 form, for example `::ffff:127.0.0.1`. The peer check treats it like the IPv4 address. |
| **bind** | Reserve an address and port for a listening socket. |
| **privileged port** | A port below 1024. On macOS a normal user may bind it on `0.0.0.0` or `[::]`, but not on `127.0.0.1` or `::1`. |
| **TLS** | Transport Layer Security, the encryption behind HTTPS. |
| **SNI** | Server Name Indication: the name the client sends at the start of a TLS connection. The daemon uses it to choose the leaf certificate. |
| **ALPN** | Application-Layer Protocol Negotiation: the part of the TLS start where client and server agree on HTTP/2 (`h2`) or HTTP/1.1. |
| **SAN** | Subject Alternative Name: the field of a certificate that lists the names it is valid for. |
| **EKU** | Extended Key Usage: the field that says what a certificate may be used for. A TLS server certificate needs `serverAuth`. |
| **name constraints** | An optional field of a CA certificate that limits which names it may sign. Planned for the local CA if all main clients accept it (open point U1). |
| **plain TCP** | A TCP connection that carries some other protocol (Postgres, Redis) directly, with no HTTP and no name inside. |
| **SNI routing for TCP** | Choosing a TCP route by the TLS SNI name. Deferred (open point U5): many database clients do not send SNI in their first bytes. |
| **WebSocket** | A long-lived two-way connection that starts as an HTTP request with `Upgrade`. Vite and Next.js use it for hot reload. |
| **hot reload**, HMR | Hot module replacement: the dev server pushes code changes to the open page without a full reload. |
| **MCP** | Model Context Protocol: the protocol coding agents such as Claude Code use to call tools. |
| **stdio** | Standard input and output. The MCP shim talks MCP to the agent over stdio. |
| **Unix socket** | A socket that is a file on disk (`daemon.sock`) instead of a network port. Only processes that can open the file can connect. |
| **kqueue** | The macOS system call for event notification. The daemon uses `EVFILT_PROC` with `NOTE_EXIT` to learn when an owner process exits. |
| **flock** | A file lock. The daemon holds it on `daemon.lock`, so a second daemon on the same data folder exits at once. |
| **LaunchAgent** | A background program that macOS starts for the logged-in user. The daemon runs as one. |
| **SMAppService** | The macOS API an app uses to register its own LaunchAgent. |
| **menu bar extra** | An icon on the right side of the macOS menu bar, next to the clock. SwiftUI builds it with `MenuBarExtra`. |
| **`LSUIElement`** | An `Info.plist` key. When it is `YES`, the app has no Dock icon and no main menu. |
| **login keychain** | The user's own keychain on macOS. It stores the trust setting for the local CA. |
| **ADR** | Architecture Decision Record: a document in `docs/adr/` that records a decision and why. |

## Distribution and updates

Decided in [ADR 02](adr/02-distribution-and-self-update-2026-09-26/README.md).

| Term | Meaning |
|---|---|
| **release** | A GitHub release `vX.Y.Z` on `shchahrykovich/localrouter` with one asset, `localrouter-X.Y.Z.dmg`. |
| **version** | `[workspace.package] version` in `Cargo.toml`: the one source for every program and for `Info.plist`. |
| **Developer ID** | The Apple certificate ("Developer ID Application") that signs release builds. Its team id is the **Team ID** the updater compares. |
| **notarization** | Apple's automated check of a signed image (`xcrun notarytool`). The result is **stapled** to the DMG so it works offline. |
| **Gatekeeper**, `spctl` | The macOS component that decides whether signed code or an image may open. The updater asks it about each downloaded DMG. |
| **ad-hoc signed** | Signed without a certificate (`codesign --sign -`). Local builds from `scripts/install.sh` are ad-hoc and never update themselves. |
| **self-update** | The app checks `releases/latest`, downloads the DMG after a click, checks it, and a detached **install script** swaps the bundle and restarts the daemon. |
| **bundle layout** | Where programs live in `LocalRouter.app`: `Contents/MacOS/LocalRouter` (app), `Contents/MacOS/localrouterd` (daemon), `Contents/Helpers/localrouter` (CLI). The CLI is not in `MacOS` because `localrouter` and `LocalRouter` are one name on a case-insensitive disk. |
| **Install Command Line Tool** | The menu command that links `~/.local/bin/localrouter` to the bundled CLI. |
| **Install Claude Code Instructions** | The menu command that links `~/.claude/LocalRouter.md` to the note in the bundle (`Contents/Resources/LocalRouter.md`) and adds `@LocalRouter.md` as the first line of `~/.claude/CLAUDE.md`. It replaces only a link into a LocalRouter bundle, and does nothing without `~/.claude`. |
| **`.env.notarize`** | Notary credentials at the repository root, ignored by git, read by the release scripts and never exported. |
