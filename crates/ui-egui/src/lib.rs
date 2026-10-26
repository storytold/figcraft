//! The FigCraft egui frontend.
//!
//! A thin shell over [`figcraft_engine::Session`]: panels read engine state and act by running
//! commands; the canvas forwards pointer events to the engine's tools and draws overlays. Nothing
//! below this crate knows about egui, so the frontend can be replaced.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod a11y;
pub mod assets_panel;
pub mod bidi;
pub mod board_bar;
pub mod board_facilitate;
mod board_picker;
pub mod branch_ui;
pub mod canvas;
pub mod cjk;
pub mod colorpicker;
pub mod comments;
pub mod component_panel;
pub mod control;
pub mod credits;
pub mod cursors;
pub mod dev_compare;
pub mod dev_mode;
pub mod dialogs;
pub mod export_panel;
pub mod field_math;
pub mod font_preview;
pub mod frame_labels;
pub mod frame_presets;
pub mod frame_stats;
pub mod guides;
pub mod handles;
pub mod home;
pub mod home_projects;
pub mod home_recents;
pub mod i18n;
pub mod icons;
pub mod import_report;
pub mod layers;
pub mod layout_panel;
pub mod list_drag;
pub mod live;
pub mod live_text;
pub mod measure_overlay;
pub mod menus;
pub mod missing_fonts;
pub mod online;
pub mod pending_open;
pub mod plugins_ui;
pub mod present;
pub mod proto_canvas;
pub mod proto_editor;
pub mod proto_panel;
pub mod render_worker;
pub mod right;
pub mod scale_panel;
pub mod section_panel;
pub mod selection_colors;
mod selection_spacing;
pub mod shortcuts;
pub mod shortcuts_panel;
pub mod style_picker;
pub mod tabs;
pub mod theme;
pub mod toolbar;
pub mod typography;
pub mod ui_scale;
pub mod updates_ui;
pub mod variable_ui;
pub mod variables_panel;
pub mod vector_panel;
pub mod versions;
pub mod widget_bar;
pub mod widgets;
pub mod windows;

#[cfg(test)]
mod board_picker_tests;
#[cfg(test)]
mod board_ui_tests;
#[cfg(test)]
mod theme_startup_tests;

#[cfg(test)]
mod a11y_tests;
#[cfg(test)]
mod async_dialog_tests;
#[cfg(test)]
mod branch_ui_tests;
#[cfg(test)]
mod canvas_double_click_tests;
#[cfg(test)]
mod canvas_drag_tests;
#[cfg(test)]
mod canvas_fixes_tests;
#[cfg(test)]
mod canvas_image_rect_tests;
#[cfg(test)]
mod clipboard_fence_tests;
#[cfg(test)]
mod colorpicker_eyedropper_tests;
#[cfg(test)]
mod colorpicker_selection_tests;
#[cfg(test)]
mod comments_tests;
#[cfg(test)]
mod component_props_reorder_tests;
#[cfg(test)]
mod component_swap_default_tests;
#[cfg(test)]
mod context_menu_tests;
#[cfg(test)]
mod corner_individual_tests;
#[cfg(test)]
mod cursors_tests;
#[cfg(test)]
mod design_paste_tests;
#[cfg(test)]
mod dev_mode_tests;
#[cfg(test)]
mod devmode_tail_tests;
#[cfg(test)]
mod duplicate_paint_tests;
#[cfg(test)]
mod export_preview_tests;
#[cfg(test)]
mod field_tests;
#[cfg(test)]
mod frame_label_tests;
#[cfg(test)]
mod gap_handle_tests;
#[cfg(test)]
mod gradient_shorthand_tests;
#[cfg(test)]
mod guide_release_tests;
#[cfg(test)]
mod home_projects_tests;
#[cfg(test)]
mod home_recents_tests;
#[cfg(test)]
mod home_tests;
#[cfg(test)]
mod idle_tests;
#[cfg(test)]
mod ime_tests;
#[cfg(test)]
mod import_report_tests;
#[cfg(test)]
mod layers_behaviour_tests;
#[cfg(test)]
mod layers_drag_tests;
#[cfg(test)]
mod layers_keyboard_tests;
#[cfg(test)]
mod layers_keys_tests;
#[cfg(test)]
mod layers_rename_tests;
#[cfg(test)]
mod live_chat_tests;
#[cfg(test)]
mod live_docs_tests;
#[cfg(test)]
mod live_join_tests;
#[cfg(test)]
mod live_server_tests;
#[cfg(test)]
mod live_share_tests;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod live_text_tests;
#[cfg(test)]
mod measure_nudge_tests;
#[cfg(test)]
mod menu_aim_tests;
#[cfg(test)]
mod menu_contrast_tests;
#[cfg(test)]
mod menu_input_tests;
#[cfg(test)]
mod missing_fonts_tests;
#[cfg(test)]
mod online_tests;
#[cfg(test)]
mod open_issues_tests;
#[cfg(test)]
mod padding_individual_tests;
#[cfg(test)]
mod palette_tests;
#[cfg(test)]
mod panel_remove_tests;
#[cfg(test)]
mod panel_reorder_tests;
#[cfg(test)]
mod panel_width_tests;
#[cfg(test)]
mod paste_image_tests;
#[cfg(test)]
mod pen_mode_tests;
#[cfg(test)]
mod perf_tests;
#[cfg(test)]
mod plugin_panel_tests;
#[cfg(test)]
mod plugin_save_ui_tests;
#[cfg(test)]
mod plugins_browse_tests;
#[cfg(test)]
mod popup_width_tests;
#[cfg(test)]
mod present_links_tests;
#[cfg(test)]
mod present_tab_tests;
#[cfg(test)]
mod present_tests;
#[cfg(test)]
mod present_view_tests;
#[cfg(test)]
mod property_menu_tests;
#[cfg(test)]
mod proto_editor_tests;
#[cfg(test)]
mod proto_reorder_tests;
#[cfg(test)]
mod proto_responsive_tests;
#[cfg(test)]
mod resources_tests;
#[cfg(test)]
mod save_as_tests;
#[cfg(test)]
mod select_all_tests;
#[cfg(test)]
mod shortcut_keys_tests;
#[cfg(test)]
mod shortcuts_tests;
#[cfg(test)]
mod style_folder_rename_tests;
#[cfg(test)]
mod text_enter_tests;
#[cfg(test)]
mod titlebar_tests;
#[cfg(test)]
mod ui_scale_tests;
#[cfg(test)]
mod variables_export_ui_tests;
#[cfg(test)]
mod variables_rename_tests;
#[cfg(test)]
mod variants_discovery_tests;
#[cfg(test)]
mod vector_keys_tests;
#[cfg(test)]
mod vector_panel_tests;
#[cfg(test)]
mod vector_tail_ui_tests;
#[cfg(test)]
mod web_clipboard_tests;
#[cfg(test)]
mod web_save_tests;
#[cfg(test)]
mod windows_qa_tests;
#[cfg(test)]
mod windows_tests;

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll, Wake, Waker};

use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

pub use control::{ControlReply, ControlRequest, RequestGate, RequestState};
pub use home_projects::ProjectsNotice;

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
    /// Keep the project library now (folders, favourites); `Err`: why it wasn't kept.
    fn save_projects(&mut self, _projects: &figcraft_engine::projects::Projects) -> Result<(), String> {
        Ok(())
    }
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
}

pub type DialogFuture = Pin<Box<dyn Future<Output = Option<PathBuf>>>>;
/// A picked file's name and contents ([`Services::pick_open_contents`]); `None` when the picker
/// was cancelled, `Err` with what to tell the user when the file couldn't be read.
pub type ContentsFuture = Pin<Box<dyn Future<Output = Option<Result<(String, Vec<u8>), String>>>>>;
/// A dropped file's contents, read once they arrive (`Err`: what to tell the user when they
/// couldn't be read).
pub type ReadFuture = Pin<Box<dyn Future<Output = Result<Vec<u8>, String>>>>;
type PickFuture = Pin<Box<dyn Future<Output = Option<Picked>>>>;
type DialogDone = Box<dyn FnOnce(&mut FigApp, Picked)>;

/// What a dialog picked: a path, or a file's name and contents where there are no paths, or why
/// the picked file couldn't be read.
enum Picked {
    Path(PathBuf),
    Contents(String, Vec<u8>),
    Unreadable(String),
}

impl Picked {
    /// The file as `file.open` and `file.import` take it (`Err`: what to tell the user instead).
    fn file_params(self) -> Result<Value, String> {
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

/// A native dialog in flight. The app keeps working while it is open, so its result goes to the
/// file and window it was opened for, not to whichever is in front when it closes.
struct PendingDialog {
    future: PickFuture,
    /// What was picked, kept until the window that holds `doc` draws.
    picked: Option<Picked>,
    done: DialogDone,
    /// The file it acts on (`None`: none, such as Open).
    doc: Option<u64>,
    /// The window it was opened from.
    window: u64,
}

/// Wakes the main window, which hands the result on to the window it belongs to.
struct RepaintWaker(egui::Context);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum RightTab {
    Design,
    Prototype,
    Inspect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum LeftTab {
    File,
    Assets,
}

/// The window's title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TitleBar {
    /// The system draws it (and the browser): the tab strip sits below.
    #[default]
    Native,
    /// A frameless window: the tab strip is the title bar, with our own window buttons and edges.
    Custom,
    /// macOS: the system's window buttons inset at the left of the tab strip.
    Inset,
}

/// How often open documents get an autosave checkpoint (seconds).
pub const AUTOSAVE_SECS: f64 = 30.0 * 60.0;

/// Most changed layers whose names are checked for CJK scripts one by one (beyond: all of them).
const CJK_CHANGES: usize = 10_000;

/// With auto-save on: how long after a file last changed it is saved, and how long a failed save
/// waits before trying again (seconds).
pub const AUTO_SAVE_SECS: f64 = 3.0;
const AUTO_SAVE_RETRY_SECS: f64 = 60.0;

/// A file being encoded for auto-save: its snapshot and, once done, its bytes.
pub type AutoSaveEncoding = std::thread::JoinHandle<(figcraft_engine::cmd::file::FileSnapshot, figcraft_engine::Result<Vec<u8>>)>;

/// How long a toast shows (seconds).
const TOAST_SECS: f64 = 3.0;

/// How long a toast has been showing ([`UiState::toast`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToastClock {
    /// Raised, not drawn yet.
    Raised,
    /// Drawn once; the next frame starts the clock.
    Drawn,
    /// Showing since this time (UI clock, seconds).
    Since(f64),
}

/// Draw another frame `secs` from now (nothing else needs drawing until then: an idle window draws
/// no frames).
#[track_caller]
pub(crate) fn repaint_in(ctx: &egui::Context, secs: f64) {
    if secs.is_finite() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(secs.max(0.0)));
    }
}

/// A monotonic clock for background work (autosave, recovery, capture deadlines).
///
/// `Context::input(|i| i.time)` is the UI pass's time and is stale in `run_logic`; wall-clock time
/// can jump. `web_time::Instant` is monotonic on native and in the browser. Tests drive deadlines
/// directly with [`FrameClock::set_test_now`].
pub struct FrameClock {
    origin: web_time::Instant,
    test_now: Option<f64>,
}

impl Default for FrameClock {
    /// Start the frame clock at the current web time with no injected test time.
    fn default() -> Self {
        Self { origin: web_time::Instant::now(), test_now: None }
    }
}

impl FrameClock {
    /// Seconds since the app started (or the injected test time).
    pub fn now(&self) -> f64 {
        self.test_now.unwrap_or_else(|| self.origin.elapsed().as_secs_f64())
    }

    /// Set a fake monotonic time (tests); `None` returns to the real clock.
    pub fn set_test_now(&mut self, secs: Option<f64>) {
        self.test_now = secs;
    }
}

