//! The clip settings window: a card floating over the canvas, anchored to the inspector's left
//! edge. It opens from the Animations list or when a clip is added.
//!
//! It holds the clip's kind, Type (in / out / custom), Direction and Amount (or the custom From /
//! To values), Delay, Duration and the easing editor. The right panel keeps the layer's own
//! fields; the clip's settings are not one of its sections.

use egui::{Align, Align2, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{Clip, ClipKind, ClipMode, Ease};

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
    let cr = Rect::from_min_size(
        pos2(r.left() + CONTROL_X, r.top() + (ROW_H - CONTROL_H) / 2.0),
        vec2(CONTROL_W, CONTROL_H),
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(cr).layout(Layout::left_to_right(Align::Center)));
    child.spacing_mut().item_spacing = vec2(4.0, 0.0);
    add(&mut child);
}

/// The hairline between the row groups.
fn rule(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([r.left_center(), r.right_center()], egui::Stroke::new(1.0, t.border));
}

/// Draws the open clip's window, if any, and applies its edits.
pub fn show(app: &mut FigApp, ctx: &egui::Context, region: Rect, t: &Tokens) {
    let Some((layer, cid)) = app.ui.motion.clip_popover else { return };
    let clip = app
        .session
        .doc()
        .ok()
        .and_then(|d| d.node(layer))
        .and_then(|n| n.motion.clips.iter().find(|c| c.id == cid))
        .cloned();
    // The clip may have been removed (or undone) since it was opened.
    let Some(orig) = clip else {
        app.ui.motion.clip_popover = None;
        app.ui.motion.clip_pop = None;
        return;
    };
    let title = format!("{} · {}", orig.kind.name(), orig.mode.label(orig.kind));
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

        // Kind.
        field(ui, t, tr("Animation"), |ui| {
            let resp = widgets::dropdown(ui, orig.kind.name(), ui.available_width(), None);
            menu_popup(&resp).show(|ui| {
                for k in ClipKind::ALL {
                    if menu_check(ui, k == orig.kind, k.name(), "") {
                        if k != orig.kind {
                            // The new kind keeps the clip's identity and timing, and takes the
                            // kind's own direction / amount.
                            let mut c = Clip::new(k, orig.mode);
                            c.id = orig.id;
                            c.delay_ms = orig.delay_ms;
                            c.duration_ms = orig.duration_ms;
                            c.easing = orig.easing;
                            c.repeat = orig.repeat;
                            c.yoyo = orig.yoyo;
                            c.visible = orig.visible;
                            new = c;
                            changed = true;
                        }
                        ui.close();
                    }
                }
            });
        });

        // Type: Slide in / out / Custom.
        field(ui, t, tr("Type"), |ui| {
            let resp = widgets::dropdown(ui, &orig.mode.label(orig.kind), ui.available_width(), None);
            menu_popup(&resp).show(|ui| {
                for m in [ClipMode::In, ClipMode::Out, ClipMode::Custom] {
                    if menu_check(ui, m == orig.mode, &m.label(orig.kind), "") {
                        new.mode = m;
                        changed = true;
                        ui.close();
                    }
                }
            });
        });

        if orig.mode == ClipMode::Custom {
            // The free start / end values.
            let vec2 = matches!(orig.kind, ClipKind::Position | ClipKind::Scale | ClipKind::Size);
            let (f, suffix) = amount_unit(orig.kind);
            for (name, to) in [("From", false), ("To", true)] {
                field(ui, t, tr(name), |ui| {
                    let v = if to { orig.to } else { orig.from };
                    if vec2 {
                        // Two 60 pt fields with a 4 pt gap.
                        let half = (ui.available_width() - 4.0) / 2.0;
                        let id = ui.id().with(("clip_from_x", to));
                        if let Some(e) = widgets::number_field_mixed(
                            ui,
                            id,
                            Lead::Text("X"),
                            Some((v.x * f as f64 * 100.0).round() / 100.0),
                            suffix,
                            half,
                            1.0,
                        ) && let Some(FieldEdit::Set(x) | FieldEdit::Scrub(x, _)) = e.settled()
                        {
                            let nv = figcraft_geom::Vec2::new(x / f as f64, v.y);
                            if to { new.to = nv; } else { new.from = nv; }
                            changed = true;
                        }
                        let id = ui.id().with(("clip_from_y", to));
                        if let Some(e) = widgets::number_field_mixed(
                            ui,
                            id,
                            Lead::Text("Y"),
                            Some((v.y * f as f64 * 100.0).round() / 100.0),
                            suffix,
                            half,
                            1.0,
                        ) && let Some(FieldEdit::Set(y) | FieldEdit::Scrub(y, _)) = e.settled()
                        {
                            let nv = figcraft_geom::Vec2::new(v.x, y / f as f64);
                            if to { new.to = nv; } else { new.from = nv; }
                            changed = true;
                        }
                    } else {
                        let id = ui.id().with(("clip_from", to));
                        if let Some(e) = widgets::number_field_mixed(
                            ui,
                            id,
                            Lead::None,
                            Some((v.x * f as f64 * 100.0).round() / 100.0),
                            suffix,
                            ui.available_width(),
                            1.0,
                        ) && let Some(FieldEdit::Set(x) | FieldEdit::Scrub(x, _)) = e.settled()
                        {
                            let nv = figcraft_geom::Vec2::new(x / f as f64, v.y);
                            if to { new.to = nv; } else { new.from = nv; }
                            changed = true;
                        }
                    }
                });
            }
        } else {
            // Direction.
            let dirs = orig.kind.directions();
            if !dirs.is_empty() {
                field(ui, t, tr("Direction"), |ui| {
                    let cur = dirs
                        .iter()
                        .position(|d| *d == orig.direction)
                        .map_or_else(|| orig.direction.label(orig.mode), |i| dirs[i].label(orig.mode));
                    let resp = widgets::dropdown(ui, &cur, ui.available_width(), None);
                    menu_popup(&resp).show(|ui| {
                        for d in dirs {
                            if menu_check(ui, *d == orig.direction, &d.label(orig.mode), "") {
                                new.direction = *d;
                                changed = true;
                                ui.close();
                            }
                        }
                    });
                });
            }
            // Amount.
            field(ui, t, tr(amount_label(orig.kind)), |ui| {
                let (f, suffix) = amount_unit(orig.kind);
                let id = ui.id().with("clip_amount");
                if let Some(e) = widgets::number_field_mixed(
                    ui,
                    id,
                    Lead::None,
                    Some((orig.amount as f64 * f as f64 * 100.0).round() / 100.0),
                    suffix,
                    ui.available_width(),
                    1.0,
                ) && let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, _)) = e.settled()
                {
                    new.amount = (v / f as f64) as f32;
                    changed = true;
                }
            });
        }

        // The hairline between the type block and the timing rows.
        ui.add_space(4.0);
        rule(ui, t);
        ui.add_space(4.0);

        // Delay and duration, each on its own labeled row.
        field(ui, t, tr("Delay"), |ui| {
            let id = ui.id().with("clip_delay");
            if let Some(e) = widgets::number_field_mixed(ui, id, Lead::None, Some(orig.delay_ms as f64), "ms", ui.available_width(), 1.0)
                && let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, _)) = e.settled()
            {
                new.delay_ms = v.max(0.0) as u32;
                changed = true;
            }
        });
        field(ui, t, tr("Duration"), |ui| {
            let id = ui.id().with("clip_duration");
            if let Some(e) = widgets::number_field_mixed(ui, id, Lead::None, Some(orig.duration_ms as f64), "ms", ui.available_width(), 1.0)
                && let Some(FieldEdit::Set(v) | FieldEdit::Scrub(v, _)) = e.settled()
            {
                new.duration_ms = v.max(1.0) as u32;
                changed = true;
            }
        });

        // Easing: the preset dropdown, then the Curve | Spring editor.
        field(ui, t, tr("Easing"), |ui| {
            let resp = widgets::dropdown(ui, orig.easing.name(), ui.available_width(), None);
            menu_popup(&resp).show(|ui| {
                for (name, e) in Ease::PRESETS {
                    if menu_check(ui, orig.easing == e, name, "") {
                        new.easing = e;
                        changed = true;
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
        if ch.ease.is_some() {
            drag.ease = ch.ease;
        }
    });

    // A preset, a field or a dropdown is one undo step; a handle drag stays open until release.
    if let Some(e) = drag.ease {
        new.easing = e;
        changed = true;
    }
    if changed {
        let _ = app.session.begin_interaction("Edit animation clip");
        if let Ok(doc) = app.session.doc_mut() {
            if let Some(n) = doc.node_mut(layer)
                && let Some(c) = n.motion.clips.iter_mut().find(|c| c.id == cid)
            {
                *c = new.clone();
                c.sanitize();
            }
            doc.mark_dirty([layer]);
        }
        if drag.ease.is_none() || drag.stopped {
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
