# 3. The QR code in the Proxy tab, the socket API and the CLI

## Context

The user asked: "on proxy page show QR code for iphone to route traffic via
proxy" (2026-10-06). The Proxy tab today has the proxy switch, the log
buttons, a `proxy env` line, "Open Chrome via Proxy" and the inspect button
([ProxyView.swift](../../../apps/menubar/Sources/LocalRouter/Views/ProxyView.swift)).
ADR 09 left clients to the CLI: "The menu bar app does not manage clients
yet."

A phone needs four things to be true at once: the proxy on, LAN access on,
this network allowed, and a LAN client. Today these live in three places
(Proxy tab, Settings → Routing, the CLI). The QR panel puts them in one.

## Decision

1. **A "Phone…" button** in the Proxy tab, next to "Open Chrome via Proxy".
   It replaces the request list of the tab with the phone panel; "Done" goes
   back. The panel shows one of two states.
2. **No phone yet: a checklist and one button.**

   ```
   Use the proxy from a phone on this Wi-Fi
   ✓ Forward proxy on
   ✗ LAN access is off                      [Turn On]
   ✗ This network (router 192.168.0.1) is not allowed   [Allow]
                                            [Set Up a Phone]
   ```

   Each ✗ row has the button that fixes it, with the same calls Settings →
   Routing uses (`set_config` with `allow_lan`, `lan_networks`). "Set Up a
   Phone" is enabled when every row is ✓. It adds the client `iphone` (or
   `iphone-2`, `iphone-3` when the name is taken) with `lan: true` and the
   next free port after the proxy ports, as `proxy client add` does.
3. **A phone exists: the QR code and the values.**

   ```
   ┌────────────┐  Scan with the iPhone camera.
   │ ▓▓ ▓ ▓▓▓▓ │  Server    192.168.0.10        [copy]
   │ ▓ ▓▓▓ ▓ ▓ │  Port      8878                [copy]
   │ ▓▓▓ ▓ ▓▓▓ │  Username  iphone              [copy]
   └────────────┘  Password  k7mq-2xph-9tdw-r4nc [copy]
   HTTPS sites fail on the phone until it trusts the CA (step 2 on the page).
   The phone has no internet on this Wi-Fi while the Mac sleeps.
   [New Password]   [Remove Phone]                         [Done]
   ```

   - The QR code holds the setup URL from `get_proxy` and nothing else. It
     is drawn with Core Image (`CIFilter.qrCodeGenerator()`, correction
     level M), scaled without smoothing. No new dependency.
   - When a condition of step 2 fails later (another Wi-Fi, LAN access
     turned off), the QR code is hidden and the failing row is shown with
     its button, so a user never scans a code that cannot work.
   - When the Mac's address differs from the one the phone was set up with,
     the phone must get the new server value. The panel cannot know what
     the phone has; it shows the current address in large type.
   - The phone's requests already appear in the Proxy tab's request list,
     like every proxied request. The panel adds nothing for that.
   - Under the QR code: "If the page does not open: the phone must be on the
     same Wi-Fi as the Mac. Some guest networks block devices from reaching
     each other." The Mac cannot detect this.
   - "Remove Phone" asks first: "Turn Configure Proxy off on the phone
     first. Otherwise the phone has no internet on this Wi-Fi." Removing the
     client deletes its password and closes its port.
4. **Socket API 1.7** (minor: only additions).

   | Change | Shape |
   |---|---|
   | `ProxyClient.lan` | `bool`, in `get_config`, `set_config.proxy_clients`, `status.proxy.clients[]` |
   | `get_proxy` with `client` of a LAN client | reply gains `lan: { "address": "192.168.0.10", "port": 8878, "username": "iphone", "password": "k7mq-…", "setup_url": "http://…", "problems": [] }`; `address` and `setup_url` are `null` when the network is not recognised, and `problems` says why. Not present for a client without `lan` |
   | new method `new_proxy_password` | params `{ "client": "iphone" }`, reply `{ "password", "setup_url" }`; error `not_found` for an unknown client, `invalid_request` for a client without `lan` |

   `api/examples/` gets `get_proxy_lan_client`, `set_config_proxy_clients_lan`
   and `new_proxy_password`; both contract tests walk them.
5. **CLI.** `proxy client add <name> --lan [--port N]`;
   `proxy client setup <name>` prints the four values and the setup URL;
   `proxy client password <name> --new`. The CLI does not draw a QR code in
   the terminal: that needs a new crate, and the app does it.
6. **MCP: nothing new, one thing removed.** There are still nine tools. The
   `get_proxy` tool removes `lan.password` and `lan.setup_url` from its
   reply, and says "A phone is set up in the app: Proxy tab → Phone…". The
   same rule as ADR 08: an agent must not open the Mac to a network, and it
   must not hand out the key to it.
7. **Texts that say "the proxy is this Mac only"** change to "the proxy is
   this Mac only, except a phone client (ADR 10)": `help.md`, `note.md`,
   `mcp.md`, and Settings → Routing
   ([SettingsView.swift:177](../../../apps/menubar/Sources/LocalRouter/Views/SettingsView.swift)).
   The agent texts (`note.md`, `mcp.md`, `help.md`) also say: "The proxy
   address `127.0.0.1` does not work from a phone: it is the phone itself. A
   phone is set up in the app, Proxy tab → Phone…"

## Trade-offs

- **The panel replaces the request list while it is open.** The popover is
  small; a QR code next to the list would make both too small to use.
- **Only one action to set up.** "Set Up a Phone" does not turn on LAN
  access for the user. The user presses each ✗ button on purpose: allowing
  a network opens ports 80 and 443 too, and the user must see that.

## Tests

T12 (QR payload and panel state, Swift), T13 (API examples and contract
tests), T14 (CLI), T15 (MCP strips the password), M1, M7. See the
[test plan](07-test-plan.md).
