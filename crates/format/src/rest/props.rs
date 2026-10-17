//! Property mapping for the REST file JSON: JSON readers, enums, paints, effects, strokes,
//! corners, constraints, auto layout, layout grids and export settings.

use figcraft_doc::color::{BlendMode, Color, ColorStop, Gradient, GradientKind, ImageFilters, ImagePaint, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::geom::Affine;
use figcraft_doc::{
    AutoLayout, Blur, BlurType, CellAlign, ChildLayout, Constraints, ContentAlign, CrossAlign, Effect, EffectKind, ExportFormat, ExportSetting,
    ExportSize, Glass, GridAlign, GridKind, GridTrack, HConstraint, LayoutGrid, LayoutMode, MainAlign, Noise, NoiseMode, Shadow, Sizing, StrokeAlign,
    StrokeCap, StrokeJoin, StrokeProps, Texture, VConstraint, WidthPoint, WidthProfile,
};
use serde_json::Value;

use super::Ctx;

/// Largest width/height/stroke/radius/spacing value taken from the file.
pub(super) const MAX_SIZE: f64 = 1e6;
/// Largest coordinate taken from the file.
pub(super) const MAX_COORD: f64 = 1e7;
/// Most entries read from any one list (paints, effects, dashes, grids…).
const MAX_LIST: usize = 256;

// ---- JSON readers (missing, wrongly typed or non-finite values read as `None`) ----

pub(super) fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k)?.as_f64().filter(|x| x.is_finite())
}

/// A non-negative length, capped.
pub(super) fn len(v: &Value, k: &str) -> Option<f64> {
    num(v, k).map(|x| x.clamp(0.0, MAX_SIZE))
}

pub(super) fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k)?.as_str()
}

pub(super) fn flag(v: &Value, k: &str) -> Option<bool> {
    v.get(k)?.as_bool()
}

pub(super) fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(|a| a.get(..a.len().min(MAX_LIST)).unwrap_or_default()).unwrap_or_default()
}

/// `{x, y}`.
pub(super) fn vec2(v: &Value) -> Option<(f64, f64)> {
    Some((num(v, "x")?, num(v, "y")?))
}

/// `{r, g, b, a}` with components 0..1 (`a` defaults to 1).
pub(super) fn color(v: &Value) -> Option<Color> {
    let c = |k: &str| num(v, k).map(|x| x.clamp(0.0, 1.0) as f32);
    Some(Color::new(c("r")?, c("g")?, c("b")?, c("a").unwrap_or(1.0)))
}

/// A 2×3 matrix `[[a, c, e], [b, d, f]]` (x' = a·x + c·y + e, y' = b·x + d·y + f).
pub(super) fn transform(v: &Value) -> Option<Affine> {
    let rows = v.as_array()?;
    let row = |i: usize| -> Option<[f64; 3]> {
        let r = rows.get(i)?.as_array()?;
        let n = |j: usize| r.get(j)?.as_f64().filter(|x| x.is_finite());
        Some([n(0)?, n(1)?, n(2)?])
    };
    let ([a, c, e], [b, d, f]) = (row(0)?, row(1)?);
    if e.abs() > MAX_COORD || f.abs() > MAX_COORD || [a, b, c, d].iter().any(|x| x.abs() > 1e6) {
        return None;
    }
    Some(Affine::new([a, b, c, d, e, f]))
}

// ---- enums ----

pub(crate) fn blend_mode(s: &str) -> Option<BlendMode> {
    Some(match s {
        "PASS_THROUGH" => BlendMode::PassThrough,
        "NORMAL" => BlendMode::Normal,
        "DARKEN" => BlendMode::Darken,
        "MULTIPLY" => BlendMode::Multiply,
        "LINEAR_BURN" => BlendMode::PlusDarker,
        "COLOR_BURN" => BlendMode::ColorBurn,
        "LIGHTEN" => BlendMode::Lighten,
        "SCREEN" => BlendMode::Screen,
        "LINEAR_DODGE" => BlendMode::PlusLighter,
        "COLOR_DODGE" => BlendMode::ColorDodge,
        "OVERLAY" => BlendMode::Overlay,
        "SOFT_LIGHT" => BlendMode::SoftLight,
        "HARD_LIGHT" => BlendMode::HardLight,
        "DIFFERENCE" => BlendMode::Difference,
        "EXCLUSION" => BlendMode::Exclusion,
        "HUE" => BlendMode::Hue,
        "SATURATION" => BlendMode::Saturation,
        "COLOR" => BlendMode::Color,
        "LUMINOSITY" => BlendMode::Luminosity,
        _ => return None,
    })
}

