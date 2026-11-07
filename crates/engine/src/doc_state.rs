use super::view_state::ViewState;
use super::{Arc, BTreeSet, Deserialize, Document, NodeId, Serialize, cmd, components, file_access, guides, library, rotation_origin, vectoredit};

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
    /// Inside a group, the undo depth just after the step the group pushed: the group's later
    /// edits add to that step instead of keeping a snapshot each (#1099).
    pub(crate) group_step: Option<usize>,
}

/// An in-progress gesture: the document before it started, so previews re-apply from scratch.
#[derive(Clone, Debug)]
pub struct Interaction {
    pub label: String,
    pub doc: Arc<Document>,
    pub selection: Vec<NodeId>,
    /// The gesture so far only moved the selected layers by this much (page units) from `doc`:
    /// their outlines, box and everything else that doesn't depend on where they are can follow
    /// by shifting what was worked out from `doc` (#1571). `None` for any other change.
    pub moved_by: Option<(f64, f64)>,
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
    pub(crate) saved_doc: Arc<Document>,
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
    pub(crate) versions_changed: bool,
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
    pub(crate) view_from_prefs: bool,
    /// The last duplicate while its copies are the selection: Duplicate repeats its offset (see
    /// [`cmd::edit::Duplicated`]; not saved).
    pub(crate) last_duplicate: Option<cmd::edit::Duplicated>,
    images_left_in_file: Option<(String, usize)>,
    /// The file on disk as this tab last read or wrote it: a save refuses to replace what another
    /// app (figcraft-cli, say) wrote to it since (#1330).
    pub(crate) on_disk: Option<library::Stamp>,
}

/// A version shown on the canvas in place of the document: read-only until it's restored or the
/// preview ends, which puts the document back as it was.
#[derive(Clone, Debug)]
pub struct VersionPreview {
    pub id: String,
    pub name: String,
    /// When the version was saved (Unix seconds).
    pub time: u64,
    pub(crate) doc: Arc<Document>,
    pub(crate) selection: Vec<NodeId>,
    pub(crate) page: NodeId,
}

/// Why edits are refused while a version is shown.
pub const PREVIEW_READ_ONLY: &str = "you're viewing an earlier version: restore it or go back to the current version to edit";

/// Commands that first end a version preview: they act on the document itself. Saving does so
/// only with a native destination; export copies and pending format choices keep the preview.
pub(crate) const ENDS_PREVIEW: &[&str] =
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
