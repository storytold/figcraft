//! Dev Mode's compare view: a frame or section as it was when it was marked ready for dev, or in
//! a saved version picked at the bottom, next to how it is now (side by side, or one over the
//! other with a slider), the changed layers outlined, and the list of what changed
//! (`dev.compare`). Rendered once per document version. The branch review ([`crate::branch_ui`])
//! shows two versions with the same pieces.

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::{DevStatus, Document, NodeId};
use figcraft_engine::devcompare::{self, ChangeKind, Comparison, LayerChange};
use serde_json::json;

use crate::FigApp;
use crate::i18n::{tr, tr_noop, trf, trn};
use crate::icons::Icon;
use crate::theme::{ACCENT, DEV_CHANGED, Tokens, regular, semibold};
use crate::widgets;

/// Width of the list of changes.
pub(crate) const LIST_W: f32 = 330.0;
/// Height of the header and footer rows.
pub(crate) const BAR_H: f32 = 48.0;
/// Most texture pixels on an image's longer side.
const TEXTURE_SIDE: f32 = 1400.0;

/// How the two versions are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    SideBySide,
    /// The new version over the old one, the old one showing left of a slider.
    Overlay,
}

/// The open compare view.
pub struct CompareView {
    pub id: NodeId,
    pub mode: Mode,
    /// Where the overlay slider is (0 = all new, 1 = all old).
    pub split: f32,
    /// The saved version compared with (its id); `None`: the snapshot from when it was marked
    /// ready for dev.
    pub version: Option<String>,
    /// The change row under the pointer (its layer is highlighted).
    hovered: Option<NodeId>,
    cache: Option<Cached>,
}

impl CompareView {
    pub fn new(id: NodeId, version: Option<String>) -> Self {
        Self { id, mode: Mode::default(), split: 0.5, version, hovered: None, cache: None }
    }

    /// The view `dev.compare {show}` (or `ui.set`) asks for: `data` holds the layer's `id` and the
    /// `version` to compare with.
    pub fn requested(data: &serde_json::Value) -> Option<Self> {
        let id = data.get("id").and_then(figcraft_engine::params::id_of)?;
        Some(Self::new(id, data.get("version").and_then(serde_json::Value::as_str).map(str::to_string)))
    }
}

/// What the view shows, built for one version of the document and one "before".
struct Cached {
    doc: Arc<Document>,
    version: Option<String>,
    /// The version's document (read once per version picked).
    version_doc: Option<Arc<Document>>,
    cmp: Comparison,
    before: Option<Shot>,
    after: Option<Shot>,
}

/// A saved version as the picker lists it: (id, name, when).
type VersionRow = (String, String, u64);

/// One version rendered: its texture, the page area it shows and where each listed layer is.
pub(crate) struct Shot {
    tex: egui::TextureHandle,
    area: figcraft_geom::Rect,
    boxes: Vec<(NodeId, ChangeKind, figcraft_geom::Rect)>,
}

impl Shot {
    /// Layer `id` of `doc` rendered, the layers `marks` outlined.
    pub(crate) fn new(ctx: &egui::Context, doc: &Document, id: NodeId, marks: impl Iterator<Item = (NodeId, ChangeKind)>) -> Option<Self> {
        let area = figcraft_render::export_bounds(doc, id)?;
        let tex = crate::assets_panel::thumbnail(ctx, doc, id, TEXTURE_SIDE)?;
        let boxes = marks.filter_map(|(c, kind)| doc.world_bounds(c).map(|b| (c, kind, b))).collect();
        Some(Self { tex, area, boxes })
    }

    /// The old version outlines what was removed, the new one what was added or changed.
    fn of_changes(ctx: &egui::Context, doc: &Document, id: NodeId, changes: &[LayerChange], old: bool) -> Option<Self> {
        Self::new(ctx, doc, id, changes.iter().filter(|c| c.depth > 0 && (c.change == ChangeKind::Removed) == old).map(|c| (c.id, c.change)))
    }

    /// Its size in page units.
    fn size(&self) -> egui::Vec2 {
        vec2(self.area.width().max(1e-6) as f32, self.area.height().max(1e-6) as f32)
    }
}

