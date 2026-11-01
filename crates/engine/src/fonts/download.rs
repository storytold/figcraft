//! Downloading a missing font (#754) through the suite's font store, `craft-font-store`: its
//! pinned Google Fonts and Noto indexes (compiled in) tell which families can be had, and it
//! downloads their files, checks each one's size and sha256 and installs them into the one folder
//! every Craft app's font scan reads. Desktop only.
//!
//! Downloading is off until the user turns it on, in the app's own window ([`allow_downloads`],
//! which is not a command). The commands drive the store as its agent, as the store's rules
//! have it: they install while the user has downloading on, are refused while it is off, and can
//! never turn it on.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};

use craft_font_catalog::{FontEnv, font_store_dir};
use craft_font_store::{Caller, Cancel, FontStore, Index, Network};
use serde_json::{Value, json};

use super::Download;
use crate::cmd::CommandSpec;
use crate::params::req_string;
use crate::{EngineError, Result, Session, bad, spec};

/// Lower-case family → (spelling, download). Installs are the machine's, so the jobs are the
/// process's, whichever document asked.
static JOBS: Mutex<BTreeMap<String, (String, Download)>> = Mutex::new(BTreeMap::new());
/// Set by [`use_store_dir`].
static STORE_DIR: OnceLock<PathBuf> = OnceLock::new();

fn jobs() -> std::sync::MutexGuard<'static, BTreeMap<String, (String, Download)>> {
    JOBS.lock().unwrap_or_else(PoisonError::into_inner)
}

fn set(family: &str, state: Download) {
    if let Some(job) = jobs().get_mut(&family.to_lowercase()) {
        job.1 = state;
    }
}

/// Use `dir` as the font store instead of the suite's folder, for this process (tests: they never
/// read the user's setting). The first call wins.
pub fn use_store_dir(dir: PathBuf) {
    let _ = STORE_DIR.set(dir);
}

fn store_dir() -> Option<PathBuf> {
    if let Some(dir) = STORE_DIR.get() {
        return Some(dir.clone());
    }
    // This crate's tests never touch the user's store.
    if cfg!(test) {
        return Some(std::env::temp_dir().join(format!("figcraft-font-store-tests-{}", std::process::id())));
    }
    font_store_dir(&FontEnv::from_process())
}

fn store(caller: Caller) -> Option<FontStore> {
    Some(FontStore::new(store_dir()?, caller))
}

/// The indexes, read once: the built-in ones compiled in and the sources the user added as last
/// fetched. No request.
fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| store(Caller::Agent).map(|s| s.offline_indexes()).unwrap_or_else(|| Index::bundled().unwrap_or_default()))
}

/// The name of the source `family` can be downloaded from (`Google Fonts`, `Noto`), if any.
pub fn source(family: &str) -> Option<String> {
    let index = index();
    index.family(family.trim()).and_then(|f| index.source_of(f)).map(|s| s.name.clone())
}

/// Whether the user turned downloading fonts on (the setting is the store's, shared by every
/// Craft app).
pub fn downloads_allowed() -> bool {
    store(Caller::Agent).is_some_and(|s| s.network() == Network::On)
}

/// Turn downloading fonts on. Only from the user's own action in the app's interface (a click in
/// the Missing fonts window), never from a command: agents can't.
pub fn allow_downloads() -> std::result::Result<(), String> {
    store(Caller::User).ok_or_else(|| "this system has no font store folder".to_string())?.enable_network().map_err(|e| e.to_string())
}

/// The downloads of this process: (family, state).
pub fn downloads() -> Vec<(String, Download)> {
    jobs().values().cloned().collect()
}

fn state_json(family: &str, state: &Download) -> Value {
    match state {
        Download::Running { done, total } => json!({"family": family, "state": "downloading", "done": done, "total": total}),
        Download::Installed => json!({"family": family, "state": "installed"}),
        Download::Failed(error) => json!({"family": family, "state": "failed", "error": error}),
    }
}

/// `font.download`: start downloading `family`'s files on a thread of its own, or report the
/// download under way; a family that is there now fits the open document's text to it instead.
fn download(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "font.download";
    let family = req_string(CMD, p, "family")?.trim().to_string();
    let key = family.to_lowercase();
    let state = jobs().get(&key).map(|(_, st)| st.clone());
    if let Some(running @ Download::Running { .. }) = &state {
        return Ok(state_json(&family, running));
    }
    // Installed: even when no scan sees it (system fonts opted out), it isn't downloaded again.
    if state == Some(Download::Installed) || figcraft_text::has_family(&family) {
        jobs().remove(&key);
        let refitted = crate::fonts::refit_open(s, None, Some(std::slice::from_ref(&family)));
        return Ok(json!({"family": family, "state": "installed", "refitted": refitted}));
    }
    let index = index();
    let found = index.family(&family).ok_or_else(|| bad(CMD, format!("{family} is not in the font store's index")))?;
    let store = store(Caller::Agent).ok_or_else(|| EngineError::Other("this system has no font store folder".into()))?;
    if store.network() != Network::On {
        return Err(EngineError::Disabled(CMD.into(), "downloading fonts is turned off; the user turns it on in Text ▸ Missing fonts".into()));
    }
    let total: u64 = found.files.iter().map(|f| f.size).sum();
    let source = index.source_of(found).map(|src| src.name.clone());
    jobs().insert(key, (found.family.clone(), Download::Running { done: 0, total }));
    let name = found.family.clone();
    let spawned = std::thread::Builder::new().name("figcraft-font-download".into()).spawn(move || {
        let result = store.install_family(index, found, &Cancel::new(), &mut |p| set(&name, Download::Running { done: p.done, total: p.total }));
        match result {
            Ok(_) => {
                // The new files are scanned here, off the interface's thread.
                figcraft_text::rescan_system_fonts();
                set(&name, Download::Installed);
            }
            Err(e) => set(&name, Download::Failed(e.to_string())),
        }
    });
    if let Err(e) = spawned {
        jobs().remove(&family.to_lowercase());
        return Err(EngineError::Other(format!("couldn't start the download: {e}")));
    }
    Ok(json!({"family": found.family, "source": source, "state": "downloading", "done": 0, "total": total, "files": found.files.len()}))
}

/// The font commands that download: `font.download` and `font.downloads`.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "font.download",
            "Download font",
            "{family}  downloads a missing font family from the font store (Google Fonts, Noto: pinned, checksum-verified files installed into the Craft apps' shared font folder) on a thread of its own; refused while the user has downloading turned off (only the user turns it on, in Text ▸ Missing fonts). Call again to see its progress; once it is installed, the call fits the open document's text to it (text.refit) → {family, state: downloading|installed, done?, total?, refitted?}",
            true,
            download
        ),
        spec!(
            "font.downloads",
            "Font downloads",
            "{}  this session's font downloads and whether downloading fonts is turned on → {allowed, downloads: [{family, state: downloading|installed|failed, done?, total?, error?}]}",
            false,
            |_, _| {
                let list: Vec<Value> = downloads().iter().map(|(f, st)| state_json(f, st)).collect();
                Ok(json!({"allowed": downloads_allowed(), "downloads": list}))
            }
        ),
    ]
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
