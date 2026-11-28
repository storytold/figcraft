//! Write FigCraft plugins in Rust.
//!
//! A plugin is a `cdylib` built for `wasm32-unknown-unknown`. It gets the command the user picked
//! with its parameters and the selection, and acts on the document by calling FigCraft commands
//! (the ids `figcraft-cli commands` lists) with JSON parameters. Everything one run does is one
//! undo step; when the run returns an error (or panics), the document is put back.
//!
//! ```ignore
//! use figcraft_plugin_sdk::{Input, Result, Value, call, entry, json};
//!
//! fn run(input: Input) -> Result<Value> {
//!     for id in &input.selection {
//!         call("object.rename", &json!({"ids": [id], "name": "Tidy"}))?;
//!     }
//!     Ok(json!(format!("Renamed {} layers", input.selection.len())))
//! }
//!
//! entry!(run);
//! ```
//!
//! A plugin can also show a panel ([`show_ui`]): FigCraft draws its controls and calls the plugin
//! again for each button press or changed value, with [`Input::event`] set. The module stays
//! loaded while the panel is open, so statics keep their values between those calls. Settings
//! ([`setting`], [`set_setting`]) last from one run to the next.
//!
//! A widget plugin (API 3, `"widget": true`) draws a layer on the canvas: when it's inserted and
//! each time a part of it named as a `target` is clicked, the plugin is called with
//! [`Input::widget`] set (its state and what was clicked) and hands back the new state and the
//! layers with [`render_widget`]. Each call starts afresh: the state is all a widget remembers.
//! With API 4 a widget can also have editable text fields (a text layer with a `field` name; an
//! edit calls back with [`Widget::field`] and [`Widget::value`], and the value is already in the
//! state under the field's name) and a property menu (`menu` beside `root`; choosing an item calls
//! back with [`Widget::menu`] and, for a toggle or a colour, [`Widget::value`]). With API 5 a
//! widget's callback can [`show_ui`]: the panel's callbacks then come with both [`Input::event`] and
//! [`Input::widget`] (its state; no target, field or menu), and [`render_widget`] redraws it.
//!
//! The SDK is `no_std` (it needs `alloc`); outside WebAssembly every host call fails, so a plugin's
//! own logic can still be unit-tested on the host. See `docs/plugins.md` for the ABI underneath.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub use serde_json::{self, Value, json};

/// What a plugin function returns: its output (shown or returned to the caller), or an error that
/// fails the run and rolls the document back.
pub type Result<T> = core::result::Result<T, String>;

/// The run's input.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    /// The manifest command the user picked.
    pub command: String,
    /// Its parameters, with defaults filled in.
    pub params: Value,
    /// The selected layer ids.
    pub selection: Vec<u64>,
    /// Set when the plugin's panel calls it back.
    pub event: Option<Event>,
    /// Set when the plugin's widget is inserted or clicked.
    pub widget: Option<Widget>,
}

/// A widget being inserted (no `target`, `field` or `menu`), clicked, edited or given a menu item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Widget {
    /// The widget's layer id.
    pub id: u64,
    /// Its state as last handed back (an empty object when it's new).
    pub state: Value,
    /// The target clicked.
    pub target: Option<String>,
    /// API 4: the editable field edited.
    pub field: Option<String>,
    /// API 4: the property menu item chosen.
    pub menu: Option<String>,
    /// API 4: the field's new text, a toggle's new state (`true`/`false`) or the colour chosen
    /// (null for a click or an action).
    pub value: Value,
}

/// A button pressed or a value changed in the plugin's panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    /// The control's name.
    pub control: String,
    /// A field's new value (null for a button).
    pub value: Value,
    /// Every field's value, by name.
    pub values: Value,
}

