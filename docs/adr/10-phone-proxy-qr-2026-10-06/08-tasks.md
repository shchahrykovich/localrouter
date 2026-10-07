# 8. Tasks

| # | Task | Depends on |
|---|---|---|
| 0 | M0 on a real iPhone: what iOS does with a Wi-Fi profile that carries a proxy | none |
| 1 | `ProxyClient.lan`, `LanNetwork.wifi_name` and `phone.rs` (password, compare, page, profiles) | 0 |
| 2 | The local-target rule in `upstream.rs` | none |
| 3 | The forward proxy on a LAN client: 407, 403, setup paths, logging | 1, 2 |
| 4 | The daemon: binding, accept check, password file, `new_proxy_password`, setup URL, API 1.7 | 1, 3 |
| 5 | CLI and MCP | 4 |
| 6 | The app: API types, `PhoneSetup`, the panel, the QR code | 4 |
| 7 | Texts: `help.md`, `note.md`, `mcp.md`, Settings → Routing, ADR 06 note | 4 |
| 8 | Verify: E1 and the manual tests M1 to M7 | 5, 6, 7 |
| 9 | Release notes, M8, ADR status | 8 |

Task 0 comes first: its answers fix the profile's fields, and U4 can stop
the design. Task 2 does not depend on it and can run at the same time. Tasks
5, 6 and 7 run in parallel after 4.

## 0. Check iOS first

**Deps:** none. **Tests:** M0.

Run M0 from the test plan with a hand-made profile and mitmproxy as the test
proxy. No LocalRouter code. Write the results into U2, U4, U5, U6 and U7 of
the manifest and decide, in this ADR: `Manual` or `Auto` (U2); the Wi-Fi
password field (U5, and where it would be kept); the text of step 4 of the
page (U6); IPv4 address or Bonjour name (U7). **If U4 fails, stop and
redesign change 1.** Done when the five answers are in the manifest and
change 2 names the chosen fields.

## 1. The config field and the phone module

**Deps:** 0. **Tests:** T1, T2, T9.

Add `lan: bool` to `ProxyClient` and `wifi_name: Option<String>` to
`LanNetwork` (both left out of `config.json` when unset). New
`libs/core/src/phone.rs`: `new_password()` from the system random source;
`same_password(a, b)` as one fold over every byte (review checkpoint for I4:
no early return); `check_basic(header, name, password)`; `setup_page(values)
-> String` with the Install button, the steps and the manual fallback of
change 2, every value escaped; `phone_profile(values) -> Vec<u8>` with the
fields task 0 chose and the Wi-Fi name as a required value (I15);
`ca_profile(ca_der) -> Vec<u8>`; `pac(address, port)` if task 0 picked
`Auto`. Done when T1, T2 and T9 pass and `plutil -lint` accepts both
profiles.

## 2. The local-target rule

**Deps:** none. **Tests:** T4.

`upstream::is_local_target(addr, own: &[IpAddr]) -> bool` for loopback,
IPv4-mapped loopback, unspecified and own addresses. `Upstream::connect`
takes a `refuse: Option<&dyn Fn(IpAddr) -> bool>` checked on every resolved
address before connecting; a refusal is a distinct error, so the forward
proxy can answer 403 instead of 502. Done when the unit cases of T4 pass and
every existing `forward.rs` test still passes (they pass `None`).

## 3. The forward proxy on a LAN client

**Deps:** 1, 2. **Tests:** T3, T4, T8, T10.

`ClientPort` gets `lan: bool` and the client's password (shared, replaced on
a new password). In `ForwardProxy::handle` and `connect`, for a LAN client
and a non-loopback peer, in this order: the setup paths (before `seen()` and
`log_http`, I9); `check_basic` or 407 (I3); the target rule or 403 (I5). The
`ForwardProxy` holds the rule as a value so tests can narrow it. Extend the
`Harness` with `fake_peer` and `lan`. Done when the T3, T4 (integration), T8
and T10 tests pass and the regression tests listed in the test plan pass.

## 4. The daemon

**Deps:** 1, 3. **Tests:** T5, T6, T7, T11, T13.

- `listen.rs`: the accept decision for a proxy port (T5, I2).
- `daemon.rs`: a LAN client binds with `bind_all` (I1 stays for the others);
  `accept_proxy` runs `lan_peer_allowed` off the accept loop for a LAN
  client, as `accept_http` does; the password file with mode `0600` (I7),
  its order in `set_config` (data-flow file, flow A) and at start (I12); the
  per-client connection token and `new_proxy_password` (I11).
