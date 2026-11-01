use super::paint::paint;
use super::{Kv, Mapper, Obj, Value, View, copy_num, copy_str, json, network, num};

/// The value from the last override layer holding a valid one for an optional text setting; invalid instance rows fall through.
fn text_num(v: &View, key: &str) -> Option<f64> {
    v.layers.iter().rev().find_map(|layer| {
        layer.get(key).and_then(|value| match value {
            Kv::Bool(_) => None,
            _ => value.f64(),
        })
    })
}

fn text_flag(v: &View, key: &str) -> Option<bool> {
    v.layers.iter().rev().find_map(|layer| layer.get(key).and_then(Kv::bool))
}

fn insert_text_options(style: &mut Obj, v: &View) {
    if let Some(value) = text_num(v, "listSpacing") {
        style.insert("listSpacing".into(), json!(value));
    }
    for k in ["hangingList", "hangingPunctuation"] {
        if let Some(value) = text_flag(v, k) {
            style.insert(k.into(), json!(value));
        }
    }
    if let Some(value) = leading_trim(v) {
        style.insert("leadingTrim".into(), json!(value));
    }
}

fn leading_trim<'a>(v: &View<'a>) -> Option<&'a str> {
    v.layers.iter().rev().find_map(|layer| match layer.get("leadingTrim").and_then(Kv::str) {
        Some(value @ ("NONE" | "CAP_HEIGHT")) => Some(value),
        _ => None,
    })
}

/// The chars that start a wrapped line in the layout the file saved with the text
/// (`derivedTextData`'s `baselines`, each line's `firstCharacter` in UTF-16 units), when that layout
/// is for these characters; lines after a hard break are left out (empty: it wraps nowhere). None
/// when the saved layout is missing, for other text or inconsistent.
fn saved_line_starts(v: &View, chars: &str) -> Option<Vec<usize>> {
    // Derived data from below the layer that set the characters was laid out for other text.
    let top = |k: &str| v.layers.iter().rposition(|l| l.get(k).is_some());
    if top("derivedTextData")? < top("textData")? {
        return None;
    }
    let baselines = v.get("derivedTextData")?.get("baselines")?.list();
    // UTF-16 offset of each char, and of the end.
    let mut offsets: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    let mut unit = 0;
    for c in chars.chars() {
        offsets.push(unit);
        unit += c.len_utf16();
    }
    offsets.push(unit);
    let list: Vec<char> = chars.chars().collect();
    let mut starts = Vec::new();
    let mut prev = None;
    for b in baselines {
        let u = num(b, "firstCharacter").filter(|u| *u >= 0.0 && u.fract() == 0.0 && *u < unit as f64)? as usize;
        let c = offsets.binary_search(&u).ok()?;
        // Lines start at the first char, then further on each time.
        if !prev.map_or(c == 0, |p| c > p) {
            return None;
        }
        prev = Some(c);
        if c > 0 && !list.get(c - 1).is_some_and(|ch| matches!(ch, '\n' | '\u{2028}')) {
            starts.push(c);
        }
    }
    prev.map(|_| starts)
}

impl<'a> Mapper<'a> {
    pub(super) fn text(&mut self, o: &mut Obj, v: &View<'a>) {
        let (key, legacy) = ("styleIdForText", "inheritTextStyleID");
        let style_node = self.style_link(v, key, legacy).and_then(|s| self.nodes.get(&s).copied());
        // Overrides above the one that links the style beat it; it beats the copies below.
        let sv = v.styled(style_node.filter(|s| s.get("styleType").and_then(Kv::str) == Some("TEXT")), key, legacy);
        let td = v.get("textData");
        let chars = td.and_then(|t| t.get("characters")).and_then(Kv::str).unwrap_or("");
        o.insert("characters".into(), json!(chars));
        // Where the file's own layout wrapped the lines (not a REST field).
        if let Some(starts) = saved_line_starts(v, chars) {
            o.insert("_figLineStarts".into(), json!(starts));
        }
        let mut style = text_style(&sv, None);
        // The style's typography beats the layer's own copy of it (the copy is stale when the style
        // changed). The file cannot tell a stale copy of these four values from a deliberate local
        // one, and "original", "none" and 0 could never be set locally if the style always won, so
        // a value the layer or an override sets itself is taken as deliberate.
        if let Some(decoration @ ("NONE" | "UNDERLINE" | "STRIKETHROUGH")) = v.str("textDecoration") {
            style.insert("textDecoration".into(), json!(decoration));
        }
        if let Some(skip_ink) = v.flag("textDecorationSkipInk") {
            style.insert("textDecorationSkipInk".into(), json!(skip_ink));
        }
        if let Some(case @ ("ORIGINAL" | "UPPER" | "LOWER" | "TITLE" | "SMALL_CAPS" | "SMALL_CAPS_FORCED")) = v.str("textCase") {
            style.insert("textCase".into(), json!(case));
        }
        for k in ["textAlignHorizontal", "textAlignVertical", "textAutoResize", "textTruncation"] {
            copy_str(&mut style, v, k, k);
        }
        // Paragraph settings belong to the effective text layer, including explicit zero clears.
        for k in ["paragraphSpacing", "paragraphIndent"] {
            copy_num(&mut style, v, k, k);
        }
        insert_text_options(&mut style, v);
        if let Some(n) = v.num("maxLines").filter(|n| *n >= 1.0) {
            style.insert("maxLines".into(), json!(n));
        }
        let size = sv.num("fontSize");
        if let Some(b) = o.get_mut("style").and_then(|s| s.get_mut("boundVariables")).map(Value::take) {
            style.insert("boundVariables".into(), b);
        }
        o.insert("style".into(), Value::Object(style));
        let Some(td) = td else { return };
        char_styles(o, td, size, &mut |w| self.warn(w));
        let raw = td.get("lines").map(Kv::list).unwrap_or_default();
        let lines: Vec<&str> = raw
            .iter()
            .map(|l| match l.get("lineType").and_then(Kv::str) {
                Some("ORDERED_LIST") => "ORDERED",
                Some("UNORDERED_LIST") => "UNORDERED",
                _ => "NONE",
            })
            .collect();
        if lines.iter().any(|l| *l != "NONE") {
            o.insert("lineTypes".into(), json!(lines));
        }
        // Where a numbered list starts counting, less one (not a REST field).
        let starts: Vec<u32> =
            raw.iter().map(|l| num(l, "listStartOffset").filter(|n| n.is_finite()).map_or(0, |n| n.clamp(0.0, f64::from(u32::MAX)) as u32)).collect();
        if starts.iter().any(|n| *n > 0) {
            o.insert("listStartOffsets".into(), json!(starts));
        }
    }