impl Input {
    /// Parse the input JSON FigCraft hands to the plugin.
    pub fn parse(text: &str) -> Result<Input> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("input: {e}"))?;
        Ok(Input {
            command: v.get("command").and_then(Value::as_str).unwrap_or_default().to_string(),
            params: v.get("params").cloned().unwrap_or(Value::Null),
            selection: ids(v.get("selection")),
            event: v.get("event").map(|e| Event {
                control: e.get("control").and_then(Value::as_str).unwrap_or_default().to_string(),
                value: e.get("value").cloned().unwrap_or(Value::Null),
                values: v.get("values").cloned().unwrap_or(Value::Null),
            }),
            widget: v.get("widget").map(|w| {
                let event = |key: &str| w.get("event").and_then(|e| e.get(key));
                let name = |key: &str| event(key).and_then(Value::as_str).map(ToString::to_string);
                Widget {
                    id: w.get("id").and_then(Value::as_u64).unwrap_or_default(),
                    state: w.get("state").cloned().unwrap_or_else(|| json!({})),
                    target: name("target"),
                    field: name("field"),
                    menu: name("menu"),
                    value: event("value").cloned().unwrap_or(Value::Null),
                }
            }),
        })
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        self.params.get(name).and_then(Value::as_str)
    }

    pub fn f64(&self, name: &str) -> Option<f64> {
        self.params.get(name).and_then(Value::as_f64).filter(|v| v.is_finite())
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        self.params.get(name).and_then(Value::as_bool)
    }
}

/// Layer ids in a JSON array.
fn ids(v: Option<&Value>) -> Vec<u64> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default()
}

/// Run FigCraft command `command` with `params`: its result, or why it was refused or failed.
pub fn call(command: &str, params: &Value) -> Result<Value> {
    let params = serde_json::to_string(params).map_err(|e| e.to_string())?;
    let status = host::call(command.as_bytes(), params.as_bytes());
    let reply = String::from_utf8(host::reply()).map_err(|_| "the reply isn't UTF-8".to_string())?;
    if status != 0 {
        return Err(reply);
    }
    serde_json::from_str(&reply).map_err(|e| format!("`{command}` returned {e}"))
}

/// The selected layer ids now (the input has them as they were when the run started).
pub fn selection() -> Vec<u64> {
    host::selection();
    let reply = host::reply();
    ids(serde_json::from_slice::<Value>(&reply).ok().as_ref())
}

/// A line for the run's log (returned with the result, and in FigCraft's log).
pub fn log(line: &str) {
    host::log(line.as_bytes());
}

/// A short message shown to the user.
pub fn notify(message: &str) {
    host::notify(message.as_bytes());
}

/// Show the plugin's panel, or replace the one showing: `{"title", "controls": [...]}` (see
/// `docs/plugins.md`). An invalid panel is an error.
pub fn show_ui(panel: &Value) -> Result<()> {
    let spec = serde_json::to_string(panel).map_err(|e| e.to_string())?;
    if host::ui_show(spec.as_bytes()) == 0 { Ok(()) } else { Err(String::from_utf8_lossy(&host::reply()).into_owned()) }
}

/// Close the plugin's panel (the plugin isn't called again until it's run again).
pub fn close_ui() {
    host::ui_close();
}

/// Draw the widget being inserted or clicked: `{"state": {...}, "root": {"type": "frame", ...}}`
/// (see `docs/plugins.md`). Without `state` the widget keeps the one it has. An invalid
/// description is an error.
pub fn render_widget(widget: &Value) -> Result<()> {
    let spec = serde_json::to_string(widget).map_err(|e| e.to_string())?;
    if host::widget_render(spec.as_bytes()) == 0 { Ok(()) } else { Err(String::from_utf8_lossy(&host::reply()).into_owned()) }
}

/// The plugin's setting `key`, as stored by [`set_setting`].
pub fn setting(key: &str) -> Option<Value> {
    if host::setting_get(key.as_bytes()) != 0 {
        return None;
    }
    serde_json::from_slice(&host::reply()).ok()
}

/// Remember `value` under `key` for later runs (64 KB of settings per plugin).
pub fn set_setting(key: &str, value: &Value) -> Result<()> {
    let v = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if host::setting_set(key.as_bytes(), v.as_bytes()) == 0 { Ok(()) } else { Err(String::from_utf8_lossy(&host::reply()).into_owned()) }
}

/// Forget setting `key`.
pub fn delete_setting(key: &str) {
    host::setting_delete(key.as_bytes());
}

