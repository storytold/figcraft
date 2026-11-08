//! The Memory usage window (View ▸ Memory usage…): what the open file and the app's
//! bookkeeping take. Read-only; recomputed only while the window draws.

use egui::Context;
use figcraft_engine::widgets;

use crate::FigApp;
use crate::dialogs::window;
use crate::i18n::tr;
use crate::theme::Tokens;

/// Draw the window while `app.ui.memory_window` is set.
pub fn show(app: &mut FigApp, ctx: &Context) {
    if !app.ui.memory_window {
        return;
    }
    let mut open = true;
    window(tr("Memory usage"), ctx).id(egui::Id::unique("MemoryUsage")).open(&mut open).show(ctx, |ui| {
        ui.set_min_width(320.0);
        let t = Tokens::get(ui.ctx());
        let Ok(ds) = app.session.ds() else {
            ui.label(tr("No file is open."));
            return;
        };
        let bytes = serde_json::to_vec(ds.doc.as_ref()).map(|v| v.len()).unwrap_or(0);
        let nodes = ds.doc.ids().count();
        let images = ds.doc.images.len();
        let widget_layers = ds.doc.ids().filter(|i| ds.doc.node(*i).is_some_and(widgets::is_widget)).count();
        let (undo, redo) = app.session.undo_depth();
        egui::Grid::new("memory_rows").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label(tr("File in memory"));
            ui.label(bytes_label(bytes));
            ui.end_row();
            ui.label(tr("Layers"));
            ui.label(nodes.to_string());
            ui.end_row();
            ui.label(tr("Images"));
            ui.label(images.to_string());
            ui.end_row();
            ui.label(tr("Widget layers"));
            ui.label(widget_layers.to_string());
            ui.end_row();
            ui.label(tr("Undo steps kept"));
            ui.label(undo.to_string());
            ui.end_row();
            ui.label(tr("Redo steps kept"));
            ui.label(redo.to_string());
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        let times = app.frame_stats.times();
        egui::Grid::new("memory_frames").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label(tr("Frames drawn"));
            ui.label(app.frame_stats.frames.to_string());
            ui.end_row();
            ui.label(tr("Frames timed"));
            ui.label(times["frames"].to_string());
            ui.end_row();
            ui.label(tr("Mean frame"));
            ui.label(format!("{:.1} ms", times["meanMs"].as_f64().unwrap_or(0.0)));
            ui.end_row();
            ui.label(tr("Slowest frame"));
            ui.label(format!("{:.1} ms", times["maxMs"].as_f64().unwrap_or(0.0)));
            ui.end_row();
        });
        ui.add_space(8.0);
        crate::widgets::label(
            ui,
            tr("The file's size is its saved form in memory; image caches and the graphics memory behind the canvas aren't counted."),
            t.text_secondary,
        );
    });
    app.ui.memory_window = open;
}

/// Bytes as `1.2 MB`-style text.
fn bytes_label(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bytes_label_formats_units() {
        assert_eq!(super::bytes_label(512), "512 B");
        assert_eq!(super::bytes_label(2048), "2.0 KB");
        assert_eq!(super::bytes_label(2 * 1024 * 1024), "2.0 MB");
    }
}
