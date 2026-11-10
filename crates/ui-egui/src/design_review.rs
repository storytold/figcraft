//! The Design review window: run `design.audit` on the selection or page, list what it finds,
//! select the layer behind a finding, ignore what is intentional and repair the safe ones with
//! `design.fix`.
//!
//! The window is UI state (`UiState::design_review`); the commands stay in the engine, so the
//! same check is available to the CLI, MCP and scripts without this window.

use std::collections::BTreeSet;

use egui::{Align2, Color32, Context, CornerRadius, RichText, Sense, Stroke, StrokeKind, vec2};
use figcraft_doc::{DesignPreset, NodeId, trim_num};
use serde_json::{Value, json};

use crate::FigApp;
use crate::dialogs::floating;
use crate::i18n::{tr, tr_noop, trf};
use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::widgets;

/// What the check looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Selection,
    Page,
    Document,
}

impl Scope {
    fn id(self) -> &'static str {
        match self {
            Scope::Selection => "selection",
            Scope::Page => "page",
            Scope::Document => "document",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Scope::Selection => "Selection",
            Scope::Page => "Page",
            Scope::Document => "Document",
        }
    }
}

/// The severity filter above the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeverityFilter {
    All,
    High,
    Medium,
    Low,
}

impl SeverityFilter {
    fn matches(self, severity: &str) -> bool {
        match self {
            SeverityFilter::All => true,
            SeverityFilter::High => severity == "high",
            SeverityFilter::Medium => severity == "medium",
            SeverityFilter::Low => severity == "low",
        }
    }
}

/// A rule id as the sentence the catalogs translate; unknown ids read as themselves.
fn rule_label(rule: &str) -> &str {
    match rule {
        "spacing.off-grid" => tr_noop("Spacing is off the grid"),
        "radius.off-scale" => tr_noop("Corner radius is off the scale"),
        "type.sizes" => tr_noop("Too many type sizes"),
        "type.families" => tr_noop("Too many font families"),
        "type.weight" => tr_noop("Heading weight is too heavy"),
        "type.italic" => tr_noop("Italic interface text"),
        "type.too-small" => tr_noop("Body text is too small"),
        "type.negative-tracking" => tr_noop("Small text has negative tracking"),
        "color.gradient" => tr_noop("A gradient fill"),
        "color.gradient-text" => tr_noop("Gradient text"),
        "color.glow" => tr_noop("A glow instead of a shadow"),
        "color.colored-shadow" => tr_noop("A coloured shadow"),
        "color.border-opacity" => tr_noop("A border stronger than a hairline"),
        "color.contrast" => tr_noop("Text contrast is too low"),
        "token.hardcoded-color" => tr_noop("Colour matches a variable but is not bound"),
        "token.hardcoded-number" => tr_noop("Value matches a variable but is not bound"),
        "icon.emoji" => tr_noop("Emoji in the interface"),
        "copy.exclamation" => tr_noop("An exclamation mark in copy"),
        "copy.trailing-period" => tr_noop("A short label ends in a period"),
        "copy.hype" => tr_noop("Hype words in copy"),
        "naming.auto" => tr_noop("A layer still has its default name"),
        "detail.shadow-count" => tr_noop("Too many shadows or blurs"),
        "layout.text-overflow" => tr_noop("Text overflows its box"),
        "color.accents" => tr_noop("More than two accent colours"),
        "layout.no-auto-layout" => tr_noop("A frame with no auto layout"),
        "layout.clipped" => tr_noop("A frame clips its content"),
        _ => rule,
    }
}

/// The preset's name in the interface catalogs (the key is the English text).
fn preset_label(preset: DesignPreset) -> &'static str {
    match preset {
        DesignPreset::Recommended => tr_noop("Recommended"),
        DesignPreset::Strict => tr_noop("Strict"),
        DesignPreset::Accessibility => tr_noop("Accessibility"),
    }
}

