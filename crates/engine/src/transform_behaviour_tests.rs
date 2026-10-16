//! Transform behaviour suite, written from the public documentation of design tools: resizing
//! (Shift, Alt, both, flipping past an edge), rotating (Shift in 15° steps), groups and several
//! layers with different rotations or flips, frames and constraints, lines, text, nudging, the
//! pixel grid, smart guides and the transform box itself. Each rule is a table row or an assert
//! with a one-line derivation; where the documentation is silent the row says so and pins
//! today's behaviour.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{Cursor, Handle, PointerEvent, PointerKind, handle_at, handle_cursor, resized_box, selection_box};
use figcraft_doc::{NodeId, TextAutoResize};
use figcraft_geom::{Point, Rect, is_flipped, rotation_degrees};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&ex(s, "shape.rectangle", p))
}

fn set(s: &mut Session, ids: &[NodeId], props: Value) {
    ex(s, "node.set", json!({"ids": ids, "props": props}));
}

/// A session with snapping to layers off (the rows that test snapping turn it back on).
fn session() -> Session {
    let mut s = Session::new();
    ex(&mut s, "view.snapObjects", json!({"on": false}));
    s
}

#[derive(Clone, Copy, Default)]
struct Mods {
    shift: bool,
    alt: bool,
    cmd: bool,
}

fn ev(kind: PointerKind, p: Point, m: Mods) -> PointerEvent {
    let mut e = PointerEvent::at(kind, p.x, p.y, 1.0);
    (e.shift, e.alt, e.cmd) = (m.shift, m.alt, m.cmd);
    e
}

