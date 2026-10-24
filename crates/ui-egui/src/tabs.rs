//! The desktop tab strip: home, one tab per open file, new tab. In a frameless window it is also
//! the title bar: its empty space drags the window (double-click maximises), and it carries the
//! window buttons; the window's edges resize it.

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::NodeId;
use serde_json::json;

use crate::a11y::Tip;
use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::present::{PresentTab, Presenter};
use crate::theme::{Tokens, regular};
use crate::{FigApp, TitleBar};

pub const HEIGHT: f32 = 40.0;
const MIN_TAB_WIDTH: f32 = 96.0;
const OVERFLOW_WIDTH: f32 = 32.0;
const OPEN_TIME: f64 = 0.18;

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

/// Final document index and insertion marker for a tab dropped among the visible tabs.
fn tab_drop_position(visible: &[(usize, Rect)], from: usize, pointer_x: f32) -> Option<(usize, f32)> {
    let others: Vec<_> = visible.iter().copied().filter(|(index, _)| *index != from).collect();
    let slot = others.partition_point(|(_, rect)| rect.center().x < pointer_x);
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
        icons::paint(ui.painter(), Rect::from_center_size(hr.center(), vec2(16.0, 16.0)), Icon::Home, t.icon);
        if hresp.clicked() {
            pause_present(app);
            app.ui.home = true;
        }
        x += 44.0;
        ui.painter().line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], Stroke::new(1.0, t.border));
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
        let room = (rect.max.x - buttons_w - 70.0 - x - 34.0).max(0.0);
        let present_pref: Vec<f32> = app.ui.presents.iter().map(present_tab_width).collect();
        let present_on = app.ui.present.as_ref().and(app.ui.present_active);
        let active_present = present_on.and_then(|id| app.ui.presents.iter().position(|p| p.id == id));
        let present_layout = tab_layout(&present_pref, present_room(room, &present_pref, !document_tabs.is_empty()), active_present);
        let present_used = present_layout.widths.iter().sum::<f32>() + if present_layout.overflow { OVERFLOW_WIDTH } else { 0.0 };
        let available = (room - present_used).max(0.0);
        let active_tab = document_tabs.iter().position(|&i| i == app.session.active);
        let layout = tab_layout(&preferred, available, active_tab);
        let mut tab_rects = Vec::with_capacity(layout.widths.len());
        let mut tab_x = x;
        for &width in &layout.widths {
            tab_rects.push(Rect::from_min_size(pos2(tab_x, rect.min.y), vec2(width, HEIGHT)));
            tab_x += width;
        }
        let mut switch = None;
        let mut close = None;
        // A tab dragged off the strip, or "Open in new window", moves to a window of its own.
        let mut detach = None;
        let mut dragging = None;
        let mut dropped = None;
        let several = !ui.ctx().embed_viewports();
        for (slot, &w) in layout.widths.iter().enumerate() {
            let i = document_tabs[layout.first + slot];
            let d = &app.session.docs[i];
            let active = i == app.session.active && !app.ui.home && app.ui.present.is_none();
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
            if (resp.dragged() || resp.drag_stopped())
                && let Some(p) = ui.input(|inp| inp.pointer.latest_pos())
            {
                if resp.drag_stopped() {
                    if several && (p.y - rect.center().y).abs() > HEIGHT * 2.0 {
                        detach = Some((i, Some(p)));
                    } else if (rect.min.y - 12.0..=rect.max.y + 12.0).contains(&p.y) {
                        dropped = Some((i, p.x));
                    }
                } else {
                    dragging = Some((i, p));
                }
            }
            if several {
                resp.context_menu(|ui| {
                    ui.set_width(200.0);
                    if crate::menus::item(ui, tr("Open in new window"), "", true, false, &t) {
                        detach = Some((i, None));
                        ui.close();
                    }
                });
            }
            let label = match live {
                Some(true) => trf("{name}, in a live session", &[("name", name)]),
                Some(false) => trf("{name}, online, open only to you", &[("name", name)]),
                None => name.clone(),
            };
            let resp = if live.is_some() { resp.on_hover_text(&label) } else { resp };
            crate::a11y::tab(&resp, &label, active);
            let progress = open_progress(&opening, d.uid, now);
            let paint_r = r.translate(vec2(-28.0 * (1.0 - progress).powi(2), 0.0));
            let painter = ui.painter().with_clip_rect(r);
            if active {
                painter.rect_filled(paint_r, CornerRadius::ZERO, t.tab_active.linear_multiply(progress));
            } else if resp.hovered() || cresp.hovered() {
                painter.rect_filled(paint_r, CornerRadius::ZERO, t.hover.linear_multiply(progress));
            }
            icons::paint(
                &painter,
                Rect::from_center_size(pos2(paint_r.min.x + 18.0, paint_r.center().y), vec2(14.0, 14.0)),
                icons::file_mark(d.doc.kind),
                crate::theme::ACCENT.linear_multiply(progress),
            );
            // The unsaved-changes dot follows the name on screen, whichever way the name reads.
            let shown = crate::bidi::paint_name(
                &painter,
                pos2(paint_r.min.x + 32.0, paint_r.center().y),
                name,
                regular(12.0),
                t.text.linear_multiply(progress),
                (w - mark_w - if dirty { 72.0 } else { 60.0 }).max(0.0),
            );
            if let Some(shared) = live {
                let at = Rect::from_center_size(pos2(paint_r.max.x - 38.0, paint_r.center().y), vec2(14.0, 14.0));
                icons::paint(&painter, at, crate::online::mark(shared), if shared { t.success } else { t.icon_secondary }.linear_multiply(progress));
            }
            if dirty {
                painter.text(pos2(shown.max.x + 4.0, paint_r.center().y), Align2::LEFT_CENTER, "•", regular(12.0), t.text.linear_multiply(progress));
            }
            let cr = Rect::from_center_size(close_r.center() + vec2(paint_r.min.x - r.min.x, 0.0), vec2(18.0, 18.0));
            // The close target never overlaps the draggable body, including on inactive tabs.
            if resp.hovered() || cresp.hovered() || active {
                if cresp.hovered() {
                    painter.rect_filled(cr, CornerRadius::same(4), t.pressed.linear_multiply(progress));
                }
                icons::paint(&painter, cr.shrink(4.0), Icon::Close, t.icon_secondary.linear_multiply(progress));
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
        let visible: Vec<(usize, Rect)> = tab_rects.iter().copied().enumerate().map(|(slot, r)| (document_tabs[layout.first + slot], r)).collect();
        if let Some((from, p)) = dragging
            && (rect.min.y - 12.0..=rect.max.y + 12.0).contains(&p.y)
            && let Some((_, at)) = tab_drop_position(&visible, from, p.x)
        {
            ui.painter().line_segment([pos2(at, rect.min.y + 5.0), pos2(at, rect.max.y - 5.0)], Stroke::new(3.0, crate::theme::ACCENT));
        }
        let reorder = dropped.and_then(|(from, at)| tab_drop_position(&visible, from, at).map(|(to, _)| (from, to)));
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
        // Presentations live in their own tabs. Switching back to the design keeps the player
        // state (current screen, navigation history and scroll position) for the return trip.
        let mut to_present = None;
        let mut close_present_tab = None;
        for (tab, &w) in app.ui.presents.iter().skip(present_layout.first).zip(&present_layout.widths) {
            let active = present_on == Some(tab.id);
            let (clicked, closed) = present_tab(ui, &t, tab, Rect::from_min_size(pos2(x, rect.min.y), vec2(w, HEIGHT)), active);
            if clicked && !active {
                to_present = Some(tab.id);
            }
            if closed {
                close_present_tab = Some(tab.id);
            }
            x += w;
            ui.painter().line_segment([pos2(x, rect.min.y + 10.0), pos2(x, rect.max.y - 10.0)], Stroke::new(1.0, t.border));
        }
        if present_layout.overflow {
            let r = Rect::from_min_size(pos2(x, rect.min.y), vec2(OVERFLOW_WIDTH, HEIGHT));
            let resp = ui.interact(r, ui.id().with("present_overflow"), Sense::click()).tip(tr("Open presentations"));
            crate::a11y::menu_button(&resp, tr("Open presentations"), crate::a11y::menu_open(&resp));
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
            }
            icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), Icon::Chevron, t.icon);
            crate::right::menu_popup(&resp).show(|ui| {
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for tab in &app.ui.presents {
                        let on = present_on == Some(tab.id);
                        if crate::right::menu_check(ui, on, &present_tab_name(tab), "") {
                            if !on {
                                to_present = Some(tab.id);
                            }
                            ui.close();
                        }
                    }
                });
            });
            x += OVERFLOW_WIDTH;
        }
        // New tab.
        let nr = Rect::from_center_size(pos2(x + 20.0, rect.center().y), vec2(28.0, 28.0));
        let nresp = ui.interact(nr, ui.id().with("newtab"), Sense::click()).tip(crate::shortcuts::tip(tr("New design file"), "file.new"));
        if nresp.hovered() {
            ui.painter().rect_filled(nr, CornerRadius::same(5), t.hover);
        }
        icons::paint(ui.painter(), nr.shrink(7.0), Icon::Plus, t.icon);
        if nresp.clicked() {
            pause_present(app);
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
        if app.ui.title_bar != TitleBar::Native && x + 34.0 < theme_r.min.x {
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
            pause_present(app);
            app.run("doc.switch", json!({"index": i}));
            app.ui.home = false;
        }
        if let Some(id) = close_present_tab {
            close_present(app, id);
        } else if let Some(id) = to_present {
            show_present(app, id);
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
        if !opening.is_empty() {
            ui.ctx().request_repaint();
        }
    });
}

