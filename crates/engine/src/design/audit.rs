//! Walking the scope and turning what it finds into findings.
//!
//! Nothing here edits the document: `design.audit` reads pages and reports. The walk stops at an
//! instance's children (they mirror the main component, which is audited where it lives), skips
//! hidden layers and caps the number of nodes so a huge file cannot stall the app.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_color::{BlendMode, Color, Paint, PaintKind};
use figcraft_doc::{
    CollectionId, DesignPreset, Document, Effect, EffectKind, LayoutMode, ModeId, Node, NodeId, NodeKind, Shadow, Sizing, StrokeAlign,
    TextAutoResize, TextData, VariableValue,
};
use serde_json::{Map, Value, json};

use crate::cmd::lists::paint_binding_key;

use super::rules::{Severity, rule};

/// Most layers one audit looks at; past it the report says `truncated`. A call may raise it with
/// `limit` (a CI gate over a huge file), but never past [`MAX_LIMIT`].
pub(crate) const MAX_NODES: usize = 50_000;
/// The most nodes one audit may be asked to look at, so a typo cannot stall a machine for minutes.
pub(crate) const MAX_LIMIT: usize = 1_000_000;
/// Most text layers one audit lays out for the overflow check (text layout is the expensive part).
const MAX_TEXT_MEASURED: usize = 2_000;
/// Most auto layout frames one audit measures for clipped children (bounds are per-subtree).
const MAX_FRAMES_MEASURED: usize = 500;
/// Most siblings or children one backdrop lookup scans before it stops guessing; a flat frame with
/// tens of thousands of layers must not turn every text check into a full scan.
const MAX_BACKDROP_SCAN: usize = 512;

/// The design system a run checks against: the file's settings (`design.configure`) with an
/// optional preset override from the call.
#[derive(Clone)]
pub(crate) struct Settings {
    pub(crate) preset: DesignPreset,
    pub(crate) grid: f64,
    /// Allowed corner radii; empty: any multiple of the grid from the grid up, or a pill.
    pub(crate) radii: Vec<f64>,
    /// Brand accents as 8-bit RGBA.
    pub(crate) brand: Vec<[u8; 4]>,
    pub(crate) disabled: BTreeSet<String>,
    /// Variable modes the run resolves colours in (empty: the document's own modes).
    pub(crate) modes: BTreeMap<CollectionId, ModeId>,
    /// The same modes by collection and mode name, for the message and the report.
    pub(crate) mode_labels: BTreeMap<String, String>,
}

impl Settings {
    pub(crate) fn new(doc: &Document, preset: Option<DesignPreset>) -> Self {
        let config = &doc.design_review;
        Self {
            preset: preset.unwrap_or(config.preset),
            grid: config.grid(),
            radii: config.radii.iter().copied().filter(|r| r.is_finite() && (0.0..=999.0).contains(r)).collect(),
            brand: config.brand.iter().map(|c| c.to_rgba8()).collect(),
            disabled: config.disabled.iter().cloned().collect(),
            modes: BTreeMap::new(),
            mode_labels: BTreeMap::new(),
        }
    }

    /// Resolve colours in `modes` (collection and mode ids, already checked against the document).
    pub(crate) fn in_modes(mut self, doc: &Document, modes: BTreeMap<CollectionId, ModeId>) -> Self {
        for (collection, mode) in &modes {
            let Some(c) = doc.variables.collection_by_id(collection) else { continue };
            let Some(m) = c.mode_by_id(mode) else { continue };
            self.mode_labels.insert(c.name.clone(), m.name.clone());
        }
        self.modes = modes;
        self
    }

    /// The mode names for a finding's message, when any are overridden.
    fn mode_note(&self) -> String {
        self.mode_labels.iter().map(|(c, m)| format!("{c}: {m}")).collect::<Vec<_>>().join(", ")
    }

    /// Whether a run using these settings reports `id`.
    pub(crate) fn enabled(&self, id: &str) -> bool {
        !self.disabled.contains(id) && super::rules::enabled(id, self.preset)
    }

    fn on_grid(&self, v: f64) -> bool {
        v.is_finite() && ((v / self.grid) - (v / self.grid).round()).abs() <= 0.01
    }

    pub(crate) fn snap(&self, v: f64) -> f64 {
        if v.is_finite() { ((v / self.grid).round() * self.grid).max(0.0) } else { 0.0 }
    }

    pub(crate) fn radius_ok(&self, r: f64, w: f64, h: f64) -> bool {
        if !r.is_finite() || r <= 0.01 {
            return true;
        }
        let min = w.min(h);
        let pill = min > 0.0 && (r - min / 2.0).abs() <= 1.0;
        if pill {
            return true;
        }
        if !self.radii.is_empty() {
            return self.radii.iter().any(|a| (r - a).abs() <= 0.5);
        }
        r >= self.grid && self.on_grid(r)
    }

    pub(crate) fn snap_radius(&self, r: f64) -> f64 {
        if !r.is_finite() {
            return self.grid;
        }
        if !self.radii.is_empty() {
            let mut best = self.grid;
            let mut distance = f64::INFINITY;
            for a in &self.radii {
                let d = (r - a).abs();
                if d < distance {
                    best = *a;
                    distance = d;
                }
            }
            return best;
        }
        ((r / self.grid).round() * self.grid).max(self.grid)
    }

