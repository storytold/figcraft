//! The clip settings window: a card floating over the canvas, anchored to the inspector's left
//! edge. It opens from the Animations list or when a clip is added.
//!
//! It holds the clip's kind, Type (in / out / custom), Direction and Amount (or the custom From /
//! To values), Delay, Duration and the easing editor. The right panel keeps the layer's own
//! fields; the clip's settings are not one of its sections.

use egui::{Align, Align2, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{Clip, ClipKind, ClipMode, Ease};
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::panels::right::{menu_check, menu_popup};
use crate::popover::{Hold, Open, Popover};
use crate::theme::{Tokens, regular};
use crate::widgets::{self, FieldEdit, Lead};

/// The card's width: 240 pt of content plus 12 pt padding on each side.
const WIDTH: f32 = 264.0;

/// What `amount` means for a kind: its display factor and suffix.
fn amount_unit(kind: ClipKind) -> (f32, &'static str) {
    match kind {
        ClipKind::Position => (1.0, ""),
        ClipKind::Rotation => (1.0, "°"),
        _ => (100.0, "%"),
    }
}

/// The label above the amount field.
fn amount_label(kind: ClipKind) -> &'static str {
    match kind {
        ClipKind::Position => "Distance",
        ClipKind::Rotation => "Angle",
        ClipKind::Opacity => "Opacity",
        ClipKind::Scale => "Scale",
        ClipKind::Size => "Size",
    }
}

/// The popover's left-right layout: a label inset 16 pt from the card's left edge, the control in a
/// fixed column at 108 pt, 124 pt wide.
const ROW_H: f32 = 32.0;
const CONTROL_X: f32 = 108.0;
const CONTROL_W: f32 = 124.0;
const CONTROL_H: f32 = 24.0;

/// A labeled row: `label` on the left, `add` laying out the control in the right-hand column.
fn field<F: FnOnce(&mut Ui)>(ui: &mut Ui, t: &Tokens, label: &str, add: F) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    ui.painter().text(pos2(r.left() + 4.0, r.center().y), Align2::LEFT_CENTER, label, regular(11.0), t.text_secondary);
    let cr = Rect::from_min_size(pos2(r.left() + CONTROL_X, r.top() + (ROW_H - CONTROL_H) / 2.0), vec2(CONTROL_W, CONTROL_H));
    let mut child = ui.new_child(UiBuilder::new().max_rect(cr).layout(Layout::left_to_right(Align::Center)));
    child.spacing_mut().item_spacing = vec2(4.0, 0.0);
    add(&mut child);
}

/// The hairline between the row groups.
fn rule(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([r.left_center(), r.right_center()], egui::Stroke::new(1.0, t.border));
}

/// What a row of the window edits: the clip as the document holds it, and the copy this frame builds.
/// Each row returns whether it changed that copy.
struct Row<'a> {
    orig: &'a Clip,
    new: &'a mut Clip,
}

/// The kind menu: picking another kind keeps the clip's identity and timing, and takes the kind's
/// own direction and amount.
fn kind_row(ui: &mut Ui, t: &Tokens, r: &mut Row) -> bool {
    let orig = r.orig;
    let mut changed = false;
    field(ui, t, tr("Animation"), |ui| {
        let resp = widgets::dropdown(ui, tr(orig.kind.name()), ui.available_width(), None);
        menu_popup(&resp).show(|ui| {
            for k in ClipKind::ALL {
                if menu_check(ui, k == orig.kind, tr(k.name()), "") {
                    if k != orig.kind {
                        let mut c = Clip::new(k, orig.mode);
                        c.id = orig.id;
                        c.delay_ms = orig.delay_ms;
                        c.duration_ms = orig.duration_ms;
                        c.easing = orig.easing;
                        c.repeat = orig.repeat;
                        c.yoyo = orig.yoyo;
                        c.visible = orig.visible;
                        *r.new = c;
                        changed = true;
                    }
                    ui.close();
                }
            }
        });
    });
    changed
}

/// Type: slide in, slide out, or the custom from/to pair.
fn type_row(ui: &mut Ui, t: &Tokens, r: &mut Row) -> bool {
    let orig = r.orig;
    let mut changed = false;
    field(ui, t, tr("Type"), |ui| {
        let resp = widgets::dropdown(ui, &orig.mode.label(orig.kind), ui.available_width(), None);
        menu_popup(&resp).show(|ui| {
            for m in [ClipMode::In, ClipMode::Out, ClipMode::Custom] {
                if menu_check(ui, m == orig.mode, &m.label(orig.kind), "") {
                    r.new.mode = m;
                    changed = true;
                    ui.close();
                }
            }
        });
    });
    changed
}

/// Which end of the custom pair a field edits, and which component of it.
#[derive(Clone, Copy, PartialEq)]
enum End {
    From,
    To,
}

