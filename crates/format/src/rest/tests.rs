//! REST API file JSON import tests. Every fixture is synthetic, written by hand from the published
//! schema.

use super::*;
use figcraft_doc::color::{BlendMode, GradientKind, ImageScaleMode, PaintKind};
use figcraft_doc::{
    CrossAlign, EffectKind, ExportFormat, ExportSize, GridKind, HConstraint, LetterSpacing, LineHeight, ListStyle, MainAlign, Sizing, StrokeAlign,
    StrokeCap, StrokeJoin, TextAlignH, TextAlignV, TextAutoResize, TextCase, TextDecoration, VConstraint,
};
use serde_json::json;

fn file(pages: Value) -> Value {
    json!({"name": "Fixture", "schemaVersion": 0, "document": {"id": "0:0", "name": "Document", "type": "DOCUMENT", "children": pages}})
}

fn page(name: &str, children: Value) -> Value {
    json!({"id": format!("0:{name}"), "name": name, "type": "CANVAS", "backgroundColor": {"r": 0.1, "g": 0.2, "b": 0.3, "a": 1.0}, "children": children})
}

fn imp(v: &Value) -> (Document, Vec<String>) {
    import(v, &no_images).unwrap()
}

/// One page holding `children`.
fn one(children: Value) -> (Document, Vec<String>) {
    imp(&file(json!([page("Page", children)])))
}

fn named<'a>(doc: &'a Document, name: &str) -> &'a Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no node {name}"))
}

