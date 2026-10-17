//! The data directory: the files a server keeps, their share links and the server key, and the
//! [`Backend`] the hub serves them through.
//!
//! ```text
//! <data>/LOCK                    held while a server (or a command changing the directory) uses it
//! <data>/server.json             where the running server listens (for the command line)
//! <data>/server.key              the server key: managing files (publish, links, removal)
//! <data>/files/<id>/meta.json    name, created, updated, sequence number
//! <data>/files/<id>/links.json   share links: id, access, label, created, SHA-256 of the token,
//!                                and a join code's hash and wrapped token (see `codes`)
//! <data>/files/<id>/snapshot.bin the session state at the last compaction
//! <data>/files/<id>/log.bin      what changed since (see `disk`)
//! <data>/files/<id>/library-<version>.bin  its team library, when one was published (a `.figcraft`
//!                                archive; `library.bin` from servers before versioned names)
//! <data>/files/<id>/usage.json   how other files use that library, as their apps report it
//! <data>/plugins/                the plugin directory (see `directory`)
//! <data>/blocked.json            addresses the operator blocked
//! <data>/reports.json            reports people sent about files
//! ```
//!
//! A server open to the public ([`HostingConfig::open`]) also keeps files anyone published
//! without the key: such a file has an owner token (stored as its hash) that manages it, the
//! address it came from (for the per-address caps and the operator), and it is deleted once nobody
//! has opened it for [`HostingConfig::idle_secs`].
//!
//! An open file appends its server's journal to its log; the hub has the store make the appends
//! durable before anyone hears of them. After [`StoreConfig::compact_after`] entries or
//! [`StoreConfig::compact_bytes`] of log, when everyone has left, and when the server stops, the
//! state is written as the next snapshot and the log starts over. Link tokens are stored only as
//! hashes and compared in constant time. Whatever is damaged at startup is skipped and reported,
//! never deleted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Cursor, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use figcraft_collab::message::{
    Access, Admin, AdminReply, ClientMsg, FileInfo, FileRow, FileUsage, HostInfo, LibraryReply, LibraryReq, LinkInfo, MAX_OWNED, NewLink, OwnedFile,
    PluginReply, PluginReq, ReportInfo, Usage,
};
use figcraft_collab::{Change, Replica, Server, same_secret};
use figcraft_collab_net::hub::{Admission, Backend, Closing, Handled, OpenFile, ip_key};
use figcraft_collab_net::token;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::abuse::{Abuse, AbuseConfig, Action};
use crate::codes::{self, Sealed};
use crate::directory::{Directory, DirectoryConfig};
use crate::disk::{self, LIBRARY, LINKS, LOG, LOG_HEADER, META, SNAPSHOT, USAGE};

pub const LOCK: &str = "LOCK";
pub const KEY: &str = "server.key";
pub const FILES: &str = "files";
pub const BLOCKED: &str = "blocked.json";
pub const REPORTS: &str = "reports.json";

/// Most addresses blocked, and most reports kept (the oldest go first).
const MAX_BLOCKED: usize = 100_000;
const MAX_REPORTS: usize = 1_000;
/// Most join codes remembered as no longer valid.
const MAX_GONE_CODES: usize = 10_000;
/// What typing a join code that opens nothing is answered with, by why.
pub const NO_CODE: &str = "No session with that code";
pub const CODE_GONE: &str = "That code has expired or its sharing was stopped";
pub const CODES_BUSY: &str = "Join codes are paused on this server for a while (too many wrong ones were tried): join with the invite link";
/// Longest report reason kept (characters).
const MAX_REASON: usize = 500;
/// How often idle files are looked for (seconds), besides at startup.
const SWEEP_EVERY: u64 = 600;
/// A file over its cap is compacted to make room at most this often (seconds): a compaction
/// rewrites the whole file on the hub thread.
const FORCED_COMPACTION_EVERY: u64 = 60;
/// Most problems kept for the operator (the oldest go first).
const MAX_PROBLEMS: usize = 200;

/// Why a download is turned away (an address's share, or everyone's, is used up for now).
pub const BUSY_DOWNLOADS: &str = "too many downloads from your address or from everyone right now: try again in a minute";

/// Why publishing without the key is refused on a server that isn't open to the public.
pub const KEY_ONLY: &str = "this server takes files only from people who have its key: add ?key=… to the address (its operator has the key)";

/// Why a file published without the key can't hold a team library on a public server (a library
/// is a whole archive checked on the server; strangers' files don't get that).
pub const NO_PUBLIC_LIBRARY: &str = "on this public server only files published with the server key can hold a team library";

/// Most files whose usage of one library is kept (the least recently changed go first), and most
/// components one file's report keeps.
const MAX_USERS: usize = 200;
const MAX_REPORTED: usize = 1_000;
/// Largest `usage.json` read.
const MAX_USAGE_FILE: u64 = 4 << 20;
/// A library's usage reports are kept in memory and written at most this often (seconds), and
/// when the server stops.
const USAGE_WRITE_EVERY: u64 = 60;
/// Bytes of team libraries served a second, everyone together, and at once.
const LIBRARY_RATE: f64 = (32u64 << 20) as f64;
const LIBRARY_BURST: f64 = (512u64 << 20) as f64;
// A /48 (four addresses' worth) may download at most an eighth of everyone's budgets.
const _: () = assert!(crate::abuse::DOWNLOAD_RATE * 4.0 <= LIBRARY_RATE / 8.0 && crate::abuse::DOWNLOAD_BURST * 4.0 <= LIBRARY_BURST / 8.0);

/// What the store enforces.
#[derive(Clone, Debug, PartialEq)]
pub struct StoreConfig {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_links: usize,
    pub compact_after: usize,
    pub compact_bytes: u64,
    pub core: figcraft_collab::Limits,
    pub directory: DirectoryConfig,
    pub hosting: HostingConfig,
    /// Below this much free space on the data directory's disk, changes and new files are
    /// refused (with the reason) rather than half written.
    pub min_free_bytes: u64,
}

/// Hosting for the public (see the server's settings).
#[derive(Clone, Debug, PartialEq)]
pub struct HostingConfig {
    /// Anyone may publish without the key.
    pub open: bool,
    pub operator: String,
    pub terms: String,
    /// Files published without the key are deleted after this long unopened (0: kept).
    pub idle_secs: u64,
    pub max_file_mb: u64,
    /// Publishing without the key stops while the files take this much in all.
    pub max_total_bytes: u64,
    /// Files published without the key per address, and their bytes.
    pub max_files_per_ip: usize,
    pub max_bytes_per_ip: u64,
    pub abuse: AbuseConfig,
}

impl Default for HostingConfig {
    fn default() -> Self {
        Self {
            open: false,
            operator: String::new(),
            terms: String::new(),
            idle_secs: 0,
            max_file_mb: 64,
            max_total_bytes: u64::MAX,
            max_files_per_ip: usize::MAX,
            max_bytes_per_ip: u64::MAX,
            abuse: AbuseConfig { open: false, connects_per_minute: 30, publishes_per_hour: 10, messages_per_second: 200 },
        }
    }
}

/// A file's description (`meta.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Meta {
    name: String,
    created: u64,
    updated: u64,
    #[serde(default)]
    seq: u64,
    /// The team library published under the file, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    library: Option<LibraryMeta>,
    /// Published without the key: SHA-256 of its owner token (hex), and the address it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ip: Option<String>,
    /// When someone last opened it (seconds since 1970).
    #[serde(default)]
    opened: u64,
}

impl Meta {
    /// When it was last used: made, changed or opened.
    fn used(&self) -> u64 {
        self.created.max(self.updated).max(self.opened)
    }
}

/// What a file's team library is ([`library_file`] holds it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct LibraryMeta {
    id: String,
    name: String,
    version: u64,
}

/// A share link (`links.json`): the token itself is never stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Link {
    id: String,
    access: Access,
    label: String,
    created: u64,
    /// SHA-256 of the token, hex.
    hash: String,
    /// Its join code, when one was handed out (never the code itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<Sealed>,
}

impl Link {
    fn info(&self, file: &str) -> LinkInfo {
        LinkInfo { id: self.id.clone(), file: file.to_string(), access: self.access, label: self.label.clone(), created: self.created }
    }
}

