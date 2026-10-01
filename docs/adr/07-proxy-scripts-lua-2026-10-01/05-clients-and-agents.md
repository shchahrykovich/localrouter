# 5. Clients and agents: set rules, read results, learn the API

**Status:** Proposed.

## Context

The user's request names agents as the main users: "agent should be able to
get params of proxy. assign lua scripts based on rules." An agent needs four
things: the proxy settings (ADR 06 `get_proxy`), the script API reference, a
way to test and assign a script, and a way to see whether its rule works.

## Decision

### Socket API 1.3 → 1.4 (additive)

| Method | What |
|---|---|
| `set_script_rule` | create or replace a rule by `id`. With `check_only: true`, check everything (fields, `output_dir` probe, compile, kind) and store nothing |
| `remove_script_rule` | remove by `id`; `{"removed": true|false}` |
| `list_script_rules` | every rule with its state (below) |

`METHODS` grows from 13 to 16. `get_proxy` (ADR 06) gains `script_rules`, the
same list, so one call gives an agent everything about the proxy.

The reply of `set_script_rule`:

```json
{
  "rule": { "id": "claude-capture", "kind": "log", "host": "api.anthropic.com", "path": "/v1/messages", "...": "..." },
  "replaced": false,
  "inspect": { "host_added": true, "ca_created": true, "ca_trusted": false },
  "notes": [
    "api.anthropic.com is now inspected. Clients must trust the inspection CA: NODE_EXTRA_CA_CERTS=/Users/me/Library/Application Support/LocalRouter/inspect-ca/ca.pem, or run: localrouter proxy trust",
    "The proxy is off. This rule sees only router traffic until: localrouter proxy on"
  ]
}
```

The state of each rule in `list_script_rules`:

| Field | Meaning |
|---|---|
| `kind` | `intercept` or `log`, from the script |
| `enabled` | `false` after 20 failures in a row, or when set so |
| `matched` | exchanges that matched since the rule was set |
| `answered` | intercept: responses returned by `on_request` |
| `errors`, `last_error` | count; the last message and its time |
| `dropped` | log: copies dropped because a limit was reached |
| `bytes_written` | log: bytes written to capture files |
| `script_loaded_at`, `script_sha256` | which version of the file runs |

### Request log

HTTP entries get two optional fields, so older clients still decode them:
`rules` (ids of the rules that ran) and `script_error` (the id of a rule that
failed). Bodies and headers are still never in the request log.

### MCP: two more tools, nine in total

| Tool | Arguments |
|---|---|
| `set_script_rule` | `id`, `host`, `path`, `methods`, `script`, `output_dir`, `order`, `on_error`, `max_capture_bytes`, `enabled`, `note`, `owner_pid`, `persistent`, `check_only` |
| `remove_script_rule` | `id` |

There is no `reveal_secrets` argument: an agent that sends it gets an error,
because MCP arguments refuse unknown fields. Listing is `get_proxy`.

`apps/cli/tests/mcp.rs` changes its snapshot on purpose to nine tools
(ADR 01, invariant I13).

### The script reference

A new text template `libs/core/src/scripts.md`: the script shape, both kinds
with an example each, the fields of `req`, `res` and `ex`, the modules, the
limits, the body classes with their defaults, event streams, `Range` and
`206`, saved bodies, and the rules about secrets and globals. It is served at
`router.localhost/scripts` (the help page links it) and printed by
`localrouter rules api`. It names its instance through the templates of
ADR 04. The `set_script_rule` tool description points to it.

### CLI

| Command | What |
|---|---|
| `localrouter rules` | the list with counters and last errors |
| `localrouter rules add <id> --host H --script F [--path P] [--method M]… [--output-dir D] [--order N] [--on-error pass] [--persistent] [--owner-pid N] [--note T]` | `set_script_rule`; relative `--script` and `--output-dir` are made absolute |
| `localrouter rules add … --reveal-secrets` | refused unless standard input is a terminal, then asks for `yes` |
| `localrouter rules check <file> [--output-dir D]` | `check_only` |
| `localrouter rules rm <id>`, `rules enable <id>`, `rules disable <id>` | |
| `localrouter rules api` | prints the reference |

### Menu bar app

A **Scripts** list in Settings: id, kind, host and path, the counters, the
last error in red, an Enabled switch and Remove. A rule disabled after 20
failures shows the reason. Logs entries show the rule ids that ran.

### Agent texts

`help.md`, `note.md` and `mcp.md` get a short part:

1. the two kinds in one sentence each, and the reference at
   `router.localhost/scripts`;
2. test first with `check_only`, then assign;
3. give a rule `owner_pid` of a process you started for the job, or set a
   session rule *before* you start the job and remove it when done (a rule
   set after the job starts misses its first requests); a persistent rule
   only when the user asks;
4. a log rule never needs `response_body`; an intercept rule with
   `response_body` stops streaming;
5. scripts may live in the project, but `output_dir` with captures must not
   be committed: captures hold prompts and API replies;
6. secret headers are hidden; only the user can reveal them;
7. bodies come in classes (text, events, media, multipart, binary): a log
   rule copies text and events by default; for streams use `on_event`; to keep
   images, video, downloads or uploads use `save`, which writes files, not
   `copy`.

## Trade-offs

- **Nine tools.** Each tool costs a few lines in every agent's tool list. The
  alternative, one `script_rules` tool with an `action` argument, is harder
  for agents to call correctly.
- **A reference page instead of long tool descriptions.** The agent reads it
  once when it needs it, not on every session.

## Tests

T13 (daemon methods), T14 (contract), T15 (CLI), T16 (MCP), T17 (agent texts
and reference). See the [test plan](09-test-plan.md).
