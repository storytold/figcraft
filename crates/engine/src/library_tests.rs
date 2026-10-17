//! Local libraries: publish, enable, place, update, read-only copies, damaged files.

use super::*;
use crate::components::mirror_issues;
use crate::{EngineError, Session};
use serde_json::{Value, json};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    match s.execute(cmd, &p) {
        Ok(v) => v,
        Err(e) => panic!("{cmd} {p}: {e}"),
    }
}

fn idv(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().node(id).unwrap().clone()
}

fn fill(s: &Session, id: NodeId) -> String {
    node(s, id).fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

fn chars(s: &Session, id: NodeId) -> String {
    node(s, id).kind.text().map(|t| t.characters.clone()).unwrap_or_default()
}

fn healthy(s: &Session) {
    let doc = s.doc().unwrap();
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());
    assert!(mirror_issues(doc).is_empty(), "{:?}", mirror_issues(doc));
}

/// A scratch directory for one test.
fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-lib-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn path_str(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

fn dirty_library_document() -> (Document, NodeId) {
    let mut doc = Document::new("Dirty library cache");
    let store = doc.ensure_library_store();
    let remote = doc.alloc_id();
    doc.insert(Node::new(remote, NodeKind::Frame, "Remote", 10.0, 10.0), store, None).unwrap();
    (doc, remote)
}

/// An imported cache is dirty until the first edit; that alone is no edit of the library copies.
#[test]
fn read_only_allows_unchanged_predirty_remote_nodes() {
    let (before, remote) = dirty_library_document();
    assert!(before.dirty().contains(&remote));

    let mut after = before.clone();
    after.add_page("Unrelated page");
    assert!(after.dirty().contains(&remote), "the imported remote node remains in the pending dirty set");
    check_read_only("page.add", &before, &after).unwrap();

    let mut edited = before.clone();
    edited.get_mut(remote).unwrap().opacity = 0.5;
    assert_eq!(check_read_only("node.set", &before, &edited), Err(READ_ONLY.into()));

    let mut moved = before.clone();
    moved.reparent(remote, before.first_page().unwrap(), None).unwrap();
    assert_eq!(check_read_only("node.reparent", &before, &moved), Err(READ_ONLY.into()));

    let mut deleted = before.clone();
    deleted.remove(remote).unwrap();
    assert_eq!(check_read_only("edit.delete", &before, &deleted), Err(READ_ONLY.into()));
}

/// The library file's layers.
struct Kit {
    button: NodeId,
    label: NodeId,
    card: NodeId,
}

/// A library file: "Button" (blue, "OK" label, its fill linked to the "Brand" paint style) and
/// "Card" (a frame holding a Button instance), plus a private "_Draft" component.
fn kit(s: &mut Session) -> Kit {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 40, "name": "Button"})));
    let label = idv(&ex(s, "text.create", json!({"text": "OK", "x": 10, "y": 10, "parent": f, "props": {"name": "Label"}})));
    ex(s, "node.set", json!({"id": f, "props": {"fill": "#0000ff"}}));
    ex(s, "style.create", json!({"type": "paint", "name": "Brand", "from": f}));
    let button = idv(&ex(s, "component.create", json!({"ids": [f]})));
    let c = idv(&ex(s, "frame.create", json!({"x": 0, "y": 100, "width": 200, "height": 100, "name": "Card"})));
    ex(s, "instance.create", json!({"component": button, "x": 10, "y": 10, "parent": c}));
    let card = idv(&ex(s, "component.create", json!({"ids": [c]})));
    let d = idv(&ex(s, "frame.create", json!({"x": 300, "y": 0, "width": 10, "height": 10, "name": "_Draft"})));
    ex(s, "component.create", json!({"ids": [d]}));
    Kit { button, label, card }
}

/// Publish the active document and save it to `path`; returns the library id.
fn publish_to(s: &mut Session, path: &str) -> String {
    let r = ex(s, "library.publish", json!({"description": "Kit"}));
    ex(s, "file.saveAs", json!({"path": path}));
    r["id"].as_str().unwrap().to_string()
}

