//! Same-origin file-provider bridge. Persistence belongs to the registered provider, not WASM.
//!
//! Callbacks enqueue commands for a later egui frame. Exporting bytes never acknowledges them.

use figcraft_ui_egui::file_providers::{Host, Resource};
use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

thread_local! {
    static CURRENT_CONTEXT: std::cell::RefCell<Option<egui::Context>> = const { std::cell::RefCell::new(None) };
}
const API: &str = "figcraftFileProviders";
const EVENT: &str = "figcraft-file-provider";
const MAX_BYTES: u32 = 80 * 1024 * 1024;

fn error(message: impl ToString) -> JsValue {
    JsValue::from_str(&message.to_string())
}

fn object(value: &serde_json::Value) -> Result<JsValue, JsValue> {
    js_sys::JSON::parse(&value.to_string())
}

fn set(target: &JsValue, key: &str, value: &JsValue) -> Result<(), JsValue> {
    if Reflect::set(target, &key.into(), value)? { Ok(()) } else { Err(error("file-provider API is not writable")) }
}

// Inspect UTF-16 length before copying a host string into WASM memory.
fn bounded_string(value: JsValue, max_units: u32) -> Result<String, JsValue> {
    let text = value.dyn_into::<js_sys::JsString>().map_err(|_| error("resource fields must be strings"))?;
    if text.length() > max_units {
        return Err(error("resource field is too large"));
    }
    text.as_string().ok_or_else(|| error("invalid resource field"))
}

fn resource(value: JsValue) -> Result<Resource, JsValue> {
    if value.is_null() || !value.is_object() || Array::is_array(&value) {
        return Err(error("resource entries must be objects"));
    }
    let entry = value.dyn_into::<Object>()?;
    let id = bounded_string(Reflect::get(&entry, &"id".into())?, 160)?;
    let name = bounded_string(Reflect::get(&entry, &"name".into())?, 255)?;
    let updated = Reflect::get(&entry, &"updatedAt".into())?;
    let updated_at = if updated.is_undefined() { String::new() } else { bounded_string(updated, 100)? };
    Ok(Resource { id, name, updated_at })
}

fn wake_later(ctx: &egui::Context) {
    let fallback = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        CURRENT_CONTEXT.with(|current| current.borrow().as_ref().unwrap_or(&fallback).request_repaint());
    });
}

