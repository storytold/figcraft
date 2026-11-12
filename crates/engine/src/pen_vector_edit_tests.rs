//! From the pen into vector edit mode (#1430): a double-click or Enter while the pen is out ends
//! the path and edits the vector with the Move tool's point handles, instead of drawing on (a
//! double-click used to leave a zero-length segment, or a stray two-point vector on the layer it
//! was meant to edit). Escape still keeps the pen (#760).

use serde_json::json;

use crate::Session;
use crate::tools::{PointerEvent, PointerKind, ToolId};
use crate::vectoredit::VTool;
use figcraft_doc::{NodeId, NodeKind};

fn press(s: &mut Session, kinds: &[PointerKind], x: f64, y: f64) {
    for kind in kinds {
        s.pointer(&PointerEvent::at(*kind, x, y, 1.0)).unwrap();
    }
}

fn click(s: &mut Session, x: f64, y: f64) {
    press(s, &[PointerKind::Down, PointerKind::Up], x, y);
}

/// A double-click as the canvas sends it: two clicks, then the double-click.
fn double_click(s: &mut Session, x: f64, y: f64) {
    press(s, &[PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick], x, y);
}

fn layers(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().node(s.page().unwrap()).unwrap().children.clone()
}

/// (points, segments) of vector `id`.
fn shape(s: &Session, id: NodeId) -> (usize, usize) {
    match &s.doc().unwrap().node(id).unwrap().kind {
        NodeKind::Vector { network } => (network.vertices.len(), network.segments.len()),
        k => panic!("not a vector: {k:?}"),
    }
}

/// The layer and tool vector edit mode has (with no pen path open).
fn editing(s: &Session) -> Option<(NodeId, VTool)> {
    s.ds().unwrap().vector_edit.as_ref().filter(|v| v.anchor.is_none()).map(|v| (v.id, v.tool))
}

/// A pen path from (100, 100) to (200, 100), still being drawn; its layer.
fn drawing() -> (Session, NodeId) {
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let id = s.tool.pen.as_ref().and_then(|p| p.id).unwrap();
    (s, id)
}

#[test]
fn a_double_click_with_the_pen_on_a_layer_edits_it_instead_of_drawing() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    let [rect] = <[NodeId; 1]>::try_from(layers(&s)).unwrap();
    s.set_tool(ToolId::Pen);
    double_click(&mut s, 150.0, 100.0);
    assert_eq!(layers(&s), vec![rect], "no stray vector drawn");
    assert_eq!(editing(&s), Some((rect, VTool::Move)), "its points are edited with the Move tool");
    assert!(s.tool.pen.is_none());
    assert_eq!(s.tool.tool, ToolId::Move);
    // Off any layer the double-click draws nothing either.
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    double_click(&mut s, 400.0, 400.0);
    assert!(layers(&s).is_empty() && s.tool.pen.is_none() && editing(&s).is_none());
}

#[test]
fn a_double_click_while_drawing_ends_the_path_there_and_edits_it() {
    let (mut s, id) = drawing();
    let undo = s.undo_depth().0;
    double_click(&mut s, 200.0, 200.0);
    assert_eq!(shape(&s, id), (3, 2), "the double-click's point once, no zero-length segment");
    assert!(s.tool.pen.is_none());
    assert_eq!(editing(&s), Some((id, VTool::Move)));
    assert_eq!(s.undo_depth().0, undo + 1, "the path is one undo step");
}

#[test]
fn enter_while_drawing_ends_the_path_and_edits_it_with_the_move_tool() {
    let (mut s, id) = drawing();
    assert!(s.tool_key("Enter", false, false, false).unwrap());
    assert!(s.tool.pen.is_none());
    assert_eq!(editing(&s), Some((id, VTool::Move)));
    assert_eq!(shape(&s, id), (2, 1));
    // The next Enter leaves edit mode.
    assert!(s.tool_key("Enter", false, false, false).unwrap());
    assert!(s.ds().unwrap().vector_edit.is_none());
    assert_eq!(s.selection(), vec![id]);
}

#[test]
fn in_edit_mode_the_pen_goes_to_the_move_tool_on_a_double_click_or_enter() {
    // Escape keeps the pen in edit mode (#760); a double-click on the path then edits it rather
    // than adding a point where it landed.
    let (mut s, id) = drawing();
    s.escape().unwrap();
    assert_eq!(editing(&s), Some((id, VTool::Pen)));
    double_click(&mut s, 150.0, 100.0);
    assert_eq!(editing(&s), Some((id, VTool::Move)));
    assert_eq!(shape(&s, id), (2, 1), "no point split into the segment");
    // Off the path: no lone point left behind.
    let (mut s, id) = drawing();
    s.escape().unwrap();
    double_click(&mut s, 150.0, 300.0);
    assert_eq!((editing(&s), shape(&s, id)), (Some((id, VTool::Move)), (2, 1)));
    // Enter ends the pen's path in edit mode and takes the Move tool; the next one leaves.
    let (mut s, id) = drawing();
    s.escape().unwrap();
    click(&mut s, 150.0, 300.0);
    click(&mut s, 250.0, 300.0);
    assert!(s.tool_key("Enter", false, false, false).unwrap());
    assert_eq!((editing(&s), shape(&s, id)), (Some((id, VTool::Move)), (4, 2)));
    assert!(s.tool_key("Enter", false, false, false).unwrap());
    assert!(s.ds().unwrap().vector_edit.is_none());
}