/// The side panels' widths: the least and most a drag of their edge gives, what a double-click on
/// the edge puts back, and how much of the window they leave the canvas.
pub const LEFT_PANEL: std::ops::RangeInclusive<f32> = 200.0..=480.0;
pub const RIGHT_PANEL: std::ops::RangeInclusive<f32> = 240.0..=480.0;
pub const PANEL_DEFAULT: f32 = 240.0;
/// Windows narrower than this (logical points: phones, portrait tablets) show one side panel at a
/// time over the canvas, opened from two buttons, instead of both beside it.
pub const NARROW_WIDTH: f32 = 760.0;
/// What an open side panel leaves of a narrow window: the column of panel buttons.
const MOBILE_STRIP: f32 = 48.0;

/// The side panel a narrow window shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MobilePanel {
    Left,
    Right,
}

impl UiState {
    /// Opens the left panel; a narrow window shows it in place of the right one.
    pub fn open_left_panel(&mut self) {
        self.left_collapsed = false;
        self.mobile_panel = Some(MobilePanel::Left);
    }

    /// Closes the left panel.
    pub fn close_left_panel(&mut self) {
        self.left_collapsed = true;
        if self.mobile_panel == Some(MobilePanel::Left) {
            self.mobile_panel = None;
        }
    }
}
pub const CANVAS_MIN: f32 = 160.0;

/// The widths the side panels are drawn at in `avail` points (the window less the icon rail), from
/// the widths asked for (none while a panel is closed, which takes no room): each within its
/// limits, and narrower in a small window so the canvas keeps [`CANVAS_MIN`] (the right panel
/// gives way last).
pub fn fit_panels(avail: f32, left: Option<f32>, right: Option<f32>) -> (f32, f32) {
    let fit = |w: f32, range: &std::ops::RangeInclusive<f32>, room: f32| {
        let w = if w.is_finite() { w } else { PANEL_DEFAULT };
        w.clamp(*range.start(), room.clamp(*range.start(), *range.end()))
    };
    let left_min = left.map_or(0.0, |_| *LEFT_PANEL.start());
    let right = right.map_or(0.0, |r| fit(r, &RIGHT_PANEL, avail - left_min - CANVAS_MIN));
    (left.map_or(0.0, |l| fit(l, &LEFT_PANEL, avail - right - CANVAS_MIN)), right)
}

/// UI-only state (serialisable for `ui.inspect`).
pub struct UiState {
    /// The widths asked for the side panels (a drag of their edge; [`fit_panels`] gives what a
    /// small window draws).
    pub left_width: f32,
    pub right_width: f32,
    pub right_tab: RightTab,
    /// The tab of the previous frame, to see the right panel change mode.
    pub last_right_tab: RightTab,
    pub left_tab: LeftTab,
    pub pages_open: bool,
    /// Pages panel search: open, and the case-insensitive name filter.
    pub page_search: bool,
    pub page_filter: String,
    /// The height asked for the pages list (`None`: a share of the panel).
    pub pages_height: Option<f32>,
    /// The document and active page last drawn in the pages list.
    pub pages_seen_page: Option<(u64, NodeId)>,
    pub expanded: BTreeSet<NodeId>,
    /// The layer being renamed in the layers panel.
    pub rename: Option<layers::Rename>,
    pub rename_page: Option<(NodeId, String)>,
    /// The frame or section whose name label on the canvas is being renamed (a double-click on it).
    pub rename_label: Option<layers::Rename>,
    pub main_menu: bool,
    pub palette: Option<String>,
    /// The Actions palette's tab.
    pub palette_tab: menus::PaletteTab,
    /// The palette row Up and Down are on (Enter runs it).
    pub palette_hi: usize,
    /// The palette ignores one outside click after opening (the click that opened it).
    pub palette_armed: bool,
    /// The context menu has been drawn at least once (its opening frame's press is spared).
    pub context_menu_shown: bool,
    /// This frame's Actions toolbar button rect: presses on it toggle, never dismiss.
    pub actions_rect: Option<egui::Rect>,
    /// The Variables editor, when open.
    pub variables: Option<variables_panel::Editor>,
    pub about: bool,
    /// The tab the About window shows.
    pub about_tab: dialogs::AboutTab,
    pub preferences: bool,
    /// A menu command that needs a value before the engine can run it.
    pub menu_input: Option<dialogs::MenuInput>,
    /// File ▸ Rename: the file name in the left panel starts its inline rename on the next frame.
    pub rename_file: bool,
    /// What the interface size drew last frame and whether the window held it back. Set by
    /// [`FigApp::logic`] for the main window.
    pub ui_scale: ui_scale::Plan,
    pub shortcuts_panel: bool,
    /// The Keyboard shortcuts panel's tab and search text.
    pub shortcuts_tab: figcraft_engine::keymap::Group,
    pub shortcuts_query: String,
    /// An opacity digit just typed and when (a second one soon after makes an exact percentage).
    pub opacity_digit: Option<(u32, f64)>,
    pub home: bool,
    /// The home screen's section, and the Templates search and filter.
    pub home_view: home::Home,
    /// How the window's title bar is drawn (the desktop app sets it when it opens the window).
    pub title_bar: TitleBar,
    /// Every icon on one screen, for reviewing the set (control channel `ui.set {iconGallery}`).
    pub icon_gallery: bool,
    /// A message at the bottom of the window and how long it has been showing.
    pub toast: Option<(String, ToastClock)>,
    pub present: Option<present::Presenter>,
    pub presents: Vec<present::PresentTab>,
    /// The tab shown last. It is on screen while `present` has a value.
    pub present_active: Option<u64>,
    pub present_next: u64,
    /// The full-window presenter's options (how it sizes the prototype), kept between
    /// presentations.
    pub present_view: present::PresentView,
    /// The prototype preview pane floating over the canvas, when open.
    pub preview: Option<present::Pane>,
    pub picker: Option<colorpicker::PickerTarget>,
    pub context_menu: Option<(egui::Pos2, Vec<NodeId>)>,
    /// The context menu was asked for from the keyboard (`view.contextMenu`): it opens at the
    /// focused layer row or the canvas selection and takes the keyboard.
    pub key_menu: bool,
    /// A selection box drawn from the keyboard (`select.area`), page `[x0, y0, x1, y1]` with the
    /// marquee's start at `x0, y0`.
    pub key_box: Option<[f64; 4]>,
    /// Layers panel search: open, the filter text, the shift-click range anchor and the selection
    /// the tree last revealed (ancestors expand and the row scrolls into view when it changes).
    pub layer_search: bool,
    pub layer_filter: String,
    pub layer_anchor: Option<NodeId>,
    /// The layers panel has the keyboard: arrow keys move through its rows instead of nudging
    /// (set by a click on a row, cleared by a press on the canvas).
    pub layers_focus: bool,
    /// The file (its `uid`) whose pages list has the keyboard: Delete with no layers selected deletes
    /// its current page (set by a click on a page row or the add-page button, cleared by a click on a
    /// layer row or a press on the canvas).
    pub pages_focus: Option<u64>,
    pub layers_seen_sel: Vec<NodeId>,
    pub dark: bool,
    pub follow_system: bool,
    pub tool_flyout: Option<&'static str>,
    /// On a board: the Design panel is shown (boards hide it until asked for).
    pub board_panel: bool,
    pub last_error: Option<String>,
    /// Files dropped on the home screen or the canvas, opened once their card has been on screen
    /// ([`pending_open::pending_opens`]), and the frames it has been drawn.
    pub pending_opens: Vec<pending_open::PendingOpen>,
    pub pending_open_frames: u8,
    /// The latest successful file import's summary and compatibility notes.
    pub import_report: Option<import_report::Report>,
    /// Files the user opened whose fonts are yet to be checked (by document uid).
    pub font_checks: Vec<u64>,
    pub missing_fonts: Option<missing_fonts::Shown>,
    pub screenshot_requests: Vec<(u64, String)>,
    pub frame_ms: f64,
    pub render_ms: f64,
    pub quit: bool,
    /// The Export dialog is open.
    pub export_preview: bool,
    /// The Export dialog's PDF print options (bleed, marks, CMYK).
    pub pdf_print: figcraft_doc::PrintSettings,
    /// The Color profile dialog is open, with this profile chosen.
    pub color_profile: Option<figcraft_color::ColorProfile>,
    /// The open style picker.
    pub style_picker: Option<style_picker::Picker>,
    /// The style picker shows colour styles as a grid of swatches.
    pub style_grid: bool,
    /// Style folders closed in the style picker.
    pub style_folds: BTreeSet<String>,
    /// The live text preview the panels show (`text.preview`'s command and params) and the
    /// document revision it was shown at: it ends the first frame nothing asks for it.
    pub text_preview: Option<(String, Value, u64)>,
    /// A panel asked for the preview this frame.
    pub text_preview_wanted: bool,
    /// The Design panel's Export section shows its preview.
    pub export_thumb: bool,
    pub zoom_menu: bool,
    pub find: Option<(String, String)>,
    /// The Rename layers dialog (several layers), while open.
    pub batch_rename: Option<dialogs::BatchRenameUi>,
    pub left_collapsed: bool,
    /// The side panel open in a narrow window (a phone), where only one shows at a time.
    pub mobile_panel: Option<MobilePanel>,
    /// The window was narrow (one side panel at a time) at the last frame.
    pub narrow: bool,
    pub comments: comments::CommentUi,
    pub versions: versions::VersionUi,
    /// The branch review dialog, while open.
    pub branch_review: Option<branch_ui::ReviewView>,
    /// Dev Mode: annotations on the canvas and the ready-for-dev list.
    pub dev: dev_mode::DevUi,

    /// What FigCraft last put on the system clipboard: when the clipboard still holds it, a paste
    /// uses our copied layers; anything else there (an image, SVG, text) came from another app.
    /// `None` also means a programmatic copy changed the engine clipboard without writing the OS
    /// clipboard, so the system text is no longer known to be ours.
    pub last_copy: Option<String>,
    /// The engine clipboard's layers generation when `last_copy` was mirrored. A programmatic
    /// copy changes the engine clipboard without writing the OS clipboard; the generation then
    /// no longer matches, so the stale mirror is not mistaken for the user's own copy.
    pub last_copy_generation: u64,
    /// Since the last V key release: whether its press arrived, and whether a paste event did
    /// (`shortcuts::swallowed_paste`).
    pub v_pressed: bool,
    pub paste_event: bool,
    /// The window got the focus back since the last V release.
    pub refocused: bool,
    /// A save shortcut pressed while a panel field had the keyboard: the field lets go (which
    /// commits what was typed) and the command runs next frame for that document/window.
    pub after_field: Option<shortcuts::AfterFieldSave>,
    /// The modifiers held, as of the last modifier change in the raw events: a paste event doesn't
    /// carry them.
    pub held: egui::Modifiers,
    /// When (monotonic [`FrameClock`], seconds) open documents next get an autosave checkpoint.
    pub next_autosave: f64,
    /// With auto-save on: when (monotonic [`FrameClock`], seconds) files with unsaved changes are next saved, and
    /// the revisions of those files it waits on (a change restarts the wait).
    pub auto_save_at: Option<(f64, u64)>,
    /// Files being encoded for auto-save on threads of their own (tab `uid`).
    pub auto_saving: Vec<(u64, AutoSaveEncoding)>,
    /// The Assets panel shows the libraries view (publish this file, enabled libraries).
    pub libraries: bool,
    /// Keyboard navigation: the region with the keyboard, keyboard mode (F6, Tab).
    pub nav: a11y::Nav,
    /// The Manage plugins window is open.
    pub plugins_window: bool,
    /// The Manage plugins window's tab, directory fields and pending install.
    pub plugins_manage: plugins_ui::PluginsWindow,
    /// A plugin command's parameters form, when open.
    pub plugin_form: Option<plugins_ui::PluginForm>,
    /// Live sessions: remote cursors as drawn, the Join dialog.
    pub live: live::LiveUi,
    /// Home ▸ Online.
    pub online: online::OnlineUi,
    /// The window was asked to close with unsaved changes: the question shows.
    pub confirm_close: bool,
    /// The file a Save as started from the unsaved-changes question is saving: the window closes
    /// once it is saved, and stays open if the dialog is cancelled or the save fails.
    pub close_after_save_as: Option<u64>,
    /// Save was answered: the files save next frame, once a panel field the click took the
    /// keyboard from has committed what was typed (as [`UiState::after_field`]).
    pub save_on_close: bool,
}

