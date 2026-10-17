//! The right panel: Design, Prototype and Inspect tabs.

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_color::{BlendMode, Paint, PaintKind};
use figcraft_doc::*;
use figcraft_engine::cmd::lists::{moved_index, removed_index};
use serde_json::{Value, json};

use crate::a11y::{self, Tip};
use crate::i18n::{tr, trf, trn};
use crate::icons::{self, Icon};
use crate::list_drag::{ListDrag, move_flags, remove_flags};
use crate::style_picker::{self, StyleSlot};
use crate::theme::{ACCENT, Tokens, regular, semibold};
use crate::widgets::{self, Edit, FIELD_H, FieldEdit, Lead, PAD};
use crate::{FigApp, RightTab};

/// Common value of `f` across `ids` (None when mixed or empty).
pub(crate) fn common<T: PartialEq + Clone>(doc: &Document, ids: &[NodeId], f: impl Fn(&Node) -> T) -> Option<T> {
    let mut it = ids.iter().filter_map(|i| doc.node(*i));
    let first = f(it.next()?);
    if it.all(|n| f(n) == first) { Some(first) } else { None }
}

impl FigApp {
    /// Set properties on `ids` (one undo step).
    pub fn set_props(&mut self, ids: &[NodeId], props: Value) {
        if ids.is_empty() {
            return;
        }
        self.run("node.set", json!({"ids": ids, "props": props}));
    }

    /// Apply a field edit: scrubs are live previews committed as one step.
    pub fn field(&mut self, ids: &[NodeId], key: &str, edit: Option<FieldEdit>, map: impl Fn(f64) -> Value) {
        if ids.is_empty() {
            return;
        }
        self.field_run(edit, "node.set", |v| json!({"ids": ids, "props": { key: map(v) }}));
    }

    /// [`Self::field`] for a field showing `shown(layer)`: a relative edit of mixed values (`+10`,
    /// `*2`, an arrow key's step) goes to each layer from its own value, in one undo step.
    pub fn field_of(&mut self, ids: &[NodeId], key: &str, edit: Option<Edit>, shown: impl Fn(&Node) -> f64, map: impl Fn(f64) -> Value) {
        let rel = match edit {
            Some(Edit::Each(rel)) => rel,
            Some(Edit::All(e)) => return self.field(ids, key, Some(e), map),
            None => return,
        };
        let Ok(doc) = self.session.doc() else { return };
        let values: Vec<(NodeId, f64)> = ids.iter().filter_map(|i| Some((*i, rel.apply(shown(doc.node(*i)?))?))).collect();
        if values.is_empty() || self.session.begin_interaction("Edit").is_err() {
            return;
        }
        for (id, v) in values {
            if let Err(e) = self.session.execute("node.set", &json!({"ids": [id], "props": { key: map(v) }})) {
                self.failed("node.set", &e);
            }
        }
        let _ = self.session.commit_interaction();
    }

    /// Apply a field edit through any command: `params` builds the command's params from the value.
    pub fn field_run(&mut self, edit: Option<FieldEdit>, cmd: &str, params: impl Fn(f64) -> Value) {
        match edit {
            Some(FieldEdit::Set(v)) => {
                self.run(cmd, params(v));
            }
            Some(FieldEdit::Scrub(v, done)) => {
                let _ = self.session.begin_interaction("Edit");
                let _ = self.session.execute(cmd, &params(v));
                if done {
                    let _ = self.session.commit_interaction();
                }
            }
            None => {}
        }
    }
}

pub fn right_panel(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    // Top bar: people (you and the others in a live session), present, share.
    let (top, _) = ui.allocate_exact_size(vec2(w, 48.0), Sense::hover());
    // As wide as its label needs (longer in some languages).
    let label = ui.painter().layout_no_wrap(tr("Share").to_string(), semibold(11.5), Color32::WHITE);
    let sw = (label.size().x + 24.0).max(56.0);
    let share = Rect::from_min_size(pos2(top.max.x - 14.0 - sw, top.center().y - 16.0), vec2(sw, 32.0));
    let sresp = ui.interact(share, ui.id().with("share"), Sense::click()).tip(tr("Share: live session, export"));
    ui.painter().rect_filled(share, CornerRadius::same(6), if sresp.hovered() { t.accent_hover } else { ACCENT });
    ui.painter().galley(share.center() - label.size() / 2.0, label, Color32::WHITE);
    crate::live::share_popover(app, &sresp);
    // Play presents; its chevron opens the play menu (present, or preview in a pane).
    let play = Rect::from_min_size(pos2(share.min.x - 50.0, top.center().y - 14.0), vec2(42.0, 28.0));
    let (tri_r, chev_r) =
        (Rect::from_min_max(play.min, pos2(play.max.x - 16.0, play.max.y)), Rect::from_min_max(pos2(play.max.x - 16.0, play.min.y), play.max));
    let present_keys = crate::toolbar::pretty_shortcut(crate::shortcuts::shortcut("proto.present").unwrap_or(""));
    let presp = ui.interact(tri_r, ui.id().with("play"), Sense::click()).tip(format!("{}  {present_keys}", tr("Present")));
    let mresp = ui.interact(chev_r, ui.id().with("play_menu"), Sense::click()).tip(tr("Present or preview"));
    if presp.hovered() || mresp.hovered() {
        ui.painter().rect_filled(play, CornerRadius::same(5), t.hover);
    }
    let tri = Rect::from_center_size(play.center() - vec2(7.0, 0.0), vec2(16.0, 16.0));
    let c = tri.center();
    ui.painter().add(egui::Shape::closed_line(vec![c + vec2(-5.0, -7.0), c + vec2(7.0, 0.0), c + vec2(-5.0, 7.0)], Stroke::new(1.4, t.icon)));
    icons::paint(ui.painter(), Rect::from_center_size(play.center() + vec2(12.0, 0.0), vec2(10.0, 10.0)), Icon::Chevron, t.icon_secondary);
    crate::live::people_bar(app, ui, Rect::from_min_max(pos2(top.min.x + 12.0, top.min.y), pos2(play.min.x - 6.0, top.max.y)));
    if presp.clicked() {
        app.run("proto.present", json!({}));
    }
    menu_popup(&mresp).width(200.0).show(|ui| {
        if menu_item(ui, tr("Present"), &present_keys) {
            app.run("proto.present", json!({}));
            ui.close();
        }
        let (label, cmd) = if app.ui.preview.is_some() { (tr("Close preview"), "proto.closePreview") } else { (tr("Preview"), "proto.preview") };
        if menu_item(ui, label, "") {
            app.run(cmd, json!({}));
            ui.close();
        }
    });
    // Tabs row.
    let (tabs, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    ui.painter().line_segment([tabs.left_top(), tabs.right_top()], Stroke::new(1.0, t.border));
    let mut x = tabs.min.x + 8.0;
    for (tab, label) in [(RightTab::Design, tr("Design")), (RightTab::Prototype, tr("Prototype"))] {
        let galley = ui.painter().layout_no_wrap(label.into(), semibold(11.0), t.text);
        let r = Rect::from_min_size(pos2(x, tabs.center().y - 12.0), vec2(galley.size().x + 16.0, 24.0));
        let resp = ui.interact(r, ui.id().with(("rtab", label)), Sense::click());
        let active = app.ui.right_tab == tab;
        a11y::tab(&resp, label, active);
        if active {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        ui.painter().galley(pos2(r.min.x + 8.0, r.center().y - galley.size().y / 2.0), galley, if active { t.text } else { t.text_secondary });
        if resp.clicked() {
            app.ui.right_tab = tab;
        }
        x = r.max.x + 2.0;
    }
    // Zoom menu.
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let zr = Rect::from_min_size(pos2(tabs.max.x - 64.0, tabs.center().y - 12.0), vec2(56.0, 24.0));
    let zresp = ui.interact(zr, ui.id().with("zoommenu"), Sense::click());
    // The value is only spelled out while a screen reader reads the tree.
    let value = if a11y::active(&zresp.ctx) { format!("{}%", (zoom * 100.0).round()) } else { String::new() };
    a11y::combo(&zresp, tr("Zoom"), &value, a11y::menu_open(&zresp));
    if zresp.hovered() {
        ui.painter().rect_filled(zr, CornerRadius::same(5), t.hover);
    }
    ui.painter().text(pos2(zr.min.x + 6.0, zr.center().y), Align2::LEFT_CENTER, format!("{}%", (zoom * 100.0).round()), regular(11.0), t.text);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(zr.max.x - 9.0, zr.center().y), vec2(10.0, 10.0)), Icon::Chevron, t.icon_secondary);
    menu_popup(&zresp).width(200.0).show(|ui| {
        for (label, cmd) in [
            (tr("Zoom in"), "view.zoomIn"),
            (tr("Zoom out"), "view.zoomOut"),
            (tr("Zoom to fit"), "view.zoomFit"),
            (tr("Zoom to 50%"), "view.zoom50"),
            (tr("Zoom to 100%"), "view.zoom100"),
            (tr("Zoom to 200%"), "view.zoom200"),
            (tr("Zoom to selection"), "view.zoomSelection"),
        ] {
            if menu_item(ui, label, crate::shortcuts::shortcut(cmd).unwrap_or("")) {
                app.run(cmd, json!({}));
                ui.close();
            }
        }
        ui.separator();
        let Ok(v) = app.session.ds().map(|d| d.view.clone()) else { return };
        for (label, scale) in [(tr("Pixel preview 1x"), 1u8), (tr("Pixel preview 2x"), 2)] {
            let on = v.pixel_preview && v.pixel_scale == scale;
            if menu_check(ui, on, label, "") {
                app.run("view.pixelPreview", json!({"on": !on, "scale": scale}));
                ui.close();
            }
        }
        let toggles: [(&str, &str, bool); 5] = [
            (tr("Pixel grid"), "view.pixelGrid", v.pixel_grid),
            (tr("Snap to pixel grid"), "view.snapPixel", v.snap_pixel),
            (tr("Layout grids"), "view.layoutGrids", v.layout_grids),
            (tr("Rulers"), "view.rulers", v.rulers),
            (tr("Outlines"), "view.outlines", v.outlines),
        ];
        for (label, cmd, on) in toggles {
            if menu_check(ui, on, label, crate::shortcuts::shortcut(cmd).unwrap_or("")) {
                app.run(cmd, json!({}));
                ui.close();
            }
        }
    });
    ui.painter().line_segment([tabs.left_bottom(), tabs.right_bottom()], Stroke::new(1.0, t.border));
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
        if app.session.tool.tool == figcraft_engine::ToolId::Comment {
            crate::comments::panel(app, ui);
            ui.add_space(80.0);
            return;
        }
        match app.ui.right_tab {
            // The Frame tool shows the frame presets.
            RightTab::Design if app.session.tool.tool == figcraft_engine::ToolId::Frame => crate::frame_presets::panel(app, ui),
            RightTab::Design => design(app, ui),
            RightTab::Prototype => crate::proto_panel::prototype(app, ui),
            RightTab::Inspect => inspect(app, ui),
        }
        ui.add_space(80.0);
        a11y::scroll_focus(&mut app.ui.nav, ui);
    });
}

/// The smallest width a popup menu is measured in (see [`menu_popup`]).
const MENU_MIN_W: f32 = 48.0;
/// A menu row's inset either side, and the gap between its label and its shortcut or art.
const MENU_PAD: f32 = 8.0;
const MENU_GAP: f32 = 24.0;

/// The style of popup menus: egui's, with text kept on one line (a menu grows to its widest item).
pub(crate) fn menu_style(s: &mut egui::Style) {
    egui::containers::menu::menu_style(s);
    s.wrap_mode = Some(egui::TextWrapMode::Extend);
}

/// The menu a dropdown or button opens: as wide as its widest row and at least as wide as
/// `anchor`. egui measures a popup in a first, invisible pass as wide as the width it is given
/// (else its 600 px default area width) and keeps the size measured; rows ([`menu_row`]) take the
/// larger of that width and their content, so the menu fits its content. Chain `.width(w)` for a
/// menu at least `w` wide.
/// The blend modes as menu items in groups (pass-through only for layers); the one picked.
pub fn blend_menu(ui: &mut Ui, with_pass_through: bool) -> Option<BlendMode> {
    let mut picked = None;
    for b in BlendMode::ALL.into_iter().filter(|b| with_pass_through || *b != BlendMode::PassThrough) {
        if menu_item(ui, tr(b.label()), "") {
            picked = Some(b);
            ui.close();
        }
        if matches!(b, BlendMode::Normal | BlendMode::ColorBurn | BlendMode::ColorDodge | BlendMode::HardLight | BlendMode::Exclusion) {
            ui.separator();
        }
    }
    picked
}

pub fn menu_popup(anchor: &egui::Response) -> egui::Popup<'static> {
    egui::Popup::menu(anchor).width(anchor.rect.width().max(MENU_MIN_W)).style(menu_style)
}

/// A context menu (secondary click on `resp`) sized like [`menu_popup`].
pub fn context_popup(resp: &egui::Response) -> egui::Popup<'static> {
    egui::Popup::context_menu(resp).width(MENU_MIN_W).style(menu_style)
}

pub fn menu_item(ui: &mut Ui, label: &str, shortcut: &str) -> bool {
    menu_row(ui, label, shortcut, 0.0, |_, _, _| {}).clicked()
}

/// A menu row with art drawn into it after the label (`art(painter, row rect, text colour)`),
/// taking `art_w` from the row's right edge.
pub fn menu_item_with(ui: &mut Ui, label: &str, shortcut: &str, art_w: f32, art: impl FnOnce(&egui::Painter, Rect, Color32)) -> bool {
    menu_row(ui, label, shortcut, art_w, art).clicked()
}

/// The width a menu row needs for a label `label_w` wide and, at its right end, a shortcut or
/// art `trail` wide (0 for neither).
pub(crate) fn menu_row_width(label_w: f32, trail: f32) -> f32 {
    MENU_PAD + label_w + if trail > 0.0 { MENU_GAP + trail } else { MENU_PAD }
}

