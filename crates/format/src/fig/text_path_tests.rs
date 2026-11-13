//! Mapper → private REST ingress tests for source text-path fields.

use super::kiwi::Kv;

const SCHEMA: &str = "
enum NodeType DOCUMENT CANVAS TEXT TEXT_PATH SYMBOL INSTANCE
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
message SymbolData GUID symbolID, float uniformScaleFactor
message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, Vector size, Matrix transform, TextData textData, float fontSize, FontName fontName, TextPathStart textPathStart, VectorData vectorData, StyleType styleType, StyleId styleIdForText, TextDecoration textDecoration, SymbolData symbolData
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
    canvas_of(message(true))
}

fn canvas_of(message: Kv<'static>) -> Vec<u8> {
    let schema = super::kiwi::Schema::parse(SCHEMA).unwrap();
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

/// A component holding a text on a path (path 100 long) and a half-scale instance of it.
fn scaled_instance_message() -> Kv<'static> {
    let identity = || o(vec![("m00", f(1.0)), ("m01", f(0.0)), ("m02", f(0.0)), ("m10", f(0.0)), ("m11", f(1.0)), ("m12", f(0.0))]);
    let nodes = vec![
        node(0, None, "DOCUMENT", vec![]),
        node(1, Some(0), "CANVAS", vec![]),
        node(5, Some(1), "SYMBOL", vec![("name", s("Main")), ("size", point(100.0, 100.0)), ("transform", identity())]),
        node(
            6,
            Some(5),
            "TEXT_PATH",
            vec![
                ("name", s("arc")),
                ("size", point(100.0, 0.0)),
                ("transform", identity()),
                ("textData", o(vec![("characters", s("alpha"))])),
                ("fontSize", f(24.0)),
                ("fontName", o(vec![("family", s("Inter")), ("style", s("Regular"))])),
                ("textPathStart", o(vec![("tValue", f(0.5)), ("forward", Kv::Bool(true))])),
                ("vectorData", o(vec![("vectorNetworkBlob", u(0)), ("normalizedSize", point(100.0, 0.0))])),
            ],
        ),
        node(
            9,
            Some(1),
            "INSTANCE",
            vec![
                ("name", s("Half")),
                ("size", point(50.0, 50.0)),
                ("transform", identity()),
                ("symbolData", o(vec![("symbolID", guid(5)), ("uniformScaleFactor", f(0.5))])),
            ],
        ),
    ];
    o(vec![("nodeChanges", a(nodes)), ("blobs", a(vec![o(vec![("bytes", Kv::Bytes(blob()))])]))])
}

#[test]
fn a_text_on_a_path_in_a_scaled_instance_has_its_path_scaled() {
    let (json, warnings, _) = super::map::to_rest(&scaled_instance_message(), "Scaled path", crate::rest::MAX_NODES);
    assert!(!warnings.iter().any(|w| w.contains("text path")), "{warnings:?}");
    let (doc, _) = crate::rest::import_fig(&json, &|_| None).unwrap();
    let path_of = |parent: &str| {
        let parent = doc.nodes().find(|n| n.name == parent).unwrap();
        let node = doc.node(parent.children[0]).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("expected text") };
        text.path.as_deref().unwrap().d.clone()
    };
    assert_eq!(path_of("Main"), "M0,0 L100,0");
    assert_eq!(path_of("Half"), "M0,0 L50,0");
}

// The `.fig` export writes text on a path.

use figcraft_doc::geom::kurbo::PathEl;
use figcraft_doc::{Document, NodeId, TextAlignV};

const W1: &str = "text on a path with more than one line was written as plain text";
const W2: &str = "text on a path the format can't hold was written as plain text";
const W3: &str = "the parts of a text's path after the first were left out";
const W4: &str = "text on a path in an instance but not in its main component, or the other way round, was written as the main has it";
const MARK: u32 = 0x01ff_ffff;

/// What an export wrote: its warnings, the layer records and the blobs.
struct Written {
    warnings: Vec<String>,
    records: Vec<Kv<'static>>,
    blobs: Vec<Vec<u8>>,
}

