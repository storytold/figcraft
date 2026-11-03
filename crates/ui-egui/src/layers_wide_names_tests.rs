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
