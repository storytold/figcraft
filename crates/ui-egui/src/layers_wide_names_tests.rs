//! Long and deeply nested names in the Layers panel (#1904), drawn headless: the list is as wide
//! as its widest name needs and scrolls sideways, so a name reads in full without widening the
//! sidebar, as the reference app's Layers panel does with its horizontal scrollbar; names that
//! fit leave the list as wide as the panel; a name past the width cap (#1650) still elides.

use egui::{Event, MouseWheelUnit, Pos2, Shape, TouchPhase, Vec2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A rectangle named `name` on the page, or inside `parent`.
pub(crate) fn named(s: &mut Session, name: &str, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": 0, "y": 0, "width": 20, "height": 20, "props": {"name": name}});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(s.execute("shape.rectangle", &p).unwrap())
}

/// The painted text starting with `prefix`: (whether it was elided, its position, its size).
pub(crate) fn painted(u: &mut Ui, prefix: &str) -> (bool, Pos2, Vec2) {
    let shapes = crate::live_tests::flat(&u.frame_output(vec![]).shapes);
    shapes
        .iter()
        .find_map(|s| match s {
            Shape::Text(t) if t.galley.text().starts_with(prefix) => Some((t.galley.elided, t.pos, t.galley.size())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text starting {prefix:?}"))
}

/// Turn the wheel sideways by `dx` points over `at`, and let the scroll settle.
fn wheel_sideways(u: &mut Ui, at: Pos2, dx: f32) {
    let wheel = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        source: egui::MouseWheelSource::Unknown,
        delta: vec2(-dx, 0.0),
        phase: TouchPhase::Move,
        modifiers: Default::default(),
    };
    u.frame(vec![Event::PointerMoved(at), wheel]);
    for _ in 0..30 {
        u.frame(vec![]);
    }
}

const LONG: &str = "Size=small, Type=primary, State=default, Icon=leading, Width=full";

#[test]
fn a_long_name_is_painted_in_full_and_the_list_scrolls_sideways() {
    let mut s = Session::new();
    let long = named(&mut s, LONG, None);
    named(&mut s, "Short", None);
    let mut u = Ui::new(s);
    let (elided, p, size) = painted(&mut u, "Size=small");
    assert!(!elided, "the whole name, not an elided one");
    // The panel is 240 pt wide: the name runs past it, into the space a sideways scroll shows.
    assert!(p.x + size.x > 320.0, "the name runs past the panel: {} + {}", p.x, size.x);
    let at = u.row_at(long);
    wheel_sideways(&mut u, at, 120.0);
    let (_, scrolled, _) = painted(&mut u, "Size=small");
    assert!(scrolled.x <= p.x - 100.0, "the list scrolled sideways: {} -> {}", p.x, scrolled.x);
    // Short names move with it (one list), not on their own.
    let (_, short, _) = painted(&mut u, "Short");
    assert!(short.x < 100.0, "the short name moved with the list: {}", short.x);
}

#[test]
fn a_deeply_nested_name_gets_its_indent_too() {
    let mut s = Session::new();
    // A name that fits at the top level, under five frames: the indent pushes it past the panel.
    let mut parent = None;
    for i in 0..5 {
        let f = id(s
            .execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100, "parent": parent, "props": {"name": format!("Frame {i}")}}))
            .unwrap());
        parent = Some(f);
    }
    let leaf = named(&mut s, "Component set original (não usar)", parent);
    s.execute("select.set", &json!({"ids": [leaf]})).unwrap();
    let mut u = Ui::new(s);
    let (elided, p, size) = painted(&mut u, "Component set original");
    assert!(!elided, "the nested name reads in full");
    assert!(p.x + size.x > 320.0, "it runs past the panel: {} + {}", p.x, size.x);
}

#[test]
fn names_that_fit_leave_the_list_as_wide_as_the_panel() {
    let mut s = Session::new();
    let short = named(&mut s, "Short", None);
    let mut u = Ui::new(s);
    let (elided, p, _) = painted(&mut u, "Short");
    assert!(!elided);
    let at = u.row_at(short);
    wheel_sideways(&mut u, at, 120.0);
    let (_, after, _) = painted(&mut u, "Short");
    assert_eq!(after.x, p.x, "nothing to scroll to sideways");
}

#[test]
fn a_name_past_the_room_cap_still_elides() {
    let mut s = Session::new();
    let name = "Layer ".repeat(500);
    named(&mut s, &name, None);
    let mut u = Ui::new(s);
    let (elided, _, size) = painted(&mut u, "Layer Layer");
    assert!(elided, "a 3000-character name elides");
    assert!(size.x <= crate::layers::NAME_ROOM + 1.0, "at the cap, not thousands of points wide: {}", size.x);
}