impl Default for UiState {
    /// Seed the default UI state (panel widths, tabs, pages, Home, presenters, pickers and
    /// dialogs off, and the rest of the transient UI fields); `FigApp::new` overrides panel
    /// widths and the pages height from preferences.
    fn default() -> Self {
        Self {
            left_width: PANEL_DEFAULT,
            right_width: PANEL_DEFAULT,
            right_tab: RightTab::Design,
            last_right_tab: RightTab::Design,
            left_tab: LeftTab::File,
            pages_open: true,
            page_search: false,
            page_filter: String::new(),
            pages_height: None,
            pages_seen_page: None,
            expanded: BTreeSet::new(),
            rename: None,
            rename_page: None,
            rename_label: None,
            main_menu: false,
            palette: None,
            palette_tab: menus::PaletteTab::All,
            palette_hi: 0,
            palette_armed: false,
            context_menu_shown: false,
            actions_rect: None,
            variables: None,
            about: false,
            about_tab: dialogs::AboutTab::About,
            preferences: false,
            menu_input: None,
            rename_file: false,
            ui_scale: ui_scale::plan(1.0, None, None),
            shortcuts_panel: false,
            shortcuts_tab: figcraft_engine::keymap::Group::Essential,
            shortcuts_query: String::new(),
            opacity_digit: None,
            home: false,
            home_view: home::Home::default(),
            title_bar: TitleBar::Native,
            icon_gallery: false,
            toast: None,
            present: None,
            presents: Vec::new(),
            present_active: None,
            present_next: 1,
            present_view: present::PresentView::default(),
            preview: None,
            picker: None,
            context_menu: None,
            key_menu: false,
            key_box: None,
            layer_search: false,
            layer_filter: String::new(),
            layer_anchor: None,
            layers_focus: false,
            pages_focus: None,
            layers_seen_sel: Vec::new(),
            dark: false,
            follow_system: false,
            tool_flyout: None,
            board_panel: false,
            last_error: None,
            pending_opens: Vec::new(),
            pending_open_frames: 0,
            import_report: None,
            font_checks: Vec::new(),
            missing_fonts: None,
            screenshot_requests: Vec::new(),
            frame_ms: 0.0,
            render_ms: 0.0,
            quit: false,
            export_preview: false,
            pdf_print: figcraft_doc::PrintSettings::default(),
            color_profile: None,
            style_picker: None,
            style_grid: false,
            style_folds: BTreeSet::new(),
            text_preview: None,
            text_preview_wanted: false,
            export_thumb: false,
            zoom_menu: false,
            find: None,
            batch_rename: None,
            left_collapsed: false,
            mobile_panel: None,
            narrow: false,
            comments: comments::CommentUi::default(),
            versions: versions::VersionUi::default(),
            branch_review: None,
            dev: dev_mode::DevUi::default(),

            last_copy: None,
            last_copy_generation: 0,
            v_pressed: false,
            paste_event: false,
            refocused: false,
            after_field: None,
            held: egui::Modifiers::NONE,
            next_autosave: 0.0,
            auto_save_at: None,
            auto_saving: Vec::new(),
            libraries: false,
            nav: a11y::Nav::default(),
            plugins_window: false,
            plugins_manage: plugins_ui::PluginsWindow::default(),
            plugin_form: None,
            live: live::LiveUi::default(),
            online: online::OnlineUi::default(),
            confirm_close: false,
            close_after_save_as: None,
            save_on_close: false,
        }
    }
}

/// A control reply waiting for the pass that presents an injected input batch. The viewport keeps
/// one window's batch from acknowledging (or erroring) another window's request.
pub(crate) struct PendingAck {
    pub(crate) viewport: egui::ViewportId,
    pub(crate) tx: std::sync::mpsc::Sender<ControlReply>,
    pub(crate) reply: ControlReply,
}

/// The whole FigCraft UI.
pub struct FigApp {
    pub session: Session,
    pub ui: UiState,
    pub canvas: canvas::CanvasState,
    pub services: Box<dyn Services>,
    worker: Option<render_worker::Worker>,
    fonts_installed: bool,
    /// System faces for scripts the built-in fonts lack were added (after the font scan), with
    /// faces for these CJK scripts ([`cjk`]) in this UI language's order.
    script_fonts: Option<(u8, &'static str)>,
    /// The tab (uid) and document version whose names were last checked for CJK scripts.
    cjk_checked: Option<(u64, Arc<figcraft_doc::Document>)>,
    /// The language preference the UI strings follow (applied when it changes).
    language: Option<String>,
    /// A field had the keyboard when the last frame ended.
    field_focus: bool,
    /// A panel's width changed and isn't kept in the preferences yet (its edge is being dragged).
    panels_unsaved: bool,
    /// Control-channel requests waiting for the next frame.
    pub control_queue: Vec<ControlRequest>,
    /// Input events from the control channel (`ui.input`) for the next frame's raw input.
    pub control_input: Vec<egui::Event>,
    /// Frames drawn and what asked for them (`ui.inspect`).
    pub frame_stats: frame_stats::FrameStats,
    /// The other windows (desktop app).
    pub windows: windows::Windows,
    /// `window.new` / `window.focus` asked for by commands, for the end of the frame.
    window_requests: Vec<figcraft_engine::UiRequest>,
    /// Commands waiting for a server ([`FigApp::run_background`]) and what to do with their results.
    waiting: Vec<(String, Done)>,
    /// Native dialogs complete asynchronously so their modal event loop never re-enters winit.
    pending_dialog: Option<PendingDialog>,
    /// The latest UI context schedules the first poll and wakes the app when a dialog completes.
    ui_context: Option<egui::Context>,
    /// The automation roots the app was started with: every frame and control request runs
    /// confined to them ([`windows::guard`]).
    pub automation_roots: Option<Arc<figcraft_engine::file_access::Roots>>,
    /// The active-document id when synthetic input was queued: a batch is dropped when the file
    /// changed before a UI pass applied it.
    pub(crate) control_input_doc: Option<u64>,
    /// This frame applies events injected through the control channel: no system clipboard
    /// read or write may happen for them.
    pub(crate) control_synthetic: bool,
    /// `ui.input` replies waiting for the pass that actually presents the injected batch (a
    /// discarded sizing pass must not acknowledge it).
    pub(crate) input_acks: Vec<PendingAck>,
    /// Replies whose batch this frame took into its raw input: sent only if the pass is presented.
    pub(crate) applied_acks: Vec<PendingAck>,
    /// The batch taken into this frame's raw input, kept so a discarded pass gives it back.
    pub(crate) applied_input: Option<Vec<egui::Event>>,
    /// `handle("ui.input")` queued raw events whose acknowledgement waits for the applied pass
    /// (controller-only input is applied at once and needs no fence).
    pub(crate) control_input_queued: bool,
    /// The viewport whose batch this frame took into its raw input (`applied_input`/`applied_acks`).
    pub(crate) applied_frame: Option<egui::ViewportId>,
    /// Monotonic time for background work and capture deadlines.
    pub(crate) clock: FrameClock,
    /// Screenshot requests admitted, settling and awaiting readback.
    pub(crate) capture: control::CaptureState,
    /// [`Self::background_tick`] ran for this pass (so [`Self::ui`] does not repeat it).
    logic_ticked: bool,
    /// The last frame whose UI setup ran (multi-pass frames set up once).
    ui_frame_stamp: Option<(egui::ViewportId, u64)>,
    /// IME owner/generation/change-cause state for the on-canvas text editor, per viewport.
    pub(crate) ime: egui::ime_context::ImeContextTracker,
    /// The check for a newer release and its prompt (desktop; [`updates_ui`]).
    pub(crate) updates_rt: updates_ui::Runtime,
    /// Test-only count of the one IME observation each viewport pass makes.
    #[cfg(test)]
    pub(crate) ime_observations: u32,
}

/// What to do with the result of a command run in the background.
pub type Done = Box<dyn FnOnce(&mut FigApp, &egui::Context, Value)>;

impl FigApp {
    /// Construct `FigApp` from a session and services, taking panel widths and pages height
    /// from preferences and initializing UI, canvas, clocks, capture/IME and transient channel
    /// state.
    pub fn new(session: Session, services: Box<dyn Services>) -> Self {
        // The main window's panels open as wide as they were left.
        let (left_width, right_width) = session.prefs.panel_widths;
        let pages_height = session.prefs.pages_height;
        Self {
            ui: UiState { left_width, right_width, pages_height, ..UiState::default() },
            session,
            canvas: canvas::CanvasState::default(),
            services,
            worker: None,
            fonts_installed: false,
            script_fonts: None,
            cjk_checked: None,
            language: None,
            field_focus: false,
            panels_unsaved: false,
            control_queue: Vec::new(),
            control_input: Vec::new(),
            frame_stats: frame_stats::FrameStats::default(),
            windows: windows::Windows::default(),
            window_requests: Vec::new(),
            waiting: Vec::new(),
            pending_dialog: None,
            ui_context: None,
            automation_roots: None,
            control_input_doc: None,
            control_synthetic: false,
            input_acks: Vec::new(),
            applied_acks: Vec::new(),
            applied_input: None,
            control_input_queued: false,
            applied_frame: None,
            clock: FrameClock::default(),
            capture: control::CaptureState::default(),
            logic_ticked: false,
            ui_frame_stamp: None,
            ime: egui::ime_context::ImeContextTracker::new(),
            updates_rt: updates_ui::Runtime::default(),
            #[cfg(test)]
            ime_observations: 0,
        }
    }

    /// Dismiss the composer after a successful switch away from the Comment tool.
    pub(crate) fn clear_comment_draft_after_tool_set(&mut self, id: &str, succeeded: bool) {
        if id == "tool.set" && succeeded && self.session.tool.tool != figcraft_engine::tools::ToolId::Comment {
            self.ui.comments.draft = None;
        }
    }

    /// Run a command, surfacing errors as a toast. Returns the result.
    pub fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        let layers_generation = self.session.clipboard.layers_generation;
        let r = self.session.execute(id, &params);
        self.clear_comment_draft_after_tool_set(id, r.is_ok());
        self.handle_ui_requests();
        self.invalidate_synthetic_copy(layers_generation);
        // A command may change the file the window shows (`doc.switch`, `doc.nextTab`,
        // `file.open`/`file.close`). Record that presentation before returning, so an in-flight
        // screenshot admitted on the old file cannot accept pixels from the new one.
        self.observe_presentation();
        r.map_err(|e| self.failed(id, &e)).ok()
    }

    /// A programmatic command that changed the engine clipboard never wrote the system clipboard:
    /// forget the old system mirror so a later user paste does not trust it as our own copy.
    /// `before` is the layers generation captured before the command ran.
    pub(crate) fn invalidate_synthetic_copy(&mut self, before: u64) {
        if self.control_synthetic && self.session.clipboard.layers_generation != before {
            self.ui.last_copy = None;
        }
    }

    /// Record the file the window drawing now shows, with a monotonic generation that changes on
    /// every switch (an A→B→A switch is two generations, never the admitted one again).
    pub(crate) fn observe_presentation(&mut self) {
        let window = self.windows.current;
        self.windows.presentation(&self.session, window);
    }

    /// [`run`](Self::run) for something the user did (a dialog, a drop, a paste, a recent file):
    /// a file it imports shows its import report. Programmatic calls use `run` and never open one.
    pub fn run_user(&mut self, id: &str, params: Value) -> Option<Value> {
        let result = self.run(id, params.clone());
        if let Some(result) = &result {
            import_report::capture(self, id, &params, result);
        }
        missing_fonts::check_later(self, id, result.as_ref());
        result
    }

