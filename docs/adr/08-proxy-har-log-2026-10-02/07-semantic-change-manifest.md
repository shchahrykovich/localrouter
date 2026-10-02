# 6. Semantic Change Manifest

## 1. Manifest status

```text
Manifest status: PLANNED
```

Everything below is intended behaviour. Nothing is built.

## 2. Semantic change summary

```text
Artifacts
+ 1 background thread per daemon (har-writer), only while there are records to write
+ 1 built-in host name (proxy.localhost), with a second address (router.localhost/proxy-log/)
+ 7 viewer paths (/, /viewer.js, /viewer.css, /api/files, /api/entries, /files/<name>, /api/live)
+ 4 configuration fields (3 log, 1 lan_networks)
+ 1 status block (network)
+ 1 CLI command group: lan (state, allow, deny, forget)
+ 1 CLI command group: proxy log (state, on, off, limits, open, path)
+ 1 reserved host key for new routes and script rules (proxy)
~ 1 socket API result (get_proxy gains log), 2 types (Config, SetConfigParams)
~ 1 MCP tool description (get_proxy); MCP tools stay at 9
~ 3 agent texts (help.md, note.md, mcp.md)
~ 4 app places (Settings Proxy page, Settings Routing page, Proxy tab, right-click menu)
~ 1 connection check (listen.rs peer_allowed: + network of the interface)

Persistent data
+ 1 new file type: HAR 1.2 files in <logs>/proxy/, at most 5 per instance
+ 1 file: inspect-ca/bundle.pem (system roots + inspection CA, public only)

Runtime effects
+ 1 file APPEND path (one per proxied request)
+ 1 file CREATE path (roll)
+ 1 file DELETE path (prune past the 5th)
+ 1 file REPAIR path (at start)
+ 1 file READ path (viewer, streamed)
+ 1 event stream (live feed, only while a page is open)
+ 1 system table READ path (routing and ARP, per connection from another machine)
+ 1 config WRITE at the first start after the update (allow_lan -> lan_networks)
- 1 in-memory log write (requests to proxy.localhost are no longer recorded)

External effects
0

Destructive operations
1: prune deletes HAR files past the 5th (only files the writer named)

Migrations
1: allow_lan true with no lan_networks becomes the current network only

Unresolved effects
2 (bodies in the HAR; redaction of query parameters)
```

## 3. Source of truth

```text
BEFORE
  in-memory request log (RequestLog)
    role:       TEMPORARY, the only record of proxy traffic
    rebuilt by: nothing; lost at restart

AFTER
  in-memory request log
    role:       TEMPORARY, unchanged (no headers, no query: ADR 01 I10)

  <logs>/proxy/proxy-*.har
    role:       CANONICAL for proxy traffic history (the only copy with headers)
    rebuilt by: nothing; traffic cannot be replayed

  viewer pages and /api/files
    role:       DERIVED, read from the files and the writer's state at each request

  config.json
    role:       CANONICAL for the three log settings and lan_networks, as for every other setting

  the current network id
    role:       EXTERNAL, read from macOS routing and ARP tables at each check; never stored
```

## 4. Artifacts

```text
Code artifacts
+ libs/core/src/har/ (mod, entry, writer, viewer, viewer.html/.js/.css)
+ libs/core/src/secrets.rs (moved from scripts/lua_api.rs)
+ apps/menubar/Sources/LocalRouterKit/ProxyLog.swift
+ apps/daemon/src/network.rs, apps/cli/src/lan.rs
~ forward.rs, proxy.rs, routes.rs, tls.rs, config.rs, api.rs, help.rs
~ help.md, note.md, mcp.md
~ apps/daemon/src/daemon.rs
~ apps/cli/src/proxy.rs, apps/cli/src/mcp.rs
~ Api.swift, SettingsView.swift, ProxyView.swift, StatusItemController.swift, SettingsTree.swift

Infrastructure
~ the instance's logs folder: + subfolder proxy/

Persistent artifacts
+ up to 5 HAR files per instance, each up to proxy_log_file_mb MB

Configuration
+ proxy_log (default true)
+ proxy_log_file_mb (default 20, 1..200)
+ proxy_log_file_requests (default 5000, 100..1,000,000)
+ lan_networks (default empty; filled once from the current network when allow_lan was true)
~ allow_lan: now the main switch for the networks in lan_networks, no longer "every network"

API
~ API_VERSION 1.4 -> 1.5
~ GetProxyResult + log
~ Config, SetConfigParams + 4 fields
~ StatusResult + network
+ api/examples/set_config_proxy_log.request.json

Database entities:  0 (the project has none)
New socket methods: 0
New MCP tools:      0
External services:  0
```

