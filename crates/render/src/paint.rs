//! Paints → tiny-skia shaders, in a node's local space (width × height box).

use std::sync::Arc;

use figcraft_color::{BlendMode, Color, Gradient, GradientKind, ImagePaint, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::{Document, ImageData};
use tiny_skia::{FilterQuality, GradientStop, Pixmap, Point, Shader, SpreadMode, Transform};

pub fn ts_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(c.r, c.g, c.b, c.a).unwrap_or(tiny_skia::Color::BLACK)
}

pub fn ts_blend(b: BlendMode) -> tiny_skia::BlendMode {
    use tiny_skia::BlendMode as T;
    match b {
        BlendMode::PassThrough | BlendMode::Normal => T::SourceOver,
        BlendMode::Darken => T::Darken,
        BlendMode::Multiply => T::Multiply,
        // Plus darker has no tiny-skia mode: `composite` draws it; elsewhere the nearest one.
        BlendMode::PlusDarker => T::Multiply,
        BlendMode::ColorBurn => T::ColorBurn,
        BlendMode::Lighten => T::Lighten,
        BlendMode::Screen => T::Screen,
        BlendMode::PlusLighter => T::Plus,
        BlendMode::ColorDodge => T::ColorDodge,
        BlendMode::Overlay => T::Overlay,
        BlendMode::SoftLight => T::SoftLight,
        BlendMode::HardLight => T::HardLight,
        BlendMode::Difference => T::Difference,
        BlendMode::Exclusion => T::Exclusion,
        BlendMode::Hue => T::Hue,
        BlendMode::Saturation => T::Saturation,
        BlendMode::Color => T::Color,
        BlendMode::Luminosity => T::Luminosity,
    }
}

/// Draw `src` (top-left at `at`) onto `dst` at `opacity`, blended by `blend`.
pub fn composite(dst: &mut Pixmap, at: (i32, i32), src: &Pixmap, opacity: f32, blend: BlendMode) {
    if blend == BlendMode::PlusDarker {
        crate::fx::plus_darker(dst, at, src, opacity);
    } else {
        let paint = tiny_skia::PixmapPaint { opacity, blend_mode: ts_blend(blend), ..Default::default() };
        dst.draw_pixmap(at.0, at.1, src.as_ref(), &paint, Transform::identity(), None);
    }
}

fn stops(g: &Gradient, opacity: f32) -> Vec<GradientStop> {
    g.sorted_stops().into_iter().map(|s| GradientStop::new(s.position, ts_color(s.color.with_alpha(s.color.a * opacity)))).collect()
}

/// The unit-box → local mapping for a gradient's handles: maps (0,0)→from, (1,0)→to,
/// (0,1)→width (relative to from), in a w×h box.
fn handle_frame(g: &Gradient, w: f64, h: f64) -> Transform {
    let f = (g.from.0 * w, g.from.1 * h);
    let t = (g.to.0 * w, g.to.1 * h);
    let k = (g.width.0 * w, g.width.1 * h);
    Transform::from_row((t.0 - f.0) as f32, (t.1 - f.1) as f32, (k.0 - f.0) as f32, (k.1 - f.1) as f32, f.0 as f32, f.1 as f32)
}

/// What a paint needs beyond a shader: an owned pixmap backing an image/diamond pattern.
pub enum Prepared {
    Shader(Shader<'static>),
    Pattern { pixmap: Arc<Pixmap>, mode: SpreadMode, quality: FilterQuality, opacity: f32, transform: Transform },
}

impl Prepared {
    pub fn shader(&self) -> Shader<'_> {
        match self {
            Prepared::Shader(s) => s.clone(),
            Prepared::Pattern { pixmap, mode, quality, opacity, transform } => {
                tiny_skia::Pattern::new(pixmap.as_ref().as_ref(), *mode, *quality, *opacity, *transform)
            }
        }
    }
}

fn diamond_pixmap(g: &Gradient, opacity: f32) -> Option<Arc<Pixmap>> {
    const N: u32 = 256;
    let mut p = Pixmap::new(N, N)?;
    let half = f64::from(N) / 2.0;
    for (i, px) in p.data_mut().as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = ((i as u32 % N) as f64 + 0.5 - half) / half;
        let y = ((i as u32 / N) as f64 + 0.5 - half) / half;
        let t = (x.abs() + y.abs()) as f32;
        let c = g.sample(t);
        let a = c.a * opacity;
        let q = |v: f32| (v * a * 255.0).round().clamp(0.0, 255.0) as u8;
        px.copy_from_slice(&[q(c.r), q(c.g), q(c.b), (a * 255.0).round().clamp(0.0, 255.0) as u8]);
    }
    Some(Arc::new(p))
}

