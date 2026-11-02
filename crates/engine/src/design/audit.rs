//! Walking the scope and turning what it finds into findings.
//!
//! Nothing here edits the document: `design.audit` reads pages and reports. The walk stops at an
//! instance's children (they mirror the main component, which is audited where it lives), skips
//! hidden layers and caps the number of nodes so a huge file cannot stall the app.

use std::collections::BTreeSet;

use figcraft_color::{Color, Paint, PaintKind};
use figcraft_doc::{Document, Effect, EffectKind, LayoutMode, Node, NodeId, NodeKind, Shadow, StrokeAlign, TextAutoResize, TextData};
use serde_json::{Map, Value, json};

use super::rules::{Severity, rule};

/// Most layers one audit looks at; past it the report says `truncated`.
const MAX_NODES: usize = 50_000;
/// Most text layers one audit lays out for the overflow check (text layout is the expensive part).
const MAX_TEXT_MEASURED: usize = 2_000;

/// One finding, owned so `design.fix` can act on it after the walk.
#[derive(Clone)]
pub(crate) struct Finding {
    pub(crate) rule: &'static str,
    pub(crate) severity: Severity,
    pub(crate) node: NodeId,
    pub(crate) path: String,
    pub(crate) message: String,
    pub(crate) fixable: bool,
    pub(crate) suggestion: Option<Value>,
}

impl Finding {
    fn json(&self) -> Value {
        let mut v = json!({
            "rule": self.rule,
            "severity": self.severity.id(),
            "node": self.node,
            "path": self.path,
            "message": self.message,
            "fixable": self.fixable,
        });
        if let (Some(s), Some(o)) = (&self.suggestion, v.as_object_mut()) {
            o.insert("suggestion".into(), s.clone());
        }
        v
    }
}

/// What one walk found.
pub(crate) struct Report {
    pub(crate) findings: Vec<Finding>,
    pub(crate) nodes: usize,
    pub(crate) truncated: bool,
}

struct Ctx<'a> {
    doc: &'a Document,
    findings: Vec<Finding>,
    visited: usize,
    text_nodes: usize,
    /// Font sizes in half-pixels, so 12.5 stays distinct from 12.
    sizes: BTreeSet<i64>,
    families: BTreeSet<String>,
    /// Layers carrying at least one shadow.
    shadowed: usize,
    /// Text layers laid out for the overflow check.
    measured: usize,
    truncated: bool,
}

/// Walk `roots` and collect every finding. `design.audit` reports them; `design.fix` acts on the
/// ones marked fixable.
pub(crate) fn collect(doc: &Document, roots: &[NodeId], anchor: Option<NodeId>, anchor_path: &str, scope: &str) -> Report {
    let mut ctx = Ctx {
        doc,
        findings: Vec::new(),
        visited: 0,
        text_nodes: 0,
        sizes: BTreeSet::new(),
        families: BTreeSet::new(),
        shadowed: 0,
        measured: 0,
        truncated: false,
    };
    for root in roots {
        ctx.walk(*root);
    }
    ctx.scope_rules(anchor, anchor_path, scope);
    Report { findings: ctx.findings, nodes: ctx.visited, truncated: ctx.truncated }
}

/// Audit `roots` (pages, frames or layers), labelled `scope` in the report. Aggregate findings
/// (too many type sizes…) point at `anchor`.
pub(crate) fn run(
    doc: &Document,
    roots: &[NodeId],
    scope: &str,
    anchor: Option<NodeId>,
    anchor_path: &str,
    min: Severity,
    filter: Option<&BTreeSet<String>>,
) -> Value {
    let report = collect(doc, roots, anchor, anchor_path, scope);
    let shown: Vec<&Finding> =
        report.findings.iter().filter(|f| f.severity >= min).filter(|f| filter.is_none_or(|set| set.contains(f.rule))).collect();
    let mut counts = [0usize; 3];
    for f in &shown {
        counts[f.severity.index()] += 1;
    }
    let score = (100.0 - shown.iter().map(|f| f.severity.penalty()).sum::<f64>()).clamp(0.0, 100.0).round();
    let findings: Vec<Value> = shown.into_iter().map(Finding::json).collect();
    json!({
        "scope": scope,
        "score": score,
        "counts": {"high": counts[2], "medium": counts[1], "low": counts[0]},
        "nodes": report.nodes,
        "findings": findings,
        "truncated": report.truncated,
    })
}