/// A menu row (see [`menu_item_with`]); its response tells hovering too. It spans the menu and
/// is at least as wide as its label, shortcut and art.
pub fn menu_row(ui: &mut Ui, label: &str, shortcut: &str, art_w: f32, art: impl FnOnce(&egui::Painter, Rect, Color32)) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let text = ui.painter().layout_no_wrap(label.to_owned(), regular(11.5), Color32::PLACEHOLDER);
    let keys =
        (!shortcut.is_empty()).then(|| ui.painter().layout_no_wrap(crate::toolbar::pretty_shortcut(shortcut), regular(11.0), Color32::PLACEHOLDER));
    let trail = keys.as_ref().map_or(0.0, |k| k.size().x + MENU_PAD).max(art_w);
    let w = ui.available_width().max(menu_row_width(text.size().x, trail).ceil());
    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    a11y::menu_item(&resp, label, shortcut, None);
    // The keyboard's item is highlighted like the hovered one.
    a11y::own_focus(&resp);
    let hovered = resp.hovered() || resp.has_focus();
    if hovered {
        ui.painter().rect_filled(r, CornerRadius::same(5), ACCENT);
    }
    let col = if hovered { Color32::WHITE } else { t.text };
    ui.painter().galley(pos2(r.min.x + MENU_PAD, r.center().y - text.size().y / 2.0), text, col);
    if let Some(k) = keys {
        let at = pos2(r.max.x - MENU_PAD - k.size().x, r.center().y - k.size().y / 2.0);
        ui.painter().galley(at, k, if hovered { Color32::WHITE } else { t.text_secondary });
    }
    art(ui.painter(), r, col);
    resp
}

/// A menu item with a check mark when `on` (the unchecked ones line up with it).
pub fn menu_check(ui: &mut Ui, on: bool, label: &str, shortcut: &str) -> bool {
    marked_row(ui, Some(on), label, shortcut)
}

/// A plain menu item lined up with the check items above or below it.
pub fn menu_item_beside_checks(ui: &mut Ui, label: &str, shortcut: &str) -> bool {
    marked_row(ui, None, label, shortcut)
}

fn marked_row(ui: &mut Ui, on: Option<bool>, label: &str, shortcut: &str) -> bool {
    let resp = menu_row(ui, &format!("{}{label}", if on == Some(true) { "✓ " } else { "    " }), shortcut, 0.0, |_, _, _| {});
    a11y::menu_item(&resp, label, shortcut, on);
    resp.clicked()
}

pub(crate) fn row(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui)) {
    row_rect(ui, h, add);
}

/// [`row`], returning the area the row takes.
pub(crate) fn row_rect(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui)) -> Rect {
    block(ui, h, (h - FIELD_H) / 2.0, add)
}

/// A panel row `h` tall whose content is inset `inset_y` from its top and bottom (for content
/// taller than one field); returns the area the row takes.
pub(crate) fn block(ui: &mut Ui, h: f32, inset_y: f32, add: impl FnOnce(&mut Ui)) -> Rect {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(PAD, inset_y))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing = vec2(8.0, 0.0);
    add(&mut child);
    r
}

fn design(app: &mut FigApp, ui: &mut Ui) {
    let Ok(ds) = app.session.ds() else { return };
    // The panel shows the document itself, not a live text preview over it.
    let doc = ds.text_preview.clone().unwrap_or_else(|| ds.doc.clone());
    let sel = ds.selection.clone();
    let vector_edit = ds.vector_edit.clone();
    if sel.is_empty() {
        page_section(app, ui, &doc);
        return;
    }
    header(app, ui, &doc, &sel);
    widgets::separator(ui);
    // Editing a vector with points selected, the points' fields stand in for the layer's position
    // and dimensions.
    let points_selected = vector_edit.as_ref().is_some_and(|ve| !ve.vertices.is_empty() || !ve.segments.is_empty());
    let vector_edit_none = vector_edit.is_none();
    match vector_edit {
        Some(ve) => crate::vector_panel::points(app, ui, &doc, &ve),
        None => {
            crate::section_panel::section(app, ui, &doc, &sel);
            if app.session.tool.tool == figcraft_engine::ToolId::Scale {
                crate::scale_panel::section(app, ui, &doc, &sel);
                widgets::separator(ui);
            }
            position(app, ui, &doc, &sel);
        }
    }
    widgets::separator(ui);
    if !points_selected {
        crate::layout_panel::layout_section(app, ui, &doc, &sel);
        widgets::separator(ui);
    }
    if vector_edit_none && (arc_section(app, ui, &doc, &sel) || points_section(app, ui, &doc, &sel)) {
        widgets::separator(ui);
    }
    appearance(app, ui, &doc, &sel);
    widgets::separator(ui);
    if variable_modes(app, ui, &doc, &sel) {
        widgets::separator(ui);
    }
    if sel.iter().any(|i| matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Text(_)))) {
        crate::typography::typography(app, ui, &doc, &sel);
        widgets::separator(ui);
    }
    paints(app, ui, &doc, &sel, true);
    widgets::separator(ui);
    paints(app, ui, &doc, &sel, false);
    widgets::separator(ui);
    effects(app, ui, &doc, &sel);
    widgets::separator(ui);
    crate::selection_colors::section(app, ui, &doc, &sel);
    if sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like())) {
        grids(app, ui, &doc, &sel);
        widgets::separator(ui);
    }
    crate::export_panel::section(app, ui, &doc, &sel);
}

fn page_section(app: &mut FigApp, ui: &mut Ui, doc: &Document) {
    let t = Tokens::get(ui.ctx());
    let Ok(page) = app.session.page() else { return };
    let bg = match doc.node(page).map(|n| &n.kind) {
        Some(NodeKind::Page { background, .. }) => *background,
        _ => figcraft_color::Color::DEFAULT_PAGE,
    };
    widgets::section_header(ui, tr("Page"), &[], false);
    let mut open_picker = false;
    let mut new_hex = None;
    row(ui, 32.0, |ui| {
        let p = Paint::solid(bg);
        if widgets::swatch(ui, &p, 16.0).clicked() {
            open_picker = true;
        }
        if let Some(h) = widgets::text_field(ui, Id::new("pagebg"), &bg.to_hex(), 96.0, "") {
            new_hex = Some(h);
        }
        widgets::label(ui, "100 %", t.text);
    });
    if open_picker {
        app.ui.picker = Some(crate::colorpicker::PickerTarget::PageBackground);
    }
    if let Some(h) = new_hex
        && let Some(c) = figcraft_color::Color::from_hex_or_name(&h)
    {
        app.run("page.setBackground", json!({"color": c.to_css_hex()}));
    }
    widgets::separator(ui);
    widgets::section_header(ui, tr("Color profile"), &[], false);
    row(ui, 32.0, |ui| {
        let resp = widgets::dropdown(ui, doc.color_profile.label(), 140.0, None);
        menu_popup(&resp).show(|ui| {
            for p in figcraft_color::ColorProfile::ALL {
                if menu_item(ui, p.label(), "") {
                    if p != doc.color_profile {
                        app.ui.color_profile = Some(p);
                    }
                    ui.close();
                }
            }
        });
    });
    widgets::separator(ui);
    // Local variables & styles.
    let vars = doc.variables.variables.iter().filter(|v| !figcraft_engine::library::is_remote_variable(doc, &v.id)).count();
    widgets::section_header(ui, tr("Local variables"), &[], vars == 0);
    row(ui, 32.0, |ui| {
        let r = widgets::dropdown(ui, &trf("Open variables ({count})", &[("count", &vars)]), 200.0, Some(Icon::Styles));
        if r.clicked() {
            app.ui.preferences = false;
            app.ui.palette = None;
            app.ui.variables.get_or_insert_with(Default::default);
        }
    });
    widgets::separator(ui);
    widgets::section_header(ui, tr("Local styles"), &[], figcraft_engine::library::local_styles(doc).next().is_none());
    crate::style_picker::local_styles(app, ui, doc);
    widgets::separator(ui);
    crate::export_panel::page_section(app, ui, doc, page);
}

fn header(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let t = Tokens::get(ui.ctx());
    let label = match common(doc, sel, |n| n.kind.label()) {
        Some(l) if sel.len() == 1 => tr(l).to_string(),
        // The plural ("Frames") is a key of its own.
        Some(l) => tr(&format!("{l}s")).to_string(),
        None => tr("Mixed").into(),
    };
    let is_frame = sel.len() == 1 && sel.first().and_then(|i| doc.node(*i)).is_some_and(|n| matches!(n.kind, NodeKind::Frame));
    // Frames and groups convert into each other from the type menu.
    let is_group = !sel.is_empty() && sel.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Group)));
    let kind_items = |app: &mut FigApp, ui: &mut Ui, frame: bool| {
        for (to_frame, label, cmd) in [(true, tr("Frame"), "object.convertToFrame"), (false, tr("Group"), "object.convertToGroup")] {
            if menu_check(ui, to_frame == frame, label, "") {
                if to_frame != frame {
                    app.run(cmd, json!({"ids": sel}));
                }
                ui.close();
            }
        }
    };
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    let lr = Rect::from_min_size(pos2(r.min.x + 8.0, r.center().y - 12.0), vec2(120.0, 24.0));
    let resp = ui.interact(lr, ui.id().with("typedd"), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(lr, CornerRadius::same(5), t.hover);
    }
    ui.painter().text(pos2(lr.min.x + 8.0, lr.center().y), Align2::LEFT_CENTER, &label, semibold(13.0), t.text);
    if is_frame || is_group {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(lr.min.x + 18.0 + label.len() as f32 * 6.5, lr.center().y), vec2(10.0, 10.0)),
            Icon::Chevron,
            t.icon_secondary,
        );
    }
    if is_group {
        menu_popup(&resp).width(180.0).show(|ui| kind_items(app, ui, false));
    }
    if is_frame {
        let sr = Rect::from_min_size(pos2(lr.max.x + 4.0, r.center().y - 12.0), vec2(24.0, 24.0));
        let sresp = ui.interact(sr, ui.id().with("swap_orient"), Sense::click()).tip(tr("Swap orientation"));
        if sresp.hovered() {
            ui.painter().rect_filled(sr, CornerRadius::same(5), t.hover);
        }
        icons::paint(ui.painter(), sr.shrink(5.0), Icon::Rotate, t.icon);
        if sresp.clicked() {
            app.run("frame.swapOrientation", json!({"ids": sel}));
        }
        let ids: Vec<NodeId> = sel.to_vec();
        menu_popup(&resp).width(260.0).show(|ui| {
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                kind_items(app, ui, true);
                ui.separator();
                let mut cat = "";
                for (c, name, pw, ph) in figcraft_engine::cmd::create::FRAME_PRESETS {
                    if *c != cat {
                        cat = c;
                        ui.label(egui::RichText::new(tr(c)).font(semibold(11.0)).color(t.text_secondary));
                    }
                    if menu_item(ui, name, &format!("{}×{}", pw, ph)) {
                        app.set_props(&ids, json!({"width": pw, "height": ph}));
                        ui.close();
                    }
                }
            });
        });
    }
    // Quick actions on the right.
    let mut x = r.max.x - 8.0;
    let actions = [
        (Icon::Component, tr("Create component"), "component.create"),
        (Icon::Mask, tr("Use as mask"), "object.mask"),
        (Icon::Union, tr("Boolean groups"), "bool.union"),
    ];
    // Layers in boolean groups, or one boolean group: the operation they use (checked in the menu).
    let current = figcraft_engine::booleans::shown_op(doc, sel);
    for (icon, label, cmd) in actions.into_iter().rev() {
        let br = Rect::from_min_size(pos2(x - 24.0, r.center().y - 12.0), vec2(24.0, 24.0));
        let tip = if cmd == "bool.union" { label } else { crate::shortcuts::tip(label, cmd) };
        let bresp = ui.interact(br, ui.id().with(("qa", cmd)), Sense::click()).tip(tip);
        if bresp.hovered() {
            ui.painter().rect_filled(br, CornerRadius::same(5), t.hover);
        }
        // The boolean button shows the operation in use.
        let icon = match (cmd, current) {
            ("bool.union", Some(op)) => crate::layers::op_icon(op),
            _ => icon,
        };
        icons::paint(ui.painter(), br.shrink(5.0), icon, t.icon);
        if cmd == "bool.union" {
            menu_popup(&bresp).width(180.0).show(|ui| {
                for (op, c) in [
                    (BoolOp::Union, "bool.union"),
                    (BoolOp::Subtract, "bool.subtract"),
                    (BoolOp::Intersect, "bool.intersect"),
                    (BoolOp::Exclude, "bool.exclude"),
                ] {
                    let key = crate::shortcuts::shortcut(c).unwrap_or("");
                    let picked = match current {
                        Some(cur) => menu_check(ui, cur == op, tr(op.label()), key),
                        None => menu_item(ui, tr(op.label()), key),
                    };
                    if picked {
                        app.run(c, json!({}));
                        ui.close();
                    }
                }
                let key = crate::shortcuts::shortcut("vector.flatten").unwrap_or("");
                let flatten = if current.is_some() { menu_item_beside_checks(ui, tr("Flatten"), key) } else { menu_item(ui, tr("Flatten"), key) };
                if flatten {
                    app.run("vector.flatten", json!({}));
                    ui.close();
                }
            });
        } else if bresp.clicked() {
            app.run(cmd, json!({}));
        }
        x -= 26.0;
    }
    crate::component_panel::instance_section(app, ui, doc, sel);
}

fn position(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    widgets::section_header(ui, tr("Position"), &[], false);
    // Layers inside an instance take their place and constraints from the main component.
    let nested = sel.iter().any(|i| figcraft_engine::components::inside_instance(doc, *i));
    ui.add_enabled_ui(!nested, |ui| position_fields(app, ui, doc, sel))
        .response
        .on_disabled_hover_text(tr("Layers in an instance take their position from the main component"));
    ui.add_space(8.0);
}

