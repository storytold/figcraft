//! Figma-style compact controls: number fields with scrub labels, text fields, dropdowns,
//! segmented buttons, colour swatches and section headers.

use egui::{Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::a11y::{self, Tip};
pub use crate::field_math::{Rel, Typed, eval, parse, stepped};
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};

pub const ROW_H: f32 = 32.0;
pub const FIELD_H: f32 = 24.0;
pub const PAD: f32 = 16.0;

/// What a numeric field's label shows: text ("X") or an icon with the field's name (for screen
/// readers: an icon doesn't say which property it is).
#[derive(Clone, Copy)]
pub enum Lead<'a> {
    Text(&'a str),
    Icon(Icon, &'a str),
    None,
    /// No lead, only a name for screen readers (the label is beside the field).
    Named(&'a str),
}

impl Lead<'_> {
    /// The field's name for screen readers ("W" is "Width").
    pub fn name(&self) -> &str {
        match self {
            Lead::Text("W") => tr("Width"),
            Lead::Text("H") => tr("Height"),
            Lead::Text(s) => s,
            Lead::Icon(_, name) | Lead::Named(name) => name,
            Lead::None => "",
        }
    }
}

/// Result of a field edit.
pub enum FieldEdit {
    /// Committed a new value (Enter, Tab or focus loss; an arrow key's step).
    Set(f64),
    /// Scrubbed by dragging the label (live; `.1` is true when the drag ended).
    Scrub(f64, bool),
}

/// What a field over several layers gives ([`number_field_mixed`]): one value for all of them, or
/// (while it shows no single value: "Mixed") an edit relative to each layer's own (`+10`, `*2`, an
/// arrow key's step).
pub enum Edit {
    All(FieldEdit),
    Each(Rel),
}

impl Edit {
    /// For a field that has no value per layer to work from: `+10` is 10, as typed into an empty
    /// field, and other relative edits are nothing.
    pub fn settled(self) -> Option<FieldEdit> {
        match self {
            Edit::All(e) => Some(e),
            Edit::Each(Rel::Add(v)) => Some(FieldEdit::Set(v)),
            Edit::Each(_) => None,
        }
    }
}

/// The nudge amounts the arrow keys step pixel fields by (plain, with Shift): the preferences'
/// small and big nudge. Set once a frame.
pub fn set_nudge(ctx: &egui::Context, small: f64, big: f64) {
    ctx.data_mut(|d| d.insert_temp(Id::new("field_nudge"), (small, big)));
}

/// What an arrow key (with Shift: `.1`) and a pixel of scrubbing step a field by: a field in
/// pixels (no unit, whole steps) takes the nudge preferences from the keys, every other field
/// its own step and ten of them (1% and 10% of opacity, 1° and 10° of rotation).
fn key_steps(ctx: &egui::Context, suffix: &str, step: f64) -> (f64, f64) {
    let nudge =
        ctx.data(|d| d.get_temp::<(f64, f64)>(Id::new("field_nudge"))).filter(|(s, b)| s.is_finite() && b.is_finite() && *s > 0.0 && *b > 0.0);
    match nudge {
        Some(n) if suffix.is_empty() && step == 1.0 => n,
        _ => (step, step * 10.0),
    }
}

/// How far a scrub of `dx` points moves a value whose field steps by `step`: 1 step a point, ten
/// with Shift, a tenth with Alt (fine).
pub fn scrub_amount(dx: f64, step: f64, shift: bool, alt: bool) -> f64 {
    dx * step
        * match (shift, alt) {
            (true, false) => 10.0,
            (false, true) => 0.1,
            _ => 1.0,
        }
}

/// The value a scrub that began at `from` and has moved `amount` (see [`scrub_amount`]) shows:
/// whole steps of `quantum` away from where it began.
pub fn scrubbed(from: f64, amount: f64, quantum: f64) -> f64 {
    if quantum > 0.0 && quantum.is_finite() { stepped(from, (amount / quantum).round(), quantum) } else { from }
}

/// The shared label-scrub gesture, including changing modifier rates and raw edge motion.
/// Callers supply the hit area and apply live values until the final `Scrub(_, true)`.
pub(crate) fn scrub(ui: &Ui, id: Id, response: &Response, value: Option<f64>, step: f64) -> Option<FieldEdit> {
    let v = value?;
    // Where the scrub began and how far it has taken the value (the modifiers can change on
    // the way: each stretch counts at its own rate).
    let scrub_id = id.with("scrub");
    if response.dragged() || response.drag_stopped() {
        let began = response.drag_started();
        let (v0, mut amount): (f64, f64) = ui.data(|d| d.get_temp(scrub_id)).filter(|_| !began).unwrap_or((v, 0.0));
        let (dx, shift, alt) = ui.input(|i| {
            let p = &i.pointer;
            // As it begins: all the way from the press (a drag is one only after a few points).
            let moved = if began { p.hover_pos().zip(p.press_origin()).map_or(0.0, |(at, from)| at.x - from.x) } else { p.delta().x };
            // With the pointer stopped at the edge of the screen (or of a window that fills
            // it) the mouse's own motion goes on, where the platform reports it: the scrub
            // does too. Elsewhere the scrub just stops there.
            let raw = p.motion().map_or(0.0, |m| m.x / i.pixels_per_point);
            let at_edge = moved == 0.0 && p.hover_pos().is_none_or(|at| at.x <= 1.0 || at.x >= i.content_rect().max.x - 1.0);
            (f64::from(if at_edge { raw } else { moved }), i.modifiers.shift, i.modifiers.alt)
        });
        // The frame the button goes up moves nothing: the scrub ends where the last one left it.
        if response.dragged() {
            amount += scrub_amount(dx, step, shift, alt);
            ui.data_mut(|d| d.insert_temp(scrub_id, (v0, amount)));
        }
        let quantum = if alt { step * 0.1 } else { step };
        return Some(FieldEdit::Scrub(scrubbed(v0, amount, quantum), response.drag_stopped()));
    }
    None
}

pub(crate) fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}

