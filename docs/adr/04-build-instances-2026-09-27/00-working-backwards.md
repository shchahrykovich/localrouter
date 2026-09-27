# 0. Working backwards: what users might say

**Status:** Mixed. Quotes marked (real) are the maintainer's own words from the
design conversation of 2026-09-27, copied as written. All other quotes are
simulated: no user has seen this feature. They were written to test the design
before it is built.

## Real feedback

These messages started this ADR and set its limits:

- (real) "in settings add ability to choose .localhost - root domain. bby
  default .localhost, but user can choose custom. in this case we should check
  /etc/hosts and add it there if missing. we need root permissions for this.
  the idea is to test dev/local and prod builds at the same time"
- (real) "add settings file which will be stored in the app's folder. allow to
  specify port for daemin. the idea is to run dev and prod at the same time. so
  dev and prod both listen for .localhost. but prod 80/443 and dev custom port
  via config. [...] if port is custom show this section. does it make sense?"
- (real) "in this case we need to have localrouter-dev. can we in config add
  postfix -dev in dev, and for prod nothing?"
- (real) "it means postfix will be used in all help materials"
- (real) On ports in Settings: "only in config". On the Claude Code button in
  the dev build: "this is ok, do not remove button".

The first message asked for a custom root domain and `/etc/hosts`. The
conversation showed that a root domain does not separate two daemons (both
need port 443) and that `/etc/hosts` has no wildcards and needs root for every
route. The goal, "dev and prod at the same time", is what this ADR builds; the
root domain is not in it.

## The announcement

> **Run a development build of LocalRouter next to the release.**
>
> `scripts/install.sh` now installs **LocalRouter-dev.app** next to your
> LocalRouter.app instead of replacing it. The dev build has its own daemon,
> routes, certificate and command:
>
> ```
> localrouter-dev add shop 5173
> open https://shop.localhost:7443
> ```
>
> The release keeps `https://shop.localhost` on ports 80 and 443. The dev build
> uses ports 7080 and 7443; change them in its `config.json`. Settings shows
> the ports whenever they are not 80 and 443.
>
> Coding agents get `localrouter-dev mcp` and their own note,
> `LocalRouter-dev.md`, which tells them to use the dev build only when you
> ask for it. Every help text in the dev build names `localrouter-dev` and its
> ports.
>
> The one thing to remember: browsers share cookies between ports, so a login
> on `shop.localhost` also shows up on `shop.localhost:7443`.

## Roles

| Role | Setup | Wants |
|---|---|---|
| Maintainer (main user) | develops LocalRouter; release installed from GitHub; uses it for daily work | test a local build without losing the working release |
| Coding agent (automated client) | Claude Code with one or both notes and MCP servers | to register routes on the right instance without asking |
| Release-only user | installs releases, never builds | nothing changes |
| Contributor on a team | has only the dev build, or only the release | project docs that work for both |
| Operator (release process) | runs `publish.sh` | no suffixed build ever ships |

## Reactions

### Problems in the user's own setup

**R1: "My API server on 8080 does not start any more since I installed the dev build."**
A dev daemon on 8080 would take a very common dev server port.
**covered**: the defaults are 7080 and 7443. [02](02-ports-and-settings.md).

**R2: "I have LOCALROUTER_HOME in my shell from testing, and localrouter-dev shows no routes."**
`LOCALROUTER_HOME` wins over the suffix, so the CLI reads a test folder.
**gap, fixed (G4)**: `status` prints the instance and the data folder.
[01](01-instance-suffix.md).