/// A pointer drag from `a` to `b` (one move), as one gesture.
fn drag(s: &mut Session, a: Point, b: Point, m: Mods) {
    s.pointer(&ev(PointerKind::Down, a, m)).unwrap();
    s.pointer(&ev(PointerKind::Move, b, m)).unwrap();
    s.pointer(&ev(PointerKind::Up, b, m)).unwrap();
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().world_bounds(id).unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn near_rect(a: Rect, b: Rect) -> bool {
    near(a.x0, b.x0) && near(a.y0, b.y0) && near(a.x1, b.x1) && near(a.y1, b.y1)
}

fn size(s: &Session, id: NodeId) -> (f64, f64) {
    let n = s.doc().unwrap().node(id).unwrap();
    (n.width, n.height)
}

fn rotation(s: &Session, id: NodeId) -> f64 {
    rotation_degrees(s.doc().unwrap().node(id).unwrap().transform)
}

fn flipped(s: &Session, id: NodeId) -> bool {
    is_flipped(s.doc().unwrap().world_transform(id))
}

fn undo_steps(s: &Session) -> usize {
    s.ds().unwrap().history.undo.len()
}

const NONE: Mods = Mods { shift: false, alt: false, cmd: false };
const SHIFT: Mods = Mods { shift: true, alt: false, cmd: false };
const ALT: Mods = Mods { shift: false, alt: true, cmd: false };
const BOTH: Mods = Mods { shift: true, alt: true, cmd: false };
const CMD: Mods = Mods { shift: false, alt: false, cmd: true };

#[test]
fn handle_drags_resize_by_the_documented_rules() {
    // A 100×50 box; (handle, pointer in the box's axes, modifiers, new box x0 y0 x1 y1, derivation).
    let rows: [(Handle, (f64, f64), Mods, (f64, f64, f64, f64), &str); 11] = [
        (Handle::Corner(2), (150.0, 80.0), NONE, (0.0, 0.0, 150.0, 80.0), "a corner moves the two edges it joins"),
        (Handle::Edge(1), (150.0, 999.0), NONE, (0.0, 0.0, 150.0, 50.0), "an edge moves only its side"),
        (Handle::Corner(2), (150.0, 60.0), SHIFT, (0.0, 0.0, 150.0, 75.0), "Shift keeps the proportions (following the axis dragged the most)"),
        (Handle::Edge(1), (150.0, 0.0), SHIFT, (0.0, -12.5, 150.0, 62.5), "Shift on an edge scales the other axis about its middle"),
        (Handle::Corner(2), (150.0, 80.0), ALT, (-50.0, -30.0, 150.0, 80.0), "Alt resizes from the centre"),
        (Handle::Corner(2), (75.0, 40.0), ALT, (25.0, 10.0, 75.0, 40.0), "Alt shrinks about the centre too"),
        (Handle::Edge(3), (10.0, 0.0), ALT, (10.0, 0.0, 90.0, 50.0), "Alt on an edge moves the opposite edge the other way"),
        (Handle::Corner(2), (150.0, 60.0), BOTH, (-50.0, -25.0, 150.0, 75.0), "Shift+Alt: proportional, from the centre"),
        (Handle::Edge(1), (-40.0, 0.0), NONE, (0.0, 0.0, -40.0, 50.0), "dragging an edge past the opposite one flips (reversed box)"),
        (Handle::Corner(0), (110.0, 60.0), NONE, (110.0, 60.0, 100.0, 50.0), "a corner dragged past the opposite one flips both ways"),
        (Handle::Corner(2), (-50.0, -25.0), SHIFT, (0.0, 0.0, -50.0, -25.0), "a proportional flip keeps the proportions"),
    ];
    for (h, q, m, want, why) in rows {
        let got = resized_box(h, Point::new(q.0, q.1), 100.0, 50.0, m.shift, m.alt);
        assert!(near(got.0, want.0) && near(got.1, want.1) && near(got.2, want.2) && near(got.3, want.3), "{why}: {got:?}");
    }
}

#[test]
fn resizing_and_flipping_one_layer() {
    let mut s = session();
    // A rotated layer resizes along its own axes, keeping its rotation and the opposite corner.
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[r], json!({"rotation": 30}));
    ex(&mut s, "select.set", json!({"ids": [r]}));
    let world = s.doc().unwrap().world_transform(r);
    let steps = undo_steps(&s);
    drag(&mut s, world * Point::new(100.0, 50.0), world * Point::new(150.0, 80.0), NONE);
    assert!(near(size(&s, r).0, 150.0) && near(size(&s, r).1, 80.0) && near(rotation(&s, r), 30.0), "rotated layers resize in their own axes");
    assert!((s.doc().unwrap().world_transform(r) * Point::ZERO - world * Point::ZERO).hypot() < 1e-6, "the opposite corner stays put");
    assert_eq!(undo_steps(&s), steps + 1, "one undo step per gesture");
    // Dragging the right edge past the left one flips the layer.
    let f = rect(&mut s, 300.0, 0.0, 100.0, 50.0, None);
    ex(&mut s, "select.set", json!({"ids": [f]}));
    drag(&mut s, Point::new(400.0, 25.0), Point::new(260.0, 25.0), NONE);
    assert!(flipped(&s, f) && near_rect(bounds(&s, f), Rect::new(260.0, 0.0, 300.0, 50.0)), "flipped, 40 wide, left of where it was");
    // Rotation handles: just outside a corner; Shift turns in 15° steps.
    let t = rect(&mut s, 0.0, 200.0, 100.0, 100.0, None);
    ex(&mut s, "select.set", json!({"ids": [t]}));
    let (c, from) = (Point::new(50.0, 250.0), Point::new(110.0, 190.0));
    let a = (from - c).atan2() + 37f64.to_radians();
    let to = c + figcraft_geom::Vec2::from_angle(a) * 80.0;
    drag(&mut s, from, to, NONE);
    assert!(near(rotation(&s, t).abs(), 37.0), "a free turn: {}", rotation(&s, t));
    ex(&mut s, "edit.undo", json!({}));
    drag(&mut s, from, to, SHIFT);
    assert!(near(rotation(&s, t).abs(), 30.0), "Shift: the nearest 15° step: {}", rotation(&s, t));
}

