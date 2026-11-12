//! `design.fix`: repair the findings `design.audit` marks fixable, as one undo step.
//!
//! Every repair is the safe, unambiguous one the audit already suggested: spacing snapped to the
//! 4px grid, radii to the scale, heavy headings to Semi Bold, italic and negative tracking removed,
//! glows taken out, shadow colours neutralised and strong borders thinned. Content rules (copy,
//! naming) and gradients are left alone: those need a decision, not arithmetic.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_color::{Color, PaintKind};
use figcraft_doc::{DesignPreset, Document, EffectKind, LayoutMode, LetterSpacing, Node, NodeId, NodeKind, TextData, TextStyle};
use serde_json::{Value, json};

use super::audit::{Report, Settings, backdrop, collect, is_colored_shadow, is_glow, text_color};

/// Fix what `roots` made fixable among `filter`'s rules. With `exact_ids`, only findings on those
/// layers are repaired, not findings on their descendants. The walk stops at `max_nodes` nodes.
pub(crate) fn run(
    doc: &mut Document,
    roots: &[NodeId],
    anchor: Option<NodeId>,
    anchor_path: &str,
    scope: &str,
    preset: Option<DesignPreset>,
    filter: Option<&BTreeSet<String>>,
    exact_ids: Option<&[NodeId]>,
    max_nodes: usize,
    dry_run: bool,
) -> Value {
    let settings = Settings::new(doc, preset);
    let before = collect(doc, roots, anchor, anchor_path, scope, &settings, max_nodes);
    let targets: Vec<(NodeId, &'static str)> = before
        .findings
        .iter()
        .filter(|f| f.fixable && settings.enabled(f.rule))
        .filter(|f| filter.is_none_or(|set| set.contains(f.rule)))
        .filter(|f| exact_ids.is_none_or(|ids| ids.contains(&f.node)))
        .map(|f| (f.node, f.rule))
        .collect();
    let mut by_rule: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut changed = 0usize;
    let mut skipped = 0usize;
    if dry_run {
        changed = targets.len();
        for (_, rule) in &targets {
            *by_rule.entry(*rule).or_default() += 1;
        }
    } else {
        for (id, rule) in &targets {
            match apply(doc, *id, rule, &settings) {
                Some(true) => {
                    changed += 1;
                    *by_rule.entry(*rule).or_default() += 1;
                }
                _ => skipped += 1,
            }
        }
    }
    let after = collect(doc, roots, anchor, anchor_path, scope, &settings, max_nodes);
    json!({
        "scope": scope,
        "dryRun": dry_run,
        "changed": changed,
        "skipped": skipped,
        "byRule": by_rule,
        "before": tally(&before, &settings),
        "after": tally(&after, &settings),
    })
}

fn tally(report: &Report, settings: &Settings) -> Value {
    let shown: Vec<&super::audit::Finding> = report.findings.iter().filter(|f| settings.enabled(f.rule)).collect();
    let mut counts = [0usize; 3];
    for f in &shown {
        counts[f.severity.index()] += 1;
    }
    let score = (100.0 - shown.iter().map(|f| f.severity.penalty()).sum::<f64>()).clamp(0.0, 100.0).round();
    json!({
        "score": score,
        "counts": {"high": counts[2], "medium": counts[1], "low": counts[0]},
        "nodes": report.nodes,
        "findings": shown.len(),
    })
}

/// Apply one finding's repair. `None` when the layer is gone or the repair does not apply to it;
/// `Some(false)` when it applied to nothing (a no-op the caller counts as skipped).
fn apply(doc: &mut Document, id: NodeId, rule: &str, settings: &Settings) -> Option<bool> {
    if rule == "color.contrast" {
        return apply_contrast(doc, id);
    }
    let n = doc.node_mut(id)?;
    match rule {
        "spacing.off-grid" => apply_spacing(n, settings),
        "radius.off-scale" => {
            if ["cornerRadius", "topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"].iter().any(|k| n.bound.contains_key(*k)) {
                return Some(false);
            }
            apply_radius(n, settings)
        }
        "type.weight" | "type.italic" | "type.negative-tracking" => {
            if text_styled(n) || ["fontSize", "fontStyle", "fontWeight", "letterSpacing"].iter().any(|k| n.bound.contains_key(*k)) {
                return Some(false);
            }
            match rule {
                "type.weight" => apply_text(n, cap_weight),
                "type.italic" => apply_text(n, unitalic),
                _ => apply_text(n, zero_tracking),
            }
        }
        "color.glow" | "color.colored-shadow" => {
            if n.styles.effect.is_some() || n.bound.keys().any(|k| k.starts_with("effects/")) {
                return Some(false);
            }
            if rule == "color.glow" { apply_glow(n) } else { apply_neutral_shadows(n) }
        }
        "color.border-opacity" => {
            if n.styles.stroke.is_some() || n.bound.keys().any(|k| k.starts_with("strokes/")) {
                return Some(false);
            }
            apply_border(n)
        }
        _ => Some(false),
    }
}

/// A text layer whose type comes from a text style (the whole layer or one run).
fn text_styled(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Text(t) => n.styles.text.is_some() || t.style.style_id.is_some() || t.runs.iter().any(|r| r.style.style_id.is_some()),
        _ => false,
    }
}

