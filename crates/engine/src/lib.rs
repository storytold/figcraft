//! The FigCraft engine.
//!
//! Every user-visible action is a command with a stable id (`node.create`, `align.left`,
//! `layout.add`, `component.create`…) taking JSON params. The egui UI, the CLI, the control channel
//! and the MCP server all go through [`Session::execute`], so everything is scriptable and every
//! change is one undo step. Pointer tools ([`tools`]) and text editing reduce to the same document
//! edits and are journaled the same way.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// Tests make and inspect files directly; shipped code goes through `file_access` (clippy.toml).
#![cfg_attr(test, allow(clippy::disallowed_methods))]

pub mod al_keys;
pub mod annotations;
pub mod arc_handles;
pub mod background;
pub mod batch;
pub mod board;
pub mod board_table;
pub mod board_templates;
pub mod booleans;
pub mod branches;
pub mod catalog;
pub mod cmd;
pub mod code_connect;
pub mod codegen;
pub mod collab;
pub mod component_codegen;
pub mod components;
pub mod datafill;
pub mod dev_measurements;
pub mod dev_resources;
pub mod devcompare;
pub mod devinspect;
pub mod distance;
pub mod doc_urls;
pub mod drop;
pub mod file_access;
pub mod fonts;
pub mod guard;
pub mod guides;
pub mod inspect;
pub mod invite_links;
pub mod keymap;
pub mod library;
pub mod library_server;
pub mod library_swap;
pub mod live;
pub mod measure;
pub mod online;
pub mod params;
pub mod playground;
pub mod plugin_directory;
pub mod plugins;
pub mod projects;
pub mod proto_run;
pub mod rename;
pub mod rotation_origin;
pub mod selection_colors;
pub mod selection_reorder;
pub mod selection_spacing;
pub mod shape_handles;
pub mod shapebuilder;
pub mod snap;
pub mod style_vars;
pub mod synthetic;
pub mod synthetic_ds;
pub mod tailwind;
pub mod templates;
pub mod textedit;
pub mod textfit;
pub mod textpreview;
pub mod token_roles;
pub mod tools;
pub mod vectoredit;
pub mod widgets;

pub use batch::{BatchScope, BatchStep};
pub use cmd::{CommandSpec, all_commands, find_command};
pub use figcraft_collab;
pub use figcraft_doc as doc;
pub use figcraft_svg::looks_like_svg;
pub use tools::{PointerEvent, PointerKind, ToolId};

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use figcraft_doc::{Document, Node, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no open document")]
    NoDocument,
    #[error("no such node {0}")]
    NoNode(NodeId),
    #[error("{0}")]
    Other(String),
    /// A command run in the background sent a request to a server and runs again with the answer
    /// ([`background`]); never returned to other callers.
    #[error("waiting for the server")]
    Waiting,
    /// A bug: the command panicked. The document was rolled back.
    #[error("internal error in `{cmd}`: {msg} (the document was left as it was before; please report this bug)")]
    Internal { cmd: String, msg: String },
}

impl From<figcraft_doc::DocError> for EngineError {
    fn from(e: figcraft_doc::DocError) -> Self {
        match e {
            figcraft_doc::DocError::NoNode(id) => EngineError::NoNode(id),
            other => EngineError::Other(other.to_string()),
        }
    }
}

impl EngineError {
    /// This error from step `step` of a batch, saying so in its message; the kind stays (errors
    /// without a message stay as they are).
    fn at_step(self, step: usize) -> Self {
        match self {
            EngineError::BadParams { cmd, msg } => EngineError::BadParams { cmd, msg: format!("step {step}: {msg}") },
            EngineError::Other(msg) => EngineError::Other(format!("step {step}: {msg}")),
            e => e,
        }
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

pub fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.to_string(), msg: msg.into() }
}

/// One undo step.
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub label: String,
    pub doc: Arc<Document>,
    pub selection: Vec<NodeId>,
    pub page: NodeId,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub undo: Vec<HistoryEntry>,
    pub redo: Vec<HistoryEntry>,
    pub limit: usize,
}

/// The canvas view of a document: what's at the centre of the viewport and the zoom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    /// Page coordinate at the viewport centre.
    pub cx: f64,
    pub cy: f64,
    /// Zoom (1 = 100%).
    pub zoom: f64,
    /// Viewport size in logical pixels (set by the UI; used by zoom-to-fit).
    pub vw: f64,
    pub vh: f64,
    pub pixel_grid: bool,
    pub layout_grids: bool,
    pub rulers: bool,
    pub outlines: bool,
    /// Outline mode for the selected layers only (the rest draws normally).
    pub outlines_selection: bool,
    /// Outline mode also outlines hidden layers (View ▸ Outlines ▸ Include hidden layers).
    pub outline_hidden: bool,
    /// Outline mode also draws every layer's box (View ▸ Outlines ▸ Include object bounds).
    pub outline_bounds: bool,
    /// Outline the masks (they don't draw themselves).
    pub mask_outlines: bool,
    pub pixel_preview: bool,
    /// Pixel preview resolution: rendered pixels per page pixel (1 or 2).
    pub pixel_scale: u8,
    pub show_slices: bool,
    pub comments: bool,
    /// Others' cursors in a live session (View ▸ Multiplayer cursors).
    pub multiplayer_cursors: bool,
    pub frame_outlines: bool,
    /// Soft-proof sRGB: the canvas marks the colours of a Display P3 file that sRGB can't show.
    pub proof_srgb: bool,
    pub snap_geometry: bool,
    pub snap_objects: bool,
    pub snap_pixel: bool,
    pub ui_hidden: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            cx: 0.0,
            cy: 0.0,
            zoom: 1.0,
            vw: 1200.0,
            vh: 800.0,
            pixel_grid: true,
            layout_grids: true,
            rulers: false,
            outlines: false,
            outlines_selection: false,
            outline_hidden: false,
            outline_bounds: false,
            mask_outlines: false,
            pixel_preview: false,
            pixel_scale: 1,
            show_slices: false,
            comments: true,
            multiplayer_cursors: true,
            frame_outlines: false,
            proof_srgb: false,
            snap_geometry: true,
            snap_objects: true,
            snap_pixel: true,
            ui_hidden: false,
        }
    }
}

impl ViewState {
    pub const MIN_ZOOM: f64 = 0.02;
    pub const MAX_ZOOM: f64 = 256.0;
    /// Page coordinate at the viewport's top-left.
    pub fn origin(&self) -> (f64, f64) {
        let z = self.zoom.max(1e-9);
        (self.cx - self.vw / 2.0 / z, self.cy - self.vh / 2.0 / z)
    }
    pub fn set_zoom_at(&mut self, zoom: f64, anchor: (f64, f64)) {
        let z = if zoom.is_finite() { zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM) } else { self.zoom };
        let old = self.zoom.max(1e-9);
        // Keep the page point under `anchor` fixed.
        self.cx = anchor.0 + (self.cx - anchor.0) * old / z;
        self.cy = anchor.1 + (self.cy - anchor.1) * old / z;
        self.zoom = z;
    }
    /// Fit a page rectangle into the viewport with margin.
    pub fn fit(&mut self, r: figcraft_geom::Rect, max_zoom: f64) {
        if !(r.width() > 0.0 || r.height() > 0.0) || !r.x0.is_finite() {
            self.cx = r.center().x;
            self.cy = r.center().y;
            return;
        }
        let margin = 64.0;
        let zx = (self.vw - margin * 2.0).max(50.0) / r.width().max(1e-6);
        let zy = (self.vh - margin * 2.0).max(50.0) / r.height().max(1e-6);
        self.zoom = zx.min(zy).clamp(Self::MIN_ZOOM, max_zoom);
        self.cx = r.center().x;
        self.cy = r.center().y;
    }
    /// The zoom from which the pixel grid shows (400%).
    pub const PIXEL_GRID_ZOOM: f64 = 4.0;
    /// Whether the pixel grid draws: it's on and the canvas is zoomed in to 400% or more.
    pub fn pixel_grid_shown(&self) -> bool {
        self.pixel_grid && self.zoom >= Self::PIXEL_GRID_ZOOM - 1e-9
    }
    /// Pixel preview's resolution (rendered pixels per page pixel) when it's on and zoomed in past
    /// it; below that the canvas is already coarser than the preview.
    pub fn pixel_preview_scale(&self) -> Option<f64> {
        let k = f64::from(self.pixel_scale.clamp(1, 2));
        (self.pixel_preview && self.zoom > k).then_some(k)
    }
    /// A wheel or trackpad scroll of (`dx`, `dy`) screen points: the canvas pans with it (content
    /// follows the fingers); with Shift a plain vertical wheel pans sideways.
    pub fn scroll(&mut self, dx: f64, dy: f64, shift: bool) {
        let (dx, dy) = if shift && dx == 0.0 { (dy, 0.0) } else { (dx, dy) };
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let z = self.zoom.max(1e-6);
        self.cx -= dx / z;
        self.cy -= dy / z;
    }
    pub fn page_to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        let (ox, oy) = self.origin();
        ((x - ox) * self.zoom, (y - oy) * self.zoom)
    }
    pub fn screen_to_page(&self, x: f64, y: f64) -> (f64, f64) {
        let (ox, oy) = self.origin();
        let z = self.zoom.max(1e-9);
        (ox + x / z, oy + y / z)
    }
}

/// An in-progress gesture: the document before it started, so previews re-apply from scratch.
#[derive(Clone, Debug)]
pub struct Interaction {
    pub label: String,
    pub doc: Arc<Document>,
    pub selection: Vec<NodeId>,
}