#[test]
fn groups_scale_children_keeping_their_rotation_and_flip() {
    let mut s = session();
    // A group of a 100×20 bar turned 90° (40..60 × -40..60) and a 50×50 square at (100, 0), its
    // box 40..150 × -40..60. Stretching it to twice its width (right edge to 260):
    let bar = rect(&mut s, 0.0, 0.0, 100.0, 20.0, None);
    set(&mut s, &[bar], json!({"rotation": 90}));
    let sq = rect(&mut s, 100.0, 0.0, 50.0, 50.0, None);
    ex(&mut s, "object.flipH", json!({"ids": [sq]}));
    let g = id(&ex(&mut s, "object.group", json!({"ids": [bar, sq]})));
    assert!(near_rect(bounds(&s, g), Rect::new(40.0, -40.0, 150.0, 60.0)));
    ex(&mut s, "select.set", json!({"ids": [g]}));
    drag(&mut s, Point::new(150.0, 10.0), Point::new(260.0, 10.0), NONE);
    assert!(near_rect(bounds(&s, g), Rect::new(40.0, -40.0, 260.0, 60.0)), "the group is twice as wide, as tall as before: {:?}", bounds(&s, g));
    // Each child scales with the group across the page: the bar's own height (now horizontal)
    // doubles and its length stays; both keep their rotation and flip.
    assert!(near(size(&s, bar).0, 100.0) && near(size(&s, bar).1, 40.0) && near(rotation(&s, bar), 90.0), "{:?}", size(&s, bar));
    assert!(near_rect(bounds(&s, bar), Rect::new(40.0, -40.0, 80.0, 60.0)), "{:?}", bounds(&s, bar));
    assert!(flipped(&s, sq) && near_rect(bounds(&s, sq), Rect::new(160.0, 0.0, 260.0, 50.0)), "{:?}", bounds(&s, sq));
}

#[test]
fn nested_turned_groups_and_constraints_scale_along_each_layers_axes() {
    let mut s = session();
    // A group turned 90° holding a 100×20 bar, inside an outer group with a square: stretching the
    // outer group twice as wide makes the bar's page width (its own height) twice as large.
    let bar = rect(&mut s, 0.0, 0.0, 100.0, 20.0, None);
    let inner = id(&ex(&mut s, "object.group", json!({"ids": [bar]})));
    set(&mut s, &[inner], json!({"rotation": 90}));
    let sq = rect(&mut s, 100.0, 0.0, 50.0, 50.0, None);
    let outer = id(&ex(&mut s, "object.group", json!({"ids": [inner, sq]})));
    let (b0, o0) = (bounds(&s, bar), bounds(&s, outer));
    set(&mut s, &[outer], json!({"width": o0.width() * 2.0}));
    let b1 = bounds(&s, bar);
    assert!(near(b1.width(), b0.width() * 2.0) && near(b1.height(), b0.height()), "{b0:?} → {b1:?}");
    assert!(near(size(&s, bar).0, 100.0) && near(size(&s, bar).1, 40.0));
    // A frame's child turned 90° with Scale constraints: the frame twice as wide makes the child's
    // page width (its own height) twice as large, not its length.
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 200, "height": 200})));
    let c = rect(&mut s, 50.0, 50.0, 100.0, 20.0, Some(f));
    set(&mut s, &[c], json!({"rotation": 90, "constraints": {"horizontal": "scale", "vertical": "scale"}}));
    let before = bounds(&s, c);
    set(&mut s, &[f], json!({"width": 400}));
    let after = bounds(&s, c);
    assert!(near(after.width(), before.width() * 2.0) && near(after.height(), before.height()), "{before:?} → {after:?}");
    assert!(near(after.x0, before.x0 * 2.0) && near(rotation(&s, c), 90.0));
}

#[test]
fn several_layers_with_different_rotations_resize_together() {
    let mut s = session();
    // P upright (0..100 × 0..50); Q 100×50 turned 90° about its centre (225..275 × -25..75): a
    // page-aligned box 0..275 × -25..75.
    let p = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    let q = rect(&mut s, 200.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[q], json!({"rotation": 90}));
    ex(&mut s, "select.set", json!({"ids": [p, q]}));
    let b = selection_box(s.doc().unwrap(), &[p, q]).unwrap();
    assert_eq!(b[0], Point::new(0.0, -25.0));
    assert_eq!(b[2], Point::new(275.0, 75.0));
    // Twice as wide from the right edge: everything scales across the page, each layer along its
    // own axes, keeping its rotation.
    let steps = undo_steps(&s);
    drag(&mut s, Point::new(275.0, 25.0), Point::new(550.0, 25.0), NONE);
    assert!(near_rect(bounds(&s, p), Rect::new(0.0, 0.0, 200.0, 50.0)), "{:?}", bounds(&s, p));
    assert!(near_rect(bounds(&s, q), Rect::new(450.0, -25.0, 550.0, 75.0)), "{:?}", bounds(&s, q));
    assert!(near(size(&s, q).0, 100.0) && near(size(&s, q).1, 100.0) && near(rotation(&s, q), 90.0), "{:?}", size(&s, q));
    assert_eq!(undo_steps(&s), steps + 1);
    ex(&mut s, "edit.undo", json!({}));
    // Dragged past the opposite edge, the arrangement mirrors and each layer flips.
    drag(&mut s, Point::new(275.0, 25.0), Point::new(-275.0, 25.0), NONE);
    assert!(near_rect(bounds(&s, p), Rect::new(-100.0, 0.0, 0.0, 50.0)), "{:?}", bounds(&s, p));
    assert!(near_rect(bounds(&s, q), Rect::new(-275.0, -25.0, -225.0, 75.0)), "{:?}", bounds(&s, q));
    assert!(flipped(&s, p) && flipped(&s, q));
}

