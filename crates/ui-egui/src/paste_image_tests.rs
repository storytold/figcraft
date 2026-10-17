//! Pasting an image copied in another app (#663). On the desktop, egui-winit swallows Cmd/Ctrl+V's
//! press and sends a paste event only when the clipboard holds text: an image leaves just the
//! key's release. The browser sends the press and a paste event (`web_clipboard_tests`).

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, Services};

/// What the system clipboard holds when a paste chord is pressed (#716).
#[derive(Clone, Copy, Debug)]
enum Held {
    /// FigCraft's own copy (its SVG text).
    Ours,
    /// Text copied in another app.
    Text,
    /// An image copied in another app.
    Image,
}

const HELD: [Held; 3] = [Held::Ours, Held::Text, Held::Image];

#[derive(Default)]
struct Clip {
    text: Option<String>,
    image: Option<(u32, u32, Vec<u8>)>,
}

/// The desktop clipboard.
struct Desktop(Rc<RefCell<Clip>>);

impl Services for Desktop {
    fn clipboard_set_text(&mut self, s: &str) {
        *self.0.borrow_mut() = Clip { text: Some(s.into()), image: None };
    }
    fn clipboard_get_text(&mut self) -> Option<String> {
        self.0.borrow().text.clone()
    }
    fn clipboard_get_image(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        self.0.borrow().image.clone()
    }
}

struct Win {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
    clip: Rc<RefCell<Clip>>,
    shapes: Vec<Shape>,
}

impl Win {
    fn new() -> Self {
        let mut s = Session::new();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
        let ctx = egui::Context::default();
        let clip = Rc::new(RefCell::new(Clip::default()));
        let app = FigApp::new(s, Box::new(Desktop(clip.clone())));
        let mut w = Self { ctx, app, time: 0.0, clip, shapes: Vec::new() };
        for _ in 0..3 {
            w.frame(vec![]);
        }
        w
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.frame_with(events, false);
    }

    /// `take_v`: something ahead of the app (a prototype's key trigger) consumes a V press.
    fn frame_with(&mut self, events: Vec<Event>, take_v: bool) {
        self.time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| {
            if take_v {
                ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::V));
            }
            app.ui(ui);
        });
        out.textures_delta.clear();
        self.shapes = out.shapes.into_iter().map(|s| s.shape).collect();
    }

    fn copy_image_elsewhere(&mut self) {
        *self.clip.borrow_mut() = Clip { text: None, image: Some((2, 2, vec![255; 16])) };
    }

    /// Cmd/Ctrl+V as egui-winit delivers it: the press brings a paste event when the clipboard
    /// holds text and nothing otherwise; the release comes as a key event.
    fn cmd_v(&mut self, m: Modifiers) {
        let text = self.clip.borrow().text.clone().filter(|t| !t.is_empty());
        self.frame(text.map(Event::Paste).into_iter().collect());
        self.frame(vec![key(Key::V, false, m)]);
        self.frame(vec![]);
    }

    /// A Cmd/Ctrl+V chord with its modifiers, as egui-winit delivers it: they go down first, the V
    /// press brings a paste event only for text, its release comes as a key event, then they're
    /// let go.
    fn chord(&mut self, m: Modifiers) {
        let text = self.clip.borrow().text.clone().filter(|t| !t.is_empty());
        self.frame(vec![Event::ModifiersChanged(m)]);
        self.frame(text.map(Event::Paste).into_iter().collect());
        self.frame(vec![key(Key::V, false, m)]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    /// A red rectangle A copied in FigCraft, and a grey rectangle B selected; then `held` on the
    /// system clipboard. Returns (A, B).
    fn copy_a_select_b(&mut self, held: Held) -> (NodeId, NodeId) {
        let s = &mut self.app.session;
        let a = s.doc().unwrap().nodes().find(|n| n.name == "Rectangle").unwrap().id;
        s.execute("fill.set", &json!({"ids": [a.0], "color": "#e53935"})).unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": 400, "y": 250, "width": 100, "height": 50})).unwrap();
        let b = NodeId(b["id"].as_u64().unwrap());
        s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        crate::menus::invoke(&mut self.app, "edit.copy");
        self.app.session.execute("edit.copyProperties", &json!({})).unwrap();
        self.app.session.execute("select.set", &json!({"ids": [b.0]})).unwrap();
        match held {
            Held::Ours => {}
            Held::Text => self.clip.borrow_mut().text = Some("hello from another app".into()),
            Held::Image => self.copy_image_elsewhere(),
        }
        (a, b)
    }

    fn fill(&self, id: NodeId) -> String {
        let n = figcraft_engine::inspect::node_json(self.app.session.doc().unwrap(), id, 0);
        n["fills"][0]["color"].as_str().unwrap_or_default().to_string()
    }

    /// Top-lefts of the rectangles other than `b`.
    fn rects_besides(&self, b: NodeId) -> Vec<(i64, i64)> {
        let doc = self.app.session.doc().unwrap();
        let mut at: Vec<_> = doc
            .nodes()
            .filter(|n| n.name == "Rectangle" && n.id != b)
            .filter_map(|n| doc.world_bounds(n.id))
            .map(|r| (r.x0.round() as i64, r.y0.round() as i64))
            .collect();
        at.sort();
        at
    }

    fn named(&self, name: &str) -> usize {
        self.app.session.doc().unwrap().nodes().filter(|n| n.name == name).count()
    }

    fn texts(&self) -> usize {
        self.app.session.doc().unwrap().nodes().filter(|n| matches!(n.kind, figcraft_doc::NodeKind::Text(_))).count()
    }

    fn rects(&self) -> usize {
        self.named("Rectangle")
    }
}

// egui sets `command` for Cmd on macOS and for Ctrl elsewhere; macOS Ctrl is a separate key.
const CMD: Modifiers = if cfg!(target_os = "macos") { Modifiers::COMMAND } else { Modifiers::COMMAND.plus(Modifiers::CTRL) };

fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
}