impl Ctx<'_> {
    fn walk(&mut self, root: NodeId) {
        let mut stack = vec![(root, String::new())];
        while let Some((id, parent)) = stack.pop() {
            if self.visited >= MAX_NODES {
                self.truncated = true;
                return;
            }
            let Some(n) = self.doc.node(id) else { continue };
            if !n.visible || n.opacity <= 0.001 {
                continue;
            }
            self.visited += 1;
            let name = label(n);
            let path = if parent.is_empty() { name } else { format!("{parent} / {name}") };
            if !n.is_page() {
                self.check_node(n, &path);
            }
            // An instance's children mirror its main component; audit the main where it lives.
            if matches!(&n.kind, NodeKind::Instance { .. }) {
                continue;
            }
            for child in n.children.iter().rev() {
                stack.push((*child, path.clone()));
            }
        }
    }

    fn check_node(&mut self, n: &Node, path: &str) {
        self.check_naming(n, path);
        self.check_spacing(n, path);
        self.check_radius(n, path);
        self.check_paints(n, path);
        self.check_effects(n, path);
        if let NodeKind::Text(t) = &n.kind {
            self.check_text(n, t, path);
        }
    }

    fn check_naming(&mut self, n: &Node, path: &str) {
        if is_default_name(&n.name) {
            self.push("naming.auto", n.id, path, format!("Layer still has its default name \"{}\"", n.name.trim()), None);
        }
    }

    fn check_spacing(&mut self, n: &Node, path: &str) {
        let Some(al) = &n.layout else { return };
        let mut off = Vec::new();
        let mut suggestion = Map::new();
        if al.mode != LayoutMode::Grid {
            if !on_grid(al.gap) {
                off.push(format!("gap {}", al.gap));
                suggestion.insert("gap".into(), json!(snap(al.gap)));
            }
            if al.wrap && !on_grid(al.cross_gap) {
                off.push(format!("cross gap {}", al.cross_gap));
                suggestion.insert("crossGap".into(), json!(snap(al.cross_gap)));
            }
        }
        let sides = ["top", "right", "bottom", "left"];
        let mut padding_off = false;
        for (i, side) in sides.iter().enumerate() {
            if let Some(v) = al.padding.get(i).copied().filter(|v| !on_grid(*v)) {
                off.push(format!("padding {side} {v}"));
                padding_off = true;
            }
        }
        if padding_off {
            suggestion.insert("padding".into(), json!(al.padding.map(snap)));
        }
        if !off.is_empty() {
            let suggestion = if suggestion.is_empty() { None } else { Some(Value::Object(suggestion)) };
            self.push("spacing.off-grid", n.id, path, format!("Off the 4px grid: {}", off.join(", ")), suggestion);
        }
    }

    fn check_radius(&mut self, n: &Node, path: &str) {
        let mut bad: Vec<f64> = n.corners.radii.iter().copied().filter(|r| *r > 0.01 && !radius_ok(*r, n.width, n.height)).collect();
        if bad.is_empty() {
            return;
        }
        bad.sort_by(f64::total_cmp);
        bad.dedup_by(|a, b| (*a - *b).abs() <= 0.01);
        let shown: Vec<String> = bad.iter().map(|r| r.to_string()).collect();
        let suggestion = json!({"cornerRadius": snap_radius(bad.first().copied().unwrap_or(0.0))});
        self.push(
            "radius.off-scale",
            n.id,
            path,
            format!("Corner radius off the scale: {} (use 4, 8, 12, 16, 20… or a pill)", shown.join(", ")),
            Some(suggestion),
        );
    }

    fn check_paints(&mut self, n: &Node, path: &str) {
        let is_text = matches!(&n.kind, NodeKind::Text(_));
        let fills: &[Paint] = match &n.kind {
            NodeKind::Text(t) => t.style.fills.as_deref().unwrap_or(&n.fills),
            _ => &n.fills,
        };
        let fills_gradient = fills.iter().any(|p| p.visible && matches!(&p.kind, PaintKind::Gradient(_)));
        let strokes_gradient = n.strokes.iter().any(|p| p.visible && matches!(&p.kind, PaintKind::Gradient(_)));
        if fills_gradient || strokes_gradient {
            let id = if is_text { "color.gradient-text" } else { "color.gradient" };
            self.push(id, n.id, path, "Painted with a gradient (a hue-to-hue wash is the usual AI tell)".into(), None);
        }
        if has_stroke(n) && border_is_strong(self.doc, n) {
            self.push(
                "color.border-opacity",
                n.id,
                path,
                "Border is more opaque than a hairline (6–10% is enough)".into(),
                Some(json!({"strokeOpacity": 0.08})),
            );
        }
    }

    fn check_effects(&mut self, n: &Node, path: &str) {
        let mut glow = false;
        let mut colored = false;
        let mut shadows = 0;
        for e in n.effects.iter().filter(|e| e.visible) {
            if let EffectKind::DropShadow(s) | EffectKind::InnerShadow(s) = &e.kind {
                shadows += 1;
                glow |= is_glow(e);
                colored |= is_colored_shadow(s);
            }
        }
        if shadows > 0 {
            self.shadowed += 1;
        }
        if glow {
            self.push(
                "color.glow",
                n.id,
                path,
                "Shadow has no offset and a large blur, so it reads as a glow".into(),
                Some(json!({"removeEffects": ["dropShadow", "innerShadow"]})),
            );
        }
        if colored {
            self.push(
                "color.colored-shadow",
                n.id,
                path,
                "Shadow carries a visible hue; shadows read as depth only when neutral".into(),
                Some(json!({"neutralShadow": true})),
            );
        }
    }

    fn check_text(&mut self, n: &Node, t: &TextData, path: &str) {
        let chars = t.characters.trim();
        let s = &t.style;
        let size = if s.font_size.is_finite() { s.font_size } else { 0.0 };
        let weight = s.weight();
        if !chars.is_empty() {
            self.text_nodes += 1;
            count_size(&mut self.sizes, size);
            self.families.insert(family_key(&s.font_family));
        }
        for run in &t.runs {
            if run.end > run.start {
                count_size(&mut self.sizes, run.style.font_size);
                self.families.insert(family_key(&run.style.font_family));
            }
        }

        if size > 0.0 && size < 11.0 && !chars.is_empty() {
            self.push("type.too-small", n.id, path, format!("Body text at {size}px is below the 11px floor"), None);
        }
        if size >= 20.0 && weight > 600 {
            self.push(
                "type.weight",
                n.id,
                path,
                format!("Heading weight {weight} is heavier than a heading needs"),
                Some(json!({"fontWeight": 600})),
            );
        }
        if s.italic() && !chars.is_empty() {
            self.push("type.italic", n.id, path, "Interface text is italic".into(), Some(json!({"italic": false})));
        }
        let tracking = s.letter_spacing.resolve(size);
        if tracking.is_finite() && tracking < -0.01 && size > 0.0 && size < 24.0 {
            self.push(
                "type.negative-tracking",
                n.id,
                path,
                format!("Small text is tracked {tracking}px tighter; use size or weight instead"),
                Some(json!({"letterSpacing": 0})),
            );
        }

        if let (Some(fg), Some(bg)) = (text_color(n, t), backdrop(self.doc, n)) {
            let ratio = fg.contrast(bg);
            if ratio.is_finite() {
                let large = size >= 24.0 || (size >= 18.0 && weight >= 700);
                let need = if large { 3.0 } else { 4.5 };
                if ratio < need {
                    self.push(
                        "color.contrast",
                        n.id,
                        path,
                        format!("Contrast {ratio:.2}:1, needs {need:.2}:1 ({} on {})", fg.to_hex(), bg.to_hex()),
                        None,
                    );
                }
            }
        }

        if contains_emoji(&t.characters) || contains_emoji(&n.name) {
            self.push("icon.emoji", n.id, path, "Emoji in interface text or a layer name; use a line icon set instead".into(), None);
        }
        if t.characters.contains('!') || t.characters.contains('！') {
            self.push("copy.exclamation", n.id, path, "Copy uses an exclamation mark".into(), Some(json!({"removeExclamations": true})));
        }
        if short_label_ends_with_period(chars) {
            self.push("copy.trailing-period", n.id, path, "Short label ends with a period".into(), Some(json!({"removeTrailingPeriod": true})));
        }
        if has_hype(chars) {
            self.push("copy.hype", n.id, path, "Copy uses a hype word (\"AI-powered\", \"supercharge\"…)".into(), None);
        }

        // A fixed-width text box whose stored height is shorter than the laid-out text clips or
        // overlaps what follows it. Truncating boxes and auto-width boxes are intentional.
        if !chars.is_empty()
            && n.width > 0.0
            && self.measured < MAX_TEXT_MEASURED
            && matches!(t.auto_resize, TextAutoResize::None | TextAutoResize::Height)
        {
            self.measured += 1;
            let needed = figcraft_text::layout_cached(t, Some(n.width), None).height;
            if needed.is_finite() && needed > n.height + 1.0 {
                self.push(
                    "layout.text-overflow",
                    n.id,
                    path,
                    format!("Text needs {needed:.0}px of height but its box is {:.0}px; it clips or overlaps", n.height),
                    None,
                );
            }
        }
    }

    fn scope_rules(&mut self, anchor: Option<NodeId>, path: &str, scope: &str) {
        let Some(node) = anchor else { return };
        if self.text_nodes >= 8 {
            if self.sizes.len() > 5 {
                let sizes: Vec<String> = self.sizes.iter().map(|v| trim_half(*v)).collect();
                self.push("type.sizes", node, path, format!("{} font sizes in this {scope}: {}", sizes.len(), sizes.join(", ")), None);
            }
            if self.families.len() > 2 {
                let families: Vec<&str> = self.families.iter().map(String::as_str).collect();
                self.push("type.families", node, path, format!("{} font families in this {scope}: {}", families.len(), families.join(", ")), None);
            }
        }
        if self.visited >= 20 && self.shadowed > 5 && self.shadowed * 5 > self.visited {
            self.push("detail.shadow-count", node, path, format!("{} of {} layers carry a shadow or blur", self.shadowed, self.visited), None);
        }
    }

    fn push(&mut self, id: &'static str, node: NodeId, path: &str, message: String, suggestion: Option<Value>) {
        let Some(r) = rule(id) else { return };
        self.findings.push(Finding { rule: r.id, severity: r.severity, node, path: path.to_string(), message, fixable: r.fixable, suggestion });
    }
}