/// Darken (or lighten, on a dark backdrop) the text colour until it reaches the WCAG ratio. The
/// hue is kept as far as the blend allows; when even black or white cannot reach the ratio, the
/// text is left as it is and counted as skipped.
fn apply_contrast(doc: &mut Document, id: NodeId) -> Option<bool> {
    let n = doc.node(id)?;
    let NodeKind::Text(t) = &n.kind else { return Some(false) };
    if n.styles.fill.is_some() || n.bound.keys().any(|k| k.starts_with("fills/")) {
        return Some(false);
    }
    let fg = text_color(n, t)?;
    let bg = backdrop(doc, n)?;
    let size = t.style.font_size;
    let large = size >= 24.0 || (size >= 18.0 && t.style.weight() >= 700);
    let need = if large { 3.0 } else { 4.5 };
    if fg.contrast(bg) >= need {
        return Some(false);
    }
    let fixed = accessible(fg, bg, need);
    if fixed.contrast(bg) < need {
        return Some(false);
    }
    let n = doc.node_mut(id)?;
    let NodeKind::Text(t) = &mut n.kind else { return Some(false) };
    let paints = match &mut t.style.fills {
        Some(fills) => fills,
        None => &mut n.fills,
    };
    let paint = paints.iter_mut().rev().find(|p| p.visible && matches!(&p.kind, PaintKind::Solid { .. }))?;
    paint.opacity = 1.0;
    if let PaintKind::Solid { color } = &mut paint.kind {
        *color = fixed.with_alpha(fg.a);
        Some(true)
    } else {
        Some(false)
    }
}

/// The closest shade of `fg` towards black (light backdrop) or white (dark backdrop) that meets
/// `need` against `bg`, by bisection.
fn accessible(fg: Color, bg: Color, need: f32) -> Color {
    let target = if bg.luminance() > 0.5 { Color::BLACK } else { Color::WHITE };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut best = target;
    for _ in 0..24 {
        let t = (lo + hi) / 2.0;
        let c = Color::new(fg.r + (target.r - fg.r) * t, fg.g + (target.g - fg.g) * t, fg.b + (target.b - fg.b) * t, 1.0);
        if c.contrast(bg) >= need {
            best = c;
            hi = t;
        } else {
            lo = t;
        }
    }
    best
}

fn apply_spacing(n: &mut Node, settings: &Settings) -> Option<bool> {
    // Any spacing binding owns the whole layout value: fixing one field next to a token would
    // leave a mixed layout and is not what the audit promised.
    if [
        "gap",
        "columnGap",
        "rowGap",
        "crossGap",
        "padding",
        "paddingHorizontal",
        "paddingVertical",
        "paddingTop",
        "paddingRight",
        "paddingBottom",
        "paddingLeft",
    ]
    .iter()
    .any(|k| n.bound.contains_key(*k))
    {
        return Some(false);
    }
    let al = n.layout.as_mut()?;
    let mut changed = false;
    if al.mode != LayoutMode::Grid {
        let gap = settings.snap(al.gap);
        if (gap - al.gap).abs() > 1e-6 {
            al.gap = gap;
            changed = true;
        }
        if al.wrap {
            let cross = settings.snap(al.cross_gap);
            if (cross - al.cross_gap).abs() > 1e-6 {
                al.cross_gap = cross;
                changed = true;
            }
        }
    }
    for side in &mut al.padding {
        let value = settings.snap(*side);
        if (value - *side).abs() > 1e-6 {
            *side = value;
            changed = true;
        }
    }
    Some(changed)
}

fn apply_radius(n: &mut Node, settings: &Settings) -> Option<bool> {
    let (w, h) = (n.width, n.height);
    let pill = w.min(h);
    let mut changed = false;
    for r in &mut n.corners.radii {
        if *r > 0.01 && !settings.radius_ok(*r, w, h) {
            *r = if pill > 0.0 && (*r - pill / 2.0).abs() <= 1.0 { pill / 2.0 } else { settings.snap_radius(*r) };
            changed = true;
        }
    }
    Some(changed)
}

/// Run `f` over a text layer's base style and its runs; true when any style changed.
fn apply_text(n: &mut Node, f: fn(&mut TextStyle) -> bool) -> Option<bool> {
    let NodeKind::Text(t) = &mut n.kind else { return Some(false) };
    let TextData { style, runs, .. } = t;
    let mut changed = f(style);
    for run in runs {
        changed |= f(&mut run.style);
    }
    Some(changed)
}

fn cap_weight(s: &mut TextStyle) -> bool {
    if s.font_size >= 20.0 && s.weight() > 600 {
        s.set_font_style("Semi Bold");
        true
    } else {
        false
    }
}

fn unitalic(s: &mut TextStyle) -> bool {
    if !s.italic() {
        return false;
    }
    let words: Vec<&str> = s.font_style.split_whitespace().filter(|w| !w.eq_ignore_ascii_case("italic")).collect();
    let name = if words.is_empty() { "Regular".to_string() } else { words.join(" ") };
    s.set_font_style(name);
    true
}

fn zero_tracking(s: &mut TextStyle) -> bool {
    let px = s.letter_spacing.resolve(s.font_size);
    if s.font_size > 0.0 && s.font_size < 24.0 && px.is_finite() && px < -0.01 {
        s.letter_spacing = LetterSpacing::Percent(0.0);
        true
    } else {
        false
    }
}

fn apply_glow(n: &mut Node) -> Option<bool> {
    let before = n.effects.len();
    n.effects.retain(|e| !is_glow(e));
    Some(n.effects.len() != before)
}

fn apply_neutral_shadows(n: &mut Node) -> Option<bool> {
    let mut changed = false;
    for e in &mut n.effects {
        if let EffectKind::DropShadow(s) | EffectKind::InnerShadow(s) = &mut e.kind
            && is_colored_shadow(s)
        {
            s.color = Color::new(0.0, 0.0, 0.0, s.color.a);
            changed = true;
        }
    }
    Some(changed)
}

fn apply_border(n: &mut Node) -> Option<bool> {
    let mut changed = false;
    for p in &mut n.strokes {
        if p.visible && p.opacity > 0.08 {
            p.opacity = 0.08;
            changed = true;
        }
    }
    Some(changed)
}
