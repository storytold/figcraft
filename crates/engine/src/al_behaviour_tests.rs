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
