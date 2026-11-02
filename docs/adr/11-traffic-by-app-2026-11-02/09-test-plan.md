# 9. Test plan

## What the repo can run today

| Kind | Tool | Where |
|---|---|---|
| Rust unit tests | `cargo test` | `#[cfg(test)] mod tests` inside each file (`libs/core/src/*.rs`, `apps/daemon/src/*.rs`) |
| Rust integration | `cargo test` | `libs/core/tests/*.rs` (`forward.rs`, `proxy.rs`, `api_examples.rs`, `no_fixed_names.rs`), `apps/daemon/tests/api.rs` (starts a real daemon in a temp `LOCALROUTER_HOME`), `apps/cli/tests/*.rs` (`e2e.rs`, `mcp.rs`) |
| Swift unit tests | `swift test --package-path apps/menubar` | `apps/menubar/Tests/LocalRouterKitTests/` (only `LocalRouterKit`; the `LocalRouter` app target and its views have no tests) |
| Contract tests | both of the above | `api_examples.rs` and `ApiContractTests.swift` walk `api/examples/`: a new method without examples fails by itself |
| UI tests | none | no XCUITest, no snapshot tests |

So every rule that can be tested lives in `libs/core` or `LocalRouterKit`, not
in a view. The views are checked by hand (M1 to M4).

## Overview

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| T1 | unit (core) | the nettop parser | parser | nettop output: a captured fixture | CI |
| T2 | unit (core) | `via` rules, daemon pid drop, proxy join, app paths | attribution | samples and paths: built in the test | CI |
| T3 | unit (core) | minute lines, reading back, retention | history, real temp folder | the clock | CI |
| T4 | integration (core) | the proxy `peer_port` table | real `ForwardProxy`, real sockets | upstream: local test server | CI |
| T5 | unit (core) | geo URL rule, month fallback, `.part` then rename | geo module | the downloader and the database reader: fakes | CI |
| T6 | integration (daemon) | `get_app_traffic`, config fields, places off means no `geo/` | real daemon, socket | nettop: `LOCALROUTER_TEST_NETTOP` script | CI |
| T7 | integration (daemon) | sampler starts and stops with `traffic` | real daemon, real child process | nettop: the test script | CI |
| T8 | integration (daemon) | sampler failure does not stop routing; backoff | real daemon, real route | nettop: a script that exits | CI |
| T9 | unit (Swift) | signature statuses and SHA-256 | Security.framework, real binaries made in the test | nothing | CI |
| T10 | unit (Swift) | VirusTotal client: GET only, parsing, limits, Apple skipped, key only in Keychain | client, limiter | network: `URLProtocol` stub; Keychain: a test service name | CI |
| T11 | unit (Swift) | "needs review" rule, trusted by SHA-256, cache expiry | rules, cache file in a temp folder | check results: built in the test | CI |
| T12 | unit (Swift) | the Ask AI question and links | `AskAI` | nothing | CI |
| T13 | unit (Swift) | map projection, arcs, time zone places | `MapProjection` | nothing | CI |
| T14 | unit (Swift) | app grouping helpers, the API version gate | helpers | nothing | CI |
| T15 | unit (Swift) | menu items and the Settings page | `AppMenu`, `SettingsTree` | the context: built in the test | CI |
| E1 | end-to-end (daemon) | one app's router, proxy and direct traffic, read by the real nettop | real daemon, real nettop, real route, real proxy | the internet: a listener on the Mac's own LAN address | local, macOS only |
| M0 | manual, before code | the seven unknowns U3 to U7 | everything real | none | once |
| M1 to M7 | manual | screens, real apps, real VirusTotal, failures | everything real | none | before release |
| M8 | manual | post-release smoke | the released build | none | after release |

Replaced parts and who runs them for real:

| Replaced in | Part | Run for real in |
|---|---|---|
| T1, T2, T6, T7, T8 | nettop | E1, M1, M7 |
| T5 | DB-IP download and `.mmdb` reading | M3 |
| T10 | VirusTotal | M5 |
| T10 | the user's Keychain | M5 |
| T11 | real check results | M4 |
| all | the views | M1 to M4 |

**Gap:** no automated test reads a real `.mmdb` file. A small test database
needs a writer tool the repo does not have. M3 covers it by hand.

## 1. Automated tests (CI)

### Core: `libs/core/src/traffic/`

