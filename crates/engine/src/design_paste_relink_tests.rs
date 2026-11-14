//! Instances pasted from another design app link to the mains pasted earlier (#2022). The paste
//! carries the mains the instances use in a hidden canvas; those mains are matched with the
//! components this file already holds by the identity recorded on them at their own paste (the
//! layer guid within one file, or the library key). Payloads are synthetic, built in code.

use base64::Engine as _;
use figcraft_format::fig::kiwi::Kv;
use serde_json::json;

use figcraft_doc::NodeKind;

use super::{NodeId, Session, params};
use crate::cmd::edit::{CLIP_CLOSE, CLIP_OPEN};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

type Fields = Vec<(&'static str, Kv<'static>)>;

fn o(f: Fields) -> Kv<'static> {
    Kv::Object(f)
}

fn g(local: u64) -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(local))])
}

fn xy(x: f32, y: f32) -> Kv<'static> {
    o(vec![("x", Kv::Float(x)), ("y", Kv::Float(y))])
}

fn node(id: u64, parent: Option<(u64, &str)>, ty: &'static str, name: &str, mut more: Fields) -> Kv<'static> {
    let mut v = vec![("guid", if id == 0 { o(vec![("sessionID", Kv::Uint(0)), ("localID", Kv::Uint(0))]) } else { g(id) })];
    if let Some((p, pos)) = parent {
        let pg = if p == 0 { o(vec![("sessionID", Kv::Uint(0)), ("localID", Kv::Uint(0))]) } else { g(p) };
        v.push(("parentIndex", o(vec![("guid", pg), ("position", Kv::Str(pos.into()))])));
    }
    v.push(("type", Kv::Enum(ty)));
    v.push(("name", Kv::Str(name.into())));
    v.append(&mut more);
    o(v)
}

fn main_nodes(parent: u64, id: u64, key: Option<&'static str>) -> Vec<Kv<'static>> {
    let mut f = vec![("size", xy(40.0, 20.0))];
    if let Some(k) = key {
        f.push(("key", Kv::Str(k.into())));
    }
    vec![node(id, Some((parent, "!")), "SYMBOL", "Button", f), node(id + 1, Some((id, "!")), "RECTANGLE", "Face", vec![("size", xy(40.0, 20.0))])]
}

/// A paste of the main alone, as the first copy.
fn payload_main(id: u64, key: Option<&'static str>) -> Vec<Kv<'static>> {
    let mut v = vec![node(0, None, "DOCUMENT", "Document", vec![]), node(1, Some((0, "!")), "CANVAS", "Page", vec![])];
    v.extend(main_nodes(1, id, key));
    v
}

/// A paste of an instance of the main `id`, whose definition travels in the hidden canvas.
fn payload_instance(id: u64, key: Option<&'static str>, with_main: bool) -> Vec<Kv<'static>> {
    let mut v = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(2, Some((0, "~")), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))]),
        node(30, Some((1, "!")), "INSTANCE", "Copy", vec![("size", xy(40.0, 20.0)), ("symbolData", o(vec![("symbolID", g(id))]))]),
    ];
    if with_main {
        v.extend(main_nodes(2, id, key));
    }
    v
}

fn html(nodes: Vec<Kv<'static>>, file: &str) -> String {
    let bytes = figcraft_format::fig::pack_canvas(&schema(), &o(vec![("nodeChanges", Kv::Array(nodes))])).expect("packs");
    let meta = B64.encode(json!({"fileKey": file}).to_string());
    format!(
        r#"<meta charset="utf-8"><span data-metadata="<!--(figmeta){meta}(/figmeta)-->"></span><span data-buffer="<!--{CLIP_OPEN}{}{CLIP_CLOSE}-->"></span>"#,
        B64.encode(bytes)
    )
}

fn paste(s: &mut Session, html: String) -> serde_json::Value {
    // A paste goes into the selected frame: the main just pasted is not a place for its instances.
    s.execute("select.set", &json!({"ids": []})).expect("clears");
    s.execute("edit.pasteDesignLayers", &json!({ "html": html })).expect("pastes")
}

