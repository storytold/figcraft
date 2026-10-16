//! Session restore in the interface (the engine's [`figcraft_engine::restore`] has the tabs): each
//! window's place on screen, state and panels are kept with the preferences as they change, and
//! put back when `session.restore` opened the tabs again. A window comes back where it was only
//! where a screen still shows it ([`place`]).

use egui::{Context, Pos2, Vec2, ViewportCommand, ViewportId};
use figcraft_engine::restore::{Placed, SavedSession, SavedTab, SavedWindow};
use serde_json::{Value, json};

use crate::{FigApp, LeftTab, RightTab, ToastClock, UiState};

/// How much of a window's top strip (where it is grabbed to move it) a screen must show for the
/// window to count as on it: this high, and this wide (or the whole window, when narrower).
const STRIP: f32 = 32.0;
const HOLD: f32 = 120.0;

/// The seconds at most between two writes of a session that keeps changing, and how long after
/// windows were put back a crash still counts as one during the restore.
const WRITE_EVERY: f64 = 1.0;
const SETTLED_AFTER: f64 = 1.0;

/// Where a window is: its outer position and inner size while it is neither maximized nor full
/// screen (screen points), and those states.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub pos: Option<Pos2>,
    pub size: Vec2,
    pub maximized: bool,
    pub fullscreen: bool,
}

impl Default for Geometry {
    fn default() -> Self {
        Self { pos: None, size: Vec2::new(1280.0, 800.0), maximized: false, fullscreen: false }
    }
}

impl Geometry {
    /// Follow the window drawing now. A minimized window says nothing about where it is.
    pub(crate) fn track(&mut self, ctx: &Context) {
        ctx.input(|i| {
            let v = i.viewport();
            self.maximized = v.maximized.unwrap_or(self.maximized);
            self.fullscreen = v.fullscreen.unwrap_or(self.fullscreen);
            if v.minimized == Some(true) || self.maximized || self.fullscreen {
                return;
            }
            self.pos = v.outer_rect.map(|r| r.min).or(self.pos);
            self.size = v.inner_rect.map_or(self.size, |r| r.size());
        });
    }

    fn rect(&self) -> Option<[f32; 4]> {
        self.pos.map(|p| [p.x, p.y, self.size.x, self.size.y])
    }

    /// As saved, fitted to the screens there are now.
    fn of(saved: &SavedWindow, monitors: &[[f32; 4]]) -> Self {
        let rect = saved.rect.map(|r| place(r, saved.monitor, monitors));
        Self {
            pos: rect.map(|r| Pos2::new(r[0], r[1])),
            size: rect.map_or(Self::default().size, |r| Vec2::new(r[2], r[3])),
            maximized: saved.maximized,
            fullscreen: saved.fullscreen,
        }
    }

    pub(crate) fn json(&self, monitors: &[[f32; 4]]) -> Value {
        json!({"rect": self.rect(), "maximized": self.maximized, "fullscreen": self.fullscreen, "monitor": self.rect().and_then(|r| monitor_of(r, monitors))})
    }
}

/// How much of `[x, y, width, height]` rectangles `a` and `b` share: width and height.
fn shared(a: [f32; 4], b: [f32; 4]) -> (f32, f32) {
    let side = |a0: f32, al: f32, b0: f32, bl: f32| ((a0 + al).min(b0 + bl) - a0.max(b0)).max(0.0);
    (side(a[0], a[2], b[0], b[2]), side(a[1], a[3], b[1], b[3]))
}

/// The screen showing most of `rect`.
pub fn monitor_of(rect: [f32; 4], monitors: &[[f32; 4]]) -> Option<[f32; 4]> {
    let area = |m: &[f32; 4]| {
        let (w, h) = shared(rect, *m);
        w * h
    };
    monitors.iter().copied().filter(|m| area(m) > 0.0).max_by(|a, b| area(a).total_cmp(&area(b)))
}

/// Where a window saved at `rect` (`[x, y, width, height]`) on screen `monitor` opens on the
/// screens there are now: where it was while a screen shows enough of its top strip to grab;
/// otherwise moved onto its screen if that is still there, else the nearest one, and no larger
/// than it. With no screens known it stays as saved.
pub fn place(rect: [f32; 4], monitor: Option<[f32; 4]>, monitors: &[[f32; 4]]) -> [f32; 4] {
    let [x, y, w, h] = rect;
    let sane = |m: &&[f32; 4]| m.iter().all(|n| n.is_finite()) && m[2] >= 1.0 && m[3] >= 1.0;
    let monitors: Vec<[f32; 4]> = monitors.iter().filter(sane).copied().collect();
    if monitors.is_empty() || !rect.iter().all(|n| n.is_finite()) {
        return rect;
    }
    let grabbable = |m: &[f32; 4]| {
        let (sw, sh) = shared([x, y, w, STRIP.min(h)], *m);
        sw >= HOLD.min(w) && sh >= (STRIP / 2.0).min(h)
    };
    if monitors.iter().any(grabbable) {
        return rect;
    }
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let away = |m: &[f32; 4]| (m[0] + m[2] / 2.0 - cx).hypot(m[1] + m[3] / 2.0 - cy);
    let same = |m: &&[f32; 4]| monitor.is_some_and(|s| m.iter().zip(s).all(|(a, b)| (a - b).abs() < 1.0));
    let Some(m) = monitors.iter().find(same).or_else(|| monitors.iter().min_by(|a, b| away(a).total_cmp(&away(b)))) else { return rect };
    let (w, h) = (w.min(m[2]), h.min(m[3]));
    [x.clamp(m[0], m[0] + m[2] - w), y.clamp(m[1], m[1] + m[3] - h), w, h]
}

