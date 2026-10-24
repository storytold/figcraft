//! The Variables editor's commands: renaming, scopes, duplicates, modes, collections and aliases.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap()
}

fn tokens(s: &mut Session) {
    ex(s, "variable.createCollection", json!({"name": "Tokens", "modes": ["Light", "Dark"]}));
    ex(s, "variable.create", json!({"collection": "Tokens", "name": "space/sm", "type": "number", "value": 8}));
    ex(s, "variable.create", json!({"collection": "Tokens", "name": "space/md", "type": "number", "value": 16}));
    ex(s, "variable.create", json!({"collection": "Tokens", "name": "brand", "type": "color", "value": "#0D99FF"}));
}

#[test]
fn rename_describe_scope_and_duplicate() {
    let mut s = Session::new();
    tokens(&mut s);
    ex(&mut s, "variable.update", json!({"variable": "space/sm", "name": " spacing / small ", "description": "Tight gaps"}));
    let v = s.doc().unwrap().variables.get("spacing/small").cloned().unwrap();
    assert_eq!(v.description, "Tight gaps");
    // Names stay unique in a collection, and can't be blank.
    assert!(s.execute("variable.update", &json!({"variable": "spacing/small", "name": "space/md"})).is_err());
    assert!(s.execute("variable.update", &json!({"variable": "spacing/small", "name": " / "})).is_err());
    // Scopes: a subset limits pickers, all of them (or ALL_SCOPES) is no limit, [] hides it.
    ex(&mut s, "variable.update", json!({"variable": "spacing/small", "scopes": ["GAP", "CORNER_RADIUS"]}));
    let v = s.doc().unwrap().variables.get("spacing/small").cloned().unwrap();
    assert!(v.in_scope("GAP") && !v.in_scope("OPACITY"));
    ex(&mut s, "variable.update", json!({"variable": "spacing/small", "scopes": ["ALL_SCOPES"]}));
    assert_eq!(s.doc().unwrap().variables.get("spacing/small").unwrap().scopes, None);
    ex(&mut s, "variable.update", json!({"variable": "spacing/small", "scopes": []}));
    assert!(!s.doc().unwrap().variables.get("spacing/small").unwrap().in_scope("GAP"));
    assert!(s.execute("variable.update", &json!({"variable": "brand", "scopes": ["GAP"]})).is_err());
    ex(&mut s, "variable.update", json!({"variable": "brand", "scopes": ["ALL_FILLS"]}));
    let brand = s.doc().unwrap().variables.get("brand").cloned().unwrap();
    assert!(brand.in_scope("TEXT_FILL") && !brand.in_scope("STROKE_COLOR"));
    // Duplicates sit after the original with a fresh id and name.
    let id = ex(&mut s, "variable.duplicate", json!({"variable": "brand"}))["id"].as_str().unwrap().to_string();
    let id2 = ex(&mut s, "variable.duplicate", json!({"variable": "brand"}))["id"].as_str().unwrap().to_string();
    let vars = &s.doc().unwrap().variables;
    assert_eq!(vars.get(&id).unwrap().name, "brand copy");
    assert_eq!(vars.get(&id2).unwrap().name, "brand copy 2");
    assert_eq!(vars.get(&id).unwrap().scopes, brand.scopes);
}

#[test]
fn modes_and_collections_rename_and_delete() {
    let mut s = Session::new();
    tokens(&mut s);
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "width", "variable": "space/md"}));
    ex(&mut s, "variable.set", json!({"variable": "space/md", "mode": "Dark", "value": 24}));
    ex(&mut s, "variable.setMode", json!({"ids": [f], "collection": "Tokens", "mode": "Dark"}));
    assert_eq!(s.doc().unwrap().node(f).unwrap().width, 24.0);
    ex(&mut s, "variable.renameMode", json!({"collection": "Tokens", "mode": "Dark", "name": "Night"}));
    // Deleting the mode a frame uses puts it back on the default mode.
    ex(&mut s, "variable.removeMode", json!({"collection": "Tokens", "mode": "Night"}));
    assert_eq!(s.doc().unwrap().node(f).unwrap().width, 16.0);
    assert!(s.execute("variable.removeMode", &json!({"collection": "Tokens", "mode": "Light"})).is_err(), "last mode stays");
    // A new mode gets a fresh id even after a delete.
    let m = ex(&mut s, "variable.addMode", json!({"collection": "Tokens", "name": "Dim"}))["id"].as_str().unwrap().to_string();
    let coll = s.doc().unwrap().variables.collection("Tokens").cloned().unwrap();
    assert_eq!(coll.modes.iter().filter(|x| x.id == m).count(), 1);
    ex(&mut s, "variable.createCollection", json!({"name": "Other"}));
    assert!(s.execute("variable.renameCollection", &json!({"collection": "Tokens", "name": "Other"})).is_err());
    ex(&mut s, "variable.renameCollection", json!({"collection": "Tokens", "name": "Primitives"}));
    // Deleting a collection removes its variables; bound layers keep their values.
    let r = ex(&mut s, "variable.deleteCollection", json!({"collection": "Primitives"}));
    assert_eq!(r["deletedVariables"], 3);
    let doc = s.doc().unwrap();
    assert!(doc.variables.get("space/md").is_none());
    let n = doc.node(f).unwrap();
    assert!(n.bound.is_empty() && n.var_modes.is_empty());
    assert_eq!(n.width, 16.0);
    // Undo brings it all back.
    ex(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().variables.get("space/md").is_some());
}

