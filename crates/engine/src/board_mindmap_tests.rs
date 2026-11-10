//! Mind maps (#2226): `board.mindMap` and `board.mindMapAdd` make nodes and branches, and the
//! normalize pass lays a touched map out again from its first node after every edit.

use figcraft_color::{Color, Paint};
use figcraft_doc::{BoardItem, Connector, Document, END_GAP, End, LetterSpacing, LineHeight, NodeId, NodeKind, Routing, Side, StrokeCap};
use figcraft_geom::{Point, Rect};
use serde_json::{Value, json};

use crate::Session;
use crate::board::text_of;
use crate::board_tests::{board, connector, drawn, id, near, node};
use crate::collab_live_tests::Room;
use crate::tools::{PointerEvent, PointerKind, ToolId};

fn bx(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().world_bounds(id).unwrap()
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// `board.mindMap` at (100, 200): the first node, its three children and their branches.
fn map(s: &mut Session) -> (NodeId, [NodeId; 3], [NodeId; 3]) {
    let out = s.execute("board.mindMap", &json!({"x": 100, "y": 200})).unwrap();
    let kids: Vec<Value> = out["children"].as_array().unwrap().clone();
    let (c, k): (Vec<NodeId>, Vec<NodeId>) = kids.iter().map(|v| (id(v, "id"), id(v, "connector"))).unzip();
    (id(&out, "id"), [c[0], c[1], c[2]], [k[0], k[1], k[2]])
}

fn text_data(s: &Session, n: NodeId) -> figcraft_doc::TextData {
    let t = text_of(s.doc().unwrap(), n).unwrap();
    match node(s, t).kind {
        NodeKind::Text(td) => td,
        other => panic!("not text: {other:?}"),
    }
}

fn is_mind_node(s: &Session, n: NodeId) -> bool {
    node(s, n).board == Some(BoardItem::MindNode)
}

/// The nodes hanging off `parent`, top to bottom, in `doc`.
fn kids(doc: &Document, parent: NodeId) -> Vec<NodeId> {
    let mut v: Vec<(f64, NodeId)> = doc
        .nodes()
        .filter_map(|n| match &n.board {
            Some(BoardItem::Connector(Connector { from: End::Node { node, .. }, to: End::Node { node: to, .. }, branch: true, .. }))
                if *node == parent =>
            {
                doc.world_bounds(*to).map(|b| (b.center().y, *to))
            }
            _ => None,
        })
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|p| p.1).collect()
}

fn places(s: &Session, ids: &[NodeId]) -> Vec<Rect> {
    ids.iter().map(|i| bx(s, *i)).collect()
}

fn branches(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().nodes().filter(|n| matches!(&n.board, Some(BoardItem::Connector(c)) if c.branch)).map(|n| n.id).collect()
}

/// Starter child `i` (`k`, on branch `line`) of the first node: its text, its place and its branch.
fn check_child(s: &Session, root: NodeId, (k, line): (NodeId, NodeId), i: usize) {
    let r = bx(s, root);
    let td = text_data(s, k);
    assert_eq!(td.characters, ["Idea 1", "Idea 2", "Idea 3"][i]);
    assert_eq!((td.style.font_size, td.style.font_style.as_str()), (16.0, "Medium"));
    assert_eq!((td.style.line_height, td.style.letter_spacing), (LineHeight::Percent(150.0), LetterSpacing::Percent(-1.1)));
    let b = bx(s, k);
    assert!(same(b.x0, r.x1 + 88.0), "child {i}: 88 right of the root");
    assert_eq!(b.height(), 24.0);
    let t = bx(s, text_of(s.doc().unwrap(), k).unwrap());
    assert!(same(b.width(), t.width()), "child {i}: the node is its text, no padding");
    assert!(is_mind_node(s, k));
    let want = Connector {
        from: End::Node { node: root, side: Side::Right, at: None },
        to: End::Node { node: k, side: Side::Left, at: None },
        routing: Routing::Curved,
        path: None,
        branch: true,
    };
    assert_eq!(connector(s, line), want);
    let l = node(s, line);
    assert_eq!((l.stroke.cap_start, l.stroke.cap_end), (StrokeCap::Round, StrokeCap::Round));
    assert_eq!((l.stroke.weight, l.strokes), (4.0, vec![Paint::solid(Color::from_rgb8_const(0x75, 0x75, 0x75))]));
    let pts = drawn(s, line);
    assert!(near(pts[0], Point::new(r.x1 + END_GAP, r.center().y)), "{pts:?}");
    assert!(near(*pts.last().unwrap(), Point::new(b.x0 - END_GAP, b.center().y)), "{pts:?}");
}

