//! Copy, cut and paste in vector edit mode (#1875), measured in Figma: Copy takes the selected
//! segments (a lone point: the segments touching it) and a paste outside is a new vector in place;
//! Cut removes them, splitting the path in the same layer; Paste while editing adds the clipboard's
//! segments or a copied layer's outline to the vector's network in place, selected.

use serde_json::{Value, json};

use crate::{EngineError, Session, params};
use figcraft_doc::{NodeId, NodeKind, VectorNetwork};

fn ex(s: &mut Session, c: &str, p: Value) -> Value {
    s.execute(c, &p).unwrap_or_else(|e| panic!("{c}: {e}"))
}

fn id_of(v: &Value) -> NodeId {
    params::id_of(&v["id"]).unwrap()
}

/// Open path A-B-C-D (three segments) with a look of its own, in edit mode.
fn path(s: &mut Session) -> NodeId {
    let r = ex(s, "vector.create", json!({"points": [[100, 100], [200, 150], [300, 100], [400, 200]], "closed": false}));
    let id = id_of(&r);
    ex(s, "node.set", json!({"id": id, "props": {"strokeWeight": 7, "opacity": 0.5}}));
    ex(s, "vector.edit.begin", json!({"id": id}));
    id
}

fn net(s: &Session, id: NodeId) -> VectorNetwork {
    crate::vectoredit::network(s.doc().unwrap(), id).unwrap().clone()
}

fn layers(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().node(s.page().unwrap()).unwrap().children.clone()
}

/// The network's points on the page, rounded, in vertex order.
fn page_points(s: &Session, id: NodeId) -> Vec<(i64, i64)> {
    let w = s.doc().unwrap().world_transform(id);
    net(s, id).vertices.iter().map(|v| w * v.point()).map(|p| (p.x.round() as i64, p.y.round() as i64)).collect()
}

/// The number of separate paths a network is made of.
fn subpaths(n: &VectorNetwork) -> usize {
    let mut group: Vec<usize> = (0..n.vertices.len()).collect();
    fn root(g: &[usize], mut i: usize) -> usize {
        while g[i] != i {
            i = g[i];
        }
        i
    }
    for sg in &n.segments {
        let (a, b) = (root(&group, sg.a), root(&group, sg.b));
        group[a] = b;
    }
    (0..n.vertices.len()).filter(|i| root(&group, *i) == *i && n.segments.iter().any(|sg| root(&group, sg.a) == *i)).count()
}

fn select(s: &mut Session, vertices: &[usize], segments: &[usize]) {
    ex(s, "vector.edit.select", json!({"vertices": vertices, "segments": segments}));
}

fn leave(s: &mut Session) {
    ex(s, "vector.edit.end", json!({}));
    assert!(s.ds().unwrap().vector_edit.is_none());
}

fn world_rect(s: &Session, id: NodeId) -> (f64, f64, f64, f64) {
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    (b.x0, b.y0, b.width(), b.height())
}

#[test]
fn a_copied_segment_pastes_outside_as_a_new_vector_in_place() {
    let mut s = Session::new();
    let id = path(&mut s);
    select(&mut s, &[], &[1]);
    let r = ex(&mut s, "edit.copy", json!({}));
    assert_eq!(r["segments"], 1);
    assert_eq!(net(&s, id).segments.len(), 3, "Copy changes nothing");
    leave(&mut s);
    ex(&mut s, "edit.paste", json!({}));
    let all = layers(&s);
    assert_eq!(all.len(), 2);
    let new = all[1];
    let n = net(&s, new);
    assert_eq!((n.vertices.len(), n.segments.len()), (2, 1));
    assert_eq!(page_points(&s, new), vec![(200, 150), (300, 100)], "at the segment's own page position");
    assert_eq!(world_rect(&s, new), (200.0, 100.0, 100.0, 50.0));
    // The layer's look comes along (the width profile aside).
    let node = s.doc().unwrap().node(new).unwrap();
    assert_eq!((node.stroke.weight, node.opacity), (7.0, 0.5));
    assert_eq!(s.selection(), vec![new]);
}

