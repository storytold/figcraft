//! Icon fonts from the outside: the commands that turn layers into icons, keep them tidy and put
//! one in a text layer.

use super::*;
use crate::iconfont::test_guard;
use figcraft_geom::kurbo::{PathEl, Point};
use figcraft_geom::{BezPath, Shape};
use serde_json::{Value, json};

fn sess() -> Session {
    Session::new()
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    match s.execute(cmd, &p) {
        Ok(v) => v,
        Err(e) => panic!("{cmd} {p}: {e}"),
    }
}

fn rect(s: &mut Session, w: f64, h: f64) -> NodeId {
    let r = ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": w, "height": h}));
    params::id_of(&r["id"]).expect("a rectangle id")
}

/// The layer's name, which an icon made from it takes.
fn name_of(s: &Session, id: NodeId) -> String {
    s.doc().expect("a document").node(id).expect("the layer").name.clone()
}

/// The icons of the (only) icon font, from `icon.list`.
fn icons(s: &mut Session, font: Option<&str>) -> Vec<Value> {
    let p = match font {
        Some(f) => json!({"font": f}),
        None => json!({}),
    };
    let list = ex(s, "icon.list", p);
    list["fonts"][0]["glyphs"].as_array().cloned().unwrap_or_default()
}

fn families() -> Vec<String> {
    figcraft_text::fontdb::family_names()
}

fn registered(family: &str) -> bool {
    families().iter().any(|n| n == family)
}

fn stored_path(s: &mut Session) -> BezPath {
    let d = icons(s, None)[0]["path"].as_str().expect("a stored path").to_string();
    BezPath::from_svg(&d).expect("the stored path reads back")
}

/// [`to_em_square`](crate::cmd::icon::to_em_square) with the frame taken from the path itself: the
/// winding tests care about the contours, not about where an icon's box puts them.
fn fit(path: &BezPath, even_odd: bool) -> Option<BezPath> {
    crate::cmd::icon::to_em_square(path, even_odd, path.bounding_box())
}

/// A square, all lines, wound the way `BezPath` writes one.
fn square(x: f64, y: f64, size: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((x, y));
    p.line_to((x + size, y));
    p.line_to((x + size, y + size));
    p.line_to((x, y + size));
    p.close_path();
    p
}

/// A circle, as four cubic segments, for holes and curves that straight squares can't stand in for.
fn circle(cx: f64, cy: f64, r: f64) -> BezPath {
    let k = 0.5522847498307936 * r;
    let mut p = BezPath::new();
    p.move_to((cx + r, cy));
    p.curve_to((cx + r, cy + k), (cx + k, cy + r), (cx, cy + r));
    p.curve_to((cx - k, cy + r), (cx - r, cy + k), (cx - r, cy));
    p.curve_to((cx - r, cy - k), (cx - k, cy - r), (cx, cy - r));
    p.curve_to((cx + k, cy - r), (cx + r, cy - k), (cx + r, cy));
    p.close_path();
    p
}

fn subpaths(p: &BezPath) -> Vec<BezPath> {
    let mut out: Vec<BezPath> = Vec::new();
    for el in p.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            out.push(BezPath::new());
        }
        if let Some(last) = out.last_mut() {
            last.push(*el);
        }
    }
    out
}

/// Signed area (shoelace) of one subpath: a rectangle is all lines, so this is exact.
fn signed_area(p: &BezPath) -> f64 {
    let (mut total, mut start, mut prev) = (0.0, (0.0, 0.0), (0.0, 0.0));
    for el in p.elements() {
        match el {
            PathEl::MoveTo(p) => {
                start = (p.x, p.y);
                prev = start;
            }
            PathEl::LineTo(p) => {
                total += prev.0 * p.y - p.x * prev.1;
                prev = (p.x, p.y);
            }
            PathEl::ClosePath => {
                total += prev.0 * start.1 - start.0 * prev.1;
                prev = start;
            }
            _ => {}
        }
    }
    total / 2.0
}

