//! Brushes: strokes drawn with a shape instead of a band. A stretch brush bends one shape along
//! each subpath; a scatter brush drops copies of a shape along it, spaced by the stroke weight and
//! jittered in size, rotation and distance from the path by a seeded (repeatable) sequence. The
//! shapes are the built-in presets below, drawn in code. A width profile (pen pressure) scales the
//! brush along the path. The result may overlap itself; fill it with the non-zero rule.

use crate::width::{Polyline, polylines};
use figcraft_doc::{Brush, StrokeProps};
use figcraft_geom::kurbo::{self, Affine, BezPath, PathEl, Point, Vec2};

/// What a preset does with its shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushKind {
    Stretch,
    Scatter,
}

/// A built-in brush: id (saved in files), label, kind.
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: BrushKind,
}

/// The built-in brushes, stretch first.
pub const PRESETS: &[Preset] = &[
    Preset { id: "taper", label: "Taper", kind: BrushKind::Stretch },
    Preset { id: "ink", label: "Ink", kind: BrushKind::Stretch },
    Preset { id: "ribbon", label: "Ribbon", kind: BrushKind::Stretch },
    Preset { id: "bristle", label: "Bristle", kind: BrushKind::Stretch },
    Preset { id: "dots", label: "Dots", kind: BrushKind::Scatter },
    Preset { id: "stars", label: "Stars", kind: BrushKind::Scatter },
    Preset { id: "confetti", label: "Confetti", kind: BrushKind::Scatter },
    Preset { id: "leaves", label: "Leaves", kind: BrushKind::Scatter },
];

/// The most copies a scatter brush makes along a whole path; denser spacing spreads them out.
pub const MAX_COPIES: usize = 5_000;
/// Along-path steps of a stretched shape, per subpath.
const STRETCH_STEPS: usize = 512;
/// Along-path steps of a stretched shape over the whole path (many subpaths get fewer each).
const MAX_STRETCH_STEPS: usize = 65_536;

/// The built-in brush `id`.
pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// A preset's shape: stretch shapes span x 0..1 along the path and y -0.5..0.5 across it (one
/// stroke weight); scatter shapes are about one unit wide, centred on the origin, pointing along x.
fn shape(id: &str) -> Option<BezPath> {
    let mut p = BezPath::new();
    match id {
        "taper" => {
            p.move_to((0.0, 0.0));
            p.curve_to((0.3, -0.5), (0.7, -0.5), (1.0, 0.0));
            p.curve_to((0.7, 0.5), (0.3, 0.5), (0.0, 0.0));
            p.close_path();
        }
        // A blunt start, uneven edges and a long tail.
        "ink" => band(&mut p, |x| {
            let w = (x / 0.04).min(1.0) * ((1.0 - x) / 0.3).min(1.0);
            (-0.5 * w + 0.04 * (9.0 * std::f64::consts::PI * x).sin() * w, 0.45 * w + 0.04 * (13.0 * std::f64::consts::PI * x + 1.0).sin() * w)
        }),
        // Edges that cross three times: a twisting ribbon.
        "ribbon" => band(&mut p, |x| {
            let c = 0.5 * (3.0 * std::f64::consts::PI * x).cos();
            (-c, c)
        }),
        // Four thin strands with ragged ends.
        "bristle" => {
            for k in 0..4 {
                let (k, y) = (f64::from(k), -0.375 + 0.25 * f64::from(k));
                let (x0, x1) = (0.03 * k, 1.0 - 0.04 * (3.0 - k));
                let mut s = BezPath::new();
                band(&mut s, |x| {
                    let u = ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
                    let h = 0.09 * (std::f64::consts::PI * u).sin().max(0.0).sqrt();
                    (y - h, y + h)
                });
                p.extend(s);
            }
        }
        "dots" => p.extend(kurbo::Shape::path_elements(&kurbo::Circle::new((0.0, 0.0), 0.5), 0.01)),
        "stars" => {
            let pts = figcraft_geom::star_points(1.0, 1.0, 5, 0.4);
            for (i, q) in pts.iter().enumerate() {
                let q = Point::new(q.x - 0.5, q.y - 0.5);
                if i == 0 { p.move_to(q) } else { p.line_to(q) }
            }
            p.close_path();
        }
        "confetti" => p.extend(kurbo::Shape::path_elements(&kurbo::Rect::new(-0.3, -0.3, 0.3, 0.3), 0.01)),
        "leaves" => {
            p.move_to((-0.5, 0.0));
            p.curve_to((-0.2, -0.3), (0.2, -0.3), (0.5, 0.0));
            p.curve_to((0.2, 0.3), (-0.2, 0.3), (-0.5, 0.0));
            p.close_path();
        }
        _ => return None,
    }
    Some(p)
}