/// One presentation tab in `r`: whether it was clicked and whether it was closed (its X or a
/// middle click).
fn present_tab(ui: &mut Ui, t: &Tokens, tab: &PresentTab, r: Rect, active: bool) -> (bool, bool) {
    let label = present_tab_name(tab);
    let body_r = Rect::from_min_max(r.min, pos2(r.max.x - 32.0, r.max.y));
    let close_r = Rect::from_min_max(body_r.right_top(), r.max);
    let resp = ui.interact(body_r, ui.id().with(("presentation_tab", tab.id)), Sense::click()).tip(tr("Present"));
    let cresp = ui.interact(close_r, ui.id().with(("presentation_close", tab.id)), Sense::click()).tip(tr("Close preview"));
    crate::a11y::tab(&resp, &label, active);
    let hovered = resp.hovered() || cresp.hovered();
    if active {
        ui.painter().rect_filled(r, CornerRadius::ZERO, t.tab_active);
    } else if hovered {
        ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 19.0, r.center().y), vec2(14.0, 14.0)), Icon::Play, t.icon);
    crate::bidi::paint_name(ui.painter(), pos2(r.min.x + 34.0, r.center().y), &label, regular(12.0), t.text, r.width() - 62.0);
    if hovered || active {
        let cr = Rect::from_center_size(close_r.center(), vec2(18.0, 18.0));
        if cresp.hovered() {
            ui.painter().rect_filled(cr, CornerRadius::same(4), t.pressed);
        }
        icons::paint(ui.painter(), cr.shrink(4.0), Icon::Close, t.icon_secondary);
    }
    let closed = cresp.clicked() || resp.middle_clicked() || cresp.middle_clicked();
    (resp.clicked(), closed)
}