/// Build the shader for `paint` on a `w`×`h` local box shown at `scale` device pixels per unit
/// (the resolution pattern tiles are drawn at). Returns `None` when the paint draws nothing
/// (invisible, fully transparent, missing image or pattern source).
pub fn prepare(paint: &Paint, w: f64, h: f64, scale: f64, doc: &Document) -> Option<Prepared> {
    prepare_at(paint, w, h, scale, doc, 0.0, false)
}

/// [`prepare`] `time` seconds into animated GIFs (0: their first frame); on the canvas (`preview`)
/// a large image shown smaller than its pixels draws from a smaller copy (see [`still`]).
pub fn prepare_at(paint: &Paint, w: f64, h: f64, scale: f64, doc: &Document, time: f64, preview: bool) -> Option<Prepared> {
    if !paint.visible || paint.opacity <= 0.0 {
        return None;
    }
    let op = paint.opacity.clamp(0.0, 1.0);
    match &paint.kind {
        PaintKind::Solid { color } => {
            let c = color.with_alpha(color.a * op);
            (c.a > 0.0).then(|| Prepared::Shader(Shader::SolidColor(ts_color(c))))
        }
        PaintKind::Gradient(g) => {
            let frame = handle_frame(g, w.max(1e-6), h.max(1e-6));
            let st = stops(g, op);
            let shader = match g.kind {
                GradientKind::Linear => {
                    tiny_skia::LinearGradient::new(Point::from_xy(0.0, 0.0), Point::from_xy(1.0, 0.0), st, SpreadMode::Pad, frame)
                }
                GradientKind::Radial => {
                    tiny_skia::RadialGradient::new(Point::from_xy(0.0, 0.0), 0.0, Point::from_xy(0.0, 0.0), 1.0, st, SpreadMode::Pad, frame)
                }
                GradientKind::Angular => tiny_skia::SweepGradient::new(Point::from_xy(0.0, 0.0), 0.0, 360.0, st, SpreadMode::Pad, frame),
                GradientKind::Diamond => {
                    let pm = diamond_pixmap(g, op)?;
                    // Pattern pixel space (0..256) → unit square (-1..1) → handle frame.
                    let to_unit = Transform::from_row(2.0 / 256.0, 0.0, 0.0, 2.0 / 256.0, -1.0, -1.0);
                    return Some(Prepared::Pattern {
                        pixmap: pm,
                        mode: SpreadMode::Pad,
                        quality: FilterQuality::Bilinear,
                        opacity: 1.0,
                        transform: to_unit.post_concat(frame),
                    });
                }
            };
            shader.map(Prepared::Shader)
        }
        PaintKind::Image(img) => {
            let data = doc.images.get(&img.image);
            let moving = crate::images::frame_at(&img.image, data, &img.filters, doc.color_profile, time);
            let pm = match &moving {
                Some(f) => f.clone(),
                None => still(img, data, w, h, scale, doc, preview)?,
            };
            let transform = image_transform(img, f64::from(pm.width()), f64::from(pm.height()), w, h);
            if img.scale_mode == ImageScaleMode::Tile {
                return Some(Prepared::Pattern { pixmap: pm, mode: SpreadMode::Repeat, quality: FilterQuality::Bicubic, opacity: op, transform });
            }
            // An image covering the whole box fills it to its edges (a transparent border would
            // fade them where the image is scaled up).
            if covers_box(img, f64::from(pm.width()), f64::from(pm.height()), w, h) {
                return Some(Prepared::Pattern { pixmap: pm, mode: SpreadMode::Pad, quality: FilterQuality::Bicubic, opacity: op, transform });
            }
            // Fit/Crop: a transparent 1-px border so Pad spreads transparency, not edge pixels.
            // A GIF's later frames each show once per loop: not worth caching padded.
            let padded = match moving {
                Some(_) => Arc::new(crate::images::pad(&pm)?),
                None => crate::images::padded(&img.image, &img.filters, doc.color_profile, &pm)?,
            };
            let transform = Transform::from_translate(-1.0, -1.0).post_concat(transform);
            Some(Prepared::Pattern { pixmap: padded, mode: SpreadMode::Pad, quality: FilterQuality::Bicubic, opacity: op, transform })
        }
        PaintKind::Pattern(pt) => crate::pattern::prepare(doc, pt, w, h, scale, time, op),
    }
}

