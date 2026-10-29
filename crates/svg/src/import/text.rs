//! SVG text → text layers. Each absolutely positioned chunk (`<text x y>`, `<tspan x y>`) is a
//! line; lines stacked at a steady pitch with the same anchor and font become one multi-line
//! layer with that line height. `<tspan>` styles become style runs. The layer is placed so its
//! first baseline sits on the SVG's `y` and its anchor (start, middle, end) on `x`, with the font
//! the SVG names when it's installed (else the first installed of its fallbacks or generic family).
//!
//! usvg lays text out before handing it over and drops what it can't lay out, so one face from
//! FigCraft's font database stands in for every family while parsing; FigCraft then lays the text
//! out with the real fonts. Text on one path comes in as text on a path; text on several paths,
//! with per-glyph rotation or vertical comes in as outlines.

use std::collections::HashSet;
use std::sync::Arc;

use figcraft_doc::{LetterSpacing, LineHeight, Node, NodeId, NodeKind, StyleRun, TextAlignH, TextCase, TextData, TextDecoration, TextStyle};
use figcraft_geom::{Affine, Point};
use figcraft_text::fontdb::FontBytes;
use usvg::{FontFamily, TextAnchor};

use super::paint::{Target, paint};
use super::shape::place;
use super::{Ctx, children, finish, finite, layer_name, xf};

/// Family names FigCraft can draw (lower case).
pub(crate) struct Fonts(HashSet<String>);

impl Fonts {
    fn load() -> Self {
        Fonts(figcraft_text::fontdb::family_names().into_iter().map(|f| f.to_lowercase()).collect())
    }
    /// Installed under this name, or under the family it names (`system-ui` on a Mac is its
    /// System Font, see `fontdb::apple_alias`).
    fn has(&self, family: &str) -> bool {
        let key = family.to_lowercase();
        self.0.contains(&key) || figcraft_text::fontdb::apple_alias(&key).is_some_and(|target| self.0.contains(target))
    }
}

struct Shared(Arc<Vec<u8>>);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

/// Let usvg parse text: one face stands in for every family (see the module docs).
pub(crate) fn prepare(o: &mut usvg::Options) {
    let Some(face) = figcraft_text::fontdb::find_face("Inter", "Regular") else { return };
    let source = match &face.bytes {
        FontBytes::Static(b) => usvg::fontdb::Source::Binary(Arc::new(*b)),
        FontBytes::Shared(v) => usvg::fontdb::Source::Binary(Arc::new(Shared(v.clone()))),
    };
    let mut db = usvg::fontdb::Database::new();
    db.load_font_source(source);
    o.fontdb = Arc::new(db);
    o.font_family = "serif".into();
    o.font_resolver.select_font = Box::new(|_, db| db.faces().next().map(|f| f.id));
    o.font_resolver.select_fallback = Box::new(|_, _, _| None);
}

/// Installed stand-ins for the generic families, best first.
fn generic(name: &str) -> Option<&'static [&'static str]> {
    const SANS: &[&str] = &["Inter", "Helvetica", "Arial", "Segoe UI", "Roboto", "Noto Sans", "DejaVu Sans", "Liberation Sans"];
    Some(match name.to_ascii_lowercase().as_str() {
        "serif" => &["Times New Roman", "Times", "Georgia", "Noto Serif", "DejaVu Serif", "Liberation Serif"],
        "monospace" => &["Menlo", "Consolas", "Courier New", "Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono"],
        "sans-serif" | "cursive" | "fantasy" | "system-ui" => SANS,
        _ => return None,
    })
}

