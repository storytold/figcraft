//! Files nested as deep as the importers go, on the browser's stack (#1101): a `.fig` file and a
//! REST file 256 layers deep (and deeper, which the importers cut off) import on a 1 MB stack, the
//! size wasm32 gets. The REST import's layer walk used about 4 KB of stack a level, a megabyte at
//! 256; it now runs on a work list. The `.fig` mapping stays recursive at about 1.8 KB a level.

use serde_json::{Value, json};

use super::kiwi::Kv;
use super::map;

/// The stack wasm32 runs on.
const WASM_STACK: usize = 1 << 20;

fn on_wasm_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(WASM_STACK).spawn(f).unwrap().join().unwrap()
}

type Fields = Vec<(&'static str, Kv<'static>)>;

fn o(fields: Fields) -> Kv<'static> {
    Kv::Object(fields)
}

fn g(id: u64) -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(id))])
}

fn node(id: u64, parent: Option<u64>, ty: &'static str) -> Kv<'static> {
    let mut fields = vec![("guid", g(id)), ("phase", Kv::Enum("CREATED")), ("type", Kv::Enum(ty)), ("name", Kv::Str(format!("L{id}")))];
    if let Some(parent) = parent {
        fields.push(("parentIndex", o(vec![("guid", g(parent)), ("position", Kv::Str("a".into()))])));
    }
    o(fields)
}

/// A `.fig` message: a page holding `depth` frames, each inside the last.
fn deep_fig(depth: u64) -> Kv<'static> {
    let mut nodes = vec![node(1, None, "DOCUMENT"), node(2, Some(1), "CANVAS")];
    nodes.extend((0..depth).map(|i| node(10 + i, Some(if i == 0 { 2 } else { 9 + i }), "FRAME")));
    o(vec![("nodeChanges", Kv::Array(nodes))])
}

/// A REST file: a page holding `depth` frames, each inside the last.
fn deep_rest(depth: usize) -> Value {
    let mut layer = json!({"id": format!("1:{depth}"), "type": "FRAME", "name": "Innermost"});
    for i in (0..depth - 1).rev() {
        layer = json!({"id": format!("1:{i}"), "type": "FRAME", "name": format!("L{i}"), "children": [layer]});
    }
    json!({"name": "Deep", "document": {"id": "0:0", "type": "DOCUMENT", "children": [{"id": "0:1", "type": "CANVAS", "name": "Page", "children": [layer]}]}})
}

/// The `.fig` import of a file `depth` frames deep on the wasm stack: (layers, warnings).
fn fig_import(depth: u64) -> (usize, Vec<String>) {
    on_wasm_stack(move || {
        let msg = deep_fig(depth);
        let (json, mut warnings, networks) = map::to_rest(&msg, "Deep", 2_000_000);
        let (doc, more) = crate::rest::import_with_networks(&json, &|_: &str| None, networks, 2_000_000).unwrap();
        warnings.extend(more);
        (doc.node_count(), warnings)
    })
}

#[test]
fn a_fig_file_as_deep_as_the_import_goes_fits_the_wasm_stack() {
    let (layers, warnings) = fig_import(256);
    assert_eq!(layers, 257, "the page and every frame");
    assert!(!warnings.iter().any(|w| w.contains("deeper")), "{warnings:?}");
    // Deeper than that, the rest is cut off with a note (still on the small stack).
    let (layers, warnings) = fig_import(400);
    assert!(layers < 400, "{layers}");
    assert!(warnings.iter().any(|w| w.contains("nested deeper than 256 levels")), "{warnings:?}");
}

#[test]
fn a_rest_file_as_deep_as_the_import_goes_fits_the_wasm_stack() {
    let (layers, warnings) = on_wasm_stack(|| {
        let v = deep_rest(256);
        let (doc, warnings) = crate::rest::import(&v, &|_: &str| None).unwrap();
        (doc.node_count(), warnings)
    });
    assert_eq!(layers, 257, "the page and every frame");
    assert!(!warnings.iter().any(|w| w.contains("deeper")), "{warnings:?}");
    let (layers, warnings) = on_wasm_stack(|| {
        let v = deep_rest(400);
        let (doc, warnings) = crate::rest::import(&v, &|_: &str| None).unwrap();
        (doc.node_count(), warnings)
    });
    assert!(layers < 400, "{layers}");
    assert!(warnings.iter().any(|w| w.contains("nested deeper than 256 levels")), "{warnings:?}");
}

#[test]
fn the_work_list_keeps_the_files_order_and_fits_groups_to_their_children() {
    // Siblings in order, depth first, and a group's bounds from its children once they are in.
    let v = json!({"name": "Order", "document": {"id": "0:0", "type": "DOCUMENT", "children": [{"id": "0:1", "type": "CANVAS", "name": "Page", "children": [
        {"id": "1:1", "type": "GROUP", "name": "G", "absoluteBoundingBox": {"x": 0, "y": 0, "width": 1, "height": 1}, "children": [
            {"id": "1:2", "type": "RECTANGLE", "name": "A", "absoluteBoundingBox": {"x": 10, "y": 20, "width": 30, "height": 40}},
            {"id": "1:3", "type": "FRAME", "name": "B", "absoluteBoundingBox": {"x": 50, "y": 20, "width": 10, "height": 10}, "children": [
                {"id": "1:4", "type": "RECTANGLE", "name": "C", "absoluteBoundingBox": {"x": 52, "y": 22, "width": 2, "height": 2}}
            ]}
        ]},
        {"id": "1:5", "type": "RECTANGLE", "name": "D", "absoluteBoundingBox": {"x": 0, "y": 100, "width": 5, "height": 5}}
    ]}]}});
    let (doc, _) = crate::rest::import(&v, &|_: &str| None).unwrap();
    let page = doc.first_page().unwrap();
    let name = |id| doc.node(id).unwrap().name.clone();
    let top: Vec<String> = doc.node(page).unwrap().children.iter().map(|c| name(*c)).collect();
    assert_eq!(top, ["G", "D"]);
    let g = doc.node(page).unwrap().children[0];
    let kids: Vec<String> = doc.node(g).unwrap().children.iter().map(|c| name(*c)).collect();
    assert_eq!(kids, ["A", "B"]);
    let b = doc.node(g).unwrap().children[1];
    assert_eq!(doc.node(b).unwrap().children.iter().map(|c| name(*c)).collect::<Vec<_>>(), ["C"]);
    let group = doc.node(g).unwrap();
    assert_eq!((group.width, group.height), (50.0, 40.0), "the group fits A and B");
}
