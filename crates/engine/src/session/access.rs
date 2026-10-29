use super::{DocState, Document, EngineError, NodeId, Result, Session, Value, json};

/// The file [`Session::bring_forward`] put in front for one request, and the tab to bring back.
#[must_use = "pass it to `Session::put_back`"]
pub struct Forward {
    target: u64,
    shown: Option<u64>,
}

impl Session {
    /// The uid of open file `target`: its `id` (from `doc.list` or `document.inspect`) or its path.
    pub fn document_uid(&self, target: &Value) -> Result<u64> {
        let found = match target {
            Value::Number(n) => n.as_u64().and_then(|id| self.docs.iter().find(|d| d.uid == id)),
            Value::String(p) => self.docs.iter().find(|d| d.path.as_deref() == Some(p.as_str())),
            _ => None,
        };
        found
            .map(|d| d.uid)
            .ok_or_else(|| EngineError::Other(format!("no open document {target} (`document` takes an `id` from doc.list, or an open file's path)")))
    }

    /// Put open file `target` in front for one request (a control call naming its `document`),
    /// without the user's tab changing on screen: run the request, then [`Session::put_back`].
    pub fn bring_forward(&mut self, target: &Value) -> Result<Forward> {
        let uid = self.document_uid(target)?;
        let shown = self.ds().ok().map(|d| d.uid);
        if let Some(i) = self.docs.iter().position(|d| d.uid == uid) {
            self.active = i;
        }
        Ok(Forward { target: uid, shown })
    }

    /// Bring back the tab that was in front before [`Session::bring_forward`], unless the request
    /// itself moved the front to another open file (`file.open`, `doc.switch`…), as it would have.
    pub fn put_back(&mut self, f: Forward) {
        let moved = self.docs.iter().any(|d| d.uid == f.target) && self.ds().ok().map(|d| d.uid) != Some(f.target);
        if let Some(i) = f.shown.and_then(|u| self.docs.iter().position(|d| d.uid == u)).filter(|_| !moved) {
            self.active = i;
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
