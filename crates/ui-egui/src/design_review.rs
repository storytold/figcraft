//! The Design review window: run `design.audit` on the selection or page, list what it finds,
//! select the layer behind a finding, ignore what is intentional and repair the safe ones with
//! `design.fix`.
//!
//! The window is UI state (`UiState::design_review`); the commands stay in the engine, so the
//! same check is available to the CLI, MCP and scripts without this window.

use std::collections::BTreeSet;

use egui::{Context, RichText, Sense, vec2};
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::FigApp;
use crate::dialogs::floating;
use crate::i18n::{tr, tr_noop, trf};
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
        "spacing.off-grid" => tr_noop("Spacing is off the 4px grid"),
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
        _ => rule,
    }
}

/// The window's state, kept while it is open.
pub struct State {
    pub scope: Scope,
    pub filter: SeverityFilter,
    /// Re-run after an edit, once the document has been still for a moment.
    pub live: bool,
    /// Show findings the user ignored in this session.
    pub show_ignored: bool,
    pub report: Option<Value>,
    pub error: Option<String>,
    /// Session-only ignores: one finding and whole rules.
    pub(crate) ignored: BTreeSet<(String, u64)>,
    pub(crate) ignored_rules: BTreeSet<String>,
    /// The document revision and egui time of the last check (live re-lint).
    revision: u64,
    last_check: f64,
}

