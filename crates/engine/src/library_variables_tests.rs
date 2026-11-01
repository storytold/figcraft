//! Library variables: publishing collections (hidden variables, versions), binding, aliasing and
//! explicit modes from a library, read-only copies, updates, components that bind variables, and
//! files that bring copies along.

use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
use crate::library::*;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    match s.execute(cmd, &p) {
        Ok(v) => v,
        Err(e) => panic!("{cmd} {p}: {e}"),
    }
}

fn idv(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn fill(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

fn healthy(s: &Session) {
    let doc = s.doc().unwrap();
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());
}

/// A scratch directory for one test.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-libvars-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn path_str(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

/// A library file's variables: "Theme" (Light/Dark) with `primary` (blue/white), `radius` (4),
/// `accent` aliasing `base` (hidden from publishing), and a private "_Scratch" collection.
fn theme(s: &mut Session) {
    ex(s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "primary", "type": "color", "values": {"Light": "#0000ff", "Dark": "#ffffff"}}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "radius", "type": "number", "value": 4}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "base", "type": "color", "value": "#00ff00"}));
    ex(s, "variable.update", json!({"variable": "base", "hiddenFromPublishing": true}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "accent", "type": "color", "value": "#000000"}));
    ex(s, "variable.set", json!({"variable": "accent", "value": {"alias": "base"}}));
    ex(s, "variable.createCollection", json!({"name": "_Scratch"}));
    ex(s, "variable.create", json!({"collection": "_Scratch", "name": "tmp", "type": "number", "value": 1}));
}

/// Publish the active document and save it to `path`; returns the library id.
fn publish_to(s: &mut Session, path: &str) -> String {
    let r = ex(s, "library.publish", json!({}));
    ex(s, "file.saveAs", json!({"path": path}));
    r["id"].as_str().unwrap().to_string()
}

fn republish(lib: &mut Session) {
    ex(lib, "library.publish", json!({}));
    ex(lib, "file.save", json!({}));
}

/// The local copy of library variable `name`.
fn copy_of(s: &Session, name: &str) -> Variable {
    let doc = s.doc().unwrap();
    doc.variables.variables.iter().find(|v| v.name == name && is_remote_variable(doc, &v.id)).cloned().unwrap()
}

#[test]
fn publishing_collections_hides_and_versions_variables() {
    let mut s = Session::new();
    theme(&mut s);
    let r = ex(&mut s, "library.publish", json!({}));
    assert_eq!((r["collections"].as_u64(), r["variables"].as_u64()), (Some(1), Some(3)), "{r}");
    let lib = s.doc().unwrap().published.clone().unwrap();
    let m = &lib.manifest;
    let var = |name: &str| m.variables.iter().find(|v| v.name == name).cloned();
    assert!(m.collections.iter().all(|c| c.name != "_Scratch"), "names starting with _ stay private");
    assert!(var("primary").is_some_and(|v| v.published && v.version == 1 && v.kind == "color"));
    // A variable hidden from publishing stays out of the offer, but comes along for the alias.
    assert!(var("base").is_some_and(|v| !v.published));
    assert!(var("tmp").is_none());
    assert_eq!(lib.doc.variables.variables.len(), 4);
    // Saved and read back, the manifest and snapshot are the same.
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let read = figcraft_format::read_library(std::io::Cursor::new(&bytes)).unwrap().unwrap();
    assert_eq!(read.manifest, *m);
    assert_eq!(read.doc.variables, lib.doc.variables);

    // Nothing changed: refused. A change bumps only the variable it touches.
    assert!(s.execute("library.publish", &json!({})).unwrap_err().to_string().contains("no changes"));
    ex(&mut s, "variable.set", json!({"variable": "radius", "value": 8}));
    assert_eq!(ex(&mut s, "library.publish", json!({}))["version"], 2);
    let m = s.doc().unwrap().published.clone().unwrap().manifest.clone();
    let version = |name: &str| m.variables.iter().find(|v| v.name == name).map(|v| v.version);
    assert_eq!((version("radius"), version("primary")), (Some(2), Some(1)));
    assert_eq!(m.collections.first().map(|c| c.version), Some(1), "the collection itself didn't change");
    // A new mode changes the collection.
    ex(&mut s, "variable.addMode", json!({"collection": "Theme", "name": "Dim"}));
    ex(&mut s, "library.publish", json!({}));
    let m = s.doc().unwrap().published.clone().unwrap().manifest.clone();
    assert_eq!(m.collections.first().map(|c| c.version), Some(3));
    // Hiding a published variable is a change; collections can be included and excluded by name.
    ex(&mut s, "variable.update", json!({"variable": "radius", "hiddenFromPublishing": true}));
    let r = ex(&mut s, "library.publish", json!({}));
    assert_eq!(r["variables"], 2);
    let r = ex(&mut s, "library.publish", json!({"include": ["_Scratch"]}));
    assert_eq!((r["collections"].as_u64(), r["variables"].as_u64()), (Some(1), Some(1)));
    ex(&mut s, "library.publish", json!({"include": ["Theme", "_Scratch"]}));
    let r = ex(&mut s, "library.publish", json!({"exclude": ["_Scratch"]}));
    assert_eq!(r["collections"], 1, "the default follows the last publish");
    let list = ex(&mut s, "library.list", json!({"assets": true}));
    assert_eq!(list["publishable"]["collections"].as_array().map(Vec::len), Some(2), "{list}");
    // Variables alone can be published; nothing at all can't.
    let mut empty = Session::new();
    assert!(empty.execute("library.publish", &json!({})).is_err());
}