/// One custom value field: both components of a vector kind, or the scalar a single one carries.
fn custom_field(ui: &mut Ui, r: &mut Row, end: End, axis: Option<bool>) -> bool {
    let orig = r.orig;
    let (f, suffix) = amount_unit(orig.kind);
    let v = if end == End::To { orig.to } else { orig.from };
    let half = (ui.available_width() - 4.0) / 2.0;
    let (lead, current, width) = match axis {
        Some(true) => (Lead::Text("X"), v.x, half),
        Some(false) => (Lead::Text("Y"), v.y, half),
        None => (Lead::None, v.x, ui.available_width()),
    };
    let id = ui.scope_id().with(("clip_custom", end == End::To, axis));
    let Some(e) = widgets::number_field_mixed(ui, id, lead, Some((current * f as f64 * 100.0).round() / 100.0), suffix, width, 1.0) else {
        return false;
    };
    let Some(FieldEdit::Set(n) | FieldEdit::Scrub(n, _)) = e.settled() else { return false };
    let value = n / f as f64;
    let nv = match axis {
        Some(true) => figcraft_geom::Vec2::new(value, v.y),
        Some(false) => figcraft_geom::Vec2::new(v.x, value),
        None => figcraft_geom::Vec2::new(value, v.y),
    };
    if end == End::To {
        r.new.to = nv;
    } else {
        r.new.from = nv;
    }
    true
}

/// The custom start and end values, on their own labeled rows.
fn custom_rows(ui: &mut Ui, t: &Tokens, r: &mut Row) -> bool {
    let vector = matches!(r.orig.kind, ClipKind::Position | ClipKind::Scale | ClipKind::Size);
    let mut changed = false;
    for (label, end) in [(tr("From"), End::From), (tr("To"), End::To)] {
        field(ui, t, label, |ui| {
            if vector {
                // Two 60 pt fields with a 4 pt gap.
                changed |= custom_field(ui, r, end, Some(true));
                changed |= custom_field(ui, r, end, Some(false));
            } else {
                changed |= custom_field(ui, r, end, None);
            }
        });
    }
    changed
}

/// Direction, for the kinds that have one.
fn direction_row(ui: &mut Ui, t: &Tokens, r: &mut Row) -> bool {
    let orig = r.orig;
    let dirs = orig.kind.directions();
    if dirs.is_empty() {
        return false;
    }
    let mut changed = false;
    field(ui, t, tr("Direction"), |ui| {
        let cur = dirs.iter().position(|d| *d == orig.direction).map_or_else(|| orig.direction.label(orig.mode), |i| dirs[i].label(orig.mode));
        let resp = widgets::dropdown(ui, &cur, ui.available_width(), None);
        menu_popup(&resp).show(|ui| {
            for d in dirs {
                if menu_check(ui, *d == orig.direction, &d.label(orig.mode), "") {
                    r.new.direction = *d;
                    changed = true;
                    ui.close();
                }
            }
        });
    });
    changed
}

/// How far, how much or how fast the clip goes.
fn amount_row(ui: &mut Ui, t: &Tokens, r: &mut Row) -> bool {
    let orig = r.orig;
    let mut changed = false;
    field(ui, t, tr(amount_label(orig.kind)), |ui| {
        let (f, suffix) = amount_unit(orig.kind);
        let id = ui.scope_id().with("clip_amount");
        let current = Some((orig.amount as f64 * f as f64 * 100.0).round() / 100.0);
        if let Some(e) = widgets::number_field_mixed(ui, id, Lead::None, current, suffix, ui.available_width(), 1.0)
            && let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, _)) = e.settled()
        {
            r.new.amount = (v / f as f64) as f32;
            changed = true;
        }
    });
    changed
}

/// Delay and duration, each on its own labeled row.
fn timing_rows(ui: &mut Ui, t: &Tokens, r: &mut Row) -> bool {
    let orig = r.orig;
    let mut changed = false;
    field(ui, t, tr("Delay"), |ui| {
        let id = ui.scope_id().with("clip_delay");
        if let Some(e) = widgets::number_field_mixed(ui, id, Lead::None, Some(orig.delay_ms as f64), "ms", ui.available_width(), 1.0)
            && let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, _)) = e.settled()
        {
            r.new.delay_ms = v.max(0.0) as u32;
            changed = true;
        }
    });
    field(ui, t, tr("Duration"), |ui| {
        let id = ui.scope_id().with("clip_duration");
        if let Some(e) = widgets::number_field_mixed(ui, id, Lead::None, Some(orig.duration_ms as f64), "ms", ui.available_width(), 1.0)
            && let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, _)) = e.settled()
        {
            r.new.duration_ms = v.max(1.0) as u32;
            changed = true;
        }
    });
    changed
}

