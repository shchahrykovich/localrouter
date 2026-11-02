# 2. Places and the traffic map window

**Status:** Proposed (not built).

## Context

The user asked for "a separate window for a world map". A map needs a place
(city, country, latitude, longitude) for each remote address, and a place for
this Mac.

## Decision

### 2.1 Places come from a database on this Mac

The daemon reads two DB-IP Lite databases in MaxMind format (`.mmdb`) with the
`maxminddb` crate:

| File | Gives | License |
|---|---|---|
| `<data>/geo/dbip-city-lite.mmdb` | city, country, latitude, longitude | CC BY 4.0 |
| `<data>/geo/dbip-asn-lite.mmdb` | the network owner, for example `GitHub, Inc.` | CC BY 4.0 |

CC BY 4.0 requires credit. The map window footer and the Settings page show
"IP geolocation by DB-IP" with a link to `https://db-ip.com`.

A lookup never uses the network. The places are city-level guesses: a CDN
address often maps to the user's own city, which is why the design shows
"London, same city".

### 2.2 Download only on request

The files are not in the app bundle (the city file is large; task 0 measures
it). The daemon downloads them when the user turns on **Show places on the
map** or presses **Update**:

1. `update_geo_db` builds the URLs for the current month:
   `https://download.db-ip.com/free/dbip-city-lite-YYYY-MM.mmdb.gz` and
   `…/dbip-asn-lite-YYYY-MM.mmdb.gz`. If the current month answers 404, it
   tries the previous month.
2. It downloads through `upstream::platform_tls()` (verifying TLS, as every
   other outbound connection).
3. It writes to `<data>/geo/*.mmdb.part`, opens the result with `maxminddb` to
   prove it is valid, then renames it over the old file.
4. A failed download leaves the old file as it was.

There is no automatic update. The Settings page shows the database date and an
**Update** button.

### 2.3 Where this Mac is

The map needs one point for "This Mac". A public-IP lookup would be a network
call, so it is not used.

1. Default: the city of the time zone (`TimeZone.current.identifier`, for
   example `Europe/London` → London). A small table in the app maps time zone
   ids to coordinates. An unknown id gives no point, and the arcs start at
   the map center line.
2. The user can change it in Settings → Traffic ("This Mac is in: London").

### 2.4 The map window

A normal window, opened by **Map** in the Traffic tab and **Open Traffic Map**
in the menu. Layout from the canvas artboard `Map.dc.html`:

| Part | Content |
|---|---|
| title bar | range `1 hour` / `24 hours` / `7 days`, search (app, address, owner or country) |
| left | apps sorted by bytes in, `All apps` first; an app that needs review has a red shield; "Not on the map": router bytes and bytes with no place |
| center | the dot map, one arc from this Mac to each place, line width by bytes in; city labels for the largest places; zoom buttons |
| under the map | the histogram of the range (sent up, received down) |
| table | place, addresses, the app with the most data, in, out |
| right | Summary: sent, received; **Through** (router, proxy, direct); Top apps; Top owners; Top countries |

The dot map is drawn with a SwiftUI `Canvas`, not MapKit, so it needs no map
tiles from the network.

- The land dots are a resource file `TrafficMap/land-dots.json`: about 4,000
  points, made once from Natural Earth `land-50m` (public domain) by the
  script in task 9, and committed.
- `MapProjection` (in `LocalRouterKit`) holds the Mercator projection with the
  crop of the canvas (longitude −130 to 160, latitude 74 to −50) and the
  great-circle points of an arc. It is a pure type, so it has unit tests.

"Top domains" in the design becomes **Top owners** (the ASN owner), because
direct traffic has no host names in this ADR (change 1, table in 1.1). Proxy
and router traffic still show host names.

## Tests

T5 and T13 in the [test plan](09-test-plan.md), and M3.
