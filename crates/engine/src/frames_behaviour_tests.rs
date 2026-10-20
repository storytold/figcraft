//! Frames, groups and sections behaviour suite: each case is a rule from the public documentation
//! of frames, groups and sections (frames vs. groups when resized, constraints, clip content,
//! frame selection, grouping and ungrouping, X/Y relative to the frame, click targets, drawing a
//! frame or section around layers, dropping into and out of frames, where sections may go,
//! resizing sections) with the result the rule implies. The comment on each case is the one-line
//! derivation. Where the documentation is silent the case says so and pins today's behaviour.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_geom::Point;

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn frame(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(ex(s, "frame.create", p))
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h, "props": {"fills": ["#FF0000"]}});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(ex(s, "shape.rectangle", p))
}

fn section(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id(ex(s, "node.create", json!({"type": "section", "x": x, "y": y, "width": w, "height": h})))
}

/// Page bounds as (x, y, w, h).
fn wb(s: &Session, n: NodeId) -> (f64, f64, f64, f64) {
    let b = s.doc().unwrap().world_bounds(n).unwrap();
    (b.x0, b.y0, b.width(), b.height())
}

fn parent(s: &Session, n: NodeId) -> Option<NodeId> {
    s.doc().unwrap().node(n).unwrap().parent
}

fn ev(kind: PointerKind, x: f64, y: f64, cmd: bool) -> PointerEvent {
    let mut e = PointerEvent::at(kind, x, y, 1.0);
    e.cmd = cmd;
    e
}

/// Press at `a`, drag to `b` in a few steps and release.
fn drag(s: &mut Session, a: (f64, f64), b: (f64, f64), cmd: bool) {
    s.pointer(&ev(PointerKind::Down, a.0, a.1, cmd)).unwrap();
    for k in 1..=4 {
        let t = f64::from(k) / 4.0;
        s.pointer(&ev(PointerKind::Move, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, cmd)).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, b.0, b.1, cmd)).unwrap();
}

/// The RGBA of `node` rendered at 1x, at (`x`, `y`) of its export box.
fn pixel(s: &Session, node: NodeId, x: u32, y: u32) -> [u8; 4] {
    let pix = figcraft_render::render_node(s.doc().unwrap(), node, 1.0).unwrap();
    let i = ((y * pix.width() + x) * 4) as usize;
    let d = pix.data();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

#[test]
fn resizing_a_frame_applies_constraints_and_a_group_scales() {
    // A 100-wide frame with a 20-wide child at x 10 is resized to 200 (one edit each).
    let cases = [
        // Left keeps the distance to the left edge.
        ("left", (10.0, 20.0)),
        // Right keeps the distance to the right edge: 100 - 30 = 70, so 200 - 70 - 20 = 110.
        ("right", (110.0, 20.0)),
        // Left & right keep both distances: 10 on the left, 70 on the right, 200 - 80 wide.
        ("leftRight", (10.0, 120.0)),
        // Center keeps the offset from the centre: moves by (200 - 100) / 2.
        ("center", (60.0, 20.0)),
        // Scale keeps the proportions: x and width × 2.
        ("scale", (20.0, 40.0)),
    ];
    for (h, want) in cases {
        let mut s = Session::new();
        let f = frame(&mut s, 0.0, 0.0, 100.0, 100.0, None);
        let c = rect(&mut s, 10.0, 10.0, 20.0, 20.0, Some(f));
        ex(&mut s, "node.set", json!({"id": c, "props": {"constraints": {"horizontal": h}}}));
        ex(&mut s, "node.set", json!({"id": f, "props": {"width": 200}}));
        let (x, _, w, _) = wb(&s, c);
        assert_eq!((x, w), want, "{h}");
        s.undo().unwrap();
        assert_eq!(wb(&s, c).0, 10.0, "{h}: one undo step");
    }
    // Groups have no constraints for their layers: resizing one scales what's in it (0..20 and
    // 30..50 in a 50-wide group resized to 100: both × 2).
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0, None);
    let b = rect(&mut s, 30.0, 0.0, 20.0, 20.0, None);
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    ex(&mut s, "node.set", json!({"id": g, "props": {"width": 100}}));
    assert_eq!((wb(&s, a), wb(&s, b)), ((0.0, 0.0, 40.0, 20.0), (60.0, 0.0, 40.0, 20.0)));
}

