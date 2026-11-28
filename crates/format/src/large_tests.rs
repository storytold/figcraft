//! `document.json` at scale: a large document round-trips exactly, layer fields at their defaults
//! are left out (and come back), files that spell every field out still open, and the header is
//! read without building the document.

use figcraft_doc::color::{BlendMode, Color, Paint};
use figcraft_doc::geom::Affine;
use figcraft_doc::{
    AutoLayout, ChildLayout, Document, Effect, EffectKind, ExportSetting, Node, NodeKind, Shadow, Sizing, StrokeAlign, TextAlignH, TextData,
};

use super::*;

/// About `screens` × 25 layers of every common kind, with most layer fields off their defaults
/// somewhere.
fn large(screens: usize) -> Document {
    let mut d = Document::new("Large");
    let page = d.first_page().unwrap();
    let button = d
        .create(
            page,
            NodeKind::Component { description: "A button".into(), props: Default::default(), prop_order: Vec::new() },
            "Button",
            0.0,
            -80.0,
            96.0,
            32.0,
        )
        .unwrap();
    let label = d.create(button, NodeKind::Text(TextData::new("OK")), "Label", 8.0, 8.0, 40.0, 16.0).unwrap();
    d.get_mut(label).unwrap().plugin_data.insert("agent".into(), "note".into());
    for s in 0..screens {
        let screen = d.create(page, NodeKind::Frame, &format!("Screen {s}"), (s % 20) as f64 * 400.0, (s / 20) as f64 * 300.0, 360.0, 240.0).unwrap();
        let n = d.get_mut(screen).unwrap();
        n.clip = true;
        n.layout = Some(AutoLayout { gap: 6.0, padding: [4.0; 4], ..AutoLayout::default() });
        n.exports.push(ExportSetting::default());
        n.corners.radii = [4.0, 0.0, 4.0, 0.0];
        for k in 0..20 {
            let kind = match k % 4 {
                0 => NodeKind::Rectangle,
                1 => NodeKind::Ellipse { start: 0.5, sweep: 3.0, inner: 0.25 },
                2 => {
                    let mut t = TextData::new(format!("Item {k}\nsecond line"));
                    t.align_h = TextAlignH::Center;
                    t.paragraph_spacing = 4.0;
                    t.features.insert("liga".into(), false);
                    NodeKind::Text(t)
                }
                _ => NodeKind::Line,
            };
            let id = d.create(screen, kind, &format!("Layer {k}"), 10.0 + k as f64 * 16.0, 50.0, 14.0, 14.0).unwrap();
            let n = d.get_mut(id).unwrap();
            match k % 5 {
                0 => {
                    n.opacity = 0.5;
                    n.blend = BlendMode::Multiply;
                }
                1 => {
                    n.visible = false;
                    n.locked = true;
                }
                2 => {
                    n.transform = Affine::rotate(0.3) * n.transform;
                    n.strokes = vec![Paint::solid(Color::from_hex("336699").unwrap())];
                    n.stroke.weight = 2.0;
                    n.stroke.align = StrokeAlign::Center;
                    n.stroke.dashes = vec![2.0, 1.0];
                }
                3 => {
                    n.effects.push(Effect {
                        kind: EffectKind::DropShadow(Shadow { offset: (1.0, 2.0), radius: 3.0, ..Shadow::default() }),
                        visible: true,
                    });
                    n.child_layout = ChildLayout { sizing_h: Sizing::Fill, min_w: Some(4.0), ..ChildLayout::default() };
                    n.constrain_proportions = true;
                }
                _ => {
                    n.is_mask = k % 2 == 0;
                    n.fills.clear();
                }
            }
        }
        let props = [("Label".to_string(), json!("Go"))].into_iter().collect();
        let inst = d.create(screen, NodeKind::Instance { component: button, props }, "Button", 10.0, 200.0, 96.0, 32.0).unwrap();
        let n = d.get_mut(inst).unwrap();
        n.source = Some(button);
        n.overrides.insert("fills".into());
        n.prop_refs.insert("visible".into(), "Show".into());
        let l = d.create(inst, NodeKind::Text(TextData::new("Go")), "Label", 8.0, 8.0, 40.0, 16.0).unwrap();
        d.get_mut(l).unwrap().source = Some(label);
    }
    d
}