#[test]
fn a_mind_map_is_a_first_node_with_three_children_on_branches() {
    let mut s = board();
    let before = s.doc().unwrap().ids().count();
    let depth = s.undo_depth().0;
    let (root, kids, lines) = map(&mut s);
    assert_eq!(node(&s, root).board, Some(BoardItem::MindNode));
    assert!(node(&s, root).is_board_atom());
    let r = bx(&s, root);
    assert_eq!((r.x0, r.y0), (100.0, 200.0));
    let td = text_data(&s, root);
    assert_eq!((td.characters.as_str(), td.style.font_size, td.style.font_style.as_str()), ("Topic", 24.0, "Bold"));
    assert_eq!((td.style.line_height, td.style.letter_spacing), (LineHeight::Percent(150.0), LetterSpacing::Percent(-1.9)));
    assert_eq!(r.height(), 36.0, "one line at 150%");
    let text = text_of(s.doc().unwrap(), root).unwrap();
    assert_eq!(node(&s, text).fills, vec![Paint::solid(Color::from_rgb8_const(0x1E, 0x1E, 0x1E))]);
    for (i, pair) in kids.into_iter().zip(lines).enumerate() {
        check_child(&s, root, pair, i);
    }
    let h = bx(&s, kids[0]).height();
    assert!(same(bx(&s, kids[1]).center().y, r.center().y));
    assert!(same(bx(&s, kids[0]).center().y, r.center().y - h - 48.0));
    assert!(same(bx(&s, kids[2]).center().y, r.center().y + h + 48.0));
    assert_eq!(s.selection(), vec![root]);
    assert_eq!(s.undo_depth().0, depth + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().ids().count(), before);
}

