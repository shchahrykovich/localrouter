# 2. The setup page and the CA profile

## Context

A QR code can only give the phone a URL or a piece of text. iOS has no URL
that changes the Wi-Fi proxy: the user must go to Settings → Wi-Fi → (i) →
Configure Proxy and type the server, port, user name and password. To read
HTTPS (inspection, ADR 06 change 2), the phone must also install the
inspection CA as a profile and turn on full trust for it.

So the QR code opens a page that tells the user, on the phone, exactly what to
type and gives the CA as a file iOS can install.

How iOS takes a proxy and a CA, as known on 2026-10-06 (each step is checked
by M3 on a real iPhone):

| Step on the iPhone | Where |
|---|---|
| Manual proxy for one Wi-Fi network: server, port, Authentication with user name and password | Settings → Wi-Fi → (i) → Configure Proxy → Manual |
| Install a CA: download a `.mobileconfig` in Safari, then install it | Settings → Profile Downloaded → Install |
| Trust the CA for TLS | Settings → General → About → Certificate Trust Settings |
| Remove the CA | Settings → General → VPN & Device Management |

The proxy setting belongs to one Wi-Fi network. On another Wi-Fi or on
cellular the phone does not use it.

## Decision

1. **The page lives on the phone port.** A request without a full URL
   (`GET /a`) on a proxy port gets the 400 page today
   ([forward.rs:425](../../../libs/core/src/forward.rs)). On a LAN client
   port, `GET /setup/<password>` gets the setup page instead, and
   `GET /setup/<password>/ca.mobileconfig` gets the profile. The password
   in the path is the key: the QR code holds it, and a person who scanned it
   could read the password on the page anyway. Any other path, or an old or
   wrong password, gets the same 400 page as today. The page is served to a
   peer that passes the LAN check of change 1 step 3, and to this Mac (so
   the user can open it on the Mac to look).
2. **The setup URL is built by the daemon**:
   `http://<address>:<port>/setup/<password>`. `<address>` is the IPv4
   address of the interface that `status.network` names (the interface of
   the default route). It is plain `http`: the phone does not trust any
   LocalRouter CA yet. With no recognised network or no IPv4 address there is
   no URL, and `get_proxy` says why (change 3).
3. **The page** is one static HTML page, with no script, in the style of the
   existing pages (`proxy::page`). Headers: `Cache-Control: no-store`,
   `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'`,
   `Referrer-Policy: no-referrer`. Numbered steps:
   1. **Proxy.** "Settings → Wi-Fi → (i) next to *this network* →
      Configure Proxy → Manual." Then the values, large and selectable:
      Server `192.168.0.10`, Port `8878`, Authentication on, Username
      `iphone`, Password `k7mq-2xph-9tdw-r4nc`.
   2. **HTTPS** (only when the inspection CA exists and `inspect_hosts` is
      not empty): "Open this page in Safari, tap *Download the CA profile*,
      then Settings → Profile Downloaded → Install, then Settings → General →
      About → Certificate Trust Settings → turn on *LocalRouter Inspection
      CA …*. Until then, HTTPS sites fail on this phone." Apps that pin
      their certificates (many banking apps) fail while their host is
      inspected; take the host out of the inspect list on the Mac.
      The step also says: "The Mac reads this phone's HTTPS and keeps it in
      the proxy log, with cookies." With no inspection CA the step says:
      "HTTPS goes through as a tunnel; there is nothing to install."
   3. **Check.** "Open any site. The Proxy tab on the Mac lists this
      phone's requests."
   4. **When you are done.** "Configure Proxy → Off. While it is on, this
      phone has no internet on this Wi-Fi when the Mac sleeps, when the proxy
      is off, or when the Mac gets a new address. Remove the profile in
      Settings → General → VPN & Device Management."
4. **The profile** is a `.mobileconfig` with one `com.apple.security.root`
   payload that holds the inspection CA certificate in DER form, and
   nothing else. `Content-Type: application/x-apple-aspen-config`.
   `PayloadIdentifier` is the instance's bundle id plus the CA fingerprint,
   so installing the same CA again replaces the old profile, and a new CA
   (after `reset_inspect_ca`) is a new profile. It is not signed; iOS shows
   "Not Verified". The CA key never leaves `inspect-ca/`.
5. **Setup requests are never logged.** Their path holds the password. They
   are answered before the request log and the HAR log see them, like the
   viewer (ADR 08 I8 keeps the viewer out of the log). A wrong
   `/setup/...` is logged as today's 400, with the path cut to `/setup/…`.

## Trade-offs

- **The page is in English only.** iOS menu names change with the phone's
  language. A user with a German iPhone reads "Settings → Wi-Fi" and finds
  "Einstellungen → WLAN". Accepted for now.
- **No signed profile.** Signing needs a certificate iOS trusts. "Not
  Verified" is shown for every self-made CA profile, including those of
  Charles and mitmproxy.
- **The local CA is not in the profile.** Whether Safari on iOS sends
  `*.localhost` names to the proxy at all is not known (manifest
  unresolved effect U1). Until M6 answers it, the profile carries only the
  inspection CA.

## Tests

T8 (the page, its headers, wrong password gives 400), T9 (the profile
content), T10 (setup requests are not logged), T11 (the setup URL), M3.
See the [test plan](07-test-plan.md).
