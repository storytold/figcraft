//! Several windows (desktop app): File ▸ New window, a tab's "Open in new window" or a tab dragged
//! off the tab strip opens files in another OS window with its own tabs, panels and canvas. The
//! session (open files, preferences, libraries, the clipboard) is shared: a window is a set of tabs
//! plus the UI state it swaps into the app while it draws. The main window shows every open file no
//! other window holds. The browser has one window.
//!
//! Other windows are egui deferred viewports: each draws only when it needs to. Their draw
//! callbacks run on the main thread but must be `Send`, so the desktop app keeps its [`FigApp`] in
//! a thread-local ([`install`], [`with_app`]).

use std::cell::RefCell;

use egui::{Context, Pos2, Vec2, ViewportBuilder, ViewportCommand, ViewportId};
use figcraft_engine::DocState;
use serde_json::{Value, json};

use crate::canvas::CanvasState;
use crate::i18n::{tr, trf};
use crate::render_worker::Worker;
use crate::{FigApp, TitleBar, UiState};

/// Another window: its tabs and the UI state it swaps into the app while it draws.
pub struct Window {
    pub id: u64,
    /// Its open files (document uids; the tabs keep the session's order).
    pub docs: Vec<u64>,
    /// The file it shows.
    active: Option<u64>,
    pane: Pane,
    /// Where it opens and its size, then where it is (screen points).
    pos: Option<Pos2>,
    size: Vec2,
}

/// A window's own UI state.
#[derive(Default)]
struct Pane {
    ui: UiState,
    canvas: CanvasState,
    worker: Option<Worker>,
    field_focus: bool,
    control_input: Vec<egui::Event>,
}

/// The app's other windows.
#[derive(Default)]
pub struct Windows {
    pub list: Vec<Window>,
    /// The window drawing now (0: the main window).
    pub current: u64,
    /// The window last in front: control requests without a `window` go to it.
    pub focused: u64,
    /// The main window's file while another window draws.
    main_active: Option<u64>,
    /// Windows to close (their files close with them), at the main window's next frame.
    closing: Vec<u64>,
    last_id: u64,
}

impl Windows {
    /// The window holding open file `uid` (0: the main window).
    pub fn of(&self, uid: u64) -> u64 {
        self.list.iter().find(|w| w.docs.contains(&uid)).map_or(0, |w| w.id)
    }

    pub fn exists(&self, id: u64) -> bool {
        id == 0 || self.list.iter().any(|w| w.id == id)
    }
}

/// Window `id`'s egui viewport.
pub fn viewport(id: u64) -> ViewportId {
    if id == 0 { ViewportId::ROOT } else { ViewportId::from_hash_of(("figcraft_window", id)) }
}

thread_local! {
    static APP: RefCell<Option<FigApp>> = const { RefCell::new(None) };
}

/// Keep the app where other windows' draw callbacks find it (the desktop app, once, on the main
/// thread).
pub fn install(app: FigApp) {
    APP.with(|a| *a.borrow_mut() = Some(app));
}

/// Run `f` on the installed app (`None` before [`install`]).
pub fn with_app<R>(f: impl FnOnce(&mut FigApp) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Last-resort guard around a frame: a panic becomes a message and recovery copies of unsaved
/// files, not a lost document.
pub fn guard(app: &mut FigApp, ctx: &Context, f: impl FnOnce(&mut FigApp)) {
    // Started with automation roots, everything the window and the control channel do is confined.
    let roots = app.automation_roots.clone();
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| figcraft_engine::file_access::confined(roots.as_ref(), || f(&mut *app))));
    if let Err(e) = res {
        let msg = e.downcast_ref::<&str>().map(ToString::to_string).or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
        log::error!("UI panic: {msg}");
        app.toast(format!("Internal error (please report): {msg}"));
        ctx.request_repaint();
        app.services.emergency_save(&app.session);
    }
}