/// A paint's or effect's blend mode (pass-through only applies to layers).
fn paint_blend(v: &Value) -> BlendMode {
    match text(v, "blendMode").and_then(blend_mode) {
        Some(BlendMode::PassThrough) | None => BlendMode::Normal,
        Some(b) => b,
    }
}

fn sizing(s: &str) -> Option<Sizing> {
    Some(match s {
        "FIXED" => Sizing::Fixed,
        "HUG" => Sizing::Hug,
        "FILL" => Sizing::Fill,
        _ => return None,
    })
}

// ---- paints ----

/// A `fills` / `strokes` list.
pub(super) fn paints(cx: &mut Ctx, v: &Value, k: &str) -> Option<Vec<Paint>> {
    let a = v.get(k)?.as_array()?;
    Some(a.iter().take(MAX_LIST).filter_map(|p| paint(cx, p)).collect())
}

fn paint(cx: &mut Ctx, p: &Value) -> Option<Paint> {
    let ty = text(p, "type")?;
    let mut opacity = num(p, "opacity").unwrap_or(1.0).clamp(0.0, 1.0) as f32;
    let kind = match ty {
        "SOLID" => {
            let c = p.get("color").and_then(color).unwrap_or(Color::BLACK);
            opacity *= c.a;
            PaintKind::Solid { color: c.with_alpha(1.0) }
        }
        "GRADIENT_LINEAR" => PaintKind::Gradient(gradient(GradientKind::Linear, p)),
        "GRADIENT_RADIAL" => PaintKind::Gradient(gradient(GradientKind::Radial, p)),
        "GRADIENT_ANGULAR" => PaintKind::Gradient(gradient(GradientKind::Angular, p)),
        "GRADIENT_DIAMOND" => PaintKind::Gradient(gradient(GradientKind::Diamond, p)),
        "IMAGE" => PaintKind::Image(image(cx, p)),
        other => {
            cx.warn(format!("{} paints are not supported and were skipped", clip_str(other)));
            return None;
        }
    };
    Some(Paint { kind, opacity, visible: flag(p, "visible").unwrap_or(true), blend: paint_blend(p) })
}

/// Gradient handles are already in the node's unit box (start, end, width), like ours.
fn gradient(kind: GradientKind, p: &Value) -> Gradient {
    let mut g = Gradient::default_for(kind, Color::BLACK);
    let h: Vec<(f64, f64)> =
        list(p, "gradientHandlePositions").iter().filter_map(vec2).map(|(x, y)| (x.clamp(-1e3, 1e3), y.clamp(-1e3, 1e3))).collect();
    if let [from, to, rest @ ..] = h.as_slice() {
        g.from = *from;
        g.to = *to;
        // Without the width handle: perpendicular to the main axis, same length.
        g.width = rest.first().copied().unwrap_or((from.0 - (to.1 - from.1), from.1 + (to.0 - from.0)));
    }
    let stops: Vec<ColorStop> = list(p, "gradientStops")
        .iter()
        .filter_map(|s| Some(ColorStop { position: num(s, "position")?.clamp(0.0, 1.0) as f32, color: s.get("color").and_then(color)? }))
        .collect();
    if !stops.is_empty() {
        g.stops = stops;
    }
    g
}

