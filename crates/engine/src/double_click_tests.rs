//! Double-clicks as a mouse sends them (press, release, press, double-click, with a pixel or two
//! of hand movement between) into a group's text, and the drag threshold that keeps such clicks
//! from moving or resizing what they land on.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeId;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// A frame holding a group of a rectangle and a text layer at (60, 60); returns (group, text).
fn scene(s: &mut Session) -> (NodeId, NodeId) {
    // The text is measured with a font: without craft-fonts that is a system one, scanned here
    // rather than by whichever test happens to run first (#333).
    figcraft_text::scan_system_fonts();
    let f = id_of(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 300}))["id"]).unwrap();
    let r = id_of(&ex(s, "shape.rectangle", json!({"x": 40, "y": 40, "width": 200, "height": 60, "parent": f}))["id"]).unwrap();
    let t = id_of(&ex(s, "text.create", json!({"text": "Hi there", "x": 60, "y": 60, "parent": f, "props": {"fontSize": 16}}))["id"]).unwrap();
    let g = id_of(&ex(s, "object.group", json!({"ids": [r, t]}))["id"]).unwrap();
    ex(s, "select.none", json!({}));
    (g, t)
}

fn send(s: &mut Session, kind: PointerKind, x: f64, y: f64, zoom: f64) {
    s.pointer(&PointerEvent::at(kind, x, y, zoom)).unwrap();
}

/// A click at (x, y) whose release is `jitter` away.
fn click(s: &mut Session, x: f64, y: f64, jitter: f64, zoom: f64) {
    send(s, PointerKind::Down, x, y, zoom);
    send(s, PointerKind::Move, x + jitter, y, zoom);
    send(s, PointerKind::Up, x + jitter, y, zoom);
}

/// What the canvas sends for a double-click: a click, a second press, the double-click (no
/// release), then the pointer hovering on.
fn double_click(s: &mut Session, x: f64, y: f64, jitter: f64, zoom: f64) {
    click(s, x, y, jitter, zoom);
    send(s, PointerKind::Down, x, y, zoom);
    send(s, PointerKind::Move, x + jitter, y, zoom);
    send(s, PointerKind::DoubleClick, x + jitter, y, zoom);
    send(s, PointerKind::Move, x + jitter + 1.0, y, zoom);
}

fn editing(s: &Session) -> Option<NodeId> {
    s.ds().unwrap().text_edit.as_ref().map(|t| t.id)
}

fn origin(s: &Session, id: NodeId) -> (f64, f64, f64, f64) {
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    (b.x0, b.y0, b.width(), b.height())
}

#[test]
fn double_clicks_reach_and_edit_text_in_a_group_wherever_they_land() {
    // The middle of the text (a move would start there), near its edges (a resize would) and
    // with the hand moving a pixel or two between press and release.
    for (x, y) in [(100.0, 70.0), (100.0, 61.0), (100.0, 78.0), (61.0, 70.0), (80.0, 70.0)] {
        for jitter in [0.0, 1.0, 2.0] {
            let mut s = Session::new();
            let (g, t) = scene(&mut s);
            let before = origin(&s, t);
            click(&mut s, x, y, jitter, 1.0);
            assert_eq!(s.selection(), vec![g], "a click picks the group ({x}, {y}, {jitter})");
            double_click(&mut s, x, y, jitter, 1.0);
            assert_eq!(s.selection(), vec![t], "a double-click picks the text ({x}, {y}, {jitter})");
            assert_eq!(editing(&s), None);
            double_click(&mut s, x, y, jitter, 1.0);
            assert_eq!(editing(&s), Some(t), "the next one edits it ({x}, {y}, {jitter})");
            assert_eq!(origin(&s, t), before, "and nothing was dragged ({x}, {y}, {jitter})");
        }
    }
}

#[test]
fn editing_text_keeps_other_document_changes_since_open() {
    let mut s = Session::new();
    let first = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 80, "height": 40}))["id"]).unwrap();
    let deleted = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 120, "y": 20, "width": 80, "height": 40}))["id"]).unwrap();
    let text = id_of(&ex(&mut s, "text.create", json!({"text": "A", "x": 20, "y": 100}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"ids": [first], "props": {"x": 650}}));
    ex(&mut s, "edit.delete", json!({"ids": [deleted]}));

    let bounds = s.doc().unwrap().world_bounds(text).unwrap();
    double_click(&mut s, bounds.center().x, bounds.center().y, 0.0, 1.0);
    assert_eq!(editing(&s), Some(text));
    crate::textedit::insert(&mut s, "B").unwrap();
    crate::textedit::end(&mut s).unwrap();

    assert_eq!(s.doc().unwrap().world_bounds(first).unwrap().x0, 650.0);
    assert!(!s.doc().unwrap().contains(deleted));
    assert_eq!(s.doc().unwrap().node(text).unwrap().kind.text().unwrap().characters, "B");
}

#[test]
fn a_press_drags_only_past_the_threshold_in_screen_pixels() {
    for (zoom, by, moves) in [(1.0, 2.0, false), (1.0, 4.0, true), (4.0, 0.5, false), (4.0, 1.0, true), (0.25, 8.0, false), (0.25, 16.0, true)] {
        let mut s = Session::new();
        let r = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 200}))["id"]).unwrap();
        ex(&mut s, "view.snapPixel", json!({"on": false}));
        // A move from the middle.
        send(&mut s, PointerKind::Down, 100.0, 100.0, zoom);
        send(&mut s, PointerKind::Move, 100.0 + by, 100.0, zoom);
        send(&mut s, PointerKind::Up, 100.0 + by, 100.0, zoom);
        assert_eq!(origin(&s, r).0 != 0.0, moves, "move by {by} at {zoom}");
        // Past the threshold the layer follows the pointer from where it was pressed, and coming
        // back near the start keeps following.
        send(&mut s, PointerKind::Down, 100.0, 100.0, zoom);
        send(&mut s, PointerKind::Move, 100.0 + 40.0 / zoom, 100.0, zoom);
        send(&mut s, PointerKind::Move, 100.0 + 1.0 / zoom, 100.0, zoom);
        let x = origin(&s, r).0;
        send(&mut s, PointerKind::Up, 100.0 + 1.0 / zoom, 100.0, zoom);
        s.undo().unwrap();
        assert!(x != 0.0 || !moves, "back near the start still follows at {zoom}");
    }
    // A resize from the right edge.
    for (by, resizes) in [(2.0, false), (4.0, true)] {
        let mut s = Session::new();
        let r = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 200}))["id"]).unwrap();
        send(&mut s, PointerKind::Down, 200.0, 100.0, 1.0);
        send(&mut s, PointerKind::Move, 200.0 + by, 100.0, 1.0);
        send(&mut s, PointerKind::Up, 200.0 + by, 100.0, 1.0);
        assert_eq!(origin(&s, r).2 != 200.0, resizes, "resize by {by}");
    }
}
