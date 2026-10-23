//! The left panel: file header, File/Assets tabs, pages and the layers tree.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::json;

use crate::a11y::{self, Region, RowInfo, Tip};
use crate::i18n::{tr, trf, trn};
use crate::icons::{self, Icon};
use crate::theme::{ACCENT, COMPONENT, Tokens, regular, semibold};
use crate::{FigApp, LeftTab};

pub const ROW: f32 = 32.0;

/// A boolean operation's icon.
pub fn op_icon(op: figcraft_doc::BoolOp) -> Icon {
    match op {
        figcraft_doc::BoolOp::Union => Icon::Union,
        figcraft_doc::BoolOp::Subtract => Icon::Subtract,
        figcraft_doc::BoolOp::Intersect => Icon::Intersect,
        figcraft_doc::BoolOp::Exclude => Icon::Exclude,
    }
}

pub fn node_icon(doc: &Document, id: NodeId) -> Icon {
    let Some(n) = doc.node(id) else { return Icon::Rectangle };
    // An operand combining by an operation other than its group's shows that operation.
    if let Some(op) = figcraft_engine::booleans::distinct_op(doc, id) {
        return op_icon(op);
    }
    match &n.kind {
        NodeKind::Page { .. } => Icon::Page,
        NodeKind::Frame if figcraft_engine::components::is_slot(doc, id) => Icon::Slot,
        NodeKind::Frame => {
            if let Some(l) = &n.layout {
                match l.mode {
                    figcraft_doc::LayoutMode::Vertical => Icon::LayoutV,
                    figcraft_doc::LayoutMode::Grid => Icon::LayoutGrid,
                    _ => Icon::LayoutH,
                }
            } else {
                Icon::Frame
            }
        }
        NodeKind::Group => Icon::Group,
        NodeKind::Section { .. } => Icon::Section,
        NodeKind::Rectangle => {
            if n.fills.iter().any(|f| matches!(f.kind, figcraft_color::PaintKind::Image(_))) {
                Icon::Image
            } else {
                Icon::Rectangle
            }
        }
        NodeKind::Ellipse { .. } => Icon::Ellipse,
        NodeKind::Polygon { .. } => Icon::Polygon,
        NodeKind::Star { .. } => Icon::Star,
        NodeKind::Line => {
            if n.stroke.cap_end.is_arrow() {
                Icon::Arrow
            } else {
                Icon::Line
            }
        }
        NodeKind::Vector { .. } => Icon::Vector,
        NodeKind::Text(_) => Icon::Text,
        NodeKind::BooleanOp { op } => op_icon(*op),
        NodeKind::Component { .. } => Icon::Component,
        NodeKind::ComponentSet { .. } => Icon::ComponentSet,
        NodeKind::Instance { .. } => Icon::Instance,
        NodeKind::Slice => Icon::Slice,
    }
}

/// The slim icon rail at the far left: main menu, File, Assets, Variables.
pub fn rail(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let r = ui.max_rect();
    let mut y = r.min.y + 8.0;
    // Main menu (logo).
    let lr = Rect::from_center_size(pos2(r.center().x, y + 16.0), vec2(36.0, 32.0));
    let lresp = ui.interact(lr, ui.id().with("rail_menu"), Sense::click()).tip(tr("Main menu"));
    if lresp.hovered() || app.ui.main_menu {
        ui.painter().rect_filled(lr, CornerRadius::same(6), t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(lr.center(), vec2(18.0, 18.0)), Icon::Logo, t.icon);
    if lresp.clicked() {
        app.ui.main_menu = !app.ui.main_menu;
    }
    if app.ui.main_menu {
        a11y::popup(&mut app.ui.nav, lresp.id, crate::menus::main_menu_id());
    }
    y += 44.0;
    ui.painter().line_segment([pos2(r.min.x + 18.0, y), pos2(r.max.x - 18.0, y)], Stroke::new(1.0, t.border));
    y += 12.0;
    let items =
        [(Icon::Page, tr("File"), Some(LeftTab::File)), (Icon::Assets, tr("Assets"), Some(LeftTab::Assets)), (Icon::Styles, tr("Variables"), None)];
    for (icon, label, tab) in items {
        let ir = Rect::from_center_size(pos2(r.center().x, y + 14.0), vec2(36.0, 28.0));
        let hit = Rect::from_min_max(pos2(r.min.x, y), pos2(r.max.x, y + 48.0));
        let resp = ui.interact(hit, ui.id().with(("rail", label)), Sense::click()).tip(label);
        let active = tab.is_some_and(|tb| tb == app.ui.left_tab && !app.ui.left_collapsed && !app.ui.versions.open);
        if tab.is_some() {
            a11y::tab(&resp, label, active);
        }
        if active {
            ui.painter().rect_filled(ir, CornerRadius::same(6), t.selected_row);
        } else if resp.hovered() {
            ui.painter().rect_filled(ir, CornerRadius::same(6), t.hover);
        }
        icons::paint(ui.painter(), Rect::from_center_size(ir.center(), vec2(16.0, 16.0)), icon, if active { t.accent_text } else { t.icon });
        let shown = crate::widgets::elide_font(ui, label, (r.width() - 6.0).max(0.0), regular(10.0));
        ui.painter().text(pos2(r.center().x, y + 38.0), Align2::CENTER_CENTER, shown, regular(10.0), t.text);
        if resp.clicked() {
            match tab {
                Some(tb) => {
                    // Leaving the version history shows the tab rather than collapsing it.
                    if active {
                        app.ui.left_collapsed = true;
                    } else {
                        app.ui.left_tab = tb;
                        app.ui.left_collapsed = false;
                    }
                    app.ui.versions = Default::default();
                }
                None => {
                    app.ui.variables.get_or_insert_with(Default::default);
                }
            }
        }
        y += 60.0;
    }
}

pub fn left_panel(app: &mut FigApp, ui: &mut Ui) {
    if app.ui.versions.open {
        crate::versions::panel(app, ui);
        return;
    }
    let t = Tokens::get(ui.ctx());
    let full = ui.max_rect();
    // Header: file name (or "Assets") + drafts + collapse.
    let (head, _) = ui.allocate_exact_size(vec2(full.width(), 64.0), Sense::hover());
    if app.ui.left_tab == LeftTab::Assets {
        ui.painter().text(pos2(head.min.x + 16.0, head.min.y + 24.0), Align2::LEFT_CENTER, tr("Assets"), semibold(13.0), t.text);
    } else {
        let name = app.session.doc().map(|d| d.name.clone()).unwrap_or_default();
        let nr = Rect::from_min_max(pos2(head.min.x + 14.0, head.min.y + 14.0), pos2(head.max.x - 48.0, head.min.y + 34.0));
        let start = std::mem::take(&mut app.ui.rename_file);
        if let (Some(new), _) = crate::widgets::rename_field(ui, Id::new("filename"), &name, nr, semibold(13.0), start) {
            app.run("file.rename", json!({"name": new}));
        }
        ui.painter().text(pos2(head.min.x + 16.0, head.min.y + 46.0), Align2::LEFT_CENTER, tr("Drafts"), regular(11.0), t.text_secondary);
    }
    let hr = Rect::from_center_size(pos2(head.max.x - 24.0, head.min.y + 24.0), vec2(24.0, 24.0));
    let hresp = ui.interact(hr, ui.id().with("hideui"), Sense::click()).tip(tr("Collapse panel"));
    if hresp.hovered() {
        ui.painter().rect_filled(hr, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), hr.shrink(5.0), Icon::Panels, t.icon);
    if hresp.clicked() {
        app.ui.left_collapsed = true;
    }
    ui.painter().line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.border));
    match app.ui.left_tab {
        LeftTab::File => {
            pages(app, ui);
            layers(app, ui);
        }
        LeftTab::Assets => crate::assets_panel::show(app, ui),
    }
}

