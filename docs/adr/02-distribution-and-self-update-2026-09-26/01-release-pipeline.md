# 1. Release pipeline: Developer ID, notarized DMG, GitHub Releases

**Status:** As built (2026-09-26). Not yet run end to end (see the test plan, M9).

**Context.** The app must be installable from GitHub, and even the first
version must update itself. VibeViewer already solves this on the same Mac with
the same signing identity, so LocalRouter copies its approach instead of adding
Sparkle or a Homebrew cask.

![From source to a GitHub release](diagrams/01-release-pipeline.svg)

## Decision

| Step | Script | What it does |
|---|---|---|
| Version | `scripts/release.sh` | Bumps `[workspace.package] version` in `Cargo.toml` (the single source). `--patch` by default, or `--minor`, `--major`, `--set X.Y.Z`, `--no-bump`. Restores the old version if a later step fails. |
| Build | `scripts/build-app.sh` | `cargo build --release` for the daemon and the CLI, `swift build -c release` for the app, assembles `LocalRouter.app`, renders `Info.plist` and the LaunchAgent plist, draws the icon. |
| Sign | `scripts/build-app.sh --developer-id` | Nested programs first, then the bundle, with `--options runtime --timestamp`. The identity is the keychain hash of "Developer ID Application", narrowed by `APPLE_TEAM_ID`. |
| Package | `scripts/release.sh` | `dist/localrouter-X.Y.Z.dmg` with an `Applications` link. |
| Notarize | `scripts/notarize.sh` | Signs the DMG, `xcrun notarytool submit --wait`, `xcrun stapler staple`, then `spctl --assess`. |
| Publish | `scripts/publish.sh` | Checks `gh` login and that the repository has a commit, runs `release.sh --notarize`, refuses an image that fails `stapler validate`, `gh release create vX.Y.Z --latest`, uploads, compares the downloaded size. |

Credentials come from the environment or from `.env.notarize` at the
repository root (ignored by git). They are never exported, so no child process
of the build sees them. The variable names are the same as in VibeViewer, so
the same file works for both.

Decided here (open point U2 of ADR 01): bundle id `dev.localrouter.app`,
LaunchAgent label `dev.localrouter.app.daemon`, Developer ID signing and
notarization for releases, ad-hoc signing for local builds.

Trade-off: releases are built by hand on one Mac, not in CI. The signing
certificate never leaves that Mac.

Not in this version: universal binaries (only `aarch64-apple-darwin` is
installed), a Homebrew cask.

## Tests

- Shell syntax of every script: `bash -n scripts/*.sh`.
- `scripts/build-app.sh` checks that all three programs exist and that
  `Contents/MacOS/LocalRouter` is the Swift binary (see [03](03-bundle-layout.md)).
- The full publish run is manual test M9.
