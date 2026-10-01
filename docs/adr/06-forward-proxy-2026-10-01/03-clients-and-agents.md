# 3. Clients and agents: how a person or an agent gets and uses the proxy

**Status:** Proposed.

## Context

A proxy is useful only when a client is told to use it, and inspection works
only when that client trusts the inspection CA. Each client is told in its own
way. A coding agent must be able to ask LocalRouter for these settings, in a
form it can use without guessing.

## Decision

### Config and socket API (1.2 → 1.3, additive)

New `config.json` fields, all optional, with instance defaults:

| Field | Default | Meaning |
|---|---|---|
| `proxy_enabled` | `false` | bind the proxy port |
| `proxy_port` | `8877` (release), `7877` (suffixed) | the proxy port |
| `inspect_hosts` | `[]` | host patterns to inspect ([02](02-inspection-ca.md)) |

`set_config` takes the same three fields. `inspect_hosts` is replaced as a
whole list; the CLI reads, changes and writes it. Two clients that change the
list at the same moment can lose one change (accepted: settings are changed
by hand, rarely).

New methods, so `METHODS` grows from 11 to 13:

| Method | Who | What |
|---|---|---|
| `get_proxy` | everyone, and the MCP tool | everything a client needs to use the proxy (below) |
| `reset_inspect_ca` | user action only (CLI, app) | delete and make a new inspection CA |

`status` gets an optional `proxy` field: enabled, port, bound addresses,
errors, and the inspection CA state. Old clients ignore it.

### `get_proxy` reply

```json
{
  "enabled": true,
  "url": "http://127.0.0.1:8877",
  "port": 8877,
  "bound": ["127.0.0.1:8877", "[::1]:8877"],
  "errors": [],
  "inspect_hosts": ["api.example.com"],
  "inspect_set": ["api.example.com", "*.anthropic.com"],
  "inspect_ca": {
    "state": "ok",
    "pem_path": "/Users/me/Library/Application Support/LocalRouter/inspect-ca/ca.pem",
    "common_name": "LocalRouter Inspection 1a2b3c4d",
    "trusted": false
  },
  "env": {
    "HTTPS_PROXY": "http://127.0.0.1:8877",
    "HTTP_PROXY": "http://127.0.0.1:8877",
    "NO_PROXY": "localhost,127.0.0.1,::1,.localhost",
    "NODE_USE_ENV_PROXY": "1",
    "NODE_EXTRA_CA_CERTS": "/Users/me/Library/Application Support/LocalRouter/inspect-ca/ca.pem"
  },
  "chrome_args": [
    "--user-data-dir=/Users/me/Library/Caches/LocalRouter/chrome-proxy",
    "--proxy-server=http://127.0.0.1:8877",
    "--no-first-run",
    "--no-default-browser-check"
  ],
  "notes": ["The inspection CA is not trusted in the login keychain: Chrome will refuse inspected hosts. Run: localrouter proxy trust"]
}
```

- `inspect_set` is `inspect_hosts` plus the hosts of enabled script rules
  (ADR 07), so an agent sees exactly which hosts are read.
- `NO_PROXY` leaves `.localhost` out of the proxy: dev servers are reached
  directly, as today. ADR 07 scripts still run on them at the router.
- `NODE_USE_ENV_PROXY=1` is there because the built-in `fetch` of recent
  Node.js versions reads `HTTPS_PROXY` only when this variable is set. Older
  Node.js versions ignore it. Which programs honour it is checked in M5.
- `inspect_ca` is `null` before the CA exists, and `NODE_EXTRA_CA_CERTS` is
  then left out.
- `notes` holds plain sentences for each thing that will not work yet:
  proxy off, port not bound, CA not trusted.
- `chrome_args` use a separate Chrome profile in the user's Caches folder,
  so the user's main Chrome is not changed. It is outside the data folder:
  Chrome writes it, not the daemon (ADR 01, invariant I1).

### CLI

| Command | What |
|---|---|
| `localrouter proxy` | the `get_proxy` reply as text |
| `localrouter proxy on`, `proxy off`, `proxy port 8877` | `set_config`. `proxy off` also prints: "Programs started with HTTPS_PROXY=http://127.0.0.1:8877 now fail to connect. Restart them without it." |
| `localrouter proxy env` | `export …` lines, for `eval "$(localrouter proxy env)" && claude` |
| `localrouter proxy chrome` | opens Chrome with `chrome_args` (`open -na "Google Chrome" --args …`) |
| `localrouter proxy inspect add <pattern>`, `rm`, `list` | change `inspect_hosts` |
| `localrouter proxy trust`, `untrust`, `ca-path`, `ca reset` | the inspection CA, like the existing `trust` commands |

### MCP: a seventh tool, `get_proxy`

