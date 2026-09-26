# 10. Semantic Change Manifest

## 1. Manifest status

```text
Manifest status: PLANNED
```

This describes intended behaviour only. The repository has no code yet.

## 2. Semantic change summary

```text
Artifacts
+ 3 executables: localrouterd, localrouter, LocalRouter.app
+ 1 library crate: localrouter-core
+ 1 LaunchAgent (registered through SMAppService, plist inside the app bundle)
+ 4 shared HTTP listeners: 80 and 443, on IPv4 and on IPv6
+ 2 loopback listeners per TCP route (127.0.0.1 and ::1), opened and closed at run time
+ 2 route protocols: http, tcp
+ 1 Unix socket API with 11 methods
+ 6 MCP tools
+ 10 CLI commands

Persistent data
+ 2 settings/state files: config.json, routes.json
+ 2 CA files: ca/ca.key, ca/ca.pem
+ 2 daemon log files: daemon.log, daemon.log.1
+ 1 lock file: daemon.lock
+ 1 trust setting in the login keychain (after "Trust" only)

Runtime effects
+ 1 file REPLACE path (routes.json), 1 file REPLACE path (config.json)
+ 1 file CREATE path (CA folder), 1 file DELETE path (CA reset)
+ 1 keychain WRITE path, 1 keychain DELETE path
+ 3 outbound CALL paths, loopback only: HTTP forward, HTTPS forward, TCP byte copy
+ 1 BIND path and 1 CLOSE path for TCP route listeners
+ 3 in-memory WRITE paths (route table, cert cache, request log)

Modified
0 (empty repository)

External effects
+ macOS login keychain trust setting
+ TCP ports 80 and 443 are held while the daemon runs
+ one loopback port per TCP route is held while the route exists
0 network calls leave the Mac

Destructive operations
1 (CA reset deletes the old CA)

Unresolved effects
5
```

Important zeroes:

```text
Database entities:            0
HTTP services on the internet: 0
Root processes:               0
System DNS or /etc changes:   0
Background jobs or schedules: 0
```

## 3. Source of truth

```text
CANONICAL
  config.json          owner: localrouterd
  routes.json          owner: localrouterd, persistent routes only
  ca/ca.key, ca.pem    owner: localrouterd

CANONICAL, MEMORY ONLY (lost on daemon stop, by design)
  owned and session routes
  request log ring buffer

DERIVED
  in-memory route table    built from routes.json at start, plus API calls
  leaf certificates         made from ca.key on demand; rebuilt after restart

EXTERNAL
  login keychain trust setting
    authority: the user. The daemon never writes it and cannot read it back
    reliably; `status` asks macOS whether ca.pem is trusted.

REPLICA
  Swift API structs in apps/menubar/   copy of libs/core/src/api.rs shapes,
                                kept equal by the contract test (I11)
```

## 4. Artifacts

```text
Executables
+ localrouterd                   Rust daemon, one per user
+ localrouter                    Rust CLI; `localrouter mcp` is the MCP server
+ LocalRouter.app                Swift menu bar app

Libraries
+ localrouter-core               Rust crate, no binary

Services
+ LaunchAgent <bundle-id>.daemon registered by the app through SMAppService

Network listeners
+ 0.0.0.0:80   + 0.0.0.0:443   + [::]:80   + [::]:443   (shared, HTTP routes)
+ 127.0.0.1:<listen_port> and [::1]:<listen_port> per TCP route (dynamic)

Socket API (Unix socket daemon.sock), 11 methods
+ hello  status  register_route  unregister_route  list_routes
+ find_free_port  get_logs  subscribe_logs  get_config  set_config  reset_ca

MCP tools, 6
+ register_route  unregister_route  list_routes  find_free_port  get_logs  status

CLI commands, 10
+ add  rm  list  logs  status  trust  untrust  ca-path  ca reset  mcp

Persistent artifacts (per user)
+ 1 config.json            ~300 bytes
+ 1 routes.json            ~200 bytes per persistent route
+ 2 CA files               ~1 KB together
+ 0 to 2 daemon log files  at most 5 MB each
+ 1 daemon.lock            empty
+ 0 or more routes.json.bad-<time>   only after a failed load
+ 1 keychain trust setting (after "Trust")

Configuration
+ LOCALROUTER_HOME         environment variable, overrides the data folder (tests)
+ ports in config.json     default 80 and 443; tests use 0 (random free port)
```

