//! Text layout: shaping styled spans, fallback fonts, bidirectional text, line breaking,
//! alignment and decorations.

use std::sync::Arc;

use figcraft_doc::{
    DecorationLength, DecorationStyle, LeadingTrim, ListStyle, ParaList, TextAlignH, TextAutoResize, TextCase, TextData, TextDecoration, TextStyle,
    valid_feature_tag,
};
use figcraft_geom::{Affine, BezPath, Rect};
use harfrust::{Direction, Feature, ShapeOptions, ShaperInstance, Tag, UnicodeBuffer, Variation};

use crate::color::{ColorInk, MAX_PX_PER_EM};
use crate::fontdb::{Coords, Face, db, no_coords};

/// A positioned glyph (pen position on the baseline, layout coordinates, y down).
#[derive(Clone, Debug)]
pub struct Glyph {
    pub face: Arc<Face>,
    /// Variable font coordinates (empty = the face's default instance).
    pub coords: Coords,
    pub gid: u32,
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub advance: f64,
    /// Source char range of the glyph's cluster.
    pub char_start: usize,
    pub char_end: usize,
    /// Index into [`TextLayout::styles`].
    pub style: usize,
    /// Part of a right-to-left run: its first char sits at the glyph's right edge.
    pub rtl: bool,
    /// Moves the placed glyph onto the text's path (identity without one).
    pub warp: Affine,
}

impl Glyph {
    /// The glyph outline placed in layout coordinates.
    pub fn path(&self) -> BezPath {
        self.layer_path(self.gid)
    }

    /// The outline of one of the face's glyphs (a COLR layer), placed like this glyph.
    pub fn layer_path(&self, gid: u32) -> BezPath {
        let s = self.size / self.face.upem;
        let mut p = (*self.face.outline_at(gid, &self.coords)).clone();
        p.apply_affine(self.warp * Affine::new([s, 0.0, 0.0, s, self.x, self.y]));
        p
    }

    /// The glyph's colour ink at `scale` device pixels per layout unit, placed in layout
    /// coordinates (a bitmap's pixels map through the affine; COLR layers are drawn like the
    /// outline). `None` — the usual case — means the outline is what draws.
    pub fn color(&self, scale: f64) -> Option<ColorInk> {
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        if let Some(layers) = self.face.color_layers(self.gid) {
            return Some(ColorInk::Layers(layers));
        }
        // Whole device pixels per em: rasters stay on the pixel grid and the cache stays small.
        let px = (self.size * scale).round().clamp(1.0, MAX_PX_PER_EM);
        let bitmap = self.face.color_bitmap(self.gid, &self.coords, px)?;
        let s = px / self.size;
        let place = self.warp * Affine::translate((self.x + bitmap.left / s, self.y + bitmap.top / s)) * Affine::scale(1.0 / s);
        Some(ColorInk::Bitmap(bitmap, place))
    }
}

#[derive(Clone, Debug)]
pub struct Line {
    /// Glyph index range.
    pub glyph_start: usize,
    pub glyph_end: usize,
    pub char_start: usize,
    pub char_end: usize,
    pub top: f64,
    pub height: f64,
    pub baseline: f64,
    pub width: f64,
    pub x: f64,
    /// The line ends a paragraph (hard break or end of text).
    pub para_end: bool,
    /// The paragraph runs right to left (its first strong character is right-to-left).
    pub rtl: bool,
}

/// A decoration line (underline/strikethrough) to paint with the style's fills.
#[derive(Clone, Debug)]
pub struct Decoration {
    /// Everything the line paints (a wavy line's band includes its swing).
    pub rect: Rect,
    pub style: usize,
    pub kind: DecorationStyle,
    pub thickness: f64,
}

/// Most dots or wave samples drawn for one decoration.
const MAX_DECO_STEPS: usize = 20_000;

impl Decoration {
    /// The line's outline: a bar, a row of dots or a wave, `thickness` thick.
    pub fn path(&self) -> BezPath {
        use figcraft_geom::kurbo::Shape;
        let (r, th) = (self.rect, self.thickness.max(0.01));
        let cy = r.center().y;
        match self.kind {
            DecorationStyle::Solid => r.to_path(0.1),
            DecorationStyle::Dotted => {
                let mut p = BezPath::new();
                let mut x = r.x0 + th / 2.0;
                for _ in 0..MAX_DECO_STEPS {
                    if x > r.x1 - th / 2.0 + 1e-9 && !p.elements().is_empty() {
                        break;
                    }
                    p.extend(figcraft_geom::kurbo::Circle::new((x, cy), th / 2.0).to_path(0.05));
                    x += th * 2.0;
                }
                p
            }
            DecorationStyle::Wavy => {
                let (amp, period) = wave(th);
                let n = (((r.width() / period) * 8.0).ceil() as usize).clamp(2, MAX_DECO_STEPS);
                let at = |i: usize, side: f64| {
                    let x = r.x0 + r.width() * i as f64 / n as f64;
                    figcraft_geom::Point::new(x, cy + amp * (std::f64::consts::TAU * (x - r.x0) / period).sin() + side * th / 2.0)
                };
                let mut p = BezPath::new();
                p.move_to(at(0, -1.0));
                for i in 1..=n {
                    p.line_to(at(i, -1.0));
                }
                for i in (0..=n).rev() {
                    p.line_to(at(i, 1.0));
                }
                p.close_path();
                p
            }
        }
    }
}

/// A wavy line's swing either side of its centre and its wavelength, for a thickness.
fn wave(th: f64) -> (f64, f64) {
    let amp = th * 0.75 + 0.5;
    (amp, amp * 6.0)
}

/// A decoration length in px (`auto`: what the font says).
fn deco_len(m: DecorationLength, size: f64, auto: f64) -> f64 {
    let v = match m {
        DecorationLength::Auto => auto,
        DecorationLength::Pixels(v) => v,
        DecorationLength::Percent(p) => size * p / 100.0,
    };
    if v.is_finite() { v } else { auto }
}

/// Opening punctuation, which hangs outside a line's start.
fn opens(c: char) -> bool {
    matches!(c, '"' | '\'' | '\u{201C}' | '\u{2018}' | '\u{201E}' | '\u{201A}' | '\u{00AB}' | '\u{2039}' | '(' | '[' | '{' | '\u{00BF}' | '\u{00A1}')
}

/// Closing punctuation and stops, which hang outside a line's end.
fn closes(c: char) -> bool {
    matches!(
        c,
        '.' | ','
            | ':'
            | ';'
            | '!'
            | '?'
            | '"'
            | '\''
            | '\u{201D}'
            | '\u{2019}'
            | '\u{00BB}'
            | '\u{203A}'
            | ')'
            | ']'
            | '}'
            | '-'
            | '\u{2013}'
            | '\u{2014}'
            | '\u{2026}'
    )
}

