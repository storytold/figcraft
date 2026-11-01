//! Recovery copies, so a crash, a forced quit or a power cut loses at most a few seconds of work.
//!
//! While the app runs, each document with unsaved changes is copied to `Recovery/<pid>/<uid>.figcraft`
//! in the config folder, in the background, at most every [`INTERVAL`] seconds, and the copy goes
//! once the document is saved or closed. The app holds a lock on its folder. At startup, the folders
//! whose lock is free belong to runs that ended without closing: their copies open again as
//! unsaved documents (`file.recover`). Each such copy **stays until this run has its own copy of
//! the document** (or it is saved or closed): until then it is the only copy on disk, and a run
//! that can't keep copies of its own leaves it for the next start. Then it goes, and its folder
//! with its last copy. Closing the app normally removes its own.
//!
//! A document that is one of your online files has a note beside its copy (`<uid>.online`, written
//! after the copy it describes): the server, the file's id, and who the app was in the file with
//! the last change of the server's it had; never a token. The next start joins that file again
//! as its owner (`collab.recoverFile`) and **keeps the copy until the outcome is known**: the
//! server has everything in it (the copy goes), the copy's own unconfirmed changes were put on
//! top of the server's file and sent, the copy was opened beside the file because both had
//! changed, or the file couldn't be joined and the copy was opened as any other. Whatever a copy
//! became is copied into this run's folder before the old copy is removed. Without the note, or
//! when the file is no longer recorded, the copy opens as any other.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use figcraft_engine::branches::{needs_copy, recovery_copy, stamp};
use figcraft_engine::{DocState, Session};
use figcraft_format::{Branch, Version};
use serde_json::json;

/// Seconds between copies of a document that keeps changing.
pub const INTERVAL: f64 = 10.0;

const LOCK: &str = "lock";

/// The app's recovery, shared by its frame loop and its services (the panic path).
pub type Shared = Arc<Mutex<Option<Recovery>>>;

pub fn lock(r: &Shared) -> MutexGuard<'_, Option<Recovery>> {
    r.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct Recovery {
    dir: PathBuf,
    /// Held while the app runs: a free lock means the run that wrote the folder is gone.
    _lock: File,
    /// What was copied of each document, and the write in flight.
    written: HashMap<u64, Copy>,
    /// When (UI clock) the next copies are due.
    next: f64,
    /// Copies of online files from earlier runs, kept until their outcome is known.
    waiting: Vec<Waiting>,
}

/// The state of the copy of one document: only a write that succeeded counts as a copy.
#[derive(Default)]
struct Copy {
    /// The [`stamp`] whose copy was written.
    done: Option<Vec<u64>>,
    /// The write in flight: the stamp it copies, and whether it succeeded once it ends.
    flight: Option<(Vec<u64>, JoinHandle<bool>)>,
}

impl Copy {
    /// Take in the result of the write in flight once it ended; a failed one leaves its revision uncopied.
    fn reap(&mut self) {
        if self.flight.as_ref().is_some_and(|f| f.1.is_finished()) {
            self.join();
        }
    }

    /// Wait for the write in flight and take in its result.
    fn join(&mut self) {
        if let Some((stamp, h)) = self.flight.take()
            && h.join().unwrap_or(false)
        {
            self.done = Some(stamp);
        }
    }

    /// Whether `stamp` has neither been copied nor is being copied.
    fn pending(&self, stamp: &[u64]) -> bool {
        self.done.as_deref() != Some(stamp) && self.flight.as_ref().is_none_or(|f| f.0 != stamp)
    }
}

/// A copy from an earlier run, kept until what it became is safe.
pub struct Waiting {
    until: Until,
    copy: PathBuf,
    /// The lock of the folder it is in: no other start takes the copy meanwhile (none for a copy
    /// left directly in the root by older versions).
    _lock: Option<Arc<File>>,
}

/// What a kept copy waits for.
enum Until {
    /// One of your online files, joined again as this session: its outcome.
    Session(u64),
    /// Opened as document `uid`: this run's own copy of it (or its save or close).
    Copied(u64),
}

/// What [`recover_all`] brought back.
#[derive(Default)]
pub struct Recovered {
    /// Documents opened from their copies.
    pub copies: usize,
    /// The copies kept until what they became is safe ([`Recovery::adopt`]).
    pub waiting: Vec<Waiting>,
}

