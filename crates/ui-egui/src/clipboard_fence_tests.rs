//! Programmatic gestures never replace the user's system clipboard: keys and clicks injected
//! through the control channel (`ui.input`, `ui.menu.invoke`, …) may copy inside FigCraft but the
//! OS clipboard is the user's. Every copy command goes through `FigApp::clipboard_set_text` /
//! `FigApp::clipboard_set_png`, which drop the write while the frame's input is synthetic.

use std::sync::{Arc, Mutex};

use egui::{Event, Pos2, RawInput, Rect, vec2};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::control::ControlRequest;
use crate::{FigApp, Services};

#[derive(Default)]
struct Recorded {
    text: Vec<String>,
    pngs: usize,
    /// What the system clipboard holds now (the last text written).
    last_text: Option<String>,
}

/// The system clipboard of the test machine.
struct Clipboard(Arc<Mutex<Recorded>>);

impl Services for Clipboard {
    /// Record a system-clipboard text write in the test clipboard service.
    fn clipboard_set_text(&mut self, s: &str) {
        let mut r = self.0.lock().unwrap();
        r.text.push(s.to_string());
        r.last_text = Some(s.to_string());
    }
    /// Return the test system clipboard's last text.
    fn clipboard_get_text(&mut self) -> Option<String> {
        self.0.lock().unwrap().last_text.clone()
    }
    /// Count a PNG clipboard write in the test clipboard service.
    fn clipboard_set_png(&mut self, _w: u32, _h: u32, _rgba: &[u8]) {
        self.0.lock().unwrap().pngs += 1;
    }
}

struct Win {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
    recorded: Arc<Mutex<Recorded>>,
}

impl Win {
    /// Create a test window over a session containing one selected rectangle.
    fn new() -> Self {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
        let mut win = Self::with_session(s);
        let _ = win.app.run("select.all", json!({}));
        win
    }

    /// Build a test window around `s` with a recording clipboard service and run three frames.
    fn with_session(s: Session) -> Self {
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let app = FigApp::new(s, Box::new(Clipboard(recorded.clone())));
        let mut win = Self { ctx: egui::Context::default(), app, time: 0.0, recorded };
        for _ in 0..3 {
            win.frame(vec![]);
        }
        win
    }

    /// Run one production UI frame with `events` at the next 60 Hz test time.
    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        self.app.clock.set_test_now(Some(self.time));
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
    }

    /// Return the recorded `(text writes, PNG writes)` count.
    fn writes(&self) -> (usize, usize) {
        let r = self.recorded.lock().unwrap();
        (r.text.len(), r.pngs)
    }
}

/// Asserts that a control-channel copy shortcut writes no system clipboard, still runs the
/// command and is acknowledged.
#[test]
fn a_synthetic_copy_shortcut_leaves_the_system_clipboard_alone() {
    let mut w = Win::new();
    assert!(!w.app.session.selection().is_empty(), "something to copy");
    let (tx, rx) = std::sync::mpsc::channel();
    w.app.control_queue.push(ControlRequest {
        id: json!(1),
        method: "ui.input".into(),
        // Shift+Cmd+C: `edit.copyAsPng`, which wrote the OS clipboard directly before the fence.
        params: json!({"events": [{"key": "C", "shift": true, "cmd": true}]}),
        reply: tx,
        state: Default::default(),
    });
    w.frame(vec![]);
    let events = crate::windows::take_input(&mut w.app, egui::ViewportId::ROOT);
    assert!(!events.is_empty(), "the injected shortcut");
    w.frame(events);
    assert_eq!(w.writes(), (0, 0), "a control-channel gesture never replaces the user's clipboard");
    assert!(
        w.app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("Copied")),
        "the command itself ran (only its OS clipboard write is fenced): {:?}",
        w.app.ui.toast
    );
    assert!(rx.try_recv().is_ok(), "the batch is acknowledged");
}

/// Asserts copy commands run under the synthetic-control fence write nothing, while the same
/// command afterwards writes once.
#[test]
fn a_copy_command_invoked_for_a_synthetic_click_still_keeps_the_clipboard() {
    let mut w = Win::new();
    // What a synthetic pointer click on Copy runs, fenced as `control::drain`/`background` fence it.
    w.app.control_synthetic = true;
    crate::menus::invoke(&mut w.app, "edit.copyAsCss");
    crate::menus::invoke(&mut w.app, "edit.copyAsText");
    assert_eq!(w.writes(), (0, 0));
    w.app.control_synthetic = false;
    // A user's own copy still writes.
    crate::menus::invoke(&mut w.app, "edit.copyAsCss");
    assert_eq!(w.writes(), (1, 0));
}

