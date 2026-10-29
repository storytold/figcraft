//! Live boolean editing in the renderer: per-operand operations, how much of a group is
//! recomputed when one operand changes, the bounded path caches, and one-sided widths on screen.

use super::*;
use crate::boolean::{PathCache, boolean_steps, fold_cached};
use figcraft_color::{Color, Paint};
use figcraft_doc::{BoolOp, Document, NodeId, NodeKind, WidthPoint, WidthProfile};
use figcraft_geom::kurbo::{BezPath, Shape};

fn dark(p: &Pixmap, x: u32, y: u32) -> bool {
    let i = ((y * p.width() + x) * 4) as usize;
    p.data().get(i).is_some_and(|r| *r < 40)
}

fn render(d: &Document, p: NodeId) -> Pixmap {
    render_view(d, p, &View { origin: (0.0, 0.0), scale: 1.0, width: 200, height: 200 }, &Options::default()).unwrap()
}

/// A boolean group doing `op` over squares 20 wide at x = 10, 25, 40, … (overlapping neighbours),
/// on a page of its own.
fn row_of_squares(op: BoolOp, n: usize) -> (Document, NodeId, NodeId, Vec<NodeId>) {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let b = d.create(p, NodeKind::BooleanOp { op }, "B", 0.0, 0.0, 0.0, 0.0).unwrap();
    d.get_mut(b).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let kids = (0..n).map(|i| d.create(b, NodeKind::Rectangle, "R", 10.0 + 15.0 * i as f64, 10.0, 20.0, 20.0).unwrap()).collect();
    d.fit_to_children(b).unwrap();
    (d, p, b, kids)
}

fn outline(d: &Document, id: NodeId) -> BezPath {
    node_outline(d, d.node(id).unwrap()).unwrap().0
}

#[test]
fn operands_combine_by_their_own_operations() {
    // Union of three squares in a row, the middle one subtracted instead.
    let (mut d, p, b, kids) = row_of_squares(BoolOp::Union, 3);
    let union = outline(&d, b).area().abs();
    assert!((union - 20.0 * 50.0).abs() < 0.5, "{union}");
    d.get_mut(kids[1]).unwrap().bool_op = Some(BoolOp::Subtract);
    let mixed = outline(&d, b);
    // (A − B) ∪ C: A keeps x 10..25, C is whole (x 40..60).
    assert!((mixed.area().abs() - 20.0 * (15.0 + 20.0)).abs() < 0.5, "{}", mixed.area());
    let pix = render(&d, p);
    assert!(dark(&pix, 15, 20) && !dark(&pix, 32, 20) && dark(&pix, 50, 20));
    // The bottom operand's own operation changes nothing.
    d.get_mut(kids[0]).unwrap().bool_op = Some(BoolOp::Intersect);
    assert!((outline(&d, b).area().abs() - mixed.area().abs()).abs() < 1e-9);
}

#[test]
fn editing_one_operand_recombines_only_from_it_up() {
    // Distinct sizes so no other test's cached results can be reused.
    let (mut d, _, b, kids) = row_of_squares(BoolOp::Union, 8);
    for (i, k) in kids.iter().enumerate() {
        d.get_mut(*k).unwrap().height = 20.0 + i as f64 * 0.37;
    }
    let steps = |d: &Document| {
        let before = boolean_steps();
        let _ = outline(d, b);
        boolean_steps() - before
    };
    assert_eq!(steps(&d), 7, "the first time: every operand above the bottom one");
    assert_eq!(steps(&d), 0, "unchanged: from the cache");
    // Moving the sixth operand (a drag frame) recombines it and the two above it.
    let mut moved = d.clone();
    moved.get_mut(kids[5]).unwrap().transform = figcraft_geom::Affine::translate((3.0, 1.5)) * d.node(kids[5]).unwrap().transform;
    assert_eq!(steps(&moved), 3);
    // The top one: one step. Going back: cached.
    let mut top = moved.clone();
    top.get_mut(kids[7]).unwrap().height = 33.3;
    assert_eq!(steps(&top), 1);
    assert_eq!(steps(&moved), 0);
    assert_eq!(steps(&d), 0);
    // Hiding an operand drops it from the fold (those above it are recombined).
    let mut hidden = d.clone();
    hidden.get_mut(kids[6]).unwrap().visible = false;
    assert_eq!(steps(&hidden), 1);
}

