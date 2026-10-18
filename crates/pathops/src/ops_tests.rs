//! Boolean groups whose operands carry their own operations: the fold bottom to top, its
//! agreement with a uniform operation, and single steps.

use figcraft_doc::BoolOp;
use figcraft_geom::Rect;
use figcraft_geom::kurbo::{BezPath, Shape};

use crate::{boolean, boolean_ops, boolean_step, normalized};

fn sq(x: f64, y: f64, s: f64) -> BezPath {
    Rect::new(x, y, x + s, y + s).to_path(0.1)
}

fn area(p: &BezPath) -> f64 {
    p.area().abs()
}

#[test]
fn each_operand_combines_with_the_result_below_it() {
    let (a, b, c) = (sq(0.0, 0.0, 10.0), sq(5.0, 0.0, 10.0), sq(0.0, 0.0, 15.0));
    // (A ∪ B) ∩ C: the union spans x 0..15, y 0..10 (150), all inside C.
    let r = boolean_ops([(&a, BoolOp::Union), (&b, BoolOp::Union), (&c, BoolOp::Intersect)]).unwrap();
    assert!((area(&r) - 150.0).abs() < 0.5, "{}", area(&r));
    // (A ∪ B) − a 5 × 5 square in the middle.
    let hole = sq(5.0, 2.5, 5.0);
    let r = boolean_ops([(&a, BoolOp::Union), (&b, BoolOp::Union), (&hole, BoolOp::Subtract)]).unwrap();
    assert!((area_eo(&r) - 125.0).abs() < 0.5, "{}", area_eo(&r));
    // The bottom operand's own operation is unused.
    let r2 = boolean_ops([(&a, BoolOp::Exclude), (&b, BoolOp::Union), (&hole, BoolOp::Subtract)]).unwrap();
    assert!((area_eo(&r2) - 125.0).abs() < 0.5);
}

/// Even-odd area: contours nested in another count negative (holes).
fn area_eo(p: &BezPath) -> f64 {
    let mut total = 0.0;
    let subs: Vec<BezPath> = split(p);
    for (i, s) in subs.iter().enumerate() {
        let inside = subs.iter().enumerate().filter(|(j, o)| *j != i && o.bounding_box().contains_rect(s.bounding_box())).count();
        total += if inside % 2 == 0 { area(s) } else { -area(s) };
    }
    total
}

fn split(p: &BezPath) -> Vec<BezPath> {
    use figcraft_geom::kurbo::PathEl;
    let mut out: Vec<BezPath> = Vec::new();
    for el in p.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            out.push(BezPath::new());
        }
        if let Some(last) = out.last_mut() {
            last.push(*el);
        }
    }
    out
}

#[test]
fn a_uniform_fold_matches_the_group_operation() {
    let (a, b, c) = (sq(0.0, 0.0, 10.0), sq(5.0, 5.0, 10.0), sq(8.0, 0.0, 4.0));
    for op in [BoolOp::Union, BoolOp::Subtract, BoolOp::Intersect, BoolOp::Exclude] {
        let all = boolean(&[a.clone(), b.clone(), c.clone()], op).unwrap();
        let folded = boolean_ops([(&a, op), (&b, op), (&c, op)]).unwrap();
        assert!((area_eo(&all) - area_eo(&folded)).abs() < 1e-6, "{op:?}");
    }
    // Subtract removes every later operand from the first.
    let s = boolean(&[a.clone(), b.clone(), c.clone()], BoolOp::Subtract).unwrap();
    assert!((area_eo(&s) - (100.0 - 25.0 - 8.0)).abs() < 0.5, "{}", area_eo(&s));
}

