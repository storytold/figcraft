//! Export behaviour suite, written from the public documentation of design tools: export settings
//! (scales, fixed widths and heights, suffixes, formats, the advanced options), what an export
//! covers (effects, strokes, hidden layers, masks, slices, clipping, rotation, sub-pixel sizes),
//! exporting several layers, Export frames to PDF and the Copy as actions. Each rule is a table row
//! or an assert with a one-line derivation; where the documentation is silent the row says so and
//! pins today's behaviour.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use base64::Engine as _;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{Effect, EffectKind, NodeId};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// A rectangle filled with `fill` (hex).
fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, fill: &str, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    let r = id(&ex(s, "shape.rectangle", p));
    set(s, &[r], json!({"fills": [fill]}));
    r
}

fn frame(s: &mut Session, x: f64, y: f64, w: f64, h: f64, clip: bool) -> NodeId {
    let f = id(&ex(s, "frame.create", json!({"x": x, "y": y, "width": w, "height": h})));
    set(s, &[f], json!({"clip": clip, "fills": []}));
    f
}

fn set(s: &mut Session, ids: &[NodeId], props: Value) {
    ex(s, "node.set", json!({"ids": ids, "props": props}));
}

fn name(s: &mut Session, id: NodeId, name: &str) {
    set(s, &[id], json!({"name": name}));
}

/// The bytes `file.export` returns for `p`.
fn export(s: &mut Session, p: Value) -> Vec<u8> {
    let r = ex(s, "file.export", p);
    base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap()
}

fn png_size(b: &[u8]) -> (u32, u32) {
    assert!(b.starts_with(b"\x89PNG"), "not a PNG");
    (u32::from_be_bytes([b[16], b[17], b[18], b[19]]), u32::from_be_bytes([b[20], b[21], b[22], b[23]]))
}

fn pixels(b: &[u8]) -> image::RgbaImage {
    image::load_from_memory(b).unwrap().to_rgba8()
}

fn text(b: &[u8]) -> String {
    String::from_utf8(b.to_vec()).unwrap()
}

/// A drop shadow: offset, blur radius, spread.
fn shadow(s: &mut Session, id: NodeId, offset: (f64, f64), radius: f64, spread: f64) {
    let mut e = Effect::of_type("dropShadow").unwrap();
    if let EffectKind::DropShadow(sh) = &mut e.kind {
        (sh.offset, sh.radius, sh.spread) = (offset, radius, spread);
    }
    s.doc_mut().unwrap().get_mut(id).unwrap().effects.push(e);
}

fn file_names(r: &Value) -> Vec<String> {
    r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn export_sizes_follow_the_scale_or_the_fixed_side() {
    let mut s = Session::new();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0, "#FF0000", None);
    // (size, pixel size, derivation)
    let rows: [(&str, (u32, u32), &str); 10] = [
        ("0.5x", (50, 25), "half scale halves both sides"),
        ("0.75x", (75, 38), "docs silent: a fractional side rounds up (37.5 → 38)"),
        ("1x", (100, 50), "1x is the layer's own size"),
        ("1.5x", (150, 75), "1.5x multiplies both sides"),
        ("2x", (200, 100), "2x doubles both sides"),
        ("3x", (300, 150), "3x triples both sides"),
        ("4x", (400, 200), "4x is the largest preset"),
        ("512w", (512, 256), "a width fixes the width; the height keeps the proportions"),
        ("200h", (400, 200), "a height fixes the height; the width keeps the proportions"),
        ("64w", (64, 32), "a fixed width can also shrink"),
    ];
    for (size, want, why) in rows {
        assert_eq!(png_size(&export(&mut s, json!({"ids": [r], "size": size}))), want, "{size}: {why}");
    }
    assert_eq!(png_size(&export(&mut s, json!({"ids": [r], "scale": 3}))), (300, 150), "`scale` is the same as `3x`");
    let odd = rect(&mut s, 0.0, 100.0, 100.0, 30.0, "#FF0000", None);
    assert_eq!(png_size(&export(&mut s, json!({"ids": [odd], "size": "512w"}))), (512, 154), "docs silent: the other side rounds up (153.6)");
}

