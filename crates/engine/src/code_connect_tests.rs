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
    let screen = leptos_arkui_and_trees(&mut s, c, i);
    rejects_bad_links(&mut s, c, screen);
    disconnect_then_undo(&mut s, c, i);
}

fn leptos_arkui_and_trees(s: &mut Session, c: NodeId, i: NodeId) -> NodeId {
    // Leptos: a `use` line and `view!` markup (strings with escapes in braces).
    ex(
        s,
        "codeConnect.set",
        json!({"id": c, "language": "leptos", "importPath": "crate::ui", "template": "<Card title={title} dotted={Show dot} />"}),
    );
    assert_eq!(code(s, i, "leptos"), "use crate::ui::Card;\n\n<Card title={\"Say \\\"hi\\\"\"} dotted={false} />\n");
    // ArkUI: an ArkTS import and an object-literal call.
    ex(
        s,
        "codeConnect.set",
        json!({"id": c, "language": "arkui", "importPath": "@acme/arkui", "template": "Card({ title: {title}, dotted: {Show dot} })"}),
    );
    assert_eq!(code(s, i, "arkui"), "import { Card } from \"@acme/arkui\";\n\nCard({ title: \"Say \\\"hi\\\"\", dotted: false })\n");
    let v = ex(s, "inspect.componentCode", json!({"id": i, "language": "arkui"}));
    assert_eq!(v["usage"], "Card({ title: \"Say \\\"hi\\\"\", dotted: false })\n");
    assert_eq!(v["connected"]["import"], "import { Card } from \"@acme/arkui\";");
    assert!(s.execute("codeConnect.set", &json!({"id": c, "language": "leptos", "importPath": "crate/ui"})).is_err(), "a Rust path");
    ex(s, "codeConnect.remove", json!({"id": c, "language": "leptos"}));
    // inspect.componentCode: the snippet as the usage, and the link.
    let v = ex(s, "inspect.componentCode", json!({"id": i, "language": "swiftui"}));
    assert_eq!(v["usage"], "Card(title: \"Say \\\"hi\\\"\", style: .plain)\n");
    assert_eq!(v["connected"]["import"], "import AcmeUI");
    assert_eq!(ex(s, "inspect.componentCode", json!({"id": i, "language": "compose"}))["connected"], Value::Null);
    // Inside a generated layer tree, connected instances are their snippet.
    let screen = idv(&ex(s, "frame.create", json!({"x": 0, "y": 500, "width": 400, "height": 300, "name": "Screen"})));
    ex(s, "instance.create", json!({"component": c, "x": 20, "y": 30, "parent": screen}));
    assert!(code(s, screen, "react").contains("<Card title=\"Hello\" dot={true} onClick={onClick}>•</Card>\n"));
    // Literal braces stay; fields left out keep their value.
    ex(s, "codeConnect.set", json!({"id": c, "template": "<Card style={{gap: 4}} label={title} />"}));
    assert_eq!(code(s, c, "react"), "import { Card } from \"@acme/ui\";\n\n<Card style={{gap: 4}} label=\"Hello\" />\n");
    screen
}

fn rejects_bad_links(s: &mut Session, c: NodeId, screen: NodeId) {
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
}

