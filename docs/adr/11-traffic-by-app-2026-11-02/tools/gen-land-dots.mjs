import * as d3 from 'd3-geo';
import {feature} from 'topojson-client';
import fs from 'fs';
const topo=JSON.parse(fs.readFileSync('node_modules/world-atlas/land-50m.json'));
const land=feature(topo,topo.objects.land);
const W=760,s=W/((290)*Math.PI/180);
const tx=s*130*Math.PI/180, ty=s*Math.log(Math.tan(Math.PI/4+37*Math.PI/180));
const H=Math.round(ty - s*Math.log(Math.tan(Math.PI/4-25*Math.PI/180)));
const proj=d3.geoMercator().scale(s).translate([tx,ty]);
const step=6;let c=[];
for(let y=step/2;y<H;y+=step)for(let x=step/2;x<W;x+=step){const p=proj.invert([x,y]);if(d3.geoContains(land,p))c.push('<circle cx="'+x+'" cy="'+y+'" r="1.7"/>');}
fs.writeFileSync('land-dots.svg','<svg xmlns="http://www.w3.org/2000/svg" width="'+W+'" height="'+H+'" viewBox="0 0 '+W+' '+H+'"><g fill="#C4CBC6">'+c.join('')+'</g></svg>');
const home=[-0.13,51.51];
const cities={ashburn:[-77.49,39.04],frankfurt:[8.68,50.11],amsterdam:[4.90,52.37],dublin:[-6.26,53.35],sanjose:[-121.89,37.34],stockholm:[18.07,59.33],saopaulo:[-46.63,-23.55],mumbai:[72.88,19.08],singapore:[103.82,1.35],tokyo:[139.69,35.68],sydney:[151.21,-33.87],london:home};
const out={W,H,dots:c.length,home:proj(home).map(v=>+v.toFixed(1)),cities:{}};
for(const [k,ll] of Object.entries(cities)){const xy=proj(ll).map(v=>+v.toFixed(1));const it=d3.geoInterpolate(home,ll);let pts=[];for(let i=0;i<=20;i++){pts.push(proj(it(i/20)).map(v=>Math.round(v)).join(','));}out.cities[k]={xy,path:'M'+pts.join(' L')};}
console.log(JSON.stringify(out,null,1));
