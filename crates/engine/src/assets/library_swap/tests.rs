//! Swapping a library: instances, styles and variables move to the other library's assets of the
//! same names (overrides kept), what has no match stays and is reported, one undo step.

use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
use crate::library::{is_remote_style, library_of};
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

/// A scratch directory, removed when dropped.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A library file at `path` with "Button" (filled `color`, a "Label" text), a "Brand" paint style,
/// a "Theme" collection with an `accent` colour, and `extra` components; returns its id.
fn library(path: &str, color: &str, extra: &[&str]) -> String {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 40, "name": "Button"})));
    ex(&mut s, "text.create", json!({"text": "OK", "x": 10, "y": 10, "parent": f, "props": {"name": "Label"}}));
    ex(&mut s, "node.set", json!({"id": f, "props": {"fill": color}}));
    ex(&mut s, "component.create", json!({"ids": [f]}));
    let r = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 100, "width": 10, "height": 10})));
    ex(&mut s, "node.set", json!({"id": r, "props": {"fill": color}}));
    ex(&mut s, "style.create", json!({"type": "paint", "name": "Brand", "from": r}));
    ex(&mut s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "accent", "type": "color", "value": color}));
    for (i, name) in extra.iter().enumerate() {
        let e = idv(&ex(&mut s, "frame.create", json!({"x": 200 + 50 * i, "y": 0, "width": 20, "height": 20, "name": name})));
        ex(&mut s, "component.create", json!({"ids": [e]}));
    }
    let id = ex(&mut s, "library.publish", json!({}))["id"].as_str().unwrap().to_string();
    ex(&mut s, "file.saveAs", json!({"path": path}));
    id
}

fn main_of(s: &Session, inst: NodeId) -> NodeId {
    match s.doc().unwrap().node(inst).unwrap().kind {
        NodeKind::Instance { component, .. } => component,
        _ => panic!("not an instance"),
    }
}

/// A session with libraries A (blue, plus `extra_a` components) and B (green) enabled, files in a
/// scratch directory named by `tag`; returns the directory guard, the session and both library ids.
fn two_libraries(tag: &str, extra_a: &[&str]) -> (Scratch, Session, String, String) {
    let dir = Scratch(std::env::temp_dir().join(format!("figcraft-libswap-{tag}-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let (pa, pb) = (dir.0.join("a.figcraft").to_string_lossy().into_owned(), dir.0.join("b.figcraft").to_string_lossy().into_owned());
    let a = library(&pa, "#0000ff", extra_a);
    let b = library(&pb, "#00ff00", &[]);
    let mut s = Session::new();
    ex(&mut s, "library.add", json!({"path": pa}));
    ex(&mut s, "library.add", json!({"path": pb}));
    (dir, s, a, b)
}

#[test]
fn swap_moves_instances_styles_and_variables_by_name() {
    let (_dir, mut s, a, b) = two_libraries("main", &["Only in A"]);
    let inst = idv(&ex(&mut s, "library.place", json!({"library": a, "component": "Button", "x": 0, "y": 0})));
    let label = s.doc().unwrap().node(inst).unwrap().children[0];
    ex(&mut s, "text.set", json!({"id": label, "characters": "Buy"}));
    let only = idv(&ex(&mut s, "library.place", json!({"library": a, "component": "Only in A", "x": 300, "y": 0})));
    let rect = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 200, "width": 10, "height": 10})));
    ex(&mut s, "library.applyStyle", json!({"library": a, "style": "Brand", "ids": [rect]}));
    let dot = idv(&ex(&mut s, "shape.rectangle", json!({"x": 50, "y": 200, "width": 10, "height": 10})));
    ex(&mut s, "library.bindVariable", json!({"library": a, "variable": "accent", "field": "fill", "ids": [dot]}));
    let extended = ex(&mut s, "library.extendCollection", json!({"library": a, "collection": "Theme"}))["id"].clone();
    let before = s.doc().unwrap().clone();

    let r = ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    assert_eq!((r["components"].as_u64(), r["styles"].as_u64(), r["variables"].as_u64()), (Some(1), Some(1), Some(1)), "{r}");
    assert_eq!(r["unmatched"], json!(["component Only in A", "extended collection Theme (extended)"]));
    assert_eq!(r["removed"], false, "Only in A and the extension still use library A");
    let doc = s.doc().unwrap();
    assert_eq!(library_of(doc, main_of(&s, inst)), Some(b.as_str()), "the instance shows B's Button");
    assert_eq!(library_of(doc, main_of(&s, only)), Some(a.as_str()), "no match: still A's");
    let label = doc.node(inst).unwrap().children[0];
    assert_eq!(doc.node(label).unwrap().kind.text().map(|t| t.characters.as_str()), Some("Buy"), "the override stays");
    let fill = |id: NodeId| doc.node(id).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default();
    assert_eq!(fill(inst), "00FF00", "B's Button");
    let style = doc.node(rect).unwrap().styles.fill.clone().unwrap();
    assert!(is_remote_style(doc, &style) && doc.libraries.style_import(&style).is_some_and(|r| r.library == b));
    assert_eq!(fill(rect), "00FF00", "the style's paint follows");
    let var = doc.node(dot).unwrap().bound.get("fills/0/color").or_else(|| doc.node(dot).unwrap().bound.values().next()).cloned().unwrap();
    assert!(doc.libraries.variable_import(&var).is_some_and(|r| r.library == b), "bound to B's accent");
    assert!(doc.libraries.styles.iter().all(|r| r.library == b), "A's unused style copy is gone");
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());

    // One undo step brings A back.
    s.undo().unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(library_of(doc, main_of(&s, inst)), Some(a.as_str()));
    assert_eq!(doc.libraries.as_ref(), before.libraries.as_ref());

    // Swapping what has a match for everything removes the old library.
    ex(&mut s, "edit.delete", json!({"ids": [only]}));
    ex(&mut s, "variable.deleteCollection", json!({"collection": extended}));
    let r = ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    assert_eq!((r["unmatched"].as_array().map(Vec::len), &r["removed"]), (Some(0), &json!(true)), "{r}");
    assert!(s.doc().unwrap().libraries.enabled.iter().all(|l| l.id != a));
    // The same library, or one not enabled, is refused.
    assert!(s.execute("library.swap", &json!({"from": b, "to": b})).is_err());
    assert!(s.execute("library.swap", &json!({"from": a, "to": b})).is_err());
}