#[test]
fn export_file_names_take_the_suffix_and_slashes_make_folders() {
    let mut s = Session::new();
    let icon = rect(&mut s, 0.0, 0.0, 32.0, 32.0, "#FF0000", None);
    name(&mut s, icon, "Icon");
    ex(&mut s, "export.add", json!({"ids": [icon]}));
    ex(&mut s, "export.add", json!({"ids": [icon]}));
    ex(&mut s, "export.add", json!({"ids": [icon], "setting": {"size": "1x", "suffix": "-dark", "format": "svg"}}));
    ex(&mut s, "export.add", json!({"ids": [icon], "setting": {"size": "1x", "format": "png"}}));
    let close = rect(&mut s, 50.0, 0.0, 16.0, 16.0, "#000000", None);
    name(&mut s, close, "icons/close");
    ex(&mut s, "export.add", json!({"ids": [close]}));
    let sneaky = rect(&mut s, 80.0, 0.0, 16.0, 16.0, "#000000", None);
    name(&mut s, sneaky, "../up/./x");
    ex(&mut s, "export.add", json!({"ids": [sneaky]}));
    let r = ex(&mut s, "file.exportLayers", json!({"ids": [icon, close, sneaky]}));
    // (file, derivation)
    let rows: [(&str, &str); 6] = [
        ("Icon.png", "a 1x setting without a suffix is the layer name"),
        ("Icon@2x.png", "a new setting steps the scale up and fills in a matching @2x suffix"),
        ("Icon-dark.svg", "the suffix goes after the layer name (docs silent on a prefix: there is none, names start with the layer name)"),
        ("Icon 2.png", "docs silent: a name used twice gets a number"),
        ("icons/close.png", "a slash in a layer name exports into a folder of that name"),
        ("up/x.png", "docs silent: `.` and `..` segments are dropped, so a name never leaves the export folder"),
    ];
    let names = file_names(&r);
    for (i, (want, why)) in rows.iter().enumerate() {
        assert_eq!(names.get(i).map(String::as_str), Some(*want), "{why}");
    }
    // Into a folder: the slash makes a subfolder.
    let dir = std::env::temp_dir().join(format!("figcraft-export-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    ex(&mut s, "file.exportLayers", json!({"ids": [close], "dir": dir.to_string_lossy()}));
    assert!(dir.join("icons").join("close.png").is_file(), "the folder is created");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_formats_and_what_each_keeps() {
    let mut s = Session::new();
    let e = id(&ex(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 40, "height": 40})));
    set(&mut s, &[e], json!({"fills": ["#FF0000"]}));
    let png = export(&mut s, json!({"ids": [e], "format": "png"}));
    let jpg = export(&mut s, json!({"ids": [e], "format": "jpg"}));
    let svg = export(&mut s, json!({"ids": [e], "format": "svg"}));
    let pdf = export(&mut s, json!({"ids": [e], "format": "pdf"}));
    // (bytes, signature, derivation)
    let rows: [(&[u8], &[u8], &str); 4] = [
        (&png, b"\x89PNG", "PNG is a raster image"),
        (&jpg, b"\xFF\xD8\xFF", "JPG is a raster image"),
        (&svg, b"<svg", "SVG is vector markup"),
        (&pdf, b"%PDF", "PDF is a vector document"),
    ];
    for (bytes, sig, why) in rows {
        assert!(bytes.starts_with(sig), "{why}");
    }
    assert_eq!(pixels(&png).get_pixel(0, 0)[3], 0, "PNG keeps transparency outside the circle");
    let corner = *pixels(&jpg).get_pixel(0, 0);
    assert!(corner[0] > 245 && corner[1] > 245 && corner[2] > 245, "JPG has no transparency: transparent areas are white");
    let mid = *pixels(&jpg).get_pixel(20, 20);
    assert!(mid[0] > 240 && mid[1] < 16 && mid[2] < 16, "docs silent: JPG quality is fixed and high (flat colours come back within a few levels)");
}

#[test]
fn contents_only_leaves_out_overlapping_layers() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0, "#FF0000", None);
    rect(&mut s, 50.0, 50.0, 100.0, 100.0, "#0000FF", None);
    let own = pixels(&export(&mut s, json!({"ids": [a]})));
    assert_eq!(own.get_pixel(75, 75).0, [255, 0, 0, 255], "by default only the layer's own contents are exported");
    let all = export(&mut s, json!({"ids": [a], "contentsOnly": false}));
    assert_eq!(png_size(&all), (100, 100), "with contents only off the export keeps the layer's bounds");
    assert_eq!(pixels(&all).get_pixel(75, 75).0, [0, 0, 255, 255], "with contents only off, layers overlapping it are included");
    // The setting is kept on the layer and used by its exports.
    ex(&mut s, "export.add", json!({"ids": [a], "setting": {"size": "1x", "contentsOnly": false}}));
    assert!(!s.doc().unwrap().node(a).unwrap().exports[0].contents_only, "the export setting keeps the option");
    let r = ex(&mut s, "file.exportLayers", json!({"ids": [a]}));
    let data = base64::engine::general_purpose::STANDARD.decode(r["files"][0]["data"].as_str().unwrap()).unwrap();
    assert_eq!(pixels(&data).get_pixel(75, 75).0, [0, 0, 255, 255], "a layer's own setting is honoured");
}

