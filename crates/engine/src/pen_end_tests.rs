//! Ending a pen path with Escape (#760): the path's vector stays in edit mode with the pen, so the
//! next path is drawn into the same layer; another Escape leaves edit mode. (Enter goes on with the
//! Move tool instead: `pen_vector_edit_tests`.)

use crate::Session;
use crate::tools::{PointerEvent, PointerKind, ToolId};
use crate::vectoredit::VTool;
use figcraft_doc::NodeKind;

fn click(s: &mut Session, x: f64, y: f64) {
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::at(kind, x, y, 1.0)).unwrap();
    }
}

fn layers(s: &Session) -> usize {
    s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len()
}

/// (points, segments) of vector `id`.
fn shape(s: &Session, id: figcraft_doc::NodeId) -> (usize, usize) {
    match &s.doc().unwrap().node(id).unwrap().kind {
        NodeKind::Vector { network } => (network.vertices.len(), network.segments.len()),
        k => panic!("not a vector: {k:?}"),
    }
}

/// A two-point pen path, ended with `end`.
fn drawn(end: fn(&mut Session)) -> (Session, figcraft_doc::NodeId) {
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    let id = s.tool.pen.as_ref().and_then(|p| p.id).unwrap();
    let undo = s.undo_depth().0;
    end(&mut s);
    assert!(s.tool.pen.is_none(), "the path ended");
    assert_eq!(s.undo_depth().0, undo + 1, "the path is one undo step");
    let ve = s.ds().unwrap().vector_edit.clone().expect("still editing the vector");
    assert_eq!((ve.id, ve.tool, ve.anchor), (id, VTool::Pen, None), "the pen, with no open end");
    (s, id)
}

#[test]
fn escape_keeps_the_vector_in_edit_mode_with_the_pen() {
    // Escape, and `pen_end` that it runs.
    let ends: [fn(&mut Session); 2] = [|s| s.escape().unwrap(), |s| s.pen_end().unwrap()];
    for end in ends {
        let (mut s, id) = drawn(end);
        // The next path goes into the same layer, unconnected to the first.
        click(&mut s, 100.0, 300.0);
        click(&mut s, 200.0, 300.0);
        assert_eq!(layers(&s), 1, "no new layer");
        assert_eq!(shape(&s, id), (4, 2), "two separate paths in one vector network");
        // Escape ends that path, the next one leaves edit mode.
        s.escape().unwrap();
        assert!(s.ds().unwrap().vector_edit.is_some());
        s.escape().unwrap();
        assert!(s.ds().unwrap().vector_edit.is_none(), "the second Escape leaves edit mode");
        assert_eq!(s.ds().unwrap().selection, vec![id]);
    }
}

#[test]
fn escape_before_a_segment_leaves_nothing_to_edit() {
    let mut s = Session::new();
    s.set_tool(ToolId::Pen);
    click(&mut s, 100.0, 100.0);
    s.escape().unwrap();
    assert!(s.tool.pen.is_none());
    assert!(s.ds().unwrap().vector_edit.is_none());
    assert_eq!(layers(&s), 0);
}
