//! How faithfully a `.fig` round trip (or any two documents, or two renders) keeps a design:
//! a structural report from comparing two documents layer by layer, and a pixel report from
//! comparing two RGBA buffers. Both are deterministic (fixed walk and summation order, no hashing)
//! and print as JSON and as a Markdown scorecard.
//!
//! Layers are matched by position: page `i` with page `i`, and child `j` of a matched layer with
//! child `j` of its counterpart, when both are the same kind of layer. A matched layer's bounding
//! box error is the L∞ distance between its page bounds in both documents ([`bbox_linf`]).
//!
//! Pixels: `ssd` is the exact sum of squared channel differences over every channel (alpha
//! included), so it is 0 exactly when the buffers are equal, symmetric, and at most
//! `len × 255²`. `diff_count` counts channels differing by more than `tol`. SSIM is computed on
//! luminance (each pixel over white) in 8 × 8 windows (smaller at the edges), averaged, and
//! clamped to `0..=1`; the stabilising constants keep flat images defined, and an empty image
//! scores 1. Non-finite numbers (PSNR of identical images is infinite) print as the strings
//! `"inf"` and `"nan"` in JSON, since JSON numbers can't hold them.

use std::fmt::Write as _;

use figcraft_doc::color::Paint;
use figcraft_doc::geom::Rect;
use figcraft_doc::{Document, Node, NodeId, NodeKind};
use serde_json::{Value, json};

/// The largest squared difference of one 8-bit channel.
pub const MAX_CHANNEL_SSD: u64 = 255 * 255;
/// How many of the worst-placed layers a report lists.
const WORST: usize = 5;
/// Two opacities closer than this match.
const OPACITY_EPS: f32 = 1e-6;
/// Two numbers in a paint closer than this match: `.fig` files store paints in 32-bit floats, so
/// a gradient handle comes back a few ten-millionths off.
const PAINT_EPS: f64 = 1e-5;
/// SSIM window side, in pixels.
const SSIM_WINDOW: usize = 8;

/// A matched layer and how far its box moved.
#[derive(Clone, Debug, PartialEq)]
pub struct BBoxDiff {
    /// Child indices from the page down (`0/2/1`: page 0, its child 2, that one's child 1).
    pub path: String,
    pub name: String,
    /// L∞ distance between the two page-space boxes ([`bbox_linf`]).
    pub linf: f64,
}

/// What a document comparison found.
#[derive(Clone, Debug, PartialEq)]
pub struct StructuralReport {
    /// Layers under the pages (pages themselves not counted).
    pub expected_layers: usize,
    pub actual_layers: usize,
    /// Pages and layers paired by position with a layer of the same kind.
    pub matched_layers: usize,
    pub bbox_max_linf: f64,
    pub bbox_mean_linf: f64,
    /// Matched text layers, and those with the same characters.
    pub text_total: usize,
    pub text_matched: usize,
    /// Matched layers with the same fills and strokes (numbers within a hundred-thousandth: `.fig`
    /// stores paints in 32-bit floats) and opacity.
    pub fill_matched: usize,
    pub stroke_matched: usize,
    pub opacity_matched: usize,
    /// Import warnings the caller attached ([`StructuralReport::with_warnings`]).
    pub warnings: usize,
    /// The matched layers that moved furthest, worst first (ties by path).
    pub worst: Vec<BBoxDiff>,
}

impl StructuralReport {
    /// Attach the number of import warnings that produced the compared document.
    pub fn with_warnings(mut self, warnings: usize) -> Self {
        self.warnings = warnings;
        self
    }

    pub fn layer_count_preserved(&self) -> bool {
        self.expected_layers == self.actual_layers
    }

    /// Matched layers over expected ones (1 when nothing was expected).
    pub fn match_rate(&self) -> f64 {
        rate(self.matched_layers, self.expected_layers)
    }
    pub fn text_match_rate(&self) -> f64 {
        rate(self.text_matched, self.text_total)
    }
    pub fn fill_match_rate(&self) -> f64 {
        rate(self.fill_matched, self.matched_layers)
    }
    pub fn stroke_match_rate(&self) -> f64 {
        rate(self.stroke_matched, self.matched_layers)
    }
    pub fn opacity_match_rate(&self) -> f64 {
        rate(self.opacity_matched, self.matched_layers)
    }