/// What a number field's variable button did.
pub enum BindEvent {
    /// Open the variable picker below this point.
    Open(egui::Pos2),
    /// Detach the bound variable.
    Detach,
}

/// A number field `width` wide. `value` None shows "Mixed". `suffix` (e.g. "%" or "°") is shown
/// after the number. Typing supports simple math (`10+5`, `*2`).
pub fn number_field(ui: &mut Ui, id: Id, lead: Lead, value: Option<f64>, suffix: &str, width: f32, step: f64) -> Option<FieldEdit> {
    number_field_mixed(ui, id, lead, value, suffix, width, step)?.settled()
}

/// [`number_field`] over several layers: without a single value (`value` None, "Mixed") a relative
/// edit comes back as one for each layer ([`Edit::Each`]).
pub fn number_field_mixed(ui: &mut Ui, id: Id, lead: Lead, value: Option<f64>, suffix: &str, width: f32, step: f64) -> Option<Edit> {
    number_field_bindable(ui, id, lead, value, suffix, width, step, None, tr("Mixed")).0
}

/// [`number_field`] that shows `empty` (dim) for no value instead of "Mixed" (e.g. "Auto"): there
/// is no value to edit relative to, so `+4` is 4.
pub fn number_field_or(ui: &mut Ui, id: Id, value: Option<f64>, empty: &str, width: f32, step: f64) -> Option<FieldEdit> {
    number_field_impl(ui, id, Lead::None, value, "", width, step, None, empty).0?.settled()
}

/// [`number_field`] that can bind a variable: `binding` None = not bindable, `Some(None)` = shows
/// a variable button on hover, `Some(Some(name))` = bound (shows the name; click to change, the
/// detach button unbinds). No value shows `empty` ("Mixed", "Auto").
pub fn number_field_bindable(
    ui: &mut Ui,
    id: Id,
    lead: Lead,
    value: Option<f64>,
    suffix: &str,
    width: f32,
    step: f64,
    binding: Option<Option<&str>>,
    empty: &str,
) -> (Option<Edit>, Option<BindEvent>) {
    number_field_impl(ui, id, lead, value, suffix, width, step, binding, empty)
}

