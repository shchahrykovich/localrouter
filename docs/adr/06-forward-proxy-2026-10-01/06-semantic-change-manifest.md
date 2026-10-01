# 6. Semantic Change Manifest

## 1. Manifest status

```text
Manifest status: PLANNED
```

This file describes intended behaviour. Nothing here is built.

## 2. Semantic change summary

```text
Artifacts
+ 1 network listener pair (proxy port, 127.0.0.1 and ::1)
+ 1 certificate authority (inspection CA, inspect-ca/)
+ 2 socket API methods (get_proxy, reset_inspect_ca)
+ 1 MCP tool (get_proxy)
+ 1 CLI command group (proxy ...)
+ 1 menu item (Open Chrome via Proxy, only when Chrome is installed)
+ 1 crate dependency (rustls-platform-verifier)
~ 3 config fields added to config.json
~ 1 log entry shape (optional via, mode, bytes_in, bytes_out)
~ status reply (optional proxy field)

Persistent data
+ 2 files (inspect-ca/ca.key, inspect-ca/ca.pem), only after first need
~ config.json gains 3 fields

Runtime effects
+ outbound connections from the daemon to any host on any port
+ DNS lookups by the daemon
+ TLS termination for names that are not .localhost
+ in-memory leaf certificates for names in the inspect set

Modified
~ 6 existing components (core tls, proxy, logs, config, api; daemon; CLI; MCP; app)

External effects
+ network traffic to the internet, on behalf of local clients

Destructive operations
1 (reset_inspect_ca deletes the inspection CA; user action)

Unresolved effects
3
```

## 3. Source of truth

```text
BEFORE
  config.json        role: CANONICAL, written only by the daemon
  ca/                role: CANONICAL

AFTER
  config.json        role: CANONICAL; also holds proxy_enabled, proxy_port, inspect_hosts
  ca/                role: CANONICAL, unchanged
  inspect-ca/        role: CANONICAL; rebuilt by: nothing (a reset makes a different CA)

DERIVED (memory only)
  inspect set        = config inspect_hosts + hosts of enabled ADR 07 rules
  leaf certificates  from inspect-ca/, 90 days, never written

EXTERNAL
  login keychain     trust for inspect-ca/ca.pem; written by the CLI or app, never by the daemon
  client settings    HTTPS_PROXY, NODE_EXTRA_CA_CERTS, Chrome flags; held by each client, not by LocalRouter
```

## 4. Artifacts

```text
Infrastructure
+ listener pair 127.0.0.1:<proxy_port>, [::1]:<proxy_port>   (only while proxy_enabled)

Persistent artifacts
+ inspect-ca/ca.key   mode 0600
+ inspect-ca/ca.pem   mode 0644
~ config.json         + proxy_enabled, proxy_port, inspect_hosts

Code
+ libs/core/src/forward.rs, upstream.rs, inspect.rs
+ apps/daemon/src/proxy_listen.rs
~ libs/core/src/tls.rs, proxy.rs, logs.rs, config.rs, api.rs, help.md, note.md, mcp.md
~ apps/daemon/src/daemon.rs, socket.rs
~ apps/cli/src/main.rs, trust.rs, mcp.rs
~ apps/menubar Api.swift, Settings view, Logs view, status item right-click menu
+ apps/menubar/Sources/LocalRouterKit/ChromeLauncher.swift

Interfaces
+ socket methods get_proxy, reset_inspect_ca
+ MCP tool get_proxy
+ CLI proxy, proxy on|off|port|env|chrome|inspect add|rm|list|trust|untrust|ca-path|ca reset
+ UI  right-click menu item "Open Chrome via Proxy"
+ api/examples/get_proxy.*.json, reset_inspect_ca.*.json, status with proxy, log.event with via

Zeroes
Database entities:  0
New programs:       0
New processes:      0
Background jobs:    0
Files written outside the data folder by the daemon: 0
(Chrome writes its own profile in ~/Library/Caches/LocalRouter<suffix>/chrome-proxy)
```

## 5. Runtime effects

