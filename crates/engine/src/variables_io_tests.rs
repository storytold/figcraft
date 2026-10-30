//! Extended variable collections (overrides, library updates, reset), variables import and
//! export (JSON round trip, merging by name, CSS, Dart, junk), gradient stops bound to variables and
//! variant properties driven by string variables.

use figcraft_color::{Color, ColorStop, Gradient, GradientKind, Paint, PaintKind};
use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
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

fn var<'a>(s: &'a Session, collection: &str, name: &str) -> &'a Variable {
    let doc = s.doc().unwrap();
    doc.variables.variables.iter().find(|v| v.collection == collection && v.name == name).unwrap()
}

fn hex(v: Option<&VariableValue>) -> String {
    match v {
        Some(VariableValue::Color(c)) => c.to_hex(),
        other => format!("{other:?}"),
    }
}

fn fill_hex(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

fn healthy(s: &Session) {
    let doc = s.doc().unwrap();
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());
}

#[test]
fn extended_collections_override_follow_updates_and_reset() {
    let d = std::env::temp_dir().join(format!("figcraft-extvars-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let lib_path = d.join("theme.figcraft").to_string_lossy().into_owned();
    let mut lib = Session::new();
    ex(&mut lib, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(
        &mut lib,
        "variable.create",
        json!({"collection": "Theme", "name": "primary", "type": "color", "values": {"Light": "#0000ff", "Dark": "#ffffff"}}),
    );
    ex(&mut lib, "variable.create", json!({"collection": "Theme", "name": "radius", "type": "number", "value": 4}));
    let lid = ex(&mut lib, "library.publish", json!({}))["id"].as_str().unwrap().to_string();
    ex(&mut lib, "file.saveAs", json!({"path": lib_path}));

    let mut s = Session::new();
    ex(&mut s, "library.add", json!({"path": lib_path}));
    let ext = ex(&mut s, "library.extendCollection", json!({"library": lid, "collection": "Theme"}))["id"].as_str().unwrap().to_string();
    let (coll, parent) = {
        let doc = s.doc().unwrap();
        let c = doc.variables.collections.iter().find(|c| c.id == ext).unwrap().clone();
        let p = doc.variables.collections.iter().find(|p| Some(&p.id) == c.extends.as_ref()).unwrap().clone();
        assert!(crate::library::is_remote_collection(doc, &p.id), "it extends the library's copy");
        (c, p)
    };
    assert_eq!(coll.name, "Theme (extended)");
    assert_eq!(coll.modes, parent.modes, "the same modes");
    let (light, dark) = (coll.modes[0].id.clone(), coll.modes[1].id.clone());
    let primary = var(&s, &ext, "primary").clone();
    assert!(primary.extends.is_some() && primary.overrides.is_empty());
    assert_eq!(hex(primary.values.get(&dark)), "FFFFFF");
    assert_eq!(var(&s, &ext, "radius").values.get(&light), Some(&VariableValue::Number(4.0)));

    // An override in one mode; bound layers see it in that mode.
    ex(&mut s, "variable.set", json!({"variable": primary.id, "mode": "Dark", "value": "#ff0000"}));
    assert_eq!(var(&s, &ext, "primary").overrides.iter().collect::<Vec<_>>(), [&dark]);
    let r = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": primary.id}));
    ex(&mut s, "variable.setMode", json!({"ids": [r], "collection": ext, "mode": "Dark"}));
    assert_eq!(fill_hex(&s, r), "FF0000");
    // Variables of its own are fine; modes and inherited names and variables aren't its to change.
    ex(&mut s, "variable.create", json!({"collection": ext, "name": "extra", "type": "number", "value": 1}));
    for (cmd, p) in [
        ("variable.addMode", json!({"collection": ext, "name": "Dim"})),
        ("variable.renameMode", json!({"collection": ext, "mode": "Dark", "name": "Night"})),
        ("variable.removeMode", json!({"collection": ext, "mode": "Dark"})),
        ("variable.update", json!({"variable": primary.id, "name": "mine"})),
        ("variable.delete", json!({"variable": primary.id})),
        ("variable.resetOverride", json!({"variable": "extra"})),
    ] {
        assert!(s.execute(cmd, &p).is_err(), "{cmd}");
    }
    let dup = ex(&mut s, "variable.duplicate", json!({"variable": primary.id}))["id"].as_str().unwrap().to_string();
    assert!(s.doc().unwrap().variables.get(&dup).is_some_and(|v| v.extends.is_none() && v.overrides.is_empty()), "a copy is its own");

    // The library changes a value and gains a variable: inherited values follow, overrides stay.
    ex(&mut lib, "variable.set", json!({"variable": "primary", "mode": "Light", "value": "#00ff00"}));
    ex(&mut lib, "variable.set", json!({"variable": "primary", "mode": "Dark", "value": "#222222"}));
    ex(&mut lib, "variable.create", json!({"collection": "Theme", "name": "gap", "type": "number", "value": 8}));
    ex(&mut lib, "library.publish", json!({}));
    ex(&mut lib, "file.save", json!({}));
    ex(&mut s, "library.acceptUpdates", json!({}));
    let p = var(&s, &ext, "primary");
    assert_eq!((hex(p.values.get(&light)), hex(p.values.get(&dark))), ("00FF00".into(), "FF0000".into()));
    assert!(var(&s, &ext, "gap").extends.is_some(), "the new library variable is inherited");
    assert_eq!(fill_hex(&s, r), "FF0000");
    // Reset: the inherited value again.
    ex(&mut s, "variable.resetOverride", json!({"variable": primary.id, "mode": "Dark"}));
    assert!(var(&s, &ext, "primary").overrides.is_empty());
    assert_eq!(fill_hex(&s, r), "222222");
    healthy(&s);

    // Saved and reopened, it is the same; removing the library keeps what it extends.
    let user = d.join("app.figcraft").to_string_lossy().into_owned();
    ex(&mut s, "file.saveAs", json!({"path": user}));
    let mut t = Session::empty();
    ex(&mut t, "file.open", json!({"path": user}));
    assert_eq!(t.doc().unwrap().variables, s.doc().unwrap().variables);
    ex(&mut s, "library.remove", json!({"id": lid}));
    assert!(s.doc().unwrap().variables.collections.iter().any(|c| c.id == parent.id));
    healthy(&s);
    assert!(s.execute("library.extendCollection", &json!({"library": lid, "collection": "Theme"})).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

/// A "Theme" collection (Light/Dark) with one variable of each type and an alias.
fn theme(s: &mut Session) {
    ex(s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(
        s,
        "variable.create",
        json!({"collection": "Theme", "name": "color/primary", "type": "color", "values": {"Light": "#0000ff80", "Dark": "#ffffff"}}),
    );
    ex(s, "variable.create", json!({"collection": "Theme", "name": "radius", "type": "number", "values": {"Light": 4, "Dark": 6.5}}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "label", "type": "string", "value": "Say \"hi\""}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "on", "type": "boolean", "value": false}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "accent", "type": "color", "value": "#000000"}));
    ex(s, "variable.set", json!({"variable": "accent", "mode": "Dark", "value": {"alias": "color/primary"}}));
    ex(s, "variable.update", json!({"variable": "radius", "scopes": ["CORNER_RADIUS"], "description": "Corners"}));
}

/// (collection, variable, [(mode, value)], scopes), all by name.
type Row = (String, String, Vec<(String, String)>, Option<Vec<String>>);

/// Collections and variables by name with their values by mode name (aliases by target name).
fn by_name(s: &Session) -> Vec<Row> {
    let vars = &s.doc().unwrap().variables;
    let mut out: Vec<_> = vars
        .variables
        .iter()
        .map(|v| {
            let c = vars.collection(&v.collection).unwrap();
            let values = c
                .modes
                .iter()
                .map(|m| {
                    let val = match v.values.get(&m.id) {
                        Some(VariableValue::Alias(a)) => format!("→ {}", vars.get(a).unwrap().name),
                        Some(x) => format!("{x:?}"),
                        None => "-".into(),
                    };
                    (m.name.clone(), val)
                })
                .collect();
            (c.name.clone(), v.name.clone(), values, v.scopes.clone())
        })
        .collect();
    out.sort();
    out
}

#[test]
fn variables_export_and_import_round_trip() {
    let mut s = Session::new();
    theme(&mut s);
    let text = ex(&mut s, "variable.export", json!({}))["text"].as_str().unwrap().to_string();
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert!(figcraft_format::rest::is_variables_json(&parsed));

    // Into another file: the same collections, variables, values, aliases and scopes.
    let mut t = Session::new();
    let r = ex(&mut t, "variable.import", json!({"variables": text}));
    assert_eq!((r["collections"].as_u64(), r["variables"].as_u64()), (Some(1), Some(5)), "{r}");
    assert_eq!(by_name(&t), by_name(&s));
    assert_eq!(t.doc().unwrap().variables.get("radius").unwrap().description, "Corners");

    // Back into the same file: nothing new, nothing changed; edited values update in place.
    let before = s.doc().unwrap().variables.clone();
    let r = ex(&mut s, "variable.import", json!({"variables": parsed}));
    assert_eq!((r["collections"].as_u64(), r["variables"].as_u64(), r["updated"].as_u64()), (Some(0), Some(0), Some(5)), "{r}");
    assert_eq!(s.doc().unwrap().variables, before);
    let mut edited = parsed.clone();
    let rid = before.get("radius").unwrap().id.clone();
    let light = before.collections[0].modes[0].id.clone();
    edited["meta"]["variables"][&rid]["valuesByMode"][&light] = json!(12);
    edited["meta"]["variableCollections"][&before.collections[0].id]["modes"].as_array_mut().unwrap().push(json!({"modeId": "x", "name": "Dim"}));
    edited["meta"]["variables"][&rid]["valuesByMode"]["x"] = json!(1);
    ex(&mut s, "variable.import", json!({"variables": edited}));
    let radius = s.doc().unwrap().variables.get("radius").unwrap().clone();
    assert_eq!(radius.id, rid, "the same variable (bindings keep working)");
    assert_eq!(radius.values.get(&light), Some(&VariableValue::Number(12.0)));
    let coll = s.doc().unwrap().variables.collections[0].clone();
    assert_eq!(coll.modes.len(), 3, "the new mode is added");
    assert_eq!(radius.values.get(&coll.modes[2].id), Some(&VariableValue::Number(1.0)));
    assert_eq!(s.doc().unwrap().variables.get("on").unwrap().values.len(), 3, "the others get a value in it too");
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().variables, before, "one undo step");

    // CSS: the default mode on :root, the others by attribute; aliases refer to their target.
    let css = ex(&mut s, "variable.export", json!({"format": "css", "collections": ["Theme"]}))["text"].as_str().unwrap().to_string();
    for want in [
        ":root {\n",
        "  --radius: 4px;\n",
        "[data-theme=\"Dark\"] {\n",
        "  --radius: 6.5px;\n",
        "  --accent: var(--color-primary);\n",
        "  --label: \"Say  hi \";\n",
    ] {
        assert!(css.contains(want), "{want:?} in\n{css}");
    }
    // Written to a file and read back from it.
    let path = std::env::temp_dir().join(format!("figcraft-vars-io-{}.json", std::process::id())).to_string_lossy().into_owned();
    ex(&mut s, "variable.export", json!({"path": path}));
    let mut u = Session::new();
    ex(&mut u, "variable.import", json!({"variables": path}));
    assert_eq!(by_name(&u), by_name(&s));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn stored_variable_ids_keep_remapped_import_aliases() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Theme"}));
    let shadow = ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "shadow", "type": "number", "value": 100}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let target = ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "target", "type": "number", "value": 5}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    ex(&mut s, "variable.update", json!({"variable": shadow, "name": target}));
    let imported = json!({"meta": {
        "variableCollections": {"c": {"id": "c", "name": "Theme", "modes": [{"modeId": "m", "name": "Mode 1"}]}},
        "variables": {
            "t": {"id": "t", "name": "target", "variableCollectionId": "c", "resolvedType": "FLOAT", "valuesByMode": {"m": 5}},
            "a": {"id": "a", "name": "alias", "variableCollectionId": "c", "resolvedType": "FLOAT",
                  "valuesByMode": {"m": {"type": "VARIABLE_ALIAS", "id": "t"}}}
        }
    }});
    let result = ex(&mut s, "variable.import", json!({"variables": imported}));
    assert_eq!(result["warnings"], json!([]));
    let vars = &s.doc().unwrap().variables;
    let alias = vars.get("alias").unwrap();
    assert_eq!(alias.values.values().next(), Some(&VariableValue::Alias(target.clone())));
    assert_eq!(vars.resolve(&alias.id, &Default::default()), Some(VariableValue::Number(5.0)));
    // The command's alias argument accepts names, but an exact id comes first (#1613): the
    // variable renamed to target's id doesn't take its place.
    ex(&mut s, "variable.set", json!({"variable": "alias", "value": {"alias": target}}));
    let vars = &s.doc().unwrap().variables;
    assert_eq!(vars.resolve(&vars.get("alias").unwrap().id, &Default::default()), Some(VariableValue::Number(5.0)));
}

