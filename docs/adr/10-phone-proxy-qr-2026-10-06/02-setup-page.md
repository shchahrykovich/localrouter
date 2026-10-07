# 2. The setup page: one profile, one "Install" button

## Context

A QR code can only give the phone a URL or a piece of text. A web page
cannot change the iPhone's Wi-Fi proxy. On an iPhone that no company manages
(not supervised), there are two ways to set a proxy:

| Way | What the user does | The proxy password |
|---|---|---|
| By hand | Settings → Wi-Fi → (i) → Configure Proxy → Manual, then types server, port, user name, password | typed by the user |
| A configuration profile (`.mobileconfig`) with a Wi-Fi payload | downloads it in Safari, taps Install, enters the phone's passcode | inside the profile, never shown |

A Wi-Fi payload (`com.apple.wifi.managed`) describes one Wi-Fi network by
its name (SSID) and can carry its proxy: `ProxyType` `Manual` with
`ProxyServer`, `ProxyServerPort`, `ProxyUsername`, `ProxyPassword`, or
`Auto` with `ProxyPACURL` and `ProxyPACFallbackAllowed`. A global proxy for
every network (`com.apple.proxy.http.global`) exists, but only for
supervised devices, so it is not an option here. Sources:
[Apple, WiFi payload](https://developer.apple.com/documentation/devicemanagement/wifi),
[Apple, Global HTTP Proxy](https://developer.apple.com/documentation/devicemanagement/globalhttpproxy).

The user asked for this flow: "I want the user to take a photo and go to
work" (2026-10-06, translated from Russian). A profile is the closest iOS allows: scan, Install,
passcode, Install. iOS always asks before it installs a profile and before it
trusts a CA, so these taps cannot be removed.

Four things about the profile way are not known yet. They are checked on a
real iPhone **before anything is built** (M0 in the [test plan](07-test-plan.md),
task 0):

| # | Question | Why it matters |
|---|---|---|
| U4 | Does iOS send the proxy password for `CONNECT` (HTTPS)? | without it, the password design fails |
| U5 | Does iOS accept a Wi-Fi payload **without the Wi-Fi password** for a network the phone already knows? | if not, the user must give the Mac the Wi-Fi password once |
| U6 | Does removing the profile also remove the Wi-Fi network from the phone? | if yes, "turn the proxy off" means "join the Wi-Fi again" |
| U2 | Does `ProxyType` `Auto` with a PAC URL and `ProxyPACFallbackAllowed` work with a proxy password, and does the phone go direct when the Mac is away? | if yes, the phone keeps its internet when the Mac sleeps |

## Decision

1. **The page lives on the phone port.** A request without a full URL
   (`GET /a`) on a proxy port gets the 400 page today
   ([forward.rs:425](../../../libs/core/src/forward.rs)). On a LAN client
   port these paths answer instead:

   | Path | Answer |
   |---|---|
   | `GET /setup/<password>` | the setup page |
   | `GET /setup/<password>/phone.mobileconfig` | the phone profile: Wi-Fi proxy and CA |
   | `GET /setup/<password>/ca.mobileconfig` | the CA alone, for the manual way |
   | `GET /setup/<password>/proxy.pac` | the PAC file, only if M0 picks `Auto` (U2) |

   The password in the path is the key: the QR code holds it, and the profile
   carries it anyway. Any other path, or an old or wrong password, gets the
   same 400 page as today. The paths are served to a peer that passes the LAN
   check of change 1 step 3, and to this Mac.
2. **The phone profile** (`phone.mobileconfig`,
   `Content-Type: application/x-apple-aspen-config`), not signed:
   - **One Wi-Fi payload** for the Wi-Fi name of the current network
     (`LanNetwork.wifi_name`, change 3): `SSID_STR`, `AutoJoin` true,
     `EncryptionType` `Any`, no Wi-Fi `Password` (unless M0 shows that U5
     needs it), and the proxy: `ProxyType` `Manual`, `ProxyServer` the Mac's
     address, `ProxyServerPort` the client port, `ProxyUsername` the client
     name, `ProxyPassword` its password. If M0 shows that U2 works, `Auto`
     with `ProxyPACURL` `http://<address>:<port>/setup/<password>/proxy.pac`
     and `ProxyPACFallbackAllowed` true is used instead.
   - **One `com.apple.security.root` payload** with the inspection CA
     certificate in DER form, only when the inspection CA exists and
     `inspect_hosts` is not empty. Never a private key.
   - `PayloadIdentifier`: the instance's bundle id plus the client name, so
     scanning again (new password, new address) replaces the old profile
     instead of adding a second one. The CA payload's identifier adds the CA
     fingerprint, so a new CA is a new certificate.
   - Display name: "LocalRouter proxy for *Wi-Fi name*". Description: "Sends
     this iPhone's traffic on *Wi-Fi name* through *Mac name*, which can read
     it. Remove this profile to stop."
   - With no Wi-Fi name known, there is no phone profile: the page shows only
     the manual way, and the Mac's panel asks for the name (change 3).
3. **The setup URL** is built by the daemon:
   `http://<address>:<port>/setup/<password>`. `<address>` is the IPv4
   address of the interface that `status.network` names. It is plain `http`:
   the phone does not trust any LocalRouter CA yet. With no recognised
   network or no IPv4 address there is no URL, and `get_proxy` says why. M0
   also checks whether the Mac's Bonjour name (`<name>.local`) works as
   `ProxyServer` (U7); if it does, the profile uses the name, and a new
   address no longer breaks the phone.
4. **The page** is one static HTML page, with no script, in the style of the
   existing pages (`proxy::page`). Headers: `Cache-Control: no-store`,
   `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'`,
   `Referrer-Policy: no-referrer`. Its content:
   1. A large **Install** button (a link to `phone.mobileconfig`), and under
      it: "Then open Settings → Profile Downloaded → Install, and enter this
      iPhone's passcode." If the page is not in Safari: "Open this page in
      Safari; other browsers cannot install profiles."
   2. Only when the profile carries the CA: "To read HTTPS: Settings →
      General → About → Certificate Trust Settings → turn on *LocalRouter
      Inspection CA …*. Until then, HTTPS sites fail on this phone. The Mac
      reads this phone's HTTPS and keeps it in the proxy log, with cookies."
      Apps that pin their certificates (many banking apps) fail while their
      host is inspected; take the host out of the inspect list on the Mac.
   3. "Check: open any site. The Proxy tab on the Mac lists this phone's
      requests."
   4. **To stop**: "Settings → General → VPN & Device Management →
      *LocalRouter proxy* → Remove Profile." With `Manual` (no PAC fallback)
      it adds: "While the profile is installed, this phone has no internet on
      *Wi-Fi name* when the Mac sleeps or the proxy is off." If M0 shows U6
      (removal forgets the network), it adds: "Removing it also forgets
      *Wi-Fi name*; join it again with its password."
   5. **Did not work?** A `<details>` block, closed: the manual way, with the
      four values (Server, Port, Username, Password) and a link to
      `ca.mobileconfig`.
5. **Setup requests are never logged.** Their path holds the password. They
   are answered before the request log and the HAR log see them, like the
   viewer (ADR 08 I8). A wrong `/setup/...` is logged as today's 400, with the
   path cut to `/setup/…`.

## Trade-offs

- **The profile replaces the phone's settings for that Wi-Fi network.** With
  a profile installed, the user cannot change the proxy of that network by
  hand (believed; M0 checks it). To stop, the user removes the profile.
- **A Wi-Fi name is needed.** macOS hides the Wi-Fi name from a program
  without Location Services permission (ADR 08, change 4). The app asks for
  that permission once, or the user types the name (change 3). The daemon
  never asks.
- **The page is in English only.** iOS menu names change with the phone's
  language. Accepted for now.
- **No signed profile.** iOS shows "Not Verified" for every self-made profile,
  including those of Charles and mitmproxy.
- **The local CA is not in the profile.** Whether Safari on iOS sends
  `*.localhost` names to the proxy is not known (U1, M6).

## Tests

M0 first (before task 1). Then T8 (the page and its paths), T9 (both
profiles), T10 (setup requests are not logged), T11 (the setup URL), M3.
See the [test plan](07-test-plan.md).
