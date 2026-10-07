# 7. Test plan

This ADR is proposed. Every test below is to be written; none exists yet.

**M0 runs first, before any code.** It answers U4, U5, U6, U7 and U2 on a
real iPhone with a hand-made profile and a test proxy. Its results decide the
fields of the phone profile (task 1), and U4 can stop the whole design.

## What the repo can run today

| Kind | Tool | Where |
|---|---|---|
| Rust unit tests | `cargo test` | `#[cfg(test)]` modules; daemon unit tests live in the binary crate |
| Rust integration tests | `cargo test` | `libs/core/tests/forward.rs` (a `Harness` with a real `ForwardProxy`, test upstreams on loopback), `apps/daemon/tests/api.rs` (a real `localrouterd` over the socket, `LOCALROUTER_HOME` in a temp dir, ports `0`) |
| CLI and MCP tests | `cargo test -p localrouter` | `apps/cli/tests/cli.rs`, `mcp.rs`, `e2e.rs` (journey E1 of ADR 01) |
| Contract tests | `cargo test`, `swift test` | `libs/core/tests/api_examples.rs`, `ApiContractTests.swift`, both walk `api/examples/` |
| Swift unit tests | `swift test --package-path apps/menubar` | `LocalRouterKitTests` only; no UI tests of the views |
| iPhone | none | no device or simulator automation in the repo; every phone check is manual |

Debug builds read `LOCALROUTER_TEST_NETWORK` (ADR 08) to fake the current
network. No test can make a connection from another machine.

## A finding from planning

**The 403 rule would block every success test on a LAN client.** Every test
upstream in `forward.rs` binds `127.0.0.1` (`echo_server`, `tls_server`). A
request from a LAN peer to that address is exactly what change 1 step 5
refuses. Two sides:

| Side | What it does |
|---|---|
| The real rule (change 1 step 5) | refuses a target that resolves to `127.0.0.0/8`, `::1` or an own address |
| The test harness ([forward.rs:251](../../../libs/core/tests/forward.rs)) | starts every upstream on `127.0.0.1` |

So the rule is a value the `ForwardProxy` holds (`is_local_target`), and the
success tests of T3 give it a rule that allows the test upstream's port. That
replaced rule is tested on its own (T4), and with the real rule from start to
end only in E1 (a `.localhost` route, which the rule allows) and M5.

A second gap of the same kind: `Harness` takes the peer from the real
`accept` ([forward.rs:334](../../../libs/core/tests/forward.rs)), which is
always `127.0.0.1`. The LAN tests need a `fake_peer` on the harness
(`192.168.0.23:50000`). The real accept path with a non-loopback peer runs in
E1 only.

