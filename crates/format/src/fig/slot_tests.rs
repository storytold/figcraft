//! Component slot content from `.fig` files: SLOT definitions, the frames that reference them and
//! the content subtrees that instances assign, on synthetic messages.

use serde_json::Value;

use super::kiwi::Kv;
use super::map::to_rest;

fn g(local: u32) -> Kv<'static> {
    Kv::Object(vec![("sessionID", Kv::Uint(9)), ("localID", Kv::Uint(u64::from(local)))])
}

fn o(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}

fn node(local: u32, kind: &'static str, parent: Option<u32>, mut fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    let mut all = vec![("guid", g(local)), ("type", Kv::Enum(kind)), ("name", Kv::Str("Synthetic".into()))];
    if let Some(parent) = parent {
        all.push(("parentIndex", o(vec![("guid", g(parent)), ("position", Kv::Str("!".into()))])));
    }
    all.append(&mut fields);
    o(all)
}

fn mapped(msg: &Kv<'_>) -> Value {
    to_rest(msg, "Synthetic resources", crate::rest::MAX_NODES).0
}

fn named_id<'b>(v: &'b Value, id: &str) -> Option<&'b Value> {
    if v.get("id").and_then(Value::as_str) == Some(id) {
        return Some(v);
    }
    v.get("children").and_then(Value::as_array).into_iter().flatten().find_map(|v| named_id(v, id))
}

fn changes<'a>(msg: &'a mut Kv<'static>) -> &'a mut Vec<Kv<'static>> {
    let Kv::Object(fields) = msg else { panic!() };
    let (_, Kv::Array(rows)) = fields.iter_mut().find(|(key, _)| *key == "nodeChanges").unwrap() else { panic!() };
    rows
}

