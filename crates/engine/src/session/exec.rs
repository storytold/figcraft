use super::{
    Arc, CommandSpec, Document, ENDS_PREVIEW, EngineError, Node, NodeId, PREVIEW_READ_ONLY, Result, Session, Value, bad, board, branches, cmd,
    components, events, find_command, guard, library, normalize, params, textpreview,
};

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
    /// Append a live event with the active document's identity (nothing without one). Inside a
    /// [`Session::grouped`] call the event is buffered and committed only when the group succeeds.
    fn record_event(&mut self, kind: &str, data: Value) {
        let (document, path) = match self.ds() {
            Ok(d) => (d.doc.name.clone(), d.path.clone()),
            Err(_) => (String::new(), None),
        };
        if self.group_depth > 0 {
            self.pending_events.push((kind.to_string(), document, path, data));
        } else {
            self.events.push(kind, &document, path.as_deref(), data);
        }
    }

    /// Restore the batch context an inner command had at entry when it panicked.
    pub(crate) fn restore_after_internal_error(&mut self, group_depth: usize, pending_events: usize) {
        self.group_depth = group_depth;
        self.pending_events.truncate(pending_events);
    }

    /// Commit events buffered by a successful group.
    pub(super) fn flush_pending_events(&mut self) {
        let pending = std::mem::take(&mut self.pending_events);
        for (kind, document, path, data) in pending {
            self.events.push(&kind, &document, path.as_deref(), data);
        }
    }

    /// [`Session::execute`] for an AI agent named `agent` (the client's name; empty when it gave
    /// none), or for the user (`None`): the comments it makes are signed with the agent's name and
    /// marked as an agent's.
    pub fn execute_as(&mut self, agent: Option<&str>, id: &str, params: &Value) -> Result<Value> {
        let was = std::mem::replace(&mut self.agent, agent.map(cmd::comment::agent_author));
        let r = self.execute(id, params);
        self.agent = was;
        r
    }

    /// Run a command by id. Mutations become one undo step; a failure or panic leaves the document
    /// as it was.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        crate::cmd::refuse_unknown_params(id, params)?;
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
        // An inner command of a batch can panic while the outer group is still open; remember the
        // context so the panic does not clear the group's buffered events.
        let group_depth_before = self.group_depth;
        let pending_events_before = self.pending_events.len();
        let result = guard::run(id, || (spec.run)(self, params))
            .and_then(|v| self.check_instance_structure(id, &before).map(|()| v))
            .and_then(|v| self.check_preview(id, &before).map(|()| v))
            .and_then(|v| self.check_live_access(id, &before).map(|()| v))
            .and_then(|v| self.check_finite_geometry(id, &before).map(|()| v))
            // Normalising is part of the command: a panic in it rolls the edit back too (#1085).
            .and_then(|v| {
                guard::run(id, || {
                    self.normalize_edit(spec, &before);
                    Ok(v)
                })
            });
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
        self.after_comment_command(id, params, &result, group_depth_before, pending_events_before);
        result
    }

    /// Record the live event a successful mutating `comment.*` command makes. A panic caught by the
    /// guard can leave a group's event buffer open: restore the depth and buffer it had when the
    /// command started (`group_depth`, `pending_events`), not the whole session's.
    fn after_comment_command(&mut self, id: &str, params: &Value, result: &Result<Value>, group_depth: usize, pending_events: usize) {
        match result {
            Ok(v) => {
                if let Some(kind) = events::comment_event_kind(id) {
                    let comment_id = v.get("id").and_then(Value::as_u64).or_else(|| params.get("id").and_then(params::id_of).map(|n| n.0));
                    self.record_event(
                        kind,
                        serde_json::json!({"command": id, "commentId": comment_id, "reply": v.get("reply").cloned().unwrap_or(Value::Null)}),
                    );
                }
            }
            Err(EngineError::Internal { .. }) => self.restore_after_internal_error(group_depth, pending_events),
            Err(_) => {}
        }
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

    /// Normalise what a command changed in the document it ran on (text boxes, groups, instances,
    /// auto layout…).
    fn normalize_edit(&mut self, spec: &CommandSpec, before: &Option<(Arc<Document>, Vec<NodeId>, NodeId, u64)>) {
        let Some((bdoc, _, _, uid)) = before else { return };
        let Some(ds) = self.docs.iter_mut().find(|d| d.uid == *uid) else { return };
        // Saving may adopt the chosen filename as the document title. The encoded snapshot is
        // already final; normalising it again would detach it from the saved baseline.
        if !Arc::ptr_eq(&ds.doc, bdoc) && !matches!(spec.id, "file.save" | "file.saveAs") {
            normalize(ds.doc_mut(), bdoc);
        }
    }

    /// Bookkeeping after a successful command (normalised already): push undo, bump revision.
    fn after_command(&mut self, spec: &CommandSpec, before: Option<(Arc<Document>, Vec<NodeId>, NodeId, u64)>) {
        let Some((bdoc, bsel, bpage, uid)) = before else {
            if let Ok(ds) = self.ds_mut() {
                ds.revision += 1;
            }
            return;
        };
        let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid) else { return };
        let edit = !Arc::ptr_eq(&ds.doc, &bdoc) && ds.interaction.is_none();
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