fn pages(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (hr, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    let tresp = ui.interact(Rect::from_min_size(hr.min, vec2(w - 40.0, 40.0)), ui.id().with("pages_toggle"), Sense::click());
    a11y::toggle(&tresp, tr("Pages"), app.ui.pages_open);
    ui.painter().text(pos2(hr.min.x + 16.0, hr.center().y), Align2::LEFT_CENTER, tr("Pages"), semibold(11.0), t.text);
    if tresp.clicked() {
        app.ui.pages_open = !app.ui.pages_open;
    }
    let sr = Rect::from_center_size(pos2(hr.max.x - 50.0, hr.center().y), vec2(24.0, 24.0));
    let sresp = ui.interact(sr, ui.id().with("lsearch"), Sense::click()).tip(tr("Search pages"));
    if sresp.hovered() || app.ui.page_search {
        ui.painter().rect_filled(sr, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), sr.shrink(5.0), Icon::Search, t.icon);
    let search_id = Id::new("page_search_field");
    if sresp.clicked() {
        app.ui.page_search = !app.ui.page_search;
        app.ui.page_filter.clear();
        if app.ui.page_search {
            app.ui.pages_open = true;
        }
    }
    let ar = Rect::from_center_size(pos2(hr.max.x - 22.0, hr.center().y), vec2(24.0, 24.0));
    let aresp = ui.interact(ar, ui.id().with("addpage"), Sense::click()).tip(tr("Add new page"));
    if aresp.hovered() {
        ui.painter().rect_filled(ar, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), ar.shrink(5.0), Icon::Plus, t.icon);
    if aresp.clicked() {
        app.ui.pages_focus = app.session.ds().ok().map(|d| d.uid);
        app.run("page.add", json!({}));
    }
    if !app.ui.pages_open {
        return;
    }
    if app.ui.page_search {
        search_field(ui, &t, w, search_id, tr("Search pages"), &mut app.ui.page_filter, &mut app.ui.page_search, sresp.clicked());
    }
    let Ok(ds) = app.session.ds() else { return };
    let pages: Vec<(NodeId, String, bool)> =
        ds.doc.pages.iter().filter_map(|p| ds.doc.node(*p).map(|n| (*p, n.name.clone(), ds.doc.is_divider(*p)))).collect();
    let filter = app.ui.page_filter.trim().to_lowercase();
    let filtering = !filter.is_empty();
    let cur = ds.page;
    let uid = ds.uid;
    let sel = ds.selection.clone();
    let mut select = None;
    let mut rename_done: Option<(NodeId, String)> = None;
    // Layers dragged from the list onto another page move there.
    let dragging: Option<NodeId> = ui.data(|d| d.get_temp(layer_drag_id()));
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let mut move_to: Option<(NodeId, String)> = None;
    // Pages are dragged up and down the list to reorder them.
    let mut reorder = crate::list_drag::ListDrag::in_order(ui, "pages");
    let page_key = (uid, cur);
    let scroll_to_cur = app.ui.pages_seen_page != Some(page_key);
    let mut current_row_rendered = false;
    // A long list scrolls in its own area, so the layers keep their room below it.
    let max_h = pages_list_height(app.ui.pages_height, ui.available_height());
    let mut moved = egui::ScrollArea::vertical().id_salt("pages_list").max_height(max_h).show(ui, |ui| {
        for (k, (id, name, divider)) in pages.iter().cloned().enumerate() {
            if filtering && (divider || !name.to_lowercase().contains(&filter)) {
                continue;
            }
            let sense = if filtering { Sense::click() } else { Sense::click_and_drag() };
            let (r, resp) = ui.allocate_exact_size(vec2(w, 32.0), sense);
            if id == cur {
                current_row_rendered = true;
                if scroll_to_cur {
                    ui.scroll_to_rect(r, None);
                }
            }
            if !filtering {
                reorder.row_by(&resp, k, r, || if divider { tr("Divider").to_string() } else { name.clone() });
            }
            a11y::list_item(&resp, if divider { tr("Divider") } else { &name }, id == cur);
            let r = r.shrink2(vec2(8.0, 0.0));
            a11y::ring_around(ui, r, &resp);
            let renaming = app.ui.rename_page.as_ref().is_some_and(|(i, _)| *i == id);
            if divider && !renaming {
                // A divider is a line between pages: it opens nothing and takes no layers.
                ui.painter().line_segment([pos2(r.min.x + 10.0, r.center().y), pos2(r.max.x - 10.0, r.center().y)], Stroke::new(1.0, t.border));
            } else if resp.hovered() || id == cur {
                ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
            }
            if dragging.is_some()
                && id != cur
                && !divider
                // The gap between rows counts too.
                && pointer.is_some_and(|p| resp.interact_rect.expand2(vec2(0.0, 3.0)).contains(p))
            {
                ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
                if ui.input(|i| i.pointer.any_released()) {
                    move_to = Some((id, name.clone()));
                }
            }
            if renaming {
                let tr = Rect::from_min_max(pos2(r.min.x + 8.0, r.min.y + 4.0), pos2(r.max.x - 4.0, r.max.y - 4.0));
                let start = ui.data_mut(|d| d.remove_temp::<bool>(page_rename_start_id())).unwrap_or(false);
                let font = if id == cur { semibold(11.0) } else { regular(11.0) };
                if let (Some(new), _) = crate::widgets::rename_field(ui, Id::new(("pagename", id)), &name, tr, font, start) {
                    rename_done = Some((id, new));
                }
            } else if !divider {
                let font = if id == cur { semibold(11.0) } else { regular(11.0) };
                crate::bidi::paint_name(ui.painter(), pos2(r.min.x + 10.0, r.center().y), &name, font, t.text, r.width() - 14.0);
            }
            if resp.clicked() && !divider {
                select = Some(id);
                app.ui.pages_focus = Some(uid);
                app.ui.layers_focus = false;
            }
            if resp.double_clicked() {
                app.ui.rename_page = Some((id, name.clone()));
                // The field takes the focus next frame, once it's drawn: focusing it now would put a
                // node that isn't there yet in the screen reader tree, which AccessKit aborts on.
                ui.data_mut(|d| d.insert_temp(page_rename_start_id(), true));
            }
            crate::right::context_popup(&resp).show(|ui| {
                if ui.button(tr("Rename page")).clicked() {
                    app.ui.rename_page = Some((id, name.clone()));
                    ui.close();
                }
                if ui.button(tr("Duplicate page")).clicked() {
                    app.run("page.duplicate", json!({"id": id}));
                    ui.close();
                }
                if ui.button(tr("Delete page")).clicked() {
                    delete_page(app, id);
                    ui.close();
                }
            });
        }
        reorder.finish(ui)
    });
    if current_row_rendered {
        app.ui.pages_seen_page = Some(page_key);
    }
    // A drop outside the list's visible area (and the row gap at its edges) isn't on a page row,
    // even one scrolled out of view.
    if !ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| moved.inner_rect.expand2(vec2(0.0, 3.0)).contains(p)) {
        moved.inner = None;
        move_to = None;
    }
    if let Some((from, to)) = moved.inner
        && let Some((id, ..)) = pages.get(from)
    {
        app.run("page.move", json!({"id": id, "index": to}));
    }
    if let Some((id, new)) = rename_done {
        app.ui.rename_page = None;
        app.run("page.rename", json!({"id": id, "name": new}));
    }
    if let Some((page, name)) = move_to {
        let ids = dragged_layers(dragging, &sel);
        if app.run("object.moveToPage", json!({"ids": ids, "page": page})).is_some() {
            app.toast(trf("Moved to {name}", &[("name", &name)]));
        }
    }
    if let Some(p) = select
        && p != cur
    {
        app.run("page.select", json!({"id": p}));
    }
    // `inner_rect` is the room the list was given; a short list shrinks to its rows.
    pages_edge(app, ui, moved.inner_rect.height().min(moved.content_size.y));
}

