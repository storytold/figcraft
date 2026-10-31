//! FigCraft desktop app.
//!
//! Usage: `figcraft [--control <port>] [files…] [figcraft://join?…]` (an invite link joins that
//! live session; the app registers itself as the opener of such links: `collab.registerLinks`).
//!
//! `--control <port>` (or `FIGCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server:
//! `{"id":1,"method":"engine.execute","params":{"command":"shape.rectangle","params":{"x":0,"y":0,"width":100,"height":100}}}`
//! → `{"id":1,"ok":true,"result":{…}}`. Methods: `figcraft_ui_egui::control`.
//!
//! `--automation-read-root <dir>` / `--automation-write-root <dir>` (each repeatable) confine the
//! files the window and the control channel read and write (`figcraft_engine::file_access`); the
//! app's own folders (settings, recovery copies, installed plugins) and opening the files given on
//! the command line are not. A root that isn't an existing folder stops the app at startup, and so
//! does any flag the app doesn't know (a mistyped root flag must not start the app unconfined).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

// A missing Windows GNU flag re-enables the COM error-info lookup that crashes while AccessKit
// reports a removed UI Automation node. Catch build configurations that drop the workaround.
#[cfg(all(target_os = "windows", target_env = "gnu", not(windows_slim_errors)))]
compile_error!("Windows GNU builds must set windows_slim_errors to keep UI Automation responsive");

#[cfg(target_os = "macos")]
mod apple_events;
mod args;
mod control_server;
mod gamepad;
mod graphics;
mod recovery;
mod services;
mod startup;

#[cfg(all(test, target_os = "macos"))]
mod apple_events_tests;
#[cfg(test)]
mod args_tests;
#[cfg(test)]
mod graphics_tests;
#[cfg(test)]
mod recovery_online_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod services_tests;
#[cfg(test)]
mod settings_dir_tests;
#[cfg(test)]
mod startup_tests;

use std::sync::{Arc, Mutex};

use figcraft_engine::Session;
use figcraft_ui_egui::windows::with_app;
use figcraft_ui_egui::{ControlRequest, FigApp};
use serde_json::json;

/// The app state lives in [`figcraft_ui_egui::windows`] (other windows draw it too).
struct App {
    queue: Arc<Mutex<Vec<ControlRequest>>>,
    ctx_slot: Arc<Mutex<Option<egui::Context>>>,
    /// The window title last sent: a viewport command asks for another frame, so it's only sent
    /// when the title changes (sending it every frame kept an idle window redrawing).
    title: String,
    /// Shared with the services, which write the copies after a panic.
    recovery: recovery::Shared,
    /// Documents and quit requests from Finder, the Dock and Open With.
    #[cfg(target_os = "macos")]
    apple_events: fmv_macos_events::Inbox,
}

impl eframe::App for App {
    /// Native eframe logic callback: store the first context, drain control requests, run
    /// guarded background work, poll macOS open events, handle close requests and title
    /// changes, and schedule recovery repaints on the app clock.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        {
            let mut slot = self.ctx_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(ctx.clone());
            }
        }
        let title = with_app(|fig| {
            control_server::drain_queue(&self.queue, &mut fig.control_queue);
            // Phase-safe background work: control queues, autosave/recovery deadlines, live polling.
            // It runs on logic-only callbacks too, so it never reads UI input.
            figcraft_ui_egui::windows::guard(fig, ctx, |fig| fig.background_tick(ctx));
            #[cfg(target_os = "macos")]
            apple_events::poll(&self.apple_events, fig, ctx);
            // Closing the window asks about unsaved changes (and keeps the app running while other
            // windows are open).
            figcraft_ui_egui::windows::on_close_request(fig, ctx);
            fig.title()
        });
        // Window title follows the document.
        if let Some(title) = title
            && title != self.title
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        // Recovery deadlines use the same monotonic clock as the app's background work: a
        // logic-only tick leaves `ctx.input(i.time)` stale.
        if let Some(wait) = with_app(|fig| {
            let now = fig.now_secs();
            recovery::lock(&self.recovery).as_mut().and_then(|r| r.tick(&fig.session, now))
        })
        .flatten()
        {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Pointer and key events sent over the control channel (`ui.input`), for the window
        // about to draw.
        if let Some(events) = with_app(|fig| figcraft_ui_egui::windows::take_input(fig, raw_input.viewport_id)) {
            raw_input.events.extend(events);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        services::parented(frame, || with_app(|fig| figcraft_ui_egui::windows::guard(fig, &ctx, |fig| fig.ui(ui))));
    }

    fn on_exit(&mut self) {
        // With the app out of reach (busy), its recovery copies stay for the next start.
        let copies = recovery::lock(&self.recovery).take();
        with_app(|fig| services::shutdown(fig, services::config_dir().as_deref(), copies));
        // Everything is kept: end before the windows' teardown, where AppKit's Touch Bar support
        // throws on the content view AccessKit restores (craft-libs#3, #781).
        #[cfg(target_os = "macos")]
        std::process::exit(0);
    }
}

