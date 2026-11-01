//! Instance expansion stays near linear on crafted files (#1077): variable nodes under a main
//! aren't walked again for every instance (they were never layers, so the layer cap never stopped
//! them), and a nested instance takes only the outer instance's override rows addressed below it
//! instead of copying all of them. Overrides still reach the layers they name.

use std::time::{Duration, Instant};

use figcraft_doc::{Document, NodeId};

use super::kiwi::Kv;
use super::map;

type Fields = Vec<(&'static str, Kv<'static>)>;

fn o(fields: Fields) -> Kv<'static> {
    Kv::Object(fields)
}

fn g(id: u64) -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(id))])
}

fn node(id: u64, parent: Option<u64>, ty: &'static str, name: &str, more: Fields) -> Kv<'static> {
    let mut fields = vec![("guid", g(id)), ("phase", Kv::Enum("CREATED")), ("type", Kv::Enum(ty)), ("name", Kv::Str(name.into()))];
    if let Some(parent) = parent {
        fields.push(("parentIndex", o(vec![("guid", g(parent)), ("position", Kv::Str(format!("{id:08}")))])));
    }
    fields.extend(more);
    o(fields)
}

fn instance_of(main: u64, overrides: Vec<Kv<'static>>) -> Fields {
    vec![("symbolData", o(vec![("symbolID", g(main)), ("symbolOverrides", Kv::Array(overrides))]))]
}

/// Map and import `nodes`; the document and how long it took.
fn import(nodes: Vec<Kv<'static>>) -> (Document, Duration) {
    let msg = o(vec![("nodeChanges", Kv::Array(nodes))]);
    let start = Instant::now();
    let (json, _, networks) = map::to_rest(&msg, "Crafted", 2_000_000);
    let took = start.elapsed();
    let (doc, _) = crate::rest::import_with_networks(&json, &|_: &str| None, networks, 2_000_000).unwrap();
    (doc, took)
}

fn page_children(doc: &Document) -> Vec<NodeId> {
    let page = doc.first_page().unwrap();
    doc.node(page).unwrap().children.clone()
}

#[test]
fn variable_nodes_under_a_main_are_not_walked_for_every_instance() {
    let mut nodes = vec![node(1, None, "DOCUMENT", "Document", vec![]), node(2, Some(1), "CANVAS", "Page", vec![])];
    nodes.push(node(10, Some(2), "SYMBOL", "Main", vec![]));
    nodes.push(node(11, Some(10), "RECTANGLE", "R", vec![]));
    nodes.extend((0..5_000).map(|i| node(100_000 + i, Some(10), "VARIABLE", "V", vec![])));
    nodes.extend((0..1_000).map(|i| node(500_000 + i, Some(2), "INSTANCE", "I", instance_of(10, vec![]))));
    let (doc, took) = import(nodes);
    // Five million needless visits took seconds; now they are skipped.
    assert!(took < Duration::from_secs(5), "{took:?}");
    let kids = page_children(&doc);
    assert_eq!(kids.len(), 1_001, "the main and its instances");
    assert!(kids.iter().all(|k| doc.node(*k).unwrap().children.len() == 1), "each holds the rectangle and no variable layer");
}

#[test]
fn nested_instances_take_only_the_override_rows_below_them() {
    let mut nodes = vec![node(1, None, "DOCUMENT", "Document", vec![]), node(2, Some(1), "CANVAS", "Page", vec![])];
    nodes.push(node(10, Some(2), "SYMBOL", "Inner", vec![]));
    nodes.push(node(11, Some(10), "RECTANGLE", "R", vec![]));
    nodes.push(node(20, Some(2), "SYMBOL", "Outer", vec![]));
    nodes.extend((0..1_000).map(|i| node(30_000 + i, Some(20), "INSTANCE", "Nested", instance_of(10, vec![]))));
    // 20 000 rows spread over the nested instances (for layers they don't have), and one that
    // hides the rectangle of nested instance 7.
    let mut rows: Vec<Kv<'static>> = (0..20_000)
        .map(|i| o(vec![("guidPath", o(vec![("guids", Kv::Array(vec![g(30_000 + i % 1_000), g(900_000 + i)]))])), ("visible", Kv::Bool(false))]))
        .collect();
    rows.push(o(vec![("guidPath", o(vec![("guids", Kv::Array(vec![g(30_007), g(11)]))])), ("visible", Kv::Bool(false))]));
    nodes.push(node(40, Some(2), "INSTANCE", "Outer instance", instance_of(20, rows)));
    let (doc, took) = import(nodes);
    // Each nested instance copied all 20 000 rows: twenty million copies.
    assert!(took < Duration::from_secs(5), "{took:?}");
    let outer = *page_children(&doc).iter().find(|k| doc.node(**k).unwrap().name == "Outer instance").unwrap();
    let nested = doc.node(outer).unwrap().children.clone();
    assert_eq!(nested.len(), 1_000);
    let shown: Vec<bool> = nested.iter().map(|n| doc.node(doc.node(*n).unwrap().children[0]).unwrap().visible).collect();
    assert!(!shown[7], "the row for nested instance 7's rectangle still applies");
    assert_eq!(shown.iter().filter(|v| **v).count(), 999, "and only there");
}