/// `bind_alias_mode_read_only_and_updates`: every edit of a copy's variable or collection is refused.
fn assert_copies_read_only(s: &mut Session, local: &str) {
    let coll = copy_of(s, "primary").collection;
    for (cmd, p) in [
        ("variable.set", json!({"variable": local, "value": "#ff0000"})),
        ("variable.update", json!({"variable": local, "name": "mine"})),
        ("variable.delete", json!({"variable": local})),
        ("variable.duplicate", json!({"variable": local})),
        ("variable.create", json!({"collection": coll, "name": "extra", "type": "number", "value": 1})),
        ("variable.addMode", json!({"collection": coll, "name": "Mine"})),
        ("variable.renameMode", json!({"collection": coll, "mode": "Dark", "name": "Night"})),
        ("variable.renameCollection", json!({"collection": coll, "name": "Mine"})),
        ("variable.deleteCollection", json!({"collection": coll})),
    ] {
        let e = s.execute(cmd, &p).unwrap_err();
        assert!(e.to_string().contains("can't be edited"), "{cmd}: {e}");
    }
}

/// `bind_alias_mode_read_only_and_updates`: saved and reopened, the file renders without the library.
fn reopened_without_library(s: &mut Session, d: &std::path::Path, lib_path: &str, lid: &str, outer: NodeId) {
    // Saved and reopened, the copies, records and bindings stay; the file renders without the library.
    let user_path = path_str(&d.join("app.figcraft"));
    ex(s, "file.saveAs", json!({"path": user_path}));
    std::fs::remove_file(lib_path).unwrap();
    let mut t = Session::empty();
    ex(&mut t, "file.open", json!({"path": user_path}));
    assert_eq!(t.doc().unwrap().libraries, s.doc().unwrap().libraries);
    assert_eq!(fill(&t, outer), "FF0000");
    assert!(t.execute("library.bindVariable", &json!({"library": lid, "variable": "primary", "field": "fill", "ids": [outer]})).is_err());
}

/// `bind_alias_mode_read_only_and_updates`: the library changes its variables; the copies, bindings and alias follow.
fn library_update_follows(s: &mut Session, lib: &mut Session, local: &str, [inner, outer]: [NodeId; 2]) {
    let modes = Default::default();
    // The library changes its variables and publishes again.
    ex(lib, "variable.set", json!({"variable": "primary", "mode": "Light", "value": "#ff0000"}));
    ex(lib, "variable.set", json!({"variable": "primary", "mode": "Dark", "value": "#222222"}));
    ex(lib, "variable.set", json!({"variable": "base", "value": "#00ffff"}));
    ex(lib, "variable.renameMode", json!({"collection": "Theme", "mode": "Dark", "name": "Night"}));
    republish(lib);
    let u = ex(s, "library.updates", json!({}));
    assert_eq!(u["count"], 3, "primary, base and the collection: {u}");
    assert_eq!(u["libraries"][0]["variables"].as_array().map(Vec::len), Some(2));
    let r = ex(s, "library.acceptUpdates", json!({}));
    assert_eq!((r["variables"].as_u64(), r["collections"].as_u64()), (Some(2), Some(1)), "{r}");
    // Same copies (same ids), new values: the bound layers and the alias follow.
    assert_eq!(copy_of(s, "primary").id, local);
    assert_eq!((fill(s, inner), fill(s, outer)), ("222222".into(), "FF0000".into()));
    assert!(
        matches!(s.doc().unwrap().variables.resolve(&s.doc().unwrap().variables.get("brand").unwrap().id, &modes), Some(VariableValue::Color(c)) if c.to_hex() == "00FFFF")
    );
    assert_eq!(s.doc().unwrap().variables.collections[0].modes[1].name, "Night");
    assert_eq!(ex(s, "library.updates", json!({}))["count"], 0);
    // Undo takes the update back.
    ex(s, "edit.undo", json!({}));
    assert_eq!(fill(s, outer), "0000FF");
    ex(s, "edit.redo", json!({}));
    healthy(s);
}