## 5. Runtime effects

```text
WRITE (append)
type:                   file write
target:                 the current <logs>/proxy/proxy-*.har
operation:              one positioned write over the closing line, under the file lock
trigger:                a proxied request reaches its response headers, or a tunnel closes
cardinality:            one entry per proxied request, while the log is on
write_idempotent:       no (repeating it adds a second entry)
producer_deterministic: no (times, order of concurrent requests)
retention:              until the file is pruned (5 files)
destructive:            no
reversible:             no (the entry can be deleted only with the file)

WRITE (create)
type:                   file create
target:                 <logs>/proxy/proxy-YYYYMMDD-HHMMSS.har
trigger:                first entry after start or after the log is turned on; the current file is full or gone
cardinality:            one per roll
write_idempotent:       no (a second roll makes a second file)
producer_deterministic: no (the name holds the time)
destructive:            no

DELETE (prune)
type:                   file delete
target:                 HAR files past the 5 newest, by name, matching the pattern only
trigger:                every new file
cardinality:            0 or more per roll
write_idempotent:       yes (deleting a deleted file is a no-op)
producer_deterministic: yes (the 5 newest by name)
destructive:            yes
reversible:             no

WRITE (repair)
type:                   file truncate and write
target:                 the newest HAR file of an earlier run, when its last line does not parse
trigger:                daemon start
cardinality:            0 or 1 per start
write_idempotent:       yes (a repaired file is left as it is)
producer_deterministic: yes
destructive:            yes, for the cut last entry only (it was not valid JSON)

WRITE (CA bundle)
type:                   file replace, after one CALL of /usr/bin/security
target:                 inspect-ca/bundle.pem
trigger:                the inspection CA is made or changes; at start when the file is older than 30 days
cardinality:            rare
write_idempotent:       yes
producer_deterministic: no (the system roots change with macOS updates)
destructive:            no (it replaces a file the daemon made)

WRITE (LAN update)
type:                   config.json replace
target:                 lan_networks
trigger:                first start with allow_lan true and no lan_networks
cardinality:            once per installation
write_idempotent:       yes (a second start finds the field and does nothing)
producer_deterministic: no (depends on the network at that moment)
destructive:            no

READ
type:                   file read
target:                 the HAR folder listing and one file
trigger:                a GET to the viewer from a loopback peer
destructive:            no

EMIT
type:                   server-sent events
target:                 each open /api/live connection
trigger:                each written entry, a new file, the log turned off
cardinality:            one event per entry per open page; a slow page gets "lagged" and reloads

CALL (no longer)
type:                   in-memory log push
target:                 RequestLog
trigger:                a request to proxy.localhost
change:                 no longer pushed
```

## 6. Reads and writes

```text
READ
- config.json: proxy_log, proxy_log_file_mb, proxy_log_file_requests, secret_headers,
  allow_lan, lan_networks
- macOS interface list, routing table, ARP table (sysctl), per connection from another machine
- <logs>/proxy/ listing and files (viewer, prune, repair)

WRITE
- <logs>/proxy/proxy-*.har
- config.json (through set_config only, as today)

EXTERNAL READ:  none
EXTERNAL WRITE: none

New hidden dependency
- The viewer's "current file is complete JSON" depends on the writer never
  changing a byte before the closing line, and on the viewer reading the
  length under the writer's lock. A reader outside the daemon
  (jq, Finder Quick Look) can read a file in the middle of a write and see a
  cut last line. See risks.
```

