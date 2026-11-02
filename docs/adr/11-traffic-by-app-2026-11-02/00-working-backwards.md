# 0. Working backwards

**Status:** Mixed. Quotes marked (real) are the user's own messages in the
design session of 2026-11-02, in Russian, with an English translation. All
other quotes are simulated: no user has seen this feature.

## Real feedback

| Quote (real) | Translation | Answered in |
|---|---|---|
| "в таб траффик я хочу видеть кто что использует на моем маке. а так же отдельно окно для карты мира" | "in the Traffic tab I want to see who uses what on my Mac, and a separate window for a world map" | [01](01-app-traffic.md), [02](02-places-and-map.md) |
| "нету топ apps" | "there are no top apps" (in the map Summary) | [02](02-places-and-map.md), 2.4 |
| "нужен какой-то механизм проверки подлинности приложения и возможно отправки его на проверку virus total" | "we need a way to check that an app is genuine, and maybe send it to VirusTotal" | [03](03-app-checks.md) |
| "те кликнул и понял что за оно" | "so I click and understand what it is" | [03](03-app-checks.md), 3.5 |
| "супер было бы чат спрашивать - не в local router а через deep link или Link в chat gpt отправлять" | "it would be great to ask a chat, not inside LocalRouter, but by a deep link or a link to ChatGPT" | [04](04-ask-ai.md) |
| "а здесь лучше - router, proxy, direct. Through LocalRouter - Through" | "here it is better as router, proxy, direct, and 'Through LocalRouter' becomes 'Through'" | [01](01-app-traffic.md), 1.3 |
| "наверно тут надо обновить и добавить трафик" | "the menu probably needs an update with traffic" | [05](05-ui.md), 5.2 |

## The announcement

> **LocalRouter 0.2: see which apps use the network**
>
> The Traffic tab has a new **Apps** view. It lists every program on your Mac
> that sent or received data in the last hour, with a small activity graph
> and the bytes in and out. Each connection is marked **router** (through
> your `.localhost` routes), **proxy** (through the LocalRouter proxy) or
> **direct**.
>
> Press **Map** to see where the data went, on a world map.
>
> Click an app to see who signed it, whether Apple notarized it, and, with
> your free VirusTotal key, what VirusTotal knows about its hash. Not sure
> what a program is? **Ask Claude** or **Ask ChatGPT** opens a chat with the
> question already written.
>
> It is off until you turn it on. LocalRouter reads the traffic with
> `nettop`, which comes with macOS. Your history stays on your Mac.

## Roles

| Role | Setup and want |
|---|---|
| Main user | a developer with LocalRouter, wants to know what talks to the internet |
| Automated client | Claude Code with the LocalRouter MCP server |
| A user who does not want it | uses LocalRouter only for `.localhost` names |
| A team where not everyone has the tool | shares a repo with people without LocalRouter |
| Operator | the LocalRouter maintainer who ships releases and the `-dev` build |

## Reactions

### Positive

**R1: "I turned it on and saw that Spotify uploads more than I thought."**
The goal. **covered** by [01](01-app-traffic.md).

**R2: "The map shows that my traffic to Ashburn is mostly Chrome."**
**covered** by the table under the map, [02](02-places-and-map.md), 2.4.

**R3: "Ask Claude told me what that LaunchAgent was in ten seconds."**
**covered** by [04](04-ask-ai.md).

### Problems in the user's own setup

