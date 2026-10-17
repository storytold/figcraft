//! The cursor each tool asks for (`tool.overlay.cursor`), by tool, modifier and what the pointer
//! is over: tables of tool × modifiers × hover target → cursor, the cursors of gestures under way
//! (panning, duplicating, rotating, an edit an instance refuses) and of vector edit mode.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{Cursor, PenCursor, PointerEvent, PointerKind, ToolId};
use figcraft_doc::NodeId;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

#[derive(Clone, Copy, Default)]
struct Mods {
    shift: bool,
    alt: bool,
    cmd: bool,
}

const NONE: Mods = Mods { shift: false, alt: false, cmd: false };
const ALT: Mods = Mods { shift: false, alt: true, cmd: false };
const CMD: Mods = Mods { shift: false, alt: false, cmd: true };

fn ev(kind: PointerKind, (x, y): (f64, f64), m: Mods) -> PointerEvent {
    PointerEvent { shift: m.shift, alt: m.alt, cmd: m.cmd, ..PointerEvent::at(kind, x, y, 1.0) }
}

fn hover(s: &mut Session, at: (f64, f64), m: Mods) -> Cursor {
    s.pointer(&ev(PointerKind::Hover, at, m)).unwrap();
    s.tool.overlay.cursor
}

/// A resize cursor's direction, 0–180° (it has two heads).
fn resize(c: Cursor) -> f64 {
    match c {
        Cursor::Resize(a) => a.rem_euclid(180.0),
        other => panic!("not a resize cursor: {other:?}"),
    }
}

