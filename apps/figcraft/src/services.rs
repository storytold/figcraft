//! Desktop services: native file dialogs, clipboard, URLs, persisted preferences.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use figcraft_engine::{Prefs, Session};
use figcraft_ui_egui::{DialogFuture, FigApp, Services};
use serde_json::{Value, json};

pub struct Desktop {
    clipboard: Option<arboard::Clipboard>,
    egui_ctx: Option<egui::Context>,
    /// Where the preferences live ([`config_dir`]).
    dir: Option<PathBuf>,
    pub prefs: Option<Prefs>,
    /// The project library read at startup (`None`: none yet), what to tell about it, and why it
    /// must not be saved (its file is there but couldn't be read).
    pub projects: Option<figcraft_engine::projects::Projects>,
    pub projects_notice: Option<figcraft_ui_egui::ProjectsNotice>,
    projects_blocked: Option<String>,
    /// The system's reduce-motion setting (read in the background at startup).
    reduce_motion: Arc<AtomicBool>,
    /// The system's colour scheme ([`watch_system_dark`]): 0 unknown, 1 dark, 2 light.
    dark: Arc<AtomicU8>,
    /// The system's interface language (read at startup).
    language: Option<String>,
    /// Recovery copies of unsaved files (written at once after a panic).
    recovery: crate::recovery::Shared,
    /// The folder of the last file or folder a dialog picked, where the next one starts.
    last_folder: Arc<Mutex<Option<PathBuf>>>,
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
        let loaded = dir.as_deref().map(crate::projects_store::read).unwrap_or_default();
        let reduce_motion = Arc::new(AtomicBool::new(false));
        let flag = reduce_motion.clone();
        std::thread::spawn(move || flag.store(system_reduce_motion(), Ordering::Relaxed));
        let dark = Arc::new(AtomicU8::new(DARK_UNKNOWN));
        Self {
            clipboard: arboard::Clipboard::new().ok(),
            egui_ctx: None,
            dir,
            prefs,
            projects: loaded.projects,
            projects_notice: loaded.notice,
            projects_blocked: loaded.blocked,
            reduce_motion,
            dark,
            language: system_language(),
            recovery,
            last_folder: Arc::default(),
        }
    }

    /// winit reports no system theme on Linux and the BSDs: there the freedesktop colour scheme
    /// answers instead, read once and then watched in the background, calling `wake` when it
    /// changes (an idle window draws no frames). Elsewhere this does nothing.
    pub fn follow_system_dark(&self, wake: impl Fn() + Send + 'static) {
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Err(e) = std::thread::Builder::new().name("figcraft-theme".into()).spawn({
            let dark = self.dark.clone();
            move || watch_system_dark(&dark, &wake)
        }) {
            log::info!("colour scheme: can't start the watcher: {e}");
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        let _ = wake;
    }
}

/// The system's preferred interface languages, most preferred first, comma-separated
/// ("nl-NL,de-DE,en-US"): Windows' language list, macOS's AppleLanguages, `LANGUAGE` and the
/// locale variables elsewhere. The UI shows the first language in the list that has a catalog.
/// Windows keeps only its first preferred language (and English when it has no catalog): that
/// list is also the input-language list, so a language added for a keyboard (Deutsch on a Polish
/// system) must not decide the interface language.
fn system_language() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        windows::System::UserProfile::GlobalizationPreferences::Languages()
            .and_then(|l| l.GetAt(0))
            .map(|s| s.to_string())
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

/// [`Desktop::dark`]: the colour scheme is unknown, dark, or light.
const DARK_UNKNOWN: u8 = 0;
const DARK_DARK: u8 = 1;
const DARK_LIGHT: u8 = 2;

