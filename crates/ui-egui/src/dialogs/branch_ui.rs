//! Branches in the UI: the banner on a branch tab ("Branch of <file>", Review and merge) and the
//! review dialog: the main file and the branch side by side (or one over the other) with
//! Dev Mode's compare pieces, the conflicts with a branch / main choice each, what changed on each
//! side, and Update from main / Merge. The Branches list is in the version history panel.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Align2, CornerRadius, Id, Rect, Sense, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId};
use figcraft_engine::branches::{self, BranchState, Review, Side, SideChanges};
use figcraft_engine::devcompare::ChangeKind;
use serde_json::json;

use crate::FigApp;
use crate::dev_compare::{self, BAR_H, LIST_W, Mode, Shot};
use crate::i18n::{tr, trf, trn};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// The open review dialog.
pub struct ReviewView {
    /// The main file's tab (its uid) and the branch.
    pub main: u64,
    pub id: String,
    pub mode: Mode,
    /// Where the overlay slider is (0 = all branch, 1 = all main).
    pub split: f32,
    /// The side chosen per conflict key.
    pub picks: BTreeMap<String, Side>,
    /// The layer whose frame the images show.
    selected: Option<NodeId>,
    hovered: Option<NodeId>,
    cache: Option<Cached>,
}

impl ReviewView {
    pub fn new(main: u64, id: String) -> Self {
        Self { main, id, mode: Mode::default(), split: 0.5, picks: BTreeMap::new(), selected: None, hovered: None, cache: None }
    }
}

/// What the dialog shows, built for one version of the main file and the branch.
struct Cached {
    docs: [Arc<Document>; 3],
    review: Result<Review, String>,
    /// The frame the shots show, and the main file's and the branch's renderings of it.
    shots: Option<(NodeId, Option<Shot>, Option<Shot>)>,
}

/// Open the review of branch `id` of the file shown (or of the file a branch tab belongs to).
pub fn open(app: &mut FigApp, id: Option<&str>) {
    let Ok((mi, bi)) = branches::locate(&app.session, "branch.review", id) else { return };
    let found = app.session.docs.get(mi).and_then(|m| Some((m.uid, m.branches.get(bi)?.id.clone())));
    app.ui.branch_review = found.map(|(main, id)| ReviewView::new(main, id));
}

/// Escape closes the dialog (before the canvas's shortcuts see it).
pub fn keys(app: &mut FigApp, ctx: &egui::Context) {
    if app.ui.branch_review.is_some() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.ui.branch_review = None;
    }
}

/// On a branch tab: a banner over the canvas naming the branch and its file, with Review and
/// merge.
pub fn banner(app: &mut FigApp, ctx: &egui::Context, canvas: Rect) {
    let Ok(ds) = app.session.ds() else { return };
    let Some((main, id)) = ds.branch_of.clone() else { return };
    // A branch version being viewed shows its own banner in this place.
    if ds.focus.is_some() || ds.preview.is_some() || app.ui.branch_review.is_some() {
        return;
    }
    let Some(m) = app.session.docs.iter().find(|d| d.uid == main) else { return };
    let Some(b) = m.branches.iter().find(|b| b.id == id) else { return };
    let (file, name, active) = (m.doc.name.clone(), b.name.clone(), b.state == BranchState::Active);
    let t = Tokens::get(ctx);
    let review = widgets::canvas_banner(ctx, Id::unique("branch_banner"), canvas, |ui| {
        ui.label(egui::RichText::new(format!("“{name}”")).font(semibold(11.0)).color(t.text));
        let state =
            if active { trf("Branch of {file}", &[("file", &file)]) } else { trf("Branch of {file} · merged or archived", &[("file", &file)]) };
        ui.label(egui::RichText::new(state).font(regular(11.0)).color(t.text_secondary));
        ui.add_space(4.0);
        widgets::button(ui, tr("Review and merge"), true, active).clicked()
    });
    if review {
        app.ui.branch_review = Some(ReviewView::new(main, id));
    }
}

/// What the footer or a row asked for.
enum Action {
    Show(NodeId),
    Pick(String, Side),
    Update,
    Merge,
    Close,
}

/// The top-level frame (a page's child) `id` is in, or `id` itself.
fn frame_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let mut cur = id;
    for _ in 0..10_000 {
        let parent = doc.node(cur)?.parent?;
        if doc.node(parent)?.is_page() {
            return Some(cur);
        }
        cur = parent;
    }
    None
}

/// `doc`'s rendering of frame `frame`, the layers of `changes` in it outlined.
fn shot(ctx: &egui::Context, doc: &Document, frame: NodeId, changes: &SideChanges, conflicts: &[NodeId]) -> Option<Shot> {
    doc.node(frame)?;
    let marks = changes.layers.iter().map(|c| (c.id, c.change)).chain(conflicts.iter().map(|id| (*id, ChangeKind::Changed)));
    Shot::new(ctx, doc, frame, marks.filter(|(id, _)| *id != frame && frame_of(doc, *id) == Some(frame)))
}