/// Asserts a real `ui.menu.invoke` of Copy link is fenced from the system clipboard, while the
/// same menu item invoked by the user writes the selected layer's link.
#[test]
fn a_synthetic_ui_menu_invoke_copy_link_keeps_the_clipboard_then_a_user_copy_writes_the_link() {
    let mut w = Win::new();
    let selected = *w.app.session.selection().first().expect("a selected layer");
    let link = format!("figcraft://node/{}", selected.0);

    let (tx, rx) = std::sync::mpsc::channel();
    w.app.control_queue.push(ControlRequest {
        id: json!(1),
        method: "ui.menu.invoke".into(),
        params: json!({"id": "edit.copyLink"}),
        reply: tx,
        state: Default::default(),
    });
    w.frame(vec![]);
    let reply = rx.try_recv().expect("the menu invocation is acknowledged");
    assert_eq!(reply.id, json!(1));
    assert!(reply.result.is_ok(), "the menu item ran: {:?}", reply.result);
    assert_eq!(w.writes(), (0, 0), "a synthetic menu invocation never replaces the user's clipboard");
    assert_eq!(w.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Link copied"));
    assert!(!w.app.control_synthetic, "the fence ends with the frame");

    crate::menus::invoke(&mut w.app, "edit.copyLink");
    assert_eq!(w.writes(), (1, 0), "the user's invocation writes exactly one text clipboard entry");
    let recorded = w.recorded.lock().unwrap();
    assert_eq!(recorded.text, vec![link.clone()]);
    assert_eq!(recorded.last_text.as_deref(), Some(link.as_str()));
}

/// Return the characters of a text node in the test window.
fn edit_text(w: &Win, id: NodeId) -> String {
    match &w.app.session.doc().unwrap().node(id).unwrap().kind {
        NodeKind::Text(t) => t.characters.clone(),
        _ => panic!("not text"),
    }
}

/// Asserts a synthetic Cut deletes the selected text, keeps the edit session active, writes
/// no system clipboard, and ends its fence with the frame.
#[test]
fn a_synthetic_cut_deletes_the_selection_but_leaves_the_users_clipboard_alone() {
    let mut s = Session::new();
    let id = NodeId(s.execute("text.create", &json!({"text": "abc", "x": 40, "y": 40})).unwrap()["id"].as_u64().unwrap());
    let mut w = Win::with_session(s);
    figcraft_engine::textedit::begin(&mut w.app.session, id, Some(3), false).unwrap();
    figcraft_engine::textedit::set_caret(&mut w.app.session, 0, true).unwrap();

    // A control-channel gesture is fenced from the OS clipboard, but its Cut still edits the
    // document: the selection is deleted exactly as a user's Cut would delete it.
    w.app.push_control_input(vec![Event::Cut]);
    let events = crate::windows::take_input(&mut w.app, egui::ViewportId::ROOT);
    assert_eq!(events, [Event::Cut], "the production input path owns the synthetic frame");
    w.frame(events);
    assert_eq!(w.writes(), (0, 0), "a synthetic cut never writes the user's clipboard");
    assert_eq!(edit_text(&w, id), "", "the selected text is deleted");
    assert!(w.app.session.ds().unwrap().text_edit.is_some(), "the edit session stays active");
    assert!(!w.app.control_synthetic, "the fence ends with the frame");
}

/// Asserts a programmatic copy leaves the system clipboard alone and invalidates the stale
/// mirror, so a later user paste uses the system text rather than the programmatic layers.
#[test]
fn a_programmatic_copy_does_not_let_a_stale_mirror_hide_the_users_paste() {
    let mut s = Session::new();
    let hello = figcraft_doc::NodeId(s.execute("text.create", &json!({"text": "hello", "x": 10, "y": 10})).unwrap()["id"].as_u64().unwrap());
    let other = figcraft_doc::NodeId(s.execute("text.create", &json!({"text": "other", "x": 10, "y": 80})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [hello]})).unwrap();
    let mut w = Win::with_session(s);

    // The user copies the "hello" text layer: the OS clipboard and `last_copy` become "hello".
    crate::menus::invoke(&mut w.app, "edit.copy");
    assert_eq!(w.app.ui.last_copy.as_deref(), Some("hello"));
    assert_eq!(w.writes().0, 1, "the user's copy reaches the system clipboard");

    // A control-channel copy changes the engine clipboard to "other" but never writes the system
    // clipboard. The stale "hello" mirror must not claim the programmatic selection is ours.
    let _ = w.app.run("select.set", json!({"ids": [other]}));
    w.app.control_synthetic = true;
    crate::menus::invoke(&mut w.app, "edit.copy");
    w.app.control_synthetic = false;
    assert_eq!(w.writes().0, 1, "the programmatic copy leaves the system clipboard alone");
    assert_eq!(w.app.ui.last_copy, None, "the old system-clipboard mirror is invalidated");

    // The user pastes: the system clipboard's own text wins over the programmatic layers.
    crate::menus::paste(&mut w.app, Some("hello"), None);
    let doc = w.app.session.doc().unwrap();
    let texts: Vec<String> = doc
        .pages
        .iter()
        .filter_map(|p| doc.node(*p))
        .flat_map(|p| p.children.iter().copied())
        .filter_map(|id| doc.node(id).and_then(|n| n.kind.text().map(|t| t.characters.clone())))
        .collect();
    assert_eq!(texts.iter().filter(|t| t.as_str() == "hello").count(), 2, "the user's text was pasted");
    assert_eq!(texts.iter().filter(|t| t.as_str() == "other").count(), 1, "the programmatic selection was not pasted");
}

#[test]
fn a_synthetic_copy_for_ai_keeps_the_clipboard_then_a_user_copy_writes() {
    let mut s = Session::new();
    s.execute("comment.add", &json!({"x": 10, "y": 10, "text": "Fix the padding", "author": "You"})).unwrap();
    let mut w = Win::with_session(s);
    w.app.control_synthetic = true;
    crate::comments::copy_for_ai(&mut w.app, None);
    assert_eq!(w.writes(), (0, 0), "synthetic input must not replace the user's clipboard");
    w.app.control_synthetic = false;
    crate::comments::copy_for_ai(&mut w.app, None);
    assert_eq!(w.writes(), (1, 0), "the user's copy writes once");
    let recorded = w.recorded.lock().unwrap();
    assert!(recorded.last_text.as_deref().is_some_and(|t| t.contains("Fix the padding")), "{:?}", recorded.last_text);
}
