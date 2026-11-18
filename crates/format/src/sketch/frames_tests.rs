//! Sketch 2025 and newer: frames and graphics (groups with a `groupBehavior`), stacks, corners in
//! the style, pins and sizing, border sides.

use figcraft_doc::{Document, LayoutMode, Node, NodeKind};
use serde_json::{Value, json};

use super::fixture::*;
use super::import;

fn open(layers: Vec<Value>) -> (Document, Vec<String>) {
    let z = sketch_zip("2026.3.1", 196, document(&["P"]), &[page("P", "Page", layers)], &[]);
    import(&z, "f").unwrap()
}

fn node<'d>(doc: &'d Document, name: &str) -> &'d Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn frames_graphics_and_clipping() {
    let mut clip_none = frame_group("Loose", 0.0, 0.0, 50.0, 50.0, vec![]);
    clip_none["clippingBehavior"] = json!(2);
    let mut graphic = frame_group("Icon", 100.0, 0.0, 24.0, 24.0, vec![]);
    graphic["groupBehavior"] = json!(2);
    let mut plain = group("Plain", 200.0, 0.0, 10.0, 10.0, vec![rect("Inside", 0.0, 0.0, 10.0, 10.0)]);
    plain["groupBehavior"] = json!(0);
    let mut filled = frame_group("Filled", 300.0, 0.0, 50.0, 50.0, vec![]);
    filled["style"]["fills"] = json!([solid_fill(1.0, 0.0, 0.0, 1.0)]);
    let (doc, _) = open(vec![frame_group("Screen", 0.0, 100.0, 390.0, 844.0, vec![]), clip_none, graphic, plain, filled]);
    assert!(matches!(node(&doc, "Screen").kind, NodeKind::Frame) && node(&doc, "Screen").clip);
    assert!(!node(&doc, "Loose").clip);
    assert!(matches!(node(&doc, "Icon").kind, NodeKind::Frame) && node(&doc, "Icon").clip);
    assert!(matches!(node(&doc, "Plain").kind, NodeKind::Group));
    assert_eq!(node(&doc, "Filled").fills.len(), 1);
    // Frames show what their style says: no white background of their own.
    assert!(node(&doc, "Screen").fills.is_empty());
}

