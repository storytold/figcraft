//! The cursor the canvas shows: the translation of every engine cursor into a system cursor or a
//! glyph drawn at the pointer, and (driven through egui headless) what the window asks for with
//! each tool, while panning or zooming, over a ruler guide, and over a layers row that takes no
//! drop.

use egui::{CursorIcon, Event, Key, Modifiers, Pos2, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use figcraft_engine::tools::{Cursor, PenCursor, ToolId};
use serde_json::json;

use crate::cursors::{Glyph, Shown, of, on_canvas, resize_icon};
use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

#[test]
fn every_engine_cursor_has_a_system_cursor_or_a_glyph() {
    use Shown::{Drawn, System};
    // (engine cursor, what shows, its name in `ui.inspect`)
    let rows: Vec<(Cursor, Shown, &str)> = vec![
        (Cursor::Default, System(CursorIcon::Default), "default"),
        (Cursor::Crosshair, Drawn(Glyph::Crosshair), "crosshair"),
        (Cursor::Move, System(CursorIcon::Move), "move"),
        (Cursor::Resize(0.0), System(CursorIcon::ResizeHorizontal), "ew-resize"),
        (Cursor::Resize(90.0), System(CursorIcon::ResizeVertical), "ns-resize"),
        (Cursor::Resize(45.0), System(CursorIcon::ResizeNwSe), "nwse-resize"),
        (Cursor::Resize(135.0), System(CursorIcon::ResizeNeSw), "nesw-resize"),
        (Cursor::Rotate(45.0), Drawn(Glyph::Rotate(45.0)), "rotate 45"),
        (Cursor::Rotate(-45.0), Drawn(Glyph::Rotate(315.0)), "rotate 315"),
        (Cursor::Grab, System(CursorIcon::Grab), "grab"),
        (Cursor::Grabbing, System(CursorIcon::Grabbing), "grabbing"),
        (Cursor::Text, System(CursorIcon::Text), "text"),
        (Cursor::Pen(PenCursor::New), Drawn(Glyph::Pen(PenCursor::New)), "pen"),
        (Cursor::Pen(PenCursor::Close), Drawn(Glyph::Pen(PenCursor::Close)), "pen-close"),
        (Cursor::Pen(PenCursor::Add), Drawn(Glyph::Pen(PenCursor::Add)), "pen-add"),
        (Cursor::Pen(PenCursor::Bend), Drawn(Glyph::Pen(PenCursor::Bend)), "pen-bend"),
        (Cursor::Copy, System(CursorIcon::Copy), "copy"),
        (Cursor::Pointer, System(CursorIcon::PointingHand), "pointer"),
        (Cursor::Eyedropper, Drawn(Glyph::Eyedropper), "eyedropper"),
        (Cursor::Comment, Drawn(Glyph::Comment), "comment"),
        (Cursor::NotAllowed, System(CursorIcon::NotAllowed), "not-allowed"),
    ];
    for (cursor, shown, name) in rows {
        assert_eq!(of(cursor), shown, "{cursor:?}");
        assert_eq!(shown.name(), name, "{cursor:?}");
        // A drawn cursor hides the system's; a system cursor is never "other".
        assert_eq!(shown.icon() == CursorIcon::None, matches!(shown, Drawn(_)), "{cursor:?}");
        assert_ne!(name, "other");
    }
    // Every tool's own cursor is in the table (none falls through to a cursor without a name).
    for t in ToolId::ALL {
        assert_ne!(of(t.cursor()).name(), "other", "{t:?}");
    }
    // Junk angles don't break it.
    assert_eq!(of(Cursor::Resize(f64::NAN)), System(CursorIcon::Default));
    assert_eq!(of(Cursor::Rotate(f64::INFINITY)), Drawn(Glyph::Rotate(0.0)));
}

#[test]
fn the_crosshair_is_drawn_dark_with_a_white_edge_so_it_shows_on_white() {
    use egui::epaint::Shape;
    let shown = of(Cursor::Crosshair);
    assert_eq!(shown, Shown::Drawn(Glyph::Crosshair));
    // The system's pointer is hidden under it, whatever its colour.
    assert_eq!(shown.icon(), CursorIcon::None);
    let at = pos2(50.0, 40.0);
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let ctx = ui.ctx().clone();
        let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("crosshair_test"));
        crate::cursors::paint(&ctx.layer_painter(layer), at, Glyph::Crosshair);
    });
    out.textures_delta.clear();
    let mut strokes = Vec::new();
    for clipped in &out.shapes {
        if let Shape::LineSegment { points, stroke } = &clipped.shape {
            strokes.push((points.to_vec(), stroke.color, stroke.width));
        }
    }
    let dark: Vec<_> = strokes.iter().filter(|s| s.1 == crate::cursors::INK).collect();
    let white: Vec<_> = strokes.iter().filter(|s| s.1 == egui::Color32::WHITE).collect();
    assert!(!dark.is_empty() && !white.is_empty(), "{strokes:?}");
    // The white edge is wider than the dark line and is painted first.
    assert!(white.iter().all(|w| w.2 > dark[0].2), "{strokes:?}");
    assert_eq!(strokes[0].1, egui::Color32::WHITE);
    // The arms are centred on the pointer and leave its pixel clear.
    for (pts, ..) in &dark {
        for p in pts {
            let d = (p.x - at.x).abs().max((p.y - at.y).abs());
            assert!((2.0..=8.0).contains(&d), "{p:?}");
            assert!((p.x - at.x).abs() < 0.01 || (p.y - at.y).abs() < 0.01, "{p:?}");
        }
    }
    assert_eq!(dark.len(), 4);
}

