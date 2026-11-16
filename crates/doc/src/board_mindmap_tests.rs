//! Mind map layout (`board_mindmap`): columns 88 apart, siblings 48 apart, stacks centred on their
//! parents (#2226).

use figcraft_geom::{Point, Rect};

use crate::NodeId;
use crate::board_mindmap::{MIND_GAP_X, MIND_GAP_Y, MIND_MAX_NODES, MindBox, mind_layout};

/// The first node, always at the origin, 200 x 36.
fn root() -> MindBox {
    MindBox { id: NodeId(1), rect: Rect::new(0.0, 0.0, 200.0, 36.0), parent: None, left: false }
}

/// A node `w` x `h` whose current top is `y` (only its middle, which orders siblings, counts).
fn node(id: u64, parent: usize, (w, h): (f64, f64), y: f64) -> MindBox {
    MindBox { id: NodeId(id), rect: Rect::new(0.0, y, w, y + h), parent: Some(parent), left: false }
}

fn at(placed: &[(NodeId, Point)], id: u64) -> (f64, f64) {
    let p = placed.iter().find(|(n, _)| *n == NodeId(id)).map(|(_, p)| *p).unwrap_or_else(|| panic!("node {id} not placed in {placed:?}"));
    (p.x, p.y)
}

/// The root and three children (100, 60 and 80 wide) whose middles are in that order; `middle` is
/// the second child's size.
fn three(middle: (f64, f64)) -> Vec<MindBox> {
    vec![root(), node(2, 0, (100.0, 24.0), 0.0), node(3, 0, middle, 22.0 - middle.1 / 2.0), node(4, 0, (80.0, 24.0), 20.0)]
}

/// The root, two children (100 and 60 wide) under it, two more under the first and one under the second.
fn six() -> Vec<MindBox> {
    vec![
        root(),
        node(2, 0, (100.0, 24.0), 0.0),
        node(3, 0, (60.0, 24.0), 10.0),
        node(4, 1, (40.0, 24.0), 0.0),
        node(5, 1, (40.0, 24.0), 10.0),
        node(6, 2, (40.0, 24.0), 0.0),
    ]
}

#[test]
fn three_children_stand_88_right_of_the_root_72_apart_centred_on_it() {
    assert_eq!((MIND_GAP_X, MIND_GAP_Y), (88.0, 48.0));
    let placed = mind_layout(&three((60.0, 24.0)));
    assert_eq!([at(&placed, 2), at(&placed, 3), at(&placed, 4)], [(288.0, -66.0), (288.0, 6.0), (288.0, 78.0)]);
    assert!(placed.iter().all(|(n, _)| *n != NodeId(1)), "the first node stays where it is");
}

#[test]
fn a_taller_node_pushes_its_neighbours_half_its_growth_each_way() {
    let placed = mind_layout(&three((60.0, 72.0)));
    assert_eq!([at(&placed, 2), at(&placed, 3), at(&placed, 4)], [(288.0, -90.0), (288.0, -18.0), (288.0, 102.0)]);
}

#[test]
fn grandchildren_share_a_column_past_the_widest_sibling_and_their_parents_make_room() {
    let nodes = six();
    let placed = mind_layout(&nodes);
    // 476 = 288 + 100 + 88, although B is only 60 wide.
    let got: Vec<(f64, f64)> = (2..=6).map(|i| at(&placed, i)).collect();
    assert_eq!(got, [(288.0, -30.0), (288.0, 78.0), (476.0, -66.0), (476.0, 6.0), (476.0, 78.0)]);
}

#[test]
fn left_children_mirror_the_right_ones() {
    let mut l = node(2, 0, (80.0, 24.0), 0.0);
    l.left = true;
    let placed = mind_layout(&[root(), l, node(3, 1, (50.0, 24.0), 0.0)]);
    assert_eq!((at(&placed, 2), at(&placed, 3)), ((-168.0, 6.0), (-306.0, 6.0)));
}

#[test]
fn the_layout_is_a_fixed_point_ties_go_by_id_and_junk_is_skipped() {
    let nodes = six();
    let placed = mind_layout(&nodes);
    let again: Vec<MindBox> = nodes
        .iter()
        .map(|b| {
            let p = placed.iter().find(|(n, _)| *n == b.id).map_or(b.rect.origin(), |(_, p)| *p);
            MindBox { rect: Rect::from_origin_size(p, b.rect.size()), ..*b }
        })
        .collect();
    assert_eq!(mind_layout(&again), placed, "a laid out map is left as it is");
    // The same height: the lower id goes first.
    let tied = mind_layout(&[root(), node(9, 0, (10.0, 24.0), 5.0), node(7, 0, (10.0, 24.0), 5.0)]);
    assert!(at(&tied, 7).1 < at(&tied, 9).1);
    // Junk: nothing placed, nothing panics.
    assert!(mind_layout(&[]).is_empty());
    let mut nan = three((60.0, 24.0));
    nan[2].rect = Rect::new(f64::NAN, 0.0, 1.0, 1.0);
    assert!(mind_layout(&nan).is_empty());
    let mut orphan = three((60.0, 24.0));
    orphan.push(node(8, 9, (10.0, 24.0), 0.0));
    assert!(!mind_layout(&orphan).iter().any(|(n, _)| *n == NodeId(8)), "a parent that doesn't come first");
    let mut huge = vec![root()];
    huge.extend((0..MIND_MAX_NODES as u64).map(|i| node(i + 2, 0, (10.0, 24.0), i as f64)));
    assert!(mind_layout(&huge).is_empty());
}
