//! Clawku: File › Open asks the host page for a file from its own storage instead of opening the
//! browser's file picker. Opt-in: only a page that loads the app with `?host=parent&open=host`
//! (Clawku) gets `<app>:pick` (`{ type, purpose }`) on the same-origin parent; every other page
//! keeps the browser picker. The host answers with the usual `<app>:open` message, or not at all.

use wasm_bindgen::JsValue;

/// Ask the host for a file to open; `false` when this page isn't hosted that way (use the picker).
pub fn host_pick(app: &str, purpose: &str) -> bool {
    let Some(win) = web_sys::window() else { return false };
    let search = win.location().search().unwrap_or_default();
    let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) else { return false };
    if params.get("host").as_deref() != Some("parent") || params.get("open").as_deref() != Some("host") {
        return false;
    }
    let Ok(Some(parent)) = win.parent() else { return false };
    if js_sys::Object::is(&parent, &win) {
        return false; // not framed
    }
    let Ok(origin) = win.location().origin() else { return false };
    let msg = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("type"), &JsValue::from_str(&format!("{app}:pick")));
    let _ = js_sys::Reflect::set(&msg, &JsValue::from_str("purpose"), &JsValue::from_str(purpose));
    parent.post_message(&msg, &origin).is_ok()
}
