# 4. LAN access per network

## Context

"Allow LAN access" (`allow_lan`, ADR 01) is one switch for every network.
When it is on, any machine on the same network can reach ports 80 and 443 and
through them the user's dev servers. A user who turns it on at home to test on
a phone also opens the dev servers in a café, on a train, at a conference.

The user asked: remember networks, and allow LAN access only on the networks
where the user allowed it.

What this protects, so the change is not read as more than it is: it stops
**other machines on an untrusted network from connecting to this Mac** on
ports 80 and 443. It does not change what happens to the user's own traffic
on that network. The CA risk (a stolen CA key used to impersonate sites) is a
different threat and is not affected.

![LAN access per network](diagrams/08-lan-per-network.svg)

## How a network is recognised

macOS hides the Wi-Fi name without Location Services permission. Checked on
2026-10-02 on this project's Mac:

| Source | Value without permission |
|---|---|
| Wi-Fi name (SSID), BSSID (`ipconfig getsummary en0`) | `<redacted>` |
| `networksetup -getairportnetwork en0` | "You are not associated with an AirPort network." |
| Default router (`route -n get default`) | `192.168.0.1` on `en0` |
| Router MAC address (ARP table) | `18:35:d1:15:d1:a8` |

**Decision: a network is its router's MAC address.** The network id is
`mac:18:35:d1:15:d1:a8`. It needs no permission, it works the same for Wi-Fi
and Ethernet, and it is harder to copy than a Wi-Fi name: anyone can name a
café network "Home", but to match the id they must also know and copy the
home router's MAC address. That is possible, and it is an accepted risk (below).

The daemon never asks for Location Services. It is a LaunchAgent without UI.
The user gives each network a name ("Home", "Office").

**Which network a connection belongs to.** A Mac can be on several networks
at once (Wi-Fi and Ethernet). The daemon uses the interface the connection
came in on:

1. `local_addr()` of the accepted socket gives the local IP, for example
   `192.168.0.23`.
2. The interface list (`getifaddrs`) gives the interface of that IP: `en0`.
3. The routing table (`sysctl` `NET_RT_DUMP`) gives the default router of
   `en0`: `192.168.0.1`.
4. The ARP table (`sysctl` `NET_RT_FLAGS`) gives its MAC address.

**As built (D1 in [plan versus actual](10-plan-vs-actual.md)).** Steps 3 and
4 read the macOS System Configuration store instead of the routing and ARP
tables: `State:/Network/Service/*/IPv4` has `InterfaceName`,
`ARPResolvedIPAddress` and `ARPResolvedHardwareAddress`. On macOS 26 the ARP
table is empty for a program without Local Network access; the store is not.

No step runs a program, and nothing is cached. The lookup runs only for a
connection from another machine while `allow_lan` is on, which is rare. A
connection from this Mac is accepted before any lookup, as today. This follows
the resource rule in [change 1](01-har-files.md#resource-use): no watcher
thread and no cache that lives longer than one connection check.

**Unknown network.** No router, no ARP entry, a VPN interface (`utun`), or an
IPv6-only network: the id is unknown, and the connection is refused. An
unknown network cannot be allowed.

## Settings

`config.json`:

| Field | Meaning |
|---|---|
| `allow_lan` | stays: the main switch. Off means this Mac only, on every network. |
| `lan_networks` | new: the allowed networks, `[{ "id": "mac:18:35:d1:15:d1:a8", "name": "Home", "router": "192.168.0.1" }]` |

A connection from another machine is accepted only when `allow_lan` is on
**and** the id of its network is in `lan_networks`. In the list means allowed;
"Forget" removes it.

**Update from an older version.** A `config.json` with `allow_lan: true` and
no `lan_networks` field means "everywhere" today. At the first start after the
update the daemon adds the network the Mac is on at that moment (if it is
known) to `lan_networks`, writes `config.json`, and status shows a note:
"LAN access now works per network. Allowed on: 192.168.0.1 (router
18:35:…). Name it or add other networks in Settings → Routing." If the network
is unknown at that moment, the list stays empty and the note says so. The
daemon never turns an old `allow_lan: true` into "everywhere".

## Clients

| Client | Change |
|---|---|
| Socket API (1.5, same bump as change 3) | `Config` + `lan_networks`; `SetConfigParams` + `lan_networks` (the whole list, as `inspect_hosts`); `status` + `network`: `{ "id", "name", "router", "interface", "lan_allowed" }` for the network of the default route, `null` when unknown |
| CLI | `lan` (current network, allowed or not, the list), `lan allow [--name Home]` (the current network), `lan deny` (the current network), `lan forget <name or id>` |
| MCP | nothing new. The `status` tool passes the `network` block through. Changing LAN access is not an MCP action: an agent must not open the user's dev servers to a network. |
| App, Settings → Routing | the "Allow LAN access" switch stays; under it: "This network: Home (router 192.168.0.1). Allowed." with "Allow on this network" or "Stop allowing"; the list of allowed networks with name, router, "Rename", "Forget"; the text "On other networks only this Mac can reach ports 80 and 443." |

The status is read when the Settings page opens; nothing watches the network
in the background. The page has a "Check again" button for a user who just
changed networks.

## What does not change

- The proxy port, TCP routes and the viewer stay this Mac only, whatever
  `lan_networks` says (ADR 06 I-rules, [change 2](02-viewer.md#who-can-read-it)).
- Binding: ports 80 and 443 still bind `0.0.0.0` and `[::]`, and every
  connection is still checked before a byte is read (`listen.rs`).
- A LAN connection already open when the Mac moves to another network is not
  closed. Moving networks usually drops it anyway; the next connection is
  checked.

## Accepted risks

- A café attacker who knows the home router's MAC address can copy it and be
  treated as home. They must also have `allow_lan` on the user's side and
  target this Mac on purpose.
- Two places with the same router model are different networks (MAC
  addresses differ), so a user must allow each one.

## Tests

T14 (the decision for each case), T15 (network id from routing and ARP data),
T16 (daemon: `lan_networks`, `status.network`, the update from `allow_lan:
true`), M9 (home and phone hotspot), M10 (Wi-Fi and Ethernet at once). See the
[test plan](08-test-plan.md).
