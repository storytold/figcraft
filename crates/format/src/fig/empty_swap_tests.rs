//! #2206: Figma writes the empty GUID (4294967295:4294967295) in `overriddenSymbolID` for "no
//! swap". The instance keeps the main its `symbolData` names instead of becoming a frame.

use figcraft_doc::{Document, NodeId, NodeKind};

use super::kiwi::{Kv, Schema};
use super::source_identity_tests::{node, row, symbol};
use super::tests::{arr, canvas, g, o};
use super::*;

/// The GUID Figma writes for "none".
fn empty() -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(4_294_967_295)), ("localID", Kv::Uint(4_294_967_295))])
}

/// A Badge instance of main 10 whose `overriddenSymbolID` is `swap`.
fn badge(id: u64, name: &str, swap: Kv<'static>) -> Kv<'static> {
    node(id, Some(30), "INSTANCE", name, vec![("symbolData", symbol(10, vec![])), ("overriddenSymbolID", swap)])
}

/// An override row for the badge at `id` that writes the empty GUID.
fn reset(id: u64) -> Kv<'static> {
    row(&[id], vec![("overriddenSymbolID", empty())])
}

/// A Badge main with a label, a Card main holding a Badge instance whose `overriddenSymbolID` is
/// the empty GUID (as in the report) and a second one the Card swaps to an Error variant, and a
/// Card instance whose override rows write the empty GUID over both.
fn import_cards() -> (Document, Vec<String>) {
    let message = o(vec![(
        "nodeChanges",
        arr(vec![
            node(1, None, "DOCUMENT", "Document", vec![]),
            node(2, Some(1), "CANVAS", "Page", vec![]),
            node(10, Some(2), "SYMBOL", "Property 1=Informative", vec![]),
            node(11, Some(10), "RECTANGLE", "Label", vec![]),
            node(20, Some(2), "SYMBOL", "Property 1=Error", vec![]),
            node(21, Some(20), "RECTANGLE", "Error label", vec![]),
            node(30, Some(2), "SYMBOL", "Card", vec![]),
            badge(31, "Badge", empty()),
            badge(32, "Swapped badge", g(20)),
            node(40, Some(2), "INSTANCE", "Card copy", vec![("symbolData", symbol(30, vec![reset(31), reset(32)]))]),
        ]),
    )]);
    // The export schema: the shared test schema has no `overriddenSymbolID`.
    let schema = Schema::parse(export::SCHEMA).unwrap();
    import(&canvas(&schema, &message), "Empty swap.fig").unwrap()
}

fn named(doc: &Document, name: &str) -> NodeId {
    doc.nodes().find(|n| n.name == name).unwrap().id
}

fn badge_in(doc: &Document, card: &str, index: usize) -> NodeId {
    doc.node(named(doc, card)).unwrap().children[index]
}

fn main_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    match doc.node(id)?.kind {
        NodeKind::Instance { component, .. } => Some(component),
        _ => None,
    }
}

#[test]
fn an_empty_swap_guid_keeps_the_instance_of_its_main() {
    let (doc, notes) = import_cards();
    assert!(notes.is_empty(), "no missing components: {notes:?}");
    let main = named(&doc, "Property 1=Informative");
    for card in ["Card", "Card copy"] {
        let b = doc.node(badge_in(&doc, card, 0)).unwrap();
        assert!(matches!(b.kind, NodeKind::Instance { component, .. } if component == main), "{card}: {:?}", b.kind);
        let label = b.children.first().and_then(|c| doc.node(*c)).map(|n| n.name.clone());
        assert_eq!(label.as_deref(), Some("Label"), "{card}: the main's layers");
    }
}

#[test]
fn an_empty_swap_guid_on_a_row_resets_a_swap_below_it_to_the_default_main() {
    let (doc, notes) = import_cards();
    assert!(notes.is_empty(), "{notes:?}");
    // In the Card main the badge is swapped to Error; the Card instance's row writes the empty
    // GUID over that swap: back to the main its symbolData names.
    assert_eq!(main_of(&doc, badge_in(&doc, "Card", 1)), Some(named(&doc, "Property 1=Error")));
    assert_eq!(main_of(&doc, badge_in(&doc, "Card copy", 1)), Some(named(&doc, "Property 1=Informative")));
}