    /// Every layer matched and kept its box (within `bbox_tol`), text, paints and opacity.
    pub fn is_exact(&self, bbox_tol: f64) -> bool {
        self.layer_count_preserved()
            && self.matched_layers == self.expected_layers
            && self.bbox_max_linf <= bbox_tol
            && self.text_matched == self.text_total
            && [self.fill_matched, self.stroke_matched, self.opacity_matched].iter().all(|n| *n == self.matched_layers)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "expectedLayers": self.expected_layers,
            "actualLayers": self.actual_layers,
            "layerCountPreserved": self.layer_count_preserved(),
            "matchedLayers": self.matched_layers,
            "matchRate": number(self.match_rate()),
            "bboxMaxLinf": number(self.bbox_max_linf),
            "bboxMeanLinf": number(self.bbox_mean_linf),
            "textMatchRate": number(self.text_match_rate()),
            "fillMatchRate": number(self.fill_match_rate()),
            "strokeMatchRate": number(self.stroke_match_rate()),
            "opacityMatchRate": number(self.opacity_match_rate()),
            "warnings": self.warnings,
            "worst": self.worst.iter().map(|w| json!({"path": w.path, "name": w.name, "linf": number(w.linf)})).collect::<Vec<_>>(),
        })
    }

    /// A Markdown table of `rows` (name, report), one row each, in the order given.
    pub fn markdown_table(rows: &[(String, StructuralReport)]) -> String {
        let mut out = String::from(
            "| File | Layers (expected → actual) | Matched | BBox max L∞ (px) | BBox mean L∞ (px) | Text | Fill | Stroke | Opacity | Warnings |\n\
             |---|---|---|---|---|---|---|---|---|---|\n",
        );
        for (name, r) in rows {
            let _ = writeln!(
                out,
                "| {} | {} → {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                name.replace('|', "\\|"),
                r.expected_layers,
                r.actual_layers,
                percent(r.match_rate()),
                fixed(r.bbox_max_linf),
                fixed(r.bbox_mean_linf),
                percent(r.text_match_rate()),
                percent(r.fill_match_rate()),
                percent(r.stroke_match_rate()),
                percent(r.opacity_match_rate()),
                r.warnings
            );
        }
        out
    }
}

/// What a pixel comparison found.
#[derive(Clone, Debug, PartialEq)]
pub struct PixelDiffReport {
    pub width: u32,
    pub height: u32,
    /// Sum of squared channel differences.
    pub ssd: u64,
    pub rmse: f64,
    /// Infinite for identical images.
    pub psnr_db: f64,
    /// Mean luminance SSIM, `0..=1`.
    pub ssim: f64,
    /// Channels differing by more than `tol`.
    pub diff_count: u64,
    /// `diff_count` as a percentage of all channels.
    pub diff_pct: f64,
    pub tol: u8,
}

impl PixelDiffReport {
    pub fn to_json(&self) -> Value {
        json!({
            "width": self.width,
            "height": self.height,
            "ssd": self.ssd,
            "rmse": number(self.rmse),
            "psnrDb": number(self.psnr_db),
            "ssim": number(self.ssim),
            "diffCount": self.diff_count,
            "diffPct": number(self.diff_pct),
            "tol": self.tol,
        })
    }

    pub fn to_markdown(&self) -> String {
        format!(
            "| Size | SSD | RMSE | PSNR (dB) | SSIM | Channels off by > {} |\n|---|---|---|---|---|---|\n| {} × {} | {} | {} | {} | {} | {} |\n",
            self.tol,
            self.width,
            self.height,
            self.ssd,
            fixed(self.rmse),
            fixed(self.psnr_db),
            fixed(self.ssim),
            percent(self.diff_pct / 100.0)
        )
    }
}

/// Why two buffers couldn't be compared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PixelDiffError {
    /// `width × height × 4` overflows.
    TooLarge,
    /// A buffer's length isn't `width × height × 4`.
    WrongLength { expected: usize, a: usize, b: usize },
}

impl std::fmt::Display for PixelDiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "the image size overflows"),
            Self::WrongLength { expected, a, b } => write!(f, "expected {expected} bytes of RGBA, got {a} and {b}"),
        }
    }
}

/// Sum of squared differences of two equally long buffers; `None` when their lengths differ.
pub fn ssd(a: &[u8], b: &[u8]) -> Option<u64> {
    (a.len() == b.len()).then(|| a.iter().zip(b).map(|(x, y)| u64::from(x.abs_diff(*y)).pow(2)).sum())
}

