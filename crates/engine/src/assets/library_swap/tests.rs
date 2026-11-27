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
    library_with(path, color, extra, false)
}

/// [`library`], and with `linked` also "Badge", filled by the Brand style, and "Dot", its fill
/// bound to `accent`: components whose own layers link the library's style and variable.
fn library_with(path: &str, color: &str, extra: &[&str], linked: bool) -> String {
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
    if linked {
        let badge = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 30, "height": 30, "name": "Badge"})));
        ex(&mut s, "style.apply", json!({"ids": [badge], "style": "Brand"}));
        let dot = idv(&ex(&mut s, "frame.create", json!({"x": 50, "y": 300, "width": 30, "height": 30, "name": "Dot"})));
        ex(&mut s, "variable.bind", json!({"ids": [dot], "field": "fill", "variable": "accent"}));
        ex(&mut s, "component.create", json!({"ids": [badge]}));
        ex(&mut s, "component.create", json!({"ids": [dot]}));
        // A Theme variable no other library has, with a value of its own in Dark.
        ex(&mut s, "variable.create", json!({"collection": "Theme", "name": "glow", "type": "color", "value": "#111111"}));
        ex(&mut s, "variable.set", json!({"variable": "glow", "mode": "Dark", "value": "#222222"}));
    }
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
    two_libraries_with(tag, extra_a, false)
}

