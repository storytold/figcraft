//! Typography section of the Design panel: font family (searchable, previewed, recent first),
//! style, size (with common sizes), line height, letter spacing, paragraph spacing, resizing,
//! alignment, and the Type settings popover (decoration, case, lists, indent, truncation, OpenType
//! features and variable font axes). While text is being edited with a range selected, character
//! edits apply to the range (`text.style`, `text.setVariations`); otherwise they apply to whole
//! layers. Differing values show "Mixed".

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::*;
use serde_json::{Value, json};

use crate::FigApp;
use crate::i18n::{tr, tr_noop, trf};
use crate::icons::{self, Icon};
use crate::right::{common, menu_item, menu_popup, row};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, FIELD_H, FieldEdit, Lead, PAD};

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
        match widgets::number_field(ui, Id::new("ty_path_start"), lead, start.map(|s| s * 100.0), "%", 96.0, 1.0) {
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
/// (through `node.set`, which records overrides inside instances).
fn commit_text(app: &mut FigApp, ids: &[NodeId], range: Option<(NodeId, usize, usize)>, props: Value) {
    match range {
        Some((_, a, b)) if a != b => {
            app.run("text.style", text_style_params(ids, range, props));
        }
        _ => app.set_props(ids, props),
    }
}

/// `text.style` params for `props` on the selected chars of the text being edited, else on the
/// whole of every layer in `ids` (previews; commits go through [`commit_text`]).
fn text_style_params(ids: &[NodeId], range: Option<(NodeId, usize, usize)>, mut props: Value) -> Value {
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

/// The font rows: family, style and size, line height and letter spacing.
fn font_rows(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], styles: &[TextStyle], editing: bool, apply_style: &dyn Fn(&mut FigApp, &str, Value)) {
    let t = Tokens::get(ui.ctx());
    let range = figcraft_engine::textedit::selected_range(&app.session);

    // Font family.
    let family = uniform(styles, |s| s.font_family.clone());
    let fstyle = uniform(styles, |s| s.font_style.clone());
    row(ui, 32.0, |ui| {
        let w = ui.available_width();
        let resp = widgets::dropdown(ui, family.as_deref().unwrap_or(tr(MIXED)), w, None);
        menu_popup(&resp).width(w.max(240.0)).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            let pick = font_picker(ui, family.as_deref(), resp.clicked());
            if let Some(f) = pick.picked {
                remember_family(ui.ctx(), &f);
                apply_style(app, "fontFamily", json!(f));
                ui.close();
            } else if let Some(f) = pick.highlighted {
                // The canvas shows the family the arrow keys are on.
                app.preview_text(ui.ctx(), "text.style", text_style_params(ids, range, json!({"fontFamily": f})));
            }
        });
    });

    // Style + size.
    let half = (ui.available_width() - PAD * 2.0 - 8.0) / 2.0;
    row(ui, 32.0, |ui| {
        let resp = widgets::dropdown(ui, fstyle.as_deref().unwrap_or(tr(MIXED)), half, None);
        let fam = family.clone();
        menu_popup(&resp).width(200.0).show(|ui| match &fam {
            Some(fam) => {
                let list = figcraft_text::family_styles(fam);
                if list.is_empty() {
                    widgets::label(ui, tr("Font not installed"), t.text_tertiary);
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for s in list {
                        let mark = if fstyle.as_deref() == Some(s.as_str()) { "✓ " } else { "   " };
                        let (clicked, hovered) = menu_item_hover(ui, &format!("{mark}{s}"));
                        if clicked {
                            apply_style(app, "fontStyle", json!(s));
                            ui.close();
                        } else if hovered {
                            // The canvas shows the style (a variable font's named instance too).
                            app.preview_text(ui.ctx(), "text.style", text_style_params(ids, range, json!({"fontStyle": s})));
                        }
                    }
                });
            }
            None => widgets::label(ui, tr("Pick one font family first"), t.text_tertiary),
        });
        let size = uniform(styles, |s| s.font_size);
        ui.spacing_mut().item_spacing.x = 0.0;
        // The whole layer's size can follow a variable; a selected range just takes a value.
        let e = if editing {
            widgets::number_field(ui, Id::new("ty_size"), Lead::None, size, "", half - 20.0, 1.0)
        } else {
            crate::variable_ui::number_field(app, ui, ids, "fontSize", Id::new("ty_size"), Lead::None, size, "", half - 20.0)
        };
        match e {
            Some(FieldEdit::Set(v)) => apply_style(app, "fontSize", json!(v.clamp(1.0, 5000.0))),
            // The text editing session already groups undo; just apply.
            Some(FieldEdit::Scrub(v, _)) if editing => apply_style(app, "fontSize", json!(v.clamp(1.0, 5000.0))),
            Some(FieldEdit::Scrub(v, done)) => {
                live(app, "Font size", "node.set", json!({"ids": ids, "props": {"fontSize": v.clamp(1.0, 5000.0)}}), done)
            }
            None => {}
        }
        let more = icons::button(ui, Icon::Chevron, 20.0, false, tr("Font size"));
        menu_popup(&more).width(90.0).show(|ui| {
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for v in SIZES {
                    let mark = if size == Some(v) { "✓ " } else { "   " };
                    if menu_item(ui, &format!("{mark}{}", trim_num(v)), "") {
                        apply_style(app, "fontSize", json!(v));
                        ui.close();
                    }
                }
            });
        });
    });

    // Line height + letter spacing.
    row(ui, 32.0, |ui| {
        let lh = uniform(styles, |s| s.line_height);
        let lh_label = lh.map(|l| l.label()).unwrap_or_else(|| tr(MIXED).into());
        let field = SpacingField { field: "lineHeight", id: Id::new("ty_lh"), glyph: LeadGlyph::LineHeight, tip: tr("Line height") };
        if let Some(edit) = spacing_field(app, ui, ids, &field, &lh_label, half, editing)
            && let Some(v) = parse_line_height(&edit.text(), lh)
        {
            commit_spacing(app, ids, range, &field, &edit, v);
        }
        let ls = uniform(styles, |s| s.letter_spacing);
        let ls_label = ls.map(|l| l.label()).unwrap_or_else(|| tr(MIXED).into());
        let field = SpacingField { field: "letterSpacing", id: Id::new("ty_ls"), glyph: LeadGlyph::LetterSpacing, tip: tr("Letter spacing") };
        if let Some(edit) = spacing_field(app, ui, ids, &field, &ls_label, half, editing)
            && let Some(v) = parse_letter_spacing(&edit.text(), ls)
        {
            commit_spacing(app, ids, range, &field, &edit, v);
        }
    });
}

