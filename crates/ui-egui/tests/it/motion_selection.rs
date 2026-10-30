//! Timeline selection rules: a layer click carries all its children, re-clicking toggles it off,
//! Shift extends, a plain click on a fresh item replaces the selection, blank clears only the
//! timeline selection (the canvas keeps its layers), and a clip click opens its settings popover.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{self, AnimProperty, ClipKind, ClipMode};
use figcraft_engine::Session;
use figcraft_ui_egui::motion::{SelItem, set_open};
use figcraft_ui_egui::{FigApp, NoServices};
use serde_json::json;

struct Frames {
    ctx: egui::Context,
    app: FigApp,
    time: f64,
}

impl Frames {
    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let input =
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(self.time), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| self.app.ui(ui));
        output.textures_delta.clear();
    }
}

/// A primary click at `pos` (Shift held when `shift`).
fn click(u: &mut Frames, pos: Pos2, shift: bool) {
    // egui only takes `input.modifiers` from a ModifiersChanged event, not from PointerButton.
    let mods = if shift { Modifiers { shift: true, ..Default::default() } } else { Modifiers::default() };
    u.frame(vec![Event::ModifiersChanged(mods), Event::PointerMoved(pos)]);
    u.frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: mods }]);
    u.frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: mods }]);
    u.frame(vec![]);
}

/// A document with one rectangle: an Opacity track keyed at 0 and 800 ms, and one Position clip.
/// Returns the harness, the node id and the clip id.
fn setup() -> (Frames, NodeId, u32) {
    let mut session = Session::new();
    session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap();
    let mut u = Frames { ctx: egui::Context::default(), app: FigApp::new(session, Box::new(NoServices)), time: 0.0 };
    for _ in 0..3 {
        u.frame(vec![]);
    }
    // The last node is the rectangle (the first is the page itself, which selection rejects).
    let id = u.app.session.doc().unwrap().nodes().max_by_key(|n| n.id.0).unwrap().id;
    u.app.session.set_selection(vec![id]);
    let cid = {
        let doc = u.app.session.doc_mut().unwrap();
        let n = doc.node_mut(id).unwrap();
        let v0 = motion::keyframed_value_at(n, AnimProperty::Opacity, 0.0).unwrap();
        n.motion.ensure_track(AnimProperty::Opacity).set_key(0, v0, None);
        let v1 = motion::keyframed_value_at(n, AnimProperty::Opacity, 800.0).unwrap();
        n.motion.ensure_track(AnimProperty::Opacity).set_key(800, v1, None);
        let mut c = motion::Clip::new(ClipKind::Position, ClipMode::In);
        c.id = n.motion.next_clip_id();
        c.delay_ms = 0;
        let cid = c.id;
        n.motion.clips.push(c);
        n.motion.normalize();
        cid
    };
    set_open(&mut u.app, true);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    (u, id, cid)
}

// The docked timeline spans the full window width, y 591..900 (309 pt tall, 47 pt header). Rows
// start at y = 646 with 24 pt rows: 0 the layer, 1 the clip, 2 the Opacity track. track_x = 297,
// scale = (1440 - 297 - 9.5 - 16) / 2000, so x_of(0) = 306.5, x_of(800) = 753.5.
const LAYER: Pos2 = Pos2::new(150.0, 658.0);
const CLIP_MID: Pos2 = Pos2::new(440.0, 682.0);
const TRACK_LEFT: Pos2 = Pos2::new(150.0, 706.0);
const TRACK_BLANK: Pos2 = Pos2::new(900.0, 706.0);
const SEG_MID: Pos2 = Pos2::new(500.0, 706.0);
const KEY0: Pos2 = Pos2::new(306.5, 706.0);

