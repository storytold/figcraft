//! Desktop services: native file dialogs, clipboard, URLs, persisted preferences.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use figcraft_engine::{Prefs, Session};
use figcraft_ui_egui::{FigApp, Services};
use serde_json::{Value, json};

pub struct Desktop {
    clipboard: Option<arboard::Clipboard>,
    /// Where the preferences live ([`config_dir`]).
    dir: Option<PathBuf>,
    pub prefs: Option<Prefs>,
    /// The system's reduce-motion setting (read in the background at startup).
    reduce_motion: Arc<AtomicBool>,
    /// The system's interface language (read at startup).
    language: Option<String>,
    /// Recovery copies of unsaved files (written at once after a panic).
    recovery: crate::recovery::Shared,
}

pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.map(|b| b.join("FigCraft"))
}

impl Desktop {
    pub fn load(recovery: crate::recovery::Shared) -> Self {
        Self::load_from(config_dir(), recovery)
    }

    /// With the preferences kept in `dir`.
    pub fn load_from(dir: Option<PathBuf>, recovery: crate::recovery::Shared) -> Self {
        let state: Value =
            dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(STATE)).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
        let prefs = state.get("prefs").and_then(|p| serde_json::from_value::<Prefs>(p.clone()).ok()).map(|mut p| {
            // Recent files lived next to the preferences before they moved into them.
            if p.recent_files.is_empty()
                && let Some(old) = state.get("recents").and_then(Value::as_array)
            {
                p.recent_files = old.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
            }
            p.recent_files.truncate(figcraft_engine::cmd::file::MAX_RECENT);
            p
        });
        let reduce_motion = Arc::new(AtomicBool::new(false));
        let flag = reduce_motion.clone();
        std::thread::spawn(move || flag.store(system_reduce_motion(), Ordering::Relaxed));
        Self { clipboard: arboard::Clipboard::new().ok(), dir, prefs, reduce_motion, language: system_language(), recovery }
    }
}

/// The system's preferred interface languages, most preferred first, comma-separated
/// ("nl-NL,de-DE,en-US"): Windows' language list, macOS's AppleLanguages, `LANGUAGE` and the
/// locale variables elsewhere. The UI shows the first one it has a catalog for.
fn system_language() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        windows::System::UserProfile::GlobalizationPreferences::Languages()
            .map(|l| l.into_iter().map(|s| s.to_string()).collect::<Vec<_>>().join(","))
            .inspect_err(|e| log::info!("language: can't read the system languages: {e}"))
            .ok()
    }
    // `defaults` prints a list: ( "es-ES", en ).
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok()?;
        let list: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .split(['(', ',', ')'])
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Some(list.join(","))
    }
    // gettext's `LANGUAGE` priority list ("fr:de"), then the locale ("fr_FR.UTF-8").
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX");
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(var);
        let list: Vec<String> = var("LANGUAGE").map(|l| l.replace(':', ",")).into_iter().chain(locale).collect();
        (!list.is_empty()).then(|| list.join(","))
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

/// Whether the system's accessibility settings ask for reduced motion: Windows' animation
/// effects (`UISettings.AnimationsEnabled`), macOS's Reduce motion, GNOME's animations.
fn system_reduce_motion() -> bool {
    #[cfg(target_os = "windows")]
    {
        match windows::UI::ViewManagement::UISettings::new().and_then(|s| s.AnimationsEnabled()) {
            Ok(on) => !on,
            Err(e) => {
                log::info!("reduce motion: can't read the animation setting: {e}");
                false
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        command_says("defaults", &["read", "com.apple.universalaccess", "reduceMotion"], "1")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        command_says("gsettings", &["get", "org.gnome.desktop.interface", "enable-animations"], "false")
    }
    #[cfg(not(any(windows, unix)))]
    {
        false
    }
}

/// Whether `program args` runs and prints `want` (trimmed).
#[cfg(unix)]
fn command_says(program: &str, args: &[&str], want: &str) -> bool {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == want)
}

/// The preferences' file in the config folder.
const STATE: &str = "state.json";

pub fn save_state(app: &FigApp) {
    if let Some(dir) = config_dir() {
        write_prefs(&dir, &app.session.prefs);
    }
}

/// Write `prefs` to `dir` atomically (a temporary file renamed over the old one: a crash leaves
/// either version whole).
fn write_prefs(dir: &Path, prefs: &Prefs) {
    let tmp = dir.join("state.json.tmp");
    let written = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&tmp, json!({"prefs": prefs}).to_string()))
        .and_then(|()| std::fs::rename(&tmp, dir.join(STATE)));
    if let Err(e) = written {
        log::warn!("preferences: couldn't save them in {}: {e}", dir.display());
    }
}

