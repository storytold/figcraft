//! Dragging layers across nested auto layout frames, and fill/hug sizing that stays coherent.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::{NodeId, Sizing};
use figcraft_geom::{Point, Rect};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, parent: Option<NodeId>, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(s.execute("shape.rectangle", &p).unwrap())
}

fn frame(s: &mut Session, parent: Option<NodeId>, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(s.execute("frame.create", &p).unwrap())
}

fn set(s: &mut Session, node: NodeId, props: Value) {
    s.execute("node.set", &json!({"id": node, "props": props})).unwrap();
}

fn bounds(s: &Session, node: NodeId) -> Rect {
    s.doc().unwrap().world_bounds(node).unwrap()
}

fn kids(s: &Session, node: NodeId) -> Vec<NodeId> {
    s.doc().unwrap().node(node).unwrap().children.clone()
}

fn parent(s: &Session, node: NodeId) -> NodeId {
    s.doc().unwrap().node(node).unwrap().parent.unwrap()
}

fn sizing(s: &Session, node: NodeId) -> (Sizing, Sizing) {
    let cl = &s.doc().unwrap().node(node).unwrap().child_layout;
    (cl.sizing_h, cl.sizing_v)
}

/// Select a nested layer first (a plain click picks the frame's top-level child).
fn select(s: &mut Session, node: NodeId) {
    s.execute("select.set", &json!({"ids": [node]})).unwrap();
}

fn ev(kind: PointerKind, p: Point) -> PointerEvent {
    PointerEvent::at(kind, p.x, p.y, 1.0)
}

/// Press on `from`, move halfway and to `to`, and leave the button down (the caller checks the
/// overlay, then releases with [`release`]).
fn drag_to(s: &mut Session, from: Point, to: Point) {
    s.pointer(&ev(PointerKind::Down, from)).unwrap();
    s.pointer(&ev(PointerKind::Move, from.lerp(to, 0.5))).unwrap();
    s.pointer(&ev(PointerKind::Move, to)).unwrap();
}

fn release(s: &mut Session, at: Point) {
    s.pointer(&ev(PointerKind::Up, at)).unwrap();
}

fn drag(s: &mut Session, from: Point, to: Point) {
    drag_to(s, from, to);
    release(s, to);
}

struct Nest {
    outer: NodeId,
    a: NodeId,
    b: NodeId,
    inner: NodeId,
    x: NodeId,
    y: NodeId,
}

/// A vertical auto layout column (fixed 300 wide, hugging its height) holding rectangles `a` and
/// `b` and a horizontal auto layout row `inner` (hugging) with rectangles `x` and `y`.
fn nest(s: &mut Session) -> Nest {
    let outer = frame(s, None, 0.0, 0.0, 300.0, 300.0);
    let a = rect(s, Some(outer), 0.0, 0.0, 120.0, 40.0);
    let b = rect(s, Some(outer), 0.0, 50.0, 120.0, 40.0);
    let inner = frame(s, Some(outer), 0.0, 100.0, 200.0, 60.0);
    let x = rect(s, Some(inner), 0.0, 0.0, 50.0, 50.0);
    let y = rect(s, Some(inner), 60.0, 0.0, 50.0, 50.0);
    s.execute("layout.add", &json!({"ids": [inner], "direction": "horizontal"})).unwrap();
    set(s, inner, json!({"padding": 10, "gap": 10}));
    s.execute("layout.add", &json!({"ids": [outer], "direction": "vertical"})).unwrap();
    set(s, outer, json!({"padding": 10, "gap": 10, "sizingH": "fixed", "width": 300}));
    s.execute("select.none", &json!({})).unwrap();
    Nest { outer, a, b, inner, x, y }
}

