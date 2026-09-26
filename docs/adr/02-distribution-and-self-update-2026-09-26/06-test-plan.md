# 6. Test plan

As built. Swift tests run with `swift test --package-path apps/menubar`
(XCTest, comes with Xcode). Script checks run with `bash -n`.

| ID | Kind | What it proves | Real parts | Replaced parts | Runs |
|---|---|---|---|---|---|
| S1 | unit | version order, trusted hosts, release parsing, refusal of bad releases | `Updater` | GitHub API (fixed JSON) | `swift test` |
| S2 | unit | the install script quotes paths, checks the Team ID, restarts the daemon, is valid bash | generated script, `bash -n` | the script is not executed | `swift test` |
| S3 | unit | CLI link: made, idempotent, replaces an old LocalRouter link, never a foreign file | `CLIInstaller`, temp folders | `~/.local/bin` (temp folder) | `swift test` |
| S4 | unit | bundled program paths never collide when case is ignored | `BundleLayout` | nothing | `swift test` |
| S5 | build check | the bundle has three programs and the right app binary | `build-app.sh` | Developer ID (ad-hoc) | `scripts/build-app.sh` |
| M9 | manual | first release end to end: publish, download, install, start | everything | nothing | by hand |
| M10 | manual | an installed release updates to the next one | everything | nothing | by hand |

Replaced parts and who covers them for real: the GitHub API and the DMG (S1, S2)
are covered by M10; Developer ID signing (S5) by M9.

## Results on 2026-09-26

| Test | Result |
|---|---|
| S1 to S4 | 13 Swift tests pass (`swift test`) |
| S5 | pass: `Contents/MacOS/LocalRouter`, `Contents/MacOS/localrouterd`, `Contents/Helpers/localrouter` |
| Bundle smoke test | the bundled daemon and CLI ran in a temp `LOCALROUTER_HOME` with random ports: 5 routes, both ports bound, CA created, idle memory **5.0 MB** |
| M9, M10 | **not run yet**: the GitHub repository is empty, and a release needs a commit to tag |

## M9. First release from GitHub

Costs: one Apple notary submission. Creates a public release.

| Check | Expected |
|---|---|
| `scripts/publish.sh --set 0.1.0` | release `v0.1.0` on GitHub with `localrouter-0.1.0.dmg`; size check passes |
| Download the DMG on a Mac, open it | no Gatekeeper warning |
| Drag to Applications, open | icon next to the clock; macOS reports a background item; the daemon runs |
| Settings → Trust…, then open a route over HTTPS | no certificate warning |
| ⋯ → Install Command Line Tool… | `~/.local/bin/localrouter --version` prints 0.1.0 |

## M10. Update from one release to the next

| Check | Expected |
|---|---|
| With 0.1.0 installed, `scripts/publish.sh` (0.1.1) | release `v0.1.1` |
| ⋯ → Check for Updates… | "Update to 0.1.1" |
| Click it | the app quits and comes back; Settings shows 0.1.1; `localrouter status` shows daemon 0.1.1 |
| Routes and trust | unchanged after the update |
| `~/Library/Logs/LocalRouter/update-*.log` | ends with "update finished" |
| Copy the app to ~/Downloads and check there | "Move LocalRouter to the Applications folder first" |

## Invariants to tests

| Invariant | Automated | Manual |
|---|---|---|
| U-I1. Download only from GitHub over https | S1 | M10 |
| U-I2. Only Gatekeeper-accepted images | | M10 |
| U-I3. Same Team ID | S2 | M10 |
| U-I4. No case collision in the bundle | S4, S5 | |
| U-I5. Update keeps routes, settings, CA | | M10 |
| U-I6. Credentials never exported | | review of `release-lib.sh` |