/// The Pages list has the keyboard: a page row (or the add-page button) of the open file was
/// clicked last, and the list is still on screen.
pub(crate) fn pages_have_keyboard(app: &FigApp) -> bool {
    let Ok(ds) = app.session.ds() else { return false };
    app.ui.pages_focus == Some(ds.uid) && app.ui.pages_open && !app.ui.left_collapsed && !ds.view.ui_hidden && app.ui.left_tab == LeftTab::File
}

/// Delete `page` (no confirmation: Undo brings it back) and say so.
pub(crate) fn delete_page(app: &mut FigApp, page: NodeId) {
    if app.run("page.delete", json!({"id": page})).is_some() {
        app.toast(tr("Page deleted"));
    }
}

/// The pages list's height in a panel with `avail` below the Pages header: the height asked for
/// (by default 40% of it), at least three rows, and leaving the layers their header and three rows.
pub(crate) fn pages_list_height(asked: Option<f32>, avail: f32) -> f32 {
    let least = 3.0 * ROW;
    let most = (avail - 40.0 - 3.0 * ROW).max(least);
    asked.filter(|h| h.is_finite()).unwrap_or(avail * 0.4).clamp(least, most)
}

/// The line between the pages and the layers: dragging it sizes the pages list (`shown`: its
/// height now), a double-click puts back the default. The main window keeps it in the preferences.
fn pages_edge(app: &mut FigApp, ui: &mut Ui, shown: f32) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (line, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
    let grab = ui.style().interaction.resize_grab_radius_side;
    let id = Id::new("pages_edge");
    let resp = ui.interact(line.expand2(vec2(0.0, grab)), id, Sense::click_and_drag());
    let active = resp.hovered() || resp.dragged();
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let (width, colour) = if active { (2.0, ACCENT) } else { (1.0, t.border) };
    ui.painter().line_segment([line.left_center(), line.right_center()], Stroke::new(width, colour));
    let asked = if resp.double_clicked() {
        Some(None)
    } else if resp.dragged() {
        // The edge stays under the pointer where it was grabbed.
        if resp.drag_started() {
            ui.data_mut(|d| d.insert_temp(id, shown));
        }
        let from: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(shown);
        let moved = ui.input(|i| Some(i.pointer.latest_pos()?.y - i.pointer.press_origin()?.y)).unwrap_or(0.0);
        Some(Some(from + moved))
    } else {
        None
    };
    if let Some(h) = asked
        && h != app.ui.pages_height
    {
        app.ui.pages_height = h;
        if app.windows.current == 0 {
            app.session.prefs.pages_height = h;
            app.panels_unsaved = true;
        }
    }
}

/// A page rename just asked for by a double click: its field takes the focus when drawn (temp data).
fn page_rename_start_id() -> Id {
    Id::new("page_rename_start")
}

/// The layer row being dragged in the panel (temp data).
fn layer_drag_id() -> Id {
    Id::new("layer_drag")
}

/// A press on a row's eye or padlock dragged across other rows: every row passed gets the state the
/// first one switched to, in one step on release (temp data).
#[derive(Clone)]
struct Sweep {
    lock: bool,
    /// Hidden for the eye, locked for the padlock.
    on: bool,
    ids: Vec<NodeId>,
}

fn sweep_id() -> Id {
    Id::new("layer_toggle_sweep")
}

/// What a drag of row `dragging` moves: the selection when the row is in it, else the row.
fn dragged_layers(dragging: Option<NodeId>, sel: &[NodeId]) -> Vec<NodeId> {
    match dragging {
        Some(dg) if sel.contains(&dg) => sel.to_vec(),
        Some(dg) => vec![dg],
        None => Vec::new(),
    }
}

