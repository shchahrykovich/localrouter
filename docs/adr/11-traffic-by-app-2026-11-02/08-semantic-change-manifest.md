# 8. Semantic change manifest

## 1. Manifest status

```text
Manifest status: PLANNED
```

## 2. Semantic change summary

```text
Artifacts
+ 1 background child process (nettop), owned by the daemon
+ 1 socket API method that reads (get_app_traffic)
+ 1 socket API method that downloads (update_geo_db)
+ 3 config fields
+ 2 windows (traffic map, app details), 1 popover view (Apps), 1 Settings page
+ 3 menu actions
+ 1 in-memory table in the forward proxy (peer_port)

Persistent data
+ 1 day file per day of history in <data>/traffic/
+ 2 database files in <data>/geo/
+ 1 cache file in the app's Caches folder
+ 1 Keychain item
+ 1 UserDefaults list (trusted programs)

Runtime effects
+ 1 file APPEND path (minute lines)
+ 1 file DELETE path (retention)
+ 1 file REPLACE path (geo databases)
+ 1 file REPLACE path (check cache)
+ 3 outbound HTTPS calls (db-ip download, VirusTotal lookup, VirusTotal upload)
+ 1 browser open with user data in the URL (Ask AI)
+ 1 signal to another process (Quit process, SIGTERM)

Modified
~ forward proxy, daemon status, Traffic tab, right-click menu, Settings

External effects
3 (db-ip download, VirusTotal lookup, VirusTotal upload) and 1 user-started browser open

Destructive operations
1 local (history retention), 1 to another program (SIGTERM, after a confirm)

Unresolved effects
7
```

## 3. Source of truth

```text
BEFORE
  no data about apps other than the request log (memory) and the HAR files

AFTER
  nettop output
    role:       EXTERNAL (macOS), read only, never stored raw
  <data>/traffic/YYYY-MM-DD.ndjson
    role:       CANONICAL for traffic history
    rebuilt by: nothing (lost history is lost)
  daemon memory buckets
    role:       CACHE of the last hour; rebuilt from nettop after a restart
                (the hour before the restart comes back only at minute grain,
                from the day file)
  <data>/geo/*.mmdb
    role:       REPLICA of DB-IP Lite; authority: db-ip.com
  app-checks.json
    role:       CACHE; rebuilt by new checks and lookups
  trusted list (UserDefaults)
    role:       CANONICAL; the user's decisions
  Keychain VirusTotal key
    role:       CANONICAL; the user's secret
```

## 4. Artifacts

```text
Processes
+ nettop child of localrouterd (only while traffic = true)

Interfaces
+ socket method get_app_traffic
+ socket method update_geo_db
~ socket method set_config: 3 fields
~ socket method status: field traffic (off | running | restarting) and traffic_error
~ API_VERSION 1.8 -> 1.9

Configuration
+ traffic: bool
+ traffic_history_days: 1 | 7 | 30
+ traffic_places: bool

Files (stores)
+ <data>/traffic/                     folder, mode 0700
+ <data>/traffic/YYYY-MM-DD.ndjson     one per day, mode 0600
+ <data>/geo/dbip-city-lite.mmdb       mode 0644
+ <data>/geo/dbip-asn-lite.mmdb        mode 0644
+ ~/Library/Caches/<bundle id>/app-checks.json
+ Keychain generic password, service by the instance rule (ADR 04)
+ apps/menubar/.../TrafficMap/land-dots.json   committed resource

App UI
+ Apps view, traffic map window, app details window, Settings -> Traffic
~ Requests view (switch), right-click menu (Traffic group), status icon (red dot)

Database entities: 0
MCP tools:         0 new (stays at nine)
CLI commands:      0 new
Events:            0 new socket events
```

## 5. Runtime effects

