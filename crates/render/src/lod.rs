//! Level of detail for canvas previews ([`crate::Options::lod`]): what a zoomed-out view can't
//! show in detail is drawn approximately, so a page of tens of thousands of layers renders in a
//! frame.
//!
//! - A layer whose whole subtree paints into less than [`TINY_PX`] device pixels each way is
//!   *splatted*: every visible layer in it adds its paints' colours, weighted by how much of its
//!   box they cover, to the pixels its box overlaps (a box filter). No paths, layers or effects.
//! - Text below [`GREEK_PX`] (its largest font size on screen) is *greeked*: one bar per line,
//!   across the line's glyphs, at about the ink density of text.
//!
//! - Small axis-aligned rectangles with solid fills are splatted instead of filled as paths.
//!
//! Only views at [`MAX_SCALE`] or below are approximated: 1:1 and closer previews are exact, and
//! so are exports, thumbnails and every other render. Whatever LOD can't approximate faithfully
//! (image paints, masks) takes the exact path at any scale, except that previews draw a large image
//! shown smaller than its pixels from a smaller copy ([`crate::images::shown`]).

use figcraft_color::{BlendMode, Paint, PaintKind};
use figcraft_doc::{Document, ListStyle, Node, NodeKind, StrokeAlign, TextAutoResize, TextData};
use figcraft_geom::{Affine, Rect};
use tiny_skia::Pixmap;

use crate::draw::{Ctx, ts_transform};

/// Views zoomed out further than this (device pixels per unit, at most) use LOD: closer views
/// are always exact.
pub const MAX_SCALE: f64 = 0.5;
/// Subtrees smaller than this on screen (device pixels, each way) are splatted.
pub const TINY_PX: f64 = 2.0;
/// Text whose largest font size is below this on screen (device pixels) is greeked.
pub const GREEK_PX: f64 = 4.0;
/// Share of a line's (or a text box's) area that glyph ink covers.
const TEXT_INK: f32 = 0.25;
/// Height of a greeked line's bar, relative to the font size (about the x-height and a bit).
const BAR_HEIGHT: f64 = 0.55;
/// Where a single line's bar sits in its box, as shares of the box height: from about the
/// x-height down to the baseline of an "auto" line height.
const LINE_BAR: (f64, f64) = (0.35, 0.8);
/// Deepest subtree splatted (deeper ones take the exact path).
const MAX_DEPTH: usize = 64;
/// Largest rectangle (device pixels²) whose fills are splatted rather than filled as a path.
const SMALL_AREA: f64 = 1024.0;
/// Corners rounded by less than this (device pixels) are drawn square when splatting.
const SQUARE_CORNER_PX: f64 = 1.0;

/// The largest font size a text uses.
fn max_font_size(t: &TextData) -> f64 {
    t.runs.iter().map(|r| r.style.font_size).fold(t.style.font_size, f64::max)
}

/// Whether text node `n` is greeked at `scale` device pixels per unit.
pub fn greeked(n: &Node, scale: f64) -> bool {
    matches!(&n.kind, NodeKind::Text(t) if max_font_size(t) * scale < GREEK_PX)
}

/// The paints of a text: the node's and its runs' own.
fn text_paints<'a>(n: &'a Node, t: &'a TextData) -> impl Iterator<Item = &'a Paint> {
    n.fills.iter().chain(t.runs.iter().filter_map(|r| r.style.fills.as_ref()).flatten())
}

/// Whether the subtree of `n` can be splatted: no image or pattern paints (their colours aren't known
/// without sampling them) and no masks (which change what their siblings show).
pub fn splattable(doc: &Document, n: &Node, depth: usize) -> bool {
    splattable_within(doc, n, depth, None).0
}