fn image(cx: &mut Ctx, p: &Value) -> ImagePaint {
    let scale_mode = match text(p, "scaleMode") {
        Some("FIT") => ImageScaleMode::Fit,
        Some("TILE") => ImageScaleMode::Tile,
        Some("STRETCH") => ImageScaleMode::Crop,
        _ => ImageScaleMode::Fill,
    };
    // STRETCH: the image transform maps the unit box to image space, like our crop matrix.
    let crop = (scale_mode == ImageScaleMode::Crop)
        .then(|| p.get("imageTransform").and_then(transform))
        .flatten()
        .map(|t| t.as_coeffs())
        .filter(|c| c[0].abs() > 1e-9 && c[3].abs() > 1e-9);
    let f = p.get("filters");
    let filter = |k: &str| f.and_then(|f| num(f, k)).unwrap_or(0.0).clamp(-1.0, 1.0) as f32;
    let filter_fields = ["exposure", "contrast", "saturation", "temperature", "tint", "highlights", "shadows"];
    let mut has_filters = false;
    if let Some(filters) = f.and_then(Value::as_object) {
        for (key, value) in filters {
            match value.as_f64().filter(|value| value.is_finite()) {
                Some(value) if filter_fields.contains(&key.as_str()) => has_filters |= value != 0.0,
                Some(0.0) => {}
                Some(_) => cx.warn(format!("nonzero unsupported image filter `{}` was skipped", clip_str(key))),
                None => cx.warn(format!("malformed image filter `{}` was skipped", clip_str(key))),
            }
        }
    } else if f.is_some_and(|filters| !filters.is_null()) {
        cx.warn("malformed image filters container was skipped");
    }
    let fig = cx.fig_sources.is_some();
    if has_filters && fig {
        cx.warn("image filters use FigCraft's approximate renderer");
    }
    ImagePaint {
        image: cx.image_key(text(p, "imageRef").unwrap_or("")),
        scale_mode,
        scale: num(p, "scalingFactor").filter(|s| *s > 0.0).map_or(1.0, |s| s.min(1e3)),
        rotation: (num(p, "rotation").unwrap_or(0.0).clamp(-1e6, 1e6) / 90.0).round().rem_euclid(4.0) as i32,
        crop,
        filters: ImageFilters {
            exposure: filter("exposure"),
            contrast: filter("contrast"),
            saturation: filter("saturation"),
            temperature: filter("temperature"),
            // The files' tint runs from green (negative) to magenta (positive), in REST file JSON and
            // .fig alike; a positive native tint adds green. (`0 - x`: no negative zero.)
            tint: 0.0 - filter("tint"),
            highlights: filter("highlights"),
            shadows: filter("shadows"),
        },
    }
}

// ---- effects ----

pub(super) fn effects(cx: &mut Ctx, v: &Value) -> Vec<Effect> {
    list(v, "effects").iter().filter_map(|e| effect(cx, e)).collect()
}

fn effect(cx: &mut Ctx, e: &Value) -> Option<Effect> {
    let radius = len(e, "radius").unwrap_or(0.0);
    let shadow = || Shadow {
        color: e.get("color").and_then(color).unwrap_or(Color::new(0.0, 0.0, 0.0, 0.25)),
        offset: e.get("offset").and_then(vec2).map_or((0.0, 0.0), |(x, y)| (x.clamp(-MAX_SIZE, MAX_SIZE), y.clamp(-MAX_SIZE, MAX_SIZE))),
        radius,
        spread: num(e, "spread").unwrap_or(0.0).clamp(-MAX_SIZE, MAX_SIZE),
        blend: paint_blend(e),
        show_behind: flag(e, "showShadowBehindNode").unwrap_or(false),
    };
    let kind = match text(e, "type")? {
        "DROP_SHADOW" => EffectKind::DropShadow(shadow()),
        "INNER_SHADOW" => EffectKind::InnerShadow(shadow()),
        "LAYER_BLUR" => EffectKind::LayerBlur(blur(e, radius)),
        "BACKGROUND_BLUR" => EffectKind::BackgroundBlur(blur(e, radius)),
        "NOISE" => {
            let d = Noise::default();
            let (size, size_y) = noise_size(e, d.size);
            EffectKind::Noise(Noise {
                mode: match text(e, "noiseType") {
                    Some("DUOTONE") => NoiseMode::Duo,
                    Some("MULTITONE") => NoiseMode::Multi,
                    _ => NoiseMode::Mono,
                },
                size,
                size_y,
                density: num(e, "density").map_or(d.density, |v| v.clamp(0.0, 1.0)),
                color: e.get("color").and_then(color).unwrap_or(d.color),
                secondary: e.get("secondaryColor").and_then(color).unwrap_or(d.secondary),
                opacity: num(e, "opacity").map_or(d.opacity, |v| v.clamp(0.0, 1.0) as f32),
                blend: paint_blend(e),
            })
        }
        "TEXTURE" => {
            let d = Texture::default();
            let (size, size_y) = noise_size(e, d.size);
            EffectKind::Texture(Texture {
                size,
                size_y,
                radius: len(e, "radius").unwrap_or(d.radius),
                clip_to_shape: flag(e, "clipToShape").unwrap_or(false),
            })
        }
        "GLASS" => {
            let d = Glass::default();
            EffectKind::Glass(
                Glass {
                    light_angle: num(e, "lightAngle").unwrap_or(d.light_angle),
                    light_intensity: num(e, "lightIntensity").unwrap_or(d.light_intensity),
                    refraction: num(e, "refraction").unwrap_or(d.refraction),
                    depth: num(e, "depth").unwrap_or(d.depth),
                    dispersion: num(e, "dispersion").unwrap_or(d.dispersion),
                    frost: len(e, "radius").unwrap_or(d.frost),
                    splay: num(e, "splay").unwrap_or(d.splay),
                }
                .sanitized(),
            )
        }
        other => {
            cx.warn(format!("{} effects are not supported and were skipped", clip_str(other)));
            return None;
        }
    };
    Some(Effect { kind, visible: flag(e, "visible").unwrap_or(true) })
}

