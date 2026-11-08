//! An instance scaled with the Scale tool stores `symbolData.uniformScaleFactor` (#2091 item 1):
//! its layers draw the main's lengths times the factor (composed through nested instances),
//! unless an override or derived row gives the value, which is at the instance's scale already.
//! Fixtures built in code with the parent module's helpers.

use figcraft_doc::{Document, EffectKind, LetterSpacing, LineHeight, Node, NodeKind};

use super::kiwi::{Kv, Schema};
use super::tests::{arr, at, canvas, e, f, g, node, o, s, xy};
use super::*;

const SCHEMA: &str = "
enum NodeType NONE DOCUMENT CANVAS FRAME ROUNDED_RECTANGLE TEXT SYMBOL INSTANCE
enum StackMode NONE HORIZONTAL VERTICAL
enum EffectType DROP_SHADOW
enum NumberUnits RAW PIXELS PERCENT
struct GUID uint sessionID, uint localID
struct Vector float x, float y
struct Matrix float m00, float m01, float m02, float m10, float m11, float m12
message Number float value, NumberUnits units
message ParentIndex GUID guid, string position
message FontName string family, string style, string postscript
message TextData string characters
message Effect EffectType type, float radius, float spread, Vector offset
message GUIDPath GUID[] guids
message SymbolData GUID symbolID, NodeChange[] symbolOverrides, float uniformScaleFactor
message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, Vector size, Matrix transform, float cornerRadius, float strokeWeight, float[] dashPattern, float fontSize, FontName fontName, TextData textData, SymbolData symbolData, StackMode stackMode, float stackSpacing, float stackHorizontalPadding, float stackVerticalPadding, float stackCounterSpacing, Effect[] effects, Number lineHeight, Number letterSpacing, NodeChange[] derivedSymbolData, GUIDPath guidPath
message Blob byte[] bytes
message Message NodeChange[] nodeChanges, Blob[] blobs
";

fn schema() -> Schema {
    Schema::parse(SCHEMA).unwrap()
}

fn import_nodes(nodes: Vec<Kv<'static>>) -> Document {
    import(&canvas(&schema(), &o(vec![("nodeChanges", arr(nodes))])), "x").unwrap().0
}

fn page() -> Vec<Kv<'static>> {
    vec![node(0, None, "DOCUMENT", "D", vec![]), node(1, Some((0, "!")), "CANVAS", "P", vec![])]
}

/// A copy of `n` with field `key` set to `value`.
fn set(n: Kv<'static>, key: &'static str, value: Kv<'static>) -> Kv<'static> {
    let Kv::Object(mut fields) = n else { panic!("not a node") };
    fields.retain(|(k, _)| *k != key);
    fields.push((key, value));
    Kv::Object(fields)
}

/// An instance of main `main` with `factor` (none: the field is missing) and extra fields.
fn instance(
    id: u64,
    parent: (u64, &str),
    name: &str,
    main: u64,
    size: (f64, f64),
    factor: Option<f64>,
    mut more: Vec<(&'static str, Kv<'static>)>,
) -> Kv<'static> {
    let mut symbol = vec![("symbolID", g(main))];
    if let Some(k) = factor {
        symbol.push(("uniformScaleFactor", f(k)));
    }
    let mut fields = vec![("size", xy(size.0, size.1)), ("transform", at(300.0, 0.0)), ("symbolData", o(symbol))];
    fields.append(&mut more);
    node(id, Some(parent), "INSTANCE", name, fields)
}

/// A 256 px component "Icon" holding a 200 px rectangle at (20, 40) with radius 64 and a stroke,
/// dashes and a shadow.
fn icon() -> Vec<Kv<'static>> {
    let shadow = o(vec![("type", e("DROP_SHADOW")), ("radius", f(20.0)), ("spread", f(4.0)), ("offset", xy(10.0, 6.0))]);
    vec![
        node(5, Some((1, "!")), "SYMBOL", "Icon", vec![("size", xy(256.0, 256.0)), ("transform", at(0.0, 0.0))]),
        node(
            6,
            Some((5, "!")),
            "ROUNDED_RECTANGLE",
            "Mark",
            vec![
                ("size", xy(200.0, 200.0)),
                ("transform", at(20.0, 40.0)),
                ("cornerRadius", f(64.0)),
                ("strokeWeight", f(8.0)),
                ("dashPattern", arr(vec![f(16.0), f(8.0)])),
                ("effects", arr(vec![shadow])),
            ],
        ),
    ]
}

