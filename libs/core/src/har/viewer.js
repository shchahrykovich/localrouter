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
    const { view, tl, hide, group, detailW } = S;
    localStorage.setItem(STORE_KEY, JSON.stringify({ view, tl, hide, group, detailW }));
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

const DEFAULT_CLIENT = "default"; // the main proxy port: its entries have no _client

/** The proxy client in the page's address: proxy.localhost/<name>, or
 *  router.localhost/proxy-log/<name>. Empty for all clients. */
function clientFromPath() {
  const last = location.pathname.split("/").pop() || "";
  return /^[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?$/.test(last) ? last : "";
}

/** The name of the proxy port that carried an entry. */
function clientOf(e) {
  return e._client || DEFAULT_CLIENT;
}

function clientMatch(e) {
  return !S.client || clientOf(e) === S.client;
}

const saved = load();
const S = {
  client: clientFromPath(), // "" for every proxy port
  q: "",
  inv: false,
  type: "all",
  fM: {}, fS: {}, fMode: {}, fHost: {},
  view: saved.view === "list" ? "list" : "tree",
  group: saved.group || "none",
  collapsed: {},          // list view: closed groups
  tx: {},                 // tree view: node key → open
  sort: null,             // {key, dir}
  hide: saved.hide || {},
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
let rows = [];            // rows of every file, newest first
const byKey = new Map();  // row key → row
// The page shows all files as one list. Older entries are read page by page,
// file by file from the newest: `reading` is where the next page comes from,
// {file, before} (`before` null: from the end of that file), or null once the
// oldest file is read.
let reading = null;
let started = false;      // the first page was asked for
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
    // An entry is known by its place in its file: `_id` starts again at 1
    // when the daemon restarts, so it is unique only for an open request.
    key: !e._open && e._at !== undefined && f ? f + "@" + e._at : "o" + (e._id !== undefined ? e._id : Math.random()),
    id: e._id, at: e._at, file: f, e,
    start: isNaN(start) ? 0 : start,
    method: e.request.method, status,
    url: e.request.url, scheme: u.scheme, host: u.host, path: u.path,
    type, mode: e._mode || "", client: clientOf(e),
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
  const names = new Set(info.files.map((f) => f.name));
  // The log keeps only the newest files: rows of a deleted file go too.
  if (rows.some((x) => x.file && !names.has(x.file))) {
    rows = rows.filter((x) => !x.file || names.has(x.file));
    byKey.clear();
    for (const x of rows) byKey.set(x.key, x);
    if (S.sel && !byKey.has(S.sel)) S.sel = null;
  }
  // Older files are deleted first, so the rest of the walk went with it.
  if (reading && !names.has(reading.file)) reading = null;
  if (!started && info.files.length) {
    started = true;
    reading = { file: info.files[0].name, before: null };
    loadEntries().catch(failure);
  }
  render();
}

/** Where the walk goes after `name`: the end of the next older file. */
function olderThan(name) {
  const i = info ? info.files.findIndex((f) => f.name === name) : -1;
  const next = i >= 0 ? info.files[i + 1] : null;
  return next ? { file: next.name, before: null } : null;
}

/** Read everything again, as after a lag of the live feed. Requests still
 *  open belong to the live feed, not to a file: they stay. */
function restart() {
  rows = rows.filter((x) => x.open && clientMatch(x.e));
  byKey.clear();
  for (const x of rows) byKey.set(x.key, x);
  if (S.sel && !byKey.has(S.sel)) S.sel = null;
  reading = null;
  started = false;
  loadFiles().catch(failure);
}

/** One page of older entries, from where `reading` points. A file that ends
 *  before the page is full is followed by the next older one, so small files
 *  read as one list. */
async function loadEntries() {
  if (!reading || loadingMore) return;
  loadingMore = true;
  try {
    let got = 0;
    let calls = 0;
    while (reading && got < PAGE) {
      const from = reading;
      let url = "api/entries?file=" + encodeURIComponent(from.file) + "&limit=" + (PAGE - got);
      if (from.before !== null) url += "&before=" + from.before;
      if (S.client) url += "&client=" + encodeURIComponent(S.client);
      calls += 1;
      const r = await fetch(url, { cache: "no-store" });
      // Deleted since the last /api/files: so are the older ones.
      if (r.status === 404) {
        if (reading === from) reading = null;
        break;
      }
      if (!r.ok) throw new Error("api/entries: " + r.status);
      const page = await r.json();
      if (reading !== from) break;
      for (const e of page.entries) upsert(rowOf(e, from.file));
      got += page.entries.length;
      if (page.before !== null) {
        // The server stopped inside the file: at the limit, or at its byte
        // cap. With a client filter a page can be empty: read on, a little.
        reading = { file: from.file, before: page.before };
        if (got > 0 || calls >= 8) break;
        continue;
      }
      reading = olderThan(from.file);
    }
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
  on("file", () => loadFiles().catch(failure));
  on("off", () => loadFiles().catch(failure));
  on("lagged", restart);
}

function liveEntry(e) {
  if (!info) return;
  const f = info.files.find((x) => x.name === e._file);
  if (f) f.entries = (f.entries || 0) + 1;
  else loadFiles().catch(failure);
  if (!clientMatch(e)) return;
  const r = rowOf(e, e._file);
  // The open row of this request becomes the row of its entry.
  const open = e._id !== undefined ? byKey.get("o" + e._id) : null;
  if (open) {
    rows.splice(rows.indexOf(open), 1);
    byKey.delete(open.key);
    fresh.delete(open.key);
    if (S.sel === open.key) S.sel = r.key;
    if (S.ctx && S.ctx.key === open.key) S.ctx.key = r.key;
  }
  const row = upsert(r);
  if (row.id !== undefined) liveMsgs.delete(row.id);
  fresh.add(row.key);
  sortRows();
  render();
}

function liveOpen(e) {
  if (!clientMatch(e)) return;
  const r = upsert(rowOf(e, null));
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
  const r = byKey.get("o" + m.id);
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
  const hay = [r.method, String(r.status), r.url, r.mode, r.type, r.client].join(" ").toLowerCase();
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

const SORT_KEYS = {
  time: (r) => r.start, method: (r) => r.method, status: (r) => r.status, type: (r) => r.type,
  host: (r) => r.host, request: (r) => r.path, mode: (r) => r.mode,
  ms: (r) => (r.open ? Infinity : r.ms), bar: (r) => r.start, size: (r) => r.size,
};

/** The rows the filters let through, newest first or in the picked sort order. */
function visible(span) {
  const out = filtered(span);
  const by = S.sort && SORT_KEYS[S.sort.key];
  if (by) {
    const d = S.sort.dir === "asc" ? 1 : -1;
    out.sort((a, b) => {
      const x = by(a);
      const y = by(b);
      return (x < y ? -1 : x > y ? 1 : 0) * d;
    });
  }
  return out;
}

function filtered(span) {
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

// The table columns: key, label, width. "request" is always on; "host" is a
// column of its own in the list view only.
const COLS = [
  ["time", "Time", "90px"],
  ["method", "Method", "70px"],
  ["status", "Status", "62px"],
  ["type", "Type", "52px"],
  ["host", "Host", "minmax(100px, 0.6fr)"],
  ["request", "Path", "minmax(120px, 1fr)"],
  ["mode", "Mode", "84px"],
  ["client", "Proxy", "90px"],
  ["ms", "Duration", "84px"],
  ["bar", "Waterfall", "minmax(80px, 140px)"],
  ["size", "Size", "64px"],
];
// Columns that step aside while the details are open, to leave the request room.
const AUTO_HIDE = new Set(["type", "host", "mode", "client", "bar"]);
// Columns sorted largest first on the first click.
const NUMERIC = new Set(["time", "ms", "size"]);

function userOn(k) {
  return k === "request" || !S.hide[k];
}

/** The columns drawn, in order, as [key, label, width]. The tree view puts
 *  the request first and has no host column: the tree shows the host. */
function columns(sel) {
  const tree = S.view === "tree";
  const list = tree
    ? [["request", "Request", sel ? "minmax(140px, 1fr)" : "minmax(200px, 1fr)"], ...COLS.filter(([k]) => k !== "request" && k !== "host")]
    : COLS;
  return list.filter(([k]) => userOn(k) && !(sel && AUTO_HIDE.has(k)) && (k !== "client" || manyClients()));
}

/** The Proxy column, group and menu are shown only when they tell
 *  something: all clients on the page, and more than one proxy port. */
function manyClients() {
  return !S.client && !!info && (info.clients || []).length > 1;
}

/** Show one proxy client's requests, or every client's (""). The address
 *  follows, so the page can be bookmarked: proxy.localhost/<name>. */
function setClient(name) {
  S.pop = null;
  if (name === S.client) return render();
  S.client = name;
  if (S.group === "client" && name) S.group = "none";
  try {
    history.replaceState(null, "", name || "./");
  } catch (e) {
    // A page opened from a file has no history to change.
  }
  held = held.filter(([, data]) => !data.request || clientMatch(data));
  restart();
}

let lastVis = [];         // the rows the last draw showed, for Expand all

function draw() {
  const span = timeSpan();
  const vis = visible(span);
  lastVis = vis;
  const sel = (S.sel && byKey.get(S.sel)) || null;
  drawHeader();
  drawToolbar(vis);
  drawTypes();
  // The layout first: the timeline reads its canvas width.
  drawLayout(sel);
  $("timeline").hidden = !S.tl;
  $("tl-btn").classList.toggle("on", S.tl);
  drawTable(span, vis, sel);
  if (S.tl) drawTimeline(span, vis);
  drawDetail(sel);
  drawPops(vis);
  fresh = new Set();
}

function drawHeader() {
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
  // The client menu: once there is a second proxy port, or a page for one.
  $("client-anchor").hidden = !(S.client || (info && (info.clients || []).length > 1));
  $("client-label").textContent = S.client || "All";
  $("client-btn").classList.toggle("active", S.pop === "client" || !!S.client);
  document.title = S.client ? "Proxy log: " + S.client : "Proxy log";
  const tree = S.view === "tree";
  // One button for each pair: the icon shows the state, the title the click.
  setToggle("view-btn", "icon-tree", "icon-list", tree, tree ? "Tree view. Click for the list view" : "List view. Click for the tree view");
  const allOpen = treeAllOpen(vis);
  setToggle("tree-btn", "icon-collapse", "icon-expand", allOpen, allOpen ? "Collapse all" : "Expand all");
  $("group-anchor").hidden = tree;
  $("tree-btn").hidden = !tree;

  $("live-dot").className = "dot" + (S.live ? " live" : "");
  $("icon-pause").toggleAttribute("hidden", !S.live);
  $("icon-play").toggleAttribute("hidden", S.live);
  const lb = $("live-btn");
  const title = S.live ? "Live, click to pause" : "Paused, click to resume";
  lb.title = title;
  lb.setAttribute("aria-label", title);

  const span = timeSpan();
  const range = rangeTimes(span);
  const rb = $("range");
  rb.hidden = !range;
  if (range) rb.textContent = clock(range[0]) + " to " + clock(range[1]);
  $("count").textContent = count(vis.length) + " of " + count(rows.length) + (!S.live && held.length ? ", " + count(held.length) + " new while paused" : "");
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
  // At most 3 digits after the point: 70 s is "1.167 min", not "1.1666666 min".
  const r = (x) => String(+x.toFixed(3));
  if (ms === 0) return "0";
  if (ms < 1000) return r(ms) + " ms";
  if (ms < 60000) return r(ms / 1000) + " s";
  if (ms < 3600000) return r(ms / 60000) + " min";
  return r(ms / 3600000) + " h";
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
  client: (r) => r.client,
};
const GROUPS = [["none", "None"], ["host", "Host"], ["type", "Type"], ["status", "Status class"], ["method", "Method"], ["mode", "Mode"], ["client", "Proxy"]];

function svg(attrs, ...kids) {
  const el = document.createElementNS("http://www.w3.org/2000/svg", attrs.tag || "svg");
  for (const [k, v] of Object.entries(attrs)) if (k !== "tag") el.setAttribute(k, v);
  for (const kid of kids) el.append(kid);
  return el;
}

/** The two small sort arrows of a column head. */
function sortArrows(dir) {
  const up = dir ? (dir === "asc" ? "1" : "0.2") : "0.35";
  const down = dir ? (dir === "desc" ? "1" : "0.2") : "0.35";
  return svg({ width: "8", height: "11", viewBox: "0 0 8 11", fill: "currentColor", class: "arrows", "aria-hidden": "true" },
    svg({ tag: "path", d: "M4 0.5L7.2 4.3H0.8z", opacity: up }),
    svg({ tag: "path", d: "M4 10.5L0.8 6.7h6.4z", opacity: down }));
}

/** A click on a column head: first order, the other order, then no sort. */
function cycleSort(k) {
  const first = NUMERIC.has(k) ? "desc" : "asc";
  const second = first === "asc" ? "desc" : "asc";
  const cur = S.sort && S.sort.key === k ? S.sort.dir : null;
  S.sort = cur === null ? { key: k, dir: first } : cur === first ? { key: k, dir: second } : null;
  render();
}

// ---- the tree view: host, then one folder per path segment, then requests

function segments(r) {
  if (r.mode === "tunnel") return [];
  return r.path.split(/[?#]/)[0].split("/").filter(Boolean);
}

/** The last part of the path and the query, as a request shows in the tree. */
function leafOf(r) {
  if (r.mode === "tunnel") return { name: "(tunnel)", query: "" };
  const segs = segments(r);
  const cut = r.path.search(/[?#]/);
  const base = cut < 0 ? r.path : r.path.slice(0, cut);
  const name = segs.length ? segs[segs.length - 1] + (base.endsWith("/") ? "/" : "") : "/";
  return { name, query: cut < 0 ? "" : r.path.slice(cut) };
}

/** The tree node keys of a row, from the host down: "host", "host/a", "host/a/b". */
function nodeKeys(r) {
  const keys = [r.host || "(no host)"];
  for (const seg of segments(r).slice(0, -1)) keys.push(keys[keys.length - 1] + "/" + seg);
  return keys;
}

/** Hosts start closed, folders open. */
function nodeOpen(key, kind) {
  return key in S.tx ? S.tx[key] : kind !== "host";
}

/** Every host and folder of the tree is open. */
function treeAllOpen(vis) {
  for (const r of vis) {
    const keys = nodeKeys(r);
    for (let i = 0; i < keys.length; i++) if (!nodeOpen(keys[i], i === 0 ? "host" : "folder")) return false;
  }
  return true;
}

function setToggle(id, onIcon, offIcon, on, title) {
  $(onIcon).style.display = on ? "" : "none";
  $(offIcon).style.display = on ? "none" : "";
  $(id).title = title;
  $(id).setAttribute("aria-label", title);
}

function setAllNodes(open) {
  const tx = {};
  for (const r of lastVis) for (const k of nodeKeys(r)) tx[k] = open;
  S.tx = tx;
  render();
}

function buildTree(vis) {
  const top = new Map();
  for (const r of vis) {
    const keys = nodeKeys(r);
    const segs = segments(r);
    let map = top;
    let node = null;
    keys.forEach((k, depth) => {
      let n = map.get(k);
      if (!n) {
        const kind = depth === 0 ? "host" : "folder";
        n = { key: k, label: depth === 0 ? k : segs[depth - 1] + "/", depth, kind, kids: new Map(), reqs: [], n: 0, err: 0, bytes: 0 };
        map.set(k, n);
      }
      n.n++;
      n.bytes += Math.max(0, r.size);
      if (isError(r)) n.err++;
      node = n;
      map = n.kids;
    });
    node.reqs.push(r);
  }
  return top;
}

function nodeEl(n, open) {
  return h("div", { class: "node k-" + n.kind, style: { paddingLeft: 14 + n.depth * 18 + "px" }, title: n.key, onClick: () => { S.tx[n.key] = !open; render(); } },
    h("span", { class: "caret-g", text: open ? "▾" : "▸" }),
    h("span", { class: "label", text: n.label }),
    h("span", { class: "n", text: count(n.n) }),
    h("span", { class: "grow" }),
    n.err ? h("span", { class: "errs", text: count(n.err) + " failed" }) : null,
    h("span", { class: "totals", text: fmtSize(n.bytes) }));
}

function drawTable(span, vis, sel) {
  const tree = S.view === "tree";
  const cols = columns(sel);
  const on = new Set(cols.map((c) => c[0]));
  const scroll = $("scroll");
  scroll.style.setProperty("--cols", cols.map((c) => c[2]).join(" "));

  const head = $("thead");
  head.textContent = "";
  for (const [k, label] of cols) {
    const dir = S.sort && S.sort.key === k ? S.sort.dir : null;
    const name = k === "request" ? (tree ? "Request" : "Path") : label;
    head.append(h("button", {
      class: "th" + (dir ? " on" : "") + (k === "ms" || k === "size" ? " r" : "") + (k === "request" && tree ? " indent" : ""),
      type: "button", title: "Sort by " + (k === "request" ? "path" : label.toLowerCase()), onClick: () => cycleSort(k),
    }, h("span", { text: name }), sortArrows(dir)));
  }

  const range = rangeTimes(span);
  const ctx = {
    on, sel, tree,
    v0: range ? range[0] : span.t0,
    v1: range ? range[1] : span.t1,
    // The host column steps aside while the details are open: the path then shows it.
    hostInPath: !tree && !on.has("host") && userOn("host"),
  };
  const body = $("rows");
  body.textContent = "";
  order = [];
  let drawnRows = 0;
  const frag = document.createDocumentFragment();
  const addRow = (r, opts) => {
    order.push(r.key);
    if (drawnRows >= MAX_DRAWN) return;
    drawnRows++;
    frag.append(rowEl(r, ctx, opts));
  };
  if (tree) {
    const byLabel = (a, b) => a.label.localeCompare(b.label);
    const walk = (n) => {
      const open = nodeOpen(n.key, n.kind);
      frag.append(nodeEl(n, open));
      if (!open) return;
      [...n.kids.values()].sort(byLabel).forEach(walk);
      for (const r of n.reqs) addRow(r, { indent: (n.depth + 1) * 18 + 6 });
    };
    [...buildTree(vis).values()].sort(byLabel).forEach(walk);
  } else if (S.group === "none") {
    for (const r of vis) addRow(r, {});
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
      if (!closed) for (const r of list) addRow(r, { noHost: S.group === "host" });
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
    else empty.append("No requests yet.");
  } else {
    empty.append("No requests match this filter.");
  }
  $("more-wrap").hidden = reading === null || rows.length >= MAX_ROWS || !rows.length;
  $("more").disabled = loadingMore;
}

/** The request cell: in the tree the last path part, in the list the path. */
function requestCell(r, ctx, opts) {
  if (ctx.tree) {
    const leaf = leafOf(r);
    return h("div", { class: "c-req", title: r.url, style: { paddingLeft: opts.indent + "px" } },
      h("span", { class: "path-main", text: leaf.name }), h("span", { class: "path-query", text: leaf.query }));
  }
  if (r.mode === "tunnel") {
    return h("div", { class: "c-req", title: r.url }, ctx.hostInPath && !opts.noHost ? h("span", { class: "host-pre", text: r.host + " " }) : null, h("span", { class: "path-query", text: "(tunnel)" }));
  }
  const cut = r.path.search(/[?#]/);
  return h("div", { class: "c-req", title: r.url },
    ctx.hostInPath && !opts.noHost ? h("span", { class: "host-pre", text: r.host }) : null,
    h("span", { class: "path-main", text: cut < 0 ? r.path : r.path.slice(0, cut) }),
    h("span", { class: "path-query", text: cut < 0 ? "" : r.path.slice(cut) }));
}

function rowEl(r, ctx, opts) {
  const ws = r.type === "ws";
  const cells = [];
  for (const k of ctx.on) {
    if (k === "request") cells.push(requestCell(r, ctx, opts));
    else if (k === "time") cells.push(h("div", { class: "c-time", text: clock(r.start) }));
    else if (k === "method") cells.push(h("div", { class: "c-method m-" + r.method, text: r.method }));
    else if (k === "status") cells.push(h("div", null, h("span", { class: "pill " + statusClass(r.status), text: r.status ? String(r.status) : "…" })));
    else if (k === "type") cells.push(h("div", null, h("span", { class: "tpill" + (ws ? " ws" : ""), text: r.type })));
    else if (k === "host") cells.push(h("div", { class: "c-host", title: r.host, text: r.host }));
    else if (k === "mode") cells.push(h("div", { class: "c-mode" + (r.mode !== "inspect" ? " other" : ""), text: r.mode }));
    else if (k === "client") cells.push(h("div", { class: "c-client", title: r.client, text: r.client }));
    else if (k === "ms") {
      let text;
      let cls = "c-ms r";
      if (r.open && ws) { text = "live"; cls += " live"; }
      else if (r.open) { text = "receiving"; cls += " live"; }
      else if (ws) text = fmtMs(r.receive || r.ms);
      else { text = fmtMs(r.ms); if (r.ms > 1000) cls += " slow"; }
      cells.push(h("div", { class: cls, text }));
    } else if (k === "bar") {
      const left = Math.max(0, Math.min(98, ((r.start - ctx.v0) / (ctx.v1 - ctx.v0)) * 100));
      const width = Math.max(1.5, Math.min(100 - left, ((endOf(r) - Math.max(r.start, ctx.v0)) / (ctx.v1 - ctx.v0)) * 100));
      cells.push(h("div", { class: "wf" }, h("div", { class: ws ? "ws" : isError(r) ? "err" : "", style: { left: left + "%", width: width + "%" } })));
    } else if (k === "size") {
      const text = ws ? (r.messages !== null ? count(r.messages) + " msg" : "") : fmtSize(r.size);
      cells.push(h("div", { class: "c-size r", text }));
    }
  }
  return h("div", {
    class: "row" + (ctx.sel && ctx.sel.key === r.key ? " picked" : "") + (fresh.has(r.key) ? " fresh" : ""),
    onClick: () => select(r.key),
    onContextmenu: (ev) => {
      ev.preventDefault();
      S.sel = r.key;
      S.ctx = { x: ev.clientX, y: ev.clientY, key: r.key };
      render();
    },
  }, cells);
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
  const live = r.open && r.id !== undefined ? liveMsgs.get(r.id) : null;
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
    h("button", { class: "btn small dark", type: "button", text: "Copy as cURL", onClick: () => copyCurl(r) }),
    h("button", { class: "btn small", type: "button", title: "Change the request, then copy it as cURL (E)", text: "Edit as cURL", onClick: () => openBuilder(r) }),
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
  for (const id of ["menu", "filters", "group", "cols", "client"]) {
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
  if (S.pop === "client") drawClient($("client"));
  drawCtx();
}

/** The proxy ports: all, the main one, then one per client. A client that
 *  was removed keeps its page, for its old entries. */
function drawClient(p) {
  const list = info ? (info.clients || []).slice() : [];
  if (S.client && !list.some((c) => c.name === S.client)) list.push({ name: S.client, port: null, removed: true });
  const item = (name, label, hint) =>
    h("button", { class: "item" + (S.client === name ? " picked" : ""), type: "button", onClick: () => setClient(name) },
      h("span", { text: label }), h("span", { class: "hint", text: S.client === name ? "✓" : hint }));
  p.append(item("", "All", ""));
  p.append(h("div", { class: "pop-sep" }));
  for (const c of list) {
    const hint = c.removed ? "removed" : c.port ? ":" + c.port : "off";
    p.append(item(c.name, c.name, hint));
  }
  p.append(h("div", { class: "pop-sep" }), h("div", { class: "note", text: "One proxy port per client. Add one: " + (info ? info.cli : "") + " proxy client add <name>" }));
}

function drawMenu(m) {
  m.append(h("div", { class: "pop-title", text: "View" }));
  m.append(h("button", { class: "item", type: "button", onClick: () => { S.tl = !S.tl; save(); render(); } }, h("span", { class: "box" + (S.tl ? " on" : "") }), "Timeline"));
  const wf = !S.hide.bar;
  m.append(h("button", { class: "item", type: "button", onClick: () => { S.hide.bar = wf; save(); render(); } }, h("span", { class: "box" + (wf ? " on" : "") }), "Waterfall column"));
  m.append(h("div", { class: "pop-sep" }), h("div", { class: "pop-title", text: "Download a log file" }));
  if (!info || !info.files.length) m.append(h("div", { class: "note", text: "No file yet. The first request through the proxy makes one." }));
  for (const f of info ? info.files : []) {
    const meta = (f.entries !== null && f.entries !== undefined ? count(f.entries) + " requests, " : "") + fmtSize(f.size);
    m.append(h("a", { class: "file-item", href: "files/" + encodeURIComponent(f.name) + "?download=1", title: "Download " + f.name, onClick: () => setTimeout(closePops, 0) },
      h("div", { class: "file-name" }, h("span", { text: f.name }), f.current ? h("span", { class: "tag-live", text: "Live" }) : null),
      h("div", { class: "file-meta", text: meta })));
  }
  if (info && info.files.length) {
    m.append(h("div", { class: "note", text: "The table shows the requests of all these files. To open one in Chrome: DevTools → Network → Import HAR file, or drag the file onto the Network panel." }));
  }
  if (info) {
    m.append(h("div", { class: "pop-sep" }));
    m.append(h("div", { class: "facts" },
      h("span", { text: "Folder" }), h("span", { class: "mono", title: info.folder, text: info.folder }),
      h("span", { text: "Rotation" }), h("span", { text: info.file_mb + " MB or " + count(info.file_requests) + " requests, keep " + info.keep_files + " files" }),
      h("span", { text: "Bodies" }), h("span", { text: "First 1 MB of each, headers as sent" }),
      info.dropped ? h("span", { text: "Dropped" }) : null,
      info.dropped ? h("span", { class: "bad-text", text: count(info.dropped) + " requests not written (queue full)" }) : null));
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
    if (k === "client" && !manyClients()) continue;
    p.append(h("button", { class: "item" + (S.group === k ? " picked" : ""), type: "button", onClick: () => { S.group = k; S.collapsed = {}; S.pop = null; save(); render(); } },
      h("span", { text: label }), S.group === k ? h("span", { class: "hint", text: "✓" }) : null));
  }
  if (S.group !== "none") {
    const keys = new Set(rows.map(GROUP_KEYS[S.group] || GROUP_KEYS.host));
    p.append(h("div", { class: "pop-sep" }), h("div", { class: "split" },
      h("button", { type: "button", text: "Expand all", onClick: () => { S.collapsed = {}; render(); } }),
      h("button", { type: "button", text: "Collapse all", onClick: () => { S.collapsed = Object.fromEntries([...keys].map((k) => [k, true])); render(); } })));
  }
}

function drawCols(p) {
  const sel = S.sel && byKey.get(S.sel);
  for (const [k, label] of COLS) {
    if (k === "client" && !manyClients()) continue;
    const locked = k === "request";
    const on = userOn(k);
    const note = locked ? "always" : k === "host" && S.view === "tree" ? "list view only" : sel && on && AUTO_HIDE.has(k) ? "hidden with details" : "";
    p.append(h("button", { class: "item" + (locked ? " locked" : ""), type: "button", onClick: () => { if (locked) return; S.hide[k] = on; save(); render(); } },
      h("span", { class: "box" + (on ? " on" : "") }), h("span", { class: "grow", text: label }),
      h("span", { class: "hint", text: note })));
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
  box.style.top = Math.min(S.ctx.y, window.innerHeight - 280) + "px";
  const item = (label, fn, hint) => h("button", { class: "item", type: "button", onClick: () => { S.ctx = null; fn(); render(); } },
    h("span", { text: label }), hint ? h("span", { class: "hint", text: hint }) : null);
  box.append(
    h("div", { class: "ctx-title", text: r.method + " " + r.host + (r.mode === "tunnel" ? "" : r.path) }),
    item("Build request", () => openBuilder(r), "E"),
    h("div", { class: "pop-sep" }),
    item("Copy URL", () => copy("URL copied", r.url)),
    item("Copy as cURL", () => copyCurl(r)),
    item("Copy as JSON", () => copyJson(r)),
    h("div", { class: "pop-sep" }),
    item("Show only this host", () => { S.fHost = { [r.host]: true }; }),
    item("Hide this host", () => { $("q").value = r.host; S.inv = true; S.q = r.host; }),
  );
}

// ---- the request builder: change a request, then copy it as cURL. The
// viewer is read only (ADR 08, I10): it never sends a request itself.

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"];

function builderShown() {
  return !$("builder").hidden;
}

function closeBuilder() {
  const back = $("builder");
  back.hidden = true;
  back.textContent = "";
}

async function openBuilder(r) {
  S.ctx = null;
  S.pop = null;
  render();
  let e = r.e;
  try {
    e = (await wholeEntry(r)) || e;
  } catch (err) {
    // Without the whole entry the builder has no body.
  }
  const p = e.request.postData;
  const binary = !!(p && p.text && p.encoding === "base64");
  drawBuilder(r, {
    method: r.method,
    url: r.url,
    headers: curlHeaders(e.request.headers),
    body: p && p.text && !binary ? p.text : "",
    binary,
  });
}

function drawBuilder(r, b) {
  const back = $("builder");
  back.textContent = "";
  const method = h("select", { class: "field mono method", "aria-label": "Method" });
  for (const m of METHODS.includes(b.method) ? METHODS : [b.method, ...METHODS]) {
    method.append(h("option", { text: m, selected: m === b.method }));
  }
  const url = h("input", { class: "field mono grow", "aria-label": "URL", spellcheck: "false" });
  url.value = b.url;

  const list = h("div", { class: "hlist" });
  const hcount = h("span");
  const recount = () => { hcount.textContent = count(list.children.length); };
  const addHeader = (k, v) => {
    const name = h("input", { class: "field mono name", placeholder: "name", spellcheck: "false" });
    const value = h("input", { class: "field mono", placeholder: "value", spellcheck: "false" });
    name.value = k;
    value.value = v;
    const row = h("div", { class: "hrow" }, name, value,
      h("button", { class: "field x", type: "button", title: "Remove header", text: "×", onClick: () => { row.remove(); recount(); } }));
    list.append(row);
    return name;
  };
  for (const [k, v] of b.headers) addHeader(k, v);
  recount();

  const body = h("textarea", { class: "field mono body", placeholder: "Raw request body", spellcheck: "false" });
  body.value = b.body;
  const bodyNote = h("span");
  const noBody = () => method.value === "GET" || method.value === "HEAD";
  const noteBody = () => {
    bodyNote.textContent = noBody() ? "ignored for " + method.value : b.binary ? "the recorded body is binary and is not shown here" : "";
  };
  method.addEventListener("change", noteBody);
  noteBody();

  const read = () => ({
    method: method.value,
    url: url.value.trim(),
    headers: [...list.children].map((row) => [row.children[0].value.trim(), row.children[1].value]).filter(([k]) => k),
    body: noBody() ? "" : body.value,
  });
  const copyIt = () => {
    const x = read();
    copy("cURL copied", curlOf(x.method, x.url, x.headers, x.body, false));
    closeBuilder();
  };

  back.append(h("div", { class: "modal", role: "dialog", "aria-modal": "true", "aria-label": "Build request" },
    h("div", { class: "modal-head" },
      h("b", { text: "Build request" }),
      h("span", { class: "sub grow", text: "Based on " + r.method + " " + r.host + " at " + clock(r.start) }),
      h("button", { class: "btn x", type: "button", title: "Close (Esc)", text: "×", onClick: closeBuilder })),
    h("div", { class: "modal-body" },
      h("div", { class: "line" }, method, url),
      h("div", { class: "sec" }, h("div", { class: "sec-head" }, h("b", { text: "Headers" }), hcount), list,
        h("div", null, h("button", { class: "link-btn flush", type: "button", text: "Add header", onClick: () => { addHeader("", "").focus(); recount(); } }))),
      h("div", { class: "sec" }, h("div", { class: "sec-head" }, h("b", { text: "Body" }), bodyNote), body)),
    h("div", { class: "modal-foot" },
      h("span", { class: "sub grow", text: "The viewer does not send requests. Run the command in a terminal." }),
      h("button", { class: "btn", type: "button", text: "Cancel", onClick: closeBuilder }),
      h("button", { class: "btn teal", type: "button", text: "Copy as cURL", onClick: copyIt }))));
  back.hidden = false;
  url.focus();
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

/** The request headers a cURL command carries, as [name, value]. */
function curlHeaders(list) {
  return (list || []).filter((x) => !x.name.startsWith(":") && !SKIP_CURL.has(x.name.toLowerCase())).map((x) => [x.name, x.value]);
}

function curlOf(method, url, headers, body, binary) {
  const parts = ["curl " + shellQuote(url)];
  if (method !== "GET" && !(method === "POST" && (body || binary))) parts.push("-X " + method);
  for (const [k, v] of headers) parts.push("-H " + shellQuote(k + ": " + v));
  if (headers.some(([k]) => k.toLowerCase() === "accept-encoding")) parts.push("--compressed");
  if (body) parts.push("--data-raw " + shellQuote(body));
  // Last: the shell comment ends the command.
  else if (binary) parts.push("--data-binary @- # base64 body, see Copy as JSON");
  return parts.join(" \\\n  ");
}

async function copyCurl(r) {
  let e = r.e;
  try {
    e = (await wholeEntry(r)) || e;
  } catch (err) {
    // Without the whole entry the command has no body.
  }
  const p = e.request.postData;
  const binary = !!(p && p.text && p.encoding === "base64");
  copy("cURL copied", curlOf(r.method, r.url, curlHeaders(e.request.headers), p && p.text && !binary ? p.text : "", binary));
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
  if (builderShown()) {
    if (e.key === "Escape") closeBuilder();
    return;
  }
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
  if ((e.key === "e" || e.key === "E") && !e.metaKey && !e.ctrlKey && !e.altKey) {
    const r = S.sel && byKey.get(S.sel);
    if (r) openBuilder(r);
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
  $("client-btn").addEventListener("click", toggle("client"));
  $("menu-btn").addEventListener("click", toggle("menu"));
  $("view-btn").addEventListener("click", () => { S.view = S.view === "tree" ? "list" : "tree"; save(); render(); });
  $("tree-btn").addEventListener("click", () => setAllNodes(!treeAllOpen(lastVis)));
  $("builder").addEventListener("mousedown", (e) => { if (e.target === $("builder")) closeBuilder(); });
  $("live-btn").addEventListener("click", () => setLive(!S.live));
  $("range").addEventListener("click", () => { S.range = null; render(); });
  $("tl-btn").addEventListener("click", () => { S.tl = !S.tl; save(); render(); });
  $("tl-canvas").addEventListener("mousedown", tlDown);
  $("more").addEventListener("click", () => loadEntries().catch(failure));
  $("resize").addEventListener("mousedown", resizeDown);
  $("resize").addEventListener("dblclick", () => { S.detailW = null; S.max = false; save(); render(); });
  $("scroll").addEventListener("scroll", () => {
    const s = $("scroll");
    if (!loadingMore && reading !== null && rows.length < MAX_ROWS && s.scrollTop + s.clientHeight > s.scrollHeight - 200) {
      loadEntries().catch(failure);
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
