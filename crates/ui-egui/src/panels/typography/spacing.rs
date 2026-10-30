use super::glyphs::{LeadGlyph, paint_lead};
use super::{
    Align2, CornerRadius, FIELD_H, FieldEdit, FigApp, Id, LetterSpacing, LineHeight, MIXED, NodeId, Rect, Sense, Stroke, StrokeKind, Tokens, Ui,
    Value, VariableType, commit_text, json, live, pos2, regular, tr, trim_num, vec2, widgets,
};

/// Line height, letter spacing or paragraph spacing: property, field id, glyph and tooltip.
pub(super) struct SpacingField {
    pub(super) field: &'static str,
    pub(super) id: Id,
    pub(super) glyph: LeadGlyph,
    pub(super) tip: &'static str,
}

pub(super) enum SpacingEdit {
    Typed(String),
    Scrub(f64, bool),
}

impl SpacingEdit {
    pub(super) fn text(&self) -> String {
        match self {
            Self::Typed(text) => text.clone(),
            Self::Scrub(value, _) => trim_num(*value),
        }
    }
}

/// Keep typed unit parsing and character-range targeting; only a scrub outside a text-editing
/// session needs its own interaction. The editing session already owns its undo step.
pub(super) fn commit_spacing(
    app: &mut FigApp,
    ids: &[NodeId],
    range: Option<(NodeId, usize, usize)>,
    field: &SpacingField,
    edit: &SpacingEdit,
    value: Value,
) {
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
pub(super) fn spacing_input(ui: &mut Ui, field: &SpacingField, value: &str, width: f32) -> Option<SpacingEdit> {
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
pub(super) fn spacing_field(
    app: &mut FigApp,
    ui: &mut Ui,
    ids: &[NodeId],
    f: &SpacingField,
    value: &str,
    width: f32,
    range: bool,
) -> Option<SpacingEdit> {
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

/// A spacing value beside its scrub icon, centered in the same field while idle and editing.
/// Commits typed text on Enter or focus loss; Escape cancels the draft.
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
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, crate::theme::ACCENT), StrokeKind::Inside);
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
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect.shrink2(vec2(2.0, 0.0))));
            let r = child.add(
                egui::TextEdit::singleline(&mut text)
                    .id(id.with("te"))
                    .frame(egui::Frame::NONE)
                    .font(regular(11.0))
                    .align(Align2::LEFT_CENTER)
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