#[test]
fn corners_from_the_style() {
    let mut a = rect("Each", 0.0, 0.0, 40.0, 40.0);
    a["style"]["corners"] =
        json!({"_class": "MSImmutableStyleCorners", "radii": [1, 2, 3, 4], "style": 0, "smoothing": 0.6, "prefersConcentric": false});
    let mut b = rect("Smooth", 50.0, 0.0, 40.0, 40.0);
    b["style"]["corners"] = json!({"_class": "MSImmutableStyleCorners", "radii": [12], "style": 1, "smoothing": 0.6, "prefersConcentric": false});
    let mut f = frame_group("Card", 100.0, 0.0, 100.0, 100.0, vec![]);
    f["style"]["corners"] = json!({"_class": "MSImmutableStyleCorners", "radii": [16, 16, 16, 16], "style": 0, "smoothing": 0.6});
    let (doc, _) = open(vec![a, b, f]);
    assert_eq!(node(&doc, "Each").corners.radii, [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(node(&doc, "Smooth").corners.radii, [12.0; 4]);
    assert!((node(&doc, "Smooth").corners.smoothing - 0.6).abs() < 1e-9);
    assert_eq!(node(&doc, "Each").corners.smoothing, 0.0);
    assert_eq!(node(&doc, "Card").corners.radii, [16.0; 4]);
}

#[test]
fn stacks_become_auto_layout() {
    // A row: Sketch lists the first item last (on top); justify evenly, centred, gap 8.
    let a = rect("First", 0.0, 0.0, 20.0, 20.0);
    let b = with(rect("Second", 0.0, 0.0, 30.0, 10.0), "horizontalSizing", json!(2));
    let mut free = rect("Free", 0.0, 0.0, 5.0, 5.0);
    free["flexItem"] = json!({"_class": "MSImmutableFlexItem", "ignoreLayout": true, "alignSelf": 5, "preserveSpaceWhenHidden": false});
    let mut row = stack(frame_group("Row", 0.0, 0.0, 200.0, 40.0, vec![free, b, a]), 0, 5, 1, 8.0, [4.0, 6.0, 10.0, 2.0]);
    row["horizontalSizing"] = json!(0);
    row["verticalSizing"] = json!(1);
    let col = stack(frame_group("Column", 0.0, 100.0, 100.0, 100.0, vec![rect("One", 0.0, 0.0, 10.0, 10.0)]), 1, 3, 3, 0.0, [0.0; 4]);
    let (doc, _) = open(vec![row, col]);
    let r = node(&doc, "Row");
    let l = r.layout.as_ref().expect("auto layout");
    assert_eq!(l.mode, LayoutMode::Horizontal);
    assert_eq!(l.gap, 8.0);
    assert_eq!(l.padding, [6.0, 10.0, 2.0, 4.0], "top, right, bottom, left");
    assert!(l.reverse_z, "the first item is on top, as in Sketch");
    let text = format!("{l:?}");
    assert!(text.contains("SpaceEvenly") || text.contains("Evenly"), "{text}");
    let order: Vec<&str> = r.children.iter().map(|c| doc.node(*c).unwrap().name.as_str()).collect();
    assert_eq!(order, ["First", "Second", "Free"], "flow order is the reverse of Sketch's list");
    assert!(node(&doc, "Free").child_layout.absolute);
    assert_eq!(node(&doc, "Second").child_layout.sizing_h, figcraft_doc::Sizing::Fill);
    assert_eq!(r.child_layout.sizing_v, figcraft_doc::Sizing::Hug);
    let c = node(&doc, "Column").layout.as_ref().unwrap();
    assert_eq!(c.mode, LayoutMode::Vertical);
    assert!(format!("{c:?}").contains("SpaceBetween"), "{c:?}");
}

/// A frame resized as an instance's layer follows its pins and sizing (Sketch 2025+ keep them in
/// `horizontalPins`/`horizontalSizing`, `resizingConstraint` stays 63).
#[test]
fn pins_and_sizing_resize_instances() {
    let mut r = rect("R", 20.0, 30.0, 100.0, 40.0);
    r["horizontalPins"] = json!(1); // left
    r["horizontalSizing"] = json!(0); // fixed
    r["verticalPins"] = json!(0);
    r["verticalSizing"] = json!(3); // relative
    let mut m = master("M", "S", "Box", 200.0, 100.0, vec![r]);
    m["groupBehavior"] = json!(1);
    let (doc, _) = {
        let z = sketch_zip(
            "2026.3.1",
            196,
            document(&["P", "S"]),
            &[page("P", "Page", vec![instance("I", "S", 0.0, 0.0, 320.0, 60.0, &[])]), page("S", "Symbols", vec![m])],
            &[],
        );
        import(&z, "f").unwrap()
    };
    let inst = doc.nodes().find(|n| n.name == "I").unwrap();
    let r = doc.subtree(inst.id).into_iter().filter_map(|i| doc.node(i)).find(|n| n.name == "R").unwrap();
    let b = doc.world_bounds(r.id).unwrap();
    assert!((b.x0 - 20.0).abs() < 0.5 && (b.width() - 100.0).abs() < 0.5, "{b:?}");
    assert!((b.y0 - 18.0).abs() < 0.5 && (b.height() - 24.0).abs() < 0.5, "{b:?}");
}

#[test]
fn graphics_scale_their_layers() {
    let mut r = rect("R", 20.0, 30.0, 100.0, 40.0);
    r["horizontalPins"] = json!(5);
    r["horizontalSizing"] = json!(0);
    let mut m = master("M", "S", "Glyph", 200.0, 100.0, vec![r]);
    m["groupBehavior"] = json!(2);
    let z = sketch_zip(
        "2026.3.1",
        196,
        document(&["P", "S"]),
        &[page("P", "Page", vec![instance("I", "S", 0.0, 0.0, 400.0, 200.0, &[])]), page("S", "Symbols", vec![m])],
        &[],
    );
    let (doc, _) = import(&z, "f").unwrap();
    let inst = doc.nodes().find(|n| n.name == "I").unwrap();
    let r = doc.subtree(inst.id).into_iter().filter_map(|i| doc.node(i)).find(|n| n.name == "R").unwrap();
    let b = doc.world_bounds(r.id).unwrap();
    assert!((b.x0 - 40.0).abs() < 0.5 && (b.width() - 200.0).abs() < 0.5, "{b:?}");
}

#[test]
fn border_sides() {
    let r = styled(
        rect("Sided", 0.0, 0.0, 40.0, 40.0),
        "borders",
        json!([{"_class": "border", "isEnabled": true, "fillType": 0, "color": color(0.0, 0.0, 0.0, 1.0), "position": 1, "thickness": 8,
            "sides": {"_class": "MSImmutableStyleBorderSides", "leftInset": 7, "rightInset": 7, "topInset": 0, "bottomInset": 5}}]),
    );
    // A side is as thick as the border less its inset.
    let (doc, _) = open(vec![r]);
    assert_eq!(node(&doc, "Sided").stroke.sides, Some([8.0, 1.0, 3.0, 1.0]));
}

#[test]
fn swatches_bind_solid_fills_only() {
    let mut d = document(&["P"]);
    d["sharedSwatches"] = json!({"_class": "swatchContainer", "objects": [{"_class": "swatch", "do_objectID": "SW", "name": "Brand", "value": color(0.0, 0.0, 1.0, 1.0)}]});
    let mut g = json!({"_class": "fill", "isEnabled": true, "fillType": 1, "color": color(0.0, 0.0, 1.0, 1.0),
        "gradient": {"_class": "gradient", "gradientType": 0, "from": "{0.5, 0}", "to": "{0.5, 1}", "stops": [
            {"_class": "gradientStop", "position": 0, "color": color(1.0, 0.0, 0.0, 1.0)}, {"_class": "gradientStop", "position": 1, "color": color(0.0, 0.0, 1.0, 1.0)}]}});
    g["color"]["swatchID"] = json!("SW");
    let z = sketch_zip("2026.3.1", 196, d, &[page("P", "Page", vec![styled(rect("G", 0.0, 0.0, 10.0, 10.0), "fills", json!([g]))])], &[]);
    let (_, warnings) = import(&z, "f").unwrap();
    assert!(!warnings.iter().any(|w| w.contains("not supported")), "{warnings:?}");
}

/// Sketch 2025+ masters keep their background, corners and stack in their style and layout, and
/// their instances show the same (the background when the master includes it in instances and
/// exports).
#[test]
fn masters_and_instances_keep_frame_styles_and_stacks() {
    let mut m = master("M", "S", "Button", 120.0, 44.0, vec![rect("Icon", 0.0, 0.0, 20.0, 20.0), rect("Label", 0.0, 0.0, 60.0, 20.0)]);
    m["groupBehavior"] = json!(1);
    m["clippingBehavior"] = json!(2);
    m["includeBackgroundColorInInstance"] = json!(true);
    m["includeBackgroundColorInExport"] = json!(true);
    m["style"]["fills"] = json!([solid_fill(0.0, 0.0, 1.0, 1.0)]);
    m["style"]["corners"] = json!({"_class": "MSImmutableStyleCorners", "radii": [8], "style": 1, "smoothing": 0.6});
    let m = stack(m, 0, 1, 1, 10.0, [16.0, 12.0, 16.0, 12.0]);
    let z = sketch_zip(
        "2026.3.1",
        196,
        document(&["P", "S"]),
        &[page("P", "Page", vec![instance("I", "S", 0.0, 0.0, 120.0, 44.0, &[])]), page("S", "Symbols", vec![m])],
        &[],
    );
    let (doc, _) = import(&z, "f").unwrap();
    let main = doc.nodes().find(|n| matches!(n.kind, NodeKind::Component { .. })).unwrap();
    let inst = doc.nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap();
    for n in [main, inst] {
        assert_eq!(n.fills.len(), 1, "{}", n.name);
        assert_eq!(n.corners.radii, [8.0; 4], "{}", n.name);
        assert!(n.layout.is_some(), "{}", n.name);
        assert!(!n.clip, "{}", n.name);
        let order: Vec<&str> = n.children.iter().map(|c| doc.node(*c).unwrap().name.as_str()).collect();
        assert_eq!(order, ["Label", "Icon"], "{}", n.name);
    }
}

fn solid_of(n: &Node) -> Option<[f32; 3]> {
    n.fills.iter().find_map(|p| match &p.kind {
        figcraft_doc::color::PaintKind::Solid { color } => Some([color.r, color.g, color.b]),
        _ => None,
    })
}

fn open_symbols(layers: Vec<Value>, masters: Vec<Value>) -> Document {
    let z = sketch_zip("2026.3.1", 196, document(&["P", "S"]), &[page("P", "Page", layers), page("S", "Symbols", masters)], &[]);
    import(&z, "f").unwrap().0
}

fn in_instance<'d>(doc: &'d Document, inst: &str, name: &str) -> &'d Node {
    let i = doc.nodes().find(|n| n.name == inst && matches!(n.kind, NodeKind::Instance { .. })).unwrap_or_else(|| panic!("no {inst}"));
    doc.subtree(i.id).into_iter().filter_map(|id| doc.node(id)).find(|n| n.name == name).unwrap_or_else(|| panic!("no {name} in {inst}"))
}

