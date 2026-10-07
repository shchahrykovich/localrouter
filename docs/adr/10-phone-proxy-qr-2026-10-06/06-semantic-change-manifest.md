# 6. Semantic Change Manifest

## 1. Manifest status

```text
Manifest status: PLANNED
```

Everything below is intended behaviour. Effects that no decision covers yet
are in section 12, not in the planned effects.

## 2. Semantic change summary

```text
Artifacts
+ 1 kind of listener: a proxy port on 0.0.0.0 and [::] (one per LAN client)
+ 1 file artifact: proxy-passwords.json (mode 0600)
+ 3 HTTP answers on a LAN client port: the setup page, the phone profile
  (Wi-Fi proxy and CA), the CA profile alone (+1 PAC file if M0 picks it)
+ 2 HTTP refusals: 407 (no or wrong password), 403 (the Mac's own address)
+ 1 socket API method: new_proxy_password (API 1.7)
+ 4 CLI commands: proxy client add --lan, proxy client setup, proxy client password --new, lan name
+ 1 UI panel: Proxy tab, Phone…
+ 1 macOS permission the app may ask for: Location Services (to read the Wi-Fi name)
~ 2 configuration fields: ProxyClient.lan, LanNetwork.wifi_name
~ 1 reply: get_proxy gains lan for a LAN client
~ 1 MCP tool reply: get_proxy strips the password
~ 4 texts: help.md, note.md, mcp.md, Settings → Routing

Persistent data
+ 1 password per LAN client (16 characters)

Runtime effects
+ 1 file WRITE path (proxy-passwords.json)
+ 1 network accept path from other machines on the proxy (LAN client only)
+ 1 network lookup caller (lan_peer_allowed, now also for the phone port)
+ outbound traffic for another device (the phone) through this Mac

Modified
~ 4 existing components: forward proxy, upstream connect, daemon proxy start, Proxy tab

External effects
+ the phone's internet traffic leaves through this Mac

Destructive operations
0 (removing a LAN client deletes its password; it is random and replaceable)

Unresolved effects
7 (four of them are answered by M0 before the build starts)
```

## 3. Source of truth

```text
BEFORE
  config.json          CANONICAL for proxy clients (ADR 09)

AFTER
  config.json          CANONICAL for proxy clients, including lan
  proxy-passwords.json CANONICAL for the password of each LAN client
                       rebuilt by: nothing; a lost file means a new password

DERIVED
  setup_url            derived from: the interface address + the client port + the password
                       derived by: the daemon, on each get_proxy (never stored)
  the QR image         derived from: setup_url, by the app, never stored
  phone.mobileconfig   derived from: the password, wifi_name, the address, inspect-ca/ca.pem;
                       built on each request, never stored on the Mac
  ca.mobileconfig      derived from: inspect-ca/ca.pem, on each request, never stored
  wifi_name            CANONICAL in config.json; first read by the app from CoreWLAN or typed

EXTERNAL
  the iPhone's installed phone profile  a copy of the address, the port, the password, the
                                       Wi-Fi name and the CA; LocalRouter cannot read or
                                       change it; scanning again replaces it
  (manual way) the typed proxy setting a copy the user typed
```

The two external copies are the source of most user problems (section 11):
LocalRouter changes its side and the phone keeps the old values.

## 4. Artifacts

```text
Listeners
+ phone port: 0.0.0.0:<port> and [::]:<port>, one per LAN client, while the proxy is on
~ main proxy port and loopback clients: unchanged, 127.0.0.1 and ::1 only

Files
+ <data folder>/proxy-passwords.json   {"iphone": "k7mq-2xph-9tdw-r4nc"}   mode 0600
~ <data folder>/config.json            proxy_clients[].lan, lan_networks[].wifi_name

HTTP answers (LAN client port only)
+ GET /setup/<password>                     the setup page
+ GET /setup/<password>/phone.mobileconfig  the phone profile: one Wi-Fi payload with the
                                            proxy and its password, and the CA if inspecting
+ GET /setup/<password>/ca.mobileconfig     the CA alone, for the manual way
+ GET /setup/<password>/proxy.pac           only if M0 picks the Auto proxy (U2)
+ 407 Proxy Authentication Required
+ 403 for a target that is this Mac

Socket API (1.6 → 1.7, additions only)
+ new_proxy_password
~ ProxyClient.lan, GetProxyResult.lan

CLI
+ proxy client add <name> --lan
+ proxy client setup <name>
+ proxy client password <name> --new
+ lan name <wifi name>

App
+ Proxy tab → Phone… panel with the QR code
+ NSLocationUsageDescription in Info.plist; CoreWLAN read after the user allows it

Database entities: 0
Background workers: 0 (no watcher, no timer; the network is read per connection, as ADR 08)
MCP tools: 0 new (still nine)
```

