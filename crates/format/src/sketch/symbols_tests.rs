//! Sketch symbols → components and instances with overrides.

use figcraft_doc::{Document, Node, NodeKind};
use serde_json::{Value, json};

use super::fixture::*;
use super::{import, import_within_nodes};

const BLACK: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

/// One page with `layers`, one "Symbols" page with `masters`, and `document` changes.
fn open_doc(doc_json: Value, layers: Vec<Value>, masters: Vec<Value>) -> (Document, Vec<String>) {
    let z = sketch_zip("50.2", 103, doc_json, &[page("P", "Page", layers), page("S", "Symbols", masters)], &[]);
    import(&z, "f").unwrap()
}

/// [`open`] for a file saved by another Sketch.
fn open_version(app: &str, version: u64, layers: Vec<Value>, masters: Vec<Value>) -> (Document, Vec<String>) {
    let z = sketch_zip(app, version, document(&["P", "S"]), &[page("P", "Page", layers), page("S", "Symbols", masters)], &[]);
    import(&z, "f").unwrap()
}

fn open(layers: Vec<Value>, masters: Vec<Value>) -> (Document, Vec<String>) {
    let mut d = document(&["P", "S"]);
    d["do_objectID"] = json!("DOC");
    open_doc(d, layers, masters)
}

fn named<'d>(doc: &'d Document, name: &str) -> Vec<&'d Node> {
    doc.nodes().filter(|n| n.name == name).collect()
}

fn inside<'d>(doc: &'d Document, root: &Node, name: &str) -> &'d Node {
    doc.subtree(root.id).into_iter().filter_map(|id| doc.node(id)).find(|n| n.name == name).unwrap_or_else(|| panic!("no {name}"))
}

fn chars(n: &Node) -> &str {
    match &n.kind {
        NodeKind::Text(td) => &td.characters,
        other => panic!("{other:?}"),
    }
}

fn instance_of<'d>(doc: &'d Document, name: &str) -> (&'d Node, &'d Node) {
    let inst = named(doc, name).into_iter().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap_or_else(|| panic!("no instance {name}"));
    let NodeKind::Instance { component, .. } = inst.kind else { panic!("not an instance") };
    (inst, doc.node(component).unwrap())
}

fn button() -> Value {
    master("M1", "S1", "Button", 100.0, 40.0, vec![text("Label", 10.0, 10.0, 80.0, 20.0, "Go", &[(0, 2, "Helvetica", 14.0, BLACK)])])
}

#[test]
fn master_is_component_instance_links() {
    let (doc, _) = open(vec![instance("I1", "S1", 0.0, 0.0, 100.0, 40.0, &[])], vec![button()]);
    let (inst, main) = instance_of(&doc, "I1");
    assert!(matches!(main.kind, NodeKind::Component { .. }));
    assert_eq!(main.name, "Button");
    assert_eq!(chars(inside(&doc, inst, "Label")), "Go");
}

#[test]
fn string_override() {
    let (doc, _) = open(vec![instance("I1", "S1", 0.0, 0.0, 100.0, 40.0, &[("Label_stringValue", json!("OK"))])], vec![button()]);
    let (inst, main) = instance_of(&doc, "I1");
    assert_eq!(chars(inside(&doc, inst, "Label")), "OK");
    assert_eq!(chars(inside(&doc, main, "Label")), "Go");
}

#[test]
fn nested_symbol_swap_and_hide() {
    let icon_a = master("MA", "SA", "Icon A", 20.0, 20.0, vec![rect("Dot A", 0.0, 0.0, 20.0, 20.0)]);
    let icon_b = master("MB", "SB", "Icon B", 20.0, 20.0, vec![rect("Dot B", 0.0, 0.0, 20.0, 20.0)]);
    let card = master("MC", "SC", "Card", 100.0, 40.0, vec![instance("N", "SA", 0.0, 0.0, 20.0, 20.0, &[])]);
    let swapped = instance("Swapped", "SC", 0.0, 0.0, 100.0, 40.0, &[("N_symbolID", json!("SB"))]);
    let hidden = instance("Hidden", "SC", 0.0, 100.0, 100.0, 40.0, &[("N_symbolID", json!(""))]);
    let (doc, _) = open(vec![swapped, hidden], vec![icon_a, icon_b, card]);
    let (sw, _) = instance_of(&doc, "Swapped");
    assert!(doc.subtree(sw.id).into_iter().filter_map(|i| doc.node(i)).any(|n| n.name == "Dot B"));
    assert!(!doc.subtree(sw.id).into_iter().filter_map(|i| doc.node(i)).any(|n| n.name == "Dot A"));
    let (hd, _) = instance_of(&doc, "Hidden");
    let nested = doc.node(hd.children[0]).unwrap();
    assert!(!nested.visible);
}