#[test]
fn other_groups_are_not_recomputed() {
    let (mut d, _, b, kids) = row_of_squares(BoolOp::Exclude, 4);
    let (d2, _, b2, _) = row_of_squares(BoolOp::Intersect, 5);
    for (i, k) in kids.iter().enumerate() {
        d.get_mut(*k).unwrap().width = 21.0 + i as f64 * 0.29;
    }
    let _ = outline(&d, b);
    let _ = outline(&d2, b2);
    let before = boolean_steps();
    d.get_mut(kids[3]).unwrap().width = 30.1;
    let _ = outline(&d, b);
    let _ = outline(&d2, b2);
    assert_eq!(boolean_steps() - before, 1, "only the edited group's top step");
}

#[test]
fn caches_drop_the_least_recently_used_half() {
    let mut c = PathCache::new(4);
    let p = std::sync::Arc::new(BezPath::new());
    for k in 0..4 {
        c.insert(k, p.clone());
    }
    // Using 0 and 1 keeps them when the cache fills up.
    assert!(c.get(0).is_some() && c.get(1).is_some());
    c.insert(9, p.clone());
    assert!(c.get(0).is_some() && c.get(1).is_some() && c.get(9).is_some());
    assert!(c.get(2).is_none() && c.get(3).is_none());
    // Junk and degenerate folds.
    assert!(fold_cached(&[]).unwrap().elements().is_empty());
    let mut bad = BezPath::new();
    bad.move_to((f64::NAN, 0.0));
    bad.line_to((1.0, 1.0));
    let sq = figcraft_geom::Rect::new(0.0, 0.0, 1.0, 1.0).to_path(0.1);
    assert!(fold_cached(&[(sq, BoolOp::Union), (bad, BoolOp::Union)]).unwrap().elements().is_empty());
}

#[test]
fn one_sided_widths_paint_one_side() {
    // A line from (10, 100) to (190, 100), 20 wide, all of it on the right (+y, looking along it).
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let l = d.create(p, NodeKind::Line, "L", 10.0, 100.0, 180.0, 0.0).unwrap();
    let n = d.get_mut(l).unwrap();
    n.strokes = vec![Paint::solid(Color::BLACK)];
    n.stroke.weight = 20.0;
    n.stroke.width_profile = Some(WidthProfile { points: vec![WidthPoint::from_sides(0.0, 0.0, 2.0), WidthPoint::from_sides(1.0, 0.0, 2.0)] });
    let pix = render(&d, p);
    assert!(dark(&pix, 100, 110) && dark(&pix, 100, 118), "the right side, 20 deep");
    assert!(!dark(&pix, 100, 95), "nothing on the left");
    // The paint bounds reach the widest side.
    let b = d.world_render_bounds(l).unwrap();
    assert!(b.y1 >= 120.0 - 1e-6, "{b:?}");
}

