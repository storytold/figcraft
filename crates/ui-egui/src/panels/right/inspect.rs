use super::menu::{menu_item, menu_popup, row};
use super::{
    Align2, Color32, CornerRadius, Document, ExportFormat, ExportSetting, FigApp, Icon, Id, NodeId, PAD, Paint, Rect, Sense, Stroke, Tip, Tokens, Ui,
    a11y, icons, pos2, regular, semibold, tr, trf, vec2, widgets,
};

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
    let mut right = r.max.x - widgets::PAD_RIGHT;
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
        app.clipboard_set_text(&p.value);
        app.toast(trf("Copied {value}", &[("value", &label.to_lowercase())]));
    }
}

/// The thumbnails and chosen export formats of the Inspect tab's assets, kept between frames: the
/// thumbnails until the document changes, the formats for the session.
#[derive(Clone, Default)]
struct AssetUi {
    revision: u64,
    thumbs: std::collections::HashMap<NodeId, Option<egui::TextureHandle>>,
    formats: std::collections::HashMap<NodeId, usize>,
}

/// An asset row's height and its preview's side.
const ASSET_ROW_H: f32 = 56.0;

const ASSET_THUMB: f32 = 40.0;

/// The selection's assets (layers with export settings, icons, images) as a list of a preview, a
/// name and a format: click one to export it in that format, or export them all with their own
/// export settings.
fn inspect_assets(app: &mut FigApp, ui: &mut Ui, doc: &Document, assets: &[figcraft_engine::devinspect::Asset]) {
    let t = Tokens::get(ui.ctx());
    widgets::separator(ui);
    let export_all = [(Icon::Export, tr("Export all"))];
    let all = widgets::section_header(ui, tr("Assets"), if assets.is_empty() { &[] } else { &export_all }, false).is_some();
    if assets.is_empty() {
        widgets::note(ui, tr("No assets: no export settings, icons or images. Add export settings in the Design tab."), t.text_secondary);
        return;
    }
    let mut export_all_ids: Option<Vec<NodeId>> = all.then(|| assets.iter().map(|a| a.id).collect());
    let mut export_one: Option<(NodeId, ExportSetting, String)> = None;
    let revision = app.session.ds().map(|d| d.revision).unwrap_or(0);
    let cache_id = Id::unique("inspect_asset_ui");
    let mut state: AssetUi = ui.data(|d| d.get_temp(cache_id)).unwrap_or_default();
    if state.revision != revision {
        state.revision = revision;
        state.thumbs.clear();
    }
    let px = ASSET_THUMB * ui.ctx().pixels_per_point();
    for a in assets {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ASSET_ROW_H), Sense::click());
        let resp = resp.tip(trf("Export {name}", &[("name", &a.name)]));
        if resp.hovered() {
            ui.painter().rect_filled(r.shrink2(vec2(8.0, 1.0)), CornerRadius::same(6), t.hover);
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(r.max.x - widgets::PAD_RIGHT - 6.0, r.center().y), vec2(12.0, 12.0)),
                Icon::Export,
                t.icon,
            );
        }
        // The preview, as exporting it paints it (a slice shows the page under it). Rows scrolled
        // out of view draw nothing, so they don't pay for a thumbnail either.
        let thumb = Rect::from_min_size(pos2(r.min.x + PAD, r.center().y - ASSET_THUMB / 2.0), vec2(ASSET_THUMB, ASSET_THUMB));
        ui.painter().rect_filled(thumb, CornerRadius::same(4), t.field);
        let tex = if ui.is_rect_visible(r) {
            state.thumbs.entry(a.id).or_insert_with(|| crate::assets_panel::thumbnail(ui.ctx(), doc, a.id, px)).clone()
        } else {
            None
        };
        if let Some(tex) = tex {
            let size = tex.size_vec2();
            let inner = thumb.shrink(3.0);
            // Never past 1:1 physical pixels, so small assets stay crisp.
            let k = (inner.width() / size.x).min(inner.height() / size.y).min(1.0 / ui.ctx().pixels_per_point()).max(0.0);
            let img = Rect::from_center_size(thumb.center(), size * k);
            ui.painter().with_clip_rect(inner).image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        ui.painter().rect_stroke(thumb, CornerRadius::same(4), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        // The name, and the format to export it as (its first export setting's, until chosen).
        let x = thumb.max.x + 10.0;
        let name_r = Rect::from_min_max(pos2(x, r.min.y + 5.0), pos2(r.max.x - widgets::PAD_RIGHT, r.min.y + 27.0));
        let name = widgets::elide(ui, &a.name, name_r.width());
        ui.painter().with_clip_rect(name_r).text(pos2(name_r.min.x, name_r.center().y), Align2::LEFT_CENTER, &name, regular(11.0), t.text);
        let idx = state
            .formats
            .get(&a.id)
            .copied()
            .filter(|i| *i < ExportFormat::ALL.len())
            .unwrap_or_else(|| ExportFormat::ALL.iter().position(|f| Some(*f) == a.settings.first().map(|s| s.format)).unwrap_or(0));
        let fmt = ExportFormat::ALL[idx];
        let btn = Rect::from_min_size(pos2(x, r.min.y + 31.0), vec2(74.0, 20.0));
        let btn_resp = ui.interact(btn, Id::unique(("inspect_asset_format", a.id)), Sense::click());
        a11y::combo(&btn_resp, tr("Format"), fmt.label(), a11y::menu_open(&btn_resp));
        if btn_resp.hovered() {
            ui.painter().rect_filled(btn, CornerRadius::same(5), t.field);
            ui.painter().rect_stroke(btn, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), egui::StrokeKind::Inside);
        } else {
            ui.painter().rect_filled(btn, CornerRadius::same(5), t.field);
        }
        ui.painter().text(pos2(btn.min.x + 8.0, btn.center().y), Align2::LEFT_CENTER, fmt.label(), regular(11.0), t.text);
        icons::paint(ui.painter(), Rect::from_center_size(pos2(btn.max.x - 9.0, btn.center().y), vec2(10.0, 10.0)), Icon::Chevron, t.icon_secondary);
        menu_popup(&btn_resp).show(|ui| {
            for (i, f) in ExportFormat::ALL.iter().enumerate() {
                if menu_item(ui, f.label(), "") {
                    state.formats.insert(a.id, i);
                    ui.close();
                }
            }
        });
        if resp.clicked() && !btn_resp.hovered() {
            let mut setting = a.settings.first().cloned().unwrap_or_default();
            setting.format = fmt;
            let file = format!("{}{}.{}", a.name.rsplit('/').next().unwrap_or(&a.name), setting.file_suffix(), fmt.ext());
            export_one = Some((a.id, setting, file));
        }
    }
    ui.data_mut(|d| d.insert_temp(cache_id, state));
    if let Some(ids) = export_all_ids.take() {
        crate::export_panel::export_layers(app, &ids, true);
    } else if let Some((id, setting, file)) = export_one {
        crate::export_panel::save_one(app, &[id], &setting, &file);
    }
    ui.add_space(4.0);
}

