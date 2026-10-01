# 0. Working backwards: what users might say

**Status:** Mixed. Quotes marked (real) are the project owner's own requests,
from the design conversation on 2026-10-01. The GitHub repository has no
issues yet (checked on 2026-10-01). Every other quote is simulated: written by
us to test the design before it is built.

| Mark | Meaning |
|---|---|
| **covered** | the ADR decides it; the link says where |
| **accepted** | the ADR knows about it and chose to leave it; the link says why |
| **gap, fixed** | the first draft did not answer it; the ADR now does |

## Real feedback

> "in addition to router traffic, let's create proxy. so that i can point my
> chrome to it or claude code and process traffic" (real)

> "so agent should be able to get params of proxy" (real)

> "can we add on right click on icon 'Open chrome via proxy'? this only if
> chrome is installed. in this mode this one chrome instance runs via proxy
> which are specified via command arguments" (real)

All three are answered: the proxy port ([01](01-proxy-port.md)),
`get_proxy` ([03](03-clients-and-agents.md)), and the right-click item
([03](03-clients-and-agents.md#right-click-menu-open-chrome-via-proxy)).

## The announcement

> **LocalRouter 0.2: a proxy for your browser and your agent**
>
> LocalRouter can now be the HTTP proxy of one Chrome window or one Claude
> Code session, so you can see every request they make.
>
> Right-click the menu bar icon and choose **Open Chrome via Proxy**. A
> separate Chrome window opens that sends its traffic through LocalRouter.
> Your normal Chrome does not change. For Claude Code or any command-line
> tool:
>
> ```
> eval "$(localrouter proxy env)" && claude
> ```
>
> By default LocalRouter only passes HTTPS through and shows which hosts were
> called. To see each request of a host, add it:
> `localrouter proxy inspect add api.example.com`, then
> `localrouter proxy trust` once. Your dev names keep working as before.
>
> Agents ask for everything they need with the new `get_proxy` tool.

## Who reacts

| Role | Setup | What they want |
|---|---|---|
| R1. Web developer | Chrome, a frontend that calls third-party APIs | see what the page really calls |
| R2. Developer who works through Claude Code | Claude Code with the LocalRouter MCP tools | see what their agent and its test runs send |
| R3. Automated client: a coding agent | reads the MCP instructions and tool descriptions literally | set up a browser or test run through the proxy |
| R4. User who does not want the feature | LocalRouter for dev names only | nothing changes, nothing is recorded |
| R5. Team member without LocalRouter | clones the same repository | nothing in the repository needs LocalRouter |
| R6. Operator or maintainer | upgrades, downgrades, answers questions | no surprise after a downgrade |

## Positive reactions

**R1: "One click and I have a Chrome that shows me every host the page
talks to."** **covered**, [03](03-clients-and-agents.md#right-click-menu-open-chrome-via-proxy).

**R2: "I added api.example.com to inspect and I see each POST, with the path
and status."** **covered**, [02](02-inspection-ca.md#what-inspection-shows-in-this-adr).

**R4: "I updated and nothing changed. No new certificate, no new port."**
The proxy is off by default and the inspection CA is made only on first need.
**covered**, [01](01-proxy-port.md) decision 3, manifest I5.

## Negative reactions: the user's own setup

**R1: "Chrome via Proxy opened, but my bookmarks and logins are gone."**
A separate profile is the only way to give one Chrome instance a proxy flag
while the normal Chrome runs. **accepted**,
[03](03-clients-and-agents.md#right-click-menu-open-chrome-via-proxy) point 4.

**R1: "My site opens fine in the proxy Chrome, but none of the requests to
shop.localhost show up."** Chrome never sends loopback names to a proxy.
Router requests are still in Logs, without the `proxy` label.
**covered**, [03](03-clients-and-agents.md#right-click-menu-open-chrome-via-proxy) point 6.

**R2: "I asked Claude to send its own traffic through the proxy. It said yes,
and nothing changed."** A running program does not re-read `HTTPS_PROXY`.
**covered**, [03](03-clients-and-agents.md) "An agent cannot move its own
traffic"; the agent texts say it (I15).

**R2: "My Node test uses fetch and its calls do not show up."** Recent
Node.js `fetch` reads `HTTPS_PROXY` only with `NODE_USE_ENV_PROXY=1`.
**gap, fixed (G1)**: `get_proxy.env` and `proxy env` include it; M5 checks one
runner. [03](03-clients-and-agents.md).

**R1: "After `proxy inspect add example.com`, Chrome shows a certificate
error."** The inspection CA is not trusted yet. `get_proxy.notes` and the
popover say so. **covered**, [02](02-inspection-ca.md) trade-offs.

## Negative reactions: the feature's own behaviour

**R2: "I turned the proxy off and now Claude Code cannot reach the API at
all."** The shell still has `HTTPS_PROXY`. Nothing LocalRouter can do reaches
that process. **gap, fixed (G2)**: `proxy off` prints which programs will fail
and what to do; the agent texts say it. [03](03-clients-and-agents.md) CLI
table, T10.

**R5: "Since yesterday every API call in our test suite fails on my laptop.
Someone's agent put HTTPS_PROXY=http://127.0.0.1:8877 into the project's
.env."** **gap, fixed (G3)**: the agent texts carry the rule "never write
proxy settings into project files", and T12 checks it. Manifest I15.

**R4: "Does LocalRouter now record my banking site?"** No: tunnel by default,
no bodies or headers in the log, log in memory only.
**covered**, [02](02-inspection-ca.md) decision 1, manifest I7, I11.

**R1: "A site I inspect broke: the app pins its certificate."**
**accepted**, [02](02-inspection-ca.md) trade-offs. Only listed hosts are
affected; remove the host to fix it.

**R1: "Which of my two Chrome windows is the proxy one?"** Chrome has no flag
to name a profile. **accepted**, manifest blast radius, "User's normal
Chrome".

**R6: "After I downgraded, my inspect list was gone."** An older daemon drops
the unknown fields on its next write. **accepted**, manifest section 10.

**R6: "Another account on this Mac uses my proxy."** Loopback is shared by
all accounts. **accepted for now**, manifest unresolved effect U2, decided in
task 12 before release.

**R2: "At work every connection must go through the company proxy, and
LocalRouter's proxy cannot reach anything."** **accepted**, manifest
unresolved effect U1.

**R6: "The local CA and the inspection CA both say LocalRouter in Keychain
Access; which do I remove?"** The common names differ ("Inspection").
**covered**, [02](02-inspection-ca.md) decision 3.

## A simulated session: the agent

The user says: "Run the checkout Playwright test through the LocalRouter
proxy and show me which APIs it calls."

1. The agent reads the MCP instructions and sees the `get_proxy` tool.
2. It calls `get_proxy`. Reply: `enabled: false`, a note "The proxy is off.
   Run: localrouter proxy on".
3. It runs `localrouter proxy on` in its shell. The user sees the command.
4. It calls `get_proxy` again: `url`, `env`, `chrome_args`, no `inspect_ca`.
5. It decides the test needs only host names, so it does not inspect.
   *Without the texts of this ADR, the agent would likely add
   `HTTPS_PROXY` to `playwright.config.ts`. The rule in the agent texts (G3)
   stops this.*
6. It starts the test with the proxy as a launch option of the test command
   only: `HTTPS_PROXY=… npx playwright test checkout`, or Playwright's
   `--proxy-server` from `chrome_args`.
7. It calls `get_logs` with a limit and lists the hosts with `via: "proxy"`.
8. The user asks for request paths of `api.payments.example`. The agent runs
   `localrouter proxy inspect add api.payments.example`. `get_proxy.notes` says
   the CA is not trusted. The agent tells the user to run
   `localrouter proxy trust` (it cannot type the password), and passes
   `NODE_EXTRA_CA_CERTS` to the test run meanwhile.
9. It runs the test again and reports the paths from `get_logs`.

This is the script of manual test M5.

## Gaps

| # | Gap | Fix | Where |
|---|---|---|---|
| G1 | Node.js `fetch` ignores `HTTPS_PROXY` without `NODE_USE_ENV_PROXY` | add it to `env` | [03](03-clients-and-agents.md), T10, M5 |
| G2 | after `proxy off`, configured programs fail with no hint | `proxy off` prints which programs fail and what to do | [03](03-clients-and-agents.md), T10 |
| G3 | an agent writes proxy settings into the project, and teammates break | a rule in the agent texts | [03](03-clients-and-agents.md), manifest I15, T12, task 9 |
