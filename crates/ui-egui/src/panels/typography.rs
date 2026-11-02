//! Typography section of the Design panel: font family (searchable, previewed, recent first),
//! style, size (with common sizes), line height, letter spacing, paragraph spacing, resizing,
//! alignment, and the Type settings popover (decoration, case, lists, indent, truncation, OpenType
//! features and variable font axes). While text is being edited with a range selected, character
//! edits apply to the range (`text.style`, `text.setVariations`); otherwise they apply to whole
//! layers. Differing values show "Mixed".

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::{
    DecorationLength, Document, LeadingTrim, LetterSpacing, LineHeight, ListStyle, NodeId, NodeKind, TextAlignH, TextAlignV, TextAutoResize,
    TextCase, TextData, TextDecoration, TextStyle, VariableType, trim_num,
};
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::{tr, tr_noop, trf};
use crate::icons::{self, Icon};
use crate::right::{common, menu_item, menu_popup, row};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, FIELD_H, FieldEdit, Lead};

mod axes;
mod fields;
mod glyphs;
mod picker;
mod spacing;

use fields::{font_rows, paragraph_rows};
pub(crate) use picker::font_picker;
pub(crate) use spacing::{letter_spacing_of, line_height_of, parse_letter_spacing, parse_line_height};

/// Font sizes offered by the size dropdown.
const SIZES: [f64; 15] = [10.0, 11.0, 12.0, 13.0, 14.0, 16.0, 20.0, 24.0, 32.0, 36.0, 40.0, 48.0, 64.0, 96.0, 128.0];

/// OpenType features offered in Type settings ▸ Details: (tag, label, on by default in fonts).
const FEATURES: [(&str, &str, bool); 12] = [
    ("liga", tr_noop("Standard ligatures"), true),
    ("calt", tr_noop("Contextual alternates"), true),
    ("tnum", tr_noop("Tabular numbers"), false),
    ("frac", tr_noop("Fractions"), false),
    ("case", tr_noop("Case-sensitive forms"), false),
    ("zero", tr_noop("Slashed zero"), false),
    ("ss01", tr_noop("Stylistic set 1"), false),
    ("ss02", tr_noop("Stylistic set 2"), false),
    ("ss03", tr_noop("Stylistic set 3"), false),
    ("ss04", tr_noop("Stylistic set 4"), false),
    ("ss05", tr_noop("Stylistic set 5"), false),
    ("smcp", tr_noop("Small capitals"), false),
];

/// Shown for differing values (translated where shown: `tr(MIXED)`).
const MIXED: &str = tr_noop("Mixed");

/// The Type settings tabs (Variable only for variable fonts).
const TABS: [&str; 3] = [tr_noop("Basics"), tr_noop("Details"), tr_noop("Variable")];

/// The value shared by every style, or None when they differ.
fn uniform<T: PartialEq + Clone>(styles: &[TextStyle], f: impl Fn(&TextStyle) -> T) -> Option<T> {
    let mut it = styles.iter();
    let first = f(it.next()?);
    if it.all(|s| f(s) == first) { Some(first) } else { None }
}

/// The character styles the panel shows: the edited range's spans, else every span of every
/// selected text layer.
pub(crate) fn target_styles(doc: &Document, ids: &[NodeId], range: Option<(NodeId, usize, usize)>) -> Vec<TextStyle> {
    let mut out: Vec<TextStyle> = Vec::new();
    let mut push = |s: &TextStyle| {
        if !out.contains(s) {
            out.push(s.clone());
        }
    };
    for id in ids {
        let Some(t) = doc.node(*id).and_then(|n| n.kind.text()) else { continue };
        match range {
            Some((rid, a, b)) if rid == *id && a < b => {
                for (s, e, st) in t.spans() {
                    if s < b && e > a {
                        push(st);
                    }
                }
            }
            _ => {
                for (_, _, st) in t.spans() {
                    push(st);
                }
            }
        }
    }
    out
}

