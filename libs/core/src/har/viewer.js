// The proxy log viewer (ADR 08, change 2). Plain JavaScript, no framework.
// Every URL is relative, so the page works at the root of its own name and
// under the second address with a /proxy-log/ prefix.
"use strict";

const PAGE = 500;      // entries per /api/entries call
const MAX_ROWS = 2000; // rows kept on the page

const el = (id) => document.getElementById(id);
let info = null;       // the last /api/files answer
let file = null;       // the file shown
let rows = [];         // entries shown, newest first
let before = null;     // cursor for "Show more"
let picked = null;     // the entry in the details pane
let live = null;       // the EventSource

function make(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

function size(n) {
  if (n === null || n === undefined || n < 0) return "";
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
  return (n / 1024 / 1024).toFixed(1) + " MB";
}

function hostAndPath(url) {
  try {
    const u = new URL(url);
    return [u.host, u.pathname + u.search];
  } catch (e) {
    return ["", url];
  }
}

function clock(iso) {
  const d = new Date(iso);
  if (isNaN(d)) return "";
  const pad = (n, w) => String(n).padStart(w || 2, "0");
  return pad(d.getHours()) + ":" + pad(d.getMinutes()) + ":" + pad(d.getSeconds()) + "." + pad(d.getMilliseconds(), 3);
}

function isError(e) {
  const s = e.response ? e.response.status : 0;
  return s === 0 || s >= 400;
}

function codeLine(parent, before, code, after) {
  parent.append(before);
  parent.append(make("code", "", code));
  if (after) parent.append(after);
}

// ---- state line and files

async function loadFiles() {
  const r = await fetch("api/files", { cache: "no-store" });
  if (!r.ok) throw new Error("api/files: " + r.status);
  info = await r.json();
  showState();
  showFiles();
}

function showState() {
  const s = el("state");
  s.textContent = "";
  const part = (label, value) => {
    s.append(label + " ");
    s.append(make("b", "", value));
    s.append("   ");
  };
  part("Proxy:", info.proxy ? "on" : "off");
  part("Log:", info.log ? "on" : "off");
  part("Folder:", info.folder);
  part("Limits:", info.file_mb + " MB or " + info.file_requests + " requests per file, " + info.keep_files + " files kept");
  if (info.dropped) part("Dropped:", String(info.dropped));

  const err = el("error");
  err.hidden = !info.error;
  err.textContent = info.error ? "The log stopped: " + info.error + ". Turn it off and on again to retry." : "";

  const hint = el("hint");
  hint.textContent = "";
  hint.hidden = info.proxy && info.log;
  if (!info.proxy) {
    codeLine(hint, "The proxy is off, so nothing is recorded. Turn it on in Settings → Proxy, or run ", info.cli + " proxy on", ".");
  } else if (!info.log) {
    codeLine(hint, "The log is off. Turn it on in Settings → Proxy, or run ", info.cli + " proxy log on", ".");
  }
}

function showFiles() {
  const list = el("files");
  list.textContent = "";
  if (!info.files.length) {
    list.append(make("li", "note", "No file yet. The first proxied request makes one."));
  }
  if (!file || !info.files.some((f) => f.name === file)) {
    const current = info.files.find((f) => f.current) || info.files[0];
    if (current) pick(current.name);
  }
  for (const f of info.files) {
    const li = make("li", f.name === file ? "picked" : "");
    li.append(make("div", "name", f.name + (f.current ? "  (current)" : "")));
    const entries = f.entries === null || f.entries === undefined ? "" : f.entries + " requests, ";
    li.append(make("div", "meta", entries + size(f.size)));
    const a = make("a", "", "Download HAR");
    a.href = "files/" + encodeURIComponent(f.name) + "?download=1";
    a.addEventListener("click", (ev) => ev.stopPropagation());
    li.append(a);
    li.addEventListener("click", () => pick(f.name));
    list.append(li);
  }
}

// ---- entries

function pick(name) {
  if (file === name) return;
  file = name;
  rows = [];
  before = null;
  closeDetails();
  for (const li of el("files").children) {
    li.classList.toggle("picked", li.firstChild && li.firstChild.textContent.startsWith(name));
  }
  loadEntries(false).catch(showFailure);
}

async function loadEntries(more) {
  const name = file;
  let url = "api/entries?file=" + encodeURIComponent(name) + "&limit=" + PAGE;
  if (more && before !== null) url += "&before=" + before;
  const r = await fetch(url, { cache: "no-store" });
  if (!r.ok) throw new Error("api/entries: " + r.status);
  const page = await r.json();
  if (name !== file) return;
  rows = more ? rows.concat(page.entries) : page.entries;
  if (rows.length > MAX_ROWS) rows.length = MAX_ROWS;
  before = page.before;
  render(null);
}

function matches(e, words) {
  if (el("errors").checked && !isError(e)) return false;
  if (!words) return true;
  const hay = [e.request.method, e.request.url, String(e.response.status), e._mode || ""].join(" ").toLowerCase();
  return words.every((w) => hay.includes(w));
}

function render(fresh) {
  const body = el("rows");
  body.textContent = "";
  const words = el("filter").value.trim().toLowerCase().split(/\s+/).filter(Boolean);
  let shown = 0;
  for (const e of rows) {
    if (!matches(e, words.length ? words : null)) continue;
    body.append(row(e, e === fresh));
    shown++;
  }
  el("count").textContent = shown + " of " + rows.length + " loaded";
  const empty = el("empty");
  empty.hidden = rows.length > 0;
  empty.textContent = file ? "No requests in this file yet." : "";
  el("more").hidden = before === null || rows.length >= MAX_ROWS;
}

function row(e, fresh) {
  const tr = make("tr", (e === picked ? "picked " : "") + (fresh ? "new" : ""));
  const [host, path] = hostAndPath(e.request.url);
  const status = e.response.status;
  tr.append(make("td", "", clock(e.startedDateTime)));
  tr.append(make("td", "", e.request.method));
  tr.append(make("td", isError(e) ? "bad" : "ok", String(status)));
  tr.append(make("td", "", host));
  const p = make("td", "", path);
  p.title = e.request.url;
  tr.append(p);
  tr.append(make("td", "", e._mode || ""));
  tr.append(make("td", "num", String(Math.round(e.time))));
  tr.append(make("td", "num", size(e.response.bodySize)));
  tr.addEventListener("click", () => showDetails(e));
  return tr;
}

// ---- details

function list(parent, title, pairs) {
  parent.append(make("h3", "", title));
  const dl = make("dl");
  for (const [k, v] of pairs) {
    dl.append(make("dt", "", k));
    dl.append(make("dd", "", v));
  }
  if (!pairs.length) dl.append(make("dd", "note", "none"));
  parent.append(dl);
}

function showDetails(e) {
  picked = e;
  el("details").hidden = false;
  el("details-title").textContent = e.request.method + " " + e.request.url;
  const body = el("details-body");
  body.textContent = "";
  list(body, "General", [
    ["Status", e.response.status + " " + (e.response.statusText || "")],
    ["Started", e.startedDateTime],
    ["Time to headers", Math.round(e.time) + " ms"],
    ["HTTP", e.request.httpVersion + " / " + e.response.httpVersion],
  ]);
  const extra = Object.keys(e).filter((k) => k.startsWith("_")).map((k) => [k, typeof e[k] === "string" ? e[k] : JSON.stringify(e[k])]);
  list(body, "LocalRouter", extra);
  list(body, "Request headers", e.request.headers.map((h) => [h.name, h.value]));
  if (e.request.queryString && e.request.queryString.length) {
    list(body, "Query", e.request.queryString.map((q) => [q.name, q.value]));
  }
  list(body, "Response headers", e.response.headers.map((h) => [h.name, h.value]));
  list(body, "Timings", Object.entries(e.timings).map(([k, v]) => [k, v + " ms"]));
  body.append(make("p", "note", "Bodies are not written to the log. Secret headers show [redacted]."));
  render(null);
}

function closeDetails() {
  picked = null;
  el("details").hidden = true;
}

// ---- live

function startLive() {
  if (live) live.close();
  live = new EventSource("api/live");
  live.addEventListener("entry", (ev) => {
    const current = info && info.files.find((f) => f.current);
    if (!current || current.name !== file) return;
    let e;
    try {
      e = JSON.parse(ev.data);
    } catch (err) {
      return;
    }
    rows.unshift(e);
    if (rows.length > MAX_ROWS) rows.length = MAX_ROWS;
    current.entries = (current.entries || 0) + 1;
    render(e);
  });
  live.addEventListener("file", () => {
    const follow = !file || (info && info.files.some((f) => f.current && f.name === file));
    if (follow) file = null;
    loadFiles().catch(showFailure);
  });
  live.addEventListener("off", () => loadFiles().catch(showFailure));
  live.addEventListener("lagged", () => {
    const name = file;
    file = null;
    loadFiles().then(() => pick(name)).catch(showFailure);
  });
}

function showFailure(e) {
  const err = el("error");
  err.hidden = false;
  err.textContent = "The page could not read the log: " + e.message;
}

document.addEventListener("DOMContentLoaded", () => {
  el("filter").addEventListener("input", () => render(null));
  el("errors").addEventListener("change", () => render(null));
  el("more").addEventListener("click", () => loadEntries(true).catch(showFailure));
  el("close").addEventListener("click", () => {
    closeDetails();
    render(null);
  });
  loadFiles().then(startLive).catch(showFailure);
  // The state line (dropped, error) changes without an event.
  setInterval(() => loadFiles().catch(() => {}), 10000);
});