/// An open file's log.
struct Log {
    generation: u64,
    file: File,
    /// Records not written yet (written and synced at the next flush).
    pending: Vec<u8>,
    /// Entries and bytes since the last compaction.
    entries: usize,
    bytes: u64,
    /// Changed since the last compaction.
    dirty: bool,
}

struct Entry {
    meta: Meta,
    links: Vec<Link>,
    /// Open: its log.
    log: Option<Log>,
    /// Its snapshot's and log's bytes on disk (what was written; pending records not yet).
    bytes: u64,
    /// When it was last compacted to make room (seconds since 1970).
    forced: u64,
}

/// A library's usage reports in memory, and whether (and when) they were written.
struct UsageCache {
    list: Vec<FileUsage>,
    dirty: bool,
    written: u64,
}

/// The data directory in use (it holds the directory's lock until dropped).
pub struct Store {
    root: PathBuf,
    cfg: StoreConfig,
    key: String,
    files: BTreeMap<String, Entry>,
    /// What was skipped at startup or found damaged since, for the operator.
    problems: Vec<String>,
    /// The plugin directory.
    directory: Directory,
    /// What addresses may do how often, the ones blocked, and the reports kept.
    abuse: Abuse,
    /// Team library bytes served, everyone together.
    served: figcraft_collab_net::net::Bucket,
    /// Usage reports by library file.
    usage: BTreeMap<String, UsageCache>,
    blocked: BTreeSet<IpAddr>,
    reports: Vec<ReportInfo>,
    /// Hashes of join codes that no longer open anything (their link was revoked, their file
    /// removed or expired, or a newer code took their place): typing one says so.
    // shortcut: in memory only, so after a restart such a code reads as unknown; keep them in the
    // data directory if people ask why their code "never existed".
    gone_codes: VecDeque<String>,
    /// When idle files are looked for next (seconds since 1970).
    next_sweep: u64,
    /// Compactions so far (tests count them).
    compactions: u64,
    /// Open files closed by a compaction cut short (their snapshot replaced, their log not):
    /// reported at the next flush, so nothing appended after it is confirmed.
    cut_short: Vec<String>,
    /// The free space last seen on the data directory's disk, and when.
    free: Option<(Instant, Option<u64>)>,
    _lock: File,
}

/// Seconds since 1970.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// SHA-256 of a token, hex.
pub fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A file or link id as this store makes them (and nothing a path could misuse).
fn valid_id(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A name others may see: trimmed, no control characters, at most 256 characters.
fn clean_name(s: &str) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(256).collect()
}

/// Take the data directory's lock: `None` when someone else holds it.
pub fn lock(root: &Path) -> Result<Option<File>, String> {
    // Only the server's account reads what it keeps (files, addresses, reports, the key).
    let mut dirs = fs::DirBuilder::new();
    dirs.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut dirs, 0o700);
    dirs.create(root).map_err(|e| format!("can't create the data directory {}: {e}", root.display()))?;
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(LOCK))
        .map_err(|e| format!("can't open {}: {e}", root.join(LOCK).display()))?;
    match f.try_lock() {
        Ok(()) => Ok(Some(f)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(format!("can't lock the data directory: {e}")),
    }
}