fn disconnect_then_undo(s: &mut Session, c: NodeId, i: NodeId) {
    // Disconnect one language; undo brings it back.
    ex(s, "codeConnect.remove", json!({"id": i, "language": "react"}));
    assert!(code(s, i, "react").contains("export function Card({"));
    assert!(code(s, i, "swiftui").starts_with("import AcmeUI"));
    ex(s, "edit.undo", json!({}));
    assert!(code(s, i, "react").starts_with("import { Card }"));
    assert_eq!(ex(s, "codeConnect.remove", json!({"id": c}))["removed"], 3);
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

#[test]
fn vue_links_are_a_default_import_with_quoted_bindings() {
    let mut s = Session::new();
    let c = card(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Show dot": false, "Title": "Buy"}}));
    // A single-file component is a default import. A bound attribute takes an expression, so a
    // string keeps its JS quotes inside the attribute (as entities): `Buy` alone would be a
    // variable, not the string.
    let v = ex(
        &mut s,
        "codeConnect.set",
        json!({"id": i, "language": "vue", "importPath": "@/components/Card.vue",
               "template": "<Card :title={title} :dotted={Show dot} />"}),
    );
    let want = "import Card from \"@/components/Card.vue\";\n\n<Card :title=\"&quot;Buy&quot;\" :dotted=\"false\" />\n";
    assert_eq!(v["code"], want);
    assert_eq!(code(&mut s, i, "vue"), want);
    assert_eq!(ex(&mut s, "inspect.code", json!({"id": i, "language": "vue"}))["connected"], true);
    // No template given: the generated one binds a quoted value for every property.
    ex(&mut s, "codeConnect.remove", json!({"id": c, "language": "vue"}));
    ex(&mut s, "codeConnect.set", json!({"id": c, "language": "vue", "importPath": "./Card.vue"}));
    assert_eq!(code(&mut s, c, "vue"), "import Card from \"./Card.vue\";\n\n<Card :showDot=\"true\" :title=\"&quot;Hello&quot;\" />\n");
    // A plain string attribute (no `:`) takes the text itself, and a variant binding is a string
    // expression like any other.
    ex(&mut s, "codeConnect.set", json!({"id": i, "language": "vue", "template": "<Card title={title} :dotted={Show dot} />"}));
    assert_eq!(code(&mut s, i, "vue"), "import Card from \"./Card.vue\";\n\n<Card title=\"Buy\" :dotted=\"false\" />\n");
    let (set, hover) = button(&mut s);
    let b = idv(&ex(&mut s, "instance.create", json!({"component": hover, "x": 0, "y": 700})));
    ex(&mut s, "codeConnect.set", json!({"id": b, "language": "vue", "template": "<Button :state={state} />"}));
    assert_eq!(code(&mut s, b, "vue"), "<Button :state=\"&quot;Hover&quot;\" />\n");
    assert_eq!(id_of(&ex(&mut s, "codeConnect.set", json!({"id": set}))["component"]), Some(set));
    // A string with escapes stays one value: the attribute delimiter is what gets escaped, so the
    // binding stays a single well-formed attribute.
    ex(&mut s, "codeConnect.set", json!({"id": i, "language": "vue", "template": "<Card :title={title} :dotted={Show dot} />"}));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Title": "Say \"hi\""}}));
    let escaped = code(&mut s, i, "vue");
    let line = escaped.lines().find(|l| l.contains("&quot;")).unwrap_or_default();
    assert!(line.starts_with("<Card :title=\"&quot;Say"), "{line}");
    assert!(line.contains(":dotted=\"false\""), "{line}");
    assert!(line.ends_with("\" />"), "{line}");
    // A single-quoted attribute keeps its delimiter (the expression's double quotes stay), and a
    // placeholder in template content is text, not a quoted string.
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Title": "Buy"}}));
    ex(&mut s, "codeConnect.set", json!({"id": i, "language": "vue", "template": "<Card :title='{title}' :dotted='{Show dot}' />"}));
    assert_eq!(code(&mut s, i, "vue"), "import Card from \"./Card.vue\";\n\n<Card :title='\"Buy\"' :dotted='false' />\n");
    ex(&mut s, "codeConnect.set", json!({"id": i, "language": "vue", "template": "<Card title='{title}'>{title}</Card>"}));
    assert_eq!(code(&mut s, i, "vue"), "import Card from \"./Card.vue\";\n\n<Card title='Buy'>Buy</Card>\n");
    // A second placeholder in the same quoted attribute is still attribute text, and its value's
    // quotes are escaped.
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Title": "Say \"hi\""}}));
    ex(&mut s, "codeConnect.set", json!({"id": i, "language": "vue", "template": "<Card title=\"{title} {title}\" />"}));
    assert_eq!(code(&mut s, i, "vue"), "import Card from \"./Card.vue\";\n\n<Card title=\"Say &quot;hi&quot; Say &quot;hi&quot;\" />\n");
    // Literal text before the first placeholder keeps it inside the attribute value.
    ex(&mut s, "codeConnect.set", json!({"id": i, "language": "vue", "template": "<Card title=\"Say: {title}\" />"}));
    assert_eq!(code(&mut s, i, "vue"), "import Card from \"./Card.vue\";\n\n<Card title=\"Say: Say &quot;hi&quot;\" />\n");
    // Text after a completed tag is text, even when it contains `=`.
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Title": "Buy"}}));
    ex(&mut s, "codeConnect.set", json!({"id": i, "language": "vue", "template": "<p>Total = {title}</p>"}));
    assert_eq!(code(&mut s, i, "vue"), "import Card from \"./Card.vue\";\n\n<p>Total = Buy</p>\n");
    // Vue is linkable; a language that isn't stays refused.
    assert!(s.execute("codeConnect.set", &json!({"id": c, "language": "css"})).is_err(), "not a code language");
    assert!(s.execute("codeConnect.set", &json!({"id": c, "language": "vue", "importPath": "a b"})).is_err(), "no spaces in a path");
}

