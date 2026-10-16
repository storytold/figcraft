//! Canvas navigation, rulers, guides, layout grids, outline mode and measuring: a behaviour suite
//! written from the public documentation of design tools. Each rule is a table row with a one-line
//! derivation; where the documentation is silent the row says so and pins today's behaviour.
//!
//! Space-drag and middle-button panning, the rulers' numbering and selection highlight, dragging
//! guides out of the rulers (Alt for a copy) and the context menu's Remove guide are drawn and
//! read by the UI crate from the engine pieces tested here; they're checked in the running app.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use serde_json::{Value, json};

use crate::keymap::{Context, Mods, Platform, lookup};
use crate::params::id_of;
use crate::snap::Targets;
use crate::tools::{PointerEvent, PointerKind};
use crate::{Session, ViewState, distance, guides};
use figcraft_doc::{GridAlign, GridKind, LayoutGrid, NodeId};
use figcraft_geom::{Point, Rect};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn frame(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&ex(s, "frame.create", p))
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&ex(s, "shape.rectangle", p))
}

fn view(s: &Session) -> ViewState {
    s.ds().unwrap().view.clone()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[derive(Clone, Copy, Default)]
struct Mod {
    shift: bool,
    alt: bool,
    cmd: bool,
}

const NO: Mod = Mod { shift: false, alt: false, cmd: false };
const ALT: Mod = Mod { shift: false, alt: true, cmd: false };
const CMD_ALT: Mod = Mod { shift: false, alt: true, cmd: true };

fn ev(kind: PointerKind, x: f64, y: f64, m: Mod) -> PointerEvent {
    let mut e = PointerEvent::at(kind, x, y, 1.0);
    (e.shift, e.alt, e.cmd) = (m.shift, m.alt, m.cmd);
    e
}

fn drag(s: &mut Session, a: (f64, f64), b: (f64, f64)) {
    s.pointer(&ev(PointerKind::Down, a.0, a.1, NO)).unwrap();
    s.pointer(&ev(PointerKind::Move, b.0, b.1, NO)).unwrap();
    s.pointer(&ev(PointerKind::Up, b.0, b.1, NO)).unwrap();
}

fn click(s: &mut Session, x: f64, y: f64) {
    s.pointer(&ev(PointerKind::Down, x, y, NO)).unwrap();
    s.pointer(&ev(PointerKind::Up, x, y, NO)).unwrap();
}

fn origin(s: &Session, n: NodeId) -> (f64, f64) {
    let b = s.doc().unwrap().world_bounds(n).unwrap();
    (b.x0, b.y0)
}

/// The labels of the measurement lines Alt-hovering at `p` with modifiers `m` shows.
fn measured(s: &mut Session, p: (f64, f64), m: Mod) -> Vec<String> {
    s.pointer(&ev(PointerKind::Hover, p.0, p.1, m)).unwrap();
    let mut v: Vec<String> = s.tool.overlay.guides.iter().filter_map(|g| g.label.clone()).collect();
    v.sort();
    v
}

#[test]
fn zoom_steps_presets_and_limits() {
    let mut s = Session::new();
    // (command, zoom before, zoom after, derivation)
    let rows: Vec<(&str, f64, f64, &str)> = vec![
        ("view.zoomIn", 1.0, 2.0, "docs silent on the step: zooming in doubles (pinned)"),
        ("view.zoomOut", 1.0, 0.5, "docs silent on the step: zooming out halves (pinned)"),
        ("view.zoomIn", 1.5, 2.0, "docs silent: between steps, zooming in goes to the next step up (pinned)"),
        ("view.zoomOut", 0.25, 0.13, "docs silent: below 25% the steps are rounded to whole percents (pinned)"),
        ("view.zoomIn", 256.0, 256.0, "docs silent: the most is 25,600% (pinned)"),
        ("view.zoomOut", 0.02, 0.02, "docs silent: the least is 2% (pinned)"),
        ("view.zoom100", 3.0, 1.0, "the zoom presets include 100% (Shift+0)"),
        ("view.zoom50", 3.0, 0.5, "the zoom presets include 50%"),
        ("view.zoom200", 0.3, 2.0, "the zoom presets include 200%"),
    ];
    for (cmd, before, after, why) in rows {
        ex(&mut s, "view.set", json!({"zoom": before, "cx": 100, "cy": 50}));
        ex(&mut s, cmd, json!({}));
        let v = view(&s);
        assert!(near(v.zoom, after), "{why}: {cmd} from {before} gave {}", v.zoom);
        assert!(near(v.cx, 100.0) && near(v.cy, 50.0), "docs silent: keyboard zoom keeps the middle of the view in place (pinned): {cmd}");
    }
}

#[test]
fn zoom_to_fit_selection_and_frames() {
    let mut s = Session::new();
    // Reading order: f1 and f2 share the top row, f3 is below.
    let f1 = frame(&mut s, 0.0, 0.0, 400.0, 300.0, None);
    let f2 = frame(&mut s, 600.0, 0.0, 200.0, 200.0, None);
    let f3 = frame(&mut s, 0.0, 500.0, 100.0, 100.0, None);
    let r = rect(&mut s, 900.0, 0.0, 40.0, 40.0, None);
    ex(&mut s, "view.set", json!({"vw": 1200, "vh": 800}));
    // The zoom that fits a box with the view's 64-point margin.
    let fit = |b: Rect| ((1200.0 - 128.0) / b.width()).min((800.0 - 128.0) / b.height());
    let content = Rect::new(0.0, 0.0, 940.0, 600.0);
    // (command, selection before, zoom, centre, selection after, derivation)
    let rows: Vec<(&str, Vec<NodeId>, f64, (f64, f64), Vec<NodeId>, &str)> = vec![
        ("view.zoomFit", vec![], fit(content), (470.0, 300.0), vec![], "Shift+1 fits everything on the page into the view"),
        ("view.zoomSelection", vec![r], fit(Rect::new(0.0, 0.0, 40.0, 40.0)), (920.0, 20.0), vec![r], "Shift+2 zooms to the selection"),
        (
            "view.nextFrame",
            vec![],
            fit(Rect::new(0.0, 0.0, 400.0, 300.0)),
            (200.0, 150.0),
            vec![f1],
            "N goes to the next frame (from nothing: the first; docs silent that it selects it, pinned)",
        ),
        (
            "view.nextFrame",
            vec![f1],
            fit(Rect::new(0.0, 0.0, 200.0, 200.0)),
            (700.0, 100.0),
            vec![f2],
            "N goes to the next frame (docs silent on the order: reading order, pinned)",
        ),
        (
            "view.nextFrame",
            vec![f3],
            fit(Rect::new(0.0, 0.0, 400.0, 300.0)),
            (200.0, 150.0),
            vec![f1],
            "docs silent: after the last frame N wraps to the first (pinned)",
        ),
        ("view.prevFrame", vec![f2], fit(Rect::new(0.0, 0.0, 400.0, 300.0)), (200.0, 150.0), vec![f1], "Shift+N goes to the previous frame"),
        (
            "view.prevFrame",
            vec![f1],
            fit(Rect::new(0.0, 0.0, 100.0, 100.0)),
            (50.0, 550.0),
            vec![f3],
            "docs silent: before the first frame Shift+N wraps to the last (pinned)",
        ),
    ];
    for (cmd, before, zoom, (cx, cy), after, why) in rows {
        ex(&mut s, "view.set", json!({"zoom": 1, "cx": 0, "cy": 0}));
        ex(&mut s, "select.set", json!({"ids": before}));
        ex(&mut s, cmd, json!({}));
        let v = view(&s);
        assert!(near(v.zoom, zoom) && near(v.cx, cx) && near(v.cy, cy), "{why}: {} at ({}, {})", v.zoom, v.cx, v.cy);
        assert_eq!(s.selection(), after, "{why}");
    }
    // Docs silent: zoom to selection with nothing selected leaves the view alone (pinned).
    ex(&mut s, "view.set", json!({"zoom": 3, "cx": 7, "cy": 8}));
    ex(&mut s, "select.none", json!({}));
    ex(&mut s, "view.zoomSelection", json!({}));
    let v = view(&s);
    assert!(near(v.zoom, 3.0) && near(v.cx, 7.0) && near(v.cy, 8.0));
}

#[test]
fn the_zoom_tool_and_pointer_zoom_keep_the_point_under_the_pointer() {
    let mut s = Session::new();
    ex(&mut s, "view.set", json!({"zoom": 1, "cx": 0, "cy": 0, "vw": 1200, "vh": 800}));
    let at = |s: &Session, x: f64, y: f64| view(s).page_to_screen(x, y);
    // (params, zoom after, derivation)
    let rows: Vec<(Value, f64, &str)> = vec![
        (json!({"x": 100, "y": 50}), 2.0, "a click with the zoom tool zooms in a step about the clicked point"),
        (json!({"x": 100, "y": 50, "out": true}), 0.5, "Alt with the zoom tool zooms out about the clicked point"),
    ];
    for (p, zoom, why) in rows {
        ex(&mut s, "view.set", json!({"zoom": 1, "cx": 0, "cy": 0}));
        let before = at(&s, 100.0, 50.0);
        ex(&mut s, "view.zoomAt", p);
        let after = at(&s, 100.0, 50.0);
        assert!(near(view(&s).zoom, zoom), "{why}");
        assert!(near(before.0, after.0) && near(before.1, after.1), "{why}: the point moved from {before:?} to {after:?}");
    }
    // Dragging an area with the zoom tool fills the view with it.
    ex(&mut s, "view.zoomAt", json!({"x": 0, "y": 0, "width": 300, "height": 200}));
    let v = view(&s);
    assert!(near(v.zoom, (1072.0f64 / 300.0).min(672.0 / 200.0)) && near(v.cx, 150.0) && near(v.cy, 100.0), "a dragged area fills the view");
    // Cmd/Ctrl+scroll and pinching zoom about the pointer: the page point under it stays put.
    let mut v = ViewState { vw: 1200.0, vh: 800.0, ..ViewState::default() };
    let before = v.page_to_screen(300.0, -40.0);
    v.set_zoom_at(3.7, (300.0, -40.0));
    let after = v.page_to_screen(300.0, -40.0);
    assert!(near(before.0, after.0) && near(before.1, after.1), "pointer zoom keeps the point under the pointer");
    v.set_zoom_at(1e9, (0.0, 0.0));
    assert!(near(v.zoom, ViewState::MAX_ZOOM), "pointer zoom stops at the most");
    v.set_zoom_at(f64::NAN, (0.0, 0.0));
    assert!(near(v.zoom, ViewState::MAX_ZOOM), "a junk zoom is ignored");
}

#[test]
fn view_shortcuts_follow_the_documented_keys() {
    let m = |shift: bool, alt: bool, cmd: bool, ctrl: bool| Mods { shift, alt, cmd, ctrl };
    // (platform, key, modifiers, command, derivation)
    let rows: Vec<(Platform, &str, Mods, &str, &str)> = vec![
        (Platform::Mac, "=", m(true, false, false, false), "view.zoomIn", "Shift and + zooms in"),
        (Platform::Other, "-", m(true, false, false, false), "view.zoomOut", "Shift and - zooms out"),
        (Platform::Mac, "0", m(true, false, false, false), "view.zoom100", "Shift+0 zooms to 100%"),
        (Platform::Other, "1", m(true, false, false, false), "view.zoomFit", "Shift+1 zooms to fit"),
        (Platform::Mac, "2", m(true, false, false, false), "view.zoomSelection", "Shift+2 zooms to the selection"),
        (Platform::Mac, "N", m(false, false, false, false), "view.nextFrame", "N zooms to the next frame"),
        (Platform::Other, "N", m(true, false, false, false), "view.prevFrame", "Shift+N zooms to the previous frame"),
        (Platform::Mac, "R", m(true, false, false, false), "view.rulers", "Shift+R shows and hides the rulers"),
        (Platform::Mac, "'", m(false, false, true, false), "view.pixelGrid", "Cmd+' shows and hides the pixel grid"),
        (Platform::Other, "'", m(false, false, false, true), "view.pixelGrid", "Ctrl+' shows and hides the pixel grid"),
        (Platform::Mac, "'", m(true, false, true, false), "view.snapPixel", "Shift+Cmd+' turns snapping to the pixel grid on and off"),
        (Platform::Other, "'", m(true, false, false, true), "view.snapPixel", "Ctrl+Shift+' turns snapping to the pixel grid on and off"),
        (Platform::Mac, "P", m(false, false, false, true), "view.pixelPreview", "Control+P toggles pixel preview on macOS"),
        (Platform::Other, "P", m(false, true, false, true), "view.pixelPreview", "Ctrl+Alt+P toggles pixel preview elsewhere"),
        (Platform::Mac, "G", m(false, false, false, true), "view.layoutGrids", "Control+G shows and hides layout grids on macOS"),
        (Platform::Other, "4", m(true, false, false, true), "view.layoutGrids", "Ctrl+Shift+4 shows and hides layout grids elsewhere"),
        (Platform::Other, "G", m(true, false, false, false), "view.layoutGrids", "Shift+G shows and hides layout grids (the newer key)"),
        (Platform::Mac, "O", m(true, false, true, false), "view.outlines", "Shift+Cmd+O shows outlines"),
        (Platform::Other, "O", m(true, false, false, true), "view.outlines", "Ctrl+Shift+O shows outlines"),
    ];
    for (p, key, mods, want, why) in rows {
        assert_eq!(lookup(p, Context::Canvas, key, mods).map(|b| b.id), Some(want), "{why}");
    }
}

#[test]
fn panning_with_the_hand_tool_and_the_wheel() {
    let mut s = Session::new();
    ex(&mut s, "view.set", json!({"zoom": 1, "cx": 0, "cy": 0}));
    ex(&mut s, "tool.hand", json!({}));
    drag(&mut s, (0.0, 0.0), (100.0, 40.0));
    let v = view(&s);
    assert!(near(v.cx, -100.0) && near(v.cy, -40.0), "dragging with the hand tool moves the canvas with the pointer");
    // (zoom, scroll dx, dy, shift, centre moves by, derivation)
    let rows: Vec<(f64, f64, f64, bool, (f64, f64), &str)> = vec![
        (1.0, 0.0, -30.0, false, (0.0, 30.0), "scrolling pans up and down"),
        (2.0, 0.0, -30.0, false, (0.0, 15.0), "a scroll moves the canvas by screen points, whatever the zoom"),
        (1.0, 0.0, -30.0, true, (30.0, 0.0), "Shift and scroll pans left and right"),
        (1.0, -20.0, 0.0, true, (20.0, 0.0), "a sideways scroll stays sideways with Shift"),
        (1.0, -20.0, -10.0, false, (20.0, 10.0), "a trackpad scrolls both ways at once"),
        (1.0, f64::NAN, 5.0, false, (0.0, 0.0), "a junk scroll is ignored"),
    ];
    for (zoom, dx, dy, shift, (mx, my), why) in rows {
        let mut v = ViewState { zoom, ..ViewState::default() };
        v.scroll(dx, dy, shift);
        assert!(near(v.cx, mx) && near(v.cy, my), "{why}: ({}, {})", v.cx, v.cy);
    }
}

#[test]
fn pixel_grid_and_pixel_preview_show_from_their_zooms() {
    // (pixel grid on, zoom, shown, derivation)
    let grid: Vec<(bool, f64, bool, &str)> = vec![
        (true, 4.0, true, "the pixel grid shows at 400%"),
        (true, 3.99, false, "the pixel grid doesn't show below 400%"),
        (true, 64.0, true, "the pixel grid shows at any zoom above 400%"),
        (false, 64.0, false, "the pixel grid can be turned off"),
    ];
    for (on, zoom, shown, why) in grid {
        assert_eq!(ViewState { pixel_grid: on, zoom, ..ViewState::default() }.pixel_grid_shown(), shown, "{why}");
    }
    // (preview on, scale, zoom, rendering scale, derivation)
    let preview: Vec<(bool, u8, f64, Option<f64>, &str)> = vec![
        (true, 1, 2.0, Some(1.0), "pixel preview at 1x renders one pixel per page pixel"),
        (true, 2, 4.0, Some(2.0), "pixel preview at 2x renders two pixels per page pixel"),
        (true, 1, 1.0, None, "docs silent: at or below its resolution the canvas already shows pixels (pinned)"),
        (true, 2, 2.0, None, "docs silent: at or below its resolution the canvas already shows pixels (pinned)"),
        (false, 1, 8.0, None, "pixel preview can be turned off"),
    ];
    for (on, pixel_scale, zoom, k, why) in preview {
        assert_eq!(ViewState { pixel_preview: on, pixel_scale, zoom, ..ViewState::default() }.pixel_preview_scale(), k, "{why}");
    }
    let mut s = Session::new();
    assert_eq!(ex(&mut s, "view.pixelPreview", json!({"scale": 2})), json!({"on": true, "scale": 2}), "picking 2x turns pixel preview on at 2x");
}

#[test]
fn snapping_to_the_pixel_grid_places_and_moves_on_whole_pixels() {
    let mut s = Session::new();
    ex(&mut s, "view.snapObjects", json!({"on": false}));
    let r = rect(&mut s, 100.0, 100.0, 40.0, 40.0, None);
    // (snap to pixel grid, drag to, origin after, derivation)
    let rows: Vec<(bool, (f64, f64), (f64, f64), &str)> = vec![
        (true, (130.4, 120.3), (110.0, 100.0), "with snapping to the pixel grid a moved layer lands on whole pixels"),
        (false, (130.4, 120.3), (110.4, 100.3), "without it a layer moves by fractions"),
    ];
    for (on, to, at, why) in rows {
        ex(&mut s, "view.snapPixel", json!({"on": on}));
        drag(&mut s, (120.0, 120.0), to);
        let o = origin(&s, r);
        assert!(near(o.0, at.0) && near(o.1, at.1), "{why}: {o:?}");
        s.undo().unwrap();
    }
    ex(&mut s, "view.snapPixel", json!({"on": true}));
    ex(&mut s, "tool.rectangle", json!({}));
    drag(&mut s, (300.3, 300.6), (340.7, 330.2));
    let b = s.doc().unwrap().world_bounds(s.selection()[0]).unwrap();
    assert!([b.x0, b.y0, b.x1, b.y1].iter().all(|v| near(*v, v.round())), "a new shape is placed on whole pixels: {b:?}");
}

#[test]
fn rulers() {
    let mut s = Session::new();
    assert!(!view(&s).rulers, "docs silent: the rulers start hidden (pinned)");
    assert_eq!(ex(&mut s, "view.rulers", json!({})), json!({"on": true}), "View ▸ Rulers (Shift+R) shows them");
    // (zoom, labelled tick spacing in page pixels, derivation)
    let rows: Vec<(f64, f64, &str)> = vec![
        (1.0, 50.0, "docs silent: rulers count page pixels with labels at least 50 points apart (pinned)"),
        (10.0, 5.0, "docs silent: zoomed in, the labels get closer in page pixels (pinned)"),
        (100.0, 1.0, "docs silent: at most one label per page pixel (pinned)"),
        (0.1, 500.0, "docs silent: zoomed out, the labels get further apart (pinned)"),
        (0.0001, 10000.0, "docs silent: never fewer than one label per 10,000 pixels (pinned)"),
    ];
    for (zoom, step, why) in rows {
        assert!(near(guides::ruler_step(zoom), step), "{why}");
    }
}

/// A top-level frame `f` (0,0 400×300) holding the nested frame `n` (200,20 150×150) with `a`
/// (220,40 20×20); the page's guide at x = 600 and nothing else on the page.
struct GuideScene {
    f: NodeId,
    n: NodeId,
    a: NodeId,
    page: NodeId,
}

fn guide_scene(s: &mut Session) -> GuideScene {
    let f = frame(s, 0.0, 0.0, 400.0, 300.0, None);
    let n = frame(s, 200.0, 20.0, 150.0, 150.0, Some(f));
    let a = rect(s, 20.0, 20.0, 20.0, 20.0, Some(n));
    ex(s, "guide.add", json!({"vertical": true, "offset": 600}));
    let page = s.page().unwrap();
    ex(s, "select.none", json!({}));
    GuideScene { f, n, a, page }
}

fn guide_list(s: &Session, owner: NodeId) -> Vec<(bool, f64)> {
    s.doc().unwrap().node(owner).unwrap().guides().iter().map(|g| (g.vertical, g.offset)).collect()
}

#[test]
fn guides_belong_to_the_page_or_a_top_level_frame() {
    let mut s = Session::new();
    let k = guide_scene(&mut s);
    // (page point a guide is dropped at, owner, derivation)
    let doc = s.doc().unwrap().clone();
    let rows: Vec<((f64, f64), NodeId, &str)> = vec![
        ((100.0, 100.0), k.f, "a guide dragged into a frame becomes that frame's guide"),
        ((250.0, 60.0), k.f, "only top-level frames have guides: over a nested frame it goes to the frame holding it"),
        ((900.0, 900.0), k.page, "a guide dropped on the canvas is the page's"),
    ];
    for (p, owner, why) in rows {
        assert_eq!(guides::drop_owner(&doc, k.page, Point::new(p.0, p.1)), owner, "{why}");
    }
    assert!(s.execute("guide.add", &json!({"frame": k.n, "vertical": true, "offset": 5})).is_err(), "only top-level frames have guides");
    // A frame's guides are kept in its coordinates and move with it.
    ex(&mut s, "guide.add", json!({"frame": k.f, "vertical": false, "offset": 25}));
    ex(&mut s, "node.set", json!({"ids": [k.f], "props": {"x": 50}}));
    assert_eq!(guide_list(&s, k.f), vec![(false, 25.0)], "frame guides are objects of the frame: they move with it");
    // Moving, removing, undoing.
    ex(&mut s, "guide.move", json!({"index": 0, "offset": 650}));
    assert_eq!(guide_list(&s, k.page), vec![(true, 650.0)], "a guide can be dragged to a new place");
    ex(&mut s, "guide.remove", json!({"index": 0}));
    assert!(guide_list(&s, k.page).is_empty(), "a guide dragged back to its ruler is removed");
    s.undo().unwrap();
    assert_eq!(guide_list(&s, k.page), vec![(true, 650.0)], "removing a guide can be undone");
}

#[test]
fn a_frames_guides_stop_applying_once_it_is_inside_another_frame() {
    let mut s = Session::new();
    let k = guide_scene(&mut s);
    let g = frame(&mut s, 500.0, 400.0, 200.0, 200.0, None);
    let r = rect(&mut s, 10.0, 10.0, 20.0, 20.0, Some(g));
    ex(&mut s, "guide.add", json!({"frame": g, "vertical": true, "offset": 77}));
    // Snapping a box inside g whose left edge is 1 past the guide (page x 577).
    let snap = |s: &Session| {
        let doc = s.doc().unwrap();
        let x = doc.world_transform(g).as_coeffs()[4] + 77.0;
        Targets::gather(doc, k.page, &[r]).snap_rect(Rect::new(x + 1.0, 500.0, x + 9.0, 510.0), 4.0).0
    };
    assert_eq!(snap(&s), -1.0, "layers snap to their top-level frame's guides");
    ex(&mut s, "node.reparent", json!({"ids": [g], "parent": k.f}));
    assert_eq!(snap(&s), 0.0, "a frame with guides put into another frame loses them (they stop snapping)");
    assert_eq!(guide_list(&s, g), vec![(true, 77.0)], "docs silent: the guides are kept, so they come back with the frame (pinned)");
    ex(&mut s, "select.set", json!({"ids": [r]}));
    let x = s.doc().unwrap().world_transform(g).as_coeffs()[4] + 77.0;
    let y = s.doc().unwrap().world_bounds(g).unwrap().y1 + 50.0;
    assert!(measured(&mut s, (x, y), ALT).iter().all(|l| l != &format!("{}", x - s.doc().unwrap().world_bounds(r).unwrap().x1)));
}

#[test]
fn a_selected_guide_is_deleted_and_nudged_with_the_keyboard() {
    let mut s = Session::new();
    let k = guide_scene(&mut s);
    ex(&mut s, "guide.add", json!({"frame": k.f, "vertical": false, "offset": 40}));
    ex(&mut s, "select.set", json!({"ids": [k.a]}));
    ex(&mut s, "guide.select", json!({"index": 0}));
    assert!(s.selection().is_empty(), "selecting a guide deselects layers");
    // (key, shift, page guide after, derivation)
    let rows: Vec<(&str, bool, f64, &str)> = vec![
        ("ArrowRight", false, 601.0, "arrows nudge a selected guide by a pixel"),
        ("ArrowRight", true, 611.0, "Shift and an arrow nudge it by the big nudge"),
        ("ArrowLeft", false, 610.0, "the left arrow nudges a vertical guide left"),
    ];
    for (key, shift, after, why) in rows {
        assert!(s.tool_key(key, shift, false, false).unwrap(), "{why}");
        assert_eq!(guide_list(&s, k.page), vec![(true, after)], "{why}");
    }
    assert!(!s.tool_key("ArrowUp", false, false, false).unwrap(), "docs silent: arrows along a guide do nothing to it (pinned)");
    assert!(s.tool_key("Backspace", false, false, false).unwrap());
    assert!(guide_list(&s, k.page).is_empty(), "a selected guide is removed with Delete (Backspace)");
    // A frame guide, removed from the context menu (no index: the selected guide).
    ex(&mut s, "guide.select", json!({"frame": k.f, "index": 0}));
    assert!(s.tool_key("ArrowDown", false, false, false).unwrap());
    assert_eq!(guide_list(&s, k.f), vec![(false, 41.0)], "the down arrow nudges a horizontal guide down");
    ex(&mut s, "guide.remove", json!({}));
    assert!(guide_list(&s, k.f).is_empty(), "the context menu's Remove guide removes the guide right-clicked");
    assert!(s.execute("guide.remove", &json!({})).is_err(), "with no guide selected there's nothing to remove");
    // A click on the canvas or Escape deselects it; so does selecting a layer.
    ex(&mut s, "guide.add", json!({"vertical": true, "offset": 600}));
    ex(&mut s, "guide.select", json!({"index": 0}));
    click(&mut s, 900.0, 900.0);
    assert_eq!(s.ds().unwrap().selected_guide(), None, "a click on the canvas deselects the guide");
    ex(&mut s, "guide.select", json!({"index": 0}));
    assert!(s.tool_key("Escape", false, false, false).unwrap());
    assert_eq!(s.ds().unwrap().selected_guide(), None, "Escape deselects the guide");
    ex(&mut s, "guide.select", json!({"index": 0}));
    ex(&mut s, "select.set", json!({"ids": [k.a]}));
    assert_eq!(s.ds().unwrap().selected_guide(), None, "selecting a layer deselects the guide");
    assert!(!s.tool_key("Delete", false, false, false).unwrap(), "Delete then deletes layers, not the guide");
    assert!(s.execute("guide.select", &json!({"index": 7})).is_err(), "only an existing guide can be selected");
}

#[test]
fn removing_an_earlier_guide_keeps_the_selected_guide() {
    let mut s = Session::new();
    for offset in [100, 200, 300] {
        ex(&mut s, "guide.add", json!({"vertical": true, "offset": offset}));
    }
    let page = s.page().unwrap();
    ex(&mut s, "guide.select", json!({"index": 1}));
    ex(&mut s, "guide.remove", json!({"index": 0}));
    assert_eq!(s.ds().unwrap().selected_guide(), Some((page, 0)), "the guide at 200 stays selected at its new index");
    assert!(s.tool_key("ArrowRight", false, false, false).unwrap());
    assert_eq!(guide_list(&s, page), vec![(true, 201.0), (true, 300.0)], "the arrow nudges the selected guide, not its neighbour");
    // A later guide leaves the selection alone.
    ex(&mut s, "guide.remove", json!({"index": 1}));
    assert_eq!(s.ds().unwrap().selected_guide(), Some((page, 0)));
}

#[test]
fn guides_snap_and_measure() {
    let mut s = Session::new();
    let k = guide_scene(&mut s);
    ex(&mut s, "guide.add", json!({"vertical": true, "offset": 203}));
    let r = rect(&mut s, 500.0, 500.0, 40.0, 40.0, None);
    // Docs silent on snapping to guides (they're for positioning): a layer dragged near one snaps.
    drag(&mut s, (520.0, 520.0), (225.0, 520.0));
    assert!(near(origin(&s, r).0, 203.0), "docs silent: a moved layer snaps to a guide (pinned): {:?}", origin(&s, r));
    s.undo().unwrap();
    // Hidden with the rulers, a guide still snaps (docs silent: pinned).
    ex(&mut s, "view.rulers", json!({"on": false}));
    drag(&mut s, (520.0, 520.0), (225.0, 520.0));
    assert!(near(origin(&s, r).0, 203.0), "docs silent: guides hidden with the rulers still snap (pinned)");
    s.undo().unwrap();
    // A locked frame's guides can still be moved (docs silent: pinned).
    ex(&mut s, "guide.add", json!({"frame": k.f, "vertical": true, "offset": 10}));
    ex(&mut s, "object.toggleLock", json!({"ids": [k.f], "locked": true}));
    ex(&mut s, "guide.move", json!({"frame": k.f, "index": 0, "offset": 12}));
    assert_eq!(guide_list(&s, k.f), vec![(true, 12.0)], "docs silent: guides aren't locked with their frame (pinned)");
    ex(&mut s, "object.toggleLock", json!({"ids": [k.f], "locked": false}));
    // Alt with a selection measures to a guide under the pointer (from r at 500..540 to x 600).
    ex(&mut s, "guide.add", json!({"vertical": false, "offset": 700}));
    ex(&mut s, "select.set", json!({"ids": [r]}));
    assert_eq!(measured(&mut s, (600.5, 900.0), ALT), vec!["60"], "Alt measures from the selection to a guide");
    assert_eq!(measured(&mut s, (900.0, 700.5), ALT), vec!["160"], "Alt measures to horizontal guides too");
    ex(&mut s, "select.none", json!({}));
    assert!(measured(&mut s, (600.5, 900.0), ALT).is_empty(), "with nothing selected Alt measures nothing");
}

#[test]
fn alt_while_dragging_a_guide_measures_to_the_layers_in_its_frame() {
    let mut s = Session::new();
    let k = guide_scene(&mut s);
    rect(&mut s, 20.0, 100.0, 60.0, 40.0, Some(k.f));
    let doc = s.doc().unwrap().clone();
    let labels = |vertical: bool, at: f64| {
        let mut v: Vec<String> = distance::from_guide(&doc, k.f, vertical, at).into_iter().filter_map(|g| g.label).collect();
        v.sort();
        v
    };
    // (vertical, page offset, labels, derivation)
    let rows: Vec<(bool, f64, Vec<&str>, &str)> = vec![
        (true, 150.0, vec!["50", "70"], "across a frame, to the nearest edges on each side: b's right (80) and n's left (200)"),
        (true, 50.0, vec!["30", "30"], "a guide through a layer measures to its edges"),
        (false, 250.0, vec!["50", "80"], "a horizontal guide measures up to the nearest bottom edge (n's, 170) and down to the frame's (300)"),
        (true, 450.0, vec![], "a guide that misses the frame measures nothing"),
    ];
    for (v, at, want, why) in rows {
        let mut want: Vec<String> = want.into_iter().map(String::from).collect();
        want.sort();
        assert_eq!(labels(v, at), want, "{why}");
    }
}

fn columns(align: GridAlign, count: u32, size: f64, gutter: f64, offset: f64) -> LayoutGrid {
    LayoutGrid { kind: GridKind::Columns, align, count, size, gutter, offset, ..LayoutGrid::default() }
}

#[test]
fn layout_grid_columns_rows_and_grids_lay_out_as_documented() {
    use GridAlign::*;
    // (grid, frame w × h, first and last spans along the grid (columns: x, rows: y), count, derivation)
    let rows: Vec<(LayoutGrid, (f64, f64), (f64, f64), (f64, f64), usize, &str)> = vec![
        (columns(Stretch, 5, 0.0, 20.0, 0.0), (400.0, 300.0), (0.0, 64.0), (336.0, 400.0), 5, "stretched columns share the width less the gutters"),
        (columns(Stretch, 5, 0.0, 20.0, 10.0), (400.0, 300.0), (10.0, 70.0), (330.0, 390.0), 5, "a stretched grid's margin keeps it off both edges"),
        (columns(Stretch, 5, 0.0, 20.0, 0.0), (800.0, 300.0), (0.0, 144.0), (656.0, 800.0), 5, "stretched columns change size with the frame"),
        (columns(Min, 3, 60.0, 20.0, 10.0), (400.0, 300.0), (10.0, 70.0), (170.0, 230.0), 3, "left columns have a width and start at the offset"),
        (
            columns(Min, 3, 60.0, 20.0, 10.0),
            (800.0, 300.0),
            (10.0, 70.0),
            (170.0, 230.0),
            3,
            "fixed columns stay where they are when the frame is resized",
        ),
        (
            columns(Center, 3, 60.0, 20.0, 50.0),
            (400.0, 300.0),
            (90.0, 150.0),
            (250.0, 310.0),
            3,
            "centred columns sit in the middle; the offset is for left and right only",
        ),
        (columns(Max, 3, 60.0, 20.0, 10.0), (400.0, 300.0), (170.0, 230.0), (330.0, 390.0), 3, "right columns end the offset before the right edge"),
        (
            columns(Min, 0, 60.0, 20.0, 10.0),
            (400.0, 300.0),
            (10.0, 70.0),
            (330.0, 390.0),
            5,
            "Auto count fits as many fixed columns as there's room for",
        ),
        (
            columns(Center, 0, 100.0, 0.0, 0.0),
            (400.0, 300.0),
            (0.0, 100.0),
            (300.0, 400.0),
            4,
            "Auto count with centred columns uses the whole width",
        ),
        (
            columns(Max, 0, 500.0, 20.0, 0.0),
            (400.0, 300.0),
            (-100.0, 400.0),
            (-100.0, 400.0),
            1,
            "docs silent: Auto shows at least one column (pinned)",
        ),
        (
            columns(Stretch, 0, 0.0, 20.0, 0.0),
            (400.0, 300.0),
            (0.0, 400.0),
            (0.0, 400.0),
            1,
            "docs silent: a stretched grid has no Auto count, it shows one (pinned)",
        ),
        (
            LayoutGrid { kind: GridKind::Rows, align: Max, count: 2, size: 50.0, gutter: 10.0, offset: 20.0, ..LayoutGrid::default() },
            (400.0, 300.0),
            (170.0, 220.0),
            (230.0, 280.0),
            2,
            "bottom rows end the offset above the bottom edge",
        ),
        (
            LayoutGrid { kind: GridKind::Rows, align: Min, count: 2, size: 50.0, gutter: 10.0, offset: 20.0, ..LayoutGrid::default() },
            (400.0, 300.0),
            (20.0, 70.0),
            (80.0, 130.0),
            2,
            "top rows start the offset below the top edge",
        ),
    ];
    for (g, (w, h), first, last, count, why) in rows {
        let r = figcraft_layout::grid_lines(&g, w, h);
        let span = |r: &Rect| if g.kind == GridKind::Columns { (r.x0, r.x1) } else { (r.y0, r.y1) };
        assert_eq!(r.len(), count, "{why}");
        let (a, b) = (span(r.first().unwrap()), span(r.last().unwrap()));
        assert!(near(a.0, first.0) && near(a.1, first.1) && near(b.0, last.0) && near(b.1, last.1), "{why}: {a:?} … {b:?}");
    }
    // A square grid: lines every `size` both ways.
    let r = figcraft_layout::grid_lines(&LayoutGrid { size: 10.0, ..LayoutGrid::default() }, 40.0, 20.0);
    assert_eq!(r.len(), 5 + 3, "a grid of 10 draws a line every 10 pixels across and down");
    assert_eq!(LayoutGrid::default().size, 10.0, "the default grid is 10 pixels");
    assert_eq!((LayoutGrid::columns().count, LayoutGrid::columns().gutter), (5, 20.0), "docs silent: new columns are 5 with 20 gutters (pinned)");
}

/// A 400 × 300 frame at (100, 100) with `grids`, holding a 20 × 20 rectangle at (10, 200).
fn grid_frame(s: &mut Session, grids: Value) -> (NodeId, NodeId) {
    let f = frame(s, 100.0, 100.0, 400.0, 300.0, None);
    ex(s, "node.set", json!({"ids": [f], "props": {"grids": grids}}));
    let r = rect(s, 10.0, 200.0, 20.0, 20.0, Some(f));
    (f, r)
}

#[test]
fn layers_snap_to_layout_grids() {
    // Columns 5 × stretch, gutter 20: the second column starts at frame x 84 (page 184). Rows: 2 ×
    // top, 50 high from 20 with gutter 10: the second row starts at frame y 80 (page 180).
    let cols = json!({"kind": "columns", "count": 5, "gutter": 20, "align": "stretch"});
    let rows = json!({"kind": "rows", "count": 2, "size": 50, "gutter": 10, "offset": 20, "align": "min"});
    // The correction snapping a box whose left edge is at page x 185 and top at page y 181 gives.
    let snap = |s: &Session, r: NodeId| {
        let page = s.page().unwrap();
        let (dx, dy, _) = Targets::gather(s.doc().unwrap(), page, &[r]).snap_rect(Rect::new(185.0, 181.0, 195.0, 186.0), 3.0);
        (dx, dy)
    };
    // (grids, rotation, layout grids shown, expected correction, derivation)
    let table: Vec<(Value, f64, bool, (f64, f64), &str)> = vec![
        (json!([cols]), 0.0, true, (-1.0, 0.0), "docs silent on snapping: layers snap to the columns' edges (pinned)"),
        (json!([cols, rows]), 0.0, true, (-1.0, -1.0), "several layout grids on one frame combine (both snap)"),
        (
            json!([{"kind": "columns", "count": 5, "gutter": 20, "visible": false}]),
            0.0,
            true,
            (0.0, 0.0),
            "docs silent: a grid hidden with its eye doesn't snap (pinned)",
        ),
        (json!([cols]), 0.0, false, (-1.0, 0.0), "docs silent: grids hidden with Shift+G still snap (pinned)"),
        (json!([cols]), 30.0, true, (0.0, 0.0), "layout grids don't show on a rotated frame, so they don't snap there either"),
    ];
    for (grids, rotation, shown, want, why) in table {
        let mut s = Session::new();
        let (f, r) = grid_frame(&mut s, grids);
        ex(&mut s, "view.layoutGrids", json!({"on": shown}));
        if rotation != 0.0 {
            ex(&mut s, "node.set", json!({"id": f, "props": {"rotation": rotation}}));
            assert!(!figcraft_layout::grids_shown(s.doc().unwrap(), f), "{why}");
            let (gx, gy) = figcraft_layout::grid_snap_lines(s.doc().unwrap(), f);
            assert!(gx.is_empty() && gy.is_empty(), "{why}");
            continue;
        }
        assert!(figcraft_layout::grids_shown(s.doc().unwrap(), f));
        assert_eq!(snap(&s, r), want, "{why}");
    }
    // Docs silent: an auto layout frame's grids still snap its absolutely placed layers (pinned).
    let mut s = Session::new();
    let (f, r) = grid_frame(&mut s, json!([cols]));
    ex(&mut s, "layout.add", json!({"ids": [f], "direction": "vertical"}));
    ex(&mut s, "node.set", json!({"ids": [r], "props": {"absolute": true}}));
    let (gx, _) = figcraft_layout::grid_snap_lines(s.doc().unwrap(), f);
    assert_eq!(gx.len(), 10, "docs silent: auto layout frames keep their layout grids (pinned): {gx:?}");
    let t = Targets::gather(s.doc().unwrap(), s.page().unwrap(), &[r]);
    assert!(gx.iter().all(|x| t.xs.iter().any(|(tx, _, _)| tx == x)), "docs silent: they snap its absolutely placed layers (pinned)");
}

#[test]
fn layout_grids_are_saved_as_styles() {
    let mut s = Session::new();
    let (a, _) = grid_frame(&mut s, json!([{"kind": "columns", "count": 4}, {"kind": "grid", "size": 8}]));
    let b = frame(&mut s, 700.0, 0.0, 100.0, 100.0, None);
    let st = ex(&mut s, "style.create", json!({"type": "grid", "name": "Grid/8", "from": a}));
    ex(&mut s, "style.apply", json!({"ids": [b], "style": st["id"]}));
    let kinds: Vec<GridKind> = s.doc().unwrap().node(b).unwrap().grids.iter().map(|g| g.kind).collect();
    assert_eq!(kinds, vec![GridKind::Columns, GridKind::Grid], "several layout grids are saved and applied as one style");
}

#[test]
fn outline_mode_selects_layers_behind_others() {
    let mut s = Session::new();
    // `back` (0,0 100×100) with `front` over its left edge (-20,20 60×60), both filled.
    let back = rect(&mut s, 0.0, 0.0, 100.0, 100.0, None);
    let front = rect(&mut s, -20.0, 20.0, 60.0, 60.0, None);
    // (outline mode, click, selected, derivation)
    let rows: Vec<(bool, (f64, f64), Vec<NodeId>, &str)> = vec![
        (false, (0.5, 50.0), vec![front], "filled, the front layer takes the click"),
        (true, (0.5, 50.0), vec![back], "in outline mode a layer behind another can be selected by its outline"),
        (true, (-19.5, 50.0), vec![front], "in outline mode a layer is selected by its outline"),
        (true, (70.0, 60.0), vec![], "docs silent: in outline mode the inside of a shape isn't a target (pinned)"),
    ];
    for (on, p, after, why) in rows {
        ex(&mut s, "view.outlines", json!({"on": on}));
        ex(&mut s, "select.none", json!({}));
        click(&mut s, p.0, p.1);
        assert_eq!(s.selection(), after, "{why}");
    }
    // Docs silent: outlines for the selection only don't change what a click hits (pinned).
    ex(&mut s, "view.outlines", json!({"on": false}));
    ex(&mut s, "view.outlinesSelection", json!({"on": true}));
    ex(&mut s, "select.none", json!({}));
    click(&mut s, 0.5, 50.0);
    assert_eq!(s.selection(), vec![front]);
}

#[test]
fn alt_measures_between_layers() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, 0.0, 400.0, 300.0, None);
    let a = rect(&mut s, 20.0, 20.0, 40.0, 40.0, Some(f));
    let n = frame(&mut s, 200.0, 20.0, 150.0, 150.0, Some(f));
    rect(&mut s, 30.0, 30.0, 20.0, 20.0, Some(n));
    rect(&mut s, 120.0, 20.0, 40.0, 40.0, Some(f));
    // (selection, hover point, modifiers, labels, derivation)
    let rows: Vec<(Vec<NodeId>, (f64, f64), Mod, Vec<&str>, &str)> = vec![
        (vec![a], (140.0, 40.0), ALT, vec!["60"], "Alt over another layer measures the gap to it"),
        (vec![a], (100.0, 250.0), ALT, vec!["20", "20", "240", "340"], "Alt over the parent frame's empty space measures to its edges"),
        (vec![a], (240.0, 60.0), ALT, vec!["140"], "Alt over a nested frame measures to the frame as a whole"),
        (vec![a], (240.0, 60.0), CMD_ALT, vec!["170"], "Cmd/Ctrl+Alt measures to the nested layer under the pointer"),
        (vec![], (140.0, 40.0), ALT, vec![], "with nothing selected Alt measures nothing"),
        (vec![a], (140.0, 40.0), NO, vec![], "without Alt nothing is measured"),
    ];
    for (sel, p, m, want, why) in rows {
        ex(&mut s, "select.set", json!({"ids": sel}));
        let mut want: Vec<String> = want.into_iter().map(String::from).collect();
        want.sort();
        assert_eq!(measured(&mut s, p, m), want, "{why}");
    }
}