fn number_field_impl(
    ui: &mut Ui,
    id: Id,
    lead: Lead,
    value: Option<f64>,
    suffix: &str,
    width: f32,
    step: f64,
    binding: Option<Option<&str>>,
    empty: &str,
) -> (Option<Edit>, Option<BindEvent>) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    if let Some(Some(name)) = binding {
        return (None, bound_field(ui, id, lead, rect, name));
    }
    let mut out: Option<Edit> = None;
    let lead_w = match lead {
        Lead::None | Lead::Named(_) => 6.0,
        Lead::Text(s) => (12.0 + s.chars().count() as f32 * 7.0).max(24.0),
        Lead::Icon(..) => 24.0,
    };
    let lead_rect = Rect::from_min_size(rect.min, vec2(lead_w, FIELD_H));
    let edit_rect = Rect::from_min_max(pos2(rect.min.x + lead_w, rect.min.y), rect.max);
    let editing_id = id.with("editing");
    let mut editing: Option<String> = ui.data(|d| d.get_temp(editing_id));
    // A single value can be dragged as well as clicked (Mixed and Auto have nothing to scrub from).
    let field_sense = if value.is_some() { Sense::click_and_drag() } else { Sense::click() };
    let field_resp = ui.interact(edit_rect, id.with("field"), field_sense);
    a11y::number(&field_resp, lead.name(), value, suffix, empty);
    a11y::ring_around(ui, rect, &field_resp);
    // The label scrubs with the pointer; the keyboard stops at the value only.
    let lead_resp = ui.interact(lead_rect, id.with("lead"), Sense::CLICK | Sense::DRAG);
    // The variable button sits on the value's right end (bindable fields): it is clicked, not dragged.
    let bind_rect = Rect::from_center_size(pos2(rect.max.x - 11.0, rect.center().y), vec2(18.0, 18.0));
    let on_bind = |p: Option<egui::Pos2>| binding.is_some() && p.is_some_and(|p| bind_rect.contains(p));
    // A drag on the value scrubs too when it sets off sideways and not from the variable button; one
    // that sets off up or down scrolls the panel instead (the field holds the drag).
    let value_scrub_id = id.with("value_scrub");
    if field_resp.drag_started() {
        let (from, at) = ui.input(|i| (i.pointer.press_origin(), i.pointer.hover_pos()));
        let sideways = from.zip(at).is_some_and(|(f, a)| (a.x - f.x).abs() >= (a.y - f.y).abs());
        ui.data_mut(|d| d.insert_temp(value_scrub_id, editing.is_none() && sideways && !on_bind(from)));
    }
    let value_drag = field_resp.dragged() || field_resp.drag_stopped();
    let value_scrubs = value_drag && editing.is_none() && ui.data(|d| d.get_temp(value_scrub_id)).unwrap_or(false);
    if field_resp.dragged() && editing.is_none() && !value_scrubs {
        ui.scroll_with_delta(vec2(0.0, ui.input(|i| i.pointer.delta().y)));
    }
    let scrub_resp = if lead_resp.dragged() || lead_resp.drag_stopped() { Some(&lead_resp) } else { value_scrubs.then_some(&field_resp) };
    let hovered = field_resp.hovered() || lead_resp.hovered() || editing.is_some();
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    if editing.is_some() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.accent), StrokeKind::Inside);
    } else if hovered {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
    }
    match lead {
        Lead::Text(s) => {
            ui.painter().text(pos2(lead_rect.min.x + 8.0, rect.center().y), Align2::LEFT_CENTER, s, regular(11.0), t.text_secondary);
        }
        Lead::Icon(i, _) => {
            icons::paint(ui.painter(), Rect::from_center_size(lead_rect.center() + vec2(1.0, 0.0), vec2(14.0, 14.0)), i, t.icon_secondary)
        }
        Lead::None | Lead::Named(_) => {}
    }
    let over_value = field_resp.hovered() && editing.is_none() && value.is_some() && !on_bind(ui.input(|i| i.pointer.hover_pos()));
    if lead_resp.hovered() || lead_resp.dragged() || over_value || value_scrubs {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    // Scrub by dragging the label or the value.
    if let Some(edit) = scrub_resp.and_then(|r| scrub(ui, id, r, value, step)) {
        out = Some(Edit::All(edit));
    }
    let (key_step, key_big) = key_steps(ui.ctx(), suffix, step);
    // The keyboard is on the field (also in a window that isn't in front: keys reach one only
    // through the control channel then).
    let on_field = ui.memory(|m| m.has_focus(field_resp.id));
    // What ArrowUp or ArrowDown pressed this frame steps by (Shift: the big step).
    let arrow_step = |ui: &Ui| {
        ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key: egui::Key::ArrowUp, pressed: true, modifiers, .. } => Some(if modifiers.shift { key_big } else { key_step }),
                egui::Event::Key { key: egui::Key::ArrowDown, pressed: true, modifiers, .. } => {
                    Some(-if modifiers.shift { key_big } else { key_step })
                }
                _ => None,
            })
        })
    };
    // A relative edit of no single value goes to each layer's own.
    let set = |v: f64| Edit::All(FieldEdit::Set(v));
    let typed = |text: &str| match (parse(text, suffix == "%")?, value) {
        (Typed::Value(v), _) => Some(set(v)),
        (Typed::Rel(r), Some(cur)) => r.apply(cur).map(set),
        (Typed::Rel(r), None) => Some(Edit::Each(r)),
    };
    // With the keyboard on the field (Tab) and nothing typed yet, the arrows step it as it is.
    if editing.is_none()
        && on_field
        && let Some(by) = arrow_step(ui)
    {
        out = Some(value.map_or(Edit::Each(Rel::Add(by)), |v| set(stepped(v, 1.0, by))));
    }
    // Typing on a field the keyboard reached (Tab) edits it: the editor takes the typed text.
    let typed_now = editing.is_none() && on_field && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(_))));
    let started = editing.is_none() && (field_resp.clicked() || typed_now);
    // The text the field would commit to no effect: what it showed when the edit began, or the
    // value the arrows last stepped it to.
    let kept_id = id.with("kept");
    if started {
        editing = Some(if typed_now { String::new() } else { value.map(fmt_num).unwrap_or_default() });
        ui.data_mut(|d| {
            d.insert_temp(editing_id, editing.clone().unwrap_or_default());
            d.insert_temp(kept_id, value.map(fmt_num).unwrap_or_default());
            d.insert_temp(id.with("selectall"), !typed_now);
        });
        ui.memory_mut(|m| m.request_focus(id.with("te")));
    }
    match editing {
        Some(mut text) => {
            let te = egui::TextEdit::singleline(&mut text)
                .id(id.with("te"))
                .frame(egui::Frame::NONE)
                .font(regular(11.0))
                .desired_width(edit_rect.width() - 6.0)
                .vertical_align(egui::Align::Center);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect.shrink2(vec2(0.0, 4.0))));
            let r = child.add(te);
            a11y::own_focus(&r);
            keep_focus(&r, started);
            if ui.data(|d| d.get_temp::<bool>(id.with("selectall"))).unwrap_or(false)
                && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id.with("te"))
            {
                let ccursor = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
                state.cursor.set_char_range(Some(ccursor));
                state.store(ui.ctx(), id.with("te"));
                ui.data_mut(|d| d.remove::<bool>(id.with("selectall")));
            }
            // The Enter that started the edit (from the keyboard) doesn't end it.
            let enter = !started && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if ui.memory(|m| m.has_focus(r.id))
                && let Some(by) = arrow_step(ui)
            {
                // From what is typed (arithmetic too); no single value: each layer's own steps.
                match typed(&text) {
                    Some(Edit::All(FieldEdit::Set(v))) => {
                        let nv = stepped(v, 1.0, by);
                        text = fmt_num(nv);
                        // The stepped value stays selected: typing replaces it.
                        ui.data_mut(|d| {
                            d.insert_temp(kept_id, text.clone());
                            d.insert_temp(id.with("selectall"), true);
                        });
                        out = Some(set(nv));
                    }
                    _ if value.is_none() => out = Some(Edit::Each(Rel::Add(by))),
                    _ => {}
                }
            }
            if esc {
                // Back to what it was: nothing is committed.
                ui.data_mut(|d| d.remove::<String>(editing_id));
                ui.memory_mut(|m| m.surrender_focus(id.with("te")));
            } else if r.lost_focus() || enter {
                // Enter, Tab or a click elsewhere commits what was typed (a field left as it was
                // commits nothing: no undo step, and a rounded display isn't written back).
                let kept: String = ui.data(|d| d.get_temp(kept_id)).unwrap_or_default();
                if text != kept
                    && let Some(e) = typed(&text)
                {
                    out = Some(e);
                }
                ui.data_mut(|d| d.remove::<String>(editing_id));
            } else {
                ui.data_mut(|d| d.insert_temp(editing_id, text));
            }
        }
        None => {
            let s = match value {
                Some(v) => format!("{}{}", fmt_num(v), suffix),
                None => empty.to_string(),
            };
            let col = if value.is_some() { t.text } else { t.text_secondary };
            ui.painter().text(pos2(edit_rect.min.x, rect.center().y), Align2::LEFT_CENTER, s, regular(11.0), col);
        }
    }
    // The variable button shows while the pointer is over a bindable field.
    let mut bind = None;
    if binding.is_some() && ui.data(|d| d.get_temp::<String>(editing_id)).is_none() {
        let br = bind_rect;
        let bresp = ui.interact(br, id.with("bind"), Sense::click());
        if bresp.has_focus() || rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or(egui::Pos2::new(f32::NAN, f32::NAN))) {
            if bresp.hovered() {
                ui.painter().rect_filled(br, CornerRadius::same(4), t.hover);
            }
            icons::paint(ui.painter(), br.shrink(3.0), Icon::Variable, t.icon_secondary);
        }
        let bresp = bresp.tip(tr("Apply variable"));
        if bresp.clicked() {
            bind = Some(BindEvent::Open(rect.left_bottom()));
        }
    }
    (out, bind)
}

