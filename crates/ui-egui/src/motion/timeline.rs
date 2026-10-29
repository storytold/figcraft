//! The Motion timeline: a 34 pt header of controls, a 205 pt left column of layer / property /
//! clip rows, and the ruler and tracks beside it.

use egui::{Align2, CornerRadius, Key, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use figcraft_doc::anim::motion::{self, AnimProperty, TimelineSettings};
use figcraft_doc::{Document, NodeId};

use crate::FigApp;
use crate::a11y;
use crate::i18n::tr;
use crate::motion::{MotionUi, SelectedKey};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// The panel's height in points.
pub const HEIGHT: f32 = 309.0;
const HEADER_H: f32 = 47.0;
const ROW_H: f32 = 24.0;
const LEFT_W: f32 = 297.0;
const TIME_PAD: f32 = 9.5;
/// Row layout: 8 pt padding, a 16 pt fold arrow, 8 pt, a 16 pt kind icon, 8 pt, then the name.
const ROW_PAD: f32 = 8.0;
const ROW_ICON: f32 = 16.0;
const NAME_X: f32 = ROW_PAD + ROW_ICON + 8.0 + ROW_ICON + 8.0;
const KF_R: f32 = 5.0;

enum RowKind {
    Layer,
    Property(AnimProperty),
    Clip,
}

enum RowData {
    Layer { span: Option<(u32, u32)> },
    Property { keys: Vec<u32>, visible: bool },
    Clip { id: u32, start: u32, end: u32 },
}

struct Row {
    node: NodeId,
    kind: RowKind,
    label: String,
    data: RowData,
}

/// Which edge of a clip a resize drag moves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Start,
    End,
}

enum Act {
    Fold(NodeId),
    FoldAll,
    SelectLayer(NodeId),
    SelectKey(SelectedKey),
    AddKey(NodeId, AnimProperty, u32),
    MoveKey(SelectedKey, u32),
    SetLoop(bool),
    Play,
    AddKeyAtPlayhead,
    Seek(u32),
    ToggleKey(NodeId, AnimProperty),
    SetDuration(u32),
    ToggleVisible(NodeId),
    ToggleTrackVisible(NodeId, AnimProperty),
    MoveClipBy(NodeId, u32, i64),
    ResizeClip(NodeId, u32, Edge, i64),
    ShiftClips(NodeId, i64),
    SelectClip(NodeId, u32),
    SelectSegment(SelectedKey),
}

impl Act {
    /// Whether the act changes the document, so it needs an undo step.
    fn mutates(&self) -> bool {
        matches!(
            self,
            Act::AddKey(..)
                | Act::MoveKey(..)
                | Act::SetLoop(_)
                | Act::AddKeyAtPlayhead
                | Act::ToggleKey(..)
                | Act::SetDuration(_)
                | Act::ToggleVisible(_)
                | Act::ToggleTrackVisible(..)
                | Act::MoveClipBy(..)
                | Act::ResizeClip(..)
                | Act::ShiftClips(..)
        )
    }
}

fn set_duration(app: &mut FigApp, page: NodeId, ms: u32) {
    {
        let Ok(doc) = app.session.doc_mut() else { return };
        doc.timelines.entry(page).or_default().duration_ms = ms.max(100);
    }
    app.ui.motion.playhead_ms = app.ui.motion.playhead_ms.min(ms as f64);
}

fn value_at_prop(app: &FigApp, node: NodeId, prop: AnimProperty) -> Option<motion::AnimValue> {
    let t = app.ui.motion.playhead_ms;
    app.session.doc().ok().and_then(|d| d.node(node)).and_then(|n| motion::value_at(n, prop, t))
}

fn value_text(prop: AnimProperty, v: motion::AnimValue) -> String {
    match v {
        motion::AnimValue::Scalar(x) => format!("{:.2}{}", x * prop.display_factor(), prop.unit()),
        motion::AnimValue::Vec2(p) => format!("{:.1}, {:.1}", p.x, p.y),
        motion::AnimValue::Color(c) => {
            let [r, g, b, _] = c.to_rgba8();
            format!("#{r:02X}{g:02X}{b:02X}")
        }
    }
}

fn page_settings(app: &FigApp, page: NodeId) -> TimelineSettings {
    app.session.doc().ok().and_then(|d| d.timelines.get(&page).copied()).unwrap_or_default()
}

fn build_rows(app: &FigApp, page: NodeId) -> Vec<Row> {
    let Ok(doc) = app.session.doc() else { return Vec::new() };
    let dur = doc.timelines.get(&page).map(|s| s.duration_ms).unwrap_or(2000);
    // Animated nodes, plus the selected ones so a layer can be animated from the timeline.
    let nodes: Vec<NodeId> = app.session.selection().into_iter().filter(|n| doc.node(*n).is_some()).collect();
    let mut rows = Vec::new();
    for id in nodes {
        let Some(n) = doc.node(id) else { continue };
        rows.push(Row { node: id, kind: RowKind::Layer, label: n.name.clone(), data: RowData::Layer { span: n.motion.span_ms(dur) } });
        if app.ui.motion.folded.contains(&id) {
            continue;
        }
        // Clips first, then the keyframe tracks.
        for c in &n.motion.clips {
            rows.push(Row {
                node: id,
                kind: RowKind::Clip,
                label: format!("{} · {}", c.kind.name(), c.mode.label(c.kind)),
                data: RowData::Clip { id: c.id, start: c.delay_ms, end: c.end_ms(dur) },
            });
        }
        for tr in n.motion.tracks.iter().filter(|t| !t.is_empty()) {
            rows.push(Row {
                node: id,
                kind: RowKind::Property(tr.property),
                label: tr.property.name().to_string(),
                data: RowData::Property { keys: tr.keys.iter().map(|k| k.time_ms).collect(), visible: tr.visible },
            });
        }
    }
    rows
}