#[derive(Clone)]
pub(crate) struct Row {
    pub(crate) id: NodeId,
    depth: usize,
    has_kids: bool,
    /// Not shown on the canvas because of an ancestor (hidden, or a section hiding its contents).
    dimmed: bool,
    /// Locked because an ancestor is.
    locked_above: bool,
}

/// The layer rows of the last frame and what they were built from, so a frame where neither the
/// document, the page, the expanded set nor the filter changed doesn't walk the tree again.
#[derive(Clone, Default)]
struct RowCache {
    doc: Option<Arc<Document>>,
    page: Option<NodeId>,
    expanded: u64,
    filter: String,
    rows: Arc<Vec<Row>>,
}

fn hash_expanded(expanded: &BTreeSet<NodeId>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    expanded.hash(&mut h);
    h.finish()
}

/// The rows to list: the expanded tree, or the layers matching `filter` (lowercase).
pub(crate) fn rows_for(ui: &Ui, doc: &Arc<Document>, page: NodeId, expanded: &BTreeSet<NodeId>, filter: &str) -> Arc<Vec<Row>> {
    let ex = if filter.is_empty() { hash_expanded(expanded) } else { 0 };
    ui.data_mut(|d| {
        let c = d.get_temp_mut_or_default::<RowCache>(Id::new("layer_rows"));
        let fresh = c.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, doc)) && c.page == Some(page) && c.expanded == ex && c.filter == filter;
        if !fresh {
            let rows = listed_rows(doc, page, expanded, filter);
            *c = RowCache { doc: Some(doc.clone()), page: Some(page), expanded: ex, filter: filter.to_string(), rows: Arc::new(rows) };
        }
        c.rows.clone()
    })
}

/// The rows to list, uncached: the expanded tree, or the layers matching `filter` (lowercase).
fn listed_rows(doc: &Document, page: NodeId, expanded: &BTreeSet<NodeId>, filter: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    if filter.is_empty() {
        collect(doc, page, 0, false, false, expanded, &mut rows);
    } else {
        matching(doc, page, filter, &mut rows);
    }
    rows
}

/// Whether the children of `n` don't show on the canvas (given whether `n` itself doesn't).
fn hides_children(n: &figcraft_doc::Node, dimmed: bool) -> bool {
    dimmed || !n.visible || n.hides_contents()
}

fn collect(doc: &Document, id: NodeId, depth: usize, dimmed: bool, locked: bool, expanded: &BTreeSet<NodeId>, out: &mut Vec<Row>) {
    let Some(n) = doc.node(id) else { return };
    let dim_kids = !n.is_page() && hides_children(n, dimmed);
    let lock_kids = locked || n.locked;
    // Layers panel lists top of the stack first.
    for c in n.children.iter().rev() {
        let Some(cn) = doc.node(*c) else { continue };
        let has_kids = !cn.children.is_empty() || matches!(cn.kind, NodeKind::BooleanOp { .. });
        out.push(Row { id: *c, depth, has_kids, dimmed: dim_kids, locked_above: lock_kids });
        if expanded.contains(c) && depth < 64 {
            collect(doc, *c, depth + 1, dim_kids, lock_kids, expanded, out);
        }
    }
}

/// Layers whose name contains `q` (lowercase), top of the stack first, as a flat list.
fn matching(doc: &Document, id: NodeId, q: &str, out: &mut Vec<Row>) {
    let Some(n) = doc.node(id) else { return };
    for c in n.children.iter().rev() {
        let Some(cn) = doc.node(*c) else { continue };
        let name = if cn.name.is_empty() { cn.kind.label() } else { cn.name.as_str() };
        if name.to_lowercase().contains(q) {
            let above: Vec<&figcraft_doc::Node> = doc.ancestors(*c).iter().filter_map(|a| doc.node(*a)).filter(|a| !a.is_page()).collect();
            let dimmed = above.iter().any(|a| hides_children(a, false));
            out.push(Row { id: *c, depth: 0, has_kids: false, dimmed, locked_above: above.iter().any(|a| a.locked) });
        }
        if out.len() < 5000 {
            matching(doc, *c, q, out);
        }
    }
}

/// Arrow keys while the layers panel has the keyboard: Up/Down select the row above or below
/// (Shift extends the range from the anchor), Left collapses a row or goes to its parent, Right
/// expands a row or goes to its first child. Returns whether the key was used.
pub(crate) fn key_nav(app: &mut FigApp, key: egui::Key, shift: bool) -> bool {
    use egui::Key;
    if !matches!(key, Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight) {
        return false;
    }
    let Ok(ds) = app.session.ds() else { return false };
    let (doc, page, sel) = (ds.doc.clone(), ds.page, ds.selection.clone());
    // The rows as listed: the expanded tree, or the search results.
    let filter = app.ui.layer_filter.trim().to_lowercase();
    let rows = listed_rows(&doc, page, &app.ui.expanded, &filter);
    let select = |app: &mut FigApp, ids: Vec<NodeId>, anchor: NodeId| {
        app.ui.layer_anchor = Some(anchor);
        app.run("select.set", json!({"ids": ids}));
    };
    let Some((i, row)) = sel.last().and_then(|c| rows.iter().enumerate().find(|(_, r)| r.id == *c)) else {
        // Nothing listed is selected: Down starts at the top, Up at the bottom.
        let first = if key == Key::ArrowUp { rows.last() } else { rows.first() };
        if let Some(r) = first.filter(|_| matches!(key, Key::ArrowUp | Key::ArrowDown)) {
            select(app, vec![r.id], r.id);
        }
        return true;
    };
    let (cur, depth, has_kids) = (row.id, row.depth, row.has_kids);
    match key {
        Key::ArrowUp | Key::ArrowDown => {
            let j = if key == Key::ArrowUp { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < rows.len()) };
            let Some((j, next)) = j.and_then(|j| rows.get(j).map(|r| (j, r.id))) else { return true };
            if shift {
                // The rows from the anchor to the new one, with the moving end last.
                let anchor = app.ui.layer_anchor.filter(|a| sel.contains(a)).unwrap_or(cur);
                let a = rows.iter().position(|r| r.id == anchor).unwrap_or(i);
                let ids: Vec<NodeId> = if a <= j {
                    rows.get(a..=j).unwrap_or_default().iter().map(|r| r.id).collect()
                } else {
                    rows.get(j..=a).unwrap_or_default().iter().rev().map(|r| r.id).collect()
                };
                select(app, ids, anchor);
            } else {
                select(app, vec![next], next);
            }
        }
        Key::ArrowLeft => {
            if has_kids && filter.is_empty() && app.ui.expanded.contains(&cur) {
                app.ui.expanded.remove(&cur);
            } else if let Some(p) = doc.node(cur).and_then(|n| n.parent).filter(|p| *p != page) {
                select(app, vec![p], p);
            }
        }
        _ => {
            if has_kids && filter.is_empty() {
                if !app.ui.expanded.contains(&cur) {
                    app.ui.expanded.insert(cur);
                } else if let Some(c) = rows.get(i + 1).filter(|r| r.depth == depth + 1).map(|r| r.id) {
                    select(app, vec![c], c);
                }
            }
        }
    }
    true
}