/// A finding's values as a compact line the interface language can read: field names are
/// translated, numbers, colours and layer names stay as they are. Empty for rules whose label
/// already says everything.
pub(crate) fn detail_line(rule: &str, details: &Value) -> String {
    let number = |key: &str| details[key].as_f64().map(trim_num);
    let strings = |key: &str| -> Vec<String> {
        details[key].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    match rule {
        "spacing.off-grid" => {
            let mut parts = Vec::new();
            if let Some(gap) = number("gap") {
                parts.push(format!("{} {gap}", tr("Gap")));
            }
            if let Some(gap) = number("crossGap") {
                parts.push(format!("{} {gap}", tr("Gap")));
            }
            let padding: Vec<&Value> = details["padding"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
            if !padding.is_empty() {
                let values: Vec<String> = padding.iter().filter_map(|v| v.as_f64()).map(trim_num).collect();
                parts.push(format!("{} {}", tr("Padding"), values.join(" ")));
            }
            if let Some(grid) = number("grid") {
                parts.push(format!("{grid} px"));
            }
            parts.join(" · ")
        }
        "radius.off-scale" => {
            let radii: Vec<String> =
                details["radii"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).map(trim_num).collect()).unwrap_or_default();
            match number("suggested") {
                Some(to) if !radii.is_empty() => format!("{} → {to}", radii.join(" · ")),
                _ => radii.join(" · "),
            }
        }
        "type.sizes" => strings("sizes").join(" · "),
        "type.families" => strings("families").join(" · "),
        "type.weight" => number("weight").map(|w| format!("{} {w}", tr("Weight"))).unwrap_or_default(),
        "type.too-small" => number("size").map(|s| format!("{s} px")).unwrap_or_default(),
        "type.negative-tracking" => number("tracking").map(|t| format!("{t} px")).unwrap_or_default(),
        "color.contrast" => {
            let (Some(ratio), Some(need)) = (details["ratio"].as_f64(), details["needed"].as_f64()) else { return String::new() };
            let fg = details["foreground"].as_str().unwrap_or("");
            let bg = details["background"].as_str().unwrap_or("");
            let mut line = format!("{ratio:.2}:1 < {need:.2}:1 · #{fg} / #{bg}");
            if let Some(modes) = details["modes"].as_object() {
                let names: Vec<String> = modes.iter().map(|(c, m)| format!("{c}: {}", m.as_str().unwrap_or(""))).collect();
                if !names.is_empty() {
                    line.push_str(" · ");
                    line.push_str(&names.join(", "));
                }
            }
            line
        }
        "color.accents" => strings("colors").join(" · "),
        "token.hardcoded-color" => {
            let hits: Vec<String> = details["hits"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|h| {
                            let kind = match h["kind"].as_str()? {
                                "fill" => tr("Fill").to_string(),
                                "stroke" => tr("Stroke").to_string(),
                                "text" => tr("Text").to_string(),
                                other => other.to_string(),
                            };
                            Some(format!("{kind} {} = {}", h["color"].as_str()?, h["token"].as_str()?))
                        })
                        .collect()
                })
                .unwrap_or_default();
            hits.join(" · ")
        }
        "token.hardcoded-number" => {
            // The same value on four padding sides is one line, not four.
            let mut lines = std::collections::BTreeSet::new();
            for h in details["hits"].as_array().into_iter().flatten() {
                let (Some(field), Some(value), Some(token)) = (h["field"].as_str().map(style_field_label), h["value"].as_f64(), h["token"].as_str())
                else {
                    continue;
                };
                lines.insert(format!("{field} {} = {token}", trim_num(value)));
            }
            lines.into_iter().collect::<Vec<_>>().join(" · ")
        }
        "naming.auto" => details["name"].as_str().map(|name| format!("“{name}”")).unwrap_or_default(),
        "detail.shadow-count" => match (details["shadowed"].as_u64(), details["nodes"].as_u64()) {
            (Some(shadowed), Some(nodes)) => format!("{shadowed} / {nodes}"),
            _ => String::new(),
        },
        "layout.text-overflow" => match (number("needed"), number("height")) {
            (Some(needed), Some(height)) => format!("{needed} px > {height} px"),
            _ => String::new(),
        },
        "layout.no-auto-layout" => details["children"].as_u64().map(|n| format!("{n} {}", tr("Layers"))).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The interface's name for a value's field (`paddingTop` → Padding).
fn style_field_label(field: &str) -> String {
    if field.starts_with("padding") {
        return tr("Padding").to_string();
    }
    if field.ends_with("Radius") || field == "cornerRadius" {
        return tr("Corner radius").to_string();
    }
    match field {
        "gap" | "columnGap" | "rowGap" | "crossGap" => tr("Gap").to_string(),
        "fontSize" => tr("Font size").to_string(),
        "fontWeight" => tr("Weight").to_string(),
        "opacity" => tr("Opacity").to_string(),
        "letterSpacing" => tr("Letter spacing").to_string(),
        _ => field.to_string(),
    }
}

/// Every rule id, for the design system's turned-off list (keep in sync with `rule_label`).
const RULE_IDS: &[&str] = &[
    "spacing.off-grid",
    "radius.off-scale",
    "type.sizes",
    "type.families",
    "type.weight",
    "type.italic",
    "type.too-small",
    "type.negative-tracking",
    "color.gradient",
    "color.gradient-text",
    "color.glow",
    "color.colored-shadow",
    "color.border-opacity",
    "color.contrast",
    "color.accents",
    "token.hardcoded-color",
    "token.hardcoded-number",
    "icon.emoji",
    "copy.exclamation",
    "copy.trailing-period",
    "copy.hype",
    "naming.auto",
    "detail.shadow-count",
    "layout.text-overflow",
    "layout.no-auto-layout",
    "layout.clipped",
];

/// `"0, 6 10"` (commas, spaces or semicolons) → radii, capped and rounded; `None` when a part is
/// not a usable number.
pub(crate) fn parse_num_list(text: &str, cap: usize) -> Option<Vec<f64>> {
    let mut out = Vec::new();
    for part in text.split([',', ' ', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let v: f64 = part.parse().ok()?;
        if !v.is_finite() || !(0.0..=999.0).contains(&v) {
            return None;
        }
        out.push((v * 100.0).round() / 100.0);
    }
    (out.len() <= cap).then_some(out)
}

/// `"#FF0000, #0A0"` → hex colour strings (`#0A0` expands); `None` when a part is not one.
pub(crate) fn parse_color_list(text: &str, cap: usize) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for part in text.split([',', ' ', ';']) {
        let part = part.trim().trim_start_matches('#');
        if part.is_empty() {
            continue;
        }
        let hex = match part.len() {
            6 if part.bytes().all(|b| b.is_ascii_hexdigit()) => part.to_ascii_uppercase(),
            3 if part.bytes().all(|b| b.is_ascii_hexdigit()) => {
                let expanded: String = part.chars().flat_map(|c| [c, c]).collect();
                expanded.to_ascii_uppercase()
            }
            _ => return None,
        };
        out.push(format!("#{hex}"));
    }
    (out.len() <= cap).then_some(out)
}

/// Why a finding is not fixable, as the chip's label and its hover text.
fn report_reason(finding: &Value) -> (String, String) {
    match finding["reason"].as_str() {
        Some("bound") => (tr("Report only").to_string(), tr("Bound to a variable or style; fix the variable or unbind first.").to_string()),
        Some("modes") => (tr("Report only").to_string(), tr("Read in the chosen variable modes; fixing would change the base colours.").to_string()),
        _ => (tr("Report only").to_string(), tr("Needs a decision (content, naming or gradients); the review does not guess.").to_string()),
    }
}

/// Run `design.configure` with the design-system changes the window collected. `radii`, `brand`
/// and `toggle` come from the editor's fields; `toggle` is a rule id and whether it is now off.
/// False when there was nothing to apply.
pub(crate) fn apply_design_system(app: &mut FigApp, radii: Option<Vec<f64>>, brand: Option<Vec<String>>, toggle: Option<(String, bool)>) -> bool {
    use serde_json::Map;
    let mut params = Map::new();
    if let Some(r) = radii {
        params.insert("radii".into(), json!(r));
    }
    if let Some(b) = brand {
        params.insert("brand".into(), json!(b));
    }
    if let Some((rule, off)) = toggle {
        let mut disabled: Vec<String> = app.session.ds().map(|d| d.doc.design_review.disabled.clone()).unwrap_or_default();
        if off {
            if !disabled.contains(&rule) {
                disabled.push(rule);
                disabled.sort();
            }
        } else {
            disabled.retain(|x| x != &rule);
        }
        params.insert("disabled".into(), json!(disabled));
    }
    if params.is_empty() {
        return false;
    }
    app.run("design.configure", Value::Object(params));
    true
}

/// The window's state, kept while it is open.
pub struct State {
    pub scope: Scope,
    pub filter: SeverityFilter,
    /// Re-run after an edit, once the document has been still for a moment.
    pub live: bool,
    /// Show the findings the user ignored (saved files keep them in the preferences).
    pub show_ignored: bool,
    /// The file's preset and spacing grid, as the check uses them.
    pub preset: DesignPreset,
    pub grid: f64,
    /// The design-system editor is open, with its draft fields.
    pub show_config: bool,
    pub(crate) radii_text: String,
    pub(crate) brand_text: String,
    pub(crate) config_error: Option<String>,
    pub report: Option<Value>,
    pub error: Option<String>,
    /// Ignores: one finding and whole rules, kept in the preferences by document path.
    pub(crate) ignored: BTreeSet<(String, u64)>,
    pub(crate) ignored_rules: BTreeSet<String>,
    /// The document this review belongs to; switching tabs starts a fresh review rather than
    /// letting an old report's layer ids act on another file.
    pub(crate) doc_uid: u64,
    /// Live re-lint: the revision last seen, the revision the last check covered, and when the
    /// seen revision last changed.
    seen_revision: u64,
    checked_revision: u64,
    changed_at: f64,
}

impl State {
    fn new(app: &FigApp) -> Self {
        // A design review looks at the screen, not at whatever happens to be selected.
        let key = document_key(app);
        let mut ignored = BTreeSet::new();
        let mut ignored_rules = BTreeSet::new();
        if !key.is_empty() {
            for entry in app.session.prefs.design_review_ignored.get(&key).into_iter().flatten() {
                match entry.split_once('@').and_then(|(rule, node)| node.parse::<u64>().ok().map(|node| (rule, node))) {
                    Some((rule, node)) => {
                        ignored.insert((rule.to_string(), node));
                    }
                    None => {
                        ignored_rules.insert(entry.clone());
                    }
                }
            }
        }
        let config = app.session.ds().ok().map(|d| d.doc.design_review.clone()).unwrap_or_default();
        Self {
            scope: Scope::Page,
            filter: SeverityFilter::All,
            live: false,
            show_ignored: false,
            preset: config.preset,
            grid: config.grid(),
            show_config: false,
            radii_text: String::new(),
            brand_text: String::new(),
            config_error: None,
            report: None,
            error: None,
            ignored,
            ignored_rules,
            doc_uid: active_uid(app),
            seen_revision: 0,
            checked_revision: 0,
            changed_at: 0.0,
        }
    }

    fn is_ignored(&self, finding: &Value) -> bool {
        let rule = finding["rule"].as_str().unwrap_or("");
        let node = finding["node"].as_u64().unwrap_or(0);
        self.ignored_rules.contains(rule) || self.ignored.iter().any(|(r, n)| r == rule && *n == node)
    }

    /// Whether the list shows the finding: the severity filter passes and it is not ignored (or
    /// ignored findings are being shown).
    fn accepts(&self, finding: &Value) -> bool {
        self.filter.matches(finding["severity"].as_str().unwrap_or("")) && (self.show_ignored || !self.is_ignored(finding))
    }

    fn ignored_count(&self) -> usize {
        self.report.as_ref().and_then(|r| r["findings"].as_array()).map_or(0, |fs| fs.iter().filter(|f| self.is_ignored(f)).count())
    }

    /// Unique layers behind the visible findings, for `design.fix {ids}`. `include_ignored` also
    /// takes findings the user ignored (Select all while they are shown).
    fn nodes(&self, rule: Option<&str>, include_ignored: bool) -> Vec<NodeId> {
        let Some(findings) = self.report.as_ref().and_then(|r| r["findings"].as_array()) else { return Vec::new() };
        let mut seen = BTreeSet::new();
        findings
            .iter()
            .filter(|f| self.filter.matches(f["severity"].as_str().unwrap_or("")) && (include_ignored || !self.is_ignored(f)))
            .filter(|f| rule.is_none_or(|rule| f["rule"] == rule))
            .filter_map(|f| f["node"].as_u64().map(NodeId))
            .filter(|id| seen.insert(*id))
            .collect()
    }

    /// The rules among the findings Fix all may repair (ignored findings are left out).
    fn fix_rules(&self) -> Vec<String> {
        let Some(findings) = self.report.as_ref().and_then(|r| r["findings"].as_array()) else { return Vec::new() };
        findings
            .iter()
            .filter(|f| self.filter.matches(f["severity"].as_str().unwrap_or("")) && !self.is_ignored(f))
            .filter_map(|f| f["rule"].as_str().map(str::to_string))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

enum Action {
    Check,
    FixAll,
    FixFinding(String, NodeId),
}

/// Open the window and run the first check.
pub(crate) fn open(app: &mut FigApp) {
    app.ui.design_review = Some(State::new(app));
    check(app);
}

/// A review belongs to one document. When the active tab changes, start a fresh review for it so
/// an old report's layer ids can never act on another file.
pub(crate) fn sync_document(app: &mut FigApp) {
    if app.session.docs.is_empty() {
        app.ui.design_review = None;
        return;
    }
    let uid = active_uid(app);
    let switched = app.ui.design_review.as_ref().is_some_and(|s| s.doc_uid != uid);
    if switched {
        app.ui.design_review = Some(State::new(app));
        check(app);
    }
}

fn active_uid(app: &FigApp) -> u64 {
    app.session.docs.get(app.session.active).map(|d| d.uid).unwrap_or(0)
}

/// The preferences key the window's ignores are kept under: the document's path, empty for a
/// file that has never been saved (its ignores last for the session).
fn document_key(app: &FigApp) -> String {
    app.session.ds().ok().and_then(|d| d.path.clone()).unwrap_or_default()
}

/// Ignores kept for one document and documents remembered, so a long-lived settings file stays
/// small. Beyond these, the entries that sort first are dropped.
const MAX_IGNORES_PER_DOC: usize = 500;
const MAX_IGNORED_DOCS: usize = 200;

/// Keep the window's ignores in the preferences under the document's path. An unsaved file has no
/// key and keeps them for the session only.
pub(crate) fn persist_ignores(app: &mut FigApp, state: &State) {
    let key = document_key(app);
    if key.is_empty() {
        return;
    }
    let mut entries: Vec<String> = state.ignored_rules.iter().cloned().collect();
    entries.extend(state.ignored.iter().map(|(rule, node)| format!("{rule}@{node}")));
    entries.sort();
    entries.truncate(MAX_IGNORES_PER_DOC);
    if entries.is_empty() {
        app.session.prefs.design_review_ignored.remove(&key);
    } else {
        app.session.prefs.design_review_ignored.insert(key.clone(), entries);
    }
    while app.session.prefs.design_review_ignored.len() > MAX_IGNORED_DOCS {
        let Some(oldest) = app.session.prefs.design_review_ignored.keys().find(|k| **k != key).cloned() else { break };
        app.session.prefs.design_review_ignored.remove(&oldest);
    }
    app.services.save_prefs(&app.session.prefs);
}

/// Run `design.audit` for the window's scope and keep the report.
pub(crate) fn check(app: &mut FigApp) {
    let Some(scope) = app.ui.design_review.as_ref().map(|s| s.scope) else { return };
    let revision = app.session.ds().map(|d| d.revision).unwrap_or(0);
    let result = app.run("design.audit", json!({"scope": scope.id()}));
    if let Some(state) = app.ui.design_review.as_mut() {
        match result {
            Some(report) => {
                // The file's settings, not the window's memory, decide the next run.
                if let Some(preset) = report["preset"].as_str().and_then(DesignPreset::from_id) {
                    state.preset = preset;
                }
                if let Some(grid) = report["grid"].as_f64() {
                    state.grid = grid;
                }
                state.report = Some(report);
                state.error = None;
            }
            None => state.error = Some(tr("The check failed; see the message below.").to_string()),
        }
        // This check covers everything up to now: a later edit is what makes live re-lint due.
        state.seen_revision = revision;
        state.checked_revision = revision;
    }
}

fn fix(app: &mut FigApp, mut params: Value) {
    // The window repairs the rows the user kept, never a descendant finding it ignored.
    if let Some(o) = params.as_object_mut() {
        o.insert("exact".into(), Value::Bool(true));
    }
    if let Some(result) = app.run("design.fix", params) {
        let changed = result["changed"].as_u64().unwrap_or(0);
        app.toast(trf("Fixed {n}", &[("n", &changed)]));
    }
    check(app);
}

/// Repair the visible findings the audit marks fixable, then check again.
pub(crate) fn fix_all(app: &mut FigApp) {
    let Some(state) = app.ui.design_review.as_ref() else { return };
    let ids = state.nodes(None, false);
    let rules = state.fix_rules();
    if ids.is_empty() || rules.is_empty() {
        return;
    }
    fix(app, json!({"ids": ids, "rules": rules}));
}

fn fix_finding(app: &mut FigApp, rule: &str, node: NodeId) {
    fix(app, json!({"ids": [node], "rules": [rule]}));
}

/// Post one finding as a comment anchored to its layer, so the team can discuss it in the
/// comments workflow (`fix_comments` / `watch_comments`).
pub(crate) fn comment_on(app: &mut FigApp, node: NodeId, rule: &str, message: &str, path: &str) {
    let found = app.session.doc().ok().and_then(|d| Some((d.world_bounds(node)?.center(), d.page_of(node))));
    let Some((center, page)) = found else { return };
    let text = format!("{} · {rule}\n{message}\n{path}", tr("Design review"));
    let text: String = text.chars().take(1_000).collect();
    app.run("comment.add", json!({"x": center.x, "y": center.y, "text": text, "anchor": node, "page": page}));
}

fn ui_font(size: f32) -> egui::FontId {
    egui::FontId::proportional(size)
}

/// A small rounded pill: the scope and severity choices, the grid sizes and the counts.
fn chip(ui: &mut egui::Ui, t: &Tokens, label: &str, selected: bool, dot: Option<Color32>) -> egui::Response {
    let font = ui_font(11.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), t.text);
    let dot_space = if dot.is_some() { 13.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + dot_space + 20.0, 26.0), Sense::click());
    crate::a11y::button(&resp, label);
    let fill = if selected {
        t.panel
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    if selected {
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
    }
    let mut x = rect.left() + 10.0;
    if let Some(color) = dot {
        ui.painter().circle_filled(egui::pos2(x + 3.0, rect.center().y), 3.0, color);
        x += 13.0;
    }
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        if selected || resp.hovered() { t.text } else { t.text_secondary },
    );
    resp
}

/// A row of chips inside a quiet well.
fn segmented(ui: &mut egui::Ui, t: &Tokens, options: &[(&str, bool)]) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::NONE.fill(t.field).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, (label, selected)) in options.iter().enumerate() {
                if chip(ui, t, label, *selected, None).clicked() {
                    clicked = Some(i);
                }
            }
        });
    });
    clicked
}

