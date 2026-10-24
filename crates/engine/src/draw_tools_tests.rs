//! Drawing extras: text on a path (commands, resize, undo, SVG round trip) and pattern fills
//! (tiles render live from their source, copies keep their own source, exports).

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_color::PaintKind;
use figcraft_doc::{NodeId, NodeKind};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn text_path(s: &Session, t: NodeId) -> Option<figcraft_doc::TextPath> {
    match &s.doc().unwrap().node(t)?.kind {
        NodeKind::Text(td) => td.path.as_deref().cloned(),
        _ => None,
    }
}

/// The RGBA of `node` rendered at 1x, at (`x`, `y`) of its box.
fn pixel(s: &Session, node: NodeId, x: u32, y: u32) -> [u8; 4] {
    let pix = figcraft_render::render_node(s.doc().unwrap(), node, 1.0).unwrap();
    let i = ((y * pix.width() + x) * 4) as usize;
    let d = pix.data();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

#[test]
fn text_goes_onto_a_path_and_off_again() {
    let mut s = Session::new();
    let t = id(s.execute("text.create", &json!({"text": "Around", "x": 0, "y": 0})).unwrap());
    let v = id(s.execute("vector.create", &json!({"path": "M0 0 C50 -50 150 -50 200 0", "x": 40, "y": 100})).unwrap());
    // Without a path selected it says what's missing.
    assert!(s.execute("text.onPath", &json!({"ids": [t]})).is_err());
    let (vxf, vw) = s.doc().map(|d| d.node(v).map(|n| (n.transform, n.width))).unwrap().unwrap();
    let r = s.execute("text.onPath", &json!({"ids": [t, v], "start": 0.25})).unwrap();
    assert_eq!(id(r), t);
    let doc = s.doc().unwrap();
    assert!(doc.node(v).is_none(), "the path layer became the text's path");
    let n = doc.node(t).unwrap();
    assert_eq!((n.transform, n.width), (vxf, vw), "in the path's place");
    assert_eq!(s.selection(), vec![t]);
    let tp = text_path(&s, t).unwrap();
    assert_eq!((tp.start, tp.flip), (0.25, false));
    s.execute("text.pathOptions", &json!({"ids": [t], "flip": true, "start": 7})).unwrap();
    let tp = text_path(&s, t).unwrap();
    assert_eq!((tp.start, tp.flip), (1.0, true), "start clamps into 0..1");
    // Resizing scales the path with the box.
    let w = s.doc().unwrap().node(t).unwrap().width;
    s.execute("transform.resize", &json!({"ids": [t], "width": w * 2.0})).unwrap();
    let bb = figcraft_geom::Shape::bounding_box(&text_path(&s, t).unwrap().path());
    assert!((bb.width() - w * 2.0).abs() < 1.0, "{bb:?}");
    s.execute("text.removeFromPath", &json!({"ids": [t]})).unwrap();
    assert!(text_path(&s, t).is_none());
    for _ in 0..4 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert!(s.doc().unwrap().node(v).is_some() && text_path(&s, t).is_none(), "undo brings the path back");
}

#[test]
fn svg_text_paths_import_as_text_on_a_path() {
    // SVG text is parsed with Inter standing in for every family (CI has no fonts).
    if figcraft_text::find_face("Inter", "Regular").is_none() {
        eprintln!("no Inter; skipping");
        return;
    }
    let mut s = Session::new();
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><path id="p" d="M10 80 Q100 0 190 80" fill="none"/><text font-size="12"><textPath href="#p" startOffset="50%" text-anchor="middle">Curved words</textPath></text></svg>"##;
    let doc = s.doc_mut().unwrap();
    let page = doc.current_page.unwrap();
    figcraft_svg::import_svg(doc, svg, page, 0.0, 0.0, "curve").unwrap();
    let texts: Vec<_> = s.doc().unwrap().nodes().filter_map(|n| n.kind.text().cloned()).collect();
    let t = texts.iter().find(|t| t.characters == "Curved words").expect("a text layer");
    let tp = t.path.as_ref().expect("on a path");
    assert!((tp.start - 0.5).abs() < 0.01, "{}", tp.start);
    assert_eq!(t.align_h, figcraft_doc::TextAlignH::Center);
}

/// A text layer with its own green fill bound to `ink` and a layer blur, and a curve; the
/// variables `ink` and `shadow` exist.
fn text_and_curve(s: &mut Session, path_props: Value) -> (NodeId, NodeId) {
    let t = id(s.execute("text.create", &json!({"text": "Around", "x": 0, "y": 0})).unwrap());
    let v = id(s.execute("vector.create", &json!({"path": "M0 0 C50 -50 150 -50 200 0", "x": 40, "y": 100, "props": path_props})).unwrap());
    s.execute("variable.createCollection", &json!({"name": "Paint"})).unwrap();
    for (name, value) in [("ink", "#00FF00"), ("shadow", "#FF000080")] {
        s.execute("variable.create", &json!({"collection": "Paint", "name": name, "type": "color", "value": value})).unwrap();
    }
    s.execute("variable.bind", &json!({"ids": [t], "field": "fills/0/color", "variable": "ink"})).unwrap();
    s.execute("effect.add", &json!({"ids": [t], "type": "layerBlur"})).unwrap();
    (t, v)
}

#[test]
fn text_put_on_a_filled_path_takes_its_fill_and_effects() {
    let mut s = Session::new();
    let (t, v) = text_and_curve(&mut s, json!({"fill": "#FF0000"}));
    let brand = s.execute("style.create", &json!({"type": "paint", "name": "Brand", "value": "#3366FF"})).unwrap()["id"].clone();
    s.execute("style.apply", &json!({"ids": [v], "style": brand})).unwrap();
    s.execute("effect.add", &json!({"ids": [v], "type": "dropShadow"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [v], "field": "effects/0/color", "variable": "shadow"})).unwrap();
    let path = s.doc().unwrap().node(v).unwrap().clone();
    let text = s.doc().unwrap().node(t).unwrap().clone();
    assert!(path.styles.fill.is_some() && path.bound.contains_key("effects/0/color"));
    s.execute("text.onPath", &json!({"ids": [t, v]})).unwrap();
    let n = s.doc().unwrap().node(t).unwrap();
    // "the vector object's fill and effects are automatically transferred to the text layer"
    assert_eq!((&n.fills, &n.styles.fill), (&path.fills, &path.styles.fill), "the path's fill and its style");
    assert_eq!((&n.effects, &n.styles.effect), (&path.effects, &path.styles.effect), "the path's effects");
    assert_eq!(n.bound, path.bound, "the path's bindings replace the text's fill binding");
    s.execute("edit.undo", &json!({})).unwrap();
    let n = s.doc().unwrap().node(t).unwrap();
    assert_eq!((&n.fills, &n.styles.fill), (&text.fills, &text.styles.fill), "undo gives the text its own fill back");
    assert_eq!((&n.effects, &n.styles.effect), (&text.effects, &text.styles.effect), "undo gives the text its own effects back");
    assert_eq!(n.bound, text.bound, "undo gives the text its own fill binding back");
}

#[test]
fn text_put_on_a_stroke_only_path_keeps_its_own_fill_and_effects() {
    let mut s = Session::new();
    let (t, v) = text_and_curve(&mut s, json!({"fills": []}));
    let text = s.doc().unwrap().node(t).unwrap().clone();
    assert!(!text.fills.is_empty() && !text.effects.is_empty());
    s.execute("text.onPath", &json!({"ids": [t, v]})).unwrap();
    let n = s.doc().unwrap().node(t).unwrap();
    assert_eq!((&n.fills, &n.effects, &n.bound), (&text.fills, &text.effects, &text.bound), "a path with no fill or effects to give");
}

#[test]
fn pattern_fills_tile_their_source_live() {
    let mut s = Session::new();
    let tile = id(s.execute("shape.rectangle", &json!({"x": 500, "y": 0, "width": 10, "height": 10})).unwrap());
    s.execute("fill.set", &json!({"ids": [tile], "color": "#FF0000"})).unwrap();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    s.execute("fill.set", &json!({"ids": [r], "color": "#FFFFFF"})).unwrap();
    s.execute("fill.add", &json!({"ids": [r]})).unwrap();
    assert!(s.execute("fill.pattern", &json!({"ids": [r], "source": r})).is_err(), "not itself");
    s.execute("fill.pattern", &json!({"ids": [r], "source": tile, "spacing": [10, 10]})).unwrap();
    assert!(matches!(&s.doc().unwrap().node(r).unwrap().fills[1].kind, PaintKind::Pattern(p) if p.source == tile.0 && p.spacing == (10.0, 10.0)));
    assert_eq!(pixel(&s, r, 5, 5), [255, 0, 0, 255]);
    assert_eq!(pixel(&s, r, 15, 5), [255, 255, 255, 255], "the gap shows the fill below");
    assert_eq!(pixel(&s, r, 45, 45), [255, 0, 0, 255]);
    // Live: recolouring the source recolours the tiles.
    s.execute("fill.set", &json!({"ids": [tile], "color": "#0000FF"})).unwrap();
    assert_eq!(pixel(&s, r, 5, 5), [0, 0, 255, 255]);
    // Half-shifted rows.
    s.execute("fill.pattern", &json!({"ids": [r], "source": tile, "spacing": [10, 10], "tile": "horizontalHex"})).unwrap();
    assert_eq!(pixel(&s, r, 15, 25), [0, 0, 255, 255]);
    assert_eq!(pixel(&s, r, 5, 25), [255, 255, 255, 255]);
    // A copy of the source together with the layer tiles the copied source.
    s.execute("object.group", &json!({"ids": [tile, r]})).unwrap();
    s.execute("edit.duplicate", &json!({})).unwrap();
    let doc = s.doc().unwrap();
    let sources: Vec<u64> = doc
        .nodes()
        .flat_map(|n| n.fills.iter())
        .filter_map(|p| match &p.kind {
            PaintKind::Pattern(pt) => Some(pt.source),
            _ => None,
        })
        .collect();
    assert_eq!(sources.len(), 2);
    assert_ne!(sources[0], sources[1], "each copy tiles its own source");
    // Exports: SVG as a <pattern> of vectors, PDF without failing.
    let svg = figcraft_svg::export_svg(doc, &[r], &figcraft_svg::SvgOptions::default()).unwrap();
    assert!(svg.contains("<pattern") && svg.contains("fill=\"url(#pattern"), "{svg}");
    assert!(figcraft_pdf::export_nodes_pdf(doc, &[r]).is_ok());
}

#[test]
fn a_pattern_of_itself_draws_nothing_instead_of_recursing() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    // Hand-written (the command refuses this): the layer tiles itself.
    s.execute("fill.set", &json!({"ids": [r], "paint": {"type": "pattern", "source": r.0, "scale": 0.25}})).unwrap();
    let doc = s.doc().unwrap();
    assert!(figcraft_render::render_node(doc, r, 1.0).is_some());
    assert!(figcraft_svg::export_svg(doc, &[r], &figcraft_svg::SvgOptions::default()).is_some());
    assert!(figcraft_pdf::export_nodes_pdf(doc, &[r]).is_ok());
}