**R3: "I am logged in to shop.localhost:7443 as a test user, and now my other tab is logged in as that user too."**
Browsers do not separate cookies by port.
**accepted**: the help page says so. [manifest, blast radius](07-semantic-change-manifest.md#11-blast-radius).

**R4: "db.shop.localhost:15432 works in the release, but the dev build says the listen port is taken."**
Two instances cannot share a TCP listen port.
**accepted**: the error is shown; use another listen port in the dev build.
[manifest, blast radius](07-semantic-change-manifest.md#11-blast-radius).

**R5: "I copied localrouter-dev to /usr/local/bin/lr and now it shows my release routes."**
A renamed copy has an empty suffix.
**accepted**: risk R3; the install button makes a link, and links are
resolved. [manifest, risks](07-semantic-change-manifest.md#13-risks).

### Problems in the feature's own behaviour

**R6 (maintainer): "I asked Claude to register the dev server on the dev build. It said done, but the dev app shows nothing."**
Both notes are imported, they read almost the same, and the agent used the
first one: `localrouter`.
**gap, fixed (G1)**: the suffixed note and MCP instructions say when to use
them. [03](03-texts.md); checked by M3.

**R7 (maintainer): "I changed the port in config.json. Nothing happened."**
Ports are bound once at start.
**gap, fixed (G2)**: Settings shows "Restart the daemon to use ports …" and
the restart command to copy. [02](02-ports-and-settings.md). Automatic reload is
unresolved, not now. [manifest, unresolved](07-semantic-change-manifest.md#12-unresolved-effects).

**R8 (maintainer): "I changed the port, then switched Allow LAN access, and after the restart the old port was back."**
Today `set_config` saves the config it read at start.
**gap, fixed (G3)**: `set_config` reads the file first (I8).
[02](02-ports-and-settings.md).

**R9 (coding agent): "`localrouter guide`: command not found."**
The note now points to the CLI, and the user never pressed "Install Command
Line Tool…". Today's `curl` line would have worked.
**gap, fixed (G5)**: the note keeps a `curl` fallback with the instance's
default help URL. [03](03-texts.md).

**R10 (maintainer): "I ran install.sh and my release app is gone."**
That is today's behaviour.
**covered**: `install.sh` builds `-dev` by default (I6).
[04](04-build-install-update.md).

**R11 (maintainer): "I signed the dev build with my Developer ID to test the updater, and it turned itself into the release."**
**covered**: the updater is off with a suffix (I5).
[04](04-build-install-update.md).

**R12 (maintainer): "Two orange-ish icons in the menu bar. Which one is the dev build?"**
**covered**: the release icon is not orange; tooltip and header name the
instance. [02](02-ports-and-settings.md).

**R13 (maintainer): "I uninstalled the dev build and my release lost its certificate."**
**covered**: uninstall is per instance; each CA has its own name (I10).
[04](04-build-install-update.md).

**R14 (maintainer): "The dev build's config.json had a typo, and the dev daemon grabbed port 443 when I had quit the release."**
Found while writing the manifest: a file that does not parse falls back to 80
and 443. **gap, fixed (G6)**: instance defaults in every fallback (I9).
[02](02-ports-and-settings.md).

### The users who do not want the feature

**R15 (release-only user): "Did anything change for me?"**
**covered**: with no suffix every name, folder and port is today's (I1). The
note now says `localrouter guide` with a `curl` fallback.
[01](01-instance-suffix.md), [03](03-texts.md).

**R16 (contributor): "Our README says `localrouter add api.shop 8000`. I only have the dev build."**
Project files name whatever the team writes. **accepted**: LocalRouter's texts
already tell agents not to make a project need LocalRouter; the contributor
runs `localrouter-dev` instead.

**R17 (operator): "publish.sh built LocalRouter-dev by mistake."**
**covered**: `publish.sh` refuses a suffix (I7). [04](04-build-install-update.md).

**R18 (maintainer, positive): "I work on the release all day and test my branch on :7443. Nothing gets in each other's way."**
**covered**: the goal. [01](01-instance-suffix.md).

**R19 (maintainer, positive): "The agent read the dev note and gave me the :7443 URL without asking."**
**covered**: [03](03-texts.md); M3.

## A simulated session: a coding agent with both notes

The maintainer has both notes imported and both MCP servers added.

1. The user says: "Run the dev server of this branch and register it on the
   dev build of LocalRouter."
2. The agent has two notes in its context: `LocalRouter.md` first (the older
   line in `CLAUDE.md`), then `LocalRouter-dev.md`.
3. **With today's texts, the agent goes wrong here.** Both notes say "use
   `localrouter`" with the same steps. The agent runs `localrouter add
   feat-x.shop 5174`. The route lands in the release. (Gap G1.)
4. With the fixed texts, `LocalRouter-dev.md` starts with "Use
   `localrouter-dev` only when the user asks for the dev build". The user said
   "dev build", so the agent picks it.
5. The agent runs `localrouter-dev guide` and reads the ports 7080 and 7443.
6. It starts the server and calls the `register_route` tool of the
   `localrouter-dev` MCP server with `owner_pid`.
7. It reports `https://feat-x.shop.localhost:7443`.
8. It checks with `localrouter-dev list`; the release's `localrouter list`
   does not show the route.

This is the script for manual test M3.

## Gaps and fixes

| # | Gap | Fix | Where |
|---|---|---|---|
| G1 | The agent cannot tell the two notes apart | "when to use it" sentence in the suffixed note and MCP instructions | [03](03-texts.md), T5, T8, M3, task 3, 4 |
| G2 | A hand edit of the ports seems to do nothing | Settings shows the restart line with the command | [02](02-ports-and-settings.md), T12, M4, task 6 |
| G3 | `set_config` undoes a hand edit | read the file first (I8) | [02](02-ports-and-settings.md), manifest I8, T4, task 2 |
| G4 | `LOCALROUTER_HOME` hides which folder is used | `status` prints instance and folder | [01](01-instance-suffix.md), T7, task 3 |
| G5 | `guide` fails without the CLI installed | `curl` fallback line in the note | [03](03-texts.md), T5, task 3 |
| G6 | A broken or partial `config.json` gives 80 and 443 | instance defaults in every fallback (I9) | [02](02-ports-and-settings.md), manifest I9, T3, task 2 |

All six are fixed in the files listed. No gap is left only in this file.
