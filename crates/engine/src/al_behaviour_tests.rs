//! Auto layout behaviour through commands (the rules of `figcraft_layout`'s behaviour suite that
//! involve edits): what sizing a layer may take, how a hugging frame follows its children, the
//! settings' defaults, canvas stacking in renders and exports, and one undo step per edit.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{CellAlign, CrossAlign, LayoutMode, MainAlign, NodeId, Sizing};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, parent: NodeId, w: f64, h: f64, fill: &str) -> NodeId {
    id(s.execute("shape.rectangle", &json!({"parent": parent, "x": 0, "y": 0, "width": w, "height": h, "props": {"fill": fill}})).unwrap())
}

/// A frame with auto layout (no padding, no gap) and the given extra props.
fn al_frame(s: &mut Session, props: Value) -> NodeId {
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap());
    let mut all = json!({"layout": "horizontal", "padding": 0, "gap": 0, "fills": []});
    if let (Some(all), Value::Object(extra)) = (all.as_object_mut(), props) {
        all.extend(extra);
    }
    set(s, f, all);
    f
}

fn set(s: &mut Session, node: NodeId, props: Value) {
    s.execute("node.set", &json!({"id": node, "props": props})).unwrap();
}

fn size(s: &Session, node: NodeId) -> (f64, f64) {
    let n = s.doc().unwrap().node(node).unwrap();
    (n.width, n.height)
}

fn pos(s: &Session, node: NodeId) -> (f64, f64) {
    let c = s.doc().unwrap().node(node).unwrap().transform.as_coeffs();
    (c[4], c[5])
}

fn sizing(s: &Session, node: NodeId) -> (Sizing, Sizing) {
    let cl = &s.doc().unwrap().node(node).unwrap().child_layout;
    (cl.sizing_h, cl.sizing_v)
}

/// The RGBA of `node` rendered at 1x, at (`x`, `y`) of its box.
fn pixel(s: &Session, node: NodeId, x: u32, y: u32) -> [u8; 4] {
    let pix = figcraft_render::render_node(s.doc().unwrap(), node, 1.0).unwrap();
    let i = ((y * pix.width() + x) * 4) as usize;
    let d = pix.data();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

#[test]
fn fill_on_a_child_turns_the_hugging_parent_fixed() {
    // Documented: a child set to Fill container makes its parent stop hugging that axis.
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({"sizingH": "hug", "sizingV": "hug"}));
    let a = rect(&mut s, f, 50.0, 20.0, "#FF0000");
    rect(&mut s, f, 30.0, 40.0, "#0000FF");
    assert_eq!(size(&s, f), (80.0, 40.0));
    set(&mut s, a, json!({"sizingH": "fill"}));
    assert_eq!(sizing(&s, f), (Sizing::Fixed, Sizing::Hug));
    assert_eq!(sizing(&s, a).0, Sizing::Fill);
    // Fixed at the size it had: the fill child takes what the other child leaves.
    assert_eq!(size(&s, f), (80.0, 40.0));
    assert_eq!(size(&s, a), (50.0, 20.0));
    s.undo().unwrap();
    assert_eq!(sizing(&s, f), (Sizing::Hug, Sizing::Hug));
}