/// A graphic's fill tints its layers (it's no background); an instance's fill, or a `fillColor`
/// override, tints the instance's layers.
#[test]
fn fills_of_graphics_and_instances_are_tints() {
    let glyph = styled(rect("Glyph", 0.0, 0.0, 24.0, 24.0), "fills", json!([solid_fill(0.0, 0.0, 0.0, 1.0)]));
    let mut icon = master("MI", "SI", "Icon", 24.0, 24.0, vec![glyph]);
    icon["groupBehavior"] = json!(2);
    let mut tint = solid_fill(1.0, 0.0, 0.0, 1.0);
    tint["layeringType"] = json!(1);
    icon["style"]["fills"] = json!([tint]);
    let nested = instance("N", "SI", 0.0, 0.0, 24.0, 24.0, &[]);
    let mut button = master("MB", "SB", "Button", 100.0, 40.0, vec![nested]);
    button["groupBehavior"] = json!(1);
    let own = instance("Own", "SI", 0.0, 0.0, 24.0, 24.0, &[("Own_fillColor", color(0.0, 0.0, 1.0, 1.0))]);
    let via = instance("Via", "SB", 0.0, 100.0, 100.0, 40.0, &[("N_fillColor", color(0.0, 1.0, 0.0, 1.0))]);
    let doc = open_symbols(vec![own, via], vec![icon, button]);
    let main = doc.nodes().find(|n| n.name == "Icon" && matches!(n.kind, NodeKind::Component { .. })).unwrap();
    assert!(main.fills.is_empty(), "a graphic's fill isn't its background");
    let in_main = doc.subtree(main.id).into_iter().filter_map(|i| doc.node(i)).find(|n| n.name == "Glyph").unwrap();
    assert_eq!(solid_of(in_main), Some([1.0, 0.0, 0.0]));
    assert_eq!(solid_of(in_instance(&doc, "Own", "Glyph")), Some([0.0, 0.0, 1.0]));
    assert!(doc.nodes().find(|n| n.name == "Own").unwrap().fills.is_empty());
    assert_eq!(solid_of(in_instance(&doc, "Via", "Glyph")), Some([0.0, 1.0, 0.0]));
}