/// The family to use: the first the SVG lists that's installed (a generic family as the first of
/// its installed stand-ins, unless a font answers to its name: `system-ui` on a Mac), else the
/// first it names, else the default.
fn family(list: &[FontFamily], fonts: &Fonts) -> String {
    let name = |f: &FontFamily| match f {
        FontFamily::Serif => "serif".to_string(),
        FontFamily::SansSerif => "sans-serif".to_string(),
        FontFamily::Cursive => "cursive".to_string(),
        FontFamily::Fantasy => "fantasy".to_string(),
        FontFamily::Monospace => "monospace".to_string(),
        FontFamily::Named(n) => n.trim().to_string(),
    };
    for f in list {
        let n = name(f);
        if fonts.has(&n) {
            return n;
        }
        if let Some(c) = generic(&n).and_then(|cands| cands.iter().find(|c| fonts.has(c))) {
            return (*c).to_string();
        }
    }
    list.iter().map(name).find(|n| generic(n).is_none() && !n.is_empty()).unwrap_or_else(|| TextStyle::default().font_family)
}

/// A style name for a CSS weight and slant ("Semi Bold Italic").
fn style_name(weight: u16, italic: bool) -> String {
    let w = match (weight + 50) / 100 {
        0 | 1 => "Thin",
        2 => "Extra Light",
        3 => "Light",
        4 => "Regular",
        5 => "Medium",
        6 => "Semi Bold",
        7 => "Bold",
        8 => "Extra Bold",
        _ => "Black",
    };
    match (w, italic) {
        ("Regular", true) => "Italic".into(),
        (w, true) => format!("{w} Italic"),
        (w, false) => w.into(),
    }
}

fn span_style(s: &usvg::TextSpan, scale: f64, fonts: &Fonts) -> TextStyle {
    let font = s.font();
    let deco = s.decoration();
    TextStyle {
        font_family: family(font.families(), fonts),
        font_style: style_name(font.weight(), font.style() != usvg::FontStyle::Normal),
        font_size: f64::from(s.font_size().get()) * scale,
        letter_spacing: {
            let v = f64::from(s.letter_spacing()) * scale;
            if v == 0.0 { LetterSpacing::default() } else { LetterSpacing::Pixels(v) }
        },
        decoration: if deco.underline().is_some() {
            TextDecoration::Underline
        } else if deco.line_through().is_some() {
            TextDecoration::Strikethrough
        } else {
            TextDecoration::None
        },
        case: if s.small_caps() { TextCase::SmallCaps } else { TextCase::Original },
        ..TextStyle::default()
    }
}

/// One chunk: its anchor point (text space), characters and spans (char ranges) with their styles.
struct Line<'a> {
    x: f64,
    y: f64,
    anchor: TextAnchor,
    text: String,
    spans: Vec<(usize, usize, &'a usvg::TextSpan, TextStyle)>,
}

impl Line<'_> {
    /// The style of its first span.
    fn style(&self) -> Option<&TextStyle> {
        self.spans.first().map(|s| &s.3)
    }
}

fn anchor_k(a: TextAnchor) -> f64 {
    match a {
        TextAnchor::Start => 0.0,
        TextAnchor::Middle => 0.5,
        TextAnchor::End => 1.0,
    }
}

