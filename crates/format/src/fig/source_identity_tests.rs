//! Synthetic .fig source identity: cached library components, contextual sources, main switches
//! and invalid provenance.

use figcraft_doc::color::PaintKind;
use figcraft_doc::{Document, NodeId, NodeKind, PropDef};
use serde_json::{Value, json};

use super::kiwi::{Kv, Schema};
use super::*;

type Fields = Vec<(&'static str, Kv<'static>)>;

fn o(fields: Fields) -> Kv<'static> {
    Kv::Object(fields)
}

fn g(id: u64) -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(id))])
}

fn s(value: &str) -> Kv<'static> {
    Kv::Str(value.into())
}

fn matrix(x: f32) -> Kv<'static> {
    o(vec![
        ("m00", Kv::Float(1.0)),
        ("m01", Kv::Float(0.0)),
        ("m02", Kv::Float(x)),
        ("m10", Kv::Float(0.0)),
        ("m11", Kv::Float(1.0)),
        ("m12", Kv::Float(0.0)),
    ])
}

fn node(id: u64, parent: Option<u64>, ty: &'static str, name: &str, more: Fields) -> Kv<'static> {
    let mut fields = vec![("guid", g(id)), ("phase", Kv::Enum("CREATED")), ("type", Kv::Enum(ty)), ("name", s(name))];
    if let Some(parent) = parent {
        fields.push(("parentIndex", o(vec![("guid", g(parent)), ("position", s(&format!("{id:04}")))])));
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

fn solid(r: f32, g: f32, b: f32) -> Kv<'static> {
    o(vec![
        ("type", Kv::Enum("SOLID")),
        ("color", o(vec![("r", Kv::Float(r)), ("g", Kv::Float(g)), ("b", Kv::Float(b)), ("a", Kv::Float(1.0))])),
        ("opacity", Kv::Float(1.0)),
        ("visible", Kv::Bool(true)),
    ])
}

fn schema() -> Schema {
    Schema::parse(export::SCHEMA).unwrap()
}

fn cached_component_message() -> Kv<'static> {
    o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "DOCUMENT", "Document", vec![]),
            node(2, Some(1), "CANVAS", "Visible page", vec![]),
            node(
                3,
                Some(1),
                "CANVAS",
                "Remote library",
                vec![
                    ("internalOnly", Kv::Bool(true)),
                    (
                        "variableModeBySetMap",
                        o(vec![("entries", Kv::Array(vec![o(vec![("variableSetID", o(vec![("guid", g(30))])), ("variableModeID", g(52))])]))]),
                    ),
                ],
            ),
            node(
                30,
                Some(3),
                "VARIABLE_SET",
                "Remote theme",
                vec![(
                    "variableSetModes",
                    Kv::Array(vec![
                        o(vec![("id", g(51)), ("name", s("Light")), ("sortPosition", s("!"))]),
                        o(vec![("id", g(52)), ("name", s("Dark")), ("sortPosition", s("\""))]),
                    ]),
                )],
            ),
            node(
                9,
                Some(3),
                "FRAME",
                "Cached component set",
                vec![("isStateGroup", Kv::Bool(true)), ("size", o(vec![("x", Kv::Float(24.0)), ("y", Kv::Float(24.0))])), ("transform", matrix(0.0))],
            ),
            node(
                10,
                Some(9),
                "SYMBOL",
                "Cached remote icon",
                vec![("size", o(vec![("x", Kv::Float(24.0)), ("y", Kv::Float(24.0))])), ("transform", matrix(0.0))],
            ),
            node(
                11,
                Some(10),
                "ROUNDED_RECTANGLE",
                "Cached path",
                vec![("size", o(vec![("x", Kv::Float(24.0)), ("y", Kv::Float(24.0))])), ("transform", matrix(0.0))],
            ),
            node(
                12,
                Some(10),
                "INSTANCE",
                "Nested cached instance",
                vec![
                    ("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])),
                    ("transform", matrix(0.0)),
                    ("symbolData", symbol(22, vec![row(&[23], vec![("fillPaints", Kv::Array(vec![solid(0.2, 0.7, 0.1)]))])])),
                ],
            ),
            node(
                13,
                Some(9),
                "SYMBOL",
                "Nested cached main",
                vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
            ),
            node(
                14,
                Some(13),
                "ROUNDED_RECTANGLE",
                "Nested cached path",
                vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
            ),
            node(
                15,
                Some(10),
                "INSTANCE",
                "Nested internal instance",
                vec![
                    ("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])),
                    ("transform", matrix(0.0)),
                    ("symbolData", symbol(13, vec![])),
                ],
            ),
            node(
                20,
                Some(2),
                "INSTANCE",
                "Visible icon instance",
                vec![
                    ("size", o(vec![("x", Kv::Float(24.0)), ("y", Kv::Float(24.0))])),
                    ("transform", matrix(0.0)),
                    ("symbolData", symbol(10, vec![])),
                ],
            ),
            node(
                22,
                Some(2),
                "SYMBOL",
                "Visible nested main",
                vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
            ),
            node(
                23,
                Some(22),
                "ROUNDED_RECTANGLE",
                "Visible nested path",
                vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
            ),
        ]),
    )])
}

fn cached_component_fixture() -> Vec<u8> {
    let schema = schema();
    super::tests::canvas(&schema, &cached_component_message())
}

fn cached_component_override_fixture() -> Vec<u8> {
    let mut message = cached_component_message();
    let Kv::Object(fields) = &mut message else { panic!("cached component fixture has an unexpected wire shape") };
    let changes = &mut fields.iter_mut().find(|(key, _)| *key == "nodeChanges").unwrap().1;
    let Kv::Array(nodes) = changes else { panic!("cached component fixture has an unexpected wire shape") };
    let instance = nodes.iter_mut().find(|node| node.get("name").and_then(Kv::str) == Some("Visible icon instance")).unwrap();
    let Kv::Object(fields) = instance else { panic!("cached component fixture has an unexpected wire shape") };
    fields.iter_mut().find(|(key, _)| *key == "symbolData").unwrap().1 =
        symbol(10, vec![row(&[12], vec![("symbolData", symbol(22, vec![row(&[26], vec![("overriddenSymbolID", g(24))])]))])]);
    nodes.extend([
        node(
            24,
            Some(3),
            "SYMBOL",
            "Overridden cached main",
            vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
        ),
        node(
            25,
            Some(24),
            "ROUNDED_RECTANGLE",
            "Overridden cached path",
            vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
        ),
        node(
            26,
            Some(22),
            "INSTANCE",
            "Deep cached instance",
            vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0)), ("symbolData", symbol(27, vec![]))],
        ),
        node(
            27,
            Some(3),
            "SYMBOL",
            "Base deep main",
            vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
        ),
        node(
            28,
            Some(27),
            "ROUNDED_RECTANGLE",
            "Base deep path",
            vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
        ),
    ]);
    super::tests::canvas(&schema(), &message)
}

