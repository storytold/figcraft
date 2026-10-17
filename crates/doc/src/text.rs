//! Text node data: characters plus a base style and styled runs.

use figcraft_color::{Color, Paint};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", content = "value", rename_all = "camelCase")]
#[derive(Default)]
pub enum LineHeight {
    #[default]
    Auto,
    Pixels(f64),
    Percent(f64),
}

impl LineHeight {
    /// Resolved line height in px for `font_size` (`auto` uses the font's natural ratio).
    pub fn resolve(self, font_size: f64, natural_ratio: f64) -> f64 {
        match self {
            LineHeight::Auto => font_size * natural_ratio,
            LineHeight::Pixels(v) => v.max(0.0),
            LineHeight::Percent(p) => font_size * p.max(0.0) / 100.0,
        }
    }
    pub fn label(self) -> String {
        match self {
            LineHeight::Auto => "Auto".into(),
            LineHeight::Pixels(v) => trim_num(v),
            LineHeight::Percent(p) => format!("{}%", trim_num(p)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", content = "value", rename_all = "camelCase")]
pub enum LetterSpacing {
    Pixels(f64),
    Percent(f64),
}

impl Default for LetterSpacing {
    fn default() -> Self {
        LetterSpacing::Percent(0.0)
    }
}

impl LetterSpacing {
    pub fn resolve(self, font_size: f64) -> f64 {
        match self {
            LetterSpacing::Pixels(v) => v,
            LetterSpacing::Percent(p) => font_size * p / 100.0,
        }
    }
    pub fn label(self) -> String {
        match self {
            LetterSpacing::Pixels(v) => trim_num(v),
            LetterSpacing::Percent(p) => format!("{}%", trim_num(p)),
        }
    }
}

pub fn trim_num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract().abs() < 1e-9 { format!("{}", r as i64) } else { format!("{r}") }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextDecoration {
    #[default]
    None,
    Underline,
    Strikethrough,
}

/// How an underline or strikethrough is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DecorationStyle {
    #[default]
    Solid,
    Dotted,
    Wavy,
}

/// A decoration length: `Auto` (the font's own value), pixels, or a percentage of the font size.
pub type DecorationLength = LineHeight;

/// How a decoration line looks; the default is the font's own solid line in the text's colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DecorationLook {
    pub style: DecorationStyle,
    pub thickness: DecorationLength,
    /// Underlines: from the baseline down to the line's top edge (negative: above the baseline).
    pub offset: DecorationLength,
    /// Underlines break where descenders cross them.
    pub skip_ink: bool,
    /// The line's own colour (None: the text's fills).
    pub color: Option<Color>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextCase {
    #[default]
    Original,
    Upper,
    Lower,
    Title,
    SmallCaps,
}

impl TextCase {
    pub fn apply(self, s: &str) -> String {
        self.apply_after(s, None)
    }

    /// Apply the case to `s`, which follows `prev` in the text (so title case only capitalises
    /// the first letter of each word even when a word spans several styled runs).
    pub fn apply_after(self, s: &str, prev: Option<char>) -> String {
        match self {
            TextCase::Original | TextCase::SmallCaps => s.to_string(),
            TextCase::Upper => s.to_uppercase(),
            TextCase::Lower => s.to_lowercase(),
            TextCase::Title => {
                let mut out = String::with_capacity(s.len());
                let mut start = prev.is_none_or(|c| !is_word_char(c));
                for ch in s.chars() {
                    if start && ch.is_alphanumeric() {
                        out.extend(ch.to_uppercase());
                        start = false;
                    } else {
                        out.push(ch);
                        start = !is_word_char(ch);
                    }
                }
                out
            }
        }
    }

    /// Whether the char at a position preceded by `prev` starts a word (for title case).
    pub fn starts_word(prev: Option<char>) -> bool {
        prev.is_none_or(|c| !is_word_char(c))
    }
}

/// Letters, digits and apostrophes continue a word (so "don't" stays "Don't").
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '\'' | '\u{2019}')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextAlignH {
    #[default]
    Left,
    Center,
    Right,
    Justified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextAlignV {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// How a text box resizes as its content changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextAutoResize {
    /// Auto width: one line per paragraph, box grows both ways.
    #[default]
    WidthAndHeight,
    /// Fixed width, height grows.
    Height,
    /// Fixed size.
    None,
    /// Fixed size, truncate with an ellipsis.
    Truncate,
}

/// Vertical trim: what bounds a text box vertically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LeadingTrim {
    /// The full line height of the first and last lines.
    #[default]
    None,
    /// From the first line's cap height to the last line's baseline.
    CapHeight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ListStyle {
    #[default]
    None,
    Bulleted,
    Numbered,
}

/// Deepest list nesting (levels 0..=MAX_LIST_INDENT).
pub const MAX_LIST_INDENT: u8 = 8;

/// One paragraph's list settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ParaList {
    pub list: ListStyle,
    /// Nesting level (0 = outermost; plain paragraphs are always 0).
    pub indent: u8,
    /// A numbered list that starts counting at this paragraph counts from `start_offset + 1`
    /// (items that go on counting ignore it; other paragraphs are always 0).
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub start_offset: u32,
}

/// Character style. In runs, every field is an override; `None` means "inherit the base".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyle {
    pub font_family: String,
    /// The style name, e.g. "Regular", "Bold", "Semi Bold Italic".
    pub font_style: String,
    pub font_size: f64,
    pub line_height: LineHeight,
    pub letter_spacing: LetterSpacing,
    pub decoration: TextDecoration,
    /// How the decoration looks (None: the default look; boxed to keep styles small).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoration_look: Option<Box<DecorationLook>>,
    pub case: TextCase,
    /// Run fills (None = node fills).
    pub fills: Option<Vec<Paint>>,
    /// Hyperlink.
    pub link: Option<String>,
    /// Variable font axis values in user space, keyed by 4-char OpenType axis tag (e.g. `wght`:
    /// 550, `wdth`: 85). Empty = what the style name implies; axes the face lacks are ignored.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub variations: std::collections::BTreeMap<String, f64>,
    /// The text style (shared style id) these chars are linked to. A layer's chars can link to
    /// different styles; editing what a style sets unlinks the chars edited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 12.0,
            line_height: LineHeight::Auto,
            letter_spacing: LetterSpacing::default(),
            decoration: TextDecoration::None,
            decoration_look: None,
            case: TextCase::Original,
            fills: None,
            link: None,
            variations: std::collections::BTreeMap::new(),
            style_id: None,
        }
    }
}