#[test]
fn drag_into_a_nested_auto_layout_frame_takes_the_slot_there() {
    let mut s = Session::new();
    let n = nest(&mut s);
    assert_eq!(kids(&s, n.outer), vec![n.a, n.b, n.inner]);
    let undo_depth = s.ds().unwrap().history.undo.len();
    // A goes between x and y: just left of y's centre.
    let to = Point::new(bounds(&s, n.y).x0 + 5.0, bounds(&s, n.y).center().y);
    let from = bounds(&s, n.a).center();
    drag_to(&mut s, from, to);
    let ov = &s.tool.overlay;
    assert_eq!(ov.drop_target, Some(n.inner), "the receiving frame is outlined");
    let line = ov.insert_line.expect("insertion line");
    assert!(line[0] == line[2] && line[0] < bounds(&s, n.y).x0, "a vertical line before y: {line:?}");
    // The preview shows a in its slot already; the line marks it there, mid-gap (gap 10).
    assert_eq!(line[0], bounds(&s, n.a).x0 - 5.0);
    assert!(ov.guides.is_empty());
    release(&mut s, to);
    assert_eq!(parent(&s, n.a), n.inner);
    assert_eq!(kids(&s, n.inner), vec![n.x, n.a, n.y]);
    assert_eq!(kids(&s, n.outer), vec![n.b, n.inner]);
    // Laid out in the row: x, then a, then y.
    assert!(bounds(&s, n.a).x0 > bounds(&s, n.x).x1 && bounds(&s, n.y).x0 > bounds(&s, n.a).x1);
    let ov = &s.tool.overlay;
    assert!(ov.insert_line.is_none() && ov.drop_target.is_none(), "the overlay clears on release");
    // One undo step puts everything back.
    assert_eq!(s.ds().unwrap().history.undo.len(), undo_depth + 1);
    s.undo().unwrap();
    assert_eq!(kids(&s, n.outer), vec![n.a, n.b, n.inner]);
    assert_eq!(kids(&s, n.inner), vec![n.x, n.y]);
}

#[test]
fn drag_out_of_a_nested_frame_into_its_parent_layout() {
    let mut s = Session::new();
    let n = nest(&mut s);
    // x goes between a and b in the outer column (just above b's centre, left of the row).
    let to = Point::new(bounds(&s, n.b).x0 + 5.0, bounds(&s, n.b).center().y - 5.0);
    select(&mut s, n.x);
    let from = bounds(&s, n.x).center();
    drag_to(&mut s, from, to);
    assert_eq!(s.tool.overlay.drop_target, Some(n.outer));
    let line = s.tool.overlay.insert_line.expect("insertion line");
    assert!(line[1] == line[3], "a horizontal line in a column: {line:?}");
    release(&mut s, to);
    assert_eq!(kids(&s, n.outer), vec![n.a, n.x, n.b, n.inner]);
    assert_eq!(kids(&s, n.inner), vec![n.y]);
    // Still within a few units of its own row (and over nothing deeper): it stays there.
    let row = bounds(&s, n.inner);
    select(&mut s, n.y);
    let from = bounds(&s, n.y).center();
    drag(&mut s, from, Point::new(row.x1 + 4.0, row.center().y));
    assert_eq!(parent(&s, n.y), n.inner);
}