/// A field bound to a variable: the lead, then the variable's name in a chip; the chip opens the
/// picker and the detach button unbinds.
/// A field showing the variable `name` bound to it in `rect`: click to change, detach to unbind.
pub fn bound_field(ui: &mut Ui, id: Id, lead: Lead, rect: Rect, name: &str) -> Option<BindEvent> {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, id.with("bound"), Sense::click());
    a11y::combo(&resp, lead.name(), name, false);
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    if resp.hovered() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
    }
    let lead_w = match lead {
        Lead::None | Lead::Named(_) => 4.0,
        Lead::Text(s) => {
            ui.painter().text(pos2(rect.min.x + 8.0, rect.center().y), Align2::LEFT_CENTER, s, regular(11.0), t.text_secondary);
            (12.0 + s.chars().count() as f32 * 7.0).max(24.0)
        }
        Lead::Icon(i, _) => {
            icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.min.x + 13.0, rect.center().y), vec2(14.0, 14.0)), i, t.icon_secondary);
            24.0
        }
    };
    let dr = Rect::from_center_size(pos2(rect.max.x - 11.0, rect.center().y), vec2(18.0, 18.0));
    let chip = Rect::from_min_max(pos2(rect.min.x + lead_w - 2.0, rect.min.y + 3.0), pos2(dr.min.x - 2.0, rect.max.y - 3.0));
    ui.painter().rect_filled(chip, CornerRadius::same(4), t.hover);
    ui.painter().rect_stroke(chip, CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
    // Grouped names ("size/card") show their last part; the tooltip has the full name.
    let short = name.rsplit('/').next().unwrap_or(name);
    let galley = ui.painter().layout_no_wrap(short.to_string(), regular(11.0), t.text);
    ui.painter().with_clip_rect(chip.shrink2(vec2(5.0, 0.0))).galley(pos2(chip.min.x + 5.0, chip.center().y - galley.size().y / 2.0), galley, t.text);
    let dresp = ui.interact(dr, id.with("detach"), Sense::click()).tip(tr("Detach variable"));
    if resp.hovered() || dresp.hovered() || dresp.has_focus() {
        if dresp.hovered() {
            ui.painter().rect_filled(dr, CornerRadius::same(4), t.hover);
        }
        icons::paint(ui.painter(), dr.shrink(3.0), Icon::Detach, t.icon_secondary);
    }
    if dresp.clicked() {
        return Some(BindEvent::Detach);
    }
    let resp = resp.on_hover_text(name);
    resp.clicked().then(|| BindEvent::Open(rect.left_bottom()))
}

/// A text field that commits on Enter / focus loss. Returns the new text when committed.
pub fn text_field(ui: &mut Ui, id: Id, value: &str, width: f32, placeholder: &str) -> Option<String> {
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    text_field_in(ui, id, value, rect, placeholder, placeholder)
}

/// [`text_field`] without a placeholder, named `name` for screen readers (its label is beside it).
pub fn text_field_named(ui: &mut Ui, id: Id, value: &str, width: f32, name: &str) -> Option<String> {
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    text_field_in(ui, id, value, rect, "", name)
}

/// A multi-line text field across the panel (descriptions, notes) that commits when it loses
/// focus: returns the new text then, when it changed. The draft lives only while it has focus.
pub fn multiline_field(ui: &mut Ui, id: Id, value: &str, hint: &str, rows: usize) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let te = id.with("te");
    let focused = ui.memory(|m| m.has_focus(te));
    let mut text: String = if focused { ui.data(|d| d.get_temp(id)).unwrap_or_else(|| value.to_string()) } else { value.to_string() };
    let mut done = false;
    ui.horizontal(|ui| {
        ui.add_space(PAD);
        let w = ui.available_width() - PAD;
        let r = egui::Frame::NONE.fill(t.field).corner_radius(CornerRadius::same(5)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .id(te)
                    .frame(egui::Frame::NONE)
                    .font(regular(11.0))
                    .hint_text(hint)
                    .desired_rows(rows)
                    .desired_width(w - 12.0),
            )
        });
        done = r.inner.lost_focus();
    });
    if done {
        ui.data_mut(|d| d.remove::<String>(id));
        return (text != value).then_some(text);
    }
    if ui.memory(|m| m.has_focus(te)) {
        ui.data_mut(|d| d.insert_temp(id, text));
    }
    None
}

/// An editable field with a menu of `options` behind the chevron at its right (export sizes…).
/// Returns the typed text (on Enter / focus loss) or the option picked.
pub fn combo_field(ui: &mut Ui, id: Id, value: &str, width: f32, options: &[String]) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    let mut out = text_field_in(ui, id, value, rect, "", "");
    // The chevron sits on top of the field's right end.
    let chev = Rect::from_min_max(pos2(rect.max.x - 18.0, rect.min.y), rect.max);
    let resp = ui.interact(chev, id.with("menu"), Sense::click());
    a11y::combo(&resp, tr("Presets"), value, a11y::menu_open(&resp));
    if resp.hovered() {
        ui.painter().rect_filled(chev.shrink(2.0), CornerRadius::same(4), t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(chev.center(), vec2(12.0, 12.0)), Icon::Chevron, t.icon_secondary);
    crate::right::menu_popup(&resp).show(|ui| {
        for o in options {
            if crate::right::menu_item(ui, o, "") {
                out = Some(o.clone());
                ui.close();
            }
        }
    });
    out
}