#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub glyphs: Vec<Glyph>,
    pub lines: Vec<Line>,
    pub decorations: Vec<Decoration>,
    /// The distinct styles the glyphs refer to.
    pub styles: Vec<TextStyle>,
    /// Size of the laid-out content.
    pub width: f64,
    pub height: f64,
    /// Whether text was cut off (truncation).
    pub truncated: bool,
    /// Where the text sits on its path (text on a path).
    pub on_path: Option<crate::onpath::OnPath>,
}

/// Ratio of the "Auto" line height to the font size (font ascent+descent+gap, per face).
fn natural_ratio(face: &Face) -> f64 {
    ((face.ascent + face.descent + face.line_gap.max(0.0)) / face.upem).max(0.5)
}

/// A line's height: whole pixels (halves round up), whatever unit it was given in. Heights under a
/// pixel stay as they are, so tiny text keeps its lines apart.
fn whole_px(line_height: f64) -> f64 {
    if line_height >= 1.0 { (line_height + 0.5).floor() } else { line_height }
}

/// Advances sit on a grid of this many steps per em, then scale with the font size.
const ADVANCE_GRID: f64 = 1024.0;

/// `units` (font units) on the advance grid, in ems.
fn on_grid(units: f64, upem: f64) -> f64 {
    (units * ADVANCE_GRID / upem).round() / ADVANCE_GRID
}

struct Item {
    face: Arc<Face>,
    coords: Coords,
    /// Glyph scale relative to the style's size (synthesised small caps).
    scale: f64,
    gid: u32,
    adv: f64,
    /// The letter spacing included in `adv` (none after a line's last visible glyph).
    spacing: f64,
    dx: f64,
    dy: f64,
    char_start: usize,
    char_end: usize,
    style: usize,
    ch: char,
    /// Bidi embedding level (odd = right to left).
    level: u8,
}

impl Item {
    fn is_space(&self) -> bool {
        crate::breaks::is_space(self.ch)
    }
}

/// Bidi embedding levels per char, and whether each char's paragraph runs right to left.
pub(crate) struct Bidi {
    pub(crate) levels: Vec<u8>,
    pub(crate) rtl_para: Vec<bool>,
}

impl Bidi {
    /// None when the text has nothing right to left (the common case costs one scan).
    pub(crate) fn of(text: &str) -> Option<Bidi> {
        if !text.chars().any(is_rtl_char) {
            return None;
        }
        let info = unicode_bidi::BidiInfo::new(text, None);
        let mut paras = info.paragraphs.iter().peekable();
        let n = text.chars().count();
        let (mut levels, mut rtl_para) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for (b, _) in text.char_indices() {
            while paras.peek().is_some_and(|p| p.range.end <= b) {
                paras.next();
            }
            levels.push(info.levels.get(b).map(|l| l.number()).unwrap_or(0));
            rtl_para.push(paras.peek().is_some_and(|p| p.level.is_rtl()));
        }
        Some(Bidi { levels, rtl_para })
    }
}

/// Characters of right-to-left scripts and the explicit right-to-left marks.
pub(crate) fn is_rtl_char(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF | 0x200F | 0x202B | 0x202E | 0x2067)
}

/// Visual order of a line's items from their levels (rule L2: from the highest level down to the
/// lowest odd one, reverse every run at that level or above).
pub(crate) fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(lowest_odd) = levels.iter().copied().filter(|l| l % 2 == 1).min() else { return order };
    let highest = levels.iter().copied().max().unwrap_or(0);
    let level = |order: &[usize], i: usize| order.get(i).and_then(|k| levels.get(*k)).copied().unwrap_or(0);
    for lvl in (lowest_odd..=highest).rev() {
        let mut i = 0;
        while i < order.len() {
            if level(&order, i) < lvl {
                i += 1;
                continue;
            }
            let start = i;
            while i < order.len() && level(&order, i) >= lvl {
                i += 1;
            }
            if let Some(run) = order.get_mut(start..i) {
                run.reverse();
            }
        }
    }
    order
}

