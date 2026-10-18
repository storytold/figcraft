//! Team libraries: a file's published library kept on a `figcraft-server` under that file, and
//! used by other files through one of its share links (a view link is enough to read it).
//!
//! A library on a server is the same published library a library file holds (manifest and
//! snapshot, with per-asset versions), carried as a small `.figcraft` archive holding only it. Its
//! link takes the place of a library file's path in [`LibraryLink::path`], so everything else
//! (the Assets panel, placing components, updates with overrides kept) works as for library files.
//! The engine never opens a socket: requests go through the app's [`LiveLink`](crate::live::LiveLink).
//! A command that needs a library it hasn't fetched yet waits for the server (the UI runs such
//! commands without waiting: [`crate::background`]); the Assets panel refreshes in the background
//! ([`Session::refresh_server_libraries`]) and keeps the last library fetched while the server
//! can't be reached.

use std::io::Cursor;
use std::sync::Arc;

use figcraft_collab::message::{LibraryReply, LibraryReq, Usage};
use figcraft_doc::{Document, NodeId, NodeKind, PublishedLibrary};

use crate::live::{NetReply, NetRequest};
use crate::{EngineError, Result, Session};

/// What a server library that hasn't been fetched yet says.
pub const NOT_FETCHED: &str = "the library hasn't been fetched from its server yet";
/// What a server file without a library says.
pub const NO_LIBRARY: &str = "that file has no library on the server yet: publish it there first (Libraries ▸ Publish to server)";
/// Most components one usage report lists.
pub const MAX_USAGE: usize = 10_000;

/// Whether a library's path is a share link of a file on a server (else it is a library file).
pub fn is_server_link(path: &str) -> bool {
    let p = path.trim_start().to_ascii_lowercase();
    ["figcraft://", "ws://", "wss://"].iter().any(|s| p.starts_with(s))
}

/// A library path to show: a server link without its token.
pub fn shown_path(path: &str) -> String {
    if !is_server_link(path) {
        return path.to_string();
    }
    let Some(at) = path.find("token=") else { return path.to_string() };
    let end = path.get(at..).and_then(|rest| rest.find('&')).map_or(path.len(), |e| at + e);
    format!("{}token=…{}", path.get(..at).unwrap_or_default(), path.get(end..).unwrap_or_default())
}

/// `lib` as the archive a server keeps: a `.figcraft` file holding only the library.
pub fn archive(lib: &PublishedLibrary) -> Result<Vec<u8>> {
    let mut holder = Document::new(lib.manifest.name.clone());
    holder.published = Some(Arc::new(lib.clone()));
    figcraft_format::save_bytes(&holder, None).map_err(|e| EngineError::Other(format!("couldn't pack the library: {e}")))
}

/// The library in an archive from a server (untrusted: read with the library file's limits).
pub fn read_archive(bytes: &[u8]) -> std::result::Result<PublishedLibrary, String> {
    match figcraft_format::read_library(Cursor::new(bytes)) {
        Ok(Some(lib)) => Ok(lib),
        Ok(None) => Err("the server sent a file without a library".into()),
        Err(e) => Err(format!("the server sent a library that can't be read ({e})")),
    }
}

/// How `doc` uses library `library`: its instances of each of the library's components (outside
/// the library copies themselves), as the server's usage report.
// shortcut: files are told apart by their names (a file has no id of its own) and a file that
// removes the library stays in the reports; send a file id and a removal once files have ids.
pub fn usage(doc: &Document, library: &str) -> Usage {
    let store = doc.library_store();
    let mut counts = std::collections::BTreeMap::<NodeId, u32>::new();
    let records: std::collections::BTreeMap<NodeId, NodeId> =
        doc.libraries.components.iter().filter(|r| r.library == library).map(|r| (r.root, r.key)).collect();
    if !records.is_empty() {
        for n in doc.nodes() {
            if let NodeKind::Instance { component, .. } = &n.kind
                && store.is_none_or(|st| doc.page_of(n.id) != Some(st))
                && let Some(key) = crate::library::asset_root(doc, *component).and_then(|r| records.get(&r))
            {
                let c = counts.entry(*key).or_default();
                *c = c.saturating_add(1);
            }
        }
    }
    Usage { file: doc.name.clone(), components: counts.into_iter().take(MAX_USAGE).collect() }
}

/// A reply that doesn't answer the request.
fn unexpected(r: &LibraryReply) -> EngineError {
    EngineError::Other(format!("the server answered {r:?}"))
}

impl Session {
    /// The share link of the active document's live session when the document is a server file
    /// you may edit (where `library.publishToServer` publishes by default).
    pub fn server_link(&self) -> Option<String> {
        let uid = self.ds().ok()?.uid;
        let live = Some(self.live_of(uid)).filter(|l| l.syncing() && l.doc_uid == Some(uid))?;
        live.info.as_ref().filter(|i| i.role == crate::live::Role::Guest && i.access == crate::live::Access::Edit).map(|i| i.link.clone())
    }