## 7. Interfaces and events

```text
~ SOCKET get_proxy                     result + log
~ SOCKET get_config, set_config        + 3 fields
~ MCP    get_proxy                     description + log; result + log; env + 6 names
~ CLI    proxy env                     + SSL_CERT_FILE, REQUESTS_CA_BUNDLE, CURL_CA_BUNDLE,
                                         http_proxy, https_proxy, no_proxy
= MCP    other 8 tools                 unchanged; still 9 tools
+ CLI    proxy log [on|off|limits|open|path]
~ CLI    proxy, proxy on               + one "Log:" line
+ HTTP   proxy.localhost /, /viewer.js, /viewer.css, /api/files, /files/<name>, /api/live
+ HTTP   router.localhost/proxy-log/…  the same paths
+ EVENT  SSE entry, file, off, lagged (on /api/live)
~ ROUTE  register_route, set_script_rule refuse host key proxy
~ SOCKET status                        + network
+ CLI    lan [allow|deny|forget]
= MCP    status passes network through; no tool changes LAN access
```

## 8. External side effects

```text
External side effects: none
```

The writer and the viewer never open a network connection. The viewer page
loads nothing from the internet (CSP `default-src 'self'`).

## 9. Invariants

```text
I1.  With proxy_log off, a proxied request makes no HAR record: no header copy,
     no allocation, no queue send.
     enforced by: one atomic load before any copy; T4 (off: no record), T5.

I2.  Writing the log never delays or changes traffic: the network task only
     calls try_send; a full queue drops the record and counts it.
     enforced by: T5 (a stalled writer, 10,000 requests all complete, dropped > 0).

I3.  Every HAR file the writer leaves is valid HAR 1.2 JSON after each entry,
     and every /files/<name> answer is complete JSON.
     enforced by: T2 (parse after every append), T6 (read during writes), E1.

I4.  A header in the secret list is written as [redacted]; Proxy-Authorization
     is not written at all; a script rule's reveal_secrets never changes the file.
     enforced by: T3, E1.

I5.  The writer deletes only files in <logs>/proxy/ whose names match
     proxy-\d{8}-\d{6}(-\d+)?\.har, and keeps the 5 newest of them.
     enforced by: T2 (a foreign file and a sixth file).

I6.  A file holds at most proxy_log_file_requests entries and at most
     proxy_log_file_mb MB, except a file of one entry larger than the limit.
     enforced by: T2.

I7.  Requests to proxy.localhost are written neither to a HAR file nor to the
     in-memory log; requests to router.localhost are never written to a HAR file.
     enforced by: T6.

I8.  The viewer answers only loopback peers, also with allow_lan on.
     enforced by: T6 (a non-loopback peer gets 403).

I9.  The viewer serves only regular files in <logs>/proxy/ whose names match
     the pattern; no other path, no symbolic link.
     enforced by: T6 (.., %2e%2e, a link to ca.key, config.json).

I10. The viewer changes no state: only GET and HEAD, no CORS headers, CSP
     default-src 'self' on every answer.
     enforced by: T6.

I11. New routes and script rules refuse host key proxy; a saved route proxy is
     kept at load, wins over the viewer, and status names the problem.
     enforced by: T7.

I12. The MCP server lists exactly nine tools, and none changes log settings.
     enforced by: T11 (apps/cli/tests/mcp.rs).

I13. No text writes proxy.localhost as a fixed name; texts use {{PROXY_LOG_URL}}.
     enforced by: T12 (no_fixed_names.rs, NoFixedNamesTests.swift).

I14. Tests never write HAR files outside LOCALROUTER_HOME.
     enforced by: the folder comes from Paths; T2 and T8 use temp homes.

I15. A write error stops the writer without a retry loop; get_proxy reports the
     error; traffic is not affected.
     enforced by: T2 (a read-only folder), T4.

I16. A daemon run never appends to a file of an earlier run; at start it repairs
     the newest earlier file if its last line does not parse, before the viewer
     answers.
     enforced by: T2 (a file cut in the middle of a line).

I17. A connection from another machine to port 80 or 443 is accepted only when
     allow_lan is on and the network id of the interface it came in on is in
     lan_networks; an unknown network id is refused.
     enforced by: listen::peer_allowed; T14 (every case), M9, M10.

I18. The update never turns allow_lan true into "every network": lan_networks
     gets the current network or stays empty, also when the config write fails.
     enforced by: T16.

I19. The proxy port, TCP routes and the viewer stay this Mac only, whatever
     lan_networks says.
     enforced by: T6 (viewer), T14 (proxy port and TCP listeners are not
     passed to the LAN check).

I20. Recognising a network runs no program, asks for no permission (Location
     Services) and keeps no cache or watcher.
     enforced by: network.rs uses getifaddrs and sysctl only; T15; code review
     in task 14.

I21. With no proxied request for 30 seconds and no viewer page open, the log
     holds no thread, no open file, no queue and no live channel.
     enforced by: T17.

I22. No request to the viewer holds a whole HAR file in memory: files are
     streamed in 64 KB chunks and /api/entries reads at most one chunk plus
     the entries it returns.
     enforced by: T6 (a file larger than 1 MB arrives in chunks of at most
     64 KB; /api/entries on a 50 MB file).

I23. inspect-ca/bundle.pem holds the macOS system roots followed by the
     inspection CA, and no private key; proxy env never points SSL_CERT_FILE,
     REQUESTS_CA_BUNDLE or CURL_CA_BUNDLE at the inspection CA alone.
     enforced by: T8 (the bundle parses, has more than 100 certificates, ends
     with the inspection CA, has no "PRIVATE KEY"); E2.
```