pub fn panel(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Ok(page) = app.session.page() else { return };
    let settings = page_settings(app, page);
    advance(app, ui, &settings);

    let full = ui.max_rect();
    app.ui.motion.scrubbing = false;
    a11y::container(ui, egui::accesskit::Role::Pane, tr("Animation"));
    ui.painter().rect_filled(full, CornerRadius::ZERO, t.panel);
    let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
    let body = Rect::from_min_max(pos2(full.min.x, header.bottom()), full.max);
    // Vertical scroll: clamp to the content and take the wheel over the rows.
    let content = build_rows(app, page).len() as f32 * ROW_H + 8.0;
    let max_scroll = (content - body.height()).max(0.0);
    if ui.rect_contains_pointer(body) {
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy != 0.0 {
            app.ui.motion.scroll_y -= dy;
        }
    }
    app.ui.motion.scroll_y = app.ui.motion.scroll_y.clamp(0.0, max_scroll);
    // Rows first, then the header and ruler, so scrolled rows never bleed into them.
    let mut acts = body_rows(app, ui, body, header.top(), &t, page, &settings);
    acts.extend(header_row(app, ui, header, &t, &settings));
    acts.extend(ruler(app, ui, header, &t, &settings));
    apply(app, ui, page, acts, full);
    // Border on the top edge, like the other panels.
    ui.painter().line_segment([full.left_top(), full.right_top()], Stroke::new(1.0, t.border));
}

fn advance(app: &mut FigApp, ui: &Ui, settings: &TimelineSettings) {
    if !app.ui.motion.playing {
        return;
    }
    let dt = ui.ctx().input(|i| i.stable_dt) as f64 * 1000.0;
    let dur = app.ui.motion.drag_duration.unwrap_or(settings.duration_ms).max(1) as f64;
    let mut ph = app.ui.motion.playhead_ms + dt;
    if ph >= dur {
        if settings.looping {
            ph %= dur;
        } else {
            ph = dur;
            app.ui.motion.playing = false;
        }
    }
    app.ui.motion.playhead_ms = ph;
    ui.ctx().request_repaint();
}

fn header_row(app: &mut FigApp, ui: &mut Ui, header: Rect, t: &Tokens, settings: &TimelineSettings) -> Vec<Act> {
    let mut acts = Vec::new();
    let cy = header.top() + HEADER_H / 2.0;
    let at = |cx: f32, w: f32, h: f32| Rect::from_center_size(pos2(header.left() + cx, cy), vec2(w, h));
    let playing = app.ui.motion.playing;

    // Play / pause.
    if header_button(app, ui, at(15.0, 22.0, 24.0), "play", false, if playing { tr("Pause") } else { tr("Play") }, t) {
        acts.push(Act::Play);
    }
    // Add a keyframe at the playhead for the selected layer's animated properties.
    if header_button(app, ui, at(38.0, 22.0, 24.0), "add_key", false, "Add keyframe", t) {
        acts.push(Act::AddKeyAtPlayhead);
    }
    // Current time and duration fields.
    let time = format!("{:.2}", app.ui.motion.playhead_ms / 1000.0);
    if let Some(v) = time_field(ui, at(68.0, 38.0, 20.0), "cur", &time, t) {
        acts.push(Act::Seek((v * 1000.0).max(0.0) as u32));
    }
    if let Some(v) = time_field(ui, at(108.0, 38.0, 20.0), "dur", &format!("{:.2}", settings.duration_ms as f64 / 1000.0), t) {
        acts.push(Act::SetDuration((v * 1000.0).clamp(100.0, 60_000.0) as u32));
    }
    // Time format (seconds).
    let fr = at(137.0, 20.0, 20.0);
    let fresp = ui.interact(fr, ui.scope_id().with("motion_format"), Sense::click());
    if fresp.hovered() {
        ui.painter().rect_filled(fr, CornerRadius::same(4), t.hover);
    }
    ui.painter().text(fr.center(), Align2::CENTER_CENTER, "s", regular(10.0), t.text_secondary);
    // Loop.
    if header_button(app, ui, at(160.0, 22.0, 24.0), "loop", settings.looping, tr("Loop"), t) {
        acts.push(Act::SetLoop(!settings.looping));
    }
    // Collapse / expand every layer.
    if header_button(app, ui, at(190.0, 22.0, 24.0), "fold", false, "Collapse layers", t) {
        acts.push(Act::FoldAll);
    }
    // Export (right side).
    let eb = Rect::from_center_size(pos2(header.right() - 46.0, cy), vec2(24.0, 24.0));
    header_button(app, ui, eb, "export", false, "Export animation", t);
    // Collapse the whole panel.
    let cb = Rect::from_center_size(pos2(header.right() - 18.0, cy), vec2(24.0, 24.0));
    if header_button(app, ui, cb, "collapse", false, "Close", t) {
        crate::motion::set_open(app, false);
        app.ui.motion.playing = false;
    }
    // Separator under the header and after the left column.
    ui.painter().line_segment([pos2(header.left(), header.bottom()), pos2(header.right(), header.bottom())], Stroke::new(1.0, t.border));
    ui.painter()
        .line_segment([pos2(header.left() + LEFT_W, header.top()), pos2(header.left() + LEFT_W, header.bottom())], Stroke::new(1.0, t.border));
    acts
}