fn position_fields(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids = sel.to_vec();
    let col = widgets::col_width(ui.available_width());
    widgets::sublabels(ui, tr("Alignment"), None);
    row(ui, 32.0, |ui| {
        let cmds = ["align.left", "align.hcenter", "align.right", "align.top", "align.vcenter", "align.bottom"];
        let tip = crate::shortcuts::tip;
        let items = [
            (Icon::AlignLeft, tip(tr("Align left"), cmds[0])),
            (Icon::AlignHCenter, tip(tr("Align horizontal centers"), cmds[1])),
            (Icon::AlignRight, tip(tr("Align right"), cmds[2])),
            (Icon::AlignTop, tip(tr("Align top"), cmds[3])),
            (Icon::AlignVCenter, tip(tr("Align vertical centers"), cmds[4])),
            (Icon::AlignBottom, tip(tr("Align bottom"), cmds[5])),
        ];
        let w = col / 3.0;
        let picked = ui.add_enabled_ui(figcraft_engine::cmd::arrange::can_align(doc, sel), |ui| {
            let h = widgets::segmented(ui, &items[..3], None, w);
            let v = widgets::segmented(ui, &items[3..], None, w).map(|i| i + 3);
            h.or(v)
        });
        picked.response.on_disabled_hover_text(tr("A layer on its own aligns to its frame: select more layers, or one in a frame"));
        if let Some(i) = picked.inner {
            app.run(cmds[i], json!({"ids": ids}));
        }
        if sel.len() > 2 {
            let more = icons::button(ui, Icon::DistributeH, 24.0, false, tr("Distribute and tidy"));
            menu_popup(&more).show(|ui| {
                for (l, c) in [
                    (tr("Distribute horizontal spacing"), "distribute.horizontal"),
                    (tr("Distribute vertical spacing"), "distribute.vertical"),
                    (tr("Tidy up"), "arrange.tidy"),
                ] {
                    if menu_item(ui, l, crate::shortcuts::shortcut(c).unwrap_or("")) {
                        app.run(c, json!({"ids": ids}));
                        ui.close();
                    }
                }
            });
        }
    });
    // In the frame's space: groups have none of their own.
    // What each field shows of a layer (a relative edit of mixed values starts from it).
    let x_of = |n: &Node| (doc.frame_relative(n.id).as_coeffs()[4] * 100.0).round() / 100.0;
    let y_of = |n: &Node| (doc.frame_relative(n.id).as_coeffs()[5] * 100.0).round() / 100.0;
    let rot_of = |n: &Node| (figcraft_geom::rotation_degrees(n.transform) * 100.0).round() / 100.0;
    let (x, y, rot) = (common(doc, sel, x_of), common(doc, sel, y_of), common(doc, sel, rot_of));
    let parent_al = sel.iter().all(|i| doc.node(*i).and_then(|n| n.parent).and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some()));
    let absolute = common(doc, sel, |n| n.child_layout.absolute);
    widgets::sublabels(ui, tr("Position"), None);
    row(ui, 32.0, |ui| {
        let e = widgets::number_field_mixed(ui, Id::new("pos_x"), Lead::Text("X"), x, "", col, 1.0);
        app.field_of(&ids, "x", e, x_of, |v| json!(v));
        let e = widgets::number_field_mixed(ui, Id::new("pos_y"), Lead::Text("Y"), y, "", col, 1.0);
        app.field_of(&ids, "y", e, y_of, |v| json!(v));
        // Children of auto layout frames can ignore the layout (absolute position).
        if parent_al {
            let on = absolute.unwrap_or(false);
            if icons::button(
                ui,
                Icon::AbsolutePosition,
                24.0,
                on,
                if on { tr("Use auto layout") } else { tr("Ignore auto layout (absolute position)") },
            )
            .clicked()
            {
                app.set_props(&ids, json!({"absolute": !on}));
            }
        }
    });
    widgets::sublabels(ui, tr("Rotation"), None);
    row(ui, 32.0, |ui| {
        let e = widgets::number_field_mixed(ui, Id::new("pos_rot"), Lead::Icon(Icon::Angle, tr("Rotation")), rot, "°", col, 1.0);
        app.field_of(&ids, "rotation", e, rot_of, |v| json!(v));
        let items = [
            (Icon::Rotate, tr("Rotate 90° right")),
            (Icon::FlipH, crate::shortcuts::tip(tr("Flip horizontal"), "object.flipH")),
            (Icon::FlipV, crate::shortcuts::tip(tr("Flip vertical"), "object.flipV")),
        ];
        if let Some(i) = widgets::segmented(ui, &items, None, col / 3.0) {
            app.run(["object.rotateRight", "object.flipH", "object.flipV"][i], json!({"ids": ids}));
        }
    });
    // Constraints (children of frames without auto layout, or ignoring it).
    let in_frame = sel.iter().all(|i| {
        doc.node(*i).is_some_and(|n| {
            n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.kind.is_frame_like() && (p.layout.is_none() || n.child_layout.absolute))
        })
    });
    if in_frame {
        let c = common(doc, sel, |n| n.constraints);
        widgets::sublabels(ui, tr("Constraints"), None);
        row(ui, 32.0, |ui| {
            let h = c.map(|c| format!("{:?}", c.horizontal)).unwrap_or_else(|| "Mixed".into());
            let resp = widgets::dropdown(ui, tr(&pretty(&h)), col, None);
            menu_popup(&resp).show(|ui| {
                for (l, v) in [
                    (tr("Left"), "left"),
                    (tr("Right"), "right"),
                    (tr("Left & right"), "leftRight"),
                    (tr("Center"), "center"),
                    (tr("Scale"), "scale"),
                ] {
                    if menu_item(ui, l, "") {
                        app.set_props(&ids, json!({"constraints": {"horizontal": v}}));
                        ui.close();
                    }
                }
            });
            let v = c.map(|c| format!("{:?}", c.vertical)).unwrap_or_else(|| "Mixed".into());
            let resp = widgets::dropdown(ui, tr(&pretty(&v)), col, None);
            menu_popup(&resp).show(|ui| {
                for (l, val) in [
                    (tr("Top"), "top"),
                    (tr("Bottom"), "bottom"),
                    (tr("Top & bottom"), "topBottom"),
                    (tr("Center"), "center"),
                    (tr("Scale"), "scale"),
                ] {
                    if menu_item(ui, l, "") {
                        app.set_props(&ids, json!({"constraints": {"vertical": val}}));
                        ui.close();
                    }
                }
            });
        });
    }
}

/// The selected ellipses' arc: Start (degrees), Sweep (a share of a full turn, negative running
/// the other way) and Ratio (the inner radius), the same values the canvas arc handles drag
/// (`node.set`'s `arcStart` / `arcSweep` / `arcInner`; bindable to number variables, sweep and
/// ratio ones holding percentages). Returns whether it showed.
fn arc_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) -> bool {
    let arcs: Vec<(f64, f64, f64)> = sel
        .iter()
        .filter_map(|i| match doc.node(*i)?.kind {
            NodeKind::Ellipse { start, sweep, inner } => Some((start, sweep, inner)),
            _ => None,
        })
        .collect();
    if arcs.is_empty() || arcs.len() != sel.len() {
        return false;
    }
    let ids = sel.to_vec();
    let same = |f: &dyn Fn(&(f64, f64, f64)) -> f64| -> Option<f64> {
        let v = (f(arcs.first()?) * 100.0).round() / 100.0;
        arcs.iter().all(|a| ((f(a) * 100.0).round() / 100.0 - v).abs() < 1e-9).then_some(v)
    };
    let start = same(&|a| a.0.to_degrees().rem_euclid(360.0));
    let sweep = same(&|a| a.1 / std::f64::consts::TAU * 100.0);
    let ratio = same(&|a| a.2 * 100.0);
    widgets::section_header(ui, tr("Arc"), &[], false);
    let col = widgets::col_width(ui.available_width());
    let field = |app: &mut FigApp, ui: &mut Ui, key: &str, icon: Icon, name: &str, value: Option<f64>, suffix: &str| {
        crate::variable_ui::number_field(app, ui, &ids, key, Id::new(("arc", key)), Lead::Icon(icon, name), value, suffix, col)
    };
    widgets::sublabels(ui, tr("Start"), Some(tr("Sweep")));
    row(ui, 32.0, |ui| {
        let e = field(app, ui, "arcStart", Icon::ArcStart, tr("Arc start"), start, "°");
        app.field(&ids, "arcStart", e, |v| json!(v));
        let e = field(app, ui, "arcSweep", Icon::ArcSweep, tr("Arc sweep"), sweep, "%");
        app.field(&ids, "arcSweep", e, |v| json!(v.clamp(-100.0, 100.0) * 3.6));
    });
    widgets::sublabels(ui, tr("Ratio"), None);
    row(ui, 32.0, |ui| {
        let e = field(app, ui, "arcInner", Icon::ArcRatio, tr("Arc ratio"), ratio, "%");
        app.field(&ids, "arcInner", e, |v| json!((v / 100.0).clamp(0.0, 0.99)));
    });
    ui.add_space(8.0);
    true
}

/// The selected polygons' and stars' point count (3 to 60 on stars), and the stars' ratio (the
/// inner radius as a share of the outer one), as `node.set`'s `pointCount` / `innerRadius`.
/// Returns whether it showed.
fn points_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) -> bool {
    let shapes: Vec<(u32, Option<f64>)> = sel
        .iter()
        .filter_map(|i| match doc.node(*i)?.kind {
            NodeKind::Polygon { count } => Some((count, None)),
            NodeKind::Star { count, inner_ratio } => Some((count, Some(inner_ratio))),
            _ => None,
        })
        .collect();
    if shapes.is_empty() || shapes.len() != sel.len() {
        return false;
    }
    let ids = sel.to_vec();
    let count = shapes.first().map(|s| s.0).filter(|c| shapes.iter().all(|s| s.0 == *c)).map(f64::from);
    let stars: Vec<f64> = shapes.iter().filter_map(|s| s.1).collect();
    let ratio = stars.first().map(|r| (r * 1000.0).round() / 10.0).filter(|r| stars.iter().all(|x| ((x * 1000.0).round() / 10.0 - r).abs() < 1e-9));
    let all_stars = stars.len() == shapes.len();
    widgets::section_header(ui, tr(if all_stars { "Star" } else { "Polygon" }), &[], false);
    let col = widgets::col_width(ui.available_width());
    widgets::sublabels(ui, tr("Count"), all_stars.then_some(tr("Ratio")));
    row(ui, 32.0, |ui| {
        let e = widgets::number_field(ui, Id::new("pts_count"), Lead::Icon(Icon::Polygon, tr("Count")), count, "", col, 1.0);
        app.field(&ids, "pointCount", e, |v| json!(v.round().clamp(3.0, 120.0)));
        if all_stars {
            let e = widgets::number_field(ui, Id::new("pts_ratio"), Lead::Icon(Icon::Star, tr("Ratio")), ratio, "%", col, 1.0);
            app.field(&ids, "innerRadius", e, |v| json!((v / 100.0).clamp(0.001, 1.0)));
        }
    });
    ui.add_space(8.0);
    true
}

/// Constraint names as shown (English keys, translated at display).
fn pretty(s: &str) -> String {
    match s {
        "LeftRight" => "Left & right".into(),
        "TopBottom" => "Top & bottom".into(),
        other => other.to_string(),
    }
}

fn appearance(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids = sel.to_vec();
    let visible = common(doc, sel, |n| n.visible).unwrap_or(true);
    let hdr = widgets::section_header(
        ui,
        tr("Appearance"),
        &[
            (if visible { Icon::Eye } else { Icon::EyeOff }, crate::shortcuts::tip(tr("Show/Hide"), "object.toggleVisible")),
            (Icon::Variable, tr("Apply variable to visibility")),
            (Icon::Blend, tr("Blend mode")),
        ],
        false,
    );
    if hdr == Some(0) {
        app.run("object.toggleVisible", json!({"ids": ids}));
    }
    if hdr == Some(1) {
        crate::variable_ui::open_at_pointer(ui, &ids, "visible", VariableType::Boolean);
    }
    if hdr == Some(2) {
        ui.data_mut(|d| d.insert_temp(Id::new("blend_open"), !d.get_temp::<bool>(Id::new("blend_open")).unwrap_or(false)));
    }
    let col = widgets::col_width(ui.available_width());
    crate::variable_ui::bound_row(app, ui, &ids, "visible", VariableType::Boolean, Lead::Icon(Icon::Eye, tr("Visible")), col * 2.0 + 8.0);
    // Masks: how they mask.
    if sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.is_mask)) {
        let mt = common(doc, sel, |n| n.mask_type);
        widgets::sublabels(ui, tr("Mask"), None);
        row(ui, 32.0, |ui| {
            let resp = widgets::dropdown(ui, mt.map(|m| tr(m.label())).unwrap_or(tr("Mixed")), col * 2.0 + 8.0, Some(Icon::Mask));
            menu_popup(&resp).show(|ui| {
                for m in MaskType::ALL {
                    if menu_item(ui, tr(m.label()), "") {
                        app.set_props(&ids, json!({"maskType": m.id()}));
                        ui.close();
                    }
                }
            });
        });
    }
    let op_of = |n: &Node| (f64::from(n.opacity) * 100.0).round();
    let op = common(doc, sel, op_of);
    let radius_ok = sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.has_corner_radius()));
    // Only rectangles and frames round each corner on its own.
    let indep_ok = radius_ok && sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.has_independent_corners()));
    let r = common(doc, sel, |n| n.corners.uniform().map(|r| (r * 100.0).round() / 100.0));
    widgets::sublabels(ui, tr("Opacity"), radius_ok.then_some(tr("Corner radius")));
    row(ui, 32.0, |ui| {
        let lead = Lead::Icon(Icon::Opacity, tr("Opacity"));
        let e = crate::variable_ui::number_field_mixed(app, ui, &ids, "opacity", Id::new("ap_op"), lead, op, "%", col);
        app.field_of(&ids, "opacity", e, op_of, |v| json!((v / 100.0).clamp(0.0, 1.0)));
        if radius_ok {
            let e = crate::variable_ui::number_field_mixed(
                app,
                ui,
                &ids,
                "cornerRadius",
                Id::new("ap_rad"),
                Lead::Icon(Icon::CornerRadius, tr("Corner radius")),
                r.flatten(),
                "",
                col,
            );
            // A layer with corners of its own starts from its largest.
            let radius_of = |n: &Node| n.corners.uniform().unwrap_or_else(|| n.corners.radii.iter().copied().fold(0.0, f64::max));
            app.field_of(&ids, "cornerRadius", e, radius_of, |v| json!(v.max(0.0)));
            let open = ui.data(|d| d.get_temp::<bool>(Id::new("indep_corners"))).unwrap_or(false);
            if indep_ok && icons::button(ui, Icon::IndependentCorners, 24.0, open, tr("Individual corners")).clicked() {
                ui.data_mut(|d| d.insert_temp(Id::new("indep_corners"), !open));
            }
        }
    });
    if indep_ok && ui.data(|d| d.get_temp::<bool>(Id::new("indep_corners"))).unwrap_or(false) {
        let radii = common(doc, sel, |n| n.corners.radii);
        let keys = [
            ("topLeftRadius", tr("Top left radius")),
            ("topRightRadius", tr("Top right radius")),
            ("bottomRightRadius", tr("Bottom right radius")),
            ("bottomLeftRadius", tr("Bottom left radius")),
        ];
        row(ui, 32.0, |ui| {
            for (k, (key, name)) in keys.iter().enumerate().take(2) {
                let e = crate::variable_ui::number_field(
                    app,
                    ui,
                    &ids,
                    key,
                    Id::new(("corner", k)),
                    Lead::Icon(Icon::CornerRadius, name),
                    radii.map(|r| r[k]),
                    "",
                    col,
                );
                app.field(&ids, key, e, |v| json!(v.max(0.0)));
            }
        });
        row(ui, 32.0, |ui| {
            for (k, (key, name)) in keys.iter().enumerate().skip(2) {
                let e = crate::variable_ui::number_field(
                    app,
                    ui,
                    &ids,
                    key,
                    Id::new(("corner", k)),
                    Lead::Icon(Icon::CornerRadius, name),
                    radii.map(|r| r[k]),
                    "",
                    col,
                );
                app.field(&ids, key, e, |v| json!(v.max(0.0)));
            }
        });
        let sm = common(doc, sel, |n| (n.corners.smoothing * 100.0).round());
        widgets::sublabels(ui, tr("Corner smoothing"), None);
        row(ui, 32.0, |ui| {
            let e = widgets::number_field(ui, Id::new("smoothing"), Lead::None, sm, "%", col, 1.0);
            app.field(&ids, "cornerSmoothing", e, |v| json!((v / 100.0).clamp(0.0, 1.0)));
            widgets::label(ui, "iOS: 60%", Tokens::get(ui.ctx()).text_tertiary);
        });
    }
    let blend = common(doc, sel, |n| n.blend);
    let show_blend = ui.data(|d| d.get_temp::<bool>(Id::new("blend_open"))).unwrap_or(false)
        || blend.is_some_and(|b| !matches!(b, BlendMode::Normal | BlendMode::PassThrough));
    if show_blend {
        widgets::sublabels(ui, tr("Blend mode"), None);
        row(ui, 32.0, |ui| {
            let label = blend.map(|b| tr(b.label())).unwrap_or(tr("Mixed"));
            let resp = widgets::dropdown(ui, label, col * 2.0 + 8.0, Some(Icon::Blend));
            menu_popup(&resp).width(200.0).show(|ui| {
                if let Some(b) = blend_menu(ui, true) {
                    app.set_props(&ids, json!({"blend": b.label()}));
                }
            });
        });
    }
    ui.add_space(8.0);
}