/// The desktop's colour scheme through the freedesktop settings portal
/// (`org.freedesktop.appearance` / `color-scheme`: 1 dark, 2 light, 0 no preference, which is
/// light), read once and then watched for changes, each one calling `wake`. Without a session bus
/// or a portal the scheme stays unknown and the UI keeps its light fallback.
#[cfg(all(unix, not(target_os = "macos")))]
fn watch_system_dark(dark: &AtomicU8, wake: &dyn Fn()) {
    use zbus::zvariant::Value;
    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    const SETTINGS: &str = "org.freedesktop.portal.Settings";
    const APPEARANCE: &str = "org.freedesktop.appearance";
    // `Read` (portals before 1.17, which lack `ReadOne`) wraps the value in one more variant.
    let scheme = |v: &Value| match v {
        Value::U32(x) => *x,
        Value::Value(inner) => match **inner {
            Value::U32(x) => x,
            _ => 0,
        },
        _ => 0,
    };
    let store = |v: u32| {
        let new = match v {
            1 => DARK_DARK,
            2 => DARK_LIGHT,
            _ => DARK_UNKNOWN,
        };
        if dark.swap(new, Ordering::Relaxed) != new {
            wake();
        }
    };
    let Ok(conn) = zbus::blocking::connection::Builder::session().and_then(|b| b.build()) else {
        log::info!("colour scheme: no session bus: the system theme isn't followed");
        return;
    };
    let read = |method: &str| -> zbus::Result<u32> {
        let reply = conn.call_method(Some(PORTAL), "/org/freedesktop/portal/desktop", Some(SETTINGS), method, &(APPEARANCE, "color-scheme"))?;
        let body = reply.body();
        Ok(scheme(&body.deserialize::<Value>()?))
    };
    match read("ReadOne").or_else(|_| read("Read")) {
        Ok(v) => store(v),
        Err(e) => log::info!("colour scheme: can't read the system setting: {e}"),
    }
    let watch = || -> zbus::Result<()> {
        let rule =
            zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender(PORTAL)?.interface(SETTINGS)?.member("SettingChanged")?.build();
        for msg in zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(8))? {
            let msg = msg?;
            let body = msg.body();
            let Ok((namespace, key, value)) = body.deserialize::<(String, String, Value<'_>)>() else { continue };
            if namespace == APPEARANCE && key == "color-scheme" {
                store(scheme(&value));
            }
        }
        Ok(())
    };
    if let Err(e) = watch() {
        log::info!("colour scheme: can't watch the system setting: {e}");
    }
}

/// Show `path` in the file manager: selected in Finder and Explorer, its folder elsewhere.
fn reveal(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    };
    #[cfg(windows)]
    let mut command = {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        let mut c = std::process::Command::new("explorer");
        c.arg(arg);
        c
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    // The file manager keeps running; reap it off this thread so it leaves no zombie.
    let mut child = command.spawn()?;
    let _ = std::thread::Builder::new().name("figcraft-reveal".into()).spawn(move || child.wait());
    Ok(())
}

/// The preferences' file in the config folder.
const STATE: &str = "state.json";

/// Everything quitting keeps, all done when it returns (macOS then ends the process): the
/// preferences in `dir`, this run's recovery copies ([`Recovery::quit`](crate::recovery::Recovery::quit))
/// and the live sessions' ends (one this app hosts tells everyone and stops; a joined one leaves).
pub fn shutdown(app: &mut FigApp, dir: Option<&Path>, recovery: Option<crate::recovery::Recovery>) {
    if let Some(dir) = dir {
        write_prefs(dir, &app.session.prefs);
    }
    // The library is kept as it changes; whatever is left unsaved is kept now.
    if std::mem::take(&mut app.session.projects.unsaved)
        && let Err(e) = app.services.save_projects(&app.session.projects)
    {
        log::warn!("projects: {e}");
    }
    // Before the sessions end: a recovered online file's copy waits for its session's outcome.
    if let Some(r) = recovery {
        r.quit(&app.session, app.ui.quit);
    }
    drop(std::mem::take(&mut app.session.live));
}

/// Write `prefs` to `dir` atomically (a temporary file renamed over the old one: a crash leaves
/// either version whole). They hold online owner tokens, so on Unix only the owner can read them.
pub(crate) fn write_prefs(dir: &Path, prefs: &Prefs) {
    let tmp = dir.join("state.json.tmp");
    let written = std::fs::create_dir_all(dir)
        .and_then(|()| write_private(&tmp, json!({"prefs": prefs}).to_string().as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, dir.join(STATE)));
    if let Err(e) = written {
        log::warn!("preferences: couldn't save them in {}: {e}", dir.display());
    }
}