/// The text being edited on the canvas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextEdit {
    pub id: NodeId,
    /// Caret position and selection anchor (char indices).
    pub caret: usize,
    pub anchor: usize,
    /// The node was created by this edit (deleted if left empty).
    pub created: bool,
    /// Identity of this editing session (`textedit::begin` assigns a fresh one).
    ///
    /// The IME owner is `(document uid, node)`, which stays the same through ordinary edits
    /// and caret moves; this token changes only when editing is started again, so an IME
    /// context is never silently inherited by a replaced target that reuses the same node.
    ///
    /// It is deliberately not serialized: current consumers only inspect a live session. A
    /// serialized `TextEdit` restored from data would get zero here, so it must not be restored
    /// into a live editor without minting (or otherwise invalidating) a new session.
    #[serde(default, skip_serializing)]
    pub editing_session: u64,
    /// An IME composition in progress, shown inline at the caret; the document gets it on commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preedit: Option<Preedit>,
}

/// IME composition text (not yet in the document) and the IME's cursor inside it (chars).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preedit {
    pub text: String,
    pub cursor: usize,
}

/// Per-document editing state.
#[derive(Clone, Debug)]
pub struct DocState {
    pub doc: Arc<Document>,
    pub selection: Vec<NodeId>,
    pub page: NodeId,
    pub history: History,
    saved_doc: Arc<Document>,
    pub path: Option<String>,
    pub revision: u64,
    pub view: ViewState,
    pub interaction: Option<Interaction>,
    pub text_edit: Option<TextEdit>,
    /// Hovered node (canvas hover outline).
    pub hover: Option<NodeId>,
    /// Vector edit mode: the vector being edited, its selected points and the edit tool.
    pub vector_edit: Option<vectoredit::VectorEdit>,
    pub uid: u64,
    /// The file's version history, oldest first (saved in the `.figcraft` file; not undoable).
    pub versions: Vec<figcraft_format::Version>,
    /// The file's branches (saved in the `.figcraft` file; not undoable).
    pub branches: Vec<figcraft_format::Branch>,
    /// This tab shows a branch: the `uid` of its main file's tab and the branch id. Its edits go
    /// into the main file's branch (and are saved with the main file).
    pub branch_of: Option<(u64, String)>,
    /// The version history or the branches changed since the last save.
    versions_changed: bool,
    /// A version shown in place of the document (read-only).
    pub preview: Option<VersionPreview>,
    /// The document under a live text preview (a hovered font family or style, text style or
    /// font instance, a dragged axis slider): `doc` shows the preview until [`textpreview::end`] puts this back.
    pub text_preview: Option<Arc<Document>>,
    /// The document as of the last version saved or restored (none yet): autosaves, restores and
    /// merges only add a checkpoint for changes since.
    checkpoint: Option<Arc<Document>>,
    /// Dev Mode focus view: the frame or section shown alone on the canvas (see [`Self::focused`]).
    pub focus: Option<NodeId>,
    /// The ruler guide clicked last: its page or frame and index (see [`Self::selected_guide`]).
    pub guide: Option<(NodeId, usize)>,
    /// The selection's rotation origin, when moved or shown (see [`rotation_origin`]; not saved).
    pub(crate) rotation_origin: Option<rotation_origin::RotationOrigin>,
    /// The View toggles the preferences keep were applied ([`Session::apply_view_prefs`]).
    view_from_prefs: bool,
    /// The last duplicate while its copies are the selection: Duplicate repeats its offset (see
    /// [`cmd::edit::Duplicated`]; not saved).
    pub(crate) last_duplicate: Option<cmd::edit::Duplicated>,
    images_left_in_file: Option<(String, usize)>,
    /// The file on disk as this tab last read or wrote it: a save refuses to replace what another
    /// app (figcraft-cli, say) wrote to it since (#1330).
    on_disk: Option<library::Stamp>,
}

/// A version shown on the canvas in place of the document: read-only until it's restored or the
/// preview ends, which puts the document back as it was.
#[derive(Clone, Debug)]
pub struct VersionPreview {
    pub id: String,
    pub name: String,
    /// When the version was saved (Unix seconds).
    pub time: u64,
    doc: Arc<Document>,
    selection: Vec<NodeId>,
    page: NodeId,
}

/// Why edits are refused while a version is shown.
pub const PREVIEW_READ_ONLY: &str = "you're viewing an earlier version: restore it or go back to the current version to edit";

/// Commands that first end a version preview: they act on the document itself. Saving does so
/// only with a native destination; export copies and pending format choices keep the preview.
const ENDS_PREVIEW: &[&str] =
    &["file.save", "file.saveAs", "file.markSaved", "file.close", "file.revert", "file.saveVersion", "file.duplicate", "version.restore"];

static NEXT_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl DocState {
    pub fn new(mut doc: Document, path: Option<String>) -> Self {
        if doc.pages.is_empty() {
            doc.repair();
        }
        doc.link_layer_text_styles();
        cmd::styles::apply_paint_bindings_on_load(&mut doc);
        components::mark_layer_modes(&mut doc);
        // Initial import settlement belongs to the saved baseline, not the user's first edit.
        let page = doc.current_page.filter(|p| doc.pages.contains(p)).or_else(|| doc.first_page()).unwrap_or(NodeId(0));
        let doc = Arc::new(doc);
        let mut st = Self {
            checkpoint: None,
            saved_doc: doc.clone(),
            doc,
            selection: Vec::new(),
            page,
            history: History { limit: 1000, ..Default::default() },
            path,
            revision: 1,
            view: ViewState::default(),
            interaction: None,
            text_edit: None,
            hover: None,
            vector_edit: None,
            uid: NEXT_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            versions: Vec::new(),
            branches: Vec::new(),
            branch_of: None,
            versions_changed: false,
            preview: None,
            text_preview: None,
            focus: None,
            guide: None,
            rotation_origin: None,
            view_from_prefs: false,
            last_duplicate: None,
            images_left_in_file: None,
            on_disk: None,
        };
        st.zoom_to_fit();
        st
    }

    pub fn with_images_left_in_file(mut self, path: &str, count: usize) -> Self {
        self.images_left_in_file = (count > 0).then(|| (path.to_string(), count));
        self
    }

    /// The tab's file now holds what this tab read or wrote.
    pub(crate) fn note_on_disk(&mut self) {
        self.on_disk = self.path.as_deref().and_then(|p| library::stat(p).ok());
    }

    pub fn images_lost_saving_to(&self, path: &str) -> Option<usize> {
        self.images_left_in_file.as_ref().filter(|(file, _)| file_access::same_file(file, path)).map(|(_, count)| *count)
    }

    /// The selected ruler guide (its owner and index): it counts while no layer is selected and it
    /// is still there on this page.
    pub fn selected_guide(&self) -> Option<(NodeId, usize)> {
        let (owner, i) = self.guide.filter(|_| self.selection.is_empty())?;
        let on_page = owner == self.page || self.doc.top_level_frame(owner).is_some_and(|f| self.doc.ancestors(f).contains(&self.page));
        (on_page && guides::keeps_guides(&self.doc, owner) && self.doc.node(owner).is_some_and(|n| i < n.guides().len())).then_some((owner, i))
    }

    /// With the version history read from its file.
    pub fn with_versions(mut self, versions: Vec<figcraft_format::Version>) -> Self {
        self.versions = versions;
        self
    }

    /// With the branches read from its file.
    pub fn with_branches(mut self, branches: Vec<figcraft_format::Branch>) -> Self {
        self.branches = branches;
        self
    }

    /// The branches, for editing (the file then has unsaved changes).
    pub fn branches_mut(&mut self) -> &mut Vec<figcraft_format::Branch> {
        self.versions_changed = true;
        &mut self.branches
    }

    /// The document itself, not a version or live text preview shown in its place.
    pub fn document_itself(&self) -> &Arc<Document> {
        self.preview.as_ref().map(|p| &p.doc).or(self.text_preview.as_ref()).unwrap_or(&self.doc)
    }

    /// The version history, for editing (the file then has unsaved changes).
    pub fn versions_mut(&mut self) -> &mut Vec<figcraft_format::Version> {
        self.versions_changed = true;
        &mut self.versions
    }

    /// The document changed since the last version was saved or restored (always, before one).
    pub fn changed_since_checkpoint(&self) -> bool {
        let doc = self.preview.as_ref().map_or(&self.doc, |p| &p.doc);
        // Normalising after a command can copy a document without changing it: compare contents then.
        self.checkpoint.as_ref().is_none_or(|c| !Arc::ptr_eq(doc, c) && **doc != **c)
    }

    /// Note that a version of the current document was just saved (or restored).
    pub(crate) fn mark_checkpoint(&mut self) {
        self.checkpoint = Some(self.doc.clone());
    }

    /// Push a snapshot undo step: the document was `before` (shown with `selection` on `page`).
    pub(crate) fn push_undo(&mut self, label: &str, before: Arc<Document>, selection: Vec<NodeId>, page: NodeId) {
        self.history.undo.push(HistoryEntry { label: label.to_string(), doc: before, selection, page });
        if self.history.undo.len() > self.history.limit.max(1) {
            self.history.undo.remove(0);
        }
        self.history.redo.clear();
    }

    pub fn is_dirty(&self) -> bool {
        !Arc::ptr_eq(self.document_itself(), &self.saved_doc) || self.versions_changed
    }

    /// Show `version` in place of the document (or switch to it from another preview).
    pub fn start_preview(&mut self, id: &str, name: &str, time: u64, doc: Arc<Document>) {
        self.end_preview();
        self.preview = Some(VersionPreview {
            id: id.to_string(),
            name: name.to_string(),
            time,
            doc: std::mem::replace(&mut self.doc, doc),
            selection: std::mem::take(&mut self.selection),
            page: self.page,
        });
        self.page = Some(self.page)
            .filter(|pg| self.doc.pages.contains(pg))
            .or_else(|| self.doc.current_page.filter(|pg| self.doc.pages.contains(pg)))
            .or_else(|| self.doc.first_page())
            .unwrap_or(self.page);
        self.text_edit = None;
        self.vector_edit = None;
        self.interaction = None;
        self.revision += 1;
    }

    /// End a version preview, putting the document, selection and page back. False when none.
    pub fn end_preview(&mut self) -> bool {
        let Some(p) = self.preview.take() else { return false };
        self.doc = p.doc;
        self.selection = p.selection;
        self.page = p.page;
        self.text_edit = None;
        self.vector_edit = None;
        self.interaction = None;
        self.clean_selection();
        self.revision += 1;
        true
    }

    /// Shown as having unsaved changes until it is saved (a document recovered after a crash).
    pub fn unsaved(mut self) -> Self {
        self.saved_doc = Arc::new(Document::new(self.doc.name.clone()));
        self
    }

    pub fn mark_saved(&mut self) {
        self.saved_doc = self.doc.clone();
        self.versions_changed = false;
    }

    pub fn doc_mut(&mut self) -> &mut Document {
        Arc::make_mut(&mut self.doc)
    }

    /// Fit the current page's content (or centre on the origin when empty).
    pub fn zoom_to_fit(&mut self) {
        match self.doc.page_content_bounds(self.page) {
            Some(b) => self.view.fit(b, 1.0),
            None => {
                self.view.zoom = 1.0;
                self.view.cx = self.view.vw / 2.0 - 100.0;
                self.view.cy = self.view.vh / 2.0 - 100.0;
            }
        }
    }

    /// The layer the focus view shows, while it exists on the current page.
    pub fn focused(&self) -> Option<NodeId> {
        self.focus.filter(|f| self.doc.page_of(*f) == Some(self.page))
    }

    /// Drop selected ids that no longer exist or aren't on the current page.
    pub fn clean_selection(&mut self) {
        let doc = self.doc.clone();
        let page = self.page;
        self.selection.retain(|id| doc.contains(*id) && doc.page_of(*id) == Some(page) && *id != page);
        let mut seen = BTreeSet::new();
        self.selection.retain(|id| seen.insert(*id));
        if let Some(te) = &self.text_edit
            && !doc.contains(te.id)
        {
            self.text_edit = None;
        }
        // Vector edit mode ends when its layer is gone (or no longer a vector, e.g. undone).
        if self.vector_edit.as_mut().is_some_and(|ve| doc.page_of(ve.id) != Some(page) || !ve.clamp_doc(&doc)) {
            self.vector_edit = None;
        }
        // A moved rotation origin lasts as long as its selection.
        if self.rotation_origin.as_ref().is_some_and(|o| o.ids != self.selection) {
            self.rotation_origin = None;
        }
        // So does the last duplicate (undo reuses ids: a new layer could take a copy's).
        if self.last_duplicate.as_ref().is_some_and(|d| !d.is_selection(&self.selection)) {
            self.last_duplicate = None;
        }
    }
}