## Overview

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| T1 | unit | `ProxyClient.lan`: default false, left out when false, round trip | `config.rs` | none | `cargo test -p localrouter-core --lib config::tests` |
| T2 | unit | password format, two calls differ, the compare | `phone.rs` | none | `cargo test -p localrouter-core --lib phone::tests` |
| T3 | integration | 407 without or with a wrong password, for HTTP, `CONNECT` and `.localhost`; success with the right one | `ForwardProxy` | the peer (`fake_peer`), the local-target rule (allows the test upstream) | `cargo test -p localrouter-core --test forward lan_client` |
| T4 | unit + integration | `is_local_target` for every address class; 403 and no connection for `127.0.0.1`, `::ffff:127.0.0.1`, an own address | `upstream.rs`, `ForwardProxy` | the own-address list (passed in) | `cargo test -p localrouter-core --test forward local_target` |
| T5 | unit | the accept decision of a proxy port: loopback, LAN client, plain client | `listen.rs` | the network id (passed in) | `cargo test -p localrouterd --bin localrouterd listen::tests` |
| T6 | integration | binding: a LAN client on `0.0.0.0`/`[::]`; the main port and a plain client on loopback, also with `allow_lan` | `localrouterd` | none | `cargo test -p localrouterd --test api lan_client_binds` |
| T7 | integration | the password file (`0600`, life), `get_proxy.lan`, `new_proxy_password` closes a tunnel, restart makes a missing password, no reply or log holds the password | `localrouterd` | the network (`LOCALROUTER_TEST_NETWORK`) | `cargo test -p localrouterd --test api lan_client` |
| T8 | integration | the setup page on a LAN client; 400 for a wrong password, on the main port and on a plain client; the headers | `ForwardProxy` | the peer | `cargo test -p localrouter-core --test forward setup_page` |
| T9 | unit | the phone profile: one Wi-Fi payload with the proxy fields, the CA payload only when inspecting, no key, no Wi-Fi password; the CA profile: one root payload; both parse with `plutil` | `phone.rs`, `/usr/bin/plutil` | none | `cargo test -p localrouter-core --lib phone::tests::profile` |
| T10 | integration | setup requests are in neither log; a wrong one is logged as `/setup/…`; no HAR entry holds the password | `ForwardProxy`, `HarLog` | the peer | `cargo test -p localrouter-core --test forward setup_not_logged` |
| T11 | unit + integration | the setup URL from the interface address; `null` with a reason when the network is unknown | `network.rs`, `localrouterd` | the network and its address (`LOCALROUTER_TEST_NETWORK` gets a 4th field) | `cargo test -p localrouterd --test api setup_url` |
| T12 | unit (Swift) | the panel's checklist state for each case, the Wi-Fi name row included; the QR image decodes back to the setup URL | `PhoneSetup.swift`, Core Image | the status (decoded from JSON), the Wi-Fi name source | `swift test --package-path apps/menubar --filter PhoneSetupTests` |
| T13 | contract | the new fields and method in both languages | `api_examples.rs`, `ApiContractTests.swift` | none | `cargo test -p localrouter-core --test api_examples` and `swift test --package-path apps/menubar --filter ApiContractTests` |
| T14 | integration | the CLI: `add --lan`, `setup`, `password --new` | `localrouter`, `localrouterd` | the network | `cargo test -p localrouter --test cli proxy_client_lan` |
| T15 | integration | MCP `get_proxy` has no password and no setup URL; still nine tools | `localrouter mcp` | none | `cargo test -p localrouter --test mcp` |
| E1 | end-to-end | the phone journey through the real accept path from a non-loopback address | `localrouter`, `localrouterd`, the Mac's own LAN address | the network id (`LOCALROUTER_TEST_NETWORK`), the phone (a Rust client) | `cargo test -p localrouter --test e2e phone` |
| M0 | manual, first | what iOS does with a Wi-Fi profile that carries a proxy (U4, U5, U6, U7, U2) | iPhone, home Wi-Fi | LocalRouter (a test proxy with a password instead) | by hand, before task 1 |
| M1 | manual | the panel in each state, the Location Services request | the app | none | by hand |
| M2 | manual | a real iPhone through LocalRouter after scan and Install, HTTP and HTTPS | iPhone, Wi-Fi | none | by hand |
| M3 | manual | the setup page, the phone profile, the manual fallback | iPhone | none | by hand |
| M4 | manual | another machine without the password; a network not allowed | a second Mac | none | by hand |
| M5 | manual | the phone cannot reach the Mac's loopback | iPhone | none | by hand |
| M6 | manual | what iOS does with `*.localhost` through the proxy (U1) | iPhone | none | by hand |
| M7 | manual | the phone when the Mac sleeps, the proxy is off, the address changes | iPhone | none | by hand |
| M8 | manual | post-release smoke test | the release app | none | by hand |

**Replaced parts and who covers them for real.**

