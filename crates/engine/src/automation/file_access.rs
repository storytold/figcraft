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
//!
//! On Windows, resolving a path opens it, and opening a path on another computer contacts that
//! computer and may send it the signed-in user's credentials. So on Windows, before anything is
//! resolved, a path on another computer whose share no root is on, a device path, or one reached
//! through a link that leads to them is refused by how it is spelled (#1094).
#![allow(clippy::disallowed_methods)]

use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use craft_docpath::Refused;

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
        if cfg!(windows) {
            match refused_unresolved(path, &roots) {
                Some(Unresolved::Spelled(Refused::Network)) => {
                    return Err(format!(
                        "automation path rejected: {asked} is on another computer, and no folder that may be {what} is on its share"
                    ));
                }
                Some(Unresolved::Spelled(_)) => return Err(format!("automation path rejected: {asked} names a device, not a file")),
                // Where a link leads is only told by the usual message: it would say what is there.
                Some(Unresolved::Linked) => return Err(outside(asked, &roots, what)),
                None => {}
            }
        }
        match resolve(path) {
            Ok(resolved) if roots.iter().any(|r| resolved.starts_with(r)) => return Ok(resolved),
            Ok(_) => {}
            // Why a path can't be resolved says what exists there: only told for paths inside a root.
            Err(why) if inside_as_far_as_it_exists(path, &roots) => return Err(format!("automation path rejected: {asked}: {why}")),
            Err(_) => {}
        }
        Err(outside(asked, &roots, what))
    }

    /// The folders as JSON, resolved: `{read: [every folder that may be read], write: [the write roots]}`.
    pub fn to_json(&self) -> serde_json::Value {
        let list = |v: &[PathBuf]| v.iter().map(|p| shown(p)).collect::<Vec<_>>();
        serde_json::json!({"read": list(&[self.read.as_slice(), self.write.as_slice()].concat()), "write": list(&self.write)})
    }
}

/// Why `asked` is refused: it is outside every one of `roots`, the folders that may be `what`
/// ("read" or "written"). The message lists those folders, or says that no folder may be `what`
/// when `roots` is empty.
fn outside(asked: std::path::Display<'_>, roots: &[PathBuf], what: &str) -> String {
    if roots.is_empty() {
        return format!("automation path rejected: no folder may be {what} (no root for it was given): {asked}");
    }
    let shown: Vec<String> = roots.iter().map(|r| shown(r)).collect();
    format!("automation path rejected: {asked} is outside the folders that may be {what}: {}", shown.join(", "))
}

/// Why a path is refused before it is resolved (#1094).
#[derive(Debug, PartialEq, Eq)]
enum Unresolved {
    /// Its spelling leads to another computer ([`Refused::Network`]) or a device ([`Refused::Device`]).
    Spelled(Refused),
    /// A link on the way leads there, or the way there can't be looked at: a link or folder on it
    /// can't be read, it has more than [`MAX_LINKS`] links, or the path can't be made absolute.
    Linked,
}

/// How many links are followed on the way to a path before it is given up on, as on Linux.
const MAX_LINKS: u8 = 40;

/// Why `path` is refused before anything on the way to it is opened, if it is: resolving it would
/// open a path on another computer, which on Windows contacts that computer and may send it the
/// signed-in user's credentials, or open a device (#1094). That is the case when the path as
/// given or made absolute is refused by its spelling ([`refused_spelling`]), or when a link on
/// the way, or on the way on from where one leads, leads to such a path ([`link_refused`]). A path
/// on the share of one of `roots` goes on to the usual check. A path that can't be made absolute
/// is refused as well, rather than left to what comes next.
fn refused_unresolved(path: &Path, roots: &[PathBuf]) -> Option<Unresolved> {
    // Made absolute without looking at the disk (on Windows `GetFullPathNameW`, which also turns a
    // bare device name such as `NUL` into `\\.\NUL`, as opening it would). Like opening, it takes
    // `..` by name, so `link\..\x` is `x` and never goes through the link.
    let Ok(abs) = std::path::absolute(path) else { return Some(Unresolved::Linked) };
    if let Some(why) = [path, abs.as_path()].into_iter().find_map(|p| refused_spelling(&p.to_string_lossy(), roots)) {
        return Some(Unresolved::Spelled(why));
    }
    link_refused(&abs, roots, MAX_LINKS, &on_disk).then_some(Unresolved::Linked)
}

/// What is at a path, looked at without following a link there.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    /// Nothing ([`nothing_there`]): resolving the path meets the same there and goes no further.
    Missing,
    /// Something that can't be looked at (no permission, for one). It may be a link that resolving
    /// the path would still follow, so the path is refused.
    Unreadable,
    /// A file or a folder.
    Plain,
    /// A link, with where it leads (`None` when that can't be read).
    Link(Option<PathBuf>),
}