/// Control-channel input events (`ui.input`) for the window `viewport` draws next.
pub fn take_input(app: &mut FigApp, viewport_id: ViewportId) -> Vec<egui::Event> {
    match app.windows.list.iter_mut().find(|w| viewport(w.id) == viewport_id) {
        Some(w) => std::mem::take(&mut w.pane.control_input),
        None if viewport_id == ViewportId::ROOT => std::mem::take(&mut app.control_input),
        None => Vec::new(),
    }
}

fn title_of(ds: Option<&DocState>) -> String {
    match ds {
        Some(ds) => format!("{}{} – FigCraft", ds.doc.name, if ds.is_dirty() { " •" } else { "" }),
        None => "FigCraft".into(),
    }
}

impl FigApp {
    /// The window drawing now shows this open file as a tab.
    pub fn shows(&self, uid: u64) -> bool {
        self.windows.of(uid) == self.windows.current
    }

    /// The window drawing now has tabs (else it shows Home).
    pub fn has_files(&self) -> bool {
        self.session.docs.iter().any(|d| self.shows(d.uid))
    }

    /// The window title: the file it shows and whether it has unsaved changes.
    pub fn title(&self) -> String {
        title_of(self.session.ds().ok().filter(|d| self.shows(d.uid)))
    }

    /// Files in this window with unsaved changes (a branch tab's edits live on in its file).
    fn unsaved(&self) -> Vec<String> {
        self.session.docs.iter().filter(|d| self.shows(d.uid) && d.is_dirty() && d.branch_of.is_none()).map(|d| d.doc.name.clone()).collect()
    }
}

/// Make the session's active file one the window drawing now shows: `want`, else the one active
/// now, else its first.
fn show_own(app: &mut FigApp, want: Option<u64>) {
    let mine = |d: &DocState| app.shows(d.uid);
    let pick = want
        .and_then(|u| app.session.docs.iter().position(|d| d.uid == u && mine(d)))
        .or_else(|| app.session.ds().ok().filter(|d| mine(d)).map(|_| app.session.active))
        .or_else(|| app.session.docs.iter().position(mine));
    if let Some(i) = pick {
        app.session.active = i;
    }
}

/// Swap a window's UI state with the app's; what the app shares between windows stays.
fn exchange(app: &mut FigApp, p: &mut Pane) {
    std::mem::swap(&mut app.ui, &mut p.ui);
    std::mem::swap(&mut app.ui.last_copy, &mut p.ui.last_copy);
    std::mem::swap(&mut app.ui.quit, &mut p.ui.quit);
    std::mem::swap(&mut app.ui.dark, &mut p.ui.dark);
    std::mem::swap(&mut app.canvas, &mut p.canvas);
    std::mem::swap(&mut app.worker, &mut p.worker);
    std::mem::swap(&mut app.field_focus, &mut p.field_focus);
    std::mem::swap(&mut app.control_input, &mut p.control_input);
}

/// Open a window (`window.new`) with open file `index` moved into it, the file at `path`, or a new
/// file, at screen point `at` (else beside this window). Returns its id. With one window (the
/// browser) or a screen reader running, a file opens as a tab here instead (and the error says so).
pub fn open(app: &mut FigApp, ctx: &Context, index: Option<usize>, path: Option<&str>, at: Option<Pos2>) -> Result<u64, String> {
    // shortcut: eframe 0.36 gives only the main window an AccessKit adapter, so other windows have
    // no screen-reader tree; while a screen reader runs files stay tabs here. Upgrade when eframe
    // adapts every viewport.
    let one = if ctx.embed_viewports() {
        Some("This build has one window: files open as tabs")
    } else if crate::a11y::active(ctx) {
        Some("With a screen reader running, files open as tabs in the main window")
    } else {
        None
    };
    let uid = match (index, path) {
        (Some(i), _) => match (app.session.docs.get(i), one) {
            (Some(_), Some(why)) => return Err(why.into()),
            (Some(d), None) => d.uid,
            (None, _) => return Err(format!("no open file {i}")),
        },
        (None, path) => {
            let opened = match path {
                Some(p) => app.run("file.open", json!({"path": p})),
                None => app.run("file.new", json!({})),
            };
            let uid = opened.and_then(|_| app.session.ds().ok().map(|d| d.uid));
            app.ui.home = false;
            uid.ok_or_else(|| app.ui.last_error.clone().unwrap_or_else(|| "no file opened".into()))?
        }
    };
    if let Some(why) = one {
        return Err(why.into());
    }
    for w in &mut app.windows.list {
        w.docs.retain(|u| *u != uid);
    }
    let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
    app.windows.last_id += 1;
    let id = app.windows.last_id;
    app.windows.list.push(Window {
        id,
        docs: vec![uid],
        active: Some(uid),
        pane: Pane {
            // A new window opens like the one it comes from: the same tabs, panel widths and pages
            // list height.
            ui: UiState {
                title_bar: app.ui.title_bar,
                right_tab: app.ui.right_tab,
                left_tab: app.ui.left_tab,
                left_width: app.ui.left_width,
                right_width: app.ui.right_width,
                pages_height: app.ui.pages_height,
                ..UiState::default()
            },
            ..Pane::default()
        },
        pos: at.or_else(|| outer.map(|r| r.min + Vec2::splat(32.0))),
        size: inner.map_or(Vec2::new(1280.0, 800.0), |r| r.size()),
    });
    app.windows.focused = id;
    ctx.request_repaint_of(ViewportId::ROOT);
    Ok(id)
}