| Replaced in | What it hides | Covered for real by |
|---|---|---|
| `fake_peer` (T3, T8, T10) | that `accept_proxy` passes the real peer and runs the LAN check | E1, M2 |
| the local-target rule (T3) | that the real rule allows a `.localhost` route and refuses the rest | T4 (unit), E1, M5 |
| `LOCALROUTER_TEST_NETWORK` (T7, T11, T14, E1) | that `network.rs` reads the real network and address | M2, M4 |
| the Rust client in E1 | how iOS answers 407, `CONNECT` with Basic, the profile | M0 (with a test proxy), then M2, M3 with LocalRouter |
| the test proxy in M0 | LocalRouter's own 407 and `Proxy-Authenticate` text | M2 |
| the Wi-Fi name source in T12 | CoreWLAN and the Location Services dialog | M1 |
| no test for timing | that the compare is constant-time | review in task 1 (I4) |

## 1. Automated tests (CI)

### Core library: `libs/core/src/config.rs`, `libs/core/src/phone.rs` (unit)

| Test | Case |
|---|---|
| `config::tests::proxy_client_lan_default_and_round_trip` (T1) | `{"name":"chrome","port":7878}` reads `lan: false`; `lan: true` round trips; `lan: false` is not written |
| `phone::tests::password_format` (T2) | 19 characters, four groups of four, only the 32 allowed characters; 100 calls give 100 different passwords |
| `phone::tests::same_password` (T2, I4) | equal → true; one wrong last character → false; shorter, longer, empty → false |
| `phone::tests::basic_header` (T2) | `Basic aXBob25lOms3bXE...` for `iphone` and its password decodes to true; wrong user name → false; not Basic → false |
| `phone::tests::profile_phone` (T9, I10) | `plutil -convert json` reads it; one `com.apple.wifi.managed` payload with `SSID_STR` the given name, `AutoJoin` true, `ProxyType` `Manual`, `ProxyServer`, `ProxyServerPort`, `ProxyUsername` the client, `ProxyPassword` the password, and no `Password` key; one `com.apple.security.root` payload with the DER of the test CA; no `PRIVATE KEY` text; the identifier holds the bundle id and the client name |
| `phone::tests::profile_phone_without_ca` (T9, I10) | with no inspection CA, or an empty inspect list: the Wi-Fi payload only |
| `phone::tests::profile_phone_auto` (T9) | with the Auto option (if M0 picks it): `ProxyType` `Auto`, `ProxyPACURL` on the setup path, `ProxyPACFallbackAllowed` true; the PAC text is `PROXY <address>:<port>; DIRECT` |
| `phone::tests::profile_ca` (T9, I10) | the CA profile: one root payload, the identifier holds the CA fingerprint |
| `phone::tests::setup_page_escapes` (T8) | a client name and address are HTML-escaped |

### Core library: `libs/core/src/upstream.rs` (unit)

| Test | Case |
|---|---|
| `upstream::tests::is_local_target` (T4, I5) | `127.0.0.1`, `127.5.5.5`, `::1`, `::ffff:127.0.0.1`, `0.0.0.0`, `::`, an address in the own list → local; `192.168.0.5`, `1.1.1.1`, `2606:4700::1111` → not local |

### Forward proxy: `libs/core/tests/forward.rs` (integration)

The `Harness` gets `fake_peer: Option<SocketAddr>` and `lan: bool` for its
client port.

