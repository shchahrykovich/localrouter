# ADR 06. Forward proxy: send Chrome or Claude Code through LocalRouter

**Status:** Accepted. Built on 2026-10-01: tasks 1 to 10 and the automated
end-to-end test E1d. Open: manual tests M1 to M6, the decision on U2, and the
release (task 12). Differences from the plan: [08-tasks.md](08-tasks.md#plan-vs-actual).

## Summary

**In one sentence.** The daemon gets a third way in, a forward proxy on
`127.0.0.1:8877` (`libs/core/src/forward.rs`), that Chrome or Claude Code can
use as their HTTP proxy, so LocalRouter sees which hosts they call and, for
hosts the user lists, each request.

**In three sentences.**

Today LocalRouter only sees requests for `.localhost` names, so the user
cannot see what a browser page or a coding agent sends to real servers.

The daemon will listen on a loopback-only proxy port, off by default; it
passes HTTPS through as a tunnel, and inspects only hosts in an inspect set,
with a second CA (the inspection CA) that the user trusts separately.

Agents read the URL, environment variables and Chrome flags with the new MCP
tool `get_proxy`, and the user can right-click the menu bar icon to open a
separate Chrome window that uses the proxy.

**In seven sentences.**

A forward proxy is the server a client sends all its requests to; Chrome and
most command-line tools support one through a flag or `HTTPS_PROXY`.

This ADR adds one to the daemon: a listener pair on `127.0.0.1` and `::1`
(port 8877, or 7877 for a suffixed instance), never open to the LAN, turned
on and off without a restart, with `.localhost` names still answered by the
route table.

`CONNECT` requests are tunnels by default, so nothing is read; a host in the
inspect set (`inspect_hosts` in `config.json`, plus the hosts of ADR 07 rules)
is inspected with leaves from a new inspection CA in `inspect-ca/`, made only
when first needed.

The daemon now opens connections to the internet for the first time, and it
checks every server certificate with the macOS trust store, so the proxy
never makes a connection less safe than it was.

Clients get everything from `get_proxy` (socket API 1.3, and a seventh MCP
tool), from `localrouter proxy …` commands, and from the right-click item
**Open Chrome via Proxy**, shown only when Chrome is installed; it starts one
Chrome instance with its own profile and the proxy flag.

The main trade-offs: a second CA to trust, no support yet for a company
upstream proxy (U1), and a port that every account on the Mac can reach (U2).

Tests cover each rule against local fake servers in CI, and real Chrome,
Claude Code and the internet in manual tests.

## Index

| # | Change | Kind | File |
|---|---|---|---|
| 0 | Working backwards: what users might say | simulation | [00-working-backwards.md](00-working-backwards.md) |
| 1 | The proxy port: a forward proxy on loopback | feature | [01-proxy-port.md](01-proxy-port.md) |
| 2 | Tunnel by default, inspect only listed hosts, with a second CA | feature, security | [02-inspection-ca.md](02-inspection-ca.md) |
| 3 | Clients and agents: config, socket API, CLI, MCP, app, Chrome menu item | feature | [03-clients-and-agents.md](03-clients-and-agents.md) |
| 4 | Components: where the proxy lives | overview | [04-components.md](04-components.md) |
| 5 | Data flow | data flow | [05-data-flow.md](05-data-flow.md) |
| 6 | Semantic Change Manifest | manifest | [06-semantic-change-manifest.md](06-semantic-change-manifest.md) |
| 7 | Test plan | test plan | [07-test-plan.md](07-test-plan.md) |
| 8 | Tasks | build plan | [08-tasks.md](08-tasks.md) |

Companion: [ADR 07](../07-proxy-scripts-lua-2026-10-01/README.md) adds Lua
scripts that read and change the traffic this proxy carries.

## The through-line

**The proxy is a way in, not a second product.** It runs in the same daemon,
reuses the route table for `.localhost` names, writes the same request log,
and follows the same rules: loopback only, the daemon owns all state, the CLI
and the app are thin clients. What is new is limited on purpose: it reads
nothing it was not asked to read, and it never trusts a server the Mac would
not trust.

## Why read the working-backwards file

[00-working-backwards.md](00-working-backwards.md) found three gaps, all now
fixed in the ADR. The one that changed the design most: an agent could write
`HTTPS_PROXY` into a project file and break every teammate who does not have
LocalRouter, so the agent texts now carry that rule (I15). It also found that
Node.js `fetch` needs `NODE_USE_ENV_PROXY=1`.

## Why read the manifest

[06-semantic-change-manifest.md](06-semantic-change-manifest.md) holds three
things no change file argues:

1. **The daemon's first outbound connections.** Until now every connection
   went to loopback. The manifest lists what that changes and what a rollback
   cannot undo (requests already sent, keychain trust, client settings).
2. **A compatibility trap it found.** A new log entry `kind` would make older
   CLIs fail to decode `get_logs`. Proxy entries reuse `kind: "http"` with
   optional fields (I16).
3. **Two open decisions.** U1, a company upstream proxy, and U2, who may use
   the port on a Mac with several accounts.

## Why read the test plan

[07-test-plan.md](07-test-plan.md) lists 14 automated test groups, one
end-to-end test and six manual tests. It names what CI must replace: the
internet, the macOS trust store, keychain trust and Chrome itself. One
question only a manual test can answer: whether Claude Code reads
`NODE_EXTRA_CA_CERTS` (M2).

## Notable artifacts

- New files: `libs/core/src/forward.rs`, `upstream.rs`, `inspect.rs`,
  `apps/daemon/src/proxy_listen.rs`, `apps/menubar/Sources/LocalRouterKit/ChromeLauncher.swift`.
- New data: `inspect-ca/ca.key` (mode `0600`) and `inspect-ca/ca.pem`.
- Config: `proxy_enabled`, `proxy_port`, `inspect_hosts`.
- Socket API 1.3: `get_proxy`, `reset_inspect_ca`, `status.proxy`; log entry
  fields `via`, `mode`, `bytes_in`, `bytes_out`.
- MCP: seventh tool `get_proxy`.
- New dependency: `rustls-platform-verifier`.
- No new program, process or background job.

## Not in this ADR

- Scripts that read or change requests and bodies: [ADR 07](../07-proxy-scripts-lua-2026-10-01/README.md).
- An upstream (company) proxy (U1), proxy authentication (U2).
- The macOS system proxy setting, PAC files.
- Browsers other than Google Chrome in the right-click menu.
- SOCKS, HTTP/3.

## Diagrams

All diagrams are D2 sources in [diagrams/](diagrams/) with rendered `.svg`
files. `diagrams/_palette.d2` is the shared colour palette, copied from ADR 05.
Render one with:
`d2 --theme 0 --pad 20 diagrams/01-proxy-port.d2 diagrams/01-proxy-port.svg`