#[test]
fn a_copied_point_takes_the_segments_touching_it() {
    let mut s = Session::new();
    path(&mut s);
    select(&mut s, &[1], &[]);
    ex(&mut s, "edit.copy", json!({}));
    leave(&mut s);
    ex(&mut s, "edit.paste", json!({}));
    let new = *layers(&s).last().unwrap();
    let n = net(&s, new);
    assert_eq!((n.vertices.len(), n.segments.len()), (3, 2), "A-B and B-C");
    assert_eq!(page_points(&s, new), vec![(100, 100), (200, 150), (300, 100)]);
}

#[test]
fn nothing_selected_still_copies_the_whole_layer() {
    let mut s = Session::new();
    path(&mut s);
    ex(&mut s, "edit.copy", json!({}));
    leave(&mut s);
    ex(&mut s, "edit.paste", json!({}));
    let new = *layers(&s).last().unwrap();
    assert_eq!(net(&s, new).segments.len(), 3);
}

#[test]
fn cut_removes_the_segment_in_the_same_layer_and_stays_in_edit_mode() {
    let mut s = Session::new();
    let id = path(&mut s);
    select(&mut s, &[], &[1]);
    ex(&mut s, "edit.cut", json!({}));
    assert_eq!(layers(&s), vec![id], "no new layer");
    let n = net(&s, id);
    assert_eq!(n.segments.len(), 2);
    assert_eq!(subpaths(&n), 2, "the path split in two");
    let ve = s.ds().unwrap().vector_edit.clone().expect("edit mode stays on");
    assert!(ve.segments.is_empty() && ve.vertices.is_empty());
    leave(&mut s);
    ex(&mut s, "edit.paste", json!({}));
    let all = layers(&s);
    assert_eq!(all.len(), 2);
    let new = all[1];
    assert_eq!(net(&s, new).segments.len(), 1);
    assert_eq!(page_points(&s, new), vec![(200, 150), (300, 100)]);
}

#[test]
fn cut_is_one_undo_step() {
    let mut s = Session::new();
    let id = path(&mut s);
    select(&mut s, &[], &[1]);
    ex(&mut s, "edit.cut", json!({}));
    assert_eq!(net(&s, id).segments.len(), 2);
    s.undo().unwrap();
    assert_eq!(net(&s, id).segments.len(), 3);
    assert_eq!(page_points(&s, id), vec![(100, 100), (200, 150), (300, 100), (400, 200)]);
}

#[test]
fn a_cut_point_takes_its_segments_with_it() {
    let mut s = Session::new();
    let id = path(&mut s);
    select(&mut s, &[1], &[]);
    ex(&mut s, "edit.cut", json!({}));
    assert_eq!(net(&s, id).segments.len(), 1, "only C-D is left");
    leave(&mut s);
    ex(&mut s, "edit.paste", json!({}));
    let new = *layers(&s).last().unwrap();
    assert_eq!(net(&s, new).segments.len(), 2, "A-B and B-C");
}

#[test]
fn pasting_segments_while_editing_adds_them_to_the_same_network_in_place() {
    let mut s = Session::new();
    let id = path(&mut s);
    select(&mut s, &[], &[1]);
    ex(&mut s, "edit.copy", json!({}));
    let r = ex(&mut s, "edit.paste", json!({}));
    assert_eq!(r["vertices"], 2);
    assert_eq!(layers(&s), vec![id], "no new layer");
    let n = net(&s, id);
    assert_eq!((n.vertices.len(), n.segments.len()), (6, 4));
    assert_eq!(page_points(&s, id)[4..], [(200, 150), (300, 100)], "on top of the original");
    let ve = s.ds().unwrap().vector_edit.clone().unwrap();
    assert_eq!(ve.vertices, vec![4, 5], "the pasted points are selected");
    assert!(ve.segments.is_empty());
    // One undo reverts it.
    s.undo().unwrap();
    assert_eq!(net(&s, id).segments.len(), 3);
    assert_eq!(net(&s, id).vertices.len(), 4);
}

