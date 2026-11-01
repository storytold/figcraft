//! The right panel: Design, Prototype and Inspect tabs.

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_color::{BlendMode, Paint, PaintKind};
use figcraft_doc::{
    Blur, BlurType, BoolOp, Brush, Document, Effect, EffectKind, ExportFormat, ExportSetting, Glass, GridAlign, GridKind, LayoutGrid,
    MAX_BLUR_RADIUS, MaskType, Node, NodeId, NodeKind, Noise, NoiseMode, Shadow, StrokeAlign, StrokeCap, StrokeJoin, StrokeProps, Texture,
    VariableType, WidthPoint, WidthPreset, WidthProfile,
};
use figcraft_engine::cmd::lists::{moved_index, removed_index};
use serde_json::{Value, json};

use crate::a11y::{self, Tip};
use crate::i18n::{tr, trf, trn};
use crate::icons::{self, Icon};
use crate::list_drag::{ListDrag, move_flags, remove_flags};
use crate::style_picker::{self, StyleSlot};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, Edit, FIELD_H, FieldEdit, Lead, PAD};
use crate::{FigApp, RightTab};

mod effects;
mod inspect;
mod menu;
mod paints;
mod stroke;

pub(crate) use effects::effect_open;
use effects::{effects, grids};
use inspect::inspect;
pub(crate) use inspect::{inspect_section, prop_row};
#[cfg(test)]
pub(crate) use menu::menu_row_width;
#[cfg(test)]
pub(crate) use menu::preview_blend;
pub use menu::{blend_menu, context_popup, menu_check, menu_item, menu_item_beside_checks, menu_item_with, menu_popup, menu_row};
pub(crate) use menu::{block, menu_row_lit, menu_style, row, row_rect};
use paints::{appearance, paints, variable_modes};
pub(crate) use stroke::{CAP_ART_W, cap_art, join_label};

