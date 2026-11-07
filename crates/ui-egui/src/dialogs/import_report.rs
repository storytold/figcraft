//! The import report: what a successful `.fig`, REST JSON or SVG import left out or changed, shown
//! in a dialog window after the file opens, so the notes don't vanish with a toast.

use egui::RichText;
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::{tr, trn, try_tr};
use crate::theme::{Tokens, regular};

/// A report stays small even when a damaged input produces many or very long notes.
pub const MAX_NOTES: usize = 500;
pub const MAX_NOTE_CHARS: usize = 2_000;

#[derive(Clone, Debug)]
pub struct Report {
    pub name: String,
    pub pages: Option<u64>,
    pub layers: Option<u64>,
    pub images: Option<u64>,
    /// The pages of an imported file were added to the open document.
    pub appended: bool,
    /// An SVG was placed on the current page of the open document.
    pub placed: bool,
    /// The document the report belongs to: it goes when that document closes.
    pub uid: u64,
    pub notes: Vec<String>,
    /// `notes` as the window shows them, in the UI language (worked out once, not every frame).
    pub shown: Vec<String>,
    pub note_count: usize,
}

impl Report {
    /// Only successful `file.open` and `file.import` results with import `warnings` create a report;
    /// a native file's `repairs` are shown by the engine's own message, and an SVG opened without
    /// notes has nothing to report. `params` are the command's, for the file's name.
    pub fn from_result(command: &str, params: &Value, result: &Value) -> Option<Self> {
        if !matches!(command, "file.open" | "file.openRecent" | "file.import" | "edit.pasteDesignLayers") {
            return None;
        }
        let warnings = result.get("warnings").and_then(Value::as_array)?;
        // A paste is reported when something was left out or changed, as an import into the page.
        let pasted = command == "edit.pasteDesignLayers";
        if pasted && warnings.is_empty() {
            return None;
        }
        let file_name = params.get("path").and_then(Value::as_str).and_then(|p| std::path::Path::new(p).file_name()).and_then(|n| n.to_str());
        if command != "file.import" && warnings.is_empty() && file_name.is_some_and(|n| n.to_ascii_lowercase().ends_with(".svg")) {
            return None;
        }
        // `file.import` of an SVG places a frame on the current page (no `pages`): its notes
        // are reported, but no pages were appended.
        let appended = command == "file.import" && result.get("pages").is_some();
        if command == "file.import" && !appended && warnings.is_empty() {
            return None;
        }
        let summary = result.get("summary");
        let count = |key: &str| summary.and_then(|s| s.get(key)).and_then(Value::as_u64);
        let name = summary
            .and_then(|s| s.get("name"))
            .and_then(Value::as_str)
            .or(file_name)
            .or_else(|| params.get("name").and_then(Value::as_str))
            .unwrap_or(if pasted { tr("Pasted design layers") } else { tr("Imported document") });
        let mut report = Self {
            name: name.chars().take(512).collect(),
            pages: count("pages"),
            layers: count("layers"),
            images: count("images"),
            appended,
            placed: (command == "file.import" && !appended) || pasted,
            uid: 0,
            notes: Vec::new(),
            shown: Vec::new(),
            note_count: 0,
        };
        for note in warnings.iter().filter_map(Value::as_str) {
            report.add_note(note);
        }
        Some(report)
    }

    fn add_note(&mut self, note: &str) {
        self.note_count = self.note_count.saturating_add(1);
        if self.notes.len() < MAX_NOTES {
            let mut text: String = note.chars().take(MAX_NOTE_CHARS).collect();
            if note.chars().nth(MAX_NOTE_CHARS).is_some() {
                text.push('…');
            }
            self.shown.push(note_text(&text));
            self.notes.push(text);
        }
    }

    pub fn inspect(&self) -> Value {
        json!({"name": self.name, "pages": self.pages, "layers": self.layers, "images": self.images,
            "appended": self.appended, "placed": self.placed, "warnings": self.notes, "warningCount": self.note_count})
    }
}

/// `note` in the UI language when the catalog has it (the importers' fixed notes: `.fig` imports
/// name "the file" where REST imports name the JSON they read); a repeat count (` (×N)`) is kept.
/// The stored notes are lower-case fragments; a known note is shown as a sentence, so its first
/// letter is capitalised here. Other notes (SVG's, say) are shown as they are.
pub(crate) fn note_text(note: &str) -> String {
    let translated = |base: &str| {
        let Some(text) = try_tr(base) else { return base.to_owned() };
        let mut chars = text.chars();
        chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
    };
    match note.rsplit_once(" (×") {
        Some((base, count)) if count.strip_suffix(')').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) => {
            format!("{} (×{count}", translated(base))
        }
        _ => translated(note),
    }
}

/// Show the report for a file the user opened or imported (menus, drops, pastes, recents, the
/// system's open event); commands from the control channel, MCP or scripts never call this.
pub fn capture(app: &mut FigApp, command: &str, params: &Value, result: &Value) {
    if let Some(mut report) = Report::from_result(command, params, result) {
        report.uid = app.session.ds().map_or(0, |d| d.uid);
        app.ui.import_report = Some(report);
    }
}

pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    // The report goes with its document.
    if app.ui.import_report.as_ref().is_some_and(|r| !app.session.docs.iter().any(|d| d.uid == r.uid)) {
        app.ui.import_report = None;
    }
    let Some(report) = &app.ui.import_report else { return };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut close_clicked = false;
    crate::dialogs::window(tr("Import report"), ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(RichText::new(&report.name).strong());
        ui.label(if report.appended {
            tr("Pages added to the document")
        } else if report.placed {
            tr("Placed on the current page")
        } else {
            tr("Opened as a new document")
        });
        let counts: Vec<String> = [(tr("Pages"), report.pages), (tr("Layers"), report.layers), (tr("Images"), report.images)]
            .into_iter()
            .filter_map(|(label, count)| count.map(|n| format!("{label}: {n}")))
            .collect();
        if !counts.is_empty() {
            ui.label(counts.join(" · "));
        }
        ui.add_space(8.0);
        if report.notes.is_empty() {
            ui.label(tr("No compatibility warnings reported"));
        } else {
            ui.label(RichText::new(tr("Compatibility notes")).strong());
            ui.label(
                RichText::new(tr(
                    "FigCraft doesn't support every feature of these files yet. The notes list what was left out or changed in the imported copy; the original file is unchanged.",
                ))
                .color(t.text_secondary)
                .font(regular(11.0)),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("import_report_notes").max_height(260.0).show(ui, |ui| {
                for note in &report.shown {
                    ui.label(note);
                    ui.add_space(4.0);
                }
            });
        }
        let omitted = report.note_count.saturating_sub(report.notes.len());
        if omitted > 0 {
            ui.label(trn("{n} more notes omitted from this report", omitted, &[]));
        }
        ui.add_space(12.0);
        close_clicked = crate::widgets::button(ui, tr("Close"), true, true).clicked();
    });
    if !open || close_clicked {
        app.ui.import_report = None;
    }
}