/// Review: the widest name isn't always the longest in bytes. Fifty rows of narrow letters outrank
/// one row of wide ones by the bound; it is laid out all the same, so it reads in full.
#[test]
fn a_wide_name_among_many_long_narrow_ones_is_still_measured() {
    let mut s = Session::new();
    for _ in 0..50 {
        named(&mut s, &"i".repeat(100), None);
    }
    named(&mut s, &"W".repeat(25), None);
    let mut u = Ui::new(s);
    let (elided, _, size) = painted(&mut u, "WWWW");
    assert!(!elided, "the wide name reads in full");
    // Wider than the panel leaves a name, so it reads in full only if it was measured, and well
    // under the cap, with or without craft-fonts.
    assert!(size.x > 200.0 && size.x < crate::layers::NAME_ROOM - 100.0, "the test's name is {} pt wide", size.x);
}

/// Review: a name is laid out as it is painted, on one line, so line breaks in it don't make it
/// measure narrower than it paints.
#[test]
fn a_name_with_line_breaks_is_measured_as_painted() {
    let mut s = Session::new();
    named(&mut s, "Size=small,\nType=primary,\nState=default,\nIcon=leading,\nWidth=full", None);
    let mut u = Ui::new(s);
    let (elided, ..) = painted(&mut u, "Size=small,");
    assert!(!elided, "the whole name on one line");
}

/// Review: a wide row runs on under the canvas; a dragged layer released there is no drop.
#[test]
fn a_drag_released_over_the_canvas_drops_nowhere() {
    let mut s = Session::new();
    let wide = named(&mut s, &"Size=small, Type=primary, ".repeat(6), None);
    let short = named(&mut s, "Short", None);
    let page = s.page().unwrap();
    let order = s.doc().unwrap().node(page).unwrap().children.clone();
    let mut u = Ui::new(s);
    let from = u.row_at(short);
    let level = u.row_at(wide).y;
    // Over the canvas, level with the wide row (which runs on under it).
    let over_canvas = Pos2::new(800.0, level);
    u.drag_to(from, over_canvas);
    u.button(over_canvas, false);
    for _ in 0..5 {
        u.frame(vec![]);
    }
    assert_eq!(u.app.session.doc().unwrap().node(page).unwrap().children, order, "no reorder");
}

fn hidden_layer_eye(name: &str, mirrored: bool) -> (Ui, NodeId, egui::Rect) {
    let mut s = Session::new();
    let layer =
        id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20, "props": {"name": name, "visible": false}})).unwrap());
    let mut u = Ui::new(s);
    u.app.ui.mirrored = mirrored;
    u.ctx.enable_accesskit();
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let eye = nodes
        .iter()
        .find(|(_, n)| n.role() == egui::accesskit::Role::Button && n.label() == Some("Hidden"))
        .and_then(|(_, n)| n.bounds())
        .expect("the eye of the hidden layer");
    (u, layer, egui::Rect::from_min_max(Pos2::new(eye.x0 as f32, eye.y0 as f32), Pos2::new(eye.x1 as f32, eye.y1 as f32)))
}

#[test]
fn the_eye_of_a_long_named_layer_sits_at_the_panels_edge_and_is_clickable() {
    for mirrored in [false, true] {
        let (_, _, short) = hidden_layer_eye("Short", mirrored);
        let (mut u, layer, long) = hidden_layer_eye(LONG, mirrored);
        let (a, b) = if mirrored { (long.min.x, short.min.x) } else { (long.max.x, short.max.x) };
        assert!((a - b).abs() < 1.0, "mirrored {mirrored}: the eye at the panel's edge, not past it: {a} vs {b}");
        u.button(long.center(), true);
        u.button(long.center(), false);
        assert!(u.app.session.doc().unwrap().node(layer).unwrap().visible, "mirrored {mirrored}: the eye shows the layer");
    }
}

#[test]
fn a_paragraph_named_text_layer_widens_the_list_no_more_than_the_name_cap() {
    let mut s = Session::new();
    named(&mut s, &"After the Chicago World's Fair, Tiffany built Laurelton Hall on Long Island. ".repeat(4), None);
    let mut u = Ui::new(s);
    let (elided, p, size) = painted(&mut u, "After the Chicago");
    assert!(elided, "a paragraph elides at the cap");
    assert!(p.x + size.x <= p.x + 400.0 + 1.0, "the name takes at most 400 pt: {}", size.x);
}