#[test]
fn a_layer_becomes_an_icon_of_a_new_font() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    let layer = name_of(&s, r);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    assert_eq!(made["name"], layer.as_str(), "the icon is named after its layer");
    assert_eq!(made["codepoint"], 0xE000, "and mapped into the private use area");
    assert_eq!(made["fontName"], "Icons");
    let list = ex(&mut s, "icon.list", json!({}));
    assert_eq!(list["fonts"].as_array().map(Vec::len), Some(1));
    assert_eq!(icons(&mut s, None).len(), 1);
    assert!(!stored_path(&mut s).elements().is_empty());
    assert!(registered("Icons"), "the family is there for a text layer to pick");
}

#[test]
fn code_points_are_handed_out_in_order() {
    let _guard = test_guard();
    let mut s = sess();
    for _ in 0..3 {
        let r = rect(&mut s, 100.0, 100.0);
        ex(&mut s, "icon.create", json!({"ids": [r]}));
    }
    let cps: Vec<u64> = icons(&mut s, None).iter().filter_map(|g| g["codepoint"].as_u64()).collect();
    assert_eq!(cps, vec![0xE000, 0xE001, 0xE002]);
}

#[test]
fn a_second_icon_of_the_same_name_is_numbered() {
    let _guard = test_guard();
    let mut s = sess();
    let a = rect(&mut s, 100.0, 100.0);
    let b = rect(&mut s, 50.0, 50.0);
    let layer = name_of(&s, a);
    assert_eq!(name_of(&s, b), layer, "two layers of the same name");
    ex(&mut s, "icon.create", json!({"ids": [a]}));
    let second = ex(&mut s, "icon.create", json!({"ids": [b]}));
    assert_eq!(second["name"], format!("{layer} 2"));
}

#[test]
fn an_asked_for_code_point_is_used_and_a_clash_is_refused() {
    let _guard = test_guard();
    let mut s = sess();
    let a = rect(&mut s, 100.0, 100.0);
    let b = rect(&mut s, 50.0, 50.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [a], "codepoint": "U+F123"}));
    assert_eq!(made["codepoint"], 0xF123);
    // The same code point again is refused: one icon per code point.
    assert!(s.execute("icon.create", &json!({"ids": [b], "codepoint": "F123"})).is_err());
    // A number works too.
    let made = ex(&mut s, "icon.create", json!({"ids": [b], "codepoint": 0xF124}));
    assert_eq!(made["codepoint"], 0xF124);
}

#[test]
fn an_icon_can_be_renamed_moved_and_deleted() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    let name = made["name"].as_str().expect("a name").to_string();

    ex(&mut s, "icon.rename", json!({"name": name, "to": "Home"}));
    assert_eq!(icons(&mut s, None)[0]["name"], "Home");

    ex(&mut s, "icon.setCodepoint", json!({"name": "Home", "codepoint": "E100"}));
    assert_eq!(icons(&mut s, None)[0]["codepoint"], 0xE100);

    ex(&mut s, "icon.delete", json!({"name": "Home"}));
    assert!(icons(&mut s, None).is_empty());
}

#[test]
fn an_icon_can_be_taken_from_its_layer_again() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    let name = made["name"].as_str().expect("a name").to_string();
    assert!((stored_path(&mut s).bounding_box().width() - 1000.0).abs() < 0.5);
    // Resize the layer and take its outline again.
    ex(&mut s, "transform.resize", json!({"ids": [r], "width": 200.0, "height": 100.0}));
    ex(&mut s, "icon.update", json!({"name": name}));
    let bb = stored_path(&mut s).bounding_box();
    assert!((bb.width() - 1000.0).abs() < 0.5, "{bb:?}");
    assert!((bb.height() - 500.0).abs() < 0.5, "{bb:?}");
}

#[test]
fn the_outline_is_fitted_to_the_em_square() {
    let _guard = test_guard();
    let mut s = sess();
    // Twice as wide as it is tall: the long side fills the 800 units of the 1000-unit em.
    let r = rect(&mut s, 100.0, 50.0);
    ex(&mut s, "icon.create", json!({"ids": [r]}));
    let bb = stored_path(&mut s).bounding_box();
    assert!((bb.width() - 1000.0).abs() < 0.5, "{bb:?}");
    assert!((bb.height() - 500.0).abs() < 0.5, "{bb:?}");
    assert!((bb.center().x - 500.0).abs() < 0.5, "{bb:?}");
    assert!((bb.center().y - 500.0).abs() < 0.5, "{bb:?}");
}

