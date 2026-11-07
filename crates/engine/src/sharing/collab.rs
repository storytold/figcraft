//! The engine's side of real-time collaboration (`figcraft_collab`): its normalize pass as the
//! recompute hook, and `collab.diff`, which shows the change batch an edit syncs as.

use std::collections::BTreeSet;
use std::sync::Arc;

use figcraft_collab::{Recompute, Replica, diff, message};
use figcraft_doc::{Document, NodeId};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::params::string;
use crate::{EngineError, Result, Session, bad, normalize, spec};

/// Derived data follows the engine's normalize pass: text boxes, groups, instances following
/// their mains, variable bindings and auto layout, recomputed over the touched nodes.
pub struct EngineRecompute;

impl Recompute for EngineRecompute {
    fn recompute(&self, doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>) {
        doc.mark_dirty(touched.iter().copied());
        normalize(doc, before);
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "collab.diff",
        "Show sync changes",
        "{against? (undo: the last undoable action, the default | saved: since the last save)} → the changes a live session would send, their count and binary size",
        false,
        diff_cmd
    )]
}

fn diff_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let ds = s.ds()?;
    let from: Arc<Document> = match string(p, "against").as_deref().unwrap_or("undo") {
        "undo" if s.live_doc(ds.uid) => {
            return Err(EngineError::Disabled("collab.diff".into(), "in a live session undo is per user: compare against \"saved\"".into()));
        }
        "undo" => {
            ds.history.undo.last().map(|e| e.doc.clone()).ok_or_else(|| EngineError::Disabled("collab.diff".into(), "nothing to undo".into()))?
        }
        "saved" => ds.saved_doc.clone(),
        other => return Err(bad("collab.diff", format!("`against` is \"undo\" or \"saved\", not {other:?}"))),
    };
    let base = Replica::new((*from).clone());
    let d = diff(&base, &ds.doc).map_err(|e| EngineError::Other(e.to_string()))?;
    let bytes = message::encode(&d.changes).map_err(|e| EngineError::Other(e.to_string()))?.len();
    Ok(json!({"changes": d.changes, "count": d.changes.len(), "bytes": bytes}))
}
