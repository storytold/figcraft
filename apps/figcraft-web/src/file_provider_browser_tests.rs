//! Original synthetic FigCraft native archive and generated PNG, MIT OR Apache-2.0.
//! One owned browser page uses the exported Window API and actual application consumer.
//! Rust callbacks throw genuine JavaScript exceptions; no handwritten JavaScript or graphics.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use base64::Engine as _;
use figcraft_engine::Session;
use figcraft_ui_egui::{FigApp, Services, file_providers::Host};
use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn error(message: impl ToString) -> JsValue {
    JsValue::from_str(&message.to_string())
}
fn field(object: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    Reflect::get(object, &name.into())
}
fn text(object: &JsValue, name: &str) -> Result<String, JsValue> {
    field(object, name)?.as_string().ok_or_else(|| error(format!("missing string field {name}")))
}
fn call(object: &JsValue, name: &str, arguments: &[JsValue]) -> Result<JsValue, JsValue> {
    let function = field(object, name)?.dyn_into::<Function>()?;
    let array = Array::new();
    for argument in arguments {
        array.push(argument);
    }
    function.apply(object, &array)
}

struct Page {
    window: web_sys::Window,
    api: JsValue,
    host: Host,
    ctx: egui::Context,
    events: Rc<RefCell<Vec<JsValue>>>,
    listener: Closure<dyn FnMut(web_sys::Event)>,
}
impl Page {
    fn install(app: &FigApp) -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| error("real browser window required"))?;
        if Reflect::has(&window, &super::API.into())? {
            return Err(error("fixture requires a fresh owned page, never an existing editor"));
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(event) = event.dyn_ref::<web_sys::CustomEvent>() {
                observed.borrow_mut().push(event.detail());
            }
        });
        window.add_event_listener_with_callback(super::EVENT, listener.as_ref().unchecked_ref())?;
        // Cleanup is armed before install can publish an API/context and then return an error.
        let mut page = Self { window, api: JsValue::UNDEFINED, host: app.file_providers.clone(), ctx: egui::Context::default(), events, listener };
        super::install(page.host.clone(), page.ctx.clone())?;
        page.api = field(&page.window, super::API)?;
        Ok(page)
    }
    fn api(&self, method: &str, arguments: &[JsValue]) -> Result<JsValue, JsValue> {
        call(&self.api, method, arguments)
    }
    fn matching(&self, kind: &str, request: &str) -> Vec<JsValue> {
        self.events
            .borrow()
            .iter()
            .filter(|event| text(event, "type").is_ok_and(|value| value == kind) && text(event, "requestId").is_ok_and(|value| value == request))
            .cloned()
            .collect()
    }
    fn one(&self, kind: &str, request: &str) -> Result<JsValue, JsValue> {
        let events = self.matching(kind, request);
        if events.len() != 1 {
            return Err(error(format!("expected one real window {kind} for {request}, got {}", events.len())));
        }
        events.into_iter().next().ok_or_else(|| error("missing real event"))
    }
    fn tick(&self, app: &mut FigApp) {
        app.background_tick(&self.ctx);
    }
    fn latest(&self, kind: &str) -> Result<JsValue, JsValue> {
        self.events
            .borrow()
            .iter()
            .rev()
            .find(|event| text(event, "type").is_ok_and(|value| value == kind))
            .cloned()
            .ok_or_else(|| error(format!("missing real window {kind}")))
    }
    fn capture(&self, app: &mut FigApp) -> Result<String, JsValue> {
        let since = self.events.borrow().len();
        app.save(); // Actual interactive Save, not Session::file.saveAs or a Host helper.
        super::publish(&self.host);
        let dialogs: Vec<_> =
            self.events.borrow().iter().skip(since).filter(|event| text(event, "type").is_ok_and(|kind| kind == "saveDialog")).cloned().collect();
        if dialogs.len() != 1 {
            return Err(error("actual Save did not publish one captured chooser"));
        }
        let request = text(&dialogs[0], "requestId")?;
        self.api("resolveSaveDialog", &[request.clone().into(), "provider".into(), "Fixture.figcraft".into()])?;
        Ok(request)
    }
    // Catch the production callback at Function::call0, then restore the exact original own
    // descriptor (or remove our shadow) before propagating its Result or making assertions.
    // Rust/JS fatal traps and process termination are outside this recoverable-exception proof.
    fn with_replacement(&self, target: &JsValue, key: &str, replacement: &JsValue) -> Result<Result<JsValue, JsValue>, JsValue> {
        let object = field(&js_sys::global(), "Object")?;
        let descriptor = call(&object, "getOwnPropertyDescriptor", &[target.clone(), key.into()])?;
        let original = field(target, key)?;
        let host = self.host.clone();
        let operation = Closure::<dyn FnMut()>::new(move || super::publish(&host));
        let outcome = match Reflect::set(target, &key.into(), replacement) {
            Ok(true) => operation.as_ref().unchecked_ref::<Function>().call0(&JsValue::UNDEFINED),
            Ok(false) => Err(error("injected method is not writable")),
            Err(error) => Err(error),
        };
        if descriptor.is_undefined() {
            if !Reflect::delete_property(target.unchecked_ref::<Object>(), &key.into())? {
                return Err(error("could not remove injected method"));
            }
        } else {
            call(&object, "defineProperty", &[target.clone(), key.into(), descriptor.clone()])?;
        }
        let restored = call(&object, "getOwnPropertyDescriptor", &[target.clone(), key.into()])?;
        if field(target, key)? != original || restored.is_undefined() != descriptor.is_undefined() {
            return Err(error("injected property was not restored exactly"));
        }
        if !descriptor.is_undefined() {
            for attribute in ["value", "get", "set", "writable", "enumerable", "configurable"] {
                if field(&restored, attribute)? != field(&descriptor, attribute)? {
                    return Err(error("injected property descriptor changed"));
                }
            }
        }
        Ok(outcome)
    }
    fn fail_copy(&self) -> Result<u32, JsValue> {
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let fault = Closure::<dyn FnMut(JsValue) -> Result<JsValue, JsValue>>::new(move |_| {
            observed.set(observed.get() + 1);
            Err(js_sys::RangeError::new("synthetic FigCraft byte-copy failure").into())
        });
        let array = field(&js_sys::global(), "Uint8Array")?;
        self.with_replacement(&array, "from", fault.as_ref())??;
        Ok(calls.get())
    }
    fn fail_dispatch(&self, kind: &str, accept_save_first: bool) -> Result<u32, JsValue> {
        let original = field(&self.window, "dispatchEvent")?.dyn_into::<Function>()?;
        let window: JsValue = self.window.clone().into();
        let api = self.api.clone();
        let wanted = kind.to_string();
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let fault = Closure::<dyn FnMut(JsValue) -> Result<JsValue, JsValue>>::new(move |event: JsValue| {
            let detail = field(&event, "detail")?;
            if text(&detail, "type")? != wanted {
                return original.call1(&window, &event);
            }
            observed.set(observed.get() + 1);
            if accept_save_first {
                original.call1(&window, &event)?;
                call(&api, "saveResult", &[text(&detail, "requestId")?.into(), "saved".into(), "remote".into()])?;
            }
            Err(js_sys::RangeError::new("synthetic FigCraft event dispatch failure").into())
        });
        self.with_replacement(&self.window, "dispatchEvent", fault.as_ref())??;
        Ok(calls.get())
    }
}
impl Drop for Page {
    fn drop(&mut self) {
        let _ = self.window.remove_event_listener_with_callback(super::EVENT, self.listener.as_ref().unchecked_ref());
        let _ = self.host.disconnect();
        let _ = Reflect::delete_property(self.window.unchecked_ref::<Object>(), &super::API.into());
        super::CURRENT_CONTEXT.with(|current| *current.borrow_mut() = None);
    }
}