#[test]
fn cmd_resize_of_a_frame_ignores_constraints() {
    // Documented: holding Cmd (Ctrl) while resizing a frame leaves its layers as they are.
    for (cmd, want_x, want_w) in [(false, 170.0, 120.0), (true, 70.0, 20.0)] {
        let mut s = Session::new();
        let f = frame(&mut s, 0.0, 0.0, 100.0, 100.0, None);
        let c = rect(&mut s, 70.0, 40.0, 20.0, 20.0, Some(f));
        let wide = rect(&mut s, 10.0, 70.0, 20.0, 20.0, Some(f));
        ex(&mut s, "node.set", json!({"id": c, "props": {"constraints": {"horizontal": "right"}}}));
        ex(&mut s, "node.set", json!({"id": wide, "props": {"constraints": {"horizontal": "leftRight"}}}));
        ex(&mut s, "select.set", json!({"ids": [f]}));
        // The right edge's handle, dragged 100 to the right.
        drag(&mut s, (100.0, 50.0), (200.0, 50.0), cmd);
        assert_eq!(wb(&s, f).2, 200.0, "cmd {cmd}");
        // Right: 70 + 100, left & right: 20 + 100 wide; with Cmd both stay as they were.
        assert_eq!((wb(&s, c).0, wb(&s, wide).2), (want_x, want_w), "cmd {cmd}");
    }
}

#[test]
fn resizing_a_section_leaves_its_contents() {
    // Documented: resizing a section doesn't move or resize what's in it (no constraints there).
    let mut s = Session::new();
    let sec = section(&mut s, 0.0, 0.0, 400.0, 300.0);
    let f = frame(&mut s, 300.0, 200.0, 50.0, 50.0, Some(sec));
    ex(&mut s, "node.set", json!({"id": f, "props": {"constraints": {"horizontal": "scale", "vertical": "bottom"}}}));
    let before = wb(&s, f);
    ex(&mut s, "node.set", json!({"id": sec, "props": {"width": 800, "height": 600}}));
    assert_eq!(wb(&s, f), before, "a field edit");
    // Dragging the left edge: the section's origin moves, the frame doesn't.
    ex(&mut s, "select.set", json!({"ids": [sec]}));
    drag(&mut s, (0.0, 300.0), (-100.0, 300.0), false);
    assert_eq!(wb(&s, sec), (-100.0, 0.0, 900.0, 600.0));
    assert_eq!(wb(&s, f), before, "a handle drag");
    // Anchored resizes too.
    ex(&mut s, "transform.resize", json!({"ids": [sec], "width": 500, "anchor": "r"}));
    assert_eq!(wb(&s, f), before, "transform.resize");
}

#[test]
fn clip_content_hides_and_unhides_what_overflows() {
    // Documented: with Clip content on, what lies outside the frame isn't shown (or clickable);
    // off, it is. A 100×100 frame with a child from 80 to 140.
    for (clip, hit_outside, export_w) in [(true, false, 100), (false, true, 140)] {
        let mut s = Session::new();
        let f = frame(&mut s, 0.0, 0.0, 100.0, 100.0, None);
        let c = rect(&mut s, 80.0, 10.0, 60.0, 20.0, Some(f));
        ex(&mut s, "node.set", json!({"id": f, "props": {"clip": clip}}));
        let page = s.page().unwrap();
        let hit = s.doc().unwrap().hit_test(page, Point::new(120.0, 20.0), 0.0, None);
        assert_eq!(hit == Some(c), hit_outside, "clip {clip}");
        assert_eq!(figcraft_render::render_node(s.doc().unwrap(), f, 1.0).unwrap().width(), export_w, "clip {clip}");
    }
    // New frames clip (silent on the default for Frame selection: pinned to the frame default).
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 10.0, 10.0, None);
    assert!(s.doc().unwrap().node(f).unwrap().clip);
}