/// The window's main action (Check design, Fix all).
fn primary_button(ui: &mut egui::Ui, t: &Tokens, label: &str, icon: Option<Icon>) -> egui::Response {
    let font = ui_font(11.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), t.accent_on);
    let icon_space = if icon.is_some() { 16.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + icon_space + 24.0, 28.0), Sense::click());
    crate::a11y::button(&resp, label);
    ui.painter().rect_filled(rect, CornerRadius::same(6), if resp.hovered() { t.accent_hover } else { t.accent });
    let mut x = rect.left() + 12.0;
    if let Some(icon) = icon {
        icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(x + 6.0, rect.center().y), vec2(13.0, 13.0)), icon, t.accent_on);
        x += 16.0;
    }
    ui.painter().text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, label, font, t.accent_on);
    resp
}

/// A quiet icon-only action.
fn icon_button(ui: &mut egui::Ui, t: &Tokens, icon: Icon, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
    }
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), vec2(14.0, 14.0)),
        icon,
        if resp.hovered() { t.text } else { t.icon_secondary },
    );
    let resp = resp.on_hover_text(tip);
    crate::a11y::button(&resp, tip);
    resp
}

fn score_color(t: &Tokens, score: f64) -> Color32 {
    if score >= 90.0 {
        t.success
    } else if score >= 70.0 {
        // The theme's amber is for fills; as text on a light panel it needs a darker ink.
        if t.dark { t.warning } else { Color32::from_rgb(0xB4, 0x53, 0x09) }
    } else {
        t.danger
    }
}