#[test]
fn variables_import_checks_what_it_reads() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Theme"}));
    ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "size", "type": "color", "value": "#000000"}));
    let before = s.doc().unwrap().variables.clone();
    for p in
        [json!({}), json!({"variables": 3}), json!({"variables": "{"}), json!({"variables": {"nope": 1}}), json!({"variables": "/no/such/file.json"})]
    {
        assert!(s.execute("variable.import", &p).is_err(), "{p}");
    }
    for p in [json!({"collections": "x"}), json!({"collections": ["nope"]}), json!({"format": "xml"}), json!({"collections": [3]})] {
        assert!(s.execute("variable.export", &p).is_err(), "{p}");
    }
    assert_eq!(s.doc().unwrap().variables, before);
    // Junk entries are skipped with warnings; a type clash and an alias loop don't get in.
    let mode = json!([{"modeId": "m", "name": "Mode 1"}]);
    let alias = |id: &str| json!({"type": "VARIABLE_ALIAS", "id": id});
    let junk = json!({"meta": {
    "variableCollections": {"c": {"id": "c", "name": "Theme", "modes": mode}, "bad": {"name": "x", "modes": "no"}},
    "variables": {
        "size": {"id": "size", "name": "size", "variableCollectionId": "c", "resolvedType": "FLOAT", "valuesByMode": {"m": 3}},
        "a": {"id": "a", "name": "a", "variableCollectionId": "c", "resolvedType": "FLOAT", "valuesByMode": {"m": alias("b")}},
        "b": {"id": "b", "name": "b", "variableCollectionId": "c", "resolvedType": "FLOAT", "valuesByMode": {"m": alias("a")}},
        "w": {"id": "w", "name": "w", "variableCollectionId": "c", "resolvedType": "WEIRD", "valuesByMode": {"m": 1}},
        "n": {"id": "n", "name": "n", "variableCollectionId": "c", "resolvedType": "FLOAT", "valuesByMode": {"m": "NaN"}}
    }}});
    let r = ex(&mut s, "variable.import", json!({"variables": junk}));
    let warnings = r["warnings"].to_string();
    assert!(warnings.contains("`size` is a color variable"), "{r}");
    assert!(warnings.contains("refer back to itself"), "{r}");
    let vars = &s.doc().unwrap().variables;
    assert!(matches!(vars.get("size").unwrap().values.values().next(), Some(VariableValue::Color(_))), "the clash left it alone");
    assert_eq!(vars.collections.len(), 1);
    let (a, b) = (vars.get("a").unwrap(), vars.get("b").unwrap());
    assert!(!a.values.values().chain(b.values.values()).all(|v| matches!(v, VariableValue::Alias(_))), "no loop");
    healthy(&s);
}