/// The smallest and largest interface size, relative to the system's display scale (1.0 is the
/// system's own size).
pub const UI_SCALE_MIN: f64 = 0.75;
pub const UI_SCALE_MAX: f64 = 3.0;

/// `scale` kept between [`UI_SCALE_MIN`] and [`UI_SCALE_MAX`]; not a finite number: `None`.
pub fn clamp_ui_scale(scale: f64) -> Option<f64> {
    scale.is_finite().then(|| scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX))
}

/// Reads the saved interface size. Anything but a number reads as 1 (the system's size) and a
/// number out of range is clamped, so a settings file edited by hand never costs the other
/// preferences.
fn read_ui_scale<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<f64, D::Error> {
    Ok(Value::deserialize(d)?.as_f64().and_then(clamp_ui_scale).unwrap_or(1.0))
}

/// App-wide preferences that commands read (nudge amounts, behaviours).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub nudge_small: f64,
    pub nudge_big: f64,
    pub keep_tool: bool,
    pub rename_duplicates: bool,
    pub highlight_on_hover: bool,
    pub number_keys_opacity: bool,
    pub show_dimensions: bool,
    /// "system" by default; an explicit "light" or "dark" choice stays saved.
    pub theme: String,
    /// The widths of the main window's left and right panels (the interface keeps them within its
    /// limits; other windows have their own while they are open).
    pub panel_widths: (f32, f32),
    /// The height asked for the main window's pages list, dragged at its lower edge (`None`: a
    /// share of the panel; the interface keeps it within its limits).
    pub pages_height: Option<f32>,
    /// The interface language: a language code ("en", "es", "pt-BR") or "system" (the OS or browser
    /// language). Languages without a catalog show English.
    pub language: String,
    /// The name comments are signed with (and whose comments count as yours).
    pub user_name: String,
    /// Reduce motion: the interface's own animations are off (the system's setting turns them off
    /// too). Prototype transitions still play.
    pub reduce_motion: bool,
    /// How large the whole interface is drawn, relative to the system's display scale: 1.0 is the
    /// system's own size, 1.25 a quarter larger (kept between [`UI_SCALE_MIN`] and
    /// [`UI_SCALE_MAX`]). The canvas zoom is separate.
    #[serde(deserialize_with = "read_ui_scale")]
    pub ui_scale: f64,
    /// Files that have a path are saved by themselves a few seconds after they change (the app's
    /// timer; off: only File ▸ Save writes them).
    pub auto_save: bool,
    /// The desktop app starts on the Home screen instead of in a new file.
    pub start_on_home: bool,
    /// `figcraft://` invite links open this app (`collab.registerLinks`); `None`: on for installed
    /// builds, off for development builds.
    pub invite_links: Option<bool>,
    /// Installed plugins (Plugins menu), in menu order.
    #[serde(serialize_with = "plugins::persisted")]
    pub plugins: Vec<plugins::InstalledPlugin>,
    /// Plugins' own settings (`setting_set`), by plugin id, then key: JSON values, capped per
    /// plugin.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub plugin_settings: BTreeMap<String, BTreeMap<String, Value>>,
    /// The plugin directory browsed last (`ws://host:port/`, never with a key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_directory: Option<String>,
    /// The server files are shared through online (Share ▸ Online, join codes; Publish to server
    /// suggests it too): `wss://host/`, never with a key. [`live::DEFAULT_SERVER`] unless the user
    /// chose another; empty: online sharing is off (saved as empty, so it stays off).
    pub publish_server: String,
    /// Recently opened or saved files, newest first (File ▸ Open recent; at most
    /// [`cmd::file::MAX_RECENT`]).
    pub recent_files: Vec<String>,
    /// The files you put on a server without its key, with their owner tokens (passwords: see
    /// [`online`]), most recently opened first, at most [`online::MAX_ONLINE_FILES`].
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "online::read")]
    pub online_files: Vec<online::OnlineFile>,
    /// The Getting started tips on the home screen were closed (Help ▸ Getting started shows them
    /// again).
    pub getting_started_dismissed: bool,
    /// A prototype's Open link goes to a website without first telling the viewer that it leaves
    /// the prototype (the notice's "Don't show this again"; `proto.linkNotice` shows it again).
    pub link_notice_skipped: bool,
    /// How each home screen section shows its files, by section id (`"recents"`): `"grid"` (cards
    /// with the picture each file carries) or `"list"` (a table). Missing: the grid.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub home_views: BTreeMap<String, String>,
    /// What the home screen's recent files are ordered by: `"modified"`, `"name"` or `"created"`.
    pub home_sort: String,
    /// Which way: `"desc"` (newest first, Z to A) or `"asc"`.
    pub home_order: String,
    /// The View menu's toggles turned on or off (rulers, pixel grid, snapping…), by command id:
    /// files opened or made later start with them. Missing: the toggle's default.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub view: BTreeMap<String, bool>,
    /// Recently run actions (the Actions palette's Recents), newest first, at most
    /// [`MAX_RECENT_ACTIONS`]. Entries are `command:<id>`, `plugin:<plugin>/<command>` or
    /// `widget:<plugin>`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_actions: Vec<String>,
}

/// Recently run actions kept for the Actions palette (newest first).
pub const MAX_RECENT_ACTIONS: usize = 5;

impl Default for Prefs {
    fn default() -> Self {
        Self {
            nudge_small: 1.0,
            nudge_big: 10.0,
            keep_tool: false,
            rename_duplicates: false,
            highlight_on_hover: true,
            number_keys_opacity: true,
            show_dimensions: true,
            theme: "system".into(),
            panel_widths: (240.0, 240.0),
            pages_height: None,
            language: "system".into(),
            user_name: cmd::comment::DEFAULT_AUTHOR.into(),
            reduce_motion: false,
            ui_scale: 1.0,
            auto_save: false,
            start_on_home: false,
            invite_links: None,
            plugins: Vec::new(),
            plugin_settings: BTreeMap::new(),
            plugin_directory: None,
            publish_server: live::DEFAULT_SERVER.into(),
            recent_files: Vec::new(),
            online_files: Vec::new(),
            getting_started_dismissed: false,
            link_notice_skipped: false,
            home_views: BTreeMap::new(),
            home_sort: "modified".into(),
            home_order: "desc".into(),
            view: BTreeMap::new(),
            recent_actions: Vec::new(),
        }
    }
}