/// [`text_field`] drawn into `rect`; `name` names it for screen readers.
fn text_field_in(ui: &mut Ui, id: Id, value: &str, rect: Rect, placeholder: &str, name: &str) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let editing_id = id.with("editing");
    let mut editing: Option<String> = ui.data(|d| d.get_temp(editing_id));
    let resp = ui.interact(rect, id.with("click"), Sense::click());
    a11y::text(&resp, name, value);
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    if editing.is_some() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.accent), StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
    }
    let started = editing.is_none() && resp.clicked();
    if started {
        editing = Some(value.to_string());
        ui.memory_mut(|m| m.request_focus(id.with("te")));
        select_all_next(ui, id.with("te"));
    }
    let mut out = None;
    match editing {
        Some(mut text) => {
            let r = field_editor(ui, rect, id.with("te"), &mut text, "");
            a11y::own_focus(&r);
            keep_focus(&r, started);
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
            let (s, col) = if value.is_empty() { (placeholder, t.text_tertiary) } else { (value, t.text) };
            // Long text stays inside the field.
            let painter = ui.painter().with_clip_rect(rect.shrink2(vec2(8.0, 0.0)));
            painter.text(pos2(rect.min.x + 8.0, rect.center().y), Align2::LEFT_CENTER, s, regular(11.0), col);
        }
    }
    out
}

/// The editor inside a field's `rect`: one line of text in the field's font, without a frame of
/// its own (the field draws it), showing `hint` while empty.
fn field_editor(ui: &mut Ui, rect: Rect, id: Id, text: &mut String, hint: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(8.0, 4.0))));
    let r = child.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(regular(11.0))
            .hint_text(egui::RichText::new(hint).font(regular(11.0)).color(t.text_tertiary))
            .desired_width(rect.width() - 16.0),
    );
    select_all_on_focus(ui, &r, text);
    r
}

/// A text field that is typed into in place: `text` changes as it is typed (the caller decides
/// what Enter does). As tall as a button and drawn like [`text_field`], so the two line up in a
/// row. Returns the editor's response (name it for screen readers with `a11y::text`).
pub fn text_input(ui: &mut Ui, id: Id, text: &mut String, width: f32, hint: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    let r = field_editor(ui, rect, id, text, hint);
    if r.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.accent), StrokeKind::Inside);
    } else if ui.rect_contains_pointer(rect) {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
    }
    r
}

/// Focus for a field's editor that appears on the click that starts editing: egui takes focus
/// away from a widget a click lands outside of, and a widget that is new this frame isn't under
/// the pointer yet, so the focus is asked for again once it exists.
fn keep_focus(r: &Response, started: bool) {
    if started && !r.has_focus() {
        r.request_focus();
    }
}

/// Selects all of a one-line editor's `text` when it gains the keyboard (a click, Tab or a focus
/// request), so what is typed replaces the value, as in Figma's fields. Call it after adding the
/// editor (`r` is its response). egui puts the caret where the press landed, so the selection is
/// laid again until the button is released; a drag from the press selects what it covers instead.
pub fn select_all_on_focus(ui: &Ui, r: &Response, text: &str) {
    let pending = select_all_id(r.id);
    if r.gained_focus() {
        ui.data_mut(|d| d.insert_temp(pending, true));
    }
    if !ui.data(|d| d.get_temp::<bool>(pending)).unwrap_or(false) {
        return;
    }
    // Not focused yet (the click that starts an edit can land before the editor exists): wait.
    if !r.has_focus() {
        return;
    }
    let (down, dragged) = ui.input(|i| (i.pointer.any_down(), i.pointer.total_drag_delta().is_some_and(|d| d.length() > 6.0)));
    if down && dragged {
        ui.data_mut(|d| d.remove::<bool>(pending));
        return;
    }
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), r.id);
        ui.ctx().request_repaint();
    }
    if !down {
        ui.data_mut(|d| d.remove::<bool>(pending));
    }
}

/// Has [`select_all_on_focus`] select all of editor `te_id` from now on (an edit that starts
/// with the editor already focused, which then gains no focus).
fn select_all_next(ui: &Ui, te_id: Id) {
    ui.data_mut(|d| d.insert_temp(select_all_id(te_id), true));
}

fn select_all_id(te_id: Id) -> Id {
    te_id.with("select_all_on_focus")
}

/// What a dropdown shows before its label.
#[derive(Clone, Copy)]
pub enum DropLead {
    Icon(Icon),
    /// A colour dot (a category's colour).
    Dot(Color32),
}

/// A dropdown-looking button showing `label`; returns the response (open a menu on click).
pub fn dropdown(ui: &mut Ui, label: &str, width: f32, lead: Option<Icon>) -> Response {
    dropdown_lead(ui, label, width, lead.map(DropLead::Icon))
}

/// [`dropdown`] with an icon or a colour dot before the label.
pub fn dropdown_lead(ui: &mut Ui, label: &str, width: f32, lead: Option<DropLead>) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::click());
    a11y::combo(&resp, "", label, a11y::menu_open(&resp));
    let bg = t.field;
    ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    if resp.hovered() {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.field_hover_border), StrokeKind::Inside);
    }
    let mut x = rect.min.x + 8.0;
    match lead {
        Some(DropLead::Icon(i)) => {
            icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 6.0, rect.center().y), vec2(14.0, 14.0)), i, t.icon_secondary);
            x += 18.0;
        }
        Some(DropLead::Dot(c)) => {
            ui.painter().circle_filled(pos2(x + 5.0, rect.center().y), 4.5, c);
            x += 16.0;
        }
        None => {}
    }
    // One line: a label longer than the field (a variable font style like "SemiBold SemiCondensed",
    // a long translation) ends with an ellipsis and shows in full on hover.
    let shown = elide(ui, label, rect.max.x - x - 20.0);
    let galley = ui.painter().layout_no_wrap(shown.clone(), regular(11.0), t.text);
    ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, t.text);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.max.x - 10.0, rect.center().y), vec2(12.0, 12.0)), Icon::Chevron, t.icon_secondary);
    if shown != label { resp.on_hover_text(label) } else { resp }
}

/// `s` shortened with an ellipsis to fit `max_w` on one line at the UI's 11 px.
pub fn elide(ui: &Ui, s: &str, max_w: f32) -> String {
    elide_font(ui, s, max_w, regular(11.0))
}

