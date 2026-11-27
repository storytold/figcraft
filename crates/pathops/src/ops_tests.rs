//! Boolean groups whose operands carry their own operations: the fold bottom to top, its
//! agreement with a uniform operation, and single steps.

use figcraft_doc::BoolOp;
use figcraft_geom::kurbo::{BezPath, Shape};
use figcraft_geom::{Point, Rect};

use crate::{boolean, boolean_ops, boolean_step, simplify_with_rule};

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

#[test]
fn fill_rule_normalization_keeps_even_odd_holes_and_more_than_4096_contours() {
    let mut ring = sq(0.0, 0.0, 20.0);
    ring.extend(sq(5.0, 5.0, 10.0).elements().iter().copied());
    let ring = simplify_with_rule(&ring, true).unwrap();
    assert!(ring.winding(Point::new(2.0, 2.0)) != 0);
    assert_eq!(ring.winding(Point::new(10.0, 10.0)), 0, "same-direction inner contour remains a hole");

    let mut many = BezPath::new();
    for i in 0..4097 {
        many.extend(sq(i as f64 * 2.0, 0.0, 1.0).elements().iter().copied());
    }
    let all = simplify_with_rule(&many, false).unwrap();
    assert!(all.winding(Point::new(8192.5, 0.5)) != 0, "last contour survives beyond the old 4096-face cap");
    assert!((area(&all) - 4097.0).abs() < 1e-6);
}

/// Two near-touching curved shapes, reduced from linesweeper's own regression input (`sasha_0`,
/// MIT OR Apache-2.0) for the sweep that never ended in 0.4: it looped allocating until the
/// system ran out of memory, so Save froze drawing the thumbnail of a file with such a boolean
/// group (#2394). Run on a thread so a hang fails here instead of stalling the test run.
#[test]
fn a_near_degenerate_boolean_finishes() {
    let a = BezPath::from_svg(
        "M6025,9840.5 C5968.463992389773,10053.842780234541 5933.1526880433003,10032.968601506755 5908.0786848870821,9992.7335731863113 \
         C5922.5320746617581,10015.926221894515 5948.1042351958995,10013.643209438689 5948.6368267411644,10013.571880444044 Z",
    )
    .unwrap();
    let b = BezPath::from_svg(
        "M5919.7428657446435,9896.7806765477981 C6009.9213151129161,9929.2664268136868 5908.0786848870839,9992.7335731863132 5908.0786848870839,9992.7335731863132 \
         C5922.5320813616336,10015.926232645474 5948.1042554148153,10013.643206799046 5948.6368267411754,10013.571880444044 \
         C5946.7576348925104,10013.823557352434 5941.1787512170895,10015.181060952815 5932.3878200776162,10016.112588163749 Z",
    )
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(boolean_step(&a, &b, BoolOp::Union)));
    let r = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the sweep should finish");
    assert!(r.is_ok_and(|p| !p.elements().is_empty()));
}
