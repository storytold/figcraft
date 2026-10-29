use super::{Arc, FigApp, Future, PathBuf, Pin, Session, Value, Wake, i18n, json};

pub const MAX_CLIPBOARD_PNG_BYTES: usize = 256 * 1024 * 1024;

/// Platform services the app crate provides (file dialogs, clipboard, URLs).
pub trait Services {
    fn pick_open(&mut self, _title: &str, _exts: &[&str]) -> DialogFuture {
        Box::pin(std::future::ready(None))
    }
    /// Pick a file and read it, where the platform hands over a file's name and contents rather
    /// than a path to it (the browser). `None`: the platform picks paths ([`Services::pick_open`]).
    fn pick_open_contents(&mut self, _title: &str, _exts: &[&str]) -> Option<ContentsFuture> {
        None
    }
    fn pick_save(&mut self, _title: &str, _name: &str, _exts: &[&str]) -> DialogFuture {
        Box::pin(std::future::ready(None))
    }
    /// The platform saves files by handing them over as downloads (the browser), not through
    /// save dialogs.
    fn downloads(&self) -> bool {
        false
    }
    /// Hand `bytes` over as a download named `name`; false when nothing was delivered.
    fn download(&mut self, _name: &str, _bytes: &[u8]) -> bool {
        false
    }
    /// A folder to write several files into.
    fn pick_folder(&mut self, _title: &str) -> DialogFuture {
        Box::pin(std::future::ready(None))
    }
    fn clipboard_set_text(&mut self, _s: &str) {}
    /// Give native services access to the window context for platform clipboard output.
    fn set_egui_context(&mut self, _ctx: &egui::Context) {}
    fn clipboard_get_text(&mut self) -> Option<String> {
        None
    }
    fn clipboard_get_html(&mut self) -> Option<String> {
        None
    }
    fn clipboard_set_png(&mut self, _w: u32, _h: u32, _rgba: &[u8]) {}
    /// An image on the system clipboard as (width, height, straight RGBA).
    fn clipboard_get_image(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        None
    }
    fn clipboard_get_png(&mut self) -> Option<Vec<u8>> {
        None
    }
    fn open_url(&mut self, _url: &str) {}
    /// Show the file at `path` in the platform's file manager (Finder, Explorer), selected where the
    /// platform can.
    fn reveal_path(&mut self, _path: &std::path::Path) {}
    /// The system asks for reduced motion (its accessibility setting).
    fn reduce_motion(&self) -> bool {
        false
    }
    /// The system asks for the dark interface (the freedesktop colour scheme): for platforms whose
    /// windowing stack doesn't report a theme (winit gives none on Linux and the BSDs).
    fn system_dark(&self) -> Option<bool> {
        None
    }
    /// After a panic in a frame: keep recovery copies of unsaved files.
    fn emergency_save(&mut self, _session: &Session) {}
    /// Keep the preferences now (the installed plugins or their settings changed), as the app
    /// does when it quits.
    fn save_prefs(&mut self, _prefs: &figcraft_engine::Prefs) {}
    /// The OS or browser language as a tag ("es-ES"), for the "system" language preference.
    fn system_language(&self) -> Option<String> {
        None
    }
    /// The controller buttons held now on any controller: bit i is standard gamepad layout index i
    /// (0 the bottom face button … 16 home; `figcraft_doc::proto::GAMEPAD_BUTTONS`). The prototype
    /// player asks every frame while its prototype has gamepad triggers.
    fn gamepad_held(&mut self) -> u32 {
        0
    }
    /// The fonts installed on this computer can be asked for and haven't been (the browser's Local
    /// Font Access; the desktop app sees them anyway).
    fn can_ask_for_local_fonts(&self) -> bool {
        false
    }
    /// Ask for them (the browser asks the person): they join the font list by name, each read when
    /// text uses it.
    fn ask_for_local_fonts(&mut self) {}
}

pub type DialogFuture = Pin<Box<dyn Future<Output = Option<PathBuf>>>>;