fn cached_component_default_swap_fixture() -> Vec<u8> {
    let mut message = cached_component_message();
    let Kv::Object(fields) = &mut message else { panic!("cached component fixture has an unexpected wire shape") };
    let changes = &mut fields.iter_mut().find(|(key, _)| *key == "nodeChanges").unwrap().1;
    let Kv::Array(nodes) = changes else { panic!("cached component fixture has an unexpected wire shape") };
    let owner = nodes.iter_mut().find(|node| node.get("name").and_then(Kv::str) == Some("Visible nested main")).unwrap();
    let Kv::Object(fields) = owner else { panic!("cached component fixture has an unexpected wire shape") };
    fields.push((
        "componentPropDefs",
        Kv::Array(vec![o(vec![
            ("id", g(90)),
            ("name", s("Icon")),
            ("type", Kv::Enum("INSTANCE_SWAP")),
            ("initialValue", o(vec![("guidValue", g(24))])),
        ])]),
    ));
    nodes.extend([
        node(
            24,
            Some(3),
            "SYMBOL",
            "Default cached main",
            vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
        ),
        node(
            25,
            Some(24),
            "ROUNDED_RECTANGLE",
            "Default cached path",
            vec![("size", o(vec![("x", Kv::Float(12.0)), ("y", Kv::Float(12.0))])), ("transform", matrix(0.0))],
        ),
    ]);
    super::tests::canvas(&schema(), &message)
}

fn assert_default_swap(doc: &Document) {
    let owner = named_node(doc, "Visible nested main");
    let target = named_node(doc, "Default cached main");
    let NodeKind::Component { props, .. } = &doc.node(owner).unwrap().kind else { panic!("visible component lost its property definition") };
    assert_eq!(props.get("Icon"), Some(&PropDef::InstanceSwap { default: target, preferred: Vec::new() }), "{props:?}");
    assert!(doc.is_remote(target));
}

fn named_node(doc: &Document, name: &str) -> NodeId {
    doc.nodes().find(|node| node.name == name).unwrap().id
}

fn assert_dark_remote_mode(doc: &Document, node: NodeId) {
    let collection = doc.variables.collections.iter().find(|collection| collection.name == "Remote theme").unwrap();
    let dark = collection.modes.iter().find(|mode| mode.name == "Dark").unwrap();
    assert_eq!(doc.modes_for(node).get(&collection.id), Some(&dark.id));
}

fn assert_no_linked_library_records(doc: &Document) {
    assert!(doc.libraries.enabled.is_empty());
    assert!(doc.libraries.components.is_empty());
    assert!(doc.libraries.styles.is_empty());
    assert!(doc.libraries.collections.is_empty());
    assert!(doc.libraries.variables.is_empty());
}

#[test]
fn hidden_cached_component_is_retained_and_linked() {
    let (doc, warnings) = import(&cached_component_fixture(), "Cached library.fig").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(doc.pages.len(), 1, "hidden cache must not become a visible page");
    let check = |doc: &Document| {
        let store = doc.library_store().expect("private .fig cache is retained");
        let component = named_node(doc, "Cached remote icon");
        let path = doc.node(component).unwrap().children[0];
        let set = doc.node(component).unwrap().parent.unwrap();
        assert!(matches!(doc.node(set).unwrap().kind, NodeKind::ComponentSet { .. }));
        assert_eq!(doc.node(doc.node(set).unwrap().parent.unwrap()).unwrap().parent, Some(store));
        assert!(doc.is_remote(component) && doc.is_remote(path));
        assert!(matches!(doc.node(component).unwrap().kind, NodeKind::Component { .. }));
        assert_dark_remote_mode(doc, component);
        assert_no_linked_library_records(doc);
        // The visible instance resolves to the cached main, and its layer to the main's layer.
        let instance = named_node(doc, "Visible icon instance");
        assert!(matches!(doc.node(instance).unwrap().kind, NodeKind::Instance { component: linked, .. } if linked == component));
        assert_eq!(doc.node(doc.node(instance).unwrap().children[0]).unwrap().source, Some(path));
        // A cached main nested in another cached main is kept too, and so is an instance in the
        // cache of a visible main, with the override its row carries.
        let nested_main = named_node(doc, "Nested cached main");
        let nested = named_node(doc, "Nested internal instance");
        assert!(doc.is_remote(nested_main));
        assert!(matches!(doc.node(nested).unwrap().kind, NodeKind::Instance { component: linked, .. } if linked == nested_main));
        let external_main = named_node(doc, "Visible nested main");
        let external = doc
            .node(component)
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|id| doc.node(*id).is_some_and(|n| n.name == "Nested cached instance"))
            .unwrap();
        assert!(matches!(doc.node(external).unwrap().kind, NodeKind::Instance { component: linked, .. } if linked == external_main));
        let external_child = doc.node(external).unwrap().children[0];
        assert_eq!(doc.node(external_child).unwrap().source, Some(doc.node(external_main).unwrap().children[0]));
        assert_eq!(fill_rgb_node(doc, external_child), Some([f64::from(0.2_f32), f64::from(0.7_f32), f64::from(0.1_f32)]));
    };
    check(&doc);
    let (saved, warnings) = crate::load_bytes(&crate::save_bytes(&doc, None).unwrap()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    check(&saved);
}

#[test]
fn visible_instance_override_retains_and_links_hidden_cached_main() {
    let (doc, warnings) = import(&cached_component_override_fixture(), "Cached override.fig").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let nested = named_node(&doc, "Nested cached instance");
    let deep = named_node(&doc, "Deep cached instance");
    assert_eq!(doc.node(deep).unwrap().parent, Some(nested));
    let NodeKind::Instance { component: overridden, .. } = doc.node(deep).unwrap().kind else { panic!("nested-row override lost its instance") };
    let overridden = doc.node(overridden).unwrap();
    assert_eq!(overridden.name, "Overridden cached main");
    assert!(doc.is_remote(overridden.id));
    assert_eq!(doc.node(doc.node(deep).unwrap().children[0]).unwrap().source, Some(overridden.children[0]));
}

