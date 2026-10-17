//! `.fig` text skip-ink values map, inherit and round-trip as native decoration look state. Skip
//! ink only shows on an underline, so the fixtures are underlined.

use super::kiwi::Kv;
use super::{export, import};
use figcraft_doc::{Document, NodeKind, StyleKind, StyleRun, TextData, TextDecoration, TextStyle};
use serde_json::Value;

fn object(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}

fn guid(id: u64) -> Kv<'static> {
    object(vec![("sessionID", Kv::Uint(8)), ("localID", Kv::Uint(id))])
}

fn node(id: u64, ty: &'static str, name: &str, parent: Option<u64>, mut fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    let mut out = vec![("guid", guid(id)), ("type", Kv::Enum(ty)), ("name", Kv::Str(name.into()))];
    if let Some(parent) = parent {
        out.push(("parentIndex", object(vec![("guid", guid(parent)), ("position", Kv::Str("a".into()))])));
    }
    out.append(&mut fields);
    object(out)
}

fn font() -> Kv<'static> {
    object(vec![("family", Kv::Str("Inter".into())), ("style", Kv::Str("Regular".into())), ("postscript", Kv::Str(String::new()))])
}

fn raw(base: Option<Kv<'static>>, runs: Vec<Kv<'static>>, chars: &str, ids: Vec<u64>, shared: Option<Option<Kv<'static>>>) -> Value {
    let mut layer = vec![
        ("fontName", font()),
        ("fontSize", Kv::Float(20.0)),
        ("textDecoration", Kv::Enum("UNDERLINE")),
        (
            "textData",
            object(vec![
                ("characters", Kv::Str(chars.into())),
                ("characterStyleIDs", Kv::Array(ids.into_iter().map(Kv::Uint).collect())),
                ("styleOverrideTable", Kv::Array(runs)),
            ]),
        ),
    ];
    if let Some(value) = base {
        layer.push(("textDecorationSkipInk", value));
    }
    let mut nodes = vec![node(1, "DOCUMENT", "Document", None, vec![]), node(2, "CANVAS", "Page", Some(1), vec![])];
    if let Some(shared) = shared {
        let mut fields =
            vec![("styleType", Kv::Enum("TEXT")), ("fontName", font()), ("fontSize", Kv::Float(20.0)), ("textDecoration", Kv::Enum("UNDERLINE"))];
        if let Some(value) = shared {
            fields.push(("textDecorationSkipInk", value));
        }
        nodes.push(node(10, "TEXT", "Body", Some(2), fields));
        layer.push(("styleIdForText", object(vec![("guid", guid(10))])));
    }
    nodes.push(node(11, "TEXT", "Label", Some(2), layer));
    let message = object(vec![("nodeChanges", Kv::Array(nodes))]);
    super::map::to_rest(&message, "Synthetic skip ink", crate::rest::MAX_NODES).0
}

fn imported(value: &Value) -> Document {
    let (doc, warnings) = crate::rest::import_fig(value, &|_| None).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    doc
}

fn label(doc: &Document) -> &TextData {
    doc.nodes().find(|n| n.name == "Label").unwrap().kind.text().unwrap()
}

fn skip_at(text: &TextData, index: usize) -> bool {
    text.runs.iter().find(|r| r.start <= index && index < r.end).map_or_else(|| text.style.look().skip_ink, |r| r.style.look().skip_ink)
}

#[test]
fn node_boolean_false_true_omitted_and_malformed_keep_typed_semantics() {
    for (raw_value, expected) in [(Some(Kv::Bool(true)), true), (Some(Kv::Bool(false)), false), (None, false), (Some(Kv::Str("true".into())), false)]
    {
        let doc = imported(&raw(raw_value.clone(), vec![], "A", vec![0], None));
        assert_eq!(label(&doc).style.look().skip_ink, expected, "source value {raw_value:?}");
    }
}

#[test]
fn runs_inherit_explicit_false_and_true_by_utf16_character_ids() {
    let table = vec![
        object(vec![("styleID", Kv::Uint(1))]),
        object(vec![("styleID", Kv::Uint(2)), ("textDecorationSkipInk", Kv::Bool(false))]),
        object(vec![("styleID", Kv::Uint(3)), ("textDecorationSkipInk", Kv::Bool(true))]),
    ];
    let doc = imported(&raw(Some(Kv::Bool(true)), table, "A😀BC", vec![0, 1, 1, 2, 3], None));
    let text = label(&doc);
    assert_eq!(text.characters.chars().count(), 4);
    assert_eq!(text.characters.encode_utf16().count(), 5, "the emoji occupies two source style-ID units");
    assert_eq!((0..text.char_count()).map(|i| skip_at(text, i)).collect::<Vec<_>>(), vec![true, true, false, true]);

    let false_base = imported(&raw(
        Some(Kv::Bool(false)),
        vec![object(vec![("styleID", Kv::Uint(1)), ("textDecorationSkipInk", Kv::Bool(true))])],
        "ab",
        vec![1, 0],
        None,
    ));
    assert_eq!((0..2).map(|i| skip_at(label(&false_base), i)).collect::<Vec<_>>(), vec![true, false]);
}