| Test | Case |
|---|---|
| `lan_client_needs_the_password` (T3, I3) | from `192.168.0.23`: absolute-form GET without `Proxy-Authorization` → 407 with `Proxy-Authenticate: Basic`; the echo server's counter stays 0 |
| `lan_client_wrong_password_on_connect` (T3, I3) | `CONNECT example.test:443` with a wrong password → 407; no TLS server connection |
| `lan_client_localhost_needs_the_password` (T3, I3) | `GET http://shop.localhost/` without password → 407; the route's target counter stays 0 |
| `lan_client_with_the_password_works` (T3) | the same three with the right password → 200, the tunnel, the route; HAR entries have `_client` and no `Proxy-Authorization` |
| `lan_client_loopback_peer_needs_no_password` (T3) | peer `127.0.0.1`, no header → 200 (ADR 09 behaviour) |
| `local_target_is_refused_for_a_lan_peer` (T4, I5) | with the real rule: `GET http://127.0.0.1:<echo>/`, `CONNECT [::ffff:127.0.0.1]:<echo>` and a name the test resolver maps to `127.0.0.1` → 403; counter 0. A `.localhost` route → 200 |
| `local_target_is_allowed_for_a_loopback_peer` (T4) | the same requests from `127.0.0.1` → as today |
| `setup_page_on_a_lan_client` (T8, I13) | `GET /setup/<password>` → 200, the four values, `Cache-Control: no-store`, the CSP, `Referrer-Policy: no-referrer` |
| `setup_page_refusals` (T8, I13) | wrong password, old password after a change, `/setup/` alone → 400 "This port is a proxy"; on the main port and on a plain client the right path → 400 |
| `setup_profile` (T9) | `/setup/<password>/phone.mobileconfig` and `/ca.mobileconfig` → 200, `application/x-apple-aspen-config`; with no Wi-Fi name, `phone.mobileconfig` → 400 and the page shows only the manual way (I15) |
| `setup_not_logged` (T10, I9, I6) | after the setup page and the profile: no request log entry and no HAR entry; after a wrong `/setup/x`: one entry with path `/setup/…`; no entry anywhere holds the password text |

### Daemon: `apps/daemon` (unit and integration)

| Test | Case |
|---|---|
| `listen::tests::proxy_accept_decision` (T5, I2) | loopback → serve on every port; non-loopback on main or plain client → refuse; on a LAN client → `peer_allowed` with `allow_lan` and the list (every row of ADR 08 T14 again) |
| `api.rs::lan_client_binds` (T6, I1) | with `allow_lan` true: `status.proxy.bound` is `127.0.0.1:p`, `[::1]:p`; a plain client the same; a LAN client lists `0.0.0.0:q` and `[::]:q` |
| `api.rs::lan_client_password_file` (T7, I7) | `set_config` adds a LAN client → `proxy-passwords.json` exists, mode `0600`, one entry; `get_proxy {client}` has `lan.password` equal to it; removing the client removes the entry |
| `api.rs::lan_client_password_nowhere_else` (T7, I6) | the password text is in no reply of `get_config`, `status`, `get_proxy` without `client`, `logs`, and not in the daemon log file. Extends `no_reply_contains_key_material` |
| `api.rs::new_proxy_password_closes_tunnels` (T7, I11) | open a `CONNECT` tunnel through the LAN client from loopback, call `new_proxy_password` → the tunnel is closed; the reply's password differs; `not_found` for an unknown client; `invalid_request` for a plain client |
| `api.rs::lan_client_password_made_at_start` (T7, I12) | delete `proxy-passwords.json`, restart → a new password exists before `status` shows the port bound; an entry for a removed client is gone |
| `api.rs::setup_url` (T11) | `LOCALROUTER_TEST_NETWORK=mac:…,192.168.0.1,en0,192.168.0.10` → `http://192.168.0.10:<port>/setup/<password>`; `none` → `setup_url: null`, `problems` names the reason |
| `api.rs::wifi_name` (T11, I15) | `set_config lan_networks` with `wifi_name` → saved, in `status.network.wifi_name`; `get_proxy.lan.profile` true; without it → `profile: false` |

### CLI and MCP: `apps/cli/tests` (integration)

| Test | Case |
|---|---|
| `cli.rs::proxy_client_lan` (T14) | `proxy client add iphone --lan` → listed with `lan`; `lan name Home-5G` → saved on the current network; `proxy client setup iphone` prints the URL, Server, Port, Username, Password; `proxy client password iphone --new` prints a new one |
| `mcp.rs` (T15, I8) | the tool list is still nine; `get_proxy {client: "iphone"}` has `lan.address` and `lan.port` but no `password` and no `setup_url`; the text names the Proxy tab |

