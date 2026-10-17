//! Mapper → private REST ingress tests for source text-path fields.

use super::kiwi::Kv;

const SCHEMA: &str = "
enum NodeType DOCUMENT CANVAS TEXT TEXT_PATH
enum StyleType NONE TEXT
enum TextDecoration NONE UNDERLINE STRIKETHROUGH
struct GUID uint sessionID, uint localID
struct StyleId GUID guid
struct ParentIndex GUID guid, string position
struct Vector float x, float y
struct Matrix float m00, float m01, float m02, float m10, float m11, float m12
struct FontName string family, string style
message TextOverride uint styleID, FontName fontName, float fontSize
message TextData string characters, uint[] characterStyleIDs, TextOverride[] styleOverrideTable
message TextPathStart float tValue, bool forward
message VectorData uint vectorNetworkBlob, Vector normalizedSize
message Blob byte[] bytes
message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, Vector size, Matrix transform, TextData textData, float fontSize, FontName fontName, TextPathStart textPathStart, VectorData vectorData, StyleType styleType, StyleId styleIdForText, TextDecoration textDecoration
message Message NodeChange[] nodeChanges, Blob[] blobs
";
use figcraft_doc::{NodeKind, TextPath};

fn o(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}

fn a(values: Vec<Kv<'static>>) -> Kv<'static> {
    Kv::Array(values)
}

fn f(value: f32) -> Kv<'static> {
    Kv::Float(value)
}

fn u(value: u64) -> Kv<'static> {
    Kv::Uint(value)
}

fn s(value: &'static str) -> Kv<'static> {
    Kv::Str(value.into())
}

fn point(x: f32, y: f32) -> Kv<'static> {
    o(vec![("x", f(x)), ("y", f(y))])
}

fn guid(id: u64) -> Kv<'static> {
    o(vec![("sessionID", u(1)), ("localID", u(id))])
}

fn node(id: u64, parent: Option<u64>, ty: &'static str, fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    let mut node = vec![("guid", guid(id)), ("type", s(ty))];
    if let Some(parent) = parent {
        node.push(("parentIndex", o(vec![("guid", guid(parent)), ("position", s("!"))])));
    }
    node.extend(fields);
    o(node)
}

fn blob() -> Vec<u8> {
    let mut bytes = Vec::new();
    let u32le = |out: &mut Vec<u8>, n: u32| out.extend_from_slice(&n.to_le_bytes());
    let f32le = |out: &mut Vec<u8>, n: f32| out.extend_from_slice(&n.to_le_bytes());
    u32le(&mut bytes, 2); // vertices
    u32le(&mut bytes, 1); // segments
    u32le(&mut bytes, 0); // regions
    for (x, y) in [(0.0, 0.0), (100.0, 0.0)] {
        u32le(&mut bytes, 0);
        f32le(&mut bytes, x);
        f32le(&mut bytes, y);
    }
    u32le(&mut bytes, 0x01ff_ffff); // source start-segment marker
    u32le(&mut bytes, 0);
    f32le(&mut bytes, 0.0);
    f32le(&mut bytes, 0.0);
    u32le(&mut bytes, 1);
    f32le(&mut bytes, 0.0);
    f32le(&mut bytes, 0.0);
    bytes
}

fn canvas() -> Vec<u8> {
    let schema = super::kiwi::Schema::parse(SCHEMA).unwrap();
    let message = message(true);
    let message = super::kiwi::encode::message(&schema, schema.find("Message").unwrap(), &message).unwrap();
    let mut out = b"fig-kiwi\0\0\0\0".to_vec();
    for raw in [super::kiwi::encode::schema(&schema), message] {
        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &raw).unwrap();
        let compressed = encoder.finish().unwrap();
        out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        out.extend_from_slice(&compressed);
    }
    out
}

fn message(with_path: bool) -> Kv<'static> {
    let mut nodes = vec![node(0, None, "DOCUMENT", vec![]), node(1, Some(0), "CANVAS", vec![])];
    let mut blobs = Vec::new();
    if with_path {
        blobs.push(o(vec![("bytes", Kv::Bytes(blob()))]));
        let text = o(vec![
            ("characters", s("alpha")),
            ("characterStyleIDs", a(vec![u(0), u(0), u(0), u(2), u(2)])),
            (
                "styleOverrideTable",
                a(vec![o(vec![("styleID", u(2)), ("fontName", o(vec![("family", s("Inter")), ("style", s("Medium"))])), ("fontSize", f(30.0))])]),
            ),
        ]);
        nodes.push(node(
            2,
            Some(1),
            "TEXT_PATH",
            vec![
                ("name", s("alpha path")),
                ("size", point(100.0, 0.0)),
                ("transform", o(vec![("m00", f(1.0)), ("m01", f(0.0)), ("m02", f(0.0)), ("m10", f(0.0)), ("m11", f(1.0)), ("m12", f(0.0))])),
                ("textData", text),
                ("fontSize", f(24.0)),
                ("fontName", o(vec![("family", s("Inter")), ("style", s("Regular"))])),
                ("textPathStart", o(vec![("tValue", f(0.5)), ("forward", Kv::Bool(true))])),
                ("vectorData", o(vec![("vectorNetworkBlob", u(0)), ("normalizedSize", point(100.0, 0.0))])),
            ],
        ));
    } else {
        nodes.push(node(2, Some(1), "TEXT_PATH", vec![("name", s("broken path")), ("textData", o(vec![("characters", s("fallback"))]))]));
    }
    o(vec![("nodeChanges", a(nodes)), ("blobs", a(blobs))])
}

