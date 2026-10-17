//! How auto layout frames and text size on import (hug or fixed), on encoded `.fig` files built in
//! code and REST file JSON: the sizing a file records, an auto layout frame that records none, and
//! text that hugs by its resize mode.

use figcraft_doc::{Document, Sizing};
use serde_json::{Value, json};

use super::kiwi::{Kv, Schema};

const SCHEMA: &str = "
enum NodeType NONE DOCUMENT CANVAS FRAME TEXT SYMBOL INSTANCE
enum StackMode NONE HORIZONTAL VERTICAL
enum StackSize FIXED RESIZE_TO_FIT RESIZE_TO_FIT_WITH_IMPLICIT_SIZE
enum TextAutoResize NONE WIDTH_AND_HEIGHT HEIGHT
struct GUID uint sessionID, uint localID
struct Vector float x, float y
struct Matrix float m00, float m01, float m02, float m10, float m11, float m12
message ParentIndex GUID guid, string position
message TextData string characters
message SymbolData GUID symbolID
message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, Vector size, Matrix transform, StackMode stackMode, StackSize stackPrimarySizing, StackSize stackCounterSizing, TextAutoResize textAutoResize, float fontSize, TextData textData, SymbolData symbolData
message Message NodeChange[] nodeChanges
";

type Fields = Vec<(&'static str, Kv<'static>)>;

fn o(fields: Fields) -> Kv<'static> {
    Kv::Object(fields)
}

fn g(id: u64) -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(id))])
}

fn xy(x: f64, y: f64) -> Kv<'static> {
    o(vec![("x", Kv::Float(x as f32)), ("y", Kv::Float(y as f32))])
}

fn at(x: f64, y: f64) -> Kv<'static> {
    let f = |v: f64| Kv::Float(v as f32);
    o(vec![("m00", f(1.0)), ("m01", f(0.0)), ("m02", f(x)), ("m10", f(0.0)), ("m11", f(1.0)), ("m12", f(y))])
}