### Contract: `api/examples/` (T13)

New files: `get_proxy_lan_client.request.json`/`.reply.json`,
`set_config_proxy_clients_lan.*`, `new_proxy_password.*`. Both contract tests
walk the folder and fail on a method without an example, so they grow by
themselves.

### Swift: `apps/menubar/Tests/LocalRouterKitTests` (unit)

| Test | Case |
|---|---|
| `PhoneSetupTests.testChecklist` (T12) | from decoded `status` JSON: proxy off; LAN off; network not allowed; network unknown; no Wi-Fi name; all good → the rows, their buttons, and whether "Set Up a Phone" is enabled |
| `PhoneSetupTests.testWifiName` (T12, I14) | a typed name is sent as `lan_networks[].wifi_name` of the current network only; the other networks' names stay; no Location Services call happens unless "Use My Location" was pressed |
| `PhoneSetupTests.testScanAgainNote` (T12) | after a new password or a new address, the panel shows "Scan again and tap Install" |
| `PhoneSetupTests.testNextName` (T12) | `iphone` free → `iphone`; taken → `iphone-2`; both taken → `iphone-3` |
| `PhoneSetupTests.testQrRoundTrip` (T12) | the QR image for a setup URL is read back by `CIDetector` (type QR code) to the same URL |
| `PhoneSetupTests.testQrHiddenWhenNotReady` (T12) | a LAN client exists but LAN access is off → no QR payload, the failing row |
| `ApiContractTests` (T13) | every new example decodes; `lan` round trips |

### Regression for existing behaviour

The existing ADR 06 and ADR 09 tests stay unchanged and must pass:
`origin_form_gets_the_this_is_a_proxy_page`, `own_address_is_a_loop`,
`localhost_names_are_answered_by_the_route_table`, the proxy client tests in
`api.rs`, `no_fixed_names.rs` (the new texts use the templates).

## 2. Automated end-to-end test: E1, the phone journey

`apps/cli/tests/e2e.rs`, function `phone`. It runs the real accept path with
a non-loopback peer: the test connects to the Mac's own LAN address (the first
non-loopback IPv4 of `getifaddrs`). A connection from `192.168.0.10` to
`192.168.0.10` has that address as its peer, so the daemon treats it as
another machine. The test is skipped with a message when the Mac has no such
address.

1. Start `localrouterd` with `LOCALROUTER_TEST_NETWORK=mac:02:00:00:00:00:01,192.168.0.1,<iface>,<own address>`.
2. `localrouter proxy on`. Check: `status.proxy.bound` is loopback only.
3. Start a test dev server on `127.0.0.1` and `localrouter add shop <port>`.
4. `localrouter proxy client add iphone --lan`. Check: the client lists
   `0.0.0.0:q`.
5. From the own address, `GET http://shop.localhost/` through port `q`
   without a password. Check: the connection is closed with no answer
   (`allow_lan` is off).
6. `localrouter lan allow --name Test`, then turn on LAN access with
   `set_config {"allow_lan": true}` over the socket (the CLI has no command
   for the switch; `lan allow` only adds the network). Repeat step 5.
   Check: 407.
7. `localrouter lan name Test-WiFi`, then `localrouter proxy client setup
   iphone`. Check: the URL starts with `http://<own address>:q/setup/`. `GET`
   that URL and `<url>/phone.mobileconfig` from the own address. Check: 200
   for both; the profile's `SSID_STR` is `Test-WiFi` and its `ProxyPassword`
   is the password; `localrouter logs` has no `/setup/` line.
8. Repeat step 5 with `Proxy-Authorization`. Check: 200 from the dev server;
   the HAR entry has `_client: "iphone"`.
9. With the password, `GET http://127.0.0.1:<dev port>/`. Check: 403, and the
   dev server's counter did not move.
10. `localrouter proxy client password iphone --new`. Repeat step 8 with the
    old password. Check: 407.
