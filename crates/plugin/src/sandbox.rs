//! The sandbox: a plugin module runs in a metered WebAssembly interpreter (wasmi; no JIT) with a
//! memory cap, and sees the outside world only through the host functions below. There is no WASI:
//! no file system, network, clock or randomness.
//!
//! # ABI (version 4; version 1 is the first seven imports, version 2 adds panels and settings,
//! version 3 widgets, version 4 `save_file`)
//!
//! The module exports its `memory` and `figcraft_run(input_len: i32) -> i32`. Host functions are
//! imported from the module `figcraft`; strings are UTF-8 `(ptr, len)` pairs in the plugin's memory.
//!
//! | Import | Signature | Does |
//! |---|---|---|
//! | `call` | `(cmd_ptr, cmd_len, params_ptr, params_len) -> i32` | Runs a command with JSON params. 0: ok, the reply holds the result JSON; 1: refused or failed, the reply holds the message. |
//! | `selection` | `() -> i32` | The reply becomes the selected layer ids (a JSON array); returns its length. |
//! | `reply_len` | `() -> i32` | Length of the reply. |
//! | `reply_read` | `(ptr, len) -> i32` | Copies up to `len` bytes of the reply to `ptr`; returns the count. |
//! | `log` | `(ptr, len)` | A line for the run's log. |
//! | `notify` | `(ptr, len)` | A short message shown to the user. |
//! | `set_output` | `(ptr, len)` | The run's result (JSON or text), or its error message when it fails. |
//! | `ui_show` | `(ptr, len) -> i32` | Shows (or replaces) the plugin's panel: a [`Panel`] as JSON. 1: invalid, the reply says why. |
//! | `ui_close` | `()` | Closes the panel. |
//! | `setting_get` | `(key_ptr, key_len) -> i32` | 0: the reply holds the stored JSON; 1: no such setting. |
//! | `setting_set` | `(key_ptr, key_len, value_ptr, value_len) -> i32` | Stores JSON under the key. 1: refused, the reply says why. |
//! | `setting_delete` | `(key_ptr, key_len)` | Forgets a setting. |
//! | `widget_render` | `(ptr, len) -> i32` | Draws the widget being called back: a [`WidgetRender`] as JSON. 1: invalid, the reply says why. |
//! | `save_file` | `(name_ptr, name_len, mime_ptr, mime_len, data_ptr, data_len) -> i32` | Offers a file to save (needs `permissions.saveFile`). The host asks the user where it goes; the plugin never gets a path. 0: accepted, 1: refused, the reply says why. |
//!
//! When `figcraft_run` starts, the reply holds the input JSON (`input_len` bytes):
//! `{"api", "command", "params", "selection"}`, plus `event` and `values` when a panel calls the
//! plugin back, or `widget` (`{id, state, event}`) when a widget does. It returns 0 on success; anything else fails the run.
//!
//! An [`Instance`] keeps the module's memory between calls, so a plugin with a panel open keeps its
//! state from one callback to the next. Every call gets the full [`Limits`] again.

use wasmi::{
    Caller, Config, Engine, Extern, FuncType, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode, TypedFunc, TypedResumableCall, Val,
    ValType,
};

use crate::manifest::Panel;
use crate::widget::WidgetRender;

/// The module host functions are imported from.
pub const IMPORT_MODULE: &str = "figcraft";
/// The exported entry point.
pub const ENTRY: &str = "figcraft_run";

/// The host functions: (name, params, results), all i32.
const IMPORTS: &[(&str, usize, usize)] = &[
    ("call", 4, 1),
    ("selection", 0, 1),
    ("reply_len", 0, 1),
    ("reply_read", 2, 1),
    ("log", 2, 0),
    ("notify", 2, 0),
    ("set_output", 2, 0),
    ("ui_show", 2, 1),
    ("ui_close", 0, 0),
    ("setting_get", 2, 1),
    ("setting_set", 4, 1),
    ("setting_delete", 2, 0),
    ("widget_render", 2, 1),
    ("save_file", 6, 1),
];

