//! The desktop tab strip: home, one tab per open file, new tab. In a frameless window it is also
//! the title bar: its empty space drags the window (double-click maximises), and it carries the
//! window buttons; the window's edges resize it.

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::a11y::Tip;
use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular};
use crate::{FigApp, TitleBar};

pub const HEIGHT: f32 = 40.0;
const MIN_TAB_WIDTH: f32 = 96.0;
const OVERFLOW_WIDTH: f32 = 32.0;
const OPEN_TIME: f64 = 0.18;

fn tab_name_width(width: f32, mark_width: f32, dirty: bool, close_visible: bool) -> f32 {
    let trailing = match (close_visible || mark_width > 0.0, dirty) {
        (true, true) => 72.0,
        (true, false) => 60.0,
        (false, true) => 56.0,
        (false, false) => 40.0,
    };
    (width - mark_width - trailing).max(0.0)
}

/// Which tabs were open last frame and when the newly opened ones appeared. It is updated in place
/// in egui's memory; it only allocates when the set of tabs changes.
#[derive(Clone, Default)]
struct TabMotion {
    initialized: bool,
    known: Vec<u64>,
    opening: Vec<(u64, f64)>,
}

impl TabMotion {
    /// Track tabs added since the previous frame and expire their animations.
    fn begin_frame(&mut self, current: impl Iterator<Item = u64> + Clone, now: f64, reduce_motion: bool) {
        if !self.known.iter().copied().eq(current.clone()) {
            if self.initialized && !reduce_motion {
                for uid in current.clone().filter(|uid| !self.known.contains(uid)) {
                    self.opening.push((uid, now));
                }
            }
            self.known = current.collect();
        }
        self.initialized = true;
        if reduce_motion {
            self.opening.clear();
        } else {
            let known = &self.known;
            self.opening.retain(|(uid, started)| known.contains(uid) && now - *started < OPEN_TIME);
        }
    }
}

/// The entering tab's progress among `opening` (from [`TabMotion`]), or 1 when it is not animating.
fn open_progress(opening: &[(u64, f64)], uid: u64, now: f64) -> f32 {
    opening.iter().find(|(u, _)| *u == uid).map_or(1.0, |(_, started)| ((now - started) / OPEN_TIME).clamp(0.0, 1.0) as f32)
}

fn divider_rect(x: f32, rect: Rect) -> Rect {
    Rect::from_min_size(pos2(x, rect.min.y), vec2(1.0, rect.height()))
}

struct TabLayout {
    first: usize,
    widths: Vec<f32>,
    overflow: bool,
}

/// Shrink open tabs before moving the excess into the overflow menu.
fn tab_layout(preferred: &[f32], available: f32, active: Option<usize>) -> TabLayout {
    let count = preferred.len();
    let overflow = count as f32 * MIN_TAB_WIDTH > available;
    let visible = if overflow { ((available - OVERFLOW_WIDTH).max(0.0) / MIN_TAB_WIDTH).floor() as usize } else { count }.min(count);
    let first = active.unwrap_or(0).saturating_sub(visible / 2).min(count - visible);
    let mut widths = preferred[first..first + visible].to_vec();
    let budget = (available - if overflow { OVERFLOW_WIDTH } else { 0.0 }).max(0.0);
    let mut excess = (widths.iter().sum::<f32>() - budget).max(0.0);
    while excess > 0.01 {
        let flexible = widths.iter().filter(|w| **w > MIN_TAB_WIDTH + 0.01).count();
        if flexible == 0 {
            break;
        }
        let share = excess / flexible as f32;
        let mut removed = 0.0;
        for width in &mut widths {
            let shrink = (*width - MIN_TAB_WIDTH).max(0.0).min(share);
            *width -= shrink;
            removed += shrink;
        }
        excess -= removed;
    }
    TabLayout { first, widths, overflow }
}

#[derive(Clone)]
struct TabThumbnail {
    texture: egui::TextureHandle,
    background: Color32,
}

#[derive(Clone, Default)]
struct TabThumbnailCache {
    key: Option<(u64, u64, bool)>,
    thumbnail: Option<TabThumbnail>,
}

#[derive(Clone, Copy)]
struct TabDrag {
    uid: u64,
    grab_offset: f32,
}

/// Final document index and insertion marker for a tab dropped among the visible tabs.
fn tab_drop_position(visible: &[(usize, Rect)], from: usize, pointer_x: f32) -> Option<(usize, f32)> {
    let moving = visible.iter().find(|(index, _)| *index == from)?.1;
    let others: Vec<_> = visible.iter().copied().filter(|(index, _)| *index != from).collect();
    let slot = others.partition_point(|(index, rect)| {
        let boundary = if *index < from { rect.center().x + moving.width() * 0.5 } else { rect.center().x - moving.width() * 0.5 };
        boundary <= pointer_x
    });
    if let Some(&(index, rect)) = others.get(slot) {
        Some((index - usize::from(from < index), rect.min.x))
    } else {
        let &(index, rect) = others.last()?;
        Some((index + usize::from(from > index), rect.max.x))
    }
}