/// Paragraph spacing, resizing, alignment and the Type settings popover.
fn paragraph_rows(
    app: &mut FigApp,
    ui: &mut Ui,
    doc: &Document,
    ids: &[NodeId],
    styles: &[TextStyle],
    apply_style: &dyn Fn(&mut FigApp, &str, Value),
) {
    let half = (ui.available_width() - PAD * 2.0 - 8.0) / 2.0;
    // Paragraph spacing + resizing.
    let ps = common(doc, ids, |n| n.kind.text().map(|t| t.paragraph_spacing.to_bits())).flatten().map(f64::from_bits);
    let resize = common(doc, ids, |n| n.kind.text().map(|t| t.auto_resize)).flatten();
    row(ui, 32.0, |ui| {
        let ps_label = ps.map(trim_num).unwrap_or_else(|| tr(MIXED).into());
        let field =
            SpacingField { field: "paragraphSpacing", id: Id::new("ty_ps"), glyph: LeadGlyph::ParagraphSpacing, tip: tr("Paragraph spacing") };
        if let Some(edit) = spacing_input(ui, &field, &ps_label, half)
            && let Some(v) = widgets::eval(&edit.text(), ps)
        {
            commit_spacing(app, ids, None, &field, &edit, json!(v.clamp(0.0, 10_000.0)));
        }
        let items =
            [(Icon::Hug, tr("Auto width")), (Icon::LayoutV, tr("Auto height")), (Icon::Fixed, tr("Fixed size")), (Icon::Dots, tr("Truncate text"))];
        let active = resize.map(|r| match r {
            TextAutoResize::WidthAndHeight => 0,
            TextAutoResize::Height => 1,
            TextAutoResize::None => 2,
            TextAutoResize::Truncate => 3,
        });
        if let Some(i) = widgets::segmented(ui, &items, active, (half / 4.0).floor()) {
            let v = ["widthAndHeight", "height", "none", "truncate"].get(i).copied().unwrap_or("none");
            app.set_props(ids, json!({"textAutoResize": v}));
        }
    });

    // Alignment + Type settings.
    let align = common(doc, ids, |n| n.kind.text().map(|t| (t.align_h, t.align_v))).flatten();
    row(ui, 32.0, |ui| {
        let items = [
            (Icon::TextAlignLeft, tr("Align left")),
            (Icon::TextAlignCenter, tr("Align center")),
            (Icon::TextAlignRight, tr("Align right")),
            (Icon::TextAlignJustify, tr("Justify")),
        ];
        let active = align.map(|(h, _)| match h {
            TextAlignH::Left => 0,
            TextAlignH::Center => 1,
            TextAlignH::Right => 2,
            TextAlignH::Justified => 3,
        });
        if let Some(i) = widgets::segmented(ui, &items, active, 24.0) {
            let v = ["left", "center", "right", "justified"].get(i).copied().unwrap_or("left");
            app.set_props(ids, json!({"textAlign": v}));
        }
        let items = [(Icon::TextTop, tr("Align top")), (Icon::TextMiddle, tr("Align middle")), (Icon::TextBottom, tr("Align bottom"))];
        let active = align.map(|(_, v)| match v {
            TextAlignV::Top => 0,
            TextAlignV::Middle => 1,
            TextAlignV::Bottom => 2,
        });
        if let Some(i) = widgets::segmented(ui, &items, active, 24.0) {
            let v = ["top", "middle", "bottom"].get(i).copied().unwrap_or("top");
            app.set_props(ids, json!({"textAlignVertical": v}));
        }
        let open_id = Id::new("ty_settings_open");
        let open = egui::Popup::is_id_open(ui.ctx(), open_id);
        let more = icons::button(ui, Icon::Dots, 24.0, open, tr("Type settings"));
        egui::Popup::from_toggle_button_response(&more)
            .id(open_id)
            .width(264.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| type_settings(app, ui, doc, ids, styles, apply_style));
    });
}

