//! Suggest auto layout (#1823): a frame's layers are split into rows or columns by the gaps between
//! them, each band of several layers gets a frame of its own, and the design keeps its placement.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{LayoutMode, NodeId, NodeKind, Sizing};

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn frame(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id(&ex(s, "frame.create", json!({"x": x, "y": y, "width": w, "height": h})))
}

fn rect(s: &mut Session, parent: NodeId, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id(&ex(s, "shape.rectangle", json!({"parent": parent, "x": x, "y": y, "width": w, "height": h})))
}

/// A layer's box on the page: `[x0, y0, x1, y1]`, rounded.
fn page_box(s: &Session, n: NodeId) -> [f64; 4] {
    let b = s.doc().unwrap().world_bounds(n).unwrap();
    [b.x0, b.y0, b.x1, b.y1].map(|v| (v * 1000.0).round() / 1000.0)
}

fn boxes(s: &Session, ns: &[NodeId]) -> Vec<[f64; 4]> {
    ns.iter().map(|n| page_box(s, *n)).collect()
}

fn kids(s: &Session, n: NodeId) -> Vec<NodeId> {
    s.doc().unwrap().node(n).unwrap().children.clone()
}

fn mode(s: &Session, n: NodeId) -> Option<LayoutMode> {
    s.doc().unwrap().node(n).unwrap().layout.as_ref().map(|l| l.mode)
}

/// A frame made by Suggest auto layout: no fill, no clipping, hugging what is in it.
fn assert_band(s: &Session, n: NodeId, m: LayoutMode) {
    let node = s.doc().unwrap().node(n).unwrap();
    assert!(matches!(node.kind, NodeKind::Frame), "a band is a frame");
    assert!(node.fills.is_empty() && !node.clip, "a band has no fill and does not clip");
    assert_eq!(mode(s, n), Some(m));
    assert_eq!((node.child_layout.sizing_h, node.child_layout.sizing_v), (Sizing::Hug, Sizing::Hug));
}