#[test]
fn design_panel_edits_reach_every_selected_layer() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    let b = rect(&mut s, 200.0, 0.0, 60.0, 30.0, None);
    set(&mut s, &[b], json!({"rotation": 20}));
    // Typing W, H, X or rotation into a mixed field sets it on each layer (one undo step).
    let steps = undo_steps(&s);
    set(&mut s, &[a, b], json!({"width": 80, "rotation": 45, "x": 10}));
    assert_eq!(undo_steps(&s), steps + 1);
    for i in [a, b] {
        assert!(near(size(&s, i).0, 80.0) && near(rotation(&s, i), 45.0), "{i:?}");
        assert!(near(s.doc().unwrap().node(i).unwrap().transform.as_coeffs()[4], 10.0));
    }
    // A flipped layer reads back the rotation typed and stays flipped (a horizontal flip reads
    // as 180°, the form the rotation field shows).
    ex(&mut s, "object.flipH", json!({"ids": [a]}));
    set(&mut s, &[a], json!({"rotation": 30}));
    assert!(near(rotation(&s, a), 30.0) && flipped(&s, a), "{}", rotation(&s, a));
}

#[test]
fn constrain_proportions_keeps_the_other_side_in_ratio() {
    let mut s = session();
    // The Layout panel's link is a per-layer property: typing W or H scales the other side by the
    // node's own ratio; both sides in one edit stay as typed, and so does an unlinked edit.
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[r], json!({"constrainProportions": true}));
    set(&mut s, &[r], json!({"width": 200}));
    assert_eq!(size(&s, r), (200.0, 100.0), "W ×2 makes H ×2");
    set(&mut s, &[r], json!({"height": 25}));
    assert_eq!(size(&s, r), (50.0, 25.0), "H 25 makes W follow the ratio");
    set(&mut s, &[r], json!({"width": 30, "height": 40}));
    assert_eq!(size(&s, r), (30.0, 40.0), "both sides in one edit stay as typed");
    set(&mut s, &[r], json!({"constrainProportions": false}));
    set(&mut s, &[r], json!({"width": 60}));
    assert_eq!(size(&s, r), (60.0, 40.0), "unlinked: the other side stays put");
    // The short aliases work the same, and each layer of a selection follows its own ratio.
    let a = rect(&mut s, 0.0, 0.0, 20.0, 10.0, None);
    let b = rect(&mut s, 100.0, 0.0, 30.0, 10.0, None);
    set(&mut s, &[a, b], json!({"constrainProportions": true}));
    set(&mut s, &[a, b], json!({"h": 20}));
    assert_eq!((size(&s, a), size(&s, b)), ((40.0, 20.0), (60.0, 20.0)), "each layer keeps its own ratio");
    // A zero side has no ratio to keep: the edit applies to the side typed only.
    let z = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    set(&mut s, &[z], json!({"constrainProportions": true, "height": 0}));
    set(&mut s, &[z], json!({"width": 20}));
    assert_eq!(size(&s, z), (20.0, 0.0), "a zero height leaves nothing to scale");
}