/// A layer or background blur of `radius`: progressive (`blurType` `PROGRESSIVE`) from
/// `startRadius` at `startOffset` to `radius` at `endOffset`, else uniform.
fn blur(e: &Value, radius: f64) -> Blur {
    let d = Blur::default();
    let blur_type = if text(e, "blurType") == Some("PROGRESSIVE") { BlurType::Progressive } else { BlurType::Uniform };
    Blur {
        radius,
        blur_type,
        start_radius: len(e, "startRadius").unwrap_or(d.start_radius),
        start: e.get("startOffset").and_then(vec2).unwrap_or(d.start),
        end: e.get("endOffset").and_then(vec2).unwrap_or(d.end),
    }
    .sanitized()
}

/// A noise or texture effect's grain size: `noiseSize` on both axes, or `noiseSizeVector`.
fn noise_size(e: &Value, default: f64) -> (f64, Option<f64>) {
    let size = len(e, "noiseSize").unwrap_or(default);
    match e.get("noiseSizeVector").and_then(vec2) {
        Some((x, y)) if x != y => (x.clamp(0.0, MAX_SIZE), Some(y.clamp(0.0, MAX_SIZE))),
        Some((x, _)) => (x.clamp(0.0, MAX_SIZE), None),
        None => (size, None),
    }
}

// ---- strokes, corners, constraints ----

pub(super) fn stroke_props(v: &Value, sp: &mut StrokeProps) {
    if let Some(w) = len(v, "strokeWeight") {
        sp.weight = w;
    }
    if let Some(s) = v.get("individualStrokeWeights") {
        let side = |k: &str| len(s, k).unwrap_or(sp.weight);
        sp.sides = Some([side("top"), side("right"), side("bottom"), side("left")]);
    }
    match text(v, "strokeAlign") {
        Some("INSIDE") => sp.align = StrokeAlign::Inside,
        Some("CENTER") => sp.align = StrokeAlign::Center,
        Some("OUTSIDE") => sp.align = StrokeAlign::Outside,
        _ => {}
    }
    let cap = |k: &str| {
        text(v, k).map(|c| match c {
            "ROUND" => StrokeCap::Round,
            "SQUARE" => StrokeCap::Square,
            "LINE_ARROW" => StrokeCap::LineArrow,
            "TRIANGLE_ARROW" | "TRIANGLE_FILLED" => StrokeCap::TriangleArrow,
            "DIAMOND_FILLED" => StrokeCap::DiamondFilled,
            "CIRCLE_FILLED" => StrokeCap::CircleFilled,
            _ => StrokeCap::None,
        })
    };
    if let Some(c) = cap("strokeCap") {
        sp.cap_start = c;
        sp.cap_end = c;
    }
    // Each end on its own (our extension, written by the `.fig` import).
    if let Some(c) = cap("strokeCapStart") {
        sp.cap_start = c;
    }
    if let Some(c) = cap("strokeCapEnd") {
        sp.cap_end = c;
    }
    match text(v, "strokeJoin") {
        Some("MITER") => sp.join = StrokeJoin::Miter,
        Some("BEVEL") => sp.join = StrokeJoin::Bevel,
        Some("ROUND") => sp.join = StrokeJoin::Round,
        _ => {}
    }
    if v.get("strokeDashes").is_some() {
        sp.dashes = list(v, "strokeDashes").iter().filter_map(Value::as_f64).filter(|d| d.is_finite()).map(|d| d.clamp(0.0, MAX_SIZE)).collect();
        if sp.dashes.iter().all(|d| *d <= 0.0) {
            sp.dashes.clear();
        }
    }
    // A variable width: `[t, scale, bias]` points (our extension, written by the `.fig` import).
    let point =
        |p: &Value| Some(WidthPoint { t: p.get(0)?.as_f64()?, scale: p.get(1)?.as_f64()?, bias: p.get(2).and_then(Value::as_f64).unwrap_or(0.0) });
    let points: Vec<WidthPoint> = list(v, "strokeWidthProfile").iter().take(WidthProfile::MAX_POINTS).filter_map(point).collect();
    if !points.is_empty() {
        sp.width_profile = Some(WidthProfile { points }.sanitized());
    }
    // The miter angle (degrees) below which joins bevel → the equivalent miter limit.
    if let Some(a) = num(v, "strokeMiterAngle").filter(|a| *a > 0.0 && *a < 180.0) {
        sp.miter_limit = (1.0 / (a.to_radians() / 2.0).sin()).clamp(1.0, 100.0);
    }
}

