# Land dots generator

The generator moved to [`scripts/map/gen-land-dots.mjs`](../../../../scripts/map/gen-land-dots.mjs)
(task 9). It reads Natural Earth `land-50m` (public domain, from the
`world-atlas` package), projects it with Mercator on the 760 × 446 crop
(longitude −130 to 160, latitude 74 to −50), keeps one point every 6 px that
is on land, and prints `land-dots.json` (`{"width", "height", "dots": [[x, y], …]}`).
The output is committed as
`apps/menubar/Sources/LocalRouter/TrafficMap/land-dots.json` (4,027 dots).

The design canvas version also printed the arc paths from London to sample
cities; `MapProjection.swift` makes the arcs now.

Run it in an empty folder:

```
npm init -y >/dev/null && npm install d3-geo@3.1.1 topojson-client@3.1.0 world-atlas@2.0.2 && cp <repo>/scripts/map/gen-land-dots.mjs . && node gen-land-dots.mjs > land-dots.json
```