fn ids(r: &serde_json::Value) -> Vec<NodeId> {
    r["ids"].as_array().unwrap().iter().filter_map(params::id_of).collect()
}

fn components(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().nodes().filter(|n| matches!(n.kind, NodeKind::Component { .. })).map(|n| n.id).collect()
}

fn instance_main(s: &Session, instance: NodeId) -> NodeId {
    match &s.doc().unwrap().node(instance).unwrap().kind {
        NodeKind::Instance { component, .. } => *component,
        k => panic!("not an instance: {k:?}"),
    }
}

#[test]
fn instances_pasted_after_their_mains_link_to_them() {
    let mut s = Session::new();
    let a = paste(&mut s, html(payload_main(20, None), "F1"));
    let main = ids(&a)[0];
    assert_eq!(components(&s), [main]);
    let r = paste(&mut s, html(payload_instance(20, None, true), "F1"));
    let copy = ids(&r)[0];
    assert_eq!(instance_main(&s, copy), main, "the instance shows the main pasted first");
    assert_eq!(components(&s), [main], "no second component came along");
    let doc = s.doc().unwrap();
    let face = doc.node(copy).unwrap().children[0];
    let source = doc.node(face).unwrap().source.expect("the layer is linked to the main's layer");
    assert!(doc.is_ancestor(main, source), "inside the main");
    assert!(r.get("warnings").is_none_or(|w| w.as_array().is_some_and(|w| w.is_empty())), "{r}");
    s.execute("select.set", &json!({"ids": [copy]})).unwrap();
    s.execute("component.goToMain", &json!({"id": copy})).expect("goToMain succeeds");
}

#[test]
fn one_undo_removes_the_pasted_instance_and_keeps_the_main() {
    let mut s = Session::new();
    let main = ids(&paste(&mut s, html(payload_main(20, None), "F1")))[0];
    let copy = ids(&paste(&mut s, html(payload_instance(20, None, true), "F1")))[0];
    assert!(s.undo().unwrap());
    let doc = s.doc().unwrap();
    assert!(!doc.contains(copy));
    assert!(doc.contains(main));
    assert_eq!(components(&s), [main]);
}

#[test]
fn a_main_from_another_figma_file_is_not_matched_by_guid_alone() {
    let mut s = Session::new();
    let main = ids(&paste(&mut s, html(payload_main(20, None), "F1")))[0];
    let r = paste(&mut s, html(payload_instance(20, None, true), "F2"));
    let copy = ids(&r)[0];
    assert_ne!(instance_main(&s, copy), main, "the same guid in another file is another component");
    assert_eq!(components(&s), [main], "no component came along with the instance");
    assert!(!s.doc().unwrap().contains(instance_main(&s, copy)), "its main stayed behind");
    assert!(r["warnings"].to_string().contains("stay unlinked"), "{r}");
}

#[test]
fn the_same_library_key_links_whatever_the_guids() {
    let mut s = Session::new();
    let main = ids(&paste(&mut s, html(payload_main(20, Some("btn")), "F1")))[0];
    let r = paste(&mut s, html(payload_instance(77, Some("btn"), true), "F2"));
    let copy = ids(&r)[0];
    assert_eq!(instance_main(&s, copy), main);
    assert_eq!(components(&s), [main], "the pasted main did not come along as a second component");
}

#[test]
fn an_instance_whose_main_never_arrives_stays_as_today() {
    let mut s = Session::new();
    let r = paste(&mut s, html(payload_instance(20, None, true), "F1"));
    let copy = ids(&r)[0];
    let main = instance_main(&s, copy);
    assert!(!s.doc().unwrap().contains(main), "the main stayed behind");
    let warnings = r["warnings"].to_string();
    assert!(warnings.contains("stay unlinked"), "{warnings}");
    s.execute("component.restoreMain", &json!({"id": copy})).expect("the main can be restored");
}

