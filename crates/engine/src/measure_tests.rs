//! Measuring: Alt-hover distances (layers, parents, guides, nested, rotated, several selected),
//! Alt-drag neighbours, Shift+Alt auto layout padding and the `measure.*` commands.

use serde_json::{Value, json};

use crate::Session;
use crate::distance::{between, label, to_guide};
use crate::params::id_of;
use crate::snap::Guide;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeId;
use figcraft_geom::Rect;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id_of(&ex(s, "shape.rectangle", p)["id"]).unwrap()
}

fn frame(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&ex(s, "frame.create", json!({"x": x, "y": y, "width": w, "height": h}))["id"]).unwrap()
}

fn hover(s: &mut Session, x: f64, y: f64, alt: bool, shift: bool, cmd: bool) -> Vec<Guide> {
    let mut ev = PointerEvent::at(PointerKind::Hover, x, y, 1.0);
    (ev.alt, ev.shift, ev.cmd) = (alt, shift, cmd);
    s.pointer(&ev).unwrap();
    s.tool.overlay.guides.clone()
}

/// The labelled lines as (label, x0, y0, x1, y1), rounded.
fn labelled(g: &[Guide]) -> Vec<(String, [i64; 4])> {
    let mut v: Vec<(String, [i64; 4])> =
        g.iter().filter(|g| !g.dashed).map(|g| (g.label.clone().unwrap_or_default(), [g.x0, g.y0, g.x1, g.y1].map(|v| v.round() as i64))).collect();
    v.sort();
    v
}

fn labels(g: &[Guide]) -> Vec<String> {
    labelled(g).into_iter().map(|l| l.0).collect()
}

#[test]
fn labels_round_to_two_decimals() {
    assert_eq!(label(50.0), "50");
    assert_eq!(label(12.345), "12.35");
    assert_eq!(label(-3.5), "3.5");
}

#[test]
fn gaps_between_boxes_side_by_side_and_diagonal() {
    let a = Rect::new(0.0, 0.0, 50.0, 50.0);
    // Side by side: one line across the shared rows.
    let g = between(a, Rect::new(100.0, 10.0, 150.0, 70.0));
    assert_eq!(labelled(&g), vec![("50".into(), [50, 30, 100, 30])]);
    assert!(g.iter().all(|g| !g.dashed));
    // Diagonal: both gaps from the middle of the selection, each with a dashed extension to the box.
    let g = between(a, Rect::new(100.0, 120.0, 150.0, 150.0));
    assert_eq!(labelled(&g), vec![("50".into(), [50, 25, 100, 25]), ("70".into(), [25, 50, 25, 120])]);
    let dashed: Vec<[i64; 4]> = g.iter().filter(|g| g.dashed).map(|g| [g.x0, g.y0, g.x1, g.y1].map(|v| v.round() as i64)).collect();
    assert_eq!(dashed, vec![[100, 25, 100, 120], [25, 120, 100, 120]]);
    assert!(g.iter().filter(|g| g.dashed).all(|g| g.label.is_none()));
    // Touching boxes have no gap to draw.
    assert!(between(a, Rect::new(50.0, 0.0, 80.0, 50.0)).is_empty());
}

#[test]
fn containment_measures_to_each_edge_both_ways() {
    let inner = Rect::new(10.0, 20.0, 60.0, 50.0);
    let outer = Rect::new(0.0, 0.0, 100.0, 100.0);
    let want = vec![
        ("10".to_string(), [0, 35, 10, 35]),
        ("20".to_string(), [35, 0, 35, 20]),
        ("40".to_string(), [60, 35, 100, 35]),
        ("50".to_string(), [35, 50, 35, 100]),
    ];
    assert_eq!(labelled(&between(inner, outer)), want);
    // Hovering a layer inside the selection measures from it to the selection's edges.
    assert_eq!(labelled(&between(outer, inner)), want);
}

#[test]
fn overlapping_boxes_measure_between_matching_edges() {
    let g = between(Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(50.0, 30.0, 200.0, 80.0));
    assert_eq!(labels(&g), vec!["100", "20", "30", "50"]);
}

#[test]
fn guides_measure_to_the_near_side_or_both() {
    let a = Rect::new(0.0, 0.0, 50.0, 40.0);
    assert_eq!(labelled(&to_guide(a, true, 300.0)), vec![("250".into(), [50, 20, 300, 20])]);
    assert_eq!(labelled(&to_guide(a, false, -10.0)), vec![("10".into(), [25, -10, 25, 0])]);
    assert_eq!(labels(&to_guide(a, true, 20.0)), vec!["20", "30"]);
    assert!(to_guide(a, true, f64::NAN).is_empty());
}