#[test]
fn text_color_overrides() {
    let label = text("Label", 0.0, 0.0, 80.0, 20.0, "Go", &[(0, 2, "Helvetica", 14.0, [0.0, 0.0, 0.0, 1.0])]);
    let m = master("M", "S", "Button", 100.0, 40.0, vec![label]);
    let doc = open_symbols(vec![instance("I", "S", 0.0, 0.0, 100.0, 40.0, &[("Label_textColor", color(1.0, 0.0, 0.0, 1.0))])], vec![m]);
    let n = in_instance(&doc, "I", "Label");
    let red = |p: &figcraft_doc::color::Paint| matches!(p.kind, figcraft_doc::color::PaintKind::Solid { color } if color.r == 1.0 && color.g == 0.0);
    let NodeKind::Text(td) = &n.kind else { panic!("not text") };
    assert!(n.fills.iter().any(red) || td.style.fills.as_ref().is_some_and(|f| f.iter().any(red)), "{:?} {:?}", n.fills, td.style.fills);
}

#[test]
fn blurs_list() {
    let blur = |ty: i64, r: f64| json!({"_class": "blur", "isEnabled": true, "type": ty, "radius": r, "center": "{0.5, 0.5}", "motionAngle": 25});
    let g = styled(rect("Glow", 0.0, 0.0, 40.0, 40.0), "blurs", json!([blur(0, 90.0)]));
    let m = styled(rect("Comet", 50.0, 0.0, 40.0, 4.0), "blurs", json!([blur(1, 6.0)]));
    let b = styled(rect("Glass", 100.0, 0.0, 40.0, 40.0), "blurs", json!([blur(3, 10.0)]));
    let (doc, warnings) = open(vec![g, m, b]);
    assert!(matches!(node(&doc, "Glow").effects[0].kind, figcraft_doc::EffectKind::LayerBlur(_)));
    assert!(matches!(node(&doc, "Glass").effects[0].kind, figcraft_doc::EffectKind::BackgroundBlur(_)));
    assert!(matches!(node(&doc, "Comet").effects[0].kind, figcraft_doc::EffectKind::LayerBlur(_)), "motion blur drawn as a layer blur");
    assert!(warnings.iter().any(|w| w.contains("motion")), "{warnings:?}");
}