    /// Carry supported text-path data through the private .fig REST ingress only.
    pub(super) fn text_path(&mut self, o: &mut Obj, v: &View<'a>) {
        let fail = |this: &mut Self, reason: &str| {
            this.warn(format!("text path {reason}; text imports off its path"));
        };
        let Some(start) = v.get("textPathStart") else {
            fail(self, "has no start data");
            return;
        };
        let (Some(t), Some(forward)) = (num(start, "tValue"), start.get("forward").and_then(Kv::bool)) else {
            fail(self, "has invalid start data");
            return;
        };
        let Some(vd) = v.get("vectorData") else {
            fail(self, "has no vector data");
            return;
        };
        let Some(blob) = num(vd, "vectorNetworkBlob")
            .filter(|i| i.is_finite() && *i >= 0.0 && i.fract() == 0.0)
            .and_then(|i| usize::try_from(i as u64).ok())
            .and_then(|i| self.blobs.get(i))
            .copied()
        else {
            fail(self, "has no vector network");
            return;
        };
        let Some(net) = network::decode(blob) else {
            fail(self, "has damaged vector data");
            return;
        };
        let Some(norm) = vd.get("normalizedSize") else {
            fail(self, "has invalid normalized size");
            return;
        };
        let Some(size) = v.get("size") else {
            fail(self, "has invalid dimensions");
            return;
        };
        let (Some(nx), Some(ny), Some(w), Some(h)) = (num(norm, "x"), num(norm, "y"), num(size, "x"), num(size, "y")) else {
            fail(self, "has invalid dimensions");
            return;
        };
        let axis_scale = |size: f64, normalized: f64, values: &[f64]| -> Option<f64> {
            if !size.is_finite() || size < 0.0 || !normalized.is_finite() || normalized < 0.0 {
                return None;
            }
            if normalized > 1e-9 {
                if size <= 0.0 {
                    return None;
                }
                return Some(size / normalized);
            }
            (size == 0.0 && values.iter().all(|v| *v == 0.0)).then_some(1.0)
        };
        let xs: Vec<f64> = net.vertices.iter().map(|p| p.x).chain(net.segments.iter().flat_map(|s| [s.ta.0, s.tb.0])).collect();
        let ys: Vec<f64> = net.vertices.iter().map(|p| p.y).chain(net.segments.iter().flat_map(|s| [s.ta.1, s.tb.1])).collect();
        let sx = axis_scale(w, nx, &xs);
        let sy = axis_scale(h, ny, &ys);
        let (Some(sx), Some(sy)) = (sx, sy) else {
            fail(self, "has invalid path scale");
            return;
        };
        if v.str("textAlignVertical").is_some_and(|x| x != "TOP") {
            fail(self, "uses unsupported vertical placement");
            return;
        }
        if v.get("textData").and_then(|td| td.get("characters")).and_then(Kv::str).is_some_and(|s| s.chars().any(|c| matches!(c, '\n' | '\r'))) {
            fail(self, "contains multiple lines");
            return;
        }
        let path = match crate::fig::text_path::from_network(&net, sx, sy, t, forward) {
            Ok(path) => path,
            Err(reason) => {
                fail(self, reason);
                return;
            }
        };
        let has_decoration = |style: &Value| style.get("textDecoration").and_then(Value::as_str).is_some_and(|x| x != "NONE");
        let decorated = o.get("style").is_some_and(has_decoration)
            || o.get("styleOverrideTable").and_then(Value::as_object).into_iter().flat_map(|styles| styles.values()).any(has_decoration);
        if decorated {
            self.warn("text path underline or strikethrough decoration is not rendered");
        }
        o.insert("_figTextPath".into(), json!({"d": path.d, "start": path.start, "flip": path.flip}));
    }
}

