"use strict";

// The proxy log viewer. Data: /api/files (the folder), /api/entries (pages
// of entry summaries, newest first, without bodies), /api/entry (one whole
// entry) and /api/live (server-sent events: entry, open, message, file, off,
// lagged). The page is static: every element is built here with textContent,
// and positions are set through element.style (the CSP forbids inline style).

const PAGE = 500;          // entries per /api/entries call
const MAX_ROWS = 5000;     // rows kept on the page
const MAX_DRAWN = 1500;    // rows drawn in the table at once
const MAX_LINES = 4000;    // lines of a body preview
const FULL_CACHE = 30;     // whole entries kept
const STORE_KEY = "lr-proxy-log-view";

const $ = (id) => document.getElementById(id);

// ---- small helpers

/** Build an element. props: class, text, title, style (object), on* (function), any attribute. */
function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v === undefined || v === null || v === false) continue;
      if (k === "class") el.className = v;
      else if (k === "text") el.textContent = v;
      else if (k === "style") Object.assign(el.style, v);
      else if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2).toLowerCase(), v);
      else el.setAttribute(k, v === true ? "" : v);
    }
  }
  for (const kid of kids.flat()) {
    if (kid === undefined || kid === null || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

function fmtSize(n) {
  if (n === null || n === undefined || n < 0) return "";
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
  return (n / 1024 / 1024).toFixed(1) + " MB";
}

function fmtMs(ms) {
  if (ms < 1000) return Math.round(ms) + " ms";
  if (ms < 60000) return (ms / 1000).toFixed(ms < 10000 ? 2 : 1) + " s";
  return Math.floor(ms / 60000) + " min " + Math.round((ms % 60000) / 1000) + " s";
}

function clock(ms) {
  const d = new Date(ms);
  if (isNaN(d)) return "";
  const p = (n, w) => String(n).padStart(w || 2, "0");
  return p(d.getHours()) + ":" + p(d.getMinutes()) + ":" + p(d.getSeconds()) + "." + p(d.getMilliseconds(), 3);
}

function count(n) {
  return n.toLocaleString("en-US");
}

function load() {
  try {
    return JSON.parse(localStorage.getItem(STORE_KEY) || "{}") || {};
  } catch (e) {
    return {};
  }
}

function save() {
  try {
    const { density, tl, hide, group, detailW } = S;
    localStorage.setItem(STORE_KEY, JSON.stringify({ density, tl, hide, group, detailW }));
  } catch (e) {
    // Private windows: the view is not remembered.
  }
}

const STATUS_TEXT = {
  100: "Continue", 101: "Switching Protocols", 200: "OK", 201: "Created", 202: "Accepted", 204: "No Content",
  206: "Partial Content", 301: "Moved Permanently", 302: "Found", 303: "See Other", 304: "Not Modified",
  307: "Temporary Redirect", 308: "Permanent Redirect", 400: "Bad Request", 401: "Unauthorized", 403: "Forbidden",
  404: "Not Found", 405: "Method Not Allowed", 408: "Request Timeout", 409: "Conflict", 413: "Content Too Large",
  429: "Too Many Requests", 500: "Internal Server Error", 502: "Bad Gateway", 503: "Service Unavailable",
  504: "Gateway Timeout", 508: "Loop Detected",
};

function statusClass(s) {
  if (!s) return "s-none";
  if (s === 101) return "s-up";
  if (s >= 500) return "s-bad";
  if (s >= 400) return "s-warn";
  if (s >= 300) return "s-redir";
  return "s-ok";
}

function header(list, name) {
  const h = (list || []).find((x) => x.name.toLowerCase() === name);
  return h ? h.value : "";
}

function essence(mime) {
  return (mime || "").split(";")[0].trim().toLowerCase();
}

/** A row's type, as in Chrome's Network panel. */
function typeOf(e) {
  if (e._mode === "tunnel" || e.request.method === "CONNECT") return "tunnel";
  if (e._resourceType === "websocket" || e.response.status === 101) return "ws";
  const dest = header(e.request.headers, "sec-fetch-dest");
  const byDest = { document: "doc", iframe: "doc", frame: "doc", script: "js", style: "css", image: "img", font: "font", audio: "media", video: "media", track: "media", manifest: "other" };
  if (byDest[dest]) return byDest[dest];
  const m = essence(e.response.content && e.response.content.mimeType);
  if (m === "text/html") return dest === "empty" ? "fetch" : "doc";
  if (m.includes("javascript") || m.includes("ecmascript")) return "js";
  if (m === "text/css") return "css";
  if (m.startsWith("image/")) return "img";
  if (m.startsWith("font/") || m.includes("font")) return "font";
  if (m.startsWith("audio/") || m.startsWith("video/")) return "media";
  return "fetch";
}

function splitUrl(url) {
  try {
    const u = new URL(url);
    return { scheme: u.protocol + "//", host: u.host, path: u.pathname + u.search + u.hash };
  } catch (e) {
    return { scheme: "", host: "", path: url };
  }
}

function isError(r) {
  return r.status === 0 || r.status >= 400 || !!r.e._bodyError;
}

// ---- state

const saved = load();
const S = {
  q: "",
  inv: false,
  type: "all",
  fM: {}, fS: {}, fMode: {}, fHost: {},
  group: saved.group || "none",
  collapsed: {},
  hide: saved.hide || {},
  density: saved.density || "compact",
  tl: saved.tl !== false,
  live: true,
  range: null,            // [a, b], fractions of the timeline span
  sel: null,              // row key
  tab: null,
  ff: "all",              // message filter
  frame: null,            // message index
  detailW: saved.detailW || null,
  max: false,
  pop: null,              // "filters", "group", "cols", "menu"
  ctx: null,              // {x, y, key}
};

let info = null;          // the last /api/files answer
let file = null;          // the file shown
let rows = [];            // rows, newest first
const byKey = new Map();  // row key → row
let before = null;        // cursor for older entries
let loadingMore = false;
const full = new Map();   // file@at → whole entry
const fullWait = new Map();
const liveMsgs = new Map(); // _id → messages of an open WebSocket
let held = [];            // live events while paused
let order = [];           // keys in table order, for the arrow keys
let toastTimer = null;
let drawn = null;         // what the detail pane shows, to skip redraws
let fresh = new Set();

function rowOf(e, f) {
  const u = splitUrl(e.request.url);
  const status = e.response.status || 0;
  const type = typeOf(e);
  if (type === "ws") u.scheme = u.scheme === "http://" ? "ws://" : "wss://";
  const start = Date.parse(e.startedDateTime);
  const r = {
    key: e._id !== undefined ? "i" + e._id : "a" + (e._at !== undefined ? e._at : Math.random()),
    id: e._id, at: e._at, file: f, e,
    start: isNaN(start) ? 0 : start,
    method: e.request.method, status,
    url: e.request.url, scheme: u.scheme, host: u.host, path: u.path,
    type, mode: e._mode || "",
    wait: (e.timings && e.timings.wait) || 0,
    receive: (e.timings && e.timings.receive) || 0,
    ms: e.time || 0,
    size: e.response.bodySize >= 0 ? e.response.bodySize : e._bytesOut !== undefined ? e._bytesOut : -1,
    open: !!e._open,
    messages: e._messages !== undefined ? e._messages : null,
  };
  return r;
}

function upsert(r) {
  const old = byKey.get(r.key);
  if (old) {
    Object.assign(old, r);
    return old;
  }
  byKey.set(r.key, r);
  rows.push(r);
  return r;
}

function sortRows() {
  rows.sort((a, b) => b.start - a.start);
  if (rows.length > MAX_ROWS) {
    for (const r of rows.splice(MAX_ROWS)) byKey.delete(r.key);
  }
}

function endOf(r) {
  return r.open ? Date.now() : r.start + Math.max(r.ms, 0);
}

// ---- loading

async function loadFiles() {
  const r = await fetch("api/files", { cache: "no-store" });
  if (!r.ok) throw new Error("api/files: " + r.status);
  info = await r.json();
  if (!file || !info.files.some((f) => f.name === file)) {
    const current = info.files.find((f) => f.current) || info.files[0];
    if (current) pick(current.name);
  }
  render();
}

function pick(name) {
  if (file === name) return;
  // Requests still open belong to the live feed, not to a file: keep them
  // when the page follows the feed to a new file.
  const keep = following() ? rows.filter((r) => r.open) : [];
  file = name;
  rows = keep;
  byKey.clear();
  for (const r of keep) byKey.set(r.key, r);
  before = null;
  S.sel = null;
  S.range = null;
  loadEntries(false).catch(failure);
  render();
}

async function loadEntries(more) {
  const name = file;
  let url = "api/entries?file=" + encodeURIComponent(name) + "&limit=" + PAGE;
  if (more && before !== null) url += "&before=" + before;
  loadingMore = true;
  try {
    const r = await fetch(url, { cache: "no-store" });
    if (!r.ok) throw new Error("api/entries: " + r.status);
    const page = await r.json();
    if (name !== file) return;
    for (const e of page.entries) upsert(rowOf(e, name));
    before = page.before;
    sortRows();
  } finally {
    loadingMore = false;
    render();
  }
}

async function wholeEntry(r) {
  if (r.at === undefined || r.at === null || !r.file) return null;
  const k = r.file + "@" + r.at;
  if (full.has(k)) return full.get(k);
  if (fullWait.has(k)) return fullWait.get(k);
  const p = fetch("api/entry?file=" + encodeURIComponent(r.file) + "&at=" + r.at, { cache: "no-store" })
    .then((resp) => {
      if (!resp.ok) throw new Error("api/entry: " + resp.status);
      return resp.json();
    })
    .then((e) => {
      full.set(k, e);
      if (full.size > FULL_CACHE) full.delete(full.keys().next().value);
      return e;
    })
    .finally(() => fullWait.delete(k));
  fullWait.set(k, p);
  return p;
}

function cachedEntry(r) {
  return r.at !== undefined && r.at !== null ? full.get(r.file + "@" + r.at) : undefined;
}

// ---- live feed

/** The page shows the file new entries go to: the current one, or the
 *  newest while none is open (after a daemon restart the next entry starts
 *  a new file). */
function following() {
  if (!info || !file) return true;
  const target = info.files.find((f) => f.current) || info.files[0];
  return !target || target.name === file;
}

function startLive() {
  const live = new EventSource("api/live");
  const on = (name, fn) =>
    live.addEventListener(name, (ev) => {
      let data;
      try {
        data = JSON.parse(ev.data);
      } catch (e) {
        return;
      }
      if (!S.live && (name === "entry" || name === "open" || name === "message")) {
        held.push([fn, data]);
        if (held.length > 20000) held.shift();
        return;
      }
      fn(data);
    });
  on("entry", liveEntry);
  on("open", liveOpen);
  on("message", liveMessage);
  on("file", (name) => {
    if (following()) {
      loadFiles().then(() => pick(name)).catch(failure);
    } else {
      loadFiles().catch(failure);
    }
  });
  on("off", () => loadFiles().catch(failure));
  on("lagged", () => {
    const name = file;
    file = null;
    loadFiles().then(() => pick(name)).catch(failure);
  });
}

function liveEntry(e) {
  if (!info) return;
  const current = info.files.find((f) => f.name === e._file);
  if (current) current.entries = (current.entries || 0) + 1;
  if (!file) {
    loadFiles().catch(failure);
    return;
  }
  if (e._file !== file) {
    if (following()) loadFiles().then(() => { pick(e._file); liveEntry(e); }).catch(failure);
    return;
  }
  const r = upsert(rowOf(e, file));
  if (r.id !== undefined) liveMsgs.delete(r.id);
  fresh.add(r.key);
  sortRows();
  render();
}

function liveOpen(e) {
  if (!following()) return;
  const r = upsert(rowOf(e, file));
  fresh.add(r.key);
  sortRows();
  render();
}

function liveMessage(m) {
  let list = liveMsgs.get(m.id);
  if (!list) {
    list = [];
    liveMsgs.set(m.id, list);
  }
  list.push(m.message);
  const r = byKey.get("i" + m.id);
  if (r) {
    r.messages = list.length;
    render();
  }
}

function setLive(on) {
  S.live = on;
  if (on) {
    const queued = held;
    held = [];
    for (const [fn, data] of queued) fn(data);
  }
  render();
}

// ---- filtering

function rangeTimes(span) {
  if (!S.range || S.range[1] - S.range[0] < 0.005) return null;
  return [span.t0 + S.range[0] * span.len, span.t0 + S.range[1] * span.len];
}

function textMatch(r, q) {
  if (!q) return true;
  const hay = [r.method, String(r.status), r.url, r.mode, r.type].join(" ").toLowerCase();
  return q.every((w) => hay.includes(w));
}

function matchType(r, t) {
  if (t === "all") return true;
  if (t === "err") return isError(r);
  return r.type === t;
}

function statusGroup(r) {
  return r.status ? Math.floor(r.status / 100) + "xx" : "none";
}

function anyOn(o) {
  return Object.values(o).some(Boolean);
}

function visible(span) {
  const q = S.q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const range = rangeTimes(span);
  return rows.filter((r) => {
    if (range && (r.start > range[1] || endOf(r) < range[0])) return false;
    if (!matchType(r, S.type)) return false;
    const t = textMatch(r, q);
    if (S.inv && q.length ? t : !t) return false;
    if (anyOn(S.fM) && !S.fM[r.method]) return false;
    if (anyOn(S.fS) && !S.fS[statusGroup(r)]) return false;
    if (anyOn(S.fMode) && !S.fMode[r.mode]) return false;
    if (anyOn(S.fHost) && !S.fHost[r.host]) return false;
    return true;
  });
}

function timeSpan() {
  if (!rows.length) return { t0: 0, t1: 1, len: 1 };
  let t0 = Infinity;
  let t1 = -Infinity;
  for (const r of rows) {
    if (r.start < t0) t0 = r.start;
    const e = endOf(r);
    if (e > t1) t1 = e;
  }
  const pad = Math.max(200, (t1 - t0) * 0.02);
  t0 -= pad;
  t1 += pad;
  return { t0, t1, len: t1 - t0 };
}

// ---- drawing

// One draw for many changes, at most about 8 per second while live events
// arrive: a draw rebuilds the table. A timer, not requestAnimationFrame: a
// page in a background tab still keeps its rows (Chrome slows its timers).
let pending = false;
let lastDraw = 0;
function render() {
  if (pending) return;
  pending = true;
  const wait = Math.max(0, 120 - (performance.now() - lastDraw));
  setTimeout(() => {
    pending = false;
    lastDraw = performance.now();
    draw();
  }, wait);
}

const COLS = [
  ["time", "Time", "96px"],
  ["method", "Method", "62px"],
  ["status", "Status", "46px"],
  ["type", "Type", "54px"],
  ["request", "Request", "minmax(0, 1fr)"],
  ["mode", "Mode", "64px"],
  ["ms", "Duration", "68px"],
  ["bar", "Waterfall", "minmax(80px, 140px)"],
  ["size", "Size", "68px"],
];

function userOn(k) {
  return k === "request" || !S.hide[k];
}

function draw() {
  const span = timeSpan();
  const vis = visible(span);
  const sel = S.sel && byKey.get(S.sel);
  const shownSel = sel && vis.includes(sel) ? sel : sel || null;
  drawHeader();
  drawToolbar(vis);
  drawTypes();
  // The layout first: the timeline reads its canvas width.
  drawLayout(shownSel);
  $("timeline").hidden = !S.tl;
  $("tl-btn").classList.toggle("on", S.tl);
  drawTable(span, vis, shownSel);
  if (S.tl) drawTimeline(span, vis);
  drawDetail(shownSel);
  drawPops(vis);
  fresh = new Set();
}

function drawHeader() {
  const st = $("state");
  st.textContent = "";
  if (info) {
    st.append(h("span", { class: "chip " + (info.proxy ? "on" : "off"), text: "Proxy " + (info.proxy ? "on" : "off") }));
    st.append(h("span", { class: "chip " + (info.log ? "on" : "off"), text: "Log " + (info.log ? "on" : "off") }));
    if (info.dropped) st.append(h("span", { class: "chip bad", text: count(info.dropped) + " dropped" }));
  }
  const b = $("banner");
  b.textContent = "";
  b.className = "banner";
  b.hidden = true;
  if (!info) return;
  if (info.error) {
    b.hidden = false;
    b.classList.add("bad");
    b.append("The log stopped: " + info.error + ". Turn it off and on again: ", h("code", { text: info.cli + " proxy log off && " + info.cli + " proxy log on" }));
  } else if (!info.proxy) {
    b.hidden = false;
    b.append("The proxy is off, so nothing is recorded. Turn it on in Settings → Proxy, or run ", h("code", { text: info.cli + " proxy on" }), ".");
  } else if (!info.log) {
    b.hidden = false;
    b.append("The log is off. Turn it on in Settings → Proxy, or run ", h("code", { text: info.cli + " proxy log on" }), ".");
  }
}

function drawToolbar(vis) {
  $("inv").classList.toggle("on", S.inv);
  const n = Object.values(S.fM).filter(Boolean).length + Object.values(S.fS).filter(Boolean).length +
    Object.values(S.fMode).filter(Boolean).length + Object.values(S.fHost).filter(Boolean).length;
  const badge = $("filter-count");
  badge.hidden = !n;
  badge.textContent = String(n);
  $("filters-btn").classList.toggle("active", S.pop === "filters" || n > 0);
  $("group-btn").classList.toggle("active", S.pop === "group" || S.group !== "none");
  $("cols-btn").classList.toggle("active", S.pop === "cols");
  $("menu-btn").classList.toggle("active", S.pop === "menu");
  $("group-label").textContent = (GROUPS.find((g) => g[0] === S.group) || GROUPS[0])[1];
  $("live-dot").className = "dot" + (S.live ? " live" : "");
  $("live-label").textContent = S.live ? "Live" : held.length ? "Paused, " + count(held.length) + " new" : "Paused";
  const span = timeSpan();
  const range = rangeTimes(span);
  const rb = $("range");
  rb.hidden = !range;
  if (range) rb.textContent = clock(range[0]) + " to " + clock(range[1]);
  $("count").textContent = count(vis.length) + " of " + count(rows.length);
}

const TYPES = [
  ["all", "All"], ["fetch", "Fetch/XHR"], ["doc", "Doc"], ["js", "JS"], ["css", "CSS"], ["img", "Img"],
  ["media", "Media"], ["font", "Font"], ["ws", "WS"], ["tunnel", "Tunnel"], ["err", "Errors"],
];
const ALWAYS = new Set(["all", "fetch", "doc", "js", "img", "ws", "err"]);

function drawTypes() {
  const box = $("types");
  box.textContent = "";
  for (const [k, label] of TYPES) {
    const n = rows.filter((r) => matchType(r, k)).length;
    if (!ALWAYS.has(k) && !n && S.type !== k) continue;
    box.append(h("button", { class: "type" + (S.type === k ? " on" : ""), type: "button", onClick: () => { S.type = k; render(); } }, label, h("span", { text: count(n) })));
  }
}

// The timeline: one bar per request in lanes, time ticks, the picked
// request, and a dragged time range. Drawn on a canvas.
let lanes = new Map();
function assignLanes(span) {
  lanes = new Map();
  const ends = [];
  const minW = span.len * 0.01;
  for (const r of [...rows].sort((a, b) => a.start - b.start)) {
    const end = Math.max(endOf(r), r.start + minW);
    let l = ends.findIndex((e) => e < r.start);
    if (l < 0) {
      l = ends.length;
      ends.push(0);
    }
    ends[l] = end;
    lanes.set(r.key, Math.min(l, 14));
  }
}

function cssVar(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function niceStep(len, px) {
  const steps = [100, 250, 500, 1000, 2000, 5000, 10000, 15000, 30000, 60000, 120000, 300000, 600000, 900000, 1800000, 3600000, 7200000, 21600000];
  const want = len / Math.max(2, px / 110);
  return steps.find((s) => s >= want) || steps[steps.length - 1];
}

function relLabel(ms) {
  if (ms === 0) return "0";
  if (ms < 1000) return ms + " ms";
  if (ms < 60000) return ms / 1000 + " s";
  if (ms < 3600000) return ms / 60000 + " min";
  return ms / 3600000 + " h";
}

function drawTimeline(span, vis) {
  const c = $("tl-canvas");
  const w = c.clientWidth;
  const hgt = c.clientHeight;
  if (!w || !hgt) return;
  const dpr = window.devicePixelRatio || 1;
  if (c.width !== Math.round(w * dpr) || c.height !== Math.round(hgt * dpr)) {
    c.width = Math.round(w * dpr);
    c.height = Math.round(hgt * dpr);
  }
  const g = c.getContext("2d");
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, w, hgt);
  const x = (t) => ((t - span.t0) / span.len) * w;
  const step = niceStep(span.len, w);
  g.font = "11px " + cssVar("--mono");
  g.textBaseline = "top";
  for (let t = 0; t <= span.len; t += step) {
    const px = Math.round(x(span.t0 + t)) + 0.5;
    g.fillStyle = cssVar("--line-soft");
    g.fillRect(px, 0, 1, hgt);
    g.fillStyle = cssVar("--faint");
    g.fillText(relLabel(t), px + 5, 5);
  }
  assignLanes(span);
  const shown = new Set(vis.map((r) => r.key));
  const teal = cssVar("--teal");
  const errC = "#d9534f";
  for (const r of rows) {
    const lane = lanes.get(r.key) || 0;
    const left = x(r.start);
    const width = Math.max(2, x(endOf(r)) - left);
    const top = 26 + lane * 5;
    g.globalAlpha = shown.has(r.key) ? (r.key === S.sel ? 1 : 0.9) : 0.18;
    if (r.type === "ws") {
      g.fillStyle = teal;
      g.fillRect(left, top, width, 3);
    } else {
      const waitW = r.ms ? width * Math.min(1, r.wait / r.ms) : width;
      g.fillStyle = isError(r) ? errC : "#2e9e57";
      g.fillRect(left, top, Math.max(1, waitW), 3);
      g.fillStyle = isError(r) ? "#b42318" : "#2b5fa8";
      g.fillRect(left + waitW, top, Math.max(0, width - waitW), 3);
    }
  }
  g.globalAlpha = 1;
  const sel = S.sel && byKey.get(S.sel);
  if (sel) {
    g.fillStyle = teal;
    g.fillRect(Math.round(x(sel.start)), 22, 1, hgt - 22);
  }
  const range = rangeTimes(span);
  if (range) {
    const a = x(range[0]);
    const b = x(range[1]);
    g.fillStyle = cssVar("--page");
    g.globalAlpha = 0.8;
    g.fillRect(0, 0, a, hgt);
    g.fillRect(b, 0, w - b, hgt);
    g.globalAlpha = 1;
    g.fillStyle = teal;
    g.fillRect(a, 0, 1, hgt);
    g.fillRect(b, 0, 1, hgt);
  }
}

function tlDown(ev) {
  const c = $("tl-canvas");
  const rect = c.getBoundingClientRect();
  const p = (cx) => Math.max(0, Math.min(1, (cx - rect.left) / rect.width));
  const a = p(ev.clientX);
  S.range = [a, a];
  render();
  const move = (e) => {
    const b = p(e.clientX);
    S.range = [Math.min(a, b), Math.max(a, b)];
    render();
  };
  const up = (e) => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    const b = p(e.clientX);
    S.range = Math.abs(b - a) < 0.005 ? null : [Math.min(a, b), Math.max(a, b)];
    render();
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
}

const GROUP_KEYS = {
  host: (r) => r.host || "(no host)",
  type: (r) => r.type,
  status: (r) => statusGroup(r),
  mode: (r) => r.mode || "(none)",
  method: (r) => r.method,
};
const GROUPS = [["none", "None"], ["host", "Host"], ["type", "Type"], ["status", "Status class"], ["method", "Method"], ["mode", "Mode"]];

function drawTable(span, vis, sel) {
  const cols = {};
  for (const [k] of COLS) cols[k] = userOn(k) && !(sel && (k === "mode" || k === "bar"));
  const grid = COLS.filter(([k]) => cols[k]).map((c) => c[2]).join(" ");
  const scroll = $("scroll");
  scroll.style.setProperty("--cols", grid);
  scroll.classList.toggle("dense", S.density === "compact");

  const head = $("thead");
  head.textContent = "";
  for (const [k, label] of COLS) {
    if (!cols[k]) continue;
    head.append(h("div", { class: k === "ms" || k === "size" ? "r" : "", text: label }));
  }

  const range = rangeTimes(span);
  const v0 = range ? range[0] : span.t0;
  const v1 = range ? range[1] : span.t1;
  const body = $("rows");
  body.textContent = "";
  order = [];
  let drawnRows = 0;
  const frag = document.createDocumentFragment();
  const addRow = (r, noHost) => {
    order.push(r.key);
    if (drawnRows >= MAX_DRAWN) return;
    drawnRows++;
    frag.append(rowEl(r, cols, v0, v1, noHost, sel));
  };
  if (S.group === "none") {
    for (const r of vis) addRow(r, false);
  } else {
    const groups = new Map();
    for (const r of vis) {
      const k = GROUP_KEYS[S.group](r);
      if (!groups.has(k)) groups.set(k, []);
      groups.get(k).push(r);
    }
    for (const [k, list] of groups) {
      const closed = !!S.collapsed[k];
      const errs = list.filter(isError).length;
      const bytes = list.reduce((a, r) => a + Math.max(0, r.size), 0);
      frag.append(h("div", { class: "group", onClick: () => { S.collapsed[k] = !S.collapsed[k]; render(); } },
        h("div", { class: "caret-g", text: closed ? "▸" : "▾" }),
        h("div", { class: "label", text: k }),
        h("div", { class: "n", text: count(list.length) }),
        h("div", { class: "grow" }),
        errs ? h("div", { class: "errs", text: errs + " failed" }) : null,
        h("div", { class: "totals", text: fmtSize(bytes) })));
      if (!closed) for (const r of list) addRow(r, S.group === "host");
    }
  }
  body.append(frag);
  if (order.length > MAX_DRAWN) {
    body.append(h("div", { class: "empty", text: "Showing the first " + count(MAX_DRAWN) + " of " + count(order.length) + " requests. Narrow the filter to see the rest." }));
  }

  const empty = $("empty");
  empty.textContent = "";
  empty.hidden = vis.length > 0;
  if (!rows.length) {
    if (!info) empty.append("Loading…");
    else if (!info.files.length) empty.append("No requests yet. The first request through the proxy starts a file.");
    else empty.append("No requests in this file yet.");
  } else {
    empty.append("No requests match this filter.");
  }
  $("more-wrap").hidden = before === null || rows.length >= MAX_ROWS || !rows.length;
  $("more").disabled = loadingMore;
}

function rowEl(r, cols, v0, v1, noHost, sel) {
  const ws = r.type === "ws";
  const cells = [];
  if (cols.time) cells.push(h("div", { class: "c-time", text: clock(r.start) }));
  if (cols.method) cells.push(h("div", { class: "c-method m-" + r.method, text: r.method }));
  if (cols.status) cells.push(h("div", null, h("span", { class: "pill " + statusClass(r.status), text: r.status ? String(r.status) : "…" })));
  if (cols.type) cells.push(h("div", null, h("span", { class: "tpill" + (ws ? " ws" : ""), text: r.type })));
  cells.push(h("div", { class: "c-req", title: r.url },
    noHost ? null : h("span", { class: "host", text: r.host }),
    h("span", { class: "path", text: r.mode === "tunnel" ? "" : r.path })));
  if (cols.mode) cells.push(h("div", { class: "muted", text: r.mode }));
  if (cols.ms) {
    let text;
    let cls = "c-ms r";
    if (r.open && ws) { text = "live"; cls += " live"; }
    else if (r.open) { text = "receiving"; cls += " live"; }
    else if (ws) text = fmtMs(r.receive || r.ms);
    else { text = fmtMs(r.ms); if (r.ms > 1000) cls += " slow"; }
    cells.push(h("div", { class: cls, text }));
  }
  if (cols.bar) {
    const left = Math.max(0, Math.min(98, ((r.start - v0) / (v1 - v0)) * 100));
    const width = Math.max(1.5, Math.min(100 - left, ((endOf(r) - Math.max(r.start, v0)) / (v1 - v0)) * 100));
    cells.push(h("div", { class: "wf" }, h("div", { class: ws ? "ws" : isError(r) ? "err" : "", style: { left: left + "%", width: width + "%" } })));
  }
  if (cols.size) {
    const text = ws ? (r.messages !== null ? count(r.messages) + " msg" : "") : fmtSize(r.size);
    cells.push(h("div", { class: "c-size r", text }));
  }
  const el = h("div", {
    class: "row" + (sel && sel.key === r.key ? " picked" : "") + (fresh.has(r.key) ? " fresh" : ""),
    onClick: () => select(r.key),
    onContextmenu: (ev) => {
      ev.preventDefault();
      S.sel = r.key;
      S.ctx = { x: ev.clientX, y: ev.clientY, key: r.key };
      render();
    },
  }, cells);
  return el;
}

function select(key) {
  if (S.sel !== key) {
    S.sel = key;
    S.frame = null;
  }
  render();
}

function drawLayout(sel) {
  const layout = $("layout");
  layout.classList.toggle("with-detail", !!sel && !S.max);
  layout.classList.toggle("max", !!sel && S.max);
  const w = S.detailW || Math.round(Math.max(400, Math.min(window.innerWidth - 360, window.innerWidth * 0.44)));
  layout.style.setProperty("--detail-w", w + "px");
  $("detail").hidden = !sel;
}

// ---- the detail pane

function tabsFor(r, e) {
  if (r.type === "ws") return [["messages", "Messages", r.messages], ["headers", "Headers"], ["timing", "Timing"]];
  const tabs = [["preview", "Preview"], ["response", "Response"]];
  const post = e && e.request.postData;
  if (post) tabs.push(["payload", "Payload"]);
  tabs.push(["headers", "Headers"], ["timing", "Timing"]);
  return tabs;
}

function drawDetail(r) {
  if (!r) {
    drawn = null;
    return;
  }
  const e = cachedEntry(r) || r.e;
  const tabs = tabsFor(r, e);
  const tab = tabs.some((t) => t[0] === S.tab) ? S.tab : tabs[0][0];
  const needWhole = tab !== "headers" && tab !== "timing" && r.at !== undefined && !cachedEntry(r);
  const live = r.id !== undefined ? liveMsgs.get(r.id) : null;
  const key = [r.key, r.status, r.open, tab, S.ff, S.frame, live ? live.length : 0, !!cachedEntry(r), S.max].join("|");

  // The head changes with the row (status, open), the body only with this key.
  drawDetailHead(r, e, tabs, tab);
  if (drawn === key) return;
  drawn = key;
  const body = $("dbody");
  body.textContent = "";
  body.className = "dbody";
  if (needWhole) {
    body.append(h("div", { class: "void", text: "Loading…" }));
    wholeEntry(r).then(() => { drawn = null; render(); }).catch((err) => {
      body.textContent = "";
      body.append(h("div", { class: "void", text: "The entry could not be read: " + err.message }));
    });
    return;
  }
  if (tab === "messages") drawMessages(body, r, e, live);
  else if (tab === "preview") drawPreview(body, r, e);
  else if (tab === "response") drawResponse(body, r, e);
  else if (tab === "payload") drawPayload(body, e);
  else if (tab === "headers") drawHeaders(body, r, e);
  else drawTiming(body, r);
}

function drawDetailHead(r, e, tabs, tab) {
  const top = $("dtop");
  top.textContent = "";
  top.append(h("span", { class: "mono c-method m-" + r.method, text: r.method }));
  const st = r.status ? r.status + " " + (e.response.statusText || STATUS_TEXT[r.status] || "") : "waiting";
  top.append(h("span", { class: "mono pill " + statusClass(r.status), text: st.trim() }));
  if (r.type === "ws") {
    top.append(h("span", { class: "ws-state" }, h("span", { class: "dot" + (r.open ? " live" : "") }), r.open ? "Open" : "Closed"));
  }
  let summary;
  if (r.type === "ws") summary = (r.messages !== null ? count(r.messages) + " messages" : "") + (r.open ? "" : ", open " + fmtMs(r.receive));
  else if (r.mode === "tunnel") summary = fmtMs(r.ms) + ", " + fmtSize(e._bytesIn || 0) + " up, " + fmtSize(e._bytesOut || 0) + " down";
  else summary = (r.open ? "receiving" : fmtMs(r.ms)) + ", " + (r.size > 0 ? fmtSize(r.size) : "no body");
  top.append(h("span", { class: "sum", text: summary }));
  top.append(h("button", { class: "btn small" + (S.max ? " active" : ""), type: "button", title: S.max ? "Show the request list" : "Expand the details to full width", text: S.max ? "Restore" : "Expand", onClick: () => { S.max = !S.max; render(); } }));
  top.append(h("button", { class: "btn x", type: "button", title: "Close (Esc)", text: "×", onClick: () => { S.sel = null; render(); } }));

  const url = $("durl");
  url.textContent = "";
  url.append(h("span", { class: "scheme", text: r.scheme }), h("span", { text: r.host }), h("span", { class: "path", text: r.mode === "tunnel" ? "" : r.path }));

  const act = $("dactions");
  act.textContent = "";
  act.append(
    h("button", { class: "btn small", type: "button", text: "Copy as cURL", onClick: () => copyCurl(r) }),
    h("button", { class: "btn small", type: "button", text: "Copy as JSON", onClick: () => copyJson(r) }),
    h("button", { class: "btn small", type: "button", text: "Copy URL", onClick: () => copy("URL copied", r.url) }),
  );

  const tb = $("dtabs");
  tb.textContent = "";
  for (const [k, label, n] of tabs) {
    tb.append(h("button", { class: "dtab" + (tab === k ? " on" : ""), type: "button", onClick: () => { S.tab = k; render(); } },
      label, n !== undefined && n !== null ? h("span", { text: count(n) }) : null));
  }
}

/** A titled block. `meta` is text, or an element that may change later. */
function section(title, meta, ...content) {
  const m = meta instanceof Node ? meta : meta ? h("span", { text: meta }) : null;
  return h("div", { class: "sec" }, h("div", { class: "sec-head" }, h("b", { text: title }), m), ...content);
}

/** JSON as indented, colored lines. */
function jsonLines(value) {
  const out = [];
  const line = (ind, key, parts, comma) => {
    if (out.length >= MAX_LINES) return;
    const el = h("div", { style: { paddingLeft: 12 + ind * 16 + "px" } });
    if (key !== null) el.append(h("span", { class: "j-k", text: JSON.stringify(key) + ": " }));
    for (const [cls, text] of parts) el.append(h("span", { class: cls, text }));
    if (comma) el.append(h("span", { class: "j-p", text: "," }));
    out.push(el);
  };
  const walk = (v, ind, key, comma) => {
    if (out.length >= MAX_LINES) return;
    if (v && typeof v === "object") {
      const arr = Array.isArray(v);
      const ents = arr ? v.map((x) => [null, x]) : Object.entries(v);
      if (!ents.length) return line(ind, key, [["j-p", arr ? "[]" : "{}"]], comma);
      line(ind, key, [["j-p", arr ? "[" : "{"]], false);
      ents.forEach(([k, x], i) => walk(x, ind + 1, k, i < ents.length - 1));
      line(ind, null, [["j-p", arr ? "]" : "}"]], comma);
    } else {
      const cls = typeof v === "string" ? "j-s" : typeof v === "number" ? "j-n" : "j-l";
      line(ind, key, [[cls, typeof v === "string" ? JSON.stringify(v) : String(v)]], comma);
    }
  };
  walk(value, 0, null, false);
  if (out.length >= MAX_LINES) out.push(h("div", { class: "j-p", style: { paddingLeft: "12px" }, text: "… cut at " + count(MAX_LINES) + " lines. Use Response or Copy as JSON for all of it." }));
  return h("div", { class: "card lines" }, out);
}

function textLines(text) {
  const lines = text.split("\n");
  const box = h("div", { class: "card lines" });
  for (const l of lines.slice(0, MAX_LINES)) box.append(h("div", { style: { paddingLeft: "12px" }, text: l || " " }));
  if (lines.length > MAX_LINES) box.append(h("div", { class: "j-p", style: { paddingLeft: "12px" }, text: "… " + count(lines.length - MAX_LINES) + " more lines in Response" }));
  return box;
}

function bytesOf(content) {
  if (content.encoding === "base64") {
    try {
      const bin = atob(content.text);
      const out = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
      return out;
    } catch (e) {
      return new Uint8Array();
    }
  }
  return new TextEncoder().encode(content.text);
}

function hexDump(bytes, limit) {
  const box = h("div", { class: "card hex" });
  const n = Math.min(bytes.length, limit);
  for (let i = 0; i < n; i += 16) {
    const chunk = Array.from(bytes.slice(i, Math.min(i + 16, n)));
    box.append(h("div", null,
      h("span", { class: "off", text: i.toString(16).padStart(8, "0") }),
      h("span", { text: chunk.map((b) => b.toString(16).padStart(2, "0")).join(" ").padEnd(47, " ") }),
      h("span", { class: "ascii", text: chunk.map((b) => (b > 31 && b < 127 ? String.fromCharCode(b) : ".")).join("") })));
  }
  return box;
}

function parseJson(text) {
  const t = (text || "").trim();
  if (!t || (t[0] !== "{" && t[0] !== "[")) return undefined;
  try {
    return JSON.parse(t);
  } catch (e) {
    return undefined;
  }
}

function noBodyReason(r, e) {
  if (r.mode === "tunnel") return "The proxy did not read this connection: it was a tunnel. Add the host to the inspect list to read its requests.";
  if (r.open) return "The response is still on its way. Its body shows when it ends.";
  if (r.status === 304) return "304 Not Modified: the browser used its cached copy.";
  if (r.status >= 300 && r.status < 400) return "A redirect with no body.";
  if (e.response.bodySize === 0 || r.method === "HEAD" || r.status === 204) return "This response has no body.";
  return "The body was not recorded. Entries written before bodies were logged have none.";
}

function bodyMeta(content, mime) {
  const parts = [];
  if (mime) parts.push(mime);
  if (content.size !== undefined) parts.push(fmtSize(content.size));
  return parts.join(", ");
}

function drawPreview(body, r, e) {
  const c = e.response.content || {};
  if (!c.text) {
    body.append(h("div", { class: "void", text: noBodyReason(r, e) }));
    return;
  }
  const mime = essence(c.mimeType);
  const meta = bodyMeta(c, mime);
  const notes = [];
  if (c._truncated) notes.push(h("div", { class: "warnline", text: "Only the first part of this body was kept (1 MB on the wire, 4 MB decoded)." }));
  if (c._decodeError) notes.push(h("div", { class: "warnline", text: c._decodeError }));
  if (mime.startsWith("image/")) {
    const src = c.encoding === "base64" ? "data:" + mime + ";base64," + c.text : "data:" + mime + ";charset=utf-8," + encodeURIComponent(c.text);
    const img = h("img", { alt: "", src });
    const metaEl = h("span", { text: meta });
    img.addEventListener("load", () => { metaEl.textContent = meta + ", " + img.naturalWidth + " × " + img.naturalHeight; });
    body.append(section("Image", metaEl, h("div", { class: "imgbox" }, img), notes));
    return;
  }
  if (c.encoding === "base64") {
    const bytes = bytesOf(c);
    body.append(section("Binary", meta + ", first " + fmtSize(Math.min(bytes.length, 512)), hexDump(bytes, 512), notes));
    return;
  }
  const parsed = mime.includes("json") || mime === "" || mime.startsWith("text/") ? parseJson(c.text) : undefined;
  if (parsed !== undefined) {
    body.append(section("JSON", meta, jsonLines(parsed), notes));
    return;
  }
  if (mime === "text/html") {
    const frame = h("iframe", { class: "frame-html", sandbox: "", title: "HTML preview" });
    frame.srcdoc = c.text;
    body.append(section("HTML", meta + ", scripts off", frame, notes));
    return;
  }
  if (mime === "text/event-stream") {
    body.append(section("Event stream", meta, textLines(c.text), notes));
    return;
  }
  body.append(section(mime.includes("javascript") ? "JavaScript" : "Text", meta, textLines(c.text), notes));
}

function drawResponse(body, r, e) {
  const c = e.response.content || {};
  if (!c.text) {
    body.append(h("div", { class: "void", text: noBodyReason(r, e) }));
    return;
  }
  if (c.encoding === "base64") {
    body.append(section("Base64", bodyMeta(c, essence(c.mimeType)), h("div", { class: "card raw", text: c.text })));
    return;
  }
  body.append(h("div", { class: "card raw", text: c.text }));
}

function drawPayload(body, e) {
  const p = e.request.postData;
  if (!p || !p.text) {
    body.append(h("div", { class: "void", text: "No request body was recorded." }));
    return;
  }
  const mime = essence(p.mimeType);
  const meta = (mime ? mime + ", " : "") + fmtSize(e.request.bodySize);
  const notes = p._truncated ? h("div", { class: "warnline", text: "Only the first part of this body was kept." }) : null;
  if (p.encoding === "base64") {
    body.append(section("Binary", meta, hexDump(bytesOf(p), 512), notes));
    return;
  }
  if (mime === "application/x-www-form-urlencoded") {
    const pairs = [...new URLSearchParams(p.text)].map(([k, v]) => [k, v]);
    body.append(section("Form data", meta, kv(pairs), notes));
    return;
  }
  const parsed = parseJson(p.text);
  body.append(section(parsed !== undefined ? "JSON" : "Text", meta, parsed !== undefined ? jsonLines(parsed) : textLines(p.text), notes));
}

function kv(pairs) {
  const box = h("div", { class: "card kv" });
  if (!pairs.length) box.append(h("div", null, h("span", { class: "none", text: "none" })));
  for (const [k, v] of pairs) box.append(h("div", null, h("span", { class: "k", text: k }), h("span", { class: "v", text: v })));
  return box;
}

function drawHeaders(body, r, e) {
  const general = [
    ["Request URL", r.url],
    ["Method", r.method],
    ["Status", r.status ? r.status + " " + (e.response.statusText || STATUS_TEXT[r.status] || "") : "waiting"],
    ["Protocol", (e.request.httpVersion || "") + (e.response.httpVersion && e.response.httpVersion !== e.request.httpVersion ? " → " + e.response.httpVersion : "")],
    ["Started", new Date(r.start).toLocaleString() + " (" + clock(r.start) + ")"],
  ];
  body.append(section("General", null, kv(general)));
  const lr = [];
  for (const k of ["_mode", "_route", "_scripts", "_scriptError", "_bodyError", "_bytesIn", "_bytesOut", "_webSocketDropped", "_id"]) {
    if (e[k] !== undefined) lr.push([k, typeof e[k] === "string" ? e[k] : JSON.stringify(e[k])]);
  }
  const c = e.response.content || {};
  if (c._decodeError) lr.push(["_decodeError", c._decodeError]);
  body.append(section("LocalRouter", null, kv(lr)));
  const q = e.request.queryString || [];
  if (q.length) body.append(section("Query", count(q.length), kv(q.map((x) => [x.name, x.value]))));
  const rq = e.request.headers || [];
  body.append(section("Request headers", count(rq.length), kv(rq.map((x) => [x.name, x.value]))));
  const rs = e.response.headers || [];
  body.append(section("Response headers", count(rs.length), kv(rs.map((x) => [x.name, x.value]))));
}

function drawTiming(body, r) {
  const ws = r.type === "ws";
  const parts = ws
    ? [["Handshake (until 101)", r.wait, "#8fb3e0"], [r.open ? "Open so far" : "Open", r.open ? Date.now() - r.start - r.wait : r.receive, cssVar("--teal")]]
    : r.mode === "tunnel"
      ? [["Tunnel open", r.ms, "#c98a3d"]]
      : [["Waiting (until response headers)", r.wait, "#8fb3e0"], [r.open ? "Receiving so far" : "Receiving the body", r.open ? Date.now() - r.start - r.wait : r.receive, "#2b5fa8"]];
  const total = parts.reduce((a, p) => a + Math.max(0, p[1]), 0) || 1;
  const bar = h("div", { class: "tbar" });
  for (const [, ms, color] of parts) bar.append(h("div", { style: { width: (Math.max(0, ms) / total) * 100 + "%", background: color } }));
  const list = h("div", { class: "card kv" });
  for (const [label, ms, color] of parts) {
    list.append(h("div", { class: "trow" }, h("span", { class: "sw", style: { background: color } }), h("span", { text: label }), h("span", { class: "ms", text: fmtMs(Math.max(0, ms)) })));
  }
  list.append(h("div", { class: "trow" }, h("span"), h("b", { text: "Total" }), h("span", { class: "ms", text: fmtMs(total) })));
  body.append(section("Timing", "Started " + clock(r.start), bar, list,
    h("div", { class: "note", text: "The proxy measures from the moment it got the request. DNS, connect and TLS to the server are inside Waiting." })));
}

function drawMessages(body, r, e, live) {
  body.className = "dbody frames";
  const all = (e._webSocketMessages || []).concat(e._webSocketMessages ? [] : live || []);
  const sent = all.filter((m) => m.type === "send").length;
  const bar = h("div", { class: "fbar" });
  const seg = h("div", { class: "seg" });
  for (const [k, label] of [["all", "All"], ["sent", "Sent"], ["recv", "Received"]]) {
    seg.append(h("button", { class: S.ff === k ? "on" : "", type: "button", text: label, onClick: () => { S.ff = k; render(); } }));
  }
  bar.append(seg, h("div", { class: "grow" }), h("span", { class: "count mono", text: "↑ " + count(sent) + "  ↓ " + count(all.length - sent) + (e._webSocketDropped ? "  (" + count(e._webSocketDropped) + " not kept)" : "") }));
  const list = h("div", { class: "flist" });
  const shown = all.map((m, i) => ({ m, i })).filter(({ m }) => S.ff === "all" || (S.ff === "sent" ? m.type === "send" : m.type === "receive"));
  const pickedIdx = S.frame !== null && all[S.frame] ? S.frame : all.length ? all.length - 1 : null;
  for (const { m, i } of shown.slice(-MAX_DRAWN)) {
    const up = m.type === "send";
    const ctl = m.opcode >= 8;
    const label = m.opcode === 8 ? "close " + m.data : m.opcode === 9 ? "ping " + m.data : m.opcode === 10 ? "pong " + m.data : m.opcode === 2 ? "binary, " + fmtSize(m._size) : m.data;
    list.append(h("div", { class: "frow" + (up ? " up" : "") + (i === pickedIdx ? " picked" : ""), onClick: () => { S.frame = i; render(); } },
      h("span", { class: "dir " + (up ? "up" : "down"), text: up ? "↑" : "↓" }),
      h("span", { class: "txt" + (ctl ? " ctl" : ""), text: label }),
      h("span", { class: "r", text: fmtSize(m._size !== undefined ? m._size : m.data.length) }),
      h("span", { class: "r", text: clock(m.time * 1000).slice(3) })));
  }
  if (!all.length) list.append(h("div", { class: "empty", text: r.open ? "No messages yet." : "No messages were recorded." }));
  const detail = h("div", { class: "fdetail" });
  if (pickedIdx !== null) {
    const m = all[pickedIdx];
    const kind = m.opcode === 8 ? "close" : m.opcode === 9 ? "ping" : m.opcode === 10 ? "pong" : m.opcode === 2 ? "binary" : "text";
    const head = h("div", { class: "sec-head" }, h("b", { text: (m.type === "send" ? "Sent" : "Received") + ", " + kind }), h("span", { text: clock(m.time * 1000) + ", " + fmtSize(m._size) + (m._truncated ? ", cut at 64 KB" : "") }));
    let content;
    if (m.opcode === 2) content = hexDump(bytesOf({ text: m.data, encoding: "base64" }), 1024);
    else {
      const sio = /^(\d+)([[{].*)$/s.exec(m.data);
      const parsed = parseJson(sio ? sio[2] : m.data);
      if (sio && parsed !== undefined) head.append(h("span", { text: "socket.io packet " + sio[1] }));
      content = parsed !== undefined ? jsonLines(parsed) : textLines(m.data);
    }
    detail.append(head, content);
    requestAnimationFrame(() => {
      const picked = list.querySelector(".picked");
      if (picked && S.frame === null) picked.scrollIntoView({ block: "nearest" });
    });
  }
  body.append(bar, list, detail);
}

// ---- pop-overs and menus

function closePops() {
  S.pop = null;
  S.ctx = null;
  render();
}

function scrim(z) {
  const el = h("div", { class: "scrim", onClick: closePops, onContextmenu: (ev) => { ev.preventDefault(); closePops(); } });
  if (z) el.style.zIndex = z;
  return el;
}

function drawPops(vis) {
  for (const id of ["menu", "filters", "group", "cols"]) {
    const pop = $(id);
    pop.hidden = S.pop !== id;
    pop.textContent = "";
  }
  document.querySelectorAll("body > .scrim").forEach((s) => s.remove());
  if (S.pop) document.body.append(scrim());
  if (S.pop === "menu") drawMenu($("menu"));
  if (S.pop === "filters") drawFilters($("filters"), vis);
  if (S.pop === "group") drawGroup($("group"));
  if (S.pop === "cols") drawCols($("cols"));
  drawCtx();
}

function drawMenu(m) {
  m.append(h("div", { class: "pop-title", text: "View" }));
  const seg = h("div", { class: "seg" });
  for (const [k, label] of [["compact", "Compact"], ["comfortable", "Comfortable"]]) {
    seg.append(h("button", { class: S.density === k ? "on" : "", type: "button", text: label, onClick: () => { S.density = k; save(); render(); } }));
  }
  m.append(h("div", { class: "row-line" }, h("span", { text: "Row density" }), seg));
  m.append(h("button", { class: "item check-item" + (S.tl ? " on" : ""), type: "button", onClick: () => { S.tl = !S.tl; save(); render(); } }, h("span", { class: "box" + (S.tl ? " on" : "") }), "Timeline"));
  const wf = !S.hide.bar;
  m.append(h("button", { class: "item", type: "button", onClick: () => { S.hide.bar = wf; save(); render(); } }, h("span", { class: "box" + (wf ? " on" : "") }), "Waterfall column"));
  m.append(h("div", { class: "pop-sep" }), h("div", { class: "pop-title", text: "Log files" }));
  if (!info || !info.files.length) m.append(h("div", { class: "note", text: "No file yet. The first request through the proxy makes one." }));
  for (const f of info ? info.files : []) {
    const meta = (f.entries !== null && f.entries !== undefined ? count(f.entries) + " requests, " : "") + fmtSize(f.size);
    m.append(h("div", { class: "file-item" + (f.name === file ? " picked" : ""), onClick: () => { S.pop = null; pick(f.name); } },
      h("div", { class: "file-name" }, h("span", { text: f.name }), f.current ? h("span", { class: "tag-live", text: "Live" }) : null),
      h("div", { class: "file-meta", text: meta })));
  }
  if (file) {
    const cur = info && info.files.find((f) => f.name === file);
    m.append(h("div", { class: "pop-sep" }));
    m.append(h("a", { class: "item", href: "files/" + encodeURIComponent(file) + "?download=1", onClick: () => setTimeout(closePops, 0) },
      h("span", { text: "Download HAR" }), h("span", { class: "hint", text: cur ? fmtSize(cur.size) : "" })));
    m.append(h("div", { class: "note", text: "Chrome: DevTools → Network → Import HAR file, or drag the file onto the Network panel." }));
  }
  if (info) {
    m.append(h("div", { class: "pop-sep" }));
    m.append(h("div", { class: "facts" },
      h("span", { text: "Folder" }), h("span", { class: "mono", title: info.folder, text: info.folder }),
      h("span", { text: "Rotation" }), h("span", { text: info.file_mb + " MB or " + count(info.file_requests) + " requests, keep " + info.keep_files + " files" }),
      h("span", { text: "Bodies" }), h("span", { text: "First 1 MB of each, headers as sent" })));
  }
}

function chipSet(name, values, fn) {
  const counts = new Map();
  for (const r of rows) {
    const k = fn(r);
    counts.set(k, (counts.get(k) || 0) + 1);
  }
  return [...counts.entries()].map(([k, n]) => ({ k, n, set: name }));
}

function drawFilters(p, vis) {
  const sections = [
    ["Method", chipSet("fM", null, (r) => r.method)],
    ["Status", chipSet("fS", null, statusGroup).sort((a, b) => a.k.localeCompare(b.k))],
    ["Mode", chipSet("fMode", null, (r) => r.mode)],
    ["Host", chipSet("fHost", null, (r) => r.host).sort((a, b) => b.n - a.n).slice(0, 12)],
  ];
  for (const [title, chips] of sections) {
    if (!chips.length) continue;
    const box = h("div", { class: "chips" });
    for (const c of chips) {
      const on = !!S[c.set][c.k];
      box.append(h("button", { class: "fchip" + (on ? " on" : ""), type: "button", title: c.k, onClick: () => { S[c.set][c.k] = !on; render(); } }, c.k || "(none)", h("span", { text: count(c.n) })));
    }
    p.append(h("div", { class: "sec" }, h("div", { class: "chips-title", text: title }), box));
  }
  p.append(h("div", { class: "pop-foot" }, h("span", { class: "count", text: count(vis.length) + " of " + count(rows.length) + " shown" }),
    h("button", { class: "link-btn", type: "button", text: "Clear all", onClick: () => { S.fM = {}; S.fS = {}; S.fMode = {}; S.fHost = {}; render(); } })));
}

function drawGroup(p) {
  for (const [k, label] of GROUPS) {
    p.append(h("button", { class: "item" + (S.group === k ? " picked" : ""), type: "button", onClick: () => { S.group = k; S.collapsed = {}; S.pop = null; save(); render(); } },
      h("span", { text: label }), S.group === k ? h("span", { class: "hint", text: "✓" }) : null));
  }
  if (S.group !== "none") {
    const keys = new Set(rows.map(GROUP_KEYS[S.group]));
    p.append(h("div", { class: "pop-sep" }), h("div", { class: "split" },
      h("button", { type: "button", text: "Expand all", onClick: () => { S.collapsed = {}; render(); } }),
      h("button", { type: "button", text: "Collapse all", onClick: () => { S.collapsed = Object.fromEntries([...keys].map((k) => [k, true])); render(); } })));
  }
}

function drawCols(p) {
  const sel = S.sel && byKey.get(S.sel);
  for (const [k, label] of COLS) {
    const locked = k === "request";
    const on = userOn(k);
    const auto = sel && (k === "mode" || k === "bar");
    p.append(h("button", { class: "item" + (locked ? " locked" : ""), type: "button", onClick: () => { if (locked) return; S.hide[k] = on; save(); render(); } },
      h("span", { class: "box" + (on ? " on" : "") }), h("span", { class: "grow", text: label }),
      h("span", { class: "hint", text: locked ? "always" : auto && on ? "hidden with details" : "" })));
  }
  p.append(h("div", { class: "pop-sep" }), h("button", { class: "link-btn", type: "button", text: "Reset columns", onClick: () => { S.hide = {}; save(); render(); } }));
}

function drawCtx() {
  const box = $("ctx");
  box.textContent = "";
  const r = S.ctx && byKey.get(S.ctx.key);
  box.hidden = !r;
  if (!r) return;
  document.body.append(scrim(40));
  box.style.left = Math.min(S.ctx.x, window.innerWidth - 242) + "px";
  box.style.top = Math.min(S.ctx.y, window.innerHeight - 240) + "px";
  const item = (label, fn) => h("button", { class: "item", type: "button", onClick: () => { S.ctx = null; fn(); render(); } }, label);
  box.append(
    h("div", { class: "ctx-title", text: r.method + " " + r.host + (r.mode === "tunnel" ? "" : r.path) }),
    item("Copy URL", () => copy("URL copied", r.url)),
    item("Copy as cURL", () => copyCurl(r)),
    item("Copy as JSON", () => copyJson(r)),
    h("div", { class: "pop-sep" }),
    item("Show only this host", () => { S.fHost = { [r.host]: true }; }),
    item("Hide this host", () => { S.q = ""; $("q").value = r.host; S.inv = true; S.q = r.host; }),
  );
}

// ---- copy

function toast(text) {
  const t = $("toast");
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { t.hidden = true; }, 1600);
}

async function copy(label, text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch (e) {
    const ta = h("textarea", { style: { position: "fixed", opacity: "0" } });
    ta.value = text;
    document.body.append(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
  toast(label);
}

function shellQuote(s) {
  return "'" + String(s).replace(/'/g, "'\\''") + "'";
}

const SKIP_CURL = new Set(["content-length", "host", "connection", "proxy-connection", "keep-alive", "transfer-encoding"]);

async function copyCurl(r) {
  let e = r.e;
  try {
    e = (await wholeEntry(r)) || e;
  } catch (err) {
    // Without the whole entry the command has no body.
  }
  const parts = ["curl " + shellQuote(r.url)];
  if (r.method !== "GET" && !(r.method === "POST" && e.request.postData)) parts.push("-X " + r.method);
  for (const x of e.request.headers || []) {
    if (x.name.startsWith(":") || SKIP_CURL.has(x.name.toLowerCase())) continue;
    parts.push("-H " + shellQuote(x.name + ": " + x.value));
  }
  const p = e.request.postData;
  if (p && p.text) parts.push(p.encoding === "base64" ? "--data-binary @- # base64 body, see Copy as JSON" : "--data-raw " + shellQuote(p.text));
  if (header(e.request.headers, "accept-encoding")) parts.push("--compressed");
  copy("cURL copied", parts.join(" \\\n  "));
}

async function copyJson(r) {
  let e = r.e;
  try {
    e = (await wholeEntry(r)) || e;
  } catch (err) {
    // Copy the summary.
  }
  copy("JSON copied", JSON.stringify(e, null, 2));
}

// ---- resize, keys, errors

function resizeDown(ev) {
  ev.preventDefault();
  const sx = ev.clientX;
  const sw = $("detail").getBoundingClientRect().width;
  const move = (e) => {
    S.detailW = Math.round(Math.max(360, Math.min(window.innerWidth - 360, sw - (e.clientX - sx))));
    S.max = false;
    render();
  };
  const up = () => {
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    save();
  };
  document.body.style.cursor = "col-resize";
  document.body.style.userSelect = "none";
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
}

function onKey(e) {
  const tag = e.target.tagName;
  if (e.key === "Escape") {
    if (S.pop || S.ctx) return closePops();
    if (tag === "INPUT" && e.target.value) return;
    if (S.sel) {
      S.sel = null;
      render();
    }
    return;
  }
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
  if (e.key === "/" && !e.metaKey && !e.ctrlKey) {
    e.preventDefault();
    $("q").focus();
    return;
  }
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    if (!order.length) return;
    e.preventDefault();
    const i = order.indexOf(S.sel);
    const n = i < 0 ? 0 : Math.max(0, Math.min(order.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)));
    select(order[n]);
    requestAnimationFrame(() => {
      const el = document.querySelector(".row.picked");
      if (el) el.scrollIntoView({ block: "nearest" });
    });
  }
}

function failure(e) {
  const b = $("banner");
  b.hidden = false;
  b.className = "banner bad";
  b.textContent = "The page could not read the log: " + e.message;
}

document.addEventListener("DOMContentLoaded", () => {
  $("q").addEventListener("input", (e) => { S.q = e.target.value; render(); });
  $("inv").addEventListener("click", () => { S.inv = !S.inv; render(); });
  const toggle = (name) => () => { S.pop = S.pop === name ? null : name; S.ctx = null; render(); };
  $("filters-btn").addEventListener("click", toggle("filters"));
  $("group-btn").addEventListener("click", toggle("group"));
  $("cols-btn").addEventListener("click", toggle("cols"));
  $("menu-btn").addEventListener("click", toggle("menu"));
  $("live-btn").addEventListener("click", () => setLive(!S.live));
  $("range").addEventListener("click", () => { S.range = null; render(); });
  $("tl-btn").addEventListener("click", () => { S.tl = !S.tl; save(); render(); });
  $("tl-canvas").addEventListener("mousedown", tlDown);
  $("more").addEventListener("click", () => loadEntries(true).catch(failure));
  $("resize").addEventListener("mousedown", resizeDown);
  $("resize").addEventListener("dblclick", () => { S.detailW = null; S.max = false; save(); render(); });
  $("scroll").addEventListener("scroll", () => {
    const s = $("scroll");
    if (!loadingMore && before !== null && rows.length < MAX_ROWS && s.scrollTop + s.clientHeight > s.scrollHeight - 200) {
      loadEntries(true).catch(failure);
    }
  });
  window.addEventListener("keydown", onKey);
  window.addEventListener("resize", render);
  new ResizeObserver(render).observe($("tl-canvas"));
  // Open rows: their duration and the timeline grow.
  setInterval(() => { if (rows.some((r) => r.open)) render(); }, 1000);
  loadFiles().then(startLive).catch(failure);
  setInterval(() => loadFiles().catch(() => {}), 10000);
});