#[test]
fn frame_selection_wraps_the_layers_where_the_topmost_was() {
    // Documented: Frame selection puts the selected layers in a new frame the size of their
    // bounds, in the place of the topmost selected layer; the layers don't move on the page.
    let mut s = Session::new();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0, None);
    let other = rect(&mut s, 200.0, 0.0, 10.0, 10.0, None);
    let b = rect(&mut s, 50.0, 40.0, 30.0, 30.0, None);
    let top = rect(&mut s, 300.0, 0.0, 10.0, 10.0, None);
    let f = id(ex(&mut s, "object.frameSelection", json!({"ids": [b, a]})));
    let doc = s.doc().unwrap();
    // Bounds: 10..80 by 10..70.
    assert_eq!(wb(&s, f), (10.0, 10.0, 70.0, 60.0));
    assert_eq!(doc.node(f).unwrap().children, vec![a, b], "the layers keep their order");
    let page = s.page().unwrap();
    assert_eq!(doc.node(page).unwrap().children, vec![other, f, top], "where b (the topmost selected) was");
    assert_eq!((wb(&s, a), wb(&s, b)), ((10.0, 10.0, 20.0, 20.0), (50.0, 40.0, 30.0, 30.0)));
    assert_eq!(s.selection(), vec![f]);
    // Inside a frame, the new frame nests there.
    let outer = frame(&mut s, 500.0, 0.0, 200.0, 200.0, None);
    let c = rect(&mut s, 20.0, 20.0, 10.0, 10.0, Some(outer));
    let inner = id(ex(&mut s, "object.frameSelection", json!({"ids": [c]})));
    assert_eq!(parent(&s, inner), Some(outer));
    // One undo step.
    s.undo().unwrap();
    assert_eq!(parent(&s, c), Some(outer));
}

#[test]
fn grouping_and_ungrouping_keep_layers_in_place() {
    // Documented: a group's bounds hug its layers; grouping and ungrouping don't move anything,
    // also once the group is rotated; ungrouping selects the layers.
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 20.0, None);
    let b = rect(&mut s, 60.0, 30.0, 20.0, 20.0, None);
    ex(&mut s, "node.set", json!({"id": b, "props": {"rotation": 45}}));
    let before = (wb(&s, a), wb(&s, b));
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    assert_eq!((wb(&s, a), wb(&s, b)), before);
    let hug = s.doc().unwrap().bounds_of(&[a, b]).unwrap();
    assert_eq!(wb(&s, g), (hug.x0, hug.y0, hug.width(), hug.height()), "the group hugs its layers");
    // Rotating the group turns its layers with it; ungrouping leaves them where they ended up.
    ex(&mut s, "transform.rotate", json!({"ids": [g], "degrees": 30}));
    let worlds = |s: &Session| [a, b].map(|i| s.doc().unwrap().world_transform(i));
    let turned = worlds(&s);
    ex(&mut s, "object.ungroup", json!({"ids": [g]}));
    for (x, y) in worlds(&s).iter().zip(turned.iter()) {
        let (p, q) = (x.as_coeffs(), y.as_coeffs());
        assert!(p.iter().zip(q.iter()).all(|(u, v)| (u - v).abs() < 1e-9), "{p:?} {q:?}");
    }
    assert!(s.doc().unwrap().node(g).is_none());
    assert_eq!(s.selection(), vec![a, b]);
    // Nested groups: ungrouping the outer one keeps the inner one.
    let c = rect(&mut s, 200.0, 0.0, 10.0, 10.0, None);
    let inner = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    let outer = id(ex(&mut s, "object.group", json!({"ids": [inner, c]})));
    ex(&mut s, "object.ungroup", json!({"ids": [outer]}));
    assert_eq!(s.doc().unwrap().node(inner).unwrap().children, vec![a, b]);
    // Silent on what ungrouping does with the group's opacity: pinned (multiplied into the layers).
    let g = id(ex(&mut s, "object.group", json!({"ids": [c]})));
    ex(&mut s, "node.set", json!({"id": g, "props": {"opacity": 0.5}}));
    ex(&mut s, "object.ungroup", json!({"ids": [g]}));
    assert_eq!(s.doc().unwrap().node(c).unwrap().opacity, 0.5);
}

