//! Paint and effects behaviour through commands (the rules of `figcraft_render`'s paint and
//! effects suites that involve edits or exports): the eyedropper, blend modes and effects set
//! through commands, and how the SVG and PDF exports carry isolating groups and shadows that hide
//! behind their layer.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, ToolId};
use figcraft_color::BlendMode;
use figcraft_doc::{EffectKind, NodeId};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, parent: Option<NodeId>, (x, y, w, h): (f64, f64, f64, f64), fill: &str) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h, "props": {"fill": fill}});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(s.execute("shape.rectangle", &p).unwrap())
}

fn set(s: &mut Session, node: NodeId, props: Value) {
    s.execute("node.set", &json!({"id": node, "props": props})).unwrap();
}

fn svg(s: &Session, ids: &[NodeId]) -> String {
    figcraft_svg::export_svg(s.doc().unwrap(), ids, &figcraft_svg::SvgOptions::default()).unwrap()
}

#[test]
fn the_eyedropper_picks_the_colour_seen_and_fills_the_selection() {
    // Documented: the eyedropper picks the colour under the pointer as it looks on the canvas and
    // applies it to the selected layer's fill (opaque: the colour seen, not a layer's alpha).
    let mut s = Session::new();
    s.execute("page.setBackground", &json!({"color": "#FFFFFF"})).unwrap();
    let a = rect(&mut s, None, (0.0, 0.0, 100.0, 100.0), "#FF0000");
    set(&mut s, a, json!({"opacity": 0.5}));
    let b = rect(&mut s, None, (200.0, 0.0, 50.0, 50.0), "#00FF00");
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("edit.pickColor", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 50.0, 50.0, 1.0)).unwrap();
    let fill = s.doc().unwrap().node(b).unwrap().fills[0].solid_color().unwrap();
    let [r, g, bl, al] = fill.to_rgba8();
    assert!(r == 255 && g.abs_diff(128) <= 1 && bl.abs_diff(128) <= 1 && al == 255, "picked {}", fill.to_css_hex());
    assert_eq!(s.recent_colors.first(), Some(&fill), "the picked colour joins the recent colours");
    assert_eq!(s.tool.tool, ToolId::Move, "picking ends the eyedropper");
    // One undo step takes the fill back.
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().node(b).unwrap().fills[0].solid_color().map(|c| c.to_hex()), Some("00FF00".into()));
}

#[test]
fn the_eyedropper_of_a_colour_picker_samples_without_filling() {
    // Opened from a colour picker (`apply: false`), the eyedropper hands the colour to the picker,
    // which writes it to its own paint (a stroke, an effect...): the fill stays as it was.
    let mut s = Session::new();
    s.execute("page.setBackground", &json!({"color": "#FFFFFF"})).unwrap();
    rect(&mut s, None, (0.0, 0.0, 100.0, 100.0), "#FF0000");
    let b = rect(&mut s, None, (200.0, 0.0, 50.0, 50.0), "#00FF00");
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let undo = s.undo_depth().0;
    s.execute("edit.pickColor", &json!({"apply": false})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 50.0, 50.0, 1.0)).unwrap();
    assert_eq!(s.doc().unwrap().node(b).unwrap().fills[0].solid_color().map(|c| c.to_hex()), Some("00FF00".into()));
    assert_eq!(s.undo_depth().0, undo, "nothing to undo");
    assert_eq!(s.tool.picked.take().map(|c| c.to_hex()), Some("FF0000".into()));
    assert_eq!(s.tool.tool, ToolId::Move, "picking ends the eyedropper");
    // The I key's eyedropper fills the selection again.
    s.execute("edit.pickColor", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 50.0, 50.0, 1.0)).unwrap();
    assert_eq!(s.doc().unwrap().node(b).unwrap().fills[0].solid_color().map(|c| c.to_hex()), Some("FF0000".into()));
    assert_eq!(s.tool.picked, None);
}

