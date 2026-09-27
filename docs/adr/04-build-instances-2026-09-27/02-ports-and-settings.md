# 2. Ports: only in config.json, shown in Settings when not 80 and 443

## Context

Both instances answer every `*.localhost` name. A browser connects to an IP
address and a port, not to a name, so the port decides which instance gets a
request:

| URL | Connects to | Answered by |
|---|---|---|
| `https://shop.localhost` | 127.0.0.1:443 | release daemon |
| `https://shop.localhost:7443` | 127.0.0.1:7443 | dev daemon |

The ports already live in `config.json` (`http_port`, `https_port`,
`libs/core/src/config.rs:9-21`). The daemon binds them once at start
(`apps/daemon/src/daemon.rs:179-196`). The URLs a route shows already carry a
port that is not 80 or 443 (`daemon.rs:402-406`). So the proxy needs no change.
What is missing is a way to give a new instance other ports, and a way to see
them.

## Decision

### Default ports by suffix

When the daemon starts and `config.json` does not exist, it writes one:

| Suffix | `http_port` | `https_port` |
|---|---|---|
| empty | 80 | 443 |
| any other | 7080 | 7443 |

An existing `config.json` is never rewritten at start. The ports are then the
user's to change.

The same instance defaults apply in two more cases where today's code uses
`Config::default()`, which is 80 and 443:

| Case | Today | After |
|---|---|---|
| `config.json` does not parse | moved aside, `Config::default()` used (`apps/daemon/src/store.rs:53-63`) | moved aside, instance defaults used |
| A field is missing, for example the user wrote only `{"https_port": 7444}` | `#[serde(default)]` fills `http_port` with 80 (`libs/core/src/config.rs:8`) | the missing field gets the instance default, 7080 |

So a suffixed daemon never reaches for the release's ports by accident (I9).
`Config` gets a constructor `Config::defaults_for(&Instance)`, and the
`Default` implementation is no longer used by the daemon.

**Why 7080 and 7443, not 8080 and 8443.** 8080 is a very common port for dev
servers. A dev daemon on 8080 would stop the user's own servers from starting.
Browsers refuse to connect to some ports (the "bad port" list of the Fetch
standard, read on 2026-09-27); 7080 and 7443 are not on it, and neither are 8080
and 8443.

A third instance (for example `-test`) also gets 7080 and 7443 by default, so it
fails to bind while `-dev` runs. The daemon already reports a bind failure in
its status (`daemon.rs:183-196`), and the user sets other ports in that
instance's `config.json`. This is accepted: one dev instance is the case this
ADR is for.

### Ports are edited only in config.json

The maintainer decided: no port fields in Settings. The ports change by editing
`<data folder>/config.json` and restarting the daemon.

To make that safe, `set_config` changes: today it starts from the config it
holds in memory (`daemon.rs:567`) and saves the whole config (`daemon.rs:590`).
A port edited by hand after the daemon started is put back without a message
the next time the user switches "Subdomain fallback" or "Allow LAN access".

Two scenarios with today's code:

**Scenario A: works.**
1. The dev daemon starts with ports 7080 and 7443.
2. The user edits `config.json` to 7081 and 7444.
3. The user restarts the daemon. It binds 7081 and 7444.

**Scenario B: broken.**
1. The dev daemon starts with ports 7080 and 7443.
2. The user edits `config.json` to 7081 and 7444.
3. Before a restart, the user switches "Allow LAN access" in Settings.
4. `set_config` saves the in-memory config: ports 7080 and 7443.
5. The user restarts the daemon. It binds 7080 and 7443 again. The edit is lost
   and nothing said so.

After this ADR, `set_config` reads `config.json` from disk, changes only the
fields it was given, and writes the result (I8). If the ports in the file differ
from the bound ports, the reply says `restart_needed: true`.

![Flow C: a setting changes after a hand edit](diagrams/08-flow-set-config.svg)

### The Daemon section in Settings

The Settings tab shows a **Daemon** section with Version, HTTP and HTTPS only
when a bound port is not 80 or 443. This is what the maintainer asked for. With
80 and 443 there is nothing to tell the user, and the section stays hidden, as
it is today.

The section also shows, when it is shown:

1. The path of `config.json` to edit, with a "Show in Finder" button.
2. When the file's ports differ from the bound ports: "Restart the daemon to use
   ports 7081 and 7444", and a copy line with the restart command for this
   instance, for example
   `launchctl kickstart -k gui/$(id -u)/dev.localrouter.app-dev.daemon`.

The problems part of the section (listen errors, a bad `routes.json`, the Login
Items note) stays as it is today: shown whenever there is a problem.

The rule "show the section" is a pure function in `LocalRouterKit`, so it is
unit tested (I12).

### Other visible names

- The popover header says `LocalRouter-dev` in a suffixed instance.
- The menu bar tooltip says `LocalRouter-dev`. The icon stays orange for any
  suffixed instance and for any ad-hoc build (`StatusItemController.swift:34-52`).

## Tests

T3 (default ports, no rewrite), T4 (`set_config` keeps hand edits), T12 (the
Daemon section rule), M4 (edit ports, restart, check). See the
[test plan](08-test-plan.md).