fn at(x: f64, y: f64) -> Value {
    json!([[1, 0, x], [0, 1, y]])
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn solid(r: f64, g: f64, b: f64) -> Value {
    json!({"type": "SOLID", "color": {"r": r, "g": g, "b": b, "a": 1}})
}

#[test]
fn pages_tree_and_layer_basics() {
    let v = file(json!([
        page(
            "Cover",
            json!([
                {"id": "1:1", "name": "Card", "type": "FRAME", "relativeTransform": at(10.0, 20.0), "size": {"x": 200, "y": 100},
                 "fills": [solid(1.0, 1.0, 1.0)], "clipsContent": false, "opacity": 0.5, "blendMode": "MULTIPLY", "locked": true,
                 "cornerRadius": 12, "cornerSmoothing": 0.6,
                 "children": [
                    {"id": "1:2", "name": "Hidden", "type": "RECTANGLE", "visible": false, "relativeTransform": at(5.0, 5.0), "size": {"x": 10, "y": 10},
                     "rectangleCornerRadii": [1, 2, 3, 4], "isMask": true, "fills": []},
                    {"id": "1:3", "name": "Slice", "type": "SLICE", "relativeTransform": at(0.0, 0.0), "size": {"x": 50, "y": 50},
                     "exportSettings": [{"suffix": "@2x", "format": "PNG", "constraint": {"type": "SCALE", "value": 2}},
                                        {"suffix": "", "format": "SVG", "constraint": {"type": "WIDTH", "value": 64}}]}
                 ]}
            ])
        ),
        page("Second", json!([]))
    ]));
    let (doc, warnings) = imp(&v);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(doc.name, "Fixture");
    assert_eq!(doc.pages.len(), 2, "the blank starting page is dropped");
    let p0 = doc.node(doc.pages[0]).unwrap();
    assert_eq!(p0.name, "Cover");
    assert!(matches!(p0.kind, NodeKind::Page { background, .. } if (background.g - 0.2).abs() < 1e-6));
    assert_eq!(doc.current_page, Some(doc.pages[0]));
    let card = named(&doc, "Card");
    assert!(matches!(card.kind, NodeKind::Frame));
    assert_eq!((card.width, card.height), (200.0, 100.0));
    assert!(!card.clip && card.locked);
    assert!(close(f64::from(card.opacity), 0.5));
    assert_eq!(card.blend, BlendMode::Multiply);
    assert_eq!(card.corners.radii, [12.0; 4]);
    assert!(close(card.corners.smoothing, 0.6));
    assert_eq!(doc.world_bounds(card.id).unwrap().x0, 10.0);
    let hidden = named(&doc, "Hidden");
    assert!(!hidden.visible && hidden.is_mask);
    assert_eq!(hidden.corners.radii, [1.0, 2.0, 3.0, 4.0]);
    assert!(hidden.fills.is_empty());
    assert_eq!(doc.world_bounds(hidden.id).unwrap().x0, 15.0);
    let slice = named(&doc, "Slice");
    assert!(matches!(slice.kind, NodeKind::Slice));
    assert_eq!(slice.exports.len(), 2);
    assert_eq!(slice.exports[0].suffix, "@2x");
    assert_eq!(slice.exports[0].size, ExportSize::Scale { value: 2.0 });
    assert_eq!(slice.exports[1].format, ExportFormat::Svg);
    assert_eq!(slice.exports[1].size, ExportSize::Width { value: 64.0 });
    assert!(doc.validate().is_empty());
}

#[test]
fn geometry_rotation_and_group_spaces() {
    let (s, c) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
    let (doc, _) = one(json!([
        {"id": "1:1", "name": "Rotated", "type": "RECTANGLE", "relativeTransform": [[c, s, 10], [-s, c, 20]], "size": {"x": 40, "y": 20}, "fills": []},
        // No transform: the box comes from the absolute bounds and the rotation (radians).
        {"id": "1:2", "name": "ByBounds", "type": "RECTANGLE", "rotation": std::f64::consts::FRAC_PI_2,
         "absoluteBoundingBox": {"x": 0, "y": 0, "width": 50, "height": 100}, "fills": []},
        {"id": "2:1", "name": "Frame", "type": "FRAME", "relativeTransform": at(100.0, 100.0), "size": {"x": 300, "y": 300},
         "absoluteBoundingBox": {"x": 100, "y": 100, "width": 300, "height": 300}, "children": [
            {"id": "2:2", "name": "Group", "type": "GROUP", "relativeTransform": at(50.0, 50.0), "size": {"x": 100, "y": 100},
             "absoluteBoundingBox": {"x": 150, "y": 150, "width": 100, "height": 100}, "children": [
                // Placed in the frame's space (the group doesn't define one).
                {"id": "2:3", "name": "InFrameSpace", "type": "RECTANGLE", "relativeTransform": at(60.0, 60.0), "size": {"x": 40, "y": 40},
                 "absoluteBoundingBox": {"x": 160, "y": 160, "width": 40, "height": 40}},
                // Placed in the group's space: the absolute bounds tell.
                {"id": "2:4", "name": "InGroupSpace", "type": "RECTANGLE", "relativeTransform": at(50.0, 50.0), "size": {"x": 40, "y": 40},
                 "absoluteBoundingBox": {"x": 200, "y": 200, "width": 40, "height": 40}}
             ]}
         ]}
    ]));
    let r = named(&doc, "Rotated");
    let k = r.transform.as_coeffs();
    assert!(close(k[0], c) && close(k[1], -s) && close(k[2], s) && close(k[3], c) && close(k[4], 10.0) && close(k[5], 20.0));
    let b = named(&doc, "ByBounds");
    assert!(close(b.width, 100.0) && close(b.height, 50.0));
    let wb = doc.world_bounds(b.id).unwrap();
    assert!(close(wb.x0, 0.0) && close(wb.y0, 0.0) && close(wb.x1, 50.0) && close(wb.y1, 100.0), "{wb:?}");
    let a = doc.world_bounds(named(&doc, "InFrameSpace").id).unwrap();
    assert!(close(a.x0, 160.0) && close(a.y0, 160.0), "{a:?}");
    let g = doc.world_bounds(named(&doc, "InGroupSpace").id).unwrap();
    assert!(close(g.x0, 200.0) && close(g.y0, 200.0), "{g:?}");
    // The group wraps its children.
    let gb = doc.world_bounds(named(&doc, "Group").id).unwrap();
    assert!(close(gb.x0, 160.0) && close(gb.x1, 240.0), "{gb:?}");
}

#[test]
fn paints_gradients_and_image_placeholders() {
    let rect = |name: &str, fills: Value| json!({"id": name, "name": name, "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10}, "fills": fills});
    let (doc, warnings) = one(json!([
        rect(
            "Solid",
            json!([{"type": "SOLID", "color": {"r": 1, "g": 0, "b": 0, "a": 0.5}, "opacity": 0.5, "blendMode": "SCREEN", "visible": false}])
        ),
        rect(
            "Linear",
            json!([{"type": "GRADIENT_LINEAR", "gradientHandlePositions": [{"x": 0, "y": 0.5}, {"x": 1, "y": 0.5}, {"x": 0, "y": 1}],
            "gradientStops": [{"position": 0, "color": {"r": 1, "g": 0, "b": 0, "a": 1}}, {"position": 1, "color": {"r": 0, "g": 0, "b": 1, "a": 0}}]}])
        ),
        rect(
            "Radial",
            json!([{"type": "GRADIENT_RADIAL", "gradientHandlePositions": [{"x": 0.5, "y": 0.5}, {"x": 1, "y": 0.5}, {"x": 0.5, "y": 1}],
            "gradientStops": [{"position": 0, "color": {"r": 1, "g": 1, "b": 1, "a": 1}}]}])
        ),
        rect("Angular", json!([{"type": "GRADIENT_ANGULAR", "gradientHandlePositions": [{"x": 0.5, "y": 0.5}, {"x": 1, "y": 0.5}]}])),
        rect("Diamond", json!([{"type": "GRADIENT_DIAMOND"}])),
        rect(
            "Image",
            json!([{"type": "IMAGE", "imageRef": "abc123", "scaleMode": "TILE", "scalingFactor": 0.5, "rotation": 90,
            "filters": {"exposure": 0.25, "saturation": -2}}])
        ),
        rect("Image2", json!([{"type": "IMAGE", "imageRef": "abc123", "scaleMode": "STRETCH", "imageTransform": [[0.5, 0, 0.25], [0, 0.5, 0.25]]}])),
        rect("Emoji", json!([{"type": "EMOJI"}, solid(0.0, 1.0, 0.0)]))
    ]));
    let s = &named(&doc, "Solid").fills[0];
    assert!(matches!(s.kind, PaintKind::Solid { color } if color.r == 1.0 && color.a == 1.0));
    assert!(close(f64::from(s.opacity), 0.25), "paint opacity × colour alpha");
    assert!(close(f64::from(s.color_alpha), 0.5), "the colour's part, which a bound variable replaces");
    assert!(!s.visible && s.blend == BlendMode::Screen);
    let PaintKind::Gradient(g) = &named(&doc, "Linear").fills[0].kind else { panic!() };
    assert_eq!((g.kind, g.from, g.to, g.width), (GradientKind::Linear, (0.0, 0.5), (1.0, 0.5), (0.0, 1.0)));
    assert_eq!(g.stops.len(), 2);
    assert_eq!(g.stops[1].color.a, 0.0);
    let PaintKind::Gradient(r) = &named(&doc, "Radial").fills[0].kind else { panic!() };
    assert_eq!((r.kind, r.from, r.to, r.width), (GradientKind::Radial, (0.5, 0.5), (1.0, 0.5), (0.5, 1.0)));
    let PaintKind::Gradient(a) = &named(&doc, "Angular").fills[0].kind else { panic!() };
    assert_eq!(a.kind, GradientKind::Angular);
    assert_eq!(a.width, (0.5, 1.0), "missing width handle: perpendicular");
    assert!(matches!(&named(&doc, "Diamond").fills[0].kind, PaintKind::Gradient(d) if d.kind == GradientKind::Diamond && !d.stops.is_empty()));
    let PaintKind::Image(img) = &named(&doc, "Image").fills[0].kind else { panic!() };
    assert_eq!(img.image, "abc123", "the imageRef names the placeholder");
    assert_eq!((img.scale_mode, img.scale, img.rotation), (ImageScaleMode::Tile, 0.5, 1));
    assert!(close(f64::from(img.filters.exposure), 0.25) && img.filters.saturation == -1.0);
    assert!(doc.images.get("abc123").is_some_and(|i| i.width == PLACEHOLDER_SIDE), "grey placeholder stored");
    let PaintKind::Image(crop) = &named(&doc, "Image2").fills[0].kind else { panic!() };
    assert_eq!(crop.scale_mode, ImageScaleMode::Crop);
    assert_eq!(crop.crop, Some([0.5, 0.0, 0.0, 0.5, 0.25, 0.25]));
    assert_eq!(named(&doc, "Emoji").fills.len(), 1, "unsupported paints are skipped");
    assert!(warnings.iter().any(|w| w.contains("EMOJI")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.starts_with("1 image without image data shows")), "{warnings:?}");
}

#[test]
fn image_filter_tint_turns_magenta_positive_into_native_green_positive() {
    let rect = |name: &str, tint: f64| {
        json!({
            "id": name, "name": name, "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10},
            "fills": [{"type": "IMAGE", "imageRef": "tint", "filters": {"tint": tint}}]
        })
    };
    let (doc, warnings) = one(json!([rect("Magenta", 0.75), rect("Green", -0.5), rect("Neutral", 0.0)]));
    let image = |name: &str| {
        let PaintKind::Image(image) = &named(&doc, name).fills[0].kind else { panic!("{name} is not an image fill") };
        image
    };
    // The same sign as a `.fig` import (`fig::tests::image_filters`).
    assert!(close(f64::from(image("Magenta").filters.tint), -0.75));
    assert!(close(f64::from(image("Green").filters.tint), 0.5));
    assert!(image("Neutral").filters.tint.is_sign_positive(), "no negative zero");
    assert!(!warnings.iter().any(|w| w.contains("approximate renderer")), "{warnings:?}");
}

#[test]
fn image_bytes_come_from_the_resolver() {
    let png = {
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([0, 0, 255, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let v = file(json!([page(
        "P",
        json!([{"id": "1", "name": "I", "type": "RECTANGLE", "size": {"x": 3, "y": 2}, "relativeTransform": at(0.0, 0.0),
        "fills": [{"type": "IMAGE", "imageRef": "ref1", "scaleMode": "FIT"}]}])
    )]));
    let resolver = |r: &str| (r == "ref1").then(|| png.clone());
    let (doc, warnings) = import(&v, &resolver).unwrap();
    let PaintKind::Image(img) = &named(&doc, "I").fills[0].kind else { panic!() };
    assert_eq!(img.scale_mode, ImageScaleMode::Fit);
    assert_eq!(doc.images.get(&img.image).map(|i| (i.width, i.height)), Some((3, 2)));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn strokes_and_effects() {
    let (doc, warnings) = one(json!([
        {"id": "1", "name": "S", "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10},
         "strokes": [solid(0.0, 0.0, 0.0)], "strokeWeight": 3, "strokeAlign": "OUTSIDE", "strokeCap": "ROUND", "strokeJoin": "BEVEL",
         "strokeDashes": [4, 2], "strokeMiterAngle": 60, "individualStrokeWeights": {"top": 1, "right": 2, "bottom": 3, "left": 4},
         "effects": [
            {"type": "DROP_SHADOW", "color": {"r": 0, "g": 0, "b": 0, "a": 0.5}, "offset": {"x": 2, "y": 6}, "radius": 8, "spread": 3,
             "blendMode": "NORMAL", "showShadowBehindNode": true, "visible": true},
            {"type": "INNER_SHADOW", "color": {"r": 1, "g": 0, "b": 0, "a": 1}, "offset": {"x": 0, "y": 1}, "radius": 2, "visible": false},
            {"type": "LAYER_BLUR", "radius": 5},
            {"type": "BACKGROUND_BLUR", "radius": 9},
            {"type": "SPARKLE", "radius": 1}
         ]},
        {"id": "2", "name": "L", "type": "LINE", "relativeTransform": at(0.0, 0.0), "size": {"x": 100, "y": 0}, "strokes": [solid(1.0, 0.0, 0.0)],
         "strokeCap": "TRIANGLE_FILLED", "strokeWeight": 2}
    ]));
    let s = named(&doc, "S");
    assert_eq!(s.stroke.weight, 3.0);
    assert_eq!(s.stroke.align, StrokeAlign::Outside);
    assert_eq!((s.stroke.cap_start, s.stroke.cap_end), (StrokeCap::Round, StrokeCap::Round));
    assert_eq!(s.stroke.join, StrokeJoin::Bevel);
    assert_eq!(s.stroke.dashes, vec![4.0, 2.0]);
    assert!(close(s.stroke.miter_limit, 2.0), "60° → 1/sin(30°)");
    assert_eq!(s.stroke.sides, Some([1.0, 2.0, 3.0, 4.0]));
    assert_eq!(s.effects.len(), 4);
    let EffectKind::DropShadow(d) = &s.effects[0].kind else { panic!() };
    assert_eq!((d.offset, d.radius, d.spread, d.show_behind), ((2.0, 6.0), 8.0, 3.0, true));
    assert!(close(f64::from(d.color.a), 0.5));
    assert!(matches!(&s.effects[1].kind, EffectKind::InnerShadow(i) if i.radius == 2.0) && !s.effects[1].visible);
    assert!(matches!(s.effects[2].kind, EffectKind::LayerBlur(ref b) if b.radius == 5.0));
    assert!(matches!(s.effects[3].kind, EffectKind::BackgroundBlur(ref b) if b.radius == 9.0));
    assert!(warnings.iter().any(|w| w.contains("SPARKLE")), "{warnings:?}");
    let l = named(&doc, "L");
    assert!(matches!(l.kind, NodeKind::Line));
    assert_eq!((l.width, l.height), (100.0, 0.0));
    assert_eq!(l.stroke.cap_end, StrokeCap::TriangleArrow);
    assert_eq!(l.stroke.align, StrokeAlign::Center, "lines keep their centred stroke");
}

#[test]
fn vectors_ellipses_stars_and_booleans() {
    let base = |name: &str, ty: &str| json!({"id": name, "name": name, "type": ty, "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10}});
    let mut tri = base("Tri", "VECTOR");
    tri["fills"] = json!([solid(1.0, 0.0, 0.0)]);
    tri["fillGeometry"] = json!([{"path": "M0 0L10 0L10 10Z", "windingRule": "EVENODD"}]);
    let mut wire = base("Wire", "VECTOR");
    wire["fills"] = json!([]);
    wire["strokes"] = json!([solid(0.0, 0.0, 1.0)]);
    wire["strokeWeight"] = json!(2);
    wire["strokeGeometry"] = json!([{"path": "M0 0L10 0L10 2L0 2Z", "windingRule": "NONZERO"}]);
    let mut empty = base("Empty", "VECTOR");
    empty["fillGeometry"] = json!([{"path": "M 0 0 L x y"}]);
    let mut arc = base("Arc", "ELLIPSE");
    arc["arcData"] = json!({"startingAngle": 0, "endingAngle": std::f64::consts::PI, "innerRadius": 0.5});
    let mut star = base("Star", "STAR");
    star["fillGeometry"] = json!([{"path": "M5 0L6 4L10 4L7 6L8 10L5 7L2 10L3 6L0 4L4 4Z"}]);
    let plain_star = base("PlainStar", "STAR");
    let mut counted = base("Counted", "REGULAR_POLYGON");
    counted["pointCount"] = json!(6);
    let mut boolean = base("Bool", "BOOLEAN_OPERATION");
    boolean["booleanOperation"] = json!("SUBTRACT");
    boolean["children"] =
        json!([base("A", "RECTANGLE"), {"id": "B", "name": "B", "type": "ELLIPSE", "relativeTransform": at(5.0, 5.0), "size": {"x": 10, "y": 10}}]);
    let (doc, warnings) = one(json!([tri, wire, empty, arc, star, plain_star, counted, boolean]));
    let NodeKind::Vector { network } = &named(&doc, "Tri").kind else { panic!() };
    assert_eq!(network.vertices.len(), 3);
    assert_eq!(network.regions.len(), 1);
    assert_eq!(network.regions[0].winding, WindingRule::EvenOdd);
    let w = named(&doc, "Wire");
    assert!(matches!(&w.kind, NodeKind::Vector { network } if !network.regions.is_empty()), "stroke outline becomes the shape");
    assert!(w.strokes.is_empty() && matches!(w.fills[0].kind, PaintKind::Solid { color } if color.b == 1.0));
    assert!(matches!(&named(&doc, "Empty").kind, NodeKind::Vector { network } if network.vertices.is_empty()));
    assert!(warnings.iter().any(|w| w.contains("invalid or oversized vector path")), "{warnings:?}");
    let NodeKind::Ellipse { start, sweep, inner } = named(&doc, "Arc").kind else { panic!() };
    assert!(close(start, 0.0) && close(sweep, std::f64::consts::PI) && close(inner, 0.5));
    assert!(matches!(named(&doc, "Star").kind, NodeKind::Vector { .. }), "stars with geometry keep their exact outline");
    assert!(matches!(named(&doc, "PlainStar").kind, NodeKind::Star { count: 5, .. }));
    assert!(matches!(named(&doc, "Counted").kind, NodeKind::Polygon { count: 6 }));
    let b = named(&doc, "Bool");
    assert!(matches!(b.kind, NodeKind::BooleanOp { op: BoolOp::Subtract }));
    assert_eq!(b.children.len(), 2);
    assert!(close(b.width, 15.0), "booleans wrap their children");
}

#[test]
fn text_styles_and_runs() {
    let (doc, warnings) = one(json!([
        {"id": "1", "name": "Title", "type": "TEXT", "relativeTransform": at(0.0, 0.0), "size": {"x": 200, "y": 40},
         "characters": "a\u{1F600}b c",
         "fills": [solid(0.2, 0.2, 0.2)],
         "style": {"fontFamily": "Inter", "fontPostScriptName": "Inter-Regular", "fontStyle": "Regular", "fontWeight": 400, "fontSize": 24,
                   "lineHeightPx": 29, "lineHeightUnit": "INTRINSIC_%", "letterSpacing": 0, "textAlignHorizontal": "CENTER",
                   "textAlignVertical": "CENTER", "textAutoResize": "HEIGHT", "textCase": "UPPER", "textDecoration": "UNDERLINE",
                   "paragraphSpacing": 8, "paragraphIndent": 4, "leadingTrim": "CAP_HEIGHT", "openTypeFlags": {"LIGA": 0, "ss01": 1}},
         // UTF-16 code units: the emoji takes two.
         "characterStyleOverrides": [0, 1, 1, 2],
         "styleOverrideTable": {"1": {"fontWeight": 700}, "2": {"fills": [solid(1.0, 0.0, 0.0)], "hyperlink": {"type": "URL", "url": "https://example.com"}}},
         "lineTypes": ["UNORDERED"]},
        {"id": "2", "name": "Body", "type": "TEXT", "relativeTransform": at(0.0, 50.0), "size": {"x": 100, "y": 20}, "characters": "xy",
         "style": {"fontFamily": "Roboto", "fontWeight": 600, "italic": true, "fontSize": 16, "lineHeightPx": 32, "lineHeightUnit": "PIXELS",
                   "letterSpacing": 1.5, "textTruncation": "ENDING", "maxLines": 2},
         "characterStyleOverrides": [0, 3], "styleOverrideTable": {"3": {"lineHeightPercentFontSize": 150, "lineHeightUnit": "FONT_SIZE_%", "textCase": "SMALL_CAPS_FORCED", "textDecoration": "STRIKETHROUGH"}}},
        {"id": "3", "name": "NoFills", "type": "TEXT", "relativeTransform": at(0.0, 80.0), "size": {"x": 10, "y": 10}, "characters": "z", "style": {}}
    ]));
    assert!(warnings.is_empty(), "{warnings:?}");
    let t = named(&doc, "Title");
    let NodeKind::Text(td) = &t.kind else { panic!() };
    assert_eq!(td.characters, "a\u{1F600}b c");
    assert_eq!((td.style.font_family.as_str(), td.style.font_style.as_str(), td.style.font_size), ("Inter", "Regular", 24.0));
    assert_eq!(td.style.line_height, LineHeight::Auto);
    assert_eq!(td.style.letter_spacing, LetterSpacing::Percent(0.0));
    assert_eq!((td.style.case, td.style.decoration), (TextCase::Upper, TextDecoration::Underline));
    assert_eq!((td.align_h, td.align_v, td.auto_resize), (TextAlignH::Center, TextAlignV::Middle, TextAutoResize::Height));
    assert_eq!((td.paragraph_spacing, td.paragraph_indent), (8.0, 4.0));
    assert_eq!(td.leading_trim, figcraft_doc::LeadingTrim::CapHeight);
    assert_eq!(td.features.get("liga"), Some(&false));
    assert_eq!(td.features.get("ss01"), Some(&true));
    assert_eq!(td.list, ListStyle::Bulleted);
    assert_eq!(td.runs.len(), 2, "{:?}", td.runs);
    assert_eq!((td.runs[0].start, td.runs[0].end, td.runs[0].style.font_style.as_str()), (1, 2, "Bold"));
    assert_eq!((td.runs[1].start, td.runs[1].end), (2, 3));
    assert!(td.runs[1].style.fills.as_ref().is_some_and(|f| matches!(f[0].kind, PaintKind::Solid { color } if color.r == 1.0)));
    assert_eq!(td.runs[1].style.link.as_deref(), Some("https://example.com"));
    assert!(matches!(t.fills[0].kind, PaintKind::Solid { color } if (color.r - 0.2).abs() < 1e-6));
    let NodeKind::Text(body) = &named(&doc, "Body").kind else { panic!() };
    assert_eq!(body.style.font_style, "Semi Bold Italic");
    assert_eq!(body.style.line_height, LineHeight::Pixels(32.0));
    assert_eq!(body.style.letter_spacing, LetterSpacing::Pixels(1.5));
    assert_eq!((body.auto_resize, body.max_lines), (TextAutoResize::Truncate, 2));
    assert_eq!(body.runs.len(), 1);
    assert_eq!(body.runs[0].style.line_height, LineHeight::Percent(150.0));
    assert_eq!((body.runs[0].style.case, body.runs[0].style.decoration), (TextCase::SmallCaps, TextDecoration::Strikethrough));
    assert_eq!(body.runs[0].style.font_style, "Semi Bold Italic", "overrides inherit the base");
    assert!(!named(&doc, "NoFills").fills.is_empty(), "text keeps its default fill");
}

#[test]
fn max_lines_apply_only_while_truncating() {
    let (doc, _) = one(json!([
        {"id": "1", "name": "Auto", "type": "TEXT", "relativeTransform": at(0.0, 0.0), "size": {"x": 100, "y": 20}, "characters": "a",
         "style": {"textAutoResize": "HEIGHT", "textTruncation": "DISABLED", "maxLines": 2}}
    ]));
    let NodeKind::Text(td) = &named(&doc, "Auto").kind else { panic!() };
    assert_eq!((td.auto_resize, td.max_lines), (TextAutoResize::Height, 0));
    assert!(!td.truncates());
}

#[test]
fn auto_layout_constraints_and_grids() {
    let (doc, _) = one(json!([
        {"id": "1", "name": "Stack", "type": "FRAME", "relativeTransform": at(0.0, 0.0), "size": {"x": 100, "y": 300},
         "layoutMode": "VERTICAL", "primaryAxisAlignItems": "SPACE_BETWEEN", "counterAxisAlignItems": "CENTER",
         "paddingTop": 1, "paddingRight": 2, "paddingBottom": 3, "paddingLeft": 4, "itemSpacing": 8, "counterAxisSpacing": 5,
         "layoutWrap": "WRAP", "primaryAxisSizingMode": "AUTO", "counterAxisSizingMode": "FIXED", "strokesIncludedInLayout": true,
         "itemReverseZIndex": true,
         "layoutGrids": [{"pattern": "COLUMNS", "sectionSize": 20, "visible": true, "color": {"r": 1, "g": 0, "b": 0, "a": 0.1},
                          "alignment": "CENTER", "gutterSize": 12, "offset": 3, "count": 4},
                         {"pattern": "GRID", "sectionSize": 8, "visible": false, "color": {"r": 0, "g": 0, "b": 1, "a": 0.2}}],
         "children": [
            {"id": "2", "name": "Grow", "type": "RECTANGLE", "relativeTransform": at(4.0, 1.0), "size": {"x": 94, "y": 50}, "layoutGrow": 1, "layoutAlign": "STRETCH"},
            {"id": "3", "name": "Abs", "type": "FRAME", "relativeTransform": at(10.0, 10.0), "size": {"x": 30, "y": 30}, "layoutPositioning": "ABSOLUTE",
             "layoutSizingHorizontal": "HUG", "minWidth": 10, "maxWidth": 200, "minHeight": 5, "maxHeight": null,
             "constraints": {"horizontal": "LEFT_RIGHT", "vertical": "BOTTOM"}}
         ]},
        {"id": "4", "name": "Grid", "type": "FRAME", "relativeTransform": at(200.0, 0.0), "size": {"x": 300, "y": 200},
         "layoutMode": "GRID", "gridColumnCount": 3, "gridRowCount": 2, "gridColumnGap": 4, "gridRowGap": 6,
         "_figSource": {"gridTracks": {"columnTracks": [{"sizing": "fixed", "value": 120}] }},
         "children": [{"id": "5", "name": "Cell", "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10},
                       "gridColumnAnchorIndex": 1, "gridRowAnchorIndex": 0, "gridColumnSpan": 2, "gridRowSpan": 1}]}
    ]));
    let stack = named(&doc, "Stack");
    let al = stack.layout.as_ref().unwrap();
    assert_eq!(al.mode, LayoutMode::Vertical);
    assert_eq!((al.main_align, al.cross_align), (MainAlign::SpaceBetween, CrossAlign::Center));
    assert_eq!(al.padding, [1.0, 2.0, 3.0, 4.0]);
    assert_eq!((al.gap, al.cross_gap, al.wrap, al.strokes_included, al.reverse_z), (8.0, 5.0, true, true, true));
    assert_eq!((stack.child_layout.sizing_h, stack.child_layout.sizing_v), (Sizing::Fixed, Sizing::Hug), "vertical: primary axis hugs");
    assert_eq!(stack.grids.len(), 2);
    assert_eq!((stack.grids[0].kind, stack.grids[0].count, stack.grids[0].gutter, stack.grids[0].offset), (GridKind::Columns, 4, 12.0, 3.0));
    assert_eq!(stack.grids[0].align, figcraft_doc::GridAlign::Center);
    assert!(!stack.grids[1].visible && stack.grids[1].size == 8.0);
    let grow = named(&doc, "Grow");
    assert_eq!((grow.child_layout.sizing_h, grow.child_layout.sizing_v), (Sizing::Fill, Sizing::Fill));
    let abs = named(&doc, "Abs");
    assert!(abs.child_layout.absolute);
    assert_eq!(abs.child_layout.sizing_h, Sizing::Hug);
    assert_eq!(
        (abs.child_layout.min_w, abs.child_layout.max_w, abs.child_layout.min_h, abs.child_layout.max_h),
        (Some(10.0), Some(200.0), Some(5.0), None)
    );
    assert_eq!((abs.constraints.horizontal, abs.constraints.vertical), (HConstraint::LeftRight, VConstraint::Bottom));
    let grid = named(&doc, "Grid").layout.clone().unwrap();
    assert_eq!((grid.mode, grid.grid_columns, grid.grid_rows, grid.gap, grid.cross_gap), (LayoutMode::Grid, 3, 2, 4.0, 6.0));
    assert!(grid.column_tracks.is_empty(), "private .fig track metadata is ignored by the public REST importer");
    let cell = &named(&doc, "Cell").child_layout;
    assert_eq!((cell.grid_column, cell.grid_row, cell.grid_column_span, cell.grid_row_span), (Some(1), Some(0), 2, 1));
}

fn components_file() -> Value {
    json!({
        "name": "Kit",
        "document": {"id": "0:0", "type": "DOCUMENT", "children": [page("Components", json!([
            {"id": "1:1", "name": "Button", "type": "COMPONENT_SET", "relativeTransform": at(0.0, 0.0), "size": {"x": 200, "y": 100},
             "strokes": [solid(0.6, 0.3, 1.0)], "strokeDashes": [10, 5],
             "componentPropertyDefinitions": {
                "State": {"type": "VARIANT", "defaultValue": "Default", "variantOptions": ["Default", "Hover"]},
                "Label#1:9": {"type": "TEXT", "defaultValue": "OK"},
                "Icon#2:0": {"type": "INSTANCE_SWAP", "defaultValue": "3:1", "preferredValues": []}},
             "children": [
                {"id": "1:2", "name": "State=Default", "type": "COMPONENT", "relativeTransform": at(10.0, 10.0), "size": {"x": 80, "y": 40},
                 "children": [{"id": "1:3", "name": "Label", "type": "TEXT", "relativeTransform": at(8.0, 8.0), "size": {"x": 30, "y": 20},
                               "characters": "OK", "style": {"fontSize": 14}, "componentPropertyReferences": {"characters": "Label#1:9"}}]},
                {"id": "1:4", "name": "State=Hover", "type": "COMPONENT", "relativeTransform": at(110.0, 10.0), "size": {"x": 80, "y": 40}}
             ]},
            {"id": "3:1", "name": "Icon", "type": "COMPONENT", "relativeTransform": at(300.0, 0.0), "size": {"x": 24, "y": 24},
             "componentPropertyDefinitions": {"Show#4:1": {"type": "BOOLEAN", "defaultValue": true}}},
            {"id": "5:1", "name": "Button", "type": "INSTANCE", "componentId": "1:2", "relativeTransform": at(0.0, 200.0), "size": {"x": 80, "y": 40},
             "componentProperties": {"Label#1:9": {"type": "TEXT", "value": "Go"}, "State": {"type": "VARIANT", "value": "Default"},
                                     "Icon#2:0": {"type": "INSTANCE_SWAP", "value": "3:1"}},
             "overrides": [{"id": "I5:1;1:3", "overriddenFields": ["characters", "fills", "exportSettings", "unknownField"]}],
             "children": [{"id": "I5:1;1:3", "name": "Label", "type": "TEXT", "relativeTransform": at(8.0, 8.0), "size": {"x": 30, "y": 20},
                           "characters": "Go", "style": {"fontSize": 14}, "componentPropertyReferences": {"characters": "Label#1:9"}}]},
            {"id": "6:1", "name": "Remote", "type": "INSTANCE", "componentId": "99:1", "relativeTransform": at(0.0, 300.0), "size": {"x": 10, "y": 10}}
        ]))]},
        "components": {"1:2": {"key": "k1", "name": "State=Default", "description": "Primary button", "componentSetId": "1:1"},
                       "3:1": {"key": "k3", "name": "Icon", "description": "An icon"},
                       "99:1": {"key": "k99", "name": "Remote", "description": "", "remote": true}},
        "componentSets": {"1:1": {"key": "s1", "name": "Button", "description": "Buttons"}},
        "styles": {}
    })
}

#[test]
fn components_sets_instances_and_properties() {
    let (doc, warnings) = imp(&components_file());
    let set = named(&doc, "Button");
    assert!(matches!(&set.kind, NodeKind::ComponentSet { description } if description == "Buttons"));
    assert_eq!(set.stroke.dashes, vec![10.0, 5.0]);
    let def = named(&doc, "State=Default");
    let icon = named(&doc, "Icon");
    let NodeKind::Component { description, props, .. } = &def.kind else { panic!() };
    assert_eq!(description, "Primary button");
    assert_eq!(props.get("Label"), Some(&PropDef::Text { default: "OK".into() }), "{props:?}");
    assert_eq!(props.get("Icon"), Some(&PropDef::InstanceSwap { default: icon.id, preferred: Vec::new() }));
    assert!(!props.contains_key("State"), "variant properties come from the variant names");
    let NodeKind::Component { props: hover_props, .. } = &named(&doc, "State=Hover").kind else { panic!() };
    assert!(hover_props.contains_key("Label"), "variants share the set's definitions");
    assert!(
        matches!(&icon.kind, NodeKind::Component { description, props, .. } if description == "An icon" && props.get("Show") == Some(&PropDef::Boolean { default: true }))
    );
    let label = doc.node(def.children[0]).unwrap();
    assert_eq!(label.prop_refs.get("characters").map(String::as_str), Some("Label"));
    let inst = doc.nodes().find(|n| matches!(n.kind, NodeKind::Instance { .. })).unwrap();
    let NodeKind::Instance { component, props } = &inst.kind else { panic!() };
    assert_eq!(*component, def.id);
    assert_eq!(props.get("Label"), Some(&json!("Go")));
    assert_eq!(props.get("Icon"), Some(&json!(icon.id.0)));
    assert!(!props.contains_key("State"));
    let inst_label = doc.node(inst.children[0]).unwrap();
    assert!(["characters", "fills", "exports"].iter().all(|g| inst_label.overrides.contains(*g)) && inst_label.overrides.len() == 3);
    assert!(matches!(named(&doc, "Remote").kind, NodeKind::Frame));
    assert!(warnings.iter().any(|w| w.contains("aren't in the file")), "{warnings:?}");
}

#[test]
fn component_cycles_stay_frames() {
    let comp = |id: &str, name: &str, inst_of: &str| {
        json!({"id": id, "name": name, "type": "COMPONENT", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10},
               "children": [{"id": format!("{id}i"), "name": format!("{name} inner"), "type": "INSTANCE", "componentId": inst_of,
                             "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10}}]})
    };
    let (doc, warnings) = one(json!([comp("a", "A", "b"), comp("b", "B", "a"), comp("c", "C", "c"), comp("d", "D", "a")]));
    for n in ["A inner", "B inner", "C inner"] {
        assert!(matches!(named(&doc, n).kind, NodeKind::Frame), "{n}");
    }
    // D uses A, which doesn't lead back to D: a fine instance.
    assert!(matches!(named(&doc, "D inner").kind, NodeKind::Instance { .. }));
    assert!(warnings.iter().any(|w| w.contains("inside itself")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("inside their own main")), "{warnings:?}");
}

#[test]
fn shared_styles_link_with_metadata_names() {
    let mut v = file(json!([page(
        "P",
        json!([
            {"id": "1", "name": "R", "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10},
             "fills": [solid(0.0, 0.5, 1.0)], "effects": [{"type": "LAYER_BLUR", "radius": 2}], "styles": {"fill": "S:1", "effect": "S:4"}},
            {"id": "2", "name": "T", "type": "TEXT", "relativeTransform": at(0.0, 20.0), "size": {"x": 10, "y": 10}, "characters": "Hi",
             "style": {"fontFamily": "Inter", "fontSize": 32}, "styles": {"text": "S:2"}}
        ])
    )]));
    v["styles"] = json!({"S:1": {"key": "a", "name": "Brand/Primary", "styleType": "FILL", "description": "Main"},
                         "S:2": {"key": "b", "name": "Heading", "styleType": "TEXT"},
                         "S:3": {"key": "c", "name": "Unused", "styleType": "FILL"},
                         "S:4": {"key": "d", "name": "Soft", "styleType": "EFFECT"}});
    let (doc, warnings) = imp(&v);
    assert_eq!(doc.styles.len(), 3);
    let fill = doc.style("S:1").unwrap();
    assert_eq!((fill.name.as_str(), fill.description.as_str()), ("Brand/Primary", "Main"));
    assert!(matches!(&fill.kind, StyleKind::Paint { paints } if paints.len() == 1));
    assert!(matches!(&doc.style("S:2").unwrap().kind, StyleKind::Text { style } if style.font_size == 32.0));
    assert!(matches!(&doc.style("Soft").unwrap().kind, StyleKind::Effect { effects } if effects.len() == 1));
    let r = named(&doc, "R");
    assert_eq!((r.styles.fill.as_deref(), r.styles.effect.as_deref()), (Some("S:1"), Some("S:4")));
    assert_eq!(named(&doc, "T").styles.text.as_deref(), Some("S:2"));
    assert!(warnings.iter().any(|w| w.starts_with("1 style that no layer uses was skipped")), "{warnings:?}");
}

#[test]
fn nodes_response_flows_and_detection() {
    let v = json!({"name": "Partial", "nodes": {
        "1:2": {"document": {"id": "1:2", "name": "Screen", "type": "FRAME", "relativeTransform": at(0.0, 0.0), "size": {"x": 50, "y": 40},
                             "absoluteBoundingBox": {"x": 500, "y": 300, "width": 50, "height": 40}}, "components": {}, "styles": {}},
        "9:9": null}});
    assert!(is_rest_json(&v));
    let (doc, _) = imp(&v);
    let screen = named(&doc, "Screen");
    let b = doc.world_bounds(screen.id).unwrap();
    assert_eq!((b.x0, b.y0), (500.0, 300.0), "roots are placed by their absolute bounds");
    assert_eq!(doc.node(doc.page_of(screen.id).unwrap()).unwrap().name, "Imported layers");

    let mut f = file(json!([page(
        "Flows",
        json!([{"id": "7:1", "name": "Home", "type": "FRAME", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10},
        "interactions": [{"trigger": {"type": "ON_CLICK"}, "actions": []}], "boundVariables": {"width": {"type": "VARIABLE_ALIAS", "id": "v"}}}])
    )]));
    f["document"]["children"][0]["flowStartingPoints"] = json!([{"nodeId": "7:1", "name": "Onboarding"}, {"nodeId": "nope", "name": "X"}]);
    let (doc, warnings) = imp(&f);
    let NodeKind::Page { flows, .. } = &doc.node(doc.pages[0]).unwrap().kind else { panic!() };
    assert_eq!(flows.len(), 1);
    assert_eq!((flows[0].name.as_str(), flows[0].start), ("Onboarding", named(&doc, "Home").id));
    assert!(warnings.iter().any(|w| w.contains("interactions")) && warnings.iter().any(|w| w.contains("variables bound to")), "{warnings:?}");

    // Native FigCraft JSON and other text are left to the other loaders.
    let native = serde_json::to_vec(&crate::to_json(&Document::new("x")).unwrap()).unwrap();
    assert!(try_import(&native, &no_images).is_none());
    assert!(try_import(b"PK\x03\x04", &no_images).is_none());
    assert!(try_import(b"{\"document\": {\"type\": \"FRAME\"}}", &no_images).is_none());
    assert!(matches!(try_import(b"{\"document\": {\"type\": \"DOCUMENT\"", &no_images), Some(Err(_))));
    assert!(import(&json!({"nodes": {}}), &no_images).is_err());
}

#[test]
fn unknown_types_and_missing_fields_warn() {
    let (doc, warnings) = one(json!([
        {"id": "1", "type": "STICKY", "name": "Note", "characters": "hi"},
        {"id": "2", "name": "NoType"},
        {"id": "3", "name": "Bare", "type": "RECTANGLE"},
        {"id": "4", "name": "Leafy", "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 1, "y": 1},
         "children": [{"id": "5", "name": "Orphan", "type": "RECTANGLE"}]},
        {"id": "6", "name": "Bare2", "type": "ELLIPSE"}
    ]));
    assert_eq!(doc.node_count(), 4, "page + Bare + Leafy + Bare2");
    for w in ["STICKY layers", "without a type", "without a position", "children of RECTANGLE"] {
        assert!(warnings.iter().any(|x| x.contains(w)), "{w}: {warnings:?}");
    }
    assert!(warnings.iter().any(|x| x.ends_with("(×2)")), "repeats are counted: {warnings:?}");
}

// ---- robustness ----

/// Every value of `v` (depth first), as JSON pointers.
pub(super) fn pointers(v: &Value, at: String, out: &mut Vec<String>) {
    out.push(at.clone());
    match v {
        Value::Object(m) => m.iter().for_each(|(k, c)| pointers(c, format!("{at}/{}", k.replace('~', "~0").replace('/', "~1")), out)),
        Value::Array(a) => a.iter().enumerate().for_each(|(i, c)| pointers(c, format!("{at}/{i}"), out)),
        _ => {}
    }
}

#[test]
fn junk_values_never_panic() {
    let junk = [
        json!(null),
        json!("NaN"),
        json!("Infinity"),
        json!(1e308),
        json!(-1e308),
        json!(-1),
        json!([]),
        json!({}),
        json!(true),
        json!("x"),
        json!([[1e308, 0, 0], [0, 1, 0]]),
    ];
    let base = kitchen_sink();
    let mut ptrs = Vec::new();
    pointers(&base, String::new(), &mut ptrs);
    for p in ptrs.iter().filter(|p| !p.is_empty()) {
        for j in &junk {
            let mut v = base.clone();
            if let Some(slot) = v.pointer_mut(p) {
                *slot = j.clone();
            }
            if let Ok((doc, _)) = import(&v, &no_images) {
                assert!(doc.validate().is_empty(), "{p} = {j}");
                for n in doc.nodes() {
                    assert!(n.width.is_finite() && n.height.is_finite() && n.transform.as_coeffs().iter().all(|c| c.is_finite()), "{p} = {j}");
                }
            }
        }
    }
}

/// The components fixture plus one layer of every other mapped kind.
fn kitchen_sink() -> Value {
    let mut v = components_file();
    let extra = json!([
        {"id": "k1", "name": "Styled", "type": "RECTANGLE", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10}, "styles": {"fill": "S:1"},
         "fills": [{"type": "GRADIENT_LINEAR", "gradientHandlePositions": [{"x": 0, "y": 0}, {"x": 1, "y": 1}, {"x": 0, "y": 1}],
                    "gradientStops": [{"position": 0, "color": {"r": 1, "g": 0, "b": 0, "a": 1}}]},
                   {"type": "IMAGE", "imageRef": "r", "scaleMode": "STRETCH", "imageTransform": [[1, 0, 0], [0, 1, 0]], "filters": {"tint": 0.5}}],
         "strokes": [solid(0.0, 0.0, 0.0)], "strokeWeight": 2, "individualStrokeWeights": {"top": 1, "right": 1, "bottom": 1, "left": 1},
         "strokeDashes": [1, 2], "strokeMiterAngle": 30,
         "effects": [{"type": "DROP_SHADOW", "color": {"r": 0, "g": 0, "b": 0, "a": 1}, "offset": {"x": 1, "y": 1}, "radius": 1, "spread": 1}],
         "rectangleCornerRadii": [1, 2, 3, 4], "constraints": {"horizontal": "SCALE", "vertical": "CENTER"},
         "exportSettings": [{"suffix": "", "format": "JPG", "constraint": {"type": "HEIGHT", "value": 10}}]},
        {"id": "k2", "name": "Txt", "type": "TEXT", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10}, "characters": "ab\u{1F600}c",
         "style": {"fontFamily": "Inter", "fontWeight": 500, "fontSize": 12, "lineHeightPx": 14, "lineHeightUnit": "PIXELS", "letterSpacing": 1,
                   "openTypeFlags": {"liga": 1}, "hyperlink": {"type": "URL", "url": "https://example.com"}},
         "characterStyleOverrides": [1, 1, 2, 2, 1], "styleOverrideTable": {"1": {"fontStyle": "Bold"}, "2": {"fills": [solid(1.0, 0.0, 0.0)]}},
         "lineTypes": ["ORDERED", "NONE"]},
        {"id": "k3", "name": "Vec", "type": "VECTOR", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10}, "fills": [solid(0.0, 1.0, 0.0)],
         "fillGeometry": [{"path": "M0 0C5 0 10 5 10 10L0 10Z", "windingRule": "NONZERO"}],
         "strokeGeometry": [{"path": "M0 0L1 0L1 1Z", "windingRule": "EVENODD"}]},
        {"id": "k4", "name": "Auto", "type": "FRAME", "relativeTransform": at(0.0, 0.0), "size": {"x": 100, "y": 100}, "layoutMode": "HORIZONTAL",
         "layoutWrap": "WRAP", "itemSpacing": 4, "counterAxisSpacing": 4, "paddingLeft": 2, "primaryAxisSizingMode": "AUTO",
         "layoutGrids": [{"pattern": "ROWS", "sectionSize": 10, "count": 3, "gutterSize": 2, "alignment": "MIN", "offset": 1, "visible": true,
                          "color": {"r": 0, "g": 0, "b": 1, "a": 0.1}}],
         "children": [
            {"id": "k5", "name": "G", "type": "GROUP", "relativeTransform": at(1.0, 1.0), "size": {"x": 5, "y": 5}, "layoutGrow": 1,
             "absoluteBoundingBox": {"x": 1, "y": 1, "width": 5, "height": 5},
             "children": [{"id": "k6", "name": "E", "type": "ELLIPSE", "relativeTransform": at(1.0, 1.0), "size": {"x": 5, "y": 5},
                           "absoluteBoundingBox": {"x": 1, "y": 1, "width": 5, "height": 5},
                           "arcData": {"startingAngle": 1, "endingAngle": 2, "innerRadius": 0.2}}]},
            {"id": "k7", "name": "B", "type": "BOOLEAN_OPERATION", "booleanOperation": "EXCLUDE", "relativeTransform": at(0.0, 0.0),
             "size": {"x": 5, "y": 5}, "children": [{"id": "k8", "name": "S", "type": "STAR", "relativeTransform": at(0.0, 0.0), "size": {"x": 5, "y": 5}}]}
         ]},
        {"id": "k9", "name": "Sec", "type": "SECTION", "sectionContentsHidden": true, "relativeTransform": at(0.0, 0.0), "size": {"x": 50, "y": 50},
         "children": [{"id": "k10", "name": "Use", "type": "INSTANCE", "componentId": "3:1", "relativeTransform": at(0.0, 0.0), "size": {"x": 24, "y": 24},
                       "componentProperties": {"Show#4:1": {"type": "BOOLEAN", "value": false}}}]}
    ]);
    if let (Some(kids), Some(more)) = (v.pointer_mut("/document/children/0/children").and_then(Value::as_array_mut), extra.as_array()) {
        kids.extend(more.iter().cloned());
    }
    v["styles"] = json!({"S:1": {"key": "a", "name": "Fill", "styleType": "FILL"}});
    v["document"]["children"][0]["flowStartingPoints"] = json!([{"nodeId": "5:1", "name": "Flow"}]);
    v
}

#[test]
fn kitchen_sink_imports_cleanly() {
    let (doc, warnings) = imp(&kitchen_sink());
    assert!(doc.validate().is_empty());
    assert!(matches!(named(&doc, "Sec").kind, NodeKind::Section { contents_hidden: true }));
    assert!(matches!(&named(&doc, "Use").kind, NodeKind::Instance { props, .. } if props.get("Show") == Some(&json!(false))));
    assert!(matches!(named(&doc, "Txt").kind, NodeKind::Text(ref t) if t.list == ListStyle::Numbered && t.runs.len() == 3));
    assert!(warnings.iter().all(|w| !w.contains("panick")), "{warnings:?}");
}

#[test]
fn truncated_and_mutated_bytes_never_panic() {
    let bytes = serde_json::to_vec(&components_file()).unwrap();
    for cut in (0..bytes.len()).step_by(37) {
        if let Some(Ok((doc, _))) = try_import(&bytes[..cut], &no_images) {
            assert!(doc.validate().is_empty());
        }
    }
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..400 {
        let mut b = bytes.clone();
        for _ in 0..(next() % 8 + 1) {
            let i = (next() as usize) % b.len();
            b[i] = b"0123456789-.e{}[]\":,xNnul "[(next() as usize) % 26];
        }
        let _ = try_import(&b, &no_images);
    }
}

#[test]
fn deep_nesting_and_huge_values_are_capped() {
    let mut node = json!({"id": "leaf", "name": "Leaf", "type": "RECTANGLE", "relativeTransform": at(1.0, 1.0), "size": {"x": 1, "y": 1}});
    for i in 0..400 {
        node = json!({"id": format!("g{i}"), "name": "G", "type": "FRAME", "relativeTransform": at(1.0, 1.0), "size": {"x": 1e300, "y": -5},
                      "cornerRadius": 1e300, "strokeWeight": 1e300, "itemSpacing": -1e300, "layoutMode": "HORIZONTAL", "children": [node]});
    }
    let (doc, warnings) = one(json!([node]));
    assert!(doc.node_count() <= MAX_DEPTH + 3);
    assert!(warnings.iter().any(|w| w.contains("nested deeper")), "{warnings:?}");
    for n in doc.nodes() {
        assert!(n.width <= MAX_SIZE && n.height >= 0.0 && n.corners.radii[0] <= MAX_SIZE && n.stroke.weight <= MAX_SIZE);
    }
    // serde_json's own recursion limit turns very deep text into an error, not a crash.
    let deep = format!("{{\"document\":{{\"type\":\"DOCUMENT\",\"children\":{}{}}}}}", "[".repeat(5000), "]".repeat(5000));
    assert!(matches!(try_import(deep.as_bytes(), &no_images), Some(Err(_))));
}

#[test]
fn layer_count_is_capped() {
    let kids: Vec<Value> = (0..MAX_NODES + 5).map(|i| json!({"id": i.to_string(), "type": "RECTANGLE", "name": "R"})).collect();
    let (doc, warnings) = one(Value::Array(kids));
    assert_eq!(doc.node_count(), MAX_NODES + 1);
    assert!(warnings.iter().any(|w| w.contains("more than")), "{warnings:?}");
}

#[test]
fn the_fig_import_sets_its_own_layer_cap() {
    // The `.fig` import passes a cap from the file's size: above `MAX_NODES` every layer comes in.
    let kids: Vec<Value> = (0..MAX_NODES + 5).map(|i| json!({"id": i.to_string(), "type": "RECTANGLE", "name": "R"})).collect();
    let json = file(json!([page("Page", Value::Array(kids))]));
    let (doc, warnings) = import_with_networks(&json, &no_images, Vec::new(), MAX_NODES + 10).unwrap();
    assert_eq!(doc.node_count(), MAX_NODES + 6);
    assert!(!warnings.iter().any(|w| w.contains("more than")), "{warnings:?}");
    // Below it, the import stops at the cap it was given.
    let few: Vec<Value> = (0..10).map(|i| json!({"id": i.to_string(), "type": "RECTANGLE", "name": "R"})).collect();
    let (doc, warnings) = import_with_networks(&file(json!([page("Page", Value::Array(few))])), &no_images, Vec::new(), 4).unwrap();
    assert_eq!(doc.node_count(), 5);
    assert!(warnings.iter().any(|w| w.contains("more than 4 layers")), "{warnings:?}");
}