/// Every layer of `a` is in `b`, identical, and vice versa; the document-level fields too.
fn assert_same_doc(a: &Document, b: &Document) {
    assert_eq!(a.node_count(), b.node_count());
    for n in a.nodes() {
        assert_eq!(Some(n), b.node(n.id), "layer {} `{}`", n.id.0, n.name);
    }
    assert_eq!((&a.name, &a.pages, &a.styles, &a.variables, &a.current_page), (&b.name, &b.pages, &b.styles, &b.variables, &b.current_page));
}

#[test]
fn large_document_roundtrips() {
    let d = large(200);
    assert!(d.node_count() > 4000);
    let bytes = save_bytes(&d, None).unwrap();
    let (back, issues) = load_bytes(&bytes).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert_same_doc(&d, &back);
    // Saving what was read writes the same document again.
    let again = save_bytes(&back, None).unwrap();
    let json = |b: &[u8]| {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(b)).unwrap();
        read_member(&mut z, "document.json", MAX_MEMBER).unwrap()
    };
    assert_eq!(json(&bytes), json(&again));
    // The streamed JSON and the JSON value the document converts to hold the same document (the
    // value spells `f32`s out at `f64` precision).
    let mut streamed = Vec::new();
    write_json(&d, &mut streamed).unwrap();
    let (a, _) = from_json(std::str::from_utf8(&streamed).unwrap()).unwrap();
    let (b, _) = from_json(&serde_json::to_string(&to_json(&d).unwrap()).unwrap()).unwrap();
    assert_same_doc(&a, &b);
    assert_same_doc(&d, &a);
}

#[test]
fn defaults_are_left_out_and_come_back() {
    let mut d = Document::new("Defaults");
    let p = d.first_page().unwrap();
    let r = d.create(p, NodeKind::Rectangle, "Plain", 5.0, 0.0, 10.0, 10.0).unwrap();
    let t = d.create(p, NodeKind::Text(TextData::new("Hi")), "Text", 0.0, 20.0, 10.0, 10.0).unwrap();
    let v = to_json(&d).unwrap();
    let node = |id: figcraft_doc::NodeId| v["nodes"][id.0.to_string()].as_object().unwrap().clone();
    let plain = node(r);
    for field in [
        "visible",
        "locked",
        "opacity",
        "children",
        "strokes",
        "stroke",
        "effects",
        "corners",
        "constraints",
        "layout",
        "child_layout",
        "clip",
        "is_mask",
        "grids",
        "exports",
        "interactions",
        "overflow",
        "scroll_position",
        "styles",
        "bound",
        "var_modes",
        "constrain_proportions",
        "source",
        "overrides",
        "prop_refs",
        "plugin_data",
    ] {
        assert!(!plain.contains_key(field), "`{field}` is at its default and left out");
    }
    for field in ["id", "name", "type", "parent", "transform", "width", "height", "fills", "blend"] {
        assert!(plain.contains_key(field), "`{field}` is written");
    }
    let text = node(t);
    for field in ["runs", "paragraph_spacing", "paragraph_indent", "list", "max_lines", "features"] {
        assert!(!text.contains_key(field), "text `{field}` is at its default and left out");
    }
    assert!(text.contains_key("characters") && text.contains_key("style"));
    let (back, _) = from_json(&serde_json::to_string(&v).unwrap()).unwrap();
    assert_same_doc(&d, &back);
}