#[test]
fn bind_alias_mode_read_only_and_updates() {
    let d = dir("flow");
    let lib_path = path_str(&d.join("theme.figcraft"));
    let mut lib = Session::new();
    theme(&mut lib);
    let lid = publish_to(&mut lib, &lib_path);

    let mut s = Session::new();
    let added = ex(&mut s, "library.add", json!({"path": lib_path}));
    assert_eq!(added["variables"].as_array().map(Vec::len), Some(3), "{added}");
    assert_eq!(added["collections"][0]["modes"].as_array().map(Vec::len), Some(2));
    let frame = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 200})));
    let inner = idv(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20, "parent": frame})));
    let outer = idv(&ex(&mut s, "shape.rectangle", json!({"x": 300, "y": 0, "width": 20, "height": 20})));

    // Binding copies the variable in (linked, read-only) and the layers take its value.
    let r = ex(&mut s, "library.bindVariable", json!({"library": lid, "variable": "primary", "field": "fill", "ids": [inner, outer]}));
    let local = r["variable"].as_str().unwrap().to_string();
    assert_eq!((fill(&s, inner), fill(&s, outer)), ("0000FF".into(), "0000FF".into()));
    ex(&mut s, "library.bindVariable", json!({"library": lid, "variable": "radius", "field": "cornerRadius", "ids": [outer]}));
    // Undo and redo (as commands, like the menus) take a copy back and bring it again.
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().variables.variables.len(), 1);
    ex(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc().unwrap().variables.variables.len(), 2);
    {
        let doc = s.doc().unwrap();
        assert!(is_remote_variable(doc, &local));
        assert_eq!(doc.variables.collections.len(), 1, "one copy of the collection for both");
        assert!(local_collections(doc).next().is_none(), "library copies aren't local collections");
        assert!(publishable(doc).collections.is_empty());
    }
    // Binding again reuses the copy; hidden or unknown variables can't be bound.
    ex(&mut s, "library.bindVariable", json!({"library": lid, "variable": "primary", "field": "fill", "ids": [outer]}));
    assert_eq!(s.doc().unwrap().variables.variables.len(), 2);
    for v in ["base", "tmp", "nope"] {
        assert!(s.execute("library.bindVariable", &json!({"library": lid, "variable": v, "field": "fill", "ids": [outer]})).is_err(), "{v}");
    }

    // Explicit mode: the frame shows the library collection's Dark mode.
    let r = ex(&mut s, "library.setVariableMode", json!({"library": lid, "collection": "Theme", "mode": "Dark", "ids": [frame]}));
    assert_eq!(r["collection"].as_str(), Some(s.doc().unwrap().variables.collections[0].id.as_str()));
    assert_eq!((fill(&s, inner), fill(&s, outer)), ("FFFFFF".into(), "0000FF".into()));
    assert!(s.execute("library.setVariableMode", &json!({"library": lid, "collection": "Theme", "mode": "Nope"})).is_err());
    assert!(s.execute("library.setVariableMode", &json!({"library": lid, "collection": "_Scratch", "mode": "Mode 1"})).is_err());

    // A local variable aliases a library one (copied in with the hidden variable it aliases).
    ex(&mut s, "variable.createCollection", json!({"name": "App"}));
    ex(&mut s, "variable.create", json!({"collection": "App", "name": "brand", "type": "color", "value": "#123456"}));
    ex(&mut s, "variable.create", json!({"collection": "App", "name": "gap", "type": "number", "value": 2}));
    ex(&mut s, "library.aliasVariable", json!({"library": lid, "alias": "accent", "variable": "brand"}));
    let modes = Default::default();
    assert!(
        matches!(s.doc().unwrap().variables.resolve(&s.doc().unwrap().variables.get("brand").unwrap().id, &modes), Some(VariableValue::Color(c)) if c.to_hex() == "00FF00")
    );
    assert!(s.execute("library.aliasVariable", &json!({"library": lid, "alias": "primary", "variable": "gap"})).is_err(), "types differ");
    assert!(s.execute("library.aliasVariable", &json!({"library": lid, "alias": "primary", "variable": local})).is_err(), "copies are read-only");

    // The copies are read-only.
    assert_copies_read_only(&mut s, &local);
    // Local edits still work.
    ex(&mut s, "variable.set", json!({"variable": "gap", "value": 3}));
    healthy(&s);

    library_update_follows(&mut s, &mut lib, &local, [inner, outer]);

    reopened_without_library(&mut s, &d, &lib_path, &lid, outer);

    // Removing the library keeps the copies in use and drops the rest.
    ex(&mut s, "variable.bind", json!({"ids": [outer], "field": "cornerRadius", "variable": null}));
    ex(&mut s, "library.remove", json!({"id": lid}));
    let doc = s.doc().unwrap();
    let names: Vec<&str> = doc.variables.variables.iter().filter(|v| is_remote_variable(doc, &v.id)).map(|v| v.name.as_str()).collect();
    assert_eq!(names.len(), 3, "primary (bound), accent and base (aliased): {names:?}");
    assert!(!names.contains(&"radius"));
    assert_eq!(doc.libraries.collections.len(), 1);
    healthy(&s);
    let _ = std::fs::remove_dir_all(&d);
}

