//! The layer tree an import makes: wrappers merged, names kept, masks and effects in the right
//! order, files on disk never read.

use figcraft_color::BlendMode;
use figcraft_doc::{Document, MaskType, Node, NodeId, StrokeAlign};

use super::import_svg_with_notes;

fn import(body: &str) -> (Document, NodeId) {
    let svg =
        format!(r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="200">{body}</svg>"##);
    let mut d = Document::new("t");
    let p = d.first_page().unwrap();
    let f = import_svg_with_notes(&mut d, svg.as_bytes(), p, 0.0, 0.0, "x").unwrap().frame;
    assert!(d.validate().is_empty(), "{:?}", d.validate());
    (d, f)
}

/// `(depth, type, name)` for every layer under the frame.
fn tree(d: &Document, f: NodeId) -> Vec<(usize, &'static str, String)> {
    fn walk(d: &Document, id: NodeId, depth: usize, out: &mut Vec<(usize, &'static str, String)>) {
        let Some(n) = d.node(id) else { return };
        out.push((depth, n.kind.type_name(), n.name.clone()));
        for c in &n.children {
            walk(d, *c, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    for c in &d.node(f).unwrap().children {
        walk(d, *c, 0, &mut out);
    }
    out
}

fn node<'a>(d: &'a Document, f: NodeId, name: &str) -> &'a Node {
    d.subtree(f).into_iter().filter_map(|i| d.node(i)).find(|n| n.name == name).unwrap()
}

/// A 4 × 2 PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAACCAIAAADwyuo0AAAAE0lEQVR4nGP4z8AARGDiPwMyBwBnsgf5Q2OaZwAAAABJRU5ErkJggg==";

fn t(depth: usize, kind: &'static str, name: &str) -> (usize, &'static str, String) {
    (depth, kind, name.to_string())
}

#[test]
fn wrappers_merge_into_their_element() {
    // A transform, opacity and blend on one element: one layer carrying them.
    let (d, f) = import(r##"<rect id="r" width="10" height="10" transform="translate(5 5)" opacity="0.5" style="mix-blend-mode:darken"/>"##);
    assert_eq!(tree(&d, f), [t(0, "RECTANGLE", "r")]);
    let r = node(&d, f, "r");
    assert_eq!((r.opacity, r.blend), (0.5, BlendMode::Darken));
    assert_eq!(d.world_transform(d.node(f).unwrap().children[0]).translation().x, 5.0);
    // A named group stays, even around one shape; an unnamed one with several children stays too.
    let (d, f) = import(r##"<g id="logo"><circle r="5"/></g><g opacity="0.5"><rect width="1" height="1"/><rect x="2" width="1" height="1"/></g>"##);
    assert_eq!(
        tree(&d, f),
        [t(0, "GROUP", "logo"), t(1, "ELLIPSE", "Ellipse"), t(0, "GROUP", "Group"), t(1, "RECTANGLE", "Rectangle"), t(1, "RECTANGLE", "Rectangle")]
    );
    // Empty groups go.
    let (d, f) = import(r##"<g id="empty"/><g><g/></g>"##);
    assert!(tree(&d, f).is_empty());
}

#[test]
fn group_opacity_and_layer_opacity_multiply_when_merged() {
    let (d, f) = import(r##"<g opacity="0.5"><rect id="r" width="10" height="10" opacity="0.5"/></g>"##);
    assert_eq!(tree(&d, f), [t(0, "RECTANGLE", "r")]);
    assert_eq!(node(&d, f, "r").opacity, 0.25);
}

#[test]
fn clip_mask_and_filter_nest_in_svg_order() {
    // Filter first, then clip, then mask: mask group › clip, (mask group › mask, (content with the filter)).
    let (d, f) = import(
        r##"<clipPath id="c"><rect width="50" height="50"/></clipPath><mask id="m"><rect width="100" height="100" fill="white"/></mask><filter id="b"><feGaussianBlur stdDeviation="2"/></filter><g id="art" clip-path="url(#c)" mask="url(#m)" filter="url(#b)" opacity="0.5"><rect width="80" height="80"/><circle cx="20" cy="20" r="10"/></g>"##,
    );
    assert_eq!(
        tree(&d, f),
        [
            t(0, "GROUP", "art"),
            t(1, "RECTANGLE", "Rectangle"),
            t(1, "GROUP", "Mask group"),
            t(2, "RECTANGLE", "Rectangle"),
            t(2, "GROUP", "Group"),
            t(3, "RECTANGLE", "Rectangle"),
            t(3, "ELLIPSE", "Ellipse"),
        ]
    );
    let art = node(&d, f, "art");
    assert_eq!(art.opacity, 0.5);
    let kids: Vec<&Node> = art.children.iter().filter_map(|c| d.node(*c)).collect();
    assert!(kids[0].is_mask && kids[0].mask_type == MaskType::Vector);
    let inner: Vec<&Node> = kids[1].children.iter().filter_map(|c| d.node(*c)).collect();
    assert!(inner[0].is_mask && inner[0].mask_type == MaskType::Luminance);
    assert_eq!(inner[1].effects.len(), 1, "the filter is on the content, under the clip and mask");
}

#[test]
fn a_clip_around_everything_is_dropped() {
    let (d, f) = import(
        r##"<clipPath id="c"><rect width="100" height="100"/></clipPath><g clip-path="url(#c)"><rect x="10" y="10" width="20" height="20"/></g>"##,
    );
    assert_eq!(tree(&d, f), [t(0, "RECTANGLE", "Rectangle")]);
    // One that cuts in stays.
    let (d, f) = import(
        r##"<clipPath id="c"><rect width="15" height="100"/></clipPath><g clip-path="url(#c)"><rect x="10" y="10" width="20" height="20"/></g>"##,
    );
    assert_eq!(tree(&d, f).len(), 3);
}

#[test]
fn an_empty_mask_hides_its_layer() {
    let (d, f) = import(r##"<mask id="m"/><rect width="10" height="10" mask="url(#m)"/><rect id="kept" width="5" height="5"/>"##);
    assert_eq!(tree(&d, f), [t(0, "RECTANGLE", "kept")]);
}

#[test]
fn a_stroke_painted_under_the_fill_shows_its_outer_half() {
    let (d, f) = import(r##"<rect id="r" width="10" height="10" fill="red" stroke="blue" stroke-width="4" paint-order="stroke"/>"##);
    let r = node(&d, f, "r");
    assert_eq!((r.stroke.align, r.stroke.weight), (StrokeAlign::Outside, 2.0));
}

#[test]
fn strokes_scale_with_their_layer() {
    let (d, f) = import(r##"<g transform="scale(3)"><line id="l" x2="10" stroke="black" stroke-width="2" stroke-dasharray="4 2"/></g>"##);
    let l = node(&d, f, "l");
    assert_eq!((l.width, l.stroke.weight, l.stroke.dashes.clone()), (30.0, 6.0, vec![12.0, 6.0]));
}

/// `stroke-dashoffset` is how far into the pattern the stroke starts. A closed outline runs the
/// pattern from its start, so the offset rotates the pattern; an open line starts half a dash in by
/// itself (see `figcraft_pathops::width::line_dashes`), so an offset of half the first dash is
/// already there and only that one is left out.
#[test]
fn a_closed_outline_keeps_a_dash_offset_of_half_a_dash() {
    let (d, f) = import(
        r##"<rect id="r" width="40" height="40" fill="none" stroke="#000" stroke-dasharray="4 2" stroke-dashoffset="2"/><path id="p" d="M0 60 L40 60 L20 100 Z" fill="none" stroke="#000" stroke-dasharray="4 2" stroke-dashoffset="2"/><circle id="c" cx="100" cy="100" r="20" fill="none" stroke="#000" stroke-dasharray="4 2" stroke-dashoffset="2"/><line id="l" y1="120" y2="120" x2="40" stroke="#000" stroke-dasharray="4 2" stroke-dashoffset="2"/>"##,
    );
    // Two in: a dash of 2 is left, then the gap, then the pattern as it is (`dash_offset`).
    let rotated = vec![2.0, 2.0, 2.0, 0.0];
    assert_eq!(node(&d, f, "r").stroke.dashes, rotated, "rectangle");
    assert_eq!(node(&d, f, "p").stroke.dashes, rotated, "closed path");
    assert_eq!(node(&d, f, "c").stroke.dashes, rotated, "circle");
    assert_eq!(node(&d, f, "l").stroke.dashes, vec![4.0, 2.0], "an open line starts half a dash in anyway");
}

/// An open path is a line in this respect: half a dash in is where it starts anyway, so that
/// offset is left out, and any other is applied; an outline with an open subpath among closed
/// ones is open.
#[test]
fn an_open_path_starts_half_a_dash_in_and_takes_other_offsets() {
    let (d, f) = import(
        r##"<path id="half" d="M0 0 L40 0 L40 40" fill="none" stroke="#000" stroke-dasharray="4 2" stroke-dashoffset="2"/><path id="one" d="M0 60 L40 60" fill="none" stroke="#000" stroke-dasharray="4 2" stroke-dashoffset="1"/><path id="mixed" d="M0 100 L40 100 L20 140 Z M60 100 L100 100" fill="none" stroke="#000" stroke-dasharray="4 2" stroke-dashoffset="2"/>"##,
    );
    assert_eq!(node(&d, f, "half").stroke.dashes, vec![4.0, 2.0], "half a dash in is where an open path starts");
    assert_eq!(node(&d, f, "one").stroke.dashes, vec![3.0, 2.0, 1.0, 0.0], "one in: three of the first dash are left");
    assert_eq!(node(&d, f, "mixed").stroke.dashes, vec![4.0, 2.0], "an open subpath makes the outline open");
}

/// A text stroke runs along glyph contours, which close, so its offset is applied as on a shape.
#[test]
fn a_text_stroke_keeps_a_dash_offset_of_half_a_dash() {
    if figcraft_text::fontdb::find_face("Inter", "Regular").is_none() {
        return;
    }
    let (d, f) = import(
        r##"<text id="t" x="10" y="30" font-family="Inter" font-size="20" fill="none" stroke="#000" stroke-dasharray="4 2" stroke-dashoffset="2">T</text>"##,
    );
    assert_eq!(node(&d, f, "t").stroke.dashes, vec![2.0, 2.0, 2.0, 0.0]);
}

#[test]
fn linked_files_are_never_read() {
    let dir = std::env::temp_dir().join("figcraft-svg-linked");
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("secret.png");
    std::fs::write(&png, base64::Engine::decode(&base64::engine::general_purpose::STANDARD, PNG).unwrap()).unwrap();
    let href = png.to_string_lossy().replace('\\', "/");
    let (d, f) = import(&format!(r##"<image width="4" height="2" href="{href}"/><image width="4" height="2" href="file:///{href}"/>"##));
    assert!(tree(&d, f).is_empty() && d.images.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_svg_image_is_imported_as_layers() {
    use base64::Engine as _;
    let inner = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect id="dot" width="10" height="10" fill="#0f0"/></svg>"##;
    let b64 = base64::engine::general_purpose::STANDARD.encode(inner);
    let (d, f) = import(&format!(r##"<image id="badge" x="20" y="30" width="40" height="40" href="data:image/svg+xml;base64,{b64}"/>"##));
    let dot = node(&d, f, "dot");
    assert_eq!((dot.kind.type_name(), dot.width), ("RECTANGLE", 40.0));
    let id = d.subtree(f).into_iter().find(|i| d.node(*i).is_some_and(|n| n.name == "dot")).unwrap();
    let o = d.world_transform(id).translation();
    assert_eq!((o.x, o.y), (20.0, 30.0));
}

#[test]
fn text_layers_are_named_by_their_text_or_id() {
    if figcraft_text::fontdb::find_face("Inter", "Regular").is_none() {
        return;
    }
    let (d, f) = import(r##"<text x="10" y="20">Hello there</text><text id="Caption_x20_1" x="10" y="60">Small</text>"##);
    assert_eq!(tree(&d, f), [t(0, "TEXT", "Hello there"), t(0, "TEXT", "Caption 1")]);
    // A chunk without its own x continues where the one before ends.
    let (d, f) = import(r##"<text x="10" y="20">Ab<tspan y="40">C</tspan>d</text>"##);
    assert_eq!(tree(&d, f), [t(0, "TEXT", "Ab"), t(0, "TEXT", "Cd")]);
    let x = |name: &str| d.world_transform(d.subtree(f).into_iter().find(|i| d.node(*i).is_some_and(|n| n.name == name)).unwrap()).translation().x;
    let advance = figcraft_text::layout(node(&d, f, "Ab").kind.text().unwrap(), None, None).width;
    assert!((x("Cd") - (10.0 + advance)).abs() < 1e-6, "{} vs {}", x("Cd"), 10.0 + advance);
    // A scaled text keeps its look in the font size.
    let (d, f) = import(r##"<text transform="scale(2)" x="10" y="20" font-size="10">Big</text>"##);
    let n = node(&d, f, "Big");
    assert_eq!(n.kind.text().map(|t| t.style.font_size), Some(20.0));
    assert_eq!(n.transform.as_coeffs()[0], 1.0);
}