#[test]
fn a_hole_drawn_even_odd_survives_the_fitting() {
    let _guard = test_guard();
    // Two squares wound the same way: only an even-odd read leaves the middle out, and a font's
    // `glyf` table always fills non-zero, so the fitting has to oppose the two contours.
    let mut ring = square(0.0, 0.0, 100.0);
    ring.extend(square(25.0, 25.0, 50.0).elements().iter().copied());

    let fitted = fit(&ring, true).expect("fitted");
    let subs = subpaths(&fitted);
    assert_eq!(subs.len(), 2, "the hole is still a contour of its own");
    assert!(signed_area(&subs[0]) * signed_area(&subs[1]) < 0.0, "and it winds against its outline");
    assert!((fitted.bounding_box().width() - 1000.0).abs() < 0.5);

    // Read as non-zero, the same two squares are solid: the hole was never a hole.
    let solid = fit(&ring, false).expect("fitted");
    assert_eq!(subpaths(&solid).len(), 1);
}

#[test]
fn a_curved_hole_drawn_even_odd_survives_the_fitting() {
    let _guard = test_guard();
    // Two circles wound the same way: only an even-odd read leaves the middle out, and a font's
    // `glyf` table always fills non-zero, so the fitting has to oppose the two contours. Curved
    // (cubic) contours, to prove `normalized` handles them and not just straight edges.
    let mut donut = circle(500.0, 500.0, 450.0);
    donut.extend(circle(500.0, 500.0, 200.0).elements().iter().copied());

    let fitted = fit(&donut, true).expect("fitted");
    let subs = subpaths(&fitted);
    assert_eq!(subs.len(), 2, "the hole is still a contour of its own");
    assert!(fitted.winding(Point::new(500.0, 850.0)) != 0, "the ring fills");
    assert_eq!(fitted.winding(Point::new(500.0, 500.0)), 0, "the hole is empty");
    assert!((fitted.bounding_box().width() - 1000.0).abs() < 1.0, "{:?}", fitted.bounding_box());

    // Read as non-zero, the same two circles are solid: the hole was never a hole.
    let solid = fit(&donut, false).expect("fitted");
    assert_eq!(subpaths(&solid).len(), 1);
}

#[test]
fn a_font_can_be_made_renamed_and_deleted() {
    let _guard = test_guard();
    let mut s = sess();
    ex(&mut s, "icon.font.create", json!({"name": "Brand"}));
    ex(&mut s, "icon.font.create", json!({}));
    let list = ex(&mut s, "icon.list", json!({}));
    let names: Vec<&str> = list["fonts"].as_array().expect("fonts").iter().filter_map(|f| f["name"].as_str()).collect();
    assert_eq!(names, vec!["Brand", "Icons"]);

    ex(&mut s, "icon.font.rename", json!({"font": "Brand", "name": "Marks"}));
    let list = ex(&mut s, "icon.list", json!({}));
    assert_eq!(list["fonts"][0]["name"], "Marks");

    ex(&mut s, "icon.font.delete", json!({"font": "Marks"}));
    let list = ex(&mut s, "icon.list", json!({}));
    assert_eq!(list["fonts"].as_array().map(Vec::len), Some(1));
}

#[test]
fn two_fonts_have_to_be_told_apart() {
    let _guard = test_guard();
    let mut s = sess();
    ex(&mut s, "icon.font.create", json!({"name": "A"}));
    ex(&mut s, "icon.font.create", json!({"name": "B"}));
    let r = rect(&mut s, 100.0, 100.0);
    // Guessing which font an icon belongs in would put it somewhere the user didn't mean.
    assert!(s.execute("icon.create", &json!({"ids": [r]})).is_err());
    ex(&mut s, "icon.create", json!({"ids": [r], "font": "B"}));
    assert_eq!(icons(&mut s, Some("B")).len(), 1);
    assert!(icons(&mut s, Some("A")).is_empty());
    // An icon name that only one font has needs no `font`.
    ex(&mut s, "icon.setCodepoint", json!({"name": "Rectangle", "codepoint": "E000"}));
    assert_eq!(icons(&mut s, Some("B"))[0]["codepoint"], 0xE000);
}

