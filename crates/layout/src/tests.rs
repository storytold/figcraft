use super::*;
use figcraft_doc::{AutoLayout, Document, NodeKind};

fn setup(mode: LayoutMode) -> (Document, NodeId, Vec<NodeId>) {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 300.0, 100.0).unwrap();
    d.get_mut(f).unwrap().layout = Some(AutoLayout { mode, gap: 10.0, padding: [5.0, 5.0, 5.0, 5.0], ..Default::default() });
    let a = d.create(f, NodeKind::Rectangle, "A", 0.0, 0.0, 50.0, 20.0).unwrap();
    let b = d.create(f, NodeKind::Rectangle, "B", 0.0, 0.0, 30.0, 40.0).unwrap();
    (d, f, vec![a, b])
}

pub(super) fn pos(d: &Document, id: NodeId) -> (f64, f64, f64, f64) {
    let n = d.node(id).unwrap();
    let c = n.transform.as_coeffs();
    (c[4], c[5], n.width, n.height)
}

#[test]
fn horizontal_stack() {
    let (mut d, f, k) = setup(LayoutMode::Horizontal);
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(pos(&d, k[0]), (5.0, 5.0, 50.0, 20.0));
    assert_eq!(pos(&d, k[1]), (65.0, 5.0, 30.0, 40.0));
}

#[test]
fn hug_container() {
    let (mut d, f, _) = setup(LayoutMode::Horizontal);
    let n = d.get_mut(f).unwrap();
    n.child_layout.sizing_h = Sizing::Hug;
    n.child_layout.sizing_v = Sizing::Hug;
    layout_frame(&mut d, f, &NoMeasure);
    let n = d.node(f).unwrap();
    assert_eq!((n.width, n.height), (5.0 + 50.0 + 10.0 + 30.0 + 5.0, 50.0));
}

#[test]
fn vertical_center_alignment() {
    let (mut d, f, k) = setup(LayoutMode::Vertical);
    d.get_mut(f).unwrap().layout.as_mut().unwrap().cross_align = CrossAlign::Center;
    layout_frame(&mut d, f, &NoMeasure);
    // Inner width 290; A (50 wide) centred → x = 5 + 120.
    assert_eq!(pos(&d, k[0]).0, 125.0);
    assert_eq!(pos(&d, k[1]).1, 5.0 + 20.0 + 10.0);
}

#[test]
fn fill_children_share_space() {
    let (mut d, f, k) = setup(LayoutMode::Horizontal);
    for c in &k {
        d.get_mut(*c).unwrap().child_layout.sizing_h = Sizing::Fill;
    }
    d.get_mut(k[1]).unwrap().child_layout.sizing_v = Sizing::Fill;
    layout_frame(&mut d, f, &NoMeasure);
    // Inner 290 - gap 10 = 280 → 140 each.
    assert_eq!(pos(&d, k[0]).2, 140.0);
    assert_eq!(pos(&d, k[1]).0, 5.0 + 140.0 + 10.0);
    assert_eq!(pos(&d, k[1]).3, 90.0);
    // Max width on one.
    d.get_mut(k[0]).unwrap().child_layout.max_w = Some(100.0);
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(pos(&d, k[0]).2, 100.0);
    assert_eq!(pos(&d, k[1]).2, 180.0);
}

#[test]
fn space_between_and_absolute() {
    let (mut d, f, k) = setup(LayoutMode::Horizontal);
    d.get_mut(f).unwrap().layout.as_mut().unwrap().main_align = MainAlign::SpaceBetween;
    let abs = d.create(f, NodeKind::Rectangle, "Abs", 7.0, 7.0, 10.0, 10.0).unwrap();
    d.get_mut(abs).unwrap().child_layout.absolute = true;
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(pos(&d, k[1]).0, 300.0 - 5.0 - 30.0);
    assert_eq!(pos(&d, abs).0, 7.0);
}

#[test]
fn wrap_into_rows() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(f).unwrap().layout = Some(AutoLayout { wrap: true, gap: 10.0, cross_gap: 5.0, padding: [0.0; 4], ..Default::default() });
    d.get_mut(f).unwrap().child_layout.sizing_v = Sizing::Hug;
    let ids: Vec<NodeId> = (0..5).map(|i| d.create(f, NodeKind::Rectangle, &format!("R{i}"), 0.0, 0.0, 40.0, 20.0).unwrap()).collect();
    layout_frame(&mut d, f, &NoMeasure);
    // Two per row (40+10+40 = 90 ≤ 100).
    assert_eq!(pos(&d, ids[2]), (0.0, 25.0, 40.0, 20.0));
    assert_eq!(d.node(f).unwrap().height, 20.0 * 3.0 + 5.0 * 2.0);
}