impl Recovery {
    /// Take this run's folder under `root` (`None` when it can't be created or locked: no copies).
    /// A crashed run with the same process id may still hold its folder (its copies kept): then
    /// another name is taken.
    pub fn start(root: &Path) -> Option<Self> {
        let pid = std::process::id();
        (0..10).find_map(|n| {
            let dir = root.join(if n == 0 { pid.to_string() } else { format!("{pid}-{n}") });
            std::fs::create_dir_all(&dir).ok()?;
            let lock = lock_folder(&dir).ok()?;
            Some(Self { dir, _lock: lock, written: HashMap::new(), next: 0.0, waiting: Vec::new() })
        })
    }

    /// Keep earlier runs' copies until what they became is safe.
    pub fn adopt(&mut self, waiting: Vec<Waiting>) {
        self.waiting.extend(waiting);
    }

    /// Copy the documents that changed since their last copy and drop the copies of documents that
    /// were saved or closed. Returns how long until it needs to run again (`None`: nothing pending,
    /// an idle window can sleep).
    pub fn tick(&mut self, s: &Session, now: f64) -> Option<f64> {
        self.written.values_mut().for_each(Copy::reap);
        self.settle(s);
        // A join on its way settles within its timeouts: look again soon. A copy waiting for this
        // run's own is looked at again with the next copies.
        let joining = self.waiting.iter().any(|w| matches!(w.until, Until::Session(_)));
        let settling = if joining { Some(0.5) } else { (!self.waiting.is_empty()).then_some(INTERVAL) };
        if now < self.next {
            return self.pending(s).then_some(self.next - now).or(settling);
        }
        self.write(s, false);
        self.next = now + INTERVAL;
        // A copy that failed is tried again at the next turn, without another edit.
        settling.or(self.pending(s).then_some(INTERVAL))
    }

    /// Whether a document with unsaved changes has a revision that is neither copied nor being copied.
    fn pending(&self, s: &Session) -> bool {
        s.docs.iter().any(|d| needs_copy(s, d) && self.written.get(&d.uid).is_none_or(|w| w.pending(&stamp(s, d))))
    }

