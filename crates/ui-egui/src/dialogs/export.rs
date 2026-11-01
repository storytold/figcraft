use super::{
    Align2, Context, CornerRadius, ExportFormat, ExportSetting, ExportSize, FigApp, Path, RichText, Sense, Stroke, Tokens, Value, export_jobs,
    export_layers, exportable_layers, json, regular, save_one, suggested_name, tr, trf, trn, vec2, window,
};

/// A `.fig` Save as result is an export; only native saves report that the document was saved.
pub(crate) fn file_saved(app: &mut FigApp, result: &Value) {
    if result["format"] == "fig" {
        fig_exported(app, result);
    } else {
        app.toast(trf("Saved {name}", &[("name", &result_file_name(result))]));
    }
}

fn result_file_name(result: &Value) -> String {
    match result["path"].as_str() {
        Some(p) => Path::new(p).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        // A download (#969).
        None => result["name"].as_str().unwrap_or_default().to_owned(),
    }
}

/// The `.fig` result stays an export: native save state is unchanged, and omissions are reported.
pub(crate) fn fig_exported(app: &mut FigApp, result: &Value) {
    let file = result_file_name(result);
    let left_out = result["warnings"].as_array().map_or(0, Vec::len);
    app.toast(match left_out {
        0 => trf("Exported {name}", &[("name", &file)]),
        n => trn("Exported {name}; {n} kinds of content were left out", n, &[("name", &file)]),
    });
}

/// Save the whole file as `.fig`; the toast says how much was left out.
fn save_fig(app: &mut FigApp, name: &str) {
    let filename = format!("{name}.fig");
    if app.services.downloads() {
        if let Some(mut r) = app.run("file.export", json!({"format": "fig"}))
            && app.download(&filename, &r)
        {
            r["name"] = json!(filename);
            fig_exported(app, &r);
        }
        return;
    }
    app.pick_save(tr("Export"), &filename, &["fig"], |app, p| {
        if let Some(r) = app.run("file.export", json!({"format": "fig", "path": p.to_string_lossy()})) {
            fig_exported(app, &r);
        }
    });
}

/// What the Export dialog was asked to do.
enum ExportChoice {
    /// Every layer with export settings, with those settings.
    Layers,
    /// The layers together as one file.
    Once(ExportSetting),
    /// The whole file as `.fig` (experimental).
    Fig,
}

/// The Export dialog: the layers' own export settings when they have some, and one-off presets for
/// the selection (or the page) as one file.
pub(super) fn export_dialog(app: &mut FigApp, ctx: &Context) {
    let t = Tokens::get(ctx);
    let mut open = true;
    let sel = app.session.selection();
    let Ok(ds) = app.session.ds() else {
        app.ui.export_preview = false;
        return;
    };
    let (doc, page) = (ds.doc.clone(), ds.page);
    let ids = if sel.is_empty() { doc.node(page).map(|p| p.children.clone()).unwrap_or_default() } else { sel.clone() };
    // With nothing selected, every layer on the page that has export settings.
    let with_settings = if sel.is_empty() { exportable_layers(&doc, page) } else { sel };
    let jobs = export_jobs(&doc, &with_settings);
    let print = app.ui.pdf_print;
    let preset = |size: f64, format| ExportSetting { size: ExportSize::Scale { value: size }, format, print, ..Default::default() };
    let presets = [
        preset(1.0, ExportFormat::Png),
        preset(2.0, ExportFormat::Png),
        preset(3.0, ExportFormat::Png),
        preset(2.0, ExportFormat::Jpg),
        preset(2.0, ExportFormat::Webp),
        preset(1.0, ExportFormat::Svg),
        preset(1.0, ExportFormat::Pdf),
        preset(1.0, ExportFormat::VectorDrawable),
    ];
    let mut choice = None;
    window(tr("Export"), ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(320.0);
        let caption = |ui: &mut egui::Ui, s: &str| {
            ui.label(RichText::new(s).font(regular(11.0)).color(t.text_secondary));
        };
        let divider = |ui: &mut egui::Ui| {
            ui.add_space(10.0);
            let (r, _) = ui.allocate_exact_size(vec2(320.0, 1.0), Sense::hover());
            ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.0, t.border));
            ui.add_space(10.0);
        };
        let row_tip = |ui: &mut egui::Ui, label: &str, tip: Option<&str>| {
            let (r, resp) = ui.allocate_exact_size(vec2(320.0, 30.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
            }
            ui.painter().text(r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, label, regular(12.0), t.text);
            match tip {
                Some(tip) => resp.on_hover_text(tip).clicked(),
                None => resp.clicked(),
            }
        };
        let row = |ui: &mut egui::Ui, label: &str| row_tip(ui, label, None);
        if !jobs.is_empty() {
            caption(ui, tr("With the layers' export settings"));
            const SHOWN: usize = 6;
            for j in jobs.iter().take(SHOWN) {
                ui.label(RichText::new(&j.file).font(regular(12.0)).color(t.text));
            }
            if jobs.len() > SHOWN {
                caption(ui, &trn("and {n} more", jobs.len() - SHOWN, &[]));
            }
            ui.add_space(6.0);
            let layers = jobs.iter().map(|j| j.id).collect::<std::collections::BTreeSet<_>>().len();
            let label = trn("Export {n} layers", layers, &[]);
            if crate::widgets::button(ui, &label, true, true).clicked() {
                choice = Some(ExportChoice::Layers);
            }
            divider(ui);
        }
        caption(ui, &trn("{n} layers as one file", ids.len(), &[]));
        ui.add_space(4.0);
        for p in &presets {
            let label = if p.format.is_raster() { format!("{} @{}", p.format.label(), p.size.label()) } else { p.format.label().to_string() };
            if row_tip(ui, &label, crate::export_panel::format_tip(p.format)) {
                choice = Some(ExportChoice::Once(p.clone()));
            }
        }
        // The PDF preset's print options.
        ui.add_space(6.0);
        caption(ui, tr("PDF print options"));
        if let Some(next) = crate::export_panel::print_options(ui, egui::Id::unique("export_dialog_print"), &print) {
            app.ui.pdf_print = next;
        }
        divider(ui);
        caption(ui, tr("The whole file"));
        ui.add_space(4.0);
        if row(ui, tr(".fig (experimental)")) {
            choice = Some(ExportChoice::Fig);
        }
        caption(ui, tr("Other apps may not open every .fig file exported here."));
    });
    let close = choice.is_some() || !open;
    match choice {
        Some(ExportChoice::Layers) => export_layers(app, &with_settings, false),
        Some(ExportChoice::Once(s)) if !ids.is_empty() => save_one(app, &ids, &s, &suggested_name(&doc, &ids, &s, "Export")),
        Some(ExportChoice::Once(_)) => app.toast(tr("Nothing to export")),
        Some(ExportChoice::Fig) => save_fig(app, &doc.name),
        None => {}
    }
    if close {
        app.ui.export_preview = false;
    }
}