Fixture: `libs/core/src/traffic/testdata/nettop-two-samples.csv`, captured with
the command in [01](01-app-traffic.md), with addresses changed to documentation
ranges (`192.0.2.0/24`, `203.0.113.0/24`, `2001:db8::/32`).

| Test file | Case |
|---|---|
| `nettop.rs` | two samples split at the header line; the first is marked baseline |
| `nettop.rs` | IPv4 `addr:port`, IPv6 `addr.port`, IPv6 with `%en0` scope |
| `nettop.rs` | listening sockets (`*`, empty counters) are skipped |
| `nettop.rs` | a cut process name keeps its pid (`localrouterd-de.913` → pid 913) |
| `nettop.rs` | a sample whose lines all fail to parse returns an error, not an empty sample |
| `attribute.rs` | loopback to the HTTP port → `router`; to the HTTPS port → `router`; to a TCP route listen port → `router` |
| `attribute.rs` | loopback to the main proxy port and to a proxy client port → `proxy` |
| `attribute.rs` | loopback to another port → `direct` with no place; LAN or public remote → `direct` |
| `attribute.rs` | the daemon pid's lines are dropped; total of all apps equals total minus the daemon (I2) |
| `attribute.rs` | proxy join: a `proxy` connection whose local port is in the peer table gets its host and upstream address; one that is not keeps `host: null` |
| `attribute.rs` | app path: outermost `.app`; `/usr/libexec/…` → macOS services; `None` path → nettop name with "(exited)" |
| `history.rs` | buckets of 5 s add up to the minute line |
| `history.rs` | a day file with a half last line reads back without that line |
| `history.rs` | two `7d` reads in the same minute open the day files once (G5) |
| `history.rs` | retention with `days = 1` deletes yesterday's file at midnight + 1 day; the 500 MB cap deletes the oldest first (I4) |
| `geo.rs` | the URL for `2026-11`; 404 falls back to `2026-10` (fake fetcher) |
| `geo.rs` | a failed open of the `.part` file leaves the old file and deletes the `.part` |

```
cargo test -p localrouter-core --lib traffic
```

### Core integration: `libs/core/tests/forward.rs`

| Test file | Case |
|---|---|
| `forward.rs` | a request through the proxy adds one `peer_port` entry with host and upstream address; it stays 30 s after close |
| `forward.rs` | 5,000 connections keep the table at 4,096 entries (I13) |
| `forward.rs` | regression: every existing test still passes |

```
cargo test -p localrouter-core --test forward peer_port
```

### Daemon: `apps/daemon/tests/api.rs`

`LOCALROUTER_TEST_NETTOP` (debug builds only) names a shell script that
prints the fixture, then sleeps; a second script prints one sample and exits.
Test config as today: ports `0`, `"inspect_hosts":[]`.

| Test file | Case |
|---|---|
| `api.rs` | `traffic` off: `status.traffic == "off"`, no child process, no `traffic/` folder (I1) |
| `api.rs` | `set_config traffic: true` starts the child; `false` ends it within 2 s (I1) |
| `api.rs` | `get_app_traffic range: 1h` returns the fixture's apps, sorted by bytes in, with `by_via` |
| `api.rs` | `traffic_places: false`: no `geo/` folder, `place` is null (I11) |
| `api.rs` | the exiting script: `status.traffic == "restarting"`; a route request still answers 200 (I5) |
| `api.rs` | `set_config` round trip of the three fields; unset fields are left out of `config.json` |

```
cargo test -p localrouterd --test api traffic
```

### Contract

`api/examples/get_app_traffic.request.json`, `.reply.json`,
`update_geo_db.request.json`, `.reply.json`, `set_config_traffic.request.json`.
The two contract tests walk the folder and need no edit (I15).

```
cargo test -p localrouter-core --test api_examples && swift test --package-path apps/menubar --filter ApiContractTests
```

### Swift: `apps/menubar/Tests/LocalRouterKitTests/`

