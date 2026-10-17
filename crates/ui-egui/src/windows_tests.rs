//! Several windows: a window holds its own tabs; closing it closes its files, asking first when
//! one has unsaved changes, and the main window takes over another window instead of quitting.

use std::sync::mpsc::{Receiver, channel};

use egui::{RawInput, ViewportEvent, ViewportId, ViewportInfo};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::control::{ControlReply, ControlRequest};
use crate::{FigApp, NoServices};

/// One main-window frame, as the desktop app runs it; `close` asks the window to close.
fn frame(ctx: &egui::Context, app: &mut FigApp, close: bool) {
    let mut input = RawInput::default();
    if close {
        input.viewports.insert(ViewportId::ROOT, ViewportInfo { events: vec![ViewportEvent::Close], ..Default::default() });
    }
    ctx.run_ui(input, |ui| {
        crate::windows::on_close_request(app, ui.ctx());
        app.ui(ui);
    })
    .textures_delta
    .clear();
}

/// A control request queued for the UI, and where its reply arrives.
fn request(app: &mut FigApp, method: &str, params: Value) -> Receiver<ControlReply> {
    let (reply, rx) = channel();
    app.control_queue.push(ControlRequest { id: json!(1), method: method.into(), params, reply, cancelled: Default::default() });
    rx
}

/// The desktop app's logic as eframe runs it for the main window, with each window's visibility
/// (`hidden`: minimised or covered). While the main window is hidden too this is all that runs.
fn logic(ctx: &egui::Context, app: &mut FigApp, hidden: &[ViewportId]) {
    let mut input = RawInput::default();
    for id in std::iter::once(ViewportId::ROOT).chain(app.windows.list.iter().map(|w| crate::windows::viewport(w.id))) {
        input.viewports.insert(id, ViewportInfo { minimized: Some(false), occluded: Some(hidden.contains(&id)), ..Default::default() });
    }
    let _ = ctx.run_logic(&input, |ctx| crate::control::drain_hidden(app, ctx));
}

fn names(app: &FigApp) -> Vec<String> {
    app.session.docs.iter().map(|d| d.doc.name.clone()).collect()
}

#[test]
fn closing_a_window_in_the_presenter_shows_the_unsaved_changes_dialog() {
    let mut session = Session::new();
    let frame_id = session.execute("node.create", &json!({"type": "frame", "width": 402, "height": 874})).unwrap()["id"].clone();
    let ctx = egui::Context::default();
    let mut app = FigApp::new(session, Box::new(NoServices));
    frame(&ctx, &mut app, false);
    app.session.execute("proto.present", &json!({"start": frame_id})).unwrap();
    app.handle_ui_requests();
    frame(&ctx, &mut app, false);
    assert!(app.ui.present.is_some());

    frame(&ctx, &mut app, true);
    assert!(app.ui.confirm_close);
    assert!(ctx.memory(|memory| memory.area_rect(egui::Id::new(Some(crate::i18n::tr("Unsaved changes")))).is_some()), "dialog must be on screen");
    frame(&ctx, &mut app, false);
    assert!(app.ui.present.is_some(), "asking to close does not dismiss the preview");
}

