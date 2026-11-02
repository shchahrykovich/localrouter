# 10. Tasks

| # | Task | Depends on |
|---|---|---|
| 0 | M0: answer U3 to U7 | none |
| 1 | Core: nettop parser and attribution | 0 (U4) |
| 2 | Core: the forward proxy `peer_port` table | none |
| 3 | Core: history files and retention | 1 |
| 4 | Core: geo module | 0 (U5) |
| 5 | Daemon: sampler, config, API 1.9, geo download | 1, 2, 3, 4 |
| 6 | Swift: API types, app grouping, version gate | 5 |
| 7 | Swift: app identity, VirusTotal, review rules | 6 |
| 8 | Swift: Ask AI | 7 |
| 9 | App UI: Apps view, details window, map window, menu, Settings, uninstall | 6, 7, 8 |
| 10 | Texts: dictionary, Settings keywords, ADR links | 9 |
| 11 | Verify: E1 and M1 to M7 | 9, 10 |
| 12 | Release notes, M8, ADR status | 11 |

Tracks that run in parallel: task 2 next to tasks 1 and 3; task 4 next to
1 to 3. Tasks 7 and 8 can start on fake data before task 5 is done, if the
`Api.swift` types of task 6 are written first.

## 0. Answer the unknowns

**Deps:** none. **Tests:** M0.

Run M0. Write each answer into U3 to U7 of the
[manifest](08-semantic-change-manifest.md). If nettop on macOS 14 or 15 prints
another format, add a second fixture and a note in [01](01-app-traffic.md). If
nettop costs more than 1% CPU at 5 s, change the default interval in 01 and in
the test plan. If a chat link ignores `q`, write "copy fallback" for it in
[04](04-ask-ai.md). Done when every U3 to U7 has a written answer.

## 1. Core: nettop parser and attribution

**Deps:** 0. **Tests:** T1, T2.

New `libs/core/src/traffic/mod.rs`, `nettop.rs`, `attribute.rs`, and the
fixture `testdata/nettop-two-samples.csv` (addresses changed to documentation
ranges). `nettop::parse_sample(&str) -> Result<Sample, ParseError>`; a sample
holds processes with pid, cut name and connections (protocol, local, remote,
bytes in, bytes out). `attribute::apply(sample, &Ports, daemon_pid,
&PeerTable, &dyn Fn(pid) -> Option<PathBuf>) -> Vec<AppBytes>` applies the
`via` rules (I3), drops the daemon (I2), joins the proxy table, and groups by
app path. No I/O except the path callback. Done when T1 and T2 pass.

## 2. Core: the forward proxy `peer_port` table

**Deps:** none. **Tests:** T4.

In `forward.rs`, a `PeerTable` (a `Mutex<VecDeque>` or a small map with a
size cap of 4,096, oldest dropped). Insert on accept with the client name;
fill host and upstream address when the target is known; add bytes in the
existing copy loops without awaiting the lock across I/O (I13); keep closed
entries 30 s. The table is shared with the daemon by `Arc`. **Review
checkpoint:** the lock is never held across an `.await`. Done when T4 and all
existing `forward.rs` tests pass.

## 3. Core: history files and retention

**Deps:** 1. **Tests:** T3.

`traffic/history.rs`: memory buckets of 5 s for the last hour; one NDJSON line
per active pair at the end of each minute, appended with `O_APPEND`; reading
day files for `24h`, `7d`, `30d` into 96 buckets, cached until the next minute
line (G5); retention by days and by the
500 MB cap (I4); folder `0700`, files `0600`. Done when T3 passes. Then run it
for one day on a real Mac (with task 5) and write the measured day-file size
into the manifest, section 10.

## 4. Core: geo module

**Deps:** 0. **Tests:** T5.

Add the `maxminddb` crate. `traffic/geo.rs`: `Places::open(dir)`,
`Places::lookup(IpAddr) -> Option<Place>` (city, country, lat, lon, owner);
`download_urls(year, month)`; `install(fetch, dir)` that writes `.part`,
opens it, then renames (I11). The fetch function is a parameter, so tests use
a fake. Done when T5 passes.

## 5. Daemon: sampler, config, API 1.9, geo download

**Deps:** 1, 2, 3, 4. **Tests:** T6, T7, T8, E1.

- `libs/core/src/config.rs`: `traffic`, `traffic_history_days`,
  `traffic_places`, `#[serde(default)]`, left out when unset.
- `libs/core/src/api.rs`: `GetAppTrafficParams`, `AppTrafficResult` and
  parts, `UpdateGeoDbResult`, `status.traffic`, `status.traffic_error`;
  `METHODS` gains `get_app_traffic` and `update_geo_db`; `API_VERSION`
  `1.9`.
- `api/examples/`: `get_app_traffic.request.json`, `.reply.json`,
  `update_geo_db.request.json`, `.reply.json`,
  `set_config_traffic.request.json` (I15).