Read only. Its description: "How to send traffic through the LocalRouter
proxy: proxy URL, environment variables for Claude Code and Node.js, Chrome
flags, which hosts are inspected, and whether the inspection CA is trusted."
The MCP server then lists exactly seven tools; `apps/cli/tests/mcp.rs` changes
on purpose (ADR 01, invariant I13).

Turning the proxy on and adding inspect hosts are **not** MCP tools, the same
as `set_config` today. An agent runs the CLI, which the user sees in the
transcript.

**An agent cannot move its own traffic.** A process reads `HTTPS_PROXY` when
it starts. Claude Code that is already running keeps its connections. The
agent texts say this plainly: the agent can start *other* programs with the
`env` values (a test run, a Chrome it opens), or tell the user to restart
Claude Code with `eval "$(localrouter proxy env)" && claude`.

### Request log

Proxy requests are written as the existing `http` entry, so older clients
still decode `get_logs` (a new `kind` would make them fail). New optional
fields:

| Field | Values |
|---|---|
| `via` | `"proxy"`; absent for router traffic |
| `mode` | `"http"`, `"inspect"`, `"tunnel"` |
| `bytes_in`, `bytes_out` | tunnels only |

A tunnel entry has method `CONNECT`, path `""`, status `200` (or `502` when
the server did not answer). Query strings, headers and bodies are never
stored (ADR 01, invariant I10).

### Menu bar app

A **Proxy** section in Settings: on and off, port, the inspect list, and
Trust inspection or Untrust. The Logs view shows `via proxy` entries with a
small label. `Api.swift` gets the new types.

### Right-click menu: Open Chrome via Proxy

The status item's right-click menu gets one item, **Open Chrome via Proxy**.

1. **Shown only when Google Chrome is installed.** The app asks
   `NSWorkspace.urlForApplication(withBundleIdentifier: "com.google.Chrome")`
   each time the menu opens. No Chrome: no item. Chrome Canary, Chromium,
   Brave and Edge are not detected in this ADR.
2. **Proxy off: the item turns it on first.** It calls `set_config
   proxy_enabled true`. If that fails (port taken), the popover opens with the
   error and Chrome is not started.
3. **It starts a separate Chrome instance** with the arguments from
   `get_proxy.chrome_args`, the same ones `localrouter proxy chrome` uses:

   ```
   open -na "Google Chrome" --args --user-data-dir=<Caches>/LocalRouter<suffix>/chrome-proxy --proxy-server=http://127.0.0.1:8877 --no-first-run --no-default-browser-check
   ```

   The app runs Chrome with `NSWorkspace.openApplication(at:configuration:)`,
   `createsNewApplicationInstance = true` and these `arguments`, not through a
   shell.
4. **Why a separate profile is required.** Chrome reads `--proxy-server` only
   when a new browser process starts. If the user's normal Chrome is running,
   a second launch with the same profile only opens a window in the running
   process, and the flag is ignored. A different `--user-data-dir` forces a
   new process. So only this one Chrome instance uses the proxy; the user's
   normal Chrome, its tabs and its settings do not change.
5. **Clicking again** while the proxy Chrome runs opens a new window in that
   same proxy instance (same profile), not a third Chrome.
6. **Names stay local.** Chrome by itself sends `localhost`, `*.localhost`,
   `127.0.0.1` and `[::1]` directly, not to a proxy. So dev names in that
   window work as before and do not appear as proxy traffic.
7. The popover opens after the click and shows the result: "Chrome started
   with the proxy at 127.0.0.1:8877", plus the `notes` of `get_proxy` (for
   example "inspection CA not trusted").

The profile folder is outside the data folder, so ADR 01 invariant I1 holds:
Chrome writes it, not the daemon. A suffixed instance uses its own folder and
port, so the `-dev` and release proxy Chromes can run at the same time.

### Agent texts

`help.md`, `note.md` and `mcp.md` get a short "Proxy" part: what it is, the
`get_proxy` tool and `proxy env`, that an agent cannot move its own traffic,
and that inspected hosts need trust, which only the user can give (macOS asks
for the password). One rule is stated as a rule: **never write proxy settings
into files of the project** (`.env`, `.claude/settings.json`, test configs).
Other people on the project may not have LocalRouter, and every request of
theirs would fail. Pass the values to a command, or let the user put them in
their own shell. They name their instance through the
existing templates (ADR 04); `no_fixed_names.rs` checks this.

## Trade-offs

- **`inspect_hosts` in `config.json` is lost on downgrade.** An older daemon
  ignores the new fields, and its next `set_config` writes `config.json`
  without them. The inspection CA folder stays on disk.
- **Seven tools instead of six.** One more tool in every agent's list. A
  separate read tool is clearer than a `proxy` argument on `status`.

## Tests

T8 (config fields and defaults), T9 (socket API, examples, Swift contract),
T10 (CLI), T11 (MCP tool list and `get_proxy`), T12 (agent texts), T14
(Chrome launcher in Swift), M3 (the menu item by hand). See the
[test plan](07-test-plan.md).