    /// Show command `id`'s error.
    fn failed(&mut self, id: &str, e: &figcraft_engine::EngineError) {
        let msg = e.to_string();
        log::warn!("{id}: {msg}");
        self.show_error(msg);
    }

    /// Show an error as a toast, kept as the last error.
    pub(crate) fn show_error(&mut self, msg: String) {
        self.toast(msg.clone());
        self.ui.last_error = Some(msg);
    }

    /// Run a command that may talk to a server without waiting for it (the engine's
    /// [`start_background`](Session::start_background)): `done` gets the result, at once or when
    /// the answer arrives (the network wakes the window then); errors show as with [`run`](Self::run).
    /// [`Session::waiting_for`] tells the panels to show that it's on its way.
    pub fn run_background(&mut self, ctx: &egui::Context, id: &str, params: Value, done: Done) {
        self.waiting.retain(|(c, _)| c != id);
        let r = self.session.start_background(id, params);
        self.handle_ui_requests();
        match r {
            Ok(Some(v)) => done(self, ctx, v),
            Ok(None) => self.waiting.push((id.to_string(), done)),
            Err(e) => self.failed(id, &e),
        }
    }

    /// Stop waiting for command `id` (Cancel, or its window closed): its answer is dropped.
    pub fn cancel_background(&mut self, id: &str) {
        self.session.cancel_background(id);
        self.waiting.retain(|(c, _)| c != id);
    }

    /// Hand the results of background commands that finished to what's waiting for them.
    fn background_done(&mut self, ctx: &egui::Context) {
        while let Some((id, r)) = self.session.take_background_done() {
            self.handle_ui_requests();
            let done = self.waiting.iter().position(|(c, _)| *c == id).map(|i| self.waiting.remove(i).1);
            match (r, done) {
                (Ok(v), Some(done)) => done(self, ctx, v),
                (Ok(_), None) => {}
                (Err(e), _) => self.failed(&id, &e),
            }
        }
    }

    /// Show what `command params` would do on the canvas as a live preview (`text.preview`) while
    /// a font family, font style, text style, named instance or axis slider is hovered or dragged. Call it every frame
    /// the preview should stay: it ends on the first frame nothing asks for it.
    pub fn preview_text(&mut self, ctx: &egui::Context, command: &str, params: Value) {
        self.ui.text_preview_wanted = true;
        let revision = self.session.ds().map_or(0, |d| d.revision);
        if let Some((c, p, rev)) = &self.ui.text_preview
            && c == command
            && *p == params
            && (*rev == revision || figcraft_engine::textpreview::active(&self.session))
        {
            return;
        }
        let shown = self.session.execute("text.preview", &json!({"command": command, "params": params})).is_ok();
        let revision = self.session.ds().map_or(0, |d| d.revision);
        self.ui.text_preview = shown.then(|| (command.to_string(), params, revision));
        // The canvas drew before the panels: show the preview next frame.
        ctx.request_repaint();
    }

    /// End the live text preview the panels show (Escape, or nothing hovered any more).
    pub fn end_text_preview(&mut self, ctx: &egui::Context) {
        if self.ui.text_preview.take().is_some() {
            let _ = self.session.execute("text.endPreview", &json!({}));
            ctx.request_repaint();
        }
    }

    /// Dev Mode (the Inspect tab): the canvas measures on hover without Alt and edits nothing;
    /// entering it ends editing text or vectors.
    fn sync_dev_mode(&mut self) {
        let dev = self.ui.right_tab == RightTab::Inspect;
        if dev && !self.session.tool.dev_mode {
            let _ = self.session.drop_gesture();
            let editing = self.session.ds().map(|d| (d.text_edit.is_some(), d.vector_edit.is_some())).unwrap_or_default();
            for (on, cmd) in [(editing.0, "text.edit.end"), (editing.1, "vector.edit.end")] {
                if on {
                    self.run(cmd, json!({}));
                }
            }
        }
        self.session.tool.dev_mode = dev;
    }

    /// Leaving Design ends a pen path as Escape does and goes back to Move: Prototype and Inspect
    /// don't draw (#307). Checked as the frame starts (the control channel) and after the panels
    /// and shortcuts (the mode switcher, the tabs, Shift+E/Shift+D), before the canvas sees more.
    fn end_pen_outside_design(&mut self) {
        if self.ui.right_tab == self.ui.last_right_tab {
            return;
        }
        self.ui.last_right_tab = self.ui.right_tab;
        if self.ui.right_tab != RightTab::Design && self.session.tool.tool == figcraft_engine::tools::ToolId::Pen {
            self.run("tool.set", json!({"tool": "move"}));
        }
    }

