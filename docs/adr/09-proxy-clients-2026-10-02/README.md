# ADR 09: Proxy clients, one proxy port per program

**Status:** Accepted, implemented 2026-10-02.

## Context

The forward proxy (ADR 06) has one port, `127.0.0.1:8877`. Every program
that uses it writes to the same proxy log (ADR 08). When Chrome, two Claude
Code sessions and a test run all use the proxy, the log does not say which
program sent which request.

The user asked for "several proxies, so that I can assign each to a
different client", with the viewer at `proxy.localhost/client1`,
`proxy.localhost/client2`, a view of all of them, and a drop-down per client.

## How a client can be told apart

A forward proxy sees a TCP connection, then `GET http://…` or
`CONNECT host:443`. It has no URL path of its own. So the proxy can know the
client only in two ways:

| Way | Example | Chrome `--proxy-server` | `HTTPS_PROXY` (curl, Node.js, Claude Code) |
|---|---|---|---|
| The port the client connects to | `http://127.0.0.1:8878` | ✓ | ✓ |
| A user name in `Proxy-Authorization` | `http://chrome@127.0.0.1:8877` | ✗ (Chrome cannot put credentials in the flag; it asks the user) | ✓ |

## Decision

1. **A proxy client is one more proxy port with a name.** `config.json` has
   `proxy_clients: [{"name": "chrome", "port": 8878}]`. Names are 1 to 32 of
   `a-z`, `0-9` and `-`. `default`, `all`, `api` and `files` are reserved:
   `default` is the main port, and the others are paths of the viewer.
2. **The client ports follow the main port.** They bind when the proxy turns
   on and close (with their connections) when it turns off. Each binds
   `127.0.0.1` and `::1` only, like the main port (ADR 06, I1). A client port
   may not be the HTTP, HTTPS or main proxy port, or another client's.
3. **`set_config` with `proxy_clients` replaces the list**, like
   `inspect_hosts` and `lan_networks`. A client that the call adds or moves
   must bind, or the call fails with `port_in_use` and changes nothing (as
   ADR 06, I12). A client that only binds again (proxy on, daemon start)
   binds if it can; a failure is in `status.proxy.clients[].errors`.
4. **The log names the client.** Each HAR entry from a client port has
   `_client: "<name>"`. The main port writes no `_client`, so files written
   before this ADR read as `default`. A `.localhost` request through a client
   port and a tunnel carry the name too.
5. **The loop check knows every proxy port.** A request on one proxy port
   for another proxy port of the same daemon gets `508`.
6. **`get_proxy` takes `client`.** The reply then has that port's `url`,
   `env` and `chrome_args` (with a Chrome profile of its own,
   `chrome-proxy-<name>`, because Chrome sends a new window to the instance
   that already runs with the same profile), `client`, and `log.url` set to
   `proxy.localhost/<name>`. Every reply lists `clients`. API 1.6.
7. **The viewer.**
   - `proxy.localhost/<name>` and `router.localhost/proxy-log/<name>` serve
     the page with that client chosen, for `default` and the configured
     clients. Any other name is 404.
   - `/api/files` lists `clients` (name and bound port).
   - `/api/entries?client=<name>` returns only that client's entries. One
     call looks at most at 64 MB of entry lines (`SCAN_BYTES`) and then
     gives a cursor, so a filter that matches little never reads a whole file.
   - The page has a "Proxy" menu: All, then each port. Choosing one changes
     the address (`history.replaceState`). With all clients shown and more
     than one port, the table has a "Proxy" column and a "Proxy" group.
8. **CLI:** `proxy client add <name> [--port N]` (without `--port`: the next
   free port after the main port and the clients' ports), `proxy client rm`,
   `proxy client` (the list), and `--client` on `proxy env`, `proxy chrome`
   and `proxy log open`. MCP `get_proxy` takes `client`; there are still nine
   tools.

## Trade-offs

- **One log for all clients.** The 5 files and their limits are shared. A
  busy client pushes the entries of a quiet one out of the kept files. Files
  per client would avoid this, but "All" would then have to merge several
  files by time.
- **No Proxy-Authorization names.** A program that can only use one proxy
  URL for several of its own parts cannot split them. Chrome could not use
  this way at all, so it is left for later.
- **Script rules do not see the client.** A Lua script cannot yet tell which
  port carried an exchange.
- **The menu bar app does not manage clients yet.** It decodes the new
  fields; adding and removing clients is a CLI command.

## Tests

- `config.rs`: names, defaults, round trip.
- `har/entry.rs`: `_client` written for a client, absent for the main port.
- `har/viewer.rs`: the client filter, the 64 MB scan limit with its cursor,
  the client pages and the `/<name>/` redirect.
- `libs/core/tests/forward.rs`: every mode on a client port (http, inspect,
  `.localhost` by absolute form and by `CONNECT`, tunnel, 508) carries the
  name; the main port writes none.
- `apps/daemon/tests/api.rs`: bind with the proxy and the list, off closes
  open tunnels, a taken port changes nothing, name and port checks, saved
  clients bind at start, the loop check, `get_proxy` per client, the viewer's
  list, filter and pages.
- `api/examples/`: `get_proxy_client`, `set_config_proxy_clients`, and the
  `clients` fields in `status`, `get_proxy` and `get_config`, checked by both
  contract tests.