/// Common value of `f` across `ids` (None when mixed or empty).
pub(crate) fn common<T: PartialEq + Clone>(doc: &Document, ids: &[NodeId], f: impl Fn(&Node) -> T) -> Option<T> {
    crate::selection_cache::with_nodes(doc, ids, |it| {
        let mut it = it.flatten();
        let first = f(it.next()?);
        if it.all(|n| f(n) == first) { Some(first) } else { None }
    })
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

/// Draw the right panel: people, present and share controls, Design/Prototype tabs and zoom
/// menu, then route the scroll area to comments, frame presets, design, prototype or inspect.
pub fn right_panel(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // The panel's edge, drawn here: the frame has no stroke (a stroke counts as margin and would
    // take 1 pt off each side of the panel (240 pt by default)).
    let edge = ui.max_rect();
    app.ui.right_panel_rect = Some(edge);
    ui.painter().line_segment([edge.left_top(), edge.left_bottom()], Stroke::new(1.0, t.border));
    let w = ui.available_width();
    // Top bar: people (you and the others in a live session), present, share.
    let (top, _) = ui.allocate_exact_size(vec2(w, 48.0), Sense::hover());
    // As wide as its label needs (longer in some languages).
    let label = ui.painter().layout_no_wrap(tr("Share").to_string(), semibold(11.5), t.accent_on);
    let sw = (label.size().x + 24.0).max(56.0);
    let share = Rect::from_min_size(pos2(top.max.x - 14.0 - sw, top.center().y - 16.0), vec2(sw, 32.0));
    let sresp = ui.interact(share, ui.scope_id().with("share"), Sense::click()).tip(tr("Share: live session, export"));
    ui.painter().rect_filled(share, CornerRadius::same(6), if sresp.hovered() { t.accent_hover } else { t.accent });
    ui.painter().galley(share.center() - label.size() / 2.0, label, t.accent_on);
    crate::live::share_popover(app, &sresp);
    // Play presents; its chevron opens the play menu (present, or preview in a pane).
    let play = Rect::from_min_size(pos2(share.min.x - 50.0, top.center().y - 14.0), vec2(42.0, 28.0));
    let (tri_r, chev_r) =
        (Rect::from_min_max(play.min, pos2(play.max.x - 16.0, play.max.y)), Rect::from_min_max(pos2(play.max.x - 16.0, play.min.y), play.max));
    let present_keys = crate::toolbar::pretty_shortcut(crate::shortcuts::shortcut("proto.present").unwrap_or(""));
    let presp = ui.interact(tri_r, ui.scope_id().with("play"), Sense::click()).tip(format!("{}  {present_keys}", tr("Present")));
    let mresp = ui.interact(chev_r, ui.scope_id().with("play_menu"), Sense::click()).tip(tr("Present or preview"));
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
        let resp = ui.interact(r, ui.scope_id().with(("rtab", label)), Sense::click());
        let active = app.ui.right_tab == tab;
        a11y::tab(&resp, label, active);
        if active {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        ui.painter().galley(pos2(r.min.x + 8.0, r.center().y - galley.size().y / 2.0), galley, if active { t.text } else { t.text_secondary });
        if resp.clicked() {
            if app.ui.right_tab != tab {
                crate::proto_panel::open_editor(ui.ctx(), None);
            }
            app.ui.right_tab = tab;
        }
        x = r.max.x + 2.0;
    }
    // Zoom menu.
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let zr = Rect::from_min_size(pos2(tabs.max.x - 64.0, tabs.center().y - 12.0), vec2(56.0, 24.0));
    let zresp = ui.interact(zr, ui.scope_id().with("zoommenu"), Sense::click());
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

fn design(app: &mut FigApp, ui: &mut Ui) {
    let Ok(ds) = app.session.ds() else { return };
    // The panel shows the document itself, not a live text preview over it.
    let doc = ds.text_preview.clone().unwrap_or_else(|| ds.doc.clone());
    // While a gesture only moves the selection, nothing the sections show changes but where it
    // is: they show the document it started from, so what they keep stays kept (#1571).
    let shown = crate::selection_cache::moved(ds).filter(|_| ds.text_preview.is_none()).map_or_else(|| doc.clone(), |(b, _)| b);
    let sel = ds.selection.clone();
    let vector_edit = ds.vector_edit.clone();
    if sel.is_empty() {
        page_section(app, ui, &doc);
        return;
    }
    crate::selection_cache::show(&shown, &sel);
    header(app, ui, &shown, &sel);
    widgets::separator(ui);
    // Editing a vector with points selected, the points' fields stand in for the layer's position
    // and dimensions.
    let points_selected = vector_edit.as_ref().is_some_and(|ve| !ve.vertices.is_empty() || !ve.segments.is_empty());
    let vector_edit_none = vector_edit.is_none();
    match vector_edit {
        Some(ve) => crate::vector_panel::points(app, ui, &shown, &ve),
        None => {
            crate::section_panel::section(app, ui, &shown, &sel);
            if app.session.tool.tool == figcraft_engine::ToolId::Scale {
                crate::scale_panel::section(app, ui, &shown, &sel);
                widgets::separator(ui);
            }
            position(app, ui, &doc, &shown, &sel);
        }
    }
    widgets::separator(ui);
    if !points_selected {
        crate::layout_panel::layout_section(app, ui, &shown, &sel);
        widgets::separator(ui);
    }
    if vector_edit_none && arc_section(app, ui, &shown, &sel) || points_section(app, ui, &shown, &sel) {
        widgets::separator(ui);
    }
    appearance(app, ui, &shown, &sel);
    widgets::separator(ui);
    if variable_modes(app, ui, &shown, &sel) {
        widgets::separator(ui);
    }
    if crate::selection_cache::with_nodes(&shown, &sel, |it| it.flatten().any(|n| matches!(n.kind, NodeKind::Text(_)))) {
        crate::typography::typography(app, ui, &shown, &sel);
        widgets::separator(ui);
    }
    paints(app, ui, &shown, &sel, true);
    widgets::separator(ui);
    paints(app, ui, &shown, &sel, false);
    widgets::separator(ui);
    effects(app, ui, &shown, &sel);
    widgets::separator(ui);
    crate::selection_colors::section(app, ui, &shown, &sel);
    if crate::selection_cache::every(&shown, &sel, |n| n.kind.is_frame_like()) {
        grids(app, ui, &shown, &sel);
        widgets::separator(ui);
    }
    crate::export_panel::section(app, ui, &shown, &sel);
}

/// Draw the page section: background and colour profile, local variables/styles, and the
/// page export section.
fn page_section(app: &mut FigApp, ui: &mut Ui, doc: &Document) {
    let t = Tokens::get(ui.ctx());
    let Ok(page) = app.session.page() else { return };
    let bg = match doc.node(page).map(|n| &n.kind) {
        Some(NodeKind::Page { background, .. }) => *background,
        _ => figcraft_color::Color::DEFAULT_PAGE,
    };
    widgets::section_header(ui, tr("Page"), &[], false);
    let mut new_hex = None;
    row(ui, 32.0, |ui| {
        let p = Paint::solid(bg);
        let sw = widgets::swatch(ui, &p, 16.0);
        app.picker_control(crate::colorpicker::PickerTarget::PageBackground, &sw);
        if let Some(h) = widgets::text_field(ui, Id::unique("pagebg"), &bg.to_hex(), 96.0, "") {
            new_hex = Some(h);
        }
        widgets::label(ui, "100 %", t.text);
    });
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

/// Draw the selection header: type name (or Mixed), frame/group conversion, orientation
/// swap and frame presets, component/mask/boolean quick actions, and instance details.
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
    let is_group = !sel.is_empty() && crate::selection_cache::every(doc, sel, |n| matches!(n.kind, NodeKind::Group));
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
    // Keep the type menu and frame actions together. A short label must not leave
    // the orientation icon floating in the middle of the header.
    // The handoff icon also fits beside the frame controls at the panel minimum.
    let max_label_width = if is_frame { (w - 202.0).clamp(40.0, 120.0) } else { 92.0 };
    let shown = if is_frame || is_group { widgets::elide_font(ui, &label, max_label_width, semibold(13.0)) } else { label.clone() };
    let label_width = if is_frame || is_group { ui.painter().layout_no_wrap(shown.clone(), semibold(13.0), t.text).size().x + 28.0 } else { 120.0 };
    let lr = Rect::from_min_size(pos2(r.min.x + 8.0, r.center().y - 12.0), vec2(label_width, 24.0));
    let resp = ui.interact(lr, ui.scope_id().with("typedd"), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(lr, CornerRadius::same(5), t.hover);
    }
    let resp = if shown == label { resp } else { resp.on_hover_text(&label) };
    let label_rect = ui.painter().text(pos2(lr.min.x + 8.0, lr.center().y), Align2::LEFT_CENTER, &shown, semibold(13.0), t.text);
    if is_frame || is_group {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(label_rect.right() + 8.0, lr.center().y), vec2(10.0, 10.0)),
            Icon::Chevron,
            t.icon_secondary,
        );
    }
    if is_group {
        menu_popup(&resp).width(180.0).show(|ui| kind_items(app, ui, false));
    }
    if is_frame {
        let sr = Rect::from_min_size(pos2(lr.max.x + 2.0, r.center().y - 12.0), vec2(24.0, 24.0));
        let sresp = ui.interact(sr, ui.scope_id().with("swap_orient"), Sense::click()).tip(tr("Swap orientation"));
        if sresp.hovered() {
            ui.painter().rect_filled(sr, CornerRadius::same(5), t.hover);
        }
        icons::paint(ui.painter(), sr.shrink(5.0), Icon::Rotate, t.icon);
        if sresp.clicked() {
            app.run("frame.swapOrientation", json!({"ids": sel}));
        }
        let fit_r = sr.translate(vec2(26.0, 0.0));
        let fit_tip = crate::shortcuts::tip(tr("Fit to contents"), "object.resizeToFit");
        if widgets::icon_button_at(ui, ui.scope_id().with("frame_fit_contents"), fit_r, Icon::FitContents, fit_tip) {
            app.run("object.resizeToFit", json!({"ids": sel}));
        }
        if figcraft_engine::devinspect::has_status(doc, sel[0]) {
            crate::section_panel::status_icon(app, ui, doc, sel, fit_r.translate(vec2(26.0, 0.0)));
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
        let bresp = ui.interact(br, ui.scope_id().with(("qa", cmd)), Sense::click()).tip(tip);
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

/// Whether a selected layer is an instance or in one, and whether one is inside an instance (not
/// the instance itself): the Design panel asks every frame, kept while the document and the
/// selection stay the same ([`crate::selection_cache`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InInstance {
    pub any: bool,
    pub nested: bool,
}

pub(crate) fn in_instance(ctx: &egui::Context, doc: &Arc<Document>, sel: &[NodeId]) -> InInstance {
    use figcraft_engine::components::{inside_instance, owning_instance};
    crate::selection_cache::cached(ctx, "in instance", doc, sel, &[], || InInstance {
        any: sel.iter().any(|i| owning_instance(doc, *i).is_some()),
        nested: sel.iter().any(|i| inside_instance(doc, *i)),
    })
}

/// The Position section: its values from `doc`, what doesn't change with where the layers are from
/// `shown` (the panel's document, see `design`).
fn position(app: &mut FigApp, ui: &mut Ui, doc: &Arc<Document>, shown: &Arc<Document>, sel: &[NodeId]) {
    widgets::section_header(ui, tr("Position"), &[], false);
    // Layers inside an instance take their place and constraints from the main component.
    let nested = in_instance(ui.ctx(), shown, sel).nested;
    ui.add_enabled_ui(!nested, |ui| position_fields(app, ui, doc, sel))
        .response
        .on_disabled_hover_text(tr("Layers in an instance take their position from the main component"));
    ui.add_space(8.0);
}

/// What the position fields show of the selection (`None`: mixed), kept like [`in_instance`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PositionValues {
    pub can_align: bool,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub rot: Option<f64>,
    /// Every selected layer is in an auto layout frame, and whether they ignore its layout.
    pub parent_al: bool,
    pub absolute: Option<bool>,
    /// Every selected layer is in a frame that places it by constraints, and those constraints.
    pub in_frame: bool,
    pub constraints: Option<figcraft_doc::Constraints>,
}

/// What the X field shows of a layer (in its frame's space: groups have none of their own); a
/// relative edit of mixed values starts from it.
fn x_of(doc: &Document, n: &Node) -> f64 {
    (doc.frame_relative(n.id).as_coeffs()[4] * 100.0).round() / 100.0
}

fn y_of(doc: &Document, n: &Node) -> f64 {
    (doc.frame_relative(n.id).as_coeffs()[5] * 100.0).round() / 100.0
}

fn rot_of(n: &Node) -> f64 {
    (figcraft_geom::rotation_degrees(n.transform) * 100.0).round() / 100.0
}

pub(crate) fn position_values(ctx: &egui::Context, doc: &Arc<Document>, sel: &[NodeId]) -> PositionValues {
    let parent = |n: &Node| n.parent.and_then(|p| doc.node(p));
    crate::selection_cache::cached(ctx, "position", doc, sel, &[], || PositionValues {
        can_align: figcraft_engine::cmd::arrange::can_align(doc, sel),
        x: common(doc, sel, |n| x_of(doc, n)),
        y: common(doc, sel, |n| y_of(doc, n)),
        rot: common(doc, sel, rot_of),
        parent_al: sel.iter().all(|i| doc.node(*i).and_then(parent).is_some_and(|p| p.layout.is_some())),
        absolute: common(doc, sel, |n| n.child_layout.absolute),
        // Constraints: children of frames without auto layout, or ignoring it.
        in_frame: sel.iter().all(|i| {
            doc.node(*i).is_some_and(|n| parent(n).is_some_and(|p| p.kind.is_frame_like() && (p.layout.is_none() || n.child_layout.absolute)))
        }),
        constraints: common(doc, sel, |n| n.constraints),
    })
}

/// Draw alignment and distribution, X/Y and rotation with flip/rotate controls, the
/// absolute-position toggle and frame constraints.
fn position_fields(app: &mut FigApp, ui: &mut Ui, doc: &Arc<Document>, sel: &[NodeId]) {
    let pv = position_values(ui.ctx(), doc, sel);
    let col = widgets::col_width(ui.available_width());
    align_row(app, ui, sel, pv.can_align, col);
    position_row(app, ui, doc, sel, &pv, col);
    rotation_row(app, ui, sel, pv.rot, col);
    if pv.in_frame {
        constraints_row(app, ui, sel, pv.constraints, col);
    }
}

/// Alignment, and distribution and tidy up for three or more layers.
fn align_row(app: &mut FigApp, ui: &mut Ui, sel: &[NodeId], can_align: bool, col: f32) {
    let ids = sel.to_vec();
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
        let picked = ui.add_enabled_ui(can_align, |ui| {
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
}

/// X and Y, and the absolute-position toggle for children of auto layout frames.
fn position_row(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId], pv: &PositionValues, col: f32) {
    let ids = sel.to_vec();
    widgets::sublabels(ui, tr("Position"), None);
    row(ui, 32.0, |ui| {
        let e = widgets::number_field_mixed(ui, Id::unique("pos_x"), Lead::Text("X"), pv.x, "", col, 1.0);
        app.field_of(&ids, "x", e, |n| x_of(doc, n), |v| json!(v));
        let e = widgets::number_field_mixed(ui, Id::unique("pos_y"), Lead::Text("Y"), pv.y, "", col, 1.0);
        app.field_of(&ids, "y", e, |n| y_of(doc, n), |v| json!(v));
        // Children of auto layout frames can ignore the layout (absolute position).
        if pv.parent_al {
            let on = pv.absolute.unwrap_or(false);
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
}

/// Rotation, with the rotate and flip buttons.
fn rotation_row(app: &mut FigApp, ui: &mut Ui, sel: &[NodeId], rot: Option<f64>, col: f32) {
    let ids = sel.to_vec();
    widgets::sublabels(ui, tr("Rotation"), None);
    row(ui, 32.0, |ui| {
        let e = widgets::number_field_mixed(ui, Id::unique("pos_rot"), Lead::Icon(Icon::Angle, tr("Rotation")), rot, "°", col, 1.0);
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
}

/// The horizontal and vertical constraints of children of frames without auto layout.
fn constraints_row(app: &mut FigApp, ui: &mut Ui, sel: &[NodeId], c: Option<figcraft_doc::Constraints>, col: f32) {
    let ids = sel.to_vec();
    widgets::sublabels(ui, tr("Constraints"), None);
    row(ui, 32.0, |ui| {
        let h = c.map(|c| format!("{:?}", c.horizontal)).unwrap_or_else(|| "Mixed".into());
        let resp = widgets::dropdown(ui, tr(&pretty(&h)), col, None);
        menu_popup(&resp).show(|ui| {
            for (l, v) in
                [(tr("Left"), "left"), (tr("Right"), "right"), (tr("Left & right"), "leftRight"), (tr("Center"), "center"), (tr("Scale"), "scale")]
            {
                if menu_item(ui, l, "") {
                    app.set_props(&ids, json!({"constraints": {"horizontal": v}}));
                    ui.close();
                }
            }
        });
        let v = c.map(|c| format!("{:?}", c.vertical)).unwrap_or_else(|| "Mixed".into());
        let resp = widgets::dropdown(ui, tr(&pretty(&v)), col, None);
        menu_popup(&resp).show(|ui| {
            for (l, val) in
                [(tr("Top"), "top"), (tr("Bottom"), "bottom"), (tr("Top & bottom"), "topBottom"), (tr("Center"), "center"), (tr("Scale"), "scale")]
            {
                if menu_item(ui, l, "") {
                    app.set_props(&ids, json!({"constraints": {"vertical": val}}));
                    ui.close();
                }
            }
        });
    });
}

/// The selected ellipses' arc: Start (degrees), Sweep (a share of a full turn, negative running
/// the other way) and Ratio (the inner radius), the same values the canvas arc handles drag
/// (`node.set`'s `arcStart` / `arcSweep` / `arcInner`; bindable to number variables, sweep and
/// ratio ones holding percentages). Returns whether it showed.
fn arc_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) -> bool {
    let arcs: Vec<(f64, f64, f64)> = crate::selection_cache::with_nodes(doc, sel, |it| {
        it.flatten()
            .filter_map(|n| match n.kind {
                NodeKind::Ellipse { start, sweep, inner } => Some((start, sweep, inner)),
                _ => None,
            })
            .collect()
    });
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
        crate::variable_ui::number_field(app, ui, &ids, key, Id::unique(("arc", key)), Lead::Icon(icon, name), value, suffix, col)
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
    let shapes: Vec<(u32, Option<f64>)> = crate::selection_cache::with_nodes(doc, sel, |it| {
        it.flatten()
            .filter_map(|n| match n.kind {
                NodeKind::Polygon { count } => Some((count, None)),
                NodeKind::Star { count, inner_ratio } => Some((count, Some(inner_ratio))),
                _ => None,
            })
            .collect()
    });
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
        let e = widgets::number_field(ui, Id::unique("pts_count"), Lead::Icon(Icon::Polygon, tr("Count")), count, "", col, 1.0);
        app.field(&ids, "pointCount", e, |v| json!(v.round().clamp(3.0, 120.0)));
        if all_stars {
            let e = widgets::number_field(ui, Id::unique("pts_ratio"), Lead::Icon(Icon::Star, tr("Ratio")), ratio, "%", col, 1.0);
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