/// A library's colour calculations come in reading this file's copies of their operands (the
/// library's ids mean nothing here), keep following the library's updates, and keep the copies they
/// read when the library is removed.
#[test]
fn library_colour_calculations_read_the_copies() {
    let d = dir("calc");
    let lib_path = path_str(&d.join("theme.figcraft"));
    let mut lib = Session::new();
    theme(&mut lib);
    ex(
        &mut lib,
        "variable.create",
        json!({"collection": "Theme", "name": "hover", "type": "color", "value": {"alpha": {"variable": "base", "opacity": 0.5}}}),
    );
    let tint = json!({"mix": {"from": {"variable": "primary"}, "to": {"literal": "#ffffff"}, "t": 0.5}});
    ex(&mut lib, "variable.create", json!({"collection": "Theme", "name": "tint", "type": "color", "value": tint}));
    let lid = publish_to(&mut lib, &lib_path);
    let lib_ids: Vec<String> = lib.doc().unwrap().variables.variables.iter().map(|v| v.id.clone()).collect();

    let mut s = Session::new();
    // Local variables first, so the copies get ids the library's don't share.
    ex(&mut s, "variable.createCollection", json!({"name": "App"}));
    for name in ["one", "two", "three", "four", "five", "six", "seven"] {
        ex(&mut s, "variable.create", json!({"collection": "App", "name": name, "type": "number", "value": 1}));
    }
    ex(&mut s, "library.add", json!({"path": lib_path}));
    let a = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    let b = idv(&ex(&mut s, "shape.rectangle", json!({"x": 20, "y": 0, "width": 10, "height": 10})));
    ex(&mut s, "library.bindVariable", json!({"library": lid, "variable": "hover", "field": "fill", "ids": [a]}));
    ex(&mut s, "library.bindVariable", json!({"library": lid, "variable": "tint", "field": "fill", "ids": [b]}));
    let shown = |s: &Session, id: NodeId| Some(crate::paint_tests::fill(s, id).to_css_hex());
    assert_eq!((shown(&s, a), shown(&s, b)), (Some("#00FF0080".into()), Some("#8080FF".into())));
    let (base, primary) = (copy_of(&s, "base").id, copy_of(&s, "primary").id);
    assert!(!lib_ids.contains(&base) && !lib_ids.contains(&primary), "copies have ids of their own");
    let reads = |name: &str| copy_of(&s, name).values.values().flat_map(|v| v.references().into_iter().cloned()).collect::<Vec<_>>();
    assert_eq!(reads("hover"), [base.clone(), base]);
    assert_eq!(reads("tint"), [primary.clone(), primary.clone()]);

    ex(&mut lib, "variable.set", json!({"variable": "base", "value": "#ff0000"}));
    ex(&mut lib, "variable.set", json!({"variable": "primary", "mode": "Light", "value": "#000000"}));
    republish(&mut lib);
    ex(&mut s, "library.acceptUpdates", json!({}));
    assert_eq!((shown(&s, a), shown(&s, b)), (Some("#FF000080".into()), Some("#808080".into())));

    // A local calculation is the only reader of `primary` once the layers let go of the library.
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "App", "name": "wash", "type": "color", "value": {"mix": {"from": {"variable": primary}, "to": {"literal": "#ffffff"}, "t": 0.5}}}),
    );
    ex(&mut s, "variable.bind", json!({"ids": [a, b], "field": "fill", "variable": null}));
    ex(&mut s, "library.remove", json!({"id": lid}));
    let doc = s.doc().unwrap();
    assert!(doc.variables.get_by_id(&primary).is_some(), "the copy a local calculation reads stays");
    let wash = doc.variables.get("wash").unwrap();
    let Some(VariableValue::Color(c)) = doc.variables.resolve(&wash.id, &Default::default()) else { panic!("wash resolves to a colour") };
    assert_eq!(c.to_rgba8(), [128, 128, 128, 255]);
    healthy(&s);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn library_components_keep_their_variable_bindings() {
    let d = dir("components");
    let lib_path = path_str(&d.join("kit.figcraft"));
    let mut lib = Session::new();
    theme(&mut lib);
    let f = idv(&ex(&mut lib, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 40, "name": "Chip"})));
    ex(&mut lib, "variable.bind", json!({"ids": [f], "field": "fill", "variable": "primary"}));
    ex(&mut lib, "variable.bind", json!({"ids": [f], "field": "cornerRadius", "variable": "tmp"}));
    ex(&mut lib, "variable.setMode", json!({"ids": [f], "collection": "Theme", "mode": "Dark"}));
    let chip = idv(&ex(&mut lib, "component.create", json!({"ids": [f]})));
    let lid = publish_to(&mut lib, &lib_path);
    // The private collection's variable comes along (not offered) for the component.
    let m = lib.doc().unwrap().published.clone().unwrap().manifest.clone();
    assert!(m.variables.iter().any(|v| v.name == "tmp" && !v.published));

    let mut s = Session::new();
    ex(&mut s, "library.add", json!({"path": lib_path}));
    let inst = idv(&ex(&mut s, "library.place", json!({"library": lid, "component": "Chip", "x": 0, "y": 0})));
    assert_eq!(fill(&s, inst), "FFFFFF", "the component's own mode comes along");
    {
        let doc = s.doc().unwrap();
        let node = doc.node(inst).unwrap();
        let bound = node.bound.get("fill").unwrap();
        assert!(is_remote_variable(doc, bound), "bound to the copy, not to a variable of the file");
        assert!(node.var_modes.keys().all(|c| is_remote_collection(doc, c)));
        assert_eq!(doc.libraries.variables.len(), 2);
    }
    // A value of a mode the component doesn't show: only the variable changes.
    ex(&mut lib, "variable.set", json!({"variable": "primary", "mode": "Light", "value": "#444444"}));
    republish(&mut lib);
    let version = |lib: &Session| lib.doc().unwrap().published.as_ref().and_then(|p| p.manifest.component(chip)).map(|a| a.version);
    assert_eq!(version(&lib), Some(1), "the component didn't change");
    assert_eq!(ex(&mut s, "library.updates", json!({}))["count"], 1);
    ex(&mut s, "library.acceptUpdates", json!({}));
    assert_eq!(fill(&s, inst), "FFFFFF");
    let light = copy_of(&s, "primary").values.values().next().cloned();
    assert!(matches!(light, Some(VariableValue::Color(c)) if c.to_hex() == "444444"));
    // The value it shows: the component (its resolved fill) and the variable update, the instance follows.
    ex(&mut lib, "variable.set", json!({"variable": "primary", "mode": "Dark", "value": "#333333"}));
    republish(&mut lib);
    assert_eq!(version(&lib), Some(3));
    assert_eq!(ex(&mut s, "library.updates", json!({}))["count"], 2);
    ex(&mut s, "library.acceptUpdates", json!({}));
    assert_eq!(fill(&s, inst), "333333");
    healthy(&s);

    // Another file importing these pages brings the copies and their records along.
    let user_path = path_str(&d.join("user.figcraft"));
    ex(&mut s, "file.saveAs", json!({"path": user_path}));
    let mut t = Session::new();
    ex(&mut t, "file.import", json!({"path": user_path}));
    let doc = t.doc().unwrap();
    assert_eq!(doc.libraries.variables.len(), 2);
    assert_eq!(doc.libraries.collections.len(), 2);
    assert!(local_collections(doc).next().is_none());
    healthy(&t);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn library_variable_commands_fail_cleanly() {
    let mut s = Session::new();
    for cmd in ["library.importVariable", "library.bindVariable", "library.aliasVariable", "library.setVariableMode"] {
        for p in [json!({}), json!({"library": 1}), json!({"library": "x", "variable": [], "alias": null, "collection": {}, "mode": 3})] {
            assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
        }
    }
    assert!(s.doc().unwrap().libraries.is_empty());
}