impl State {
    fn new() -> Self {
        // A design review looks at the screen, not at whatever happens to be selected.
        Self {
            scope: Scope::Page,
            filter: SeverityFilter::All,
            live: false,
            show_ignored: false,
            report: None,
            error: None,
            ignored: BTreeSet::new(),
            ignored_rules: BTreeSet::new(),
            revision: 0,
            last_check: 0.0,
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
    app.ui.design_review = Some(State::new());
    check(app);
}

/// Run `design.audit` for the window's scope and keep the report.
pub(crate) fn check(app: &mut FigApp) {
    let Some(scope) = app.ui.design_review.as_ref().map(|s| s.scope) else { return };
    let result = app.run("design.audit", json!({"scope": scope.id()}));
    if let Some(state) = app.ui.design_review.as_mut() {
        match result {
            Some(report) => {
                state.report = Some(report);
                state.error = None;
            }
            None => state.error = Some(tr("The check failed; see the message below.").to_string()),
        }
    }
}

fn fix(app: &mut FigApp, params: Value) {
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

fn sync_revision(app: &mut FigApp, now: f64) {
    let revision = app.session.ds().map(|d| d.revision).unwrap_or(0);
    if let Some(state) = app.ui.design_review.as_mut() {
        state.revision = revision;
        state.last_check = now;
    }
}

/// Draw the window. Called once a frame from `dialogs::show`.
pub(crate) fn show(app: &mut FigApp, ctx: &Context) {
    let Some(mut state) = app.ui.design_review.take() else { return };
    let t = Tokens::get(ctx);
    let now = ctx.input(|i| i.time);
    let revision = app.session.ds().map(|d| d.revision).unwrap_or(0);
    // Live re-lint: the document changed and has been still for a moment.
    let live_due = state.live && revision != state.revision && now - state.last_check >= 0.4;
    let mut open = true;
    let mut action: Option<Action> = None;
    let mut select: Option<NodeId> = None;
    let mut comment: Option<(NodeId, String, String, String)> = None;
    let mut ignore_finding: Option<(String, u64)> = None;
    let mut ignore_rule: Option<String> = None;
    let mut select_rule: Option<String> = None;
    let mut clear_ignores = false;
    let mut scope_changed = false;

    let screen = ctx.content_rect();
    let default = egui::pos2((screen.center().x - 180.0).max(screen.min.x), (screen.center().y - 220.0).max(screen.min.y));
    floating(tr("Design review"), ctx).default_pos(default).resizable(true).movable(true).default_width(360.0).open(&mut open).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.horizontal(|ui| {
            for scope in [Scope::Selection, Scope::Page, Scope::Document] {
                if ui.selectable_value(&mut state.scope, scope, tr(scope.label())).changed() {
                    scope_changed = true;
                }
            }
            if widgets::button(ui, tr("Check design"), true, true).clicked() {
                action = Some(Action::Check);
            }
        });
        ui.horizontal(|ui| {
            for (filter, label) in
                [(SeverityFilter::All, "All"), (SeverityFilter::High, "High"), (SeverityFilter::Medium, "Medium"), (SeverityFilter::Low, "Low")]
            {
                ui.selectable_value(&mut state.filter, filter, tr(label));
            }
            ui.checkbox(&mut state.live, tr("Live"));
            ui.checkbox(&mut state.show_ignored, tr("Show ignored"));
            let ignored = state.ignored_count();
            if ignored > 0 {
                ui.label(RichText::new(format!("({ignored})")).size(10.0).color(t.text_secondary));
            }
        });

        if let Some(error) = &state.error {
            ui.colored_label(t.danger, error);
        }

        let all_findings = state.report.as_ref().and_then(|r| r["findings"].as_array()).map(Vec::as_slice).unwrap_or(&[]);
        let findings: Vec<&Value> = all_findings.iter().filter(|f| state.accepts(f)).collect();
        if let Some(report) = &state.report {
            let score = report["score"].as_f64().unwrap_or(0.0);
            let high = report["counts"]["high"].as_u64().unwrap_or(0);
            let medium = report["counts"]["medium"].as_u64().unwrap_or(0);
            let low = report["counts"]["low"].as_u64().unwrap_or(0);
            ui.label(
                RichText::new(format!("{} {score:.0}   {} {high}  ·  {} {medium}  ·  {} {low}", tr("Score"), tr("High"), tr("Medium"), tr("Low")))
                    .size(12.0)
                    .color(t.text_secondary),
            );
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
            ui.painter().rect_filled(bar, egui::CornerRadius::same(3), t.field);
            let filled = egui::Rect::from_min_size(bar.min, vec2(bar.width() * (score.clamp(0.0, 100.0) as f32 / 100.0), 6.0));
            let color = if score >= 90.0 {
                t.success
            } else if score >= 70.0 {
                t.warning
            } else {
                t.danger
            };
            ui.painter().rect_filled(filled, egui::CornerRadius::same(3), color);
        }
        ui.separator();

        if findings.is_empty() {
            ui.label(RichText::new(tr("No findings.")).size(12.0).color(t.text_secondary));
        } else {
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                for (i, finding) in findings.iter().copied().enumerate() {
                    let rule = finding["rule"].as_str().unwrap_or("");
                    let severity = finding["severity"].as_str().unwrap_or("");
                    let path = finding["path"].as_str().unwrap_or("");
                    let message = finding["message"].as_str().unwrap_or("");
                    let fixable = finding["fixable"].as_bool().unwrap_or(false);
                    let node = finding["node"].as_u64().map(NodeId);
                    let color = match severity {
                        "high" => t.danger,
                        "medium" => t.warning,
                        _ => t.text_tertiary,
                    };
                    ui.push_id(i, |ui| {
                        ui.horizontal(|ui| {
                            let (dot, _) = ui.allocate_exact_size(vec2(9.0, 9.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 3.0, color);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(tr(rule_label(rule))).size(12.0).strong()).on_hover_text(rule);
                                ui.label(RichText::new(path).size(10.0).color(t.text_secondary));
                                ui.label(RichText::new(message).size(11.0).color(t.text));
                                ui.horizontal(|ui| {
                                    if widgets::button(ui, tr("Select"), false, node.is_some()).clicked() {
                                        select = node;
                                    }
                                    if fixable
                                        && let Some(id) = node
                                        && widgets::button(ui, tr("Fix"), false, true).clicked()
                                    {
                                        action = Some(Action::FixFinding(rule.to_string(), id));
                                    }
                                    if widgets::button(ui, tr("Comment"), false, node.is_some()).clicked()
                                        && let Some(id) = node
                                    {
                                        comment = Some((id, rule.to_string(), message.to_string(), path.to_string()));
                                    }
                                    ui.menu_button("…", |ui| {
                                        if let Some(id) = node
                                            && ui.button(tr("Ignore this finding")).clicked()
                                        {
                                            ignore_finding = Some((rule.to_string(), id.0));
                                            ui.close();
                                        }
                                        if ui.button(tr("Ignore all of this rule")).clicked() {
                                            ignore_rule = Some(rule.to_string());
                                            ui.close();
                                        }
                                        let count = all_findings.iter().filter(|f| f["rule"] == rule).count();
                                        if ui.button(format!("{} ({count})", tr("Select all"))).clicked() {
                                            select_rule = Some(rule.to_string());
                                            ui.close();
                                        }
                                    });
                                });
                            });
                        });
                        ui.separator();
                    });
                }
            });
        }

        let fixable = findings.iter().filter(|f| f["fixable"].as_bool().unwrap_or(false)).count();
        ui.horizontal(|ui| {
            if state.ignored_count() > 0 && widgets::button(ui, tr("Clear ignores"), false, true).clicked() {
                clear_ignores = true;
            }
            let label = if fixable == 0 { tr("Fix all").to_string() } else { format!("{} ({fixable})", tr("Fix all")) };
            if widgets::button(ui, &label, false, fixable > 0).clicked() {
                action = Some(Action::FixAll);
            }
        });
    });

    if !open {
        return;
    }
    if let Some((rule, node)) = ignore_finding {
        state.ignored.insert((rule, node));
    }
    if let Some(rule) = ignore_rule {
        state.ignored_rules.insert(rule);
    }
    if clear_ignores {
        state.ignored.clear();
        state.ignored_rules.clear();
    }
    if let Some(rule) = select_rule {
        let ids = state.nodes(Some(&rule), state.show_ignored);
        app.session.set_selection(ids);
    }
    app.ui.design_review = Some(state);
    if let Some(id) = select {
        app.session.set_selection(vec![id]);
    }
    if let Some((id, rule, message, path)) = comment {
        comment_on(app, id, &rule, &message, &path);
    }
    match action {
        Some(Action::Check) => check(app),
        Some(Action::FixAll) => fix_all(app),
        Some(Action::FixFinding(rule, node)) => fix_finding(app, &rule, node),
        None if scope_changed || live_due => check(app),
        None => {}
    }
    sync_revision(app, now);
}
