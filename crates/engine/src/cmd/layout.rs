//! Auto layout commands beyond `layout.add` / `layout.remove` (in `structure`) and the layout
//! keys of `node.set` (gap, padding, alignment, sizing, min/max, absolute, grid cells): grid tracks.

use figcraft_doc::{GridTrack, LayoutMode, Sizing};
use serde_json::Value;

use super::props::sizing;
use super::{CommandSpec, ok};
use crate::params::{get, num, req_num, req_targets, string};
use crate::{Result, Session, bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "layout.setTrack",
        "Set grid track size",
        "{ids?, axis: columns|rows, index (0-based), sizing: fixed|hug|fill, value? (fixed: px, default the track's current size; fill: fr, default 1)}",
        true,
        set_track
    )]
}

/// Same cap the layout pass applies to grid track counts.
const MAX_TRACKS: usize = 1000;

fn set_track(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layout.setTrack";
    let ids = req_targets(CMD, s, p)?;
    let columns = match string(p, "axis").map(|a| a.to_ascii_lowercase()).as_deref() {
        Some("columns" | "column" | "col" | "x") => true,
        Some("rows" | "row" | "y") => false,
        _ => return Err(bad(CMD, "`axis` must be columns or rows")),
    };
    let index = req_num(CMD, p, "index")?;
    let kind = get(p, "sizing").and_then(sizing).ok_or_else(|| bad(CMD, "`sizing` must be fixed, hug or fill"))?;
    let value = num(p, "value");
    let doc = s.doc_mut()?;
    for id in ids {
        // Current track sizes (switching to Fixed keeps the size the track has now).
        let tracks_now = figcraft_layout::grid_geometry(doc, id).map(|g| if columns { g.columns } else { g.rows }).unwrap_or_default();
        let n = doc.get_mut(id)?;
        let Some(l) = n.layout.as_mut().filter(|l| l.mode == LayoutMode::Grid) else {
            return Err(bad(CMD, format!("node {id} is not a grid auto layout frame")));
        };
        let count = tracks_now.len().max(if columns { l.grid_columns } else { l.grid_rows } as usize).min(MAX_TRACKS);
        if !(0.0..count as f64).contains(&index) {
            return Err(bad(CMD, format!("`index` must be 0..{count}")));
        }
        let i = index as usize;
        let track = match kind {
            Sizing::Fixed => {
                let current = tracks_now.get(i).map(|(a, b)| b - a).unwrap_or(100.0);
                GridTrack::fixed(value.unwrap_or(current).clamp(0.0, 1e6))
            }
            Sizing::Hug => GridTrack::hug(),
            Sizing::Fill => GridTrack::fill(value.unwrap_or(1.0).clamp(0.0, 1e4)),
        };
        let tracks = if columns { &mut l.column_tracks } else { &mut l.row_tracks };
        if tracks.len() <= i {
            tracks.resize(i + 1, GridTrack::default());
        }
        if let Some(t) = tracks.get_mut(i) {
            *t = track;
        }
    }
    ok()
}

#[cfg(test)]
mod tests {
    use figcraft_doc::{CrossAlign, Document, GridTrack, LayoutMode, Node, NodeId, Sizing};
    use serde_json::{Value, json};

    use crate::{EngineError, Session, params};

    fn id(v: &Value) -> NodeId {
        params::id_of(&v["id"]).unwrap()
    }

    fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
        s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
    }

    fn node(s: &Session, id: NodeId) -> Node {
        s.doc().unwrap().node(id).unwrap().clone()
    }

    fn x(s: &Session, id: NodeId) -> (f64, f64) {
        let c = node(s, id).transform.as_coeffs();
        (c[4], c[5])
    }

    /// A horizontal auto layout frame (padding 0, gap 10) with two 40×20 rectangles.
    fn al_frame(s: &mut Session) -> (NodeId, NodeId, NodeId) {
        let f = id(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 100, "props": {"layout": "horizontal", "gap": 10}})));
        let a = id(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20, "parent": f})));
        let b = id(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20, "parent": f})));
        (f, a, b)
    }

    #[test]
    fn min_max_props_undo_and_order() {
        let mut s = Session::new();
        let (f, a, _) = al_frame(&mut s);
        ex(&mut s, "node.set", json!({"id": f, "props": {"sizingH": "hug"}}));
        assert_eq!(node(&s, f).width, 90.0);
        ex(&mut s, "node.set", json!({"id": f, "props": {"minWidth": 200}}));
        assert_eq!(node(&s, f).width, 200.0);
        // A max below the min pulls the min down with it.
        ex(&mut s, "node.set", json!({"id": f, "props": {"maxWidth": 150}}));
        let n = node(&s, f);
        assert_eq!((n.child_layout.min_w, n.child_layout.max_w, n.width), (Some(150.0), Some(150.0), 150.0));
        s.undo().unwrap();
        assert_eq!(node(&s, f).width, 200.0);
        // null removes the limit.
        ex(&mut s, "node.set", json!({"id": f, "props": {"minWidth": null, "maxWidth": null}}));
        assert_eq!(node(&s, f).width, 90.0);
        // Child limits clamp a fixed child; negatives become 0; junk is refused and rolled back.
        ex(&mut s, "node.set", json!({"id": a, "props": {"minWidth": 70, "maxHeight": -5}}));
        let n = node(&s, a);
        assert_eq!((n.width, n.height, n.child_layout.max_h), (70.0, 0.0, Some(0.0)));
        let before = s.ds().unwrap().history.undo.len();
        assert!(s.execute("node.set", &json!({"id": a, "props": {"minWidth": "wide"}})).is_err());
        assert!(s.execute("node.set", &json!({"id": a, "props": {"maxWidth": [1]}})).is_err());
        assert_eq!(s.ds().unwrap().history.undo.len(), before);
        assert_eq!(node(&s, a).child_layout.min_w, Some(70.0));
    }

    #[test]
    fn absolute_child_keeps_place_and_follows_constraints() {
        let mut s = Session::new();
        let (f, a, b) = al_frame(&mut s);
        ex(&mut s, "node.set", json!({"id": b, "props": {"sizingH": "fill"}}));
        let at = x(&s, b);
        ex(&mut s, "node.set", json!({"id": b, "props": {"absolute": true, "constraints": {"horizontal": "right"}}}));
        let n = node(&s, b);
        assert!(n.child_layout.absolute);
        assert_eq!(n.child_layout.sizing_h, Sizing::Fixed, "an absolute child can't fill");
        assert_eq!(x(&s, b), at, "toggling absolute keeps the child where it is");
        // The flow ignores it: changing the gap moves nothing else.
        ex(&mut s, "node.set", json!({"id": f, "props": {"paddingLeft": 30}}));
        assert_eq!(x(&s, a).0, 30.0);
        assert_eq!(x(&s, b), at);
        // Resizing the frame applies its constraints (right: keeps its right margin).
        let right_margin = 300.0 - at.0 - node(&s, b).width;
        ex(&mut s, "node.set", json!({"id": f, "props": {"width": 400}}));
        assert!((400.0 - x(&s, b).0 - node(&s, b).width - right_margin).abs() < 1e-9);
        // Undo the resize, then back into the flow.
        s.undo().unwrap();
        assert_eq!(x(&s, b), at);
        ex(&mut s, "node.set", json!({"id": b, "props": {"absolute": false}}));
        assert_eq!(x(&s, b).0, 30.0 + 40.0 + 10.0);
        assert!(s.execute("node.set", &json!({"id": b, "props": {"absolute": "maybe"}})).is_err());
    }

    /// Each side key writes its own padding slot and leaves the other three alone.
    #[test]
    fn each_side_padding_sets_its_side_alone() {
        let mut s = Session::new();
        let (f, _, _) = al_frame(&mut s);
        // One side at a time: the other three stay 0 ([top, right, bottom, left]).
        for (key, v, want) in [
            ("paddingTop", 8.0, [8.0, 0.0, 0.0, 0.0]),
            ("paddingRight", 12.0, [0.0, 12.0, 0.0, 0.0]),
            ("paddingBottom", 16.0, [0.0, 0.0, 16.0, 0.0]),
            ("paddingLeft", 20.0, [0.0, 0.0, 0.0, 20.0]),
        ] {
            ex(&mut s, "node.set", json!({"id": f, "props": {"padding": 0}}));
            ex(&mut s, "node.set", json!({"id": f, "props": {key: v}}));
            assert_eq!(node(&s, f).layout.as_ref().unwrap().padding, want, "{key}");
        }
        // The [t, r, b, l] array and the axis pairs agree with the four sides.
        ex(&mut s, "node.set", json!({"id": f, "props": {"padding": [8, 12, 16, 20]}}));
        assert_eq!(node(&s, f).layout.as_ref().unwrap().padding, [8.0, 12.0, 16.0, 20.0]);
        ex(&mut s, "node.set", json!({"id": f, "props": {"paddingHorizontal": 4}}));
        assert_eq!(node(&s, f).layout.as_ref().unwrap().padding, [8.0, 4.0, 16.0, 4.0]);
        ex(&mut s, "node.set", json!({"id": f, "props": {"paddingVertical": 2}}));
        assert_eq!(node(&s, f).layout.as_ref().unwrap().padding, [2.0, 4.0, 2.0, 4.0]);
        // Negatives clamp to 0; junk is refused.
        ex(&mut s, "node.set", json!({"id": f, "props": {"paddingTop": -5}}));
        assert_eq!(node(&s, f).layout.as_ref().unwrap().padding[0], 0.0);
        assert!(s.execute("node.set", &json!({"id": f, "props": {"paddingLeft": "wide"}})).is_err());
    }

    #[test]
    fn junk_axis_padding_is_refused_and_keeps_padding() {
        let mut s = Session::new();
        let (f, _, _) = al_frame(&mut s);
        ex(&mut s, "node.set", json!({"id": f, "props": {"padding": 16}}));
        for key in ["paddingHorizontal", "paddingVertical"] {
            for junk in [json!("abc"), json!({"a": 1}), json!([1, 2])] {
                assert!(s.execute("node.set", &json!({"id": f, "props": {key: junk}})).is_err(), "{key}");
                assert_eq!(node(&s, f).layout.as_ref().unwrap().padding, [16.0; 4], "{key}");
            }
        }
    }

    #[test]
    fn baseline_alignment_lines_up_text() {
        let mut s = Session::new();
        let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 100, "props": {"layout": "horizontal"}})));
        let small = id(&ex(&mut s, "text.create", json!({"text": "Small", "x": 0, "y": 0, "parent": f, "props": {"fontSize": 12}})));
        let big = id(&ex(&mut s, "text.create", json!({"text": "Big", "x": 0, "y": 0, "parent": f, "props": {"fontSize": 48}})));
        ex(&mut s, "node.set", json!({"id": f, "props": {"crossAlign": "baseline"}}));
        assert_eq!(node(&s, f).layout.unwrap().cross_align, CrossAlign::Baseline);
        let base =
            |s: &Session, t: NodeId| figcraft_text::layout_node(&node(s, t)).and_then(|(l, dy)| l.lines.first().map(|line| line.baseline + dy));
        // Only meaningful when the text was measured (fonts available).
        if let (Some(bs), Some(bb)) = (base(&s, small), base(&s, big))
            && (bb - bs).abs() > 1.0
        {
            assert!((x(&s, small).1 + bs - (x(&s, big).1 + bb)).abs() < 1e-6);
        }
        // Vertical layouts treat baseline as left.
        ex(&mut s, "node.set", json!({"id": f, "props": {"layout": "vertical"}}));
        assert_eq!(x(&s, small).0, x(&s, big).0);
    }

    #[test]
    fn grid_tracks_cells_and_undo() {
        let mut s = Session::new();
        let f = id(&ex(
            &mut s,
            "frame.create",
            json!({"x": 0, "y": 0, "width": 320, "height": 200, "props": {"layout": "grid", "gridColumns": 3, "columnGap": 10, "rowGap": 10}}),
        ));
        let kids: Vec<NodeId> =
            (0..4).map(|_| id(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20, "parent": f})))).collect();
        ex(&mut s, "node.set", json!({"id": f, "props": {"columnTracks": [100, "1fr", "auto"]}}));
        let l = node(&s, f).layout.unwrap();
        assert_eq!(l.column_tracks, vec![GridTrack::fixed(100.0), GridTrack::fill(1.0), GridTrack::hug()]);
        // 320 - 20 gaps - 100 - 20 (hug) = 180 for the 1fr column.
        assert_eq!(x(&s, kids[1]).0, 110.0);
        assert_eq!(x(&s, kids[2]).0, 300.0);
        // layout.setTrack: switching to fixed keeps the current size.
        ex(&mut s, "layout.setTrack", json!({"ids": [f], "axis": "columns", "index": 1, "sizing": "fixed"}));
        assert_eq!(node(&s, f).layout.unwrap().column_tracks.get(1).copied(), Some(GridTrack::fixed(180.0)));
        ex(&mut s, "layout.setTrack", json!({"ids": [f], "axis": "rows", "index": 0, "sizing": "fixed", "value": 50}));
        assert_eq!(node(&s, f).layout.unwrap().row_tracks, vec![GridTrack::fixed(50.0)]);
        s.undo().unwrap();
        assert!(node(&s, f).layout.unwrap().row_tracks.is_empty());
        // Cells: anchor the last child at column 2, row 1, span the first over two columns.
        ex(&mut s, "node.set", json!({"id": kids[3], "props": {"gridColumn": 2, "gridRow": 1}}));
        ex(&mut s, "node.set", json!({"id": kids[0], "props": {"gridColumnSpan": 2, "sizingH": "fill"}}));
        assert_eq!(node(&s, kids[0]).width, 100.0 + 10.0 + 180.0);
        let info = crate::inspect::node_json(s.doc().unwrap(), kids[3], 0);
        assert_eq!(info["gridCell"]["column"], json!(2));
        assert_eq!(info["gridCell"]["row"], json!(1));
        assert_eq!(info["gridCell"]["anchored"], json!(true));
        ex(&mut s, "node.set", json!({"id": kids[3], "props": {"gridColumn": "auto", "gridRow": null}}));
        assert_eq!(node(&s, kids[3]).child_layout.grid_column, None);
        // Fewer columns drop the extra tracks.
        ex(&mut s, "node.set", json!({"id": f, "props": {"gridColumns": 2}}));
        assert_eq!(node(&s, f).layout.unwrap().column_tracks.len(), 2);
        // Errors roll back.
        for bad in [
            json!({"ids": [f], "axis": "diagonal", "index": 0, "sizing": "fixed"}),
            json!({"ids": [f], "axis": "columns", "index": 9, "sizing": "fixed"}),
            json!({"ids": [f], "axis": "columns", "index": -1, "sizing": "hug"}),
            json!({"ids": [f], "axis": "columns", "index": 0, "sizing": "stretchy"}),
            json!({"ids": [kids[1]], "axis": "columns", "index": 0, "sizing": "hug"}),
        ] {
            assert!(s.execute("layout.setTrack", &bad).is_err(), "{bad}");
        }
        assert!(s.execute("node.set", &json!({"id": f, "props": {"columnTracks": ["1fr", "wide"]}})).is_err());
        assert!(s.execute("node.set", &json!({"id": f, "props": {"rowTracks": 5}})).is_err());
    }

    #[test]
    fn set_track_clamps_hostile_grid_counts() {
        let mut s = Session::new();
        let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 320, "height": 200, "props": {"layout": "grid"}})));
        // A crafted file can carry a count node.set would have clamped.
        s.doc_mut().unwrap().get_mut(f).unwrap().layout.as_mut().unwrap().grid_columns = 5_000_000;
        let p = json!({"ids": [f], "axis": "columns", "index": 4_999_999, "sizing": "hug"});
        assert!(s.execute("layout.setTrack", &p).is_err());
        assert!(node(&s, f).layout.unwrap().column_tracks.len() <= 1000);
    }

    #[test]
    fn layout_props_survive_hostile_values() {
        let junk = [json!(null), json!("NaN"), json!(-1e308), json!(1e308), json!([]), json!({}), json!(true), json!("-5fr")];
        let keys = [
            "minWidth",
            "maxWidth",
            "minHeight",
            "maxHeight",
            "absolute",
            "columnTracks",
            "rowTracks",
            "gridColumn",
            "gridRow",
            "gridColumnSpan",
            "gridRowSpan",
            "gridColumns",
            "gridRows",
            "columnGap",
            "rowGap",
            "crossAlign",
        ];
        for k in keys {
            for j in &junk {
                let mut s = Session::new();
                let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "props": {"layout": "grid"}})));
                let r = id(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10, "parent": f})));
                for target in [f, r] {
                    if let Err(EngineError::Internal { cmd, msg }) = s.execute("node.set", &json!({"id": target, "props": {k: j}})) {
                        panic!("{cmd} panicked on {k}={j}: {msg}");
                    }
                }
                let doc: &Document = s.doc().unwrap();
                for n in [f, r] {
                    let n = doc.node(n).unwrap();
                    assert!(n.width.is_finite() && n.height.is_finite(), "{k}={j}");
                }
                assert!(doc.node(f).unwrap().layout.as_ref().is_some_and(|l| l.mode == LayoutMode::Grid));
            }
        }
        // Tracks as track objects and extreme strings.
        let t: Vec<Option<GridTrack>> =
            [json!({"sizing": "fill", "value": 2}), json!({"sizing": "fixed"}), json!("120px"), json!("1e400fr"), json!("hug")]
                .iter()
                .map(super::super::props::track_of)
                .collect();
        assert_eq!(t, vec![Some(GridTrack::fill(2.0)), None, Some(GridTrack::fixed(120.0)), None, Some(GridTrack::hug())]);
    }
}