/// [`two_libraries`], with A made by [`library_with`] and `linked_a`.
fn two_libraries_with(tag: &str, extra_a: &[&str], linked_a: bool) -> (Scratch, Session, String, String) {
    let dir = Scratch(std::env::temp_dir().join(format!("figcraft-libswap-{tag}-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let (pa, pb) = (dir.0.join("a.figcraft").to_string_lossy().into_owned(), dir.0.join("b.figcraft").to_string_lossy().into_owned());
    let a = library_with(&pa, "#0000ff", extra_a, linked_a);
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

/// Copies of A nothing uses (a component imported but not placed, a variable imported but not
/// bound) go with the swap, as they did; the result now names them (#2569). A copy the swap moves
/// to B isn't among them.
#[test]
fn swap_lists_the_unused_copies_it_removes() {
    let (_dir, mut s, a, b) = two_libraries("unused", &["Only in A"]);
    let placed = idv(&ex(&mut s, "library.place", json!({"library": a, "component": "Button", "x": 0, "y": 0})));
    let only = id_of(&ex(&mut s, "library.import", json!({"library": a, "component": "Only in A"}))["component"]).unwrap();
    ex(&mut s, "library.importVariable", json!({"library": a, "variable": "accent"}));

    let r = ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    assert_eq!(r["dropped"], json!(["component Only in A", "variable Theme/accent", "collection Theme"]), "{r}");
    assert_eq!((r["components"].as_u64(), r["variables"].as_u64(), &r["unmatched"]), (Some(1), Some(0), &json!([])), "{r}");
    assert_eq!(r["removed"], true, "{r}");
    let doc = s.doc().unwrap();
    assert!(!doc.contains(only), "the dropped copy is gone");
    assert_eq!(library_of(doc, main_of(&s, placed)), Some(b.as_str()), "the placed Button moved, so it isn't listed");
    assert!(doc.variables.variables.is_empty() && doc.variables.collections.is_empty(), "{:?}", doc.variables);

    // One undo step brings them back.
    s.undo().unwrap();
    let doc = s.doc().unwrap();
    assert!(doc.contains(only) && library_of(doc, only) == Some(a.as_str()));
    assert_eq!(doc.variables.variables.len(), 1);

    // A deleted instance leaves its copy unused, so it is listed too; a placed copy B has no match
    // for stays and is listed as unmatched.
    ex(&mut s, "edit.delete", json!({"ids": [placed]}));
    let kept = idv(&ex(&mut s, "library.place", json!({"library": a, "component": "Only in A", "x": 0, "y": 0})));
    let r = ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    assert_eq!(r["dropped"], json!(["component Button", "variable Theme/accent", "collection Theme"]), "{r}");
    assert_eq!((&r["unmatched"], &r["removed"]), (&json!(["component Only in A"]), &json!(false)), "{r}");
    assert_eq!(library_of(s.doc().unwrap(), main_of(&s, kept)), Some(a.as_str()));
}

/// A component B has no match for stays on A, but the Brand style and `accent` variable its own
/// layers link do have matches (#2568). The swap re-points them in A's read-only copy as on the
/// placed instances, so an instance placed after the swap looks like the ones placed before it,
/// and A's style and variable copies nothing links any more go.
#[test]
fn swap_repoints_the_styles_and_variables_of_component_copies_it_keeps() {
    let (_dir, mut s, a, b) = two_libraries_with("copies", &[], true);
    let badge = idv(&ex(&mut s, "library.place", json!({"library": a, "component": "Badge", "x": 0, "y": 0})));
    let dot = idv(&ex(&mut s, "library.place", json!({"library": a, "component": "Dot", "x": 100, "y": 0})));
    let fill = |s: &Session, id: NodeId| s.doc().unwrap().node(id).unwrap().fills[0].solid_color().unwrap().to_hex();
    assert_eq!((fill(&s, badge), fill(&s, dot)), ("0000FF".to_string(), "0000FF".to_string()));

    let r = ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    assert_eq!((r["components"].as_u64(), r["styles"].as_u64(), r["variables"].as_u64()), (Some(0), Some(1), Some(1)), "{r}");
    assert_eq!(r["unmatched"], json!(["component Badge", "component Dot"]), "{r}");
    let (badge_main, dot_main) = (main_of(&s, badge), main_of(&s, dot));
    let doc = s.doc().unwrap();
    assert_eq!((library_of(doc, badge_main), library_of(doc, dot_main)), (Some(a.as_str()), Some(a.as_str())), "no match: still A's");
    // The copies link B's style and variable, as the instances do.
    for (id, what) in [(badge_main, "the Badge copy"), (badge, "the Badge instance")] {
        let style = doc.node(id).unwrap().styles.fill.clone().unwrap_or_default();
        assert!(doc.libraries.style_import(&style).is_some_and(|i| i.library == b), "{what} links {style}");
    }
    for (id, what) in [(dot_main, "the Dot copy"), (dot, "the Dot instance")] {
        let bound = doc.node(id).unwrap().bound.get("fill").cloned().unwrap_or_default();
        assert!(doc.libraries.variable_import(&bound).is_some_and(|i| i.library == b), "{what} binds {bound}");
    }
    assert!(doc.libraries.styles.iter().all(|i| i.library == b), "A's Brand copy is linked by nothing: {:?}", doc.libraries.styles);
    assert!(doc.libraries.variables.iter().all(|i| i.library == b), "A's accent copy is bound by nothing: {:?}", doc.libraries.variables);
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());

    // Instances placed after the swap show B's colours, as those placed before it.
    let late_badge = idv(&ex(&mut s, "instance.create", json!({"component": badge_main, "x": 0, "y": 100})));
    let late_dot = idv(&ex(&mut s, "instance.create", json!({"component": dot_main, "x": 100, "y": 100})));
    let shown: Vec<String> = [badge, dot, late_badge, late_dot].iter().map(|i| fill(&s, *i)).collect();
    assert_eq!(shown, ["00FF00"; 4]);

    // One undo of each instance and the swap puts the copies back on A's style and variable.
    s.undo().unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    let doc = s.doc().unwrap();
    let style = doc.node(badge_main).unwrap().styles.fill.clone().unwrap_or_default();
    assert!(doc.libraries.style_import(&style).is_some_and(|i| i.library == a), "{style}");
    let bound = doc.node(dot_main).unwrap().bound.get("fill").cloned().unwrap_or_default();
    assert!(doc.libraries.variable_import(&bound).is_some_and(|i| i.library == a), "{bound}");
    assert_eq!((fill(&s, badge), fill(&s, dot)), ("0000FF".to_string(), "0000FF".to_string()));
}

/// A frame choosing A's Theme mode Dark holds a layer bound to `accent` (B has it) and one bound
/// to `glow` (only A has it). The swap gives the frame B's Dark for `accent` and keeps A's Dark for
/// `glow`, which stays on A, so it doesn't fall back to Light.
#[test]
fn swap_keeps_the_mode_of_a_collection_that_still_has_a_variable_on_the_old_library() {
    let (_dir, mut s, a, b) = two_libraries_with("modes", &[], true);
    let frame = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let rect = |s: &mut Session, x: f64| idv(&ex(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 10, "height": 10, "parent": frame})));
    let (accent, glow) = (rect(&mut s, 0.0), rect(&mut s, 20.0));
    ex(&mut s, "library.bindVariable", json!({"library": a, "variable": "accent", "field": "fill", "ids": [accent]}));
    ex(&mut s, "library.bindVariable", json!({"library": a, "variable": "glow", "field": "fill", "ids": [glow]}));
    ex(&mut s, "library.setVariableMode", json!({"library": a, "collection": "Theme", "mode": "Dark", "ids": [frame]}));
    let fill = |s: &Session, id: NodeId| s.doc().unwrap().node(id).unwrap().fills[0].solid_color().unwrap().to_hex();
    assert_eq!(fill(&s, glow), "222222", "Dark before the swap");

    let r = ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    assert_eq!((r["variables"].as_u64(), &r["unmatched"]), (Some(1), &json!(["variable Theme/glow"])), "{r}");
    let doc = s.doc().unwrap();
    let modes = &doc.node(frame).unwrap().var_modes;
    let lib = |c: &str| doc.libraries.collection_import(c).map(|r| r.library.clone());
    let mut owners: Vec<Option<String>> = modes.keys().map(|c| lib(c)).collect();
    owners.sort();
    let mut expected = vec![Some(a.clone()), Some(b.clone())];
    expected.sort();
    assert_eq!(owners, expected, "B's Theme mode, and A's kept: {modes:?}");
    assert_eq!((fill(&s, accent), fill(&s, glow)), ("00FF00".to_string(), "222222".to_string()), "glow stays on A's Dark");

    // A mode chosen of B's Theme since wins over A's Dark moved onto it by a second swap.
    ex(&mut s, "library.setVariableMode", json!({"library": b, "collection": "Theme", "mode": "Light", "ids": [frame]}));
    ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    assert_eq!(fill(&s, accent), "00FF00", "B's Light kept");
    let doc = s.doc().unwrap();
    let b_mode = doc
        .node(frame)
        .unwrap()
        .var_modes
        .iter()
        .find(|(c, _)| doc.libraries.collection_import(c).is_some_and(|r| r.library == b))
        .map(|(c, m)| (c.clone(), m.clone()));
    let (b_coll, mode) = b_mode.unwrap();
    assert_eq!(doc.variables.collection_by_id(&b_coll).and_then(|c| c.mode_by_id(&mode)).map(|m| m.name.as_str()), Some("Light"));

    // With nothing left on A's Theme, its mode goes with it.
    ex(&mut s, "edit.delete", json!({"ids": [glow]}));
    ex(&mut s, "library.swap", json!({"from": a, "to": b}));
    let doc = s.doc().unwrap();
    assert!(doc.node(frame).unwrap().var_modes.keys().all(|c| doc.libraries.collection_import(c).is_some_and(|r| r.library == b)));
}
