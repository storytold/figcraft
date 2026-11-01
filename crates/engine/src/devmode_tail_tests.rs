//! Dev Mode, the tail: import lines for the connected components a layer tree calls.

use crate::Session;
use crate::components::tests::{ex, idv};
use figcraft_doc::NodeId;
use serde_json::json;

fn code(s: &mut Session, id: NodeId, lang: &str) -> String {
    ex(s, "inspect.code", json!({"id": id, "language": lang}))["code"].as_str().unwrap_or_default().to_string()
}

/// A component named `name` (an empty frame) at `x`.
fn component(s: &mut Session, name: &str, x: f64) -> NodeId {
    let f = idv(&ex(s, "frame.create", json!({"x": x, "y": 1000, "width": 40, "height": 20, "name": name})));
    idv(&ex(s, "component.create", json!({"ids": [f]})))
}

fn instance(s: &mut Session, c: NodeId, parent: NodeId) -> NodeId {
    idv(&ex(s, "instance.create", json!({"component": c, "x": 0, "y": 0, "parent": parent})))
}

fn frame(s: &mut Session, name: &str, parent: Option<NodeId>) -> NodeId {
    idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 300, "name": name, "parent": parent})))
}

fn count(code: &str, line: &str) -> usize {
    code.lines().filter(|l| *l == line).count()
}

#[test]
fn layer_trees_import_the_connected_components_they_call_once_at_the_top() {
    let mut s = Session::new();
    let badge = component(&mut s, "Badge", 0.0);
    let button = component(&mut s, "Button", 100.0);
    let toast = component(&mut s, "Toast", 200.0);
    let divider = component(&mut s, "Divider", 300.0);
    for (lang, path) in [("react", "@acme/ui"), ("swiftui", "AcmeUI"), ("leptos", "crate::ui"), ("compose", "com.acme.ui.Badge")] {
        ex(&mut s, "codeConnect.set", json!({"id": badge, "language": lang, "importPath": path}));
    }
    ex(&mut s, "codeConnect.set", json!({"id": button, "importPath": "@acme/buttons"}));
    ex(&mut s, "codeConnect.set", json!({"id": toast, "importPath": "@acme/toast"}));
    let screen = frame(&mut s, "Screen", None);
    let row = frame(&mut s, "Row", Some(screen));
    for (c, parent) in [(badge, screen), (button, screen), (badge, screen), (divider, screen), (badge, row)] {
        instance(&mut s, c, parent);
    }
    let hidden = instance(&mut s, toast, row);
    ex(&mut s, "node.set", json!({"id": hidden, "props": {"visible": false}}));

    let react = code(&mut s, screen, "react");
    assert!(react.starts_with("import { Badge } from \"@acme/ui\";\nimport { Button } from \"@acme/buttons\";\nimport \"./"), "{react}");
    assert_eq!(count(&react, "import { Badge } from \"@acme/ui\";"), 1, "once each: {react}");
    assert!(!react.contains("@acme/toast"), "hidden layers aren't in the code: {react}");
    let swift = code(&mut s, screen, "swiftui");
    assert!(swift.starts_with("import AcmeUI\n\n") && count(&swift, "import AcmeUI") == 1, "per language: {swift}");
    assert!(!swift.contains("acme/buttons"), "{swift}");
    let leptos = code(&mut s, screen, "leptos");
    assert!(leptos.starts_with("use crate::ui::Badge;\nuse leptos::prelude::*;\n"), "joined to the code's own imports: {leptos}");
    assert!(code(&mut s, screen, "compose").starts_with("import com.acme.ui.Badge\n\n"));
    assert!(!code(&mut s, screen, "css").contains("import"), "CSS has no imports");
    // The Inspect panel shows inspect.code's text (the same function).
    let doc = s.doc().unwrap();
    assert_eq!(crate::codegen::code_for(doc, screen, crate::codegen::CodeLang::React), react);

    // A connected layer itself: its import and snippet, unchanged.
    assert_eq!(code(&mut s, badge, "react"), "import { Badge } from \"@acme/ui\";\n\n<Badge />\n");

    // A component's definition, and an instance of it (its call, then the definition).
    let card_frame = frame(&mut s, "Card", None);
    instance(&mut s, badge, card_frame);
    let card = idv(&ex(&mut s, "component.create", json!({"ids": [card_frame]})));
    assert!(
        code(&mut s, card, "react").starts_with("import { Badge } from \"@acme/ui\";\nimport \"./card.css\";"),
        "{}",
        code(&mut s, card, "react")
    );
    let page = frame(&mut s, "Page", None);
    let card_i = instance(&mut s, card, page);
    assert!(code(&mut s, card_i, "react").starts_with("import { Badge } from \"@acme/ui\";\n\n<Card />"), "{}", code(&mut s, card_i, "react"));
    assert!(code(&mut s, page, "react").starts_with("import \"./"), "an unconnected call's insides aren't in the code");
}