/// What a run may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Instruction budget (wasmi fuel). Each host call costs [`Limits::call_fuel`] more,
    /// plus one fuel per byte copied out of or into guest memory.
    pub fuel: u64,
    pub call_fuel: u64,
    /// Linear memory cap in bytes.
    pub memory_bytes: usize,
    /// Largest module accepted.
    pub module_bytes: usize,
    /// Host calls per run.
    pub max_calls: u32,
    /// Largest string passed either way (command params, results, logs).
    pub max_message: usize,
    /// Log lines kept and forwarded per run. Exceeding any log limit fails the run.
    pub max_logs: usize,
    /// Largest log line in UTF-8 bytes, also subject to [`Limits::max_message`].
    pub max_log_line_bytes: usize,
    /// Total log bytes kept and forwarded per run (excluding string/vector overhead).
    pub max_log_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 1_000_000_000,
            call_fuel: 10_000,
            memory_bytes: 64 * 1024 * 1024,
            module_bytes: 16 * 1024 * 1024,
            max_calls: 100_000,
            max_message: 16 * 1024 * 1024,
            max_logs: 1000,
            max_log_line_bytes: 8 * 1024,
            max_log_bytes: 1024 * 1024,
        }
    }
}

/// Why a run failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PluginError {
    #[error("not a valid plugin module: {0}")]
    InvalidModule(String),
    #[error("the plugin ran out of its instruction budget (an endless loop?)")]
    OutOfFuel,
    #[error("the plugin used more than its memory limit")]
    OutOfMemory,
    #[error("the plugin crashed: {0}")]
    Trap(String),
    #[error("the plugin went over a limit: {0}")]
    Limit(String),
    #[error("{0}")]
    Failed(String),
}

/// What the sandbox asks of its host (the engine): run commands, read the selection, log, keep
/// settings.
pub trait Host {
    /// Run `command` with `params` (JSON text): the result JSON, or why it was refused or failed.
    fn call(&mut self, command: &str, params: &str) -> Result<String, String>;
    /// The selected layer ids as a JSON array.
    fn selection(&mut self) -> String;
    /// A message for the user.
    fn notify(&mut self, message: &str);
    /// An admitted line of the run's log (also in [`RunOutput::logs`]). Log limits apply before
    /// this is called, including on runs that later fail.
    fn log(&mut self, _line: &str) {}
    /// The plugin's setting `key` (JSON text).
    fn setting(&mut self, _key: &str) -> Option<String> {
        None
    }
    /// Store (`Some`) or forget (`None`) setting `key`, or say why not.
    fn set_setting(&mut self, _key: &str, _value: Option<&str>) -> Result<(), String> {
        Err("settings aren't available here".into())
    }
    /// Offer the file `name` (MIME type `mime`) for saving, or say why not. The host picks the
    /// destination; the plugin never learns it.
    fn save_file(&mut self, _name: &str, _mime: &str, _data: &[u8]) -> Result<(), String> {
        Err("saving files isn't available here".into())
    }
}

/// What a run asked of the panel.
#[derive(Clone, Debug, PartialEq)]
pub enum UiChange {
    /// Show this panel (or replace the one showing).
    Show(Panel),
    Close,
}

/// A finished run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunOutput {
    /// What the plugin passed to `set_output` (empty when nothing).
    pub output: String,
    pub logs: Vec<String>,
    /// Host calls made.
    pub calls: u32,
    /// Fuel used.
    pub fuel: u64,
    /// The last `ui_show` / `ui_close` of the run.
    pub ui: Option<UiChange>,
    /// The last `widget_render` of the run.
    pub widget: Option<WidgetRender>,
}

/// A host call that needs the [`Host`]: the plugin is paused while the host answers it (see
/// [`Instance::call`]), so the store never borrows the host and can outlive one call.
enum Request {
    Call(String, String),
    Selection,
    Log(String),
    Notify(String),
    SettingGet(String),
    SettingSet(String, String),
    SettingDelete(String),
    SaveFile(String, String, Vec<u8>),
}

struct State {
    limits: Limits,
    store_limits: StoreLimits,
    reply: Vec<u8>,
    out: RunOutput,
    log_bytes: usize,
    /// A limit the host tripped (reported instead of the trap it causes).
    abort: Option<PluginError>,
    request: Option<Request>,
}

impl State {
    /// Make `message` the reply and return status 1 (refused).
    fn refuse(&mut self, message: String) -> i32 {
        self.set_reply(message.into_bytes());
        1
    }

    /// Make `bytes` the reply; returns its length.
    fn set_reply(&mut self, bytes: Vec<u8>) -> i32 {
        let max = self.limits.max_message;
        let bytes = if bytes.len() > max { format!("the result is larger than {max} bytes").into_bytes() } else { bytes };
        let len = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
        self.reply = bytes;
        len
    }

