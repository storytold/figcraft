//! A colour picked with the eyedropper while holding Cmd/Ctrl+Shift becomes a colour style or a
//! colour variable, as in the reference app: the eyedropper asks for this window
//! (`dialog.createFromColor`), which asks which to make, what to call it and (a variable) in which
//! collection, then runs `style.create` or `variable.create`.

use egui::{Context, RichText};
use figcraft_color::{Color, Paint};
use serde_json::{Value, json};

use super::window;
use crate::FigApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use crate::widgets;

/// The window's state: the colour, what to make of it and its name.
#[derive(Clone, Debug, PartialEq)]
pub struct SampledColor {
    /// The document it was picked in: the window goes when another is in front.
    pub uid: u64,
    /// `#rrggbb`.
    pub color: String,
    /// A variable (else a style).
    pub variable: bool,
    pub name: String,
    /// The local collection a variable goes in (`None`: a new one).
    pub collection: Option<String>,
}

/// `dialog.createFromColor {color}` from the eyedropper: the window for the document in front, with
/// the next free style name ("Color 1"…) and its first collection.
pub fn request(app: &mut FigApp, data: &Value) {
    let Some(color) = data.get("color").and_then(Value::as_str).and_then(Color::from_hex) else { return };
    let Ok(ds) = app.session.ds() else { return };
    let collection = figcraft_engine::library::local_collections(&ds.doc).next().map(|c| c.id.clone());
    app.ui.sampled_color = Some(SampledColor {
        uid: ds.uid,
        color: color.to_css_hex(),
        variable: false,
        name: crate::style_picker::next_name(&ds.doc, "Color"),
        collection,
    });
}

/// The command that makes what `s` asks for: a paint style, a colour variable in its collection,
/// or (no collection) a new collection and the variable in it, one undo step.
pub fn create_params(app: &FigApp, s: &SampledColor) -> (&'static str, Value) {
    let name = s.name.trim();
    if !s.variable {
        return ("style.create", json!({"type": "paint", "name": name, "value": s.color}));
    }
    let variable = |collection: Value| json!({"collection": collection, "name": name, "type": "color", "value": s.color});
    match &s.collection {
        Some(c) => ("variable.create", variable(json!(c))),
        None => {
            let taken = |n: &str| app.session.doc().is_ok_and(|d| d.variables.collections.iter().any(|c| c.name == n));
            let collection = crate::variables_panel::unique("Collection", taken);
            let new = json!({"command": "variable.createCollection", "params": {"name": collection, "modes": ["Mode 1"]}, "as": "collection"});
            let add = json!({"command": "variable.create", "params": variable(json!({"$ref": "collection"}))});
            ("command.batch", json!({"label": "Create variable", "commands": [new, add]}))
        }
    }
}

/// A variable's collection: one of the file's, or a new one.
fn collection_picker(ui: &mut egui::Ui, s: &mut SampledColor, collections: &[(String, String)]) {
    let shown = s.collection.as_ref().and_then(|id| collections.iter().find(|c| &c.0 == id)).map_or(tr("New collection"), |c| c.1.as_str());
    let resp = widgets::dropdown(ui, shown, 260.0, None);
    crate::right::menu_popup(&resp).show(|ui| {
        for (id, label) in collections {
            if crate::right::menu_item(ui, label, "") {
                s.collection = Some(id.clone());
            }
        }
        if crate::right::menu_item(ui, tr("New collection"), "") {
            s.collection = None;
        }
    });
}

/// The window: a swatch of the colour, Style or Variable, its name, a variable's collection,
/// Cancel and Create (Enter creates).
pub fn show(app: &mut FigApp, ctx: &Context) {
    let Some(mut s) = app.ui.sampled_color.clone() else { return };
    if app.session.ds().map(|d| d.uid).ok() != Some(s.uid) {
        app.ui.sampled_color = None;
        return;
    }
    let t = Tokens::get(ctx);
    let collections: Vec<(String, String)> = app
        .session
        .doc()
        .map(|d| figcraft_engine::library::local_collections(d).map(|c| (c.id.clone(), c.name.clone())).collect())
        .unwrap_or_default();
    let (mut open, mut cancel, mut create) = (true, false, false);
    window(tr("Create from color"), ctx).id(egui::Id::unique("Create from color")).open(&mut open).show(ctx, |ui| {
        ui.set_width(280.0);
        ui.horizontal(|ui| {
            let paint = Color::from_hex(&s.color).map(Paint::solid);
            if let Some(p) = &paint {
                widgets::swatch(ui, p, 24.0);
            }
            ui.label(RichText::new(s.color.trim_start_matches('#').to_uppercase()).color(t.text));
        });
        ui.add_space(8.0);
        if widgets::radio(ui, !s.variable, tr("Style")) {
            s.variable = false;
        }
        if widgets::radio(ui, s.variable, tr("Variable")) {
            s.variable = true;
        }
        ui.add_space(6.0);
        let name = ui.add(egui::TextEdit::singleline(&mut s.name).hint_text(tr("Name")).desired_width(260.0));
        widgets::select_all_on_focus(ui, &name, &s.name);
        if ui.memory(|m| m.focused().is_none()) {
            name.request_focus();
        }
        if s.variable {
            ui.add_space(6.0);
            collection_picker(ui, &mut s, &collections);
        }
        ui.add_space(10.0);
        let ready = !s.name.trim().is_empty();
        ui.horizontal(|ui| {
            cancel = widgets::button(ui, tr("Cancel"), false, true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            let enter = name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            create = (widgets::button(ui, tr("Create"), true, ready).clicked() || enter) && ready;
        });
    });
    let made = create && {
        let (cmd, params) = create_params(app, &s);
        app.run(cmd, params).is_some()
    };
    // A name already taken leaves the window open to change it (the error says why).
    app.ui.sampled_color = (open && !cancel && !made).then_some(s);
}