/// Read the server key, or make one.
fn load_key(root: &Path) -> Result<String, String> {
    let path = root.join(KEY);
    match fs::read_to_string(&path) {
        Ok(k) if valid_id(k.trim()) && k.trim().len() >= 16 => Ok(k.trim().to_string()),
        Ok(_) => Err(format!("{} isn't a server key (delete it to make a new one)", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => write_key(root),
        Err(e) => Err(format!("can't read {}: {e}", path.display())),
    }
}

/// Make a new server key (only the operator's account can read it on Unix).
fn write_key(root: &Path) -> Result<String, String> {
    let key = token::secret()?;
    let path = root.join(KEY);
    disk::write_atomic_private(&path, format!("{key}\n").as_bytes()).map_err(|e| format!("can't write {}: {e}", path.display()))?;
    Ok(key)
}

impl Store {
    /// Use the data directory `root` (made if missing). Fails when another server or command holds
    /// it. Files that can't be read are skipped and listed in [`problems`](Self::problems).
    pub fn open(root: &Path, cfg: StoreConfig) -> Result<Store, String> {
        Self::try_open(root, cfg)?.ok_or_else(|| format!("another figcraft-server (or a command) is using {}", root.display()))
    }

    /// [`open`](Self::open), or `None` when someone else holds the directory.
    pub fn try_open(root: &Path, cfg: StoreConfig) -> Result<Option<Store>, String> {
        let Some(lock) = lock(root)? else { return Ok(None) };
        let key = load_key(root)?;
        let dir = root.join(FILES);
        fs::create_dir_all(&dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
        let (directory, startup_problems) = Directory::open(root, cfg.directory);
        let abuse = Abuse::new(cfg.hosting.abuse);
        let mut store = Store {
            root: root.to_path_buf(),
            cfg,
            key,
            files: BTreeMap::new(),
            problems: Vec::new(),
            directory,
            abuse,
            served: figcraft_collab_net::net::Bucket::new(LIBRARY_RATE, LIBRARY_BURST),
            usage: BTreeMap::new(),
            blocked: BTreeSet::new(),
            reports: Vec::new(),
            gone_codes: VecDeque::new(),
            next_sweep: 0,
            compactions: 0,
            cut_short: Vec::new(),
            free: None,
            _lock: lock,
        };
        for problem in startup_problems {
            store.note(problem);
        }
        store.scan(&dir);
        store.blocked = store.read_list::<String>(BLOCKED).iter().filter_map(|s| s.parse().ok()).map(ip_key).collect();
        store.reports = store.read_list(REPORTS);
        store.sweep();
        Ok(Some(store))
    }

    /// A list kept at the root (missing: empty; damaged: empty and reported, kept until the next
    /// change writes over it).
    fn read_list<T: serde::de::DeserializeOwned>(&mut self, name: &str) -> Vec<T> {
        let path = self.root.join(name);
        if !path.exists() {
            return Vec::new();
        }
        match disk::read_capped(&path, 16 << 20).and_then(|b| serde_json::from_slice(&b).map_err(|e| format!("{name} is damaged ({e})"))) {
            Ok(list) => list,
            Err(e) => {
                self.note(e);
                Vec::new()
            }
        }
    }

    fn scan(&mut self, dir: &Path) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => return self.note(format!("can't list {}: {e}", dir.display())),
        };
        let mut ids: Vec<(String, PathBuf)> = entries.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
        ids.sort();
        for (id, path) in ids {
            if !path.is_dir() {
                continue;
            }
            if !valid_id(&id) {
                self.note(format!("skipped {}: not a file folder of this server", path.display()));
                continue;
            }
            if self.files.len() >= self.cfg.max_files {
                self.note(format!("skipped file {id}: over the {}-file cap", self.cfg.max_files));
                continue;
            }
            match self.check(&path) {
                Ok(entry) => {
                    self.files.insert(id, entry);
                }
                Err(why) => self.note(format!("skipped file {id}: {why}")),
            }
        }
    }

    /// A file folder's description and links, after checking its snapshot and log. Leftover
    /// temporary files (writes cut short) are removed.
    fn check(&self, path: &Path) -> Result<Entry, String> {
        for name in [SNAPSHOT, LOG, META, LINKS] {
            let tmp = disk::temp_of(&path.join(name));
            if tmp.exists() {
                let _ = fs::remove_file(&tmp);
            }
        }
        let meta: Meta = serde_json::from_slice(&disk::read_capped(&path.join(META), 1 << 20)?).map_err(|e| format!("{META} is damaged ({e})"))?;
        let links: Vec<Link> =
            serde_json::from_slice(&disk::read_capped(&path.join(LINKS), 16 << 20)?).map_err(|e| format!("{LINKS} is damaged ({e})"))?;
        let generation = disk::check_snapshot(&disk::read_capped(&path.join(SNAPSHOT), self.snapshot_cap())?)?;
        // A damaged log only loses its end; it is reported when the file opens.
        if let Ok(bytes) = disk::read_capped(&path.join(LOG), self.cfg.max_file_bytes)
            && let Ok(log) = disk::read_log(&bytes)
            && log.generation != generation
        {
            log::info!("file at {}: its log isn't of its snapshot's generation; it is replayed over it when the file opens", path.display());
        }
        let bytes = [SNAPSHOT, LOG].iter().filter_map(|n| fs::metadata(path.join(n)).ok()).map(|m| m.len()).sum();
        Ok(Entry { meta, links, log: None, bytes, forced: 0 })
    }

    /// The server key (for managing files).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Replace the server key; the new one.
    pub fn rotate_key(&mut self) -> Result<String, String> {
        self.key = write_key(&self.root)?;
        Ok(self.key.clone())
    }

    /// The plugin directory.
    pub fn directory(&mut self) -> &mut Directory {
        &mut self.directory
    }

    /// What was skipped or found damaged.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// How many files are kept.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// No files are kept.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.join(FILES).join(id)
    }

    fn entry(&mut self, id: &str) -> Result<&mut Entry, String> {
        self.files.get_mut(id).ok_or_else(|| "this file isn't on the server (it may have been removed)".to_string())
    }

    /// A file's bytes on disk (its snapshot and log), with the log records not written yet.
    fn bytes(&self, id: &str) -> u64 {
        self.files.get(id).map_or(0, |e| e.bytes + e.log.as_ref().map_or(0, |l| l.pending.len() as u64))
    }

    /// When a file published without the key is deleted unless someone opens it.
    fn expires(&self, meta: &Meta) -> Option<u64> {
        let idle = self.cfg.hosting.idle_secs;
        (meta.owner.is_some() && idle > 0).then(|| meta.used().saturating_add(idle))
    }

    fn info(&self, id: &str, e: &Entry, open: Option<OpenFile>) -> FileInfo {
        let bytes = self.bytes(id);
        FileInfo {
            id: id.to_string(),
            name: e.meta.name.clone(),
            created: e.meta.created,
            updated: e.meta.updated,
            seq: open.map_or(e.meta.seq, |o| o.seq),
            bytes,
            people: open.map_or(0, |o| o.people),
        }
    }

    /// Keep a document (its snapshot) as a new file with an edit link and a view link.
    pub fn publish(&mut self, name: &str, snapshot: &[Change]) -> Result<(FileInfo, NewLink, NewLink), String> {
        self.keep(name, snapshot, None, &|_| Ok(())).map(|(info, edit, view, _)| (info, edit, view))
    }

    /// Keep a document published without the key from `ip`, within the caps on what one address
    /// and everyone may keep: the file, its links and its owner token.
    pub fn publish_open(&mut self, ip: IpAddr, name: &str, snapshot: &[Change]) -> Result<(FileInfo, NewLink, NewLink, String), String> {
        let h = &self.cfg.hosting;
        let (open, max_files, max_ip, max_all) = (h.open, h.max_files_per_ip, h.max_bytes_per_ip, h.max_total_bytes);
        if !open {
            return Err(KEY_ONLY.into());
        }
        if !self.abuse.allow(ip, Action::Publish, Instant::now()) {
            return Err("too many files published from your address lately: try again in an hour".into());
        }
        let ip_text = ip.to_string();
        let mine: Vec<&String> = self.files.iter().filter(|(_, e)| e.meta.ip.as_deref() == Some(ip_text.as_str())).map(|(id, _)| id).collect();
        if mine.len() >= max_files {
            return Err(format!(
                "your address keeps {} files on this server already, the most it may: remove one (with its owner token) or wait until one expires",
                mine.len()
            ));
        }
        let used_ip: u64 = mine.iter().map(|id| self.bytes(id)).sum();
        let used_all: u64 = self.files.keys().map(|id| self.bytes(id)).sum();
        let fits = |n: u64| {
            if used_ip.saturating_add(n) > max_ip {
                return Err(format!("your address's files would take more than the {} MiB it may keep here: remove one first", max_ip >> 20));
            }
            if used_all.saturating_add(n) > max_all {
                return Err("this server is full: it takes no more files for now (try again later, or another server)".into());
            }
            Ok(())
        };
        let (info, edit, view, owner) = self.keep(name, snapshot, Some(ip), &fits)?;
        Ok((info, edit, view, owner.unwrap_or_default()))
    }

    /// Keep a document as a new file: published without the key from `ip` (it gets an owner
    /// token), else by the operator. `fits` checks its size against the caps that apply.
    fn keep(
        &mut self,
        name: &str,
        snapshot: &[Change],
        ip: Option<IpAddr>,
        fits: &dyn Fn(u64) -> Result<(), String>,
    ) -> Result<(FileInfo, NewLink, NewLink, Option<String>), String> {
        if self.files.len() >= self.cfg.max_files {
            return Err(format!("the server keeps at most {} files", self.cfg.max_files));
        }
        if let Some(why) = self.low_disk() {
            return Err(why);
        }
        let replica = Replica::from_snapshot(snapshot).map_err(|e| format!("the file couldn't be read: {e}"))?;
        if replica.document().node_count() > self.cfg.core.max_nodes {
            return Err(format!("the file has more than {} layers", self.cfg.core.max_nodes));
        }
        let mut doc = replica.document().clone();
        let name = match clean_name(name) {
            n if n.is_empty() => doc.name.clone(),
            n => n,
        };
        doc.name = name.clone();
        let server = Server::new(doc, "", self.cfg.core);
        let state = server.state().map_err(|e| e.to_string())?;
        let bytes = disk::encode_snapshot(1, &state)?;
        if bytes.len() as u64 > self.cfg.max_file_bytes {
            return Err(format!("the file is {} MiB; this server keeps files up to {} MiB", bytes.len() >> 20, self.cfg.max_file_bytes >> 20));
        }
        fits(bytes.len() as u64)?;
        let id = loop {
            let id = token::file_id()?;
            if !self.files.contains_key(&id) && !self.dir(&id).exists() {
                break id;
            }
        };
        let t = now();
        let (edit, edit_token) = new_link(Access::Edit, "Published (edit)", t)?;
        let (view, view_token) = new_link(Access::View, "Published (view)", t)?;
        let owner = ip.map(|_| token::secret()).transpose()?;
        let meta =
            Meta { name, created: t, updated: t, seq: 0, library: None, owner: owner.as_deref().map(hash), ip: ip.map(|i| i.to_string()), opened: 0 };
        let dir = self.dir(&id);
        let written = fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| disk::write_atomic(&dir.join(SNAPSHOT), &bytes).map_err(|e| e.to_string()))
            .and_then(|()| disk::write_atomic(&dir.join(LOG), &disk::log_header(1)).map_err(|e| e.to_string()))
            .and_then(|()| write_json(&dir.join(LINKS), &[edit.clone(), view.clone()]))
            // Last: a folder without its description is an upload cut short.
            .and_then(|()| write_json(&dir.join(META), &meta));
        if let Err(e) = written {
            let _ = fs::remove_dir_all(&dir);
            return Err(format!("couldn't keep the file: {e}"));
        }
        let entry = Entry { meta, links: vec![edit.clone(), view.clone()], log: None, bytes: (bytes.len() + LOG_HEADER) as u64, forced: 0 };
        self.files.insert(id.clone(), entry);
        let info = self.files.get(&id).map(|e| self.info(&id, e, None)).ok_or("the file vanished")?;
        match ip {
            Some(ip) => log::info!("event=published file={id} bytes={} ip={ip} by=public", bytes.len()),
            None => log::info!("event=published file={id} bytes={} by=key", bytes.len()),
        }
        Ok((info, NewLink { link: edit.info(&id), token: edit_token }, NewLink { link: view.info(&id), token: view_token }, owner))
    }

    /// The files kept, with what the operator moderates by.
    pub fn listing(&self, open: &dyn Fn(&str) -> Option<OpenFile>) -> Vec<FileRow> {
        self.files
            .iter()
            .map(|(id, e)| FileRow {
                info: self.info(id, e, open(id)),
                opened: e.meta.opened,
                expires: self.expires(&e.meta),
                open: e.meta.owner.is_some(),
                ip: e.meta.ip.clone().unwrap_or_default(),
                reports: u32::try_from(self.reports.iter().filter(|r| r.file == *id).count()).unwrap_or(u32::MAX),
            })
            .collect()
    }

    /// Owner token `owner` manages file `file` (one published without the key).
    fn owns(&self, file: &str, owner: &str) -> bool {
        let presented = hash(owner);
        self.files.get(file).and_then(|e| e.meta.owner.as_deref()).is_some_and(|h| same_secret(&presented, h))
    }

    /// What became of the files these `(file id, owner token)` pairs own: each one's name, links
    /// and expiry, or `None` when that token manages no file of that id here (which doesn't say
    /// whether the id exists). Asking opens nothing: it doesn't count as using a file.
    pub fn owned(&self, files: &[(String, String)]) -> AdminReply {
        if files.len() > MAX_OWNED {
            return AdminReply::Failed { reason: format!("ask about at most {MAX_OWNED} files at a time") };
        }
        let one = |(file, owner): &(String, String)| {
            let e = self.files.get(file).filter(|_| self.owns(file, owner))?;
            Some(OwnedFile { name: e.meta.name.clone(), links: e.links.iter().map(|l| l.id.clone()).collect(), expires: self.expires(&e.meta) })
        };
        AdminReply::Owned(files.iter().map(one).collect())
    }

    /// Rename a file (its listing; the document inside keeps its own name).
    pub fn rename(&mut self, file: &str, name: &str) -> Result<(), String> {
        let name = clean_name(name);
        if name.is_empty() {
            return Err("give the file a name".into());
        }
        let path = self.dir(file).join(META);
        let entry = self.entry(file)?;
        let before = std::mem::replace(&mut entry.meta.name, name);
        if let Err(e) = write_json(&path, &entry.meta) {
            entry.meta.name = before;
            return Err(format!("couldn't rename the file: {e}"));
        }
        Ok(())
    }

    /// Delete the files published without the key that nobody opened for the idle time (not
    /// those open now); their ids.
    pub fn sweep(&mut self) -> Vec<String> {
        let t = now();
        self.next_sweep = t.saturating_add(SWEEP_EVERY);
        let due: Vec<String> =
            self.files.iter().filter(|(_, e)| e.log.is_none() && self.expires(&e.meta).is_some_and(|x| x <= t)).map(|(id, _)| id.clone()).collect();
        for id in &due {
            match self.remove(id) {
                Ok(()) => log::info!("event=expired file={id}"),
                Err(e) => log::warn!("event=expire_failed file={id} reason={e:?}"),
            }
        }
        due
    }

    /// Make a file look unused for `secs` more (tests: expiry without waiting).
    #[cfg(test)]
    pub fn age(&mut self, file: &str, secs: u64) {
        let path = self.dir(file).join(META);
        if let Some(e) = self.files.get_mut(file) {
            let m = &mut e.meta;
            (m.created, m.updated, m.opened) = (m.created.saturating_sub(secs), m.updated.saturating_sub(secs), m.opened.saturating_sub(secs));
            let _ = write_json(&path, m);
        }
    }

    /// Make an open file's log refuse writes (tests: a full or failing disk).
    #[cfg(test)]
    pub fn break_log(&mut self, file: &str) {
        let path = self.dir(file).join(LOG);
        if let Some(log) = self.files.get_mut(file).and_then(|e| e.log.as_mut())
            && let Ok(f) = File::open(path)
        {
            log.file = f;
        }
    }

    /// Note that a file was opened now (it is used: it doesn't expire).
    fn touch(&mut self, file: &str) {
        let path = self.dir(file).join(META);
        if let Some(e) = self.files.get_mut(file) {
            e.meta.opened = now();
            if let Err(err) = write_json(&path, &e.meta) {
                log::warn!("file {file}: couldn't update {META}: {err}");
            }
        }
    }

    /// What the server says about its hosting (with `file`'s expiry).
    fn host_info(&self, file: Option<&str>) -> HostInfo {
        let h = &self.cfg.hosting;
        let expires = file.and_then(|f| self.files.get(f)).and_then(|e| self.expires(&e.meta));
        HostInfo { open: h.open, operator: h.operator.clone(), terms: h.terms.clone(), idle_secs: h.idle_secs, max_file_mb: h.max_file_mb, expires }
    }

    /// Block or unblock an address (an IPv6 address: its /64); the address as kept.
    pub fn block(&mut self, ip: &str, on: bool) -> Result<IpAddr, String> {
        let ip = ip_key(ip.trim().parse::<IpAddr>().map_err(|_| format!("`{}` isn't an IP address", ip.trim()))?);
        if on && !self.blocked.contains(&ip) && self.blocked.len() >= MAX_BLOCKED {
            return Err(format!("at most {MAX_BLOCKED} addresses can be blocked"));
        }
        let changed = if on { self.blocked.insert(ip) } else { self.blocked.remove(&ip) };
        if changed {
            let list: Vec<String> = self.blocked.iter().map(IpAddr::to_string).collect();
            write_json(&self.root.join(BLOCKED), &list).map_err(|e| format!("couldn't keep the blocked addresses: {e}"))?;
            log::info!("event={} ip={ip}", if on { "blocked" } else { "unblocked" });
        }
        Ok(ip)
    }

    /// The blocked addresses.
    pub fn blocked(&self) -> Vec<String> {
        self.blocked.iter().map(IpAddr::to_string).collect()
    }

    /// The reports kept, oldest first.
    pub fn reports(&self) -> &[ReportInfo] {
        &self.reports
    }

    /// Keep a report about `file` (from someone who joined it with `link`, at `ip`).
    pub fn keep_report(&mut self, file: &str, link: Option<&str>, ip: IpAddr, reason: &str) -> Result<(), String> {
        let reason: String = reason.trim().chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_REASON).collect();
        if reason.is_empty() {
            return Err("say what is wrong with the file".into());
        }
        let name = self.entry(file)?.meta.name.clone();
        if !self.abuse.allow(ip, Action::Report, Instant::now()) {
            return Err("too many reports from your address lately: try again in an hour".into());
        }
        self.reports.push(ReportInfo {
            at: now(),
            file: file.to_string(),
            name,
            link: link.unwrap_or_default().to_string(),
            ip: ip.to_string(),
            reason,
        });
        if self.reports.len() > MAX_REPORTS {
            let extra = self.reports.len() - MAX_REPORTS;
            self.reports.drain(..extra);
        }
        write_json(&self.root.join(REPORTS), &self.reports).map_err(|e| format!("couldn't keep the report: {e}"))?;
        log::info!("event=report file={file} ip={ip}");
        Ok(())
    }

    /// The files kept.
    pub fn list(&self, open: &dyn Fn(&str) -> Option<OpenFile>) -> Vec<FileInfo> {
        self.files.iter().map(|(id, e)| self.info(id, e, open(id))).collect()
    }

    /// A file's links.
    pub fn links(&mut self, file: &str) -> Result<Vec<LinkInfo>, String> {
        Ok(self.entry(file)?.links.iter().map(|l| l.info(file)).collect())
    }

    /// A new link to a file.
    pub fn create_link(&mut self, file: &str, access: Access, label: &str) -> Result<NewLink, String> {
        let max = self.cfg.max_links;
        let path = self.dir(file).join(LINKS);
        let entry = self.entry(file)?;
        if entry.links.len() >= max {
            return Err(format!("a file has at most {max} links: revoke one first"));
        }
        let (link, token) = new_link(access, &clean_name(label), now())?;
        entry.links.push(link.clone());
        if let Err(e) = write_json(&path, &entry.links) {
            entry.links.pop();
            return Err(format!("couldn't keep the link: {e}"));
        }
        Ok(NewLink { link: link.info(file), token })
    }

    /// The link of `entry` whose token `token` is (every link is compared, in constant time, so
    /// timing doesn't tell which one matched).
    fn link_of(entry: &Entry, token: &str) -> Option<usize> {
        let presented = hash(token);
        let mut found = None;
        for (i, l) in entry.links.iter().enumerate() {
            if same_secret(&presented, &l.hash) {
                found = Some(i);
            }
        }
        found
    }

    /// These join codes no longer open anything: typing one says so.
    fn forget_codes(&mut self, codes: impl IntoIterator<Item = Sealed>) {
        self.gone_codes.extend(codes.into_iter().map(|c| c.hash));
        let extra = self.gone_codes.len().saturating_sub(MAX_GONE_CODES);
        self.gone_codes.drain(..extra);
    }

    /// Where the link with the join code `key` is: its file, its place among the file's links
    /// and its token (every code is compared, so timing doesn't tell which one matched).
    fn find_code(&self, key: &codes::Key) -> Option<(String, usize, String)> {
        let mut found = None;
        for (id, e) in &self.files {
            for (i, l) in e.links.iter().enumerate() {
                if let Some(token) = l.code.as_ref().and_then(|c| key.open(c)) {
                    found = Some((id.clone(), i, token));
                }
            }
        }
        found
    }

    /// A new join code for the link of `file` whose token is `token` (it takes the place of the
    /// link's earlier code): the code, shown this once.
    pub fn set_code(&mut self, file: &str, token: &str) -> Result<String, String> {
        let at = Self::link_of(self.entry(file)?, token).ok_or("that token isn't one of this file's links (it may have been revoked)")?;
        // 45 random bits: one already in use is all but impossible, and never handed out twice.
        let (code, key) = loop {
            let code = codes::new_code()?;
            let key = codes::Key::new(&code);
            if self.find_code(&key).is_none() {
                break (code, key);
            }
        };
        let sealed = key.seal(token)?;
        let path = self.dir(file).join(LINKS);
        let entry = self.entry(file)?;
        let link = entry.links.get_mut(at).ok_or("that file has no such link")?;
        let before = link.code.replace(sealed);
        let id = link.id.clone();
        if let Err(e) = write_json(&path, &entry.links) {
            if let Some(l) = entry.links.get_mut(at) {
                l.code = before;
            }
            return Err(format!("couldn't keep the join code: {e}"));
        }
        self.forget_codes(before);
        log::info!("event=code file={file} link={id}");
        Ok(code)
    }

    /// The file a join code typed at `ip` opens, and its link's token; wrong codes count against
    /// the address (see `abuse`), and nothing is looked up for an address that is locked out.
    pub fn resolve_code(&mut self, ip: IpAddr, typed: &str) -> Result<(String, String), String> {
        let now = Instant::now();
        if let Some(left) = self.abuse.code_locked(ip, now) {
            return Err(format!(
                "Too many wrong codes from your address: try again in {} minutes, or join with the invite link",
                left.as_secs() / 60 + 1
            ));
        }
        if !self.abuse.allow(ip, Action::Code, now) {
            return Err("Too many codes tried from your address: wait a minute and try again".into());
        }
        if !self.abuse.codes_open(now) {
            return Err(CODES_BUSY.into());
        }
        // Anything that isn't a code's symbols is a wrong code too (and costs no hashing).
        let key = figcraft_collab::code::normalize(typed).ok().filter(|c| c.len() == figcraft_collab::code::ONLINE_LEN).map(|c| codes::Key::new(&c));
        let found = key.as_ref().and_then(|k| self.find_code(k));
        self.abuse.code_tried(ip, found.is_some(), now);
        match found {
            Some((file, _, token)) => {
                log::info!("event=code_used file={file} ip={ip}");
                Ok((file, token))
            }
            None => {
                let gone = key.is_some_and(|k| self.gone_codes.iter().any(|g| g == k.hash()));
                Err(if gone { CODE_GONE } else { NO_CODE }.into())
            }
        }
    }

    /// Revoke a link.
    pub fn revoke(&mut self, file: &str, link: &str) -> Result<(), String> {
        let path = self.dir(file).join(LINKS);
        let entry = self.entry(file)?;
        let at = entry.links.iter().position(|l| l.id == link).ok_or("that file has no such link")?;
        let gone = entry.links.remove(at);
        if let Err(e) = write_json(&path, &entry.links) {
            entry.links.insert(at, gone);
            return Err(format!("couldn't revoke the link: {e}"));
        }
        log::info!("revoked link {link} of file {file}");
        self.forget_codes(gone.code);
        Ok(())
    }

    /// Remove a file and its links for good. `Err` when its folder couldn't be deleted (it is no
    /// longer served either way).
    pub fn remove(&mut self, file: &str) -> Result<(), String> {
        // Closed first: an open log would keep the folder from being deleted on Windows.
        let entry = self.files.remove(file).ok_or("this file isn't on the server (it may have been removed)")?;
        self.forget_codes(entry.links.iter().filter_map(|l| l.code.clone()));
        drop(entry);
        let dir = self.dir(file);
        fs::remove_dir_all(&dir)
            .map_err(|e| format!("it is no longer served, but its folder {} couldn't be deleted ({e}); delete it by hand", dir.display()))?;
        log::info!("removed file {file}");
        Ok(())
    }

    /// A file's team library for `ip`, unless its version is `have`; within what one address and
    /// everyone may download (the size is known before anything is read).
    fn fetch_library(&mut self, ip: IpAddr, file: &str, have: Option<u64>) -> Result<LibraryReply, String> {
        let (dir, max) = (self.dir(file), self.cfg.max_file_bytes);
        let Some(lib) = self.entry(file)?.meta.library.clone() else { return Ok(LibraryReply::Empty) };
        if have == Some(lib.version) {
            return Ok(LibraryReply::Unchanged { version: lib.version });
        }
        let path = library_file(&dir, lib.version);
        let size = fs::metadata(&path).map_or(0, |m| m.len());
        let now = Instant::now();
        if !self.abuse.allow(ip, Action::Download(usize::try_from(size).unwrap_or(usize::MAX)), now) || !self.served.take_n(now, size as f64) {
            return Err(BUSY_DOWNLOADS.into());
        }
        Ok(LibraryReply::Library { version: lib.version, archive: disk::read_capped(&path, max)? })
    }

    /// Keep `archive` as a file's team library: it must read as a library (with the limits files
    /// using it read it with), be the file's library if it has one, and be a newer version.
    // shortcut: the archive is checked on the hub thread (other files' sessions wait meanwhile);
    // move it to a worker if large libraries make that noticeable.
    pub fn publish_library(&mut self, file: &str, archive: &[u8]) -> Result<u64, String> {
        let max = self.cfg.max_file_bytes;
        if self.cfg.hosting.open && self.entry(file)?.meta.owner.is_some() {
            return Err(NO_PUBLIC_LIBRARY.into());
        }
        if archive.len() as u64 > max {
            return Err(format!("the library is {} MiB; this server keeps files up to {} MiB", archive.len() >> 20, max >> 20));
        }
        // Everything it unpacks to counts against the same cap, so a small archive can't expand
        // into a large library in memory.
        let lib = match figcraft_format::read_library_within(Cursor::new(archive), max) {
            Ok(Some(lib)) => lib,
            Ok(None) => return Err("that isn't a published library".into()),
            Err(e) => return Err(format!("the library can't be read: {e}")),
        };
        if lib.doc.node_count() > self.cfg.core.max_nodes {
            return Err(format!("the library has more than {} layers", self.cfg.core.max_nodes));
        }
        let m = &lib.manifest;
        let dir = self.dir(file);
        let entry = self.entry(file)?;
        if let Some(old) = &entry.meta.library {
            if old.id != m.id {
                return Err(format!("this file's library on the server is another one (\"{}\")", old.name));
            }
            if m.version <= old.version {
                return Err(format!("version {} is on the server already: publish again to follow it", old.version));
            }
        }
        // Each version has a file of its own, written before the description names it: a failed
        // description write or a crash in between leaves the previous version served whole, never
        // the new bytes under the old number.
        let path = dir.join(library_name(m.version));
        disk::write_atomic(&path, archive).map_err(|e| format!("couldn't keep the library: {e}"))?;
        let before = entry.meta.library.replace(LibraryMeta { id: m.id.clone(), name: m.name.clone(), version: m.version });
        if let Err(e) = write_json(&dir.join(META), &entry.meta) {
            // Never served (the description doesn't name it); the next publish of it replaces it.
            if let Err(r) = fs::remove_file(&path) {
                log::warn!("file {file}: an unpublished library version couldn't be deleted: {r}");
            }
            entry.meta.library = before;
            return Err(format!("couldn't keep the library: {e}"));
        }
        // The previous version isn't served any more.
        if let Some(Err(e)) = before.map(|old| fs::remove_file(library_file(&dir, old.version))) {
            log::warn!("file {file}: the previous library version couldn't be deleted: {e}");
        }
        log::info!("file {file}: published library version {} ({} bytes)", m.version, archive.len());
        Ok(m.version)
    }

    /// A library's usage reports in memory (read from disk the first time; a damaged list starts
    /// over: it is only statistics). `None` when the file has no library.
    fn usage_of(&mut self, file: &str) -> Result<Option<&mut UsageCache>, String> {
        if self.entry(file)?.meta.library.is_none() {
            return Ok(None);
        }
        if !self.usage.contains_key(file) {
            let path = self.dir(file).join(USAGE);
            let list = if path.exists() {
                disk::read_capped(&path, MAX_USAGE_FILE).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
            } else {
                Vec::new()
            };
            self.usage.insert(file.to_string(), UsageCache { list, dirty: false, written: now() });
        }
        Ok(self.usage.get_mut(file))
    }

    /// The usage reports kept for a file's library (none when it has no library).
    pub fn library_usage(&mut self, file: &str) -> Result<Vec<FileUsage>, String> {
        Ok(self.usage_of(file)?.map(|u| u.list.clone()).unwrap_or_default())
    }

    /// Keep how one file uses a file's library (in memory; written when it changed, at most every
    /// [`USAGE_WRITE_EVERY`] seconds).
    pub fn report_usage(&mut self, file: &str, usage: Usage) -> Result<(), String> {
        let name = clean_name(&usage.file);
        if name.is_empty() {
            return Ok(());
        }
        let mut components: Vec<_> = usage.components.into_iter().filter(|(_, n)| *n > 0).take(MAX_REPORTED * 4).collect();
        components.sort();
        components.dedup_by_key(|c| c.0);
        components.truncate(MAX_REPORTED);
        let Some(u) = self.usage_of(file)? else { return Ok(()) };
        match u.list.iter_mut().find(|f| f.usage.file == name) {
            Some(f) if f.usage.components == components => return Ok(()),
            Some(f) => {
                f.usage.components = components;
                f.updated = now();
            }
            None => u.list.push(FileUsage { usage: Usage { file: name, components }, updated: now() }),
        }
        if u.list.len() > MAX_USERS {
            u.list.sort_by_key(|f| std::cmp::Reverse(f.updated));
            u.list.truncate(MAX_USERS);
        }
        u.dirty = true;
        if now() >= u.written.saturating_add(USAGE_WRITE_EVERY) {
            self.write_usage(false);
        }
        Ok(())
    }

    /// Write the usage reports that changed (`all`: whenever they were last written).
    fn write_usage(&mut self, all: bool) {
        let t = now();
        let root = self.root.join(FILES);
        for (file, u) in &mut self.usage {
            if u.dirty && (all || t >= u.written.saturating_add(USAGE_WRITE_EVERY)) {
                if let Err(e) = write_json(&root.join(file).join(USAGE), &u.list) {
                    log::warn!("file {file}: library usage couldn't be kept: {e}");
                }
                (u.dirty, u.written) = (false, t);
            }
        }
    }

    /// Whether `file` has room for about `n` more bytes: its cap, and for a file published
    /// without the key its address's share and everyone's. A log that took it over its cap is
    /// compacted first (the state may be smaller than its history).
    fn room_for(&mut self, file: &str, server: &Server, n: u64) -> Result<(), String> {
        if let Some(why) = self.low_disk() {
            return Err(why);
        }
        let max = self.cfg.max_file_bytes;
        let over = |s: &Store| s.bytes(file).saturating_add(n) > max;
        // A compaction to make room at most once a minute per file (it rewrites the whole file).
        let t = now();
        let may_compact = self.files.get(file).is_some_and(|e| {
            t >= e.forced.saturating_add(FORCED_COMPACTION_EVERY) && e.log.as_ref().is_some_and(|l| l.entries > 0 || !l.pending.is_empty())
        });
        if over(self) && may_compact {
            if let Some(e) = self.files.get_mut(file) {
                e.forced = t;
            }
            self.compact(file, server);
        }
        if over(self) {
            return Err(format!("this file has reached the server's limit of {} MiB: that change wasn't kept", max >> 20));
        }
        let h = &self.cfg.hosting;
        let Some(ip) = self.files.get(file).filter(|e| e.meta.owner.is_some()).and_then(|e| e.meta.ip.clone()) else { return Ok(()) };
        let (max_ip, max_all) = (h.max_bytes_per_ip, h.max_total_bytes);
        let used_ip: u64 = self.files.iter().filter(|(_, e)| e.meta.ip.as_deref() == Some(ip.as_str())).map(|(id, _)| self.bytes(id)).sum();
        if used_ip.saturating_add(n) > max_ip {
            return Err(format!("the files published from this file's address have reached their {} MiB: that change wasn't kept", max_ip >> 20));
        }
        let used_all: u64 = self.files.keys().map(|id| self.bytes(id)).sum();
        if used_all.saturating_add(n) > max_all {
            return Err("this server is full: that change wasn't kept".into());
        }
        Ok(())
    }

    /// Load a file's session: its snapshot, then its log (a damaged end is dropped and reported).
    fn load(&mut self, file: &str) -> Result<Server, String> {
        self.touch(file);
        let dir = self.dir(file);
        let (generation, state) = disk::decode_snapshot(&disk::read_capped(&dir.join(SNAPSHOT), self.snapshot_cap())?)?;
        let mut server = Server::restore(state, "", self.cfg.core).map_err(|e| format!("the snapshot can't be used: {e}"))?;
        let mut issues = Vec::new();
        let mut fresh_log = true;
        let mut kept = 0;
        match disk::read_capped(&dir.join(LOG), self.cfg.max_file_bytes).and_then(|b| disk::read_log(&b)) {
            Ok(log) if log.generation == generation => {
                fresh_log = false;
                for entry in log.entries {
                    if let Err(e) = server.replay(entry) {
                        issues.push(format!("file {file}: its log stops making sense after {kept} entries ({e}); the rest was dropped"));
                        fresh_log = true;
                        break;
                    }
                    kept += 1;
                }
                if let Some(why) = log.damaged {
                    issues.push(format!("file {file}: its log ended with {why}; that part was dropped"));
                    fresh_log = true;
                }
            }
            // From before a compaction that was cut short, the snapshot has it all; or a generation
            // field damaged, which no checksum covers. Replaying tells them apart without trusting
            // it: entries the snapshot holds are skipped, so only edits it lacks change anything.
            Ok(log) => {
                let before = server.seq();
                let mut stopped = None;
                for entry in log.entries {
                    if let Err(e) = server.replay(entry) {
                        stopped = Some(e);
                        break;
                    }
                }
                let (snap, found, recovered) = (generation, log.generation, server.seq().saturating_sub(before));
                if recovered > 0 {
                    issues.push(format!(
                        "file {file}: its log's generation ({found}) doesn't match its snapshot's ({snap}), but it held \
                         {recovered} changes the snapshot didn't; they were recovered"
                    ));
                }
                if let Some(e) = stopped {
                    issues.push(format!(
                        "file {file}: its log's generation ({found}) doesn't match its snapshot's ({snap}) and it stops \
                         making sense after {recovered} newer changes ({e}); the rest was dropped"
                    ));
                }
            }
            Err(why) => issues.push(format!("file {file}: {why}; starting a new log")),
        }
        for i in issues {
            log::warn!("{i}");
            self.note(i);
        }
        server.journaling(true);
        let file_handle = OpenOptions::new().append(true).open(dir.join(LOG));
        let log = match (fresh_log, file_handle) {
            (false, Ok(f)) => {
                let bytes = f.metadata().map_or(LOG_HEADER as u64, |m| m.len());
                // Replayed entries make it worth compacting once everyone has left.
                Log { generation, file: f, pending: Vec::new(), entries: kept, bytes, dirty: kept > 0 }
            }
            // A new snapshot and log of the next generation, so appends start clean.
            _ => self.rewrite(file, generation + 1, &server).map_err(|e| e.why)?,
        };
        self.entry(file)?.log = Some(log);
        Ok(server)
    }

    /// The largest snapshot read back: a file's cap plus room for what its state keeps beside the
    /// document (recent batches for reconnecting clients, deletions), which may take it over.
    fn snapshot_cap(&self) -> u64 {
        self.cfg.max_file_bytes.saturating_mul(2)
    }

    /// Write `server`'s state as snapshot `generation` and start an empty log of it.
    fn rewrite(&mut self, file: &str, generation: u64, server: &Server) -> Result<Log, RewriteError> {
        let dir = self.dir(file);
        let kept = |why: String| RewriteError { why, snapshot_replaced: false };
        let state = server.state().map_err(|e| kept(e.to_string()))?;
        let bytes = disk::encode_snapshot(generation, &state).map_err(kept)?;
        // Never replace a readable snapshot with one that can't be opened again: the old snapshot
        // and log still hold everything.
        if bytes.len() as u64 > self.snapshot_cap() {
            return Err(kept(format!("the snapshot would be {} MiB, more than can be opened again", bytes.len() >> 20)));
        }
        if bytes.len() as u64 > self.cfg.max_file_bytes {
            log::warn!("file {file} is {} MiB, over the {} MiB cap (kept anyway)", bytes.len() >> 20, self.cfg.max_file_bytes >> 20);
        }
        disk::write_atomic(&dir.join(SNAPSHOT), &bytes).map_err(|e| kept(format!("couldn't write the snapshot: {e}")))?;
        // From here the old log no longer goes with the snapshot on disk.
        let replaced = |why: String| RewriteError { why, snapshot_replaced: true };
        disk::write_atomic(&dir.join(LOG), &disk::log_header(generation)).map_err(|e| replaced(format!("couldn't start the log: {e}")))?;
        let f = OpenOptions::new().append(true).open(dir.join(LOG)).map_err(|e| replaced(format!("couldn't open the log: {e}")))?;
        if let Ok(e) = self.entry(file) {
            e.bytes = (bytes.len() + LOG_HEADER) as u64;
            e.meta.seq = server.seq();
            let meta = e.meta.clone();
            if let Err(err) = write_json(&dir.join(META), &meta) {
                log::warn!("file {file}: couldn't update {META}: {err}");
            }
        }
        Ok(Log { generation, file: f, pending: Vec::new(), entries: 0, bytes: LOG_HEADER as u64, dirty: false })
    }

    /// Compactions so far.
    pub fn compactions(&self) -> u64 {
        self.compactions
    }

    /// Why nothing more may be written when the data directory's disk is nearly full (less free
    /// than [`StoreConfig::min_free_bytes`]); looked up at most once a second. Where the free space
    /// can't be told, nothing is refused.
    fn low_disk(&mut self) -> Option<String> {
        let now = Instant::now();
        let free = match self.free {
            Some((at, free)) if now.saturating_duration_since(at) < std::time::Duration::from_secs(1) => free,
            _ => {
                let free = disk::free_bytes(&self.root);
                self.free = Some((now, free));
                free
            }
        };
        free.filter(|f| *f < self.cfg.min_free_bytes)
            .map(|_| "the server's disk is nearly full: it takes no changes or new files for now (tell its operator)".to_string())
    }

    /// Note a problem for the operator (logged too); only the latest [`MAX_PROBLEMS`] are kept.
    fn note(&mut self, problem: String) {
        if self.problems.len() >= MAX_PROBLEMS {
            self.problems.remove(0);
        }
        self.problems.push(problem);
    }

    /// Compact an open file: its state as the next snapshot, an empty log.
    fn compact(&mut self, file: &str, server: &Server) {
        self.compactions += 1;
        let Some(generation) = self.files.get(file).and_then(|e| e.log.as_ref()).map(|l| l.generation) else { return };
        match self.rewrite(file, generation + 1, server) {
            Ok(log) => {
                if let Ok(e) = self.entry(file) {
                    e.log = Some(log);
                }
                log::debug!("compacted file {file}");
            }
            // The new snapshot holds everything, but a restart would drop whatever is appended to
            // the old log now (its generation no longer matches): the file closes, the hub hears
            // it at the next flush, and it opens again from the new snapshot.
            Err(RewriteError { why, snapshot_replaced: true }) => {
                self.note(format!("file {file}: compaction was cut short ({why}); it was closed"));
                log::error!("file {file}: compaction was cut short: {why}");
                if let Some(e) = self.files.get_mut(file) {
                    e.log = None;
                }
                self.cut_short.push(file.to_string());
            }
            // The old snapshot and log still hold everything: carry on appending.
            Err(RewriteError { why, .. }) => log::error!("file {file}: compaction failed: {why}"),
        }
    }

    /// Write and sync what's pending in every open log; the files whose log couldn't be written.
    /// Those are closed here (their pending records dropped: nobody was told of them) and open
    /// again from what is on disk.
    fn flush_all(&mut self) -> Vec<String> {
        let mut failed = std::mem::take(&mut self.cut_short);
        for (id, e) in &mut self.files {
            if let Some(log) = e.log.as_mut() {
                match flush_log(id, log) {
                    Ok(n) => e.bytes += n,
                    Err(()) => {
                        e.log = None;
                        failed.push(id.clone());
                    }
                }
            }
        }
        failed
    }
}