/// The bounds of the buttons named `label` (AccessKit), from left to right.
fn buttons(u: &mut Ui, label: &str) -> Vec<egui::Rect> {
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let mut rects: Vec<egui::Rect> = nodes
        .iter()
        .filter(|(_, n)| n.role() == egui::accesskit::Role::Button && n.label() == Some(label))
        .filter_map(|(_, n)| n.bounds())
        .map(|b| egui::Rect::from_min_max(Pos2::new(b.x0 as f32, b.y0 as f32), Pos2::new(b.x1 as f32, b.y1 as f32)))
        .collect();
    rects.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
    rects
}

/// Review: an ordinary row as wide as the view keeps its eye where it was before the eye and padlock
/// were held in view (measured on `main`: 262 to 282 pt, mirrored 1158 to 1178 pt), with a vertical
/// scrollbar or without.
#[test]
fn the_eye_of_an_ordinary_row_stays_where_it_was() {
    for count in [1, 60] {
        for mirrored in [false, true] {
            let mut s = Session::new();
            for i in 0..count {
                let props = json!({"name": format!("Rect {i}"), "visible": i + 1 != count});
                s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20, "props": props})).unwrap();
            }
            let mut u = Ui::new(s);
            u.app.ui.mirrored = mirrored;
            u.ctx.enable_accesskit();
            for _ in 0..3 {
                u.frame(vec![]);
            }
            let eye = buttons(&mut u, "Hidden");
            let x = if mirrored { 1158.0 } else { 262.0 };
            assert_eq!(eye.iter().map(|r| (r.min.x, r.max.x)).collect::<Vec<_>>(), [(x, x + 20.0)], "{count} rows, mirrored {mirrored}");
        }
    }
}

/// Review: while a sweep across the eyes is held, a long-named row it took in shows its eye, so its
/// name stays clipped before the eye after the pointer has moved past it.
#[test]
fn a_swept_long_named_row_keeps_its_name_clipped_before_the_eye() {
    let mut s = Session::new();
    let long = named(&mut s, LONG, None);
    let short = named(&mut s, "Short", None);
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    let at = u.row_at(short);
    u.frame(vec![Event::PointerMoved(at)]);
    let eye = *buttons(&mut u, "Hidden").first().expect("the hovered row's eye");
    let below = u.row_at(long) + vec2(0.0, 40.0);
    u.frame(vec![Event::PointerMoved(eye.center())]);
    u.drag_to(eye.center(), Pos2::new(eye.center().x, below.y));
    let out = u.frame_output(vec![]);
    let clip = out
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            Shape::Text(t) if t.galley.text().starts_with("Size=small") => Some(c.clip_rect),
            _ => None,
        })
        .expect("the long name");
    u.button(Pos2::new(eye.center().x, below.y), false);
    let doc = u.app.session.doc().unwrap();
    assert!(!doc.node(long).unwrap().visible && !doc.node(short).unwrap().visible, "the sweep hid both");
    assert!(clip.max.x <= eye.min.x - 20.0, "the name ends before the padlock and eye: {} vs eye at {}", clip.max.x, eye.min.x);
}

/// Review: on a row nested so deep that its disclosure triangle sits where the eye and padlock
/// would be at the view's edge, they stay at the row's end, so the triangle still opens and closes
/// the layer.
#[test]
fn the_disclosure_triangle_of_a_deeply_nested_row_is_not_under_the_padlock() {
    let mut s = Session::new();
    let mut parent = None;
    let mut deepest = None;
    for i in 0..13 {
        let f = id(s
            .execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100, "parent": parent, "props": {"name": format!("F{i}")}}))
            .unwrap());
        parent = Some(f);
        deepest = Some(f);
    }
    let deepest = deepest.unwrap();
    let leaf = named(&mut s, "Leaf", parent);
    s.execute("select.set", &json!({"ids": [leaf]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    let tri = *buttons(&mut u, "Collapse").last().expect("the deepest frame's triangle");
    u.frame(vec![Event::PointerMoved(tri.center())]);
    u.frame(vec![Event::PointerMoved(tri.center())]);
    u.button(tri.center(), true);
    u.button(tri.center(), false);
    u.frame(vec![]);
    assert!(!u.app.ui.expanded.contains(&deepest), "the triangle closed the frame");
    let n = u.app.session.doc().unwrap().node(deepest).unwrap();
    assert!(n.visible && !n.locked, "neither the eye nor the padlock took the click");
}