/// Where a version is drawn: its top-left on screen and the screen points per page unit.
#[derive(Clone, Copy)]
struct Place {
    min: egui::Pos2,
    scale: f32,
}

impl Place {
    /// A `size` (page units) fitted and centred in `r`, at most 200%.
    fn fit(r: Rect, size: egui::Vec2) -> Self {
        let scale = (r.width() / size.x).min(r.height() / size.y).clamp(1e-3, 2.0);
        Self { min: r.center() - size * scale / 2.0, scale }
    }

    fn image(self, shot: &Shot) -> Rect {
        Rect::from_min_size(self.min, shot.size() * self.scale)
    }

    /// Page point → screen, for `shot`.
    fn at(self, shot: &Shot, x: f64, y: f64) -> egui::Pos2 {
        self.min + vec2((x - shot.area.x0) as f32, (y - shot.area.y0) as f32) * self.scale
    }
}

/// The colour a kind of change is marked with.
pub(crate) fn kind_color(t: &Tokens, k: ChangeKind) -> Color32 {
    match k {
        ChangeKind::Added => t.success,
        ChangeKind::Removed => t.danger,
        ChangeKind::Changed => DEV_CHANGED,
    }
}

/// Escape closes the view (before the canvas's shortcuts see it).
pub fn keys(app: &mut FigApp, ctx: &egui::Context) {
    if app.ui.dev.compare.is_some() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.ui.dev.compare = None;
    }
}

/// The compare view window, while open.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut view) = app.ui.dev.compare.take() else { return };
    let Ok(ds) = app.session.ds() else { return };
    let doc = ds.doc.clone();
    let Some(node) = doc.node(view.id) else { return };
    let ready = node.dev_status == DevStatus::ReadyForDev;
    // Newest first; a version deleted meanwhile falls back to the ready-for-dev snapshot (or the
    // newest version, for a layer that isn't ready for dev).
    let versions: Vec<VersionRow> = ds.versions.iter().rev().map(|v| (v.id.clone(), v.name.clone(), v.time)).collect();
    if view.version.as_ref().is_some_and(|v| !versions.iter().any(|(id, ..)| id == v)) || (!ready && view.version.is_none()) {
        view.version = if ready { None } else { versions.first().map(|(id, ..)| id.clone()) };
    }
    if view.version.is_none() && !ready {
        return;
    }
    if view.cache.as_ref().is_none_or(|c| !Arc::ptr_eq(&c.doc, &doc) || c.version != view.version) {
        let kept = view.cache.take().filter(|c| c.version == view.version).and_then(|c| c.version_doc);
        let version_doc = match &view.version {
            Some(v) => kept.or_else(|| ds.versions.iter().find(|x| x.id == *v).and_then(|x| x.document().ok())),
            None => None,
        };
        let (cmp, before) = match &version_doc {
            Some(b) => (devcompare::compare_with(b, &doc, view.id), Some(b.clone())),
            None if view.version.is_some() => (devcompare::empty(&doc, view.id), None),
            None => (devcompare::compare(&doc, view.id), devcompare::before_doc(&doc, view.id).map(Arc::new)),
        };
        let before = before.and_then(|b| Shot::of_changes(ctx, &b, view.id, &cmp.changes, true));
        let after = Shot::of_changes(ctx, &doc, view.id, &cmp.changes, false);
        view.cache = Some(Cached { doc: doc.clone(), version: view.version.clone(), version_doc, cmp, before, after });
    }
    let version_name = view.version.as_ref().and_then(|v| versions.iter().find(|(id, ..)| id == v)).map(|(_, name, _)| name.clone());
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let size = vec2(1120.0_f32.min(screen.width() - 64.0).max(560.0), 700.0_f32.min(screen.height() - 96.0).max(320.0));
    let mut open = true;
    let mut action = None;
    egui::Window::new("Compare changes")
        .id(Id::unique("dev_compare_window"))
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
            header(ui, &t, tr("Compare changes"), &cache.cmp.name, &mut view.mode, &mut open);
            let body_h = size.y - BAR_H * 2.0 - 2.0;
            let top_down = egui::Layout::top_down(egui::Align::Min);
            ui.horizontal_top(|ui| {
                let images_w = size.x - LIST_W - 1.0;
                let (r, _) = ui.allocate_exact_size(vec2(images_w, body_h), Sense::hover());
                compared(ui, r, cache, version_name.as_deref(), view.mode, &mut view.split, view.hovered);
                let (line, _) = ui.allocate_exact_size(vec2(1.0, body_h), Sense::hover());
                ui.painter().rect_filled(line, CornerRadius::ZERO, t.border);
                ui.allocate_ui_with_layout(vec2(LIST_W, body_h), top_down, |ui| {
                    if let Some(a) = list(ui, &t, &cache.cmp, view.version.is_some(), &mut view.hovered) {
                        action = Some(a);
                    }
                });
            });
            if let Some(a) = footer(ui, &t, &cache.cmp, ready, &versions, view.version.as_deref()) {
                action = Some(a);
            }
        });
    match action {
        Some(Action::Select(id)) => {
            app.run("select.set", json!({"ids": [id]}));
        }
        Some(Action::Resolve) => {
            if app.run("dev.resolveChange", json!({"ids": [view.id]})).is_some() {
                app.toast(tr("Change resolved"));
                open = false;
            }
        }
        Some(Action::Close) => open = false,
        Some(Action::Against(v)) => view.version = v,
        None => {}
    }
    if open {
        app.ui.dev.compare = Some(view);
    }
}