pub fn tab_bar(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("tab_bar").exact_size(HEIGHT).frame(egui::Frame::NONE.fill(t.tab_bar)).show(ui, |ui| {
        let rect = ui.max_rect();
        let buttons_w = if app.ui.title_bar == TitleBar::Custom { window_buttons(app, ui, rect) } else { 0.0 };
        let mut x = rect.min.x + if app.ui.title_bar == TitleBar::Inset { 76.0 } else { 8.0 };
        // Home.
        let hr = Rect::from_min_size(pos2(x, rect.min.y), vec2(44.0, HEIGHT));
        let hresp = ui.interact(hr, ui.id().with("home"), Sense::click()).tip(tr("Home"));
        let home_active = (app.ui.home || !app.has_files()) && app.ui.present.is_none();
        crate::a11y::tab(&hresp, tr("Home"), home_active);
        if home_active {
            ui.painter().rect_filled(hr, CornerRadius::ZERO, t.tab_active);
        } else if hresp.hovered() {
            ui.painter().rect_filled(hr, CornerRadius::ZERO, t.hover);
        }
        ui.painter().rect_filled(divider_rect(hr.min.x, hr), CornerRadius::ZERO, t.border_strong);
        icons::paint(ui.painter(), Rect::from_center_size(hr.center(), vec2(16.0, 16.0)), Icon::Home, t.icon);
        if hresp.clicked() {
            if let Some(p) = app.ui.present.take() {
                app.ui.paused_present = Some(p);
            }
            app.ui.home = true;
        }
        x += 44.0;
        ui.painter().rect_filled(divider_rect(x, rect), CornerRadius::ZERO, t.border_strong);
        let document_tabs: Vec<usize> = app.session.docs.iter().enumerate().filter_map(|(i, d)| app.shows(d.uid).then_some(i)).collect();
        let now = ui.input(|input| input.time);
        let motion_id = ui.id().with("tab_motion");
        let current_uids = document_tabs.iter().filter_map(|&i| app.session.docs.get(i).map(|d| d.uid));
        let reduce_motion = app.reduce_motion();
        // Cloning `opening` is free while nothing animates (an empty Vec).
        let opening = ui.ctx().data_mut(|data| {
            let motion = data.get_temp_mut_or_default::<TabMotion>(motion_id);
            motion.begin_frame(current_uids, now, reduce_motion);
            motion.opening.clone()
        });
        let preferred: Vec<f32> = document_tabs
            .iter()
            .map(|&i| {
                let d = &app.session.docs[i];
                let mark_w = if live_mark(app, d.uid).is_some() { 18.0 } else { 0.0 };
                let dot_w = if d.is_dirty() && mark_w > 0.0 { 12.0 } else { 0.0 };
                (regular(12.0).size * 0.6 * d.doc.name.chars().count() as f32 + 64.0 + mark_w + dot_w).clamp(120.0, 220.0)
            })
            .collect();
        // Leave the new-tab button, theme toggle, window controls and a draggable gap free.
        let present_w = if app.ui.present.is_some() || app.ui.paused_present.is_some() { 128.0 } else { 0.0 };
        let available = (rect.max.x - buttons_w - 70.0 - x - 34.0 - present_w).max(0.0);
        let active_tab = document_tabs.iter().position(|&i| i == app.session.active);
        let layout = tab_layout(&preferred, available, active_tab);
        let mut target_tab_rects = Vec::with_capacity(layout.widths.len());
        let mut tab_rects = Vec::with_capacity(layout.widths.len());
        let mut tab_x = x;
        for (slot, &width) in layout.widths.iter().enumerate() {
            let target = Rect::from_min_size(pos2(tab_x, rect.min.y), vec2(width, HEIGHT));
            target_tab_rects.push(target);
            let uid = app.session.docs[document_tabs[layout.first + slot]].uid;
            let animated_x = ui.ctx().animate_value_with_time(ui.id().with(("tab_position", uid)), tab_x, 0.14);
            tab_rects.push(Rect::from_min_size(pos2(animated_x, rect.min.y), vec2(width, HEIGHT)));
            tab_x += width;
        }
        let mut switch = None;
        let mut close = None;
        let mut close_many = None;
        // A tab dragged off the strip, or "Open in new window", moves to a window of its own.
        let mut detach = None;
        let mut dragging = None;
        let mut dropped = None;
        let mut drag_overlay = None;
        let mut preview = None;
        let mut rename_commit = None;
        let drag_id = ui.id().with("tab_drag");
        let mut tab_drag = ui.ctx().data(|data| data.get_temp::<TabDrag>(drag_id));
        let mut tab_drag_active = false;
        let several = !ui.ctx().embed_viewports();
        for (slot, &w) in layout.widths.iter().enumerate() {
            let i = document_tabs[layout.first + slot];
            let d = &app.session.docs[i];
            let active = i == app.session.active && !app.ui.home && app.ui.present.is_none();
            let icon_color = if active { t.accent } else { t.icon_secondary };
            let text_color = if active { t.text } else { t.text_secondary };
            let name = &d.doc.name;
            // In a live session: a mark after the name says whether others get in.
            let live = live_mark(app, d.uid);
            let mark_w = if live.is_some() { 18.0 } else { 0.0 };
            let dirty = d.is_dirty();
            let r = tab_rects[slot];
            let body_r = Rect::from_min_max(r.min, pos2(r.max.x - 32.0, r.max.y));
            let close_r = Rect::from_min_max(body_r.right_top(), r.max);
            let resp = ui.interact(body_r, ui.id().with(("tab", d.uid)), Sense::click_and_drag());
            let cresp = ui.interact(close_r, ui.id().with(("tabclose", d.uid)), Sense::click()).tip(tr("Close tab"));
            if resp.drag_started()
                && let Some(pointer) = ui.input(|input| input.pointer.press_origin().or(input.pointer.interact_pos()))
            {
                tab_drag = Some(TabDrag { uid: d.uid, grab_offset: pointer.x - r.min.x });
                tab_drag_active = true;
            }
            if (resp.dragged() || resp.drag_stopped())
                && let Some(p) = ui.input(|inp| inp.pointer.latest_pos())
            {
                tab_drag_active = true;
                if resp.drag_stopped() {
                    if several && (p.y - rect.center().y).abs() > HEIGHT * 2.0 {
                        detach = Some((i, Some(p)));
                    } else if (rect.min.y - 12.0..=rect.max.y + 12.0).contains(&p.y)
                        && let Some(drag) = tab_drag.filter(|drag| drag.uid == d.uid)
                    {
                        dropped = Some((i, p.x - drag.grab_offset + r.width() * 0.5));
                    }
                } else {
                    dragging = Some((i, p));
                }
            }
            if resp.hovered() && !active && !resp.clicked() && !resp.dragged() && !cresp.hovered() && app.ui.rename_tab != Some(d.uid) {
                preview = Some((d.uid, r));
            }
            crate::right::context_popup(&resp).width(220.0).show(|ui| {
                ui.set_width(220.0);
                if crate::menus::item(ui, tr("Rename File"), "", true, false, &t) {
                    app.ui.rename_tab = Some(d.uid);
                    switch = Some(i);
                    ui.close();
                }
                if several && crate::menus::item(ui, tr("Open in new window"), "", true, false, &t) {
                    detach = Some((i, None));
                    ui.close();
                }
                ui.separator();
                if crate::menus::item(ui, tr("Close"), "", true, false, &t) {
                    close = Some(i);
                    ui.close();
                }
                if crate::menus::item(ui, tr("Close Other Tabs"), "", document_tabs.len() > 1, false, &t) && document_tabs.len() > 1 {
                    close_many = Some(Some(d.uid));
                    ui.close();
                }
                if crate::menus::item(ui, tr("Close All Tabs"), "", true, false, &t) {
                    close_many = Some(None);
                    ui.close();
                }
            });
            let label = match live {
                Some(true) => trf("{name}, in a live session", &[("name", name)]),
                Some(false) => trf("{name}, online, open only to you", &[("name", name)]),
                None => name.clone(),
            };
            let resp = if live.is_some() { resp.on_hover_text(&label) } else { resp };
            crate::a11y::tab(&resp, &label, active);
            let is_dragging = dragging.is_some_and(|(index, _)| index == i);
            let mut paint_r = r;
            if is_dragging && let Some((_, p)) = dragging {
                let left = tab_drag.filter(|drag| drag.uid == d.uid).map_or(p.x - r.width() * 0.5, |drag| p.x - drag.grab_offset);
                paint_r = r.translate(vec2(left - r.min.x, 0.0));
                drag_overlay = Some((paint_r, name.clone(), dirty, icon_color, text_color));
            }
            let editing_tab = app.ui.rename_tab == Some(d.uid);
            let name_start = (resp.double_clicked() || resp.triple_clicked())
                && ui.input(|input| input.pointer.interact_pos()).is_some_and(|p| {
                    let label_rect = Rect::from_min_max(pos2(r.min.x + 32.0, r.min.y), pos2(r.max.x - 40.0, r.max.y));
                    label_rect.contains(p)
                });
            if name_start {
                app.ui.rename_tab = Some(d.uid);
            }
            let renaming = editing_tab || name_start;
            let progress = open_progress(&opening, d.uid, now);
            if !is_dragging {
                paint_r = r.translate(vec2(-28.0 * (1.0 - progress).powi(2), 0.0));
            }
            let painter = ui.painter().with_clip_rect(if is_dragging { rect } else { r });
            if !is_dragging {
                let fill = if active {
                    t.tab_active
                } else if resp.hovered() || cresp.hovered() {
                    t.hover
                } else {
                    t.tab_bar
                };
                painter.rect_filled(paint_r, CornerRadius::ZERO, fill.linear_multiply(progress));
                painter.rect_stroke(paint_r, CornerRadius::ZERO, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
                icons::paint(
                    &painter,
                    Rect::from_center_size(pos2(paint_r.min.x + 18.0, paint_r.center().y), vec2(14.0, 14.0)),
                    Icon::Logo,
                    icon_color.linear_multiply(progress),
                );
                let name_rect = Rect::from_min_max(pos2(paint_r.min.x + 32.0, paint_r.min.y + 3.0), pos2(paint_r.max.x - 40.0, paint_r.max.y - 3.0));
                if renaming {
                    let field_id = ui.id().with(("tab_name", d.uid));
                    let start = name_start || (editing_tab && !ui.memory(|memory| memory.has_focus(field_id.with("te"))));
                    let (renamed, open) = crate::widgets::rename_field(ui, field_id, name, name_rect, regular(12.0), start);
                    if let Some(name) = renamed {
                        app.ui.rename_tab = None;
                        rename_commit = Some((d.uid, name));
                    } else if !open {
                        app.ui.rename_tab = None;
                    }
                } else {
                    let shown = crate::bidi::paint_name_faded(
                        &painter,
                        pos2(paint_r.min.x + 32.0, paint_r.center().y),
                        name,
                        regular(12.0),
                        text_color.linear_multiply(progress),
                        tab_name_width(w, mark_w, dirty, resp.hovered() || cresp.hovered() || active),
                        fill.linear_multiply(progress),
                    );
                    // The unsaved-changes dot follows the name on screen, whichever way the name reads.
                    if dirty {
                        painter.text(
                            pos2(shown.max.x + 4.0, paint_r.center().y),
                            Align2::LEFT_CENTER,
                            "•",
                            regular(12.0),
                            text_color.linear_multiply(progress),
                        );
                    }
                }
                if let Some(shared) = live {
                    let at = Rect::from_center_size(pos2(paint_r.max.x - 38.0, paint_r.center().y), vec2(14.0, 14.0));
                    icons::paint(
                        &painter,
                        at,
                        crate::online::mark(shared),
                        if shared { t.success } else { t.icon_secondary }.linear_multiply(progress),
                    );
                }
                let cr = Rect::from_center_size(close_r.center() + vec2(paint_r.min.x - r.min.x, 0.0), vec2(18.0, 18.0));
                // The close target never overlaps the draggable body, including on inactive tabs.
                if resp.hovered() || cresp.hovered() || active {
                    if cresp.hovered() {
                        painter.rect_filled(cr, CornerRadius::same(4), t.pressed.linear_multiply(progress));
                    }
                    icons::paint(&painter, cr.shrink(4.0), Icon::Close, t.icon_secondary.linear_multiply(progress));
                }
            }
            if cresp.clicked() {
                close = Some(i);
            }
            if resp.clicked() && close.is_none() {
                switch = Some(i);
            }
            if resp.middle_clicked() {
                close = Some(i);
            }
            x += w;
            ui.painter().line_segment([pos2(x, rect.min.y + 10.0), pos2(x, rect.max.y - 10.0)], Stroke::new(1.0, t.border));
        }
        let visible: Vec<(usize, Rect)> =
            target_tab_rects.iter().copied().enumerate().map(|(slot, r)| (document_tabs[layout.first + slot], r)).collect();
        if let Some(drag) = tab_drag {
            let pointer_down = ui.input(|input| input.pointer.primary_down());
            if pointer_down {
                tab_drag_active = true;
                if let (Some(from), Some(p)) =
                    (app.session.docs.iter().position(|doc| doc.uid == drag.uid), ui.input(|input| input.pointer.latest_pos()))
                {
                    dragging = Some((from, p));
                }
            } else {
                tab_drag_active = false;
                if dropped.is_none()
                    && detach.is_none()
                    && let (Some(from), Some(p)) =
                        (app.session.docs.iter().position(|doc| doc.uid == drag.uid), ui.input(|input| input.pointer.latest_pos()))
                    && (rect.min.y - 12.0..=rect.max.y + 12.0).contains(&p.y)
                    && let Some((_, tab_rect)) = visible.iter().find(|(index, _)| *index == from)
                {
                    dropped = Some((from, p.x - drag.grab_offset + tab_rect.width() * 0.5));
                }
            }
        }
        if tab_drag_active {
            if let Some(drag) = tab_drag {
                ui.ctx().data_mut(|data| data.insert_temp(drag_id, drag));
            }
        } else {
            ui.ctx().data_mut(|data| data.remove::<TabDrag>(drag_id));
        }
        if let Some((uid, name)) = rename_commit {
            let active_uid = app.session.docs.get(app.session.active).map(|doc| doc.uid);
            if let Some(index) = app.session.docs.iter().position(|doc| doc.uid == uid) {
                if active_uid != Some(uid) {
                    app.run("doc.switch", json!({"index": index}));
                }
                app.run("file.rename", json!({"name": name}));
                if active_uid != Some(uid)
                    && let Some(index) = active_uid.and_then(|active| app.session.docs.iter().position(|doc| doc.uid == active))
                {
                    app.run("doc.switch", json!({"index": index}));
                }
            }
        }
        let live_reorder = dragging.and_then(|(from, p)| {
            if !(rect.min.y - 12.0..=rect.max.y + 12.0).contains(&p.y) {
                return None;
            }
            let drag = tab_drag.filter(|drag| app.session.docs.get(from).is_some_and(|doc| doc.uid == drag.uid))?;
            let center = visible.iter().find(|(index, _)| *index == from).map_or(p.x, |(_, r)| p.x - drag.grab_offset + r.width() * 0.5);
            tab_drop_position(&visible, from, center).filter(|(to, _)| *to != from).map(|(to, _)| (from, to))
        });
        let reorder = live_reorder.or_else(|| dropped.and_then(|(from, center)| tab_drop_position(&visible, from, center).map(|(to, _)| (from, to))));
        if let Some((r, name, dirty, icon_color, text_color)) = drag_overlay {
            let painter = ui.painter().with_clip_rect(rect);
            painter.rect_filled(r.translate(vec2(0.0, 3.0)), CornerRadius::same(4), t.shadow);
            painter.rect_filled(r.shrink(1.0), CornerRadius::same(4), t.tab_active);
            painter.rect_stroke(r.shrink(1.0), CornerRadius::same(4), Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
            icons::paint(&painter, Rect::from_center_size(pos2(r.min.x + 18.0, r.center().y), vec2(14.0, 14.0)), Icon::Logo, icon_color);
            let shown = crate::bidi::paint_name_faded(
                &painter,
                pos2(r.min.x + 32.0, r.center().y),
                &name,
                regular(12.0),
                text_color,
                tab_name_width(r.width(), 0.0, dirty, false),
                t.tab_active,
            );
            if dirty {
                painter.text(pos2(shown.max.x + 4.0, r.center().y), Align2::LEFT_CENTER, "•", regular(12.0), text_color);
            }
        }
        if let Some((uid, tab_rect)) = preview
            && let Some(doc) = app.session.docs.iter().find(|doc| doc.uid == uid)
        {
            let key = (doc.uid, doc.revision, t.dark);
            let cache_id = ui.id().with("tab_thumbnail_cache");
            let thumbnail = ui.ctx().data(|data| data.get_temp::<TabThumbnailCache>(cache_id)).unwrap_or_default();
            let thumbnail = if thumbnail.key == Some(key) {
                thumbnail.thumbnail
            } else {
                let rendered = tab_thumbnail(ui.ctx(), &doc.doc, doc.uid, doc.revision, t.canvas);
                ui.ctx().data_mut(|data| data.insert_temp(cache_id, TabThumbnailCache { key: Some(key), thumbnail: rendered.clone() }));
                rendered
            };
            if let Some(thumbnail) = thumbnail {
                let screen = ui.ctx().content_rect();
                let preview_size = vec2(352.0, 246.0);
                let preview_pos = pos2(
                    (tab_rect.center().x - preview_size.x * 0.5).clamp(screen.min.x + 8.0, screen.max.x - preview_size.x - 8.0),
                    rect.max.y + 8.0,
                );
                egui::Area::new(egui::Id::new(("tab_preview", uid))).order(egui::Order::Tooltip).fixed_pos(preview_pos).interactable(false).show(
                    ui.ctx(),
                    |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            let image_rect = ui.allocate_exact_size(vec2(320.0, 180.0), Sense::hover()).0;
                            ui.painter().rect_filled(image_rect, CornerRadius::same(4), thumbnail.background);
                            let image_size = thumbnail.texture.size_vec2();
                            let fitted = (image_rect.width() / image_size.x).min(image_rect.height() / image_size.y);
                            let image_rect = Rect::from_center_size(image_rect.center(), image_size * fitted);
                            ui.painter().image(
                                thumbnail.texture.id(),
                                image_rect,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(&doc.doc.name).font(regular(14.0)).color(t.text));
                        });
                    },
                );
            }
        }
        if layout.overflow {
            let r = Rect::from_min_size(pos2(x, rect.min.y), vec2(OVERFLOW_WIDTH, HEIGHT));
            let resp = ui.interact(r, ui.id().with("tab_overflow"), Sense::click()).tip(tr("Open files"));
            crate::a11y::menu_button(&resp, tr("Open files"), crate::a11y::menu_open(&resp));
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
            }
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), Icon::Chevron, t.icon);
            crate::right::menu_popup(&resp).show(|ui| {
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for &i in &document_tabs {
                        let Some(d) = app.session.docs.get(i) else { continue };
                        let on = i == app.session.active && !app.ui.home && app.ui.present.is_none();
                        let label = if d.is_dirty() { format!("{} •", d.doc.name) } else { d.doc.name.clone() };
                        if crate::right::menu_check(ui, on, &label, "") {
                            switch = Some(i);
                            ui.close();
                        }
                    }
                });
            });
            x += OVERFLOW_WIDTH;
        }
        // Presentations live in their own tab. Switching back to the design keeps the player
        // state (current screen, navigation history and scroll position) for the return trip.
        if app.ui.present.is_some() || app.ui.paused_present.is_some() {
            let w = 128.0;
            let r = Rect::from_min_size(pos2(x, rect.min.y), vec2(w, HEIGHT));
            let body_r = Rect::from_min_max(r.min, pos2(r.max.x - 32.0, r.max.y));
            let close_r = Rect::from_min_max(body_r.right_top(), r.max);
            let resp = ui.interact(body_r, ui.id().with("presentation_tab"), Sense::click()).tip(tr("Present"));
            let close_resp = ui.interact(close_r, ui.id().with("presentation_close"), Sense::click()).tip(tr("Close preview"));
            let active = app.ui.present.is_some();
            crate::a11y::tab(&resp, tr("Present"), active);
            if active {
                ui.painter().rect_filled(r, CornerRadius::ZERO, t.tab_active);
            } else if resp.hovered() || close_resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
            }
            icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 19.0, r.center().y), vec2(14.0, 14.0)), Icon::Play, t.icon);
            ui.painter().text(pos2(x + 34.0, r.center().y), Align2::LEFT_CENTER, tr("Present"), regular(12.0), t.text);
            let cr = Rect::from_center_size(close_r.center(), vec2(18.0, 18.0));
            if resp.hovered() || close_resp.hovered() || active {
                if close_resp.hovered() {
                    ui.painter().rect_filled(cr, CornerRadius::same(4), t.pressed);
                }
                icons::paint(ui.painter(), cr.shrink(4.0), Icon::Close, t.icon_secondary);
            }
            if close_resp.clicked() {
                app.ui.present = None;
                app.ui.paused_present = None;
                app.ui.present_uid = None;
            } else if resp.clicked() && !active {
                resume_presentation(app);
            }
            x += w;
            ui.painter().line_segment([pos2(x, rect.min.y + 10.0), pos2(x, rect.max.y - 10.0)], Stroke::new(1.0, t.border));
        }
        // New tab.
        let nr = Rect::from_center_size(pos2(x + 20.0, rect.center().y), vec2(28.0, 28.0));
        let nresp = ui.interact(nr, ui.id().with("newtab"), Sense::click()).tip(crate::shortcuts::tip(tr("New design file"), "file.new"));
        if nresp.hovered() {
            ui.painter().rect_filled(nr, CornerRadius::same(5), t.hover);
        }
        icons::paint(ui.painter(), nr.shrink(7.0), Icon::Plus, t.icon);
        if nresp.clicked() {
            if let Some(p) = app.ui.present.take() {
                app.ui.paused_present = Some(p);
            }
            app.run("file.new", json!({}));
            app.ui.home = false;
        }
        // Right side: theme toggle (left of the window buttons).
        let theme_r = Rect::from_center_size(pos2(rect.max.x - buttons_w - 24.0, rect.center().y), vec2(28.0, 28.0));
        let tresp = ui.interact(theme_r, ui.id().with("theme"), Sense::click()).tip(tr("Toggle light/dark theme"));
        if tresp.hovered() {
            ui.painter().rect_filled(theme_r, CornerRadius::same(5), t.hover);
        }
        icons::paint(ui.painter(), theme_r.shrink(7.0), if app.ui.dark { Icon::Sun } else { Icon::Moon }, t.icon);
        if tresp.clicked() {
            let cmd = if app.ui.dark { "view.themeLight" } else { "view.themeDark" };
            app.run(cmd, json!({}));
        }
        // Only the free space drags the window. A full-bar drag target steals tab drags.
        let tab_press_origin = ui.input(|input| input.pointer.press_origin()).is_some_and(|origin| visible.iter().any(|(_, r)| r.contains(origin)));
        if app.ui.title_bar != TitleBar::Native && !tab_drag_active && !tab_press_origin && x + 34.0 < theme_r.min.x {
            let free = Rect::from_min_max(pos2(x + 34.0, rect.min.y), pos2(theme_r.min.x, rect.max.y));
            let drag = ui.interact(free, ui.id().with("titlebar_drag"), Sense::click_and_drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let max = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
        }
        if let Some(i) = switch {
            if let Some(p) = app.ui.present.take() {
                app.ui.paused_present = Some(p);
            }
            app.run("doc.switch", json!({"index": i}));
            app.ui.home = false;
        }
        if let Some((i, at)) = detach {
            // Where the pointer let go, on the screen.
            let at = at.and_then(|p| ui.input(|inp| inp.viewport().inner_rect).map(|r| r.min + p.to_vec2() - vec2(60.0, HEIGHT / 2.0)));
            if let Err(e) = crate::windows::open(app, ui.ctx(), Some(i), None, at) {
                app.toast(e);
            }
        }
        if let Some((from, to)) = reorder
            && from != to
        {
            app.run("file.moveTab", json!({"from": from, "to": to}));
        }
        if let Some(i) = close {
            // A branch tab's edits live on in its main file: closing it loses nothing.
            let dirty = app.session.docs.get(i).is_some_and(|d| d.is_dirty() && d.branch_of.is_none());
            if dirty {
                app.ui.find = None;
                app.toast(trf(
                    "Unsaved changes: save first, or close again with {key} to discard",
                    &[("key", &crate::toolbar::pretty_shortcut("Alt"))],
                ));
                if ui.input(|inp| inp.modifiers.alt) {
                    app.run("file.close", json!({"index": i, "force": true}));
                }
            } else {
                app.run("file.close", json!({"index": i}));
            }
        }
        if let Some(keep_uid) = close_many
            && close_tabs(app, keep_uid, ui.input(|input| input.modifiers.alt))
        {
            app.toast(trf("Unsaved changes: save first, or close again with {key} to discard", &[("key", &crate::toolbar::pretty_shortcut("Alt"))]));
        }
        if app.ui.present_uid.is_some_and(|uid| !app.session.docs.iter().any(|d| d.uid == uid)) {
            app.ui.present = None;
            app.ui.paused_present = None;
            app.ui.present_uid = None;
        }
        if !opening.is_empty() {
            ui.ctx().request_repaint();
        }
    });
}