#[test]
fn visible_component_default_swap_retains_hidden_cached_main() {
    let (doc, warnings) = import(&cached_component_default_swap_fixture(), "Cached default swap.fig").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_default_swap(&doc);
    let (saved, _) = crate::load_bytes(&crate::save_bytes(&doc, None).unwrap()).unwrap();
    assert_default_swap(&saved);
}

#[test]
fn public_rest_import_ignores_private_fig_provenance() {
    let value = json!({
        "name": "Public REST",
        "document": {"type": "DOCUMENT", "children": [{"type": "CANVAS", "name": "Page", "children": []}]},
        "_figLibraryContexts": [{"type": "FRAME", "name": "Private cache", "children": [{"type": "COMPONENT", "name": "Must not import", "children": []}]}]
    });
    let (doc, warnings) = crate::rest::import(&value, &|_| None).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(doc.library_store().is_none() && doc.nodes().all(|node| node.name != "Must not import"));
    // Valid provenance on mapped layers links nothing either.
    for message in [message(false), external_namespace_message()] {
        let (json, _, _) = super::map::to_rest(&message, "Identity", crate::rest::MAX_NODES);
        let (doc, _) = crate::rest::import(&json, &|_| None).unwrap();
        assert!(doc.nodes().all(|n| n.source.is_none()));
    }
}

#[test]
fn the_last_change_of_a_guid_wins_and_removed_changes_delete() {
    let moved = node(12, Some(11), "FRAME", "Moved frame", vec![]);
    let mut removed = node(13, Some(10), "FRAME", "Removed frame", vec![]);
    let Kv::Object(fields) = &mut removed else { panic!() };
    fields.iter_mut().find(|(key, _)| *key == "phase").unwrap().1 = Kv::Enum("REMOVED");
    let message = o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "DOCUMENT", "Document", vec![]),
            node(2, Some(1), "CANVAS", "Page", vec![]),
            node(10, Some(2), "FRAME", "Left", vec![]),
            node(11, Some(2), "FRAME", "Right", vec![]),
            node(12, Some(10), "FRAME", "Moved frame", vec![]),
            node(13, Some(10), "FRAME", "Removed frame", vec![]),
            moved,
            removed,
        ]),
    )]);
    let (doc, _) = import(&super::tests::canvas(&schema(), &message), "Changes.fig").unwrap();
    assert_eq!(doc.node(named_node(&doc, "Moved frame")).unwrap().parent, Some(named_node(&doc, "Right")));
    assert!(doc.nodes().all(|n| n.name != "Removed frame"));
    assert!(doc.node(named_node(&doc, "Left")).unwrap().children.is_empty());
}

#[test]
fn internal_library_parent_cycles_are_bounded() {
    let message = o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "CANVAS", "Visible page", vec![]),
            node(2, Some(1), "INSTANCE", "Visible cyclic main", vec![("symbolData", symbol(11, vec![]))]),
            node(3, Some(9), "CANVAS", "Cyclic library", vec![("internalOnly", Kv::Bool(true))]),
            node(9, Some(11), "FRAME", "Loop frame", vec![]),
            node(10, Some(3), "SYMBOL", "Unrelated component", vec![]),
            node(11, Some(3), "SYMBOL", "Loop component", vec![]),
        ]),
    )]);
    let (doc, warnings) = import(&super::tests::canvas(&schema(), &message), "Cyclic library.fig").unwrap();
    assert!(warnings.iter().any(|warning| warning.contains("deeper than")), "{warnings:?}");
    assert!(doc.node_count() < 1000, "the loop is cut at the nesting limit");
}

#[test]
fn parent_loops_through_the_internal_canvas_end() {
    // The canvas is a child of a frame inside one of its own symbols, and the used main sits below
    // that frame: the walk up from the main reaches the canvas again.
    let message = o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "CANVAS", "Visible page", vec![]),
            node(2, Some(1), "INSTANCE", "Uses the main", vec![("symbolData", symbol(12, vec![]))]),
            node(3, Some(9), "CANVAS", "Loop library", vec![("internalOnly", Kv::Bool(true))]),
            node(9, Some(11), "FRAME", "Loop frame", vec![]),
            node(11, Some(3), "SYMBOL", "Loop symbol", vec![]),
            node(12, Some(9), "SYMBOL", "Used main", vec![]),
        ]),
    )]);
    let (doc, _) = import(&super::tests::canvas(&schema(), &message), "Loop.fig").unwrap();
    assert!(doc.nodes().any(|n| n.name == "Used main"));
}

fn fill_rgb_node(doc: &Document, id: NodeId) -> Option<[f64; 3]> {
    let node = doc.node(id)?;
    match &node.fills.first()?.kind {
        PaintKind::Solid { color } => Some([f64::from(color.r), f64::from(color.g), f64::from(color.b)]),
        _ => None,
    }
}

fn alias_schema() -> Schema {
    let schema = export::SCHEMA.replace("message NodeChange GUID guid=1,", "message NodeChange GUID guid=1, GUID overrideKey=500,");
    Schema::parse(&schema).unwrap()
}

fn alias(id: u64) -> Fields {
    vec![("overrideKey", g(id))]
}

fn import_alias_message(message: Kv<'static>) -> Document {
    let schema = alias_schema();
    let bytes = super::tests::canvas(&schema, &message);
    let (doc, warnings) = import(&bytes, "Alias.fig").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    doc
}

fn fill_rgb(doc: &Document, name: &str) -> Option<[f64; 3]> {
    let node = doc.nodes().find(|n| n.name == name)?;
    fill_rgb_node(doc, node.id)
}

fn alias_priority_message() -> Kv<'static> {
    let nested_local = symbol(10, vec![row(&[112], vec![("fillPaints", Kv::Array(vec![solid(0.0, 0.0, 1.0)]))])]);
    o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "DOCUMENT", "Document", vec![]),
            node(2, Some(1), "CANVAS", "Page", vec![]),
            node(10, Some(2), "SYMBOL", "Inner main", vec![]),
            node(11, Some(10), "FRAME", "Normal frame", vec![]),
            node(12, Some(11), "ROUNDED_RECTANGLE", "Alias leaf", alias(112)),
            node(20, Some(2), "SYMBOL", "Outer main", vec![]),
            node(
                21,
                Some(20),
                "INSTANCE",
                "Nested slot",
                vec![
                    ("overrideKey", g(221)),
                    ("symbolData", nested_local),
                    ("derivedSymbolData", Kv::Array(vec![row(&[112], vec![("opacity", Kv::Float(0.25))])])),
                ],
            ),
            node(
                30,
                Some(2),
                "INSTANCE",
                "Alias copy",
                vec![
                    (
                        "symbolData",
                        symbol(
                            20,
                            vec![
                                row(&[221, 112], vec![("fillPaints", Kv::Array(vec![solid(0.0, 1.0, 0.0)]))]),
                                row(&[21, 12], vec![("fillPaints", Kv::Array(vec![solid(1.0, 0.0, 0.0)]))]),
                            ],
                        ),
                    ),
                    ("derivedSymbolData", Kv::Array(vec![row(&[221, 112], vec![("fillPaints", Kv::Array(vec![solid(1.0, 0.5, 0.0)]))])])),
                ],
            ),
        ]),
    )])
}

