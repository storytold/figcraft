//! The easing panel: shown in the inspector when a segment between two keyframes is selected.
//!
//! Lists the preset curves and applies the chosen one to the selected keyframe (the segment's
//! start), so the interpolation of that segment changes immediately.

use egui::{Align2, CornerRadius, Layout, Rect, RichText, Sense, Shape, Stroke, Ui, UiBuilder, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{
    DEFAULT_ELASTICITY, ELASTICITY_RANGE, Ease, SpringAlgorithm, spring_angular_frequency, spring_damping_ratio,
    spring_stiffness_damping, unit_mass,
};

use crate::FigApp;
use crate::motion::SelectedKey;
use crate::panels::right::row;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets::{self, FieldEdit, Lead};

pub fn panel(app: &mut FigApp, ui: &mut Ui, rect: Rect, t: &Tokens) {
    ui.painter().line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0, t.border));
    egui::ScrollArea::vertical().id_salt("motion_easing").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
        let Some(k) = app.ui.motion.selected else {
            ui.label(RichText::new("Select a segment").font(regular(11.0)).color(t.text_tertiary));
            return;
        };
        let current = easing_of(app, k);
        // Header: back arrow at 8 pt, the title centred — the other panels' header row.
        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        let br = Rect::from_min_size(pos2(head.min.x + 8.0, head.center().y - 12.0), vec2(24.0, 24.0));
        let bresp = ui.interact(br, ui.scope_id().with("ease_back"), Sense::click()).on_hover_text("Back");
        if bresp.hovered() {
            ui.painter().rect_filled(br, CornerRadius::same(5), t.hover);
        }
        ui.painter().rect_filled(Rect::from_center_size(br.center(), vec2(12.0, 12.0)), CornerRadius::same(2), t.icon);
        ui.painter().text(head.center(), Align2::CENTER_CENTER, "Easing", semibold(11.5), t.text);
        widgets::separator(ui);
        widgets::sublabels(ui, current.name(), None);
        // Curve | Spring tabs, the spring-aware graph and the values field. A handle drag is one
        // undo step; a discrete change (tab, Reset, typed value) commits on its own.
        let h = editor_height(current);
        let (er, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        let ch = editor(ui, er, "panel", widgets::PAD, current, t);
        if ch.started {
            let _ = app.session.begin_interaction("Ease curve");
        }
        if let Some(e) = ch.ease {
            let live = !ch.discrete && !ch.stopped;
            apply(app, k, e, live);
        }
        if ch.stopped {
            let _ = app.session.commit_interaction();
        }
        widgets::separator(ui);
        widgets::sublabels(ui, "Presets", None);
        let wide = widgets::col_width(ui.available_width()) * 2.0 + 8.0;
        for (name, ease) in Ease::PRESETS {
            row(ui, 32.0, |ui| {
                let on = current == ease;
                let (r, resp) = ui.allocate_exact_size(vec2(wide, 24.0), Sense::click());
                if on {
                    ui.painter().rect_filled(r, CornerRadius::same(5), t.selected_row);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
                }
                ui.painter().text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, name, regular(10.0), t.text);
                if resp.clicked() {
                    apply(app, k, ease, false);
                }
            });
        }
        if bresp.clicked() {
            app.ui.motion.easing_target = false;
        }
    });
}

fn easing_of(app: &FigApp, k: SelectedKey) -> Ease {
    app.session
        .doc()
        .ok()
        .and_then(|d| d.node(k.node))
        .and_then(|n| n.motion.track(k.property))
        .and_then(|tr| tr.key_at(k.time_ms))
        .map(|key| key.easing)
        .unwrap_or(Ease::Linear)
}