#[test]
fn text_sizing_2025() {
    let mut fixed = text("Fixed", 0.0, 0.0, 100.0, 40.0, "Wraps here", &[(0, 10, "Helvetica", 14.0, [0.0, 0.0, 0.0, 1.0])]);
    fixed.as_object_mut().unwrap().remove("textBehaviour");
    fixed["horizontalSizing"] = json!(0);
    fixed["verticalSizing"] = json!(1);
    let mut fit = text("Fit", 0.0, 50.0, 100.0, 20.0, "Grows", &[(0, 5, "Helvetica", 14.0, [0.0, 0.0, 0.0, 1.0])]);
    fit.as_object_mut().unwrap().remove("textBehaviour");
    fit["horizontalSizing"] = json!(1);
    fit["verticalSizing"] = json!(1);
    let (doc, _) = open(vec![fixed, fit]);
    let auto = |n: &str| match &node(&doc, n).kind {
        NodeKind::Text(td) => td.auto_resize,
        _ => panic!("not text"),
    };
    assert_eq!(auto("Fixed"), figcraft_doc::TextAutoResize::Height);
    assert_eq!(auto("Fit"), figcraft_doc::TextAutoResize::WidthAndHeight);
}

#[test]
fn stacks_are_no_smart_layout_note() {
    let (_, warnings) = open(vec![stack(frame_group("Row", 0.0, 0.0, 100.0, 40.0, vec![rect("A", 0.0, 0.0, 10.0, 10.0)]), 0, 0, 0, 0.0, [0.0; 4])]);
    assert!(!warnings.iter().any(|w| w.contains("Smart Layout")), "{warnings:?}");
}

/// Sketch 2025+: a fill with `layeringType` 1 is a tint (of a frame or master too), 0 a background.
#[test]
fn layering_type_decides_tint_or_background() {
    let dot = styled(rect("Dot", 0.0, 0.0, 10.0, 10.0), "fills", json!([solid_fill(0.0, 0.0, 0.0, 1.0)]));
    let mut tint = solid_fill(0.0, 0.0, 1.0, 1.0);
    tint["layeringType"] = json!(1);
    let mut bar = master("MB", "SB", "Bar", 100.0, 20.0, vec![dot]);
    bar["groupBehavior"] = json!(1);
    bar["style"]["fills"] = json!([tint]);
    let mut bg = solid_fill(1.0, 0.0, 0.0, 1.0);
    bg["layeringType"] = json!(0);
    let mut btn = master("MC", "SC", "Btn", 100.0, 20.0, vec![]);
    btn["groupBehavior"] = json!(1);
    btn["includeBackgroundColorInInstance"] = json!(true);
    btn["includeBackgroundColorInExport"] = json!(true);
    btn["style"]["fills"] = json!([bg]);
    let doc = open_symbols(vec![instance("B", "SB", 0.0, 0.0, 100.0, 20.0, &[]), instance("C", "SC", 0.0, 50.0, 100.0, 20.0, &[])], vec![bar, btn]);
    let b = doc.nodes().find(|n| n.name == "B").unwrap();
    assert!(b.fills.is_empty(), "a tint isn't a background");
    assert_eq!(solid_of(in_instance(&doc, "B", "Dot")), Some([0.0, 0.0, 1.0]));
    assert_eq!(solid_of(doc.nodes().find(|n| n.name == "C").unwrap()), Some([1.0, 0.0, 0.0]));
}