#[test]
fn variables_import_caps_the_file_it_reads() {
    let dir = std::env::temp_dir().join(format!("figcraft-vars-io-cap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new();
    theme(&mut s);
    let small = dir.join("theme.json").to_string_lossy().into_owned();
    ex(&mut s, "variable.export", json!({"path": small}));
    let mut t = Session::new();
    ex(&mut t, "variable.createCollection", json!({"name": "Other"}));
    // One byte over the limit (sparse, so nothing is written), a folder, and a device: refused,
    // nothing changes.
    let big = dir.join("big.json");
    std::fs::File::create(&big).unwrap().set_len(256 * 1024 * 1024 + 1).unwrap();
    let mut refused = vec![big.to_string_lossy().into_owned(), dir.to_string_lossy().into_owned()];
    if cfg!(unix) {
        refused.push("/dev/null".to_string());
    }
    for bad in &refused {
        let before = (format!("{:?}", t.docs), t.active);
        let e = t.execute("variable.import", &json!({"variables": bad})).unwrap_err().to_string();
        assert!(["more than the 256 MB allowed here", "is a folder, not a file", "isn't a regular file"].iter().any(|w| e.contains(w)), "{bad}: {e}");
        assert_eq!((format!("{:?}", t.docs), t.active), before, "{bad}");
    }
    // A file under the limit still imports.
    ex(&mut t, "variable.import", json!({"variables": small}));
    assert_eq!(t.doc().unwrap().variables.collections.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rectangle filled with a three-stop linear gradient, and colour variables `brand` and `ink`.
fn gradient_rect(s: &mut Session) -> NodeId {
    let r = idv(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40})));
    let mut g = Gradient::default_for(GradientKind::Linear, Color::BLACK);
    g.stops = vec![
        ColorStop { position: 0.0, color: Color::BLACK },
        ColorStop { position: 0.5, color: Color::WHITE },
        ColorStop { position: 1.0, color: Color::BLACK },
    ];
    ex(s, "fill.set", json!({"ids": [r], "index": 0, "paint": Paint::gradient(g)}));
    ex(s, "variable.createCollection", json!({"name": "T"}));
    ex(s, "variable.create", json!({"collection": "T", "name": "brand", "type": "color", "value": "#ff0000"}));
    ex(s, "variable.create", json!({"collection": "T", "name": "ink", "type": "color", "value": "#00ff00"}));
    r
}

fn stops(s: &Session, r: NodeId) -> Vec<String> {
    match &s.doc().unwrap().node(r).unwrap().fills[0].kind {
        PaintKind::Gradient(g) => g.stops.iter().map(|st| st.color.to_hex()).collect(),
        k => panic!("{k:?}"),
    }
}

fn bound_keys(s: &Session, r: NodeId) -> Vec<String> {
    s.doc().unwrap().node(r).unwrap().bound.keys().cloned().collect()
}

#[test]
fn gradient_stops_bind_to_variables_and_follow_their_stops() {
    let mut s = Session::new();
    let r = gradient_rect(&mut s);
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fills/0/stops/2/color", "variable": "brand"}));
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fills/0/stops/1/color", "variable": "ink"}));
    assert_eq!(stops(&s, r), ["000000", "00FF00", "FF0000"]);
    ex(&mut s, "variable.set", json!({"variable": "brand", "value": "#0000ff"}));
    assert_eq!(stops(&s, r), ["000000", "00FF00", "0000FF"]);
    // Removing the first stop moves the bindings down with their stops.
    let cur = |s: &Session| s.doc().unwrap().node(r).unwrap().fills[0].clone();
    let mut p = cur(&s);
    if let PaintKind::Gradient(g) = &mut p.kind {
        g.stops.remove(0);
    }
    ex(&mut s, "fill.set", json!({"ids": [r], "index": 0, "paint": p}));
    assert_eq!(bound_keys(&s, r), ["fills/0/stops/0/color", "fills/0/stops/1/color"]);
    assert_eq!(stops(&s, r), ["00FF00", "0000FF"]);
    // Moving a stop (or recolouring another paint) keeps them; removing a bound stop drops its binding.
    let mut p = cur(&s);
    if let PaintKind::Gradient(g) = &mut p.kind {
        g.stops[0].position = 0.9;
        g.stops.push(ColorStop { position: 0.1, color: Color::WHITE });
    }
    ex(&mut s, "fill.set", json!({"ids": [r], "index": 0, "paint": p}));
    assert_eq!(bound_keys(&s, r).len(), 2);
    let mut p = cur(&s);
    if let PaintKind::Gradient(g) = &mut p.kind {
        g.stops.remove(1);
    }
    ex(&mut s, "fill.set", json!({"ids": [r], "index": 0, "paint": p}));
    assert_eq!(bound_keys(&s, r), ["fills/0/stops/0/color"]);
    // The paint moves up the list with its binding; a solid paint has no stops to bind.
    ex(&mut s, "fill.add", json!({"ids": [r], "color": "#cccccc"}));
    ex(&mut s, "fill.move", json!({"ids": [r], "from": 0, "to": 1}));
    assert_eq!(bound_keys(&s, r), ["fills/1/stops/0/color"]);
    ex(&mut s, "fill.set", json!({"ids": [r], "index": 1, "color": "#123456"}));
    assert!(bound_keys(&s, r).is_empty());
    // Only colour variables, and junk stop keys bind nothing.
    assert!(crate::cmd::lists::stop_binding("stops/-1/color").is_none() && crate::cmd::lists::stop_binding("stops/1/color") == Some(1));
    healthy(&s);
}

#[test]
fn variant_properties_follow_string_variables() {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for (k, (state, color)) in [("Default", "#ff0000"), ("Hover", "#0000ff")].iter().enumerate() {
        let f =
            idv(&ex(&mut s, "frame.create", json!({"x": k as f64 * 150.0, "y": 0, "width": 100, "height": 40, "name": format!("State={state}")})));
        ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 40, "parent": f, "props": {"name": "Bg", "fill": color}}));
        variants.push(idv(&ex(&mut s, "component.create", json!({"ids": [f]}))));
    }
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": variants.clone()})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 200})));
    ex(&mut s, "variable.createCollection", json!({"name": "UI", "modes": ["Calm", "Busy"]}));
    ex(&mut s, "variable.create", json!({"collection": "UI", "name": "state", "type": "string", "values": {"Calm": "Default", "Busy": "Hover"}}));
    ex(&mut s, "variable.create", json!({"collection": "UI", "name": "count", "type": "number", "value": 1}));
    let main = |s: &Session| match s.doc().unwrap().node(i).unwrap().kind {
        NodeKind::Instance { component, .. } => component,
        _ => panic!(),
    };
    assert!(s.execute("variable.bind", &json!({"ids": [i], "field": "componentProperties/State", "variable": "count"})).is_err());
    ex(&mut s, "variable.bind", json!({"ids": [i], "field": "componentProperties/State", "variable": "state"}));
    assert_eq!(main(&s), variants[0]);
    ex(&mut s, "variable.setMode", json!({"ids": [i], "collection": "UI", "mode": "Busy"}));
    assert_eq!(main(&s), variants[1]);
    let bg = s.doc().unwrap().node(i).unwrap().children[0];
    assert_eq!(fill_hex(&s, bg), "0000FF", "the instance shows the variant");
    // A value naming no variant changes nothing.
    ex(&mut s, "variable.set", json!({"variable": "state", "mode": "Busy", "value": "Pressed"}));
    assert_eq!(main(&s), variants[1]);
    healthy(&s);
}