## 5. Runtime effects

```text
BIND
type:                   listening sockets
target:                 0.0.0.0:<port>, [::]:<port>
trigger:                proxy on with a LAN client, daemon start, set_config adds a LAN client
cardinality:            two sockets per LAN client
write_idempotent:       yes (a bound port is not bound twice)
producer_deterministic: yes
destructive:            no
reversible:             yes (proxy off, client removed)

WRITE
type:                   file replace
target:                 proxy-passwords.json
operation:              CREATE / REPLACE (temp file, rename)
trigger:                a client becomes LAN; new_proxy_password; a LAN client at start with no password; removal of a client
cardinality:            one file, one entry per LAN client
write_idempotent:       no (each write makes a new random password)
producer_deterministic: no (random)
retention:              until the client is removed
destructive:            yes for the old password, by intent
reversible:             no (the old password cannot come back; a new one is made)

READ
type:                   System Configuration store (network id)
trigger:                each connection to a LAN client port from another machine
cardinality:            one lookup per such connection, off the accept loop
write_idempotent:       n/a
producer_deterministic: yes for one network state

EMIT
type:                   HTTP response 407 / 403
trigger:                a request from another machine with no or wrong password / to this Mac's own address
cardinality:            one per refused request; logged with _client, without the password

CALL
type:                   outbound connection to the internet
trigger:                each request from the phone that passes the checks
cardinality:            as many as the phone's apps make; iOS apps make background requests all the time
write_idempotent:       depends on the request (not the proxy's choice)
producer_deterministic: n/a

WRITE
type:                   HAR entries and request log entries
target:                 the existing proxy log files
trigger:                each phone request (not setup requests)
cardinality:            one entry per request, _client: "<name>"
retention:              the existing roll and prune (ADR 08)
```

## 6. Reads and writes

```text
READ
- config.json in memory: allow_lan, lan_networks, proxy_clients
- proxy-passwords.json (at start, then in memory)
- the System Configuration store (network id; ADR 08 code)
- the interface list (getifaddrs): the Mac's own addresses, the setup URL address
- inspect-ca/ca.pem (the profiles)
- lan_networks[].wifi_name (the phone profile)
- CoreWLAN, in the app only, after Location Services permission

WRITE
- proxy-passwords.json
- config.json (existing set_config path)
- the request log and the HAR log (existing paths, new client)

EXTERNAL READ
- none (the phone's settings cannot be read)

EXTERNAL WRITE
- none by LocalRouter directly; the user installs the profile, and iOS writes the
  Wi-Fi network's proxy settings and the CA from it
```

New hidden dependency: **the phone's internet on one Wi-Fi network now
depends on this Mac being awake, on the same network, at the same address,
with the proxy on.** Nothing in LocalRouter can see that dependency.

## 7. Interfaces and events

```text
+ SOCKET  new_proxy_password {client} -> {password, setup_url}
~ SOCKET  set_config.proxy_clients[].lan, get_config, status.proxy.clients[].lan
~ SOCKET  set_config.lan_networks[].wifi_name, status.network.wifi_name
~ SOCKET  get_proxy {client}: lan {address, port, username, password, setup_url, profile, problems}
~ MCP     get_proxy: lan.password and lan.setup_url removed; text points to the app
+ CLI     proxy client add --lan, proxy client setup, proxy client password --new, lan name
+ HTTP    GET /setup/<password>, …/phone.mobileconfig, …/ca.mobileconfig, (…/proxy.pac)
+ HTTP    407 with Proxy-Authenticate: Basic (LAN client port, non-loopback peer)
+ HTTP    403 for the Mac's own addresses (LAN client port, non-loopback peer)
  Events: none
```

