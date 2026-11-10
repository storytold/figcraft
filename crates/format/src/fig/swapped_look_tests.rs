//! A nested instance swapped to another main takes that main's look, group by group (#2201).

use figcraft_doc::color::{Paint, PaintKind};
use figcraft_doc::{Document, NodeId};

use super::kiwi::{Kv, Schema};
use super::tests::{arr, g, o, s, solid};
use super::*;

type Fields = Vec<(&'static str, Kv<'static>)>;
type Rgb = (f32, f32, f32);

/// A node under `parent` (the shared `node` with a position from the id).
fn node(id: u64, parent: Option<u64>, ty: &'static str, name: &str, more: Fields) -> Kv<'static> {
    let pos = format!("{id:04}");
    super::tests::node(id, parent.map(|p| (p, pos.as_str())), ty, name, more)
}

fn symbol(main: u64, overrides: Vec<Kv<'static>>) -> Kv<'static> {
    o(vec![("symbolID", g(main)), ("symbolOverrides", arr(overrides))])
}

fn row(path: &[u64], more: Fields) -> Kv<'static> {
    let mut fields = vec![("guidPath", o(vec![("guids", arr(path.iter().map(|id| g(*id)).collect()))]))];
    fields.extend(more);
    o(fields)
}

fn paints((r, gr, b): Rgb) -> Kv<'static> {
    arr(vec![solid(f64::from(r), f64::from(gr), f64::from(b))])
}

/// `paints` whose one paint is bound to the variable `var`.
fn bound_paints((r, gr, b): Rgb, var: u64) -> Kv<'static> {
    let alias = o(vec![("value", o(vec![("alias", o(vec![("guid", g(var))]))])), ("dataType", Kv::Enum("ALIAS"))]);
    let mut paint = match solid(f64::from(r), f64::from(gr), f64::from(b)) {
        Kv::Object(fields) => fields,
        _ => Vec::new(),
    };
    paint.push(("colorVar", alias));
    arr(vec![Kv::Object(paint)])
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
const BOUND: Rgb = (0.3, 0.6, 0.3);

/// A variant main: both paints with their styles and a stroke weight, as the file stores them.
/// `fill` None: the fill links no style.
fn variant(id: u64, name: &str, styles: (Option<u64>, Rgb, u64, Rgb), weight: f32, child: u64) -> Vec<Kv<'static>> {
    let (fill, fill_rgb, stroke, stroke_rgb) = styles;
    let mut fields = vec![("fillPaints", paints(fill_rgb))];
    fields.extend(fill.map(|f| ("styleIdForFill", style_link(f))));
    fields.extend([("strokePaints", paints(stroke_rgb)), ("styleIdForStrokeFill", style_link(stroke)), ("strokeWeight", Kv::Float(weight))]);
    vec![node(id, Some(2), "SYMBOL", name, fields), node(child, Some(id), "RECTANGLE", &format!("{name} child"), vec![])]
}

/// What differs between the payloads: the badge's own fill (None: the copy of its main's), the
/// swap row's extra fields, the main the badge records and whether the Info fill links its style.
struct Case {
    badge_fill: Option<Fields>,
    swap_extra: Fields,
    recorded_main: u64,
    info_fill_style: bool,
}

impl Case {
    fn copy() -> Self {
        Self { badge_fill: None, swap_extra: vec![], recorded_main: 10, info_fill_style: true }
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
        node(
            7,
            Some(2),
            "VARIABLE_SET",
            "Theme",
            vec![("variableSetModes", Kv::Array(vec![o(vec![("id", g(90)), ("name", s("Light")), ("sortPosition", s("!"))])]))],
        ),
        node(
            8,
            Some(2),
            "VARIABLE",
            "badge-bg",
            vec![
                ("variableSetID", o(vec![("guid", g(7))])),
                ("variableResolvedType", Kv::Enum("COLOR")),
                (
                    "variableDataValues",
                    o(vec![(
                        "entries",
                        Kv::Array(vec![o(vec![
                            ("modeID", g(90)),
                            (
                                "variableData",
                                o(vec![
                                    (
                                        "value",
                                        o(vec![(
                                            "colorValue",
                                            o(vec![("r", Kv::Float(0.3)), ("g", Kv::Float(0.6)), ("b", Kv::Float(0.3)), ("a", Kv::Float(1.0))]),
                                        )]),
                                    ),
                                    ("dataType", Kv::Enum("COLOR")),
                                ]),
                            ),
                        ])]),
                    )]),
                ),
            ],
        ),
    ];
    let info_fill = case.info_fill_style.then_some(3);
    changes.extend(variant(10, "Type=Info", (info_fill, INFO_FILL, 4, INFO_STROKE), 1.0, 11));
    changes.extend(variant(20, "Type=Ideal", (Some(5), IDEAL_FILL, 6, IDEAL_STROKE), 2.0, 21));
    changes.push(node(30, Some(2), "SYMBOL", "Card", vec![]));
    let mut badge = vec![
        ("symbolData", symbol(case.recorded_main, vec![])),
        ("strokePaints", paints(INFO_STROKE)),
        ("styleIdForStrokeFill", style_link(4)),
        ("strokeWeight", Kv::Float(1.0)),
    ];
    match case.badge_fill {
        Some(fields) => badge.extend(fields),
        None => {
            badge.push(("fillPaints", paints(INFO_FILL)));
            badge.extend(info_fill.map(|f| ("styleIdForFill", style_link(f))));
        }
    }
    changes.push(node(31, Some(30), "INSTANCE", "Badge", badge));
    let mut swap = vec![("overriddenSymbolID", g(20))];
    swap.extend(case.swap_extra);
    changes.push(node(40, Some(2), "INSTANCE", "Swapped", vec![("symbolData", symbol(30, vec![row(&[31], swap)]))]));
    changes.push(node(41, Some(2), "INSTANCE", "Plain", vec![("symbolData", symbol(30, vec![]))]));
    o(vec![("nodeChanges", Kv::Array(changes))])
}

