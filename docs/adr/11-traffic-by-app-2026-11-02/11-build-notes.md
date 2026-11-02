# 11. Build notes

**Status:** Built 2026-11-02 on branch `adr-11-traffic-by-app`. The automated
tests pass. The manual tests M0 to M8 are not done yet, except the parts of
M0 below.

## What M0 answered

| Unknown | Answer |
|---|---|
| U4, nettop output | Measured on macOS 27.0.1: the same format as in [01](01-app-traffic.md). Two more facts: a process name can have a dot (`python3.10.19798`: the pid is after the last dot), and an unconnected UDP socket can have counters (`udp4 *:5353<->*:*,82149271,17676530,`). macOS 14 and 15 are not checked yet. |
| U5, DB-IP size | 2026-11: city 60.1 MB and ASN 5.2 MB as `.gz` (`content-length` of the download). The city file is about 130 MB unpacked. Download time is not measured. |
| U3, chat links | Not checked. Both buttons open the link; `AskAI.Service.copyFallback` switches a service to the copy fallback. |
| U6, VirusTotal terms | Not read yet. Blocks the release of the VirusTotal part, not the build. |
| U7, CPU of nettop | Not measured. |

## A fact found in the build: nettop's output is buffered

nettop writes to a pipe with full buffering. A sample does not arrive in one
piece: its last lines often come with the next write, 5 seconds later. The
first sampler ended a sample after a short pause and lost those lines, so
processes with high pids were missing (the end-to-end test E1 found it).

The fix: a sample ends only at the next header line (`Splitter` in
`traffic/nettop.rs`). The cost: a sample reaches the history about 5 seconds
later. A pause is used only to report text that never had a header (a
changed format, gap G2).

## Where the build differs from the text

| # | Text | Build | Why |
|---|---|---|---|
| D1 | Two new methods | Three: `get_app_traffic`, `update_geo_db`, and `clear_traffic_history` | **Clear history** in Settings needs a method. |
| D2 | The contract tests "need no edit" | Each test maps a method to its types, so both got one line per method | They fail on an unknown method by design. |
| D3 | `/usr/bin/` is in "macOS services" | `/usr/bin/` programs (`curl`, `git`, `ssh`) are their own rows. Apps in `/System/Applications/` stay apps. | M1 expects a `curl` row; a user wants to see these. |
| D4 | The peer table holds bytes | It holds the client name, host and upstream address only | The bytes come from nettop. |
| D5 | The proxy knows each request's upstream address | A tunnel gives it at once. A request on a pooled connection uses `Upstream::last_address`, the address the last connection to that host and port went to. | hyper's pool hides which connection a request used. |
| D6 | Memory buckets of 5 s | One entry per minute per pair; the open minute adds each 5-second sample. The hour view has 60 buckets of 1 minute. | Same data at the grain the reply uses. |
| D7 | The Apps view asks every 2 s | Every 3 s, with the app's status loop | One loop instead of two. |
| D8 | Day files `YYYY-MM-DD` | The UTC date | No local-time zone in the daemon. |
| D9 | — | An unconnected UDP socket with bytes counts as `direct` with no address | It has bytes and no peer (U4). |
| D10 | `status.traffic`, `traffic_error` | Also `traffic_history_bytes` and `geo_db` (`{date, bytes}`); `update_geo_db` returns the same `{date, bytes}` | Settings shows the size on disk and the database month. |
| D11 | Header: "who started it, first seen" | The pids only | There is no data source for either yet. |
| D12 | Map ranges 1 hour, 24 hours, 7 days | Also 30 days | The history keeps 30 days. |
| D13 | `land-dots.json` as a resource | Committed at `apps/menubar/Sources/LocalRouter/TrafficMap/`, excluded from SwiftPM, copied by `scripts/build-app.sh` to `Contents/Resources/`, read with `Bundle.main`. Generator in `scripts/map/`. | A SwiftPM resource bundle at the `.app` root breaks code signing. |
| D14 | The question shortens the hash | The whole hash, shortened only when the URL would pass 2,000 characters | Rule 3 of [04](04-ask-ai.md). |
| D15 | — | The VirusTotal limiter's counts are saved in `app-checks.json` | "Lookups today" stays right after a restart. |

## Tests that ran

| Test | Result |
|---|---|
| T1 to T5 (`cargo test -p localrouter-core --lib traffic`, `--test forward peer_port`) | pass |
| T6 to T8, E1 (`cargo test -p localrouterd --test api`) | pass; E1 ran the real nettop on macOS 27.0.1 |
| contract tests, Rust and Swift | pass |
| T9 to T15 (`swift test --package-path apps/menubar`) | pass, 201 Swift tests |
| `cargo test --workspace`, `cargo clippy --workspace --all-targets` | pass, no warnings |
| `scripts/build-app.sh --suffix -dev` | builds; `codesign --verify --deep --strict` passes; `land-dots.json` is in the bundle |
| M0 to M8 | not run, except U4 and U5 above |