#[test]
fn a_child_resized_to_the_full_available_space_becomes_fill() {
    // Documented: "Child objects of an auto layout frame will also be set to Fill container if
    // they are manually resized to the full available space of the parent frame."
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({"padding": 10, "gap": 10})); // 200 × 100, fixed
    let a = rect(&mut s, f, 50.0, 20.0, "#FF0000");
    // Along the flow, alone in the frame: its width less the padding (200 - 20).
    s.execute("transform.resize", &json!({"ids": [a], "width": 180})).unwrap();
    assert_eq!(sizing(&s, a), (Sizing::Fill, Sizing::Fixed));
    assert_eq!(size(&s, a), (180.0, 20.0));
    s.undo().unwrap();
    assert_eq!(sizing(&s, a), (Sizing::Fixed, Sizing::Fixed), "undone as part of the edit");
    // Across the flow, beside a sibling too: the frame's height less its padding.
    let b = rect(&mut s, f, 50.0, 20.0, "#0000FF");
    set(&mut s, b, json!({"height": 80}));
    assert_eq!(sizing(&s, b), (Sizing::Fixed, Sizing::Fill));
    assert_eq!(size(&s, b), (50.0, 80.0));
    // Short of it: fixed, as before.
    set(&mut s, b, json!({"height": 70}));
    assert_eq!(sizing(&s, b), (Sizing::Fixed, Sizing::Fixed));
    // Along the flow beside a sibling, what is left is not "the full available space": a width
    // that happens to match it (180 - 10 - 50) stays fixed.
    s.execute("transform.resize", &json!({"ids": [b], "width": 120})).unwrap();
    assert_eq!(sizing(&s, b), (Sizing::Fixed, Sizing::Fixed));
    // A hugging frame has no full space to offer: the child stays fixed.
    let g = al_frame(&mut s, json!({"sizingH": "hug", "sizingV": "hug", "padding": 10}));
    let c = rect(&mut s, g, 50.0, 20.0, "#00FF00");
    set(&mut s, c, json!({"width": 180}));
    assert_eq!(sizing(&s, c), (Sizing::Fixed, Sizing::Fixed));
    // An inside stroke the layout includes takes room like the padding: the full space is what
    // layout leaves the child (200 - 2 × 10 - 2 × 4), not the frame less its padding alone.
    let h = al_frame(&mut s, json!({"padding": 10, "strokesIncluded": true, "stroke": "#000000", "strokeWeight": 4, "strokeAlign": "inside"}));
    let d = rect(&mut s, h, 50.0, 20.0, "#FF0000");
    set(&mut s, d, json!({"width": 180}));
    assert_eq!(sizing(&s, d), (Sizing::Fixed, Sizing::Fixed), "wider than the room the stroke leaves");
    assert_eq!(size(&s, d), (180.0, 20.0), "and kept as typed");
    set(&mut s, d, json!({"width": 172}));
    assert_eq!(sizing(&s, d), (Sizing::Fill, Sizing::Fixed));
    assert_eq!(size(&s, d), (172.0, 20.0));
    // Excluded from the layout, the stroke takes no room: the frame less its padding again.
    set(&mut s, h, json!({"strokesIncluded": false}));
    set(&mut s, d, json!({"width": 172}));
    assert_eq!(sizing(&s, d), (Sizing::Fixed, Sizing::Fixed));
}

#[test]
fn hug_and_fill_only_where_they_apply() {
    // Documented: Hug applies to auto layout frames (and text); Fill to children of auto layout,
    // never to a top-level frame.
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({}));
    let r = rect(&mut s, f, 50.0, 20.0, "#FF0000");
    assert!(s.execute("node.set", &json!({"id": r, "props": {"sizingH": "hug"}})).is_err());
    assert!(s.execute("node.set", &json!({"id": f, "props": {"sizingH": "fill"}})).is_err());
    assert_eq!(sizing(&s, f), (Sizing::Fixed, Sizing::Fixed));
}

