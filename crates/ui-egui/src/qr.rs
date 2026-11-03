//! QR codes drawn in the window (Share ▸ View on another device shows the address a phone opens
//! as one). Encoded with `qrcodegen`; drawn as dark runs on a light square with the standard quiet
//! zone, in fixed black and white whatever the theme (cameras read dark on light), with cells on
//! whole pixels so nothing blurs.

use std::sync::Arc;

use egui::{Color32, CornerRadius, Id, Rect, Sense, Ui, pos2, vec2};
use qrcodegen::{QrCode, QrCodeEcc};

/// Light cells around the code, on each side (the standard's quiet zone).
const QUIET: i32 = 4;

/// The code's cells: `side` rows of dark (true) and light cells, without the quiet zone. `None`:
/// `text` is too long for a QR code.
pub fn cells(text: &str) -> Option<Vec<Vec<bool>>> {
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let n = qr.size();
    Some((0..n).map(|y| (0..n).map(|x| qr.get_module(x, y)).collect()).collect())
}

/// Draw the QR code of `text` at most `side` points wide; false when it is too long for one
/// (nothing is drawn).
pub fn show(ui: &mut Ui, text: &str, side: f32) -> bool {
    // Encoded once per text, not each frame the popover draws.
    let id = Id::unique(("qr", text));
    let rows = match ui.data(|d| d.get_temp::<Arc<Option<Vec<Vec<bool>>>>>(id)) {
        Some(rows) => rows,
        None => {
            let rows = Arc::new(cells(text));
            ui.data_mut(|d| d.insert_temp(id, rows.clone()));
            rows
        }
    };
    let Some(rows) = rows.as_ref() else { return false };
    let n = i32::try_from(rows.len()).unwrap_or(i32::MAX).saturating_add(2 * QUIET);
    let ppp = ui.ctx().pixels_per_point();
    // Each cell a whole number of pixels.
    let cell = ((side * ppp / n as f32).floor().max(1.0)) / ppp;
    let full = cell * n as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(full, full), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), Color32::WHITE);
    let origin = rect.min + vec2(QUIET as f32 * cell, QUIET as f32 * cell);
    for (y, row) in rows.iter().enumerate() {
        // Each run of dark cells as one rectangle.
        let mut x = 0;
        while x < row.len() {
            if !row.get(x).copied().unwrap_or(false) {
                x += 1;
                continue;
            }
            let start = x;
            while row.get(x).copied().unwrap_or(false) {
                x += 1;
            }
            let min = origin + vec2(start as f32 * cell, y as f32 * cell);
            painter.rect_filled(Rect::from_min_max(min, pos2(origin.x + x as f32 * cell, min.y + cell)), CornerRadius::ZERO, Color32::BLACK);
        }
    }
    true
}