    /// Once a frame, after the panels: a preview nothing asked for this frame ends.
    fn end_unwanted_text_preview(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.ui.text_preview_wanted) {
            self.end_text_preview(ctx);
        }
    }

    /// Finish the frame that applies an injected batch: acknowledge it once the pass is presented,
    /// and stop fencing the system clipboard. Only the viewport whose batch this frame took acts.
    pub(crate) fn end_input_frame(&mut self, ctx: &egui::Context) {
        if self.applied_frame != Some(ctx.viewport_id()) {
            return;
        }
        self.applied_input = None;
        if ctx.will_discard() {
            // Keep `applied_frame`/`applied_acks`: the rerun pass must present (and acknowledge)
            // the same frame without replaying the input.
            return;
        }
        self.applied_frame = None;
        for ack in self.applied_acks.drain(..) {
            let _ = ack.tx.send(ack.reply);
        }
        self.control_synthetic = false;
    }

    /// The app's monotonic clock in seconds, also usable from logic-only callbacks (recovery,
    /// control deadlines). Never `ctx.input(i.time)`, which is stale there.
    pub fn now_secs(&self) -> f64 {
        self.clock.now()
    }

    /// Queue raw input injected through the control channel for the window drawing next, tagged
    /// with the document it belongs to (a batch is dropped if another file is active by then).
    pub(crate) fn push_control_input(&mut self, events: Vec<egui::Event>) -> bool {
        if events.is_empty() {
            return false;
        }
        let doc = self.session.docs.get(self.session.active).map(|d| d.uid);
        if !self.control_input.is_empty() && self.control_input_doc != doc {
            // A queued batch belongs to the file that was open when it was injected; it cannot be
            // merged with one for the file showing now. Its acknowledgement says so.
            self.control_input.clear();
            let viewport = crate::windows::viewport(self.windows.current);
            crate::windows::fail_input_acks(self, viewport);
        }
        // Always the live pane vector: while a child window draws, `windows::enter` has swapped
        // its pane into the app, and `leave` swaps these events back into that pane.
        self.control_input.extend(events);
        self.control_input_doc = doc;
        self.control_synthetic = true;
        true
    }

    /// The acknowledgements waiting for `viewport`'s input batch (other windows keep theirs).
    pub(crate) fn take_input_acks(&mut self, viewport: egui::ViewportId) -> Vec<PendingAck> {
        let mut mine = Vec::new();
        for ack in std::mem::take(&mut self.input_acks) {
            if ack.viewport == viewport {
                mine.push(ack);
            } else {
                self.input_acks.push(ack);
            }
        }
        mine
    }

    /// Put `text` on the system clipboard. Input injected through the control channel never
    /// replaces what the user copied: this is the one write path every copy command goes through.
    pub(crate) fn clipboard_set_text(&mut self, text: &str) {
        if self.control_synthetic {
            return;
        }
        self.services.clipboard_set_text(text);
    }

    /// [`Self::clipboard_set_text`] for an image (straight RGBA).
    pub(crate) fn clipboard_set_png(&mut self, width: u32, height: u32, rgba: &[u8]) {
        if self.control_synthetic {
            return;
        }
        self.services.clipboard_set_png(width, height, rgba);
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.ui.toast = Some((msg.into(), ToastClock::Raised));
    }

    /// Say that `n` documents left unsaved by a crash were opened again from recovery copies.
    pub fn note_recovered(&mut self, n: usize) {
        if n > 0 {
            self.toast(i18n::trn("Recovered {n} unsaved files after the app quit unexpectedly: save them to keep them", n, &[]));
        }
    }

    /// Act on UI requests queued by commands (dialogs, panels).
    pub fn handle_ui_requests(&mut self) {
        let reqs = std::mem::take(&mut self.session.ui_requests);
        for r in reqs {
            match r.kind.as_str() {
                "dialog.about" => {
                    self.ui.about = true;
                    self.ui.about_tab = dialogs::AboutTab::About;
                }
                "dialog.preferences" => self.ui.preferences = true,
                "dialog.missingFonts" => missing_fonts::request(self),
                "panel.shortcuts" => self.ui.shortcuts_panel = !self.ui.shortcuts_panel,
                "palette.actions" => menus::toggle_palette(&mut self.ui),
                "panel.design" if self.session.doc().is_ok_and(figcraft_engine::board::is_board) => self.ui.board_panel = !self.ui.board_panel,
                "panel.design" => self.ui.right_tab = RightTab::Design,
                "board.editText" => {
                    if let Some(id) = r.data.get("id").and_then(figcraft_engine::params::id_of)
                        && let Err(e) = figcraft_engine::board::edit_text(&mut self.session, id)
                    {
                        self.toast(e.to_string());
                    }
                }
                "panel.prototype" => self.ui.right_tab = RightTab::Prototype,
                "panel.designPrototype" => {
                    self.ui.right_tab = if self.ui.right_tab == RightTab::Prototype { RightTab::Design } else { RightTab::Prototype };
                }
                "panel.devMode" => {
                    self.ui.right_tab = if self.ui.right_tab == RightTab::Inspect { RightTab::Design } else { RightTab::Inspect };
                }
                "panel.leftSidebar" => {
                    let shown = if self.ui.narrow { self.ui.mobile_panel == Some(MobilePanel::Left) } else { !self.ui.left_collapsed };
                    if shown {
                        self.ui.close_left_panel();
                    } else {
                        self.ui.open_left_panel();
                    }
                }
                "panel.widths" => {
                    let width = |k: &str| r.data.get(k).and_then(Value::as_f64).map(|w| w as f32);
                    let (left, right) = (width("left"), width("right"));
                    let reset = left.is_none() && right.is_none();
                    self.set_panel_widths(
                        left.unwrap_or(if reset { PANEL_DEFAULT } else { self.ui.left_width }),
                        right.unwrap_or(if reset { PANEL_DEFAULT } else { self.ui.right_width }),
                    );
                    self.services.save_prefs(&self.session.prefs);
                }
                "focus.region" => self.ui.nav.step(r.data.get("step").and_then(Value::as_i64).unwrap_or(1).clamp(-4, 4) as i32),
                "panel.layers" => self.ui.left_tab = LeftTab::File,
                "panel.assets" => {
                    self.ui.left_tab = LeftTab::Assets;
                    self.ui.open_left_panel();
                    self.ui.versions = Default::default();
                }
                "panel.versionHistory" => versions::open(self),
                "view.devCompare" => {
                    self.ui.dev.compare = dev_compare::CompareView::requested(&r.data);
                }
                "panel.libraries" => {
                    self.ui.left_tab = LeftTab::Assets;
                    self.ui.open_left_panel();
                    self.ui.libraries = true;
                }
                "home" => self.ui.home = true,
                "home.templates" => {
                    self.ui.home = true;
                    self.ui.home_view.section = home::Section::Templates;
                }
                "home.gettingStarted" => {
                    self.ui.home = true;
                    self.ui.home_view.section = home::Section::Recents;
                    self.services.save_prefs(&self.session.prefs);
                }
                "dialog.saveAs" => self.save_as_dialog(),
                "dialog.colorProfile" => self.ui.color_profile = self.session.doc().ok().map(|d| d.color_profile),
                "present" => {
                    if let Some(start) = r.data.get("start").and_then(figcraft_engine::params::id_of) {
                        tabs::open_present(self, start);
                    }
                }
                "present.options" => {
                    let v = &mut self.ui.present_view;
                    if let Some(s) = r.data.get("scale").and_then(|s| s.as_str()).and_then(present::PresentScale::parse) {
                        v.scale = s;
                    }
                    if let Some(f) = r.data.get("deviceFrame").and_then(|f| f.as_bool()) {
                        v.device_frame = f;
                    }
                    if let Some(h) = r.data.get("hotspotHints").and_then(|h| h.as_bool()) {
                        v.hotspot_hints = h;
                    }
                    if let Some(i) = r.data.get("inspect").and_then(|i| i.as_bool()) {
                        v.inspect = i;
                    }
                    if let Some(p) = self.ui.present.as_mut() {
                        p.view.device_frame = v.device_frame;
                        p.view.hotspot_hints = v.hotspot_hints;
                        p.view.inspect = v.inspect;
                        p.set_scale(v.scale);
                    }
                }
                "preview" => {
                    if let Some(start) = r.data.get("start").and_then(figcraft_engine::params::id_of) {
                        self.ui.preview = Some(present::Pane::new(start, &self.session));
                    }
                }
                "preview.close" => self.ui.preview = None,
                "open.help" => self.services.open_url("https://getartcraft.com/apps/figcraft"),
                "layers.collapseAll" => layers::collapse_all(self),
                "contextMenu" => self.ui.key_menu = true,
                "selectArea" => self.start_key_box(),
                "canvas.renameLabel" => {
                    self.ui.rename_label = r.data.get("id").and_then(figcraft_engine::params::id_of).map(|id| layers::Rename { id, start: true });
                }
                "layers.rename" => {
                    let ids: Vec<NodeId> =
                        r.data.get("ids").and_then(Value::as_array).into_iter().flatten().filter_map(figcraft_engine::params::id_of).collect();
                    if ids.len() > 1 {
                        dialogs::open_batch_rename(self, &ids);
                    } else {
                        layers::start_rename(self, &ids);
                    }
                }
                "comment.open" => {
                    if let Some(id) = r.data.get("id").and_then(Value::as_u64) {
                        self.ui.comments.show(id);
                    }
                }
                "comment.new" => {
                    let x = r.data.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let y = r.data.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    comments::start_draft(self, x, y);
                }
                "dialog.unsaved" => self.toast(i18n::tr("This file has unsaved changes")),
                "dialog.plugins" => self.ui.plugins_window = true,
                "dialog.pluginSave" => plugins_ui::save_file(self, &r.data),
                "dialog.cursorChat" => live::open_chat(self),
                "window.new" | "window.focus" | "window.fullscreen" => self.window_requests.push(r),
                _ => {}
            }
        }
        if let Some(m) = self.session.message.take() {
            self.toast(m);
        }
    }

    /// Set the widths asked for this window's side panels (within their limits); the main
    /// window's are kept in the preferences.
    pub fn set_panel_widths(&mut self, left: f32, right: f32) {
        let within = |w: f32, range: std::ops::RangeInclusive<f32>| if w.is_finite() { w.clamp(*range.start(), *range.end()) } else { PANEL_DEFAULT };
        (self.ui.left_width, self.ui.right_width) = (within(left, LEFT_PANEL), within(right, RIGHT_PANEL));
        if self.windows.current == 0 {
            self.session.prefs.panel_widths = (self.ui.left_width, self.ui.right_width);
        }
    }

    /// The edge between a side panel and the canvas, at `x` down `rows`: dragging it sizes the
    /// panel (`grows`: +1 for the left panel, whose edge is on its right, −1 for the right one), a
    /// double-click puts back the default width. Returns the width asked for, when it changed.
    fn panel_edge(ui: &mut egui::Ui, name: &str, x: f32, rows: egui::Rangef, width: f32, grows: f32) -> Option<f32> {
        let grab = ui.style().interaction.resize_grab_radius_side;
        let rect = egui::Rect::from_x_y_ranges(x - grab..=x + grab, rows);
        let id = egui::Id::unique(("panel_edge", name));
        // Not focusable: an edge is a pointer target, so it must not take Tab/F6 focus from the
        // panels' controls (see `a11y::focusables`).
        let resp = ui.interact(rect, id, egui::Sense::CLICK | egui::Sense::DRAG);
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().vline(x, rows, egui::Stroke::new(2.0, theme::ACCENT));
        }
        if resp.double_clicked() {
            return Some(PANEL_DEFAULT);
        }
        // The edge stays under the pointer where it was grabbed (also back from past a limit).
        let at = resp.interact_pointer_pos().filter(|_| resp.dragged())?;
        if resp.drag_started() {
            let pressed = ui.input(|i| i.pointer.press_origin()).unwrap_or(at);
            ui.data_mut(|d| d.insert_temp(id, pressed.x - x));
        }
        let grabbed: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
        let w = width + (at.x - grabbed - x) * grows;
        (w != width).then_some(w)
    }

    pub fn save_as_dialog(&mut self) {
        if self.services.downloads() {
            if let Some(r) = self.run("file.saveAs", json!({"data": true}))
                && self.download(r["name"].as_str().unwrap_or_default(), &r)
            {
                self.run("file.markSaved", json!({}));
                dialogs::file_saved(self, &r);
            }
            return;
        }
        let Ok(name) = self.session.doc().map(|d| d.name.clone()) else { return };
        self.pick_save(i18n::tr("Save as"), &format!("{name}.figcraft"), &["figcraft", "fig"], |app, mut p| {
            if p.extension().is_none_or(OsStr::is_empty) {
                if !p.set_extension("figcraft") {
                    app.toast(i18n::tr("Use the .figcraft or .fig extension."));
                    return;
                }
                // This name wasn't confirmed by the system dialog: don't replace an existing file.
                match p.try_exists() {
                    Ok(false) => {}
                    Ok(true) => {
                        app.toast(i18n::tr("A .figcraft file with this name already exists. Choose its full name to confirm replacement."));
                        return;
                    }
                    Err(e) => {
                        app.toast(e.to_string());
                        return;
                    }
                }
            } else if !p.extension().and_then(OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("figcraft") || e.eq_ignore_ascii_case("fig")) {
                app.toast(i18n::tr("Use the .figcraft or .fig extension."));
                return;
            }
            if let Some(r) = app.run("file.saveAs", json!({"path": p.to_string_lossy()})) {
                dialogs::file_saved(app, &r);
            }
        });
    }

    pub fn save(&mut self) {
        // A branch tab saves its main file.
        let tab = figcraft_engine::branches::file_tab(&self.session);
        // A path ending in .fig is never saved over silently: a .fig destination is an export copy.
        let native_path = self
            .session
            .docs
            .get(tab)
            .and_then(|d| d.path.as_deref())
            .is_some_and(|path| !Path::new(path).extension().and_then(OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("fig")));
        if native_path {
            if self.run("file.save", json!({})).is_some() {
                self.toast(i18n::tr("Saved"));
            }
        } else {
            self.save_as_dialog();
        }
    }

    pub fn open_dialog(&mut self) {
        // Opening needs no file in front: it goes ahead even if that file closed meanwhile.
        self.pick_file(i18n::tr("Open"), OPEN_EXTENSIONS, false, |app, file| {
            if app.run_user("file.open", file).is_some() {
                app.ui.home = false;
            }
        });
    }

    pub fn import_dialog(&mut self, place: bool) {
        let exts: &[&str] = if place {
            &["png", "jpg", "jpeg", "webp", "gif", "tif", "tiff"]
        } else {
            &["svg", "png", "jpg", "jpeg", "webp", "gif", "tif", "tiff", "figcraft", "fig", "sketch"]
        };
        let title = if place { i18n::tr("Place image") } else { i18n::tr("Import") };
        self.pick_file(title, exts, true, |app, file| {
            app.run_user("file.import", file);
        });
    }

    /// Pick a file to open or import; `done` gets it as `file.open` and `file.import` take it (a
    /// path, or the name and contents where the platform has no paths). `pin`: as
    /// [`FigApp::pick_open`]'s file in front.
    fn pick_file(&mut self, title: &str, exts: &[&str], pin: bool, done: impl FnOnce(&mut FigApp, Value) + 'static) {
        self.start_pick(
            |services| -> PickFuture {
                match services.pick_open_contents(title, exts) {
                    Some(read) => {
                        Box::pin(async move { read.await.map(|r| r.map_or_else(Picked::Unreadable, |(name, bytes)| Picked::Contents(name, bytes))) })
                    }
                    None => {
                        let pick = services.pick_open(title, exts);
                        Box::pin(async move { pick.await.map(Picked::Path) })
                    }
                }
            },
            pin,
            move |app, picked| match picked.file_params() {
                Ok(file) => done(app, file),
                Err(why) => app.show_error(why),
            },
        );
    }

    /// Pick a file to open. `done` runs with the file in front now back in front (and not at all
    /// if that file closed meanwhile).
    pub fn pick_open(&mut self, title: &str, exts: &[&str], done: impl FnOnce(&mut FigApp, PathBuf) + 'static) {
        self.start_dialog(|services| services.pick_open(title, exts), true, done);
    }

    /// Pick where to save; `done` runs as [`FigApp::pick_open`]'s does.
    pub fn pick_save(&mut self, title: &str, name: &str, exts: &[&str], done: impl FnOnce(&mut FigApp, PathBuf) + 'static) {
        self.start_dialog(|services| services.pick_save(title, name, exts), true, done);
    }

    /// Hand a command result's base64 `data` over as a download named `name`, where the platform
    /// saves by downloads rather than save dialogs (the browser, #969). Says so when nothing arrived.
    pub(crate) fn download(&mut self, name: &str, result: &Value) -> bool {
        use base64::Engine as _;
        let bytes = result["data"].as_str().and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok());
        if bytes.is_some_and(|b| self.services.download(name, &b)) {
            return true;
        }
        self.toast(i18n::trf("Couldn't download {name}", &[("name", &name)]));
        false
    }

    /// Pick a folder; `done` runs as [`FigApp::pick_open`]'s does.
    pub fn pick_folder(&mut self, title: &str, done: impl FnOnce(&mut FigApp, PathBuf) + 'static) {
        self.start_dialog(|services| services.pick_folder(title), true, done);
    }

    /// Open a native dialog from the window drawing now, for the file in front (`pin`).
    fn start_dialog(&mut self, start: impl FnOnce(&mut dyn Services) -> DialogFuture, pin: bool, done: impl FnOnce(&mut FigApp, PathBuf) + 'static) {
        self.start_pick(
            |services| -> PickFuture {
                let pick = start(services);
                Box::pin(async move { pick.await.map(Picked::Path) })
            },
            pin,
            move |app, picked| {
                if let Picked::Path(p) = picked {
                    done(app, p);
                }
            },
        );
    }

    /// [`FigApp::start_dialog`] for any kind of pick.
    fn start_pick(&mut self, start: impl FnOnce(&mut dyn Services) -> PickFuture, pin: bool, done: impl FnOnce(&mut FigApp, Picked) + 'static) {
        if self.pending_dialog.is_some() {
            return;
        }
        let doc = if pin { self.session.ds().ok().filter(|d| self.shows(d.uid)).map(|d| d.uid) } else { None };
        let future = start(self.services.as_mut());
        self.pending_dialog = Some(PendingDialog { future, picked: None, done: Box::new(done), doc, window: self.windows.current });
        if let Some(ctx) = &self.ui_context {
            ctx.request_repaint();
        }
    }

    /// Once the dialog has a path, its continuation runs in the window that holds its file, with
    /// that file in front; then the tab in front before comes back (unless the dialog opened a file).
    fn poll_dialog(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.pending_dialog else { return };
        if d.picked.is_none() {
            let waker = Waker::from(Arc::new(RepaintWaker(ctx.clone())));
            let Poll::Ready(picked) = d.future.as_mut().poll(&mut TaskContext::from_waker(&waker)) else { return };
            d.picked = picked;
        }
        let (doc, origin, picked) = (d.doc, d.window, d.picked.is_some());
        let index = doc.map(|u| self.session.docs.iter().position(|ds| ds.uid == u));
        // Cancelled, or its file closed meanwhile.
        if !picked || index == Some(None) {
            self.pending_dialog = None;
            return;
        }
        let window = match doc {
            Some(u) => self.windows.of(u),
            None if self.windows.exists(origin) => origin,
            None => 0,
        };
        if window != self.windows.current {
            ctx.request_repaint_of(windows::viewport(window));
            return;
        }
        let Some(PendingDialog { picked: Some(picked), done, .. }) = self.pending_dialog.take() else { return };
        let before = self.session.ds().ok().map(|d| d.uid);
        if let Some(Some(i)) = index {
            self.session.active = i;
        }
        done(self, picked);
        if doc.is_some()
            && self.session.ds().ok().map(|d| d.uid) == doc
            && let Some(i) = before.and_then(|u| self.session.docs.iter().position(|d| d.uid == u))
        {
            self.session.active = i;
        }
    }

    /// Reduced motion: the preference or the system's setting.
    pub fn reduce_motion(&self) -> bool {
        self.session.prefs.reduce_motion || self.services.reduce_motion()
    }

    /// The CJK, Bengali and Thai scripts the UI shows: the language's, the language picker's (Preferences open),
    /// typed text's, and those in the open file's names and comments (read when it changes, not
    /// mid-drag). Nothing is checked once every script has a face.
    fn cjk_needed(&mut self, ctx: &egui::Context, installed: u8) -> u8 {
        if installed == cjk::ALL {
            return installed;
        }
        let mut need = i18n::current().map_or(0, |l| cjk::language(l.code));
        if self.ui.preferences {
            need |= i18n::languages().iter().filter(|l| cjk::can_show(l.code)).fold(0, |m, l| m | cjk::language(l.code));
        }
        need |= ctx.input(|i| {
            i.events.iter().fold(0, |m, e| match e {
                egui::Event::Text(t) | egui::Event::Ime(egui::ImeEvent::Preedit { text: t, .. } | egui::ImeEvent::Commit(t)) => m | cjk::scripts(t),
                _ => m,
            })
        });
        let dragging = ctx.input(|i| i.pointer.any_down());
        // Pans and zooms change the revision too: only a new version of the document is looked
        // at, and only the layers it changed (a large file has hundreds of thousands of names).
        let current = self.session.docs.get(self.session.active).filter(|_| !dragging).map(|d| (d.uid, d.doc.clone()));
        if let Some((uid, doc)) = current {
            let before = self.cjk_checked.take().filter(|(u, _)| *u == uid).map(|(_, b)| b);
            if !before.as_ref().is_some_and(|b| Arc::ptr_eq(b, &doc)) {
                let mut scan = |s: &str| need |= cjk::scripts(s);
                scan(&doc.name);
                match before.as_ref().and_then(|b| doc.changed_ids(b, CJK_CHANGES)) {
                    Some(ids) => ids.into_iter().filter_map(|id| doc.node(id)).for_each(|n| scan(&n.name)),
                    None => doc.nodes().for_each(|n| scan(&n.name)),
                }
                doc.comments.iter().flat_map(|c| std::iter::once(&c.first).chain(&c.replies)).for_each(|m| {
                    scan(&m.author);
                    scan(&m.text);
                });
            }
            self.cjk_checked = Some((uid, doc));
        }
        need
    }

    /// Work safe from a logic-only callback (`App::logic`, which also runs while hidden) and from
    /// a normal UI pass: consumed control work, live polling, background completions, autosave and
    /// recovery deadlines. It never reads UI input, so it must not scan events, run shortcuts or
    /// lay out anything.
    pub fn background_tick(&mut self, ctx: &egui::Context) {
        control::background(self, ctx);
        // What happened in a live session (the network woke us for it).
        if self.session.live_poll() {
            if let Some(m) = self.session.message.take() {
                self.toast(live::failure_text(&m));
            }
            self.background_done(ctx);
        }
        // Files that arrived without a command (joined, recovered) take the kept View toggles too.
        self.session.apply_view_prefs();
        let now = self.clock.now();
        self.autosave_tick(ctx, now);
        self.auto_save_tick(ctx, now);
        updates_ui::tick(self, ctx);
        self.logic_ticked = true;
    }

    /// UI-phase setup that needs a real current `Ui`: fonts/language/theme, interface zoom and the
    /// render worker. Runs once per frame (pass 0) for the main window, and once per frame for
    /// every child window (`windows::draw`), never on a logic-only tick.
    fn ui_setup(&mut self, ctx: &egui::Context) {
        self.viewport_setup(ctx);
    }

    /// The app-global part of [`Self::ui_setup`] that every window's frame needs: language, fonts,
    /// theme and `ui.dark`, the render worker, and the interface zoom plan. eframe only calls
    /// `App::ui` for the root window, so a child must run this itself before it draws; a child that
    /// only runs while the root is minimised still gets theme switches, CJK faces for its own
    /// document, and the right `ui.dark` for its tabs and dialogs.
    pub(crate) fn viewport_setup(&mut self, ctx: &egui::Context) {
        if self.language.as_ref() != Some(&self.session.prefs.language) {
            i18n::set(&self.session.prefs.language, self.services.system_language().as_deref());
            self.language = Some(self.session.prefs.language.clone());
        }
        // Screen-reader and bidi language of the UI (fork E17).
        ctx.set_language(i18n::current().map(|l| l.code.to_string()));
        if !self.fonts_installed {
            // Cmd/Ctrl+= / - / 0 zoom the canvas (view.zoomIn, view.zoomOut, view.zoom100): egui's
            // own keyboard zoom would scale the whole interface on the same keys.
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
            // A double-click's window is the platform's (0.5 s, Windows' default), not egui's
            // stricter 0.3 s: a double-click the OS takes must enter a shape's points too.
            ctx.memory_mut(|m| m.options.input_options.max_double_click_delay = 0.5);
            theme::install_fonts(ctx, false, 0);
            self.fonts_installed = true;
            // Installed fonts apply from the next frame; a child window draws on here, so ask for
            // that frame (the root draws nothing on the frame it installs them).
            ctx.request_repaint();
        } else if figcraft_text::system_fonts_scanned() {
            // Once the system fonts are indexed: faces for names in scripts the UI fonts lack, and
            // for each CJK script once something shows it.
            let installed = self.script_fonts.map_or(0, |(s, _)| s);
            let cjk = installed | self.cjk_needed(ctx, installed);
            // The UI language's script leads the CJK faces, so another language lays them out again.
            let lang = i18n::current().map_or("en", |l| l.code);
            if self.script_fonts != Some((cjk, lang)) {
                self.script_fonts = Some((cjk, lang));
                theme::install_fonts(ctx, true, cjk);
            }
        }
        // egui's zoom is for the whole app and multiplies each window's own system scale, so the
        // size is the same relative to every screen; the main window, whose size can hold a size
        // back, sets it. The browser keeps its own zoom. Every window records its own plan (the
        // Preferences warning and `ui.inspect` read it), from its own size.
        {
            let window_px = ctx.content_rect().size() * ctx.pixels_per_point();
            let size = if cfg!(target_arch = "wasm32") { 1.0 } else { self.session.prefs.ui_scale };
            let plan = ui_scale::plan(size, ctx.native_pixels_per_point(), Some((window_px.x, window_px.y)));
            if ctx.viewport_id() == egui::ViewportId::ROOT {
                ctx.set_zoom_factor(plan.zoom);
            }
            self.ui.ui_scale = plan;
        }
        let dark = match self.session.prefs.theme.as_str() {
            "dark" => true,
            // The active visuals were set by us last frame (possibly for a manual theme). Read the
            // integration's OS theme; winit reports none on Linux and the BSDs, where the platform
            // services' colour scheme answers instead. Without either, keep the light fallback.
            "system" => ctx.system_theme().map(|t| t == egui::Theme::Dark).or_else(|| self.services.system_dark()).unwrap_or(false),
            _ => false,
        };
        self.ui.dark = dark;
        // New files take the theme's page colour (existing pages keep theirs).
        self.session.set_ui_dark(dark);
        theme::apply_with(ctx, if dark { theme::Tokens::dark() } else { theme::Tokens::light() }, self.reduce_motion());
        if self.worker.is_none() {
            self.worker = Some(render_worker::Worker::new(ctx.clone()));
        }
    }

    /// Every [`AUTOSAVE_SECS`], each open document with unsaved changes since its last version
    /// gets an autosave checkpoint in its version history.
    fn autosave_tick(&mut self, ctx: &egui::Context, now: f64) {
        if now < self.ui.next_autosave {
            // Wake up for it even if nothing else draws a frame by then.
            repaint_in(ctx, self.ui.next_autosave - now);
            return;
        }
        if self.ui.next_autosave > 0.0 {
            let active = self.session.active;
            for i in 0..self.session.docs.len() {
                self.session.active = i;
                let _ = self.session.execute("version.autosave", &json!({}));
            }
            self.session.active = active;
        }
        self.ui.next_autosave = now + AUTOSAVE_SECS;
        repaint_in(ctx, AUTOSAVE_SECS);
    }

    /// With auto-save on, files that have a path are saved [`AUTO_SAVE_SECS`] after they last
    /// changed (not during a gesture, a live preview or while a version is shown). The file is
    /// encoded on a thread of its own, so a big one doesn't hold up the window, and written here.
    fn auto_save_tick(&mut self, ctx: &egui::Context, now: f64) {
        let mut failed = self.auto_save_done();
        // The browser has no files to save to.
        let on = !cfg!(target_arch = "wasm32") && self.session.prefs.auto_save;
        let saving = &self.ui.auto_saving;
        let due = |d: &&figcraft_engine::DocState| {
            d.path.as_deref().is_some_and(|p| d.images_lost_saving_to(p).is_none())
                && d.branch_of.is_none()
                && d.preview.is_none()
                && d.is_dirty()
                && !saving.iter().any(|(uid, _)| *uid == d.uid)
        };
        let changes = if on { self.session.docs.iter().filter(due).map(|d| d.revision).reduce(u64::wrapping_add) } else { None };
        match changes {
            None => self.ui.auto_save_at = None,
            Some(changes) => {
                let at = match self.ui.auto_save_at {
                    Some((at, c)) if c == changes && failed.is_none() => at,
                    _ => now + if failed.is_some() { AUTO_SAVE_RETRY_SECS } else { AUTO_SAVE_SECS },
                };
                self.ui.auto_save_at = Some((at, changes));
                if now < at || self.session.docs.iter().any(|d| d.interaction.is_some() || d.text_preview.is_some()) {
                    // Due but busy: look again in a second.
                    repaint_in(ctx, if now < at { at - now } else { 1.0 });
                } else {
                    self.ui.auto_save_at = None;
                    let files: Vec<_> =
                        self.session.docs.iter().filter(due).map(|d| (d.uid, figcraft_engine::cmd::file::FileSnapshot::of(d))).collect();
                    for (uid, file) in files {
                        let encode = move || {
                            let bytes = file.encode();
                            (file, bytes)
                        };
                        match std::thread::Builder::new().name("figcraft-save".into()).spawn(encode) {
                            Ok(h) => self.ui.auto_saving.push((uid, h)),
                            Err(e) => {
                                failed.get_or_insert(e.to_string());
                                self.ui.auto_save_at = Some((now + AUTO_SAVE_RETRY_SECS, changes));
                            }
                        }
                    }
                }
            }
        }
        if !self.ui.auto_saving.is_empty() {
            // Look again shortly for the files being encoded.
            repaint_in(ctx, 0.1);
        }
        if let Some(e) = failed {
            self.toast(i18n::trf("Couldn't save automatically: {error}", &[("error", &e)]));
        }
    }

    /// Write the files whose auto-save encoding finished; the first error, if one failed.
    fn auto_save_done(&mut self) -> Option<String> {
        if self.ui.auto_saving.is_empty() {
            return None;
        }
        let mut failed = None;
        let done: Vec<_> = self.ui.auto_saving.extract_if(.., |(_, h)| h.is_finished()).collect();
        for (_, h) in done {
            let saved = match h.join() {
                Ok((snapshot, Ok(bytes))) => snapshot.save(&mut self.session, &bytes).map(|_| ()).map_err(|e| e.to_string()),
                Ok((_, Err(e))) => Err(e.to_string()),
                Err(_) => Err("the file couldn't be encoded".to_string()),
            };
            if let Err(e) = saved {
                failed.get_or_insert(e);
            }
        }
        failed
    }

    /// Draw the whole UI: the main window (the other windows draw themselves).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        let t0 = std::time::Instant::now();
        let ctx = ui.ctx().clone();
        self.ui_context = Some(ctx.clone());
        // `App::logic` already ticked this pass; direct harnesses do not call it. The marker only
        // covers the tick that shares this pass: clearing it here keeps a direct `App::ui` harness
        // ticking on every frame (it used to tick every other frame).
        if !std::mem::take(&mut self.logic_ticked) {
            self.background_tick(&ctx);
            self.logic_ticked = false;
        }
        // Setup once per frame (pass 0), not once per pass and never on a logic-only tick.
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        let first = !self.fonts_installed;
        if ctx.current_pass_index() == 0 && self.ui_frame_stamp != Some(stamp) {
            self.ui_frame_stamp = Some(stamp);
            self.frame_stats.record(&ctx);
            self.ui_setup(&ctx);
        }
        if first && self.fonts_installed {
            // New fonts apply from the next frame; draw nothing until then. Anything taken into
            // this frame's (undrawn) input waits for the first drawn frame.
            ctx.request_repaint();
            if let Some(events) = self.applied_input.take() {
                self.push_control_input(events);
            }
            self.input_acks.append(&mut self.applied_acks);
            self.applied_frame = None;
            self.control_synthetic = false;
            // The next (drawn) frame ticks for itself in a direct harness.
            self.logic_ticked = false;
            self.observe_ime(&ctx, false);
            return;
        }
        windows::begin(self, &ctx);
        let (files, prefs) = (self.session.docs.len(), self.session.prefs.clone());
        self.frame(ui);
        windows::settle(self, &ctx, files);
        if self.session.prefs != prefs {
            windows::repaint_others(self, &ctx);
        }
        windows::show(self, &ctx);
        // The injected batch was applied by the frame that took it. A discarded sizing pass keeps
        // its acknowledgement for the presented pass but never replays the input (its side effects
        // already happened and would otherwise duplicate).
        self.end_input_frame(&ctx);
        // Plugins installed, removed or set up are kept at once: a crash doesn't lose them.
        // So are the owner tokens of files put online.
        if std::mem::take(&mut self.session.plugins.unsaved) | std::mem::take(&mut self.session.live.unsaved) {
            self.services.save_prefs(&self.session.prefs);
        }
        // So is the project library, each time it changes.
        self.save_projects();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.ui.frame_ms = t0.elapsed().as_secs_f64() * 1000.0;
        }
    }

    /// Whether the right panel shows: never with the UI `hidden`; always on a design file; on a
    /// board, which has no prototypes or Dev Mode (its tab stays Design), only when asked for.
    fn right_panel_shown(&mut self, hidden: bool) -> bool {
        let board = self.session.doc().is_ok_and(figcraft_engine::board::is_board);
        if board {
            self.ui.right_tab = RightTab::Design;
        }
        !hidden && (!board || self.ui.board_panel)
    }

    /// A narrow window's two buttons for the side panels (touch-sized); the open one closes again.
    fn mobile_buttons(&mut self, ctx: &egui::Context, at: egui::Pos2) {
        use a11y::Tip as _;
        let t = theme::Tokens::get(ctx);
        egui::Area::new(egui::Id::unique("mobile_panel_buttons")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
            for (side, icon, name) in
                [(MobilePanel::Left, icons::Icon::Layers, i18n::tr("Layers")), (MobilePanel::Right, icons::Icon::Panels, i18n::tr("Design"))]
            {
                let open = self.ui.mobile_panel == Some(side);
                let (r, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                let resp = resp.tip(name);
                let fill = if open { t.accent } else { t.panel };
                ui.painter().rect(r, 8.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                icons::paint(ui.painter(), r.shrink(10.0), icon, if open { t.accent_on } else { t.icon });
                if resp.clicked() {
                    if side == MobilePanel::Left && !open {
                        self.ui.open_left_panel();
                    } else {
                        self.ui.mobile_panel = if open { None } else { Some(side) };
                    }
                }
                ui.add_space(4.0);
            }
        });
    }

    /// One frame of the window drawing now.
    pub(crate) fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Bind this input batch before controls or pointer input can replace its composing
        // target/caret. Outgoing IME receipts run after mutation and cannot authorize it.
        let ime_input = shortcuts::ImeInputBaseline::capture(self);
        let t = theme::Tokens::get(&ctx);
        // A field that had the keyboard when the frame began keeps its keys from the shortcuts
        // even if it lets go during the frame (Enter commits it), or as it began (egui drops the
        // focus on Escape before the frame, which cancels the field's edit and nothing else).
        let typing = std::mem::take(&mut self.field_focus) || a11y::field_focused(&ctx);
        // Bind the originating field/file before tabs can switch. Its widget commits only after
        // processing this batch; delivery remains deferred until the following frame.
        let field_save = shortcuts::prepare_field_save(self, &ctx);
        let swallowed = shortcuts::swallowed_paste(self, &ctx);
        // A dialog that closed acts first, in the window it was opened from.
        self.poll_dialog(&ctx);
        // Control requests run first so their results reflect this frame.
        control::drain(self, &ctx);
        windows::requests(self, &ctx);
        selection_spacing::sync(self, &ctx);
        // Before any widget: the navigation keys (F6, Tab, arrows, Escape) move the keyboard.
        a11y::begin(self, &ctx, ui.scope_id());
        if self.ui.title_bar == TitleBar::Custom {
            tabs::resize_edges(&ctx);
        }
        if self.ui.icon_gallery {
            icons::gallery(&ctx);
        }
        theme::set_doc_profile(&ctx, self.session.doc().map_or(figcraft_color::ColorProfile::Srgb, |d| d.color_profile));
        widgets::set_nudge(&ctx, self.session.prefs.nudge_small, self.session.prefs.nudge_big);
        self.sync_dev_mode();
        self.end_pen_outside_design();
        tabs::relink_presents(self);
        tabs::tab_bar(self, ui);
        // Control/window work and this pass's physical tab click may have replaced the owner.
        // A shared field ID in that replacement must never consume the old commit marker.
        let field_save = shortcuts::revalidate_field_save(self, &ctx, field_save);
        if self.ui.present.is_some() {
            // A close request can arrive while the presenter fills the window. Let its unsaved
            // changes dialog take Escape and draw above the player just as it does in the editor.
            dialogs::keys(self, &ctx);
            present::show(self, ui);
            dialogs::show(self, &ctx);
            updates_ui::show(self, &ctx);
            // What went wrong while presenting (a widget's plugin failing) shows over it.
            self.toast_ui(&ctx);
            windows::requests(self, &ctx);
            shortcuts::clear_field_save_marker(&ctx);
            self.observe_ime(&ctx, false);
            return;
        }
        if !self.has_files() || self.ui.home {
            home::show(self, ui);
            // The shortcuts panel belongs to the canvas; it doesn't hang over the home screen
            // (cleared after the home widgets, which may have just asked for it).
            self.ui.shortcuts_panel = false;
            dialogs::keys(self, &ctx);
            dialogs::show(self, &ctx);
            updates_ui::show(self, &ctx);
            self.toast_ui(&ctx);
            windows::requests(self, &ctx);
            shortcuts::clear_field_save_marker(&ctx);
            self.observe_ime(&ctx, false);
            return;
        }
        let hidden = self.session.ds().map(|d| d.view.ui_hidden).unwrap_or(false);
        let narrow = !hidden && ui.available_width() < NARROW_WIDTH;
        let mobile = self.ui.mobile_panel.filter(|_| narrow);
        self.ui.narrow = narrow;
        // A narrow window shows the right panel when its button asks for it, a board's too.
        let right_open = (self.right_panel_shown(hidden) && !narrow) || mobile == Some(MobilePanel::Right);
        if !hidden && (!narrow || mobile == Some(MobilePanel::Left)) {
            egui::Panel::left("left_rail")
                .exact_size(56.0)
                .resizable(false)
                .frame(egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)))
                .show(ui, |ui| {
                    a11y::region(&mut self.ui.nav, ui, a11y::Region::Layers, egui::accesskit::Role::Navigation, i18n::tr("Sidebar"));
                    layers::rail(self, ui)
                });
        }
        // The panels are as wide as asked, less in a window too small for them and the canvas.
        let left_open = !hidden && if narrow { mobile == Some(MobilePanel::Left) } else { !self.ui.left_collapsed };
        let (left_w, right_w) = if narrow {
            let w = (ui.available_width() - MOBILE_STRIP).max(0.0);
            (if left_open { w } else { 0.0 }, w)
        } else {
            fit_panels(ui.available_width(), left_open.then_some(self.ui.left_width), right_open.then_some(self.ui.right_width))
        };
        let rows = ui.available_rect_before_wrap().y_range();
        let mut edges: Vec<(&str, f32, f32, f32)> = Vec::new();
        if left_open {
            if !narrow {
                edges.push(("left", ui.available_rect_before_wrap().min.x + left_w, left_w, 1.0));
            }
            egui::Panel::left("left_panel")
                .resizable(false)
                .exact_size(left_w)
                .frame(egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)))
                .show(ui, |ui| {
                    let label = match self.ui.left_tab {
                        _ if self.ui.versions.open => i18n::tr("Version history"),
                        LeftTab::File => i18n::tr("Layers"),
                        LeftTab::Assets => i18n::tr("Assets"),
                    };
                    a11y::region(&mut self.ui.nav, ui, a11y::Region::Layers, egui::accesskit::Role::Pane, label);
                    layers::left_panel(self, ui)
                });
        }
        if right_open {
            if !narrow {
                edges.push(("right", ui.available_rect_before_wrap().max.x - right_w, right_w, -1.0));
            }
            egui::Panel::right("right_panel")
                .resizable(false)
                .exact_size(right_w)
                .frame(egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)))
                .show(ui, |ui| {
                    let label = match self.ui.right_tab {
                        RightTab::Design => i18n::tr("Design"),
                        RightTab::Prototype => i18n::tr("Prototype"),
                        RightTab::Inspect => i18n::tr("Dev Mode"),
                    };
                    a11y::region(&mut self.ui.nav, ui, a11y::Region::Properties, egui::accesskit::Role::Pane, label);
                    right::right_panel(self, ui)
                });
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| {
            let worker = self.worker.take();
            let mut worker = worker.unwrap_or_else(|| render_worker::Worker::new(ctx.clone()));
            canvas::show(self, ui, &mut worker);
            self.worker = Some(worker);
            if !hidden && mobile.is_none() {
                ui.scope_builder(egui::UiBuilder::new().id_salt("toolbar"), |ui| {
                    a11y::container(ui, egui::accesskit::Role::Toolbar, i18n::tr("Tools"));
                    toolbar::show(self, ui);
                });
            }
        });
        if narrow {
            let x = if mobile == Some(MobilePanel::Left) { ctx.content_rect().max.x - MOBILE_STRIP + 4.0 } else { ctx.content_rect().min.x + 4.0 };
            self.mobile_buttons(&ctx, egui::pos2(x, rows.min + 8.0));
        }
        // The panels' edges, over the canvas and the panels (drawn after them: they take the press).
        for (name, x, shown, grows) in edges {
            if let Some(w) = Self::panel_edge(ui, name, x, rows, shown, grows) {
                let (l, r) = if name == "left" { (w, self.ui.right_width) } else { (self.ui.left_width, w) };
                self.set_panel_widths(l, r);
                self.panels_unsaved = true;
            }
        }
        // Kept once the drag is over (not on every pointer move).
        if self.panels_unsaved && !ctx.input(|i| i.pointer.any_down()) {
            self.panels_unsaved = false;
            self.services.save_prefs(&self.session.prefs);
        }
        // Before the shortcuts: keys the hovered pane's prototype uses don't reach the editor, and
        // the arrows step through comment threads with the Comment tool.
        present::show_pane(self, &ctx);
        comments::keys(self, &ctx);
        dev_compare::keys(self, &ctx);
        branch_ui::keys(self, &ctx);
        live::dialog_keys(self, &ctx);
        dialogs::keys(self, &ctx);
        shortcuts::handle_input(self, &ctx, typing, swallowed, ime_input, field_save);
        menus::popups(self, &ctx);
        variable_ui::picker(self, &ctx);
        style_picker::show(self, &ctx);
        self.end_unwanted_text_preview(&ctx);
        self.end_pen_outside_design();
        dialogs::show(self, &ctx);
        updates_ui::show(self, &ctx);
        self.toast_ui(&ctx);
        // A rename started after the layers panel drew (Cmd/Ctrl+R, a double-click) opens its field
        // next frame.
        if self.ui.rename.is_some_and(|r| r.start) && !hidden && !self.ui.left_collapsed && self.ui.left_tab == LeftTab::File {
            ctx.request_repaint();
        }
        a11y::end(self, &ctx);
        self.field_focus = a11y::field_focused(&ctx);
        windows::requests(self, &ctx);
        // Your pointer, selection and view to the others in each live session (when they
        // changed), from the window with its file.
        let windows = &self.windows;
        self.session.live_sync_in(|uid| uid.map_or(0, |u| windows.of(u)) == windows.current);
        // The canvas editor's IME context is observed exactly once per pass, after every command
        // this pass ran (shortcuts, menus, dialogs, window requests and live sync). A focused panel
        // field or an open palette owns the platform IME instead; the early `home`/present paths
        // above make the same single idle observation.
        self.observe_ime(&ctx, !typing && !self.field_focus && self.ui.palette.is_none());
    }

    /// The pass's one IME observation: the focused on-canvas editor, or an explicit idle.
    ///
    /// Calling `ImeContextTracker::observe` more than once in a pass would treat an idle followed
    /// by a focused observation as a context change (dropping the frame's receipts) and bump the
    /// generation. Keeping this in one place makes the fork contract structural.
    fn observe_ime(&mut self, ctx: &egui::Context, focused: bool) {
        #[cfg(test)]
        {
            self.ime_observations = self.ime_observations.saturating_add(1);
        }
        if focused {
            shortcuts::ime_output(self, ctx);
        } else {
            let _ = self.ime.observe(ctx.viewport_id(), None);
        }
    }

    /// Draw the toast: start its lifetime on the frame **after** the first drawn frame, request
    /// that frame and the frame that hides it, expire and remove it at the deadline, and place it
    /// at the bottom centre.
    fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((msg, clock)) = &mut self.ui.toast else { return };
        let now = ctx.input(|i| i.time);
        // The clock starts on the frame after the one that first draws the toast: that frame's
        // time was taken before whatever raised it ran (a file dialog, a long open), and would
        // count that wait against it, so a failed open's message expired before it was seen.
        let left = match *clock {
            ToastClock::Raised => {
                *clock = ToastClock::Drawn;
                ctx.request_repaint();
                TOAST_SECS
            }
            ToastClock::Drawn => {
                *clock = ToastClock::Since(now);
                TOAST_SECS
            }
            ToastClock::Since(t) => TOAST_SECS - (now - t),
        };
        if left <= 0.0 {
            self.ui.toast = None;
            return;
        }
        let msg = msg.clone();
        // Nothing moves while it shows: the next frame it needs is the one that hides it.
        if matches!(clock, ToastClock::Since(_)) {
            repaint_in(ctx, left);
        }
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("toast"))
            .fixed_pos(egui::pos2(screen.center().x, screen.max.y - 96.0))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(0x1E, 0x1E, 0x1E))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        // Wrapped at a readable width (the area alone would wrap at a sliver).
                        let width = (screen.width() - 64.0).clamp(120.0, 480.0);
                        let galley = ui.painter().layout(msg, theme::regular(12.0), egui::Color32::WHITE, width);
                        ui.label(galley);
                    });
            });
    }

    /// UI state as JSON (control channel `ui.inspect`).
    pub fn inspect(&self) -> Value {
        let mut state = json!({
            "rightTab": self.ui.right_tab,
            "leftTab": self.ui.left_tab,
            "home": self.ui.home,
            "online": online::inspect(self),
            "tabs": self.session.docs.iter().map(|d| json!({"name": d.doc.name, "live": tabs::live_mark(self, d.uid).map(|shared| if shared { "shared" } else { "private" })})).collect::<Vec<_>>(),
            "homeSection": self.ui.home_view.section.id(),
            "homeRecents": home_recents::inspect(self),
            "dark": self.ui.dark,
            "focus": {"region": self.ui.nav.region.id(), "keyboard": self.ui.nav.keyboard},
            "reduceMotion": self.reduce_motion(),
            "presenting": self.ui.present.is_some(),
            "presenter": tabs::shown_or_paused(self).map(present::Presenter::inspect),
            "presentOptions": {"scale": self.ui.present_view.scale.id(), "deviceFrame": self.ui.present_view.device_frame, "comments": self.ui.present_view.comments, "hotspotHints": self.ui.present_view.hotspot_hints, "inspect": self.ui.present_view.inspect},
            "preview": self.ui.preview.as_ref().map(|p| p.player.inspect()),
            "mainMenu": self.ui.main_menu,
            "palette": self.ui.palette.is_some(),
            "variables": self.ui.variables.is_some(),
            "versionHistory": self.ui.versions.open,
            "branchReview": self.ui.branch_review.as_ref().map(|r| json!({"id": r.id, "picks": r.picks, "mode": if r.mode == dev_compare::Mode::Overlay { "overlay" } else { "sideBySide" }})),
            "devCompare": self.ui.dev.compare.as_ref().map(|c| json!({"id": c.id, "mode": if c.mode == dev_compare::Mode::Overlay { "overlay" } else { "sideBySide" }, "split": c.split, "version": c.version})),
            "devFocus": self.session.ds().ok().and_then(figcraft_engine::DocState::focused),
            "annotationFilter": self.ui.dev.categories.id(),
            "colorProfileDialog": self.ui.color_profile.map(|p| p.id()),
            "stylePicker": self.ui.style_picker.as_ref().map(|p| p.slot().key()),
            "stylePickerView": if self.ui.style_grid { "grid" } else { "list" },
            "textPreview": figcraft_engine::textpreview::active(&self.session),
            "comments": {
                "open": self.ui.comments.open,
                "draft": self.ui.comments.draft.as_ref().map(|d| [d.0, d.1]),
                "filter": self.ui.comments.filter.id(),
                "sort": self.ui.comments.sort.id(),
                "search": self.ui.comments.search,
                "editing": self.ui.comments.editing.as_ref().map(|e| [e.0, e.1 as u64]),
                "mentions": self.ui.comments.mention.names(),
            },
            "frameMs": self.ui.frame_ms,
            "renderMs": self.ui.render_ms,
            "framesDrawn": self.frame_stats.frames,
            "repaintCauses": self.frame_stats.causes(),
            "canvas": {
                "rect": [self.canvas.rect.min.x, self.canvas.rect.min.y, self.canvas.rect.max.x, self.canvas.rect.max.y],
                "renderWork": self.canvas.render_work.map(|w| match w {
                    figcraft_render::Work::Reused => json!({"kind": "reused"}),
                    figcraft_render::Work::Partial { pixels } => json!({"kind": "partial", "pixels": pixels}),
                    figcraft_render::Work::Full => json!({"kind": "full"}),
                }),
                "quality": if self.canvas.tex_coarse { "preview" } else { "refined" },
                "previewLatencyMs": self.canvas.preview_latency_ms,
                "refineLatencyMs": self.canvas.refine_latency_ms,
            },
            "live": live::inspect(self),
            "window": self.windows.current,
            "windows": windows::list(self),
            "tool": self.session.tool_info(),
            "cursor": self.canvas.cursor.map(cursors::Shown::name),
            "panels": {"left": self.ui.left_width, "right": self.ui.right_width, "leftCollapsed": self.ui.left_collapsed},
            "status": self.session.status(),
            "lastError": self.ui.last_error,
            "importReport": self.ui.import_report.as_ref().map(import_report::Report::inspect),
        });
        state["homeSearch"] = json!(self.ui.home_view.file_query);
        state["homeProjects"] = home_projects::inspect(self);
        // Set apart from the literal above, which is at the macro recursion limit.
        state["uiScale"] = json!({"choice": self.session.prefs.ui_scale, "zoom": self.ui.ui_scale.zoom, "drawn": self.ui.ui_scale.drawn, "limited": self.ui.ui_scale.limited});
        state["pendingOpens"] = json!(self.ui.pending_opens.len());
        state["missingFonts"] = json!(self.ui.missing_fonts.as_ref().map(missing_fonts::Shown::inspect));
        state["presentTabs"] = tabs::present_tabs_inspect(self);
        state
    }
}