#[test]
fn a_hugging_frame_follows_its_children_through_nested_levels() {
    // Documented: hug resizes the frame from its children; nested frames keep their own padding
    // and gap.
    let mut s = Session::new();
    let outer = al_frame(&mut s, json!({"layout": "vertical", "sizingH": "hug", "sizingV": "hug", "padding": 10, "gap": 5}));
    let inner = id(s.execute("frame.create", &json!({"parent": outer, "x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    set(&mut s, inner, json!({"layout": "horizontal", "padding": 4, "gap": 2, "sizingH": "hug", "sizingV": "hug", "fills": []}));
    let a = rect(&mut s, inner, 30.0, 20.0, "#FF0000");
    rect(&mut s, inner, 30.0, 10.0, "#0000FF");
    rect(&mut s, outer, 20.0, 20.0, "#00FF00");
    // Inner: 4 + 30 + 2 + 30 + 4 = 70 by 4 + 20 + 4 = 28; outer: 10 + 70 + 10 by 10 + 28 + 5 + 20 + 10.
    assert_eq!(size(&s, inner), (70.0, 28.0));
    assert_eq!(size(&s, outer), (90.0, 73.0));
    // A child grows: both levels follow in the same edit, and one undo puts it all back.
    set(&mut s, a, json!({"width": 60, "height": 40}));
    assert_eq!(size(&s, inner), (100.0, 48.0));
    assert_eq!(size(&s, outer), (120.0, 93.0));
    s.undo().unwrap();
    assert_eq!(size(&s, outer), (90.0, 73.0));
}

#[test]
fn new_auto_layout_includes_inside_strokes() {
    // Documented: by default inside strokes are included in auto layout sizes.
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    set(&mut s, f, json!({"stroke": "#000000", "strokeWeight": 3, "strokeAlign": "inside"}));
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    assert!(s.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().strokes_included);
    set(&mut s, f, json!({"padding": 0, "sizingH": "hug", "sizingV": "hug"}));
    let r = rect(&mut s, f, 40.0, 20.0, "#FF0000");
    assert_eq!(size(&s, f), (46.0, 26.0));
    assert_eq!(pos(&s, r), (3.0, 3.0));
    // Excluded: the stroke takes no space (one undo step brings it back).
    set(&mut s, f, json!({"strokesIncluded": false}));
    assert_eq!(size(&s, f), (40.0, 20.0));
    s.undo().unwrap();
    assert_eq!(size(&s, f), (46.0, 26.0));
}

#[test]
fn auto_gaps_and_cell_alignment_through_node_set() {
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({}));
    let k: Vec<NodeId> = (0..3).map(|_| rect(&mut s, f, 20.0, 20.0, "#FF0000")).collect();
    // 200 - 60 = 140: evenly 35 everywhere; around 46.67 between, half at the edges.
    set(&mut s, f, json!({"mainAlign": "spaceEvenly"}));
    assert_eq!(s.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().main_align, MainAlign::SpaceEvenly);
    assert_eq!(k.iter().map(|c| pos(&s, *c).0).collect::<Vec<_>>(), vec![35.0, 90.0, 145.0]);
    set(&mut s, f, json!({"primaryAxisAlignItems": "SPACE_AROUND"}));
    assert!((pos(&s, k[0]).0 - 70.0 / 3.0).abs() < 1e-9);
    assert!(s.execute("node.set", &json!({"id": f, "props": {"mainAlign": "spaceSideways"}})).is_err());
    // A grid child aligns in its own cell; auto goes back to the frame's alignment.
    set(&mut s, f, json!({"layout": "grid", "columnTracks": ["1fr", "1fr", "1fr"], "rowTracks": ["1fr"], "mainAlign": "min"}));
    set(&mut s, k[1], json!({"gridAlignH": "max", "gridChildVerticalAlign": "CENTER"}));
    let cl = s.doc().unwrap().node(k[1]).unwrap().child_layout.clone();
    assert_eq!((cl.grid_align_h, cl.grid_align_v), (Some(CellAlign::Max), Some(CellAlign::Center)));
    // Columns of 200 / 3 (the second ends at 133.33) in one 100 px row.
    let (x, y) = pos(&s, k[1]);
    assert!((x - (400.0 / 3.0 - 20.0)).abs() < 1e-9 && (y - 40.0).abs() < 1e-9, "{x} {y}");
    set(&mut s, k[1], json!({"gridAlignH": "auto", "gridAlignV": null}));
    assert!((pos(&s, k[1]).0 - 200.0 / 3.0).abs() < 1e-9);
    assert!(s.execute("node.set", &json!({"id": k[1], "props": {"gridAlignH": "baseline"}})).is_err());
}

#[test]
fn vertical_wrap_through_node_set() {
    // Documented: a vertical flow wraps into a new column when it reaches the frame's height.
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({"layout": "vertical", "wrap": true, "gap": 10, "crossGap": 5}));
    let k: Vec<NodeId> = (0..3).map(|_| rect(&mut s, f, 20.0, 40.0, "#FF0000")).collect();
    assert_eq!(k.iter().map(|c| pos(&s, *c)).collect::<Vec<_>>(), vec![(0.0, 0.0), (0.0, 50.0), (25.0, 0.0)]);
}

#[test]
fn vertical_wrap_columns_take_arrows_and_drops() {
    // 40 px tall items, gap 10, 160 px tall: three per column, columns 30 px apart.
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({"layout": "vertical", "wrap": true, "gap": 10, "crossGap": 10, "height": 160}));
    let k: Vec<NodeId> = (0..6).map(|_| rect(&mut s, f, 20.0, 40.0, "#FF0000")).collect();
    let kids = |s: &Session| s.doc().unwrap().node(f).unwrap().children.clone();
    assert_eq!(pos(&s, k[3]), (30.0, 0.0), "second column");
    // Right: the second item moves into the second column, beside where it was.
    s.execute("transform.nudge", &json!({"ids": [k[1]], "dx": 1, "dy": 0})).unwrap();
    assert_eq!(kids(&s), vec![k[0], k[2], k[3], k[4], k[1], k[5]]);
    assert_eq!(pos(&s, k[1]), (30.0, 50.0));
    // Down still steps along the flow.
    s.execute("transform.nudge", &json!({"ids": [k[1]], "dx": 0, "dy": 1})).unwrap();
    assert_eq!(kids(&s), vec![k[0], k[2], k[3], k[4], k[5], k[1]]);
    // A drop between two items of a column: before the lower one, a horizontal insertion line.
    let (index, line) = crate::drop::auto_layout_slot(s.doc().unwrap(), f, figcraft_geom::Point::new(35.0, 45.0), &[]).unwrap();
    assert_eq!(index, 4);
    assert_eq!(line, Some([30.0, 48.0, 50.0, 48.0]));
}

#[test]
fn adding_auto_layout_to_a_frame_keeps_its_size() {
    // An existing frame with content hugs it (padding inferred, so the size is preserved);
    // an empty one keeps its size instead of collapsing to its padding.
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200})).unwrap());
    rect(&mut s, f, 40.0, 20.0, "#FF0000");
    s.execute("layout.add", &json!({"ids": [f]})).unwrap();
    assert_eq!(sizing(&s, f), (Sizing::Hug, Sizing::Hug));
    assert_eq!(size(&s, f), (300.0, 200.0));
    // An empty frame keeps its size too.
    let e = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200})).unwrap());
    s.execute("layout.add", &json!({"ids": [e]})).unwrap();
    assert_eq!(sizing(&s, e), (Sizing::Fixed, Sizing::Fixed));
    assert_eq!(size(&s, e), (300.0, 200.0));
}

