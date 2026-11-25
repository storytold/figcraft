//! The craft-ui runtime, hosted from FigCraft's own eframe `App` (craft-libs docs/craft-framework.md, section 2.2).
//! The desktop app calls [`install`] once, [`FigApp::before_input`] in `raw_input_hook`, [`FigApp::close_path`] last
//! in `logic`, and [`before_ui`] first in `ui`; the browser build and tests install with [`install_headless`]. The
//! framework dialogs (the update dialogs) run on [`FigApp::framework_dialogs`] after FigCraft's own UI.
//!
//! The runtime takes Escape, Tab and the clipboard events for the whole window unless one of FigCraft's own egui
//! widgets has the keyboard. [`return_window_escape`] puts the window's Escape back into egui's input at the start of
//! each window's frame, so FigCraft's dialogs, menus, tools and shortcuts read it from egui as they read every other
//! key. The canvas is declared to the runtime with [`host_canvas`], which keeps the edit chords, the clipboard
//! events and FigCraft's Alt+digit panel keys in egui's input for its shortcuts while nothing has the keyboard; the
//! focused canvas keeps Tab and Escape too.

use std::sync::{Arc, OnceLock};

use craft_host::target::ShellInfo;
use craft_host::{AppInfo, Host};
use craft_keymap::{Chord, Mods, Overrides};
use craft_ui::fonts::{I18nScripts, SystemFonts, UiFonts};
use craft_ui::runtime::commands::{CommandInput, CommandSet as _, NavCommand, QuitCommand};
use craft_ui::runtime::env::{RootEnv, Scale};
use craft_ui::runtime::focus::{EditSet, KeepKeys, KeyUse};
use craft_ui::runtime::frame::{FrameInput, FrameOutput};
use craft_ui::runtime::keys::{Intent, with_window_intents};
use craft_ui::runtime::overlay::{self, OpenFilter};
use craft_ui::shell::embed;
use craft_window::system::WindowSystem;
use figcraft_engine::keymap::{KEYS, Platform};

use crate::FigApp;

/// Each pass of the root window, first in `App::ui`: hands the runtime the native window it answers the hit test of
/// (Windows) and fits the window to its monitor once.
#[cfg(not(target_arch = "wasm32"))]
pub use craft_ui::shell::embed::before_ui;

/// The id every OS window of the app carries, as the main window's builder sets it.
const APP_ID: &str = "ai.storyteller.figcraft";

/// The scripts of the interface languages, for the font stack.
static SCRIPTS: I18nScripts = I18nScripts;

/// The runtime's first font stack: egui's bundled faces and no installed system fonts. FigCraft's own definitions
/// ([`crate::theme::install_fonts`]) replace it before anything is drawn.
fn ui_fonts() -> UiFonts<'static> {
    UiFonts::new(&SCRIPTS).system_fonts(SystemFonts::Off)
}

fn install_with(ctx: &egui::Context, shell: ShellInfo, windows: Arc<dyn WindowSystem>) -> Result<(), String> {
    let spec = craft_ui::InstallSpec::new(shell, windows).app_id(APP_ID);
    embed::install(ctx, spec, &ui_fonts()).map(drop).map_err(|error| format!("the UI runtime could not be installed: {error}"))
}

/// Installs the runtime on the desktop window's context, once, before the first pass. FigCraft draws its own title
/// bar where it has one (`TitleBar::Custom` and `TitleBar::Inset`), so no frame decision is handed over and the
/// runtime answers no hit test of the window.
#[cfg(not(target_arch = "wasm32"))]
pub fn install(ctx: &egui::Context) -> Result<(), String> {
    let shell = ShellInfo::default().backend(craft_host::target::Backend::from_env());
    install_with(ctx, shell, Arc::new(craft_window::system::SystemWindows))
}

/// Installs the runtime on a context with no native window of its own: the browser build's canvas and test windows.
pub fn install_headless(ctx: &egui::Context) {
    if let Err(error) = install_with(ctx, ShellInfo::default(), Arc::new(craft_window::system::NoWindows)) {
        log::warn!("{error}");
    }
}

/// A context with the runtime installed and no native window, for the test windows that draw FigCraft's UI.
pub fn headless_context() -> egui::Context {
    let ctx = egui::Context::default();
    install_headless(&ctx);
    ctx
}