fn warnings(r: &Value) -> Vec<String> {
    r["warnings"].as_array().map(|a| a.iter().filter_map(|w| w.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// The issue's design: four 50 x 50 squares in a 200 x 200 frame, two rows of two.
fn squares() -> (Session, NodeId, [NodeId; 4]) {
    let mut s = Session::new();
    let f = frame(&mut s, 100.0, 50.0, 200.0, 200.0);
    let sq = [(0.0, 0.0), (60.0, 0.0), (0.0, 60.0), (60.0, 60.0)].map(|(x, y)| rect(&mut s, f, x, y, 50.0, 50.0));
    (s, f, sq)
}

#[test]
fn two_rows_of_two_squares_become_a_vertical_frame_of_two_horizontal_rows() {
    let (mut s, f, sq) = squares();
    let before_frame = page_box(&s, f);
    let before = boxes(&s, &sq);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert_eq!(result["ids"], json!([f]));
    assert!(warnings(&result).is_empty(), "{result}");
    assert_eq!(mode(&s, f), Some(LayoutMode::Vertical));
    let rows = kids(&s, f);
    assert_eq!(rows.len(), 2, "two rows");
    for (row, pair) in rows.iter().zip([[sq[0], sq[1]], [sq[2], sq[3]]]) {
        assert_band(&s, *row, LayoutMode::Horizontal);
        assert_eq!(kids(&s, *row), pair);
    }
    let node = s.doc().unwrap().node(f).unwrap();
    assert_eq!(node.child_layout.sizing_h, Sizing::Fixed, "the frame keeps its size");
    assert_eq!(node.child_layout.sizing_v, Sizing::Fixed);
    assert_eq!(page_box(&s, f), before_frame);
    assert_eq!(boxes(&s, &sq), before, "every square stays where it was");
    assert_eq!(s.selection(), vec![f]);
}

#[test]
fn suggesting_auto_layout_is_one_undo_step() {
    let (mut s, f, sq) = squares();
    let rows_before = kids(&s, f);
    let steps = s.ds().unwrap().history.undo.len();
    ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert_eq!(s.ds().unwrap().history.undo.len(), steps + 1);
    let rows = kids(&s, f);
    s.undo().unwrap();
    assert_eq!(kids(&s, f), rows_before);
    assert_eq!(mode(&s, f), None);
    assert!(rows.iter().all(|r| s.doc().unwrap().node(*r).is_none()), "the band frames are gone");
    assert_eq!(
        boxes(&s, &sq),
        [[100.0, 50.0, 150.0, 100.0], [160.0, 50.0, 210.0, 100.0], [100.0, 110.0, 150.0, 160.0], [160.0, 110.0, 210.0, 160.0]]
    );
}

#[test]
fn three_squares_in_a_row_get_one_horizontal_layout_without_nesting() {
    let mut s = Session::new();
    let f = frame(&mut s, 20.0, 30.0, 200.0, 100.0);
    let sq = [0.0, 70.0, 140.0].map(|x| rect(&mut s, f, x, 25.0, 50.0, 50.0));
    let before = boxes(&s, &sq);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert!(warnings(&result).is_empty(), "{result}");
    assert_eq!(mode(&s, f), Some(LayoutMode::Horizontal));
    assert_eq!(kids(&s, f), sq, "the squares are the frame's own layers");
    assert_eq!(boxes(&s, &sq), before);
}

#[test]
fn three_squares_in_a_column_get_one_vertical_layout_without_nesting() {
    let mut s = Session::new();
    let f = frame(&mut s, 20.0, 30.0, 100.0, 200.0);
    let sq = [0.0, 70.0, 140.0].map(|y| rect(&mut s, f, 25.0, y, 50.0, 50.0));
    let before = boxes(&s, &sq);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert!(warnings(&result).is_empty(), "{result}");
    assert_eq!(mode(&s, f), Some(LayoutMode::Vertical));
    assert_eq!(kids(&s, f), sq);
    assert_eq!(boxes(&s, &sq), before);
}

#[test]
fn a_header_above_two_columns_of_squares_gives_a_row_per_line() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 200.0);
    let header = rect(&mut s, f, 0.0, 0.0, 200.0, 50.0);
    let sq = [(0.0, 60.0), (0.0, 120.0), (110.0, 60.0), (110.0, 120.0)].map(|(x, y)| rect(&mut s, f, x, y, 50.0, 50.0));
    let all = [vec![header], sq.to_vec()].concat();
    let before = boxes(&s, &all);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert!(warnings(&result).is_empty(), "{result}");
    // The squares line up in two rows, so the gaps between rows win over the gap between columns.
    assert_eq!(mode(&s, f), Some(LayoutMode::Vertical));
    let top = kids(&s, f);
    assert_eq!(top.len(), 3);
    assert_eq!(top.first(), Some(&header));
    assert_band(&s, top[1], LayoutMode::Horizontal);
    assert_eq!(kids(&s, top[1]), vec![sq[0], sq[2]]);
    assert_band(&s, top[2], LayoutMode::Horizontal);
    assert_eq!(kids(&s, top[2]), vec![sq[1], sq[3]]);
    assert_eq!(boxes(&s, &all), before);
}

#[test]
fn a_header_above_a_tall_layer_beside_a_column_nests_three_levels() {
    let mut s = Session::new();
    let f = frame(&mut s, 40.0, 10.0, 200.0, 200.0);
    let header = rect(&mut s, f, 0.0, 0.0, 200.0, 40.0);
    let tall = rect(&mut s, f, 0.0, 60.0, 50.0, 110.0);
    let sq = [60.0, 120.0].map(|y| rect(&mut s, f, 110.0, y, 50.0, 50.0));
    let all = [header, tall, sq[0], sq[1]];
    let before = boxes(&s, &all);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert!(warnings(&result).is_empty(), "{result}");
    assert_eq!(mode(&s, f), Some(LayoutMode::Vertical));
    let top = kids(&s, f);
    assert_eq!(top.len(), 2);
    assert_eq!(top.first(), Some(&header));
    let band = top[1];
    assert_band(&s, band, LayoutMode::Horizontal);
    let cols = kids(&s, band);
    assert_eq!(cols.len(), 2);
    assert_eq!(cols.first(), Some(&tall));
    assert_band(&s, cols[1], LayoutMode::Vertical);
    assert_eq!(kids(&s, cols[1]), sq);
    assert_eq!(boxes(&s, &all), before);
}

#[test]
fn unequal_gaps_use_the_smallest_and_say_so() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 300.0, 100.0);
    let sq = [0.0, 60.0, 130.0].map(|x| rect(&mut s, f, x, 10.0, 50.0, 50.0));
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert_eq!(mode(&s, f), Some(LayoutMode::Horizontal));
    let w = warnings(&result);
    assert_eq!(w.len(), 1, "{result}");
    assert!(w[0].contains("gaps"), "{}", w[0]);
    let b = boxes(&s, &sq);
    assert_eq!(b[0], [0.0, 10.0, 50.0, 60.0]);
    assert_eq!(b[1], [60.0, 10.0, 110.0, 60.0]);
    assert_eq!(b[2], [120.0, 10.0, 170.0, 60.0], "the last square moves along the row only");
}

