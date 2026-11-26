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
// The build script includes the same file for the icon and version it embeds.
#[cfg(test)]
mod build_channel;
mod build_flavor;
mod control_server;
mod gamepad;
mod graphics;
mod launch_profile;
mod nightly_session;
mod process_identity;
mod profile_root;
mod recovery;
mod services;
mod startup;
mod window_frame;

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
    /// Keeps the custom title bar's undecorated windows borderless (Windows, #1343).
    window_frame: window_frame::WindowFrame,
    /// Documents and quit requests from Finder, the Dock and Open With.
    #[cfg(target_os = "macos")]
    apple_events: fmv_macos_events::Inbox,
    /// The Nightly session record this window owns: finished on the controlled exit, where macOS
    /// ends the process and `run_native` never returns.
    nightly: crate::nightly_session::Shared,
    /// Set once this window drew the interrupted-session notice in a frame.
    nightly_notice: Arc<std::sync::atomic::AtomicBool>,
    /// The interrupted-session notice is queued but no frame has drawn it yet. Until one does, the
    /// earlier session's record stays for the next start to report (a renderer failure before the
    /// first frame must not acknowledge it).
    nightly_notice_pending: bool,
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
        // Last: closing the main window asks about unsaved changes (and keeps the app running
        // while other windows are open), through the runtime's close path.
        with_app(|fig| fig.close_path(ctx));
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // winit gives an undecorated window its caption back when it changes the window's flags
        // (maximize, restore, ...): take it off again before the frame is laid out (#1343).
        self.window_frame.run();
        // Pointer and key events sent over the control channel (`ui.input`), for the window
        // about to draw.
        if let Some(events) = with_app(|fig| {
            fig.before_input(ctx);
            figcraft_ui_egui::windows::take_input(fig, raw_input.viewport_id)
        }) {
            raw_input.events.extend(events);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        figcraft_ui_egui::framework::before_ui(&ctx, &*frame);
        services::parented(frame, || with_app(|fig| figcraft_ui_egui::windows::guard(fig, &ctx, |fig| fig.ui(ui))));
        // This frame drew the queued interrupted-session notice: the interruption is now reported.
        if self.nightly_notice_pending {
            self.nightly_notice_pending = false;
            self.nightly_notice.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn on_exit(&mut self) {
        // With the app out of reach (busy), its recovery copies stay for the next start.
        let copies = recovery::lock(&self.recovery).take();
        with_app(|fig| {
            services::shutdown(fig, services::config_dir().as_deref(), copies);
            // An update that waits for the app to close installs now: the app lives in a
            // thread-local, and the process can end without dropping it.
            fig.start_pending_update();
        });
        // The controlled exit: on macOS this is the only path that runs (Cmd+Q ends the process
        // inside AppKit after `on_exit`), so the Nightly session record is finished here. The
        // call after `run_native` is the no-op fallback for a start that created no window.
        crate::nightly_session::finish_once(&self.nightly, self.nightly_notice.load(std::sync::atomic::Ordering::Relaxed));
        // Everything is kept: end before the windows' teardown, where AppKit's Touch Bar support
        // throws on the content view AccessKit restores (craft-libs#3, #781).
        #[cfg(target_os = "macos")]
        std::process::exit(0);
    }
}

/// The Windows taskbar identity of the Nightly: distinct from stable's, so both can be open.
const NIGHTLY_WINDOW_ID: &str = "ai.storyteller.figcraft.nightly-v1";

/// The window and Wayland/X11 app id: the Nightly's own, so its window is not the stable app's.
fn window_app_id() -> &'static str {
    if launch_profile::current().is_nightly() { NIGHTLY_WINDOW_ID } else { "ai.storyteller.figcraft" }
}

/// Whether a desktop session can show a dialog (a headless run has no one to ask).
fn desktop_session() -> bool {
    cfg!(windows)
        || cfg!(target_os = "macos")
        || (cfg!(target_os = "linux") && ["DISPLAY", "WAYLAND_DISPLAY"].into_iter().any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty())))
}

fn initialize_nightly_identity() -> Result<(), String> {
    process_identity::initialize(build_flavor::COMPILED == build_flavor::BuildFlavor::NightlyV1, NIGHTLY_WINDOW_ID)
}

/// Whether a refusal may also open a native message box. Standard error always carries the reason;
/// this only adds a box where one renders without the app's event loop. Never on macOS: rfd's alert
/// there needs the AppKit run loop this start has not reached, so a refused nightly launch hung in
/// an invisible modal instead of exiting (GridCraft Mac smoke, 2026-11-26).
fn refusal_dialog(flavor: build_flavor::BuildFlavor, desktop_session: bool, macos: bool) -> bool {
    build_flavor::needs_native_error(flavor, desktop_session) && !macos
}

