use super::*;

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
            .and_then(|v| self.check_finite_geometry(id, &before).map(|()| v))
            .and_then(|v| self.check_text_fonts(id, &before).map(|()| v));
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

    /// Preserve absent-font text until an explicit, installed font replacement is selected.
    fn check_text_fonts(&self, id: &str, before: &Option<(Arc<Document>, Vec<NodeId>, NodeId, u64)>) -> Result<()> {
        let Some((old, _, _, uid)) = before else { return Ok(()) };
        let Some(ds) = self.docs.iter().find(|d| d.uid == *uid) else { return Ok(()) };
        if Arc::ptr_eq(&ds.doc, old) {
            return Ok(());
        }
        Self::check_text_font_edit(id, old, &ds.doc)
    }

    pub(super) fn check_text_font_edit(id: &str, old: &Document, candidate: &Document) -> Result<()> {
        for n in candidate.nodes() {
            if old.node_arc(n.id).zip(candidate.node_arc(n.id)).is_some_and(|(a, b)| Arc::ptr_eq(a, b)) {
                continue;
            }
            let Some(b) = old.node(n.id) else { continue };
            let (figcraft_doc::NodeKind::Text(a), figcraft_doc::NodeKind::Text(t)) = (&b.kind, &n.kind) else { continue };
            if a == t && b.width == n.width && b.height == n.height {
                continue;
            }
            if !figcraft_text::appearance::has_saved(a, Some(b.width), Some(b.height)) {
                continue;
            }
            if figcraft_text::appearance::has_saved(t, Some(n.width), Some(n.height)) {
                // Metadata-only edits do not need installed fonts or new outlines.
                continue;
            }
            let mut a = a.clone();
            let mut t = t.clone();
            a.appearance = None;
            t.appearance = None;
            if (a != t || b.width != n.width || b.height != n.height)
                && !figcraft_text::appearance::fonts_available(&a)
                && !figcraft_text::appearance::fonts_available(&t)
            {
                return Err(EngineError::Disabled(
                    id.into(),
                    "Install the missing fonts or explicitly replace them with installed fonts before editing this text".into(),
                ));
            }
        }
        Ok(())
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
    pub(crate) fn check_instance_structure(&self, id: &str, before: &Option<(Arc<Document>, Vec<NodeId>, NodeId, u64)>) -> Result<()> {
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
        // Hosting likewise only captures text metadata into the final shared document; keep its
        // saved baseline and the host's view aligned with the snapshot sent to the link.
        if changed && !matches!(spec.id, "file.save" | "file.saveAs" | "collab.host" | "collab.share") {
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
}
