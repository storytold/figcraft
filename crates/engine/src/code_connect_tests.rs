//! Code connect: components linked to their code components show the real snippet, with the
//! instance's values, instead of generated code.

use crate::components::tests::{ex, idv};
use crate::params::id_of;
use crate::{EngineError, Session};
use figcraft_doc::*;
use serde_json::{Value, json};

fn code(s: &mut Session, id: NodeId, lang: &str) -> String {
    ex(s, "inspect.code", json!({"id": id, "language": lang}))["code"].as_str().unwrap_or_default().to_string()
}

/// A "Card" component with a "Title" text property and a "Show dot" boolean property.
fn card(s: &mut Session) -> NodeId {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    let title = idv(&ex(s, "text.create", json!({"text": "Hello", "x": 10, "y": 10, "parent": f, "props": {"name": "Title"}})));
    let dot = idv(&ex(s, "shape.ellipse", json!({"x": 170, "y": 10, "width": 20, "height": 20, "parent": f, "props": {"name": "Dot"}})));
    let c = idv(&ex(s, "component.create", json!({"ids": [f]})));
    ex(s, "component.addProperty", json!({"id": dot, "name": "Show dot", "type": "boolean", "default": true}));
    ex(s, "component.addProperty", json!({"id": c, "name": "Title", "type": "text", "layer": title}));
    c
}

/// A "Button" set with State=Default|Hover. Returns (set, hover variant).
fn button(s: &mut Session) -> (NodeId, NodeId) {
    let variants: Vec<NodeId> = ["Default", "Hover"]
        .iter()
        .enumerate()
        .map(|(k, state)| {
            let f =
                idv(&ex(s, "frame.create", json!({"x": k as f64 * 150.0, "y": 300, "width": 100, "height": 40, "name": format!("State={state}")})));
            idv(&ex(s, "component.create", json!({"ids": [f]})))
        })
        .collect();
    let set = idv(&ex(s, "component.combineVariants", json!({"ids": variants.clone()})));
    ex(s, "node.set", json!({"id": set, "props": {"name": "Button"}}));
    (set, variants[1])
}

#[test]
fn connected_instances_show_the_real_snippet() {
    let mut s = Session::new();
    let c = card(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Show dot": false, "Title": "Buy"}}));
    let template = "<Card title={title} dot={Show dot} onClick={onClick}>{Show dot ? \"•\" : \"\"}</Card>";
    let v = ex(&mut s, "codeConnect.set", json!({"id": i, "importPath": "@acme/ui", "template": template}));
    assert_eq!(id_of(&v["component"]), Some(c), "stored on the main");
    assert_eq!(v["unknown"], json!(["onClick"]), "code-only names are kept as written");
    let want = "import { Card } from \"@acme/ui\";\n\n<Card title=\"Buy\" dot={false} onClick={onClick}></Card>\n";
    assert_eq!(v["code"], want);
    assert_eq!(code(&mut s, i, "react"), want);
    assert_eq!(ex(&mut s, "inspect.code", json!({"id": i, "language": "react"}))["connected"], true);
    // The main itself: the defaults. Instances never carry links themselves.
    assert_eq!(code(&mut s, c, "react"), "import { Card } from \"@acme/ui\";\n\n<Card title=\"Hello\" dot={true} onClick={onClick}>•</Card>\n");
    assert!(s.doc().unwrap().node(i).unwrap().code_links.is_empty());
    // Escaped per language: a JSX string with escapes is an expression.
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Title": "Say \"hi\""}}));
    assert!(code(&mut s, i, "react").contains("title={\"Say \\\"hi\\\"\"}"));
    ex(
        &mut s,
        "codeConnect.set",
        json!({"id": c, "language": "swiftui", "importPath": "AcmeUI", "template": "Card(title: {title}, style: {Show dot ? \".dotted\" : \".plain\"})"}),
    );
    assert_eq!(code(&mut s, i, "swiftui"), "import AcmeUI\n\nCard(title: \"Say \\\"hi\\\"\", style: .plain)\n");
    assert!(code(&mut s, i, "compose").contains("@Composable"), "other languages stay generated");
    // Leptos: a `use` line and `view!` markup (strings with escapes in braces).
    ex(
        &mut s,
        "codeConnect.set",
        json!({"id": c, "language": "leptos", "importPath": "crate::ui", "template": "<Card title={title} dotted={Show dot} />"}),
    );
    assert_eq!(code(&mut s, i, "leptos"), "use crate::ui::Card;\n\n<Card title={\"Say \\\"hi\\\"\"} dotted={false} />\n");
    assert!(s.execute("codeConnect.set", &json!({"id": c, "language": "leptos", "importPath": "crate/ui"})).is_err(), "a Rust path");
    ex(&mut s, "codeConnect.remove", json!({"id": c, "language": "leptos"}));
    // inspect.componentCode: the snippet as the usage, and the link.
    let v = ex(&mut s, "inspect.componentCode", json!({"id": i, "language": "swiftui"}));
    assert_eq!(v["usage"], "Card(title: \"Say \\\"hi\\\"\", style: .plain)\n");
    assert_eq!(v["connected"]["import"], "import AcmeUI");
    assert_eq!(ex(&mut s, "inspect.componentCode", json!({"id": i, "language": "compose"}))["connected"], Value::Null);
    // Inside a generated layer tree, connected instances are their snippet.
    let screen = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 500, "width": 400, "height": 300, "name": "Screen"})));
    ex(&mut s, "instance.create", json!({"component": c, "x": 20, "y": 30, "parent": screen}));
    assert!(code(&mut s, screen, "react").contains("<Card title=\"Hello\" dot={true} onClick={onClick}>•</Card>\n"));
    // Literal braces stay; fields left out keep their value.
    ex(&mut s, "codeConnect.set", json!({"id": c, "template": "<Card style={{gap: 4}} label={title} />"}));
    assert_eq!(code(&mut s, c, "react"), "import { Card } from \"@acme/ui\";\n\n<Card style={{gap: 4}} label=\"Hello\" />\n");
    // Validation.
    for bad in [
        json!({"id": c, "template": "{title ? \"a\"}"}),
        json!({"id": c, "template": "  "}),
        json!({"id": c, "name": "1Card"}),
        json!({"id": c, "importPath": "a\"b"}),
        json!({"id": c, "language": "swiftui", "importPath": "Acme UI"}),
        json!({"id": c, "language": "html"}),
        json!({"id": c, "template": 5}),
        json!({"id": screen}),
    ] {
        assert!(s.execute("codeConnect.set", &bad).is_err(), "{bad}");
    }
    // Disconnect one language; undo brings it back.
    ex(&mut s, "codeConnect.remove", json!({"id": i, "language": "react"}));
    assert!(code(&mut s, i, "react").contains("export function Card({"));
    assert!(code(&mut s, i, "swiftui").starts_with("import AcmeUI"));
    ex(&mut s, "edit.undo", json!({}));
    assert!(code(&mut s, i, "react").starts_with("import { Card }"));
    assert_eq!(ex(&mut s, "codeConnect.remove", json!({"id": c}))["removed"], 2);
    assert!(s.execute("codeConnect.remove", &json!({"id": c})).is_err(), "nothing left");
}

