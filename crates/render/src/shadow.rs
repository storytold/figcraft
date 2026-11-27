//! Drop and inner shadows, with spread that offsets the shape's outline.
//!
//! Spread moves a shadow's outline out (or in) by the spread: the shape is filled and its outline
//! stroked twice the spread wide with mitred joins, drawn over it to grow and cut away to shrink.
//! Ellipses stay ellipses, stars keep their points, text keeps its letterforms, and a rectangle's
//! corner radii grow with the spread. Layers whose pixels aren't one outline (groups, frames that
//! don't clip, ...) spread their pixels instead (a square neighbourhood).
//!
//! The canvas, PNG/JPG/WebP exports and the PDF export's shadow images all come from here.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use figcraft_color::{Paint, PaintKind};
use figcraft_doc::{Document, EffectKind, Node, NodeKind, SPREAD_MITER_LIMIT, Shadow};
use figcraft_geom::{Affine, BezPath};
use tiny_skia::{FillRule, Pixmap, PixmapPaint, Transform};

use crate::draw::{fill_to_stroke_middle, hash_path, node_outline, stroke_area, stroke_rule, ts_path, ts_transform};
use crate::fx;

/// The largest spread drawn, in device pixels.
const MAX_SPREAD_PX: f64 = 1e6;

/// A shape in local space: an outline and the rule it fills with.
pub type Outline = (BezPath, FillRule);

/// The outlines a node's shadows are cast by when they spread, in local space: its fill outline
/// (when a fill shows) and its stroke area. `None` when the layer's pixels aren't those outlines:
/// groups, frames that don't clip their children or show no fill, layers filled only with images
/// or patterns (whose transparent pixels cast no shadow).
pub fn silhouette(doc: &Document, n: &Node) -> Option<Vec<Outline>> {
    outlines(doc, n, |f| f.visible && f.opacity > 0.0)
}

/// The area a node's paints cover, in local space: the outline of its visible fills (translucent
/// or at 0% too) and its stroke area; what hides its drop shadows unless they show behind
/// transparent areas. `None` where [`silhouette`] is.
pub fn cover(doc: &Document, n: &Node) -> Option<Vec<Outline>> {
    outlines(doc, n, |f| f.visible)
}

/// Whether the fills `counts` picks (one at least) all paint their own alpha within `n`'s outline,
/// not the whole outline: patterns, and images with transparent pixels or that leave part of the
/// box empty (Fit, a smaller crop). The layer's pixels then cast and hide its shadows.
fn alpha_fills(doc: &Document, n: &Node, counts: impl Fn(&Paint) -> bool) -> bool {
    let mut fills = n.fills.iter().filter(|f| counts(f)).peekable();
    fills.peek().is_some()
        && fills.all(|f| match &f.kind {
            PaintKind::Pattern(_) => true,
            PaintKind::Image(img) => !crate::paint::fills_box_opaque(doc, img, n.width, n.height),
            PaintKind::Solid { .. } | PaintKind::Gradient(_) => false,
        })
}

/// Whether `n`'s inner shadows are cast from around its pixels, not its outline: groups (no
/// outline of their own) and layers whose fills paint their own alpha (see [`alpha_fills`]).
pub fn inner_from_pixels(doc: &Document, n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group) || alpha_fills(doc, n, |f| f.visible)
}

fn outlines(doc: &Document, n: &Node, fill_counts: fn(&Paint) -> bool) -> Option<Vec<Outline>> {
    outlines_with(doc, n, fill_counts, false)
}