struct NoServices;
impl Services for NoServices {}
struct NativeResource {
    hash: String,
    bytes: Vec<u8>,
}
fn resource_app() -> Result<(FigApp, NativeResource), JsValue> {
    let mut session = Session::empty();
    session.execute("file.new", &json!({})).map_err(error)?;
    // Original 2x2 PNG generated in Rust, MIT OR Apache-2.0; no external image/font.
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([196, 40, 20, 255]))
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(error)?;
    let hash = figcraft_format::image_hash(&bytes);
    session
        .execute("file.placeImage", &json!({"name":"Synthetic.png","data":base64::engine::general_purpose::STANDARD.encode(&bytes)}))
        .map_err(error)?;
    session.execute("file.saveVersion", &json!({"name":"Captured version"})).map_err(error)?;
    let main_tab = session.active;
    session.execute("branch.create", &json!({"name":"Captured branch"})).map_err(error)?;
    session.active = main_tab;
    Ok((FigApp::new(session, Box::new(NoServices)), NativeResource { hash, bytes }))
}
fn verify_native(bytes: &[u8], resource: &NativeResource) -> Result<(), JsValue> {
    assert!(!bytes.is_empty());
    let project = figcraft_format::load_bytes_with_versions(bytes).map_err(error)?;
    assert!(project.issues.is_empty());
    assert_eq!(project.doc.images.get(&resource.hash).ok_or_else(|| error("native image missing"))?.bytes.as_slice(), resource.bytes.as_slice());
    let version = project.versions.iter().find(|version| version.name == "Captured version").ok_or_else(|| error("native version missing"))?;
    assert_eq!(
        version.document().map_err(error)?.images.get(&resource.hash).ok_or_else(|| error("version image missing"))?.bytes.as_slice(),
        resource.bytes.as_slice()
    );
    let branch = project.branches.iter().find(|branch| branch.name == "Captured branch").ok_or_else(|| error("native branch missing"))?;
    for document in [&branch.base, &branch.head] {
        assert_eq!(document.images.get(&resource.hash).ok_or_else(|| error("branch image missing"))?.bytes.as_slice(), resource.bytes.as_slice());
    }
    Ok(())
}

