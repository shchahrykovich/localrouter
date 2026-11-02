# Land dots generator

`gen-land-dots.mjs` made the dot map of the design canvas. It reads Natural
Earth `land-50m` (public domain, from the `world-atlas` package), projects it
with Mercator on the 760 × 446 crop (longitude −130 to 160, latitude 74 to
−50), keeps one point every 6 px that is on land, and prints the arc paths
from London to the sample cities.

It writes `land-dots.svg` today. Task 9 changes the output to
`land-dots.json` (a list of `[x, y]`) for the Swift `Canvas`, and moves the
script to `scripts/map/`.

Run it in an empty folder:

```
npm init -y >/dev/null && npm install d3-geo@3.1.1 topojson-client@3.1.0 world-atlas@2.0.2 && node gen-land-dots.mjs > out.json
```
