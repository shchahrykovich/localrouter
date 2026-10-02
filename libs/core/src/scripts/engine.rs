//! The Lua runtime (ADR 07, change 3): the sandbox, the limits, the script
//! threads, loading and reloading a script file, and one call.
//!
//! Network tasks never run Lua. They send a job to a small pool of operating
//! system threads and wait for the answer (I5).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;
use mlua::{ChunkMode, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use serde::{Deserialize, Serialize};

use crate::scripts::bodies::{BodyClass, Classes};
use crate::scripts::events::Event;
use crate::scripts::lua_api::{
    self, BodyRule, CaptureTarget, Exchange, InCall, LineLimiter, RequestInfo, ResponseInfo, ScriptResponse, StateCtx,
};
use crate::scripts::rules::ScriptRule;

/// Time per intercept call (`on_request`, `on_response`, `on_event`).
pub const INTERCEPT_TIME: Duration = Duration::from_millis(50);
/// Time per log call (`on_exchange`, `on_event`).
pub const LOG_TIME: Duration = Duration::from_secs(2);
/// Time for the top level of the file.
pub const LOAD_TIME: Duration = Duration::from_millis(100);
/// Memory per Lua state.
pub const MEMORY_LIMIT: usize = 64 << 20;
/// Size of a script file.
pub const MAX_SCRIPT_BYTES: u64 = 1 << 20;
/// Lua states per intercept rule.
pub const INTERCEPT_STATES: usize = 4;
/// A rule is turned off after this many failed calls in a row (I6).
pub const MAX_FAILURES_IN_A_ROW: u32 = 20;

const SCRIPT_KEY: &str = "localrouter.script";

/// The kind of a script, from its `kind` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptKind {
    Intercept,
    Log,
}

/// What a script declares in the table it returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptInfo {
    pub kind: ScriptKind,
    pub request_body: Classes,
    pub response_body: Classes,
    pub copy: Classes,
    pub save: Classes,
    pub on_request: bool,
    pub on_response: bool,
    pub on_event: bool,
    pub on_exchange: bool,
}

/// Which file a loaded version came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileId {
    modified: Option<SystemTime>,
    size: u64,
    inode: u64,
}

fn file_id(path: &Path) -> std::io::Result<FileId> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path)?;
    Ok(FileId { modified: m.modified().ok(), size: m.len(), inode: m.ino() })
}

/// One loaded, checked version of a script file.
#[derive(Debug)]
pub struct Loaded {
    pub info: ScriptInfo,
    source: Bytes,
    /// The file name, as Lua messages show it: `capture.lua`.
    pub name: String,
    pub sha256: String,
    pub loaded_at: u64,
    file: FileId,
    pub version: u64,
}

static VERSIONS: AtomicU64 = AtomicU64::new(1);

/// Read, compile and run the top level of a script file in a fresh sandbox,
/// then check the table it returns (ADR 07, change 3, loading). The error is
/// the Lua message with the file name and line.
pub fn load_file(path: &Path) -> Result<Loaded, String> {
    let shown = path.display();
    let file = file_id(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("the script file is gone: {shown}"),
        std::io::ErrorKind::PermissionDenied => format!(
            "cannot read {shown}: {e}. macOS keeps apps out of Desktop, Documents and Downloads: keep scripts elsewhere"
        ),
        _ => format!("cannot read {shown}: {e}"),
    })?;
    if file.size > MAX_SCRIPT_BYTES {
        return Err(format!("{shown} is larger than 1 MB"));
    }
    let source = std::fs::read(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
    if source.starts_with(b"\x1bLua") {
        return Err(format!("{shown} is precompiled Lua bytecode; give the .lua text"));
    }
    let name = path.file_name().map_or_else(|| shown.to_string(), |n| n.to_string_lossy().into_owned());
    let check = StateCtx { rule_id: "check".into(), log: true, capture: None, lines: Arc::default() };
    let (_lua, table) = new_state(&source, &name, &check)?;
    let info = read_info(&table, &name)?;
    let digest = ring::digest::digest(&ring::digest::SHA256, &source);
    Ok(Loaded {
        info,
        source: Bytes::from(source),
        name,
        sha256: digest.as_ref().iter().map(|b| format!("{b:02x}")).collect(),
        loaded_at: crate::logs::now_ms(),
        file,
        version: VERSIONS.fetch_add(1, Ordering::Relaxed),
    })
}