fn score_bar(ui: &mut egui::Ui, t: &Tokens, score: f64, track: Color32) {
    let width = ui.available_width();
    let (bar, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    ui.painter().rect_filled(bar, CornerRadius::same(3), track);
    let filled = egui::Rect::from_min_size(bar.min, vec2(bar.width() * (score.clamp(0.0, 100.0) as f32 / 100.0), 6.0));
    ui.painter().rect_filled(filled, CornerRadius::same(3), score_color(t, score));
}

/// The score, its bar and the severity counts that double as the filter.
fn score_block(ui: &mut egui::Ui, t: &Tokens, score: f64, high: u64, medium: u64, low: u64, filter: &mut SeverityFilter, live: &mut bool) -> bool {
    let mut changed = false;
    let choices = [
        (SeverityFilter::All, tr("All"), high + medium + low, t.text_secondary),
        (SeverityFilter::High, tr("High"), high, t.danger),
        (SeverityFilter::Medium, tr("Medium"), medium, t.warning),
        (SeverityFilter::Low, tr("Low"), low, t.text_tertiary),
    ];
    let counts = |ui: &mut egui::Ui, filter: &mut SeverityFilter, changed: &mut bool| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (choice, label, count, color) in &choices {
                if chip(ui, t, &format!("{label} {count}"), *filter == *choice, Some(*color)).clicked() && *filter != *choice {
                    *filter = *choice;
                    *changed = true;
                }
            }
        });
    };
    egui::Frame::NONE.fill(t.field).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(format!("{score:.0}")).size(26.0).strong().color(score_color(t, score)));
                ui.label(RichText::new(tr("Score")).size(9.5).color(t.text_tertiary));
            });
            ui.add_space(12.0);
            ui.vertical(|ui| {
                score_bar(ui, t, score, t.panel);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    counts(ui, filter, &mut changed);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        changed |= ui.checkbox(live, tr("Live")).changed();
                    });
                });
            });
        });
    });
    changed
}