fn find<'d>(doc: &'d Document, name: &str) -> &'d Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no layer {name}"))
}

/// The first child of the layer named `name`.
fn first_child<'d>(doc: &'d Document, name: &str) -> &'d Node {
    doc.node(find(doc, name).children[0]).unwrap()
}

fn mark(doc: &Document, name: &str) -> ([f64; 2], [f64; 2], [f64; 4]) {
    let m = first_child(doc, name);
    let t = m.transform.translation();
    ([m.width, m.height], [t.x, t.y], m.corners.radii)
}

#[test]
fn a_scaled_instance_scales_the_lengths_of_a_layer_it_gives_no_row_for() {
    let mut nodes = page();
    nodes.extend(icon());
    nodes.push(instance(9, (1, "\""), "Half", 5, (128.0, 128.0), Some(0.5), vec![]));
    let doc = import_nodes(nodes);
    let (size, place, radii) = mark(&doc, "Half");
    assert_eq!((size, place, radii), ([100.0, 100.0], [10.0, 20.0], [32.0; 4]));
    let m = first_child(&doc, "Half");
    assert_eq!(m.stroke.weight, 4.0);
    assert_eq!(m.stroke.dashes, [8.0, 4.0]);
    let EffectKind::DropShadow(sh) = &m.effects[0].kind else { panic!("not a drop shadow") };
    assert_eq!((sh.radius, sh.spread, sh.offset), (10.0, 2.0, (5.0, 3.0)));
    // The instance itself is at the size the file gives it.
    let inst = find(&doc, "Half");
    assert_eq!([inst.width, inst.height], [128.0, 128.0]);
    // The main is untouched.
    assert_eq!(first_child(&doc, "Icon").corners.radii, [64.0; 4]);
}

#[test]
fn a_value_a_derived_row_gives_is_not_scaled_twice() {
    let mut nodes = page();
    nodes.extend(icon());
    let row = o(vec![("guidPath", o(vec![("guids", arr(vec![g(6)]))])), ("size", xy(100.0, 100.0))]);
    nodes.push(instance(9, (1, "\""), "Half", 5, (128.0, 128.0), Some(0.5), vec![("derivedSymbolData", arr(vec![row]))]));
    let doc = import_nodes(nodes);
    let (size, place, radii) = mark(&doc, "Half");
    // The derived size is taken as written; the radius and place, which no row gives, scale.
    assert_eq!((size, place, radii), ([100.0, 100.0], [10.0, 20.0], [32.0; 4]));
}

#[test]
fn an_override_row_is_not_scaled_either() {
    let mut nodes = page();
    nodes.extend(icon());
    let row = o(vec![("guidPath", o(vec![("guids", arr(vec![g(6)]))])), ("cornerRadius", f(7.0)), ("strokeWeight", f(3.0))]);
    let symbol = o(vec![("symbolID", g(5)), ("uniformScaleFactor", f(0.5)), ("symbolOverrides", arr(vec![row]))]);
    nodes.push(node(9, Some((1, "\"")), "INSTANCE", "Half", vec![("size", xy(128.0, 128.0)), ("transform", at(300.0, 0.0)), ("symbolData", symbol)]));
    let doc = import_nodes(nodes);
    let m = first_child(&doc, "Half");
    assert_eq!(m.corners.radii, [7.0; 4]);
    assert_eq!(m.stroke.weight, 3.0);
    // What the override leaves alone still scales.
    assert_eq!(m.stroke.dashes, [8.0, 4.0]);
}

