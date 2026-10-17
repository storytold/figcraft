//! Text layers and their shared text styles on import: which of the style, the layer's own
//! values and an instance's overrides win, on messages built in code.

use figcraft_doc::{Document, LetterSpacing, StyleKind, TextCase, TextDecoration};
use serde_json::Value;

use super::kiwi::Kv;

type Fields = Vec<(&'static str, Kv<'static>)>;

const NO_STYLE: u64 = 4_294_967_295;

fn object(fields: Fields) -> Kv<'static> {
    Kv::Object(fields)
}

fn guid(id: u64) -> Kv<'static> {
    object(vec![("sessionID", Kv::Uint(9)), ("localID", Kv::Uint(id))])
}

fn link(id: u64) -> Kv<'static> {
    object(vec![("guid", guid(id))])
}

fn node(id: u64, ty: &'static str, name: &str, parent: Option<u64>, mut more: Fields) -> Kv<'static> {
    let mut fields = vec![("guid", guid(id)), ("type", Kv::Enum(ty)), ("name", Kv::Str(name.into()))];
    if let Some(parent) = parent {
        fields.push(("parentIndex", object(vec![("guid", guid(parent)), ("position", Kv::Str("a".into()))])));
    }
    fields.append(&mut more);
    object(fields)
}

fn font(family: &str) -> Kv<'static> {
    object(vec![("family", Kv::Str(family.into())), ("style", Kv::Str("Regular".into())), ("postscript", Kv::Str(String::new()))])
}

fn chars(text: &str) -> Kv<'static> {
    object(vec![("characters", Kv::Str(text.into()))])
}

/// The REST JSON of a page holding `nodes`.
fn rest(nodes: Vec<Kv<'static>>) -> Value {
    let base = vec![node(1, "DOCUMENT", "Document", None, vec![]), node(2, "CANVAS", "Page", Some(1), vec![])];
    let message = object(vec![("nodeChanges", Kv::Array(base.into_iter().chain(nodes).collect()))]);
    super::map::to_rest(&message, "Synthetic text", crate::rest::MAX_NODES).0
}

fn import(value: &Value) -> Document {
    let (doc, warnings) = crate::rest::import_fig(value, &|_| None).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    doc
}

/// Depth-first search for a REST node by name below `v`.
fn find<'v>(v: &'v Value, name: &str) -> Option<&'v Value> {
    if v["name"] == name {
        return Some(v);
    }
    v["children"].as_array()?.iter().find_map(|c| find(c, name))
}

/// A shared text style (Inter, 20 px, with `shared` fields) and a layer linking it whose own
/// copy of the typography is stale (40 px, another family) and which sets `local`.
fn linked(shared: Fields, local: Fields) -> Document {
    let mut definition = vec![("styleType", Kv::Enum("TEXT")), ("fontName", font("Inter")), ("fontSize", Kv::Float(20.0))];
    definition.extend(shared);
    let mut layer = vec![("fontName", font("Stale copy")), ("fontSize", Kv::Float(40.0)), ("styleIdForText", link(10)), ("textData", chars("a\nb"))];
    layer.extend(local);
    import(&rest(vec![node(10, "TEXT", "Shared", Some(2), definition), node(11, "TEXT", "Label", Some(2), layer)]))
}

struct Seen {
    case: TextCase,
    decoration: TextDecoration,
    spacing: f64,
    indent: f64,
    shared_case: TextCase,
}

fn seen(doc: &Document) -> Seen {
    let label = doc.nodes().find(|n| n.name == "Label").unwrap();
    let text = label.kind.text().unwrap();
    // The shared style's typography beats the layer's stale copy, whatever the layer sets locally.
    assert_eq!((text.style.font_family.as_str(), text.style.font_size), ("Inter", 20.0));
    let linked = doc.style(label.styles.text.as_deref().unwrap()).unwrap();
    assert_eq!(linked.name, "Shared");
    let StyleKind::Text { style } = &linked.kind else { panic!("the linked definition must stay a text style") };
    Seen {
        case: text.style.case,
        decoration: text.style.decoration,
        spacing: text.paragraph_spacing,
        indent: text.paragraph_indent,
        shared_case: style.case,
    }
}

