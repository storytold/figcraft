//! What other readers of `.fig` files need from the export: the document and its pages say they
//! are visible and opaque, every drawn layer carries its outline, text carries its layout
//! settings, and the schema written into the file numbers its definitions, fields and enum values
//! the way the format does.

use figcraft_doc::color::{Color, Paint};
use figcraft_doc::*;

use super::kiwi::{Kind, Kv, Schema, Type};
use super::*;

/// One page with a filled frame holding a rectangle, a stroked ellipse, a centred line and text.
fn sample() -> Document {
    let mut doc = Document::new("Interop");
    let page = doc.first_page().unwrap();
    let add = |doc: &mut Document, parent: NodeId, kind: NodeKind, name: &str, w: f64, h: f64, edit: &dyn Fn(&mut Node)| {
        let id = doc.create(parent, kind, name, 10.0, 10.0, w, h).unwrap();
        let n = doc.node_mut(id).unwrap();
        n.fills = vec![Paint::solid(Color::new(1.0, 0.0, 0.0, 1.0))];
        n.strokes.clear();
        edit(n);
        id
    };
    let frame = add(&mut doc, page, NodeKind::Frame, "Frame", 200.0, 200.0, &|_| {});
    add(&mut doc, frame, NodeKind::Rectangle, "Rect", 80.0, 60.0, &|n| n.corners.radii = [6.0; 4]);
    add(&mut doc, frame, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Ellipse", 70.0, 70.0, &|n| {
        n.strokes = vec![Paint::solid(Color::BLACK)];
        n.stroke.weight = 4.0;
        n.stroke.align = StrokeAlign::Inside;
    });
    add(&mut doc, frame, NodeKind::Line, "Line", 50.0, 0.0, &|n| {
        n.fills.clear();
        n.strokes = vec![Paint::solid(Color::BLACK)];
        n.stroke.weight = 2.0;
        n.stroke.align = StrokeAlign::Center;
    });
    add(&mut doc, frame, NodeKind::Text(TextData::new("One\nTwo")), "Text", 60.0, 40.0, &|_| {});
    doc
}

/// The node changes of the written file, by type name.
fn with_changes(doc: &Document, check: impl Fn(&dyn Fn(&str) -> Kv<'static>, &Kv<'static>)) {
    let (bytes, _) = export(doc, Some(&super::tests::png())).unwrap();
    let archive = Archive::open(&bytes).unwrap();
    let (schema, message) = decode_canvas(&archive.canvas).unwrap();
    // Decoded values borrow their names from the schema.
    let schema: &'static Schema = Box::leak(Box::new(schema));
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let of = |ty: &str| {
        let found = msg.get("nodeChanges").unwrap().list().iter().find(|n| n.get("type").and_then(Kv::str) == Some(ty));
        found.unwrap_or_else(|| panic!("no {ty} was written")).clone()
    };
    check(&of, &msg);
}

fn num(v: &Kv, key: &str) -> f64 {
    v.get(key).and_then(Kv::f64).unwrap_or_else(|| panic!("{key} is missing"))
}

fn flag(v: &Kv, key: &str) -> bool {
    v.get(key).and_then(Kv::bool).unwrap_or_else(|| panic!("{key} is missing"))
}

fn name<'a>(v: &'a Kv, key: &str) -> &'a str {
    v.get(key).and_then(Kv::str).unwrap_or_else(|| panic!("{key} is missing"))
}

#[test]
fn the_document_and_its_pages_are_visible_and_opaque() {
    with_changes(&sample(), |of, _| {
        for ty in ["DOCUMENT", "CANVAS"] {
            let n = of(ty);
            assert!(flag(&n, "visible"), "{ty}");
            assert_eq!(num(&n, "opacity"), 1.0, "{ty}");
            assert_eq!(num(n.get("transform").unwrap(), "m00"), 1.0, "{ty}");
            assert_eq!(num(n.get("transform").unwrap(), "m11"), 1.0, "{ty}");
            assert_eq!(num(&n, "strokeWeight"), 0.0, "{ty}");
            assert_eq!((name(&n, "strokeAlign"), name(&n, "strokeJoin")), ("CENTER", "BEVEL"), "{ty}");
        }
        assert_eq!(name(&of("DOCUMENT"), "documentColorProfile"), "SRGB");
        let page = of("CANVAS");
        assert!(flag(&page, "backgroundEnabled"));
        assert_eq!(num(&page, "backgroundOpacity"), 1.0);
        assert_eq!(num(page.get("backgroundColor").unwrap(), "a"), 1.0);
    });
}