/// [`outlines`]; `kids_within`: the caller knows the children paint nothing outside the outline.
fn outlines_with(doc: &Document, n: &Node, fill_counts: fn(&Paint) -> bool, kids_within: bool) -> Option<Vec<Outline>> {
    let has_fill = n.fills.iter().any(fill_counts);
    let is_bool = matches!(n.kind, NodeKind::BooleanOp { .. });
    let is_text = matches!(n.kind, NodeKind::Text(_));
    let kids = !is_bool && !kids_within && n.children.iter().any(|c| doc.node(*c).is_some_and(|c| c.visible));
    if matches!(n.kind, NodeKind::Group | NodeKind::Page { .. })
        || (kids && !(n.kind.is_frame_like() && n.clip && has_fill))
        || alpha_fills(doc, n, fill_counts)
    {
        return None;
    }
    let closed = is_bool || is_text || figcraft_doc::geometry::is_closed(n);
    let outline = node_outline(doc, n);
    let mut out = Vec::new();
    if has_fill
        && closed
        && let Some((path, rule)) = outline.clone()
    {
        // As the layer fills it: out to the middle of a stroke that hides it, so the fill and the
        // stroke share no anti-aliased edge that would count twice.
        out.push(fill_to_stroke_middle(n, &path, rule).map_or((path, rule), |(p, r)| ((*p).clone(), r)));
    }
    if n.has_visible_stroke() {
        // Booleans and text are stroked along the outline they fill.
        let geom = if is_bool || is_text { outline.map(|(p, _)| p) } else { figcraft_doc::geometry::stroke_geometry(n) };
        if let Some(g) = geom {
            out.push(((*stroke_area(n, &g, closed)).clone(), stroke_rule(n, closed)));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Cache of spread bands: (outline, width, direction, resolution) → the stroked band.
fn band_cache() -> &'static Mutex<HashMap<u64, Option<Arc<tiny_skia::Path>>>> {
    static C: OnceLock<Mutex<HashMap<u64, Option<Arc<tiny_skia::Path>>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The band within `half` of `path`'s outline, mitred at corners, in local space. Shrinking first
/// resolves overlapping non-zero contours (overlapping glyph parts, vector regions), whose inner
/// edges aren't outline and must not be cut away.
fn band(path: &BezPath, rule: FillRule, half: f64, shrink: bool, res: f32) -> Option<Arc<tiny_skia::Path>> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    hash_path(path, &mut h);
    (rule == FillRule::EvenOdd, half.to_bits(), shrink, res.to_bits()).hash(&mut h);
    let key = h.finish();
    if let Some(b) = band_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
        return b.clone();
    }
    let simplified = (shrink && rule == FillRule::Winding).then(|| figcraft_pathops::simplify(path).ok()).flatten();
    let stroke = tiny_skia::Stroke {
        width: (half * 2.0) as f32,
        line_join: tiny_skia::LineJoin::Miter,
        miter_limit: SPREAD_MITER_LIMIT as f32,
        line_cap: tiny_skia::LineCap::Butt,
        ..Default::default()
    };
    let made = ts_path(simplified.as_ref().unwrap_or(path)).and_then(|p| p.stroke(&stroke, res)).map(Arc::new);
    let mut c = band_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if c.len() > 1024 {
        c.clear();
    }
    c.insert(key, made.clone());
    made
}

fn white(blend: tiny_skia::BlendMode) -> tiny_skia::Paint<'static> {
    tiny_skia::Paint { shader: tiny_skia::Shader::SolidColor(tiny_skia::Color::WHITE), anti_alias: true, blend_mode: blend, ..Default::default() }
}

/// The coverage of `outlines` grown (`spread > 0`) or shrunk (`spread < 0`) by `spread` local
/// units, drawn through `xf` (local → pixels) into a `w` × `h` pixmap (white, premultiplied).
pub fn spread_alpha(outlines: &[Outline], spread: f64, xf: Affine, w: u32, h: u32) -> Option<Pixmap> {
    let mut p = Pixmap::new(w, h)?;
    let t = ts_transform(xf);
    let fill = white(tiny_skia::BlendMode::SourceOver);
    for (path, rule) in outlines {
        if let Some(tp) = ts_path(path) {
            p.fill_path(&tp, &fill, *rule, t, None);
        }
    }
    // Past a million pixels every spread looks the same, and the rasteriser needs sane coordinates.
    let px = xf.determinant().abs().sqrt();
    let half = if px.is_finite() && px > 0.0 { spread.abs().min(MAX_SPREAD_PX / px) } else { 0.0 };
    if !(half.is_finite() && half > 1e-6) {
        return Some(p);
    }
    // Curves flatten for the pixel size, bucketed to powers of two so zooming reuses bands.
    let res = tiny_skia::PathStroker::compute_resolution_scale(&t);
    let res = if res.is_finite() && res > 0.0 { 2f32.powi(res.log2().ceil().clamp(-8.0, 8.0) as i32) } else { 1.0 };
    let paint = white(if spread > 0.0 { tiny_skia::BlendMode::SourceOver } else { tiny_skia::BlendMode::Clear });
    for (path, rule) in outlines {
        if let Some(b) = band(path, *rule, half, spread < 0.0, res) {
            p.fill_path(&b, &paint, FillRule::Winding, t, None);
        }
    }
    Some(p)
}