/// Lay out `text` for a box of `box_width` (used when the text wraps or aligns; for auto width
/// pass `None`). `box_height` is used for truncation only.
pub fn layout(text: &TextData, box_width: Option<f64>, box_height: Option<f64>) -> TextLayout {
    let chars: Vec<char> = text.characters.chars().collect();
    let spans = text.spans();
    let mut styles: Vec<TextStyle> = Vec::new();
    let mut span_style: Vec<usize> = Vec::with_capacity(spans.len());
    for (_, _, s) in &spans {
        let idx = match styles.iter().position(|x| x == *s) {
            Some(i) => i,
            None => {
                styles.push((*s).clone());
                styles.len() - 1
            }
        };
        span_style.push(idx);
    }
    let mut dbw = db().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut fallbacks = dbw.fallbacks();
    let mut faces: Vec<Option<Arc<Face>>> = Vec::with_capacity(styles.len());
    let mut style_fallbacks: Vec<Vec<Arc<Face>>> = Vec::with_capacity(styles.len());
    for s in &styles {
        faces.push(dbw.find(&s.font_family, &s.font_style).or_else(|| dbw.default_face()));
        style_fallbacks.push(dbw.fallbacks_for(&s.font_style));
    }
    // Colour emoji are their own, large faces (Apple Color Emoji is ~180 MB): they are read only
    // for a text with emoji, or with characters nothing above covers.
    let covered = |c: char| c == '\n' || faces.iter().flatten().any(|f| f.has_char(c)) || fallbacks.iter().any(|f| f.has_char(c));
    let emoji = if chars.iter().any(|c| is_emoji(*c) || !covered(*c)) { dbw.emoji_fallbacks() } else { Vec::new() };
    fallbacks.extend(emoji.iter().cloned());
    drop(dbw);

    // OpenType features the user toggled (global for the node).
    let features: Vec<Feature> = text
        .features
        .iter()
        .filter(|(k, _)| valid_feature_tag(k))
        .filter_map(|(k, on)| <[u8; 4]>::try_from(k.as_bytes()).ok().map(|t| Feature::new(Tag::new(&t), u32::from(*on), ..)))
        .collect();

    // Shape each span, split by paragraph, bidi run and fallback face.
    let bidi = Bidi::of(&text.characters);
    let level_at = |i: usize| bidi.as_ref().and_then(|b| b.levels.get(i)).copied().unwrap_or(0);
    let rtl_para_at = |i: usize| bidi.as_ref().and_then(|b| b.rtl_para.get(i)).copied().unwrap_or(false);
    // A grapheme cluster (emoji sequence, letter with marks) is shaped with one face.
    let cluster_start = crate::breaks::grapheme_starts(&text.characters);
    let mut items: Vec<Item> = Vec::with_capacity(chars.len());
    for (si, (start, end, _)) in spans.iter().enumerate() {
        let st_idx = span_style.get(si).copied().unwrap_or(0);
        let Some(style) = styles.get(st_idx) else { continue };
        let Some(Some(primary)) = faces.get(st_idx) else { continue };
        let size = sanitize_size(style.font_size);
        let spacing = style.letter_spacing.resolve(size);
        // Segment by face.
        let mut seg_start = *start;
        while seg_start < *end {
            let face_for = |c: char| -> Arc<Face> {
                // Emoji take a colour face before a face without colour glyphs (the style's own or a
                // symbol font), which would draw them in one colour.
                if is_emoji(c)
                    && !primary.has_color()
                    && let Some(f) = emoji.iter().find(|f| f.has_char(c))
                {
                    return f.clone();
                }
                if c == '\n' || primary.has_char(c) {
                    return primary.clone();
                }
                let fallbacks = style_fallbacks.get(st_idx).unwrap_or(&fallbacks);
                fallbacks.iter().find(|f| f.has_char(c)).cloned().unwrap_or_else(|| primary.clone())
            };
            let first = chars.get(seg_start).copied().unwrap_or(' ');
            let face = face_for(first);
            let level = level_at(seg_start);
            let mut seg_end = seg_start + 1;
            while seg_end < *end
                && chars.get(seg_end).is_some_and(|c| {
                    *c != '\n' && first != '\n' && (!cluster_start.get(seg_end).copied().unwrap_or(true) || face_for(*c).id == face.id)
                })
                && level_at(seg_end) == level
            {
                seg_end += 1;
            }
            if first == '\n' {
                items.push(Item {
                    face: face.clone(),
                    coords: no_coords(),
                    scale: 1.0,
                    gid: 0,
                    adv: 0.0,
                    spacing: 0.0,
                    dx: 0.0,
                    dy: 0.0,
                    char_start: seg_start,
                    char_end: seg_start + 1,
                    style: st_idx,
                    ch: '\n',
                    level,
                });
            } else {
                let seg = Segment { start: seg_start, end: seg_end, style, size, spacing, st_idx, level };
                shape_segment(&chars, &seg, &face, &features, &mut items);
            }
            seg_start = seg_end;
        }
    }

    // List markers sit in a gutter (one more per nesting level) and wrapped lines hang at the
    // text start; a hanging list puts the outermost gutter outside the box. Paragraph indent
    // applies to plain paragraphs only. Right-to-left paragraphs mirror both: the gutter and the
    // indent are on the right, the text ends where they start.
    let lists: Vec<ParaList> = (0..text.paragraph_count()).map(|p| text.para_list(p)).collect();
    let markers = list_markers(&lists);
    let step = list_step(&lists, &markers, styles.first(), faces.first().cloned().flatten().as_ref());
    let hang_list = if text.hanging_list { step } else { 0.0 };
    // Paragraph indent applies to start-aligned text only (left, or right for a right-to-left paragraph).
    let indent_px = if text.paragraph_indent.is_finite() { text.paragraph_indent.clamp(0.0, 10_000.0) } else { 0.0 };
    let indent_at = |char_idx: usize| {
        let start_aligned = if rtl_para_at(char_idx) { text.align_h == TextAlignH::Right } else { text.align_h == TextAlignH::Left };
        if start_aligned { indent_px } else { 0.0 }
    };
    let list_of = |p: usize| lists.get(p).copied().unwrap_or_default();
    // The start indent of a line of paragraph `p` (`first`: its first line, starting at char `ci`).
    let line_indent_of = |p: usize, first: bool, ci: usize| {
        let pl = list_of(p);
        match pl.list {
            ListStyle::None if first => indent_at(ci),
            ListStyle::None => 0.0,
            _ => step * (f64::from(pl.indent) + 1.0) - hang_list,
        }
    };
    // Hanging punctuation: at the start edge of left-aligned and justified lines, at the end edge
    // of right-aligned and justified ones (left-to-right paragraphs).
    let hang_start = text.hanging_punctuation && matches!(text.align_h, TextAlignH::Left | TextAlignH::Justified);
    let hang_end = text.hanging_punctuation && matches!(text.align_h, TextAlignH::Right | TextAlignH::Justified);

    // Line breaking.
    let wrap = match text.auto_resize {
        TextAutoResize::WidthAndHeight => None,
        _ => box_width.filter(|w| w.is_finite() && *w > 0.0),
    };
    let mut lines: Vec<(usize, usize, bool, usize)> = Vec::new(); // item range, paragraph end, paragraph
    let mut line_start = 0;
    let mut i = 0;
    let mut x = 0.0;
    let mut last_break: Option<usize> = None;
    let mut para_first_line = true;
    let mut para = 0;
    while i < items.len() {
        let Some(it) = items.get(i) else { break };
        if it.ch == '\n' {
            lines.push((line_start, i + 1, true, para));
            i += 1;
            line_start = i;
            x = 0.0;
            last_break = None;
            para_first_line = true;
            para += 1;
            continue;
        }
        let line_indent = line_indent_of(para, para_first_line, it.char_start);
        let w = it.adv;
        // Hanging punctuation takes no room inside the box.
        let ltr = !rtl_para_at(it.char_start);
        let hung = items.get(line_start).filter(|f| hang_start && ltr && opens(f.ch)).map_or(0.0, |f| f.adv)
            + if hang_end && ltr && closes(it.ch) { w } else { 0.0 };
        if let Some(maxw) = wrap
            && x + w - it.spacing > maxw - line_indent + hung + 1e-6
            && i > line_start
            && !it.is_space()
        {
            let brk = last_break.filter(|b| *b >= line_start).map(|b| b + 1).unwrap_or(i);
            lines.push((line_start, brk, false, para));
            line_start = brk;
            i = brk;
            x = 0.0;
            last_break = None;
            para_first_line = false;
            continue;
        }
        x += w;
        if items.get(i + 1).is_some_and(|next| crate::breaks::break_between(it.ch, next.ch)) {
            last_break = Some(i);
        }
        i += 1;
    }
    if line_start < items.len() || lines.is_empty() || lines.last().is_some_and(|l| l.2) {
        lines.push((line_start, items.len(), true, para));
    }

    // Line metrics and positions.
    let default_style = styles.first().cloned().unwrap_or_default();
    let default_face = faces.first().cloned().flatten();
    let mut out = TextLayout { styles: styles.clone(), ..Default::default() };
    let mut y = 0.0;
    let mut para_start_line = true;
    let max_lines = if text.max_lines > 0 && matches!(text.auto_resize, TextAutoResize::Truncate) { text.max_lines as usize } else { usize::MAX };
    // A style's vertical metrics (ascent, descent, line height, cap height) come from its own
    // font, never a fallback face, so lines mixing scripts keep the spacing of lines that don't.
    let metrics = |st_idx: usize| -> [f64; 4] {
        let st = styles.get(st_idx).unwrap_or(&default_style);
        let size = sanitize_size(st.font_size);
        match faces.get(st_idx).and_then(Option::as_ref).or(default_face.as_ref()) {
            Some(f) => {
                let k = size / f.upem;
                [f.ascent * k, f.descent * k, whole_px(st.line_height.resolve(size, natural_ratio(f))), f.cap_height * k]
            }
            None => [size * 0.8, size * 0.2, whole_px(st.line_height.resolve(size, 1.2)), size * 0.7],
        }
    };
    let mut first_cap = 0.0;
    for (li, (s, e, para_end, para)) in lines.iter().enumerate() {
        if li >= max_lines {
            out.truncated = true;
            break;
        }
        // Metrics from the styles present on the line (or the style at the break for empty lines).
        let [mut asc, mut desc, mut lh, mut cap] = [0.0_f64; 4];
        let mut seen = usize::MAX;
        for st in items.get(*s..*e).unwrap_or(&[]).iter().map(|it| it.style) {
            if st != seen {
                seen = st;
                let m = metrics(st);
                (asc, desc, lh, cap) = (asc.max(m[0]), desc.max(m[1]), lh.max(m[2]), cap.max(m[3]));
            }
        }
        if seen == usize::MAX {
            [asc, desc, lh, cap] = metrics(items.get(s.saturating_sub(1)).map_or(0, |it| it.style));
        }
        if li == 0 {
            first_cap = cap;
        }
        if para_start_line && li > 0 {
            // Between two items of a list: the list spacing.
            let in_list = |p: usize| list_of(p).list != ListStyle::None;
            let gap = if in_list(*para) && para.checked_sub(1).is_some_and(in_list) { text.list_spacing } else { text.paragraph_spacing };
            y += if gap.is_finite() { gap.clamp(0.0, 10_000.0) } else { 0.0 };
        }
        let baseline = y + (lh - (asc + desc)) / 2.0 + asc;
        // Trailing spaces don't count for width/alignment.
        let mut vis_end = *e;
        while vis_end > *s && items.get(vis_end - 1).is_some_and(|it| it.is_space() || it.ch == '\n') {
            vis_end -= 1;
        }
        // Letter spacing separates glyphs: none after the line's last visible one.
        let end_spacing = vis_end.checked_sub(1).filter(|i| *i >= *s).and_then(|i| items.get(i)).map_or(0.0, |it| it.spacing);
        let width: f64 = items.get(*s..vis_end).unwrap_or(&[]).iter().map(|it| it.adv).sum::<f64>() - end_spacing;
        let first_char = items.get(*s).map(|it| it.char_start).unwrap_or(0);
        let rtl_line = rtl_para_at(first_char);
        let line_indent = line_indent_of(*para, para_start_line, first_char);
        // Hanging punctuation at the line's edges (the visible text's first and last chars).
        let hang_l = items.get(*s).filter(|it| hang_start && !rtl_line && vis_end > *s && opens(it.ch)).map_or(0.0, |it| it.adv);
        let hang_r =
            items.get(vis_end.wrapping_sub(1)).filter(|it| hang_end && !rtl_line && vis_end > *s + 1 && closes(it.ch)).map_or(0.0, |it| it.adv);
        let gstart = out.glyphs.len();
        // Justify: extra space per gap.
        let gaps = items.get(*s..vis_end).unwrap_or(&[]).iter().filter(|it| it.is_space()).count();
        let justify_extra = match (text.align_h, wrap) {
            (TextAlignH::Justified, Some(maxw)) if !para_end && gaps > 0 => ((maxw - line_indent + hang_l + hang_r - width) / gaps as f64).max(0.0),
            _ => 0.0,
        };
        // Where the visible text ends once justified.
        let text_end = width + justify_extra * gaps as f64;
        // A right-to-left line's trailing spaces sit at its visual left (rule L1, below) and hang
        // outside it, so its visible text spans [0, text_end] with the gutter and indent after it.
        let trailing: f64 = items
            .get(vis_end..*e)
            .unwrap_or(&[])
            .iter()
            .filter(|it| it.ch != '\n')
            .map(|it| it.adv + if it.is_space() { justify_extra } else { 0.0 })
            .sum();
        let mut pen = if rtl_line { -trailing } else { line_indent - hang_l };
        // List marker (before the line's text; zero-length char range so carets skip it).
        let pl = list_of(*para);
        if para_start_line && pl.list != ListStyle::None {
            let marker = markers.get(*para).cloned().unwrap_or_default();
            let st_idx = items.get(*s).map(|it| it.style).unwrap_or(0);
            let at = items.get(*s).map(|it| it.char_start).unwrap_or(chars.len());
            // The marker takes the paragraph style's own font (not a fallback the line's first
            // char needed, e.g. for Hebrew), so every bullet and number of a list matches; a
            // nested bullet the font lacks comes from a fallback font.
            let has = |f: &Arc<Face>| marker.chars().all(|c| f.has_char(c));
            let own = faces.get(st_idx).cloned().flatten().filter(has);
            let marker_face = match (own, styles.get(st_idx)) {
                (Some(f), Some(st)) => {
                    let coords = f.coords(&f.axis_values(st));
                    Some((f, coords))
                }
                _ => fallbacks
                    .iter()
                    .find(|f| has(f))
                    .map(|f| (f.clone(), no_coords()))
                    .or_else(|| items.get(*s).map(|it| (it.face.clone(), it.coords.clone())))
                    .or_else(|| default_face.clone().map(|f| (f, no_coords()))),
            };
            if let (Some(st), Some((face, coords))) = (styles.get(st_idx), marker_face) {
                let size = sanitize_size(st.font_size);
                let k = size / face.upem;
                let numbered = pl.list == ListStyle::Numbered;
                // Right to left, the number reads "1." with its full stop on the left (".1").
                let marker = if rtl_line && numbered { format!(".{}", marker.trim_end_matches('.')) } else { marker };
                let advance = marker_advance(&face, &coords, &marker, size);
                // Numbers end (start, right to left) half an em from the text; bullets sit at the
                // gutter's outer edge.
                let gutter = step * f64::from(pl.indent) - hang_list;
                let mut mx = match (rtl_line, numbered) {
                    (false, true) => gutter + (step - size * 0.5 - advance).max(0.0),
                    (false, false) => gutter,
                    (true, true) => text_end + size * 0.5,
                    (true, false) => text_end + (step - advance).max(0.0),
                };
                for c in marker.chars() {
                    let gid = face.glyph_for(c);
                    let adv = face.advance_at(gid, &coords) * k;
                    out.glyphs.push(Glyph {
                        face: face.clone(),
                        coords: coords.clone(),
                        gid,
                        x: mx,
                        y: baseline,
                        size,
                        advance: adv,
                        char_start: at,
                        char_end: at,
                        style: st_idx,
                        rtl: false,
                        warp: Affine::IDENTITY,
                    });
                    mx += adv;
                }
            }
        }
        // Right-to-left runs are placed in visual order; trailing spaces take the paragraph's
        // direction (rule L1).
        let line_items = items.get(*s..*e).unwrap_or(&[]);
        let order: Vec<usize> = if bidi.is_some() {
            let para_level = u8::from(rtl_line);
            let levels: Vec<u8> = line_items.iter().enumerate().map(|(k, it)| if *s + k >= vis_end { para_level } else { it.level }).collect();
            visual_order(&levels)
        } else {
            (0..line_items.len()).collect()
        };
        for it in order.iter().filter_map(|k| line_items.get(*k)) {
            if it.ch == '\n' {
                continue;
            }
            let size = styles.get(it.style).map(|st| sanitize_size(st.font_size)).unwrap_or(12.0) * it.scale;
            if it.gid != 0 || !it.is_space() {
                out.glyphs.push(Glyph {
                    face: it.face.clone(),
                    coords: it.coords.clone(),
                    gid: it.gid,
                    x: pen + it.dx,
                    y: baseline - it.dy,
                    size,
                    advance: it.adv,
                    char_start: it.char_start,
                    char_end: it.char_end,
                    style: it.style,
                    rtl: it.level % 2 == 1,
                    warp: Affine::IDENTITY,
                });
            }
            pen += it.adv;
            if it.is_space() {
                pen += justify_extra;
            }
        }
        let char_start = items.get(*s).map(|it| it.char_start).unwrap_or_else(|| chars.len());
        let char_end = items.get(e.saturating_sub(1)).map(|it| it.char_end).unwrap_or(char_start).max(char_start);
        out.lines.push(Line {
            glyph_start: gstart,
            glyph_end: out.glyphs.len(),
            char_start,
            char_end,
            top: y,
            height: lh,
            baseline,
            width: width + line_indent - hang_l - hang_r,
            x: 0.0,
            para_end: *para_end,
            rtl: rtl_line,
        });
        y += lh;
        para_start_line = *para_end;
    }
    out.width = out.lines.iter().map(|l| l.width).fold(0.0, f64::max);
    // Vertical trim: the box starts at the first line's cap height and ends at the last baseline.
    let trim = text.leading_trim == LeadingTrim::CapHeight && text.path.is_none();
    if trim && let Some(first) = out.lines.first() {
        let shift = first.baseline - first_cap;
        for l in &mut out.lines {
            l.top -= shift;
            l.baseline -= shift;
        }
        for g in &mut out.glyphs {
            g.y -= shift;
        }
    }
    let bottom = |l: &Line| if trim { l.baseline } else { l.top + l.height };
    out.height = out.lines.last().map_or(y, bottom).max(0.0);

    // Truncation by height.
    if matches!(text.auto_resize, TextAutoResize::Truncate)
        && let Some(h) = box_height
        && out.height > h + 0.5
    {
        let keep = out.lines.iter().take_while(|l| bottom(l) <= h + 0.5).count().max(1);
        if keep < out.lines.len() {
            out.lines.truncate(keep);
            let gend = out.lines.last().map(|l| l.glyph_end).unwrap_or(0);
            out.glyphs.truncate(gend);
            out.truncated = true;
            out.height = out.lines.last().map_or(0.0, bottom);
        }
    }
    if out.truncated {
        add_ellipsis(&mut out, wrap);
    }

    // Horizontal alignment within the box.
    let align_width = match text.auto_resize {
        TextAutoResize::WidthAndHeight => out.width,
        _ => box_width.unwrap_or(out.width),
    };
    for li in 0..out.lines.len() {
        let Some(line) = out.lines.get(li) else { continue };
        let dx = match text.align_h {
            TextAlignH::Left | TextAlignH::Justified => 0.0,
            TextAlignH::Center => (align_width - line.width) / 2.0,
            TextAlignH::Right => align_width - line.width,
        };
        if dx != 0.0 {
            let (gs, ge) = (line.glyph_start, line.glyph_end);
            for g in out.glyphs.get_mut(gs..ge).unwrap_or(&mut []) {
                g.x += dx;
            }
            if let Some(l) = out.lines.get_mut(li) {
                l.x = dx;
            }
        }
    }

    // Decorations per contiguous glyph run of the same style on a line; a decoration colour
    // paints with a style of its own, added after the glyphs' styles.
    let mut decos = Vec::new();
    let mut deco_styles: Vec<TextStyle> = Vec::new();
    for line in &out.lines {
        let gl = out.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]);
        let mut j = 0;
        while j < gl.len() {
            let Some(g0) = gl.get(j) else { break };
            // List markers and the ellipsis are not decorated.
            if g0.char_end == g0.char_start {
                j += 1;
                continue;
            }
            let st = out.styles.get(g0.style).cloned().unwrap_or_default();
            let mut k = j + 1;
            while gl.get(k).is_some_and(|g| g.style == g0.style && g.char_end > g.char_start) {
                k += 1;
            }
            if st.decoration != TextDecoration::None
                && let Some(last) = gl.get(k - 1)
            {
                let x0 = g0.x;
                let x1 = last.x + last.advance;
                // The style's own font and size (not a fallback face or a small-cap scale), so the
                // line runs at one height and weight through mixed scripts.
                let f = faces.get(g0.style).and_then(Option::as_ref).unwrap_or(&g0.face);
                let size = sanitize_size(st.font_size);
                let s = size / f.upem;
                let look = st.look();
                let th = deco_len(look.thickness, size, (f.underline_thickness * s).max(0.5)).clamp(0.1, size);
                let underline = st.decoration == TextDecoration::Underline;
                // The line's top edge (an underline's own offset is measured from the baseline).
                let top = match look.offset {
                    DecorationLength::Auto => line.baseline - if underline { f.underline_pos } else { f.strike_pos } * s - th / 2.0,
                    m if underline => line.baseline + deco_len(m, size, 0.0).clamp(-size, size),
                    _ => line.baseline - f.strike_pos * s - th / 2.0,
                };
                let style = match look.color {
                    Some(c) => {
                        let own = TextStyle { fills: Some(vec![figcraft_doc::color::Paint::solid(c)]), ..st.clone() };
                        let k = deco_styles.iter().position(|d| *d == own).unwrap_or_else(|| {
                            deco_styles.push(own);
                            deco_styles.len() - 1
                        });
                        out.styles.len() + k
                    }
                    None => g0.style,
                };
                let swing = if look.style == DecorationStyle::Wavy { wave(th).0 } else { 0.0 };
                // Skip ink: the underline stops a little before each glyph crossing it.
                let gap = th.max(1.0);
                let cuts = if underline && look.skip_ink {
                    ink_cuts(gl.get(j..k).unwrap_or(&[]), top - gap / 2.0 - swing, top + th + gap / 2.0 + swing, gap)
                } else {
                    Vec::new()
                };
                let mut from = x0;
                for (c0, c1) in cuts.into_iter().chain([(x1, x1)]) {
                    if c0.min(x1) > from + 0.5 {
                        let rect = Rect::new(from, top - swing, c0.min(x1), top + th + swing);
                        decos.push(Decoration { rect, style, kind: look.style, thickness: th });
                    }
                    from = from.max(c1);
                }
            }
            j = k;
        }
    }
    out.decorations = decos;
    out.styles.extend(deco_styles);
    if let Some(tp) = &text.path {
        crate::onpath::place(&mut out, tp, text.align_h);
    }
    out
}

