//! The prototype player driven headless through egui: the preview pane opens from a command,
//! follows edits and the editor's selection, takes the keys its prototype uses, and the presented
//! frame scrolls inside the device's screen with the wheel and by dragging.

use egui::{Event, Key, Pos2, RawInput, Rect, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::{FigApp, NoServices};

pub(crate) struct Ui {
    pub(crate) ctx: egui::Context,
    pub(crate) app: FigApp,
    pub(crate) time: f64,
}

impl Ui {
    pub(crate) fn new(s: Session) -> Self {
        let mut u = Self { ctx: egui::Context::default(), app: FigApp::new(s, Box::new(NoServices)), time: 0.0 };
        u.frames(3, Vec::new);
        u
    }

    pub(crate) fn frame(&mut self, events: Vec<Event>) {
        self.frame_output(events);
    }

    /// [`Self::frame`], returning what egui output (the AccessKit tree when it's enabled).
    pub(crate) fn frame_output(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
        out
    }

    pub(crate) fn frames(&mut self, n: usize, events: impl Fn() -> Vec<Event>) {
        for _ in 0..n {
            self.frame(events());
        }
    }

    pub(crate) fn run(&mut self, cmd: &str, params: Value) -> Value {
        let r = self.app.session.execute(cmd, &params).unwrap();
        self.app.handle_ui_requests();
        self.frame(vec![]);
        r
    }

    /// The preview pane's window on screen.
    pub(crate) fn pane(&self) -> Rect {
        self.ctx.memory(|m| m.area_rect(crate::present::pane_id())).expect("the pane is open")
    }

    /// The pane's player state (`ui.inspect` → `preview`).
    pub(crate) fn preview(&self) -> Value {
        self.app.inspect()["preview"].clone()
    }

    pub(crate) fn scroll_y(&self) -> f64 {
        self.preview()["scroll"][1].as_f64().unwrap()
    }

    pub(crate) fn button(&mut self, p: Pos2, pressed: bool) {
        self.frame(vec![
            Event::PointerMoved(p),
            Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
        ]);
    }

    pub(crate) fn key(&mut self, at: Pos2, key: Key) {
        let k = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
        self.frame(vec![Event::PointerMoved(at), k(true), k(false)]);
    }
}

pub(crate) fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// An iPhone 16 page: screen A (393×2000, scrolls vertically, a tab bar fixed to its bottom, a
/// rectangle selected inside it) and screen B.
pub(crate) fn setup() -> (Ui, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let ex = |s: &mut Session, c: &str, p: Value| s.execute(c, &p).unwrap();
    ex(&mut s, "proto.settings", json!({"device": "iPhone 16"}));
    let a = id(&ex(&mut s, "node.create", json!({"type": "frame", "name": "A", "x": 0, "y": 0, "width": 393, "height": 2000})));
    ex(&mut s, "node.set", json!({"id": a, "props": {"overflow": "vertical"}}));
    let card = id(&ex(&mut s, "node.create", json!({"type": "rectangle", "parent": a, "x": 20, "y": 100, "width": 200, "height": 100})));
    let bar = id(&ex(&mut s, "node.create", json!({"type": "rectangle", "parent": a, "x": 0, "y": 1940, "width": 393, "height": 60})));
    ex(&mut s, "node.set", json!({"id": bar, "props": {"scrollPosition": "fixed", "constraints": {"vertical": "bottom"}}}));
    let b = id(&ex(&mut s, "node.create", json!({"type": "frame", "name": "B", "x": 500, "y": 0, "width": 393, "height": 852})));
    ex(&mut s, "select.set", json!({"ids": [card]}));
    (Ui::new(s), a, b, card)
}

#[test]
fn preview_pane_opens_follows_edits_and_the_selection() {
    let (mut u, a, b, card) = setup();
    assert_eq!(u.run("proto.preview", json!({}))["start"], json!(a), "the selected layer's frame");
    let pane = u.pane();
    assert!(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0)).contains_rect(pane), "{pane:?}");
    assert_eq!(u.preview()["current"], json!(a));
    assert_eq!(u.preview()["device"], "iPhone 16");
    // Edits show live.
    u.run("node.set", json!({"id": a, "props": {"name": "Home"}}));
    assert_eq!(u.preview()["name"], "Home");
    // Arrow keys over the pane step to the next frame there and don't nudge the selection.
    let before = u.app.session.doc().unwrap().world_bounds(card).unwrap();
    u.key(pane.center(), Key::ArrowRight);
    assert_eq!(u.preview()["current"], json!(b));
    assert_eq!(u.app.session.doc().unwrap().world_bounds(card).unwrap(), before);
    // Away from the pane the editor has the keyboard.
    u.key(pos2(600.0, 450.0), Key::ArrowRight);
    assert_eq!(u.preview()["current"], json!(b));
    assert_ne!(u.app.session.doc().unwrap().world_bounds(card).unwrap(), before, "nudged");
    // Selecting another top-level frame in the editor shows it.
    u.run("select.set", json!({"ids": [b]}));
    assert_eq!(u.preview()["current"], json!(b));
    u.run("select.set", json!({"ids": [a]}));
    assert_eq!(u.preview()["current"], json!(a));
    // Deselecting and selecting the same frame again keeps where the preview got to.
    u.key(pane.center(), Key::ArrowRight);
    u.run("select.set", json!({"ids": []}));
    u.run("select.set", json!({"ids": [a]}));
    assert_eq!(u.preview()["current"], json!(b));
    // Deleting the screen shown falls back to the start, here B itself: nothing to show.
    u.run("proto.preview", json!({"start": b}));
    u.run("edit.delete", json!({"ids": [b]}));
    assert!(u.app.ui.preview.is_some(), "the pane stays open");
    u.run("edit.undo", json!({}));
    assert_eq!(u.preview()["current"], json!(b), "back when the frame is");
    u.run("proto.closePreview", json!({}));
    assert!(u.app.ui.preview.is_none());
}