#[test]
fn cmd_v_pastes_an_image_copied_elsewhere() {
    let mut w = Win::new();
    w.copy_image_elsewhere();
    w.cmd_v(CMD);
    assert_eq!(w.named("Pasted image"), 1, "the image was pasted from the key's release");
    w.cmd_v(CMD);
    assert_eq!(w.named("Pasted image"), 2, "and again");
}

#[test]
fn cmd_released_before_v_still_pastes_the_image() {
    let mut w = Win::new();
    w.copy_image_elsewhere();
    w.cmd_v(Modifiers::NONE);
    assert_eq!(w.named("Pasted image"), 1);
}

#[test]
fn text_pastes_once_from_its_paste_event() {
    let mut w = Win::new();
    *w.clip.borrow_mut() = Clip { text: Some("hello from another app".into()), image: None };
    w.cmd_v(CMD);
    assert_eq!(w.texts(), 1, "the paste event pasted the text, the release nothing more");
    assert_eq!(w.rects(), 1);
}

#[test]
fn text_pastes_once_when_another_key_is_let_go_first() {
    let mut w = Win::new();
    *w.clip.borrow_mut() = Clip { text: Some("hello".into()), image: None };
    // Cmd+A then Cmd+V typed fast: A comes up after V went down.
    w.frame(vec![key(Key::A, true, CMD)]);
    w.frame(vec![Event::Paste("hello".into())]);
    w.frame(vec![key(Key::A, false, CMD)]);
    w.frame(vec![key(Key::V, false, CMD)]);
    w.frame(vec![]);
    assert_eq!(w.texts(), 1);
    assert_eq!(w.rects(), 1);
}

#[test]
fn a_v_press_consumed_elsewhere_does_not_paste_on_release() {
    let mut w = Win::new();
    w.copy_image_elsewhere();
    w.frame_with(vec![key(Key::V, true, Modifiers::NONE)], true);
    w.frame(vec![key(Key::V, false, Modifiers::NONE)]);
    w.frame(vec![]);
    assert_eq!(w.named("Pasted image"), 0);
}