## 5. Runtime effects

```text
WRITE
type:                   file replace
target:                 routes.json
operation:              write temp, fsync, rename
trigger:                register or unregister of a persistent route
cardinality:            one full-file write per change
write_idempotent:       yes
producer_deterministic: yes (routes sorted by host)
retention:              until the route is removed
destructive:            replaces the previous file
reversible:             yes, by registering the old route again
```

```text
WRITE
type:                   file replace
target:                 config.json
operation:              write temp, fsync, rename
trigger:                set_config from the app or CLI
write_idempotent:       yes
producer_deterministic: yes
destructive:            replaces the previous settings
reversible:             yes
```

```text
WRITE
type:                   file create
target:                 ca/ (ca.key, ca.pem)
operation:              write into ca.tmp-<pid>/, fsync, rename folder
trigger:                daemon start when ca/ does not exist, or reset_ca
cardinality:            once per user, and once per reset
write_idempotent:       no  (a second run would make a different key; guarded by I7)
producer_deterministic: no  (random key)
retention:              until reset or uninstall
destructive:            no
reversible:             no  (a lost key cannot be made again)
```

```text
DELETE
type:                   file delete
target:                 ca/
trigger:                reset_ca (user action only, never agents)
write_idempotent:       yes
destructive:            yes
reversible:             no. Every certificate the old root signed stops being valid.
```

```text
WRITE / DELETE
type:                   keychain trust setting
target:                 login keychain, ca.pem
operation:              security add-trusted-cert / remove-trusted-cert
trigger:                user clicks Trust / Untrust, or runs trust / untrust
write_idempotent:       yes
producer_deterministic: yes
destructive:            no
reversible:             yes, with untrust
```

```text
CALL
type:                   HTTP or HTTPS forward
target:                 HTTP route target, loopback only
trigger:                every accepted request that matches a route
cardinality:            one upstream request per client request
write_idempotent:       depends on the client request; the proxy adds no retries
producer_deterministic: n/a
note:                   for an https:// target the upstream certificate is NOT
                        verified (dev servers use self-signed certificates)
```

```text
CALL
type:                   TCP byte copy
target:                 TCP route target, loopback only
trigger:                every connection accepted on a TCP route's listen port
cardinality:            one upstream connection per client connection
write_idempotent:       n/a (opaque bytes; the daemon adds no retries)
```

```text
BIND / CLOSE
type:                   TCP listener
target:                 127.0.0.1:<listen_port> and [::1]:<listen_port>
trigger:                BIND on register of a TCP route and on daemon start for
                        persistent TCP routes; CLOSE on unregister, owner exit,
                        daemon stop, or a failed register call
cardinality:            two sockets per TCP route
destructive:            CLOSE also closes every open connection of that route
reversible:             yes, by registering the route again
```

```text
WRITE (memory)
targets:                route table, leaf cert cache, request log ring buffer
retention:              until daemon stop; request log capped at 1,000 entries
```

```text
DELETE (memory)
type:                   route removal
trigger:                kqueue NOTE_EXIT for the owner pid
cardinality:            one per owned route
```

```text
APPEND
type:                   file append
target:                 daemon.log, rotated to daemon.log.1 at 5 MB
trigger:                daemon events: start, stop, bind errors, refused peers
                        (at most one refused-peer line per second)
```

## 6. Reads and writes