/// Writes `ease` into the selected keyframe's segment. `live` keeps the interaction open (a handle
/// drag), so the whole drag is one undo step.
fn apply(app: &mut FigApp, k: SelectedKey, ease: Ease, live: bool) {
    let _ = app.session.begin_interaction("Ease curve");
    if let Ok(doc) = app.session.doc_mut() {
        if let Some(tr) = doc.node_mut(k.node).and_then(|n| n.motion.track_mut(k.property))
            && let Some(idx) = tr.key_index(k.time_ms)
        {
            tr.keys[idx].easing = ease;
        }
        doc.mark_dirty([k.node]);
    }
    if !live {
        let _ = app.session.commit_interaction();
    }
}

/// What a frame of the easing editor changed: a new easing, the drag's start / end (a handle drag
/// stays open so the whole drag is one undo step), and whether the change was a discrete one (a tab
/// switch, the Reset button, or a typed value) that should commit on its own.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EaseChange {
    pub ease: Option<Ease>,
    pub started: bool,
    pub stopped: bool,
    pub discrete: bool,
}

/// Height of the graph box.
const GRAPH_H: f32 = 144.0;

/// Width of a value field on a label row (the settings window's control column).
const CONTROL_W: f32 = 124.0;

/// The easing graph: x is time, y is progress. Curves show the bezier's two control points; springs
/// show their response with a draggable peak. A handle drag stays open so the whole drag is one undo
/// step.
pub(crate) fn graph(ui: &mut Ui, rect: Rect, ease: Ease, salt: &str, t: &Tokens) -> EaseChange {
    ui.painter().rect_filled(rect, CornerRadius::same(6), t.field);
    if ease.is_spring() {
        spring_graph(ui, rect, salt, ease, t)
    } else {
        curve_graph(ui, rect, salt, ease, t)
    }
}