/// The markers of a text's paragraphs ("" for plain ones). Numbers count the items of one list at
/// one level, restarting after a plain paragraph, a shallower item or the other kind of list,
/// from the first item's start number; levels go 1. a. i. and • ◦ ▪, then round again.
pub(crate) fn list_markers(lists: &[ParaList]) -> Vec<String> {
    const BULLETS: [&str; 3] = ["\u{2022}", "\u{25E6}", "\u{25AA}"];
    let mut count = [(ListStyle::None, 0usize); figcraft_doc::MAX_LIST_INDENT as usize + 1];
    lists
        .iter()
        .map(|pl| {
            if pl.list == ListStyle::None {
                count.fill((ListStyle::None, 0));
                return String::new();
            }
            let d = usize::from(pl.indent).min(count.len() - 1);
            for c in count.iter_mut().skip(d + 1) {
                *c = (ListStyle::None, 0);
            }
            let Some(c) = count.get_mut(d) else { return String::new() };
            let first = usize::try_from(pl.start_offset).unwrap_or(usize::MAX).saturating_add(1);
            *c = if c.0 == pl.list { (pl.list, c.1.saturating_add(1)) } else { (pl.list, first) };
            match (pl.list, d % 3) {
                (ListStyle::Numbered, 1) => format!("{}.", alpha(c.1)),
                (ListStyle::Numbered, 2) => format!("{}.", roman(c.1)),
                (ListStyle::Numbered, _) => format!("{}.", c.1),
                _ => BULLETS.get(d % 3).copied().unwrap_or("\u{2022}").to_string(),
            }
        })
        .collect()
}