- `network.rs`: the IPv4 address of an interface; `LOCALROUTER_TEST_NETWORK`
  takes an optional 4th field, the address (debug builds only).
- `api.rs`: the fields (`lan`, `wifi_name`, `status.network.wifi_name`),
  `GetProxyResult.lan` with `profile`, `new_proxy_password`, `API_VERSION`
  1.7; `socket.rs` dispatch; `api/examples/` files (T13).
- The setup paths read `wifi_name` of the current network for the phone
  profile; with none, `phone.mobileconfig` is 400 (I15).
- Extend `no_reply_contains_key_material` to the password (I6).

Done when T5, T6, T7, T11 and the Rust half of T13 pass.

## 5. CLI and MCP

**Deps:** 4. **Tests:** T14, T15.

`proxy client add --lan`, `proxy client setup`, `proxy client password
--new`, `lan name <wifi name>`. The MCP `get_proxy` tool removes `lan.password` and `lan.setup_url`
and adds the line about the Proxy tab (I8). `apps/cli/tests/mcp.rs` still
counts nine tools. Done when T14 and T15 pass.

## 6. The app

**Deps:** 4. **Tests:** T12, T13.

`Api.swift`: the fields and the method. New `LocalRouterKit/PhoneSetup.swift`:
the checklist from `status` and `get_proxy` (with the Wi-Fi name row), the
next free name, the QR payload, the "scan again" note (testable without UI).
New `LocalRouter/WifiName.swift`: the only place that asks for Location
Services (I14) and reads the name with CoreWLAN. `scripts/build-app.sh`:
`NSLocationUsageDescription` in `Info.plist`. New `Views/PhoneView.swift`:
the two states of change 3, the QR image from `CIFilter.qrCodeGenerator()`
scaled without smoothing, black on white in both appearances, "Type by
hand", the "Remove Phone" warning, the guest Wi-Fi line. `ProxyView.swift`:
the "Phone…" button. Done when T12 and the Swift half of T13 pass and the
panel opens in the dev app.

## 7. Texts

**Deps:** 4. **Tests:** the existing `no_fixed_names.rs`, `agent_texts.rs`.

`help.md`, `note.md`, `mcp.md`: the proxy is this Mac only except a phone
client; `127.0.0.1` does not work from a phone; a phone is set up in the
app; an agent cannot set it up (G7). Settings →
Routing text at `SettingsView.swift:177`. A one-line note in ADR 06
decision 2 pointing to this ADR. Done when the text tests pass and
`router.localhost` shows the new lines.

## 8. Verify

**Deps:** 5, 6, 7. **Tests:** E1, M1, M2, M3, M4, M5, M6, M7.

Write E1 in `apps/cli/tests/e2e.rs`. Install the dev app
(`scripts/install.sh --user --launch`) and run M1 to M7 with a real iPhone.
Check that M2 and M3 behave as M0 found with the test proxy; a difference is
a bug in LocalRouter's 407 or profile. Write the result of M6 into U1.
Record the 10-minute idle count of M2 in the manifest's data impact.

## 9. Release notes, smoke test, status

**Deps:** 8. **Tests:** M8.

Release notes from the announcement in the working-backwards file. After the
release, run M8. Append the Actual Change Manifest and a plan-versus-actual
file; set the README status to as-built, with each difference marked.

## Invariants to tasks

| Invariant | Task |
|---|---|
| I1 main port and plain clients loopback only | 4 |
| I2 LAN peer only with allow_lan and an allowed network | 4 |
| I3 407 without the password | 3 |
| I4 the compare looks at every byte | 1 |
| I5 403 for the Mac's own addresses | 2, 3 |
| I6 the password nowhere else | 3, 4 |
| I7 the file is 0600 | 4 |
| I8 MCP has no password | 5 |
| I9 setup requests not logged | 3 |
| I10 the profiles hold only what change 2 lists | 1 |
| I11 a new password closes the connections | 4 |
| I12 a LAN client always has a password | 4 |
| I13 /setup only on a LAN client | 3 |
| I14 only the app asks for Location Services | 6 |
| I15 no phone profile without a Wi-Fi name | 1, 4 |

Every test ID of the test plan is in a task: M0 (0); T1, T2, T9 (1); T4 (2, 3); T3,
T8, T10 (3); T5, T6, T7, T11, T13 (4); T14, T15 (5); T12, T13 (6); E1, M1 to
M7 (8); M8 (9).