/// Why a snapshot and log couldn't be rewritten, and whether the snapshot on disk was replaced
/// before it failed (then the old log no longer goes with it).
struct RewriteError {
    why: String,
    snapshot_replaced: bool,
}

/// Write and sync a log's pending records: the bytes written, or `Err` when that failed (the log
/// can't be trusted to hold them).
fn flush_log(id: &str, log: &mut Log) -> Result<u64, ()> {
    if log.pending.is_empty() {
        return Ok(0);
    }
    match log.file.write_all(&log.pending).and_then(|()| log.file.sync_data()) {
        Ok(()) => {
            let n = log.pending.len() as u64;
            log.bytes += n;
            log.pending.clear();
            Ok(n)
        }
        Err(e) => {
            log::error!("file {id}: couldn't write its log: {e}");
            Err(())
        }
    }
}

/// The name of the file holding version `version` of a file's team library.
pub fn library_name(version: u64) -> String {
    format!("library-{version}.bin")
}

/// Where version `version` of the team library in `dir` is: its own file, or `library.bin` for a
/// version published before each version had one.
fn library_file(dir: &Path, version: u64) -> PathBuf {
    let path = dir.join(library_name(version));
    if path.exists() { path } else { dir.join(LIBRARY) }
}

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    disk::write_atomic(path, &text).map_err(|e| e.to_string())
}

