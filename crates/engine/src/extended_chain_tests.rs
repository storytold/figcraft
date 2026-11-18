//! Extended collections' tail: variables a library adds to a collection this file extends are an
//! update of their own, and a library collection that is itself an extended collection can be
//! extended again, values flowing down the chain as each file accepts updates. A collection of
//! this file extends too, following its edits at once (and chains of them).

use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn var<'a>(s: &'a Session, collection: &str, name: &str) -> &'a Variable {
    let doc = s.doc().unwrap();
    doc.variables.variables.iter().find(|v| v.collection == collection && v.name == name).unwrap()
}

/// `name`'s value in mode `mode` (by name) of `collection`, as text.
fn value(s: &Session, collection: &str, name: &str, mode: &str) -> String {
    let doc = s.doc().unwrap();
    let m = doc.variables.collection(collection).and_then(|c| c.find_mode(mode)).unwrap().id.clone();
    match var(s, collection, name).values.get(&m) {
        Some(VariableValue::Color(c)) => c.to_hex(),
        Some(VariableValue::Number(n)) => n.to_string(),
        other => format!("{other:?}"),
    }
}

fn healthy(s: &Session) {
    let doc = s.doc().unwrap();
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-extchain-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A library file with "Theme" (Light/Dark: `primary`, `radius`) and "Other" (`size`), published
/// and saved at `path`; returns (session, library id).
fn core(path: &str) -> (Session, String) {
    let mut lib = Session::new();
    ex(&mut lib, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(
        &mut lib,
        "variable.create",
        json!({"collection": "Theme", "name": "primary", "type": "color", "values": {"Light": "#0000ff", "Dark": "#ffffff"}}),
    );
    ex(&mut lib, "variable.create", json!({"collection": "Theme", "name": "radius", "type": "number", "value": 4}));
    ex(&mut lib, "variable.createCollection", json!({"name": "Other"}));
    ex(&mut lib, "variable.create", json!({"collection": "Other", "name": "size", "type": "number", "value": 1}));
    let id = ex(&mut lib, "library.publish", json!({}))["id"].as_str().unwrap().to_string();
    ex(&mut lib, "file.saveAs", json!({"path": path}));
    (lib, id)
}

fn republish(lib: &mut Session) {
    ex(lib, "library.publish", json!({}));
    ex(lib, "file.save", json!({}));
}

#[test]
fn variables_new_to_an_extended_collection_are_an_update_of_their_own() {
    let d = dir("new");
    let path = d.join("core.figcraft").to_string_lossy().into_owned();
    let (mut lib, lid) = core(&path);
    let mut s = Session::new();
    ex(&mut s, "library.add", json!({"path": path}));
    let ext = ex(&mut s, "library.extendCollection", json!({"library": lid, "collection": "Theme"}))["id"].as_str().unwrap().to_string();
    assert_eq!(ex(&mut s, "library.updates", json!({}))["count"], 0);

    // The library only adds a variable to the extended collection (and one to a collection this
    // file doesn't use): that alone is an update, listed by name.
    ex(&mut lib, "variable.create", json!({"collection": "Theme", "name": "gap", "type": "number", "value": 8}));
    ex(&mut lib, "variable.create", json!({"collection": "Other", "name": "unused", "type": "number", "value": 2}));
    republish(&mut lib);
    let u = ex(&mut s, "library.updates", json!({}));
    assert_eq!(u["count"], 1, "{u}");
    let row = &u["libraries"][0]["variables"][0];
    assert_eq!((&row["name"], &row["id"]), (&json!("gap"), &Value::Null), "no copy yet: no id");
    // Accepting brings it in, inheriting; then nothing is pending.
    let done = ex(&mut s, "library.acceptUpdates", json!({}));
    assert_eq!(done["variables"], 1);
    let gap = var(&s, &ext, "gap");
    assert!(gap.extends.is_some() && gap.overrides.is_empty());
    assert_eq!(value(&s, &ext, "gap", "Dark"), "8");
    assert_eq!(ex(&mut s, "library.updates", json!({}))["count"], 0);
    // Docs silent: the variable added to a collection nobody here uses stays out.
    assert!(!s.doc().unwrap().variables.variables.iter().any(|v| v.name == "unused"));
    healthy(&s);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn an_extended_collection_can_be_extended_again() {
    let d = dir("chain");
    let core_path = d.join("core.figcraft").to_string_lossy().into_owned();
    let brand_path = d.join("brand.figcraft").to_string_lossy().into_owned();
    let (mut core_lib, core_id) = core(&core_path);

    // A brand library extends the core theme, overrides a value, adds a variable and publishes.
    let mut brand = Session::new();
    ex(&mut brand, "library.add", json!({"path": core_path}));
    let bext = ex(&mut brand, "library.extendCollection", json!({"library": core_id, "collection": "Theme", "name": "Brand"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let bprimary = var(&brand, &bext, "primary").id.clone();
    ex(&mut brand, "variable.set", json!({"variable": bprimary, "mode": "Light", "value": "#ff0000"}));
    ex(&mut brand, "variable.create", json!({"collection": bext, "name": "logo", "type": "number", "value": 3}));
    let brand_id = ex(&mut brand, "library.publish", json!({}))["id"].as_str().unwrap().to_string();
    ex(&mut brand, "file.saveAs", json!({"path": brand_path}));

    // A product file extends the brand's extended collection: the brand's values, modes and
    // variables come along, and it overrides one of its own.
    let mut s = Session::new();
    ex(&mut s, "library.add", json!({"path": brand_path}));
    let pext = ex(&mut s, "library.extendCollection", json!({"library": brand_id, "collection": "Brand"}))["id"].as_str().unwrap().to_string();
    let modes: Vec<String> = s.doc().unwrap().variables.collection(&pext).unwrap().modes.iter().map(|m| m.name.clone()).collect();
    assert_eq!(modes, ["Light", "Dark"]);
    assert_eq!(value(&s, &pext, "primary", "Light"), "FF0000", "the brand's override is inherited");
    assert_eq!(value(&s, &pext, "primary", "Dark"), "FFFFFF", "the core value is inherited");
    assert_eq!(value(&s, &pext, "logo", "Light"), "3", "the brand's own variable is inherited");
    let radius = var(&s, &pext, "radius").id.clone();
    ex(&mut s, "variable.set", json!({"variable": radius, "mode": "Dark", "value": 12}));
    let r = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"]).unwrap();
    let primary = var(&s, &pext, "primary").id.clone();
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": primary}));

    // The core changes: the brand accepts and republishes, then the product accepts. Values
    // follow down the chain; each level's overrides stay.
    ex(&mut core_lib, "variable.set", json!({"variable": "primary", "mode": "Dark", "value": "#222222"}));
    ex(&mut core_lib, "variable.set", json!({"variable": "radius", "mode": "Light", "value": 6}));
    republish(&mut core_lib);
    ex(&mut brand, "library.acceptUpdates", json!({}));
    assert_eq!(value(&brand, &bext, "primary", "Dark"), "222222");
    assert_eq!(value(&brand, &bext, "primary", "Light"), "FF0000");
    republish(&mut brand);
    assert!(ex(&mut s, "library.updates", json!({}))["count"].as_u64().unwrap() > 0);
    ex(&mut s, "library.acceptUpdates", json!({}));
    assert_eq!(value(&s, &pext, "primary", "Dark"), "222222");
    assert_eq!(value(&s, &pext, "primary", "Light"), "FF0000");
    assert_eq!(value(&s, &pext, "radius", "Light"), "6");
    assert_eq!(value(&s, &pext, "radius", "Dark"), "12", "the product's override stays");
    healthy(&s);

    // Saved and reopened, the chain's end is the same.
    let user = d.join("product.figcraft").to_string_lossy().into_owned();
    ex(&mut s, "file.saveAs", json!({"path": user}));
    let mut t = Session::empty();
    ex(&mut t, "file.open", json!({"path": user}));
    assert_eq!(t.doc().unwrap().variables, s.doc().unwrap().variables);
    std::fs::remove_dir_all(&d).ok();
}

/// "Theme" (Light/Dark: `primary`, `radius`) in this file, extended; returns the extension's id.
fn local_theme(s: &mut Session) -> String {
    ex(s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "primary", "type": "color", "values": {"Light": "#0000ff", "Dark": "#ffffff"}}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "radius", "type": "number", "value": 4}));
    ex(s, "variable.extendCollection", json!({"collection": "Theme"}))["id"].as_str().unwrap().to_string()
}

fn theme_id(s: &Session) -> String {
    s.doc().unwrap().variables.collection("Theme").unwrap().id.clone()
}

#[test]
fn a_collection_of_this_file_extends_and_follows_its_edits() {
    let mut s = Session::new();
    let ext = local_theme(&mut s);
    let theme = theme_id(&s);
    let doc = s.doc().unwrap();
    let c = doc.variables.collection_by_id(&ext).unwrap();
    assert_eq!((c.name.as_str(), c.extends.as_deref()), ("Theme (extended)", Some(theme.as_str())));
    assert_eq!(c.modes, doc.variables.collection_by_id(&theme).unwrap().modes, "the same modes, ids included");
    assert_eq!(value(&s, &ext, "primary", "Dark"), "FFFFFF");
    assert!(var(&s, &ext, "radius").extends.is_some());
    // A second one gets a free name; a name can be given.
    let again = ex(&mut s, "variable.extendCollection", json!({"collection": theme, "name": " Brand "}))["id"].as_str().unwrap().to_string();
    assert_eq!(s.doc().unwrap().variables.collection_by_id(&again).unwrap().name, "Brand");
    let third = ex(&mut s, "variable.extendCollection", json!({"collection": "Theme"}))["id"].as_str().unwrap().to_string();
    assert_eq!(s.doc().unwrap().variables.collection_by_id(&third).unwrap().name, "Theme (extended) 2");

    // Overridden in Light; then the parent changes both modes: Dark follows, Light stays.
    let radius = var(&s, &ext, "radius").id.clone();
    ex(&mut s, "variable.set", json!({"variable": radius, "mode": "Light", "value": 8}));
    let parent_radius = var(&s, &theme, "radius").id.clone();
    ex(&mut s, "variable.set", json!({"variable": parent_radius, "mode": "Light", "value": 6}));
    ex(&mut s, "variable.set", json!({"variable": parent_radius, "mode": "Dark", "value": 10}));
    assert_eq!((value(&s, &ext, "radius", "Light"), value(&s, &ext, "radius", "Dark")), ("8".into(), "10".into()));
    assert_eq!(value(&s, &theme, "radius", "Light"), "6", "the parent keeps its own value");
    // Undo takes the inherited value back with the parent's.
    s.undo().unwrap();
    assert_eq!(value(&s, &ext, "radius", "Dark"), "4");
    s.redo().unwrap();

    // New variables and modes, renames and mode moves arrive at once.
    ex(&mut s, "variable.create", json!({"collection": theme, "name": "gap", "type": "number", "value": 2}));
    ex(&mut s, "variable.addMode", json!({"collection": theme, "name": "Contrast"}));
    ex(&mut s, "variable.update", json!({"variable": parent_radius, "name": "corner"}));
    ex(&mut s, "variable.moveMode", json!({"collection": theme, "mode": "Dark", "to": 0}));
    let doc = s.doc().unwrap();
    assert_eq!(doc.variables.collection_by_id(&ext).unwrap().modes, doc.variables.collection_by_id(&theme).unwrap().modes);
    assert_eq!(value(&s, &ext, "gap", "Contrast"), "2");
    assert_eq!(var(&s, &ext, "corner").id, radius, "renamed, the same variable");
    // Its modes and inherited names stay the parent's to edit.
    assert!(s.execute("variable.addMode", &json!({"collection": ext, "name": "Mine"})).is_err());
    assert!(s.execute("variable.update", &json!({"variable": radius, "name": "mine"})).is_err());
    ex(&mut s, "variable.resetOverride", json!({"variable": radius}));
    assert_eq!(value(&s, &ext, "corner", "Light"), "6");
    healthy(&s);

    // Saved and reopened, still extending.
    let d = dir("local");
    let path = d.join("local.figcraft").to_string_lossy().into_owned();
    ex(&mut s, "file.saveAs", json!({"path": path}));
    let mut t = Session::empty();
    ex(&mut t, "file.open", json!({"path": path}));
    assert_eq!(t.doc().unwrap().variables, s.doc().unwrap().variables);
    std::fs::remove_dir_all(&d).ok();

    // The parent deleted, the extension becomes a plain collection keeping its values.
    ex(&mut s, "variable.deleteCollection", json!({"collection": theme}));
    let doc = s.doc().unwrap();
    assert!(doc.variables.collection_by_id(&ext).unwrap().extends.is_none());
    assert!(doc.variables.variables.iter().filter(|v| v.collection == ext).all(|v| v.extends.is_none() && v.overrides.is_empty()));
    assert_eq!(value(&s, &ext, "corner", "Dark"), "10");
    healthy(&s);
    assert!(s.execute("variable.extendCollection", &json!({"collection": "nope"})).is_err());
}

#[test]
fn an_extended_collection_of_this_file_extends_again() {
    let mut s = Session::new();
    let brand = local_theme(&mut s);
    let theme = theme_id(&s);
    let primary = var(&s, &brand, "primary").id.clone();
    ex(&mut s, "variable.set", json!({"variable": primary, "mode": "Light", "value": "#ff0000"}));
    ex(&mut s, "variable.create", json!({"collection": brand, "name": "logo", "type": "number", "value": 3}));
    let product = ex(&mut s, "variable.extendCollection", json!({"collection": brand}))["id"].as_str().unwrap().to_string();
    assert_eq!(value(&s, &product, "primary", "Light"), "FF0000", "the brand's override is inherited");
    assert!(var(&s, &product, "primary").overrides.is_empty(), "but isn't the product's own override");
    assert_eq!(value(&s, &product, "logo", "Dark"), "3", "the brand's own variable is inherited");
    // The theme changes: values flow down through the brand where nobody overrode them.
    let core = var(&s, &theme, "primary").id.clone();
    ex(&mut s, "variable.set", json!({"variable": core, "mode": "Dark", "value": "#222222"}));
    ex(&mut s, "variable.set", json!({"variable": core, "mode": "Light", "value": "#00ff00"}));
    assert_eq!(value(&s, &product, "primary", "Dark"), "222222");
    assert_eq!(value(&s, &product, "primary", "Light"), "FF0000");
    ex(&mut s, "variable.addMode", json!({"collection": theme, "name": "Contrast"}));
    assert_eq!(s.doc().unwrap().variables.collection_by_id(&product).unwrap().modes.len(), 3);
    healthy(&s);
}

#[test]
fn a_loop_of_extensions_comes_apart_into_plain_collections() {
    let mut s = Session::new();
    let ext = local_theme(&mut s);
    let theme = theme_id(&s);
    // A hostile file: the theme extends its own extension.
    let doc = s.doc_mut().unwrap();
    doc.variables.collections.iter_mut().find(|c| c.id == theme).unwrap().extends = Some(ext.clone());
    crate::cmd::variables::sync_extensions(doc);
    assert!(doc.variables.collections.iter().all(|c| c.extends.is_none()));
    assert!(doc.variables.variables.iter().all(|v| v.extends.is_none()));
    assert_eq!(value(&s, &ext, "primary", "Light"), "0000FF", "values kept");
}