#[test]
fn sets_connect_through_the_set_and_round_trip_as_json() {
    let mut s = Session::new();
    let (set, hover) = button(&mut s);
    let v = ex(&mut s, "codeConnect.set", json!({"id": hover}));
    assert_eq!(id_of(&v["component"]), Some(set));
    ex(&mut s, "codeConnect.set", json!({"id": set, "language": "swiftui"}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": hover, "x": 0, "y": 500})));
    assert_eq!(code(&mut s, i, "react"), "<Button state=\"Hover\" />\n", "the default template passes every property");
    assert_eq!(code(&mut s, i, "swiftui"), "Button(state: .hover)\n");
    // Export, then import into another file with a Button set.
    let text = ex(&mut s, "codeConnect.export", json!({}))["text"].as_str().unwrap().to_string();
    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(file["version"], 1);
    assert_eq!(
        file["links"][0],
        json!({"component": "Button", "language": "react", "importPath": "", "name": "Button", "template": "<Button state={state} />"})
    );
    let mut t = Session::new();
    let (_, hover2) = button(&mut t);
    let i2 = idv(&ex(&mut t, "instance.create", json!({"component": hover2, "x": 0, "y": 500})));
    let mut links = file["links"].as_array().unwrap().clone();
    links.push(json!({"component": "Nope", "language": "react", "name": "X", "template": "<X />"}));
    links.push(json!({"component": "Button", "language": "react", "name": 5, "template": "<X />"}));
    links.push(json!({"component": "Button", "language": "react", "name": "Bad name", "template": "<X />"}));
    let r = ex(&mut t, "codeConnect.import", json!({"json": {"version": 1, "links": links}}));
    assert_eq!(r["linked"], 2);
    assert_eq!(r["warnings"].as_array().unwrap().len(), 3, "{r}");
    assert_eq!(code(&mut t, i2, "swiftui"), "Button(state: .hover)\n");
    // Through a file, as JSON text.
    let path = std::env::temp_dir().join(format!("figcraft-connect-{}.json", std::process::id()));
    let path = path.to_string_lossy().to_string();
    ex(&mut s, "codeConnect.export", json!({"path": path}));
    let mut u = Session::new();
    button(&mut u);
    assert_eq!(ex(&mut u, "codeConnect.import", json!({"path": path}))["linked"], 2);
    assert_eq!(ex(&mut u, "codeConnect.import", json!({"json": text}))["linked"], 2);
    let _ = std::fs::remove_file(&path);
    // Malformed files are refused whole.
    let too_many: Vec<Value> = (0..=crate::code_connect::MAX_FILE_LINKS).map(|_| json!({})).collect();
    for bad in [
        json!({"json": {"version": 2, "links": []}}),
        json!({"json": {"links": 5}}),
        json!({"json": 5}),
        json!({"json": "{"}),
        json!({"json": {"links": too_many}}),
        json!({}),
    ] {
        assert!(u.execute("codeConnect.import", &bad).is_err(), "{bad}");
    }
}

#[test]
fn code_connect_commands_survive_junk() {
    let junk = [
        json!({}),
        json!(null),
        json!({"id": "x", "language": [1], "template": {"a": 1}, "name": null}),
        json!({"ids": [-1, "0:0"], "importPath": 1e300, "json": [null]}),
        json!({"template": "{".repeat(5000), "name": "\u{0}\u{2028}", "path": 7}),
        json!({"template": "{a ? \"}\" : \"{\"} {{{}}} {?} {\"} {:}", "json": {"links": [null, 5, {"component": [1]}]}}),
    ];
    for cmd in ["codeConnect.set", "codeConnect.remove", "codeConnect.export", "codeConnect.import"] {
        for j in &junk {
            let mut s = Session::new();
            let c = card(&mut s);
            ex(&mut s, "codeConnect.set", json!({"id": c}));
            ex(&mut s, "select.set", json!({"ids": [c]}));
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
}