/// The layer's name, or a stand-in when it has none; long names are cut so paths stay readable.
fn label(n: &Node) -> String {
    let name = n.name.trim();
    if name.is_empty() {
        return format!("{} {}", n.kind.type_name(), n.id.0);
    }
    let mut chars = name.chars();
    let short: String = chars.by_ref().take(60).collect();
    if chars.next().is_some() { format!("{short}…") } else { short }
}

fn on_grid(v: f64) -> bool {
    v.is_finite() && ((v / 4.0) - (v / 4.0).round()).abs() <= 0.01
}

pub(crate) fn snap(v: f64) -> f64 {
    if v.is_finite() { ((v / 4.0).round() * 4.0).max(0.0) } else { 0.0 }
}

pub(crate) fn radius_ok(r: f64, w: f64, h: f64) -> bool {
    let min = w.min(h);
    let pill = min > 0.0 && (r - min / 2.0).abs() <= 1.0;
    pill || (r >= 4.0 && on_grid(r))
}

pub(crate) fn snap_radius(r: f64) -> f64 {
    if r.is_finite() { ((r / 4.0).round() * 4.0).max(4.0) } else { 4.0 }
}

fn has_stroke(n: &Node) -> bool {
    n.stroke.weight > 0.01 || n.stroke.sides.is_some_and(|s| s.iter().any(|w| *w > 0.01))
}

