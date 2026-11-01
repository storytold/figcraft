//! CSS colour functions usvg doesn't read, resolved before it parses the document (it would paint
//! them black, or leave the stroke out): `color()` in `srgb` or `display-p3`, converted to the
//! document's profile, and `color-mix()` in `srgb`, `oklab` or `oklch`, evaluated here. Each call
//! in a colour attribute, a `style` attribute or a `<style>` sheet becomes a hex or `rgba()` colour.
//! Other colour spaces are approximated (a note says how), and a call that can't be read becomes a
//! mid grey with a note, never a silent black.
//!
//! Colours without a space of their own (hex, `rgb()`, names…) are values in the document's
//! profile, as everywhere else in an import; `color()` says its space, so it is converted. The
//! OKLab and OKLCH mixes use the sRGB formulas.

use std::ops::Range;

use figcraft_color::{Color, ColorProfile, Oklch};

/// What a call that can't be read becomes: a mid grey, so it doesn't pass for a black on purpose.
const FALLBACK: Color = Color::from_rgb8_const(0x80, 0x80, 0x80);
/// Most calls rewritten in one document.
const MAX_CALLS: usize = 100_000;
/// Most `color-mix()` calls inside one another.
const MAX_NESTING: usize = 8;
/// Longest call read: a colour is short, so a longer one is garbage.
const MAX_CALL_LEN: usize = 2048;
/// Chroma under which an OKLCH hue means nothing (a grey).
const GREY_CHROMA: f32 = 0.000_2;

/// The attributes that can hold a colour (with `style`, which can hold any property).
const COLOR_ATTRS: [&str; 8] = ["fill", "stroke", "stop-color", "flood-color", "lighting-color", "color", "solid-color", "style"];

