//! Automation roots (#715): the folders the files an agent's commands read and write must lie in.
//!
//! `figcraft-cli mcp --automation-read-root DIR --automation-write-root DIR` and the desktop app
//! started with the same flags run their work [`confined`] to [`Roots`]: while a scope is active on
//! the thread, every file the engine reads must lie inside a read or a write root and every file it
//! writes inside a write root; anything else is an ordinary error. The engine reaches the file
//! system only through this module (`crates/engine/clippy.toml` denies `std::fs` and friends
//! anywhere else), so every command, present or future, is covered. Without a scope nothing changes.
//!
//! A path is made absolute against the working directory and resolved before it is compared: its
//! links are followed (`std::fs::canonicalize`), and for a file that doesn't exist yet its nearest
//! existing folder is resolved and the rest must be plain names. The comparison is by whole
//! components (`/work2` is not inside `/work`), and the I/O uses the resolved path, so what was
//! checked is what is opened.
#![allow(clippy::disallowed_methods)]

use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use crate::{EngineError, Result};

/// What a command does with a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// The folders automation may read (read and write roots) and write (write roots), resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

impl Roots {
    /// Roots from the folders given (relative ones against the working directory). Each must be an
    /// existing folder.
    pub fn new(read: &[String], write: &[String]) -> std::result::Result<Self, String> {
        let resolve = |flag: &str, dirs: &[String]| {
            dirs.iter()
                .map(|d| match std::fs::canonicalize(d) {
                    Ok(p) if p.is_dir() => Ok(p),
                    Ok(_) => Err(format!("{flag} {d}: not a folder")),
                    Err(e) => Err(format!("{flag} {d}: {e}")),
                })
                .collect::<std::result::Result<Vec<_>, String>>()
        };
        Ok(Self { read: resolve("--automation-read-root", read)?, write: resolve("--automation-write-root", write)? })
    }

    /// The resolved path to use for `access` to `path`, or why it is refused.
    pub fn check(&self, path: &Path, access: Access) -> std::result::Result<PathBuf, String> {
        let asked = path.display();
        let (roots, what) = match access {
            Access::Read => ([self.read.as_slice(), self.write.as_slice()].concat(), "read"),
            Access::Write => (self.write.clone(), "written"),
        };
        match resolve(path) {
            Ok(resolved) if roots.iter().any(|r| resolved.starts_with(r)) => return Ok(resolved),
            Ok(_) => {}
            // Why a path can't be resolved says what exists there: only told for paths inside a root.
            Err(why) if inside_as_far_as_it_exists(path, &roots) => return Err(format!("automation path rejected: {asked}: {why}")),
            Err(_) => {}
        }
        if roots.is_empty() {
            return Err(format!("automation path rejected: no folder may be {what} (no root for it was given): {asked}"));
        }
        let shown: Vec<String> = roots.iter().map(|r| shown(r)).collect();
        Err(format!("automation path rejected: {asked} is outside the folders that may be {what}: {}", shown.join(", ")))
    }

    /// The folders as JSON, resolved: `{read: [every folder that may be read], write: [the write roots]}`.
    pub fn to_json(&self) -> serde_json::Value {
        let list = |v: &[PathBuf]| v.iter().map(|p| shown(p)).collect::<Vec<_>>();
        serde_json::json!({"read": list(&[self.read.as_slice(), self.write.as_slice()].concat()), "write": list(&self.write)})
    }
}

/// Whether `path` lies in one of `roots` once made absolute with its `.` and `..` taken by name and
/// its longest leading part that exists resolved, so links on the way are followed as the roots'
/// were (`/tmp/work/x` is inside root `/private/tmp/work`, a link out of a root leads out of it).
fn inside_as_far_as_it_exists(path: &Path, roots: &[PathBuf]) -> bool {
    let Ok(abs) = std::path::absolute(path) else { return false };
    let mut lexical = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            c => lexical.push(c),
        }
    }
    let mut rest: Vec<&OsStr> = Vec::new();
    let mut at = lexical.as_path();
    let resolved = loop {
        if let Ok(mut base) = std::fs::canonicalize(at) {
            base.extend(rest.iter().rev());
            break base;
        }
        let (Some(name), Some(parent)) = (at.file_name(), at.parent()) else { return false };
        rest.push(name);
        at = parent;
    };
    roots.iter().any(|r| resolved.starts_with(r))
}

/// Whether `path` lies in the app's own folder `folder`, both resolved (links and `..` followed);
/// false when either can't be resolved.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn within_app_folder(path: &Path, folder: &Path) -> bool {
    matches!((std::fs::canonicalize(path), std::fs::canonicalize(folder)), (Ok(p), Ok(f)) if p.starts_with(&f))
}

pub fn same_file(a: impl AsRef<Path>, b: impl AsRef<Path>) -> bool {
    let (a, b) = (a.as_ref(), b.as_ref());
    a == b || same_file::is_same_file(a, b).unwrap_or(false)
}

/// `p` without Windows' `\\?\` prefix, for messages.
fn shown(p: &Path) -> String {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!(r"\\{unc}"),
        None => s.strip_prefix(r"\\?\").unwrap_or(&s).to_string(),
    }
}