#[test]
fn grid_mode() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 210.0, 110.0).unwrap();
    d.get_mut(f).unwrap().layout = Some(AutoLayout {
        mode: LayoutMode::Grid,
        grid_columns: 2,
        grid_rows: 2,
        gap: 10.0,
        cross_gap: 10.0,
        padding: [0.0; 4],
        ..Default::default()
    });
    let ids: Vec<NodeId> = (0..4).map(|i| d.create(f, NodeKind::Rectangle, &format!("R{i}"), 0.0, 0.0, 10.0, 10.0).unwrap()).collect();
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(pos(&d, ids[3]).0, 110.0);
    assert_eq!(pos(&d, ids[3]).1, 60.0);
}

#[test]
fn nested_hug_propagates_with_layout_page() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let outer = d.create(p, NodeKind::Frame, "Outer", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(outer).unwrap().layout = Some(AutoLayout { padding: [0.0; 4], ..Default::default() });
    d.get_mut(outer).unwrap().child_layout.sizing_h = Sizing::Hug;
    let inner = d.create(outer, NodeKind::Frame, "Inner", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(inner).unwrap().layout = Some(AutoLayout { padding: [0.0; 4], gap: 0.0, ..Default::default() });
    d.get_mut(inner).unwrap().child_layout.sizing_h = Sizing::Hug;
    d.create(inner, NodeKind::Rectangle, "R", 0.0, 0.0, 77.0, 10.0).unwrap();
    layout_page(&mut d, p, &NoMeasure);
    assert_eq!(d.node(inner).unwrap().width, 77.0);
    assert_eq!(d.node(outer).unwrap().width, 77.0);
}

#[test]
fn constraints_on_resize() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    let r = d.create(f, NodeKind::Rectangle, "R", 80.0, 10.0, 10.0, 10.0).unwrap();
    d.get_mut(r).unwrap().constraints = figcraft_doc::Constraints { horizontal: HConstraint::Right, vertical: VConstraint::TopBottom };
    let c = d.create(f, NodeKind::Rectangle, "C", 45.0, 45.0, 10.0, 10.0).unwrap();
    d.get_mut(c).unwrap().constraints = figcraft_doc::Constraints { horizontal: HConstraint::Center, vertical: VConstraint::Scale };
    set_size(&mut d, f, 200.0, 200.0);
    apply_constraints(&mut d, f, 100.0, 100.0);
    assert_eq!(pos(&d, r), (180.0, 10.0, 10.0, 110.0));
    assert_eq!(pos(&d, c), (95.0, 90.0, 10.0, 20.0));
}

#[test]
fn hostile_values() {
    let (mut d, f, _) = setup(LayoutMode::Horizontal);
    let n = d.get_mut(f).unwrap();
    n.layout.as_mut().unwrap().gap = f64::NAN;
    n.layout.as_mut().unwrap().padding = [f64::INFINITY, -1.0, f64::NAN, 1e300];
    n.layout.as_mut().unwrap().grid_columns = 0;
    layout_frame(&mut d, f, &NoMeasure);
    d.get_mut(f).unwrap().layout.as_mut().unwrap().mode = LayoutMode::Grid;
    layout_frame(&mut d, f, &NoMeasure);
    let _ = grid_lines(&LayoutGrid { size: 0.0, ..Default::default() }, 1e9, 1e9);
}

/// A fake measurer: each text has a natural (unwrapped) size and a first baseline; wrapping at a
/// narrower width adds lines of the natural height.
pub(super) struct FakeMeasure(pub(super) std::collections::HashMap<NodeId, (f64, f64, f64)>);

impl Measure for FakeMeasure {
    fn text_height(&self, _: &Document, id: NodeId, width: f64) -> Option<f64> {
        let (w, h, _) = self.0.get(&id).copied()?;
        Some(h * (w / width.max(1.0)).ceil().max(1.0))
    }
    fn text_size(&self, _: &Document, id: NodeId) -> Option<(f64, f64)> {
        self.0.get(&id).map(|(w, h, _)| (*w, *h))
    }
    fn text_baseline(&self, _: &Document, id: NodeId) -> Option<f64> {
        self.0.get(&id).map(|t| t.2)
    }
}

