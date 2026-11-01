//! Sketch text layers → REST text nodes.
//!
//! A text layer's `attributedString` holds the `string` and `attributes`: runs given by `location`
//! and `length` in UTF-16 code units (like the REST importer's per-character style ids), each with
//! a font (PostScript name and size), colour, kerning, paragraph style, decorations and text
//! transform. `textBehaviour` sizes the box (0 auto width, 1 fixed width, 2 fixed); the style's
//! `textStyle` gives the vertical alignment.

use serde_json::{Map as JsonMap, Value, json};

use super::tree::{Cx, color};

/// Most characters read from one text layer (the REST importer's own limit).
const MAX_CHARS: usize = 1 << 20;
/// Most runs read from one text layer.
const MAX_RUNS: usize = 10_000;

/// Fill `node` (a TEXT node) from text layer `l`.
pub(crate) fn node(cx: &mut Cx, l: &Value, node: &mut JsonMap<String, Value>) {
    let a = l.get("attributedString");
    let string: String = a.and_then(|a| a.get("string")).and_then(Value::as_str).unwrap_or("").chars().take(MAX_CHARS).collect();
    let units = string.encode_utf16().count();
    let runs: &[Value] = a.and_then(|a| a.get("attributes")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    if runs.len() > MAX_RUNS {
        cx.warn(format!("text with more than {MAX_RUNS} styled runs keeps the first ones"));
    }
    let fallback = l.get("style").and_then(|s| s.get("textStyle")).and_then(|t| t.get("encodedAttributes"));
    let styles: Vec<(usize, usize, Value)> = runs
        .iter()
        .take(MAX_RUNS)
        .filter_map(|r| {
            let start = usize::try_from(r.get("location")?.as_u64()?).ok()?.min(units);
            let len = usize::try_from(r.get("length")?.as_u64()?).unwrap_or(usize::MAX);
            Some((start, start.saturating_add(len).min(units), type_style(cx, r.get("attributes")?)))
        })
        .collect();
    let base = styles.first().map(|s| s.2.clone()).or_else(|| fallback.map(|f| type_style(cx, f))).unwrap_or_else(|| type_style(cx, &json!({})));
    let mut style = base.clone();
    let mut fills = None;
    if let Some(m) = style.as_object_mut() {
        fills = m.remove("fills");
        // Paragraph settings are the first run's.
        let paragraph = runs.first().and_then(|r| r.get("attributes")).or(fallback).and_then(|a| a.get("paragraphStyle"));
        let align = match paragraph.and_then(|p| p.get("alignment")).and_then(Value::as_i64) {
            Some(1) => "RIGHT",
            Some(2) => "CENTER",
            Some(3) => "JUSTIFIED",
            _ => "LEFT",
        };
        m.insert("textAlignHorizontal".into(), json!(align));
        if let Some(sp) = paragraph.and_then(|p| p.get("paragraphSpacing")).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0) {
            m.insert("paragraphSpacing".into(), json!(sp.min(1e4)));
        }
        let vertical = l.get("style").and_then(|s| s.get("textStyle")).and_then(|t| t.get("verticalAlignment")).and_then(Value::as_i64);
        m.insert(
            "textAlignVertical".into(),
            json!(match vertical {
                Some(1) => "CENTER",
                Some(2) => "BOTTOM",
                _ => "TOP",
            }),
        );
        // Sketch 2025 and newer size text by `horizontalSizing`/`verticalSizing` (1 fits).
        let fits = |k: &str| l.get(k).and_then(Value::as_i64) == Some(1);
        let resize = match l.get("textBehaviour").and_then(Value::as_i64) {
            Some(1) => "HEIGHT",
            Some(2) => "NONE",
            Some(_) => "WIDTH_AND_HEIGHT",
            None if l.get("horizontalSizing").is_none() || fits("horizontalSizing") => "WIDTH_AND_HEIGHT",
            None if fits("verticalSizing") => "HEIGHT",
            None => "NONE",
        };
        m.insert("textAutoResize".into(), json!(resize));
    }
    node.insert("characters".into(), json!(string));
    node.insert("style".into(), style);
    if let Some(f) = fills {
        node.insert("fills".into(), f);
    }
    // Runs that differ from the first: a style id per UTF-16 unit.
    let mut ids = vec![0u64; units];
    let mut table = JsonMap::new();
    for (start, end, s) in &styles {
        if *s == base {
            continue;
        }
        let id = table.len() as u64 + 1;
        table.insert(id.to_string(), s.clone());
        for slot in ids.get_mut(*start..*end).into_iter().flatten() {
            *slot = id;
        }
    }
    if !table.is_empty() {
        node.insert("characterStyleOverrides".into(), json!(ids));
        node.insert("styleOverrideTable".into(), Value::Object(table));
    }
}