#[test]
fn penpot_v20_export_limits_shadows_in_all_directions() {
    let mut session = Session::new();
    for (index, offset, size) in
        [((0.0, 0.0), (-12.0, -7.0), (112, 107)), ((200.0, 0.0), (13.0, -9.0), (113, 109)), ((400.0, 0.0), (-3.0, 14.0), (103, 114))]
    {
        let r = rect(&mut session, index.0, index.1, 100.0, 100.0, "#FF0000", None);
        shadow(&mut session, r, offset, 0.0, 0.0);
        assert_eq!(png_size(&export(&mut session, json!({"ids":[r]}))), size, "a sharp offset shadow must extend only the touched export edges");
        assert_eq!(
            png_size(&export(&mut session, json!({"ids":[r], "includeBoundingBox": true}))),
            (100, 100),
            "explicit bounding-box exports must not grow due to a shadow"
        );
    }
}

#[test]
fn what_an_export_covers() {
    let mut s = Session::new();
    let size = |s: &mut Session, id: NodeId| png_size(&export(s, json!({"ids": [id]})));
    // Strokes: (align, weight, size, derivation)
    let strokes: [(&str, f64, (u32, u32), &str); 3] = [
        ("inside", 4.0, (100, 50), "an inside stroke stays within the layer"),
        ("center", 4.0, (104, 54), "a centre stroke reaches half its weight outside"),
        ("outside", 4.0, (108, 58), "an outside stroke reaches its whole weight outside"),
    ];
    for (k, (align, w, want, why)) in strokes.into_iter().enumerate() {
        let r = rect(&mut s, 0.0, 200.0 * k as f64, 100.0, 50.0, "#FF0000", None);
        set(&mut s, &[r], json!({"strokes": ["#000000"], "strokeWeight": w, "strokeAlign": align}));
        assert_eq!(size(&mut s, r), want, "{why}");
    }
    // A shadow extends the export towards its offset: blur 4 reaches 6 (1.5 × blur, docs silent on
    // the exact reach), moved 10 down, so 6 more on the left and right, 16 more below, none above.
    let r = rect(&mut s, 400.0, 0.0, 100.0, 100.0, "#FF0000", None);
    shadow(&mut s, r, (0.0, 10.0), 4.0, 0.0);
    assert_eq!(size(&mut s, r), (112, 116), "effects extend the bounds, in the direction the shadow falls");
    assert_eq!(pixels(&export(&mut s, json!({"ids": [r]}))).get_pixel(6, 0).0, [255, 0, 0, 255], "the layer's top edge is the image's top edge");
    // Include bounding box: the layer's box, whatever it paints outside it.
    let b = export(&mut s, json!({"ids": [r], "includeBoundingBox": true}));
    assert_eq!(png_size(&b), (100, 100), "include bounding box exports the layer's own box");
    // Frames: clipping frames export their box, others everything they hold.
    let clip = frame(&mut s, 0.0, 800.0, 100.0, 100.0, true);
    rect(&mut s, 50.0, 50.0, 100.0, 100.0, "#00FF00", Some(clip));
    assert_eq!(size(&mut s, clip), (100, 100), "a frame that clips its content exports its own box");
    let open = frame(&mut s, 300.0, 800.0, 100.0, 100.0, false);
    rect(&mut s, 50.0, 50.0, 100.0, 100.0, "#00FF00", Some(open));
    assert_eq!(size(&mut s, open), (150, 150), "a frame that doesn't clip exports the content outside it too");
    // Hidden layers paint nothing and take no room.
    let holder = frame(&mut s, 600.0, 800.0, 100.0, 100.0, false);
    let hidden = rect(&mut s, 50.0, 50.0, 200.0, 200.0, "#00FF00", Some(holder));
    set(&mut s, &[hidden], json!({"visible": false}));
    assert_eq!(size(&mut s, holder), (100, 100), "a hidden layer doesn't make the export larger");
    let lone = export(&mut s, json!({"ids": [hidden]}));
    assert!(pixels(&lone).pixels().all(|p| p[3] == 0), "docs silent: exporting a hidden layer gives an empty image of its size");
    // Rotation: the export is the rotated layer's upright bounding box.
    let rot = rect(&mut s, 0.0, 1200.0, 100.0, 50.0, "#FF0000", None);
    set(&mut s, &[rot], json!({"rotation": 90}));
    assert_eq!(size(&mut s, rot), (50, 100), "a layer turned 90° exports its upright bounds");
    let rot45 = rect(&mut s, 300.0, 1200.0, 100.0, 100.0, "#FF0000", None);
    set(&mut s, &[rot45], json!({"rotation": 45}));
    assert_eq!(size(&mut s, rot45), (142, 142), "a square turned 45° exports its diagonal (141.4, rounded up)");
    // Sub-pixel bounds round outwards to whole pixels at the export scale (docs silent).
    let sub = rect(&mut s, 600.5, 1200.25, 10.2, 10.2, "#FF0000", None);
    assert_eq!(size(&mut s, sub), (11, 11), "docs silent: 10.2 rounds up to 11");
    assert_eq!(png_size(&export(&mut s, json!({"ids": [sub], "scale": 2}))), (21, 21), "docs silent: at 2x 20.4 rounds up to 21");
    assert!(pixels(&export(&mut s, json!({"ids": [sub]}))).get_pixel(0, 0)[3] > 200, "the image starts at the layer's own edge");
}