#[test]
fn override_aliases_normalize_lazily_and_keep_outer_wire_order() {
    let doc = import_alias_message(alias_priority_message());
    let main = named(&doc, "Inner main");
    let leaf = child(&doc, child(&doc, main, 0), 0);
    let outer_main = named(&doc, "Outer main");
    let main_slot = child(&doc, outer_main, 0);
    let main_leaf = child(&doc, child(&doc, main_slot, 0), 0);
    let copy = named(&doc, "Alias copy");
    let nested = child(&doc, copy, 0);
    let frame = child(&doc, nested, 0);
    let expanded_leaf = child(&doc, frame, 0);
    assert_eq!(doc.node(main_leaf).unwrap().source, Some(leaf));
    assert_eq!(doc.node(expanded_leaf).unwrap().source, Some(main_leaf));
    assert_eq!(fill_rgb_node(&doc, expanded_leaf), Some([1.0, 0.5, 0.0]));
    assert_eq!(doc.node(expanded_leaf).unwrap().opacity, 0.25);
    assert_eq!(fill_rgb_node(&doc, leaf), None, "source node stays unchanged");
}

#[test]
fn selected_swapped_main_owns_alias_resolution_scope() {
    let message = o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "DOCUMENT", "Document", vec![]),
            node(2, Some(1), "CANVAS", "Page", vec![]),
            node(10, Some(2), "SYMBOL", "Old main", vec![]),
            node(11, Some(10), "ROUNDED_RECTANGLE", "Old leaf", alias(112)),
            node(20, Some(2), "SYMBOL", "Swapped main", vec![]),
            node(21, Some(20), "ROUNDED_RECTANGLE", "Swapped leaf", alias(112)),
            node(
                30,
                Some(2),
                "INSTANCE",
                "Swap",
                vec![
                    ("overriddenSymbolID", g(20)),
                    ("symbolData", symbol(10, vec![row(&[112], vec![("fillPaints", Kv::Array(vec![solid(1.0, 0.0, 0.0)]))])])),
                ],
            ),
        ]),
    )]);
    let doc = import_alias_message(message);
    let copy = named(&doc, "Swap");
    let source = named(&doc, "Swapped leaf");
    let expanded = child(&doc, copy, 0);
    assert_eq!(doc.node(expanded).unwrap().source, Some(source));
    assert_eq!(fill_rgb_node(&doc, expanded), Some([1.0, 0.0, 0.0]));
    assert_eq!(fill_rgb(&doc, "Old leaf"), None);
}

#[test]
fn ambiguous_alias_is_ignored_but_current_guid_still_matches() {
    let message = o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "DOCUMENT", "Document", vec![]),
            node(2, Some(1), "CANVAS", "Page", vec![]),
            node(10, Some(2), "SYMBOL", "Main", vec![]),
            node(11, Some(10), "ROUNDED_RECTANGLE", "First", alias(13)),
            node(12, Some(10), "ROUNDED_RECTANGLE", "Second", alias(112)),
            node(13, Some(10), "ROUNDED_RECTANGLE", "Current GUID", alias(112)),
            node(
                20,
                Some(2),
                "INSTANCE",
                "Copy",
                vec![(
                    "symbolData",
                    symbol(
                        10,
                        vec![
                            row(&[112], vec![("fillPaints", Kv::Array(vec![solid(1.0, 0.0, 0.0)]))]),
                            row(&[13], vec![("fillPaints", Kv::Array(vec![solid(0.0, 0.0, 1.0)]))]),
                        ],
                    ),
                )],
            ),
        ]),
    )]);
    let doc = import_alias_message(message);
    let copy = named(&doc, "Copy");
    let first = child(&doc, copy, 0);
    let second = child(&doc, copy, 1);
    let current = child(&doc, copy, 2);
    assert_eq!(doc.node(first).unwrap().source, Some(named(&doc, "First")));
    assert_eq!(fill_rgb_node(&doc, first), None, "ambiguous alias must not be guessed");
    assert_eq!(doc.node(second).unwrap().source, Some(named(&doc, "Second")));
    assert_eq!(fill_rgb_node(&doc, second), None, "ambiguous alias must not match either target");
    assert_eq!(doc.node(current).unwrap().source, Some(named(&doc, "Current GUID")));
    assert_eq!(fill_rgb_node(&doc, current), Some([0.0, 0.0, 1.0]), "a current GUID outranks an alias with the same key");
}

