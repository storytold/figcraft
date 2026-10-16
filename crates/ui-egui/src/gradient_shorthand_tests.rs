//! The control channel takes the documented gradient shorthand, as MCP does (#252).

use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

#[test]
fn engine_execute_takes_the_gradient_shorthand() {
    let mut u = Ui::new(Session::new());
    let exec = |u: &mut Ui, command: &str, params: serde_json::Value| {
        crate::control::handle(&mut u.app, &u.ctx.clone(), "engine.execute", &json!({"command": command, "params": params}))
    };
    let r = exec(&mut u, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap()["id"].clone();
    let short = json!({"type": "linearGradient", "stops": ["#ff0000", "#0000ff"], "from": [0, 0.5], "to": [1, 0.5]});
    exec(&mut u, "node.set", json!({"ids": [r], "fills": [short]})).unwrap();
    let n = exec(&mut u, "node.get", json!({"id": r, "depth": 0})).unwrap();
    let f = &n["fills"][0];
    assert_eq!((f["type"].clone(), f["kind"].clone(), f["width"].clone()), (json!("gradient"), json!("linear"), json!([0.0, 1.5])), "{n}");
    assert_eq!(f["stops"].as_array().map(Vec::len), Some(2), "{n}");
}