#[test]
fn publish_manifest_roundtrip() {
    let mut s = Session::new();
    let k = kit(&mut s);
    let r = ex(&mut s, "library.publish", json!({"description": "Our kit"}));
    assert_eq!(r["version"], 1);
    assert_eq!((r["components"].as_u64(), r["styles"].as_u64()), (Some(2), Some(1)), "{r}");
    let doc = s.doc().unwrap();
    let lib = doc.published.clone().unwrap();
    let m = &lib.manifest;
    assert!(figcraft_format::is_valid_library_id(&m.id));
    assert_eq!(m.description, "Our kit");
    assert!(m.component(k.button).is_some_and(|a| a.published && a.version == 1));
    assert!(m.component(k.card).is_some_and(|a| a.published));
    assert!(m.components.iter().all(|a| a.name != "_Draft"), "names starting with _ stay private");
    // The snapshot holds the assets with the file's ids, and nothing else of the file.
    assert!(lib.doc.node(k.label).is_some());
    assert!(lib.doc.validate().is_empty());
    assert_eq!(lib.doc.styles.len(), 1);
    // Saved and read back: the author's file and a reader of the library see the same manifest.
    let bytes = figcraft_format::save_bytes(doc, None).unwrap();
    let (back, issues) = figcraft_format::load_bytes(&bytes).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(back.published.as_ref().map(|p| &p.manifest), Some(m));
    let read = figcraft_format::read_library(std::io::Cursor::new(&bytes)).unwrap().unwrap();
    assert_eq!(read.manifest, *m);
    assert_eq!(read.doc.node_count(), lib.doc.node_count());
    // Older files and files never published have no library; readers that don't know libraries
    // are unaffected (the entries are extra archive members).
    let plain = figcraft_format::save_bytes(&Document::new("x"), None).unwrap();
    assert!(figcraft_format::read_library(std::io::Cursor::new(&plain)).unwrap().is_none());
    assert!(figcraft_format::load_bytes(&plain).unwrap().0.published.is_none());

    // Nothing changed: refused. A change bumps only what it touches (and what uses it).
    let e = s.execute("library.publish", &json!({})).unwrap_err();
    assert!(e.to_string().contains("no changes"), "{e}");
    ex(&mut s, "node.set", json!({"id": k.card, "props": {"fill": "#eeeeee"}}));
    let r = ex(&mut s, "library.publish", json!({}));
    assert_eq!(r["version"], 2);
    let m = s.doc().unwrap().published.clone().unwrap().manifest.clone();
    assert_eq!(m.component(k.card).map(|a| a.version), Some(2));
    assert_eq!(m.component(k.button).map(|a| a.version), Some(1), "untouched assets keep their version");
    assert_eq!(m.description, "Our kit", "the description carries over");
    // Excluding an asset; a used component still comes along (not offered).
    let r = ex(&mut s, "library.publish", json!({"exclude": ["Button"]}));
    assert_eq!(r["components"], 1);
    let m = s.doc().unwrap().published.clone().unwrap().manifest.clone();
    assert!(m.component(k.button).is_some_and(|a| !a.published));
    // Unknown assets are an error, not ignored.
    assert!(s.execute("library.publish", &json!({"include": ["Nope"]})).is_err());
    assert!(s.execute("library.publish", &json!({"include": "Button"})).is_err());
    // Undo takes the publish back.
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().published.as_ref().map(|p| p.manifest.version), Some(2));
}