/// `[tl, tr, br, bl]` radii and smoothing, when the node sets them.
pub(super) fn corners(v: &Value, radii: &mut [f64; 4], smoothing: &mut f64) {
    if let Some(r) = len(v, "cornerRadius") {
        *radii = [r; 4];
    }
    let each: Vec<f64> =
        list(v, "rectangleCornerRadii").iter().filter_map(Value::as_f64).filter(|r| r.is_finite()).map(|r| r.clamp(0.0, MAX_SIZE)).collect();
    if let [a, b, c, d] = each.as_slice() {
        *radii = [*a, *b, *c, *d];
    }
    if let Some(s) = num(v, "cornerSmoothing") {
        *smoothing = s.clamp(0.0, 1.0);
    }
}

pub(super) fn constraints(v: &Value) -> Option<Constraints> {
    let c = v.get("constraints")?;
    Some(Constraints {
        horizontal: match text(c, "horizontal") {
            Some("RIGHT") => HConstraint::Right,
            Some("LEFT_RIGHT") => HConstraint::LeftRight,
            Some("CENTER") => HConstraint::Center,
            Some("SCALE") => HConstraint::Scale,
            _ => HConstraint::Left,
        },
        vertical: match text(c, "vertical") {
            Some("BOTTOM") => VConstraint::Bottom,
            Some("TOP_BOTTOM") => VConstraint::TopBottom,
            Some("CENTER") => VConstraint::Center,
            Some("SCALE") => VConstraint::Scale,
            _ => VConstraint::Top,
        },
    })
}

// ---- auto layout ----