## 8. External side effects

```text
+ outbound internet traffic on behalf of another device
  scale: the phone's whole traffic on that Wi-Fi; an idle iPhone still makes
         background requests (iCloud, app refresh, telemetry)
  cost:  none in money; HAR log space (section 10)
```

## 9. Invariants

```text
I1.  The main proxy port and every proxy client without lan bind 127.0.0.1 and ::1
     only, also with allow_lan on (ADR 06 I1 keeps holding).
     enforced by: bind_proxy for them; T6

I2.  A LAN client port serves a peer that is not loopback only when allow_lan is on
     and the peer's network id is in lan_networks. The decision is made before any
     byte is read.
     enforced by: the accept loop calls lan_peer_allowed; T5, T6, E1

I3.  On a LAN client port, a request from a non-loopback peer without the right
     Proxy-Authorization gets 407 and makes no upstream connection, no route lookup,
     and runs no script rule.
     enforced by: the check is the first step of ForwardProxy::handle; T3, E1

I4.  The password compare looks at every byte whatever the first difference is.
     enforced by: phone::same_password, written as one fold over all bytes, and a review
     checkpoint in task 1; T3 checks wrong-length and wrong-last-char refusals

I5.  On a LAN client port, for a non-loopback peer, a target that resolves to a
     loopback address (127.0.0.0/8, ::1, ::ffff:127.0.0.0/104), an unspecified
     address, or an address of this Mac's interfaces gets 403, and no connection is
     made to it. .localhost names go to the route table and are not refused.
     enforced by: upstream::is_local_target on the resolved addresses; T4, E1, M5

I6.  The password is never in config.json, get_config, status, the request log, the
     HAR log or the daemon's log file.
     enforced by: the separate file; Proxy-Authorization left out of HAR (ADR 08 I4);
     setup requests not logged (I9); T7, T10

I7.  proxy-passwords.json is created with mode 0600 and replaced keeping it.
     enforced by: store::replace with mode; T7

I8.  The MCP get_proxy reply never contains lan.password or lan.setup_url.
     enforced by: the MCP tool removes the fields; T15

I9.  A request for /setup/<right password>... is in neither the request log nor the
     HAR log. A wrong /setup/... is logged with the path cut to /setup/….
     enforced by: the setup answer comes before seen()/log_http; T10

I10. The phone profile holds exactly one Wi-Fi payload (the Wi-Fi name, auto-join, and
     the proxy fields: server, port, user name, password, or the PAC URL) and at most
     one com.apple.security.root payload (only with the inspection CA and a non-empty
     inspect list). The CA profile holds only the root payload. Neither holds a private
     key. Neither holds a Wi-Fi password unless M0 decides U5 so.
     enforced by: phone::phone_profile, phone::ca_profile; T9

I11. new_proxy_password closes every open connection of that client before it
     returns; the old password gets 407 afterwards.
     enforced by: the per-client connection token; T7

I12. A LAN client is never served without a password: a missing password at start is
     made new before the port binds.
     enforced by: start_saved_proxy order; T7

I13. /setup/... is answered only on a LAN client port. On the main port and on a
     loopback client it is the 400 page, as today.
     enforced by: ClientPort.lan; T8

I14. Only the menu bar app asks for Location Services, and only when the user presses
     "Use My Location". The daemon and the CLI never do; a typed Wi-Fi name works
     without the permission.
     enforced by: one call site in WifiName.swift; T12; M1

I15. A phone profile is never built without a Wi-Fi name: with no wifi_name the page
     offers only the manual way, and get_proxy says profile: false.
     enforced by: phone_profile takes the name as a required value; T8, T12
```

## 10. Data impact