#[test]
fn a_press_anywhere_on_a_corner_handle_resizes() {
    // The canvas draws each corner handle as an 8 px square centred on the corner, and the
    // corner-radius handle sits 10 px in along the diagonal: a press on the square's inner half
    // resizes (both sides, in ratio with the link on), it doesn't pick the radius handle.
    let radii = |s: &Session, r: NodeId| s.doc().unwrap().node(r).unwrap().corners.radii;
    for lock in [false, true] {
        let mut s = session();
        let r = rect(&mut s, 0.0, 0.0, 1000.0, 600.0, None);
        set(&mut s, &[r], json!({"constrainProportions": lock}));
        ex(&mut s, "select.set", json!({"ids": [r]}));
        let on_square = Point::new(997.0, 597.0);
        s.pointer(&ev(PointerKind::Hover, on_square, NONE)).unwrap();
        assert!(matches!(s.tool.overlay.cursor, Cursor::Resize(_)), "lock {lock}: {:?}", s.tool.overlay.cursor);
        drag(&mut s, on_square, Point::new(1197.0, 637.0), NONE);
        let (w, h) = size(&s, r);
        let want_h = if lock { 718.2 } else { 637.0 };
        assert!(near(w, 1197.0) && near(h, want_h), "lock {lock}: both sides follow the corner, got {w}×{h}");
        assert_eq!(radii(&s, r), [0.0; 4]);
    }
    // Further in, past the square, the radius handle still answers.
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 1000.0, 600.0, None);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    drag(&mut s, Point::new(993.0, 593.0), Point::new(980.0, 580.0), NONE);
    assert_eq!(size(&s, r), (1000.0, 600.0));
    assert!(radii(&s, r)[0] > 0.0, "the radius handle rounds the corners");
}

#[test]
fn frames_resize_with_constraints_and_cmd_ignores_them() {
    let mut s = session();
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100})));
    let c = rect(&mut s, 150.0, 10.0, 40.0, 40.0, Some(f));
    set(&mut s, &[c], json!({"constraints": {"horizontal": "right"}}));
    ex(&mut s, "select.set", json!({"ids": [f]}));
    drag(&mut s, Point::new(200.0, 50.0), Point::new(300.0, 50.0), NONE);
    assert_eq!(bounds(&s, c).x0, 250.0, "a Right constraint keeps the distance to the right edge");
    ex(&mut s, "edit.undo", json!({}));
    drag(&mut s, Point::new(200.0, 50.0), Point::new(300.0, 50.0), CMD);
    assert_eq!(bounds(&s, c).x0, 150.0, "Cmd/Ctrl while resizing a frame ignores constraints");
    assert_eq!(bounds(&s, f).width(), 300.0);
    // From the left edge too: the children stay where they are on the page.
    drag(&mut s, Point::new(0.0, 50.0), Point::new(-50.0, 50.0), CMD);
    assert_eq!((bounds(&s, f).x0, bounds(&s, c).x0), (-50.0, 150.0));
}

#[test]
fn lines_and_text() {
    let mut s = session();
    // A line's ends set its length and angle; Shift keeps the angle to 45° steps.
    let l = id(&ex(&mut s, "shape.line", json!({"x": 0, "y": 0, "width": 100})));
    ex(&mut s, "select.set", json!({"ids": [l]}));
    drag(&mut s, Point::new(100.0, 0.0), Point::new(60.0, 80.0), NONE);
    assert!(near(size(&s, l).0, 100.0) && near(rotation(&s, l).abs(), 53.13010235415598), "{} {}", size(&s, l).0, rotation(&s, l));
    ex(&mut s, "edit.undo", json!({}));
    drag(&mut s, Point::new(100.0, 0.0), Point::new(60.0, 80.0), SHIFT);
    assert!(near(rotation(&s, l).abs(), 45.0), "{}", rotation(&s, l));
    // Text: a side handle makes auto-width text fixed width (auto height); another handle a fixed
    // box.
    let t = id(&ex(&mut s, "text.create", json!({"text": "Hello world", "x": 0, "y": 200})));
    let auto = |s: &Session| s.doc().unwrap().node(t).unwrap().kind.text().map(|d| d.auto_resize);
    // Large type, so the box is big enough for side handles whatever font the machine falls back to.
    set(&mut s, &[t], json!({"fontSize": 48}));
    assert_eq!(auto(&s), Some(TextAutoResize::WidthAndHeight));
    ex(&mut s, "select.set", json!({"ids": [t]}));
    let b = bounds(&s, t);
    // Text sizes need a font (CI may have none).
    if b.height() > 4.0 {
        drag(&mut s, Point::new(b.x1 + 2.0, b.center().y), Point::new(b.x1 + 30.0, b.center().y), NONE);
        assert_eq!(auto(&s), Some(TextAutoResize::Height), "a side handle: auto height");
        let b = bounds(&s, t);
        drag(&mut s, Point::new(b.x1 + 2.0, b.y1 + 2.0), Point::new(b.x1 + 10.0, b.y1 + 20.0), NONE);
        assert_eq!(auto(&s), Some(TextAutoResize::None), "a corner: fixed size");
    }
}