pub(super) fn text(d: &mut Document, parent: NodeId, name: &str) -> NodeId {
    d.create(parent, NodeKind::Text(figcraft_doc::TextData::new(name)), name, 0.0, 0.0, 10.0, 10.0).unwrap()
}

#[test]
fn min_max_clamp_fixed_children_and_frame() {
    let (mut d, f, k) = setup(LayoutMode::Horizontal);
    d.get_mut(k[0]).unwrap().child_layout.min_w = Some(80.0);
    d.get_mut(k[1]).unwrap().child_layout.max_h = Some(25.0);
    d.get_mut(f).unwrap().child_layout.max_w = Some(200.0);
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(pos(&d, k[0]).2, 80.0);
    assert_eq!(pos(&d, k[1]), (95.0, 5.0, 30.0, 25.0));
    // A fixed frame wider than its max width is clamped too.
    assert_eq!(d.node(f).unwrap().width, 200.0);
}

#[test]
fn min_max_on_hug_and_fill() {
    let (mut d, f, _) = setup(LayoutMode::Horizontal);
    {
        let n = d.get_mut(f).unwrap();
        n.child_layout.sizing_h = Sizing::Hug;
        n.child_layout.min_w = Some(200.0);
    }
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(d.node(f).unwrap().width, 200.0);
    {
        let n = d.get_mut(f).unwrap();
        n.child_layout.min_w = None;
        n.child_layout.max_w = Some(60.0);
    }
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(d.node(f).unwrap().width, 60.0);
    // A fill child with a min width wider than its share keeps the min.
    let (mut d, f, k) = setup(LayoutMode::Horizontal);
    for c in &k {
        d.get_mut(*c).unwrap().child_layout.sizing_h = Sizing::Fill;
    }
    d.get_mut(k[0]).unwrap().child_layout.min_w = Some(200.0);
    layout_frame(&mut d, f, &NoMeasure);
    assert_eq!(pos(&d, k[0]).2, 200.0);
    assert_eq!(pos(&d, k[1]).2, 80.0);
}

#[test]
fn min_over_max_and_junk_limits() {
    let (mut d, f, k) = setup(LayoutMode::Vertical);
    {
        let cl = &mut d.get_mut(k[0]).unwrap().child_layout;
        cl.min_w = Some(90.0);
        cl.max_w = Some(40.0);
        cl.min_h = Some(f64::NAN);
        cl.max_h = Some(f64::NEG_INFINITY);
    }
    layout_frame(&mut d, f, &NoMeasure);
    // Min wins over a smaller max; non-finite limits are ignored.
    assert_eq!(pos(&d, k[0]).2, 90.0);
    assert_eq!(pos(&d, k[0]).3, 20.0);
    assert_eq!(clamp_size(-5.0, None, None), 0.0);
    assert_eq!(clamp_size(f64::NAN, Some(3.0), None), 3.0);
}

#[test]
fn hug_text_wraps_at_max_width() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(f).unwrap().layout = Some(AutoLayout { padding: [0.0; 4], ..Default::default() });
    let t = text(&mut d, f, "Long");
    d.get_mut(t).unwrap().child_layout.sizing_h = Sizing::Hug;
    let m = FakeMeasure([(t, (200.0, 20.0, 15.0))].into_iter().collect());
    layout_frame(&mut d, f, &m);
    assert_eq!(pos(&d, t).2, 200.0);
    d.get_mut(t).unwrap().child_layout.max_w = Some(80.0);
    layout_frame(&mut d, f, &m);
    let n = d.node(t).unwrap();
    assert_eq!((n.width, n.height), (80.0, 60.0));
    assert!(matches!(&n.kind, NodeKind::Text(tx) if tx.auto_resize == TextAutoResize::Height));
    // Without the max it hugs its natural width again.
    d.get_mut(t).unwrap().child_layout.max_w = None;
    layout_frame(&mut d, f, &m);
    let n = d.node(t).unwrap();
    assert_eq!((n.width, n.height), (200.0, 20.0));
    assert!(matches!(&n.kind, NodeKind::Text(tx) if tx.auto_resize == TextAutoResize::WidthAndHeight));
}