/// [`splattable`], kept in and read from `memo` when there is one (layers with children only).
/// Also returns how many levels the walk went down, or `None` when the nesting cap decided it: such
/// answers depend on the depth they were asked at and aren't kept, and kept answers are read only
/// where their levels still fit under the cap.
pub(crate) fn splattable_within<'a>(doc: &'a Document, n: &Node, depth: usize, memo: Option<&crate::draw::Bounds<'a>>) -> (bool, Option<usize>) {
    if depth > MAX_DEPTH {
        return (false, None);
    }
    let memo = memo.filter(|_| !n.children.is_empty());
    if let Some((ok, levels)) = memo.and_then(|m| m.splat.borrow().get(&n.id).copied())
        && depth + levels <= MAX_DEPTH
    {
        return (ok, Some(levels));
    }
    if matches!(&n.kind, NodeKind::Vector { network } if network.regions.iter().any(|r| r.fills.is_some() || r.paint_space.is_some())) {
        return (false, Some(0));
    }
    let image = |p: &Paint| p.visible && matches!(p.kind, PaintKind::Image(_) | PaintKind::Pattern(_));
    let own = match &n.kind {
        NodeKind::Text(t) => text_paints(n, t).any(image),
        _ => n.fills.iter().any(image),
    };
    let (mut ok, mut below) = (!(own || n.strokes.iter().any(image)), Some(0));
    if ok {
        for c in n.children.iter().filter_map(|c| memo.map_or_else(|| doc.node(*c), |m| m.node(doc, *c))).filter(|c| c.visible) {
            if c.is_mask {
                ok = false;
                break;
            }
            let (c_ok, c_levels) = splattable_within(doc, c, depth + 1, memo);
            below = below.zip(c_levels).map(|(b, l)| b.max(l + 1));
            if !c_ok {
                ok = false;
                break;
            }
        }
    }
    if let (Some(m), Some(l)) = (memo, below) {
        m.splat.borrow_mut().insert(n.id, (ok, l));
    }
    (ok, below)
}

/// A paint's average colour as straight RGBA (alpha times the paint's opacity); `None` for
/// paints that draw nothing or can't be averaged (images).
fn paint_color(p: &Paint) -> Option<[f32; 4]> {
    if !p.visible || p.opacity <= 0.0 {
        return None;
    }
    let op = p.opacity.clamp(0.0, 1.0);
    let c = match &p.kind {
        PaintKind::Solid { color } => [color.r, color.g, color.b, color.a],
        PaintKind::Gradient(g) => {
            let stops = g.sorted_stops();
            let n = stops.len().max(1) as f32;
            stops.iter().fold([0.0; 4], |acc, s| [acc[0] + s.color.r / n, acc[1] + s.color.g / n, acc[2] + s.color.b / n, acc[3] + s.color.a / n])
        }
        PaintKind::Image(_) | PaintKind::Pattern(_) => return None,
    };
    let c = c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    (c[3] * op > 0.0).then_some([c[0], c[1], c[2], c[3] * op])
}

/// The share of a node's box its fill covers.
fn fill_share(n: &Node) -> f32 {
    let (w, h) = (n.width.abs(), n.height.abs());
    match &n.kind {
        NodeKind::Ellipse { sweep, inner, .. } => {
            let arc = (sweep.abs() / std::f64::consts::TAU).min(1.0);
            let ring = 1.0 - inner.clamp(0.0, 1.0).powi(2);
            (std::f64::consts::FRAC_PI_4 * arc * ring) as f32
        }
        NodeKind::Polygon { .. } => 0.6,
        NodeKind::Star { .. } | NodeKind::Vector { .. } | NodeKind::BooleanOp { .. } => 0.5,
        _ => {
            // Rounded corners cut (4 − π) r² from the box.
            let r = (n.corners.radii.iter().sum::<f64>() / 4.0).clamp(0.0, w.min(h) / 2.0);
            let area = w * h;
            if area > 0.0 { (1.0 - (4.0 - std::f64::consts::PI) * r * r / area).clamp(0.0, 1.0) as f32 } else { 1.0 }
        }
    }
}

