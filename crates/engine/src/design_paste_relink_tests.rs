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
}

#[test]
fn the_same_library_key_links_whatever_the_guids() {
    let mut s = Session::new();
    let main = ids(&paste(&mut s, html(payload_main(20, Some("btn")), "F1")))[0];
    let r = paste(&mut s, html(payload_instance(77, Some("btn"), true), "F2"));
    let copy = ids(&r)[0];
    assert_eq!(instance_main(&s, copy), main);
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

/// The export schema, with the library `key` field the payloads of a library's components carry.
fn schema() -> String {
    figcraft_format::fig::EXPORT_SCHEMA.replacen("message NodeChange ", "message NodeChange string key=9999, ", 1)
}