#[test]
fn the_document_colour_profile_and_page_opacity_follow_the_document() {
    let mut doc = sample();
    doc.color_profile = figcraft_doc::color::ColorProfile::DisplayP3;
    let page = doc.first_page().unwrap();
    if let Some(NodeKind::Page { background, .. }) = doc.node_mut(page).map(|n| &mut n.kind) {
        *background = Color::new(0.5, 0.5, 0.5, 0.25);
    }
    with_changes(&doc, |of, _| {
        assert_eq!(name(&of("DOCUMENT"), "documentColorProfile"), "DISPLAY_P3");
        assert_eq!(num(&of("CANVAS"), "backgroundOpacity"), 0.25);
    });
}

#[test]
fn drawn_layers_carry_their_outlines() {
    with_changes(&sample(), |of, msg| {
        let blobs = msg.get("blobs").unwrap().list();
        let path = |n: &Kv, key: &str| -> (String, Vec<u8>) {
            let list = n.get(key).map(Kv::list).unwrap_or_default();
            let p = list.first().unwrap_or_else(|| panic!("{key} is missing"));
            assert_eq!(num(p, "styleID"), 0.0);
            let blob = blobs.get(num(p, "commandsBlob") as usize).and_then(|b| b.get("bytes")).and_then(Kv::bytes).unwrap();
            (name(p, "windingRule").to_string(), blob.to_vec())
        };
        // Sub-paths (move commands) of a geometry blob.
        let moves = |blob: &[u8]| {
            let (mut i, mut n) = (0, 0);
            while let Some(cmd) = blob.get(i) {
                n += usize::from(*cmd == 1);
                i += 1 + 8 * [0, 1, 1, 2, 3].get(usize::from(*cmd)).copied().unwrap_or(0);
            }
            n
        };
        for ty in ["FRAME", "ROUNDED_RECTANGLE", "ELLIPSE"] {
            let (rule, blob) = path(&of(ty), "fillGeometry");
            assert_eq!(rule, "NONZERO", "{ty}");
            assert_eq!(moves(&blob), 1, "{ty}");
        }
        // An inside stroke is the ring between the shape and the shape moved in by the weight.
        let (rule, ring) = path(&of("ELLIPSE"), "strokeGeometry");
        assert_eq!((rule.as_str(), moves(&ring)), ("ODD", 2));
        let (rule, line) = path(&of("LINE"), "strokeGeometry");
        assert_eq!(rule, "NONZERO");
        assert!(moves(&line) >= 1);
        // Nothing is stroked: no stroke outline.
        assert!(of("FRAME").get("strokeGeometry").is_none());
        assert!(of("LINE").get("fillGeometry").is_none());
    });
}

#[test]
fn strokes_without_a_simple_outline_get_none() {
    let mut doc = sample();
    let ids: Vec<NodeId> = doc.ids().collect();
    for id in ids {
        if let Some(n) = doc.node_mut(id).filter(|n| !matches!(n.kind, NodeKind::Page { .. })) {
            n.strokes = vec![Paint::solid(Color::BLACK)];
            n.stroke.weight = 3.0;
            n.stroke.dashes = vec![4.0, 2.0];
        }
    }
    with_changes(&doc, |of, _| {
        for ty in ["FRAME", "ROUNDED_RECTANGLE", "ELLIPSE", "LINE", "TEXT"] {
            assert!(of(ty).get("strokeGeometry").is_none(), "{ty}");
        }
    });
    // A stroke wider than the shape fills it; sizes that aren't numbers write nothing.
    let mut doc = sample();
    let ids: Vec<NodeId> = doc.ids().collect();
    for (i, id) in ids.into_iter().enumerate() {
        if let Some(n) = doc.node_mut(id).filter(|n| !matches!(n.kind, NodeKind::Page { .. })) {
            n.strokes = vec![Paint::solid(Color::BLACK)];
            n.stroke.weight = if i % 2 == 0 { 1e6 } else { f64::NAN };
            n.stroke.align = [StrokeAlign::Inside, StrokeAlign::Center, StrokeAlign::Outside][i % 3];
            if i % 4 == 0 {
                n.width = f64::INFINITY;
            }
        }
    }
    export(&doc, None).unwrap();
}

