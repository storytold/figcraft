//! Tidy up (`arrange.tidy`) against what was measured in Figma's web app on 2026-11-12: a row, a
//! column and a 2 x 3 grid of unequal sizes, eight runs on the gap, and the cases where the menu
//! row is greyed out. Figma keeps the order and sizes, evens one gap per axis and keeps the first
//! layer where it is. Tests whose message says "not measured in Figma" pin FigCraft's own choice.

use crate::Session;
use figcraft_doc::NodeId;
use serde_json::json;

fn make(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}
fn xy(s: &Session, id: NodeId) -> (f64, f64) {
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    (b.x0, b.y0)
}
fn size(s: &Session, id: NodeId) -> (f64, f64) {
    let n = s.doc().unwrap().node(id).unwrap();
    (n.width, n.height)
}
fn tidy(s: &mut Session, ids: &[NodeId]) {
    s.execute("arrange.tidy", &json!({"ids": ids})).unwrap();
}
fn can(s: &Session, ids: &[NodeId]) -> bool {
    crate::tidy::can_tidy(s.doc().unwrap(), ids)
}
fn all_xy(s: &Session, ids: &[NodeId]) -> Vec<(f64, f64)> {
    ids.iter().map(|i| xy(s, *i)).collect()
}
/// Squares of 50 at Y 0 with the given X values.
fn squares(s: &mut Session, xs: &[f64]) -> Vec<NodeId> {
    xs.iter().map(|x| make(s, *x, 0.0, 50.0, 50.0)).collect()
}
fn xs_of(s: &Session, ids: &[NodeId]) -> Vec<f64> {
    ids.iter().map(|i| xy(s, *i).0).collect()
}

fn row(s: &mut Session, xs: [f64; 4]) -> Vec<NodeId> {
    vec![make(s, xs[0], 0., 50., 30.), make(s, xs[1], 10., 70., 50.), make(s, xs[2], -5., 40., 40.), make(s, xs[3], 8., 60., 20.)]
}
fn grid(s: &mut Session) -> Vec<NodeId> {
    vec![
        make(s, 0., 0., 50., 30.),
        make(s, 80., 5., 70., 50.),
        make(s, 190., -3., 40., 40.),
        make(s, 2., 90., 60., 20.),
        make(s, 85., 95., 40., 50.),
        make(s, 200., 92., 55., 35.),
    ]
}

#[test]
fn a_row_keeps_its_order_sizes_and_y_and_evens_the_gaps() {
    let mut s = Session::new();
    let ids = row(&mut s, [0., 80., 190., 260.]);
    let before = s.undo_depth();
    tidy(&mut s, &ids);
    assert_eq!(all_xy(&s, &ids), vec![(0., 0.), (80., 10.), (180., -5.), (250., 8.)]);
    assert_eq!(size(&s, ids[1]), (70., 50.));
    assert_eq!(size(&s, ids[3]), (60., 20.));
    assert_eq!(s.undo_depth().0, before.0 + 1);
    s.undo().unwrap();
    assert_eq!(xy(&s, ids[3]), (260., 8.));
}

#[test]
fn gaps_that_all_differ_take_their_mean_taken_down() {
    let mut s = Session::new();
    let ids = row(&mut s, [0., 130., 220., 305.]);
    tidy(&mut s, &ids);
    assert_eq!(all_xy(&s, &ids), vec![(0., 0.), (98., 10.), (216., -5.), (304., 8.)]);
    // Gaps 30, 40, 21 (the third layer 49 wide, 50 tall).
    let mut s = Session::new();
    let ids =
        vec![make(&mut s, 0., 0., 50., 30.), make(&mut s, 80., 10., 70., 50.), make(&mut s, 190., -5., 49., 50.), make(&mut s, 260., 8., 60., 20.)];
    tidy(&mut s, &ids);
    assert_eq!(xs_of(&s, &ids), vec![0., 80., 180., 259.]);
}

#[test]
fn a_column_keeps_x_and_evens_the_vertical_gaps() {
    let mut s = Session::new();
    let ids =
        vec![make(&mut s, 0., 0., 50., 30.), make(&mut s, 8., 60., 70., 50.), make(&mut s, -5., 150., 40., 40.), make(&mut s, 6., 220., 60., 20.)];
    tidy(&mut s, &ids);
    assert_eq!(all_xy(&s, &ids), vec![(0., 0.), (8., 60.), (-5., 140.), (6., 210.)]);
}