#[test]
fn selecting_several_layers_makes_one_icon_each() {
    let _guard = test_guard();
    let mut s = sess();
    let a = rect(&mut s, 100.0, 100.0);
    let b = rect(&mut s, 50.0, 50.0);
    let c = rect(&mut s, 25.0, 25.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [a, b, c]}));
    assert_eq!(made["icons"].as_array().map(Vec::len), Some(3), "three layers, three icons: {made}");
    let all = icons(&mut s, None);
    assert_eq!(all.len(), 3, "not one union of the three");
    // Each is fitted to the em square on its own, so all three fill it (a union of a 100, 50 and 25
    // square would too, but be a single contour).
    for g in &all {
        let outline = BezPath::from_svg(g["path"].as_str().expect("a path")).expect("reads back");
        assert!((outline.bounding_box().width() - 1000.0).abs() < 0.5, "{} fills the em: {:?}", g["name"], outline.bounding_box());
    }
    let cps: Vec<u64> = all.iter().filter_map(|g| g["codepoint"].as_u64()).collect();
    assert_eq!(cps, vec![0xE000, 0xE001, 0xE002], "each gets its own code point");
}

#[test]
fn an_asked_for_code_point_only_binds_the_first_of_several() {
    let _guard = test_guard();
    let mut s = sess();
    let a = rect(&mut s, 100.0, 100.0);
    let b = rect(&mut s, 50.0, 50.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [a, b], "codepoint": "U+F100"}));
    let cps: Vec<u64> = made["icons"].as_array().expect("icons").iter().filter_map(|i| i["codepoint"].as_u64()).collect();
    assert_eq!(cps, vec![0xF100, 0xE000], "the asked-for one, then the next free one");
}

#[test]
fn each_icon_keeps_the_layer_it_was_made_from() {
    let _guard = test_guard();
    let mut s = sess();
    let a = rect(&mut s, 100.0, 100.0);
    let b = rect(&mut s, 50.0, 50.0);
    ex(&mut s, "icon.create", json!({"ids": [a, b]}));
    // Both are squares, so both fill the em (800 wide, 800 tall). Make one oblong and take its
    // outline again: only that icon must change, which shows as its height halving.
    ex(&mut s, "transform.resize", json!({"ids": [a], "width": 200.0, "height": 100.0}));
    let name = icons(&mut s, None)
        .iter()
        .find(|g| g["node"].as_u64() == Some(a.0))
        .and_then(|g| g["name"].as_str())
        .expect("the icon made from `a`")
        .to_string();
    let heights = |s: &mut Session| -> Vec<(String, f64)> {
        icons(s, None).iter().map(|g| (g["name"].as_str().unwrap_or_default().to_string(), stored_bb(g).1)).collect()
    };
    let before = heights(&mut s);
    ex(&mut s, "icon.update", json!({"name": name}));
    let after = heights(&mut s);
    for ((n, b), (_, a2)) in before.iter().zip(after.iter()) {
        if *n == name {
            assert!((a2 - 500.0).abs() < 0.5, "the icon of the oblong layer is 1000x500: {a2} (was {b})");
        } else {
            assert_eq!(b, a2, "the icon of the untouched layer did not move");
        }
    }
}

/// The width of a stored glyph outline from an `icon.list` entry.
fn stored_bb(g: &Value) -> (f64, f64) {
    let p = BezPath::from_svg(g["path"].as_str().expect("a path")).expect("reads back");
    (p.bounding_box().width(), p.bounding_box().height())
}

/// The `NodeKind` of a layer, for asserting what a layer *is*.
fn kind_of(s: &Session, id: NodeId) -> String {
    format!("{:?}", s.doc().expect("a document").node(id).expect("the layer").kind)
}

#[test]
fn a_layer_that_becomes_an_icon_is_an_icon_layer_not_a_shape() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    assert!(kind_of(&s, r).starts_with("Rectangle"));
    ex(&mut s, "icon.create", json!({"ids": [r]}));
    let kind = kind_of(&s, r);
    assert!(kind.starts_with("Icon"), "the layer is an icon now, got {kind}");
    assert!(kind.contains("glyph"), "{kind}");
    // The icon is a container (it holds whatever the layer held) and draws a glyph, not a box.
    assert!(s.doc().expect("a document").node(r).expect("the layer").kind.is_frame_like());
}