/// Say why a launch profile was refused: in a dialog for a Nightly started from a Windows or
/// X11/Wayland desktop (its release build has no console), and on standard error in every case (macOS
/// included: a modal there before the event loop never appears). Always exits 2.
fn report_launch_profile_error(error: &str) -> ! {
    eprintln!("FigCraft launch profile: {error}");
    if refusal_dialog(build_flavor::COMPILED, desktop_session(), cfg!(target_os = "macos")) {
        if let Err(error) = initialize_nightly_identity() {
            eprintln!("FigCraft launch profile: {error}");
        } else {
            let _ = rfd::MessageDialog::new()
                .set_title("FigCraft Nightly")
                .set_description(error)
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        }
    }
    std::process::exit(2)
}

/// Select the immutable launch profile before any startup service or side effect, and answer the
/// version query first: `None` means the query did (there is nothing left to start).
///
/// Stable keeps its arguments and its directory policy untouched; only a Nightly build answers the
/// Launcher's `--launch-profile nightly-v1 --profile-root <dir> --version` form.
fn prepare_launch() -> Option<Vec<std::ffi::OsString>> {
    let argv: Vec<_> = std::env::args_os().skip(1).collect();
    let nightly_build = build_flavor::COMPILED == build_flavor::BuildFlavor::NightlyV1;
    if nightly_build && (build_flavor::version_query(&argv) || launch_profile::version_query(&argv)) {
        // Before root validation, logging, the update helper, fonts or any window: a runner reads
        // this line and the process ends. Nothing else may be printed here.
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), "FigCraft {}", build_flavor::version(build_flavor::COMPILED, env!("FIGCRAFT_BUILT_VERSION")));
        return None;
    }
    let early = launch_profile::parse(argv)
        .and_then(|launch| {
            build_flavor::require_profile(build_flavor::COMPILED, &launch.profile)?;
            initialize_nightly_identity()?;
            Ok(launch)
        })
        .and_then(|launch| {
            launch_profile::prepare(launch, |root| {
                launch_profile::validate_root(root, |key| std::env::var_os(key), std::env::current_exe().ok().as_deref())
            })
        })
        .unwrap_or_else(|error| report_launch_profile_error(&error));
    if let Err(error) = launch_profile::install(early.profile) {
        report_launch_profile_error(&error);
    }
    if let Some(root) = launch_profile::current().root() {
        // Everything that names or finds the app now names the Nightly, from the first frame on.
        figcraft_ui_egui::channel::install_nightly(env!("FIGCRAFT_BUILT_VERSION"), option_env!("FIGCRAFT_BUILD_SHA"));
        // The suite's font store is shared with the stable app: the Nightly installs and reads its
        // fonts inside its own profile instead.
        figcraft_engine::fonts::download::use_store_dir(root.join("Fonts"));
    }
    Some(early.args)
}