/// Axes a style name implies (weight, italic, slant): picking a named style clears their custom values.
pub const STYLE_AXES: [&str; 3] = ["wght", "ital", "slnt"];

impl TextStyle {
    /// Numeric weight: a custom `wght` axis value, else the one implied by the style name.
    pub fn weight(&self) -> u16 {
        match self.variations.get("wght") {
            Some(w) if w.is_finite() => w.round().clamp(1.0, 1000.0) as u16,
            _ => style_weight(&self.font_style),
        }
    }
    pub fn italic(&self) -> bool {
        self.font_style.to_ascii_lowercase().contains("italic")
            || self.variations.get("ital").is_some_and(|v| *v >= 0.5)
            || self.variations.get("slnt").is_some_and(|v| *v != 0.0)
    }

    /// Take what text style `def` sets (font, size, line height, letter spacing, decoration, case
    /// and axis values) and link to it as `id`; fills and hyperlinks stay.
    pub fn take_text_style(&mut self, def: &TextStyle, id: &str) {
        *self = TextStyle { fills: self.fills.take(), link: self.link.take(), style_id: Some(id.to_string()), ..def.clone() };
    }

    /// Whether these chars show what text style `def` sets.
    pub fn shows_text_style(&self, def: &TextStyle) -> bool {
        self.font_family == def.font_family
            && self.font_style == def.font_style
            && self.font_size == def.font_size
            && self.line_height == def.line_height
            && self.letter_spacing == def.letter_spacing
            && self.decoration == def.decoration
            && self.decoration_look == def.decoration_look
            && self.case == def.case
            && self.variations == def.variations
    }