fn close_tabs(app: &mut FigApp, keep_uid: Option<u64>, discard: bool) -> bool {
    if let Some(uid) = keep_uid
        && let Some(index) = app.session.docs.iter().position(|doc| doc.uid == uid && app.shows(doc.uid))
    {
        app.run("doc.switch", json!({"index": index}));
        app.ui.home = false;
    }

    let close_uids: Vec<u64> =
        app.session.docs.iter().rev().filter(|doc| Some(doc.uid) != keep_uid && app.shows(doc.uid)).map(|doc| doc.uid).collect();
    let mut unsaved = false;
    for uid in close_uids {
        let Some(index) = app.session.docs.iter().position(|doc| doc.uid == uid) else { continue };
        let dirty = app.session.docs.get(index).is_some_and(|doc| doc.is_dirty() && doc.branch_of.is_none());
        if dirty && !discard {
            unsaved = true;
            continue;
        }
        app.run("file.close", json!({"index": index, "force": dirty}));
    }
    if keep_uid.is_none() && !app.session.docs.iter().any(|doc| app.shows(doc.uid)) {
        app.ui.home = true;
    }
    unsaved
}

fn tab_thumbnail(ctx: &egui::Context, doc: &figcraft_doc::Document, uid: u64, revision: u64, background: Color32) -> Option<TabThumbnail> {
    let page = doc.node(doc.first_page()?)?;
    let layers: Vec<_> = page.children.iter().copied().take(8).collect();
    let bounds = layers.iter().filter_map(|id| figcraft_render::export_bounds(doc, *id)).reduce(|a, b| a.union(b))?;
    if !(bounds.width() > 0.0 && bounds.height() > 0.0) {
        return None;
    }
    let area = figcraft_geom::Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1);
    let max_size = vec2(320.0, 180.0);
    let scale = (f64::from(max_size.x) / area.width()).min(f64::from(max_size.y) / area.height()) * f64::from(ctx.pixels_per_point());
    let pixels = figcraft_render::render_area(doc, &layers, area, scale.min(1.0))?;
    let image = crate::render_worker::screen_image(&pixels, doc.color_profile);
    let texture = ctx.load_texture(format!("tab-thumb-{uid}-{revision}"), image, egui::TextureOptions::LINEAR);
    Some(TabThumbnail { texture, background })
}