    /// Answer a paused host call; the host function's result, if it has one.
    fn answer(&mut self, host: &mut dyn Host, request: Request) -> Option<i32> {
        match request {
            Request::Call(cmd, params) => {
                let (status, reply) = match host.call(&cmd, &params) {
                    Ok(v) => (0, v),
                    Err(m) => (1, m),
                };
                self.set_reply(reply.into_bytes());
                Some(status)
            }
            Request::Selection => Some(self.set_reply(host.selection().into_bytes())),
            Request::Log(line) => {
                host.log(&line);
                self.out.logs.push(line);
                None
            }
            Request::Notify(msg) => {
                host.notify(&msg);
                None
            }
            Request::SettingGet(key) => Some(match host.setting(&key) {
                Some(v) => {
                    self.set_reply(v.into_bytes());
                    0
                }
                None => {
                    self.set_reply(Vec::new());
                    1
                }
            }),
            Request::SettingSet(key, value) => Some(match host.set_setting(&key, Some(&value)) {
                Ok(()) => 0,
                Err(m) => {
                    self.set_reply(m.into_bytes());
                    1
                }
            }),
            Request::SettingDelete(key) => {
                let _ = host.set_setting(&key, None);
                None
            }
            Request::SaveFile(name, mime, data) => Some(match host.save_file(&name, &mime, &data) {
                Ok(()) => 0,
                Err(m) => {
                    self.set_reply(m.into_bytes());
                    1
                }
            }),
        }
    }
}

fn engine() -> Engine {
    let mut c = Config::default();
    c.consume_fuel(true);
    Engine::new(&c)
}

fn func_type(params: usize, results: usize) -> FuncType {
    FuncType::new(vec![ValType::I32; params], vec![ValType::I32; results])
}

/// Compile and check a module: its size, that it imports only FigCraft's host functions (no WASI)
/// and exports `memory` and `figcraft_run`.
pub fn check_module(bytes: &[u8], limits: &Limits) -> Result<(), PluginError> {
    compile(&engine(), bytes, limits).map(|_| ())
}

fn compile(engine: &Engine, bytes: &[u8], limits: &Limits) -> Result<Module, PluginError> {
    if bytes.len() > limits.module_bytes {
        return Err(PluginError::InvalidModule(format!("larger than {} MB", limits.module_bytes / (1024 * 1024))));
    }
    if !bytes.starts_with(b"\0asm") {
        return Err(PluginError::InvalidModule("not a WebAssembly binary".into()));
    }
    let module = Module::new(engine, bytes).map_err(|e| PluginError::InvalidModule(e.to_string()))?;
    for imp in module.imports() {
        let known = IMPORTS.iter().find(|(n, ..)| imp.module() == IMPORT_MODULE && *n == imp.name());
        match (known, imp.ty().func()) {
            (Some((_, p, r)), Some(ty)) if *ty == func_type(*p, *r) => {}
            (Some(_), _) => {
                return Err(PluginError::InvalidModule(format!("`{}.{}` is imported with the wrong signature", imp.module(), imp.name())));
            }
            (None, _) => {
                return Err(PluginError::InvalidModule(format!(
                    "it imports `{}.{}`, which FigCraft doesn't provide (plugins get no file system, network or clock)",
                    imp.module(),
                    imp.name()
                )));
            }
        }
    }
    let export = |name: &str| module.exports().find(|e| e.name() == name).map(|e| e.ty().clone());
    if export("memory").is_none_or(|t| t.memory().is_none()) {
        return Err(PluginError::InvalidModule("it doesn't export its `memory`".into()));
    }
    if export(ENTRY).is_none_or(|t| t.func() != Some(&func_type(1, 1))) {
        return Err(PluginError::InvalidModule(format!("it doesn't export `{ENTRY}(input_len: i32) -> i32`")));
    }
    Ok(module)
}

/// Run a plugin module once: `input` is the JSON handed to `figcraft_run`.
pub fn run(bytes: &[u8], input: &str, limits: &Limits, host: &mut dyn Host) -> Result<RunOutput, PluginError> {
    Instance::new(bytes, limits)?.call(input, host)
}

/// A started plugin module whose memory lasts from one call to the next.
pub struct Instance {
    store: Store<State>,
    entry: TypedFunc<i32, i32>,
}

impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instance").finish_non_exhaustive()
    }
}

