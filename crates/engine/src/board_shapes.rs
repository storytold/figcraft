//! The board's shape library: the usual flowchart and diagram shapes, drawn as vectors that fill
//! their box (FigCraft's own outlines, from the standard flowchart vocabulary).
//!
//! A library shape is a vector layer whose `plugin_data["craftjam.shape"]` names its preset, so the
//! context toolbar shows it and `shape.setType` can turn it into another one at the same size,
//! keeping its text and connectors.

use figcraft_doc::{Node, NodeKind, VectorNetwork};
use figcraft_geom::{BezPath, Point};

/// The plugin data key naming a layer's library shape.
pub const KEY: &str = "craftjam.shape";

/// A library shape: its id (commands), name (UI) and group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub group: Group,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Basic,
    Flowchart,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Basic => "Basic",
            Group::Flowchart => "Flowchart",
        }
    }
}

const fn p(id: &'static str, label: &'static str, group: Group) -> Preset {
    Preset { id, label, group }
}

/// Every library shape drawn as a vector (the rectangle, ellipse, diamond, triangle and star are
/// the editor's own kinds: see `cmd::board::kind_of`).
pub const PRESETS: &[Preset] = &[
    p("roundedRectangle", "Rounded rectangle", Group::Basic),
    p("triangleDown", "Downward triangle", Group::Basic),
    p("pentagon", "Pentagon", Group::Basic),
    p("hexagon", "Hexagon", Group::Basic),
    p("octagon", "Octagon", Group::Basic),
    p("plus", "Plus", Group::Basic),
    p("arrowRight", "Right arrow", Group::Basic),
    p("arrowLeft", "Left arrow", Group::Basic),
    p("chevron", "Chevron", Group::Basic),
    p("speechBubble", "Speech bubble", Group::Basic),
    p("parallelogram", "Data (parallelogram)", Group::Flowchart),
    p("parallelogramLeft", "Left parallelogram", Group::Flowchart),
    p("cylinder", "Database (cylinder)", Group::Flowchart),
    p("cylinderHorizontal", "Direct access storage", Group::Flowchart),
    p("document", "Document", Group::Flowchart),
    p("documents", "Multiple documents", Group::Flowchart),
    p("predefinedProcess", "Predefined process", Group::Flowchart),
    p("manualInput", "Manual input", Group::Flowchart),
    p("trapezoid", "Manual operation", Group::Flowchart),
    p("preparation", "Preparation", Group::Flowchart),
    p("internalStorage", "Internal storage", Group::Flowchart),
    p("delay", "Delay", Group::Flowchart),
    p("display", "Display", Group::Flowchart),
    p("offPage", "Off-page reference", Group::Flowchart),
    p("or", "Or", Group::Flowchart),
    p("summingJunction", "Summing junction", Group::Flowchart),
];

pub fn preset(id: &str) -> Option<Preset> {
    PRESETS.iter().copied().find(|p| p.id.eq_ignore_ascii_case(id))
}

/// The library shape `n` is, if it is one.
pub fn preset_of(n: &Node) -> Option<Preset> {
    matches!(n.kind, NodeKind::Vector { .. }).then(|| n.plugin_data.get(KEY).and_then(|id| preset(id))).flatten()
}

/// A polygon through `pts` (unit coordinates) in a `w`×`h` box.
fn poly(path: &mut BezPath, pts: &[(f64, f64)], w: f64, h: f64) {
    for (i, (x, y)) in pts.iter().enumerate() {
        let q = Point::new(x * w, y * h);
        if i == 0 { path.move_to(q) } else { path.line_to(q) }
    }
    path.close_path();
}

