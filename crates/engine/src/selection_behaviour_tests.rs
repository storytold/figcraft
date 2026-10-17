//! Selection behaviour suite, written from the public documentation of design tools: each rule
//! is a table row with the selection it implies and a one-line derivation. Where the
//! documentation is silent the row says so and pins today's behaviour.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, handle_at};
use figcraft_doc::NodeId;
use figcraft_geom::Point;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, x: f64, y: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": 40, "height": 40});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&ex(s, "shape.rectangle", p))
}

/// The layers of the scene, by name.
#[allow(dead_code)]
struct Scene {
    /// Top-level frame (0,0 400×300) holding: group `g` of `a` (20,20) and `b` (80,20); `c`
    /// (20,120); nested frame `n` (200,20 150×150) with `d` (220,40); boolean `u` of two
    /// rectangles at (20,200) and (40,200).
    f: NodeId,
    g: NodeId,
    a: NodeId,
    b: NodeId,
    c: NodeId,
    n: NodeId,
    d: NodeId,
    u: NodeId,
    /// On the page: `r` (500,0); `r2` (500,100) under the locked `lk` (same place); the hidden
    /// `hid` (500,200); the empty frame `empty` (600,0 80×80).
    r: NodeId,
    r2: NodeId,
    lk: NodeId,
    hid: NodeId,
    empty: NodeId,
}

fn scene(s: &mut Session) -> Scene {
    let f = id(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 300})));
    let a = rect(s, 20.0, 20.0, Some(f));
    let b = rect(s, 80.0, 20.0, Some(f));
    let g = id(&ex(s, "object.group", json!({"ids": [a, b]})));
    let c = rect(s, 20.0, 120.0, Some(f));
    let n = id(&ex(s, "frame.create", json!({"x": 200, "y": 20, "width": 150, "height": 150, "parent": f})));
    // Positions are in the parent's space.
    let d = rect(s, 20.0, 20.0, Some(n));
    let e1 = rect(s, 20.0, 200.0, Some(f));
    let e2 = rect(s, 40.0, 200.0, Some(f));
    let u = id(&ex(s, "bool.union", json!({"ids": [e1, e2]})));
    let r = rect(s, 500.0, 0.0, None);
    let r2 = rect(s, 500.0, 100.0, None);
    let lk = rect(s, 500.0, 100.0, None);
    ex(s, "object.toggleLock", json!({"ids": [lk], "locked": true}));
    let hid = rect(s, 500.0, 200.0, None);
    ex(s, "object.toggleVisible", json!({"ids": [hid], "visible": false}));
    let empty = id(&ex(s, "frame.create", json!({"x": 600, "y": 0, "width": 80, "height": 80})));
    ex(s, "select.none", json!({}));
    Scene { f, g, a, b, c, n, d, u, r, r2, lk, hid, empty }
}

#[derive(Clone, Copy, Default)]
struct Mods {
    shift: bool,
    cmd: bool,
}

const NONE: Mods = Mods { shift: false, cmd: false };
const SHIFT: Mods = Mods { shift: true, cmd: false };
const CMD: Mods = Mods { shift: false, cmd: true };

fn ev(kind: PointerKind, x: f64, y: f64, m: Mods) -> PointerEvent {
    let mut e = PointerEvent::at(kind, x, y, 1.0);
    (e.shift, e.cmd) = (m.shift, m.cmd);
    e
}

/// A press and release without moving.
fn click(s: &mut Session, x: f64, y: f64, m: Mods) {
    s.pointer(&ev(PointerKind::Down, x, y, m)).unwrap();
    s.pointer(&ev(PointerKind::Up, x, y, m)).unwrap();
}

fn drag(s: &mut Session, a: (f64, f64), b: (f64, f64), m: Mods) {
    s.pointer(&ev(PointerKind::Down, a.0, a.1, m)).unwrap();
    s.pointer(&ev(PointerKind::Move, b.0, b.1, m)).unwrap();
    s.pointer(&ev(PointerKind::Up, b.0, b.1, m)).unwrap();
}

fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
    v.sort();
    v
}

#[test]
fn clicking_selects_by_the_documented_rules() {
    let mut s = Session::new();
    let k = scene(&mut s);
    // (point, modifiers, selection before, selection after, derivation)
    let rows: Vec<((f64, f64), Mods, Vec<NodeId>, Vec<NodeId>, &str)> = vec![
        ((520.0, 20.0), NONE, vec![], vec![k.r], "a click selects the layer under the pointer"),
        ((30.0, 30.0), NONE, vec![], vec![k.g], "a group is selected as a whole"),
        ((30.0, 130.0), NONE, vec![], vec![k.c], "a layer in a top-level frame is selected directly (frames hold their layers like the canvas)"),
        ((230.0, 50.0), NONE, vec![], vec![k.n], "silent: a nested frame selects as a whole, like a group (pinned)"),
        ((30.0, 210.0), NONE, vec![], vec![k.u], "a boolean group is selected as a whole"),
        ((30.0, 30.0), CMD, vec![], vec![k.a], "Cmd/Ctrl-click selects the deepest layer"),
        ((230.0, 50.0), CMD, vec![], vec![k.d], "Cmd/Ctrl-click reaches into nested frames"),
        ((30.0, 210.0), CMD, vec![], vec![sel_operand(&s, k.u, 30.0)], "Cmd/Ctrl-click reaches a boolean's operand"),
        ((90.0, 30.0), NONE, vec![k.a], vec![k.b], "with a layer selected, its siblings select at its level"),
        ((520.0, 20.0), SHIFT, vec![k.c], vec![k.c, k.r], "Shift-click adds to the selection"),
        ((30.0, 130.0), SHIFT, vec![k.c, k.r], vec![k.r], "Shift-click on a selected layer removes it"),
        ((520.0, 120.0), NONE, vec![], vec![k.r2], "a locked layer can't be selected on the canvas: the click goes to what is under it"),
        ((520.0, 220.0), NONE, vec![k.r], vec![], "a hidden layer can't be selected on the canvas (empty canvas deselects)"),
        ((390.0, 290.0), NONE, vec![k.r], vec![], "the background of a top-level frame with layers is like the canvas: a click deselects"),
        ((390.0, 290.0), NONE, vec![k.f], vec![k.f], "a selected top-level frame stays selected when its background is clicked"),
        ((390.0, 290.0), CMD, vec![], vec![k.f], "silent: Cmd/Ctrl-click on a frame's background selects the frame (the deepest layer there)"),
        ((640.0, 40.0), NONE, vec![], vec![k.empty], "an empty top-level frame is selected by a click on it"),
    ];
    for (p, m, before, after, why) in rows {
        ex(&mut s, "select.set", json!({"ids": before}));
        click(&mut s, p.0, p.1, m);
        assert_eq!(sorted(s.selection()), sorted(after), "{why}");
    }
}

/// The operand of boolean `u` under x (its children are the two rectangles).
fn sel_operand(s: &Session, u: NodeId, x: f64) -> NodeId {
    let doc = s.doc().unwrap();
    let kids = &doc.node(u).unwrap().children;
    // The later child paints on top: (40,200) covers x ≥ 40, else the first.
    if x >= 40.0 { kids[1] } else { kids[0] }
}

