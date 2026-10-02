//! Lua scripts for traffic (ADR 07): the rule table, and the hook the router
//! and the forward proxy call for each HTTP exchange.
//!
//! - Intercept rules run while the client waits and may change or answer a
//!   request, change a response, and change or drop each event of a stream.
//! - Log rules get a copy of the finished exchange, through a queue that
//!   drops instead of waiting, so they never slow or change traffic (I2).
//!
//! With no matching rule, [`Scripts::matching`] returns `None` before any
//! allocation, and the exchange runs no Lua and holds or copies no body (I1).

pub mod bodies;
pub mod engine;
pub mod events;
pub mod lua_api;
pub mod rules;

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body as _, Frame, SizeHint};
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, Request, Response, StatusCode, Uri};
use tokio::sync::{mpsc, oneshot};

use crate::api::ScriptRuleView;
use crate::proxy::{Body, escape, page};
use crate::secrets::SecretHeaders;
use bodies::{Budget, BodyClass, Decoder, Encoding, LimitedVec, Reservation};
use engine::{ActiveRule, Call, Loaded, Output, Pool, ScriptKind};
use events::{Event, Format, Splitter};
use lua_api::{BodyMeta, BodyRule, Exchange, Message, RequestInfo, ResponseInfo, ScriptResponse};
use rules::{OnError, RuleHost, ScriptRule};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The limits of ADR 07. Tests use smaller ones ([`Scripts::with_limits`]).
#[derive(Debug, Clone)]
pub struct Limits {
    pub hold: usize,
    pub copy: usize,
    pub budget: usize,
    pub event: usize,
    pub queue_items: usize,
    pub queue_bytes: usize,
    pub save_queue: usize,
    pub intercept_time: Duration,
    pub log_time: Duration,
    /// Tests only: the saved-body writer waits this long before each write,
    /// to play a slow disk.
    pub save_delay: Option<Duration>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            hold: bodies::HOLD_LIMIT,
            copy: bodies::COPY_LIMIT,
            budget: bodies::BUDGET,
            event: bodies::EVENT_LIMIT,
            queue_items: bodies::QUEUE_ITEMS,
            queue_bytes: bodies::QUEUE_BYTES,
            save_queue: bodies::SAVE_QUEUE,
            intercept_time: engine::INTERCEPT_TIME,
            log_time: engine::LOG_TIME,
            save_delay: None,
        }
    }
}

/// Counters for tests: proof that the path without rules does nothing (T18).
#[derive(Debug, Default)]
pub struct Stats {
    pub lua_calls: AtomicU64,
    pub held_bytes: AtomicU64,
    pub copied_bytes: AtomicU64,
}

/// A rule in the table, and its queue when it is a log rule.
pub struct Entry {
    pub active: Arc<ActiveRule>,
    queue: Option<LogQueue>,
}

impl Entry {
    fn info(&self) -> Option<engine::ScriptInfo> {
        self.active.info()
    }
}

/// One item for a log rule's script.
struct LogItem {
    call: Call,
    bytes: usize,
    _memory: Option<Arc<Reservation>>,
}

struct LogQueue {
    tx: mpsc::UnboundedSender<LogItem>,
    items: Arc<AtomicUsize>,
    bytes: Arc<AtomicUsize>,
}

/// Called with the id of a rule turned off after 20 failures in a row.
pub type OnDisable = Arc<dyn Fn(String) + Send + Sync>;

/// The rules and the script threads. One per daemon.
pub struct Scripts {
    rules: RwLock<Arc<Vec<Arc<Entry>>>>,
    pub budget: Arc<Budget>,
    pub limits: Limits,
    /// Shared with the HAR writer (ADR 08): one list for both.
    pub secrets: Arc<SecretHeaders>,
    on_disable: Mutex<Option<OnDisable>>,
    pub stats: Stats,
    this: Weak<Scripts>,
}

/// The script threads of intercept calls. Log calls have their own threads,
/// so a slow log script never makes a client wait (I2).
fn pool(kind: Option<ScriptKind>) -> &'static Pool {
    static INTERCEPT: OnceLock<Pool> = OnceLock::new();
    static LOG: OnceLock<Pool> = OnceLock::new();
    match kind {
        Some(ScriptKind::Log) => LOG.get_or_init(|| Pool::new("lua-log")),
        _ => INTERCEPT.get_or_init(|| Pool::new("lua")),
    }
}

/// A unique, sortable id for an exchange: 10 characters of time and 16 of
/// randomness, in Crockford base 32 (like a ULID).
pub fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    let random = (h.finish() as u128) << 16 | (COUNTER.load(Ordering::Relaxed) as u128 & 0xffff);
    let time = crate::logs::now_ms() as u128;
    let value = time << 80 | random & ((1u128 << 80) - 1);
    (0..26).rev().map(|i| ALPHABET[(value >> (i * 5) & 31) as usize] as char).collect()
}

/// The rules that match one request.
#[derive(Default)]
pub struct Matched {
    intercept: Vec<Arc<Entry>>,
    log: Vec<Arc<Entry>>,
}

impl Matched {
    fn ids(&self) -> Vec<String> {
        self.intercept.iter().chain(&self.log).map(|e| e.active.id().to_string()).collect()
    }
}

/// Where a request came from, for `req.source`, `req.route` and the URL parts.
pub struct Ctx {
    /// `router` or `proxy`.
    pub source: &'static str,
    pub route: Option<String>,
    pub scheme: &'static str,
    /// Lower case, without the port.
    pub host: String,
    pub port: u16,
}

/// The response, and what the request log says about scripts.
pub struct Outcome {
    pub response: Response<Body>,
    /// Ids of the rules that ran.
    pub rules: Vec<String>,
    /// The id of a rule that failed.
    pub script_error: Option<String>,
}

/// What `send` gives back: the response, and the error text when the
/// response is LocalRouter's own error page (the server did not answer).
pub type Sent = (Response<Body>, Option<String>);

impl Scripts {
    pub fn new() -> Arc<Self> {
        Self::with_limits(Limits::default())
    }