#[test]
fn the_frame_scrolls_on_the_device_screen_by_wheel_and_drag() {
    let (mut u, a, b, _) = setup();
    u.run("proto.preview", json!({"start": a}));
    assert_eq!(u.preview()["viewport"], json!([393.0, 852.0]));
    assert_eq!(u.preview()["scrollRange"], json!([0.0, 1148.0]));
    let c = u.pane().center();
    // The wheel scrolls (egui smooths it over a few frames).
    let wheel = move || {
        vec![
            Event::PointerMoved(c),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -60.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            },
        ]
    };
    u.frames(6, wheel);
    u.frames(20, Vec::new);
    let after_wheel = u.scroll_y();
    assert!(after_wheel > 0.0, "{after_wheel}");
    // Dragging up scrolls further, by the drag over the zoom, and isn't a click.
    u.button(c, true);
    for k in 1..=10 {
        u.frame(vec![Event::PointerMoved(c - vec2(0.0, 10.0 * k as f32))]);
    }
    u.button(c - vec2(0.0, 100.0), false);
    let dragged = u.scroll_y() - after_wheel;
    assert!(dragged > 100.0, "the screen is shown below 100%: {dragged}");
    // Never past the end: dragged further it stretches, and springs back when let go.
    for _ in 0..20 {
        u.button(c, true);
        u.frame(vec![Event::PointerMoved(c - vec2(0.0, 300.0))]);
        u.button(c - vec2(0.0, 300.0), false);
    }
    u.frames(90, Vec::new);
    assert_eq!(u.scroll_y(), 1148.0);
    // Navigating starts the next frame at the top.
    u.key(c, Key::ArrowRight);
    assert_eq!(u.preview()["current"], json!(b));
    assert_eq!(u.scroll_y(), 0.0);
}

/// A long top-level frame scrolls on a phone with no overflow setting, as in Figma: the landing
/// page template's Phone frame ("No scrolling", hugging its height) scrolls by the wheel.
#[test]
fn a_top_level_frame_taller_than_the_phone_scrolls_without_an_overflow_setting() {
    let mut s = Session::new();
    s.execute("file.newFromTemplate", &json!({"template": "landing-page"})).unwrap();
    s.execute("proto.settings", &json!({"device": "iPhone 16", "responsive": true})).unwrap();
    let doc = s.doc().unwrap();
    let page = doc.first_page().unwrap();
    let phone = doc.top_frames(page).into_iter().find(|f| doc.node(*f).unwrap().name == "Phone").unwrap();
    assert_eq!(doc.node(phone).unwrap().overflow, figcraft_doc::proto::Overflow::None);
    let mut u = Ui::new(s);
    u.run("proto.preview", json!({"start": phone}));
    let range = u.preview()["scrollRange"][1].as_f64().unwrap();
    assert!(range > 1000.0, "the page is far taller than the screen: {range}");
    let c = u.pane().center();
    let wheel = move || {
        vec![
            Event::PointerMoved(c),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -60.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            },
        ]
    };
    u.frames(6, wheel);
    u.frames(20, Vec::new);
    assert!(u.scroll_y() > 0.0, "{}", u.scroll_y());
}

#[test]
fn full_window_presenter_shares_the_player() {
    let (mut u, a, ..) = setup();
    u.run("proto.present", json!({"start": a}));
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(a));
    crate::control::handle(&mut u.app, &u.ctx, "ui.set", &json!({"presentScroll": [0, 5000]})).unwrap();
    assert_eq!(u.app.inspect()["presenter"]["scroll"], json!([0.0, 1148.0]));
    u.frame(vec![]);
    u.key(pos2(700.0, 450.0), Key::Escape);
    assert!(u.app.ui.present.is_none());
    // The control channel opens and closes the pane too.
    crate::control::handle(&mut u.app, &u.ctx, "ui.set", &json!({"preview": a.0})).unwrap();
    assert_eq!(u.preview()["current"], json!(a));
    crate::control::handle(&mut u.app, &u.ctx, "ui.set", &json!({"preview": false})).unwrap();
    assert!(u.app.ui.preview.is_none());
    assert!(crate::control::handle(&mut u.app, &u.ctx, "ui.set", &json!({"presentScroll": [0, 1]})).is_err(), "nothing presented");
}
