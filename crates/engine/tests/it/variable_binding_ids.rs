use figcraft_engine::Session;
use serde_json::{Value, json};

/// Variable A is renamed to variable B's id; returns (session, B's id).
fn renamed_to_other_id(a_type: &str, a_value: Value) -> (Session, String) {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    let a = s.execute("variable.create", &json!({"collection": "Theme", "name": "shadow", "type": a_type, "value": a_value})).unwrap();
    let a = a["id"].as_str().unwrap().to_string();
    let b = s.execute("variable.create", &json!({"collection": "Theme", "name": "surface", "type": "color", "value": "#0D99FF"})).unwrap();
    let b = b["id"].as_str().unwrap().to_string();
    s.execute("variable.update", &json!({"variable": a, "name": b})).unwrap();
    (s, b)
}

#[test]
fn a_bound_id_wins_over_a_variable_named_like_it() {
    let (mut s, b) = renamed_to_other_id("color", json!("#FF0000"));
    let r = s.execute("node.create", &json!({"type": "rectangle", "props": {"fill": {"$var": b}}})).unwrap();
    let node = s.execute("node.get", &json!({"id": r["id"]})).unwrap();
    let text = node.to_string();
    assert!(text.contains(&format!("\"boundVariables\":{{\"fill\":\"{b}\"}}")), "{text}");
    assert!(text.contains("#0D99FF"), "{text}");
    assert!(!text.contains("#FF0000"), "{text}");
}

#[test]
fn the_type_check_uses_the_bound_id() {
    let (mut s, b) = renamed_to_other_id("number", json!(4));
    let r = s.execute("node.create", &json!({"type": "rectangle", "props": {"fill": {"$var": b}}}));
    assert!(r.is_ok(), "{r:?}");
}