    /// This style as a text style's definition (linked to nothing).
    pub fn as_text_style(&self) -> TextStyle {
        TextStyle { style_id: None, link: None, ..self.clone() }
    }

    /// How the decoration looks.
    pub fn look(&self) -> DecorationLook {
        self.decoration_look.as_deref().copied().unwrap_or_default()
    }

    /// Change how the decoration looks (the default look is stored as None).
    pub fn set_look(&mut self, f: impl FnOnce(&mut DecorationLook)) {
        let mut look = self.look();
        f(&mut look);
        self.decoration_look = (look != DecorationLook::default()).then(|| Box::new(look));
    }

    /// Switch to the named style, dropping custom values of the axes the name implies.
    pub fn set_font_style(&mut self, name: impl Into<String>) {
        self.font_style = name.into();
        for tag in STYLE_AXES {
            self.variations.remove(tag);
        }
    }
}

/// Weight from a style name ("Semi Bold Italic" → 600).
pub fn style_weight(style: &str) -> u16 {
    let s = style.to_ascii_lowercase().replace([' ', '-'], "");
    let table: [(&str, u16); 10] = [
        ("thin", 100),
        ("extralight", 200),
        ("ultralight", 200),
        ("light", 300),
        ("medium", 500),
        ("semibold", 600),
        ("demibold", 600),
        ("extrabold", 800),
        ("ultrabold", 800),
        ("black", 900),
    ];
    for (k, w) in table {
        if s.contains(k) {
            return w;
        }
    }
    if s.contains("bold") {
        return 700;
    }
    if s.contains("heavy") {
        return 900;
    }
    400
}

/// A styled range of characters (char indices, end exclusive).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StyleRun {
    pub start: usize,
    pub end: usize,
    pub style: TextStyle,
}

/// Text node content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TextData {
    pub characters: String,
    pub style: TextStyle,
    /// Ranges whose style differs from `style`; full styles (already merged), non-overlapping, sorted.
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub runs: Vec<StyleRun>,
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub auto_resize: TextAutoResize,
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub align_h: TextAlignH,
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub align_v: TextAlignV,
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub paragraph_spacing: f64,
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub paragraph_indent: f64,
    /// The list style of paragraphs `paragraphs` doesn't cover (all of them when it's empty).
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub list: ListStyle,
    /// Per-paragraph list settings, by paragraph index; empty when every paragraph is `list` at
    /// level 0. Kept in step with the characters by [`TextData::replace_range`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paragraphs: Vec<ParaList>,
    /// Space between two items of a list (px), in place of the paragraph spacing.
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub list_spacing: f64,
    /// Opening and closing punctuation at a line's aligned edge sits outside the box.
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub hanging_punctuation: bool,
    /// List bullets and numbers sit outside the box; item text lines up with plain paragraphs.
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub hanging_list: bool,
    /// Vertical trim (`capHeight`: the box runs from the first line's cap height to the last
    /// line's baseline).
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub leading_trim: LeadingTrim,
    /// Max lines when truncating (0 = as many as fit).
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub max_lines: u32,
    /// OpenType feature toggles (e.g. "liga": false).
    #[serde(skip_serializing_if = "crate::node::is_default")]
    pub features: std::collections::BTreeMap<String, bool>,
    /// The path the text follows (text on a path); `None`: lines in the box. Boxed to keep text
    /// data small.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Box<TextPath>>,
}

/// Text on a path: the baseline follows `d`. The text starts at `start` (aligned there by the
/// horizontal alignment: left starts, centre centres, right ends at it); glyphs past an open
/// path's ends are hidden, a closed path wraps them around.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TextPath {
    /// SVG path data in the layer's own space.
    pub d: String,
    /// Where the text starts along the path, 0..1 by length.
    pub start: f64,
    /// On the other side of the path, running the other way.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
}

impl TextPath {
    /// A text path following `path` (layer space).
    pub fn new(path: &figcraft_geom::BezPath) -> Self {
        Self { d: path.to_svg(), start: 0.0, flip: false }
    }