#[test]
fn library_style_bindings_ignore_unrelated_local_variables() {
    let d = dir("style-bound");
    let path = path_str(&d.join("lib.figcraft"));
    let mut lib = Session::new();
    ex(&mut lib, "variable.createCollection", json!({"name": "Tokens"}));
    let var = ex(&mut lib, "variable.create", json!({"collection": "Tokens", "name": "Brand", "type": "color", "value": "#0000ff"}))["id"].clone();
    let style = ex(&mut lib, "style.create", json!({"type": "paint", "name": "Brand", "value": "#0000ff"}))["id"].clone();
    ex(&mut lib, "style.bind", json!({"style": style, "field": "fill", "variable": var}));
    let lid = publish_to(&mut lib, &path);

    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Local"}));
    let local = ex(&mut s, "variable.create", json!({"collection": "Local", "name": "Unrelated", "type": "color", "value": "#ff0000"}))["id"].clone();
    assert_eq!(local, var, "the ids must collide for this test");
    let r = idv(&ex(&mut s, "shape.rectangle", json!({"width": 40, "height": 40})));
    ex(&mut s, "library.add", json!({"path": path}));
    ex(&mut s, "library.applyStyle", json!({"library": lid, "style": "Brand", "id": r}));
    assert_eq!(fill(&s, r), "0000FF");
    healthy(&s);

    // An updated style follows the library's variable, not the local one.
    ex(&mut lib, "variable.set", json!({"variable": "Brand", "value": "#00ffff"}));
    republish(&mut lib);
    ex(&mut s, "library.acceptUpdates", json!({}));
    assert_eq!(fill(&s, r), "00FFFF");
    healthy(&s);
    let _ = std::fs::remove_dir_all(&d);
}