#[test]
fn add_place_and_update_keep_overrides() {
    let d = dir("flow");
    let lib_path = path_str(&d.join("kit.figcraft"));
    let mut lib = Session::new();
    let k = kit(&mut lib);
    let lid = publish_to(&mut lib, &lib_path);

    let mut s = Session::new();
    let r = ex(&mut s, "library.add", json!({"path": lib_path}));
    assert_eq!(r["id"], lid.as_str());
    assert_eq!(r["components"].as_array().map(Vec::len), Some(2));
    let list = ex(&mut s, "library.list", json!({"assets": true}));
    assert_eq!(list["libraries"][0]["updates"], 0);
    assert_eq!(list["libraries"][0]["components"].as_array().map(Vec::len), Some(2));

    // Place the Card: it comes with the Button it uses and the style the Button links to.
    let r = ex(&mut s, "library.place", json!({"library": lid, "component": "Card", "x": 300, "y": 300}));
    let inst = idv(&r);
    let main = id_of(&r["component"]).unwrap();
    {
        let doc = s.doc().unwrap();
        assert!(doc.is_remote(main) && !doc.is_remote(inst));
        assert!(!doc.pages.contains(&doc.library_store().unwrap()), "the store is not a page");
        assert_eq!(doc.libraries.components.len(), 2, "Card and the Button it uses");
        assert_eq!(doc.styles.len(), 1);
        assert!(is_remote_style(doc, &doc.styles[0].id));
        assert!(local_components(doc).is_empty(), "library copies aren't local components");
    }
    healthy(&s);
    // Placing again reuses the copies.
    let r2 = ex(&mut s, "library.place", json!({"library": lid, "component": k.card.0, "x": 600, "y": 300}));
    assert_eq!(id_of(&r2["component"]), Some(main));
    assert_eq!(s.doc().unwrap().libraries.components.len(), 2);
    s.undo().unwrap();
    // A Button on its own, with its label overridden.
    let b = idv(&ex(&mut s, "library.place", json!({"library": lid, "component": "Button", "x": 0, "y": 0})));
    let b_label = node(&s, b).children[0];
    ex(&mut s, "text.set", json!({"id": b_label, "characters": "Buy"}));
    assert!(node(&s, b_label).overrides.contains("characters"));
    assert_eq!(fill(&s, b), "0000FF");
    healthy(&s);

    // Library copies are read-only.
    let remote_button = match node(&s, b).kind {
        NodeKind::Instance { component, .. } => component,
        _ => panic!("not an instance"),
    };
    let unchanged = s.doc().unwrap().clone();
    let e = s.execute("node.set", &json!({"id": remote_button, "props": {"fill": "#ff0000"}})).unwrap_err();
    assert!(e.to_string().contains("can't be edited"), "{e}");
    assert_eq!(s.doc().unwrap(), &unchanged, "a refused remote edit rolls back");
    for (cmd, args) in
        [("node.reparent", json!({"ids": [remote_button], "parent": s.page().unwrap()})), ("edit.delete", json!({"ids": [remote_button]}))]
    {
        assert!(s.execute(cmd, &args).is_err(), "{cmd} must preserve read-only copies");
        assert_eq!(s.doc().unwrap(), &unchanged, "refused {cmd} rolls back");
    }
    let style = s.doc().unwrap().styles[0].id.clone();
    assert!(s.execute("style.update", &json!({"style": style, "name": "Mine"})).is_err());
    assert!(s.execute("style.delete", &json!({"style": style})).is_err());
    let e = s.execute("component.goToMain", &json!({"id": b})).unwrap_err();
    assert!(e.to_string().contains("library"), "{e}");
    assert!(s.execute("select.set", &json!({"ids": [remote_button]})).is_err(), "library copies can't be selected");
    assert!(s.doc().unwrap().pages.contains(&s.page().unwrap()));
    assert!(s.execute("component.pushChanges", &json!({"id": b})).is_err());
    assert_eq!(fill(&s, remote_button), "0000FF");

    // The library changes the Button (fill via its style, and its label) and publishes again.
    ex(&mut lib, "style.update", json!({"style": "Brand", "value": "#00ff00"}));
    ex(&mut lib, "text.set", json!({"id": k.label, "characters": "Go"}));
    ex(&mut lib, "library.publish", json!({}));
    ex(&mut lib, "file.save", json!({}));

    let u = ex(&mut s, "library.updates", json!({}));
    assert_eq!(u["count"], 3, "Button, Card and the style: {u}");
    let label_ids: Vec<NodeId> = s.doc().unwrap().subtree(b);
    ex(&mut s, "library.acceptUpdates", json!({}));
    assert_eq!(fill(&s, b), "00FF00", "the instance follows the updated main");
    assert_eq!(chars(&s, b_label), "Buy", "its override is kept");
    assert_eq!(s.doc().unwrap().subtree(b), label_ids, "layer ids are kept");
    assert_eq!(fill(&s, remote_button), "00FF00");
    let style = s.doc().unwrap().styles[0].clone();
    assert!(matches!(&style.kind, StyleKind::Paint { paints } if paints[0].solid_color().unwrap().to_hex() == "00FF00"));
    let doc = s.doc().unwrap();
    assert_eq!(doc.libraries.link(&lid).map(|l| l.version), Some(2));
    // The Card's nested Button follows too.
    let card_inst = doc.nodes().find(|n| matches!(n.kind, NodeKind::Instance { component, .. } if component == main)).map(|n| n.id).unwrap();
    let nested = node(&s, card_inst).children[0];
    assert_eq!(fill(&s, nested), "00FF00");
    assert_eq!(ex(&mut s, "library.updates", json!({}))["count"], 0);
    healthy(&s);
    // A structural change: the Button gains a layer (the Card's nested Button too).
    ex(&mut lib, "shape.ellipse", json!({"x": 80, "y": 10, "width": 8, "height": 8, "parent": k.button, "props": {"name": "Dot"}}));
    ex(&mut lib, "library.publish", json!({}));
    ex(&mut lib, "file.save", json!({}));
    ex(&mut s, "library.acceptUpdates", json!({"ids": [lid]}));
    assert_eq!(node(&s, b).children.len(), 2);
    assert_eq!(node(&s, nested).children.len(), 2);
    assert_eq!(chars(&s, b_label), "Buy", "overrides survive a structural update");
    assert_eq!(s.doc().unwrap().libraries.link(&lid).map(|l| l.version), Some(3));
    healthy(&s);
    // Overrides still work after the update; resetting shows the library's text.
    ex(&mut s, "component.resetInstance", json!({"ids": [b]}));
    assert_eq!(chars(&s, node(&s, b).children[0]), "Go");

    // Saved and reopened, the document keeps its links, copies and records.
    let user_path = path_str(&d.join("app.figcraft"));
    ex(&mut s, "file.saveAs", json!({"path": user_path}));
    let mut t = Session::empty();
    ex(&mut t, "file.open", json!({"path": user_path}));
    healthy(&t);
    assert_eq!(t.doc().unwrap().libraries, s.doc().unwrap().libraries);

    // Removing the library keeps the copies instances use and drops the rest.
    ex(&mut s, "component.detach", json!({"ids": [card_inst]}));
    ex(&mut s, "library.remove", json!({"id": lid}));
    let doc = s.doc().unwrap();
    assert!(doc.libraries.enabled.is_empty());
    assert_eq!(doc.libraries.components.len(), 1, "only the Button is still used");
    assert!(doc.contains(remote_button) && !doc.contains(main));
    healthy(&s);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn library_styles_apply_and_import_with_pages() {
    let d = dir("styles");
    let lib_path = path_str(&d.join("kit.figcraft"));
    let mut lib = Session::new();
    kit(&mut lib);
    let lid = publish_to(&mut lib, &lib_path);

    let mut s = Session::new();
    ex(&mut s, "library.add", json!({"path": lib_path}));
    let r = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    let st = ex(&mut s, "library.applyStyle", json!({"library": lid, "style": "Brand", "ids": [r]}));
    let local = st["style"].as_str().unwrap().to_string();
    assert_eq!(node(&s, r).styles.fill.as_deref(), Some(local.as_str()));
    assert_eq!(fill(&s, r), "0000FF");
    // Applying it again reuses the copy.
    ex(&mut s, "library.applyStyle", json!({"library": lid, "style": "Brand", "ids": [r]}));
    assert_eq!(s.doc().unwrap().styles.len(), 1);
    assert!(s.execute("library.applyStyle", &json!({"library": lid, "style": "Nope", "ids": [r]})).is_err());
    let inst = idv(&ex(&mut s, "library.place", json!({"library": lid, "component": "Card", "x": 0, "y": 100})));
    let path = path_str(&d.join("user.figcraft"));
    ex(&mut s, "file.saveAs", json!({"path": path}));

    // Importing that file's pages into another document brings the copies along.
    let mut t = Session::new();
    ex(&mut t, "file.import", json!({"path": path}));
    healthy(&t);
    let doc = t.doc().unwrap();
    assert_eq!(doc.libraries.components.len(), 2);
    assert!(doc.libraries.link(&lid).is_some());
    let copy = doc.nodes().find(|n| n.name == node(&s, inst).name && matches!(n.kind, NodeKind::Instance { .. }) && !doc.is_remote(n.id)).unwrap();
    let NodeKind::Instance { component, .. } = copy.kind else { panic!("not an instance") };
    assert!(doc.is_remote(component));
    let store_roots = |s: &Session| {
        let doc = s.doc().unwrap();
        doc.library_store().and_then(|id| doc.node(id)).map_or(0, |n| n.children.len())
    };
    assert_eq!(store_roots(&t), 2, "recorded copies are not copied twice");
    // A second import shares the copies it already has.
    ex(&mut t, "file.import", json!({"path": path}));
    healthy(&t);
    assert_eq!(t.doc().unwrap().libraries.components.len(), 2);
    assert_eq!(store_roots(&t), 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn bad_library_files_are_errors() {
    let d = dir("bad");
    let mut s = Session::new();
    let missing = path_str(&d.join("missing.figcraft"));
    assert!(matches!(s.execute("library.add", &json!({"path": missing})), Err(EngineError::Other(_))));
    let junk = d.join("junk.figcraft");
    std::fs::write(&junk, b"not a zip at all").unwrap();
    assert!(s.execute("library.add", &json!({"path": path_str(&junk)})).is_err());
    std::fs::write(&junk, b"{\"name\": \"json\"}").unwrap();
    assert!(s.execute("library.add", &json!({"path": path_str(&junk)})).is_err());
    // A FigCraft file that was never published.
    let plain = d.join("plain.figcraft");
    std::fs::write(&plain, figcraft_format::save_bytes(&Document::new("Plain"), None).unwrap()).unwrap();
    let e = s.execute("library.add", &json!({"path": path_str(&plain)})).unwrap_err();
    assert!(e.to_string().contains("no published library"), "{e}");
    // A truncated library (damaged entries are covered by the format crate's tests).
    let mut lib = Session::new();
    kit(&mut lib);
    ex(&mut lib, "library.publish", json!({}));
    let good = figcraft_format::save_bytes(lib.doc().unwrap(), None).unwrap();
    let cut = d.join("cut.figcraft");
    std::fs::write(&cut, &good[..good.len() / 2]).unwrap();
    assert!(s.execute("library.add", &json!({"path": path_str(&cut)})).is_err());
    // Oversized files are refused before they are read.
    let ok = d.join("ok.figcraft");
    std::fs::write(&ok, &good).unwrap();
    let mut cache = Cache::default();
    let e = cache.load_limited(&path_str(&ok), 16).unwrap_err();
    assert!(e.to_string().contains("too large"), "{e}");
    assert!(cache.load(&path_str(&ok)).is_ok());
    // Its own library can't be added to a file.
    let own = d.join("own.figcraft");
    ex(&mut lib, "file.saveAs", json!({"path": path_str(&own)}));
    assert!(lib.execute("library.add", &json!({"path": path_str(&own)})).is_err());
    // Commands naming no library fail cleanly, and nothing was enabled.
    for cmd in ["library.place", "library.import", "library.applyStyle", "library.remove"] {
        assert!(s.execute(cmd, &json!({"library": "x", "id": "x", "component": 1, "style": "x"})).is_err(), "{cmd}");
    }
    assert!(s.doc().unwrap().libraries.is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn library_variants_swaps_and_undo() {
    let d = dir("variants");
    let lib_path = path_str(&d.join("chips.figcraft"));
    let mut lib = Session::new();
    let mut variants = Vec::new();
    for (i, color) in ["#ff0000", "#00ff00"].iter().enumerate() {
        let f = idv(&ex(&mut lib, "frame.create", json!({"x": i as f64 * 100.0, "y": 0, "width": 60, "height": 20, "name": "Chip"})));
        ex(&mut lib, "node.set", json!({"id": f, "props": {"fill": color}}));
        variants.push(idv(&ex(&mut lib, "component.create", json!({"ids": [f]}))));
    }
    ex(&mut lib, "component.combineVariants", json!({"ids": variants}));
    for name in ["Icon A", "Icon B"] {
        let f = idv(&ex(&mut lib, "frame.create", json!({"x": 0, "y": 100, "width": 16, "height": 16, "name": name})));
        ex(&mut lib, "component.create", json!({"ids": [f]}));
    }
    let lid = publish_to(&mut lib, &lib_path);

    let mut s = Session::new();
    ex(&mut s, "library.add", json!({"path": lib_path}));
    let before = s.doc().unwrap().clone();
    let chip = idv(&ex(&mut s, "library.place", json!({"library": lid, "component": "Chip", "x": 0, "y": 0})));
    assert_eq!(fill(&s, chip), "FF0000");
    // Variant properties switch between the library's variants.
    ex(&mut s, "instance.setProperties", json!({"ids": [chip], "properties": {"Property 1": "Variant2"}}));
    assert_eq!(fill(&s, chip), "00FF00");
    let NodeKind::Instance { component, .. } = node(&s, chip).kind else { panic!("not an instance") };
    assert!(s.doc().unwrap().is_remote(component));
    // Swapping to another library component (imported first).
    let icon = idv(&ex(&mut s, "library.place", json!({"library": lid, "component": "Icon A", "x": 100, "y": 0})));
    let b = id_of(&ex(&mut s, "library.import", json!({"library": lid, "component": "Icon B"}))["component"]).unwrap();
    ex(&mut s, "instance.swap", json!({"ids": [icon], "component": b}));
    assert!(matches!(node(&s, icon).kind, NodeKind::Instance { component, .. } if component == b));
    healthy(&s);
    // Undo goes back step by step, to before the first copy.
    for _ in 0..5 {
        s.undo().unwrap();
    }
    assert_eq!(s.doc().unwrap().libraries, before.libraries);
    assert_eq!(s.doc().unwrap().node_count(), before.node_count());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn library_survives_saving_with_versions() {
    let d = dir("versions");
    let path = path_str(&d.join("kit.figcraft"));
    let mut lib = Session::new();
    kit(&mut lib);
    let r = ex(&mut lib, "library.publish", json!({}));
    ex(&mut lib, "file.saveVersion", json!({"name": "Published"}));
    ex(&mut lib, "file.saveAs", json!({"path": path}));
    // Reopened: both the version history and the library are there.
    let mut back = Session::empty();
    ex(&mut back, "file.open", json!({"path": path}));
    assert_eq!(ex(&mut back, "version.list", json!({})).as_array().map(Vec::len), Some(1));
    assert_eq!(back.doc().unwrap().published.as_ref().map(|p| p.manifest.id.clone()), r["id"].as_str().map(str::to_string));
    // Restoring a version keeps what the file published; saved again, other files can use it.
    ex(&mut back, "version.restore", json!({"index": 0}));
    assert!(back.doc().unwrap().published.is_some());
    ex(&mut back, "file.save", json!({}));
    let mut user = Session::new();
    let added = ex(&mut user, "library.add", json!({"path": path}));
    assert_eq!(added["id"], r["id"]);
    ex(&mut user, "library.place", json!({"library": added["id"], "component": "Card", "x": 0, "y": 0}));
    healthy(&user);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_file_stamp_from_the_moment_it_was_read_does_not_prove_it_unchanged() {
    use std::time::{Duration, SystemTime};
    let read_at = SystemTime::now();
    let entry = |modified: SystemTime| Entry {
        stamp: Some(Stamp { len: 10, modified: Some(modified) }),
        read_at: Some(read_at),
        result: Err("unused".into()),
    };
    // Written in the same coarse tick as the read: a same-length rewrite would keep this stamp.
    let fresh = entry(read_at);
    assert!(!fresh.still_valid(&Stamp { len: 10, modified: Some(read_at) }));
    // Written well before the read: an unchanged stamp means an unchanged file.
    let old = read_at - Duration::from_secs(10);
    assert!(entry(old).still_valid(&Stamp { len: 10, modified: Some(old) }));
    assert!(!entry(old).still_valid(&Stamp { len: 11, modified: Some(old) }));
}