```text
CALL  outbound connection
type:                   TCP connect, optional TLS
target:                 any host and port a local client names
trigger:                absolute-form request, or CONNECT, on the proxy port
cardinality:            one per request, or reused from the pool (90 s idle)
write_idempotent:       n/a (no local state)
producer_deterministic: no (the server answers what it answers)
retention:              none
destructive:            depends on the request the client sent; the proxy adds nothing
reversible:             no: what a request did on a remote server cannot be undone

CALL  DNS lookup
target:                 macOS resolver (getaddrinfo)
trigger:                each new upstream connection
producer_deterministic: no

WRITE  request log entry (memory)
trigger:                each proxied HTTP request, each closed tunnel
cardinality:            one per request or tunnel; ring buffer of log_size
write_idempotent:       no (a new entry each time)
producer_deterministic: yes
retention:              until pushed out or the daemon stops

WRITE  inspect-ca/ (CREATE)
trigger:                inspect set becomes non-empty and the folder does not exist
cardinality:            once per data folder
write_idempotent:       yes (exists → no write)
producer_deterministic: no (new random key)
destructive:            no

DELETE inspect-ca/ (then CREATE)
trigger:                reset_inspect_ca, a user action
destructive:            yes; clients that trusted the old CA stop accepting inspected hosts
reversible:             no

WRITE  config.json (REPLACE)
trigger:                set_config with a proxy field
write_idempotent:       yes
producer_deterministic: yes

CALL  start a Chrome process
type:                   NSWorkspace.openApplication, new instance, fixed arguments
trigger:                the user clicks Open Chrome via Proxy, or runs proxy chrome
cardinality:            one process per click while none runs with that profile;
                        later clicks open a window in the running one
write_idempotent:       yes (same profile folder)
producer_deterministic: yes (arguments come from get_proxy)
retention:              the profile folder stays until the user deletes it
destructive:            no

CREATE in-memory leaf certificate
trigger:                CONNECT to a host in the inspect set, no cached leaf
retention:              memory, refreshed after 60 days
```

## 6. Reads and writes

```text
READ
- config.json (at start and on set_config)
- inspect-ca/ca.key, ca.pem (at start, on create, on reset)
- route table (for .localhost names through the proxy)
- macOS trust store (upstream certificate checks)
- login keychain trust state (get_proxy, status)

WRITE
- config.json
- inspect-ca/
- request log (memory)

EXTERNAL READ
- DNS
- internet servers (responses)

EXTERNAL WRITE
- internet servers (the client's requests, sent on)
```

Hidden dependency created by this change: the inspect set depends on ADR 07's
rule table as well as on `config.json`. A rule added through ADR 07 makes a
host inspected without a change to `inspect_hosts`. `get_proxy` returns
`inspect_set` so this is visible.

## 7. Interfaces and events

```text
+ SOCKET get_proxy
+ SOCKET reset_inspect_ca             (not an MCP tool)
~ SOCKET status                       + optional proxy
~ SOCKET set_config / get_config      + proxy_enabled, proxy_port, inspect_hosts
~ EVENT  log                          + optional via, mode, bytes_in, bytes_out
~ API_VERSION 1.2 → 1.3               (minor: additive)
+ MCP    get_proxy                    (tool count 6 → 7)
+ CLI    proxy ...
+ HTTP   proxy protocol on 127.0.0.1:<proxy_port> (absolute form, CONNECT)
REST, webhooks, queues: unchanged (none exist)
```

## 8. External side effects

```text
+ Network traffic from the daemon to the internet
  scale:  whatever the configured clients send; Claude Code and Chrome can send
          many megabytes per minute (streamed model replies, web pages)
  cost:   none charged by LocalRouter; the user's network and remote APIs see
          the same requests as without the proxy
+ Remote servers see the same client IP and the same headers as without the proxy
```

## 9. Invariants