/// Bring window `id` to the front; control requests go to it from now on.
pub fn focus(app: &mut FigApp, ctx: &Context, id: u64) -> Result<Value, String> {
    if !app.windows.exists(id) {
        return Err(format!("no window {id}"));
    }
    app.windows.focused = id;
    ctx.send_viewport_cmd_to(viewport(id), ViewportCommand::Focus);
    Ok(json!({"focused": id}))
}

/// Act on `window.new` / `window.focus` asked for by commands (menus, the Actions palette,
/// `engine.execute`).
pub(crate) fn requests(app: &mut FigApp, ctx: &Context) {
    for r in std::mem::take(&mut app.window_requests) {
        let n = |k: &str| r.data.get(k).and_then(Value::as_u64);
        if r.kind == "window.fullscreen" {
            let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            let on = r.data.get("on").and_then(Value::as_bool).unwrap_or(!full);
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(on));
            continue;
        }
        let res = if r.kind == "window.new" {
            open(app, ctx, n("index").map(|i| i as usize), r.data.get("path").and_then(Value::as_str), None).map(|_| ())
        } else {
            focus(app, ctx, n("id").unwrap_or(u64::MAX)).map(|_| ())
        };
        if let Err(e) = res {
            app.toast(e);
        }
    }
}

/// Every window: id, title, its files and the one it shows (`window.list`).
pub fn list(app: &FigApp) -> Value {
    let active_of = |id: u64| -> Option<u64> {
        if id == app.windows.current {
            app.session.ds().ok().map(|d| d.uid)
        } else if id == 0 {
            app.windows.main_active
        } else {
            app.windows.list.iter().find(|w| w.id == id).and_then(|w| w.active)
        }
    };
    let ids = std::iter::once(0).chain(app.windows.list.iter().map(|w| w.id));
    Value::Array(
        ids.map(|id| {
            let mine = |d: &&DocState| app.windows.of(d.uid) == id;
            let shown = app.session.docs.iter().filter(mine).find(|d| Some(d.uid) == active_of(id));
            json!({
                "id": id,
                "main": id == 0,
                "title": title_of(shown),
                "files": app.session.docs.iter().filter(mine).map(|d| d.doc.name.as_str()).collect::<Vec<_>>(),
                "active": shown.map(|d| d.doc.name.as_str()),
                "focused": id == app.windows.focused,
            })
        })
        .collect(),
    )
}

/// The window drawing now was asked to close (its close button, the system's): its files close
/// with it, after asking when one has unsaved changes. The main window closes last: while other
/// windows are open it takes over one of them. The desktop app calls it from the main window's
/// `logic` (which runs even while the window is minimised); other windows call it themselves.
pub fn on_close_request(app: &mut FigApp, ctx: &Context) {
    if app.ui.quit || !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    let unsaved = !app.unsaved().is_empty();
    if app.windows.current == 0 && (unsaved || !app.windows.list.is_empty()) {
        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    }
    request_close(app, ctx);
}