/// The local box a node's strokes paint in, and the share of it they cover.
fn stroke_box(n: &Node) -> (Rect, f32) {
    let wt = n.stroke.sides.map(|s| s.iter().copied().fold(0.0, f64::max)).unwrap_or(n.stroke.weight).max(0.0);
    let out = match n.stroke.align {
        StrokeAlign::Inside => 0.0,
        StrokeAlign::Center => wt / 2.0,
        StrokeAlign::Outside => wt,
    };
    // Open paths (lines) are stroked along their length; a line's box has no height of its own.
    let closed = figcraft_doc::geometry::is_closed(n);
    let out = if closed { out } else { wt / 2.0 };
    let r = n.local_rect().inflate(out, out);
    let (w, h) = (n.width.abs(), n.height.abs());
    let length = if closed { 2.0 * (w + h) } else { w.hypot(h) };
    let area = r.area().abs();
    let share = if area > 0.0 { (length * wt / area).clamp(0.0, 1.0) } else { 0.0 };
    (r, share as f32)
}

/// How much of a text box glyph ink covers, estimated from its characters (no layout).
fn text_share(t: &TextData, w: f64, h: f64) -> f32 {
    let area = (w * h).abs();
    let fs = max_font_size(t).max(0.0);
    let chars = t.characters.chars().filter(|c| !c.is_whitespace()).count() as f64;
    // An average glyph is about half an em wide and a line about 1.2 em tall.
    let ink = chars * 0.5 * fs * 1.2 * fs;
    if area > 0.0 && ink.is_finite() { TEXT_INK * (ink / area).clamp(0.0, 1.0) as f32 } else { 0.0 }
}

/// Fill a small rectangle (frame, component, instance…) by splatting: axis-aligned, its corners
/// rounded by less than [`SQUARE_CORNER_PX`] and only solid, normally blended fills, where a box
/// filter is what anti-aliasing computes anyway, without a path. Returns whether it did.
pub fn splat_fills(pix: &mut Pixmap, n: &Node, xf: Affine) -> bool {
    use NodeKind as K;
    if !matches!(n.kind, K::Rectangle | K::Frame | K::Component { .. } | K::ComponentSet { .. } | K::Instance { .. } | K::Section { .. })
        || !axis_aligned(xf)
    {
        return false;
    }
    let c = xf.as_coeffs();
    let radius = n.corners.radii.iter().copied().fold(0.0, f64::max);
    let dev = figcraft_geom::transform_rect_bbox(xf, n.local_rect());
    let simple =
        |p: &Paint| !p.visible || (matches!(p.kind, PaintKind::Solid { .. }) && matches!(p.blend, BlendMode::Normal | BlendMode::PassThrough));
    if !(radius * c[0].abs().max(c[3].abs()) < SQUARE_CORNER_PX && dev.area().abs() <= SMALL_AREA && n.fills.iter().all(simple)) {
        return false;
    }
    for color in n.fills.iter().filter_map(paint_color) {
        splat(pix, dev, color);
    }
    true
}

/// Splat node `n` (whose transform is already in `xf`) and its subtree onto `pix`. `clip` is the
/// device rectangle clipping frames above leave visible; `alpha` the opacity they multiply in.
pub fn splat_subtree(ctx: &Ctx, pix: &mut Pixmap, n: &Node, xf: Affine, clip: Option<Rect>, alpha: f32, depth: usize) {
    if depth > 256 || ctx.skip.contains(&n.id) || !n.visible || n.is_mask {
        return;
    }
    let alpha = alpha * n.opacity.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let local = n.local_rect();
    let paint = |pix: &mut Pixmap, r: Rect, paints: &mut dyn Iterator<Item = &Paint>, share: f32| {
        for c in paints.filter_map(paint_color) {
            splat_local(pix, r, xf, clip, [c[0], c[1], c[2], c[3] * alpha * share]);
        }
    };
    match &n.kind {
        NodeKind::Page { .. } | NodeKind::Group => {}
        NodeKind::Text(t) => paint(pix, local, &mut text_paints(n, t), text_share(t, n.width, n.height)),
        _ => {
            if figcraft_doc::geometry::is_closed(n) {
                paint(pix, local, &mut n.fills.iter(), fill_share(n));
            }
        }
    }
    if !matches!(n.kind, NodeKind::BooleanOp { .. }) && !n.hides_contents() {
        let clip = if n.clip && n.kind.is_frame_like() {
            let own = figcraft_geom::transform_rect_bbox(xf, local);
            Some(clip.map_or(own, |c| c.intersect(own)))
        } else {
            clip
        };
        for c in n.paint_order().filter_map(|c| ctx.bounds.node(ctx.doc, c)) {
            splat_subtree(ctx, pix, c, xf * c.transform, clip, alpha, depth + 1);
        }
    }
    if n.has_visible_stroke() && !matches!(n.kind, NodeKind::Text(_)) {
        let (r, share) = stroke_box(n);
        paint(pix, r, &mut n.strokes.iter(), share);
    }
}