#[test]
fn overlapping_layers_leave_the_frame_without_auto_layout_and_say_so() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 200.0);
    let a = rect(&mut s, f, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, f, 50.0, 50.0, 100.0, 100.0);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert_eq!(mode(&s, f), None);
    assert_eq!(kids(&s, f), vec![a, b]);
    assert_eq!(warnings(&result).len(), 1, "{result}");
    assert_eq!(boxes(&s, &[a, b]), vec![[0.0, 0.0, 100.0, 100.0], [50.0, 50.0, 150.0, 150.0]]);
}

#[test]
fn a_group_becomes_a_frame_in_place_and_follows_the_rules() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let (a, b, c, d) = (
        rect(&mut s, page, 10.0, 20.0, 40.0, 40.0),
        rect(&mut s, page, 70.0, 20.0, 40.0, 40.0),
        rect(&mut s, page, 10.0, 80.0, 40.0, 40.0),
        rect(&mut s, page, 70.0, 80.0, 40.0, 40.0),
    );
    let group = id(&ex(&mut s, "object.group", json!({"ids": [a, b, c, d]})));
    let before = boxes(&s, &[group, a, b, c, d]);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [group]}));
    assert_eq!(result["ids"], json!([group]), "the group keeps its identity");
    assert!(matches!(s.doc().unwrap().node(group).unwrap().kind, NodeKind::Frame));
    assert_eq!(mode(&s, group), Some(LayoutMode::Vertical));
    let rows = kids(&s, group);
    assert_eq!(rows.len(), 2);
    assert_band(&s, rows[0], LayoutMode::Horizontal);
    assert_eq!(kids(&s, rows[0]), vec![a, b]);
    assert_eq!(kids(&s, rows[1]), vec![c, d]);
    assert_eq!(boxes(&s, &[group, a, b, c, d]), before);
}

#[test]
fn several_layers_are_wrapped_in_a_frame_that_is_then_split() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let sq = [(0.0, 0.0), (80.0, 0.0), (0.0, 80.0), (80.0, 80.0)].map(|(x, y)| rect(&mut s, page, x, y, 50.0, 50.0));
    let before = boxes(&s, &sq);
    let result = ex(&mut s, "layout.suggest", json!({"ids": sq}));
    let wrapper = id_of(&result["ids"][0]).unwrap();
    assert!(s.doc().unwrap().node(wrapper).unwrap().fills.is_empty());
    assert_eq!(mode(&s, wrapper), Some(LayoutMode::Vertical));
    assert_eq!(kids(&s, wrapper).len(), 2);
    assert_eq!(boxes(&s, &sq), before);
}