/// Offer a file to save: a suggested file `name`, its MIME type and the bytes. FigCraft asks the
/// user where it goes (a download in the browser) once the run succeeds; the plugin never learns
/// a path. Needs `"api": 4` and `permissions.saveFile` in the manifest. A refusal (no permission,
/// a bad name, too much data, a second file in one run) is an error.
pub fn save_file(name: &str, mime: &str, data: &[u8]) -> Result<()> {
    if host::save_file(name.as_bytes(), mime.as_bytes(), data) == 0 { Ok(()) } else { Err(String::from_utf8_lossy(&host::reply()).into_owned()) }
}

/// The body of `figcraft_run` (use [`entry!`]): read the input, run `f`, hand back its output.
pub fn run_plugin(input_len: i32, f: fn(Input) -> Result<Value>) -> i32 {
    let input = host::reply();
    let input = input.get(..usize::try_from(input_len).unwrap_or(0)).unwrap_or(&input);
    let result = core::str::from_utf8(input).map_err(|_| "the input isn't UTF-8".to_string()).and_then(Input::parse).and_then(f);
    match result {
        Ok(v) => {
            host::set_output(serde_json::to_string(&v).unwrap_or_default().as_bytes());
            0
        }
        Err(e) => {
            host::set_output(e.as_bytes());
            1
        }
    }
}

/// Export `f: fn(Input) -> Result<Value>` as the plugin's entry point.
#[macro_export]
macro_rules! entry {
    ($f:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn figcraft_run(input_len: i32) -> i32 {
            $crate::run_plugin(input_len, $f)
        }
    };
}

/// The host functions FigCraft provides (module `figcraft`).
#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod host {
    use alloc::vec;
    use alloc::vec::Vec;

    mod sys {
        #[link(wasm_import_module = "figcraft")]
        unsafe extern "C" {
            pub fn call(cmd: *const u8, cmd_len: usize, params: *const u8, params_len: usize) -> i32;
            pub fn selection() -> i32;
            pub fn reply_len() -> i32;
            pub fn reply_read(ptr: *mut u8, len: usize) -> i32;
            pub fn log(ptr: *const u8, len: usize);
            pub fn notify(ptr: *const u8, len: usize);
            pub fn set_output(ptr: *const u8, len: usize);
            pub fn ui_show(ptr: *const u8, len: usize) -> i32;
            pub fn ui_close();
            pub fn setting_get(key: *const u8, key_len: usize) -> i32;
            pub fn setting_set(key: *const u8, key_len: usize, value: *const u8, value_len: usize) -> i32;
            pub fn setting_delete(key: *const u8, key_len: usize);
            pub fn widget_render(ptr: *const u8, len: usize) -> i32;
            pub fn save_file(name: *const u8, name_len: usize, mime: *const u8, mime_len: usize, data: *const u8, data_len: usize) -> i32;
        }
    }

    // SAFETY (every call below): the host reads only the `len` bytes at `ptr`, which are a live
    // slice, and writes only up to `len` bytes into the buffer it's given; it checks bounds itself.
    pub fn call(cmd: &[u8], params: &[u8]) -> i32 {
        unsafe { sys::call(cmd.as_ptr(), cmd.len(), params.as_ptr(), params.len()) }
    }
    pub fn selection() -> i32 {
        unsafe { sys::selection() }
    }
    pub fn reply() -> Vec<u8> {
        let len = usize::try_from(unsafe { sys::reply_len() }).unwrap_or(0);
        let mut buf = vec![0u8; len];
        let got = unsafe { sys::reply_read(buf.as_mut_ptr(), buf.len()) };
        buf.truncate(usize::try_from(got).unwrap_or(0));
        buf
    }
    pub fn log(s: &[u8]) {
        unsafe { sys::log(s.as_ptr(), s.len()) }
    }
    pub fn notify(s: &[u8]) {
        unsafe { sys::notify(s.as_ptr(), s.len()) }
    }
    pub fn set_output(s: &[u8]) {
        unsafe { sys::set_output(s.as_ptr(), s.len()) }
    }
    pub fn ui_show(s: &[u8]) -> i32 {
        unsafe { sys::ui_show(s.as_ptr(), s.len()) }
    }
    pub fn ui_close() {
        unsafe { sys::ui_close() }
    }
    pub fn setting_get(key: &[u8]) -> i32 {
        unsafe { sys::setting_get(key.as_ptr(), key.len()) }
    }
    pub fn setting_set(key: &[u8], value: &[u8]) -> i32 {
        unsafe { sys::setting_set(key.as_ptr(), key.len(), value.as_ptr(), value.len()) }
    }
    pub fn setting_delete(key: &[u8]) {
        unsafe { sys::setting_delete(key.as_ptr(), key.len()) }
    }
    pub fn widget_render(s: &[u8]) -> i32 {
        unsafe { sys::widget_render(s.as_ptr(), s.len()) }
    }
    pub fn save_file(name: &[u8], mime: &[u8], data: &[u8]) -> i32 {
        unsafe { sys::save_file(name.as_ptr(), name.len(), mime.as_ptr(), mime.len(), data.as_ptr(), data.len()) }
    }
}

