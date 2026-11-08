//! Undo while the pen draws (#1745): each undo takes back the path's last anchor, not the whole
//! path (nor the step before it); redo puts the anchors back.

use crate::Session;
use crate::tools::{PointerEvent, PointerKind, ToolId};
use figcraft_doc::NodeKind;
use serde_json::json;

fn click(s: &mut Session, x: f64, y: f64) {
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::at(kind, x, y, 1.0)).unwrap();
    }
}

fn layers(s: &Session) -> usize {
    s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len()
}

fn pen_points(s: &Session) -> Option<usize> {
    s.tool.pen.as_ref().map(|p| p.points.len())
}

/// (vertices, segments) of the pen's vector, if it has one.
fn vector(s: &Session) -> Option<(usize, usize)> {
    let id = s.tool.pen.as_ref().and_then(|p| p.id)?;
    match &s.doc().unwrap().node(id)?.kind {
        NodeKind::Vector { network } => Some((network.vertices.len(), network.segments.len())),
        _ => None,
    }
}

fn undo(s: &mut Session) -> bool {
    s.execute("edit.undo", &json!({})).unwrap()["undone"] == true
}

fn redo(s: &mut Session) {
    s.execute("edit.redo", &json!({})).unwrap();
}

#[test]
fn undo_while_drawing_takes_back_one_anchor() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 400, "y": 400, "width": 50, "height": 50})).unwrap();
    assert_eq!(layers(&s), 1);
    s.set_tool(ToolId::Pen);
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    click(&mut s, 200.0, 200.0);
    assert_eq!((pen_points(&s), vector(&s), layers(&s)), (Some(3), Some((3, 2)), 2));

    assert!(undo(&mut s));
    assert_eq!((pen_points(&s), vector(&s), layers(&s)), (Some(2), Some((2, 1)), 2), "only the last anchor goes");
    assert!(undo(&mut s));
    assert_eq!((pen_points(&s), vector(&s), layers(&s)), (Some(1), None, 1), "one anchor, no layer yet");

    // Redo puts the anchors back in order.
    redo(&mut s);
    assert_eq!((pen_points(&s), vector(&s), layers(&s)), (Some(2), Some((2, 1)), 2));
    redo(&mut s);
    assert_eq!((pen_points(&s), vector(&s), layers(&s)), (Some(3), Some((3, 2)), 2));
    let pts: Vec<_> = s.tool.pen.as_ref().unwrap().points.iter().map(|(p, _, _)| (p.x, p.y)).collect();
    assert_eq!(pts, vec![(100.0, 100.0), (200.0, 100.0), (200.0, 200.0)]);

    for _ in 0..3 {
        assert!(undo(&mut s));
    }
    assert_eq!((pen_points(&s), layers(&s)), (None, 1), "the path is gone, the rectangle stays");
    assert!(undo(&mut s));
    assert_eq!(layers(&s), 0, "the next undo takes the rectangle");
}