fn resume_presentation(app: &mut FigApp) {
    let Some(uid) = app.ui.present_uid else { return };
    let Some(index) = app.session.docs.iter().position(|d| d.uid == uid) else {
        app.ui.paused_present = None;
        app.ui.present_uid = None;
        return;
    };
    app.run("doc.switch", json!({"index": index}));
    app.ui.home = false;
    app.ui.present = app.ui.paused_present.take();
}

/// What the tab of document `uid` says about its live session: others get in (`true`: a session
/// this app hosts or joined, a file shared online), only you do (`false`: a file of yours on a
/// server, not shared now), or it has none.
pub fn live_mark(app: &FigApp, uid: u64) -> Option<bool> {
    let live = app.session.live_of(uid);
    live.shared().filter(|_| live.doc_uid == Some(uid))
}

/// Minimise, maximise/restore and close at the right end of the title bar; returns their width.
fn window_buttons(app: &mut FigApp, ui: &mut Ui, bar: Rect) -> f32 {
    const W: f32 = 46.0;
    let t = Tokens::get(ui.ctx());
    let max = ui.input(|i| i.viewport().maximized.unwrap_or(false));
    let buttons = [
        (Icon::WinMinimize, tr("Minimize"), egui::ViewportCommand::Minimized(true)),
        (
            if max { Icon::WinRestore } else { Icon::WinMaximize },
            if max { tr("Restore") } else { tr("Maximize") },
            egui::ViewportCommand::Maximized(!max),
        ),
        (Icon::Close, tr("Close"), egui::ViewportCommand::Close),
    ];
    for (k, (icon, tip, cmd)) in buttons.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(bar.max.x - W * (3 - k) as f32, bar.min.y), vec2(W, bar.height()));
        let resp = ui.interact(r, ui.id().with(("window_button", k)), Sense::click()).tip(tip);
        let close = k == 2;
        let mut col = t.icon;
        if resp.hovered() {
            // Closing turns the button red, the others take the hover tint.
            let fill = if close { Color32::from_rgb(0xE8, 0x11, 0x23) } else { t.hover };
            ui.painter().rect_filled(r, CornerRadius::ZERO, fill);
            if close {
                col = Color32::WHITE;
            }
        }
        icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), icon, col);
        if resp.clicked() {
            if close {
                crate::windows::request_close(app, ui.ctx());
            } else {
                ui.ctx().send_viewport_cmd(cmd);
            }
        }
    }
    W * 3.0
}