11. `localrouter proxy off`. Check: port `q` refuses connections.

## 3. Manual tests

### M0. What iOS does with a Wi-Fi profile (first, before task 1)

Run on a real iPhone on home Wi-Fi, before any code is written. Use a test
proxy with a password on the Mac, listening on the LAN, for example
`mitmproxy --listen-host 0.0.0.0 --listen-port 8878 --proxyauth iphone:test`
(installed with Homebrew, removed afterwards). Write the profile by hand from
change 2, serve it with `python3 -m http.server`, and open it in Safari.

| Check | Expected, or what to record |
|---|---|
| Install the profile with `ProxyType` `Manual`, user `iphone`, password `test`, **no Wi-Fi `Password`**, for the Wi-Fi the phone already uses (U5) | record: the phone stays on the Wi-Fi / asks for the Wi-Fi password / fails to join |
| Safari: an `http://` and an `https://` site (U4) | both load; mitmproxy shows the requests with the user `iphone`; no password prompt on the phone |
| An app (for example Weather) (U4) | works, its requests appear |
| Settings → Wi-Fi → (i): can the proxy be changed by hand while the profile is installed? | record |
| `ProxyServer` set to the Mac's Bonjour name `<name>.local` instead of the IPv4 address (U7) | record: works / fails / slow |
| Profile with `ProxyType` `Auto`, `ProxyPACURL` `http://<mac>:8000/proxy.pac` returning `PROXY <mac>:8878; DIRECT`, `ProxyPACFallbackAllowed` true (U2) | requests go through mitmproxy; record whether the password is sent |
| With the Auto profile, stop mitmproxy and the PAC server (the Mac is "away") (U2) | record: the phone goes direct, and how many seconds the first page waits |
| Remove the profile (U6) | record: the phone stays on the Wi-Fi / forgets it |

Write each result into the manifest's U2, U4 to U7. **If U4 fails, stop:
the password design of change 1 does not work.**

### M1. The panel (UI)

| Check | Expected |
|---|---|
| Proxy off, open Phone… | the checklist; "Set Up a Phone" disabled; "Turn On" next to the proxy row |
| "Use My Location" | the system dialog shows the text from change 3; after Allow, the Wi-Fi name row shows the real name |
| Deny the permission, type the name | the row turns ✓; no second dialog appears |
| Press each ✗ button | the row turns ✓; Settings → Routing shows the same network allowed |
| Set Up a Phone | the QR code and the install steps; "Type by hand" shows the four values; copy buttons copy |
| Turn off LAN access in Settings | the QR code disappears; the LAN row is ✗ |
| New Password | the QR code and the password change; the panel says "Scan again and tap Install" |
| Remove Phone | the warning text (remove the profile on the phone first); then the checklist again |
| Dark mode | the QR code is black on white in both modes (a camera needs that) |

### M2. A real iPhone through LocalRouter

| Check | Expected |
|---|---|
| Scan the QR code, tap Install, install the profile | no typing; the phone stays on the Wi-Fi (as M0 found) |
| Safari: an `http://` site | loads; the Proxy tab lists it with the phone's client |
| Safari: an `https://` site, inspection CA trusted | loads; the HAR entry has the decrypted request |
| New Password on the Mac, do not scan again | the phone's requests get 407; record what iOS shows |
| Scan again, Install | the new profile replaces the old one (one profile in the list); requests work |
| An app (for example Weather) | works; its requests appear |
| Count the phone's entries for 10 minutes while idle | the number is written into the manifest's data impact |

### M3. The setup page, the profile and the manual fallback

| Check | Expected |
|---|---|
| Scan the QR code with the camera | Safari opens the setup page with one Install button |
| Open the setup URL in Chrome on the phone | the page says to open it in Safari |
| Install, turn on full trust for the CA | the CA appears in Certificate Trust Settings; inspected HTTPS works |
| Reset the inspection CA on the Mac, scan again, install | the profile is replaced; the new CA is trusted after the trust switch |
| "Did not work?": type the four values by hand, install `ca.mobileconfig` | the manual way works the same |
| Remove the profile | the phone goes direct; the Wi-Fi stays or is forgotten as M0 found, and step 4 of the page says the same |

