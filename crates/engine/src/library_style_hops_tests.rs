//! Styles that reach a file through another library (#2414): a kit's style used by a component of
//! a second library is, in a file linking both, the kit's own bound copy, so a mode set on the kit's
//! collection colours it. Before, the file got a second, unbound copy frozen in the kit's default mode.

use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// The main component named `name` (a library copy or this file's own).
fn component(s: &Session, name: &str) -> NodeId {
    s.doc().unwrap().nodes().find(|n| n.name == name && matches!(n.kind, NodeKind::Component { .. })).unwrap().id
}

/// Publish `s` and save it as `name`.figcraft in `dir`; returns (its path, its library id).
fn publish(s: &mut Session, dir: &std::path::Path, name: &str) -> (String, String) {
    let lid = ex(s, "library.publish", json!({}))["id"].as_str().unwrap().to_string();
    let path = dir.join(format!("{name}.figcraft")).to_string_lossy().into_owned();
    ex(s, "file.saveAs", json!({"path": path}));
    (path, lid)
}

#[test]
fn a_kit_style_through_another_library_is_the_kits_bound_copy() {
    let dir = std::env::temp_dir().join(format!("figcraft-style-hops-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // kit: a Theme (Light/Brand) colour and a style bound to it, used by "kit-button".
    let mut kit = Session::new();
    ex(&mut kit, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Brand"]}));
    ex(
        &mut kit,
        "variable.create",
        json!({"collection": "Theme", "name": "primary", "type": "color", "values": {"Light": "#7B1FA2", "Brand": "#1565C0"}}),
    );
    let src = id(&ex(&mut kit, "node.create", json!({"type": "rectangle", "x": 0, "y": 200, "width": 40, "height": 40})));
    ex(&mut kit, "variable.bind", json!({"ids": [src], "field": "fill", "variable": "primary"}));
    ex(&mut kit, "style.create", json!({"type": "paint", "name": "kit/primary", "from": src}));
    let c = id(&ex(&mut kit, "node.create", json!({"type": "component", "width": 120, "height": 40, "name": "kit-button"})));
    let bg = id(&ex(&mut kit, "node.create", json!({"type": "rectangle", "parent": c, "width": 120, "height": 40, "name": "bg"})));
    ex(&mut kit, "style.apply", json!({"ids": [bg], "style": "kit/primary", "target": "fill"}));
    let (kit_path, kit_id) = publish(&mut kit, &dir, "kit");

    // auth: "auth-button" holds a kit-button.
    let mut auth = Session::new();
    ex(&mut auth, "library.add", json!({"path": kit_path}));
    ex(&mut auth, "library.import", json!({"library": kit_id, "component": "kit-button"}));
    let c = id(&ex(&mut auth, "node.create", json!({"type": "component", "width": 200, "height": 40, "name": "auth-button"})));
    let main = component(&auth, "kit-button");
    ex(&mut auth, "instance.create", json!({"component": main, "parent": c}));
    let (auth_path, auth_id) = publish(&mut auth, &dir, "auth");

    // app: links both, uses an auth-button in a frame set to the kit's Brand mode.
    let mut app = Session::new();
    ex(&mut app, "library.add", json!({"path": kit_path}));
    ex(&mut app, "library.add", json!({"path": auth_path}));
    ex(&mut app, "library.import", json!({"library": kit_id, "component": "kit-button"}));
    ex(&mut app, "library.import", json!({"library": auth_id, "component": "auth-button"}));
    let f = id(&ex(&mut app, "node.create", json!({"type": "frame", "width": 240, "height": 100, "name": "Brand"})));
    ex(&mut app, "library.setVariableMode", json!({"library": kit_id, "collection": "Theme", "mode": "Brand", "ids": [f]}));
    let main = component(&app, "auth-button");
    ex(&mut app, "instance.create", json!({"component": main, "parent": f}));

    let doc = app.doc().unwrap();
    let kit_styles: Vec<&Style> = doc.styles.iter().filter(|s| s.name == "kit/primary").collect();
    assert_eq!(kit_styles.len(), 1, "one copy of the kit's style: {:?}", kit_styles.iter().map(|s| &s.id).collect::<Vec<_>>());
    let style = kit_styles[0];
    assert!(doc.libraries.styles.iter().any(|r| r.local == style.id && r.library == kit_id), "it is the kit's copy: {}", style.id);
    let var = style.bound.values().next().expect("the copy stays bound");
    assert!(doc.libraries.variables.iter().any(|r| r.local == *var && r.library == kit_id), "to the kit's variable: {var}");
    // The bg inside the placed auth-button uses it.
    let placed: Vec<NodeId> = doc.subtree(f).into_iter().filter(|n| doc.node(*n).is_some_and(|n| n.name == "bg")).collect();
    assert_eq!(placed.len(), 1);
    assert_eq!(doc.node(placed[0]).unwrap().styles.fill.as_deref(), Some(style.id.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}