/// A new link and its token.
fn new_link(access: Access, label: &str, created: u64) -> Result<(Link, String), String> {
    let token = token::secret()?;
    Ok((Link { id: token::file_id()?, access, label: label.to_string(), created, hash: hash(&token), code: None }, token))
}

impl Backend for Store {
    fn admit(&mut self, file: &str, token: &str) -> Result<Admission, String> {
        let entry = self.entry(file)?;
        let link = Self::link_of(entry, token).and_then(|i| entry.links.get(i));
        link.map(|l| Admission { access: l.access, link: Some(l.id.clone()) })
            .ok_or_else(|| "this link isn't valid (it may have been revoked)".to_string())
    }

    fn open(&mut self, file: &str) -> Result<Server, String> {
        self.load(file).map_err(|e| {
            let why = format!("file {file} can't be opened: {e}");
            log::error!("{why}");
            self.note(why);
            "this file can't be opened on the server (its data is damaged); tell the server's owner".to_string()
        })
    }

    fn changed(&mut self, file: &str, server: &mut Server) {
        let journal = server.take_journal();
        if journal.is_empty() {
            return;
        }
        let (after, max_bytes) = (self.cfg.compact_after, self.cfg.compact_bytes);
        let Some(e) = self.files.get_mut(file) else { return };
        let Some(log) = e.log.as_mut() else { return };
        for j in &journal {
            match disk::record(j) {
                Ok(r) => log.pending.extend(r),
                Err(err) => log::error!("file {file}: a change couldn't be logged: {err}"),
            }
        }
        log.entries += journal.len();
        log.dirty = true;
        e.meta.updated = now();
        if log.entries >= after || log.bytes + log.pending.len() as u64 >= max_bytes {
            // The snapshot holds what's pending too.
            self.compact(file, server);
        }
    }