/// How far a text layer moves so its lines sit where Sketch draws them. With a fixed line height,
/// Sketch puts a line's extra height (line height − the font's own, taken as 1.2 × its size) above
/// the text (`lineSpacingBehaviour` 0) or below it (1); FigCraft, like Sketch's default (2), puts
/// half on each side. Every line gets the same extra, so the whole layer moves.
pub(crate) fn baseline_shift(l: &Value) -> f64 {
    let first = l
        .get("attributedString")
        .and_then(|a| a.get("attributes"))
        .and_then(Value::as_array)
        .and_then(|r| r.first())
        .and_then(|r| r.get("attributes"));
    let Some(a) = first else { return 0.0 };
    let size = a.get("MSAttributedStringFontAttribute").and_then(|f| f.get("attributes")).and_then(|f| f.get("size")).and_then(Value::as_f64);
    let line = a.get("paragraphStyle").and_then(|p| p.get("maximumLineHeight").or_else(|| p.get("minimumLineHeight"))).and_then(Value::as_f64);
    let (Some(size), Some(line)) = (size.filter(|s| s.is_finite() && *s > 0.0), line.filter(|v| v.is_finite() && *v > 0.0)) else { return 0.0 };
    let extra = (line - size * 1.2).clamp(-1e4, 1e4);
    match l.get("lineSpacingBehaviour").and_then(Value::as_i64) {
        Some(0) => extra / 2.0,
        Some(1) => -extra / 2.0,
        _ => 0.0,
    }
}