```text
CALL
type:                   child process
target:                 /usr/bin/nettop -L 0 -s 5 -x -n -d -J bytes_in,bytes_out
trigger:                daemon start with traffic = true, or set_config traffic -> true
cardinality:            one process; restarted with backoff 1 s .. 60 s
write_idempotent:       n/a
producer_deterministic: no (live system state)
destructive:            no

WRITE
type:                   file append
target:                 <data>/traffic/YYYY-MM-DD.ndjson
trigger:                end of each minute with traffic
cardinality:            one line per active (app, via, remote, port, proto) pair per minute
write_idempotent:       no (a repeated append doubles a minute)
producer_deterministic: no
retention:              traffic_history_days, and at most 500 MB in total
destructive:            no
reversible:             yes (delete the files)

DELETE
type:                   file delete
target:                 day files older than traffic_history_days, then the oldest above 500 MB
trigger:                daemon start, every hour, set_config of traffic_history_days, Clear history
write_idempotent:       yes
producer_deterministic: yes
destructive:            yes (history cannot be rebuilt)
reversible:             no

CALL + WRITE
type:                   external HTTPS GET, file replace
target:                 download.db-ip.com -> <data>/geo/*.mmdb
trigger:                update_geo_db (user turns on places, or presses Update)
cardinality:            2 files per call
write_idempotent:       yes
producer_deterministic: yes for one month's file
destructive:            replaces the previous month's file
reversible:             no (the old file is not kept); a new download restores a working file

CALL
type:                   external HTTPS GET
target:                 virustotal.com/api/v3/files/{sha256}
trigger:                a new non-Apple executable, lookups on, key saved; or Check again
cardinality:            at most 4 a minute, 500 a day
write_idempotent:       yes
producer_deterministic: no (VirusTotal results change over time)
destructive:            no

CALL
type:                   external HTTPS POST of a file
target:                 virustotal.com/api/v3/files
trigger:                Upload file to VirusTotal... and the confirm dialog
cardinality:            one file per confirm
write_idempotent:       yes for VirusTotal (same hash)
producer_deterministic: n/a
destructive:            no locally; the file is now shared with third parties
reversible:             no

CALL
type:                   open URL in the browser
target:                 claude.ai/new?q=... or chatgpt.com/?q=...
trigger:                Ask Claude, Ask ChatGPT
cardinality:            one per click
reversible:             no (the question is in the chat service)

CALL
type:                   signal SIGTERM
target:                 the pid of the app in the details window
trigger:                Quit process and the confirm dialog
destructive:            yes for the other program's unsaved state
reversible:             no
```

## 6. Reads and writes

```text
READ
- nettop output, proc_pidpath, config ports, routes (TCP listen ports)
- the forward proxy peer_port table
- <data>/traffic/*.ndjson (ranges longer than 1 hour)
- <data>/geo/*.mmdb
- executable files (signature, SHA-256)
- Keychain item, UserDefaults trusted list, app-checks.json

WRITE
- <data>/traffic/*.ndjson, <data>/geo/*.mmdb
- app-checks.json, Keychain item, UserDefaults trusted list

EXTERNAL READ
- download.db-ip.com, virustotal.com (lookup)

EXTERNAL WRITE
- virustotal.com (upload, only on confirm)
- claude.ai, chatgpt.com (the question, only on click)
```

Hidden dependency created: the Apps view depends on the text format of a
macOS tool. A new macOS can change it without notice (U4, and the risk
below).

## 7. Interfaces and events

```text
+ SOCKET get_app_traffic {range, app?}
+ SOCKET update_geo_db {}
~ SOCKET set_config: traffic, traffic_history_days, traffic_places
~ SOCKET status: traffic, traffic_error
~ API_VERSION 1.9 (minor: an app for 1.8 still works with a 1.9 daemon)
  MCP: unchanged, nine tools
  CLI: unchanged
  HTTP (router.localhost, proxy.localhost): unchanged
```

## 8. External side effects

```text
+ HTTPS GET download.db-ip.com
  first use: 2 files (size measured in task 0, U5)
  steady state: only when the user presses Update
+ HTTPS GET virustotal.com
  first day: one per non-Apple executable seen, capped at 500 a day
  steady state: new or changed executables, plus a re-check after 7 days
  cost: free with the user's own public API key
+ HTTPS POST virustotal.com: only on confirm; the file becomes visible to
  VirusTotal's partners
+ browser open of claude.ai / chatgpt.com with the question in the URL
```

