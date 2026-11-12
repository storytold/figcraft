//! Sketch symbols → components and instances with overrides.

use figcraft_doc::{Document, Node, NodeKind};
use serde_json::{Value, json};

use super::fixture::*;
use super::{import, import_within_nodes};

const BLACK: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

/// A 1×1 PNG whose red channel is `n`: images the importer can decode, one per `n`.
fn png(n: u8) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([n, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

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

/// A master's background shows in its instances only when the master includes it both in
/// instances and in exports (Sketch's own renders and canvas, on files built for it).
#[test]
fn master_background_in_instances_only_when_included() {
    let bg = |id: &str, sid: &str, instances: bool, exports: bool| {
        let mut m = master(id, sid, id, 20.0, 20.0, vec![]);
        m["hasBackgroundColor"] = json!(true);
        m["backgroundColor"] = json!({"_class": "color", "red": 0, "green": 0, "blue": 0, "alpha": 1});
        m["includeBackgroundColorInInstance"] = json!(instances);
        m["includeBackgroundColorInExport"] = json!(exports);
        m
    };
    let (doc, _) = open(
        vec![
            instance("Hidden", "SH", 0.0, 0.0, 20.0, 20.0, &[]),
            instance("InstancesOnly", "SI", 25.0, 0.0, 20.0, 20.0, &[]),
            instance("ExportsOnly", "SE", 50.0, 0.0, 20.0, 20.0, &[]),
            instance("Shown", "SS", 75.0, 0.0, 20.0, 20.0, &[]),
        ],
        vec![bg("MH", "SH", false, false), bg("MI", "SI", true, false), bg("ME", "SE", false, true), bg("MS", "SS", true, true)],
    );
    let (hidden, main) = instance_of(&doc, "Hidden");
    assert!(hidden.fills.is_empty(), "{:?}", hidden.fills);
    assert_eq!(main.fills.len(), 1, "the master shows its background");
    assert!(instance_of(&doc, "InstancesOnly").0.fills.is_empty());
    assert!(instance_of(&doc, "ExportsOnly").0.fills.is_empty());
    assert_eq!(instance_of(&doc, "Shown").0.fills.len(), 1);
}

/// Sketch 2025+: a symbol's (`groupBehavior` 0) or frame master's fills are its background, shown
/// in instances on the same terms.
#[test]
fn style_background_in_instances_only_when_included() {
    let m = |id: &str, sid: &str, behaviour: i64, shown: bool| {
        let mut m = master(id, sid, id, 20.0, 20.0, vec![]);
        m["groupBehavior"] = json!(behaviour);
        m["style"]["fills"] = json!([solid_fill(1.0, 1.0, 1.0, 1.0)]);
        m["includeBackgroundColorInInstance"] = json!(shown);
        m["includeBackgroundColorInExport"] = json!(shown);
        m
    };
    let (doc, _) = open_version(
        "2026.3.1",
        196,
        vec![
            instance("Symbol", "S0", 0.0, 0.0, 20.0, 20.0, &[]),
            instance("Frame", "S1", 25.0, 0.0, 20.0, 20.0, &[]),
            instance("Shown", "S2", 50.0, 0.0, 20.0, 20.0, &[]),
        ],
        vec![m("M0", "S0", 0, false), m("M1", "S1", 1, false), m("M2", "S2", 0, true)],
    );
    assert!(instance_of(&doc, "Symbol").0.fills.is_empty());
    assert!(instance_of(&doc, "Frame").0.fills.is_empty());
    assert_eq!(instance_of(&doc, "Shown").0.fills.len(), 1);
}

/// A symbol clips its layers only with `clippingBehavior` 1, whatever its `groupBehavior`.
#[test]
fn symbols_clip_only_when_asked() {
    let m = |id: &str, sid: &str, behaviour: Option<i64>, clipping: Option<i64>| {
        let mut m = master(id, sid, id, 20.0, 20.0, vec![rect(&format!("R{id}"), 0.0, 10.0, 20.0, 20.0)]);
        if let Some(b) = behaviour {
            m["groupBehavior"] = json!(b);
        }
        if let Some(c) = clipping {
            m["clippingBehavior"] = json!(c);
        }
        m
    };
    let masters = vec![
        m("M0", "S0", Some(0), Some(0)),
        m("M1", "S1", Some(1), Some(0)),
        m("M2", "S2", Some(2), Some(0)),
        m("M3", "S3", Some(1), Some(1)),
        m("M4", "S4", Some(1), Some(2)),
        m("M5", "S5", None, None),
    ];
    let layers = (0..6).map(|i| instance(&format!("I{i}"), &format!("S{i}"), 30.0 * f64::from(i), 0.0, 20.0, 20.0, &[])).collect();
    let (doc, _) = open_version("2026.3.1", 196, layers, masters);
    let clips: Vec<bool> = (0..6).map(|i| instance_of(&doc, &format!("I{i}")).0.clip).collect();
    assert_eq!(clips, [false, false, false, true, false, false]);
}

/// An image override on a shape replaces its top image fill's image.
#[test]
fn image_overrides_on_shapes() {
    let pattern = |r: &str| {
        json!({"_class": "fill", "isEnabled": true, "fillType": 4, "patternFillType": 1, "patternTileScale": 1,
            "image": {"_class": "MSJSONFileReference", "_ref_class": "MSImageData", "_ref": r}})
    };
    let avatar = styled(rect("Avatar", 0.0, 0.0, 20.0, 20.0), "fills", json!([pattern("images/a.png"), pattern("images/b.png")]));
    let over = json!({"_class": "MSJSONFileReference", "_ref_class": "MSImageData", "_ref": "images/c.png"});
    let (a, b, c) = (png(1), png(2), png(3));
    let z = sketch_zip(
        "2026.3.1",
        196,
        document(&["P", "S"]),
        &[
            page("P", "Page", vec![instance("I", "S", 0.0, 0.0, 20.0, 20.0, &[("Avatar_image", over)])]),
            page("S", "Symbols", vec![master("M", "S", "Contact", 20.0, 20.0, vec![avatar])]),
        ],
        &[("images/a.png", &a), ("images/b.png", &b), ("images/c.png", &c)],
    );
    let (doc, _) = import(&z, "f").unwrap();
    let (inst, main) = instance_of(&doc, "I");
    let images = |n: &Node| -> Vec<String> {
        n.fills
            .iter()
            .filter_map(|p| match &p.kind {
                figcraft_doc::color::PaintKind::Image(i) => Some(i.image.clone()),
                _ => None,
            })
            .collect()
    };
    let (shown, master_images) = (images(inside(&doc, inst, "Avatar")), images(inside(&doc, main, "Avatar")));
    assert_eq!(shown.len(), 2);
    assert_ne!(master_images[0], master_images[1]);
    assert_eq!(shown[0], master_images[0], "the lower fill keeps its image");
    assert_ne!(shown[1], master_images[1], "the top fill takes the override");
}

/// An image override still shows when the same layer's style is overridden too (the style comes
/// first, then the image).
#[test]
fn image_overrides_survive_layer_style_overrides() {
    let pattern = |r: &str| {
        json!({"_class": "fill", "isEnabled": true, "fillType": 4, "patternFillType": 1, "patternTileScale": 1,
            "image": {"_class": "MSJSONFileReference", "_ref_class": "MSImageData", "_ref": r}})
    };
    let mut d = document(&["P", "S"]);
    d["layerStyles"]["objects"] = json!([{"_class": "sharedStyle", "do_objectID": "LS", "name": "Round",
        "value": {"_class": "style", "fills": [pattern("images/a.png")], "borders": []}}]);
    let avatar = styled(rect("Avatar", 0.0, 0.0, 20.0, 20.0), "fills", json!([pattern("images/a.png")]));
    let over = json!({"_class": "MSJSONFileReference", "_ref_class": "MSImageData", "_ref": "images/c.png"});
    let (a, c) = (png(1), png(3));
    let z = sketch_zip(
        "2026.3.1",
        196,
        d,
        &[
            page("P", "Page", vec![instance("I", "S", 0.0, 0.0, 20.0, 20.0, &[("Avatar_layerStyle", json!("LS")), ("Avatar_image", over)])]),
            page("S", "Symbols", vec![master("M", "S", "Contact", 20.0, 20.0, vec![avatar])]),
        ],
        &[("images/a.png", &a), ("images/c.png", &c)],
    );
    let (doc, _) = import(&z, "f").unwrap();
    let (inst, main) = instance_of(&doc, "I");
    let image = |n: &Node| {
        n.fills.iter().find_map(|p| match &p.kind {
            figcraft_doc::color::PaintKind::Image(i) => Some(i.image.clone()),
            _ => None,
        })
    };
    let (shown, original) = (image(inside(&doc, inst, "Avatar")), image(inside(&doc, main, "Avatar")));
    assert!(shown.is_some() && shown != original, "{shown:?} {original:?}");
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