    /// One request about the team library behind share link `link` (a refusal is an error), with
    /// the request answered when a command running in the background got its answer earlier
    /// ([`crate::background`]). Callers outside the background wait for the server (at most the
    /// connect timeout when it can't be reached).
    pub(crate) fn library_exchange(&mut self, link: &str, req: LibraryReq) -> Result<(LibraryReply, Option<LibraryReq>)> {
        let (reply, sent) = self.net(NetRequest::Library { link: link.to_string(), req })?;
        let sent = match sent {
            Some(NetRequest::Library { req, .. }) => Some(req),
            _ => None,
        };
        match reply {
            NetReply::Library(Ok(LibraryReply::Failed { reason }) | Err(reason)) => Err(EngineError::Other(reason)),
            NetReply::Library(Ok(r)) => Ok((r, sent)),
            _ => Err(EngineError::Other("the server answered something else".into())),
        }
    }

    /// [`library_exchange`](Self::library_exchange) without the request.
    pub(crate) fn library_request(&mut self, link: &str, req: LibraryReq) -> Result<LibraryReply> {
        self.library_exchange(link, req).map(|(r, _)| r)
    }

    /// The published library at `path`: a library file (read again when it changed), or a server
    /// library link (fetched once; then the background refresh keeps it current).
    pub fn load_library(&mut self, path: &str) -> Result<Arc<PublishedLibrary>> {
        self.fetch_library(path, false)
    }

    /// [`load_library`](Self::load_library); with `fresh`, a team library's server is asked for a
    /// newer version first (`library.updates`, `library.list`). Fetches report how the active
    /// document uses the library.
    pub fn fetch_library(&mut self, path: &str, fresh: bool) -> Result<Arc<PublishedLibrary>> {
        if !is_server_link(path) {
            // A library link is stored in the document, so opening someone else's file would
            // otherwise reach whatever path it names. Only a path on this computer is followed: a
            // network share would make opening a document contact that server, and on Windows
            // send the credentials of whoever opened it.
            if !craft_docpath::is_local(path) {
                return Err(EngineError::Other(format!("{path}: a library is opened only from this computer")));
            }
            return self.libraries.load(path);
        }
        let have = match self.libraries.cached(path) {
            Some(Ok(lib)) if !fresh => return Ok(lib.clone()),
            Some(Ok(lib)) => Some(lib.manifest.version),
            _ => None,
        };
        let usage = self.doc().ok().map(|d| {
            let id = d.libraries.enabled.iter().find(|l| l.path == path).map(|l| l.id.clone()).unwrap_or_default();
            usage(d, &id)
        });
        let reply = match self.net(NetRequest::Library { link: path.to_string(), req: LibraryReq::Fetch { have, usage } }) {
            Ok((NetReply::Library(r), _)) => r,
            Ok(_) => Err("the server answered something else".into()),
            Err(EngineError::Waiting) => return Err(EngineError::Waiting),
            Err(e) => Err(e.to_string()),
        };
        self.take_library(path, reply).map_err(EngineError::Other)
    }

    /// Ask the servers of the active document's team libraries for newer versions, in the
    /// background, reporting how the document uses each (the Assets panel calls it now and then).
    pub fn refresh_server_libraries(&mut self) {
        let Ok(doc) = self.doc() else { return };
        let jobs: Vec<(String, LibraryReq)> = doc
            .libraries
            .enabled
            .iter()
            .filter(|l| is_server_link(&l.path))
            .map(|l| {
                let have = match self.libraries.cached(&l.path) {
                    Some(Ok(lib)) => Some(lib.manifest.version),
                    _ => None,
                };
                (l.path.clone(), LibraryReq::Fetch { have, usage: Some(usage(doc, &l.id)) })
            })
            .collect();
        if let (false, Some(net)) = (jobs.is_empty(), self.live.link()) {
            net.library_background(jobs);
        }
    }

    /// A background library request was answered: a newer library replaces the one shown ("Library
    /// updates available" follows).
    pub(crate) fn library_answer(&mut self, link: &str, reply: std::result::Result<LibraryReply, String>) {
        let _ = self.take_library(link, reply);
    }

    /// Take the answer to a fetch of the team library behind `link` into the cache. While the
    /// server can't be reached (or refuses) the last library fetched stays in use.
    fn take_library(&mut self, link: &str, reply: std::result::Result<LibraryReply, String>) -> std::result::Result<Arc<PublishedLibrary>, String> {
        let kept = match self.libraries.cached(link) {
            Some(Ok(lib)) => Some(lib.clone()),
            _ => None,
        };
        let result = match reply {
            Ok(LibraryReply::Library { archive, .. }) => read_archive(&archive).map(Arc::new),
            Ok(LibraryReply::Unchanged { .. }) => kept.clone().ok_or_else(|| NOT_FETCHED.to_string()),
            Ok(LibraryReply::Empty) => Err(NO_LIBRARY.into()),
            Ok(LibraryReply::Failed { reason }) | Err(reason) => Err(reason),
            Ok(other) => Err(unexpected(&other).to_string()),
        };
        match (result, kept) {
            (Err(e), Some(lib)) => {
                log::info!("team library {}: {e}; the last version fetched stays in use", shown_path(link));
                Ok(lib)
            }
            (result, _) => {
                self.libraries.insert(link, result.clone());
                result
            }
        }
    }
}

#[cfg(test)]
#[path = "library_server_tests.rs"]
mod tests;