#[test]
fn nudging_and_the_pixel_grid() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0, None);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    // Arrows move by 1, Shift+arrows by the big nudge (10, a preference).
    ex(&mut s, "transform.nudge", json!({"dx": 1, "dy": 0}));
    ex(&mut s, "transform.nudge", json!({"dx": 0, "dy": 1, "big": true}));
    assert_eq!(bounds(&s, r).origin(), Point::new(1.0, 10.0));
    ex(&mut s, "pref.nudge", json!({"big": 8}));
    ex(&mut s, "transform.nudge", json!({"dx": -1, "dy": 0, "big": true}));
    assert_eq!(bounds(&s, r).origin(), Point::new(-7.0, 10.0));
    // With the pixel grid on, moves, resizes and new shapes land on whole pixels.
    drag(&mut s, Point::new(20.0, 20.0), Point::new(30.3, 20.6), NONE);
    assert_eq!(bounds(&s, r).origin(), Point::new(3.0, 11.0), "moved by a whole pixel");
    drag(&mut s, Point::new(103.0, 61.0), Point::new(150.4, 80.6), NONE);
    assert_eq!(size(&s, r), (147.0, 70.0), "resized to whole pixels");
    // Off, they follow the pointer exactly.
    ex(&mut s, "view.snapPixel", json!({"on": false}));
    drag(&mut s, Point::new(150.0, 81.0), Point::new(160.5, 81.25), NONE);
    assert_eq!(size(&s, r), (157.5, 70.25));
}

#[test]
fn smart_guides_snap_to_edges_centres_and_equal_spacing() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, None);
    let c = rect(&mut s, 400.0, 0.0, 100.0, 100.0, None);
    let b = rect(&mut s, 200.0, 300.0, 100.0, 100.0, None);
    let _ = (a, c);
    ex(&mut s, "select.set", json!({"ids": [b]}));
    // (drag to, B's box after, derivation): B is grabbed at its centre (250, 350).
    let rows: [((f64, f64), (f64, f64), &str); 4] = [
        ((153.0, 350.0), (100.0, 300.0), "silent: an edge within 5 screen px of another layer's edge snaps to it (pinned)"),
        ((302.0, 52.0), (252.0, 0.0), "edges and centres snap (its top to the others' tops)"),
        ((248.0, 52.0), (200.0, 0.0), "between two layers, it snaps to equal gaps on both sides"),
        ((852.0, 52.0), (800.0, 0.0), "beyond the last of a row, to the gap the row already has"),
    ];
    for (to, want, why) in rows {
        drag(&mut s, Point::new(250.0, 350.0), Point::new(to.0, to.1), NONE);
        assert_eq!(bounds(&s, b).origin(), Point::new(want.0, want.1), "{why}");
        ex(&mut s, "edit.undo", json!({}));
    }
    // While dragging, the equal gaps show as labelled lines; Cmd/Ctrl turns snapping off.
    s.pointer(&ev(PointerKind::Down, Point::new(250.0, 350.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, Point::new(248.0, 52.0), NONE)).unwrap();
    let labels: Vec<String> = s.tool.overlay.guides.iter().filter_map(|g| g.label.clone()).collect();
    assert_eq!(labels, vec!["100", "100"], "{:?}", s.tool.overlay.guides);
    s.pointer(&ev(PointerKind::Move, Point::new(248.0, 52.0), CMD)).unwrap();
    s.pointer(&ev(PointerKind::Up, Point::new(248.0, 52.0), CMD)).unwrap();
    assert_eq!(bounds(&s, b).origin(), Point::new(198.0, 2.0));
}