/// Whether text is one plain line whose box is exactly its extent (auto width, one paragraph, no
/// runs, lists or indents): greeked from the box alone, without laying it out.
fn single_line(t: &TextData) -> bool {
    t.auto_resize == TextAutoResize::WidthAndHeight
        && t.runs.is_empty()
        && t.list == ListStyle::None
        && t.paragraphs.is_empty()
        && !t.hanging_punctuation
        && t.paragraph_indent == 0.0
        && !t.characters.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
}

/// Greek text node `n` (its transform in `xf`): a bar per laid-out line, across the line's
/// glyphs, kept inside `limit` (local; what the node's bounds allow it to paint).
pub fn greek_text(pix: &mut Pixmap, n: &Node, xf: Affine, limit: Rect) {
    if let NodeKind::Text(t) = &n.kind
        && single_line(t)
    {
        if !t.characters.trim().is_empty() {
            let bar = Rect::new(0.0, n.height * LINE_BAR.0, n.width, n.height * LINE_BAR.1).intersect(limit);
            for c in n.fills.iter().filter_map(paint_color) {
                fill_local(pix, bar, xf, [c[0], c[1], c[2], c[3] * TEXT_INK]);
            }
        }
        return;
    }
    let Some((layout, dy)) = figcraft_text::layout_node(n) else { return };
    for line in &layout.lines {
        let glyphs = layout.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]);
        let Some(first) = glyphs.first() else { continue };
        let size = glyphs.iter().map(|g| g.size.abs()).fold(0.0, f64::max);
        let x0 = glyphs.iter().map(|g| g.x.min(g.x + g.advance)).fold(f64::INFINITY, f64::min);
        let x1 = glyphs.iter().map(|g| g.x.max(g.x + g.advance)).fold(f64::NEG_INFINITY, f64::max);
        let bar = Rect::new(x0, line.baseline + dy - size * BAR_HEIGHT, x1, line.baseline + dy).intersect(limit);
        if !(bar.width() > 0.0 && bar.height() > 0.0) {
            continue;
        }
        let fills = layout.styles.get(first.style).and_then(|s| s.fills.as_ref()).unwrap_or(&n.fills);
        for c in fills.iter().filter_map(paint_color) {
            fill_local(pix, bar, xf, [c[0], c[1], c[2], c[3] * TEXT_INK]);
        }
    }
}

/// Whether `xf` keeps rectangles axis-aligned (no rotation or skew).
fn axis_aligned(xf: Affine) -> bool {
    let c = xf.as_coeffs();
    c[1] == 0.0 && c[2] == 0.0
}

/// Fill local rectangle `r` through `xf` with straight RGBA `color`: a box-filtered splat when
/// it stays axis-aligned, an anti-aliased path otherwise.
fn fill_local(pix: &mut Pixmap, r: Rect, xf: Affine, color: [f32; 4]) {
    if axis_aligned(xf) {
        splat(pix, figcraft_geom::transform_rect_bbox(xf, r), color);
        return;
    }
    let (Some(rect), Some(c)) = (
        tiny_skia::Rect::from_ltrb(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32),
        tiny_skia::Color::from_rgba(color[0], color[1], color[2], color[3]),
    ) else {
        return;
    };
    let paint = tiny_skia::Paint { shader: tiny_skia::Shader::SolidColor(c), anti_alias: true, ..Default::default() };
    pix.fill_rect(rect, &paint, ts_transform(xf), None);
}