#[test]
fn making_a_layer_an_icon_leaves_its_size_and_place_alone() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 200.0, 100.0);
    ex(&mut s, "transform.moveTo", json!({"ids": [r], "x": 30.0, "y": 40.0}));
    let before = s.doc().expect("a document").node(r).expect("the layer").clone();
    ex(&mut s, "icon.create", json!({"ids": [r]}));
    let after = s.doc().expect("a document").node(r).expect("the layer").clone();
    assert_eq!((after.width, after.height), (before.width, before.height), "its size is its own, not the em square's");
    assert_eq!(after.transform, before.transform, "and its place");
}

#[test]
fn making_a_layer_an_icon_keeps_its_contents_editable() {
    let _guard = test_guard();
    let mut s = sess();
    // A frame with a shape inside it: making it an icon must not consume the shape.
    let f = ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
    let fid = params::id_of(&f["id"]).expect("a frame id");
    let inner = ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40, "parent": fid}));
    let inner = params::id_of(&inner["id"]).expect("an inner id");
    ex(&mut s, "icon.create", json!({"ids": [fid]}));
    let doc = s.doc().expect("a document").clone();
    let n = doc.node(fid).expect("the icon layer");
    assert_eq!(n.children, vec![inner], "the layer keeps what it held");
    let child = doc.node(inner).expect("the inner layer");
    assert!(matches!(child.kind, figcraft_doc::NodeKind::Rectangle), "and it is still a plain editable shape: {:?}", child.kind);
}

#[test]
fn an_icons_background_comes_with_it() {
    let _guard = test_guard();
    let mut s = sess();
    // A frame that is wider than it is tall: its fill is the icon's background, so the glyph is the
    // frame's box (out of which the em fitting is measured) with what it holds drawn on top.
    let f = ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 200}));
    let fid = params::id_of(&f["id"]).expect("a frame id");
    ex(&mut s, "shape.rectangle", json!({"x": 150, "y": 50, "width": 100, "height": 100, "parent": fid}));
    ex(&mut s, "icon.create", json!({"ids": [fid]}));
    let bb = stored_path(&mut s).bounding_box();
    // The frame's 400×200 fills the em's 800 across and centres: the background is in the glyph.
    assert!((bb.width() - 1000.0).abs() < 1.0, "the background is the frame's box: {bb:?}");
    assert!((bb.height() - 500.0).abs() < 1.0, "{bb:?}");
}

/// The container's box is the window the glyph is measured through: move a shape inside it and the
/// glyph moves with it, keeping its size. Measuring the outline instead made every icon fill the em
/// square whole, so swapping one shape for another threw the whole icon across the glyph.
#[test]
fn an_icons_shapes_land_where_its_container_puts_them() {
    let _guard = test_guard();
    let at = |x: f64, y: f64| {
        let mut s = sess();
        let f = ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 400}));
        let fid = params::id_of(&f["id"]).expect("a frame id");
        ex(&mut s, "shape.rectangle", json!({"x": x, "y": y, "width": 100, "height": 100, "parent": fid}));
        // The frame is left without a fill, so the glyph is the shape alone and its place is plain.
        s.doc_mut().expect("a document").get_mut(fid).expect("the frame").fills.clear();
        ex(&mut s, "icon.create", json!({"ids": [fid]}));
        stored_path(&mut s).bounding_box()
    };
    // The frame's 400 units fill the em's 800, so a shape is drawn at twice its place in it.
    let tl = at(0.0, 0.0);
    assert!((tl.x0 - 0.0).abs() < 1.0 && (tl.y0 - 0.0).abs() < 1.0, "top left: {tl:?}");
    let br = at(300.0, 300.0);
    assert!((br.x0 - 750.0).abs() < 1.0 && (br.y0 - 750.0).abs() < 1.0, "bottom right: {br:?}");
    assert!((br.width() - tl.width()).abs() < 1.0, "it moved, it did not grow: {tl:?} {br:?}");
}