/// A request from a command to the UI (open a dialog or panel, show a message). Programmatic
/// callers ignore them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UiRequest {
    pub kind: String,
    pub data: Value,
}

/// Clipboard contents (internal; the UI mirrors to the system clipboard).
#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    /// Copied subtrees: (root, nodes) in z-order.
    pub nodes: Vec<(NodeId, Vec<figcraft_doc::Node>)>,
    pub images: std::collections::BTreeMap<String, figcraft_doc::ImageData>,
    /// Copied properties (Copy properties / Paste properties).
    pub props: Option<Value>,
    /// A copied fill or stroke row (`fill.copy`, `stroke.copy`): whether it's a fill, and the paint.
    pub paint: Option<(bool, figcraft_color::Paint)>,
    /// The stroke settings (`weight`, alignment, sides…) copied with a stroke's paint.
    pub stroke: Option<figcraft_doc::StrokeProps>,
    /// A copied effect row (`effect.copy`).
    pub effect: Option<figcraft_doc::Effect>,
    /// The page-space bounds of the copied selection.
    pub bounds: Option<figcraft_geom::Rect>,
    pub source_doc: u64,
    /// Copied component/set roots, excluding cuts and originals discarded by undo (ids are reused).
    pub copied_components: std::collections::BTreeSet<NodeId>,
    /// Monotonic generation of the copied layers (contents, images, bounds and source). A UI
    /// clipboard mirror records it, so a later programmatic copy that never touched the system
    /// clipboard is not mistaken for the user's own copy just because the text still matches.
    pub layers_generation: u64,
    /// The document copied from (pastes into other files follow its mains) and its file path.
    pub source: Option<Arc<Document>>,
    pub source_path: Option<String>,
}

/// The editing session: open documents, the current tool, clipboard and prefs.
pub struct Session {
    pub docs: Vec<DocState>,
    pub active: usize,
    pub clipboard: Clipboard,
    pub prefs: Prefs,
    pub tool: tools::ToolState,
    pub ui_requests: Vec<UiRequest>,
    /// The interface's theme once the UI knows it ([`Session::set_ui_dark`]): new files get the
    /// dark page colour while it is dark (added pages take the current page's). Headless sessions (CLI, MCP, exports) have no
    /// theme, so what scripts make is the same everywhere.
    pub ui_dark: Option<bool>,
    /// Command log (id + params) for actions/automation. File imports that carry the file bytes
    /// (`file.import` with `data`) are not logged, so the log never holds a copy of a pasted file.
    pub journal: Vec<(String, Value)>,
    /// Nesting depth of [`Session::grouped`]; tabs closed while it is above zero are kept in
    /// `closed_in_group` so a failed group can bring them back.
    group_depth: usize,
    closed_in_group: Vec<DocState>,
    /// Recently used colours.
    pub recent_colors: Vec<figcraft_color::Color>,
    /// The last undoable command executed through [`Session::execute`], excluding "Repeat".
    pub last_command: Option<(String, Value)>,
    /// Status / toast message for the UI.
    pub message: Option<String>,
    /// Library files read for the open documents.
    pub libraries: library::Cache,
    /// Plugin modules loaded in this session and the last plugin run.
    pub plugins: plugins::Runtime,
    /// The live sessions (collaboration), one per document at most: status, peers, presence.
    /// [`Session::live`] is the active document's.
    pub live: live::Lives,
    /// Commands the UI runs without waiting on a server ([`Session::start_background`]).
    pub background: background::Background,
    /// The home screen's folders and favourites (`projects.*`); files opened or saved join it.
    pub projects: projects::Projects,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a command, or a batch wrapping one, imports a file passed as Base64 `data`.
fn command_carries_import_bytes(id: &str, params: &Value) -> bool {
    match id {
        "file.import" => params::get(params, "data").is_some(),
        "command.batch" => params.get("commands").and_then(Value::as_array).is_some_and(|commands| {
            commands.iter().any(|c| {
                c.get("command").and_then(Value::as_str).zip(c.get("params")).is_some_and(|(id, params)| command_carries_import_bytes(id, params))
            })
        }),
        _ => false,
    }
}

impl Session {
    /// A session with one new document.
    pub fn new() -> Self {
        let mut s = Self::empty();
        s.docs.push(DocState::new(Document::new("Untitled"), None));
        s
    }

    /// A session with no documents (the home screen).
    pub fn empty() -> Self {
        Self {
            docs: Vec::new(),
            active: 0,
            clipboard: Clipboard::default(),
            prefs: Prefs::default(),
            tool: tools::ToolState::default(),
            ui_requests: Vec::new(),
            ui_dark: None,
            journal: Vec::new(),
            group_depth: 0,
            closed_in_group: Vec::new(),
            recent_colors: Vec::new(),
            last_command: None,
            message: None,
            libraries: library::Cache::default(),
            plugins: plugins::Runtime::default(),
            live: live::Lives::default(),
            background: background::Background::default(),
            projects: projects::Projects::default(),
        }
    }

    /// The background a new file's page gets: the dark page colour while the interface is dark.
    pub fn new_page_background(&self) -> figcraft_color::Color {
        if self.ui_dark == Some(true) { figcraft_color::Color::DARK_PAGE } else { figcraft_color::Color::DEFAULT_PAGE }
    }

    /// Tell the engine the interface's theme (the UI does, every frame). The first time, a new
    /// file nothing was done to yet (the one the app opened with, made before the theme was
    /// known) takes the theme's page colour; later changes of theme leave existing pages alone.
    pub fn set_ui_dark(&mut self, dark: bool) {
        if self.ui_dark.replace(dark).is_some() || !dark {
            return;
        }
        for ds in &mut self.docs {
            let untouched = ds.path.is_none() && ds.history.undo.is_empty() && ds.history.redo.is_empty() && !ds.is_dirty() && ds.versions.is_empty();
            let pages = ds.doc.pages.clone();
            let blank = pages.iter().all(|p| {
                ds.doc.node(*p).is_some_and(|n| {
                    n.children.is_empty()
                        && matches!(n.kind, figcraft_doc::NodeKind::Page { background, .. } if background == figcraft_color::Color::DEFAULT_PAGE)
                })
            });
            if !(untouched && blank) {
                continue;
            }
            let mut doc = (*ds.doc).clone();
            for p in pages {
                if let Some(figcraft_doc::NodeKind::Page { background, .. }) = doc.node_mut(p).map(|n| &mut n.kind) {
                    *background = figcraft_color::Color::DARK_PAGE;
                }
            }
            // Still a file without changes.
            ds.doc = Arc::new(doc);
            ds.saved_doc = ds.doc.clone();
            ds.revision += 1;
        }
    }

    pub fn with_document(doc: Document) -> Self {
        let mut s = Self::empty();
        s.docs.push(DocState::new(doc, None));
        s
    }

    /// Files new to the session start with the View toggles the preferences keep.
    pub fn apply_view_prefs(&mut self) {
        for ds in self.docs.iter_mut().filter(|d| !d.view_from_prefs) {
            cmd::view::apply_kept(&self.prefs, &mut ds.view);
            ds.view_from_prefs = true;
        }
    }

    pub fn ds(&self) -> Result<&DocState> {
        self.docs.get(self.active).ok_or(EngineError::NoDocument)
    }

    pub fn ds_mut(&mut self) -> Result<&mut DocState> {
        self.docs.get_mut(self.active).ok_or(EngineError::NoDocument)
    }

    pub fn doc(&self) -> Result<&Document> {
        Ok(self.ds()?.doc.as_ref())
    }

    pub fn doc_mut(&mut self) -> Result<&mut Document> {
        Ok(self.ds_mut()?.doc_mut())
    }

    pub fn selection(&self) -> Vec<NodeId> {
        self.ds().map(|d| d.selection.clone()).unwrap_or_default()
    }

    pub fn set_selection(&mut self, ids: Vec<NodeId>) {
        if let Ok(ds) = self.ds_mut() {
            ds.selection = ids;
            ds.clean_selection();
            ds.revision += 1;
        }
    }

    pub fn page(&self) -> Result<NodeId> {
        Ok(self.ds()?.page)
    }