/// A menu item that also reports the pointer being over it: (clicked, hovered).
fn menu_item_hover(ui: &mut Ui, label: &str) -> (bool, bool) {
    let r = crate::right::menu_row(ui, label, "", 0.0, |_, _, _| {});
    (r.clicked(), r.hovered())
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

/// The variable font the shown styles use (one family, one variable face), if any.
fn variable_face(styles: &[TextStyle]) -> Option<Arc<figcraft_text::Face>> {
    let family = uniform(styles, |s| s.font_family.clone())?;
    let face = figcraft_text::find_face(&family, &styles.first()?.font_style)?;
    (face.is_variable() && face.family.eq_ignore_ascii_case(&family)).then_some(face)
}

/// The Type settings popover: Basics (decoration, case, list, indent, truncation), Details
/// (OpenType features) and, for variable fonts, Variable (axis sliders).
fn type_settings(
    app: &mut FigApp,
    ui: &mut Ui,
    doc: &Document,
    ids: &[NodeId],
    styles: &[TextStyle],
    apply_style: &dyn Fn(&mut FigApp, &str, Value),
) {
    let t = Tokens::get(ui.ctx());
    let tab_id = Id::new("ty_settings_tab");
    let vf = variable_face(styles);
    let tabs = &TABS[..if vf.is_some() { 3 } else { 2 }];
    let mut tab: usize = ui.data(|d| d.get_temp(tab_id)).filter(|i| *i < tabs.len()).unwrap_or(0);
    ui.horizontal(|ui| {
        for (i, l) in tabs.iter().enumerate() {
            let col = if tab == i { t.text } else { t.text_secondary };
            let r = ui.add(egui::Label::new(egui::RichText::new(tr(l)).font(semibold(11.5)).color(col)).sense(Sense::click()));
            if r.clicked() {
                tab = i;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(tab_id, tab));
    ui.add_space(4.0);
    let w = ui.available_width();
    if tab == 0 {
        let deco = uniform(styles, |s| s.decoration);
        caption(ui, tr("Decoration"));
        let active = deco.map(|d| match d {
            TextDecoration::None => 0,
            TextDecoration::Underline => 1,
            TextDecoration::Strikethrough => 2,
        });
        let items = [("—", tr_noop("None")), ("U", tr_noop("Underline")), ("S", tr_noop("Strikethrough"))];
        if let Some(i) = text_seg(ui, "deco", &items, active, w) {
            apply_style(app, "textDecoration", json!((["none", "underline", "strikethrough"].get(i).copied().unwrap_or("none"))));
        }
        if let Some(d @ (TextDecoration::Underline | TextDecoration::Strikethrough)) = deco {
            decoration_details(app, ui, styles, apply_style, d == TextDecoration::Underline, w);
        }
        caption(ui, tr("Letter case"));
        let case = uniform(styles, |s| s.case);
        let active = case.map(|c| match c {
            TextCase::Original => 0,
            TextCase::Upper => 1,
            TextCase::Lower => 2,
            TextCase::Title => 3,
            TextCase::SmallCaps => 4,
        });
        let items = [
            ("—", tr_noop("As typed")),
            ("AG", tr_noop("Uppercase")),
            ("ag", tr_noop("Lowercase")),
            ("Ag", tr_noop("Title case")),
            ("Aɢ", tr_noop("Small caps")),
        ];
        if let Some(i) = text_seg(ui, "case", &items, active, w) {
            apply_style(app, "textCase", json!((["original", "upper", "lower", "title", "smallCaps"].get(i).copied().unwrap_or("original"))));
        }
        caption(ui, tr("List style"));
        // While editing: the selected paragraphs' list.
        let range = figcraft_engine::textedit::selected_range(&app.session);
        let list = common(doc, ids, |n| {
            n.kind.text().and_then(|t| {
                let (a, b) = range.filter(|r| r.0 == n.id).map_or((0, t.char_count()), |(_, a, b)| (a, b));
                t.common_para_list(a, b).map(|p| p.list)
            })
        })
        .flatten();
        let active = list.map(|l| match l {
            ListStyle::None => 0,
            ListStyle::Bulleted => 1,
            ListStyle::Numbered => 2,
        });
        let items = [("—", tr_noop("No list")), ("•", tr_noop("Bulleted list")), ("1.", tr_noop("Numbered list"))];
        if let Some(i) = text_seg(ui, "list", &items, active, w) {
            app.run("text.listStyle", json!({"ids": ids, "style": (["none", "bulleted", "numbered"].get(i).copied().unwrap_or("none"))}));
        }
        ui.add_space(6.0);
        let spacing = common(doc, ids, |n| n.kind.text().map(|t| t.list_spacing.to_bits())).flatten().map(f64::from_bits);
        field_row(ui, tr("List spacing"), |ui, fw| {
            if let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) =
                widgets::number_field(ui, Id::new("ty_list_spacing"), Lead::None, spacing, "", fw, 1.0)
            {
                app.set_props(ids, json!({"listSpacing": v.clamp(0.0, 10_000.0)}));
            }
        });
        ui.add_space(4.0);
        let indent = common(doc, ids, |n| n.kind.text().map(|t| t.paragraph_indent.to_bits())).flatten().map(f64::from_bits);
        // Indent applies to start-aligned text only; elsewhere the field stays visible but off.
        let start_aligned =
            common(doc, ids, |n| n.kind.text().map(|t| t.align_h != TextAlignH::Center && t.align_h != TextAlignH::Justified)).flatten();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(start_aligned != Some(false), |ui| {
                ui.add_sized(vec2(110.0, FIELD_H), egui::Label::new(egui::RichText::new(tr("Paragraph indent")).font(regular(11.0)).color(t.text)));
                let field = widgets::number_field(ui, Id::new("ty_indent"), Lead::None, indent, "", w - 118.0, 1.0);
                if let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) = field {
                    app.set_props(ids, json!({"paragraphIndent": v.clamp(0.0, 10_000.0)}));
                }
            })
            .response
            .on_disabled_hover_text(tr("Paragraph indent applies to left-aligned text"));
        });
        ui.add_space(4.0);
        for (key, label) in [("hangingPunctuation", tr("Hanging punctuation")), ("hangingList", tr("Hanging list"))] {
            let all_on = ids.iter().all(|i| {
                doc.node(*i).and_then(|n| n.kind.text()).is_some_and(|t| if key == "hangingList" { t.hanging_list } else { t.hanging_punctuation })
            });
            if widgets::checkbox(ui, all_on, label) {
                app.set_props(ids, json!({ key: !all_on }));
            }
        }
        caption(ui, tr("Vertical trim"));
        let trim = common(doc, ids, |n| n.kind.text().map(|t| usize::from(t.leading_trim == LeadingTrim::CapHeight))).flatten();
        let items = [(tr("Standard"), tr_noop("Standard")), ("Ag", tr_noop("Cap height to baseline"))];
        if let Some(i) = text_seg(ui, "trim", &items, trim, w) {
            app.set_props(ids, json!({"leadingTrim": if i == 1 { "capHeight" } else { "none" }}));
        }
        let resize = common(doc, ids, |n| n.kind.text().map(|t| t.auto_resize)).flatten();
        caption(ui, tr("Truncate text"));
        let active = resize.map(|r| usize::from(r == TextAutoResize::Truncate));
        match text_seg(ui, "trunc", &[(tr("Off"), tr_noop("Don't truncate")), ("…", tr_noop("Truncate with an ellipsis"))], active, w) {
            Some(1) => app.set_props(ids, json!({"textAutoResize": "truncate"})),
            Some(_) if resize != Some(TextAutoResize::Height) && resize != Some(TextAutoResize::WidthAndHeight) => {
                app.set_props(ids, json!({"textAutoResize": "none"}))
            }
            _ => {}
        }
        // Max lines only matter while truncating; no limit shows "Auto".
        if resize == Some(TextAutoResize::Truncate) {
            let max = common(doc, ids, |n| n.kind.text().map(|t| t.max_lines)).flatten();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_sized(vec2(110.0, FIELD_H), egui::Label::new(egui::RichText::new(tr("Max lines")).font(regular(11.0)).color(t.text)));
                let shown = max.map(f64::from).filter(|m| *m > 0.0);
                let empty = if max.is_some() { tr("Auto") } else { tr(MIXED) };
                if let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) =
                    widgets::number_field_or(ui, Id::new("ty_maxlines"), shown, empty, w - 118.0, 1.0)
                {
                    app.run("text.maxLines", json!({"ids": ids, "lines": v.round().clamp(0.0, 10_000.0)}));
                }
            });
        }
    } else if let (2, Some(face)) = (tab, &vf) {
        variable_axes(app, ui, ids, styles, face);
    } else {
        // Which features the font has (first style shown).
        let face = uniform(styles, |s| (s.font_family.clone(), s.font_style.clone())).and_then(|(f, s)| figcraft_text::find_face(&f, &s));
        let face_name = face.as_ref().map(|f| format!("{} {}", f.family, f.style)).unwrap_or_else(|| tr("Mixed fonts").into());
        widgets::label(ui, &face_name, t.text_secondary);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (tag, label, default_on) in FEATURES {
                let state = common(doc, ids, |n| n.kind.text().map(|t| t.features.get(tag).copied())).flatten();
                let on = state.map(|s| s.unwrap_or(default_on));
                let supported = face.as_ref().is_none_or(|f| f.has_feature(tag.as_bytes().try_into().unwrap_or(b"    ")));
                if feature_row(ui, tag, tr(label), on, supported) {
                    let want = !on.unwrap_or(false);
                    // Back to the font's default clears the override.
                    let v = if want == default_on { Value::Null } else { json!(want) };
                    app.run("text.setFeatures", json!({"ids": ids, "features": {tag: v}}));
                }
            }
        });
    }
}

/// A Type settings row: a label, then `add` with the rest of the width.
fn field_row<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui, f32) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, FIELD_H), egui::Label::new(egui::RichText::new(label).font(regular(11.0)).color(t.text)));
        add(ui, w - 118.0)
    })
    .inner
}

/// How the decoration looks: style, thickness, (underlines) offset and skip ink, and colour.
fn decoration_details(app: &mut FigApp, ui: &mut Ui, styles: &[TextStyle], apply_style: &dyn Fn(&mut FigApp, &str, Value), underline: bool, w: f32) {
    caption(ui, tr("Decoration style"));
    let active = uniform(styles, |s| s.look().style as usize);
    let items = [(tr("Solid"), tr_noop("Solid")), (tr("Dotted"), tr_noop("Dotted")), (tr("Wavy"), tr_noop("Wavy"))];
    if let Some(i) = text_seg(ui, "deco_style", &items, active, w) {
        apply_style(app, "textDecorationStyle", json!((["solid", "dotted", "wavy"].get(i).copied().unwrap_or("solid"))));
    }
    ui.add_space(4.0);
    let mut lengths = vec![("textDecorationThickness", tr("Thickness"), uniform(styles, |s| s.look().thickness))];
    if underline {
        lengths.push(("textDecorationOffset", tr("Offset"), uniform(styles, |s| s.look().offset)));
    }
    for (key, label, value) in lengths {
        field_row(ui, label, |ui, fw| {
            let (shown, empty) = match value {
                Some(DecorationLength::Pixels(v)) => (Some(v), String::new()),
                Some(DecorationLength::Percent(p)) => (None, format!("{}%", trim_num(p))),
                Some(DecorationLength::Auto) => (None, tr("Auto").to_string()),
                None => (None, tr(MIXED).to_string()),
            };
            let auto = value == Some(DecorationLength::Auto);
            let field_w = if auto { fw } else { fw - 36.0 };
            if let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) =
                widgets::number_field_or(ui, Id::new(("ty_deco", key)), shown, &empty, field_w, 0.5)
            {
                // A thickness of 0 or less goes back to the font's.
                let v = if key == "textDecorationThickness" && v <= 0.0 { json!("auto") } else { json!(v.clamp(-1000.0, 1000.0)) };
                apply_style(app, key, v);
            }
            let tip = tr("Back to the font's value");
            if !auto
                && ui
                    .add(
                        egui::Label::new(egui::RichText::new(tr("Auto")).font(regular(11.0)).color(Tokens::get(ui.ctx()).accent))
                            .sense(Sense::click()),
                    )
                    .on_hover_text(tip)
                    .clicked()
            {
                apply_style(app, key, json!("auto"));
            }
        });
    }
    if underline {
        let skip = uniform(styles, |s| s.look().skip_ink);
        if widgets::checkbox(ui, skip == Some(true), tr("Skip ink")) {
            apply_style(app, "textDecorationSkipInk", json!(skip != Some(true)));
        }
    }
    let color = uniform(styles, |s| s.look().color.map(|c| format!("#{}", c.to_hex())));
    field_row(ui, tr("Colour"), |ui, fw| {
        let (value, hint) = match &color {
            Some(c) => (c.clone().unwrap_or_default(), tr("Auto")),
            None => (String::new(), tr(MIXED)),
        };
        if let Some(s) = widgets::text_field(ui, Id::new("ty_deco_color"), &value, fw, hint) {
            let s = s.trim();
            if s.is_empty() || s.eq_ignore_ascii_case("auto") {
                apply_style(app, "textDecorationColor", json!("auto"));
            } else if let Some(c) = figcraft_color::Color::parse_css(s).or_else(|| figcraft_color::Color::from_hex(s)) {
                apply_style(app, "textDecorationColor", json!(format!("#{}", c.to_hex())));
            } else {
                app.toast(trf("{value} isn't a colour", &[("value", &s)]));
            }
        }
    });
}