struct Deadline(Instant);

/// A Lua state with only the allowed libraries (I4), the modules, the memory
/// limit and the time hook, after its top level ran. Returns the state and
/// the table the file returned.
fn new_state(source: &[u8], name: &str, ctx: &StateCtx) -> Result<(Lua, Table), String> {
    let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE | StdLib::OS;
    let lua = Lua::new_with(libs, LuaOptions::default()).map_err(|e| e.to_string())?;
    lua.set_memory_limit(MEMORY_LIMIT).map_err(|e| e.to_string())?;
    sandbox(&lua).map_err(|e| format!("cannot build the sandbox: {e}"))?;
    lua_api::install(&lua, ctx).map_err(|e| format!("cannot build the sandbox: {e}"))?;
    lua.set_app_data(InCall(false));
    lua.set_app_data(Deadline(Instant::now() + LOAD_TIME));
    lua.set_hook(HookTriggers::new().every_nth_instruction(1000), |lua, _| {
        match lua.app_data_ref::<Deadline>() {
            Some(d) if Instant::now() > d.0 => Err(mlua::Error::runtime("time limit: the script ran too long")),
            _ => Ok(VmState::Continue),
        }
    })
    .map_err(|e| e.to_string())?;
    let value: Value = lua
        .load(source)
        .set_name(format!("@{name}"))
        .set_mode(ChunkMode::Text)
        .call(())
        .map_err(|e| message(&e))?;
    let Value::Table(table) = value else {
        return Err(format!("{name} must end with `return {{ kind = \"intercept\" or \"log\", ... }}`; it returned a {}", value.type_name()));
    };
    lua.set_named_registry_value(SCRIPT_KEY, &table).map_err(|e| e.to_string())?;
    Ok((lua, table))
}

/// Remove what a script must not reach: files, other code, the debug
/// library, `string.dump`, and every `os` function but time, date and clock.
fn sandbox(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    for name in ["dofile", "loadfile", "load", "loadstring", "require", "package", "io", "debug"] {
        g.raw_set(name, Value::Nil)?;
    }
    let string: Table = g.get("string")?;
    string.raw_set("dump", Value::Nil)?;
    let os: Table = g.get("os")?;
    let safe = lua.create_table()?;
    for name in ["time", "date", "clock"] {
        safe.raw_set(name, os.get::<Value>(name)?)?;
    }
    g.raw_set("os", safe)?;
    let gc: mlua::Function = g.get("collectgarbage")?;
    g.raw_set(
        "collectgarbage",
        lua.create_function(move |_, opt: Option<String>| {
            if opt.as_deref() == Some("count") {
                gc.call::<Value>("count")
            } else {
                Err(mlua::Error::runtime("collectgarbage: only \"count\" is allowed"))
            }
        })?,
    )?;
    Ok(())
}

/// A Lua error as one line with the file and line: `capture.lua:12: ...`.
pub fn message(e: &mlua::Error) -> String {
    match e {
        mlua::Error::RuntimeError(m) | mlua::Error::SyntaxError { message: m, .. } => first_line(m),
        mlua::Error::MemoryError(_) => format!("memory limit: the script used more than {} MB", MEMORY_LIMIT >> 20),
        mlua::Error::CallbackError { cause, .. } => message(cause),
        mlua::Error::WithContext { cause, .. } => message(cause),
        other => first_line(&other.to_string()),
    }
}

fn first_line(m: &str) -> String {
    let line = m.lines().next().unwrap_or(m).trim();
    line.strip_prefix("runtime error: ").unwrap_or(line).to_string()
}