#[test]
fn inner_shadows_in_the_shadow_list() {
    let mut sh =
        json!({"_class": "shadow", "isEnabled": true, "blurRadius": 4, "offsetX": 0, "offsetY": 2, "spread": 0, "color": color(0.0, 0.0, 0.0, 0.5)});
    sh["isInnerShadow"] = json!(true);
    let (doc, _) = open(vec![styled(rect("Inset", 0.0, 0.0, 40.0, 40.0), "shadows", json!([sh]))]);
    assert!(matches!(node(&doc, "Inset").effects[0].kind, figcraft_doc::EffectKind::InnerShadow(_)));
}

#[test]
fn visibility_overrides() {
    let m = master("M", "S", "Input", 100.0, 40.0, vec![rect("Label", 0.0, 0.0, 50.0, 10.0)]);
    let doc = open_symbols(vec![instance("I", "S", 0.0, 0.0, 100.0, 40.0, &[("Label_isVisible", json!("0"))])], vec![m]);
    assert!(!in_instance(&doc, "I", "Label").visible);
}

/// An instance that fits its content hugs it even when its master has a fixed size.
#[test]
fn instance_sizing_is_its_own() {
    let m = stack(master("M", "S", "Chip", 60.0, 24.0, vec![rect("A", 0.0, 0.0, 20.0, 20.0)]), 0, 0, 1, 0.0, [0.0; 4]);
    let mut i = instance("I", "S", 0.0, 0.0, 60.0, 24.0, &[]);
    i["horizontalSizing"] = json!(1);
    let doc = open_symbols(vec![i], vec![m]);
    let n = doc.nodes().find(|n| n.name == "I").unwrap();
    assert_eq!(n.layout.as_ref().map(|_| format!("{:?}", n.child_layout.sizing_h)), Some("Hug".to_string()));
}

/// A tint keeps each layer's own opacity.
#[test]
fn tints_keep_opacity() {
    let faint = styled(rect("Faint", 0.0, 0.0, 10.0, 10.0), "fills", json!([solid_fill(0.0, 0.0, 0.0, 0.25)]));
    let mut g = group("Tinted", 0.0, 0.0, 10.0, 10.0, vec![faint]);
    let mut tint = solid_fill(1.0, 0.0, 0.0, 1.0);
    tint["layeringType"] = json!(1);
    g["groupBehavior"] = json!(0);
    g["style"] = json!({"_class": "style", "fills": [tint]});
    let (doc, _) = open(vec![g]);
    let f = &node(&doc, "Faint").fills[0];
    let figcraft_doc::color::PaintKind::Solid { color } = &f.kind else { panic!("not solid") };
    assert_eq!((color.r, color.g), (1.0, 0.0));
    assert!((color.a * f.opacity - 0.25).abs() < 1e-3, "{color:?} {}", f.opacity);
    assert!(node(&doc, "Tinted").fills.is_empty());
}