```text
READ
- config.json, routes.json         at daemon start
- ca/ca.key, ca/ca.pem             at daemon start
- ca/ca.pem                        CLI and app, for trust / untrust
- owner process exit               kqueue EVFILT_PROC on owner pids

WRITE
- config.json, routes.json, ca/, daemon.log, daemon.lock   (daemon only)

EXTERNAL READ
- login keychain trust state       status (daemon asks macOS, read only)

EXTERNAL WRITE
- login keychain trust setting     app and CLI only
```

Hidden dependency created by this design: any tool that trusts only the macOS
keychain works after "Trust"; any tool with its own CA list (Firefox, Node,
Python, Java, Docker containers) silently still fails until the user sets it up.

## 7. Interfaces and events

```text
+ SOCKET  11 methods on daemon.sock, JSON lines, api_version 1.0
+ MCP     6 tools through `localrouter mcp` (stdio)
+ CLI     10 commands
+ HTTP    ports 80 and 443 on all interfaces (peer-checked), HTTP routes
+ HTTP    308 redirect to https:// for routes with https_only
+ TCP     one loopback listen port per TCP route
+ HTTP    404 page (lists routes) and 502 page (shows target and note)

Events:   0 (subscribe_logs is a stream on the socket, not an event bus)
REST:     0 public endpoints for control; control goes only through the socket

register_route parameters: host, protocol (http | tcp), target, listen_port
(tcp only), https_only (http only), note, owner_pid, persistent
```

Not exposed to agents on purpose: `set_config` and `reset_ca`. An agent cannot
turn on LAN access or replace the CA.

## 8. External side effects

```text
+ login keychain trust setting
  scale: one entry per user

+ ports 80 and 443 held while the daemon runs
  effect: another local program that wants these ports (nginx, Docker port
  publishing) fails to start, or LocalRouter fails to bind (see Blast radius)

+ one loopback port per TCP route, held while the route exists
  effect: a program that later wants that port fails with "address in use"

+ macOS application firewall prompt, possibly once

Network traffic leaving the Mac: none. No update check, no telemetry.
Cost: none.
```

## 9. Invariants

```text
I1.  Only localrouterd writes files in the data folder.
     enforced by: file-writing code lives in apps/daemon; apps/cli and the
     Swift app have no path to it. Test T3, plus T11: a test reads
     `cargo metadata` and fails if apps/cli depends on apps/daemon.

I2.  A connection from a non-loopback peer is closed before any byte is read,
     unless allow_lan is on.
     enforced by: peer filter in listen.rs; T4 unit tests; M4 from a second device.

I3.  A route with owner_pid is never written to routes.json, and a request with
     both persistent and owner_pid is refused.
     enforced by: T3.

I4.  routes.json and config.json are replaced atomically; a failed write leaves
     memory and disk equal to their state before the call.
     enforced by: T3 (write into a read-only folder, assert old state kept).

I5.  A leaf certificate is issued only for a name that ends in .localhost and
     has a route; it has the name in SAN, serverAuth EKU, validity 90 days.
     enforced by: T2.

I6.  ca.key is created with mode 0600 and never appears in a socket reply, an
     MCP result or a log line.
     enforced by: T2 (mode), T6 (no reply contains "PRIVATE KEY").

I7.  The daemon never replaces an existing CA except through reset_ca.
     enforced by: T2 (a damaged ca/ leaves HTTPS off and the files untouched).

I8.  At most one daemon runs per data folder.
     enforced by: flock on daemon.lock; T6 starts a second daemon and expects exit.

I9.  Route targets are loopback addresses only.
     enforced by: validation in routes.rs; T1.

I10. The request log holds at most N entries and never stores query strings,
     headers or bodies.
     enforced by: T8.

I11. Rust and Swift decode the same API JSON: every file in api/examples
     decodes on both sides.
     enforced by: T9 (Rust test and XCTest walk the same folder).

I12. The Host header reaches the dev server unchanged.
     enforced by: T5.

I13. The MCP server exposes exactly the six tools listed in section 7.
     enforced by: T7 (tools/list snapshot).

I14. A client refuses a daemon whose api_version has a different major number.
     enforced by: T6 and T7.

I15. TCP route listeners bind only 127.0.0.1 and ::1, whatever allow_lan says.
     enforced by: T12 (check the bound address of every TCP route socket).

I16. A register_route call that fails leaves no new listener open.
     enforced by: T12 (port taken on ::1 only: call fails, 127.0.0.1 port free again).

I17. A TCP route's listen_port is unique among routes, is not an HTTP port, and
     is not equal to its own target port.
     enforced by: validation in routes.rs; T1.

I18. Removing a TCP route closes its listener and all its open connections.
     enforced by: T12.

I19. Fields are valid only for their protocol: listen_port only for tcp,
     https_only only for http, tcp:// targets only for tcp, http(s):// only for http.
     enforced by: T1.
```