impl Services for Desktop {
    // Async dialogs: the synchronous `rfd` API runs a nested modal event loop, which re-enters
    // winit from inside the frame and panics on macOS when any further event (a file dragged onto
    // the app) arrives while the dialog is up.
    fn pick_open(&mut self, title: &str, exts: &[&str]) -> figcraft_ui_egui::DialogFuture {
        let d = rfd::AsyncFileDialog::new().set_title(title.to_string()).add_filter("Supported files", exts);
        Box::pin(async move { d.pick_file().await.map(|f| f.path().to_path_buf()) })
    }
    fn pick_save(&mut self, title: &str, name: &str, exts: &[&str]) -> figcraft_ui_egui::DialogFuture {
        let d = rfd::AsyncFileDialog::new().set_title(title.to_string()).set_file_name(name.to_string()).add_filter("File", exts);
        Box::pin(async move { d.save_file().await.map(|f| f.path().to_path_buf()) })
    }
    fn pick_folder(&mut self, title: &str) -> figcraft_ui_egui::DialogFuture {
        let d = rfd::AsyncFileDialog::new().set_title(title.to_string());
        Box::pin(async move { d.pick_folder().await.map(|f| f.path().to_path_buf()) })
    }
    fn clipboard_set_text(&mut self, s: &str) {
        if let Some(c) = self.clipboard.as_mut() {
            let _ = c.set_text(s.to_string());
        }
    }
    fn clipboard_get_text(&mut self) -> Option<String> {
        self.clipboard.as_mut()?.get_text().ok()
    }
    fn clipboard_set_png(&mut self, w: u32, h: u32, rgba: &[u8]) {
        if let Some(c) = self.clipboard.as_mut() {
            let _ = c.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(rgba) });
        }
    }
    fn clipboard_get_image(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let img = self.clipboard.as_mut()?.get_image().ok()?;
        let (w, h) = (u32::try_from(img.width).ok()?, u32::try_from(img.height).ok()?);
        // Refuse absurd sizes (and mismatched buffers) rather than allocate.
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 100_000_000 || img.bytes.len() != (w as usize) * (h as usize) * 4 {
            return None;
        }
        Some((w, h, img.bytes.into_owned()))
    }
    fn open_url(&mut self, url: &str) {
        let _ = webbrowser::open(url);
    }
    fn save_prefs(&mut self, prefs: &Prefs) {
        if let Some(dir) = &self.dir {
            write_prefs(dir, prefs);
        }
    }
    fn emergency_save(&mut self, session: &Session) {
        if let Some(r) = crate::recovery::lock(&self.recovery).as_mut() {
            r.write(session, true);
        }
    }
    fn reduce_motion(&self) -> bool {
        self.reduce_motion.load(Ordering::Relaxed)
    }
    fn gamepad_held(&mut self) -> u32 {
        crate::gamepad::held()
    }
    fn system_language(&self) -> Option<String> {
        self.language.clone()
    }
}

/// The window icon from the bundled app icon (if present).
pub fn window_icon() -> Option<egui::IconData> {
    // macOS wants Apple's padded grid; the other platforms the rounded tile edge to edge.
    #[cfg(target_os = "macos")]
    let bytes = include_bytes!("../../../assets/app-icon/figcraft-256.png");
    #[cfg(not(target_os = "macos"))]
    let bytes = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.figcraft.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width: w, height: h })
}
