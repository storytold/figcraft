use figcraft_doc::NodeId;
use serde_json::json;

use crate::Session;
use crate::tools::{PointerEvent, PointerKind};

fn rect(s: &mut Session, x: f64, props: serde_json::Value) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 100, "height": 100, "props": props})).unwrap();
    crate::params::id_of(&r["id"]).unwrap()
}

fn hex(paints: &[figcraft_color::Paint]) -> Vec<String> {
    paints.iter().map(|p| p.solid_color().map(|c| c.to_hex()).unwrap_or_default()).collect()
}

fn pick_red_onto(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    s.execute("edit.pickColor", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 50.0, 50.0, 1.0)).unwrap();
}

fn scene() -> Session {
    let mut s = Session::new();
    s.execute("page.setBackground", &json!({"color": "#FFFFFF"})).unwrap();
    rect(&mut s, 0.0, json!({"fills": ["#FF0000"]}));
    s
}

#[test]
fn a_layer_with_only_a_stroke_takes_the_pick_on_its_stroke() {
    let mut s = scene();
    let outline = rect(&mut s, 200.0, json!({"fills": [], "strokes": ["#0000FF"], "strokeWeight": 2}));
    pick_red_onto(&mut s, &[outline]);
    let n = s.doc().unwrap().node(outline).unwrap().clone();
    assert!(n.fills.is_empty(), "no fill appears: {:?}", n.fills);
    assert_eq!(hex(&n.strokes), ["FF0000"]);
    assert_eq!(n.stroke.weight, 2.0, "the stroke keeps its weight");
}

#[test]
fn a_hidden_fill_counts_as_none_and_a_visible_one_takes_the_pick() {
    let mut s = scene();
    let hidden = rect(&mut s, 200.0, json!({"fills": [{"type": "solid", "color": "#00FF00", "visible": false}], "strokes": ["#0000FF"]}));
    let filled = rect(&mut s, 400.0, json!({"fills": ["#00FF00"], "strokes": ["#0000FF"]}));
    let bare = rect(&mut s, 600.0, json!({"fills": [], "strokes": []}));
    let undo = s.undo_depth().0;
    pick_red_onto(&mut s, &[hidden, filled, bare]);
    let d = s.doc().unwrap();
    let (h, f, b) = (d.node(hidden).unwrap(), d.node(filled).unwrap(), d.node(bare).unwrap());
    assert_eq!((hex(&h.strokes), h.fills.iter().any(|p| p.visible)), (vec!["FF0000".to_string()], false));
    assert_eq!((hex(&f.fills), hex(&f.strokes)), (vec!["FF0000".to_string()], vec!["0000FF".to_string()]));
    assert_eq!(hex(&b.fills), ["FF0000"], "with neither, the fill as before");
    assert_eq!(s.undo_depth().0, undo + 1, "one step for the whole selection");
    s.execute("edit.undo", &json!({})).unwrap();
    let d = s.doc().unwrap();
    assert_eq!(hex(&d.node(hidden).unwrap().strokes), ["0000FF"]);
    assert_eq!(hex(&d.node(filled).unwrap().fills), ["00FF00"]);
}