fn message(swapped: bool) -> Kv<'static> {
    let shape = |id, parent, x| node(id, Some(parent), "ROUNDED_RECTANGLE", "Same layer", vec![("transform", matrix(x))]);
    let mut changes = vec![
        node(1, None, "DOCUMENT", "Identity", vec![]),
        node(2, Some(1), "CANVAS", "Page", vec![]),
        node(10, Some(2), "SYMBOL", "Renamed Left main", vec![]),
        node(11, Some(10), "FRAME", "Frame", vec![("transform", matrix(2.0))]),
        shape(12, 11, 2.0),
        shape(13, 11, 4.0),
        shape(14, 11, 6.0),
        node(15, Some(11), "FRAME", "Deep frame", vec![]),
        node(16, Some(15), "FRAME", "Deeper frame", vec![]),
        shape(17, 16, 8.0),
        node(
            20,
            Some(2),
            "SYMBOL",
            "State=Right, Renamed=Yes",
            vec![(
                "componentPropDefs",
                Kv::Array(vec![o(vec![
                    ("id", g(200)),
                    ("name", s("Enabled")),
                    ("type", Kv::Enum("BOOL")),
                    ("initialValue", o(vec![("boolValue", Kv::Bool(true))])),
                ])]),
            )],
        ),
        node(21, Some(20), "FRAME", "Frame", vec![("transform", matrix(if swapped { 23.0 } else { 2.0 }))]),
        shape(22, 21, 2.0),
        shape(23, 21, 4.0),
        shape(24, 21, 6.0),
        node(30, Some(2), "SYMBOL", "Outer", vec![]),
        node(40, Some(30), "INSTANCE", "Original wrapper", vec![("symbolData", symbol(50, vec![row(&[60, 21], vec![("transform", matrix(3.0))])]))]),
        node(50, Some(2), "SYMBOL", "Renamed wrapper main", vec![]),
        node(
            60,
            Some(50),
            "INSTANCE",
            "Same nested slot",
            vec![
                ("symbolData", symbol(20, vec![])),
                ("visible", Kv::Bool(false)),
                ("opacity", Kv::Float(0.3)),
                ("mask", Kv::Bool(true)),
                ("componentPropAssignments", Kv::Array(vec![o(vec![("defID", g(200)), ("value", o(vec![("boolValue", Kv::Bool(false))]))])])),
            ],
        ),
        node(61, Some(50), "INSTANCE", "Same nested slot", vec![("symbolData", symbol(20, vec![]))]),
    ];
    let mut nested = vec![
        ("name", s("Renamed nested slot")),
        ("visible", Kv::Bool(true)),
        ("opacity", Kv::Float(1.0)),
        ("mask", Kv::Bool(false)),
        ("componentPropAssignments", Kv::Array(vec![])),
    ];
    if swapped {
        nested.push(("overriddenSymbolID", g(10)));
    }
    changes.push(node(
        70,
        Some(2),
        "INSTANCE",
        "Copy",
        vec![(
            "symbolData",
            symbol(
                30,
                vec![
                    row(&[40], vec![("name", s("Renamed wrapper"))]),
                    row(&[40, 60], nested),
                    row(&[40, 60, if swapped { 11 } else { 21 }], vec![("transform", matrix(2.0))]),
                ],
            ),
        )],
    ));
    o(vec![("nodeChanges", Kv::Array(changes))])
}

fn import_message(swapped: bool) -> Document {
    let schema = Schema::parse(export::SCHEMA).unwrap();
    let bytes = super::tests::canvas(&schema, &message(swapped));
    let (doc, notes) = import(&bytes, "Identity.fig").unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    doc
}

fn named(doc: &Document, name: &str) -> NodeId {
    doc.nodes().find(|n| n.name == name).unwrap().id
}

fn child(doc: &Document, id: NodeId, index: usize) -> NodeId {
    doc.node(id).unwrap().children[index]
}

fn assert_contextual(doc: &Document) {
    let outer = named(doc, "Outer");
    let right = named(doc, "State=Right, Renamed=Yes");
    let wrapper_main = named(doc, "Renamed wrapper main");
    let copy = named(doc, "Copy");
    let main_wrapper = child(doc, outer, 0);
    let copy_wrapper = child(doc, copy, 0);
    assert_eq!(doc.node(copy).unwrap().source, Some(outer));
    assert_eq!(doc.node(copy_wrapper).unwrap().source, Some(main_wrapper));
    assert!(matches!(&doc.node(copy_wrapper).unwrap().kind, NodeKind::Instance {component, ..} if *component == wrapper_main));
    for index in 0..2 {
        let default_slot = child(doc, main_wrapper, index);
        let copy_slot = child(doc, copy_wrapper, index);
        assert_eq!(doc.node(copy_slot).unwrap().source, Some(default_slot), "slot {index}");
        assert!(matches!(&doc.node(default_slot).unwrap().kind, NodeKind::Instance {component, ..} if *component == right));
        assert!(matches!(&doc.node(copy_slot).unwrap().kind, NodeKind::Instance {component, ..} if *component == right));
    }
    let default_slot = child(doc, main_wrapper, 0);
    let copy_slot = child(doc, copy_wrapper, 0);
    let inherited = child(doc, default_slot, 0);
    let copy_frame = child(doc, copy_slot, 0);
    assert!(matches!(doc.node(copy_frame).unwrap().kind, NodeKind::Frame));
    assert_eq!(doc.node(inherited).unwrap().transform.translation().x, 3.0);
    assert_eq!(doc.node(copy_frame).unwrap().transform.translation().x, 2.0);
    assert_eq!(doc.node(copy_frame).unwrap().source, Some(inherited));
    for index in 0..3 {
        assert_eq!(doc.node(child(doc, copy_frame, index)).unwrap().source, Some(child(doc, inherited, index)));
        assert_eq!(doc.node(child(doc, copy_frame, index)).unwrap().transform.translation().x, 2.0 + 2.0 * index as f64);
    }
    let a = doc.node(default_slot).unwrap();
    let b = doc.node(copy_slot).unwrap();
    assert!(!a.visible && a.is_mask && a.opacity < 1.0);
    assert!(b.visible && !b.is_mask && b.opacity == 1.0);
    assert!(matches!(&a.kind, NodeKind::Instance {props, ..} if props.len() == 1 && props.values().all(|p| p == &json!(false))));
    assert!(matches!(&b.kind, NodeKind::Instance {props, ..} if props.is_empty()));
}

#[test]
fn contextual_sources_preserve_inherited_geometry_and_explicit_clears() {
    assert_contextual(&import_message(false));
}

fn assert_swapped(doc: &Document) {
    let outer = named(doc, "Outer");
    let right = named(doc, "State=Right, Renamed=Yes");
    let copy = named(doc, "Copy");
    let main_wrapper = child(doc, outer, 0);
    let copy_wrapper = child(doc, copy, 0);
    let copy_slot = child(doc, copy_wrapper, 0);
    let default_slot = child(doc, main_wrapper, 0);
    let left = named(doc, "Renamed Left main");
    assert_eq!(doc.node(copy).unwrap().source, Some(outer));
    assert_eq!(doc.node(copy_wrapper).unwrap().source, Some(main_wrapper));
    assert_eq!(doc.node(copy_slot).unwrap().source, Some(default_slot));
    assert!(matches!(&doc.node(copy_slot).unwrap().kind, NodeKind::Instance {component, props} if *component == left && props.is_empty()));
    assert!(matches!(&doc.node(default_slot).unwrap().kind, NodeKind::Instance {component, ..} if *component == right));
    let current_frame = child(doc, copy_slot, 0);
    let left_frame = child(doc, left, 0);
    assert!(matches!(doc.node(current_frame).unwrap().kind, NodeKind::Frame));
    assert_eq!(doc.node(current_frame).unwrap().source, Some(left_frame));
    assert_eq!(doc.node(current_frame).unwrap().transform.translation().x, 2.0);
    assert_eq!(doc.node(child(doc, right, 0)).unwrap().transform.translation().x, 23.0);
    for index in 0..3 {
        assert_eq!(doc.node(child(doc, current_frame, index)).unwrap().source, Some(child(doc, left_frame, index)));
        assert_eq!(doc.node(child(doc, current_frame, index)).unwrap().transform.translation().x, 2.0 + 2.0 * index as f64);
    }
    let deep_copy = child(doc, child(doc, child(doc, current_frame, 3), 0), 0);
    let deep_main = child(doc, child(doc, child(doc, left_frame, 3), 0), 0);
    assert_eq!(doc.node(deep_copy).unwrap().source, Some(deep_main));
    assert_eq!(doc.node(deep_copy).unwrap().transform.translation().x, 8.0);
    let other = child(doc, copy_wrapper, 1);
    assert_eq!(doc.node(other).unwrap().source, Some(child(doc, main_wrapper, 1)));
    assert!(matches!(&doc.node(other).unwrap().kind, NodeKind::Instance {component, ..} if *component == right));
    assert!(doc.node(copy_slot).unwrap().visible && !doc.node(copy_slot).unwrap().is_mask && doc.node(copy_slot).unwrap().opacity == 1.0);
}

