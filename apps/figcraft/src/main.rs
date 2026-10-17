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
//! the command line are not. A root that isn't an existing folder stops the app at startup.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
mod apple_events;
mod args;
mod control_server;
mod gamepad;
mod graphics;
#[cfg(target_os = "linux")]
mod linux_window;
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
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        {
            let mut slot = self.ctx_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(ctx.clone());
            }
        }
        let title = with_app(|fig| {
            control_server::drain_queue(&self.queue, &mut fig.control_queue);
            figcraft_ui_egui::control::drain_hidden(fig, ctx);
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
        let now = ctx.input(|i| i.time);
        if let Some(wait) = with_app(|fig| recovery::lock(&self.recovery).as_mut().and_then(|r| r.tick(&fig.session, now))).flatten() {
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
        with_app(|fig| services::save_state(fig));
        if let Some(r) = recovery::lock(&self.recovery).take() {
            r.finish();
        }
    }
}

fn main() -> eframe::Result {
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
    let fonts_slot = ctx_slot.clone();
    std::thread::spawn(move || {
        figcraft_text::scan_system_fonts();
        if let Some(c) = fonts_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
            c.request_repaint();
        }
    });
    let mut session = Session::empty();
    // Live sessions: the network threads wake the window when something arrives.
    let wake_slot = ctx_slot.clone();
    session.live.install(Box::new(figcraft_collab_net::NetLink::new(Arc::new(move || {
        if let Some(c) = wake_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
            c.request_repaint();
        }
    }))));
    for f in &args.files {
        if let Err(e) = session.execute("file.open", &json!({"path": f})) {
            log::error!("{f}: {e}");
        }
    }
    let opened: Vec<u64> = session.docs.iter().map(|d| d.uid).collect();
    // Documents a crashed or killed run left unsaved come back; this run keeps its own copies.
    let recovery_root = services::config_dir().map(|d| d.join("Recovery"));
    // The preferences first: they hold the online files a recovered document may be one of.
    let recovery = recovery::Shared::default();
    let svc = services::Desktop::load(recovery.clone());
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
    // Downloaded fonts go in the user's font folder, where the next start finds them too.
    session.fonts_dir = figcraft_text::user_font_dir();
    // Invite links open this app (Windows registers per user at run time; installed builds by
    // default, never a build from source unless asked).
    session.live.links = figcraft_collab_net::links::system();
    session.invite_links_startup();
    // A first run (no settings kept yet, nothing to open) starts on the home screen, with its
    // tips and templates; later runs start in a new file.
    let first_run = svc.prefs.is_none() && args.join.is_none();
    if session.docs.is_empty() && !first_run {
        let _ = session.execute("file.new", &json!({}));
    }
    // The interface language from the first frame on, and for the message when the window can't
    // open (no frame then).
    figcraft_ui_egui::i18n::set(&session.prefs.language, figcraft_ui_egui::Services::system_language(&svc).as_deref());
    let mut fig = FigApp::new(session, Box::new(svc));
    fig.automation_roots = roots;
    fig.ui.home = false;
    // Files named on the command line are the user's too: their missing fonts are listed.
    fig.ui.font_checks = opened;
    fig.note_recovered(recovered.copies);
    if let Some(link) = &args.join {
        figcraft_ui_egui::live::open_invite(&mut fig, link);
    }
    let queue: Arc<Mutex<Vec<ControlRequest>>> = Arc::new(Mutex::new(Vec::new()));
    if let Some(p) = port {
        control_server::start(p, queue.clone(), ctx_slot.clone());
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
    // Start frameless on Linux; the app creator checks the actual display backend
    // before enabling a client-side frame. Other windows inherit that decision.
    fig.ui.title_bar = desktop_title_bar();
    viewport = window_chrome(viewport, fig.ui.title_bar);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let mut options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, ..Default::default() };
    graphics::configure(&mut options.wgpu_options);
    figcraft_ui_egui::windows::install(fig);
    #[cfg(target_os = "linux")]
    let linux_display = linux_window::display_socket();
    // Registered before the event loop starts, so it catches the Finder event that launched the app
    // as well as later ones. Lives until the event loop returns; the app creator only borrows it.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;
    let result = eframe::run_native(
        "FigCraft",
        options,
        Box::new(move |_cc| {
            #[cfg(target_os = "linux")]
            if linux_window::needs_native_frame(_cc, linux_display) {
                let _ = with_app(|fig| fig.ui.title_bar = figcraft_ui_egui::TitleBar::Native);
                _cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
            }
            Ok(Box::new(App {
                queue,
                ctx_slot,
                title: String::new(),
                recovery,
                #[cfg(target_os = "macos")]
                apple_events: apple_events.connect(&_cc.egui_ctx),
            }))
        }),
    );
    // Say why instead of quitting silently: release builds on Windows have no console.
    if let Err(e) = &result {
        startup::failed(e, port.is_some());
    }
    result
}

fn desktop_title_bar() -> figcraft_ui_egui::TitleBar {
    use figcraft_ui_egui::TitleBar;
    if cfg!(target_os = "macos") { TitleBar::Inset } else { TitleBar::Custom }
}

fn window_chrome(viewport: egui::ViewportBuilder, title_bar: figcraft_ui_egui::TitleBar) -> egui::ViewportBuilder {
    use figcraft_ui_egui::TitleBar;
    match title_bar {
        TitleBar::Inset => viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false),
        TitleBar::Native => viewport.with_decorations(true),
        TitleBar::Custom => viewport.with_decorations(false),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_window_tests {
    #[test]
    fn linux_starts_with_the_existing_frameless_title_bar() {
        let bar = super::desktop_title_bar();
        assert_eq!(bar, figcraft_ui_egui::TitleBar::Custom);
        let viewport = super::window_chrome(egui::ViewportBuilder::default(), bar);
        assert_eq!(viewport.decorations, Some(false));
    }
}