#[test]
fn glass_and_progressive_blurs() {
    let glass = json!({"_class": "blur", "isEnabled": true, "type": 4, "radius": 10, "distortion": 0.6, "depth": 0.7, "center": "{0.5, 0.5}"});
    let prog = json!({"_class": "blur", "isEnabled": true, "type": 3, "radius": 24, "isProgressive": true, "center": "{0.5, 0.5}",
        "gradient": {"_class": "gradient", "gradientType": 0, "from": "{0.5, 0}", "to": "{0.5, 1}", "stops": [
            {"_class": "gradientStop", "position": 0, "color": color(0.0, 0.0, 0.0, 1.0)}, {"_class": "gradientStop", "position": 1, "color": color(0.0, 0.0, 0.0, 0.0)}]}});
    let (doc, warnings) = open(vec![
        styled(rect("Glass", 0.0, 0.0, 40.0, 40.0), "blurs", json!([glass])),
        styled(rect("Prog", 50.0, 0.0, 40.0, 40.0), "blurs", json!([prog])),
    ]);
    assert!(matches!(node(&doc, "Glass").effects[0].kind, figcraft_doc::EffectKind::Glass(_)), "{warnings:?}");
    let figcraft_doc::EffectKind::BackgroundBlur(b) = &node(&doc, "Prog").effects[0].kind else { panic!("not a background blur") };
    assert_eq!(b.blur_type, figcraft_doc::BlurType::Progressive);
    // Sketch's radius is the blur's standard deviation; FigCraft's twice it.
    assert_eq!((b.start_radius, b.radius), (48.0, 0.0));
}

/// Angled (2), inside-square (3) and inside-arc (4) corners make the rectangle a path of that outline.
#[test]
fn cut_corners_become_paths() {
    let cut = |name: &str, style: i64| {
        let mut r = rect(name, 0.0, 0.0, 100.0, 80.0);
        r["style"]["corners"] = json!({"_class": "MSImmutableStyleCorners", "radii": [10], "style": style, "smoothing": 0.6});
        r
    };
    let (doc, _) = open(vec![cut("Angled", 2), cut("Square", 3), cut("Arc", 4)]);
    let net = |n: &str| match &node(&doc, n).kind {
        NodeKind::Vector { network } => network.clone(),
        other => panic!("{n}: {other:?}"),
    };
    let a = net("Angled");
    assert_eq!(a.vertices.len(), 8);
    assert!(a.vertices.iter().any(|v| (v.x, v.y) == (0.0, 10.0)) && a.vertices.iter().any(|v| (v.x, v.y) == (10.0, 0.0)));
    let s = net("Square");
    assert_eq!(s.vertices.len(), 12);
    assert!(s.vertices.iter().any(|v| (v.x, v.y) == (10.0, 10.0)) && s.vertices.iter().any(|v| (v.x, v.y) == (90.0, 70.0)));
    let arc = net("Arc");
    assert_eq!(arc.vertices.len(), 8);
    assert!(arc.segments.iter().any(|g| g.ta != (0.0, 0.0)), "the corners are arcs");
}

/// Tints and colour overrides that aren't colours, and fills that aren't objects, are junk: left
/// alone, never a panic.
#[test]
fn junk_tints_and_colour_overrides() {
    let child = styled(rect("Child", 0.0, 0.0, 10.0, 10.0), "fills", json!([solid_fill(0.0, 0.0, 0.0, 1.0)]));
    let mut g = group("G", 0.0, 0.0, 10.0, 10.0, vec![child.clone()]);
    g["style"] = json!({"_class": "style", "fills": [{"_class": "fill", "isEnabled": true, "fillType": 0, "layeringType": 1, "color": 5}]});
    let (_, _) = open(vec![g]);
    let mut odd = rect("L", 0.0, 0.0, 10.0, 10.0);
    odd["style"]["fills"] = json!([true]);
    let m = master("M", "S", "Odd", 10.0, 10.0, vec![odd]);
    let _ = open_symbols(vec![instance("I", "S", 0.0, 0.0, 10.0, 10.0, &[("L_fillColor", color(1.0, 0.0, 0.0, 1.0))])], vec![m]);
}

/// Tinting copies a group's layers: the copies count against the import's work budget, so nested
/// tinted groups of large layers can't make the import copy without end; past the budget, tints
/// are skipped with a note.
#[test]
fn nested_tints_are_budgeted() {
    // Text that doesn't compress, so the page stays within the archive's size checks.
    let mut seed = 0x2545_f491_u32;
    let big: String = (0..1_000_000)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            char::from(b'a' + (seed % 26) as u8)
        })
        .collect();
    let mut l = styled(rect("Leaf", 0.0, 0.0, 10.0, 10.0), "fills", json!([solid_fill(0.0, 0.0, 0.0, 1.0)]));
    for i in 0..35 {
        let mut pad = rect(&big, 0.0, 0.0, 1.0, 1.0);
        pad["do_objectID"] = json!(format!("pad{i}"));
        let mut g = group(&format!("G{i}"), 0.0, 0.0, 10.0, 10.0, vec![l, pad]);
        let mut t = solid_fill(1.0, 0.0, 0.0, 1.0);
        t["layeringType"] = json!(1);
        g["style"] = json!({"_class": "style", "fills": [t]});
        l = g;
    }
    let (_, warnings) = open(vec![l]);
    assert!(warnings.iter().any(|w| w.contains("tint")), "{warnings:?}");
}

