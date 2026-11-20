//! Same-origin file-provider bridge. Persistence belongs to the registered provider, not WASM.
//!
//! Callbacks enqueue commands for a later egui frame. Exporting bytes never acknowledges them.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use figcraft_ui_egui::file_providers::{Host, Resource};
use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};

const API: &str = "figcraftFileProviders";
const EVENT: &str = "figcraft-file-provider";
const MAX_BYTES: u32 = 80 * 1024 * 1024;

thread_local! {
    static INSTALLED: RefCell<Option<Installed>> = const { RefCell::new(None) };
}

struct Installed {
    api: Object,
    editor: Rc<Editor>,
}

struct Editor {
    host: Host,
    ctx: egui::Context,
}

#[derive(Clone)]
pub(crate) struct Provider(Weak<Editor>);

impl Provider {
    fn active(&self) -> Result<Rc<Editor>, JsValue> {
        self.0
            .upgrade()
            .filter(|editor| INSTALLED.with_borrow(|current| current.as_ref().is_some_and(|current| Rc::ptr_eq(editor, &current.editor))))
            .ok_or_else(|| error("this file-provider API belongs to a previous editor"))
    }

    fn wake_later(&self) {
        let provider = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(editor) = provider.active() {
                editor.ctx.request_repaint();
            }
        });
    }

    /// Announce the installed API only once the owning editor's startup has succeeded.
    pub(crate) fn announce_ready(&self) {
        if self.active().is_ok()
            && let Err(error) = dispatch(&serde_json::json!({"type": "ready", "protocolVersion": 1}), None)
        {
            log::error!("file provider ready event could not be delivered: {error:?}");
        }
    }

    pub(crate) fn publish(&self) {
        let Ok(editor) = self.active() else { return };
        let more = editor.host.publish_events_with(
            |metadata, bytes| {
                self.active()?;
                dispatch(metadata, bytes)
            },
            |error| log::error!("file-provider event delivery failed: {error:?}"),
        );
        if more && self.active().is_ok() {
            editor.ctx.request_repaint();
        }
    }

    pub(crate) fn uninstall(&self) {
        if self.active().is_ok()
            && let Some(window) = web_sys::window()
            && let Err(error) = uninstall(&window)
        {
            log::error!("file provider capability could not be removed: {error:?}");
        }
    }
}

#[wasm_bindgen]
extern "C" {
    // Slice conversion's infallible Rust signature does not catch JavaScript allocation errors.
    #[wasm_bindgen(catch, js_namespace = Uint8Array, js_name = from)]
    fn copy_event_bytes(bytes: &[u8]) -> Result<Uint8Array, JsValue>;
}

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

fn uninstall(window: &web_sys::Window) -> Result<(), JsValue> {
    // Release the editor and any pending captures even when page code has replaced the property.
    let current = INSTALLED.with_borrow_mut(Option::take);
    if let Some(current) = current
        && Object::is(&Reflect::get(window, &API.into())?, current.api.as_ref())
        && !Reflect::delete_property(window.unchecked_ref::<Object>(), &API.into())?
    {
        return Err(error("file-provider API could not be removed"));
    }
    Ok(())
}