/// The images area: before and after, or a note saying why there is nothing to show.
/// `version_name`: the saved version compared with (`None`: the ready-for-dev snapshot).
fn compared(ui: &mut Ui, r: Rect, cache: &Cached, version_name: Option<&str>, mode: Mode, split: &mut f32, hovered: Option<NodeId>) {
    let t = &Tokens::get(ui.ctx());
    let against = version_name.is_some();
    if !cache.cmp.snapshot {
        let msg = if against {
            tr("This version can't be read.")
        } else if cache.cmp.changed {
            tr(
                "What this changed from isn't known: it changed before FigCraft kept a copy of ready-for-dev frames. Resolve the change (or mark it ready again) to compare from now on.",
            )
        } else {
            tr("Nothing changed since this was marked ready for dev.")
        };
        message(ui, t, r, msg);
    } else if !cache.cmp.changed && against {
        message(ui, t, r, tr("Nothing changed since this version."));
    } else if let (Some(before), Some(after)) = (&cache.before, &cache.after) {
        images(ui, t, r, [(before, version_name.unwrap_or(tr("Before"))), (after, tr("Now"))], mode, split, hovered);
    } else if against {
        message(ui, t, r, tr("This version doesn't have it: all of it is new."));
    }
}

enum Action {
    Select(NodeId),
    Resolve,
    Close,
    /// Compare with this saved version (`None`: the snapshot from when it was marked ready).
    Against(Option<String>),
}

/// The title row: `title` and `name`, the side by side / overlay switch and the close button.
pub(crate) fn header(ui: &mut Ui, t: &Tokens, title: &str, name: &str, mode: &mut Mode, open: &mut bool) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_H), Sense::hover());
    let p = ui.painter();
    let title = p.layout_no_wrap(title.into(), semibold(13.0), t.text);
    let tw = title.size().x;
    p.galley(pos2(r.min.x + 16.0, r.center().y - title.size().y / 2.0), title, t.text);
    let name = widgets::elide(ui, name, 320.0);
    ui.painter().text(pos2(r.min.x + 28.0 + tw, r.center().y), Align2::LEFT_CENTER, name, regular(12.0), t.text_secondary);
    // Side by side / overlay, as a two-way switch left of the close button.
    let modes = [(Mode::SideBySide, tr("Side by side")), (Mode::Overlay, tr("Overlay"))];
    let mut x = r.max.x - 48.0;
    for (m, label) in modes.iter().rev() {
        let g = ui.painter().layout_no_wrap((*label).into(), regular(11.0), t.text);
        let cell = Rect::from_min_max(pos2(x - g.size().x - 20.0, r.center().y - 12.0), pos2(x, r.center().y + 12.0));
        let resp = ui.interact(cell, Id::unique(("compare_mode", label)), Sense::click());
        let on = *mode == *m;
        crate::a11y::tab(&resp, label, on);
        if on {
            ui.painter().rect_filled(cell, CornerRadius::same(5), t.field);
            ui.painter().rect_stroke(cell, CornerRadius::same(5), Stroke::new(1.0, t.border), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(cell, CornerRadius::same(5), t.hover);
        }
        ui.painter().galley(cell.center() - g.size() / 2.0, g, if on { t.text } else { t.text_secondary });
        if resp.clicked() {
            *mode = *m;
        }
        x = cell.min.x - 4.0;
    }
    if widgets::icon_button_at(
        ui,
        Id::unique("compare_close"),
        Rect::from_center_size(pos2(r.max.x - 24.0, r.center().y), vec2(24.0, 24.0)),
        Icon::Close,
        tr("Close"),
    ) {
        *open = false;
    }
    let line = Rect::from_min_max(pos2(r.min.x, r.max.y - 1.0), r.max);
    ui.painter().rect_filled(line, CornerRadius::ZERO, t.border);
}