    /// Run a command by id. Mutations become one undo step; a failure or panic leaves the document
    /// as it was.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        board::refuse_on_board(self, id)?;
        // Revert replaces the document's uid: refuse before ending previews or syncing branches.
        if id == "file.revert" && self.live().doc_uid.is_some() {
            return Err(EngineError::Disabled(id.into(), "leave the live session before reverting to the saved file".into()));
        }
        branches::sync(self);
        self.apply_view_prefs();
        if textpreview::COMMANDS.contains(&id) {
            // A live preview is no edit: no undo step, normalisation or rollback here.
            let r = guard::run(id, || (spec.run)(self, params));
            if r.is_err() {
                textpreview::end(self);
            }
            return r;
        }
        // Every other command acts on the document itself, not on a live text preview.
        textpreview::end(self);
        if ENDS_PREVIEW.contains(&id)
            && (!matches!(id, "file.save" | "file.saveAs") || cmd::file::save_ends_preview(self, id, params))
            && let Ok(ds) = self.ds_mut()
        {
            ds.end_preview();
        }
        let before = self.ds().ok().map(|d| (d.doc.clone(), d.selection.clone(), d.page, d.uid));
        // Cut and copy write the clipboard before a later check can refuse the command.
        let clipboard_before = self.clipboard.clone();
        let result = guard::run(id, || (spec.run)(self, params))
            .and_then(|v| self.check_instance_structure(id, &before).map(|()| v))
            .and_then(|v| self.check_preview(id, &before).map(|()| v))
            .and_then(|v| self.check_live_access(id, &before).map(|()| v))
            .and_then(|v| self.check_finite_geometry(id, &before).map(|()| v));
        match &result {
            Ok(_) => {
                self.after_command(spec, before);
                self.apply_view_prefs();
                if spec.history && id != "edit.repeat" {
                    self.last_command = Some((id.to_string(), params.clone()));
                }
                // Imported bytes already live in the document: a Base64 copy in the log would stay
                // for the whole session.
                if !command_carries_import_bytes(id, params) {
                    self.journal.push((id.to_string(), params.clone()));
                    if self.journal.len() > 5000 {
                        self.journal.drain(..1000);
                    }
                }
            }
            Err(_) => {
                // Roll back any partial edit — also during an interaction, where `before` is the
                // interaction's last valid preview: restore it and leave the gesture itself (its
                // own snapshot) alone.
                if let Some((doc, sel, page, uid)) = before
                    && let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid)
                {
                    ds.doc = doc;
                    ds.selection = sel;
                    ds.page = page;
                }
                self.clipboard = clipboard_before;
            }
        }
        result
    }

    /// A version shown in place of the document is read-only: refuse an edit (it is rolled back).
    fn check_preview(&self, id: &str, before: &Option<(Arc<Document>, Vec<NodeId>, NodeId, u64)>) -> Result<()> {
        let Some((bdoc, _, _, uid)) = before else { return Ok(()) };
        let Some(ds) = self.docs.iter().find(|d| d.uid == *uid) else { return Ok(()) };
        if ds.preview.is_none() || Arc::ptr_eq(&ds.doc, bdoc) || id == "version.preview" {
            return Ok(());
        }
        Err(EngineError::Disabled(id.to_string(), PREVIEW_READ_ONLY.into()))
    }

    /// A layer's position and size stay finite: infinity and NaN are written as `null`, so a file
    /// holding them saves but can't be reopened. Refuse such an edit (it is rolled back).
    fn check_finite_geometry(&self, id: &str, before: &Option<(Arc<Document>, Vec<NodeId>, NodeId, u64)>) -> Result<()> {
        let Some((bdoc, _, _, uid)) = before else { return Ok(()) };
        let Some(ds) = self.docs.iter().find(|d| d.uid == *uid) else { return Ok(()) };
        if Arc::ptr_eq(&ds.doc, bdoc) {
            return Ok(());
        }
        let broken = |n: &Node| !(n.transform.as_coeffs().iter().all(|c| c.is_finite()) && n.width.is_finite() && n.height.is_finite());
        let untouched = |n: &Node| bdoc.node_arc(n.id).zip(ds.doc.node_arc(n.id)).is_some_and(|(a, b)| Arc::ptr_eq(a, b));
        match ds.doc.nodes().find(|n| broken(n) && !untouched(n)) {
            Some(n) => Err(bad(id, format!("\"{}\" would get a position or size beyond the number range", n.name))),
            None => Ok(()),
        }
    }

    /// A live session's document you may only view (or that is on its way to a server) is
    /// read-only: refuse an edit (it is rolled back).
    fn check_live_access(&self, id: &str, before: &Option<(Arc<Document>, Vec<NodeId>, NodeId, u64)>) -> Result<()> {
        let Some((bdoc, _, _, uid)) = before else { return Ok(()) };
        let Some(ds) = self.docs.iter().find(|d| d.uid == *uid) else { return Ok(()) };
        match self.live_read_only(*uid) {
            Some(why) if !Arc::ptr_eq(&ds.doc, bdoc) => Err(EngineError::Disabled(id.to_string(), why.into())),
            _ => Ok(()),
        }
    }

    /// Instances can't gain, lose or reorder layers, nor move, resize or re-constrain the layers in
    /// them, and library copies are read-only: refuse such an edit (it is rolled back).
    fn check_instance_structure(&self, id: &str, before: &Option<(Arc<Document>, Vec<NodeId>, NodeId, u64)>) -> Result<()> {
        let Some((bdoc, _, _, uid)) = before else { return Ok(()) };
        let Some(ds) = self.docs.iter().find(|d| d.uid == *uid) else { return Ok(()) };
        if Arc::ptr_eq(&ds.doc, bdoc) {
            return Ok(());
        }
        // "Repeat" runs the repeated command.
        let effective = match (&self.last_command, id) {
            (Some((last, _)), "edit.repeat") => last.as_str(),
            _ => id,
        };
        library::check_read_only(effective, bdoc, &ds.doc)
            .and_then(|()| components::check_structure_edit(bdoc, &ds.doc))
            .and_then(|()| components::check_override_edit(bdoc, &ds.doc))
            .map_err(|m| EngineError::Disabled(id.to_string(), m))
    }

    /// Bookkeeping after a successful command: normalise what changed, push undo, bump revision.
    fn after_command(&mut self, spec: &CommandSpec, before: Option<(Arc<Document>, Vec<NodeId>, NodeId, u64)>) {
        let Some((bdoc, bsel, bpage, uid)) = before else {
            if let Ok(ds) = self.ds_mut() {
                ds.revision += 1;
            }
            return;
        };
        let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid) else { return };
        let changed = !Arc::ptr_eq(&ds.doc, &bdoc);
        // Saving may adopt the chosen filename as the document title. The encoded snapshot is
        // already final; normalising it again would detach it from the saved baseline.
        if changed && !matches!(spec.id, "file.save" | "file.saveAs") {
            normalize(ds.doc_mut(), &bdoc);
        }
        let edit = changed && ds.interaction.is_none();
        ds.clean_selection();
        ds.revision += 1;
        if edit {
            self.record(uid, spec.history.then_some(spec.label), bdoc, bsel, bpage);
        }
    }

    /// Record an edit of document `uid` (it was `before`, shown with `selection` on `page`) as one
    /// undo step called `label` (`None`: not undoable). On a live session's document the edit is
    /// synced and the step is the user's own ([`live`]); otherwise it is a snapshot of `before`.
    pub(crate) fn record(&mut self, uid: u64, label: Option<&str>, before: Arc<Document>, selection: Vec<NodeId>, page: NodeId) {
        if self.live_doc(uid) {
            return self.live_edit(uid, label);
        }
        if let (Some(label), Some(ds)) = (label, self.docs.iter_mut().find(|d| d.uid == uid)) {
            ds.push_undo(label, before, selection, page);
        }
    }

    /// How many undo and redo steps the active document has (the user's own in a live session).
    pub fn undo_depth(&self) -> (usize, usize) {
        match self.ds() {
            Ok(ds) if self.live_doc(ds.uid) => self.live().client().map_or((0, 0), figcraft_collab::Client::history_len),
            Ok(ds) => (ds.history.undo.len(), ds.history.redo.len()),
            Err(_) => (0, 0),
        }
    }

    /// The labels of the active document's last `n` undo steps, the next one first.
    pub fn undo_labels(&self, n: usize) -> Vec<String> {
        match self.ds() {
            Ok(ds) if self.live_doc(ds.uid) => {
                self.live().client().map(|c| c.undo_labels().rev().take(n).map(str::to_string).collect()).unwrap_or_default()
            }
            Ok(ds) => ds.history.undo.iter().rev().take(n).map(|h| h.label.clone()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Start a gesture: later [`preview`](Self::preview) calls re-apply edits to this snapshot.
    pub fn begin_interaction(&mut self, label: &str) -> Result<()> {
        textpreview::end(self);
        if let Some(why) = self.active_read_only() {
            return Err(EngineError::Disabled(label.to_string(), why.into()));
        }
        let ds = self.ds_mut()?;
        if ds.preview.is_some() {
            return Err(EngineError::Disabled(label.to_string(), PREVIEW_READ_ONLY.into()));
        }
        if ds.interaction.is_none() {
            ds.interaction = Some(Interaction { label: label.to_string(), doc: ds.doc.clone(), selection: ds.selection.clone() });
        }
        Ok(())
    }

    /// Replace the document with `f(snapshot)` for a live preview during a gesture.
    pub fn preview(&mut self, f: impl FnOnce(&mut Document, &mut Vec<NodeId>) -> Result<()>) -> Result<()> {
        let ds = self.ds_mut()?;
        let Some(it) = ds.interaction.clone() else { return Err(EngineError::Other("no gesture in progress".into())) };
        let mut doc = (*it.doc).clone();
        let mut sel = it.selection.clone();
        // What the gesture changes on the selected layers is an explicit edit.
        let before = cmd::styles::bound_before(&doc, &it.selection);
        let res = f(&mut doc, &mut sel);
        if res.is_ok() {
            cmd::styles::detach_edited(&mut doc, &before);
            normalize(&mut doc, &it.doc);
            // The same preview again (a handle held still) keeps the document: a new one would
            // re-render the canvas for nothing.
            if doc == *ds.doc && sel == ds.selection {
                return res;
            }
            ds.doc = Arc::new(doc);
            ds.selection = sel;
            ds.clean_selection();
            ds.revision += 1;
        }
        res
    }

    /// Finish a gesture as one undo step (no step if nothing changed).
    pub fn commit_interaction(&mut self) -> Result<()> {
        let ds = self.ds_mut()?;
        let Some(it) = ds.interaction.take() else { return Ok(()) };
        let (uid, page) = (ds.uid, ds.page);
        ds.revision += 1;
        if !Arc::ptr_eq(&it.doc, &ds.doc) && *it.doc != *ds.doc {
            self.record(uid, Some(&it.label), it.doc, it.selection, page);
        } else {
            ds.doc = it.doc;
        }
        Ok(())
    }

    /// Abandon a gesture, restoring the document.
    pub fn cancel_interaction(&mut self) -> Result<()> {
        let ds = self.ds_mut()?;
        if let Some(it) = ds.interaction.take() {
            ds.doc = it.doc;
            ds.selection = it.selection;
            ds.revision += 1;
        }
        Ok(())
    }

    pub fn undo(&mut self) -> Result<bool> {
        textpreview::end(self);
        let live = self.ds().is_ok_and(|d| self.live_doc(d.uid));
        let ds = self.ds_mut()?;
        if ds.preview.is_some() {
            return Err(EngineError::Disabled("edit.undo".into(), PREVIEW_READ_ONLY.into()));
        }
        if let Some(it) = ds.interaction.take()
            && live
        {
            // The gesture in progress is dropped (it was never synced).
            ds.doc = it.doc;
        }
        ds.text_edit = None;
        if live {
            return Ok(self.live_travel(true));
        }
        let Some(e) = ds.history.undo.pop() else { return Ok(false) };
        let redo = HistoryEntry { label: e.label.clone(), doc: ds.doc.clone(), selection: ds.selection.clone(), page: ds.page };
        ds.doc = e.doc;
        cmd::facilitate::keep(&redo.doc, &mut ds.doc);
        ds.history.redo.push(redo);
        ds.selection = e.selection;
        if ds.doc.pages.contains(&e.page) {
            ds.page = e.page;
        }
        ds.clean_selection();
        ds.revision += 1;
        let (uid, doc) = (ds.uid, ds.doc.clone());
        if self.clipboard.source_doc == uid {
            self.clipboard.copied_components.retain(|id| {
                doc.node(*id)
                    .is_some_and(|n| matches!(n.kind, figcraft_doc::NodeKind::Component { .. } | figcraft_doc::NodeKind::ComponentSet { .. }))
            });
        }
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool> {
        textpreview::end(self);
        let live = self.ds().is_ok_and(|d| self.live_doc(d.uid));
        let ds = self.ds_mut()?;
        if ds.preview.is_some() {
            return Err(EngineError::Disabled("edit.redo".into(), PREVIEW_READ_ONLY.into()));
        }
        if let Some(it) = ds.interaction.take()
            && live
        {
            ds.doc = it.doc;
        }
        ds.text_edit = None;
        if live {
            return Ok(self.live_travel(false));
        }
        let Some(e) = ds.history.redo.pop() else { return Ok(false) };
        let undo = HistoryEntry { label: e.label.clone(), doc: ds.doc.clone(), selection: ds.selection.clone(), page: ds.page };
        ds.doc = e.doc;
        cmd::facilitate::keep(&undo.doc, &mut ds.doc);
        ds.history.undo.push(undo);
        ds.selection = e.selection;
        if ds.doc.pages.contains(&e.page) {
            ds.page = e.page;
        }
        ds.clean_selection();
        ds.revision += 1;
        Ok(true)
    }

    /// Queue a UI request (dialogs etc.).
    pub fn request_ui(&mut self, kind: &str, data: Value) {
        self.ui_requests.push(UiRequest { kind: kind.to_string(), data });
    }

    /// Run several commands as one undo step. A param value `{"$step": n}` is replaced by the `id`
    /// in the result of the earlier step `n`, and `{"$ref": "name"}` by the `id` in the result of
    /// the earlier step whose `as` is `name`; either takes a `field`, a key or slash path (`ids/0`)
    /// into that result. So a step can target a layer that an earlier step of the same batch
    /// created. Any object with a `$step` or `$ref` key in a step's params is such a reference,
    /// except inside a nested `command.batch` step: its references are to its own steps.
    pub fn batch(&mut self, label: &str, steps: &[BatchStep]) -> Result<Vec<Value>> {
        let mut scope = BatchScope::new(steps)?;
        self.grouped(label, |s| {
            for step in steps {
                s.batch_step(&mut scope, &step.command, &step.params)?;
            }
            Ok(scope.into_results())
        })
    }

    /// The next step of a batch: the references in `params` resolve against `scope` (the steps
    /// before it), the command runs and its result joins `scope`. [`Session::batch`] runs a list
    /// of these as one undo step; a caller with its own list — the CLI's script runner — uses this
    /// so its steps resolve references the same way, inside its own [`Session::grouped`].
    pub fn batch_step(&mut self, scope: &mut BatchScope, cmd: &str, params: &Value) -> Result<()> {
        let p = if cmd == "command.batch" { params.clone() } else { scope.resolve(params)? };
        // Say which step failed; a nested batch names its own steps.
        let v = self.execute(cmd, &p).map_err(|e| if cmd == "command.batch" { e } else { e.at_step(scope.next_step()) })?;
        scope.push(v);
        Ok(())
    }

    /// Take the tabs `pred` matches out of `docs`; inside a group they are kept so that a failed
    /// group can reopen them.
    pub(crate) fn remove_tabs(&mut self, mut pred: impl FnMut(&DocState) -> bool) {
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.docs).into_iter().partition(|d| pred(d));
        self.docs = keep;
        if self.group_depth > 0 {
            self.closed_in_group.extend(gone);
        }
    }

    /// Run `f` as one undo step called `label`: the commands it executes push no steps of their
    /// own, and when it fails the documents stay at their group-start state.
    pub fn grouped<T>(&mut self, label: &str, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.ds()?;
        let before: Vec<_> =
            self.docs.iter().map(|ds| (ds.uid, ds.doc.clone(), ds.selection.clone(), ds.page, ds.history.undo.len(), ds.history.limit)).collect();
        // An edit inside clears the redo steps; a failed group has to bring them back.
        let redo_before: Vec<_> = self.docs.iter().map(|ds| ds.history.redo.clone()).collect();
        let active = self.active;
        let journal_len = self.journal.len();

        // The steps inside are dropped again below, so they mustn't push the oldest ones out.
        for ds in &mut self.docs {
            ds.history.limit = usize::MAX;
        }
        for (uid, _, _, _, _, _) in &before {
            self.live_hold(*uid, true);
        }
        self.group_depth += 1;
        let result = f(self);
        self.group_depth -= 1;
        for (uid, _, _, _, _, _) in &before {
            self.live_hold(*uid, false);
        }

        if result.is_err() {
            // Tabs the group closed come back from the stash, and the group-start order returns.
            let mut current = std::mem::take(&mut self.docs);
            for (uid, ..) in &before {
                let tab = match current.iter().position(|d| d.uid == *uid) {
                    Some(i) => Some(current.swap_remove(i)),
                    None => self.closed_in_group.iter().position(|d| d.uid == *uid).map(|i| self.closed_in_group.swap_remove(i)),
                };
                self.docs.extend(tab);
            }
            for ((uid, doc, selection, page, undo_len, limit), redo) in before.iter().zip(redo_before) {
                if let Some(ds) = self.docs.iter_mut().find(|ds| ds.uid == *uid) {
                    ds.history.redo = redo;
                    ds.doc = doc.clone();
                    ds.selection = selection.clone();
                    ds.page = *page;
                    ds.history.undo.truncate(*undo_len);
                    ds.history.limit = *limit;
                }
            }
            self.active = active;
            self.journal.truncate(journal_len);
            if self.group_depth == 0 {
                self.closed_in_group.clear();
            }
            return result;
        }
        if self.group_depth == 0 {
            self.closed_in_group.clear();
        }

        let changed: Vec<_> = before
            .iter()
            .filter_map(|(uid, doc, selection, page, _, _)| {
                let ds = self.docs.iter().find(|d| d.uid == *uid)?;
                (!Arc::ptr_eq(doc, &ds.doc) && ds.interaction.is_none()).then(|| (*uid, doc.clone(), selection.clone(), *page))
            })
            .collect();
        for ds in &mut self.docs {
            if let Some((_, _, _, _, undo_len, limit)) = before.iter().find(|before| before.0 == ds.uid) {
                ds.history.limit = *limit;
                ds.history.undo.truncate(*undo_len);
            }
        }
        for (uid, doc, selection, page) in changed {
            self.record(uid, Some(label), doc, selection, page);
        }
        result
    }

    /// A summary for status displays.
    pub fn status(&self) -> Value {
        match self.ds() {
            Ok(ds) => json!({
                "document": ds.doc.name,
                "path": ds.path,
                "dirty": ds.is_dirty(),
                "page": ds.page,
                "selection": ds.selection,
                "tool": self.tool.tool.id(),
                "zoom": ds.view.zoom,
                "undo": self.undo_depth().0,
                "redo": self.undo_depth().1,
                "documents": self.docs.len(),
            }),
            Err(_) => json!({"documents": 0}),
        }
    }
}

/// Nodes whose `Arc` differs from `before` (new or edited).
pub fn changed_nodes(doc: &Document, before: &Document) -> Vec<NodeId> {
    doc.ids()
        .filter(|id| match (doc.node_arc(*id), before.node_arc(*id)) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            _ => true,
        })
        .collect()
}

/// Keep derived state consistent after edits: text boxes resize to their content, groups wrap
/// their children, instances follow their main components, auto layout re-flows.
pub fn normalize(doc: &mut Document, before: &Document) {
    // Only what was touched since the last normalisation (edited, added or removed).
    let mut touched: BTreeSet<NodeId> = doc.take_dirty();
    // A variable or style edit changes no layer directly, but bound and linked layers must follow it.
    if touched.is_empty() && doc.variables == before.variables && doc.styles == before.styles {
        return;
    }
    let m = measure::TextMeasure;
    // 0. Variable bindings and style links feed node values.
    let scope = binding_scope(doc, before, &touched);
    if doc.variables != before.variables || doc.styles != before.styles {
        style_vars::settle(doc);
    }
    cmd::styles::apply_bindings_to(doc, scope.as_ref());
    style_vars::refresh(doc, Some(before), scope.as_ref());
    if doc.styles != before.styles {
        let changed_styles: Vec<String> = doc.styles.iter().filter(|s| !before.styles.contains(s)).map(|s| s.id.clone()).collect();
        for sid in changed_styles {
            cmd::styles::propagate_style(doc, &sid);
        }
    }
    touched.extend(doc.take_dirty());
    // Lists and text chars edited away from their style unlink from it.
    let live_now: Vec<NodeId> = touched.iter().copied().filter(|i| doc.contains(*i)).collect();
    cmd::styles::sync_list_styles(doc, before, &live_now);
    cmd::styles::sync_text_styles(doc, &live_now);
    touched.extend(doc.take_dirty());
    let live = |doc: &Document, t: &BTreeSet<NodeId>| -> Vec<NodeId> { t.iter().copied().filter(|i| doc.contains(*i)).collect() };
    // 0b. Fill and hug only where they can apply (a layer moved out of auto layout, a parent that
    // started to hug or lost auto layout): the rest becomes fixed at its current size.
    for id in live(doc, &touched) {
        figcraft_layout::coherent_sizing(doc, id);
    }
    touched.extend(doc.take_dirty());
    // 1. Text auto-size (a sticky's note first takes the size its length calls for).
    board::settle_text(doc, before, &touched);
    touched.extend(doc.take_dirty());
    for id in live(doc, &touched) {
        let Some(n) = doc.node(id) else { continue };
        if let Some((w, h)) = figcraft_text::auto_size(n)
            && ((w - n.width).abs() > 1e-6 || (h - n.height).abs() > 1e-6)
        {
            figcraft_layout::set_size(doc, id, w, h);
        }
    }
    touched.extend(doc.take_dirty());
    // 2. Instances follow edited main components (and record overrides inside instances).
    let changed = live(doc, &touched);
    components::sync_after_edit(doc, before, &changed, &touched);
    // Rebuilt copies took their mains' values: their bindings resolve in their own modes.
    let synced = doc.take_dirty();
    if !synced.is_empty() {
        cmd::styles::apply_bindings_to(doc, Some(&synced));
        style_vars::refresh(doc, Some(before), Some(&synced));
        touched.extend(synced);
    }
    touched.extend(doc.take_dirty());
    // Parents of removed nodes (as they were before) count as touched.
    let removed_parents: Vec<NodeId> =
        touched.iter().filter(|i| !doc.contains(**i)).filter_map(|i| before.node(*i).and_then(|n| n.parent)).filter(|p| doc.contains(*p)).collect();
    // 3. Groups / booleans wrap their children.
    let mut fit: BTreeSet<NodeId> = BTreeSet::new();
    for id in live(doc, &touched).into_iter().chain(removed_parents.iter().copied()) {
        if doc.node(id).is_some_and(|n| n.kind.bounds_follow_children()) {
            fit.insert(id);
        }
        for a in doc.ancestors(id) {
            if doc.node(a).is_some_and(|n| n.kind.bounds_follow_children()) {
                fit.insert(a);
            } else {
                break;
            }
        }
    }
    let mut fit: Vec<NodeId> = fit.into_iter().collect();
    fit.sort_by_key(|id| std::cmp::Reverse(doc.ancestors(*id).len()));
    for id in fit {
        if doc.node(id).is_some_and(|n| n.children.is_empty()) {
            // Empty groups disappear, like the incumbent.
            let _ = doc.remove(id);
        } else {
            let _ = doc.fit_to_children(id);
        }
    }
    touched.extend(doc.take_dirty());
    // 4. Auto layout for frames where something that sizes or places layers changed.
    let mut frames: BTreeSet<NodeId> = BTreeSet::new();
    let moved = live(doc, &touched).into_iter().filter(|id| affects_layout(before, doc, *id));
    for id in moved.chain(removed_parents) {
        if !doc.contains(id) {
            continue;
        }
        let mut chain = vec![id];
        chain.extend(doc.ancestors(id));
        for c in chain {
            if doc.node(c).is_some_and(|n| n.layout.is_some()) {
                frames.insert(c);
            }
        }
    }
    let mut frames: Vec<NodeId> = frames.into_iter().collect();
    frames.sort_by_key(|id| std::cmp::Reverse(doc.ancestors(*id).len()));
    for f in frames {
        // Layers the layout leaves where they were stay untouched.
        let kept: Vec<(Arc<Node>, bool)> = doc
            .node(f)
            .map(|n| std::iter::once(f).chain(n.children.iter().copied()).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|i| doc.node_arc(i).map(|a| (a.clone(), doc.dirty().contains(&i))))
            .collect();
        figcraft_layout::layout_frame(doc, f, &m);
        for (a, was_dirty) in kept {
            doc.keep_if_unchanged(a, was_dirty);
        }
    }
    touched.extend(doc.take_dirty());
    // 4b. Board connectors follow the layers they join; their labels sit at their middles.
    board::settle_routes(doc, before, &touched);
    touched.extend(doc.take_dirty());
    // 5. Ready-for-dev layers whose contents changed show "Changed".
    mark_dev_changes(doc, before, &touched);
    doc.take_dirty();
    // 5. Comment pins follow their layers to other pages and stay put when the layers are deleted.
    cmd::comment::follow_anchors(doc, before);
}

/// Whether layer `id` changed (since `before`) in a way that can move or resize layers in an auto
/// layout frame around it: new, resized, moved, shown or hidden, restructured, other text or
/// layout settings.
fn affects_layout(before: &Document, doc: &Document, id: NodeId) -> bool {
    let (Some(a), Some(b)) = (before.node(id), doc.node(id)) else { return true };
    a.width != b.width
        || a.height != b.height
        || a.transform != b.transform
        || a.visible != b.visible
        || a.parent != b.parent
        || a.children != b.children
        || a.layout != b.layout
        || a.child_layout != b.child_layout
        || a.stroke != b.stroke
        || a.strokes.len() != b.strokes.len()
        || a.kind != b.kind
}

/// The layers whose variable bindings an edit may have changed: every bound layer when a variable
/// changed, else the touched layers and everything below those that moved, are new, chose
/// another variable mode or became an instance of another main (modes are inherited, an
/// instance's from its main too), and the instances of a main that chose another mode.
fn binding_scope(doc: &Document, before: &Document, touched: &BTreeSet<NodeId>) -> Option<BTreeSet<NodeId>> {
    if doc.variables != before.variables {
        return None;
    }
    let main = |n: &Node| match n.kind {
        figcraft_doc::NodeKind::Instance { component, .. } => Some(component),
        _ => None,
    };
    let mut out = BTreeSet::new();
    for &id in touched {
        let Some(n) = doc.node(id) else { continue };
        let Some(b) = before.node(id) else {
            out.extend(doc.subtree(id));
            continue;
        };
        let modes = b.var_modes != n.var_modes;
        if modes || b.parent != n.parent || main(b) != main(n) {
            out.extend(doc.subtree(id));
        } else {
            out.insert(id);
        }
        if modes && matches!(n.kind, figcraft_doc::NodeKind::Component { .. }) {
            for i in components::instances_of(doc, id) {
                out.extend(doc.subtree(i));
            }
        }
    }
    Some(out)
}

/// Flag the frames and sections marked ready for dev (before this edit) that contain a layer whose
/// design `touched` changed (added, removed or edited; the handoff status itself and hiding a
/// section's contents don't count).
fn mark_dev_changes(doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) {
    let ready = |d: &Document, id: NodeId| d.node(id).is_some_and(|n| n.dev_status == figcraft_doc::DevStatus::ReadyForDev);
    let mut changed: BTreeSet<NodeId> = BTreeSet::new();
    for &id in touched {
        // Ready-for-dev frames around it first (the cheap test).
        let mut chain = vec![id];
        chain.extend(if doc.contains(id) { doc.ancestors(id) } else { before.ancestors(id) });
        chain.retain(|m| ready(before, *m) && ready(doc, *m));
        if chain.is_empty() {
            continue;
        }
        let differs = match (before.node(id), doc.node(id)) {
            (Some(a), Some(b)) => !a.same_design(b),
            (None, None) => false,
            _ => true,
        };
        if differs {
            changed.extend(chain);
        }
    }
    for m in changed {
        if doc.node(m).is_some_and(|n| !n.dev_changed) {
            // Its first change since it was marked: keep what it looked like (for Compare changes).
            if let Some(snap) = before.snapshot_subtree(m) {
                doc.dev_snapshots.insert(m, Arc::new(snap));
            }
            if let Some(n) = doc.node_mut(m) {
                n.dev_changed = true;
            }
        }
    }
    // Snapshots last until the change is resolved, the layer is marked again or it's deleted.
    if !doc.dev_snapshots.is_empty() {
        let keep = |d: &Document, id: NodeId| d.node(id).is_some_and(|n| n.dev_status == figcraft_doc::DevStatus::ReadyForDev && n.dev_changed);
        let stale: Vec<NodeId> = doc.dev_snapshots.keys().copied().filter(|id| !keep(doc, *id)).collect();
        for id in stale {
            doc.dev_snapshots.remove(&id);
        }
    }
}

#[cfg(test)]
mod a11y_keys_tests;
#[cfg(test)]
mod al_behaviour_tests;
#[cfg(test)]
mod al_content_align_tests;
#[cfg(test)]
mod al_drag_tests;
#[cfg(test)]
mod al_keys_tests;
#[cfg(test)]
mod al_nesting_tests;
#[cfg(test)]
mod annotation_categories_tests;
#[cfg(test)]
mod annotations_tests;
#[cfg(test)]
mod arc_fields_tests;
#[cfg(test)]
mod arc_handles_tests;
#[cfg(test)]
mod autosave_tests;
#[cfg(test)]
mod batch_rollback_tests;
#[cfg(test)]
mod batch_step_ref_tests;
#[cfg(test)]
mod binding_lists_tests;
#[cfg(test)]
mod board_connector_tests;
#[cfg(test)]
mod board_tables_tests;
#[cfg(test)]
mod board_tests;
#[cfg(test)]
mod board_tools_tests;
#[cfg(test)]
mod booleans_tests;
#[cfg(test)]
mod bound_alpha_tests;
#[cfg(test)]
mod branch_live_tests;
#[cfg(test)]
mod branch_tests;
#[cfg(test)]
mod branch_versions_tests;
#[cfg(test)]
mod brush_tests;
#[cfg(test)]
mod canvas_fixes_tests;
#[cfg(test)]
mod code_connect_tests;
#[cfg(test)]
mod collab_ids_tests;
#[cfg(test)]
mod collab_live_tests;
#[cfg(test)]
mod collab_tests;
#[cfg(test)]
mod collab_undo_tests;
#[cfg(test)]
mod comments_polish_tests;
#[cfg(test)]
mod component_codegen_tests;
#[cfg(test)]
mod components_behaviour_tests;
#[cfg(test)]
mod components_scale_tests;
#[cfg(test)]
mod convert_container_tests;
#[cfg(test)]
mod cursor_tests;
#[cfg(test)]
mod dark_page_text_tests;
#[cfg(test)]
mod datafill_tests;
#[cfg(test)]
mod design_paste_tests;
#[cfg(test)]
mod dev_measurements_tests;
#[cfg(test)]
mod dev_resources_tests;
#[cfg(test)]
mod devcompare_tests;
#[cfg(test)]
mod devfocus_tests;
#[cfg(test)]
mod devinspect_tests;
#[cfg(test)]
mod devmode_behaviour_tests;
#[cfg(test)]
mod devmode_tail_tests;
#[cfg(test)]
mod doc_urls_tests;
#[cfg(test)]
mod double_click_tests;
#[cfg(test)]
mod draw_tools_tests;
#[cfg(test)]
mod duplicate_spacing_tests;
#[cfg(test)]
mod effects_tests;
#[cfg(test)]
mod export_behaviour_tests;
#[cfg(test)]
mod export_depth_tests;
#[cfg(test)]
mod extended_chain_tests;
#[cfg(test)]
mod facilitate_tests;
#[cfg(test)]
mod fig_export_assets_tests;
#[cfg(test)]
mod fig_export_tests;
#[cfg(test)]
mod fig_import_tests;
#[cfg(test)]
mod fill_none_tests;
#[cfg(test)]
mod finite_geometry_tests;
#[cfg(test)]
mod frame_guides_tests;
#[cfg(test)]
mod frame_label_tests;
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod frames_behaviour_tests;
#[cfg(test)]
mod fx_behaviour_tests;
#[cfg(test)]
mod gesture_preview_tests;
#[cfg(test)]
mod gradient_shorthand_tests;
#[cfg(test)]
mod hide_publishing_tests;
#[cfg(test)]
mod history_behaviour_tests;
#[cfg(test)]
mod html_codegen_tests;
#[cfg(test)]
mod huge_stroke_tests;
#[cfg(test)]
mod id_params_behaviour_tests;
#[cfg(test)]
mod images_left_in_file_tests;
#[cfg(test)]
mod import_export_tests;
#[cfg(test)]
mod import_journal_tests;
#[cfg(test)]
mod inspect_behaviour_tests;
#[cfg(test)]
mod instance_modes_tests;
#[cfg(test)]
mod instance_resizing_tests;
#[cfg(test)]
mod interaction_rollback_tests;
#[cfg(test)]
mod invite_links_tests;
#[cfg(test)]
mod keymap_tests;
#[cfg(test)]
mod library_path_tests;
#[cfg(test)]
mod library_range_styles_tests;
#[cfg(test)]
mod library_variables_tests;
#[cfg(test)]
mod line_endings_tests;
#[cfg(test)]
mod live_access_tests;
#[cfg(test)]
mod live_docs_tests;
#[cfg(test)]
mod live_share_tests;
#[cfg(test)]
mod live_signal_tests;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod mask_tests;
#[cfg(test)]
mod masks_behaviour_tests;
#[cfg(test)]
mod measure_tests;
#[cfg(test)]
mod menu_gaps_tests;
#[cfg(test)]
mod nested_override_tests;
#[cfg(test)]
mod node_set_refusals_tests;
#[cfg(test)]
mod online_tests;
#[cfg(test)]
mod open_issues_tests;
#[cfg(test)]
mod outline_options_tests;
#[cfg(test)]
mod page_clipboard_tests;
#[cfg(test)]
mod page_copy_navigation_tests;
#[cfg(test)]
mod page_target_tests;
#[cfg(test)]
mod paint_index_tests;
#[cfg(test)]
mod paint_reorder_tests;
#[cfg(test)]
mod paint_tests;
#[cfg(test)]
mod panels_behaviour_tests;
#[cfg(test)]
mod pdf_print_tests;
#[cfg(test)]
mod pen_end_tests;
#[cfg(test)]
mod pen_overlay_tests;
#[cfg(test)]
mod pixel_preview_tests;
#[cfg(test)]
mod playground_tests;
#[cfg(test)]
mod plugin_data_tests;
#[cfg(test)]
mod plugin_directory_tests;
#[cfg(test)]
mod plugin_panels_tests;
#[cfg(test)]
pub(crate) mod plugins_tests;
#[cfg(test)]
mod progressive_blur_tests;
#[cfg(test)]
mod projects_tests;
#[cfg(test)]
mod property_copy_tests;
#[cfg(test)]
mod proto_behaviour_tests;
#[cfg(test)]
mod proto_conditional_tests;
#[cfg(test)]
mod proto_delay_tests;
#[cfg(test)]
mod proto_multi_tests;
#[cfg(test)]
mod proto_overlay_anchor_tests;
#[cfg(test)]
mod proto_polish_tests;
#[cfg(test)]
mod proto_present_tests;
#[cfg(test)]
mod proto_responsive_tests;
#[cfg(test)]
mod proto_run_tests;
#[cfg(test)]
mod proto_tail_tests;
#[cfg(test)]
mod rest_slot_tests;
#[cfg(test)]
mod retained_walk_tests;
#[cfg(test)]
mod rotated_guides_tests;
#[cfg(test)]
mod rotated_select_tests;
#[cfg(test)]
mod rotation_origin_tests;
#[cfg(test)]
mod save_disk_changes_tests;
#[cfg(test)]
mod save_format_tests;
#[cfg(test)]
mod scale_tool_tests;
#[cfg(test)]
mod section_contents_tests;
#[cfg(test)]
mod section_tests;
#[cfg(test)]
mod select_area_tests;
#[cfg(test)]
mod selection_behaviour_tests;
#[cfg(test)]
mod selection_depth_tests;
#[cfg(test)]
mod shape_handles_tests;
#[cfg(test)]
mod shapebuilder_strokes_tests;
#[cfg(test)]
mod shapebuilder_tests;
#[cfg(test)]
mod shapes_behaviour_tests;
#[cfg(test)]
mod shortcut_parity_tests;
#[cfg(test)]
mod sketch_import_tests;
#[cfg(test)]
mod slot_tests;
#[cfg(test)]
mod stroke_style_tests;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
mod style_vars_tests;
#[cfg(test)]
mod styles_behaviour_tests;
#[cfg(test)]
mod svg_import_tests;
#[cfg(test)]
mod tailwind_tests;
#[cfg(test)]
mod templates_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_behaviour_tests;
#[cfg(test)]
mod text_fill_tests;
#[cfg(test)]
mod text_metrics_local_tests;
#[cfg(test)]
mod text_motion_tests;
#[cfg(test)]
mod text_style_picker_tests;
#[cfg(test)]
mod text_tail_tests;
#[cfg(test)]
mod textfit_tests;
#[cfg(test)]
mod theme_pages_tests;
#[cfg(test)]
mod tiff_tests;
#[cfg(test)]
mod transform_behaviour_tests;
#[cfg(test)]
mod variable_tests;
#[cfg(test)]
mod variables_behaviour_tests;
#[cfg(test)]
mod variables_editor_tests;
#[cfg(test)]
mod variables_import_shapes_tests;
#[cfg(test)]
mod variables_io_tests;
#[cfg(test)]
mod variants_discovery_tests;
#[cfg(test)]
mod vector_behaviour_tests;
#[cfg(test)]
mod vector_drawable_export_tests;
#[cfg(test)]
mod vector_fill_rule_tests;
#[cfg(test)]
mod vector_multi_behaviour_tests;
#[cfg(test)]
mod vector_offset_tests;
#[cfg(test)]
mod vector_path_set_tests;
#[cfg(test)]
mod version_preview_tests;
#[cfg(test)]
mod version_tests;
#[cfg(test)]
mod view_behaviour_tests;
#[cfg(test)]
mod vue_codegen_tests;
#[cfg(test)]
mod widget_input_tests;
#[cfg(test)]
mod widgets_tests;

#[cfg(test)]
mod selection_reorder_tests;
#[cfg(test)]
mod selection_spacing_tests;