#[test]
fn group_bounds_follow_their_layers_and_empty_groups_go() {
    // Documented: a group is as big as its layers; it changes when they do, and deleting its last
    // layer deletes it.
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0, None);
    let b = rect(&mut s, 30.0, 0.0, 20.0, 20.0, None);
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    ex(&mut s, "transform.move", json!({"ids": [b], "dx": 50, "dy": 10}));
    assert_eq!(wb(&s, g), (0.0, 0.0, 100.0, 30.0));
    ex(&mut s, "edit.delete", json!({"ids": [a, b]}));
    assert!(s.doc().unwrap().node(g).is_none());
}

#[test]
fn group_opacity_applies_to_the_whole_group() {
    // Documented: a group's opacity applies to the group as one, so overlapping layers in a 50%
    // group don't show through each other; two 50% layers do (alpha 0.5 + 0.5 × 0.5 = 0.75).
    let mut s = Session::new();
    let a = rect(&mut s, 0.0, 0.0, 30.0, 10.0, None);
    let b = rect(&mut s, 20.0, 0.0, 30.0, 10.0, None);
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    ex(&mut s, "node.set", json!({"id": g, "props": {"opacity": 0.5}}));
    let (alone, overlap) = (pixel(&s, g, 5, 5)[3], pixel(&s, g, 25, 5)[3]);
    assert!(alone.abs_diff(128) <= 1 && overlap == alone, "{alone} {overlap}");
    ex(&mut s, "node.set", json!({"ids": [a, b], "props": {"opacity": 0.5}}));
    ex(&mut s, "node.set", json!({"id": g, "props": {"opacity": 1}}));
    assert!(pixel(&s, g, 25, 5)[3].abs_diff(191) <= 1);
}

#[test]
fn x_and_y_are_relative_to_the_frame_not_the_group() {
    // Documented: X and Y are measured from the top-left of the frame the layer is in (the canvas
    // if none); groups don't start a coordinate space of their own.
    let mut s = Session::new();
    let f = frame(&mut s, 100.0, 100.0, 300.0, 300.0, None);
    let a = rect(&mut s, 30.0, 40.0, 20.0, 20.0, Some(f));
    let b = rect(&mut s, 80.0, 60.0, 20.0, 20.0, Some(f));
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    let xy = |s: &mut Session, n: NodeId| {
        let v = ex(s, "node.get", json!({"id": n}));
        (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap())
    };
    let cases = [
        // The group sits at the top-left of its layers, in the frame.
        (g, (30.0, 40.0)),
        // Its first layer is where the group starts: 30, 40 in the frame, not 0, 0 in the group.
        (a, (30.0, 40.0)),
        (b, (80.0, 60.0)),
    ];
    for (n, want) in cases {
        assert_eq!(xy(&mut s, n), want, "{n}");
    }
    // Setting X places the layer in the frame; the group follows (b now from 50 to 70).
    ex(&mut s, "node.set", json!({"id": b, "props": {"x": 50}}));
    assert_eq!(xy(&mut s, b), (50.0, 60.0));
    assert_eq!(wb(&s, b).0, 150.0);
    assert_eq!(wb(&s, g), (130.0, 140.0, 40.0, 40.0));
    // A group on the canvas: page coordinates.
    let c = rect(&mut s, 500.0, 20.0, 10.0, 10.0, None);
    let d = rect(&mut s, 520.0, 50.0, 10.0, 10.0, None);
    ex(&mut s, "object.group", json!({"ids": [c, d]}));
    assert_eq!(xy(&mut s, d), (520.0, 50.0));
    // A rotated group: its layers' X/Y are still in the frame's axes.
    ex(&mut s, "transform.rotate", json!({"ids": [g], "degrees": 90}));
    let t = s.doc().unwrap().world_transform(a).as_coeffs();
    let (x, y) = xy(&mut s, a);
    assert!((x - (t[4] - 100.0)).abs() < 1e-9 && (y - (t[5] - 100.0)).abs() < 1e-9, "{x} {y}");
}