/// `msg` in the middle of the images' area `r`.
pub(crate) fn message(ui: &Ui, t: &Tokens, r: Rect, msg: &str) {
    let p = ui.painter_at(r);
    p.rect_filled(r, CornerRadius::ZERO, t.canvas);
    let g = p.layout(msg.into(), regular(12.0), t.text_secondary, (r.width() - 120.0).max(120.0));
    p.galley(r.center() - g.size() / 2.0, g, t.text_secondary);
}

/// The two versions (`[(old, its label), (new, its label)]`) in `r`.
pub(crate) fn images(ui: &mut Ui, t: &Tokens, r: Rect, shots: [(&Shot, &str); 2], mode: Mode, split: &mut f32, hovered: Option<NodeId>) {
    let p = ui.painter_at(r);
    p.rect_filled(r, CornerRadius::ZERO, t.canvas);
    let [(before, before_label), (after, after_label)] = shots;
    // Room for the labels above the images.
    let inner = r.shrink2(vec2(24.0, 20.0));
    let label_y = inner.min.y + 8.0;
    let area = Rect::from_min_max(pos2(inner.min.x, inner.min.y + 22.0), inner.max);
    match mode {
        Mode::SideBySide => {
            let w = (area.width() - 24.0) / 2.0;
            let left = Rect::from_min_size(area.min, vec2(w, area.height()));
            let right = Rect::from_min_size(pos2(left.max.x + 24.0, area.min.y), vec2(w, area.height()));
            for (shot, half, label) in [(before, left, before_label), (after, right, after_label)] {
                let place = Place::fit(half, shot.size());
                p.text(pos2(place.min.x, label_y), Align2::LEFT_CENTER, label, semibold(11.0), t.text_secondary);
                pane(&p, t, place, shot, hovered);
            }
        }
        Mode::Overlay => {
            // One placement for both, from their top-left corners, so layers that stayed put line up.
            let size = before.size().max(after.size());
            let place = Place::fit(area, size);
            let img = Rect::from_min_size(place.min, size * place.scale);
            let split_x = img.min.x + img.width() * *split;
            pane(&p, t, place, after, hovered);
            pane(&p.with_clip_rect(Rect::from_min_max(r.min, pos2(split_x, r.max.y))), t, place, before, hovered);
            p.text(pos2(img.min.x, label_y), Align2::LEFT_CENTER, before_label, semibold(11.0), t.text_secondary);
            p.text(pos2(img.max.x, label_y), Align2::RIGHT_CENTER, after_label, semibold(11.0), t.text_secondary);
            // The slider: a line with a knob, dragged (or clicked) anywhere over the images.
            let resp = ui.interact(img, Id::unique("compare_split"), Sense::click_and_drag());
            if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
                *split = ((pos.x - img.min.x) / img.width().max(1.0)).clamp(0.0, 1.0);
            }
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            p.line_segment([pos2(split_x, img.min.y), pos2(split_x, img.max.y)], Stroke::new(2.0, ACCENT));
            let knob = pos2(split_x, img.center().y);
            p.circle_filled(knob, 10.0, ACCENT);
            crate::icons::paint(&p, Rect::from_center_size(knob, vec2(12.0, 12.0)), Icon::DistributeH, Color32::WHITE);
        }
    }
}

