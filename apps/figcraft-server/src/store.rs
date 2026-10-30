//! The data directory: the files a server keeps, their share links and the server key, and the
//! [`Backend`] the hub serves them through.
//!
//! ```text
//! <data>/LOCK                    held while a server (or a command changing the directory) uses it
//! <data>/server.json             where the running server listens (for the command line)
//! <data>/server.key              the server key: managing files (publish, links, removal)
//! <data>/files/<id>/meta.json    name, created, updated, sequence number
//! <data>/files/<id>/links.json   share links: id, access, label, created, SHA-256 of the token,
//!                                a join code's hash and wrapped token (see `codes`), and a team
//!                                link's token (see `team`)
//! <data>/files/<id>/snapshot.bin the session state at the last compaction
//! <data>/files/<id>/log.bin      what changed since (see `disk`)
//! <data>/files/<id>/library-<version>.bin  its team library, when one was published (a `.figcraft`
//!                                archive; `library.bin` from servers before versioned names)
//! <data>/files/<id>/usage.json   how other files use that library, as their apps report it
//! <data>/files/<id>/actions.json its components' insertions and detachments week by week
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

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Cursor, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use figcraft_collab::message::{
    Access, Admin, AdminReply, ClientMsg, Detached, FileDetached, FileInfo, FileRow, FileUsage, HostInfo, LibraryReply, LibraryReq, LinkInfo,
    MAX_OWNED, NewLink, OwnedFile, PluginReply, PluginReq, ReportInfo, Usage, WeekActions,
};
use figcraft_collab::{Change, NodeId, Replica, Server, same_secret};
use figcraft_collab_net::hub::{Admission, Backend, Closing, Handled, OpenFile, ip_blocks, ip_key};
use figcraft_collab_net::token;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::abuse::{Abuse, AbuseConfig, Action};
use crate::codes::{self, Sealed};
use crate::directory::{Directory, DirectoryConfig};
use crate::disk::{self, ACTIONS, LIBRARY, LINKS, LOG, LOG_HEADER, META, SNAPSHOT, USAGE};

mod backend;
mod files;
mod journal;
mod library;
mod links;
pub(crate) mod team;

pub use library::library_name;

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

/// Why a request that changes the server's files is turned away (the address's share is used up).
pub const BUSY_ADMIN: &str = "too many changes to this server's files from your address: wait a minute and try again";

/// Why publishing without the key is refused on a server that isn't open to the public.
pub const KEY_ONLY: &str = "this server takes files only from people who have its key: add ?key=… to the address (its operator has the key)";

/// Why a file published without the key can't hold a team library on a public server (a library
/// is a whole archive checked on the server; strangers' files don't get that).
pub const NO_PUBLIC_LIBRARY: &str = "on this public server only files published with the server key can hold a team library";

/// Most files whose usage of one library is kept (the least recently changed go first), and most
/// components one file's report keeps.
const MAX_USERS: usize = 200;
const MAX_REPORTED: usize = 1_000;
/// Largest `usage.json` (and `actions.json`) read.
const MAX_USAGE_FILE: u64 = 4 << 20;
/// Weeks of a library's insertions and detachments kept (a year and the current week), and most
/// rows (a component in a week) kept in all.
const MAX_WEEKS: u64 = 53;
const MAX_WEEK_ROWS: usize = 20_000;
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
    /// The team list (`team_files`): what its links let people do; `None`: no team list.
    pub team: Option<Access>,
    /// Where the browser build is served (`web_url`; empty: not said).
    pub web: String,
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
            team: None,
            web: String::new(),
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

/// A share link (`links.json`): the token itself is never stored, except a team link's (the server
/// hands it to whoever asks the team list for it).
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
    /// A team link's token: the server made the link for its team list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    team: Option<TeamToken>,
}

/// A team link's token, kept so the team list can hand it out; never shown in debug output.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
struct TeamToken(String);

impl std::fmt::Debug for TeamToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TeamToken(..)")
    }
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
    /// Closed lately: its session kept loaded ([`PARK_FOR`]).
    parked: Option<Parked>,
    /// The `opened` time `meta.json` holds ([`TOUCH_EVERY`]).
    opened_written: u64,
}

/// A file everyone left lately, kept loaded so a join soon after neither reads it nor rewrites it
/// (#1074): its session and its log (synced, its state compacted as on any close).
struct Parked {
    server: Server,
    log: Log,
    at: Instant,
}

/// How long a file everyone left stays loaded.
const PARK_FOR: std::time::Duration = std::time::Duration::from_secs(60);
/// Most files kept loaded after everyone left (the one parked longest goes first).
const MAX_PARKED: usize = 16;
/// Most bytes (on disk) of the files kept loaded after everyone left.
const MAX_PARKED_BYTES: u64 = 256 << 20;
/// How often, at most, a file's `opened` time is written while it keeps being opened (seconds):
/// expiry counts in days, so it needn't be written at every join and leave.
const TOUCH_EVERY: u64 = 3600;

/// A library's usage reports in memory, and whether (and when) they were written.
struct UsageCache {
    list: Vec<UsageEntry>,
    /// Insertions and detachments week by week, over every file (kept in `actions.json`).
    weeks: Vec<WeekActions>,
    dirty: bool,
    written: u64,
}

