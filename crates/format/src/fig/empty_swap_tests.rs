//! #2206: Figma writes the empty GUID (4294967295:4294967295) in `overriddenSymbolID` for "no
//! swap". The instance keeps the main its `symbolData` names instead of becoming a frame.

use figcraft_doc::{Document, NodeId, NodeKind};

use super::kiwi::{Kv, Schema};
use super::*;

type Fields = Vec<(&'static str, Kv<'static>)>;

fn o(fields: Fields) -> Kv<'static> {
    Kv::Object(fields)
}

fn g(id: u64) -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(id))])
}

/// The GUID Figma writes for "none".
fn empty() -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(4_294_967_295)), ("localID", Kv::Uint(4_294_967_295))])
}

fn node(id: u64, parent: Option<u64>, ty: &'static str, name: &str, more: Fields) -> Kv<'static> {
    let mut fields = vec![("guid", g(id)), ("phase", Kv::Enum("CREATED")), ("type", Kv::Enum(ty)), ("name", Kv::Str(name.into()))];
    if let Some(parent) = parent {
        fields.push(("parentIndex", o(vec![("guid", g(parent)), ("position", Kv::Str(format!("{id:04}")))])));
    }
    fields.extend(more);
    o(fields)
}

fn symbol(main: u64, overrides: Vec<Kv<'static>>) -> Kv<'static> {
    o(vec![("symbolID", g(main)), ("symbolOverrides", Kv::Array(overrides))])
}

fn row(path: &[u64], more: Fields) -> Kv<'static> {
    let mut fields = vec![("guidPath", o(vec![("guids", Kv::Array(path.iter().map(|id| g(*id)).collect()))]))];
    fields.extend(more);
    o(fields)
}

/// A Badge main with a label, a Card main holding a Badge instance whose `overriddenSymbolID` is
/// the empty GUID (as in the report) and a second one the Card swaps to an Error variant, and a
/// Card instance whose override rows write the empty GUID over both.
fn import_cards() -> (Document, Vec<String>) {
    let message = o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "DOCUMENT", "Document", vec![]),
            node(2, Some(1), "CANVAS", "Page", vec![]),
            node(10, Some(2), "SYMBOL", "Property 1=Informative", vec![]),
            node(11, Some(10), "RECTANGLE", "Label", vec![]),
            node(20, Some(2), "SYMBOL", "Property 1=Error", vec![]),
            node(21, Some(20), "RECTANGLE", "Error label", vec![]),
            node(30, Some(2), "SYMBOL", "Card", vec![]),
            node(31, Some(30), "INSTANCE", "Badge", vec![("symbolData", symbol(10, vec![])), ("overriddenSymbolID", empty())]),
            node(32, Some(30), "INSTANCE", "Swapped badge", vec![("symbolData", symbol(10, vec![])), ("overriddenSymbolID", g(20))]),
            node(
                40,
                Some(2),
                "INSTANCE",
                "Card copy",
                vec![(
                    "symbolData",
                    symbol(30, vec![row(&[31], vec![("overriddenSymbolID", empty())]), row(&[32], vec![("overriddenSymbolID", empty())])]),
                )],
            ),
        ]),
    )]);
    let schema = Schema::parse(export::SCHEMA).unwrap();
    import(&super::tests::canvas(&schema, &message), "Empty swap.fig").unwrap()
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