#[test]
fn resize_arrows_follow_the_angle_in_eighths() {
    // (degrees, the two-headed arrow): each covers 45° about its own direction, any turn.
    let rows = [
        (0.0, CursorIcon::ResizeHorizontal),
        (22.0, CursorIcon::ResizeHorizontal),
        (23.0, CursorIcon::ResizeNwSe),
        (67.0, CursorIcon::ResizeNwSe),
        (68.0, CursorIcon::ResizeVertical),
        (112.0, CursorIcon::ResizeVertical),
        (113.0, CursorIcon::ResizeNeSw),
        (157.0, CursorIcon::ResizeNeSw),
        (158.0, CursorIcon::ResizeHorizontal),
        (180.0, CursorIcon::ResizeHorizontal),
        (225.0, CursorIcon::ResizeNwSe),
        (-45.0, CursorIcon::ResizeNeSw),
        (-90.0, CursorIcon::ResizeVertical),
        (720.0 + 30.0, CursorIcon::ResizeNwSe),
    ];
    for (a, want) in rows {
        assert_eq!(resize_icon(a), want, "{a}°");
    }
}

#[test]
fn panning_and_the_zoom_tool_come_before_the_tools_cursor() {
    use Shown::System;
    // (panning, a button down, zoom tool, tool cursor) → shown
    let rows = [
        ((true, false, None, Cursor::Crosshair), System(CursorIcon::Grab)),
        ((true, true, None, Cursor::Crosshair), System(CursorIcon::Grabbing)),
        ((true, true, Some(false), Cursor::Default), System(CursorIcon::Grabbing)),
        ((false, false, Some(false), Cursor::Crosshair), System(CursorIcon::ZoomIn)),
        ((false, true, Some(true), Cursor::Default), System(CursorIcon::ZoomOut)),
        ((false, true, None, Cursor::Copy), System(CursorIcon::Copy)),
    ];
    for ((pan, down, zoom, cursor), want) in rows {
        assert_eq!(on_canvas(pan, down, zoom, cursor), want, "pan {pan}, down {down}, zoom {zoom:?}, {cursor:?}");
    }
}

/// A frame with the pointer at `p` and `keys` held; the cursor the window asked for and the
/// canvas's own.
fn at(u: &mut Ui, p: Pos2, keys: &[Key], modifiers: Modifiers) -> (CursorIcon, Option<String>) {
    let mut events = vec![Event::PointerMoved(p)];
    events.extend(keys.iter().map(|key| Event::Key { key: *key, physical_key: None, pressed: true, repeat: false, modifiers }));
    u.frame_with(events, modifiers);
    // Once more, settled (the tool sees the pointer where it is).
    let out = u.frame_with(vec![Event::PointerMoved(p)], modifiers);
    let shown = u.app.canvas.cursor.map(Shown::name);
    let released: Vec<Event> =
        keys.iter().map(|key| Event::Key { key: *key, physical_key: None, pressed: false, repeat: false, modifiers }).collect();
    u.frame_with(released, modifiers);
    (out.platform_output.cursor_icon, shown)
}

