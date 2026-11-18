//! Shift with the Pencil draws one straight segment from where the stroke started, at the nearest
//! 45° step, as the Line tool and the Pen keep theirs (#2447).

use serde_json::json;

use crate::Session;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeKind;

fn ev(kind: PointerKind, x: f64, y: f64, shift: bool) -> PointerEvent {
    PointerEvent { shift, ..PointerEvent::at(kind, x, y, 1.0) }
}

/// Draws a wobbly stroke from (0, 0) to `end`, Shift held throughout or not, and returns the
/// stroke's size.
fn stroke(shift: bool, end: (f64, f64)) -> (f64, f64) {
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    s.pointer(&ev(PointerKind::Down, 0.0, 0.0, shift)).unwrap();
    for i in 1..=20 {
        let t = f64::from(i) / 20.0;
        s.pointer(&ev(PointerKind::Move, end.0 * t, end.1 * t + (t * 9.0).sin() * 6.0, shift)).unwrap();
    }
    if shift {
        assert_eq!(s.tool.overlay.pen.len(), 2, "the preview is one segment: {:?}", s.tool.overlay.pen);
    }
    s.pointer(&ev(PointerKind::Up, end.0, end.1, shift)).unwrap();
    let doc = s.doc().unwrap();
    let n = doc.nodes().find(|n| matches!(n.kind, NodeKind::Vector { .. })).expect("a pencil stroke");
    let b = doc.world_bounds(n.id).unwrap();
    (b.width(), b.height())
}

#[test]
fn shift_keeps_a_pencil_stroke_straight_at_45_degree_steps() {
    // Nearly level: level, as long as the pointer is far from the start.
    let (w, h) = stroke(true, (100.0, 12.0));
    assert!((w - 100.0_f64.hypot(12.0)).abs() < 0.5 && h < 0.5, "{w} x {h}");
    // Nearly diagonal: at 45°.
    let (w, h) = stroke(true, (80.0, 70.0));
    assert!((w - h).abs() < 0.5 && (w.hypot(h) - 80.0_f64.hypot(70.0)).abs() < 0.5, "{w} x {h}");
    // Without Shift the stroke follows the pointer.
    let (_, h) = stroke(false, (100.0, 12.0));
    assert!(h > 12.0, "{h}");
}

#[test]
fn a_shift_click_with_the_pencil_draws_nothing() {
    let mut s = Session::new();
    s.execute("tool.set", &json!({"tool": "pencil"})).unwrap();
    let before = s.doc().unwrap().nodes().count();
    s.pointer(&ev(PointerKind::Down, 10.0, 10.0, true)).unwrap();
    s.pointer(&ev(PointerKind::Up, 10.0, 10.0, true)).unwrap();
    assert_eq!(s.doc().unwrap().nodes().count(), before);
}