/// a, b, ... z, aa, ab, ...
fn alpha(mut n: usize) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(char::from(b'a' + (n % 26) as u8));
        n /= 26;
    }
    out.iter().rev().collect()
}

/// Lower-case roman numerals (past 3999: the number).
fn roman(mut n: usize) -> String {
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    let mut out = String::new();
    for (v, r) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while n >= v {
            out.push_str(r);
            n -= v;
        }
    }
    out
}

/// The width of one list level's gutter: room for the widest number plus half an em, at least one
/// and a half ems (0 without lists).
fn list_step(lists: &[ParaList], markers: &[String], style: Option<&TextStyle>, face: Option<&Arc<Face>>) -> f64 {
    if lists.iter().all(|l| l.list == ListStyle::None) {
        return 0.0;
    }
    let size = style.map(|s| sanitize_size(s.font_size)).unwrap_or(12.0);
    let widest = lists
        .iter()
        .zip(markers)
        .filter(|(l, _)| l.list == ListStyle::Numbered)
        .map(|(_, m)| face.map(|f| marker_advance(f, &no_coords(), m, size)).unwrap_or(size * 0.6 * m.chars().count() as f64))
        .fold(0.0, f64::max);
    (widest + size * 0.5).max(size * 1.5)
}

fn marker_advance(face: &Face, coords: &Coords, s: &str, size: f64) -> f64 {
    let k = size / face.upem;
    s.chars().map(|c| face.advance_at(face.glyph_for(c), coords) * k).sum()
}