**R4: "My VPN is on, and the totals are twice what my provider says."**
A VPN tunnel carries other apps' bytes again, and nettop cannot tell which.
**accepted**, with a help text, [manifest](08-semantic-change-manifest.md#13-risks).

**R5: "My own Rust tool is flagged."**
`cargo` builds are ad-hoc signed. Ad-hoc does not need review unless the user
turns that on. **covered** by [03](03-app-checks.md), 3.4.

**R6: "VirusTotal says it does not know our internal tool. I uploaded it, and
now our company binary is with security companies."**
The dialog says this before the upload, and Cancel sends nothing.
**accepted**: the user decided. [03](03-app-checks.md), 3.3.

### Problems in the feature's own behaviour

**R7: "Chrome shows 41 addresses, but I wanted to see github.com."**
nettop has no host names for direct traffic. **accepted**; host names need a
network filter, U1 in the [manifest](08-semantic-change-manifest.md#12-unresolved-effects).

**R8: "Every byte I send through the proxy is counted twice: once for Chrome
and once for LocalRouter."**
The daemon's pid is dropped. **covered** by I2,
[01](01-app-traffic.md), 1.3.

**R9: "After the update the Apps view is empty and says 'No traffic'."**
The app was new and the daemon was still 1.8, so `get_app_traffic` did not
exist. This was a silent failure. **gap G1, fixed**: I16 and the "too old"
view, [05](05-ui.md), 5.4.

**R10: "After a macOS update the Apps view stays empty."**
nettop changed its output and nothing parsed. **gap G2, fixed**:
`traffic_error` "nettop output not understood",
[manifest](08-semantic-change-manifest.md#13-risks), task 5.

**R11: "The arcs start in the middle of the ocean."**
The first draft had no point for this Mac. **gap G3, fixed**: the time zone
city and a setting, [02](02-places-and-map.md), 2.3.

**R12: "`curl` appears as `curl.4711` with no icon and no checks."**
The process ended before its path was read. **gap G4, fixed**: "(exited)"
rows, [01](01-app-traffic.md), 1.5.

**R13: "The map window makes the fan spin on 30 days."**
The first draft read up to 30 day files every 5 seconds. **gap G5, fixed**:
the per-minute cache, [01](01-app-traffic.md), 1.7.

**R14: "I pressed Quit process on mDNSResponder and lost the internet."**
**gap G6, fixed**: no Quit for Apple programs, [03](03-app-checks.md), 3.5.

**R15: "On the first day I got twenty notifications at once."**
**gap G7, fixed**: one summary notification above three in ten minutes,
[03](03-app-checks.md), 3.4.

**R16 (user who does not want it): "I only use routes. Is LocalRouter now
watching my apps?"**
No: `traffic` is off by default, no nettop process runs, no history is
written. **covered** by I1.

**R17 (team): "Does my repo get anything new?"**
No project file changes: the history is in the data folder, the checks in the
app's caches. **covered** by the [manifest](08-semantic-change-manifest.md#6-reads-and-writes).

**R18 (operator): "Every `-dev` build asks for the Keychain again."**
The Keychain access list names the ad-hoc signature. **accepted**, a
developer-only cost, [manifest](08-semantic-change-manifest.md#13-risks).

**R19 (operator): "Do releases need a new entitlement or profile?"**
No: nettop needs nothing. **covered** by [01](01-app-traffic.md), 1.1.

## A simulated session with the automated client

The user asks Claude Code: "which program on my Mac keeps calling
203.0.113.45?"

1. Claude Code reads the LocalRouter note (`LocalRouter.md`). It lists the
   MCP tools: routes, ports, proxy. Nothing about apps.
2. It calls `list_routes`. The address is not a route.
3. **Here today's texts make it act wrongly:** it may tell the user to turn
   on the proxy log and look in HAR files, which only show proxy traffic.
4. A correct answer is "open LocalRouter → Traffic → Apps and search for the
   address".

This ADR adds no MCP tool (I14). **accepted**: the agent cannot read app
traffic. A later ADR can add a tool; then the note and `mcp.md` must say what
it shows and that direct traffic has addresses, not host names.

## Gaps

| # | Gap | Fix | Where |
|---|---|---|---|
| G1 | a new app with an old daemon shows "No traffic" | version gate and a "too old" view | 05, manifest I16, T14, task 6 |
| G2 | a changed nettop format shows nothing | `traffic_error` when a sample fails to parse | 01, manifest risks, T1, task 5 |
| G3 | no point for this Mac on the map | time zone city, setting | 02, T13, task 9 |
| G4 | ended processes have no path | "(exited)" rows | 01, T2, task 1 |
| G5 | long ranges read many files every 5 s | per-minute cache | 01, manifest risks, T3, task 3 |
| G6 | Quit on a system process | no Quit for Apple programs | 03, T11, task 7 |
| G7 | many notifications at once | summary notification | 03, T11, task 7 |
