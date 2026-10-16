//! Exporting: the Design panel's Export section (a row per export setting with size, suffix,
//! format and its advanced options, PDF print settings, a preview, and "Export N layers"), the
//! page's Export button, and the export actions and print options the Export dialog shares. Every
//! change runs an `export.*` or `file.export*` command.

use std::collections::BTreeSet;

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::*;
use figcraft_engine::cmd::file::export_jobs;
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::{tr, trf, trn};
use crate::icons::{self, Icon};
use crate::right::{common, menu_item, menu_popup, row};
use crate::theme::{Tokens, regular};
use crate::widgets::{self, FIELD_H};

/// Re-render the preview at most this often (seconds) while the document changes.
const PREVIEW_REFRESH: f64 = 0.3;
/// Height of the preview box.
const PREVIEW_H: f32 = 160.0;

/// The Export section for the selected layers.
pub fn section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let t = Tokens::get(ui.ctx());
    let ids = sel.to_vec();
    let shared = common(doc, sel, |n| n.exports.clone());
    let mixed = shared.is_none() && sel.len() > 1;
    let list = shared.unwrap_or_default();
    let tip = if mixed { tr("Replace mixed export settings") } else { tr("Add export setting") };
    if widgets::section_header(ui, tr("Export"), &[(Icon::Plus, tip)], list.is_empty()).is_some() {
        if mixed {
            app.set_props(&ids, json!({"exports": [ExportSetting::default()]}));
        } else {
            app.run("export.add", json!({"ids": ids}));
        }
    }
    if mixed {
        widgets::note(ui, tr("Click + to replace mixed content"), t.text_secondary);
    }
    for (i, e) in list.iter().enumerate() {
        setting_row(app, ui, &ids, i, e);
    }
    let jobs = export_jobs(doc, sel);
    if jobs.is_empty() {
        return;
    }
    if let Some(first) = jobs.first().map(|j| j.id) {
        preview(app, ui, doc, first);
    }
    let layers: BTreeSet<NodeId> = jobs.iter().map(|j| j.id).collect();
    let label = match (layers.len(), layers.first().and_then(|i| doc.node(*i))) {
        (1, Some(n)) => trf("Export {name}", &[("name", &n.name)]),
        (k, _) => trn("Export {n} layers", k, &[]),
    };
    if wide_button(ui, &label) {
        export_layers(app, sel, false);
    }
}

/// One export setting: size (typed or picked), suffix, format, advanced options and remove; a
/// PDF's print settings under it.
fn setting_row(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], i: usize, e: &ExportSetting) {
    row(ui, 32.0, |ui| {
        // Tighter spacing so the advanced options and remove buttons fit beside the fields.
        ui.spacing_mut().item_spacing.x = 4.0;
        let (size_w, format_w, button, gap) = (52.0, 54.0, 22.0, ui.spacing().item_spacing.x);
        let suffix_w = (ui.available_width() - size_w - format_w - button * 2.0 - gap * 4.0).max(32.0);
        let presets: Vec<String> =
            ExportSize::SCALES.iter().map(|value| ExportSize::Scale { value: *value }.label()).chain(["512w".into(), "512h".into()]).collect();
        if let Some(s) = widgets::combo_field(ui, Id::new(("export_size", i)), &e.size.label(), size_w, &presets)
            && ExportSize::parse(&s) != Some(e.size)
        {
            app.run("export.update", json!({"ids": ids, "index": i, "size": s}));
        }
        if let Some(sfx) = widgets::text_field(ui, Id::new(("export_suffix", i)), &e.suffix, suffix_w, tr("Suffix"))
            && sfx != e.suffix
        {
            app.run("export.update", json!({"ids": ids, "index": i, "suffix": sfx}));
        }
        let resp = widgets::dropdown(ui, e.format.label(), format_w, None);
        menu_popup(&resp).show(|ui| {
            for f in ExportFormat::ALL {
                if menu_item(ui, f.label(), "") {
                    app.run("export.update", json!({"ids": ids, "index": i, "format": f.ext()}));
                    ui.close();
                }
            }
        });
        let more = icons::button(ui, Icon::Dots, button, false, tr("Export settings"));
        egui::Popup::from_toggle_button_response(&more)
            .id(Id::new(("export_more", i)))
            .width(216.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                if let Some(mut params) = advanced_options(ui, e) {
                    params["ids"] = json!(ids);
                    params["index"] = json!(i);
                    app.run("export.update", params);
                }
            });
        if icons::button(ui, Icon::Minus, button, false, tr("Remove export setting")).clicked() {
            app.run("export.remove", json!({"ids": ids, "index": i}));
        }
    });
    if e.format == ExportFormat::Pdf {
        print_row(app, ui, ids, i, &e.print);
    }
}

