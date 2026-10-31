//! `window.figcraft`: the control channel for the page, as promises.
//!
//! The same JSON methods as the desktop control server (`docs/control-protocol.md`), so an agent
//! driving FigCraft in a browser tab (or an `<iframe>` in another app) uses the same commands as
//! the CLI and the MCP server:
//!
//! ```js
//! await figcraft.execute("node.create", {kind: "frame", name: "Home"})  // engine command → its result
//! await figcraft.request("document.inspect")                            // control method → its result
//! await figcraft.commands()                                             // command list
//! const {data} = await figcraft.screenshot()                            // the canvas as PNG (base64)
//! await figcraft.openFile("home.fig", bytes)                            // `file.open` with the bytes
//! figcraft.captureDownloads(true)                                       // saves stay in the page…
//! figcraft.files(); await figcraft.readFile("home.figcraft")            // …to be read back
//! ```
//!
//! Results resolve with the method's `result`; errors reject with an `Error`.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::{Receiver, TryRecvError};

use base64::Engine as _;
use figcraft_ui_egui::{ControlReply, ControlRequest};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::saved_files::SavedFiles;

type Waiting = (Receiver<ControlReply>, js_sys::Function, js_sys::Function);

thread_local! {
    static QUEUE: RefCell<Vec<ControlRequest>> = const { RefCell::new(Vec::new()) };
    static WAITING: RefCell<Vec<Waiting>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    static CAPTURE: Cell<bool> = const { Cell::new(false) };
    static SAVED: RefCell<SavedFiles> = RefCell::new(SavedFiles::default());
}

pub fn connect(ctx: egui::Context) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

/// The page's requests since the last frame, for the app's control queue.
pub fn take_requests() -> Vec<ControlRequest> {
    QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Keep a save instead of downloading it, when the page asked for that. True when kept.
pub fn capture(name: &str, bytes: &[u8]) -> bool {
    if !CAPTURE.with(Cell::get) {
        return false;
    }
    SAVED.with(|s| s.borrow_mut().keep(name, bytes));
    true
}

fn to_js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

fn from_js(v: &JsValue) -> Value {
    if v.is_undefined() || v.is_null() {
        return json!({});
    }
    js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

/// Settle the promises of answered requests (called every frame).
pub fn poll_replies() {
    WAITING.with(|w| {
        w.borrow_mut().retain(|(rx, resolve, reject)| match rx.try_recv() {
            Ok(ControlReply { result: Ok(v), .. }) => {
                let _ = resolve.call1(&JsValue::NULL, &to_js(&v));
                false
            }
            Ok(ControlReply { result: Err(e), .. }) => {
                let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new(&e));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => {
                let _ = reject.call1(&JsValue::NULL, &js_sys::Error::new("request dropped"));
                false
            }
        });
    });
}

fn request(method: &str, params: Value) -> js_sys::Promise {
    let method = method.to_string();
    let p = js_sys::Promise::new(&mut |resolve, reject| {
        let (reply, rx) = std::sync::mpsc::channel();
        let id = NEXT_ID.with(|n| {
            n.set(n.get() + 1);
            n.get()
        });
        let req = ControlRequest { id: json!(id), method: method.clone(), params: params.clone(), reply, state: Default::default() };
        QUEUE.with(|q| q.borrow_mut().push(req));
        WAITING.with(|w| w.borrow_mut().push((rx, resolve, reject)));
    });
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
    p
}

fn set_fn(obj: &js_sys::Object, name: &str, f: JsValue) -> Result<(), JsValue> {
    js_sys::Reflect::set(obj, &name.into(), &f).map(|_| ())
}

/// Install `window.figcraft`.
pub fn install() -> Result<(), JsValue> {
    let obj = js_sys::Object::new();
    set_fn(
        &obj,
        "request",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|method: String, params: JsValue| request(&method, from_js(&params)))
            .into_js_value(),
    )?;
    set_fn(
        &obj,
        "execute",
        Closure::<dyn Fn(String, JsValue) -> js_sys::Promise>::new(|command: String, params: JsValue| {
            request("engine.execute", json!({"command": command, "params": from_js(&params)}))
        })
        .into_js_value(),
    )?;
    set_fn(&obj, "commands", Closure::<dyn Fn() -> js_sys::Promise>::new(|| request("engine.commands", json!({}))).into_js_value())?;
    set_fn(&obj, "inspect", Closure::<dyn Fn() -> js_sys::Promise>::new(|| request("document.inspect", json!({}))).into_js_value())?;
    set_fn(
        &obj,
        "screenshot",
        Closure::<dyn Fn(JsValue) -> js_sys::Promise>::new(|params: JsValue| request("ui.screenshot", from_js(&params))).into_js_value(),
    )?;
    set_fn(
        &obj,
        "openFile",
        Closure::<dyn Fn(String, js_sys::Uint8Array) -> js_sys::Promise>::new(|name: String, bytes: js_sys::Uint8Array| {
            let data = base64::engine::general_purpose::STANDARD.encode(bytes.to_vec());
            request("engine.execute", json!({"command": "file.open", "params": {"name": name, "data": data}}))
        })
        .into_js_value(),
    )?;
    set_fn(&obj, "captureDownloads", Closure::<dyn Fn(bool)>::new(|on: bool| CAPTURE.with(|c| c.set(on))).into_js_value())?;
    set_fn(
        &obj,
        "files",
        Closure::<dyn Fn() -> JsValue>::new(|| {
            SAVED.with(|s| to_js(&Value::Array(s.borrow().list().map(|(n, size)| json!({"path": n, "size": size})).collect())))
        })
        .into_js_value(),
    )?;
    set_fn(
        &obj,
        "readFile",
        Closure::<dyn Fn(String) -> Result<JsValue, JsValue>>::new(|name: String| {
            SAVED.with(|s| {
                s.borrow()
                    .get(&name)
                    .map(|b| js_sys::Uint8Array::from(b).into())
                    .ok_or_else(|| js_sys::Error::new(&format!("{name}: not found")).into())
            })
        })
        .into_js_value(),
    )?;
    let w = web_sys::window().ok_or("no window")?;
    js_sys::Reflect::set(&w, &"figcraft".into(), &obj)?;
    Ok(())
}