    /// Whether `c` is one of the file's brand accents (within a few 8-bit steps per channel).
    pub(crate) fn is_brand(&self, c: Color) -> bool {
        let rgba = c.to_rgba8();
        self.brand.iter().any(|b| (0..4).all(|i| b[i].abs_diff(rgba[i]) <= 12))
    }

    /// The settings a report or the window shows.
    pub(crate) fn json(&self) -> Value {
        let mut v = json!({"preset": self.preset.id(), "grid": self.grid});
        if !self.mode_labels.is_empty()
            && let Some(o) = v.as_object_mut()
        {
            o.insert("modes".into(), json!(self.mode_labels));
        }
        v
    }
}

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
    /// The values behind the message, for a localized line in the window and for agents.
    pub(crate) details: Option<Value>,
    /// Why a finding is not fixable: `"bound"` (owned by a variable or style), `"modes"` (read in
    /// the chosen variable modes), `None` (needs a decision). Only for `fixable: false`.
    pub(crate) reason: Option<&'static str>,
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
        if let (Some(d), Some(o)) = (&self.details, v.as_object_mut()) {
            o.insert("details".into(), d.clone());
        }
        if !self.fixable
            && let Some(o) = v.as_object_mut()
        {
            o.insert("reason".into(), json!(self.reason.unwrap_or("decision")));
        }
        v
    }
}

/// What one walk found.
pub(crate) struct Report {
    pub(crate) findings: Vec<Finding>,
    pub(crate) nodes: usize,
    pub(crate) truncated: bool,
    /// The node cap this report was collected under (`limit`).
    pub(crate) limit: usize,
}

/// A colour token's default-mode value, for the hardcoded-value check.
struct TokenColor {
    name: String,
    rgba: [u8; 4],
}

/// A number token's default-mode value.
struct TokenNumber {
    name: String,
    value: f64,
}

/// The file's variables in their collections' default modes. Layers with explicit modes are left
/// out of the token checks (the default value may not be what they show).
struct Tokens {
    colors: Vec<TokenColor>,
    numbers: Vec<TokenNumber>,
}

impl Tokens {
    fn new(doc: &Document) -> Self {
        if doc.variables.variables.is_empty() {
            return Self { colors: Vec::new(), numbers: Vec::new() };
        }
        let modes: BTreeMap<String, String> =
            doc.variables.collections.iter().filter_map(|c| c.default_mode().map(|m| (c.id.clone(), m.clone()))).collect();
        let mut colors = Vec::new();
        let mut numbers = Vec::new();
        for var in &doc.variables.variables {
            let name = if var.name.is_empty() { var.id.clone() } else { var.name.clone() };
            match doc.variables.resolve(&var.id, &modes) {
                Some(VariableValue::Color(c)) if c.a > 0.01 => colors.push(TokenColor { name, rgba: c.to_rgba8() }),
                Some(VariableValue::Number(n)) if n.is_finite() && n > 0.01 => numbers.push(TokenNumber { name, value: n }),
                _ => {}
            }
        }
        colors.sort_by_key(|c| c.rgba);
        colors.dedup_by(|a, b| a.rgba == b.rgba);
        numbers.sort_by(|a, b| a.value.total_cmp(&b.value));
        numbers.dedup_by(|a, b| (a.value - b.value).abs() <= 1e-6);
        Self { colors, numbers }
    }

    fn is_empty(&self) -> bool {
        self.colors.is_empty() && self.numbers.is_empty()
    }

    fn number(&self, value: f64) -> Option<&TokenNumber> {
        self.numbers.iter().find(|t| (t.value - value).abs() <= 1e-6)
    }

    fn color(&self, color: Color) -> Option<&TokenColor> {
        let rgba = color.to_rgba8();
        self.colors.iter().find(|t| t.rgba == rgba)
    }
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
    /// Auto layout frames measured for clipped children.
    measured_frames: usize,
    tokens: Tokens,
    /// The design system this run checks against.
    settings: Settings,
    /// Distinct accent colours, with the first layer that paints each (for `color.accents`).
    accents: BTreeMap<[u8; 4], String>,
    truncated: bool,
    /// The node cap this run stops at.
    max_nodes: usize,
}

