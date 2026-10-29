use super::axes::variable_axes;
use super::glyphs::{LeadGlyph, caption, feature_row, text_seg};
use super::picker::{font_picker, remember_family};
use super::spacing::{SpacingField, commit_spacing, parse_letter_spacing, parse_line_height, spacing_field, spacing_input};
use super::{
    Arc, DecorationLength, Document, FEATURES, FIELD_H, FieldEdit, FigApp, Icon, Id, Lead, LeadingTrim, ListStyle, MIXED, NodeId, SIZES, Sense, TABS,
    TextAlignH, TextAlignV, TextAutoResize, TextCase, TextData, TextDecoration, TextStyle, Tokens, Ui, Value, VariableType, common, field_row, icons,
    json, live, menu_item, menu_popup, regular, row, semibold, text_style_params, tr, tr_noop, trf, trim_num, uniform, vec2, widgets,
};

/// The font rows: family, style and size, line height and letter spacing.
pub(super) fn font_rows(
    app: &mut FigApp,
    ui: &mut Ui,
    ids: &[NodeId],
    styles: &[TextStyle],
    editing: bool,
    apply_style: &dyn Fn(&mut FigApp, &str, Value),
) {
    let t = Tokens::get(ui.ctx());
    let range = figcraft_engine::textedit::selected_range(&app.session);

    // Font family.
    let family = uniform(styles, |s| s.font_family.clone());
    let fstyle = uniform(styles, |s| s.font_style.clone());
    row(ui, 32.0, |ui| {
        let w = ui.available_width();
        let resp = font_dropdown(app, ui, ids, "fontFamily", family.as_deref(), w, editing);
        let before_search = |ui: &mut Ui| !editing && apply_variable_item(ui, ids, "fontFamily", &resp);
        if let Some(pick) = font_picker(ui, &resp, family.as_deref(), w.max(240.0), before_search) {
            if let Some(f) = pick.picked {
                remember_family(ui.ctx(), &f);
                apply_style(app, "fontFamily", json!(f));
            } else if let Some(h) = pick.hovered.or(pick.highlighted) {
                // The canvas shows the hovered family (or the one the arrow keys are on);
                // closing without picking ends the preview.
                app.preview_text(ui.ctx(), "text.style", text_style_params(ids, range, json!({"fontFamily": h})));
            }
        }
    });

    // Style + size.
    // One of the two field columns of the section's grid.
    let half = widgets::col_width(ui.available_width());
    row(ui, 32.0, |ui| {
        let resp = font_dropdown(app, ui, ids, "fontStyle", fstyle.as_deref(), half, editing);
        let fam = family.clone();
        menu_popup(&resp).width(200.0).show(|ui| {
            if !editing && apply_variable_item(ui, ids, "fontStyle", &resp) {
                return;
            }
            match &fam {
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
            }
        });
        let size = uniform(styles, |s| s.font_size);
        ui.spacing_mut().item_spacing.x = 0.0;
        // The whole layer's size can follow a variable; a selected range just takes a value.
        let e = if editing {
            widgets::number_field(ui, Id::unique("ty_size"), Lead::None, size, "", ui.available_width() - 20.0, 1.0)
        } else {
            crate::variable_ui::number_field(app, ui, ids, "fontSize", Id::unique("ty_size"), Lead::None, size, "", ui.available_width() - 20.0)
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
        let field = SpacingField { field: "lineHeight", id: Id::unique("ty_lh"), glyph: LeadGlyph::LineHeight, tip: tr("Line height") };
        if let Some(edit) = spacing_field(app, ui, ids, &field, &lh_label, half, editing)
            && let Some(v) = parse_line_height(&edit.text(), lh)
        {
            commit_spacing(app, ids, range, &field, &edit, v);
        }
        let ls = uniform(styles, |s| s.letter_spacing);
        let ls_label = ls.map(|l| l.label()).unwrap_or_else(|| tr(MIXED).into());
        let field = SpacingField { field: "letterSpacing", id: Id::unique("ty_ls"), glyph: LeadGlyph::LetterSpacing, tip: tr("Letter spacing") };
        if let Some(edit) = spacing_field(app, ui, ids, &field, &ls_label, half, editing)
            && let Some(v) = parse_letter_spacing(&edit.text(), ls)
        {
            commit_spacing(app, ids, range, &field, &edit, v);
        }
    });
}

/// Paragraph spacing, resizing, alignment and the Type settings popover.
pub(super) fn paragraph_rows(
    app: &mut FigApp,
    ui: &mut Ui,
    doc: &Document,
    ids: &[NodeId],
    styles: &[TextStyle],
    apply_style: &dyn Fn(&mut FigApp, &str, Value),
) {
    // One of the two field columns of the section's grid.
    let half = widgets::col_width(ui.available_width());
    // Paragraph spacing + resizing.
    let ps = common(doc, ids, |n| n.kind.text().map(|t| t.paragraph_spacing.to_bits())).flatten().map(f64::from_bits);
    let resize = common(doc, ids, |n| n.kind.text().map(|t| t.auto_resize)).flatten();
    row(ui, 32.0, |ui| {
        let ps_label = ps.map(trim_num).unwrap_or_else(|| tr(MIXED).into());
        let field =
            SpacingField { field: "paragraphSpacing", id: Id::unique("ty_ps"), glyph: LeadGlyph::ParagraphSpacing, tip: tr("Paragraph spacing") };
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
        let open_id = Id::unique("ty_settings_open");
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
pub(super) fn menu_item_hover(ui: &mut Ui, label: &str) -> (bool, bool) {
    let r = crate::right::menu_row(ui, label, "", 0.0, |_, _, _| {});
    (r.clicked(), r.hovered())
}

/// The font family or font style dropdown for `field`, `width` wide, showing `shown` (Mixed when
/// None). While whole layers are edited and `field` is bound to the same variable on every one of
/// `ids`, the variable's chip is drawn in place of the dropdown (a click picks another variable,
/// its detach button unbinds it), and the returned response opens no menu.
fn font_dropdown(app: &mut FigApp, ui: &mut Ui, ids: &[NodeId], field: &str, shown: Option<&str>, width: f32, editing: bool) -> egui::Response {
    if !editing && app.session.doc().ok().and_then(|d| crate::variable_ui::bound_name(d, ids, field)).is_some() {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
        crate::variable_ui::bound_chip(app, ui, ids, field, VariableType::String, rect);
        return resp;
    }
    widgets::dropdown(ui, shown.unwrap_or(tr(MIXED)), width, None)
}

/// Apply variable at the top of the font family or font style menu opened from `anchor`: a click
/// opens the variable picker on string variables for `field` of `ids` and closes the menu (true).
fn apply_variable_item(ui: &mut Ui, ids: &[NodeId], field: &str, anchor: &egui::Response) -> bool {
    let picked = menu_item(ui, tr("Apply variable"), "");
    if picked {
        crate::variable_ui::open(ui, ids, field, VariableType::String, anchor.rect.left_bottom());
        ui.close();
    } else {
        ui.separator();
    }
    picked
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
    let tab_id = Id::unique("ty_settings_tab");
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
                widgets::number_field(ui, Id::unique("ty_list_spacing"), Lead::None, spacing, "", fw, 1.0)
            {
                app.set_props(ids, json!({"listSpacing": v.clamp(0.0, 10_000.0)}));
            }
        });
        ui.add_space(4.0);
        let indent = common(doc, ids, |n| n.kind.text().map(|t| t.paragraph_indent.to_bits())).flatten().map(f64::from_bits);
        // Indent applies to start-aligned text only; elsewhere the field stays visible but off.
        let start_aligned =
            common(doc, ids, |n| n.kind.text().map(|t| t.align_h != TextAlignH::Center && t.align_h != TextAlignH::Justified)).flatten();
        ui.add_enabled_ui(start_aligned != Some(false), |ui| {
            let field =
                field_row(ui, tr("Paragraph indent"), |ui, fw| widgets::number_field(ui, Id::unique("ty_indent"), Lead::None, indent, "", fw, 1.0));
            if let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) = field {
                app.set_props(ids, json!({"paragraphIndent": v.clamp(0.0, 10_000.0)}));
            }
        })
        .response
        .on_disabled_hover_text(tr("Paragraph indent applies to left-aligned text"));
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
        // Auto-height text with a line limit truncates too.
        let truncating = common(doc, ids, |n| n.kind.text().map(TextData::truncates)).flatten();
        caption(ui, tr("Truncate text"));
        match text_seg(
            ui,
            "trunc",
            &[(tr("Off"), tr_noop("Don't truncate")), ("…", tr_noop("Truncate with an ellipsis"))],
            truncating.map(usize::from),
            w,
        ) {
            Some(1) if truncating != Some(true) => app.set_props(ids, json!({"textAutoResize": "truncate"})),
            Some(0) if resize == Some(TextAutoResize::Height) => {
                if truncating == Some(true) {
                    app.set_props(ids, json!({"maxLines": 0}));
                }
            }
            Some(0) if resize != Some(TextAutoResize::WidthAndHeight) => app.set_props(ids, json!({"textAutoResize": "none"})),
            _ => {}
        }
        // Max lines only matter while truncating; no limit shows "Auto".
        if truncating == Some(true) {
            let max = common(doc, ids, |n| n.kind.text().map(|t| t.max_lines)).flatten();
            ui.add_space(4.0);
            let shown = max.map(f64::from).filter(|m| *m > 0.0);
            let empty = if max.is_some() { tr("Auto") } else { tr(MIXED) };
            if let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, true)) =
                field_row(ui, tr("Max lines"), |ui, fw| widgets::number_field_or(ui, Id::unique("ty_maxlines"), shown, empty, fw, 1.0))
            {
                app.run("text.maxLines", json!({"ids": ids, "lines": v.round().clamp(0.0, 10_000.0)}));
            }
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
                widgets::number_field_or(ui, Id::unique(("ty_deco", key)), shown, &empty, field_w, 0.5)
            {
                // A thickness of 0 or less goes back to the font's.
                let v = if key == "textDecorationThickness" && v <= 0.0 { json!("auto") } else { json!(v.clamp(-1000.0, 1000.0)) };
                apply_style(app, key, v);
            }
            let tip = tr("Back to the font's value");
            if !auto
                && ui
                    .add(
                        egui::Label::new(egui::RichText::new(tr("Auto")).font(regular(11.0)).color(Tokens::get(ui.ctx()).accent_text))
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
        if let Some(s) = widgets::text_field(ui, Id::unique("ty_deco_color"), &value, fw, hint) {
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