/// [`elide`] in `font`.
pub fn elide_font(ui: &Ui, s: &str, max_w: f32, font: egui::FontId) -> String {
    let fits = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x <= max_w;
    if fits(s) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    (1..chars.len())
        .rev()
        .map(|n| format!("{}…", chars.iter().take(n).collect::<String>().trim_end()))
        .find(|c| fits(c))
        .unwrap_or_else(|| "…".into())
}

/// [`elide_font`] cut at its start, keeping its end: a path keeps its file and the folders
/// nearest it.
pub fn elide_start_font(ui: &Ui, s: &str, max_w: f32, font: egui::FontId) -> String {
    let fits = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x <= max_w;
    if fits(s) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    (1..chars.len())
        .rev()
        .map(|n| format!("…{}", chars[chars.len() - n..].iter().collect::<String>().trim_start()))
        .find(|c| fits(c))
        .unwrap_or_else(|| "…".into())
}

/// A row of icon buttons; returns the index clicked.
pub fn segmented(ui: &mut Ui, items: &[(Icon, &str)], active: Option<usize>, cell: f32) -> Option<usize> {
    let names: Vec<&str> = items.iter().map(|(_, tip)| *tip).collect();
    segments(ui, &names, active, cell, true, |ui, r, i, on, t| {
        if let Some((icon, _)) = items.get(i) {
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), *icon, if on { t.icon } else { t.icon_secondary });
        }
    })
}

/// A choice between a few labelled options across the available width (the active one raised);
/// returns the index clicked. A label too long for its cell ends with an ellipsis.
pub fn segmented_text(ui: &mut Ui, labels: &[&str], active: usize) -> Option<usize> {
    let cell = (ui.available_width() / labels.len().max(1) as f32).floor();
    segments(ui, labels, Some(active), cell, false, |ui, r, i, on, t| {
        let label = labels.get(i).copied().unwrap_or_default();
        let font = if on { semibold(11.0) } else { regular(11.0) };
        let shown = elide_font(ui, label, r.width() - 12.0, font.clone());
        ui.painter().text(r.center(), Align2::CENTER_CENTER, shown, font, if on { t.text } else { t.text_secondary });
    })
}

/// The cells of a segmented control, each `cell` wide and named by `names` (shown on hover when
/// `tips`), drawn by `paint(ui, cell, index, active, tokens)`; returns the index clicked.
fn segments(
    ui: &mut Ui,
    names: &[&str],
    active: Option<usize>,
    cell: f32,
    tips: bool,
    paint: impl Fn(&Ui, Rect, usize, bool, &Tokens),
) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let w = cell * names.len() as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(w, FIELD_H), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    let mut out = None;
    for (i, name) in names.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.min.x + i as f32 * cell, rect.min.y), vec2(cell, FIELD_H));
        let mut resp = ui.interact(r, ui.id().with(("seg", i, *name)), Sense::click());
        if tips {
            resp = resp.on_hover_text(*name);
        }
        a11y::toggle(&resp, name, active == Some(i));
        if active == Some(i) {
            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(4), t.panel);
            ui.painter().rect_stroke(r.shrink(2.0), CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(4), t.hover);
        }
        paint(ui, r, i, active == Some(i), &t);
        if resp.clicked() {
            out = Some(i);
        }
    }
    out
}

/// A row that shows or hides what is under it: a chevron and `label`; returns whether it was
/// clicked.
pub fn disclosure(ui: &mut Ui, label: &str, open: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(11.0), t.text_secondary);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 22.0, FIELD_H), Sense::click());
    a11y::disclosure(&resp, label, open);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    let chevron = Rect::from_center_size(pos2(rect.min.x + 8.0, rect.center().y), vec2(10.0, 10.0));
    icons::paint(ui.painter(), chevron, if open { Icon::Chevron } else { Icon::ChevronRight }, t.icon_secondary);
    ui.painter().galley(pos2(rect.min.x + 18.0, rect.center().y - galley.size().y / 2.0), galley, t.text_secondary);
    resp.clicked()
}

/// A section header ("Fill", "Stroke"…) with optional right-side icon buttons; returns which
/// button was clicked. `dim` greys the title (empty sections).
pub fn section_header(ui: &mut Ui, title: &str, buttons: &[(Icon, &str)], dim: bool) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    ui.painter().text(
        pos2(rect.min.x + PAD, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        semibold(11.0),
        if dim { t.text_secondary } else { t.text },
    );
    let mut out = None;
    let mut x = rect.max.x - 8.0;
    for (i, (icon, tip)) in buttons.iter().enumerate().rev() {
        let r = Rect::from_min_size(pos2(x - 24.0, rect.center().y - 12.0), vec2(24.0, 24.0));
        let resp = ui.interact(r, ui.id().with(("hdr", title, i)), Sense::click()).tip(*tip);
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
        }
        icons::paint(ui.painter(), r.shrink(5.0), *icon, t.icon);
        if resp.clicked() {
            out = Some(i);
        }
        x -= 24.0;
    }
    out
}

/// An icon button at `rect` (hover highlight, tooltip); true when clicked.
pub fn icon_button_at(ui: &mut Ui, id: Id, rect: Rect, icon: Icon, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(rect, id, Sense::click()).tip(tip);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.hover);
    }
    icons::paint(ui.painter(), rect.shrink(5.0), icon, t.icon);
    resp.clicked()
}

/// A 24×24 icon button laid out in place; true when clicked.
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
    let id = ui.id().with(("icon_button", tip));
    icon_button_at(ui, id, rect, icon, tip)
}

/// A bar floating at the top centre of the `canvas` area (a version preview, the focus view):
/// `add` lays out its contents in a row.
pub fn canvas_banner<R>(ctx: &egui::Context, id: Id, canvas: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ctx);
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(pos2(canvas.center().x, canvas.min.y + 12.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.panel)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(CornerRadius::same(8))
                .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 12, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin { left: 14, right: 8, top: 8, bottom: 8 })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        add(ui)
                    })
                    .inner
                })
                .inner
        })
        .inner
}