/// Outside WebAssembly (a plugin's own unit tests) every host call fails.
#[cfg(not(target_arch = "wasm32"))]
mod host {
    use alloc::vec::Vec;

    pub fn call(_cmd: &[u8], _params: &[u8]) -> i32 {
        1
    }
    pub fn selection() -> i32 {
        0
    }
    pub fn reply() -> Vec<u8> {
        b"not running inside FigCraft".to_vec()
    }
    pub fn log(_s: &[u8]) {}
    pub fn notify(_s: &[u8]) {}
    pub fn set_output(_s: &[u8]) {}
    pub fn ui_show(_s: &[u8]) -> i32 {
        1
    }
    pub fn ui_close() {}
    pub fn setting_get(_key: &[u8]) -> i32 {
        1
    }
    pub fn setting_set(_key: &[u8], _value: &[u8]) -> i32 {
        1
    }
    pub fn setting_delete(_key: &[u8]) {}
    pub fn widget_render(_s: &[u8]) -> i32 {
        1
    }
    pub fn save_file(_name: &[u8], _mime: &[u8], _data: &[u8]) -> i32 {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_parses_and_host_calls_fail_off_wasm() {
        let i = Input::parse(r#"{"api":1,"command":"grid","params":{"n":3,"s":"x","b":true},"selection":[4,"x",7]}"#).unwrap_or_default();
        assert_eq!(i.command, "grid");
        assert_eq!(i.selection, alloc::vec![4, 7]);
        assert_eq!((i.f64("n"), i.str("s"), i.bool("b")), (Some(3.0), Some("x"), Some(true)));
        assert!(Input::parse("nope").is_err());
        assert!(call("node.get", &json!({"id": 1})).is_err());
        assert!(selection().is_empty());
        assert!(i.event.is_none());
        let e = Input::parse(r#"{"command":"panel","event":{"control":"go","value":null},"values":{"n":2}}"#)
            .unwrap_or_default()
            .event
            .unwrap_or_default();
        assert_eq!((e.control.as_str(), &e.value, &e.values), ("go", &Value::Null, &json!({"n": 2})));
        assert!(show_ui(&json!({})).is_err() && setting("k").is_none() && set_setting("k", &json!(1)).is_err());
        assert!(render_widget(&json!({})).is_err() && i.widget.is_none());
        assert!(save_file("a.txt", "text/plain", b"x").is_err());
        let w = Input::parse(r#"{"command":"widget","widget":{"id":9,"state":{"n":1},"event":{"target":"inc"}}}"#)
            .unwrap_or_default()
            .widget
            .unwrap_or_default();
        assert_eq!((w.id, &w.state, w.target.as_deref()), (9, &json!({"n": 1}), Some("inc")));
        let e =
            Input::parse(r#"{"widget":{"id":9,"state":{},"event":{"field":"label","value":"Hi"}}}"#).unwrap_or_default().widget.unwrap_or_default();
        assert_eq!((e.field.as_deref(), &e.value, e.target.is_none()), (Some("label"), &json!("Hi"), true));
        let m = Input::parse(r##"{"widget":{"id":9,"state":{},"event":{"menu":"color","value":"#FF0000"}}}"##)
            .unwrap_or_default()
            .widget
            .unwrap_or_default();
        assert_eq!((m.menu.as_deref(), &m.value), (Some("color"), &json!("#FF0000")));
    }
}