/// Invisible strips along a frameless window's edges and corners that resize it.
pub fn resize_edges(ctx: &egui::Context) {
    use egui::{CursorIcon, ResizeDirection as D};
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    const E: f32 = 5.0;
    let s = ctx.content_rect();
    let (x0, y0, x1, y1) = (s.min.x, s.min.y, s.max.x, s.max.y);
    let zones = [
        (Rect::from_min_max(pos2(x0 + E, y0), pos2(x1 - E, y0 + E)), D::North, CursorIcon::ResizeNorth),
        (Rect::from_min_max(pos2(x0 + E, y1 - E), pos2(x1 - E, y1)), D::South, CursorIcon::ResizeSouth),
        (Rect::from_min_max(pos2(x0, y0 + E), pos2(x0 + E, y1 - E)), D::West, CursorIcon::ResizeWest),
        (Rect::from_min_max(pos2(x1 - E, y0 + E), pos2(x1, y1 - E)), D::East, CursorIcon::ResizeEast),
        (Rect::from_min_max(pos2(x0, y0), pos2(x0 + E, y0 + E)), D::NorthWest, CursorIcon::ResizeNorthWest),
        (Rect::from_min_max(pos2(x1 - E, y0), pos2(x1, y0 + E)), D::NorthEast, CursorIcon::ResizeNorthEast),
        (Rect::from_min_max(pos2(x0, y1 - E), pos2(x0 + E, y1)), D::SouthWest, CursorIcon::ResizeSouthWest),
        (Rect::from_min_max(pos2(x1 - E, y1 - E), pos2(x1, y1)), D::SouthEast, CursorIcon::ResizeSouthEast),
    ];
    egui::Area::new(egui::Id::new("window_resize_edges")).order(egui::Order::Foreground).fixed_pos(s.min).interactable(true).show(ctx, |ui| {
        for (k, (r, dir, cursor)) in zones.into_iter().enumerate() {
            let resp = ui.interact(r, egui::Id::new(("window_edge", k)), Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(cursor);
            }
            if resp.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
            }
        }
    });
}