#[test]
fn local_skip_ink_override_preserves_shared_style_and_omitted_value_inherits() {
    for (local, expected) in [(Some(false), false), (None, true)] {
        let value = raw(local.map(Kv::Bool), vec![], "ab", vec![0, 0], Some(Some(Kv::Bool(true))));
        let doc = imported(&value);
        let node = doc.nodes().find(|n| n.name == "Label").unwrap();
        assert_eq!(node.kind.text().unwrap().style.look().skip_ink, expected);
        let linked = doc.style(node.styles.text.as_deref().unwrap()).unwrap();
        let StyleKind::Text { style } = &linked.kind else { panic!("linked definition must remain text") };
        assert!(style.look().skip_ink, "local value must not alter definition");
    }
}

#[test]
fn instance_path_override_preserves_explicit_false_over_true_main() {
    let main_text = node(
        21,
        "TEXT",
        "Label",
        Some(20),
        vec![
            ("fontName", font()),
            ("fontSize", Kv::Float(20.0)),
            ("textDecoration", Kv::Enum("UNDERLINE")),
            ("textDecorationSkipInk", Kv::Bool(true)),
            ("textData", object(vec![("characters", Kv::Str("A".into()))])),
        ],
    );
    let instance = node(
        30,
        "INSTANCE",
        "Copy",
        Some(2),
        vec![(
            "symbolData",
            object(vec![
                ("symbolID", guid(20)),
                (
                    "symbolOverrides",
                    Kv::Array(vec![object(vec![
                        ("guidPath", object(vec![("guids", Kv::Array(vec![guid(21)]))])),
                        ("textDecorationSkipInk", Kv::Bool(false)),
                    ])]),
                ),
            ]),
        )],
    );
    let message = object(vec![(
        "nodeChanges",
        Kv::Array(vec![
            node(1, "DOCUMENT", "Document", None, vec![]),
            node(2, "CANVAS", "Page", Some(1), vec![]),
            node(20, "SYMBOL", "Main", Some(2), vec![]),
            main_text,
            instance,
        ]),
    )]);
    let value = super::map::to_rest(&message, "Instance skip ink", crate::rest::MAX_NODES).0;
    let copy = value["document"]["children"][0]["children"].as_array().unwrap().iter().find(|n| n["name"] == "Copy").unwrap();
    let label = copy["children"].as_array().unwrap().iter().find(|n| n["name"] == "Label").unwrap();
    assert_eq!(label["style"]["textDecorationSkipInk"], false);
    let doc = imported(&value);
    let copy = doc.nodes().find(|n| n.name == "Copy").unwrap();
    let instance_label = doc.node(*copy.children.first().unwrap()).unwrap();
    assert!(!instance_label.kind.text().unwrap().style.look().skip_ink);
}