/// A regular polygon of `n` points, flat at the bottom, filling a `w`×`h` box.
fn regular(path: &mut BezPath, n: usize, w: f64, h: f64, start: f64) {
    let pts: Vec<(f64, f64)> = (0..n)
        .map(|k| {
            let a = start + std::f64::consts::TAU * k as f64 / n as f64;
            (0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin())
        })
        .collect();
    // Stretch to the box.
    let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), (x, _)| (a.min(*x), b.max(*x)));
    let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), (_, y)| (a.min(*y), b.max(*y)));
    let norm: Vec<(f64, f64)> = pts.iter().map(|(x, y)| ((x - x0) / (x1 - x0).max(1e-9), (y - y0) / (y1 - y0).max(1e-9))).collect();
    poly(path, &norm, w, h);
}

/// Half an ellipse's handle length for a quarter arc (as a fraction of its radius).
const K: f64 = 0.552_284_75;

/// `preset`'s outline in a `w`×`h` box (from its top-left), or `None` for an unknown preset.
pub fn path(preset: &str, w: f64, h: f64) -> Option<BezPath> {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let mut b = BezPath::new();
    let pt = |x: f64, y: f64| Point::new(x * w, y * h);
    match preset {
        "roundedRectangle" => {
            let r = (h / 2.0).min(w / 2.0);
            let k = r * K;
            b.move_to((r, 0.0));
            b.line_to((w - r, 0.0));
            b.curve_to((w - r + k, 0.0), (w, r - k), (w, r));
            b.line_to((w, h - r));
            b.curve_to((w, h - r + k), (w - r + k, h), (w - r, h));
            b.line_to((r, h));
            b.curve_to((r - k, h), (0.0, h - r + k), (0.0, h - r));
            b.line_to((0.0, r));
            b.curve_to((0.0, r - k), (r - k, 0.0), (r, 0.0));
            b.close_path();
        }
        "triangleDown" => poly(&mut b, &[(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)], w, h),
        "pentagon" => regular(&mut b, 5, w, h, -std::f64::consts::FRAC_PI_2),
        "hexagon" => poly(&mut b, &[(0.25, 0.0), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.25, 1.0), (0.0, 0.5)], w, h),
        "octagon" => regular(&mut b, 8, w, h, std::f64::consts::PI / 8.0),
        "plus" => poly(
            &mut b,
            &[
                (0.35, 0.0),
                (0.65, 0.0),
                (0.65, 0.35),
                (1.0, 0.35),
                (1.0, 0.65),
                (0.65, 0.65),
                (0.65, 1.0),
                (0.35, 1.0),
                (0.35, 0.65),
                (0.0, 0.65),
                (0.0, 0.35),
                (0.35, 0.35),
            ],
            w,
            h,
        ),
        "arrowRight" => poly(&mut b, &[(0.0, 0.25), (0.6, 0.25), (0.6, 0.0), (1.0, 0.5), (0.6, 1.0), (0.6, 0.75), (0.0, 0.75)], w, h),
        "arrowLeft" => poly(&mut b, &[(1.0, 0.25), (0.4, 0.25), (0.4, 0.0), (0.0, 0.5), (0.4, 1.0), (0.4, 0.75), (1.0, 0.75)], w, h),
        "chevron" => poly(&mut b, &[(0.0, 0.0), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.0, 1.0), (0.25, 0.5)], w, h),
        "speechBubble" => {
            poly(&mut b, &[(0.0, 0.0), (1.0, 0.0), (1.0, 0.75), (0.45, 0.75), (0.25, 1.0), (0.25, 0.75), (0.0, 0.75)], w, h);
        }
        "parallelogram" => poly(&mut b, &[(0.2, 0.0), (1.0, 0.0), (0.8, 1.0), (0.0, 1.0)], w, h),
        "parallelogramLeft" => poly(&mut b, &[(0.0, 0.0), (0.8, 0.0), (1.0, 1.0), (0.2, 1.0)], w, h),
        "trapezoid" => poly(&mut b, &[(0.0, 0.0), (1.0, 0.0), (0.8, 1.0), (0.2, 1.0)], w, h),
        "manualInput" => poly(&mut b, &[(0.0, 0.3), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], w, h),
        "preparation" => poly(&mut b, &[(0.15, 0.0), (0.85, 0.0), (1.0, 0.5), (0.85, 1.0), (0.15, 1.0), (0.0, 0.5)], w, h),
        "offPage" => poly(&mut b, &[(0.0, 0.0), (1.0, 0.0), (1.0, 0.7), (0.5, 1.0), (0.0, 0.7)], w, h),
        "cylinder" => {
            // A body with an elliptical top (its front rim drawn across) and bottom.
            let e = (h * 0.12).min(w * 0.25);
            let k = K;
            b.move_to((0.0, e));
            b.curve_to((0.0, e - e * k), (w / 2.0 - w / 2.0 * k, 0.0), (w / 2.0, 0.0));
            b.curve_to((w / 2.0 + w / 2.0 * k, 0.0), (w, e - e * k), (w, e));
            b.line_to((w, h - e));
            b.curve_to((w, h - e + e * k), (w / 2.0 + w / 2.0 * k, h), (w / 2.0, h));
            b.curve_to((w / 2.0 - w / 2.0 * k, h), (0.0, h - e + e * k), (0.0, h - e));
            b.close_path();
            b.move_to((0.0, e));
            b.curve_to((0.0, e + e * k), (w / 2.0 - w / 2.0 * k, 2.0 * e), (w / 2.0, 2.0 * e));
            b.curve_to((w / 2.0 + w / 2.0 * k, 2.0 * e), (w, e + e * k), (w, e));
        }
        "cylinderHorizontal" => {
            let e = (w * 0.12).min(h * 0.25);
            let k = K;
            b.move_to((e, 0.0));
            b.line_to((w - e, 0.0));
            b.curve_to((w - e + e * k, 0.0), (w, h / 2.0 - h / 2.0 * k), (w, h / 2.0));
            b.curve_to((w, h / 2.0 + h / 2.0 * k), (w - e + e * k, h), (w - e, h));
            b.line_to((e, h));
            b.curve_to((e - e * k, h), (0.0, h / 2.0 + h / 2.0 * k), (0.0, h / 2.0));
            b.curve_to((0.0, h / 2.0 - h / 2.0 * k), (e - e * k, 0.0), (e, 0.0));
            b.close_path();
            b.move_to((w - e, 0.0));
            b.curve_to((w - e - e * k, 0.0), (w - 2.0 * e, h / 2.0 - h / 2.0 * k), (w - 2.0 * e, h / 2.0));
            b.curve_to((w - 2.0 * e, h / 2.0 + h / 2.0 * k), (w - e - e * k, h), (w - e, h));
        }
        "document" => {
            // A sheet whose bottom edge waves once.
            b.move_to(pt(0.0, 0.0));
            b.line_to(pt(1.0, 0.0));
            b.line_to(pt(1.0, 0.85));
            b.curve_to(pt(0.75, 0.7), pt(0.5, 1.05), pt(0.25, 0.95));
            b.curve_to(pt(0.12, 0.9), pt(0.05, 0.86), pt(0.0, 0.88));
            b.close_path();
        }
        "documents" => {
            // Two sheets peeking out behind a document.
            poly(&mut b, &[(0.1, 0.0), (1.0, 0.0), (1.0, 0.75), (0.95, 0.75), (0.95, 0.06), (0.1, 0.06)], w, h);
            poly(&mut b, &[(0.05, 0.06), (0.95, 0.06), (0.95, 0.8), (0.9, 0.8), (0.9, 0.12), (0.05, 0.12)], w, h);
            b.move_to(pt(0.0, 0.12));
            b.line_to(pt(0.9, 0.12));
            b.line_to(pt(0.9, 0.88));
            b.curve_to(pt(0.68, 0.75), pt(0.45, 1.05), pt(0.22, 0.95));
            b.curve_to(pt(0.1, 0.9), pt(0.04, 0.88), pt(0.0, 0.9));
            b.close_path();
        }
        "predefinedProcess" => {
            poly(&mut b, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], w, h);
            for x in [0.1, 0.9] {
                b.move_to(pt(x, 0.0));
                b.line_to(pt(x, 1.0));
            }
        }
        "internalStorage" => {
            poly(&mut b, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], w, h);
            b.move_to(pt(0.15, 0.0));
            b.line_to(pt(0.15, 1.0));
            b.move_to(pt(0.0, 0.2));
            b.line_to(pt(1.0, 0.2));
        }
        "delay" => {
            let r = (h / 2.0).min(w / 2.0);
            let k = r * K;
            b.move_to((0.0, 0.0));
            b.line_to((w - r, 0.0));
            b.curve_to((w - r + k, 0.0), (w, r - k), (w, r));
            b.line_to((w, h - r));
            b.curve_to((w, h - r + k), (w - r + k, h), (w - r, h));
            b.line_to((0.0, h));
            b.close_path();
        }
        "display" => {
            let r = (h / 2.0).min(w / 2.0);
            let k = r * K;
            b.move_to((w * 0.15, 0.0));
            b.line_to((w - r, 0.0));
            b.curve_to((w - r + k, 0.0), (w, r - k), (w, r));
            b.line_to((w, h - r));
            b.curve_to((w, h - r + k), (w - r + k, h), (w - r, h));
            b.line_to((w * 0.15, h));
            b.line_to((0.0, h / 2.0));
            b.close_path();
        }
        "or" | "summingJunction" => {
            let (cx, cy, rx, ry) = (w / 2.0, h / 2.0, w / 2.0, h / 2.0);
            b.move_to((cx + rx, cy));
            b.curve_to((cx + rx, cy + ry * K), (cx + rx * K, cy + ry), (cx, cy + ry));
            b.curve_to((cx - rx * K, cy + ry), (cx - rx, cy + ry * K), (cx - rx, cy));
            b.curve_to((cx - rx, cy - ry * K), (cx - rx * K, cy - ry), (cx, cy - ry));
            b.curve_to((cx + rx * K, cy - ry), (cx + rx, cy - ry * K), (cx + rx, cy));
            b.close_path();
            if preset == "or" {
                b.move_to((cx, cy - ry));
                b.line_to((cx, cy + ry));
                b.move_to((cx - rx, cy));
                b.line_to((cx + rx, cy));
            } else {
                let d = std::f64::consts::FRAC_1_SQRT_2;
                b.move_to((cx - rx * d, cy - ry * d));
                b.line_to((cx + rx * d, cy + ry * d));
                b.move_to((cx + rx * d, cy - ry * d));
                b.line_to((cx - rx * d, cy + ry * d));
            }
        }
        _ => return None,
    }
    Some(b)
}

