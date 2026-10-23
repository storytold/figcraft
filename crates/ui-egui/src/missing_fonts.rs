//! The Missing fonts window (#752): the fonts a file uses that aren't installed, whose text is
//! drawn in a fallback font. It opens by itself after the user opens a file that has any (once
//! the installed fonts are indexed and any import report is closed), and from Text ▸ Missing fonts.

use egui::RichText;
use figcraft_engine::fonts::{Missing, missing};
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::{tr, trn};
use crate::theme::{Tokens, regular};

#[derive(Clone, Debug)]
pub struct Shown {
    /// The document it lists: the window goes when that document closes.
    pub uid: u64,
    pub families: Vec<Missing>,
    /// Asked for before the installed fonts were indexed: the list comes once they are.
    pub indexing: bool,
}

impl Shown {
    pub fn inspect(&self) -> Value {
        json!({
            "families": self.families.iter().map(|m| json!({"family": m.family, "layers": m.layers})).collect::<Vec<_>>(),
            "indexing": self.indexing,
        })
    }
}

/// After the user opened a file, or imported pages into one (results with a `summary`; not a
/// placed image or SVG, a file that was open already, or Clear recent): look for missing fonts in
/// it once that can be told ([`show`]). Not in the browser, which has only its built-in fonts.
pub fn check_later(app: &mut FigApp, command: &str, result: Option<&Value>) {
    if !cfg!(target_arch = "wasm32")
        && matches!(command, "file.open" | "file.openRecent" | "file.import")
        && result.is_some_and(|r| r.get("summary").is_some())
        && let Ok(ds) = app.session.ds()
    {
        app.ui.font_checks.push(ds.uid);
    }
}

/// The window for document `uid`, listing what it misses (even nothing, when asked for).
fn list(app: &mut FigApp, uid: u64) -> Option<Vec<Missing>> {
    let ds = app.session.docs.iter().find(|d| d.uid == uid)?;
    let families = missing(&ds.doc);
    app.ui.missing_fonts = Some(Shown { uid, families: families.clone(), indexing: false });
    Some(families)
}

/// Text ▸ Missing fonts (`dialog.missingFonts`): the window for the file in front.
pub fn request(app: &mut FigApp) {
    let Ok(uid) = app.session.ds().map(|d| d.uid) else { return };
    if figcraft_text::installed_fonts_known() {
        list(app, uid);
    } else {
        app.ui.missing_fonts = Some(Shown { uid, families: Vec::new(), indexing: true });
    }
}

pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    // Joined server documents bypass file.open; check them once their tab is live.
    if let Ok(ds) = app.session.ds() {
        let uid = ds.uid;
        if app.session.live().status == figcraft_engine::live::Status::Live
            && app.session.live().info.as_ref().is_some_and(|info| info.role == figcraft_engine::live::Role::Guest)
            && app.ui.font_checked.insert(uid)
        {
            app.ui.font_checks.push(uid);
        }
    }
    // Whether the installed fonts are indexed is asked only while a list waits for it.
    let waiting = app.ui.missing_fonts.as_ref().filter(|m| m.indexing).map(|m| m.uid);
    if (waiting.is_some() || !app.ui.font_checks.is_empty()) && figcraft_text::installed_fonts_known() {
        if let Some(uid) = waiting {
            list(app, uid);
        }
        if !app.ui.font_checks.is_empty() && app.ui.import_report.is_none() && app.ui.missing_fonts.is_none() {
            let uid = app.ui.font_checks.remove(0);
            if list(app, uid).is_some_and(|f| f.is_empty()) {
                app.ui.missing_fonts = None;
            }
        }
    }
    if app.ui.missing_fonts.as_ref().is_some_and(|m| !app.session.docs.iter().any(|d| d.uid == m.uid)) {
        app.ui.missing_fonts = None;
    }
    let Some(shown) = &app.ui.missing_fonts else { return };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut close_clicked = false;
    crate::dialogs::window(tr("Missing fonts"), ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(380.0);
        if shown.indexing {
            ui.label(tr("Checking the fonts installed on this computer…"));
        } else if shown.families.is_empty() {
            ui.label(tr("All the fonts this file uses are installed"));
        } else {
            ui.label(
                RichText::new(tr(
                    "This file uses fonts that aren't installed on this computer. Saved text outlines preserve its appearance where available. Install the missing fonts or choose replacements before editing text.",
                ))
                .color(t.text_secondary)
                .font(regular(11.0)),
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical().id_salt("missing_fonts").max_height(260.0).show(ui, |ui| {
                for m in &shown.families {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&m.family).strong());
                        let used =
                            if m.layers == 0 { tr("only in a text style or variable").to_string() } else { trn("{n} text layers", m.layers, &[]) };
                        ui.label(RichText::new(used).color(t.text_secondary));
                    });
                }
            });
        }
        ui.add_space(12.0);
        close_clicked = crate::widgets::button(ui, tr("Close"), true, true).clicked();
    });
    if !open || close_clicked {
        app.ui.missing_fonts = None;
    }
}