/// A device distance as a whole pixel offset (non-finite → 0, capped).
pub(crate) fn whole_px(v: f64) -> i32 {
    if v.is_finite() { v.round().clamp(-100_000.0, 100_000.0) as i32 } else { 0 }
}

fn color4(c: figcraft_color::Color) -> [f32; 4] {
    let c = c.sanitized();
    [c.r, c.g, c.b, c.a]
}

/// What casts a node's shadows.
pub struct Caster<'a> {
    /// The node's pixels (content and inner shadows, with the effects drawn on top of them).
    pub layer: &'a Pixmap,
    /// Local → layer pixels.
    pub xf: Affine,
    /// Device pixels per document unit.
    pub scale: f64,
    /// See [`silhouette`]; only when a shadow spreads.
    pub silhouette: Option<Vec<Outline>>,
    /// The node's [`cover`] in the layer's pixels; only when a shadow hides behind the layer.
    pub cover: Option<Pixmap>,
    /// The node's opacity, which the layer is drawn over its shadows at.
    pub opacity: f32,
}

fn drop_shadows(n: &Node) -> impl Iterator<Item = &Shadow> {
    n.effects.iter().filter(|e| e.visible).filter_map(|e| if let EffectKind::DropShadow(s) = &e.kind { Some(s) } else { None })
}

/// What hides node `n`'s drop shadows that don't show behind transparent areas: its [`cover`],
/// or `None` when its pixels do that (layers that aren't one outline; effects that move the
/// pixels past the paints) or no shadow hides.
pub fn hiding_cover(doc: &Document, n: &Node) -> Option<Vec<Outline>> {
    let moved = n.effects.iter().any(|e| matches!(e.kind, EffectKind::LayerBlur(_) | EffectKind::Texture(_)) && e.reach() > 0.0);
    if moved || !drop_shadows(n).any(|s| !s.show_behind) {
        return None;
    }
    cover(doc, n)
}

impl<'a> Caster<'a> {
    /// What casts node `n`'s drop shadows from its `layer` (local → pixels through `xf`).
    pub fn new(doc: &Document, n: &Node, layer: &'a Pixmap, xf: Affine, scale: f64) -> Self {
        let silhouette = if drop_shadows(n).any(|s| s.spread != 0.0) { silhouette(doc, n) } else { None };
        let cover = hiding_cover(doc, n).and_then(|c| spread_alpha(&c, 0.0, xf, layer.width(), layer.height()));
        Caster { layer, xf, scale, silhouette, cover, opacity: n.opacity.clamp(0.0, 1.0) }
    }
}