#[test]
fn windows_hold_tabs_and_close_with_their_files() {
    let mut s = Session::empty();
    for n in ["A", "B", "C"] {
        s.execute("file.new", &json!({"name": n})).unwrap();
    }
    let ctx = egui::Context::default();
    // As eframe does where windows are native.
    ctx.set_embed_viewports(false);
    let mut app = FigApp::new(s, Box::new(NoServices));
    frame(&ctx, &mut app, false);
    let w = crate::control::handle(&mut app, &ctx, "window.new", &json!({"index": 0})).unwrap()["window"].as_u64().unwrap();
    frame(&ctx, &mut app, false);
    let list = crate::windows::list(&app);
    assert_eq!(list[0]["files"], json!(["B", "C"]));
    assert_eq!(list[1]["files"], json!(["A"]));
    assert_eq!(list[1]["id"], json!(w));
    assert!(app.session.ds().unwrap().doc.name != "A", "the main window shows its own files");
    assert!(crate::control::handle(&mut app, &ctx, "window.focus", &json!({"id": 99})).is_err());

    // Unsaved changes: the window stays and asks.
    let b = app.session.docs.iter().position(|d| d.doc.name == "B").unwrap();
    app.session.execute("doc.switch", &json!({"index": b})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    frame(&ctx, &mut app, true);
    assert!(app.ui.confirm_close);
    assert_eq!(names(&app), ["A", "B", "C"]);

    // Saved (here: discarded) files close with the main window, which takes over the other one.
    app.ui.confirm_close = false;
    let b = app.session.docs.iter().position(|d| d.doc.name == "B").unwrap();
    app.session.execute("file.close", &json!({"index": b, "force": true})).unwrap();
    frame(&ctx, &mut app, true);
    frame(&ctx, &mut app, false);
    assert_eq!(names(&app), ["A"]);
    assert!(app.windows.list.is_empty());
    assert!(!app.ui.quit, "the app keeps running");
    assert_eq!(app.session.ds().unwrap().doc.name, "A");
}

#[test]
fn another_hidden_window_answers_requests_that_need_no_frame_in_its_own_file() {
    let mut s = Session::empty();
    for n in ["A", "B"] {
        s.execute("file.new", &json!({"name": n})).unwrap();
    }
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = FigApp::new(s, Box::new(NoServices));
    frame(&ctx, &mut app, false);
    let w = crate::control::handle(&mut app, &ctx, "window.new", &json!({"index": 0})).unwrap()["window"].as_u64().unwrap();
    frame(&ctx, &mut app, false);
    let other = crate::windows::viewport(w);
    let rect = |size: u32| json!({"window": w, "command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": size, "height": size}});
    let dirty = |app: &FigApp, name: &str| app.session.docs.iter().find(|d| d.doc.name == name).unwrap().is_dirty();

    // The main window is on screen, the other one covered: the request to it is answered in its
    // file (A), and the main window keeps showing its own (B).
    let edit = request(&mut app, "engine.execute", rect(10));
    logic(&ctx, &mut app, &[other]);
    assert!(edit.try_recv().expect("answered without the other window's frame").result.is_ok());
    assert!(dirty(&app, "A") && !dirty(&app, "B"), "in the other window's file");
    assert_eq!(app.session.ds().unwrap().doc.name, "B");
    assert_eq!(app.windows.current, 0);

    // Both hidden: still answered, and inspection sees the other window's file.
    let inspect = request(&mut app, "document.inspect", json!({"window": w, "depth": 0}));
    logic(&ctx, &mut app, &[ViewportId::ROOT, other]);
    assert_eq!(inspect.try_recv().expect("answered").result.unwrap()["name"], json!("A"));
    assert_eq!(app.session.ds().unwrap().doc.name, "B");

    // What needs a frame, and an unknown window, are errors at once: nothing waits for a window
    // that may stay hidden.
    let shot = request(&mut app, "ui.screenshot", json!({"window": w, "path": "unused.png"}));
    let unknown = request(&mut app, "window.list", json!({"window": 99}));
    logic(&ctx, &mut app, &[ViewportId::ROOT, other]);
    assert!(shot.try_recv().expect("answered").result.unwrap_err().contains(&format!("window {w} is minimised or hidden")));
    assert!(unknown.try_recv().expect("answered").result.unwrap_err().contains("no window 99"));
    assert!(app.control_queue.is_empty());
}

#[test]
fn a_request_for_a_window_on_screen_wakes_it_while_the_main_window_is_hidden() {
    let mut s = Session::empty();
    for n in ["A", "B"] {
        s.execute("file.new", &json!({"name": n})).unwrap();
    }
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = FigApp::new(s, Box::new(NoServices));
    frame(&ctx, &mut app, false);
    let w = crate::control::handle(&mut app, &ctx, "window.new", &json!({"index": 0})).unwrap()["window"].as_u64().unwrap();
    frame(&ctx, &mut app, false);
    let other = crate::windows::viewport(w);
    // Which windows are asked to draw from now on (an idle window draws no frames until asked).
    let woken = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = woken.clone();
    ctx.set_request_repaint_callback(move |info| log.lock().unwrap().push(info.viewport_id));

    // Only the main window's logic runs while it is hidden: the request for the other window,
    // which is on screen, waits for that window's frame, and that window is asked to draw it.
    let ui = request(&mut app, "ui.inspect", json!({"window": w}));
    logic(&ctx, &mut app, &[ViewportId::ROOT]);
    assert!(ui.try_recv().is_err(), "answered in the other window's frame");
    assert!(woken.lock().unwrap().contains(&other), "the other window is asked to draw: {:?}", woken.lock().unwrap());
}