/// The part of the strip's free `room` the presentation tabs may use: all they want, but at most
/// half of it while file tabs share the strip, so they shrink or overflow before covering controls.
fn present_room(room: f32, preferred: &[f32], files: bool) -> f32 {
    preferred.iter().sum::<f32>().min(if files { room / 2.0 } else { room })
}

fn present_tab_width(tab: &PresentTab) -> f32 {
    (regular(12.0).size * 0.6 * present_tab_name(tab).chars().count() as f32 + 64.0).clamp(140.0, 260.0)
}

/// "file - page", or the file alone when the page has no name.
fn present_tab_name(tab: &PresentTab) -> String {
    if tab.page_name.is_empty() { tab.file.clone() } else { format!("{} - {}", tab.file, tab.page_name) }
}

/// The presentation tabs for `ui.inspect`: each one's name and whether it is on screen.
pub fn present_tabs_inspect(app: &FigApp) -> serde_json::Value {
    let on_screen = |id| app.ui.present.is_some() && app.ui.present_active == Some(id);
    app.ui.presents.iter().map(|t| json!({"name": present_tab_name(t), "active": on_screen(t.id)})).collect()
}

/// The presenter on screen, else the paused one of the active presentation tab.
pub fn shown_or_paused(app: &FigApp) -> Option<&Presenter> {
    app.ui.present.as_ref().or_else(|| {
        let id = app.ui.present_active?;
        app.ui.presents.iter().find(|t| t.id == id)?.pres.as_ref()
    })
}

/// The index of the open file with `uid`, when a file tab shows it.
fn owner(app: &FigApp, uid: u64) -> Option<usize> {
    app.session.docs.iter().position(|d| d.uid == uid && app.shows(uid))
}

/// The presentation on screen plays the file in front (or no tab holds it). Only then may it read
/// and change that file.
pub fn present_owner_active(app: &FigApp) -> bool {
    let Some(tab) = app.ui.present_active.and_then(|id| app.ui.presents.iter().find(|t| t.id == id)) else { return true };
    owner(app, tab.doc_uid).is_some_and(|i| i == app.session.active)
}