#[test]
fn masks_and_slices_export_what_shows() {
    let mut s = Session::new();
    // A 50×50 circle masking a 200×200 square: the group exports what the mask shows.
    let m = id(&ex(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 50, "height": 50})));
    let big = rect(&mut s, 0.0, 0.0, 200.0, 200.0, "#FF0000", None);
    let g = id(&ex(&mut s, "object.mask", json!({"ids": [m, big]})));
    assert_eq!(png_size(&export(&mut s, json!({"ids": [g]}))), (50, 50), "a mask limits what shows, so the export is the mask's area");
    // A slice exports everything visible in its area, from any layer.
    let under = rect(&mut s, 300.0, 0.0, 100.0, 100.0, "#00FF00", None);
    let slice = id(&ex(&mut s, "node.create", json!({"type": "slice", "x": 325, "y": 25, "width": 50, "height": 50})));
    let png = export(&mut s, json!({"ids": [slice]}));
    assert_eq!(png_size(&png), (50, 50), "a slice exports its own area");
    assert_eq!(pixels(&png).get_pixel(25, 25).0, [0, 255, 0, 255], "a slice exports the layers under it");
    let _ = under;
}

/// Ignore overlapping layers (contents only) on a slice: inside a frame or group the slice exports
/// only what is in that container; off, or on the page, everything in its area.
#[test]
fn a_slice_in_a_frame_or_group_exports_only_that_container() {
    let mut s = Session::new();
    let slice_in = |s: &mut Session, parent: Option<NodeId>, x: f64| {
        let mut p = json!({"type": "slice", "x": x, "y": 25, "width": 50, "height": 50});
        if let Some(parent) = parent {
            p["parent"] = json!(parent);
        }
        id(&ex(s, "node.create", p))
    };
    let at = |s: &mut Session, p: Value| pixels(&export(s, p)).get_pixel(40, 40).0;
    // A frame holding a red square and a slice, with a blue square on the page above it.
    let f = frame(&mut s, 0.0, 0.0, 100.0, 100.0, false);
    rect(&mut s, 0.0, 0.0, 100.0, 100.0, "#FF0000", Some(f));
    let in_frame = slice_in(&mut s, Some(f), 25.0);
    rect(&mut s, 50.0, 50.0, 100.0, 100.0, "#0000FF", None);
    assert_eq!(png_size(&export(&mut s, json!({"ids": [in_frame]}))), (50, 50), "the slice still exports its own area");
    assert_eq!(at(&mut s, json!({"ids": [in_frame]})), [255, 0, 0, 255], "on by default: only the frame's content, the blue square left out");
    assert_eq!(at(&mut s, json!({"ids": [in_frame], "contentsOnly": false})), [0, 0, 255, 255], "off: everything in the slice's area");
    // The same in a group.
    let red = rect(&mut s, 300.0, 0.0, 100.0, 100.0, "#FF0000", None);
    let grouped = slice_in(&mut s, None, 325.0);
    let g = id(&ex(&mut s, "object.group", json!({"ids": [red, grouped]})));
    assert_eq!(s.doc().unwrap().node(grouped).unwrap().parent, Some(g));
    rect(&mut s, 350.0, 50.0, 100.0, 100.0, "#0000FF", None);
    assert_eq!(at(&mut s, json!({"ids": [grouped]})), [255, 0, 0, 255], "in a group: only the group's content");
    assert_eq!(at(&mut s, json!({"ids": [grouped], "contentsOnly": false})), [0, 0, 255, 255], "off: everything in its area");
    // On the page the option has no effect.
    rect(&mut s, 600.0, 0.0, 100.0, 100.0, "#FF0000", None);
    let top = slice_in(&mut s, None, 625.0);
    rect(&mut s, 650.0, 50.0, 100.0, 100.0, "#0000FF", None);
    assert_eq!(at(&mut s, json!({"ids": [top]})), [0, 0, 255, 255], "on the page: everything in its area, with the option on");
    assert_eq!(at(&mut s, json!({"ids": [top], "contentsOnly": false})), [0, 0, 255, 255], "on the page: everything in its area, with it off");
}