/// A component set of two variants (`v1`, `v2`; each with one layer after it) under `parent`.
fn set_nodes(parent: u64, set: u64, v1: u64, v2: u64) -> Vec<Kv<'static>> {
    let flag = vec![("isStateGroup", Kv::Bool(true)), ("size", xy(100.0, 20.0))];
    let mut v = vec![node(set, Some((parent, "!")), "FRAME", "Button", flag)];
    for (at, id, name) in [("!", v1, "State=Default"), ("\"", v2, "State=Hover")] {
        v.push(node(id, Some((set, at)), "SYMBOL", name, vec![("size", xy(40.0, 20.0))]));
        v.push(node(id + 1, Some((id, "!")), "RECTANGLE", "Face", vec![("size", xy(40.0, 20.0))]));
    }
    v
}

fn page_nodes() -> Vec<Kv<'static>> {
    vec![node(0, None, "DOCUMENT", "Document", vec![]), node(1, Some((0, "!")), "CANVAS", "Page", vec![])]
}

fn hidden_canvas() -> Kv<'static> {
    node(2, Some((0, "~")), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))])
}

fn instance_of(id: u64, main: u64, parent: u64, pos: &'static str) -> Kv<'static> {
    node(id, Some((parent, pos)), "INSTANCE", "Copy", vec![("size", xy(40.0, 20.0)), ("symbolData", o(vec![("symbolID", g(main))]))])
}

fn sets(s: &Session) -> usize {
    s.doc().unwrap().nodes().filter(|n| matches!(n.kind, NodeKind::ComponentSet { .. })).count()
}

#[test]
fn a_variant_instance_links_to_the_set_pasted_earlier() {
    let mut s = Session::new();
    let mut first = page_nodes();
    first.extend(set_nodes(1, 20, 21, 23));
    paste(&mut s, html(first, "F1"));
    let before = components(&s);
    assert_eq!(before.len(), 2);
    assert_eq!(sets(&s), 1);
    let mut second = page_nodes();
    second.push(hidden_canvas());
    second.push(instance_of(30, 23, 1, "!"));
    second.extend(set_nodes(2, 20, 21, 23));
    let r = paste(&mut s, html(second, "F1"));
    let main = instance_main(&s, ids(&r)[0]);
    assert!(before.contains(&main), "the instance's main is a variant already in the file");
    assert_eq!(s.doc().unwrap().node(main).unwrap().name, "State=Hover");
    assert_eq!(components(&s), before, "no variant came along");
    assert_eq!(sets(&s), 1, "no second set came along");
}

#[test]
fn a_nested_instance_inside_a_pasted_main_links_too() {
    let mut s = Session::new();
    let outer = |parent: u64| {
        vec![
            node(20, Some((parent, "!")), "SYMBOL", "Outer", vec![("size", xy(80.0, 40.0))]),
            node(21, Some((20, "!")), "RECTANGLE", "Face", vec![("size", xy(80.0, 40.0))]),
            instance_of(25, 40, 20, "\""),
        ]
    };
    let inner = |parent: u64, pos: &'static str| {
        vec![
            node(40, Some((parent, pos)), "SYMBOL", "Inner", vec![("size", xy(40.0, 20.0))]),
            node(41, Some((40, "!")), "RECTANGLE", "Face", vec![("size", xy(40.0, 20.0))]),
        ]
    };
    let mut first = page_nodes();

    first.extend(outer(1));
    first.extend(inner(1, "\""));
    paste(&mut s, html(first, "F1"));
    let main = s.doc().unwrap().nodes().find(|n| n.name == "Outer").unwrap().id;
    let before = components(&s);
    let mut second = page_nodes();
    second.push(hidden_canvas());
    second.push(instance_of(30, 20, 1, "!"));
    second.extend(outer(2));
    second.extend(inner(2, "\""));
    let r = paste(&mut s, html(second, "F1"));
    let copy = ids(&r)[0];
    assert_eq!(instance_main(&s, copy), main, "the copy shows the main pasted first");
    assert_eq!(components(&s), before, "no component came along");
    let doc = s.doc().unwrap();
    let nested: Vec<NodeId> =
        doc.subtree(copy).into_iter().filter(|i| *i != copy && matches!(doc.node(*i).unwrap().kind, NodeKind::Instance { .. })).collect();
    assert_eq!(nested.len(), 1);
    let in_main: Vec<NodeId> = doc.subtree(main).into_iter().filter(|i| matches!(doc.node(*i).unwrap().kind, NodeKind::Instance { .. })).collect();
    assert_eq!(in_main.len(), 1);
    let inner = instance_main(&s, in_main[0]);
    assert_eq!(instance_main(&s, nested[0]), inner, "the nested instance shows the inner main the first paste brought");
    assert!(doc.contains(inner));
}

