//! Plugins: WebAssembly modules that act on the document through the same commands as the menus,
//! CLI and MCP server, inside the sandbox of `figcraft-plugin`. This module is the policy: which
//! commands a plugin may call (its manifest's permissions, and never files, other documents, the
//! undo history, dialogs or plugins), one undo step per run or panel callback, the panels plugins
//! keep open, their settings and the installed-plugin list kept in the preferences. Widgets (plugin
//! layers on the canvas) are in [`crate::widgets`], installing from a server's plugin directory in
//! [`crate::plugin_directory`]. See `docs/plugins.md`.

use std::collections::HashMap;
use std::sync::Arc;

use figcraft_plugin::{Instance, Limits, PluginError, RunOutput, UiChange};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};

use crate::cmd::{CommandSpec, find_command};
use crate::params::{get, num, string};
use crate::{EngineError, Result, Session, bad, spec};

pub use figcraft_plugin::{Access, MANIFEST_FILE, Manifest, Panel, Param, ParamKind};

/// What one plugin may keep in its settings (keys and JSON values, in bytes), and its longest key.
const MAX_SETTINGS_BYTES: usize = 64 * 1024;
const MAX_SETTING_KEY: usize = 128;
/// Maximum number of plugin execution records kept in [`Runtime::history`].
pub const MAX_RUN_HISTORY: usize = 50;

/// A recorded plugin execution (command, panel event or widget callback) for the Plugin Dev Console.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRunRecord {
    pub plugin: String,
    pub command: String,
    pub ok: bool,
    pub output: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub calls: u32,
    pub fuel: u64,
    pub fuel_limit: u64,
}

/// An installed plugin (in the preferences).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub manifest: Manifest,
    /// Its manifest file; `None` when it was imported without one (its module then lives in this
    /// session only and isn't saved with the preferences).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The plugin directory it was installed from (`ws://host:port/`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Saves only the plugins imported from a manifest file (the others can't be loaded again).
pub(crate) fn persisted<S: Serializer>(list: &[InstalledPlugin], s: S) -> std::result::Result<S::Ok, S::Error> {
    s.collect_seq(list.iter().filter(|p| p.path.is_some()))
}

/// Per-session plugin state.
#[derive(Debug, Default)]
pub struct Runtime {
    /// Modules of plugins imported without a manifest file, by plugin id.
    pub(crate) modules: HashMap<String, Arc<Vec<u8>>>,
    /// The last run (plugin id, command id, params): "Run last plugin".
    pub last: Option<(String, String, Value)>,
    /// The open plugin panels, one per plugin, in the order they opened.
    pub panels: Vec<OpenPanel>,
    /// Where plugins installed from a directory are kept (the app sets a folder of the user's;
    /// without one, such plugins last for the session only).
    pub install_dir: Option<std::path::PathBuf>,
    /// The plugin directory listed last (Plugins ▸ Manage plugins ▸ Browse shows it).
    pub directory: Option<crate::plugin_directory::Listing>,
    /// The installed plugins or their settings changed since the app last saved the preferences
    /// (it saves them at once, so a crash keeps them).
    pub unsaved: bool,
    /// Recent plugin executions (oldest first, capped at [`MAX_RUN_HISTORY`]).
    pub history: Vec<PluginRunRecord>,
}

/// Record a completed plugin execution in `s.plugins.history`, trimming oldest entries past [`MAX_RUN_HISTORY`].
pub(crate) fn record_run(s: &mut Session, rec: PluginRunRecord) {
    s.plugins.history.push(rec);
    let over = s.plugins.history.len().saturating_sub(MAX_RUN_HISTORY);
    if over > 0 {
        s.plugins.history.drain(..over);
    }
}

impl Runtime {
    /// Plugin `id`'s open panel.
    pub fn panel(&self, id: &str) -> Option<&OpenPanel> {
        self.panels.iter().find(|o| o.manifest.id == id)
    }
}

/// A plugin's open panel: the running module (its memory lasts from one callback to the next),
/// the panel it showed and the values entered.
#[derive(Debug)]
pub struct OpenPanel {
    pub manifest: Manifest,
    /// The command that opened it, its parameters and its undo step's name.
    pub command: String,
    params: Value,
    label: String,
    pub panel: Panel,
    pub values: Map<String, Value>,
    instance: Instance,
    /// API 5: the widget whose callback opened the panel; its callbacks can redraw it.
    pub widget: Option<figcraft_doc::NodeId>,
}

/// Show `panel` as plugin `m`'s one open panel (replacing any it has), keeping `instance` loaded
/// for its callbacks. `opener` is the command that opened it, its parameters and its undo step's name.
pub(crate) fn open_panel(
    s: &mut Session,
    m: &Manifest,
    opener: (&str, Value, String),
    panel: Panel,
    instance: Instance,
    widget: Option<figcraft_doc::NodeId>,
) {
    let (command, params, label) = opener;
    let values = panel.values(&Map::new());
    close_panel(s, &m.id);
    s.plugins.panels.push(OpenPanel { manifest: m.clone(), command: command.into(), params, label, panel, values, instance, widget });
}

impl OpenPanel {
    /// The panel's title: its own, else the plugin's name.
    pub fn title(&self) -> &str {
        if self.panel.title.is_empty() { &self.manifest.name } else { &self.panel.title }
    }

    fn describe(&self) -> Value {
        json!({"plugin": self.manifest.id, "command": self.command, "title": self.title(), "controls": self.panel.controls, "values": self.values})
    }
}

/// Command prefixes no plugin may call, whatever its permissions: files and the app (plugins get
/// no file system), other documents, version history, the undo history, plugins themselves and
/// nested batches (a run already is one undo step), widgets (only a click calls one back).
const DENIED: &[&str] = &[
    "widget.",
    "file.",
    "doc.",
    "version.",
    "app.",
    "pref.",
    "help.",
    "plugin.",
    "plugins.",
    "libraries.",
    "library.add",
    // Contacts the servers a file names, and remembers them as yours (#1097).
    "library.check",
    // Uploads the open document to a server.
    "library.publishToServer",
    "edit.undo",
    "edit.redo",
    "edit.repeat",
    "command.batch",
    // Runs other commands inside (duplicate, edit, export), past this policy, like a batch.
    "data.fill",
    // Network sessions, online files and their recovery copies on disk.
    "collab.",
    // Opens files in new windows.
    "window.",
    "view.theme",
];
/// Read-only exceptions to [`DENIED`].
const ALLOWED: &[&str] = &["doc.list", "version.list"];