#[test]
fn canvas_stacking_paints_and_exports_the_first_child_on_top() {
    // Documented: the last child is on top by default; "first on top" flips which one is drawn
    // over the other, and the layer order stays the same.
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({"sizingH": "hug", "sizingV": "hug", "gap": -20}));
    let a = rect(&mut s, f, 50.0, 20.0, "#FF0000");
    let b = rect(&mut s, f, 50.0, 20.0, "#0000FF");
    let order = s.doc().unwrap().node(f).unwrap().children.clone();
    let svg = |s: &Session| figcraft_svg::export_svg(s.doc().unwrap(), &[f], &figcraft_svg::SvgOptions::default()).unwrap().to_lowercase();
    let red_first = |svg: &str| svg.find("ff0000").unwrap() < svg.find("0000ff").unwrap();
    // The overlap is x 30..50.
    assert_eq!(pixel(&s, f, 40, 10), [0, 0, 255, 255]);
    assert!(red_first(&svg(&s)));
    set(&mut s, f, json!({"reverseZ": true}));
    assert_eq!(pixel(&s, f, 40, 10), [255, 0, 0, 255]);
    assert!(!red_first(&svg(&s)));
    assert!(figcraft_pdf::export_nodes_pdf(s.doc().unwrap(), &[f]).is_ok());
    // Places and layers stay; one undo step puts the last child back on top.
    assert_eq!(s.doc().unwrap().node(f).unwrap().children, order);
    assert_eq!((pos(&s, a), pos(&s, b)), ((0.0, 0.0), (30.0, 0.0)));
    s.undo().unwrap();
    assert_eq!(pixel(&s, f, 40, 10), [0, 0, 255, 255]);
}

#[test]
fn a_size_given_with_a_new_path_fixes_that_axis() {
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({}));
    let v = id(s.execute("node.create", &json!({"type": "vector", "path": "M0 0 L10 10", "closed": false, "parent": f})).unwrap());
    set(&mut s, v, json!({"sizingH": "fill"}));
    assert_eq!(sizing(&s, v).0, Sizing::Fill);
    set(&mut s, v, json!({"path": "M0 0 L20 5", "width": 30}));
    assert_eq!(sizing(&s, v).0, Sizing::Fixed);
    assert!((size(&s, v).0 - 30.0).abs() < 1e-6, "{:?}", size(&s, v));
    // The axis is judged by the size the path is given, not the one before it: the frame's full
    // height (100) given with a path fills; a smaller height given with the next path is fixed,
    // though the layer filled the frame until then.
    set(&mut s, v, json!({"path": "M0 0 L20 5", "height": 100}));
    assert_eq!(sizing(&s, v).1, Sizing::Fill);
    assert!((size(&s, v).1 - 100.0).abs() < 1e-6, "{:?}", size(&s, v));
    set(&mut s, v, json!({"height": 40, "path": "M0 0 L20 5"}));
    assert_eq!(sizing(&s, v).1, Sizing::Fixed);
    assert!((size(&s, v).1 - 40.0).abs() < 1e-6, "{:?}", size(&s, v));
    // Rotated, the layer's local size isn't the room layout gives it: a height matching the
    // frame's stays fixed rather than have Fill squash the path.
    set(&mut s, v, json!({"rotation": 90}));
    set(&mut s, v, json!({"path": "M0 0 L20 5", "height": 100}));
    assert_eq!(sizing(&s, v).1, Sizing::Fixed);
}