    /// The path (empty when `d` doesn't parse).
    pub fn path(&self) -> figcraft_geom::BezPath {
        figcraft_geom::BezPath::from_svg(&self.d).unwrap_or_default()
    }

    /// The start offset clamped into 0..1 (NaN: 0).
    pub fn start(&self) -> f64 {
        if self.start.is_finite() { self.start.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Scale the path by `sx`, `sy` (the layer was resized).
    pub fn scale(&mut self, sx: f64, sy: f64) {
        if sx.is_finite() && sy.is_finite() {
            let mut p = self.path();
            p.apply_affine(figcraft_geom::Affine::scale_non_uniform(sx, sy));
            self.d = p.to_svg();
        }
    }
}

/// Whether `tag` is a valid OpenType feature tag (four printable ASCII chars).
pub fn valid_feature_tag(tag: &str) -> bool {
    tag.len() == 4 && tag.bytes().all(|b| (0x20..0x7f).contains(&b))
}

/// Whether `tag` is a valid OpenType variation axis tag (same shape as a feature tag).
pub fn valid_axis_tag(tag: &str) -> bool {
    valid_feature_tag(tag)
}

impl TextData {
    pub fn new(characters: impl Into<String>) -> Self {
        Self { characters: characters.into(), ..Default::default() }
    }

    /// How far text on a path paints past its path's box (glyphs stand on the path, lines after
    /// the first run beside it): about one and a half of the largest size per line; 0 off a path.
    pub fn path_reach(&self) -> f64 {
        if self.path.is_none() {
            return 0.0;
        }
        let lines = self.characters.lines().count().clamp(1, 10_000) as f64;
        self.max_size() * 1.5 * lines
    }

    /// How far vertically trimmed text paints past its box (descenders below the last baseline,
    /// accents above the first cap height): half the largest size; 0 untrimmed.
    pub fn trim_reach(&self) -> f64 {
        if self.leading_trim == LeadingTrim::None { 0.0 } else { self.max_size() * 0.5 }
    }

    /// The largest font size used (capped like layout caps it).
    fn max_size(&self) -> f64 {
        self.spans().iter().map(|(_, _, s)| s.font_size).filter(|v| v.is_finite()).fold(0.0, f64::max).min(5000.0)
    }

    pub fn char_count(&self) -> usize {
        self.characters.chars().count()
    }

    /// The style at char index `i`.
    pub fn style_at(&self, i: usize) -> &TextStyle {
        self.runs.iter().find(|r| r.start <= i && i < r.end).map(|r| &r.style).unwrap_or(&self.style)
    }

    /// Spans of uniform style covering the whole text: (start, end, style).
    pub fn spans(&self) -> Vec<(usize, usize, &TextStyle)> {
        let n = self.char_count();
        let mut out = Vec::new();
        let mut pos = 0;
        let mut runs: Vec<&StyleRun> = self.runs.iter().filter(|r| r.start < r.end && r.start < n).collect();
        runs.sort_by_key(|r| r.start);
        for r in runs {
            let start = r.start.max(pos);
            let end = r.end.min(n);
            if start >= end {
                continue;
            }
            if start > pos {
                out.push((pos, start, &self.style));
            }
            out.push((start, end, &r.style));
            pos = end;
        }
        if pos < n || out.is_empty() {
            out.push((pos, n, &self.style));
        }
        out
    }

    /// Apply `f` to the style of chars `start..end` (all text when the range is the whole text or
    /// empty text), splitting runs as needed and merging equal neighbours.
    pub fn restyle(&mut self, start: usize, end: usize, f: impl Fn(&mut TextStyle)) {
        let n = self.char_count();
        let (start, end) = (start.min(n), end.min(n));
        if n == 0 || (start == 0 && end >= n) {
            f(&mut self.style);
            for r in &mut self.runs {
                f(&mut r.style);
            }
            self.normalize();
            return;
        }
        if start >= end {
            return;
        }
        // Expand to per-char styles, modify, re-compress.
        let mut per: Vec<TextStyle> = (0..n).map(|i| self.style_at(i).clone()).collect();
        for s in per.iter_mut().take(end).skip(start) {
            f(s);
        }
        self.runs.clear();
        let mut i = 0;
        while i < n {
            let Some(s) = per.get(i) else { break };
            let mut j = i + 1;
            while per.get(j).is_some_and(|t| t == s) {
                j += 1;
            }
            if *s != self.style {
                self.runs.push(StyleRun { start: i, end: j, style: s.clone() });
            }
            i = j;
        }
    }

    /// Drop empty/out-of-range runs and runs equal to the base style, and merge touching equal runs.
    pub fn normalize(&mut self) {
        let n = self.char_count();
        let base = self.style.clone();
        self.runs.retain(|r| r.start < r.end && r.start < n && r.style != base);
        for r in &mut self.runs {
            r.end = r.end.min(n);
        }
        self.runs.sort_by_key(|r| r.start);
        let mut merged: Vec<StyleRun> = Vec::with_capacity(self.runs.len());
        for r in std::mem::take(&mut self.runs) {
            match merged.last_mut() {
                Some(prev) if prev.end == r.start && prev.style == r.style => prev.end = r.end,
                _ => merged.push(r),
            }
        }
        self.runs = merged;
    }

    /// Replace chars `start..end` with `text`, shifting runs (inserted text takes the style at `start`).
    pub fn replace_range(&mut self, start: usize, end: usize, text: &str) {
        let n = self.char_count();
        let (start, end) = (start.min(n), end.min(n).max(start.min(n)));
        let byte = |i: usize| self.characters.char_indices().nth(i).map(|(b, _)| b).unwrap_or(self.characters.len());
        let (bs, be) = (byte(start), byte(end));
        if !self.paragraphs.is_empty() {
            // Paragraphs merged by removed newlines take the first one's list; new ones copy it.
            let p = self.para_of(start);
            let removed = self.characters.get(bs..be).unwrap_or("").matches('\n').count();
            let added = text.matches('\n').count();
            self.fill_paragraphs();
            let at = self.para_list(p);
            let tail = (p + 1).min(self.paragraphs.len());
            self.paragraphs.drain(tail..(p + 1 + removed).min(self.paragraphs.len()));
            self.paragraphs.splice(tail..tail, std::iter::repeat_n(at, added));
        }
        let mut s = String::with_capacity(self.characters.len() + text.len());
        s.push_str(self.characters.get(..bs).unwrap_or(""));
        s.push_str(text);
        s.push_str(self.characters.get(be..).unwrap_or(""));
        self.characters = s;
        let inserted = text.chars().count();
        let removed = end - start;
        for r in &mut self.runs {
            let shift = |p: usize| -> usize {
                if p <= start {
                    p
                } else if p >= end {
                    // Runs of a file that reach past the text (nothing clamps them on load)
                    // would otherwise overflow here; `normalize` below drops what is left.
                    p.saturating_sub(removed).saturating_add(inserted)
                } else {
                    start + inserted
                }
            };
            // A run touching the insertion point at its end grows with the typed text.
            let new_start = shift(r.start);
            let new_end = if r.end == start && r.start < start { start + inserted } else { shift(r.end) };
            r.start = new_start;
            r.end = new_end;
        }
        self.normalize();
    }

    pub fn paragraph_count(&self) -> usize {
        self.characters.matches('\n').count() + 1
    }

    /// The paragraph char index `i` is in.
    pub fn para_of(&self, i: usize) -> usize {
        self.characters.chars().take(i).filter(|c| *c == '\n').count()
    }

    /// Paragraph `p`'s list settings.
    pub fn para_list(&self, p: usize) -> ParaList {
        self.paragraphs.get(p).copied().unwrap_or(ParaList { list: self.list, ..ParaList::default() })
    }

    /// Spell out every paragraph's list settings in `paragraphs` (and drop extra entries).
    fn fill_paragraphs(&mut self) {
        let n = self.paragraph_count();
        let fill = ParaList { list: self.list, ..ParaList::default() };
        self.paragraphs.resize(n, fill);
    }

    /// The paragraphs chars `start..end` touch (a caret: its paragraph; a selection ending just
    /// after a newline leaves the next paragraph out).
    pub fn para_span(&self, start: usize, end: usize) -> (usize, usize) {
        let (s, e) = (start.min(end), start.max(end));
        let (a, b) = (self.para_of(s), self.para_of(e));
        let ends_at_start = e > s && b > a && self.characters.chars().nth(e - 1) == Some('\n');
        (a, if ends_at_start { b - 1 } else { b })
    }

    /// Apply `f` to the list settings of the paragraphs chars `start..end` touch (plain
    /// paragraphs stay at level 0, only numbered ones keep a start, levels stop at
    /// [`MAX_LIST_INDENT`]); one style for all with no nesting or start folds back into `list`.
    pub fn set_para_lists(&mut self, start: usize, end: usize, f: impl Fn(&mut ParaList)) {
        let (a, b) = self.para_span(start, end);
        self.fill_paragraphs();
        for pl in self.paragraphs.get_mut(a..=b).unwrap_or(&mut []) {
            f(pl);
            if pl.list == ListStyle::None {
                pl.indent = 0;
            }
            if pl.list != ListStyle::Numbered {
                pl.start_offset = 0;
            }
            pl.indent = pl.indent.min(MAX_LIST_INDENT);
        }
        if let Some(first) = self.paragraphs.first().copied()
            && first == (ParaList { list: first.list, ..ParaList::default() })
            && self.paragraphs.iter().all(|p| *p == first)
        {
            self.list = first.list;
            self.paragraphs.clear();
        }
    }

    /// The list style and level shared by the paragraphs chars `start..end` touch (the first
    /// one's settings), or None when they differ. Start numbers don't count.
    pub fn common_para_list(&self, start: usize, end: usize) -> Option<ParaList> {
        let (a, b) = self.para_span(start, end);
        let first = self.para_list(a);
        (a..=b).all(|p| self.para_list(p).list == first.list && self.para_list(p).indent == first.indent).then_some(first)
    }

    /// Link chars `start..end` (all of them when the range is empty or the whole text) to text
    /// style `id`, defined as `def`.
    pub fn link_text_style(&mut self, start: usize, end: usize, def: &TextStyle, id: &str) {
        let (start, end) = if start == end { (0, self.char_count()) } else { (start, end) };
        self.restyle(start, end, |s| s.take_text_style(def, id));
    }

    /// Unlink chars `start..end` (all when empty or whole) from their text styles; values stay.
    pub fn unlink_text_style(&mut self, start: usize, end: usize) {
        let (start, end) = if start == end { (0, self.char_count()) } else { (start, end) };
        self.restyle(start, end, |s| s.style_id = None);
    }

    /// The text style every char is linked to: `Some(None)` when none is, None when they differ.
    pub fn text_style_link(&self) -> Option<Option<&str>> {
        let mut spans = self.spans().into_iter().map(|(_, _, s)| s.style_id.as_deref());
        let first = spans.next()?;
        spans.all(|s| s == first).then_some(first)
    }

    /// Apply `f` to every distinct style (the base and each run) and merge runs that became equal.
    pub(crate) fn each_style(&mut self, mut f: impl FnMut(&mut TextStyle)) {
        f(&mut self.style);
        for r in &mut self.runs {
            f(&mut r.style);
        }
        self.normalize();
    }

    /// Whether any char links to text style `id`.
    pub fn links_text_style(&self, id: &str) -> bool {
        std::iter::once(&self.style).chain(self.runs.iter().map(|r| &r.style)).any(|s| s.style_id.as_deref() == Some(id))
    }

    /// Follow text style `id`'s new definition wherever chars link to it.
    pub fn update_text_style(&mut self, id: &str, def: &TextStyle) {
        self.each_style(|s| {
            if s.style_id.as_deref() == Some(id) {
                s.take_text_style(def, id);
            }
        });
    }

    /// Point text style links elsewhere: `f(id)` gives the new id, None unlinks (values stay).
    pub fn map_text_styles(&mut self, f: impl Fn(&str) -> Option<String>) {
        self.each_style(|s| s.style_id = s.style_id.as_deref().and_then(&f));
    }

    /// Unlink chars whose style is gone (`def` gives None) or that no longer show it (edited since).
    pub fn check_text_styles<'a>(&mut self, def: impl Fn(&str) -> Option<&'a TextStyle>) {
        self.each_style(|s| {
            if s.style_id.as_deref().is_some_and(|id| !def(id).is_some_and(|d| s.shows_text_style(d))) {
                s.style_id = None;
            }
        });
    }

    /// Whether every char shares one style (the panel shows "Mixed" otherwise).
    pub fn is_uniform(&self) -> bool {
        self.runs.is_empty()
    }
}

#[cfg(test)]
mod case_tests {
    use super::*;

