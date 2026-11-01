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

/// A click (press and release) at (`x`, `y`) at 100%.
fn click(s: &mut Session, x: f64, y: f64) {
    use crate::tools::{PointerEvent, PointerKind};
    s.pointer(&PointerEvent::at(PointerKind::Down, x, y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, x, y, 1.0)).unwrap();
}

fn editing(s: &Session) -> Option<NodeId> {
    s.ds().unwrap().text_edit.as_ref().map(|te| te.id)
}

#[test]
fn the_text_on_a_path_tool_types_new_text_onto_the_path_it_clicks() {
    let mut s = Session::new();
    let v = id(s.execute("vector.create", &json!({"path": "M0 0 L200 0", "x": 40, "y": 100, "props": {"fill": "#FF0000"}})).unwrap());
    s.execute("effect.add", &json!({"ids": [v], "type": "dropShadow"})).unwrap();
    let (vxf, vw, fills, effects) = {
        let n = s.doc().unwrap().node(v).unwrap();
        (n.transform, n.width, n.fills.clone(), n.effects.clone())
    };
    assert!(!fills.is_empty() && !effects.is_empty());
    s.set_tool(crate::tools::ToolId::TextOnPath);
    // Away from the path a click does nothing.
    click(&mut s, 140.0, 160.0);
    assert!(editing(&s).is_none() && s.doc().unwrap().node(v).is_some(), "only a path takes text");
    // A quarter of the way along, a few pixels off the line.
    click(&mut s, 90.0, 103.0);
    let t = editing(&s).expect("typing starts on the path");
    assert!(s.doc().unwrap().node(v).is_none(), "the path layer became the text's path");
    crate::textedit::insert(&mut s, "Along").unwrap();
    crate::textedit::end(&mut s).unwrap();
    let doc = s.doc().unwrap();
    let n = doc.node(t).unwrap();
    assert_eq!((n.transform, n.width), (vxf, vw), "in the path's place");
    assert_eq!(n.kind.text().map(|t| t.characters.as_str()), Some("Along"));
    assert_eq!((&n.fills, &n.effects), (&fills, &effects), "the path's fill and effects carry over to the text");
    let tp = text_path(&s, t).unwrap();
    assert!((tp.start - 0.25).abs() < 0.01, "the text starts where the path was clicked: {}", tp.start);
    assert_eq!(s.tool.tool, crate::tools::ToolId::Move, "back to Move after placing, as with the Text tool");
    s.execute("edit.undo", &json!({})).unwrap();
    let doc = s.doc().unwrap();
    assert!(doc.node(v).is_some() && doc.node(t).is_none(), "one undo step brings the path back");
}

#[test]
fn a_shape_in_front_whose_path_is_away_from_the_click_does_not_hide_the_path_behind() {
    let mut s = Session::new();
    let back = id(s.execute("vector.create", &json!({"path": "M0 0 L200 0", "x": 40, "y": 100})).unwrap());
    // A filled square over the line: the click is inside it, 20 or more from its outline.
    let front = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 80, "width": 100, "height": 60})).unwrap());
    s.set_tool(crate::tools::ToolId::TextOnPath);
    click(&mut s, 90.0, 100.0);
    assert!(editing(&s).is_some(), "typing starts on the path behind");
    let doc = s.doc().unwrap();
    assert!(doc.node(back).is_none() && doc.node(front).is_some(), "the clicked path took the text");
}

#[test]
fn a_path_scaled_more_one_way_is_reached_where_it_shows() {
    let mut s = Session::new();
    let v = id(s.execute("vector.create", &json!({"path": "M0 0 L100 100", "x": 0, "y": 0})).unwrap());
    // Ten times wider than tall: on the page the line runs from (0, 0) to (1000, 100).
    s.doc_mut().unwrap().get_mut(v).unwrap().transform = figcraft_geom::Affine::scale_non_uniform(10.0, 1.0);
    s.set_tool(crate::tools::ToolId::TextOnPath);
    // 5 below the line on the page, halfway along it.
    click(&mut s, 500.0, 55.0);
    let t = editing(&s).expect("typing starts on the path");
    let tp = text_path(&s, t).unwrap();
    assert!((tp.start - 0.5).abs() < 0.01, "{}", tp.start);
    // A layer shown at a tenth of its size: its outline is still reached 5 screen pixels away.
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 1000, "height": 1000})).unwrap());
    s.doc_mut().unwrap().get_mut(r).unwrap().transform = figcraft_geom::Affine::scale(0.1);
    s.set_tool(crate::tools::ToolId::TextOnPath);
    click(&mut s, 105.0, 50.0);
    assert!(editing(&s).is_some() && s.doc().unwrap().node(r).is_none(), "typing starts on the small square's path");
}