fn rotate(c: Cursor) -> f64 {
    match c {
        Cursor::Rotate(a) => a.rem_euclid(360.0),
        other => panic!("not a rotate cursor: {other:?}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// A square (100,100)–(200,200), selected, and the text "Hello" at (400, 100), not selected.
fn scene() -> (Session, NodeId, NodeId) {
    // The text is measured with a font: without craft-fonts that is a system one, scanned here
    // rather than by whichever test happens to run first (#333).
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let r = id(&ex(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})));
    let t = id(&ex(&mut s, "text.create", json!({"text": "Hello", "x": 400, "y": 100, "props": {"fontSize": 40}})));
    ex(&mut s, "select.set", json!({"ids": [r]}));
    (s, r, t)
}

const EMPTY: (f64, f64) = (700.0, 600.0);
const ON_SQUARE: (f64, f64) = (150.0, 150.0);
const ON_TEXT: (f64, f64) = (420.0, 120.0);

#[test]
fn each_tool_has_its_cursor_by_modifier_and_what_is_under_the_pointer() {
    use Cursor::*;
    // (tool, modifiers, where, cursor, why)
    type Row = (ToolId, Mods, (f64, f64), Cursor, &'static str);
    let rows: Vec<Row> = vec![
        (ToolId::Move, NONE, EMPTY, Default, "the arrow over empty canvas"),
        (ToolId::Move, NONE, ON_SQUARE, Default, "the arrow over a layer"),
        (ToolId::Move, NONE, ON_TEXT, Default, "the arrow over a text that isn't being edited"),
        (ToolId::Move, ALT, ON_SQUARE, Copy, "Alt over a selected layer: a drag makes a copy"),
        (ToolId::Move, ALT, ON_TEXT, Default, "Alt over another layer measures to it"),
        (ToolId::Move, ALT, EMPTY, Default, "Alt over empty canvas"),
        (ToolId::Scale, NONE, ON_SQUARE, Default, "the Scale tool away from the handles"),
        (ToolId::Scale, ALT, ON_SQUARE, Default, "the Scale tool doesn't duplicate"),
        (ToolId::Hand, NONE, EMPTY, Grab, "the open hand"),
        (ToolId::Hand, NONE, ON_SQUARE, Grab, "the open hand over a layer too"),
        (ToolId::Frame, NONE, EMPTY, Crosshair, "frame"),
        (ToolId::Section, NONE, EMPTY, Crosshair, "section"),
        (ToolId::Slice, NONE, EMPTY, Crosshair, "slice"),
        (ToolId::Rectangle, NONE, ON_SQUARE, Crosshair, "shapes are drawn over layers too"),
        (ToolId::Line, NONE, EMPTY, Crosshair, "line"),
        (ToolId::Arrow, NONE, EMPTY, Crosshair, "arrow"),
        (ToolId::Ellipse, NONE, EMPTY, Crosshair, "ellipse"),
        (ToolId::Polygon, NONE, EMPTY, Crosshair, "polygon"),
        (ToolId::Star, NONE, EMPTY, Crosshair, "star"),
        (ToolId::Pencil, NONE, EMPTY, Crosshair, "pencil"),
        (ToolId::Text, NONE, EMPTY, Crosshair, "the Text tool makes a text where it is pressed"),
        (ToolId::Text, NONE, ON_SQUARE, Crosshair, "over a shape too"),
        (ToolId::Text, NONE, ON_TEXT, Text, "the I-beam over a text: a click edits it there"),
        (ToolId::Pen, NONE, EMPTY, Pen(PenCursor::New), "the pen: a new point"),
        (ToolId::Comment, NONE, EMPTY, Comment, "the comment pin"),
        (ToolId::Eyedropper, NONE, ON_SQUARE, Eyedropper, "the eyedropper"),
    ];
    for (tool, m, at, want, why) in rows {
        let (mut s, ..) = scene();
        s.set_tool(tool);
        assert_eq!(s.tool.overlay.cursor, tool.cursor(), "{tool:?}: its cursor shows as soon as it is chosen");
        assert_eq!(hover(&mut s, at, m), want, "{tool:?} at {at:?}: {why}");
    }
    // Every tool has a row above.
    for t in ToolId::ALL {
        assert!(matches!(t.cursor(), Default | Crosshair | Grab | Pen(_) | Comment | Eyedropper), "{t:?}");
    }
}

#[test]
fn handles_show_arrows_that_follow_the_handle_and_the_boxs_rotation() {
    let (mut s, r, _) = scene();
    // (where, the arrow's direction in degrees: 0 = left-right, 45 = top left to bottom right)
    let handles = [((200.0, 150.0), 0.0), ((150.0, 100.0), 90.0), ((200.0, 200.0), 45.0), ((100.0, 200.0), 135.0), ((100.0, 100.0), 45.0)];
    for (at, want) in handles {
        let got = resize(hover(&mut s, at, NONE));
        assert!(close(got, want), "handle at {at:?}: {got}° (wanted {want}°)");
    }
    // Just outside each corner: the rotate cursor, turned to that corner (from the centre).
    for (at, want) in [((208.0, 208.0), 45.0), ((92.0, 208.0), 135.0), ((92.0, 92.0), 225.0), ((208.0, 92.0), 315.0)] {
        let got = rotate(hover(&mut s, at, NONE));
        assert!(close(got, want), "outside the corner at {at:?}: {got}° (wanted {want}°)");
    }
    // The Scale tool's handles too.
    s.set_tool(ToolId::Scale);
    assert!(close(resize(hover(&mut s, (200.0, 150.0), NONE)), 0.0));
    s.set_tool(ToolId::Move);
    // A box turned by 30° turns its cursors by 30°: the handle that was at the right edge.
    ex(&mut s, "node.set", json!({"id": r, "rotation": 30}));
    let corners = crate::tools::selection_box(s.doc().unwrap(), &[r]).unwrap();
    let mid = |a: usize, b: usize| ((corners[a].x + corners[b].x) / 2.0, (corners[a].y + corners[b].y) / 2.0);
    let edge = resize(hover(&mut s, mid(1, 2), NONE));
    assert!(close(edge, 30.0) || close(edge, 150.0), "the right edge's arrow turned with the box: {edge}°");
    let centre = mid(0, 2);
    let c = corners[2];
    let out = (c.x + (c.x - centre.0) * 0.15, c.y + (c.y - centre.1) * 0.15);
    let want = (c.y - centre.1).atan2(c.x - centre.0).to_degrees().rem_euclid(360.0);
    let got = rotate(hover(&mut s, out, NONE));
    assert!(close(got, want) && !close(got, 45.0), "the rotate cursor points away from the turned box's centre: {got}° (wanted {want}°)");
    // A locked layer has no handles: the arrow.
    ex(&mut s, "node.set", json!({"id": r, "rotation": 0, "locked": true}));
    assert_eq!(hover(&mut s, (200.0, 150.0), NONE), Cursor::Default);
}

#[test]
fn gestures_show_what_they_are_doing() {
    // Panning: the hand closes.
    let (mut s, r, _) = scene();
    s.set_tool(ToolId::Hand);
    s.pointer(&ev(PointerKind::Down, EMPTY, NONE)).unwrap();
    assert_eq!(s.tool.overlay.cursor, Cursor::Grabbing);
    s.pointer(&ev(PointerKind::Up, EMPTY, NONE)).unwrap();
    assert_eq!(hover(&mut s, EMPTY, NONE), Cursor::Grab, "and opens again");
    // Alt-drag: the copy cursor from the press to the release; a plain drag: the arrow.
    for (m, want) in [(ALT, Cursor::Copy), (NONE, Cursor::Default)] {
        let (mut s, ..) = scene();
        s.pointer(&ev(PointerKind::Down, ON_SQUARE, m)).unwrap();
        assert_eq!(s.tool.overlay.cursor, want, "on the press");
        s.pointer(&ev(PointerKind::Move, (180.0, 170.0), m)).unwrap();
        s.pointer(&ev(PointerKind::Move, (260.0, 190.0), m)).unwrap();
        assert_eq!(s.tool.overlay.cursor, want, "while dragging");
        s.pointer(&ev(PointerKind::Up, (260.0, 190.0), m)).unwrap();
    }
    // Rotating: the cursor turns with the pointer about the centre.
    s.set_tool(ToolId::Move);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    s.pointer(&ev(PointerKind::Down, (208.0, 208.0), NONE)).unwrap();
    for (at, want) in [((150.0, 260.0), 90.0), ((40.0, 150.0), 180.0), ((150.0, 40.0), 270.0)] {
        s.pointer(&ev(PointerKind::Move, at, NONE)).unwrap();
        let got = rotate(s.tool.overlay.cursor);
        assert!(close(got, want), "rotating with the pointer at {at:?}: {got}° (wanted {want}°)");
    }
    s.pointer(&ev(PointerKind::Up, (150.0, 40.0), NONE)).unwrap();
    // While a text is edited, the I-beam over it (with the Move tool) and the arrow elsewhere.
    let (mut s, _, t) = scene();
    ex(&mut s, "text.edit.begin", json!({"id": t}));
    assert_eq!(hover(&mut s, ON_TEXT, NONE), Cursor::Text);
    assert_eq!(hover(&mut s, EMPTY, NONE), Cursor::Default);
    ex(&mut s, "text.edit.end", json!({}));
    assert_eq!(hover(&mut s, ON_TEXT, NONE), Cursor::Default, "editing over: the arrow");
}

#[test]
fn an_edit_an_instance_refuses_shows_not_allowed_and_is_undone() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 200})));
    ex(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100, "parent": f}));
    let main = id(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let inst = id(&ex(&mut s, "instance.create", json!({"component": main, "x": 400, "y": 0})));
    // The rectangle inside the instance: (450,50)–(550,150).
    let inner = s.doc().unwrap().node(inst).unwrap().children[0];
    let before = s.doc().unwrap().world_bounds(inner).unwrap();
    assert_eq!((before.x0, before.y0, before.x1, before.y1), (450.0, 50.0, 550.0, 150.0));
    ex(&mut s, "select.set", json!({"ids": [inner]}));
    // Resizing it by its right edge: a size of its own (an override).
    assert!(close(resize(hover(&mut s, (550.0, 100.0), NONE)), 0.0), "the handle shows its arrow");
    s.pointer(&ev(PointerKind::Down, (550.0, 100.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, (570.0, 100.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, (600.0, 100.0), NONE)).unwrap();
    assert_ne!(s.tool.overlay.cursor, Cursor::NotAllowed);
    s.pointer(&ev(PointerKind::Up, (600.0, 100.0), NONE)).unwrap();
    let before = s.doc().unwrap().world_bounds(inner).unwrap();
    assert_eq!((before.x0, before.x1), (450.0, 600.0), "the resize stays");
    // Moving it: only the main component can.
    s.pointer(&ev(PointerKind::Down, (500.0, 100.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, (510.0, 110.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, (520.0, 120.0), NONE)).unwrap();
    assert_eq!(s.tool.overlay.cursor, Cursor::NotAllowed);
    s.pointer(&ev(PointerKind::Up, (520.0, 120.0), NONE)).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(inner).unwrap(), before, "the move was undone");
}

#[test]
fn a_pen_click_that_wobbles_a_pixel_is_still_a_click() {
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    // A real click: the mouse moves a pixel or two between press and release.
    s.pointer(&ev(PointerKind::Down, (100.0, 100.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, (102.0, 101.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Up, (102.0, 101.0), NONE)).unwrap();
    let out = s.tool.pen.as_ref().and_then(|p| p.points.first()).map(|(_, _, o)| *o);
    assert_eq!(out.map(|o| o.hypot()), Some(0.0), "no handle pulled out");
    assert!(s.tool.overlay.pen_handles.is_empty());
    // Past the slop it is a drag: the handle follows the pointer from where it was pressed.
    s.pointer(&ev(PointerKind::Down, (200.0, 100.0), NONE)).unwrap();
    s.pointer(&ev(PointerKind::Move, (204.0, 100.0), NONE)).unwrap();
    let out = s.tool.pen.as_ref().and_then(|p| p.points.last()).map(|(_, _, o)| (o.x, o.y));
    assert_eq!(out, Some((4.0, 0.0)));
}

#[test]
fn the_pen_shows_what_a_click_would_do() {
    // The Pen tool drawing a new path: three points, then over each of them.
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    for at in [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0)] {
        s.pointer(&ev(PointerKind::Down, at, NONE)).unwrap();
        s.pointer(&ev(PointerKind::Up, at, NONE)).unwrap();
    }
    let rows = [
        ((300.0, 300.0), PenCursor::New, "empty canvas: a new point"),
        ((101.0, 101.0), PenCursor::Close, "its first point: closes the path"),
        ((200.0, 101.0), PenCursor::Close, "another of its points: connects to it and ends the path"),
        ((200.0, 200.0), PenCursor::New, "its last point: nothing to connect"),
    ];
    for (at, want, why) in rows {
        assert_eq!(hover(&mut s, at, NONE), Cursor::Pen(want), "{why}");
    }
    s.escape().unwrap();
    // With an open path selected, its end points go on with it.
    let path = id(&ex(&mut s, "vector.create", json!({"points": [[400, 100], [500, 100], [500, 200]], "closed": false})));
    ex(&mut s, "select.set", json!({"ids": [path]}));
    s.set_tool(ToolId::Pen);
    assert_eq!(hover(&mut s, (500.0, 200.0), NONE), Cursor::Pen(PenCursor::Close), "an end point of the selected open path");
    assert_eq!(hover(&mut s, (500.0, 100.0), NONE), Cursor::Pen(PenCursor::New), "its middle point isn't an end");
}

#[test]
fn vector_edit_mode_shows_each_tool_and_modifier() {
    let mut s = Session::new();
    let path = id(&ex(&mut s, "vector.create", json!({"points": [[100, 100], [300, 100], [300, 300]], "closed": false})));
    ex(&mut s, "vector.edit.begin", json!({"id": path}));
    let (point, segment, empty) = ((300.0, 100.0), (200.0, 100.0), (200.0, 250.0));
    use Cursor::*;
    // (vector tool, modifiers, where, cursor, why)
    type Row = (&'static str, Mods, (f64, f64), Cursor, &'static str);
    let rows: Vec<Row> = vec![
        ("move", NONE, point, Default, "the arrow over a point"),
        ("move", NONE, segment, Default, "the arrow over a segment"),
        ("move", NONE, empty, Default, "the arrow elsewhere"),
        ("move", ALT, segment, Pen(PenCursor::Add), "Alt over a segment adds a point there"),
        ("move", ALT, point, Default, "Alt over a point adds nothing"),
        ("move", CMD, segment, Pen(PenCursor::Bend), "Cmd/Ctrl bends the segment"),
        ("move", CMD, point, Pen(PenCursor::Bend), "Cmd/Ctrl bends the point's handles"),
        ("move", CMD, empty, Crosshair, "Cmd/Ctrl over nothing"),
        ("pen", NONE, empty, Pen(PenCursor::New), "the pen: a new point"),
        ("pen", NONE, point, Pen(PenCursor::Close), "the pen over a point: connects to it or goes on from it"),
        ("pen", NONE, segment, Pen(PenCursor::Add), "the pen over a segment: adds a point"),
        ("bend", NONE, segment, Pen(PenCursor::Bend), "the Bend tool"),
        ("bend", NONE, empty, Pen(PenCursor::Bend), "the Bend tool anywhere"),
        ("paint", NONE, empty, Crosshair, "paint bucket"),
        ("lasso", NONE, empty, Crosshair, "lasso"),
        ("cut", NONE, segment, Crosshair, "cut"),
        ("eraser", NONE, empty, Crosshair, "eraser (its brush is drawn around the pointer)"),
        ("width", NONE, segment, Crosshair, "variable width"),
    ];
    for (tool, m, at, want, why) in rows {
        ex(&mut s, "vector.edit.tool", json!({"tool": tool}));
        assert_eq!(hover(&mut s, at, m), want, "{tool} at {at:?}: {why}");
    }
}