/// Install an initially disabled API for this editor. Each replacement invalidates the
/// previous nonce, provider connection and in-flight requests.
pub(crate) fn install(host: Host, ctx: egui::Context, embedded: bool) -> Result<Option<Provider>, JsValue> {
    let window = web_sys::window().ok_or_else(|| error("no window"))?;
    uninstall(&window)?;
    // The embedded host owns its file delivery, including across a same-page restart.
    if embedded {
        return Ok(None);
    }
    if Reflect::has(&window, &API.into())? {
        return Err(error("the page already owns the file-provider API name"));
    }
    let crypto = Reflect::get(&window, &"crypto".into())?;
    let random = Reflect::get(&crypto, &"getRandomValues".into())?.dyn_into::<Function>()?;
    let nonce = Uint8Array::new_with_length(16);
    random.call1(&crypto, &nonce)?;
    host.set_session(nonce.to_vec().iter().map(|byte| format!("{byte:02x}")).collect()).map_err(error)?;

    let editor = Rc::new(Editor { host, ctx });
    let provider = Provider(Rc::downgrade(&editor));
    let api = Object::new();
    set(&api, "protocolVersion", &1.into())?;
    set(&api, "app", &"figcraft".into())?;
    set(&api, "capabilities", &object(&serde_json::json!(["revisionAck", "externalRecents", "saveDestination", "openBytes"]))?)?;

    let active = provider.clone();
    let register = Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> Result<String, JsValue>>::new(move |id, label, namespace| {
        let id = bounded_string(id, 160)?;
        let label = bounded_string(label, 100)?;
        let namespace = bounded_string(namespace, 160)?;
        let connection = active.active()?.host.connect(id, label, namespace).map_err(error)?;
        active.wake_later();
        Ok(connection)
    });
    set(&api, "registerProvider", &register.into_js_value())?;

    let active = provider.clone();
    let unregister = Closure::<dyn FnMut() -> Result<bool, JsValue>>::new(move || {
        active.active()?.host.disconnect().map_err(error)?;
        active.wake_later();
        Ok(true)
    });
    set(&api, "unregisterProvider", &unregister.into_js_value())?;

    let active = provider.clone();
    let enabled = Closure::<dyn FnMut(JsValue) -> Result<bool, JsValue>>::new(move |value: JsValue| {
        let value = value.as_bool().ok_or_else(|| error("enabled must be a boolean"))?;
        active.active()?.host.set_enabled(value).map_err(error)?;
        active.wake_later();
        Ok(true)
    });
    set(&api, "setEnabled", &enabled.into_js_value())?;

    let active = provider.clone();
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
        active.active()?.host.set_resources(&connection, resources).map_err(error)?;
        active.wake_later();
        Ok(())
    });
    set(&api, "setResources", &resources.into_js_value())?;

    let active = provider.clone();
    let request_open = Closure::<dyn FnMut(JsValue) -> Result<String, JsValue>>::new(move |resource| {
        let resource = bounded_string(resource, 160)?;
        let request = active.active()?.host.request_open(&resource).map_err(error)?;
        active.wake_later();
        Ok(request)
    });
    set(&api, "requestOpen", &request_open.into_js_value())?;

    let active = provider.clone();
    let open = Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> Result<(), JsValue>>::new(move |request, name, value: JsValue| {
        let request = bounded_string(request, 160)?;
        let name = bounded_string(name, 255)?;
        let bytes = value.dyn_into::<Uint8Array>().map_err(|_| error("bytes must be a Uint8Array"))?;
        if bytes.length() > MAX_BYTES {
            return Err(error("file exceeds the provider limit"));
        }
        active.active()?.host.open(request, name, bytes.to_vec()).map_err(error)?;
        active.wake_later();
        Ok(())
    });
    set(&api, "openResource", &open.into_js_value())?;

    let active = provider.clone();
    let cancel_open = Closure::<dyn FnMut(JsValue, JsValue) -> Result<(), JsValue>>::new(move |request, message| {
        let request = bounded_string(request, 160)?;
        let message = bounded_string(message, 1024)?;
        active.active()?.host.cancel_open(request, message).map_err(error)?;
        active.wake_later();
        Ok(())
    });
    set(&api, "cancelOpen", &cancel_open.into_js_value())?;

    let active = provider.clone();
    let result = Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> Result<(), JsValue>>::new(move |request, status, resource| {
        let request = bounded_string(request, 160)?;
        let status = bounded_string(status, 8)?;
        let resource = bounded_string(resource, 160)?;
        active.active()?.host.result(request, status, resource).map_err(error)?;
        active.wake_later();
        Ok(())
    });
    set(&api, "saveResult", &result.into_js_value())?;

    let active = provider.clone();
    let dialog = Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> Result<(), JsValue>>::new(move |request, destination, name| {
        let request = bounded_string(request, 160)?;
        let destination = bounded_string(destination, 8)?;
        let name = bounded_string(name, 255)?;
        active.active()?.host.resolve_dialog(&request, &destination, &name).map_err(error)?;
        active.wake_later();
        Ok(())
    });
    set(&api, "resolveSaveDialog", &dialog.into_js_value())?;

    let active = provider.clone();
    let snapshot = Closure::<dyn FnMut() -> Result<JsValue, JsValue>>::new(move || object(&active.active()?.host.snapshot()));
    set(&api, "snapshot", &snapshot.into_js_value())?;
    let api = Object::freeze(&api);
    set(&window, API, api.as_ref())?;
    INSTALLED.with_borrow_mut(|current| *current = Some(Installed { api, editor }));
    Ok(Some(provider))
}

fn dispatch(metadata: &serde_json::Value, bytes: Option<Vec<u8>>) -> Result<(), JsValue> {
    let detail = object(metadata)?;
    if let Some(bytes) = bytes {
        set(&detail, "bytes", &copy_event_bytes(&bytes)?.into())?;
    }
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&detail);
    let event = web_sys::CustomEvent::new_with_event_init_dict(EVENT, &init)?;
    web_sys::window().ok_or_else(|| error("no window"))?.dispatch_event(&event)?;
    Ok(())
}

#[cfg(test)]
#[path = "file_provider_browser_tests.rs"]
mod browser_tests;