fn import_case(case: Case) -> (Document, Vec<String>) {
    // Our export never writes the older `inheritFillStyleID`; files from Figma can.
    let schema = export::SCHEMA.replace("message NodeChange GUID guid=1,", "message NodeChange GUID guid=1, GUID inheritFillStyleID=501,");
    let schema = Schema::parse(&schema).unwrap();
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
    let (doc, notes) = import_case(Case { badge_fill: Some(vec![("fillPaints", paints(RED))]), ..Case::copy() });
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
fn a_fill_row_without_a_style_link_beats_the_new_mains_style() {
    // The previous main's fill links no style, so a row that recolours the badge has none to
    // detach: its paints must still win over the style the new main links.
    let (doc, notes) = import_case(Case { info_fill_style: false, swap_extra: vec![("fillPaints", paints(BLUE))], ..Case::copy() });
    assert!(notes.is_empty(), "{notes:?}");
    let swapped = doc.node(badge(&doc, "Swapped")).unwrap();
    assert!(is_rgb(&swapped.fills, BLUE), "fill {:?}", swapped.fills);
    assert_eq!(swapped.styles.fill, None);
    assert!(is_rgb(&swapped.strokes, IDEAL_STROKE), "stroke {:?}", swapped.strokes);
    assert_eq!(style_name(&doc, &swapped.styles.stroke).as_deref(), Some("Ideal stroke"));
}

#[test]
fn a_rows_older_style_field_beats_the_new_mains_newer_one() {
    // The older `inheritFillStyleID` on the swap row is the topmost word on the fill's style: a
    // detach keeps the row's paints, a link takes that style.
    let detach = o(vec![("sessionID", Kv::Uint(4_294_967_295)), ("localID", Kv::Uint(4_294_967_295))]);
    let (doc, notes) = import_case(Case { swap_extra: vec![("fillPaints", paints(BLUE)), ("inheritFillStyleID", detach)], ..Case::copy() });
    assert!(notes.is_empty(), "{notes:?}");
    let swapped = doc.node(badge(&doc, "Swapped")).unwrap();
    assert!(is_rgb(&swapped.fills, BLUE), "fill {:?}", swapped.fills);
    assert_eq!(swapped.styles.fill, None);
    let (doc, _) = import_case(Case { swap_extra: vec![("inheritFillStyleID", g(3))], ..Case::copy() });
    let swapped = doc.node(badge(&doc, "Swapped")).unwrap();
    assert!(is_rgb(&swapped.fills, INFO_FILL), "fill {:?}", swapped.fills);
    assert_eq!(style_name(&doc, &swapped.styles.fill).as_deref(), Some("Info fill"));
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

#[test]
fn a_bound_fill_under_the_old_mains_style_link_survives_the_swap() {
    // The badge keeps the old main's fill style link but binds its own fill to a variable: `look`
    // draws that binding over the style, so the swap must not replace it with the new main's fill.
    let bound = vec![("fillPaints", bound_paints(BOUND, 8)), ("styleIdForFill", style_link(3))];
    let (doc, notes) = import_case(Case { badge_fill: Some(bound), ..Case::copy() });
    assert!(notes.is_empty(), "{notes:?}");
    let swapped = doc.node(badge(&doc, "Swapped")).unwrap();
    assert!(is_rgb(&swapped.fills, BOUND), "fill {:?}", swapped.fills);
    assert_eq!(style_name(&doc, &swapped.styles.fill).as_deref(), Some("Info fill"));
    assert!(is_rgb(&swapped.strokes, IDEAL_STROKE), "stroke {:?}", swapped.strokes);
    assert_eq!(swapped.stroke.weight, 2.0);
    let style = doc.styles.iter().find(|s| s.name == "Info fill").unwrap();
    // The binding moves onto the style, as for any styled layer (`rest::styles_take_bindings`).
    assert_eq!(style.bound.get("fills/0/color").map(String::as_str), Some("1:8"), "{:?}", style.bound);
}