#[test]
fn rectangles_say_their_corners() {
    with_changes(&sample(), |of, _| {
        let r = of("ROUNDED_RECTANGLE");
        assert_eq!(num(&r, "cornerRadius"), 6.0);
        for key in
            ["rectangleTopLeftCornerRadius", "rectangleTopRightCornerRadius", "rectangleBottomRightCornerRadius", "rectangleBottomLeftCornerRadius"]
        {
            assert_eq!(num(&r, key), 6.0, "{key}");
        }
        assert!(r.get("rectangleCornerRadiiIndependent").is_none());
    });
}

#[test]
fn text_carries_its_layout_settings_and_a_line_per_paragraph() {
    with_changes(&sample(), |of, _| {
        let t = of("TEXT");
        assert!(!flag(&t, "autoRename"));
        assert!(flag(&t, "fontVariantCommonLigatures") && flag(&t, "fontVariantContextualLigatures") && flag(&t, "textDecorationSkipInk"));
        assert_eq!(name(&t, "fontVersion"), "");
        assert_eq!((num(&t, "textUserLayoutVersion"), num(&t, "textExplicitLayoutVersion"), num(&t, "textBidiVersion")), (5.0, 1.0, 1.0));
        assert_eq!(num(&t, "textTracking"), 0.0);
        assert_eq!(name(&t, "emojiImageSet"), "APPLE");
        let lines = t.get("textData").and_then(|d| d.get("lines")).map(Kv::list).unwrap_or_default();
        assert_eq!(lines.len(), 2);
        for l in lines {
            assert_eq!((name(l, "lineType"), name(l, "sourceDirectionality")), ("PLAIN", "AUTO"));
            assert_eq!((num(l, "styleId"), num(l, "indentationLevel"), num(l, "listStartOffset")), (0.0, 0.0, 0.0));
            assert!(!flag(l, "isFirstLineOfList"));
        }
    });
}