/// A layer that draws nothing — hidden, or with no fill — must not arrive in the font: an unfilled
/// path taken as ink is exactly the solid block an icon used to come out as.
#[test]
fn an_icon_leaves_out_what_the_canvas_does_not_draw() {
    let _guard = test_guard();
    let mut s = sess();
    let f = ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 400}));
    let fid = params::id_of(&f["id"]).expect("a frame id");
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100, "parent": fid}));
    let hidden = ex(&mut s, "shape.rectangle", json!({"x": 200, "y": 0, "width": 100, "height": 100, "parent": fid}));
    let hidden = params::id_of(&hidden["id"]).expect("a shape id");
    let bare = ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 200, "width": 100, "height": 100, "parent": fid}));
    let bare = params::id_of(&bare["id"]).expect("a shape id");
    {
        let doc = s.doc_mut().expect("a document");
        // The frame has no background of its own in this one.
        doc.get_mut(fid).expect("the frame").fills.clear();
        doc.get_mut(hidden).expect("the hidden layer").visible = false;
        doc.get_mut(bare).expect("the unfilled layer").fills.clear();
    }
    ex(&mut s, "icon.create", json!({"ids": [fid]}));
    let bb = stored_path(&mut s).bounding_box();
    // Only the one drawn shape is left, so the glyph is it and nothing else: 100 units of the
    // frame's 400, drawn at twice its place, sitting in the em's top-left quarter.
    assert!((bb.width() - 250.0).abs() < 1.0, "{bb:?}");
    assert!((bb.height() - 250.0).abs() < 1.0, "{bb:?}");
    assert!((bb.x0 - 0.0).abs() < 1.0 && (bb.y0 - 0.0).abs() < 1.0, "at the drawn shape's place, not stretched over the box: {bb:?}");
}

#[test]
fn a_plain_shape_with_nothing_inside_it_is_still_its_own_outline() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 400.0, 200.0);
    ex(&mut s, "icon.create", json!({"ids": [r]}));
    let bb = stored_path(&mut s).bounding_box();
    assert!((bb.width() - 1000.0).abs() < 1.0, "{bb:?}");
    assert!((bb.height() - 500.0).abs() < 1.0, "a shape keeps its own proportions: {bb:?}");
}

#[test]
fn detaching_an_icon_leaves_the_size_the_icon_had() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 200.0, 100.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    let name = made["name"].as_str().expect("a name").to_string();
    ex(&mut s, "icon.detach", json!({"ids": [r]}));
    let n = s.doc().expect("a document").node(r).expect("the layer").clone();
    assert_eq!((n.width, n.height), (200.0, 100.0), "detaching undoes creating, including the size");
    let _ = name;
}

#[test]
fn an_icon_can_still_be_edited_and_its_outline_taken_again() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    let name = made["name"].as_str().expect("a name").to_string();
    // `icon.update` re-reads the outline: it draws from the node itself now, so this still works
    // and follows the icon's own size.
    ex(&mut s, "transform.resize", json!({"ids": [r], "width": 200.0, "height": 100.0}));
    ex(&mut s, "icon.update", json!({"name": name}));
    let bb = stored_path(&mut s).bounding_box();
    // The icon's own box is the frame now (200 × 100), and it takes the whole em square: the
    // longest side becomes 1000, so the outline is 1000 × 500 and sits in the em's middle band.
    assert!((bb.width() - 1000.0).abs() < 0.5, "{bb:?}");
    assert!((bb.height() - 500.0).abs() < 0.5, "{bb:?}");
    assert!((bb.y0 - 250.0).abs() < 0.5, "centred in the em square, got {bb:?}");
}

#[test]
fn detaching_an_icon_puts_the_layer_back_and_takes_the_glyph_out() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    ex(&mut s, "icon.create", json!({"ids": [r]}));
    assert_eq!(icons(&mut s, None).len(), 1);

    let out = ex(&mut s, "icon.detach", json!({"ids": [r]}));
    assert_eq!(out["detached"].as_array().map(Vec::len), Some(1));
    assert!(kind_of(&s, r).starts_with("Rectangle"), "the layer is plain again, got {}", kind_of(&s, r));
    assert!(icons(&mut s, None).is_empty(), "and the icon left its font");
    assert!(!registered("Icons"), "so the family goes with it");
}

#[test]
fn detaching_a_plain_layer_is_refused() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    assert!(s.execute("icon.detach", &json!({"ids": [r]})).is_err());
}