    fn flush(&mut self) -> Vec<String> {
        self.flush_all()
    }

    fn room(&mut self, file: &str, server: &Server, bytes: usize) -> Result<(), String> {
        self.room_for(file, server, bytes as u64)
    }

    fn idle(&mut self, file: &str, server: &mut Server) -> bool {
        let dirty = self.files.get_mut(file).and_then(|e| e.log.as_mut()).is_some_and(|log| {
            let _ = flush_log(file, log);
            log.dirty
        });
        if dirty {
            self.compact(file, server);
        }
        // It closes anyway, everything in its snapshot: a compaction cut short needs no report.
        self.cut_short.retain(|f| f != file);
        if let Some(e) = self.files.get_mut(file) {
            e.log = None;
        }
        // Everyone just left: idle time counts from now.
        self.touch(file);
        true
    }

    fn stop(&mut self, open: Vec<(&str, &mut Server)>) {
        self.write_usage(true);
        self.flush_all();
        for (file, server) in open {
            if self.files.get(file).and_then(|e| e.log.as_ref()).is_some_and(|l| l.dirty) {
                self.compact(file, server);
            }
        }
    }

    fn manage(&mut self, key: &str) -> Result<(), String> {
        match same_secret(key, &self.key) {
            true => Ok(()),
            false if self.cfg.hosting.open => {
                Err("wrong server key (this server is open to the public: publish without a key, with an up-to-date FigCraft)".into())
            }
            false => Err("wrong server key".into()),
        }
    }