#[test]
fn a_duplicated_main_does_not_take_later_pastes() {
    let mut s = Session::new();
    let main = ids(&paste(&mut s, html(payload_main(20, Some("btn")), "F1")))[0];
    // An instance, detached, then made a component: a copy of the main that FigCraft made.
    let inst = ids(&s.execute("edit.duplicate", &json!({"ids": [main]})).unwrap())[0];
    s.execute("component.detach", &json!({"ids": [inst]})).unwrap();
    s.execute("component.create", &json!({"ids": [inst]})).unwrap();
    assert!(matches!(s.doc().unwrap().node(inst).unwrap().kind, NodeKind::Component { .. }));
    let copy_data = &s.doc().unwrap().node(inst).unwrap().plugin_data;
    assert!(copy_data.keys().all(|k| !matches!(k.as_str(), "fig.guid" | "fig.key" | "fig.file")), "{copy_data:?}");
    let r = paste(&mut s, html(payload_instance(20, Some("btn"), true), "F1"));
    assert_eq!(instance_main(&s, ids(&r)[0]), main, "the original takes the paste");
    // Without the original, the copy is not the pasted main.
    s.execute("edit.delete", &json!({"ids": [main]})).unwrap();
    let r = paste(&mut s, html(payload_instance(20, Some("btn"), true), "F1"));
    assert_ne!(instance_main(&s, ids(&r)[0]), inst, "the copy never carried the main's identity");
}

#[test]
fn a_duplicated_variant_does_not_carry_the_identity() {
    let mut s = Session::new();
    let mut first = page_nodes();
    first.extend(set_nodes(1, 20, 21, 23));
    paste(&mut s, html(first, "F1"));
    let before = components(&s);
    let guids = |s: &Session| s.doc().unwrap().nodes().filter(|n| n.plugin_data.contains_key("fig.guid")).count();
    let n = guids(&s);
    s.execute("edit.duplicate", &json!({"ids": [before[0]]})).unwrap();
    assert_eq!(components(&s).len(), 3, "the copy is a new variant");
    assert_eq!(guids(&s), n, "the copy carries none of the identity");
}

/// A `.fig` file with a main (guid 20) and an instance of it, as File > Open reads it.
fn fig_file(main_name: &str) -> String {
    let mut v = page_nodes();
    v.push(node(20, Some((1, "!")), "SYMBOL", main_name, vec![("size", xy(40.0, 20.0)), ("key", Kv::Str("lib".into()))]));
    v.push(node(21, Some((20, "!")), "RECTANGLE", "Face", vec![("size", xy(40.0, 20.0))]));
    v.push(instance_of(30, 20, 1, "\""));
    let bytes = figcraft_format::fig::pack_canvas(&schema(), &o(vec![("nodeChanges", Kv::Array(v))])).expect("packs");
    B64.encode(bytes)
}

#[test]
fn mains_from_opened_files_carry_no_figma_identity() {
    let mut s = Session::empty();
    s.execute("file.open", &json!({"data": fig_file("Button"), "name": "A.fig"})).unwrap();
    let doc = s.doc().unwrap();
    assert!(doc.nodes().any(|n| matches!(n.kind, NodeKind::Component { .. })));
    assert!(doc.nodes().all(|n| n.plugin_data.keys().all(|k| !k.starts_with("fig."))));
}