#[test]
fn paths_a_mask_or_a_turned_clipping_frame_hides_take_no_text() {
    let mut s = Session::new();
    // A square masked by a circle in its top-left corner: its right edge doesn't show.
    let m = id(s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    s.execute("object.mask", &json!({"ids": [m, r]})).unwrap();
    // A clipping frame turned 45°, with a line outside it: inside the frame's bounding box, but clipped.
    let f = id(s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 100, "height": 100, "props": {"clip": true}})).unwrap());
    let l = id(s.execute("vector.create", &json!({"path": "M0 0 L40 0", "x": -60, "y": 50, "parent": f})).unwrap());
    s.execute("node.set", &json!({"id": f, "props": {"rotation": 45}})).unwrap();
    let hidden = s.doc().unwrap().world_transform(l) * figcraft_geom::Point::new(20.0, 0.0);
    let frame_box = s.doc().unwrap().world_bounds(f).unwrap();
    assert!(frame_box.contains(hidden), "the line's middle is inside the frame's bounding box");
    s.set_tool(crate::tools::ToolId::TextOnPath);
    for (x, y) in [(100.0, 50.0), (hidden.x, hidden.y)] {
        click(&mut s, x, y);
        assert!(editing(&s).is_none(), "nothing shows at ({x}, {y})");
    }
    let doc = s.doc().unwrap();
    assert!(doc.node(r).is_some() && doc.node(l).is_some());
}

#[test]
fn text_on_a_stroke_only_path_gets_the_usual_fill() {
    let mut s = Session::new();
    let v = id(s.execute("vector.create", &json!({"path": "M0 0 C50 -50 150 -50 200 0", "x": 40, "y": 100, "props": {"fills": []}})).unwrap());
    assert!(!s.doc().unwrap().node(v).unwrap().fills.iter().any(|f| f.visible), "a path with a stroke only");
    s.set_tool(crate::tools::ToolId::TextOnPath);
    click(&mut s, 40.0, 100.0);
    let t = editing(&s).expect("typing starts on the path");
    assert!(s.doc().unwrap().node(t).unwrap().fills.iter().any(|f| f.visible), "the text can be seen");
}

#[test]
fn text_on_a_path_left_empty_puts_the_path_back_without_an_undo_step() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap());
    let locked = id(s.execute("shape.rectangle", &json!({"x": 400, "y": 100, "width": 100, "height": 100, "props": {"locked": true}})).unwrap());
    let before = (s.doc().unwrap().clone(), s.undo_labels(100).len());
    s.set_tool(crate::tools::ToolId::TextOnPath);
    click(&mut s, 400.0, 150.0);
    assert!(editing(&s).is_none(), "a locked layer's path doesn't take text");
    // Inside the square, away from its outline, nothing happens either.
    click(&mut s, 150.0, 150.0);
    assert!(editing(&s).is_none());
    click(&mut s, 100.0, 150.0);
    assert!(editing(&s).is_some() && s.doc().unwrap().node(r).is_none());
    crate::textedit::end(&mut s).unwrap();
    assert_eq!(*s.doc().unwrap(), before.0, "the square is back as it was");
    assert_eq!(s.undo_labels(100).len(), before.1, "no undo step");
    assert!(s.doc().unwrap().node(locked).is_some());
    // A double-click on the path, then nothing typed: the same (with this tool and the Text tool).
    use crate::tools::{PointerEvent, PointerKind, ToolId};
    for tool in [ToolId::TextOnPath, ToolId::Text] {
        s.set_tool(tool);
        click(&mut s, 100.0, 150.0);
        s.pointer(&PointerEvent::at(PointerKind::DoubleClick, 100.0, 150.0, 1.0)).unwrap();
        assert!(editing(&s).is_some(), "{tool:?}");
        crate::textedit::end(&mut s).unwrap();
        assert_eq!(*s.doc().unwrap(), before.0, "{tool:?}: the file is back as it was");
        assert_eq!(s.undo_labels(100).len(), before.1, "{tool:?}: no undo step");
    }
}