/// The auto layout of a frame-like node (`layoutMode` other than `NONE`).
pub(super) fn auto_layout(v: &Value, fig_tracks: bool) -> Option<AutoLayout> {
    let mode = match text(v, "layoutMode")? {
        "HORIZONTAL" => LayoutMode::Horizontal,
        "VERTICAL" => LayoutMode::Vertical,
        "GRID" => LayoutMode::Grid,
        _ => return None,
    };
    let pad = |k: &str| len(v, k).unwrap_or(0.0);
    let count = |k: &str, d: u32| num(v, k).filter(|n| *n >= 1.0).map_or(d, |n| n.min(1000.0) as u32);
    let grid = mode == LayoutMode::Grid;
    Some(AutoLayout {
        mode,
        wrap: text(v, "layoutWrap") == Some("WRAP"),
        gap: if grid { len(v, "gridColumnGap") } else { num(v, "itemSpacing").map(|g| g.clamp(-MAX_SIZE, MAX_SIZE)) }.unwrap_or(0.0),
        cross_gap: if grid { len(v, "gridRowGap") } else { len(v, "counterAxisSpacing") }.unwrap_or(0.0),
        padding: [pad("paddingTop"), pad("paddingRight"), pad("paddingBottom"), pad("paddingLeft")],
        main_align: match text(v, "primaryAxisAlignItems") {
            Some("CENTER") => MainAlign::Center,
            Some("MAX") => MainAlign::Max,
            Some("SPACE_BETWEEN") => MainAlign::SpaceBetween,
            Some("SPACE_AROUND") => MainAlign::SpaceAround,
            Some("SPACE_EVENLY") => MainAlign::SpaceEvenly,
            _ => MainAlign::Min,
        },
        cross_align: match text(v, "counterAxisAlignItems") {
            Some("CENTER") => CrossAlign::Center,
            Some("MAX") => CrossAlign::Max,
            Some("BASELINE") => CrossAlign::Baseline,
            _ => CrossAlign::Min,
        },
        content_align: match text(v, "counterAxisAlignContent") {
            Some("SPACE_BETWEEN") => ContentAlign::SpaceBetween,
            _ => ContentAlign::Auto,
        },
        grid_columns: count("gridColumnCount", 2),
        grid_rows: count("gridRowCount", 2),
        strokes_included: flag(v, "strokesIncludedInLayout").unwrap_or(false),
        reverse_z: flag(v, "itemReverseZIndex").unwrap_or(false),
        column_tracks: if fig_tracks && grid { fig_grid_tracks(v, "columnTracks") } else { Vec::new() },
        row_tracks: if fig_tracks && grid { fig_grid_tracks(v, "rowTracks") } else { Vec::new() },
    })
}

fn fig_grid_tracks(v: &Value, axis: &str) -> Vec<GridTrack> {
    v.get(crate::rest::FIG_SOURCE_FIELD)
        .and_then(|source| source.get("gridTracks"))
        .and_then(|tracks| tracks.get(axis))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(1_000)
        .map(|track| {
            let value = track.get("value").and_then(Value::as_f64).filter(|n| n.is_finite() && (0.0..=MAX_SIZE).contains(n));
            match (track.get("sizing").and_then(Value::as_str), value) {
                (Some("fixed"), Some(px)) => GridTrack::fixed(px),
                (Some("fill"), Some(fr)) if fr > 0.0 => GridTrack::fill(fr),
                (Some("hug"), _) => GridTrack::hug(),
                _ => GridTrack::default(),
            }
        })
        .collect()
}

/// How the node sizes and sits in its parent: its own hug/fixed axes (auto layout frames), its
/// fill/fixed axes as a child of an auto layout `parent_mode`, absolute positioning, min/max sizes
/// and grid cells.
pub(super) fn child_layout(v: &Value, own: Option<LayoutMode>, parent_mode: Option<LayoutMode>) -> ChildLayout {
    let mut cl = ChildLayout::default();
    let horizontal_main = |m: LayoutMode| m != LayoutMode::Vertical;
    // Legacy fields first, the explicit per-axis sizing wins.
    // A text's resize mode is its sizing: auto width hugs both axes, auto height its height.
    if text(v, "type") == Some("TEXT") && v.get("style").and_then(|s| text(s, "textTruncation")) != Some("ENDING") {
        match v.get("style").and_then(|s| text(s, "textAutoResize")) {
            Some("WIDTH_AND_HEIGHT") => (cl.sizing_h, cl.sizing_v) = (Sizing::Hug, Sizing::Hug),
            Some("HEIGHT") => cl.sizing_v = Sizing::Hug,
            _ => {}
        }
    }
    if let Some(m) = own {
        // Both default to AUTO (hug) when the file leaves them out.
        let hug = |k: &str| text(v, k) != Some("FIXED");
        let (main, cross) = (hug("primaryAxisSizingMode"), hug("counterAxisSizingMode"));
        let (h, w) = if horizontal_main(m) { (main, cross) } else { (cross, main) };
        if h {
            cl.sizing_h = Sizing::Hug;
        }
        if w {
            cl.sizing_v = Sizing::Hug;
        }
    }
    if let Some(m) = parent_mode {
        let grow = num(v, "layoutGrow").is_some_and(|g| g > 0.0);
        let stretch = text(v, "layoutAlign") == Some("STRETCH");
        let (fill_h, fill_v) = if horizontal_main(m) { (grow, stretch) } else { (stretch, grow) };
        if fill_h {
            cl.sizing_h = Sizing::Fill;
        }
        if fill_v {
            cl.sizing_v = Sizing::Fill;
        }
    }
    if let Some(s) = text(v, "layoutSizingHorizontal").and_then(sizing) {
        cl.sizing_h = s;
    }
    if let Some(s) = text(v, "layoutSizingVertical").and_then(sizing) {
        cl.sizing_v = s;
    }
    cl.absolute = text(v, "layoutPositioning") == Some("ABSOLUTE");
    cl.min_w = len(v, "minWidth");
    cl.max_w = len(v, "maxWidth");
    cl.min_h = len(v, "minHeight");
    cl.max_h = len(v, "maxHeight");
    let index = |k: &str| num(v, k).filter(|n| *n >= 0.0).map(|n| n.min(1000.0) as u32);
    cl.grid_column = index("gridColumnAnchorIndex");
    cl.grid_row = index("gridRowAnchorIndex");
    cl.grid_column_span = index("gridColumnSpan").unwrap_or(0);
    cl.grid_row_span = index("gridRowSpan").unwrap_or(0);
    let align = |k: &str| match text(v, k) {
        Some("MIN") => Some(CellAlign::Min),
        Some("CENTER") => Some(CellAlign::Center),
        Some("MAX") => Some(CellAlign::Max),
        _ => None,
    };
    cl.grid_align_h = align("gridChildHorizontalAlign");
    cl.grid_align_v = align("gridChildVerticalAlign");
    cl
}