/// The Inspect tab (Dev Mode): the handoff status, annotations, properties, assets and code of
/// the selected layer, or the list of what's ready for dev.
pub(super) fn inspect(app: &mut FigApp, ui: &mut Ui) {
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
        crate::dev_mode::measurements_section(app, ui, &doc, &sel);
        crate::dev_mode::plugins_section(app, ui);
        return;
    };
    let name = doc.node(first).map(|n| n.name.clone()).unwrap_or_default();
    widgets::section_header(ui, &name, &[], false);
    crate::dev_mode::status(app, ui, &doc, &sel);
    crate::dev_mode::annotations_section(app, ui, &doc, first);
    crate::dev_mode::dev_resources_section(app, ui, &doc, first);
    crate::dev_mode::measurements_section(app, ui, &doc, &sel);
    use figcraft_engine::devinspect::ColorFormat;
    let fmt_id = Id::unique("inspect_color_format");
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
    inspect_assets(app, ui, &doc, &figcraft_engine::devinspect::assets(&doc, &sel));
    use figcraft_engine::codegen::CodeLang;
    let lang_id = Id::unique("inspect_lang");
    let lang = ui.data(|d| d.get_temp::<usize>(lang_id)).and_then(|i| CodeLang::ALL.get(i).copied()).unwrap_or_default();
    // A component tried with other property values: its code replaces the layer's below.
    let played = crate::dev_mode::playground_section(app, ui, &doc, first, lang);
    widgets::separator(ui);
    let copy = widgets::section_header(ui, tr("Code"), &[(Icon::Export, tr("Copy"))], false).is_some();
    // Pixels or rem (of a 16 px root) for the web languages.
    let rem_id = Id::unique("inspect_rem");
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
        app.clipboard_set_text(&code);
        app.toast(trf("Copied {value}", &[("value", &lang.label())]));
    }
    crate::dev_mode::connect_section(app, ui, &doc, first, lang);
    crate::dev_mode::plugins_section(app, ui);
}