/// The custom title bar's Close button uses the same unsaved-file check as a system close event.
pub(crate) fn request_close(app: &mut FigApp, ctx: &Context) {
    if !app.unsaved().is_empty() {
        app.ui.confirm_close = true;
        ctx.request_repaint();
    } else {
        close_current(app, ctx);
    }
}

fn close_current(app: &mut FigApp, ctx: &Context) {
    app.windows.closing.push(app.windows.current);
    ctx.request_repaint_of(ViewportId::ROOT);
}

/// What the unsaved-changes question was answered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseAnswer {
    /// Save every file with unsaved changes, then close (Save as for a file without a path).
    Save,
    /// Close and lose the changes.
    DontSave,
    /// Keep the window open.
    Cancel,
}

/// The unsaved-changes question when a window closes: Don't Save / Cancel / Save, as the other
/// Craft apps ask it.
pub(crate) fn confirm_close(app: &mut FigApp, ctx: &Context) {
    if std::mem::take(&mut app.ui.save_on_close) {
        save_then_close(app, ctx);
        return;
    }
    // A Save as this question started: once its dialog is answered, closing goes on if the file
    // was saved; cancelled or failed, the window stays open.
    if let Some(uid) = app.ui.close_after_save_as {
        if app.pending_dialog.is_some() {
            return;
        }
        app.ui.close_after_save_as = None;
        // Saved: go on with the other files, then close.
        if app.session.docs.iter().find(|d| d.uid == uid).is_none_or(|d| !d.is_dirty()) {
            save_then_close(app, ctx);
        }
        return;
    }
    if !app.ui.confirm_close {
        return;
    }
    let names = app.unsaved();
    if names.is_empty() {
        app.ui.confirm_close = false;
        return;
    }
    let t = crate::theme::Tokens::get(ctx);
    let mut open = true;
    let mut answer = None;
    crate::dialogs::window(tr("Unsaved changes"), ctx).order(egui::Order::Foreground).open(&mut open).show(ctx, |ui| {
        ui.set_width(340.0);
        crate::widgets::label(ui, &trf("Unsaved changes in {names}. Save them before closing?", &[("names", &names.join(", "))]), t.text);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if crate::widgets::button(ui, tr("Don't Save"), false, true).clicked() {
                answer = Some(CloseAnswer::DontSave);
            }
            if crate::widgets::button(ui, tr("Cancel"), false, true).clicked() {
                answer = Some(CloseAnswer::Cancel);
            }
            if crate::widgets::button(ui, tr("Save"), true, true).clicked() {
                answer = Some(CloseAnswer::Save);
            }
        });
        // Save is the default: Enter answers it when no button has the keyboard.
        if answer.is_none() && ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            answer = Some(CloseAnswer::Save);
        }
    });
    if !open {
        answer = Some(CloseAnswer::Cancel);
    }
    if let Some(a) = answer {
        answer_close(app, ctx, a);
    }
}

/// Act on the unsaved-changes question's answer.
pub(crate) fn answer_close(app: &mut FigApp, ctx: &Context, answer: CloseAnswer) {
    app.ui.confirm_close = false;
    match answer {
        CloseAnswer::Cancel => {}
        CloseAnswer::DontSave => close_current(app, ctx),
        CloseAnswer::Save => {
            app.ui.save_on_close = true;
            ctx.request_repaint();
        }
    }
}