/// The easing preset menu, then the curve or spring editor under it. Returns whether a preset was
/// picked, and the drag the editor reported.
fn easing_rows(ui: &mut Ui, t: &Tokens, r: &mut Row) -> (bool, crate::motion::easing::EaseChange) {
    let orig = r.orig;
    let mut picked = false;
    let mut drag = crate::motion::easing::EaseChange::default();
    field(ui, t, tr("Easing"), |ui| {
        let resp = widgets::dropdown(ui, orig.easing.name(), ui.available_width(), None);
        menu_popup(&resp).show(|ui| {
            for (name, e) in Ease::PRESETS {
                if menu_check(ui, orig.easing == e, name, "") {
                    r.new.easing = e;
                    picked = true;
                    ui.close();
                }
            }
        });
    });
    let h = crate::motion::easing::editor_height(orig.easing);
    let (er, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let ch = crate::motion::easing::editor(ui, er, "clip", 4.0, orig.easing, t);
    drag.started |= ch.started;
    drag.stopped |= ch.stopped;
    drag.discrete |= ch.discrete;
    if ch.ease.is_some() {
        drag.ease = ch.ease;
    }
    (picked, drag)
}

/// Draws the open clip's window, if any, and applies its edits.
pub fn show(app: &mut FigApp, ctx: &egui::Context, region: Rect, t: &Tokens) {
    let Some((layer, cid)) = app.ui.motion.clip_popover else { return };
    let clip = app.session.doc().ok().and_then(|d| d.node(layer)).and_then(|n| n.motion.clips.iter().find(|c| c.id == cid)).cloned();
    // The clip may have been removed (or undone) since it was opened.
    let Some(orig) = clip else {
        app.ui.motion.clip_popover = None;
        app.ui.motion.clip_pop = None;
        return;
    };
    let title = format!("{} · {}", tr(orig.kind.name()), orig.mode.label(orig.kind));
    // Kept across frames (its height is measured on the first layout pass); a fresh one would be
    // laid out invisibly every frame and never show.
    let mut pop = match app.ui.motion.clip_pop.take() {
        Some(p) => p,
        None => Popover::new(("motion_clip_popover", layer, cid), title.clone()).width(WIDTH).pad(12.0),
    };
    pop.title = title;
    // The click (or menu item) that opened it is still this frame's press: it must not close it.
    if std::mem::take(&mut app.ui.motion.clip_just_opened) {
        pop = pop.close_on_outside(false);
    }
    let mut new = orig.clone();
    let mut changed = false;
    let mut drag = crate::motion::easing::EaseChange::default();
    let anchor = app.ui.motion.clip_anchor;
    let y = anchor.map_or(region.top() + 100.0, |r| r.top() - 16.0);
    // The row that opened it stays clickable: its press toggles the window instead of closing it.
    let held: Vec<Rect> = anchor.into_iter().collect();
    let close = pop.show(ctx, Open::Anchor(pos2(region.left() - 8.0, y)), Hold { rects: &held, escape: false }, |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
        let mut row = Row { orig: &orig, new: &mut new };
        changed |= kind_row(ui, t, &mut row);
        changed |= type_row(ui, t, &mut row);
        if orig.mode == ClipMode::Custom {
            changed |= custom_rows(ui, t, &mut row);
        } else {
            changed |= direction_row(ui, t, &mut row);
            changed |= amount_row(ui, t, &mut row);
        }
        // The hairline between the type block and the timing rows.
        ui.add_space(4.0);
        rule(ui, t);
        ui.add_space(4.0);
        changed |= timing_rows(ui, t, &mut row);
        let (picked, reported) = easing_rows(ui, t, &mut row);
        changed |= picked;
        drag = reported;
    });

    // A preset, a field or a dropdown is one undo step; a handle drag stays open until release.
    if let Some(e) = drag.ease {
        new.easing = e;
        changed = true;
    }
    if changed {
        // The interaction holds a handle drag together (opening it again is a no-op), so its
        // frames are one undo step; a discrete change closes it at once and is a step of its own.
        // A document that refuses it (read-only, or a preview) writes nothing.
        if app.session.begin_interaction("Edit animation clip").is_ok() {
            app.run("motion.clipUpdate", json!({"node": layer.0, "id": cid, "clip": new}));
        }
        if drag.ease.is_none() || drag.discrete || drag.stopped {
            let _ = app.session.commit_interaction();
        }
    } else if drag.stopped {
        let _ = app.session.commit_interaction();
    }
    if close {
        app.ui.motion.clip_popover = None;
        app.ui.motion.clip_pop = None;
    } else {
        app.ui.motion.clip_pop = Some(pop);
    }
}

/// The clip the window is on (for tests).
pub type Target = (NodeId, u32);