#[test]
fn export_writes_one_actual_node_value_and_complete_run_values_without_skip_only_warning() {
    let mut doc = Document::new("Skip ink export");
    let mut text = TextData::new("abcd");
    text.style.decoration = TextDecoration::Underline;
    text.style.set_look(|look| look.skip_ink = false);
    let mut skipped = text.style.clone();
    skipped.set_look(|look| look.skip_ink = true);
    text.runs = vec![StyleRun { start: 1, end: 3, style: skipped }];
    doc.create(doc.first_page().unwrap(), NodeKind::Text(text), "Label", 0.0, 0.0, 100.0, 30.0).unwrap();

    let (bytes, warnings) = export(&doc, None).unwrap();
    assert!(warnings.is_empty(), "skip-ink is supported: {warnings:?}");
    let archive = super::Archive::open(&bytes).unwrap();
    let (schema, body) = super::decode_canvas(&archive.canvas).unwrap();
    let message = schema.decode_message(schema.find("Message").unwrap(), &body).unwrap();
    let value = super::debug_json(&message);
    let layer = value["nodeChanges"].as_array().unwrap().iter().find(|n| n["name"] == "Label").unwrap();
    assert_eq!(layer["textDecorationSkipInk"], false);
    assert_eq!(layer.as_object().unwrap().keys().filter(|k| *k == "textDecorationSkipInk").count(), 1);
    let rows = layer["textData"]["styleOverrideTable"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["textDecorationSkipInk"], true);
    let (back, warnings) = import(&bytes, "Skip ink export").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!((0..4).map(|i| skip_at(label(&back), i)).collect::<Vec<_>>(), vec![false, true, true, false]);

    let mut custom = TextData::new("x");
    custom.style.set_look(|look| look.skip_ink = true);
    custom.style.set_look(|look| look.thickness = figcraft_doc::DecorationLength::Pixels(2.0));
    let mut custom_doc = Document::new("Unsupported decoration");
    custom_doc.create(custom_doc.first_page().unwrap(), NodeKind::Text(custom), "Custom", 0.0, 0.0, 100.0, 30.0).unwrap();
    let (_, warnings) = export(&custom_doc, None).unwrap();
    assert!(warnings.iter().any(|w| w.contains("text decoration looks")), "custom look warning missing: {warnings:?}");

    let mut variable = TextData::new("x");
    variable.style.variations.insert("wght".into(), 400.0);
    let mut variable_doc = Document::new("Variable font warning");
    variable_doc.create(variable_doc.first_page().unwrap(), NodeKind::Text(variable), "Variable", 0.0, 0.0, 100.0, 30.0).unwrap();
    let (_, warnings) = export(&variable_doc, None).unwrap();
    assert!(warnings.iter().any(|w| w.contains("variable font settings")), "variation warning missing: {warnings:?}");

    let mut shared = TextStyle { decoration: TextDecoration::Underline, ..TextStyle::default() };
    shared.set_look(|look| look.skip_ink = true);
    let mut local = TextStyle { decoration: TextDecoration::Underline, ..TextStyle::default() };
    local.set_look(|look| look.skip_ink = false);
    let mut linked_doc = Document::new("Shared skip ink");
    linked_doc.styles.push(figcraft_doc::Style {
        id: "body".into(),
        name: "Body".into(),
        description: String::new(),
        kind: StyleKind::Text { style: shared },
        bound: Default::default(),
    });
    let id = linked_doc
        .create(linked_doc.first_page().unwrap(), NodeKind::Text(TextData { style: local, ..TextData::new("x") }), "Label", 0.0, 0.0, 100.0, 30.0)
        .unwrap();
    linked_doc.node_mut(id).unwrap().styles.text = Some("body".into());
    let (bytes, warnings) = export(&linked_doc, None).unwrap();
    assert!(warnings.is_empty(), "skip-ink-only shared styles are supported: {warnings:?}");
    let (back, warnings) = import(&bytes, "Shared skip ink").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let node = back.nodes().find(|n| n.name == "Label").unwrap();
    assert!(!node.kind.text().unwrap().style.look().skip_ink);
    let linked = back.style(node.styles.text.as_deref().unwrap()).unwrap();
    let StyleKind::Text { style } = &linked.kind else { panic!("shared style link must remain text") };
    assert!(style.look().skip_ink, "local false must not alter the shared definition");
}

#[test]
fn skip_ink_without_an_underline_stays_out_of_the_look_and_exports_the_default() {
    // Plain text with the files' default value imports with the default look...
    let mut value = raw(Some(Kv::Bool(true)), vec![], "ab", vec![0, 0], None);
    value["document"]["children"][0]["children"][0]["style"].as_object_mut().unwrap().remove("textDecoration");
    let doc = imported(&value);
    assert_eq!(label(&doc).style.decoration, TextDecoration::None);
    assert!(label(&doc).style.decoration_look.is_none());

    // ...an underlined run of it keeps the layer's value...
    let mut value =
        raw(Some(Kv::Bool(true)), vec![object(vec![("styleID", Kv::Uint(1)), ("textDecoration", Kv::Enum("UNDERLINE"))])], "ab", vec![1, 0], None);
    value["document"]["children"][0]["children"][0]["style"].as_object_mut().unwrap().remove("textDecoration");
    let doc = imported(&value);
    assert_eq!((0..2).map(|i| skip_at(label(&doc), i)).collect::<Vec<_>>(), vec![true, false]);

    // ...and default native text writes the default, which reads back as the default look.
    let mut doc = Document::new("Plain");
    doc.create(doc.first_page().unwrap(), NodeKind::Text(TextData::new("x")), "Label", 0.0, 0.0, 100.0, 30.0).unwrap();
    let (bytes, _) = export(&doc, None).unwrap();
    let archive = super::Archive::open(&bytes).unwrap();
    let (schema, body) = super::decode_canvas(&archive.canvas).unwrap();
    let message = schema.decode_message(schema.find("Message").unwrap(), &body).unwrap();
    let value = super::debug_json(&message);
    let layer = value["nodeChanges"].as_array().unwrap().iter().find(|n| n["name"] == "Label").unwrap();
    assert_eq!(layer["textDecorationSkipInk"], true);
    let (back, _) = import(&bytes, "Plain").unwrap();
    assert!(label(&back).style.decoration_look.is_none());
}
