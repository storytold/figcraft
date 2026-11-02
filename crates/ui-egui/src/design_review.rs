//! The Design review window: run `design.audit` on the selection or page, list what it finds,
//! select the layer behind a finding and repair the safe ones with `design.fix`.
//!
//! The window is UI state (`UiState::design_review`); the commands stay in the engine, so the
//! same check is available to the CLI, MCP and scripts without this window.

use egui::{Context, RichText, Sense, vec2};
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::FigApp;
use crate::dialogs::floating;
use crate::i18n::{tr, trf};
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

/// The window's state, kept while it is open.
pub struct State {
    pub scope: Scope,
    pub report: Option<Value>,
    pub error: Option<String>,
}

impl State {
    fn new() -> Self {
        // A design review looks at the screen, not at whatever happens to be selected.
        Self { scope: Scope::Page, report: None, error: None }
    }
}

enum Action {
    Check,
    FixAll,
    FixRule(String),
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

/// Repair every finding the audit marks fixable, then check again.
pub(crate) fn fix_all(app: &mut FigApp) {
    fix(app, None);
}

fn fix_rule(app: &mut FigApp, rule: &str) {
    fix(app, Some(rule));
}

fn fix(app: &mut FigApp, rule: Option<&str>) {
    let Some(scope) = app.ui.design_review.as_ref().map(|s| s.scope) else { return };
    let params = match rule {
        Some(rule) => json!({"scope": scope.id(), "rules": [rule]}),
        None => json!({"scope": scope.id()}),
    };
    if let Some(result) = app.run("design.fix", params) {
        let changed = result["changed"].as_u64().unwrap_or(0);
        app.toast(trf("Fixed {n} findings", &[("n", &changed)]));
    }
    check(app);
}

/// Draw the window. Called once a frame from `dialogs::show`.
pub(crate) fn show(app: &mut FigApp, ctx: &Context) {
    let Some(mut state) = app.ui.design_review.take() else { return };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut action: Option<Action> = None;
    let mut select: Option<NodeId> = None;
    let mut scope_changed = false;

    floating(tr("Design review"), ctx)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .resizable(true)
        .default_width(360.0)
        .open(&mut open)
        .show(ctx, |ui| {
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

            if let Some(error) = &state.error {
                ui.colored_label(t.danger, error);
            }

            let findings = state.report.as_ref().and_then(|r| r["findings"].as_array()).map(Vec::as_slice).unwrap_or(&[]);
            if let Some(report) = &state.report {
                let score = report["score"].as_f64().unwrap_or(0.0);
                let high = report["counts"]["high"].as_u64().unwrap_or(0);
                let medium = report["counts"]["medium"].as_u64().unwrap_or(0);
                let low = report["counts"]["low"].as_u64().unwrap_or(0);
                ui.label(
                    RichText::new(format!(
                        "{} {score:.0}   {} {high}  ·  {} {medium}  ·  {} {low}",
                        tr("Score"),
                        tr("High"),
                        tr("Medium"),
                        tr("Low")
                    ))
                    .size(12.0)
                    .color(t.text_secondary),
                );
            }
            ui.separator();

            if findings.is_empty() {
                ui.label(RichText::new(tr("No findings.")).size(12.0).color(t.text_secondary));
            } else {
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    for (i, finding) in findings.iter().enumerate() {
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
                                    ui.label(RichText::new(rule).size(12.0).strong());
                                    ui.label(RichText::new(path).size(10.0).color(t.text_secondary));
                                    ui.label(RichText::new(message).size(11.0).color(t.text));
                                    ui.horizontal(|ui| {
                                        if widgets::button(ui, tr("Select"), false, node.is_some()).clicked() {
                                            select = node;
                                        }
                                        if fixable && widgets::button(ui, tr("Fix"), false, true).clicked() {
                                            action = Some(Action::FixRule(rule.to_string()));
                                        }
                                    });
                                });
                            });
                            ui.separator();
                        });
                    }
                });
            }

            let fixable = findings.iter().filter(|f| f["fixable"].as_bool().unwrap_or(false)).count();
            let label = if fixable == 0 { tr("Fix all").to_string() } else { format!("{} ({fixable})", tr("Fix all")) };
            if widgets::button(ui, &label, false, fixable > 0).clicked() {
                action = Some(Action::FixAll);
            }
        });

    if !open {
        return;
    }
    app.ui.design_review = Some(state);
    if let Some(id) = select {
        app.session.set_selection(vec![id]);
    }
    match action {
        Some(Action::Check) => check(app),
        Some(Action::FixAll) => fix_all(app),
        Some(Action::FixRule(rule)) => fix_rule(app, &rule),
        None if scope_changed => check(app),
        None => {}
    }
}