/// Where a dragged row will land: an accent line across `x0..x1` at `y`.
pub fn insertion_line(painter: &egui::Painter, x0: f32, x1: f32, y: f32) {
    painter.line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(2.0, crate::theme::ACCENT));
}

/// What a drag carries, following the pointer at `at`: `galley` in a small outlined box above
/// everything else, with the grabbing hand.
pub fn drag_ghost(ctx: &egui::Context, at: egui::Pos2, galley: std::sync::Arc<egui::Galley>) {
    let t = Tokens::get(ctx);
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("drag_ghost")));
    let r = Rect::from_min_size(at + vec2(14.0, 10.0), galley.size() + vec2(16.0, 8.0));
    painter.rect_filled(r, CornerRadius::same(5), t.panel);
    painter.rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, crate::theme::ACCENT), StrokeKind::Inside);
    painter.galley(r.min + vec2(8.0, 4.0), galley, t.text);
}

pub fn separator(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, t.border));
}

/// Paint a checkerboard (transparency) in `rect`.
pub fn checker(painter: &egui::Painter, rect: Rect, cell: f32) {
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::WHITE);
    let nx = (rect.width() / cell).ceil() as i32;
    let ny = (rect.height() / cell).ceil() as i32;
    for y in 0..ny {
        for x in 0..nx {
            if (x + y) % 2 == 0 {
                continue;
            }
            let r = Rect::from_min_size(pos2(rect.min.x + x as f32 * cell, rect.min.y + y as f32 * cell), vec2(cell, cell)).intersect(rect);
            painter.rect_filled(r, CornerRadius::ZERO, Color32::from_gray(0xE6));
        }
    }
}

/// A paint swatch (with checkerboard under transparent colours).
pub fn swatch(ui: &mut Ui, paint: &figcraft_color::Paint, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    // The value is only spelled out while a screen reader reads the tree.
    let value = if a11y::active(&resp.ctx) { paint_name(paint) } else { String::new() };
    a11y::color(&resp, tr("Color"), &value);
    paint_swatch(ui.painter(), rect, paint);
    resp
}

/// A paint as a screen reader says it: "#FF0000", "#FF0000 50%", "Gradient", "Image", "Pattern".
pub fn paint_name(paint: &figcraft_color::Paint) -> String {
    use figcraft_color::PaintKind;
    match &paint.kind {
        PaintKind::Solid { color } => {
            let hex = color.to_hex();
            let alpha = color.a * paint.opacity;
            if alpha < 0.999 { format!("{hex} {}%", (alpha * 100.0).round()) } else { hex }
        }
        PaintKind::Gradient(_) => tr("Gradient").into(),
        PaintKind::Image(_) => tr("Image").into(),
        PaintKind::Pattern(_) => tr("Pattern").into(),
    }
}

pub fn paint_swatch(painter: &egui::Painter, rect: Rect, paint: &figcraft_color::Paint) {
    let t = Tokens::get(painter.ctx());
    use figcraft_color::PaintKind;
    let clip = painter.with_clip_rect(rect);
    checker(&clip, rect, rect.width() / 4.0);
    match &paint.kind {
        PaintKind::Solid { color } => {
            // Left half opaque, right half with alpha (like Figma's swatches).
            let c = crate::theme::doc_c32(painter.ctx(), color.with_alpha(color.a * paint.opacity));
            let opaque = crate::theme::doc_c32(painter.ctx(), color.with_alpha(1.0));
            clip.rect_filled(Rect::from_min_max(rect.min, pos2(rect.center().x, rect.max.y)), CornerRadius::ZERO, opaque);
            clip.rect_filled(Rect::from_min_max(pos2(rect.center().x, rect.min.y), rect.max), CornerRadius::ZERO, c);
            if paint.opacity >= 0.999 && color.a >= 0.999 {
                clip.rect_filled(rect, CornerRadius::ZERO, opaque);
            }
        }
        PaintKind::Gradient(g) => {
            let n = 16;
            for i in 0..n {
                let c = crate::theme::doc_c32(painter.ctx(), g.sample(i as f32 / (n - 1) as f32));
                let x0 = rect.min.x + rect.width() * i as f32 / n as f32;
                let x1 = rect.min.x + rect.width() * (i + 1) as f32 / n as f32;
                clip.rect_filled(Rect::from_min_max(pos2(x0, rect.min.y), pos2(x1, rect.max.y)), CornerRadius::ZERO, c);
            }
        }
        PaintKind::Image(_) | PaintKind::Pattern(_) => {
            let icon = if matches!(paint.kind, PaintKind::Image(_)) { Icon::Image } else { Icon::ViewGrid };
            clip.rect_filled(rect, CornerRadius::ZERO, Color32::from_gray(0xB3));
            icons::paint(&clip, rect.shrink(2.0), icon, Color32::WHITE);
        }
    }
    painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, t.border), StrokeKind::Inside);
}

/// A small checkbox with label.
pub fn checkbox(ui: &mut Ui, on: bool, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(200.0), FIELD_H), Sense::click());
    a11y::checkbox(&resp, label, on);
    let b = Rect::from_min_size(pos2(rect.min.x, rect.center().y - 7.0), vec2(14.0, 14.0));
    if on {
        ui.painter().rect_filled(b, CornerRadius::same(3), t.accent);
        icons::paint(ui.painter(), b.shrink(1.5), Icon::Check, Color32::WHITE);
    } else {
        ui.painter().rect_stroke(b, CornerRadius::same(3), Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
    }
    ui.painter().text(pos2(b.max.x + 8.0, rect.center().y), Align2::LEFT_CENTER, label, regular(11.0), t.text);
    resp.clicked()
}

/// A radio button with label; returns whether it was clicked.
pub fn radio(ui: &mut Ui, on: bool, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(240.0), FIELD_H), Sense::click());
    a11y::radio(&resp, label, on);
    let c = pos2(rect.min.x + 7.0, rect.center().y);
    if on {
        ui.painter().circle_filled(c, 7.0, t.accent);
        ui.painter().circle_filled(c, 2.5, Color32::WHITE);
    } else {
        ui.painter().circle_stroke(c, 6.5, Stroke::new(1.0, if resp.hovered() { t.accent } else { t.border_strong }));
    }
    ui.painter().text(pos2(rect.min.x + 22.0, rect.center().y), Align2::LEFT_CENTER, label, regular(11.0), t.text);
    resp.clicked()
}