/// One version drawn at `place`, its listed layers outlined (the hovered one filled).
fn pane(p: &egui::Painter, t: &Tokens, place: Place, shot: &Shot, hovered: Option<NodeId>) {
    let img = place.image(shot);
    widgets::checker(p, img, 8.0);
    p.image(shot.tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    p.rect_stroke(img, CornerRadius::ZERO, Stroke::new(1.0, t.border), StrokeKind::Outside);
    for (id, kind, b) in &shot.boxes {
        let box_r = Rect::from_two_pos(place.at(shot, b.x0, b.y0), place.at(shot, b.x1, b.y1)).expand(1.0);
        let col = kind_color(t, *kind);
        let hot = hovered == Some(*id);
        if hot {
            p.rect_filled(box_r, CornerRadius::ZERO, col.gamma_multiply(0.18));
        }
        p.rect_stroke(box_r, CornerRadius::ZERO, Stroke::new(if hot { 2.0 } else { 1.0 }, col), StrokeKind::Outside);
    }
}

/// The changed layers with their property changes; a click on a layer selects it.
fn list(ui: &mut Ui, t: &Tokens, cmp: &Comparison, from_version: bool, hovered: &mut Option<NodeId>) -> Option<Action> {
    let mut out = None;
    let summary = if !cmp.snapshot {
        tr("Changes unknown").to_string()
    } else if cmp.changes.is_empty() {
        tr("No changes").to_string()
    } else {
        let parts: Vec<String> = [(cmp.modified, tr_noop("{n} changed")), (cmp.added, tr_noop("{n} added")), (cmp.removed, tr_noop("{n} removed"))]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, key)| trn(key, *n, &[]))
            .collect();
        trn("{n} layers · {parts}", cmp.changes.len(), &[("parts", &parts.join(", "))])
    };
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    ui.painter().text(pos2(r.min.x + 16.0, r.center().y), Align2::LEFT_CENTER, summary, semibold(11.0), t.text);
    let mut now_hovered = None;
    egui::ScrollArea::vertical().id_salt("compare_list").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for c in &cmp.changes {
            let tip = match (c.change, from_version) {
                (ChangeKind::Removed, false) => tr("Removed since it was marked"),
                (ChangeKind::Removed, true) => tr("Removed since this version"),
                _ => tr("Select layer"),
            };
            let resp = change_row(ui, t, c.change, &c.name, c.depth, c.layers, false, tip);
            if resp.hovered() {
                now_hovered = Some(c.id);
            }
            if resp.clicked() && c.change != ChangeKind::Removed {
                out = Some(Action::Select(c.id));
            }
            property_lines(ui, t, c.depth, c.properties.iter().map(|pc| pc.text.as_str()));
        }
        if cmp.truncated {
            widgets::note(ui, &trf("Only the first {n} changed layers are listed.", &[("n", &devcompare::MAX_CHANGES)]), t.text_secondary);
        }
    });
    *hovered = now_hovered;
    out
}