/// Type settings ▸ Variable: the font's named instances, then a slider and a field per visible
/// axis (values the style name implies show until changed; "Mixed" when the shown styles differ).
/// The canvas previews a hovered instance and a dragged slider or field live; letting go commits
/// one undo step, Escape cancels.
fn variable_axes(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], styles: &[TextStyle], face: &figcraft_text::Face) {
    let t = Tokens::get(ui.ctx());
    let range = figcraft_engine::textedit::selected_range(&app.session);
    widgets::label(ui, &face.family, t.text_secondary);
    named_instances(app, ui, ids, range, styles, face);
    // Each style's axis values; a style drawn by another face (e.g. a separate italic file) is mixed.
    let per_style: Vec<Option<Vec<f64>>> = styles
        .iter()
        .map(|s| figcraft_text::find_face(&s.font_family, &s.font_style).filter(|f| f.id == face.id).map(|_| face.axis_values(s)))
        .collect();
    let w = ui.available_width();
    let field_w = 52.0;
    let label_w = 84.0;
    let slider_w = (w - label_w - field_w - 12.0).max(40.0);
    let cancel_id = Id::new("ty_axis_cancelled");
    // A cancelled gesture ends when the pointer is let go.
    if !ui.input(|i| i.pointer.any_down()) {
        ui.data_mut(|d| d.remove::<bool>(cancel_id));
    }
    for (i, axis) in face.axes.iter().enumerate().filter(|(_, a)| !a.hidden) {
        let first = per_style.first().cloned().flatten().and_then(|v| v.get(i).copied());
        let value = first.filter(|v0| per_style.iter().all(|vs| vs.as_ref().and_then(|v| v.get(i)).is_some_and(|v| (v - v0).abs() < 1e-9)));
        // While a drag previews, the controls show the dragged value (the panel shows the document).
        let live_id = Id::new(("ty_axis_live", &axis.tag));
        if app.ui.text_preview.is_none() {
            ui.data_mut(|d| d.remove::<f64>(live_id));
        }
        let shown = ui.data(|d| d.get_temp::<f64>(live_id)).or(value).map(|v| round_axis(axis, v));
        // (value, gesture ended)
        let mut set: Option<(Value, bool)> = None;
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (lr, _) = ui.allocate_exact_size(vec2(label_w, FIELD_H), Sense::hover());
            let name = ui.painter().layout(axis.name.clone(), regular(11.0), t.text, label_w);
            let tip = trf(
                "{name} ({tag}): {min} – {max}, default {default}. Double-click the slider to go back to the style's value; Escape while dragging cancels.",
                &[
                    ("name", &axis.name),
                    ("tag", &axis.tag),
                    ("min", &trim_num(axis.min)),
                    ("max", &trim_num(axis.max)),
                    ("default", &trim_num(axis.default)),
                ],
            );
            ui.interact(lr, Id::new(("ty_axis_label", &axis.tag)), Sense::hover()).on_hover_text(tip);
            ui.painter().galley(pos2(lr.min.x, lr.center().y - name.size().y / 2.0), name, t.text);
            match axis_slider(ui, Id::new(("ty_axis_slider", &axis.tag)), axis, shown, slider_w) {
                Some((Some(v), done)) => set = Some((json!(round_axis(axis, v)), done)),
                Some((None, _)) => set = Some((Value::Null, true)),
                None => {}
            }
            let step = if axis.max - axis.min > 20.0 { 1.0 } else { 0.1 };
            match widgets::number_field(ui, Id::new(("ty_axis_field", &axis.tag)), Lead::None, shown, "", field_w, step) {
                Some(FieldEdit::Set(v)) => set = Some((json!(axis.clamp(round_axis(axis, v))), true)),
                Some(FieldEdit::Scrub(v, done)) => set = Some((json!(axis.clamp(round_axis(axis, v))), done)),
                None => {}
            }
        });
        let Some((v, done)) = set else { continue };
        // Escape cancels a previewing gesture (egui may end the drag on it too): the preview ends
        // and the rest of the gesture is ignored until the pointer is let go.
        let previewing = ui.data(|d| d.get_temp::<f64>(live_id)).is_some();
        if previewing && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            ui.data_mut(|d| {
                d.insert_temp(cancel_id, true);
                d.remove::<f64>(live_id);
            });
            app.end_text_preview(ui.ctx());
            continue;
        }
        if ui.data(|d| d.get_temp::<bool>(cancel_id)).unwrap_or(false) {
            continue;
        }
        let params = json!({"ids": ids, "variations": {axis.tag.clone(): v}});
        if done {
            ui.data_mut(|d| d.remove::<f64>(live_id));
            app.run("text.setVariations", params);
        } else {
            if let Some(x) = v.as_f64() {
                ui.data_mut(|d| d.insert_temp(live_id, x));
            }
            app.preview_text(ui.ctx(), "text.setVariations", params);
        }
    }
    if styles.iter().any(|s| !s.variations.is_empty()) {
        ui.add_space(8.0);
        let r = ui.add(egui::Label::new(egui::RichText::new(tr("Reset to style")).font(regular(11.0)).color(t.accent)).sense(Sense::click()));
        if r.on_hover_text(tr("Back to the values the font style implies")).clicked() {
            app.run("text.setVariations", json!({"ids": ids, "variations": {}, "replace": true}));
        }
    }
}