/// Where glyphs' outlines cross the band `y0..y1`: one x range per glyph, widened by `gap`.
fn ink_cuts(glyphs: &[Glyph], y0: f64, y1: f64, gap: f64) -> Vec<(f64, f64)> {
    let mut cuts: Vec<(f64, f64)> = Vec::new();
    for g in glyphs {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut prev: Option<figcraft_geom::Point> = None;
        figcraft_geom::kurbo::flatten(g.path().iter(), 0.25, |el| {
            use figcraft_geom::kurbo::PathEl;
            let p = match el {
                PathEl::MoveTo(p) => {
                    prev = Some(p);
                    return;
                }
                PathEl::LineTo(p) => p,
                _ => return,
            };
            if let Some(a) = prev {
                // The part of segment a-p inside the band.
                let (t0, t1) = if (p.y - a.y).abs() < 1e-12 {
                    if (y0..=y1).contains(&a.y) { (0.0, 1.0) } else { (1.0, 0.0) }
                } else {
                    let (ta, tb) = ((y0 - a.y) / (p.y - a.y), (y1 - a.y) / (p.y - a.y));
                    (ta.min(tb).max(0.0), ta.max(tb).min(1.0))
                };
                if t0 <= t1 {
                    for t in [t0, t1] {
                        let x = a.x + (p.x - a.x) * t;
                        (lo, hi) = (lo.min(x), hi.max(x));
                    }
                }
            }
            prev = Some(p);
        });
        if lo <= hi {
            cuts.push((lo - gap, hi + gap));
        }
    }
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    cuts
}

fn add_ellipsis(out: &mut TextLayout, wrap: Option<f64>) {
    let Some(line) = out.lines.last().cloned() else { return };
    let Some(last) = out.glyphs.get(line.glyph_end.saturating_sub(1)).cloned() else { return };
    let face = last.face.clone();
    let gid = face.glyph_for('…');
    let gid = if gid == 0 { face.glyph_for('.') } else { gid };
    let k = last.size / face.upem;
    let adv = face.advance_at(gid, &last.coords) * k;
    let limit = wrap.unwrap_or(f64::INFINITY);
    // Drop glyphs until the ellipsis fits, then any trailing spaces ("word…", not "word …").
    while out.glyphs.len() > line.glyph_start && out.glyphs.last().is_some_and(|g| g.x + g.advance + adv > limit) {
        out.glyphs.pop();
    }
    while out.glyphs.len() > line.glyph_start && out.glyphs.last().is_some_and(|g| g.char_end > g.char_start && g.gid == g.face.glyph_for(' ')) {
        out.glyphs.pop();
    }
    let x = out.glyphs.last().filter(|_| out.glyphs.len() > line.glyph_start).map(|g| g.x + g.advance).unwrap_or(0.0);
    out.glyphs.push(Glyph {
        face,
        coords: last.coords.clone(),
        gid,
        x,
        y: line.baseline,
        size: last.size,
        advance: adv,
        char_start: last.char_end,
        char_end: last.char_end,
        style: last.style,
        rtl: false,
        warp: Affine::IDENTITY,
    });
    if let Some(l) = out.lines.last_mut() {
        l.glyph_end = out.glyphs.len();
        l.width = x + adv;
    }
}

fn sanitize_size(s: f64) -> f64 {
    if s.is_finite() { s.clamp(0.5, 5000.0) } else { 12.0 }
}

/// A character drawn as a colour emoji by default: flags and the pictograph blocks.
/// shortcut: the older symbols (U+2600–U+27BF) keep their text look even before U+FE0F; read the
/// emoji presentation data if they should turn colour too.
fn is_emoji(c: char) -> bool {
    matches!(c, '\u{1F1E6}'..='\u{1F1FF}' | '\u{1F300}'..='\u{1FAFF}')
}

/// One run of chars shaped with a single face and style.
struct Segment<'a> {
    start: usize,
    end: usize,
    style: &'a TextStyle,
    size: f64,
    spacing: f64,
    st_idx: usize,
    /// Bidi level (odd: shaped right to left).
    level: u8,
}

/// Synthesised small caps: lower-case letters drawn as capitals at this fraction of the size.
const SMALL_CAP_SCALE: f64 = 0.72;