/// A layer being renamed in the layers panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rename {
    pub id: NodeId,
    /// The rename just started: its field takes the keyboard (and its row scrolls into view) the
    /// next time the panel draws.
    pub start: bool,
}

/// Start renaming a layer in the layers panel (Cmd/Ctrl+R): the topmost listed of `ids`, which
/// becomes the selection. Shows the panel and opens the rows above the layer.
pub(crate) fn start_rename(app: &mut FigApp, ids: &[NodeId]) {
    let Ok(ds) = app.session.ds() else { return };
    let (doc, page) = (ds.doc.clone(), ds.page);
    for a in ids.iter().flat_map(|i| doc.ancestors(*i)).filter(|a| *a != page) {
        app.ui.expanded.insert(a);
    }
    let rows = listed_rows(&doc, page, &app.ui.expanded, &app.ui.layer_filter.trim().to_lowercase());
    let Some(id) = rows.iter().map(|r| r.id).find(|r| ids.contains(r)) else { return };
    app.ui.left_tab = LeftTab::File;
    app.ui.left_collapsed = false;
    rename_layer(app, id);
}

/// Collapse layers: every row closes except those leading to the selection.
pub(crate) fn collapse_all(app: &mut FigApp) {
    app.ui.expanded.clear();
    let Ok(ds) = app.session.ds() else { return };
    app.ui.expanded.extend(ds.selection.iter().flat_map(|s| ds.doc.ancestors(*s)).filter(|a| *a != ds.page));
}

/// Rename `id` in its row, selected (the selection follows a rename that moves with Tab).
fn rename_layer(app: &mut FigApp, id: NodeId) {
    app.ui.rename = Some(Rename { id, start: true });
    if app.session.selection() != [id] {
        app.run("select.set", json!({"ids": [id]}));
    }
    app.ui.layer_anchor = Some(id);
    // The rename scrolls its row into view itself.
    app.ui.layers_seen_sel = app.session.selection();
}

fn rename_id(id: NodeId) -> Id {
    Id::new(("rename", id))
}

/// Left edge of a row's contents (disclosure triangle) in the row's inner rect `rr`.
fn row_x0(rr: Rect, depth: usize) -> f32 {
    rr.min.x + 4.0 + depth as f32 * 16.0
}

/// The name field of the row being renamed (row rect `r`): the new name once committed. The
/// rename ends when the field lets go of the keyboard.
fn rename_field(app: &mut FigApp, ui: &mut Ui, id: NodeId, depth: usize, name: &str, r: Rect) -> Option<String> {
    let rr = r.shrink2(vec2(8.0, 0.0));
    let rect = Rect::from_min_max(pos2(row_x0(rr, depth) + 32.0, r.min.y + 4.0), pos2(rr.max.x - 4.0, r.max.y - 4.0));
    let start = app.ui.rename.is_some_and(|x| x.start);
    let (out, open) = crate::widgets::rename_field(ui, rename_id(id), name, rect, regular(11.0), start);
    app.ui.rename = open.then_some(Rename { id, start: false });
    out
}

/// Expand or collapse `id` and everything inside it (Alt-click on a disclosure triangle).
fn set_expanded_deep(doc: &Document, id: NodeId, open: bool, expanded: &mut BTreeSet<NodeId>) {
    let mut stack = vec![id];
    while let Some(n) = stack.pop() {
        if open {
            expanded.insert(n);
        } else {
            expanded.remove(&n);
        }
        if let Some(node) = doc.node(n) {
            stack.extend(node.children.iter().copied());
        }
    }
}

/// The search row under the Pages or Layers header: a live filter `text`, taking the keyboard when
/// `focus` (just opened). Escape in it clears it and closes the row (`open`), and goes no further
/// (the canvas keeps its selection).
fn search_field(ui: &mut Ui, t: &Tokens, w: f32, id: Id, hint: &str, text: &mut String, open: &mut bool, focus: bool) {
    let (fr, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
    let field = Rect::from_min_max(pos2(fr.min.x + 12.0, fr.min.y + 2.0), pos2(fr.max.x - 12.0, fr.max.y - 6.0));
    ui.painter().rect_filled(field, CornerRadius::same(5), t.field);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(field.min.x + 12.0, field.center().y), vec2(12.0, 12.0)), Icon::Search, t.icon_secondary);
    let te_rect = Rect::from_min_max(pos2(field.min.x + 24.0, field.center().y - 7.0), pos2(field.max.x - 6.0, field.center().y + 7.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(te_rect));
    let r = child.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(regular(11.0))
            .hint_text(hint)
            .vertical_align(egui::Align::Center)
            .desired_width(te_rect.width()),
    );
    // After the field: the click that opened it ends elsewhere, which would take the focus back.
    if focus {
        r.request_focus();
    }
    crate::widgets::select_all_on_focus(ui, &r, text);
    if r.has_focus() {
        ui.painter().rect_stroke(field, CornerRadius::same(5), Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
    }
    if (r.has_focus() || r.lost_focus()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        *open = false;
        text.clear();
    }
}

