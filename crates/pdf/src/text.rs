//! Text: real, selectable text in embedded (subset) fonts, positioned glyph by glyph from the
//! `figcraft_text` layout the renderer uses, with outline fallbacks.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

use figcraft_doc::{Node, NodeKind};
use figcraft_geom::{Affine, BezPath};
use figcraft_text::fontdb::FontBytes;
use figcraft_text::{Face, Glyph, TextLayout};
use krilla::geom::Point;
use krilla::paint::{Fill, FillRule};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};

use figcraft_color::{GradientKind, Paint, PaintKind};

use crate::draw::{Exporter, above, affine_ok, blend, stroke_rule, to_xf, vector_paint};
use crate::raster::Raster;

/// Glyphs of one face and size on one line, written as one text run.
struct Run {
    face: u32,
    font: Font,
    size: f64,
    x0: f64,
    y0: f64,
    /// (glyph id, x, y, advance, source text range in `text`)
    glyphs: Vec<(u32, f64, f64, f64, Range<usize>)>,
    text: String,
    /// The last glyph's source chars (glyphs of one cluster share their text).
    last_chars: Option<(usize, usize)>,
}

/// Draw a text node; `xf` maps its local space to the page.
pub(crate) fn draw_text(ex: &mut Exporter, s: &mut Surface, n: &Node, xf: Affine, inner: &[Raster]) {
    let NodeKind::Text(t) = &n.kind else { return };
    let Some((layout, dy)) = figcraft_text::layout_node(n) else { return };
    let gxf = xf * Affine::translate((0.0, dy));
    if !affine_ok(gxf) {
        return;
    }
    if ex.masking == 0 && ex.tiling == 0 {
        record_links(ex, &layout, gxf);
    }
    let (w, h) = (n.width, n.height);
    let chars: Vec<char> = t.characters.chars().collect();
    let styles: BTreeSet<usize> = layout.glyphs.iter().map(|g| g.style).chain(layout.decorations.iter().map(|d| d.style)).collect();
    // Every run's fills, then the inner shadows over them, then the strokes on top.
    let mut stroked = Vec::new();
    for k in styles {
        let fills = layout.styles.get(k).and_then(|st| st.fills.as_ref()).unwrap_or(&n.fills);
        let mut decorations = BezPath::new();
        for d in layout.decorations.iter().filter(|d| d.style == k) {
            decorations.extend(d.path());
        }
        let mut outlines = BezPath::new();
        for g in layout.glyphs.iter().filter(|g| g.style == k) {
            outlines.extend(g.path());
        }
        outlines.extend(decorations.iter());
        let mut real_text_written = false;
        for p in fills {
            if !p.visible || !above(p.opacity, 0.0) {
                continue;
            }
            let embeddable = matches!(&p.kind, PaintKind::Solid { .. })
                || matches!(&p.kind, PaintKind::Gradient(g) if matches!(g.kind, GradientKind::Linear | GradientKind::Radial));
            // shortcut: text on a path is written as outlines (not selectable); turn each glyph
            // with its own text matrix when selectable path text is asked for.
            if embeddable && !real_text_written && !ex.opts.outline_text && layout.on_path.is_none() {
                // The first fill writes the text itself; the rest paint its outlines on top.
                real_text_written = true;
                let mut rest = glyph_runs(ex, s, &layout, k, &chars, p, w, h, gxf);
                rest.extend(decorations.iter());
                ex.fill(s, &rest, FillRule::NonZero, p, w, h, gxf);
            } else {
                ex.fill(s, &outlines, FillRule::NonZero, p, w, h, gxf);
            }
        }
        if n.has_visible_stroke() {
            stroked.push(outlines);
        }
    }
    ex.place_inner(s, inner);
    // Glyph outlines are closed: inside and outside strokes clip to them, as on the canvas (#615).
    for outlines in &stroked {
        let area = figcraft_render::draw::stroke_area(n, outlines, true);
        for p in &n.strokes {
            ex.fill(s, &area, stroke_rule(n, true), p, w, h, gxf);
        }
    }
}

/// Most hyperlink areas on one page.
const MAX_LINKS: usize = 10_000;

/// Note the hyperlinked text of a layout (`gxf`: layout → PDF page): one area per line and run of
/// glyphs linking to the same URL.
fn record_links(ex: &mut Exporter, layout: &TextLayout, gxf: Affine) {
    for line in &layout.lines {
        let mut run: Option<(&str, f64, f64)> = None;
        let glyphs = layout.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]);
        for g in glyphs.iter().map(Some).chain([None]) {
            let link = g.and_then(|g| {
                let url = layout.styles.get(g.style)?.link.as_deref().filter(|u| !u.trim().is_empty())?;
                let (a, b) = (g.x, g.x + g.advance);
                (a.is_finite() && b.is_finite()).then(|| (url, a.min(b), a.max(b)))
            });
            if let (Some(r), Some((url, a, b))) = (run.as_mut(), link)
                && r.0 == url
            {
                r.1 = r.1.min(a);
                r.2 = r.2.max(b);
                continue;
            }
            if let Some((url, x0, x1)) = run.take()
                && ex.links.len() < MAX_LINKS
            {
                let area = gxf.transform_rect_bbox(figcraft_geom::Rect::new(x0, line.top, x1, line.top + line.height));
                ex.links.push((area, url.to_string()));
            }
            run = link;
        }
    }
}

