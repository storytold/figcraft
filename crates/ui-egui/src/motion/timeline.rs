//! The Motion timeline: a 34 pt header of controls, a 205 pt left column of
//! layer / property / clip rows, and the ruler and tracks beside it.

use egui::{Align2, CornerRadius, Key, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use figcraft_doc::anim::motion::{self, AnimProperty, Ease, TimelineSettings};
use figcraft_doc::{Document, NodeId};

use std::collections::HashSet;

use crate::FigApp;
use crate::a11y;
use crate::i18n::tr;
use crate::motion::{ClipDrag, ClipEdge, LayerDrag, MotionUi, SelItem, SelectedKey, SelectedTrack};
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// The panel's height in points.
pub const HEIGHT: f32 = 309.0;

/// The docked height: whatever the top edge was dragged to, or [`HEIGHT`] until it is.
pub fn height(m: &crate::motion::MotionUi) -> f32 {
    if m.timeline_h > 0.0 { m.timeline_h } else { HEIGHT }
}

/// Smallest height the top edge can be dragged to.
pub const MIN_H: f32 = 160.0;
const HEADER_H: f32 = 47.0;
const ROW_H: f32 = 24.0;
const LEFT_W: f32 = 297.0;
const TIME_PAD: f32 = 9.5;
/// Row layout: 8 pt padding, a 16 pt fold arrow, 8 pt, a 16 pt kind icon, 8 pt, then the name.
const ROW_PAD: f32 = 8.0;
const ROW_ICON: f32 = 16.0;
const NAME_X: f32 = ROW_PAD + ROW_ICON + 8.0 + ROW_ICON + 8.0;
const KF_R: f32 = 5.0;
/// How far inside a clip's end a press trims that end instead of moving the clip.
const EDGE_HIT: f32 = 8.0;
/// The fastest the track scrolls while the duration edge is held in the scroll zone (px/s); the
/// zone ramps 0→1 across its last 48 pt.
const SCROLL_SPEED: f32 = 1200.0;

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

enum Act {
    Fold(NodeId),
    FoldAll,
    /// A click on a timeline item. The bool is the Shift modifier: with it the item is added to (or
    /// removed from) the selection; without it a click replaces the selection (and a layer expands to
    /// all its children), while re-clicking an already-selected item toggles just it off.
    Click(SelItem, bool),
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
    /// Recompute a clip from the state it had when the drag began plus the pointer's total delta.
    ClipDrag { node: NodeId, id: u32, edge: ClipEdge, orig: motion::Clip, dx_ms: i64 },
    /// Recompute a layer's whole motion (move or stretch) from its state at the drag start.
    LayerDrag { node: NodeId, edge: ClipEdge, lo: u32, hi: u32, orig: motion::NodeMotion, dx_ms: i64 },
    /// A drag began: later edits go into this one undo step.
    BeginEdit(&'static str),
    /// The drag ended: commit the step (nothing changed records nothing).
    EndEdit,
    /// Clear every timeline selection (keyframe, track, clip) — a click on empty track lane.
    ClearSelection,
}

/// Writes one page's timeline settings. A page that lands back on the defaults leaves the map, so
/// a file that was never retimed keeps no entry.
fn set_timeline(app: &mut FigApp, page: NodeId, edit: impl FnOnce(&mut TimelineSettings)) {
    let Ok(doc) = app.session.doc_mut() else { return };
    let mut settings = doc.timelines.get(&page).copied().unwrap_or_default();
    edit(&mut settings);
    let settings = settings.sanitized();
    if settings.is_default() {
        doc.timelines.remove(&page);
    } else {
        doc.timelines.insert(page, settings);
    }
}

fn set_duration(app: &mut FigApp, page: NodeId, ms: u32) {
    let ms = ms.max(100);
    set_timeline(app, page, |s| s.duration_ms = ms);
    app.ui.motion.playhead_ms = app.ui.motion.playhead_ms.min(ms as f64);
}

/// Run `f` as one undo step named `label`; an edit that changes nothing records no step.
fn edit(app: &mut FigApp, label: &str, f: impl FnOnce(&mut FigApp)) {
    if app.session.begin_interaction(label).is_err() {
        return;
    }
    f(app);
    let _ = app.session.commit_interaction();
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
        rows.push(Row {
            node: id,
            kind: RowKind::Layer,
            label: n.name.clone(),
            data: RowData::Layer { span: n.motion.span_ms(dur) },
        });
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
    apply(app, ui, page, full, acts);
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
    ui.painter().line_segment([pos2(header.left() + LEFT_W, header.top()), pos2(header.left() + LEFT_W, header.bottom())], Stroke::new(1.0, t.border));
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
    ui.painter().rect_filled(Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), CornerRadius::same(2), if selected { t.accent_text } else { t.icon_secondary });
    resp.clicked()
}

/// A small editable time box (seconds). Returns the typed value on Enter or when it loses focus.
fn time_field(ui: &mut Ui, rect: Rect, key: &str, text: &str, t: &Tokens) -> Option<f64> {
    let id = ui.scope_id().with(("motion_time_field", key));
    let mut buf = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| text.to_string());
    ui.painter().rect_filled(rect, CornerRadius::same(4), t.field);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 1.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = child.add_sized(
        rect.size() - vec2(8.0, 2.0),
        egui::TextEdit::singleline(&mut buf).font(egui::FontId::proportional(10.0)).align(egui::Align2::CENTER_TOP)
            .frame(egui::Frame::NONE),
    );
    let lost = resp.lost_focus();
    if !resp.has_focus() && !lost {
        buf = text.to_string();
    }
    ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    let commit = lost || (resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
    commit.then(|| buf.trim().parse::<f64>().ok()).flatten()
}

/// Milliseconds per point, the same in the ruler and the tracks. The small right pad leaves just
/// enough room for the duration handle's grab zone; the content reaches the lane's right edge so the
/// keyframes and playhead line up with the track background instead of stopping short of it.
fn pxms(track_x: f32, right: f32, duration_ms: u32) -> f32 {
    ((right - track_x - TIME_PAD - 16.0).max(40.0)) / duration_ms.max(1) as f32
}

/// The time ruler: the top-right band beside the header controls (same height as the header).
fn ruler(app: &mut FigApp, ui: &mut Ui, header: Rect, t: &Tokens, settings: &TimelineSettings) -> Vec<Act> {
    let track_x = header.left() + LEFT_W;
    let ruler = Rect::from_min_max(pos2(track_x, header.top()), pos2(header.right(), header.bottom()));
    let dur = settings.duration_ms.max(1) as f64;
    let scale = pxms(track_x, header.right(), settings.duration_ms);
    let scroll = app.ui.motion.scroll_x;
    let x_of = |time: f64| track_x + TIME_PAD + (time as f32) * scale - scroll;
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
    let end = dur + step + (scroll.max(0.0) / scale) as f64;
    while (tick as f64) <= end {
        let x = x_of(tick as f64);
        if x >= ruler.left() && x <= ruler.right() {
            let major = tick.is_multiple_of(step_u);
            let h = if major { 6.0 } else { 3.0 };
            ui.painter().line_segment([pos2(x, base - h), pos2(x, base)], Stroke::new(1.0, if major { t.text_tertiary } else { t.border }));
            if major {
                ui.painter().text(pos2(x + 3.0, ruler.top() + 17.0), Align2::LEFT_CENTER, format!("{:.2}s", tick as f64 / 1000.0), regular(10.0), t.text_secondary);
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
        let time = ((p.x - track_x - TIME_PAD + scroll) / scale) as f64;
        app.ui.motion.playhead_ms = settings.snap_to_frame(time.clamp(0.0, dur) as f32) as f64;
    }
    Vec::new()
}

#[allow(clippy::cognitive_complexity)]
fn body_rows(app: &mut FigApp, ui: &mut Ui, body: Rect, header_top: f32, t: &Tokens, page: NodeId, settings: &TimelineSettings) -> Vec<Act> {
    let mut acts = Vec::new();
    // The Shift modifier extends the selection instead of replacing it (rules: single unless multi).
    let shift = ui.input(|i| i.modifiers.shift);
    let track_x = body.min.x + LEFT_W;
    let track_right = body.max.x;
    // A press on the empty track lane (no keyframe or segment under it) clears the selection.
    // Registered before the rows so every row, keyframe and segment stays on top and keeps its own
    // press — this only fires where nothing else is.
    let lane = Rect::from_min_max(pos2(track_x, body.min.y), pos2(track_right, body.max.y));
    if ui.interact(lane, ui.scope_id().with("motion_lane"), Sense::click()).clicked() {
        acts.push(Act::ClearSelection);
    }
    // The scale stays fixed while the duration edge is dragged; only the right edge scrolls when
    // the pointer passes it. Rescaling every frame made both directions "live" and wrong.
    let dur = settings.duration_ms.max(1) as f64;
    let scale = pxms(track_x, track_right, settings.duration_ms);
    let snap = |time: f64| settings.snap_to_frame(time.clamp(0.0, dur) as f32) as f64;
    // The duration edge sets a new length, so it may pass the current one (up to the model's cap):
    // clamping to `dur` here is what stopped the drag dead at the current length.
    let snap_dur = |time: f64| settings.snap_to_frame(time.clamp(0.0, TimelineSettings::MAX_DURATION_MS as f64) as f32) as f64;

    // The handle's grab zone sits at the committed duration's x (before any scroll this frame).
    let grab_x = track_x + TIME_PAD + settings.duration_ms as f32 * scale - app.ui.motion.scroll_x;
    let handle = Rect::from_min_max(pos2(grab_x - 8.0, body.min.y), pos2(grab_x + 8.0, body.max.y));
    let hresp = ui.interact(handle, ui.scope_id().with("motion_duration_handle"), Sense::drag());
    if hresp.hovered() || hresp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if hresp.drag_started() {
        app.ui.motion.scroll_x = 0.0;
        app.ui.motion.drag_duration = Some(settings.duration_ms);
        acts.push(Act::BeginEdit("Animation duration"));
    }
    if hresp.dragged()
        && let Some(p) = hresp.interact_pointer_pos()
    {
        // The last 48 pt of the track is the scroll zone: the speed ramps 0→1 across it, and the
        // track keeps scrolling while the pointer is held there — `request_repaint` is what keeps
        // the frames coming (without it the scroll only advanced when the mouse moved).
        let zone = 48.0_f32.min(track_right - track_x - TIME_PAD).max(1.0);
        if p.x > track_right - zone {
            let ramp = ((p.x - (track_right - zone)) / zone).clamp(0.0, 1.0);
            let dt = ui.ctx().input(|i| i.stable_dt).max(0.0);
            app.ui.motion.scroll_x += ramp * SCROLL_SPEED * dt;
            ui.ctx().request_repaint();
        }
        let t = ((p.x.min(track_right) - track_x - TIME_PAD + app.ui.motion.scroll_x) / scale).max(0.0) as f64;
        app.ui.motion.drag_duration = Some(snap_dur(t).clamp(100.0, TimelineSettings::MAX_DURATION_MS as f64) as u32);
    }
    if hresp.drag_stopped() {
        app.ui.motion.scroll_x = 0.0;
        if let Some(ms) = app.ui.motion.drag_duration.take() {
            acts.push(Act::SetDuration(ms));
        }
        acts.push(Act::EndEdit);
    }

    // Read the scroll *after* the drag updated it, so the rows and the ruler (drawn next) agree:
    // one frame of lag between them is what made the ruler stutter while the edge was held out.
    let scroll = app.ui.motion.scroll_x;
    let x_of = |time: f64| track_x + TIME_PAD + (time as f32) * scale - scroll;
    let time_of = |x: f32| (((x - track_x - TIME_PAD + scroll) / scale).max(0.0)) as f64;

    // Past the duration the track area is shaded; its edge drags to resize the timeline.
    let shown = app.ui.motion.drag_duration.unwrap_or(settings.duration_ms);
    let end_x = x_of(shown as f64);
    if end_x < track_right {
        let outside = Rect::from_min_max(pos2(end_x.max(track_x), body.min.y), pos2(track_right, body.max.y));
        ui.painter().rect_filled(outside, CornerRadius::ZERO, t.border.gamma_multiply(0.35));
        ui.painter().line_segment([pos2(end_x, body.min.y), pos2(end_x, body.max.y)], Stroke::new(1.0, t.border_strong));
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
        // All selection state now lives in `selection`; read it directly here.
        let sel = &app.ui.motion.selection;
        let layer_sel = sel.contains(&SelItem::Layer(row.node));
        let prop = prop_of(row);
        let track_sel = matches!(&row.data, RowData::Property { .. }) && sel.contains(&SelItem::Track(row.node, prop));
        let clip_sel = if let RowData::Clip { id, .. } = &row.data { sel.contains(&SelItem::Clip(row.node, *id)) } else { false };
        let prop_has_item = matches!(&row.data, RowData::Property { .. })
            && sel.iter().any(|s| matches!(s, SelItem::Key(n, p, _) | SelItem::Segment(n, p, _) if *n == row.node && *p == prop));
        // `selected_row` and `selected_row_child` swap which is darker between the themes, so pick by
        // luminance: the selected parent block reads deeper, its children take the lighter "big
        // background" tint (matching the layers list). A layer selected pulls in every child, so the
        // layer row is the deep block and each child row the light one; picking a single child with no
        // parent just lights that row with the deep colour.
        let lum = |c: egui::Color32| -> f32 { 0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32 };
        let (main_bg, big_bg) = if lum(t.selected_row) <= lum(t.selected_row_child) {
            (t.selected_row, t.selected_row_child)
        } else {
            (t.selected_row_child, t.selected_row)
        };
        let bg = match &row.data {
            RowData::Layer { .. } => layer_sel.then_some(main_bg),
            RowData::Property { .. } => {
                if layer_sel {
                    Some(big_bg)
                } else if track_sel || prop_has_item {
                    Some(t.selected_row)
                } else {
                    None
                }
            }
            RowData::Clip { .. } => {
                if layer_sel {
                    Some(big_bg)
                } else if clip_sel {
                    Some(t.selected_row)
                } else {
                    None
                }
            }
        };
        if let Some(bg) = bg {
            ui.painter().rect_filled(rect, CornerRadius::ZERO, bg);
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
                    acts.push(Act::Click(SelItem::Layer(row.node), shift));
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
                    // The grips: 4 pt in from each end, 2 wide, 14 tall, rounded.
                    if x1 - x0 >= 20.0 {
                        for gx in [x0 + 4.0, x1 - 6.0] {
                            ui.painter().rect_filled(
                                Rect::from_min_size(pos2(gx, cy - 7.0), vec2(2.0, 14.0)),
                                CornerRadius::same(1),
                                egui::Color32::WHITE,
                            );
                        }
                    }
                    // One hit test on the whole bar: the outer 8 pt of each end time-stretches the
                    // layer about the other end, the middle slides the whole animation. A press
                    // records the motion as it was, and every frame recomputes it from that
                    // original plus the pointer's total delta — clamping each item at 0 on its own
                    // is what squashed the span when the middle was dragged left.
                    let sresp = ui.interact(r, ui.scope_id().with(("motion_summary", i)), Sense::click_and_drag());
                    let edge = EDGE_HIT.min((x1 - x0).max(1.0) / 2.0);
                    let hit = |x: f32| {
                        if x - x0 <= edge {
                            ClipEdge::TrimStart
                        } else if x1 - x <= edge {
                            ClipEdge::TrimEnd
                        } else {
                            ClipEdge::Move
                        }
                    };
                    if let Some(p) = sresp.hover_pos().or_else(|| sresp.interact_pointer_pos()) {
                        let e = app.ui.motion.layer_drag.as_ref().map_or_else(|| hit(p.x), |d| d.edge);
                        ui.ctx().set_cursor_icon(match e {
                            ClipEdge::Move => egui::CursorIcon::Move,
                            _ => egui::CursorIcon::ResizeHorizontal,
                        });
                    }
                    if sresp.drag_started()
                        && let Some(p) = ui.ctx().input(|i| i.pointer.press_origin()).or_else(|| sresp.interact_pointer_pos())
                    {
                        let orig = app.session.doc().ok().and_then(|d| d.node(row.node)).map(|n| n.motion.clone());
                        if let Some(orig) = orig {
                            // The press position decides the end, matching the hover cursor.
                            app.ui.motion.layer_drag =
                                Some(LayerDrag { node: row.node, edge: hit(p.x), origin_x: p.x, lo: *a, hi: *b, orig });
                            acts.push(Act::BeginEdit("Edit animation span"));
                        }
                    }
                    if sresp.dragged()
                        && let (Some(drag), Some(p)) = (app.ui.motion.layer_drag.clone(), sresp.interact_pointer_pos())
                    {
                        app.ui.motion.scrubbing = true;
                        let dx = ((p.x - drag.origin_x) / scale) as i64;
                        acts.push(Act::LayerDrag { node: drag.node, edge: drag.edge, lo: drag.lo, hi: drag.hi, orig: drag.orig, dx_ms: dx });
                    }
                    if sresp.drag_stopped() {
                        app.ui.motion.layer_drag = None;
                        acts.push(Act::EndEdit);
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
                // A press on the left column picks the track; one on the empty track lane clears the
                // selection (the keyframe and segment widgets above it still win their own presses,
                // so this only fires on truly empty lane).
                if rresp.clicked() {
                    let px = rresp.interact_pointer_pos().map(|p| p.x).unwrap_or(rect.right());
                    if px <= body.min.x + LEFT_W {
                        acts.push(Act::Click(SelItem::Track(row.node, prop_of(row)), shift));
                    } else {
                        acts.push(Act::ClearSelection);
                    }
                }
                let alpha = if *visible { 1.0 } else { 0.35 };
                let name = widgets::elide(ui, &row.label, 50.0);
                ui.painter().text(pos2(name_x, cy), Align2::LEFT_CENTER, name, regular(10.0), if *visible { t.text_secondary } else { t.text_tertiary });
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
                ui.painter().rect_filled(Rect::from_center_size(pr.center(), vec2(10.0, 10.0)), CornerRadius::same(2), if prev.is_some() { t.icon_secondary } else { t.text_tertiary });
                let dresp = ui.interact(dr, ui.scope_id().with(("motion_diamond", i)), Sense::click());
                // Lit whenever the track already has a keyframe (it is animated); the diamond is
                // solid only while the playhead is parked on one (the original `on_key` rule).
                let has_keys = !keys.is_empty();
                let dcol = if has_keys { t.accent } else if dresp.hovered() { t.icon } else { t.icon_secondary };
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
                ui.painter().rect_filled(Rect::from_center_size(nr.center(), vec2(10.0, 10.0)), CornerRadius::same(2), if next.is_some() { t.icon_secondary } else { t.text_tertiary });
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

                // The track's baseline.
                ui.painter().line_segment([pos2(track_x + TIME_PAD, cy), pos2(track_right, cy)], Stroke::new(1.0, t.border.gamma_multiply(alpha)));
                // Each segment's connection; the selected one is 2 px and accented, with a style block.
                let prop = prop_of(row);
                // Times whose diamonds light up: any selected key, plus both ends of any selected
                // segment on this track.
                let mut lit: HashSet<u32> = HashSet::new();
                for &k in keys {
                    if sel.contains(&SelItem::Key(row.node, prop, k)) {
                        lit.insert(k);
                    }
                }
                let mut seg_starts: Vec<u32> = Vec::new();
                for s in sel.iter() {
                    if let SelItem::Segment(n, p, t) = *s
                        && n == row.node
                        && p == prop
                    {
                        seg_starts.push(t);
                        lit.insert(t);
                        if let Some(pos) = keys.iter().position(|&k| k == t)
                            && let Some(&next) = keys.get(pos + 1)
                        {
                            lit.insert(next);
                        }
                    }
                }
                for (ki, w) in keys.windows(2).enumerate() {
                    let (xa, xb) = (x_of(w[0] as f64), x_of(w[1] as f64));
                    // A selected Segment item lights its segment; the Easing panel shows only when
                    // exactly one segment is selected (set by `sync_caches`).
                    let seg_sel = seg_starts.contains(&w[0]);
                    // The link between two keys is always there — only its weight and the mid-point
                    // curve badge change with the selection.
                    let line = if seg_sel { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.accent.gamma_multiply(0.35 * alpha)) };
                    // Run from the diamonds' left/right tips, not their centres, so a hollow key's
                    // interior is not crossed by the line.
                    let off = KF_R.min((xb - xa) / 2.0);
                    ui.painter().line_segment([pos2(xa + off, cy), pos2(xb - off, cy)], line);
                    if seg_sel && xb - xa > 24.0 {
                        // A rounded block at the midpoint, holding a mini easing curve.
                        let c = pos2((xa + xb) / 2.0, cy);
                        let gr = Rect::from_center_size(c, vec2(16.0, 16.0));
                        ui.painter().rect_filled(gr, CornerRadius::same(3), t.panel);
                        ui.painter().rect_stroke(gr, CornerRadius::same(3), Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                        if let Some(ease) = seg_easing(app, row.node, prop, w[0]) {
                            mini_curve(ui.painter(), c, ease, t.text);
                        }
                    }
                    let seg = Rect::from_min_max(pos2(xa, cy - 5.0), pos2(xb, cy + 5.0));
                    let sresp = ui.interact(seg, ui.scope_id().with(("motion_seg", i, ki)), Sense::click());
                    if sresp.clicked() {
                        acts.push(Act::Click(SelItem::Segment(row.node, prop, w[0]), shift));
                    }
                }
                for (ki, &time) in keys.iter().enumerate() {
                    let c = pos2(x_of(time as f64), cy);
                    let hit = Rect::from_center_size(c, vec2(KF_R * 2.0 + 6.0, KF_R * 2.0 + 6.0));
                    // Keyed by index, not time: an id that changes mid-drag drops the drag.
                    let kresp = ui.interact(hit, ui.scope_id().with(("motion_kf", i, ki)), Sense::click_and_drag());
                    let selected = lit.contains(&time);
                    // Two states: selected is a solid accent diamond; every other key is a hollow
                    // one with the same accent outline — green either way, never a dim grey.
                    let fill = if selected { t.accent.gamma_multiply(alpha) } else { egui::Color32::TRANSPARENT };
                    let edge = Stroke::new(1.0, t.accent.gamma_multiply(alpha));
                    ui.painter().add(Shape::convex_polygon(
                        vec![c + vec2(0.0, -KF_R), c + vec2(KF_R, 0.0), c + vec2(0.0, KF_R), c + vec2(-KF_R, 0.0)],
                        fill,
                        edge,
                    ));
                    if kresp.clicked() {
                        acts.push(Act::Click(SelItem::Key(row.node, prop_of(row), time), shift));
                    }
                    if kresp.drag_started() {
                        acts.push(Act::BeginEdit("Move keyframe"));
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
                    if kresp.drag_stopped() {
                        acts.push(Act::EndEdit);
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
                // The grips: 4 pt in from each end, 2 wide, the 22 pt bar less 8 (14) tall, rounded.
                if b - a >= 20.0 {
                    for gx in [a + 4.0, b - 6.0] {
                        ui.painter().rect_filled(
                            Rect::from_min_size(pos2(gx, cy - 7.0), vec2(2.0, 14.0)),
                            CornerRadius::same(1),
                            egui::Color32::WHITE,
                        );
                    }
                }
                let kind = row.label.split(" · ").next().unwrap_or(&row.label);
                let label = widgets::elide(ui, kind, (r.width() - 14.0).max(0.0));
                ui.painter().text(r.center(), Align2::CENTER_CENTER, label, regular(9.0), t.accent_on);
                // One hit test on the whole bar: the outer 8 pt of each end trims
                // that end, the middle slides the clip. A press records the clip as it was, and
                // every frame recomputes it from that original plus the pointer's total delta, so
                // frames never accumulate drift and a middle drag never changes the length.
                let cresp = ui.interact(r, ui.scope_id().with(("motion_clip", i)), Sense::click_and_drag());
                // 8 pt in from each end trims that end; the middle moves. Capped at half the width
                // so a clip narrower than 16 pt still has two ends instead of one big zone.
                let edge = EDGE_HIT.min((b - a).max(1.0) / 2.0);
                let hit = |x: f32| {
                    if x - a <= edge {
                        ClipEdge::TrimStart
                    } else if b - x <= edge {
                        ClipEdge::TrimEnd
                    } else {
                        ClipEdge::Move
                    }
                };
                // `hover_pos` shows the cursor before the press; `interact_pointer_pos` only has a
                // value once the button is down.
                if let Some(p) = cresp.hover_pos().or_else(|| cresp.interact_pointer_pos()) {
                    let edge = app.ui.motion.clip_drag.as_ref().map_or_else(|| hit(p.x), |d| d.edge);
                    ui.ctx().set_cursor_icon(match edge {
                        ClipEdge::Move => egui::CursorIcon::Move,
                        ClipEdge::TrimStart | ClipEdge::TrimEnd => egui::CursorIcon::ResizeHorizontal,
                    });
                }
                if cresp.drag_started()
                    && let Some(p) = ui.ctx().input(|i| i.pointer.press_origin()).or_else(|| cresp.interact_pointer_pos())
                {
                    let orig = app
                        .session
                        .doc()
                        .ok()
                        .and_then(|d| d.node(row.node))
                        .and_then(|n| n.motion.clips.iter().find(|c| c.id == *id))
                        .cloned();
                    if let Some(orig) = orig {
                        // The edge comes from where the button went down, not from where the drag
                        // threshold was crossed — otherwise a press on an end that moves a few
                        // points inward turns into a move and the cursor lied.
                        app.ui.motion.clip_drag = Some(ClipDrag { node: row.node, id: *id, edge: hit(p.x), origin_x: p.x, orig });
                        acts.push(Act::Click(SelItem::Clip(row.node, *id), shift));
                        acts.push(Act::BeginEdit("Edit animation clip"));
                    }
                }
                if cresp.dragged()
                    && let (Some(drag), Some(p)) = (app.ui.motion.clip_drag.clone(), cresp.interact_pointer_pos())
                {
                    app.ui.motion.scrubbing = true;
                    let dx = ((p.x - drag.origin_x) / scale) as i64;
                    acts.push(Act::ClipDrag { node: drag.node, id: drag.id, edge: drag.edge, orig: drag.orig, dx_ms: dx });
                }
                if cresp.drag_stopped() {
                    app.ui.motion.clip_drag = None;
                    acts.push(Act::EndEdit);
                }
                if cresp.clicked() {
                    acts.push(Act::Click(SelItem::Clip(row.node, *id), shift));
                }
            }
        }
    }

    // Vertical divider after the left column, and the playhead over everything.
    ui.painter().line_segment([pos2(track_x, body.min.y), pos2(track_x, body.max.y)], Stroke::new(1.0, t.border));
    let px = x_of(app.ui.motion.playhead_ms);
    // The head sits on the ruler's tick line (bottom of the header) so the axis reads against the
    // time scale instead of floating at the top of the band.
    let y0 = header_top + HEADER_H - 6.0;
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

/// The easing applied to the segment that starts at `time_ms` (the start keyframe's easing).
fn seg_easing(app: &FigApp, node: NodeId, prop: AnimProperty, time_ms: u32) -> Option<Ease> {
    app.session
        .doc()
        .ok()
        .and_then(|d| d.node(node))
        .and_then(|n| n.motion.track(prop))
        .and_then(|tr| tr.key_at(time_ms))
        .map(|k| k.easing)
}

/// The easing's curve drawn at 10×7 inside a segment's style block.
fn mini_curve(p: &egui::Painter, c: egui::Pos2, ease: Ease, col: egui::Color32) {
    let (lo, hi) = ease.value_range();
    let span = (hi - lo).max(1e-3);
    let (w, h) = (10.0, 7.0);
    let pts: Vec<egui::Pos2> = ease
        .sample(14)
        .iter()
        .map(|q| pos2(c.x - w / 2.0 + q[0] * w, c.y + h / 2.0 - (q[1] - lo) / span * h))
        .collect();
    p.add(Shape::line(pts, Stroke::new(1.0, col)));
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

/// The items a click on `item` adds to the selection. A layer pulls in every track and clip under it,
/// so selecting a parent carries all its children; every other item adds just itself.
fn expand(app: &FigApp, _page: NodeId, item: SelItem) -> Vec<SelItem> {
    if let SelItem::Layer(n) = item {
        let mut v = vec![SelItem::Layer(n)];
        if let Ok(doc) = app.session.doc()
            && let Some(node) = doc.node(n)
        {
            for tr in node.motion.tracks.iter().filter(|t| !t.is_empty()) {
                v.push(SelItem::Track(n, tr.property));
            }
            for c in &node.motion.clips {
                v.push(SelItem::Clip(n, c.id));
            }
        }
        v
    } else {
        vec![item]
    }
}

/// Reconciles the single-item caches (which the inspector and keyframe ops read) from the `selection`
/// set. `selected_clip` is left alone so the inspector's own clip pick isn't overwritten every frame.
fn sync_caches(app: &mut FigApp) {
    let sel = &app.ui.motion.selection;
    app.ui.motion.easing_target = sel.iter().filter(|s| matches!(s, SelItem::Segment(..))).count() == 1;
    app.ui.motion.selected = sel.iter().find_map(|s| match s {
        SelItem::Key(n, p, t) => Some(SelectedKey { node: *n, property: *p, time_ms: *t }),
        SelItem::Segment(n, p, t) => Some(SelectedKey { node: *n, property: *p, time_ms: *t }),
        _ => None,
    });
    app.ui.motion.selected_track = sel.iter().find_map(|s| {
        if let SelItem::Track(n, p) = s { Some(SelectedTrack { node: *n, property: *p }) } else { None }
    });
}

#[allow(clippy::cognitive_complexity)]
fn apply(app: &mut FigApp, ui: &Ui, page: NodeId, area: Rect, acts: Vec<Act>) {
    // Drop selections whose layer is no longer in the canvas/document selection, so rows that
    // vanished don't keep stale highlights.
    let live = app.session.selection();
    app.ui.motion.selection.retain(|s| live.contains(&s.node()));
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
            Act::Click(item, shift) => {
                // A layer click expands to every track and clip under it (rule: picking a parent
                // selects all its children together).
                let add = expand(app, page, item);
                let primary_in = app.ui.motion.selection.contains(&item);
                if primary_in {
                    // Re-clicking a selected item toggles just it (and its expansion) off.
                    for s in &add {
                        app.ui.motion.selection.remove(s);
                    }
                    if let SelItem::Clip(n, cid) = item {
                        if app.ui.motion.selected_clip == Some((n, cid)) {
                            app.ui.motion.selected_clip = None;
                        }
                        if app.ui.motion.clip_popover == Some((n, cid)) {
                            app.ui.motion.clip_popover = None;
                            app.ui.motion.clip_pop = None;
                        }
                    }
                } else if shift {
                    // Shift extends the selection instead of replacing it.
                    for s in &add {
                        app.ui.motion.selection.insert(*s);
                    }
                    if let SelItem::Clip(n, cid) = item {
                        app.ui.motion.selected_clip = Some((n, cid));
                        app.ui.motion.clip_popover = Some((n, cid));
                        app.ui.motion.clip_just_opened = true;
                    }
                } else {
                    // Plain click: a single, unique selection.
                    app.ui.motion.selection.clear();
                    for s in &add {
                        app.ui.motion.selection.insert(*s);
                    }
                    if let SelItem::Clip(n, cid) = item {
                        app.ui.motion.selected_clip = Some((n, cid));
                        app.ui.motion.clip_popover = Some((n, cid));
                        app.ui.motion.clip_just_opened = true;
                    } else {
                        app.ui.motion.selected_clip = None;
                    }
                }
                // Keep the layer visible: mirror a layer pick into the canvas selection without ever
                // removing from it.
                if let SelItem::Layer(n) = item {
                    let mut sel = app.session.selection();
                    if !sel.contains(&n) {
                        sel.push(n);
                        app.session.set_selection(sel);
                    }
                }
                sync_caches(app);
            }
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
            Act::BeginEdit(label) => {
                let _ = app.session.begin_interaction(label);
            }
            Act::EndEdit => {
                let _ = app.session.commit_interaction();
            }
            Act::AddKeyAtPlayhead => {
                if let Some(id) = app.session.selection().first().copied() {
                    let time = app.ui.motion.playhead_ms.max(0.0) as u32;
                    edit(app, "Add keyframe", |a| add_key_for_layer(a, id, time));
                }
            }
            Act::AddKey(node, prop, time) => edit(app, "Add keyframe", |a| add_key(a, node, prop, time)),
            Act::MoveKey(k, time) => {
                // Keep the moved key (and its segment, if selected) under its new time.
                let had_seg = app.ui.motion.selection.iter().any(|s| {
                    matches!(s, SelItem::Segment(n, p, t) if *n == k.node && *p == k.property && *t == k.time_ms)
                });
                app.ui.motion.selection.retain(|s| {
                    !matches!(s, SelItem::Key(n, p, t) | SelItem::Segment(n, p, t) if *n == k.node && *p == k.property && *t == k.time_ms)
                });
                move_key(app, k, time);
                app.ui.motion.selection.insert(SelItem::Key(k.node, k.property, time));
                if had_seg {
                    app.ui.motion.selection.insert(SelItem::Segment(k.node, k.property, time));
                }
                app.ui.motion.selected = Some(SelectedKey { node: k.node, property: k.property, time_ms: time });
            }
            Act::SetLoop(on) => edit(app, "Loop animation", |a| set_loop(a, page, on)),
            Act::Seek(t) => app.ui.motion.playhead_ms = t as f64,
            Act::SetDuration(ms) => edit(app, "Animation duration", |a| set_duration(a, page, ms)),
            Act::ClearSelection => {
                // Clear only the timeline selection, not the canvas/document one: clicking blank leaves
                // the layers where they are (rule: blank = no timeline selection).
                app.ui.motion.selection.clear();
                app.ui.motion.selected = None;
                app.ui.motion.selected_track = None;
                app.ui.motion.selected_clip = None;
                app.ui.motion.easing_target = false;
                app.ui.motion.clip_popover = None;
                app.ui.motion.clip_pop = None;
            }
            Act::ToggleVisible(n) => edit(app, "Show/hide layer", |a| {
                let Ok(doc) = a.session.doc_mut() else { return };
                if let Some(nd) = doc.node_mut(n) {
                    nd.visible = !nd.visible;
                }
                doc.mark_dirty([n]);
            }),
            Act::LayerDrag { node, edge, lo, hi, orig, dx_ms } => {
                let Ok(doc) = app.session.doc_mut() else { return };
                if let Some(nd) = doc.node_mut(node) {
                    nd.motion = layer_dragged(&orig, lo, hi, edge, dx_ms);
                }
                doc.mark_dirty([node]);
            }
            Act::ClipDrag { node, id, edge, orig, dx_ms } => {
                let Ok(doc) = app.session.doc_mut() else { return };
                if let Some(c) = doc.node_mut(node).and_then(|nd| nd.motion.clips.iter_mut().find(|c| c.id == id)) {
                    *c = clip_dragged(&orig, edge, dx_ms);
                }
                doc.mark_dirty([node]);
            }
            Act::ToggleTrackVisible(n, prop) => edit(app, "Show/hide track", |a| {
                let Ok(doc) = a.session.doc_mut() else { return };
                if let Some(tr) = doc.node_mut(n).and_then(|nd| nd.motion.track_mut(prop)) {
                    tr.visible = !tr.visible;
                }
                doc.mark_dirty([n]);
            }),
            Act::ToggleKey(node, prop) => {
                let time = app.ui.motion.playhead_ms.max(0.0) as u32;
                let exists = app
                    .session
                    .doc()
                    .ok()
                    .and_then(|d| d.node(node))
                    .and_then(|n| n.motion.track(prop))
                    .is_some_and(|tr| tr.key_at(time).is_some());
                if exists {
                    edit(app, "Delete keyframe", |a| delete_key(a, SelectedKey { node, property: prop, time_ms: time }));
                    app.ui.motion.selected = None;
                } else {
                    edit(app, "Add keyframe", |a| add_key(a, node, prop, time));
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
    // Read the key before consuming it: consuming first made the check below see it as gone, so
    // Delete never removed the selected keyframe.
    let del = ui.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace));
    if del && !app.ui.motion.selection.is_empty() {
        // Swallow the key so the global shortcut does not delete the whole layer.
        ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, Key::Delete);
            i.consume_key(egui::Modifiers::NONE, Key::Backspace);
        });
    }
    if del {
        // Delete everything the selection holds: keyframes/segments first, then clips.
        let keys: Vec<SelectedKey> = app
            .ui
            .motion
            .selection
            .iter()
            .filter_map(|s| match *s {
                SelItem::Key(n, p, t) | SelItem::Segment(n, p, t) => Some(SelectedKey { node: n, property: p, time_ms: t }),
                _ => None,
            })
            .collect();
        for k in keys {
            edit(app, "Delete keyframe", |a| delete_key(a, k));
        }
        let clips: Vec<(NodeId, u32)> = app
            .ui
            .motion
            .selection
            .iter()
            .filter_map(|s| if let SelItem::Clip(n, cid) = *s { Some((n, cid)) } else { None })
            .collect();
        for (n, cid) in clips {
            edit(app, "Delete animation clip", |a| delete_clip(a, n, cid));
            if app.ui.motion.selected_clip == Some((n, cid)) {
                app.ui.motion.selected_clip = None;
            }
            if app.ui.motion.clip_popover == Some((n, cid)) {
                app.ui.motion.clip_popover = None;
                app.ui.motion.clip_pop = None;
            }
        }
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
    app.ui.motion.selection.insert(SelItem::Key(node, prop, time_ms));
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
    app.ui.motion.selection.retain(|s| {
        !matches!(s, SelItem::Key(n, p, t) | SelItem::Segment(n, p, t) if *n == k.node && *p == k.property && *t == k.time_ms)
    });
    if app.ui.motion.selected == Some(k) {
        app.ui.motion.selected = None;
    }
}

fn delete_clip(app: &mut FigApp, node: NodeId, id: u32) {
    let Ok(doc) = app.session.doc_mut() else { return };
    if let Some(n) = doc.node_mut(node) {
        n.motion.clips.retain(|c| c.id != id);
        n.motion.normalize();
    }
    doc.mark_dirty([node]);
    app.ui.motion.selection.remove(&SelItem::Clip(node, id));
}

/// The clip as moved / trimmed by `dx_ms` from its state at the start of the drag.
///
/// `Move` slides the whole clip (delay only), `TrimStart` moves the start and keeps the end,
/// `TrimEnd` moves the end and keeps the start. Recomputing from the original each frame (rather
/// than adding a per-frame delta) is what keeps a move from drifting the length.
fn clip_dragged(orig: &motion::Clip, edge: ClipEdge, dx_ms: i64) -> motion::Clip {
    let mut c = orig.clone();
    let d0 = orig.duration_ms.max(1) as i64;
    let cycles = match orig.repeat {
        motion::ClipRepeat::Times(n) => n.max(1) as i64,
        _ => 1,
    };
    let total = d0 * cycles;
    match edge {
        ClipEdge::Move => {
            c.delay_ms = (orig.delay_ms as i64 + dx_ms).max(0) as u32;
        }
        ClipEdge::TrimStart => {
            let end = orig.delay_ms as i64 + total;
            let start = (orig.delay_ms as i64 + dx_ms).clamp(0, end - cycles);
            c.delay_ms = start as u32;
            c.duration_ms = (((end - start) / cycles).max(1)) as u32;
        }
        ClipEdge::TrimEnd => {
            let end = (orig.delay_ms as i64 + total + dx_ms).max(orig.delay_ms as i64 + cycles);
            c.duration_ms = (((end - orig.delay_ms as i64) / cycles).max(1)) as u32;
        }
    }
    c
}

/// The layer's whole motion as moved or time-stretched by `dx_ms` from its state at the drag start.
/// `Move` slides everything by one delta clamped by the earliest item (clamping each item at 0 on
/// its own squashed the span); the ends stretch the clips and keys about the other end.
fn layer_dragged(orig: &motion::NodeMotion, lo: u32, hi: u32, edge: ClipEdge, dx_ms: i64) -> motion::NodeMotion {
    let mut m = orig.clone();
    let min_start = m
        .clips
        .iter()
        .map(|c| c.delay_ms)
        .chain(m.tracks.iter().flat_map(|t| t.keys.iter().map(|k| k.time_ms)))
        .min()
        .unwrap_or(0);
    if edge == ClipEdge::Move {
        let d = dx_ms.max(-(min_start as i64));
        for c in &mut m.clips {
            c.delay_ms = (c.delay_ms as i64 + d) as u32;
        }
        for t in &mut m.tracks {
            for k in &mut t.keys {
                k.time_ms = (k.time_ms as i64 + d) as u32;
            }
        }
        return m;
    }
    let span = (hi.saturating_sub(lo)).max(1) as f64;
    let (new_lo, new_hi) = if edge == ClipEdge::TrimStart {
        ((lo as i64 + dx_ms).clamp(0, hi as i64 - 1) as u32, hi)
    } else {
        (lo, (hi as i64 + dx_ms).max(lo as i64 + 1) as u32)
    };
    let f = (new_hi - new_lo) as f64 / span;
    let map = |t: u32| ((new_lo as f64 + (t as f64 - lo as f64) * f).round().max(0.0)) as u32;
    for c in &mut m.clips {
        let start = map(c.delay_ms);
        let end = map(c.delay_ms.saturating_add(c.duration_ms.max(1)));
        c.delay_ms = start;
        c.duration_ms = end.saturating_sub(start).max(1);
    }
    for t in &mut m.tracks {
        for k in &mut t.keys {
            k.time_ms = map(k.time_ms);
        }
    }
    m
}

fn set_loop(app: &mut FigApp, page: NodeId, on: bool) {
    set_timeline(app, page, |s| s.looping = on);
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

    fn clip() -> motion::Clip {
        motion::Clip::new(motion::ClipKind::Position, motion::ClipMode::In)
    }

    #[test]
    fn moving_a_clip_never_changes_its_length() {
        let orig = clip().delay(1000).dur(500);
        let m = clip_dragged(&orig, ClipEdge::Move, 250);
        assert_eq!((m.delay_ms, m.duration_ms), (1250, 500));
        let m = clip_dragged(&orig, ClipEdge::Move, -300);
        assert_eq!((m.delay_ms, m.duration_ms), (700, 500));
        // It never goes before 0.
        let m = clip_dragged(&orig, ClipEdge::Move, -5000);
        assert_eq!((m.delay_ms, m.duration_ms), (0, 500));
    }

    #[test]
    fn trimming_a_clip_end_keeps_the_other_end() {
        let orig = clip().delay(1000).dur(500);
        // Right end right: only longer.
        let t = clip_dragged(&orig, ClipEdge::TrimEnd, 200);
        assert_eq!((t.delay_ms, t.duration_ms), (1000, 700));
        // Right end left: only shorter — it must not move the clip.
        let t = clip_dragged(&orig, ClipEdge::TrimEnd, -300);
        assert_eq!((t.delay_ms, t.duration_ms), (1000, 200));
        // Left end left: the start moves, the end stays.
        let t = clip_dragged(&orig, ClipEdge::TrimStart, -150);
        assert_eq!((t.delay_ms, t.duration_ms), (850, 650));
        // Left end right: the start moves, the end stays.
        let t = clip_dragged(&orig, ClipEdge::TrimStart, 100);
        assert_eq!((t.delay_ms, t.duration_ms), (1100, 400));
    }

    #[test]
    fn clip_ends_clamp_without_moving_the_other() {
        let orig = clip().delay(0).dur(300);
        let t = clip_dragged(&orig, ClipEdge::TrimStart, -100);
        assert_eq!((t.delay_ms, t.duration_ms), (0, 300), "the start cannot go before 0");
        let t = clip_dragged(&orig, ClipEdge::TrimEnd, -1000);
        assert_eq!(t.duration_ms, 1, "a clip keeps at least 1 ms");
    }
}
