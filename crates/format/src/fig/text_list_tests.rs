//! Numbered lists that start past 1 (#686): each text line says where its list starts counting,
//! less one (`listStartOffset`). Fixtures built in code with the parent module's helpers.

use figcraft_doc::{ListStyle, NodeKind, ParaList};

use super::*;

const LIST_SCHEMA: &str = "
enum NodeType NONE DOCUMENT CANVAS TEXT
enum LineType PLAIN ORDERED_LIST UNORDERED_LIST
struct GUID uint sessionID, uint localID
struct Vector float x, float y
struct Matrix float m00, float m01, float m02, float m10, float m11, float m12
message ParentIndex GUID guid, string position
message TextLineData LineType lineType, int indentationLevel, int listStartOffset, bool isFirstLineOfList
message TextData string characters, TextLineData[] lines
message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, Vector size, Matrix transform, float fontSize, TextData textData
message Message NodeChange[] nodeChanges
";

/// A list line: type, start offset, first of its list.
fn line(ty: &'static str, offset: i64, first: bool) -> Kv<'static> {
    let level = i64::from(ty != "PLAIN");
    o(vec![("lineType", e(ty)), ("indentationLevel", Kv::Int(level)), ("listStartOffset", Kv::Int(offset)), ("isFirstLineOfList", Kv::Bool(first))])
}

fn text_node(id: u64, name: &str, chars: &str, lines: Vec<Kv<'static>>) -> Kv<'static> {
    let pos = ["!", "\"", "#", "$", "%", "&"][id as usize % 6];
    node(
        id,
        Some((1, pos)),
        "TEXT",
        name,
        vec![
            ("size", xy(200.0, 60.0)),
            ("transform", at(0.0, 0.0)),
            ("fontSize", f(16.0)),
            ("textData", o(vec![("characters", s(chars)), ("lines", arr(lines))])),
        ],
    )
}

fn starts(doc: &figcraft_doc::Document, name: &str) -> Vec<(ListStyle, u32)> {
    let NodeKind::Text(t) = &named(doc, name).kind else { panic!("{name} is no text") };
    (0..t.paragraph_count()).map(|p| t.para_list(p)).map(|p: ParaList| (p.list, p.start_offset)).collect()
}

#[test]
fn numbered_lists_keep_their_start() {
    use ListStyle::{Bulleted as B, None as P, Numbered as N};
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        text_node(2, "From 1", "Start here", vec![line("ORDERED_LIST", 0, true)]),
        text_node(3, "From 2", "Next try this", vec![line("ORDERED_LIST", 1, true)]),
        text_node(4, "First only", "a\nb\nc", vec![line("ORDERED_LIST", 3, true), line("ORDERED_LIST", 0, false), line("ORDERED_LIST", 0, false)]),
        text_node(5, "Every line", "a\nb\nc", vec![line("ORDERED_LIST", 3, true), line("ORDERED_LIST", 3, false), line("ORDERED_LIST", 3, false)]),
        text_node(6, "Mixed", "x\na\nb", vec![line("PLAIN", 0, false), line("ORDERED_LIST", 4, true), line("UNORDERED_LIST", 2, true)]),
        text_node(7, "Junk", "a", vec![line("ORDERED_LIST", -5, true)]),
    ];
    let sc = Schema::parse(LIST_SCHEMA).unwrap();
    let (doc, warnings) = import(&canvas(&sc, &o(vec![("nodeChanges", arr(nodes))])), "x").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(starts(&doc, "From 1"), [(N, 0)]);
    assert_eq!(starts(&doc, "From 2"), [(N, 1)], "shows 2.");
    assert_eq!(starts(&doc, "First only"), [(N, 3), (N, 0), (N, 0)]);
    assert_eq!(starts(&doc, "Every line"), [(N, 3), (N, 3), (N, 3)]);
    assert_eq!(starts(&doc, "Mixed"), [(P, 0), (N, 4), (B, 0)], "bullets have no number, so no start");
    assert_eq!(starts(&doc, "Junk"), [(N, 0)], "a negative offset counts from 1");
}

#[test]
fn the_export_writes_where_a_list_starts() {
    let mut doc = figcraft_doc::Document::new("Lists");
    let page = doc.first_page().unwrap();
    use ListStyle::{Bulleted as B, None as P, Numbered as N};
    let mut td = figcraft_doc::TextData::new("two\nthree\nplain\nsix\ndot");
    let pl = |list, start_offset| ParaList { list, indent: 0, start_offset };
    td.paragraphs = vec![pl(N, 1), pl(N, 0), pl(P, 0), pl(N, 5), pl(B, 4)];
    doc.create(page, NodeKind::Text(td), "List", 0.0, 0.0, 200.0, 80.0).unwrap();
    let (bytes, _) = super::super::export(&doc, None).unwrap();
    let a = Archive::open(&bytes).unwrap();
    let (schema, message) = decode_canvas(&a.canvas).unwrap();
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let text = msg.get("nodeChanges").unwrap().list().iter().find(|n| n.get("name").and_then(Kv::str) == Some("List")).unwrap();
    let lines = text.get("textData").and_then(|d| d.get("lines")).map(Kv::list).unwrap();
    let offsets: Vec<f64> = lines.iter().map(|l| l.get("listStartOffset").and_then(Kv::f64).unwrap()).collect();
    // Every line of a numbered list carries its first item's start (a reader that takes it per
    // line or per list counts the same); bullets and plain lines carry none.
    assert_eq!(offsets, [1.0, 1.0, 0.0, 5.0, 0.0]);
    let (back, _) = import(&bytes, "x").unwrap();
    assert_eq!(starts(&back, "List"), [(N, 1), (N, 1), (P, 0), (N, 5), (B, 0)]);
}
