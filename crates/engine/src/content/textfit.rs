//! `text.fit`: text sized to fit its box, for posts and cards whose copy changes (a carousel
//! filled from data, a translated headline). The largest font size, down to a floor, at which the
//! text stays within `maxLines` lines and its box's height; then a last line that is a single word
//! (an orphan) is avoided by shrinking a little more, else by keeping the last two words together
//! with a no-break space. Every style run scales by the same factor, so mixed sizes keep their
//! proportions.

use figcraft_doc::{NodeKind, TextAutoResize, TextData};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::{boolean, num, req_targets};
use crate::{Result, Session, bad, spec};

const CMD: &str = "text.fit";

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "text.fit",
        "Fit text to its box",
        "{ids? (text layers; default the selection), maxLines? (default: no limit but the box height), minSize? (smallest font size, px; default 60% of the current), orphans? (avoid a one-word last line; default true)}  needs a fixed width (fixed or auto-height text)  → {fitted: [{id, fontSize, lines, fits, orphan}]}",
        true,
        fit
    )]
}

/// The layout of `t` with every run's font size times `k`, at `width`: (line count, height,
/// whether the last line is an orphan: one word, wrapped from the line above rather than a
/// paragraph of its own after a hard break, which no size can fix).
fn measure(t: &TextData, k: f64, width: f64) -> (usize, f64, bool) {
    let mut t = t.clone();
    // Every line counts, not only those the layer's own line limit shows.
    t.max_lines = 0;
    let n = t.char_count();
    t.restyle(0, n, |s| s.font_size = (s.font_size * k).clamp(1.0, 5000.0));
    let l = figcraft_text::layout(&t, Some(width), None);
    let orphan = l.lines.len() > 1
        && l.lines.last().is_some_and(|line| {
            let mut before = t.characters.chars().skip(line.char_start.saturating_sub(1));
            let wrapped = line.char_start > 0 && before.next() != Some('\n');
            wrapped
                && t.characters
                    .chars()
                    .skip(line.char_start)
                    .take(line.char_end.saturating_sub(line.char_start))
                    .collect::<String>()
                    .split_whitespace()
                    .count()
                    == 1
        });
    (l.lines.len(), l.height, orphan)
}

fn fit(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets(CMD, s, p)?;
    let max_lines = num(p, "maxLines").filter(|v| v.is_finite() && *v >= 1.0).map(|v| v.min(1000.0) as usize);
    let orphans = boolean(p, "orphans").unwrap_or(true);
    let mut out = Vec::new();
    let doc = s.doc_mut()?;
    for id in ids {
        let n = doc.node(id).ok_or_else(|| bad(CMD, format!("no layer {id}")))?;
        let NodeKind::Text(t) = &n.kind else { return Err(bad(CMD, format!("{id} is not text"))) };
        if t.auto_resize == TextAutoResize::WidthAndHeight {
            return Err(bad(CMD, format!("{id} grows to its text: give it a fixed width (auto height or fixed) first")));
        }
        let (width, height_cap) = (n.width, if t.auto_resize == TextAutoResize::None { n.height } else { f64::INFINITY });
        let size = t.style.font_size.max(1.0);
        let min = num(p, "minSize").filter(|v| v.is_finite() && *v >= 1.0).unwrap_or(size * 0.6).min(size);
        let fits = |m: (usize, f64, bool)| max_lines.is_none_or(|ml| m.0 <= ml) && m.1 <= height_cap + 0.5;
        let k = largest_fit(t, width, (size, min), &fits);
        let (k, nbsp) = if orphans { avoid_orphan(t, k, width, (size, min)) } else { (k, false) };
        let n = doc.get_mut(id)?;
        let NodeKind::Text(t) = &mut n.kind else { continue };
        let count = t.char_count();
        if k < 1.0 {
            // Each run rounds to a tenth, but not below its share of the floor.
            t.restyle(0, count, |st| {
                let floor = st.font_size * min / size;
                st.font_size = ((st.font_size * k * 10.0).round() / 10.0).max(floor).clamp(1.0, 5000.0)
            });
        }
        let after = if nbsp { keep_together(t, width, &fits) } else { measure(t, 1.0, width) };
        // Reported from the final text (rounded sizes, the no-break space), which callers rely on.
        out.push(json!({"id": id.0, "fontSize": (t.style.font_size * 10.0).round() / 10.0, "lines": after.0, "fits": fits(after), "orphan": orphans && after.2}));
    }
    Ok(json!({"fitted": out}))
}

/// The largest factor in [min/size, 1] at which `t` fits: a binary search to a tenth of a pixel,
/// then rounded down to a whole-tenth size (which reads better in the panel) but never below the
/// floor. Text that fits keeps its exact size.
fn largest_fit(t: &TextData, width: f64, (size, min): (f64, f64), fits: &dyn Fn((usize, f64, bool)) -> bool) -> f64 {
    let (mut lo, mut hi) = (min / size, 1.0);
    if fits(measure(t, 1.0, width)) {
        return 1.0;
    }
    if fits(measure(t, lo, width)) {
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if fits(measure(t, mid, width)) {
                lo = mid
            } else {
                hi = mid
            }
            if (hi - lo) * size < 0.1 {
                break;
            }
        }
    }
    ((size * lo * 10.0).floor() / 10.0).max(min) / size
}

/// Up to 8% smaller than `k` (not below the floor) may pull a word down to an orphaned last line.
/// The factor, and whether an orphan is left (to be kept together with a no-break space).
fn avoid_orphan(t: &TextData, mut k: f64, width: f64, (size, min): (f64, f64)) -> (f64, bool) {
    let mut orphan = measure(t, k, width).2;
    let mut tries = 0;
    while orphan && tries < 8 && size * k * 0.99 >= min {
        k *= 0.99;
        orphan = measure(t, k, width).2;
        tries += 1;
    }
    (k, orphan)
}

/// Keep the last two words together with a no-break space, unless that breaks the limit (an
/// orphan is better). Same character count, so style runs keep their ranges. The final layout.
fn keep_together(t: &mut TextData, width: f64, fits: &dyn Fn((usize, f64, bool)) -> bool) -> (usize, f64, bool) {
    // Only inside the last paragraph: a space before a hard break joins nothing on the last line.
    let para = t.characters.rfind('\n').map_or(0, |i| i + 1);
    let Some(i) = t.characters.trim_end().get(para..).and_then(|p| p.rfind(' ')).map(|i| para + i) else {
        return measure(t, 1.0, width);
    };
    t.characters.replace_range(i..i + 1, "\u{a0}");
    let after = measure(t, 1.0, width);
    if fits(after) {
        return after;
    }
    t.characters.replace_range(i..i + '\u{a0}'.len_utf8(), " ");
    measure(t, 1.0, width)
}