fn left_id(t: LeftTab) -> &'static str {
    match t {
        LeftTab::File => "file",
        LeftTab::Assets => "assets",
    }
}

fn right_id(t: RightTab) -> &'static str {
    match t {
        RightTab::Design => "design",
        RightTab::Prototype => "prototype",
        RightTab::Inspect => "inspect",
    }
}

/// Put a saved window's panels back into `ui`.
fn panels(ui: &mut UiState, saved: &SavedWindow) {
    ui.left_tab = if saved.left_tab == "assets" { LeftTab::Assets } else { LeftTab::File };
    ui.right_tab = match saved.right_tab.as_str() {
        "prototype" => RightTab::Prototype,
        "inspect" => RightTab::Inspect,
        _ => RightTab::Design,
    };
    ui.left_collapsed = saved.left_collapsed;
    if let Some([left, right]) = saved.panel_widths {
        let within = |w: f32, range: std::ops::RangeInclusive<f32>| w.clamp(*range.start(), *range.end());
        (ui.left_width, ui.right_width) = (within(left, crate::LEFT_PANEL), within(right, crate::RIGHT_PANEL));
    }
}

/// The windows and tabs open now (the main window draws: `app.ui` is its own).
pub(crate) fn snapshot(app: &FigApp) -> SavedSession {
    let wins = &app.windows;
    // Online files still on their way back stay tabs of their windows (of the main window, when
    // theirs is gone).
    let joining = |id: u64| app.session.restore.joining().filter(move |(k, _)| wins.restored(*k) == id).map(|(_, tab)| tab.clone());
    let window = |id: u64, geo: &Geometry, ui: &UiState, active: Option<u64>| {
        let open = app.session.docs.iter().filter(|d| wins.of(d.uid) == id).filter_map(|d| Some((d.uid, app.session.saved_tab(d)?)));
        let (uids, mut tabs): (Vec<u64>, Vec<SavedTab>) = open.unzip();
        tabs.extend(joining(id));
        tabs.truncate(figcraft_engine::restore::MAX_TABS);
        SavedWindow {
            rect: geo.rect(),
            maximized: geo.maximized,
            fullscreen: geo.fullscreen,
            monitor: geo.rect().and_then(|r| monitor_of(r, &app.monitors)),
            front: uids.iter().position(|u| Some(*u) == active).unwrap_or(0),
            tabs,
            left_tab: left_id(ui.left_tab).into(),
            right_tab: right_id(ui.right_tab).into(),
            left_collapsed: ui.left_collapsed,
            panel_widths: Some([ui.left_width, ui.right_width]),
        }
    };
    let main = window(0, &wins.main, &app.ui, app.session.ds().ok().map(|d| d.uid));
    let others = wins.list.iter().map(|w| window(w.id, &w.geo, &w.pane.ui, w.active)).filter(|w| !w.tabs.is_empty());
    SavedSession { windows: std::iter::once(main).chain(others).take(figcraft_engine::restore::MAX_WINDOWS).collect() }
}

/// After the main window drew: keep the session with the preferences when it changed (at once,
/// then at most every [`WRITE_EVERY`] seconds while it keeps changing; nothing while it doesn't),
/// and tell the app once a restore has settled.
pub(crate) fn keep(app: &mut FigApp, ctx: &Context) {
    // The browser keeps no preferences: nothing to keep a session in.
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let now = ctx.input(|i| i.time);
    if let Some(at) = app.restore.settles {
        if now >= at {
            app.restore.settles = None;
            app.services.restoring(false);
        } else {
            crate::repaint_in(ctx, at - now);
        }
    }
    // A window on its way out isn't how the app was left.
    if app.ui.quit || app.session.restore.placing.is_some() {
        return;
    }
    let session = snapshot(app);
    if session != app.session.prefs.session {
        app.session.prefs.session = session;
        app.restore.unsaved = true;
    }
    if !app.restore.unsaved {
        return;
    }
    // Not while something is dragged (a panel's edge): the frame it is let go in writes.
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    if now < app.restore.next_write {
        crate::repaint_in(ctx, app.restore.next_write - now);
        return;
    }
    app.restore.unsaved = false;
    app.restore.next_write = now + WRITE_EVERY;
    app.services.save_prefs(&app.session.prefs);
}

/// What the interface remembers between frames about keeping and restoring the session.
#[derive(Default)]
pub struct Restore {
    /// The saved session changed and isn't written yet; when (UI clock) it may be written next.
    unsaved: bool,
    next_write: f64,
    /// When (UI clock) the restore under way counts as done (the app's crash guard goes then).
    settles: Option<f64>,
}