#[test]
fn a_new_child_goes_below_its_siblings_and_the_stack_stays_centred() {
    let mut s = board();
    let (root, kids3, _) = map(&mut s);
    let h = bx(&s, kids3[0]).height();
    let was = places(&s, &kids3);
    s.ui_requests.clear();
    let out = s.execute("board.mindMapAdd", &json!({"id": root, "text": "Four", "edit": true})).unwrap();
    let new = id(&out, "id");
    assert!(is_mind_node(&s, new));
    assert_eq!(text_data(&s, new).style.font_size, 16.0);
    let order = kids(s.doc().unwrap(), root);
    assert_eq!(order, [kids3[0], kids3[1], kids3[2], new]);
    let cy: Vec<f64> = order.iter().map(|n| bx(&s, *n).center().y).collect();
    assert!(cy.windows(2).all(|w| same(w[1] - w[0], h + 48.0)), "{cy:?}");
    assert!(same((cy[0] + cy[3]) / 2.0, bx(&s, root).center().y));
    assert!(same(bx(&s, kids3[0]).y0, was[0].y0 - (h + 48.0) / 2.0), "the first one moved up 36");
    assert_eq!(s.selection(), vec![new]);
    assert!(s.ui_requests.iter().any(|q| q.kind == "board.editText" && q.data["id"] == json!(new)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(places(&s, &kids3), was, "the three are back exactly");
    // Without an id the selected first node is the one.
    s.execute("select.set", &json!({"ids": [root]})).unwrap();
    s.execute("board.mindMapAdd", &json!({})).unwrap();
    assert_eq!(kids(s.doc().unwrap(), root).len(), 4);
}

#[test]
fn a_childs_child_starts_the_next_column_and_a_sibling_goes_right_after() {
    let mut s = board();
    let (root, [c0, c1, c2], _) = map(&mut s);
    let h = bx(&s, c0).height();
    let g1 = id(&s.execute("board.mindMapAdd", &json!({"id": c0})).unwrap(), "id");
    let g2 = id(&s.execute("board.mindMapAdd", &json!({"id": c0})).unwrap(), "id");
    let column = [c0, c1, c2].iter().map(|n| bx(&s, *n).x1).fold(f64::MIN, f64::max) + 88.0;
    assert!(same(bx(&s, g1).x0, column) && same(bx(&s, g2).x0, column), "past the widest of the three");
    assert!(same(bx(&s, g2).center().y - bx(&s, g1).center().y, h + 48.0));
    assert!(same((bx(&s, g1).center().y + bx(&s, g2).center().y) / 2.0, bx(&s, c0).center().y));
    for g in [g1, g2] {
        let line = s.doc().unwrap().nodes().find(|n| matches!(&n.board, Some(BoardItem::Connector(c)) if c.to.node() == Some(g))).unwrap().id;
        assert!(matches!(connector(&s, line).from, End::Node { node, side: Side::Right, .. } if node == c0));
    }
    assert!(same(bx(&s, c1).center().y - bx(&s, c0).center().y, (2.0 * h + 48.0) / 2.0 + 48.0 + h / 2.0), "108 below");
    let new = id(&s.execute("board.mindMapAdd", &json!({"id": c1, "relation": "sibling"})).unwrap(), "id");
    let line = s.doc().unwrap().nodes().find(|n| matches!(&n.board, Some(BoardItem::Connector(c)) if c.to.node() == Some(new))).unwrap().id;
    assert!(matches!(connector(&s, line).from, End::Node { node, .. } if node == root));
    assert_eq!(kids(s.doc().unwrap(), root), [c0, c1, new, c2]);
}

#[test]
fn more_lines_push_the_siblings_apart() {
    let mut s = board();
    let (root, [c0, c1, c2], _) = map(&mut s);
    let (h, was) = (bx(&s, c1).height(), places(&s, &[c0, c1, c2]));
    let t = text_of(s.doc().unwrap(), c1).unwrap();
    s.execute("node.set", &json!({"id": t, "props": {"characters": "one\ntwo\nthree"}})).unwrap();
    let h3 = bx(&s, c1).height();
    assert_eq!((h, h3), (24.0, 72.0));
    assert!(same(bx(&s, c1).center().y, bx(&s, root).center().y));
    for k in [c0, c2] {
        assert!(same((bx(&s, k).center().y - bx(&s, c1).center().y).abs(), (h + h3) / 2.0 + 48.0));
    }
    assert!(same(bx(&s, c0).y0, was[0].y0 - (h3 - h) / 2.0), "24 up");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(places(&s, &[c0, c1, c2]), was);
}

#[test]
fn the_first_node_carries_the_map_and_a_child_moved_by_hand_goes_back_or_swaps() {
    let mut s = board();
    let (root, kids3, lines) = map(&mut s);
    let was = places(&s, &kids3);
    s.execute("transform.move", &json!({"ids": [root], "dx": 50, "dy": 30})).unwrap();
    for (k, w) in kids3.iter().zip(&was) {
        let b = bx(&s, *k);
        assert!(same(b.x0, w.x0 + 50.0) && same(b.y0, w.y0 + 30.0), "{b:?} from {w:?}");
    }
    for l in lines {
        assert!(near(drawn(&s, l)[0], Point::new(bx(&s, root).x1 + END_GAP, bx(&s, root).center().y)));
    }
    let now = places(&s, &kids3);
    s.execute("transform.move", &json!({"ids": [kids3[2]], "dx": 300, "dy": 5})).unwrap();
    assert_eq!(places(&s, &kids3), now, "it goes back");
    s.execute("transform.move", &json!({"ids": [kids3[0]], "dx": 0, "dy": 100})).unwrap();
    assert_eq!(kids(s.doc().unwrap(), root), [kids3[1], kids3[0], kids3[2]], "past the middle one's middle: they swap");
    assert!(same(bx(&s, kids3[1]).center().y, bx(&s, root).center().y - 72.0));
}

#[test]
fn a_deleted_node_leaves_its_children_in_place_and_takes_its_branches() {
    let mut s = board();
    let (root, [c0, c1, c2], _) = map(&mut s);
    let h = bx(&s, c0).height();
    let g = id(&s.execute("board.mindMapAdd", &json!({"id": c1})).unwrap(), "id");
    let at = bx(&s, g);
    let (undo, _) = s.undo_depth();
    s.execute("edit.delete", &json!({"ids": [c1]})).unwrap();
    let left = branches(&s);
    assert_eq!(left.len(), 2, "the root's two others");
    for l in &left {
        assert!(!matches!(connector(&s, *l).to, End::Node { node, .. } if node == c1 || node == g), "none to the gone node or its child");
    }
    assert_eq!(bx(&s, g), at, "the child stays where it was");
    assert!(is_mind_node(&s, g));
    let r = bx(&s, root).center().y;
    assert!(same(bx(&s, c0).center().y, r - (h + 48.0) / 2.0) && same(bx(&s, c2).center().y, r + (h + 48.0) / 2.0));
    let line = s.doc().unwrap().nodes().find(|n| matches!(&n.board, Some(BoardItem::Connector(c)) if c.to.node() == Some(c2))).unwrap().id;
    let spot = bx(&s, c2);
    s.execute("edit.delete", &json!({"ids": [line]})).unwrap();
    assert_eq!(bx(&s, c2), spot, "a detached node stays");
    assert!(same(bx(&s, c0).center().y, r));
    assert_eq!(branches(&s).len(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.undo_depth().0, undo);
    assert_eq!(branches(&s).len(), 4);
    assert!(s.doc().unwrap().contains(c1));
}

#[test]
fn two_people_adding_at_once_end_with_one_layout() {
    let mut s = board();
    let (root, [c0, ..], _) = map(&mut s);
    let h = bx(&s, c0).height();
    let mut room = Room::new(s.doc().unwrap().clone(), 2, 7);
    for i in 0..2 {
        room.s(i).execute("board.mindMapAdd", &json!({"id": root})).unwrap();
    }
    room.settle();
    room.check();
    let doc = room.server.document();
    let order = kids(doc, root);
    assert_eq!(order.len(), 5);
    let centres = |doc: &Document, order: &[NodeId]| order.iter().map(|n| doc.world_bounds(*n).unwrap().center().y).collect::<Vec<f64>>();
    let cy = centres(doc, &order);
    assert!(cy.windows(2).all(|w| w[1] > w[0] && same(w[1] - w[0], h + 48.0)), "{cy:?}");
    assert!(same((cy[0] + cy[4]) / 2.0, doc.world_bounds(root).unwrap().center().y));
    // One makes a child three lines tall while the other adds one.
    let t = text_of(doc, order[1]).unwrap();
    room.s(0).execute("node.set", &json!({"id": t, "props": {"characters": "one\ntwo\nthree"}})).unwrap();
    room.s(1).execute("board.mindMapAdd", &json!({"id": root})).unwrap();
    room.settle();
    room.check();
    let doc = room.server.document();
    let order = kids(doc, root);
    assert_eq!(order.len(), 6);
    let (boxes, mut gap_ok) = (order.iter().map(|n| doc.world_bounds(*n).unwrap()).collect::<Vec<Rect>>(), true);
    for w in boxes.windows(2) {
        gap_ok &= same(w[1].y0 - w[0].y1, 48.0);
    }
    assert!(gap_ok, "48 between every pair: {boxes:?}");
    assert!(boxes.iter().any(|b| same(b.height(), 3.0 * h)));
    let (top, bottom) = (boxes[0].y0, boxes[5].y1);
    assert!(same((top + bottom) / 2.0, doc.world_bounds(root).unwrap().center().y));
}

#[test]
fn refusals_saved_files_and_a_second_pass_changes_nothing() {
    let mut d = Session::new();
    d.execute("file.new", &json!({})).unwrap();
    for cmd in ["board.mindMap", "board.mindMapAdd"] {
        assert!(matches!(d.execute(cmd, &json!({})), Err(crate::EngineError::Disabled(..))), "{cmd} on a design file");
    }
    let mut s = board();
    let (root, ..) = map(&mut s);
    let sticky = id(&s.execute("board.sticky", &json!({"x": 900, "y": 0})).unwrap(), "id");
    for p in [json!({"id": sticky}), json!({"id": root, "relation": "cousin"}), json!({"id": root, "relation": "sibling"})] {
        assert!(s.execute("board.mindMapAdd", &p).is_err(), "{p}");
    }
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("board.mindMapAdd", &json!({})).is_err(), "nothing selected");
    let twenty_one: Vec<String> = (0..21).map(|i| format!("n{i}")).collect();
    assert!(s.execute("board.mindMap", &json!({"children": twenty_one})).is_err());
    assert!(s.execute("board.mindMap", &json!({"children": "x"})).is_err());
    let doc = s.doc().unwrap();
    let (back, _) = figcraft_format::from_json(&figcraft_format::to_json(doc).unwrap().to_string()).unwrap();
    assert_eq!(back.nodes().filter(|n| n.board == Some(BoardItem::MindNode)).count(), 4);
    assert_eq!(back.nodes().filter(|n| matches!(&n.board, Some(BoardItem::Connector(c)) if c.branch)).count(), 3);
    // An ordinary connector writes no `branch`; a file without it reads `false`.
    let mut plain = board();
    let a = id(&plain.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap(), "id");
    let b = id(&plain.execute("board.sticky", &json!({"x": 400, "y": 0})).unwrap(), "id");
    plain.execute("board.connector", &json!({"from": a, "to": b})).unwrap();
    assert!(!figcraft_format::to_json(plain.doc().unwrap()).unwrap().to_string().contains("\"branch\""));
    let item: BoardItem = serde_json::from_value(json!({"type": "connector", "from": {"x": 0, "y": 0}, "to": {"x": 1, "y": 1}})).unwrap();
    assert!(matches!(item, BoardItem::Connector(c) if !c.branch));
    // Every layer touched again: a laid out map is not an edit.
    let mut again = doc.clone();
    again.mark_dirty(doc.ids().collect::<Vec<_>>());
    crate::normalize(&mut again, doc);
    assert!(again == *doc, "a second pass changes nothing");
}

/// Every layer touched again on a copy of `s`'s document: a map in place is left as it is.
fn second_pass_is_a_no_op(s: &Session) {
    let doc = s.doc().unwrap();
    let mut again = doc.clone();
    again.mark_dirty(doc.ids().collect::<Vec<_>>());
    crate::normalize(&mut again, doc);
    assert!(again == *doc, "a second pass changes nothing");
}

#[test]
fn a_map_around_its_own_container_stays_put() {
    let mut s = board();
    let sec = id(&s.execute("node.create", &json!({"type": "section", "x": 0, "y": 0, "width": 800, "height": 600})).unwrap(), "id");
    let out = s.execute("board.mindMap", &json!({"parent": sec, "x": 40, "y": 40})).unwrap();
    let line = id(&out["children"][1], "connector");
    s.execute("board.reattach", &json!({"id": line, "end": "to", "target": sec})).unwrap();
    let was = bx(&s, sec);
    second_pass_is_a_no_op(&s);
    s.execute("node.set", &json!({"id": id(&out, "id"), "props": {"name": "again"}})).unwrap();
    assert_eq!(bx(&s, sec), was, "the section stays where it is");
    second_pass_is_a_no_op(&s);
}

#[test]
fn a_branch_moved_by_its_start_leaves_no_gap_and_odd_structures_settle() {
    let mut s = board();
    let (root, [c0, c1, c2], lines) = map(&mut s);
    let (h, at) = (bx(&s, c1).height(), bx(&s, c1));
    s.execute("board.reattach", &json!({"id": lines[1], "end": "from", "target": {"x": 900, "y": 900}})).unwrap();
    assert_eq!(branches(&s).len(), 2, "one branch fewer");
    assert_eq!(bx(&s, c1), at, "that child stays where it was");
    let r = bx(&s, root).center().y;
    assert!(same(bx(&s, c0).center().y, r - (h + 48.0) / 2.0), "{:?}", bx(&s, c0));
    assert!(same(bx(&s, c2).center().y, r + (h + 48.0) / 2.0), "{:?}", bx(&s, c2));
    second_pass_is_a_no_op(&s);

    // Two branches arrive at one node: the lower connector id stays its parent.
    let mut s = board();
    let (root, [c0, c1, _], lines) = map(&mut s);
    let g = id(&s.execute("board.mindMapAdd", &json!({"id": c0})).unwrap(), "id");
    let gl = branches(&s).into_iter().find(|b| !lines.contains(b)).unwrap();
    s.execute("board.reattach", &json!({"id": gl, "end": "to", "target": c1})).unwrap();
    assert_eq!(kids(s.doc().unwrap(), root).len(), 3);
    assert!(same(bx(&s, c1).x0, bx(&s, root).x1 + 88.0), "c1 stays in the root's column");
    assert!(is_mind_node(&s, g));
    second_pass_is_a_no_op(&s);

    // A cycle: nothing in it moves, and nothing changes after.
    let mut s = board();
    let (_, [c0, ..], lines) = map(&mut s);
    let g = id(&s.execute("board.mindMapAdd", &json!({"id": c0})).unwrap(), "id");
    let gg = id(&s.execute("board.mindMapAdd", &json!({"id": g})).unwrap(), "id");
    s.execute("board.reattach", &json!({"id": lines[0], "end": "from", "target": gg})).unwrap();
    let ring = places(&s, &[c0, g, gg]);
    second_pass_is_a_no_op(&s);
    assert_eq!(places(&s, &[c0, g, gg]), ring);
}

#[test]
fn the_caps_refuse_what_is_too_far_and_cut_what_is_too_long() {
    let mut s = board();
    assert!(s.execute("board.mindMap", &json!({"x": 2.0e9})).is_err());
    assert!(s.execute("board.mindMap", &json!({"y": -2.0e9})).is_err());
    let long = "x".repeat(600);
    let out = s.execute("board.mindMap", &json!({"text": long, "children": [long]})).unwrap();
    assert_eq!(text_data(&s, id(&out, "id")).characters.chars().count(), 500);
    assert_eq!(text_data(&s, id(&out["children"][0], "id")).characters.chars().count(), 500);
}

fn next_sticky(s: &mut Session, p: Value) -> NodeId {
    id(&s.execute("board.nextSticky", &p).unwrap(), "id")
}

fn arrives_from(s: &Session, n: NodeId) -> Option<NodeId> {
    s.doc().unwrap().nodes().find_map(|l| match &l.board {
        Some(BoardItem::Connector(c)) if c.branch && c.to.node() == Some(n) => c.from.node(),
        _ => None,
    })
}

#[test]
fn the_next_sticky_key_gives_a_mind_map_node_its_next_sibling() {
    let mut s = board();
    let (root, [c0, c1, c2], _) = map(&mut s);
    let depth = s.undo_depth().0;
    let out = s.execute("board.nextSticky", &json!({"id": c1})).unwrap();
    let a = id(&out, "id");
    assert!(is_mind_node(&s, a));
    assert!(connector(&s, id(&out, "connector")).branch);
    assert_eq!(arrives_from(&s, a), Some(root));
    assert_eq!(kids(s.doc().unwrap(), root), [c0, c1, a, c2]);
    assert_eq!(s.undo_depth().0, depth + 1);
    // Unequal heights and a subtree.
    s.execute("board.mindMapAdd", &json!({"id": c1})).unwrap();
    s.execute("board.mindMapAdd", &json!({"id": c1})).unwrap();
    s.execute("node.set", &json!({"id": text_of(s.doc().unwrap(), c1).unwrap(), "props": {"characters": "one\ntwo\nthree"}})).unwrap();
    let b = next_sticky(&mut s, json!({"id": c1}));
    assert_eq!(kids(s.doc().unwrap(), root), [c0, c1, b, a, c2]);
    assert!(same(bx(&s, b).x0, bx(&s, c1).x0));
    let mids: Vec<f64> = [c0, c1, b, a, c2].iter().map(|n| bx(&s, *n).center().y).collect();
    assert!(mids.windows(2).all(|w| w[0] < w[1]), "{mids:?}");
    assert!(bx(&s, b).y0 >= bx(&s, c1).y1 + 48.0 - 1e-6);
    // The last sibling, and the selection as the default.
    s.execute("select.set", &json!({"ids": [c2]})).unwrap();
    let last = next_sticky(&mut s, json!({}));
    assert_eq!(kids(s.doc().unwrap(), root).last(), Some(&last));
    // While typing.
    crate::board::edit_text(&mut s, c0).unwrap();
    s.ui_requests.clear();
    let d = next_sticky(&mut s, json!({"edit": true}));
    assert!(s.ds().unwrap().text_edit.is_none());
    assert!(s.ui_requests.iter().any(|q| q.kind == "board.editText" && q.data["id"] == json!(d)));
    let order = kids(s.doc().unwrap(), root);
    assert_eq!(order.iter().position(|n| *n == d), order.iter().position(|n| *n == c0).map(|i| i + 1));
    assert_eq!(s.selection(), vec![d]);
    // The first node has no sibling: refused, and an open edit stays open.
    let count = s.doc().unwrap().ids().count();
    crate::board::edit_text(&mut s, root).unwrap();
    let err = s.execute("board.nextSticky", &json!({"id": root})).unwrap_err().to_string();
    assert!(err.contains("first node"), "{err}");
    assert!(s.ds().unwrap().text_edit.is_some());
    assert_eq!(s.doc().unwrap().ids().count(), count);
}

#[test]
fn tab_walks_a_nodes_siblings_in_the_maps_order() {
    let mut s = board();
    let (_, [c0, c1, c2], _) = map(&mut s);
    s.execute("select.set", &json!({"ids": [c0]})).unwrap();
    for want in [c1, c2, c0] {
        s.execute("select.next", &json!({})).unwrap();
        assert_eq!(s.selection(), vec![want]);
    }
    s.execute("select.previous", &json!({})).unwrap();
    assert_eq!(s.selection(), vec![c2]);
    s.execute("transform.move", &json!({"ids": [c0], "dx": 0, "dy": 100})).unwrap();
    s.execute("select.set", &json!({"ids": [c1]})).unwrap();
    s.execute("select.next", &json!({})).unwrap();
    assert_eq!(s.selection(), vec![c0]);
    // An only child stays.
    let g = id(&s.execute("board.mindMapAdd", &json!({"id": c1})).unwrap(), "id");
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    s.execute("select.next", &json!({})).unwrap();
    assert_eq!(s.selection(), vec![g]);
    // Layers keep the layer order.
    let mut t = board();
    let a = id(&t.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap(), "id");
    let b = id(&t.execute("board.sticky", &json!({"x": 400, "y": 0})).unwrap(), "id");
    t.execute("select.set", &json!({"ids": [b]})).unwrap();
    t.execute("select.next", &json!({})).unwrap();
    assert_eq!(t.selection(), vec![a]);
    let mut u = board();
    let (root, _, _) = map(&mut u);
    u.execute("select.set", &json!({"ids": [root]})).unwrap();
    u.execute("select.next", &json!({})).unwrap();
    assert_eq!(u.selection().len(), 1);
}

#[test]
fn the_mind_map_tool_starts_the_first_nodes_text_at_the_click() {
    let mut s = board();
    s.execute("tool.mindMap", &json!({})).unwrap();
    assert_eq!(s.tool.tool, ToolId::MindMap);
    s.pointer(&PointerEvent::at(PointerKind::Down, 300.0, 200.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 300.0, 200.0, 1.0)).unwrap();
    let nodes: Vec<NodeId> = s.doc().unwrap().nodes().filter(|n| n.board == Some(BoardItem::MindNode)).map(|n| n.id).collect();
    assert_eq!((nodes.len(), branches(&s).len()), (4, 3));
    let root = *nodes.iter().find(|n| arrives_from(&s, **n).is_none()).unwrap();
    assert_eq!(bx(&s, root).x0, 300.0);
    assert!(same(bx(&s, root).center().y, 200.0));
    let te = s.ds().unwrap().text_edit.clone().expect("typing in the first node");
    assert_eq!(crate::board::owner_of_text(s.doc().unwrap(), te.id), Some(root));
    assert_eq!((te.anchor.min(te.caret), te.anchor.max(te.caret)), (0, 5));
    assert_eq!(s.tool.tool, ToolId::Move);
    crate::textedit::insert(&mut s, "Plan").unwrap();
    crate::textedit::end(&mut s).unwrap();
    assert_eq!(text_data(&s, root).characters, "Plan");
    for k in nodes.iter().filter(|n| **n != root) {
        assert!(same(bx(&s, *k).x0, bx(&s, root).x1 + 88.0));
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text_data(&s, root).characters, "Topic");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().nodes().all(|n| n.board != Some(BoardItem::MindNode)));
    // Tools kept on.
    s.prefs.keep_tool = true;
    s.execute("tool.mindMap", &json!({})).unwrap();
    for y in [200.0, 900.0] {
        s.pointer(&PointerEvent::at(PointerKind::Down, 300.0, y, 1.0)).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Up, 300.0, y, 1.0)).unwrap();
    }
    assert_eq!(s.doc().unwrap().nodes().filter(|n| n.board == Some(BoardItem::MindNode)).count(), 8);
    assert_eq!(s.tool.tool, ToolId::MindMap);
    // Refusals and names.
    let mut d = Session::new();
    for (cmd, p) in [("tool.mindMap", json!({})), ("tool.set", json!({"tool": "mindMap"}))] {
        assert!(matches!(d.execute(cmd, &p), Err(crate::EngineError::Disabled(..))), "{cmd} on a design file");
    }
    assert_eq!(ToolId::from_id("mindMap"), Some(ToolId::MindMap));
    assert!(ToolId::MindMap.on_boards());
}

#[test]
fn a_board_colour_on_a_mind_map_node_is_its_text_colour() {
    let mut s = board();
    let (_, [c0, ..], lines) = map(&mut s);
    let ink = Paint::solid(Color::from_rgb8_const(0x1E, 0x1E, 0x1E));
    s.execute("board.setColor", &json!({"ids": [c0, lines[0]], "color": "blue"})).unwrap();
    let text = text_of(s.doc().unwrap(), c0).unwrap();
    assert_eq!(node(&s, text).fills, node(&s, lines[0]).strokes);
    assert!(node(&s, c0).fills.is_empty(), "no box");
    s.execute("board.setColor", &json!({"ids": [c0], "color": "#FF0000"})).unwrap();
    assert_eq!(node(&s, text).fills, vec![Paint::solid(Color::from_rgb8_const(0xFF, 0, 0))]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, text).fills, vec![ink.clone()]);
    let st = id(&s.execute("board.sticky", &json!({"x": 900, "y": 0})).unwrap(), "id");
    s.execute("board.setColor", &json!({"ids": [c0, st], "color": "green"})).unwrap();
    assert_ne!(node(&s, text).fills, vec![ink]);
    assert_eq!(node(&s, st).fills.len(), 1);
}