#[allow(clippy::too_many_arguments)]
fn header_button(app: &mut FigApp, ui: &mut Ui, rect: Rect, key: &str, selected: bool, tip: &str, t: &Tokens) -> bool {
    let _ = app;
    let resp = ui.interact(rect, ui.scope_id().with(("motion_hbtn", key)), Sense::click()).on_hover_text(tip);
    a11y::button(&resp, tip);
    if selected || resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.hover);
    }
    ui.painter().rect_filled(
        Rect::from_center_size(rect.center(), vec2(14.0, 14.0)),
        CornerRadius::same(2),
        if selected { t.accent_text } else { t.icon_secondary },
    );
    resp.clicked()
}

/// A small editable time box (seconds). Returns the typed value on Enter or when it loses focus.
fn time_field(ui: &mut Ui, rect: Rect, key: &str, text: &str, t: &Tokens) -> Option<f64> {
    let id = ui.scope_id().with(("motion_time_field", key));
    let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| text.to_string());
    ui.painter().rect_filled(rect, CornerRadius::same(4), t.field);
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 1.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = child.add_sized(
        rect.size() - vec2(8.0, 2.0),
        egui::TextEdit::singleline(&mut buf).font(egui::FontId::proportional(10.0)).align(egui::Align2::CENTER_TOP).frame(egui::Frame::NONE),
    );
    let lost = resp.lost_focus();
    if !resp.has_focus() && !lost {
        buf = text.to_string();
    }
    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    let commit = lost || (resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
    commit.then(|| buf.trim().parse::<f64>().ok()).flatten()
}

/// Milliseconds per point, the same in the ruler and the tracks.
fn pxms(track_x: f32, right: f32, duration_ms: u32) -> f32 {
    ((right - track_x - TIME_PAD - 84.0).max(40.0)) / duration_ms.max(1) as f32
}

/// The time ruler: the top-right band beside the header controls (same height as the header).
fn ruler(app: &mut FigApp, ui: &mut Ui, header: Rect, t: &Tokens, settings: &TimelineSettings) -> Vec<Act> {
    let track_x = header.left() + LEFT_W;
    let ruler = Rect::from_min_max(pos2(track_x, header.top()), pos2(header.right(), header.bottom()));
    let dur = settings.duration_ms.max(1) as f64;
    let scale = pxms(track_x, header.right(), settings.duration_ms);
    let x_of = |time: f64| track_x + TIME_PAD + (time as f32) * scale;
    let step = tick_step(dur).max(1.0);
    let step_u = step as u32;
    let minor = if step_u.is_multiple_of(5) {
        step_u / 5
    } else if step_u.is_multiple_of(2) {
        step_u / 2
    } else {
        step_u
    };
    let base = ruler.bottom();
    let mut tick = 0u32;
    while (tick as f64) <= dur + step {
        let x = x_of(tick as f64);
        if x >= ruler.left() && x <= ruler.right() {
            let major = tick.is_multiple_of(step_u);
            let h = if major { 6.0 } else { 3.0 };
            ui.painter().line_segment([pos2(x, base - h), pos2(x, base)], Stroke::new(1.0, if major { t.text_tertiary } else { t.border }));
            if major {
                ui.painter().text(
                    pos2(x + 3.0, ruler.top() + 17.0),
                    Align2::LEFT_CENTER,
                    format!("{:.2}s", tick as f64 / 1000.0),
                    regular(10.0),
                    t.text_secondary,
                );
            }
        }
        tick = tick.saturating_add(minor.max(1));
    }
    let rresp = ui.interact(ruler, ui.scope_id().with("motion_ruler"), Sense::click_and_drag());
    if rresp.clicked() || rresp.dragged() {
        app.ui.motion.scrubbing = true;
    }
    if (rresp.clicked() || rresp.dragged())
        && let Some(p) = rresp.interact_pointer_pos()
    {
        let time = ((p.x - track_x - TIME_PAD) / scale) as f64;
        app.ui.motion.playhead_ms = settings.snap_to_frame(time.clamp(0.0, dur) as f32) as f64;
    }
    Vec::new()
}

