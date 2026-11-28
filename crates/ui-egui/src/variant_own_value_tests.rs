//! A selected variant's property dropdowns in the Design panel, driven through egui headless: they
//! show the variant's own values, and choosing one renames that variant only. The set's default
//! (and so the variants' places) stays (#2349).

use egui::accesskit::{Node, Role};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::component_swap_default_tests::{centre, click, tree};
use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn combos(nodes: &[Node]) -> Vec<String> {
    nodes.iter().filter(|n| n.role() == Role::ComboBox).filter_map(|n| n.value().map(str::to_string)).collect()
}

#[test]
fn a_selected_variants_dropdown_shows_and_sets_its_own_value() {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for (k, name) in ["Looking=Top right, Size=S", "Looking=Down left, Size=S", "Looking=Top right, Size=L"].iter().enumerate() {
        let f = id(s.execute("frame.create", &json!({"x": 0, "y": 20 + k * 44, "width": 80, "height": 30, "name": name})).unwrap());
        variants.push(id(s.execute("component.create", &json!({"ids": [f]})).unwrap()));
    }
    id(s.execute("component.combineVariants", &json!({"ids": variants})).unwrap());
    let down = variants[1];
    s.execute("select.set", &json!({"ids": [down]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    let place = |u: &Ui| variants.iter().map(|v| u.app.session.doc().unwrap().world_bounds(*v).unwrap().y0).collect::<Vec<_>>();
    let before = place(&u);

    // The dropdowns show this variant's values, not the default's (Top right, S).
    let nodes = tree(&mut u);
    let shown = combos(&nodes);
    assert!(shown.contains(&"Down left".to_string()) && shown.contains(&"S".to_string()), "the variant's own values: {shown:?}");
    assert!(!shown.contains(&"Top right".to_string()), "not the set's default: {shown:?}");

    // Choosing L for Size renames this variant; the others and every place stay.
    let size = nodes.iter().find(|n| n.role() == Role::ComboBox && n.value() == Some("S")).expect("the Size dropdown");
    click(&mut u, centre(size));
    let nodes = tree(&mut u);
    let l = nodes
        .iter()
        .find(|n| n.label() == Some("L") && !matches!(n.role(), Role::ComboBox | Role::TextInput) && n.bounds().is_some())
        .expect("L in the menu");
    click(&mut u, centre(l));
    let doc = u.app.session.doc().unwrap();
    let names: Vec<&str> = variants.iter().map(|v| doc.node(*v).unwrap().name.as_str()).collect();
    assert_eq!(names, ["Looking=Top right, Size=S", "Looking=Down left, Size=L", "Looking=Top right, Size=L"]);
    assert_eq!(place(&u), before, "no variant moves");
}

#[test]
fn a_long_variant_dropdown_scrolls_within_the_viewport() {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for k in 0..50 {
        let name = format!("icon=v{k:02}");
        let f = id(s.execute("frame.create", &json!({"x": 0, "y": 20 + k * 44, "width": 24, "height": 24, "name": name})).unwrap());
        variants.push(id(s.execute("component.create", &json!({"ids": [f]})).unwrap()));
    }
    id(s.execute("component.combineVariants", &json!({"ids": variants})).unwrap());
    let inst = id(s.execute("instance.create", &json!({"component": variants[0], "x": 120, "y": 20})).unwrap());
    s.execute("select.set", &json!({"ids": [inst]})).unwrap();

    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    let nodes = tree(&mut u);
    let before: Vec<egui::LayerId> = u.ctx.memory(|m| m.layer_ids().collect());
    let combo = nodes.iter().find(|n| n.role() == Role::ComboBox && n.value() == Some("v00")).expect("the icon variant dropdown");
    click(&mut u, centre(combo));
    // menu_popup measures its rows on frame 1 and enables scrolling on frame 2.
    let mut nodes = tree(&mut u);
    for _ in 0..3 {
        nodes = tree(&mut u);
    }

    let menu = u
        .ctx
        .memory(|m| m.layer_ids().collect::<Vec<_>>())
        .into_iter()
        .filter(|l| l.order == egui::Order::Foreground && !before.contains(l))
        .filter_map(|l| u.ctx.memory(|m| m.area_rect(l.id)))
        .max_by(|a, b| a.height().total_cmp(&b.height()))
        .expect("the variant dropdown menu is open");
    let viewport = u.ctx.content_rect();
    assert!(menu.top() >= viewport.top() && menu.bottom() <= viewport.bottom(), "menu {menu:?} stays inside viewport {viewport:?}");
    assert!(menu.height() < viewport.height(), "50 variants are capped rather than overflowing: {menu:?}");

    let v00 = nodes
        .iter()
        .find(|n| n.label() == Some("v00") && !matches!(n.role(), Role::ComboBox | Role::TextInput) && n.bounds().is_some())
        .expect("v00 in the menu");
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        source: egui::MouseWheelSource::Unknown,
        delta: egui::vec2(0.0, -5000.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    };
    u.frame(vec![egui::Event::PointerMoved(centre(v00)), wheel]);
    for _ in 0..30 {
        nodes = tree(&mut u);
    }

    let v49 = nodes
        .iter()
        .find(|n| n.label() == Some("v49") && !matches!(n.role(), Role::ComboBox | Role::TextInput) && n.bounds().is_some())
        .expect("v49 in the scrolled menu");
    let v49_pos = centre(v49);
    assert!(v49_pos.y >= viewport.top() && v49_pos.y <= viewport.bottom(), "v49 scrolled into viewport: {v49_pos:?}");
    click(&mut u, v49_pos);
    let doc = u.app.session.doc().unwrap();
    let figcraft_doc::NodeKind::Instance { component, .. } = &doc.node(inst).unwrap().kind else {
        panic!("expected instance");
    };
    assert_eq!(*component, variants[49], "picking the last variant from the scrolled menu switches the instance");
}