#[test]
fn clicks_pick_children_of_top_level_frames_and_whole_groups() {
    // Documented: top-level frames (also those in sections) act as boards, so a click picks
    // their direct child; groups and nested frames are picked whole; a double click goes one
    // level in; Cmd/Ctrl-click picks the layer itself.
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 300.0, 200.0, None);
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0, Some(f));
    let b = rect(&mut s, 40.0, 10.0, 20.0, 20.0, Some(f));
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    let nested = frame(&mut s, 100.0, 100.0, 80.0, 80.0, Some(f));
    let deep = rect(&mut s, 10.0, 10.0, 20.0, 20.0, Some(nested));
    let loose = rect(&mut s, 200.0, 20.0, 20.0, 20.0, Some(f));
    let sec = section(&mut s, 0.0, 400.0, 400.0, 300.0);
    let sf = frame(&mut s, 20.0, 20.0, 100.0, 100.0, Some(sec));
    let sr = rect(&mut s, 10.0, 10.0, 20.0, 20.0, Some(sf));
    let doc = s.doc().unwrap().clone();
    let pick = |leaf, ctx: &[NodeId], deep| figcraft_doc::geometry::select_target(&doc, leaf, ctx, deep);
    let enter = |leaf, sel: &[NodeId]| figcraft_doc::geometry::enter_target(&doc, leaf, sel);
    let cases: [(&str, NodeId, NodeId); 10] = [
        // A layer in a group in a top-level frame: the group.
        ("group", pick(a, &[], false), g),
        // A plain child of a top-level frame: itself.
        ("child", pick(loose, &[], false), loose),
        // A layer in a nested frame: the nested frame (a child of the board).
        ("nested frame", pick(deep, &[], false), nested),
        // The frame's own background: the frame.
        ("background", pick(f, &[], false), f),
        // A frame in a section is a top-level frame: its child.
        ("frame in a section", pick(sr, &[], false), sr),
        // Cmd/Ctrl-click: the layer itself.
        ("deep click", pick(a, &[], true), a),
        // With the group selected, a click on its layer picks the layer (siblings resolve there).
        ("in the selected group", pick(b, &[a], false), b),
        // A double click on the selected group goes into it.
        ("double click", enter(a, &[g]), a),
        // A double click on the selected nested frame goes into it.
        ("double click a frame", enter(deep, &[nested]), deep),
        // A double click with nothing selected first picks what a click would.
        ("double click, nothing selected", enter(a, &[]), g),
    ];
    for (rule, got, want) in cases {
        assert_eq!(got, want, "{rule}");
    }
}

#[test]
fn clicking_a_top_level_frame_with_layers_selects_it_and_a_drag_marquees() {
    // Regression: once a top-level frame holds layers, a plain click on its empty background must
    // select the frame (it can't be reached any other way by a click); a drag from there marquees
    // the layers inside it (Figma). Before the fix the background was treated like the canvas and
    // only ever started a marquee, so the frame could never be clicked to select.
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 300.0, 200.0, None);
    // A child that leaves empty background to click and drag over.
    let r = rect(&mut s, 10.0, 10.0, 20.0, 20.0, Some(f));
    // A drag from the (unselected) background that encloses the child marquees the frame's layers.
    s.pointer(&ev(PointerKind::Down, 0.0, 0.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, 150.0, 100.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, 300.0, 200.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Up, 300.0, 200.0, false)).unwrap();
    assert_eq!(s.selection().as_slice(), &[r], "a drag on the frame's background marquees its layers");
    // A plain click on the now-empty background of the frame selects the frame.
    s.pointer(&ev(PointerKind::Down, 150.0, 100.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Up, 150.0, 100.0, false)).unwrap();
    assert_eq!(s.selection().as_slice(), &[f], "a click on the frame's background selects the frame");
    // With the frame already selected, a drag on its background moves it (Figma), not a marquee.
    s.pointer(&ev(PointerKind::Down, 150.0, 100.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Move, 160.0, 110.0, false)).unwrap();
    s.pointer(&ev(PointerKind::Up, 160.0, 110.0, false)).unwrap();
    assert_eq!(s.selection().as_slice(), &[f], "a drag on a selected frame's background moves it");
}