pub(crate) fn text(cx: &mut Ctx, t: &usvg::Text, parent: NodeId, base: Affine, depth: usize) {
    let world = base * xf(t.abs_transform());
    if !finite(world) {
        return;
    }
    let along_path = t.chunks().iter().any(|c| matches!(c.text_flow(), usvg::TextFlow::Path(_)));
    let plain = !t.rotate().iter().any(|r| *r != 0.0) && t.writing_mode() != usvg::WritingMode::TopToBottom;
    if along_path && plain && on_path(cx, t, world, parent) {
        return;
    }
    if along_path || !plain {
        cx.note("text on a path, vertical or with rotated glyphs came in as outlines");
        if let Some(id) = cx.group(parent, &layer_name(t.id(), "Text"), true) {
            children(cx, t.flattened(), id, base, depth + 1);
            finish(cx, id);
        }
        return;
    }
    // Uniform scale goes into the font size; anything else stays in the layer's transform.
    let (m, s) = match place(world, Point::ZERO, 1.0, 1.0) {
        Some((m, sx, sy)) if (sx - sy).abs() <= 1e-6 * sx.max(sy) => (m, sx),
        _ => (world, 1.0),
    };
    let fonts = cx.fonts.get_or_insert_with(Fonts::load);
    let mut lines: Vec<Line> = Vec::new();
    let (mut pen_x, mut pen_y, mut char_at) = (0.0, 0.0, 0usize);
    let chunks = t.chunks();
    for (i, chunk) in chunks.iter().enumerate() {
        let text = chunk.text();
        let count = text.chars().count();
        let d = |v: &[f32]| f64::from(v.get(char_at).copied().unwrap_or(0.0));
        let x = chunk.x().map_or(pen_x, f64::from) + d(t.dx());
        let y = chunk.y().map_or(pen_y, f64::from) + d(t.dy());
        char_at += count;
        let ci = |b: usize| text.get(..b).map_or(count, |p| p.chars().count());
        let spans: Vec<_> = chunk.spans().iter().map(|sp| (ci(sp.start()), ci(sp.end()), sp, span_style(sp, s, fonts))).collect();
        if spans.is_empty() {
            continue;
        }
        let line = Line { x, y, anchor: chunk.anchor(), text: text.to_string(), spans };
        // The next chunk without its own x continues where this one ends.
        if chunks.get(i + 1).is_some_and(|c| c.x().is_none()) {
            let width = figcraft_text::layout(&text_data(std::slice::from_ref(&line), s), None, None).width / s;
            pen_x = x + (1.0 - anchor_k(line.anchor)) * width;
        }
        pen_y = y;
        lines.push(line);
    }
    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        if let (Some(a), Some(b)) = (lines.get(i), lines.get(i + 1)) {
            let step = b.y - a.y;
            while let (Some(prev), Some(next)) = (lines.get(j - 1), lines.get(j)) {
                let same = next.anchor == a.anchor && (next.x - a.x).abs() < 0.01 && next.style() == a.style() && step > 0.0;
                if !same || (next.y - prev.y - step).abs() > 0.01 {
                    break;
                }
                j += 1;
            }
        }
        if let Some(block) = lines.get(i..j) {
            layer(cx, t, block, m, s, world, parent);
        }
        i = j;
    }
}

/// The text of `lines` (one per line, stacked at their pitch; `s`: their scale): characters, the
/// first span's style and a run per span, fills still to come.
fn text_data(lines: &[Line], s: f64) -> TextData {
    let mut td = TextData::new(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(
        "
",
    ));
    if let Some(style) = lines.first().and_then(Line::style) {
        td.style = style.clone();
    }
    if let (Some(a), Some(b)) = (lines.first(), lines.get(1)) {
        td.style.line_height = LineHeight::Pixels((b.y - a.y) * s);
    }
    let mut offset = 0;
    for l in lines {
        for (a, b, _, st) in &l.spans {
            td.runs.push(StyleRun { start: offset + a, end: offset + b, style: TextStyle { line_height: td.style.line_height, ..st.clone() } });
        }
        offset += l.text.chars().count() + 1;
    }
    td
}

/// Text on one path (a single `<textPath>`) as a text layer on that path; false when the text
/// isn't one chunk on one path.
fn on_path(cx: &mut Ctx, t: &usvg::Text, world: Affine, parent: NodeId) -> bool {
    let [chunk] = t.chunks() else { return false };
    let usvg::TextFlow::Path(tp) = chunk.text_flow() else { return false };
    let path = super::to_bez(tp.path());
    let bb = figcraft_geom::Shape::bounding_box(&path);
    let Some(len) = figcraft_geom::measure::polylines(&path).and_then(|l| l.first().map(|l| l.length())).filter(|l| *l > 1e-9) else { return false };
    let fonts = cx.fonts.get_or_insert_with(Fonts::load);
    let text = chunk.text();
    let count = text.chars().count();
    let ci = |b: usize| text.get(..b).map_or(count, |p| p.chars().count());
    let spans: Vec<_> = chunk.spans().iter().map(|sp| (ci(sp.start()), ci(sp.end()), sp, span_style(sp, 1.0, fonts))).collect();
    let line = Line { x: 0.0, y: 0.0, anchor: chunk.anchor(), text: text.to_string(), spans };
    let mut td = text_data(std::slice::from_ref(&line), 1.0);
    td.align_h = align(line.anchor);
    let mut local = path.clone();
    local.apply_affine(Affine::translate((-bb.x0, -bb.y0)));
    td.path = Some(Box::new(figcraft_doc::TextPath { start: f64::from(tp.start_offset()) / len, ..figcraft_doc::TextPath::new(&local) }));
    let transform = world * Affine::translate((bb.x0, bb.y0));
    add_layer(
        cx,
        TextLayer { source: t, block: &[line], data: td, transform, size: (bb.width().max(1.0), bb.height().max(1.0)), world, scale: 1.0, parent },
    );
    true
}