    /// Remove earlier runs' copies once what they became is safe: copied here (the write
    /// succeeded), saved or closed. An online file's copy goes once its outcome is known and every
    /// document is safe, since which of them it became isn't known.
    pub fn settle(&mut self, s: &Session) {
        let joined = |w: &Waiting| matches!(w.until, Until::Session(id) if !s.live.recovering(id));
        if self.waiting.iter().any(joined) {
            self.write(s, true);
        }
        let all_safe = s.docs.iter().all(|d| self.safe(s, d));
        let (done, waiting): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut self.waiting).into_iter().partition(|w| match w.until {
            Until::Session(_) => joined(w) && all_safe,
            Until::Copied(uid) => s.docs.iter().find(|d| d.uid == uid).is_none_or(|d| self.safe(s, d)),
        });
        self.waiting = waiting;
        done.into_iter().for_each(remove_kept);
    }

    /// Whether document `d`'s work is on disk: saved (or nothing to keep), or copied here (a write
    /// that succeeded) with no write in flight.
    fn safe(&self, s: &Session, d: &DocState) -> bool {
        !needs_copy(s, d) || self.written.get(&d.uid).is_some_and(|w| w.done.is_some() && w.flight.is_none())
    }

    /// Copy every document with unsaved changes; `wait`: before returning (after a panic).
    pub fn write(&mut self, s: &Session, wait: bool) {
        for d in &s.docs {
            let path = self.dir.join(format!("{}.figcraft", d.uid));
            let note = path.with_extension(ONLINE);
            if !needs_copy(s, d) {
                if let Some(mut w) = self.written.remove(&d.uid) {
                    // A write still running would bring the copy back after it was removed.
                    w.join();
                    let _ = std::fs::remove_file(&path);
                    let _ = std::fs::remove_file(&note);
                }
                continue;
            }
            let entry = self.written.entry(d.uid).or_default();
            // An emergency save must copy the newest revision: let the write in flight finish first.
            if wait {
                entry.join();
            } else {
                entry.reap();
            }
            // A write still in flight (for any revision) is waited for; a failed one is tried again.
            let stamp = stamp(s, d);
            if entry.done.as_ref() == Some(&stamp) || entry.flight.is_some() {
                continue;
            }
            let (doc, versions, branches) = recovery_copy(s, d);
            // Which online file it is and what of the server's this very copy has (a line each:
            // the server, the file's id, then the session, client id, last change and the secret the
            // client id resumes with), while it
            // is one. Written after the copy, never before: a note must not promise more of the
            // server's changes than its copy holds.
            let online =
                s.online_of(d.uid).map(|(server, file, at)| format!("{server}\n{file}\n{} {} {} {}", at.session, at.client_id, at.seq, at.secret));
            let job = move || {
                if !write_atomic(&path, &doc, &versions, &branches) {
                    return false;
                }
                let _ = match online {
                    Some(text) => std::fs::write(&note, text),
                    None => std::fs::remove_file(&note),
                };
                true
            };
            // The stamp counts as copied only once its write succeeded.
            if wait {
                if job() {
                    entry.done = Some(stamp);
                }
            } else {
                entry.flight = std::thread::Builder::new().name("recovery".into()).spawn(job).ok().map(|h| (stamp, h));
            }
        }
        // Closed documents.
        let open: Vec<u64> = s.docs.iter().map(|d| d.uid).collect();
        let dir = &self.dir;
        self.written.retain(|uid, w| {
            let keep = open.contains(uid);
            if !keep {
                w.join();
                let _ = std::fs::remove_file(dir.join(format!("{uid}.figcraft")));
                let _ = std::fs::remove_file(dir.join(format!("{uid}.{ONLINE}")));
            }
            keep
        });
    }

    /// Test hook: pretend a write of `uid` at `revision` is in flight in `writer`.
    #[cfg(test)]
    pub fn hold(&mut self, uid: u64, revision: u64, writer: JoinHandle<bool>) {
        self.written.insert(uid, Copy { done: None, flight: Some((vec![revision], writer)) });
    }

    /// Quitting: a normal quit ([`finish`](Self::finish)) when the unsaved-changes question was
    /// `asked` or nothing is unsaved. Otherwise (macOS's Quit menu ends the app without asking) the
    /// unsaved documents are copied and the copies stay for the next start, as after a crash.
    pub fn quit(mut self, s: &Session, asked: bool) {
        if asked || !s.docs.iter().any(|d| needs_copy(s, d)) {
            return self.finish();
        }
        self.write(s, true);
        self.settle(s);
    }

    /// A normal quit: the documents were saved or their changes discarded on purpose. (Earlier
    /// runs' copies still waiting for their outcome stay for the next start.)
    pub fn finish(self) {
        let Self { dir, _lock, written, waiting, .. } = self;
        // The documents earlier runs' copies became were saved or discarded too.
        waiting.into_iter().filter(|w| matches!(w.until, Until::Copied(_))).for_each(remove_kept);
        // Windows removes a folder only once nothing in it is open: copies being written first.
        for (_, h) in written.into_values().filter_map(|w| w.flight) {
            let _ = h.join();
        }
        drop(_lock);
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Remove a kept copy and its note, and its folder with its last copy (while another is kept
/// there, its lock is held).
fn remove_kept(w: Waiting) {
    let _ = std::fs::remove_file(&w.copy);
    let _ = std::fs::remove_file(w.copy.with_extension(ONLINE));
    let last = w._lock.as_ref().is_some_and(|l| Arc::strong_count(l) == 1);
    let dir = w.copy.parent().map(Path::to_path_buf).filter(|_| last);
    drop(w);
    if let Some(dir) = dir {
        let _ = std::fs::remove_file(dir.join(LOCK));
        let _ = std::fs::remove_dir(dir);
    }
}

/// The extension of the note beside the copy of a document that is one of your online files.
const ONLINE: &str = "online";

/// How long the startup waits, once, for folder locks that may free themselves (#334).
#[cfg(unix)]
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_millis(100);

/// Lock the folder `dir` (`WouldBlock` while another process holds it).
fn lock_folder(dir: &Path) -> Result<File, std::fs::TryLockError> {
    let lock = File::options().write(true).create(true).truncate(false).open(dir.join(LOCK)).map_err(std::fs::TryLockError::Error)?;
    lock.try_lock()?;
    Ok(lock)
}

/// The copy at `file` is of one of your online files: join that file again as its owner, keeping
/// the copy to compare with what arrives. The session, or `None` when the copy isn't one (or its
/// file isn't recorded any more, or is open already): then it opens as any other copy.
fn rejoin_online(file: &Path, s: &mut Session) -> Option<u64> {
    let note = std::fs::read_to_string(file.with_extension(ONLINE)).ok()?;
    let mut lines = note.lines().map(str::trim);
    let (server, id, at) = (lines.next()?, lines.next()?, lines.next()?);
    let mut at = at.split(' ').map(|n| n.parse::<u64>().ok());
    let (session, client, seq) = (at.next()??, at.next()??, at.next()??);
    // Absent in notes written before ids had secrets.
    let secret = at.next().flatten().unwrap_or(0);
    let params =
        json!({"path": file.to_string_lossy(), "server": server, "file": id, "session": session, "client": client, "seq": seq, "secret": secret});
    s.execute("collab.recoverFile", &params).ok()?.get("session")?.as_u64()
}

/// Write `doc` with its version history and branches to `path` through a temporary file; false
/// when it couldn't be written (the copy before it is still there).
fn write_atomic(path: &Path, doc: &figcraft_engine::doc::Document, versions: &[Version], branches: &[Branch]) -> bool {
    let Ok(bytes) = figcraft_format::save_bytes_with_history(doc, None, versions, branches) else { return false };
    let tmp = path.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        let Ok(mut f) = opts.open(&tmp) else { return false };
        if std::io::Write::write_all(&mut f, &bytes).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return false;
        }
    }
    #[cfg(not(unix))]
    {
        if std::fs::write(&tmp, bytes).is_err() {
            return false;
        }
    }
    std::fs::rename(&tmp, path).is_ok()
}

