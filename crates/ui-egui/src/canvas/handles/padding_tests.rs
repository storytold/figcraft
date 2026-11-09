//! Padding bands follow the content box, including the frame's inside stroke.

use egui::{Event, Rect, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::{Session, params::id_of};
use serde_json::json;

use super::{Drag, al_zones, drag_id};
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

#[test]
fn padding_drag_starts_only_on_the_central_handle_and_undoes_in_one_step() {
    let (mut u, f) = scene();
    u.app.run("node.set", json!({"ids": [f], "props": {"strokeWeight": 24}}));
    u.frame(vec![]);
    let top = al_zones(&u.app).unwrap().bands[0].1;
    // The band away from its handle remains available to ordinary canvas tools.
    let away = top.center() + vec2(50.0, 0.0);
    u.button(away, true);
    assert!(!matches!(u.ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Padding { .. })));
    u.button(away, false);
    u.app.run("select.set", json!({"ids": [f]}));
    u.frame(vec![]);

    let center = al_zones(&u.app).unwrap().bands[0].1.center();
    let zoom = u.app.session.ds().unwrap().view.zoom as f32;
    u.button(center, true);
    assert!(matches!(u.ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Padding { side: 0, .. })));
    let to = center + vec2(0.0, 6.0 * zoom);
    u.frame(vec![Event::PointerMoved(to)]);
    u.button(to, false);
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().padding, [16.0, 20.0, 30.0, 40.0]);
    u.app.run("edit.undo", json!({}));
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().padding, [10.0, 20.0, 30.0, 40.0]);
}

#[test]
fn every_padding_grip_has_a_resize_cursor_and_links_the_requested_sides() {
    for modifiers in
        [egui::Modifiers::NONE, egui::Modifiers::ALT, egui::Modifiers::SHIFT, egui::Modifiers { alt: true, shift: true, ..Default::default() }]
    {
        for side in 0..4 {
            let (mut u, f) = scene();
            let center = al_zones(&u.app).unwrap().bands[side].1.center();
            let output = u.frame_with(vec![Event::PointerMoved(center)], modifiers);
            let cursor = if side.is_multiple_of(2) { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal };
            assert_eq!(output.platform_output.cursor_icon, cursor, "padding side {side}");
            u.button(center, true);
            assert!(matches!(u.ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Padding { side: active, .. }) if active == side));
            let zoom = u.app.session.ds().unwrap().view.zoom as f32;
            let delta = [vec2(0.0, 6.0), vec2(-6.0, 0.0), vec2(0.0, -6.0), vec2(6.0, 0.0)][side] * zoom;
            let to = center + delta;
            u.frame_with(vec![Event::PointerMoved(to)], modifiers);
            u.button(to, false);
            let mut expected = [10.0, 20.0, 30.0, 40.0];
            let value = expected[side] + 6.0;
            if modifiers.alt && modifiers.shift {
                expected = [value; 4];
            } else {
                expected[side] = value;
                if modifiers.alt || modifiers.shift {
                    expected[(side + 2) % 4] = value;
                }
            }
            assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().padding, expected);
            u.app.run("edit.undo", json!({}));
            assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().padding, [10.0, 20.0, 30.0, 40.0]);
        }
    }
}

#[test]
fn modifier_keys_highlight_linked_padding_bands_before_and_during_a_drag() {
    let (mut u, _) = scene();
    let zones = al_zones(&u.app).unwrap();
    let center = zones.bands[0].1.center();
    u.frame(vec![Event::PointerMoved(center)]);
    // Pressing modifiers without moving the pointer previews the other affected sides.
    for (modifiers, active) in [
        (egui::Modifiers::NONE, [true, false, false, false]),
        (egui::Modifiers::ALT, [true, false, true, false]),
        (egui::Modifiers { alt: true, shift: true, ..Default::default() }, [true; 4]),
        (egui::Modifiers::NONE, [true, false, false, false]),
    ] {
        let output = u.frame_with(vec![], modifiers);
        let shapes = crate::live_tests::flat(&output.shapes);
        for ((side, rect, _), active) in zones.bands.iter().zip(active) {
            let lit = shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.rect == *rect && r.fill == super::PINK.gamma_multiply(0.25)));
            assert_eq!(lit, active, "side {side}, modifiers {modifiers:?}");
        }
    }
    u.button(center, true);
    let modifiers = egui::Modifiers { alt: true, shift: true, ..Default::default() };
    let output = u.frame_with(vec![], modifiers);
    let zones = al_zones(&u.app).unwrap();
    let shapes = crate::live_tests::flat(&output.shapes);
    for (_, rect, _) in &zones.bands {
        assert!(shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.rect == *rect && r.fill == super::PINK.gamma_multiply(0.25))));
    }
    u.button(center, false);
}