/// Who FigCraft is to the framework's dialogs (the update dialogs show its name and icon).
pub(crate) fn host() -> Arc<Host> {
    let builder = Host::builder(AppInfo { id: "figcraft", name: "FigCraft", version: env!("CARGO_PKG_VERSION") });
    #[cfg(not(target_arch = "wasm32"))]
    let builder = builder.icon_png(include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.figcraft.png"));
    Arc::new(builder.build())
}

/// F6 and Shift+F6 are FigCraft's own region moves (`view.nextRegion`, `view.previousRegion`), so the framework's pane
/// moves are unbound. So is the framework's Quit, which asks the main window to close: FigCraft's Ctrl+Q closes the
/// window it is pressed in ([`crate::windows::quit_key`]). Overrides are keyed by command id, which the command set
/// gives.
fn command_overrides() -> Arc<Overrides> {
    static OVERRIDES: OnceLock<Arc<Overrides>> = OnceLock::new();
    OVERRIDES
        .get_or_init(|| {
            let mut overrides = Overrides::new();
            overrides.insert(NavCommand::NextPane.id(), Vec::new());
            overrides.insert(NavCommand::PreviousPane.id(), Vec::new());
            overrides.insert(QuitCommand::Quit.id(), Vec::new());
            Arc::new(overrides)
        })
        .clone()
}

/// The Alt+digit chords FigCraft's keymap binds (the panel keys, Alt+1 Layers and the rest). On Windows and Linux
/// the runtime's Alt+numpad code entry takes every Alt+digit a canvas does not claim.
fn alt_digit_claims() -> &'static [Chord] {
    static CLAIMS: OnceLock<Vec<Chord>> = OnceLock::new();
    CLAIMS.get_or_init(|| {
        let mut claims: Vec<Chord> = KEYS
            .iter()
            .flat_map(|binding| binding.keys(Platform::Mac).iter().chain(binding.keys(Platform::Other)))
            .filter_map(|spec| Chord::parse(spec).ok())
            .filter(|chord| chord.mods == Mods::ALT && chord.key.is_digit())
            .collect();
        claims.sort();
        claims.dedup();
        claims
    })
}

impl FigApp {
    /// The framework's part of `App::raw_input_hook`, once per frame before the first pass: takes what the runtime
    /// gathered since the last frame and gives the coming passes the root values (theme mode, interface scale and
    /// direction).
    pub fn before_input(&mut self, ctx: &egui::Context) {
        embed::before_input(ctx, |output| self.frame_input(output));
    }

    /// The root values of this frame. FigCraft registers no commands with the runtime and lets it toggle no chrome, so
    /// the output carries nothing it acts on. The interface scale is the main window's plan
    /// ([`crate::ui_scale::plan`], recorded by its last pass): the runtime is the one writer of egui's zoom factor. A
    /// Ctrl+wheel stays a wheel event: FigCraft's canvas reads it itself (Preferences ▸ Scroll wheel zoom swaps it with
    /// the plain wheel).
    fn frame_input(&mut self, _output: FrameOutput) -> FrameInput {
        let mode = if self.ui.dark { craft_theme::Mode::Dark } else { craft_theme::Mode::Light };
        let direction = if self.ui.mirrored { craft_i18n::Direction::Rtl } else { craft_i18n::Direction::Ltr };
        let root = RootEnv::new().mode(mode).scale(Scale::new(self.ui.ui_scale.zoom)).direction(direction).ctrl_wheel_pinch(false);
        FrameInput::new(root).commands(CommandInput::new().overrides(command_overrides())).host(Arc::clone(&self.framework_host))
    }

    /// The framework's part of `App::logic`, last, after FigCraft's own logic: the main window's close path, decided
    /// by [`crate::windows::close_decision`].
    pub fn close_path(&mut self, ctx: &egui::Context) {
        embed::logic(ctx, || crate::windows::close_decision(self, ctx));
    }
}

/// Declares the canvas `id`, drawn in this pass, as its window's document canvas. With nothing focused it is the
/// window's key target: the edit chords, the clipboard events and the Alt+digit panel keys stay in egui's input for
/// FigCraft's shortcuts, as do the arrows, Enter and the page keys; a Tab moves the focus into the window. The focused
/// canvas has the same keys and Escape, and with `keeps_tab` Tab and Shift+Tab; without it they move the focus on.
pub(crate) fn host_canvas(ctx: &egui::Context, id: egui::Id, keeps_tab: bool) {
    let keys = KeyUse::CANVAS.with_edits(EditSet::ALL).with_claims(alt_digit_claims());
    let keys = if keeps_tab { keys.with_keep(KeepKeys::Tab) } else { keys };
    craft_ui::runtime::focus::host_canvas(ctx, id, keys);
}

/// Puts the window's Escape (an Escape while none of FigCraft's widgets and no framework component has the keyboard,
/// which the runtime routes to the window as its cancel) back into egui's input as a key press, first in the window's
/// frame, so FigCraft's dialogs, menus, tools and shortcuts read it from egui as they read every other key. While a
/// modal framework dialog is open the Escape is the dialog's, which reads it when it runs, after FigCraft's UI.
pub(crate) fn return_window_escape(ctx: &egui::Context) {
    if overlay::any_open(ctx, OpenFilter::Modal) {
        return;
    }
    let mut taken = false;
    with_window_intents(ctx, false, |intents| {
        taken = intents.take(Intent::Cancel);
        taken
    });
    if taken {
        ctx.input_mut(|i| {
            let modifiers = i.modifiers;
            let key = egui::Key::Escape;
            i.events.insert(0, egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers });
        });
    }
}
