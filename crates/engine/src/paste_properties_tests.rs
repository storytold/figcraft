//! Copy properties and Paste properties (Alt+Cmd/Ctrl+C and V) carry what a layer's look comes
//! from as well as its values: its colour, effect and grid styles, the variables bound to what is
//! copied and a frame's layout grids. A link the file pasted into doesn't have is left out.

use figcraft_doc::{LayoutGrid, NodeId};
use serde_json::{Value, json};

use crate::Session;

fn id(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn frame(s: &mut Session, x: f64) -> NodeId {
    id(s.execute("frame.create", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap())
}

fn copy_onto(s: &mut Session, from: NodeId, to: NodeId) {
    s.execute("edit.copyProperties", &json!({"id": from})).unwrap();
    s.execute("edit.pasteProperties", &json!({"ids": [to]})).unwrap();
}

#[test]
fn a_frame_pastes_its_styles_variables_and_layout_grids() {
    let mut s = Session::new();
    let (a, b, c) = (frame(&mut s, 0.0), frame(&mut s, 200.0), frame(&mut s, 400.0));
    s.execute("variable.createCollection", &json!({"name": "Tokens"})).unwrap();
    for (name, value) in [("surface", "#FF0000"), ("other", "#0000FF")] {
        s.execute("variable.create", &json!({"collection": "Tokens", "name": name, "type": "color", "value": value})).unwrap();
    }
    s.execute("variable.create", &json!({"collection": "Tokens", "name": "radius", "type": "number", "value": 16})).unwrap();
    s.execute("variable.bind", &json!({"ids": [a], "field": "fills/0/color", "variable": "surface"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [a], "field": "cornerRadius", "variable": "radius"})).unwrap();
    s.execute("node.set", &json!({"ids": [a], "props": {"grids": [serde_json::to_value(LayoutGrid::default()).unwrap()]}})).unwrap();
    // B is bound to another colour, which the paste replaces.
    s.execute("variable.bind", &json!({"ids": [b], "field": "fills/0/color", "variable": "other"})).unwrap();
    let shadow = s.execute("style.create", &json!({"type": "effect", "name": "Shadow", "value": [{"type": "dropShadow"}]})).unwrap()["id"].clone();
    let green = s.execute("style.create", &json!({"type": "paint", "name": "Green", "value": "#00FF00"})).unwrap()["id"].clone();
    s.execute("style.apply", &json!({"ids": [c], "style": green})).unwrap();
    s.execute("style.apply", &json!({"ids": [c], "style": shadow})).unwrap();

    copy_onto(&mut s, a, b);
    let (na, nb) = (s.doc().unwrap().node(a).unwrap().clone(), s.doc().unwrap().node(b).unwrap().clone());
    assert_eq!(nb.bound.get("fills/0/color"), na.bound.get("fills/0/color"), "the fill follows the same variable");
    assert_eq!(nb.bound.get("cornerRadius"), na.bound.get("cornerRadius"), "and so do the corners");
    assert_eq!(nb.corners.radii, [16.0; 4]);
    assert_eq!(nb.grids, na.grids, "a frame's layout grids come along");

    copy_onto(&mut s, c, a);
    let (na, nc) = (s.doc().unwrap().node(a).unwrap().clone(), s.doc().unwrap().node(c).unwrap().clone());
    assert_eq!(na.styles.fill, nc.styles.fill, "the fill style is linked");
    assert!(nc.styles.effect.is_some() && na.styles.effect == nc.styles.effect, "the effect style too");
    assert!(!na.bound.contains_key("fills/0/color"), "the style replaces the fill's variable");
    assert!(!na.bound.contains_key("cornerRadius"), "C's corners aren't bound, so A's aren't any more");
    assert!(na.grids.is_empty(), "nor has it layout grids");
    assert_eq!(na.fills, nc.fills);
}

#[test]
fn links_another_file_lacks_paste_as_values() {
    let mut s = Session::new();
    let a = frame(&mut s, 0.0);
    s.execute("variable.createCollection", &json!({"name": "Tokens"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Tokens", "name": "surface", "type": "color", "value": "#FF0000"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [a], "field": "fills/0/color", "variable": "surface"})).unwrap();
    let st = s.execute("style.create", &json!({"type": "paint", "name": "Line", "value": "#00FF00"})).unwrap()["id"].clone();
    s.execute("node.set", &json!({"ids": [a], "props": {"strokes": [{"type": "solid", "color": "#000000"}]}})).unwrap();
    s.execute("style.apply", &json!({"ids": [a], "style": st, "target": "stroke"})).unwrap();
    s.execute("edit.copyProperties", &json!({"id": a})).unwrap();
    let copied = s.doc().unwrap().node(a).unwrap().clone();
    s.execute("file.new", &json!({})).unwrap();
    let b = frame(&mut s, 0.0);
    // This file's own style and variable come out with the copied ids: they are not the copied ones.
    s.execute("variable.createCollection", &json!({"name": "Theirs"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theirs", "name": "ink", "type": "color", "value": "#0000FF"})).unwrap();
    s.execute("style.create", &json!({"type": "paint", "name": "Ink", "value": "#0000FF"})).unwrap();
    let doc = s.doc().unwrap();
    let vid = copied.bound["fills/0/color"].as_str();
    assert!(doc.style(st.as_str().unwrap()).is_some() && doc.variables.get_by_id(vid).is_some(), "the ids are taken here");
    s.execute("edit.pasteProperties", &json!({"ids": [b]})).unwrap();
    let nb = s.doc().unwrap().node(b).unwrap();
    assert_eq!((nb.fills.clone(), nb.strokes.clone()), (copied.fills, copied.strokes), "the values arrive");
    assert!(nb.bound.is_empty() && nb.styles.fill.is_none() && nb.styles.stroke.is_none(), "{:?} {:?}", nb.bound, nb.styles);
}

#[test]
fn a_missing_style_unlinks_the_layers_own() {
    let mut s = Session::new();
    let (a, b) = (frame(&mut s, 0.0), frame(&mut s, 200.0));
    // B's own style gives the same colour, so the paste changes no value that would unlink it.
    let gone = s.execute("style.create", &json!({"type": "paint", "name": "Gone", "value": "#00FF00"})).unwrap()["id"].clone();
    let own = s.execute("style.create", &json!({"type": "paint", "name": "Own", "value": "#00FF00"})).unwrap()["id"].clone();
    s.execute("style.apply", &json!({"ids": [a], "style": gone, "target": "stroke"})).unwrap();
    s.execute("style.apply", &json!({"ids": [b], "style": own, "target": "stroke"})).unwrap();
    s.execute("edit.copyProperties", &json!({"id": a})).unwrap();
    s.execute("style.delete", &json!({"style": gone})).unwrap();

    s.execute("edit.pasteProperties", &json!({"ids": [b]})).unwrap();
    let nb = s.doc().unwrap().node(b).unwrap();
    assert!(nb.styles.stroke.is_none(), "A's strokes had no style here, so B's aren't linked: {:?}", nb.styles);
}

#[test]
fn a_binding_on_what_the_copy_doesnt_carry_stays() {
    let mut s = Session::new();
    let a = frame(&mut s, 0.0);
    let t = id(s.execute("text.create", &json!({"text": "Hi", "x": 0, "y": 200})).unwrap());
    s.execute("variable.createCollection", &json!({"name": "Tokens"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Tokens", "name": "body", "type": "number", "value": 18})).unwrap();
    s.execute("variable.bind", &json!({"ids": [t], "field": "fontSize", "variable": "body"})).unwrap();
    let before = s.doc().unwrap().node(t).unwrap().bound.get("fontSize").cloned();

    copy_onto(&mut s, a, t);
    let nt = s.doc().unwrap().node(t).unwrap();
    assert!(before.is_some() && nt.bound.get("fontSize") == before.as_ref(), "a frame carries no font size: {:?}", nt.bound);
}
