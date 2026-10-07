# ADR 10: A phone on the forward proxy, set up by a QR code

**Status:** Proposed, 2026-10-06. Not built. Changes ADR 06 decision 2
("the proxy port is loopback only") for one kind of proxy client.

## Summary

**In one sentence.** A proxy client can be a LAN client: its port accepts an
iPhone on an allowed network with a password, and the Proxy tab shows a QR
code that installs one configuration profile with the Wi-Fi proxy, the
password and the CA, so the user types nothing.

**In three sentences.** An iPhone cannot use the forward proxy today, because
every proxy port listens on `127.0.0.1` only.

This ADR lets one proxy client (ADR 09) listen on the LAN, behind the same
network rule as ports 80 and 443 (ADR 08), a password, and a rule that keeps
the phone away from the Mac's own loopback servers. The user scans a QR code
and taps Install: a profile sets the proxy of that Wi-Fi network, with the
password inside it, and the phone's requests appear in the proxy log as
`_client: "iphone"`.

**In seven sentences.** iOS does not let a QR code or a web page change the
Wi-Fi proxy, but a configuration profile with a Wi-Fi payload can set it,
including the proxy user name and password, on an iPhone no company manages.

The proxy listens on loopback only (ADR 06), because an open proxy on the LAN
lets anyone use the Mac and read what it records. This ADR adds `lan: true`
to a proxy client: that one port binds `0.0.0.0` and `[::]`, accepts another
machine only when `allow_lan` is on and the network is in `lan_networks`,
answers `407` without the password from `proxy-passwords.json` (mode `0600`),
and answers `403` for any target that is this Mac's loopback or own address,
while `.localhost` routes still work.

The same port serves `/setup/<password>`: a page with one Install button for
`phone.mobileconfig` (a Wi-Fi payload for the network's name with the proxy
and its password, and the inspection CA), a manual fallback, and nothing of
it is logged. The Proxy tab's "Phone…" panel fixes each missing condition with
one button, gets the Wi-Fi name (Location Services on request, or typed), and
shows the QR code; socket API 1.7 adds `lan`, `wifi_name` and
`new_proxy_password`, and the MCP tool never returns the password.

Five behaviours of iOS are not verified yet, and M0 checks them on a real
iPhone with a test proxy before any code is written: the password for
`CONNECT`, a profile without the Wi-Fi password, what removing the profile
does, the Mac's Bonjour name, and an automatic proxy that goes direct when the
Mac is away. Then come 15 automated tests, one end-to-end journey through
the Mac's own LAN address, and the manual checks with LocalRouter itself.

## Context

The user asked: "on proxy page show QR code for iphone to route traffic via
proxy" (2026-10-06). The answer before this ADR, given in the same session,
was that it needs a decision first: ADR 06 decision 2 says the proxy port is
loopback only, and an iPhone is another machine.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 0 | What users would say, checked against the ADR | simulation | [00-working-backwards.md](00-working-backwards.md) |
| 1 | The phone port: LAN binding, the network rule, the password, the 403 rule | feature | [01-phone-port.md](01-phone-port.md) |
| 2 | The setup page: one profile, one Install button | feature | [02-setup-page.md](02-setup-page.md) |
| 3 | The QR code and the Wi-Fi name in the Proxy tab, socket API 1.7, CLI, MCP | feature, UI | [03-qr-code-and-api.md](03-qr-code-and-api.md) |
| 4 | Where the code lives | overview | [04-components.md](04-components.md) |
| 5 | Setup, a phone request, a new password | data flow | [05-data-flow.md](05-data-flow.md) |
| 6 | What exists and happens afterwards | manifest | [06-semantic-change-manifest.md](06-semantic-change-manifest.md) |
| 7 | The tests that prove it | test plan | [07-test-plan.md](07-test-plan.md) |
| 8 | The order of work | build plan | [08-tasks.md](08-tasks.md) |

## Why read the working-backwards file

It found 10 gaps. Three changed the design most:

- **G8 (real feedback)**: the first version asked the user to type four
  values into iOS Settings. The user asked for "take a photo and go to work".
  The design moved to a profile that carries the values and the password.

- **G4**: through a LAN proxy port, the phone (or anyone with the password)
  could reach every loopback server of the Mac, such as
  `http://127.0.0.1:5432`. `upstream.rs` has no rule against it, because
  until now only the Mac itself could use the proxy. Change 1 adds the 403
  rule.
- **G7**: an agent that reads today's texts tells the user to set
  `127.0.0.1:8877` on the phone, which is the phone itself.

## Why read the manifest

- **The phone's internet depends on the Mac.** With a manual proxy, on that
  Wi-Fi the phone has no internet when the Mac sleeps or the proxy is off. No
  rollback removes the profile from the phone; only the user can.
- **Seven unresolved effects, five of them answered by M0 before any code.**
  U4 can stop the build: it is not yet seen that iOS sends the proxy password
  from a profile for `CONNECT`. U2 can remove the main cost: an automatic
  proxy with a fallback would keep the phone online when the Mac is away. U5
  decides whether LocalRouter must ever hold the Wi-Fi password.
- **The phone's traffic fills the shared proxy log**, so other clients'
  entries are pruned sooner.

## Why read the test plan

M0 comes before any code: a hand-made profile and mitmproxy as a test proxy
on a real iPhone. Its answers fix the profile's fields.

It found that the new 403 rule blocks every success test, because every test
upstream binds `127.0.0.1`. The rule is therefore a value the forward proxy
holds, narrowed in the core tests and real in E1. E1 reaches the real accept
path with a non-loopback peer by connecting to the Mac's own LAN address.

## Notable artifacts

- `proxy-passwords.json` in the data folder, mode `0600`.
- `ProxyClient.lan` and `LanNetwork.wifi_name` in `config.json`.
- Socket API 1.7: `new_proxy_password`; `get_proxy.lan`.
- `phone.mobileconfig` and `ca.mobileconfig`, built on request, never stored.
- `NSLocationUsageDescription` in the app's `Info.plist`.
- `libs/core/src/phone.rs`, `LocalRouterKit/PhoneSetup.swift`,
  `LocalRouter/WifiName.swift`, `Views/PhoneView.swift`.
- No migration.

## Through-line

Every protection of the forward proxy so far came from one fact: the client
is this Mac. Moving one port to the LAN removes that fact, so this ADR puts
back, one by one, what loopback gave for free: who may connect (the network
rule), who may use it (the password), and what it may reach (the 403 rule).