#[test]
fn layer_click_carries_children_and_reclick_toggles_off() {
    let (mut u, id, cid) = setup();
    click(&mut u, LAYER, false);
    let sel = &u.app.ui.motion.selection;
    assert!(sel.contains(&SelItem::Layer(id)), "layer in selection: {sel:?}");
    assert!(sel.contains(&SelItem::Track(id, AnimProperty::Opacity)), "track pulled in: {sel:?}");
    assert!(sel.contains(&SelItem::Clip(id, cid)), "clip pulled in: {sel:?}");
    assert_eq!(sel.len(), 3, "exactly the parent and its children: {sel:?}");
    // The caches follow so the inspector shows the pick.
    assert!(u.app.ui.motion.selected_track.is_some());
    assert_eq!(u.app.ui.motion.selected_track.unwrap().property, AnimProperty::Opacity);

    // Re-clicking the layer toggles the whole expansion off.
    click(&mut u, LAYER, false);
    assert!(u.app.ui.motion.selection.is_empty(), "re-click clears: {:?}", u.app.ui.motion.selection);
}

#[test]
fn shift_extends_and_toggles_single_items() {
    let (mut u, id, cid) = setup();
    click(&mut u, LAYER, false);
    // The track is already in the set (parent carried it), so Shift+click toggles just it off.
    click(&mut u, TRACK_LEFT, true);
    let sel = &u.app.ui.motion.selection;
    assert!(!sel.contains(&SelItem::Track(id, AnimProperty::Opacity)), "track toggled off: {sel:?}");
    assert!(sel.contains(&SelItem::Layer(id)) && sel.contains(&SelItem::Clip(id, cid)), "rest kept: {sel:?}");
    // Shift+click again adds it back without disturbing the others.
    click(&mut u, TRACK_LEFT, true);
    let sel = &u.app.ui.motion.selection;
    assert_eq!(sel.len(), 3, "track back in: {sel:?}");
    assert!(sel.contains(&SelItem::Track(id, AnimProperty::Opacity)));
}

#[test]
fn plain_click_replaces_selection() {
    let (mut u, id, cid) = setup();
    // A plain click with nothing selected makes the clicked item the whole selection.
    click(&mut u, CLIP_MID, false);
    let sel = &u.app.ui.motion.selection;
    assert_eq!(*sel, [SelItem::Clip(id, cid)].into_iter().collect(), "unique selection: {sel:?}");
    // Clicking another item replaces it again (the layer expands to its children).
    click(&mut u, LAYER, false);
    let sel = &u.app.ui.motion.selection;
    assert_eq!(sel.len(), 3, "layer replaces with its children: {sel:?}");
}

#[test]
fn blank_clears_timeline_only() {
    let (mut u, id, _cid) = setup();
    click(&mut u, LAYER, false);
    assert!(!u.app.ui.motion.selection.is_empty());
    click(&mut u, TRACK_BLANK, false);
    assert!(u.app.ui.motion.selection.is_empty(), "blank clears: {:?}", u.app.ui.motion.selection);
    // The canvas/document selection keeps the layer, so its rows stay visible.
    assert!(u.app.session.selection().contains(&id), "canvas selection kept");
}

#[test]
fn clip_click_opens_and_reclick_closes_popover() {
    let (mut u, id, cid) = setup();
    click(&mut u, CLIP_MID, false);
    assert_eq!(u.app.ui.motion.clip_popover, Some((id, cid)), "popover opened on select");
    click(&mut u, CLIP_MID, false);
    assert!(u.app.ui.motion.clip_popover.is_none(), "re-click closes the popover");
    assert!(u.app.ui.motion.selection.is_empty(), "toggled off");
}

#[test]
fn key_and_segment_clicks() {
    let (mut u, id, _cid) = setup();
    // The first keyframe.
    click(&mut u, KEY0, false);
    assert_eq!(u.app.ui.motion.selection, [SelItem::Key(id, AnimProperty::Opacity, 0)].into_iter().collect());
    assert!(!u.app.ui.motion.easing_target, "a key alone is not an easing target");
    // A segment click selects the segment and makes it the easing target.
    click(&mut u, SEG_MID, false);
    assert_eq!(u.app.ui.motion.selection, [SelItem::Segment(id, AnimProperty::Opacity, 0)].into_iter().collect());
    assert!(u.app.ui.motion.easing_target, "single segment is the easing target");
    // The track pick through its left column.
    click(&mut u, TRACK_LEFT, false);
    assert_eq!(u.app.ui.motion.selection, [SelItem::Track(id, AnimProperty::Opacity)].into_iter().collect());
    assert!(!u.app.ui.motion.easing_target);
}