```text
I1. The proxy port binds only 127.0.0.1 and ::1, both or neither, whatever
    allow_lan says. No wildcard socket is ever bound for it.
    enforced by: proxy_listen.rs; T1 reads the bound addresses.

I2. With proxy_enabled false, no proxy socket is open. Turning it off closes
    the listener and every open proxy connection.
    enforced by: T7.

I3. The inspection CA signs a leaf only for a name in the inspect set at the
    time of the CONNECT. The 80/443 listeners never use the inspection CA.
    The local CA still signs only .localhost names with a route (ADR 01 I5).
    enforced by: separate CertStore with its own allow function; T5.

I4. inspect-ca/ca.key is created with mode 0600 and never appears in a socket
    reply, an MCP result or a log line. The daemon never replaces an existing
    inspection CA except through reset_inspect_ca.
    enforced by: T5; T9 (no reply contains "PRIVATE KEY").

I5. The inspection CA is not created until the inspect set is non-empty.
    enforced by: T5 (fresh daemon, proxy on, no inspect host → no inspect-ca/).

I6. Every upstream TLS connection verifies the server certificate with the
    macOS trust store. A failed check answers 502 and sends no request bytes.
    enforced by: upstream.rs takes only a verifying ClientConfig; T6.

I7. A tunnel passes bytes unchanged in both directions and reads nothing after
    the CONNECT request headers.
    enforced by: T3 (random bytes both ways compare equal).

I8. A .localhost name through the proxy is answered from the route table and
    is never resolved by DNS or sent to another machine.
    enforced by: T2 with a resolver that fails the test if it is called.

I9. A request whose destination is the proxy's own listen address is answered
    508 and not forwarded.
    enforced by: T2.

I10. Proxy-Authorization and Proxy-Connection never reach the upstream; the
     proxy adds no Via or X-Forwarded-* header to proxied traffic.
     enforced by: T2 (echo upstream).

I11. Proxy log entries never store a query string, a header or a body
     (ADR 01 I10 extended).
     enforced by: T13.

I12. Turning the proxy on, off, or moving its port needs no daemon restart and
     does not touch routes. A failed bind changes nothing.
     enforced by: T7.

I13. Every API example decodes in Rust and Swift (ADR 01 I11), including the
     new ones; METHODS lists 13 methods, each with an example.
     enforced by: T9.

I14. The MCP server lists exactly seven tools. get_proxy changes nothing.
     enforced by: T11.

I15. help.md, note.md and mcp.md describe the proxy, get_proxy, proxy env, the
     trust step, that an agent cannot move its own running traffic, and the
     rule never to write proxy settings into project files; they name their
     instance through templates.
     enforced by: T12 (agent_texts.rs, no_fixed_names.rs).

I16. An older client decodes get_logs from a newer daemon: proxy entries use
     kind "http" with optional fields only.
     enforced by: T9 (decode the new example with the 1.2 shape).

I17. The menu item Open Chrome via Proxy is shown only when the app finds
     Google Chrome (bundle id com.google.Chrome). It starts Chrome with a
     --user-data-dir that is not Chrome's default profile, so the user's
     normal Chrome never gets the proxy flag.
     enforced by: ChromeLauncher builds its arguments from get_proxy;
     T14 (Swift unit test) checks the arguments and the hidden case; M3.

I18. The app, the CLI and get_proxy use one argument list for Chrome: the
     daemon's chrome_args. No client builds its own.
     enforced by: T10 and T14 compare against the get_proxy example.
```

I16 came from writing this manifest: the first draft of
[03](03-clients-and-agents.md) added a log `kind` `proxy`, which an older CLI
would fail to decode. The change file now reuses `http`.

## 10. Data impact

```text
New persistent data
+ inspect-ca/ (2 small files), only for users who inspect
~ config.json + 3 fields

Migration         none (missing fields get instance defaults, ADR 04 rule)
Backfill          none
Derived rebuild   none
Destructive       reset_inspect_ca (user action)
Expected growth   none on disk; memory: one leaf per inspected name
Retention         unchanged (request log in memory)

Compatibility with older readers
- an older daemon ignores the proxy fields; its next set_config writes config.json
  without them → proxy settings lost on downgrade (inspect-ca/ stays on disk)
- an older CLI or app decodes log entries (I16) and ignores status.proxy
```

## 11. Blast radius

