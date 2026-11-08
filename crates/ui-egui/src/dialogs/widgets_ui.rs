//! The widgets window (Widgets ▸ Manage widgets…): the widgets in the open file, with
//! select-all. Read-only listing; actions run commands.

use egui::Context;
use figcraft_engine::widgets;
use serde_json::json;

use crate::FigApp;
use crate::dialogs::window;
use crate::i18n::tr;
use crate::theme::Tokens;

/// Draw the window while `app.ui.widgets_window` is set.
pub fn show(app: &mut FigApp, ctx: &Context) {
    if !app.ui.widgets_window {
        return;
    }
    let mut open = true;
    window(tr("Widgets"), ctx).id(egui::Id::unique("WidgetsWindow")).open(&mut open).show(ctx, |ui| {
        ui.set_min_width(340.0);
        let t = Tokens::get(ui.ctx());
        let Ok(ds) = app.session.ds() else {
            ui.label(tr("No file is open."));
            return;
        };
        let rows: Vec<(String, String)> = ds
            .doc
            .ids()
            .filter_map(|i| {
                let n = ds.doc.node(i)?;
                widgets::is_widget(n).then(|| (n.name.clone(), n.plugin_data.get(widgets::PLUGIN).cloned().unwrap_or_default()))
            })
            .collect();
        if rows.is_empty() {
            ui.label(tr("No widgets in this file. A widget plugin draws one on the canvas."));
            return;
        }
        ui.label(format!("{}: {}", tr("Widgets in this file"), rows.len()));
        ui.add_space(6.0);
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            egui::Grid::new("widget_rows").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                for (name, plugin) in rows {
                    ui.label(name);
                    ui.label(plugin);
                    ui.end_row();
                }
            });
        });
        ui.add_space(10.0);
        if ui.button(tr("Select all widgets")).clicked() {
            app.run("widgets.selectAll", json!({}));
        }
        ui.add_space(4.0);
        crate::widgets::label(ui, tr("Widgets redraw themselves from their plugin's state; use the plugin's panel to change one."), t.text_secondary);
    });
    app.ui.widgets_window = open;
}