#[test]
fn variables_export_as_dart_constants_for_flutter() {
    let mut s = Session::new();
    theme(&mut s);
    let dart = ex(&mut s, "variable.export", json!({"format": "dart"}))["text"].as_str().unwrap().to_string();
    for want in [
        "import 'dart:ui';\n",
        "/// Theme · Light\nabstract final class Theme {\n",
        "  static const Color colorPrimary = Color(0x800000FF);\n",
        "  static const double radius = 4;\n",
        "  static const String label = 'Say \"hi\"';\n",
        "  static const bool on = false;\n",
        "  static const Color accent = Color(0xFF000000);\n",
        "/// Theme · Dark\nabstract final class ThemeDark {\n",
        "  static const double radius = 6.5;\n",
        // An alias is its target's value in that mode.
        "  static const Color accent = Color(0xFFFFFFFF);\n",
    ] {
        assert!(dart.contains(want), "{want:?} in\n{dart}");
    }
    // Every class lists every variable once.
    assert_eq!(dart.matches("static const").count(), 10, "{dart}");
}

#[test]
fn dart_export_names_are_valid_dart() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "2 brand-tokens", "modes": ["Default", "high contrast"]}));
    ex(&mut s, "variable.createCollection", json!({"name": "Brand tokens"}));
    for name in ["space/2xl", "class", "a b", "a-b", "\u{65e5}\u{672c}", "it's $5\\"] {
        ex(&mut s, "variable.create", json!({"collection": "2 brand-tokens", "name": name, "type": "string", "value": name}));
    }
    let dart = ex(&mut s, "variable.export", json!({"format": "dart"}))["text"].as_str().unwrap().to_string();
    for want in [
        "abstract final class V2BrandTokens {",
        "abstract final class V2BrandTokensHighContrast {",
        "abstract final class BrandTokens {",
        "static const String space2xl = 'space/2xl';",
        "static const String class_ = 'class';",
        // Two names that read the same in Dart stay apart.
        "static const String aB = 'a b';",
        "static const String aB2 = 'a-b';",
        // Nothing usable in a name: still a name.
        "static const String v = '\u{65e5}\u{672c}';",
        "static const String itS5 = 'it\\'s \\$5\\\\';",
    ] {
        assert!(dart.contains(want), "{want:?} in\n{dart}");
    }
}

