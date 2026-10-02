# {{APP}} scripts

{{INSTANCE_NOTE}}A script is a Lua 5.4 file that runs on the HTTP traffic of a route
(`shop.localhost`) or of the proxy (`api.example.com`). A **script rule** says
which traffic a script runs on. There are two kinds of script:

- **intercept**: runs while the client waits. It may change a request, answer
  it without the server, change a response, and change or drop each event of
  a stream.
- **log**: gets a copy of each finished request and response, after the
  client has it. It writes files in its `output_dir`. It can never slow or
  change traffic.

This page is the reference. Print it again: `{{CLI}} rules api`, or
`curl -s {{HELP_URL}}/scripts`

## Steps for a coding agent

1. Write the `.lua` file.
2. Test it, with nothing stored: `{{CLI}} rules check ./capture.lua`, or MCP
   `set_script_rule` with `check_only: true`. A mistake comes back with the
   file and line: `capture.lua:12: attempt to index a nil value (global 'jsn')`.
3. Set the rule **before** you start the job it is for, so it sees the first
   requests:
   `{{CLI}} rules add claude --host api.anthropic.com --path /v1/messages --script ./capture.lua --output-dir ./captures`
   With MCP: `set_script_rule` with `id`, `host`, `script` and, for a log
   rule, `output_dir` (absolute paths).
4. Give the rule `owner_pid` of a process you started for the job, so the rule
   goes away with it. Otherwise remove it when the job is done:
   `{{CLI}} rules rm claude` (MCP `remove_script_rule`). Make a rule
   `persistent` only when the user asks.
5. See whether it works: `{{CLI}} rules` (MCP `get_proxy`, field
   `script_rules`): `matched`, `errors`, `last_error`, `dropped`,
   `bytes_written`. `{{CLI}} logs` shows the rules that ran on each request.

A rule on a host outside `.localhost` makes that host **inspected** by the
proxy, so its HTTPS requests can be read. Programs must send their traffic
through the proxy (`{{CLI}} proxy env`) and trust the inspection CA
(`{{CLI}} proxy trust`, which only the user can do).

## Script rules

| Field | Meaning |
|---|---|
| `id` | the rule's name: `a-z`, `0-9`, `-`. Setting an existing id replaces that rule |
| `host` | `api.example.com`, `*.example.com` (one or more labels in front), `*` (every host outside `.localhost`; it makes the proxy inspect every host), or a route: `shop.localhost`, `*.shop.localhost` |
| `path` | a path prefix: `/v1` matches `/v1` and `/v1/messages`, never `/v1x` |
| `methods` | `["POST"]`; none means every method |
| `script` | absolute path of the `.lua` file, at most 1 MB |
| `output_dir` | log rules: absolute path of an existing folder, the only one the script writes |
| `order` | intercept rules run from low to high (default 100), then by id |
| `on_error` | intercept rules: `fail` (default: the client gets a 502 page that names the rule) or `pass` (the traffic goes on unchanged) |
| `max_capture_bytes` | log rules: bytes the rule may write, files and saved bodies together (default 1 GiB, counted from when the rule was set or the daemon started) |
| `enabled` | a disabled rule matches nothing and keeps its counters |
| `note` | why the rule exists |
| `owner_pid` | the rule is removed when this process exits |
| `persistent` | kept after a daemon restart |

Scripts run on route traffic (dev servers and folder routes) and on proxy
traffic that is plain `http://` or inspected. They never run on this help
page, the 404 page, TCP routes, or tunnels.

After **20 failed calls in a row** a rule is turned off, and `last_error`
says why. Turn it on again: `{{CLI}} rules enable <id>`.

## The shape of a script

A script returns a table with `kind` and its functions.

Intercept:

```lua
-- add-trace.lua: add a header, block one path, change one JSON field
return {
  kind = "intercept",
  request_body = { "text" },   -- hold text request bodies for on_request (true means the same)
  response_body = false,       -- hold no response bodies (the default)

  on_request = function(req)
    req.headers["x-trace"] = "lr-" .. req.id
    if req.path == "/v1/admin" then
      return { status = 403, headers = { ["content-type"] = "text/plain" }, body = "blocked" }
    end
    if req.body then
      local data = json.decode(req.body)
      data.max_tokens = 256
      req.body = json.encode(data)
    end
  end,

  on_response = function(req, res)
    res.headers["x-seen-by"] = "localrouter"
  end,

  -- Each event of a server-sent events or NDJSON stream, as it passes.
  -- Return false to drop the event.
  on_event = function(req, res, event)
    if event.event == "ping" then return false end
  end,
}
```