#[test]
fn stored_variable_ids_keep_library_swap_matches_on_the_real_copies() {
    let dir = Scratch(std::env::temp_dir().join(format!("figcraft-libswap-ids-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let (pa, pb) = (dir.0.join("a.figcraft").to_string_lossy().into_owned(), dir.0.join("b.figcraft").to_string_lossy().into_owned());
    let a = library(&pa, "#0000ff", &[]);
    let b = library(&pb, "#00ff00", &[]);
    let mut s = Session::new();
    let shadow_collection = ex(&mut s, "variable.createCollection", json!({"name": "Shadow"}));
    let shadow = ex(&mut s, "variable.create", json!({"collection": "Shadow", "name": "shadow", "type": "color", "value": "#ff0000"}));
    ex(&mut s, "library.add", json!({"path": pa}));
    ex(&mut s, "library.add", json!({"path": pb}));
    let dot = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    ex(&mut s, "library.bindVariable", json!({"library": a, "variable": "accent", "field": "fill", "ids": [dot]}));
    ex(&mut s, "library.setVariableMode", json!({"library": a, "collection": "Theme", "mode": "Dark", "ids": [dot]}));
    let doc = s.doc().unwrap();
    let target = doc.libraries.variables[0].local.clone();
    let collection = doc.libraries.collections[0].local.clone();
    ex(&mut s, "variable.update", json!({"variable": shadow["id"], "name": target}));
    ex(&mut s, "variable.renameCollection", json!({"collection": shadow_collection["id"], "name": collection}));
    let result = ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    assert_eq!(result["variables"], 1);
    assert_eq!(result["unmatched"], json!([]));
    let doc = s.doc().unwrap();
    let node = doc.node(dot).unwrap();
    assert_eq!(node.fills[0].solid_color().unwrap().to_hex(), "00FF00");
    assert!(!node.bound.is_empty());
    assert!(!node.var_modes.is_empty());
    assert!(node.bound.values().all(|v| doc.libraries.variable_import(v).is_some_and(|r| r.library == b)));
    assert!(node.var_modes.keys().all(|c| doc.libraries.collection_import(c).is_some_and(|r| r.library == b)));
}

/// A local colour calculation reading a library variable moves to the other library's variable of
/// the same name, as a binding does.
#[test]
fn swap_repoints_colour_calculations() {
    let (_dir, mut s, a, b) = two_libraries("calc", &[]);
    let accent = ex(&mut s, "library.importVariable", json!({"library": a, "variable": "accent"}))["variable"].clone();
    ex(&mut s, "variable.createCollection", json!({"name": "App"}));
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "App", "name": "veil", "type": "color", "value": {"alpha": {"variable": accent, "opacity": 0.5}}}),
    );
    let wash = json!({"mix": {"from": {"variable": accent}, "to": {"literal": "#ffffff"}, "t": 0.5}});
    ex(&mut s, "variable.create", json!({"collection": "App", "name": "wash", "type": "color", "value": wash}));

    let r = ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    assert_eq!(r["variables"].as_u64(), Some(1), "{r}");
    let doc = s.doc().unwrap();
    let vars = &doc.variables;
    let shown = |name: &str| match vars.resolve(&vars.get(name).unwrap().id, &Default::default()) {
        Some(VariableValue::Color(c)) => c.to_css_hex(),
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!((shown("veil"), shown("wash")), ("#00FF0080".to_string(), "#80FF80".to_string()));
    let reads: Vec<&VariableId> = ["veil", "wash"].iter().flat_map(|n| vars.get(n).unwrap().values.values()).flat_map(|v| v.references()).collect();
    assert!(reads.iter().all(|r| doc.libraries.variable_import(r).is_some_and(|i| i.library == b)), "{reads:?}");
}