## 10. Data impact

```text
Migration          none (no earlier data). routes.json and config.json carry
                   "version": 1 so a later change can migrate them.
Backfill           none
Derived rebuild    leaf certificates and the route table, on every start
Destructive        reset_ca deletes the CA; a failed routes.json load moves the
                   file aside (not deleted)
Expected growth    routes.json: ~200 bytes per persistent route
                   daemon.log: capped at 2 x 5 MB
                   routes.json.bad-*: one file per failed load, not cleaned up
                   automatically (small, and the user may need it)
Retention          until the user removes routes or uninstalls
Data loss          owned and session routes are lost on daemon stop, by design
```

## 11. Blast radius

```text
Other programs on ports 80 and 443 (nginx, Apache, Docker, another proxy)
dependency:   the same TCP ports
impact:       whoever starts second cannot bind
failure mode: if LocalRouter is second, status and the menu bar show the error
              and routes do not work on that port. If LocalRouter is first, the
              other program fails with its own "address in use" error, which
              does not mention LocalRouter.

Dev servers
dependency:   receive X-Forwarded-For/Proto/Host and a .localhost Host header
impact:       apps that trust X-Forwarded-Host build absolute URLs with
              feat.shop.localhost (usually what we want)
failure mode: a dev server with a Host allow-list other than Vite's default
              answers 403 or "Invalid Host header". Visible, not silent.

Tools with their own CA list (Firefox, Node, Python, Java, containers)
dependency:   the local CA
impact:       HTTPS fails until each tool is configured
failure mode: a TLS error in that tool. Visible, not silent.

Two agents working on the same project
dependency:   register_route replaces an existing host
impact:       the second agent's route replaces the first
failure mode: SILENT for the first agent: its URL now shows the other
              worktree. Only the reply to the second agent says replaced: true.

Programs that want a port a TCP route holds
dependency:   the same loopback port
impact:       whoever binds second fails
failure mode: if the route is first, the other program fails with its own
              "address in use" error, which does not mention LocalRouter.
              If the other program is first, register_route fails with a clear
              error, or a persistent route shows listen_failed at start.

Database and cache clients using a TCP route
dependency:   the route exists
impact:       after unregister or owner exit, connections are closed and new
              ones are refused
failure mode: visible "connection refused" in the client.

Anything on the LAN
dependency:   the shared HTTP sockets bound on all interfaces
impact:       none while I2 holds
failure mode: a bug in the peer check exposes every dev server to the LAN.
```

Confirmed unaffected: system DNS settings, `/etc/hosts`, `/etc/resolver`, other
users on the same Mac (each user has their own daemon, but see Risks for
ports), the network outside the Mac.

## 12. Unresolved effects