#[test]
fn double_click_enters_one_level() {
    let mut s = Session::new();
    let k = scene(&mut s);
    let comp_child = rect(&mut s, 800.0, 0.0, None);
    let comp = id(&ex(&mut s, "component.create", json!({"ids": [comp_child]})));
    let inst = id(&ex(&mut s, "instance.create", json!({"component": comp, "x": 300, "y": 200, "parent": k.f})));
    let inst_child = s.doc().unwrap().node(inst).unwrap().children[0];
    // (point, selection before, selection after, derivation)
    let rows: Vec<((f64, f64), Vec<NodeId>, Vec<NodeId>, &str)> = vec![
        ((30.0, 30.0), vec![k.g], vec![k.a], "double-click on a group selects the layer inside it"),
        ((230.0, 50.0), vec![k.n], vec![k.d], "double-click on a nested frame selects the layer inside it"),
        ((30.0, 210.0), vec![k.u], vec![sel_operand(&s, k.u, 30.0)], "double-click on a boolean group selects its operand"),
        ((320.0, 220.0), vec![inst], vec![inst_child], "double-click on an instance selects the layer inside it"),
        ((30.0, 30.0), vec![], vec![k.a], "a double-click's first click selects the group, the second enters it"),
    ];
    for (p, before, after, why) in rows {
        ex(&mut s, "select.set", json!({"ids": before}));
        click(&mut s, p.0, p.1, NONE);
        s.pointer(&ev(PointerKind::DoubleClick, p.0, p.1, NONE)).unwrap();
        assert_eq!(s.selection(), after, "{why}");
    }
    // A selected shape: vector edit mode.
    ex(&mut s, "select.set", json!({"ids": [k.r]}));
    s.pointer(&ev(PointerKind::DoubleClick, 520.0, 20.0, NONE)).unwrap();
    assert_eq!(s.ds().unwrap().vector_edit.as_ref().map(|v| v.id), Some(k.r), "double-click on a shape edits its points");
}

#[test]
fn marquee_selects_what_it_touches() {
    let mut s = Session::new();
    let k = scene(&mut s);
    // (from, to, modifiers, selection before, selection after, derivation)
    let rows: Vec<((f64, f64), (f64, f64), Mods, Vec<NodeId>, Vec<NodeId>, &str)> = vec![
        ((480.0, -20.0), (505.0, 10.0), NONE, vec![], vec![k.r], "a marquee selects layers it touches, not only those it contains"),
        ((480.0, -20.0), (560.0, 260.0), NONE, vec![], vec![k.r, k.r2], "locked and hidden layers aren't selected by a marquee"),
        (
            (-10.0, -10.0),
            (450.0, 40.0),
            NONE,
            vec![],
            vec![],
            "from the canvas, a top-level frame is selected only when the marquee covers all of it",
        ),
        ((-10.0, -10.0), (450.0, 310.0), NONE, vec![], vec![k.f], "a marquee covering a top-level frame selects the frame"),
        (
            (390.0, 290.0),
            (25.0, 125.0),
            NONE,
            vec![],
            vec![k.c, k.n, k.u],
            "starting on a top-level frame's background selects the layers in it the marquee touches",
        ),
        (
            (450.0, 290.0),
            (25.0, 25.0),
            CMD,
            vec![],
            vec![k.a, k.b, k.c, k.d, k.u],
            "silent: Cmd/Ctrl-marquee selects the innermost layers it touches (pinned)",
        ),
        ((480.0, -20.0), (505.0, 10.0), SHIFT, vec![k.c], vec![k.c, k.r], "Shift-marquee adds to the selection"),
        ((480.0, -20.0), (505.0, 10.0), SHIFT, vec![k.c, k.r], vec![k.c], "Shift-marquee takes out what was selected and is touched again"),
    ];
    for (a, b, m, before, after, why) in rows {
        ex(&mut s, "select.set", json!({"ids": before}));
        drag(&mut s, a, b, m);
        assert_eq!(sorted(s.selection()), sorted(after), "{why}");
    }
}