#[test]
fn local_case_decoration_and_paragraph_values_beat_the_shared_style() {
    let shared = vec![
        ("textCase", Kv::Enum("UPPER")),
        ("textDecoration", Kv::Enum("UNDERLINE")),
        ("paragraphSpacing", Kv::Float(24.0)),
        ("paragraphIndent", Kv::Float(12.0)),
    ];
    // Nothing set on the layer: the style's values stay.
    let s = seen(&linked(shared.clone(), vec![]));
    assert_eq!((s.case, s.decoration, s.spacing, s.indent), (TextCase::Upper, TextDecoration::Underline, 24.0, 12.0));
    // Explicit values on the layer win, and "original", "none" and zero clear the style's.
    let cleared = vec![
        ("textCase", Kv::Enum("ORIGINAL")),
        ("textDecoration", Kv::Enum("NONE")),
        ("paragraphSpacing", Kv::Float(0.0)),
        ("paragraphIndent", Kv::Float(0.0)),
    ];
    let s = seen(&linked(shared.clone(), cleared));
    assert_eq!((s.case, s.decoration, s.spacing, s.indent), (TextCase::Original, TextDecoration::None, 0.0, 0.0));
    assert_eq!(s.shared_case, TextCase::Upper, "a local value must not change the definition");
    let local = vec![
        ("textCase", Kv::Enum("SMALL_CAPS_FORCED")),
        ("textDecoration", Kv::Enum("STRIKETHROUGH")),
        ("paragraphSpacing", Kv::Float(6.0)),
        ("paragraphIndent", Kv::Float(3.0)),
    ];
    let s = seen(&linked(vec![], local));
    assert_eq!((s.case, s.decoration, s.spacing, s.indent), (TextCase::SmallCaps, TextDecoration::Strikethrough, 6.0, 3.0));
    // A value the importer does not know leaves the style's in place.
    let s = seen(&linked(shared, vec![("textCase", Kv::Enum("UNSUPPORTED_CASE")), ("textDecoration", Kv::Enum("WAVY"))]));
    assert_eq!((s.case, s.decoration), (TextCase::Upper, TextDecoration::Underline));
}

#[test]
fn an_instance_override_beats_the_shared_style_and_a_detach_unlinks_it() {
    let value = rest(vec![
        node(10, "TEXT", "Body", Some(2), vec![("styleType", Kv::Enum("TEXT")), ("fontName", font("Inter")), ("fontSize", Kv::Float(14.0))]),
        node(20, "SYMBOL", "Main", Some(2), vec![]),
        node(
            21,
            "TEXT",
            "Label",
            Some(20),
            vec![("fontName", font("Inter")), ("fontSize", Kv::Float(14.0)), ("styleIdForText", link(10)), ("textData", chars("Hi"))],
        ),
        node(
            30,
            "INSTANCE",
            "Inst",
            Some(2),
            vec![(
                "symbolData",
                object(vec![
                    ("symbolID", guid(20)),
                    (
                        "symbolOverrides",
                        Kv::Array(vec![object(vec![
                            ("guidPath", object(vec![("guids", Kv::Array(vec![guid(21)]))])),
                            ("fontSize", Kv::Float(30.0)),
                        ])]),
                    ),
                ]),
            )],
        ),
        // An explicit "no style" detaches, even when the older field still names a style.
        node(
            40,
            "TEXT",
            "Detached",
            Some(2),
            vec![
                ("fontName", font("Roboto")),
                ("fontSize", Kv::Float(40.0)),
                ("styleIdForText", object(vec![("guid", object(vec![("sessionID", Kv::Uint(NO_STYLE)), ("localID", Kv::Uint(NO_STYLE))]))])),
                ("inheritTextStyleID", guid(10)),
                ("textData", chars("ab")),
            ],
        ),
    ]);
    let label = find(find(&value["document"], "Inst").unwrap(), "Label").unwrap();
    assert_eq!(label["style"]["fontSize"].as_f64(), Some(30.0), "the override beats the style");
    let detached = find(&value["document"], "Detached").unwrap();
    assert!(detached["styles"].get("text").is_none(), "{}", detached["styles"]);
    assert_eq!(detached["style"]["fontSize"].as_f64(), Some(40.0));
}