/// A closed band from x 0 to 1 between edges `f(x) = (top, bottom)`.
fn band(p: &mut BezPath, f: impl Fn(f64) -> (f64, f64)) {
    const N: usize = 64;
    let xs = (0..=N).map(|i| i as f64 / N as f64);
    for (i, x) in xs.clone().enumerate() {
        let q = (x, f(x).0);
        if i == 0 { p.move_to(q) } else { p.line_to(q) }
    }
    for x in xs.rev() {
        p.line_to((x, f(x).1));
    }
    p.close_path();
}

/// A repeatable pseudo-random number in 0..1 for copy `i`, channel `k` (splitmix64).
fn rand(seed: u32, i: u64, k: u64) -> f64 {
    let mut z = u64::from(seed) ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k.wrapping_mul(0xD1B5_4A32_D192_ED03);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// The stroke of `path` drawn with `brush` at `width` (the profile, if any, scales it along each
/// subpath). `None` for an unknown preset or a path too long to measure.
pub fn outline(path: &BezPath, width: f64, props: &StrokeProps, brush: &Brush) -> Option<BezPath> {
    let p = preset(&brush.preset)?;
    let unit = shape(p.id)?;
    let lines = polylines(path)?;
    // Sanitized once: at most 64 points, so each sample's symmetry scan stays cheap.
    let profile = props.profile().map(|pr| pr.sanitized());
    let scale = |t: f64| width * profile.as_ref().map_or(1.0, |pr| pr.scale_at(t));
    let mut out = BezPath::new();
    match p.kind {
        BrushKind::Stretch => {
            let steps = (MAX_STRETCH_STEPS / lines.len().max(1)).clamp(8, STRETCH_STEPS);
            for line in &lines {
                stretch(&mut out, &unit, line, steps, &scale);
            }
        }
        BrushKind::Scatter => scatter(&mut out, &unit, &lines, width, brush, &scale),
    }
    Some(out)
}

/// `unit` bent along `line` in at most `steps` steps along it: x by length, y across at the scaled
/// width.
fn stretch(out: &mut BezPath, unit: &BezPath, line: &Polyline, steps: usize, scale: &dyn Fn(f64) -> f64) {
    let map = |q: Point| line.at(q.x).map(|(p, n)| p + n * (q.y * scale(q.x)));
    let mut last = Point::ZERO;
    kurbo::flatten(unit.iter(), 0.002, |el| match el {
        PathEl::MoveTo(q) => {
            last = q;
            if let Some(m) = map(q) {
                out.move_to(m);
            }
        }
        PathEl::LineTo(q) => {
            // Short steps along x so the shape bends with the path.
            let n = (((q.x - last.x).abs() * steps as f64).ceil() as usize).clamp(1, steps);
            for k in 1..=n {
                if let Some(m) = map(last.lerp(q, k as f64 / n as f64)) {
                    out.line_to(m);
                }
            }
            last = q;
        }
        PathEl::ClosePath => out.close_path(),
        _ => {}
    });
}

/// Copies of `unit` along `lines`, `brush.spacing` weights apart, jittered.
fn scatter(out: &mut BezPath, unit: &BezPath, lines: &[Polyline], width: f64, brush: &Brush, scale: &dyn Fn(f64) -> f64) {
    let total: f64 = lines.iter().map(Polyline::length).sum();
    let b = brush.sanitized();
    let step = (b.spacing * width).max(total / MAX_COPIES as f64).max(1e-3);
    let mut i = 0u64;
    for line in lines {
        let len = line.length();
        let mut s = step / 2.0;
        while s < len {
            let t = s / len;
            if let Some((p, n)) = line.at(t) {
                let r = |k| rand(b.seed, i, k) * 2.0 - 1.0;
                let size = scale(t) * (1.0 + b.size_jitter * r(0)).max(0.05);
                let dir = Vec2::new(n.y, -n.x);
                let angle = dir.atan2() + b.rotation_jitter * std::f64::consts::PI * r(1);
                let at = p + n * (b.scatter * width * r(2));
                let xf = Affine::translate(at.to_vec2()) * Affine::rotate(angle) * Affine::scale(size);
                out.extend(unit.iter().map(|el| xf * el));
            }
            i += 1;
            s += step;
        }
    }
}