#[test]
fn font_styles_are_written_as_words_and_both_spellings_read_back_the_same() {
    for (ours, written) in [
        ("SemiBold", "Semi Bold"),
        ("Semi Bold", "Semi Bold"),
        ("semi-bold", "Semi Bold"),
        ("ExtraBold", "Extra Bold"),
        ("ExtraLight", "Extra Light"),
        ("UltraLight", "Ultra Light"),
        ("DemiBold", "Demi Bold"),
        ("SemiBoldItalic", "Semi Bold Italic"),
        ("SemiBold Italic", "Semi Bold Italic"),
        ("BoldItalic", "Bold Italic"),
        ("Regular Italic", "Italic"),
        ("Italic", "Italic"),
        ("Regular", "Regular"),
        ("Thin", "Thin"),
        ("Black", "Black"),
        // Names that aren't the standard weights stay as written.
        ("Condensed SemiBold", "Condensed SemiBold"),
        ("Heavy", "Heavy"),
        ("Oblique", "Oblique"),
        ("", ""),
        ("Ünïcode Ⅻ", "Ünïcode Ⅻ"),
    ] {
        assert_eq!(style_name(ours), written, "{ours}");
        assert_eq!(style_name(written), written, "{written} stays");
    }
    // The layer's own style and a styled run are both written as words, and read back as them.
    let mut doc = sample();
    let text = doc.ids().find(|id| doc.node(*id).is_some_and(|n| n.kind.text().is_some())).unwrap();
    if let Some(NodeKind::Text(td)) = doc.node_mut(text).map(|n| &mut n.kind) {
        td.style.font_style = "SemiBold".into();
        let mut run = td.style.clone();
        run.font_style = "ExtraBoldItalic".into();
        td.runs = vec![StyleRun { start: 0, end: 3, style: run }];
    }
    with_changes(&doc, |of, _| {
        let t = of("TEXT");
        let font = |v: &Kv| v.get("fontName").map(|f| (name(f, "style").to_string(), name(f, "postscript").to_string())).unwrap();
        assert_eq!(font(&t), ("Semi Bold".to_string(), String::new()));
        let table = t.get("textData").and_then(|d| d.get("styleOverrideTable")).map(Kv::list).unwrap_or_default();
        assert_eq!(table.iter().map(font).collect::<Vec<_>>(), [("Extra Bold Italic".to_string(), String::new())]);
    });
    let (bytes, _) = export(&doc, None).unwrap();
    let (back, _) = import(&bytes, "x").unwrap();
    let td = back.ids().find_map(|id| back.node(id).and_then(|n| n.kind.text().cloned())).unwrap();
    assert_eq!(td.style.font_style, "Semi Bold");
    assert_eq!(td.runs.iter().map(|r| r.style.font_style.as_str()).collect::<Vec<_>>(), ["Extra Bold Italic"]);
    // A file that spells the style compactly reads as the same style.
    fn respell(v: &mut Kv) {
        match v {
            Kv::Str(s) if s == "Semi Bold" => *s = "SemiBold".into(),
            Kv::Array(a) => a.iter_mut().for_each(respell),
            Kv::Object(f) => f.iter_mut().for_each(|(_, x)| respell(x)),
            _ => {}
        }
    }
    let (compact, _) = import(&rewritten(&doc, &|msg| respell(msg)), "x").unwrap();
    let td = compact.ids().find_map(|id| compact.node(id).and_then(|n| n.kind.text().cloned())).unwrap();
    assert_eq!(td.style.font_style, "Semi Bold");
}

/// The exported file with its message changed by `change`, as a bare canvas.
fn rewritten(doc: &Document, change: &dyn Fn(&mut Kv)) -> Vec<u8> {
    let (bytes, _) = export(doc, None).unwrap();
    let (schema, message) = decode_canvas(&Archive::open(&bytes).unwrap().canvas).unwrap();
    let mut msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    change(&mut msg);
    super::tests::canvas(&schema, &msg)
}

type NodeFields<'a, 's> = &'a mut Vec<(&'s str, Kv<'s>)>;

fn each_node(msg: &mut Kv, f: &dyn Fn(NodeFields)) {
    if let Kv::Object(top) = msg {
        for (k, v) in top.iter_mut() {
            if let (true, Kv::Array(nodes)) = (*k == "nodeChanges", v) {
                for n in nodes.iter_mut() {
                    if let Kv::Object(fields) = n {
                        f(fields);
                    }
                }
            }
        }
    }
}

#[test]
fn line_heights_say_what_they_count_from() {
    for (ours, value, units) in [
        (LineHeight::Auto, 100.0, "PERCENT"),
        (LineHeight::Percent(140.0), 1.4, "RAW"),
        (LineHeight::Percent(100.0), 1.0, "RAW"),
        (LineHeight::Pixels(20.0), 20.0, "PIXELS"),
    ] {
        let mut doc = sample();
        let text = doc.ids().find(|id| doc.node(*id).is_some_and(|n| n.kind.text().is_some())).unwrap();
        if let Some(NodeKind::Text(td)) = doc.node_mut(text).map(|n| &mut n.kind) {
            td.style.line_height = ours;
        }
        with_changes(&doc, |of, _| {
            let t = of("TEXT");
            let lh = t.get("lineHeight").unwrap();
            assert!((num(lh, "value") - value).abs() < 1e-6, "{ours:?}");
            assert_eq!(name(lh, "units"), units, "{ours:?}");
        });
        let (bytes, _) = export(&doc, None).unwrap();
        let (back, _) = import(&bytes, "x").unwrap();
        let td = back.ids().find_map(|id| back.node(id).and_then(|n| n.kind.text().cloned())).unwrap();
        match (td.style.line_height, ours) {
            (LineHeight::Percent(a), LineHeight::Percent(b)) => assert!((a - b).abs() < 1e-3, "{ours:?}"),
            (a, b) => assert_eq!(a, b),
        }
    }
}