/// The variable modes of the selected frames (sections, components, instances): a row per
/// collection with more than one mode, Auto (the mode the layer inherits from its parent) or a mode
/// of its own (`variable.setMode`). Collections from enabled libraries follow under their
/// library's name (`library.setVariableMode` copies the collection in). Returns whether it showed.
fn variable_modes(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) -> bool {
    if !sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like())) {
        return false;
    }
    let libs = crate::variable_ui::libraries(app, doc);
    let rows = crate::variable_ui::mode_rows(doc, &libs);
    if rows.is_empty() {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    widgets::section_header(ui, tr("Variable modes"), &[], false);
    // The collection's name, then a wider mode picker ("Auto (Light)") across both field columns.
    let col = widgets::col_width(ui.available_width());
    let (name_w, pick_w) = ((col * 0.7).round(), (col * 1.3).round());
    // Auto resolves from the parent of the first selected layer.
    let parent_modes = sel.first().and_then(|i| doc.node(*i)).and_then(|n| n.parent).map(|p| doc.modes_for(p)).unwrap_or_default();
    let mut group = None;
    for r in &rows {
        let c = r.coll;
        // Library collections under their library's name.
        if r.library != group {
            group = r.library;
            if let Some((_, lib)) = r.library {
                widgets::sublabels(ui, lib, None);
            }
        }
        let name_of = |m: &str| c.modes.iter().find(|x| x.id == m).map_or("", |x| x.name.as_str());
        // Each layer's own mode (None: Auto); None overall when they differ.
        let own = match r.local {
            Some(l) => common(doc, sel, |n| n.var_modes.get(l).cloned()),
            None => Some(None),
        };
        let inherited = r.local.and_then(|l| parent_modes.get(l)).or(c.default_mode()).map_or("", |m| name_of(m));
        let auto = trf("Auto ({mode})", &[("mode", &inherited)]);
        let label = match &own {
            Some(Some(m)) => name_of(m).to_string(),
            Some(None) => auto.clone(),
            None => tr("Mixed").into(),
        };
        let hover = match r.library {
            Some((_, lib)) => trf("{name} (from {library})", &[("name", &c.name), ("library", &lib)]),
            None => c.name.clone(),
        };
        row(ui, 32.0, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(name_w, FIELD_H), Sense::hover());
            let name = widgets::elide(ui, &c.name, name_w);
            ui.painter().text(pos2(rect.min.x, rect.center().y), Align2::LEFT_CENTER, &name, regular(11.0), t.text);
            if name != c.name || r.library.is_some() {
                ui.interact(rect, Id::new(("var_mode_name", &c.id)), Sense::hover()).on_hover_text(&hover);
            }
            let resp = widgets::dropdown(ui, &label, pick_w, None);
            menu_popup(&resp).width(180.0).show(|ui| {
                let mut pick = None;
                if menu_check(ui, own == Some(None), &auto, "") {
                    pick = Some(Value::Null);
                }
                ui.separator();
                for m in &c.modes {
                    if menu_check(ui, own.as_ref().is_some_and(|o| o.as_ref() == Some(&m.id)), &m.name, "") {
                        pick = Some(json!(m.id));
                    }
                }
                if let Some(mode) = pick {
                    set_variable_mode(app, sel, r, mode);
                    ui.close();
                }
            });
        });
    }
    ui.add_space(8.0);
    true
}

/// Set `sel`'s mode of the collection of `row` (`mode` null: Auto). A library collection goes
/// through `library.setVariableMode` (copying it in); Auto on one never used here is already so.
fn set_variable_mode(app: &mut FigApp, sel: &[NodeId], row: &crate::variable_ui::ModeRow, mode: Value) {
    match (row.library, row.key, row.local) {
        (Some((lib, _)), Some(key), _) if !mode.is_null() => {
            app.run("library.setVariableMode", json!({"library": lib, "collection": key, "ids": sel, "mode": mode}));
        }
        (_, _, Some(local)) => {
            app.run("variable.setMode", json!({"ids": sel, "collection": local, "mode": mode}));
        }
        _ => {}
    }
}

/// While a range of text is selected, the Fill section edits that range's colour.
fn range_fill(app: &mut FigApp, ui: &mut Ui, doc: &Document, id: NodeId, start: usize, end: usize) {
    let t = Tokens::get(ui.ctx());
    widgets::section_header(ui, tr("Fill"), &[], false);
    // The fills over the range, if every character shares them.
    let fills: Option<Vec<Paint>> = doc.node(id).and_then(|n| n.kind.text().map(|tx| (n, tx))).and_then(|(n, tx)| {
        let mut it =
            tx.spans().into_iter().filter(|(s, e, _)| *s < end && *e > start).map(|(_, _, st)| st.fills.clone().unwrap_or_else(|| n.fills.clone()));
        let first = it.next()?;
        it.all(|f| f == first).then_some(first)
    });
    row(ui, 32.0, |ui| {
        let w = ui.available_width() - 28.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(5), t.field);
        let sw = Rect::from_min_size(pos2(r.min.x + 4.0, r.center().y - 8.0), vec2(16.0, 16.0));
        let text_r = Rect::from_min_max(pos2(sw.max.x + 8.0, r.min.y), pos2(r.max.x - 4.0, r.max.y));
        match fills.as_ref().and_then(|f| f.first()) {
            Some(p) => {
                widgets::paint_swatch(ui.painter(), sw, p);
                if ui.interact(sw.expand(2.0), Id::new("range_swatch"), Sense::click()).clicked() {
                    app.ui.picker = Some(crate::colorpicker::PickerTarget::TextRange { id, start, end });
                }
                if let Some(c) = p.solid_color() {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_r));
                    if let Some(h) =
                        widgets::text_field_inline(&mut child, Id::new("range_hex"), &c.to_hex(), text_r.shrink2(vec2(0.0, 4.0)), regular(11.0))
                        && let Some(nc) = figcraft_color::Color::from_hex_or_name(&h)
                    {
                        app.run("text.style", json!({"id": id, "start": start, "end": end, "textFills": [nc.to_css_hex()]}));
                    }
                } else {
                    ui.painter().text(pos2(text_r.min.x, r.center().y), Align2::LEFT_CENTER, tr("Gradient"), regular(11.0), t.text);
                }
            }
            None => {
                ui.painter().text(pos2(text_r.min.x, r.center().y), Align2::LEFT_CENTER, tr("Mixed"), regular(11.0), t.text_secondary);
                if ui.interact(r, Id::new("range_mixed"), Sense::click()).clicked() {
                    app.ui.picker = Some(crate::colorpicker::PickerTarget::TextRange { id, start, end });
                }
            }
        }
        // Back to the layer's fill.
        if icons::button(ui, Icon::Detach, 24.0, false, tr("Use the layer's fill")).clicked() {
            app.run("text.style", json!({"id": id, "start": start, "end": end, "textFills": null}));
        }
    });
}

fn paints(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId], fills: bool) {
    if fills
        && let Some((id, a, b)) = figcraft_engine::textedit::selected_range(&app.session)
        && a < b
        && sel.contains(&id)
    {
        range_fill(app, ui, doc, id, a, b);
        return;
    }
    let t = Tokens::get(ui.ctx());
    let ids = sel.to_vec();
    let list = common(doc, sel, |n| if fills { n.fills.clone() } else { n.strokes.clone() });
    let title = if fills { tr("Fill") } else { tr("Stroke") };
    let empty = list.as_ref().is_none_or(|l| l.is_empty());
    let slot = if fills { StyleSlot::Fill } else { StyleSlot::Stroke };
    if let Some(i) = widgets::section_header(
        ui,
        title,
        &[(Icon::Styles, tr("Styles")), (Icon::Plus, if fills { tr("Add fill") } else { tr("Add stroke") })],
        empty,
    ) {
        if i == 1 {
            // The new paint goes on top; open its picker. A mixed selection has no one index to edit.
            if app.run(if fills { "fill.add" } else { "stroke.add" }, json!({"ids": ids})).is_some()
                && let Some(l) = &list
            {
                app.ui.picker = Some(crate::colorpicker::PickerTarget::Paint { fills, index: l.len(), ids: ids.clone() });
            }
        } else {
            style_picker::open(app, ui, &ids, slot);
        }
    }
    style_picker::chip(app, ui, doc, &ids, slot);
    match &list {
        None => widgets::note(ui, tr("Click + to replace mixed content"), t.text_secondary),
        Some(l) => {
            // The last paint is on top: shown first. Rows drag to reorder.
            let mut drag = ListDrag::new(ui, if fills { "fills" } else { "strokes" });
            for (i, p) in l.iter().enumerate().rev() {
                let r = paint_row(app, ui, &ids, i, p, fills);
                drag.row(ui, i, r, r, || paint_label(p));
            }
            if let Some((from, to)) = drag.finish(ui) {
                app.run(if fills { "fill.move" } else { "stroke.move" }, json!({"ids": ids, "from": from, "to": to}));
                // An open colour picker keeps editing the paint it was on.
                if let Some(crate::colorpicker::PickerTarget::Paint { fills: f, index, .. }) = &mut app.ui.picker
                    && *f == fills
                {
                    *index = moved_index(*index, from, to);
                }
            }
        }
    }
    if !fills && !empty {
        let weight = common(doc, sel, |n| n.stroke.weight);
        let align = common(doc, sel, |n| n.stroke.align);
        let half = (ui.available_width() - PAD * 2.0 - 8.0) / 2.0;
        row(ui, 32.0, |ui| {
            let resp = widgets::dropdown(
                ui,
                match align {
                    Some(StrokeAlign::Inside) => tr("Inside"),
                    Some(StrokeAlign::Center) => tr("Center"),
                    Some(StrokeAlign::Outside) => tr("Outside"),
                    None => tr("Mixed"),
                },
                half,
                None,
            );
            menu_popup(&resp).show(|ui| {
                for (l, v) in [(tr("Inside"), "inside"), (tr("Center"), "center"), (tr("Outside"), "outside")] {
                    if menu_item(ui, l, "") {
                        app.set_props(&ids, json!({"strokeAlign": v}));
                        ui.close();
                    }
                }
            });
            let sided = sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like() || matches!(n.kind, NodeKind::Rectangle)));
            let weight_w = half - if sided { 60.0 } else { 32.0 };
            let e = crate::variable_ui::number_field_mixed(
                app,
                ui,
                &ids,
                "strokeWeight",
                Id::new("stroke_w"),
                Lead::Icon(Icon::Menu, tr("Stroke weight")),
                weight,
                "",
                weight_w,
            );
            app.field_of(&ids, "strokeWeight", e, |n| n.stroke.weight, |v| json!(v.max(0.0)));
            if sided {
                let sides_btn = icons::button(ui, Icon::IndependentCorners, 24.0, false, tr("Individual strokes"));
                menu_popup(&sides_btn).show(|ui| {
                    let w = weight.unwrap_or(1.0);
                    for (l, v) in [
                        (tr("All"), Value::Null),
                        (tr("Top"), json!([w, 0.0, 0.0, 0.0])),
                        (tr("Right"), json!([0.0, w, 0.0, 0.0])),
                        (tr("Bottom"), json!([0.0, 0.0, w, 0.0])),
                        (tr("Left"), json!([0.0, 0.0, 0.0, w])),
                        (tr("Custom"), json!([w, w, w, w])),
                    ] {
                        if menu_item(ui, l, "") {
                            app.set_props(&ids, json!({"strokeSides": v}));
                            ui.close();
                        }
                    }
                });
            }
            let adv_id = Id::new("stroke_adv");
            let open = ui.data(|d| d.get_temp::<bool>(adv_id)).unwrap_or(false);
            if icons::button(ui, Icon::Dots, 24.0, open, tr("Advanced stroke")).clicked() {
                ui.data_mut(|d| d.insert_temp(adv_id, !open));
            }
        });
        // Per-side weights, two to a row (like the corner radii) so a bound variable's name fits:
        // each side binds to a number variable on its own.
        if let Some(sides) = common(doc, sel, |n| n.stroke.sides).flatten() {
            let col = widgets::col_width(ui.available_width());
            let fields = ["T", "R", "B", "L"].into_iter().zip(figcraft_engine::cmd::props::STROKE_SIDES);
            for first in [0, 2] {
                row(ui, 32.0, |ui| {
                    for (k, (label, field)) in fields.clone().enumerate().skip(first).take(2) {
                        let v = sides.get(k).copied();
                        let e = crate::variable_ui::number_field(app, ui, &ids, field, Id::new(("stroke_side", k)), Lead::Text(label), v, "", col);
                        app.field(&ids, field, e, |x| json!(x.max(0.0)));
                    }
                });
            }
        }
        if ui.data(|d| d.get_temp::<bool>(Id::new("stroke_adv"))).unwrap_or(false) {
            stroke_settings(app, ui, doc, sel);
        }
    }
}

