//! The documented gradient shorthand (`{"type":"linearGradient","stops":["#hex",…]}`) works
//! through every route into the engine, not only MCP (#252): `figcraft-cli run` and the control
//! channel send params straight to `Session::execute`.

use figcraft_color::Paint;
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;
use crate::params::{id_of, paint_of};

/// The red→blue left-to-right gradient as written in the docs.
fn short() -> Value {
    json!({"type": "linearGradient", "stops": ["#ff0000", "#0000ff"], "from": [0, 0.5], "to": [1, 0.5]})
}

/// The same gradient in the engine's own form.
fn long() -> Value {
    json!({"type": "gradient", "kind": "linear",
        "stops": [{"position": 0, "color": {"r": 1, "g": 0, "b": 0, "a": 1}},
                  {"position": 1, "color": {"r": 0, "g": 0, "b": 1, "a": 1}}],
        "from": [0, 0.5], "to": [1, 0.5], "width": [0, 1.5]})
}

fn rect(s: &mut Session) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap()["id"]).unwrap()
}

fn fills(s: &Session, id: NodeId) -> Vec<Paint> {
    s.doc().unwrap().node(id).unwrap().fills.clone()
}

#[test]
fn the_shorthand_reads_as_the_long_form() {
    assert_eq!(paint_of(&short()).expect("the documented shorthand is a paint"), paint_of(&long()).unwrap());
}

#[test]
fn node_set_takes_the_shorthand() {
    let mut s = Session::new();
    let r = rect(&mut s);
    s.execute("node.set", &json!({"ids": [r], "fills": [short()]})).unwrap();
    s.execute("node.set", &json!({"ids": [r], "strokes": [short()]})).unwrap();
    let want = paint_of(&long()).unwrap();
    assert_eq!(fills(&s, r), vec![want.clone()]);
    assert_eq!(s.doc().unwrap().node(r).unwrap().strokes, vec![want]);
}

#[test]
fn node_create_fill_add_and_fill_set_take_the_shorthand() {
    let want = paint_of(&long()).unwrap();
    let mut s = Session::new();
    let v = s.execute("node.create", &json!({"type": "rectangle", "width": 200, "height": 100, "props": {"fills": [short()]}})).unwrap();
    let r = id_of(&v["id"]).unwrap();
    assert_eq!(fills(&s, r), vec![want.clone()]);
    s.execute("fill.add", &json!({"ids": [r], "paint": short()})).unwrap();
    assert_eq!(fills(&s, r), vec![want.clone(), want.clone()]);
    let r2 = rect(&mut s);
    s.execute("fill.set", &json!({"ids": [r2], "index": 0, "paint": short()})).unwrap();
    assert_eq!(fills(&s, r2), vec![want]);
}

#[test]
fn shorthand_defaults_match_the_mcp_server() {
    // No handles: top to bottom, with a square width handle.
    let p = paint_of(&json!({"type": "linearGradient", "stops": ["#000000", "#ffffff"]})).unwrap();
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(
        (v["kind"].clone(), v["from"].clone(), v["to"].clone(), v["width"].clone()),
        (json!("linear"), json!([0.5, 0.0]), json!([0.5, 1.0]), json!([-0.5, 0.0]))
    );
    // Plain colour stops are spread evenly.
    let p = paint_of(&json!({"type": "radialGradient", "stops": ["#000000", "#808080", "#ffffff"]})).unwrap();
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["kind"], "radial");
    let pos: Vec<f64> = v["stops"].as_array().unwrap().iter().map(|s| s["position"].as_f64().unwrap()).collect();
    assert_eq!(pos, vec![0.0, 0.5, 1.0]);
    for kind in ["angular", "diamond"] {
        let p = paint_of(&json!({"type": format!("{kind}Gradient"), "stops": ["#000000", "#ffffff"]})).unwrap();
        assert_eq!(serde_json::to_value(&p).unwrap()["kind"], kind);
    }
    // A CSS colour inside a solid paint or a positioned stop is read too.
    assert_eq!(paint_of(&json!({"type": "solid", "color": "#00ff00"})), paint_of(&json!("#00ff00")));
    let p = paint_of(&json!({"type": "gradient", "stops": [{"position": 0, "color": "#ff0000"}, {"position": 1, "color": "#0000ff"}], "from": [0, 0.5], "to": [1, 0.5]})).unwrap();
    assert_eq!(p, paint_of(&long()).unwrap());
}

#[test]
fn junk_is_still_refused() {
    let mut s = Session::new();
    let r = rect(&mut s);
    for junk in [
        json!({"type": "linearGradient", "stops": "red"}),
        json!({"type": "linearGradient", "stops": [42]}),
        json!({"type": "fooGradient", "stops": ["#ff0000"]}),
        json!({"type": "linearGradient", "stops": ["#ff0000"], "from": "left"}),
    ] {
        let e = s.execute("node.set", &json!({"ids": [r], "fills": [junk.clone()]})).unwrap_err().to_string();
        assert!(e.contains("`fills` must be paints or colours"), "{junk}: {e}");
    }
    // Deep nesting doesn't overflow the stack.
    let mut deep = json!("#ff0000");
    for _ in 0..200 {
        deep = json!({"type": "linearGradient", "stops": [{"position": 0, "color": deep}]});
    }
    assert!(paint_of(&deep).is_none());
}