/// Whether a stroke reads as a strong border rather than a hairline: its colour blended over what
/// is behind it, compared with that backdrop. With no known backdrop, the paint's alpha decides.
fn border_is_strong(doc: &Document, n: &Node) -> bool {
    let Some(p) = n.strokes.iter().rev().find(|p| p.visible) else { return false };
    let Some(c) = p.solid_color() else { return p.opacity > 0.12 };
    // An inside stroke sits on the layer's own fill; centre and outside strokes sit on what is
    // behind it. Default strokes are inside, so a border on a dark panel is judged on that panel.
    let bg = if n.stroke.align == StrokeAlign::Inside { solid_fill(n).or_else(|| backdrop(doc, n)) } else { backdrop(doc, n) };
    match bg {
        Some(bg) => blend(c, bg).contrast(bg) > 1.5,
        None => c.a > 0.12,
    }
}

fn blend(fg: Color, bg: Color) -> Color {
    let a = fg.a;
    Color::new(fg.r * a + bg.r * (1.0 - a), fg.g * a + bg.g * (1.0 - a), fg.b * a + bg.b * (1.0 - a), 1.0)
}

fn chroma(c: Color) -> f32 {
    let [r, g, b, _] = c.to_rgba8();
    let (max, min) = (r.max(g).max(b) as f32, r.min(g).min(b) as f32);
    (max - min) / 255.0
}