impl Instance {
    /// Compile, check and start a module. A start function can't call host functions.
    pub fn new(bytes: &[u8], limits: &Limits) -> Result<Self, PluginError> {
        let engine = engine();
        let module = compile(&engine, bytes, limits)?;
        let store_limits =
            StoreLimitsBuilder::new().memory_size(limits.memory_bytes).memories(1).tables(4).table_elements(100_000).instances(1).build();
        let state = State { limits: *limits, store_limits, reply: Vec::new(), out: RunOutput::default(), log_bytes: 0, abort: None, request: None };
        let mut store = Store::new(&engine, state);
        store.limiter(|s| &mut s.store_limits);
        store.set_fuel(limits.fuel).map_err(|e| PluginError::Trap(e.to_string()))?;
        let linker = linker(&engine)?;
        let instance = linker.instantiate_and_start(&mut store, &module).map_err(|e| classify(&mut store, &e, true))?;
        let entry = instance.get_typed_func::<i32, i32>(&store, ENTRY).map_err(|e| PluginError::InvalidModule(e.to_string()))?;
        Ok(Self { store, entry })
    }

    /// Call `figcraft_run` with `input`, with the full limits. Host calls that need `host` pause
    /// the plugin, are answered here and resume it.
    pub fn call(&mut self, input: &str, host: &mut dyn Host) -> Result<RunOutput, PluginError> {
        let limits = self.store.data().limits;
        let input_len = i32::try_from(input.len()).ok().filter(|_| input.len() <= limits.max_message);
        let input_len = input_len.ok_or_else(|| PluginError::Limit("the input is too large".into()))?;
        let st = self.store.data_mut();
        st.reply = input.as_bytes().to_vec();
        st.out = RunOutput::default();
        st.log_bytes = 0;
        st.abort = None;
        st.request = None;
        self.store.set_fuel(limits.fuel).map_err(|e| PluginError::Trap(e.to_string()))?;
        let mut call = self.entry.call_resumable(&mut self.store, input_len).map_err(|e| classify(&mut self.store, &e, false))?;
        let status = loop {
            match call {
                TypedResumableCall::Finished(status) => break status,
                TypedResumableCall::OutOfFuel(_) => return Err(PluginError::OutOfFuel),
                TypedResumableCall::HostTrap(paused) => {
                    let Some(request) = self.store.data_mut().request.take() else {
                        return Err(classify(&mut self.store, paused.host_error(), false));
                    };
                    let results: Vec<Val> = self.store.data_mut().answer(host, request).map(Val::I32).into_iter().collect();
                    call = paused.resume(&mut self.store, &results).map_err(|e| classify(&mut self.store, &e, false))?;
                }
            }
        };
        let fuel = limits.fuel.saturating_sub(self.store.get_fuel().unwrap_or(0));
        let mut out = std::mem::take(&mut self.store.data_mut().out);
        out.fuel = fuel;
        if status != 0 {
            let msg = if out.output.trim().is_empty() { format!("the plugin failed (status {status})") } else { out.output };
            return Err(PluginError::Failed(msg));
        }
        Ok(out)
    }
}

/// The error a failed instantiation or call stands for: a limit the host tripped first, else the
/// trap.
fn classify(store: &mut Store<State>, e: &wasmi::Error, instantiating: bool) -> PluginError {
    if let Some(abort) = store.data_mut().abort.take() {
        return abort;
    }
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => PluginError::OutOfFuel,
        Some(TrapCode::GrowthOperationLimited) => PluginError::OutOfMemory,
        Some(code) => PluginError::Trap(code.to_string()),
        // A memory larger than the cap fails instantiation.
        None if instantiating && e.to_string().to_ascii_lowercase().contains("limit") => PluginError::OutOfMemory,
        None if instantiating => PluginError::InvalidModule(e.to_string()),
        None => PluginError::Trap(e.to_string()),
    }
}

type Ctx<'a> = Caller<'a, State>;

/// Stop the run because of `err` (the trap it causes reports `err`).
fn abort(caller: &mut Ctx<'_>, err: PluginError) -> wasmi::Error {
    let msg = err.to_string();
    caller.data_mut().abort = Some(err);
    wasmi::Error::new(msg)
}

/// Pause the plugin until the host answers `request`.
// shortcut: a host function tail-called (`return_call`) from `figcraft_run` itself can't be
// resumed and fails the run; fine for compiled plugins, which don't do that.
fn ask(caller: &mut Ctx<'_>, request: Request) -> wasmi::Error {
    caller.data_mut().request = Some(request);
    wasmi::Error::new("a host function was called outside figcraft_run")
}

