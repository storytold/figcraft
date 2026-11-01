//! Component properties as current `.fig` files write them, on synthetic messages: the copy of a
//! set's definitions each variant keeps (`parentPropDefId`, no name or type of its own), and
//! assignments that carry their value only in the variable form (`varValue`).

use figcraft_doc::NodeKind;
use serde_json::Value;

use super::kiwi::Kv;
use super::map::to_rest;

fn g(local: u32) -> Kv<'static> {
    Kv::Object(vec![("sessionID", Kv::Uint(9)), ("localID", Kv::Uint(u64::from(local)))])
}

fn o(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}

fn s(text: &str) -> Kv<'static> {
    Kv::Str(text.into())
}

fn node(local: u32, kind: &'static str, name: &str, parent: Option<u32>, mut fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    let mut all = vec![("guid", g(local)), ("type", Kv::Enum(kind)), ("name", s(name))];
    if let Some(parent) = parent {
        all.push(("parentIndex", o(vec![("guid", g(parent)), ("position", s("!"))])));
    }
    all.append(&mut fields);
    o(all)
}

/// A layer's `field` bound to property definition `def`.
fn bound(field: &'static str, def: u32) -> (&'static str, Kv<'static>) {
    let entry = o(vec![
        ("variableField", Kv::Enum(field)),
        ("variableData", o(vec![("value", o(vec![("propRefValue", o(vec![("defId", g(def))]))])), ("dataType", Kv::Enum("PROP_REF"))])),
    ]);
    ("parameterConsumptionMap", o(vec![("entries", Kv::Array(vec![entry]))]))
}

fn definition(id: u32, name: &str, kind: &'static str, initial: Kv<'static>) -> Kv<'static> {
    o(vec![("id", g(id)), ("name", s(name)), ("type", Kv::Enum(kind)), ("initialValue", initial)])
}

/// An assignment of `def` that leaves `value` empty and says it in `varValue` only.
fn variable_assignment(def: u32, data_type: &'static str, value: Kv<'static>) -> Kv<'static> {
    o(vec![
        ("defID", g(def)),
        ("value", o(vec![])),
        ("varValue", o(vec![("value", value), ("dataType", Kv::Enum(data_type)), ("resolvedDataType", Kv::Enum(data_type))])),
    ])
}

fn message(nodes: Vec<Kv<'static>>) -> Kv<'static> {
    let mut all = vec![node(1, "DOCUMENT", "Document", None, vec![]), node(2, "CANVAS", "Page", Some(1), vec![])];
    all.extend(nodes);
    o(vec![("nodeChanges", Kv::Array(all))])
}

fn by_id<'v>(v: &'v Value, id: &str) -> &'v Value {
    fn find<'v>(v: &'v Value, id: &str) -> Option<&'v Value> {
        if v.get("id").and_then(Value::as_str) == Some(id) {
            return Some(v);
        }
        v.get("children").and_then(Value::as_array).into_iter().flatten().find_map(|v| find(v, id))
    }
    find(v, id).unwrap_or_else(|| panic!("no layer {id}"))
}

fn instance_props(doc: &figcraft_doc::Document) -> &std::collections::BTreeMap<String, Value> {
    let instance = doc.ids().filter_map(|id| doc.node(id)).find(|n| n.name == "Instance").unwrap();
    let NodeKind::Instance { props, .. } = &instance.kind else { panic!("not an instance") };
    props
}

/// A "Field" set with a BOOL definition (9:70), a variant whose "Hint text" reads its copy (9:71),
/// and an instance of the variant that sets the property false through definition `assigned`.
fn field_set(assigned: u32) -> Kv<'static> {
    message(vec![
        node(
            9,
            "FRAME",
            "Field",
            Some(2),
            vec![
                ("isStateGroup", Kv::Bool(true)),
                ("componentPropDefs", Kv::Array(vec![definition(70, "Hint", "BOOL", o(vec![("boolValue", Kv::Bool(true))]))])),
            ],
        ),
        node(
            10,
            "SYMBOL",
            "State=Default",
            Some(9),
            vec![("componentPropDefs", Kv::Array(vec![o(vec![("id", g(71)), ("parentPropDefId", g(70))])]))],
        ),
        node(11, "TEXT", "Hint text", Some(10), vec![bound("VISIBLE", 71)]),
        node(
            20,
            "INSTANCE",
            "Instance",
            Some(2),
            vec![
                ("symbolData", o(vec![("symbolID", g(10))])),
                ("componentPropAssignments", Kv::Array(vec![o(vec![("defID", g(assigned)), ("value", o(vec![("boolValue", Kv::Bool(false))]))])])),
            ],
        ),
    ])
}