/// One changed layer (or other object) in a list of changes: its name `depth` deep (`layers`: how
/// many layers it is), tagged with the kind of change, `selected` when it is the one shown.
pub(crate) fn change_row(
    ui: &mut Ui,
    t: &Tokens,
    kind: ChangeKind,
    name: &str,
    depth: usize,
    layers: usize,
    selected: bool,
    tip: &str,
) -> egui::Response {
    let indent = 16.0 + (depth.min(6) as f32) * 10.0;
    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let resp = resp.on_hover_text(tip);
    crate::a11y::list_item(&resp, &format!("{} {name}", tr(kind.label())), selected);
    if selected {
        ui.painter().rect_filled(row.shrink2(vec2(8.0, 1.0)), CornerRadius::same(5), t.selected_row);
    } else if resp.hovered() {
        ui.painter().rect_filled(row.shrink2(vec2(8.0, 1.0)), CornerRadius::same(5), t.hover);
    }
    let col = kind_color(t, kind);
    let tag = ui.painter().layout_no_wrap(tr(kind.label()).into(), semibold(9.0), col);
    let tag_r = Rect::from_min_size(pos2(row.max.x - 16.0 - tag.size().x - 10.0, row.center().y - 8.0), vec2(tag.size().x + 10.0, 16.0));
    ui.painter().rect_filled(tag_r, CornerRadius::same(4), col.gamma_multiply(0.15));
    ui.painter().galley(tag_r.center() - tag.size() / 2.0, tag, col);
    ui.painter().circle_filled(pos2(row.min.x + indent + 3.0, row.center().y), 3.0, col);
    let name = if layers > 1 { trf("{name} ({layers} layers)", &[("name", &name), ("layers", &layers)]) } else { name.to_string() };
    let name = widgets::elide(ui, &name, (tag_r.min.x - row.min.x - indent - 20.0).max(20.0));
    let strike = kind == ChangeKind::Removed;
    let galley = ui.painter().layout_no_wrap(name, regular(11.0), if strike { t.text_secondary } else { t.text });
    let at = pos2(row.min.x + indent + 12.0, row.center().y - galley.size().y / 2.0);
    if strike {
        let y = at.y + galley.size().y / 2.0;
        ui.painter().line_segment([pos2(at.x, y), pos2(at.x + galley.size().x, y)], Stroke::new(1.0, t.text_secondary));
    }
    ui.painter().galley(at, galley, t.text);
    resp
}

/// The property changes under a [`change_row`] (`width 120 → 136`).
pub(crate) fn property_lines<'a>(ui: &mut Ui, t: &Tokens, depth: usize, lines: impl Iterator<Item = &'a str>) {
    let indent = 16.0 + (depth.min(6) as f32) * 10.0;
    let mut any = false;
    for text in lines {
        any = true;
        let width = ui.available_width() - indent - 28.0;
        let g = ui.painter().layout(text.to_string(), regular(10.0), t.text_secondary, width.max(40.0));
        let (pr, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y + 4.0), Sense::hover());
        ui.painter().galley(pos2(pr.min.x + indent + 12.0, pr.min.y + 2.0), g, t.text_secondary);
    }
    if any {
        ui.add_space(4.0);
    }
}

/// Resolve and Close, and left of them what it compares with: the snapshot from when it was
/// marked ready for dev (`ready`) or one of the saved `versions` (`version`, the one picked).
fn footer(ui: &mut Ui, t: &Tokens, cmp: &Comparison, ready: bool, versions: &[VersionRow], version: Option<&str>) -> Option<Action> {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_H + 1.0), Sense::hover());
    ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 1.0)), CornerRadius::ZERO, t.border);
    let mut out = None;
    let inner = Rect::from_min_max(pos2(r.min.x + 16.0, r.min.y + 1.0), pos2(r.max.x - 16.0, r.max.y));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if !versions.is_empty() {
            widgets::label(ui, tr("Compare with"), t.text_secondary);
            let picked = version.and_then(|v| versions.iter().find(|(id, ..)| id == v));
            let label = picked.map_or_else(|| tr("When marked ready").to_string(), |(_, name, _)| name.clone());
            let resp = widgets::dropdown(ui, &label, 220.0, Some(Icon::Clock));
            crate::right::menu_popup(&resp).show(|ui| {
                if ready && crate::right::menu_check(ui, version.is_none(), tr("When marked ready"), "") {
                    out = Some(Action::Against(None));
                    ui.close();
                }
                for (id, name, time) in versions {
                    if crate::right::menu_check(ui, version == Some(id.as_str()), name, &crate::comments::ago_long(*time)) {
                        out = Some(Action::Against(Some(id.clone())));
                        ui.close();
                    }
                }
            });
        }
    });
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if version.is_none() {
            let resolve = widgets::button(ui, tr("Resolve change"), true, cmp.changed)
                .on_hover_text(tr("Back to Ready for dev: the next change compares with how it is now"));
            if resolve.clicked() {
                out = Some(Action::Resolve);
            }
        }
        if widgets::button(ui, tr("Close"), false, true).clicked() {
            out = Some(Action::Close);
        }
    });
    out
}