fn main() -> eframe::Result {
    // Windows: the update helper is a copy of this program started with `--finish-update`.
    #[cfg(windows)]
    if let Some(result) = craft_update::self_update::finish(&figcraft_ui_egui::UPDATE_APP, &std::env::args_os().skip(1).collect::<Vec<_>>()) {
        std::process::exit(i32::from(result.is_err()));
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!(
            "FigCraft {}{}",
            env!("CARGO_PKG_VERSION"),
            option_env!("FIGCRAFT_BUILD_SHA").filter(|s| !s.is_empty()).map(|s| format!(" ({s})")).unwrap_or_default()
        );
        return Ok(());
    }
    let args = args::parse(&args);
    if let Some(flag) = args.unknown.first() {
        eprintln!(
            "FigCraft: unknown option {flag}\nUsage: figcraft [--control PORT] [--automation-read-root DIR]... [--automation-write-root DIR]... [FILE | figcraft://join?...]..."
        );
        std::process::exit(2);
    }
    let roots = match (
        args.read_roots.is_empty() && args.write_roots.is_empty(),
        figcraft_engine::file_access::Roots::new(&args.read_roots, &args.write_roots),
    ) {
        (true, _) => None,
        (false, Ok(r)) => Some(Arc::new(r)),
        (false, Err(e)) => {
            eprintln!("FigCraft: {e}");
            std::process::exit(2);
        }
    };
    let port = args.control.or_else(|| std::env::var("FIGCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok()));
    // System fonts: index them in the background, then wake the UI (an idle window draws no frames)
    // to lay text out again with them.
    let ctx_slot: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
    // Wakes the window (once it is there) from another thread.
    let wake = |slot: &Arc<Mutex<Option<egui::Context>>>| {
        let slot = slot.clone();
        move || {
            if let Some(c) = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                c.request_repaint();
            }
        }
    };
    let fonts_wake = wake(&ctx_slot);
    std::thread::spawn(move || {
        figcraft_text::scan_system_fonts();
        fonts_wake();
    });
    let mut session = Session::empty();
    // Live sessions: the network threads wake the window when something arrives.
    session.live.install(Box::new(figcraft_collab_net::NetLink::new(Arc::new(wake(&ctx_slot)))));
    // A file that won't open is said in the window too: it may have been double-clicked.
    let mut failed_open = None;
    for f in &args.files {
        if let Err(e) = session.execute("file.open", &json!({"path": f})) {
            log::error!("{f}: {e}");
            let name = std::path::Path::new(f).file_name().map_or_else(|| f.into(), |n| n.to_string_lossy());
            failed_open = Some(format!("{name}: {e}"));
        }
    }
    let opened: Vec<u64> = session.docs.iter().map(|d| d.uid).collect();
    // Documents a crashed or killed run left unsaved come back; this run keeps its own copies.
    let recovery_root = services::config_dir().map(|d| d.join("Recovery"));
    // The preferences first: they hold the online files a recovered document may be one of.
    let recovery = recovery::Shared::default();
    let svc = services::Desktop::load(recovery.clone());
    svc.follow_system_dark(wake(&ctx_slot));
    if let Some(prefs) = svc.prefs.clone() {
        session.prefs = prefs;
    }
    let recovered = recovery_root.as_deref().map(|r| recovery::recover_all(r, &mut session)).unwrap_or_default();
    let mut copies = recovery_root.as_deref().and_then(recovery::Recovery::start);
    // Copies of online files stay until their files arrive from their servers.
    if let Some(r) = copies.as_mut() {
        r.adopt(recovered.waiting);
    }
    *recovery::lock(&recovery) = copies;
    // Plugins installed from a server's plugin directory live with the user's settings.
    session.plugins.install_dir = services::config_dir().map(|d| d.join("Plugins"));
    // Invite links open this app (Windows registers per user at run time; installed builds by
    // default, never a build from source unless asked).
    session.live.links = figcraft_collab_net::links::system();
    session.invite_links_startup();
    let first_run = svc.prefs.is_none() && args.join.is_none();
    if session.docs.is_empty() && opens_new_file(first_run, session.prefs.start_on_home, port.is_some()) {
        let _ = session.execute("file.new", &json!({}));
    }
    // The interface language from the first frame on, and for the message when the window can't
    // open (no frame then).
    figcraft_ui_egui::i18n::set(&session.prefs.language, figcraft_ui_egui::Services::system_language(&svc).as_deref());
    let mut fig = FigApp::new(session, Box::new(svc));
    fig.set_startup_update_check(true);
    fig.automation_roots = roots;
    fig.ui.home = false;
    // Files named on the command line are the user's too: their missing fonts are listed.
    fig.ui.font_checks = opened;
    fig.note_recovered(recovered.copies);
    if let Some(message) = failed_open {
        fig.show_error(message);
    }
    if let Some(link) = &args.join {
        figcraft_ui_egui::live::open_invite(&mut fig, link);
    }
    let queue: Arc<Mutex<Vec<ControlRequest>>> = Arc::new(Mutex::new(Vec::new()));
    if let Some(p) = port {
        control_server::start(p, graphics::port_wait(), queue.clone(), ctx_slot.clone(), fig.session.events.clone());
    }
    let icon = services::window_icon();
    // The app id is the Wayland app_id and the X11 window class on Linux: the desktop entry's name
    // and StartupWMClass (FreeBSD's package matches the binary's name instead).
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("FigCraft")
        .with_app_id("ai.storyteller.figcraft")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([800.0, 500.0])
        .with_drag_and_drop(true);
    // Who draws the window frame is decided once, before the viewport is built, by the shared
    // craft-window crate (the rest of the Craft suite uses it too). On macOS the system frame is
    // kept with FigCraft's inset traffic lights; on Windows it is the system frame; on Linux the
    // decision follows the desktop: a Wayland compositor with server-side decorations (KDE, Sway,
    // Hyprland, COSMIC) or an X11 window manager keeps the native frame, while GNOME's Mutter —
    // which draws no server-side frame — leaves it to the app's own custom title bar. This replaces
    // the old bespoke xdg-decoration probe that always drew FigCraft's custom title bar on KDE.
    fig.ui.title_bar = desktop_title_bar();
    viewport = window_chrome(viewport, fig.ui.title_bar);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let mut options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, ..Default::default() };
    let graphics = graphics::Plan::configure(&mut options.wgpu_options);
    // Counts the frames drawn, which tells a window that failed starting up from one that ran.
    let window = ctx_slot.clone();
    figcraft_ui_egui::windows::install(fig);
    // Registered before the event loop starts, so it catches the Finder event that launched the app
    // as well as later ones. Lives until the event loop returns; the app creator only borrows it.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;
    let result = eframe::run_native(
        "FigCraft",
        options,
        Box::new(|cc| {
            if let Some(state) = &cc.wgpu_render_state {
                graphics.opened(&state.adapter.get_info());
            }
            with_app(|fig| fig.services.set_egui_context(&cc.egui_ctx));
            // winit's own Quit (⌘Q) ends the process through `terminate:` without ever asking the
            // app, so unsaved changes are lost with no warning (issue #857). This menu's Quit
            // sends the root window a close request instead, and the unsaved-changes question
            // asks just as it does for the close button. Failure keeps winit's menu: the app
            // runs, but ⌘Q ends it without asking.
            #[cfg(target_os = "macos")]
            if let Err(e) = craft_menu::mac::install_app_menu(&cc.egui_ctx, "FigCraft") {
                log::warn!("app menu: kept winit's Quit, which ends the app without asking: {e}");
            }
            Ok(Box::new(App {
                queue,
                ctx_slot,
                title: String::new(),
                recovery,
                #[cfg(target_os = "macos")]
                apple_events: apple_events.connect(&cc.egui_ctx),
            }))
        }),
    );
    let frames = window.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map_or(0, egui::Context::cumulative_frame_nr);
    // Start again on other graphics, or say why instead of quitting silently: release builds on
    // Windows have no console.
    graphics.finish(result, frames, port.is_some())
}