#[allow(clippy::cognitive_complexity)]
fn body_rows(app: &mut FigApp, ui: &mut Ui, body: Rect, header_top: f32, t: &Tokens, page: NodeId, settings: &TimelineSettings) -> Vec<Act> {
    let mut acts = Vec::new();
    let track_x = body.min.x + LEFT_W;
    let track_right = body.max.x;
    let dur = settings.duration_ms.max(1) as f64;
    let scale = pxms(track_x, track_right, settings.duration_ms);
    let x_of = |time: f64| track_x + TIME_PAD + (time as f32) * scale;
    let time_of = |x: f32| (((x - track_x - TIME_PAD) / scale).max(0.0)) as f64;
    let snap = |time: f64| settings.snap_to_frame(time.clamp(0.0, dur) as f32) as f64;

    // Past the duration the track area is shaded; its edge drags to resize the timeline.
    // The visual edge follows the drag, but the grab zone stays where the drag started, and the
    // time->pixel scale never changes while dragging (that feedback made the handle run away).
    let shown = app.ui.motion.drag_duration.unwrap_or(settings.duration_ms);
    let end_x = x_of(shown as f64);
    if end_x < track_right {
        let outside = Rect::from_min_max(pos2(end_x.max(track_x), body.min.y), pos2(track_right, body.max.y));
        ui.painter().rect_filled(outside, CornerRadius::ZERO, t.border.gamma_multiply(0.35));
        ui.painter().line_segment([pos2(end_x, body.min.y), pos2(end_x, body.max.y)], Stroke::new(1.0, t.border_strong));
    }
    let grab_x = x_of(settings.duration_ms as f64);
    let handle = Rect::from_min_max(pos2(grab_x - 8.0, body.min.y), pos2(grab_x + 8.0, body.max.y));
    let hresp = ui.interact(handle, ui.scope_id().with("motion_duration_handle"), Sense::drag());
    if hresp.hovered() || hresp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if hresp.dragged()
        && let Some(p) = hresp.interact_pointer_pos()
    {
        app.ui.motion.drag_duration = Some(snap(time_of(p.x)).clamp(100.0, 60_000.0) as u32);
    }
    if hresp.drag_stopped()
        && let Some(ms) = app.ui.motion.drag_duration.take()
    {
        acts.push(Act::SetDuration(ms));
    }

    let rows = build_rows(app, page);
    let rows_top = body.min.y + 8.0 - app.ui.motion.scroll_y;
    for (i, row) in rows.iter().enumerate() {
        let y = rows_top + i as f32 * ROW_H;
        if y + ROW_H < body.min.y {
            continue;
        }
        if y > body.max.y {
            break;
        }
        let rect = Rect::from_min_max(pos2(body.min.x, y), pos2(track_right, y + ROW_H));
        let selected_layer = app.session.selection().contains(&row.node);
        if selected_layer {
            ui.painter().rect_filled(rect, CornerRadius::ZERO, t.selected_row);
        }
        let cy = rect.center().y;
        let eye = Rect::from_center_size(pos2(body.min.x + LEFT_W - 15.0, cy), vec2(16.0, 16.0));
        let name_x = body.min.x + NAME_X;

        match &row.data {
            RowData::Layer { span } => {
                // Fold chevron at +9, name at +39, eye at right-15.
                let c = pos2(body.min.x + ROW_PAD + ROW_ICON / 2.0, cy);
                let ar = Rect::from_center_size(c, vec2(ROW_ICON, ROW_ICON));
                ui.painter().rect_filled(Rect::from_center_size(c, vec2(10.0, 10.0)), CornerRadius::same(2), t.icon_secondary);
                if ui.interact(ar, ui.scope_id().with(("motion_fold", i)), Sense::click()).clicked() {
                    acts.push(Act::Fold(row.node));
                }
                // Row click covers the left column only, so the summary bar keeps its drags.
                let left_row = Rect::from_min_max(pos2(body.min.x, y), pos2(body.min.x + LEFT_W, y + ROW_H));
                let lresp = ui.interact(left_row, ui.scope_id().with(("motion_layer", i)), Sense::click());
                if lresp.clicked() {
                    acts.push(Act::SelectLayer(row.node));
                }
                // The kind icon's 16 pt slot (its artwork comes later).
                let name = widgets::elide(ui, &row.label, (LEFT_W - NAME_X - 30.0).max(20.0));
                ui.painter().text(pos2(name_x, cy), Align2::LEFT_CENTER, name, semibold(10.5), t.text);
                if let Some((a, b)) = span {
                    // A 24 pt row, the rounded bar 22 (one point in from the top and bottom).
                    let x0 = x_of(*a as f64);
                    let x1 = x_of(*b as f64).max(x0 + 3.0);
                    let r = Rect::from_min_max(pos2(x0, cy - 11.0), pos2(x1, cy + 11.0));
                    ui.painter().rect_filled(r, CornerRadius::same(4), t.accent.gamma_multiply(0.45));
                    // Dragging the summary shifts the whole animation in time.
                    let sresp = ui.interact(r, ui.scope_id().with(("motion_summary", i)), Sense::click_and_drag());
                    if sresp.hovered() || sresp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if sresp.dragged() {
                        app.ui.motion.scrubbing = true;
                        let d = (sresp.drag_delta().x / scale) as i64;
                        if d != 0 {
                            acts.push(Act::ShiftClips(row.node, d));
                        }
                    }
                }
                if ui.rect_contains_pointer(rect) {
                    ui.painter().rect_filled(Rect::from_center_size(eye.center(), vec2(12.0, 12.0)), CornerRadius::same(2), t.icon_secondary);
                    if ui.interact(eye, ui.scope_id().with(("motion_layer_eye", i)), Sense::click()).clicked() {
                        acts.push(Act::ToggleVisible(row.node));
                    }
                }
            }
            RowData::Property { keys, visible } => {
                let rresp = ui.interact(rect, ui.scope_id().with(("motion_prop", i)), Sense::click());
                let alpha = if *visible { 1.0 } else { 0.35 };
                let name = widgets::elide(ui, &row.label, 50.0);
                ui.painter().text(
                    pos2(name_x, cy),
                    Align2::LEFT_CENTER,
                    name,
                    regular(10.0),
                    if *visible { t.text_secondary } else { t.text_tertiary },
                );
                // Previous / current / next keyframe controls at +95 / +109 / +123.
                let pr = Rect::from_center_size(pos2(body.min.x + 95.0, cy), vec2(14.0, ROW_H));
                let dr = Rect::from_center_size(pos2(body.min.x + 109.0, cy), vec2(14.0, ROW_H));
                let nr = Rect::from_center_size(pos2(body.min.x + 123.0, cy), vec2(14.0, ROW_H));
                let prev = keys.iter().copied().filter(|k| *k < app.ui.motion.playhead_ms as u32).max();
                let next = keys.iter().copied().filter(|k| *k > app.ui.motion.playhead_ms as u32).min();
                let on_key = keys.contains(&(app.ui.motion.playhead_ms.max(0.0) as u32));
                if ui.interact(pr, ui.scope_id().with(("motion_prev", i)), Sense::click()).clicked()
                    && let Some(p) = prev
                {
                    acts.push(Act::Seek(p));
                }
                ui.painter().rect_filled(
                    Rect::from_center_size(pr.center(), vec2(10.0, 10.0)),
                    CornerRadius::same(2),
                    if prev.is_some() { t.icon_secondary } else { t.text_tertiary },
                );
                let dresp = ui.interact(dr, ui.scope_id().with(("motion_diamond", i)), Sense::click());
                let dcol = if on_key {
                    t.accent
                } else if dresp.hovered() {
                    t.icon
                } else {
                    t.icon_secondary
                };
                ui.painter().add(Shape::convex_polygon(
                    vec![dr.center() + vec2(0.0, -3.5), dr.center() + vec2(3.5, 0.0), dr.center() + vec2(0.0, 3.5), dr.center() + vec2(-3.5, 0.0)],
                    if on_key { t.accent } else { egui::Color32::TRANSPARENT },
                    Stroke::new(1.0, dcol),
                ));
                if dresp.clicked() {
                    acts.push(Act::ToggleKey(row.node, prop_of(row)));
                }
                if ui.interact(nr, ui.scope_id().with(("motion_next", i)), Sense::click()).clicked()
                    && let Some(p) = next
                {
                    acts.push(Act::Seek(p));
                }
                ui.painter().rect_filled(
                    Rect::from_center_size(nr.center(), vec2(10.0, 10.0)),
                    CornerRadius::same(2),
                    if next.is_some() { t.icon_secondary } else { t.text_tertiary },
                );
                if let Some(v) = value_at_prop(app, row.node, prop_of(row)) {
                    let s = widgets::elide(ui, &value_text(prop_of(row), v), (LEFT_W - 137.0 - 20.0).max(20.0));
                    ui.painter().text(pos2(body.min.x + 137.0, cy), Align2::LEFT_CENTER, s, regular(10.0), t.text_secondary);
                }
                if ui.rect_contains_pointer(rect) {
                    ui.painter().rect_filled(Rect::from_center_size(eye.center(), vec2(12.0, 12.0)), CornerRadius::same(2), t.icon_secondary);
                    if ui.interact(eye, ui.scope_id().with(("motion_track_eye", i)), Sense::click()).clicked() {
                        acts.push(Act::ToggleTrackVisible(row.node, prop_of(row)));
                    }
                }

                // The track: a line and its keyframes.
                ui.painter().line_segment([pos2(track_x + TIME_PAD, cy), pos2(track_right, cy)], Stroke::new(1.0, t.border.gamma_multiply(alpha)));
                // Clicking the line between two keys opens the easing panel for that segment.
                for (ki, w) in keys.windows(2).enumerate() {
                    let (xa, xb) = (x_of(w[0] as f64), x_of(w[1] as f64));
                    let seg = Rect::from_min_max(pos2(xa, cy - 5.0), pos2(xb, cy + 5.0));
                    let sresp = ui.interact(seg, ui.scope_id().with(("motion_seg", i, ki)), Sense::click());
                    if sresp.clicked() {
                        acts.push(Act::SelectSegment(SelectedKey { node: row.node, property: prop_of(row), time_ms: w[0] }));
                    }
                }
                for (ki, &time) in keys.iter().enumerate() {
                    let c = pos2(x_of(time as f64), cy);
                    let hit = Rect::from_center_size(c, vec2(KF_R * 2.0 + 6.0, KF_R * 2.0 + 6.0));
                    // Keyed by index, not time: an id that changes mid-drag drops the drag.
                    let kresp = ui.interact(hit, ui.scope_id().with(("motion_kf", i, ki)), Sense::click_and_drag());
                    let selected = app.ui.motion.selected == Some(SelectedKey { node: row.node, property: prop_of(row), time_ms: time });
                    let fill = if selected || kresp.hovered() || kresp.dragged() { t.accent_hover } else { t.accent };
                    ui.painter().add(Shape::convex_polygon(
                        vec![c + vec2(0.0, -KF_R), c + vec2(KF_R, 0.0), c + vec2(0.0, KF_R), c + vec2(-KF_R, 0.0)],
                        fill.gamma_multiply(alpha),
                        Stroke::NONE,
                    ));
                    if kresp.clicked() {
                        acts.push(Act::SelectKey(SelectedKey { node: row.node, property: prop_of(row), time_ms: time }));
                    }
                    if kresp.dragged() {
                        // Dragging a keyframe keeps the canvas previewing.
                        app.ui.motion.scrubbing = true;
                    }
                    if kresp.dragged()
                        && let Some(p) = kresp.interact_pointer_pos()
                    {
                        acts.push(Act::MoveKey(SelectedKey { node: row.node, property: prop_of(row), time_ms: time }, snap(time_of(p.x)) as u32));
                    }
                }
                if rresp.double_clicked()
                    && let Some(p) = rresp.interact_pointer_pos()
                {
                    acts.push(Act::AddKey(row.node, prop_of(row), snap(time_of(p.x)) as u32));
                }
            }
            RowData::Clip { id, start, end } => {
                let name = widgets::elide(ui, &row.label, (LEFT_W - NAME_X - 30.0).max(20.0));
                ui.painter().text(pos2(name_x, cy), Align2::LEFT_CENTER, name, regular(10.0), t.text_secondary);
                if ui.rect_contains_pointer(rect) {
                    ui.painter().rect_filled(Rect::from_center_size(eye.center(), vec2(12.0, 12.0)), CornerRadius::same(2), t.icon_secondary);
                }
                let a = x_of(*start as f64);
                let b = x_of(*end as f64).max(a + 6.0);
                let r = Rect::from_min_max(pos2(a, cy - 11.0), pos2(b, cy + 11.0));
                ui.painter().rect_filled(r, CornerRadius::same(4), t.accent.gamma_multiply(0.36));
                ui.painter().rect_stroke(r, CornerRadius::same(4), Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                let kind = row.label.split(" · ").next().unwrap_or(&row.label);
                let label = widgets::elide(ui, kind, (r.width() - 14.0).max(0.0));
                ui.painter().text(r.center(), Align2::CENTER_CENTER, label, regular(9.0), t.accent_on);
                // Dragging the bar moves the clip in time, live.
                // Edge handles resize the clip, the middle moves it.
                let lh = Rect::from_min_max(pos2(a - 4.0, cy - 11.0), pos2(a + 4.0, cy + 11.0));
                let rh = Rect::from_min_max(pos2(b - 4.0, cy - 11.0), pos2(b + 4.0, cy + 11.0));
                for (hr, edge) in [(lh, Edge::Start), (rh, Edge::End)] {
                    let hres = ui.interact(hr, ui.scope_id().with(("motion_clip_edge", i, edge == Edge::Start)), Sense::click_and_drag());
                    if hres.hovered() || hres.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if hres.dragged() {
                        app.ui.motion.scrubbing = true;
                        let d = (hres.drag_delta().x / scale) as i64;
                        if d != 0 {
                            acts.push(Act::ResizeClip(row.node, *id, edge, d));
                        }
                    }
                }
                let bresp = ui.interact(r, ui.scope_id().with(("motion_clip", i)), Sense::click_and_drag());
                if bresp.clicked() {
                    acts.push(Act::SelectClip(row.node, *id));
                }
                if bresp.dragged() {
                    app.ui.motion.scrubbing = true;
                    let delta = (bresp.drag_delta().x / scale) as i64;
                    if delta != 0 {
                        acts.push(Act::MoveClipBy(row.node, *id, delta));
                    }
                }
            }
        }
    }

    // Vertical divider after the left column, and the playhead over everything.
    ui.painter().line_segment([pos2(track_x, body.min.y), pos2(track_x, body.max.y)], Stroke::new(1.0, t.border));
    let px = x_of(app.ui.motion.playhead_ms);
    let y0 = header_top + 6.0;
    ui.painter().line_segment([pos2(px, y0), pos2(px, body.max.y)], Stroke::new(1.0, t.accent));
    ui.painter().circle_filled(pos2(px, y0 + 1.0), 5.5, t.accent);
    acts
}

fn prop_of(row: &Row) -> AnimProperty {
    match row.kind {
        RowKind::Property(p) => p,
        _ => AnimProperty::Opacity,
    }
}

fn settings_fps(app: &FigApp, page: NodeId) -> u32 {
    app.session.doc().ok().and_then(|d| d.timelines.get(&page).copied()).map(|s| s.fps).unwrap_or(60)
}

fn settings_dur(app: &FigApp, page: NodeId) -> u32 {
    app.session.doc().ok().and_then(|d| d.timelines.get(&page).copied()).map(|s| s.duration_ms).unwrap_or(2000)
}

fn acts2_play(app: &mut FigApp) {
    app.ui.motion.playing = !app.ui.motion.playing;
}

fn apply(app: &mut FigApp, ui: &Ui, page: NodeId, acts: Vec<Act>, area: Rect) {
    // A gesture is one undo step: opened on its first change and closed when the pointer is up.
    if acts.iter().any(Act::mutates) && !app.ui.motion.gesture_open && app.session.begin_interaction("Motion").is_ok() {
        app.ui.motion.gesture_open = true;
    }
    for act in acts {
        match act {
            Act::Fold(id) => {
                if !app.ui.motion.folded.remove(&id) {
                    app.ui.motion.folded.insert(id);
                }
            }
            Act::FoldAll => {
                if app.ui.motion.folded.is_empty() {
                    if let Ok(doc) = app.session.doc() {
                        for id in motion::animated_nodes(doc, page) {
                            app.ui.motion.folded.insert(id);
                        }
                    }
                } else {
                    app.ui.motion.folded.clear();
                }
            }
            Act::SelectLayer(id) => app.session.set_selection(vec![id]),
            Act::SelectKey(k) => app.ui.motion.selected = Some(k),
            Act::Play => {
                if app.ui.motion.playing {
                    app.ui.motion.playing = false;
                } else {
                    // Pressing play on the last frame restarts from the top instead of twitching.
                    let settings = app.session.doc().ok().and_then(|d| d.timelines.get(&page).copied()).unwrap_or_default();
                    if app.ui.motion.playhead_ms >= settings.duration_ms as f64 && !settings.looping {
                        app.ui.motion.playhead_ms = 0.0;
                    }
                    app.ui.motion.playing = true;
                }
            }
            Act::AddKeyAtPlayhead => {
                if let Some(id) = app.session.selection().first().copied() {
                    add_key_for_layer(app, id, app.ui.motion.playhead_ms.max(0.0) as u32);
                }
            }
            Act::AddKey(node, prop, time) => add_key(app, node, prop, time),
            Act::MoveKey(k, time) => {
                move_key(app, k, time);
                app.ui.motion.selected = Some(SelectedKey { time_ms: time, ..k });
            }
            Act::SetLoop(on) => set_loop(app, page, on),
            Act::Seek(t) => app.ui.motion.playhead_ms = t as f64,
            Act::SetDuration(ms) => set_duration(app, page, ms),
            Act::ToggleVisible(n) => {
                let Ok(doc) = app.session.doc_mut() else { return };
                if let Some(nd) = doc.node_mut(n) {
                    nd.visible = !nd.visible;
                }
                doc.mark_dirty([n]);
            }
            Act::SelectClip(n, cid) => {
                app.ui.motion.selected_clip = Some((n, cid));
                app.ui.motion.easing_target = false;
            }
            Act::SelectSegment(k) => {
                app.ui.motion.selected = Some(k);
                app.ui.motion.easing_target = true;
            }
            Act::ShiftClips(n, delta) => {
                let Ok(doc) = app.session.doc_mut() else { return };
                if let Some(nd) = doc.node_mut(n) {
                    for c in &mut nd.motion.clips {
                        c.delay_ms = (c.delay_ms as i64 + delta).max(0) as u32;
                    }
                    for tr in &mut nd.motion.tracks {
                        for k in &mut tr.keys {
                            k.time_ms = (k.time_ms as i64 + delta).max(0) as u32;
                        }
                    }
                }
                doc.mark_dirty([n]);
            }
            Act::ResizeClip(n, cid, edge, delta) => {
                let Ok(doc) = app.session.doc_mut() else { return };
                if let Some(c) = doc.node_mut(n).and_then(|nd| nd.motion.clips.iter_mut().find(|c| c.id == cid)) {
                    resize_clip(c, edge, delta);
                }
                doc.mark_dirty([n]);
            }
            Act::MoveClipBy(n, cid, delta) => {
                let Ok(doc) = app.session.doc_mut() else { return };
                if let Some(c) = doc.node_mut(n).and_then(|nd| nd.motion.clips.iter_mut().find(|c| c.id == cid)) {
                    c.delay_ms = (c.delay_ms as i64 + delta).max(0) as u32;
                }
                doc.mark_dirty([n]);
            }
            Act::ToggleTrackVisible(n, prop) => {
                let Ok(doc) = app.session.doc_mut() else { return };
                if let Some(tr) = doc.node_mut(n).and_then(|nd| nd.motion.track_mut(prop)) {
                    tr.visible = !tr.visible;
                }
                doc.mark_dirty([n]);
            }
            Act::ToggleKey(node, prop) => {
                let time = app.ui.motion.playhead_ms.max(0.0) as u32;
                let exists =
                    app.session.doc().ok().and_then(|d| d.node(node)).and_then(|n| n.motion.track(prop)).is_some_and(|tr| tr.key_at(time).is_some());
                if exists {
                    delete_key(app, SelectedKey { node, property: prop, time_ms: time });
                    app.ui.motion.selected = None;
                } else {
                    add_key(app, node, prop, time);
                }
            }
        }
    }
    // Shortcuts, only when no text field has the keyboard: Space plays, , / . step one frame.
    if ui.memory(|m| m.focused()).is_none() {
        let step = 1000.0 / settings_fps(app, page).max(1) as f64;
        // Only while the pointer is over the timeline: Space also pans the canvas when held.
        if ui.rect_contains_pointer(area) && ui.input(|i| i.key_pressed(Key::Space)) {
            acts2_play(app);
        }
        if ui.input(|i| i.key_pressed(Key::Comma)) {
            app.ui.motion.playhead_ms = (app.ui.motion.playhead_ms - step).max(0.0);
        }
        if ui.input(|i| i.key_pressed(Key::Period)) {
            app.ui.motion.playhead_ms += step;
        }
        if ui.input(|i| i.key_pressed(Key::Home)) {
            app.ui.motion.playhead_ms = 0.0;
        }
        if ui.input(|i| i.key_pressed(Key::End)) {
            app.ui.motion.playhead_ms = settings_dur(app, page) as f64;
        }
    }
    if let Some(k) = app.ui.motion.selected
        && (ui.input(|i| i.key_pressed(Key::Delete)) || ui.input(|i| i.key_pressed(Key::Backspace)))
    {
        // Swallow the key so the global shortcut does not delete the whole layer.
        ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, Key::Delete);
            i.consume_key(egui::Modifiers::NONE, Key::Backspace);
        });
        if !app.ui.motion.gesture_open && app.session.begin_interaction("Motion").is_ok() {
            app.ui.motion.gesture_open = true;
        }
        delete_key(app, k);
        app.ui.motion.selected = None;
    }
    if app.ui.motion.gesture_open && !ui.input(|i| i.pointer.any_down()) {
        let _ = app.session.commit_interaction();
        app.ui.motion.gesture_open = false;
    }
}