#[test]
fn scaling_text_scales_its_path() {
    let mut s = Session::new();
    let t = id(s.execute("text.create", &json!({"text": "Around", "x": 0, "y": 0})).unwrap());
    let v = id(s.execute("vector.create", &json!({"path": "M0 0 C50 -50 150 -50 200 0", "x": 40, "y": 100})).unwrap());
    s.execute("text.onPath", &json!({"ids": [t, v]})).unwrap();
    let w0 = figcraft_geom::Shape::bounding_box(&text_path(&s, t).unwrap().path()).width();
    s.execute("transform.scale", &json!({"ids": [t], "factor": 2.0})).unwrap();
    let w1 = figcraft_geom::Shape::bounding_box(&text_path(&s, t).unwrap().path()).width();
    assert!((w1 - w0 * 2.0).abs() < 1.0, "path width {w0} -> {w1}");
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
fn text_put_on_a_path_with_an_empty_effect_style_keeps_its_own_effects() {
    let mut s = Session::new();
    let (t, v) = text_and_curve(&mut s, json!({"fills": []}));
    let none = s.execute("style.create", &json!({"type": "effect", "name": "None", "value": []})).unwrap()["id"].clone();
    s.execute("style.apply", &json!({"ids": [v], "style": none})).unwrap();
    let path = s.doc().unwrap().node(v).unwrap().clone();
    assert!(path.effects.is_empty() && path.styles.effect.is_some());
    let text = s.doc().unwrap().node(t).unwrap().clone();
    s.execute("text.onPath", &json!({"ids": [t, v]})).unwrap();
    let n = s.doc().unwrap().node(t).unwrap();
    assert_eq!((&n.effects, &n.styles.effect), (&text.effects, &text.styles.effect), "a path whose effect style draws no effects");
}

#[test]
fn text_put_on_a_path_takes_a_fill_binding_made_through_fills() {
    let mut s = Session::new();
    let (t, v) = text_and_curve(&mut s, json!({"fill": "#FF0000"}));
    s.execute("variable.bind", &json!({"ids": [v], "field": "fills", "variable": "shadow"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [t], "field": "fills", "variable": "ink"})).unwrap();
    let path = s.doc().unwrap().node(v).unwrap().clone();
    assert!(path.bound.contains_key("fills"));
    s.execute("text.onPath", &json!({"ids": [t, v]})).unwrap();
    let n = s.doc().unwrap().node(t).unwrap();
    assert_eq!(n.bound, path.bound, "the path's `fills` binding replaces the text's");
    assert_eq!(n.fills, path.fills);
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

#[test]
fn the_text_on_a_path_tool_carries_a_fill_bound_through_fills() {
    let mut s = Session::new();
    let v = id(s.execute("vector.create", &json!({"path": "M0 0 L200 0", "x": 40, "y": 100, "props": {"fill": "#FF0000"}})).unwrap());
    s.execute("variable.createCollection", &json!({"name": "Paint"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Paint", "name": "ink", "type": "color", "value": "#00FF00"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [v], "field": "fills", "variable": "ink"})).unwrap();
    let bound = s.doc().unwrap().node(v).unwrap().bound.clone();
    assert!(bound.contains_key("fills"));
    s.set_tool(crate::tools::ToolId::TextOnPath);
    click(&mut s, 90.0, 103.0);
    let t = editing(&s).expect("typing starts on the path");
    assert_eq!(s.doc().unwrap().node(t).unwrap().bound, bound, "the fill keeps following its variable");
}