/// A faded layer in a stack keeps its place in the stack: the group the fade makes takes the
/// layer's id, and with it its sizing.
#[test]
fn faded_stack_items_keep_their_sizing() {
    let fade = json!({"_class": "graphicsContextSettings", "opacity": 1, "isProgressive": true,
        "gradient": {"_class": "gradient", "gradientType": 0, "from": "{0.5, 0}", "to": "{0.5, 1}", "stops": [
            {"_class": "gradientStop", "position": 0, "color": color(0.0, 0.0, 0.0, 1.0)}, {"_class": "gradientStop", "position": 1, "color": color(0.0, 0.0, 0.0, 0.0)}]}});
    let mut item = styled(rect("Item", 0.0, 0.0, 50.0, 20.0), "contextSettings", fade);
    item["horizontalSizing"] = json!(2);
    let (doc, _) = open(vec![stack(frame_group("Row", 0.0, 0.0, 200.0, 20.0, vec![item]), 0, 0, 0, 0.0, [0.0; 4])]);
    let row = node(&doc, "Row");
    let wrap = doc.node(row.children[0]).unwrap();
    assert_eq!(format!("{:?}", wrap.child_layout.sizing_h), "Fill", "{:?}", wrap.child_layout);
}

/// Tints came with Sketch 64 (format 124 and newer): before them a group's fill draws nothing, so
/// its layers keep their own colours (seen on a Sketch 63 file's renders).
#[test]
fn group_fills_tint_only_from_sketch_64() {
    let make = |version: u64| {
        let white = styled(rect("Dot", 0.0, 0.0, 10.0, 10.0), "fills", json!([solid_fill(1.0, 1.0, 1.0, 1.0)]));
        let g = styled(group("G", 0.0, 0.0, 10.0, 10.0, vec![white]), "fills", json!([solid_fill(0.0, 0.0, 0.0, 1.0)]));
        let z = sketch_zip(if version > 123 { "64" } else { "63.1" }, version, document(&["P"]), &[page("P", "Page", vec![g])], &[]);
        import(&z, "f").unwrap().0
    };
    assert_eq!(solid_of(node(&make(123), "Dot")), Some([1.0, 1.0, 1.0]));
    assert_eq!(solid_of(node(&make(124), "Dot")), Some([0.0, 0.0, 0.0]));
}

/// A stretching stack (`alignItems` 3) fills its items across, fixed and hugging ones too.
#[test]
fn stretching_stacks_fill_items_across() {
    let mut fixed = rect("Fixed", 0.0, 0.0, 60.0, 20.0);
    fixed["horizontalSizing"] = json!(0);
    let mut hug = frame_group("Hug", 0.0, 20.0, 60.0, 20.0, vec![]);
    hug["horizontalSizing"] = json!(1);
    let col = |name: &str, align: i64| stack(frame_group(name, 0.0, 0.0, 60.0, 40.0, vec![hug.clone(), fixed.clone()]), 1, 0, align, 0.0, [0.0; 4]);
    let (doc, _) = open(vec![col("Start", 0), col("Stretch", 3)]);
    let sizing = |parent: &str, name: &str| {
        let p = node(&doc, parent);
        let n = p.children.iter().filter_map(|c| doc.node(*c)).find(|n| n.name == name).unwrap();
        n.child_layout.sizing_h
    };
    use figcraft_doc::Sizing as LayoutSizing;
    assert_eq!(sizing("Start", "Fixed"), LayoutSizing::Fixed);
    assert_eq!(sizing("Stretch", "Fixed"), LayoutSizing::Fill);
    assert_eq!(sizing("Stretch", "Hug"), LayoutSizing::Fill);
}