#[test]
fn a_direction_forces_the_top_level_and_nested_frames_follow_the_gaps() {
    let (mut s, f, sq) = squares();
    let before = boxes(&s, &sq);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f], "direction": "horizontal"}));
    assert!(warnings(&result).is_empty(), "{result}");
    assert_eq!(mode(&s, f), Some(LayoutMode::Horizontal));
    let cols = kids(&s, f);
    assert_eq!(cols.len(), 2, "two columns");
    assert_band(&s, cols[0], LayoutMode::Vertical);
    assert_eq!(kids(&s, cols[0]), vec![sq[0], sq[2]]);
    assert_band(&s, cols[1], LayoutMode::Vertical);
    assert_eq!(kids(&s, cols[1]), vec![sq[1], sq[3]]);
    assert_eq!(boxes(&s, &sq), before);
}

#[test]
fn a_frame_with_one_layer_is_laid_out_like_add_auto_layout() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 100.0);
    let a = rect(&mut s, f, 20.0, 20.0, 60.0, 40.0);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert!(warnings(&result).is_empty(), "{result}");
    assert_eq!(mode(&s, f), Some(LayoutMode::Horizontal));
    assert_eq!(kids(&s, f), vec![a]);
}

#[test]
fn add_auto_layout_still_makes_a_single_row() {
    let (mut s, f, sq) = squares();
    ex(&mut s, "layout.add", json!({"ids": [f]}));
    let mut got = kids(&s, f);
    got.sort();
    assert_eq!(got, sq.to_vec(), "no nested frames");
}

#[test]
fn a_centred_title_above_a_card_is_lined_up_at_the_card_edge_with_a_warning() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 200.0);
    let title = rect(&mut s, f, 60.0, 0.0, 80.0, 30.0);
    let card = rect(&mut s, f, 0.0, 40.0, 200.0, 100.0);
    let result = ex(&mut s, "layout.suggest", json!({"ids": [f]}));
    assert_eq!(mode(&s, f), Some(LayoutMode::Vertical));
    assert_eq!(kids(&s, f), vec![title, card]);
    let w = warnings(&result);
    assert_eq!(w.len(), 1, "{result}");
    assert!(w[0].contains("different offsets"), "{}", w[0]);
    assert_eq!(page_box(&s, title), [0.0, 0.0, 80.0, 30.0], "the title's left edge is the card's now");
    assert_eq!(page_box(&s, card), [0.0, 40.0, 200.0, 140.0], "the rest stays");
}

#[test]
fn instances_and_layers_inside_them_are_refused_and_a_main_component_works() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 200.0);
    rect(&mut s, f, 0.0, 0.0, 50.0, 50.0);
    rect(&mut s, f, 0.0, 60.0, 50.0, 30.0);
    let nested = id(&ex(&mut s, "frame.create", json!({"parent": f, "x": 0, "y": 100, "width": 100, "height": 50})));
    rect(&mut s, nested, 0.0, 0.0, 20.0, 20.0);
    rect(&mut s, nested, 40.0, 0.0, 20.0, 20.0);
    let comp = id(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let inst = id(&ex(&mut s, "instance.create", json!({"component": comp, "x": 400, "y": 0})));
    let inside: Vec<NodeId> = kids(&s, inst).into_iter().filter(|k| matches!(s.doc().unwrap().node(*k).unwrap().kind, NodeKind::Frame)).collect();
    assert!(!inside.is_empty(), "the instance has a frame of its own");
    let page = s.page().unwrap();
    let snapshot = |s: &Session| (kids(s, page), kids(s, inst), kids(s, inside[0]));
    let before = snapshot(&s);
    let steps = s.ds().unwrap().history.undo.len();
    for target in [inst, inside[0]] {
        let e = s.execute("layout.suggest", &json!({"ids": [target]})).unwrap_err().to_string();
        assert!(e.contains("instance"), "{e}");
    }
    assert_eq!(snapshot(&s), before, "nothing changed");
    assert_eq!(s.ds().unwrap().history.undo.len(), steps, "no history step");
    assert!(s.doc().unwrap().node(inst).unwrap().layout.is_none());
    // The main component is a frame and works.
    let result = ex(&mut s, "layout.suggest", json!({"ids": [comp]}));
    assert_eq!(result["ids"], json!([comp]));
}