#[test]
fn dart_export_names_never_shadow_dart_types() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Color"}));
    ex(&mut s, "variable.createCollection", json!({"name": "String"}));
    for name in ["double", "int", "bool", "num"] {
        ex(&mut s, "variable.create", json!({"collection": "Color", "name": name, "type": "number", "value": 1}));
    }
    ex(&mut s, "variable.create", json!({"collection": "Color", "name": "ratio", "type": "number", "value": 0.125}));
    ex(&mut s, "variable.create", json!({"collection": "Color", "name": "third", "type": "number", "value": 1.0 / 3.0}));
    ex(&mut s, "variable.create", json!({"collection": "Color", "name": "huge", "type": "number", "value": 1.5e21}));
    let dart = ex(&mut s, "variable.export", json!({"format": "dart"}))["text"].as_str().unwrap().to_string();
    // A class `Color` would hide dart:ui's `Color`, a constant `double` the type of those after it.
    for want in [
        "abstract final class Color_ {",
        "abstract final class String_ {",
        "static const double double_ = 1;",
        "static const double int_ = 1;",
        "static const double bool_ = 1;",
        "static const double num_ = 1;",
        // Numbers keep their full precision.
        "static const double ratio = 0.125;",
        "static const double third = 0.3333333333333333;",
        // Too big for an exact integer literal: an exponent, still a double.
        "static const double huge = 1.5e21;",
    ] {
        assert!(dart.contains(want), "{want:?} in\n{dart}");
    }
}

