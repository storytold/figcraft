//! FigCraft on Android — GameActivity + eframe/wgpu, reusing the desktop engine and egui UI.
//!
//! The engine, the ribbon, the grid and the theme are untouched: the same
//! [`figcraft_engine::Session`] and [`figcraft_ui_egui::FigApp`] the desktop shell builds are
//! wired up here. What the desktop shell gets from the OS, this crate takes from the activity:
//!
//! | Desktop (`apps/figcraft`) | Android (this crate) |
//! |---|---|
//! | `std::env::args()` (files, `--sample`) | none: the app starts on a new document |
//! | `config_dir()` from `HOME`/`XDG_CONFIG_HOME` | [`AndroidApp::internal_data_path`] |
//! | `rfd` file dialogs, `arboard` clipboard | not wired yet: `Services` is left at its default |
//! | `sys-locale` interface language | English until a language picker is wired |
//! | `craft-print` printers | none |
//! | desktop-sized UI (1440×900, hover menus) | [`TouchShell`] enlarges touch targets |
//!
//! Known gaps are listed in the crate README; none of them stop the UI from coming up.
#![cfg(target_os = "android")]

mod saf;
mod saf_services;

use std::path::PathBuf;

use figcraft_engine::Session;
use figcraft_ui_egui::FigApp;
use winit::platform::android::activity::AndroidApp;

/// How much to scale the desktop layout up on a phone.
///
/// Measured on a 1080×2400 @ ~420 dpi phone: egui's logical size is about 411×914 pt at
/// `pixels_per_point ≈ 2.625`, while the ribbon + grid layout is designed for ≥900 pt wide.
/// Scaling up only shrinks the logical viewport further and makes the ribbon overlap itself, so
/// the shell does not scale; the layout needs a touch rework (collapsed ribbon, bottom app),
/// not a zoom.
const TOUCH_SCALE: f32 = 1.0;

/// System font files that carry CJK, in preference order. egui's default fonts are Latin-only, so
/// without one of these every Chinese/Japanese/Korean glyph renders as a blank box.
const CJK_CANDIDATES: &[&str] = &[
    "/system/fonts/NotoSansCJK-Regular.ttc",
    "/system/fonts/NotoSansCJKsc-Regular.otf",
    "/system/fonts/NotoSansSC-Regular.otf",
    "/system/fonts/DroidSansFallbackFull.ttf",
    "/system/fonts/DroidSansFallback.ttf",
];

/// The Android entry point. winit's `android-activity` glue looks this symbol up in the cdylib,
/// so the name and the signature are fixed — hence `unsafe(no_mangle)` and the crate-level
/// `unsafe_code = "deny"` rather than the workspace's `forbid`.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("FigCraft"));
    log::info!("figcraft-android: starting (FigCraft {})", env!("CARGO_PKG_VERSION"));

    let data_dir = app.internal_data_path().or_else(|| app.external_data_path());
    if let Some(dir) = &data_dir {
        log::info!("figcraft-android: data dir {}", dir.display());
        std::fs::create_dir_all(dir).ok();
    }

    let mut session = Session::new();
    let (ctrl_tx, ctrl_rx) = std::sync::mpsc::channel::<figcraft_ui_egui::ControlRequest>();
    let activity = app.activity_as_ptr() as usize;
    let egui_ctx: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>> = std::sync::Arc::new(std::sync::Mutex::new(None));
    let services = saf_services::SafServices::new(activity, data_dir.clone(), std::sync::Arc::clone(&egui_ctx));
    let mut fa = FigApp::new(session, Box::new(services));
    // UI state is not persisted on Android yet: the desktop's ui.json/prefs.json pair is
    // not loaded or saved here.

    let options = eframe::NativeOptions {
        android_app: Some(app),
        viewport: egui::ViewportBuilder::default().with_title("FigCraft").with_app_id("ai.storyteller.figcraft"),
        ..Default::default()
    };

    let result = eframe::run_native(
        "FigCraft",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(TouchShell::new(fa, data_dir)))
        }),
    );
    if let Err(e) = result {
        log::error!("figcraft-android: eframe exited: {e}");
    }
}