/// Why a plugin may not call `command` with `params` (before running it), if it may not.
fn refusal(m: &Manifest, command: &str, params: &Value) -> Option<String> {
    if DENIED.iter().any(|p| command.starts_with(p)) && !ALLOWED.contains(&command) {
        return Some(format!("`{command}` isn't available to plugins"));
    }
    // A preview runs its inner command, which the plugin's permissions and the denied list must cover too.
    // An inner command that can't be previewed (a nested preview) is refused by the preview itself,
    // before it runs: descending into its parameters first would only rescan them.
    if command == "text.preview"
        && let Some(inner) = get(params, "command").and_then(Value::as_str)
        && crate::textpreview::PREVIEWABLE.contains(&inner)
        && let Some(why) = refusal(m, inner, get(params, "params").unwrap_or(&Value::Null))
    {
        return Some(why);
    }
    // A widget's own data is FigCraft's to write, so no plugin can forge or retarget a widget.
    if writes_widget_data(params, 0) {
        return Some(format!("plugins can't write `{}…` layer data", crate::widgets::DATA_PREFIX));
    }
    if command == "image.place" && get(params, "path").is_some() {
        return Some("plugins can't read files: pass the image as `data`".into());
    }
    if command == "text.loadFont" && get(params, "path").is_some() {
        return Some("plugins can't read files: pass the font as `data`".into());
    }
    // `path` here is a file on disk (elsewhere it is a vector path or an action's place).
    if matches!(command, "variable.export" | "codeConnect.export" | "component.playground") && get(params, "path").is_some() {
        return Some("plugins can't write files: leave out `path` to get the result back".into());
    }
    if command == "codeConnect.import" && get(params, "path").is_some() {
        return Some("plugins can't read files: pass the connections as `json`".into());
    }
    // Variables JSON may be given as a path: only objects and JSON text for plugins.
    if command == "variable.import" && get(params, "variables").and_then(Value::as_str).is_some_and(|t| !t.trim_start().starts_with('{')) {
        return Some("plugins can't read files: pass the variables JSON itself".into());
    }
    if !m.permissions.allows(command) {
        return Some(format!("`{command}` isn't in the commands {} asked permission for", m.name));
    }
    if m.permissions.access == Access::Read && find_command(command).is_some_and(|c| c.history) {
        return Some(format!("`{command}` edits the document, and {} may only read it", m.name));
    }
    None
}

/// Whether `params` set layer data (`pluginData`, anywhere in them) under a widget's own keys.
fn writes_widget_data(params: &Value, depth: usize) -> bool {
    let own = |data: &Value| data.as_object().is_some_and(|m| m.keys().any(|k| k.starts_with(crate::widgets::DATA_PREFIX)));
    // serde_json refuses nesting past 128, so nothing deeper can reach here.
    depth < 128
        && match params {
            Value::Object(m) => m.iter().any(|(k, v)| (k == "pluginData" && own(v)) || writes_widget_data(v, depth + 1)),
            Value::Array(a) => a.iter().any(|v| writes_widget_data(v, depth + 1)),
            _ => false,
        }
}

/// The sandbox's view of the session: commands go through [`refusal`] and run as nested groups,
/// so a refused one leaves nothing behind.
struct EngineHost<'s> {
    s: &'s mut Session,
    manifest: &'s Manifest,
    /// Files offered for saving in this call.
    saves: u32,
}

/// Longest file name a plugin may suggest, in characters.
const MAX_SAVE_NAME: usize = 128;
/// Longest MIME type a plugin may name, in bytes.
const MAX_SAVE_MIME: usize = 100;
/// Largest file a plugin may offer for saving, in bytes (the sandbox's message limit).
const MAX_SAVE_BYTES: usize = 16 * 1024 * 1024;
/// Files a plugin may offer per call.
const MAX_SAVES: u32 = 1;

/// A file name a plugin suggested, cleaned up: a bare name without folders, drive letters, `..` or
/// characters no file system takes. `None` when nothing usable is left.
pub(crate) fn sanitize_save_name(name: &str) -> Option<String> {
    let name = name.trim();
    let bad = name.is_empty()
        || name.chars().count() > MAX_SAVE_NAME
        || name == "."
        || name.contains("..")
        || name.chars().any(|c| c.is_control() || crate::cmd::file::FORBIDDEN_FILE_NAME_CHARS.contains(c));
    if bad { None } else { Some(name.to_string()) }
}

/// `mime` when it looks like `type/subtype` in ASCII, else the generic binary type.
fn sanitize_mime(mime: &str) -> String {
    let mime = mime.trim();
    let token = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-' | '_'));
    match mime.split_once('/') {
        Some((a, b)) if mime.len() <= MAX_SAVE_MIME && token(a) && token(b) => mime.to_ascii_lowercase(),
        _ => "application/octet-stream".into(),
    }
}

