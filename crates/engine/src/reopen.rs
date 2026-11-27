//! The files open when the desktop app quit open again at its next start, in their tabs' order
//! with the same one in front, as Figma's desktop app reopens its tabs.

use serde_json::json;

use crate::{EngineError, Session};

impl Session {
    /// Keep the open files to reopen at the next start: each tab's file that has a path, in tab
    /// order, and the one in front. Files without one (new, imported, joined from a live session)
    /// and branch tabs are left out.
    pub fn remember_open_files(&mut self) {
        let path = |i: usize| self.docs.get(i).filter(|d| d.branch_of.is_none()).and_then(|d| d.path.clone());
        self.prefs.open_files = (0..self.docs.len()).filter_map(path).collect();
        self.prefs.front_file = path(self.active);
    }

    /// Open the files [`remember_open_files`](Self::remember_open_files) kept, then bring the one
    /// that was in front forward. A file that is no longer there is skipped; the others that don't
    /// open come back with why. A file already open (a recovered copy, one named at start) keeps
    /// its tab.
    pub fn reopen_files(&mut self) -> Vec<(String, EngineError)> {
        let mut failed = Vec::new();
        for path in self.prefs.open_files.clone() {
            if !crate::file_access::exists(&path) {
                continue;
            }
            if let Err(e) = self.execute("file.open", &json!({"path": path})) {
                failed.push((path, e));
            }
        }
        let front = self.prefs.front_file.as_deref();
        if let Some(i) = self.docs.iter().position(|d| front.is_some() && d.path.as_deref() == front) {
            self.active = i;
        }
        failed
    }
}
