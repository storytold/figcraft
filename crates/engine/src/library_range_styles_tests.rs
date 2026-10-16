//! Text styles linked on character ranges travel with components (publish and place).

use crate::Session;
use figcraft_doc::NodeId;
use serde_json::{Value, json};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap_or_else(|e| panic!("{command}: {e}"))
}

fn node(value: Value) -> NodeId {
    NodeId(value["id"].as_u64().unwrap())
}

/// A component with "Hello" (chars 0..2 linked to a range style) and "World" (whole-layer style).
fn fixture() -> (Session, NodeId, String, String) {
    let mut s = Session::new();
    let frame = node(run(&mut s, "frame.create", json!({"name":"Card", "width":100, "height":40})));
    let text = node(run(&mut s, "text.create", json!({"parent":frame, "text":"Hello"})));
    let range = run(&mut s, "style.create", json!({"type":"text", "name":"Emphasis", "value":{"fontSize":32}}))["id"].as_str().unwrap().to_owned();
    run(&mut s, "style.apply", json!({"id":text, "style":range, "start":0, "end":2}));
    let whole_text = node(run(&mut s, "text.create", json!({"parent":frame, "text":"World"})));
    let whole = run(&mut s, "style.create", json!({"type":"text", "name":"Whole", "value":{"fontSize":24}}))["id"].as_str().unwrap().to_owned();
    run(&mut s, "style.apply", json!({"id":whole_text, "style":whole}));
    let component = node(run(&mut s, "component.create", json!({"ids":[frame]})));
    let source = s.doc().unwrap().node(text).unwrap();
    let chars = source.kind.text().unwrap();
    assert!(source.styles.text.is_none());
    assert_eq!(chars.style_at(0).style_id.as_deref(), Some(range.as_str()));
    assert_eq!(chars.style_at(2).style_id, None);
    (s, component, range, whole)
}

#[test]
fn component_publish_keeps_range_text_style() {
    let (mut s, component, range, whole) = fixture();
    run(&mut s, "library.publish", json!({"include":[component]}));
    let snapshot = &s.doc().unwrap().published.as_ref().unwrap().doc;
    assert!(snapshot.style(&whole).is_some());
    assert!(snapshot.style(&range).is_some(), "character-range style missing from snapshot");
}

#[test]
fn component_place_imports_range_text_style() {
    let (mut source, component, range, whole) = fixture();
    let publication = run(&mut source, "library.publish", json!({}));
    let library = source.doc().unwrap().published.as_ref().unwrap().clone();
    assert!(library.doc.style(&range).is_some());
    let mut target = Session::new();
    let path = "figcraft://library.test/range-styles";
    target.libraries.insert(path, Ok(library));
    run(&mut target, "library.add", json!({"path":path}));
    let instance = node(run(&mut target, "library.place", json!({"library":publication["id"], "component":component})));
    let doc = target.doc().unwrap();
    let children = &doc.node(instance).unwrap().children;
    let ranged = doc.node(children[0]).unwrap().kind.text().unwrap();
    let full = doc.node(children[1]).unwrap().kind.text().unwrap();
    let whole_record = doc.libraries.styles.iter().find(|r| r.key == whole).unwrap();
    assert_eq!(full.style_at(0).style_id.as_deref(), Some(whole_record.local.as_str()));
    assert_eq!(ranged.style_at(0).font_size, 32.0);
    assert!(ranged.style_at(2).style_id.is_none());
    let record = doc.libraries.styles.iter().find(|r| r.key == range);
    assert!(record.is_some(), "character-range style missing after placement");
    assert_eq!(ranged.style_at(0).style_id.as_deref(), record.map(|r| r.local.as_str()));
    assert!(doc.style(ranged.style_at(0).style_id.as_deref().unwrap()).is_some());
}