#[test]
fn round_tripping_through_detach_and_create_works() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    ex(&mut s, "icon.create", json!({"ids": [r]}));
    ex(&mut s, "icon.detach", json!({"ids": [r]}));
    // It is a plain shape again, so it can become an icon once more.
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    assert_eq!(made["codepoint"], 0xE000, "the code point is free again");
    assert_eq!(icons(&mut s, None).len(), 1);
}

/// An icon can be moved to another place in its font: the order in the panel is the order in the
/// font, so a drag up or down the list is this command.
#[test]
fn an_icon_can_be_moved_to_another_place_in_its_font() {
    let _guard = test_guard();
    let mut s = sess();
    let a = rect(&mut s, 100.0, 100.0);
    let b = rect(&mut s, 100.0, 100.0);
    let c = rect(&mut s, 100.0, 100.0);
    for (i, id) in [a, b, c].into_iter().enumerate() {
        ex(&mut s, "icon.create", json!({"ids": [id], "name": format!("I{i}")}));
    }
    let order = |s: &mut Session| -> Vec<String> { icons(s, None).iter().map(|g| g["name"].as_str().unwrap_or_default().to_string()).collect() };
    assert_eq!(order(&mut s), ["I0", "I1", "I2"]);

    // I0 to the end (place 2).
    ex(&mut s, "icon.move", json!({"name": "I0", "to": 2}));
    assert_eq!(order(&mut s), ["I1", "I2", "I0"]);

    // I2 back to the front.
    ex(&mut s, "icon.move", json!({"name": "I2", "to": 0}));
    assert_eq!(order(&mut s), ["I2", "I1", "I0"]);

    // A place past the end lands on the end, and moving onto yourself changes nothing.
    ex(&mut s, "icon.move", json!({"name": "I2", "to": 99}));
    assert_eq!(order(&mut s), ["I1", "I0", "I2"]);
    ex(&mut s, "icon.move", json!({"name": "I1", "to": 0}));
    assert_eq!(order(&mut s), ["I1", "I0", "I2"]);
}

/// Making a layer an icon when the file has no icon font at all makes one, rather than refusing:
/// "Create icon" is the whole gesture, and stopping to ask for a font first would be a dead end.
#[test]
fn making_an_icon_with_no_font_makes_one() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    assert!(made["font"].as_str().is_some(), "a font was made for it: {made}");
    assert_eq!(icons(&mut s, None).len(), 1);
    assert!(kind_of(&s, r).starts_with("Icon {"), "the layer is an icon layer: {}", kind_of(&s, r));
    // And the file now has that one font, so a second icon lands in it.
    let r2 = rect(&mut s, 100.0, 100.0);
    ex(&mut s, "icon.create", json!({"ids": [r2]}));
    assert_eq!(icons(&mut s, None).len(), 2);
}

#[test]
fn a_font_exports_as_a_ttf_with_its_code_points() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    let name = made["name"].as_str().expect("a name").to_string();
    let dir = std::env::temp_dir().join("figcraft-icon-export-test");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("icons.ttf");
    let path = path.to_string_lossy().to_string();
    let out = ex(&mut s, "icon.exportFont", json!({"font": "Icons", "path": path}));

    assert_eq!(out["font"], "Icons");
    // A minimal TrueType file: the required tables for one glyph, a few hundred bytes.
    assert!(out["bytes"].as_u64().unwrap_or(0) > 200, "a real font file: {out}");
    assert_eq!(out["bytes"].as_u64(), std::fs::metadata(path.as_str()).ok().map(|m| m.len()), "the byte count is the file's");
    let map = out["icons"].as_array().expect("the code point map");
    assert_eq!(map.len(), 1);
    assert_eq!(map[0]["name"], name.as_str());
    assert_eq!(map[0]["codepoint"], 0xE000);
    // The file on disk is a TTF: `0x00010000` is its version tag.
    let head = std::fs::read(&path).expect("it was written");
    assert_eq!(&head[..4], &[0x00, 0x01, 0x00, 0x00], "a TrueType file");
    // And the font database accepts it: the same bytes the canvas gets.
    assert!(
        figcraft_text::iconfont::build_ttf(
            "Icons",
            &[(0xE000, BezPath::from_svg(icons(&mut s, None)[0]["path"].as_str().expect("a path")).expect("reads back"))]
        )
        .is_ok()
    );
}

