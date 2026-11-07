//! Recovery copies in the browser. The browser build can't catch a panic (wasm32 always aborts),
//! so a command or frame that panics ends the page instead of failing as on the desktop. The page
//! follows its files with unsaved changes after every frame ([`Unsaved::follow`]: cheap shared
//! copies, taken only when a file changed); the panic hook writes them to the page's local storage
//! ([`store`]) before the page stops, and the next page opens them again as unsaved files
//! ([`restore`], `file.recover`), as the desktop app does with its recovery copies.

use std::sync::Arc;

use base64::Engine as _;
use figcraft_engine::branches::{needs_copy, recovery_copy, stamp};
use figcraft_engine::doc::Document;
use figcraft_engine::{DocState, Session};
use figcraft_format::{Branch, Version};
use serde_json::json;

/// A file with unsaved changes as of the last frame.
struct Copy {
    uid: u64,
    /// [`stamp`]: the revisions this copy is of.
    stamp: Vec<u64>,
    doc: Arc<Document>,
    versions: Vec<Version>,
    branches: Vec<Branch>,
}

impl Copy {
    fn of(s: &Session, d: &DocState) -> Self {
        let (doc, versions, branches) = recovery_copy(s, d);
        Self { uid: d.uid, stamp: stamp(s, d), doc, versions, branches }
    }
}

/// The page's files with unsaved changes, as of the last frame.
#[derive(Default)]
pub struct Unsaved(Vec<Copy>);

impl Unsaved {
    /// Catch up with `s`: a file that changed is taken again, the others are kept as they are.
    pub fn follow(&mut self, s: &Session) {
        let mut before = std::mem::take(&mut self.0);
        for d in s.docs.iter().filter(|d| needs_copy(s, d)) {
            let now = stamp(s, d);
            let copy = match before.iter().position(|c| c.uid == d.uid && c.stamp == now) {
                Some(i) => before.swap_remove(i),
                None => Copy::of(s, d),
            };
            self.0.push(copy);
        }
    }

    /// How many files have unsaved changes.
    pub fn count(&self) -> usize {
        self.0.len()
    }

    /// Each file as a saved `.figcraft` file in base64 (one that can't be written is left out).
    pub fn files(&self) -> Vec<String> {
        let b64 = base64::engine::general_purpose::STANDARD;
        self.0
            .iter()
            .filter_map(|c| figcraft_format::save_bytes_with_history(&c.doc, None, &c.versions, &c.branches).ok().map(|b| b64.encode(b)))
            .collect()
    }
}

/// Keep as many of `files` as the storage takes, in order: `put` stores the list's JSON and says
/// whether it fitted (a refused write leaves what was stored before). Returns how many were kept.
pub fn store(files: Vec<String>, mut put: impl FnMut(&str) -> bool) -> usize {
    let mut kept = Vec::new();
    for f in files {
        kept.push(f);
        if !put(&json!(kept).to_string()) {
            kept.pop();
        }
    }
    kept.len()
}

/// Open the copies the last page kept (`kept`: what [`store`] stored, if anything) as files with
/// unsaved changes. Returns how many came back; junk in the storage is skipped.
pub fn restore(s: &mut Session, kept: Option<String>) -> usize {
    let files: Vec<String> = kept.and_then(|k| serde_json::from_str(&k).ok()).unwrap_or_default();
    files.iter().filter(|f| s.execute("file.recover", &json!({"data": f})).is_ok()).count()
}
