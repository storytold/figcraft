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