/// A setting's advanced options: contents only and include bounding box, and for SVG outline
/// text, the id attribute and simplified strokes. Returns the changed option as command params.
fn advanced_options(ui: &mut Ui, e: &ExportSetting) -> Option<Value> {
    let mut out = None;
    let mut option = |ui: &mut Ui, key: &str, on: bool, label: &str| {
        if widgets::checkbox(ui, on, label) {
            out = Some(json!({ key: !on }));
        }
    };
    option(ui, "contentsOnly", e.contents_only, tr("Contents only"));
    option(ui, "includeBoundingBox", e.include_bounding_box, tr("Include bounding box"));
    if e.format == ExportFormat::Svg {
        option(ui, "svgOutlineText", e.svg_outline_text, tr("Outline text"));
        option(ui, "svgIdAttribute", e.svg_id_attribute, tr("Include id attribute"));
        option(ui, "svgSimplifyStroke", e.svg_simplify_stroke, tr("Simplify stroke"));
    }
    out
}

/// A PDF setting's print settings: a summary row that opens them.
fn print_row(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], i: usize, p: &PrintSettings) {
    let t = Tokens::get(ui.ctx());
    row(ui, 26.0, |ui| {
        let open_id = Id::new(("export_print", i));
        let open = egui::Popup::is_id_open(ui.ctx(), open_id);
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), FIELD_H), Sense::click());
        if resp.hovered() || open {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        let label = ui.painter().layout(print_summary(p), regular(11.0), t.text_secondary, r.width() - 32.0);
        ui.painter().galley(pos2(r.min.x + 8.0, r.center().y - label.size().y / 2.0), label, t.text_secondary);
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 12.0, r.center().y), vec2(14.0, 14.0)), Icon::Dots, t.icon_secondary);
        egui::Popup::from_toggle_button_response(&resp.on_hover_text(tr("PDF print settings")))
            .id(open_id)
            .width(216.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                if let Some(next) = print_options(ui, Id::new(("export_print_fields", i)), p) {
                    let mut params = print_params(&next);
                    params["ids"] = json!(ids);
                    params["index"] = json!(i);
                    app.run("export.update", params);
                }
            });
    });
}

/// "3 mm bleed, crop marks, CMYK" (or "RGB, no bleed").
fn print_summary(p: &PrintSettings) -> String {
    let mut parts = Vec::new();
    if p.bleed() > 0.0 {
        parts.push(trf("{length} bleed", &[("length", &PrintSettings::length_label(p.bleed()))]));
    }
    if p.crop_marks {
        parts.push(tr("crop marks").to_string());
    }
    if p.bleed_marks && p.bleed() > 0.0 {
        parts.push(tr("bleed marks").to_string());
    }
    parts.push(if p.cmyk { "CMYK" } else { "RGB" }.to_string());
    if p.bleed() <= 0.0 && !p.marks() {
        parts.push(tr("no bleed").to_string());
    }
    let s = parts.join(", ");
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or(s)
}

