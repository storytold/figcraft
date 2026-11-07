//! Fill rules on vectors: `vector.fillRule` sets the rule a filled area fills by and
//! `vector.reverseLoop` turns one of its loops round — which is how a hole is made and unmade.
//! The numbers come from the two rules' definitions: non-zero fills where the winding count is not
//! zero (so a hole is a loop running against its outer loop), even-odd where it is odd (so the
//! direction of a loop doesn't matter).

use figcraft_doc::{NodeId, NodeKind, Region, Segment, VectorNetwork, Vertex, WindingRule};
use figcraft_geom::{BezPath, Point};
use serde_json::{Value, json};

use crate::Session;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn net(s: &Session, id: NodeId) -> VectorNetwork {
    match &s.doc().unwrap().node(id).unwrap().kind {
        NodeKind::Vector { network } => network.clone(),
        other => panic!("not a vector: {other:?}"),
    }
}

/// A 100 × 100 square with a 25…75 hole, both loops in one area: the outer one clockwise, the inner
/// one the same way round (so non-zero fills the hole too until it is turned round).
fn ring() -> VectorNetwork {
    let mut path = BezPath::new();
    for (x0, y0, x1, y1) in [(0.0, 0.0, 100.0, 100.0), (25.0, 25.0, 75.0, 75.0)] {
        path.move_to((x0, y0));
        path.line_to((x1, y0));
        path.line_to((x1, y1));
        path.line_to((x0, y1));
        path.close_path();
    }
    let mut net = VectorNetwork::from_path(&path);
    net.regions[0].winding = WindingRule::NonZero;
    net
}

/// A selected vector layer holding `network`.
fn vector_layer(s: &mut Session, network: VectorNetwork) -> NodeId {
    let id = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let id = NodeId(id);
    ex(s, "vector.edit.begin", json!({"id": id}));
    ex(s, "vector.edit.end", json!({}));
    {
        let doc = s.doc_mut().unwrap();
        match &mut doc.get_mut(id).unwrap().kind {
            NodeKind::Vector { network: net } => *net = network,
            other => panic!("not a vector: {other:?}"),
        }
    }
    ex(s, "select.set", json!({"ids": [id]}));
    id
}

/// A selected vector layer holding `ring()`.
fn ringed(s: &mut Session) -> NodeId {
    vector_layer(s, ring())
}

/// Two 50 × 100 squares side by side share the vertical edge between them: one segment listed by
/// both areas' single loops. A segment shared between areas is the case `vector.reverseLoop` must
/// not flip, or a neighbour's fill or stroke would turn round with it.
fn two_squares() -> VectorNetwork {
    // 0:A(0,0) 1:B(50,0) 2:C(100,0) 3:D(100,100) 4:E(50,100) 5:F(0,100)
    // s0 A→B (left bottom), s1 B→E (the shared vertical edge), s2 E→F (left top),
    // s3 F→A (left left), s4 B→C (right bottom), s5 C→D (right right), s6 D→E (right top);
    // the right area's left edge is s1 walked the other way, so the squares share one segment.
    // region 0: left square A→B→E→F→A = [s0, s1, s2, s3].
    // region 1: right square B→C→D→E→B = [s4, s5, s6, s1] (s1 walked backwards).
    VectorNetwork {
        vertices: vec![
            Vertex::new(0.0, 0.0),
            Vertex::new(50.0, 0.0),
            Vertex::new(100.0, 0.0),
            Vertex::new(100.0, 100.0),
            Vertex::new(50.0, 100.0),
            Vertex::new(0.0, 100.0),
        ],
        segments: vec![
            Segment::line(0, 1),
            Segment::line(1, 4),
            Segment::line(4, 5),
            Segment::line(5, 0),
            Segment::line(1, 2),
            Segment::line(2, 3),
            Segment::line(3, 4),
        ],
        regions: vec![
            Region { loops: vec![vec![0, 1, 2, 3]], winding: WindingRule::NonZero, ..Default::default() },
            Region { loops: vec![vec![4, 5, 6, 1]], winding: WindingRule::NonZero, ..Default::default() },
        ],
    }
}