#[test]
fn slots_and_instance_swaps_bring_their_connected_imports() {
    let mut s = Session::new();
    let badge = component(&mut s, "Badge", 0.0);
    ex(&mut s, "codeConnect.set", json!({"id": badge, "importPath": "@acme/ui"}));
    // A panel whose slot an instance fills with a badge.
    let f = frame(&mut s, "Panel", None);
    let content = frame(&mut s, "Content", Some(f));
    let panel = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "component.makeSlot", json!({"id": content}));
    let screen = frame(&mut s, "Screen", None);
    let p = instance(&mut s, panel, screen);
    assert!(!code(&mut s, screen, "react").contains("@acme/ui"), "an empty slot");
    ex(&mut s, "instance.slotInsert", json!({"id": p, "slot": "Content", "component": badge, "replace": true}));
    assert!(code(&mut s, screen, "react").starts_with("import { Badge } from \"@acme/ui\";\n"), "{}", code(&mut s, screen, "react"));
    assert!(code(&mut s, p, "react").starts_with("import { Badge } from \"@acme/ui\";\n\n<Panel content={("), "{}", code(&mut s, p, "react"));

    // A connected chip whose template places its icon: the swapped-in connected component's import.
    let heart = component(&mut s, "Heart", 300.0);
    let chip_frame = frame(&mut s, "Chip", None);
    let nested = instance(&mut s, heart, chip_frame);
    let chip = idv(&ex(&mut s, "component.create", json!({"ids": [chip_frame]})));
    ex(&mut s, "component.addProperty", json!({"id": chip, "name": "Icon", "type": "instanceSwap", "default": heart, "layer": nested}));
    ex(&mut s, "codeConnect.set", json!({"id": chip, "importPath": "@acme/chip", "template": "<Chip icon={Icon} />"}));
    let c = instance(&mut s, chip, screen);
    ex(&mut s, "instance.setProperties", json!({"ids": [c], "properties": {"Icon": badge}}));
    assert_eq!(code(&mut s, c, "react"), "import { Chip } from \"@acme/chip\";\nimport { Badge } from \"@acme/ui\";\n\n<Chip icon={<Badge />} />\n");
    let tree = code(&mut s, screen, "react");
    assert!(tree.starts_with("import { Badge } from \"@acme/ui\";\nimport { Chip } from \"@acme/chip\";\nimport \"./"), "{tree}");
    // An unconnected swap target brings nothing; the default (Heart) isn't connected.
    ex(&mut s, "instance.setProperties", json!({"ids": [c], "properties": {"Icon": heart}}));
    assert_eq!(code(&mut s, c, "react"), "import { Chip } from \"@acme/chip\";\n\n<Chip icon={<Heart />} />\n");
}

#[test]
fn compare_changes_takes_any_saved_version_as_the_before() {
    let mut s = Session::new();
    let screen = frame(&mut s, "Screen", None);
    let button =
        idv(&ex(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 120, "height": 40, "parent": screen, "props": {"name": "Button"}})));
    let old = idv(&ex(&mut s, "shape.ellipse", json!({"x": 200, "y": 20, "width": 20, "height": 20, "parent": screen, "props": {"name": "Dot"}})));
    let v1 = ex(&mut s, "file.saveVersion", json!({"name": "Kickoff"}))["id"].as_str().unwrap().to_string();
    ex(&mut s, "node.set", json!({"id": button, "props": {"width": 136}}));
    ex(&mut s, "edit.delete", json!({"ids": [old]}));
    let added = idv(&ex(&mut s, "text.create", json!({"text": "Hi", "x": 20, "y": 100, "parent": screen, "props": {"name": "Note"}})));
    let v2 = ex(&mut s, "file.saveVersion", json!({"name": "Review"}))["id"].as_str().unwrap().to_string();
    ex(&mut s, "node.set", json!({"id": button, "props": {"width": 150}}));

    // Any frame, ready for dev or not.
    let c = ex(&mut s, "dev.compare", json!({"id": screen, "version": v1}));
    assert_eq!(c["version"]["name"], "Kickoff");
    assert_eq!(
        (c["changed"].as_bool(), c["added"].as_u64(), c["removed"].as_u64(), c["modified"].as_u64()),
        (Some(true), Some(1), Some(1), Some(1)),
        "{c}"
    );
    let change =
        |c: &serde_json::Value, id: NodeId| c["changes"].as_array().unwrap().iter().find(|l| l["id"] == json!(id)).cloned().unwrap_or_default();
    assert_eq!(change(&c, button)["properties"][0]["text"], "width 120 → 150");
    assert_eq!(change(&c, old)["change"], "removed");
    assert_eq!(change(&c, added)["change"], "added");
    // A later version: only what changed since then.
    let c = ex(&mut s, "dev.compare", json!({"id": screen, "version": v2}));
    assert_eq!(c["changes"].as_array().map(Vec::len), Some(1), "{c}");
    assert_eq!(change(&c, button)["properties"][0]["text"], "width 136 → 150");
    // A frame the version doesn't have was added as a whole.
    let fresh = frame(&mut s, "Settings", None);
    let c = ex(&mut s, "dev.compare", json!({"id": fresh, "version": v1}));
    assert_eq!(c["changes"][0]["change"], "added");
    // The snapshot from when it was marked stays the default, for ready-for-dev layers only.
    assert!(s.execute("dev.compare", &json!({"id": screen})).is_err(), "not ready for dev");
    ex(&mut s, "node.set", json!({"id": screen, "props": {"devStatus": "readyForDev"}}));
    let c = ex(&mut s, "dev.compare", json!({"id": screen}));
    assert_eq!((c["changed"].as_bool(), c["version"].is_null()), (Some(false), true), "{c}");
    for bad in [json!({"id": screen, "version": "nope"}), json!({"id": screen, "version": 5}), json!({"id": button, "version": v1})] {
        assert!(s.execute("dev.compare", &bad).is_err(), "{bad}");
    }
    // The app opens on the version asked for.
    ex(&mut s, "dev.compare", json!({"id": screen, "version": v2, "show": true}));
    assert!(s.ui_requests.iter().any(|u| u.kind == "view.devCompare" && u.data["version"] == json!(v2)));
}