#[test]
fn swapped_children_use_current_main_through_frames_and_multiple_siblings() {
    assert_swapped(&import_message(true));
}

fn edit_node(value: &mut Value, id: &str, edit: &mut dyn FnMut(&mut Value)) {
    if value.get("id").and_then(Value::as_str) == Some(id) {
        edit(value);
    }
    if let Some(kids) = value.get_mut("children").and_then(Value::as_array_mut) {
        for child in kids {
            edit_node(child, id, edit);
        }
    }
}

#[test]
fn missing_foreign_self_and_wrong_namespace_provenance_is_not_guessed() {
    for (origin, id) in [("1:99999", "I1:70;1:40"), ("1:12", "I1:70;1:40"), ("1:70", "1:70"), ("1:40", "I1:70;1:40")] {
        let (mut json, _, _) = super::map::to_rest(&message(false), "Identity", crate::rest::MAX_NODES);
        edit_node(&mut json["document"]["children"][0], id, &mut |node| {
            node[crate::rest::FIG_SOURCE_FIELD]["origin"] = json!(origin);
            if id == "1:70" {
                node["componentId"] = json!("1:70"); // a root cannot be its own component
            } else if origin == "1:40" {
                node["id"] = json!("I1:999;1:40");
            } else {
                node["id"] = json!(format!("I1:70;{origin}")); // valid namespace, but missing/foreign target
            }
        });
        let (doc, _) = crate::rest::import_fig(&json, &|_| None).unwrap();
        let target = if id == "1:70" { named(&doc, "Copy") } else { named(&doc, "Renamed wrapper") };
        assert!(doc.node(target).unwrap().source.is_none(), "{origin}/{id}");
    }
}

#[test]
fn forward_main_references_and_duplicate_ids_are_conservative() {
    let (mut json, _, _) = super::map::to_rest(&message(false), "Identity", crate::rest::MAX_NODES);
    json["document"]["children"][0]["children"].as_array_mut().unwrap().rotate_right(1);
    let (doc, _) = crate::rest::import_fig(&json, &|_| None).unwrap();
    assert!(doc.node(named(&doc, "Copy")).unwrap().source.is_some());
    // 1:40 is nested inside Outer, so duplicate that actual template at page level.
    let template = json["document"]["children"][0]["children"].as_array().unwrap().iter().find(|n| n["id"] == "1:30").unwrap()["children"][0].clone();
    json["document"]["children"][0]["children"].as_array_mut().unwrap().push(template);
    let (doc, _) = crate::rest::import_fig(&json, &|_| None).unwrap();
    assert!(doc.node(named(&doc, "Renamed wrapper")).unwrap().source.is_none());
}

#[test]
fn missing_contextual_targets_do_not_fall_back_to_a_global_slot() {
    let (mut json, _, _) = super::map::to_rest(&message(false), "Identity", crate::rest::MAX_NODES);
    edit_node(&mut json["document"]["children"][0], "1:40", &mut |node| {
        node["children"].as_array_mut().unwrap().retain(|n| n["id"] != "I1:40;1:60");
    });
    let (doc, _) = crate::rest::import_fig(&json, &|_| None).unwrap();
    let copy_slot = child(&doc, child(&doc, named(&doc, "Copy"), 0), 0);
    assert!(doc.node(copy_slot).unwrap().source.is_none());
    assert!(doc.nodes().any(|n| n.name == "Same nested slot"), "the raw slot still exists");
}

#[test]
fn instance_to_empty_frame_keeps_its_source_identity() {
    let mut msg = message(false);
    let Kv::Object(top) = &mut msg else { panic!() };
    let Kv::Array(nodes) = &mut top.iter_mut().find(|(key, _)| *key == "nodeChanges").unwrap().1 else { panic!() };
    let copy = nodes.iter_mut().find(|n| n.get("name").and_then(Kv::str) == Some("Copy")).unwrap();
    let Kv::Object(fields) = copy else { panic!() };
    let Kv::Object(symbol) = &mut fields.iter_mut().find(|(key, _)| *key == "symbolData").unwrap().1 else { panic!() };
    let Kv::Array(overrides) = &mut symbol.iter_mut().find(|(key, _)| *key == "symbolOverrides").unwrap().1 else { panic!() };
    overrides.push(row(&[40, 61], vec![("type", Kv::Enum("FRAME"))]));
    let (doc, _) = import(&super::tests::canvas(&schema(), &msg), "Identity.fig").unwrap();
    let main = child(&doc, named(&doc, "Outer"), 0);
    let copy = child(&doc, named(&doc, "Copy"), 0);
    let frame = child(&doc, copy, 1);
    assert!(matches!(doc.node(frame).unwrap().kind, NodeKind::Frame));
    assert!(doc.node(frame).unwrap().children.is_empty());
    assert_eq!(doc.node(frame).unwrap().source, Some(child(&doc, main, 1)));
}