#[cfg(test)]
mod layout_tests {
    use super::{
        MIN_TAB_WIDTH, OPEN_TIME, OVERFLOW_WIDTH, TabMotion, close_tabs, divider_rect, open_progress, tab_drop_position, tab_layout, tab_name_width,
    };
    use egui::{Rect, pos2};

    #[test]
    fn tabs_shrink_to_fit_before_overflowing() {
        let preferred = vec![120.0; 10];
        let layout = tab_layout(&preferred, 1100.0, Some(5));
        assert!(!layout.overflow);
        assert_eq!(layout.first, 0);
        assert_eq!(layout.widths.len(), preferred.len());
        assert!(layout.widths.iter().all(|&width| (width - 110.0).abs() < 0.01));
    }

    #[test]
    fn excess_tabs_leave_room_for_overflow_and_keep_the_active_tab_visible() {
        let preferred = vec![120.0; 24];
        let available = 1100.0;
        let layout = tab_layout(&preferred, available, Some(23));
        assert!(layout.overflow);
        assert!(layout.first <= 23 && 23 < layout.first + layout.widths.len());
        assert!(layout.widths.iter().all(|&width| width >= MIN_TAB_WIDTH));
        assert!(layout.widths.iter().sum::<f32>() + OVERFLOW_WIDTH <= available + 0.01);
    }