    #[test]
    fn title_case_respects_context() {
        assert_eq!(TextCase::Title.apply("hello wORLD don't 3d x-ray"), "Hello WORLD Don't 3d X-Ray");
        // Continuing a word from a previous run: no capital.
        assert_eq!(TextCase::Title.apply_after("lo world", Some('l')), "lo World");
        assert_eq!(TextCase::Title.apply_after("lo", Some(' ')), "Lo");
        assert_eq!(TextCase::Upper.apply_after("ß", Some('a')), "SS");
        assert!(valid_feature_tag("liga") && valid_feature_tag("ss01"));
        assert!(!valid_feature_tag("lig") && !valid_feature_tag("ligatures") && !valid_feature_tag("li\u{e9}"));
    }

    #[test]
    fn variations_are_backward_compatible_and_imply_weight() {
        // Files written before variable fonts load with no custom axes, and stay byte-identical.
        let old = r#"{"font_family":"Inter","font_style":"Bold","font_size":12.0}"#;
        let st: TextStyle = serde_json::from_str(old).unwrap();
        assert!(st.variations.is_empty());
        assert!(!serde_json::to_string(&st).unwrap().contains("variations"));
        let mut st = TextStyle { font_style: "Regular".into(), ..Default::default() };
        st.variations.insert("wght".into(), 640.0);
        st.variations.insert("wdth".into(), 80.0);
        assert_eq!(st.weight(), 640);
        assert!(!st.italic());
        st.variations.insert("slnt".into(), -8.0);
        assert!(st.italic());
        let round: TextStyle = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(round, st);
        // Picking a named style drops the axes its name implies, keeps the others.
        st.set_font_style("Bold");
        assert_eq!(st.weight(), 700);
        assert_eq!(st.variations.keys().collect::<Vec<_>>(), ["wdth"]);
        assert!(valid_axis_tag("wght") && !valid_axis_tag("weight"));
    }