pub fn typography(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) {
    let ids: Vec<NodeId> = sel.iter().copied().filter(|i| matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Text(_)))).collect();
    if ids.is_empty() {
        return;
    }
    // While editing a range, character edits apply to it.
    let range = figcraft_engine::textedit::selected_range(&app.session);
    let editing = range.is_some();
    let styles = target_styles(doc, &ids, range);
    let apply_style = |app: &mut FigApp, key: &str, v: Value| commit_text(app, &ids, range, json!({ key: v }));

    let hdr =
        widgets::section_header(ui, tr("Typography"), &[(Icon::Variable, tr("Apply variable to text")), (Icon::Styles, tr("Text styles"))], false);
    if hdr == Some(0) {
        crate::variable_ui::open_at_pointer(ui, &ids, "characters", VariableType::String);
    }
    if hdr == Some(1) {
        crate::style_picker::open(app, ui, &ids, crate::style_picker::StyleSlot::Text);
    }
    crate::variable_ui::bound_row(app, ui, &ids, "characters", VariableType::String, Lead::Icon(Icon::Text, tr("Text")), f32::INFINITY);
    // A text style: its chip stands for the font rows (they show again when the chars differ).
    crate::style_picker::chip(app, ui, doc, &ids, crate::style_picker::StyleSlot::Text);
    if !matches!(crate::style_picker::text_link(app, doc, &ids), Some(Some(_))) {
        font_rows(app, ui, &ids, &styles, editing, &apply_style);
    }
    paragraph_rows(app, ui, doc, &ids, &styles, &apply_style);
    path_row(app, ui, doc, sel, &ids);
}

/// Text on a path: where the text starts, which side it runs on, and taking it off the path; or,
/// with a text and a path selected, putting the text on the path.
fn path_row(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId], ids: &[NodeId]) {
    let paths: Option<Vec<(f64, bool)>> =
        ids.iter().map(|i| doc.node(*i).and_then(|n| n.kind.text()).and_then(|t| t.path.as_ref()).map(|p| (p.start(), p.flip))).collect();
    let Some(paths) = paths else {
        let other = sel.iter().find(|i| !ids.contains(i)).and_then(|i| doc.node(*i));
        if ids.len() == 1 && sel.len() == 2 && other.is_some_and(|n| figcraft_doc::geometry::stroke_geometry(n).is_some()) {
            row(ui, 32.0, |ui| {
                if widgets::button(ui, tr("Put text on path"), false, true).clicked() {
                    app.run("text.onPath", json!({ "ids": sel }));
                }
            });
        }
        return;
    };
    let start = paths.first().map(|p| p.0).filter(|s| paths.iter().all(|p| p.0 == *s));
    let flip = paths.iter().all(|p| p.1);
    row(ui, 32.0, |ui| {
        let lead = Lead::Icon(Icon::Pen, tr("Start on path"));
        match widgets::number_field(ui, Id::unique("ty_path_start"), lead, start.map(|s| s * 100.0), "%", 96.0, 1.0) {
            Some(FieldEdit::Set(v)) => {
                app.run("text.pathOptions", json!({"ids": ids, "start": v / 100.0}));
            }
            Some(FieldEdit::Scrub(v, done)) => live(app, "Start on path", "text.pathOptions", json!({"ids": ids, "start": v / 100.0}), done),
            None => {}
        }
        if icons::button(ui, Icon::FlipV, 24.0, flip, tr("Flip text to the other side")).clicked() {
            app.run("text.pathOptions", json!({"ids": ids, "flip": !flip}));
        }
        if icons::button(ui, Icon::Detach, 24.0, false, tr("Remove text from path")).clicked() {
            app.run("text.removeFromPath", json!({ "ids": ids }));
        }
    });
}

/// Set character `props` on the selected chars of the text being edited, else on whole layers
/// (through `node.set`, which records overrides inside instances). Paragraph spacing is always
/// the whole layer's: `text.style` doesn't take it.
pub(crate) fn commit_text(app: &mut FigApp, ids: &[NodeId], range: Option<(NodeId, usize, usize)>, props: Value) {
    match range {
        Some((_, a, b)) if a != b && props.get("paragraphSpacing").is_none() => {
            app.run("text.style", text_style_params(ids, range, props));
        }
        _ => app.set_props(ids, props),
    }
}