## 9. Invariants

```text
I1.  With traffic = false the daemon has no nettop child and writes nothing in <data>/traffic/.
     checked by: T7 (daemon test: status.traffic = off, no child, no folder)

I2.  The daemon's own pid never appears as an app, and its bytes are never added to any app.
     checked by: T2 (fixture with localrouterd lines)

I3.  A loopback connection to the HTTP/HTTPS port or a TCP route's listen port is via = router;
     to a proxy port is via = proxy; every other remote is via = direct.
     checked by: T2

I4.  No day file older than traffic_history_days survives a retention pass, and the folder
     stays under 500 MB after one.
     checked by: T3

I5.  A failing or missing nettop never stops routing or the proxy; status shows restarting
     and the daemon retries with backoff up to 60 s.
     checked by: T8

I6.  An automatic VirusTotal call is a GET with no body: no file bytes leave the Mac
     without the confirm dialog.
     checked by: T10 (URLProtocol stub records every request)

I7.  An executable whose status is apple is never sent to VirusTotal.
     checked by: T10

I8.  The VirusTotal key exists only in the Keychain: never in config.json, UserDefaults,
     app-checks.json, a log or a socket reply.
     checked by: T10, M5

I9.  The upload function can only be called with an UploadConfirmation, whose initializer
     is private to the confirm dialog.
     checked by: the compiler, review checkpoint in task 7, M5

I10. The Ask AI question never contains the home folder path and the URL is under 2,000
     characters.
     checked by: T12

I11. Place lookups never use the network; only update_geo_db downloads, and only with the
     verifying TLS config.
     checked by: T5, T6

I12. A trusted mark applies to one SHA-256 only.
     checked by: T11

I13. The forward proxy's peer_port table is bounded (4,096 entries, oldest dropped) and is
     never awaited on the traffic path.
     checked by: T4

I14. The MCP server still has exactly nine tools.
     checked by: apps/cli/tests/mcp.rs (unchanged)

I15. Every new socket method has request and reply examples in api/examples/.
     checked by: libs/core/tests/api_examples.rs, ApiContractTests.swift (both walk the folder)

I16. An app for API 1.9 shows "The daemon is too old for the Apps view" instead of an empty
     list when the daemon answers 1.8.
     checked by: T14
```

## 10. Data impact

```text
New persistent data
+ day files of traffic history
+ two geo database files
+ check cache, trusted list, Keychain item

Migration          none
Backfill           none (history starts when the user turns it on)
Derived rebuild    check cache only
Destructive        retention deletes history; Clear history deletes all of it

Expected growth
  not measured yet. Task 3 runs the sampler for one day on the author's Mac and
  writes the measured size of one day file here. The 500 MB cap holds either way.

Retention
  1, 7 or 30 days, and 500 MB

Compatibility
  config.json: the fields are #[serde(default)] and left out when unset, so an
  older daemon reads the file and ignores them (config.rs has no
  deny_unknown_fields).
```

## 11. Blast radius

```text
Traffic tab (TrafficView.swift)
dependency:   the Apps view becomes the default
impact:       a user who reads the request list must press Requests
failure mode: none silent; the view remembers the last choice (UserDefaults)

App 1.9 with daemon 1.8 (an update half done)
dependency:   get_app_traffic does not exist in 1.8
impact:       the call fails with "unknown method"
failure mode: SILENT if the view shows "No traffic" instead of an error -> I16

Forward proxy (forward.rs)
dependency:   records peer_port for each client connection
impact:       a little memory per connection
failure mode: unbounded growth under many connections -> I13

Daemon status and Settings -> Daemon
dependency:   new fields traffic, traffic_error
impact:       none for older clients (extra fields are ignored)

-dev instance (ad-hoc signed)
dependency:   the Keychain item's access list names the app's signature
impact:       every new -dev build asks for Keychain access again
failure mode: not silent (a macOS prompt), but annoying -> risk below

Proxy log viewer, HAR files, routes, TCP routes, scripts, CLI, MCP
confirmed unaffected: their code paths do not read anything this ADR adds.
```