#[test]
fn variant_layers_bind_to_the_set_properties_their_copies_name() {
    let msg = field_set(71);
    let (rest, _, _) = to_rest(&msg, "Synthetic", crate::rest::MAX_NODES);
    assert_eq!(by_id(&rest["document"], "9:9")["componentPropertyDefinitions"]["Hint#9:70"]["type"], "BOOLEAN");
    assert_eq!(by_id(&rest["document"], "9:11")["componentPropertyReferences"]["visible"], "Hint#9:70");
    assert_eq!(by_id(&rest["document"], "9:20")["componentProperties"]["Hint#9:70"]["value"], false);

    let (doc, _) = crate::rest::import_fig(&rest, &crate::rest::no_images).unwrap();
    let hint = doc.ids().filter_map(|id| doc.node(id)).find(|n| n.name == "Hint text" && n.source.is_none()).unwrap();
    assert_eq!(hint.prop_refs.get("visible").map(String::as_str), Some("Hint"));
    assert_eq!(instance_props(&doc).get("Hint"), Some(&Value::Bool(false)));
}

/// An assignment reaches layers that read another copy of its definition: here the instance sets
/// the set's own definition while the variant's layer reads the variant's copy.
#[test]
fn assignments_reach_layers_that_read_another_copy_of_their_definition() {
    let (rest, _, _) = to_rest(&field_set(70), "Synthetic", crate::rest::MAX_NODES);
    let instance = by_id(&rest["document"], "9:20");
    assert_eq!(instance["componentProperties"]["Hint#9:70"]["value"], false);
    assert_eq!(by_id(instance, "I9:20;9:11")["visible"], false);
}

#[test]
fn property_values_written_only_as_variables_are_applied() {
    let msg = message(vec![
        node(
            10,
            "SYMBOL",
            "Field",
            Some(2),
            vec![(
                "componentPropDefs",
                Kv::Array(vec![
                    definition(70, "Hint", "BOOL", o(vec![("boolValue", Kv::Bool(true))])),
                    definition(71, "Label", "TEXT", o(vec![("textValue", o(vec![("characters", s("Label"))]))])),
                    definition(72, "Icon", "INSTANCE_SWAP", o(vec![("guidValue", g(40))])),
                ]),
            )],
        ),
        node(11, "TEXT", "Hint text", Some(10), vec![bound("VISIBLE", 70)]),
        node(12, "TEXT", "Label", Some(10), vec![("textData", o(vec![("characters", s("Label"))])), bound("TEXT_DATA", 71)]),
        node(13, "INSTANCE", "Icon", Some(10), vec![("symbolData", o(vec![("symbolID", g(40))])), bound("OVERRIDDEN_SYMBOL_ID", 72)]),
        node(40, "SYMBOL", "Mail", Some(2), vec![]),
        node(41, "SYMBOL", "Card", Some(2), vec![]),
        node(
            20,
            "INSTANCE",
            "Instance",
            Some(2),
            vec![
                ("symbolData", o(vec![("symbolID", g(10))])),
                (
                    "componentPropAssignments",
                    Kv::Array(vec![
                        variable_assignment(70, "BOOLEAN", o(vec![("boolValue", Kv::Bool(false))])),
                        variable_assignment(71, "TEXT_DATA", o(vec![("textDataValue", o(vec![("characters", s("Name on card"))]))])),
                        variable_assignment(72, "SYMBOL_ID", o(vec![("symbolIdValue", o(vec![("guid", g(41))]))])),
                    ]),
                ),
            ],
        ),
    ]);
    let (rest, _, _) = to_rest(&msg, "Synthetic", crate::rest::MAX_NODES);
    let instance = by_id(&rest["document"], "9:20");
    assert_eq!(instance["componentProperties"]["Hint#9:70"]["value"], false);
    assert_eq!(instance["componentProperties"]["Label#9:71"]["value"], "Name on card");
    assert_eq!(instance["componentProperties"]["Icon#9:72"]["value"], "9:41");
    assert_eq!(by_id(instance, "I9:20;9:11")["visible"], false);
    assert_eq!(by_id(instance, "I9:20;9:12")["characters"], "Name on card");
    assert_eq!(by_id(instance, "I9:20;9:13")["componentId"], "9:41");

    let (doc, _) = crate::rest::import_fig(&rest, &crate::rest::no_images).unwrap();
    let props = instance_props(&doc);
    assert_eq!(props.get("Hint"), Some(&Value::Bool(false)));
    assert_eq!(props.get("Label"), Some(&Value::String("Name on card".into())));
}