/// A picked file's name and contents ([`Services::pick_open_contents`]); `None` when the picker
/// was cancelled, `Err` with what to tell the user when the file couldn't be read.
pub type ContentsFuture = Pin<Box<dyn Future<Output = Option<Result<(String, Vec<u8>), String>>>>>;

/// A dropped file's contents, read once they arrive (`Err`: what to tell the user when they
/// couldn't be read).
pub type ReadFuture = Pin<Box<dyn Future<Output = Result<Vec<u8>, String>>>>;

pub(crate) type PickFuture = Pin<Box<dyn Future<Output = Option<Picked>>>>;

pub(crate) type DialogDone = Box<dyn FnOnce(&mut FigApp, Picked)>;

/// What a dialog picked: a path, or a file's name and contents where there are no paths, or why
/// the picked file couldn't be read.
pub(crate) enum Picked {
    Path(PathBuf),
    Contents(String, Vec<u8>),
    Unreadable(String),
}

impl Picked {
    /// The file as `file.open` and `file.import` take it (`Err`: what to tell the user instead).
    pub(crate) fn file_params(self) -> Result<Value, String> {
        match self {
            Picked::Path(p) => Ok(json!({"path": p.to_string_lossy()})),
            Picked::Contents(name, bytes) => {
                use base64::Engine as _;
                Ok(json!({"data": base64::engine::general_purpose::STANDARD.encode(&bytes), "name": name}))
            }
            Picked::Unreadable(why) => Err(why),
        }
    }
}

/// What the user is told about a file whose contents couldn't be read.
pub fn unreadable(name: &str) -> String {
    i18n::trf("Couldn't read {name}.", &[("name", &name)])
}

/// What the page says once the browser build stopped on an internal error (in the browser a panic
/// ends the page): `unsaved` files had unsaved changes and `kept` of them fitted in its storage.
pub fn browser_stopped(unsaved: usize, kept: usize) -> String {
    let next = if unsaved == 0 {
        i18n::tr("Reload the page to start again.")
    } else if kept >= unsaved {
        i18n::tr("Reload the page to get your unsaved files back.")
    } else if kept == 0 {
        i18n::tr("Your unsaved files were too large for the browser to keep.")
    } else {
        i18n::tr("Reload the page to get back the unsaved files the browser had room for.")
    };
    format!("{} {next}", i18n::tr("FigCraft stopped because of an internal error."))
}

/// The largest file read in the browser: wasm has at most 4 GiB of memory, and a file's contents
/// are held about three times on their way to `file.open` (the bytes, base64, decoded again).
/// As large as one member of a `.figcraft` file may be.
const MAX_BROWSER_FILE: f64 = (512u64 << 20) as f64;

/// Checks a file of `size` bytes before the browser reads it (`Err`: what to tell the user).
pub fn browser_can_read(name: &str, size: f64) -> Result<(), String> {
    if size.is_nan() || size > MAX_BROWSER_FILE {
        return Err(i18n::trf("{name} is too large to open in the browser (over 512 MB).", &[("name", &name)]));
    }
    Ok(())
}

/// Wakes the main window, which hands the result on to the window it belongs to.
pub(crate) struct RepaintWaker(pub(crate) egui::Context);

impl Wake for RepaintWaker {
    fn wake(self: Arc<Self>) {
        self.0.request_repaint_of(egui::ViewportId::ROOT);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.request_repaint_of(egui::ViewportId::ROOT);
    }
}

/// No-op services (tests, headless screenshots).
pub struct NoServices;

impl Services for NoServices {}

/// Put `text` on the system clipboard through egui's platform output. Native integrations handle
/// it directly; the browser shell writes it while handling the copy event.
pub fn egui_copy_text(ctx: &egui::Context, text: &str) {
    ctx.copy_text(text.to_string());
}

/// Put a straight-RGBA image on the system clipboard through egui's output (see
/// [`egui_copy_text`]); a buffer that doesn't match its size is dropped.
pub fn egui_copy_png(ctx: &egui::Context, w: u32, h: u32, rgba: &[u8]) {
    let (w, h) = (w as usize, h as usize);
    if w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(rgba.len()) {
        return;
    }
    ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied([w, h], rgba));
}
