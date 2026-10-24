//! Large `.fig` files at the layer cap (#981): the budget a design-system file gets, the library
//! copies its instances need kept ahead of the pages, honest notes for what the cap kept out, and
//! one note for the cap itself.

use figcraft_doc::{Document, NodeKind};

use super::kiwi::{Kv, Schema};
use super::{MAX_FIG_NODES, export, import, layer_budget, map};

type Fields = Vec<(&'static str, Kv<'static>)>;

fn o(fields: Fields) -> Kv<'static> {
    Kv::Object(fields)
}

fn g(id: u64) -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(id))])
}

fn xy(x: f64, y: f64) -> Kv<'static> {
    o(vec![("x", Kv::Float(x as f32)), ("y", Kv::Float(y as f32))])
}

/// A layer under `parent`; siblings come in id order.
fn node(id: u64, parent: Option<u64>, ty: &'static str, name: &str, more: Fields) -> Kv<'static> {
    let mut fields = vec![("guid", g(id)), ("phase", Kv::Enum("CREATED")), ("type", Kv::Enum(ty)), ("name", Kv::Str(name.into()))];
    if let Some(parent) = parent {
        fields.push(("parentIndex", o(vec![("guid", g(parent)), ("position", Kv::Str(format!("{id:08}")))])));
    }
    fields.extend(more);
    o(fields)
}

fn instance(id: u64, page: u64, main: u64) -> Kv<'static> {
    node(id, Some(page), "INSTANCE", "Instance", vec![("symbolData", o(vec![("symbolID", g(main))]))])
}

/// A main `id` of `layers` rectangles under `parent`.
fn main_component(id: u64, parent: u64, name: &str, layers: u64) -> Vec<Kv<'static>> {
    let mut out = vec![node(id, Some(parent), "SYMBOL", name, vec![("size", xy(10.0, 10.0))])];
    out.extend((1..=layers).map(|i| node(id * 10_000 + i, Some(id), "RECTANGLE", "R", vec![("size", xy(1.0, 1.0))])));
    out
}

/// A page (2) of `count` instances of a main kept on the file's hidden library canvas (3), as a
/// design system's file carries the library components it uses.
fn library_message(count: u64, main_layers: u64) -> Kv<'static> {
    let mut nodes = vec![
        node(1, None, "DOCUMENT", "Document", vec![]),
        node(2, Some(1), "CANVAS", "Screens", vec![]),
        node(3, Some(1), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))]),
    ];
    nodes.extend(main_component(20, 3, "Library main", main_layers));
    nodes.extend((0..count).map(|i| instance(1_000 + i, 2, 20)));
    o(vec![("nodeChanges", Kv::Array(nodes))])
}

/// The `.fig` import's two stages at layer cap `cap`.
fn import_at(msg: &Kv, cap: usize) -> (Document, Vec<String>) {
    let (json, mut warnings, networks) = map::to_rest(msg, "Capped", cap);
    let (doc, more) = crate::rest::import_with_networks(&json, &|_: &str| None, networks, cap).unwrap();
    warnings.extend(more);
    (doc, warnings)
}

fn linked_to(doc: &Document, main_name: &str) -> usize {
    doc.nodes().filter(|n| matches!(n.kind, NodeKind::Instance { component, .. } if doc.node(component).is_some_and(|c| c.name == main_name))).count()
}

#[test]
fn a_design_system_sized_canvas_gets_room() {
    // The file in #981 stopped at 244 248 layers: a 7.8 MB canvas at 32 bytes a layer.
    let reported = 244_248 * 32;
    assert!(layer_budget(reported) >= 4 * 244_248, "{}", layer_budget(reported));
    // Small files still can't claim more than the REST import's cap, nor any file the ceiling.
    assert_eq!(layer_budget(1 << 20), crate::rest::MAX_NODES);
    assert_eq!(layer_budget(usize::MAX), MAX_FIG_NODES);
}