/// Walk `roots` and collect every finding. `design.audit` reports them; `design.fix` acts on the
/// ones marked fixable.
pub(crate) fn collect(
    doc: &Document,
    roots: &[NodeId],
    anchor: Option<NodeId>,
    anchor_path: &str,
    scope: &str,
    settings: &Settings,
    max_nodes: usize,
) -> Report {
    let mut ctx = Ctx {
        doc,
        findings: Vec::new(),
        visited: 0,
        text_nodes: 0,
        sizes: BTreeSet::new(),
        families: BTreeSet::new(),
        shadowed: 0,
        measured: 0,
        measured_frames: 0,
        tokens: Tokens::new(doc),
        settings: settings.clone(),
        accents: BTreeMap::new(),
        truncated: false,
        max_nodes,
    };
    for root in roots {
        ctx.walk(*root);
    }
    ctx.scope_rules(anchor, anchor_path, scope);
    Report { findings: ctx.findings, nodes: ctx.visited, truncated: ctx.truncated, limit: max_nodes }
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
    preset: Option<DesignPreset>,
    modes: BTreeMap<CollectionId, ModeId>,
    max_nodes: usize,
) -> Value {
    let settings = Settings::new(doc, preset).in_modes(doc, modes);
    let report = collect(doc, roots, anchor, anchor_path, scope, &settings, max_nodes);
    let shown: Vec<&Finding> = report
        .findings
        .iter()
        .filter(|f| f.severity >= min && settings.enabled(f.rule))
        .filter(|f| filter.is_none_or(|set| set.contains(f.rule)))
        .collect();
    let mut counts = [0usize; 3];
    for f in &shown {
        counts[f.severity.index()] += 1;
    }
    let score = (100.0 - shown.iter().map(|f| f.severity.penalty()).sum::<f64>()).clamp(0.0, 100.0).round();
    let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &shown {
        *by_rule.entry(f.rule).or_default() += 1;
    }
    let findings: Vec<Value> = shown.into_iter().map(Finding::json).collect();
    let mut out = settings.json();
    if let Some(o) = out.as_object_mut() {
        o.insert("scope".into(), json!(scope));
        o.insert("score".into(), json!(score));
        o.insert("counts".into(), json!({"high": counts[2], "medium": counts[1], "low": counts[0]}));
        o.insert("byRule".into(), json!(by_rule));
        o.insert("nodes".into(), json!(report.nodes));
        o.insert("limit".into(), json!(report.limit));
        o.insert("findings".into(), json!(findings));
        o.insert("truncated".into(), json!(report.truncated));
    }
    out
}

