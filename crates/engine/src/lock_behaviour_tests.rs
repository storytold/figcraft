//! Locked layers, as measured in Figma's web app (2026-11-10) and described in the help-center
//! article "Lock and unlock layers": the canvas never selects a locked layer or a child of a locked
//! frame; a locked layer selected in the layers panel has no handles and ignores the arrow keys, but
//! its properties still edit; a child of a locked frame can't be locked or unlocked on its own.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, handle_at};
use figcraft_doc::NodeId;
use figcraft_geom::Point;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn rect(s: &mut Session, x: f64, y: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": 40, "height": 40});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id_of(&ex(s, "shape.rectangle", p)["id"]).unwrap()
}

/// A frame `f` (0,0 300x200) holding rectangle `d` (20,20 40x40); a locked rectangle `lk`
/// (400,0 40x40) and an unlocked one `r` (400,100 40x40) on the page.
fn scene(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let f = id_of(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200}))["id"]).unwrap();
    let d = rect(s, 20.0, 20.0, Some(f));
    let lk = rect(s, 400.0, 0.0, None);
    let r = rect(s, 400.0, 100.0, None);
    ex(s, "object.toggleLock", json!({"ids": [lk], "locked": true}));
    (f, d, lk, r)
}

fn ev(kind: PointerKind, x: f64, y: f64, cmd: bool) -> PointerEvent {
    let mut e = PointerEvent::at(kind, x, y, 1.0);
    e.cmd = cmd;
    e
}

fn click(s: &mut Session, x: f64, y: f64, cmd: bool) {
    s.pointer(&ev(PointerKind::Down, x, y, cmd)).unwrap();
    s.pointer(&ev(PointerKind::Up, x, y, cmd)).unwrap();
}

fn origin(s: &Session, id: NodeId) -> (f64, f64) {
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    (b.x0, b.y0)
}

fn size(s: &Session, id: NodeId) -> (f64, f64) {
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    (b.width(), b.height())
}

fn locked(s: &Session, id: NodeId) -> bool {
    s.doc().unwrap().node(id).unwrap().locked
}

#[test]
fn canvas_never_selects_a_locked_layer_or_a_locked_frames_child() {
    let mut s = Session::new();
    let (f, d, _lk, _r) = scene(&mut s);
    click(&mut s, 420.0, 20.0, false);
    assert!(s.selection().is_empty(), "a click on a locked layer selects nothing");
    click(&mut s, 420.0, 20.0, true);
    assert!(s.selection().is_empty(), "Cmd/Ctrl-click on a locked layer selects nothing");
    s.pointer(&ev(PointerKind::Down, 390.0, -10.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, 450.0, 50.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Up, 450.0, 50.0, false)).unwrap();
    assert!(s.selection().is_empty(), "a marquee over a locked layer selects nothing");
    ex(&mut s, "object.toggleLock", json!({"ids": [f], "locked": true}));
    click(&mut s, 30.0, 30.0, false);
    assert!(!s.selection().contains(&d), "a click on a child of a locked frame doesn't select it");
    click(&mut s, 30.0, 30.0, true);
    assert!(s.selection().is_empty(), "Cmd/Ctrl-click on a child of a locked frame selects nothing");
}

#[test]
fn a_panel_selected_locked_layer_has_no_handles_ignores_arrows_and_takes_property_edits() {
    let mut s = Session::new();
    let (_f, _d, lk, _r) = scene(&mut s);
    ex(&mut s, "select.set", json!({"ids": [lk]}));
    assert_eq!(handle_at(s.doc().unwrap(), &[lk], Point::new(400.0, 0.0), 1.0), None, "no handles on a locked layer");
    let steps = s.undo_depth();
    for _ in 0..3 {
        ex(&mut s, "transform.nudge", json!({"dx": 1}));
    }
    assert_eq!(origin(&s, lk), (400.0, 0.0), "the arrow keys don't move a locked layer");
    ex(&mut s, "transform.nudgeSize", json!({"dw": 1}));
    assert_eq!(size(&s, lk), (40.0, 40.0), "the resize arrows don't resize a locked layer");
    assert_eq!(s.undo_depth(), steps, "an arrow that changes nothing adds no undo step");
    ex(&mut s, "node.set", json!({"id": lk, "props": {"x": 300}}));
    assert_eq!(origin(&s, lk).0, 300.0, "the X field still moves a locked layer");
    let before = format!("{:?}", s.doc().unwrap().node(lk).unwrap().fills);
    ex(&mut s, "fill.set", json!({"ids": [lk], "color": "#ff0000"}));
    assert_ne!(format!("{:?}", s.doc().unwrap().node(lk).unwrap().fills), before, "a fill still edits");
    assert_eq!(s.selection(), vec![lk], "the selection stays the locked layer");
}

#[test]
fn a_child_of_a_locked_frame_cannot_be_locked_or_unlocked_on_its_own() {
    let mut s = Session::new();
    let (f, d, _lk, _r) = scene(&mut s);
    ex(&mut s, "object.toggleLock", json!({"ids": [f], "locked": true}));
    ex(&mut s, "object.toggleLock", json!({"ids": [d]}));
    assert!(!locked(&s, d), "toggling a child of a locked frame doesn't lock the child");
    ex(&mut s, "select.set", json!({"ids": [d]}));
    ex(&mut s, "object.toggleLock", json!({}));
    assert!(!locked(&s, d), "Lock/Unlock on a panel-selected child of a locked frame does nothing");
    ex(&mut s, "object.toggleLock", json!({"ids": [f], "locked": false}));
    click(&mut s, 30.0, 30.0, false);
    assert_eq!(s.selection(), vec![d], "after unlocking the frame a click selects the child");
    assert!(!locked(&s, d));
}

#[test]
fn lock_unlock_on_a_mixed_selection_leaves_a_locked_frames_child_alone() {
    let mut s = Session::new();
    let (f, d, _lk, r) = scene(&mut s);
    ex(&mut s, "object.toggleLock", json!({"ids": [f], "locked": true}));
    ex(&mut s, "select.set", json!({"ids": [d, r]}));
    ex(&mut s, "object.toggleLock", json!({}));
    assert!(locked(&s, r), "the unlocked layer in the selection is locked");
    assert!(!locked(&s, d), "the locked frame's child keeps its own flag");
    ex(&mut s, "object.toggleLock", json!({"ids": [f], "locked": false}));
    click(&mut s, 30.0, 30.0, false);
    assert_eq!(s.selection(), vec![d], "so unlocking the frame frees the child");
}

#[test]
fn ctrl_shift_l_on_a_panel_selected_locked_layer_unlocks_it() {
    let mut s = Session::new();
    let (_f, _d, lk, _r) = scene(&mut s);
    ex(&mut s, "select.set", json!({"ids": [lk]}));
    ex(&mut s, "object.toggleLock", json!({}));
    assert!(!locked(&s, lk), "Lock/Unlock unlocks a locked layer");
    assert!(handle_at(s.doc().unwrap(), &[lk], Point::new(400.0, 0.0), 1.0).is_some(), "handles return");
}