Log:

```lua
-- capture.lua: one JSON line per call into output_dir/messages.jsonl
return {
  kind = "log",
  copy = { "text", "events" },  -- the default
  save = {},                    -- classes written straight to files (see Bodies)

  on_exchange = function(ex)
    capture.append("messages.jsonl", json.encode({
      time = ex.time_ms,
      method = ex.request.method,
      path = ex.request.path,
      status = ex.response.status,
      request = ex.request.body,
      response = ex.response.body,
      truncated = ex.request.body_truncated or ex.response.body_truncated,
    }) .. "\n")
  end,

  -- Optional: a copy of each event of a stream, in order. A stream that
  -- never ends never reaches on_exchange, but its events reach on_event.
  on_event = function(ex, event)
    capture.append("events.jsonl", json.encode({ i = event.index, data = event.data }) .. "\n")
  end,
}
```

A log rule never needs `response_body`. An intercept rule with
`response_body` makes the client wait for the whole body: **streaming
stops** for that body. Use `on_event` for streams instead.

## req, res and ex

`req` (intercept) and `ex.request` (log):

| Field | Example | Notes |
|---|---|---|
| `id` | `"01JB9…"` | unique per request; the same in every hook |
| `source` | `"proxy"` or `"router"` | |
| `route` | `"shop/blog"` | route traffic only |
| `method` | `"POST"` | may be changed |
| `scheme`, `host`, `port` | `"https"`, `"api.example.com"`, `443` | read only: a request cannot be sent somewhere else |
| `path` | `"/v1/messages"` | may be changed |
| `query` | `"beta=true"` | without `?`; `""` when none; may be changed |
| `headers` | `{ ["content-type"] = "application/json" }` | lower-case names; a repeated header is a list; set a value to `nil` to remove it |
| `body` | a Lua string, or `nil` | decoded from gzip, deflate, br and zstd |

`res` (intercept) and `ex.response` (log): `status`, `headers`, `body`, and
the body fields below.

`ex` (log) has `request` and `response` (the tables above), `time_ms`,
`duration_ms`, `error` (for example "the client closed the connection"),
`answered_by` (the id of an intercept rule that answered), and `upgraded`
(`true` for a WebSocket after `101`; frames are not copied).

Body fields, on `req`, `res`, `ex.request` and `ex.response`:

| Field | Example | Notes |
|---|---|---|
| `content_type` | `"video/mp4"` | the header, or `nil` |
| `body_class` | `"media"` | see Bodies |
| `body_size` | `5242880` | bytes on the wire; in `on_exchange` always |
| `body_truncated` | `false` | the copy or hold stopped at its limit |
| `body_skipped` | `"class"` | why there is no body: `class`, `too_large`, `budget`, `streamed`, `slow_disk`, `quota`, `save_error`, `upgrade`, `error` (the body broke) |
| `body_file` | `"bodies/01JB9…-res.mp4"` | log rules with `save`: the file inside `output_dir` |
| `body_file_truncated` | `false` | |
| `body_encoding` | `"gzip"` | the original `Content-Encoding` |
| `range` | `"bytes 0-1023/50000"` | `Range` on requests, `Content-Range` on responses |
| `charset` | `"utf-8"` | from the Content-Type; charsets are not converted |
| `events_skipped` | `0` | events over 1 MB, not given to a script |

`event` (`on_event`): `event`, `data`, `id`, `retry`, and for log scripts
`index` (from 1). An intercept script may change `data` and the other fields.

**What a change means.** Setting `req.body` or `res.body` replaces the body;
it is sent without `Content-Encoding`, with a new `Content-Length`. Returning
a table from `on_request` answers the request: the server is not called,
later intercept rules and every `on_response` do not run, and log rules still
see the exchange with `answered_by` set.

## Bodies

Every body has a class, from its Content-Type only (the bytes are never
looked at):