#[test]
fn single_steps_and_edge_cases() {
    let (a, b) = (sq(0.0, 0.0, 10.0), sq(5.0, 5.0, 10.0));
    assert!((area(&boolean_step(&a, &b, BoolOp::Intersect).unwrap()) - 25.0).abs() < 0.5);
    assert!((area(&boolean_step(&a, &b, BoolOp::Subtract).unwrap()) - 75.0).abs() < 0.5);
    // Nothing, and a lone operand (simplified).
    assert!(boolean_ops(std::iter::empty()).unwrap().elements().is_empty());
    assert!((area(&boolean_ops([(&a, BoolOp::Subtract)]).unwrap()) - 100.0).abs() < 1e-6);
    let mut bad = BezPath::new();
    bad.move_to((f64::NAN, 0.0));
    bad.line_to((1.0, 1.0));
    assert!(boolean_ops([(&a, BoolOp::Union), (&bad, BoolOp::Intersect)]).is_err());
}

/// Signed area (shoelace) of one subpath. Lines only, which is all a rectangle uses, so this is
/// exact and does not depend on what [`Shape::area`] does with several contours.
fn signed_area(p: &BezPath) -> f64 {
    use figcraft_geom::kurbo::PathEl;
    let (mut total, mut start, mut prev) = (0.0, (0.0, 0.0), (0.0, 0.0));
    for el in p.elements() {
        match el {
            PathEl::MoveTo(p) => {
                start = (p.x, p.y);
                prev = start;
            }
            PathEl::LineTo(p) => {
                total += prev.0 * p.y - p.x * prev.1;
                prev = (p.x, p.y);
            }
            PathEl::ClosePath => {
                total += prev.0 * start.1 - start.0 * prev.1;
                prev = start;
            }
            _ => {}
        }
    }
    total / 2.0
}

/// `sq`, twice, wound the same way: only an even-odd read leaves the middle out.
fn ring_same_winding() -> BezPath {
    let mut p = sq(0.0, 0.0, 100.0);
    p.extend(sq(25.0, 25.0, 50.0).elements().iter().copied());
    p
}

#[test]
fn normalising_reads_the_path_with_the_rule_it_was_drawn_with() {
    let ring = ring_same_winding();
    let subs = split(&ring);
    assert_eq!(subs.len(), 2);
    assert!(signed_area(&subs[0]) * signed_area(&subs[1]) > 0.0, "the two squares wind the same way");
    // Even-odd, this is a ring; non-zero, the middle is filled.
    assert!((area_eo(&ring) - 7_500.0).abs() < 1.0, "{}", area_eo(&ring));

    // Read as even-odd, the hole survives — and the contours come out opposed, so the result also
    // paints correctly under the non-zero rule a font's `glyf` table uses.
    let eo = normalized(&ring, true).unwrap();
    let esubs = split(&eo);
    assert_eq!(esubs.len(), 2);
    assert!(signed_area(&esubs[0]) * signed_area(&esubs[1]) < 0.0, "the hole now winds against its outline");
    assert!((area_eo(&eo) - 7_500.0).abs() < 1.0, "{}", area_eo(&eo));

    // Read as non-zero, the hole was never a hole: one contour, the full square.
    let nz = normalized(&ring, false).unwrap();
    assert_eq!(split(&nz).len(), 1);
    assert!((area_eo(&nz) - 10_000.0).abs() < 1.0, "{}", area_eo(&nz));
}

#[test]
fn a_path_that_already_winds_non_zero_is_left_alone() {
    // The same ring, but the inner square is wound against the outer, as a non-zero path would be.
    let mut nz_ring = sq(0.0, 0.0, 100.0);
    nz_ring.extend(sq(25.0, 25.0, 50.0).reverse_subpaths().elements().iter().copied());

    for even_odd in [false, true] {
        let n = normalized(&nz_ring, even_odd).unwrap();
        let subs = split(&n);
        assert_eq!(subs.len(), 2, "even_odd={even_odd}");
        assert!(signed_area(&subs[0]) * signed_area(&subs[1]) < 0.0, "even_odd={even_odd}");
        assert!((area_eo(&n) - 7_500.0).abs() < 1.0, "even_odd={even_odd}: {}", area_eo(&n));
    }
}