#[test]
fn vue_layer_trees_import_connected_components_in_script_setup() {
    let mut s = Session::new();
    let c = card(&mut s);
    ex(&mut s, "codeConnect.set", json!({"id": c, "language": "vue", "importPath": "./Card.vue"}));
    let screen = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 500, "width": 400, "height": 300, "name": "Screen"})));
    ex(&mut s, "instance.create", json!({"component": c, "x": 20, "y": 30, "parent": screen}));
    // A single-file component's imports belong in its `<script setup>`, not before the template.
    let code = code(&mut s, screen, "vue");
    let want = "<script setup lang=\"ts\">\n\nimport Card from \"./Card.vue\";\n\n</script>\n\n<template>\n";
    assert!(code.starts_with(want), "{code}");
    assert_eq!(code.matches("import Card").count(), 1, "{code}");
}

#[test]
fn many_entries_for_one_component_each_link() {
    let mut s = Session::new();
    let (_, hover) = button(&mut s);
    let links: Vec<Value> = (0..crate::code_connect::MAX_FILE_LINKS)
        .map(|i| json!({"component": "Button", "language": if i % 2 == 0 { "react" } else { "vue" }, "name": "Button", "template": "<Button />"}))
        .collect();
    let r = ex(&mut s, "codeConnect.import", json!({"json": {"version": 1, "links": links}}));
    assert_eq!(r["linked"], crate::code_connect::MAX_FILE_LINKS, "{r}");
    let i = idv(&ex(&mut s, "instance.create", json!({"component": hover, "x": 0, "y": 500})));
    assert_eq!(code(&mut s, i, "react"), "<Button />\n");
}

#[test]
fn a_link_names_its_source_and_translates_variant_values() {
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, hover) = button(&mut s);
    ex(
        &mut s,
        "codeConnect.set",
        json!({"id": set, "name": "Button", "importPath": "@/ui/button", "template": "<Button state={State} />",
        "source": "src/ui/button.tsx", "values": {"State": {"Hover": "hovered"}}, "notes": "Primary action"}),
    );
    let instance = idv(&ex(&mut s, "instance.create", json!({"component": hover})));
    assert!(code(&mut s, instance, "react").contains(r#"state="hovered""#), "{}", code(&mut s, instance, "react"));
    // A value without a mapping keeps the design's name.
    let plain = idv(&ex(&mut s, "instance.create", json!({"component": set})));
    assert!(code(&mut s, plain, "react").contains(r#"state="Default""#));
}

#[test]
fn a_source_outside_the_repository_is_refused() {
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let c = card(&mut s);
    for source in ["../up.tsx", "/etc/passwd", "a\\b.tsx", "a/../../b.tsx"] {
        let e = s.execute("codeConnect.set", &json!({"id": c, "source": source})).unwrap_err();
        assert!(e.to_string().contains("source"), "{source}: {e}");
    }
    assert!(s.execute("codeConnect.set", &json!({"id": c, "values": 3})).is_err());
    assert!(s.execute("codeConnect.set", &json!({"id": c, "source": "src/card.tsx"})).is_ok());
}

#[test]
fn the_new_fields_survive_a_trip_through_the_file() {
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, _) = button(&mut s);
    ex(
        &mut s,
        "codeConnect.set",
        json!({"id": set, "name": "Button", "template": "<Button />", "source": "src/b.tsx", "values": {"State": {"Hover": "h"}}, "notes": "n"}),
    );
    let text = ex(&mut s, "codeConnect.export", json!({}))["text"].as_str().unwrap().to_string();
    let mut other = Session::new();
    ex(&mut other, "file.new", json!({}));
    let (set2, _) = button(&mut other);
    assert_eq!(ex(&mut other, "codeConnect.import", json!({"json": text}))["linked"], 1);
    let link = &other.doc().unwrap().node(set2).unwrap().code_links[0];
    assert_eq!((link.source.as_str(), link.notes.as_str()), ("src/b.tsx", "n"));
    assert_eq!(link.values["State"]["Hover"], "h");
}

#[test]
fn a_file_from_before_stays_as_it_was() {
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, _) = button(&mut s);
    ex(&mut s, "codeConnect.set", json!({"id": set, "name": "Button", "template": "<Button />"}));
    let v = ex(&mut s, "codeConnect.export", json!({}));
    let parsed: Value = serde_json::from_str(v["text"].as_str().unwrap()).unwrap();
    let keys: Vec<&String> = parsed["links"][0].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["component", "importPath", "language", "name", "template"], "no new keys unless they're used");
}