fn align(a: TextAnchor) -> TextAlignH {
    match a {
        TextAnchor::Start => TextAlignH::Left,
        TextAnchor::Middle => TextAlignH::Center,
        TextAnchor::End => TextAlignH::Right,
    }
}

/// The text layer for `block` (lines sharing an anchor, x and pitch).
fn layer(cx: &mut Ctx, t: &usvg::Text, block: &[Line], m: Affine, s: f64, world: Affine, parent: NodeId) {
    let Some(first) = block.first() else { return };
    let mut td = text_data(block, s);
    td.align_h = align(first.anchor);
    // Placed by its layout (runs included): the anchor on x, the first baseline on y.
    let lay = figcraft_text::layout(&td, None, None);
    let (w, h) = (lay.width.ceil().max(1.0), lay.height.max(1.0));
    let baseline = lay.lines.first().map_or(0.0, |l| l.baseline);
    let transform = m * Affine::translate((s * first.x - anchor_k(first.anchor) * w, s * first.y - baseline));
    add_layer(cx, TextLayer { source: t, block, data: td, transform, size: (w, h), world, scale: s, parent });
}

struct TextLayer<'text, 'block, 'span> {
    source: &'text usvg::Text,
    block: &'block [Line<'span>],
    data: TextData,
    transform: Affine,
    size: (f64, f64),
    world: Affine,
    scale: f64,
    parent: NodeId,
}

/// Add text layer `td` of `block` at `transform` with box `(w, h)`, with the spans' fills and the
/// first span's stroke (`world`: the SVG text's space; `s`: the scale folded into font sizes).
fn add_layer(cx: &mut Ctx, layer: TextLayer<'_, '_, '_>) {
    let TextLayer { source: t, block, data: mut td, transform, size: (w, h), world, scale: s, parent } = layer;
    let Some(first) = block.first() else { return };
    let spans: Vec<&usvg::TextSpan> = block.iter().flat_map(|l| l.spans.iter().map(|sp| sp.2)).collect();
    if spans.iter().all(|sp| !sp.is_visible()) {
        return;
    }
    let target = Target { to_node: transform.inverse() * world, size: (w, h) };
    let mut fills = Vec::with_capacity(spans.len());
    for sp in &spans {
        let f: Vec<figcraft_color::Paint> =
            if sp.is_visible() { sp.fill().and_then(|f| paint(cx, f.paint(), f.opacity().get(), target)).into_iter().collect() } else { Vec::new() };
        fills.push(f);
    }
    let base_fill = fills.first().cloned().unwrap_or_default();
    for (run, f) in td.runs.iter_mut().zip(fills) {
        if f != base_fill {
            run.style.fills = Some(f);
        }
    }
    td.normalize();
    let name = if t.id().is_empty() { first.text.chars().take(64).collect() } else { layer_name(t.id(), "Text") };
    let mut n = Node::new(cx.doc.alloc_id(), NodeKind::Text(td), name, w, h);
    n.transform = transform;
    n.fills = base_fill;
    if let Some(st) = spans.first().and_then(|sp| sp.stroke()) {
        n.strokes = paint(cx, st.paint(), st.opacity().get(), target).into_iter().collect();
        n.stroke = super::stroke_props(st, s);
    }
    cx.add(n, parent);
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