/// With nothing to open, whether the window starts in a new file rather than on the home screen.
/// A first run (no settings kept yet) shows the home screen's tips and templates, and later runs
/// start in a new file unless the preference asks for Home, which `--control` overrides: its
/// callers expect a document to act on.
fn opens_new_file(first_run: bool, start_on_home: bool, control: bool) -> bool {
    !first_run && (!start_on_home || control)
}

/// The title bar FigCraft starts with, decided by the shared craft-window crate so every Craft app
/// agrees on who draws the frame per desktop. The override variable is `FIGCRAFT_TITLEBAR`
/// (`app` / `system`). macOS keeps its inset traffic lights unchanged (craft-window returns the
/// desktop frame there); Windows keeps the system frame; on Linux the compositor/window manager
/// decides (KDE/Sway/Hyprland/COSMIC and X11 keep the native frame, GNOME gets the custom one).
fn desktop_title_bar() -> figcraft_ui_egui::TitleBar {
    let choice = craft_window::Choice::probe(craft_window::Prefix::new("FIGCRAFT"));
    title_bar_for(choice.into_frame())
}

/// Map craft-window's [`craft_window::Frame`] onto FigCraft's own [`figcraft_ui_egui::TitleBar`].
///
/// `Frame::App` means the app draws its own frame — FigCraft's custom title bar. `Frame::Desktop`
/// means the desktop draws it, but FigCraft keeps each platform's existing look there:
/// - macOS: the inset title bar with its traffic lights (`TitleBar::Inset`), unchanged.
/// - Windows: FigCraft's own custom title bar (`TitleBar::Custom`), unchanged — Windows never used
///   the plain native frame, and craft-window returning `Desktop` on Windows must not change that.
/// - Linux and the BSDs: the compositor's native frame (`TitleBar::Native`). This is the only
///   behaviour change: KDE and the other server-side-decoration compositors now keep their frame.
fn title_bar_for(frame: craft_window::Frame) -> figcraft_ui_egui::TitleBar {
    use figcraft_ui_egui::TitleBar;
    match frame {
        craft_window::Frame::Desktop if cfg!(target_os = "macos") => TitleBar::Inset,
        // Windows kept FigCraft's own title bar before this change; keep it.
        craft_window::Frame::Desktop if cfg!(target_os = "windows") => TitleBar::Custom,
        craft_window::Frame::Desktop => TitleBar::Native,
        // The app draws the frame (GNOME's Wayland session, or a `FIGCRAFT_TITLEBAR=app` override).
        craft_window::Frame::App(_) => TitleBar::Custom,
    }
}