/// The file types File ▸ Open and a drop on the home screen open.
pub(crate) const OPEN_EXTENSIONS: &[&str] = &["figcraft", "fig", "sketch", "json", "svg", "png", "jpg", "jpeg", "webp", "gif", "tif", "tiff"];

/// Whether `path` names a file type that opens as a document (the extension, in any case).
pub(crate) fn opens(path: &Path) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|e| OPEN_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Whether `path` is a whole document (`.figcraft`, `.fig`, `.sketch`, file JSON) rather than an image or SVG:
/// dropped on the canvas it opens as a file of its own instead of adding its pages to the open one.
pub(crate) fn is_document(path: &Path) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|e| ["figcraft", "fig", "sketch", "json"].iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Reads the contents of dropped file `name` that has no file on disk to read by path.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn dropped_bytes(f: &egui::DroppedFileHandle, name: &str) -> ReadFuture {
    Box::pin(std::future::ready(f.bytes().map_err(|_| unreadable(name))))
}

/// Reads the contents of dropped file `name`; the browser hands them over asynchronously.
#[cfg(target_arch = "wasm32")]
pub(crate) fn dropped_bytes(f: &egui::DroppedFileHandle, name: &str) -> ReadFuture {
    let (f, name) = (f.clone(), name.to_string());
    Box::pin(async move {
        if let Some(file) = f.web_file() {
            browser_can_read(&name, file.size())?;
        }
        f.bytes_async().await.map_err(|_| unreadable(&name))
    })
}

/// While files are dragged over the window: a tinted, outlined `rect` with `label` in its middle,
/// over everything else, saying what a drop does there.
pub(crate) fn drop_overlay(ctx: &egui::Context, rect: egui::Rect, label: &str) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let t = theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop_overlay")));
    let area = rect.shrink(4.0);
    painter.rect_filled(area, egui::CornerRadius::same(8), t.accent.gamma_multiply(0.12));
    painter.rect_stroke(area, egui::CornerRadius::same(8), egui::Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
    let galley = painter.layout_no_wrap(label.to_owned(), theme::semibold(14.0), t.text);
    let pill = egui::Rect::from_center_size(area.center(), galley.size() + egui::vec2(32.0, 16.0));
    painter.rect_filled(pill, egui::CornerRadius::same(8), t.panel);
    painter.rect_stroke(pill, egui::CornerRadius::same(8), egui::Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
    painter.galley(pill.center() - galley.size() / 2.0, galley, t.text);
    // Hovering sends no pointer events on every platform: keep the overlay current.
    ctx.request_repaint();
}

#[cfg(test)]
mod selection_spacing_tests;

#[cfg(test)]
mod type_settings_tests;

#[cfg(test)]
mod typography_scrub_tests;