/// A width profile drawn as a filled band across `rect` (thickest point = full height); `None`
/// draws a uniform stroke.
pub(crate) fn profile_art(painter: &egui::Painter, rect: Rect, profile: Option<&WidthProfile>, col: Color32) {
    const STEPS: u32 = 40;
    let max = profile.map_or(1.0, WidthProfile::max_scale).max(1e-6);
    let (cy, half) = (rect.center().y, rect.height() / 2.0);
    let mut mesh = egui::Mesh::default();
    let (mut top, mut bottom) = (Vec::new(), Vec::new());
    for i in 0..=STEPS {
        let t = f64::from(i) / f64::from(STEPS);
        // The path runs left to right: its left side is drawn on top.
        let (l, r) = profile.map_or((0.5, 0.5), |p| {
            let (l, r) = p.sides_at(t);
            (l / max, r / max)
        });
        let x = rect.min.x + rect.width() * t as f32;
        let (hl, hr) = (((l as f32) * half).max(0.5), ((r as f32) * half).max(0.5));
        top.push(pos2(x, cy - hl));
        bottom.push(pos2(x, cy + hr));
        mesh.colored_vertex(pos2(x, cy - hl), col);
        mesh.colored_vertex(pos2(x, cy + hr), col);
    }
    for i in 0..STEPS {
        let k = 2 * i;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
    // The mesh isn't anti-aliased: smooth its edges with hairlines.
    for edge in [top, bottom] {
        painter.add(egui::Shape::line(edge, Stroke::new(0.75, col)));
    }
}

/// Stroke settings ▸ Width profile: the presets (each with its shape) and a flip.
fn width_profile_row(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], profile: Option<Option<WidthProfile>>, half: f32) {
    let t = Tokens::get(ui.ctx());
    widgets::sublabels(ui, tr("Width profile"), None);
    row(ui, 32.0, |ui| {
        let label = match &profile {
            None => tr("Mixed"),
            Some(None) => tr(WidthPreset::Uniform.label()),
            Some(Some(p)) => p.preset().map_or(tr("Custom"), |w| tr(w.label())),
        };
        let resp = widgets::dropdown(ui, label, half * 2.0 + 8.0 - 32.0, None);
        if let Some(p) = &profile {
            let r = resp.rect;
            let art = Rect::from_min_max(pos2(r.max.x - 22.0 - 56.0, r.center().y - 5.0), pos2(r.max.x - 22.0, r.center().y + 5.0));
            profile_art(ui.painter(), art, p.as_ref(), t.icon_secondary);
        }
        menu_popup(&resp).show(|ui| {
            for preset in WidthPreset::ALL {
                let prof = preset.profile();
                let art = |pt: &egui::Painter, r: Rect, col: Color32| {
                    let a = Rect::from_min_max(pos2(r.max.x - 8.0 - 64.0, r.center().y - 5.0), pos2(r.max.x - 8.0, r.center().y + 5.0));
                    profile_art(pt, a, prof.as_ref(), col);
                };
                if menu_item_with(ui, tr(preset.label()), "", 8.0 + 64.0, art) {
                    app.run("stroke.widthProfile", json!({"ids": ids, "preset": preset.id()}));
                    ui.close();
                }
            }
        });
        // Flip: the profile runs from the other end of the path.
        let custom = profile.clone().flatten();
        let flip = icons::button(ui, Icon::FlipH, 24.0, false, tr("Flip width profile"));
        if flip.clicked()
            && let Some(p) = custom
        {
            let pts: Vec<Value> =
                p.points.iter().rev().map(|q| figcraft_engine::cmd::props::width_point_json(&WidthPoint { t: 1.0 - q.t, ..*q })).collect();
            app.run("stroke.widthProfile", json!({"ids": ids, "points": pts}));
        }
    });
}

/// A brush drawn along a short line across `rect`, as outlines (egui fills only convex shapes).
pub(crate) fn brush_art(painter: &egui::Painter, rect: Rect, preset: &str, col: Color32) {
    use figcraft_geom::kurbo::{self, PathEl};
    let mut line = figcraft_geom::BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((f64::from(rect.width()), 0.0));
    let brush = Brush::new(preset);
    let props = StrokeProps { weight: f64::from(rect.height()), align: StrokeAlign::Center, brush: Some(brush.clone()), ..Default::default() };
    let Some(out) = figcraft_pathops::brush::outline(&line, props.weight, &props, &brush) else { return };
    for sub in figcraft_geom::split_subpaths(&out) {
        let mut pts = Vec::new();
        kurbo::flatten(sub.iter(), 0.25, |el| {
            if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
                pts.push(pos2(rect.min.x + p.x as f32, rect.center().y + p.y as f32));
            }
        });
        painter.add(egui::Shape::closed_line(pts, Stroke::new(1.0, col)));
    }
}

/// Stroke settings ▸ Brush: a plain stroke, a stretch brush or a scatter brush, each with its look.
fn brush_row(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], brush: Option<Option<Brush>>, width: f32) {
    use figcraft_pathops::brush::{PRESETS, preset};
    let t = Tokens::get(ui.ctx());
    widgets::sublabels(ui, tr("Brush"), None);
    row(ui, 32.0, |ui| {
        let label = match &brush {
            None => tr("Mixed"),
            Some(b) => b.as_ref().and_then(|b| preset(&b.preset)).map_or(tr("None"), |p| tr(p.label)),
        };
        let resp = widgets::dropdown(ui, label, width, None);
        a11y::combo(&resp, tr("Brush"), label, a11y::menu_open(&resp));
        if let Some(Some(b)) = &brush {
            let r = resp.rect;
            brush_art(
                ui.painter(),
                Rect::from_min_max(pos2(r.max.x - 22.0 - 56.0, r.center().y - 5.0), pos2(r.max.x - 22.0, r.center().y + 5.0)),
                &b.preset,
                t.icon_secondary,
            );
        }
        menu_popup(&resp).show(|ui| {
            if menu_item(ui, tr("None"), "") {
                app.run("stroke.brush", json!({"ids": ids, "preset": null}));
                ui.close();
            }
            for p in PRESETS {
                let art = |pt: &egui::Painter, r: Rect, col: Color32| {
                    brush_art(
                        pt,
                        Rect::from_min_max(pos2(r.max.x - 8.0 - 64.0, r.center().y - 5.0), pos2(r.max.x - 8.0, r.center().y + 5.0)),
                        p.id,
                        col,
                    );
                };
                if menu_item_with(ui, tr(p.label), "", 8.0 + 64.0, art) {
                    app.run("stroke.brush", json!({"ids": ids, "preset": p.id}));
                    ui.close();
                }
            }
        });
    });
    // A scatter brush's spacing and jitter.
    let Some(Some(b)) =
        brush.filter(|b| b.as_ref().and_then(|b| preset(&b.preset)).is_some_and(|p| p.kind == figcraft_pathops::brush::BrushKind::Scatter))
    else {
        return;
    };
    let half = (width - 8.0) / 2.0;
    let with = |f: &dyn Fn(&mut Brush)| {
        let mut nb = b.clone();
        f(&mut nb);
        serde_json::to_value(nb.sanitized()).unwrap_or_default()
    };
    widgets::sublabels(ui, tr("Spacing"), Some(tr("Scatter")));
    row(ui, 32.0, |ui| {
        let e = widgets::number_field(ui, Id::new("brush_spacing"), Lead::Icon(Icon::Gap, tr("Brush spacing")), Some(b.spacing), "×", half, 0.1);
        app.field(ids, "brush", e, |v| with(&|nb| nb.spacing = v));
        let e = widgets::number_field(ui, Id::new("brush_scatter"), Lead::Icon(Icon::Dots, tr("Brush scatter")), Some(b.scatter), "×", half, 0.1);
        app.field(ids, "brush", e, |v| with(&|nb| nb.scatter = v));
    });
    widgets::sublabels(ui, tr("Size jitter"), Some(tr("Rotation jitter")));
    row(ui, 32.0, |ui| {
        let e = widgets::number_field(
            ui,
            Id::new("brush_size_jitter"),
            Lead::Icon(Icon::Scale, tr("Brush size jitter")),
            Some(b.size_jitter * 100.0),
            "%",
            half,
            1.0,
        );
        app.field(ids, "brush", e, |v| with(&|nb| nb.size_jitter = v / 100.0));
        let e = widgets::number_field(
            ui,
            Id::new("brush_rotation_jitter"),
            Lead::Icon(Icon::Rotate, tr("Brush rotation jitter")),
            Some(b.rotation_jitter * 100.0),
            "%",
            half,
            1.0,
        );
        app.field(ids, "brush", e, |v| with(&|nb| nb.rotation_jitter = v / 100.0));
    });
}

/// Advanced stroke settings: dash pattern, width profile, join and the end points of open paths.
fn stroke_settings(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids = sel.to_vec();
    let half = (ui.available_width() - PAD * 2.0 - 8.0) / 2.0;
    let st = common(doc, sel, |n| n.stroke.clone());
    let dashes = st.as_ref().map(|s| s.dashes.clone());
    let weight = st.as_ref().map(|s| s.weight).unwrap_or(1.0).max(1.0);
    widgets::sublabels(ui, tr("Stroke style"), None);
    row(ui, 32.0, |ui| {
        let dashed = dashes.as_ref().is_some_and(|d| !d.is_empty());
        let resp = widgets::dropdown(ui, if dashed { tr("Dash") } else { tr("Solid") }, half, None);
        menu_popup(&resp).show(|ui| {
            if menu_item(ui, tr("Solid"), "") {
                app.set_props(&ids, json!({"dashes": []}));
                ui.close();
            }
            if menu_item(ui, tr("Dash"), "") {
                app.set_props(&ids, json!({"dashes": [weight * 4.0, weight * 4.0]}));
                ui.close();
            }
        });
    });
    if let Some(d) = dashes.filter(|d| !d.is_empty()) {
        let (dash, gap) = (d.first().copied().unwrap_or(4.0), d.get(1).copied().unwrap_or_else(|| d.first().copied().unwrap_or(4.0)));
        widgets::sublabels(ui, tr("Dash"), Some(tr("Gap")));
        row(ui, 32.0, |ui| {
            let e = widgets::number_field(ui, Id::new("stroke_dash"), Lead::None, Some(dash), "", half, 1.0);
            app.field(&ids, "dashes", e, |v| json!([v.max(0.0), gap]));
            let e = widgets::number_field(ui, Id::new("stroke_gap"), Lead::None, Some(gap), "", half, 1.0);
            app.field(&ids, "dashes", e, |v| json!([dash, v.max(0.0)]));
        });
    }
    width_profile_row(app, ui, &ids, st.as_ref().map(|s| s.profile().cloned()), half);
    brush_row(app, ui, &ids, st.as_ref().map(|s| s.brush.clone()), half * 2.0 + 8.0);
    let join = st.as_ref().map(|s| s.join);
    widgets::sublabels(ui, tr("Join|stroke"), (join == Some(StrokeJoin::Miter)).then_some(tr("Miter angle")));
    row(ui, 32.0, |ui| {
        let resp = widgets::dropdown(ui, join.map(join_label).unwrap_or(tr("Mixed")), half, None);
        menu_popup(&resp).show(|ui| {
            for j in StrokeJoin::ALL {
                if menu_item(ui, join_label(j), "") {
                    app.set_props(&ids, json!({"strokeJoin": j}));
                    ui.close();
                }
            }
        });
        if join == Some(StrokeJoin::Miter) {
            let angle = st.as_ref().map(|s| (s.miter_angle() * 100.0).round() / 100.0);
            let e = widgets::number_field(ui, Id::new("stroke_miter"), Lead::None, angle, "°", half, 1.0);
            app.field(&ids, "miterAngle", e, |v| json!(v));
        }
    });
    // End points only mean something on open paths.
    let open_paths = sel.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Line | NodeKind::Vector { .. })));
    if open_paths {
        widgets::sublabels(ui, tr("Start point"), Some(tr("End point")));
        row(ui, 32.0, |ui| {
            for (key, cap) in [("startCap", st.as_ref().map(|s| s.cap_start)), ("endCap", st.as_ref().map(|s| s.cap_end))] {
                let resp = widgets::dropdown(ui, cap.map(|c| tr(c.label())).unwrap_or(tr("Mixed")), half, None);
                menu_popup(&resp).show(|ui| {
                    for c in StrokeCap::ALL {
                        if menu_item_with(ui, tr(c.label()), "", CAP_ART_W, |p, r, col| cap_art(p, r, col, c, key == "startCap")) {
                            app.set_props(&ids, json!({key: format!("{c:?}")}));
                            ui.close();
                        }
                    }
                });
            }
        });
    }
    ui.add_space(4.0);
}

/// A stroke join's name in the interface.
pub(crate) fn join_label(j: StrokeJoin) -> &'static str {
    match j {
        StrokeJoin::Miter => tr("Miter"),
        StrokeJoin::Bevel => tr("Bevel"),
        StrokeJoin::Round => tr("Round"),
    }
}

/// The width [`cap_art`] takes at the right of a menu row (its arrows reach past the line's ends).
pub(crate) const CAP_ART_W: f32 = 64.0;