### M4. Another machine (failure checks)

| Check | Expected |
|---|---|
| A second Mac: `curl -x http://<mac>:8878 http://example.com` | 407 |
| The same with `-U iphone:<password>` | 200 |
| Move the Mac to a phone hotspot (network not allowed) | the second machine's connection is closed with no answer |

### M5. The Mac's loopback from the phone

| Check | Expected |
|---|---|
| iPhone Safari through the proxy: `http://127.0.0.1:<a dev port>` | 403 page |
| `http://<mac address>:7080` (the router port) | 403 page |
| `http://shop.localhost/` (a route) | see M6 |

### M6. `*.localhost` from the phone (U1)

| Check | Expected |
|---|---|
| iPhone Safari: `http://shop.localhost/` with the proxy on | record what happens: the route answers, or Safari fails without asking the proxy. Write the result into U1 |

### M7. The phone when the Mac is away

| Check | Expected |
|---|---|
| Put the Mac to sleep, use Safari | with Auto (if M0 picked it): the phone goes direct. With Manual: record the error text iOS shows, and check that step 4 of the setup page names this case |
| Proxy off on the Mac | the same |
| Remove the profile on the phone | the phone works again at once; the Wi-Fi stays or is forgotten as M0 found |

### M8. Post-release smoke test

| Check | Expected |
|---|---|
| Update the release app; open the Proxy tab | "Phone…" is there; no LAN client exists; the main port is still loopback only |
| `localrouter status` | API 1.7; no `0.0.0.0` proxy socket |

No manual check costs money or writes data that cannot be removed. M3 leaves
a CA profile on the phone; remove it at the end.

## Not in this plan without approval

| Tool | What it would cover | Cost |
|---|---|---|
| UI tests of the menu bar views (XCUITest) | the panel states of M1 | a new test target and a running app in CI |
| An iOS device farm or a simulator with a Wi-Fi proxy | M2, M3 automated | the simulator uses the Mac's network settings, not its own Wi-Fi proxy, so it would not test the real thing |

## Test data

| Fixture | Why |
|---|---|
| peer `192.168.0.23:50000` | a LAN peer for the core tests |
| network `mac:02:00:00:00:00:01,192.168.0.1,en0,192.168.0.10` | a known network with an address, for the daemon tests |
| two clients, `iphone` (lan) and `chrome` (plain), with ports next to each other | a check that only the LAN one gets the new behaviour |
| a password with every allowed character class | the format and the compare |
| Wi-Fi name `Café "Home" & 5G` | the profile and the page escape it (XML and HTML) |
| the test CA of `forward.rs` (`test_ca`) | the profile's DER |

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| I1 main port and plain clients loopback only | T6 | M8 |
| I2 LAN peer only with allow_lan and an allowed network | T5, E1 | M4 |
| I3 407 without the password, nothing upstream | T3, E1 | M2, M4 |
| I4 the compare looks at every byte | T2 (refusals); the timing by review in task 1 | none |
| I5 403 for the Mac's own addresses | T4, E1 | M5 |
| I6 the password nowhere else | T7, T10 | none |
| I7 the file is 0600 | T7 | none |
| I8 MCP has no password | T15 | none |
| I9 setup requests not logged | T10, E1 | none |
| I10 the profiles hold only what change 2 lists | T9 | M0, M3 |
| I11 a new password closes the connections | T7, E1 | M1 |
| I12 a LAN client always has a password | T7 | none |
| I13 /setup only on a LAN client | T8 | none |
| I14 only the app asks for Location Services, only on the button | T12 | M1 |
| I15 no phone profile without a Wi-Fi name | T8, T11, T12 | none |
