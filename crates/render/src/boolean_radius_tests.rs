//! A boolean group's corner radius: it rounds the result the canvas draws, and keys a cache of its
//! own so the fold underneath is not redone.

use crate::boolean::boolean_steps;
use crate::boolean_cache_tests::{dark, outline, render};
use figcraft_color::{Color, Paint};
use figcraft_doc::{BoolOp, Document, NodeId, NodeKind};
use figcraft_geom::kurbo::Shape;

/// The L of the measurement: a union of 200 by 100 and 100 by 200 rectangles, on a page of its own.
fn l_group() -> (Document, NodeId, NodeId, NodeId) {
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let g = d.create(p, NodeKind::BooleanOp { op: BoolOp::Union }, "G", 0.0, 0.0, 0.0, 0.0).unwrap();
    d.get_mut(g).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    d.create(g, NodeKind::Rectangle, "A", 0.0, 0.0, 200.0, 100.0).unwrap();
    let b = d.create(g, NodeKind::Rectangle, "B", 150.0, 0.0, 100.0, 200.0).unwrap();
    d.fit_to_children(g).unwrap();
    (d, p, g, b)
}

fn set_radius(d: &mut Document, g: NodeId, r: f64) {
    d.get_mut(g).unwrap().corners.radii = [r; 4];
}

fn area(d: &Document, g: NodeId) -> f64 {
    outline(d, g).area().abs()
}

#[test]
fn a_boolean_groups_radius_rounds_what_the_canvas_draws() {
    let (mut d, p, g, _) = l_group();
    let pix = render(&d, p);
    assert!(dark(&pix, 1, 1) && !dark(&pix, 148, 102));
    set_radius(&mut d, g, 20.0);
    let pix = render(&d, p);
    assert!(!dark(&pix, 1, 1), "the corner is cut off");
    assert!(dark(&pix, 148, 102), "the concave corner is filled in");
    assert!(dark(&pix, 100, 50));
    assert!((area(&d, g) - 34656.6).abs() < 1.0, "{}", area(&d, g));
}

#[test]
fn the_radius_keys_the_result_and_the_fold_is_not_redone() {
    let (mut d, _, g, b) = l_group();
    let sharp = outline(&d, g);
    set_radius(&mut d, g, 20.0);
    let _ = outline(&d, g);
    let before = boolean_steps();
    set_radius(&mut d, g, 40.0);
    // Four net corners of 343.36 each, plus the cubic arcs' own slight excess over a circle.
    assert!((area(&d, g) - 33626.6).abs() < 2.0, "{}", area(&d, g));
    assert_eq!(boolean_steps(), before, "a new radius reuses the fold");
    set_radius(&mut d, g, 0.0);
    assert_eq!(outline(&d, g), sharp, "radius 0 draws the fold itself");
    set_radius(&mut d, g, 20.0);
    let rounded = area(&d, g);
    d.get_mut(b).unwrap().transform = figcraft_geom::Affine::translate((10.0, 0.0)) * d.node(b).unwrap().transform;
    assert!((area(&d, g) - rounded).abs() > 1.0, "moving an operand does not draw a stale rounded path");
}