#[test]
fn deep_boolean_nesting_stops_at_the_outline_depth_limit() {
    // Booleans directly inside booleans used to restart the depth count at every level.
    let mut d = Document::new("t");
    let mut parent = d.first_page().unwrap();
    let top = d.create(parent, NodeKind::BooleanOp { op: BoolOp::Union }, "B", 0.0, 0.0, 10.0, 10.0).unwrap();
    parent = top;
    for _ in 0..20_000 {
        parent = d.create(parent, NodeKind::BooleanOp { op: BoolOp::Union }, "B", 0.0, 0.0, 10.0, 10.0).unwrap();
    }
    d.create(parent, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    // Too deep: no stack overflow, and the unreachable square contributes nothing.
    assert!(outline(&d, top).area().abs() < 1e-9);
    // Within the limit, nested booleans still combine.
    let (mut d, _, b, _) = row_of_squares(BoolOp::Union, 1);
    let inner = d.create(b, NodeKind::BooleanOp { op: BoolOp::Union }, "B", 0.0, 0.0, 0.0, 0.0).unwrap();
    d.create(inner, NodeKind::Rectangle, "R", 40.0, 10.0, 20.0, 20.0).unwrap();
    assert!((outline(&d, b).area().abs() - 2.0 * 20.0 * 20.0).abs() < 0.5);
}

#[test]
fn stroke_only_boolean_operands_keep_their_holes() {
    use figcraft_doc::StrokeAlign;
    use figcraft_geom::Point;

    for kind in [NodeKind::Polygon { count: 6 }, NodeKind::Rectangle, NodeKind::Line] {
        for align in [StrokeAlign::Inside, StrokeAlign::Center, StrokeAlign::Outside] {
            let mut d = Document::new("stroke operands");
            let page = d.first_page().unwrap();
            let b = d.create(page, NodeKind::BooleanOp { op: BoolOp::Subtract }, "B", 0.0, 0.0, 100.0, 100.0).unwrap();
            d.get_mut(b).unwrap().fills = vec![Paint::solid(Color::BLACK)];
            // A nested group exercises the same path as a direct operand.
            let g = d.create(b, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
            let id = d.create(g, kind.clone(), "stroke", 10.0, 10.0, 80.0, 80.0).unwrap();
            let n = d.get_mut(id).unwrap();
            let mut hidden_fill = Paint::solid(Color::BLACK);
            hidden_fill.visible = false;
            n.fills = vec![hidden_fill];
            n.strokes = vec![Paint::solid(Color::BLACK)];
            n.stroke.weight = 6.0;
            n.stroke.align = align;
            let path = outline(&d, b);
            assert_eq!(path.winding(Point::new(50.0, 45.0)), 0, "{kind:?} {align:?}: empty interior");
            assert!(path.area().abs() > 100.0, "{kind:?} {align:?}: stroke survives");
            assert!(!dark(&render(&d, page), 50, 45));
            if !matches!(kind, NodeKind::Line) {
                // Changing only stroke weight invalidates the boolean result.
                let area = path.area().abs();
                d.get_mut(id).unwrap().stroke.weight = 3.0;
                assert!(outline(&d, b).area().abs() < area);
                d.get_mut(id).unwrap().fills[0].visible = true;
                assert_ne!(outline(&d, b).winding(Point::new(50.0, 45.0)), 0, "visible fills keep the interior");
            }
        }
    }
}

#[test]
fn boolean_operands_preserve_side_borders_and_container_children() {
    use figcraft_doc::StrokeAlign;
    use figcraft_geom::Point;
    for kind in [NodeKind::Rectangle, NodeKind::Frame] {
        let mut d = Document::new("borders");
        let page = d.first_page().unwrap();
        let b = d.create(page, NodeKind::BooleanOp { op: BoolOp::Union }, "B", 0.0, 0.0, 100.0, 100.0).unwrap();
        let id = d.create(b, kind.clone(), "operand", 0.0, 0.0, 100.0, 100.0).unwrap();
        let n = d.get_mut(id).unwrap();
        n.fills.clear();
        n.strokes = vec![Paint::solid(Color::BLACK)];
        n.stroke.weight = 8.0;
        n.stroke.align = StrokeAlign::Inside;
        n.stroke.sides = Some([8.0, 0.0, 0.0, 0.0]);
        if matches!(kind, NodeKind::Frame) {
            d.create(id, NodeKind::Rectangle, "child", 40.0, 40.0, 20.0, 20.0).unwrap();
        }
        let p = outline(&d, b);
        assert_ne!(p.winding(Point::new(50.0, 4.0)), 0, "top border");
        assert_eq!(p.winding(Point::new(98.0, 50.0)), 0, "no right border");
        assert_eq!(p.winding(Point::new(50.0, 98.0)), 0, "no bottom border");
        if matches!(kind, NodeKind::Frame) {
            assert_ne!(p.winding(Point::new(50.0, 50.0)), 0, "child retained");
        }
        d.get_mut(id).unwrap().stroke.sides = Some([0.0, 8.0, 0.0, 0.0]);
        let p = outline(&d, b);
        assert_eq!(p.winding(Point::new(50.0, 4.0)), 0, "side edit removes top");
        assert_ne!(p.winding(Point::new(98.0, 50.0)), 0, "side edit adds right");
        d.get_mut(id).unwrap().stroke.sides = None;
        assert_ne!(outline(&d, b).winding(Point::new(50.0, 4.0)), 0, "uniform container border");
    }
}

#[test]
fn stroke_only_text_operands_use_glyphs_instead_of_the_text_box() {
    use figcraft_doc::{StrokeAlign, TextData};
    let mut d = Document::new("text strokes");
    let page = d.first_page().unwrap();
    let b = d.create(page, NodeKind::BooleanOp { op: BoolOp::Union }, "B", 0.0, 0.0, 400.0, 100.0).unwrap();
    let mut text = TextData::new("Hi");
    text.style.font_size = 32.0;
    let id = d.create(b, NodeKind::Text(text), "text", 0.0, 0.0, 400.0, 100.0).unwrap();
    let n = d.get_mut(id).unwrap();
    n.fills.clear();
    n.strokes = vec![Paint::solid(Color::BLACK)];
    n.stroke.weight = 2.0;
    n.stroke.align = StrokeAlign::Center;
    let glyphs = outline(&d, id);
    let result = outline(&d, b);
    assert_eq!(result.elements().is_empty(), glyphs.elements().is_empty());
    if !glyphs.elements().is_empty() {
        let expected = glyphs.bounding_box().inflate(3.0, 3.0);
        let actual = result.bounding_box();
        assert!(actual.x0 >= expected.x0 && actual.x1 <= expected.x1);
        assert!(actual.y0 >= expected.y0 && actual.y1 <= expected.y1);
        assert!(actual.x1 < 200.0, "the unused text box must not contribute a border");
    }
    let NodeKind::Text(text) = &mut d.get_mut(id).unwrap().kind else { panic!() };
    text.characters = "   ".into();
    assert!(outline(&d, b).elements().is_empty(), "spaces have no stroked glyphs");
}

#[test]
fn rounded_side_borders_follow_their_corners_in_booleans() {
    use figcraft_doc::StrokeAlign;
    use figcraft_geom::Point;
    let mut d = Document::new("rounded borders");
    let page = d.first_page().unwrap();
    let b = d.create(page, NodeKind::BooleanOp { op: BoolOp::Union }, "B", 0.0, 0.0, 100.0, 100.0).unwrap();
    let id = d.create(b, NodeKind::Rectangle, "operand", 0.0, 0.0, 100.0, 100.0).unwrap();
    let n = d.get_mut(id).unwrap();
    n.fills.clear();
    n.strokes = vec![Paint::solid(Color::BLACK)];
    n.stroke.align = StrokeAlign::Inside;
    n.stroke.sides = Some([8.0; 4]);
    n.corners.radii = [30.0; 4];
    let p = outline(&d, b);
    assert_ne!(p.winding(Point::new(50.0, 4.0)), 0, "top border");
    assert_eq!(p.winding(Point::new(3.0, 3.0)), 0, "the rounded corner leaves the square's tip out");
    assert_eq!(p.winding(Point::new(50.0, 50.0)), 0, "hollow inside");
}