#[test]
fn aliases_follow_their_target_and_refuse_loops_and_other_types() {
    let mut s = Session::new();
    tokens(&mut s);
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "width", "variable": "space/md"}));
    ex(&mut s, "variable.set", json!({"variable": "space/md", "value": {"alias": "space/sm"}}));
    // Stored by id, resolved through the target.
    let sm = s.doc().unwrap().variables.get("space/sm").unwrap().id.clone();
    let md = s.doc().unwrap().variables.get("space/md").cloned().unwrap();
    assert!(md.values.values().any(|v| *v == figcraft_doc::VariableValue::Alias(sm.clone())));
    assert_eq!(s.doc().unwrap().node(f).unwrap().width, 8.0);
    ex(&mut s, "variable.set", json!({"variable": "space/sm", "value": 12}));
    assert_eq!(s.doc().unwrap().node(f).unwrap().width, 12.0);
    // sm → md would loop (md already aliases sm); self-aliases too; types must match.
    assert!(s.execute("variable.set", &json!({"variable": "space/sm", "value": {"alias": "space/md"}})).is_err());
    assert!(s.execute("variable.set", &json!({"variable": "space/sm", "value": {"alias": "space/sm"}})).is_err());
    assert!(s.execute("variable.set", &json!({"variable": "space/sm", "value": {"alias": "brand"}})).is_err());
    assert!(s.execute("variable.set", &json!({"variable": "space/sm", "value": {"alias": "nope"}})).is_err());
}

#[test]
fn field_scopes_by_property_and_layer() {
    let mut s = Session::new();
    let f = id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    let r = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"]).unwrap();
    let doc = s.doc().unwrap();
    let scope = |id, field| crate::cmd::variables::field_scope(doc, id, field);
    assert_eq!(scope(f, "fill"), Some("FRAME_FILL"));
    assert_eq!(scope(r, "fill"), Some("SHAPE_FILL"));
    assert_eq!(scope(f, "paddingHorizontal"), Some("GAP"));
    assert_eq!(scope(f, "visible"), None);
}

#[test]
fn create_collection_refuses_a_taken_name() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Theme"}));
    let before = s.doc().unwrap().variables.collections.len();
    assert!(s.execute("variable.createCollection", &json!({"name": "Theme"})).is_err());
    assert_eq!(s.doc().unwrap().variables.collections.len(), before);
    ex(&mut s, "variable.createCollection", json!({"name": "Theme 2"}));
    assert_eq!(s.doc().unwrap().variables.collections.len(), before + 1);
}

/// The collection's variables in document order (what the editor's table shows).
fn order(s: &Session, collection: &str) -> Vec<String> {
    let doc = s.doc().unwrap();
    let id = doc.variables.collection(collection).map(|c| c.id.clone()).unwrap_or_default();
    doc.variables.variables.iter().filter(|v| v.collection == id).map(|v| v.name.clone()).collect()
}

#[test]
fn moving_a_variable_reorders_it_and_follows_the_folder_it_lands_in() {
    let mut s = Session::new();
    tokens(&mut s);
    // Dropped above a variable of a folder: it joins that folder (the table groups rows by name).
    ex(&mut s, "variable.move", json!({"variable": "brand", "before": "space/sm"}));
    assert_eq!(order(&s, "Tokens"), ["space/brand", "space/sm", "space/md"], "it takes the folder of the row it lands on");
    // To the end of the collection, keeping its folder.
    ex(&mut s, "variable.move", json!({"variable": "space/sm"}));
    assert_eq!(order(&s, "Tokens"), ["space/brand", "space/md", "space/sm"]);
    // Into another folder, and out of every folder.
    ex(&mut s, "variable.move", json!({"variable": "space/md", "group": "brand"}));
    assert_eq!(order(&s, "Tokens"), ["space/brand", "space/sm", "brand/md"]);
    ex(&mut s, "variable.move", json!({"variable": "brand/md", "group": ""}));
    assert_eq!(order(&s, "Tokens"), ["space/brand", "space/sm", "md"]);
    // A name the folder already holds gains a number.
    ex(&mut s, "variable.create", json!({"collection": "Tokens", "name": "brand/brand", "type": "number", "value": 1}));
    ex(&mut s, "variable.move", json!({"variable": "space/brand", "group": "brand"}));
    assert_eq!(order(&s, "Tokens"), ["space/sm", "md", "brand/brand", "brand/brand 2"]);
    // One undo step puts the list back as it was.
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(order(&s, "Tokens"), ["space/brand", "space/sm", "md", "brand/brand"]);
}

#[test]
fn moving_a_variable_refuses_what_it_cannot_place() {
    let mut s = Session::new();
    tokens(&mut s);
    ex(&mut s, "variable.createCollection", json!({"name": "Other"}));
    ex(&mut s, "variable.create", json!({"collection": "Other", "name": "gap", "type": "number", "value": 4}));
    // `before` must be a variable of the same collection.
    assert!(s.execute("variable.move", &json!({"variable": "brand", "before": "gap"})).is_err());
    assert!(s.execute("variable.move", &json!({"variable": "brand", "before": "no such variable"})).is_err());
    assert!(s.execute("variable.move", &json!({"variable": "no such variable"})).is_err());
    assert!(s.execute("variable.move", &json!({"variable": "brand", "group": 3})).is_err());
    assert_eq!(order(&s, "Tokens"), ["space/sm", "space/md", "brand"], "refused moves change nothing");
}

#[test]
fn moving_a_variable_before_another_by_id_puts_it_there() {
    let mut s = Session::new();
    tokens(&mut s);
    let ids: Vec<String> = s.doc().unwrap().variables.variables.iter().map(|v| v.id.clone()).collect();
    eprintln!("DBG ids={ids:?}");
    // space/sm, space/md, brand → move space/md before space/sm, by id.
    ex(&mut s, "variable.move", json!({"variable": ids[1], "before": ids[0]}));
    assert_eq!(order(&s, "Tokens"), ["space/md", "space/sm", "brand"], "by id, before the first");
}