impl FigApp {
    /// Restore the session on offer (the home screen's button): the app's crash guard is up
    /// first, so a crash on the way starts the next run without restoring.
    pub fn restore_last(&mut self) {
        self.services.restoring(true);
        self.run("session.restore", json!({}));
        self.ui.home = !self.has_files();
    }

    /// The last run's session is on offer (this run started without it).
    pub fn restore_offered(&self) -> bool {
        self.session.restore.last.as_ref().is_some_and(|s| s.tabs() > 0)
    }

    /// The saved session and what restoring knows (`ui.inspect`'s `session`).
    pub(crate) fn inspect_session(&self) -> Value {
        json!({
            "saved": self.session.prefs.session,
            "restore": self.session.prefs.restore_session,
            "offered": self.restore_offered(),
            "joining": self.session.restore.joining().count(),
            "monitors": self.monitors,
        })
    }
}

/// The main window's frame begins: put the windows `session.restore` opened the tabs of where
/// they were, and restored online files that arrived since into theirs.
pub(crate) fn begin(app: &mut FigApp, ctx: &Context) {
    if let Some((placed, missing)) = app.session.restore.placing.take() {
        apply(app, ctx, placed);
        if missing > 0 {
            // With what is waiting to be said (documents recovered after a crash), not instead.
            let said = app
                .ui
                .toast
                .take()
                .filter(|(_, shown)| matches!(shown, ToastClock::Raised))
                .map(|(m, _)| {
                    format!(
                        "{m}
"
                    )
                })
                .unwrap_or_default();
            app.toast(said + &crate::i18n::trn("{n} files couldn't be reopened", missing, &[]));
        }
        app.restore.settles = Some(ctx.input(|i| i.time) + SETTLED_AFTER);
    }
    if app.session.restore.joining().next().is_none() {
        return;
    }
    for a in app.session.restore_arrivals() {
        let id = app.windows.restored(a.window);
        crate::windows::adopt(app, id, &[a.uid], a.front.then_some(a.uid));
        ctx.request_repaint_of(crate::windows::viewport(id));
    }
    // One that couldn't be joined says so.
    if let Some(m) = app.session.message.take() {
        app.toast(m);
    }
}

fn apply(app: &mut FigApp, ctx: &Context, placed: Vec<Placed>) {
    // One window (the browser, a screen reader running): every tab stays here.
    let one = ctx.embed_viewports() || crate::a11y::active(ctx);
    let mut any = false;
    // A file opened since (one the app was started with) stays in front.
    let active = app.session.ds().ok().map(|d| d.uid);
    let keep_front = !placed.iter().any(|p| active.is_some_and(|a| p.docs.contains(&a)));
    for w in &mut app.windows.list {
        w.restored = None;
    }
    for (k, p) in placed.iter().enumerate() {
        any |= !p.docs.is_empty();
        let geo = Geometry::of(&p.saved, &app.monitors);
        if k == 0 {
            let (root, now) = (ViewportId::ROOT, app.windows.main);
            let send = |cmd| ctx.send_viewport_cmd_to(root, cmd);
            // A window is placed while it is neither maximized nor full screen: it leaves a
            // state first and enters one last, and one it stays in keeps the place under it.
            if now.fullscreen && !geo.fullscreen {
                send(ViewportCommand::Fullscreen(false));
            }
            if now.maximized && !geo.maximized {
                send(ViewportCommand::Maximized(false));
            }
            let stays = (now.maximized && geo.maximized) || (now.fullscreen && geo.fullscreen);
            if let Some(pos) = geo.pos.filter(|_| !stays) {
                send(ViewportCommand::OuterPosition(pos));
                send(ViewportCommand::InnerSize(geo.size));
            }
            if geo.maximized && !now.maximized {
                send(ViewportCommand::Maximized(true));
            }
            if geo.fullscreen && !now.fullscreen {
                send(ViewportCommand::Fullscreen(true));
            }
            app.windows.main = Geometry { pos: geo.pos.or(app.windows.main.pos), ..geo };
            panels(&mut app.ui, &p.saved);
            app.session.prefs.panel_widths = (app.ui.left_width, app.ui.right_width);
            crate::windows::adopt(app, 0, &p.docs, p.front.filter(|_| !keep_front));
        } else if one || (p.docs.is_empty() && !app.session.restore.joining().any(|(w, _)| w == k)) {
            // Its tabs (none open yet) are the main window's.
            crate::windows::adopt(app, 0, &p.docs, None);
        } else {
            let mut ui = UiState { title_bar: app.ui.title_bar, ..UiState::default() };
            panels(&mut ui, &p.saved);
            let id = crate::windows::push(app, ui, geo);
            if let Some(w) = app.windows.list.last_mut() {
                w.restored = Some(k);
            }
            crate::windows::adopt(app, id, &p.docs, p.front);
        }
    }
    app.windows.focused = 0;
    if any {
        app.ui.home = false;
    }
}
