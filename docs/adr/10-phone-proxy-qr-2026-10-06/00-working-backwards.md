# 0. Working backwards: a phone on the proxy

**Status:** Mixed. The one quote marked (real) is the user's request in a
Claude Code session on 2026-10-06. The repository has no GitHub issues
(checked 2026-10-06). Every other quote is simulated, written to test the
design before it is built.

## Real feedback

> "on proxy page show QR code for iphone to route traffic via proxy" (real,
> the user, 2026-10-06)

It names three things: the place (the Proxy tab), the tool (a QR code) and the
goal (the iPhone's traffic through the proxy). It does not say whether HTTPS
must be read. The design assumes yes, because `inspect_hosts` is `["*"]` by
default and the proxy log exists to read traffic.

## The announcement

> **LocalRouter: see your iPhone's traffic**
>
> The Proxy tab has a new **Phone…** button. It checks that the proxy is on
> and that your Mac allows LAN access on this Wi-Fi, then shows a QR code.
> Scan it with the iPhone camera. A page opens with the four values to type
> in Settings → Wi-Fi → Configure Proxy, and a CA profile, so the proxy can
> read HTTPS.
>
> From then on, every request of the phone on that Wi-Fi appears in the proxy
> log, marked `iphone`, next to Chrome and your agents.
>
> The phone port asks for a password, works only on the networks you allowed,
> and cannot reach your Mac's own servers, only your routes.
>
> One thing to remember: the phone uses your Mac for its internet on that
> Wi-Fi. When you are done, set Configure Proxy back to Off.

## Roles

| Role | Setup | Wants |
|---|---|---|
| Main user | a web developer, Mac and iPhone on home Wi-Fi | to see what the mobile site and an app send |
| Automated client | Claude Code with the LocalRouter MCP server | to help the user debug a mobile request |
| A user who does not want it | uses only Chrome through the proxy | nothing to change |
| A team member without LocalRouter | gets HAR files from a colleague | to read them |
| Operator | the maintainer, ships releases | no new support load, a safe rollback |
| Another person on the same Wi-Fi | a flatmate, a colleague | not part of the feature; the design must stop them |

## Reactions

### Positive

**R1: "I scanned the code, typed four things, and saw my app's API calls on
the Mac."** The goal. **covered**, [03](03-qr-code-and-api.md),
[02](02-setup-page.md).

**R2: "The phone's requests are marked iphone in the viewer; I can filter
them."** **covered** by ADR 09 `_client`, [01](01-phone-port.md) decision 1.

**R3: "Nothing changed for me; Chrome still goes through 8877."**
(the user who does not want it) The main port and plain clients stay loopback.
**covered**, I1, [01](01-phone-port.md) decision 2.

### Negative: the user's own setup

**R4: "I scanned the code and Safari says the page cannot be opened."** The
office guest Wi-Fi blocks devices from each other. LocalRouter cannot see this.
**gap, fixed** (G1): the panel names this case under the QR code.
[03](03-qr-code-and-api.md) decision 3; manifest blast radius.

**R5: "Every HTTPS site on my phone shows a certificate error."** The CA
profile is not installed or not trusted, and `inspect_hosts` is `["*"]`.
**gap, fixed** (G2): the panel and step 2 of the page say "HTTPS sites fail
on the phone until it trusts the CA". [02](02-setup-page.md),
[03](03-qr-code-and-api.md). A tunnel-only phone is **accepted, not now**:
manifest U3.

**R6: "My banking app stopped working on the home Wi-Fi."** It pins its
certificate. **accepted**: the page tells the user to take the host out of
the inspect list. [02](02-setup-page.md) decision 3 step 2.

**R7: "My German iPhone has no menu called Settings → Wi-Fi."** **accepted**:
the page is English only. [02](02-setup-page.md) trade-offs.

**R8: "With my company VPN on, the Phone… panel says the network cannot be
recognised."** The VPN interface has no router MAC (ADR 08). **accepted**,
the same as ports 80 and 443.

### Negative: the feature's own behaviour

**R9: "The next morning my phone had no internet at home. It took me an hour
to remember the proxy."** The Mac was asleep. The silent failure from the
manifest's blast radius. **gap, fixed** (G3): step 4 of the page, the panel
line "The phone has no internet on this Wi-Fi while the Mac sleeps", and the
"Remove Phone" warning. The real fix, a PAC file with a fallback, is
**not now**: manifest U2. [02](02-setup-page.md), [03](03-qr-code-and-api.md).

**R10: "After the router restarted, my Mac got a new address and the phone
stopped working."** The phone still has the old server value. **accepted**:
the panel shows the current address in large type; the user types it again.
[03](03-qr-code-and-api.md) decision 3.

**R11: "My flatmate found port 8878 open on my Mac and used it."** (the other
person) The password stops every request. **covered**, I3,
[01](01-phone-port.md) decision 4.

**R12: "From the phone I opened http://127.0.0.1:5432 through the proxy and
reached my Mac's Postgres admin."** This was the gap found while reading
`upstream.rs`: nothing refused loopback targets. **gap, fixed** (G4): the
403 rule, I5. [01](01-phone-port.md) decision 5.

**R13: "I shared my screen in a meeting with the Phone… panel open."** The QR
code and the password were visible. **accepted** with help: "New Password"
closes every open connection (I11). Manifest risks.

**R14: "The setup URL with the password is in my proxy log, and I sent that
log to a colleague."** (the team member) **gap, fixed** (G5): setup requests
are never logged (I9). [02](02-setup-page.md) decision 5.

**R15: "My Lua rule that rewrites api.example.com for Chrome now also changes
the phone's requests."** **accepted**: rules do not see the client (ADR 09
trade-off). Manifest blast radius.

