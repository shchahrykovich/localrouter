# ADR 04. Build instances: a dev build next to the release

**Status:** Proposed on 2026-09-27. Nothing is built. Keeps ADR 01's rules: only
`.localhost` names ([01-domain-tld.md](../01-project-setup-2026-09-26/01-domain-tld.md)),
no root rights ([README](../01-project-setup-2026-09-26/README.md)).

## Summary

**In one sentence.** A suffix fixed at build time (`""` for the release, `-dev`
for a development build) gives each LocalRouter instance its own app, daemon,
CLI, data folder, CA, links and ports, so a dev build runs next to the release
on one Mac.

**In three sentences.**

Today a local build and the release share every name, from the bundle id to the
data folder and socket, so a second daemon exits with "already running" and
`install.sh` replaces the release.

`build-app.sh --suffix -dev` bakes the suffix into the bundle (program file
names and `Info.plist`), and every name is derived from it in Rust, Swift and
shell, while the ports stay in the instance's own `config.json`: 80 and 443 for
the release, 7080 and 7443 for `-dev`.

Every text a person or agent reads becomes a template that names its own
instance, so an agent that follows the dev note uses `localrouter-dev` and a
`:7443` URL.

**In seven sentences.**

The maintainer wants to test a local build of LocalRouter while the release
keeps serving the daily work, and both must answer `*.localhost` names; today
this fails because both builds share the data folder, lock, socket, bundle id,
LaunchAgent label and links (`paths.rs:20-31`, `lock.rs:18-31`,
`release-lib.sh:7-8`).

A browser connects to an IP address and a port, not to a name, so the two
instances are told apart by port: the release keeps 80 and 443, and a suffixed
instance defaults to 7080 and 7443, which are not common dev server ports and
not on the browsers' blocked list.

The suffix cannot live in `config.json`, because `config.json` sits in the
folder the suffix chooses; `build-app.sh` fixes it in the bundle, the Rust
programs read it from their own file names (`localrouterd-dev`), the app from
`Info.plist`, and `LOCALROUTER_HOME` still wins for tests.

With an empty suffix every name is exactly today's name, so a release user sees
no change, and a shared table (`api/instance-names.json`) keeps the Rust and
Swift copies of the names equal.

Help texts in 13 files become templates filled from the instance; the Claude
Code note names only the CLI and points to a new read-only `guide` command, and
a suffixed note tells agents to use it only when the user asks for the dev
build.

Ports are edited only in `config.json`: Settings shows a Daemon section with
the ports when they are not 80 and 443, `set_config` stops undoing hand edits,
and no fallback path ever gives a suffixed daemon ports 80 and 443.

`install.sh` builds `-dev` by default, `publish.sh` refuses a suffix, the updater
is off in a suffixed instance, and uninstall removes only its own instance;
tests extend the existing Rust and Swift suites with renamed binaries and add a
two-instance end-to-end journey, with no new dependency.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 0 | Working backwards: what users might say | simulation | [00-working-backwards.md](00-working-backwards.md) |
| 1 | One suffix names the whole instance | feature | [01-instance-suffix.md](01-instance-suffix.md) |
| 2 | Ports: only in `config.json`, shown in Settings when not 80 and 443 | feature, bug | [02-ports-and-settings.md](02-ports-and-settings.md) |
| 3 | Every help text names its own instance | feature | [03-texts.md](03-texts.md) |
| 4 | Build, install, update and uninstall per instance | feature | [04-build-install-update.md](04-build-install-update.md) |
| 5 | Components: where the change lives | proposed | [05-components.md](05-components.md) |
| 6 | Data flow | data flow | [06-data-flow.md](06-data-flow.md) |
| 7 | Semantic Change Manifest | manifest | [07-semantic-change-manifest.md](07-semantic-change-manifest.md) |
| 8 | Test plan | test plan | [08-test-plan.md](08-test-plan.md) |
| 9 | Tasks | build plan | [09-tasks.md](09-tasks.md) |

## The through-line

Everything follows from one fact: **a browser picks a program by port, and
LocalRouter picks its files by name.** The port separates the two instances for
the browser; the suffix separates them for everything else. Because the suffix
chooses the folder, it must come from the bundle and not from a file in the
folder. Because agents act on the texts they read, the suffix must reach every
text, or a dev session quietly works on the release.

## Why read the working-backwards file

[00-working-backwards.md](00-working-backwards.md) starts from the maintainer's
real messages and simulates reactions from five roles. It found six gaps (G1 to
G6), all fixed in the change files. The two that changed the design most: with
both notes imported, an agent cannot tell which CLI to use, so a suffixed note
now says when to use it (G1); and a port edited by hand is silently undone by
the next Settings switch, so `set_config` now reads the file first (G3). The
quotes not marked (real) are simulated.

## Why read the manifest

[07-semantic-change-manifest.md](07-semantic-change-manifest.md) holds three
things no change file argues:

1. **A requirement it produced.** Today's code gives ports 80 and 443 when
   `config.json` does not parse or lacks a field (`store.rs:53-63`,
   `config.rs:8`). Invariant I9 closes every such path for a suffixed instance.
2. **A hidden dependency.** A Rust program's behaviour now depends on its own
   file name; a renamed copy of the CLI talks to the release (risk R3).
3. **What a rollback leaves behind.** The old code cannot see the dev names, so
   the dev instance must be uninstalled before a revert; the manifest lists
   what to remove by hand otherwise.

## Why read the test plan

[08-test-plan.md](08-test-plan.md) shows how to test a suffixed program without
building a bundle: copy the built binary under the suffixed name. It also names
a gap the repository's tools cannot close: nothing tests the shell scripts, so
I6 (`install.sh` keeps the release) and I7 (`publish.sh` refuses a suffix) rest
on manual tests M1 and M6.

## Notable artifacts this ADR plans

- `Instance` in Rust and Swift; `api/instance-names.json`.
- `build-app.sh --suffix`, `install.sh --suffix` (default `-dev`),
  `Info.plist` key `LRInstanceSuffix`.
- CLI command `<cli> guide`; instance and folder lines in `<cli> status`.
- Program names `localrouterd<S>` and `localrouter<S>` in the bundle.
- No new socket method, MCP tool, network endpoint or dependency.

## Not in this ADR

- A custom root domain or entries in `/etc/hosts`. The first request asked for
  it; a root domain does not separate two daemons that both need port 443, and
  `/etc/hosts` needs root for every route. `.localhost` stays the only root.
- Port fields in Settings (the maintainer chose `config.json` only).
- Re-binding ports without a restart, and a restart button (unresolved, not now).
- Separate cookies per instance (browsers do not separate cookies by port).
- Running two suffixed instances at once with default ports; a third instance
  needs its own ports in its `config.json`.

## Diagrams

All diagrams are D2 sources in [diagrams/](diagrams/) with rendered `.svg`
files. `diagrams/_palette.d2` is the shared palette from ADR 01; it has no
render of its own. Render one with:
`d2 --theme 0 --pad 20 diagrams/02-find-instance.d2 diagrams/02-find-instance.svg`.
`01-two-instances.d2` is rendered with `--layout elk`; its first comment line
says why.