/// `text.style` params for `props` on the selected chars of the text being edited, else on the
/// whole of every layer in `ids` (previews; commits go through [`commit_text`]).
pub(crate) fn text_style_params(ids: &[NodeId], range: Option<(NodeId, usize, usize)>, mut props: Value) -> Value {
    match range {
        Some((id, a, b)) if a != b => {
            props["id"] = json!(id);
            props["start"] = json!(a);
            props["end"] = json!(b);
        }
        _ => props["ids"] = json!(ids),
    }
    props
}

/// Run `cmd` as a live preview while scrubbing; the gesture becomes one undo step when `done`.
/// (While text is edited, run commands directly: the editing session already groups undo.)
pub(crate) fn live(app: &mut FigApp, label: &str, cmd: &str, params: Value, done: bool) {
    let _ = app.session.begin_interaction(label);
    if let Err(e) = app.session.execute(cmd, &params) {
        app.toast(e.to_string());
    }
    if done {
        let _ = app.session.commit_interaction();
    }
}

/// A Type settings row: a label, then `add` with the rest of the width. A label wider than its
/// column (French, say) narrows the field rather than the row outgrowing the popover, which then
/// widened every frame (#1029).
pub(crate) fn field_row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui, f32) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, FIELD_H), egui::Label::new(egui::RichText::new(label).font(regular(11.0)).color(t.text)));
        add(ui, ui.available_width())
    })
    .inner
}

