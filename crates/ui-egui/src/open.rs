#[cfg(target_arch = "wasm32")]
use super::services::browser_can_read;
use super::services::{ReadFuture, unreadable};
use super::{OsStr, Path, theme};

/// The file types File ▸ Open and a drop on the home screen open.
pub(crate) const OPEN_EXTENSIONS: &[&str] =
    &["figcraft", "fig", "jam", "penpot", "sketch", "json", "svg", "png", "jpg", "jpeg", "webp", "gif", "tif", "tiff"];

/// Whether `path` names a file type that opens as a document (the extension, in any case).
pub(crate) fn opens(path: &Path) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|e| OPEN_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Whether `path` is a whole document (`.figcraft`, `.fig`, `.jam`, `.penpot`, `.sketch`, file JSON) rather than an image or SVG:
/// dropped on the canvas it opens as a file of its own instead of adding its pages to the open one.
pub(crate) fn is_document(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|e| ["figcraft", "fig", "jam", "penpot", "sketch", "json"].iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Reads the contents of dropped file `name` that has no file on disk to read by path.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn dropped_bytes(f: &egui::DroppedFileHandle, name: &str) -> ReadFuture {
    Box::pin(std::future::ready(f.bytes().map_err(|_| unreadable(name))))
}

/// Reads the contents of dropped file `name`; the browser hands them over asynchronously.
#[cfg(target_arch = "wasm32")]
pub(crate) fn dropped_bytes(f: &egui::DroppedFileHandle, name: &str) -> ReadFuture {
    let (f, name) = (f.clone(), name.to_string());
    Box::pin(async move {
        if let Some(file) = f.web_file() {
            browser_can_read(&name, file.size())?;
        }
        f.bytes_async().await.map_err(|_| unreadable(&name))
    })
}

/// While files are dragged over the window: a tinted, outlined `rect` with `label` in its middle,
/// over everything else, saying what a drop does there.
pub(crate) fn drop_overlay(ctx: &egui::Context, rect: egui::Rect, label: &str) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let t = theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop_overlay")));
    let area = rect.shrink(4.0);
    painter.rect_filled(area, egui::CornerRadius::same(8), t.accent.gamma_multiply(0.12));
    painter.rect_stroke(area, egui::CornerRadius::same(8), egui::Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
    let galley = painter.layout_no_wrap(label.to_owned(), theme::semibold(14.0), t.text);
    let pill = egui::Rect::from_center_size(area.center(), galley.size() + egui::vec2(32.0, 16.0));
    painter.rect_filled(pill, egui::CornerRadius::same(8), t.panel);
    painter.rect_stroke(pill, egui::CornerRadius::same(8), egui::Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
    painter.galley(pill.center() - galley.size() / 2.0, galley, t.text);
    // Hovering sends no pointer events on every platform: keep the overlay current.
    ctx.request_repaint();
}