#[test]
fn alt_hover_measures_the_selection_to_the_hovered_layer() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0, None);
    rect(&mut s, 100.0, 0.0, 50.0, 50.0, None);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    assert!(hover(&mut s, 125.0, 25.0, false, false, false).is_empty(), "no Alt, no lines");
    assert_eq!(s.tool.overlay.measure_box, None);
    assert_eq!(labelled(&hover(&mut s, 125.0, 25.0, true, false, false)), vec![("50".into(), [50, 25, 100, 25])]);
    assert_eq!(s.tool.overlay.measure_box, Some([100.0, 0.0, 150.0, 50.0]), "the measured layer is outlined");
    // Releasing Alt clears them.
    hover(&mut s, 125.0, 25.0, false, false, false);
    assert_eq!(s.tool.overlay.measure_box, None);
}

#[test]
fn alt_over_the_selection_or_empty_space_measures_to_the_parent() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 100.0);
    let r = rect(&mut s, 20.0, 30.0, 40.0, 20.0, Some(f));
    ex(&mut s, "select.set", json!({"ids": [r]}));
    let want = vec![
        ("140".to_string(), [60, 40, 200, 40]),
        ("20".to_string(), [0, 40, 20, 40]),
        ("30".to_string(), [40, 0, 40, 30]),
        ("50".to_string(), [40, 50, 40, 100]),
    ];
    assert_eq!(labelled(&hover(&mut s, 30.0, 40.0, true, false, false)), want, "over the selection itself");
    assert_eq!(s.tool.overlay.measure_box, Some([0.0, 0.0, 200.0, 100.0]));
    assert_eq!(labelled(&hover(&mut s, 500.0, 500.0, true, false, false)), want, "over empty canvas");
    // A top-level selection has no parent to measure to.
    ex(&mut s, "select.set", json!({"ids": [f]}));
    assert!(hover(&mut s, 500.0, 500.0, true, false, false).is_empty());
}

#[test]
fn cmd_alt_measures_to_nested_layers() {
    let mut s = Session::new();
    let a = rect(&mut s, -100.0, 0.0, 50.0, 50.0, None);
    let f = frame(&mut s, 0.0, 0.0, 300.0, 200.0);
    let g1 = rect(&mut s, 20.0, 20.0, 40.0, 40.0, Some(f));
    let g2 = rect(&mut s, 80.0, 20.0, 40.0, 40.0, Some(f));
    ex(&mut s, "object.group", json!({"ids": [g1, g2]}));
    ex(&mut s, "select.set", json!({"ids": [a]}));
    // Alt alone measures to what a click would select (the group in the frame)...
    hover(&mut s, 100.0, 40.0, true, false, false);
    assert_eq!(s.tool.overlay.measure_box, Some([20.0, 20.0, 120.0, 60.0]));
    // ...Cmd+Alt to the layer under the pointer.
    let g = hover(&mut s, 100.0, 40.0, true, false, true);
    assert_eq!(s.tool.overlay.measure_box, Some([80.0, 20.0, 120.0, 60.0]));
    assert_eq!(labelled(&g), vec![("130".into(), [-50, 35, 80, 35])]);
}

#[test]
fn rotated_and_multiple_selections_measure_by_their_bounds() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 40.0, None);
    let b = rect(&mut s, 0.0, 100.0, 40.0, 40.0, None);
    rect(&mut s, 200.0, 0.0, 40.0, 140.0, None);
    ex(&mut s, "select.set", json!({"ids": [a, b]}));
    // The selection's union (0..40 × 0..140) to the tall layer: one gap across all rows.
    assert_eq!(labelled(&hover(&mut s, 220.0, 70.0, true, false, false)), vec![("160".into(), [40, 70, 200, 70])]);
    // A rotated layer measures by its bounding box.
    let r = rect(&mut s, 400.0, 0.0, 100.0, 100.0, None);
    ex(&mut s, "node.set", json!({"id": r, "props": {"rotation": 45}}));
    ex(&mut s, "select.set", json!({"ids": [a, b]}));
    let rb = s.doc().unwrap().world_bounds(r).unwrap();
    let g = hover(&mut s, rb.center().x, rb.center().y, true, false, false);
    assert_eq!(s.tool.overlay.measure_box, Some([rb.x0, rb.y0, rb.x1, rb.y1]));
    let gap = g.iter().find(|g| g.label.is_some()).unwrap();
    assert!((gap.x0 - 40.0).abs() < 1e-9 && (gap.x1 - rb.x0).abs() < 1e-9, "{gap:?}");
}

#[test]
fn alt_near_a_ruler_guide_measures_to_it() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 40.0, None);
    ex(&mut s, "guide.add", json!({"vertical": true, "offset": 300}));
    ex(&mut s, "select.set", json!({"ids": [a]}));
    assert_eq!(labelled(&hover(&mut s, 301.0, 500.0, true, false, false)), vec![("250".into(), [50, 20, 300, 20])]);
    assert_eq!(s.tool.overlay.measure_box, None);
}

