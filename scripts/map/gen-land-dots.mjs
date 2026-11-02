// Makes apps/menubar/Sources/LocalRouter/TrafficMap/land-dots.json, the dot
// map of the traffic map window (ADR 11, change 2).
//
// It reads Natural Earth land-50m (public domain, from the world-atlas
// package), projects it with Mercator on the 760 x 446 crop of the design
// canvas (longitude -130 to 160, latitude 74 to -50), and keeps one point
// every 6 px that is on land. MapProjection.swift uses the same projection.
//
// Run a copy of it in an empty folder (Node finds the packages next to the
// script, so the copy must sit beside node_modules):
//   npm init -y >/dev/null && npm install d3-geo@3.1.1 topojson-client@3.1.0 world-atlas@2.0.2 && cp <repo>/scripts/map/gen-land-dots.mjs . && node gen-land-dots.mjs > land-dots.json
// Add --svg to also write land-dots.svg next to it, for a look.
import * as d3 from 'd3-geo';
import {feature} from 'topojson-client';
import fs from 'fs';

const topo = JSON.parse(fs.readFileSync('node_modules/world-atlas/land-50m.json'));
const land = feature(topo, topo.objects.land);
const W = 760, s = W / (290 * Math.PI / 180);
const tx = s * 130 * Math.PI / 180, ty = s * Math.log(Math.tan(Math.PI / 4 + 37 * Math.PI / 180));
const H = Math.round(ty - s * Math.log(Math.tan(Math.PI / 4 - 25 * Math.PI / 180)));
const proj = d3.geoMercator().scale(s).translate([tx, ty]);
const step = 6;
const dots = [];
for (let y = step / 2; y < H; y += step)
    for (let x = step / 2; x < W; x += step)
        if (d3.geoContains(land, proj.invert([x, y]))) dots.push([x, y]);

if (process.argv.includes('--svg')) {
    const circles = dots.map(([x, y]) => `<circle cx="${x}" cy="${y}" r="1.7"/>`).join('');
    fs.writeFileSync('land-dots.svg', `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}"><g fill="#C4CBC6">${circles}</g></svg>`);
}
// One dot per line keeps diffs small.
console.log(`{"width":${W},"height":${H},"step":${step},"source":"Natural Earth land-50m (public domain)","dots":[\n${dots.map(d => JSON.stringify(d)).join(',\n')}\n]}`);