/// A run's attributes as a REST type style (with its colour as `fills`).
pub(crate) fn type_style(cx: &mut Cx, a: &Value) -> Value {
    let font = a.get("MSAttributedStringFontAttribute").and_then(|f| f.get("attributes"));
    let ps = font.and_then(|f| f.get("name")).and_then(Value::as_str).unwrap_or("");
    let size = font.and_then(|f| f.get("size")).and_then(Value::as_f64).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(12.0);
    let (family, style) = installed(cx, ps).unwrap_or_else(|| font_name(ps));
    if ps.is_empty() {
        cx.warn("text without a font uses Helvetica");
    }
    let mut s = json!({"fontFamily": family, "fontStyle": style, "fontSize": size.min(10_000.0)});
    // Every field is written, neutral when the run has none: a run's style starts from the text's
    // (its first run's), and an unwritten field would keep that one's value.
    s["fontVariations"] = font.and_then(|f| f.get("variation")).and_then(variations).unwrap_or_else(|| json!({}));
    let p = a.get("paragraphStyle");
    let line = |k: &str| p.and_then(|p| p.get(k)).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0);
    if let Some(lh) = line("maximumLineHeight").or_else(|| line("minimumLineHeight")) {
        s["lineHeightPx"] = json!(lh.min(1e4));
        s["lineHeightUnit"] = json!("PIXELS");
    } else {
        s["lineHeightUnit"] = json!("INTRINSIC_%");
    }
    s["letterSpacing"] = json!(a.get("kerning").and_then(Value::as_f64).filter(|k| k.is_finite()).unwrap_or(0.0).clamp(-1e4, 1e4));
    let set = |k: &str| a.get(k).and_then(Value::as_i64).is_some_and(|v| v != 0);
    s["textDecoration"] = json!(if set("underlineStyle") {
        "UNDERLINE"
    } else if set("strikethroughStyle") {
        "STRIKETHROUGH"
    } else {
        "NONE"
    });
    s["textCase"] = json!(match a.get("MSAttributedStringTextTransformAttribute").and_then(Value::as_i64) {
        Some(1) => "UPPER",
        Some(2) => "LOWER",
        _ => "ORIGINAL",
    });
    let c = a.get("MSAttributedStringColorAttribute").and_then(color).unwrap_or_else(|| json!({"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}));
    let mut paint = json!({"type": "SOLID", "color": c});
    if let Some(var) = super::style::swatch(cx, a.get("MSAttributedStringColorAttribute")) {
        paint["boundVariables"] = var;
    }
    s["fills"] = json!([paint]);
    s
}

/// A font's `variation` (axis values keyed by the axis tag read as a big-endian number, as Apple's
/// text engine keys them: 2003265652 is `wght`) as `{tag: value}`; `None` when it has none.
fn variations(v: &Value) -> Option<Value> {
    let axes: JsonMap<String, Value> = v
        .as_object()?
        .iter()
        .take(64)
        .filter_map(|(k, val)| {
            let tag = u32::try_from(k.parse::<u64>().ok()?).ok()?.to_be_bytes();
            let tag = std::str::from_utf8(&tag).ok().filter(|t| t.chars().all(|c| c.is_ascii_graphic()))?.to_string();
            let val = val.as_f64().filter(|x| x.is_finite())?;
            Some((tag, json!(val.clamp(-1e5, 1e5))))
        })
        .collect();
    (!axes.is_empty()).then_some(Value::Object(axes))
}

/// The installed family and style of PostScript name `ps`, when the import has a font lookup and
/// the font is installed.
fn installed(cx: &mut Cx, ps: &str) -> Option<(String, String)> {
    if ps.is_empty() {
        return None;
    }
    if let Some(found) = cx.font_cache.get(ps) {
        return found.clone();
    }
    let found = cx.fonts.as_ref().and_then(|f| f(ps));
    // A file names a few hundred fonts at most; junk ones don't grow the cache past that.
    if cx.font_cache.len() < 4096 {
        cx.font_cache.insert(ps.to_string(), found.clone());
    }
    found
}

/// Family and style of a font from its PostScript name: "HelveticaNeue-Bold" → ("Helvetica Neue",
/// "Bold"). Words of the family are split at case changes, which can't always match the real name
/// (a CJK family would split as "Ping Fang SC"); the engine matches families against the installed
/// fonts ignoring spaces. Apple's system fonts take the SF Pro names, PingFang its own.
pub(crate) fn font_name(ps: &str) -> (String, String) {
    let (family, style) = ps.split_once('-').unwrap_or((ps, ""));
    let style = if style.is_empty() { "Regular".to_string() } else { crate::fig::style_name(style) };
    let family = match family {
        "" => "Helvetica".to_string(),
        ".SFNSText" | ".SFUIText" | "SanFranciscoText" | ".SFProText" => "SF Pro Text".into(),
        ".SFNSDisplay" | ".SFUIDisplay" | "SanFranciscoDisplay" | ".SFProDisplay" => "SF Pro Display".into(),
        // Apple's CJK families whose names don't split at case changes.
        "PingFangSC" | "PingFangTC" | "PingFangHK" => format!("PingFang {}", family.get(8..).unwrap_or("")),
        // Apple's SF families: "SF" and then words ("SFUIText" → "SF UI Text").
        f if f.starts_with("SF") && f.chars().nth(2).is_some_and(char::is_uppercase) => format!("SF {}", words(f.get(2..).unwrap_or(""))),
        f => words(f.trim_start_matches('.')),
    };
    (family, style)
}

/// "IBMPlexSans" → "IBM Plex Sans": a space before a capital that follows a small letter, and
/// before the last capital of a run of capitals followed by a small letter.
fn words(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|p| chars.get(p));
        let next = chars.get(i + 1);
        let split = c.is_uppercase()
            && (prev.is_some_and(|p| p.is_lowercase()) || (prev.is_some_and(|p| p.is_uppercase()) && next.is_some_and(|n| n.is_lowercase())));
        if split {
            out.push(' ');
        }
        out.push(*c);
    }
    out
}