/// Charge a host call: fuel and the call count.
fn charge(caller: &mut Ctx<'_>) -> Result<(), wasmi::Error> {
    let cost = caller.data().limits.call_fuel;
    spend_fuel(caller, cost)?;
    let st = caller.data_mut();
    st.out.calls = st.out.calls.saturating_add(1);
    if st.out.calls > st.limits.max_calls {
        let n = st.limits.max_calls;
        return Err(abort(caller, PluginError::Limit(format!("more than {n} host calls"))));
    }
    Ok(())
}

/// Reserve work before copying or allocating, including when call overhead is disabled.
fn spend_fuel(caller: &mut Ctx<'_>, cost: u64) -> Result<(), wasmi::Error> {
    let Some(left) = caller.get_fuel()?.checked_sub(cost) else {
        return Err(abort(caller, PluginError::OutOfFuel));
    };
    caller.set_fuel(left)
}

/// The bytes at `(ptr, len)` in the plugin's memory.
fn read_bytes(caller: &mut Ctx<'_>, ptr: i32, len: i32) -> Result<Vec<u8>, wasmi::Error> {
    let (ptr, len) = (ptr as u32 as usize, len as u32 as usize);
    if len > caller.data().limits.max_message {
        return Err(abort(caller, PluginError::Limit(format!("a {len}-byte string is too long"))));
    }
    spend_fuel(caller, len as u64)?;
    let Some(mem) = caller.get_export("memory").and_then(Extern::into_memory) else {
        return Err(abort(caller, PluginError::InvalidModule("no memory".into())));
    };
    match ptr.checked_add(len).and_then(|end| mem.data(&*caller).get(ptr..end)) {
        Some(b) => Ok(b.to_vec()),
        None => Err(wasmi::Error::from(TrapCode::MemoryOutOfBounds)),
    }
}

/// The UTF-8 text at `(ptr, len)`.
fn read_str(caller: &mut Ctx<'_>, ptr: i32, len: i32) -> Result<String, wasmi::Error> {
    let bytes = read_bytes(caller, ptr, len)?;
    String::from_utf8(bytes).map_err(|_| abort(caller, PluginError::Trap("a string isn't valid UTF-8".into())))
}

/// Admit a log before copying guest memory. The same budgets bound retained logs and `Host::log`,
/// so dropping a retained line cannot leave unbounded forwarding to the host logger.
fn read_log(caller: &mut Ctx<'_>, ptr: i32, len: i32) -> Result<String, wasmi::Error> {
    let bytes = len as u32 as usize;
    let limits = caller.data().limits;
    if bytes > limits.max_log_line_bytes {
        return Err(abort(caller, PluginError::Limit(format!("a {bytes}-byte log line is too long (limit: {} bytes)", limits.max_log_line_bytes))));
    }
    if caller.data().out.logs.len() >= limits.max_logs {
        return Err(abort(caller, PluginError::Limit(format!("more than {} log lines", limits.max_logs))));
    }
    let total = caller
        .data()
        .log_bytes
        .checked_add(bytes)
        .filter(|total| *total <= limits.max_log_bytes)
        .ok_or_else(|| abort(caller, PluginError::Limit(format!("more than {} log bytes", limits.max_log_bytes))))?;
    let line = read_str(caller, ptr, len)?;
    caller.data_mut().log_bytes = total;
    Ok(line)
}