/// What is at `p` on disk (see [`Entry`]), looked at with [`std::fs::symlink_metadata`], which
/// doesn't follow a link there; a link's target is read with [`std::fs::read_link`].
fn on_disk(p: &Path) -> Entry {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_symlink() => Entry::Link(std::fs::read_link(p).ok()),
        Ok(_) => Entry::Plain,
        Err(e) if nothing_there(&e) => Entry::Missing,
        Err(_) => Entry::Unreadable,
    }
}

/// Whether looking at a path failed with `e` because nothing is there: no such name, a file where
/// a folder was expected, or a name no file can have. Any other failure says nothing about what is
/// there.
fn nothing_there(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::{InvalidFilename, NotADirectory, NotFound};
    matches!(e.kind(), NotFound | NotADirectory | InvalidFilename)
}

/// Why path `p` is refused by its spelling alone, if it is (see [`craft_docpath::resolve`], which
/// reads both `/` and `\` as separators and compares names ignoring case):
/// - on another computer (`\\server\share`, `//server/share`, `\\?\UNC\…`, `\\.\UNC\…`, or a
///   server without a share) and on no root's share;
/// - a device path (`\\.\pipe\…`, `\\?\GLOBALROOT\…`, `\\.\C:\…`), other than a local volume's
///   `\\?\Volume{…}\` or `\\.\Volume{…}\`, which a folder can be mounted as;
/// - or with a device name (`CON`, `NUL`, `COM1.png`, `lpt1 `…) as any of its names: each folder on
///   the way is opened in turn while the path is resolved, and the device behind a name such as
///   `LPT1` can be a printer on another computer.
fn refused_spelling(p: &str, roots: &[PathBuf]) -> Option<Refused> {
    match craft_docpath::resolve(p, None) {
        // With a root passed as the document path (what a link would be relative to), a path on
        // that root's share is allowed.
        Err(Refused::Network) if !roots.iter().any(|r| craft_docpath::resolve(p, Some(r)) != Err(Refused::Network)) => {
            return Some(Refused::Network);
        }
        Err(Refused::Device) if !is_volume(p) => return Some(Refused::Device),
        _ => {}
    }
    // `craft_docpath` stops at the first name it refuses, which may not be the device's.
    p.split(['\\', '/']).any(device_name).then_some(Refused::Device)
}

/// Whether `p` is in a local volume by its `\\?\Volume{…}` (or `\\.\Volume{…}`) name.
fn is_volume(p: &str) -> bool {
    let mut names = p.split(['\\', '/']);
    let (Some(""), Some(""), Some("?" | "."), Some(volume)) = (names.next(), names.next(), names.next(), names.next()) else {
        return false;
    };
    volume.get(..7).is_some_and(|v| v.eq_ignore_ascii_case("Volume{")) && volume.ends_with('}')
}

/// Whether a link on the way to `abs` (an absolute path) leads to a path [`refused_spelling`]
/// refuses, following at most `links` links: one that leads to another, or on through further
/// links, is followed too, as resolving the path would. A link that can't be read, a name on the
/// way that can't be looked at ([`Entry::Unreadable`]), or more links than that, are refused as
/// well. Each name on the way is looked at with `look` ([`on_disk`]), which doesn't follow a
/// link: where a link leads is looked at only once its spelling is known not to be refused.
fn link_refused(abs: &Path, roots: &[PathBuf], links: u8, look: &dyn Fn(&Path) -> Entry) -> bool {
    let mut at = PathBuf::new();
    let mut rest = abs.components();
    while let Some(c) = rest.next() {
        at.push(c);
        if !matches!(c, Component::Normal(_)) {
            continue;
        }
        let target = match look(&at) {
            Entry::Plain => continue,
            // Nothing is below what isn't there, so no link either.
            Entry::Missing => return false,
            Entry::Unreadable => return true,
            Entry::Link(target) => target,
        };
        let (Some(links), Some(target)) = (links.checked_sub(1), target) else { return true };
        // A relative target is relative to the link's folder; an absolute one replaces it.
        let next = lexical(&at.parent().unwrap_or(&at).join(target).join(rest.as_path()));
        return refused_spelling(&next.to_string_lossy(), roots).is_some() || link_refused(&next, roots, links, look);
    }
    false
}