/// `path` made absolute and resolved: links followed, and for a path that doesn't exist yet its
/// nearest existing folder resolved with the rest appended (plain names only).
fn resolve(path: &Path) -> std::result::Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("not a file path".into());
    }
    let abs = std::path::absolute(path).map_err(|e| e.to_string())?;
    let mut rest: Vec<OsString> = Vec::new();
    let mut at = abs.as_path();
    loop {
        match std::fs::canonicalize(at) {
            Ok(mut base) => {
                base.extend(rest.iter().rev());
                return Ok(base);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Writing through a link that leads nowhere would create its target, wherever it is.
                if std::fs::symlink_metadata(at).is_ok() {
                    return Err("a link that leads nowhere".into());
                }
                match at.components().next_back() {
                    Some(Component::Normal(name)) => {
                        plain_name(name)?;
                        rest.push(name.to_os_string());
                    }
                    Some(Component::CurDir) => {}
                    _ => return Err("`..` (or a drive) below a folder that doesn't exist".into()),
                }
                at = at.parent().ok_or("no existing folder")?;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// A name for a file or folder that doesn't exist yet: on Windows no device (`CON`, `NUL`, `COM1`…)
/// and no alternate data stream (`a:b`).
fn plain_name(name: &OsStr) -> std::result::Result<(), String> {
    if cfg!(windows) {
        let n = name.to_string_lossy();
        let stem = n.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.chars().count() == 4
                && stem.chars().last().is_some_and(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³')));
        if device || n.contains(':') {
            return Err(format!("`{n}` is a device or stream name, not a file"));
        }
    }
    Ok(())
}

thread_local! {
    static ROOTS: RefCell<Option<Arc<Roots>>> = const { RefCell::new(None) };
}

/// Run `f` with this thread's file access confined to `roots` (unchanged when `None`).
pub fn confined<R>(roots: Option<&Arc<Roots>>, f: impl FnOnce() -> R) -> R {
    match roots {
        Some(roots) => scoped(Some(roots.clone()), f),
        None => f(),
    }
}

/// Run `f` unconfined: only for the app's own files, never for a path a command was given.
// Only plugins read from disk use it, and the browser has no disk.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn app_owned<R>(f: impl FnOnce() -> R) -> R {
    scoped(None, f)
}

fn scoped<R>(roots: Option<Arc<Roots>>, f: impl FnOnce() -> R) -> R {
    /// Puts the outer scope's roots back, also when `f` panics.
    struct Restore(Option<Arc<Roots>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let outer = self.0.take();
            ROOTS.with(|r| *r.borrow_mut() = outer);
        }
    }
    let _restore = Restore(ROOTS.with(|r| std::mem::replace(&mut *r.borrow_mut(), roots)));
    f()
}

/// The roots this thread's file access is confined to now, if any.
pub fn current() -> Option<Arc<Roots>> {
    ROOTS.with(|r| r.borrow().clone())
}

/// The path to use for `access` to `path`: resolved when confined, else as given.
pub fn allow(path: impl AsRef<Path>, access: Access) -> Result<PathBuf> {
    let path = path.as_ref();
    match current() {
        Some(roots) => roots.check(path, access).map_err(EngineError::Other),
        None => Ok(path.to_path_buf()),
    }
}

/// Whether `path` may be accessed so now.
pub fn allowed(path: impl AsRef<Path>, access: Access) -> bool {
    allow(path, access).is_ok()
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> EngineError + '_ {
    move |e| EngineError::Other(format!("{}: {e}", path.display()))
}

/// The bytes of file `path`.
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    std::fs::read(allow(path, Access::Read)?).map_err(io(path))
}

/// The bytes of file `path`, refused when it isn't a file or is larger than `max`.
pub fn read_capped(path: impl AsRef<Path>, max: u64) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let p = allow(path, Access::Read)?;
    let meta = std::fs::metadata(&p).map_err(io(path))?;
    if !meta.is_file() || meta.len() > max {
        return Err(EngineError::Other(format!("{}: not a file of at most {max} bytes", path.display())));
    }
    std::fs::read(p).map_err(io(path))
}

/// File `path` opened for reading.
pub fn open(path: impl AsRef<Path>) -> Result<std::fs::File> {
    let path = path.as_ref();
    std::fs::File::open(allow(path, Access::Read)?).map_err(io(path))
}

/// What the file system says about `path` (links followed).
pub fn metadata(path: impl AsRef<Path>) -> Result<std::fs::Metadata> {
    let path = path.as_ref();
    std::fs::metadata(allow(path, Access::Read)?).map_err(io(path))
}

/// What the file system says about `path` itself: a link is not followed.
pub fn link_metadata(path: impl AsRef<Path>) -> Result<std::fs::Metadata> {
    let path = path.as_ref();
    std::fs::symlink_metadata(allow(path, Access::Read)?).map_err(io(path))
}

/// Whether `path` is a folder (false when it may not be read).
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    metadata(path).is_ok_and(|m| m.is_dir())
}

/// Whether `path` exists (false when it may not be read).
pub fn exists(path: impl AsRef<Path>) -> bool {
    metadata(path).is_ok()
}

/// The paths of the entries of folder `path`.
pub fn read_dir(path: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let path = path.as_ref();
    let dir = allow(path, Access::Read)?;
    Ok(std::fs::read_dir(&dir).map_err(io(path))?.flatten().map(|e| e.path()).collect())
}

/// Write `bytes` to `path` through a uniquely named staging file and one atomic rename (see
/// [`figcraft_format::write_file_atomic`]).
pub fn write(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    let path = path.as_ref();
    write_atomic(&allow(path, Access::Write)?, bytes).map_err(io(path))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    figcraft_format::write_file_atomic(path, bytes)
}

#[cfg(target_arch = "wasm32")]
fn write_atomic(_: &Path, _: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no file system in the browser"))
}

/// Create folder `path` and the folders above it that are missing.
pub fn create_dir_all(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    std::fs::create_dir_all(allow(path, Access::Write)?).map_err(io(path))
}

#[cfg(test)]
#[path = "file_access_tests.rs"]
mod tests;