#[test]
fn the_gap_is_the_most_common_one_else_the_mean() {
    // (X before, widths, X after): cases 1, 2, 3, 4, 5, 7 and 8 of the second run.
    let cases: [(&[f64], &[f64], &[f64]); 7] = [
        (&[0., 60., 160.], &[50., 50., 50.], &[0., 80., 160.]),
        (&[0., 100., 160.], &[50., 50., 50.], &[0., 80., 160.]),
        (&[0., 70., 140.], &[50., 50., 50.], &[0., 70., 140.]),
        (&[0., 50., 160.], &[50., 50., 50.], &[0., 80., 160.]),
        (&[0., 60., 120., 240.], &[50., 50., 50., 50.], &[0., 60., 120., 180.]),
        (&[0., 70., 210.], &[50., 100., 30.], &[0., 80., 210.]),
        (&[0., 60., 120., 220., 360.], &[50., 50., 50., 50., 50.], &[0., 60., 120., 180., 240.]),
    ];
    for (n, (before, widths, after)) in cases.iter().enumerate() {
        let mut s = Session::new();
        let ids: Vec<NodeId> = before.iter().zip(widths.iter()).map(|(x, w)| make(&mut s, *x, 0., *w, 50.)).collect();
        tidy(&mut s, &ids);
        assert_eq!(xs_of(&s, &ids), after.to_vec(), "case {n}");
        assert!(xy(&s, ids[0]).1 == 0.0 && xy(&s, ids[1]).1 == 0.0, "case {n}: Y kept");
    }
}

#[test]
fn a_grid_is_tidied_by_columns() {
    let mut s = Session::new();
    let ids = grid(&mut s);
    tidy(&mut s, &ids);
    assert_eq!(all_xy(&s, &ids), vec![(6., -3.), (93., -3.), (203., -3.), (1., 78.), (108., 98.), (195., 88.)]);
    assert_eq!(size(&s, ids[4]), (40., 50.));
}

#[test]
fn two_layers_and_overlapping_layers_are_left_alone() {
    let mut s = Session::new();
    let two = squares(&mut s, &[0., 87.]);
    let before = (all_xy(&s, &two), s.undo_depth());
    tidy(&mut s, &two);
    assert_eq!((all_xy(&s, &two), s.undo_depth()), before, "two layers");
    assert!(!can(&s, &two));

    let mut s = Session::new();
    let tri = vec![make(&mut s, 100., 460., 50., 30.), make(&mut s, 140., 430., 80., 60.), make(&mut s, 130., 400., 30., 90.)];
    let before = (all_xy(&s, &tri), s.undo_depth());
    tidy(&mut s, &tri);
    assert_eq!((all_xy(&s, &tri), s.undo_depth()), before, "overlapping triple");
    assert!(!can(&s, &tri));
}

#[test]
fn a_tidy_that_moves_nothing_adds_no_undo_step() {
    let mut s = Session::new();
    let ids = squares(&mut s, &[0., 70., 140.]);
    let before = s.undo_depth();
    tidy(&mut s, &ids);
    assert_eq!(s.undo_depth(), before);
    assert_eq!(xs_of(&s, &ids), vec![0., 70., 140.]);
    assert!(can(&s, &ids));
}

#[test]
fn two_equally_common_gaps_give_the_smaller() {
    let mut s = Session::new();
    let ids = squares(&mut s, &[0., 60., 120., 220., 320.]);
    tidy(&mut s, &ids);
    assert_eq!(xs_of(&s, &ids), vec![0., 60., 120., 180., 240.], "not measured in Figma: a tie takes the smaller gap");
}

#[test]
fn a_mean_is_taken_down_not_rounded() {
    let mut s = Session::new();
    let ids = squares(&mut s, &[0., 60., 135.]);
    tidy(&mut s, &ids);
    assert_eq!(xs_of(&s, &ids), vec![0., 67., 134.], "not measured in Figma: a mean of 17.5 goes down");
    let mut s = Session::new();
    let ids = squares(&mut s, &[0., 60., 135., 215.]);
    tidy(&mut s, &ids);
    assert_eq!(xs_of(&s, &ids), vec![0., 71., 142., 213.], "not measured in Figma: a mean of 21.67 goes down");
}

#[test]
fn layers_on_a_diagonal_are_a_row() {
    let mut s = Session::new();
    let ids = vec![make(&mut s, 0., 0., 50., 50.), make(&mut s, 60., 70., 50., 50.), make(&mut s, 140., 150., 50., 50.)];
    tidy(&mut s, &ids);
    assert_eq!(all_xy(&s, &ids), vec![(0., 0.), (70., 70.), (140., 150.)], "not measured in Figma: a diagonal is a row");
}

#[test]
fn a_grid_with_an_empty_place_stacks_each_column() {
    let mut s = Session::new();
    let ids = vec![
        make(&mut s, 0., 0., 50., 50.),
        make(&mut s, 70., 0., 50., 50.),
        make(&mut s, 150., 0., 50., 50.),
        make(&mut s, 0., 80., 50., 50.),
        make(&mut s, 72., 90., 50., 50.),
    ];
    tidy(&mut s, &ids);
    assert_eq!(all_xy(&s, &ids), vec![(0., 0.), (75., 0.), (150., 0.), (0., 85.), (75., 85.)], "not measured in Figma: an empty place in a grid");
}

