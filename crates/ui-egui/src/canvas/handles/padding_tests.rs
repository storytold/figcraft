//! Padding bands follow the content box, including the frame's inside stroke.

use egui::Rect;
use figcraft_doc::NodeId;
use figcraft_engine::{Session, params::id_of};
use serde_json::json;

use super::al_zones;
use crate::layers_drag_tests::Ui;

fn scene() -> (Ui, NodeId) {
    let mut s = Session::new();
    let f = id_of(
        &s.execute(
            "frame.create",
            &json!({"x": 100, "y": 100, "width": 300, "height": 200,
                    "props": {"layout": "horizontal", "gap": 10, "padding": [10, 20, 30, 40], "stroke": "#000000", "strokeWeight": 4, "strokeAlign": "inside"}
            }),
        )
        .unwrap()["id"],
    )
    .unwrap();
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    (Ui::new(s), f)
}

#[test]
fn padding_bands_follow_uniform_and_individual_inside_strokes() {
    let (mut u, f) = scene();
    for (props, stroke) in [
        (json!({"strokeWeight": 4}), [4.0; 4]),
        (json!({"strokeWeight": 24}), [24.0; 4]),
        (json!({"strokeSides": [8, 12, 16, 24]}), [8.0, 12.0, 16.0, 24.0]),
        (json!({"strokesIncluded": false}), [0.0; 4]),
        (json!({"strokesIncluded": true, "strokeAlign": "center"}), [0.0; 4]),
        (json!({"strokeAlign": "outside"}), [0.0; 4]),
    ] {
        u.app.run("node.set", json!({"ids": [f], "props": props}));
        let [t, r, b, l] = stroke;
        let expected = [
            (l, t, 300.0 - r, t + 10.0),
            (300.0 - r - 20.0, t, 300.0 - r, 200.0 - b),
            (l, 200.0 - b - 30.0, 300.0 - r, 200.0 - b),
            (l, t, l + 40.0, 200.0 - b),
        ];
        let zones = al_zones(&u.app).unwrap();
        for ((side, band, value), (x0, y0, x1, y1)) in zones.bands.iter().zip(expected) {
            let expected = Rect::from_two_pos(u.app.to_screen(100.0 + x0, 100.0 + y0), u.app.to_screen(100.0 + x1, 100.0 + y1));
            assert_eq!(*band, expected, "side {side}, stroke {stroke:?}");
            assert_eq!(*value, [10.0, 20.0, 30.0, 40.0][*side]);
        }
    }
}

#[test]
fn gap_bands_span_the_content_box_inside_stroke_and_padding() {
    let (mut u, f) = scene();
    for _ in 0..2 {
        u.app.run("shape.rectangle", json!({"parent": f, "width": 40, "height": 20}));
    }
    u.app.run("select.set", json!({"ids": [f]}));
    for (mode, expected) in [("horizontal", (184.0, 114.0, 194.0, 266.0)), ("vertical", (144.0, 134.0, 376.0, 144.0))] {
        u.app.run("node.set", json!({"ids": [f], "props": {"layout": mode}}));
        let zones = al_zones(&u.app).unwrap();
        assert_eq!(zones.gaps.len(), 1);
        let (x0, y0, x1, y1) = expected;
        assert_eq!(zones.gaps[0].rect, Rect::from_two_pos(u.app.to_screen(x0, y0), u.app.to_screen(x1, y1)));
    }
}
