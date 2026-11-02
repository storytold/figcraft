//! Board shapes: every outline stays in its box with room for text, and files keep reading the
//! shapes they saved.

use figcraft_geom::Shape;
use figcraft_geom::kurbo::PathEl;
use serde_json::json;

use crate::BoardShape;

#[test]
fn every_shape_has_its_own_id_and_a_name() {
    for (i, b) in BoardShape::ALL.into_iter().enumerate() {
        assert_eq!(BoardShape::from_id(b.id()), Some(b));
        assert!(!b.label().is_empty());
        assert!(BoardShape::ALL[..i].iter().all(|o| o.id() != b.id() && o.label() != b.label()), "{}", b.id());
        // The name saved in a file is the id the commands take.
        assert_eq!(serde_json::to_value(b).unwrap(), json!(b.id()));
    }
}

#[test]
fn outlines_fill_their_box_and_leave_room_for_text() {
    for (w, h) in [(160.0, 160.0), (240.0, 80.0), (12.0, 300.0)] {
        for b in BoardShape::ALL {
            let path = b.path(w, h);
            let r = path.bounding_box();
            let name = b.id();
            assert!(r.x0 >= -1e-6 && r.y0 >= -1e-6 && r.x1 <= w + 1e-6 && r.y1 <= h + 1e-6, "{name} leaves its {w}×{h} box: {r:?}");
            assert!(r.width() > w * 0.9 && r.height() > h * 0.75, "{name} doesn't fill its {w}×{h} box: {r:?}");
            assert!(path.area().abs() > w * h * 0.2, "{name} has little to fill");
        }
    }
    for b in BoardShape::ALL {
        let [t, r, bottom, l] = b.text_insets();
        assert!(t + bottom < 0.9 && l + r < 0.9, "{} leaves no room for text", b.id());
    }
}

#[test]
fn the_flowchart_symbols_draw_lines_inside_them() {
    for b in [BoardShape::Database, BoardShape::PredefinedProcess, BoardShape::InternalStorage] {
        assert!(b.path(160.0, 160.0).segments().count() > 4, "{}", b.id());
        // The lines are open: the outline closes once.
        let closes = b.path(160.0, 160.0).elements().iter().filter(|e| matches!(e, PathEl::ClosePath)).count();
        assert_eq!(closes, 1, "{}", b.id());
    }
}

/// The shapes older files saved keep their names.
#[test]
fn saved_shapes_keep_reading() {
    for (name, shape) in [
        ("rectangle", BoardShape::Rectangle),
        ("roundedRectangle", BoardShape::RoundedRectangle),
        ("ellipse", BoardShape::Ellipse),
        ("diamond", BoardShape::Diamond),
        ("triangle", BoardShape::Triangle),
        ("speechBubble", BoardShape::SpeechBubble),
        ("chevron", BoardShape::Chevron),
    ] {
        assert_eq!(serde_json::from_value::<BoardShape>(json!(name)).unwrap(), shape);
    }
}