fn classes(table: &Table, field: &str, name: &str, default: Classes) -> Result<Classes, String> {
    match table.raw_get::<Value>(field).map_err(|e| e.to_string())? {
        Value::Nil => Ok(default),
        Value::Boolean(false) => Ok(Classes::NONE),
        Value::Boolean(true) => Ok(Classes::of(&[BodyClass::Text])),
        Value::Table(list) => {
            let mut out = vec![];
            for item in list.sequence_values::<Value>() {
                let item = item.map_err(|e| e.to_string())?;
                let text = match &item {
                    Value::String(s) => s.to_string_lossy(),
                    other => return Err(format!("{name}: {field} lists a {}, not a class name", other.type_name())),
                };
                let class = BodyClass::parse(&text).ok_or_else(|| {
                    format!("{name}: {field} lists {text:?}; the classes are text, events, media, multipart and binary")
                })?;
                out.push(class);
            }
            Ok(Classes::of(&out))
        }
        other => Err(format!("{name}: {field} must be a list of classes, true or false, not a {}", other.type_name())),
    }
}

fn has_function(table: &Table, field: &str, name: &str) -> Result<bool, String> {
    match table.raw_get::<Value>(field).map_err(|e| e.to_string())? {
        Value::Nil => Ok(false),
        Value::Function(_) => Ok(true),
        other => Err(format!("{name}: {field} must be a function, not a {}", other.type_name())),
    }
}

/// Check the table a script returned (ADR 07, changes 1 and 4).
fn read_info(table: &Table, name: &str) -> Result<ScriptInfo, String> {
    let kind = match table.raw_get::<Value>("kind").map_err(|e| e.to_string())? {
        Value::String(s) if s.to_string_lossy() == "intercept" => ScriptKind::Intercept,
        Value::String(s) if s.to_string_lossy() == "log" => ScriptKind::Log,
        Value::Nil => return Err(format!("{name}: the returned table has no kind; set kind = \"intercept\" or \"log\"")),
        other => return Err(format!("{name}: kind must be \"intercept\" or \"log\", not {}", other.to_string().unwrap_or_default())),
    };
    let f = |field| has_function(table, field, name);
    let present = |field: &str| table.raw_get::<Value>(field).map(|v| !v.is_nil()).unwrap_or(false);
    let info = match kind {
        ScriptKind::Intercept => {
            for field in ["copy", "save", "on_exchange"] {
                if present(field) {
                    return Err(format!("{name}: {field} is for log scripts; this one has kind = \"intercept\""));
                }
            }
            let info = ScriptInfo {
                kind,
                request_body: classes(table, "request_body", name, Classes::NONE)?,
                response_body: classes(table, "response_body", name, Classes::NONE)?,
                copy: Classes::NONE,
                save: Classes::NONE,
                on_request: f("on_request")?,
                on_response: f("on_response")?,
                on_event: f("on_event")?,
                on_exchange: false,
            };
            if !(info.on_request || info.on_response || info.on_event) {
                return Err(format!("{name}: an intercept script needs on_request, on_response or on_event"));
            }
            if info.request_body.contains(BodyClass::Events) || info.response_body.contains(BodyClass::Events) {
                return Err(format!("{name}: events are never held; use on_event to see each event of a stream"));
            }
            info
        }
        ScriptKind::Log => {
            for field in ["request_body", "response_body", "on_request", "on_response"] {
                if present(field) {
                    return Err(format!(
                        "{name}: {field} is for intercept scripts; a log script lists copy and save instead"
                    ));
                }
            }
            let info = ScriptInfo {
                kind,
                request_body: Classes::NONE,
                response_body: Classes::NONE,
                copy: classes(table, "copy", name, Classes::of(&[BodyClass::Text, BodyClass::Events]))?,
                save: classes(table, "save", name, Classes::NONE)?,
                on_request: false,
                on_response: false,
                on_event: f("on_event")?,
                on_exchange: f("on_exchange")?,
            };
            if !info.on_exchange {
                return Err(format!("{name}: a log script needs on_exchange(ex)"));
            }
            if info.copy.overlaps(info.save) {
                let both: Vec<_> = BodyClass::ALL.into_iter().filter(|c| info.copy.contains(*c) && info.save.contains(*c)).map(BodyClass::as_str).collect();
                return Err(format!("{name}: {} is in both copy and save; a class is copied or saved, not both", both.join(", ")));
            }
            info
        }
    };
    Ok(info)
}