#[test]
fn alt_drag_shows_live_distances_to_the_neighbours() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 400.0, 100.0);
    let a = rect(&mut s, 10.0, 30.0, 50.0, 40.0, Some(f));
    rect(&mut s, 200.0, 30.0, 50.0, 40.0, Some(f));
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let ev = |kind, x: f64| {
        let mut e = PointerEvent::at(kind, x, 50.0, 1.0);
        e.alt = true;
        e
    };
    s.pointer(&ev(PointerKind::Down, 30.0)).unwrap();
    // Alt duplicates: the copy, dragged to 110..160, snaps to equal spacing between the original
    // and the other layer (105..155).
    s.pointer(&ev(PointerKind::Move, 130.0)).unwrap();
    let g = s.tool.overlay.guides.clone();
    assert_eq!(
        labelled(&g),
        vec![
            ("30".into(), [130, 0, 130, 30]),
            ("30".into(), [130, 70, 130, 100]),
            ("45".into(), [60, 50, 105, 50]),
            ("45".into(), [155, 50, 200, 50])
        ]
    );
    s.pointer(&ev(PointerKind::Up, 130.0)).unwrap();
    assert!(s.tool.overlay.guides.is_empty());
}

#[test]
fn shift_alt_reads_out_auto_layout_padding_and_gaps() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 300.0, 100.0);
    let c = rect(&mut s, 0.0, 0.0, 20.0, 20.0, Some(f));
    rect(&mut s, 30.0, 0.0, 20.0, 20.0, Some(f));
    ex(&mut s, "node.set", json!({"id": f, "props": {"layout": "horizontal", "padding": [12, 16, 12, 16], "gap": 8}}));
    let other = rect(&mut s, 500.0, 0.0, 10.0, 10.0, None);
    ex(&mut s, "select.set", json!({"ids": [other]}));
    let fb = s.doc().unwrap().world_bounds(f).unwrap();
    let cb = s.doc().unwrap().world_bounds(c).unwrap();
    let g = hover(&mut s, cb.center().x, cb.center().y, true, true, false);
    assert_eq!(labels(&g), vec!["12", "12", "16", "16", "8"]);
    assert_eq!(s.tool.overlay.measure_box, Some([fb.x0, fb.y0, fb.x1, fb.y1]));
    // Works without a selection too, and the command says the same.
    ex(&mut s, "select.none", json!({}));
    assert_eq!(labels(&hover(&mut s, cb.center().x, cb.center().y, true, true, false)), vec!["12", "12", "16", "16", "8"]);
    let out = ex(&mut s, "measure.padding", json!({"id": c}));
    assert_eq!(id_of(&out["frame"]), Some(f));
    assert_eq!(out["lines"].as_array().map(Vec::len), Some(5));
    // Not in auto layout: Shift+Alt measures as usual.
    assert!(s.execute("measure.padding", &json!({"id": other})).is_err());
}

#[test]
fn measure_distances_command() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 100.0);
    let r = rect(&mut s, 20.0, 30.0, 40.0, 20.0, Some(f));
    let other = rect(&mut s, 300.0, 30.0, 10.0, 10.0, None);
    let out = ex(&mut s, "measure.distances", json!({"ids": [r]}));
    assert_eq!(out["lines"].as_array().map(Vec::len), Some(4), "to the parent by default");
    let out = ex(&mut s, "measure.distances", json!({"ids": [r], "to": other}));
    assert_eq!(out["lines"][0]["label"], json!("240"));
    assert!(s.execute("measure.distances", &json!({"ids": [f]})).is_err(), "top level: pass `to`");
    assert!(s.execute("measure.distances", &json!({"ids": [r], "to": 99999})).is_err());
}

#[test]
fn leaving_the_canvas_clears_the_hover_measurement() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0, None);
    rect(&mut s, 100.0, 0.0, 50.0, 50.0, None);
    ex(&mut s, "select.set", json!({"ids": [a]}));
    assert!(!hover(&mut s, 125.0, 25.0, true, false, false).is_empty());
    assert!(s.tool.overlay.measure_box.is_some());
    s.pointer_left();
    assert!(s.tool.overlay.guides.is_empty(), "{:?}", s.tool.overlay.guides);
    assert_eq!(s.tool.overlay.measure_box, None);
    // Dev Mode measures without Alt; leaving clears those too.
    s.tool.dev_mode = true;
    assert!(!hover(&mut s, 125.0, 25.0, false, false, false).is_empty());
    s.pointer_left();
    assert!(s.tool.overlay.guides.is_empty());
}