/// The review dialog, while open.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut view) = app.ui.branch_review.take() else { return };
    let Some(mi) = app.session.docs.iter().position(|d| d.uid == view.main) else { return };
    let Some(bi) = app.session.docs.get(mi).and_then(|m| m.branches.iter().position(|b| b.id == view.id)) else { return };
    let Some((base, head, main)) = branches::documents(&app.session, mi, bi) else { return };
    let fresh = |c: &Cached| [&base, &head, &main].iter().zip(&c.docs).all(|(a, b)| Arc::ptr_eq(a, b));
    if !view.cache.as_ref().is_some_and(fresh) {
        let review = branches::review_at(&app.session, mi, bi).map_err(|e| e.to_string());
        view.picks.retain(|k, _| review.as_ref().is_ok_and(|r| r.conflicts.iter().any(|c| c.key == *k)));
        view.cache = Some(Cached { docs: [base, head.clone(), main.clone()], review, shots: None });
    }
    let Some(cache) = view.cache.as_mut() else { return };
    // The frame to show: the chosen layer's, else the first conflict's or change's.
    if let Ok(r) = &cache.review {
        let first = r.conflicts.iter().filter_map(|c| c.id).chain(r.branch.layers.iter().chain(&r.main.layers).map(|c| c.id)).next();
        let target = view.selected.or(first);
        let frame = target.and_then(|id| frame_of(&main, id).or_else(|| frame_of(&head, id)));
        if cache.shots.as_ref().map(|s| s.0) != frame {
            let conflicts: Vec<NodeId> = r.conflicts.iter().filter_map(|c| c.id).collect();
            cache.shots = frame.map(|f| (f, shot(ctx, &main, f, &r.main, &conflicts), shot(ctx, &head, f, &r.branch, &conflicts)));
        }
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let size = vec2(1120.0_f32.min(screen.width() - 64.0).max(560.0), 700.0_f32.min(screen.height() - 96.0).max(320.0));
    let mut open = true;
    let mut action = None;
    egui::Window::new("Review and merge")
        .id(Id::unique("branch_review_window"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .fixed_size(size)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(0)))
        .show(ctx, |ui| {
            ui.set_min_size(size);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let Some(cache) = view.cache.as_ref() else { return };
            let name = cache.review.as_ref().map(|r| r.name.clone()).unwrap_or_default();
            dev_compare::header(ui, &t, tr("Review and merge"), &name, &mut view.mode, &mut open);
            let body_h = size.y - BAR_H * 2.0 - 2.0;
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(size.x - LIST_W - 1.0, body_h), Sense::hover());
                match cache.shots.as_ref() {
                    Some((_, Some(m), Some(b))) => {
                        dev_compare::images(ui, &t, r, [(m, tr("Main")), (b, tr("Branch"))], view.mode, &mut view.split, view.hovered)
                    }
                    Some((_, None, Some(_))) => dev_compare::message(ui, &t, r, tr("This frame is new on the branch.")),
                    Some((_, Some(_), None)) => dev_compare::message(ui, &t, r, tr("This frame isn't on the branch.")),
                    _ => dev_compare::message(ui, &t, r, tr("No layer changes to show.")),
                }
                let (line, _) = ui.allocate_exact_size(vec2(1.0, body_h), Sense::hover());
                ui.painter().rect_filled(line, CornerRadius::ZERO, t.border);
                ui.allocate_ui_with_layout(vec2(LIST_W, body_h), egui::Layout::top_down(egui::Align::Min), |ui| match &cache.review {
                    Ok(r) => {
                        if let Some(a) = list(ui, &t, r, &view.picks, view.selected, &mut view.hovered) {
                            action = Some(a);
                        }
                    }
                    Err(e) => widgets::note(ui, &trf("This branch can't be reviewed: {e}", &[("e", e)]), t.danger),
                });
            });
            if let Ok(r) = &cache.review
                && let Some(a) = footer(ui, &t, r, &view.picks)
            {
                action = Some(a);
            }
        });
    let resolutions = || json!(view.picks.iter().map(|(k, s)| (k.clone(), json!(s.name()))).collect::<serde_json::Map<_, _>>());
    match action {
        Some(Action::Show(id)) => view.selected = Some(id),
        Some(Action::Pick(key, side)) => {
            view.picks.insert(key, side);
        }
        Some(Action::Update) => {
            if app.run("branch.updateFromMain", json!({"id": view.id, "resolutions": resolutions()})).is_some() {
                app.toast(tr("Branch updated from main"));
                view.picks.clear();
            }
        }
        Some(Action::Merge) => {
            if app.run("branch.merge", json!({"id": view.id, "resolutions": resolutions()})).is_some() {
                app.toast(tr("Branch merged"));
                open = false;
            }
        }
        Some(Action::Close) => open = false,
        None => {}
    }
    if open {
        app.ui.branch_review = Some(view);
    }
}