impl Ctx<'_> {
    fn walk(&mut self, root: NodeId) {
        let mut stack = vec![(root, String::new())];
        while let Some((id, parent)) = stack.pop() {
            if self.visited >= self.max_nodes {
                self.truncated = true;
                return;
            }
            let Some(n) = self.doc.node(id) else { continue };
            if !n.visible || n.locked || n.opacity <= 0.001 {
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
        self.check_layout(n, path);
        self.check_clipping(n, path);
        self.note_accents(n);
        self.check_tokens(n, path);
    }

    fn check_naming(&mut self, n: &Node, path: &str) {
        if is_default_name(&n.name) {
            self.push_details(
                "naming.auto",
                n.id,
                path,
                format!("Layer still has its default name \"{}\"", n.name.trim()),
                None,
                Some(json!({"name": n.name.trim()})),
            );
        }
    }

    /// A frame or component holding several layers without auto layout: in the Strict preset.
    fn check_layout(&mut self, n: &Node, path: &str) {
        if n.layout.is_some() || n.children.len() < 3 {
            return;
        }
        if matches!(n.kind, NodeKind::Frame | NodeKind::Component { .. } | NodeKind::ComponentSet { .. }) {
            let count = n.children.len();
            self.push_details(
                "layout.no-auto-layout",
                n.id,
                path,
                format!("{count} children with no auto layout"),
                None,
                Some(json!({"children": count})),
            );
        }
    }

    /// A fixed-size auto layout frame that clips children which overflow it: the rows exist but
    /// are invisible — the usual headless surprise. Hug frames grow, so they cannot clip.
    fn check_clipping(&mut self, n: &Node, path: &str) {
        if self.measured_frames >= MAX_FRAMES_MEASURED || !n.clip || n.children.is_empty() || n.layout.is_none() {
            return;
        }
        // An instance mirrors its main; the main is audited where it lives.
        if matches!(n.kind, NodeKind::Instance { .. }) {
            return;
        }
        let fixed_h = n.child_layout.sizing_h != Sizing::Hug;
        let fixed_v = n.child_layout.sizing_v != Sizing::Hug;
        if !(fixed_h || fixed_v) {
            return;
        }
        let Some(frame) = self.doc.world_bounds(n.id) else { return };
        let mut u: Option<(f64, f64, f64, f64)> = None;
        for child in &n.children {
            let Some(c) = self.doc.node(*child) else { continue };
            if !c.visible {
                continue;
            }
            if let Some(b) = self.doc.world_bounds(*child) {
                u = Some(match u {
                    None => (b.x0, b.y0, b.x1, b.y1),
                    Some((x0, y0, x1, y1)) => (x0.min(b.x0), y0.min(b.y0), x1.max(b.x1), y1.max(b.y1)),
                });
            }
        }
        self.measured_frames += 1;
        let Some((ux0, uy0, ux1, uy1)) = u else { return };
        let over_h = fixed_h && (ux1 > frame.x1 + 1.0 || ux0 < frame.x0 - 1.0);
        let over_v = fixed_v && (uy1 > frame.y1 + 1.0 || uy0 < frame.y0 - 1.0);
        if !(over_h || over_v) {
            return;
        }
        let mut suggestion = Map::new();
        if over_h {
            suggestion.insert("sizingH".into(), json!("hug"));
        }
        if over_v {
            suggestion.insert("sizingV".into(), json!("hug"));
        }
        self.push_details(
            "layout.clipped",
            n.id,
            path,
            format!(
                "Fixed-size frame clips children that reach {:.0}×{:.0} inside its {:.0}×{:.0}; they are invisible",
                ux1 - ux0,
                uy1 - uy0,
                frame.x1 - frame.x0,
                frame.y1 - frame.y0
            ),
            Some(Value::Object(suggestion)),
            Some(json!({"width": ux1 - ux0, "height": uy1 - uy0, "frameWidth": frame.x1 - frame.x0, "frameHeight": frame.y1 - frame.y0})),
        );
    }

    /// Collect the accent colours the scope paints (brand accents and neutrals excluded), for
    /// `color.accents`.
    fn note_accents(&mut self, n: &Node) {
        let paints = match &n.kind {
            NodeKind::Text(t) => t.style.fills.as_deref().unwrap_or(&n.fills),
            _ => &n.fills,
        };
        for p in paints.iter().chain(n.strokes.iter()) {
            if !p.visible || p.opacity < 0.5 {
                continue;
            }
            let PaintKind::Solid { color } = &p.kind else { continue };
            if self.settings.is_brand(*color) {
                continue;
            }
            if let Some(rgba) = accent_rgba(*color)
                && self.accents.len() < 64
            {
                self.accents.entry(rgba).or_insert_with(|| n.name.trim().to_string());
            }
        }
    }

    fn check_spacing(&mut self, n: &Node, path: &str) {
        let Some(al) = &n.layout else { return };
        let mut off = Vec::new();
        let mut suggestion = Map::new();
        if al.mode != LayoutMode::Grid {
            if !self.settings.on_grid(al.gap) {
                off.push(format!("gap {}", al.gap));
                suggestion.insert("gap".into(), json!(self.settings.snap(al.gap)));
            }
            if al.wrap && !self.settings.on_grid(al.cross_gap) {
                off.push(format!("cross gap {}", al.cross_gap));
                suggestion.insert("crossGap".into(), json!(self.settings.snap(al.cross_gap)));
            }
        }
        let sides = ["top", "right", "bottom", "left"];
        let mut padding_off = false;
        for (i, side) in sides.iter().enumerate() {
            if let Some(v) = al.padding.get(i).copied().filter(|v| !self.settings.on_grid(*v)) {
                off.push(format!("padding {side} {v}"));
                padding_off = true;
            }
        }
        if padding_off {
            suggestion.insert("padding".into(), json!(al.padding.map(|v| self.settings.snap(v))));
        }
        if !off.is_empty() {
            let suggestion = if suggestion.is_empty() { None } else { Some(Value::Object(suggestion)) };
            let mut details = Map::new();
            details.insert("grid".into(), json!(self.settings.grid));
            if !self.settings.on_grid(al.gap) {
                details.insert("gap".into(), json!(al.gap));
            }
            if al.wrap && !self.settings.on_grid(al.cross_gap) {
                details.insert("crossGap".into(), json!(al.cross_gap));
            }
            if padding_off {
                details.insert("padding".into(), json!(al.padding));
            }
            self.push_details(
                "spacing.off-grid",
                n.id,
                path,
                format!("Off the {}px grid: {}", self.settings.grid, off.join(", ")),
                suggestion,
                Some(Value::Object(details)),
            );
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
                self.not_fixable(" (bound to a variable)", "bound");
            }
        }
    }

    fn check_radius(&mut self, n: &Node, path: &str) {
        let mut bad: Vec<f64> = n.corners.radii.iter().copied().filter(|r| *r > 0.01 && !self.settings.radius_ok(*r, n.width, n.height)).collect();
        if bad.is_empty() {
            return;
        }
        bad.sort_by(f64::total_cmp);
        bad.dedup_by(|a, b| (*a - *b).abs() <= 0.01);
        let shown: Vec<String> = bad.iter().map(|r| r.to_string()).collect();
        let suggested = self.settings.snap_radius(bad.first().copied().unwrap_or(0.0));
        let suggestion = json!({"cornerRadius": suggested});
        let scale = if self.settings.radii.is_empty() {
            format!("any multiple of {}, or a pill", self.settings.grid)
        } else {
            let list: Vec<String> = self.settings.radii.iter().map(|r| r.to_string()).collect();
            format!("{}, or a pill", list.join(", "))
        };
        self.push_details(
            "radius.off-scale",
            n.id,
            path,
            format!("Corner radius off the scale: {} (use {scale})", shown.join(", ")),
            Some(suggestion),
            Some(json!({"radii": bad, "suggested": suggested})),
        );
        if ["cornerRadius", "topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"].iter().any(|k| n.bound.contains_key(*k)) {
            self.not_fixable(" (bound to a variable)", "bound");
        }
    }

    fn check_paints(&mut self, n: &Node, path: &str) {
        let is_text = matches!(&n.kind, NodeKind::Text(_));
        let fills: &[Paint] = match &n.kind {
            NodeKind::Text(t) => t.style.fills.as_deref().unwrap_or(&n.fills),
            _ => &n.fills,
        };
        let fills_gradient = fills.iter().any(|p| p.visible && matches!(&p.kind, PaintKind::Gradient(_)));
        let strokes_gradient = n.strokes.iter().any(|p| p.visible && matches!(&p.kind, PaintKind::Gradient(_)));
        let run_gradient = match &n.kind {
            NodeKind::Text(t) => t
                .runs
                .iter()
                .any(|r| r.style.fills.as_ref().is_some_and(|paints| paints.iter().any(|p| p.visible && matches!(&p.kind, PaintKind::Gradient(_))))),
            _ => false,
        };
        if fills_gradient || strokes_gradient || run_gradient {
            let id = if is_text { "color.gradient-text" } else { "color.gradient" };
            self.push(id, n.id, path, "Painted with a gradient (a hue-to-hue wash is the usual AI tell)".into(), None);
        }
        // A hairline border is for container-like shapes; an icon's stroke is its whole drawing,
        // so full opacity is the intent there, not a stray border.
        let container_like = matches!(&n.kind, NodeKind::Frame | NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Component { .. });
        if container_like && has_stroke(n) && border_is_strong(self.doc, n) {
            self.push(
                "color.border-opacity",
                n.id,
                path,
                "Border is more opaque than a hairline (6–10% is enough)".into(),
                Some(json!({"strokeOpacity": 0.08})),
            );
            if n.styles.stroke.is_some() || n.bound.keys().any(|k| k.starts_with("strokes/")) {
                self.not_fixable(" (bound to a style or variable)", "bound");
            }
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
            if n.styles.effect.is_some() || n.bound.keys().any(|k| k.starts_with("effects/")) {
                self.not_fixable(" (bound to a style or variable)", "bound");
            }
        }
        if colored {
            self.push(
                "color.colored-shadow",
                n.id,
                path,
                "Shadow carries a visible hue; shadows read as depth only when neutral".into(),
                Some(json!({"neutralShadow": true})),
            );
            if n.styles.effect.is_some() || n.bound.keys().any(|k| k.starts_with("effects/")) {
                self.not_fixable(" (bound to a style or variable)", "bound");
            }
        }
    }

    fn check_text(&mut self, n: &Node, t: &TextData, path: &str) {
        let chars = t.characters.trim();
        let s = &t.style;
        let size = if s.font_size.is_finite() { s.font_size } else { 0.0 };
        let weight = s.weight();
        // A text style (whole layer or a run) or a variable owns the type; changing it here would
        // drift from the style the user chose.
        let styled = n.styles.text.is_some()
            || s.style_id.is_some()
            || t.runs.iter().any(|r| r.style.style_id.is_some())
            || ["fontSize", "fontStyle", "fontWeight", "letterSpacing"].iter().any(|k| n.bound.contains_key(*k));
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
            self.push_details(
                "type.too-small",
                n.id,
                path,
                format!("Body text at {size}px is below the 11px floor"),
                None,
                Some(json!({"size": size})),
            );
        }
        if size >= 20.0 && weight > 600 {
            self.push_details(
                "type.weight",
                n.id,
                path,
                format!("Heading weight {weight} is heavier than a heading needs"),
                Some(json!({"fontWeight": 600})),
                Some(json!({"weight": weight})),
            );
            if styled {
                self.not_fixable(" (bound to a style or variable)", "bound");
            }
        }
        if s.italic() && !chars.is_empty() {
            self.push("type.italic", n.id, path, "Interface text is italic".into(), Some(json!({"italic": false})));
            if styled {
                self.not_fixable(" (bound to a style or variable)", "bound");
            }
        }
        let tracking = s.letter_spacing.resolve(size);
        if tracking.is_finite() && tracking < -0.01 && size > 0.0 && size < 24.0 {
            self.push_details(
                "type.negative-tracking",
                n.id,
                path,
                format!("Small text is tracked {tracking}px tighter; use size or weight instead"),
                Some(json!({"letterSpacing": 0})),
                Some(json!({"tracking": tracking})),
            );
            if styled {
                self.not_fixable(" (bound to a style or variable)", "bound");
            }
        }

        // A run with its own fill makes the text more than one colour; the base fill is not the
        // whole story, so the contrast check skips rather than judging the wrong pair.
        let mixed_fills = t.runs.iter().any(|r| r.style.fills.is_some());
        self.check_contrast(n, t, path, size, weight, mixed_fills);

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
                self.push_details(
                    "layout.text-overflow",
                    n.id,
                    path,
                    format!("Text needs {needed:.0}px of height but its box is {:.0}px; it clips or overlaps", n.height),
                    None,
                    Some(json!({"needed": needed, "height": n.height})),
                );
            }
        }
    }

    /// WCAG contrast for single-colour text, in the run's variable modes when any were given.
    fn check_contrast(&mut self, n: &Node, t: &TextData, path: &str, size: f64, weight: u16, mixed_fills: bool) {
        if mixed_fills {
            return;
        }
        let (fg, bg) = if self.settings.modes.is_empty() {
            (text_color(n, t), backdrop(self.doc, n))
        } else {
            (text_color_in(self.doc, n, t, &self.settings.modes), backdrop_in(self.doc, n, Some(&self.settings.modes)))
        };
        let (Some(fg), Some(bg)) = (fg, bg) else { return };
        let ratio = fg.contrast(bg);
        if !ratio.is_finite() {
            return;
        }
        let large = size >= 24.0 || (size >= 18.0 && weight >= 700);
        let need = if large { 3.0 } else { 4.5 };
        if ratio >= need {
            return;
        }
        let mode_note = if self.settings.mode_labels.is_empty() { String::new() } else { format!(" in {}", self.settings.mode_note()) };
        let mut details = json!({"ratio": ratio, "needed": need, "foreground": fg.to_hex(), "background": bg.to_hex()});
        if !self.settings.mode_labels.is_empty()
            && let Some(o) = details.as_object_mut()
        {
            o.insert("modes".into(), json!(self.settings.mode_labels));
        }
        self.push_details(
            "color.contrast",
            n.id,
            path,
            format!("Contrast {ratio:.2}:1, needs {need:.2}:1 ({} on {}){mode_note}", fg.to_hex(), bg.to_hex()),
            None,
            Some(details),
        );
        if !self.settings.modes.is_empty() {
            self.not_fixable(" (read in the chosen variable modes)", "modes");
        } else if n.styles.fill.is_some() || n.bound.keys().any(|k| k.starts_with("fills/")) {
            self.not_fixable(" (bound to a style or variable)", "bound");
        }
    }

    fn check_tokens(&mut self, n: &Node, path: &str) {
        if self.tokens.is_empty() || !n.var_modes.is_empty() {
            return;
        }
        // Colours: a solid paint that is exactly a colour variable's default value, unbound and
        // not styled, is a hardcoded token.
        let mut hits: Vec<(&str, String, String)> = Vec::new();
        let is_text = matches!(&n.kind, NodeKind::Text(_));
        let fills: &[Paint] = match &n.kind {
            NodeKind::Text(t) => t.style.fills.as_deref().unwrap_or(&n.fills),
            _ => &n.fills,
        };
        if n.styles.fill.is_none() {
            for (i, p) in fills.iter().enumerate() {
                if !p.visible || paint_binding_key(n, true, i).is_some() {
                    continue;
                }
                let Some(c) = p.solid_color() else { continue };
                if let Some(tok) = self.tokens.color(c) {
                    hits.push((if is_text { "text" } else { "fill" }, c.to_hex(), tok.name.clone()));
                }
            }
        }
        if n.styles.stroke.is_none() {
            for (i, p) in n.strokes.iter().enumerate() {
                if !p.visible || paint_binding_key(n, false, i).is_some() {
                    continue;
                }
                let Some(c) = p.solid_color() else { continue };
                if let Some(tok) = self.tokens.color(c) {
                    hits.push(("stroke", c.to_hex(), tok.name.clone()));
                }
            }
        }
        if !hits.is_empty() {
            hits.sort();
            hits.dedup();
            let shown: Vec<String> = hits.iter().map(|(kind, hex, token)| format!("{kind} #{hex} = \"{token}\"")).collect();
            let details: Vec<Value> =
                hits.iter().map(|(kind, hex, token)| json!({"kind": kind, "color": format!("#{hex}"), "token": token})).collect();
            self.push_details(
                "token.hardcoded-color",
                n.id,
                path,
                format!("Matches a variable but is not bound: {}", shown.join(", ")),
                None,
                Some(json!({"hits": details})),
            );
        }

        // Numbers: spacing, radii, a text size and opacity.
        let mut hits: Vec<(String, f64, String)> = Vec::new();
        {
            let mut check = |value: f64, field: &str| {
                if value.is_finite()
                    && value > 0.01
                    && let Some(tok) = self.tokens.number(value)
                {
                    hits.push((field.to_string(), value, tok.name.clone()));
                }
            };
            if let Some(al) = &n.layout {
                if al.mode != LayoutMode::Grid && !n.bound.contains_key("gap") {
                    check(al.gap, "gap");
                }
                if al.wrap && !n.bound.contains_key("crossGap") {
                    check(al.cross_gap, "crossGap");
                }
                let keys = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"];
                for (i, key) in keys.iter().enumerate() {
                    if !n.bound.contains_key(*key)
                        && let Some(v) = al.padding.get(i).copied()
                    {
                        check(v, key);
                    }
                }
            }
            if !["cornerRadius", "topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"].iter().any(|k| n.bound.contains_key(*k))
            {
                for r in n.corners.radii {
                    check(r, "cornerRadius");
                }
            }
            if let NodeKind::Text(t) = &n.kind {
                let styled = n.styles.text.is_some() || t.style.style_id.is_some() || t.runs.iter().any(|r| r.style.style_id.is_some());
                if !styled && !n.bound.contains_key("fontSize") {
                    check(t.style.font_size, "fontSize");
                }
            }
            if n.opacity < 0.99 && !n.bound.contains_key("opacity") {
                check(f64::from(n.opacity), "opacity");
            }
        }
        if !hits.is_empty() {
            hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
            hits.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() <= 1e-6);
            let shown: Vec<String> = hits.iter().map(|(field, value, token)| format!("{field} {value} = \"{token}\"")).collect();
            let details: Vec<Value> = hits.iter().map(|(field, value, token)| json!({"field": field, "value": value, "token": token})).collect();
            self.push_details(
                "token.hardcoded-number",
                n.id,
                path,
                format!("Matches a variable but is not bound: {}", shown.join(", ")),
                None,
                Some(json!({"hits": details})),
            );
        }
    }

    fn scope_rules(&mut self, anchor: Option<NodeId>, path: &str, scope: &str) {
        let Some(node) = anchor else { return };
        if self.text_nodes >= 8 {
            if self.sizes.len() > 5 {
                let sizes: Vec<String> = self.sizes.iter().map(|v| trim_half(*v)).collect();
                self.push_details(
                    "type.sizes",
                    node,
                    path,
                    format!("{} font sizes in this {scope}: {}", sizes.len(), sizes.join(", ")),
                    None,
                    Some(json!({"sizes": sizes, "count": sizes.len()})),
                );
            }
            if self.families.len() > 2 {
                let families: Vec<&str> = self.families.iter().map(String::as_str).collect();
                self.push_details(
                    "type.families",
                    node,
                    path,
                    format!("{} font families in this {scope}: {}", families.len(), families.join(", ")),
                    None,
                    Some(json!({"families": families, "count": families.len()})),
                );
            }
        }
        if self.visited >= 20 && self.shadowed > 5 && self.shadowed * 5 > self.visited {
            self.push_details(
                "detail.shadow-count",
                node,
                path,
                format!("{} of {} layers carry a shadow or blur", self.shadowed, self.visited),
                None,
                Some(json!({"shadowed": self.shadowed, "nodes": self.visited})),
            );
        }
        if self.accents.len() > 2 {
            let colors: Vec<String> = self.accents.keys().take(6).map(|c| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])).collect();
            self.push_details(
                "color.accents",
                node,
                path,
                format!("{} accent colours in this {scope}: {} (keep two)", self.accents.len(), colors.join(", ")),
                None,
                Some(json!({"count": self.accents.len(), "colors": colors})),
            );
        }
    }

    fn push(&mut self, id: &'static str, node: NodeId, path: &str, message: String, suggestion: Option<Value>) {
        self.push_details(id, node, path, message, suggestion, None);
    }

    /// A finding whose message has values, with those values in `details` too.
    fn push_details(&mut self, id: &'static str, node: NodeId, path: &str, message: String, suggestion: Option<Value>, details: Option<Value>) {
        let Some(r) = rule(id) else { return };
        self.findings.push(Finding {
            rule: r.id,
            severity: r.severity,
            node,
            path: path.to_string(),
            message,
            fixable: r.fixable,
            suggestion,
            details,
            reason: None,
        });
    }

    /// Mark the finding just pushed as not auto-fixable, with the reason in its message and as a
    /// machine code for the window (`"bound"`: owned by a variable or style; `"modes"`: read in
    /// the chosen variable modes).
    fn not_fixable(&mut self, note: &str, reason: &'static str) {
        if let Some(f) = self.findings.last_mut() {
            f.fixable = false;
            f.message.push_str(note);
            f.reason = Some(reason);
        }
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

/// A colour that reads as an accent: enough chroma to be a hue rather than a neutral, and neither
/// near-black nor near-white.
fn accent_rgba(c: Color) -> Option<[u8; 4]> {
    if !c.a.is_finite() || c.a < 0.5 {
        return None;
    }
    let max = c.r.max(c.g).max(c.b);
    let min = c.r.min(c.g).min(c.b);
    (max - min >= 0.18 && max >= 0.2 && min <= 0.88).then(|| c.to_rgba8())
}

fn has_stroke(n: &Node) -> bool {
    n.stroke.weight > 0.01 || n.stroke.sides.is_some_and(|s| s.iter().any(|w| *w > 0.01))
}

/// Whether a stroke reads as a strong border rather than a hairline: its colour blended over what
/// is behind it, compared with that backdrop. With no known backdrop, the paint's alpha decides.
fn border_is_strong(doc: &Document, n: &Node) -> bool {
    let Some(p) = n.strokes.iter().rev().find(|p| p.visible) else { return false };
    if p.blend != BlendMode::Normal {
        return false;
    }
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
    // A blend mode changes what the colour looks like over what is behind it; do not guess.
    if !matches!(n.blend, BlendMode::Normal | BlendMode::PassThrough) || p.blend != BlendMode::Normal || p.opacity < 0.95 {
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
    if p.blend != BlendMode::Normal {
        return None;
    }
    if p.opacity < 0.95 {
        return None;
    }
    match &p.kind {
        PaintKind::Solid { color } if color.a >= 0.95 => Some(*color),
        _ => None,
    }
}

/// `base`, resolved in the layer's variable modes (with `overrides` on top) when the paint's colour
/// is bound to a colour variable.
fn bound_color(doc: &Document, n: &Node, fills: bool, index: usize, base: Color, overrides: &BTreeMap<CollectionId, ModeId>) -> Color {
    let Some(key) = paint_binding_key(n, fills, index) else { return base };
    let Some(id) = n.bound.get(key) else { return base };
    let mut modes = doc.modes_for(n.id);
    modes.extend(overrides.iter().map(|(c, m)| (c.clone(), m.clone())));
    match doc.variables.resolve(id, &modes) {
        Some(VariableValue::Color(c)) => c,
        _ => base,
    }
}

/// [`solid_fill`], with bound colours resolved in `overrides` (the layer's own modes first).
fn solid_fill_in(doc: &Document, n: &Node, overrides: &BTreeMap<CollectionId, ModeId>) -> Option<Color> {
    let index = n.fills.iter().rposition(|p| p.visible)?;
    let p = n.fills.get(index)?;
    if !matches!(n.blend, BlendMode::Normal | BlendMode::PassThrough) || p.blend != BlendMode::Normal || p.opacity < 0.95 {
        return None;
    }
    let PaintKind::Solid { color } = &p.kind else { return None };
    if color.a < 0.95 {
        return None;
    }
    let color = bound_color(doc, n, true, index, *color, overrides);
    (color.a >= 0.95).then_some(color)
}

/// [`text_color`], with bound colours resolved in `overrides`.
fn text_color_in(doc: &Document, n: &Node, t: &TextData, overrides: &BTreeMap<CollectionId, ModeId>) -> Option<Color> {
    let paints = t.style.fills.as_deref().unwrap_or(&n.fills);
    let (index, p) = paints.iter().enumerate().rev().find(|(_, p)| p.visible)?;
    if p.blend != BlendMode::Normal || p.opacity < 0.95 {
        return None;
    }
    let PaintKind::Solid { color } = &p.kind else { return None };
    if color.a < 0.95 {
        return None;
    }
    let color = bound_color(doc, n, true, index, *color, overrides);
    (color.a >= 0.95).then_some(color)
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
    backdrop_in(doc, n, None)
}

/// [`backdrop`], resolving bound colours in `overrides` when given.
fn backdrop_in(doc: &Document, n: &Node, overrides: Option<&BTreeMap<CollectionId, ModeId>>) -> Option<Color> {
    let target = doc.world_bounds(n.id)?;
    let mut current = n.id;
    for _ in 0..256 {
        let node = doc.node(current)?;
        let parent_id = node.parent?;
        let parent = doc.node(parent_id)?;
        if parent.opacity < 0.99 {
            return None;
        }
        match sibling_behind(doc, parent, current, target, overrides) {
            Some(Behind::Known(c)) => return Some(c),
            Some(Behind::Unknown) => return None,
            None => {}
        }
        if let NodeKind::Page { background, .. } = &parent.kind {
            return if background.a >= 0.95 { Some(*background) } else { None };
        }
        if parent.fills.iter().any(|p| p.visible) {
            return match overrides {
                Some(modes) => solid_fill_in(doc, parent, modes),
                None => solid_fill(parent),
            };
        }
        current = parent_id;
    }
    None
}

/// The topmost sibling painted below `current` that covers `target`, if it has visible paint.
fn sibling_behind(
    doc: &Document,
    parent: &Node,
    current: NodeId,
    target: figcraft_geom::Rect,
    overrides: Option<&BTreeMap<CollectionId, ModeId>>,
) -> Option<Behind> {
    let index = parent.children.iter().position(|c| *c == current)?;
    let reverse = parent.layout.as_ref().is_some_and(|l| l.reverse_z);
    // Last child on top: siblings before `index` are below it. First on top: siblings after are.
    let total = if reverse { parent.children.len().saturating_sub(index + 1) } else { index };
    let order: Box<dyn Iterator<Item = usize>> = if reverse { Box::new(index + 1..parent.children.len()) } else { Box::new((0..index).rev()) };
    for i in order.take(MAX_BACKDROP_SCAN) {
        let Some(sibling) = parent.children.get(i).and_then(|id| doc.node(*id)) else { continue };
        if let Some(behind) = paint_behind(doc, sibling, target, 1, overrides) {
            return Some(behind);
        }
    }
    // Past the cap the topmost paint is unknown, not the page behind the container.
    (total > MAX_BACKDROP_SCAN).then_some(Behind::Unknown)
}

/// The topmost paint a layer (and what it contains) puts under `target`: its children first (they
/// paint above its fill), then its own fill. Text and lines paint glyphs or a stroke, not a
/// background; a container without a fill is entered so a background rectangle inside a group is
/// found too. Clipped children outside the frame are left out.
fn paint_behind(
    doc: &Document,
    node: &Node,
    target: figcraft_geom::Rect,
    depth: usize,
    overrides: Option<&BTreeMap<CollectionId, ModeId>>,
) -> Option<Behind> {
    if depth > 64 || !node.visible {
        return None;
    }
    let bounds = doc.world_bounds(node.id);
    let clipped_away = node.clip && bounds.is_some_and(|b| !contains(b, target));
    if !clipped_away {
        let reverse = node.layout.as_ref().is_some_and(|l| l.reverse_z);
        let order: Box<dyn Iterator<Item = usize>> =
            if reverse { Box::new(0..node.children.len()) } else { Box::new((0..node.children.len()).rev()) };
        for i in order.take(MAX_BACKDROP_SCAN) {
            let Some(child) = node.children.get(i).and_then(|id| doc.node(*id)) else { continue };
            if let Some(behind) = paint_behind(doc, child, target, depth + 1, overrides) {
                return Some(behind);
            }
        }
        if node.children.len() > MAX_BACKDROP_SCAN {
            return Some(Behind::Unknown);
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
    let color = match overrides {
        Some(modes) => solid_fill_in(doc, node, modes),
        None => solid_fill(node),
    };
    Some(match color {
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
