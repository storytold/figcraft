//! A component set's Properties at the narrowest right panel (240 px, the default), driven through
//! egui headless: every value field and every + and − button stays inside the panel, and the
//! values wrap onto more lines instead of running off its edge (found while retaking the README
//! screenshots).

use egui::accesskit::Node;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::component_swap_default_tests::tree;
use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// The right edge of what a panel row may use: the window's, less the panel's right inset.
const RIGHT: f64 = 1440.0 - crate::widgets::PAD_RIGHT as f64;

fn label(n: &Node) -> String {
    n.label().or(n.value()).unwrap_or_default().to_string()
}

#[test]
fn a_component_sets_properties_fit_the_narrowest_panel() {
    let mut s = Session::new();
    let mut variants = Vec::new();
    let names = ["Style=Primary, State=Default", "Style=Tonal, State=Hover", "Style=Outline, State=Disabled", "Style=Primary, State=Hover"];
    for (k, name) in names.iter().enumerate() {
        let f = id(s.execute("frame.create", &json!({"x": 0, "y": k * 60, "width": 80, "height": 40, "name": name})).unwrap());
        variants.push(id(s.execute("component.create", &json!({"ids": [f]})).unwrap()));
    }
    let set = id(s.execute("component.combineVariants", &json!({"ids": variants})).unwrap());
    s.execute("component.addProperty", &json!({"id": set, "name": "Show icon", "type": "boolean", "default": true})).unwrap();
    s.execute("component.addProperty", &json!({"id": set, "name": "Label", "type": "text", "default": "Continue"})).unwrap();
    s.execute("select.set", &json!({"ids": [set]})).unwrap();
    let mut u = Ui::new(s);
    assert_eq!(u.app.ui.right_width, crate::PANEL_DEFAULT);
    u.ctx.enable_accesskit();
    let nodes = tree(&mut u);
    let find = |text: &str| -> Vec<&Node> { nodes.iter().filter(|n| label(n) == text && n.bounds().is_some()).collect() };
    for text in ["Primary", "Tonal", "Outline", "Default", "Hover", "Disabled", "Delete property", "Add variant with a new value"] {
        let found = find(text);
        assert!(!found.is_empty(), "{text} is in the panel");
        for n in found {
            let b = n.bounds().unwrap_or_default();
            assert!(b.x1 <= RIGHT + 0.5, "{text} ends at {} past the panel's {RIGHT}", b.x1);
        }
    }
    assert_eq!(find("Delete property").len(), 4, "a − for each property");
    assert_eq!(find("Add variant with a new value").len(), 2, "a + for each variant property");
    // Two values to a line at this width: Outline goes on the line under Tonal.
    let y = |text: &str| find(text).first().map(|n| n.bounds().unwrap_or_default().y0).unwrap_or_default();
    assert!(y("Outline") > y("Tonal") + 10.0, "Outline wraps to the next line");
}