#[test]
fn the_browser_sequence_pastes_once() {
    let mut w = Win::new();
    w.copy_image_elsewhere();
    w.frame(vec![key(Key::V, true, CMD)]);
    w.frame(vec![key(Key::V, false, CMD)]);
    w.frame(vec![]);
    assert_eq!(w.named("Pasted image"), 1);
}

#[test]
fn v_alone_pastes_nothing() {
    let mut w = Win::new();
    w.copy_image_elsewhere();
    w.frame(vec![key(Key::V, true, Modifiers::NONE)]);
    w.frame(vec![key(Key::V, false, Modifiers::NONE)]);
    assert_eq!(w.named("Pasted image"), 0);
}

#[test]
fn a_paste_event_from_another_key_does_not_block_the_next_cmd_v() {
    let mut w = Win::new();
    *w.clip.borrow_mut() = Clip { text: Some("text".into()), image: None };
    // Shift+Insert on Windows.
    w.frame(vec![Event::Paste("text".into())]);
    w.frame(vec![key(Key::Insert, false, Modifiers::SHIFT)]);
    w.copy_image_elsewhere();
    w.cmd_v(CMD);
    assert_eq!(w.named("Pasted image"), 1);
}

#[test]
fn a_v_press_lost_to_another_window_does_not_block_the_next_cmd_v() {
    let mut w = Win::new();
    w.copy_image_elsewhere();
    w.frame(vec![key(Key::V, true, Modifiers::NONE), Event::WindowFocused(false)]);
    w.frame(vec![Event::WindowFocused(true)]);
    w.cmd_v(CMD);
    assert_eq!(w.named("Pasted image"), 1);
}

#[test]
fn v_held_while_the_window_was_away_pastes_nothing_on_release() {
    let mut w = Win::new();
    w.copy_image_elsewhere();
    // Pressed in another window: only the focus and the release arrive here.
    w.frame(vec![Event::WindowFocused(true)]);
    w.frame(vec![key(Key::V, false, Modifiers::NONE)]);
    w.frame(vec![]);
    assert_eq!(w.named("Pasted image"), 0);
    w.cmd_v(CMD);
    assert_eq!(w.named("Pasted image"), 1, "the next Cmd+V still pastes");
}

#[test]
fn cmd_v_in_a_panel_field_does_not_paste_on_the_canvas() {
    let mut w = Win::new();
    w.copy_image_elsewhere();
    w.frame(vec![]);
    w.app.field_focus = true;
    w.frame(vec![key(Key::V, false, CMD)]);
    w.frame(vec![]);
    assert_eq!(w.named("Pasted image"), 0);
}

#[test]
fn paste_here_prefers_a_newer_image_over_copied_layers() {
    let mut w = Win::new();
    w.app.session.execute("select.all", &json!({})).unwrap();
    crate::menus::invoke(&mut w.app, "edit.copy");
    assert!(!w.app.session.clipboard.nodes.is_empty(), "layers copied");
    w.app.session.execute("select.none", &json!({})).unwrap();
    w.copy_image_elsewhere();
    let at = Pos2::new(900.0, 600.0);
    for pressed in [true, false] {
        w.frame(vec![
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE },
        ]);
    }
    w.frame(vec![]);
    let item = crate::live_tests::text_center(&w.shapes, "Paste here");
    for pressed in [true, false] {
        w.frame(vec![
            Event::PointerMoved(item),
            Event::PointerButton { pos: item, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE },
        ]);
    }
    w.frame(vec![]);
    assert_eq!(w.named("Pasted image"), 1, "the newer image was pasted");
    assert_eq!(w.rects(), 1, "not the copied rectangle");
    let p = w.app.to_page(at);
    let doc = w.app.session.doc().unwrap();
    let img = doc.nodes().find(|n| n.name == "Pasted image").unwrap();
    let b = doc.world_bounds(img.id).unwrap();
    assert!((b.x0 - p.x).abs() < 0.01 && (b.y0 - p.y).abs() < 0.01, "top-left where the menu opened: {b:?} vs {p:?}");
}

