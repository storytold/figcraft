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

/// Answers each Save as with the next of `paths` (none left: cancelled) and counts the dialogs
/// asked for.
struct SaveAs {
    paths: Vec<std::path::PathBuf>,
    asked: std::rc::Rc<std::cell::Cell<usize>>,
}

impl crate::Services for SaveAs {
    fn pick_save(&mut self, _title: &str, _name: &str, _exts: &[&str]) -> crate::DialogFuture {
        self.asked.set(self.asked.get() + 1);
        let next = if self.paths.is_empty() { None } else { Some(self.paths.remove(0)) };
        Box::pin(std::future::ready(next))
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-close-save-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The main window with one file that has unsaved changes, saved at `path` first when given, and
/// the window asked to close (the question shows).
fn asking(path: Option<&std::path::Path>, save_as: Vec<std::path::PathBuf>) -> (egui::Context, FigApp, std::rc::Rc<std::cell::Cell<usize>>) {
    let mut s = Session::new();
    if let Some(p) = path {
        s.execute("file.saveAs", &json!({"path": p.to_string_lossy()})).unwrap();
    }
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let asked = std::rc::Rc::new(std::cell::Cell::new(0));
    let ctx = egui::Context::default();
    let mut app = FigApp::new(s, Box::new(SaveAs { paths: save_as, asked: asked.clone() }));
    frame(&ctx, &mut app, false);
    frame(&ctx, &mut app, true);
    assert!(app.ui.confirm_close, "the question shows");
    (ctx, app, asked)
}

fn rects_in(path: &std::path::Path) -> usize {
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
    s.doc().unwrap().nodes().filter(|n| n.name.starts_with("Rectangle")).count()
}

/// A few frames, as the window draws them.
fn settle(ctx: &egui::Context, app: &mut FigApp) {
    for _ in 0..4 {
        frame(ctx, app, false);
    }
}

#[test]
fn the_unsaved_changes_question_offers_save_dont_save_and_cancel() {
    // As the other Craft apps ask it (#1170): Don't Save, Cancel, and Save as the default.
    let (ctx, mut app, _) = asking(None, vec![]);
    ctx.enable_accesskit();
    frame(&ctx, &mut app, false);
    let out = ctx.run_ui(RawInput::default(), |ui| app.ui(ui));
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let buttons: Vec<String> =
        nodes.iter().filter(|(_, n)| n.role() == egui::accesskit::Role::Button).filter_map(|(_, n)| n.label().map(str::to_string)).collect();
    for want in ["Don't Save", "Cancel", "Save"] {
        assert!(buttons.iter().any(|b| b == want), "{want} in {buttons:?}");
    }
    assert!(!buttons.iter().any(|b| b == "Close without saving"));
}

#[test]
fn save_saves_a_file_with_a_path_and_then_closes() {
    let d = temp_dir("path");
    let path = d.join("a.figcraft");
    let (ctx, mut app, asked) = asking(Some(&path), vec![]);
    assert_eq!(rects_in(&path), 0);
    crate::windows::answer_close(&mut app, &ctx, crate::windows::CloseAnswer::Save);
    settle(&ctx, &mut app);
    assert_eq!(rects_in(&path), 1, "saved where it was");
    assert_eq!(asked.get(), 0, "no Save as for a file with a path");
    assert!(app.ui.quit, "then the window closes");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn save_waits_for_a_panel_field_to_commit() {
    // The click on Save takes the keyboard from a panel field, which commits what was typed next
    // frame: the save comes after that commit (stood in for by a command run before the frame).
    let d = temp_dir("field");
    let path = d.join("a.figcraft");
    let (ctx, mut app, _) = asking(Some(&path), vec![]);
    crate::windows::answer_close(&mut app, &ctx, crate::windows::CloseAnswer::Save);
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    settle(&ctx, &mut app);
    assert_eq!(rects_in(&path), 2, "the committed edit is saved too");
    assert!(app.ui.quit);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn save_asks_where_for_an_untitled_file_and_closes_once_saved() {
    let d = temp_dir("untitled");
    let path = d.join("new.figcraft");
    let (ctx, mut app, asked) = asking(None, vec![path.clone()]);
    crate::windows::answer_close(&mut app, &ctx, crate::windows::CloseAnswer::Save);
    settle(&ctx, &mut app);
    assert_eq!(asked.get(), 1, "Save as asks where");
    assert_eq!(rects_in(&path), 1, "saved there");
    assert!(app.ui.quit, "then the window closes");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_cancelled_save_as_or_a_failed_save_keeps_the_window_open() {
    // Save as cancelled: nothing is written, the file keeps its changes and the window stays.
    let (ctx, mut app, asked) = asking(None, vec![]);
    crate::windows::answer_close(&mut app, &ctx, crate::windows::CloseAnswer::Save);
    settle(&ctx, &mut app);
    assert_eq!(asked.get(), 1);
    assert!(!app.ui.quit && !app.ui.confirm_close, "open, and not asking again");
    assert!(app.session.ds().unwrap().is_dirty());
    assert_eq!(app.ui.close_after_save_as, None);

    // The save fails (its folder is gone): the error shows and the window stays.
    let d = temp_dir("gone");
    let path = d.join("a.figcraft");
    let (ctx, mut app, _) = asking(Some(&path), vec![]);
    std::fs::remove_dir_all(&d).unwrap();
    crate::windows::answer_close(&mut app, &ctx, crate::windows::CloseAnswer::Save);
    settle(&ctx, &mut app);
    assert!(!app.ui.quit, "a failed save keeps the window open");
    assert!(app.session.ds().unwrap().is_dirty());
}

#[test]
fn dont_save_closes_and_cancel_keeps_the_window() {
    let d = temp_dir("answers");
    let path = d.join("a.figcraft");
    let (ctx, mut app, _) = asking(Some(&path), vec![]);
    crate::windows::answer_close(&mut app, &ctx, crate::windows::CloseAnswer::Cancel);
    settle(&ctx, &mut app);
    assert!(!app.ui.quit && !app.ui.confirm_close);
    frame(&ctx, &mut app, true);
    crate::windows::answer_close(&mut app, &ctx, crate::windows::CloseAnswer::DontSave);
    settle(&ctx, &mut app);
    assert!(app.ui.quit, "Don't Save closes");
    assert_eq!(rects_in(&path), 0, "without saving");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn one_save_saves_every_untitled_file_of_the_window_then_closes() {
    // Two new files with changes: Save asks where for each in turn, then the window closes,
    // without asking the question again in between (#1173 review).
    let d = temp_dir("two");
    let (a, b) = (d.join("a.figcraft"), d.join("b.figcraft"));
    let (ctx, mut app, asked) = asking(None, vec![a.clone(), b.clone()]);
    app.session.execute("file.new", &json!({"name": "Second"})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    crate::windows::answer_close(&mut app, &ctx, crate::windows::CloseAnswer::Save);
    for _ in 0..8 {
        frame(&ctx, &mut app, false);
        assert!(!app.ui.confirm_close, "not asked again");
    }
    assert_eq!(asked.get(), 2);
    assert_eq!((rects_in(&a), rects_in(&b)), (1, 1), "both saved");
    assert!(app.ui.quit);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn enter_answers_save() {
    let d = temp_dir("enter");
    let path = d.join("a.figcraft");
    let (ctx, mut app, _) = asking(Some(&path), vec![]);
    frame(&ctx, &mut app, false);
    let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    let input = RawInput { events: vec![enter(true), enter(false)], ..Default::default() };
    ctx.run_ui(input, |ui| {
        crate::windows::on_close_request(&mut app, ui.ctx());
        app.ui(ui);
    })
    .textures_delta
    .clear();
    settle(&ctx, &mut app);
    assert_eq!(rects_in(&path), 1, "Enter saved");
    assert!(app.ui.quit);
    std::fs::remove_dir_all(&d).ok();
}