```text
New persistent data
+ proxy-passwords.json: under 100 bytes per LAN client

Changed persistent data
~ config.json: proxy_clients[].lan (omitted when false), lan_networks[].wifi_name (omitted when unset)

Migration:        none (a missing lan means false; a missing wifi_name means unknown)
Backfill:         none
Derived rebuild:  none
Destructive:      none of user data; a new password replaces the old by intent

Expected growth
- the HAR log fills faster: the phone adds its whole traffic. The 5 files and
  their limits are shared by every client (ADR 09 trade-off), so Chrome and agent
  entries are pruned sooner. Not measured yet; M2 records a 10-minute count.

Compatibility with older readers
- an older daemon ignores lan (Config::parse merges known fields) and binds the
  client on loopback only: the phone fails, nothing else does.
- an older app ignores the new fields (Swift Codable ignores unknown keys).
```

## 11. Blast radius

```text
The iPhone on that Wi-Fi network
  dependency:   the installed profile points its proxy at this Mac
  impact:       no internet on that Wi-Fi when the Mac sleeps, the proxy is off,
                the Mac gets a new address, or the client is removed. With the Auto
                proxy and the fallback (U2, if M0 confirms it) the first two go direct.
  failure mode: SILENT for apps (they just fail); Safari shows a proxy error.
                The user often does not connect it to LocalRouter days later.

The iPhone's saved Wi-Fi network
  dependency:   the profile describes the whole network, not only its proxy
  impact:       installing replaces the saved network's settings; removing the profile
                may also forget the network (U6)
  failure mode: visible: the phone leaves the Wi-Fi and asks for its password

The user's trust in the app
  dependency:   the Location Services request for the Wi-Fi name
  impact:       "why does a proxy want my location?"
  failure mode: visible; the user may deny it and type the name instead (I14)

The iPhone's HTTPS
  dependency:   inspect_hosts ["*"] by default, and the CA trust on the phone
  impact:       every HTTPS site fails until the profile is installed and trusted;
                apps that pin certificates always fail while inspected
  failure mode: LOUD in Safari (certificate warning), SILENT in apps
                ("cannot connect")

The proxy log (HAR) and the viewer
  dependency:   one shared set of files (ADR 09)
  impact:       the phone's background traffic pushes other clients' entries out
  failure mode: SILENT: the entry a user looks for is gone

Lua script rules (ADR 07)
  dependency:   a rule matches by host, not by client (ADR 09 trade-off)
  impact:       an intercept rule written for Chrome also changes the phone's traffic
  failure mode: SILENT

Ports 80 and 443
  dependency:   the checklist's "Allow" button sets the same lan_networks
  impact:       allowing the network for the phone also opens the routes to that network
  failure mode: visible: the panel and Settings → Routing say it

The macOS firewall
  dependency:   "Block all incoming connections" or a denied localrouterd
  impact:       the phone cannot connect; the same is true for ports 80 and 443 today
  failure mode: SILENT on the Mac; the phone shows a connection error

A guest Wi-Fi with client isolation
  dependency:   the phone must reach the Mac's address
  impact:       the setup page does not open; the proxy does not work
  failure mode: SILENT on the Mac

A rollback to a daemon before this ADR
  dependency:   proxy_clients[].lan is ignored
  impact:       the phone port binds loopback only
  failure mode: SILENT for the phone (no internet on that Wi-Fi)

Confirmed unaffected: the route table, TCP routes, the local CA and the inspection
CA files, the viewer's loopback-only rule (ADR 08 I8), the script engine, every
existing socket method's shape, the nine MCP tools.
```

## 12. Unresolved effects