```text
Router traffic (ports 80, 443)
dependency:   shares the daemon process, tokio runtime and request log
impact:       heavy proxy traffic (a large download through the proxy) shares CPU
              and the log ring buffer with dev server requests
failure mode: router entries are pushed out of the log sooner. Silent: the log
              just shows less router history. Logs view can filter by via.

Request log readers (Logs view, get_logs, localrouter logs)
dependency:   read all entries
impact:       entries of internet hosts appear among .localhost entries
failure mode: an agent that reads get_logs to debug its dev server sees noise;
              get_logs host filter still works

Local CA, HTTPS for dev names
dependency:   tls.rs is changed to hold two CertStores
impact:       a bug in choosing the store would give a dev name the wrong leaf
failure mode: loud: the browser refuses the certificate. T5 covers it.

Clients configured with HTTPS_PROXY when the proxy is off or the daemon is down
dependency:   the client sends every request to 127.0.0.1:<proxy_port>
impact:       all its network calls fail
failure mode: loud but confusing: "connection refused" from Claude Code or curl,
              with no mention of LocalRouter. Only notes in get_proxy and the
              agent texts explain it.

Clients that inspect a host but do not trust the inspection CA
failure mode: loud: certificate error on that host only

User's normal Chrome
dependency:   none: the proxy Chrome uses its own profile
impact:       two Chrome windows look the same
failure mode: the user types in the wrong window and sees no traffic in Logs.
              Silent. Chrome has no flag that names a profile, so the window
              cannot be labelled by LocalRouter. Accepted: the popover says
              that a new Chrome window was opened, and the proxy profile has
              its own colour and avatar once the user sets them.

Users who never turn the proxy on
dependency:   none at run time
impact:       none: no socket, no CA, no new outbound connection. Confirmed
              unaffected: routes, TCP routes, folder routes, updater, installers.
```

## 12. Unresolved effects

```text
? U1. Upstream proxy (company proxy)
status:  REQUIRES_DECISION
effect:  the daemon sends its upstream connections through another proxy
reason:  some company networks allow internet access only through their proxy;
         the daemon is a LaunchAgent and does not see the user's shell HTTPS_PROXY.
         Undecided: where the setting lives, PAC files, authentication.
blocks:  nothing in this ADR; such networks cannot use the proxy until decided
outcome: a later ADR

? U2. Who may use the proxy port
status:  REQUIRES_DECISION
effect:  restrict the port to processes of this user
reason:  loopback is shared by every account on the Mac. Another local user can
         use the proxy, and with ADR 07 log rules their traffic can reach this
         user's capture files. Options: a random token in Proxy-Authorization
         (clients support user:pass@ in the proxy URL), or a check of the peer
         process's user id. Both change how clients are set up.
blocks:  nothing for single-user Macs
outcome: a decision inside this ADR before release, or accepted with a note

? U3. System-wide proxy and PAC
status:  REJECTED for this ADR
effect:  set the macOS network proxy, or serve a PAC file at router.localhost
reason:  changes the whole Mac's traffic; out of scope. The Chrome profile and env
         variables cover the asked use cases.
```

## 13. Risks

```text
- The daemon now has a reason to open any outbound connection. A bug in the
  .localhost branch (I8) could send a dev request to the internet.
- The inspection CA key on disk can sign any name for this user. Losing it to
  malware is the same class of risk as ca/ca.key today, but a user may trust it
  without reading what it means. The Trust button text must say "lets LocalRouter
  read HTTPS traffic for the hosts you list".
- Claude Code's own TLS stack: whether the native build honours
  NODE_EXTRA_CA_CERTS is checked by manual test M2, not by CI. If it does not,
  inspection of Claude Code traffic needs another step.
- HTTP/2 to the client and HTTP/1.1 or HTTP/2 upstream: header and trailer
  handling differences (gRPC trailers) can break some APIs on inspected hosts.
- A large streaming response through an inspected host must not be buffered
  (in this ADR nothing buffers). ADR 07 adds buffering only on request.
```

## 14. Rollback

```text
Code rollback
  revert the commits. Sufficient for routes and the local CA.

Schema rollback
  config.json keeps 3 unknown fields; an older daemon ignores them and drops
  them on its next write.

Data rollback
  delete inspect-ca/ by hand if not wanted.

Infrastructure rollback
  nothing: the listener disappears with the code.

External side effects
  requests already sent to internet servers are not undone.
  keychain trust for the inspection CA stays until `proxy untrust` or the user
  removes it in Keychain Access. An older CLI has no untrust command for it.
  client settings (HTTPS_PROXY in a shell profile, Claude Code settings) stay
  and break every request of that client until the user removes them.
```
