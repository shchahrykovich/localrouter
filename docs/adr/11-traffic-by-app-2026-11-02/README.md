# ADR 11: Traffic by app, a traffic map, and app checks

**Status:** Proposed, 2026-11-02. Not built. This ADR is the spec for the
coding agent that builds it. The design is the canvas
<https://claude.ai/artifact/FsPnM9uRe3Ev3xG8sfkujT> (artboards
`TrafficApps`, `Traffic`, `Map`, `AppInfo`, `Menu`, `SettingsTraffic`).

## Summary

**In one sentence.** The daemon reads every app's network traffic from
macOS's `nettop`, the Traffic tab shows it per app as router, proxy or direct,
a map window shows where it goes, and the app checks each program's code
signature and its VirusTotal record by hash.

**In three sentences.** Today LocalRouter shows only the requests it carries
itself, so the user cannot see which programs on the Mac talk to the
internet.

The daemon runs `nettop` while the new `traffic` setting is on, drops its own
sockets, marks each connection router, proxy or direct, keeps a history in
`<data>/traffic/`, and looks up places in a DB-IP database on the Mac. The app
shows an Apps view, a map window and a details window per app, with the code
signature, a VirusTotal lookup of the SHA-256, and buttons that open Claude or
ChatGPT with a question about the program.

**In seven sentences.** The Traffic tab today lists route and proxy requests
only; a program that connects directly is invisible.

`nettop`, which ships with macOS, prints every socket of every process with
byte counts, without root and without any entitlement, so the daemon runs it
as a child process every 5 seconds while `traffic` is on (off by default).

The daemon drops its own pid (else every route and proxy byte counts twice),
marks each connection `router`, `proxy` or `direct` by its loopback port,
joins proxy connections to the forward proxy's new `peer_port` table to learn
their host, groups processes by their outermost `.app`, and appends one
NDJSON line per active pair per minute to `<data>/traffic/`, kept 1, 7 or 30
days and at most 500 MB.

Socket API 1.9 adds `get_app_traffic` and `update_geo_db`; places come from
DB-IP Lite `.mmdb` files that the daemon downloads only when the user asks,
and the map is a SwiftUI `Canvas` of land dots, so no map tiles are fetched.

The app checks each executable with Security.framework (Apple, Developer ID,
ad-hoc, unsigned, broken), looks up its SHA-256 on VirusTotal with the user's
own key from the Keychain, uploads a file only after a warning dialog, and
marks an app "needs review" with a banner, a red dot and a menu item.

The trade-off: nettop gives addresses, not host names, so direct traffic
shows the network owner and place instead of `github.com`; host names and
blocking need a network filter, which is left for a later ADR.

Tests: core unit tests on a captured nettop fixture, daemon tests with a fake
nettop, Swift tests for the checks, the question and the projection, one
end-to-end journey with the real nettop, and manual checks on real apps and a
real VirusTotal key.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 0 | What users would say, checked against the ADR | simulation | [00-working-backwards.md](00-working-backwards.md) |
| 1 | Traffic by app: the nettop sampler, router/proxy/direct, history, API 1.9 | feature | [01-app-traffic.md](01-app-traffic.md) |
| 2 | Places and the traffic map window | feature, UI | [02-places-and-map.md](02-places-and-map.md) |
| 3 | App checks: code signature and VirusTotal | feature | [03-app-checks.md](03-app-checks.md) |
| 4 | "What is this?": ask Claude or ChatGPT by a link | feature | [04-ask-ai.md](04-ask-ai.md) |
| 5 | The screens, from the canvas to the Swift files | UI | [05-ui.md](05-ui.md) |
| 6 | Where the code lives | overview | [06-components.md](06-components.md) |
| 7 | The new flows | data flow | [07-data-flow.md](07-data-flow.md) |
| 8 | What exists and happens afterwards | manifest | [08-semantic-change-manifest.md](08-semantic-change-manifest.md) |
| 9 | The tests that prove it | test plan | [09-test-plan.md](09-test-plan.md) |
| 10 | The order of work | build plan | [10-tasks.md](10-tasks.md) |

The map dots generator is in [tools/](tools/README.md).

## Why read the working-backwards file

It quotes the user's real requests from the design session and found 7 gaps.
Two changed the design most:

- **G1**: an app updated before its daemon called a method that did not exist
  and showed "No traffic", a silent failure. The Apps view now has a version
  gate (I16).
- **G2**: a new macOS can change nettop's text. Without a check the view goes
  empty with no error. The daemon now reports "nettop output not understood".

## Why read the manifest

- **Seven unresolved effects.** U1 and U2 (host names and blocking) need a
  network filter and a new ADR. U3 to U7 are answered by M0 before any code:
  whether the chat links accept a question, nettop on macOS 14 and 15, the
  database size, the VirusTotal terms, and the CPU cost.
- **Four things cannot be undone**: a file uploaded to VirusTotal, a question
  sent to a chat, a hash looked up, and a program stopped with Quit process.
- **The history is sensitive data on disk**: every address every app talked
  to. Folder `0700`, files `0600`, never served or logged.

## Why read the test plan

The `LocalRouter` app target has no tests, so every rule is placed in
`libs/core` or `LocalRouterKit` where it can be tested; the views are checked
by hand. No CI test reads a real `.mmdb` file (no writer tool in the repo),
so M3 covers it.

## Where the build differs from the canvas

The canvas shows domain names for all traffic; nettop gives addresses. The
build shows the network owner and the place, and "Top owners" instead of "Top
domains". The Settings page says "nettop" instead of "network filter". Full
list: [05](05-ui.md), 5.1.

## Notable artifacts

- Config: `traffic`, `traffic_history_days`, `traffic_places`.
- Socket API 1.9: `get_app_traffic`, `update_geo_db`, `status.traffic`.
- `<data>/traffic/YYYY-MM-DD.ndjson`, `<data>/geo/dbip-*-lite.mmdb`.
- App: `app-checks.json` in Caches, one Keychain item, a trusted list.
- `libs/core/src/traffic/`, `apps/daemon/src/traffic.rs`,
  `LocalRouterKit/{AppIdentity,VirusTotal,AppChecks,AskAI,MapProjection}.swift`.
- No migration. No new MCP tool, no new CLI command.

## Through-line

LocalRouter knew only the traffic that passed through it. This ADR lets it
see the rest from outside, with a tool macOS already has, and keeps what it
cannot see honest: addresses instead of guessed names, a hash instead of the
file, and a link to a chat instead of a chat of its own.