| Test file | Case |
|---|---|
| `AppIdentityTests.swift` | `/bin/ls` → `apple` |
| `AppIdentityTests.swift` | a copy of `/usr/bin/true` re-signed with `codesign -s - -f` → `adhoc` |
| `AppIdentityTests.swift` | the same copy after `codesign --remove-signature` → `unsigned` |
| `AppIdentityTests.swift` | a signed copy with one byte changed after signing → `broken` |
| `AppIdentityTests.swift` | SHA-256 equals `shasum -a 256` of the same file |
| `VirusTotalTests.swift` | lookup is `GET /api/v3/files/<sha256>` with `x-apikey` and no body (I6) |
| `VirusTotalTests.swift` | 200 parses stats and label; 404 is "unknown"; 429 waits |
| `VirusTotalTests.swift` | an `apple` result is never sent (I7) |
| `VirusTotalTests.swift` | the fifth lookup in one minute waits; the 501st in a day is refused |
| `VirusTotalTests.swift` | after saving a key: not in `UserDefaults`, not in the cache file (I8) |
| `AppChecksTests.swift` | needs review: unsigned, broken, `malicious` 2; not for `malicious` 1; ad-hoc only with the setting |
| `AppChecksTests.swift` | trusted SHA-256 A does not trust SHA-256 B of the same path (I12) |
| `AppChecksTests.swift` | a cached lookup older than 7 days is looked up again |
| `AppChecksTests.swift` | `canQuit` is false for `apple` programs (G6) |
| `AppChecksTests.swift` | 5 programs needing review within 10 minutes give one summary notification (G7) |
| `AskAITests.swift` | the canvas example gives the expected text |
| `AskAITests.swift` | `/Users/<name>/…` becomes `~/…`; the name appears nowhere (I10) |
| `AskAITests.swift` | addresses left out when the setting is off; URL under 2,000 characters with 500 addresses (I10) |
| `MapProjectionTests.swift` | London → (340, 137) and Sydney → (737, 389) on the 760 × 446 map, ±1 |
| `MapProjectionTests.swift` | an arc starts at this Mac and ends at the place |
| `MapProjectionTests.swift` | `Europe/London` → London; an unknown id → nil |
| `TrafficTests.swift` | grows: app row grouping helpers, byte formatting for GB |
| `TrafficTests.swift` | the version gate: `1.8` → "too old" view, `1.9` → Apps view (I16) |
| `AppMenuTests.swift` | grows: no Traffic group when the sampler is off; at most three Review items |
| `SettingsTreeTests.swift` | grows: "virustotal" and "nettop" find the Traffic page |

```
swift test --package-path apps/menubar
```

### Rules that already guard this ADR

| Test | Guards |
|---|---|
| `apps/cli/tests/mcp.rs` | nine MCP tools (I14) |
| `libs/core/tests/no_fixed_names.rs`, `NoFixedNamesTests.swift` | no fixed instance names in new strings |

## 2. Automated end-to-end test

**E1**, `apps/daemon/tests/api.rs`, test `traffic_journey`. It runs the real
`/usr/bin/nettop`, so it is skipped when the file is missing.

1. Start the daemon with `traffic: true`, the proxy on, a route `e1` to a
   local test server, and a second listener on `0.0.0.0`.
2. Find the Mac's own LAN address (as ADR 10's E1 does). Check: it is not
   loopback.
3. From the test process, keep three connections open for 12 seconds and send
   data on each: through the route (`::1`, HTTP port), through the proxy to the
   LAN listener, and direct to the LAN listener.
4. Call `get_app_traffic range: 1h`. Check: one app whose path is the test
   binary.
5. Check: its `by_via.router`, `by_via.proxy` and `by_via.direct` are all
   above zero.
6. Check: the proxy remote has the host the test asked for.
7. Check: no app has the daemon's path.

```
cargo test -p localrouterd --test api traffic_journey -- --nocapture
```

## 3. Manual tests

### M0. Unknowns, before any code

| Check | Expected |
|---|---|
| open `https://claude.ai/new?q=hello%20test` in Safari, logged in | a new chat with "hello test" in the input, or sent; write down which (U3) |
| open `https://chatgpt.com/?q=hello%20test` | the same check (U3) |
| run `nettop -L 2 -s 2 -x -n -d -J bytes_in,bytes_out` on macOS 14 and 15 | same format as in [01](01-app-traffic.md) (U4) |
| run `nettop -L 0 -s 5 -x -n -d -J bytes_in,bytes_out > /dev/null` for 10 minutes, watch Activity Monitor | average CPU written down; under 1% (U7) |
| download both DB-IP Lite files of this month | sizes and times written into U5 |
| read the VirusTotal public API terms | written answer for U6 |

### M1. Apps view with real apps (`-dev` instance)

| Check | Expected |
|---|---|
| Traffic tab with traffic off | the card with **Turn On** |
| Turn On, wait 15 s | Chrome, Claude, `mDNSResponder` under macOS services |
| play a video in Chrome for 1 minute | Chrome's in bytes grow; Activity Monitor → Network shows the same order of size |
| Chrome opens `https://shop.localhost` | a `router` line with the host `shop.localhost` |
| `curl` through the proxy to `https://example.com` | `curl (exited)` or `curl` with `proxy` and host `example.com` |
| no `localrouterd` row anywhere | true |