/// What one file reports about a library, as kept in `usage.json`: its last usage report (the
/// same JSON as a [`FileUsage`]; `updated` 0: none yet), its last detach report, and the highest
/// insertion and detach totals it reported per component (`peaks`: what counts in a week is what
/// a total grows past them, so an undo and a redo count once).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct UsageEntry {
    usage: Usage,
    #[serde(default)]
    updated: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    detached: Vec<(NodeId, u32)>,
    #[serde(default, skip_serializing_if = "is_zero")]
    detached_updated: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    peaks: Vec<(NodeId, u32, u32)>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
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
    /// Tests: the time the rate limits read, held still (`None`: the clock).
    #[cfg(test)]
    pub(crate) frozen: Option<Instant>,
    /// Usage reports by library file.
    usage: BTreeMap<String, UsageCache>,
    blocked: BTreeSet<IpAddr>,
    reports: Vec<ReportInfo>,
    /// Hashes of join codes that no longer open anything (their link was revoked, their file
    /// removed or expired, or a newer code took their place): typing one says so.
    // shortcut: in memory only, so after a restart such a code reads as unknown; keep them in the
    // data directory if people ask why their code "never existed".
    gone_codes: VecDeque<String>,
    /// The files whose links hold each join code, by the code's hash ([`Sealed::hash`]): a typed
    /// code is looked up here instead of against every link.
    codes: HashMap<String, BTreeSet<String>>,
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
    s.trim().chars().filter(|c| !c.is_control() && !is_bidi_control(*c)).take(256).collect()
}

/// A report's reason: trimmed, control and bidi characters turned into spaces, at most `MAX_REASON` characters.
fn clean_reason(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_control() || is_bidi_control(c) { ' ' } else { c }).collect();
    s.trim().chars().take(MAX_REASON).collect()
}

/// Bidirectional format characters, which could reorder what a terminal shows.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{61C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Take the data directory's lock: `None` when someone else holds it.
pub fn lock(root: &Path) -> Result<Option<File>, String> {
    // Only the server's account reads what it keeps (files, addresses, reports, the key).
    let mut dirs = fs::DirBuilder::new();
    dirs.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut dirs, 0o700);
    dirs.create(root).map_err(|e| format!("can't create the data directory {}: {e}", root.display()))?;
    // `DirBuilder` only applies the mode to directories it creates; tighten an existing one too.
    #[cfg(unix)]
    fs::set_permissions(root, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .map_err(|e| format!("can't restrict the data directory {}: {e}", root.display()))?;
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

/// The highest version a file's first library publish may claim. Publishing counts up by one (a
/// library carried over from another server keeps its number), so no real history gets near it.
/// Later publishes may go one past the version on the server, so there is always a next one.
const MAX_FIRST_LIBRARY_VERSION: u64 = 1 << 32;

impl Store {
    /// The time the rate limits are measured at: the clock, or in tests the time they hold it at.
    pub(crate) fn clock(&self) -> Instant {
        #[cfg(test)]
        if let Some(t) = self.frozen {
            return t;
        }
        Instant::now()
    }

    /// Use the data directory `root` (made if missing). Fails when another server or command holds
    /// it. Files that can't be read are skipped and listed in [`problems`](Self::problems).
    pub fn open(root: &Path, cfg: StoreConfig) -> Result<Store, String> {
        let mut store = Self::try_open(root, cfg)?.ok_or_else(|| format!("another figcraft-server (or a command) is using {}", root.display()))?;
        store.sweep();
        store.drop_stale_team_links();
        Ok(store)
    }

    /// [`open`](Self::open) without the expiry sweep (for commands that only look or edit), or
    /// `None` when someone else holds the directory.
    pub fn try_open(root: &Path, cfg: StoreConfig) -> Result<Option<Store>, String> {
        let Some(lock) = lock(root)? else { return Ok(None) };
        for name in [KEY, BLOCKED, REPORTS] {
            let _ = fs::remove_file(disk::temp_of(&root.join(name)));
        }
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
            #[cfg(test)]
            frozen: None,
            usage: BTreeMap::new(),
            blocked: BTreeSet::new(),
            reports: Vec::new(),
            gone_codes: VecDeque::new(),
            codes: HashMap::new(),
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
                    for c in entry.links.iter().filter_map(|l| l.code.as_ref()) {
                        self.codes.entry(c.hash.clone()).or_default().insert(id.clone());
                    }
                    self.files.insert(id, entry);
                }
                Err(why) => self.note(format!("skipped file {id}: {why}")),
            }
        }
    }

    /// A file folder's description and links, after checking its snapshot and log. Leftover
    /// temporary files (writes cut short) are removed.
    fn check(&self, path: &Path) -> Result<Entry, String> {
        for name in [SNAPSHOT, LOG, META, LINKS, USAGE, ACTIONS] {
            let tmp = disk::temp_of(&path.join(name));
            if tmp.exists() {
                let _ = fs::remove_file(&tmp);
            }
        }
        if let Ok(entries) = fs::read_dir(path) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("library-") && name.ends_with(".bin.tmp") {
                    let _ = fs::remove_file(e.path());
                }
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
        let opened_written = meta.opened;
        Ok(Entry { meta, links, log: None, bytes, forced: 0, parked: None, opened_written })
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
}

/// Why a snapshot and log couldn't be rewritten, and whether the snapshot on disk was replaced
/// before it failed (then the old log no longer goes with it).
struct RewriteError {
    why: String,
    snapshot_replaced: bool,
}

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    disk::write_atomic(path, &text).map_err(|e| e.to_string())
}

/// A new link and its token.
fn new_link(access: Access, label: &str, created: u64) -> Result<(Link, String), String> {
    let token = token::secret()?;
    Ok((Link { id: token::file_id()?, access, label: label.to_string(), created, hash: hash(&token), code: None, team: None }, token))
}