impl Written {
    fn of(doc: &Document) -> (Vec<u8>, Written) {
        let (bytes, warnings) = super::export(doc, None).unwrap();
        let archive = super::Archive::open(&bytes).unwrap();
        let (schema, message) = super::decode_canvas(&archive.canvas).unwrap();
        let schema: &'static super::kiwi::Schema = Box::leak(Box::new(schema));
        let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
        let records = msg.get("nodeChanges").unwrap().list().to_vec();
        let blobs = msg
            .get("blobs")
            .map(|b| b.list().iter().filter_map(|x| x.get("bytes").and_then(Kv::bytes).map(<[u8]>::to_vec)).collect())
            .unwrap_or_default();
        (bytes, Written { warnings, records, blobs })
    }

    fn record(&self, name: &str) -> &Kv<'static> {
        self.records.iter().find(|r| r.get("name").and_then(Kv::str) == Some(name)).unwrap_or_else(|| panic!("no record {name}"))
    }

    /// The network in the blob a record's `vectorData` names.
    fn network(&self, record: &Kv<'static>) -> super::network::Network {
        let i = record.get("vectorData").unwrap().get("vectorNetworkBlob").and_then(Kv::f64).unwrap() as usize;
        super::network::decode(self.blobs.get(i).unwrap()).unwrap()
    }

    fn warned(&self, text: &str) -> bool {
        self.warnings.iter().any(|w| w == text)
    }
}

fn ty<'a>(r: &'a Kv<'static>) -> &'a str {
    r.get("type").and_then(Kv::str).unwrap()
}

fn start_of(r: &Kv<'static>) -> (f64, bool) {
    let s = r.get("textPathStart").unwrap_or_else(|| panic!("no textPathStart"));
    (s.get("tValue").and_then(Kv::f64).unwrap(), s.get("forward").and_then(Kv::bool).unwrap())
}

/// A page holding one text per `(name, characters, d, start, flip, width, height)`.
fn doc_of(texts: &[(&str, &str, &str, f64, bool, f64, f64)]) -> Document {
    let mut doc = Document::new("paths");
    let page = doc.first_page().unwrap();
    for (name, chars, d, start, flip, w, h) in texts {
        let id = doc.create(page, NodeKind::Text(Default::default()), name, 0.0, 0.0, *w, *h).unwrap();
        let td = doc.node_mut(id).unwrap().kind.text_mut().unwrap();
        td.characters = (*chars).into();
        if !d.is_empty() || !chars.is_empty() {
            td.path = Some(Box::new(TextPath { d: (*d).into(), start: *start, flip: *flip }));
        }
    }
    doc
}

fn text_of<'a>(doc: &'a Document, name: &str) -> &'a figcraft_doc::TextData {
    let node = doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no layer {name}"));
    let NodeKind::Text(td) = &node.kind else { panic!("{name} is not text") };
    td
}

fn elements(d: &str) -> Vec<(char, Vec<(f64, f64)>)> {
    figcraft_doc::geom::svg_path(d)
        .unwrap()
        .elements()
        .iter()
        .map(|el| match el {
            PathEl::MoveTo(p) => ('M', vec![(p.x, p.y)]),
            PathEl::LineTo(p) => ('L', vec![(p.x, p.y)]),
            PathEl::QuadTo(a, b) => ('Q', vec![(a.x, a.y), (b.x, b.y)]),
            PathEl::CurveTo(a, b, c) => ('C', vec![(a.x, a.y), (b.x, b.y), (c.x, c.y)]),
            PathEl::ClosePath => ('Z', vec![]),
        })
        .collect()
}