#[test]
fn explicit_frame_to_instance_resets_children_to_the_resolved_main() {
    let mut msg = message(false);
    let Kv::Object(top) = &mut msg else { panic!() };
    let Kv::Array(nodes) = &mut top.iter_mut().find(|(key, _)| *key == "nodeChanges").unwrap().1 else { panic!() };
    nodes.push(node(80, Some(30), "FRAME", "Original frame", vec![]));
    nodes.push(node(81, Some(80), "ROUNDED_RECTANGLE", "Old frame child", vec![]));
    let copy = nodes.iter_mut().find(|n| n.get("name").and_then(Kv::str) == Some("Copy")).unwrap();
    let Kv::Object(fields) = copy else { panic!() };
    let Kv::Object(symbol) = &mut fields.iter_mut().find(|(key, _)| *key == "symbolData").unwrap().1 else { panic!() };
    let Kv::Array(overrides) = &mut symbol.iter_mut().find(|(key, _)| *key == "symbolOverrides").unwrap().1 else { panic!() };
    overrides.push(row(&[80], vec![("type", Kv::Enum("INSTANCE")), ("overriddenSymbolID", g(10))]));
    let (doc, notes) = import(&super::tests::canvas(&schema(), &msg), "Identity.fig").unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    let original = child(&doc, named(&doc, "Outer"), 1);
    let current = child(&doc, named(&doc, "Copy"), 1);
    let left = named(&doc, "Renamed Left main");
    assert!(matches!(doc.node(original).unwrap().kind, NodeKind::Frame));
    assert_eq!(doc.node(current).unwrap().source, Some(original));
    assert!(matches!(&doc.node(current).unwrap().kind, NodeKind::Instance {component, ..} if *component == left));
    assert_eq!(doc.node(child(&doc, current, 0)).unwrap().source, Some(child(&doc, left, 0)));
}

// The orphan symbol is decoded and expanded but is not a layer on any imported page. This
// matches an external main: its instance becomes a frame while the rendered body survives.
fn external_namespace_message() -> Kv<'static> {
    o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "DOCUMENT", "External namespace", vec![]),
            node(2, Some(1), "CANVAS", "Page", vec![]),
            node(
                10,
                Some(2),
                "SYMBOL",
                "Known main",
                vec![(
                    "componentPropDefs",
                    Kv::Array(vec![o(vec![
                        ("id", g(200)),
                        ("name", s("Enabled")),
                        ("type", Kv::Enum("BOOL")),
                        ("initialValue", o(vec![("boolValue", Kv::Bool(true))])),
                    ])]),
                )],
            ),
            node(11, Some(10), "FRAME", "Known frame", vec![("transform", matrix(2.0))]),
            node(12, Some(11), "ROUNDED_RECTANGLE", "Known leaf", vec![("transform", matrix(4.0))]),
            node(30, Some(2), "SYMBOL", "Outer external", vec![]),
            node(
                90,
                Some(30),
                "INSTANCE",
                "External body",
                vec![("symbolData", symbol(99, vec![row(&[110, 11], vec![("transform", matrix(3.0))])]))],
            ),
            node(99, None, "SYMBOL", "External orphan", vec![]),
            node(100, Some(99), "FRAME", "Body frame", vec![]),
            node(101, Some(99), "FRAME", "Other body frame", vec![]),
            node(
                110,
                Some(100),
                "INSTANCE",
                "Same slot",
                vec![
                    ("symbolData", symbol(10, vec![])),
                    ("visible", Kv::Bool(false)),
                    ("opacity", Kv::Float(0.3)),
                    ("mask", Kv::Bool(true)),
                    ("componentPropAssignments", Kv::Array(vec![o(vec![("defID", g(200)), ("value", o(vec![("boolValue", Kv::Bool(false))]))])])),
                ],
            ),
            node(111, Some(100), "INSTANCE", "Same slot", vec![("symbolData", symbol(10, vec![]))]),
            node(
                70,
                Some(2),
                "INSTANCE",
                "External copy",
                vec![(
                    "symbolData",
                    symbol(
                        30,
                        vec![
                            row(
                                &[90, 110],
                                vec![
                                    ("name", s("Renamed owned slot")),
                                    ("visible", Kv::Bool(true)),
                                    ("opacity", Kv::Float(1.0)),
                                    ("mask", Kv::Bool(false)),
                                    ("componentPropAssignments", Kv::Array(vec![])),
                                ],
                            ),
                            row(&[90, 110, 11], vec![("transform", matrix(2.0))]),
                        ],
                    ),
                )],
            ),
        ]),
    )])
}

fn assert_external_namespace(doc: &Document) {
    let outer = named(doc, "Outer external");
    let copy = named(doc, "External copy");
    let main = named(doc, "Known main");
    let body = child(doc, outer, 0);
    let copied_body = child(doc, copy, 0);
    assert!(matches!(doc.node(body).unwrap().kind, NodeKind::Frame));
    assert!(matches!(doc.node(copied_body).unwrap().kind, NodeKind::Frame));
    assert_eq!(doc.node(body).unwrap().source, None, "the absent external main must not acquire a guessed native node");
    assert_eq!(doc.node(copied_body).unwrap().source, Some(body));
    let frame = child(doc, body, 0);
    let copied_frame = child(doc, copied_body, 0);
    assert_eq!(doc.node(frame).unwrap().source, None, "materialized ordinary layers are owned by the frame");
    assert_eq!(doc.node(copied_frame).unwrap().source, Some(frame));
    for index in 0..2 {
        let slot = child(doc, frame, index);
        let copied_slot = child(doc, copied_frame, index);
        assert_eq!(doc.node(slot).unwrap().source, Some(main), "known owned instance has its real root main");
        assert_eq!(doc.node(copied_slot).unwrap().source, Some(slot), "copied slots preserve the contextual owned layer identity");
        assert!(matches!(&doc.node(slot).unwrap().kind, NodeKind::Instance {component, ..} if *component == main));
        assert!(matches!(&doc.node(copied_slot).unwrap().kind, NodeKind::Instance {component, ..} if *component == main));
        let inherited = child(doc, slot, 0);
        let current = child(doc, copied_slot, 0);
        assert_eq!(doc.node(inherited).unwrap().source, Some(child(doc, main, 0)));
        assert_eq!(doc.node(current).unwrap().source, Some(inherited));
        assert_eq!(doc.node(inherited).unwrap().transform.translation().x, if index == 0 { 3.0 } else { 2.0 });
        assert_eq!(doc.node(current).unwrap().transform.translation().x, 2.0);
        assert_eq!(doc.node(child(doc, current, 0)).unwrap().source, Some(child(doc, inherited, 0)));
        assert_eq!(doc.node(child(doc, current, 0)).unwrap().transform.translation().x, 4.0);
    }
    let old = doc.node(child(doc, frame, 0)).unwrap();
    let current = doc.node(child(doc, copied_frame, 0)).unwrap();
    assert!(!old.visible && old.is_mask && old.opacity < 1.0);
    assert!(current.visible && !current.is_mask && current.opacity == 1.0);
    assert_eq!(current.name, "Renamed owned slot");
    assert!(matches!(&old.kind, NodeKind::Instance {props, ..} if props.len() == 1));
    assert!(matches!(&current.kind, NodeKind::Instance {props, ..} if props.is_empty()));
}