/// Make `n` the library shape `preset` at its current size (a vector; its name follows when it
/// still has a kind's default name). False for an unknown preset.
pub fn apply(n: &mut Node, preset: Preset) -> bool {
    let Some(path) = path(preset.id, n.width, n.height) else { return false };
    n.kind = NodeKind::Vector { network: VectorNetwork::from_path(&path) };
    n.plugin_data.insert(KEY.to_string(), preset.id.to_string());
    // Shapes drawn with lines inside (a cylinder's rim, a predefined process's bars) need a border
    // to show them: a darker tone of their fill, when they have none.
    if LINED.contains(&preset.id) && n.strokes.is_empty() {
        let fill = n.fills.iter().find_map(|p| match &p.kind {
            figcraft_color::PaintKind::Solid { color } => Some(*color),
            _ => None,
        });
        n.strokes = vec![figcraft_color::Paint::solid(fill.map(crate::board::darker).unwrap_or(crate::board::STRONG[1].color))];
        n.stroke.weight = 1.5;
        n.stroke.align = figcraft_doc::StrokeAlign::Center;
    }
    true
}

/// The library shapes drawn with lines inside them.
pub const LINED: &[&str] = &["cylinder", "cylinderHorizontal", "documents", "predefinedProcess", "internalStorage", "or", "summingJunction"];

/// Re-draw a library shape's outline for its box (after a resize scaled it out of shape).
pub fn refit(n: &mut Node) -> bool {
    match preset_of(n) {
        Some(p) => apply(n, p),
        None => false,
    }
}