/// A drop shadow in the layer's pixels: offset, spread, blurred, tinted and (unless it shows
/// behind the layer) hidden behind the layer: behind what its paints cover, translucent parts
/// included, or (layers that aren't one outline) behind its pixels.
pub fn drop_shadow(c: &Caster, s: &Shadow) -> Option<Pixmap> {
    let (w, h) = (c.layer.width(), c.layer.height());
    let (dx, dy) = (s.offset.0 * c.scale, s.offset.1 * c.scale);
    let spread = if s.spread.is_finite() { s.spread } else { 0.0 };
    let mut sh = match c.silhouette.as_deref() {
        Some(sil) if spread != 0.0 && dx.is_finite() && dy.is_finite() => spread_alpha(sil, spread, Affine::translate((dx, dy)) * c.xf, w, h)?,
        _ => {
            let mut base = c.layer.clone();
            fx::morph(&mut base, whole_px(spread * c.scale));
            let mut out = Pixmap::new(w, h)?;
            out.draw_pixmap(whole_px(dx), whole_px(dy), base.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            out
        }
    };
    fx::blur(&mut sh, s.radius / 2.0 * c.scale);
    fx::tint(&mut sh, color4(s.color));
    if !s.show_behind {
        match &c.cover {
            Some(cover) => fx::hide_behind(&mut sh, cover, c.layer, c.opacity),
            None => fx::mask_by(&mut sh, c.layer, true),
        }
    }
    Some(sh)
}

/// An inner shadow in a `w` × `h` layer whose `shape` maps through `xf`: everything outside the
/// shape (shrunk by the spread) at the offset, blurred, tinted and clipped to the shape.
pub fn inner_shadow(shape: &Outline, w: u32, h: u32, xf: Affine, scale: f64, s: &Shadow) -> Option<Pixmap> {
    let (dx, dy) = (s.offset.0 * scale, s.offset.1 * scale);
    let spread = if s.spread.is_finite() { s.spread } else { 0.0 };
    if !(dx.is_finite() && dy.is_finite()) {
        return None;
    }
    // A buffer padded by the shadow's reach, with the shape drawn at its offset: everything outside
    // the shape casts shadow inwards, from every edge. The shape itself (not the layer's box)
    // decides what is outside: the layer can be cropped (to a render band or the view) where the
    // shape carries on.
    let reach = s.radius.abs() / 2.0 * scale * 3.0 + spread.abs() * scale * SPREAD_MITER_LIMIT + dx.abs().max(dy.abs());
    let pad = if reach.is_finite() { reach.ceil().clamp(0.0, 2048.0) as u32 + 2 } else { 2 };
    let at = Affine::translate((f64::from(pad) + dx, f64::from(pad) + dy)) * xf;
    let big = spread_alpha(std::slice::from_ref(shape), -spread, at, w + pad * 2, h + pad * 2)?;
    inner_from(big, pad, &spread_alpha(std::slice::from_ref(shape), 0.0, xf, w, h)?, scale, s)
}

/// An inner shadow for a layer that isn't one outline (a group): cast inwards from everything
/// around what its pixels `cover` (the layers it draws, together), over them. Offsets and spreads
/// go by whole pixels.
pub fn inner_shadow_of(cover: &Pixmap, scale: f64, s: &Shadow) -> Option<Pixmap> {
    let (dx, dy) = (s.offset.0 * scale, s.offset.1 * scale);
    let spread = if s.spread.is_finite() { s.spread * scale } else { 0.0 };
    if !(dx.is_finite() && dy.is_finite()) {
        return None;
    }
    let reach = s.radius.abs() / 2.0 * scale * 3.0 + spread.abs() + dx.abs().max(dy.abs());
    let pad = if reach.is_finite() { reach.ceil().clamp(0.0, 2048.0) as u32 + 2 } else { 2 };
    let at = i32::try_from(pad).ok()?;
    let mut big = Pixmap::new(cover.width() + pad * 2, cover.height() + pad * 2)?;
    big.draw_pixmap(at + whole_px(dx), at + whole_px(dy), cover.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    fx::morph(&mut big, -whole_px(spread));
    inner_from(big, pad, cover, scale, s)
}

/// The inner shadow `s` from `big`: what casts it (the shape at the shadow's offset, less its
/// spread), `pad` pixels larger each way than the layer, whose pixels `cover` the shape.
fn inner_from(mut big: Pixmap, pad: u32, cover: &Pixmap, scale: f64, s: &Shadow) -> Option<Pixmap> {
    fx::invert_alpha(&mut big);
    fx::blur(&mut big, s.radius / 2.0 * scale);
    let mut out = Pixmap::new(cover.width(), cover.height())?;
    let pad = i32::try_from(pad).ok()?;
    out.draw_pixmap(
        -pad,
        -pad,
        big.as_ref(),
        &PixmapPaint { blend_mode: tiny_skia::BlendMode::Source, ..Default::default() },
        Transform::identity(),
        None,
    );
    fx::mask_by(&mut out, cover, false);
    fx::tint(&mut out, color4(s.color));
    Some(out)
}

/// Whether `p` paints every pixel it covers at full alpha: a visible, opaque solid colour.
fn opaque(p: &Paint) -> bool {
    p.visible && p.opacity >= 1.0 && p.blend == figcraft_color::BlendMode::Normal && matches!(&p.kind, PaintKind::Solid { color } if color.a >= 1.0)
}

/// The outlines node `n`'s drop shadows are cast from when its pixels are those outlines, filled:
/// a shape or a clipping frame whose paints are opaque colours and that has no effect that moves
/// its pixels. Such a shadow needs none of the layer's pixels, so it is drawn only where it
/// shows and blurred at the resolution its radius needs ([`outline_drop_shadow`]), not from a
/// copy of the layer reaching past the view. `None`: the shadows come from the layer's pixels.
pub fn outline_caster(doc: &Document, n: &Node) -> Option<Vec<Outline>> {
    #[cfg(test)]
    if FROM_PIXELS.with(std::cell::Cell::get) {
        return None;
    }
    let shape =
        n.kind.is_frame_like() || matches!(n.kind, NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. });
    let plain = n.effects.iter().filter(|e| e.visible).all(|e| matches!(e.kind, EffectKind::DropShadow(_) | EffectKind::InnerShadow(_)));
    let fills = n.fills.iter().any(opaque);
    let strokes = !n.has_visible_stroke() || n.strokes.iter().filter(|s| s.visible).all(opaque);
    if !(shape && plain && fills && strokes && figcraft_doc::geometry::is_closed(n)) {
        return None;
    }
    // A frame that doesn't clip is its outline too while its children stay inside it. Its
    // shadows spread as its pixels do (see the module docs), not as an outline: those stay.
    let within = n.kind.is_frame_like() && !n.clip && drop_shadows(n).all(|s| s.spread == 0.0) && children_within(doc, n);
    outlines_with(doc, n, |f| f.visible && f.opacity > 0.0, within)
}

/// Whether everything the children of frame `n` paint (effects and all) lies inside its box, its
/// rounded corners cut off: they then add nothing to the pixels its fill covers.
fn children_within(doc: &Document, n: &Node) -> bool {
    let world = doc.world_transform(n.id);
    let det = world.determinant();
    if !(det.is_finite() && det.abs() > 1e-12) {
        return false;
    }
    let to_local = world.inverse();
    n.children.iter().filter(|c| doc.node(**c).is_some_and(|c| c.visible)).all(|c| {
        crate::draw::paint_bounds(doc, *c).is_some_and(|b| {
            [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)]
                .into_iter()
                .all(|p| crate::draw::in_box(n, to_local * figcraft_geom::Point::new(p.0, p.1)))
        })
    })
}