#[test]
fn drawing_a_frame_or_section_around_layers_puts_them_in_it() {
    // Documented: drawing a frame (or a section) around layers puts those fully inside it in the
    // new frame (section); layers only partly inside stay where they are.
    let mut s = Session::new();
    let inside = rect(&mut s, 20.0, 20.0, 30.0, 30.0, None);
    let partly = rect(&mut s, 180.0, 20.0, 40.0, 40.0, None);
    let out = rect(&mut s, 400.0, 400.0, 10.0, 10.0, None);
    let page = s.page().unwrap();
    ex(&mut s, "tool.frame", json!({}));
    drag(&mut s, (0.0, 0.0), (200.0, 200.0), false);
    let f = *s.selection().first().unwrap();
    assert!(matches!(s.doc().unwrap().node(f).unwrap().kind, NodeKind::Frame));
    assert_eq!((parent(&s, inside), parent(&s, partly), parent(&s, out)), (Some(f), Some(page), Some(page)));
    assert_eq!(wb(&s, inside), (20.0, 20.0, 30.0, 30.0), "it doesn't move on the page");
    // One undo step takes the frame away and puts the layer back.
    s.undo().unwrap();
    assert_eq!(parent(&s, inside), Some(page));
    assert!(s.doc().unwrap().node(f).is_none());
    // Inside a frame: the frame's own layers that end up inside the new one.
    let board = frame(&mut s, 1000.0, 0.0, 400.0, 400.0, None);
    let k = rect(&mut s, 50.0, 50.0, 20.0, 20.0, Some(board));
    ex(&mut s, "tool.frame", json!({}));
    drag(&mut s, (1020.0, 20.0), (1120.0, 120.0), false);
    let nf = *s.selection().first().unwrap();
    assert_eq!((parent(&s, nf), parent(&s, k)), (Some(board), Some(nf)));
    // A section drawn around frames takes them (and leaves what's only partly inside).
    ex(&mut s, "tool.section", json!({}));
    drag(&mut s, (-50.0, -50.0), (1500.0, 500.0), false);
    let sec = *s.selection().first().unwrap();
    assert!(matches!(s.doc().unwrap().node(sec).unwrap().kind, NodeKind::Section { .. }));
    assert_eq!((parent(&s, board), parent(&s, inside), parent(&s, out)), (Some(sec), Some(sec), Some(sec)));
    assert_eq!(parent(&s, partly), Some(sec), "fully inside this larger one");
    // A frame drawn around a section leaves the section out (sections don't go in frames).
    let lone = section(&mut s, 3000.0, 0.0, 100.0, 100.0);
    ex(&mut s, "tool.frame", json!({}));
    drag(&mut s, (2900.0, -100.0), (3200.0, 200.0), false);
    assert_eq!(parent(&s, lone), Some(page));
}

#[test]
fn dropping_into_and_pulling_out_of_frames_follows_the_pointer() {
    // Documented: a layer dragged over a frame goes into it, decided by where the pointer is
    // (silent on layers only partly inside: pinned, the pointer decides); dragged out of the frame,
    // it leaves it.
    let mut s = Session::new();
    let page = s.page().unwrap();
    let f = frame(&mut s, 0.0, 0.0, 200.0, 200.0, None);
    let r = rect(&mut s, 300.0, 0.0, 40.0, 40.0, None);
    // Pointer at 190 inside the frame; the rectangle spans 170..210, partly outside.
    drag(&mut s, (320.0, 20.0), (190.0, 20.0), false);
    assert_eq!(parent(&s, r), Some(f));
    assert_eq!(wb(&s, r).0, 170.0);
    drag(&mut s, (190.0, 20.0), (320.0, 20.0), false);
    assert_eq!(parent(&s, r), Some(page));
    // Layers of a group stay in it when dragged (the group follows).
    let a = rect(&mut s, 400.0, 0.0, 20.0, 20.0, None);
    let b = rect(&mut s, 430.0, 0.0, 20.0, 20.0, None);
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    ex(&mut s, "select.set", json!({"ids": [a]}));
    drag(&mut s, (410.0, 10.0), (100.0, 100.0), false);
    assert_eq!(parent(&s, a), Some(g));
}