#[test]
fn keyboard_selection() {
    let mut s = Session::new();
    let k = scene(&mut s);
    let page_layers = |s: &Session| s.doc().unwrap().node(s.page().unwrap()).unwrap().children.clone();
    let visible_unlocked: Vec<NodeId> = page_layers(&s).into_iter().filter(|i| ![k.lk, k.hid].contains(i)).collect();
    // (command, selection before, selection after, derivation)
    let rows: Vec<(&str, Vec<NodeId>, Vec<NodeId>, &str)> = vec![
        ("select.children", vec![k.g], vec![k.a, k.b], "Enter selects the children"),
        ("select.parent", vec![k.a], vec![k.g], "Shift+Enter selects the parent"),
        ("select.parent", vec![k.f], vec![k.f], "silent: Shift+Enter on a top-level layer keeps it (pinned)"),
        ("select.next", vec![k.b], vec![k.a], "Tab selects the next layer down the layers list"),
        ("select.previous", vec![k.a], vec![k.b], "Shift+Tab selects the previous one"),
        ("select.all", vec![k.a], vec![k.a, k.b], "Select all selects the selection's siblings"),
        ("select.all", vec![], visible_unlocked.clone(), "with nothing selected, all layers on the page (hidden and locked ones can't be selected)"),
        (
            "select.inverse",
            vec![k.r],
            visible_unlocked.iter().copied().filter(|i| *i != k.r).collect(),
            "Select inverse: the other layers Select all would select",
        ),
    ];
    for (cmd, before, after, why) in rows {
        ex(&mut s, "select.set", json!({"ids": before}));
        ex(&mut s, cmd, json!({}));
        assert_eq!(sorted(s.selection()), sorted(after), "{why}");
    }
    // Esc steps out to the parent, then deselects.
    ex(&mut s, "select.set", json!({"ids": [k.a]}));
    s.escape().unwrap();
    assert_eq!(s.selection(), vec![k.g], "Esc selects the parent");
    s.escape().unwrap();
    assert_eq!(s.selection(), vec![k.f]);
    s.escape().unwrap();
    assert_eq!(s.selection(), Vec::<NodeId>::new(), "Esc on a top-level layer deselects");
}

#[test]
fn locked_layers_selected_in_the_layers_panel_stay_put() {
    let mut s = Session::new();
    let k = scene(&mut s);
    // Selectable from the layers panel (select.set), but the canvas offers no handles and a drag
    // that starts on another selected layer leaves it where it is.
    ex(&mut s, "select.set", json!({"ids": [k.lk]}));
    let doc = s.doc().unwrap().clone();
    let corner = Point::new(540.0, 140.0);
    assert_eq!(handle_at(&doc, &[k.lk], corner, 1.0), None, "a locked layer has no resize handles");
    assert!(handle_at(&doc, &[k.r2], corner, 1.0).is_some(), "the same box unlocked has");
    ex(&mut s, "select.set", json!({"ids": [k.lk, k.r]}));
    drag(&mut s, (520.0, 20.0), (530.0, 30.0), NONE);
    let doc = s.doc().unwrap();
    assert_eq!(doc.world_bounds(k.r).unwrap().origin(), Point::new(510.0, 10.0), "the unlocked layer moves");
    assert_eq!(doc.world_bounds(k.lk).unwrap().origin(), Point::new(500.0, 100.0), "the locked one doesn't");
    // Layers inside a locked layer are locked too.
    ex(&mut s, "object.toggleLock", json!({"ids": [k.g], "locked": true}));
    assert_eq!(handle_at(s.doc().unwrap(), &[k.a], Point::new(60.0, 60.0), 1.0), None);
}