fn shape_segment(chars: &[char], seg: &Segment, face: &Arc<Face>, features: &[Feature], items: &mut Vec<Item>) {
    let (start, end, style) = (seg.start, seg.end, seg.style);
    let k = seg.size / face.upem;
    let small_caps = style.case == TextCase::SmallCaps;
    let native_smcp = small_caps && face.has_feature(b"smcp");
    // Map each source char through the case (title case looks at the char before the segment, so
    // a word split across styled runs is capitalised once) and remember synthesised small caps.
    let mut mapped: Vec<(char, usize, bool)> = Vec::with_capacity(end.saturating_sub(start));
    let mut prev = start.checked_sub(1).and_then(|i| chars.get(i)).copied();
    for (i, c) in chars.get(start..end).unwrap_or(&[]).iter().enumerate() {
        let src = start + i;
        if small_caps && !native_smcp && c.is_lowercase() {
            for u in c.to_uppercase() {
                mapped.push((u, src, true));
            }
        } else {
            let mut buf = [0u8; 4];
            for u in style.case.apply_after(c.encode_utf8(&mut buf), prev).chars() {
                mapped.push((u, src, false));
            }
        }
        prev = Some(*c);
    }
    let mut small = vec![false; end.saturating_sub(start)];
    for (_, src, sm) in &mapped {
        if *sm && let Some(f) = small.get_mut(src - start) {
            *f = true;
        }
    }
    let small_at = |src: usize| src.checked_sub(start).and_then(|i| small.get(i)).copied().unwrap_or(false);
    let mut feats: Vec<Feature> = features.to_vec();
    if native_smcp {
        feats.push(Feature::new(Tag::new(b"smcp"), 1, ..));
    }
    // Variable fonts: the axis values the style asks for (named instance, implied weight, custom).
    let values = face.axis_values(style);
    let coords = face.coords(&values);
    let mut raw: Vec<(u32, u32, i32, i32, i32)> = Vec::new();
    if let Some(hb) = face.hb() {
        let instance = (!coords.is_empty()).then(|| {
            let vars = face
                .axes
                .iter()
                .zip(&values)
                .filter_map(|(a, v)| <[u8; 4]>::try_from(a.tag.as_bytes()).ok().map(|t| Variation { tag: Tag::new(&t), value: *v as f32 }));
            ShaperInstance::from_variations(&hb, vars)
        });
        let shaper = face.shaper.shaper(&hb).instance(instance.as_ref()).build();
        let mut buf = UnicodeBuffer::new();
        for (u, src, _) in &mapped {
            buf.add(*u, *src as u32);
        }
        let rtl = seg.level % 2 == 1;
        buf.set_direction(if rtl { Direction::RightToLeft } else { Direction::LeftToRight });
        buf.guess_segment_properties();
        let gb = shaper.shape(buf, ShapeOptions::new().features(&feats));
        for (info, pos) in gb.glyph_infos().iter().zip(gb.glyph_positions()) {
            raw.push((info.glyph_id, info.cluster, pos.x_advance, pos.x_offset, pos.y_offset));
        }
        // Right-to-left glyphs come out in visual order; items are kept in logical order (lines
        // put runs back in visual order).
        if rtl {
            raw.reverse();
        }
    } else {
        for (u, src, _) in &mapped {
            let gid = face.glyph_for(*u);
            raw.push((gid, *src as u32, face.advance_at(gid, &coords) as i32, 0, 0));
        }
    }
    let n = raw.len();
    for gi in 0..n {
        let Some(&(gid, cl, xa, xo, yo)) = raw.get(gi) else { continue };
        let cl = cl as usize;
        // Cluster end: the next larger cluster value, else the segment end.
        let cend = raw.iter().skip(gi + 1).map(|r| r.1 as usize).find(|c| *c > cl).unwrap_or(end).max(cl + 1);
        let last_in_cluster = raw.get(gi + 1).is_none_or(|r| r.1 as usize != cl);
        let ch = chars.get(cl).copied().unwrap_or(' ');
        let scale = if small_at(cl) { SMALL_CAP_SCALE } else { 1.0 };
        let spacing = if last_in_cluster { seg.spacing * (cend - cl) as f64 } else { 0.0 };
        // The glyph's own advance and what shaping added to it (kerning) each land on the grid.
        let own = face.advance_at(gid, &coords);
        let adv = (on_grid(own, face.upem) + on_grid(f64::from(xa) - own, face.upem)) * seg.size * scale + spacing;
        items.push(Item {
            face: face.clone(),
            coords: coords.clone(),
            scale,
            gid,
            adv,
            spacing,
            dx: f64::from(xo) * k * scale,
            dy: f64::from(yo) * k * scale,
            char_start: cl,
            char_end: cend,
            style: seg.st_idx,
            ch,
            level: seg.level,
        });
    }
}

impl TextLayout {
    /// A box (layout coordinates) holding everything the text paints: each glyph's advance box,
    /// grown by half its size for ascenders, descenders and overhangs, plus decorations. None when
    /// nothing is laid out.
    pub fn ink_bounds(&self) -> Option<Rect> {
        let lines = self.lines.iter().map(|l| (l, self.glyphs.get(l.glyph_start..l.glyph_end).unwrap_or(&[])));
        let glyphs = lines.flat_map(|(l, gs)| {
            gs.iter().map(move |g| {
                let pad = g.size.abs() * 0.5;
                let r = Rect::new(g.x.min(g.x + g.advance) - pad, l.top - pad, g.x.max(g.x + g.advance) + pad, l.top + l.height + pad);
                figcraft_geom::transform_rect_bbox(g.warp, r)
            })
        });
        glyphs.chain(self.decorations.iter().map(|d| d.rect)).filter(|r| r.x0.is_finite() && r.y1.is_finite()).reduce(|a, b| a.union(b))
    }