#[test]
fn absolute_children_follow_constraints() {
    let (mut d, f, _) = setup(LayoutMode::Horizontal);
    let abs = d.create(f, NodeKind::Rectangle, "Badge", 280.0, 4.0, 10.0, 10.0).unwrap();
    {
        let n = d.get_mut(abs).unwrap();
        n.child_layout.absolute = true;
        n.constraints = figcraft_doc::Constraints { horizontal: HConstraint::Right, vertical: VConstraint::Top };
    }
    d.get_mut(f).unwrap().child_layout.sizing_h = Sizing::Hug;
    layout_frame(&mut d, f, &NoMeasure);
    // Hugging shrank the frame 300 → 100: the badge keeps its distance from the right edge.
    assert_eq!(d.node(f).unwrap().width, 100.0);
    assert_eq!(pos(&d, abs), (80.0, 4.0, 10.0, 10.0));
    assert!(!flow_children(&d, f).contains(&abs));
    // A plain resize of an auto layout frame moves only absolute children by their constraints.
    let first = pos(&d, d.node(f).unwrap().children[0]);
    set_size(&mut d, f, 150.0, 100.0);
    apply_constraints(&mut d, f, 100.0, 100.0);
    assert_eq!(pos(&d, abs).0, 130.0);
    assert_eq!(pos(&d, d.node(f).unwrap().children[0]), first);
}

#[test]
fn baseline_alignment_horizontal() {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(f).unwrap().layout = Some(AutoLayout { padding: [5.0; 4], gap: 0.0, cross_align: CrossAlign::Baseline, ..Default::default() });
    d.get_mut(f).unwrap().child_layout.sizing_v = Sizing::Hug;
    let small = text(&mut d, f, "small");
    let big = text(&mut d, f, "big");
    let rect = d.create(f, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 20.0).unwrap();
    let m = FakeMeasure([(small, (30.0, 14.0, 10.0)), (big, (60.0, 40.0, 30.0))].into_iter().collect());
    layout_frame(&mut d, f, &m);
    // Baselines line up at 5 + 30; the rectangle (no baseline) sits at the top.
    assert_eq!(pos(&d, small).1, 25.0);
    assert_eq!(pos(&d, big).1, 5.0);
    assert_eq!(pos(&d, rect).1, 5.0);
    // Hug height covers the shifted small text: max(20 + 14, 40) + padding.
    assert_eq!(d.node(f).unwrap().height, 50.0);
    // Nested auto layout frames align on their first child's baseline.
    let outer = d.create(p, NodeKind::Frame, "O", 0.0, 0.0, 400.0, 100.0).unwrap();
    d.get_mut(outer).unwrap().layout = Some(AutoLayout { padding: [0.0; 4], gap: 0.0, cross_align: CrossAlign::Baseline, ..Default::default() });
    d.reparent(f, outer, None).unwrap();
    let t2 = text(&mut d, outer, "t2");
    let m = FakeMeasure([(small, (30.0, 14.0, 10.0)), (big, (60.0, 40.0, 30.0)), (t2, (20.0, 60.0, 50.0))].into_iter().collect());
    layout_page(&mut d, p, &m);
    // Frame baseline = 5 + 30 = 35; t2's is 50 → the frame moves down 15.
    assert_eq!(pos(&d, f).1, 15.0);
    assert_eq!(pos(&d, t2).1, 0.0);
    // Vertical layouts read baseline as left.
    d.get_mut(outer).unwrap().layout.as_mut().unwrap().mode = LayoutMode::Vertical;
    layout_frame(&mut d, outer, &m);
    assert_eq!(pos(&d, f).0, 0.0);
    assert_eq!(pos(&d, t2).0, 0.0);
}

fn grid_frame(cols: u32, w: f64, h: f64) -> (Document, NodeId) {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "G", 0.0, 0.0, w, h).unwrap();
    d.get_mut(f).unwrap().layout = Some(AutoLayout {
        mode: LayoutMode::Grid,
        grid_columns: cols,
        grid_rows: 1,
        gap: 10.0,
        cross_gap: 10.0,
        padding: [0.0; 4],
        ..Default::default()
    });
    (d, f)
}