/// A selected vector layer holding [`two_squares`].
fn squared(s: &mut Session) -> NodeId {
    vector_layer(s, two_squares())
}

/// Two areas of two segments each over the same line: one curve above it and one below. Both areas
/// list the line's segment — the first area first (`[line, above]`), the second area last
/// (`[below, line]`) — and a two-segment ring reads the same whichever way its list is turned, so
/// these only turn round when a segment is stored the other way.
fn two_lenses() -> VectorNetwork {
    // 0:A(0,0) 1:B(100,0)
    // s0 A→B the straight line both areas use; s1 B→A curving above it; s2 A→B curving below it.
    VectorNetwork {
        vertices: vec![Vertex::new(0.0, 0.0), Vertex::new(100.0, 0.0)],
        segments: vec![
            Segment::line(0, 1),
            Segment { a: 1, b: 0, ta: (0.0, -50.0), tb: (0.0, -50.0) },
            Segment { a: 0, b: 1, ta: (0.0, 50.0), tb: (0.0, 50.0) },
        ],
        regions: vec![
            Region { loops: vec![vec![0, 1]], winding: WindingRule::NonZero, ..Default::default() },
            Region { loops: vec![vec![2, 0]], winding: WindingRule::NonZero, ..Default::default() },
        ],
    }
}

/// A selected vector layer holding [`two_lenses`].
fn lensed(s: &mut Session) -> NodeId {
    vector_layer(s, two_lenses())
}

/// Two areas whose only loop is the same closed segment (a curve the path closes with a straight
/// line): a one-segment loop's direction is its segment's stored ends, and both areas read them.
fn one_shared_segment() -> VectorNetwork {
    VectorNetwork {
        vertices: vec![Vertex::new(0.0, 0.0), Vertex::new(100.0, 0.0)],
        segments: vec![Segment { a: 0, b: 1, ta: (0.0, -60.0), tb: (0.0, -60.0) }],
        regions: vec![
            Region { loops: vec![vec![0]], winding: WindingRule::NonZero, ..Default::default() },
            Region { loops: vec![vec![0]], winding: WindingRule::NonZero, ..Default::default() },
        ],
    }
}

/// The point in the middle of the hole, and one on the square's left band.
const HOLE: Point = Point::new(50.0, 50.0);
const BAND: Point = Point::new(10.0, 50.0);

#[test]
fn a_loop_running_with_its_outer_loop_fills_the_hole_until_it_is_turned_round() {
    let mut s = Session::new();
    let id = ringed(&mut s);
    let n = net(&s, id);
    assert_eq!(n.regions.len(), 1);
    assert_eq!(n.regions[0].loops.len(), 2, "an outer loop and the hole's");
    assert!(n.region_contains(0, BAND), "the band fills");
    assert!(n.region_contains(0, HOLE), "the hole fills while its loop runs the same way");
    // Turning the inner loop round makes it a hole under non-zero.
    let r = ex(&mut s, "vector.reverseLoop", json!({"id": id, "region": 0, "loop": 1}));
    assert_eq!(r["reversed"], json!(true));
    let n = net(&s, id);
    assert!(n.region_contains(0, BAND), "the band still fills");
    assert!(!n.region_contains(0, HOLE), "the hole is empty");
    assert_eq!(n.region_winding(0, HOLE), Some(0), "its loops cancel out there");
    // Undo turns it back.
    s.undo().unwrap();
    assert!(net(&s, id).region_contains(0, HOLE), "one undo step restores the loop");
}