#[cfg(test)]
mod variable_tests {
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};
    use figcraft_engine::Session;

    use super::*;
    use crate::NoServices;

    /// The Typography section and the variable picker, drawn for a selected text layer (Inter
    /// Regular) in a file with the string variables "Heading font" and "Heading weight" and the
    /// number variable "Heading size".
    struct Panel {
        app: FigApp,
        ctx: egui::Context,
        time: f64,
        text: NodeId,
    }

    impl Panel {
        fn new() -> Self {
            let mut s = Session::new();
            let text = NodeId(s.execute("text.create", &json!({"text": "Title"})).unwrap()["id"].as_u64().unwrap());
            s.execute("variable.createCollection", &json!({"name": "Type", "modes": ["Base"]})).unwrap();
            for (name, ty, value) in
                [("Heading font", "string", json!("Inter")), ("Heading weight", "string", json!("Bold")), ("Heading size", "number", json!(32))]
            {
                s.execute("variable.create", &json!({"collection": "Type", "name": name, "type": ty, "values": {"Base": value}})).unwrap();
            }
            s.execute("select.set", &json!({ "ids": [text] })).unwrap();
            let mut p = Self { app: FigApp::new(s, Box::new(NoServices)), ctx: egui::Context::default(), time: 0.0, text };
            crate::theme::install_fonts(&p.ctx, false, 0);
            crate::theme::apply(&p.ctx, Tokens::light());
            for _ in 0..3 {
                p.frame(vec![]);
            }
            p
        }

        /// One frame with `events`; the texts drawn, with their centers.
        fn frame(&mut self, events: Vec<Event>) -> Vec<(String, Pos2)> {
            fn walk(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
                match shape {
                    egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.size() / 2.0)),
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    _ => {}
                }
            }
            self.time += 1.0 / 60.0;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut out = self.ctx.run_ui(input, |ui| {
                let ds = self.app.session.ds().unwrap();
                let doc = ds.doc.clone();
                let sel = ds.selection.clone();
                typography(&mut self.app, ui, &doc, &sel);
                crate::variable_ui::picker(&mut self.app, ui.ctx());
            });
            out.textures_delta.clear();
            let mut texts = Vec::new();
            for c in out.shapes {
                walk(&c.shape, &mut texts);
            }
            texts
        }

        /// Hover `p`, then press and release there; the texts drawn after.
        fn click(&mut self, p: Pos2) -> Vec<(String, Pos2)> {
            self.frame(vec![Event::PointerMoved(p)]);
            for pressed in [true, false] {
                self.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
            }
            self.frame(vec![])
        }

        /// The name of the variable `field` of the text layer is bound to.
        fn bound_to(&self, field: &str) -> Option<String> {
            let doc = self.app.session.doc().unwrap();
            doc.node(self.text).unwrap().bound.get(field).and_then(|v| doc.variables.get_by_id(v)).map(|v| v.name.clone())
        }
    }

    fn at(texts: &[(String, Pos2)], s: &str) -> Pos2 {
        texts
            .iter()
            .find(|(t, _)| t == s)
            .map(|(_, p)| *p)
            .unwrap_or_else(|| panic!("no text {s:?} in {:?}", texts.iter().map(|t| &t.0).collect::<Vec<_>>()))
    }

    fn shows(texts: &[(String, Pos2)], s: &str) -> bool {
        texts.iter().any(|(t, _)| t == s)
    }

    #[test]
    fn font_family_and_style_menus_start_with_apply_variable() {
        let mut p = Panel::new();
        for (field, shown, name) in [("fontStyle", "Regular", "Heading weight"), ("fontFamily", "Inter", "Heading font")] {
            let texts = p.frame(vec![]);
            let texts = p.click(at(&texts, shown));
            let texts = p.click(at(&texts, "Apply variable"));
            assert!(crate::variable_ui::picker_open(&p.ctx), "{field}: the variable picker opens");
            assert!(shows(&texts, "Heading font") && shows(&texts, "Heading weight"), "{field} offers string variables");
            assert!(!shows(&texts, "Heading size"), "{field} offers no number variables");
            p.click(at(&texts, name));
            assert_eq!(p.bound_to(field).as_deref(), Some(name));
        }
        assert!(p.app.ui.last_error.is_none(), "{:?}", p.app.ui.last_error);
    }

    /// Asserts bound font family and style chips replace the dropdowns, a selected text range
    /// offers no variable binding, and detaching restores the last values.
    #[test]
    fn bound_font_family_and_style_show_their_variables_and_detach() {
        let mut p = Panel::new();
        let ids = vec![p.text];
        for (field, var) in [("fontFamily", "Heading font"), ("fontStyle", "Heading weight")] {
            p.app.run("variable.bind", json!({"ids": ids, "field": field, "variable": var}));
        }
        let texts = p.frame(vec![]);
        assert!(shows(&texts, "Heading font") && shows(&texts, "Heading weight"), "the chips are drawn in place of the dropdowns");
        assert!(!shows(&texts, "Inter"), "the family dropdown is not drawn");
        // While characters are selected in the text, the dropdowns show the values, with no Apply variable.
        figcraft_engine::textedit::begin(&mut p.app.session, p.text, Some(1), false).unwrap();
        figcraft_engine::textedit::set_caret(&mut p.app.session, 3, true).unwrap();
        let texts = p.frame(vec![]);
        let texts = p.click(at(&texts, "Inter"));
        assert!(!shows(&texts, "Apply variable"), "a selected range takes a font, not a variable");
        p.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        figcraft_engine::textedit::end(&mut p.app.session).unwrap();
        p.frame(vec![]);
        for field in ["fontFamily", "fontStyle"] {
            let rect = p.ctx.read_response(Id::unique(("bound_chip", field, &ids[..])).with("bound")).expect("the chip").rect;
            // The chip's detach button sits at its right end.
            p.click(Pos2::new(rect.max.x - 11.0, rect.center().y));
            assert_eq!(p.bound_to(field), None, "{field} is detached");
        }
        let texts = p.frame(vec![]);
        assert!(shows(&texts, "Inter") && shows(&texts, "Bold"), "the dropdowns show the last values");
        assert!(p.app.ui.last_error.is_none(), "{:?}", p.app.ui.last_error);
    }
}