#[test]
fn the_window_asks_for_the_cursor_of_the_tool_under_the_pointer() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    let rect = u.app.session.doc().unwrap().world_bounds(r).unwrap();
    let on = u.app.to_screen(rect.center().x, rect.center().y);
    let edge = u.app.to_screen(rect.x1, rect.center().y);
    let empty = u.app.to_screen(rect.x1 + 80.0 / u.app.session.ds().unwrap().view.zoom, rect.y1);
    let no = Modifiers::NONE;
    let alt = Modifiers { alt: true, ..Modifiers::NONE };
    // (tool, where, keys held, modifiers) → (system cursor, the canvas's cursor by name)
    type Row<'a> = (&'a str, Pos2, &'a [Key], Modifiers, CursorIcon, &'a str);
    let rows: Vec<Row> = vec![
        ("move", empty, &[], no, CursorIcon::Default, "default"),
        ("move", edge, &[], no, CursorIcon::ResizeHorizontal, "ew-resize"),
        ("move", on, &[], alt, CursorIcon::Copy, "copy"),
        ("hand", empty, &[], no, CursorIcon::Grab, "grab"),
        ("rectangle", empty, &[], no, CursorIcon::None, "crosshair"),
        ("frame", on, &[], no, CursorIcon::None, "crosshair"),
        ("slice", empty, &[], no, CursorIcon::None, "crosshair"),
        ("text", empty, &[], no, CursorIcon::None, "crosshair"),
        ("pen", empty, &[], no, CursorIcon::None, "pen"),
        ("comment", empty, &[], no, CursorIcon::None, "comment"),
        ("move", empty, &[Key::Space], no, CursorIcon::Grab, "grab"),
        ("rectangle", empty, &[Key::Space], no, CursorIcon::Grab, "grab"),
        ("move", empty, &[Key::Z], no, CursorIcon::ZoomIn, "zoom-in"),
        ("move", empty, &[Key::Z], alt, CursorIcon::ZoomOut, "zoom-out"),
    ];
    for (tool, p, keys, mods, icon, name) in rows {
        u.app.run("tool.set", json!({"tool": tool}));
        let (got, shown) = at(&mut u, p, keys, mods);
        assert_eq!((got, shown.as_deref()), (icon, Some(name)), "{tool} with {keys:?} {mods:?}");
    }
    // `ui.inspect` reports it while the pointer is over the canvas, and nothing over a panel.
    u.app.run("edit.pickColor", json!({}));
    u.frame(vec![Event::PointerMoved(empty)]);
    u.frame(vec![Event::PointerMoved(empty)]);
    assert_eq!(u.app.inspect()["cursor"], "eyedropper");
    u.frame(vec![Event::PointerMoved(pos2(100.0, 300.0))]);
    assert_eq!(u.app.inspect()["cursor"], json!(null), "over the layers panel");
}

#[test]
fn a_ruler_guide_keeps_its_own_cursor_under_the_tools() {
    let mut s = Session::new();
    s.execute("view.rulers", &json!({"on": true})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "offset": 40})).unwrap();
    let mut u = Ui::new(s);
    let c = u.app.canvas.rect.center();
    let on_guide = pos2(u.app.to_screen(40.0, 0.0).x, c.y);
    let away = pos2(on_guide.x + 120.0, c.y);
    u.frame(vec![Event::PointerMoved(on_guide)]);
    let out = u.frame_output(vec![Event::PointerMoved(on_guide)]);
    assert_eq!(out.platform_output.cursor_icon, CursorIcon::ResizeHorizontal, "over a guide: the guide's arrows");
    assert!(u.app.canvas.cursor.is_none(), "the tool's cursor stays out of it");
    u.frame(vec![Event::PointerMoved(away)]);
    let out = u.frame_output(vec![Event::PointerMoved(away)]);
    assert_eq!(out.platform_output.cursor_icon, CursorIcon::Default);
    assert_eq!(u.app.canvas.cursor.map(Shown::name).as_deref(), Some("default"));
}

#[test]
fn a_layers_row_that_takes_no_drop_shows_not_allowed() {
    let mut s = Session::new();
    let frame = id(s.execute("node.create", &json!({"type": "frame", "x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let child = id(s.execute("node.create", &json!({"type": "ellipse", "parent": frame, "x": 10, "y": 10, "width": 10, "height": 10})).unwrap());
    let other = id(s.execute("node.create", &json!({"type": "rectangle", "x": 200, "y": 0, "width": 10, "height": 10})).unwrap());
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.expanded.insert(frame);
    u.frame(vec![]);
    let (f, c, o) = (u.row_at(frame), u.row_at(child), u.row_at(other));
    // The frame dragged over its own child: no drop there.
    u.drag_to(f, c);
    let out = u.frame_output(vec![Event::PointerMoved(c)]);
    assert_eq!(out.platform_output.cursor_icon, CursorIcon::NotAllowed);
    // Over another layer's row: the grabbing hand of a drag that can land.
    let out = u.frame_output(vec![Event::PointerMoved(o)]);
    assert_eq!(out.platform_output.cursor_icon, CursorIcon::Grabbing);
    u.frame(vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    u.button(o, false);
}