/// The variable font's named instances as a menu: the one the text shows ("Custom" once an axis
/// moved), hovering one previews it on the canvas, clicking applies it (its custom axis values go).
fn named_instances(
    app: &mut FigApp,
    ui: &mut Ui,
    ids: &[NodeId],
    range: Option<(NodeId, usize, usize)>,
    styles: &[TextStyle],
    face: &figcraft_text::Face,
) {
    if face.instances.is_empty() {
        return;
    }
    let current = uniform(styles, |s| (s.variations.is_empty(), s.font_style.clone()));
    let label = match &current {
        Some((true, st)) if face.instance(st).is_some() => st.clone(),
        Some(_) => tr("Custom").to_string(),
        None => tr(MIXED).to_string(),
    };
    ui.add_space(4.0);
    // An inline list (a menu would close the Type settings popover it opens from).
    let open_id = Id::new("ty_instances_open");
    let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    if widgets::dropdown(ui, &label, ui.available_width(), None).on_hover_text(tr("Named instance")).clicked() {
        open = !open;
    }
    if open {
        ui.add_space(2.0);
        egui::ScrollArea::vertical().id_salt("ty_instances").max_height(220.0).show(ui, |ui| {
            for inst in &face.instances {
                let mark = if label == inst.name { "✓ " } else { "   " };
                let (clicked, hovered) = menu_item_hover(ui, &format!("{mark}{}", inst.name));
                let props = json!({"fontStyle": inst.name, "fontVariations": null});
                if clicked {
                    commit_text(app, ids, range, props);
                    open = false;
                } else if hovered {
                    app.preview_text(ui.ctx(), "text.style", text_style_params(ids, range, props));
                }
            }
        });
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
    ui.add_space(2.0);
}

/// Axis values snap to whole numbers on wide axes (weight, width…) and tenths on narrow ones.
fn round_axis(axis: &figcraft_text::Axis, v: f64) -> f64 {
    if axis.max - axis.min > 20.0 { v.round() } else { (v * 10.0).round() / 10.0 }
}

/// A horizontal slider over an axis's range with a tick at its default. `value` None = mixed.
/// Returns the new value (None = double-click: back to the style's value) and whether the
/// gesture ended.
fn axis_slider(ui: &mut Ui, id: Id, axis: &figcraft_text::Axis, value: Option<f64>, width: f32) -> Option<(Option<f64>, bool)> {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), vec2(width - 12.0, 4.0));
    let span = (axis.max - axis.min).max(1e-9);
    let x_of = |v: f64| track.min.x + ((v - axis.min) / span).clamp(0.0, 1.0) as f32 * track.width();
    let p = ui.painter();
    p.rect_filled(track, CornerRadius::same(2), t.border);
    let dx = x_of(axis.default);
    p.line_segment([pos2(dx, track.min.y - 3.0), pos2(dx, track.max.y + 3.0)], Stroke::new(1.0, t.border_strong));
    if let Some(v) = value {
        let x = x_of(v);
        p.rect_filled(Rect::from_min_max(track.min, pos2(x, track.max.y)), CornerRadius::same(2), t.accent);
        p.circle_filled(pos2(x, track.center().y), 6.0, Color32::WHITE);
        let ring = if resp.hovered() || resp.dragged() { t.accent } else { t.border_strong };
        p.circle_stroke(pos2(x, track.center().y), 6.0, Stroke::new(1.0, ring));
    }
    if resp.double_clicked() {
        return Some((None, true));
    }
    if (resp.dragged() || resp.clicked() || resp.drag_started())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let f = f64::from(((pos.x - track.min.x) / track.width().max(1.0)).clamp(0.0, 1.0));
        return Some((Some(axis.min + f * span), resp.drag_stopped() || resp.clicked()));
    }
    if resp.drag_stopped() {
        return value.map(|v| (Some(v), true));
    }
    None
}

fn caption(ui: &mut Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.label(egui::RichText::new(s).font(regular(10.5)).color(t.text_secondary));
}

/// A segmented control with short text labels (decoration, case, list…); `tip`s are English keys
/// (matched below), translated where shown. Returns the clicked index.
fn text_seg(ui: &mut Ui, salt: &str, items: &[(&str, &str)], active: Option<usize>, width: f32) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    let cell = width / items.len().max(1) as f32;
    let mut out = None;
    for (i, (label, tip)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.min.x + i as f32 * cell, rect.min.y), vec2(cell, FIELD_H));
        let resp = ui.interact(r, Id::new(("ty_seg", salt, i)), Sense::click()).on_hover_text(tr(tip));
        crate::a11y::toggle(&resp, tr(tip), active == Some(i));
        if active == Some(i) {
            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(4), t.panel);
            ui.painter().rect_stroke(r.shrink(2.0), CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(4), t.hover);
        }
        let col = if active == Some(i) { t.text } else { t.text_secondary };
        let galley = if *tip == "Small caps" {
            small_caps_sample(ui, label, col)
        } else {
            ui.painter().layout_no_wrap((*label).to_string(), regular(11.5), col)
        };
        let gr = Rect::from_center_size(r.center(), galley.size());
        let size = galley.size();
        ui.painter().galley(gr.min, galley, col);
        // Draw the decoration on its own sample letter.
        match *tip {
            "Underline" => {
                let y = gr.max.y - 1.5;
                ui.painter().line_segment([pos2(gr.min.x, y), pos2(gr.min.x + size.x, y)], Stroke::new(1.0, col));
            }
            "Strikethrough" => {
                let y = gr.center().y + 0.5;
                ui.painter().line_segment([pos2(gr.min.x - 1.0, y), pos2(gr.min.x + size.x + 1.0, y)], Stroke::new(1.0, col));
            }
            _ => {}
        }
        if resp.clicked() {
            out = Some(i);
        }
    }
    out
}

/// The Small caps sample: a capital, then its other letters as smaller capitals. Drawn from
/// ordinary capitals because the small-capital letters (`ɢ`, U+0262) are missing from many UI
/// fonts, egui's defaults among them, where they showed as a box.
fn small_caps_sample(ui: &Ui, label: &str, col: Color32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    let mut chars = label.chars();
    let first: String = chars.next().into_iter().collect();
    let rest: String = chars.map(small_capital_to_capital).collect::<String>().to_uppercase();
    job.append(&first, 0.0, egui::TextFormat { font_id: regular(11.5), color: col, valign: egui::Align::BOTTOM, ..Default::default() });
    job.append(&rest, 0.0, egui::TextFormat { font_id: regular(8.5), color: col, valign: egui::Align::BOTTOM, ..Default::default() });
    ui.painter().layout_job(job)
}

/// The capital a small-capital letter stands for (the ones a sample uses), else the letter.
fn small_capital_to_capital(c: char) -> char {
    match c {
        'ɢ' => 'G',
        'ʜ' => 'H',
        'ɪ' => 'I',
        'ʟ' => 'L',
        'ɴ' => 'N',
        'ʀ' => 'R',
        'ʏ' => 'Y',
        c => c,
    }
}