#[test]
fn several_layers_and_frames_to_pdf() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 40.0, "#FF0000", None);
    let b = rect(&mut s, 100.0, 0.0, 20.0, 20.0, "#0000FF", None);
    name(&mut s, a, "A");
    name(&mut s, b, "B");
    ex(&mut s, "export.add", json!({"ids": [a, b]}));
    ex(&mut s, "export.add", json!({"ids": [b], "setting": {"size": "1x", "format": "svg"}}));
    let r = ex(&mut s, "file.exportLayers", json!({"ids": [a, b]}));
    assert_eq!(file_names(&r), ["A.png", "B.png", "B.svg"], "several selected layers export one file per layer and setting");
    // docs silent: one-off `file.export` of a selection draws the layers into one file covering them all.
    assert_eq!(png_size(&export(&mut s, json!({"ids": [a, b]}))), (120, 40), "docs silent: a one-off export of several layers is one image");
    // Export frames to PDF: one page per visible top-level frame, in reading order.
    let f1 = frame(&mut s, 0.0, 500.0, 100.0, 100.0, true);
    let f2 = frame(&mut s, 200.0, 500.0, 100.0, 100.0, true);
    let f3 = frame(&mut s, 400.0, 500.0, 100.0, 100.0, true);
    set(&mut s, &[f3], json!({"visible": false}));
    let r = ex(&mut s, "file.exportFramesPdf", json!({}));
    assert_eq!(r["pages"], 2, "every visible top-level frame is one page; other layers and hidden frames are left out");
    // A PDF of several frames from the export settings: one page each.
    let pdf = export(&mut s, json!({"ids": [f1, f2], "format": "pdf"}));
    let raw = String::from_utf8_lossy(&pdf);
    assert_eq!(raw.matches("/Type/Page").count() - raw.matches("/Type/Pages").count(), 2, "frames exported to PDF get a page each");
}