/// A section title with a count.
fn title(ui: &mut Ui, t: &Tokens, text: &str, n: usize) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    ui.painter().text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, format!("{text} · {n}"), semibold(11.0), t.text);
}

/// Conflicts with their choice, then each side's changes; a click on a layer shows its frame.
fn list(
    ui: &mut Ui,
    t: &Tokens,
    r: &Review,
    picks: &BTreeMap<String, Side>,
    selected: Option<NodeId>,
    hovered: &mut Option<NodeId>,
) -> Option<Action> {
    let mut out = None;
    let mut now_hovered = None;
    egui::ScrollArea::vertical().id_salt("branch_review_list").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        if !r.conflicts.is_empty() {
            title(ui, t, tr("Conflicts"), r.conflicts.len());
            for c in &r.conflicts {
                let label = match c.deleted_by {
                    Some(Side::Branch) => trf("{name} · deleted on the branch", &[("name", &c.name)]),
                    Some(Side::Main) => trf("{name} · deleted on main", &[("name", &c.name)]),
                    None => format!("{} · {}", c.name, c.property),
                };
                let resp = dev_compare::change_row(
                    ui,
                    t,
                    ChangeKind::Changed,
                    &label,
                    0,
                    1,
                    c.id.is_some() && c.id == selected,
                    tr("Changed on both sides: keep one"),
                );
                if resp.hovered() {
                    now_hovered = c.id;
                }
                if let Some(id) = c.id.filter(|_| resp.clicked()) {
                    out = Some(Action::Show(id));
                }
                let pick = picks.get(&c.key).copied();
                for (side, side_name, value) in [(Side::Branch, tr("Branch"), &c.branch), (Side::Main, tr("Main"), &c.main)] {
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        let text = widgets::elide(ui, &format!("{side_name}: {value}"), 200.0);
                        if widgets::radio(ui, pick == Some(side), &text) {
                            out = Some(Action::Pick(c.key.clone(), side));
                        }
                    });
                }
                ui.add_space(6.0);
            }
            widgets::separator(ui);
        }
        for (label, side) in [(tr("Changes on the branch"), &r.branch), (tr("Changes on main"), &r.main)] {
            title(ui, t, label, side.layers.len() + side.document.len());
            if side.layers.is_empty() && side.document.is_empty() {
                widgets::note(ui, tr("None."), t.text_secondary);
            }
            for c in &side.layers {
                let resp = dev_compare::change_row(
                    ui,
                    t,
                    c.change,
                    &c.name,
                    c.depth.saturating_sub(1),
                    c.layers,
                    selected == Some(c.id),
                    tr("Show this layer"),
                );
                if resp.hovered() {
                    now_hovered = Some(c.id);
                }
                if resp.clicked() {
                    out = Some(Action::Show(c.id));
                }
                dev_compare::property_lines(ui, t, c.depth.saturating_sub(1), c.properties.iter().map(|pc| pc.text.as_str()));
            }
            for d in &side.document {
                dev_compare::change_row(ui, t, d.change, &d.what, 0, 1, false, &d.what);
            }
            if side.truncated {
                widgets::note(ui, tr("Only the first changed layers are listed."), t.text_secondary);
            }
            ui.add_space(8.0);
        }
    });
    *hovered = now_hovered;
    out
}

fn footer(ui: &mut Ui, t: &Tokens, r: &Review, picks: &BTreeMap<String, Side>) -> Option<Action> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_H + 1.0), Sense::hover());
    ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), 1.0)), CornerRadius::ZERO, t.border);
    let left = r.conflicts.iter().filter(|c| !picks.contains_key(&c.key)).count();
    let active = r.state == BranchState::Active;
    let status = match (active, left) {
        (false, _) => tr("This branch was merged or archived.").to_string(),
        (true, 0) if r.conflicts.is_empty() => tr("No conflicts.").to_string(),
        (true, 0) => tr("Every conflict has a choice.").to_string(),
        (true, n) => trn("{n} conflicts to resolve", n, &[]),
    };
    ui.painter().text(
        pos2(rect.min.x + 16.0, rect.center().y),
        Align2::LEFT_CENTER,
        status,
        regular(11.0),
        if left > 0 { t.danger } else { t.text_secondary },
    );
    let mut out = None;
    let inner = Rect::from_min_max(pos2(rect.min.x + 16.0, rect.min.y + 1.0), pos2(rect.max.x - 16.0, rect.max.y));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let ready = active && left == 0;
        if widgets::button(ui, tr("Merge into main"), true, ready)
            .on_hover_text(tr("Merge the branch into the main file (one undo step) and archive it"))
            .clicked()
        {
            out = Some(Action::Merge);
        }
        let main_changed = !(r.main.layers.is_empty() && r.main.document.is_empty());
        if widgets::button(ui, tr("Update from main"), false, ready && main_changed)
            .on_hover_text(tr("Bring the main file's changes into the branch"))
            .clicked()
        {
            out = Some(Action::Update);
        }
        if widgets::button(ui, tr("Close"), false, true).clicked() {
            out = Some(Action::Close);
        }
    });
    out
}