/// The combined editor: Curve | Spring tabs, the graph and the values row below it. Springs show
/// their single elasticity knob; curves show the
/// "x1, y1, x2, y2" text with a 24×24 Reset button. Returns the changed easing plus the drag's
/// start / stop so the caller groups one undo step around a drag.
pub(crate) fn editor(ui: &mut Ui, rect: Rect, salt: &str, label_x: f32, ease: Ease, t: &Tokens) -> EaseChange {
    let mut out = EaseChange::default();
    let x = rect.left();
    let w = rect.width();
    let spring = ease.is_spring();

    // Tabs.
    let tabs = Rect::from_min_size(pos2(x, rect.top()), vec2(w, 24.0));
    ui.painter().rect_filled(tabs, CornerRadius::same(6), t.field);
    for (i, label) in ["Curve", "Spring"].iter().enumerate() {
        let tr = Rect::from_min_size(
            pos2(tabs.left() + 2.0 + i as f32 * (w - 4.0) / 2.0, tabs.top() + 2.0),
            vec2((w - 4.0) / 2.0, 20.0),
        );
        let resp = ui.interact(tr, ui.scope_id().with(("ease_tab", salt, i)), Sense::click());
        let sel = (i == 1) == spring;
        if sel {
            ui.painter().rect_filled(tr, CornerRadius::same(5), t.panel);
        } else if resp.hovered() {
            ui.painter().rect_filled(tr, CornerRadius::same(5), t.hover);
        }
        ui.painter().text(tr.center(), Align2::CENTER_CENTER, *label, regular(11.0), if sel { t.text } else { t.text_tertiary });
        if resp.clicked() && !sel {
            out.ease = Some(default_ease(i == 1));
            out.discrete = true;
        }
    }
    let mut y = rect.top() + 32.0;

    // Graph.
    let g = Rect::from_min_size(pos2(x, y), vec2(w, GRAPH_H));
    let ch = graph(ui, g, ease, salt, t);
    out.ease = ch.ease.or(out.ease);
    out.started |= ch.started;
    out.stopped |= ch.stopped;
    y += GRAPH_H + 8.0;

    // Values.
    let vr = Rect::from_min_size(pos2(x, y), vec2(w, 24.0));
    if spring {
        // One knob: the elasticity every spring quantity is derived from. The
        // label sits on the row's left, the field on its right (the row of the settings window).
        ui.painter().text(pos2(vr.left() + label_x, vr.center().y), Align2::LEFT_CENTER, "Elasticity", regular(11.0), t.text_secondary);
        let fr = Rect::from_min_size(pos2(vr.right() - CONTROL_W, vr.top()), vec2(CONTROL_W, 24.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(fr).layout(Layout::left_to_right(egui::Align::Center)));
        let el = ease.elasticity().unwrap_or(DEFAULT_ELASTICITY);
        let algo = ease.spring_algorithm().unwrap_or_default();
        let mass = ease.mass().unwrap_or_else(unit_mass);
        if let Some(edit) = widgets::number_field_mixed(
            &mut child,
            ui.scope_id().with(("ease_elasticity", salt)),
            Lead::None,
            Some((el as f64 * 100.0).round() / 100.0),
            "",
            fr.width(),
            0.01,
        ) && let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, _)) = edit.settled()
        {
            out.ease = Some(Ease::spring_with(v as f32, algo, mass));
            out.discrete = true;
        }
        // The physics the mapping derives: ζ = clamp(1 − e, .03, .9999), ω₀ from `algo`'s settle
        // envelope, k = mass·ω₀², c = 2·mass·ζ·ω₀.
        let (z, w0) = (spring_damping_ratio(el), spring_angular_frequency(el, algo));
        let (k, c) = spring_stiffness_damping(el, mass, algo);
        ui.painter().text(
            pos2(vr.left() + label_x, vr.bottom() + 6.0),
            Align2::LEFT_TOP,
            format!("ζ {z:.3} · ω0 {w0:.1} · k {k:.0} · c {c:.1}"),
            regular(9.5),
            t.text_tertiary,
        );
        // Which settle envelope recovers the frequency: the loose bound, or the exact one.
        let cr = Rect::from_min_size(pos2(x + label_x, vr.bottom() + 24.0), vec2(w - label_x, 24.0));
        let mut check = ui.new_child(UiBuilder::new().max_rect(cr).layout(Layout::left_to_right(egui::Align::Center)));
        if widgets::checkbox(&mut check, algo == SpringAlgorithm::Tight, "Exact envelope") {
            let next = if algo == SpringAlgorithm::Tight { SpringAlgorithm::Envelope } else { SpringAlgorithm::Tight };
            out.ease = Some(Ease::spring_with(el, next, mass));
            out.discrete = true;
        }
        // The mass the stiffness and damping are expressed against.
        let mr = Rect::from_min_size(pos2(vr.left(), cr.bottom() + 8.0), vec2(w, 24.0));
        ui.painter().text(pos2(mr.left() + label_x, mr.center().y), Align2::LEFT_CENTER, "Mass", regular(11.0), t.text_secondary);
        let mfr = Rect::from_min_size(pos2(mr.right() - CONTROL_W, mr.top()), vec2(CONTROL_W, 24.0));
        let mut mchild = ui.new_child(UiBuilder::new().max_rect(mfr).layout(Layout::left_to_right(egui::Align::Center)));
        if let Some(edit) = widgets::number_field_mixed(
            &mut mchild,
            ui.scope_id().with(("ease_mass", salt)),
            Lead::None,
            Some((mass as f64 * 100.0).round() / 100.0),
            "",
            mfr.width(),
            0.1,
        ) && let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, _)) = edit.settled()
        {
            out.ease = Some(Ease::spring_with(el, algo, (v as f32).clamp(0.01, 100.0)));
            out.discrete = true;
        }
    } else if let Some(c) = ease.control_points() {
        let fr = Rect::from_min_size(vr.min, vec2(vr.width() - 28.0, 24.0));
        let txt = c.iter().map(|v| fmt_num(*v)).collect::<Vec<_>>().join(", ");
        if let Some(s) = widgets::text_field_inline(ui, ui.scope_id().with(("ease_values", salt)), &txt, fr, regular(11.0))
            && let Some(e) = parse_values(&s)
        {
            out.ease = Some(e);
            out.discrete = true;
        }
        // 24×24 Reset button, flush right of the values field.
        let rr = Rect::from_min_size(pos2(vr.right() - 24.0, vr.top()), vec2(24.0, 24.0));
        if crate::motion::block_button(ui, ui.scope_id().with(("ease_reset", salt)), rr, false, "Reset").clicked() {
            out.ease = Some(Ease::Linear);
            out.discrete = true;
        }
    }
    out
}