#[test]
fn corner_cursors_stay_diagonal_whatever_the_proportions() {
    let mut s = session();
    let mod180 = |a: f64, b: f64| near((a - b).rem_euclid(180.0).min((b - a).rem_euclid(180.0)), 0.0);
    for (w, h) in [(65.0, 15.0), (100.0, 100.0), (60.0, 200.0), (200.0, 50.0)] {
        let r = rect(&mut s, 0.0, 0.0, w, h, None);
        let doc = s.doc().unwrap().clone();
        let q = selection_box(&doc, &[r]).unwrap();
        for (i, want) in [(0u8, 45.0), (1, 135.0), (2, 45.0), (3, 135.0)] {
            let Cursor::Resize(a) = handle_cursor(&q, Handle::Corner(i)) else { panic!() };
            assert!(mod180(a, want), "{w}x{h} corner {i}: {a}, want {want}");
        }
        // Turned, the corner cursors turn with the layer's own axes.
        set(&mut s, &[r], json!({"rotation": 30}));
        let doc = s.doc().unwrap().clone();
        let q = selection_box(&doc, &[r]).unwrap();
        let axis = (q[1] - q[0]).atan2().to_degrees();
        for (i, off) in [(0u8, 45.0), (1, 135.0), (2, 45.0), (3, 135.0)] {
            let Cursor::Resize(a) = handle_cursor(&q, Handle::Corner(i)) else { panic!() };
            assert!(mod180(a, axis + off), "{w}x{h} turned corner {i}: {a}, want {}", axis + off);
        }
    }
}

#[test]
fn the_transform_box_and_its_handles() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 100.0, None);
    let doc = s.doc().unwrap().clone();
    let z = 1.0;
    // Cursors follow the handle and the box's rotation; outside a corner, the rotation cursor.
    let q = selection_box(&doc, &[r]).unwrap();
    for (h, angle) in [(Handle::Corner(0), 45.0), (Handle::Corner(1), 135.0), (Handle::Edge(0), 90.0), (Handle::Edge(1), 180.0)] {
        assert!(matches!(handle_cursor(&q, h), Cursor::Resize(a) if near(a, angle)), "{h:?}");
    }
    assert!(matches!(handle_cursor(&q, Handle::Rotate(1)), Cursor::Rotate(a) if near(a, -45.0)), "turned to the top right corner");
    assert_eq!(handle_at(&doc, &[r], Point::new(110.0, -10.0), z), Some(Handle::Rotate(1)));
    set(&mut s, &[r], json!({"rotation": 30}));
    let doc = s.doc().unwrap().clone();
    let q = selection_box(&doc, &[r]).unwrap();
    let Cursor::Resize(a) = handle_cursor(&q, Handle::Edge(1)) else { panic!() };
    assert!(near((a - 150.0).rem_euclid(180.0), 0.0), "the edge cursor turns with the layer: {a}");
    // Hovering sets the cursor.
    ex(&mut s, "select.set", json!({"ids": [r]}));
    let corner = doc.world_transform(r) * Point::new(100.0, 100.0);
    s.pointer(&ev(PointerKind::Hover, corner, NONE)).unwrap();
    assert!(matches!(s.tool.overlay.cursor, Cursor::Resize(_)));
    // A layer smaller than its handles on screen: a press inside it moves it, the handles still
    // answer just outside it.
    let tiny = rect(&mut s, 300.0, 0.0, 4.0, 4.0, None);
    ex(&mut s, "select.set", json!({"ids": [tiny]}));
    let doc = s.doc().unwrap().clone();
    assert_eq!(handle_at(&doc, &[tiny], Point::new(302.0, 2.0), z), None, "inside: no handle");
    assert_eq!(handle_at(&doc, &[tiny], Point::new(297.0, 297.0 - 300.0), z), Some(Handle::Corner(0)), "outside the corner: its handle");
    drag(&mut s, Point::new(302.0, 2.0), Point::new(322.0, 12.0), NONE);
    assert_eq!(bounds(&s, tiny), Rect::new(320.0, 10.0, 324.0, 14.0), "a press inside moves it");
    drag(&mut s, Point::new(326.0, 16.0), Point::new(340.0, 30.0), NONE);
    assert_eq!(bounds(&s, tiny), Rect::new(320.0, 10.0, 340.0, 30.0), "the corner handle just outside resizes it");
    // At a larger zoom the same layer is big enough on screen for its handles inside it.
    assert_eq!(handle_at(s.doc().unwrap(), &[tiny], Point::new(320.2, 10.2), 20.0), Some(Handle::Corner(0)));
}