fn window_chrome(viewport: egui::ViewportBuilder, title_bar: figcraft_ui_egui::TitleBar) -> egui::ViewportBuilder {
    use figcraft_ui_egui::TitleBar;
    match title_bar {
        TitleBar::Inset => viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false),
        TitleBar::Native => viewport.with_decorations(true),
        TitleBar::Custom => viewport.with_decorations(false),
    }
}

#[cfg(test)]
mod title_bar_tests {
    use figcraft_ui_egui::TitleBar;

    /// craft-window's app frame (GNOME's Wayland session, or a `FIGCRAFT_TITLEBAR=app` override)
    /// becomes FigCraft's custom title bar on every platform, and `window_chrome` then drops the
    /// decorations so FigCraft draws its own.
    #[test]
    fn the_app_frame_maps_to_the_custom_title_bar() {
        let layout = craft_window::Layout::default();
        let bar = super::title_bar_for(craft_window::Frame::App(layout));
        assert_eq!(bar, TitleBar::Custom);
        let viewport = super::window_chrome(egui::ViewportBuilder::default(), bar);
        assert_eq!(viewport.decorations, Some(false));
    }

    /// craft-window's desktop frame keeps each platform's existing look: macOS its inset traffic
    /// lights, Windows FigCraft's own custom title bar (unchanged), and Linux/BSD the compositor's
    /// native frame. The Linux case is the only behaviour change: KDE and the other
    /// server-side-decoration compositors now keep their native frame.
    #[test]
    fn the_desktop_frame_is_mapped_per_platform() {
        let bar = super::title_bar_for(craft_window::Frame::Desktop);
        if cfg!(target_os = "macos") {
            assert_eq!(bar, TitleBar::Inset);
        } else if cfg!(target_os = "windows") {
            // Windows kept its own title bar before and must keep it now.
            assert_eq!(bar, TitleBar::Custom);
            let viewport = super::window_chrome(egui::ViewportBuilder::default(), bar);
            assert_eq!(viewport.decorations, Some(false));
        } else {
            assert_eq!(bar, TitleBar::Native);
            let viewport = super::window_chrome(egui::ViewportBuilder::default(), bar);
            assert_eq!(viewport.decorations, Some(true));
        }
    }

    /// macOS is untouched by this change: its inset title bar with the system traffic lights stays,
    /// and `window_chrome` still asks for the full-size content view rather than toggling the
    /// window decorations.
    #[test]
    #[cfg(target_os = "macos")]
    fn macos_keeps_its_inset_title_bar() {
        let bar = super::desktop_title_bar();
        assert_eq!(bar, TitleBar::Inset);
        let viewport = super::window_chrome(egui::ViewportBuilder::default(), bar);
        assert_eq!(viewport.fullsize_content_view, Some(true));
        assert_eq!(viewport.title_shown, Some(false));
    }
}

#[cfg(test)]
mod start_tests {
    use super::opens_new_file;

    #[test]
    fn the_home_preference_starts_on_home_except_for_control_callers() {
        assert!(opens_new_file(false, false, false), "a new file by default");
        assert!(!opens_new_file(false, true, false), "Home when the preference asks");
        assert!(opens_new_file(false, true, true), "--control still starts with a document");
        assert!(!opens_new_file(true, false, false), "a first run shows the home screen");
    }
}
