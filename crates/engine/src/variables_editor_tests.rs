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
fn duplicate_by_id_copies_that_variable_even_when_an_earlier_one_is_named_like_its_id() {
    let mut s = Session::new();
    tokens(&mut s);
    let brand = s.doc().unwrap().variables.get("brand").unwrap().id.clone();
    // The first variable in the list takes the later one's id as its name.
    ex(&mut s, "variable.update", json!({"variable": "space/sm", "name": brand.clone()}));
    let id = ex(&mut s, "variable.duplicate", json!({"variable": brand.clone()}))["id"].as_str().unwrap().to_string();
    let vars = &s.doc().unwrap().variables;
    assert_eq!(vars.get(&id).unwrap().name, "brand copy");
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

#[test]
fn stored_variable_ids_detach_deleted_aliases_per_mode() {
    for collection in [false, true] {
        let mut s = Session::new();
        for name in ["Base", "Theme"] {
            ex(&mut s, "variable.createCollection", json!({"name": name, "modes": ["Light", "Dark"]}));
        }
        let shadow = ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "shadow", "type": "color", "value": "#FF0000"}));
        let surface = ex(
            &mut s,
            "variable.create",
            json!({"collection": "Base", "name": "surface", "type": "color", "values": {"Light": "#0D99FF", "Dark": "#000000"}}),
        );
        // Use a different collection only when deleting the whole target collection.
        let home = if collection { "Theme" } else { "Base" };
        let alias = ex(&mut s, "variable.create", json!({"collection": home, "name": "alias", "type": "color", "value": {"alias": surface["id"]}}));
        ex(&mut s, "variable.update", json!({"variable": shadow["id"], "name": surface["id"]}));
        let aid = alias["id"].as_str().unwrap();
        let before = s.doc().unwrap().variables.get_by_id(aid).unwrap().clone();
        let expected: std::collections::BTreeMap<_, _> = before
            .values
            .keys()
            .map(|m| {
                let modes = std::collections::BTreeMap::from([(before.collection.clone(), m.clone())]);
                (m.clone(), s.doc().unwrap().variables.resolve(aid, &modes).unwrap())
            })
            .collect();
        if collection {
            ex(&mut s, "variable.deleteCollection", json!({"collection": "Base"}));
        } else {
            ex(&mut s, "variable.delete", json!({"variable": "surface"}));
        }
        assert_eq!(s.doc().unwrap().variables.get_by_id(aid).unwrap().values, expected);
    }
}

#[test]
fn stored_variable_ids_keep_collection_edits_on_the_resolved_collection() {
    let mut s = Session::new();
    let shadow = ex(&mut s, "variable.createCollection", json!({"name": "Shadow", "modes": ["Only"]}));
    let theme = ex(&mut s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    let surface = ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "surface", "type": "number", "value": 1}));
    ex(&mut s, "variable.renameCollection", json!({"collection": shadow["id"], "name": theme["id"]}));
    ex(&mut s, "variable.set", json!({"variable": "surface", "mode": "Dark", "value": 7}));
    let doc = s.doc().unwrap();
    let coll = doc.variables.collection_by_id(theme["id"].as_str().unwrap()).unwrap();
    let mode = coll.find_mode("Dark").unwrap().id.clone();
    let modes = std::collections::BTreeMap::from([(coll.id.clone(), mode)]);
    assert_eq!(doc.variables.resolve(surface["id"].as_str().unwrap(), &modes), Some(figcraft_doc::VariableValue::Number(7.0)));

    // Mode commands resolve the user's collection argument before checking its stored id.
    let doc = s.doc_mut().unwrap();
    let mut extended = doc.variables.collection_by_id(theme["id"].as_str().unwrap()).unwrap().clone();
    extended.id = "extended".into();
    extended.name = "Extended".into();
    extended.extends = Some(theme["id"].as_str().unwrap().into());
    doc.variables.collections.push(extended);
    doc.variables.collections[0].name = "extended".into();
    let before = s.doc().unwrap().clone();
    assert!(s.execute("variable.removeMode", &json!({"collection": "Extended", "mode": "Dark"})).is_err());
    assert_eq!(s.doc().unwrap(), &before);
}

/// The names of collection `c`'s variables in their stored order.
fn names_in(s: &Session, c: &str) -> Vec<String> {
    let vars = &s.doc().unwrap().variables;
    let cid = vars.collection(c).unwrap().id.clone();
    vars.variables.iter().filter(|v| v.collection == cid).map(|v| v.name.clone()).collect()
}

#[test]
fn moving_variables_reorders_their_collection_only_and_undoes() {
    let mut s = Session::new();
    tokens(&mut s);
    ex(&mut s, "variable.createCollection", json!({"name": "Other"}));
    ex(&mut s, "variable.create", json!({"collection": "Other", "name": "gap", "type": "number", "value": 4}));
    ex(&mut s, "variable.create", json!({"collection": "Tokens", "name": "space/lg", "type": "number", "value": 24}));
    let before = names_in(&s, "Tokens");
    ex(&mut s, "variable.move", json!({"variables": ["space/lg", "brand"], "before": "space/sm"}));
    let after = names_in(&s, "Tokens");
    let at = |list: &[String], n: &str| list.iter().position(|x| x == n).unwrap();
    // The moved ones come first, keeping their own order; the rest keep theirs.
    assert!(at(&after, "brand") < at(&after, "space/lg") && at(&after, "space/lg") < at(&after, "space/sm"));
    assert!(at(&after, "space/sm") < at(&after, "space/md"));
    assert_eq!(names_in(&s, "Other"), ["gap"]);
    // Without `before` they go last; a variable of another collection is refused.
    ex(&mut s, "variable.move", json!({"variables": ["space/sm"]}));
    assert_eq!(names_in(&s, "Tokens").last().map(String::as_str), Some("space/sm"));
    assert!(s.execute("variable.move", &json!({"variables": ["brand"], "before": "gap"})).is_err());
    assert!(s.execute("variable.move", &json!({"variables": ["brand", "gap"]})).is_err());
    ex(&mut s, "edit.undo", json!({}));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(names_in(&s, "Tokens"), before);
}

#[test]
fn sorting_orders_numbers_by_value_within_the_group_asked() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Palette"}));
    for n in ["indigo/500", "indigo/50", "zinc", "indigo/950", "indigo/100", "amber"] {
        ex(&mut s, "variable.create", json!({"collection": "Palette", "name": n, "type": "color", "value": "#336699"}));
    }
    ex(&mut s, "variable.sort", json!({"collection": "Palette", "group": "indigo"}));
    let names = names_in(&s, "Palette");
    let shades: Vec<&str> = names.iter().map(String::as_str).filter(|n| n.starts_with("indigo/")).collect();
    assert_eq!(shades, ["indigo/50", "indigo/100", "indigo/500", "indigo/950"]);
    // Variables outside the group stay where they were.
    assert_eq!(names.iter().position(|n| n == "zinc"), Some(2));
    ex(&mut s, "variable.sort", json!({"collection": "Palette", "descending": true}));
    let names = names_in(&s, "Palette");
    assert_eq!(names.first().map(String::as_str), Some("zinc"));
    assert_eq!(names.last().map(String::as_str), Some("amber"));
    assert!(names.iter().position(|n| n == "indigo/950") < names.iter().position(|n| n == "indigo/50"));
    assert!(s.execute("variable.sort", &json!({"collection": "Palette", "group": "teal"})).is_err());
}
