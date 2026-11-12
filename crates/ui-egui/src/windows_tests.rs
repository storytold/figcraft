//! Several windows: a window holds its own tabs; closing it closes its files, asking first when
//! one has unsaved changes, and the main window takes over another window instead of quitting.

use std::sync::mpsc::{Receiver, channel};

use egui::{RawInput, ViewportEvent, ViewportId, ViewportInfo};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::control::{ControlReply, ControlRequest};
use crate::{FigApp, NoServices};

/// One main-window frame, as the desktop app runs it; `close` asks the window to close. Returns
/// the commands the frame sent the main window.
fn frame(ctx: &egui::Context, app: &mut FigApp, close: bool) -> Vec<egui::ViewportCommand> {
    let mut input = RawInput::default();
    if close {
        input.viewports.insert(ViewportId::ROOT, ViewportInfo { events: vec![ViewportEvent::Close], ..Default::default() });
    }
    let mut out = ctx.run_ui(input, |ui| {
        crate::windows::on_close_request(app, ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    out.viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default()
}

/// A control request queued for the UI, and where its reply arrives.
fn request(app: &mut FigApp, method: &str, params: Value) -> Receiver<ControlReply> {
    let (reply, rx) = channel();
    app.control_queue.push(ControlRequest { id: json!(1), method: method.into(), params, reply, state: Default::default() });
    rx
}

/// The desktop app's logic as eframe runs it for the main window, with each window's visibility
/// (`hidden`: minimised or covered). While the main window is hidden too this is all that runs.
fn logic(ctx: &egui::Context, app: &mut FigApp, hidden: &[ViewportId]) {
    let mut input = RawInput::default();
    for id in std::iter::once(ViewportId::ROOT).chain(app.windows.list.iter().map(|w| crate::windows::viewport(w.id))) {
        input.viewports.insert(id, ViewportInfo { minimized: Some(false), occluded: Some(hidden.contains(&id)), ..Default::default() });
    }
    let _ = ctx.run_logic(&input, |ctx| crate::control::background(app, ctx));
}

fn names(app: &FigApp) -> Vec<String> {
    app.session.docs.iter().map(|d| d.doc.name.clone()).collect()
}

/// Asserts closing a window in the presenter shows the unsaved-changes dialog and does not
/// dismiss the preview.
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
    assert!(ctx.memory(|memory| memory.area_rect(egui::Id::unique(Some(crate::i18n::tr("Unsaved changes")))).is_some()), "dialog must be on screen");
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

/// Asserts a hidden other window answers edits and inspection in its own file while the main
/// window keeps its file, and a capture parks with a deadline and times out once.
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

    // A capture that needs a frame is admitted and asks the hidden window to paint, rather than
    // failing at once; only its deadline expires it (once).
    // Model a request admitted long after startup, without waiting in real time.
    app.clock.set_test_now(Some(120.0));
    let shot = request(&mut app, "ui.screenshot", json!({"window": w, "path": "unused.png"}));
    let unknown = request(&mut app, "window.list", json!({"window": 99}));
    logic(&ctx, &mut app, &[ViewportId::ROOT, other]);
    assert!(shot.try_recv().is_err(), "waits for a forced hidden paint");
    assert_eq!(app.capture.shots.len(), 1, "one admitted capture");
    assert!(unknown.try_recv().expect("answered").result.unwrap_err().contains("no window 99"));
    assert!(app.control_queue.is_empty());
    // The whole request lifetime is over: one timeout reply, and nothing retained.
    app.clock.set_test_now(Some(app.clock.now() + crate::control::SHOT_TIMEOUT + 1.0));
    logic(&ctx, &mut app, &[ViewportId::ROOT, other]);
    assert!(shot.try_recv().expect("answered").result.unwrap_err().contains("didn't deliver a screenshot"));
    assert!(app.capture.shots.is_empty(), "released after its deadline");
    app.clock.set_test_now(None);
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

/// Asserts every direct UI frame clears the logic-tick marker so background work runs each
/// frame.
#[test]
fn every_direct_ui_frame_runs_background_work() {
    // Direct `App::ui` harnesses never call `App::logic`: the tick marker the UI pass consumes
    // must not leak into the next frame, or autosave/live/control work would run every other frame.
    let ctx = egui::Context::default();
    let mut app = FigApp::new(Session::empty(), Box::new(NoServices));
    for _ in 0..3 {
        frame(&ctx, &mut app, false);
        assert!(!app.logic_ticked, "the next direct frame ticks for itself");
    }
}

/// Asserts synthetic input for a child window reaches the pane that draws it, leaving the
/// main window's pane untouched.
#[test]
fn synthetic_input_for_a_child_window_reaches_the_pane_that_draws_it() {
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
    let events = vec![egui::Event::PointerMoved(egui::pos2(10.0, 10.0))];
    // A request applied while the child draws must go to the live pane, not the swapped-out
    // default `Pane` that `enter` took out (which `leave` would overwrite).
    let pushed = crate::windows::as_window(&mut app, w, |app| app.push_control_input(events.clone()));
    assert_eq!(pushed, Some(true));
    assert!(app.control_input.is_empty(), "the main window's pane is untouched");
    assert_eq!(crate::windows::take_input(&mut app, other), events, "the child's next frame takes its batch");
}

/// Asserts a child raw-input hook with no batch does not error or drain the root window's
/// pending acknowledgement.
#[test]
fn a_frame_without_a_batch_does_not_error_another_windows_acknowledgement() {
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
    // The root's batch awaits the pass that presents it.
    let (tx, rx) = channel();
    app.input_acks.push(crate::PendingAck { viewport: ViewportId::ROOT, tx, reply: ControlReply { id: json!(5), result: Ok(json!({})) } });
    // The child's raw-input hook runs first in the same frame and has no batch of its own: that
    // is not a document change, so the root's acknowledgement must neither be errored nor drained.
    let events = crate::windows::take_input(&mut app, crate::windows::viewport(w));
    assert!(events.is_empty());
    assert!(rx.try_recv().is_err(), "no bogus document-change reply");
    assert_eq!(app.input_acks.len(), 1, "the root's ack still waits for its own batch");
    assert_eq!(app.input_acks[0].reply.id, json!(5));
}

/// Asserts a child window runs `viewport_setup` and its pane keeps the dark theme after the
/// swap back.
#[test]
fn a_child_window_runs_the_ui_setup_the_main_window_does() {
    // eframe calls `App::ui` only for the root window, so a child that draws while the root does
    // not must still install the theme/language/fonts and its own `ui.dark`.
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
    app.session.prefs.theme = "dark".into();
    let dark = crate::windows::as_window(&mut app, w, |app| {
        app.viewport_setup(&ctx);
        app.inspect()["dark"].clone()
    });
    assert_eq!(dark, Some(json!(true)), "the child's pane knows the dark theme");
    // It stays with the pane after the swap back, so its tabs and dialogs choose the right theme.
    let again = crate::windows::as_window(&mut app, w, |app| app.inspect()["dark"].clone());
    assert_eq!(again, Some(json!(true)));
}

/// Asserts a closed and later reused viewport does not inherit its old IME context
/// generation.
#[test]
fn an_emptied_window_forgets_its_ime_context() {
    // `windows::begin` ends the viewport when a window loses its last file; a later window that
    // reuses the viewport identity must not inherit the old context generation.
    use egui::ime_context::{CommittedText, ImeContextTracker, ImeObservation, ImeTarget};
    use egui::{IMEPurpose, Id};

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
    let vp = crate::windows::viewport(w);
    let target = ImeTarget::new(Id::unique("figcraft.test.ime"), 3);
    let observe = |tracker: &mut ImeContextTracker| {
        let text = "a";
        let observation = ImeObservation::new(1, 1, target).purpose(IMEPurpose::Normal).committed(Some(CommittedText::new(text, 1, 1)));
        tracker.observe(vp, Some(observation))
    };
    let first = observe(&mut app.ime).expect("first focused observation");

    if let Some(win) = app.windows.list.iter_mut().find(|x| x.id == w) {
        win.docs.clear();
    }
    frame(&ctx, &mut app, false);
    assert!(!app.windows.exists(w), "the empty window closed");
    let again = observe(&mut app.ime).expect("a new context");
    assert_ne!(again.context_generation, first.context_generation, "the closed viewport's context was forgotten");
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
    let mut out = ctx.run_ui(RawInput::default(), |ui| app.ui(ui));
    // A canvas picture that lands in this frame uploads a texture nobody applies here.
    out.textures_delta.clear();
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

/// macOS's Quit (⌘Q, the app menu's Quit item, the Dock) reaches the app as a close request on
/// the main window (#857: the craft-menu app menu replaces winit's, whose Quit ended the process
/// at once). With unsaved changes the window cancels the close, so the process stays, and asks;
/// Don't Save then closes it.
#[test]
fn a_quit_with_unsaved_changes_waits_for_the_answer() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let ctx = egui::Context::default();
    let mut app = FigApp::new(s, Box::new(NoServices));
    frame(&ctx, &mut app, false);
    let sent = frame(&ctx, &mut app, true);
    assert!(sent.contains(&egui::ViewportCommand::CancelClose), "the quit is held: {sent:?}");
    assert!(app.ui.confirm_close && !app.ui.quit, "and the question asks");
    crate::windows::answer_close(&mut app, &ctx, crate::windows::CloseAnswer::DontSave);
    let sent = frame(&ctx, &mut app, false);
    assert!(app.ui.quit && sent.contains(&egui::ViewportCommand::Close), "Don't Save quits: {sent:?}");
}

/// A quit while the main window is hidden reaches only the app's logic, which runs without the
/// frame that acts on it; a second quit before the window draws again must not count twice. The
/// main window has no unsaved changes and another window has: one quit lets the main window take
/// that window over (its files keep their changes), and a doubled one must not then quit the app
/// as if those files had been answered for.
#[test]
fn quitting_twice_before_the_window_draws_closes_it_once() {
    let mut s = Session::empty();
    for n in ["A", "B"] {
        s.execute("file.new", &json!({"name": n})).unwrap();
    }
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = FigApp::new(s, Box::new(NoServices));
    frame(&ctx, &mut app, false);
    crate::control::handle(&mut app, &ctx, "window.new", &json!({"index": 0})).unwrap();
    frame(&ctx, &mut app, false);
    // A, in the other window, has unsaved changes; B, in the main window, has none.
    let a = app.session.docs.iter().position(|d| d.doc.name == "A").unwrap();
    app.session.execute("doc.switch", &json!({"index": a})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    frame(&ctx, &mut app, false);
    // Two quits while the main window is hidden: only the logic runs.
    for _ in 0..2 {
        let mut input = RawInput::default();
        input.viewports.insert(ViewportId::ROOT, ViewportInfo { events: vec![ViewportEvent::Close], occluded: Some(true), ..Default::default() });
        let _ = ctx.run_logic(&input, |ctx| crate::windows::on_close_request(&mut app, ctx));
    }
    frame(&ctx, &mut app, false);
    frame(&ctx, &mut app, false);
    assert!(app.session.docs.iter().any(|d| d.doc.name == "A" && d.is_dirty()), "A keeps its changes: {:?}", names(&app));
    assert!(!app.ui.quit, "the app didn't quit over A's unsaved changes");
}