/// A library "Modal" holding a Button instance whose icon copy is on mode Dark: (the Button main,
/// the icon copy in Modal).
fn nested_mode_kit(lib: &mut Session) -> (NodeId, NodeId) {
    theme(lib);
    let f = idv(&ex(lib, "frame.create", json!({"x": 0, "y": 0, "width": 20, "height": 20, "name": "Icon"})));
    ex(lib, "variable.bind", json!({"ids": [f], "field": "fill", "variable": "primary"}));
    let icon = idv(&ex(lib, "component.create", json!({"ids": [f]})));
    let b = idv(&ex(lib, "frame.create", json!({"x": 100, "y": 0, "width": 60, "height": 40, "name": "Button"})));
    ex(lib, "instance.create", json!({"component": icon, "x": 10, "y": 10, "parent": b}));
    let button = idv(&ex(lib, "component.create", json!({"ids": [b]})));
    let m = idv(&ex(lib, "frame.create", json!({"x": 300, "y": 0, "width": 200, "height": 100, "name": "Modal"})));
    let close = idv(&ex(lib, "instance.create", json!({"component": button, "x": 10, "y": 10, "parent": m})));
    let nested = lib.doc().unwrap().node(close).unwrap().children[0];
    ex(lib, "variable.setMode", json!({"ids": [nested], "collection": "Theme", "mode": "Dark"}));
    ex(lib, "component.create", json!({"ids": [m]}));
    (button, nested)
}