fn main() -> eframe::Result {
    // The launch profile comes before the logger, the update helper, fonts and saved state.
    let Some(arguments) = prepare_launch() else { return Ok(()) };
    let nightly = launch_profile::current().is_nightly();
    // Windows: the update helper is a copy of this program started with `--finish-update`. The
    // Nightly has none: the Launcher owns its version.
    #[cfg(windows)]
    if !nightly
        && let Some(result) = craft_update::self_update::finish(&figcraft_ui_egui::UPDATE_APP, &std::env::args_os().skip(1).collect::<Vec<_>>())
    {
        std::process::exit(i32::from(result.is_err()));
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = if nightly {
        // The launcher hands these over after `--`, which `parse` takes as file names verbatim (that
        // is what keeps a file called `--screenshot` a file). "Open with" and a drop onto the
        // launcher pass `file://` URLs (`%U`), so decode those here as a plain argument would be;
        // every other name stays exactly as given.
        arguments
            .iter()
            .map(|a| {
                let arg = a.to_string_lossy();
                match args::file_url_path(&arg) {
                    Some(path) => path,
                    None => arg.into_owned(),
                }
            })
            .collect()
    } else {
        std::env::args().skip(1).collect()
    };
    // A stable build answers `--version` anywhere, as it always did; the Nightly's forms were
    // answered before any startup effect (`prepare_launch`).
    if !nightly && args.iter().any(|a| a == "--version" || a == "-V") {
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
    // The control channel is the stable app's (agents, automation): a Nightly never opens one,
    // and never inherits the environment variable that would.
    let port = if nightly { None } else { args.control.or_else(|| std::env::var("FIGCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok())) };
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
    // default, never a build from source unless asked). Registering is the stable app's, so a
    // Nightly links nothing and never touches the registration.
    if !nightly {
        session.live.links = figcraft_collab_net::links::system();
        session.invite_links_startup();
    }
    let first_run = svc.prefs.is_none() && args.join.is_none();
    if session.docs.is_empty() && opens_new_file(first_run, session.prefs.start_on_home, port.is_some()) {
        let _ = session.execute("file.new", &json!({}));
    }
    // The interface language from the first frame on, and for the message when the window can't
    // open (no frame then).
    figcraft_ui_egui::i18n::set(&session.prefs.language, figcraft_ui_egui::Services::system_language(&svc).as_deref());
    let mut fig = FigApp::new(session, Box::new(svc));
    if nightly {
        // The Launcher keeps the Nightly on the version it installed.
        fig.disable_updates();
    } else {
        fig.set_startup_update_check(true);
    }
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
        .with_title(figcraft_ui_egui::channel::name())
        .with_app_id(window_app_id())
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
    let custom_titlebar = fig.ui.title_bar == figcraft_ui_egui::TitleBar::Custom;
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let mut options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, ..Default::default() };
    let graphics = graphics::Plan::configure(&mut options.wgpu_options);
    // Counts the frames drawn, which tells a window that failed starting up from one that ran.
    let window = ctx_slot.clone();
    figcraft_ui_egui::windows::install(fig);
    // Nightly session health: written before the window loop and finished on the controlled exit.
    // A secondary window of the same profile records nothing (no document names or diagnostics).
    let (nightly_record, health_warning) = match nightly_session::Session::begin_with_health(
        launch_profile::current().root(),
        env!("FIGCRAFT_BUILT_VERSION"),
        option_env!("FIGCRAFT_BUILD_SHA").filter(|sha| !sha.is_empty()),
    ) {
        Ok(state) => state,
        Err(error) => {
            log::warn!("Nightly session health is unavailable: {error}");
            (None, true)
        }
    };
    let previous_unclean = nightly_record.as_ref().is_some_and(nightly_session::Session::previous_unclean);
    let nightly_notice = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if previous_unclean || health_warning {
        let message = if previous_unclean {
            "The previous FigCraft Nightly session did not finish normally. You can report a problem from About. No diagnostics have been sent."
        } else {
            "FigCraft Nightly could not record its session health in its private profile. You can still edit, but detecting an abnormal exit is unavailable."
        };
        with_app(|fig| fig.toast(message));
    }
    // Acknowledged only when a frame draws the notice (`App::ui`), never by queueing it here.
    let nightly_notice_pending = previous_unclean;
    let nightly_record: nightly_session::Shared = Arc::new(Mutex::new(nightly_record));
    let exit_record = nightly_record.clone();
    let exit_notice = nightly_notice.clone();
    // Registered before the event loop starts, so it catches the Finder event that launched the app
    // as well as later ones. Lives until the event loop returns; the app creator only borrows it.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;
    let result = eframe::run_native(
        figcraft_ui_egui::channel::name(),
        options,
        Box::new(|cc| {
            // The UI runtime FigCraft's egui UI is hosted by, before the first pass.
            figcraft_ui_egui::framework::install(&cc.egui_ctx)?;
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
            if let Err(e) = craft_menu::mac::install_app_menu(&cc.egui_ctx, figcraft_ui_egui::channel::name()) {
                log::warn!("app menu: kept winit's Quit, which ends the app without asking: {e}");
            }
            Ok(Box::new(App {
                queue,
                ctx_slot,
                title: String::new(),
                recovery,
                nightly: exit_record,
                nightly_notice: exit_notice,
                nightly_notice_pending,
                // The custom title bar's undecorated window must stay borderless, or its caption
                // and border offset the content and the pointer on some Windows systems (#1343).
                window_frame: window_frame::WindowFrame::new(custom_titlebar),
                #[cfg(target_os = "macos")]
                apple_events: apple_events.connect(&cc.egui_ctx),
            }))
        }),
    );
    let frames = window.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map_or(0, egui::Context::cumulative_frame_nr);
    // The fallback for a start that never reached `on_exit`; elsewhere the exit hook did it. It runs
    // before the graphics handoff: on Unix the fallback `exec`s the restarted window from inside
    // `graphics.finish`, so a record finished after it would never be written and the restarted
    // window would report an unclean exit (and a Windows restart races the lease instead). A
    // renderer handoff is a controlled completion (`nightly_session::complete`).
    nightly_session::finish_once(&nightly_record, nightly_notice.load(std::sync::atomic::Ordering::Relaxed));
    // Start again on other graphics, or say why instead of quitting silently: release builds on
    // Windows have no console. A handoff start does not reach the window here.
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

#[cfg(test)]
mod refusal_dialog_tests {
    use super::refusal_dialog;
    use crate::build_flavor::BuildFlavor;

    #[test]
    fn a_refusal_never_blocks_on_a_modal_before_the_event_loop_on_macos() {
        assert!(refusal_dialog(BuildFlavor::NightlyV1, true, false), "a desktop refusal still shows the box where one renders");
        assert!(!refusal_dialog(BuildFlavor::NightlyV1, true, true), "macOS has no run loop yet: the modal would hang");
        assert!(!refusal_dialog(BuildFlavor::NightlyV1, false, false), "a headless session has no box");
        assert!(!refusal_dialog(BuildFlavor::Stable, true, false), "only a nightly refusal explains itself in a box");
    }
}
