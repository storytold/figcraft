//! Layer ids in params: naming something that isn't a layer is a mistake, and says so, rather than
//! quietly meaning "none".
//!
//! `node.create`'s `parent` was `id(p, "parent").unwrap_or(page)`, so any value that isn't an id —
//! a `{"$step": n}` nothing had resolved, a name, a negative number — became "no parent" and the
//! layer landed on the page while the call reported success. The same shape sat on `image.place`
//! and `instance.create`'s parents, on `index`, and on the `ids` of every command that targets
//! layers (an entry that wasn't an id was dropped and the rest ran).

use crate::params::id_of;
use crate::{EngineError, Session};
use figcraft_doc::*;
use serde_json::{Value, json};

/// The page's children, by name (bottom first).
fn top(s: &Session) -> Vec<String> {
    let doc = s.doc().unwrap();
    let page = s.page().unwrap();
    doc.node(page).unwrap().children.iter().filter_map(|c| doc.node(*c).map(|n| n.name.clone())).collect()
}

/// The message of a failure, or `None` when the call went through.
fn failed(r: std::result::Result<Value, EngineError>) -> Option<String> {
    match r {
        Ok(_) => None,
        Err(e) => Some(e.to_string()),
    }
}

fn rect(parent: Value) -> Value {
    json!({"type": "rectangle", "name": "child", "parent": parent, "x": 0, "y": 0, "width": 10, "height": 10})
}

fn card(s: &mut Session) -> NodeId {
    let v = s.execute("node.create", &json!({"type": "frame", "name": "Card", "x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    id_of(&v["id"]).unwrap()
}

#[test]
fn a_parent_that_is_not_an_id_is_refused_rather_than_becoming_the_page() {
    let mut s = Session::new();
    let card = card(&mut s);
    for parent in [json!({"$step": 0}), json!("Card"), json!(-1), json!(true)] {
        let msg = failed(s.execute("node.create", &rect(parent.clone()))).unwrap_or_else(|| panic!("{parent} must be refused"));
        assert!(msg.contains("`parent` must be a node id"), "{parent}: {msg}");
    }
    assert_eq!(top(&s), ["Card"], "nothing landed on the page");
    let doc = s.doc().unwrap();
    assert!(doc.node(card).unwrap().children.is_empty(), "nor inside the frame");
}

#[test]
fn leaving_the_parent_out_or_null_still_means_the_page() {
    let mut s = Session::new();
    let card = card(&mut s);
    for parent in [None, Some(json!(null))] {
        let mut p = rect(json!(0));
        match parent {
            None => _ = p.as_object_mut().unwrap().remove("parent"),
            Some(v) => p["parent"] = v,
        }
        s.execute("node.create", &p).unwrap();
    }
    assert_eq!(top(&s), ["Card", "child", "child"]);
    let doc = s.doc().unwrap();
    assert!(doc.node(card).unwrap().children.is_empty());
}

#[test]
fn a_parent_id_that_names_nothing_is_still_reported() {
    let mut s = Session::new();
    card(&mut s);
    let msg = failed(s.execute("node.create", &rect(json!(99999)))).unwrap_or_else(|| panic!("a missing layer must fail"));
    assert!(msg.contains("no such node 99999"), "{msg}");
}

#[test]
fn an_index_that_is_not_a_whole_non_negative_number_is_refused() {
    let mut s = Session::new();
    let card = card(&mut s);
    // (A boolean is a number to `f64_of` — `true` reads as 1 — as for `x`, `width` and every
    // other number, so it is deliberately not in this list.)
    for index in [json!("last"), json!(1.5), json!(-1), json!(-0.5), json!(1e12)] {
        let mut p = rect(json!(card));
        p["index"] = index.clone();
        let msg = failed(s.execute("node.create", &p)).unwrap_or_else(|| panic!("index {index} must be refused"));
        assert!(msg.contains("`index`"), "{index}: {msg}");
    }
    let doc = s.doc().unwrap();
    assert!(doc.node(card).unwrap().children.is_empty(), "nothing was inserted");
}

#[test]
fn a_whole_index_still_inserts_where_it_says() {
    let mut s = Session::new();
    let card = card(&mut s);
    for name in ["a", "b"] {
        s.execute("node.create", &json!({"type": "rectangle", "name": name, "parent": card, "x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    }
    s.execute("node.create", &json!({"type": "rectangle", "name": "mid", "parent": card, "index": 1, "x": 0, "y": 0, "width": 10, "height": 10}))
        .unwrap();
    let doc = s.doc().unwrap();
    let names: Vec<&str> = doc.node(card).unwrap().children.iter().filter_map(|c| doc.node(*c).map(|n| n.name.as_str())).collect();
    assert_eq!(names, ["a", "mid", "b"]);
}

#[test]
fn ids_entries_that_are_not_ids_are_refused_instead_of_being_dropped() {
    let mut s = Session::new();
    let card = card(&mut s);
    // The junk used to be dropped and the rest applied, so `["nope", card]` renamed the frame.
    for ids in [json!(["nope"]), json!(["nope", card]), json!([{"$step": 0}, card]), json!([null])] {
        let msg =
            failed(s.execute("node.set", &json!({"ids": ids, "props": {"name": "renamed"}}))).unwrap_or_else(|| panic!("{ids} must be refused"));
        assert!(msg.contains("`ids`"), "{ids}: {msg}");
    }
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(card).unwrap().name, "Card", "nothing was renamed");
}

#[test]
fn an_ids_that_is_not_an_array_is_refused_but_null_means_the_selection() {
    let mut s = Session::new();
    let card = card(&mut s);
    let msg =
        failed(s.execute("node.set", &json!({"ids": "nope", "props": {"name": "renamed"}}))).unwrap_or_else(|| panic!("a non-array ids must fail"));
    assert!(msg.contains("`ids` must be an array"), "{msg}");
    // An explicit null says the same as leaving it out: the selection.
    s.execute("select.set", &json!({"ids": [card]})).unwrap();
    s.execute("node.set", &json!({"ids": null, "props": {"name": "Selected"}})).unwrap();
    assert_eq!(s.doc().unwrap().node(card).unwrap().name, "Selected");
}

#[test]
fn a_single_id_that_is_not_an_id_is_refused() {
    let mut s = Session::new();
    card(&mut s);
    let msg =
        failed(s.execute("node.set", &json!({"id": {"$step": 0}, "props": {"name": "renamed"}}))).unwrap_or_else(|| panic!("a bad id must fail"));
    assert!(msg.contains("`id` must be a node id"), "{msg}");
}

#[test]
fn an_image_and_an_instance_parent_are_checked_the_same_way() {
    let mut s = Session::new();
    card(&mut s);
    let img = failed(s.execute("image.place", &json!({"data": "aGk=", "parent": {"$step": 0}})));
    assert!(img.unwrap_or_default().contains("`parent` must be a node id"), "image.place");
    let inst = failed(s.execute("instance.create", &json!({"component": 99999, "parent": {"$step": 0}})));
    assert!(inst.is_some(), "instance.create refuses both a bad component and a bad parent");
}

#[test]
fn a_huge_bad_value_is_cut_short_in_the_error() {
    let mut s = Session::new();
    card(&mut s);
    let junk = "é".repeat(10_000);
    let msg = failed(s.execute("node.set", &json!({"ids": junk, "props": {"name": "renamed"}}))).unwrap_or_else(|| panic!("must fail"));
    assert!(msg.contains("`ids` must be an array") && msg.ends_with("…)"), "{msg}");
    assert!(msg.chars().count() < 200, "{} chars", msg.chars().count());
}