fn layers(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (hr, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    ui.painter().text(pos2(hr.min.x + 16.0, hr.center().y), Align2::LEFT_CENTER, tr("Layers"), semibold(11.0), t.text);
    let sr = Rect::from_center_size(pos2(hr.max.x - 22.0, hr.center().y), vec2(24.0, 24.0));
    let sresp = ui.interact(sr, ui.id().with("layer_search_btn"), Sense::click()).tip(tr("Search layers"));
    if sresp.hovered() || app.ui.layer_search {
        ui.painter().rect_filled(sr, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), sr.shrink(5.0), Icon::Search, t.icon);
    let search_id = Id::new("layer_search_field");
    if sresp.clicked() {
        app.ui.layer_search = !app.ui.layer_search;
        app.ui.layer_filter.clear();
    }
    if app.ui.layer_search {
        search_field(ui, &t, w, search_id, tr("Find layers"), &mut app.ui.layer_filter, &mut app.ui.layer_search, sresp.clicked());
    }
    let Ok(ds) = app.session.ds() else { return };
    let doc = ds.doc.clone();
    let page = ds.page;
    let sel = ds.selection.clone();
    let hover = ds.hover;
    // Reveal a new selection: expand its ancestors and scroll its row into view, once, so
    // "Collapse layers" and manual collapsing stick.
    let reveal = sel != app.ui.layers_seen_sel;
    if reveal {
        for s in &sel {
            for a in doc.ancestors(*s) {
                if a != page {
                    app.ui.expanded.insert(a);
                }
            }
        }
        app.ui.layers_seen_sel = sel.clone();
    }
    let filter = app.ui.layer_filter.trim().to_lowercase();
    let rows = rows_for(ui, &doc, page, &app.ui.expanded, &filter);
    let mut scroll_to = if reveal { sel.first().and_then(|s| rows.iter().position(|r| r.id == *s)) } else { None };
    // A rename in progress ends when its layer isn't listed any more; Tab / Shift+Tab commit it and
    // go on to the next / previous row.
    let mut rename_row: Option<(usize, NodeId, usize)> = None;
    let mut rename_next: Option<NodeId> = None;
    let renaming = app.ui.rename.is_some();
    if let Some(rn) = app.ui.rename {
        match rows.iter().position(|r| r.id == rn.id) {
            None => app.ui.rename = None,
            Some(i) => {
                if rn.start {
                    scroll_to = Some(i);
                } else if let Some(back) = crate::widgets::take_tab(ui) {
                    ui.memory_mut(|m| m.surrender_focus(rename_id(rn.id).with("te")));
                    rename_next = if back { i.checked_sub(1) } else { Some(i + 1) }.and_then(|j| rows.get(j)).map(|r| r.id);
                }
                rename_row = rows.get(i).map(|r| (i, r.id, r.depth));
            }
        }
    }
    let mut clicked: Option<(NodeId, egui::Modifiers)> = None;
    let mut toggle: Option<NodeId> = None;
    let mut deep: Option<(NodeId, bool)> = None;
    let mut vis: Option<NodeId> = None;
    let mut lock: Option<NodeId> = None;
    let mut rename_done: Option<(NodeId, String)> = None;
    let mut hovered_row: Option<NodeId> = None;
    let drag_id = layer_drag_id();
    // Escape cancels a drag.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|d| d.remove::<NodeId>(drag_id));
    }
    let dragging: Option<NodeId> = ui.data(|d| d.get_temp(drag_id));
    let mut sweep: Option<Sweep> = ui.data(|d| d.get_temp(sweep_id()));
    let moving = dragged_layers(dragging, &sel);
    // A layer can't go into itself or its own descendants.
    let blocked = |id: NodeId| moving.contains(&id) || doc.ancestors(id).iter().any(|a| moving.contains(a));
    let mut drop: Option<(NodeId, i8)> = None;
    // The drag is over a row that takes no drop: the not-allowed cursor shows.
    let mut refused = false;
    // Only the rows in view are laid out and drawn; the rest is empty scroll space.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let pitch = ROW + ui.spacing().item_spacing.y;
        let top = ui.max_rect().top();
        let total = rows.len() as f32 * pitch;
        ui.set_height(total.max(viewport.height()));
        if let Some(i) = scroll_to {
            ui.scroll_to_rect(Rect::from_min_size(pos2(ui.max_rect().left(), top + i as f32 * pitch), vec2(w, ROW)), None);
        }
        if rows.is_empty() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                let hint = if filter.is_empty() { tr("Add a frame (F) or a shape (R) to get started") } else { tr("No matching layers") };
                ui.add(egui::Label::new(egui::RichText::new(hint).font(regular(11.0)).color(t.text_secondary)).wrap());
            });
        }
        // Dragging near the list's top or bottom edge scrolls it, faster closer to the edge.
        if dragging.is_some()
            && let Some(p) = ui.input(|i| i.pointer.hover_pos())
        {
            const EDGE: f32 = 28.0;
            let view = ui.clip_rect();
            let dy = if p.y < view.min.y + EDGE {
                (view.min.y + EDGE - p.y).min(EDGE)
            } else if p.y > view.max.y - EDGE {
                -(p.y - (view.max.y - EDGE)).min(EDGE)
            } else {
                0.0
            };
            if dy != 0.0 && view.x_range().contains(p.x) {
                ui.scroll_with_delta(vec2(0.0, dy * 0.5));
                ui.ctx().request_repaint();
            }
        }
        let first = ((viewport.min.y / pitch).floor().max(0.0) as usize).min(rows.len());
        let last = ((viewport.max.y / pitch).ceil().max(0.0) as usize).saturating_add(1).min(rows.len());
        let band = Rect::from_x_y_ranges(ui.max_rect().x_range(), (top + first as f32 * pitch)..=(top + last as f32 * pitch));
        let mut band_ui = ui.new_child(egui::UiBuilder::new().max_rect(band));
        // Stable widget ids for a row wherever the list is scrolled.
        band_ui.skip_ahead_auto_ids(first);
        let ui = &mut band_ui;
        for row in rows.iter().take(last).skip(first) {
            let Some(n) = doc.node(row.id) else { continue };
            let (r, resp) = ui.allocate_exact_size(vec2(w, ROW), Sense::click_and_drag());
            let selected = sel.contains(&row.id);
            a11y::layer_row(&mut app.ui.nav, &resp);
            if sel.last() == Some(&row.id) {
                a11y::prefer(&mut app.ui.nav, Region::Layers, resp.id);
            }
            let info = RowInfo {
                kind: tr(n.kind.label()),
                depth: row.depth,
                selected,
                expanded: row.has_kids.then(|| app.ui.expanded.contains(&row.id)),
                hidden: !n.visible,
                locked: n.locked,
            };
            a11y::tree_item(&resp, if n.name.is_empty() { tr(n.kind.label()) } else { &n.name }, &info);
            let in_selected = !selected && doc.ancestors(row.id).iter().any(|a| sel.contains(a));
            let is_comp = matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. });
            let rr = r.shrink2(vec2(8.0, 0.0));
            a11y::ring_around(ui, rr, &resp);
            // Over the row, its eye and padlock included (they sit on top of it).
            let over = resp.contains_pointer();
            if selected {
                ui.painter().rect_filled(rr, CornerRadius::same(5), if is_comp { t.component_row } else { t.selected_row });
            } else if in_selected {
                ui.painter().rect_filled(rr, CornerRadius::ZERO, t.selected_row_child);
            } else if over || hover == Some(row.id) {
                ui.painter().rect_stroke(
                    rr,
                    CornerRadius::same(5),
                    Stroke::new(1.0, if is_comp { COMPONENT } else { ACCENT }),
                    egui::StrokeKind::Inside,
                );
            }
            if over {
                hovered_row = Some(row.id);
            }
            let x0 = row_x0(rr, row.depth);
            // Expand triangle.
            if row.has_kids {
                let tri = Rect::from_center_size(pos2(x0 + 6.0, r.center().y), vec2(14.0, 14.0));
                let tresp = ui.interact(tri, ui.id().with(("tri", row.id)), Sense::click());
                let open = app.ui.expanded.contains(&row.id);
                a11y::button(&tresp, if open { tr("Collapse") } else { tr("Expand") });
                icons::paint(ui.painter(), tri.shrink(1.0), if open { Icon::Chevron } else { Icon::ChevronRight }, t.icon_secondary);
                if tresp.clicked() {
                    if ui.input(|i| i.modifiers.alt) {
                        deep = Some((row.id, !open));
                    } else {
                        toggle = Some(row.id);
                    }
                }
            }
            let icon = node_icon(&doc, row.id);
            let shown = n.visible && !row.dimmed;
            let icon_col = if is_comp || icon == Icon::Slot {
                COMPONENT.gamma_multiply(if shown { 1.0 } else { 0.5 })
            } else if shown {
                t.icon
            } else {
                t.text_tertiary
            };
            icons::paint(ui.painter(), Rect::from_center_size(pos2(x0 + 22.0, r.center().y), vec2(14.0, 14.0)), icon, icon_col);
            let name_x = x0 + 36.0;
            if app.ui.rename.is_some_and(|rn| rn.id == row.id) {
                if let Some(new) = rename_field(app, ui, row.id, row.depth, &n.name, r) {
                    rename_done = Some((row.id, new));
                }
            } else {
                let col = if is_comp {
                    COMPONENT.gamma_multiply(if shown { 1.0 } else { 0.5 })
                } else if shown {
                    t.text
                } else {
                    t.text_tertiary
                };
                let font = if doc.node(n.parent.unwrap_or(page)).is_some_and(|p| p.is_page())
                    && (n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. }))
                {
                    semibold(11.0)
                } else {
                    regular(11.0)
                };
                let name = if n.name.is_empty() { tr(n.kind.label()).to_string() } else { n.name.clone() };
                // Room left of the lock and eye buttons.
                let max_w = (rr.max.x - 56.0 - name_x).max(20.0);
                crate::bidi::paint_name(ui.painter(), pos2(name_x, r.center().y), &name, font, col, max_w);
            }
            // A sweep across the eyes or padlocks takes in the rows the pointer passes.
            if let Some(sw) = sweep.as_mut()
                && !sw.ids.contains(&row.id)
                && ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| r.y_range().contains(p.y))
            {
                sw.ids.push(row.id);
            }
            let swept = |lock: bool| sweep.as_ref().filter(|sw| sw.lock == lock && sw.ids.contains(&row.id)).map(|sw| sw.on);
            // Lock / eye on hover (or when set); a layer in a locked one shows a faint padlock.
            let hidden = swept(false).unwrap_or(!n.visible);
            let locked = swept(true).unwrap_or(n.locked);
            let show_lock = over || locked || row.locked_above;
            let show_eye = over || hidden;
            let er = Rect::from_center_size(pos2(rr.max.x - 14.0, r.center().y), vec2(20.0, 20.0));
            let lr = Rect::from_center_size(pos2(rr.max.x - 36.0, r.center().y), vec2(20.0, 20.0));
            if show_eye {
                let eresp = ui.interact(er, ui.id().with(("eye", row.id)), Sense::click_and_drag());
                a11y::toggle(&eresp, tr("Hidden"), hidden);
                icons::paint(ui.painter(), er.shrink(3.0), if hidden { Icon::EyeOff } else { Icon::Eye }, t.icon_secondary);
                if eresp.clicked() {
                    vis = Some(row.id);
                }
                if eresp.drag_started() {
                    sweep = Some(Sweep { lock: false, on: n.visible, ids: vec![row.id] });
                }
            }
            if show_lock {
                let lresp = ui.interact(lr, ui.id().with(("lock", row.id)), Sense::click_and_drag());
                a11y::toggle(&lresp, tr("Locked"), locked || row.locked_above);
                let col = if locked || !row.locked_above { t.icon_secondary } else { t.text_tertiary };
                icons::paint(ui.painter(), lr.shrink(3.0), if locked || row.locked_above { Icon::Lock } else { Icon::Unlock }, col);
                if lresp.clicked() {
                    lock = Some(row.id);
                }
                if lresp.drag_started() {
                    sweep = Some(Sweep { lock: true, on: !n.locked, ids: vec![row.id] });
                }
            }
            if resp.clicked() {
                clicked = Some((row.id, ui.input(|i| i.modifiers)));
            }
            if resp.double_clicked() {
                app.ui.rename = Some(Rename { id: row.id, start: true });
            }
            if resp.secondary_clicked() {
                if !sel.contains(&row.id) {
                    clicked = Some((row.id, egui::Modifiers::NONE));
                }
                let pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(r.center());
                app.ui.context_menu = Some((pos, vec![row.id]));
            }
            // Drag to reorder.
            if resp.drag_started() {
                ui.data_mut(|d| d.insert_temp(drag_id, row.id));
            }
            let over = dragging.is_some() && ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| r.contains(p));
            // What is inside the dragged layers, and inside an instance, takes no drop.
            refused |= over
                && (doc.ancestors(row.id).iter().any(|a| moving.contains(a))
                    || n.parent.is_some_and(|p| figcraft_engine::components::structure_locked(&doc, p)));
            if dragging.is_some()
                && !blocked(row.id)
                && let Some(p) = ui.input(|i| i.pointer.hover_pos())
                && r.contains(p)
            {
                let rel = (p.y - r.min.y) / r.height();
                let into = n.kind.is_container() && !matches!(n.kind, NodeKind::Instance { .. }) && (0.25..0.75).contains(&rel);
                let zone: i8 = if into {
                    0
                } else if rel < 0.5 {
                    -1
                } else {
                    1
                };
                let y = match zone {
                    -1 => r.min.y,
                    1 => r.max.y,
                    _ => r.center().y,
                };
                if zone == 0 {
                    ui.painter().rect_stroke(rr, CornerRadius::same(5), Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
                    // Holding over a collapsed group or frame opens it, to drop deeper inside.
                    if row.has_kids && !app.ui.expanded.contains(&row.id) {
                        let hold_id = Id::new("layer_drag_hold");
                        let now = ui.input(|i| i.time);
                        match ui.data(|d| d.get_temp::<(NodeId, f64)>(hold_id)) {
                            Some((id, since)) if id == row.id => {
                                if now - since > 0.6 {
                                    toggle = Some(row.id);
                                    ui.data_mut(|d| d.remove::<(NodeId, f64)>(hold_id));
                                }
                            }
                            _ => {
                                ui.data_mut(|d| d.insert_temp(hold_id, (row.id, now)));
                            }
                        }
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                    }
                } else {
                    crate::widgets::insertion_line(ui.painter(), x0, rr.max.x, y);
                }
                if ui.input(|i| i.pointer.any_released()) {
                    drop = Some((row.id, zone));
                }
            }
        }
        // The field of a rename scrolled out of view stays (unseen), so the rename goes on.
        if let Some((i, id, depth)) = rename_row.filter(|(i, ..)| !(first..last).contains(i))
            && let Some(n) = doc.node(id)
        {
            let r = Rect::from_min_size(pos2(band.min.x, top + i as f32 * pitch), vec2(w, ROW));
            if let Some(new) = rename_field(app, ui, id, depth, &n.name, r) {
                rename_done = Some((id, new));
            }
        }
        // Empty space below clears selection.
        let rest = Rect::from_x_y_ranges(band.x_range(), (top + total)..=(top + total.max(viewport.max.y)));
        // A click target for the pointer only (no stop for the keyboard).
        if rest.height() > 0.0 && ui.interact(rest, Id::new("layers_empty_space"), Sense::CLICK).clicked() {
            app.run("select.none", json!({}));
        }
    });
    // The dragged layers' name follows the pointer.
    if let (Some(dg), Some(p)) = (dragging, ui.input(|i| i.pointer.hover_pos())) {
        let label = match moving.len() {
            1 => doc.node(dg).map(|n| n.name.clone()).unwrap_or_default(),
            n => trn("{n} layers", n, &[]),
        };
        let galley = crate::bidi::name_galley(ui.painter(), &label, regular(11.0), t.text, 240.0);
        crate::widgets::drag_ghost(ui.ctx(), p, galley);
        // Over a row that takes no drop (the dragged layers themselves and what is in them).
        if refused {
            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
        }
    }
    if let Some(sw) = sweep {
        let (released, down) = ui.input(|i| (i.pointer.any_released(), i.pointer.primary_down()));
        if released {
            let (cmd, key) = if sw.lock { ("object.toggleLock", "locked") } else { ("object.toggleVisible", "visible") };
            app.run(cmd, json!({"ids": sw.ids, key: if sw.lock { sw.on } else { !sw.on }}));
        }
        // A sweep whose release the panel never saw (it wasn't shown) is dropped.
        if released || !down {
            ui.data_mut(|d| d.remove::<Sweep>(sweep_id()));
        } else {
            ui.data_mut(|d| d.insert_temp(sweep_id(), sw));
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        ui.data_mut(|d| {
            d.remove::<NodeId>(drag_id);
            d.remove::<(NodeId, f64)>(Id::new("layer_drag_hold"));
        });
        if let (Some(_), Some((target, zone))) = (dragging, drop) {
            let ids = moving.clone();
            match zone {
                0 => {
                    app.run("node.reparent", json!({"ids": ids, "parent": target}));
                }
                z => {
                    // Above in the list = higher z = after the target among siblings.
                    if let Some(parent) = doc.node(target).and_then(|n| n.parent) {
                        let idx = doc.index_in_parent(target).unwrap_or(0);
                        let index = if z < 0 { idx + 1 } else { idx };
                        app.run("node.reparent", json!({"ids": ids, "parent": parent, "index": index}));
                    }
                }
            }
        }
    }
    if let Ok(ds) = app.session.ds_mut() {
        let h = hovered_row;
        if h.is_some() && ds.hover != h {
            ds.hover = h;
        }
    }
    if let Some(id) = toggle
        && !app.ui.expanded.remove(&id)
    {
        app.ui.expanded.insert(id);
    }
    if let Some((id, open)) = deep {
        set_expanded_deep(&doc, id, open, &mut app.ui.expanded);
    }
    if let Some(id) = vis {
        app.run("object.toggleVisible", json!({"ids": [id]}));
    }
    if let Some(id) = lock {
        app.run("object.toggleLock", json!({"ids": [id]}));
    }
    if let Some((id, new)) = rename_done {
        app.run("object.rename", json!({"ids": [id], "name": new}));
    }
    if let Some(id) = rename_next {
        rename_layer(app, id);
    }
    if renaming && app.ui.rename.is_none() {
        a11y::refocus_row(&mut app.ui.nav);
    }
    if let Some((id, m)) = clicked {
        app.ui.layers_focus = true;
        app.ui.pages_focus = None;
        let order: Vec<NodeId> = rows.iter().map(|r| r.id).collect();
        let pos = |x: NodeId| order.iter().position(|o| *o == x);
        let anchor = app.ui.layer_anchor.filter(|a| sel.contains(a)).and_then(pos);
        if m.shift
            && let (Some(ai), Some(bi)) = (anchor, pos(id))
        {
            // Shift: the visible rows from the anchor to here (added to the selection with Cmd).
            let (lo, hi) = (ai.min(bi), ai.max(bi));
            let mut s: Vec<NodeId> = if m.command { sel.clone() } else { vec![] };
            for x in order.get(lo..=hi).unwrap_or_default() {
                if !s.contains(x) {
                    s.push(*x);
                }
            }
            app.run("select.set", json!({"ids": s}));
        } else if m.shift || m.command {
            let mut s = sel.clone();
            if let Some(pos) = s.iter().position(|x| *x == id) {
                s.remove(pos);
            } else {
                s.push(id);
            }
            app.ui.layer_anchor = Some(id);
            app.run("select.set", json!({"ids": s}));
        } else {
            app.ui.layer_anchor = Some(id);
            app.run("select.set", json!({"ids": [id]}));
        }
        // A click in the panel needs no reveal scroll.
        app.ui.layers_seen_sel = app.session.selection();
        if let Ok(ds) = app.session.ds_mut()
            && ds.text_edit.is_some()
        {
            let _ = figcraft_engine::textedit::end(&mut app.session);
        }
    }
    let _ = Color32::TRANSPARENT;
}
