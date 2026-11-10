//! A nested instance swapped to another main takes that main's look, group by group (#2201).

use figcraft_doc::color::{Paint, PaintKind};
use figcraft_doc::{Document, NodeId};

use super::kiwi::{Kv, Schema};
use super::*;

type Fields = Vec<(&'static str, Kv<'static>)>;
type Rgb = (f32, f32, f32);

fn o(fields: Fields) -> Kv<'static> {
    Kv::Object(fields)
}

fn g(id: u64) -> Kv<'static> {
    o(vec![("sessionID", Kv::Uint(1)), ("localID", Kv::Uint(id))])
}

fn s(value: &str) -> Kv<'static> {
    Kv::Str(value.into())
}

fn node(id: u64, parent: Option<u64>, ty: &'static str, name: &str, more: Fields) -> Kv<'static> {
    let mut fields = vec![("guid", g(id)), ("phase", Kv::Enum("CREATED")), ("type", Kv::Enum(ty)), ("name", s(name))];
    if let Some(parent) = parent {
        fields.push(("parentIndex", o(vec![("guid", g(parent)), ("position", s(&format!("{id:04}")))])));
    }
    fields.extend(more);
    o(fields)
}

fn symbol(main: u64, overrides: Vec<Kv<'static>>) -> Kv<'static> {
    o(vec![("symbolID", g(main)), ("symbolOverrides", Kv::Array(overrides))])
}

fn row(path: &[u64], more: Fields) -> Kv<'static> {
    let mut fields = vec![("guidPath", o(vec![("guids", Kv::Array(path.iter().map(|id| g(*id)).collect()))]))];
    fields.extend(more);
    o(fields)
}

fn paints((r, gr, b): Rgb) -> Kv<'static> {
    Kv::Array(vec![o(vec![
        ("type", Kv::Enum("SOLID")),
        ("color", o(vec![("r", Kv::Float(r)), ("g", Kv::Float(gr)), ("b", Kv::Float(b)), ("a", Kv::Float(1.0))])),
        ("opacity", Kv::Float(1.0)),
        ("visible", Kv::Bool(true)),
    ])])
}

fn style_link(id: u64) -> Kv<'static> {
    o(vec![("guid", g(id))])
}

/// The link a layer writes when its fill is detached from a style.
fn no_style() -> Kv<'static> {
    o(vec![("guid", o(vec![("sessionID", Kv::Uint(4_294_967_295)), ("localID", Kv::Uint(4_294_967_295))]))])
}

const INFO_FILL: Rgb = (0.1, 0.2, 0.9);
const IDEAL_FILL: Rgb = (0.9, 0.2, 0.1);
const INFO_STROKE: Rgb = (0.1, 0.9, 0.2);
const IDEAL_STROKE: Rgb = (0.5, 0.5, 0.0);
const RED: Rgb = (1.0, 0.0, 0.0);
const BLUE: Rgb = (0.0, 0.0, 1.0);

/// A variant main: both paints with their styles and a stroke weight, as the file stores them.
fn variant(id: u64, name: &str, styles: (u64, Rgb, u64, Rgb), weight: f32, child: u64) -> Vec<Kv<'static>> {
    let (fill, fill_rgb, stroke, stroke_rgb) = styles;
    vec![
        node(
            id,
            Some(2),
            "SYMBOL",
            name,
            vec![
                ("fillPaints", paints(fill_rgb)),
                ("styleIdForFill", style_link(fill)),
                ("strokePaints", paints(stroke_rgb)),
                ("styleIdForStrokeFill", style_link(stroke)),
                ("strokeWeight", Kv::Float(weight)),
            ],
        ),
        node(child, Some(id), "RECTANGLE", &format!("{name} child"), vec![]),
    ]
}

/// What differs between the payloads: the badge's own fill (None: the copy of its main's), the
/// swap row's extra fields and the main the badge records.
struct Case {
    badge_fill: Option<Rgb>,
    swap_extra: Fields,
    recorded_main: u64,
}

impl Case {
    fn copy() -> Self {
        Self { badge_fill: None, swap_extra: vec![], recorded_main: 10 }
    }
}

fn message(case: Case) -> Kv<'static> {
    let style =
        |id: u64, name: &str, rgb: Rgb| node(id, Some(2), "RECTANGLE", name, vec![("styleType", Kv::Enum("FILL")), ("fillPaints", paints(rgb))]);
    let mut changes = vec![
        node(1, None, "DOCUMENT", "Document", vec![]),
        node(2, Some(1), "CANVAS", "Page", vec![]),
        style(3, "Info fill", INFO_FILL),
        style(4, "Info stroke", INFO_STROKE),
        style(5, "Ideal fill", IDEAL_FILL),
        style(6, "Ideal stroke", IDEAL_STROKE),
    ];
    changes.extend(variant(10, "Type=Info", (3, INFO_FILL, 4, INFO_STROKE), 1.0, 11));
    changes.extend(variant(20, "Type=Ideal", (5, IDEAL_FILL, 6, IDEAL_STROKE), 2.0, 21));
    changes.push(node(30, Some(2), "SYMBOL", "Card", vec![]));
    let mut badge = vec![
        ("symbolData", symbol(case.recorded_main, vec![])),
        ("strokePaints", paints(INFO_STROKE)),
        ("styleIdForStrokeFill", style_link(4)),
        ("strokeWeight", Kv::Float(1.0)),
    ];
    match case.badge_fill {
        Some(rgb) => badge.push(("fillPaints", paints(rgb))),
        None => badge.extend([("fillPaints", paints(INFO_FILL)), ("styleIdForFill", style_link(3))]),
    }
    changes.push(node(31, Some(30), "INSTANCE", "Badge", badge));
    let mut swap = vec![("overriddenSymbolID", g(20))];
    swap.extend(case.swap_extra);
    changes.push(node(40, Some(2), "INSTANCE", "Swapped", vec![("symbolData", symbol(30, vec![row(&[31], swap)]))]));
    changes.push(node(41, Some(2), "INSTANCE", "Plain", vec![("symbolData", symbol(30, vec![]))]));
    o(vec![("nodeChanges", Kv::Array(changes))])
}