#[test]
fn a_text_property_value_drops_the_run_styles_made_for_the_main_text() {
    let ids = [0u64, 0, 0, 1, 1, 1].map(Kv::Uint).to_vec();
    let value = rest(vec![
        node(
            20,
            "SYMBOL",
            "Main",
            Some(2),
            vec![(
                "componentPropDefs",
                Kv::Array(vec![object(vec![
                    ("id", guid(70)),
                    ("name", Kv::Str("Text".into())),
                    ("type", Kv::Enum("TEXT")),
                    ("initialValue", object(vec![("textValue", chars("Hi you"))])),
                ])]),
            )],
        ),
        node(
            21,
            "TEXT",
            "Label",
            Some(20),
            vec![
                ("fontName", font("Inter")),
                ("fontSize", Kv::Float(12.0)),
                (
                    "textData",
                    object(vec![
                        ("characters", Kv::Str("Hi you".into())),
                        ("characterStyleIDs", Kv::Array(ids)),
                        ("styleOverrideTable", Kv::Array(vec![object(vec![("styleID", Kv::Uint(1)), ("fontSize", Kv::Float(30.0))])])),
                    ]),
                ),
                (
                    "parameterConsumptionMap",
                    object(vec![(
                        "entries",
                        Kv::Array(vec![object(vec![
                            ("variableField", Kv::Enum("TEXT_DATA")),
                            ("variableData", object(vec![("value", object(vec![("propRefValue", object(vec![("defId", guid(70))]))]))])),
                        ])]),
                    )]),
                ),
            ],
        ),
        node(
            30,
            "INSTANCE",
            "Inst",
            Some(2),
            vec![
                ("symbolData", object(vec![("symbolID", guid(20))])),
                (
                    "componentPropAssignments",
                    Kv::Array(vec![object(vec![("defID", guid(70)), ("value", object(vec![("textValue", chars("Goodbye"))]))])]),
                ),
            ],
        ),
    ]);
    let main = find(find(&value["document"], "Main").unwrap(), "Label").unwrap();
    let kept: Vec<u64> = main["characterStyleOverrides"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    assert_eq!(kept, [0, 0, 0, 1, 1, 1], "the main keeps its run styles");
    let inst = find(&value["document"], "Inst").unwrap();
    let layer = find(inst, "Label").unwrap();
    assert_eq!(layer["characters"], "Goodbye");
    assert!(layer.get("characterStyleOverrides").is_none(), "positions of the main's runs do not fit the property text");
}

#[test]
fn percent_letter_spacing_stays_a_percentage() {
    let spacing = |units: &'static str, value: f32| object(vec![("value", Kv::Float(value)), ("units", Kv::Enum(units))]);
    let layer = |id, name: &str, letter: Kv<'static>| {
        node(
            id,
            "TEXT",
            name,
            Some(2),
            vec![("fontName", font("Inter")), ("fontSize", Kv::Float(20.0)), ("letterSpacing", letter), ("textData", chars("Hello"))],
        )
    };
    let doc = import(&rest(vec![layer(10, "Percent", spacing("PERCENT", 5.0)), layer(11, "Pixels", spacing("PIXELS", 2.0))]));
    let spacing = |name: &str| doc.nodes().find(|n| n.name == name).unwrap().kind.text().unwrap().style.letter_spacing;
    assert_eq!(spacing("Percent"), LetterSpacing::Percent(5.0));
    assert_eq!(spacing("Pixels"), LetterSpacing::Pixels(2.0));
}

#[test]
fn the_private_percent_is_read_for_fig_imports_only() {
    let value = rest(vec![node(
        10,
        "TEXT",
        "Label",
        Some(2),
        vec![
            ("fontName", font("Inter")),
            ("fontSize", Kv::Float(20.0)),
            ("letterSpacing", object(vec![("value", Kv::Float(5.0)), ("units", Kv::Enum("PERCENT"))])),
            ("textData", chars("Hello")),
        ],
    )]);
    let spacing = |doc: &Document| doc.nodes().find(|n| n.name == "Label").unwrap().kind.text().unwrap().style.letter_spacing;
    assert_eq!(spacing(&import(&value)), LetterSpacing::Percent(5.0));
    // The same JSON through the public REST import is pixels.
    let (public, _) = crate::rest::import(&value, &|_| None).unwrap();
    assert_eq!(spacing(&public), LetterSpacing::Pixels(1.0));
}