#[test]
fn blend_modes_and_effects_through_commands() {
    let mut s = Session::new();
    let r = rect(&mut s, None, (0.0, 0.0, 40.0, 40.0), "#CC6633");
    s.execute("fill.add", &json!({"ids": [r], "paint": "#66CCEE"})).unwrap();
    s.execute("fill.set", &json!({"ids": [r], "index": 1, "blend": "plusDarker", "opacity": 50})).unwrap();
    let n = s.doc().unwrap().node(r).unwrap();
    assert_eq!((n.fills[1].blend, n.fills[1].opacity), (BlendMode::PlusDarker, 0.5));
    // Inspecting the layer reports the paint's blend mode.
    let fills = s.execute("node.get", &json!({"id": r})).unwrap()["fills"].clone();
    assert_eq!((fills[0].get("blend"), fills[1]["blend"].as_str()), (None, Some("plusDarker")));
    // A new group passes through; Normal is a choice.
    let g = id(s.execute("object.group", &json!({"ids": [r]})).unwrap());
    assert_eq!(s.doc().unwrap().node(g).unwrap().blend, BlendMode::PassThrough);
    set(&mut s, g, json!({"blend": "normal"}));
    assert!(s.doc().unwrap().node(g).unwrap().isolates_children());
    // A new drop shadow hides behind its layer; the option turns that off.
    s.execute("effect.add", &json!({"ids": [r], "type": "dropShadow"})).unwrap();
    let shown = |s: &Session| match &s.doc().unwrap().node(r).unwrap().effects[0].kind {
        EffectKind::DropShadow(sh) => sh.show_behind,
        _ => panic!("not a shadow"),
    };
    assert!(!shown(&s));
    s.execute("effect.set", &json!({"ids": [r], "index": 0, "effect": {"type": "dropShadow", "showBehind": true}})).unwrap();
    assert!(shown(&s));
}

#[test]
fn exports_isolate_normal_groups_and_hide_shadows_behind_their_layer() {
    let mut s = Session::new();
    let g = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let k = rect(&mut s, Some(g), (10.0, 10.0, 50.0, 50.0), "#00FFFF");
    set(&mut s, k, json!({"blend": "multiply"}));
    // Pass-through: no isolation; Normal: an isolated group, in SVG and PDF.
    let out = svg(&s, &[g]);
    assert!(!out.contains("isolation:isolate"));
    // The frame clips (by default) without a clipped group around its children, which would
    // isolate them: the blending child carries the clip itself.
    assert!(!out.contains("<g clip-path") && out.contains("mix-blend-mode:multiply\" clip-path=\"url(#"), "{out}");
    set(&mut s, g, json!({"blend": "normal"}));
    assert!(svg(&s, &[g]).contains("isolation:isolate"));
    assert!(figcraft_pdf::export_nodes_pdf(s.doc().unwrap(), &[g]).is_ok());
    // A translucent layer's drop shadow is cut out by what the layer covers: the shadow's
    // filter composites it "out" of an image of the layer's outline.
    let r = rect(&mut s, None, (200.0, 0.0, 40.0, 40.0), "#FF0000");
    set(&mut s, r, json!({"effects": [{"type": "dropShadow", "offset": [10, 0], "radius": 0, "spread": 0}]}));
    s.execute("fill.set", &json!({"ids": [r], "opacity": 50})).unwrap();
    let hidden = svg(&s, &[r]);
    assert!(hidden.contains("result=\"cover\"") && hidden.contains("in2=\"cover\" operator=\"out\""), "{hidden}");
    // Shown behind transparent areas: no cut-out.
    set(&mut s, r, json!({"effects": [{"type": "dropShadow", "offset": [10, 0], "radius": 0, "spread": 0, "showBehind": true}]}));
    assert!(!svg(&s, &[r]).contains("operator=\"out\""));
    // With spread (drawn as geometry), the cut-out goes into its blur filter.
    set(&mut s, r, json!({"effects": [{"type": "dropShadow", "offset": [10, 0], "radius": 4, "spread": 3}]}));
    assert!(svg(&s, &[r]).contains("in2=\"cover\" operator=\"out\""));
    // A group (no one outline) cuts its shadow out by its pixels.
    let grp = id(s.execute("object.group", &json!({"ids": [r]})).unwrap());
    set(&mut s, r, json!({"effects": []}));
    set(&mut s, grp, json!({"effects": [{"type": "dropShadow", "offset": [10, 0], "radius": 0, "spread": 0}]}));
    assert!(svg(&s, &[grp]).contains("in2=\"SourceAlpha\" operator=\"out\""));
    assert!(figcraft_pdf::export_nodes_pdf(s.doc().unwrap(), &[grp]).is_ok());
    // The shadow's filter region reaches as far as the shadow (a share of a small layer's box
    // would cut it off).
    let small = rect(&mut s, None, (400.0, 0.0, 10.0, 10.0), "#000000");
    set(&mut s, small, json!({"effects": [{"type": "dropShadow", "offset": [30, 0], "radius": 4, "spread": 0}]}));
    let out = svg(&s, &[small]);
    let width = out
        .split("filterUnits=\"userSpaceOnUse\"")
        .nth(1)
        .and_then(|t| t.split("width=\"").nth(1))
        .and_then(|t| t.split('"').next())
        .and_then(|w| w.parse::<f64>().ok());
    assert!(width.is_some_and(|w| w >= 10.0 + 30.0 + 6.0), "{out}");
}