/// The pixmap image paint `img` draws in a `w` × `h` box at `scale` device pixels per unit: on the
/// canvas (`preview`) a copy no larger than the image shows ([`crate::images::shown`]), else the
/// image itself. Tiles keep the image: their size is in its pixels.
fn still(img: &ImagePaint, data: Option<&ImageData>, w: f64, h: f64, scale: f64, doc: &Document, preview: bool) -> Option<Arc<Pixmap>> {
    let (iw, ih) = data.map_or((0, 0), |d| (d.width, d.height));
    if preview && img.scale_mode != ImageScaleMode::Tile && iw > 0 && ih > 0 {
        let t = image_transform(img, f64::from(iw), f64::from(ih), w, h);
        let side = scale * f64::from(t.sx * t.sy - t.kx * t.ky).abs().sqrt() * f64::from(iw.max(ih));
        return crate::images::shown(&img.image, data, &img.filters, doc.color_profile, side);
    }
    crate::images::get(&img.image, data, &img.filters, doc.color_profile)
}

/// Whether image paint `img` paints its whole `w` × `h` box opaque: an image without transparent
/// pixels that covers the box (or tiles it).
/// shortcut: always looks at the image itself (exact, also on the canvas), so a large image with
/// shadows still decodes in full there; pass the canvas scale here if that turns up (#1483).
pub(crate) fn fills_box_opaque(doc: &Document, img: &ImagePaint, w: f64, h: f64) -> bool {
    let Some(pm) = crate::images::get(&img.image, doc.images.get(&img.image), &img.filters, doc.color_profile) else { return false };
    (img.scale_mode == ImageScaleMode::Tile || covers_box(img, f64::from(pm.width()), f64::from(pm.height()), w, h))
        && crate::images::opaque(&img.image, &pm)
}

/// Whether an image paint covers its whole `w` × `h` box: Fill always, Crop when its frame does.
fn covers_box(img: &ImagePaint, iw: f64, ih: f64, w: f64, h: f64) -> bool {
    match img.scale_mode {
        ImageScaleMode::Fill => true,
        ImageScaleMode::Crop => {
            let m = img.effective_crop(iw, ih, w, h);
            let (x0, y0, x1, y1) = ImagePaint::crop_rect(m);
            m[1] == 0.0 && m[2] == 0.0 && x0.min(x1) <= 0.0 && y0.min(y1) <= 0.0 && x0.max(x1) >= 1.0 && y0.max(y1) >= 1.0
        }
        ImageScaleMode::Fit | ImageScaleMode::Tile => false,
    }
}

/// Mapping from image pixels to the node's local box for each scale mode.
pub fn image_transform(img: &ImagePaint, iw: f64, ih: f64, w: f64, h: f64) -> Transform {
    let iw = iw.max(1.0);
    let ih = ih.max(1.0);
    let rot = img.rotation.rem_euclid(4);
    let (rw, rh) = if rot % 2 == 1 { (ih, iw) } else { (iw, ih) };
    // Rotation about the image centre, then translate so the rotated image's box starts at 0.
    let rot_t = Transform::from_translate(-(iw / 2.0) as f32, -(ih / 2.0) as f32)
        .post_rotate(90.0 * rot as f32)
        .post_translate((rw / 2.0) as f32, (rh / 2.0) as f32);
    let fitted = match img.scale_mode {
        ImageScaleMode::Fill => {
            let s = (w / rw).max(h / rh);
            Transform::from_scale(s as f32, s as f32).post_translate(((w - rw * s) / 2.0) as f32, ((h - rh * s) / 2.0) as f32)
        }
        ImageScaleMode::Fit => {
            let s = (w / rw).min(h / rh);
            Transform::from_scale(s as f32, s as f32).post_translate(((w - rw * s) / 2.0) as f32, ((h - rh * s) / 2.0) as f32)
        }
        ImageScaleMode::Tile => {
            let s = if img.scale.is_finite() && img.scale > 0.0 { img.scale } else { 1.0 };
            Transform::from_scale(s as f32, s as f32)
        }
        ImageScaleMode::Crop => {
            // crop = unit box → image uv (a, b, c, d, e, f). Inverse maps uv → unit box.
            let m = img.effective_crop(iw, ih, w, h);
            let uv_to_unit =
                Transform::from_row(m[0] as f32, m[1] as f32, m[2] as f32, m[3] as f32, m[4] as f32, m[5] as f32).invert().unwrap_or_default();
            Transform::from_scale((1.0 / rw) as f32, (1.0 / rh) as f32).post_concat(uv_to_unit).post_scale(w as f32, h as f32)
        }
    };
    rot_t.post_concat(fitted)
}