#[test]
fn an_instance_copied_between_two_opened_files_stays_unlinked() {
    let mut s = Session::empty();
    s.execute("file.open", &json!({"data": fig_file("Button"), "name": "A.fig"})).unwrap();
    let a_main = components(&s)[0];
    s.execute("file.open", &json!({"data": fig_file("Other"), "name": "B.fig"})).unwrap();
    let instance = s.doc().unwrap().nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap().id;
    s.execute("edit.copy", &json!({"ids": [instance]})).unwrap();
    s.active = 0;
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let r = s.execute("edit.paste", &json!({})).unwrap();
    assert_ne!(instance_main(&s, ids(&r)[0]), a_main, "the same guid in an unrelated file is not the same component");
}

#[test]
fn a_main_from_an_opened_file_is_not_matched_to_a_pasted_one() {
    let mut s = Session::empty();
    s.execute("file.open", &json!({"data": fig_file("Other"), "name": "B.fig"})).unwrap();
    let instance = s.doc().unwrap().nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap().id;
    s.execute("edit.copy", &json!({"ids": [instance]})).unwrap();
    s.execute("file.new", &json!({})).unwrap();
    let main = ids(&paste(&mut s, html(payload_main(20, None), "F1")))[0];
    // The paste above replaced the clipboard: copy again from the opened file.
    s.active = 0;
    s.execute("edit.copy", &json!({"ids": [instance]})).unwrap();
    s.active = 1;
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let r = s.execute("edit.paste", &json!({})).unwrap();
    assert_ne!(instance_main(&s, ids(&r)[0]), main);
}

/// The export schema, with the library `key` field the payloads of a library's components carry.
fn schema() -> String {
    let schema = figcraft_format::fig::EXPORT_SCHEMA.replacen("message NodeChange ", "message NodeChange string key=9999, ", 1);
    assert_ne!(schema, figcraft_format::fig::EXPORT_SCHEMA, "the key field was injected");
    schema
}

fn data(nodes: Vec<Kv<'static>>) -> String {
    B64.encode(figcraft_format::fig::pack_canvas(&schema(), &o(vec![("nodeChanges", Kv::Array(nodes))])).expect("packs"))
}

fn paste_data(s: &mut Session, data: String, file: Option<&str>) -> serde_json::Value {
    s.execute("select.set", &json!({"ids": []})).expect("clears");
    let mut p = json!({ "data": data });
    if let Some(f) = file {
        p["fileKey"] = json!(f);
    }
    s.execute("edit.pasteDesignLayers", &p).expect("pastes")
}

#[test]
fn data_with_a_file_key_links_by_guid_and_without_one_only_by_library_key() {
    let mut s = Session::new();
    let main = ids(&paste(&mut s, html(payload_main(20, None), "F1")))[0];
    let r = paste_data(&mut s, data(payload_instance(20, None, true)), Some("F1"));
    assert_eq!(instance_main(&s, ids(&r)[0]), main, "data + fileKey links by guid");
    // No file key: a guid alone never links, a library key does.
    let mut s = Session::new();
    let main = ids(&paste(&mut s, html(payload_main(20, Some("btn")), "F1"))).first().copied().unwrap();
    let r = paste_data(&mut s, data(payload_instance(20, None, true)), None);
    assert_ne!(instance_main(&s, ids(&r)[0]), main, "guid alone does not link");
    assert_eq!(components(&s), [main], "no component came along with the instance");
    assert!(r["warnings"].to_string().contains("stay unlinked"), "{r}");
    let r = paste_data(&mut s, data(payload_instance(77, Some("btn"), true)), None);
    assert_eq!(instance_main(&s, ids(&r)[0]), main, "the library key links without a file key");
}

#[test]
fn the_first_main_with_a_library_key_takes_the_paste() {
    let mut s = Session::new();
    let first = ids(&paste(&mut s, html(payload_main(20, Some("btn")), "F1")))[0];
    let second = ids(&paste(&mut s, html(payload_main(30, Some("btn")), "F2")))[0];
    assert_ne!(first, second);
    let r = paste(&mut s, html(payload_instance(77, Some("btn"), true), "F3"));
    assert_eq!(instance_main(&s, ids(&r)[0]), first, "the older component wins, every time");
    assert_eq!(components(&s).len(), 2, "no third component came along");
}