#[test]
fn a_scaled_auto_layout_instance_scales_its_gap_padding_and_text() {
    let mut nodes = page();
    nodes.push(node(
        5,
        Some((1, "!")),
        "SYMBOL",
        "Row",
        vec![
            ("size", xy(200.0, 60.0)),
            ("transform", at(0.0, 0.0)),
            ("stackMode", e("HORIZONTAL")),
            ("stackSpacing", f(20.0)),
            ("stackCounterSpacing", f(6.0)),
            ("stackHorizontalPadding", f(10.0)),
            ("stackVerticalPadding", f(10.0)),
        ],
    ));
    let font = o(vec![("family", s("Inter")), ("style", s("Regular")), ("postscript", s(""))]);
    let px = |v: f64| o(vec![("value", f(v)), ("units", e("PIXELS"))]);
    nodes.push(node(
        6,
        Some((5, "!")),
        "TEXT",
        "Label",
        vec![
            ("size", xy(40.0, 20.0)),
            ("transform", at(10.0, 10.0)),
            ("fontSize", f(16.0)),
            ("fontName", font),
            ("lineHeight", px(24.0)),
            ("letterSpacing", px(2.0)),
            ("textData", o(vec![("characters", s("Hi"))])),
        ],
    ));
    nodes.push(instance(9, (1, "\""), "RowHalf", 5, (100.0, 30.0), Some(0.5), vec![]));
    let doc = import_nodes(nodes);
    let row = find(&doc, "RowHalf");
    let layout = row.layout.as_ref().expect("the instance has its main's auto layout");
    assert_eq!((layout.gap, layout.cross_gap, layout.padding), (10.0, 3.0, [5.0; 4]));
    let NodeKind::Text(t) = &first_child(&doc, "RowHalf").kind else { panic!("not text") };
    assert_eq!(t.style.font_size, 8.0);
    assert_eq!(t.style.line_height, LineHeight::Pixels(12.0));
    assert_eq!(t.style.letter_spacing, LetterSpacing::Pixels(1.0));
    // The main keeps its own.
    let NodeKind::Text(main) = &first_child(&doc, "Row").kind else { panic!("not text") };
    assert_eq!(main.style.font_size, 16.0);
}

#[test]
fn nested_scaled_instances_compose_their_factors() {
    let mut nodes = page();
    // Inner: 200 px with a 200 px rectangle of radius 40.
    nodes.push(node(5, Some((1, "!")), "SYMBOL", "Inner", vec![("size", xy(200.0, 200.0)), ("transform", at(0.0, 0.0))]));
    nodes.push(node(
        6,
        Some((5, "!")),
        "ROUNDED_RECTANGLE",
        "Dot",
        vec![("size", xy(200.0, 200.0)), ("transform", at(0.0, 0.0)), ("cornerRadius", f(40.0))],
    ));
    // Outer: 200 px, holding Inner scaled to 100 px (factor 0.5) at (20, 20).
    nodes.push(node(7, Some((1, "\"")), "SYMBOL", "Outer", vec![("size", xy(200.0, 200.0)), ("transform", at(300.0, 0.0))]));
    nodes.push(set(instance(8, (7, "!"), "Kid", 5, (100.0, 100.0), Some(0.5), vec![]), "transform", at(20.0, 20.0)));
    // The outer one scaled to 100 px (factor 0.5).
    nodes.push(instance(9, (1, "#"), "Half", 7, (100.0, 100.0), Some(0.5), vec![]));
    let doc = import_nodes(nodes);
    let kid = first_child(&doc, "Half");
    // The nested instance itself is the outer factor times its stored size and place.
    assert_eq!(([kid.width, kid.height], [kid.transform.translation().x, kid.transform.translation().y]), ([50.0, 50.0], [10.0, 10.0]));
    // Its layers take both factors: 0.5 x 0.5 of the main's.
    let dot = doc.node(kid.children[0]).unwrap();
    assert_eq!(([dot.width, dot.height], dot.corners.radii), ([50.0, 50.0], [10.0; 4]));
}

#[test]
fn a_factor_of_one_or_none_changes_nothing() {
    let mut nodes = page();
    nodes.extend(icon());
    nodes.push(instance(9, (1, "\""), "One", 5, (256.0, 256.0), Some(1.0), vec![]));
    nodes.push(instance(10, (1, "#"), "None", 5, (256.0, 256.0), None, vec![]));
    let doc = import_nodes(nodes);
    for name in ["One", "None"] {
        let (size, place, radii) = mark(&doc, name);
        assert_eq!((size, place, radii), ([200.0, 200.0], [20.0, 40.0], [64.0; 4]), "{name}");
        assert_eq!(first_child(&doc, name).stroke.weight, 8.0, "{name}");
    }
}

#[test]
fn a_junk_factor_is_ignored() {
    // The file encoder refuses NaN and infinities; the range check turns those away too.
    for factor in [-0.5, 0.0, 1e30, 3e38, 1e-30] {
        let mut nodes = page();
        nodes.extend(icon());
        nodes.push(instance(9, (1, "\""), "Junk", 5, (256.0, 256.0), Some(factor), vec![]));
        let doc = import_nodes(nodes);
        let (size, place, radii) = mark(&doc, "Junk");
        assert_eq!((size, place, radii), ([200.0, 200.0], [20.0, 40.0], [64.0; 4]), "factor {factor}");
    }
}