/// Per-character styles of text data `td` (`characterStyleIDs` + `styleOverrideTable`) as REST
/// `characterStyleOverrides` + `styleOverrideTable`; `size`: the base font size.
pub(super) fn char_styles(o: &mut Obj, td: &Kv, size: Option<f64>, warn: &mut dyn FnMut(String)) {
    let ids: Vec<u64> = td.get("characterStyleIDs").map(Kv::list).unwrap_or_default().iter().filter_map(Kv::f64).map(|i| i as u64).collect();
    if ids.iter().all(|i| *i == 0) {
        return;
    }
    let mut table = Obj::new();
    for e in td.get("styleOverrideTable").map(Kv::list).unwrap_or_default() {
        let Some(sid) = num(e, "styleID") else { continue };
        let mut s = text_style(&View::of(e), size);
        if let Some(fills) = e.get("fillPaints") {
            s.insert("fills".into(), Value::Array(fills.list().iter().filter_map(|p| paint(p, warn)).collect()));
        }
        table.insert((sid as u64).to_string(), Value::Object(s));
    }
    o.insert("characterStyleOverrides".into(), json!(ids));
    o.insert("styleOverrideTable".into(), Value::Object(table));
}

/// The REST TypeStyle fields a node (or text override entry) sets. `size`: the font size that
/// letter spacing in percent refers to when the entry doesn't set one.
fn text_style(v: &View, size: Option<f64>) -> Obj {
    let mut s = Obj::new();
    if let Some(f) = v.get("fontName") {
        for (from, to) in [("family", "fontFamily"), ("style", "fontStyle"), ("postscript", "fontPostScriptName")] {
            if let Some(x) = f.get(from).and_then(Kv::str).filter(|x| !x.is_empty()) {
                // Compact and spaced style names ("SemiBold", "Semi Bold") are the same style.
                s.insert(to.into(), if from == "style" { json!(crate::fig::style_name(x)) } else { json!(x) });
            }
        }
    }
    copy_num(&mut s, v, "fontSize", "fontSize");
    copy_num(&mut s, v, "paragraphSpacing", "paragraphSpacing");
    copy_num(&mut s, v, "paragraphIndent", "paragraphIndent");
    insert_text_options(&mut s, v);
    copy_str(&mut s, v, "textCase", "textCase");
    copy_str(&mut s, v, "textDecoration", "textDecoration");
    if let Some(skip_ink) = v.flag("textDecorationSkipInk") {
        s.insert("textDecorationSkipInk".into(), json!(skip_ink));
    }
    copy_str(&mut s, v, "leadingTrim", "leadingTrim");
    let font_size = v.num("fontSize").or(size).unwrap_or(12.0);
    if let Some(ls) = v.get("letterSpacing") {
        let val = num(ls, "value").unwrap_or(0.0);
        let px = match ls.get("units").and_then(Kv::str) {
            Some("PIXELS") => val,
            Some("PERCENT") => {
                // Private (read by `rest/text.rs`): REST spacing is in pixels; keep the percent.
                s.insert("_letterSpacingPercent".into(), json!(val));
                val / 100.0 * font_size
            }
            _ => val * font_size,
        };
        s.insert("letterSpacing".into(), json!(px));
    }
    if let Some(lh) = v.get("lineHeight") {
        let val = num(lh, "value").unwrap_or(100.0);
        match lh.get("units").and_then(Kv::str) {
            Some("PIXELS") => {
                s.insert("lineHeightUnit".into(), json!("PIXELS"));
                s.insert("lineHeightPx".into(), json!(val));
            }
            // shortcut: percent units count from the font's own line height (100 % is "auto"), which
            // the document can only say for 100 %; other values read as % of the font size. Upgrade
            // when line heights can be a share of the font's own.
            Some("PERCENT") if (val - 100.0).abs() < 1e-6 => {
                s.insert("lineHeightUnit".into(), json!("INTRINSIC_%"));
            }
            Some("PERCENT") => {
                s.insert("lineHeightUnit".into(), json!("FONT_SIZE_%"));
                s.insert("lineHeightPercentFontSize".into(), json!(val));
            }
            _ => {
                s.insert("lineHeightUnit".into(), json!("FONT_SIZE_%"));
                s.insert("lineHeightPercentFontSize".into(), json!(val * 100.0));
            }
        }
    }
    // An empty URL is kept: on an override entry it clears the text's own link.
    if let Some(u) = v.get("hyperlink").and_then(|h| h.get("url")).and_then(Kv::str) {
        s.insert("hyperlink".into(), json!({"type": "URL", "url": u}));
    }
    s
}