/// Write `bytes` to `path`, owner read/write only on Unix from before the first byte (a leftover
/// file from a crash is narrowed too: the mode given at open only applies to new files).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(path)?;
    #[cfg(unix)]
    f.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    f.write_all(bytes)
}

thread_local! {
    /// While the main window draws: a dialog parented to it, so a dialog opened from it stays in
    /// front of it (a sheet on macOS). eframe gives other windows no handle: theirs have no parent.
    static PARENT: RefCell<Option<rfd::AsyncFileDialog>> = const { RefCell::new(None) };
}

/// Run the main window's frame `f` with the dialogs it opens parented to `frame`'s window.
pub fn parented<R>(frame: &eframe::Frame, f: impl FnOnce() -> R) -> R {
    PARENT.with(|p| p.replace(Some(rfd::AsyncFileDialog::new().set_parent(frame))));
    let r = f();
    PARENT.with(|p| p.replace(None));
    r
}

/// A dialog with `title`, parented to the main window while it draws.
fn dialog(title: &str) -> rfd::AsyncFileDialog {
    PARENT.with(|p| p.borrow().clone()).unwrap_or_default().set_title(title)
}

/// The dialog opens now; its future gives the path picked.
fn picked(pick: impl Future<Output = Option<rfd::FileHandle>> + 'static) -> DialogFuture {
    Box::pin(async move { pick.await.map(|f| f.path().to_path_buf()) })
}

/// How long a clipboard read may wait on the app that copied (Linux and the BSDs).
#[cfg(all(unix, not(target_os = "macos")))]
const CLIPBOARD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Read the clipboard. On Linux and the BSDs the read runs on a helper thread with its own handle
/// and gives up after [`CLIPBOARD_TIMEOUT`]: a Wayland read waits on the app that copied with no
/// timeout of its own, so an app that hangs must not hang the window too.
#[cfg(all(unix, not(target_os = "macos")))]
fn read_clipboard<T: Send + 'static>(_: &mut arboard::Clipboard, read: fn(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(arboard::Clipboard::new().ok().and_then(|mut c| read(&mut c)));
    });
    rx.recv_timeout(CLIPBOARD_TIMEOUT).ok().flatten()
}

/// Read the clipboard (the system's own reads don't wait on the app that copied).
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn read_clipboard<T>(c: &mut arboard::Clipboard, read: fn(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    read(c)
}

/// Native Save as filters; other callers keep their existing combined filter.
fn save_filters<'a>(exts: &'a [&'a str]) -> Vec<(&'static str, &'a [&'a str])> {
    if exts == ["figcraft", "fig"] {
        vec![("FigCraft (.figcraft)", &exts[..1]), (figcraft_ui_egui::i18n::tr(".fig (experimental)"), &exts[1..])]
    } else {
        vec![("File", exts)]
    }
}

/// The folder to start the next dialog in once `path` was picked: the file's folder, or the folder
/// picked itself.
pub(crate) fn folder_to_remember(path: &Path, is_folder: bool) -> Option<PathBuf> {
    let folder = if is_folder { Some(path) } else { path.parent() };
    folder.filter(|f| !f.as_os_str().is_empty()).map(Path::to_path_buf)
}

/// The remembered folder, unless it is gone since: then the system's own choice.
pub(crate) fn start_folder(last: Option<PathBuf>) -> Option<PathBuf> {
    last.filter(|f| f.is_dir())
}

impl Desktop {
    /// A dialog that starts in the last folder used. The Linux portal does not remember it between
    /// dialogs as macOS and Windows do.
    fn dialog_in_last_folder(&self, title: &str) -> rfd::AsyncFileDialog {
        let dialog = dialog(title);
        match start_folder(self.last_folder.lock().ok().and_then(|l| l.clone())) {
            Some(folder) => dialog.set_directory(folder),
            None => dialog,
        }
    }
}