/// A text button: `primary` is filled with the accent colour, otherwise outlined. A disabled
/// button is dimmed and never reports clicks.
pub fn button(ui: &mut Ui, label: &str, primary: bool, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(11.0), t.text);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0, FIELD_H), sense);
    a11y::button(&resp, label);
    let hot = enabled && resp.hovered();
    let (fill, text) = match (primary, hot) {
        (true, false) => (t.accent, Color32::WHITE),
        (true, true) => (t.accent_hover, Color32::WHITE),
        (false, false) => (Color32::TRANSPARENT, t.text),
        (false, true) => (t.hover, t.text),
    };
    let dim = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
    ui.painter().rect_filled(rect, CornerRadius::same(5), dim(fill));
    if !primary {
        ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, regular(11.0), dim(text));
    resp
}

/// Text in a row.
pub fn label(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(egui::RichText::new(text).font(regular(11.0)).color(color));
}

/// Text shown in `rect` that turns into an editor on click (or when its editor has focus, e.g.
/// after a double-click rename). Returns the new text on Enter / focus loss.
pub fn text_field_inline(ui: &mut Ui, id: Id, value: &str, rect: Rect, font: egui::FontId) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let te_id = id.with("te");
    let buf_id = id.with("buf");
    let focused = ui.memory(|m| m.has_focus(te_id));
    let mut buf: Option<String> = ui.data(|d| d.get_temp(buf_id));
    let resp = ui.interact(rect, id.with("click"), Sense::click());
    a11y::text(&resp, tr("Name"), value);
    let started = buf.is_none() && (focused || resp.clicked());
    if started {
        buf = Some(value.to_string());
        ui.memory_mut(|m| m.request_focus(te_id));
        select_all_next(ui, te_id);
    }
    match buf {
        Some(mut text) => {
            ui.painter().rect_filled(rect.expand(2.0), CornerRadius::same(4), t.panel);
            ui.painter().rect_stroke(rect.expand(2.0), CornerRadius::same(4), Stroke::new(1.0, t.accent), StrokeKind::Inside);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            let r = crate::bidi::single_line(&mut child, te_id, &mut text, font, rect.width());
            a11y::own_focus(&r);
            keep_focus(&r, started);
            // A rename or a hex edit starts with the whole text selected.
            select_all_on_focus(ui, &r, &text);
            // The Enter that started the edit (from the keyboard) doesn't end it.
            let enter = !started && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if esc {
                ui.data_mut(|d| d.remove::<String>(buf_id));
                ui.memory_mut(|m| m.surrender_focus(te_id));
                None
            } else if r.lost_focus() || enter {
                ui.data_mut(|d| d.remove::<String>(buf_id));
                ui.memory_mut(|m| m.surrender_focus(te_id));
                (text != value && !text.trim().is_empty()).then_some(text)
            } else {
                ui.data_mut(|d| d.insert_temp(buf_id, text));
                None
            }
        }
        None => {
            if resp.hovered() {
                ui.painter().rect_filled(rect.expand(2.0), CornerRadius::same(4), t.hover);
            }
            crate::bidi::paint_name(ui.painter(), rect.left_center(), value, font, t.text, rect.width());
            None
        }
    }
}

/// [`text_field_inline`] for a rename in progress: it takes the keyboard, with the whole name
/// selected, when `start`, and the rename is over once it lets go. Returns the new name once
/// committed and whether the rename goes on.
pub fn rename_field(ui: &mut Ui, id: Id, value: &str, rect: Rect, font: egui::FontId, start: bool) -> (Option<String>, bool) {
    let te_id = id.with("te");
    if start {
        ui.data_mut(|d| d.remove::<String>(id.with("buf")));
        ui.memory_mut(|m| m.request_focus(te_id));
    }
    let out = text_field_inline(ui, id, value, rect, font);
    let open = out.is_none() && ui.memory(|m| m.has_focus(te_id));
    (out, open)
}

/// Tab (`Some(false)`) or Shift+Tab (`Some(true)`) pressed this frame, taken from the input and
/// from egui's focus traversal, for a field that moves on to the next item itself.
pub fn take_tab(ui: &Ui) -> Option<bool> {
    let back = ui.input_mut(|i| {
        if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
            Some(true)
        } else {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Tab).then_some(false)
        }
    })?;
    ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    Some(back)
}

/// A short secondary note (an empty list's hint) that wraps inside the panel's side padding.
pub fn note(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(PAD as i8, 6)).show(ui, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(text).font(regular(11.0)).color(color)).wrap());
    });
}

/// A small grey label above a row of fields ("Position", "Dimensions"…). With two labels the
/// second starts at the second field column.
pub fn sublabels(ui: &mut Ui, a: &str, b: Option<&str>) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
    let col = col_width(w);
    // Each label fits its column on one line (long translations end with an ellipsis, in full on hover).
    let a_w = if b.is_some() { col } else { w - PAD * 2.0 };
    for (i, (text, x, max_w)) in [(a, PAD, a_w)].into_iter().chain(b.map(|b| (b, PAD + col + 8.0, w - col - 8.0 - PAD * 2.0))).enumerate() {
        let shown = elide_font(ui, text, max_w, regular(10.0));
        let at = ui.painter().text(pos2(r.min.x + x, r.max.y - 2.0), Align2::LEFT_BOTTOM, &shown, regular(10.0), t.text_secondary);
        if shown != text {
            ui.interact(at, ui.id().with(("sublabel", i)), Sense::hover()).on_hover_text(text);
        }
    }
}

/// Width of one of the two field columns in a panel `w` wide (leaving the icon column).
pub fn col_width(w: f32) -> f32 {
    ((w - PAD * 2.0 - 8.0 - 32.0) / 2.0).max(40.0)
}