/// Adds a keyframe for every property the layer already animates (the header's "+" button).
fn add_key_for_layer(app: &mut FigApp, node: NodeId, time_ms: u32) {
    let Ok(doc) = app.session.doc_mut() else { return };
    let props: Vec<AnimProperty> = doc.node(node).map(|n| n.motion.tracks.iter().map(|t| t.property).collect()).unwrap_or_default();
    if props.is_empty() {
        return;
    }
    let mut values = Vec::new();
    for p in props {
        if let Some(v) = doc.node(node).and_then(|n| motion::keyframed_value_at(n, p, time_ms as f64)) {
            values.push((p, v));
        }
    }
    if let Some(n) = doc.node_mut(node) {
        for (p, v) in values {
            n.motion.ensure_track(p).set_key(time_ms, v, None);
        }
    }
    doc.mark_dirty([node]);
}

fn add_key(app: &mut FigApp, node: NodeId, prop: AnimProperty, time_ms: u32) {
    let Ok(doc) = app.session.doc_mut() else { return };
    let value = doc.node(node).and_then(|n| motion::keyframed_value_at(n, prop, time_ms as f64));
    let Some(value) = value else { return };
    if let Some(n) = doc.node_mut(node) {
        n.motion.ensure_track(prop).set_key(time_ms, value, None);
    }
    doc.mark_dirty([node]);
    app.ui.motion.selected = Some(SelectedKey { node, property: prop, time_ms });
}