impl figcraft_plugin::Host for EngineHost<'_> {
    fn call(&mut self, command: &str, params: &str) -> std::result::Result<String, String> {
        let params: Value = serde_json::from_str(params).map_err(|e| format!("params for `{command}` aren't JSON: {e}"))?;
        if let Some(why) = refusal(self.manifest, command, &params) {
            return Err(why);
        }
        let read_only = self.manifest.permissions.access == Access::Read;
        let ui_before = self.s.ui_requests.len();
        let r = self.s.grouped(command, |s| {
            let doc = s.ds()?.doc.clone();
            let v = s.execute(command, &params)?;
            let opened_ui = s.ui_requests.len() > ui_before;
            s.ui_requests.truncate(ui_before);
            if opened_ui {
                return Err(EngineError::Disabled(command.into(), "it opens a dialog or panel, which plugins can't".into()));
            }
            if read_only && !Arc::ptr_eq(&doc, &s.ds()?.doc) {
                return Err(EngineError::Disabled(command.into(), "it changed the document, and this plugin may only read it".into()));
            }
            Ok(v)
        });
        r.map(|mut v| {
            // A plugin gets the open files' names, not where they are on disk (#1117).
            if command == "doc.list" {
                for e in v.as_array_mut().into_iter().flatten().filter_map(Value::as_object_mut) {
                    e.remove("path");
                }
            }
            v.to_string()
        })
        .map_err(|e| e.to_string())
    }

    fn selection(&mut self) -> String {
        json!(self.s.selection()).to_string()
    }

    fn notify(&mut self, message: &str) {
        self.s.message = Some(message.to_string());
    }

    fn log(&mut self, line: &str) {
        log::info!("plugin {}: {line}", self.manifest.id);
    }

    fn setting(&mut self, key: &str) -> Option<String> {
        Some(self.s.prefs.plugin_settings.get(&self.manifest.id)?.get(key)?.to_string())
    }

    fn set_setting(&mut self, key: &str, value: Option<&str>) -> std::result::Result<(), String> {
        let (all, id) = (&mut self.s.prefs.plugin_settings, &self.manifest.id);
        let Some(text) = value else {
            if let Some(mine) = all.get_mut(id)
                && mine.remove(key).is_some()
            {
                if mine.is_empty() {
                    all.remove(id);
                }
                self.s.plugins.unsaved = true;
            }
            return Ok(());
        };
        if key.is_empty() || key.chars().count() > MAX_SETTING_KEY || key.chars().any(char::is_control) {
            return Err(format!("a setting's key must be 1–{MAX_SETTING_KEY} characters without control characters"));
        }
        let too_big = || format!("a plugin's settings are limited to {} KB", MAX_SETTINGS_BYTES / 1024);
        if key.len() + text.len() > MAX_SETTINGS_BYTES {
            return Err(too_big());
        }
        let v: Value = serde_json::from_str(text).map_err(|e| format!("a setting must be JSON: {e}"))?;
        let others: usize =
            all.get(id).map_or(0, |mine| mine.iter().filter(|(k, _)| k.as_str() != key).map(|(k, v)| k.len() + v.to_string().len()).sum());
        if others + key.len() + v.to_string().len() > MAX_SETTINGS_BYTES {
            return Err(too_big());
        }
        if all.entry(id.clone()).or_default().insert(key.to_string(), v.clone()).as_ref() != Some(&v) {
            self.s.plugins.unsaved = true;
        }
        Ok(())
    }

    fn save_file(&mut self, name: &str, mime: &str, data: &[u8]) -> std::result::Result<(), String> {
        if !self.manifest.permissions.save_file {
            return Err(format!("{} didn't ask permission to save files (`permissions.saveFile`)", self.manifest.name));
        }
        let name = sanitize_save_name(name).ok_or_else(|| {
            format!("not a usable file name: use a plain name of 1–{MAX_SAVE_NAME} characters without folders or `/ \\ : .. < > \" | ? *`")
        })?;
        if data.len() > MAX_SAVE_BYTES {
            return Err(format!("a file to save is limited to {} MB", MAX_SAVE_BYTES / (1024 * 1024)));
        }
        if self.saves >= MAX_SAVES {
            return Err("a plugin can offer one file per run".into());
        }
        self.saves += 1;
        use base64::Engine as _;
        let data = base64::engine::general_purpose::STANDARD.encode(data);
        // The app asks the user where it goes; programmatic callers drop the request.
        self.s.request_ui("dialog.pluginSave", json!({"plugin": self.manifest.id, "name": name, "mime": sanitize_mime(mime), "data": data}));
        Ok(())
    }
}

/// The installed plugin `id`.
pub(crate) fn installed<'a>(s: &'a Session, cmd: &str, id: &str) -> Result<&'a InstalledPlugin> {
    s.prefs.plugins.iter().find(|p| p.manifest.id == id).ok_or_else(|| bad(cmd, format!("no plugin `{id}` is installed")))
}

/// Read a file of at most `max` bytes.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_file(path: &std::path::Path, max: usize) -> Result<Vec<u8>> {
    crate::file_access::read_capped(path, max as u64)
}

/// The manifest at `path` (the file, or the folder holding `figcraft-plugin.json`), its absolute
/// path and its module.
#[cfg(not(target_arch = "wasm32"))]
fn load_from_disk(cmd: &str, path: &str) -> Result<(Manifest, String, Vec<u8>)> {
    let mut file = std::path::PathBuf::from(crate::library::absolute_path(path.trim()));
    if crate::file_access::is_dir(&file) {
        file.push(MANIFEST_FILE);
    }
    let text = read_file(&file, figcraft_plugin::manifest::MAX_MANIFEST_BYTES)?;
    let text = String::from_utf8(text).map_err(|_| bad(cmd, "the manifest isn't UTF-8 text"))?;
    let manifest = Manifest::parse(&text).map_err(|e| bad(cmd, e))?;
    let wasm = module_file(&file, &manifest)?;
    Ok((manifest, file.to_string_lossy().into_owned(), wasm))
}

#[cfg(target_arch = "wasm32")]
fn load_from_disk(cmd: &str, _path: &str) -> Result<(Manifest, String, Vec<u8>)> {
    Err(bad(cmd, "there are no files in the browser: pass `manifest` and `wasm` (base64)"))
}

/// The module a manifest at `manifest_file` names.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn module_file(manifest_file: &std::path::Path, m: &Manifest) -> Result<Vec<u8>> {
    let dir = manifest_file.parent().unwrap_or(std::path::Path::new("."));
    read_file(&dir.join(&m.main), Limits::default().module_bytes)
}

/// The module of an installed plugin: from the session, else read again from disk (so a rebuilt
/// development plugin runs its new code).
pub(crate) fn module_of(s: &Session, p: &InstalledPlugin) -> Result<Arc<Vec<u8>>> {
    if let Some(m) = s.plugins.modules.get(&p.manifest.id) {
        return Ok(m.clone());
    }
    let Some(path) = &p.path else {
        return Err(EngineError::Other(format!("{} was imported without a manifest file: import it again", p.manifest.name)));
    };
    module_from_disk(s, p, path)
}

#[cfg(not(target_arch = "wasm32"))]
fn module_from_disk(s: &Session, p: &InstalledPlugin, path: &str) -> Result<Arc<Vec<u8>>> {
    let file = std::path::Path::new(path);
    let read = || module_file(file, &p.manifest).map(Arc::new);
    // A plugin installed from a directory lives in the app's own install folder, not among the
    // files automation roots confine.
    let own = p.source.is_some() && s.plugins.install_dir.as_deref().is_some_and(|d| crate::file_access::within_app_folder(file, d));
    if own { crate::file_access::app_owned(read) } else { read() }
}

#[cfg(target_arch = "wasm32")]
fn module_from_disk(_: &Session, _: &InstalledPlugin, path: &str) -> Result<Arc<Vec<u8>>> {
    Err(EngineError::Other(format!("{path}: there are no files in the browser")))
}

/// The sandbox limits for a run, with `fuel` lowering the instruction budget.
pub(crate) fn limits(fuel: Option<f64>) -> Limits {
    let mut limits = Limits::default();
    if let Some(f) = fuel.filter(|f| f.is_finite()) {
        limits.fuel = (f.max(1000.0) as u64).min(limits.fuel);
    }
    limits
}

pub(crate) fn plugin_error(name: &str, e: PluginError) -> EngineError {
    EngineError::Other(format!("{name}: {e}"))
}