/// [`recover_all`], for a start that keeps nothing waiting: how many documents came back as
/// unsaved copies.
#[cfg(test)]
pub fn recover(root: &Path, s: &mut Session) -> usize {
    recover_all(root, s).copies
}

/// Open the copies left by runs that ended without closing (folders under `root` whose lock is
/// free, and copies directly in `root` from older versions) as unsaved documents. Each is kept
/// until this run has its own copy of the document ([`Recovered::waiting`]); a copy of one of your
/// online files, while its file is joined again. A copy that can't be read stays where it is.
pub fn recover_all(root: &Path, s: &mut Session) -> Recovered {
    let mut out = Recovered::default();
    let Ok(entries) = std::fs::read_dir(root) else { return out };
    let mut open = |file: &Path, lock: Option<&Arc<File>>, s: &mut Session| {
        if file.extension().is_none_or(|e| e != "figcraft") {
            return;
        }
        // One of your online files comes back from its server, its copy kept until then.
        if let Some((session, lock)) = lock.and_then(|l| Some((rejoin_online(file, s)?, l))) {
            out.waiting.push(Waiting { until: Until::Session(session), copy: file.to_path_buf(), _lock: Some(lock.clone()) });
            return;
        }
        let before: Vec<u64> = s.docs.iter().map(|d| d.uid).collect();
        if s.execute("file.recover", &json!({"path": file.to_string_lossy()})).is_ok() {
            out.copies += 1;
            if let Some(uid) = s.docs.iter().map(|d| d.uid).find(|u| !before.contains(u)) {
                out.waiting.push(Waiting { until: Until::Copied(uid), copy: file.to_path_buf(), _lock: lock.cloned() });
            }
        }
    };
    let (mut free, mut held) = (Vec::new(), Vec::new());
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_dir() {
            open(&path, None, s);
            continue;
        }
        // Still running (another window of the app), or not ours to take.
        match lock_folder(&path) {
            Ok(lock) => free.push((path, lock)),
            Err(std::fs::TryLockError::WouldBlock) => held.push(path),
            Err(_) => {}
        }
    }
    // A child between its `fork` and its `exec` shares the run's open descriptors, and a `flock`
    // lasts while any of them is open, so an ended run's folder can look held for a moment. Wait
    // once, however many folders are held, and try those again (#334). Windows doesn't pass the
    // lock to child processes.
    #[cfg(unix)]
    if !held.is_empty() {
        std::thread::sleep(LOCK_WAIT);
        free.extend(held.into_iter().filter_map(|path| lock_folder(&path).ok().map(|lock| (path, lock))));
    }
    for (path, lock) in free {
        let lock = Arc::new(lock);
        let files: Vec<PathBuf> = std::fs::read_dir(&path).into_iter().flatten().flatten().map(|f| f.path()).collect();
        for f in &files {
            if f.extension().is_some_and(|e| e == "tmp") {
                // A copy cut off mid-write; the one before it is still there.
                let _ = std::fs::remove_file(f);
            } else {
                open(f, Some(&lock), s);
            }
        }
        // The notes go with their copies (one whose copy stays, stays with it).
        for f in files.iter().filter(|f| f.extension().is_some_and(|e| e == ONLINE) && !f.with_extension("figcraft").exists()) {
            let _ = std::fs::remove_file(f);
        }
        // Empty now unless a copy couldn't be read, or is kept (its lock held).
        if Arc::strong_count(&lock) == 1 {
            drop(lock);
            let _ = std::fs::remove_file(path.join(LOCK));
            let _ = std::fs::remove_dir(&path);
        }
    }
    out
}
