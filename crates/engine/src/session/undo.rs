use super::*;

impl Session {
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
            crate::derived::normalize_with_capture(&mut doc, &it.doc, false);
            Self::check_text_font_edit(&it.label, &it.doc, &doc)?;
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
            let changed = changed_nodes(&ds.doc, &it.doc);
            figcraft_text::appearance::capture(ds.doc_mut(), Some(&changed));
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
}