    #[test]
    fn drop_position_places_a_tab_between_its_neighbors() {
        let visible = (0..4)
            .map(|index| (index, Rect::from_min_max(pos2(index as f32 * 120.0, 0.0), pos2((index + 1) as f32 * 120.0, 40.0))))
            .collect::<Vec<_>>();
        assert_eq!(tab_drop_position(&visible, 0, 330.0).map(|(to, _)| to), Some(2));
        assert_eq!(tab_drop_position(&visible, 3, 30.0).map(|(to, _)| to), Some(0));
    }

    #[test]
    fn inactive_unhovered_tabs_use_space_reserved_for_the_close_icon() {
        for dirty in [false, true] {
            let idle = tab_name_width(180.0, 0.0, dirty, false);
            let hovered = tab_name_width(180.0, 0.0, dirty, true);
            assert_eq!(idle - hovered, if dirty { 16.0 } else { 20.0 });
        }
        assert_eq!(tab_name_width(180.0, 18.0, false, false), tab_name_width(180.0, 18.0, false, true));
    }

    #[test]
    fn home_dividers_are_solid_one_point_lines_spanning_the_titlebar() {
        let rect = Rect::from_min_max(pos2(76.0, 0.0), pos2(120.0, 40.0));
        assert_eq!(divider_rect(rect.min.x, rect), Rect::from_min_max(pos2(76.0, 0.0), pos2(77.0, 40.0)));
        assert_eq!(divider_rect(rect.max.x, rect), Rect::from_min_max(pos2(120.0, 0.0), pos2(121.0, 40.0)));
    }