#[test]
fn nested_path_override() {
    let inner = master("MB", "SB", "Badge", 50.0, 20.0, vec![text("Count", 0.0, 0.0, 50.0, 20.0, "1", &[(0, 1, "Helvetica", 12.0, BLACK)])]);
    let outer = master("MA", "SA", "Row", 100.0, 20.0, vec![instance("N", "SB", 50.0, 0.0, 50.0, 20.0, &[])]);
    let (doc, _) = open(vec![instance("I1", "SA", 0.0, 0.0, 100.0, 20.0, &[("N/Count_stringValue", json!("42"))])], vec![inner, outer]);
    let (inst, _) = instance_of(&doc, "I1");
    assert_eq!(chars(inside(&doc, inst, "Count")), "42");
}

#[test]
fn image_override() {
    let mut bitmap = rect("Photo", 0.0, 0.0, 40.0, 40.0);
    bitmap["_class"] = json!("bitmap");
    bitmap["image"] = json!({"_class": "MSJSONFileReference", "_ref": "images/a.png"});
    let m = master("M", "S", "Avatar", 40.0, 40.0, vec![bitmap]);
    let png = {
        let mut out = Vec::new();
        image::RgbaImage::new(1, 1).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let mut png2 = Vec::new();
    image::RgbaImage::new(2, 1).write_to(&mut std::io::Cursor::new(&mut png2), image::ImageFormat::Png).unwrap();
    let inst = instance("I1", "S", 0.0, 0.0, 40.0, 40.0, &[("Photo_image", json!({"_class": "MSJSONFileReference", "_ref": "images/b.png"}))]);
    let z = sketch_zip(
        "50.2",
        103,
        document(&["P", "S"]),
        &[page("P", "Page", vec![inst]), page("S", "Symbols", vec![m])],
        &[("images/a.png", &png), ("images/b.png", &png2)],
    );
    let (doc, _) = import(&z, "f").unwrap();
    assert_eq!(doc.images.len(), 2);
}

#[test]
fn foreign_symbols_are_components() {
    let mut d = document(&["P", "S"]);
    d["foreignSymbols"] = json!([{"_class": "MSImmutableForeignSymbol", "sourceLibraryName": "Kit", "symbolMaster": button()}]);
    let (doc, warnings) = open_doc(d, vec![instance("I1", "S1", 0.0, 0.0, 100.0, 40.0, &[])], vec![]);
    let (_, main) = instance_of(&doc, "I1");
    assert_eq!(main.name, "Button");
    let page = doc.pages.iter().copied().find(|p| doc.is_ancestor(*p, main.id)).unwrap();
    assert_eq!(doc.node(page).unwrap().name, "Library symbols");
    assert!(warnings.iter().any(|w| w.contains("Kit")), "{warnings:?}");
}

#[test]
fn missing_master_warns() {
    let (doc, warnings) = open(vec![instance("Lost", "NOPE", 0.0, 0.0, 30.0, 30.0, &[])], vec![]);
    let n = named(&doc, "Lost")[0];
    assert!(matches!(n.kind, NodeKind::Frame));
    assert_eq!((n.width, n.height), (30.0, 30.0));
    assert!(warnings.iter().any(|w| w.contains("symbol")), "{warnings:?}");
}

#[test]
fn self_containing_master_stops() {
    let a = master("MA", "SA", "Loop A", 50.0, 50.0, vec![instance("Self", "SA", 0.0, 0.0, 10.0, 10.0, &[])]);
    let b = master("MB", "SB", "Loop B", 50.0, 50.0, vec![instance("ToC", "SC", 0.0, 0.0, 10.0, 10.0, &[])]);
    let c = master("MC", "SC", "Loop C", 50.0, 50.0, vec![instance("ToB", "SB", 0.0, 0.0, 10.0, 10.0, &[])]);
    let (_, warnings) = open(vec![instance("I1", "SA", 0.0, 0.0, 50.0, 50.0, &[]), instance("I2", "SB", 100.0, 0.0, 50.0, 50.0, &[])], vec![a, b, c]);
    assert!(warnings.iter().any(|w| w.contains("contains itself")), "{warnings:?}");
}

#[test]
fn instance_explosion_is_bounded() {
    let big = master("M", "S", "Big", 100.0, 100.0, (0..1000).map(|i| rect(&format!("R{i}"), 0.0, 0.0, 1.0, 1.0)).collect());
    let many: Vec<Value> = (0..1000).map(|i| instance(&format!("I{i}"), "S", 0.0, 0.0, 100.0, 100.0, &[])).collect();
    let z = sketch_zip("50.2", 103, document(&["P", "S"]), &[page("P", "Page", many), page("S", "Symbols", vec![big])], &[]);
    let (doc, warnings) = import_within_nodes(&z, "f", 20_000).unwrap();
    assert!(doc.nodes().count() <= 20_100);
    assert!(warnings.iter().any(|w| w.contains("more than")), "{warnings:?}");
}

#[test]
fn resized_instance_follows_constraints() {
    // Pinned right with a fixed width (bits 1 and 2 cleared); a Sketch 64 file.
    let pinned = with(rect("Tag", 80.0, 0.0, 20.0, 40.0), "resizingConstraint", json!(60));
    let m = master("M", "S", "Bar", 100.0, 40.0, vec![pinned]);
    let (doc, _) = open_version("64", 125, vec![instance("Wide", "S", 0.0, 0.0, 200.0, 40.0, &[])], vec![m]);
    let (inst, _) = instance_of(&doc, "Wide");
    let tag = inside(&doc, inst, "Tag");
    let b = doc.world_bounds(tag.id).unwrap();
    assert!((b.x0 - 180.0).abs() < 1e-6 && (b.width() - 20.0).abs() < 1e-6, "{b:?}");
}

/// Overrides pointing at junk (a shared style that isn't an object, text runs that aren't) never
/// panic.
#[test]
fn override_application_survives_junk() {
    let mut d = document(&["P", "S"]);
    d["layerStyles"]["objects"] = json!([{"_class": "sharedStyle", "do_objectID": "S1", "name": "Junk", "value": 5}]);
    d["layerTextStyles"]["objects"] =
        json!([{"_class": "sharedStyle", "do_objectID": "T1", "name": "Junk text", "value": {"textStyle": {"encodedAttributes": {}}}}]);
    let mut t = text("Label", 0.0, 0.0, 50.0, 20.0, "x", &[(0, 1, "Helvetica", 9.0, BLACK)]);
    t["attributedString"]["attributes"] = json!([5, "x", null]);
    let m = master("M", "S", "Junk master", 50.0, 20.0, vec![rect("C", 0.0, 0.0, 10.0, 10.0), t]);
    let inst = instance(
        "I1",
        "S",
        0.0,
        0.0,
        50.0,
        20.0,
        &[("C_layerStyle", json!("S1")), ("Label_textStyle", json!("T1")), ("Label_stringValue", json!("y"))],
    );
    let z = sketch_zip("50.2", 103, d, &[page("P", "Page", vec![inst]), page("S", "Symbols", vec![m])], &[]);
    assert!(import(&z, "f").is_ok());
}

/// A huge master expanded by many instances stops at the file's work budget.
#[test]
fn expansion_work_is_bounded() {
    let long = "x".repeat(200_000);
    let m = master("M", "S", "Heavy", 50.0, 20.0, vec![text("Long", 0.0, 0.0, 50.0, 20.0, &long, &[(0, 200_000, "Helvetica", 9.0, BLACK)])]);
    let many: Vec<Value> = (0..3000).map(|i| instance(&format!("I{i}"), "S", 0.0, 0.0, 50.0, 20.0, &[])).collect();
    let z = sketch_zip("50.2", 103, document(&["P", "S"]), &[page("P", "Page", many), page("S", "Symbols", vec![m])], &[]);
    let (_, warnings) = import(&z, "f").unwrap();
    assert!(warnings.iter().any(|w| w.contains("expand")), "{warnings:?}");
}

/// Outer overrides fanned out to many nested instances stop at the work budget.
#[test]
fn override_fan_out_is_bounded() {
    let inner = master("MB", "SB", "Cell", 10.0, 10.0, vec![text("T", 0.0, 0.0, 10.0, 10.0, "1", &[(0, 1, "Helvetica", 9.0, BLACK)])]);
    let cells: Vec<Value> = (0..3000).map(|_| instance("X", "SB", 0.0, 0.0, 10.0, 10.0, &[])).collect();
    let grid = master("MA", "SA", "Grid", 100.0, 100.0, cells);
    let overrides: Vec<(String, Value)> = (0..3000).map(|i| (format!("X/N{i}_stringValue"), json!("v"))).collect();
    let refs: Vec<(&str, Value)> = overrides.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let z = sketch_zip(
        "50.2",
        103,
        document(&["P", "S"]),
        &[page("P", "Page", vec![instance("I", "SA", 0.0, 0.0, 100.0, 100.0, &refs)]), page("S", "Symbols", vec![inner, grid])],
        &[],
    );
    let (_, warnings) = import(&z, "f").unwrap();
    assert!(warnings.iter().any(|w| w.contains("expand")), "{warnings:?}");
}

/// Sketch 50 stores "not scaled" as `scale: 0`.
#[test]
fn scale_zero_is_unscaled() {
    let mut i = instance("I1", "S1", 0.0, 0.0, 100.0, 40.0, &[]);
    i["scale"] = json!(0);
    let (_, warnings) = open(vec![i], vec![button()]);
    assert!(!warnings.iter().any(|w| w.contains("scaled")), "{warnings:?}");
}

/// A master's background shows in its instances only when Sketch says so.
#[test]
fn master_background_in_instances_only_when_included() {
    let bg = |id: &str, sid: &str, include: bool| {
        let mut m = master(id, sid, id, 20.0, 20.0, vec![]);
        m["hasBackgroundColor"] = json!(true);
        m["backgroundColor"] = json!({"_class": "color", "red": 0, "green": 0, "blue": 0, "alpha": 1});
        m["includeBackgroundColorInInstance"] = json!(include);
        m
    };
    let (doc, _) = open(
        vec![instance("Hidden", "SH", 0.0, 0.0, 20.0, 20.0, &[]), instance("Shown", "SS", 50.0, 0.0, 20.0, 20.0, &[])],
        vec![bg("MH", "SH", false), bg("MS", "SS", true)],
    );
    let (hidden, main) = instance_of(&doc, "Hidden");
    assert!(hidden.fills.is_empty(), "{:?}", hidden.fills);
    assert_eq!(main.fills.len(), 1, "the master shows its background");
    assert_eq!(instance_of(&doc, "Shown").0.fills.len(), 1);
}

/// How Sketch lays out a resized instance's layers, measured on its renders: a master of 200×100
/// with a 100×40 rectangle at (20, 30), shown at 320×60.
#[test]
fn resized_instances_lay_out_like_sketch() {
    let case = |mask: i64, version: u64| {
        let r = with(rect("R", 20.0, 30.0, 100.0, 40.0), "resizingConstraint", json!(mask));
        let m = master("M", "S", "Box", 200.0, 100.0, vec![r]);
        let (doc, _) = open_version("64", version, vec![instance("I", "S", 0.0, 0.0, 320.0, 60.0, &[])], vec![m]);
        let (inst, _) = instance_of(&doc, "I");
        let b = doc.world_bounds(inside(&doc, inst, "R").id).unwrap();
        (b.x0, b.y0, b.width(), b.height())
    };
    let near = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
        (a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5 && (a.2 - b.2).abs() < 0.5 && (a.3 - b.3).abs() < 0.5
    };
    // Sketch 52 and newer (format 110+).
    for (mask, want) in [
        (63, (32.0, 18.0, 160.0, 24.0)),
        (59, (20.0, 18.0, 166.67, 24.0)),
        (62, (40.0, 18.0, 200.0, 24.0)),
        (61, (62.0, 18.0, 100.0, 24.0)),
        (58, (20.0, 18.0, 220.0, 24.0)),
        (57, (20.0, 18.0, 100.0, 24.0)),
        (60, (140.0, 18.0, 100.0, 24.0)),
        (54, (40.0, 12.86, 200.0, 17.14)),
        (0, (32.0, 18.0, 160.0, 24.0)),
        (47, (32.0, 10.0, 160.0, 40.0)),
    ] {
        let got = case(mask, 125);
        assert!(near(got, want), "mask {mask}: {got:?} vs {want:?}");
    }
    // Older files: everything scales.
    for mask in [59, 62, 61, 0] {
        let got = case(mask, 103);
        assert!(near(got, (32.0, 18.0, 160.0, 24.0)), "legacy mask {mask}: {got:?}");
    }
}

/// A nested swap to a symbol the file doesn't have keeps the original, as Sketch shows it.
#[test]
fn swap_to_a_missing_symbol_keeps_the_original() {
    let icon = master("MA", "SA", "Icon A", 20.0, 20.0, vec![rect("Dot A", 0.0, 0.0, 20.0, 20.0)]);
    let card = master("MC", "SC", "Card", 100.0, 40.0, vec![instance("N", "SA", 0.0, 0.0, 20.0, 20.0, &[])]);
    let (doc, warnings) = open(vec![instance("I", "SC", 0.0, 0.0, 100.0, 40.0, &[("N_symbolID", json!("GONE"))])], vec![icon, card]);
    let (inst, _) = instance_of(&doc, "I");
    assert!(doc.subtree(inst.id).into_iter().filter_map(|i| doc.node(i)).any(|n| n.name == "Dot A"));
    assert!(!warnings.iter().any(|w| w.contains("empty frames")), "{warnings:?}");
}