#[test]
fn listing_answers_what_an_agent_asks() {
    let dir = std::env::temp_dir().join(format!("figcraft-connect-list-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/button.tsx"), "").unwrap();
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, hover) = button(&mut s);
    let c = card(&mut s);
    ex(
        &mut s,
        "codeConnect.set",
        json!({"id": set, "name": "Button", "importPath": "@/ui/button", "template": "<Button />", "source": "src/button.tsx"}),
    );
    let v = ex(&mut s, "codeConnect.list", json!({"root": dir}));
    assert_eq!((v["mapped"].as_u64(), v["unmapped"].as_u64()), (Some(1), Some(1)));
    let b = v["components"].as_array().unwrap().iter().find(|e| e["id"] == json!(set)).unwrap();
    assert_eq!(b["links"][0]["import"], r#"import { Button } from "@/ui/button";"#);
    assert_eq!(b["links"][0]["sourceFound"], true);
    assert!(b["links"][0]["snippet"].as_str().unwrap().contains("<Button"));
    assert!(b["properties"].as_array().unwrap().iter().any(|p| p["type"] == "variant"));
    let only = ex(&mut s, "codeConnect.list", json!({"unmapped": true}));
    assert_eq!(only["components"][0]["id"], json!(c));
    // A layer leads to the component it uses, a frame to those it holds.
    let i = idv(&ex(&mut s, "instance.create", json!({"component": hover})));
    assert_eq!(ex(&mut s, "codeConnect.list", json!({"ids": [i]}))["components"][0]["id"], json!(set));
    ex(&mut s, "codeConnect.set", json!({"id": set, "source": "src/gone.tsx"}));
    assert_eq!(ex(&mut s, "codeConnect.list", json!({"root": dir, "ids": [set]}))["components"][0]["links"][0]["sourceFound"], false);
}