#[test]
fn pasting_the_main_after_its_instance_does_not_relink_it() {
    let mut s = Session::new();
    let copy = ids(&paste(&mut s, html(payload_instance(20, None, true), "F1")))[0];
    let gone = instance_main(&s, copy);
    paste(&mut s, html(payload_main(20, None), "F1"));
    assert_eq!(instance_main(&s, copy), gone, "the earlier instance keeps pointing at its missing main");
    assert!(!s.doc().unwrap().contains(gone));
}

#[test]
fn a_copy_of_a_pasted_instance_keeps_its_main_pointer_and_drops_the_guid() {
    let mut s = Session::new();
    let copy = ids(&paste(&mut s, html(payload_instance(20, None, true), "F1")))[0];
    let data = |s: &Session, i: NodeId| s.doc().unwrap().node(i).unwrap().plugin_data.clone();
    assert_eq!(data(&s, copy).get("fig.main").map(String::as_str), Some("1:20"));
    let dup = ids(&s.execute("edit.duplicate", &json!({"ids": [copy]})).unwrap())[0];
    let d = data(&s, dup);
    assert_eq!(d.get("fig.main").map(String::as_str), Some("1:20"), "{d:?}");
    assert!(!d.contains_key("fig.guid") && !d.contains_key("fig.key") && !d.contains_key("fig.file"), "{d:?}");
}

#[test]
fn layers_an_instance_expands_carry_no_identity() {
    let mut s = Session::new();
    let mut first = page_nodes();
    first.extend([
        node(20, Some((1, "!")), "SYMBOL", "Outer", vec![("size", xy(80.0, 40.0))]),
        instance_of(25, 40, 20, "!"),
        node(40, Some((1, "\"")), "SYMBOL", "Inner", vec![("size", xy(40.0, 20.0))]),
        node(41, Some((40, "!")), "RECTANGLE", "Face", vec![("size", xy(40.0, 20.0))]),
    ]);
    paste(&mut s, html(first, "F1"));
    let doc = s.doc().unwrap();
    let inside: Vec<_> =
        doc.nodes().filter(|n| doc.ancestors(n.id).into_iter().any(|a| matches!(doc.node(a).unwrap().kind, NodeKind::Instance { .. }))).collect();
    assert!(!inside.is_empty(), "the instance has layers of its own");
    assert!(inside.iter().all(|n| !n.plugin_data.contains_key("fig.guid")), "{:?}", inside.iter().map(|n| &n.plugin_data).collect::<Vec<_>>());
}

#[test]
fn a_file_key_is_read_from_the_clipboard_metadata_only_when_it_is_sound() {
    use crate::cmd::edit::design_clipboard_file;
    let meta = |json: &str| format!("<span>(figmeta){}(/figmeta)</span>", B64.encode(json));
    assert_eq!(design_clipboard_file(&meta(r#"{"fileKey":"F1"}"#)).as_deref(), Some("F1"));
    assert_eq!(design_clipboard_file(&meta(r#"{"fileKey":""}"#)), None, "empty");
    assert_eq!(design_clipboard_file(&meta(&format!(r#"{{"fileKey":"{}"}}"#, "k".repeat(129)))), None, "over 128 characters");
    assert_eq!(design_clipboard_file(&meta(&format!(r#"{{"fileKey":"{}"}}"#, "k".repeat(128)))).map(|k| k.len()), Some(128));
    assert_eq!(design_clipboard_file(&meta(r#"{"fileKey":7}"#)), None, "not a string");
    assert_eq!(design_clipboard_file("(figmeta)!!not base64!!(/figmeta)"), None, "malformed");
    assert_eq!(design_clipboard_file(&format!("(figmeta){}(/figmeta)", B64.encode("not json"))), None, "not json");
    assert_eq!(design_clipboard_file("(figmeta)e30="), None, "unclosed");
    assert_eq!(design_clipboard_file(&format!("(figmeta){}(/figmeta)", "A".repeat(64 * 1024 + 1))), None, "oversized");
    assert_eq!(design_clipboard_file(""), None, "empty html");
}