#[test]
fn pasting_at_a_point_moves_the_copied_top_left_there() {
    let mut s = Session::new();
    let id = path(&mut s);
    select(&mut s, &[], &[1]);
    ex(&mut s, "edit.copy", json!({}));
    ex(&mut s, "edit.paste", json!({"x": 0, "y": 0}));
    assert_eq!(page_points(&s, id)[4..], [(0, 50), (100, 0)]);
}

#[test]
fn pasting_a_rectangle_while_editing_adds_its_outline_in_the_vectors_coordinates() {
    let mut s = Session::new();
    let rect = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 500, "y": 300, "width": 60, "height": 40})));
    ex(&mut s, "select.set", json!({"ids": [rect]}));
    ex(&mut s, "edit.copy", json!({}));
    let id = path(&mut s);
    let before = world_rect(&s, id);
    ex(&mut s, "edit.paste", json!({}));
    assert_eq!(layers(&s), vec![rect, id], "no new layer");
    let n = net(&s, id);
    assert_eq!((n.vertices.len(), n.segments.len()), (8, 7), "four corners, four sides, joined to the path");
    let mut corners: Vec<(i64, i64)> = page_points(&s, id)[4..].to_vec();
    corners.sort_unstable();
    assert_eq!(corners, vec![(500, 300), (500, 340), (560, 300), (560, 340)]);
    let ve = s.ds().unwrap().vector_edit.clone().unwrap();
    assert_eq!(ve.vertices, vec![4, 5, 6, 7]);
    let after = world_rect(&s, id);
    assert!(after.2 > before.2 && after.3 > before.3, "the vector's bounds grow: {before:?} to {after:?}");
    assert_eq!((after.0, after.1), (before.0, before.1));
    assert_eq!(after.0 + after.2, 560.0);
    s.undo().unwrap();
    assert_eq!(net(&s, id).vertices.len(), 4);
    assert_eq!(world_rect(&s, id), before);
}

#[test]
fn a_rotated_vector_takes_pasted_points_in_its_own_coordinates() {
    let mut s = Session::new();
    let id = path(&mut s);
    ex(&mut s, "node.set", json!({"id": id, "props": {"rotation": 90}}));
    select(&mut s, &[], &[1]);
    ex(&mut s, "edit.copy", json!({}));
    let before = page_points(&s, id);
    assert_ne!(before[0], (100, 100), "the vector is rotated");
    ex(&mut s, "edit.paste", json!({}));
    let after = page_points(&s, id);
    assert_eq!(after[4..], before[1..3]);
}

#[test]
fn layers_with_no_outline_are_skipped_and_named() {
    let mut s = Session::new();
    let rect = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 500, "y": 300, "width": 60, "height": 40})));
    let text = id_of(&ex(&mut s, "text.create", json!({"text": "hello", "x": 500, "y": 400})));
    let name = s.doc().unwrap().node(text).unwrap().name.clone();
    // Only text: nothing to add, and nothing changes.
    ex(&mut s, "select.set", json!({"ids": [text]}));
    ex(&mut s, "edit.copy", json!({}));
    let id = path(&mut s);
    let err = s.execute("edit.paste", &json!({})).unwrap_err();
    assert!(matches!(err, EngineError::Disabled(..)), "{err}");
    assert_eq!(net(&s, id).vertices.len(), 4);
    // The rectangle joins, the text is named as skipped.
    leave(&mut s);
    ex(&mut s, "edit.copy", json!({"ids": [rect, text]}));
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    let r = ex(&mut s, "edit.paste", json!({}));
    assert_eq!(r["skipped"], json!([name]));
    assert_eq!(net(&s, id).vertices.len(), 8);
}