/// A plugin as `plugin.list` reports it.
fn describe(s: &Session, p: &InstalledPlugin) -> Value {
    let mut v = serde_json::to_value(&p.manifest).unwrap_or_else(|_| json!({"id": p.manifest.id}));
    v["path"] = json!(p.path);
    v["source"] = json!(p.source);
    v["loaded"] = json!(p.path.is_some() || s.plugins.modules.contains_key(&p.manifest.id));
    v
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let plugins: Vec<Value> = s.prefs.plugins.iter().map(|p| describe(s, p)).collect();
    let last = s.plugins.last.as_ref().map(|(id, command, _)| json!({"id": id, "command": command}));
    Ok(json!({"plugins": plugins, "last": last}))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.import";
    let (manifest, path, wasm) = match (string(p, "path"), get(p, "manifest")) {
        (Some(path), _) => {
            let (m, path, wasm) = load_from_disk(cmd, &path)?;
            (m, Some(path), wasm)
        }
        (None, Some(m)) => {
            let m = match m {
                Value::String(text) => Manifest::parse(text),
                other => Manifest::from_value(other),
            }
            .map_err(|e| bad(cmd, e))?;
            use base64::Engine as _;
            let b64 = string(p, "wasm").ok_or_else(|| bad(cmd, "pass the module as `wasm` (base64) with `manifest`"))?;
            if b64.len() > Limits::default().module_bytes / 3 * 4 + 4 {
                return Err(bad(cmd, "the module is too large"));
            }
            let wasm = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| bad(cmd, format!("`wasm`: {e}")))?;
            (m, None, wasm)
        }
        (None, None) => return Err(bad(cmd, "pass `path` (a figcraft-plugin.json or its folder), or `manifest` and `wasm`")),
    };
    figcraft_plugin::check_module(&wasm, &Limits::default()).map_err(|e| plugin_error(&manifest.name, e))?;
    let mut out = describe(s, &InstalledPlugin { manifest: manifest.clone(), path: path.clone(), source: None });
    out["replaced"] = json!(register(s, InstalledPlugin { manifest, path, source: None }, wasm));
    out["loaded"] = json!(true);
    Ok(out)
}

/// Install `entry` (its module `wasm` is kept in the session when it has no manifest file),
/// replacing a plugin with the same id (whose panel closes); whether one was replaced.
pub(crate) fn register(s: &mut Session, entry: InstalledPlugin, wasm: Vec<u8>) -> bool {
    let id = entry.manifest.id.clone();
    if entry.path.is_none() {
        s.plugins.modules.insert(id.clone(), Arc::new(wasm));
    } else {
        s.plugins.modules.remove(&id);
    }
    close_panel(s, &id);
    s.plugins.unsaved = true;
    let list = &mut s.prefs.plugins;
    match list.iter_mut().find(|x| x.manifest.id == id) {
        Some(slot) => {
            *slot = entry;
            s.prefs.plugin_settings.remove(&id);
            true
        }
        None => {
            list.push(entry);
            false
        }
    }
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.remove";
    let id = string(p, "id").ok_or_else(|| bad(cmd, "`id` required"))?;
    let p = installed(s, cmd, &id)?.clone();
    crate::plugin_directory::delete_installed(s, &p);
    s.prefs.plugins.retain(|x| x.manifest.id != id);
    s.prefs.plugin_settings.remove(&id);
    s.plugins.unsaved = true;
    s.plugins.modules.remove(&id);
    close_panel(s, &id);
    if s.plugins.last.as_ref().is_some_and(|(last, ..)| *last == id) {
        s.plugins.last = None;
    }
    Ok(json!({"removed": id}))
}