/// Save each file of this window with unsaved changes, then close it. A file without a path (or
/// one saved as .fig, which is never overwritten silently) asks where with Save as, and closing goes
/// on when that dialog saved it ([`confirm_close`]). A save that fails shows why and keeps the
/// window open.
fn save_then_close(app: &mut FigApp, ctx: &Context) {
    let files: Vec<u64> = app.session.docs.iter().filter(|d| app.shows(d.uid) && d.is_dirty() && d.branch_of.is_none()).map(|d| d.uid).collect();
    for uid in files {
        let Some(i) = app.session.docs.iter().position(|d| d.uid == uid) else { continue };
        app.session.active = i;
        let native =
            app.session.docs.get(i).and_then(|d| d.path.as_deref()).is_some_and(|p| {
                !std::path::Path::new(p).extension().and_then(std::ffi::OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("fig"))
            });
        if !native {
            app.ui.close_after_save_as = Some(uid);
            app.save_as_dialog();
            if app.pending_dialog.is_none() {
                // No dialog could start: stay open.
                app.ui.close_after_save_as = None;
            }
            ctx.request_repaint();
            return;
        }
        if app.run("file.save", json!({})).is_none() {
            return;
        }
    }
    close_current(app, ctx);
}

/// Before the main window draws: windows asked to close close, closed files leave their windows
/// (a window left with none closes) and the main window shows one of its own files.
pub(crate) fn begin(app: &mut FigApp, ctx: &Context) {
    app.windows.current = 0;
    for id in std::mem::take(&mut app.windows.closing) {
        if id == 0 && app.windows.list.is_empty() {
            app.ui.quit = true;
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
            continue;
        }
        let files: Vec<u64> = app.session.docs.iter().filter(|d| app.windows.of(d.uid) == id).map(|d| d.uid).collect();
        for uid in files {
            if let Some(i) = app.session.docs.iter().position(|d| d.uid == uid) {
                let _ = app.session.execute("file.close", &json!({"index": i, "force": true}));
            }
        }
        if id != 0 {
            app.windows.list.retain(|w| w.id != id);
            continue;
        }
        // The main window takes over another window: its files, panels and place on screen.
        let mut w = app.windows.list.remove(0);
        exchange(app, &mut w.pane);
        if let Some(p) = w.pos {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::OuterPosition(p));
        }
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::InnerSize(w.size));
        app.windows.focused = 0;
        show_own(app, w.active);
    }
    let open: Vec<u64> = app.session.docs.iter().map(|d| d.uid).collect();
    for w in &mut app.windows.list {
        w.docs.retain(|u| open.contains(u));
    }
    app.windows.list.retain(|w| !w.docs.is_empty());
    if !app.windows.exists(app.windows.focused) {
        app.windows.focused = 0;
    }
    if ctx.input(|i| i.viewport().focused == Some(true)) {
        app.windows.focused = 0;
    }
    show_own(app, None);
}

/// After a window drew: when a command showed another window's file (Ctrl+Tab, opening a file
/// open there), that window comes to the front with it. Not when files closed: the session's
/// fallback file isn't a choice.
pub(crate) fn settle(app: &mut FigApp, ctx: &Context, files_before: usize) {
    let Some(uid) = app.session.ds().ok().map(|d| d.uid) else { return };
    let owner = app.windows.of(uid);
    if owner == app.windows.current || app.session.docs.len() < files_before {
        return;
    }
    if owner == 0 {
        app.windows.main_active = Some(uid);
    } else if let Some(w) = app.windows.list.iter_mut().find(|w| w.id == owner) {
        w.active = Some(uid);
    }
    app.windows.focused = owner;
    ctx.send_viewport_cmd_to(viewport(owner), ViewportCommand::Focus);
    ctx.request_repaint_of(viewport(owner));
}

/// Something every window shows changed (the preferences, the theme): the others draw again.
pub(crate) fn repaint_others(app: &FigApp, ctx: &Context) {
    for id in std::iter::once(0).chain(app.windows.list.iter().map(|w| w.id)).filter(|id| *id != app.windows.current) {
        ctx.request_repaint_of(viewport(id));
    }
}

