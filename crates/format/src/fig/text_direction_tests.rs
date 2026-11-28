//! A paragraph's forced base direction (#1830): each text line's `sourceDirectionality` (AUTO,
//! LTR or RTL) comes in as the paragraph's direction and goes back out. Fixtures built in code with
//! the parent module's helpers.

use figcraft_doc::{NodeKind, TextDirection};

use super::*;

const DIRECTION_SCHEMA: &str = "
enum NodeType NONE DOCUMENT CANVAS TEXT
enum LineType PLAIN ORDERED_LIST UNORDERED_LIST
enum SourceDirectionality AUTO LTR RTL
struct GUID uint sessionID, uint localID
struct Vector float x, float y
struct Matrix float m00, float m01, float m02, float m10, float m11, float m12
message ParentIndex GUID guid, string position
message TextLineData LineType lineType, int indentationLevel, SourceDirectionality sourceDirectionality, int listStartOffset, bool isFirstLineOfList
message TextData string characters, TextLineData[] lines
message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, Vector size, Matrix transform, float fontSize, TextData textData
message Message NodeChange[] nodeChanges
";

/// A plain line with a base direction.
fn line(direction: &'static str) -> Kv<'static> {
    o(vec![
        ("lineType", e("PLAIN")),
        ("indentationLevel", Kv::Int(0)),
        ("sourceDirectionality", e(direction)),
        ("listStartOffset", Kv::Int(0)),
        ("isFirstLineOfList", Kv::Bool(false)),
    ])
}

fn text_node(id: u64, name: &str, chars: &str, lines: Vec<Kv<'static>>) -> Kv<'static> {
    let pos = ["!", "\"", "#", "$"][id as usize % 4];
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

fn directions(doc: &figcraft_doc::Document, name: &str) -> Vec<TextDirection> {
    let NodeKind::Text(t) = &named(doc, name).kind else { panic!("{name} is no text") };
    (0..t.paragraph_count()).map(|p| t.para_list(p).direction).collect()
}

#[test]
fn a_lines_source_directionality_is_its_paragraphs_direction_both_ways() {
    use TextDirection::{Auto as A, Ltr as L, Rtl as R};
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        text_node(2, "Detected", "abc\ndef", vec![line("AUTO"), line("AUTO")]),
        text_node(3, "Forced", "abc\ndef\nghi", vec![line("RTL"), line("AUTO"), line("LTR")]),
    ];
    let sc = Schema::parse(DIRECTION_SCHEMA).unwrap();
    let (doc, warnings) = import(&canvas(&sc, &o(vec![("nodeChanges", arr(nodes))])), "x").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(directions(&doc, "Detected"), [A, A]);
    let NodeKind::Text(t) = &named(&doc, "Detected").kind else { panic!() };
    assert!(t.paragraphs.is_empty(), "AUTO throughout keeps the paragraphs folded");
    assert_eq!(directions(&doc, "Forced"), [R, A, L]);
    // Out again: every line says its direction, and a reader gets the same paragraphs back.
    let (bytes, _) = super::super::export(&doc, None).unwrap();
    let a = Archive::open(&bytes).unwrap();
    let (schema, message) = decode_canvas(&a.canvas).unwrap();
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let text = msg.get("nodeChanges").unwrap().list().iter().find(|n| n.get("name").and_then(Kv::str) == Some("Forced")).unwrap();
    let lines = text.get("textData").and_then(|d| d.get("lines")).map(Kv::list).unwrap();
    let out: Vec<&str> = lines.iter().map(|l| l.get("sourceDirectionality").and_then(Kv::str).unwrap()).collect();
    assert_eq!(out, ["RTL", "AUTO", "LTR"]);
    let (back, _) = import(&bytes, "x").unwrap();
    assert_eq!(directions(&back, "Forced"), [R, A, L]);
    assert_eq!(directions(&back, "Detected"), [A, A]);
}