/// `pick`, with the folder of what it picks kept in `last` for the next dialog.
pub(crate) fn remembering(last: Arc<Mutex<Option<PathBuf>>>, pick: DialogFuture, is_folder: bool) -> DialogFuture {
    Box::pin(async move {
        let path = pick.await?;
        if let (Some(folder), Ok(mut last)) = (folder_to_remember(&path, is_folder), last.lock()) {
            *last = Some(folder);
        }
        Some(path)
    })
}

impl Services for Desktop {
    fn pick_open(&mut self, title: &str, exts: &[&str]) -> DialogFuture {
        let pick = picked(self.dialog_in_last_folder(title).add_filter("Supported files", exts).pick_file());
        remembering(self.last_folder.clone(), pick, false)
    }
    fn pick_save(&mut self, title: &str, name: &str, exts: &[&str]) -> DialogFuture {
        let mut dialog = self.dialog_in_last_folder(title).set_file_name(name);
        for (label, extensions) in save_filters(exts) {
            dialog = dialog.add_filter(label, extensions);
        }
        let pick = picked(dialog.save_file());
        remembering(self.last_folder.clone(), pick, false)
    }
    fn pick_folder(&mut self, title: &str) -> DialogFuture {
        let pick = picked(self.dialog_in_last_folder(title).pick_folder());
        remembering(self.last_folder.clone(), pick, true)
    }
    fn clipboard_set_text(&mut self, s: &str) {
        if let Some(ctx) = &self.egui_ctx {
            ctx.copy_text(s.to_string());
        }
    }
    fn set_egui_context(&mut self, ctx: &egui::Context) {
        self.egui_ctx = Some(ctx.clone());
    }
    fn clipboard_get_text(&mut self) -> Option<String> {
        read_clipboard(self.clipboard.as_mut()?, |c| c.get_text().ok())
    }
    fn clipboard_get_html(&mut self) -> Option<String> {
        read_clipboard(self.clipboard.as_mut()?, |c| c.get().html().ok())
    }
    fn clipboard_set_png(&mut self, w: u32, h: u32, rgba: &[u8]) {
        if let Some(c) = self.clipboard.as_mut() {
            let _ = c.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(rgba) });
        }
    }
    fn clipboard_get_image(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let img = read_clipboard(self.clipboard.as_mut()?, |c| c.get_image().ok())?;
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
    fn reveal_path(&mut self, path: &Path) {
        if let Err(e) = reveal(path) {
            log::warn!("show in folder: {}: {e}", path.display());
        }
    }
    fn save_prefs(&mut self, prefs: &Prefs) {
        if let Some(dir) = &self.dir {
            write_prefs(dir, prefs);
        }
    }
    fn save_projects(&mut self, projects: &figcraft_engine::projects::Projects) -> Result<(), String> {
        if let Some(why) = &self.projects_blocked {
            return Err(why.clone());
        }
        match &self.dir {
            Some(dir) => crate::projects_store::write(dir, projects),
            None => Ok(()),
        }
    }
    fn emergency_save(&mut self, session: &Session) {
        if let Some(r) = crate::recovery::lock(&self.recovery).as_mut() {
            r.write(session, true);
            // Earlier runs' copies now covered go, so the next start doesn't open them twice.
            r.settle(session);
        }
    }
    fn reduce_motion(&self) -> bool {
        self.reduce_motion.load(Ordering::Relaxed)
    }
    fn system_dark(&self) -> Option<bool> {
        match self.dark.load(Ordering::Relaxed) {
            DARK_DARK => Some(true),
            DARK_LIGHT => Some(false),
            _ => None,
        }
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

#[cfg(test)]
mod save_filter_tests {
    use super::save_filters;

    #[test]
    fn native_and_fig_save_filters_are_separate_with_native_first() {
        let extensions = ["figcraft", "fig"];
        assert_eq!(
            save_filters(&extensions),
            vec![("FigCraft (.figcraft)", &extensions[..1]), (figcraft_ui_egui::i18n::tr(".fig (experimental)"), &extensions[1..])]
        );
    }

    #[test]
    fn other_save_filters_keep_one_combined_group() {
        for extensions in [&["figcraft"][..], &["fig"][..], &["png"][..], &["png", "jpg", "webp"][..], &[][..]] {
            assert_eq!(save_filters(extensions), vec![("File", extensions)]);
        }
    }
}