#[test]
fn one_overlapping_pair_makes_tidy_up_unavailable() {
    let mut s = Session::new();
    let ids = squares(&mut s, &[0., 60., 105., 170.]);
    let before = (xs_of(&s, &ids), s.undo_depth());
    tidy(&mut s, &ids);
    assert_eq!((xs_of(&s, &ids), s.undo_depth()), before, "not measured in Figma: one overlapping pair");
    assert!(!can(&s, &ids), "not measured in Figma: one overlapping pair");
}

#[test]
fn layers_in_an_auto_layout_flow_are_left_alone() {
    let mut s = Session::new();
    let f = NodeId(
        s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 400, "height": 100, "props": {"layout": "horizontal"}})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let ids: Vec<NodeId> = (0..3)
        .map(|_| NodeId(s.execute("shape.rectangle", &json!({"width": 50, "height": 50, "parent": f})).unwrap()["id"].as_u64().unwrap()))
        .collect();
    let before = (all_xy(&s, &ids), s.undo_depth());
    tidy(&mut s, &ids);
    assert_eq!((all_xy(&s, &ids), s.undo_depth()), before, "not measured in Figma: auto layout flow children");
    assert!(!can(&s, &ids), "not measured in Figma: auto layout flow children");
    for id in &ids {
        s.execute("node.set", &json!({"id": id, "props": {"absolute": true}})).unwrap();
    }
    for (id, x) in ids.iter().zip([0., 60., 160.]) {
        s.execute("node.set", &json!({"id": id, "props": {"x": x, "y": 0}})).unwrap();
    }
    assert_eq!(xs_of(&s, &ids), vec![0., 60., 160.]);
    assert!(can(&s, &ids));
    tidy(&mut s, &ids);
    assert_eq!(xs_of(&s, &ids), vec![0., 80., 160.]);
}

#[test]
fn an_irregular_selection_keeps_the_earlier_rule() {
    let mut s = Session::new();
    let ids = vec![make(&mut s, 0., 0., 200., 30.), make(&mut s, 0., 50., 90., 50.), make(&mut s, 110., 50., 90., 50.)];
    tidy(&mut s, &ids);
    assert_eq!(all_xy(&s, &ids), vec![(0., 0.), (0., 70.), (220., 70.)]);
}

#[test]
fn spacing_sets_the_gap_and_takes_two_layers() {
    let mut s = Session::new();
    let ids = squares(&mut s, &[0., 60., 160.]);
    s.execute("arrange.tidy", &json!({"ids": ids, "spacing": 12})).unwrap();
    assert_eq!(xs_of(&s, &ids), vec![0., 62., 124.]);
    let mut s = Session::new();
    let two = squares(&mut s, &[0., 87.]);
    s.execute("arrange.tidy", &json!({"ids": two, "spacing": 12})).unwrap();
    assert_eq!(xs_of(&s, &two), vec![0., 62.]);
    assert!(!can(&s, &two));
}

#[test]
fn without_pixel_snapping_a_centred_layer_keeps_its_half_pixel() {
    let mut s = Session::new();
    s.execute("view.snapPixel", &json!({"on": false})).unwrap();
    let ids = grid(&mut s);
    tidy(&mut s, &ids);
    assert_eq!(
        all_xy(&s, &ids),
        vec![(6., -3.), (93., -3.), (202.5, -3.), (1., 78.), (108., 98.), (195., 88.)],
        "not measured in Figma: no rounding with Snap to pixel grid off"
    );
}

#[test]
fn a_rotated_layer_is_tidied_by_its_box() {
    let mut s = Session::new();
    let a = make(&mut s, 0., 0., 50., 50.);
    let m = make(&mut s, 80., 0., 40., 40.);
    let c = make(&mut s, 200., 0., 50., 50.);
    s.execute("node.set", &json!({"id": m, "props": {"rotation": 30}})).unwrap();
    let boxes = |s: &Session| -> Vec<figcraft_geom::Rect> { [a, m, c].iter().map(|i| s.doc().unwrap().world_bounds(*i).unwrap()).collect() };
    let b = boxes(&s);
    let (g1, g2) = (b[1].x0 - b[0].x1, b[2].x0 - b[1].x1);
    let want = ((g1 + g2) / 2.0).floor();
    let rot = figcraft_geom::rotation_degrees(s.doc().unwrap().node(m).unwrap().transform);
    tidy(&mut s, &[a, m, c]);
    let b = boxes(&s);
    assert_eq!(b[0].x0, 0.0);
    assert!(((b[1].x0 - b[0].x1) - want).abs() < 1e-6);
    assert!(((b[2].x0 - b[1].x1) - want).abs() < 1e-6);
    let after = figcraft_geom::rotation_degrees(s.doc().unwrap().node(m).unwrap().transform);
    assert!((rot - after).abs() < 1e-9, "rotation unchanged");
}