/// A short stroke ending in `cap` (on its left end for a start point), drawn at the right of a menu
/// row.
pub(crate) fn cap_art(p: &egui::Painter, r: Rect, col: Color32, cap: StrokeCap, start: bool) {
    let y = r.center().y;
    let (left, right) = (r.max.x - 58.0, r.max.x - 14.0);
    // The capped end, the plain end, and the direction from the stroke into its cap.
    let (tip, tail, d) = if start { (left, right, -1.0) } else { (right, left, 1.0) };
    let at = |back: f32, dy: f32| pos2(tip - d * back, y + dy);
    let line = |to: Pos2, w: f32| {
        p.line_segment([pos2(tail, y), to], Stroke::new(w, col));
    };
    let fill = |pts: Vec<Pos2>| {
        p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
    };
    match cap {
        StrokeCap::None => line(at(0.0, 0.0), 4.0),
        StrokeCap::Round => {
            line(at(2.0, 0.0), 4.0);
            p.circle_filled(at(2.0, 0.0), 2.0, col);
        }
        StrokeCap::Square => line(at(-2.0, 0.0), 4.0),
        StrokeCap::LineArrow => {
            line(at(0.0, 0.0), 1.5);
            p.add(egui::Shape::line(vec![at(5.5, -5.5), at(0.0, 0.0), at(5.5, 5.5)], Stroke::new(1.5, col)));
        }
        StrokeCap::TriangleArrow => {
            line(at(7.0, 0.0), 1.5);
            fill(vec![at(0.0, 0.0), at(8.0, -4.5), at(8.0, 4.5)]);
        }
        StrokeCap::ReversedTriangle => {
            line(at(8.0, 0.0), 1.5);
            fill(vec![at(8.0, 0.0), at(0.0, -4.5), at(0.0, 4.5)]);
        }
        StrokeCap::CircleFilled => {
            line(at(4.0, 0.0), 1.5);
            p.circle_filled(at(4.0, 0.0), 4.0, col);
        }
        StrokeCap::DiamondFilled => {
            line(at(5.0, 0.0), 1.5);
            fill(vec![at(0.0, 0.0), at(5.0, -5.0), at(10.0, 0.0), at(5.0, 5.0)]);
        }
    }
}

/// Paint `i`'s row (swatch, colour or kind, opacity, visibility, remove); returns its area.
fn paint_row(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], i: usize, p: &Paint, fills: bool) -> Rect {
    let t = Tokens::get(ui.ctx());
    let ids = ids.to_vec();
    let cmd = if fills { "fill.set" } else { "stroke.set" };
    row_rect(ui, 32.0, |ui| {
        let w = ui.available_width() - 56.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(5), t.field);
        let sw = Rect::from_min_size(pos2(r.min.x + 4.0, r.center().y - 8.0), vec2(16.0, 16.0));
        widgets::paint_swatch(ui.painter(), sw, p);
        let sresp = ui.interact(sw.expand(2.0), Id::new(("swatch", fills, i)), Sense::click());
        if sresp.clicked() {
            app.ui.picker = Some(crate::colorpicker::PickerTarget::Paint { fills, index: i, ids: ids.clone() });
        }
        let text_r = Rect::from_min_max(pos2(sw.max.x + 8.0, r.min.y), pos2(r.max.x - 52.0, r.max.y));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_r));
        // The variable bound to this paint's colour (its key on the layers: `fill` or `fills/<i>/color`).
        let bound = app.session.doc().ok().and_then(|d| {
            let key = ids.first().and_then(|id| d.node(*id)).and_then(|n| figcraft_engine::cmd::lists::paint_binding_key(n, fills, i))?;
            Some((key.to_string(), crate::variable_ui::bound_name(d, &ids, key)?))
        });
        match &p.kind {
            PaintKind::Solid { .. } if bound.is_some() => {
                let (key, name) = bound.clone().unwrap_or_default();
                let chip = Rect::from_min_max(pos2(text_r.min.x - 2.0, r.min.y + 3.0), pos2(text_r.max.x - 20.0, r.max.y - 3.0));
                child.painter().rect_filled(chip, CornerRadius::same(4), t.hover);
                child.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                let short = name.rsplit('/').next().unwrap_or(&name).to_string();
                child.painter().with_clip_rect(chip.shrink2(vec2(5.0, 0.0))).text(
                    pos2(chip.min.x + 5.0, r.center().y),
                    Align2::LEFT_CENTER,
                    short,
                    regular(11.0),
                    t.text,
                );
                let _ = ui.interact(chip, Id::new(("bound_paint", fills, i)), Sense::hover()).on_hover_text(&name);
                let dr = Rect::from_center_size(pos2(text_r.max.x - 9.0, r.center().y), vec2(18.0, 18.0));
                let dresp = ui.interact(dr, Id::new(("detach_paint", fills, i)), Sense::click()).tip(tr("Detach variable"));
                icons::paint(ui.painter(), dr.shrink(3.0), Icon::Detach, t.icon_secondary);
                if dresp.clicked() {
                    app.run("variable.bind", json!({"ids": ids, "field": key, "variable": null}));
                }
            }
            PaintKind::Solid { color } => {
                if let Some(h) =
                    widgets::text_field_inline(&mut child, Id::new(("hex", fills, i)), &color.to_hex(), text_r.shrink2(vec2(0.0, 4.0)), regular(11.0))
                    && let Some(c) = figcraft_color::Color::from_hex_or_name(&h)
                {
                    app.run(cmd, json!({"ids": ids, "index": i, "color": c.to_css_hex()}));
                }
            }
            _ => {
                child.painter().text(pos2(text_r.min.x, r.center().y), Align2::LEFT_CENTER, paint_label(p), regular(11.0), t.text);
            }
        }
        let op_r = Rect::from_min_max(pos2(r.max.x - 48.0, r.min.y), r.max);
        ui.painter().line_segment([pos2(op_r.min.x, r.min.y + 4.0), pos2(op_r.min.x, r.max.y - 4.0)], Stroke::new(1.0, t.panel));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(op_r));
        let e =
            widgets::number_field(&mut child, Id::new(("pop", fills, i)), Lead::None, Some((f64::from(p.opacity) * 100.0).round()), "%", 48.0, 1.0);
        match e {
            Some(FieldEdit::Set(v)) => {
                app.run(cmd, json!({"ids": ids, "index": i, "opacity": (v / 100.0).clamp(0.0, 1.0)}));
            }
            Some(FieldEdit::Scrub(v, done)) => {
                let _ = app.session.begin_interaction("Opacity");
                let _ = app.session.execute(cmd, &json!({"ids": ids, "index": i, "opacity": (v / 100.0).clamp(0.0, 1.0)}));
                if done {
                    let _ = app.session.commit_interaction();
                }
            }
            None => {}
        }
        if icons::button(ui, if p.visible { Icon::Eye } else { Icon::EyeOff }, 24.0, false, tr("Toggle visibility")).clicked() {
            app.run(cmd, json!({"ids": ids, "index": i, "visible": !p.visible}));
        }
        if icons::button(ui, Icon::Minus, 24.0, false, tr("Remove")).clicked()
            && app.run(if fills { "fill.remove" } else { "stroke.remove" }, json!({"ids": ids, "index": i})).is_some()
            && let Some(crate::colorpicker::PickerTarget::Paint { fills: f, index, .. }) = &mut app.ui.picker
            && *f == fills
        {
            // An open colour picker keeps editing the paint it was on, or closes with it.
            match removed_index(*index, i) {
                Some(k) => *index = k,
                None => app.ui.picker = None,
            }
        }
    })
}

/// A paint row's label (also what a drag of it carries): its colour, gradient kind or "Image".
fn paint_label(p: &Paint) -> String {
    match &p.kind {
        PaintKind::Solid { color } => color.to_hex(),
        // The gradient kind's name ("Linear") is a key of its own.
        PaintKind::Gradient(g) => tr(&format!("{:?}", g.kind)).to_string(),
        PaintKind::Image(_) => tr("Image").into(),
        PaintKind::Pattern(_) => tr("Pattern").into(),
    }
}

fn effects(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids = sel.to_vec();
    let list = common(doc, sel, |n| n.effects.clone());
    let empty = list.as_ref().is_none_or(|l| l.is_empty());
    if let Some(i) = widgets::section_header(ui, tr("Effects"), &[(Icon::Styles, tr("Styles")), (Icon::Plus, tr("Add effect"))], empty) {
        if i == 1 {
            app.run("effect.add", json!({"ids": ids, "type": "dropShadow"}));
        } else {
            style_picker::open(app, ui, &ids, StyleSlot::Effect);
        }
    }
    style_picker::chip(app, ui, doc, &ids, StyleSlot::Effect);
    let Some(list) = list else { return };
    let mut drag = ListDrag::new(ui, "effects");
    let mut removed = None;
    for (i, e) in list.iter().enumerate().rev() {
        let mut open = effect_open(ui.ctx(), i);
        let header = row_rect(ui, 32.0, |ui| {
            if icons::button(ui, Icon::Effects, 24.0, open, tr("Effect settings")).clicked() {
                open = !open;
                ui.data_mut(|d| d.insert_temp(Id::new((FX_OPEN, i)), open));
            }
            let w = ui.available_width() - 56.0;
            let resp = widgets::dropdown(ui, tr(e.label()), w, None);
            menu_popup(&resp).show(|ui| {
                for (l, ty) in Effect::TYPES {
                    if menu_item(ui, tr(l), "")
                        && let Some(ne) = Effect::of_type(ty)
                    {
                        app.run("effect.set", json!({"ids": ids, "index": i, "effect": ne}));
                        ui.close();
                    }
                }
            });
            if icons::button(ui, if e.visible { Icon::Eye } else { Icon::EyeOff }, 24.0, false, tr("Toggle")).clicked() {
                let mut e2 = e.clone();
                e2.visible = !e2.visible;
                app.run("effect.set", json!({"ids": ids, "index": i, "effect": e2}));
            }
            if icons::button(ui, Icon::Minus, 24.0, false, tr("Remove")).clicked()
                && app.run("effect.remove", json!({"ids": ids, "index": i})).is_some()
            {
                removed = Some(i);
            }
        });
        if open {
            effect_settings(app, ui, &ids, i, e);
        }
        drag.row(ui, i, header, header.with_max_y(ui.cursor().min.y.max(header.max.y)), || tr(e.label()).to_string());
    }
    if let Some((from, to)) = drag.finish(ui) {
        app.run("effect.move", json!({"ids": ids, "from": from, "to": to}));
        move_flags(ui, FX_OPEN, list.len(), from, to);
        if let Some(crate::colorpicker::PickerTarget::Effect { index, .. }) = &mut app.ui.picker {
            *index = moved_index(*index, from, to);
        }
    }
    // Open settings and an open colour picker stay with their effects, or go with theirs.
    if let Some(i) = removed {
        remove_flags(ui, FX_OPEN, list.len(), i);
        if let Some(crate::colorpicker::PickerTarget::Effect { index, .. }) = &mut app.ui.picker {
            match removed_index(*index, i) {
                Some(k) => *index = k,
                None => app.ui.picker = None,
            }
        }
    }
}

/// Salt of the flags that keep effect rows' settings open.
const FX_OPEN: &str = "fx_open";

/// Whether effect `i`'s settings are open in the Design panel.
pub(crate) fn effect_open(ctx: &egui::Context, i: usize) -> bool {
    ctx.data(|d| d.get_temp::<bool>(Id::new((FX_OPEN, i)))).unwrap_or(false)
}

/// A number field of effect `i` that can be bound to a variable (`effects/<i>/<prop>`).
fn fx_field(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], i: usize, prop: &str, lead: Lead, value: f64, width: f32) -> Option<FieldEdit> {
    crate::variable_ui::number_field(app, ui, ids, &format!("effects/{i}/{prop}"), Id::new(("fx", prop, i)), lead, Some(value), "", width)
}