/// What one call's auto layout keys give, whichever way they are spelled: the mode (a string, or
/// an object that replaces the whole auto layout) applies first and the other keys tune it.
#[test]
fn auto_layout_keys_in_one_call_tune_the_mode_given_beside_them() {
    for (key, mode) in ["layout", "layoutMode"].into_iter().flat_map(|k| [(k, json!("vertical")), (k, json!({"mode": "vertical"}))]) {
        let mut props = json!({"gap": 8, "padding": 4, "crossAlign": "center"});
        props[key] = mode;
        let mut s = Session::new();
        let created = id(s.execute("node.create", &json!({"type": "frame", "props": props})).unwrap());
        let set_on = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
        set(&mut s, set_on, props.clone());
        for f in [created, set_on] {
            let l = s.doc().unwrap().node(f).unwrap().layout.clone().unwrap();
            assert_eq!((l.mode, l.gap, l.padding, l.cross_align), (LayoutMode::Vertical, 8.0, [4.0; 4], CrossAlign::Center), "{props}");
        }
    }
}

/// Auto layout keys beside a `layout` that turns auto layout off would turn it back on: the call
/// is refused and the layer stays as it was.
#[test]
fn auto_layout_keys_beside_no_auto_layout_are_refused() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    for props in
        [json!({"layout": null, "padding": 4}), json!({"layout": "none", "gap": 8, "padding": 4}), json!({"layoutMode": "NONE", "wrap": true})]
    {
        let e = s.execute("node.set", &json!({"id": f, "props": props})).unwrap_err().to_string();
        assert!(e.contains("turns auto layout off"), "{props}: {e}");
        assert!(s.doc().unwrap().node(f).unwrap().layout.is_none(), "{props}");
        let e = s.execute("node.create", &json!({"type": "frame", "props": props})).unwrap_err().to_string();
        assert!(e.contains("turns auto layout off"), "{props}: {e}");
    }
    let e = s.execute("node.set", &json!({"id": f, "props": {"layout": null, "gap": 8, "padding": 4}})).unwrap_err().to_string();
    assert!(e.contains("`gap`, `padding` are auto layout properties, and `layout` in the same call"), "{e}");
    let e = s.execute("node.set", &json!({"id": f, "props": {"layoutMode": "NONE", "wrap": true}})).unwrap_err().to_string();
    assert!(e.contains("`wrap` is an auto layout property, and `layoutMode` in the same call"), "{e}");
    // Without other auto layout keys, `layout: null` still removes it.
    set(&mut s, f, json!({"layout": "vertical", "gap": 8}));
    set(&mut s, f, json!({"layout": null, "name": "Plain"}));
    assert!(s.doc().unwrap().node(f).unwrap().layout.is_none());
}

// ---- Add auto layout (Shift+A) defaults, measured in Figma's web app with synthetic rectangles ----

fn rect_at(s: &mut Session, parent: Option<NodeId>, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let (Some(parent), Some(o)) = (parent, p.as_object_mut()) {
        o.insert("parent".into(), json!(parent));
    }
    id(s.execute("shape.rectangle", &p).unwrap())
}

fn add_layout(s: &mut Session, ids: &[NodeId]) -> NodeId {
    let r = s.execute("layout.add", &json!({"ids": ids})).unwrap();
    id_of(&r["ids"][0]).unwrap()
}

fn canvas(s: &Session, node: NodeId) -> (f64, f64) {
    let c = s.doc().unwrap().world_transform(node).as_coeffs();
    (c[4], c[5])
}

#[test]
fn shift_a_on_two_rects_far_apart_spaces_them_between_at_a_fixed_width() {
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let b = rect_at(&mut s, None, 100.0, 0.0, 40.0, 40.0);
    let f = add_layout(&mut s, &[a, b]);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.main_align, l.padding), (LayoutMode::Horizontal, MainAlign::SpaceBetween, [0.0; 4]));
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Fixed, Sizing::Hug));
    assert_eq!((n.width, n.height), (140.0, 40.0));
    assert_eq!((canvas(&s, a).0, canvas(&s, b).0), (0.0, 100.0));
}