    fn connect(&mut self, ip: IpAddr) -> Result<(), String> {
        if now() >= self.next_sweep {
            self.sweep();
            self.write_usage(false);
        }
        if self.blocked.contains(&ip) {
            return Err("this server doesn't accept connections from your address".into());
        }
        if !self.abuse.allow(ip, Action::Connect, Instant::now()) {
            return Err("too many connections from your address lately: wait a minute and try again".into());
        }
        Ok(())
    }

    fn message(&mut self, ip: IpAddr, msg: &ClientMsg) -> bool {
        // Cursors and cursor chat (many people behind one address each send ~30 a second while
        // they move) have a budget of their own, beside everything else.
        let what = if matches!(msg, ClientMsg::Presence(_) | ClientMsg::Signal(_) | ClientMsg::Ping) { Action::Presence } else { Action::Message };
        self.abuse.allow(ip, what, Instant::now())
    }

    fn hosting(&mut self, file: &str) -> Option<HostInfo> {
        Some(self.host_info(Some(file)))
    }

    fn open_hosting(&mut self, _ip: IpAddr, file: &str, owner: &str) -> Result<HostInfo, String> {
        if file.is_empty() && owner.is_empty() {
            return Ok(self.host_info(None));
        }
        if !self.owns(file, owner) {
            return Err("that owner token doesn't manage a file of that id here (it may have been removed or expired)".into());
        }
        Ok(self.host_info(Some(file)))
    }

