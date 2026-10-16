#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use figcraft_engine::Session;
use serde_json::{Value, json};

fn rectangle(s: &mut Session, x: f64) -> Value {
    s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap()["id"].clone()
}

#[test]
fn merge_response_includes_outlines_for_no_op() {
    let mut s = Session::new();
    let id = rectangle(&mut s, 0.0);
    let regions = s.execute("shapeBuilder.regions", &json!({"ids": [id]})).unwrap();
    assert_eq!(regions["regions"].as_array().unwrap().len(), 1);
    let before = serde_json::to_value(s.doc().unwrap()).unwrap();
    let result = s.execute("shapeBuilder.merge", &json!({"ids": [id], "regions": [0]})).unwrap();
    assert_eq!(serde_json::to_value(s.doc().unwrap()).unwrap(), before);
    assert_eq!(result["id"], Value::Null);
    assert_eq!(result["changed"], json!([]));
    assert_eq!(result["removed"], json!([]));
    assert_eq!(result["regions"], json!([0]));
    assert_eq!(result.get("outlines"), Some(&json!([])));
}

#[test]
fn merge_response_control_includes_outlines() {
    let mut s = Session::new();
    let a = rectangle(&mut s, 0.0);
    let b = rectangle(&mut s, 50.0);
    let regions = s.execute("shapeBuilder.regions", &json!({"ids": [a, b]})).unwrap();
    assert_eq!(regions["regions"].as_array().unwrap().len(), 3);
    let result = s.execute("shapeBuilder.merge", &json!({"ids": [a, b], "regions": [1]})).unwrap();
    assert!(result["id"].is_u64());
    assert_eq!(result.get("outlines"), Some(&json!([])));
    assert_eq!(result.as_object().unwrap().len(), 5);
}