/// Makes the capability discoverable without enabling it or changing the browser's writer.
pub fn install(host: Host, ctx: egui::Context) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| error("no window"))?;
    CURRENT_CONTEXT.with(|current| *current.borrow_mut() = Some(ctx.clone()));
    if Reflect::has(&window, &API.into())? {
        // A graphics restart keeps the page capability and pending requests, and renews its wake target.
        return Ok(());
    }
    // A delayed result from an earlier page must never address a new document's request.
    let crypto = Reflect::get(&window, &"crypto".into())?;
    let random = Reflect::get(&crypto, &"getRandomValues".into())?.dyn_into::<Function>()?;
    let nonce = Uint8Array::new_with_length(16);
    random.call1(&crypto, &nonce)?;
    host.set_session(nonce.to_vec().iter().map(|byte| format!("{byte:02x}")).collect()).map_err(error)?;

    let api = Object::new();
    set(&api, "protocolVersion", &1.into())?;
    set(&api, "app", &"figcraft".into())?;
    set(&api, "capabilities", &object(&serde_json::json!(["revisionAck", "externalRecents", "saveDestination", "openBytes"]))?)?;

    let h = host.clone();
    let wake = ctx.clone();
    let register = Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> Result<String, JsValue>>::new(move |id, label, namespace| {
        let id = bounded_string(id, 160)?;
        let label = bounded_string(label, 100)?;
        let namespace = bounded_string(namespace, 160)?;
        let connection = h.connect(id, label, namespace).map_err(error)?;
        wake_later(&wake);
        Ok(connection)
    });
    set(&api, "registerProvider", register.as_ref())?;
    register.forget();

    let h = host.clone();
    let wake = ctx.clone();
    let unregister = Closure::<dyn FnMut() -> Result<bool, JsValue>>::new(move || {
        h.disconnect().map_err(error)?;
        wake_later(&wake);
        Ok(true)
    });
    set(&api, "unregisterProvider", unregister.as_ref())?;
    unregister.forget();

    let h = host.clone();
    let wake = ctx.clone();
    let enabled = Closure::<dyn FnMut(JsValue) -> Result<bool, JsValue>>::new(move |value: JsValue| {
        let value = value.as_bool().ok_or_else(|| error("enabled must be a boolean"))?;
        h.set_enabled(value).map_err(error)?;
        wake_later(&wake);
        Ok(true)
    });
    set(&api, "setEnabled", enabled.as_ref())?;
    enabled.forget();

    let h = host.clone();
    let wake = ctx.clone();
    let resources = Closure::<dyn FnMut(JsValue, JsValue) -> Result<(), JsValue>>::new(move |connection, value: JsValue| {
        let connection = bounded_string(connection, 160)?;
        if !Array::is_array(&value) {
            return Err(error("resources must be an array of at most 100 entries"));
        }
        let entries = value.dyn_into::<Array>()?;
        if entries.length() > 100 {
            return Err(error("resources must be an array of at most 100 entries"));
        }
        // Read only bounded metadata; do not copy or serialize arbitrary host properties.
        let resources = entries.iter().map(resource).collect::<Result<Vec<_>, _>>()?;
        h.set_resources(&connection, resources).map_err(error)?;
        wake_later(&wake);
        Ok(())
    });
    set(&api, "setResources", resources.as_ref())?;
    resources.forget();

    let h = host.clone();
    let wake = ctx.clone();
    let request_open = Closure::<dyn FnMut(JsValue) -> Result<String, JsValue>>::new(move |resource| {
        let resource = bounded_string(resource, 160)?;
        let request = h.request_open(&resource).map_err(error)?;
        wake_later(&wake);
        Ok(request)
    });
    set(&api, "requestOpen", request_open.as_ref())?;
    request_open.forget();

    let h = host.clone();
    let wake = ctx.clone();
    let open = Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> Result<(), JsValue>>::new(move |request, name, value: JsValue| {
        let request = bounded_string(request, 160)?;
        let name = bounded_string(name, 255)?;
        let bytes = value.dyn_into::<Uint8Array>().map_err(|_| error("bytes must be a Uint8Array"))?;
        if bytes.length() > MAX_BYTES {
            return Err(error("file exceeds the provider limit"));
        }
        h.open(request, name, bytes.to_vec()).map_err(error)?;
        wake_later(&wake);
        Ok(())
    });
    set(&api, "openResource", open.as_ref())?;
    open.forget();

    let h = host.clone();
    let wake = ctx.clone();
    let cancel_open = Closure::<dyn FnMut(JsValue, JsValue) -> Result<(), JsValue>>::new(move |request, message| {
        let request = bounded_string(request, 160)?;
        let message = bounded_string(message, 1024)?;
        h.cancel_open(request, message).map_err(error)?;
        wake_later(&wake);
        Ok(())
    });
    set(&api, "cancelOpen", cancel_open.as_ref())?;
    cancel_open.forget();

    let h = host.clone();
    let wake = ctx.clone();
    let result = Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> Result<(), JsValue>>::new(move |request, status, resource| {
        let request = bounded_string(request, 160)?;
        let status = bounded_string(status, 8)?;
        let resource = bounded_string(resource, 160)?;
        h.result(request, status, resource).map_err(error)?;
        wake_later(&wake);
        Ok(())
    });
    set(&api, "saveResult", result.as_ref())?;
    result.forget();

    let h = host.clone();
    let wake = ctx.clone();
    let dialog = Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> Result<(), JsValue>>::new(move |request, destination, name| {
        let request = bounded_string(request, 160)?;
        let destination = bounded_string(destination, 8)?;
        let name = bounded_string(name, 255)?;
        h.resolve_dialog(&request, &destination, &name).map_err(error)?;
        wake_later(&wake);
        Ok(())
    });
    set(&api, "resolveSaveDialog", dialog.as_ref())?;
    dialog.forget();

    let h = host.clone();
    let snapshot = Closure::<dyn FnMut() -> Result<JsValue, JsValue>>::new(move || object(&h.snapshot()));
    set(&api, "snapshot", snapshot.as_ref())?;
    snapshot.forget();
    set(&window, API, &Object::freeze(&api))?;
    dispatch(&serde_json::json!({"type": "ready", "protocolVersion": 1}), None)
}

fn dispatch(metadata: &serde_json::Value, bytes: Option<Vec<u8>>) -> Result<(), JsValue> {
    let detail = object(metadata)?;
    if let Some(bytes) = bytes {
        set(&detail, "bytes", &Uint8Array::from(bytes.as_slice()))?;
    }
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&detail);
    let event = web_sys::CustomEvent::new_with_event_init_dict(EVENT, &init)?;
    web_sys::window().ok_or_else(|| error("no window"))?.dispatch_event(&event)?;
    Ok(())
}

pub fn publish(host: &Host) {
    // The first drain frees a full queue; the second delivers retained terminals in order.
    for _ in 0..2 {
        let events = host.take_events();
        if events.is_empty() {
            break;
        }
        for event in events {
            if let Err(error) = dispatch(&event.metadata, event.bytes) {
                // Native pending requests survive a failing provider listener. No implicit ACK.
                log::error!("file-provider event delivery failed: {error:?}");
            }
        }
    }
}
