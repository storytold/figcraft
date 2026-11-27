use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeId;

fn ex(s: &mut Session, cmd: &str, p: Value) -> NodeId {
    id_of(&s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))["id"]).unwrap()
}

struct Board {
    frame: NodeId,
    inner: NodeId,
    rect: NodeId,
}

fn board(s: &mut Session, x: f64) -> Board {
    let frame = ex(s, "frame.create", json!({"x": x, "y": 0, "width": 300, "height": 300}));
    let rect = ex(s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100, "parent": frame}));
    let other = ex(s, "shape.rectangle", json!({"x": 200, "y": 200, "width": 40, "height": 40, "parent": frame}));
    let inner = ex(s, "object.group", json!({"ids": [rect, other]}));
    let extra = ex(s, "shape.rectangle", json!({"x": 250, "y": 20, "width": 20, "height": 20, "parent": frame}));
    ex(s, "object.group", json!({"ids": [inner, extra]}));
    Board { frame, inner, rect }
}

fn send(s: &mut Session, kind: PointerKind, x: f64, y: f64, shift: bool) {
    s.pointer(&PointerEvent { shift, ..PointerEvent::at(kind, x, y, 1.0) }).unwrap();
}

fn double_click(s: &mut Session, x: f64, y: f64, shift: bool) -> Vec<NodeId> {
    send(s, PointerKind::Down, x, y, shift);
    send(s, PointerKind::Up, x, y, shift);
    send(s, PointerKind::Down, x, y, shift);
    send(s, PointerKind::DoubleClick, x, y, shift);
    send(s, PointerKind::Up, x, y, shift);
    send(s, PointerKind::Move, x + 1.0, y, shift);
    let mut sel = s.selection();
    sel.sort();
    sel
}

fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
    v.sort();
    v
}

#[test]
fn shift_double_clicks_go_deeper_in_a_neighbouring_artboard_and_keep_the_rest() {
    let mut s = Session::new();
    let a = board(&mut s, 0.0);
    let b = board(&mut s, 400.0);
    s.execute("select.none", &json!({})).unwrap();

    assert_eq!(double_click(&mut s, 100.0, 100.0, false), vec![a.inner], "a plain double-click enters the artboard");
    assert_eq!(double_click(&mut s, 500.0, 100.0, true), sorted(vec![a.inner, b.inner]), "Shift: the neighbour's layer joins");
    assert_eq!(double_click(&mut s, 500.0, 100.0, true), sorted(vec![a.inner, b.rect]), "Shift again: a level deeper there");
    assert_eq!(double_click(&mut s, 100.0, 100.0, true), sorted(vec![a.rect, b.rect]), "and in the first artboard too");
    assert!(s.ds().unwrap().vector_edit.is_none());
    assert_eq!(s.doc().unwrap().node(a.frame).unwrap().children.len(), 1, "nothing moved out of place");
}

#[test]
fn a_shift_double_click_on_a_neighbouring_row_adds_its_layer_at_the_selected_depth() {
    let mut s = Session::new();
    let section = ex(&mut s, "node.create", json!({"type": "section", "x": -100, "y": -100, "width": 1200, "height": 1000}));
    let artboard = ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 1000, "height": 800}));
    s.execute("node.reparent", &json!({"ids": [artboard], "parent": section})).unwrap();
    let content = ex(&mut s, "frame.create", json!({"x": 100, "y": 100, "width": 600, "height": 400, "parent": artboard}));
    let specs = ex(&mut s, "frame.create", json!({"x": 0, "y": 100, "width": 600, "height": 200, "parent": content}));
    let names: Vec<NodeId> = (0..3)
        .map(|i| {
            let row = ex(&mut s, "frame.create", json!({"x": 30, "y": i * 30, "width": 560, "height": 22, "parent": specs}));
            let name = ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 120, "height": 22, "parent": row}));
            ex(&mut s, "text.create", json!({"text": "Client:", "x": 0, "y": 0, "parent": name}));
            name
        })
        .collect();
    s.execute("select.set", &json!({"ids": [names[0]]})).unwrap();

    assert_eq!(double_click(&mut s, 135.0, 238.0, true), sorted(vec![names[0], names[1]]), "the next row's name joins the selected one");
    assert_eq!(double_click(&mut s, 135.0, 268.0, true), sorted(names.clone()), "and the row after it");
}

#[test]
fn a_shift_double_click_with_the_hand_keeps_a_selection_made_after_earlier_presses() {
    let mut s = Session::new();
    let a = board(&mut s, 0.0);
    let b = board(&mut s, 400.0);
    s.execute("select.set", &json!({"ids": [a.inner]})).unwrap();
    for _ in 0..2 {
        send(&mut s, PointerKind::Down, 100.0, 100.0, false);
        send(&mut s, PointerKind::Up, 100.0, 100.0, false);
    }
    s.execute("select.set", &json!({"ids": [b.inner]})).unwrap();
    s.set_tool(crate::tools::ToolId::Hand);

    assert_eq!(double_click(&mut s, 100.0, 100.0, true), sorted(vec![a.inner, b.inner]), "the Move presses' selection doesn't come back");
}

#[test]
fn a_shift_double_click_without_its_presses_keeps_the_current_selection() {
    let mut s = Session::new();
    let a = board(&mut s, 0.0);
    let b = board(&mut s, 400.0);
    s.execute("select.set", &json!({"ids": [a.inner]})).unwrap();
    for _ in 0..2 {
        send(&mut s, PointerKind::Down, 1000.0, 1000.0, false);
        send(&mut s, PointerKind::Up, 1000.0, 1000.0, false);
    }
    s.execute("select.set", &json!({"ids": [b.inner]})).unwrap();

    send(&mut s, PointerKind::DoubleClick, 100.0, 100.0, true);
    assert_eq!(sorted(s.selection()), sorted(vec![a.inner, b.inner]), "a double-click alone works from the selection it finds");
}