/// Keep the other windows open (each draws itself when it needs to).
pub(crate) fn show(app: &FigApp, ctx: &Context) {
    for w in &app.windows.list {
        let title = title_of(app.session.docs.iter().find(|d| Some(d.uid) == w.active));
        let mut b = ViewportBuilder::default().with_title(title).with_inner_size(w.size).with_min_inner_size([800.0, 500.0]).with_drag_and_drop(true);
        if let Some(p) = w.pos {
            b = b.with_position(p);
        }
        b = match w.pane.ui.title_bar {
            TitleBar::Custom => b.with_decorations(false),
            TitleBar::Inset => b.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false),
            TitleBar::Native => b,
        };
        let id = w.id;
        ctx.show_viewport_deferred(viewport(id), b, move |ui, _| {
            let ctx = ui.ctx().clone();
            if with_app(|app| guard(app, &ctx, |app| draw(app, id, ui))).is_none() {
                log::warn!("window {id}: the app is busy or not installed");
            }
        });
    }
}

/// A window's state swapped into the app ([`enter`]) until [`leave`] swaps it back out.
struct Entered {
    id: u64,
    pane: Pane,
    /// The newest open file before: files opened after it are the window's own.
    newest: u64,
}

/// Swap window `id`'s state into the app and show its file; `None` when there is no such window.
fn enter(app: &mut FigApp, id: u64) -> Option<Entered> {
    let k = app.windows.list.iter().position(|w| w.id == id)?;
    let main_active = app.session.ds().ok().map(|d| d.uid);
    let newest = app.session.docs.iter().map(|d| d.uid).max().unwrap_or(0);
    let w = app.windows.list.get_mut(k)?;
    let want = w.active;
    let mut pane = std::mem::take(&mut w.pane);
    exchange(app, &mut pane);
    app.windows.current = id;
    app.windows.main_active = main_active;
    show_own(app, want);
    Some(Entered { id, pane, newest })
}

/// Files the entered window opened are its own.
fn adopt(app: &mut FigApp, e: &Entered) {
    let new: Vec<u64> = app.session.docs.iter().map(|d| d.uid).filter(|u| *u > e.newest && app.windows.of(*u) == 0).collect();
    if let Some(w) = app.windows.list.iter_mut().find(|w| w.id == e.id) {
        w.docs.extend(new);
    }
}

/// Swap the entered window's state back out (with where it is on screen, when known) and the
/// main window's back in.
fn leave(app: &mut FigApp, mut e: Entered, outer: Option<egui::Rect>, inner: Option<egui::Rect>) {
    let shown = app.session.ds().ok().map(|d| d.uid).filter(|u| app.windows.of(*u) == e.id);
    exchange(app, &mut e.pane);
    if let Some(w) = app.windows.list.iter_mut().find(|w| w.id == e.id) {
        w.active = shown.or(w.active);
        w.pos = outer.map(|r| r.min).or(w.pos);
        w.size = inner.map_or(w.size, |r| r.size());
        w.pane = e.pane;
    }
    app.windows.current = 0;
    let main_active = app.windows.main_active.take();
    show_own(app, main_active);
}

/// Run `f` as window `id` (0: the main window) outside its frame, with its state swapped in as
/// while it draws; `None` when there is no such window. For a window that draws nothing now
/// (minimized, or occluded: behind other windows or on another Space).
pub(crate) fn as_window<R>(app: &mut FigApp, id: u64, f: impl FnOnce(&mut FigApp) -> R) -> Option<R> {
    if id == 0 {
        // Between frames the app holds the main window's state.
        return Some(f(app));
    }
    let e = enter(app, id)?;
    let r = f(app);
    adopt(app, &e);
    leave(app, e, None, None);
    Some(r)
}

/// One frame of window `id`, with its state swapped in.
fn draw(app: &mut FigApp, id: u64, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let files_before = app.session.docs.len();
    let prefs = app.session.prefs.clone();
    let Some(e) = enter(app, id) else { return };
    if ctx.input(|i| i.viewport().focused == Some(true)) {
        app.windows.focused = id;
    }
    on_close_request(app, &ctx);
    app.logic(&ctx);
    // Within the swap, so a panic leaves every window its own state.
    guard(app, &ctx, |app| app.frame(ui));
    adopt(app, &e);
    settle(app, &ctx, files_before);
    if app.session.prefs != prefs {
        repaint_others(app, &ctx);
    }
    let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
    leave(app, e, outer, inner);
}