/// The vertical space the editor takes for `ease`: a curve shows the values row, a spring the
/// elasticity field with its physics readout, the envelope switch and the mass field, Hold
/// neither, so it is shorter.
pub(crate) fn editor_height(ease: Ease) -> f32 {
    let values = if ease.is_spring() {
        // 24 field + 24 readout + 24 switch + 8 gap + 24 mass + 8 slack before the separator
        112.0
    } else if ease.control_points().is_some() {
        24.0
    } else {
        0.0
    };
    24.0 + 8.0 + GRAPH_H + 8.0 + values
}

/// The default easing when switching to a tab: a spring tab starts at Gentle, a curve tab at Linear.
fn default_ease(spring: bool) -> Ease {
    if spring { Ease::SpringGentle } else { Ease::Linear }
}

/// The bezier graph: two control-point handles on a plot of time against progress. Dragging one
/// writes a custom bezier.
fn curve_graph(ui: &mut Ui, rect: Rect, salt: &str, ease: Ease, t: &Tokens) -> EaseChange {
    let mut out = EaseChange::default();
    // A square plot centred in the card, with room around it for handles that overshoot.
    let side = (rect.height() - 24.0).min(rect.width() - 24.0).max(24.0);
    let plot = Rect::from_center_size(rect.center(), vec2(side, side));
    let to_s = |x: f32, y: f32| pos2(plot.left() + x * plot.width(), plot.bottom() - y * plot.height());
    let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    // The 0 and 1 value guides.
    for v in [0.0, 1.0] {
        p.line_segment([to_s(0.0, v), to_s(1.0, v)], Stroke::new(1.0, t.border));
    }
    let c = ease.control_points().filter(|_| !matches!(ease, Ease::Hold));
    if let Some(c) = c {
        p.line_segment([to_s(0.0, 0.0), to_s(c[0], c[1])], Stroke::new(1.0, t.accent));
        p.line_segment([to_s(1.0, 1.0), to_s(c[2], c[3])], Stroke::new(1.0, t.accent));
    }
    let pts: Vec<egui::Pos2> = ease.sample(64).iter().map(|q| to_s(q[0], q[1])).collect();
    p.add(Shape::line(pts, Stroke::new(1.5, t.text)));
    p.text(rect.left_top() + vec2(8.0, 6.0), Align2::LEFT_TOP, ease.name(), regular(9.0), t.text_tertiary);
    let Some(c) = c else { return out };
    for i in 0..2 {
        let pos = to_s(c[2 * i], c[2 * i + 1]);
        let hr = Rect::from_center_size(pos, vec2(18.0, 18.0));
        let resp = ui.interact(hr, ui.scope_id().with(("ease_handle", salt, i)), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(if resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        out.started |= resp.drag_started();
        out.stopped |= resp.drag_stopped();
        if resp.dragged()
            && let Some(pt) = resp.interact_pointer_pos()
        {
            // x stays in 0..=1 (time must not run backwards); y may overshoot, as CSS allows.
            let x = (((pt.x - plot.left()) / plot.width()).clamp(0.0, 1.0) * 100.0).round() / 100.0;
            let y = (((plot.bottom() - pt.y) / plot.height()).clamp(-0.5, 1.5) * 100.0).round() / 100.0;
            let mut nc = c;
            nc[2 * i] = x;
            nc[2 * i + 1] = y;
            out.ease = Some(Ease::bezier(nc[0], nc[1], nc[2], nc[3]));
        }
        p.circle_filled(pos, 4.5, t.field);
        p.circle_stroke(pos, 4.0, Stroke::new(1.5, t.accent));
        if resp.hovered() || resp.dragged() {
            p.circle_filled(pos, 2.0, t.accent);
        }
    }
    out
}

/// The spring graph: the step response with a draggable peak that sets the elasticity.
fn spring_graph(ui: &mut Ui, rect: Rect, salt: &str, ease: Ease, t: &Tokens) -> EaseChange {
    let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let elasticity = ease.elasticity().unwrap_or(DEFAULT_ELASTICITY);
    let (vmin, vmax) = ease.value_range();
    let (lo, hi) = (vmin.min(0.0), vmax.max(1.0));
    let plot = Rect::from_min_max(
        pos2(rect.left() + 20.0, rect.top() + 28.0),
        pos2(rect.right() - 20.0, rect.bottom() - 28.0),
    );
    let to_s = |time: f32, v: f32| {
        pos2(plot.left() + time * plot.width(), plot.bottom() - (v - lo) / (hi - lo).max(1e-3) * plot.height())
    };
    for val in [0.0, 1.0] {
        p.line_segment([to_s(0.0, val), to_s(1.0, val)], Stroke::new(1.0, t.border));
    }
    let samples = ease.sample(96);
    p.add(Shape::line(samples.iter().map(|q| to_s(q[0], q[1])).collect(), Stroke::new(1.5, t.text)));
    // The handle sits on the overshoot peak (or the curve end when there is no overshoot).
    let (mut pt, mut pv) = (1.0f32, 1.0f32);
    for q in &samples {
        if q[1] > pv + 1e-4 {
            pt = q[0];
            pv = q[1];
        }
    }
    let pos = to_s(pt, pv);
    let hr = Rect::from_center_size(pos, vec2(16.0, 16.0));
    let hid = ui.scope_id().with(("ease_sh", salt));
    let resp = ui.interact(hr, hid, Sense::drag());
    let mut out = EaseChange::default();
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if resp.drag_started() {
        out.started = true;
        let y0 = resp.interact_pointer_pos().map_or(pos.y, |q| q.y);
        ui.ctx().data_mut(|d| d.insert_temp(hid, (elasticity, y0)));
    } else if resp.dragged()
        && let Some(q) = resp.interact_pointer_pos()
        && let Some((b0, y0)) = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(hid))
    {
        // The whole elasticity range across the plot's height, so the handle tracks the pointer.
        let gain = ELASTICITY_RANGE.1 / plot.height().max(1.0);
        let (algo, mass) = (ease.spring_algorithm().unwrap_or_default(), ease.mass().unwrap_or_else(unit_mass));
        out.ease = Some(Ease::spring_with(b0 + (y0 - q.y) * gain, algo, mass));
    }
    if resp.drag_stopped() {
        out.stopped = true;
    }
    p.circle_filled(pos, 4.5, t.field);
    p.circle_stroke(pos, 4.0, Stroke::new(1.5, t.accent));
    if resp.hovered() || resp.dragged() {
        p.circle_filled(pos, 2.0, t.accent);
    }
    out
}

fn fmt_num(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

fn parse_values(s: &str) -> Option<Ease> {
    let nums: Vec<f32> =
        s.split([',', ' ', ';']).filter(|t| !t.trim().is_empty()).map(|t| t.trim().parse::<f32>()).collect::<Result<_, _>>().ok()?;
    if nums.len() != 4 || nums.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some(Ease::bezier(nums[0], nums[1], nums[2], nums[3]))
}

/// Keeps `NodeId` visible to the module (used by tests elsewhere).
pub type Target = (NodeId, u32);
