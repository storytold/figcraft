use super::{Arc, Clipboard, DocState, Document, Prefs, Session, background, cmd, events, library, live, plugins, tools};

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
            events: Arc::new(events::EventLog::default()),
            pending_events: Vec::new(),
            agent: None,
            agent_presence: None,
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
}