#[test]
fn exporting_keeps_what_other_designs_put_in_the_file() {
    let dir = std::env::temp_dir().join(format!("figcraft-connect-others-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("figcraft.connect.json");
    std::fs::write(&path, r#"{"version":1,"project":{"name":"shop"},"links":[{"component":"Other","language":"react","name":"Other","template":"<Other />"},{"component":"Button","language":"react","name":"Old","template":"<Old />"}]}"#).unwrap();
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, _) = button(&mut s);
    ex(&mut s, "codeConnect.set", json!({"id": set, "name": "Button", "template": "<Button />"}));
    ex(&mut s, "codeConnect.export", json!({"path": path}));
    let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let names: Vec<&str> = v["links"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Button", "Other"], "this design's link wins, the other design's stays");
    assert_eq!(v["project"]["name"], "shop", "keys the command doesn't own stay");
}

#[test]
fn listing_refuses_a_malformed_target_rather_than_reading_the_selection() {
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, _) = button(&mut s);
    ex(&mut s, "select.set", json!({"ids": [set]}));
    for p in [json!({"id": "not-an-id"}), json!({"ids": [set, "nope"]}), json!({"ids": "1"}), json!({"ids": [999_999]})] {
        assert!(s.execute("codeConnect.list", &p).is_err(), "{p}");
    }
}

/// A set called `name` with a variant `prop=value` for each of `values`. Returns (set, variants).
fn set_of(s: &mut Session, name: &str, prop: &str, values: &[&str]) -> (NodeId, Vec<NodeId>) {
    let variants: Vec<NodeId> = values
        .iter()
        .enumerate()
        .map(|(k, v)| {
            let f = idv(&ex(s, "frame.create", json!({"x": k as f64 * 150.0, "y": 600, "width": 100, "height": 40, "name": format!("{prop}={v}")})));
            idv(&ex(s, "component.create", json!({"ids": [f]})))
        })
        .collect();
    let set = idv(&ex(s, "component.combineVariants", json!({"ids": variants.clone()})));
    ex(s, "node.set", json!({"id": set, "props": {"name": name}}));
    (set, variants)
}

#[test]
fn a_condition_reads_the_designs_value_not_its_name_in_code() {
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, variants) = set_of(&mut s, "Toggle", "State", &["On", "Off"]);
    ex(&mut s, "codeConnect.set", json!({"id": set, "name": "Toggle", "template": "{State ? \"enabled\" : \"disabled\"} {State}"}));
    let off = idv(&ex(&mut s, "instance.create", json!({"component": variants[1]})));
    assert!(code(&mut s, off, "react").starts_with("disabled"), "{}", code(&mut s, off, "react"));
    // `inactive` reads as true, but the condition is about the design's Off.
    ex(&mut s, "codeConnect.set", json!({"id": set, "values": {"State": {"Off": "inactive"}}}));
    let out = code(&mut s, off, "react");
    assert!(out.starts_with("disabled") && out.contains("inactive"), "{out}");
}

#[test]
fn a_mapping_keyed_by_a_name_it_cant_use_is_reported() {
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, hover) = button(&mut s);
    let r = ex(
        &mut s,
        "codeConnect.set",
        json!({"id": set, "name": "Button", "template": "<Button state={State} />", "values": {"state": {"Hover": "hovered"}}}),
    );
    assert!(r.to_string().contains("\"state\""), "the lowercase key does nothing, so it's named: {r}");
    let i = idv(&ex(&mut s, "instance.create", json!({"component": hover})));
    assert!(code(&mut s, i, "react").contains(r#"state="Hover""#));
    let r = ex(&mut s, "codeConnect.set", json!({"id": set, "values": {"State": {"Hover": "hovered"}}}));
    assert!(!r.to_string().contains("\"State\""), "{r}");
}

#[test]
fn an_import_with_malformed_values_keeps_the_link_there() {
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, _) = button(&mut s);
    ex(&mut s, "codeConnect.set", json!({"id": set, "name": "Button", "template": "<Button />", "values": {"State": {"Hover": "h"}}}));
    let links = json!([{"component": "Button", "language": "react", "name": "Other", "template": "<Other />", "values": {"State": {"Hover": 7}}}]);
    let r = ex(&mut s, "codeConnect.import", json!({"json": {"version": 1, "links": links}}));
    assert_eq!(r["linked"], 0, "{r}");
    assert!(r["warnings"][0].as_str().unwrap().contains("values"), "{r}");
    let link = &s.doc().unwrap().node(set).unwrap().code_links[0];
    assert_eq!((link.name.as_str(), link.values["State"]["Hover"].as_str()), ("Button", "h"));
}

#[test]
fn exporting_over_a_file_it_cant_read_is_refused() {
    let dir = std::env::temp_dir().join(format!("figcraft-connect-unread-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new();
    ex(&mut s, "file.new", json!({}));
    let (set, _) = button(&mut s);
    ex(&mut s, "codeConnect.set", json!({"id": set, "name": "Button", "template": "<Button />"}));
    let big = vec![b' '; 4 * 1024 * 1024 + 1];
    for (name, bytes) in [("broken.json", b"{\"links\": [".to_vec()), ("list.json", b"[]".to_vec()), ("big.json", big)] {
        let path = dir.join(name);
        std::fs::write(&path, &bytes).unwrap();
        assert!(s.execute("codeConnect.export", &json!({"path": path})).is_err(), "{name}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{name} is left as it was");
    }
    // A file that isn't there yet is written.
    let new = dir.join("figcraft.connect.json");
    let _ = std::fs::remove_file(&new);
    ex(&mut s, "codeConnect.export", json!({"path": new}));
    assert!(new.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