    #[test]
    fn new_tabs_animate_unless_motion_is_reduced() {
        let mut motion = TabMotion::default();
        motion.begin_frame([1].into_iter(), 0.0, false);
        assert!(motion.opening.is_empty(), "the tabs open at start-up don't animate");
        motion.begin_frame([1, 2].into_iter(), 1.0, false);
        assert_eq!(open_progress(&motion.opening, 2, 1.0), 0.0);
        assert_eq!(open_progress(&motion.opening, 1, 1.0), 1.0);
        motion.begin_frame([1, 2].into_iter(), 1.0 + 2.0 * OPEN_TIME, false);
        assert!(motion.opening.is_empty(), "the animation ends");
        motion.begin_frame([1, 2, 3].into_iter(), 2.0, true);
        assert!(motion.opening.is_empty(), "Reduce motion opens tabs at once");
    }

    #[test]
    fn bulk_closing_only_affects_tabs_in_the_current_window() {
        use crate::{FigApp, NoServices};
        use egui::{Context, RawInput};
        use figcraft_engine::Session;
        use serde_json::json;

        fn frame(ctx: &Context, app: &mut FigApp) {
            ctx.run_ui(RawInput::default(), |ui| app.ui(ui)).textures_delta.clear();
        }

        let mut session = Session::empty();
        for name in ["A", "B", "C"] {
            session.execute("file.new", &json!({"name": name})).unwrap();
        }
        let ctx = Context::default();
        ctx.set_embed_viewports(false);
        let mut app = FigApp::new(session, Box::new(NoServices));
        frame(&ctx, &mut app);
        let other_window = crate::control::handle(&mut app, &ctx, "window.new", &json!({"index": 0})).unwrap()["window"].as_u64().unwrap();
        frame(&ctx, &mut app);

        app.windows.current = 0;
        let keep_uid = app.session.docs.iter().find(|doc| doc.doc.name == "B").unwrap().uid;
        assert!(!close_tabs(&mut app, Some(keep_uid), false));
        assert_eq!(app.session.docs.iter().map(|doc| doc.doc.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!(app.session.docs.iter().find(|doc| doc.doc.name == "A").map(|doc| app.windows.of(doc.uid)), Some(other_window));

        app.windows.current = other_window;
        assert!(!close_tabs(&mut app, None, false));
        assert_eq!(app.session.docs.iter().map(|doc| doc.doc.name.as_str()).collect::<Vec<_>>(), ["B"]);
        assert!(app.ui.home);
    }
}