/// PDF print options: bleed (typed with a unit: "3 mm", "0.125 in", "9 pt"; anything else keeps
/// the bleed), crop and bleed marks, CMYK colour. Returns the options when one changed.
pub(crate) fn print_options(ui: &mut Ui, id: Id, p: &PrintSettings) -> Option<PrintSettings> {
    let t = Tokens::get(ui.ctx());
    let mut next = *p;
    ui.horizontal(|ui| {
        widgets::label(ui, tr("Bleed"), t.text_secondary);
        let w = (ui.available_width() - 4.0).max(48.0);
        if let Some(b) = widgets::text_field(ui, id.with("bleed"), &PrintSettings::length_label(p.bleed()), w, "0 mm")
            .and_then(|v| PrintSettings::parse_length(&v))
            .filter(|b| *b <= PrintSettings::MAX_BLEED)
        {
            next.bleed = b;
        }
    });
    for (on, label) in [(&mut next.crop_marks, tr("Crop marks")), (&mut next.bleed_marks, tr("Bleed marks")), (&mut next.cmyk, tr("CMYK colour"))] {
        if widgets::checkbox(ui, *on, label) {
            *on = !*on;
        }
    }
    if p.bleed_marks && p.bleed() <= 0.0 {
        widgets::label(ui, tr("Bleed marks need a bleed"), t.text_secondary);
    }
    (next != *p).then_some(next)
}

/// Print options as command params (`bleed`, `cropMarks`, `bleedMarks`, `cmyk`).
fn print_params(p: &PrintSettings) -> Value {
    json!({"bleed": p.bleed(), "cropMarks": p.crop_marks, "bleedMarks": p.bleed_marks, "cmyk": p.cmyk})
}

/// `file.export` params for `setting`: its size, format and options (PDF: its print options).
fn setting_params(setting: &ExportSetting) -> Value {
    let mut params = serde_json::to_value(setting).unwrap_or_else(|_| json!({}));
    if let Value::Object(m) = &mut params {
        m.remove("print");
        m.remove("suffix");
        if setting.format == ExportFormat::Pdf
            && let Value::Object(p) = print_params(&setting.print)
        {
            m.extend(p);
        }
    }
    params
}

/// The rendered preview, cached per layer and document revision.
#[derive(Clone)]
struct PreviewTex {
    id: NodeId,
    revision: u64,
    at: f64,
    tex: Option<egui::TextureHandle>,
}