#[test]
fn absent_external_mains_keep_known_owned_instances_and_contextual_copies() {
    let (doc, notes) = import(&super::tests::canvas(&schema(), &external_namespace_message()), "External.fig").unwrap();
    assert!(notes.iter().any(|w| w.contains("components that aren't in the file")), "{notes:?}");
    assert!(!notes.iter().any(|w| w.contains("source link")), "materialized own layers are not unresolved copies: {notes:?}");
    assert_external_namespace(&doc);
}

#[test]
fn external_namespace_is_private_and_requires_actual_wire_instances() {
    for mode in 0..3 {
        let (mut json, _, _) = super::map::to_rest(&external_namespace_message(), "External", crate::rest::MAX_NODES);
        if mode == 0 {
            let (doc, _) = crate::rest::import(&json, &|_| None).unwrap();
            assert!(doc.nodes().all(|n| n.source.is_none()));
            continue;
        }
        edit_node(&mut json["document"]["children"][0], "1:90", &mut |node| {
            if mode == 1 {
                node["type"] = json!("FRAME");
            } else {
                node[crate::rest::FIG_SOURCE_FIELD]["wireInstance"] = json!(false);
                node[crate::rest::FIG_SOURCE_FIELD]["wireFrame"] = json!(true);
            }
        });
        let (doc, _) = crate::rest::import_fig(&json, &|_| None).unwrap();
        let raw_slot = child(&doc, child(&doc, child(&doc, named(&doc, "Outer external"), 0), 0), 0);
        assert!(doc.node(raw_slot).unwrap().source.is_none(), "ordinary/fake frame mode {mode} must not establish a namespace");
    }
}

#[test]
fn external_namespace_does_not_cover_present_wrong_kind_or_blocked_main() {
    for mode in 0..3 {
        let (mut json, _, _) = super::map::to_rest(&external_namespace_message(), "External", crate::rest::MAX_NODES);
        let page = &mut json["document"]["children"][0];
        match mode {
            0 => page["children"].as_array_mut().unwrap().push(json!({"id":"1:99", "type":"FRAME", "name":"Wrong kind",
                "_figSource":{"origin":"1:99","expanded":false,"wireFrame":true,"wireInstance":false}})),
            1 => edit_node(page, "1:90", &mut |node| {
                node["componentId"] = json!("1:30");
            }),
            _ => page["children"].as_array_mut().unwrap().push(json!({"id":"1:99", "type":"COMPONENT", "name":"Cycle main",
                "_figSource":{"origin":"1:99","expanded":false,"wireFrame":false,"wireInstance":false},
                "children":[{"id":"1:199", "type":"INSTANCE", "name":"Cycle edge", "componentId":"1:30",
                    "_figSource":{"origin":"1:199","expanded":false,"wireFrame":false,"wireInstance":true}}]})),
        }
        let (doc, _) = crate::rest::import_fig(&json, &|_| None).unwrap();
        let body = child(&doc, named(&doc, "Outer external"), 0);
        assert!(matches!(doc.node(body).unwrap().kind, NodeKind::Frame));
        let slot = child(&doc, child(&doc, body, 0), 0);
        assert!(doc.node(slot).unwrap().source.is_none(), "existing invalid/cycle/self main mode {mode} must not become a materialized namespace");
    }
}

#[test]
fn external_namespace_rejects_foreign_siblings_duplicates_and_bad_paths() {
    for mode in 0..4 {
        let (mut json, _, _) = super::map::to_rest(&external_namespace_message(), "External", crate::rest::MAX_NODES);
        let page = &mut json["document"]["children"][0];
        match mode {
            0 => {
                // Same root namespace and GUID, but physically under a different frame slot.
                let mut moved = None;
                edit_node(page, "I1:70;1:90;1:100", &mut |node| {
                    moved = Some(node["children"].as_array_mut().unwrap().remove(0));
                });
                edit_node(page, "I1:70;1:90;1:101", &mut |node| {
                    node["children"] = json!([moved.take().unwrap()]);
                });
            }
            1 => edit_node(page, "I1:70;1:90;1:110", &mut |node| {
                node["id"] = json!("I1:70;1:999;1:110");
            }),
            2 => {
                let duplicate = page["children"].as_array().unwrap().iter().find(|n| n["id"] == "1:30").unwrap()["children"][0].clone();
                page["children"].as_array_mut().unwrap().push(duplicate);
            }
            _ => edit_node(page, "I1:90;1:100", &mut |node| {
                node["id"] = json!("I1:999;1:100");
            }),
        }
        let (doc, warnings) = crate::rest::import_fig(&json, &|_| None).unwrap();
        let slot = named(&doc, "Renamed owned slot");
        assert!(doc.node(slot).unwrap().source.is_none(), "invalid frame namespace mode {mode}");
        assert!(warnings.iter().any(|w| w.contains("source link")), "invalid expanded paths remain reported: {warnings:?}");
    }
}

fn no_style() -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(4_294_967_295)), ("localID", Kv::Uint(4_294_967_295))])
}

#[test]
fn an_explicit_no_style_detaches_and_beats_the_older_style_field() {
    let message = o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, None, "DOCUMENT", "Document", vec![]),
            node(2, Some(1), "CANVAS", "Page", vec![]),
            node(5, Some(2), "RECTANGLE", "Red", vec![("styleType", Kv::Enum("FILL"))]),
            node(6, Some(2), "RECTANGLE", "Box", vec![("styleIdForFill", o(vec![("guid", no_style())])), ("inheritFillStyleID", g(5))]),
            node(7, Some(2), "RECTANGLE", "Linked", vec![("inheritFillStyleID", g(5))]),
        ]),
    )]);
    let (json, _, _) = super::map::to_rest(&message, "Detach", crate::rest::MAX_NODES);
    let find = |name: &str| json["document"]["children"][0]["children"].as_array().unwrap().iter().find(|n| n["name"] == name).unwrap().clone();
    assert!(find("Box")["styles"].get("fill").is_none(), "{}", find("Box")["styles"]);
    assert!(find("Linked")["styles"].get("fill").is_some(), "the older field still links when nothing detaches");
}