fn move_key(app: &mut FigApp, k: SelectedKey, time_ms: u32) {
    let Ok(doc) = app.session.doc_mut() else { return };
    let Some(n) = doc.node_mut(k.node) else { return };
    let Some(tr) = n.motion.track_mut(k.property) else { return };
    let lo = tr.prev_key_time(k.time_ms).map_or(0, |p| p.saturating_add(1));
    let hi = tr.next_key_time(k.time_ms).map_or(u32::MAX, |p| p.saturating_sub(1));
    let time = time_ms.clamp(lo, hi);
    if let Some(key) = tr.key_at(k.time_ms).copied()
        && let Some(idx) = tr.key_index(k.time_ms)
    {
        tr.keys[idx] = figcraft_doc::anim::motion::Keyframe { time_ms: time, ..key };
    }
    doc.mark_dirty([k.node]);
}

fn delete_key(app: &mut FigApp, k: SelectedKey) {
    let Ok(doc) = app.session.doc_mut() else { return };
    if let Some(n) = doc.node_mut(k.node) {
        if let Some(tr) = n.motion.track_mut(k.property) {
            tr.remove_key(k.time_ms);
        }
        n.motion.normalize();
    }
    doc.mark_dirty([k.node]);
}

fn set_loop(app: &mut FigApp, page: NodeId, on: bool) {
    let Ok(doc) = app.session.doc_mut() else { return };
    doc.timelines.entry(page).or_default().looping = on;
}