fn import_case(case: Case) -> (Document, Vec<String>) {
    let schema = Schema::parse(export::SCHEMA).unwrap();
    import(&super::tests::canvas(&schema, &message(case)), "Swap.fig").unwrap()
}

fn named(doc: &Document, name: &str) -> NodeId {
    doc.nodes().find(|n| n.name == name).unwrap().id
}

fn child(doc: &Document, id: NodeId, index: usize) -> NodeId {
    doc.node(id).unwrap().children[index]
}

fn badge(doc: &Document, outer: &str) -> NodeId {
    child(doc, named(doc, outer), 0)
}

fn is_rgb(paints: &[Paint], want: Rgb) -> bool {
    let Some(PaintKind::Solid { color }) = paints.first().map(|p| &p.kind) else { return false };
    [color.r, color.g, color.b].iter().zip([want.0, want.1, want.2]).all(|(a, b)| (f64::from(*a) - f64::from(b)).abs() < 1e-3)
}

fn style_name(doc: &Document, id: &Option<String>) -> Option<String> {
    let id = id.as_ref()?;
    doc.styles.iter().find(|s| &s.id == id).map(|s| s.name.clone())
}

#[test]
fn a_swapped_nested_instance_takes_the_new_mains_look() {
    let (doc, notes) = import_case(Case::copy());
    assert!(notes.is_empty(), "{notes:?}");
    let swapped = doc.node(badge(&doc, "Swapped")).unwrap();
    assert!(is_rgb(&swapped.fills, IDEAL_FILL), "fill {:?}", swapped.fills);
    assert!(is_rgb(&swapped.strokes, IDEAL_STROKE), "stroke {:?}", swapped.strokes);
    assert_eq!(style_name(&doc, &swapped.styles.fill).as_deref(), Some("Ideal fill"));
    assert_eq!(style_name(&doc, &swapped.styles.stroke).as_deref(), Some("Ideal stroke"));
    assert_eq!(swapped.stroke.weight, 2.0);
    assert_eq!(doc.node(child(&doc, swapped.id, 0)).unwrap().name, "Type=Ideal child");
    assert!(!swapped.overrides.contains("fills") && !swapped.overrides.contains("strokes"), "{:?}", swapped.overrides);
    let plain = doc.node(badge(&doc, "Plain")).unwrap();
    assert!(is_rgb(&plain.fills, INFO_FILL));
    assert_eq!(style_name(&doc, &plain.styles.fill).as_deref(), Some("Info fill"));
    assert_eq!(style_name(&doc, &plain.styles.stroke).as_deref(), Some("Info stroke"));
    assert_eq!(plain.stroke.weight, 1.0);
}

#[test]
fn a_real_override_on_the_nested_instance_survives_the_swap() {
    let (doc, notes) = import_case(Case { badge_fill: Some(RED), ..Case::copy() });
    assert!(notes.is_empty(), "{notes:?}");
    let swapped = doc.node(badge(&doc, "Swapped")).unwrap();
    assert!(is_rgb(&swapped.fills, RED), "fill {:?}", swapped.fills);
    assert_eq!(swapped.styles.fill, None);
    assert!(is_rgb(&swapped.strokes, IDEAL_STROKE), "stroke {:?}", swapped.strokes);
    assert_eq!(style_name(&doc, &swapped.styles.stroke).as_deref(), Some("Ideal stroke"));
    let in_main = doc.node(child(&doc, named(&doc, "Card"), 0)).unwrap();
    assert!(is_rgb(&in_main.fills, RED));
}

#[test]
fn a_row_above_the_swap_beats_the_new_main() {
    let (doc, notes) = import_case(Case { swap_extra: vec![("fillPaints", paints(BLUE)), ("styleIdForFill", no_style())], ..Case::copy() });
    assert!(notes.is_empty(), "{notes:?}");
    let swapped = doc.node(badge(&doc, "Swapped")).unwrap();
    assert!(is_rgb(&swapped.fills, BLUE), "fill {:?}", swapped.fills);
    assert!(is_rgb(&swapped.strokes, IDEAL_STROKE), "stroke {:?}", swapped.strokes);
}

#[test]
fn a_swap_whose_previous_main_is_missing_takes_the_new_look() {
    let (doc, notes) = import_case(Case { recorded_main: 99, ..Case::copy() });
    // The unswapped badge in "Plain" still links the missing main and gets the usual notes.
    assert_eq!(notes.iter().filter(|n| n.contains("previous component isn't in the file")).count(), 1, "{notes:?}");
    let swapped = doc.node(badge(&doc, "Swapped")).unwrap();
    assert!(is_rgb(&swapped.fills, IDEAL_FILL), "fill {:?}", swapped.fills);
    assert!(is_rgb(&swapped.strokes, IDEAL_STROKE), "stroke {:?}", swapped.strokes);
}