fn linker(engine: &Engine) -> Result<Linker<State>, PluginError> {
    let mut l = <Linker<State>>::new(engine);
    let err = |e: wasmi::errors::LinkerError| PluginError::InvalidModule(e.to_string());
    l.func_wrap(IMPORT_MODULE, "call", |mut c: Ctx<'_>, cp: i32, cl: i32, pp: i32, pl: i32| -> Result<i32, wasmi::Error> {
        charge(&mut c)?;
        let cmd = read_str(&mut c, cp, cl)?;
        let params = read_str(&mut c, pp, pl)?;
        let params = if params.trim().is_empty() { "{}".to_string() } else { params };
        Err(ask(&mut c, Request::Call(cmd, params)))
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "selection", |mut c: Ctx<'_>| -> Result<i32, wasmi::Error> {
        charge(&mut c)?;
        Err(ask(&mut c, Request::Selection))
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "reply_len", |mut c: Ctx<'_>| -> Result<i32, wasmi::Error> {
        charge(&mut c)?;
        Ok(i32::try_from(c.data().reply.len()).unwrap_or(i32::MAX))
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "reply_read", |mut c: Ctx<'_>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
        charge(&mut c)?;
        let (ptr, want) = (ptr as u32 as usize, len as u32 as usize);
        let n = want.min(c.data().reply.len());
        spend_fuel(&mut c, n as u64)?;
        let Some(mem) = c.get_export("memory").and_then(Extern::into_memory) else {
            return Err(abort(&mut c, PluginError::InvalidModule("no memory".into())));
        };
        let (data, st) = mem.data_and_store_mut(&mut c);
        let src = st.reply.get(..n).ok_or(TrapCode::MemoryOutOfBounds)?;
        let dst = ptr.checked_add(n).and_then(|end| data.get_mut(ptr..end)).ok_or(TrapCode::MemoryOutOfBounds)?;
        dst.copy_from_slice(src);
        Ok(i32::try_from(n).unwrap_or(i32::MAX))
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "log", |mut c: Ctx<'_>, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
        charge(&mut c)?;
        let line = read_log(&mut c, ptr, len)?;
        Err(ask(&mut c, Request::Log(line)))
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "notify", |mut c: Ctx<'_>, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
        charge(&mut c)?;
        let msg = read_str(&mut c, ptr, len)?;
        let msg: String = msg.chars().filter(|ch| !ch.is_control()).take(200).collect();
        Err(ask(&mut c, Request::Notify(msg)))
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "set_output", |mut c: Ctx<'_>, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
        charge(&mut c)?;
        let out = read_str(&mut c, ptr, len)?;
        c.data_mut().out.output = out;
        Ok(())
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "ui_show", |mut c: Ctx<'_>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
        charge(&mut c)?;
        let spec = read_str(&mut c, ptr, len)?;
        let st = c.data_mut();
        Ok(match Panel::parse(&spec) {
            Ok(panel) => {
                st.out.ui = Some(UiChange::Show(panel));
                0
            }
            Err(why) => {
                st.set_reply(why.into_bytes());
                1
            }
        })
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "widget_render", |mut c: Ctx<'_>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
        charge(&mut c)?;
        let spec = read_str(&mut c, ptr, len)?;
        let st = c.data_mut();
        Ok(match WidgetRender::parse(&spec) {
            Ok(w) => {
                st.out.widget = Some(w);
                0
            }
            Err(why) => {
                st.set_reply(why.into_bytes());
                1
            }
        })
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "ui_close", |mut c: Ctx<'_>| -> Result<(), wasmi::Error> {
        charge(&mut c)?;
        c.data_mut().out.ui = Some(UiChange::Close);
        Ok(())
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "setting_get", |mut c: Ctx<'_>, kp: i32, kl: i32| -> Result<i32, wasmi::Error> {
        charge(&mut c)?;
        let key = read_str(&mut c, kp, kl)?;
        Err(ask(&mut c, Request::SettingGet(key)))
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "setting_set", |mut c: Ctx<'_>, kp: i32, kl: i32, vp: i32, vl: i32| -> Result<i32, wasmi::Error> {
        charge(&mut c)?;
        let key = read_str(&mut c, kp, kl)?;
        let value = read_str(&mut c, vp, vl)?;
        Err(ask(&mut c, Request::SettingSet(key, value)))
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "setting_delete", |mut c: Ctx<'_>, kp: i32, kl: i32| -> Result<(), wasmi::Error> {
        charge(&mut c)?;
        let key = read_str(&mut c, kp, kl)?;
        Err(ask(&mut c, Request::SettingDelete(key)))
    })
    .map_err(err)?;
    l.func_wrap(IMPORT_MODULE, "save_file", |mut c: Ctx<'_>, np: i32, nl: i32, mp: i32, ml: i32, dp: i32, dl: i32| -> Result<i32, wasmi::Error> {
        charge(&mut c)?;
        let name = read_str(&mut c, np, nl)?;
        let mime = read_str(&mut c, mp, ml)?;
        // Too much data is a refusal the plugin can handle, not a limit that fails the run.
        let (len, max) = (dl as u32 as usize, c.data().limits.max_message);
        if len > max {
            return Ok(c.data_mut().refuse(format!("a file to save is limited to {max} bytes, this one has {len}")));
        }
        let data = read_bytes(&mut c, dp, dl)?;
        Err(ask(&mut c, Request::SaveFile(name, mime, data)))
    })
    .map_err(err)?;
    Ok(l)
}
