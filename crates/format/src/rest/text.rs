//! TEXT nodes: characters, the base `style` (TypeStyle) and per-character overrides
//! (`characterStyleOverrides` + `styleOverrideTable`) → styled runs.

use std::collections::BTreeMap;

use figcraft_doc::{
    DecorationLength, DecorationStyle, LetterSpacing, LineHeight, ListStyle, ParaList, StyleRun, TextAlignH, TextAlignV, TextAutoResize, TextCase,
    TextData, TextDecoration, TextPath, TextStyle,
};
use serde_json::Value;

use super::Ctx;
use super::props::{flag, len, list, num, paints, text};

/// Most characters read from one text node.
const MAX_CHARS: usize = 1 << 20;

/// The text content of a TEXT node.
pub(super) fn text_data(cx: &mut Ctx, v: &Value) -> TextData {
    let raw = text(v, "characters").unwrap_or("");
    let characters: String = raw.chars().take(MAX_CHARS).collect();
    if characters.len() < raw.len() {
        cx.warn("very long text was cut to 1,048,576 characters");
    }
    let style_json = v.get("style").unwrap_or(&Value::Null);
    let base = type_style(cx, style_json, &TextStyle::default());
    let mut td = TextData { characters, style: base, ..TextData::default() };
    paragraph(style_json, &mut td);
    td.runs = runs(cx, v, &td);
    // Skip ink only shows on an underline. A file can carry it on any text, so elsewhere it stays out
    // of the look (runs have inherited the base value by now).
    for st in std::iter::once(&mut td.style).chain(td.runs.iter_mut().map(|r| &mut r.style)) {
        if st.decoration != TextDecoration::Underline && st.look().skip_ink {
            st.set_look(|l| l.skip_ink = false);
        }
    }
    td.normalize();
    lists(v, &mut td);
    if cx.fig_sources.is_some()
        && let Some(path) = v.get("_figTextPath")
    {
        match (path.get("d").and_then(Value::as_str), path.get("start").and_then(Value::as_f64), path.get("flip").and_then(Value::as_bool)) {
            (Some(d), Some(start), Some(flip)) if !d.is_empty() && start.is_finite() && (0.0..=1.0).contains(&start) => {
                td.path = Some(Box::new(TextPath { d: d.to_owned(), start, flip }));
            }
            _ => cx.warn("invalid internal .fig text path data was ignored"),
        }
    }
    // The line breaks the .fig file saved, for the box width the layer imports with.
    let width = v.get("size").and_then(super::props::vec2).map(|(w, _)| w.clamp(0.0, super::props::MAX_SIZE));
    if cx.fig_sources.is_some()
        && td.path.is_none()
        && td.auto_resize != TextAutoResize::WidthAndHeight
        && let (Some(starts), Some(width)) = (v.get("_figLineStarts").and_then(Value::as_array), width)
    {
        let n = td.char_count();
        let starts = starts.iter().filter_map(Value::as_u64).filter_map(|s| usize::try_from(s).ok()).filter(|s| *s < n).collect();
        td.save_breaks(starts, width);
    }
    td
}

/// Node-level text settings that live in the base TypeStyle.
fn paragraph(s: &Value, td: &mut TextData) {
    td.align_h = match text(s, "textAlignHorizontal") {
        Some("CENTER") => TextAlignH::Center,
        Some("RIGHT") => TextAlignH::Right,
        Some("JUSTIFIED") => TextAlignH::Justified,
        _ => TextAlignH::Left,
    };
    td.align_v = match text(s, "textAlignVertical") {
        Some("CENTER") => TextAlignV::Middle,
        Some("BOTTOM") => TextAlignV::Bottom,
        _ => TextAlignV::Top,
    };
    td.auto_resize = match text(s, "textAutoResize") {
        Some("HEIGHT") => TextAutoResize::Height,
        Some("WIDTH_AND_HEIGHT") => TextAutoResize::WidthAndHeight,
        Some("TRUNCATE") => TextAutoResize::Truncate,
        // Missing means a fixed box.
        _ => TextAutoResize::None,
    };
    // A line limit applies only while truncation is on; a leftover `maxLines` is ignored.
    let ending = text(s, "textTruncation") == Some("ENDING");
    td.max_lines = num(s, "maxLines").filter(|n| ending && *n >= 1.0).map_or(0, |n| n.min(10_000.0) as u32);
    // Auto height with a line limit keeps hugging its lines up to the limit.
    if ending && !td.truncates() {
        td.auto_resize = TextAutoResize::Truncate;
    }
    td.paragraph_spacing = len(s, "paragraphSpacing").unwrap_or(0.0);
    td.paragraph_indent = len(s, "paragraphIndent").unwrap_or(0.0);
    td.list_spacing = len(s, "listSpacing").unwrap_or(0.0).min(10_000.0);
    td.hanging_punctuation = flag(s, "hangingPunctuation").unwrap_or(false);
    td.hanging_list = flag(s, "hangingList").unwrap_or(false);
    if text(s, "leadingTrim") == Some("CAP_HEIGHT") {
        td.leading_trim = figcraft_doc::LeadingTrim::CapHeight;
    }
    if let Some(flags) = s.get("openTypeFlags").and_then(Value::as_object) {
        for (tag, on) in flags.iter().take(64) {
            if figcraft_doc::valid_feature_tag(&tag.to_ascii_lowercase()) {
                td.features.insert(tag.to_ascii_lowercase(), on.as_f64().is_some_and(|x| x != 0.0) || on.as_bool() == Some(true));
            }
        }
    }
}