**R16: "Chrome's entries disappear from the viewer much faster since the
phone is on."** The phone's background traffic shares the 5 files.
**accepted**, measured in M2. Manifest data impact.

**R17: "I removed the phone in the app, and the phone lost its internet."**
**gap, fixed** (G6): "Remove Phone" warns first. [03](03-qr-code-and-api.md)
decision 3.

**R18: "We rolled back to the previous version and my partner's phone had no
internet on our Wi-Fi."** The old daemon binds the client on loopback only.
**accepted**: manifest rollback names it as an external effect no rollback
undoes. (operator)

**R19: "After I reset the inspection CA, the phone still trusts the old one."**
**accepted**: the profile on the phone is an external copy. M3 checks that a
new CA is a new profile. Manifest source of truth.

**R20: "Does the phone open https://shop.localhost now?"** Not known.
**accepted, not now**: manifest U1, checked by M6.

## A simulated session with the automated client

The user says to Claude Code: "Route my iPhone through the LocalRouter proxy
so we can see what the app sends."

1. The agent reads the LocalRouter note in `CLAUDE.md` (from `note.md`): the
   proxy, `get_proxy`, the proxy log.
2. It calls the MCP tool `get_proxy`. Today the reply says the proxy is
   `127.0.0.1:8877`.
3. **Today it would act wrongly here.** Nothing says the proxy is loopback
   only, so it tells the user: "Set the iPhone's proxy to 127.0.0.1:8877."
   That address is the phone itself. The user gets no internet.
4. With this ADR, `get_proxy` lists the clients; a LAN client shows
   `lan.address` and `lan.port`, but no password, and the text says "A phone
   is set up in the app: Proxy tab → Phone…".
5. With no LAN client yet, the agent has no way to make one: there is no MCP
   tool for it, by design (ADR 08: an agent must not open the Mac to a
   network). It tells the user to open the Proxy tab.
6. The user sets up the phone. The agent reads the log through the viewer at
   `proxy.localhost/iphone` (ADR 09) and finds the app's requests.

**Gap at step 3** (G7): the texts must say that the proxy address does not
work from a phone. **gap, fixed**: `note.md`, `mcp.md` and `help.md` get the
line in task 7, and the `get_proxy` tool text in task 5.
[03](03-qr-code-and-api.md) decisions 6 and 7.

This session is the script of M2's start: give an agent this request and
check that it sends the user to the Proxy tab.

## Gaps

| # | Gap | Fix | Where |
|---|---|---|---|
| G1 | a guest Wi-Fi blocks the phone from the Mac; nothing says so | a line under the QR code | [03](03-qr-code-and-api.md) decision 3, manifest blast radius |
| G2 | HTTPS fails until the CA is trusted; the user does not know why | panel line, page step 2 | [02](02-setup-page.md), [03](03-qr-code-and-api.md) |
| G3 | the phone has no internet when the Mac is away | page step 4, panel line; PAC is U2 | [02](02-setup-page.md), [03](03-qr-code-and-api.md), manifest U2 |
| G4 | the phone reaches the Mac's loopback through the proxy | the 403 rule | [01](01-phone-port.md) decision 5, I5, T4, task 2 |
| G5 | the setup URL with the password lands in the log | setup requests not logged | [02](02-setup-page.md) decision 5, I9, T10, task 3 |
| G6 | removing the phone in the app cuts the phone's internet with no warning | a warning before "Remove Phone" | [03](03-qr-code-and-api.md) decision 3, M1, task 6 |
| G7 | an agent tells the user to set `127.0.0.1` on the phone | texts and the `get_proxy` tool text | [03](03-qr-code-and-api.md) decisions 6 and 7, task 5, task 7 |

Every gap is fixed in the files it touches, or written as an unresolved
effect in the manifest (U1 to U4).