#[test]
fn grid_tracks_fixed_hug_fr() {
    use figcraft_doc::GridTrack;
    let (mut d, f) = grid_frame(4, 400.0, 100.0);
    d.get_mut(f).unwrap().layout.as_mut().unwrap().column_tracks =
        vec![GridTrack::fixed(50.0), GridTrack::hug(), GridTrack::fill(1.0), GridTrack::fill(2.0)];
    let ids: Vec<NodeId> = (0..4).map(|i| d.create(f, NodeKind::Rectangle, &format!("R{i}"), 0.0, 0.0, 30.0 + i as f64, 20.0).unwrap()).collect();
    for c in &ids {
        d.get_mut(*c).unwrap().child_layout.sizing_h = Sizing::Fill;
    }
    // The hug column fits its (fixed) child.
    d.get_mut(ids[1]).unwrap().child_layout.sizing_h = Sizing::Fixed;
    layout_frame(&mut d, f, &NoMeasure);
    let g = grid_geometry(&d, f).unwrap();
    // 400 - 3 gaps (30) - 50 - 31 = 289 free → 1fr 96.33, 2fr 192.67.
    assert_eq!(g.columns[0], (0.0, 50.0));
    assert_eq!(g.columns[1], (60.0, 91.0));
    assert!((g.columns[2].1 - g.columns[2].0 - 289.0 / 3.0).abs() < 1e-9);
    assert!((g.columns[3].1 - 400.0).abs() < 1e-9);
    assert_eq!(pos(&d, ids[0]).2, 50.0);
    assert!((pos(&d, ids[3]).2 - 289.0 * 2.0 / 3.0).abs() < 1e-9);
    // The only row fills the height.
    assert_eq!(g.rows, vec![(0.0, 100.0)]);
}

#[test]
fn grid_cells_spans_and_anchors() {
    let (mut d, f) = grid_frame(3, 320.0, 200.0);
    let ids: Vec<NodeId> = (0..4).map(|i| d.create(f, NodeKind::Rectangle, &format!("R{i}"), 0.0, 0.0, 10.0, 10.0).unwrap()).collect();
    // R0 spans two columns and fills them; R3 is anchored at column 0, row 2.
    {
        let cl = &mut d.get_mut(ids[0]).unwrap().child_layout;
        cl.grid_column_span = 2;
        cl.sizing_h = Sizing::Fill;
    }
    {
        let cl = &mut d.get_mut(ids[3]).unwrap().child_layout;
        cl.grid_column = Some(0);
        cl.grid_row = Some(2);
    }
    layout_frame(&mut d, f, &NoMeasure);
    let g = grid_geometry(&d, f).unwrap();
    let cell = |id: NodeId| g.cells.iter().find(|c| c.id == id).copied().unwrap();
    assert_eq!((cell(ids[0]).column, cell(ids[0]).row, cell(ids[0]).column_span), (0, 0, 2));
    assert_eq!((cell(ids[1]).column, cell(ids[1]).row), (2, 0));
    assert_eq!((cell(ids[2]).column, cell(ids[2]).row), (0, 1));
    assert_eq!((cell(ids[3]).column, cell(ids[3]).row), (0, 2));
    assert_eq!(g.rows.len(), 3);
    // Columns are 100 wide: the spanning child covers 100 + 10 + 100.
    assert_eq!(pos(&d, ids[0]).2, 210.0);
    assert_eq!(pos(&d, ids[1]).0, 220.0);
    assert_eq!(pos(&d, ids[3]).1, 140.0);
}

#[test]
fn grid_hug_with_span() {
    use figcraft_doc::GridTrack;
    let (mut d, f) = grid_frame(2, 10.0, 10.0);
    {
        let n = d.get_mut(f).unwrap();
        n.child_layout.sizing_h = Sizing::Hug;
        n.child_layout.sizing_v = Sizing::Hug;
        n.layout.as_mut().unwrap().column_tracks = vec![GridTrack::hug(), GridTrack::fixed(20.0)];
    }
    d.create(f, NodeKind::Rectangle, "A", 0.0, 0.0, 40.0, 10.0).unwrap();
    let wide = d.create(f, NodeKind::Rectangle, "W", 0.0, 0.0, 100.0, 30.0).unwrap();
    d.get_mut(wide).unwrap().child_layout.grid_column_span = 2;
    layout_frame(&mut d, f, &NoMeasure);
    // Column 0 grows so the span fits: 100 - 10 gap - 20 fixed = 70.
    let n = d.node(f).unwrap();
    assert_eq!((n.width, n.height), (100.0, 10.0 + 10.0 + 30.0));
    assert_eq!(pos(&d, wide).1, 20.0);
}

