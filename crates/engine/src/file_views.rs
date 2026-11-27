//! Files open again where they were left: when a file's tab closes or the app quits, the page it
//! showed and each page's view (its centre and zoom) are kept for that file, and opening it again
//! shows them, as Figma opens a file where you were. They live in the preferences, not in the
//! file, so looking around never changes a file.

use serde::{Deserialize, Serialize};

use crate::{NodeId, Session, ViewState};

/// How many files' views are kept, the most recently left first.
pub const MAX_FILE_VIEWS: usize = 100;

/// Where a file was left.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileView {
    pub path: String,
    /// The page shown.
    pub page: NodeId,
    /// The view of each page shown while it was open.
    pub pages: Vec<PageView>,
}

/// A page's view: the page point at the canvas's centre, and the zoom.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageView {
    pub page: NodeId,
    pub cx: f64,
    pub cy: f64,
    pub zoom: f64,
}

impl Session {
    /// Keep where tab `i`'s file is left. A tab without a file of its own (a new or imported file,
    /// a branch) keeps nothing.
    pub(crate) fn remember_view(&mut self, i: usize) {
        let Some(ds) = self.docs.get(i).filter(|d| d.branch_of.is_none()) else { return };
        let Some(path) = ds.path.clone() else { return };
        let pages = ds.page_views().map(|(page, (cx, cy, zoom))| PageView { page, cx, cy, zoom }).collect();
        let view = FileView { path, page: ds.page, pages };
        let views = &mut self.prefs.file_views;
        views.retain(|v| v.path != view.path);
        views.insert(0, view);
        views.truncate(MAX_FILE_VIEWS);
        self.prefs_unsaved = true;
    }

    /// [`remember_view`](Self::remember_view) for every open tab, before the app quits.
    pub fn remember_views(&mut self) {
        for i in 0..self.docs.len() {
            self.remember_view(i);
        }
    }

    /// Show the active tab's file where it was left, if it was: its page, if the file still has
    /// it as a page that isn't a divider, at its view, and each other page at its view when it is
    /// shown. A view that isn't a finite centre with a zoom the canvas takes is left out.
    pub(crate) fn restore_view(&mut self) {
        let Ok(ds) = self.ds() else { return };
        let Some(kept) = ds.path.as_deref().and_then(|p| self.prefs.file_views.iter().find(|v| v.path == p)).cloned() else { return };
        let Ok(ds) = self.ds_mut() else { return };
        let doc = ds.doc.clone();
        let views = kept
            .pages
            .iter()
            .filter(|v| doc.pages.contains(&v.page) && [v.cx, v.cy, v.zoom].iter().all(|x| x.is_finite()))
            .filter(|v| (ViewState::MIN_ZOOM..=ViewState::MAX_ZOOM).contains(&v.zoom))
            .map(|v| (v.page, (v.cx, v.cy, v.zoom)));
        let page = Some(kept.page).filter(|p| doc.pages.contains(p) && !doc.is_divider(*p));
        ds.show_views(page, views);
    }
}

#[cfg(test)]
mod tests;