/// Run plugin `id`'s `command` with `params` as one undo step.
pub fn run(s: &mut Session, id: &str, command: Option<&str>, params: &Value, fuel: Option<f64>) -> Result<Value> {
    let cmd = "plugin.run";
    let mut p = installed(s, cmd, id)?.clone();
    // Checked again: the preferences file may have been edited by hand.
    p.manifest = Manifest::from_value(&json!(p.manifest)).map_err(|e| bad(cmd, e))?;
    let m = &p.manifest;
    let c = m.command(command).ok_or_else(|| bad(cmd, format!("{} has no command `{}`", m.name, command.unwrap_or_default())))?;
    let params = c.resolve_params(params).map_err(|e| bad(cmd, e))?;
    let wasm = module_of(s, &p)?;
    let limits = limits(fuel);
    // The API the plugin was written for: a v1 plugin gets the input it always got.
    let input = json!({"api": m.api, "command": c.id, "params": params, "selection": s.selection()}).to_string();
    s.plugins.last = Some((m.id.clone(), c.id.clone(), params.clone()));
    let label = if m.commands.len() > 1 { format!("{}: {}", m.name, c.label.trim_end_matches(['…', '.'])) } else { m.name.clone() };
    let mut instance = Instance::new(&wasm, &limits).map_err(|e| plugin_error(&m.name, e))?;
    let mut out = match call_plugin(s, m, &label, &mut instance, &input) {
        Ok(out) => out,
        Err((e, _)) => {
            record_run(
                s,
                PluginRunRecord {
                    plugin: m.id.clone(),
                    command: c.id.clone(),
                    ok: false,
                    output: Value::Null,
                    error: Some(e.to_string()),
                    logs: Vec::new(),
                    calls: 0,
                    fuel: 0,
                    fuel_limit: limits.fuel,
                },
            );
            return Err(e);
        }
    };
    match out.ui.take() {
        Some(UiChange::Show(panel)) => open_panel(s, m, (&c.id, params, label), panel, instance, None),
        Some(UiChange::Close) => close_panel(s, &m.id),
        None => {}
    }
    let res = report(s, &m.id, &c.id, out);
    record_run(
        s,
        PluginRunRecord {
            plugin: m.id.clone(),
            command: c.id.clone(),
            ok: true,
            output: res["output"].clone(),
            error: None,
            logs: res["logs"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            calls: res["calls"].as_u64().unwrap_or(0) as u32,
            fuel: res["fuel"].as_u64().unwrap_or(0),
            fuel_limit: limits.fuel,
        },
    );
    Ok(res)
}

/// Call the plugin with `input` as one undo step (none when it changed nothing), rolled back when
/// it fails. On failure, also whether the module is still fit for more calls: it returned an
/// error itself rather than trapping or running out of fuel or memory.
pub(crate) fn call_plugin(
    s: &mut Session,
    m: &Manifest,
    label: &str,
    instance: &mut Instance,
    input: &str,
) -> std::result::Result<RunOutput, (EngineError, bool)> {
    let mut usable = false;
    let ui_before = s.ui_requests.len();
    let r = s
        .grouped(label, |s| {
            let mut host = EngineHost { s, manifest: m, saves: 0 };
            instance.call(input, &mut host).map_err(|e| {
                usable = matches!(e, PluginError::Failed(_));
                plugin_error(&m.name, e)
            })
        })
        .map_err(|e| (e, usable));
    // A failed run opens no dialog (a file it offered to save is dropped with it).
    if r.is_err() {
        s.ui_requests.truncate(ui_before);
    }
    r
}

/// What `plugin.run` and `plugin.event` return.
fn report(s: &Session, id: &str, command: &str, out: RunOutput) -> Value {
    let output = serde_json::from_str::<Value>(&out.output).unwrap_or(Value::String(out.output));
    let panel = s.plugins.panel(id).map(OpenPanel::describe);
    json!({"plugin": id, "command": command, "output": output, "logs": out.logs, "calls": out.calls, "fuel": out.fuel, "panel": panel})
}

/// Close plugin `id`'s panel, if it has one open.
pub(crate) fn close_panel(s: &mut Session, id: &str) {
    s.plugins.panels.retain(|o| o.manifest.id != id);
}

fn ui(s: &mut Session, p: &Value) -> Result<Value> {
    let open = match string(p, "id") {
        Some(id) => s.plugins.panel(&id),
        None => s.plugins.panels.last(),
    };
    Ok(open.map_or(Value::Null, OpenPanel::describe))
}

/// A button press or a changed value in a plugin's panel: the plugin is called back with the
/// event and every value, as one undo step.
fn event(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.event";
    let id = string(p, "id").ok_or_else(|| bad(cmd, "`id` required (the plugin whose panel is open)"))?;
    let control = string(p, "control").ok_or_else(|| bad(cmd, "`control` required (a control's name)"))?;
    let at = s.plugins.panels.iter().position(|o| o.manifest.id == id).ok_or_else(|| bad(cmd, format!("`{id}` has no panel open")))?;
    let open = s.plugins.panels.get(at).ok_or_else(|| bad(cmd, "no panel is open"))?;
    let c = open
        .panel
        .control(&control)
        .filter(|c| !matches!(c.kind, ParamKind::Note | ParamKind::Image))
        .ok_or_else(|| bad(cmd, format!("the panel has no control `{control}`")))?;
    let field = c.is_field();
    let value = if field { c.coerce(get(p, "value").unwrap_or(&Value::Null)).map_err(|e| bad(cmd, e))? } else { Value::Null };
    let mut open = s.plugins.panels.remove(at);
    if field {
        open.values.insert(control.clone(), value.clone());
    }
    let command = open.command.clone();
    let event = json!({"control": control, "value": value});
    let input = json!({"api": open.manifest.api, "command": open.command, "params": open.params, "selection": s.selection(), "event": event, "values": open.values});
    let called = match open.widget {
        Some(w) => crate::widgets::panel_callback(s, &open.manifest, &open.label, &mut open.instance, input, w),
        None => call_plugin(s, &open.manifest, &open.label, &mut open.instance, &input.to_string()),
    };
    let mut out = match called {
        Ok(out) => out,
        Err((e, usable)) => {
            // A plugin that returned an error keeps its panel; one that crashed loses it.
            if usable {
                keep(s, at, open);
            }
            record_run(
                s,
                PluginRunRecord {
                    plugin: id,
                    command,
                    ok: false,
                    output: Value::Null,
                    error: Some(e.to_string()),
                    logs: Vec::new(),
                    calls: 0,
                    fuel: 0,
                    fuel_limit: Limits::default().fuel,
                },
            );
            return Err(e);
        }
    };
    match out.ui.take() {
        Some(UiChange::Show(panel)) => {
            open.values = panel.values(&open.values);
            open.panel = panel;
            keep(s, at, open);
        }
        Some(UiChange::Close) => {}
        None => keep(s, at, open),
    }
    let res = report(s, &id, &command, out);
    record_run(
        s,
        PluginRunRecord {
            plugin: id,
            command,
            ok: true,
            output: res["output"].clone(),
            error: None,
            logs: res["logs"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            calls: res["calls"].as_u64().unwrap_or(0) as u32,
            fuel: res["fuel"].as_u64().unwrap_or(0),
            fuel_limit: Limits::default().fuel,
        },
    );
    Ok(res)
}

/// Put a panel taken out for a callback back in its place among the open ones.
fn keep(s: &mut Session, at: usize, open: OpenPanel) {
    let at = at.min(s.plugins.panels.len());
    s.plugins.panels.insert(at, open);
}

fn close(s: &mut Session, p: &Value) -> Result<Value> {
    let id = string(p, "id").ok_or_else(|| bad("plugin.close", "`id` required"))?;
    let closed = s.plugins.panel(&id).is_some();
    close_panel(s, &id);
    Ok(json!({"closed": closed}))
}

fn run_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let id = string(p, "id").ok_or_else(|| bad("plugin.run", "`id` required (see plugin.list)"))?;
    let command = string(p, "command");
    run(s, &id, command.as_deref(), get(p, "params").unwrap_or(&Value::Null), num(p, "fuel"))
}

fn run_last(s: &mut Session, _p: &Value) -> Result<Value> {
    let Some((id, command, params)) = s.plugins.last.clone() else {
        return Err(EngineError::Disabled("plugins.runLast".into(), "no plugin has run yet".into()));
    };
    run(s, &id, Some(&command), &params, None)
}

fn manage(s: &mut Session, p: &Value) -> Result<Value> {
    s.request_ui("dialog.plugins", json!({}));
    list(s, p)
}

/// Reload an installed local development plugin from its manifest on disk, preserving its saved settings.
fn reload(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.reload";
    let id = string(p, "id").ok_or_else(|| bad(cmd, "`id` required"))?;
    let existing = installed(s, cmd, &id)?.clone();
    let Some(path) = existing.path.as_deref() else {
        return Err(bad(cmd, format!("`{id}` was imported without a manifest file and cannot be reloaded from disk")));
    };
    let (manifest, abs_path, wasm) = load_from_disk(cmd, path)?;
    if manifest.id != id {
        return Err(bad(cmd, format!("the manifest id changed from `{id}` to `{}`: remove the plugin and import it again", manifest.id)));
    }
    figcraft_plugin::check_module(&wasm, &Limits::default()).map_err(|e| plugin_error(&manifest.name, e))?;
    let name = manifest.name.clone();
    let version = manifest.version.clone();
    let new_id = manifest.id.clone();
    let settings = s.prefs.plugin_settings.get(&existing.manifest.id).cloned();
    register(s, InstalledPlugin { manifest, path: Some(abs_path), source: existing.source }, wasm);
    if let Some(v) = settings {
        s.prefs.plugin_settings.insert(new_id.clone(), v);
    }
    Ok(json!({"id": new_id, "name": name, "version": version, "reloaded": true}))
}

/// Convert a human-readable plugin name into a lowercase ASCII hyphen-separated slug.
fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "my-plugin".into()
    } else if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
        format!("plugin-{trimmed}")
    } else {
        trimmed
    }
}