## 10. Data impact

```text
Migration          one: allow_lan true without lan_networks becomes lan_networks = [the
                   current network] at the first start (change 4). The other new
                   fields get defaults (Config::parse).
Backfill           none: no history exists to fill from
Derived rebuild    none
Destructive        prune (past the 5th file); repair cuts a broken last line

New persistent data
+ HAR files, at most 5 x proxy_log_file_mb per instance (100 MB with defaults)

Expected growth
bounded by the limits; one entry is about 1 to 4 KB (headers), so 5,000
requests are about 5 to 20 MB

Retention
the 5 newest files; turning the log off keeps them; uninstall deletes them
(Uninstaller.removeFiles removes the logs folder)

Compatibility with existing readers
routes.json: unchanged. config.json: older daemons ignore unknown fields
(Config has no deny_unknown_fields). The app with a 1.4 daemon: the log block
is optional and the controls stay hidden.
```

## 11. Blast radius

```text
A user's saved route named "proxy"
dependency:   Daemon::load validates saved routes again
impact:       without care, the route is skipped at load
failure mode: SILENT: a working route disappears after an update. Prevented by
              I11 (kept at load, wins, status names it).

Disk space on a small Mac
dependency:   proxy_log on by default
impact:       up to 100 MB per instance, release and dev: 200 MB
failure mode: visible: the writer stops on "disk full" and reports it; traffic
              continues.

Time Machine and other backups
dependency:   ~/Library/Logs is backed up by default
impact:       HAR files with URLs (queries included) reach backups
failure mode: SILENT: a token in a query string is in a backup. Accepted risk
              (U2), named in the Settings text.

The Logs tab (in-memory log)
dependency:   the router pushed every request it answered
impact:       requests to proxy.localhost no longer appear
failure mode: none expected; a user who looks for the viewer's requests in Logs
              will not find them, by design.

Script rules (ADR 07)
dependency:   the secret header list moves to secrets.rs
impact:       scripts read the list from the new place
failure mode: a mistake in the move would change what scripts see; T3 and the
              existing scripts tests cover it.

Older menu bar app with a 1.5 daemon
dependency:   socket API, additive
impact:       none; the app ignores the log block
failure mode: none: the user cannot change the log from that app; the CLI can.

A phone that reached the Mac on another network before the update
dependency:   allow_lan meant every network
impact:       after the update, only the network at update time is allowed
failure mode: SILENT for the phone user: the connection closes with no page.
              The Mac side shows the status note and the Routing page.

A user on a VPN, or on a cable without a router
dependency:   the network id needs a router MAC
impact:       the network is unknown and cannot be allowed
failure mode: visible in Settings ("This network cannot be recognised");
              LAN access does not work there.

Confirmed unaffected: TCP routes, folder routes, the local CA and the
inspection CA, the updater, routes.json, script-rules.json, the MCP tool list.
```