/// Resizes a clip by dragging one edge `delta` ms to the right (negative moves it left).
fn resize_clip(c: &mut motion::Clip, edge: Edge, delta: i64) {
    match edge {
        // The end stays put: moving the start changes both the delay and the length.
        Edge::Start => {
            let end = c.delay_ms as i64 + c.duration_ms as i64;
            let start = (c.delay_ms as i64 + delta).clamp(0, end - 1);
            c.delay_ms = start as u32;
            c.duration_ms = (end - start) as u32;
        }
        // The start stays put: only the length changes.
        Edge::End => c.duration_ms = (c.duration_ms as i64 + delta).max(1) as u32,
    }
}

/// A ruler step that gives about six ticks over `duration` ms.
fn tick_step(duration_ms: f64) -> f64 {
    let raw = (duration_ms / 6.0).max(1.0);
    let p = 10f64.powf(raw.log10().floor());
    let n = raw / p;
    let m = if n <= 1.0 {
        1.0
    } else if n <= 2.0 {
        2.0
    } else if n <= 5.0 {
        5.0
    } else {
        10.0
    };
    (m * p).max(10.0)
}

/// The timeline of `doc`'s page, for the inspector and tests.
pub fn settings_of(doc: &Document, page: NodeId) -> TimelineSettings {
    doc.timelines.get(&page).copied().unwrap_or_default()
}

