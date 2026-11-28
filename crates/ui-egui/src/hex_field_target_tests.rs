//! A hex field in a fill row belongs to the layers the row showed: clicking another layer, on the
//! canvas or in the layers panel, ends the edit, and the colour goes to the layer it was typed
//! for (none when nothing was typed), not to the one clicked.

use egui::accesskit::Role;
use egui::{Event, Pos2, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: &serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().or_else(|| v["ids"][0].as_u64()).unwrap())
}

fn fill(u: &Ui, n: NodeId) -> Option<String> {
    u.app.session.doc().unwrap().node(n).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_css_hex())
}

/// The middle of the Design panel's hex field showing `hex`.
fn hex_field(u: &mut Ui, hex: &str) -> Pos2 {
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n = nodes
        .iter()
        .map(|(_, n)| n)
        .find(|n| n.role() == Role::TextInput && n.value().is_some_and(|v| v.eq_ignore_ascii_case(hex.trim_start_matches('#'))))
        .unwrap_or_else(|| panic!("no hex field showing {hex}"));
    let b = n.bounds().expect("bounds");
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

/// Press at `p` and let go: within one frame (a quick click), or held for a few frames.
fn click(u: &mut Ui, p: Pos2, held: bool) {
    let button = |pressed| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    u.frame(vec![Event::PointerMoved(p)]);
    if held {
        u.frame(vec![button(true)]);
        u.frame(vec![]);
        u.frame(vec![button(false)]);
    } else {
        u.frame(vec![button(true), button(false)]);
    }
    u.frame(vec![]);
}

/// A grey rectangle, a section and a second rectangle; the first rectangle is selected.
fn setup() -> (Ui, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let a = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    s.execute("fill.set", &json!({"ids": [a], "color": "#808080"})).unwrap();
    let sec = id(&s.execute("node.create", &json!({"type": "section", "x": 200, "y": 0, "width": 300, "height": 300})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 200, "width": 40, "height": 40})).unwrap());
    s.execute("fill.set", &json!({"ids": [b], "color": "#00FF00"})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    (u, a, sec, b)
}

/// Click into the first rectangle's hex field, type `typed` (nothing when empty) and click the
/// section, the second rectangle or empty canvas, quickly or `held`.
fn leave_hex_for(u: &mut Ui, typed: &str, target: &str, held: bool, sec: NodeId, b: NodeId) {
    let field = hex_field(u, "808080");
    click(u, field, false);
    if !typed.is_empty() {
        u.frame(vec![Event::Text(typed.into())]);
    }
    u.frame(vec![]);
    let (selects, x, y) = match target {
        "section" | "section's row" => (vec![sec], 350.0, 200.0),
        "rectangle" => (vec![b], 20.0, 220.0),
        _ => (vec![], 100.0, 100.0),
    };
    let at = if target == "section's row" {
        layer_row(u, "Section 1")
    } else {
        let at = u.app.to_screen(x, y);
        assert!(u.app.canvas.rect.contains(at), "{target} at {at:?} is on the canvas");
        at
    };
    click(u, at, held);
    u.frame(vec![]);
    let selected: Vec<NodeId> = u.app.session.ds().unwrap().selection.clone();
    assert_eq!(selected, selects, "clicking the {target} selects it");
}

/// The middle of the layers panel row named `name`.
fn layer_row(u: &mut Ui, name: &str) -> Pos2 {
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n = nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::TreeItem && n.label() == Some(name)).unwrap_or_else(|| panic!("no row {name:?}"));
    let b = n.bounds().expect("bounds");
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

/// A hex field clicked into and left for another layer doesn't paint that layer with the colour
/// it showed.
#[test]
fn a_hex_field_left_for_another_layer_leaves_that_layer_its_colour() {
    let targets = ["section", "section's row", "rectangle"];
    for (target, held) in targets.iter().flat_map(|t| [(*t, false), (*t, true)]) {
        let (mut u, a, sec, b) = setup();
        let (sec_before, b_before) = (fill(&u, sec), fill(&u, b));
        leave_hex_for(&mut u, "", target, held, sec, b);
        assert_eq!((fill(&u, sec), fill(&u, b)), (sec_before, b_before), "the clicked {target} keeps its fill (held: {held})");
        assert_eq!(fill(&u, a).as_deref(), Some("#808080"), "the rectangle edited keeps its fill ({target}, held: {held})");
    }
}

/// A hex typed and left for another layer or empty canvas colours the layer it was typed for.
#[test]
fn a_hex_typed_and_left_for_another_layer_colours_the_layer_it_was_typed_for() {
    let targets = ["section", "section's row", "rectangle", "empty canvas"];
    for (target, held) in targets.iter().flat_map(|t| [(*t, false), (*t, true)]) {
        let (mut u, a, sec, b) = setup();
        let (sec_before, b_before) = (fill(&u, sec), fill(&u, b));
        leave_hex_for(&mut u, "FF0000", target, held, sec, b);
        assert_eq!((fill(&u, sec), fill(&u, b)), (sec_before, b_before), "the clicked {target} keeps its fill (held: {held})");
        assert_eq!(fill(&u, a).as_deref(), Some("#FF0000"), "the colour goes to the layer it was typed for ({target}, held: {held})");
    }
}