/// Channels of two equally long buffers differing by more than `tol`; `None` when their lengths
/// differ.
pub fn diff_count(a: &[u8], b: &[u8], tol: u8) -> Option<u64> {
    (a.len() == b.len()).then(|| a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > tol).count() as u64)
}

/// The L∞ distance between two boxes: the largest difference of their four edges. Infinite when
/// any edge is not a number, so a broken box never counts as a match.
pub fn bbox_linf(a: Rect, b: Rect) -> f64 {
    let d = [(a.x0 - b.x0).abs(), (a.y0 - b.y0).abs(), (a.x1 - b.x1).abs(), (a.y1 - b.y1).abs()];
    if d.iter().any(|v| v.is_nan()) { f64::INFINITY } else { d.into_iter().fold(0.0, f64::max) }
}

/// Compare two RGBA8 buffers of `width × height` pixels.
pub fn compare_rgba(a: &[u8], b: &[u8], width: u32, height: u32, tol: u8) -> Result<PixelDiffReport, PixelDiffError> {
    let expected = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)).ok_or(PixelDiffError::TooLarge)?;
    if a.len() != expected || b.len() != expected {
        return Err(PixelDiffError::WrongLength { expected, a: a.len(), b: b.len() });
    }
    let ssd = ssd(a, b).unwrap_or(0);
    let diff_count = diff_count(a, b, tol).unwrap_or(0);
    let mse = if expected == 0 { 0.0 } else { ssd as f64 / expected as f64 };
    let psnr_db = if mse == 0.0 { f64::INFINITY } else { 10.0 * (MAX_CHANNEL_SSD as f64 / mse).log10() };
    Ok(PixelDiffReport {
        width,
        height,
        ssd,
        rmse: mse.sqrt(),
        psnr_db,
        ssim: luminance_ssim(a, b, width as usize, height as usize),
        diff_count,
        diff_pct: if expected == 0 { 0.0 } else { 100.0 * diff_count as f64 / expected as f64 },
        tol,
    })
}

/// A pixel's luminance (Rec. 601) over white, `0..=255`.
fn luminance(px: &[u8]) -> f64 {
    let alpha = f64::from(px[3]) / 255.0;
    let y = 0.299 * f64::from(px[0]) + 0.587 * f64::from(px[1]) + 0.114 * f64::from(px[2]);
    y * alpha + 255.0 * (1.0 - alpha)
}

/// Mean SSIM of the two images' luminance over [`SSIM_WINDOW`]-sided windows, clamped to `0..=1`.
/// Each window's means come first and its (co)variances from the deviations, so equal windows
/// score exactly 1.
fn luminance_ssim(a: &[u8], b: &[u8], width: usize, height: usize) -> f64 {
    const C1: f64 = (0.01 * 255.0) * (0.01 * 255.0);
    const C2: f64 = (0.03 * 255.0) * (0.03 * 255.0);
    let (mut total, mut windows) = (0.0, 0usize);
    let mut pairs = Vec::with_capacity(SSIM_WINDOW * SSIM_WINDOW);
    for wy in (0..height).step_by(SSIM_WINDOW) {
        for wx in (0..width).step_by(SSIM_WINDOW) {
            pairs.clear();
            for y in wy..(wy + SSIM_WINDOW).min(height) {
                for x in wx..(wx + SSIM_WINDOW).min(width) {
                    let i = (y * width + x) * 4;
                    pairs.push((luminance(&a[i..i + 4]), luminance(&b[i..i + 4])));
                }
            }
            let n = pairs.len() as f64;
            let (ma, mb) = (pairs.iter().map(|p| p.0).sum::<f64>() / n, pairs.iter().map(|p| p.1).sum::<f64>() / n);
            let (mut va, mut vb, mut cov) = (0.0, 0.0, 0.0);
            for (la, lb) in &pairs {
                let (da, db) = (la - ma, lb - mb);
                (va, vb, cov) = (va + da * da, vb + db * db, cov + da * db);
            }
            let (va, vb, cov) = (va / n, vb / n, cov / n);
            total += ((2.0 * ma * mb + C1) * (2.0 * cov + C2)) / ((ma * ma + mb * mb + C1) * (va + vb + C2));
            windows += 1;
        }
    }
    if windows == 0 { 1.0 } else { (total / windows as f64).clamp(0.0, 1.0) }
}