    /// The caret position (x, line index) before char `index`.
    pub fn caret(&self, index: usize) -> (f64, usize) {
        for (li, line) in self.lines.iter().enumerate() {
            let last_line = li + 1 == self.lines.len();
            let in_line = index >= line.char_start && (index < line.char_end || (last_line && index <= line.char_end));
            // A caret at the end of a soft-wrapped line belongs to the next line.
            if !(in_line || index == line.char_end && line.para_end && !last_line) {
                continue;
            }
            if index == line.char_end && line.para_end && !last_line {
                // Before the newline: end of this line.
                let x = self.line_end_x(line);
                return (x, li);
            }
            let gl = self.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]);
            for g in gl {
                if g.char_end == g.char_start {
                    continue;
                }
                if index >= g.char_start && index < g.char_end {
                    let frac = (index - g.char_start) as f64 / (g.char_end - g.char_start) as f64;
                    let frac = if g.rtl { 1.0 - frac } else { frac };
                    return (g.x + g.advance * frac, li);
                }
            }
            return (self.line_end_x(line), li);
        }
        let li = self.lines.len().saturating_sub(1);
        (self.lines.last().map(|l| self.line_end_x(l)).unwrap_or(0.0), li)
    }

    /// The caret one step left or right of `index` on screen, for lines with right-to-left runs
    /// (None on plain left-to-right lines: there a step is one char). At the line's edge it moves
    /// on logically, to the neighbouring line.
    pub fn visual_step(&self, index: usize, right: bool) -> Option<usize> {
        let (x, li) = self.caret(index);
        let line = self.lines.get(li)?;
        let gl = self.glyphs.get(line.glyph_start..line.glyph_end)?;
        if !line.rtl && !gl.iter().any(|g| g.rtl) {
            return None;
        }
        // The caret can't sit after a paragraph's newline on its own line.
        let end = if line.para_end && li + 1 < self.lines.len() { line.char_end.saturating_sub(1) } else { line.char_end };
        let next = (line.char_start..=end.max(line.char_start))
            .filter(|i| *i != index)
            .map(|i| (i, self.caret(i).0))
            .filter(|(_, cx)| if right { *cx > x + 0.01 } else { *cx < x - 0.01 })
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()).then(a.0.cmp(&b.0)))
            .map(|(i, _)| i);
        // At the edge: on to the next line in reading order.
        Some(next.unwrap_or(if right != line.rtl { index + 1 } else { index.saturating_sub(1) }))
    }

    /// Left edge of a left-to-right line's first char (its end when it has none).
    fn line_start_x(&self, line: &Line) -> f64 {
        let gl = self.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]);
        gl.iter().find(|g| g.char_end > g.char_start).map_or_else(|| self.line_end_x(line), |g| g.x)
    }

    /// Where a line's text ends: its right edge, or its left edge for a right-to-left paragraph.
    fn line_end_x(&self, line: &Line) -> f64 {
        let glyphs = self.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]).iter().filter(|g| g.char_end > g.char_start);
        if line.rtl {
            glyphs.map(|g| g.x).fold(f64::INFINITY, f64::min).min(line.x + line.width)
        } else {
            glyphs.map(|g| g.x + g.advance).fold(line.x, f64::max)
        }
    }

    /// Layout point `(x, y)` where it shows: moved onto the path for text on a path.
    pub fn warp_point(&self, x: f64, y: f64) -> figcraft_geom::Point {
        let p = figcraft_geom::Point::new(x, y);
        let Some(on) = &self.on_path else { return p };
        let baseline = self.lines.iter().find(|l| y <= l.top + l.height).or(self.lines.last()).map_or(y, |l| l.baseline);
        on.frame(x, baseline).map_or(p, |f| f * p)
    }

    /// Text on a path: the char nearest to where `(x, y)` shows, by the glyph it falls nearest to.
    fn hit_on_path(&self, x: f64, y: f64) -> usize {
        let p = figcraft_geom::Point::new(x, y);
        let near = self.glyphs.iter().filter(|g| g.char_end > g.char_start && g.warp.determinant().abs() > 1e-12).map(|g| {
            let q = g.warp.inverse() * p;
            let mid = g.x + g.advance / 2.0;
            ((q - figcraft_geom::Point::new(mid, g.y - g.size / 3.0)).hypot(), if q.x < mid { g.char_start } else { g.char_end })
        });
        near.min_by(|a, b| a.0.total_cmp(&b.0)).map_or(0, |(_, i)| i)
    }

    /// The char index nearest to `(x, y)` (layout coordinates).
    pub fn hit(&self, x: f64, y: f64) -> usize {
        if self.on_path.is_some() {
            return self.hit_on_path(x, y);
        }
        let Some((li, line)) =
            self.lines.iter().enumerate().find(|(_, l)| y < l.top + l.height).or_else(|| self.lines.iter().enumerate().next_back())
        else {
            return 0;
        };
        let gl = self.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]);
        for g in gl {
            if g.char_end <= g.char_start {
                continue;
            }
            if x < g.x + g.advance / 2.0 {
                // A right-to-left glyph's left half is after it.
                return if g.rtl { g.char_end } else { g.char_start };
            }
        }
        let last_line = li + 1 == self.lines.len();
        let end = if line.para_end && !last_line { line.char_end.saturating_sub(1).max(line.char_start) } else { line.char_end };
        // Right of a right-to-left line is its start.
        if line.rtl { line.char_start } else { end }
    }

    /// Selection rectangles for chars `start..end`.
    pub fn selection_rects(&self, start: usize, end: usize) -> Vec<Rect> {
        let (a, b) = (start.min(end), start.max(end));
        let mut out = Vec::new();
        for (li, line) in self.lines.iter().enumerate() {
            let last = li + 1 == self.lines.len();
            // The empty line after a trailing break has no chars, but a selection reaching the
            // end of the text covers it: it shows a paragraph-break mark at the line's start.
            let trailing_empty = last && a < b && b >= line.char_start && line.char_start == line.char_end;
            if (b <= line.char_start && !trailing_empty) || a > line.char_end || (a == line.char_end && !last) {
                continue;
            }
            if trailing_empty {
                let x = self.line_end_x(line);
                out.push(Rect::new(x, line.top, x + 4.0, line.top + line.height));
                continue;
            }
            let gl = self.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or(&[]);
            if line.rtl || gl.iter().any(|g| g.rtl) {
                out.extend(Self::mixed_selection(gl, a, b, line));
                continue;
            }
            // From the line's own start: `caret(char_start)` after a hard break is the end of the
            // line above.
            let x0 = if a <= line.char_start { self.line_start_x(line) } else { self.caret(a).0 };
            let x1 = if b >= line.char_end {
                // The last line has no paragraph break to show, so it ends at its glyphs.
                self.line_end_x(line) + if line.para_end && !last && b > line.char_end.saturating_sub(1) { 4.0 } else { 0.0 }
            } else {
                self.caret(b).0
            };
            out.push(Rect::new(x0, line.top, x1.max(x0 + 1.0), line.top + line.height));
        }
        out
    }

    /// Selection rectangles on a line with right-to-left runs: the selected part of each glyph
    /// (they're in visual order), merged where they touch.
    fn mixed_selection(gl: &[Glyph], a: usize, b: usize, line: &Line) -> Vec<Rect> {
        let mut spans: Vec<(f64, f64)> = Vec::new();
        for g in gl.iter().filter(|g| g.char_end > g.char_start) {
            let (ga, gb) = (g.char_start.max(a), g.char_end.min(b));
            if ga >= gb {
                continue;
            }
            let n = (g.char_end - g.char_start) as f64;
            let (f0, f1) = ((ga - g.char_start) as f64 / n, (gb - g.char_start) as f64 / n);
            let (x0, x1) =
                if g.rtl { (g.x + g.advance * (1.0 - f1), g.x + g.advance * (1.0 - f0)) } else { (g.x + g.advance * f0, g.x + g.advance * f1) };
            match spans.last_mut() {
                Some(last) if (x0 - last.1).abs() < 0.5 => last.1 = last.1.max(x1),
                _ => spans.push((x0, x1)),
            }
        }
        spans.into_iter().map(|(x0, x1)| Rect::new(x0, line.top, x1.max(x0 + 1.0), line.top + line.height)).collect()
    }

    /// All glyph outlines as one path (for outline/flatten and boolean ops).
    pub fn to_path(&self) -> BezPath {
        let mut p = BezPath::new();
        for g in &self.glyphs {
            p.extend(g.path());
        }
        for d in &self.decorations {
            p.extend(d.path());
        }
        p
    }
}