fn slot_message(content: Option<u32>, preferred: bool) -> Kv<'static> {
    let slot_ref = o(vec![
        ("variableField", Kv::Enum("SLOT_CONTENT_ID")),
        ("variableData", o(vec![("value", o(vec![("propRefValue", o(vec![("defId", g(70))]))]))])),
    ]);
    let mut instance_fields = vec![("symbolData", o(vec![("symbolID", g(10))]))];
    if let Some(content) = content {
        instance_fields.push((
            "componentPropAssignments",
            Kv::Array(vec![o(vec![
                ("defID", g(70)),
                ("value", o(vec![])),
                (
                    "varValue",
                    o(vec![
                        ("value", o(vec![("slotContentIdValue", o(vec![("guid", g(content))]))])),
                        ("dataType", Kv::Enum("SLOT_CONTENT_ID")),
                        ("resolvedDataType", Kv::Enum("SLOT_CONTENT_ID")),
                    ]),
                ),
            ])]),
        ));
    }
    let mut definition = vec![("id", g(70)), ("name", Kv::Str("Content".into())), ("type", Kv::Enum("SLOT"))];
    if preferred {
        definition.push((
            "preferredValues",
            o(vec![("instanceSwapValues", Kv::Array(vec![o(vec![("type", Kv::Enum("COMPONENT")), ("key", Kv::Str("unresolved-key".into()))])]))]),
        ));
    }
    o(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, "DOCUMENT", None, vec![]),
            node(2, "CANVAS", Some(1), vec![]),
            node(3, "CANVAS", Some(1), vec![("internalOnly", Kv::Bool(true))]),
            node(10, "SYMBOL", Some(2), vec![("componentPropDefs", Kv::Array(vec![o(definition)]))]),
            node(12, "FRAME", Some(10), vec![("parameterConsumptionMap", o(vec![("entries", Kv::Array(vec![slot_ref]))]))]),
            node(13, "RECTANGLE", Some(12), vec![]),
            node(20, "INSTANCE", Some(2), instance_fields),
            node(30, "FRAME", Some(3), vec![("isSlotContent", Kv::Bool(true))]),
            node(31, "RECTANGLE", Some(30), vec![]),
            node(32, "ELLIPSE", Some(30), vec![]),
            node(33, "FRAME", Some(3), vec![("isSlotContent", Kv::Bool(true))]),
        ]),
    )])
}
#[test]
fn slot_definition_reference_and_arbitrary_override_children_are_imported() {
    let rest = mapped(&slot_message(Some(30), false));
    let main = named_id(&rest["document"], "9:10").unwrap();
    assert_eq!(main["componentPropertyDefinitions"]["Content#9:70"]["type"], "SLOT");
    let main_slot = named_id(&rest["document"], "9:12").unwrap();
    assert_eq!(main_slot["componentPropertyReferences"]["slot"], "Content#9:70");
    let slot = named_id(&rest["document"], "I9:20;9:12").unwrap();
    assert!(slot.get("componentPropertyReferences").is_none());
    assert_eq!(slot[crate::rest::FIG_SOURCE_FIELD]["slotOverride"], true);
    let kids = slot["children"].as_array().unwrap();
    assert_eq!(kids.len(), 2);
    assert_eq!(kids[0]["id"], "I9:20;9:31");
    assert_eq!(kids[1]["id"], "I9:20;9:32");

    let (doc, warnings) = crate::rest::import_fig(&rest, &crate::rest::no_images).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let main = doc
        .node(*doc.pages.first().unwrap())
        .unwrap()
        .children
        .iter()
        .find_map(|id| doc.node(*id).filter(|n| n.name == "Synthetic" && matches!(n.kind, figcraft_doc::NodeKind::Component { .. })))
        .unwrap();
    assert!(
        matches!(&main.kind, figcraft_doc::NodeKind::Component { props, .. } if matches!(props.get("Content"), Some(figcraft_doc::PropDef::Slot { preferred }) if preferred.is_empty()))
    );
    let instance = doc.ids().filter_map(|id| doc.node(id)).find(|n| matches!(n.kind, figcraft_doc::NodeKind::Instance { .. })).unwrap();
    let slot = doc.node(instance.children[0]).unwrap();
    assert!(!slot.prop_refs.contains_key("slot"));
    assert_eq!(slot.source, Some(main.children[0]));
    assert!(slot.overrides.contains("slot"));
    assert_eq!(slot.children.len(), 2);
    assert!(slot.children.iter().all(|id| doc.node(*id).is_some_and(|child| child.source.is_none())));

    let (public, _) = crate::rest::import(&rest, &crate::rest::no_images).unwrap();
    let public_instance = public.ids().filter_map(|id| public.node(id)).find(|n| matches!(n.kind, figcraft_doc::NodeKind::Instance { .. })).unwrap();
    let public_slot = public.node(public_instance.children[0]).unwrap();
    assert!(!public_slot.overrides.contains("slot"));
}
#[test]
fn nested_instance_in_own_slot_content_keeps_its_own_source_tree() {
    let mut msg = slot_message(Some(30), false);
    let rows = changes(&mut msg);
    let slot_frame = rows.iter_mut().find(|row| row.get("guid").and_then(|guid| guid.get("localID")).and_then(Kv::f64) == Some(31.0)).unwrap();
    let Kv::Object(fields) = slot_frame else { panic!() };
    fields.iter_mut().find(|(key, _)| *key == "type").unwrap().1 = Kv::Enum("FRAME");
    rows.extend([
        node(34, "INSTANCE", Some(31), vec![("symbolData", o(vec![("symbolID", g(40))]))]),
        node(40, "SYMBOL", Some(2), vec![]),
        node(41, "FRAME", Some(40), vec![]),
        node(42, "RECTANGLE", Some(41), vec![]),
    ]);

    let rest = mapped(&msg);
    let (doc, warnings) = crate::rest::import_fig(&rest, &crate::rest::no_images).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let outer = doc
        .ids()
        .filter_map(|id| doc.node(id))
        .find(|n| {
            matches!(n.kind, figcraft_doc::NodeKind::Instance { .. })
                && !doc.ancestors(n.id).iter().any(|a| doc.node(*a).is_some_and(|p| matches!(p.kind, figcraft_doc::NodeKind::Instance { .. })))
        })
        .unwrap();
    let slot = doc.node(outer.children[0]).unwrap();
    let own_frame = slot.children.iter().filter_map(|id| doc.node(*id)).find(|n| matches!(n.kind, figcraft_doc::NodeKind::Frame)).unwrap();
    assert!(own_frame.source.is_none());

    let nested = slot
        .children
        .iter()
        .filter_map(|id| doc.node(*id))
        .flat_map(|n| n.children.iter())
        .filter_map(|id| doc.node(*id))
        .find(|n| matches!(n.kind, figcraft_doc::NodeKind::Instance { .. }))
        .unwrap();
    let figcraft_doc::NodeKind::Instance { component, .. } = nested.kind else { panic!("not an instance") };
    assert_eq!(nested.source, Some(component));
    let main_frame = doc.node(doc.node(component).unwrap().children[0]).unwrap();
    let expanded_frame = doc.node(nested.children[0]).unwrap();
    assert_eq!(expanded_frame.source, Some(main_frame.id));
    assert_eq!(expanded_frame.children.len(), 1);
    assert_eq!(doc.node(expanded_frame.children[0]).unwrap().source, Some(main_frame.children[0]));
}
#[test]
fn outer_slot_assignment_wins_over_a_nested_instances_own() {
    let mut msg = slot_message(Some(30), false);
    let assignment = |content: u32| {
        Kv::Array(vec![o(vec![
            ("defID", g(70)),
            ("value", o(vec![])),
            (
                "varValue",
                o(vec![
                    ("value", o(vec![("slotContentIdValue", o(vec![("guid", g(content))]))])),
                    ("dataType", Kv::Enum("SLOT_CONTENT_ID")),
                    ("resolvedDataType", Kv::Enum("SLOT_CONTENT_ID")),
                ]),
            ),
        ])])
    };
    // The nested instance inside the main assigns the empty slot content; the outer instance's content wins.
    let slot_ref = o(vec![
        ("variableField", Kv::Enum("SLOT_CONTENT_ID")),
        ("variableData", o(vec![("value", o(vec![("propRefValue", o(vec![("defId", g(70))]))]))])),
    ]);
    changes(&mut msg).extend([
        node(14, "INSTANCE", Some(10), vec![("symbolData", o(vec![("symbolID", g(40))])), ("componentPropAssignments", assignment(33))]),
        node(40, "SYMBOL", Some(2), vec![]),
        node(41, "FRAME", Some(40), vec![("parameterConsumptionMap", o(vec![("entries", Kv::Array(vec![slot_ref]))]))]),
    ]);
    let rest = mapped(&msg);
    let slot = named_id(&rest["document"], "I9:20;9:14;9:41").unwrap();
    assert_eq!(slot["children"].as_array().map_or(0, Vec::len), 2, "{slot}");
}
#[test]
fn unassigned_slot_keeps_default_and_bad_content_target_warns() {
    let rest = mapped(&slot_message(None, false));
    let slot = named_id(&rest["document"], "I9:20;9:12").unwrap();
    assert_eq!(slot["children"].as_array().unwrap().len(), 1);
    assert!(slot.get("overrides").is_none());

    let (rest, warnings, _) = to_rest(&slot_message(Some(999), false), "Synthetic", crate::rest::MAX_NODES);
    assert!(warnings.iter().any(|w| w.contains("slot content assignment target is missing or is not slot content")), "{warnings:?}");
    let slot = named_id(&rest["document"], "I9:20;9:12").unwrap();
    assert_eq!(slot["children"].as_array().unwrap().len(), 1);

    let (rest, warnings, _) = to_rest(&slot_message(Some(33), true), "Synthetic", crate::rest::MAX_NODES);
    let main = named_id(&rest["document"], "9:10").unwrap();
    assert_eq!(main["componentPropertyDefinitions"]["Content#9:70"]["type"], "SLOT");
    assert!(warnings.iter().any(|w| w.contains("preferred slot component keys cannot be resolved")), "{warnings:?}");
    let empty = named_id(&rest["document"], "I9:20;9:12").unwrap();
    assert_eq!(empty["children"].as_array().map_or(0, Vec::len), 0);
    assert_eq!(empty[crate::rest::FIG_SOURCE_FIELD]["slotOverride"], true);
}