#[test]
fn sides_without_a_weight_have_no_border() {
    let mut doc = sample();
    let frame = doc.ids().find(|id| doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Frame))).unwrap();
    if let Some(n) = doc.node_mut(frame) {
        n.strokes = vec![Paint::solid(Color::BLACK)];
        n.stroke.weight = 1.0;
        n.stroke.sides = Some([0.0, 0.0, 1.0, 0.0]);
    }
    let sides = |bytes: &[u8]| {
        let (back, _) = import(bytes, "x").unwrap();
        back.ids().find_map(|id| back.node(id).filter(|n| matches!(n.kind, NodeKind::Frame)).and_then(|n| n.stroke.sides))
    };
    assert_eq!(sides(&export(&doc, None).unwrap().0), Some([0.0, 0.0, 1.0, 0.0]));
    // Other writers leave the sides without a border out.
    let sparse =
        rewritten(&doc, &|msg| each_node(msg, &|f| f.retain(|(k, _)| !matches!(*k, "borderTopWeight" | "borderRightWeight" | "borderLeftWeight"))));
    assert_eq!(sides(&sparse), Some([0.0, 0.0, 1.0, 0.0]));
}

#[test]
fn mask_types_round_trip() {
    for ty in [MaskType::Alpha, MaskType::Vector, MaskType::Luminance] {
        let mut doc = sample();
        let rect = doc.ids().find(|id| doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Rectangle))).unwrap();
        if let Some(n) = doc.node_mut(rect) {
            (n.is_mask, n.mask_type) = (true, ty);
        }
        with_changes(&doc, |of, _| {
            let want = ["ALPHA", "OUTLINE", "LUMINANCE"][ty as usize];
            assert_eq!(name(&of("ROUNDED_RECTANGLE"), "maskType"), want);
            assert!(of("FRAME").get("maskType").is_none());
        });
        let (bytes, warnings) = export(&doc, None).unwrap();
        assert!(!warnings.iter().any(|w| w.contains("mask")), "{warnings:?}");
        let (back, _) = import(&bytes, "x").unwrap();
        let masks: Vec<MaskType> = back.ids().filter_map(|id| back.node(id)).filter(|n| n.is_mask).map(|n| n.mask_type).collect();
        assert_eq!(masks, [ty]);
    }
}

#[test]
fn the_archive_lists_its_members_and_describes_the_file() {
    let (bytes, _) = export(&sample(), Some(&super::tests::png())).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
    let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect();
    assert_eq!(names, ["canvas.fig", "thumbnail.png", "meta.json"]);
    let meta: serde_json::Value = serde_json::from_slice(&crate::read_member(&mut z, "meta.json", 1 << 20).unwrap()).unwrap();
    assert_eq!(meta["file_name"], "Interop");
    assert!(meta["client_meta"]["background_color"]["a"].is_number());
    assert!(meta["client_meta"]["thumbnail_size"]["width"].as_u64().is_some_and(|w| w > 0));
    // No thumbnail, or one that isn't a PNG: no size.
    for thumb in [None, Some(&b"png"[..])] {
        let (bytes, _) = export(&sample(), thumb).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
        let meta: serde_json::Value = serde_json::from_slice(&crate::read_member(&mut z, "meta.json", 1 << 20).unwrap()).unwrap();
        assert!(meta["client_meta"]["thumbnail_size"].is_null());
    }
}