/// A shadow with no offset and a large blur: it lights the layer instead of giving it depth.
pub(crate) fn is_glow(e: &Effect) -> bool {
    if !e.visible {
        return false;
    }
    match &e.kind {
        EffectKind::DropShadow(s) | EffectKind::InnerShadow(s) => {
            s.offset.0.abs() <= 1.0 && s.offset.1.abs() <= 1.0 && s.radius >= 8.0 && s.color.a > 0.05
        }
        _ => false,
    }
}

/// A shadow whose colour carries a visible hue.
pub(crate) fn is_colored_shadow(s: &Shadow) -> bool {
    chroma(s.color) > 0.12 && s.color.a > 0.05
}

/// The topmost visible solid paint, if it is opaque enough to know what shows through it.
fn solid_fill(n: &Node) -> Option<Color> {
    let p = n.fills.iter().rev().find(|p| p.visible)?;
    if p.opacity < 0.95 {
        return None;
    }
    match &p.kind {
        PaintKind::Solid { color } if color.a >= 0.95 => Some(*color),
        _ => None,
    }
}

/// The colour a text layer's characters show in, when it is a single opaque solid paint.
pub(crate) fn text_color(n: &Node, t: &TextData) -> Option<Color> {
    let paints = t.style.fills.as_deref().unwrap_or(&n.fills);
    let p = paints.iter().rev().find(|p| p.visible)?;
    if p.opacity < 0.95 {
        return None;
    }
    match &p.kind {
        PaintKind::Solid { color } if color.a >= 0.95 => Some(*color),
        _ => None,
    }
}

/// What lies directly behind a layer, or why it cannot be sampled.
enum Behind {
    Known(Color),
    /// A gradient, image, translucency or partial cover: the audit does not guess.
    Unknown,
}

/// The first opaque solid behind a layer. At each ancestor level, look at the siblings painted
/// below it (z-order, `reverse_z` respected) for the topmost one that covers it, then at that
/// ancestor's own fill. The old version only walked ancestors, so white text on a dark rectangle
/// sibling was compared with the page behind both and reported as unreadable.
pub(crate) fn backdrop(doc: &Document, n: &Node) -> Option<Color> {
    let target = doc.world_bounds(n.id)?;
    let mut current = n.id;
    for _ in 0..256 {
        let node = doc.node(current)?;
        let parent_id = node.parent?;
        let parent = doc.node(parent_id)?;
        if parent.opacity < 0.99 {
            return None;
        }
        match sibling_behind(doc, parent, current, target) {
            Some(Behind::Known(c)) => return Some(c),
            Some(Behind::Unknown) => return None,
            None => {}
        }
        if let NodeKind::Page { background, .. } = &parent.kind {
            return if background.a >= 0.95 { Some(*background) } else { None };
        }
        if parent.fills.iter().any(|p| p.visible) {
            return solid_fill(parent);
        }
        current = parent_id;
    }
    None
}