/// `p` with its `.` and `..` taken by name, as Windows takes them.
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// Whether `path` lies in one of `roots` once made absolute with its `.` and `..` taken by name and
/// its longest leading part that exists resolved, so links on the way are followed as the roots'
/// were (`/tmp/work/x` is inside root `/private/tmp/work`, a link out of a root leads out of it).
fn inside_as_far_as_it_exists(path: &Path, roots: &[PathBuf]) -> bool {
    let Ok(abs) = std::path::absolute(path) else { return false };
    let folded = lexical(&abs);
    let mut rest: Vec<&OsStr> = Vec::new();
    let mut at = folded.as_path();
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
        if device_name(&n) || n.contains(':') {
            return Err(format!("`{n}` is a device or stream name, not a file"));
        }
    }
    Ok(())
}

/// Whether Windows opens a device for file or folder name `n`: `CON`, `PRN`, `AUX`, `NUL`,
/// `CONIN$`, `CONOUT$`, `COM1`…`COM9` and `LPT1`…`LPT9` (also with `¹`, `²` or `³`), in any case,
/// and with anything after a dot or a colon or trailing spaces (`nul.txt`, `COM1 .png`, `con:x`).
fn device_name(n: &str) -> bool {
    let stem = n.split(['.', ':']).next().unwrap_or_default().trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.chars().count() == 4
            && stem.chars().last().is_some_and(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³')))
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

/// Why this thread's work may not reach the network now, if it may not (#1095): the automation
/// roots keep an agent to the folders it was given, so while they confine it nothing is published,
/// hosted, joined or fetched from a server either. The engine reaches the network only through the
/// live link (`crate::live::Lives`), which asks this first.
pub fn network_refused() -> Option<String> {
    current().map(|_| NETWORK_REFUSED.to_string())
}

/// What a confined command that would reach the network says.
pub const NETWORK_REFUSED: &str =
    "automation roots are set: commands don't reach the network (publishing, live sessions, team libraries and plugin directories are off)";

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

/// `bytes` for a person: "300 bytes", "12 MB", "6.5 GB".
fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1 << 20 => format!("{b} bytes"),
        b if b < 1 << 30 => format!("{} MB", b.div_ceil(1 << 20)),
        b => format!("{:.1} GB", b as f64 / f64::from(1u32 << 30)),
    }
}

/// The bytes of file `path`, at most `max` of them. A folder, a device, a pipe or a file larger
/// than `max` is refused before anything is read, so a path a caller gives can't fill memory or
/// wait forever (#1114). Every whole-file read of a path goes through here.
pub fn read_capped(path: impl AsRef<Path>, max: u64) -> Result<Vec<u8>> {
    use std::io::Read as _;
    let path = path.as_ref();
    let p = allow(path, Access::Read)?;
    let refuse = |why: String| EngineError::Other(format!("{}: {why}", path.display()));
    // Looked at before opening: opening a pipe waits for whoever writes it.
    let meta = std::fs::metadata(&p).map_err(io(path))?;
    if meta.is_dir() {
        return Err(refuse("is a folder, not a file".into()));
    }
    if !meta.is_file() {
        return Err(refuse("isn't a regular file (a device or a pipe?), so it isn't read".into()));
    }
    if meta.len() > max {
        return Err(refuse(format!("is {}, more than the {} allowed here", size(meta.len()), size(max))));
    }
    let f = std::fs::File::open(p).map_err(io(path))?;
    // What was opened, not what was looked at: the path could have been swapped in between.
    let meta = f.metadata().map_err(io(path))?;
    if !meta.is_file() {
        return Err(refuse("isn't a regular file (a device or a pipe?), so it isn't read".into()));
    }
    if meta.len() > max {
        return Err(refuse(format!("is {}, more than the {} allowed here", size(meta.len()), size(max))));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    f.take(max.saturating_add(1)).read_to_end(&mut bytes).map_err(io(path))?;
    // It grew after it was looked at.
    if bytes.len() as u64 > max {
        return Err(refuse(format!("grew past the {} allowed here while it was read", size(max))));
    }
    Ok(bytes)
}

/// File `path` opened for reading.
pub fn open(path: impl AsRef<Path>) -> Result<std::fs::File> {
    let path = path.as_ref();
    std::fs::File::open(allow(path, Access::Read)?).map_err(io(path))
}

/// What the file system says about `path` itself (a link is not followed).
pub fn link_metadata(path: impl AsRef<Path>) -> Result<std::fs::Metadata> {
    let path = path.as_ref();
    std::fs::symlink_metadata(allow(path, Access::Read)?).map_err(io(path))
}

/// What the file system says about `path` (links followed).
pub fn metadata(path: impl AsRef<Path>) -> Result<std::fs::Metadata> {
    let path = path.as_ref();
    std::fs::metadata(allow(path, Access::Read)?).map_err(io(path))
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
mod tests;