```text
? U1. Name constraints on the root CA
status:  REQUIRES_VERIFICATION
effect:  the root may sign only localhost names
reason:  not checked whether macOS trust, Chrome, Firefox (enterprise roots)
         and Node all accept a name-constrained root
blocks:  task 3 (final CA profile)
outcome: M7 decides; if any main client rejects it, ship without constraints
         and record the decision in this ADR

? U2. Code signing, notarization, bundle id, distribution
status:  REQUIRES_DECISION
effect:  who signs the app, which bundle id, DMG or Homebrew cask
reason:  SMAppService and the firewall prompt behave differently for signed and
         unsigned apps; nothing is decided
blocks:  task 11 (packaging), not the Rust tasks
outcome: separate ADR

? U3. .test TLD with a built-in DNS server
status:  DEFERRED
effect:  /etc/resolver/test and a DNS listener in the daemon
blocks:  nothing in version 1
outcome: separate ADR if needed

? U4. Where the CLI is linked for the terminal
status:  REQUIRES_DECISION
effect:  a symlink to Contents/MacOS/localrouter somewhere in PATH
reason:  /usr/local/bin needs admin on some Macs; ~/.local/bin is often not in
         PATH; Homebrew would handle it but depends on U2
blocks:  task 11, and the `claude mcp add` line in the README
outcome: decision inside this ADR before task 11

? U5. TCP routing by TLS SNI on one shared port
status:  DEFERRED
effect:  many TCP routes share one port when the client starts TLS with SNI
reason:  many database clients negotiate TLS inside their own protocol, so the
         first bytes carry no name; only some clients support direct TLS
blocks:  nothing in version 1
outcome: separate ADR if needed
```

## 13. Risks

- **HTTPS targets are not verified.** For an `https://` target the daemon accepts
  any certificate. This is safe only while I9 (loopback-only targets) holds.
- **Other users on the same Mac can connect.** Loopback is shared by every user
  of the Mac. Any local user can open a TCP route's port or the HTTP ports. This
  is no worse than the dev server's own port, which is also on loopback, but
  LocalRouter does not add any access control.

- **The local CA is a powerful secret.** Any process of the same user can read
  `ca.key`. Mode `0600` does not protect against malware running as the user.
  Without name constraints (U1), a stolen key can sign any domain that this
  user's browser will trust.
- **Two users on one Mac.** Only one daemon can hold ports 80 and 443. The second
  user's daemon reports a bind error. Version 1 does not support this.
- **`.localhost` resolution is OS behaviour.** It was measured on macOS 27.0 only.
  Programs with their own DNS code may not follow it. Not verified.
- **Unix socket path length.** macOS limits socket paths to 104 bytes. The
  default path is 52 bytes plus the home folder. A very long home path or a
  deep `LOCALROUTER_HOME` in tests fails to bind. The daemon must report this
  clearly rather than with a raw `EINVAL`.
- **PID reuse.** Between the registration check and the kqueue attach, the
  owner process could exit and its pid be reused. The window is microseconds;
  kqueue attach fails for a dead pid and the route is refused, but a reused pid
  in that window would keep the route alive until that other process exits.
- **The firewall prompt** can confuse users. "Deny" should not affect loopback
  traffic; not yet verified (M4).

## 14. Rollback

```text
Code rollback
  Nothing to roll back: no code exists before this ADR.

Uninstall on a user's Mac (the real "rollback" for this product)
  1. localrouter untrust            removes the keychain trust setting
  2. quit the app; macOS unregisters the LaunchAgent when the app is deleted
  3. delete ~/Library/Application Support/LocalRouter and ~/Library/Logs/LocalRouter

Not undone automatically
  - The keychain trust setting stays if the user deletes the app WITHOUT
    running untrust first. A trusted root with its key still on disk is left
    behind. The Settings screen gets an "Uninstall" button that runs step 1
    and 3 before the user deletes the app (task 11).
  - Settings made by hand in Firefox, NODE_EXTRA_CA_CERTS in shell profiles.
  - A CA reset cannot be undone: certificates signed by the old root, and the
    old trust, are gone.
```