#[test]
fn shift_a_gap_auto_needs_a_gap_at_least_the_narrower_child() {
    // (first size, second x, second width, second height, Auto?)
    for (w, x2, w2, h2, auto) in [
        (40.0, 72.0, 40.0, 40.0, false),
        (40.0, 80.0, 40.0, 40.0, true),
        (20.0, 52.0, 20.0, 20.0, true),
        (100.0, 160.0, 100.0, 100.0, false),
        (40.0, 100.0, 80.0, 60.0, true),
    ] {
        let mut s = Session::new();
        let a = rect_at(&mut s, None, 0.0, 0.0, w, w);
        let b = rect_at(&mut s, None, x2, 0.0, w2, h2);
        let f = add_layout(&mut s, &[a, b]);
        let n = s.doc().unwrap().node(f).unwrap();
        let l = n.layout.clone().unwrap();
        assert_eq!(l.gap, x2 - w, "{w} {x2}");
        if auto {
            assert_eq!((l.main_align, n.child_layout.sizing_h, n.width), (MainAlign::SpaceBetween, Sizing::Fixed, x2 + w2), "{w} {x2}");
        } else {
            assert_eq!(l.main_align, MainAlign::Min, "{w} {x2}");
            assert_eq!(n.child_layout.sizing_h, Sizing::Hug, "{w} {x2}");
        }
    }
}

#[test]
fn shift_a_on_three_rects_in_a_row_uses_the_mean_gap_and_hugs() {
    for (xs, gap, width) in [([0.0, 100.0, 200.0], 60.0, 240.0), ([0.0, 100.0, 180.0], 50.0, 220.0)] {
        let mut s = Session::new();
        let ids: Vec<NodeId> = xs.iter().map(|x| rect_at(&mut s, None, *x, 0.0, 40.0, 40.0)).collect();
        let f = add_layout(&mut s, &ids);
        let n = s.doc().unwrap().node(f).unwrap();
        let l = n.layout.clone().unwrap();
        assert_eq!(l.gap, gap);
        assert_eq!(l.main_align, MainAlign::Min);
        assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v, n.width), (Sizing::Hug, Sizing::Hug, width));
    }
}

#[test]
fn shift_a_on_stacked_rects_fixes_the_width_and_hugs_the_height() {
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let b = rect_at(&mut s, None, 0.0, 100.0, 80.0, 60.0);
    let f = add_layout(&mut s, &[a, b]);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.gap), (LayoutMode::Vertical, 60.0));
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Fixed, Sizing::Hug));
    assert_eq!((n.width, n.height), (80.0, 160.0));
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let b = rect_at(&mut s, None, 0.0, 240.0, 40.0, 40.0);
    let f = add_layout(&mut s, &[a, b]);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.gap), (LayoutMode::Vertical, 200.0));
    assert_eq!(l.main_align, MainAlign::Min);
    assert_eq!((n.child_layout.sizing_h, n.width), (Sizing::Fixed, 40.0));
}

#[test]
fn shift_a_on_one_rect_pads_ten_and_keeps_it_in_place() {
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let f = add_layout(&mut s, &[a]);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.gap, l.padding), (LayoutMode::Horizontal, 10.0, [10.0; 4]));
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Hug, Sizing::Hug));
    assert_eq!((n.width, n.height), (60.0, 60.0));
    assert_eq!(canvas(&s, f), (-10.0, -10.0));
    assert_eq!(canvas(&s, a), (0.0, 0.0));
    assert_eq!(size(&s, a), (40.0, 40.0));
}

fn filled_frame(s: &mut Session) -> NodeId {
    id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 200, "props": {"fill": "#ff0000"}})).unwrap())
}

#[test]
fn shift_a_on_a_frame_mirrors_its_left_and_top_offsets() {
    let mut s = Session::new();
    let f = filled_frame(&mut s);
    rect_at(&mut s, Some(f), 20.0, 30.0, 40.0, 40.0);
    rect_at(&mut s, Some(f), 120.0, 30.0, 40.0, 40.0);
    let fills = s.doc().unwrap().node(f).unwrap().fills.len();
    let clip = s.doc().unwrap().node(f).unwrap().clip;
    add_layout(&mut s, &[f]);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.gap, l.padding), (LayoutMode::Horizontal, 60.0, [30.0, 20.0, 30.0, 20.0]));
    assert_eq!(l.main_align, MainAlign::Min);
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Hug, Sizing::Hug));
    assert_eq!((n.width, n.height), (180.0, 100.0));
    assert_eq!((n.fills.len(), n.clip), (fills, clip));
}