fn effect_settings(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], i: usize, e: &Effect) {
    let half = (ui.available_width() - PAD * 2.0 - 8.0) / 2.0;
    let set = |app: &mut FigApp, e2: Effect, scrub: Option<bool>| {
        let p = json!({"ids": ids, "index": i, "effect": e2});
        match scrub {
            None => {
                app.run("effect.set", p);
            }
            Some(done) => {
                let _ = app.session.begin_interaction("Effect");
                let _ = app.session.execute("effect.set", &p);
                if done {
                    let _ = app.session.commit_interaction();
                }
            }
        }
    };
    let edit = |app: &mut FigApp, ed: Option<FieldEdit>, f: &dyn Fn(f64) -> Effect| match ed {
        Some(FieldEdit::Set(v)) => set(app, f(v), None),
        Some(FieldEdit::Scrub(v, d)) => set(app, f(v), Some(d)),
        None => {}
    };
    // A new colour for the effect (`secondary`: a duo noise's second colour).
    let edit_color = |app: &mut FigApp, secondary: bool, c: figcraft_color::Color, scrub: Option<bool>| {
        let mut e2 = e.clone();
        if let Some(slot) = e2.color_mut(secondary) {
            *slot = c;
        }
        set(app, e2, scrub);
    };
    match &e.kind {
        EffectKind::DropShadow(s) | EffectKind::InnerShadow(s) => {
            let inner = matches!(e.kind, EffectKind::InnerShadow(_));
            let mk = |s2: Shadow| Effect { kind: if inner { EffectKind::InnerShadow(s2) } else { EffectKind::DropShadow(s2) }, visible: e.visible };
            row(ui, 32.0, |ui| {
                let ed = fx_field(app, ui, ids, i, "offsetX", Lead::Text("X"), s.offset.0, half);
                edit(app, ed, &|v| mk(Shadow { offset: (v, s.offset.1), ..s.clone() }));
                let ed = fx_field(app, ui, ids, i, "offsetY", Lead::Text("Y"), s.offset.1, half);
                edit(app, ed, &|v| mk(Shadow { offset: (s.offset.0, v), ..s.clone() }));
            });
            row(ui, 32.0, |ui| {
                let ed = fx_field(app, ui, ids, i, "radius", Lead::Text(tr("Blur")), s.radius, half);
                edit(app, ed, &|v| mk(Shadow { radius: v.max(0.0), ..s.clone() }));
                let ed = fx_field(app, ui, ids, i, "spread", Lead::Text(tr("Spread")), s.spread, half);
                edit(app, ed, &|v| mk(Shadow { spread: v, ..s.clone() }));
            });
            // The colour: a bound variable's chip, else swatch, hex and alpha with a variable button.
            let color_field = format!("effects/{i}/color");
            row(ui, 32.0, |ui| {
                crate::variable_ui::bindable(app, ui, ids, &color_field, VariableType::Color, half * 2.0 + 8.0, |app, ui| {
                    fx_color(app, ui, ids, i, e, false, &edit_color)
                });
            });
            if !inner {
                row(ui, 32.0, |ui| {
                    if widgets::checkbox(ui, s.show_behind, tr("Show behind transparent areas")) {
                        set(app, mk(Shadow { show_behind: !s.show_behind, ..s.clone() }), None);
                    }
                });
            }
        }
        EffectKind::LayerBlur(b) | EffectKind::BackgroundBlur(b) => {
            let with = |f: &dyn Fn(&mut Blur)| {
                let mut e2 = e.clone();
                if let Some(b2) = e2.blur_mut() {
                    f(b2);
                }
                e2
            };
            row(ui, 32.0, |ui| {
                let resp = widgets::dropdown(ui, tr(b.blur_type.label()), half * 2.0 + 8.0, None);
                menu_popup(&resp).show(|ui| {
                    for t in BlurType::ALL {
                        if menu_item(ui, tr(t.label()), "") {
                            set(app, with(&|b2| b2.blur_type = t), None);
                            ui.close();
                        }
                    }
                });
            });
            row(ui, 32.0, |ui| {
                if b.is_progressive() {
                    // The ramp's start and end points are dragged on the canvas.
                    let ed = fx_field(app, ui, ids, i, "startRadius", Lead::Text(tr("Start")), b.start_radius, half);
                    edit(app, ed, &|v| with(&|b2| b2.start_radius = v.clamp(0.0, MAX_BLUR_RADIUS)));
                    let ed = fx_field(app, ui, ids, i, "radius", Lead::Text(tr("End")), b.radius, half);
                    edit(app, ed, &|v| with(&|b2| b2.radius = v.clamp(0.0, MAX_BLUR_RADIUS)));
                } else {
                    let ed = fx_field(app, ui, ids, i, "radius", Lead::Text(tr("Blur")), b.radius, half * 2.0 + 8.0);
                    edit(app, ed, &|v| with(&|b2| b2.radius = v.clamp(0.0, MAX_BLUR_RADIUS)));
                }
            });
        }
        EffectKind::Noise(nz) => {
            let with = |f: &dyn Fn(&mut Noise)| {
                tweaked(e, |k| {
                    if let EffectKind::Noise(n) = k {
                        f(n);
                    }
                })
            };
            row(ui, 32.0, |ui| {
                let resp = widgets::dropdown(ui, tr(nz.mode.label()), half * 2.0 + 8.0, None);
                menu_popup(&resp).show(|ui| {
                    for m in NoiseMode::ALL {
                        if menu_item(ui, tr(m.label()), "") {
                            set(app, with(&|n| n.mode = m), None);
                            ui.close();
                        }
                    }
                });
            });
            row(ui, 32.0, |ui| {
                let ed = widgets::number_field(ui, Id::new(("fx_ns", i)), Lead::Text(tr("Size")), Some(nz.size), "", half, 0.1);
                edit(app, ed, &|v| {
                    with(&|n| {
                        n.size = v.clamp(0.01, 1000.0);
                        n.size_y = None;
                    })
                });
                let ed =
                    widgets::number_field(ui, Id::new(("fx_nd", i)), Lead::Text(tr("Density")), Some((nz.density * 100.0).round()), "%", half, 1.0);
                edit(app, ed, &|v| with(&|n| n.density = (v / 100.0).clamp(0.0, 1.0)));
            });
            match nz.mode {
                NoiseMode::Mono => row(ui, 32.0, |ui| fx_color(app, ui, ids, i, e, false, &edit_color)),
                NoiseMode::Duo => {
                    row(ui, 32.0, |ui| fx_color(app, ui, ids, i, e, false, &edit_color));
                    row(ui, 32.0, |ui| fx_color(app, ui, ids, i, e, true, &edit_color));
                }
                NoiseMode::Multi => row(ui, 32.0, |ui| {
                    let op = Some((f64::from(nz.opacity) * 100.0).round());
                    let ed = widgets::number_field(ui, Id::new(("fx_no", i)), Lead::Text(tr("Opacity")), op, "%", half, 1.0);
                    edit(app, ed, &|v| with(&|n| n.opacity = (v / 100.0).clamp(0.0, 1.0) as f32));
                }),
            }
        }
        EffectKind::Texture(tx) => {
            let with = |f: &dyn Fn(&mut Texture)| {
                tweaked(e, |k| {
                    if let EffectKind::Texture(t) = k {
                        f(t);
                    }
                })
            };
            row(ui, 32.0, |ui| {
                let ed = widgets::number_field(ui, Id::new(("fx_ts", i)), Lead::Text(tr("Size")), Some(tx.size), "", half, 0.1);
                edit(app, ed, &|v| {
                    with(&|t| {
                        t.size = v.clamp(0.01, 1000.0);
                        t.size_y = None;
                    })
                });
                let ed = widgets::number_field(ui, Id::new(("fx_tr", i)), Lead::Text(tr("Radius")), Some(tx.radius), "", half, 1.0);
                edit(app, ed, &|v| with(&|t| t.radius = v.max(0.0)));
            });
            row(ui, 32.0, |ui| {
                if widgets::checkbox(ui, tx.clip_to_shape, tr("Clip to shape")) {
                    set(app, with(&|t| t.clip_to_shape = !tx.clip_to_shape), None);
                }
            });
        }
        EffectKind::Glass(g) => {
            let with = |f: &dyn Fn(&mut Glass)| {
                tweaked(e, |k| {
                    if let EffectKind::Glass(gl) = k {
                        f(gl);
                    }
                })
            };
            let pct = |v: f64| Some((v * 100.0).round());
            let unit = |v: f64| (v / 100.0).clamp(0.0, 1.0);
            row(ui, 32.0, |ui| {
                let ed = widgets::number_field(ui, Id::new(("fx_gl", i)), Lead::Text(tr("Light|glass")), Some(g.light_angle.round()), "°", half, 1.0);
                edit(app, ed, &|v| with(&|gl| gl.light_angle = v.clamp(-180.0, 180.0)));
                let ed = widgets::number_field(ui, Id::new(("fx_gi", i)), Lead::None, pct(g.light_intensity), "%", half, 1.0);
                edit(app, ed, &|v| with(&|gl| gl.light_intensity = unit(v)));
            });
            row(ui, 32.0, |ui| {
                let ed = widgets::number_field(ui, Id::new(("fx_gr", i)), Lead::Text(tr("Refraction")), pct(g.refraction), "%", half, 1.0);
                edit(app, ed, &|v| with(&|gl| gl.refraction = unit(v)));
                let ed = widgets::number_field(ui, Id::new(("fx_gd", i)), Lead::Text(tr("Depth")), Some(g.depth), "", half, 1.0);
                edit(app, ed, &|v| with(&|gl| gl.depth = v.clamp(1.0, 1000.0)));
            });
            row(ui, 32.0, |ui| {
                let ed = widgets::number_field(ui, Id::new(("fx_gs", i)), Lead::Text(tr("Dispersion")), pct(g.dispersion), "%", half, 1.0);
                edit(app, ed, &|v| with(&|gl| gl.dispersion = unit(v)));
                let ed = widgets::number_field(ui, Id::new(("fx_gf", i)), Lead::Text(tr("Frost")), Some(g.frost), "", half, 1.0);
                edit(app, ed, &|v| with(&|gl| gl.frost = v.clamp(0.0, 1000.0)));
            });
            row(ui, 32.0, |ui| {
                let ed = widgets::number_field(ui, Id::new(("fx_gp", i)), Lead::Text(tr("Splay")), pct(g.splay), "%", half, 1.0);
                edit(app, ed, &|v| with(&|gl| gl.splay = unit(v)));
            });
        }
    }
}

/// Applies a new colour to an effect: (app, secondary colour, colour, scrubbing: `Some(done)`).
type ColorEdit<'a> = dyn Fn(&mut FigApp, bool, figcraft_color::Color, Option<bool>) + 'a;

/// `e` with `f` applied to its kind.
fn tweaked(e: &Effect, f: impl FnOnce(&mut EffectKind)) -> Effect {
    let mut e2 = e.clone();
    f(&mut e2.kind);
    e2
}

/// The swatch (opens the colour picker), hex and opacity of effect `i`'s colour (`secondary`: a
/// duo noise's second colour), in the current row; `edit` applies a new one.
fn fx_color(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], i: usize, e: &Effect, secondary: bool, edit: &ColorEdit<'_>) {
    let Some(c) = e.color(secondary) else { return };
    if widgets::swatch(ui, &Paint::solid(c), 16.0).clicked() {
        app.ui.picker = Some(crate::colorpicker::PickerTarget::Effect { ids: ids.to_vec(), index: i, secondary });
    }
    if let Some(h) = widgets::text_field(ui, Id::new(("fx_c", i, secondary)), &c.to_hex(), 80.0, "")
        && let Some(nc) = figcraft_color::Color::from_hex_or_name(&h)
    {
        edit(app, secondary, nc.with_alpha(c.a), None);
    }
    let alpha = |v: f64| c.with_alpha((v / 100.0).clamp(0.0, 1.0) as f32);
    match widgets::number_field(ui, Id::new(("fx_a", i, secondary)), Lead::None, Some((f64::from(c.a) * 100.0).round()), "%", 56.0, 1.0) {
        Some(FieldEdit::Set(v)) => edit(app, secondary, alpha(v), None),
        Some(FieldEdit::Scrub(v, done)) => edit(app, secondary, alpha(v), Some(done)),
        None => {}
    }
}

fn grids(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids = sel.to_vec();
    let list = common(doc, sel, |n| n.grids.clone());
    let empty = list.as_ref().is_none_or(|l| l.is_empty());
    if let Some(i) = widgets::section_header(ui, tr("Layout grid"), &[(Icon::Styles, tr("Styles")), (Icon::Plus, tr("Add layout grid"))], empty) {
        if i == 1 {
            app.run("grid.add", json!({"ids": ids}));
        } else {
            style_picker::open(app, ui, &ids, StyleSlot::Grid);
        }
    }
    style_picker::chip(app, ui, doc, &ids, StyleSlot::Grid);
    let Some(list) = list else { return };
    // The last grid first, like paints and effects; rows drag to reorder.
    let mut drag = ListDrag::new(ui, "grids");
    let mut removed = None;
    for (i, g) in list.iter().enumerate().rev() {
        let open_id = Id::new(("grid_open", i));
        let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
        let header = row_rect(ui, 32.0, |ui| {
            if icons::button(ui, Icon::Grid, 24.0, open, tr("Layout grid settings")).clicked() {
                open = !open;
                ui.data_mut(|d| d.insert_temp(open_id, open));
            }
            let w = ui.available_width() - 56.0;
            let resp = widgets::dropdown(ui, &grid_label(g), w, None);
            menu_popup(&resp).width(220.0).show(|ui| {
                for (l, kind) in [(tr("Grid"), GridKind::Grid), (tr("Columns"), GridKind::Columns), (tr("Rows"), GridKind::Rows)] {
                    if menu_item(ui, l, "") {
                        let fresh = match kind {
                            GridKind::Grid => LayoutGrid::default(),
                            GridKind::Columns => LayoutGrid::columns(),
                            GridKind::Rows => LayoutGrid::rows(),
                        };
                        app.set_props(&ids, json!({"grids": grids_with(&list, i, |x| *x = fresh)}));
                        ui.close();
                    }
                }
            });
            if icons::button(ui, if g.visible { Icon::Eye } else { Icon::EyeOff }, 24.0, false, tr("Toggle")).clicked() {
                app.set_props(&ids, json!({"grids": grids_with(&list, i, |x| x.visible = !x.visible)}));
            }
            if icons::button(ui, Icon::Minus, 24.0, false, tr("Remove")).clicked()
                && app.run("grid.remove", json!({"ids": ids, "index": i})).is_some()
            {
                removed = Some(i);
            }
        });
        if open {
            grid_settings(app, ui, &ids, &list, i, g);
        }
        drag.row(ui, i, header, header.with_max_y(ui.cursor().min.y.max(header.max.y)), || grid_label(g));
    }
    if let Some((from, to)) = drag.finish(ui) {
        app.run("grid.move", json!({"ids": ids, "from": from, "to": to}));
        move_flags(ui, "grid_open", list.len(), from, to);
    }
    // Open settings stay with their grids, or go with theirs.
    if let Some(i) = removed {
        remove_flags(ui, "grid_open", list.len(), i);
    }
}

/// A layout grid's name in its row ("Grid (10)", "5 columns").
fn grid_label(g: &LayoutGrid) -> String {
    match g.kind {
        GridKind::Grid => trf("Grid ({size})", &[("size", &g.size)]),
        GridKind::Columns if auto_count(g) => tr("Auto columns").to_string(),
        GridKind::Rows if auto_count(g) => tr("Auto rows").to_string(),
        GridKind::Columns => trn("{n} columns", g.count.max(1) as usize, &[]),
        GridKind::Rows => trn("{n} rows", g.count.max(1) as usize, &[]),
    }
}

/// Fixed-size columns or rows counted Auto: as many as fit.
fn auto_count(g: &LayoutGrid) -> bool {
    g.count == 0 && g.align != GridAlign::Stretch
}

/// `list` with grid `i` changed by `f`.
fn grids_with(list: &[LayoutGrid], i: usize, f: impl FnOnce(&mut LayoutGrid)) -> Vec<LayoutGrid> {
    let mut gs = list.to_vec();
    if let Some(g) = gs.get_mut(i) {
        f(g);
    }
    gs
}