    fn hosted(&mut self, ip: IpAddr, owns: Option<&str>, req: Admin, open: &dyn Fn(&str) -> Option<OpenFile>) -> Handled {
        let failed = |reason: &str| Handled::from(AdminReply::Failed { reason: reason.to_string() });
        let file = match &req {
            Admin::Publish { name, snapshot } => {
                return match self.publish_open(ip, name, snapshot) {
                    Ok((file, edit, view, owner)) => {
                        let expires = self.files.get(&file.id).and_then(|e| self.expires(&e.meta));
                        AdminReply::Hosted { file, edit, view, owner, expires }.into()
                    }
                    Err(e) => {
                        log::info!("event=publish_refused ip={ip} reason={e:?}");
                        Handled::from(AdminReply::Failed { reason: e })
                    }
                };
            }
            Admin::Owned { files } => return self.owned(files).into(),
            Admin::Links { file }
            | Admin::CreateLink { file, .. }
            | Admin::Revoke { file, .. }
            | Admin::Rename { file, .. }
            | Admin::Code { file, .. }
            | Admin::Remove { file } => file,
            _ => return failed("that needs the server key"),
        };
        if owns != Some(file.as_str()) {
            return failed("an owner token manages only its own file");
        }
        self.admin(req, open)
    }

    fn report(&mut self, file: &str, link: Option<&str>, ip: IpAddr, reason: &str) -> Result<(), String> {
        self.keep_report(file, link, ip, reason)
    }

    fn admin(&mut self, req: Admin, open: &dyn Fn(&str) -> Option<OpenFile>) -> Handled {
        let failed = |reason: String| Handled::from(AdminReply::Failed { reason });
        match req {
            Admin::Publish { name, snapshot } => match self.publish(&name, &snapshot) {
                Ok((file, edit, view)) => AdminReply::Published { file, edit, view }.into(),
                Err(e) => failed(e),
            },
            Admin::Files => AdminReply::Files(self.list(open)).into(),
            Admin::Links { file } => self.links(&file).map_or_else(failed, |l| AdminReply::Links(l).into()),
            Admin::CreateLink { file, access, label } => self.create_link(&file, access, &label).map_or_else(failed, |l| AdminReply::Link(l).into()),
            Admin::Revoke { file, link } => match self.revoke(&file, &link) {
                Ok(()) => Handled { reply: AdminReply::Done, revoked: Some((file, link)), removed: None, blocked: None },
                Err(e) => failed(e),
            },
            Admin::Rename { file, name } => self.rename(&file, &name).map_or_else(failed, |()| AdminReply::Done.into()),
            Admin::Listing => AdminReply::Listing(self.listing(open)).into(),
            Admin::Reports => AdminReply::Reports(self.reports.clone()).into(),
            Admin::Block { ip } => match self.block(&ip, true) {
                Ok(ip) => Handled { reply: AdminReply::Done, revoked: None, removed: None, blocked: Some(ip) },
                Err(e) => failed(e),
            },
            Admin::Unblock { ip } => self.block(&ip, false).map_or_else(failed, |_| AdminReply::Done.into()),
            Admin::Blocked => AdminReply::Blocked(self.blocked()).into(),
            Admin::Code { file, token } => self.set_code(&file, &token).map_or_else(failed, |code| AdminReply::Code { code }.into()),
            Admin::Owned { files } => self.owned(&files).into(),
            // The hub removes people itself (it knows who is connected); it never asks the store.
            Admin::RemovePeer { .. } => failed("removing someone from a file is done by the server's hub".into()),
            Admin::Remove { file } => {
                if !self.files.contains_key(&file) {
                    return failed("this file isn't on the server (it may have been removed)".into());
                }
                let reply = match self.remove(&file) {
                    Ok(()) => AdminReply::Done,
                    Err(reason) => AdminReply::Failed { reason },
                };
                Handled { reply, revoked: None, removed: Some(file), blocked: None }
            }
        }
    }

    fn library(&mut self, ip: IpAddr, file: &str, access: Access, req: LibraryReq) -> LibraryReply {
        let failed = |reason: String| LibraryReply::Failed { reason };
        match req {
            LibraryReq::Fetch { have, usage } => {
                if let Some(u) = usage
                    && let Err(e) = self.report_usage(file, u)
                {
                    log::warn!("file {file}: a library usage report couldn't be kept: {e}");
                }
                self.fetch_library(ip, file, have).unwrap_or_else(failed)
            }
            LibraryReq::Publish { .. } if access != Access::Edit => {
                failed("you can only view this file: publishing its library needs an edit link".into())
            }
            LibraryReq::Publish { archive } => {
                self.publish_library(file, &archive).map_or_else(failed, |version| LibraryReply::Published { version })
            }
            LibraryReq::Usage => self.library_usage(file).map_or_else(failed, LibraryReply::Usage),
        }
    }

    fn plugins(&mut self, ip: IpAddr, publisher: bool, req: PluginReq) -> PluginReply {
        // One address's share first (before everyone's budget is spent or anything is read).
        if let PluginReq::Fetch { id, version } = &req
            && let Some(size) = self.directory.size_of(id, version.as_deref())
            && !self.abuse.allow(ip, Action::Download(usize::try_from(size).unwrap_or(usize::MAX)), Instant::now())
        {
            return PluginReply::Failed { reason: BUSY_DOWNLOADS.into() };
        }
        self.directory.handle(publisher, req)
    }

    fn resolve(&mut self, ip: IpAddr, code: &str) -> Result<(String, String), String> {
        self.resolve_code(ip, code)
    }

    fn closing(&self) -> Closing {
        Closing::Restarting
    }
}