impl Exporter<'_> {
    /// The PDF font of a face (cached; `None` when krilla can't read it).
    fn font(&mut self, face: &Arc<Face>) -> Option<Font> {
        if let Some(f) = self.fonts.get(&face.id) {
            return f.clone();
        }
        let data = match &face.bytes {
            FontBytes::Static(b) => krilla::Data::from(*b),
            FontBytes::Shared(v) => krilla::Data::from(v.clone()),
        };
        let font = Font::new(data, face.index);
        if font.is_none() {
            log::warn!("PDF export: the font {} {} can't be embedded; its text is outlined", face.family, face.style);
        }
        self.fonts.insert(face.id, font.clone());
        font
    }
}

/// Write the glyphs of style `style` as real text filled with `p` (layout space, mapped to the
/// page by `gxf`). Returns the outlines of glyphs that can't be written as text.
#[allow(clippy::too_many_arguments)]
fn glyph_runs(
    ex: &mut Exporter,
    s: &mut Surface,
    layout: &TextLayout,
    style: usize,
    chars: &[char],
    p: &Paint,
    w: f64,
    h: f64,
    gxf: Affine,
) -> BezPath {
    let mut rest = BezPath::new();
    for line in &layout.lines {
        let mut run: Option<Run> = None;
        for g in layout.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]).iter().filter(|g| g.style == style) {
            // Glyphs at a variable font instance other than the default are outlined (an
            // embedded font draws its default instance).
            let has_source = g.char_end > g.char_start && g.char_end <= chars.len() && g.coords.is_empty();
            let font = if has_source && g.size.is_finite() && g.size > 0.0 && g.x.is_finite() && g.y.is_finite() { ex.font(&g.face) } else { None };
            let Some(font) = font else {
                // List markers, ellipses and unembeddable fonts: outlines.
                rest.extend(g.path());
                continue;
            };
            let continues = run.as_ref().is_some_and(|r| r.face == g.face.id && (r.size - g.size).abs() < 1e-9);
            if !continues {
                if let Some(r) = run.take() {
                    write_run(s, r, p, w, h, gxf, ex.ink());
                }
                run = Some(Run { face: g.face.id, font, size: g.size, x0: g.x, y0: g.y, glyphs: Vec::new(), text: String::new(), last_chars: None });
            }
            if let Some(r) = run.as_mut() {
                push_glyph(r, g, chars);
            }
        }
        if let Some(r) = run.take() {
            write_run(s, r, p, w, h, gxf, ex.ink());
        }
    }
    rest
}

fn push_glyph(r: &mut Run, g: &Glyph, chars: &[char]) {
    let src = (g.char_start, g.char_end);
    let range = match (r.last_chars, r.glyphs.last()) {
        // Another glyph of the same cluster: share its text.
        (Some(last), Some(prev)) if last == src => prev.4.clone(),
        _ => {
            let start = r.text.len();
            r.text.extend(chars.get(g.char_start..g.char_end).unwrap_or(&[]).iter());
            start..r.text.len()
        }
    };
    r.last_chars = Some(src);
    r.glyphs.push((g.gid, g.x, g.y, g.advance, range));
}

#[allow(clippy::too_many_arguments)]
fn write_run(s: &mut Surface, r: Run, p: &Paint, w: f64, h: f64, gxf: Affine, ink: crate::print::Ink) {
    if r.glyphs.is_empty() {
        return;
    }
    let Some((paint, opacity)) = vector_paint(p, w, h, ink) else { return };
    let size = r.size;
    let glyphs: Vec<KrillaGlyph> = r
        .glyphs
        .iter()
        .enumerate()
        .map(|(i, (gid, x, y, advance, range))| {
            // Absolute positions: each advance reaches the next glyph's pen position.
            let next = r.glyphs.get(i + 1).map_or(x + advance, |n| n.1);
            KrillaGlyph::new(GlyphId::new(*gid), ((next - x) / size) as f32, 0.0, ((r.y0 - y) / size) as f32, 0.0, range.clone(), None)
        })
        .collect();
    let pb = blend(p.blend);
    if let Some(b) = pb {
        s.push_blend_mode(b);
    }
    s.push_transform(&to_xf(gxf));
    s.set_stroke(None);
    s.set_fill(Some(Fill { paint, opacity, rule: FillRule::NonZero }));
    s.draw_glyphs(Point::from_xy(r.x0 as f32, r.y0 as f32), &glyphs, r.font, &r.text, size as f32, false);
    s.pop();
    if pb.is_some() {
        s.pop();
    }
}