#[wasm_bindgen_test]
fn real_window_delivery_recovery_preserves_fig_archive_history_and_actual_app_ownership() -> Result<(), JsValue> {
    let (mut app, resource) = resource_app()?;
    let page = Page::install(&app)?;
    assert!(!page.host.enabled() && !page.host.connected());
    let connection = page.api("registerProvider", &["fixture".into(), "Fixture storage".into(), "account-one".into()])?;
    let resources = Array::new();
    for id in ["remote", "continuation"] {
        resources.push(&js_sys::JSON::parse(&json!({"id":id,"name":"Fixture.figcraft"}).to_string())?);
    }
    page.api("setResources", &[connection.clone(), resources.into()])?;
    let initial = page.capture(&mut app)?;
    super::publish(&page.host);
    let initial_bytes = field(&page.one("save", &initial)?, "bytes")?.dyn_into::<Uint8Array>()?.to_vec();
    verify_native(&initial_bytes, &resource)?;
    page.api("saveResult", &[initial.into(), "saved".into(), "remote".into()])?;
    page.tick(&mut app);
    super::publish(&page.host);
    assert!(!app.session.ds().map_err(error)?.is_dirty());

    app.session.execute("shape.rectangle", &json!({"x":4,"y":4,"width":2,"height":2})).map_err(error)?;
    let failed = page.capture(&mut app)?;
    let captured_revision = app.session.ds().map_err(error)?.revision;
    app.session.execute("shape.rectangle", &json!({"x":8,"y":8,"width":2,"height":2})).map_err(error)?;
    app.ui.comments.draft = Some((3.0, 4.0, "Comment typed after capture".into()));
    let draft = app.ui.comments.draft.clone();
    let before = app.session.ds().map_err(error)?.clone();
    let later = page.api("requestOpen", &["continuation".into()])?.as_string().ok_or_else(|| error("missing read ID"))?;
    page.api("cancelOpen", &[later.clone().into(), "later terminal survives".into()])?;
    assert_eq!(page.fail_copy()?, 1);
    assert!(page.matching("save", &failed).is_empty());
    assert_eq!(text(&page.one("openFailed", &later)?, "error")?, "later terminal survives");
    assert!(page.api("saveResult", &[failed.clone().into(), "saved".into(), "wrong-resource".into()]).is_err());
    assert!(page.host.pending_for(before.uid));
    page.tick(&mut app); // Production background_tick reaches the actual file_provider_app::tick consumer.
    assert_eq!(app.ui.comments.draft, draft);
    let current = app.session.ds().map_err(error)?;
    assert_eq!((current.uid, current.revision, &current.path), (before.uid, before.revision, &before.path));
    assert!(Arc::ptr_eq(&current.doc, &before.doc) && current.is_dirty());
    assert_eq!(page.host.bound_document("remote"), Some(before.uid));
    super::publish(&page.host);
    let terminal = page.one("saveFailed", &failed)?;
    assert_eq!(text(&terminal, "revision")?, captured_revision.to_string());
    assert_eq!(text(&terminal, "connectionId")?, connection.as_string().ok_or_else(|| error("no connection ID"))?);
    assert!(!page.host.pending_for(before.uid));
    app.ui.comments.draft = None; // Explicit fixture cancellation, not a claim that the comment was saved.
    app.session.execute("edit.undo", &json!({})).map_err(error)?;
    assert!(app.session.ds().map_err(error)?.is_dirty(), "failed capture must not advance its saved checkpoint");
    app.session.execute("edit.redo", &json!({})).map_err(error)?;
    let retry = page.capture(&mut app)?;
    super::publish(&page.host);
    let delivered = field(&page.one("save", &retry)?, "bytes")?.dyn_into::<Uint8Array>()?.to_vec();
    verify_native(&delivered, &resource)?;
    assert!(app.session.ds().map_err(error)?.is_dirty());
    page.api("saveResult", &[retry.into(), "saved".into(), "remote".into()])?;
    page.tick(&mut app);
    super::publish(&page.host);
    assert!(!app.session.ds().map_err(error)?.is_dirty());

    app.session.execute("shape.rectangle", &json!({"x":12,"y":12,"width":2,"height":2})).map_err(error)?;
    app.save();
    assert_eq!(page.fail_dispatch("saveDialog", false)?, 1);
    page.tick(&mut app);
    super::publish(&page.host);
    let choice = text(&page.latest("saveFailed")?, "requestId")?;
    assert_ne!(choice, failed);
    assert!(page.api("resolveSaveDialog", &[choice.clone().into(), "provider".into(), "Late.figcraft".into()]).is_err());
    page.one("saveFailed", &choice)?;
    let original = app.session.ds().map_err(error)?.clone();
    assert!(original.is_dirty());
    let failed_read = page.api("requestOpen", &["continuation".into()])?.as_string().ok_or_else(|| error("missing read ID"))?;
    assert_eq!(page.fail_dispatch("openRequested", false)?, 1);
    let incoming = Uint8Array::from(delivered.as_slice());
    assert!(page.api("openResource", &[failed_read.clone().into(), "Incoming.figcraft".into(), incoming.clone().into()]).is_err());
    page.tick(&mut app);
    super::publish(&page.host);
    page.one("openFailed", &failed_read)?;
    let unchanged = app.session.ds().map_err(error)?;
    assert_eq!((unchanged.uid, unchanged.revision), (original.uid, original.revision));
    assert!(Arc::ptr_eq(&unchanged.doc, &original.doc));
    let read_retry = page.api("requestOpen", &["continuation".into()])?.as_string().ok_or_else(|| error("missing retry ID"))?;
    super::publish(&page.host);
    page.one("openRequested", &read_retry)?;
    page.api("openResource", &[read_retry.clone().into(), "Incoming.figcraft".into(), incoming.into()])?;
    page.tick(&mut app);
    super::publish(&page.host);
    page.one("opened", &read_retry)?;
    let reopened = app.session.ds().map_err(error)?;
    assert_ne!(reopened.uid, original.uid);
    assert_eq!(reopened.doc.images.get(&resource.hash).ok_or_else(|| error("resource not reopened"))?.bytes.as_slice(), resource.bytes.as_slice());
    let reopened_uid = reopened.uid;

    app.session.execute("shape.rectangle", &json!({"x":16,"y":16,"width":2,"height":2})).map_err(error)?;
    let accepted = page.capture(&mut app)?;
    app.session.execute("shape.rectangle", &json!({"x":20,"y":20,"width":2,"height":2})).map_err(error)?;
    let later_state = app.session.ds().map_err(error)?.clone();
    assert_eq!(page.fail_dispatch("save", true)?, 1);
    assert!(page.matching("saveFailed", &accepted).is_empty());
    page.tick(&mut app);
    super::publish(&page.host);
    page.one("saved", &accepted)?;
    assert!(page.matching("saveFailed", &accepted).is_empty());
    let retained = app.session.ds().map_err(error)?;
    assert_eq!((retained.uid, retained.revision), (reopened_uid, later_state.revision));
    assert!(Arc::ptr_eq(&retained.doc, &later_state.doc) && retained.is_dirty());
    assert_eq!(page.host.bound_document("remote"), Some(reopened_uid));
    assert!(page.host.bound_filename(original.uid).is_none());
    assert!(app.session.docs.iter().find(|doc| doc.uid == original.uid).is_some_and(|doc| doc.is_dirty() && Arc::ptr_eq(&doc.doc, &original.doc)));
    assert!(page.api("saveResult", &[accepted.into(), "saved".into(), "remote".into()]).is_err());
    assert!(!page.host.busy());
    assert_eq!(page.api("unregisterProvider", &[])?.as_bool(), Some(true));
    assert!(!page.host.connected() && !page.host.enabled());
    Ok(())
}
