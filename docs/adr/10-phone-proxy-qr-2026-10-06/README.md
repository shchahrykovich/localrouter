# ADR 10: A phone on the forward proxy, set up by a QR code

**Status:** Proposed, 2026-10-06. Not built. Changes ADR 06 decision 2
("the proxy port is loopback only") for one kind of proxy client.

## Summary

**In one sentence.** A proxy client can be a LAN client: its port accepts an
iPhone on an allowed network with a password, and the Proxy tab shows a QR
code that opens a setup page with the values to type and the CA profile.

**In three sentences.** An iPhone cannot use the forward proxy today, because
every proxy port listens on `127.0.0.1` only.

This ADR lets one proxy client (ADR 09) listen on the LAN, behind the same
network rule as ports 80 and 443 (ADR 08), a password, and a rule that keeps
the phone away from the Mac's own loopback servers. The Proxy tab shows a QR
code; the phone opens a page with the four values for Settings and the
inspection CA as a profile, and its requests appear in the proxy log as
`_client: "iphone"`.

**In seven sentences.** iOS sets a proxy for each Wi-Fi network by hand
(server, port, user name, password), and a QR code can only open a URL, so
the phone needs a page that tells the user what to type.

The proxy listens on loopback only (ADR 06), because an open proxy on the LAN
lets anyone use the Mac and read what it records. This ADR adds `lan: true`
to a proxy client: that one port binds `0.0.0.0` and `[::]`, accepts another
machine only when `allow_lan` is on and the network is in `lan_networks`,
answers `407` without the password from `proxy-passwords.json` (mode `0600`),
and answers `403` for any target that is this Mac's loopback or own address,
while `.localhost` routes still work.

The same port serves `/setup/<password>`: a static page with the values and a
`.mobileconfig` with the inspection CA; these requests are never logged. The
app's Proxy tab gets a "Phone…" panel: a checklist that fixes each missing
condition with one button, then the QR code drawn with Core Image, the values,
"New Password" and "Remove Phone"; socket API 1.7 adds `lan` and
`new_proxy_password`, and the MCP tool never returns the password.

The main trade-off: the phone depends on the Mac for its internet on that
Wi-Fi, which the page and the panel say but cannot prevent. There is no data
migration; the plan is 15 automated tests, one end-to-end journey through
the Mac's own LAN address, and 8 manual checks with a real iPhone.

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
| 2 | The setup page and the CA profile | feature | [02-setup-page.md](02-setup-page.md) |
| 3 | The QR code in the Proxy tab, socket API 1.7, CLI, MCP | feature, UI | [03-qr-code-and-api.md](03-qr-code-and-api.md) |
| 4 | Where the code lives | overview | [04-components.md](04-components.md) |
| 5 | Setup, a phone request, a new password | data flow | [05-data-flow.md](05-data-flow.md) |
| 6 | What exists and happens afterwards | manifest | [06-semantic-change-manifest.md](06-semantic-change-manifest.md) |
| 7 | The tests that prove it | test plan | [07-test-plan.md](07-test-plan.md) |
| 8 | The order of work | build plan | [08-tasks.md](08-tasks.md) |

## Why read the working-backwards file

It found 7 gaps. Two changed the design most:

- **G4**: through a LAN proxy port, the phone (or anyone with the password)
  could reach every loopback server of the Mac, such as
  `http://127.0.0.1:5432`. `upstream.rs` has no rule against it, because
  until now only the Mac itself could use the proxy. Change 1 adds the 403
  rule.
- **G7**: an agent that reads today's texts tells the user to set
  `127.0.0.1:8877` on the phone, which is the phone itself.

## Why read the manifest

- **The phone's internet depends on the Mac.** On that Wi-Fi the phone has no
  internet when the Mac sleeps, the proxy is off, or the Mac's address
  changes. No rollback undoes the phone's setting. This is the main cost of
  the feature, and only the user can remove it.
- **Four unresolved effects.** U4 can stop the build: it is not yet seen on a
  device that iOS sends the proxy password for `CONNECT`. U1 (do `*.localhost`
  names from Safari reach the proxy?) decides whether the profile should also
  hold the local CA.
- **The phone's traffic fills the shared proxy log**, so other clients'
  entries are pruned sooner.

## Why read the test plan

It found that the new 403 rule blocks every success test, because every test
upstream binds `127.0.0.1`. The rule is therefore a value the forward proxy
holds, narrowed in the core tests and real in E1. E1 reaches the real accept
path with a non-loopback peer by connecting to the Mac's own LAN address.

## Notable artifacts

- `proxy-passwords.json` in the data folder, mode `0600`.
- `ProxyClient.lan` in `config.json`.
- Socket API 1.7: `new_proxy_password`; `get_proxy.lan`.
- `libs/core/src/phone.rs`, `LocalRouterKit/PhoneSetup.swift`,
  `Views/PhoneView.swift`.
- No migration.

## Through-line

Every protection of the forward proxy so far came from one fact: the client
is this Mac. Moving one port to the LAN removes that fact, so this ADR puts
back, one by one, what loopback gave for free: who may connect (the network
rule), who may use it (the password), and what it may reach (the 403 rule).