/// A colour read from a call, with why it is only an approximation; or why it can't be read.
type Eval = Result<(Color, Option<&'static str>), &'static str>;

/// `data` with its `color()` and `color-mix()` calls replaced by colours usvg reads, and notes on
/// those it read only approximately or not at all; `None` when there are none (or `data` isn't
/// UTF-8 XML).
pub(crate) fn resolve(data: &[u8], profile: ColorProfile) -> Option<(Vec<u8>, Vec<String>)> {
    find_call(data, 0)?;
    let text = std::str::from_utf8(data).ok()?;
    let opt = usvg::roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let xml = usvg::roxmltree::Document::parse_with_options(text, opt).ok()?;
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut notes = Vec::new();
    for range in color_ranges(&xml) {
        rewrite_calls(text, range, profile, &mut edits, &mut notes);
    }
    if edits.len() >= MAX_CALLS {
        notes.push(format!("only the first {MAX_CALLS} color() and color-mix() colours were read; the rest were left as they were"));
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (r, with) in edits {
        // Ranges never overlap (one per attribute or sheet); a stray one is skipped.
        let Some(before) = text.get(at..r.start) else { continue };
        out.push_str(before);
        out.push_str(&with);
        at = r.end;
    }
    out.push_str(text.get(at..).unwrap_or_default());
    Some((out.into_bytes(), notes))
}

/// Where colours can be: the values of colour and `style` attributes and the text of `<style>`
/// elements (byte ranges in the document).
fn color_ranges(xml: &usvg::roxmltree::Document) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for n in xml.descendants().filter(|n| n.is_element()) {
        out.extend(n.attributes().filter(|a| COLOR_ATTRS.contains(&a.name())).map(|a| a.range_value()));
        if n.tag_name().name() == "style" {
            out.extend(n.children().filter(|c| c.is_text()).map(|c| c.range()));
        }
    }
    out
}

/// Replace the calls in `range` of `text`, up to [`MAX_CALLS`] in all.
fn rewrite_calls(text: &str, range: Range<usize>, profile: ColorProfile, edits: &mut Vec<(Range<usize>, String)>, notes: &mut Vec<String>) {
    let Some(s) = text.get(range.clone()) else { return };
    let mut from = 0;
    while edits.len() < MAX_CALLS {
        let Some((start, open)) = find_call(s.as_bytes(), from) else { return };
        let Some(end) = closing(s, open) else {
            // An open call takes the rest of the value with it, as a CSS parser drops it.
            notes.push(note(s.get(start..).unwrap_or_default(), "it isn't closed (or is too long to be a colour)"));
            edits.push((range.start + start..range.end, FALLBACK.to_css()));
            return;
        };
        let call = s.get(start..end).unwrap_or_default();
        let color = match eval(call, profile, 0) {
            Ok((c, None)) => c,
            Ok((c, Some(why))) => {
                notes.push(note(call, why));
                c
            }
            Err(why) => {
                notes.push(note(call, why));
                FALLBACK
            }
        };
        edits.push((range.start + start..range.start + end, color.to_css()));
        from = end;
    }
}

fn note(call: &str, why: &str) -> String {
    let shown: String = call.chars().take(80).collect();
    let more = if shown.len() < call.len() { "…" } else { "" };
    format!("the colour `{shown}{more}`: {why}")
}

/// The next `color(` or `color-mix(` (any case) at or after `from` that starts a name: its start
/// and the index of its `(`.
fn find_call(s: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while let Some(rest) = s.get(i..) {
        let k = i + rest.iter().position(|b| b.eq_ignore_ascii_case(&b'c'))?;
        let starts_name = k == 0 || s.get(k - 1).is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_'));
        let at = |name: &[u8]| s.get(k..k + name.len()).is_some_and(|w| w.eq_ignore_ascii_case(name));
        if starts_name && at(b"color(") {
            return Some((k, k + 5));
        }
        if starts_name && at(b"color-mix(") {
            return Some((k, k + 9));
        }
        i = k + 1;
    }
    None
}

/// The index just past the `)` that closes the `(` at `open`, within [`MAX_CALL_LEN`].
fn closing(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, b) in s.bytes().enumerate().skip(open).take(MAX_CALL_LEN) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// The colour of one `color()` or `color-mix()` call, `depth` mixes deep.
fn eval(call: &str, profile: ColorProfile, depth: usize) -> Eval {
    let lower = call.trim().to_ascii_lowercase();
    if let Some(args) = lower.strip_prefix("color-mix(").and_then(|r| r.strip_suffix(')')) {
        return mix(args, profile, depth);
    }
    match lower.strip_prefix("color(").and_then(|r| r.strip_suffix(')')) {
        Some(args) => color_fn(args, profile),
        None => Err("not a colour function"),
    }
}

/// `color(<space> r g b [/ alpha])`.
fn color_fn(args: &str, profile: ColorProfile) -> Eval {
    let (channels, alpha) = match args.split_once('/') {
        Some((c, a)) => (c, Some(a.trim())),
        None => (args, None),
    };
    let words: Vec<&str> = channels.split_whitespace().collect();
    let [space, r, g, b] = words.as_slice() else {
        return Err(if words.first() == Some(&"from") { "relative colours aren't read" } else { "a colour space and three values were expected" });
    };
    let value = |p: &str| number(p).ok_or("a value isn't a number or a percentage");
    let a = match alpha {
        Some(a) => value(a)?,
        None => 1.0,
    };
    let c = Color::new(value(r)?, value(g)?, value(b)?, a);
    let (from, why) = match *space {
        "srgb" => (ColorProfile::Srgb, None),
        "display-p3" => (ColorProfile::DisplayP3, None),
        _ => (ColorProfile::Srgb, Some("only the srgb and display-p3 spaces are read: its values were taken as sRGB")),
    };
    Ok((from.conversion_to(profile).apply(c), why))
}

/// A CSS number, a percentage (100% is 1) or `none` (0); finite only.
fn number(p: &str) -> Option<f32> {
    let v = match p.strip_suffix('%') {
        _ if p == "none" => 0.0,
        Some(pc) => pc.parse::<f32>().ok()? / 100.0,
        None => p.parse::<f32>().ok()?,
    };
    v.is_finite().then_some(v)
}

/// The space a `color-mix()` interpolates in.
#[derive(Clone, Copy)]
enum Space {
    Srgb,
    Oklab,
    Oklch,
}

/// `color-mix(in <space>, <colour> [p%], <colour> [q%])`, per CSS Color 5: missing percentages
/// make up 100% (50% each when both are missing), a sum under 100% lowers the alpha, and the
/// colours mix premultiplied by their alpha (an OKLCH hue takes the shorter way round).
fn mix(args: &str, profile: ColorProfile, depth: usize) -> Eval {
    if depth >= MAX_NESTING {
        return Err("colour mixes are nested too deep");
    }
    let parts = split_top(args, |c| c == ',');
    let [method, first, second] = parts.as_slice() else { return Err("color-mix() takes a method and two colours") };
    let words: Vec<&str> = method.split_whitespace().collect();
    let (space, mut why) = match words.as_slice() {
        ["in", "srgb"] => (Space::Srgb, None),
        ["in", "oklab"] => (Space::Oklab, None),
        ["in", "oklch"] | ["in", "oklch", "shorter", "hue"] => (Space::Oklch, None),
        ["in", ..] => (Space::Srgb, Some("only srgb, oklab and oklch mixes are read: it was mixed in sRGB")),
        _ => return Err("color-mix() starts with `in` and a colour space"),
    };
    let (a, pa, why_a) = operand(first, profile, depth)?;
    let (b, pb, why_b) = operand(second, profile, depth)?;
    why = why.or(why_a).or(why_b);
    if [pa, pb].into_iter().flatten().any(|p| !(0.0..=100.0).contains(&p)) {
        return Err("mix percentages are between 0% and 100%");
    }
    let (pa, pb) = match (pa, pb) {
        (None, None) => (50.0, 50.0),
        (Some(p), None) => (p, 100.0 - p),
        (None, Some(q)) => (100.0 - q, q),
        (Some(p), Some(q)) => (p, q),
    };
    let sum = pa + pb;
    if sum <= 0.0 {
        return Err("mix percentages add up to 0%");
    }
    let mixed = interpolate(space, a, b, pb / sum);
    Ok((mixed.with_alpha(mixed.a * (sum / 100.0).min(1.0)), why))
}

/// One colour of a mix and its percentage (before or after it).
fn operand(s: &str, profile: ColorProfile, depth: usize) -> Result<(Color, Option<f32>, Option<&'static str>), &'static str> {
    let words = split_top(s, char::is_whitespace);
    let pct = |w: &str| w.strip_suffix('%').and_then(|p| p.parse::<f32>().ok()).filter(|p| p.is_finite());
    let (color, p) = match words.as_slice() {
        [c] => (*c, None),
        [c, p] if pct(p).is_some() => (*c, pct(p)),
        [p, c] if pct(p).is_some() => (*c, pct(p)),
        _ => return Err("a mix colour is a colour and an optional percentage"),
    };
    let (c, why) = if color.starts_with("color(") || color.starts_with("color-mix(") {
        eval(color, profile, depth + 1)?
    } else {
        (Color::parse_css(color).ok_or("a mix colour isn't read")?, None)
    };
    Ok((c, p, why))
}

/// `a` and `b` mixed `t` of the way to `b`, premultiplied by their alpha.
fn interpolate(space: Space, a: Color, b: Color, t: f32) -> Color {
    let alpha = a.a + (b.a - a.a) * t;
    // Premultiplied, then back; a fully transparent mix keeps the plain mix of the channels.
    let lerp = |x: f32, y: f32| {
        let v = x * a.a + (y * b.a - x * a.a) * t;
        if alpha > 0.0 { v / alpha } else { x + (y - x) * t }
    };
    let out = match space {
        Space::Srgb => Color::new(lerp(a.r, b.r), lerp(a.g, b.g), lerp(a.b, b.b), 1.0),
        Space::Oklab => {
            let ([la, xa, ya], [lb, xb, yb]) = (oklab(a), oklab(b));
            let (l, x, y) = (lerp(la, lb), lerp(xa, xb), lerp(ya, yb));
            Oklch { l, c: x.hypot(y), h: y.atan2(x).to_degrees() }.to_color()
        }
        Space::Oklch => {
            let (ca, cb) = (Oklch::from_color(a), Oklch::from_color(b));
            // A grey has no hue: it takes the other colour's.
            let (mut ha, mut hb) = (if ca.c < GREY_CHROMA { cb.h } else { ca.h }, if cb.c < GREY_CHROMA { ca.h } else { cb.h });
            if hb - ha > 180.0 {
                ha += 360.0;
            } else if ha - hb > 180.0 {
                hb += 360.0;
            }
            Oklch { l: lerp(ca.l, cb.l), c: lerp(ca.c, cb.c), h: (ha + (hb - ha) * t).rem_euclid(360.0) }.to_color()
        }
    };
    out.with_alpha(alpha)
}

/// OKLab lightness and the two axes of `c`.
fn oklab(c: Color) -> [f32; 3] {
    let o = Oklch::from_color(c);
    let (sin, cos) = o.h.to_radians().sin_cos();
    [o.l, o.c * cos, o.c * sin]
}

/// `s` split at the characters `sep` picks outside parentheses (empty parts dropped).
fn split_top(s: &str, sep: impl Fn(char) -> bool) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && sep(c) => {
                out.push(s.get(start..i).unwrap_or_default());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(s.get(start..).unwrap_or_default());
    out.into_iter().map(str::trim).filter(|p| !p.is_empty()).collect()
}