#[test]
fn library_mains_come_before_the_pages_at_the_cap() {
    // 40 instances of an 11-layer library main on a page, with room for 100 layers: mapped after
    // the pages, the main found no room and every instance became a frame of "a component that
    // isn't in the file".
    let (doc, warnings) = import_at(&library_message(40, 10), 100);
    assert!(warnings.iter().any(|w| w.contains("more than 100 layers")), "{warnings:?}");
    assert!(!warnings.iter().any(|w| w.contains("aren't in the file")), "{warnings:?}");
    let linked = linked_to(&doc, "Library main");
    assert!(linked >= 5, "{linked} instances linked to the library main: {warnings:?}");
    assert!(doc.nodes().any(|n| n.name == "Library main" && doc.is_remote(n.id)));
}

#[test]
fn instances_of_a_main_the_cap_kept_out_say_so() {
    // The page's instances come before their main: at the cap, the main is skipped though the
    // file has it.
    let mut nodes = vec![node(1, None, "DOCUMENT", "Document", vec![]), node(2, Some(1), "CANVAS", "Page", vec![])];
    nodes.extend((0..20).map(|i| instance(100 + i, 2, 900)));
    nodes.extend(main_component(900, 2, "Local main", 10));
    let (doc, warnings) = import_at(&o(vec![("nodeChanges", Kv::Array(nodes))]), 60);
    assert!(warnings.iter().any(|w| w.starts_with("instances whose component was past the layer limit were imported as frames")), "{warnings:?}");
    assert!(!warnings.iter().any(|w| w.contains("aren't in the file")), "{warnings:?}");
    assert!(!doc.nodes().any(|n| n.name == "Local main"));
}

#[test]
fn swap_defaults_the_cap_kept_out_say_so() {
    // A kept main's instance swap default names a main with no instances, which comes after
    // enough layers to be skipped at the cap.
    let swap = o(vec![
        ("id", g(70)),
        ("name", Kv::Str("Icon".into())),
        ("type", Kv::Enum("INSTANCE_SWAP")),
        ("initialValue", o(vec![("guidValue", g(900))])),
    ]);
    let mut nodes = vec![node(1, None, "DOCUMENT", "Document", vec![]), node(2, Some(1), "CANVAS", "Page", vec![])];
    nodes.push(node(50, Some(2), "SYMBOL", "Card", vec![("size", xy(10.0, 10.0)), ("componentPropDefs", Kv::Array(vec![swap]))]));
    nodes.extend(main_component(60, 2, "Filler", 40));
    nodes.extend(main_component(900, 2, "Icon main", 10));
    let (doc, warnings) = import_at(&o(vec![("nodeChanges", Kv::Array(nodes))]), 30);
    assert!(doc.nodes().any(|n| n.name == "Card"));
    assert!(!doc.nodes().any(|n| n.name == "Icon main"));
    assert!(warnings.iter().any(|w| w.starts_with("instance swap properties whose default component was past the layer limit")), "{warnings:?}");
    assert!(!warnings.iter().any(|w| w.contains("isn't in the file")), "{warnings:?}");
}

#[test]
fn a_whole_file_past_the_cap_keeps_its_library_links_and_says_so_once() {
    // A 1 001-layer library main used 210 times: about 210 000 layers in a file far too small to
    // lift the cap above the REST import's 200 000.
    let file = super::tests::canvas(&Schema::parse(export::SCHEMA).unwrap(), &library_message(210, 1_000));
    assert_eq!(layer_budget(file.len()), crate::rest::MAX_NODES);
    let (doc, warnings) = import(&file, "Design system.fig").unwrap();
    let cap: Vec<_> = warnings.iter().filter(|w| w.contains("layers; the rest were skipped")).collect();
    assert_eq!(cap.len(), 1, "{warnings:?}");
    assert!(!warnings.iter().any(|w| w.contains("aren't in the file")), "{warnings:?}");
    assert!(linked_to(&doc, "Library main") >= 190, "{warnings:?}");
}

#[test]
fn a_library_main_larger_than_the_cap_leaves_the_pages_room() {
    // A main of 300 layers with room for 100: mapped first, it took the whole budget and the page
    // came out empty.
    let (doc, warnings) = import_at(&library_message(5, 300), 100);
    let page = doc.nodes().find(|n| n.name == "Screens").expect("the page");
    let top = page.children.len();
    assert!(top >= 1, "the page kept {top} layers: {warnings:?}");
}