/// Bind a system CJK face as the last fallback of every family. egui applies new definitions on
/// the next pass, so this is called once, before the first frame.
fn setup_fonts(ctx: &egui::Context) {
    let Some((path, bytes)) = CJK_CANDIDATES.iter().find_map(|p| std::fs::read(p).ok().map(|b| (*p, b))) else {
        log::warn!("figcraft-android: no CJK system font found; CJK text will render as boxes");
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("system-cjk".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("system-cjk".to_owned());
    }
    /* Android-port: the first frame can paint with the theme's `semibold` family before the theme
     * has installed its own font definitions, and epaint panics outright on an unbound family:
     *   FontFamily::Name("semibold") is not bound to any fonts
     * That panic unwinds out of run_native, android_main returns, and the activity disappears --
     * the app "crashes" on launch with the process briefly surviving. Bind the same faces as a
     * placeholder; the theme replaces them with the real semibold on its first pass. */
    if let Some(faces) = fonts.families.get(&egui::FontFamily::Proportional).cloned() {
        fonts.families.insert(egui::FontFamily::Name("semibold".into()), faces);
    }
    ctx.set_fonts(fonts);
    log::info!("figcraft-android: CJK fallback font {path}");
}

/// The eframe app: forward everything to [`FigApp`] and apply the phone adjustments.
struct TouchShell {
    app: FigApp,
    data_dir: Option<PathBuf>,
    /// The one-shot touch tuning has run (after FigCraft's own theme is in place).
    tuned: bool,
    /// FigCraft's theme owns the font families; the system CJK face is appended after the first
    /// `ui` pass, because replacing the font set before the theme installs would drop it.
    font_done: bool,
    /// FigCraft's font setup has run at least once.
    theme_fonts_live: bool,
}

impl TouchShell {
    fn new(app: FigApp, data_dir: Option<PathBuf>) -> Self {
        Self { app, data_dir, tuned: false, font_done: false, theme_fonts_live: false }
    }

    fn tune(ctx: &egui::Context) {
        let before = ctx.pixels_per_point();
        if (TOUCH_SCALE - 1.0).abs() > f32::EPSILON {
            ctx.set_pixels_per_point((before * TOUCH_SCALE).min(4.0));
        }
        ctx.all_styles_mut(|style| {
            // Touch targets: comfortable minimum, roomier padding, a little more separation so
            // neighbouring ribbon buttons are harder to hit by accident.
            style.spacing.interact_size = egui::vec2(40.0, 40.0);
            style.spacing.button_padding = egui::vec2(10.0, 8.0);
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.slider_width = style.spacing.slider_width.max(160.0);
            // A resting finger must not raise a tooltip.
            style.interaction.show_tooltips_only_when_still = true;
        });
        let rect = ctx.viewport_rect();
        log::info!("figcraft-android: touch mode applied (ppp {before}, logical {}×{} pt)", rect.width().round(), rect.height().round(),);
    }
}

impl eframe::App for TouchShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx);
        // The desktop shell keeps the window alive while a print job runs; on Android there is no
        // print backend, so only the close-request guard is worth mirroring.
        let _ = frame;
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {}

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        if !self.tuned {
            self.tuned = true;
            Self::tune(ui.ctx());
        }
        // The theme's own font definitions only become visible a frame after the theme install;
        // appending the CJK face any earlier would drop FigCraft's named families.
        if !self.font_done && self.theme_fonts_live {
            self.font_done = true;
            append_cjk_fallback(ui.ctx());
        }
        if !self.theme_fonts_live && ui.ctx().fonts(|f| f.definitions().families.keys().any(|k| matches!(k, egui::FontFamily::Name(_)))) {
            self.theme_fonts_live = true;
        }
    }

    fn on_exit(&mut self) {
        if let Some(dir) = &self.data_dir {}
    }
}

/// Append the system CJK face to whatever families the theme has installed, keeping the theme's
/// own order (its faces win; the system face only catches glyphs they lack).
fn append_cjk_fallback(ctx: &egui::Context) {
    let Some((path, bytes)) = CJK_CANDIDATES.iter().find_map(|p| std::fs::read(p).ok().map(|b| (*p, b))) else {
        return;
    };
    ctx.all_styles_mut(|_| {});
    let mut fonts = ctx.fonts(|f| f.definitions().clone());
    fonts.font_data.insert("system-cjk".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        if !list.iter().any(|n| n == "system-cjk") {
            list.push("system-cjk".to_owned());
        }
    }
    for family in fonts.families.keys().cloned().collect::<Vec<_>>() {
        if let egui::FontFamily::Name(_) = family {
            let list = fonts.families.entry(family).or_default();
            if !list.iter().any(|n| n == "system-cjk") {
                list.push("system-cjk".to_owned());
            }
        }
    }
    ctx.set_fonts(fonts);
    log::info!("figcraft-android: CJK fallback appended from {path}");
}