/// Splat local rectangle `r` through `xf`, inside `clip`: its device bounding box, with the
/// colour thinned by how much of that box the (possibly rotated) rectangle fills.
fn splat_local(pix: &mut Pixmap, r: Rect, xf: Affine, clip: Option<Rect>, color: [f32; 4]) {
    let bbox = figcraft_geom::transform_rect_bbox(xf, r);
    let c = xf.as_coeffs();
    let area = (r.area() * (c[0] * c[3] - c[1] * c[2])).abs();
    let bbox_area = bbox.area().abs();
    // Zero-area boxes (lines) cover nothing; they're stroked through `stroke_box`.
    if bbox_area.is_nan() || bbox_area <= 0.0 {
        return;
    }
    let fill = (area / bbox_area).clamp(0.0, 1.0) as f32;
    let target = clip.map_or(bbox, |c| c.intersect(bbox));
    splat(pix, target, [color[0], color[1], color[2], color[3] * fill]);
}

/// Composite straight RGBA `color` over device rectangle `r`, each pixel weighted by how much of
/// it `r` covers (a box filter): sub-pixel layers land as their colour averaged over the pixels
/// they straddle, so they neither vanish nor jump from pixel to pixel as the view moves.
pub fn splat(pix: &mut Pixmap, r: Rect, color: [f32; 4]) {
    let a = color[3].clamp(0.0, 1.0);
    let (w, h) = (f64::from(pix.width()), f64::from(pix.height()));
    let (x0, y0, x1, y1) = (r.x0.max(0.0), r.y0.max(0.0), r.x1.min(w), r.y1.min(h));
    if !(a > 0.0 && x0.is_finite() && y0.is_finite() && x1 > x0 && y1 > y0) {
        return;
    }
    // Premultiplied source at full coverage, in 0..=255.
    let src = [color[0].clamp(0.0, 1.0) * 255.0, color[1].clamp(0.0, 1.0) * 255.0, color[2].clamp(0.0, 1.0) * 255.0, 255.0];
    let opaque = src.map(|c| (c + 0.5) as u8);
    let mix = |d: &mut [u8], cov: f32| {
        if cov >= 1.0 {
            // Fully covered by an opaque colour (the inside of most fills).
            d.copy_from_slice(&opaque);
        } else if cov > 0.0 {
            let keep = 1.0 - cov;
            // `as u8` saturates; + 0.5 rounds.
            for (d, s) in d.iter_mut().zip(src) {
                *d = (s * cov + f32::from(*d) * keep + 0.5) as u8;
            }
        }
    };
    let stride = pix.width() as usize * 4;
    let data = pix.data_mut();
    // Within the pixmap: the casts below stay in range.
    let (px0, py0, px1, py1) = (x0.floor() as usize, y0.floor() as usize, x1.ceil() as usize, y1.ceil() as usize);
    // How much of the first and last columns the rectangle covers (all of the ones between).
    let (first, last) = if px1 - px0 == 1 { ((x1 - x0) as f32, 0.0) } else { ((px0 as f64 + 1.0 - x0) as f32, (x1 - (px1 - 1) as f64) as f32) };
    for py in py0..py1 {
        let cy = (y1.min(py as f64 + 1.0) - y0.max(py as f64)) as f32 * a;
        let Some(row) = data.get_mut(py * stride + px0 * 4..py * stride + px1 * 4) else { continue };
        let mut pixels = row.chunks_exact_mut(4);
        if let Some(d) = pixels.next() {
            mix(d, first * cy);
        }
        if let Some(d) = pixels.next_back() {
            mix(d, last * cy);
        }
        for d in pixels {
            mix(d, cy);
        }
    }
}
