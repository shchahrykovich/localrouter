# ADR 02. Distribution and self-update

**Status:** Accepted, as built (2026-09-26). Manual tests M9 and M10 are open.

## Summary

**In one sentence.** LocalRouter ships as a notarized DMG on
`github.com/shchahrykovich/localrouter/releases`, and the installed app
updates itself from there, the same way VibeViewer does.

**In three sentences.**

The user wanted to install even the first version from GitHub and never
download by hand again.

`scripts/publish.sh` builds the Rust and Swift programs, signs them with
Developer ID, notarizes the DMG and creates a GitHub release. The app checks
the latest release every 6 hours and, after one click and four checks,
replaces itself and restarts the daemon.

**In seven sentences.**

The goal is one manual download, then automatic updates. The release pipeline
copies VibeViewer's scripts: one version in `Cargo.toml`, Developer ID signing
with the hardened runtime, `notarytool` and a stapled DMG, then
`gh release create`.

The updater is custom Swift code with no extra signing key: it trusts GitHub
over https, Gatekeeper (`spctl`) and the running app's Team ID. The install
runs in a detached script that swaps the bundle with two renames, restarts the
daemon with `launchctl kickstart`, and reopens the app, while routes and the CA
stay untouched outside the bundle.

The first real build found a bug the plan did not foresee: on a
case-insensitive disk the CLI `localrouter` overwrote the app `LocalRouter`, so
the CLI moved to `Contents/Helpers`. The trade-off is a manual, single-Mac
release process and Apple Silicon only. Swift tests cover the updater logic,
the script and the bundle layout; the first real release and update are manual
tests M9 and M10.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 1 | Release pipeline: Developer ID, notarized DMG, GitHub Releases | feature | [01-release-pipeline.md](01-release-pipeline.md) |
| 2 | Self-update from GitHub Releases | feature | [02-self-update.md](02-self-update.md) |
| 3 | Bundle layout: the CLI lives in `Contents/Helpers` | bug | [03-bundle-layout.md](03-bundle-layout.md) |
| 4 | Data flow | data flow | [04-data-flow.md](04-data-flow.md) |
| 5 | Semantic Change Manifest | manifest | [05-semantic-change-manifest.md](05-semantic-change-manifest.md) |
| 6 | Test plan | test plan | [06-test-plan.md](06-test-plan.md) |

This ADR is as built, so it has no task list.

## The through-line

Trust flows from one secret, the Developer ID key on the maintainer's Mac:
Apple notarizes what it signs, Gatekeeper checks the notarization, and the
installed app accepts only images from the same team. Everything else (GitHub,
the API answer, the download) is treated as untrusted transport.

## Why read the manifest

[05-semantic-change-manifest.md](05-semantic-change-manifest.md):

1. **Rollback is one-way.** The updater never installs an older version, so a
   bad release is fixed only by publishing a newer one.
2. **An update drops open connections once**, because the daemon restarts.
3. **Fork inheritance.** A socket created while the daemon starts `security`
   can be held by that child for about 50 ms; found by a flaky test.

## Why read the test plan

[06-test-plan.md](06-test-plan.md) says what is proven and what is not: the
updater logic is tested, but no release exists yet. The GitHub repository is
empty, and a release needs a commit to tag, so M9 and M10 wait for the first
push.

## Relation to ADR 01

This ADR decides ADR 01's open points U2 (bundle id `dev.localrouter.app`,
signing, distribution) and U4 (CLI link in `~/.local/bin`, made by the menu
command "Install Command Line Tool…"), and records two drifts from ADR 01:
a Swift package instead of an Xcode project, and the CLI in
`Contents/Helpers`.
