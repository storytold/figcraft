//! SVG paints → FigCraft paints: colours; gradients as editable gradients with their handles
//! placed exactly (any `gradientTransform`, skewed or not), `reflect` and `repeat` spreads baked
//! into the stops across the layer's box and a focal point as the centre; patterns as image fills.
//! A pattern of one image becomes that image (fill, fit or cropped when it shows once, tiled when
//! it repeats); any other pattern is drawn into a tile image first.

use figcraft_color::{Color, ColorStop, Gradient, GradientKind, ImagePaint, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::{Document, NodeKind};
use figcraft_geom::{Affine, Point, Rect, Vec2};

use super::shape::length_scale;
use super::{Ctx, add_image, children, finite, xf};

/// Most periods a reflected or repeated gradient is unrolled into (more pads instead).
const MAX_PERIODS: f64 = 32.0;

pub(crate) fn color(c: usvg::Color, alpha: f32) -> Color {
    Color::new(f32::from(c.red) / 255.0, f32::from(c.green) / 255.0, f32::from(c.blue) / 255.0, alpha.clamp(0.0, 1.0))
}

/// Where a paint lands: `to_node` maps the paint's user space into the layer's local space, whose
/// box is `size`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Target {
    pub to_node: Affine,
    pub size: (f64, f64),
}

impl Target {
    /// Node-local → the unit box the gradient handles live in.
    fn unit(&self, p: Point) -> (f64, f64) {
        (p.x / self.size.0.max(1e-6), p.y / self.size.1.max(1e-6))
    }
}

/// A usvg paint with `opacity`, for a layer at `t`. `None` when it paints nothing.
pub(crate) fn paint(cx: &mut Ctx, p: &usvg::Paint, opacity: f32, t: Target) -> Option<Paint> {
    let mut out = match p {
        usvg::Paint::Color(c) => Paint::solid(color(*c, 1.0)),
        usvg::Paint::LinearGradient(g) => {
            let m = t.to_node * xf(g.transform());
            let (p1, p2) = (Point::new(f64::from(g.x1()), f64::from(g.y1())), Point::new(f64::from(g.x2()), f64::from(g.y2())));
            let d = p2 - p1;
            // Colour is constant across the perpendicular in gradient space: that's the third handle.
            let frame = [m * p1, m * p2, m * (p1 + Vec2::new(-d.y, d.x))];
            Paint::gradient(spread(GradientKind::Linear, frame, stops(g.stops()), g.spread_method(), t)?)
        }
        usvg::Paint::RadialGradient(g) => {
            let m = t.to_node * xf(g.transform());
            let r = f64::from(g.r().get());
            // The focal point is the brightest spot: centre the gradient there.
            let c = Point::new(f64::from(g.fx()), f64::from(g.fy()));
            let frame = [m * c, m * (c + Vec2::new(r, 0.0)), m * (c + Vec2::new(0.0, r))];
            Paint::gradient(spread(GradientKind::Radial, frame, stops(g.stops()), g.spread_method(), t)?)
        }
        usvg::Paint::Pattern(pat) => pattern(cx, pat, t)?,
    };
    out.opacity *= opacity.clamp(0.0, 1.0);
    Some(out)
}

fn stops(s: &[usvg::Stop]) -> Vec<ColorStop> {
    s.iter().map(|s| ColorStop { position: s.offset().get(), color: color(s.color(), s.opacity().get()) }).collect()
}