#[test]
fn sections_go_on_the_page_or_in_sections() {
    // Documented: sections can be nested in sections but can't go in frames or groups.
    let mut s = Session::new();
    let page = s.page().unwrap();
    let f = frame(&mut s, 0.0, 0.0, 300.0, 300.0, None);
    let sec = section(&mut s, 500.0, 0.0, 200.0, 200.0);
    let outer = section(&mut s, 1000.0, 0.0, 600.0, 600.0);
    let r = rect(&mut s, 900.0, 0.0, 10.0, 10.0, None);
    let refused = [
        ("into a frame", json!({"ids": [sec], "parent": f})),
        ("into a group", {
            let g = id(ex(&mut s, "object.group", json!({"ids": [r]})));
            json!({"ids": [sec], "parent": g})
        }),
    ];
    for (rule, p) in refused {
        assert!(s.execute("node.reparent", &p).is_err(), "{rule}");
        assert_eq!(parent(&s, sec), Some(page), "{rule}");
    }
    assert!(s.execute("object.group", &json!({"ids": [sec]})).is_err(), "grouping a section");
    assert!(s.execute("object.frameSelection", &json!({"ids": [sec]})).is_err(), "framing a section");
    assert!(s.execute("node.create", &json!({"type": "section", "parent": f})).is_err(), "a new section in a frame");
    // Into a section: fine.
    ex(&mut s, "node.reparent", json!({"ids": [sec], "parent": outer}));
    assert_eq!(parent(&s, sec), Some(outer));
    ex(&mut s, "node.reparent", json!({"ids": [sec], "parent": page}));
    // Dragged over a frame, a section stays on the page.
    ex(&mut s, "select.set", json!({"ids": [sec]}));
    drag(&mut s, (600.0, 100.0), (150.0, 150.0), false);
    assert_eq!(parent(&s, sec), Some(page));
    assert_eq!(wb(&s, sec).0, 50.0, "but it moves");
    // Pasted with a frame selected: on the page, not in the frame.
    ex(&mut s, "edit.copy", json!({"ids": [sec]}));
    ex(&mut s, "select.set", json!({"ids": [f]}));
    let pasted = ex(&mut s, "edit.paste", json!({}));
    let p = id_of(&pasted["ids"][0]).unwrap();
    assert_eq!(parent(&s, p), Some(page));
}

#[test]
fn top_level_frames_are_screens_and_export_whole() {
    // Documented: top-level frames (on the page or in sections) are the screens of prototypes and
    // the boards that export; a nested frame is neither on its own.
    let mut s = Session::new();
    let page = s.page().unwrap();
    let a = frame(&mut s, 0.0, 0.0, 100.0, 200.0, None);
    let nested = frame(&mut s, 10.0, 10.0, 30.0, 30.0, Some(a));
    let sec = section(&mut s, 300.0, 0.0, 400.0, 400.0);
    let b = frame(&mut s, 20.0, 20.0, 50.0, 60.0, Some(sec));
    let doc = s.doc().unwrap();
    assert_eq!(doc.top_frames(page), vec![a, b]);
    assert_eq!(doc.top_level_frame(nested), Some(a));
    let px = figcraft_render::render_node(doc, a, 2.0).unwrap();
    assert_eq!((px.width(), px.height()), (200, 400), "the frame's own size at 2x");
}

#[test]
fn frames_and_groups_convert_through_frame_selection_and_remove_frame() {
    // Documented: Frame selection wraps a group in a frame, Ungroup / Remove frame take a container
    // away; layers stay where they are either way. (Silent on a one-step convert: not offered.)
    let mut s = Session::new();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0, None);
    let b = rect(&mut s, 50.0, 10.0, 20.0, 20.0, None);
    let g = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    let f = id(ex(&mut s, "object.frameSelection", json!({"ids": [g]})));
    ex(&mut s, "object.ungroup", json!({"ids": [g]}));
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(f).unwrap().children, vec![a, b]);
    assert_eq!((wb(&s, a), wb(&s, f)), ((10.0, 10.0, 20.0, 20.0), (10.0, 10.0, 60.0, 20.0)));
    let g2 = id(ex(&mut s, "object.group", json!({"ids": [a, b]})));
    ex(&mut s, "object.removeFrame", json!({"ids": [f]}));
    assert_eq!(parent(&s, g2), s.page().ok());
    assert_eq!(wb(&s, g2), (10.0, 10.0, 60.0, 20.0));
}