### M2. Menu, Settings, states

| Check | Expected |
|---|---|
| right-click the icon with traffic on | Traffic group with the totals line, Show Apps, Open Traffic Map |
| Settings → search "virustotal" | the Traffic page |
| Keep history 1 day, then look at `<data>/traffic/` the next day | only today's file |
| stop the daemon, open the Apps view | "The daemon is not running." |
| run the new app against an old 1.8 daemon | "The daemon is too old for the Apps view" |

### M3. Map and places

| Check | Expected |
|---|---|
| Settings → Show places on the map | download runs; the date appears; credit "IP geolocation by DB-IP" is shown |
| map window, 24 hours | arcs from this Mac's time zone city; a US CDN address lands in the US |
| turn Wi-Fi off, press Update | an error; the old database still works |

### M4. App checks on real programs

| Check | Expected |
|---|---|
| Chrome details | green shield, Google's signer and team id, notarized |
| Homebrew `node` details | `ad-hoc signed`, no red banner |
| build `printf 'int main(){return 0;}' > /tmp/u.c && cc /tmp/u.c -o /tmp/u && codesign --remove-signature /tmp/u`, run it with a network call (or use a script runner that connects) | red banner, red dot on the icon, one notification, **Review** item in the menu |
| mark it trusted, then rebuild it | it needs review again (new SHA-256) |
| **Quit process** on it | a confirm dialog; after confirm the process is gone |

### M5. VirusTotal with a real key

Free. The upload sends a file to a third party: use only the test binary from
M4.

| Check | Expected |
|---|---|
| save a key in Settings | "Saved in Keychain"; with the first 8 characters of the key as `K`, `defaults read dev.localrouter.app-dev \| grep -c K` and `grep -rc K "<data folder>"` print 0 |
| Chrome after a minute | VirusTotal 0 of N, "Looked up by hash" |
| an Apple program | no VirusTotal card, "Apple programs are not looked up" |
| the M4 test binary | "VirusTotal does not know this file" |
| **Upload file to VirusTotal…** | the warning dialog; Cancel sends nothing; Upload sends, and a minute later the result appears |
| the lookups counter | grows by one per lookup |

### M6. Ask AI

| Check | Expected |
|---|---|
| **Ask Claude** on the M4 binary | a new chat with the question (or the copy fallback, per M0) |
| **Ask ChatGPT** | the same |
| the question text | `~` instead of the home folder; no user name |
| setting "Put network addresses" off | no addresses in the question |

### M7. Failures

| Check | Expected |
|---|---|
| `pkill -f "nettop -L 0"` five times | status "restarting", then "running"; routes keep working |
| turn traffic off | `pgrep -f "nettop -L 0"` prints nothing |
| Clear history | `<data>/traffic/` is empty; the Apps view starts again from zero |

### M8. Post-release smoke

| Check | Expected |
|---|---|
| install the release, turn traffic on | the Apps view fills within 15 s |
| one Keychain prompt at most, at the first key save | true |
| Map opens; a review banner appears for the M4 binary | true |

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| I1 no nettop when off | T6, T7 | M7 |
| I2 daemon pid dropped | T2, E1 | M1 |
| I3 `via` rules | T2, E1 | M1 |
| I4 retention | T3 | M2 |
| I5 failure isolation | T8 | M7 |
| I6 hash only | T10 | M5 |
| I7 Apple skipped | T10 | M5 |
| I8 key only in Keychain | T10 | M5 |
| I9 upload only after confirm | compiler (private initializer) | M5 |
| I10 question without home path | T12 | M6 |
| I11 no network for places | T5, T6 | M3 |
| I12 trusted by SHA-256 | T11 | M4 |
| I13 bounded peer table | T4 | none needed |
| I14 nine MCP tools | `mcp.rs` | none |
| I15 examples for new methods | `api_examples.rs`, `ApiContractTests.swift` | none |
| I16 version gate | T14 | M2 |

## Not in this plan without approval

| Tool | Would cover | Cost |
|---|---|---|
| XCUITest or snapshot tests for the app target | the views M1 to M4 check by hand | a new test target and CI time |
| a MaxMind DB writer (for a test `.mmdb`) | real database reads in CI | a new dev dependency |