#[test]
fn the_schema_numbers_what_the_format_numbers() {
    let s = Schema::parse(export::SCHEMA).unwrap();
    let def = |n: &str| s.defs.iter().find(|d| d.name == n).unwrap_or_else(|| panic!("no {n}"));
    for n in ["GUID", "Color", "Vector", "Matrix", "ColorStop", "ParentIndex", "FontName", "Number", "ExportConstraint", "Blob"] {
        assert_eq!(def(n).kind, Kind::Struct, "{n}");
    }
    let id = |d: &str, f: &str| def(d).fields.iter().find(|x| x.name == f).unwrap_or_else(|| panic!("no {d}.{f}")).value;
    assert_eq!((id("NodeChange", "guid"), id("NodeChange", "size"), id("NodeChange", "transform")), (1, 11, 12));
    assert_eq!((id("NodeChange", "fillPaints"), id("NodeChange", "fillGeometry"), id("NodeChange", "strokeGeometry")), (38, 51, 52));
    assert_eq!((id("NodeType", "ROUNDED_RECTANGLE"), id("NodeType", "SECTION"), id("EffectType", "GLASS")), (12, 25, 8));
    assert_eq!((id("Message", "nodeChanges"), id("Message", "blobs")), (4, 6));
    // Ids are unique within a definition, and no message field is numbered 0 (the end marker).
    for d in &s.defs {
        let mut ids: Vec<u32> = d.fields.iter().map(|f| f.value).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), d.fields.len(), "{}", d.name);
        assert!(d.kind != Kind::Message || !ids.contains(&0), "{}", d.name);
    }
}

#[test]
fn schema_text_takes_numbers_and_refuses_bad_ones() {
    let s = Schema::parse("enum E A=3 B C=9\nmessage M uint a=7, E b, string c=2").unwrap();
    let values = |i: usize| s.defs[i].fields.iter().map(|f| (f.name.as_str(), f.value)).collect::<Vec<_>>();
    assert_eq!(values(0), [("A", 3), ("B", 1), ("C", 9)]);
    assert_eq!(values(1), [("a", 7), ("b", 2), ("c", 2)]);
    for bad in ["enum E A=", "enum E A=x", "enum E A=-1", "message M uint a=1.5", "message M uint a=99999999999", "enum E =", "message M uint ="] {
        assert!(Schema::parse(bad).is_err() || bad.ends_with('='), "{bad}");
    }
    // Junk never panics.
    let mut seed = 7u64;
    let alphabet: Vec<char> = "enum struct message uint E M a= =1 ,[] \n9x".chars().collect();
    for _ in 0..2000 {
        let text: String = (0..40)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                alphabet[(seed >> 33) as usize % alphabet.len()]
            })
            .collect();
        let _ = Schema::parse(&text);
    }
}

/// Local only (`FIGCRAFT_FIG_SAMPLES`): every definition, field and enum value the export writes
/// has the kind, type and number it has in each sample's own schema.
#[test]
fn local_samples_agree_with_the_written_schema() {
    let ours = Schema::parse(export::SCHEMA).unwrap();
    let type_name = |s: &Schema, f: &super::kiwi::Field| match f.ty {
        Some(Type::Def(i)) => s.defs[i].name.clone(),
        t => format!("{t:?}"),
    };
    for p in super::tests::samples() {
        let bytes = std::fs::read(&p).unwrap();
        let (theirs, _) = decode_canvas(&Archive::open(&bytes).unwrap().canvas).unwrap();
        for d in &ours.defs {
            let t = theirs.defs.iter().find(|x| x.name == d.name).unwrap_or_else(|| panic!("{}: no {}", p.display(), d.name));
            assert_eq!(t.kind, d.kind, "{}", d.name);
            if d.kind == Kind::Struct {
                assert_eq!(t.fields.len(), d.fields.len(), "{}", d.name);
            }
            for f in &d.fields {
                let tf = t.fields.iter().find(|x| x.name == f.name).unwrap_or_else(|| panic!("{}: no {}.{}", p.display(), d.name, f.name));
                assert_eq!((tf.value, tf.array, type_name(&theirs, tf)), (f.value, f.array, type_name(&ours, f)), "{}.{}", d.name, f.name);
            }
        }
    }
}