#[test]
fn paints_replaced_through_node_set_keep_unchanged_stop_bindings() {
    let mut s = Session::new();
    let r = gradient_rect(&mut s);
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fills/0/stops/2/color", "variable": "brand"}));
    ex(&mut s, "variable.bind", json!({"ids": [r], "field": "fills/0/stops/1/color", "variable": "ink"}));
    let cur = |s: &Session| s.doc().unwrap().node(r).unwrap().fills[0].clone();
    // Removing the first stop moves both bindings down with their stops.
    let mut p = cur(&s);
    if let PaintKind::Gradient(g) = &mut p.kind {
        g.stops.remove(0);
    }
    ex(&mut s, "node.set", json!({"ids": [r], "props": {"fills": [p]}}));
    assert_eq!(bound_keys(&s, r), ["fills/0/stops/0/color", "fills/0/stops/1/color"]);
    // Recolouring one bound stop detaches that stop only.
    let mut p = cur(&s);
    if let PaintKind::Gradient(g) = &mut p.kind {
        g.stops[0].color = Color::WHITE;
    }
    ex(&mut s, "node.set", json!({"ids": [r], "props": {"fills": [p]}}));
    assert_eq!(bound_keys(&s, r), ["fills/0/stops/1/color"]);
    ex(&mut s, "variable.set", json!({"variable": "brand", "value": "#0000ff"}));
    assert_eq!(stops(&s, r), ["FFFFFF", "0000FF"]);
    healthy(&s);
}