#[test]
fn grid_hostile_values() {
    use figcraft_doc::GridTrack;
    let (mut d, f) = grid_frame(0, f64::NAN, -5.0);
    {
        let l = d.get_mut(f).unwrap().layout.as_mut().unwrap();
        l.column_tracks = vec![GridTrack { sizing: Sizing::Fixed, value: f64::NAN }, GridTrack::fill(f64::INFINITY), GridTrack::fill(-3.0)];
        l.row_tracks = vec![GridTrack::fixed(1e300)];
        l.grid_rows = u32::MAX;
    }
    let r = d.create(f, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    {
        let cl = &mut d.get_mut(r).unwrap().child_layout;
        cl.grid_column = Some(u32::MAX);
        cl.grid_row = Some(u32::MAX);
        cl.grid_column_span = u32::MAX;
        cl.grid_row_span = u32::MAX;
        cl.sizing_h = Sizing::Fill;
    }
    for i in 0..20 {
        let c = d.create(f, NodeKind::Rectangle, &format!("K{i}"), 0.0, 0.0, 10.0, 10.0).unwrap();
        d.get_mut(c).unwrap().child_layout.grid_column_span = 3;
    }
    layout_frame(&mut d, f, &NoMeasure);
    let g = grid_geometry(&d, f).unwrap();
    assert!(g.columns.len() == 1 && g.rows.len() <= 1000);
    assert!(g.width.is_finite() && g.height.is_finite());
    for id in d.node(f).unwrap().children.clone() {
        let (x, y, w, h) = pos(&d, id);
        assert!(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite());
    }
}

#[test]
fn grid_lines_columns_stretch() {
    let g = LayoutGrid { kind: GridKind::Columns, count: 4, gutter: 20.0, offset: 10.0, ..LayoutGrid::default() };
    let r = grid_bands(&g, 400.0);
    assert_eq!(r.len(), 4);
    assert_eq!(r[0].0, 10.0);
    assert!((r[3].0 + r[3].1 - 390.0).abs() < 1e-9);
    let lines = grid_lines(&g, 400.0, 100.0);
    let GridLine::Band(first) = lines.first().expect("four columns") else { panic!("columns draw as bands") };
    assert_eq!((first.x0, first.x1), (10.0, 90.0), "a stretched column spans its share of the width");
}

#[test]
fn grid_lines_diagonal_bands_step_by_size_and_gutter() {
    // Three 2-wide 45° bands, 10 apart, each drawn both ways: their edges are the lines
    // x + y = a·√2 (`/`) and x − y = 100 − (a + size)·√2 (`\`, a mirrored across x = 50).
    let g = LayoutGrid { kind: GridKind::Diagonal, align: GridAlign::Min, count: 3, size: 2.0, gutter: 10.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 100.0, 60.0);
    assert_eq!(lines.len(), 6, "three bands, each a pair of diagonals");
    let sqrt2 = std::f64::consts::SQRT_2;
    for (i, line) in lines.iter().enumerate() {
        let band = i / 2;
        let (start, end) = [(0.0, 2.0), (12.0, 14.0), (24.0, 26.0)][band];
        let GridLine::Slash(poly) = line else { panic!("45° lines draw as bands") };
        // The `/` family measures x + y, the `\` family (odd indices) x − y.
        let along = if i % 2 == 0 { |p: &Point| p.x + p.y } else { |p: &Point| p.x - p.y };
        let (lo, hi) = if i % 2 == 0 { (start * sqrt2, end * sqrt2) } else { (100.0 - end * sqrt2, 100.0 - start * sqrt2) };
        let a = poly.iter().map(along).fold(f64::INFINITY, f64::min);
        let b = poly.iter().map(along).fold(f64::NEG_INFINITY, f64::max);
        assert!((a - lo).abs() < 1e-9 && (b - hi).abs() < 1e-9, "band {i} runs {a} … {b}, wanted {lo} … {hi}");
        // Clipped to the frame: every corner is inside 100 × 60.
        assert!(poly.iter().all(|p| (-1e-9..=100.0 + 1e-9).contains(&p.x) && (-1e-9..=60.0 + 1e-9).contains(&p.y)), "clipped: {poly:?}");
    }
    // Centred, one band: both diagonals through the middle of a 256 square, like an icon grid.
    let g = LayoutGrid { kind: GridKind::Diagonal, align: GridAlign::Center, count: 1, size: 1.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 256.0, 256.0);
    assert_eq!(lines.len(), 2, "one band, two diagonals");
    let (GridLine::Slash(slash), GridLine::Slash(back)) = (&lines[0], &lines[1]) else { panic!("45° lines draw as bands") };
    let mid = |poly: &[Point], f: &dyn Fn(&Point) -> f64| {
        (poly.iter().map(f).fold(f64::INFINITY, f64::min) + poly.iter().map(f).fold(f64::NEG_INFINITY, f64::max)) / 2.0
    };
    assert!((mid(slash, &|p| p.x + p.y) - 256.0).abs() < 0.01, "the `/` diagonal crosses the centre: {}", mid(slash, &|p| p.x + p.y));
    assert!(mid(back, &|p| p.x - p.y).abs() < 0.01, "the `\\` diagonal crosses the centre: {}", mid(back, &|p| p.x - p.y));
}

#[test]
fn grid_lines_rings_run_out_from_the_centre() {
    // Two 4-thick rings, 6 apart, from the centre of a 200 × 100 frame: the inscribed radius is 50.
    let g = LayoutGrid { kind: GridKind::Rings, align: GridAlign::Min, count: 2, size: 4.0, gutter: 6.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 200.0, 100.0);
    assert_eq!(lines.len(), 2);
    for (line, radius) in lines.iter().zip([2.0, 12.0]) {
        let GridLine::Ring { center, radius: r, width } = line else { panic!("rings draw as rings") };
        assert_eq!((center.x, center.y), (100.0, 50.0), "rings sit at the frame's centre");
        assert!((r - radius).abs() < 1e-9 && (*width - 4.0).abs() < 1e-9, "a {width}-thick ring at {r}: wanted {radius}");
    }
    // Stretched rings fill the inscribed circle, margin and gutter included.
    let g = LayoutGrid { kind: GridKind::Rings, count: 2, size: 0.0, gutter: 10.0, offset: 5.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 200.0, 100.0);
    let GridLine::Ring { radius, width, .. } = lines[0] else { panic!("rings") };
    assert!((width - 15.0).abs() < 1e-9, "margin 5 and gutter 10 split 50 - 2·5: {width}");
    assert!((radius - 12.5).abs() < 1e-9, "the first ring's middle: {radius}");
}

#[test]
fn grid_lines_stretch_computes_45_degree_and_ring_sizes() {
    // A stretched 45° grid divides the frame's diagonal: bands, not hairlines, whatever `size`
    // says. Three bands with a 20 gutter across (400 + 400)/√2.
    let g = LayoutGrid { kind: GridKind::Diagonal, align: GridAlign::Stretch, count: 3, size: 0.0, gutter: 20.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 400.0, 400.0);
    assert_eq!(lines.len(), 6, "three bands, each a pair of diagonals");
    assert!(lines.iter().all(|l| matches!(l, GridLine::Slash(_))), "stretch draws bands: {lines:?}");
    // Stretched rings divide the inscribed radius the same way: three rings of a real width, the
    // outermost ending at the circle the frame holds (its middle plus half its width).
    let g = LayoutGrid { kind: GridKind::Rings, align: GridAlign::Stretch, count: 3, size: 0.0, gutter: 20.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 400.0, 400.0);
    assert_eq!(lines.len(), 3, "three rings");
    let mut outer = 0.0_f64;
    for l in &lines {
        let GridLine::Ring { radius, width, .. } = l else { panic!("rings draw as rings: {l:?}") };
        assert!(*width > 0.0, "a stretched ring has a width: {l:?}");
        outer = outer.max(radius + width / 2.0);
    }
    assert!((outer - 200.0).abs() < 1e-9, "the rings stay inside the frame: {outer}");
    // A frame wider than it is tall is bounded by its height, not its width.
    let lines = grid_lines(&g, 400.0, 300.0);
    let outer = lines
        .iter()
        .filter_map(|l| match l {
            GridLine::Ring { radius, width, .. } => Some(radius + width / 2.0),
            _ => None,
        })
        .fold(0.0_f64, f64::max);
    assert!((outer - 150.0).abs() < 1e-9, "rings fit the short side: {outer}");
}

#[test]
fn auto_rings_past_the_centre_are_left_out() {
    // Centre alignment with more rings than the radius holds puts a band's middle past zero:
    // drawing it would show a ring mirrored through the centre, at the wrong place.
    let g = LayoutGrid { kind: GridKind::Rings, align: GridAlign::Center, count: 20, size: 0.0, gutter: 20.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 200.0, 200.0);
    assert!(!lines.is_empty(), "the rings that fit are drawn");
    for l in &lines {
        let GridLine::Ring { radius, .. } = l else { panic!("rings draw as rings: {l:?}") };
        assert!(*radius > 0.0, "no ring at or past the centre: {l:?}");
    }
}

#[test]
fn grid_lines_auto_width_is_a_hairline() {
    // Columns with Auto width (size 0, non-stretched) draw one-pixel lines at each band's start,
    // not filled bands.
    let g = LayoutGrid { kind: GridKind::Columns, align: GridAlign::Min, count: 3, size: 0.0, gutter: 10.0, offset: 0.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 300.0, 50.0);
    assert_eq!(lines.len(), 3, "three hairlines");
    for line in &lines {
        let GridLine::Line { a, b } = line else { panic!("auto columns draw hairlines: {line:?}") };
        assert!((b.x - a.x).abs() < 1e-9 && (b.y - a.y).abs() > 1.0, "a vertical hairline: {line:?}");
    }
    // Auto has no width of its own, so the gutter alone is the step between the lines (no band
    // width hidden in it).
    let xs: Vec<f64> = lines
        .iter()
        .map(|l| match l {
            GridLine::Line { a, .. } => a.x,
            _ => f64::NAN,
        })
        .collect();
    assert_eq!(xs, [0.0, 10.0, 20.0], "Auto steps by the gutter: {xs:?}");
    // Auto count: as many hairlines as the gutter fits.
    let g = LayoutGrid { kind: GridKind::Columns, align: GridAlign::Min, count: 0, size: 0.0, gutter: 10.0, offset: 0.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 300.0, 50.0);
    assert_eq!(lines.len(), 31, "300 wide, 10 apart: 31 lines");
    // A positive width draws filled bands again.
    let g = LayoutGrid { kind: GridKind::Columns, align: GridAlign::Min, count: 3, size: 8.0, gutter: 10.0, offset: 0.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 300.0, 50.0);
    assert!(lines.iter().all(|l| matches!(l, GridLine::Band(_))), "a set width draws bands");

    // Rows Auto width: horizontal hairlines.
    let g = LayoutGrid { kind: GridKind::Rows, align: GridAlign::Min, count: 2, size: 0.0, gutter: 10.0, offset: 0.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 50.0, 200.0);
    assert_eq!(lines.len(), 2);
    for line in &lines {
        let GridLine::Line { a, b } = line else { panic!("auto rows draw hairlines") };
        assert!((b.y - a.y).abs() < 1e-9 && (b.x - a.x).abs() > 1.0, "a horizontal hairline: {line:?}");
    }

    // 45° Auto width draws hairline segments both ways, through the centre.
    let g = LayoutGrid { kind: GridKind::Diagonal, align: GridAlign::Center, count: 1, size: 0.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 256.0, 256.0);
    assert_eq!(lines.len(), 2, "one band, two hairlines");
    assert!(lines.iter().all(|l| matches!(l, GridLine::SlashLine(_))), "auto 45° lines are hairlines");
    let (GridLine::SlashLine(slash), GridLine::SlashLine(back)) = (&lines[0], &lines[1]) else { panic!("hairlines") };
    let mid = |poly: &[Point], f: &dyn Fn(&Point) -> f64| {
        (poly.iter().map(f).fold(f64::INFINITY, f64::min) + poly.iter().map(f).fold(f64::NEG_INFINITY, f64::max)) / 2.0
    };
    assert!((mid(slash, &|p| p.x + p.y) - 256.0).abs() < 0.01, "the `/` hairline crosses the centre: {}", mid(slash, &|p| p.x + p.y));
    assert!(mid(back, &|p| p.x - p.y).abs() < 0.01, "the `\\` hairline crosses the centre: {}", mid(back, &|p| p.x - p.y));

    // Rings Auto width draw one-pixel circles (width 0).
    let g = LayoutGrid { kind: GridKind::Rings, align: GridAlign::Min, count: 2, size: 0.0, gutter: 6.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 200.0, 100.0);
    assert_eq!(lines.len(), 2);
    for line in &lines {
        let GridLine::Ring { width, .. } = line else { panic!("rings") };
        assert_eq!(*width, 0.0, "auto rings are hairlines");
    }
    // A stretched grid works its bands out from the frame whatever its width says.
    let g =
        LayoutGrid { kind: GridKind::Columns, align: GridAlign::Stretch, count: 4, size: 0.0, gutter: 10.0, offset: 0.0, ..LayoutGrid::default() };
    let lines = grid_lines(&g, 400.0, 50.0);
    let bands: Vec<Rect> = lines
        .iter()
        .filter_map(|l| match l {
            GridLine::Band(r) => Some(*r),
            _ => None,
        })
        .collect();
    assert_eq!(bands.len(), 4, "stretch draws bands, not hairlines: {lines:?}");
    assert!((bands[0].x0 - 0.0).abs() < 1e-9 && (bands[3].x1 - 400.0).abs() < 1e-9, "the bands fill the frame");
}
