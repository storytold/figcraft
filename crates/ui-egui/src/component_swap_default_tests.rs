//! The default of a main component's instance-swap property, driven through egui headless: with
//! more components than fit on the screen (an icon library), its menu scrolls, so the last one can
//! be picked (#1338).

use egui::accesskit::{Node, Role};
use egui::{Event, MouseWheelUnit, Pos2, TouchPhase, pos2, vec2};
use figcraft_doc::{NodeId, NodeKind, PropDef};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A selected "Button" component whose instance-swap property "Icon" defaults to "Heart", then 80
/// more components, the last "Zebra". Returns (ui, button, zebra).
pub(crate) fn setup() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let mut icon = |name: &str, k: usize| {
        let f =
            id(s.execute("frame.create", &json!({"x": (k % 20) * 40, "y": 200 + (k / 20) * 40, "width": 24, "height": 24, "name": name})).unwrap());
        id(s.execute("component.create", &json!({"ids": [f]})).unwrap())
    };
    let heart = icon("Heart", 0);
    let mut zebra = heart;
    for k in 1..=80 {
        zebra = icon(&if k == 80 { "Zebra".to_string() } else { format!("Glyph {k}") }, k);
    }
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 120, "height": 40, "name": "Button"})).unwrap());
    let nested = id(s.execute("instance.create", &json!({"component": heart, "x": 8, "y": 8, "parent": f})).unwrap());
    let button = id(s.execute("component.create", &json!({"ids": [f]})).unwrap());
    s.execute("component.addProperty", &json!({"id": button, "name": "Icon", "type": "instanceSwap", "default": heart, "layer": nested})).unwrap();
    s.execute("select.set", &json!({"ids": [button]})).unwrap();
    let u = Ui::new(s);
    u.ctx.enable_accesskit();
    (u, button, zebra)
}

pub(crate) fn tree(u: &mut Ui) -> Vec<Node> {
    u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes.into_iter().map(|(_, n)| n).collect()
}

pub(crate) fn centre(n: &Node) -> Pos2 {
    let b = n.bounds().unwrap();
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

pub(crate) fn click(u: &mut Ui, at: Pos2) {
    u.button(at, true);
    u.button(at, false);
}

/// Turn the mouse wheel down by `dy` points over `at`, and let the scroll settle.
fn wheel(u: &mut Ui, at: Pos2, dy: f32) {
    let wheel = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        source: egui::MouseWheelSource::Unknown,
        delta: vec2(0.0, -dy),
        phase: TouchPhase::Move,
        modifiers: Default::default(),
    };
    u.frame(vec![Event::PointerMoved(at), wheel]);
    for _ in 0..30 {
        u.frame(vec![]);
    }
}

fn swap_default(u: &Ui, button: NodeId) -> Option<NodeId> {
    match u.app.session.ds().ok()?.doc.node(button).map(|n| &n.kind) {
        Some(NodeKind::Component { props, .. }) => match props.get("Icon") {
            Some(PropDef::InstanceSwap { default, .. }) => Some(*default),
            _ => None,
        },
        _ => None,
    }
}

/// The default's dropdown on the "Icon" property row, scrolled into view in the Design panel.
pub(crate) fn default_combo(u: &mut Ui) -> Option<Node> {
    for _ in 0..20 {
        let combo = tree(u).into_iter().find(|n| n.role() == Role::ComboBox && n.label() == Some("Icon"))?;
        if centre(&combo).y < 860.0 {
            return Some(combo);
        }
        wheel(u, pos2(1300.0, 500.0), 100.0);
    }
    None
}

#[test]
fn the_default_menu_scrolls_to_its_last_component() {
    let (mut u, button, zebra) = setup();
    let combo = default_combo(&mut u).expect("the Icon property has a dropdown for its default");
    assert_eq!(combo.value(), Some("Heart"));
    // Layer rows share the names; the menu's items are the ones that weren't there before it opened.
    let before: Vec<Pos2> = tree(&mut u).iter().filter(|n| n.label().is_some_and(|l| l == "Zebra" || l == "Glyph 1")).map(centre).collect();
    click(&mut u, centre(&combo));
    let item = |u: &mut Ui, name: &str| tree(u).into_iter().find(|n| n.label() == Some(name) && !before.contains(&centre(n)));
    let top = centre(&item(&mut u, "Glyph 1").expect("the menu lists the components"));
    let bottom = centre(&item(&mut u, "Zebra").expect("the menu lists the last component"));
    assert!(bottom.y > 900.0, "80 components don't fit on the screen: Zebra at {bottom:?}");
    // The menu is capped and scrolls: wheel over it to the end of the list, and pick Zebra.
    let mut at = bottom;
    for _ in 0..40 {
        wheel(&mut u, top, 200.0);
        let next = centre(&item(&mut u, "Zebra").expect("the menu stays open while it scrolls"));
        if next == at {
            break;
        }
        at = next;
    }
    assert!(at.y < 880.0, "the menu scrolls Zebra onto the screen: at {at:?}");
    assert!(at.y - top.y < 400.0, "Zebra ends up inside the capped menu, near its top row: {top:?} → {at:?}");
    click(&mut u, at);
    assert_eq!(swap_default(&u, button), Some(zebra), "picking the last component makes it the default");
}