// ---- layout grids and export settings ----

pub(super) fn layout_grids(v: &Value) -> Vec<LayoutGrid> {
    list(v, "layoutGrids")
        .iter()
        .filter_map(|g| {
            let kind = match text(g, "pattern")? {
                "COLUMNS" => GridKind::Columns,
                "ROWS" => GridKind::Rows,
                "GRID" => GridKind::Grid,
                _ => return None,
            };
            let base = match kind {
                GridKind::Columns => LayoutGrid::columns(),
                GridKind::Rows => LayoutGrid::rows(),
                GridKind::Grid => LayoutGrid::default(),
            };
            Some(LayoutGrid {
                kind,
                size: len(g, "sectionSize").unwrap_or(base.size),
                // Auto-fit counts come through as 0, negative or huge.
                count: num(g, "count").filter(|c| *c >= 1.0 && *c <= 1000.0).map_or(0, |c| c as u32),
                gutter: len(g, "gutterSize").unwrap_or(base.gutter),
                offset: len(g, "offset").unwrap_or(0.0),
                align: match text(g, "alignment") {
                    Some("MIN") => GridAlign::Min,
                    Some("CENTER") => GridAlign::Center,
                    Some("MAX") => GridAlign::Max,
                    _ => GridAlign::Stretch,
                },
                color: g.get("color").and_then(color).unwrap_or(base.color),
                visible: flag(g, "visible").unwrap_or(true),
            })
        })
        .collect()
}

pub(super) fn export_settings(v: &Value) -> Vec<ExportSetting> {
    list(v, "exportSettings")
        .iter()
        .map(|e| {
            let c = e.get("constraint");
            let value = c.and_then(|c| num(c, "value")).filter(|x| *x > 0.0).map_or(1.0, |x| x.min(MAX_SIZE));
            ExportSetting {
                size: match c.and_then(|c| text(c, "type")) {
                    Some("WIDTH") => ExportSize::Width { value },
                    Some("HEIGHT") => ExportSize::Height { value },
                    _ => ExportSize::Scale { value: value.min(64.0) },
                },
                suffix: clip_str(text(e, "suffix").unwrap_or("")).to_string(),
                format: text(e, "format").and_then(ExportFormat::from_ext).unwrap_or_default(),
                ..Default::default()
            }
        })
        .collect()
}

/// At most 200 chars of `s` (names and labels from the file stay readable in warnings).
pub(crate) fn clip_str(s: &str) -> &str {
    s.char_indices().nth(200).and_then(|(i, _)| s.get(..i)).unwrap_or(s)
}