    #[test]
    fn list_starts_load_from_old_files_and_follow_edits() {
        use ListStyle::{Bulleted as B, None as P, Numbered as N};
        // Files written before start numbers load with none, and write the same bytes.
        let old = r#"{"list":"numbered","indent":1}"#;
        let pl: ParaList = serde_json::from_str(old).unwrap();
        assert_eq!(pl, ParaList { list: N, indent: 1, start_offset: 0 });
        assert_eq!(serde_json::to_string(&pl).unwrap(), old);
        // One numbered paragraph with a start keeps it (it doesn't fold into `list`).
        let mut t = TextData { characters: "Next".into(), ..Default::default() };
        t.set_para_lists(0, 0, |p| *p = ParaList { list: N, indent: 0, start_offset: 1 });
        assert_eq!(t.para_list(0), ParaList { list: N, indent: 0, start_offset: 1 });
        // Enter copies it to the new item; different starts don't make the list mixed.
        t.replace_range(4, 4, "\nMore");
        assert_eq!(t.para_list(1).start_offset, 1);
        t.set_para_lists(5, 5, |p| p.start_offset = 0);
        assert!(t.common_para_list(0, t.char_count()).is_some_and(|p| p.list == N));
        // Bullets have no number, so no start.
        t.set_para_lists(0, 0, |p| *p = ParaList { list: B, indent: 1, start_offset: 4 });
        assert_eq!(t.para_list(0), ParaList { list: B, indent: 1, start_offset: 0 });
        // A plain paragraph has none.
        t.set_para_lists(0, 0, |p| p.list = P);
        assert_eq!(t.para_list(0), ParaList::default());
    }
}