#[test]
fn a_parent_still_pastes_layers_while_editing() {
    let mut s = Session::new();
    let id = path(&mut s);
    select(&mut s, &[], &[1]);
    ex(&mut s, "edit.copy", json!({}));
    let page = s.page().unwrap();
    ex(&mut s, "edit.paste", json!({"parent": page}));
    assert_eq!(layers(&s).len(), 2);
    assert_eq!(net(&s, id).segments.len(), 3);
}

#[test]
fn an_empty_clipboard_refuses_to_paste_while_editing() {
    let mut s = Session::new();
    path(&mut s);
    assert!(matches!(s.execute("edit.paste", &json!({})), Err(EngineError::Disabled(..))));
}

#[test]
fn a_selected_face_copies_its_own_outline_with_the_fill() {
    let mut s = Session::new();
    let r = ex(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 80, "height": 60}));
    let id = id_of(&r);
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    select(&mut s, &[], &[0, 1, 2, 3]);
    ex(&mut s, "edit.copy", json!({}));
    leave(&mut s);
    ex(&mut s, "edit.paste", json!({}));
    let new = *layers(&s).last().unwrap();
    let n = net(&s, new);
    assert_eq!((n.segments.len(), n.regions.len()), (4, 1), "the closed shape stays filled");
    assert!(matches!(s.doc().unwrap().node(new).unwrap().kind, NodeKind::Vector { .. }));
}

#[test]
fn copy_cut_and_paste_survive_junk_in_vector_edit_mode() {
    let junk = [
        json!({}),
        json!(null),
        json!([1, 2]),
        json!({"ids": ["x", -1, 99999]}),
        json!({"id": "nope", "x": "NaN", "y": 1e308}),
        json!({"parent": "nope", "x": -1e308, "y": "x"}),
        json!({"x": 1e308, "y": 1e308}),
    ];
    for cmd in ["edit.copy", "edit.cut", "edit.paste", "edit.pasteOver", "edit.pasteReplace"] {
        for j in &junk {
            let mut s = Session::new();
            path(&mut s);
            select(&mut s, &[1], &[0]);
            ex(&mut s, "text.create", json!({"text": "t"}));
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
            // And after something was copied from the edit selection.
            let mut s = Session::new();
            path(&mut s);
            select(&mut s, &[2], &[]);
            let _ = s.execute("edit.copy", &json!({}));
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
}

#[test]
fn cutting_a_selected_region_deletes_exactly_the_segments_it_copies_and_keeps_a_tail() {
    let mut s = Session::new();
    let r = ex(&mut s, "vector.create", json!({"points": [[100, 100], [200, 100], [150, 200]], "closed": true}));
    let id = id_of(&r);
    // A tail: one open segment from the first corner to a new point.
    crate::vectoredit::edit(s.doc_mut().unwrap(), id, |n| {
        n.vertices.push(figcraft_doc::Vertex::new(20.0, 20.0));
        let tail = n.vertices.len() - 1;
        crate::vectoredit::ops::connect(n, 0, tail, (0.0, 0.0), (0.0, 0.0));
    })
    .unwrap();
    assert_eq!((net(&s, id).segments.len(), net(&s, id).regions.len()), (4, 1));
    ex(&mut s, "vector.edit.begin", json!({"id": id}));
    ex(&mut s, "vector.edit.select", json!({"regions": [0]}));
    ex(&mut s, "edit.cut", json!({}));
    let n = net(&s, id);
    assert_eq!(n.segments.len(), 1, "only the tail is left");
    assert!(n.regions.is_empty());
    leave(&mut s);
    ex(&mut s, "edit.paste", json!({}));
    let new = *layers(&s).last().unwrap();
    assert_eq!(net(&s, new).segments.len(), 3, "the copy holds the region's three segments, not the tail");
}
