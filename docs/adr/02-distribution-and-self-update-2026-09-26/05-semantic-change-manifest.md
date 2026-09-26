# 5. Semantic Change Manifest

## 1. Manifest status

```text
Manifest status: IMPLEMENTED_AS_PLANNED
```

Written after the code, from the built system (this ADR records a decision the
user made during implementation). Manual tests M9 and M10 are still open.

## 2. Semantic change summary

```text
Artifacts
+ 6 scripts: build-app.sh, release.sh, notarize.sh, publish.sh, install.sh, release-lib.sh
+ 2 plist templates, 1 icon generator, 1 notary env example
+ 1 updater (Swift), 1 menu command: Check for Updates
+ 1 install script generated at run time

Persistent data
+ dist/localrouter-X.Y.Z.dmg per release (local, ignored by git)
+ one GitHub release and one asset per version
+ update-<time>.log per update on the user's Mac

Runtime effects
+ 1 external READ: GitHub releases API (every 6 hours, and on demand)
+ 1 external READ: DMG download (on user click)
+ 1 bundle REPLACE path, 1 daemon RESTART path

Modified
~ bundle layout: CLI moved to Contents/Helpers (drift from ADR 01)

External effects
+ HTTPS requests to api.github.com and GitHub download hosts
+ Apple notary service submissions (maintainer only)

Destructive operations
1 (the install script replaces LocalRouter.app)

Unresolved effects
2
```

## 3. Source of truth

```text
CANONICAL
  Cargo.toml [workspace.package] version     the version of every program
  GitHub release "latest"                    what users update to

DERIVED
  Info.plist CFBundleShortVersionString      rendered from Cargo.toml by build-app.sh
  dist/*.dmg                                  built from the source tree
```

## 4. Artifacts

```text
Scripts
+ scripts/build-app.sh, release.sh, notarize.sh, publish.sh, install.sh, release-lib.sh
+ scripts/Info.plist.in, daemon.plist.in, make-icon.swift, notarize.env.example

Swift
+ LocalRouterKit/Updater.swift     check, parse, trust rules, download, spctl, script
+ LocalRouterKit/CLIInstaller.swift BundleLayout paths + ~/.local/bin link
+ AppModel update state, footer badge, settings toggle

Bundle
+ Contents/Helpers/localrouter
+ Contents/Library/LaunchAgents/dev.localrouter.app.daemon.plist
+ Contents/Resources/AppIcon.icns

Database entities: 0
New network listeners: 0
Background jobs: 0 (the check is a timer inside the running app)
```

## 5. Runtime effects

```text
READ (external)
type:                   HTTPS GET
target:                 api.github.com/repos/shchahrykovich/localrouter/releases/latest
trigger:                30 s after start, every 6 h, or "Check for Updates…"
cardinality:            1 request per check
write_idempotent:       n/a
retention:              none

READ (external)
type:                   HTTPS download
target:                 the release's .dmg on a GitHub host
trigger:                user clicks "Update to X.Y.Z"
cardinality:            1 per install
writes:                 $TMPDIR/LocalRouter-X.Y.Z.dmg (deleted by the script)

REPLACE
type:                   app bundle swap
target:                 /Applications/LocalRouter.app (or ~/Applications)
trigger:                all four checks passed
write_idempotent:       yes (installing the same version twice gives the same bundle)
producer_deterministic: yes (the image is fixed)
destructive:            yes, the old bundle is deleted after the swap
reversible:             only by installing the old DMG again by hand

RESTART
type:                   launchctl kickstart -k gui/<uid>/dev.localrouter.app.daemon
trigger:                after the swap
effect:                 open connections through the daemon are dropped once
```

## 6. Reads and writes

```text
READ           GitHub API, the DMG, the running app's code signature
WRITE          temp DMG, temp script, the app bundle, ~/Library/Logs/LocalRouter/update-*.log
EXTERNAL WRITE (maintainer only)  GitHub release and asset, Apple notary submission
```

## 7. Interfaces and events

```text
+ UI     "Check for Updates…" menu item, "Update to X.Y.Z" button, auto-check toggle
+ UI     "Install Command Line Tool…" menu item (links ~/.local/bin/localrouter)
+ SCRIPT scripts/publish.sh and friends (maintainer interface)
Socket API, MCP tools, CLI commands: unchanged
```

## 8. External side effects

```text
+ one HTTPS request to api.github.com per check (unauthenticated; GitHub allows
  60 per hour per IP, a check every 6 hours uses 4 per day)
+ Apple notary submissions per release
Cost: none beyond the Apple Developer membership already used for VibeViewer.
```

## 9. Invariants

```text
U-I1. The app downloads only from https GitHub hosts.
      enforced by: Updater.isTrusted, checked before and after redirects; UpdaterTests.
U-I2. The app replaces itself only with an image Gatekeeper accepts.
      enforced by: spctl in Updater.checkImage; publish.sh refuses unnotarized images.
U-I3. The new app must carry the running app's Team ID.
      enforced by: the install script; UpdaterTests checks the generated line.
U-I4. The three bundled programs have paths that differ when case is ignored.
      enforced by: BundleLayoutTests; build-app.sh checks.
U-I5. An update never touches routes, settings or the CA.
      enforced by: they live outside the bundle (ADR 01, 08-components); manual M10.
U-I6. Notary credentials are never exported to child processes.
      enforced by: lr_load_signing_env sources the file without `export` / `set -a`.
```

## 10. Data impact

```text
Migration          none
Backfill           none
Destructive        the old app bundle, replaced by the new one
Growth             one update log per update (small, not rotated)
```

## 11. Blast radius

```text
Users on an ad-hoc build (scripts/install.sh)
dependency:   spctl must accept the image
impact:       they never update automatically
failure mode: the footer shows the Gatekeeper message. Visible, by design.

Open connections through the daemon during an update
dependency:   the daemon restarts after the swap
impact:       dev-server WebSockets (hot reload) reconnect once; TCP sessions drop
failure mode: a database client sees one disconnect. Visible.

The CLI link in ~/.local/bin
dependency:   points into the bundle path, which the swap keeps
impact:       none; it follows the new bundle
```

## 12. Unresolved effects

```text
? U5. Universal (Intel) builds
status:  DEFERRED
reason:  only aarch64-apple-darwin is installed; releases are Apple Silicon only
blocks:  Intel users

? U6. Update log cleanup
status:  DEFERRED
reason:  update-*.log files are small but never deleted
blocks:  nothing
```

## 13. Risks

- **A compromised GitHub account can ship an update**, but it must also be
  signed and notarized with the same Team ID. The Developer ID key is the real
  secret.
- **Unauthenticated API limits.** Many users behind one IP could hit the 60
  requests per hour limit; the check then fails quietly and retries later.
- **Fork inheritance.** On macOS a child process can inherit a socket that is
  created at the same moment. The daemon starts `/usr/bin/security` for the
  trust check, so a TCP route port can stay held for the life of that process
  (about 50 ms). Found by a flaky test during implementation; not a data risk.

## 14. Rollback

```text
Code rollback     revert the commits; installed apps keep their current version
Release rollback  gh release delete vX.Y.Z; mark the previous release "latest".
                  Apps already updated keep the bad version until a newer one is
                  published: the updater never goes back to an older version.
User rollback     download the older DMG from the releases page and install it by hand
```