/// Compute a forward-slash relative path from `base_dir` to `target_dir` when they share a root.
#[cfg(not(target_arch = "wasm32"))]
fn relative_sdk_path(base_dir: &std::path::Path, target_dir: &std::path::Path) -> String {
    use std::path::Component;
    let base: Vec<Component<'_>> = base_dir.components().filter(|c| !matches!(c, Component::CurDir)).collect();
    let target: Vec<Component<'_>> = target_dir.components().filter(|c| !matches!(c, Component::CurDir)).collect();
    let common = base.iter().zip(target.iter()).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return target_dir.to_string_lossy().replace('\\', "/");
    }
    let mut parts: Vec<String> = Vec::new();
    for c in &base[common..] {
        if matches!(c, Component::Normal(_)) {
            parts.push("..".into());
        }
    }
    for c in &target[common..] {
        if let Component::Normal(s) = c {
            parts.push(s.to_string_lossy().into_owned());
        }
    }
    if parts.is_empty() { ".".into() } else { parts.join("/") }
}

/// Resolve the path to `figcraft-plugin-sdk` relative to `project_dir`.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_sdk_path(cmd: &str, project_dir: &std::path::Path, explicit: Option<&str>) -> Result<String> {
    if let Some(sdk) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let abs = std::path::PathBuf::from(crate::library::absolute_path(sdk));
        return Ok(relative_sdk_path(project_dir, &abs));
    }
    if let Some(env_sdk) = std::env::var_os("FIGCRAFT_PLUGIN_SDK").filter(|v| !v.is_empty()) {
        let abs = std::path::PathBuf::from(crate::library::absolute_path(&env_sdk.to_string_lossy()));
        return Ok(relative_sdk_path(project_dir, &abs));
    }
    if let Some(repo_root) = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(std::path::Path::parent) {
        let sdk_dir = repo_root.join("plugins").join("sdk");
        if crate::file_access::exists(sdk_dir.join("Cargo.toml")) {
            return Ok(relative_sdk_path(project_dir, &sdk_dir));
        }
    }
    Err(bad(cmd, "couldn't determine the `figcraft-plugin-sdk` location; set `sdk` or `FIGCRAFT_PLUGIN_SDK`"))
}