/// Layout grid `i`'s settings: size (grids); count, type, width or height, margin or offset and
/// gutter (columns and rows); colour. The numbers can be bound to number variables
/// (`layoutGrids/<i>/<size|count|gutter|offset>`).
fn grid_settings(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], list: &[LayoutGrid], i: usize, g: &LayoutGrid) {
    let col = widgets::col_width(ui.available_width());
    let set = |f: &dyn Fn(&mut LayoutGrid)| json!({"ids": ids, "props": {"grids": grids_with(list, i, f)}});
    // One bindable number field, applied with `apply` (a scrub is one undo step).
    let field = |app: &mut FigApp, ui: &mut Ui, name: &str, value: f64, apply: &dyn Fn(&mut LayoutGrid, f64)| {
        let key = format!("layoutGrids/{i}/{name}");
        let e = crate::variable_ui::number_field(app, ui, ids, &key, Id::new(("grid_field", name, i)), Lead::None, Some(value), "", col);
        app.field_run(e, "node.set", |v| set(&|x| apply(x, v)));
    };
    let size = |x: &mut LayoutGrid, v: f64| x.size = v.max(1.0);
    let offset = |x: &mut LayoutGrid, v: f64| x.offset = v.max(0.0);
    let gutter = |x: &mut LayoutGrid, v: f64| x.gutter = v.max(0.0);
    if g.kind == GridKind::Grid {
        widgets::sublabels(ui, tr("Size"), None);
        row(ui, 32.0, |ui| field(app, ui, "size", g.size, &size));
    } else {
        let cols = g.kind == GridKind::Columns;
        widgets::sublabels(ui, tr("Count"), Some(tr("Type")));
        row(ui, 32.0, |ui| {
            // 0 is Auto for fixed-size columns (rows); a stretched grid has at least one.
            let count = (!auto_count(g)).then(|| f64::from(g.count.max(1)));
            let key = format!("layoutGrids/{i}/count");
            let e =
                crate::variable_ui::number_field_or(app, ui, ids, &key, Id::new(("grid_field", "count", i)), count, col, tr("Auto"), Lead::None, "");
            app.field_run(e, "node.set", |v| {
                set(&|x| x.count = if x.align == GridAlign::Stretch { v.round().clamp(1.0, 1000.0) } else { v.round().clamp(0.0, 1000.0) } as u32)
            });
            let names =
                if cols { [tr("Stretch"), tr("Left"), tr("Center"), tr("Right")] } else { [tr("Stretch"), tr("Top"), tr("Center"), tr("Bottom")] };
            let aligns = [GridAlign::Stretch, GridAlign::Min, GridAlign::Center, GridAlign::Max];
            let cur = aligns.iter().position(|a| *a == g.align).unwrap_or(0);
            let resp = widgets::dropdown(ui, names.get(cur).copied().unwrap_or_default(), col, None);
            menu_popup(&resp).show(|ui| {
                for (k, (name, a)) in names.iter().zip(aligns).enumerate() {
                    if menu_check(ui, k == cur, name, "") {
                        // Fixed-size columns need a size of their own.
                        app.run(
                            "node.set",
                            set(&|x| {
                                x.align = a;
                                if a != GridAlign::Stretch && !(x.size.is_finite() && x.size > 0.0) {
                                    x.size = 60.0;
                                }
                                if a == GridAlign::Stretch {
                                    x.count = x.count.max(1);
                                }
                            }),
                        );
                        ui.close();
                    }
                }
            });
        });
        let span = if cols { tr("Width") } else { tr("Height") };
        match g.align {
            GridAlign::Stretch => {
                widgets::sublabels(ui, tr("Margin"), Some(tr("Gutter")));
                row(ui, 32.0, |ui| {
                    field(app, ui, "offset", g.offset, &offset);
                    field(app, ui, "gutter", g.gutter, &gutter);
                });
            }
            GridAlign::Center => {
                widgets::sublabels(ui, span, Some(tr("Gutter")));
                row(ui, 32.0, |ui| {
                    field(app, ui, "size", g.size, &size);
                    field(app, ui, "gutter", g.gutter, &gutter);
                });
            }
            GridAlign::Min | GridAlign::Max => {
                widgets::sublabels(ui, span, Some(tr("Offset")));
                row(ui, 32.0, |ui| {
                    field(app, ui, "size", g.size, &size);
                    field(app, ui, "offset", g.offset, &offset);
                });
                widgets::sublabels(ui, tr("Gutter"), None);
                row(ui, 32.0, |ui| field(app, ui, "gutter", g.gutter, &gutter));
            }
        }
    }
    // The colour: swatch, hex and alpha.
    widgets::sublabels(ui, tr("Color"), None);
    row(ui, 32.0, |ui| {
        let (sw, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        widgets::paint_swatch(ui.painter(), sw, &Paint::solid(g.color));
        if let Some(h) = widgets::text_field(ui, Id::new(("grid_hex", i)), &g.color.to_hex(), 80.0, "")
            && let Some(c) = figcraft_color::Color::from_hex_or_name(&h)
        {
            app.run("node.set", set(&|x| x.color = c.with_alpha(x.color.a)));
        }
        let e = widgets::number_field(ui, Id::new(("grid_alpha", i)), Lead::None, Some((f64::from(g.color.a) * 100.0).round()), "%", 56.0, 1.0);
        app.field_run(e, "node.set", |v| set(&|x| x.color = x.color.with_alpha((v / 100.0).clamp(0.0, 1.0) as f32)));
    });
}

/// One group of inspected properties: label, value (with a swatch for colours) and the style or
/// variable it comes from. Clicking a row copies its value.
pub(crate) fn inspect_section(app: &mut FigApp, ui: &mut Ui, s: &figcraft_engine::devinspect::Section) {
    let t = Tokens::get(ui.ctx());
    widgets::separator(ui);
    let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    ui.painter().text(pos2(hr.min.x + PAD, hr.center().y), Align2::LEFT_CENTER, tr(s.title), semibold(11.0), t.text);
    for p in &s.rows {
        prop_row(app, ui, p);
    }
    ui.add_space(4.0);
}

/// An inspected property row: label, value (with a swatch for colours) and the style or variable
/// it comes from as a chip. Clicking it copies the value.
pub(crate) fn prop_row(app: &mut FigApp, ui: &mut Ui, p: &figcraft_engine::devinspect::Prop) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let resp = resp.tip(trf("Copy {value}", &[("value", &p.value)]));
    if resp.hovered() {
        ui.painter().rect_filled(r.shrink2(vec2(8.0, 0.0)), CornerRadius::same(5), t.hover);
    }
    let painter = ui.painter();
    // The engine's property labels ("Width") are keys of their own.
    let label = tr(&p.label);
    // Elided before the value column (some languages' labels run longer).
    painter.text(
        pos2(r.min.x + PAD, r.center().y),
        Align2::LEFT_CENTER,
        widgets::elide(ui, label, 96.0 - PAD - 6.0),
        regular(11.0),
        t.text_secondary,
    );
    // The style or variable, right-aligned as a chip.
    let mut right = r.max.x - PAD;
    if let Some(token) = &p.token {
        let galley = painter.layout_no_wrap(token.clone(), regular(10.0), t.text_secondary);
        let chip =
            Rect::from_min_max(pos2((right - galley.size().x - 12.0).max(r.min.x + 196.0), r.center().y - 9.0), pos2(right, r.center().y + 9.0));
        painter.rect_filled(chip, CornerRadius::same(4), t.field);
        painter.with_clip_rect(chip.shrink2(vec2(6.0, 0.0))).galley(
            pos2(chip.min.x + 6.0, chip.center().y - galley.size().y / 2.0),
            galley,
            t.text_secondary,
        );
        right = chip.min.x - 6.0;
    }
    let mut x = r.min.x + 96.0;
    if let Some(c) = p.color.as_deref().and_then(figcraft_color::Color::from_hex) {
        let sw = Rect::from_min_size(pos2(x, r.center().y - 6.0), vec2(12.0, 12.0));
        widgets::paint_swatch(painter, sw, &Paint::solid(c));
        x += 18.0;
    }
    let value_r = Rect::from_min_max(pos2(x, r.min.y), pos2(right.max(x + 10.0), r.max.y));
    painter.with_clip_rect(value_r).text(pos2(x, r.center().y), Align2::LEFT_CENTER, &p.value, regular(11.0), t.text);
    if resp.clicked() {
        app.services.clipboard_set_text(&p.value);
        app.toast(trf("Copied {value}", &[("value", &label.to_lowercase())]));
    }
}

/// The selection's assets (layers with export settings, icons, images): click one to export it,
/// or export them all.
fn inspect_assets(app: &mut FigApp, ui: &mut Ui, assets: &[figcraft_engine::devinspect::Asset]) {
    let t = Tokens::get(ui.ctx());
    widgets::separator(ui);
    let export_all = [(Icon::Export, tr("Export all"))];
    let all = widgets::section_header(ui, tr("Assets"), if assets.is_empty() { &[] } else { &export_all }, false).is_some();
    if assets.is_empty() {
        widgets::note(ui, tr("No assets: no export settings, icons or images. Add export settings in the Design tab."), t.text_secondary);
        return;
    }
    let mut export: Option<Vec<NodeId>> = all.then(|| assets.iter().map(|a| a.id).collect());
    for a in assets {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
        let resp = resp.tip(trf("Export {name}", &[("name", &a.name)]));
        if resp.hovered() {
            ui.painter().rect_filled(r.shrink2(vec2(8.0, 0.0)), CornerRadius::same(5), t.hover);
            icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - PAD - 6.0, r.center().y), vec2(12.0, 12.0)), Icon::Export, t.icon);
        }
        let name_r = Rect::from_min_max(pos2(r.min.x + PAD, r.min.y), pos2(r.min.x + 96.0 - 4.0, r.max.y));
        ui.painter().with_clip_rect(name_r).text(pos2(name_r.min.x, r.center().y), Align2::LEFT_CENTER, &a.name, regular(11.0), t.text);
        let fmt_r = Rect::from_min_max(pos2(r.min.x + 96.0, r.min.y), pos2(r.max.x - PAD - 16.0, r.max.y));
        ui.painter().with_clip_rect(fmt_r).text(
            pos2(fmt_r.min.x, r.center().y),
            Align2::LEFT_CENTER,
            a.exports.join(", "),
            regular(11.0),
            t.text_secondary,
        );
        if resp.clicked() {
            export = Some(vec![a.id]);
        }
    }
    if let Some(ids) = export {
        crate::export_panel::export_layers(app, &ids, true);
    }
    ui.add_space(4.0);
}

/// The Inspect tab (Dev Mode): the handoff status, annotations, properties, assets and code of
/// the selected layer, or the list of what's ready for dev.
fn inspect(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Ok(ds) = app.session.ds() else { return };
    let doc = ds.doc.clone();
    let sel = ds.selection.clone();
    let Some(first) = sel.first().copied() else {
        widgets::section_header(ui, tr("Inspect"), &[], false);
        widgets::note(ui, tr("Select a layer to see its properties and code"), t.text_secondary);
        widgets::note(ui, tr("Hover other layers to measure the distance"), t.text_secondary);
        crate::dev_mode::ready_list(app, ui);
        crate::dev_mode::categories_section(app, ui);
        crate::dev_mode::plugins_section(app, ui);
        return;
    };
    let name = doc.node(first).map(|n| n.name.clone()).unwrap_or_default();
    widgets::section_header(ui, &name, &[], false);
    crate::dev_mode::status(app, ui, &doc, &sel);
    crate::dev_mode::annotations_section(app, ui, &doc, first);
    crate::dev_mode::dev_resources_section(app, ui, &doc, first);
    use figcraft_engine::devinspect::ColorFormat;
    let fmt_id = Id::new("inspect_color_format");
    let fmt = ui.data(|d| d.get_temp::<usize>(fmt_id)).and_then(|i| ColorFormat::ALL.get(i).copied()).unwrap_or_default();
    row(ui, 32.0, |ui| {
        widgets::label(ui, tr("Color format"), t.text_secondary);
        let resp = widgets::dropdown(ui, fmt.label(), 72.0, None);
        menu_popup(&resp).show(|ui| {
            for (i, f) in ColorFormat::ALL.iter().enumerate() {
                if menu_item(ui, f.label(), "") {
                    ui.data_mut(|d| d.insert_temp(fmt_id, i));
                    ui.close();
                }
            }
        });
    });
    for s in figcraft_engine::devinspect::properties_with(&doc, first, fmt) {
        inspect_section(app, ui, &s);
    }
    inspect_assets(app, ui, &figcraft_engine::devinspect::assets(&doc, &sel));
    use figcraft_engine::codegen::CodeLang;
    let lang_id = Id::new("inspect_lang");
    let lang = ui.data(|d| d.get_temp::<usize>(lang_id)).and_then(|i| CodeLang::ALL.get(i).copied()).unwrap_or_default();
    // A component tried with other property values: its code replaces the layer's below.
    let played = crate::dev_mode::playground_section(app, ui, &doc, first, lang);
    widgets::separator(ui);
    let copy = widgets::section_header(ui, tr("Code"), &[(Icon::Export, tr("Copy"))], false).is_some();
    // Pixels or rem (of a 16 px root) for the web languages.
    let rem_id = Id::new("inspect_rem");
    let web = lang.is_web();
    let rem = web && ui.data(|d| d.get_temp::<bool>(rem_id)).unwrap_or(false);
    row(ui, 32.0, |ui| {
        let resp = widgets::dropdown(ui, lang.label(), 120.0, Some(Icon::Code));
        menu_popup(&resp).show(|ui| {
            for (i, l) in CodeLang::ALL.iter().enumerate() {
                if menu_item(ui, l.label(), "") {
                    ui.data_mut(|d| d.insert_temp(lang_id, i));
                    ui.close();
                }
            }
        });
        if web {
            let resp = widgets::dropdown(ui, if rem { "rem" } else { "px" }, 56.0, None);
            menu_popup(&resp).show(|ui| {
                for (on, label) in [(false, "px"), (true, "rem")] {
                    if menu_item(ui, label, "") {
                        ui.data_mut(|d| d.insert_temp(rem_id, on));
                        ui.close();
                    }
                }
            });
        }
    });
    let code = played.unwrap_or_else(|| crate::dev_mode::code(app, ui, first, lang));
    let code = if rem { figcraft_engine::codegen::px_to_rem(&code, 16.0) } else { code };
    egui::Frame::NONE
        .fill(t.field)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(10))
        .outer_margin(egui::Margin::symmetric(16, 0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(&code).monospace().color(t.text));
        });
    ui.add_space(8.0);
    let mut copy_clicked = copy;
    row(ui, 32.0, |ui| {
        copy_clicked |= ui.button(trf("Copy {value}", &[("value", &lang.label())])).clicked();
    });
    if copy_clicked {
        app.services.clipboard_set_text(&code);
        app.toast(trf("Copied {value}", &[("value", &lang.label())]));
    }
    crate::dev_mode::connect_section(app, ui, &doc, first, lang);
    crate::dev_mode::plugins_section(app, ui);
}