fn same_path(a: &str, b: &str) {
    let (x, y) = (elements(a), elements(b));
    assert_eq!(x.len(), y.len(), "{a} / {b}");
    for ((ka, pa), (kb, pb)) in x.iter().zip(&y) {
        assert_eq!(ka, kb, "{a} / {b}");
        for (p, q) in pa.iter().zip(pb) {
            assert!((p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3, "{a} / {b}");
        }
    }
}

fn no_text_path_warning(warnings: &[String]) {
    assert!(!warnings.iter().any(|w| w.contains("text path") || w.contains("text off its path")), "{warnings:?}");
}

#[test]
fn a_text_on_a_path_survives_a_fig_export() {
    let (doc, _) = super::import(&canvas(), "Text path").unwrap();
    let (bytes, warnings) = super::export(&doc, None).unwrap();
    assert!(!warnings.iter().any(|w| w.contains("text on a path")), "{warnings:?}");
    let (_, w) = Written::of(&doc);
    let r = w.record("alpha path");
    assert_eq!(ty(r), "TEXT_PATH");
    assert_eq!(start_of(r), (0.5, true));
    assert_eq!(r.get("textAlignVertical").and_then(Kv::str), Some("TOP"));
    let normalized = r.get("vectorData").unwrap().get("normalizedSize").unwrap();
    let size = r.get("size").unwrap();
    for k in ["x", "y"] {
        assert_eq!(normalized.get(k).and_then(Kv::f64), size.get(k).and_then(Kv::f64));
    }
    assert_eq!((size.get("x").and_then(Kv::f64), size.get("y").and_then(Kv::f64)), (Some(100.0), Some(0.0)));
    let net = w.network(r);
    assert_eq!(net.vertices.iter().map(|v| (v.x, v.y)).collect::<Vec<_>>(), [(0.0, 0.0), (100.0, 0.0)]);
    assert_eq!(net.segments.len(), 1);
    assert_eq!((net.segments[0].style, net.segments[0].ta, net.segments[0].tb), (MARK, (0.0, 0.0), (0.0, 0.0)));
    assert!(net.regions.is_empty());
    let (back, import_warnings) = super::import(&bytes, "Back").unwrap();
    no_text_path_warning(&import_warnings);
    let td = text_of(&back, "alpha path");
    assert_eq!(td.path.as_deref(), Some(&TextPath { d: "M0,0 L100,0".into(), start: 0.5, flip: false }));
    assert_eq!(td.characters, "alpha");
    assert_eq!((td.runs.len(), td.runs[0].start, td.runs[0].end), (1, 3, 5));
}

#[test]
fn the_start_is_written_by_length_from_either_side() {
    const CURVE: &str = "M0,82.5 C70,-27.5 230,-27.5 300,82.5 C370,192.5 530,192.5 600,82.5";
    let mut doc = doc_of(&[
        ("far", "alpha", CURVE, 0.7490379757, false, 600.0, 165.0),
        ("near", "alpha", CURVE, 0.004744078, false, 600.0, 165.0),
        ("flipped", "alpha", "M0,0 L600,0", 0.16013986, true, 600.0, 0.0),
    ]);
    let page = doc.first_page().unwrap();
    doc.create(page, NodeKind::Text(Default::default()), "plain", 0.0, 0.0, 50.0, 20.0).unwrap();
    let (bytes, w) = Written::of(&doc);
    let marked = |name: &str| -> (usize, f64, bool) {
        let r = w.record(name);
        let net = w.network(r);
        let at: Vec<usize> = net.segments.iter().enumerate().filter(|(_, s)| s.style == MARK).map(|(i, _)| i).collect();
        assert_eq!(at.len(), 1, "{name}");
        let (t, forward) = start_of(r);
        (at[0], t, forward)
    };
    let (i, t, forward) = marked("far");
    assert_eq!((i, forward), (1, true));
    assert!((t - 0.49800667).abs() < 1e-5, "{t}");
    let (i, t, forward) = marked("near");
    assert_eq!((i, forward), (0, true));
    assert!((t - 0.0086954795).abs() < 1e-5, "{t}");
    assert!((t - 0.009488156).abs() > 5e-4, "{t}");
    let (i, t, forward) = marked("flipped");
    assert_eq!((i, forward), (0, false));
    assert!((t - 0.83986014).abs() < 1e-6, "{t}");
    let plain = w.record("plain");
    assert_eq!(ty(plain), "TEXT");
    assert!(plain.get("textPathStart").is_none() && plain.get("vectorData").is_none());
    let (back, _) = super::import(&bytes, "Back").unwrap();
    for (name, d, start, flip) in
        [("far", CURVE, 0.7490379757, false), ("near", CURVE, 0.004744078, false), ("flipped", "M0,0 L600,0", 0.16013986, true)]
    {
        let path = text_of(&back, name).path.as_deref().unwrap_or_else(|| panic!("{name} lost its path"));
        same_path(&path.d, d);
        assert!((path.start - start).abs() < 1e-6, "{name}: {}", path.start);
        assert_eq!(path.flip, flip, "{name}");
    }
}

#[test]
fn a_closed_path_is_written_as_a_cycle() {
    let doc = doc_of(&[("square", "alpha", "M0,0 L100,0 L100,100 L0,100 Z", 0.3, false, 100.0, 100.0)]);
    let (bytes, w) = Written::of(&doc);
    let r = w.record("square");
    let net = w.network(r);
    assert_eq!((net.vertices.len(), net.segments.len()), (4, 4));
    assert_eq!(net.segments.iter().map(|s| (s.a, s.b)).collect::<Vec<_>>(), [(0, 1), (1, 2), (2, 3), (3, 0)]);
    assert_eq!(net.segments.iter().map(|s| s.style).collect::<Vec<_>>(), [0, MARK, 0, 0]);
    assert!((start_of(r).0 - 0.2).abs() < 1e-6);
    let come_back = |bytes: &[u8]| {
        let (back, warnings) = super::import(bytes, "Back").unwrap();
        no_text_path_warning(&warnings);
        text_of(&back, "square").path.as_deref().unwrap().clone()
    };
    let once = come_back(&bytes);
    same_path(&once.d, "M100,0 L100,100 L0,100 L0,0 L100,0 Z");
    assert!((once.start - 0.05).abs() < 1e-6, "{}", once.start);
    let (back, _) = super::import(&bytes, "Back").unwrap();
    let (again, _) = super::export(&back, None).unwrap();
    let twice = come_back(&again);
    same_path(&twice.d, &once.d);
    assert!((twice.start - once.start).abs() < 1e-6);

    let open_end = doc_of(&[("tri", "alpha", "M0,0 L100,0 L100,100 L0,0", 0.5, false, 100.0, 100.0)]);
    let (_, w) = Written::of(&open_end);
    let net = w.network(w.record("tri"));
    assert_eq!(net.vertices.len(), 3);
    assert_eq!(net.segments.iter().map(|s| (s.a, s.b)).collect::<Vec<_>>(), [(0, 1), (1, 2), (2, 0)]);
}

#[test]
fn paths_the_format_cant_hold_fall_back_with_their_own_warning() {
    let plain = |doc: &Document, warning: &str| {
        let (bytes, w) = Written::of(doc);
        assert!(w.warned(warning), "{:?}", w.warnings);
        let r = w.record("t");
        assert_eq!(ty(r), "TEXT");
        assert!(r.get("textPathStart").is_none() && r.get("vectorData").is_none());
        let (back, warnings) = super::import(&bytes, "Back").unwrap();
        assert!(text_of(&back, "t").path.is_none());
        assert!(!warnings.iter().any(|x| x.contains("text path")), "{warnings:?}");
    };
    plain(&doc_of(&[("t", "al\npha", "M0,0 L100,0", 0.5, false, 100.0, 0.0)]), W1);
    plain(&doc_of(&[("t", "alpha", "M0,5 L100,5", 0.5, false, 100.0, 0.0)]), W2);
    let mut empty = doc_of(&[("t", "alpha", "", 0.5, false, 100.0, 20.0)]);
    let id = empty.nodes().find(|n| n.name == "t").unwrap().id;
    empty.node_mut(id).unwrap().kind.text_mut().unwrap().path = Some(Box::new(TextPath { d: String::new(), start: 0.5, flip: false }));
    plain(&empty, W2);

    // Vertical alignment: written as top, still on its path.
    let mut middle = doc_of(&[("t", "alpha", "M0,0 L100,0", 0.5, false, 100.0, 0.0)]);
    let id = middle.nodes().find(|n| n.name == "t").unwrap().id;
    middle.node_mut(id).unwrap().kind.text_mut().unwrap().align_v = TextAlignV::Middle;
    let (bytes, w) = Written::of(&middle);
    assert!(!w.warnings.iter().any(|x| x.contains("text on a path")), "{:?}", w.warnings);
    assert_eq!(w.record("t").get("textAlignVertical").and_then(Kv::str), Some("TOP"));
    let (back, _) = super::import(&bytes, "Back").unwrap();
    assert!(text_of(&back, "t").path.is_some());

    // Several parts: the first.
    let (bytes, w) = Written::of(&doc_of(&[("t", "alpha", "M0,0 L100,0 M0,50 L100,50", 0.5, false, 100.0, 50.0)]));
    assert!(w.warned(W3), "{:?}", w.warnings);
    let (back, _) = super::import(&bytes, "Back").unwrap();
    let path = text_of(&back, "t").path.as_deref().unwrap();
    same_path(&path.d, "M0,0 L100,0");
    assert!((path.start - 0.5).abs() < 1e-6);

    // A zero-length segment is dropped.
    let (bytes, w) = Written::of(&doc_of(&[("t", "alpha", "M0,0 L0,0 L100,0", 0.5, false, 100.0, 0.0)]));
    assert!(!w.warnings.iter().any(|x| x.contains("text on a path")), "{:?}", w.warnings);
    let (back, _) = super::import(&bytes, "Back").unwrap();
    same_path(&text_of(&back, "t").path.as_deref().unwrap().d, "M0,0 L100,0");

    // A closed path of one segment.
    let (bytes, w) = Written::of(&doc_of(&[("t", "alpha", "M0,0 C100,-100 -100,-100 0,0 Z", 0.5, false, 100.0, 100.0)]));
    assert!(!w.warnings.iter().any(|x| x.contains("text on a path")), "{:?}", w.warnings);
    let (back, _) = super::import(&bytes, "Back").unwrap();
    let els = elements(&text_of(&back, "t").path.as_deref().unwrap().d);
    assert_eq!(els.first().map(|e| e.1[0]), Some((0.0, 0.0)));
    let last = els.iter().rev().find(|e| e.0 != 'Z').unwrap();
    assert_eq!(last.1.last().copied(), Some((0.0, 0.0)));
}

#[test]
fn an_instance_layer_on_a_path_survives_a_fig_export() {
    let (doc, _) = super::import(&canvas_of(scaled_instance_message()), "Scaled path").unwrap();
    let (bytes, warnings) = super::export(&doc, None).unwrap();
    assert!(!warnings.iter().any(|w| w.contains("text on a path")), "{warnings:?}");
    let (back, _) = super::import(&bytes, "Back").unwrap();
    let path_of = |doc: &Document, parent: &str| {
        let parent = doc.nodes().find(|n| n.name == parent).unwrap();
        let node = doc.node(parent.children[0]).unwrap();
        let NodeKind::Text(text) = &node.kind else { panic!("expected text") };
        text.path.as_deref().map(|p| p.d.clone())
    };
    assert_eq!(path_of(&back, "Main").as_deref(), Some("M0,0 L100,0"));
    assert_eq!(path_of(&back, "Half").as_deref(), Some("M0,0 L50,0"));

    // A layer in an instance that isn't on a path where its main's layer is.
    let mut cleared = doc;
    let half: NodeId = cleared.nodes().find(|n| n.name == "Half").unwrap().children[0];
    cleared.node_mut(half).unwrap().kind.text_mut().unwrap().path = None;
    let (_, w) = Written::of(&cleared);
    assert!(w.warned(W4), "{:?}", w.warnings);
}

#[test]
fn the_schema_numbers_text_on_a_path() {
    let s = super::kiwi::Schema::parse(super::export::SCHEMA).unwrap();
    let def = |n: &str| s.defs.iter().find(|d| d.name == n).unwrap();
    assert_eq!(def("NodeType").fields.iter().find(|x| x.name == "TEXT_PATH").map(|x| x.value), Some(41));
    assert_eq!(def("NodeChange").fields.iter().find(|x| x.name == "textPathStart").map(|x| x.value), Some(432));
    let start = def("TextPathStart");
    assert!(matches!(start.kind, super::kiwi::Kind::Message));
    let fields: Vec<_> = start.fields.iter().map(|x| (x.name.as_str(), x.value)).collect();
    assert_eq!(fields, [("tValue", 1), ("forward", 2)]);
}