/// One OpenType feature toggle; `on` None = mixed. Returns true when clicked.
fn feature_row(ui: &mut Ui, tag: &str, label: &str, on: Option<bool>, supported: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    let resp = if supported { resp } else { resp.on_hover_text(tr("Not in this font")) };
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    let b = Rect::from_min_size(pos2(rect.min.x + 4.0, rect.center().y - 7.0), vec2(14.0, 14.0));
    match on {
        Some(true) => {
            ui.painter().rect_filled(b, CornerRadius::same(3), t.accent);
            icons::paint(ui.painter(), b.shrink(1.5), Icon::Check, Color32::WHITE);
        }
        Some(false) => {
            ui.painter().rect_stroke(b, CornerRadius::same(3), Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
        }
        None => {
            ui.painter().rect_filled(b, CornerRadius::same(3), t.accent);
            ui.painter().line_segment([pos2(b.min.x + 3.5, b.center().y), pos2(b.max.x - 3.5, b.center().y)], Stroke::new(1.5, Color32::WHITE));
        }
    }
    let col = if supported { t.text } else { t.text_tertiary };
    ui.painter().text(pos2(b.max.x + 8.0, rect.center().y), Align2::LEFT_CENTER, label, regular(11.0), col);
    ui.painter().text(pos2(rect.max.x - 6.0, rect.center().y), Align2::RIGHT_CENTER, tag, regular(10.0), t.text_tertiary);
    resp.clicked()
}

#[derive(Clone, Copy)]
enum LeadGlyph {
    LineHeight,
    LetterSpacing,
    ParagraphSpacing,
}

/// Paint a small original glyph for a typography field's lead.
fn paint_lead(p: &egui::Painter, r: Rect, g: LeadGlyph, col: Color32) {
    let c = r.center();
    let s = Stroke::new(1.0, col);
    match g {
        LeadGlyph::LineHeight => {
            p.line_segment([pos2(c.x - 5.0, c.y - 6.0), pos2(c.x + 5.0, c.y - 6.0)], s);
            p.line_segment([pos2(c.x - 5.0, c.y + 6.0), pos2(c.x + 5.0, c.y + 6.0)], s);
            p.text(c, Align2::CENTER_CENTER, "A", regular(9.5), col);
        }
        LeadGlyph::LetterSpacing => {
            p.line_segment([pos2(c.x - 6.0, c.y - 5.0), pos2(c.x - 6.0, c.y + 5.0)], s);
            p.line_segment([pos2(c.x + 6.0, c.y - 5.0), pos2(c.x + 6.0, c.y + 5.0)], s);
            p.text(c, Align2::CENTER_CENTER, "A", regular(9.5), col);
        }
        LeadGlyph::ParagraphSpacing => {
            for dy in [-5.0, -2.5, 2.5, 5.0] {
                p.line_segment([pos2(c.x - 5.0, c.y + dy), pos2(c.x + 5.0, c.y + dy)], s);
            }
        }
    }
}

/// Line height, letter spacing or paragraph spacing: property, field id, glyph and tooltip.
struct SpacingField {
    field: &'static str,
    id: Id,
    glyph: LeadGlyph,
    tip: &'static str,
}

enum SpacingEdit {
    Typed(String),
    Scrub(f64, bool),
}

impl SpacingEdit {
    fn text(&self) -> String {
        match self {
            Self::Typed(text) => text.clone(),
            Self::Scrub(value, _) => trim_num(*value),
        }
    }
}

/// Keep typed unit parsing and character-range targeting; only a scrub outside a text-editing
/// session needs its own interaction. The editing session already owns its undo step.
fn commit_spacing(app: &mut FigApp, ids: &[NodeId], range: Option<(NodeId, usize, usize)>, field: &SpacingField, edit: &SpacingEdit, value: Value) {
    let props = json!({field.field: value});
    match edit {
        SpacingEdit::Scrub(_, done) if figcraft_engine::textedit::selected_range(&app.session).is_none() => {
            live(app, field.tip, "node.set", json!({"ids": ids, "props": props}), *done);
        }
        _ => commit_text(app, ids, range, props),
    }
}

/// The original text editor plus the same leading-icon scrub gesture as ordinary number fields.
/// Auto and Mixed have no numeric starting value and remain unchanged by dragging.
fn spacing_input(ui: &mut Ui, field: &SpacingField, value: &str, width: f32) -> Option<SpacingEdit> {
    let rect = Rect::from_min_size(ui.cursor().min, vec2(24.0, FIELD_H));
    let typed = lead_field(ui, field.id, field.glyph, value, width, field.tip);
    if ui.data(|d| d.get_temp::<String>(field.id.with("editing")).is_some()) {
        return typed.map(SpacingEdit::Typed);
    }
    let response = ui.interact(rect, field.id.with("lead"), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let numeric = value.trim_end_matches('%').parse::<f64>().ok();
    match widgets::scrub(ui, field.id, &response, numeric, 1.0) {
        Some(FieldEdit::Scrub(v, done)) => Some(SpacingEdit::Scrub(v, done)),
        _ => typed.map(SpacingEdit::Typed),
    }
}

/// A spacing field that whole layers can bind to a number variable (in pixels): the variable's
/// chip when bound, else the editable field with a variable button on hover. While a text range is
/// selected it only takes values. Returns a typed commit or a live scrub.
fn spacing_field(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], f: &SpacingField, value: &str, width: f32, range: bool) -> Option<SpacingEdit> {
    let rect = Rect::from_min_size(ui.cursor().min, vec2(width, FIELD_H));
    if !range && crate::variable_ui::bound_chip(app, ui, ids, f.field, VariableType::Number, rect) {
        ui.allocate_rect(rect, Sense::hover());
        return None;
    }
    let out = spacing_input(ui, f, value, width);
    if !range && ui.data(|d| d.get_temp::<String>(f.id.with("editing")).is_none()) {
        crate::variable_ui::bind_button(ui, rect, ids, f.field, VariableType::Number);
    }
    out
}

fn lead_field(ui: &mut Ui, id: Id, glyph: LeadGlyph, value: &str, width: f32, tip: &str) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    let lead = Rect::from_min_size(rect.min, vec2(24.0, FIELD_H));
    let text_rect = Rect::from_min_max(pos2(lead.max.x, rect.min.y), rect.max);
    let editing_id = id.with("editing");
    let mut editing: Option<String> = ui.data(|d| d.get_temp(editing_id));
    let resp = ui.interact(rect, id.with("click"), Sense::click()).on_hover_text(tip);
    crate::a11y::text(&resp, tip, value);
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    if editing.is_some() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.accent), StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
    }
    paint_lead(ui.painter(), lead, glyph, t.icon_secondary);
    let started = editing.is_none() && resp.clicked();
    if started {
        editing = Some(if value == tr(MIXED) { String::new() } else { value.to_string() });
        ui.memory_mut(|m| m.request_focus(id.with("te")));
    }
    let mut out = None;
    match editing {
        Some(mut text) => {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect.shrink2(vec2(2.0, 4.0))));
            let r = child.add(
                egui::TextEdit::singleline(&mut text)
                    .id(id.with("te"))
                    .frame(egui::Frame::NONE)
                    .font(regular(11.0))
                    .desired_width(text_rect.width() - 6.0),
            );
            crate::a11y::own_focus(&r);
            crate::widgets::select_all_on_focus(ui, &r, &text);
            // The Enter that started the edit (from the keyboard) doesn't end it.
            let enter = !started && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if esc {
                ui.data_mut(|d| d.remove::<String>(editing_id));
            } else if r.lost_focus() || enter {
                out = Some(text);
                ui.data_mut(|d| d.remove::<String>(editing_id));
            } else {
                ui.data_mut(|d| d.insert_temp(editing_id, text));
            }
        }
        None => {
            let col = if value == tr(MIXED) { t.text_tertiary } else { t.text };
            ui.painter().text(pos2(text_rect.min.x + 2.0, rect.center().y), Align2::LEFT_CENTER, value, regular(11.0), col);
        }
    }
    out
}

/// "Auto", "24", "150%" → the `lineHeight` value. A bare number keeps the current unit when it is
/// a percentage, else means pixels. Empty text means Auto, except on a Mixed field (`cur` is
/// `None`), where nothing typed is an untouched field and commits nothing.
pub(crate) fn parse_line_height(s: &str, cur: Option<LineHeight>) -> Option<Value> {
    let s = s.trim();
    if s.is_empty() && cur.is_none() {
        return None;
    }
    if s.is_empty() || s.eq_ignore_ascii_case("auto") {
        return Some(json!("auto"));
    }
    if let Some(p) = s.strip_suffix('%') {
        let cur_pct = match cur {
            Some(LineHeight::Percent(p)) => Some(p),
            _ => None,
        };
        return widgets::eval(p.trim(), cur_pct).filter(|v| v.is_finite()).map(|v| json!(format!("{}%", trim_num(v.max(0.0)))));
    }
    if let Some(px) = s.strip_suffix("px") {
        return widgets::eval(px.trim(), None).filter(|v| v.is_finite()).map(|v| json!(v.max(0.0)));
    }
    match cur {
        Some(LineHeight::Percent(p)) => widgets::eval(s, Some(p)).filter(|v| v.is_finite()).map(|v| json!(format!("{}%", trim_num(v.max(0.0))))),
        Some(LineHeight::Pixels(px)) => widgets::eval(s, Some(px)).filter(|v| v.is_finite()).map(|v| json!(v.max(0.0))),
        _ => widgets::eval(s, None).filter(|v| v.is_finite()).map(|v| json!(v.max(0.0))),
    }
}

