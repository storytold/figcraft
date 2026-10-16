//! Dragging rows in the layers panel, driven through egui headless: holding over a collapsed
//! frame opens it, a layer can't be dropped into itself, Escape cancels, and the list scrolls at
//! its edges.

use egui::{Event, Pos2, RawInput, Rect, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, NoServices};

pub(crate) struct Ui {
    pub(crate) ctx: egui::Context,
    pub(crate) app: FigApp,
    time: f64,
}

impl Ui {
    pub(crate) fn new(s: Session) -> Self {
        let mut u = Self { ctx: egui::Context::default(), app: FigApp::new(s, Box::new(NoServices)), time: 0.0 };
        for _ in 0..3 {
            u.frame(vec![]);
        }
        u
    }

    pub(crate) fn frame(&mut self, events: Vec<Event>) {
        self.frame_output(events);
    }

    /// [`Self::frame`], returning what egui output (the AccessKit tree when it's enabled).
    pub(crate) fn frame_output(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.frame_with(events, Default::default())
    }

    /// [`Self::frame_output`] with `modifiers` held.
    pub(crate) fn frame_with(&mut self, mut events: Vec<Event>, modifiers: egui::Modifiers) -> egui::FullOutput {
        events.insert(0, Event::ModifiersChanged(modifiers));
        self.frame_raw(RawInput { events, ..Default::default() })
    }

    /// One frame of `raw` input (files dropped on or hovered over the window, say), on the test
    /// screen and clock.
    pub(crate) fn frame_raw(&mut self, raw: RawInput) -> egui::FullOutput {
        self.time += 1.0 / 60.0;
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), ..raw };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
        out.textures_delta.clear();
        out
    }

    /// The middle of `id`'s row in the layers panel (found by hovering down the panel).
    pub(crate) fn row_at(&mut self, id: NodeId) -> Pos2 {
        let mut hits = Vec::new();
        for y in (60..900).step_by(2) {
            let p = Pos2::new(150.0, y as f32);
            // The panel only sets the hover when a row is under the pointer.
            self.app.session.ds_mut().unwrap().hover = None;
            self.frame(vec![Event::PointerMoved(p)]);
            if self.app.session.ds().unwrap().hover == Some(id) {
                hits.push(p.y);
            } else if !hits.is_empty() {
                break;
            }
        }
        assert!(!hits.is_empty(), "no row for {id:?}");
        Pos2::new(150.0, (hits[0] + hits[hits.len() - 1]) / 2.0)
    }

    pub(crate) fn button(&mut self, p: Pos2, pressed: bool) {
        self.frame(vec![
            Event::PointerMoved(p),
            Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
        ]);
    }

    /// Press on `from` and move to `to` in small steps (a drag, not yet released).
    pub(crate) fn drag_to(&mut self, from: Pos2, to: Pos2) {
        self.button(from, true);
        for k in 1..=10 {
            self.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 10.0))]);
        }
    }
}

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A frame with one child (collapsed in the panel) and a rectangle beside it.
fn setup() -> (Ui, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let frame = id(s.execute("node.create", &json!({"type": "frame", "x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let child = id(s.execute("node.create", &json!({"type": "ellipse", "parent": frame, "x": 10, "y": 10, "width": 10, "height": 10})).unwrap());
    let rect = id(s.execute("node.create", &json!({"type": "rectangle", "x": 200, "y": 0, "width": 10, "height": 10})).unwrap());
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.expanded.clear();
    u.frame(vec![]);
    (u, frame, child, rect)
}

#[test]
fn holding_over_a_collapsed_frame_opens_it_and_dropping_moves_the_layer_in() {
    let (mut u, frame, child, rect) = setup();
    let (r, f) = (u.row_at(rect), u.row_at(frame));
    u.drag_to(r, f);
    assert!(!u.app.ui.expanded.contains(&frame));
    // Hold for a second over the middle of the frame's row.
    for _ in 0..60 {
        u.frame(vec![Event::PointerMoved(f)]);
    }
    assert!(u.app.ui.expanded.contains(&frame), "spring-loaded open");
    u.button(f, false);
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.node(rect).unwrap().parent, Some(frame), "dropped into the frame");
    assert!(doc.node(frame).unwrap().children.contains(&child));
}

#[test]
fn a_layer_cant_drop_into_its_own_child_and_escape_cancels() {
    let (mut u, frame, child, rect) = setup();
    u.app.ui.expanded.insert(frame);
    u.frame(vec![]);
    let (f, c) = (u.row_at(frame), u.row_at(child));
    let parent_before = u.app.session.doc().unwrap().node(frame).unwrap().parent;
    u.drag_to(f, c);
    u.button(c, false);
    assert_eq!(u.app.session.doc().unwrap().node(frame).unwrap().parent, parent_before, "not moved into itself");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    // Escape during a drag: releasing over a target then does nothing.
    let (r, f) = (u.row_at(rect), u.row_at(frame));
    u.drag_to(r, f);
    u.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    u.button(f, false);
    assert_ne!(u.app.session.doc().unwrap().node(rect).unwrap().parent, Some(frame));
}

#[test]
fn dropping_layers_on_another_page_moves_them_there() {
    let (mut u, _frame, _child, rect) = setup();
    let p1 = u.app.session.page().unwrap();
    let p2 = id(u.app.session.execute("page.add", &json!({})).unwrap());
    u.app.session.execute("page.select", &json!({"id": p1})).unwrap();
    u.frame(vec![]);
    // Find Page 2's row in the Pages list: the click that switches to it.
    let mut at = None;
    for y in (90..400).step_by(4) {
        let p = Pos2::new(150.0, y as f32);
        u.button(p, true);
        u.button(p, false);
        if u.app.session.page().unwrap() == p2 {
            // Rows are 32 high: aim at the middle, past the hit tolerance at its edge.
            at = Some(p + vec2(0.0, 12.0));
            u.app.session.execute("page.select", &json!({"id": p1})).unwrap();
            u.frame(vec![]);
            break;
        }
    }
    let page_row = at.expect("page 2 row");
    let r = u.row_at(rect);
    u.drag_to(r, page_row);
    u.button(page_row, false);
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.ancestors(rect).last().copied(), Some(p2), "moved to page 2");
}