#[test]
fn exporting_an_empty_font_is_refused() {
    let _guard = test_guard();
    let mut s = sess();
    ex(&mut s, "icon.font.create", json!({"name": "Nothing"}));
    assert!(s.execute("icon.exportFont", &json!({"font": "Nothing"})).is_err());
    assert!(s.execute("icon.exportFont", &json!({"font": "Nope"})).is_err(), "and an unknown one");
}

#[test]
fn inserting_an_icon_makes_a_text_layer_using_the_font() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    let name = made["name"].as_str().expect("a name").to_string();
    let t = ex(&mut s, "icon.insertText", json!({"name": name, "size": 32}));
    let id = params::id_of(&t["id"]).expect("a text layer");
    let doc = s.doc().expect("a document");
    let text = doc.node(id).expect("the layer").kind.text().expect("text").clone();
    assert_eq!(text.characters.chars().count(), 1);
    assert_eq!(text.characters.chars().next().map(u32::from), Some(0xE000));
    assert_eq!(text.style_at(0).font_family, "Icons");
    assert_eq!(text.style_at(0).font_size, 32.0);
    assert_eq!(s.selection(), vec![id]);
}

#[test]
fn a_generated_icon_font_renders_its_glyph_in_a_text_layer() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    let name = made["name"].as_str().expect("a name").to_string();
    let t = ex(&mut s, "icon.insertText", json!({"name": name, "size": 200}));
    let id = params::id_of(&t["id"]).expect("a text layer");
    assert!(registered("Icons"), "the font is in the database when we render");
    // Render the text layer through the real text pipeline: it must pick the icon font and draw the
    // glyph, not fall back to a system face (which would draw a missing-glyph box or nothing).
    let doc = s.doc().expect("a document").clone();
    let pix = figcraft_render::render_node(&doc, id, 2.0).expect("rendered");
    let _ = pix.save_png("D:/Projektdateien/figcraft-main/target/icon_render.png");
    let ink = pix.data().chunks(4).filter(|px| px[3] > 20).count();
    assert!(ink > 50, "the icon glyph drew something (got {ink} ink pixels of {})", pix.width() * pix.height());
}

#[test]
fn generating_a_font_compiles_and_registers_it() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    ex(&mut s, "icon.create", json!({"ids": [r]}));
    let out = ex(&mut s, "icon.generateFont", json!({"font": "Icons"}));
    assert_eq!(out["rebuilt"], 1, "it is asked for again, so it is built again");
    assert_eq!(out["fonts"][0], "Icons");
    assert!(registered("Icons"));
    // A font with nothing in it can't be compiled, and saying so beats an empty family.
    ex(&mut s, "icon.font.create", json!({"name": "Nothing"}));
    assert!(s.execute("icon.generateFont", &json!({"font": "Nothing"})).is_err());
}

#[test]
fn undo_and_redo_take_the_icon_fonts_with_them() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    assert!(s.doc().expect("a document").icon_fonts().is_empty());
    ex(&mut s, "icon.create", json!({"ids": [r]}));
    assert_eq!(s.doc().expect("a document").icon_fonts().len(), 1);
    assert!(registered("Icons"));

    assert!(s.undo().expect("undo"));
    assert!(s.doc().expect("a document").icon_fonts().is_empty());
    assert!(!registered("Icons"), "a font the document no longer carries goes");

    assert!(s.redo().expect("redo"));
    assert_eq!(s.doc().expect("a document").icon_fonts().len(), 1);
    assert!(registered("Icons"), "and comes back with it");
}

#[test]
fn icons_survive_a_round_trip_through_the_file_format() {
    let _guard = test_guard();
    let mut s = sess();
    let r = rect(&mut s, 100.0, 100.0);
    let made = ex(&mut s, "icon.create", json!({"ids": [r]}));
    let before = s.doc().expect("a document").clone();
    let text = serde_json::to_string(&before).expect("it serialises");
    let after: Document = serde_json::from_str(&text).expect("it reads back");
    assert_eq!(after.icon_fonts, before.icon_fonts);
    assert_eq!(after.icon_fonts[0].glyphs[0].codepoint, 0xE000);
    assert_eq!(after.icon_fonts[0].glyphs[0].name, made["name"]);
}