/// Scaffold a new plugin project on disk from `template` and import it into the session.
#[cfg(not(target_arch = "wasm32"))]
fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.create";
    let dir_str = string(p, "dir").filter(|d| !d.trim().is_empty()).ok_or_else(|| bad(cmd, "`dir` required"))?;
    let dir = std::path::PathBuf::from(crate::library::absolute_path(dir_str.trim()));
    let name = string(p, "name")
        .filter(|n| !n.trim().is_empty())
        .or_else(|| dir.file_name().and_then(|n| n.to_str()).map(str::to_string))
        .unwrap_or_else(|| "My Plugin".into());
    let slug = slugify(&name);
    let id = string(p, "id").filter(|i| !i.trim().is_empty()).unwrap_or_else(|| format!("org.example.{slug}"));
    let template = string(p, "template").unwrap_or_else(|| "command".into()).to_ascii_lowercase();
    let wasm_file = format!("{}.wasm", slug.replace('-', "_"));
    let name_lit = name.escape_default().to_string();
    let name_fmt = name.replace('{', "{{").replace('}', "}}").escape_default().to_string();

    let (manifest_val, lib_rs, fallback_call): (Value, String, Option<crate::plugin_directory::FallbackCall>) = match template.as_str() {
        "command" => (
            json!({
                "id": id,
                "name": name,
                "version": "0.1.0",
                "api": 4,
                "description": "A custom FigCraft command plugin.",
                "main": wasm_file,
                "permissions": {
                    "access": "write",
                    "commands": ["node.get", "node.set", "shape.rectangle"]
                },
                "commands": [
                    { "id": "run", "label": name }
                ]
            }),
            format!(
                r#"use figcraft_plugin_sdk::{{Input, Result, Value, entry, json, log, notify}};

entry!(run);

fn run(input: Input) -> Result<Value> {{
    log(&format!("Running {{}} on {{}} selected layer(s)", input.command, input.selection.len()));
    notify("Hello from {name_lit}!");
    Ok(json!({{"selected": input.selection.len()}}))
}}
"#
            ),
            Some(("notify", false, b"Hello from plugin!")),
        ),
        "panel" => (
            json!({
                "id": id,
                "name": name,
                "version": "0.1.0",
                "api": 4,
                "description": "An interactive panel plugin for FigCraft.",
                "main": wasm_file,
                "permissions": {
                    "access": "write",
                    "commands": ["shape.rectangle", "node.set"]
                },
                "commands": [
                    { "id": "open", "label": format!("{name}…") }
                ]
            }),
            format!(
                r##"use figcraft_plugin_sdk::{{Input, Result, Value, call, entry, json, notify, show_ui}};

entry!(run);

fn run(input: Input) -> Result<Value> {{
    if let Some(ev) = &input.event {{
        if ev.control == "create" {{
            let color = ev.values.get("color").and_then(Value::as_str).unwrap_or("#02da96");
            call("shape.rectangle", &json!({{"x": 0, "y": 0, "width": 120, "height": 120, "props": {{"fill": color}}}}))?;
            notify("Created rectangle");
        }}
        return Ok(Value::Null);
    }}
    show_ui(&json!({{
        "title": "{name_lit}",
        "controls": [
            {{"name": "color", "label": "Color", "type": "color", "default": "#02da96"}},
            {{"name": "create", "label": "Create Rectangle", "type": "button"}}
        ]
    }}))?;
    Ok(Value::Null)
}}
"##
            ),
            Some((
                "ui_show",
                true,
                br##"{"title":"Plugin Panel","controls":[{"name":"color","label":"Color","type":"color","default":"#02da96"},{"name":"create","label":"Create Rectangle","type":"button"}]}"##,
            )),
        ),
        "widget" => (
            json!({
                "id": id,
                "name": name,
                "version": "0.1.0",
                "api": 4,
                "description": "An interactive canvas widget plugin.",
                "main": wasm_file,
                "widget": true,
                "permissions": {
                    "access": "read"
                }
            }),
            format!(
                r##"use figcraft_plugin_sdk::{{Input, Result, Value, entry, json, render_widget}};

entry!(run);

fn run(input: Input) -> Result<Value> {{
    let mut count = input
        .widget
        .as_ref()
        .and_then(|w| w.state.get("count"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if input.widget.as_ref().and_then(|w| w.target.as_deref()) == Some("inc") {{
        count += 1;
    }}
    render_widget(&json!({{
        "state": {{"count": count}},
        "root": {{
            "type": "frame",
            "name": "{name_lit}",
            "layout": "horizontal",
            "padding": 10,
            "gap": 8,
            "fill": "#ffffff",
            "stroke": "#d8d8d8",
            "radius": 8,
            "children": [
                {{"type": "text", "text": format!("{name_fmt}: {{count}}"), "size": 14, "color": "#1e1e1e"}},
                {{"type": "frame", "name": "Add", "target": "inc", "layout": "horizontal", "padding": 6, "fill": "#02da96", "radius": 4, "children": [
                    {{"type": "text", "text": "+1", "size": 12, "bold": true, "color": "#ffffff"}}
                ]}}
            ]
        }}
    }}))?;
    Ok(json!({{"count": count}}))
}}
"##
            ),
            Some((
                "widget_render",
                true,
                br##"{"state":{"count":0},"root":{"type":"frame","name":"Widget","layout":"horizontal","padding":10,"gap":8,"fill":"#ffffff","stroke":"#d8d8d8","radius":8,"children":[{"type":"text","text":"Count: 0","size":14,"color":"#1e1e1e"},{"type":"frame","name":"Add","target":"inc","layout":"horizontal","padding":6,"fill":"#02da96","radius":4,"children":[{"type":"text","text":"+1","size":12,"bold":true,"color":"#ffffff"}]}]}}"##,
            )),
        ),
        "exporter" => (
            json!({
                "id": id,
                "name": name,
                "version": "0.1.0",
                "api": 4,
                "description": "Exports data from the current document to a file.",
                "main": wasm_file,
                "permissions": {
                    "access": "read",
                    "saveFile": true,
                    "commands": ["node.list"]
                },
                "commands": [
                    { "id": "export", "label": format!("{name}…") }
                ]
            }),
            r#"use figcraft_plugin_sdk::{Input, Result, Value, call, entry, json, save_file};

entry!(run);

fn run(_input: Input) -> Result<Value> {
    let nodes = call("node.list", &json!({}))?;
    let bytes = serde_json::to_vec_pretty(&nodes).map_err(|e| e.to_string())?;
    save_file("layers.json", "application/json", &bytes)?;
    Ok(json!({"saved": "layers.json"}))
}
"#
            .to_string(),
            Some(("set_output", false, br#"{"saved":"layers.json"}"#)),
        ),
        other => {
            return Err(bad(
                cmd,
                format!("unknown template `{other}` (expected command, panel, widget or exporter)"),
            ));
        }
    };

    let manifest_text = serde_json::to_string_pretty(&manifest_val).map_err(|e| bad(cmd, e.to_string()))?;
    // Validate the generated manifest before touching the disk.
    Manifest::parse(&manifest_text).map_err(|e| bad(cmd, e))?;

    for rel in [MANIFEST_FILE, "Cargo.toml", "src/lib.rs", wasm_file.as_str()] {
        if crate::file_access::exists(dir.join(rel)) {
            return Err(bad(cmd, format!("`{rel}` already exists in `{}`", dir.display())));
        }
    }

    let sdk_path = resolve_sdk_path(cmd, &dir, string(p, "sdk").as_deref())?;
    let cargo_toml = format!(
        r#"[package]
name = "{slug}"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
figcraft-plugin-sdk = {{ path = {sdk_path:?} }}
serde_json = "1"
"#
    );

    crate::file_access::create_dir_all(dir.join("src"))?;
    let manifest_path = dir.join(MANIFEST_FILE);
    crate::file_access::write(&manifest_path, manifest_text.as_bytes())?;
    crate::file_access::write(dir.join("Cargo.toml"), cargo_toml.as_bytes())?;
    crate::file_access::write(dir.join("src").join("lib.rs"), lib_rs.as_bytes())?;
    let stub_wasm = crate::plugin_directory::encode_wasm_module(fallback_call);
    crate::file_access::write(dir.join(&wasm_file), &stub_wasm)?;

    let manifest_path_str = manifest_path.to_string_lossy().into_owned();
    let _ = import(s, &json!({"path": manifest_path_str}))?;
    Ok(json!({
        "id": id,
        "name": name,
        "dir": dir.to_string_lossy(),
        "manifest": manifest_path_str,
        "template": template,
    }))
}

/// Stub for `plugin.create` on `wasm32` where local project creation on disk is unavailable.
#[cfg(target_arch = "wasm32")]
fn create(_s: &mut Session, _p: &Value) -> Result<Value> {
    Err(bad("plugin.create", "creating plugin projects on disk requires the desktop app or CLI"))
}

/// Locate the `cargo` executable for building WebAssembly plugins.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)]
pub(crate) fn cargo_binary() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(c) = std::env::var_os("CARGO").filter(|v| !v.is_empty()).map(PathBuf::from)
        && c.is_file()
    {
        return c;
    }
    let name = if cfg!(windows) { "cargo.exe" } else { "cargo" };
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if let Some(h) = home {
        let candidate = h.join(".cargo").join("bin").join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(name)
}

/// Resolve Cargo's effective `target_directory` for the plugin manifest in `root` via
/// `cargo metadata`, preserving `CARGO_TARGET_DIR` and workspace target directories, and falling
/// back to `CARGO_TARGET_DIR` or `<root>/target` if metadata cannot be queried.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods, clippy::disallowed_types)]
pub(crate) fn cargo_target_dir(cargo: &std::path::Path, root: &std::path::Path) -> std::path::PathBuf {
    let cargo_toml = root.join("Cargo.toml");
    if let Ok(output) = std::process::Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1", "--manifest-path"])
        .arg(&cargo_toml)
        .current_dir(root)
        .output()
        && output.status.success()
        && let Ok(meta) = serde_json::from_slice::<Value>(&output.stdout)
        && let Some(dir) = meta["target_directory"].as_str().filter(|s| !s.is_empty())
    {
        return std::path::PathBuf::from(dir);
    }
    std::env::var_os("CARGO_TARGET_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from).unwrap_or_else(|| root.join("target"))
}

/// Extract the expected `<crate_name>.wasm` filename from a plugin's `Cargo.toml`, if present.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)]
fn crate_wasm_name(root: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_pkg = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_pkg = line == "[package]";
            continue;
        }
        if in_pkg
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "name"
        {
            let name = v.trim().trim_matches('"').trim_matches('\'');
            if !name.is_empty() {
                return Some(format!("{}.wasm", name.replace('-', "_")));
            }
        }
    }
    None
}

/// Build a Rust plugin project at `manifest_or_dir` for `wasm32-unknown-unknown` in release mode
/// and copy the resulting `.wasm` artifact to the manifest's `main` path inside the plugin folder.
/// Returns `(manifest_path, wasm_out_path)`.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods, clippy::disallowed_types)]
pub fn build_plugin_project(manifest_or_dir: &std::path::Path) -> std::result::Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let manifest_path = if manifest_or_dir.file_name().is_some_and(|n| n == MANIFEST_FILE) {
        manifest_or_dir.to_path_buf()
    } else {
        manifest_or_dir.join(MANIFEST_FILE)
    };
    let root = manifest_path.parent().unwrap_or(manifest_or_dir);
    let raw = std::fs::read_to_string(&manifest_path).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = Manifest::parse(&raw).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let main_rel = std::path::Path::new(&manifest.main);
    if main_rel.is_absolute()
        || main_rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_)))
    {
        return Err(format!("{}: `main` must be a relative path inside the plugin folder", manifest_path.display()));
    }
    let wasm_out = root.join(main_rel);
    let cargo = cargo_binary();
    let output = std::process::Command::new(&cargo)
        .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            "cargo build --release --target wasm32-unknown-unknown failed".into()
        } else {
            format!("cargo build failed:\n{detail}")
        });
    }
    let target_base = cargo_target_dir(&cargo, root);
    let rel = target_base.join("wasm32-unknown-unknown").join("release");
    let expected = crate_wasm_name(root).map(|n| rel.join(n)).filter(|p| p.is_file());
    let built_wasm = match expected {
        Some(p) => p,
        None => {
            let entries = std::fs::read_dir(&rel).map_err(|e| format!("{}: {e}", rel.display()))?;
            let mut candidates: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|e| format!("{}: {e}", rel.display()))?;
                let p = entry.path();
                if p.extension().is_some_and(|ext| ext == "wasm") && p.is_file() {
                    let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    candidates.push((mtime, p));
                }
            }
            candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            candidates.pop().map(|(_, p)| p).ok_or_else(|| format!("no `.wasm` artifact found in {}", rel.display()))?
        }
    };
    if built_wasm != wasm_out {
        if let Some(parent) = wasm_out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(&built_wasm, &wasm_out).map_err(|e| format!("couldn't copy {} to {}: {e}", built_wasm.display(), wasm_out.display()))?;
    }
    if !wasm_out.is_file() {
        return Err(format!("expected built module at {} does not exist", wasm_out.display()));
    }
    Ok((manifest_path, wasm_out))
}