#[test]
fn css_export_gives_lengths_a_unit_and_keeps_opacity_bare() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Brand"}));
    ex(&mut s, "variable.create", json!({"collection": "Brand", "name": "card", "type": "number", "value": 12}));
    ex(&mut s, "variable.create", json!({"collection": "Brand", "name": "fade", "type": "number", "value": 0.5}));
    ex(&mut s, "variable.create", json!({"collection": "Brand", "name": "half", "type": "number", "value": 0.5}));
    ex(&mut s, "variable.create", json!({"collection": "Brand", "name": "dim", "type": "number", "value": 1}));
    ex(&mut s, "variable.set", json!({"variable": "dim", "value": {"alias": "half"}}));
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let g = idv(&ex(&mut s, "frame.create", json!({"x": 200, "y": 0, "width": 100, "height": 100})));
    ex(&mut s, "node.set", json!({"id": f, "props": {"cornerRadius": 12, "opacity": 0.5}}));
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "cornerRadius", "variable": "card"}));
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "opacity", "variable": "fade"}));
    ex(&mut s, "variable.bind", json!({"ids": [g], "field": "opacity", "variable": "dim"}));
    let css = ex(&mut s, "variable.export", json!({"format": "css"}))["text"].as_str().unwrap_or_default().to_string();
    // A radius is a length; opacity stays a number, also through an alias it reads.
    assert!(css.contains("--card: 12px;") && css.contains("--fade: 0.5;") && css.contains("--half: 0.5;"), "{css}");
}