/// The topmost sibling painted below `current` that covers `target`, if it has visible paint.
fn sibling_behind(doc: &Document, parent: &Node, current: NodeId, target: figcraft_geom::Rect) -> Option<Behind> {
    let index = parent.children.iter().position(|c| *c == current)?;
    let reverse = parent.layout.as_ref().is_some_and(|l| l.reverse_z);
    // Last child on top: siblings before `index` are below it. First on top: siblings after are.
    let order: Vec<usize> = if reverse { (index + 1..parent.children.len()).collect() } else { (0..index).rev().collect() };
    for i in order {
        let Some(sibling) = parent.children.get(i).and_then(|id| doc.node(*id)) else { continue };
        if let Some(behind) = paint_behind(doc, sibling, target, 1) {
            return Some(behind);
        }
    }
    None
}

/// The topmost paint a layer (and what it contains) puts under `target`: its children first (they
/// paint above its fill), then its own fill. Text and lines paint glyphs or a stroke, not a
/// background; a container without a fill is entered so a background rectangle inside a group is
/// found too. Clipped children outside the frame are left out.
fn paint_behind(doc: &Document, node: &Node, target: figcraft_geom::Rect, depth: usize) -> Option<Behind> {
    if depth > 64 || !node.visible {
        return None;
    }
    let bounds = doc.world_bounds(node.id);
    let clipped_away = node.clip && bounds.is_some_and(|b| !contains(b, target));
    if !clipped_away {
        let reverse = node.layout.as_ref().is_some_and(|l| l.reverse_z);
        let order: Vec<usize> = if reverse { (0..node.children.len()).collect() } else { (0..node.children.len()).rev().collect() };
        for i in order {
            let Some(child) = node.children.get(i).and_then(|id| doc.node(*id)) else { continue };
            if let Some(behind) = paint_behind(doc, child, target, depth + 1) {
                return Some(behind);
            }
        }
    }
    if matches!(&node.kind, NodeKind::Text(_) | NodeKind::Line) || !node.fills.iter().any(|p| p.visible) {
        return None;
    }
    let b = bounds?;
    if !overlaps(b, target) {
        return None;
    }
    if !contains(b, target) || node.opacity < 0.99 {
        return Some(Behind::Unknown);
    }
    Some(match solid_fill(node) {
        Some(c) => Behind::Known(c),
        None => Behind::Unknown,
    })
}

fn overlaps(b: figcraft_geom::Rect, target: figcraft_geom::Rect) -> bool {
    b.x0 < target.x1 && target.x0 < b.x1 && b.y0 < target.y1 && target.y0 < b.y1
}

fn contains(b: figcraft_geom::Rect, target: figcraft_geom::Rect) -> bool {
    b.x0 <= target.x0 && b.y0 <= target.y0 && b.x1 >= target.x1 && b.y1 >= target.y1
}

fn count_size(sizes: &mut BTreeSet<i64>, size: f64) {
    if size.is_finite() && size > 0.0 {
        sizes.insert((size * 2.0).round() as i64);
    }
}

fn trim_half(v: i64) -> String {
    let n = v as f64 / 2.0;
    if (n - n.round()).abs() < 0.01 { format!("{}", n.round() as i64) } else { format!("{n}") }
}

fn family_key(family: &str) -> String {
    let f = family.trim().to_ascii_lowercase();
    if f.is_empty() { "inter".into() } else { f }
}

fn contains_emoji(s: &str) -> bool {
    s.chars().any(|c| {
        let u = c as u32;
        (0x1F000..=0x1FAFF).contains(&u) || (0x2600..=0x27BF).contains(&u) || matches!(u, 0x2B50 | 0x2B55 | 0x203C | 0x2049)
    })
}

fn short_label_ends_with_period(s: &str) -> bool {
    let s = s.trim_end();
    let body = s.trim_end_matches(['.', '。']).trim_end();
    if body == s || body.is_empty() {
        return false;
    }
    !body.contains('\n') && !body.contains(". ") && body.chars().count() <= 24
}

fn has_hype(s: &str) -> bool {
    let s = s.to_ascii_lowercase();
    ["ai-powered", "supercharge", "revolutioniz", "game-chang", "unleash the"].iter().any(|w| s.contains(w))
}

fn is_default_name(name: &str) -> bool {
    let Some((word, number)) = name.trim().rsplit_once(' ') else { return false };
    const KINDS: [&str; 13] =
        ["Frame", "Rectangle", "Ellipse", "Group", "Text", "Component", "Section", "Vector", "Line", "Polygon", "Star", "Slice", "Boolean"];
    KINDS.contains(&word) && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
}