/// The icon inside the Button inside library Modal instance `inst`.
fn modal_icon(s: &Session, inst: NodeId) -> NodeId {
    let doc = s.doc().unwrap();
    doc.node(doc.node(inst).unwrap().children[0]).unwrap().children[0]
}

#[test]
fn modes_set_inside_nested_instances_come_along() {
    let d = dir("nested-modes");
    let lib_path = path_str(&d.join("kit.figcraft"));
    let mut lib = Session::new();
    let (button, _) = nested_mode_kit(&mut lib);
    let lid = publish_to(&mut lib, &lib_path);

    let mut s = Session::new();
    ex(&mut s, "library.add", json!({"path": lib_path}));
    let inst = idv(&ex(&mut s, "library.place", json!({"library": lid, "component": "Modal", "x": 0, "y": 0})));
    let copy = |s: &Session| modal_icon(s, inst);
    assert_eq!(fill(&s, copy(&s)), "FFFFFF", "the Dark mode set on the nested icon comes along");
    // An update that rebuilds the Button copies keeps it too.
    ex(&mut lib, "node.set", json!({"ids": [button], "layout": "horizontal"}));
    republish(&mut lib);
    ex(&mut s, "library.acceptUpdates", json!({}));
    assert_eq!(fill(&s, copy(&s)), "FFFFFF");
    healthy(&s);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_library_published_before_modes_were_overrides_keeps_the_modes_set_inside_nested_instances() {
    let d = dir("nested-modes-old");
    let lib_path = path_str(&d.join("kit.figcraft"));
    let mut lib = Session::new();
    let (_, nested) = nested_mode_kit(&mut lib);
    // Published before `modes` was an override group: the copy has its mode but no marker.
    lib.doc_mut().unwrap().node_mut(nested).unwrap().overrides.remove("modes");
    let lid = publish_to(&mut lib, &lib_path);

    let mut s = Session::new();
    ex(&mut s, "library.add", json!({"path": lib_path}));
    let inst = idv(&ex(&mut s, "library.place", json!({"library": lid, "component": "Modal", "x": 0, "y": 0})));
    assert_eq!(fill(&s, modal_icon(&s, inst)), "FFFFFF", "the Dark mode comes along");
    healthy(&s);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn stored_variable_ids_keep_library_alias_dependencies() {
    let mut s = Session::new();
    for name in ["Shadow", "Base", "Theme"] {
        ex(&mut s, "variable.createCollection", json!({"name": name}));
    }
    let shadow = ex(&mut s, "variable.create", json!({"collection": "Shadow", "name": "shadow", "type": "color", "value": "#FF0000"}));
    let surface = ex(&mut s, "variable.create", json!({"collection": "Base", "name": "surface", "type": "color", "value": "#0D99FF"}));
    let alias = ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "alias", "type": "color", "value": {"alias": surface["id"]}}));
    let f = idv(&ex(&mut s, "frame.create", json!({"name": "Chip", "x": 0, "y": 0, "width": 100, "height": 40})));
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "fill", "variable": alias["id"]}));
    let chip = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "variable.update", json!({"variable": shadow["id"], "name": surface["id"]}));
    let doc = s.doc().unwrap();
    let selection = Selection { components: vec![chip], ..Default::default() };
    let lib = publish(doc, None, &selection, None, 0).unwrap();
    let target = surface["id"].as_str().unwrap();
    assert!(lib.doc.variables.get_by_id(target).is_some());
    assert!(lib.doc.variables.get_by_id(shadow["id"].as_str().unwrap()).is_none());
    assert!(lib.doc.variables.collections.iter().any(|c| c.name == "Base"));

    // Import from a snapshot that retains both the dependency and its namesake.
    let lib = publish(doc, None, &publishable(doc), None, 1).unwrap();
    let mut dest = Document::new("consumer");
    let key = alias["id"].as_str().unwrap().to_string();
    let map = import_variables(&mut dest, &lib, &std::collections::BTreeSet::from([key.clone()]), &Default::default(), false);
    assert!(map.variables.contains_key(target));
    let copy = dest.variables.get_by_id(&map.variables[&key]).unwrap();
    assert!(!copy.values.is_empty());
    assert!(copy.values.values().all(|v| *v == VariableValue::Alias(map.variables[target].clone())));
    assert!(dest.variables.collections.iter().any(|c| c.name == "Base"));
}