#[test]
fn shift_a_on_a_frame_with_centred_content_spaces_it_between() {
    let mut s = Session::new();
    let f = filled_frame(&mut s);
    rect_at(&mut s, Some(f), 49.0, 49.0, 40.0, 40.0);
    rect_at(&mut s, Some(f), 211.0, 49.0, 40.0, 40.0);
    add_layout(&mut s, &[f]);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.main_align, l.padding), (MainAlign::SpaceBetween, [49.0; 4]));
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Fixed, Sizing::Hug));
    assert_eq!((n.width, n.height), (300.0, 138.0));
}

#[test]
fn shift_a_on_a_frame_with_stacked_content_fixes_the_content_width() {
    let mut s = Session::new();
    let f = filled_frame(&mut s);
    rect_at(&mut s, Some(f), 20.0, 20.0, 40.0, 40.0);
    rect_at(&mut s, Some(f), 20.0, 100.0, 40.0, 40.0);
    add_layout(&mut s, &[f]);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.gap, l.padding), (LayoutMode::Vertical, 40.0, [20.0; 4]));
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Fixed, Sizing::Hug));
    assert_eq!((n.width, n.height), (80.0, 160.0));
}

#[test]
fn shift_a_on_a_group_follows_the_wrap_rule() {
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let b = rect_at(&mut s, None, 100.0, 0.0, 40.0, 40.0);
    let g = id(s.execute("object.group", &json!({"ids": [a, b]})).unwrap());
    let f = add_layout(&mut s, &[g]);
    assert_eq!(f, g);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!(l.main_align, MainAlign::SpaceBetween);
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Fixed, Sizing::Hug));
    assert_eq!((n.width, n.height), (140.0, 40.0));
    assert!(n.fills.is_empty() && !n.clip);
    assert_eq!((canvas(&s, a).0, canvas(&s, b).0), (0.0, 100.0));
}

#[test]
fn shift_a_on_a_one_child_group_follows_the_group_rule_not_the_one_child_frame_exception() {
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let g = id(s.execute("object.group", &json!({"ids": [a]})).unwrap());
    let r = s.execute("layout.add", &json!({"ids": [g], "direction": "vertical"})).unwrap();
    assert_eq!(id_of(&r["ids"][0]), Some(g));
    let n = s.doc().unwrap().node(g).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.padding), (LayoutMode::Vertical, [0.0; 4]));
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Fixed, Sizing::Hug));
    assert!(n.fills.is_empty() && !n.clip);
}

fn add_layout_dir(s: &mut Session, ids: &[NodeId], direction: &str) -> NodeId {
    let r = s.execute("layout.add", &json!({"ids": ids, "direction": direction})).unwrap();
    id_of(&r["ids"][0]).unwrap()
}

#[test]
fn shift_a_gap_auto_compares_the_narrower_width_not_the_mean() {
    // Widths 40 and 100, gap 60: at least the narrower (40), below the mean (70).
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let b = rect_at(&mut s, None, 100.0, 0.0, 100.0, 40.0);
    let f = add_layout(&mut s, &[a, b]);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.gap, l.main_align, n.child_layout.sizing_h, n.width), (60.0, MainAlign::SpaceBetween, Sizing::Fixed, 200.0));
}

#[test]
fn shift_a_direction_vertical_on_a_horizontal_pair_stacks_with_a_fixed_width_and_no_space_between() {
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let b = rect_at(&mut s, None, 100.0, 0.0, 40.0, 40.0);
    let f = add_layout_dir(&mut s, &[a, b], "vertical");
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    // Measured along y the two overlap, so the gap clamps to 0.
    assert_eq!((l.mode, l.gap, l.main_align), (LayoutMode::Vertical, 0.0, MainAlign::Min));
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v, n.width), (Sizing::Fixed, Sizing::Hug, 140.0));
}