fn linked_underline_message() -> Kv<'static> {
    let Kv::Object(mut root) = message(true) else { panic!("expected message object") };
    let (_, Kv::Array(nodes)) = root.iter_mut().find(|(key, _)| *key == "nodeChanges").unwrap() else { panic!("expected node changes array") };
    let text = nodes.iter_mut().find(|n| n.get("name").and_then(Kv::str) == Some("alpha path")).unwrap();
    let Kv::Object(fields) = text else { panic!("expected text node object") };
    fields.push(("styleIdForText", o(vec![("guid", guid(3))])));
    nodes.push(node(
        3,
        Some(1),
        "TEXT",
        vec![
            ("name", s("Shared style")),
            ("styleType", s("TEXT")),
            ("fontName", o(vec![("family", s("Inter")), ("style", s("Regular"))])),
            ("fontSize", f(24.0)),
            ("textDecoration", s("UNDERLINE")),
        ],
    ));
    Kv::Object(root)
}

#[test]
fn mapped_text_path_reaches_native_model_with_runs_and_no_standin_warning() {
    let message = message(true);
    let (json, warnings, _) = super::map::to_rest(&message, "Text path", crate::rest::MAX_NODES);
    assert!(!warnings.iter().any(|w| w.contains("text off its path")), "{warnings:?}");
    let (doc, warnings) = crate::rest::import_fig(&json, &|_| None).unwrap();
    assert!(!warnings.iter().any(|w| w.contains("text off its path")), "{warnings:?}");
    let node = doc.nodes().find(|n| n.name == "alpha path").unwrap();
    let NodeKind::Text(text) = &node.kind else { panic!("expected text") };
    let path = text.path.as_deref().unwrap();
    assert_eq!(path, &TextPath { d: "M0,0 L100,0".into(), start: 0.5, flip: false });
    assert_eq!(text.characters, "alpha");
    assert_eq!(text.runs.len(), 1);
    assert_eq!(text.runs[0].start, 3);
    assert_eq!(text.runs[0].end, 5);
}

#[test]
fn public_rest_import_ignores_private_fig_text_path_field() {
    let message = message(true);
    let (json, _, _) = super::map::to_rest(&message, "Text path", crate::rest::MAX_NODES);
    // The mapper's private field is accepted only by import_fig, never by public REST import.
    let path = json["document"]["children"][0]["children"][0]["_figTextPath"].clone();
    assert!(path.is_object());
    let (doc, _) = crate::rest::import(&json, &|_| None).unwrap();
    let node = doc.nodes().find(|n| n.name == "alpha path").unwrap();
    let NodeKind::Text(text) = &node.kind else { panic!("expected text") };
    assert!(text.path.is_none());
}

#[test]
fn serialized_kiwi_canvas_import_decodes_start_and_builds_native_path() {
    let (doc, warnings) = super::import(&canvas(), "Serialized text path").unwrap();
    let node = doc.nodes().find(|n| n.name == "alpha path").unwrap();
    let NodeKind::Text(text) = &node.kind else { panic!("expected text") };
    let path = text.path.as_deref().unwrap();
    assert_eq!(path, &TextPath { d: "M0,0 L100,0".into(), start: 0.5, flip: false });
    assert_eq!(text.characters, "alpha");
    assert!(!warnings.iter().any(|w| w.contains("text off its path")), "{warnings:?}");
}

#[test]
fn malformed_text_path_falls_back_with_specific_warning() {
    let message = message(false);
    let (json, warnings, _) = super::map::to_rest(&message, "Text path", crate::rest::MAX_NODES);
    assert!(warnings.iter().any(|w| w.contains("text path has no start data") && w.contains("text imports off its path")), "{warnings:?}");
    let (doc, _) = crate::rest::import_fig(&json, &|_| None).unwrap();
    let node = doc.nodes().find(|n| n.name == "broken path").unwrap();
    assert!(matches!(&node.kind, NodeKind::Text(text) if text.path.is_none()));
}

#[test]
fn linked_text_style_decoration_warns_from_effective_mapped_style() {
    let message = linked_underline_message();
    let (json, warnings, _) = super::map::to_rest(&message, "Text path", crate::rest::MAX_NODES);
    assert_eq!(json["document"]["children"][0]["children"][0]["style"]["textDecoration"], "UNDERLINE");
    assert!(warnings.iter().any(|w| w == "text path underline or strikethrough decoration is not rendered"), "{warnings:?}");
}