/// Return recent plugin execution history (`plugin.logs`), optionally filtered by `id` or cleared.
fn logs(s: &mut Session, p: &Value) -> Result<Value> {
    let filter_id = string(p, "id");
    if get(p, "clear") == Some(&Value::Bool(true)) {
        match &filter_id {
            Some(id) => s.plugins.history.retain(|r| r.plugin != *id),
            None => s.plugins.history.clear(),
        }
    }
    let runs: Vec<&PluginRunRecord> = s.plugins.history.iter().filter(|r| filter_id.as_ref().is_none_or(|id| r.plugin == *id)).collect();
    Ok(json!({"runs": runs}))
}

/// Request the Plugin Dev Console dialog (`plugins.console`) and return recent execution logs.
fn console(s: &mut Session, p: &Value) -> Result<Value> {
    s.request_ui("dialog.pluginConsole", json!({}));
    logs(s, p)
}

/// Request the New Plugin dialog (`plugin.newDialog`).
fn new_dialog(s: &mut Session, _p: &Value) -> Result<Value> {
    s.request_ui("dialog.pluginNew", json!({}));
    crate::cmd::ok()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("plugin.list", "List plugins", "{} → {plugins: [{id, name, version, main, permissions, commands, path, loaded}], last}", false, list),
        spec!(
            "plugin.import",
            "Import plugin from manifest…",
            "{path (figcraft-plugin.json or its folder) | manifest (object or JSON text), wasm (base64)} replaces a plugin with the same id",
            false,
            import
        ),
        spec!("plugin.remove", "Remove plugin", "{id}", false, remove),
        spec!(
            "plugin.run",
            "Run plugin",
            "{id, command? (default: the first), params? (the command's parameters), fuel? (lower instruction budget)} one undo step → {output, logs, calls, fuel, panel}",
            false,
            run_cmd
        ),
        spec!(
            "plugin.ui",
            "Plugin panel",
            "{id?} → plugin id's open panel (without id: the one opened last) {plugin, command, title, controls, values}, or null",
            false,
            ui
        ),
        spec!(
            "plugin.event",
            "Use plugin panel control",
            "{id, control, value? (a field's new value)} presses a button or sets a field in the plugin's open panel; the plugin is called back as one undo step → like plugin.run",
            false,
            event
        ),
        spec!("plugin.close", "Close plugin panel", "{id} → {closed}", false, close),
        spec!("plugin.reload", "Reload plugin from disk", "{id} → {id, name, version, reloaded}", false, reload),
        spec!(
            "plugin.create",
            "Create new plugin project",
            "{dir, name?, id?, template? (command|panel|widget|exporter)} → {id, name, dir, manifest, template}",
            false,
            create
        ),
        spec!("plugin.logs", "Plugin execution logs", "{id?, clear?} → {runs}", false, logs),
        spec!("plugin.newDialog", "New plugin…", "{}", false, new_dialog),
        spec!("plugins.console", "Plugin console…", "{id?, clear?} → {runs}", false, console),
        spec!("plugins.runLast", "Run last plugin", "{}", false, run_last),
        spec!("plugins.manage", "Manage plugins…", "{}  opens the plugins window; returns the list like `plugin.list`", false, manage),
    ]
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    #[test]
    fn read_only_plugin_cannot_publish_to_server() {
        let m = Manifest::parse(r#"{"id":"x.y","name":"X","main":"plugin.wasm"}"#).unwrap();
        let r = refusal(&m, "library.publishToServer", &json!({}));
        assert!(r.is_some(), "library.publishToServer uploads the document and must be refused");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_data_is_found_at_any_depth_serde_accepts() {
        let depth = 100;
        let text = format!("{}{{\"pluginData\":{{\"{}x\":1}}}}{}", "{\"a\":".repeat(depth), crate::widgets::DATA_PREFIX, "}".repeat(depth));
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(writes_widget_data(&v, 0));
    }
}
