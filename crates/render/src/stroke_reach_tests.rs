//! What a stroke paints stays within the bounds an export and a redraw go by: square dash ends
//! that stand out round a curve, brush shapes past half the weight and brush copies that stray
//! further than a width profile thins them included.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Brush, Document, Node, NodeKind, StrokeAlign, StrokeCap, StrokeJoin, WidthProfile};

use super::*;

/// How many pixels `kind` (at (200, 200), `size` square) with a 40-unit stroke styled by `style`
/// paints more than a pixel outside its export bounds, on a 600 × 600 view.
fn painted_outside(kind: &NodeKind, size: f64, align: StrokeAlign, miter_limit: f64, style: fn(&mut Node)) -> usize {
    let mut d = Document::new("reach");
    let p = d.first_page().unwrap();
    let n = d.create(p, kind.clone(), "S", 200.0, 200.0, size, size).unwrap();
    let node = d.get_mut(n).unwrap();
    node.fills.clear();
    node.corners.radii = [30.0; 4];
    node.strokes = vec![Paint::solid(Color::WHITE)];
    (node.stroke.weight, node.stroke.align, node.stroke.join, node.stroke.miter_limit) = (40.0, align, StrokeJoin::Miter, miter_limit);
    style(node);
    let b = crate::export_bounds(&d, n).unwrap();
    let mut pix = Pixmap::new(600, 600).unwrap();
    render_nodes_into(&d, &[n], &View { origin: (0.0, 0.0), scale: 1.0, width: 600, height: 600 }, &mut pix);
    let (xs, ys) = (b.x0 - 1.0..=b.x1 + 1.0, b.y0 - 1.0..=b.y1 + 1.0);
    let inside = |i: usize| xs.contains(&((i % 600) as f64 + 0.5)) && ys.contains(&((i / 600) as f64 + 0.5));
    pix.data().chunks(4).enumerate().filter(|(i, p)| p[3] > 0 && !inside(*i)).count()
}

#[test]
fn a_stroke_with_corners_paints_within_its_bounds() {
    type Style = fn(&mut Node);
    let strokes: [(&str, Style); 4] = [
        ("square dash ends", |n| (n.stroke.dashes, n.stroke.dash_cap) = (vec![30.0, 30.0], Some(StrokeCap::Square))),
        ("short square dashes", |n| (n.stroke.dashes, n.stroke.dash_cap) = (vec![1.0, 7.0], Some(StrokeCap::Square))),
        ("the ink brush", |n| n.stroke.brush = Some(Brush::new("ink"))),
        ("a thinned scatter brush", |n| {
            n.stroke.brush = Some(Brush { scatter: 1.0, ..Brush::new("dots") });
            n.stroke.width_profile = Some(WidthProfile::new([(0.0, 0.5), (1.0, 0.5)]));
        }),
    ];
    let circle = NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 };
    let shapes = [(&circle, 200.0), (&circle, 30.0), (&NodeKind::Rectangle, 200.0), (&NodeKind::Star { count: 5, inner_ratio: 0.4 }, 200.0)];
    for (kind, size) in shapes {
        for (name, style) in strokes {
            for align in [StrokeAlign::Center, StrokeAlign::Outside] {
                for miter_limit in [4.0, 1.0] {
                    let outside = painted_outside(kind, size, align, miter_limit, style);
                    assert_eq!(outside, 0, "{kind:?} {size}, {name}, {align:?}, mitre limit {miter_limit}");
                }
            }
        }
    }
}