/// "2%", "1.5px", "-0.5" → the `letterSpacing` value. A bare number keeps the current unit
/// (percent by default).
pub(crate) fn parse_letter_spacing(s: &str, cur: Option<LetterSpacing>) -> Option<Value> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') {
        return widgets::eval(p.trim(), None).filter(|v| v.is_finite()).map(|v| json!(format!("{}%", trim_num(v))));
    }
    if let Some(p) = s.strip_suffix("px") {
        return widgets::eval(p.trim(), None).filter(|v| v.is_finite()).map(|v| json!(v));
    }
    match cur {
        Some(LetterSpacing::Pixels(px)) => widgets::eval(s, Some(px)).filter(|v| v.is_finite()).map(|v| json!(v)),
        Some(LetterSpacing::Percent(p)) => widgets::eval(s, Some(p)).filter(|v| v.is_finite()).map(|v| json!(format!("{}%", trim_num(v)))),
        None => widgets::eval(s, None).filter(|v| v.is_finite()).map(|v| json!(format!("{}%", trim_num(v)))),
    }
}

/// A [`parse_line_height`] value as a [`LineHeight`]: "auto", "150%" or a number of pixels.
pub(crate) fn line_height_of(v: &Value) -> Option<LineHeight> {
    match v {
        Value::String(s) if s.eq_ignore_ascii_case("auto") => Some(LineHeight::Auto),
        Value::String(s) => s.strip_suffix('%')?.trim().parse().ok().filter(|p: &f64| p.is_finite()).map(LineHeight::Percent),
        other => other.as_f64().filter(|v| v.is_finite()).map(LineHeight::Pixels),
    }
}

/// A [`parse_letter_spacing`] value as a [`LetterSpacing`]: "2%" or a number of pixels.
pub(crate) fn letter_spacing_of(v: &Value) -> Option<LetterSpacing> {
    match v {
        Value::String(s) => s.strip_suffix('%')?.trim().parse().ok().filter(|p: &f64| p.is_finite()).map(LetterSpacing::Percent),
        other => other.as_f64().filter(|v| v.is_finite()).map(LetterSpacing::Pixels),
    }
}

// ---- Font picker ----

const RECENT_KEY: &str = "figcraft_recent_fonts";
const ROW: f32 = 26.0;
/// The font list's tallest (it scrolls past that).
const LIST_HEIGHT: f32 = 380.0;

fn recent_families(ctx: &egui::Context) -> Vec<String> {
    ctx.data_mut(|d| d.get_persisted::<Vec<String>>(Id::new(RECENT_KEY))).unwrap_or_default()
}

fn remember_family(ctx: &egui::Context, family: &str) {
    ctx.data_mut(|d| {
        let v = d.get_persisted_mut_or_default::<Vec<String>>(Id::new(RECENT_KEY));
        v.retain(|f| !f.eq_ignore_ascii_case(family));
        v.insert(0, family.to_string());
        v.truncate(8);
    });
}