/// What the user asked for on one finding.
#[derive(Default)]
struct RowAction {
    select: Option<NodeId>,
    fix: Option<(String, NodeId)>,
    comment: Option<(NodeId, String, String, String)>,
    ignore: Option<(String, u64)>,
    ignore_rule: Option<String>,
    select_all: Option<String>,
}

fn finding_menu(ui: &mut egui::Ui, rule: &str, node: Option<NodeId>, all: &[Value], out: &mut RowAction) {
    ui.menu_button("…", |ui| {
        if let Some(id) = node
            && ui.button(tr("Ignore this finding")).clicked()
        {
            out.ignore = Some((rule.to_string(), id.0));
            ui.close();
        }
        if ui.button(tr("Ignore all of this rule")).clicked() {
            out.ignore_rule = Some(rule.to_string());
            ui.close();
        }
        let count = all.iter().filter(|f| f["rule"] == rule).count();
        if ui.button(format!("{} ({count})", tr("Select all"))).clicked() {
            out.select_all = Some(rule.to_string());
            ui.close();
        }
    });
}

/// One finding: its values and the actions, as a flat row.
fn finding_view(ui: &mut egui::Ui, t: &Tokens, finding: &Value, all: &[Value]) -> RowAction {
    let rule = finding["rule"].as_str().unwrap_or("");
    let severity = finding["severity"].as_str().unwrap_or("");
    let path = finding["path"].as_str().unwrap_or("");
    let detail = detail_line(rule, &finding["details"]);
    let message = finding["message"].as_str().unwrap_or("");
    let fixable = finding["fixable"].as_bool().unwrap_or(false);
    let node = finding["node"].as_u64().map(NodeId);
    let color = match severity {
        "high" => t.danger,
        "medium" => t.warning,
        _ => t.text_tertiary,
    };
    let mut out = RowAction::default();
    let actions = |ui: &mut egui::Ui, out: &mut RowAction| {
        if widgets::button(ui, tr("Select"), false, node.is_some()).clicked() {
            out.select = node;
        }
        if fixable {
            if let Some(id) = node
                && widgets::button(ui, tr("Fix"), true, true).clicked()
            {
                out.fix = Some((rule.to_string(), id));
            }
        } else {
            let (label, tip) = report_reason(finding);
            ui.label(RichText::new(label).size(9.5).color(t.text_tertiary)).on_hover_text(tip);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            finding_menu(ui, rule, node, all, out);
            if icon_button(ui, t, Icon::Comment, tr("Comment")).clicked()
                && let Some(id) = node
            {
                out.comment = Some((id, rule.to_string(), message.to_string(), path.to_string()));
            }
        });
    };
    ui.horizontal(|ui| {
        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
        if severity != "low" {
            ui.painter().circle_filled(dot.center(), 3.5, color);
        } else {
            ui.painter().circle_stroke(dot.center(), 3.0, Stroke::new(1.0, color));
        }
        ui.add_space(2.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(tr(rule_label(rule))).size(12.0).strong().color(t.text)).on_hover_text(rule);
            ui.horizontal(|ui| {
                ui.label(RichText::new(path).size(9.5).color(t.text_tertiary));
                if !detail.is_empty() {
                    ui.label(RichText::new(&detail).size(10.5).color(t.text_secondary));
                }
            });
        });
    });
    ui.horizontal(|ui| actions(ui, &mut out));
    out
}