fn node(id: u64, parent: u64, pos: &str, ty: &'static str, name: &str, mut more: Fields) -> Kv<'static> {
    let mut v = vec![("guid", g(id)), ("parentIndex", o(vec![("guid", g(parent)), ("position", Kv::Str(pos.into()))]))];
    v.extend([("type", Kv::Enum(ty)), ("name", Kv::Str(name.into()))]);
    v.append(&mut more);
    o(v)
}

/// An auto layout frame's fields: its direction and the sizing it records (`None`: none).
fn stack(mode: &'static str, primary: Option<&'static str>, counter: Option<&'static str>) -> Fields {
    let mut f = vec![("stackMode", Kv::Enum(mode))];
    f.extend(primary.map(|p| ("stackPrimarySizing", Kv::Enum(p))));
    f.extend(counter.map(|c| ("stackCounterSizing", Kv::Enum(c))));
    f
}

/// A text layer "<name> text" in `parent`, with resize mode `resize` (`None`: none recorded).
fn label(id: u64, parent: u64, name: &str, resize: Option<&'static str>) -> Kv<'static> {
    let mut f = vec![("size", xy(40.0, 16.0)), ("transform", at(0.0, 0.0)), ("fontSize", Kv::Float(12.0))];
    f.push(("textData", o(vec![("characters", Kv::Str(name.into()))])));
    f.extend(resize.map(|r| ("textAutoResize", Kv::Enum(r))));
    node(id, parent, "!", "TEXT", &format!("{name} text"), f)
}

/// A component at `x` holding a text layer, with `fields` of its own.
fn component(id: u64, x: f64, name: &str, fields: Fields, resize: Option<&'static str>) -> Vec<Kv<'static>> {
    let mut f = vec![("size", xy(40.0, 16.0)), ("transform", at(x, 0.0))];
    f.extend(fields);
    vec![node(id, 1, &format!("{id}"), "SYMBOL", name, f), label(id + 1, id, name, resize)]
}

fn import(nodes: Vec<Kv<'static>>) -> Document {
    let mut all = vec![
        o(vec![
            ("guid", o(vec![("sessionID", Kv::Uint(0)), ("localID", Kv::Uint(0))])),
            ("type", Kv::Enum("DOCUMENT")),
            ("name", Kv::Str("D".into())),
        ]),
        o(vec![
            ("guid", g(1)),
            ("parentIndex", o(vec![("guid", o(vec![("sessionID", Kv::Uint(0)), ("localID", Kv::Uint(0))])), ("position", Kv::Str("!".into()))])),
            ("type", Kv::Enum("CANVAS")),
            ("name", Kv::Str("Page".into())),
        ]),
    ];
    all.extend(nodes);
    let schema = Schema::parse(SCHEMA).unwrap();
    let bytes = super::tests::canvas(&schema, &o(vec![("nodeChanges", Kv::Array(all))]));
    super::import(&bytes, "sizing").unwrap().0
}

fn sizing(doc: &Document, name: &str) -> (Sizing, Sizing) {
    let n = doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no layer {name}"));
    (n.child_layout.sizing_h, n.child_layout.sizing_v)
}

#[test]
fn auto_layout_frames_hug_as_recorded_and_by_default_on_their_primary_axis() {
    use Sizing::{Fixed, Hug};
    let fit = Some("RESIZE_TO_FIT");
    // (component, its auto layout fields, expected (horizontal, vertical), derivation)
    let rows: Vec<(&str, Fields, (Sizing, Sizing), &str)> = vec![
        ("Button", stack("HORIZONTAL", fit, fit), (Hug, Hug), "both axes hug, as recorded"),
        ("Implicit", stack("HORIZONTAL", Some("RESIZE_TO_FIT_WITH_IMPLICIT_SIZE"), fit), (Hug, Hug), "hugging with an implicit size"),
        ("Fixed", stack("HORIZONTAL", Some("FIXED"), Some("FIXED")), (Fixed, Fixed), "both axes fixed, as recorded"),
        ("Legacy", stack("HORIZONTAL", None, fit), (Hug, Hug), "no primary sizing: the primary axis sizes by its content"),
        ("Legacy column", stack("VERTICAL", None, None), (Fixed, Hug), "no sizing: the primary (vertical) axis hugs, the counter axis is fixed"),
    ];
    let mut nodes = Vec::new();
    for (k, (name, fields, _, _)) in rows.iter().enumerate() {
        nodes.extend(component(10 * (k as u64 + 1), 60.0 * k as f64, name, fields.clone(), Some("WIDTH_AND_HEIGHT")));
    }
    let doc = import(nodes);
    for (name, _, want, why) in rows {
        assert_eq!(sizing(&doc, name), want, "{name}: {why}");
    }
}

#[test]
fn text_hugs_by_its_resize_mode() {
    use Sizing::{Fixed, Hug};
    let fit = Some("RESIZE_TO_FIT");
    let mut nodes = Vec::new();
    for (k, (name, resize)) in
        [("Auto width", Some("WIDTH_AND_HEIGHT")), ("Auto height", Some("HEIGHT")), ("Fixed box", Some("NONE")), ("Unset", None)]
            .into_iter()
            .enumerate()
    {
        nodes.extend(component(10 * (k as u64 + 1), 60.0 * k as f64, name, stack("HORIZONTAL", fit, fit), resize));
    }
    // An instance of the auto width one: it hugs, and so does its copy of the text.
    nodes.push(node(
        90,
        1,
        "~",
        "INSTANCE",
        "Auto width instance",
        [vec![("size", xy(40.0, 16.0)), ("transform", at(0.0, 100.0)), ("symbolData", o(vec![("symbolID", g(10))]))], stack("HORIZONTAL", fit, fit)]
            .concat(),
    ));
    // One that records nothing of its auto layout: it has its main's.
    nodes.push(node(
        91,
        1,
        "~~",
        "INSTANCE",
        "Bare instance",
        vec![("size", xy(40.0, 16.0)), ("transform", at(0.0, 150.0)), ("symbolData", o(vec![("symbolID", g(10))]))],
    ));
    let doc = import(nodes);
    for (name, want) in
        [("Auto width text", (Hug, Hug)), ("Auto height text", (Fixed, Hug)), ("Fixed box text", (Fixed, Fixed)), ("Unset text", (Fixed, Fixed))]
    {
        assert_eq!(sizing(&doc, name), want, "{name}");
    }
    let inst = doc.nodes().find(|n| n.name == "Auto width instance").unwrap();
    assert_eq!((inst.child_layout.sizing_h, inst.child_layout.sizing_v), (Hug, Hug));
    let copy = inst.children.first().and_then(|c| doc.node(*c)).unwrap();
    assert_eq!((copy.child_layout.sizing_h, copy.child_layout.sizing_v), (Hug, Hug));
    let bare = doc.nodes().find(|n| n.name == "Bare instance").unwrap();
    assert_eq!(bare.layout.as_ref().map(|l| l.mode), Some(figcraft_doc::LayoutMode::Horizontal));
    assert_eq!((bare.child_layout.sizing_h, bare.child_layout.sizing_v), (Hug, Hug));
}

#[test]
fn rest_files_read_left_out_sizing_modes_as_hug_and_text_by_its_resize_mode() {
    use Sizing::{Fixed, Hug};
    let at = json!([[1, 0, 0], [0, 1, 0]]);
    let text = |id: &str, name: &str, more: Value| {
        let mut t = json!({"id": id, "name": name, "type": "TEXT", "relativeTransform": at, "size": {"x": 40, "y": 16}, "characters": name,
            "style": {"fontFamily": "Inter", "fontSize": 12, "textAutoResize": "WIDTH_AND_HEIGHT"}});
        t.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
        t
    };
    let frame = |id: &str, name: &str, more: Value, kids: Value| {
        let mut f = json!({"id": id, "name": name, "type": "COMPONENT", "relativeTransform": at, "size": {"x": 40, "y": 16},
            "layoutMode": "HORIZONTAL", "children": kids});
        f.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
        f
    };
    let file = json!({"name": "Sizing", "document": {"id": "0:0", "type": "DOCUMENT", "children": [{"id": "0:1", "name": "Page", "type": "CANVAS", "children": [
        frame("1:1", "Defaults", json!({}), json!([text("1:2", "Label", json!({}))])),
        frame("2:1", "Fixed", json!({"primaryAxisSizingMode": "FIXED", "counterAxisSizingMode": "FIXED"}),
            json!([text("2:2", "Fixed label", json!({"layoutSizingHorizontal": "FIXED"}))])),
    ]}]}});
    let (doc, _) = crate::rest::import(&file, &crate::rest::no_images).unwrap();
    // The REST API leaves out the default sizing modes (AUTO); explicit per-axis sizing wins.
    for (name, want) in [("Defaults", (Hug, Hug)), ("Label", (Hug, Hug)), ("Fixed", (Fixed, Fixed)), ("Fixed label", (Fixed, Hug))] {
        assert_eq!(sizing(&doc, name), want, "{name}");
    }
}