/// `base` with the fields `s` sets (override table entries only carry what differs).
fn type_style(cx: &mut Ctx, s: &Value, base: &TextStyle) -> TextStyle {
    let mut t = base.clone();
    if let Some(f) = text(s, "fontFamily").filter(|f| !f.trim().is_empty()) {
        t.font_family = super::props::clip_str(f).to_string();
    }
    if let Some(st) = text(s, "fontStyle").filter(|f| !f.trim().is_empty()) {
        t.font_style = super::props::clip_str(st).to_string();
    } else if s.get("fontWeight").is_some() || s.get("italic").is_some() {
        let weight = num(s, "fontWeight").map_or_else(|| t.weight(), |w| w.clamp(1.0, 1000.0) as u16);
        t.font_style = style_name(weight, flag(s, "italic").unwrap_or_else(|| t.italic()));
    }
    // Variable font axes, `{tag: value}` (FigCraft's own key; the Sketch importer writes it).
    if let Some(axes) = s.get("fontVariations").and_then(Value::as_object) {
        t.variations = axes
            .iter()
            .take(64)
            .filter(|(k, _)| k.len() == 4 && k.is_ascii())
            .filter_map(|(k, v)| Some((k.clone(), v.as_f64().filter(|x| x.is_finite())?.clamp(-1e5, 1e5))))
            .collect();
    }
    if let Some(size) = num(s, "fontSize").filter(|x| *x > 0.0) {
        t.font_size = size.clamp(0.5, 10_000.0);
    }
    let px = len(s, "lineHeightPx");
    let pct = len(s, "lineHeightPercentFontSize");
    match text(s, "lineHeightUnit") {
        Some("PIXELS") => t.line_height = px.map_or(t.line_height, LineHeight::Pixels),
        Some("FONT_SIZE_%") => t.line_height = pct.map_or(t.line_height, LineHeight::Percent),
        Some("INTRINSIC_%") => t.line_height = LineHeight::Auto,
        _ => {
            if let Some(p) = pct {
                t.line_height = LineHeight::Percent(p);
            } else if let Some(p) = px {
                t.line_height = LineHeight::Pixels(p);
            }
        }
    }
    // Letter spacing is in pixels.
    if let Some(ls) = num(s, "letterSpacing") {
        t.letter_spacing = if ls == 0.0 { LetterSpacing::default() } else { LetterSpacing::Pixels(ls.clamp(-1e4, 1e4)) };
    }
    // Private, from the .fig mapper: the spacing was a share of the font size.
    if cx.fig_sources.is_some()
        && let Some(p) = num(s, "_letterSpacingPercent")
    {
        t.letter_spacing = LetterSpacing::Percent(p.clamp(-1e4, 1e4));
    }
    match text(s, "textCase") {
        Some("ORIGINAL") => t.case = TextCase::Original,
        Some("UPPER") => t.case = TextCase::Upper,
        Some("LOWER") => t.case = TextCase::Lower,
        Some("TITLE") => t.case = TextCase::Title,
        Some("SMALL_CAPS" | "SMALL_CAPS_FORCED") => t.case = TextCase::SmallCaps,
        _ => {}
    }
    match text(s, "textDecoration") {
        Some("NONE") => t.decoration = TextDecoration::None,
        Some("UNDERLINE") => t.decoration = TextDecoration::Underline,
        Some("STRIKETHROUGH") => t.decoration = TextDecoration::Strikethrough,
        _ => {}
    }
    decoration_look(s, &mut t);
    if let Some(f) = paints(cx, s, "fills") {
        t.fills = Some(f);
    }
    if let Some(h) = s.get("hyperlink") {
        match (text(h, "type"), text(h, "url")) {
            // The destination is data, not a label: `clip_str` is for names that go into warnings,
            // and clipping a URL changed it into a different, working-looking one without a word
            // (#459). The FIG importer already keeps it whole.
            (Some("URL") | None, Some(url)) if !url.is_empty() => t.link = Some(url.to_string()),
            // An empty URL clears the link the base style has.
            (Some("URL") | None, Some(_)) => t.link = None,
            _ => {}
        }
    }
    t
}

