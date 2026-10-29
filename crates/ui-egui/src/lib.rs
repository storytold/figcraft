//! The FigCraft egui frontend.
//!
//! A thin shell over [`figcraft_engine::Session`]: panels read engine state and act by running
//! commands; the canvas forwards pointer events to the engine's tools and draws overlays. Nothing
//! below this crate knows about egui, so the frontend can be replaced.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod a11y;
pub mod bidi;
pub mod canvas;
pub mod cjk;
pub mod control;
pub mod dialogs;
pub mod home;
pub mod i18n;
pub mod icons;
pub mod live;
pub mod menus;
pub mod pending_open;
pub mod popover;
pub mod present;
pub mod shortcuts;
pub mod tabs;
pub mod theme;
pub mod toolbar;
pub mod ui_scale;
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
mod blend_preview_tests;
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
mod component_swap_search_tests;
#[cfg(test)]
mod context_menu_tests;
#[cfg(test)]
mod corner_individual_tests;
#[cfg(test)]
mod crop_image_tests;
#[cfg(test)]
mod cursors_tests;
#[cfg(test)]
mod design_paste_tests;
#[cfg(test)]
mod dev_mode_tests;
#[cfg(test)]
mod devmode_tail_tests;
#[cfg(test)]
mod display_name_tests;
#[cfg(test)]
mod duplicate_paint_tests;
#[cfg(test)]
mod export_preview_tests;
#[cfg(test)]
mod field_tests;
#[cfg(test)]
mod font_menu_regrow_tests;
#[cfg(test)]
mod frame_label_tests;
#[cfg(test)]
mod gap_handle_tests;
#[cfg(test)]
mod gradient_shorthand_tests;
#[cfg(test)]
mod guide_release_tests;
#[cfg(test)]
mod guide_snap_tests;
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
mod live_joining_tests;
#[cfg(test)]
mod live_server_tests;
#[cfg(test)]
mod live_share_tests;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod live_text_tests;
#[cfg(test)]
mod local_fonts_tests;
#[cfg(test)]
mod make_slot_naming_tests;
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
mod panels;
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
mod popover_tests;
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
mod stroke_row_tests;
#[cfg(test)]
mod style_folder_rename_tests;
#[cfg(test)]
mod style_props_tests;
#[cfg(test)]
mod text_enter_tests;
#[cfg(test)]
mod titlebar_tests;
#[cfg(test)]
mod ui_params_tests;
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
pub use canvas::board_bar;
pub use canvas::board_facilitate;
pub use canvas::comments;
pub use canvas::cursors;
pub use canvas::frame_labels;
pub use canvas::frame_stats;
pub use canvas::guides;
pub use canvas::handles;
pub use canvas::live_text;
pub use canvas::measure_overlay;
pub use canvas::proto_canvas;
pub use canvas::render_worker;
use canvas::selection_spacing;
pub use canvas::widget_bar;
pub use dialogs::branch_ui;
pub use dialogs::credits;
pub use dialogs::import_report;
pub use dialogs::missing_fonts;
pub use dialogs::plugins_ui;
pub use dialogs::updates_ui;
pub use dialogs::variables_panel;
pub use home::home_recents;
pub use home::online;
pub use panels::assets_panel;
pub use panels::component_panel;
pub use panels::dev_compare;
pub use panels::dev_mode;
pub use panels::export_panel;
pub use panels::frame_presets;
pub use panels::layers;
pub use panels::layout_panel;
pub use panels::list_drag;
pub use panels::proto_editor;
pub use panels::proto_panel;
pub use panels::right;
pub use panels::scale_panel;
pub use panels::section_panel;
pub use panels::selection_colors;
pub use panels::shortcuts_panel;
pub use panels::typography;
pub use panels::variable_ui;
pub use panels::vector_panel;
pub use panels::versions;
use widgets::board_picker;
pub use widgets::colorpicker;
pub use widgets::field_math;
pub use widgets::font_preview;
pub use widgets::style_picker;

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

mod app;
mod clock;
mod open;
mod services;
mod state;

pub(crate) use clock::repaint_in;
pub use clock::{AUTO_SAVE_SECS, AUTOSAVE_SECS, FrameClock, ToastClock};
pub(crate) use open::{drop_overlay, dropped_bytes, is_document, opens};
pub use services::{
    ContentsFuture, DialogFuture, MAX_CLIPBOARD_PNG_BYTES, NoServices, ReadFuture, Services, browser_can_read, browser_stopped, egui_copy_png,
    egui_copy_text, unreadable,
};
use services::{DialogDone, PickFuture, Picked, RepaintWaker};
pub use state::{
    AutoSaveEncoding, CANVAS_MIN, LEFT_PANEL, LeftTab, MobilePanel, NARROW_WIDTH, PANEL_DEFAULT, RIGHT_PANEL, RightTab, TitleBar, fit_panels,
};

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

/// UI-only state (serialisable for `ui.inspect`).
pub struct UiState {
    /// The widths asked for the side panels (a drag of their edge; [`fit_panels`] gives what a
    /// small window draws).
    pub left_width: f32,
    pub right_width: f32,
    /// The right sidebar as drawn this frame, after fitting the window.
    pub right_panel_rect: Option<egui::Rect>,
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
    /// Crop mode (`image.crop`): the layer and the index of the image fill whose crop handles
    /// show on the canvas until Enter, Escape or a click outside the image.
    pub crop: Option<(NodeId, usize)>,
    /// The colour picker's popover while it is open (its place survives a drag).
    pub picker_popover: Option<popover::Popover>,
    /// The control that opened the picker, refreshed every frame it is drawn (the panel scrolls):
    /// a press in it is held out of the popover's closing, so its own click toggles the picker.
    pub picker_button: Option<egui::Rect>,
    /// The opening widget, so only that control refreshes the held rectangle as it moves.
    pub picker_button_id: Option<egui::Id>,
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
    /// The style picker's popover while it is open (its place survives a drag).
    pub style_picker_popover: Option<popover::Popover>,
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
            right_panel_rect: None,
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
            crop: None,
            picker_popover: None,
            picker_button: None,
            picker_button_id: None,
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
            style_picker_popover: None,
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

#[cfg(test)]
mod selection_spacing_tests;

#[cfg(test)]
mod type_settings_tests;

#[cfg(test)]
mod typography_scrub_tests;
