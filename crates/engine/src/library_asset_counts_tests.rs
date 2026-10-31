//! What a file counts as inserting and detaching a library's styles and variables, as the help
//! center's library analytics page says: applying one, or copying or duplicating content that uses
//! it, inserts it; the Detach button detaches it. Content that is an instance counts as the
//! instance alone, and local styles and variables count nothing. An undo takes its count back.

use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::{ex, idv};
use crate::library_server::asset_actions;
use figcraft_doc::{AssetCount, NodeId, PublishedLibrary};

/// A file with a published library holding a paint style, a colour variable and a Card, and a
/// second file that uses it: the second session, the library's id and its keys.
fn user() -> (Session, String, Value) {
    let mut a = Session::new();
    let r = idv(&ex(&mut a, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40})));
    let style = ex(&mut a, "style.create", json!({"type": "paint", "name": "Brand", "from": r.0}));
    let coll = ex(&mut a, "variable.createCollection", json!({"name": "Tokens"}));
    ex(&mut a, "variable.create", json!({"name": "Accent", "type": "color", "collection": coll["id"], "value": "#0D99FF"}));
    let card = idv(&ex(&mut a, "frame.create", json!({"x": 0, "y": 100, "width": 100, "height": 40, "name": "Card"})));
    ex(&mut a, "style.apply", json!({"ids": [card.0], "style": style["id"]}));
    ex(&mut a, "component.create", json!({"ids": [card.0]}));
    ex(&mut a, "library.publish", json!({}));
    let lib: PublishedLibrary = a.doc().unwrap().published.as_deref().unwrap().clone();
    let path = std::env::temp_dir().join(format!("figcraft-asset-counts-{}-{:?}.figcraft", std::process::id(), std::thread::current().id()));
    ex(&mut a, "file.save", json!({"path": path.to_string_lossy()}));
    let mut b = Session::new();
    let id = ex(&mut b, "library.add", json!({"path": path.to_string_lossy()}))["id"].as_str().unwrap().to_string();
    assert_eq!(lib.manifest.id, id);
    (b, id, json!({"style": "Brand", "variable": "Accent"}))
}

fn rect(s: &mut Session, x: i32) -> NodeId {
    idv(&ex(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 40, "height": 40})))
}

/// (inserted, detached) of the style and of the variable, as `doc` keeps them.
fn counts(s: &Session) -> ((u32, u32), (u32, u32)) {
    let of = |key: &str| {
        s.doc().unwrap().libraries.asset_counts.iter().find(|c: &&AssetCount| c.key.starts_with(key)).map_or((0, 0), |c| (c.inserted, c.detached))
    };
    (of("S:"), of("VariableID:"))
}

#[test]
fn applying_counts_once_per_layer_that_takes_it() {
    let (mut s, lib, _) = user();
    let (one, two) = (rect(&mut s, 0), rect(&mut s, 100));
    ex(&mut s, "library.applyStyle", json!({"library": lib, "style": "Brand", "ids": [one.0, two.0]}));
    assert_eq!(counts(&s).0, (2, 0));
    ex(&mut s, "library.applyStyle", json!({"library": lib, "style": "Brand", "ids": [one.0]}));
    assert_eq!(counts(&s).0, (2, 0), "a layer that has it already adds nothing");
    let three = rect(&mut s, 200);
    ex(&mut s, "library.bindVariable", json!({"library": lib, "variable": "Accent", "ids": [three.0], "field": "fill"}));
    assert_eq!(counts(&s).1, (1, 0));
}

#[test]
fn duplicating_and_pasting_content_counts_what_it_uses() {
    let (mut s, lib, _) = user();
    let one = rect(&mut s, 0);
    ex(&mut s, "library.applyStyle", json!({"library": lib, "style": "Brand", "ids": [one.0]}));
    ex(&mut s, "select.set", json!({"ids": [one.0]}));
    ex(&mut s, "edit.duplicate", json!({}));
    assert_eq!(counts(&s).0, (2, 0), "the copy inserts it too");
    ex(&mut s, "select.set", json!({"ids": [one.0]}));
    ex(&mut s, "edit.copy", json!({}));
    ex(&mut s, "edit.paste", json!({}));
    assert_eq!(counts(&s).0, (3, 0));
}

#[test]
fn an_instance_counts_as_its_component_alone() {
    let (mut s, lib, _) = user();
    let i = idv(&ex(&mut s, "library.place", json!({"library": lib, "component": "Card", "x": 0, "y": 0})));
    assert_eq!(counts(&s).0, (0, 0), "placing it");
    ex(&mut s, "select.set", json!({"ids": [i.0]}));
    ex(&mut s, "edit.duplicate", json!({}));
    assert_eq!(counts(&s).0, (0, 0), "duplicating it");
    assert_eq!(s.doc().unwrap().libraries.inserted[0].count, 2);
}

#[test]
fn only_the_detach_buttons_detach() {
    let (mut s, lib, _) = user();
    let (one, two) = (rect(&mut s, 0), rect(&mut s, 100));
    ex(&mut s, "library.applyStyle", json!({"library": lib, "style": "Brand", "ids": [one.0, two.0]}));
    ex(&mut s, "library.bindVariable", json!({"library": lib, "variable": "Accent", "ids": [two.0], "field": "stroke"}));
    ex(&mut s, "style.detach", json!({"ids": [one.0, two.0], "which": "fill"}));
    assert_eq!(counts(&s).0, (2, 2));
    ex(&mut s, "variable.bind", json!({"ids": [two.0], "field": "stroke"}));
    assert_eq!(counts(&s).1, (1, 1));
    // Another style in its place isn't a detach of the first.
    let local = ex(&mut s, "style.create", json!({"type": "paint", "name": "Mine", "from": one.0}));
    ex(&mut s, "library.applyStyle", json!({"library": lib, "style": "Brand", "ids": [one.0]}));
    ex(&mut s, "style.apply", json!({"ids": [one.0], "style": local["id"]}));
    assert_eq!(counts(&s).0, (3, 2));
}

#[test]
fn local_styles_count_nothing_and_undo_takes_a_count_back() {
    let (mut s, lib, _) = user();
    let one = rect(&mut s, 0);
    let local = ex(&mut s, "style.create", json!({"type": "paint", "name": "Mine", "from": one.0}));
    ex(&mut s, "style.apply", json!({"ids": [one.0], "style": local["id"]}));
    ex(&mut s, "style.detach", json!({"ids": [one.0], "which": "fill"}));
    assert!(s.doc().unwrap().libraries.asset_counts.is_empty());
    assert!(asset_actions(s.doc().unwrap(), &lib).is_none());
    ex(&mut s, "library.applyStyle", json!({"library": lib, "style": "Brand", "ids": [one.0]}));
    let report = asset_actions(s.doc().unwrap(), &lib).unwrap();
    assert_eq!((report.inserted.len(), report.detached.len()), (1, 0));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(counts(&s).0, (0, 0));
}