/// Keeps the shared state type visible to the module tree.
pub type State = MotionUi;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_step_gives_about_six_ticks() {
        for dur in [500.0, 2000.0, 10_000.0, 60_000.0] {
            let ticks = dur / tick_step(dur);
            assert!((2.0..=12.0).contains(&ticks), "{dur}: {ticks} ticks");
        }
    }

    #[test]
    fn settings_default_to_two_seconds() {
        let d = Document::new("t");
        let page = d.first_page().unwrap();
        let s = settings_of(&d, page);
        assert_eq!((s.duration_ms, s.fps, s.looping), (2000, 60, false));
    }

    fn clip(delay_ms: u32, duration_ms: u32) -> motion::Clip {
        let mut c = motion::Clip::new(motion::ClipKind::Opacity, motion::ClipMode::In);
        c.delay_ms = delay_ms;
        c.duration_ms = duration_ms;
        c
    }

    #[test]
    fn dragging_the_start_edge_moves_the_start_and_keeps_the_end() {
        let mut c = clip(100, 200);
        resize_clip(&mut c, Edge::Start, 50);
        assert_eq!((c.delay_ms, c.duration_ms), (150, 150), "start moves right, end stays at 300");
        resize_clip(&mut c, Edge::Start, -80);
        assert_eq!((c.delay_ms, c.duration_ms), (70, 230), "start moves left, end stays at 300");
        resize_clip(&mut c, Edge::Start, -1_000);
        assert_eq!((c.delay_ms, c.duration_ms), (0, 300), "the start stops at zero");
    }

    #[test]
    fn dragging_the_end_edge_changes_the_length_only() {
        let mut c = clip(100, 200);
        resize_clip(&mut c, Edge::End, 150);
        assert_eq!((c.delay_ms, c.duration_ms), (100, 350), "longer, start unchanged");
        resize_clip(&mut c, Edge::End, -300);
        assert_eq!((c.delay_ms, c.duration_ms), (100, 50), "shorter, start unchanged");
        resize_clip(&mut c, Edge::End, -1_000);
        assert_eq!((c.delay_ms, c.duration_ms), (100, 1), "one millisecond is the floor");
    }
}