// ---- the script threads

type Job = Box<dyn FnOnce() + Send>;

/// A few operating system threads that run Lua (the number of cores, at
/// most 4).
pub struct Pool {
    tx: Mutex<mpsc::Sender<Job>>,
}

impl Pool {
    /// `name` names the threads, for example `lua` or `lua-log`.
    pub fn new(name: &str) -> Self {
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4);
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..threads {
            let rx = rx.clone();
            let spawned = std::thread::Builder::new().name(format!("{name}-{i}")).spawn(move || {
                loop {
                    let job = match rx.lock().unwrap().recv() {
                        Ok(job) => job,
                        Err(_) => return,
                    };
                    job();
                }
            });
            if let Err(e) = spawned {
                tracing::error!("cannot start a script thread: {e}");
            }
        }
        Self { tx: Mutex::new(tx) }
    }

    /// Run `f` on a script thread; the answer arrives on the receiver.
    pub fn run<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> tokio::sync::oneshot::Receiver<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job: Job = Box::new(move || {
            let _ = tx.send(f());
        });
        let _ = self.tx.lock().unwrap().send(job);
        rx
    }
}

// ---- one rule at run time

/// The time and message of a rule's last failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastError {
    pub message: String,
    pub time_ms: u64,
}

/// Counters of a rule, in memory, from when it was set or the daemon started.
#[derive(Debug, Default)]
pub struct Counters {
    pub matched: AtomicU64,
    pub answered: AtomicU64,
    pub errors: AtomicU64,
    pub dropped: AtomicU64,
    /// Bytes written to capture files and saved bodies.
    pub written: Arc<AtomicU64>,
    in_a_row: AtomicU32,
}

/// One call into a script.
pub enum Call {
    Request { req: RequestInfo, body: BodyRule },
    Response { req: Arc<RequestInfo>, res: ResponseInfo, body: BodyRule },
    Event { req: Arc<RequestInfo>, res: Arc<ResponseInfo>, event: Event },
    Exchange(Box<Exchange>),
    LogEvent { ex: Arc<Exchange>, event: Event, index: u64 },
}

/// What a call gave back.
pub enum Output {
    Request { req: Box<RequestInfo>, body: Option<Bytes>, response: Option<ScriptResponse> },
    Response { res: ResponseInfo, body: Option<Bytes> },
    /// `None`: the script dropped the event.
    Event(Option<Event>),
    Done,
}

/// A rule in the table, with its loaded script, its Lua states and its
/// counters.
pub struct ActiveRule {
    pub rule: ScriptRule,
    script: RwLock<Option<Arc<Loaded>>>,
    last_check: Mutex<Instant>,
    /// Free states, each with the version of the script it runs.
    states: Mutex<Vec<(u64, Lua)>>,
    /// Calls that may run at the same moment: 4 for intercept, 1 for log.
    pub permits: tokio::sync::Semaphore,
    pub enabled: AtomicBool,
    pub counters: Counters,
    last_error: Mutex<Option<LastError>>,
    capture: Option<Arc<CaptureTarget>>,
    lines: Arc<LineLimiter>,
}