    /// Tests only need this: smaller limits than the real ones.
    pub fn with_limits(limits: Limits) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            rules: RwLock::new(Arc::new(vec![])),
            budget: Budget::new(limits.budget),
            limits,
            secrets: SecretHeaders::new(),
            on_disable: Mutex::new(None),
            stats: Stats::default(),
            this: this.clone(),
        })
    }

    /// Called with the rule id when a rule is turned off after 20 failures.
    pub fn set_on_disable(&self, f: OnDisable) {
        *self.on_disable.lock().unwrap() = Some(f);
    }

    /// The secret headers: the defaults plus `secret_headers` of the config.
    pub fn set_secret_headers(&self, extra: &[String]) {
        self.secrets.set(extra);
    }

    fn snapshot(&self) -> Arc<Vec<Arc<Entry>>> {
        self.rules.read().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.snapshot().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, id: &str) -> Option<Arc<Entry>> {
        self.snapshot().iter().find(|e| e.active.id() == id).cloned()
    }

    /// Every rule as it is now (with `enabled` as it is now).
    pub fn rules(&self) -> Vec<ScriptRule> {
        self.snapshot()
            .iter()
            .map(|e| ScriptRule { enabled: e.active.is_enabled(), ..e.active.rule.clone() })
            .collect()
    }

    /// Insert or replace the rule with this id. Counters carry over when the
    /// script and `output_dir` stay the same (enable, disable, a new note).
    /// Returns the rule it replaced, which [`Scripts::restore`] puts back.
    pub fn put(&self, rule: ScriptRule, loaded: Result<Loaded, String>) -> Option<Arc<Entry>> {
        let old = self.get(&rule.id);
        let keep = old.as_ref().filter(|o| o.active.rule.script == rule.script && o.active.rule.output_dir == rule.output_dir);
        let active = Arc::new(ActiveRule::new(rule, loaded, keep.map(|o| o.active.as_ref())));
        let queue = (active.info().map(|i| i.kind) == Some(ScriptKind::Log)).then(|| self.start_queue(&active)).flatten();
        let entry = Arc::new(Entry { active, queue });
        let mut rules = self.rules.write().unwrap();
        let mut list: Vec<Arc<Entry>> = rules.iter().filter(|e| e.active.id() != entry.active.id()).cloned().collect();
        list.push(entry);
        list.sort_by(|a, b| a.active.id().cmp(b.active.id()));
        *rules = Arc::new(list);
        old
    }

    pub fn remove(&self, id: &str) -> Option<Arc<Entry>> {
        let mut rules = self.rules.write().unwrap();
        let old = rules.iter().find(|e| e.active.id() == id).cloned()?;
        *rules = Arc::new(rules.iter().filter(|e| e.active.id() != id).cloned().collect());
        Some(old)
    }

    /// Undo a `put` or a `remove` whose file write failed: the rule with
    /// this id becomes `old` again, or goes away.
    pub fn restore(&self, id: &str, old: Option<Arc<Entry>>) {
        let mut rules = self.rules.write().unwrap();
        let mut list: Vec<Arc<Entry>> = rules.iter().filter(|e| e.active.id() != id).cloned().collect();
        list.extend(old);
        list.sort_by(|a, b| a.active.id().cmp(b.active.id()));
        *rules = Arc::new(list);
    }

    /// Ids of the rules owned by this process.
    pub fn owned_by(&self, pid: u32) -> Vec<String> {
        self.snapshot().iter().filter(|e| e.active.rule.owner_pid == Some(pid)).map(|e| e.active.id().to_string()).collect()
    }

    /// How a rule would look, for `check_only`: nothing is stored.
    pub fn preview(rule: ScriptRule, loaded: Loaded) -> ScriptRuleView {
        view(&ActiveRule::new(rule, Ok(loaded), None))
    }

    /// Host patterns outside `.localhost` of enabled, loaded rules: they join
    /// the inspect set (ADR 07, change 2; I11).
    pub fn inspected_hosts(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for e in self.snapshot().iter() {
            if e.active.is_enabled() && e.active.loaded().is_some() && e.active.rule.pattern().is_internet() && !out.contains(&e.active.rule.host) {
                out.push(e.active.rule.host.clone());
            }
        }
        out
    }

    /// Each rule with its state, for `list_script_rules` and `get_proxy`.
    pub fn views(&self) -> Vec<ScriptRuleView> {
        self.snapshot().iter().map(|e| view(&e.active)).collect()
    }

    pub fn view(&self, id: &str) -> Option<ScriptRuleView> {
        self.get(id).map(|e| view(&e.active))
    }

    fn start_queue(&self, active: &Arc<ActiveRule>) -> Option<LogQueue> {
        let handle = tokio::runtime::Handle::try_current().ok()?;
        let (tx, mut rx) = mpsc::unbounded_channel::<LogItem>();
        let items = Arc::new(AtomicUsize::new(0));
        let bytes = Arc::new(AtomicUsize::new(0));
        let (i, b) = (items.clone(), bytes.clone());
        let active = active.clone();
        let this = self.this.clone();
        handle.spawn(async move {
            // One call at a time, in the order exchanges ended (I16).
            while let Some(item) = rx.recv().await {
                i.fetch_sub(1, Ordering::AcqRel);
                b.fetch_sub(item.bytes, Ordering::AcqRel);
                let Some(scripts) = this.upgrade() else { return };
                if !active.is_enabled() {
                    continue;
                }
                let limit = scripts.limits.log_time;
                let _ = scripts.call(&active, item.call, limit).await;
            }
        });
        Some(LogQueue { tx, items, bytes })
    }

    /// Give a log rule one item, or drop it when the queue is full (I2).
    fn enqueue(&self, entry: &Entry, call: Call, bytes: usize, memory: Option<Arc<Reservation>>) {
        let Some(q) = &entry.queue else { return };
        if q.items.load(Ordering::Acquire) >= self.limits.queue_items
            || q.bytes.load(Ordering::Acquire) + bytes > self.limits.queue_bytes
        {
            entry.active.counters.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        q.items.fetch_add(1, Ordering::AcqRel);
        q.bytes.fetch_add(bytes, Ordering::AcqRel);
        if q.tx.send(LogItem { call, bytes, _memory: memory }).is_err() {
            entry.active.counters.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Run one call on a script thread, and count its result.
    async fn call(&self, active: &Arc<ActiveRule>, call: Call, limit: Duration) -> Result<Output, String> {
        self.stats.lua_calls.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + limit;
        let out = match tokio::time::timeout(limit, active.permits.acquire()).await {
            Ok(Ok(permit)) => {
                let a = active.clone();
                let secrets = self.secrets.get();
                let rx = pool(active.info().map(|i| i.kind)).run(move || a.execute(call, deadline, &secrets));
                let out = rx.await.unwrap_or_else(|_| Err("the script thread stopped".into()));
                drop(permit);
                out
            }
            _ => Err("busy: every Lua state of this rule was in use for the whole time limit".into()),
        };
        match &out {
            Ok(_) => active.succeeded(),
            Err(m) => self.fail(active, m),
        }
        out
    }

    /// Count a failure of a rule; when it turns the rule off, tell the daemon.
    fn fail(&self, active: &ActiveRule, message: &str) {
        if active.failed(message) {
            let cb = self.on_disable.lock().unwrap().clone();
            if let Some(cb) = cb {
                cb(active.id().to_string());
            }
        }
    }

    /// The rules that match a request, or `None` before any allocation when
    /// there are none (I1).
    pub fn matching(&self, host: &str, path: &str, method: &str) -> Option<Matched> {
        let rules = self.rules.read().unwrap();
        if rules.is_empty() || !rules.iter().any(|e| e.active.rule.matches(host, path, method) && e.active.is_enabled()) {
            return None;
        }
        let mut m = Matched::default();
        for e in rules.iter() {
            if !e.active.rule.matches(host, path, method) || !e.active.is_enabled() {
                continue;
            }
            match e.info().map(|i| i.kind) {
                Some(ScriptKind::Intercept) => m.intercept.push(e.clone()),
                Some(ScriptKind::Log) => m.log.push(e.clone()),
                None => {}
            }
        }
        m.intercept.sort_by_key(|e| (e.active.rule.order, e.active.id().to_string()));
        (!m.intercept.is_empty() || !m.log.is_empty()).then_some(m)
    }

    /// Run the matching rules on one exchange. `send` sends the request to
    /// the server (or the folder) and is not called when a script answers.
    pub async fn exchange<F, Fut>(self: &Arc<Self>, m: Matched, ctx: Ctx, req: Request<Body>, send: F) -> Outcome
    where
        F: FnOnce(Request<Body>) -> Fut,
        Fut: Future<Output = Sent>,
    {
        let started = Instant::now();
        let time_ms = crate::logs::now_ms();
        for e in m.intercept.iter().chain(&m.log) {
            e.active.counters.matched.fetch_add(1, Ordering::Relaxed);
        }
        let ids = m.ids();
        let mut script_error: Option<String> = None;

        // ---- the request
        let (mut parts, body) = req.into_parts();
        let head_request = parts.method == Method::HEAD;
        let headers = header_list(&parts.headers);
        let req_class = if body.is_end_stream() { BodyClass::None } else { BodyClass::of(text_header(&parts.headers, header::CONTENT_TYPE).as_deref()) };
        let req_encoding = Encoding::of(text_header(&parts.headers, header::CONTENT_ENCODING).as_deref());
        let mut info = RequestInfo {
            id: new_id(),
            source: ctx.source,
            route: ctx.route.clone(),
            method: parts.method.to_string(),
            scheme: ctx.scheme,
            host: ctx.host.clone(),
            port: ctx.port,
            path: parts.uri.path().to_string(),
            query: parts.uri.query().unwrap_or("").to_string(),
            msg: Message { meta: BodyMeta::from_headers(&headers, "range", req_class), headers, body: None },
        };
        let partial_request = parts.headers.contains_key(header::CONTENT_RANGE);
        let on_request: Vec<&Arc<Entry>> = m.intercept.iter().filter(|e| e.info().is_some_and(|i| i.on_request)).collect();
        let hold_request = req_class != BodyClass::None && on_request.iter().any(|e| e.info().is_some_and(|i| i.request_body.contains(req_class)));

        let mut held_request: Option<(Bytes, Bytes)> = None;
        let mut request_body = body;
        let mut body_rule = if partial_request { BodyRule::Partial } else { BodyRule::Free };
        let mut _held_memory: Option<Reservation> = None;
        if hold_request {
            match self.hold(request_body, req_encoding).await {
                Held::Whole { raw, decoded, memory } => {
                    info.msg.body = Some(decoded.clone());
                    info.msg.meta.size = Some(raw.len() as u64);
                    request_body = full(raw.clone());
                    held_request = Some((raw, decoded));
                    _held_memory = Some(memory);
                }
                Held::Not { body, reason } => {
                    info.msg.meta.skipped = Some(reason);
                    info.msg.meta.truncated = reason == "too_large";
                    if reason == "too_large" && body_rule == BodyRule::Free {
                        body_rule = BodyRule::TooLarge;
                    }
                    request_body = body;
                }
            }
        } else if req_class != BodyClass::None {
            info.msg.meta.skipped = Some("class");
        }

        let mut answer: Option<Response<Body>> = None;
        let mut answered_by: Option<String> = None;
        let mut new_request_body: Option<Bytes> = None;
        let mut request_changed = false;
        for e in &on_request {
            let call = Call::Request { req: info.clone(), body: body_rule };
            match self.call(&e.active, call, self.limits.intercept_time).await {
                Ok(Output::Request { req, body, response }) => {
                    if let Some(r) = response {
                        e.active.counters.answered.fetch_add(1, Ordering::Relaxed);
                        answered_by = Some(e.active.id().to_string());
                        answer = Some(script_response(r));
                        break;
                    }
                    info = *req;
                    request_changed = true;
                    if body.is_some() {
                        new_request_body = body;
                        // A script set a body: later rules may change it again.
                        body_rule = if partial_request { BodyRule::Partial } else { BodyRule::Free };
                    }
                }
                Ok(_) => {}
                Err(msg) => {
                    script_error = Some(e.active.id().to_string());
                    if e.active.rule.on_error() == OnError::Fail {
                        answer = Some(failed_page(e.active.id(), &msg));
                        break;
                    }
                }
            }
        }
        if request_changed {
            if let Ok(method) = Method::from_bytes(info.method.as_bytes()) {
                parts.method = method;
            }
            let pq = if info.query.is_empty() { info.path.clone() } else { format!("{}?{}", info.path, info.query) };
            if parts.uri.path_and_query().map(|p| p.as_str()) != Some(pq.as_str())
                && let Ok(uri) = pq.parse::<Uri>()
            {
                parts.uri = uri;
            }
            parts.headers = header_map(&info.msg.headers);
        }
        if let Some(b) = &new_request_body {
            for h in [header::CONTENT_LENGTH, header::CONTENT_ENCODING, header::TRANSFER_ENCODING] {
                parts.headers.remove(h);
            }
            parts.headers.insert(header::CONTENT_LENGTH, HeaderValue::from(b.len()));
            request_body = full(b.clone());
            info.msg.meta.encoding = None;
            info.msg.meta.size = Some(b.len() as u64);
        }

        // ---- log rules on the request body
        let req_plan = Plan::for_side(&m.log, req_class, &info.id, "req", info.msg.meta.content_type.as_deref(), partial_request);
        // A whole body (held, or set by a script) is copied and saved later,
        // in the task that ends the exchange: never while the client waits.
        let whole_request = new_request_body
            .clone()
            .map(|b| (b.clone(), b.len()))
            .or_else(|| held_request.as_ref().map(|(raw, d)| (d.clone(), raw.len())));
        let request_side: SideFuture = if let Some((b, wire)) = whole_request {
            self.side_from_held(b, req_plan, wire as u64)
        } else if answer.is_some() {
            // The server is not called; the body is not read.
            Box::pin(async { SideResult::default() })
        } else if req_plan.wants_bytes() {
            let (body, rx) = self.tap(request_body, req_plan, req_encoding, None);
            request_body = body;
            Box::pin(async move { rx.await.unwrap_or_default() })
        } else {
            Box::pin(async { SideResult::default() })
        };

        // ---- the server. Only a response from a script (an answer, or the
        // page of a failed rule) skips the response rules; a failure passed
        // with on_error "pass" does not.
        let scripted = answer.is_some();
        let (response, gateway_error) = match answer.take() {
            Some(a) => (a, None),
            None => {
                let req = Request::from_parts(parts, request_body);
                send(req).await
            }
        };

        // ---- the response
        let (mut rparts, rbody) = response.into_parts();
        let status = rparts.status;
        let upgraded = status == StatusCode::SWITCHING_PROTOCOLS;
        let res_class = if head_request || status.is_informational() || status == StatusCode::NO_CONTENT || status == StatusCode::NOT_MODIFIED || rbody.is_end_stream() {
            BodyClass::None
        } else {
            BodyClass::of(text_header(&rparts.headers, header::CONTENT_TYPE).as_deref())
        };
        let res_encoding = Encoding::of(text_header(&rparts.headers, header::CONTENT_ENCODING).as_deref());
        let rheaders = header_list(&rparts.headers);
        let mut res = ResponseInfo { status: status.as_u16(), msg: Message { meta: BodyMeta::from_headers(&rheaders, "content-range", res_class), headers: rheaders, body: None } };
        let mut response_body = rbody;
        let mut _res_memory: Option<Reservation> = None;
        let mut held_response: Option<Bytes> = None;
        let request_view = Arc::new(info.clone());

        let on_response: Vec<&Arc<Entry>> = if scripted || upgraded {
            vec![]
        } else {
            m.intercept.iter().filter(|e| e.info().is_some_and(|i| i.on_response)).collect()
        };
        if !on_response.is_empty() {
            let mut rule = if status == StatusCode::PARTIAL_CONTENT { BodyRule::Partial } else { BodyRule::Free };
            let hold = res_class != BodyClass::None && on_response.iter().any(|e| e.info().is_some_and(|i| i.response_body.contains(res_class)));
            if hold {
                match self.hold(response_body, res_encoding).await {
                    Held::Whole { raw, decoded, memory } => {
                        res.msg.body = Some(decoded.clone());
                        res.msg.meta.size = Some(raw.len() as u64);
                        response_body = full(raw);
                        held_response = Some(decoded);
                        _res_memory = Some(memory);
                    }
                    Held::Not { body, reason } => {
                        res.msg.meta.skipped = Some(reason);
                        res.msg.meta.truncated = reason == "too_large";
                        if reason == "too_large" && rule == BodyRule::Free {
                            rule = BodyRule::TooLarge;
                        }
                        response_body = body;
                    }
                }
            } else if res_class != BodyClass::None {
                res.msg.meta.skipped = Some("class");
            }
            let mut changed = false;
            let mut new_body: Option<Bytes> = None;
            let mut failed: Option<Response<Body>> = None;
            for e in &on_response {
                let call = Call::Response { req: request_view.clone(), res: res.clone(), body: rule };
                match self.call(&e.active, call, self.limits.intercept_time).await {
                    Ok(Output::Response { res: r, body }) => {
                        res = r;
                        changed = true;
                        if body.is_some() {
                            new_body = body;
                            rule = if status == StatusCode::PARTIAL_CONTENT { BodyRule::Partial } else { BodyRule::Free };
                        }
                    }
                    Ok(_) => {}
                    Err(msg) => {
                        script_error = Some(e.active.id().to_string());
                        if e.active.rule.on_error() == OnError::Fail {
                            failed = Some(failed_page(e.active.id(), &msg));
                            break;
                        }
                    }
                }
            }
            if let Some(page) = failed {
                let (p, b) = page.into_parts();
                rparts = p;
                response_body = b;
                res = ResponseInfo { status: rparts.status.as_u16(), msg: Message { headers: header_list(&rparts.headers), ..Message::default() } };
                res.msg.meta = BodyMeta::from_headers(&res.msg.headers, "content-range", BodyClass::Text);
            } else if changed {
                rparts.status = StatusCode::from_u16(res.status).unwrap_or(rparts.status);
                rparts.headers = header_map(&res.msg.headers);
                if let Some(b) = &new_body {
                    for h in [header::CONTENT_LENGTH, header::CONTENT_ENCODING, header::TRANSFER_ENCODING] {
                        rparts.headers.remove(h);
                    }
                    rparts.headers.insert(header::CONTENT_LENGTH, HeaderValue::from(b.len()));
                    response_body = full(b.clone());
                    res.msg.meta.encoding = None;
                    res.msg.meta.size = Some(b.len() as u64);
                    held_response = Some(b.clone());
                }
            }
        }
        let res_class = res.msg.meta.class.unwrap_or(BodyClass::None);
        let res_encoding = Encoding::of(text_header(&rparts.headers, header::CONTENT_ENCODING).as_deref());

        // ---- event streams and log rules on the response body
        // A body in an encoding the daemon cannot decode is passed as it came.
        let readable = res_encoding != Encoding::Unknown;
        let on_event: Vec<Arc<Entry>> = if scripted || upgraded || res_class != BodyClass::Events || !readable {
            vec![]
        } else {
            m.intercept.iter().filter(|e| e.info().is_some_and(|i| i.on_event)).cloned().collect()
        };
        let mut res_plan = Plan::for_side(
            &m.log,
            res_class,
            &info.id,
            "res",
            res.msg.meta.content_type.as_deref(),
            rparts.status == StatusCode::PARTIAL_CONTENT,
        );
        if !on_event.is_empty() && res_encoding != Encoding::Identity {
            // The events are written back decoded.
            rparts.headers.remove(header::CONTENT_ENCODING);
            rparts.headers.remove(header::CONTENT_LENGTH);
        }
        if !on_event.is_empty() {
            rparts.headers.remove(header::CONTENT_LENGTH);
        }
        let res_head = Arc::new(ResponseInfo { status: rparts.status.as_u16(), msg: Message { headers: header_list(&rparts.headers), body: None, meta: res.msg.meta.clone() } });
        if !m.log.is_empty() {
            res_plan.log_events =
                m.log.iter().filter(|e| readable && res_class == BodyClass::Events && e.info().is_some_and(|i| i.on_event)).cloned().collect();
        }

        let finish = Finish {
            scripts: self.clone(),
            log: m.log.clone(),
            request: info,
            req_class,
            res_head: res_head.clone(),
            time_ms,
            started,
            error: gateway_error,
            answered_by,
            upgraded,
            _memory: [_held_memory, _res_memory].into_iter().flatten().collect(),
        };
        let response = if upgraded || (m.log.is_empty() && on_event.is_empty()) {
            if !m.log.is_empty() {
                tokio::spawn(async move {
                    let req_side = request_side.await;
                    finish.done(req_side, SideResult { skipped: Some("upgrade"), ..Default::default() });
                });
            }
            Response::from_parts(rparts, response_body)
        } else if let Some(b) = held_response.filter(|_| on_event.is_empty() && res_plan.log_events.is_empty()) {
            let wire = res.msg.meta.size.unwrap_or(b.len() as u64);
            let side = self.side_from_held(b, res_plan, wire);
            tokio::spawn(async move {
                let side = side.await;
                let req_side = request_side.await;
                finish.done(req_side, side);
            });
            Response::from_parts(rparts, response_body)
        } else {
            let events = EventSetup {
                intercept: on_event,
                req: request_view,
                res: res_head,
                format: Format::of(res.msg.meta.content_type.as_deref()),
            };
            let (body, rx) = self.tap(response_body, res_plan, res_encoding, Some(events));
            tokio::spawn(async move {
                let res_side = rx.await.unwrap_or_default();
                // A request body still streaming after the response ended
                // (a server that answered early) is not waited for long.
                let req_side = tokio::time::timeout(Duration::from_secs(5), request_side).await.unwrap_or_default();
                finish.done(req_side, res_side);
            });
            Response::from_parts(rparts, body)
        };
        Outcome { response, rules: ids, script_error }
    }

    /// Hold a body until it ends, up to the hold limit after decoding.
    async fn hold(&self, mut body: Body, encoding: Encoding) -> Held {
        let mut chunks: VecDeque<Bytes> = VecDeque::new();
        let mut total = 0usize;
        let Some(mut memory) = self.budget.try_reserve(0) else { return Held::Not { body, reason: "budget" } };
        loop {
            match body.frame().await {
                None => break,
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        total += data.len();
                        let over = total > self.limits.hold;
                        let no_room = !over && !memory.grow(data.len());
                        chunks.push_back(data);
                        if over || no_room {
                            return Held::Not { body: chain(chunks, Some(body), None), reason: if over { "too_large" } else { "budget" } };
                        }
                    }
                }
                // The body broke while it was held (the client went away):
                // it goes on as it came, error included.
                Some(Err(e)) => return Held::Not { body: chain(chunks, None, Some(e)), reason: "error" },
            }
        }
        let raw: Bytes = if chunks.len() == 1 { chunks.pop_front().unwrap_or_default() } else { chunks.iter().flat_map(|c| c.iter().copied()).collect::<Vec<u8>>().into() };
        let decoded = match encoding {
            Encoding::Identity | Encoding::Unknown => raw.clone(),
            enc => match bodies::decode_all(enc, &raw, self.limits.hold) {
                Ok((_, true)) => return Held::Not { body: full(raw), reason: "too_large" },
                Ok((d, false)) => {
                    if !memory.grow(d.len()) {
                        return Held::Not { body: full(raw), reason: "budget" };
                    }
                    Bytes::from(d)
                }
                // A body that does not decode is given as it is.
                Err(_) => raw.clone(),
            },
        };
        self.stats.held_bytes.fetch_add(raw.len() as u64, Ordering::Relaxed);
        Held::Whole { raw, decoded, memory }
    }

    /// A side whose whole body is known (held, or set by a script). The
    /// bytes are in memory already, so a save has no queue limit.
    fn side_from_held(self: &Arc<Self>, decoded: Bytes, plan: Plan, wire: u64) -> SideFuture {
        let this = self.clone();
        Box::pin(async move {
            let mut side = SideResult { size: wire, ..Default::default() };
            if plan.copy {
                let n = decoded.len().min(this.limits.copy);
                match this.budget.try_reserve(n) {
                    Some(r) => {
                        side.copy = Some(decoded.slice(..n));
                        side.copy_truncated = n < decoded.len();
                        side.memory = Some(r);
                        this.stats.copied_bytes.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    None => side.copy_skipped = Some("budget"),
                }
            }
            for target in &plan.saves {
                let mut saver = Saver::start(&this, target, Encoding::Identity, usize::MAX);
                saver.push(&decoded);
                side.saves.push(saver.finish().await);
            }
            side
        })
    }

    /// Pass a body to the client while copying it, saving it and splitting
    /// it into events. Each part goes to the client first (I2); a log rule
    /// never makes the client wait. With intercept `on_event` rules, each
    /// event waits for those calls only (I21).
    fn tap(self: &Arc<Self>, mut body: Body, plan: Plan, encoding: Encoding, events: Option<EventSetup>) -> (Body, oneshot::Receiver<SideResult>) {
        let (tx, rx) = mpsc::channel::<Result<Frame<Bytes>, BoxError>>(4);
        let (done_tx, done_rx) = oneshot::channel();
        let this = self.clone();
        tokio::spawn(async move {
            let limits = &this.limits;
            let intercept = events.as_ref().map(|e| e.intercept.clone()).unwrap_or_default();
            let format = events.as_ref().and_then(|e| e.format);
            let rewrite = !intercept.is_empty() && format.is_some();
            let split = format.is_some() && (rewrite || !plan.log_events.is_empty());
            // What the client gets is decoded when events are written back.
            let copy_encoding = if rewrite { Encoding::Identity } else { encoding };
            let mut copier = plan.copy.then(|| Copier::new(copy_encoding, limits.copy, this.budget.clone()));
            let mut savers: Vec<Saver> = plan.saves.iter().map(|t| Saver::start(&this, t, copy_encoding, limits.save_queue)).collect();
            let mut splitter = split.then(|| Splitter::new(format.unwrap_or(Format::Sse)));
            let sink = SharedSink::default();
            let mut decoder = if split && encoding != Encoding::Identity { Decoder::new(encoding, sink.clone()).ok() } else { None };
            let mut side = SideResult::default();
            let mut index = 0u64;
            let mut skip_next = false;
            let ex_head = events.as_ref().map(|e| Arc::new(Exchange { request: (*e.req).clone(), response: (*e.res).clone(), ..Exchange::default() }));
            let mut cut = false;

            loop {
                let next = tokio::select! {
                    f = body.frame() => f,
                    // The client went away while the server sent nothing:
                    // stop now, as hyper would without a rule.
                    _ = tx.closed() => {
                        side.error = Some("the client closed the connection".into());
                        cut = true;
                        break;
                    }
                };
                let frame = match next {
                    None => break,
                    Some(Ok(f)) => f,
                    Some(Err(e)) => {
                        side.error = Some(format!("the body ended early: {e}"));
                        let _ = tx.send(Err(e)).await;
                        break;
                    }
                };
                let data = match frame.into_data() {
                    Ok(d) => d,
                    Err(trailers) => {
                        let _ = tx.send(Ok(trailers)).await;
                        continue;
                    }
                };
                side.size += data.len() as u64;
                if !rewrite && tx.send(Ok(Frame::data(data.clone()))).await.is_err() {
                    side.error = Some("the client closed the connection".into());
                    break;
                }
                let mut found: Vec<Event> = vec![];
                let mut cut_an_event = false;
                if let Some(s) = splitter.as_mut() {
                    let decoded: Bytes = match decoder.as_mut() {
                        Some(d) => {
                            let _ = d.write_all(&data);
                            sink.take()
                        }
                        None => data.clone(),
                    };
                    found = s.push(&decoded);
                    if s.pending() > limits.event {
                        // Pass the start of a too-large event as it came. Its
                        // rest ends in a later part: that event is skipped too.
                        found.push(Event { raw: s.take_pending(), comment: true, ..Event::default() });
                        if !skip_next {
                            side.events_skipped += 1;
                        }
                        cut_an_event = true;
                    }
                }
                let mut sent_now: Vec<Bytes> = vec![];
                for mut ev in found {
                    if skip_next {
                        // The rest of an event that was too large: it passes
                        // as it came (it was counted when it was cut).
                        skip_next = false;
                        ev.comment = true;
                    }
                    if ev.raw.len() > limits.event && !ev.comment {
                        side.events_skipped += 1;
                        ev.comment = true;
                    }
                    let out = if rewrite && !ev.comment {
                        match this.run_event(&intercept, events.as_ref(), ev).await {
                            EventResult::Send(e) => Some(e),
                            EventResult::Drop => None,
                            EventResult::Cut(msg) => {
                                side.error = Some(msg);
                                cut = true;
                                break;
                            }
                        }
                    } else {
                        Some(ev)
                    };
                    let Some(ev) = out else { continue };
                    let bytes = if ev.raw.is_empty() { ev.encode(format.unwrap_or(Format::Sse)) } else { ev.raw.clone() };
                    if rewrite {
                        if tx.send(Ok(Frame::data(bytes.clone()))).await.is_err() {
                            side.error = Some("the client closed the connection".into());
                            cut = true;
                            break;
                        }
                        sent_now.push(bytes);
                    }
                    if !ev.comment && let Some(head) = &ex_head {
                        index += 1;
                        this.queue_event(&plan.log_events, head, &ev, index);
                    }
                }
                if cut {
                    break;
                }
                if cut_an_event {
                    skip_next = true;
                }
                let copied: Vec<Bytes> = if rewrite { sent_now } else { vec![data] };
                for chunk in &copied {
                    if let Some(c) = copier.as_mut() {
                        c.push(chunk);
                    }
                    for s in savers.iter_mut() {
                        s.push(chunk);
                    }
                }
            }
            if !cut && let Some(s) = splitter.as_mut() {
                let rest = decoder.take().and_then(|d| d.finish().ok()).map(|s| s.take());
                let mut last: Vec<Event> = rest.map(|r| s.push(&r)).unwrap_or_default();
                last.extend(s.finish());
                for mut ev in last {
                    if skip_next {
                        skip_next = false;
                        ev.comment = true;
                    }
                    let out = if rewrite && !ev.comment {
                        match this.run_event(&intercept, events.as_ref(), ev).await {
                            EventResult::Send(e) => Some(e),
                            EventResult::Drop => None,
                            EventResult::Cut(msg) => {
                                side.error = Some(msg);
                                cut = true;
                                break;
                            }
                        }
                    } else {
                        Some(ev)
                    };
                    let Some(ev) = out else { continue };
                    let bytes = if ev.raw.is_empty() { ev.encode(format.unwrap_or(Format::Sse)) } else { ev.raw.clone() };
                    if rewrite {
                        let _ = tx.send(Ok(Frame::data(bytes.clone()))).await;
                        if let Some(c) = copier.as_mut() {
                            c.push(&bytes);
                        }
                        for s in savers.iter_mut() {
                            s.push(&bytes);
                        }
                    }
                    if !ev.comment && let Some(head) = &ex_head {
                        index += 1;
                        this.queue_event(&plan.log_events, head, &ev, index);
                    }
                }
            }
            if cut {
                let _ = tx.send(Err("a script rule cut the stream".into())).await;
            }
            drop(tx);
            if let Some(c) = copier {
                let (copy, truncated, skipped, memory) = c.finish();
                this.stats.copied_bytes.fetch_add(copy.as_ref().map_or(0, |c| c.len() as u64), Ordering::Relaxed);
                side.copy = copy;
                side.copy_truncated = truncated;
                side.copy_skipped = skipped;
                side.memory = memory;
            }
            for s in savers {
                side.saves.push(s.finish().await);
            }
            let _ = done_tx.send(side);
        });
        (ChannelBody { rx }.boxed_unsync(), done_rx)
    }

    async fn run_event(&self, intercept: &[Arc<Entry>], setup: Option<&EventSetup>, ev: Event) -> EventResult {
        let Some(setup) = setup else { return EventResult::Send(ev) };
        let mut ev = ev;
        for e in intercept {
            let call = Call::Event { req: setup.req.clone(), res: setup.res.clone(), event: ev.clone() };
            match self.call(&e.active, call, self.limits.intercept_time).await {
                Ok(Output::Event(Some(next))) => ev = next,
                Ok(Output::Event(None)) => return EventResult::Drop,
                Ok(_) => {}
                Err(msg) => {
                    if e.active.rule.on_error() == OnError::Fail {
                        return EventResult::Cut(format!("script rule {} failed: {msg}", e.active.id()));
                    }
                }
            }
        }
        EventResult::Send(ev)
    }

    fn queue_event(&self, rules: &[Arc<Entry>], head: &Arc<Exchange>, ev: &Event, index: u64) {
        if rules.is_empty() {
            return;
        }
        let n = ev.data.len();
        let Some(memory) = self.budget.try_reserve(n) else {
            for e in rules {
                e.active.counters.dropped.fetch_add(1, Ordering::Relaxed);
            }
            return;
        };
        let memory = Arc::new(memory);
        for e in rules {
            let call = Call::LogEvent { ex: head.clone(), event: ev.clone(), index };
            self.enqueue(e, call, n, Some(memory.clone()));
        }
    }
}

fn view(a: &ActiveRule) -> ScriptRuleView {
    let loaded = a.loaded();
    let c = &a.counters;
    ScriptRuleView {
        rule: ScriptRule { enabled: a.is_enabled(), ..a.rule.clone() },
        kind: loaded.as_ref().map(|l| l.info.kind),
        matched: c.matched.load(Ordering::Relaxed),
        answered: c.answered.load(Ordering::Relaxed),
        errors: c.errors.load(Ordering::Relaxed),
        last_error: a.last_error(),
        dropped: c.dropped.load(Ordering::Relaxed),
        bytes_written: c.written.load(Ordering::Relaxed),
        script_loaded_at: loaded.as_ref().map(|l| l.loaded_at),
        script_sha256: loaded.as_ref().map(|l| l.sha256.clone()),
    }
}

enum Held {
    Whole { raw: Bytes, decoded: Bytes, memory: Reservation },
    /// Not held: the body as it came, from its first byte.
    Not { body: Body, reason: &'static str },
}

enum EventResult {
    Send(Event),
    Drop,
    Cut(String),
}

struct EventSetup {
    intercept: Vec<Arc<Entry>>,
    req: Arc<RequestInfo>,
    res: Arc<ResponseInfo>,
    format: Option<Format>,
}

/// What the log rules want of one side's body.
#[derive(Clone, Default)]
struct Plan {
    /// Some log rule copies this class.
    copy: bool,
    saves: Vec<SaveTarget>,
    log_events: Vec<Arc<Entry>>,
}

#[derive(Clone)]
struct SaveTarget {
    entry: Arc<Entry>,
    /// `bodies/<id>-res[.part].<ext>`, inside `output_dir`.
    file: String,
}

impl Plan {
    /// `side` is `req` or `res`; `partial` marks a `206` or a request with
    /// `Content-Range`, whose saved file name gets `.part`.
    fn for_side(log: &[Arc<Entry>], class: BodyClass, id: &str, side: &str, content_type: Option<&str>, partial: bool) -> Plan {
        if class == BodyClass::None {
            return Plan::default();
        }
        let mut plan = Plan::default();
        let part = if partial { ".part" } else { "" };
        let file = format!("{id}-{side}{part}.{}", bodies::extension(content_type, class));
        for e in log {
            let Some(info) = e.info() else { continue };
            // A stream given to on_event is not also copied (section 3.4).
            if info.copy.contains(class) && !(class == BodyClass::Events && info.on_event && side == "res") {
                plan.copy = true;
            }
            if info.save.contains(class) && e.active.capture().is_some() {
                plan.saves.push(SaveTarget { entry: e.clone(), file: file.clone() });
            }
        }
        plan
    }

    fn wants_bytes(&self) -> bool {
        self.copy || !self.saves.is_empty()
    }
}

/// What a side's body gave the log rules.
#[derive(Default)]
struct SideResult {
    size: u64,
    copy: Option<Bytes>,
    copy_truncated: bool,
    copy_skipped: Option<&'static str>,
    saves: Vec<SaveResult>,
    events_skipped: u64,
    error: Option<String>,
    skipped: Option<&'static str>,
    memory: Option<Reservation>,
}

struct SaveResult {
    rule: String,
    file: Option<String>,
    truncated: bool,
    skipped: Option<&'static str>,
}

type SideFuture = Pin<Box<dyn Future<Output = SideResult> + Send>>;

/// The end of an exchange: build each log rule's copy and queue it.
struct Finish {
    scripts: Arc<Scripts>,
    log: Vec<Arc<Entry>>,
    request: RequestInfo,
    req_class: BodyClass,
    res_head: Arc<ResponseInfo>,
    time_ms: u64,
    started: Instant,
    error: Option<String>,
    answered_by: Option<String>,
    upgraded: bool,
    /// Held bodies stay in the budget while the request view still holds
    /// them, until the exchange ends (I17).
    _memory: Vec<Reservation>,
}

impl Finish {
    fn done(self, mut req: SideResult, mut res: SideResult) {
        if self.log.is_empty() {
            return;
        }
        let duration_ms = self.started.elapsed().as_millis() as u64;
        let res_class = self.res_head.msg.meta.class.unwrap_or(BodyClass::None);
        let mut memory: Option<Reservation> = None;
        for m in [req.memory.take(), res.memory.take()].into_iter().flatten() {
            match memory.as_mut() {
                Some(r) => r.absorb(m),
                None => memory = Some(m),
            }
        }
        let bytes = req.copy.as_ref().map_or(0, Bytes::len) + res.copy.as_ref().map_or(0, Bytes::len);
        let memory = memory.map(Arc::new);
        let error = self.error.clone().or(res.error.clone());
        for e in &self.log {
            let Some(info) = e.info() else { continue };
            let mut request = self.request.clone();
            fill_side(&mut request.msg, self.req_class, &req, info.copy.contains(self.req_class), e.active.id(), false);
            let mut msg = Message { headers: self.res_head.msg.headers.clone(), body: None, meta: self.res_head.msg.meta.clone() };
            let streamed = res_class == BodyClass::Events && info.on_event;
            fill_side(&mut msg, res_class, &res, info.copy.contains(res_class), e.active.id(), streamed);
            if self.upgraded {
                msg.meta.skipped = Some("upgrade");
            }
            let ex = Exchange {
                request,
                response: ResponseInfo { status: self.res_head.status, msg },
                time_ms: self.time_ms,
                duration_ms,
                error: error.clone(),
                answered_by: self.answered_by.clone(),
                upgraded: self.upgraded,
            };
            self.scripts.enqueue(e, Call::Exchange(Box::new(ex)), bytes, memory.clone());
        }
    }
}

fn fill_side(msg: &mut Message, class: BodyClass, side: &SideResult, wants_copy: bool, rule: &str, streamed: bool) {
    let meta = &mut msg.meta;
    meta.class = Some(class);
    if class == BodyClass::None {
        meta.size = Some(0);
        msg.body = None;
        return;
    }
    if side.size > 0 || meta.size.is_none() {
        meta.size = Some(side.size);
    }
    meta.events_skipped = side.events_skipped;
    msg.body = None;
    if streamed {
        meta.skipped = Some("streamed");
    } else if wants_copy {
        match &side.copy {
            Some(c) => {
                msg.body = Some(c.clone());
                meta.truncated = side.copy_truncated;
                meta.skipped = None;
            }
            None => meta.skipped = side.copy_skipped.or(side.skipped).or(Some("budget")),
        }
    } else if meta.skipped.is_none() || meta.skipped == Some("class") {
        meta.skipped = Some("class");
    }
    if let Some(s) = side.saves.iter().find(|s| s.rule == rule) {
        meta.file = s.file.clone();
        meta.file_truncated = s.truncated;
        if s.skipped.is_some() {
            meta.skipped = s.skipped;
        } else if meta.skipped == Some("class") {
            meta.skipped = None;
        }
    }
}

/// A copy for log rules: decoded as it passes, up to the copy limit, inside
/// the memory budget.
struct Copier {
    decoder: Option<Decoder<LimitedVec>>,
    memory: Option<Reservation>,
    truncated: bool,
    skipped: Option<&'static str>,
    result: Option<Vec<u8>>,
}

impl Copier {
    fn new(encoding: Encoding, limit: usize, budget: Arc<Budget>) -> Self {
        let decoder = Decoder::new(encoding, LimitedVec::new(limit)).ok();
        Self { decoder, memory: budget.try_reserve(0), truncated: false, skipped: None, result: None }
    }

    fn push(&mut self, data: &[u8]) {
        let Some(d) = self.decoder.as_mut() else { return };
        let before = d.sink().data.len();
        let r = d.write_all(data);
        let after = d.sink().data.len();
        let grown = after - before;
        let room = self.memory.as_mut().is_some_and(|m| m.grow(grown));
        if !room {
            self.decoder = None;
            self.memory = None;
            self.skipped = Some("budget");
            return;
        }
        if r.is_err() {
            // Full, or a body that does not decode: keep what came out.
            self.truncated = d.sink().truncated;
            self.result = Some(std::mem::take(&mut d.sink_mut().data));
            self.decoder = None;
        }
    }

    fn finish(mut self) -> (Option<Bytes>, bool, Option<&'static str>, Option<Reservation>) {
        if self.skipped.is_some() {
            return (None, false, self.skipped, None);
        }
        if let Some(d) = self.decoder.take() {
            match d.finish() {
                Ok(sink) => {
                    self.truncated |= sink.truncated;
                    self.result = Some(sink.data);
                }
                Err(_) => self.result = self.result.take().or(Some(vec![])),
            }
        }
        (self.result.map(Bytes::from), self.truncated, None, self.memory)
    }
}

/// Saves one body to `output_dir/bodies/` through a small queue. A full
/// queue or the quota stops the save; the client is never slowed (I23).
struct Saver {
    rule: String,
    file: Option<String>,
    tx: Option<std::sync::mpsc::Sender<Bytes>>,
    queued: Arc<AtomicUsize>,
    limit: usize,
    skipped: Option<&'static str>,
    truncated: bool,
    done: Option<oneshot::Receiver<(bool, Option<&'static str>)>>,
}

impl Saver {
    /// `queue` is the most bytes waiting for the writer before the save stops.
    fn start(scripts: &Scripts, target: &SaveTarget, encoding: Encoding, queue: usize) -> Saver {
        let rule = target.entry.active.id().to_string();
        let limits = &scripts.limits;
        let mut saver = Saver { rule, file: None, tx: None, queued: Arc::default(), limit: queue, skipped: None, truncated: false, done: None };
        let Some(capture) = target.entry.active.capture().cloned() else { return saver };
        let name = if target.file.is_empty() { format!("{}-body.bin", new_id()) } else { target.file.clone() };
        let opened = bodies::bodies_dir(&capture.dir).and_then(|dir| {
            let path = dir.join(&name);
            bodies::create_new(&path).map(|f| (f, format!("bodies/{name}")))
        });
        let (file, rel) = match opened {
            Ok(x) => x,
            Err(e) => {
                scripts.fail(&target.entry.active, &format!("cannot save a body in {}/bodies: {e}", capture.dir.display()));
                saver.skipped = Some("save_error");
                return saver;
            }
        };
        saver.file = Some(rel);
        let (tx, rx) = std::sync::mpsc::channel::<Bytes>();
        let (done_tx, done_rx) = oneshot::channel();
        let queued = saver.queued.clone();
        let delay = limits.save_delay;
        tokio::task::spawn_blocking(move || {
            let sink = QuotaFile { file, capture, quota_hit: false };
            let mut decoder = match Decoder::new(encoding, sink) {
                Ok(d) => d,
                Err(_) => {
                    let _ = done_tx.send((true, Some("save_error")));
                    return;
                }
            };
            let mut stop: Option<&'static str> = None;
            while let Ok(chunk) = rx.recv() {
                queued.fetch_sub(chunk.len(), Ordering::AcqRel);
                if stop.is_some() {
                    continue;
                }
                if let Some(d) = delay {
                    std::thread::sleep(d);
                }
                if decoder.write_all(&chunk).is_err() {
                    stop = Some(if decoder.sink().quota_hit { "quota" } else { "save_error" });
                }
            }
            if stop.is_none()
                && let Err(_) = decoder.finish()
            {
                stop = Some("save_error");
            }
            let _ = done_tx.send((stop.is_some(), stop));
        });
        saver.tx = Some(tx);
        saver.done = Some(done_rx);
        saver
    }

    fn push(&mut self, data: &[u8]) {
        let Some(tx) = &self.tx else { return };
        if self.queued.load(Ordering::Acquire).saturating_add(data.len()) > self.limit {
            // The writer fell behind: stop, keep what it has.
            self.tx = None;
            self.truncated = true;
            self.skipped = Some("slow_disk");
            return;
        }
        self.queued.fetch_add(data.len(), Ordering::AcqRel);
        if tx.send(Bytes::copy_from_slice(data)).is_err() {
            self.tx = None;
        }
    }

    async fn finish(mut self) -> SaveResult {
        self.tx = None;
        if let Some(done) = self.done.take()
            && let Ok((truncated, why)) = done.await
        {
            self.truncated |= truncated;
            if self.skipped.is_none() {
                self.skipped = why;
            }
        }
        SaveResult { rule: self.rule, file: self.file, truncated: self.truncated, skipped: self.skipped }
    }
}

/// A saved-body file that counts every byte against the rule's quota.
struct QuotaFile {
    file: std::fs::File,
    capture: Arc<lua_api::CaptureTarget>,
    quota_hit: bool,
}

impl std::io::Write for QuotaFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let left = self.capture.quota.saturating_sub(self.capture.written.load(Ordering::Acquire));
        let n = buf.len().min(left as usize);
        if n == 0 && !buf.is_empty() {
            self.quota_hit = true;
            return Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "capture quota reached"));
        }
        if self.capture.take(n as u64).is_err() {
            self.quota_hit = true;
            return Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "capture quota reached"));
        }
        match self.file.write(&buf[..n]) {
            Ok(w) => {
                self.capture.give_back((n - w) as u64);
                Ok(w)
            }
            Err(e) => {
                self.capture.give_back(n as u64);
                Err(e)
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// A sink whose bytes the event splitter takes after each write.
#[derive(Clone, Default)]
struct SharedSink(Arc<Mutex<Vec<u8>>>);

impl SharedSink {
    fn take(&self) -> Bytes {
        Bytes::from(std::mem::take(&mut *self.0.lock().unwrap()))
    }
}

impl std::io::Write for SharedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A body fed by the tap task.
struct ChannelBody {
    rx: mpsc::Receiver<Result<Frame<Bytes>, BoxError>>,
}

impl hyper::body::Body for ChannelBody {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
        self.rx.poll_recv(cx)
    }

    fn size_hint(&self) -> SizeHint {
        // Unknown: events may be changed or dropped. A Content-Length the
        // response kept still holds, because the bytes are not changed then.
        SizeHint::new()
    }
}

/// Parts already read, then the rest of a body.
struct Chain {
    chunks: VecDeque<Bytes>,
    rest: Option<Body>,
    error: Option<BoxError>,
}

fn chain(chunks: VecDeque<Bytes>, rest: Option<Body>, error: Option<BoxError>) -> Body {
    Chain { chunks, rest, error }.boxed_unsync()
}

impl hyper::body::Body for Chain {
    type Data = Bytes;
    type Error = BoxError;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, BoxError>>> {
        if let Some(c) = self.chunks.pop_front() {
            return Poll::Ready(Some(Ok(Frame::data(c))));
        }
        if let Some(e) = self.error.take() {
            return Poll::Ready(Some(Err(e)));
        }
        match self.rest.as_mut() {
            Some(rest) => Pin::new(rest).poll_frame(cx),
            None => Poll::Ready(None),
        }
    }
}

fn full(b: Bytes) -> Body {
    Full::new(b).map_err(|never| match never {}).boxed_unsync()
}

fn text_header(h: &HeaderMap, name: HeaderName) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

fn header_list(h: &HeaderMap) -> Vec<(String, Bytes)> {
    h.iter().map(|(n, v)| (n.as_str().to_string(), Bytes::copy_from_slice(v.as_bytes()))).collect()
}

fn header_map(list: &[(String, Bytes)]) -> HeaderMap {
    let mut h = HeaderMap::with_capacity(list.len());
    for (n, v) in list {
        if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(n.as_bytes()), HeaderValue::from_maybe_shared(v.clone())) {
            h.append(n, v);
        }
    }
    h
}

fn script_response(r: ScriptResponse) -> Response<Body> {
    let mut resp = Response::new(full(r.body));
    *resp.status_mut() = StatusCode::from_u16(r.status).unwrap_or(StatusCode::OK);
    *resp.headers_mut() = header_map(&r.headers);
    resp
}

/// The page a client gets when an intercept rule with `on_error: "fail"` fails.
fn failed_page(rule: &str, error: &str) -> Response<Body> {
    page(
        StatusCode::BAD_GATEWAY,
        "A script rule failed",
        format!("<p>Script rule <code>{}</code> failed: <code>{}</code></p>", escape(rule), escape(error)),
    )
}

/// The host of a rule, as the inspect set takes it.
pub fn rule_host(rule: &ScriptRule) -> Option<String> {
    match RuleHost::parse(&rule.host).ok()? {
        RuleHost::Internet(p) => Some(p.to_string()),
        RuleHost::Local { .. } => None,
    }
}
