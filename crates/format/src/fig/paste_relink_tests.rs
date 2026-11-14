//! Layers pasted from another design app keep the identity they had there (#2022): a main's guid
//! (and its library key) lands in its plugin data, on the main and on every layer in it, so
//! instances pasted later can link to it. Built in code from synthetic payloads.

use figcraft_doc::{Document, Node, NodeKind};

use super::kiwi::Kv;
use super::tests::{arr, canvas, g, node, o, s, schema, xy};
use super::try_import_pasted;

fn import(nodes: Vec<Kv<'static>>) -> Document {
    let bytes = canvas(&schema(), &o(vec![("nodeChanges", arr(nodes))]));
    try_import_pasted(&bytes, "Pasted layers").expect("design data").expect("it imports").0
}

fn named<'a>(doc: &'a Document, name: &str) -> &'a Node {
    doc.nodes().find(|n| n.name == name).expect(name)
}

#[test]
fn a_pasted_main_records_its_figma_identity() {
    let doc = import(vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(20, Some((1, "!")), "SYMBOL", "Button", vec![("size", xy(40.0, 20.0)), ("key", s("btn"))]),
        node(21, Some((20, "!")), "RECTANGLE", "Face", vec![("size", xy(40.0, 20.0))]),
        node(30, Some((1, "\"")), "RECTANGLE", "Loose", vec![("size", xy(5.0, 5.0))]),
    ]);
    let main = named(&doc, "Button");
    assert!(matches!(main.kind, NodeKind::Component { .. }));
    assert_eq!(main.plugin_data.get("fig.guid").map(String::as_str), Some("1:20"));
    assert_eq!(main.plugin_data.get("fig.key").map(String::as_str), Some("btn"));
    let face = named(&doc, "Face");
    assert_eq!(face.plugin_data.get("fig.guid").map(String::as_str), Some("1:21"), "every layer of a main");
    assert!(!face.plugin_data.contains_key("fig.key"));
    assert!(named(&doc, "Loose").plugin_data.is_empty(), "layers outside mains record nothing");
}

#[test]
fn a_cached_main_in_the_internal_canvas_is_an_instance_main_in_the_store() {
    let doc = import(vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(2, Some((0, "~")), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))]),
        node(30, Some((1, "!")), "INSTANCE", "Copy", vec![("size", xy(40.0, 20.0)), ("symbolData", o(vec![("symbolID", g(20))]))]),
        node(20, Some((2, "!")), "SYMBOL", "Button", vec![("size", xy(40.0, 20.0))]),
        node(21, Some((20, "!")), "RECTANGLE", "Face", vec![("size", xy(40.0, 20.0))]),
    ]);
    let copy = doc.nodes().find(|n| n.name == "Copy").expect("the instance");
    let NodeKind::Instance { component, .. } = &copy.kind else { panic!("the top is not an instance: {:?}", copy.kind) };
    assert!(doc.is_remote(*component), "its main is in the library store");
    let main = doc.node(*component).expect("main");
    assert_eq!(main.plugin_data.get("fig.guid").map(String::as_str), Some("1:20"));
    assert_eq!(copy.plugin_data.get("fig.main").map(String::as_str), Some("1:20"));
}

fn exported_warnings(data: &[(&str, &str)]) -> Vec<String> {
    let mut doc = Document::new("x");
    let page = doc.first_page().expect("a page");
    let id = doc.alloc_id();
    let mut n = Node::new(id, NodeKind::Frame, "A", 10.0, 10.0);
    for (k, v) in data {
        n.plugin_data.insert((*k).into(), (*v).into());
    }
    doc.insert(n, page, None).expect("inserts");
    super::export(&doc, None).expect("exports").1
}

#[test]
fn identity_keys_alone_do_not_warn_on_export_but_other_plugin_data_does() {
    let has = |w: &[String]| w.iter().any(|m| m.contains("plugin data"));
    assert!(!has(&exported_warnings(&[("fig.guid", "1:2"), ("fig.key", "k"), ("fig.file", "F"), ("fig.main", "1:3")])));
    assert!(has(&exported_warnings(&[("fig.guid", "1:2"), ("other", "x")])));
    assert!(has(&exported_warnings(&[("other", "x")])));
}

#[test]
fn pack_canvas_refuses_a_schema_that_does_not_parse_or_has_no_message() {
    let msg = o(vec![("nodeChanges", arr(vec![]))]);
    assert!(super::pack_canvas("not a schema", &msg).is_err());
    assert!(super::pack_canvas("enum A X=0", &msg).is_err(), "no Message");
    assert!(super::pack_canvas(super::EXPORT_SCHEMA, &msg).is_ok());
}