| Class | Content types | Intercept | Log |
|---|---|---|---|
| `text` | `text/*`, `application/json`, `*+json`, `application/xml`, `*+xml`, `application/javascript`, `application/x-www-form-urlencoded`, `application/graphql` | held only when listed | copied |
| `events` | `text/event-stream`, `application/x-ndjson`, `application/jsonl` | never held; use `on_event` | copied |
| `media` | `image/*`, `audio/*`, `video/*`, `font/*` | held only when listed | counted only |
| `multipart` | `multipart/*` (uploads) | held only when listed | counted only |
| `binary` | everything else, and no Content-Type | held only when listed | counted only |

- **Intercept**: `request_body` and `response_body` list the classes to hold.
  A held body waits until it is complete, up to 8 MB after decoding. A larger
  body is sent as it comes; the script sees `body = nil`,
  `body_skipped = "too_large"`, and may not change it.
- **Log, `copy`**: classes copied into memory for `on_exchange`, up to 16 MB
  each (then `body_truncated = true`). Each part goes to the client first.
- **Log, `save`**: classes written straight to
  `output_dir/bodies/<id>-req|res.<ext>` while they stream, decoded, with
  mode 0600: images, video, downloads, uploads. A class is in `copy` or in
  `save`, not both. A save stops (and the file is marked truncated) when the
  disk falls behind or the quota is reached; the client is never slowed.
- **Event streams**: with `on_event`, the stream is split into events as it
  arrives. An intercept `on_event` holds one event at a time; a log
  `on_event` gets a copy of each. A log rule with `on_event` gets
  `body_skipped = "streamed"` in `on_exchange`.
- **Partial content**: a `206` response or a request with `Content-Range`
  holds one part. Its saved file name has `.part`, `range` says which part,
  and a script may change its headers but **not its body**.
- All held bodies, copies and queued events together use at most 512 MB.
  Past that, copies are dropped and counted in `dropped`.

A Lua string holds any bytes. `json.encode` refuses a string that is not
UTF-8 (use `base64.encode`, or `save` the body); `utf8.len(s)` tells.

## Modules

| Module | Functions | Kinds |
|---|---|---|
| `json` | `encode(v)`, `decode(s)`, `null` | both |
| `sse` | `parse(s)`: a list of `{ event, data, id, retry }` from a server-sent events body | both |
| `base64` | `encode(s)`, `decode(s)` | both |
| `url` | `parse_query(s)` (a repeated key gives a list), `encode(s)`, `decode(s)` | both |
| `multipart` | `parse(body, content_type)`: a list of `{ name, filename, content_type, headers, data }` | both |
| `log` | `info(msg)`, `warn(msg)`: a line in the daemon log with the rule id; at most 200 lines a minute per rule; `print` is `log.info` | both |
| `capture` | `write(name, data)` (replace), `append(name, data)` | log only |
| standard Lua | `string`, `table`, `math`, `utf8`, `coroutine`, `os.time`, `os.date`, `os.clock` | both |

`capture` names are flat: letters, digits, `.`, `_` and `-`, starting with a
letter or digit. Files are made with mode 0600 and never through a link.
`capture` works inside `on_exchange` and `on_event`, not while the file loads.

There is no `io`, `os.execute`, `require`, `load`, `debug`, network access, or
file access other than `capture`. Precompiled bytecode is refused.

## Limits

| Limit | Value |
|---|---|
| time per intercept call | 50 ms |
| time per log call | 2 s |
| time for the top level of the file | 100 ms |
| memory per Lua state | 64 MB |
| script file | 1 MB |
| one event given to `on_event` | 1 MB |
| rules | 64 |

**Keep no state in globals.** An intercept rule runs up to 4 Lua states at
the same time and they do not share globals; a changed file makes new states.
Write what must last to a capture file.

**Reload.** The file is checked at most once a second while the rule is in
use. A changed file is loaded again; if it is broken, the old version keeps
running and `last_error` shows the message.

## Headers

Scripts see every header as it is, cookies and API keys too. A new value
replaces it; `nil` removes the header. Query strings and bodies are not
changed either: a body sent to a model API holds the prompt and the code in
it.

**Do not commit captures.** Scripts may live in the project, but keep
`output_dir` out of git (add it to `.gitignore`): captures hold prompts, API
replies, cookies, API keys and files the user saw.
