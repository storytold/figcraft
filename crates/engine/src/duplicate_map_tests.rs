//! `edit.duplicate` with `map: true` (#995): every original layer, descendants included, paired
//! with its copy, so a caller can edit inside a copy without inspecting it.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{Document, NodeId};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn idv(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// A "Screen" frame holding an instance of a "Card" component (with a "Label" sublayer), a group
/// of two rectangles and a nested frame with a text; returns (screen, card).
fn screen(s: &mut Session) -> (NodeId, NodeId) {
    let card = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 80, "height": 40, "name": "Card"})));
    ex(s, "text.create", json!({"text": "Hi", "x": 4, "y": 4, "parent": card, "props": {"name": "Label"}}));
    let card = idv(&ex(s, "component.create", json!({"ids": [card]})));
    let scr = idv(&ex(s, "frame.create", json!({"x": 200, "y": 0, "width": 300, "height": 300, "name": "Screen"})));
    ex(s, "instance.create", json!({"component": card, "x": 210, "y": 10, "parent": scr}));
    let r = |s: &mut Session, y: f64| idv(&ex(s, "shape.rectangle", json!({"x": 210, "y": y, "width": 20, "height": 20, "parent": scr})));
    let (a, b) = (r(s, 100.0), r(s, 130.0));
    ex(s, "object.group", json!({"ids": [a, b]}));
    let inner = idv(&ex(s, "frame.create", json!({"x": 250, "y": 100, "width": 100, "height": 100, "parent": scr, "name": "Inner"})));
    ex(s, "text.create", json!({"text": "Body", "x": 260, "y": 110, "parent": inner}));
    (scr, card)
}

/// The child indexes from `root` down to `id`.
fn path(doc: &Document, root: NodeId, id: NodeId) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = id;
    while at != root {
        out.push(doc.index_in_parent(at).unwrap());
        at = doc.node(at).unwrap().parent.unwrap();
    }
    out.reverse();
    out
}

fn pairs(out: &Value) -> Vec<(NodeId, NodeId)> {
    out["map"].as_object().unwrap().iter().map(|(k, v)| (NodeId(k.parse().unwrap()), id_of(v).unwrap())).collect()
}

/// The map pairs every layer of each original's subtree with the layer at the same place in its
/// copy, of the same type and name.
fn assert_complete(s: &Session, roots: &[(NodeId, NodeId)], out: &Value) {
    let doc = s.doc().unwrap();
    let map = pairs(out);
    let all: usize = roots.iter().map(|(o, _)| doc.subtree(*o).len()).sum();
    assert_eq!(map.len(), all, "every original layer, descendants included, is in the map: {out}");
    for (o, c) in map {
        let (root_o, root_c) = roots.iter().find(|(r, _)| doc.subtree(*r).contains(&o)).copied().unwrap();
        assert!(doc.subtree(root_c).contains(&c), "{c} is inside the copy of {root_o}");
        assert_eq!(path(doc, root_o, o), path(doc, root_c, c), "{c} sits where {o} does");
        let (a, b) = (doc.node(o).unwrap(), doc.node(c).unwrap());
        assert_eq!(a.name, b.name, "{o} → {c}");
        // A main component's copy is an instance of it (the tests check the roots' types).
        if o != root_o {
            assert_eq!(a.kind.type_name(), b.kind.type_name(), "{o} → {c}");
        }
    }
}

#[test]
fn map_pairs_nested_frames_groups_and_instances_with_their_copies() {
    let mut s = Session::new();
    let (scr, _) = screen(&mut s);
    let out = ex(&mut s, "edit.duplicate", json!({"ids": [scr], "map": true}));
    let copy = id_of(&out["ids"][0]).unwrap();
    assert_eq!(out["map"][scr.to_string()], json!(copy), "the root maps to the copy in `ids`");
    assert_complete(&s, &[(scr, copy)], &out);
}

#[test]
fn map_covers_several_roots_and_a_main_component_copied_as_an_instance() {
    let mut s = Session::new();
    let (scr, card) = screen(&mut s);
    let out = ex(&mut s, "edit.duplicate", json!({"ids": [card, scr], "map": true}));
    let ids: Vec<NodeId> = out["ids"].as_array().unwrap().iter().map(|v| id_of(v).unwrap()).collect();
    assert_eq!(ids.len(), 2);
    let (card_copy, scr_copy) = (out["map"][card.to_string()].clone(), out["map"][scr.to_string()].clone());
    assert!(ids.contains(&id_of(&card_copy).unwrap()) && ids.contains(&id_of(&scr_copy).unwrap()), "{out}");
    let doc = s.doc().unwrap();
    let card_copy = id_of(&card_copy).unwrap();
    assert_eq!(doc.node(card_copy).unwrap().kind.type_name(), "INSTANCE", "a main component's copy is an instance");
    assert_complete(&s, &[(card, card_copy), (scr, id_of(&scr_copy).unwrap())], &out);
}

#[test]
fn without_map_the_result_is_just_the_ids() {
    let mut s = Session::new();
    let (scr, _) = screen(&mut s);
    for p in [json!({"ids": [scr]}), json!({"ids": [scr], "map": false})] {
        let out = ex(&mut s, "edit.duplicate", p);
        let keys: Vec<&String> = out.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["ids"], "{out}");
    }
}

#[test]
fn a_batch_edits_inside_the_copy_through_the_map() {
    let mut s = Session::new();
    let (scr, _) = screen(&mut s);
    let doc = s.doc().unwrap();
    let inner = *doc.node(scr).unwrap().children.last().unwrap();
    let body = doc.node(inner).unwrap().children[0];
    let field = format!("map/{body}");
    ex(
        &mut s,
        "command.batch",
        json!({"commands": [
            {"command": "edit.duplicate", "as": "copy", "params": {"ids": [scr], "map": true}},
            {"command": "node.set", "params": {"ids": [{"$ref": "copy", "field": field}], "props": {"text": "Copy"}}},
        ]}),
    );
    let doc = s.doc().unwrap();
    let text = |id: NodeId| match &doc.node(id).unwrap().kind {
        figcraft_doc::NodeKind::Text(t) => t.characters.clone(),
        _ => String::new(),
    };
    assert_eq!(text(body), "Body", "the original is untouched");
    let copy = s.selection()[0];
    let copy_body = doc.subtree(copy).into_iter().find(|i| path(doc, copy, *i) == path(doc, scr, body)).unwrap();
    assert_eq!(text(copy_body), "Copy", "the edit landed in the copy");
}