#[test]
fn the_dragged_frame_never_receives_itself_and_plain_frames_in_a_flow_are_items() {
    let mut s = Session::new();
    let n = nest(&mut s);
    // Dragging the row by its child area over its own children: it reorders in the column (above a).
    let grab = Point::new(bounds(&s, n.inner).x0 + 3.0, bounds(&s, n.inner).y0 + 3.0);
    let to = Point::new(bounds(&s, n.a).x0 + 3.0, bounds(&s, n.a).y0 + 3.0);
    drag(&mut s, grab, to);
    assert_eq!(kids(&s, n.outer), vec![n.inner, n.a, n.b]);
    assert_eq!(kids(&s, n.inner), vec![n.x, n.y]);
    // A list of plain cards: dropping onto a card inserts next to it rather than into it.
    let list = frame(&mut s, None, 500.0, 0.0, 200.0, 400.0);
    let cards: Vec<NodeId> = (0..3).map(|i| frame(&mut s, Some(list), 0.0, i as f64 * 70.0, 150.0, 60.0)).collect();
    s.execute("layout.add", &json!({"ids": [list], "direction": "vertical"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let card0 = bounds(&s, cards[0]);
    let grab = Point::new(bounds(&s, cards[2]).x0 + 3.0, bounds(&s, cards[2]).y0 + 3.0);
    let to = Point::new(card0.center().x, card0.y0 + 5.0);
    drag_to(&mut s, grab, to);
    assert_eq!(s.tool.overlay.drop_target, None, "reordering in its own list");
    assert!(s.tool.overlay.insert_line.is_some());
    release(&mut s, to);
    assert_eq!(kids(&s, list), vec![cards[2], cards[0], cards[1]]);
    assert!(kids(&s, cards[0]).is_empty());
    // A layer inside a plain card moves within the card; dragged out over the list it joins it.
    let card = cards[1];
    let dot = rect(&mut s, Some(card), 10.0, 10.0, 40.0, 30.0);
    s.ds_mut().unwrap().view.snap_objects = false;
    select(&mut s, dot);
    let start = bounds(&s, dot);
    drag(&mut s, start.center(), start.center() + figcraft_geom::Vec2::new(30.0, 5.0));
    assert_eq!(parent(&s, dot), card);
    assert_eq!(bounds(&s, dot).x0, start.x0 + 30.0);
    let below = bounds(&s, cards[1]);
    let from = bounds(&s, dot).center();
    drag(&mut s, from, Point::new(below.x1 + 10.0, below.y1 + 2.0));
    assert_eq!(parent(&s, dot), list);
}

#[test]
fn slots_skip_children_that_ignore_auto_layout() {
    let mut s = Session::new();
    let row = frame(&mut s, None, 0.0, 0.0, 400.0, 100.0);
    let a = rect(&mut s, Some(row), 0.0, 0.0, 50.0, 50.0);
    let b = rect(&mut s, Some(row), 60.0, 0.0, 50.0, 50.0);
    s.execute("layout.add", &json!({"ids": [row], "direction": "horizontal"})).unwrap();
    set(&mut s, row, json!({"sizingH": "fixed", "width": 400}));
    // A badge pinned at the far right, first in the layer order.
    let badge = rect(&mut s, Some(row), 350.0, 0.0, 20.0, 20.0);
    set(&mut s, badge, json!({"absolute": true}));
    s.execute("node.reorder", &json!({"id": badge, "index": 0})).unwrap();
    let hidden = rect(&mut s, Some(row), 0.0, 0.0, 20.0, 20.0);
    set(&mut s, hidden, json!({"visible": false}));
    s.execute("node.reorder", &json!({"id": hidden, "index": 0})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(kids(&s, row), vec![hidden, badge, a, b]);
    // b goes before a: the line sits at a's left edge, not at the badge or the hidden layer.
    let a_left = bounds(&s, a).x0;
    let to = Point::new(a_left + 3.0, bounds(&s, a).center().y);
    let from = bounds(&s, b).center();
    drag_to(&mut s, from, to);
    let line = s.tool.overlay.insert_line.expect("insertion line");
    // b now sits where a started; the line is mid-gap before it (gap 10).
    assert!((line[0] - (a_left - 5.0)).abs() < 1e-9, "{line:?}");
    release(&mut s, to);
    let order: Vec<NodeId> = kids(&s, row).into_iter().filter(|k| *k == a || *k == b).collect();
    assert_eq!(order, vec![b, a]);
}

#[test]
fn drag_out_of_auto_layout_to_the_canvas_or_a_plain_frame() {
    let mut s = Session::new();
    let n = nest(&mut s);
    set(&mut s, n.a, json!({"sizingH": "fill"}));
    let plain = frame(&mut s, None, 600.0, 0.0, 300.0, 300.0);
    s.execute("select.none", &json!({})).unwrap();
    // Exact drop points (no snapping to the other layers' edges).
    s.ds_mut().unwrap().view.snap_objects = false;
    // Onto the empty canvas: a free layer on the page where it was dropped.
    let start = bounds(&s, n.a);
    let (from, to) = (start.center(), Point::new(400.0, 500.0));
    drag_to(&mut s, from, to);
    assert!(s.tool.overlay.insert_line.is_none() && s.tool.overlay.drop_target.is_none());
    release(&mut s, to);
    let page = s.page().unwrap();
    assert_eq!(parent(&s, n.a), page);
    let b = bounds(&s, n.a);
    assert_eq!((b.x0, b.y0), (start.x0 + to.x - from.x, start.y0 + to.y - from.y));
    // It filled the column; out of auto layout that becomes its fixed width.
    assert_eq!(sizing(&s, n.a), (Sizing::Fixed, Sizing::Fixed));
    assert_eq!(b.width(), start.width());
    // From the nested row into a plain frame: a positioned child at the drop point.
    select(&mut s, n.x);
    let start = bounds(&s, n.x);
    let (from, to) = (start.center(), Point::new(700.0, 150.0));
    drag_to(&mut s, from, to);
    assert_eq!(s.tool.overlay.drop_target, Some(plain));
    release(&mut s, to);
    assert_eq!(parent(&s, n.x), plain);
    let b = bounds(&s, n.x);
    assert_eq!((b.x0, b.y0), (start.x0 + to.x - from.x, start.y0 + to.y - from.y));
    assert_eq!(kids(&s, n.inner), vec![n.y]);
    s.undo().unwrap();
    assert_eq!(kids(&s, n.inner), vec![n.x, n.y], "one undo step");
}

#[test]
fn a_free_layer_dropped_into_auto_layout_takes_the_slot_under_the_pointer() {
    let mut s = Session::new();
    let n = nest(&mut s);
    let free = rect(&mut s, None, 600.0, 600.0, 30.0, 30.0);
    s.execute("select.none", &json!({})).unwrap();
    // Before b, not appended at the end.
    let to = Point::new(bounds(&s, n.b).x0 + 5.0, bounds(&s, n.b).y0 + 5.0);
    let from = bounds(&s, free).center();
    drag_to(&mut s, from, to);
    assert_eq!(s.tool.overlay.drop_target, Some(n.outer));
    assert!(s.tool.overlay.insert_line.is_some());
    release(&mut s, to);
    assert_eq!(kids(&s, n.outer), vec![n.a, free, n.b, n.inner]);
}

#[test]
fn fill_and_hug_stay_coherent() {
    let mut s = Session::new();
    let n = nest(&mut s);
    // The row hugs its width: a child that fills it turns the row fixed at its current width.
    let w = bounds(&s, n.inner).width();
    set(&mut s, n.x, json!({"sizingH": "fill"}));
    assert_eq!(sizing(&s, n.inner).0, Sizing::Fixed);
    assert_eq!(bounds(&s, n.inner).width(), w);
    assert_eq!(bounds(&s, n.x).width(), 50.0, "it fills what its content left");
    // Back to hug: the child that filled it becomes fixed at its size.
    set(&mut s, n.inner, json!({"sizingH": "hug"}));
    assert_eq!(sizing(&s, n.x).0, Sizing::Fixed);
    assert_eq!(bounds(&s, n.x).width(), 50.0);
    // Removing auto layout: fill children become fixed, the frame's hug becomes fixed, and its
    // own fill in the column is kept.
    set(&mut s, n.y, json!({"sizingV": "fill"}));
    set(&mut s, n.inner, json!({"sizingH": "fill"}));
    s.execute("layout.remove", &json!({"ids": [n.inner]})).unwrap();
    assert_eq!(sizing(&s, n.y), (Sizing::Fixed, Sizing::Fixed));
    assert_eq!(sizing(&s, n.inner), (Sizing::Fill, Sizing::Fixed));
    // Fill without an auto layout parent is refused.
    assert!(s.execute("node.set", &json!({"id": n.y, "props": {"sizingH": "fill"}})).is_err());
}

#[test]
fn fill_text_in_a_hugging_card_sets_the_height_in_one_step() {
    let mut s = Session::new();
    let card = frame(&mut s, None, 0.0, 0.0, 160.0, 50.0);
    let t = id(s
        .execute("text.create", &json!({"text": "A fairly long line of text that has to wrap inside the card", "x": 0, "y": 0, "parent": card}))
        .unwrap());
    s.execute("layout.add", &json!({"ids": [card], "direction": "vertical"})).unwrap();
    set(&mut s, card, json!({"padding": 10, "sizingH": "fixed", "width": 160}));
    set(&mut s, t, json!({"sizingH": "fill"}));
    let (tb, cb) = (bounds(&s, t), bounds(&s, card));
    assert_eq!(tb.width(), 140.0);
    // Whatever the fonts, the card hugs the wrapped text exactly.
    assert!((cb.height() - (tb.height() + 20.0)).abs() < 1e-6, "card {cb:?} text {tb:?}");
}

/// "To remove auto layout from a frame and all of its nested objects at once, right-click on the
/// frame and go to More layout options > Remove all auto layout." (Toggle on auto layout in designs)
#[test]
fn remove_all_auto_layout_strips_the_frame_and_every_frame_nested_in_it() {
    let mut s = Session::new();
    let outer = frame(&mut s, None, 0.0, 0.0, 300.0, 100.0);
    let inner = frame(&mut s, Some(outer), 0.0, 0.0, 100.0, 50.0);
    let deep = frame(&mut s, Some(inner), 0.0, 0.0, 40.0, 20.0);
    rect(&mut s, Some(deep), 0.0, 0.0, 10.0, 10.0);
    for (f, dir) in [(outer, "horizontal"), (inner, "vertical"), (deep, "horizontal")] {
        s.execute("node.set", &json!({"id": f, "props": {"layout": dir}})).unwrap();
    }
    let laid_out = |s: &Session| [outer, inner, deep].map(|n| s.doc().unwrap().node(n).unwrap().layout.is_some());
    // Remove auto layout takes it off the frame alone.
    s.execute("layout.remove", &json!({"ids": [outer]})).unwrap();
    assert_eq!(laid_out(&s), [false, true, true]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(laid_out(&s), [true, true, true]);
    let steps = s.ds().unwrap().history.undo.len();
    let r = s.execute("layout.removeAll", &json!({"ids": [outer]})).unwrap();
    assert_eq!(laid_out(&s), [false, false, false], "every nested frame too");
    assert_eq!(r["removed"].as_array().map(Vec::len), Some(3));
    assert_eq!(s.ds().unwrap().history.undo.len(), steps + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(laid_out(&s), [true, true, true]);
}