#[test]
fn the_rule_switches_every_area_or_only_the_one_asked_for() {
    let mut s = Session::new();
    let id = ringed(&mut s);
    // Even-odd leaves the hole empty whatever way its loop runs.
    let r = ex(&mut s, "vector.fillRule", json!({"id": id, "rule": "evenOdd"}));
    assert_eq!((r["regions"].clone(), r["rule"].clone()), (json!(1), json!("evenOdd")));
    let n = net(&s, id);
    assert_eq!(n.regions[0].winding, WindingRule::EvenOdd);
    assert!(!n.region_contains(0, HOLE), "even-odd counts the hole's loop as a second crossing");
    // And back.
    ex(&mut s, "vector.fillRule", json!({"id": id, "rule": "nonZero"}));
    assert!(net(&s, id).region_contains(0, HOLE), "non-zero fills it again");
    // One area of several, by index.
    let n = net(&s, id);
    let mut two = n.clone();
    two.regions.push(n.regions[0].clone());
    {
        let doc = s.doc_mut().unwrap();
        match &mut doc.get_mut(id).unwrap().kind {
            NodeKind::Vector { network } => *network = two,
            other => panic!("not a vector: {other:?}"),
        }
    }
    ex(&mut s, "vector.fillRule", json!({"id": id, "region": 1, "rule": "evenOdd"}));
    let n = net(&s, id);
    assert_eq!(n.regions[0].winding, WindingRule::NonZero, "the other area keeps its rule");
    assert_eq!(n.regions[1].winding, WindingRule::EvenOdd);
    // Every area at once.
    ex(&mut s, "vector.fillRule", json!({"id": id, "rule": "evenOdd"}));
    assert!(net(&s, id).regions.iter().all(|r| r.winding == WindingRule::EvenOdd));
}

#[test]
fn junk_parameters_are_refused_and_change_nothing() {
    let mut s = Session::new();
    let id = ringed(&mut s);
    let before = net(&s, id);
    for p in [
        json!({"id": id, "rule": "nope"}),
        json!({"id": id}),
        json!({"id": id, "rule": 7}),
        json!({"id": id, "rule": "evenOdd", "region": -1}),
        json!({"id": id, "rule": "evenOdd", "region": 1.5}),
        json!({"id": id, "rule": "evenOdd", "region": "x"}),
        json!({"id": id, "rule": "evenOdd", "region": 99}),
    ] {
        assert!(s.execute("vector.fillRule", &p).is_err(), "{p} is refused");
    }
    for p in [
        json!({"id": id}),
        json!({"id": id, "region": 0}),
        json!({"id": id, "region": 0, "loop": 99}),
        json!({"id": id, "region": 99, "loop": 0}),
        json!({"id": id, "region": -2, "loop": 0}),
    ] {
        assert!(s.execute("vector.reverseLoop", &p).is_err(), "{p} is refused");
    }
    assert_eq!(net(&s, id), before, "nothing changed");
    // A rectangle that was never a vector is refused too.
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    assert!(s.execute("vector.fillRule", &json!({"id": rect, "rule": "evenOdd"})).is_err());
    assert!(s.execute("vector.reverseLoop", &json!({"id": rect, "region": 0, "loop": 0})).is_err());
}

#[test]
fn reversing_a_loop_shared_with_a_neighbour_leaves_its_segment_and_fill_alone() {
    let mut s = Session::new();
    let id = squared(&mut s);
    let n = net(&s, id);
    assert_eq!(n.regions.len(), 2, "two areas, side by side");
    // Both areas fill under non-zero: a point in each.
    assert!(n.region_contains(0, Point::new(25.0, 50.0)), "the left area fills");
    assert!(n.region_contains(1, Point::new(75.0, 50.0)), "the right area fills");
    // The vertical edge between them is one segment listed by both areas' loops.
    const SHARED: usize = 1;
    assert!(n.regions[0].loops[0].contains(&SHARED) && n.regions[1].loops[0].contains(&SHARED), "one segment is shared");
    let before = n.segments[SHARED];
    let left_winding = n.region_winding(0, Point::new(25.0, 50.0)).unwrap();
    assert_ne!(left_winding, 0, "the left area winds once");
    // Reverse the left area's only loop.
    let r = ex(&mut s, "vector.reverseLoop", json!({"id": id, "region": 0, "loop": 0}));
    assert_eq!(r["reversed"], json!(true));
    let n = net(&s, id);
    // The shared segment's stored ends are untouched: reversing the loop must not flip a neighbour's
    // segment (reversing the list already turns this loop's traversal of it the other way).
    assert_eq!(n.segments[SHARED], before, "the shared segment is left as it was");
    // The neighbour's fill is unchanged: it still fills its interior.
    assert!(n.region_contains(1, Point::new(75.0, 50.0)), "the neighbour still fills");
    // The reversed loop really turned round: its winding at a left-area point flipped sign.
    assert_eq!(n.region_winding(0, Point::new(25.0, 50.0)), Some(-left_winding), "the left loop reversed");
    // Undo restores the shared segment and the loop.
    s.undo().unwrap();
    assert_eq!(net(&s, id).segments[SHARED], before, "undo restores the shared segment too");
}