#[cfg(test)]
thread_local! {
    /// Tests: every drop shadow is cast from its layer's pixels, as before [`outline_caster`].
    pub(crate) static FROM_PIXELS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Shadows blurred by less than this many device pixels (standard deviation) are drawn at full
/// resolution, as they always were.
const FULL_RES_SIGMA: f64 = 8.0;
/// The most a shadow is drawn smaller by before its blur: a blur of 4 to 8 small pixels.
const MAX_SMALLER: u32 = 256;

/// The longest side of a pixmap.
const MAX_SIDE: u32 = 16_384;
/// What the pixmap a shadow is blurred in may have more than [`MAX_SIDE`] a side: the blur's
/// reach both ways at full resolution (under [`FULL_RES_SIGMA`]: 34 pixels) and a pixel's slack.
/// Drawn smaller, it is far under the limit.
const BLUR_ROOM: u32 = 128;

/// A whole pixel range `[lo, hi)` as `(start, length)`; `None` when empty or longer than `most`.
fn span(lo: f64, hi: f64, most: u32) -> Option<(i32, u32)> {
    if !(lo.is_finite() && hi.is_finite() && hi > lo && lo.abs() < 1e9 && hi.abs() < 1e9) {
        return None;
    }
    let len = (hi - lo) as u32;
    (1..=most).contains(&len).then_some((lo as i32, len))
}

/// The most `xf` stretches a length, in any direction (its largest singular value): how far a
/// distance in local units can reach on the pixmap.
fn stretch(xf: Affine) -> f64 {
    let c = xf.as_coeffs();
    let sum = c[0] * c[0] + c[1] * c[1] + c[2] * c[2] + c[3] * c[3];
    let det = c[0] * c[3] - c[1] * c[2];
    ((sum + (sum * sum - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt()
}

/// The drop shadow `s` cast by `outlines` (local space, drawn at `xf` into a `size` pixmap), where
/// it shows on that pixmap: the shadow's pixels and where their top-left goes. Unless it shows
/// behind the layer, it is hidden behind `cover`, which the layer paints opaque at `opacity`.
///
/// A shadow blurred widely is drawn `k` times smaller, blurred there and scaled back up: a wide
/// blur has no detail a pixel across. The small pixels line up with the shape, not with the
/// pixmap, so every render of a view (a band, a patch, the whole of it) draws the same shadow.
pub fn outline_drop_shadow(
    outlines: &[Outline],
    cover: &[Outline],
    s: &Shadow,
    xf: Affine,
    scale: f64,
    size: (u32, u32),
    opacity: f32,
) -> Option<(Pixmap, (i32, i32))> {
    let spread = if s.spread.is_finite() { s.spread } else { 0.0 };
    let sigma = (s.radius / 2.0 * scale).min(fx::MAX_SIGMA);
    let sigma = if sigma.is_finite() && sigma > 0.3 { sigma } else { 0.0 };
    // As from the layer's pixels: without a spread the offset is whole pixels.
    let (dx, dy) = (s.offset.0 * scale, s.offset.1 * scale);
    let (dx, dy) = if spread != 0.0 && dx.is_finite() && dy.is_finite() { (dx, dy) } else { (f64::from(whole_px(dx)), f64::from(whole_px(dy))) };
    let cast = Affine::translate((dx, dy)) * xf;
    // Where the shadow can show: its outlines, spread and blurred, on the pixmap.
    let reach = (sigma * 4.0).ceil() + 2.0;
    // A spread grows the outline in local units: on the pixmap, as far as the transform
    // stretches them the most (it need not scale both ways alike).
    let px = stretch(xf);
    let grown = if px.is_finite() { (spread.abs() * px * SPREAD_MITER_LIMIT).min(MAX_SPREAD_PX) } else { 0.0 };
    let shape =
        outlines.iter().map(|(p, _)| figcraft_geom::transform_rect_bbox(cast, figcraft_geom::Shape::bounding_box(p))).reduce(|a, b| a.union(b))?;
    let shows = shape.inflate(grown + reach, grown + reach).intersect(figcraft_geom::Rect::new(0.0, 0.0, f64::from(size.0), f64::from(size.1)));
    let (x0, w) = span(shows.x0.floor(), shows.x1.ceil(), MAX_SIDE)?;
    let (y0, h) = span(shows.y0.floor(), shows.y1.ceil(), MAX_SIDE)?;
    let smaller = if sigma >= FULL_RES_SIGMA { (2f64.powi((sigma / 4.0).log2().floor() as i32) as u32).clamp(1, MAX_SMALLER) } else { 1 };
    let k = f64::from(smaller);
    // The small pixmap's pixels are `k` device pixels a side, counted from the shape's origin
    // (the pixmap's own pixels when nothing is drawn smaller): those the shadow's area and
    // what blurs into it take in.
    let c = cast.as_coeffs();
    let (ax, ay) = if smaller > 1 { (c[4], c[5]) } else { (0.0, 0.0) };
    let (i0, lw) =
        span(((f64::from(x0) - reach - ax) / k).floor() - 1.0, ((f64::from(x0) + f64::from(w) + reach - ax) / k).ceil() + 1.0, MAX_SIDE + BLUR_ROOM)?;
    let (j0, lh) =
        span(((f64::from(y0) - reach - ay) / k).floor() - 1.0, ((f64::from(y0) + f64::from(h) + reach - ay) / k).ceil() + 1.0, MAX_SIDE + BLUR_ROOM)?;
    let small = Affine::new([c[0] / k, c[1] / k, c[2] / k, c[3] / k, (c[4] - ax) / k - f64::from(i0), (c[5] - ay) / k - f64::from(j0)]);
    let mut low = spread_alpha(outlines, spread, small, lw, lh)?;
    fx::blur(&mut low, sigma / k);
    let mut out = Pixmap::new(w, h)?;
    let tints = fx::tints(color4(s.color));
    if smaller > 1 {
        // Device pixel centre → small pixel centres.
        let at = |v: i32, a: f64, first: i32| (f64::from(v) + 0.5 - a) / k - 0.5 - f64::from(first);
        fx::enlarge_alpha(&low, &mut out, (at(x0, ax, i0), at(y0, ay, j0)), 1.0 / k, &tints);
    } else {
        fx::copy_alpha(&low, &mut out, (x0 - i0, y0 - j0), &tints);
    }
    if !s.show_behind {
        let hides = spread_alpha(cover, 0.0, Affine::translate((-f64::from(x0), -f64::from(y0))) * xf, w, h)?;
        fx::hide_behind(&mut out, &hides, &hides, opacity);
    }
    Some((out, (x0, y0)))
}
