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

/// The four visible grips in paint order, without relying on the hit-test geometry.
fn painted_grips(output: &egui::FullOutput) -> Vec<Rect> {
    crate::live_tests::flat(&output.shapes)
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(r)
                if r.fill == super::PINK.gamma_multiply(0.6)
                    && ((r.rect.width() == 16.0 && r.rect.height() == 4.0) || (r.rect.width() == 4.0 && r.rect.height() == 16.0)) =>
            {
                Some(r.rect)
            }
            _ => None,
        })
        .collect()
}

/// Even collapsed content boxes and reduced zoom leave each painted grip independently usable.
#[test]
fn tiny_frames_keep_every_painted_padding_grip_reachable() {
    for (width, height, zoom, stroke) in [
        (300.0, 4.0, 1.0, 0.0),
        (300.0, 6.0, 1.0, 0.0),
        (300.0, 10.0, 1.0, 0.0),
        (300.0, 20.0, 0.25, 0.0),
        (300.0, 20.0, 1.0, 8.0),
        (4.0, 200.0, 1.0, 0.0),
        (4.0, 4.0, 0.25, 0.0),
    ] {
        for side in 0_usize..4 {
            let (mut u, f) = scene();
            u.app.run("node.set", json!({"ids": [f], "props": {"width": width, "height": height, "padding": 0, "strokeWeight": stroke}}));
            u.app.session.ds_mut().unwrap().view.zoom = zoom;
            let grips = painted_grips(&u.frame_output(vec![Event::PointerGone]));
            assert_eq!(grips.len(), 4);
            let center = grips[side].center();
            let output = u.frame_output(vec![Event::PointerMoved(center)]);
            assert_eq!(
                output.platform_output.cursor_icon,
                if side.is_multiple_of(2) { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal }
            );
            u.button(center, true);
            assert!(
                matches!(u.ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Padding { side: active, .. }) if active == side),
                "{width}x{height}, zoom {zoom}, stroke {stroke}, side {side}"
            );
            let to = center + [vec2(0.0, 2.0), vec2(-2.0, 0.0), vec2(0.0, -2.0), vec2(2.0, 0.0)][side] * zoom as f32;
            u.frame(vec![Event::PointerMoved(to)]);
            u.button(to, false);
            let mut expected = [0.0; 4];
            expected[side] = 2.0;
            assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().padding, expected);
            u.app.run("edit.undo", json!({}));
            assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().padding, [0.0; 4]);
        }
    }
}

/// A painted side grip wins over a neighbouring grip's enlarged, overlapping hit target.
#[test]
fn overlapping_padding_targets_use_the_nearest_grip_for_hover_and_press() {
    let (mut u, f) = scene();
    u.app.run("node.set", json!({"ids": [f], "props": {"width": 4, "height": 4, "padding": 0, "strokeWeight": 0}}));
    let grips = painted_grips(&u.frame_output(vec![Event::PointerGone]));
    assert_eq!(grips.len(), 4);
    // On the upper part of the right bar, also inside the top grip's larger hit target.
    let point = egui::pos2(grips[1].center().x - 1.0, grips[0].center().y + 5.0);
    assert!(grips[1].contains(point));
    let handles = super::padding_handles(&al_zones(&u.app).unwrap().bands);
    assert_eq!(handles.iter().filter(|(_, r, _)| r.contains(point)).count(), 2);
    let output = u.frame_output(vec![Event::PointerMoved(point)]);
    assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::ResizeHorizontal);
    let shapes = crate::live_tests::flat(&output.shapes);
    assert!(shapes.iter().any(|shape| matches!(shape, egui::Shape::Rect(r) if r.rect == grips[1] && r.fill == super::PINK)));
    assert!(!shapes.iter().any(|shape| matches!(shape, egui::Shape::Rect(r) if r.rect == grips[0] && r.fill == super::PINK)));
    u.button(point, true);
    assert!(matches!(u.ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Padding { side: 1, .. })));
    u.button(point, false);
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

/// Hover over a grip must still update the layer under the pointer without editing the document.
#[test]
fn padding_grip_hover_preserves_canvas_hover_and_resize_cursor() {
    let (mut u, f) = scene();
    let other = id_of(&u.app.session.execute("shape.rectangle", &json!({"x": 500, "y": 100, "width": 40, "height": 40})).unwrap()["id"]).unwrap();
    u.app.run("select.set", json!({"ids": [f]}));
    let over_other = u.app.to_screen(520.0, 120.0);
    u.frame(vec![Event::PointerMoved(over_other)]);
    assert_eq!(u.app.session.ds().unwrap().hover, Some(other));
    let history = u.app.session.undo_depth();
    let grip = al_zones(&u.app).unwrap().bands[0].1.center();
    let output = u.frame_output(vec![Event::PointerMoved(grip)]);
    assert_eq!(u.app.session.ds().unwrap().hover, Some(f), "the previous layer must not stay hovered");
    assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::ResizeVertical);
    assert_eq!(u.app.session.undo_depth(), history);
    assert!(u.ctx.data(|d| d.get_temp::<Drag>(drag_id())).is_none());
}

/// Right-click on any padding grip opens the ordinary canvas menu without starting a drag.
#[test]
fn padding_grips_preserve_secondary_click_context_menus() {
    for side in 0..4 {
        let (mut u, f) = scene();
        let center = al_zones(&u.app).unwrap().bands[side].1.center();
        let history = u.app.session.undo_depth();
        u.frame(vec![Event::PointerMoved(center)]);
        for pressed in [true, false] {
            u.frame(vec![Event::PointerButton { pos: center, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() }]);
        }
        let (_, targets) = u.app.ui.context_menu.as_ref().expect("right-click on the grip opens the menu");
        assert!(targets.contains(&f));
        assert!(u.ctx.data(|d| d.get_temp::<Drag>(drag_id())).is_none());
        assert_eq!(u.app.session.undo_depth(), history);
    }
}

/// Custom light/dark palettes must control both grip states and their outlines.
#[test]
fn padding_grips_use_the_theme_fill_and_outline() {
    for mut tokens in [crate::theme::Tokens::light(), crate::theme::Tokens::dark()] {
        let (mut u, _) = scene();
        let grips = painted_grips(&u.frame_output(vec![Event::PointerGone]));
        assert_eq!(grips.len(), 4);
        tokens.padding_grip = if tokens.dark { egui::Color32::from_rgb(20, 40, 60) } else { egui::Color32::from_rgb(120, 40, 60) };
        tokens.padding_grip_border = egui::Color32::from_rgb(210, 180, 150);
        for active in [false, true] {
            let events = if active { vec![Event::PointerMoved(grips[0].center())] } else { vec![Event::PointerGone] };
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, u.screen)), events, ..Default::default() };
            let mut output = u.ctx.clone().run_ui(raw, |ui| {
                crate::theme::apply(ui.ctx(), tokens);
                super::draw(&mut u.app, ui.painter());
            });
            output.textures_delta.clear();
            let shapes = crate::live_tests::flat(&output.shapes);
            for (side, grip) in grips.iter().enumerate() {
                let linked = active && side == 0;
                let fill = if linked { tokens.padding_grip } else { tokens.padding_grip.gamma_multiply(0.6) };
                assert!(shapes.iter().any(|shape| matches!(shape, egui::Shape::Rect(r) if r.rect == *grip && r.fill == fill)));
                let width = if linked { 1.5 } else { 1.0 };
                assert!(shapes.iter().any(|shape| matches!(shape, egui::Shape::Rect(r) if r.rect == *grip && r.stroke.color == tokens.padding_grip_border && r.stroke.width == width)));
            }
        }
    }
}