enum PickRow {
    Header(&'static str),
    Family(String),
}

/// What the family list did this frame.
#[derive(Default)]
pub(crate) struct FontPick {
    /// The family clicked, or the one Enter picked.
    pub picked: Option<String>,
    /// The family Up and Down are on (Enter picks it).
    pub highlighted: Option<String>,
}

/// The searchable family list: click a family, or step through it with Up and Down and press Enter.
pub(crate) fn font_picker(ui: &mut Ui, current: Option<&str>, just_opened: bool) -> FontPick {
    let t = Tokens::get(ui.ctx());
    let q_id = Id::new("font_query");
    // The row Up and Down are on, and the search it is a row of.
    let hi_id = Id::new("font_highlight");
    let mut q: String = ui.data(|d| d.get_temp(q_id)).unwrap_or_default();
    if just_opened {
        q.clear();
        ui.data_mut(|d| d.remove::<(String, usize)>(hi_id));
    }
    // Taken before the search field sees them (it would move its cursor).
    let step = ui.input_mut(|i| {
        i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) as i64
            - i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) as i64
    });
    let search = ui.add(egui::TextEdit::singleline(&mut q).hint_text(tr("Search fonts")).desired_width(ui.available_width()).font(regular(11.5)));
    crate::widgets::select_all_on_focus(ui, &search, &q);
    if just_opened {
        search.request_focus();
    }
    ui.data_mut(|d| d.insert_temp(q_id, q.clone()));
    let ql = q.trim().to_lowercase();
    let all = figcraft_text::family_names();
    let matches: Vec<String> = all.iter().filter(|f| ql.is_empty() || f.to_lowercase().contains(&ql)).cloned().collect();
    let mut rows: Vec<PickRow> = Vec::new();
    if ql.is_empty() {
        let recent: Vec<String> = recent_families(ui.ctx()).into_iter().filter(|f| all.iter().any(|a| a.eq_ignore_ascii_case(f))).collect();
        if !recent.is_empty() {
            rows.push(PickRow::Header(tr("Recent")));
            rows.extend(recent.into_iter().map(PickRow::Family));
            rows.push(PickRow::Header(tr("All fonts")));
        }
    }
    if matches.is_empty() {
        widgets::label(ui, tr("No fonts match"), t.text_tertiary);
        return FontPick::default();
    }
    rows.extend(matches.into_iter().map(PickRow::Family));
    let family = |i: usize| match rows.get(i) {
        Some(PickRow::Family(f)) => Some(f.clone()),
        _ => None,
    };
    // A new search starts again; the first step starts from the current family.
    let mut hi = ui.data(|d| d.get_temp::<(String, usize)>(hi_id)).filter(|(hq, _)| *hq == q).map(|(_, i)| i);
    let stride = ROW + ui.spacing().item_spacing.y;
    let mut offset = None;
    if step != 0 {
        let next = |i: usize| if step > 0 { i.checked_add(1) } else { i.checked_sub(1) };
        let mut from =
            hi.or_else(|| rows.iter().rposition(|r| matches!(r, PickRow::Family(f) if current.is_some_and(|c| c.eq_ignore_ascii_case(f)))));
        for _ in 0..step.unsigned_abs() {
            let mut at = match from {
                Some(i) => next(i),
                None if step > 0 => Some(0),
                None => rows.len().checked_sub(1),
            };
            // Past the headers; at either end it stays where it was.
            while let Some(i) = at.filter(|i| *i < rows.len() && family(*i).is_none()) {
                at = next(i);
            }
            if let Some(i) = at.filter(|i| family(*i).is_some()) {
                from = Some(i);
                hi = Some(i);
            }
        }
        if let Some(i) = hi {
            // Scrolled just enough to show it (the list is shorter than its tallest where the
            // window has less room).
            let (seen, height): (f32, f32) = ui.data(|d| d.get_temp(hi_id.with("view"))).unwrap_or((0.0, LIST_HEIGHT));
            let top = i as f32 * stride;
            offset = Some(seen.max(top + stride - height).min(top));
        }
    }
    match hi {
        Some(i) => ui.data_mut(|d| {
            d.insert_temp(hi_id, (q.clone(), i));
        }),
        None => ui.data_mut(|d| d.remove::<(String, usize)>(hi_id)),
    }
    if search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        let first = rows.iter().find_map(|r| match r {
            PickRow::Family(f) => Some(f.clone()),
            PickRow::Header(_) => None,
        });
        return FontPick { picked: hi.and_then(family).or(first), highlighted: None };
    }
    let mut budget = 4;
    let mut picked = None;
    let mut area = egui::ScrollArea::vertical().max_height(LIST_HEIGHT).auto_shrink([false, true]);
    if let Some(y) = offset {
        area = area.vertical_scroll_offset(y);
    }
    let shown = area.show_rows(ui, ROW, rows.len(), |ui, range| {
        for (i, r) in rows.iter().enumerate().skip(range.start).take(range.len()) {
            let w = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW), if matches!(r, PickRow::Family(_)) { Sense::click() } else { Sense::hover() });
            match r {
                PickRow::Header(h) => {
                    ui.painter().text(pos2(rect.min.x + 8.0, rect.center().y), Align2::LEFT_CENTER, *h, semibold(10.5), t.text_secondary);
                }
                PickRow::Family(f) => {
                    let hovered = resp.hovered() || hi == Some(i);
                    if hovered {
                        ui.painter().rect_filled(rect, CornerRadius::same(5), t.accent);
                    }
                    let col = if hovered { Color32::WHITE } else { t.text };
                    if current.is_some_and(|c| c.eq_ignore_ascii_case(f)) {
                        icons::paint(
                            ui.painter(),
                            Rect::from_center_size(pos2(rect.min.x + 10.0, rect.center().y), vec2(12.0, 12.0)),
                            Icon::Check,
                            col,
                        );
                    }
                    let text_min = pos2(rect.min.x + 22.0, rect.min.y);
                    match crate::font_preview::family(ui.ctx(), f, 13.0, &mut budget) {
                        Some(tex) => {
                            crate::font_preview::paint(
                                ui.painter(),
                                &tex,
                                pos2(text_min.x, rect.center().y),
                                rect.max.x - text_min.x - 6.0,
                                ROW,
                                col,
                            );
                        }
                        None => {
                            ui.painter().text(pos2(text_min.x, rect.center().y), Align2::LEFT_CENTER, f, regular(12.0), col);
                        }
                    }
                    if resp.clicked() {
                        picked = Some(f.clone());
                    }
                }
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(hi_id.with("view"), (shown.state.offset.y, shown.inner_rect.height())));
    FontPick { highlighted: hi.and_then(family), picked }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_caps_sample_draws_only_letters_the_ui_font_has() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, false, 0);
        let mut shown = None;
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            shown = Some(small_caps_sample(ui, "Aɢ", Color32::WHITE));
        });
        out.textures_delta.clear();
        let g = shown.expect("drawn");
        assert_eq!(g.job.text, "AG");
        let [a, rest] = g.job.sections.as_slice() else { panic!("two sections: {:?}", g.job.sections) };
        assert!(rest.format.font_id.size < a.format.font_id.size, "the other letters are smaller capitals");
        ctx.fonts_mut(|f| {
            for sec in &g.job.sections {
                assert!(f.has_glyphs(&sec.format.font_id, &g.job.text), "UI font draws {:?}", g.job.text);
            }
        });
        assert_eq!(small_capital_to_capital('ʀ'), 'R');
        assert_eq!(small_capital_to_capital('x'), 'x');
    }

    #[test]
    fn parses_line_height_and_letter_spacing() {
        assert_eq!(parse_line_height("Auto", None), Some(json!("auto")));
        assert_eq!(parse_line_height("", Some(LineHeight::Pixels(20.0))), Some(json!("auto")));
        assert_eq!(parse_line_height("150%", Some(LineHeight::Auto)), Some(json!("150%")));
        assert_eq!(parse_line_height("24", Some(LineHeight::Auto)), Some(json!(24.0)));
        assert_eq!(parse_line_height("24px", Some(LineHeight::Percent(120.0))), Some(json!(24.0)));
        assert_eq!(parse_line_height("130", Some(LineHeight::Percent(120.0))), Some(json!("130%")));
        assert_eq!(parse_line_height("-5", Some(LineHeight::Pixels(10.0))), Some(json!(0.0)));
        assert_eq!(parse_line_height("abc", None), None);
        // Enter on an untouched Mixed field commits nothing; typing Auto still does.
        assert_eq!(parse_line_height("", None), None);
        assert_eq!(parse_line_height("  ", None), None);
        assert_eq!(parse_letter_spacing("2%", None), Some(json!("2%")));
        assert_eq!(parse_letter_spacing("1.5px", None), Some(json!(1.5)));
        assert_eq!(parse_letter_spacing("-1", None), Some(json!("-1%")));
        assert_eq!(parse_letter_spacing("3", Some(LetterSpacing::Pixels(1.0))), Some(json!(3.0)));
        assert_eq!(parse_letter_spacing("x", Some(LetterSpacing::Percent(0.0))), None);
    }

    /// One frame of the font picker with `events`.
    fn pick(ctx: &egui::Context, current: &str, opened: bool, events: Vec<egui::Event>) -> FontPick {
        let mut out = FontPick::default();
        let mut shown = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| out = font_picker(ui, Some(current), opened));
        shown.textures_delta.clear();
        out
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn arrow_keys_step_through_the_families_and_enter_picks() {
        // #345
        figcraft_text::scan_system_fonts();
        let all = figcraft_text::family_names();
        if all.len() < 3 {
            eprintln!("fewer than three font families; skipping");
            return;
        }
        let ctx = egui::Context::default();
        let current = all[1].as_str();
        assert!(pick(&ctx, current, true, vec![]).highlighted.is_none());
        // The first step starts from the current family; each press is one step.
        let down = pick(&ctx, current, false, vec![key(egui::Key::ArrowDown)]);
        assert_eq!(down.highlighted.as_deref(), Some(all[2].as_str()));
        let up = pick(&ctx, current, false, vec![key(egui::Key::ArrowUp), key(egui::Key::ArrowUp)]);
        assert_eq!(up.highlighted.as_deref(), Some(all[0].as_str()));
        // At the top it stays; Enter picks the family it is on, not the first.
        assert_eq!(pick(&ctx, current, false, vec![key(egui::Key::ArrowUp)]).highlighted.as_deref(), Some(all[0].as_str()));
        pick(&ctx, current, false, vec![key(egui::Key::ArrowDown)]);
        let enter = pick(&ctx, current, false, vec![key(egui::Key::Enter)]);
        assert_eq!(enter.picked.as_deref(), Some(all[1].as_str()));
    }

    #[test]
    fn axis_values_round_by_axis_width() {
        let axis = |min: f64, max: f64| figcraft_text::Axis { tag: "test".into(), name: "Test".into(), min, default: min, max, hidden: false };
        assert_eq!(round_axis(&axis(100.0, 900.0), 456.6), 457.0);
        assert_eq!(round_axis(&axis(-10.0, 0.0), -4.26), -4.3);
        assert_eq!(round_axis(&axis(0.0, 1.0), 0.04), 0.0);
    }

    #[test]
    fn uniform_reports_mixed() {
        let a = TextStyle::default();
        let b = TextStyle { font_size: 30.0, ..Default::default() };
        assert_eq!(uniform(&[a.clone(), b.clone()], |s| s.font_family.clone()), Some("Inter".to_string()));
        assert_eq!(uniform(&[a.clone(), b], |s| s.font_size), None);
        assert_eq!(uniform(&[], |s: &TextStyle| s.font_size), None);
        // Ranges pick only the spans they touch.
        let mut doc = Document::new("t");
        let page = doc.first_page().unwrap();
        let id = doc.alloc_id();
        let mut td = TextData::new("Hello world");
        td.restyle(6, 11, |s| s.font_size = 40.0);
        doc.insert(Node::new(id, NodeKind::Text(td), "t", 10.0, 10.0), page, None).unwrap();
        assert_eq!(target_styles(&doc, &[id], Some((id, 0, 5))).len(), 1);
        assert_eq!(target_styles(&doc, &[id], Some((id, 4, 8))).len(), 2);
        assert_eq!(target_styles(&doc, &[id], Some((id, 3, 3))).len(), 2);
        assert_eq!(target_styles(&doc, &[id], None).len(), 2);
    }
}
