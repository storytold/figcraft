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
fn named(s: &mut Session, name: &str, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": 0, "y": 0, "width": 20, "height": 20, "props": {"name": name}});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(s.execute("shape.rectangle", &p).unwrap())
}

/// The painted text starting with `prefix`: (whether it was elided, its position, its size).
fn painted(u: &mut Ui, prefix: &str) -> (bool, Pos2, Vec2) {
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
    named(&mut s, &"W".repeat(70), None);
    let mut u = Ui::new(s);
    let (elided, ..) = painted(&mut u, "WWWW");
    assert!(!elided, "the wide name reads in full");
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