## 12. Unresolved effects

```text
? U1. Bodies in the HAR

status:  REQUIRES_DECISION
effect:  response.content.text and request.postData in the HAR, so the DevTools
         Response tab shows the body
reason:  bodies hold prompts, API replies and personal data; they are large;
         a copy would need a tee on every body, as ADR 07's log rules do, with
         limits by class, by size and a total budget. Undecided: which classes,
         which size, on by default or not, how secrets inside bodies are handled.
blocks:  nothing in this ADR; a log script rule (ADR 07) records bodies today
outcome: an ADR of its own

? U2. Redaction of secret query parameters

status:  REQUIRES_DECISION
effect:  values of query parameters such as api_key, token, access_token,
         signature written as [redacted] in request.url and request.queryString
reason:  which names, whether a config list like secret_headers, and whether a
         redacted URL still works for "copy as cURL" in DevTools
blocks:  nothing; v1 writes URLs as they are and says so in Settings and help
outcome: a decision inside this ADR before release, or a later ADR
```

## 13. Risks

```text
- A reader outside the daemon (jq, an editor) can read the current file during
  a write and see a cut last line. The viewer is safe (length under the lock);
  the agent texts tell agents to read closed files or retry.
- The default is on: a user who turns the proxy on for one test gets headers of
  every proxied request on disk without reading the Settings text. The CLI and
  Settings say so at that moment (G2), but a user who never looks does not know.
- The 4,096-record queue drops records under a burst (a page with thousands of
  requests while the disk is slow). dropped shows it; the file then has gaps
  that look like missing requests.
- Time to the response headers is not the full request time. DevTools shows
  all of it as "waiting"; a slow download looks fast.
- A café attacker who copies the home router's MAC address is treated as the
  home network (change 4, accepted risk).
- A memory regression would not be seen by any functional test: T17 checks the
  idle state, but steady-state memory under load is only checked by hand (M11).
- A file of an earlier run without a count (entries: null) can be large; the
  viewer parses it whole in the browser. The 200 MB limit keeps this to a few
  seconds; a file from a build with other limits could be larger.
```

## 14. Rollback

```text
Code rollback
  Revert the commits. Sufficient on its own for traffic and routes: an older
  daemon ignores the new config fields and never touches the HAR folder.
  BUT: an older daemon reads allow_lan true as "every network" again. A user
  who allowed LAN access at home gets it in the café after a rollback. Turn
  allow_lan off before rolling back, or accept it.

Schema rollback
  Nothing to reverse: config.json fields stay and are ignored; routes.json
  is unchanged.

Data rollback
  HAR files already written stay in <logs>/proxy/. A rollback does NOT delete
  them. Delete the folder by hand, or uninstall (which deletes the logs folder).

Infrastructure rollback
  Nothing.

External side effects
  None were made. HAR files already copied by Time Machine stay in backups.

Not reversible
  - entries already written (they hold headers and URLs)
  - files deleted by prune
  - a broken last line cut by repair
```