#[test]
fn shift_a_direction_horizontal_on_a_stacked_pair_hugs_the_width() {
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let b = rect_at(&mut s, None, 0.0, 100.0, 80.0, 60.0);
    let f = add_layout_dir(&mut s, &[a, b], "horizontal");
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.gap, l.main_align), (LayoutMode::Horizontal, 0.0, MainAlign::Min));
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Hug, Sizing::Hug));
}

#[test]
fn shift_a_overlapping_children_clamp_the_gap_to_zero_and_fractions_round() {
    let mut s = Session::new();
    let a = rect_at(&mut s, None, 0.0, 0.0, 40.0, 40.0);
    let b = rect_at(&mut s, None, 30.0, 0.0, 40.0, 40.0);
    let f = add_layout(&mut s, &[a, b]);
    let l = s.doc().unwrap().node(f).unwrap().layout.clone().unwrap();
    assert_eq!((l.gap, l.main_align), (0.0, MainAlign::Min));
    // Gaps 5 and 6 average 5.5, which rounds to 6.
    let mut s = Session::new();
    let ids: Vec<NodeId> = [0.0, 45.0, 91.0].iter().map(|x| rect_at(&mut s, None, *x, 0.0, 40.0, 40.0)).collect();
    let f = add_layout(&mut s, &ids);
    assert_eq!(s.doc().unwrap().node(f).unwrap().layout.clone().unwrap().gap, 6.0);
    // Fractional left and top offsets round to whole padding.
    let mut s = Session::new();
    let f = filled_frame(&mut s);
    rect_at(&mut s, Some(f), 20.4, 30.6, 40.0, 40.0);
    rect_at(&mut s, Some(f), 120.0, 30.6, 40.0, 40.0);
    add_layout(&mut s, &[f]);
    assert_eq!(s.doc().unwrap().node(f).unwrap().layout.clone().unwrap().padding, [31.0, 20.0, 31.0, 20.0]);
}

#[test]
fn shift_a_on_a_frame_with_one_child_keeps_the_inferred_layout() {
    let mut s = Session::new();
    let f = filled_frame(&mut s);
    rect_at(&mut s, Some(f), 20.0, 30.0, 40.0, 40.0);
    add_layout(&mut s, &[f]);
    let n = s.doc().unwrap().node(f).unwrap();
    let l = n.layout.clone().unwrap();
    assert_eq!((l.mode, l.gap, l.padding, l.main_align), (LayoutMode::Horizontal, 10.0, [30.0, 240.0, 130.0, 20.0], MainAlign::Min));
    assert_eq!((n.child_layout.sizing_h, n.child_layout.sizing_v), (Sizing::Hug, Sizing::Hug));
    assert_eq!((n.width, n.height), (300.0, 200.0));
}

#[test]
fn align_is_unavailable_for_a_child_in_an_auto_layout_flow() {
    use crate::cmd::arrange::{align_blocked_by_auto_layout, can_align};
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({"gap": 10}));
    let a = rect(&mut s, f, 20.0, 20.0, "#ff0000");
    let b = rect(&mut s, f, 20.0, 20.0, "#00ff00");
    let at = |s: &Session, i: NodeId| s.doc().unwrap().world_bounds(i).unwrap();
    let (before_a, before_b) = (at(&s, a), at(&s, b));
    assert_eq!(before_b.x0, 30.0, "the layout placed the second one after the first");
    let doc = s.doc().unwrap();
    assert!(!can_align(doc, &[b]), "Align is greyed out for one child of the frame");
    assert!(align_blocked_by_auto_layout(doc, &[b]), "because its frame places it");
    for cmd in ["align.left", "align.hcenter", "align.right", "align.top", "align.vcenter", "align.bottom"] {
        s.execute(cmd, &json!({"ids": [b]})).unwrap();
        assert_eq!((at(&s, a), at(&s, b)), (before_a, before_b), "{cmd} leaves the layout alone");
    }
}

#[test]
fn an_absolute_child_of_an_auto_layout_frame_still_aligns_to_it() {
    use crate::cmd::arrange::{align_blocked_by_auto_layout, can_align};
    let mut s = Session::new();
    let f = al_frame(&mut s, json!({}));
    let kid = rect(&mut s, f, 20.0, 20.0, "#ff0000");
    set(&mut s, kid, json!({"absolute": true}));
    let doc = s.doc().unwrap();
    assert!(can_align(doc, &[kid]));
    assert!(!align_blocked_by_auto_layout(doc, &[kid]));
    s.execute("align.right", &json!({"ids": [kid]})).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(kid).unwrap().x1, 200.0);
}