/// Live re-lint waits this long after the last change, so typing does not re-audit every frame.
const LIVE_DELAY: f64 = 0.4;

/// Whether the changed document has been still long enough for a live re-check.
pub(crate) fn live_recheck_due(live: bool, seen: u64, checked: u64, changed_at: f64, now: f64) -> bool {
    live && seen != checked && now - changed_at >= LIVE_DELAY
}

/// Draw the window. Called once a frame from `dialogs::show`.
pub(crate) fn show(app: &mut FigApp, ctx: &Context) {
    sync_document(app);
    let Some(mut state) = app.ui.design_review.take() else { return };
    let t = Tokens::get(ctx);
    let now = ctx.input(|i| i.time);
    let revision = app.session.ds().map(|d| d.revision).unwrap_or(0);
    if revision != state.seen_revision {
        state.seen_revision = revision;
        state.changed_at = now;
    }
    // Live re-lint: the document changed and has been still for a moment.
    let live_due = live_recheck_due(state.live, state.seen_revision, state.checked_revision, state.changed_at, now);
    let mut open = true;
    let mut action: Option<Action> = None;
    let mut select: Option<NodeId> = None;
    let mut comment: Option<(NodeId, String, String, String)> = None;
    let mut ignore_finding: Option<(String, u64)> = None;
    let mut ignore_rule: Option<String> = None;
    let mut select_rule: Option<String> = None;
    let mut clear_ignores = false;
    let mut scope_changed = false;
    let mut config_changed = false;
    let mut config_radii: Option<Vec<f64>> = None;
    let mut config_brand: Option<Vec<String>> = None;
    let mut config_toggle: Option<(String, bool)> = None;

    let width = 430.0;
    let screen = ctx.content_rect();
    // A fixed window height keeps the footer on the window instead of over the toolbar; the list
    // takes what is left after the header, the score and the footer.
    let window_height = (screen.height() - 120.0).clamp(420.0, 720.0);
    let default = egui::pos2((screen.center().x - width / 2.0).max(screen.min.x), (screen.center().y - window_height / 2.0).max(screen.min.y + 10.0));
    floating(tr("Design review"), ctx)
        .default_pos(default)
        .default_height(window_height)
        .resizable(true)
        .movable(true)
        .default_width(width)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                let scopes = [Scope::Selection, Scope::Page, Scope::Document];
                let options: Vec<(&str, bool)> = scopes.iter().map(|s| (tr(s.label()), *s == state.scope)).collect();
                if let Some(choice) = segmented(ui, &t, &options) {
                    state.scope = scopes[choice];
                    scope_changed = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, &t, tr("Check design"), Some(Icon::Check)).clicked() {
                        action = Some(Action::Check);
                    }
                });
            });
            ui.add_space(2.0);
            let config = app.session.ds().map(|d| d.doc.design_review.clone()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let combo = craft_ui::ComboBox::from_id_salt("design_review_preset")
                    .width(108.0)
                    .selected_text(tr(preset_label(state.preset)))
                    .show_ui(ui, |ui| {
                        for preset in DesignPreset::ALL {
                            if ui.selectable_value(&mut state.preset, preset, tr(preset_label(preset))).changed() {
                                config_changed = true;
                            }
                        }
                    });
                combo.response.on_hover_text(tr("Preset"));
                for grid in [4.0, 8.0] {
                    let selected = (state.grid - grid).abs() < 0.5;
                    let r = chip(ui, &t, &format!("{grid:.0} px"), selected, None);
                    if r.clicked() && !selected {
                        state.grid = grid;
                        config_changed = true;
                    }
                    r.on_hover_text(tr("Grid"));
                }
            });
            // The design system gets its own row: the preset row is full in the wider catalogs.
            let open = state.show_config;
            let label = if open { tr("Hide design system") } else { tr("Design system…") };
            let r = chip(ui, &t, label, open, None);
            if r.clicked() {
                state.show_config = !state.show_config;
                if state.show_config && state.radii_text.is_empty() && state.brand_text.is_empty() {
                    state.radii_text = config.radii.iter().map(|r| trim_num(*r)).collect::<Vec<_>>().join(", ");
                    state.brand_text = config.brand.iter().map(|c| format!("#{}", c.to_hex())).collect::<Vec<_>>().join(", ");
                }
            }
            if state.show_config {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Radii")).size(10.0).color(t.text_tertiary));
                    let r =
                        ui.add(egui::TextEdit::singleline(&mut state.radii_text).desired_width(180.0).hint_text(tr("Empty: grid steps and pills")));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        match parse_num_list(&state.radii_text, figcraft_doc::MAX_RADII) {
                            Some(v) => {
                                state.config_error = None;
                                state.radii_text = v.iter().map(|r| trim_num(*r)).collect::<Vec<_>>().join(", ");
                                config_radii = Some(v);
                            }
                            None => state.config_error = Some(tr("Radii must be numbers, like 0, 6, 10, 14.").to_string()),
                        }
                    }
                    r.on_hover_text(tr("Accepted corner radii in px, separated by commas or spaces."));
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("Brand colours")).size(10.0).color(t.text_tertiary));
                    let r = ui.add(egui::TextEdit::singleline(&mut state.brand_text).desired_width(180.0).hint_text(tr("#FF0000, #00AA00")));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        match parse_color_list(&state.brand_text, figcraft_doc::MAX_BRAND_COLORS) {
                            Some(v) => {
                                state.config_error = None;
                                state.brand_text = v.join(", ");
                                config_brand = Some(v);
                            }
                            None => state.config_error = Some(tr("Brand colours must be hex colours, like #FF0000, #00AA00.").to_string()),
                        }
                    }
                    r.on_hover_text(tr("Accent colours the file owns, in hex, separated by commas."));
                    for color in &config.brand {
                        let [cr, cg, cb, _] = color.to_rgba8();
                        let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(cr, cg, cb));
                    }
                });
                if let Some(error) = &state.config_error {
                    ui.colored_label(t.danger, error);
                }
                let off = |rule: &str| config.disabled.iter().any(|r| r == rule);
                ui.label(RichText::new(tr("Disabled rules")).size(10.0).color(t.text_tertiary));
                egui::ScrollArea::vertical().id_salt("design_review_disabled_rules").max_height(132.0).show(ui, |ui| {
                    for id in RULE_IDS {
                        let mut on = !off(id);
                        if ui.checkbox(&mut on, tr(rule_label(id))).changed() {
                            config_toggle = Some(((*id).to_string(), !on));
                        }
                    }
                });
            }

            if let Some(error) = &state.error {
                ui.add_space(6.0);
                ui.colored_label(t.danger, error);
            }
            let summary = state.report.as_ref().map(|r| {
                (
                    r["score"].as_f64().unwrap_or(0.0),
                    r["counts"]["high"].as_u64().unwrap_or(0),
                    r["counts"]["medium"].as_u64().unwrap_or(0),
                    r["counts"]["low"].as_u64().unwrap_or(0),
                )
            });
            if let Some((score, high, medium, low)) = summary {
                ui.add_space(10.0);
                score_block(ui, &t, score, high, medium, low, &mut state.filter, &mut state.live);
            }
            ui.add_space(10.0);

            let all_findings = state.report.as_ref().and_then(|r| r["findings"].as_array()).map(Vec::as_slice).unwrap_or(&[]);
            let findings: Vec<&Value> = all_findings.iter().filter(|f| state.accepts(f)).collect();
            if findings.is_empty() {
                let (mark, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                icons::paint(ui.painter(), egui::Rect::from_center_size(mark.center(), vec2(22.0, 22.0)), Icon::Check, t.success);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(tr("No findings.")).size(12.0).color(t.text_secondary));
                });
            } else {
                egui::ScrollArea::vertical().max_height((ui.available_height() - 54.0).max(140.0)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for (i, finding) in findings.iter().copied().enumerate() {
                        ui.push_id(i, |ui| {
                            let a = finding_view(ui, &t, finding, all_findings);
                            if let Some(id) = a.select {
                                select = Some(id);
                            }
                            if let Some((rule, node)) = a.fix {
                                action = Some(Action::FixFinding(rule, node));
                            }
                            if let Some(x) = a.comment {
                                comment = Some(x);
                            }
                            if let Some(x) = a.ignore {
                                ignore_finding = Some(x);
                            }
                            if let Some(x) = a.ignore_rule {
                                ignore_rule = Some(x);
                            }
                            if let Some(x) = a.select_all {
                                select_rule = Some(x);
                            }
                            if i + 1 < findings.len() {
                                ui.separator();
                            }
                        });
                    }
                });
            }

            ui.add_space(6.0);
            let fixable = findings.iter().filter(|f| f["fixable"].as_bool().unwrap_or(false)).count();
            drop(findings);
            ui.horizontal(|ui| {
                ui.checkbox(&mut state.show_ignored, tr("Show ignored"));
                let ignored = state.ignored_count();
                if ignored > 0 {
                    ui.label(RichText::new(format!("({ignored})")).size(10.0).color(t.text_secondary));
                }
                if state.ignored_count() > 0 && widgets::button(ui, tr("Clear ignores"), false, true).clicked() {
                    clear_ignores = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if fixable == 0 { tr("Fix all").to_string() } else { format!("{} ({fixable})", tr("Fix all")) };
                    if primary_button(ui, &t, &label, None).clicked() && fixable > 0 {
                        action = Some(Action::FixAll);
                    }
                });
            });
        });

    if !open {
        return;
    }
    let mut ignores_changed = false;
    if let Some((rule, node)) = ignore_finding {
        state.ignored.insert((rule, node));
        ignores_changed = true;
    }
    if let Some(rule) = ignore_rule {
        state.ignored_rules.insert(rule);
        ignores_changed = true;
    }
    if clear_ignores {
        state.ignored.clear();
        state.ignored_rules.clear();
        ignores_changed = true;
    }
    if ignores_changed {
        persist_ignores(app, &state);
    }
    if let Some(rule) = select_rule {
        let ids = state.nodes(Some(&rule), state.show_ignored);
        app.session.set_selection(ids);
    }
    app.ui.design_review = Some(state);
    if let Some(id) = select {
        app.session.set_selection(vec![id]);
    }
    let mut config_applied = config_changed;
    if config_changed {
        // The preset and the grid belong to the file (`design.configure`); the check then runs
        // against them.
        if let Some((preset, grid)) = app.ui.design_review.as_ref().map(|s| (s.preset, s.grid)) {
            app.run("design.configure", json!({"preset": preset.id(), "grid": grid}));
        }
    }
    if apply_design_system(app, config_radii, config_brand, config_toggle) {
        config_applied = true;
    }
    if let Some((id, rule, message, path)) = comment {
        comment_on(app, id, &rule, &message, &path);
    }
    match action {
        Some(Action::Check) => check(app),
        Some(Action::FixAll) => fix_all(app),
        Some(Action::FixFinding(rule, node)) => fix_finding(app, &rule, node),
        None if scope_changed || config_applied || live_due => check(app),
        None => {}
    }
    // A changed document that is waiting for the delay still gets its frame even when the user
    // stops moving; otherwise an idle window would never run the pending check.
    if let Some(state) = app.ui.design_review.as_ref()
        && state.live
        && state.seen_revision != state.checked_revision
    {
        let wait = (LIVE_DELAY - (now - state.changed_at)).max(0.0);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
    }
}