#[test]
fn svg_export_options() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 40.0, "#FF0000", None);
    let b = rect(&mut s, 50.0, 0.0, 40.0, 40.0, "#FF0000", None);
    name(&mut s, a, "Card");
    name(&mut s, b, "Card");
    let plain = text(&export(&mut s, json!({"ids": [a, b], "format": "svg"})));
    assert!(!plain.contains("id=\"Card"), "the id attribute is off by default");
    let ids = text(&export(&mut s, json!({"ids": [a, b], "format": "svg", "svgIdAttribute": true})));
    assert!(ids.contains("id=\"Card\""), "include id attribute names elements after their layers");
    assert!(ids.contains("id=\"Card_2\""), "docs silent: ids are unique in the file, so a repeated name gets a number");
    // Text: outlined by default; kept as text when outlining is off.
    let t = id(&ex(&mut s, "text.create", json!({"text": "Hello", "x": 0, "y": 100})));
    let outlined = text(&export(&mut s, json!({"ids": [t], "format": "svg"})));
    assert!(!outlined.contains("<text"), "outline text is on by default: text becomes paths");
    let live = text(&export(&mut s, json!({"ids": [t], "format": "svg", "svgOutlineText": false})));
    // Without a font nothing lays out, so there is no text to keep (CI has no fonts).
    let has_glyphs = figcraft_text::layout_node(s.doc().unwrap().node(t).unwrap()).is_some_and(|(l, _)| !l.glyphs.is_empty());
    if has_glyphs {
        assert!(live.contains("<text") && live.contains("Hello"), "with outline text off, text stays text (selectable, needs the font)");
    }
    // Strokes: simplified to stroke attributes by default.
    let st = rect(&mut s, 0.0, 200.0, 40.0, 40.0, "#FF0000", None);
    set(&mut s, &[st], json!({"strokes": ["#000000"], "strokeWeight": 4, "strokeAlign": "center"}));
    let simple = text(&export(&mut s, json!({"ids": [st], "format": "svg"})));
    assert!(simple.contains("stroke-width=\"4\""), "simplify stroke is on by default: a stroke attribute where it can be");
    let exact = text(&export(&mut s, json!({"ids": [st], "format": "svg", "svgSimplifyStroke": false})));
    assert!(!exact.contains("stroke-width"), "with simplify stroke off, strokes are exact outlines");
    let inside = rect(&mut s, 100.0, 200.0, 40.0, 40.0, "#FF0000", None);
    set(&mut s, &[inside], json!({"strokes": ["#000000"], "strokeWeight": 4, "strokeAlign": "inside", "cornerRadius": 8}));
    let svg = text(&export(&mut s, json!({"ids": [inside], "format": "svg"})));
    assert!(svg.contains("stroke-width=\"4\""), "an inside stroke on a rectangle simplifies too (the rectangle inset by half the weight)");
}

#[test]
fn copy_as_png_svg_and_code() {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 20.0, "#FF0000", None);
    let b = rect(&mut s, 60.0, 0.0, 40.0, 20.0, "#0000FF", None);
    let doc = s.doc().unwrap();
    let png = crate::cmd::file::copy_png(doc, &[a, b]).unwrap();
    assert_eq!((png.width(), png.height()), (200, 40), "docs silent: Copy as PNG copies the selection at 2x, all of it in one image");
    let svg = crate::cmd::file::export_bytes(doc, &[a, b], figcraft_doc::ExportFormat::Svg, 1.0).unwrap();
    assert!(text(&svg).starts_with("<svg width=\"100\""), "Copy as SVG is the selection's SVG export");
    let css = crate::codegen::code_for(doc, a, crate::codegen::CodeLang::Css);
    assert!(css.contains("background: #FF0000;"), "Copy as CSS is the layer's CSS from Inspect");
}