#[test]
fn files_with_every_field_spelled_out_still_open() {
    // A layer as earlier versions wrote it: every field, defaults included.
    let old = r#"{"format":"figcraft","version":1,"imageIndex":[],"name":"Old","pages":[1],"next_id":3,"current_page":1,
        "nodes":{
          "1":{"id":1,"name":"Page 1","type":"page","parent":null,"children":[2],"visible":true,"locked":false,"opacity":1.0,"blend":"passThrough","transform":[1.0,0.0,0.0,1.0,0.0,0.0],"width":0.0,"height":0.0,"fills":[],"strokes":[],"effects":[]},
          "2":{"id":2,"name":"Card background","type":"rectangle","parent":1,"children":[],"visible":true,"locked":false,"opacity":1.0,"blend":"normal","transform":[1.0,0.0,0.0,1.0,0.0,0.0],"width":160.0,"height":64.0,"fills":[{"type":"solid","color":{"r":0.93333334,"g":0.95686275,"b":1.0,"a":1.0},"opacity":1.0,"visible":true,"blend":"normal"}],"strokes":[],"stroke":{"weight":1.0,"align":"inside","sides":null,"cap_start":"none","cap_end":"none","join":"miter","miter_limit":4.0,"dashes":[]},"effects":[],"corners":{"radii":[0.0,0.0,0.0,0.0],"smoothing":0.0},"constraints":{"horizontal":"left","vertical":"top"},"layout":null,"child_layout":{"sizing_h":"fixed","sizing_v":"fixed","absolute":false,"min_w":null,"max_w":null,"min_h":null,"max_h":null,"grid_column":null,"grid_row":null,"grid_column_span":0,"grid_row_span":0},"clip":false,"is_mask":false,"grids":[],"exports":[],"interactions":[],"overflow":"none","scroll_position":"scrolls","styles":{"fill":null,"stroke":null,"effect":null,"grid":null,"text":null},"bound":{},"var_modes":{},"constrain_proportions":false,"source":null,"overrides":[],"prop_refs":{},"plugin_data":{}}
        }}"#;
    let (d, issues) = load_bytes(old.as_bytes()).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    let n: &Node = d.node(figcraft_doc::NodeId(2)).unwrap();
    assert_eq!((n.name.as_str(), n.width, n.height, n.visible, n.opacity), ("Card background", 160.0, 64.0, true, 1.0));
    assert_eq!(n.fills.len(), 1);
    // Written again, it's the short form, and reads back the same.
    let (again, _) = from_json(&serde_json::to_string(&to_json(&d).unwrap()).unwrap()).unwrap();
    assert_same_doc(&d, &again);
}

#[test]
fn the_header_is_read_on_its_own() {
    // A newer file is refused before its document is looked at, whatever the version's size.
    assert!(matches!(from_json(r#"{"nodes": "not a map", "version": 2}"#), Err(FormatError::Newer(2))));
    assert!(matches!(from_json(r#"{"version": 99999999999}"#), Err(FormatError::Newer(u32::MAX))));
    // A version that isn't a number is no version; anything but an object is no document.
    let d = serde_json::to_string(&to_json(&Document::new("x")).unwrap()).unwrap();
    let odd = d.replacen("\"version\":1", "\"version\":\"one\"", 1);
    assert!(from_json(&odd).is_ok());
    for junk in ["[]", "[2]", "3", "null", "\"doc\"", "{", ""] {
        assert!(matches!(from_json(junk), Err(FormatError::Corrupt(_))), "{junk}");
    }
    // The image index is read from the header too.
    let (_, _, index) = parse_json(&d.replacen("\"imageIndex\":[]", "\"imageIndex\":[{\"hash\":\"ab\"},{\"nope\":1}]", 1)).unwrap();
    assert_eq!(index, Some(["ab".to_string()].into_iter().collect()));
}

#[test]
fn document_json_may_unpack_in_proportion_to_its_packed_size() {
    const MB: u64 = 1024 * 1024;
    // A small archive keeps the member cap: it can't unpack into gigabytes.
    assert_eq!(document_limit(0, 0), MAX_MEMBER);
    assert_eq!(document_limit(MB, 2 * MB), MAX_MEMBER);
    // A real 900 000-layer document saved by us packs 1 477 139 810 bytes into 244 362 923, and
    // didn't open again under the 512 MB cap.
    if cfg!(not(target_pointer_width = "32")) {
        assert!(document_limit(244_362_923, 260 * MB) > 1_477_139_810);
    }
    // The packed size an archive claims counts only up to the archive's own size.
    assert_eq!(document_limit(u64::MAX, MB), MAX_MEMBER);
    // Never more than the ceiling.
    assert_eq!(document_limit(u64::MAX, u64::MAX), MAX_DOCUMENT);
}