impl ActiveRule {
    /// A rule with its first loaded version, or with none (a saved rule whose
    /// file did not load: it stays in the list and matches nothing).
    pub fn new(rule: ScriptRule, loaded: Result<Loaded, String>, counters: Option<&ActiveRule>) -> Self {
        let (script, error) = match loaded {
            Ok(l) => (Some(Arc::new(l)), None),
            Err(e) => (None, Some(LastError { message: e, time_ms: crate::logs::now_ms() })),
        };
        let kind = script.as_ref().map(|s| s.info.kind);
        let written = counters.map_or_else(Arc::default, |o| o.counters.written.clone());
        let capture = rule.output_dir.as_ref().filter(|_| kind == Some(ScriptKind::Log)).map(|dir| {
            Arc::new(CaptureTarget { dir: PathBuf::from(dir), quota: rule.max_capture_bytes(), written: written.clone() })
        });
        let permits = if kind == Some(ScriptKind::Log) { 1 } else { INTERCEPT_STATES };
        let this = Self {
            enabled: AtomicBool::new(rule.enabled),
            rule,
            script: RwLock::new(script),
            last_check: Mutex::new(Instant::now()),
            states: Mutex::new(vec![]),
            permits: tokio::sync::Semaphore::new(permits),
            counters: Counters { written, ..Counters::default() },
            last_error: Mutex::new(error),
            capture,
            lines: Arc::default(),
        };
        if let Some(old) = counters {
            for (a, b) in [
                (&this.counters.matched, &old.counters.matched),
                (&this.counters.answered, &old.counters.answered),
                (&this.counters.errors, &old.counters.errors),
                (&this.counters.dropped, &old.counters.dropped),
            ] {
                a.store(b.load(Ordering::Relaxed), Ordering::Relaxed);
            }
        }
        this
    }

    pub fn id(&self) -> &str {
        &self.rule.id
    }

    /// The loaded version, if any.
    pub fn loaded(&self) -> Option<Arc<Loaded>> {
        self.script.read().unwrap().clone()
    }

