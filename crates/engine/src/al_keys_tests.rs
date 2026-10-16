//! Arrow keys on auto layout children: along the flow, wrapped rows, grid cells and nested frames.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, parent: NodeId, w: f64, h: f64) -> NodeId {
    id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": w, "height": h, "parent": parent})).unwrap())
}

/// A fixed-size auto layout frame (`layout`: horizontal|vertical|grid), no padding, gap 10.
fn al(s: &mut Session, parent: Option<NodeId>, layout: &str, w: f64, h: f64) -> NodeId {
    let mut p = json!({"x": 0, "y": 0, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    let f = id(s.execute("frame.create", &p).unwrap());
    set(s, f, json!({"layout": layout, "gap": 10, "crossGap": 10, "padding": 0}));
    f
}

fn set(s: &mut Session, node: NodeId, props: Value) {
    s.execute("node.set", &json!({"id": node, "props": props})).unwrap();
}

fn kids(s: &Session, node: NodeId) -> Vec<NodeId> {
    s.doc().unwrap().node(node).unwrap().children.clone()
}

fn arrow(s: &mut Session, ids: &[NodeId], dx: i32, dy: i32) {
    s.execute("transform.nudge", &json!({"ids": ids, "dx": dx, "dy": dy})).unwrap();
}

fn parent(s: &Session, node: NodeId) -> NodeId {
    s.doc().unwrap().node(node).unwrap().parent.unwrap()
}

#[test]
fn along_the_flow_skips_hidden_and_absolute_siblings() {
    let mut s = Session::new();
    let row = al(&mut s, None, "horizontal", 400.0, 40.0);
    let [a, hidden, b, free] = [0; 4].map(|_| rect(&mut s, row, 40.0, 20.0));
    set(&mut s, hidden, json!({"visible": false}));
    set(&mut s, free, json!({"absolute": true}));
    // Right swaps with the next placed sibling; the hidden and absolute layers keep their slots.
    arrow(&mut s, &[a], 1, 0);
    assert_eq!(kids(&s, row), vec![b, hidden, a, free]);
    // At the end of the flow there's nowhere to go.
    arrow(&mut s, &[a], 1, 0);
    assert_eq!(kids(&s, row), vec![b, hidden, a, free]);
    // Across the axis with no auto layout around: nothing either.
    arrow(&mut s, &[a], 0, 1);
    assert_eq!(kids(&s, row), vec![b, hidden, a, free]);
    // One undo step per arrow.
    s.undo().unwrap();
    assert_eq!(kids(&s, row), vec![a, hidden, b, free]);
    // The absolute child nudges.
    let x = s.doc().unwrap().world_bounds(free).unwrap().x0;
    arrow(&mut s, &[free], 1, 0);
    assert_eq!(s.doc().unwrap().world_bounds(free).unwrap().x0, x + 1.0);
    assert_eq!(kids(&s, row), vec![a, hidden, b, free]);
    // A column moves on Up/Down.
    let col = al(&mut s, None, "vertical", 100.0, 300.0);
    let [c0, c1] = [0; 2].map(|_| rect(&mut s, col, 40.0, 20.0));
    arrow(&mut s, &[c1], 0, -1);
    assert_eq!(kids(&s, col), vec![c1, c0]);
    arrow(&mut s, &[c1], 1, 0);
    assert_eq!(kids(&s, col), vec![c1, c0], "Right does nothing in a lone column");
}

#[test]
fn single_item_wrapped_lines_move_and_drop_by_line() {
    let mut s = Session::new();
    // Full-width items: one per row.
    let row = al(&mut s, None, "horizontal", 160.0, 200.0);
    set(&mut s, row, json!({"wrap": true}));
    let k: Vec<NodeId> = (0..2).map(|_| rect(&mut s, row, 160.0, 20.0)).collect();
    assert_eq!(s.doc().unwrap().world_bounds(k[1]).unwrap().y0, 30.0, "second row");
    let slot = crate::drop::auto_layout_slot(s.doc().unwrap(), row, figcraft_geom::Point::new(10.0, 35.0), &[]);
    assert_eq!(slot, Some((1, Some([-2.0, 30.0, -2.0, 50.0]))));
    arrow(&mut s, &[k[1]], 0, -1);
    assert_eq!(kids(&s, row), vec![k[1], k[0]]);

    // Full-height items in a vertical wrap: one per column.
    let col = al(&mut s, None, "vertical", 200.0, 160.0);
    set(&mut s, col, json!({"wrap": true}));
    let c: Vec<NodeId> = (0..3).map(|_| rect(&mut s, col, 20.0, 160.0)).collect();
    arrow(&mut s, &[c[2]], -1, 0);
    assert_eq!(kids(&s, col), vec![c[0], c[2], c[1]]);
}

#[test]
fn wrapped_rows_move_up_and_down_a_row() {
    let mut s = Session::new();
    // 40 px items, gap 10, 160 px wide: three per row.
    let row = al(&mut s, None, "horizontal", 160.0, 200.0);
    set(&mut s, row, json!({"wrap": true}));
    let k: Vec<NodeId> = (0..6).map(|_| rect(&mut s, row, 40.0, 20.0)).collect();
    let y = |s: &Session, n: NodeId| s.doc().unwrap().world_bounds(n).unwrap().y0;
    let x = |s: &Session, n: NodeId| s.doc().unwrap().world_bounds(n).unwrap().x0;
    assert_eq!(y(&s, k[3]), 30.0, "second row");
    // Down: the second item lands in the second row, under where it was.
    arrow(&mut s, &[k[1]], 0, 1);
    assert_eq!(kids(&s, row), vec![k[0], k[2], k[3], k[4], k[1], k[5]]);
    assert_eq!((x(&s, k[1]), y(&s, k[1])), (50.0, 30.0));
    // Down again: no third row.
    arrow(&mut s, &[k[1]], 0, 1);
    assert_eq!(kids(&s, row), vec![k[0], k[2], k[3], k[4], k[1], k[5]]);
    // Up: back to the first row over where it is.
    arrow(&mut s, &[k[1]], 0, -1);
    assert_eq!((x(&s, k[1]), y(&s, k[1])), (50.0, 0.0));
    // Left/Right still step along the flow, across the row break.
    let before = kids(&s, row);
    let third = before[2];
    arrow(&mut s, &[third], 1, 0);
    assert_eq!(y(&s, third), 30.0, "the last of a row steps into the next one");
}

#[test]
fn grid_arrows_move_between_cells() {
    let mut s = Session::new();
    let g = al(&mut s, None, "grid", 200.0, 200.0);
    set(&mut s, g, json!({"gridColumns": 2, "gridRows": 2}));
    let [a, b, c, d] = [0; 4].map(|_| rect(&mut s, g, 20.0, 20.0));
    // Automatically placed children swap places in the flow.
    arrow(&mut s, &[a], 1, 0);
    assert_eq!(kids(&s, g), vec![b, a, c, d]);
    s.undo().unwrap();
    arrow(&mut s, &[a], 0, 1);
    assert_eq!(kids(&s, g), vec![b, c, a, d], "Down: the cell below (A lands at column 0, row 1)");
    let cell = |s: &Session, n: NodeId| {
        let doc = s.doc().unwrap();
        let (cells, _) = figcraft_layout::grid_place(doc, doc.node(g).unwrap().layout.as_ref().unwrap(), &kids(s, g));
        cells.iter().find(|x| x.id == n).map(|x| (x.column, x.row)).unwrap()
    };
    assert_eq!(cell(&s, a), (0, 1));
    // Past the grid's edge: nothing.
    arrow(&mut s, &[a], -1, 0);
    arrow(&mut s, &[a], 0, 1);
    assert_eq!(cell(&s, a), (0, 1));
    // Into an empty cell: the child is anchored there.
    s.execute("edit.delete", &json!({"ids": [d]})).unwrap();
    arrow(&mut s, &[a], 1, 0);
    assert_eq!(cell(&s, a), (1, 1));
    let cl = s.doc().unwrap().node(a).unwrap().child_layout.clone();
    assert_eq!((cl.grid_column, cl.grid_row), (Some(1), Some(1)));
    // Anchored children swap cells.
    set(&mut s, b, json!({"gridColumn": 0, "gridRow": 0}));
    set(&mut s, c, json!({"gridColumn": 1, "gridRow": 0}));
    arrow(&mut s, &[b], 1, 0);
    assert_eq!((cell(&s, b), cell(&s, c)), ((1, 0), (0, 0)));
}

#[test]
fn arrows_cross_into_neighbouring_nested_frames() {
    let mut s = Session::new();
    // A column of two rows of three items.
    let list = al(&mut s, None, "vertical", 300.0, 200.0);
    let r1 = al(&mut s, Some(list), "horizontal", 200.0, 40.0);
    let r2 = al(&mut s, Some(list), "horizontal", 200.0, 40.0);
    let top: Vec<NodeId> = (0..3).map(|_| rect(&mut s, r1, 40.0, 20.0)).collect();
    let bottom: Vec<NodeId> = (0..3).map(|_| rect(&mut s, r2, 40.0, 20.0)).collect();
    // Down from the middle of the top row: into the bottom row, at the same place.
    arrow(&mut s, &[top[1]], 0, 1);
    assert_eq!(parent(&s, top[1]), r2);
    assert_eq!(kids(&s, r2), vec![bottom[0], top[1], bottom[1], bottom[2]]);
    // One undo step.
    s.undo().unwrap();
    assert_eq!(kids(&s, r1), top);
    assert_eq!(kids(&s, r2), bottom);
    // Up from the top row: no row above.
    arrow(&mut s, &[top[1]], 0, -1);
    assert_eq!(kids(&s, r1), top);
    // Right at the end of a row in a column: the column doesn't flow sideways.
    arrow(&mut s, &[top[2]], 1, 0);
    assert_eq!(kids(&s, r1), top);
    // Up from the bottom row's first item: the top row's start.
    arrow(&mut s, &[bottom[0]], 0, -1);
    assert_eq!(kids(&s, r1), vec![bottom[0], top[0], top[1], top[2]]);

    // A row of two rows: Right at the end of the first enters the second at its start, Left back.
    let outer = al(&mut s, None, "horizontal", 600.0, 60.0);
    let a = al(&mut s, Some(outer), "horizontal", 150.0, 40.0);
    let b = al(&mut s, Some(outer), "horizontal", 150.0, 40.0);
    let ak: Vec<NodeId> = (0..2).map(|_| rect(&mut s, a, 20.0, 20.0)).collect();
    let bk: Vec<NodeId> = (0..2).map(|_| rect(&mut s, b, 20.0, 20.0)).collect();
    arrow(&mut s, &[ak[1]], 1, 0);
    assert_eq!(kids(&s, b), vec![ak[1], bk[0], bk[1]]);
    arrow(&mut s, &[ak[1]], -1, 0);
    assert_eq!(kids(&s, a), vec![ak[0], ak[1]], "back at the end of the first row");
    // A neighbour without auto layout, or an instance, takes nothing.
    set(&mut s, b, json!({"layout": "none"}));
    arrow(&mut s, &[ak[1]], 1, 0);
    assert_eq!(parent(&s, ak[1]), a);
}

#[test]
fn plain_frames_and_instances_keep_their_rules() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap());
    let r = id(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20, "parent": f})).unwrap());
    s.execute("transform.nudge", &json!({"ids": [r], "dx": 1, "dy": -1, "big": true})).unwrap();
    let b = s.doc().unwrap().world_bounds(r).unwrap();
    assert_eq!((b.x0, b.y0), (20.0, 0.0));
    // An instance's auto layout children can't be reordered (only its main's can).
    let main = al(&mut s, None, "horizontal", 200.0, 40.0);
    for _ in 0..2 {
        rect(&mut s, main, 20.0, 20.0);
    }
    let comp = id(s.execute("component.create", &json!({"ids": [main]})).unwrap());
    let inst = id(s.execute("instance.create", &json!({"component": comp, "x": 0, "y": 300})).unwrap());
    let first = kids(&s, inst)[0];
    assert!(s.execute("transform.nudge", &json!({"ids": [first], "dx": 1, "dy": 0})).is_err());
}