/// Compare `actual` (say, a document after a `.fig` round trip) with `expected`, layer by layer.
/// Warnings start at 0; attach them with [`StructuralReport::with_warnings`].
pub fn compare_documents(expected: &Document, actual: &Document) -> StructuralReport {
    let mut report = StructuralReport {
        expected_layers: layer_count(expected),
        actual_layers: layer_count(actual),
        matched_layers: 0,
        bbox_max_linf: 0.0,
        bbox_mean_linf: 0.0,
        text_total: 0,
        text_matched: 0,
        fill_matched: 0,
        stroke_matched: 0,
        opacity_matched: 0,
        warnings: 0,
        worst: Vec::new(),
    };
    let mut diffs = Vec::new();
    // (expected, actual, path); pages pair by index and only their layers are scored.
    let mut pending: Vec<(NodeId, NodeId, String)> =
        expected.pages.iter().zip(&actual.pages).enumerate().rev().map(|(i, (e, a))| (*e, *a, i.to_string())).collect();
    while let Some((e, a, path)) = pending.pop() {
        let (Some(en), Some(an)) = (expected.node(e), actual.node(a)) else { continue };
        if !en.is_page() {
            score(&mut report, &mut diffs, expected, actual, en, an, &path);
        }
        for (i, (ec, ac)) in en.children.iter().zip(&an.children).enumerate().rev() {
            let same_kind = expected.node(*ec).zip(actual.node(*ac)).is_some_and(|(x, y)| same_kind(&x.kind, &y.kind));
            if same_kind {
                pending.push((*ec, *ac, format!("{path}/{i}")));
            }
        }
    }
    if !diffs.is_empty() {
        report.bbox_mean_linf = diffs.iter().map(|d: &BBoxDiff| d.linf).sum::<f64>() / diffs.len() as f64;
    }
    diffs.sort_by(|x, y| y.linf.total_cmp(&x.linf).then_with(|| x.path.cmp(&y.path)));
    diffs.truncate(WORST);
    report.worst = diffs;
    report
}

fn score(report: &mut StructuralReport, diffs: &mut Vec<BBoxDiff>, expected: &Document, actual: &Document, en: &Node, an: &Node, path: &str) {
    report.matched_layers += 1;
    let linf = match (expected.world_bounds(en.id), actual.world_bounds(an.id)) {
        (Some(x), Some(y)) => bbox_linf(x, y),
        (None, None) => 0.0,
        _ => f64::INFINITY,
    };
    report.bbox_max_linf = report.bbox_max_linf.max(linf);
    diffs.push(BBoxDiff { path: path.to_string(), name: en.name.clone(), linf });
    if let (NodeKind::Text(x), NodeKind::Text(y)) = (&en.kind, &an.kind) {
        report.text_total += 1;
        report.text_matched += usize::from(x.characters == y.characters);
    }
    report.fill_matched += usize::from(paints_match(&en.fills, &an.fills));
    report.stroke_matched += usize::from(paints_match(&en.strokes, &an.strokes));
    report.opacity_matched += usize::from((en.opacity - an.opacity).abs() <= OPACITY_EPS);
}

/// The same paints, numbers within [`PAINT_EPS`] of each other.
fn paints_match(a: &[Paint], b: &[Paint]) -> bool {
    a == b || serde_json::to_value(a).ok().zip(serde_json::to_value(b).ok()).is_some_and(|(x, y)| close(&x, &y))
}

/// Two JSON values with the same shape, strings and flags, and numbers within [`PAINT_EPS`].
fn close(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().zip(y.as_f64()).is_some_and(|(x, y)| (x - y).abs() <= PAINT_EPS),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| close(x, y)),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w))),
        _ => a == b,
    }
}

/// Layers under the document's pages.
fn layer_count(doc: &Document) -> usize {
    doc.nodes().filter(|n| !n.is_page()).count()
}

fn same_kind(a: &NodeKind, b: &NodeKind) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

fn rate(n: usize, of: usize) -> f64 {
    if of == 0 { 1.0 } else { n as f64 / of as f64 }
}

/// A JSON number, or `"inf"`/`"-inf"`/`"nan"` for what JSON can't hold.
fn number(v: f64) -> Value {
    match v {
        v if v.is_nan() => json!("nan"),
        v if v == f64::INFINITY => json!("inf"),
        v if v == f64::NEG_INFINITY => json!("-inf"),
        v => json!(v),
    }
}

fn fixed(v: f64) -> String {
    if v.is_finite() { format!("{v:.4}") } else { format!("{v}") }
}

fn percent(r: f64) -> String {
    format!("{:.2}%", r * 100.0)
}