/// A "Preview" disclosure row; open, it shows the layer as it exports.
fn preview(app: &mut FigApp, ui: &mut Ui, doc: &Document, id: NodeId) {
    let t = Tokens::get(ui.ctx());
    let mut open = app.ui.export_thumb;
    row(ui, 28.0, |ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), FIELD_H), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        let icon = if open { Icon::Chevron } else { Icon::ChevronRight };
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 8.0, r.center().y), vec2(12.0, 12.0)), icon, t.icon_secondary);
        ui.painter().text(pos2(r.min.x + 20.0, r.center().y), Align2::LEFT_CENTER, tr("Preview"), regular(11.0), t.text);
        if resp.clicked() {
            open = !open;
        }
    });
    app.ui.export_thumb = open;
    if !open {
        return;
    }
    let revision = app.session.ds().map(|d| d.revision).unwrap_or(0);
    let now = ui.input(|i| i.time);
    let cache_id = Id::new("export_preview_tex");
    let cached: Option<PreviewTex> = ui.data(|d| d.get_temp(cache_id));
    let stale = cached.as_ref().is_none_or(|c| c.id != id || c.revision != revision);
    let due = cached.as_ref().is_none_or(|c| c.id != id || now - c.at >= PREVIEW_REFRESH);
    let tex = match cached {
        Some(c) if !stale || !due => {
            if stale {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(PREVIEW_REFRESH));
            }
            c.tex
        }
        _ => {
            let px = ui.available_width() * ui.ctx().pixels_per_point();
            let tex = crate::assets_panel::thumbnail(ui.ctx(), doc, id, px);
            ui.data_mut(|d| d.insert_temp(cache_id, PreviewTex { id, revision, at: now, tex: tex.clone() }));
            tex
        }
    };
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, PREVIEW_H + 8.0), Sense::hover());
    let frame = Rect::from_min_max(pos2(r.min.x + widgets::PAD, r.min.y), pos2(r.max.x - widgets::PAD, r.max.y - 8.0));
    widgets::checker(&ui.painter().with_clip_rect(frame), frame, 8.0);
    ui.painter().rect_stroke(frame, CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
    if let Some(tex) = tex {
        let [tw, th] = tex.size().map(|v| v.max(1) as f32);
        // Fit the box, never past the texture's own pixels.
        let k = ((frame.width() - 16.0) / tw).min((frame.height() - 16.0) / th).min(1.0 / ui.ctx().pixels_per_point().max(0.5)).max(0.0);
        let img = Rect::from_center_size(frame.center(), vec2(tw * k, th * k));
        ui.painter().image(tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// A full-width outlined button row; true when clicked.
pub(crate) fn wide_button(ui: &mut Ui, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut clicked = false;
    row(ui, 40.0, |ui| {
        let w = ui.available_width();
        let (r, resp) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
        let galley = ui.painter().layout(label.to_string(), regular(11.0), t.text, w - 16.0);
        ui.painter().galley(r.center() - galley.size() / 2.0, galley, t.text);
        clicked = resp.clicked();
    });
    clicked
}

/// Export `ids` with their export settings (`assets`: the Inspect tab's assets in and under them):
/// one file through a save dialog, several into a folder.
pub(crate) fn export_layers(app: &mut FigApp, ids: &[NodeId], assets: bool) {
    let jobs = match app.session.doc() {
        Ok(doc) if assets => figcraft_engine::devinspect::asset_jobs(doc, ids),
        Ok(doc) => export_jobs(doc, ids),
        Err(_) => return,
    };
    match jobs.as_slice() {
        [] => {}
        // The suggested name keeps only the file's own name (a layer's folders apply to folders).
        [one] => save_one(app, &[one.id], &one.setting, one.file.rsplit('/').next().unwrap_or(&one.file)),
        _ => {
            let ids = ids.to_vec();
            app.pick_folder(tr("Export"), move |app, dir| {
                if let Some(r) = app.run("file.exportLayers", json!({"ids": ids, "dir": dir.to_string_lossy(), "assets": assets})) {
                    let n = r["files"].as_array().map_or(0, Vec::len);
                    app.toast(trn("Exported {n} files", n, &[]));
                }
            });
        }
    }
}

/// Export `ids` together as one file with `setting`, through a save dialog suggesting `file`.
pub(crate) fn save_one(app: &mut FigApp, ids: &[NodeId], setting: &ExportSetting, file: &str) {
    let ext = setting.format.ext();
    let mut params = setting_params(setting);
    params["ids"] = json!(ids);
    app.pick_save(tr("Export"), file, &[ext], move |app, p| {
        params["path"] = json!(p.to_string_lossy());
        if app.run("file.export", params).is_some() {
            app.toast(trf("Exported {name}", &[("name", &p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default())]));
        }
    });
}

/// The file name a one-off export of `ids` suggests: the first layer's name (or `fallback`) with
/// the setting's suffix.
pub(crate) fn suggested_name(doc: &Document, ids: &[NodeId], setting: &ExportSetting, fallback: &str) -> String {
    let name = ids.first().and_then(|i| doc.node(*i)).map_or(fallback, |n| n.name.as_str());
    format!("{name}{}.{}", setting.file_suffix(), setting.format.ext())
}

/// The page's Export section: the whole page as one PNG.
pub fn page_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, page: NodeId) {
    widgets::section_header(ui, tr("Export"), &[], false);
    if wide_button(ui, tr("Export page as PNG")) {
        let all: Vec<NodeId> = doc.node(page).map(|p| p.children.clone()).unwrap_or_default();
        let setting = ExportSetting::default();
        let page_name = doc.node(page).map_or(tr("Page"), |p| p.name.as_str());
        let file = format!("{page_name}.{}", setting.format.ext());
        save_one(app, &all, &setting, &file);
    }
}