/// The gradient with handles `frame` (node space: origin, the `u = 1` end, the `v = 1` end), its
/// spread unrolled over the layer's box.
fn spread(kind: GradientKind, frame: [Point; 3], stops: Vec<ColorStop>, method: usvg::SpreadMethod, t: Target) -> Option<Gradient> {
    let [o, a, b] = frame;
    if ![o, a, b].iter().all(|p| p.x.is_finite() && p.y.is_finite()) || stops.is_empty() {
        return None;
    }
    let (lo, hi) = match method {
        usvg::SpreadMethod::Pad => (0.0, 1.0),
        _ => periods(kind, frame, t).unwrap_or((0.0, 1.0)),
    };
    let reflect = method == usvg::SpreadMethod::Reflect;
    let mut all = Vec::new();
    let n = hi - lo;
    let mut k = lo;
    while k < hi {
        let flip = reflect && (k as i64).rem_euclid(2) == 1;
        let period: Vec<ColorStop> =
            if flip { stops.iter().rev().map(|s| ColorStop { position: 1.0 - s.position, color: s.color }).collect() } else { stops.clone() };
        all.extend(period.into_iter().map(|s| ColorStop { position: ((k - lo + f64::from(s.position)) / n) as f32, color: s.color }));
        k += 1.0;
    }
    let (u, v) = (a - o, b - o);
    let (o2, a2, b2) = match kind {
        // Linear: the axis stretches over the periods; the perpendicular stays.
        GradientKind::Linear => (o + u * lo, o + u * hi, o + u * lo + v),
        _ => (o, o + u * hi, o + v * hi),
    };
    Some(Gradient { kind, stops: all, from: t.unit(o2), to: t.unit(a2), width: t.unit(b2) })
}

/// The whole periods (`lo..hi`) a reflected or repeated gradient needs to cover the layer's box.
fn periods(kind: GradientKind, [o, a, b]: [Point; 3], t: Target) -> Option<(f64, f64)> {
    let inv = Affine::new([a.x - o.x, a.y - o.y, b.x - o.x, b.y - o.y, o.x, o.y]);
    if inv.determinant().abs() < 1e-12 {
        return None;
    }
    let inv = inv.inverse();
    let (w, h) = t.size;
    let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y)| inv * Point::new(x, y));
    let (lo, hi) = match kind {
        GradientKind::Linear => corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), p| (l.min(p.x), h.max(p.x))),
        _ => (0.0, corners.iter().map(|p| p.to_vec2().hypot()).fold(0.0, f64::max)),
    };
    let (lo, hi) = (lo.floor().min(0.0), hi.ceil().max(1.0));
    (lo.is_finite() && hi.is_finite() && hi - lo <= MAX_PERIODS).then_some((lo, hi))
}

/// Largest side of a pattern tile drawn into an image, in pixels.
const MAX_TILE: f64 = 2048.0;