#[test]
fn shift_cmd_v_pastes_over_the_selection_whatever_the_clipboard_holds() {
    for held in HELD {
        let mut w = Win::new();
        let (_, b) = w.copy_a_select_b(held);
        w.chord(CMD.plus(Modifiers::SHIFT));
        assert_eq!(w.rects_besides(b), [(10, 10), (400, 250)], "{held:?}: A's copy at B's top-left, nothing else");
        assert_eq!(w.texts(), 0, "{held:?}: no text pasted");
        assert_eq!(w.named("Pasted image"), 0, "{held:?}: no image pasted");
    }
}

#[test]
fn alt_cmd_v_pastes_properties_whatever_the_clipboard_holds() {
    for held in HELD {
        let mut w = Win::new();
        let (_, b) = w.copy_a_select_b(held);
        w.chord(CMD.plus(Modifiers::ALT));
        assert_eq!(w.fill(b), "#E53935", "{held:?}: B took A's fill");
        assert_eq!(w.rects_besides(b), [(10, 10)], "{held:?}: no layer pasted");
        assert_eq!(w.texts(), 0, "{held:?}: no text pasted");
        assert_eq!(w.named("Pasted image"), 0, "{held:?}: no image pasted");
    }
}

#[test]
fn a_chord_let_go_within_one_frame_still_reads_its_modifiers() {
    let mut w = Win::new();
    let (_, b) = w.copy_a_select_b(Held::Text);
    let m = CMD.plus(Modifiers::SHIFT);
    w.frame(vec![
        Event::ModifiersChanged(m),
        Event::Paste("hello from another app".into()),
        key(Key::V, false, m),
        Event::ModifiersChanged(Modifiers::NONE),
    ]);
    w.frame(vec![]);
    assert_eq!(w.rects_besides(b), [(10, 10), (400, 250)]);
    assert_eq!(w.texts(), 0);
}

#[test]
fn cmd_let_go_before_v_still_pastes_over() {
    let mut w = Win::new();
    let (_, b) = w.copy_a_select_b(Held::Image);
    w.frame(vec![Event::ModifiersChanged(CMD.plus(Modifiers::SHIFT))]);
    w.frame(vec![Event::ModifiersChanged(Modifiers::SHIFT)]);
    w.frame(vec![key(Key::V, false, Modifiers::SHIFT)]);
    w.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(w.rects_besides(b), [(10, 10), (400, 250)], "pasted over, not flipped or a plain paste");
    assert_eq!(w.named("Pasted image"), 0);
}

#[test]
fn a_held_chord_pastes_over_once() {
    let mut w = Win::new();
    let (_, b) = w.copy_a_select_b(Held::Text);
    let m = CMD.plus(Modifiers::SHIFT);
    w.frame(vec![Event::ModifiersChanged(m)]);
    // Auto-repeat: egui-winit sends a paste event for every repeated press.
    for _ in 0..3 {
        w.frame(vec![Event::Paste("hello from another app".into())]);
    }
    w.frame(vec![key(Key::V, false, m)]);
    w.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(w.rects_besides(b), [(10, 10), (400, 250)]);
    assert_eq!(w.texts(), 0);
}

#[test]
fn shift_insert_still_pastes_text() {
    let mut w = Win::new();
    let (_, b) = w.copy_a_select_b(Held::Text);
    w.frame(vec![Event::ModifiersChanged(Modifiers::SHIFT), Event::Paste("hello from another app".into())]);
    w.frame(vec![key(Key::Insert, false, Modifiers::SHIFT), Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(w.texts(), 1);
    assert_eq!(w.rects_besides(b), [(10, 10)]);
}

#[test]
fn cmd_v_after_a_paste_chord_is_a_plain_paste_again() {
    let mut w = Win::new();
    let (_, b) = w.copy_a_select_b(Held::Text);
    w.chord(CMD.plus(Modifiers::SHIFT));
    w.chord(CMD);
    assert_eq!(w.texts(), 1, "the text was pasted by the plain Cmd+V");
    assert_eq!(w.rects_besides(b), [(10, 10), (400, 250)], "Shift+Cmd+V pasted over once");
}
