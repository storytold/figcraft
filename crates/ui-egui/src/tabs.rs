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

pub fn tab_bar(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("tab_bar").exact_size(HEIGHT).frame(egui::Frame::NONE.fill(t.tab_bar)).show(ui, |ui| {
        let rect = ui.max_rect();
        // Empty space moves the window (first, so the tabs and buttons on top take their clicks).
        if app.ui.title_bar != TitleBar::Native {
            let drag = ui.interact(rect, ui.id().with("titlebar_drag"), Sense::click_and_drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let max = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
        }
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
            if let Some(p) = app.ui.present.take() {
                app.ui.paused_present = Some(p);
            }
            app.ui.home = true;
        }
        x += 44.0;
        ui.painter().line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], Stroke::new(1.0, t.border));
        let mut switch = None;
        let mut close = None;
        // A tab dragged off the strip, or "Open in new window", moves to a window of its own.
        let mut detach = None;
        let several = !ui.ctx().embed_viewports();
        for (i, d) in app.session.docs.iter().enumerate().filter(|(_, d)| app.shows(d.uid)) {
            let active = i == app.session.active && !app.ui.home && app.ui.present.is_none();
            let name = d.doc.name.clone();
            // In a live session: a mark after the name says whether others get in.
            let live = live_mark(app, d.uid);
            let mark_w = if live.is_some() { 18.0 } else { 0.0 };
            // (A file in a session is rarely saved here: its unsaved dot gets room beside the mark.)
            let dirty = d.is_dirty();
            let dot_w = if dirty && live.is_some() { 12.0 } else { 0.0 };
            let w = (regular(12.0).size * 0.6 * name.chars().count() as f32 + 64.0 + mark_w + dot_w).clamp(120.0, 220.0);
            let r = Rect::from_min_size(pos2(x, rect.min.y), vec2(w, HEIGHT));
            let resp = ui.interact(r, ui.id().with(("tab", d.uid)), if several { Sense::click_and_drag() } else { Sense::click() });
            if resp.drag_stopped()
                && let Some(p) = ui.input(|inp| inp.pointer.latest_pos())
                && (p.y - rect.center().y).abs() > HEIGHT * 2.0
            {
                detach = Some((i, Some(p)));
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
                Some(true) => trf("{name}, in a live session", &[("name", &name)]),
                Some(false) => trf("{name}, online, open only to you", &[("name", &name)]),
                None => name.clone(),
            };
            let resp = if live.is_some() { resp.on_hover_text(&label) } else { resp };
            crate::a11y::tab(&resp, &label, active);
            if active {
                ui.painter().rect_filled(r, CornerRadius::ZERO, t.tab_active);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
            }
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(r.min.x + 18.0, r.center().y), vec2(14.0, 14.0)),
                Icon::Logo,
                crate::theme::ACCENT,
            );
            // The unsaved-changes dot follows the name on screen, whichever way the name reads.
            let shown = crate::bidi::paint_name(
                ui.painter(),
                pos2(r.min.x + 32.0, r.center().y),
                &name,
                regular(12.0),
                t.text,
                w - mark_w - if dirty { 72.0 } else { 60.0 },
            );
            if let Some(shared) = live {
                let at = Rect::from_center_size(pos2(r.max.x - 38.0, r.center().y), vec2(14.0, 14.0));
                icons::paint(ui.painter(), at, crate::online::mark(shared), if shared { t.success } else { t.icon_secondary });
            }
            if dirty {
                ui.painter().text(pos2(shown.max.x + 4.0, r.center().y), Align2::LEFT_CENTER, "•", regular(12.0), t.text);
            }
            let cr = Rect::from_center_size(pos2(r.max.x - 16.0, r.center().y), vec2(18.0, 18.0));
            if resp.hovered() || active {
                let cresp = ui.interact(cr, ui.id().with(("tabclose", d.uid)), Sense::click()).tip(tr("Close tab"));
                if cresp.hovered() {
                    ui.painter().rect_filled(cr, CornerRadius::same(4), t.pressed);
                }
                icons::paint(ui.painter(), cr.shrink(4.0), Icon::Close, t.icon_secondary);
                if cresp.clicked() {
                    close = Some(i);
                }
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
        // Presentations live in their own tab. Switching back to the design keeps the player
        // state (current screen, navigation history and scroll position) for the return trip.
        if app.ui.present.is_some() || app.ui.paused_present.is_some() {
            let w = 128.0;
            let r = Rect::from_min_size(pos2(x, rect.min.y), vec2(w, HEIGHT));
            let resp = ui.interact(r, ui.id().with("presentation_tab"), Sense::click()).tip(tr("Present"));
            let active = app.ui.present.is_some();
            crate::a11y::tab(&resp, tr("Present"), active);
            if active {
                ui.painter().rect_filled(r, CornerRadius::ZERO, t.tab_active);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
            }
            icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 19.0, r.center().y), vec2(14.0, 14.0)), Icon::Play, t.icon);
            ui.painter().text(pos2(x + 34.0, r.center().y), Align2::LEFT_CENTER, tr("Present"), regular(12.0), t.text);
            let cr = Rect::from_center_size(pos2(r.max.x - 16.0, r.center().y), vec2(18.0, 18.0));
            if resp.hovered() || active {
                let close_resp = ui.interact(cr, ui.id().with("presentation_close"), Sense::click()).tip(tr("Close preview"));
                if close_resp.hovered() {
                    ui.painter().rect_filled(cr, CornerRadius::same(4), t.pressed);
                }
                icons::paint(ui.painter(), cr.shrink(4.0), Icon::Close, t.icon_secondary);
                if close_resp.clicked() {
                    app.ui.present = None;
                    app.ui.paused_present = None;
                    app.ui.present_uid = None;
                } else if resp.clicked() && !active {
                    resume_presentation(app);
                }
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
            if app.ui.present_uid.is_some_and(|uid| !app.session.docs.iter().any(|d| d.uid == uid)) {
                app.ui.present = None;
                app.ui.paused_present = None;
                app.ui.present_uid = None;
            }
        }
        let _ = Color32::TRANSPARENT;
        let _ = Align2::LEFT_TOP;
    });
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