/// A pattern as an image paint (see the module docs). `None` when it shows nothing or can't be
/// placed (a rotated, skewed or spaced-out repeating image).
fn pattern(cx: &mut Ctx, pat: &usvg::Pattern, t: Target) -> Option<Paint> {
    let r = pat.rect();
    let (rw, rh) = (f64::from(r.width()), f64::from(r.height()));
    let tile = t.to_node * xf(pat.transform()) * Affine::translate((f64::from(r.x()), f64::from(r.y())));
    if !finite(tile) || tile.determinant().abs() < 1e-12 {
        return None;
    }
    let tile_rect = Rect::new(0.0, 0.0, rw, rh).inflate(1e-3, 1e-3);
    // One image inside the tile is used as it is; anything else is drawn into a tile image.
    let lone = lone_image(pat.root(), Affine::IDENTITY, 0).and_then(|(img, m)| {
        let bytes = match img.kind() {
            usvg::ImageKind::PNG(b) | usvg::ImageKind::JPEG(b) | usvg::ImageKind::GIF(b) | usvg::ImageKind::WEBP(b) => b.clone(),
            usvg::ImageKind::SVG(_) => return None,
        };
        let s = img.size();
        let (iw, ih) = (f64::from(s.width()), f64::from(s.height()));
        let b = figcraft_geom::transform_rect_bbox(m, Rect::new(0.0, 0.0, iw, ih));
        (tile_rect.contains(b.origin()) && tile_rect.contains(Point::new(b.x1, b.y1))).then(|| (add_image(cx.doc, bytes, iw, ih), iw, ih, m))
    });
    let (hash, iw, ih, content) = match lone {
        Some(l) => l,
        None => render_tile(cx, pat, tile)?,
    };
    let image = figcraft_geom::transform_rect_bbox(content, Rect::new(0.0, 0.0, iw, ih));
    let node = figcraft_geom::transform_rect_bbox(tile.inverse(), Rect::new(0.0, 0.0, t.size.0, t.size.1));
    // The copies of the image (one per tile) that show in the layer's box.
    let copies = |lo: f64, hi: f64, ilo: f64, ihi: f64, period: f64| -> (f64, f64) {
        let eps = 1e-6;
        (((lo - ihi) / period + eps).ceil(), ((hi - ilo) / period - eps).floor())
    };
    let (kx, ky) = (copies(node.x0, node.x1, image.x0, image.x1, rw), copies(node.y0, node.y1, image.y0, image.y1, rh));
    if !(kx.0 <= kx.1 && ky.0 <= ky.1) {
        return None;
    }
    let mut ip = ImagePaint { image: hash, scale_mode: ImageScaleMode::Fill, scale: 1.0, rotation: 0, crop: None, filters: Default::default() };
    if kx.0 == kx.1 && ky.0 == ky.1 {
        // Shows once: fill, fit or a crop placing it exactly.
        let m = tile * Affine::translate((kx.0 * rw, ky.0 * rh)) * content;
        let (w, h) = t.size;
        let same = |mode: ImageScaleMode| {
            let ts = figcraft_render::paint::image_transform(&ImagePaint { scale_mode: mode, ..ip.clone() }, iw, ih, w, h);
            let k = xf(ts).as_coeffs();
            m.as_coeffs().iter().zip(k).all(|(a, b)| (a - b).abs() <= 1e-3 * (1.0 + a.abs()))
        };
        if let Some(mode) = [ImageScaleMode::Fill, ImageScaleMode::Fit].into_iter().find(|m| same(*m)) {
            ip.scale_mode = mode;
        } else {
            let crop = Affine::scale_non_uniform(1.0 / iw.max(1e-9), 1.0 / ih.max(1e-9)) * m.inverse() * Affine::scale_non_uniform(w, h);
            ip.scale_mode = ImageScaleMode::Crop;
            ip.crop = Some(crop.as_coeffs());
        }
    } else {
        // Repeats: tiles, when the image is the whole tile, upright and evenly scaled.
        let m = tile * content;
        let [a, b, c, d, e, f] = m.as_coeffs();
        let whole = (image.width() - rw).abs() < 1e-3 * rw && (image.height() - rh).abs() < 1e-3 * rh;
        if !(whole && b.abs() < 1e-9 && c.abs() < 1e-9 && a > 0.0 && (a - d).abs() < 1e-6 * a) {
            cx.note("a rotated, skewed or spaced-out pattern was left out");
            return None;
        }
        let phase = |v: f64, period: f64| {
            let p = v.rem_euclid(period) / period;
            p.min(1.0 - p)
        };
        if phase(e, iw * a) > 1e-3 || phase(f, ih * a) > 1e-3 {
            cx.note("a pattern's offset was dropped: its tiles start at the layer's corner");
        }
        ip.scale_mode = ImageScaleMode::Tile;
        ip.scale = a;
    }
    let mut out = Paint::image(String::new(), ImageScaleMode::Fill);
    out.kind = PaintKind::Image(ip);
    Some(out)
}

/// The only thing under `g` when that's a raster image (through plain groups), with its transform
/// (composed from the groups' own transforms: see [`unstale`]).
fn lone_image(g: &usvg::Group, m: Affine, depth: usize) -> Option<(&usvg::Image, Affine)> {
    match g.children() {
        [usvg::Node::Image(i)] if i.is_visible() => Some((i.as_ref(), m)),
        [usvg::Node::Group(sub)]
            if depth < 16 && sub.opacity().get() >= 1.0 && sub.clip_path().is_none() && sub.mask().is_none() && sub.filters().is_empty() =>
        {
            lone_image(sub, m * xf(sub.transform()), depth + 1)
        }
        _ => None,
    }
}

