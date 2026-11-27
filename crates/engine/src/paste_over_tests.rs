//! Shift+Cmd/Ctrl+V with several layers selected pastes after the last of them as seen.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn frame(s: &mut Session, x: f64, y: f64) -> NodeId {
    id(&s.execute("frame.create", &json!({"x": x, "y": y, "width": 200, "height": 100})).unwrap())
}

fn kids(s: &Session, parent: NodeId) -> Vec<NodeId> {
    s.doc().unwrap().node(parent).unwrap().children.clone()
}

fn paste_over(s: &mut Session, sel: Vec<NodeId>) -> Vec<NodeId> {
    s.set_selection(sel);
    s.execute("edit.pasteOver", &json!({})).unwrap()["ids"].as_array().unwrap().iter().filter_map(id_of).collect()
}

fn top_left(s: &Session, n: NodeId) -> (f64, f64) {
    let b = s.doc().unwrap().world_bounds(n).unwrap();
    (b.x0, b.y0)
}

#[test]
fn several_rows_of_an_auto_layout_selected_get_the_copy_after_the_last_row() {
    let mut s = Session::new();
    let rows: Vec<NodeId> =
        (0..4).map(|k| id(&s.execute("shape.rectangle", &json!({"x": 0, "y": k * 30, "width": 40, "height": 20})).unwrap())).collect();
    let r = s.execute("layout.add", &json!({"ids": rows, "direction": "vertical"})).unwrap();
    let list = id_of(&r["ids"][0]).unwrap();
    assert_eq!(kids(&s, list), rows);
    s.execute("edit.copy", &json!({"ids": [rows[0]]})).unwrap();
    // Clicked from the middle down and back: the click order isn't the order on screen.
    let pasted = paste_over(&mut s, vec![rows[1], rows[3], rows[2]]);
    assert_eq!(pasted.len(), 1);
    assert_eq!(kids(&s, list), vec![rows[0], rows[1], rows[2], rows[3], pasted[0]], "after the last selected row");
    assert_eq!(s.selection(), pasted);
}

#[test]
fn several_artboards_in_a_column_get_the_copy_below_the_lowest_with_their_gap() {
    let mut s = Session::new();
    let (a, b, c) = (frame(&mut s, 0.0, 0.0), frame(&mut s, 0.0, 140.0), frame(&mut s, 0.0, 280.0));
    let source = frame(&mut s, 1000.0, 0.0);
    s.execute("edit.copy", &json!({"ids": [source]})).unwrap();
    let pasted = paste_over(&mut s, vec![a, c, b]);
    assert_eq!(pasted.len(), 1);
    assert_eq!(top_left(&s, pasted[0]), (0.0, 420.0), "below the lowest artboard, 40 below it like the others");
    let page = s.doc().unwrap().node(c).unwrap().parent.unwrap();
    let order = kids(&s, page);
    let at = |n: NodeId| order.iter().position(|i| *i == n).unwrap();
    assert_eq!(at(pasted[0]), at(c) + 1, "just above the lowest artboard");
}

#[test]
fn several_artboards_in_a_row_get_the_copy_right_of_the_rightmost_with_their_gap() {
    let mut s = Session::new();
    let (a, b, c) = (frame(&mut s, 0.0, 0.0), frame(&mut s, 260.0, 0.0), frame(&mut s, 520.0, 0.0));
    let source = frame(&mut s, 0.0, 1000.0);
    s.execute("edit.copy", &json!({"ids": [source]})).unwrap();
    let pasted = paste_over(&mut s, vec![b, a, c]);
    assert_eq!(top_left(&s, pasted[0]), (780.0, 0.0), "right of the rightmost, 60 from it like the others");
}

#[test]
fn several_overlapping_layers_get_the_copy_right_under_the_lowest() {
    let mut s = Session::new();
    let (a, b) = (frame(&mut s, 0.0, 0.0), frame(&mut s, 50.0, 30.0));
    let source = frame(&mut s, 1000.0, 0.0);
    s.execute("edit.copy", &json!({"ids": [source]})).unwrap();
    let pasted = paste_over(&mut s, vec![b, a]);
    assert_eq!(top_left(&s, pasted[0]), (50.0, 130.0));
}

#[test]
fn several_absolute_layers_of_an_auto_layout_get_the_copy_below_the_lowest() {
    let mut s = Session::new();
    let rows: Vec<NodeId> =
        (0..3).map(|k| id(&s.execute("shape.rectangle", &json!({"x": 0, "y": k * 30, "width": 40, "height": 20})).unwrap())).collect();
    s.execute("layout.add", &json!({"ids": rows, "direction": "vertical"})).unwrap();
    s.execute("node.set", &json!({"ids": rows, "props": {"absolute": true}})).unwrap();
    s.execute("edit.copy", &json!({"ids": [rows[0]]})).unwrap();
    let (x, y0) = top_left(&s, rows[1]);
    let (_, y1) = top_left(&s, rows[2]);
    let pasted = paste_over(&mut s, vec![rows[2], rows[0], rows[1]]);
    assert_eq!(pasted.len(), 1);
    assert_eq!(top_left(&s, pasted[0]), (x, y1 + 20.0 + (y1 - y0 - 20.0)), "the layout doesn't place them, so below the lowest");
}