/// Puts the presenter on screen back into its tab, where it keeps its state.
fn pause_present(app: &mut FigApp) {
    let Some(p) = app.ui.present.take() else { return };
    let id = app.ui.present_active;
    if let Some(t) = app.ui.presents.iter_mut().find(|t| Some(t.id) == id) {
        t.pres = Some(p);
    }
}

/// Presents from `start`: the tab of its page in this file shows again (from `start` when that
/// changed), else a new tab opens.
pub fn open_present(app: &mut FigApp, start: NodeId) {
    let Ok(ds) = app.session.ds() else { return };
    let (uid, path) = (ds.uid, ds.path.clone());
    let page = ds.doc.page_of(start).unwrap_or(ds.page);
    let file = ds.doc.name.clone();
    let page_name = ds.doc.node(page).map(|n| n.name.clone()).unwrap_or_default();
    let mut pres = Presenter::new(start);
    pres.view = app.ui.present_view;
    let same = |t: &&PresentTab| t.page == page && (t.doc_uid == uid || (path.is_some() && t.path == path));
    if let Some(id) = app.ui.presents.iter().find(same).map(|t| t.id) {
        show_present(app, id);
        if app.ui.present.as_ref().is_some_and(|p| p.start() != start) {
            app.ui.present = Some(pres);
        }
        return;
    }
    pause_present(app);
    let id = app.ui.present_next;
    app.ui.present_next += 1;
    app.ui.presents.push(PresentTab { id, pres: None, doc_uid: uid, path, page, file, page_name });
    app.ui.present = Some(pres);
    app.ui.present_active = Some(id);
    app.ui.home = false;
}

/// Brings tab `id` to the front and its file too, when that file is open.
fn show_present(app: &mut FigApp, id: u64) {
    app.ui.home = false;
    if app.ui.present.is_some() && app.ui.present_active == Some(id) {
        return;
    }
    pause_present(app);
    let Some(tab) = app.ui.presents.iter_mut().find(|t| t.id == id) else { return };
    let Some(p) = tab.pres.take() else { return };
    let uid = tab.doc_uid;
    if let Some(i) = owner(app, uid).filter(|i| *i != app.session.active) {
        app.run("doc.switch", json!({"index": i}));
    }
    app.ui.present = Some(p);
    app.ui.present_active = Some(id);
}

/// Closes tab `id`. If it was on screen, its file comes to the front, or Home when it is closed.
pub fn close_present(app: &mut FigApp, id: u64) {
    let Some(k) = app.ui.presents.iter().position(|t| t.id == id) else { return };
    let tab = app.ui.presents.remove(k);
    if app.ui.present_active != Some(id) {
        return;
    }
    app.ui.present_active = None;
    if app.ui.present.take().is_none() {
        return;
    }
    match owner(app, tab.doc_uid) {
        Some(i) => {
            if i != app.session.active {
                app.run("doc.switch", json!({"index": i}));
            }
            app.ui.home = false;
        }
        None => app.ui.home = true,
    }
}

/// A file that opens again has a new uid. Find it by its path to give its tabs back.
pub fn relink_presents(app: &mut FigApp) {
    let docs = &app.session.docs;
    for t in &mut app.ui.presents {
        let found = docs.iter().find(|d| d.uid == t.doc_uid).or_else(|| docs.iter().find(|d| t.path.is_some() && d.path == t.path));
        if let Some(d) = found {
            t.doc_uid = d.uid;
            t.path = d.path.clone();
            t.file = d.doc.name.clone();
            if let Some(n) = d.doc.node(t.page) {
                t.page_name = n.name.clone();
            }
        }
    }
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
    use super::{MIN_TAB_WIDTH, OPEN_TIME, OVERFLOW_WIDTH, TabMotion, open_progress, present_room, tab_drop_position, tab_layout};
    use egui::{Rect, pos2};

    #[test]
    fn many_presentation_tabs_stay_inside_their_share_of_the_strip() {
        let preferred = vec![260.0; 8];
        let room = 900.0;
        let layout = tab_layout(&preferred, present_room(room, &preferred, true), Some(7));
        assert!(layout.overflow);
        assert!(layout.first <= 7 && 7 < layout.first + layout.widths.len());
        assert!(layout.widths.iter().all(|&width| width >= MIN_TAB_WIDTH));
        assert!(layout.widths.iter().sum::<f32>() + OVERFLOW_WIDTH <= room / 2.0 + 0.01);
        assert_eq!(present_room(room, &preferred, false), room);
        assert_eq!(present_room(room, &[200.0], true), 200.0);
    }

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
}