#[test]
fn shift_drag_moves_the_selection() {
    let mut s = Session::new();
    let k = scene(&mut s);
    let at = |s: &Session, i: NodeId| s.doc().unwrap().world_bounds(i).unwrap().origin();
    let home = (Point::new(500.0, 0.0), Point::new(600.0, 0.0));
    // (selection before, Shift-drag to (from r at 520,20), selection after, where r and empty end up, derivation)
    let rows: Vec<(Vec<NodeId>, (f64, f64), Vec<NodeId>, (Point, Point), &str)> = vec![
        (
            vec![k.r],
            (560.0, 22.0),
            vec![k.r],
            (Point::new(540.0, 0.0), Point::new(600.0, 0.0)),
            "Shift-drag on the selected layer moves it along one axis and keeps it selected",
        ),
        (
            vec![k.r, k.empty],
            (560.0, 22.0),
            vec![k.r, k.empty],
            (Point::new(540.0, 0.0), Point::new(640.0, 0.0)),
            "Shift-drag on one of several selected layers moves them all",
        ),
        (
            vec![k.empty],
            (560.0, 22.0),
            vec![k.r, k.empty],
            (Point::new(540.0, 0.0), Point::new(640.0, 0.0)),
            "Shift-drag on an unselected layer adds it and moves the selection",
        ),
        (vec![k.r, k.empty], (521.0, 21.0), vec![k.empty], home, "a Shift-press that wobbles under the drag threshold is a click: the layer leaves"),
    ];
    for (before, to, after, end, why) in rows {
        ex(&mut s, "select.set", json!({"ids": before}));
        drag(&mut s, (520.0, 20.0), to, SHIFT);
        assert_eq!(sorted(s.selection()), sorted(after), "{why}");
        assert_eq!((at(&s, k.r), at(&s, k.empty)), end, "{why}");
        if end != home {
            assert!(s.undo().unwrap(), "{why}: one undo step");
            assert_eq!((at(&s, k.r), at(&s, k.empty)), home, "{why}: undone");
        }
    }
    // Shift let go before the button: still a Shift-click.
    ex(&mut s, "select.set", json!({"ids": [k.r, k.empty]}));
    s.pointer(&ev(PointerKind::Down, 520.0, 20.0, SHIFT)).unwrap();
    s.pointer(&ev(PointerKind::Up, 520.0, 20.0, NONE)).unwrap();
    assert_eq!(s.selection(), vec![k.empty], "Shift let go before the button still takes the layer out");
}

#[test]
fn alt_shift_drag_on_a_selected_layer_duplicates_it() {
    let mut s = Session::new();
    let k = scene(&mut s);
    let layers = |s: &Session| s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len();
    let n = layers(&s);
    let alt_shift = |kind, x, y| {
        let mut e = ev(kind, x, y, SHIFT);
        e.alt = true;
        e
    };
    ex(&mut s, "select.set", json!({"ids": [k.r]}));
    s.pointer(&alt_shift(PointerKind::Down, 520.0, 20.0)).unwrap();
    s.pointer(&alt_shift(PointerKind::Up, 520.0, 20.0)).unwrap();
    assert_eq!(s.selection(), Vec::<NodeId>::new(), "an Alt+Shift-click is a Shift-click: the layer leaves the selection");
    assert_eq!(layers(&s), n, "and nothing is copied");
    ex(&mut s, "select.set", json!({"ids": [k.r]}));
    s.pointer(&alt_shift(PointerKind::Down, 520.0, 20.0)).unwrap();
    s.pointer(&alt_shift(PointerKind::Move, 560.0, 22.0)).unwrap();
    s.pointer(&alt_shift(PointerKind::Up, 560.0, 22.0)).unwrap();
    let sel = s.selection();
    assert_eq!(layers(&s), n + 1, "an Alt+Shift-drag copies the layer");
    assert!(sel.len() == 1 && sel[0] != k.r, "the copy is selected");
    let doc = s.doc().unwrap();
    assert_eq!(doc.world_bounds(sel[0]).unwrap().origin(), Point::new(540.0, 0.0), "the copy moves along one axis");
    assert_eq!(doc.world_bounds(k.r).unwrap().origin(), Point::new(500.0, 0.0), "the original stays");
}

#[test]
fn shift_click_takes_a_layer_out_where_nothing_can_move() {
    let mut s = Session::new();
    let k = scene(&mut s);
    // A version being viewed is read-only: the press can't start a move.
    let v = ex(&mut s, "file.saveVersion", json!({"name": "v"}));
    ex(&mut s, "version.preview", json!({"id": v["id"]}));
    ex(&mut s, "select.set", json!({"ids": [k.r, k.empty]}));
    assert!(s.pointer(&ev(PointerKind::Down, 520.0, 20.0, SHIFT)).is_err(), "a version being viewed can't be edited");
    s.pointer(&ev(PointerKind::Up, 520.0, 20.0, SHIFT)).unwrap();
    assert_eq!(s.selection(), vec![k.empty], "Shift-click still takes the layer out");
}