- `apps/daemon/src/traffic.rs`: start and stop the child with the config
  (I1); `LOCALROUTER_TEST_NETTOP` in debug builds; backoff 1 s to 60 s and
  `traffic_error` (I5), including "nettop output not understood";
  `proc_pidpath` through `libc`; the minute writer; retention at start, every
  hour and on `set_config`.
- `socket.rs`, `daemon.rs`: the two methods. `update_geo_db` downloads with
  `upstream::platform_tls()` only.

Done when T6, T7, T8 pass and E1 passes on a Mac.

## 6. Swift: API types, app grouping, version gate

**Deps:** 5. **Tests:** T14, contract tests.

`LocalRouterKit/Api.swift`: the new types and fields. `Traffic.swift`: a
`TrafficView` mode enum (`apps`, `requests`) with the last choice in
`UserDefaults`; byte formatting up to GB; `AppsAvailability(apiVersion:)`
that returns `.tooOld` for `1.8` (I16). Done when `ApiContractTests` and T14
pass.

## 7. Swift: app identity, VirusTotal, review rules

**Deps:** 6. **Tests:** T9, T10, T11.

- `AppIdentity.swift` as in [03](03-app-checks.md), 3.1.
- `VirusTotal.swift`: lookup (GET, no body, I6), upload taking an
  `UploadConfirmation` whose initializer is private to the confirm dialog's
  file (I9, **review checkpoint**), limiter 4 a minute and 500 a day, key in
  the Keychain under the instance service name (I8), Apple results skipped
  (I7).
- `AppChecks.swift`: the cache file, 7-day expiry, the "needs review" rule,
  trusted SHA-256 list in `UserDefaults` (I12), `canQuit` false for `apple`
  (G6), the notification summary rule (G7).

Before release: the answer to U6. Done when T9, T10, T11 pass.

## 8. Swift: Ask AI

**Deps:** 7. **Tests:** T12.

`AskAI.swift`: `question(for:includeAddresses:) -> String` and `url(for:
service:) -> URL` with the rules of [04](04-ask-ai.md) (I10), and the copy
fallback chosen in task 0. Done when T12 passes.

## 9. App UI

**Deps:** 6, 7, 8. **Tests:** T13, T15, M1 to M4.

From the canvas (see the table in [05](05-ui.md)):

- `Views/TrafficView.swift`: the `Apps / Requests` switch. New
  `Views/AppsView.swift`: rows, open row with addresses, spark bars, the
  totals strip and histogram, the review banner, the off card, the too-old
  view.
- `AppDetailsWindowController.swift`, `Views/AppDetailsView.swift`:
  [03](03-app-checks.md), 3.5, with the upload and quit dialogs.
- `TrafficMapWindowController.swift`, `Views/TrafficMapView.swift`,
  `LocalRouterKit/MapProjection.swift`, the resource `land-dots.json`. Make the
  resource once with [tools/gen-land-dots.mjs](tools/gen-land-dots.mjs)
  (Natural Earth `land-50m`, a 6-point grid on the 760 × 446 crop; see
  [tools/README.md](tools/README.md)), and move the generator to
  `scripts/map/`. T13.
- `AppMenu.swift`, `StatusItemController.swift`: the Traffic group, the
  Review items, the red dot on the icon. T15.
- `SettingsTree.swift`, `Views/SettingsView.swift`: the Traffic page. T15.
- Notifications with `UNUserNotificationCenter`, one per new executable that
  needs review.
- `Uninstaller.swift`: remove `app-checks.json`, the Keychain item and the
  trusted list.

Done when T13 and T15 pass and M1 to M4 pass on the `-dev` instance.

## 10. Texts

**Deps:** 9. **Tests:** `no_fixed_names` tests.

- `docs/dictionary.md`: an entry **via (of an app connection)** with
  `router`, `proxy`, `direct`, and a line in "Words we use" that says
  "router" is allowed only as this value.
- `CLAUDE.md`: one line for ADR 11 in the ADR list, and one bullet in "Things
  that are easy to get wrong": the daemon pid drop and "nettop is a tool, not
  an API".
- `help.md`, `note.md`, `mcp.md`: no change (no new CLI or MCP tool).

Done when the no-fixed-names tests pass.

## 11. Verify

**Deps:** 9, 10. **Tests:** E1, M1 to M7.

Run E1 on a Mac. Run M1 to M7 on the `-dev` instance and tick each table.
Write any difference from the design into the ADR as a note.

## 12. Release

**Deps:** 11. **Tests:** M8.

Release notes: "Traffic → Apps: see which apps use the network, where it goes
on a map, and check who signed them". After release run M8. Set the README
status to "Accepted, built" and the manifest status; append the Actual Change
Manifest.

## Invariants to tasks

| Invariant | Task |
|---|---|
| I1 | 5 |
| I2 | 1 |
| I3 | 1 |
| I4 | 3 |
| I5 | 5 |
| I6 | 7 |
| I7 | 7 |
| I8 | 7 |
| I9 | 7 |
| I10 | 8 |
| I11 | 4, 5 |
| I12 | 7 |
| I13 | 2 |
| I14 | 5 (no MCP change; `mcp.rs` must pass unchanged) |
| I15 | 5 |
| I16 | 6 |