#[test]
fn a_two_segment_loop_turns_round_with_its_shared_segment_listed_first_or_last() {
    let mut s = Session::new();
    let id = lensed(&mut s);
    const SHARED: usize = 0;
    const ABOVE: Point = Point::new(50.0, -25.0);
    const BELOW: Point = Point::new(50.0, 25.0);
    let n = net(&s, id);
    assert_eq!(n.regions.len(), 2, "two areas over the same line");
    assert_eq!((n.regions[0].loops[0][0], n.regions[0].loops[0].len()), (SHARED, 2), "the shared segment is listed first");
    assert_eq!((n.regions[1].loops[0][1], n.regions[1].loops[0].len()), (SHARED, 2), "and last");
    assert!(n.region_contains(0, ABOVE), "the area above the line fills");
    assert!(n.region_contains(1, BELOW), "the area below it fills");
    let (above, below) = (n.region_winding(0, ABOVE).unwrap(), n.region_winding(1, BELOW).unwrap());
    assert_ne!(above, 0, "the area above winds once");
    assert_ne!(below, 0, "the area below winds once");
    let shared = n.segments[SHARED];
    // Turn the area that lists the shared segment first round: a two-segment ring reads the same
    // either way, so this only works by storing one of its segments the other way.
    assert_eq!(ex(&mut s, "vector.reverseLoop", json!({"id": id, "region": 0, "loop": 0}))["reversed"], json!(true));
    let n = net(&s, id);
    assert_eq!(n.segments[SHARED], shared, "the shared segment keeps its ends");
    assert!(n.region_contains(0, ABOVE), "the area still fills");
    assert_eq!(n.region_winding(0, ABOVE), Some(-above), "it really turned round");
    assert_eq!(n.region_winding(1, BELOW), Some(below), "the neighbour did not");
    // And the area that lists it last.
    ex(&mut s, "vector.reverseLoop", json!({"id": id, "region": 1, "loop": 0}));
    let n = net(&s, id);
    assert_eq!(n.segments[SHARED], shared, "still untouched");
    assert_eq!(n.region_winding(1, BELOW), Some(-below), "the second area really turned round");
    assert_eq!(n.region_winding(0, ABOVE), Some(-above), "the first stays as it was");
    // Undo turns each back.
    s.undo().unwrap();
    assert_eq!(net(&s, id).region_winding(1, BELOW), Some(below), "undo turns the second back");
    s.undo().unwrap();
    assert_eq!(net(&s, id).region_winding(0, ABOVE), Some(above), "and the first");
}

#[test]
fn a_one_segment_loop_shared_with_another_is_refused_instead_of_claiming_a_turn() {
    let mut s = Session::new();
    let id = vector_layer(&mut s, one_shared_segment());
    let before = net(&s, id);
    let err = s.execute("vector.reverseLoop", &json!({"id": id, "region": 0, "loop": 0})).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("shares its segments"), "the error says why: {message}");
    assert_eq!(net(&s, id), before, "nothing changed");
}