    pub fn info(&self) -> Option<ScriptInfo> {
        self.loaded().map(|l| l.info)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn last_error(&self) -> Option<LastError> {
        self.last_error.lock().unwrap().clone()
    }

    pub fn capture(&self) -> Option<&Arc<CaptureTarget>> {
        self.capture.as_ref()
    }

    fn set_error(&self, message: String) {
        *self.last_error.lock().unwrap() = Some(LastError { message, time_ms: crate::logs::now_ms() });
    }

    /// Count a failed call. Returns true when this failure turned the rule
    /// off (I6).
    pub fn failed(&self, message: &str) -> bool {
        self.counters.errors.fetch_add(1, Ordering::Relaxed);
        let in_a_row = self.counters.in_a_row.fetch_add(1, Ordering::AcqRel) + 1;
        if in_a_row >= MAX_FAILURES_IN_A_ROW && self.enabled.swap(false, Ordering::AcqRel) {
            let why = format!("disabled after {MAX_FAILURES_IN_A_ROW} failed calls in a row; the last: {message}");
            tracing::warn!("script rule {}: {why}", self.id());
            self.set_error(why);
            return true;
        }
        self.set_error(message.to_string());
        false
    }

    pub fn succeeded(&self) {
        self.counters.in_a_row.store(0, Ordering::Release);
    }

    /// At most once a second, load the file again when its time, size or
    /// inode changed. A failed load keeps the old version (I12).
    fn refresh(&self) {
        {
            let mut last = self.last_check.lock().unwrap();
            if last.elapsed() < Duration::from_secs(1) {
                return;
            }
            *last = Instant::now();
        }
        let path = Path::new(&self.rule.script);
        let current = self.loaded();
        match file_id(path) {
            Ok(id) if current.as_ref().is_some_and(|c| c.file == id) => {}
            Ok(_) => match load_file(path) {
                Ok(new) => {
                    let old_kind = current.as_ref().map(|c| c.info.kind);
                    if old_kind.is_some_and(|k| k != new.info.kind) {
                        self.set_error(format!(
                            "{}: the kind changed to {:?}; set the rule again to use it (the old version still runs)",
                            new.name, new.info.kind
                        ));
                        return;
                    }
                    if old_kind.is_none() {
                        // A rule loaded at start without a version stays off
                        // until it is set again: its fields were never checked.
                        return;
                    }
                    tracing::info!("script rule {}: loaded {} again", self.id(), new.name);
                    *self.script.write().unwrap() = Some(Arc::new(new));
                }
                Err(e) => self.set_error(format!("{e} (the last good version still runs)")),
            },
            Err(_) => {
                if current.is_some() {
                    self.set_error(format!("the script file is gone: {} (the last loaded version still runs)", path.display()));
                }
            }
        }
    }

    /// Run one call on this thread. `deadline` covers the wait for a thread
    /// and the call itself.
    pub fn execute(&self, call: Call, deadline: Instant) -> Result<Output, String> {
        if Instant::now() >= deadline {
            return Err("busy: no script thread was free within the time limit".into());
        }
        self.refresh();
        let loaded = self.loaded().ok_or("the script is not loaded")?;
        let lua = self.take_state(&loaded)?;
        lua.set_app_data(Deadline(deadline));
        lua.set_app_data(InCall(true));
        let result = self.call(&lua, &loaded, call);
        lua.set_app_data(InCall(false));
        self.states.lock().unwrap().push((loaded.version, lua));
        result
    }

    fn take_state(&self, loaded: &Loaded) -> Result<Lua, String> {
        {
            let mut states = self.states.lock().unwrap();
            states.retain(|(v, _)| *v == loaded.version);
            if let Some((_, lua)) = states.pop() {
                return Ok(lua);
            }
        }
        let ctx = StateCtx {
            rule_id: self.rule.id.clone(),
            log: loaded.info.kind == ScriptKind::Log,
            capture: self.capture.clone(),
            lines: self.lines.clone(),
        };
        new_state(&loaded.source, &loaded.name, &ctx).map(|(lua, _)| lua)
    }

    fn call(&self, lua: &Lua, loaded: &Loaded, call: Call) -> Result<Output, String> {
        let script: Table = lua.named_registry_value(SCRIPT_KEY).map_err(|e| e.to_string())?;
        let func = |name: &str| -> Result<mlua::Function, String> {
            script.raw_get::<mlua::Function>(name).map_err(|_| format!("{}: {name} is not a function", loaded.name))
        };
        let lua_err = |e: mlua::Error| message(&e);
        match call {
            Call::Request { mut req, body } => {
                let t = lua_api::request_table(lua, &req).map_err(lua_err)?;
                let ret: Value = func("on_request")?.call(&t).map_err(lua_err)?;
                let response = match ret {
                    Value::Nil => None,
                    Value::Table(r) => Some(lua_api::read_script_response(&r)?),
                    other => {
                        return Err(format!("on_request must return nil or a response table, not a {}", other.type_name()));
                    }
                };
                let changed = if response.is_none() { lua_api::read_request(&t, &mut req, body)? } else { None };
                Ok(Output::Request { req: Box::new(req), body: changed, response })
            }
            Call::Response { req, mut res, body } => {
                let rt = lua_api::request_table(lua, &req).map_err(lua_err)?;
                let t = lua_api::response_table(lua, &res).map_err(lua_err)?;
                let _: Value = func("on_response")?.call((rt, &t)).map_err(lua_err)?;
                let changed = lua_api::read_response(&t, &mut res, body)?;
                Ok(Output::Response { res, body: changed })
            }
            Call::Event { req, res, event } => {
                let rt = lua_api::request_table(lua, &req).map_err(lua_err)?;
                let st = lua_api::response_table(lua, &res).map_err(lua_err)?;
                let et = lua_api::event_fields(lua, &event, None).map_err(lua_err)?;
                let ret: Value = func("on_event")?.call((rt, st, &et)).map_err(lua_err)?;
                if matches!(ret, Value::Boolean(false)) {
                    return Ok(Output::Event(None));
                }
                Ok(Output::Event(Some(lua_api::read_event(&et, &event)?)))
            }
            Call::Exchange(ex) => {
                let t = lua_api::exchange_table(lua, &ex).map_err(lua_err)?;
                let _: Value = func("on_exchange")?.call(t).map_err(lua_err)?;
                Ok(Output::Done)
            }
            Call::LogEvent { ex, event, index } => {
                let t = lua_api::exchange_table(lua, &ex).map_err(lua_err)?;
                let et = lua_api::event_fields(lua, &event, Some(index)).map_err(lua_err)?;
                let _: Value = func("on_event")?.call((t, et)).map_err(lua_err)?;
                Ok(Output::Done)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    fn rule(script: &Path) -> ScriptRule {
        serde_json::from_value(serde_json::json!({"id": "t", "host": "a.example", "script": script})).unwrap()
    }

    fn active(dir: &Path, text: &str) -> ActiveRule {
        let p = write(dir, "t.lua", text);
        ActiveRule::new(rule(&p), load_file(&p), None)
    }

    fn request(path: &str) -> RequestInfo {
        RequestInfo { method: "GET".into(), scheme: "https", host: "a.example".into(), port: 443, path: path.into(), ..Default::default() }
    }

    fn run(rule: &ActiveRule, limit: Duration) -> Result<Output, String> {
        rule.execute(Call::Request { req: request("/"), body: BodyRule::Free }, Instant::now() + limit)
    }

    // T3, I4: each removed function is absent; bytecode is refused.
    #[test]
    fn the_sandbox_has_no_way_out() {
        let dir = tempfile::tempdir().unwrap();
        for expr in [
            "io.open('/etc/passwd')",
            "os.execute('true')",
            "os.remove('/tmp/x')",
            "os.getenv('HOME')",
            "os.exit(1)",
            "require('x')",
            "local _ = package.loaded",
            "debug.getinfo(1)",
            "load('return 1')()",
            "loadfile('/etc/passwd')",
            "dofile('/etc/passwd')",
            "string.dump(function() end)",
            "('x'):dump()",
            "collectgarbage('collect')",
        ] {
            let r = active(dir.path(), &format!("return {{ kind = 'intercept', on_request = function(req) {expr} end }}"));
            let err = match run(&r, Duration::from_secs(1)) {
                Err(e) => e,
                Ok(_) => panic!("{expr} worked"),
            };
            assert!(
                err.contains("attempt to call a nil value") || err.contains("attempt to index a nil value") || err.contains("only \"count\""),
                "{expr}: {err}"
            );
        }
        let ok = active(dir.path(), "return { kind = 'intercept', on_request = function(req) local _ = os.time() + os.clock() .. os.date('%Y') .. collectgarbage('count') end }");
        assert!(run(&ok, Duration::from_secs(1)).is_ok());
        let p = dir.path().join("b.lua");
        std::fs::write(&p, b"\x1bLua\x54\x00junk").unwrap();
        assert!(load_file(&p).unwrap_err().contains("bytecode"));
    }

    // T4, I5: limits fail the call, not the process; the next call works.
    #[test]
    fn time_and_memory_limits_fail_the_call_only() {
        let dir = tempfile::tempdir().unwrap();
        let r = active(
            dir.path(),
            "return { kind = 'intercept', on_request = function(req)
               if req.path == '/loop' then while true do end end
               if req.path == '/grow' then local t = {} for i = 1, 1e9 do t[i] = string.rep('x', 1000) .. i end end
             end }",
        );
        let started = Instant::now();
        let err = r.execute(Call::Request { req: request("/loop"), body: BodyRule::Free }, Instant::now() + INTERCEPT_TIME).err().unwrap();
        assert!(err.contains("time limit"), "{err}");
        assert!(started.elapsed() < Duration::from_millis(100), "{:?}", started.elapsed());
        let err = r.execute(Call::Request { req: request("/grow"), body: BodyRule::Free }, Instant::now() + Duration::from_secs(20)).err().unwrap();
        assert!(err.contains("memory limit"), "{err}");
        assert!(run(&r, INTERCEPT_TIME).is_ok(), "the next call on the same rule works");
        let top = write(dir.path(), "top.lua", "while true do end");
        assert!(load_file(&top).unwrap_err().contains("time limit"));
    }

    // T11, I12: what a load refuses, with the file and line.
    #[test]
    fn loading_checks_the_returned_table() {
        let dir = tempfile::tempdir().unwrap();
        for (text, want) in [
            ("return {", "t.lua:1:"),
            ("local x = jsn.encode(1) return {}", "t.lua:1: attempt to index a nil value (global 'jsn')"),
            ("return 1", "must end with"),
            ("return {}", "no kind"),
            ("return { kind = 'other' }", "must be \"intercept\" or \"log\""),
            ("return { kind = 'log' }", "needs on_exchange"),
            ("return { kind = 'intercept' }", "needs on_request"),
            ("return { kind = 'log', on_exchange = function() end, copy = { 'media' }, save = { 'media' } }", "both copy and save"),
            ("return { kind = 'intercept', on_request = function() end, response_body = { 'events' } }", "never held"),
            ("return { kind = 'intercept', on_request = function() end, request_body = { 'jpeg' } }", "the classes are"),
            ("return { kind = 'log', on_exchange = function() end, response_body = true }", "for intercept scripts"),
            ("return { kind = 'intercept', on_request = 1 }", "must be a function"),
        ] {
            let p = write(dir.path(), "t.lua", text);
            let err = load_file(&p).err().unwrap_or_else(|| panic!("{text} loaded"));
            assert!(err.contains(want), "{text}: {err}");
        }
        let p = write(dir.path(), "t.lua", "return { kind = 'intercept', on_request = function() end, request_body = true }");
        let l = load_file(&p).unwrap();
        assert_eq!(l.info.request_body, Classes::of(&[BodyClass::Text]));
        assert_eq!(l.sha256.len(), 64);
        let p = write(dir.path(), "t.lua", "return { kind = 'log', on_exchange = function() end }");
        assert_eq!(load_file(&p).unwrap().info.copy, Classes::of(&[BodyClass::Text, BodyClass::Events]));
        assert!(load_file(&dir.path().join("none.lua")).unwrap_err().contains("gone"));
    }

    // T11: an edit is used after a second; a broken edit keeps the old code;
    // a deleted file keeps the old code and says so.
    #[test]
    fn reload_uses_new_code_and_keeps_the_old_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "t.lua", "return { kind = 'intercept', on_request = function(req) req.path = '/one' end }");
        let r = ActiveRule::new(rule(&p), load_file(&p), None);
        let path_of = |r: &ActiveRule| match run(r, Duration::from_secs(1)) {
            Ok(Output::Request { req, .. }) => req.path.clone(),
            Err(e) => panic!("{e}"),
            _ => unreachable!(),
        };
        assert_eq!(path_of(&r), "/one");
        std::thread::sleep(Duration::from_millis(1100));
        write(dir.path(), "t.lua", "return { kind = 'intercept', on_request = function(req) req.path = '/two/' .. 'x' end }");
        assert_eq!(path_of(&r), "/two/x");
        std::thread::sleep(Duration::from_millis(1100));
        write(dir.path(), "t.lua", "return { kind = 'intercept', on_request = function(req) req.path = end }");
        assert_eq!(path_of(&r), "/two/x");
        assert!(r.last_error().unwrap().message.contains("t.lua:1:"), "{:?}", r.last_error());
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::remove_file(&p).unwrap();
        assert_eq!(path_of(&r), "/two/x");
        assert!(r.last_error().unwrap().message.contains("gone"));
    }

    // I6
    #[test]
    fn twenty_failures_in_a_row_turn_a_rule_off() {
        let dir = tempfile::tempdir().unwrap();
        let r = active(dir.path(), "return { kind = 'intercept', on_request = function() end }");
        for _ in 0..19 {
            assert!(!r.failed("x"));
        }
        r.succeeded();
        for _ in 0..19 {
            assert!(!r.failed("x"));
        }
        assert!(r.is_enabled());
        assert!(r.failed("t.lua:1: boom"));
        assert!(!r.is_enabled());
        assert!(r.last_error().unwrap().message.contains("disabled after 20 failed calls in a row"));
        assert_eq!(r.counters.errors.load(Ordering::Relaxed), 39);
    }

    #[test]
    fn capture_is_refused_while_the_file_loads() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "t.lua", "capture.append('a.txt', 'x') return { kind = 'log', on_exchange = function() end }");
        assert!(load_file(&p).unwrap_err().contains("not while the file loads"));
    }
}