/// A pattern's content and the transform its absolute transforms miss. usvg wraps the content of
/// a pattern in bounding-box or view box units in a group carrying that scale after the fact,
/// without updating the absolute transforms beneath: unwrap such groups into the transform.
fn unstale(root: &usvg::Group) -> (&usvg::Group, Affine) {
    let near = |a: Affine, b: Affine| a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| (x - y).abs() <= 1e-4 * (1.0 + x.abs()));
    let (mut g, mut base) = (root, Affine::IDENTITY);
    for _ in 0..4 {
        let [usvg::Node::Group(w)] = g.children() else { break };
        let t = xf(w.transform());
        let stale = match w.children().first() {
            Some(usvg::Node::Group(c)) => near(xf(c.abs_transform()), xf(c.transform())),
            Some(c) => near(xf(c.abs_transform()), Affine::IDENTITY),
            None => false,
        };
        if t == Affine::IDENTITY || !stale || !near(xf(w.abs_transform()), t) {
            break;
        }
        base *= t;
        g = w;
    }
    (g, base)
}

/// A pattern tile drawn into an image: the image's hash, its size and its pixels → pattern space
/// transform.
pub(crate) type Tile = (String, f64, f64, Affine);

/// Draws pattern `pat`'s tile into an image (at twice its size on the page, at most [`MAX_TILE`]
/// a side), once per pattern and scale in an import: other uses get the same tile.
fn render_tile(cx: &mut Ctx, pat: &usvg::Pattern, tile: Affine) -> Option<Tile> {
    if cx.pattern_depth >= 2 {
        return None;
    }
    let r = pat.rect();
    let (rw, rh) = (f64::from(r.width()), f64::from(r.height()));
    let k = (length_scale(tile) * 2.0).max(0.01).min(MAX_TILE / rw.max(rh));
    // The tile depends only on the pattern and the scale; the tree outlives the import, so its
    // address names the pattern (ids repeat across embedded documents).
    let key = (std::ptr::from_ref(pat), k.to_bits());
    if let Some(t) = cx.tiles.get(&key) {
        return t.clone();
    }
    let t = draw_tile(cx, pat, k);
    cx.tiles.insert(key, t.clone());
    t
}

/// [`render_tile`]'s drawing at scale `k`, its layers counted in the import's layer budget.
fn draw_tile(cx: &mut Ctx, pat: &usvg::Pattern, k: f64) -> Option<Tile> {
    let r = pat.rect();
    let (rw, rh) = (f64::from(r.width()), f64::from(r.height()));
    let (pw, ph) = ((rw * k).ceil().max(1.0), (rh * k).ceil().max(1.0));
    if !(pw.is_finite() && ph.is_finite()) || pw > MAX_TILE || ph > MAX_TILE || cx.full() {
        return None;
    }
    let mut sub = Document::new("Pattern");
    let page = sub.first_page()?;
    let frame = sub.create(page, NodeKind::Frame, "Tile", 0.0, 0.0, rw, rh).ok()?;
    if let Some(f) = sub.node_mut(frame) {
        f.fills.clear();
    }
    let (notes, count) = {
        let mut scx = Ctx::new(&mut sub);
        scx.count = cx.count;
        scx.pattern_depth = cx.pattern_depth + 1;
        let (content, base) = unstale(pat.root());
        children(&mut scx, content, frame, base, 0);
        (std::mem::take(&mut scx.notes), scx.count)
    };
    cx.count = count;
    cx.notes.extend(notes);
    let view = figcraft_render::View { origin: (0.0, 0.0), scale: k, width: pw as u32, height: ph as u32 };
    let mut pix = figcraft_render::Pixmap::new(pw as u32, ph as u32)?;
    figcraft_render::render_nodes_into(&sub, &[frame], &view, &mut pix);
    let png = figcraft_render::encode_png(&pix).ok()?;
    let hash = add_image(cx.doc, std::sync::Arc::new(png), pw, ph);
    Some((hash, pw, ph, Affine::scale_non_uniform(rw / pw, rh / ph)))
}