```text
? U1. *.localhost names from the phone
status:  REQUIRES_VERIFICATION
effect:  the phone opens https://shop.localhost through the proxy
reason:  it is not known whether iOS Safari sends *.localhost names to the proxy
         or resolves them on the phone itself. If it sends them, the profile could
         also carry the local CA so routes open over HTTPS.
blocks:  adding the local CA to the profile; nothing in the tasks
outcome: M6 records what iOS does; then a decision in this ADR

? U2. Automatic proxy (PAC) with a fallback to no proxy
status:  REQUIRES_VERIFICATION
effect:  the phone keeps its internet when the Mac is away
reason:  the Wi-Fi payload takes ProxyType Auto, ProxyPACURL and
         ProxyPACFallbackAllowed. With the Mac away the PAC URL does not answer, and
         the fallback should send the phone direct. Not known: whether iOS sends the
         proxy password with Auto, and how long a request waits before the fallback.
blocks:  the ProxyType value (task 1). Manual stays the default until M0 shows Auto works.
outcome: M0; if it works, Auto becomes the default and the "no internet while the
         Mac sleeps" texts go away

? U3. A tunnel-only phone (no inspection for this client)
status:  REQUIRES_DECISION
effect:  the phone works without installing a CA
reason:  inspect_hosts is one list for every client (ADR 06). A per-client switch
         changes the inspect decision in forward.rs.
blocks:  nothing
outcome: decide after M3, if users skip the CA step

? U4. iOS sends the proxy password from the profile, also for CONNECT
status:  REQUIRES_VERIFICATION
effect:  HTTPS from the phone with a password
reason:  expected from the Wi-Fi payload's ProxyUsername and ProxyPassword, not yet
         seen on a device.
blocks:  every task. If iOS does not send it for CONNECT, change 1's password step
         needs another design.
outcome: M0, before task 1

? U5. A Wi-Fi payload without the Wi-Fi password, for a network the phone knows
status:  REQUIRES_VERIFICATION
effect:  the profile sets the proxy without LocalRouter knowing the Wi-Fi password
reason:  the payload describes the whole network. Not known whether iOS keeps the
         saved password, asks for it, or fails to join.
blocks:  task 1 (the payload's fields). If iOS needs it, the panel asks for the Wi-Fi
         password once, and where it is kept (proxy-passwords.json, mode 0600) is
         decided here before building.
outcome: M0

? U6. Removing the profile forgets the Wi-Fi network
status:  REQUIRES_VERIFICATION
effect:  "stop" also means "join the Wi-Fi again"
reason:  networks installed by a profile are usually removed with it.
blocks:  the text of step 4 of the setup page (task 1)
outcome: M0

? U7. The Mac's Bonjour name as the proxy server
status:  REQUIRES_VERIFICATION
effect:  the profile uses <name>.local instead of the IPv4 address, so a new address
         does not break the phone
reason:  not known whether iOS resolves .local names for the Wi-Fi proxy, and how fast.
blocks:  the ProxyServer value (task 1)
outcome: M0
```

## 13. Risks

```text
- The password travels in clear text on the Wi-Fi with every request (Basic). WPA2/3
  protects it from other devices on a protected network, not on an open one.
  (Invariant I2 limits the networks; it does not encrypt.)
- The QR code on screen is the password. Screen sharing or a photo gives access
  until "New Password".
- The profile file holds the password. A user who sends it to a colleague (AirDrop)
  gives them the proxy; "New Password" is the answer here too.
- iOS shows "Not Verified" for the unsigned profile; users learn to tap through that
  warning, which is a bad habit for other profiles.
- A copied router MAC (ADR 08 accepted risk) now also reaches the phone port; the
  password still stops the request (I3).
- The own-address list is read per request; an interface that comes up between the
  read and the connect is not in it. Narrow window; the target must be on that
  interface.
- The phone's traffic is the user's private traffic (messages, banking); with
  inspect_hosts ["*"] it is read and written into HAR files with cookies as they
  are (ADR 08). The setup page and the panel say that HTTPS from the phone is read.
```

## 14. Rollback

```text
Code rollback
  Revert the commits. Sufficient for the Mac: the old daemon ignores lan and binds
  the client on loopback. NOT sufficient for the phone: see below.

Schema rollback
  Nothing to reverse. lan is an extra field; the old daemon drops it the next time
  it writes config.json.

Data rollback
  proxy-passwords.json stays (mode 0600, unused). Delete it by hand if wanted.

Infrastructure rollback
  Nothing.

External side effects (NOT undone by any rollback)
  - The phone keeps the profile: its proxy points at the Mac (no internet on that
    Wi-Fi with Manual) and it trusts the inspection CA until the user removes the
    profile. Removing it may also forget the Wi-Fi network (U6).
  - A reset of the inspection CA on the Mac does not remove the old CA from the phone.
  - HAR entries already written stay until pruned.
```