/// The style name for a numeric weight ("Semi Bold Italic").
fn style_name(weight: u16, italic: bool) -> String {
    let w = match weight {
        0..=149 => "Thin",
        150..=249 => "Extra Light",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "Semi Bold",
        650..=749 => "Bold",
        750..=849 => "Extra Bold",
        _ => "Black",
    };
    match (w, italic) {
        ("Regular", true) => "Italic".into(),
        (w, true) => format!("{w} Italic"),
        (w, false) => w.into(),
    }
}

/// Styled runs from the per-character override ids (trailing characters without an id use the
/// base style). The ids count UTF-16 code units, like the JavaScript strings the file comes from:
/// a character outside the Basic Multilingual Plane takes two.
fn runs(cx: &mut Ctx, v: &Value, td: &TextData) -> Vec<StyleRun> {
    let ids: Vec<u64> = v
        .get("characterStyleOverrides")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_CHARS * 2).map(|x| x.as_u64().unwrap_or(0)).collect())
        .unwrap_or_default();
    let Some(table) = v.get("styleOverrideTable").and_then(Value::as_object) else { return Vec::new() };
    if ids.iter().all(|i| *i == 0) || table.is_empty() {
        return Vec::new();
    }
    let mut per_char: Vec<u64> = Vec::with_capacity(td.char_count());
    let mut unit = 0usize;
    for ch in td.characters.chars() {
        per_char.push(ids.get(unit).copied().unwrap_or(0));
        unit += ch.len_utf16();
    }
    let mut styles: BTreeMap<u64, TextStyle> = BTreeMap::new();
    let mut out: Vec<StyleRun> = Vec::new();
    let mut i = 0;
    while i < per_char.len() {
        let Some(&id) = per_char.get(i) else { break };
        let mut j = i + 1;
        while per_char.get(j) == Some(&id) {
            j += 1;
        }
        if id != 0 {
            let style = match styles.get(&id) {
                Some(s) => Some(s.clone()),
                None => match table.get(&id.to_string()) {
                    Some(o) => {
                        let s = type_style(cx, o, &td.style);
                        styles.insert(id, s.clone());
                        Some(s)
                    }
                    None => {
                        cx.warn("text style overrides refer to missing table entries");
                        None
                    }
                },
            };
            if let Some(style) = style.filter(|s| *s != td.style) {
                out.push(StyleRun { start: i, end: j, style });
            }
        }
        i = j;
    }
    out
}

/// Each paragraph's list type, nesting level and start number from `lineTypes`,
/// `lineIndentations` and (from `.fig` files) `listStartOffsets`.
fn lists(v: &Value, td: &mut TextData) {
    let types = list(v, "lineTypes");
    let indents = list(v, "lineIndentations");
    let starts = list(v, "listStartOffsets");
    td.paragraphs = types
        .iter()
        .enumerate()
        .map(|(i, t)| ParaList {
            list: match t.as_str() {
                Some("UNORDERED") => ListStyle::Bulleted,
                Some("ORDERED") => ListStyle::Numbered,
                _ => ListStyle::None,
            },
            indent: indents.get(i).and_then(Value::as_f64).filter(|d| d.is_finite()).map_or(0, |d| d.clamp(0.0, 255.0) as u8),
            start_offset: starts.get(i).and_then(Value::as_f64).filter(|n| n.is_finite()).map_or(0, |n| n.clamp(0.0, f64::from(u32::MAX)) as u32),
        })
        .collect();
    // Clamp the levels and fold one style for all back into the layer's list style.
    td.set_para_lists(0, td.char_count(), |_| {});
}

/// The decoration's style, thickness, offset, skip ink and colour (`{value, unit}` lengths with
/// `AUTO`, `PIXELS` or `PERCENT`; a colour as a solid paint, or `AUTO`).
fn decoration_look(s: &Value, t: &mut TextStyle) {
    let length = |k: &str| {
        let m = s.get(k)?;
        let v = num(m, "value").filter(|x| x.abs() <= 10_000.0);
        match text(m, "unit") {
            Some("PIXELS") => v.map(DecorationLength::Pixels),
            Some("PERCENT") => v.map(DecorationLength::Percent),
            Some("AUTO") => Some(DecorationLength::Auto),
            _ => None,
        }
    };
    let style = match text(s, "textDecorationStyle") {
        Some("DOTTED") => Some(DecorationStyle::Dotted),
        Some("WAVY") => Some(DecorationStyle::Wavy),
        Some("SOLID") => Some(DecorationStyle::Solid),
        _ => None,
    };
    let (thickness, offset, skip) = (length("textDecorationThickness"), length("textDecorationOffset"), flag(s, "textDecorationSkipInk"));
    let color = s.get("textDecorationColor").map(|c| c.get("value").and_then(|p| p.get("color")).and_then(super::props::color));
    t.set_look(|l| {
        l.style = style.unwrap_or(l.style);
        l.thickness = thickness.unwrap_or(l.thickness);
        l.offset = offset.unwrap_or(l.offset);
        l.skip_ink = skip.unwrap_or(l.skip_ink);
        l.color = color.unwrap_or(l.color);
    });
}