#[test]
fn review_a_long_alias_chain_keeps_its_role_to_the_end() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Brand"}));
    ex(&mut s, "variable.create", json!({"collection": "Brand", "name": "o0", "type": "number", "value": 0.5}));
    // o12 -> o11 -> … -> o0: longer than any fixed number of rounds would reach.
    for i in 1..=12 {
        ex(&mut s, "variable.create", json!({"collection": "Brand", "name": format!("o{i}"), "type": "number", "value": 1}));
        ex(&mut s, "variable.set", json!({"variable": format!("o{i}"), "value": {"alias": format!("o{}", i - 1)}}));
    }
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "opacity", "variable": "o12"}));
    let css = ex(&mut s, "variable.export", json!({"format": "css"}))["text"].as_str().unwrap_or_default().to_string();
    assert!(css.contains("--o0: 0.5;"), "{css}");
}

#[test]
fn css_export_neutralises_every_css_newline_in_strings() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Brand"}));
    ex(&mut s, "variable.create", json!({"collection": "Brand", "name": "t", "type": "string", "value": "a\rb\u{c}c"}));
    for format in ["css", "tailwind"] {
        let css = ex(&mut s, "variable.export", json!({"format": format}))["text"].as_str().unwrap_or_default().to_string();
        assert!(!css.contains('\r') && !css.contains('\u{c}'), "{css:?}");
        assert!(css.contains("\"a b c\""), "{css:?}");
    }
}

#[test]
fn stored_variable_ids_keep_exported_css_alias_names() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Theme"}));
    let shadow = ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "shadow", "type": "color", "value": "#FF0000"}));
    let surface = ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "surface", "type": "color", "value": "#0D99FF"}));
    ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "alias", "type": "color", "value": {"alias": surface["id"]}}));
    ex(&mut s, "variable.update", json!({"variable": shadow["id"], "name": surface["id"], "codeSyntax": {"WEB": "--shadow"}}));
    let css = ex(&mut s, "variable.export", json!({"format": "css"}));
    assert!(css["text"].as_str().unwrap().contains("--alias: var(--surface);"));
}

#[test]
fn stored_variable_ids_reset_only_the_inherited_parent_and_collection() {
    for collection_shadow in [false, true] {
        let mut s = Session::new();
        let shadow_coll = ex(&mut s, "variable.createCollection", json!({"name": "Shadow"}));
        ex(&mut s, "variable.createCollection", json!({"name": "Base", "modes": ["Light", "Dark"]}));
        let shadow = ex(&mut s, "variable.create", json!({"collection": "Shadow", "name": "shadow", "type": "number", "value": 0}));
        let surface =
            ex(&mut s, "variable.create", json!({"collection": "Base", "name": "surface", "type": "number", "values": {"Light": 4, "Dark": 8}}));
        if collection_shadow {
            ex(&mut s, "variable.renameCollection", json!({"collection": shadow_coll["id"], "name": "extended"}));
        } else {
            ex(&mut s, "variable.update", json!({"variable": shadow["id"], "name": surface["id"]}));
        }
        let doc = s.doc_mut().unwrap();
        let parent = doc.variables.get_by_id(surface["id"].as_str().unwrap()).unwrap().clone();
        let mut coll = doc.variables.collection_by_id(&parent.collection).unwrap().clone();
        let (light, dark) = (coll.find_mode("Light").unwrap().id.clone(), coll.find_mode("Dark").unwrap().id.clone());
        coll.extends = Some(coll.id.clone());
        coll.id = "extended".into();
        coll.name = "Extended".into();
        let mut inherited = parent.clone();
        inherited.id = "inherited".into();
        inherited.name = "inherited".into();
        inherited.collection = coll.id.clone();
        inherited.extends = Some(parent.id);
        inherited.overrides = inherited.values.keys().cloned().collect();
        inherited.values.values_mut().for_each(|v| *v = VariableValue::Number(2.0));
        doc.variables.collections.push(coll);
        doc.variables.variables.push(inherited);
        ex(&mut s, "variable.resetOverride", json!({"variable": "inherited", "mode": "Dark"}));
        let result = s.doc().unwrap().variables.get_by_id("inherited").unwrap();
        assert_eq!(result.values[&light], VariableValue::Number(2.0));
        assert_eq!(result.values[&dark], VariableValue::Number(8.0));
        assert_eq!(result.overrides, std::collections::BTreeSet::from([light]));
    }
}
