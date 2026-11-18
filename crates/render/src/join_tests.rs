//! Outside strokes on rectangles honour the stroke join at the outer corners; inside strokes keep
//! the shape's square corners whatever the join.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeKind, StrokeAlign, StrokeJoin};
use figcraft_geom::kurbo::{BezPath, Point, Shape};

fn winding_at(align: StrokeAlign, join: StrokeJoin, at: Point) -> i32 {
    let mut d = Document::new("join");
    let p = d.first_page().unwrap();
    let id = d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 60.0, 40.0).unwrap();
    let n = d.get_mut(id).unwrap();
    n.strokes = vec![Paint::solid(Color::BLACK)];
    n.stroke.weight = 10.0;
    n.stroke.align = align;
    n.stroke.join = join;
    let rect: BezPath = figcraft_geom::kurbo::Rect::new(0.0, 0.0, 60.0, 40.0).to_path(0.1);
    let n = d.get(id).unwrap();
    crate::draw::stroke_area(n, &rect, true).winding(at)
}

fn covered(join: StrokeJoin) -> i32 {
    winding_at(StrokeAlign::Outside, join, Point::new(-9.5, -9.5))
}

#[test]
fn inside_round_and_bevel_joins_keep_square_corner() {
    for join in [StrokeJoin::Round, StrokeJoin::Bevel, StrokeJoin::Miter] {
        assert_ne!(winding_at(StrokeAlign::Inside, join, Point::new(0.5, 0.5)), 0, "{join:?}");
    }
}

#[test]
fn outside_round_join_rounds_outer_corner() {
    assert_eq!(covered(StrokeJoin::Round), 0);
}

#[test]
fn outside_bevel_join_cuts_outer_corner() {
    assert_eq!(covered(StrokeJoin::Bevel), 0);
}

#[test]
fn outside_miter_join_keeps_square_corner() {
    assert_ne!(covered(StrokeJoin::Miter), 0);
}

/// An ellipse drawn the other way round (a sweep of −2π) is whole: its inside stroke is the same
/// ring of two outlines as a forward one's.
#[test]
fn a_whole_ellipse_drawn_backwards_strokes_as_a_ring() {
    let ring = |sweep: f64| {
        let mut d = Document::new("ring");
        let p = d.first_page().unwrap();
        let id = d.create(p, NodeKind::Ellipse { start: 0.0, sweep, inner: 0.0 }, "E", 0.0, 0.0, 60.0, 40.0).unwrap();
        let n = d.get_mut(id).unwrap();
        n.strokes = vec![Paint::solid(Color::BLACK)];
        (n.stroke.weight, n.stroke.align) = (6.0, StrokeAlign::Inside);
        let outline: BezPath = figcraft_geom::kurbo::Ellipse::from_rect(figcraft_geom::kurbo::Rect::new(0.0, 0.0, 60.0, 40.0)).to_path(0.1);
        (*crate::draw::stroke_area(d.get(id).unwrap(), &outline, true)).clone()
    };
    let forwards = ring(std::f64::consts::TAU);
    let moves = forwards.elements().iter().filter(|e| matches!(e, figcraft_geom::kurbo::PathEl::MoveTo(_))).count();
    assert_eq!(moves, 2, "two outlines: {forwards:?}");
    assert_eq!(ring(-std::f64::consts::TAU), forwards);
}