## 12. Unresolved effects

```text
? U1 Host names of direct connections
status:  REQUIRES_DECISION
effect:  show github.com instead of 140.82.121.4 for direct traffic
reason:  needs a Network Extension content filter (system extension, provisioning
         profile, Developer ID only, user approval). Not decided.
blocks:  nothing in this ADR; the UI says "addresses"
outcome: a new ADR

? U2 Blocking an app (the switches in Little Snitch)
status:  REQUIRES_DECISION
effect:  deny a connection
reason:  same filter as U1; also a product decision (LocalRouter is a dev tool)
blocks:  nothing in this ADR
outcome: a new ADR

? U3 Do claude.ai/new?q= and chatgpt.com/?q= fill the question?
status:  UNRESOLVED, checked in M0
blocks:  the final behaviour of the two buttons in task 8 (the copy fallback is built either way)

? U4 nettop output on macOS 14 and 15
status:  UNRESOLVED, checked in M0 (measured only on macOS 26)
blocks:  task 1 acceptance

? U5 Size and download time of DB-IP Lite city and ASN files
status:  UNRESOLVED, measured in M0
blocks:  the size line in the Settings text, and whether to ask before downloading

? U6 VirusTotal public API terms for a key used inside a free distributed app
status:  UNRESOLVED, a reading of the terms in task 0
blocks:  release of task 7 (not its build)

? U7 CPU and energy cost of nettop every 5 seconds
status:  UNRESOLVED, measured in M0
blocks:  the default interval
```

## 13. Risks

```text
- nettop is a tool, not an API. A new macOS can change its columns or its name
  truncation. The parser then reads zero rows and the view shows "no traffic":
  a silent failure. Mitigation in task 1: status.traffic_error reports
  "nettop output not understood" when a sample has lines but none parse.
- A pid can be reused between the sample and proc_pidpath. The window is
  milliseconds; a wrong path puts a few KB under the wrong app.
- The proxy join misses a connection that opened and closed between two
  samples and is older than 30 s in the peer table: its bytes stay "proxy,
  no host".
- History files list every address every app talked to. That is sensitive data
  on disk. Folder 0700, files 0600, and they are never logged or served over
  HTTP.
- A VirusTotal false positive can make a user quit or delete a good program.
  The two-engine threshold and the amber "1 engine" reduce it, not remove it.
- The -dev build asks for Keychain access after each rebuild (ad-hoc
  signature). Only a developer of LocalRouter meets it.
- A VPN app counts the bytes of other apps a second time: the apps' own
  connections and the VPN's tunnel to its server both appear in nettop. The
  Sent and Received totals are then too large. nettop does not name the
  interface, so the app cannot tell which bytes are a tunnel. The help text
  of the Summary says "Totals count a VPN's tunnel again" (accepted in 00,
  R4).
- Long ranges read day files. Without the per-minute cache of 1.7 the map
  window would read up to 30 files every 5 seconds.
```

## 14. Rollback

```text
Code rollback
  revert the commits. Sufficient for behaviour: no nettop, no new UI.

Config rollback
  the three fields stay in config.json and are ignored by an older daemon.

Data rollback
  delete <data>/traffic/ and <data>/geo/ by hand, or Clear history before the
  downgrade. The old daemon never reads them.

App data
  app-checks.json, the trusted list and the Keychain item stay until deleted.
  The Uninstaller (LocalRouterKit/Uninstaller.swift) must remove all three
  (task 9).

External side effects that cannot be undone
  - files uploaded to VirusTotal stay there and with its partners
  - questions sent to claude.ai or chatgpt.com stay in those accounts
  - hashes looked up on VirusTotal are known to VirusTotal
  - a program stopped with Quit process lost its unsaved state
```
