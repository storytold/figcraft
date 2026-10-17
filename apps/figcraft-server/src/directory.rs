//! The plugin directory: plugin bundles (`figcraft_plugin::bundle`) published to the server with
//! its key, listed, searched and fetched by anyone who can reach it, and unpublished with the key
//! (there are no accounts: whoever has the key is the publisher, and the operator moderates).
//!
//! ```text
//! <data>/plugins/index.json      every version kept: manifest fields, size, SHA-256, when
//! <data>/plugins/<sha256>.bin    a bundle, named by the SHA-256 of its bytes
//! ```
//!
//! A bundle is checked in full before it is kept (it decodes within the size cap, its manifest
//! parses and asks for a plugin API this build runs, its version is newer than the one kept, its
//! module compiles and imports only what the sandbox provides, its asset names are plain) and is
//! never run here. Bundles are named by their hash, so no name a publisher picks reaches the file
//! system, and a fetch reads the file back and checks its hash before sending it. Caps: bundle
//! size, versions kept and bytes kept; fetches share a budget of bytes per second, so a crowd of
//! downloads can't queue more than a bounded amount of data.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use figcraft_collab::message::{PluginEntry, PluginReply, PluginReq};
use figcraft_collab_net::net::Bucket;
use figcraft_plugin::bundle::{Bundle, compare_versions, parse_version, sha256_hex};
use figcraft_plugin::manifest::Access;

use crate::disk;
use crate::store::now;

pub const PLUGINS: &str = "plugins";
pub const INDEX: &str = "index.json";
/// Largest `index.json` read, longest search, and most bytes of entries one listing carries.
const MAX_INDEX_FILE: u64 = 64 << 20;
const MAX_QUERY: usize = 200;
const MAX_LISTING: usize = 32 << 20;
/// What fetches may send, in MiB per second on average (each costs at least one).
const SERVE_MIB_PER_SEC: f64 = 32.0;
/// …and at once: eight times what one block of addresses may download at once, so no block can
/// take it all (see `abuse`).
const SERVE_BURST_MIB: f64 = 512.0;
const MIB: f64 = (1 << 20) as f64;

/// What the directory enforces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectoryConfig {
    /// Most versions kept (all plugins together); 0 turns the directory off.
    pub max_plugins: usize,
    /// Largest bundle.
    pub max_plugin_bytes: u64,
    /// Most bytes of bundles kept.
    pub max_total_bytes: u64,
}

/// The plugins a server keeps.
pub struct Directory {
    dir: PathBuf,
    cfg: DirectoryConfig,
    entries: Vec<PluginEntry>,
    served: Bucket,
    /// `index.json` couldn't be read: nothing is written over it until the operator fixes it.
    broken: bool,
}

/// A plugin id as manifests have them (letters, digits, `.`, `-`, `_`; 1–128).
fn valid_id(s: &str) -> bool {
    (1..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Newest first.
fn by_version(a: &PluginEntry, b: &PluginEntry) -> Ordering {
    compare_versions(&b.version, &a.version).unwrap_or(Ordering::Equal)
}

impl Directory {
    /// The directory under `root` (made when the first plugin is published). Entries whose bundle
    /// is missing or the wrong size are left out and reported, never deleted.
    pub fn open(root: &Path, cfg: DirectoryConfig) -> (Directory, Vec<String>) {
        let dir = root.join(PLUGINS);
        let burst = (2 * cfg.max_plugin_bytes) as f64 / MIB;
        let mut d = Directory { dir, cfg, entries: Vec::new(), served: Bucket::new(SERVE_MIB_PER_SEC, burst.max(SERVE_BURST_MIB)), broken: false };
        let mut problems = Vec::new();
        let index = d.dir.join(INDEX);
        let _ = fs::remove_file(disk::temp_of(&index));
        if !index.exists() {
            return (d, problems);
        }
        let all: Vec<PluginEntry> =
            match disk::read_capped(&index, MAX_INDEX_FILE).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())) {
                Ok(all) => all,
                Err(e) => {
                    problems.push(format!("plugin directory: {INDEX} can't be read ({e}); no plugins are listed or published until it is fixed"));
                    d.broken = true;
                    return (d, problems);
                }
            };
        for e in all {
            let size = fs::metadata(d.file(&e.sha256)).map(|m| m.len()).ok();
            if !valid_id(&e.id) || parse_version(&e.version).is_none() || !valid_sha(&e.sha256) || size != Some(e.bytes) {
                problems.push(format!(
                    "plugin directory: skipped {} {}: its bundle is missing or damaged",
                    e.id.escape_debug(),
                    e.version.escape_debug()
                ));
                continue;
            }
            d.entries.push(e);
        }
        (d, problems)
    }

    fn file(&self, sha: &str) -> PathBuf {
        self.dir.join(format!("{sha}.bin"))
    }

    /// Every version kept.
    pub fn entries(&self) -> &[PluginEntry] {
        &self.entries
    }

    /// Bytes of bundles kept (a bundle published under two versions counts once).
    fn total_bytes(&self) -> u64 {
        let files: BTreeMap<&str, u64> = self.entries.iter().map(|e| (e.sha256.as_str(), e.bytes)).collect();
        files.values().sum()
    }

    /// Answer one request; `publisher`: the connection has the server's key.
    pub fn handle(&mut self, publisher: bool, req: PluginReq) -> PluginReply {
        let failed = |reason: String| PluginReply::Failed { reason };
        if self.cfg.max_plugins == 0 {
            return failed("this server's plugin directory is turned off".into());
        }
        let needs_key = || failed("publishing and unpublishing plugins needs the server's key (ws://host:port/?key=…)".into());
        match req {
            PluginReq::List { query } => PluginReply::List(self.list(&query)),
            PluginReq::Fetch { id, version } => match self.fetch(&id, version.as_deref()) {
                Ok((entry, bundle)) => PluginReply::Bundle { entry, bundle },
                Err(e) => failed(e),
            },
            PluginReq::Publish { .. } | PluginReq::Unpublish { .. } if !publisher => needs_key(),
            PluginReq::Publish { bundle } => self.publish(&bundle).map_or_else(failed, PluginReply::Published),
            PluginReq::Unpublish { id, version } => self.unpublish(&id, version.as_deref()).map_or_else(failed, PluginReply::Removed),
        }
    }

    /// The latest version of each plugin matching `query` (any case, in its id, name, description
    /// or author), by name, with the versions kept.
    pub fn list(&self, query: &str) -> Vec<PluginEntry> {
        let q: String = query.trim().chars().take(MAX_QUERY).collect::<String>().to_lowercase();
        let mut by_id: BTreeMap<&str, Vec<&PluginEntry>> = BTreeMap::new();
        for e in &self.entries {
            by_id.entry(e.id.as_str()).or_default().push(e);
        }
        let mut out: Vec<PluginEntry> = by_id
            .into_values()
            .filter_map(|mut versions| {
                versions.sort_by(|a, b| by_version(a, b));
                let mut latest = (*versions.first()?).clone();
                latest.versions = versions.iter().map(|v| v.version.clone()).collect();
                Some(latest)
            })
            .filter(|e| q.is_empty() || [&e.id, &e.name, &e.description, &e.author].iter().any(|f| f.to_lowercase().contains(&q)))
            .collect();
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
        // A listing stays well within a message.
        let mut size = 0;
        out.retain(|e| {
            size += 256 + e.name.len() + e.description.len() + e.author.len() + e.commands.iter().map(|c| c.len() + 8).sum::<usize>();
            size <= MAX_LISTING
        });
        out
    }

    /// The size of version `version` (or the latest) of plugin `id`'s bundle, without reading it.
    pub fn size_of(&self, id: &str, version: Option<&str>) -> Option<u64> {
        self.entries
            .iter()
            .filter(|e| e.id == id && version.is_none_or(|v| compare_versions(&e.version, v) == Some(Ordering::Equal)))
            .min_by(|a, b| by_version(a, b))
            .map(|e| e.bytes)
    }

    /// Version `version` (or the latest) of plugin `id`, and its bundle, read back and checked
    /// against its hash.
    pub fn fetch(&mut self, id: &str, version: Option<&str>) -> Result<(PluginEntry, Vec<u8>), String> {
        let entry = self
            .entries
            .iter()
            .filter(|e| e.id == id && version.is_none_or(|v| compare_versions(&e.version, v) == Some(Ordering::Equal)))
            .min_by(|a, b| by_version(a, b))
            .cloned()
            .ok_or_else(|| match version {
                Some(v) => format!("the directory has no version {} of that plugin", v.chars().take(32).collect::<String>()),
                None => "the directory has no such plugin (it may have been unpublished)".to_string(),
            })?;
        if !self.served.take_n(Instant::now(), 1.0 + entry.bytes as f64 / MIB) {
            return Err("the server is busy sending plugins: try again in a moment".into());
        }
        let bytes = disk::read_capped(&self.file(&entry.sha256), self.cfg.max_plugin_bytes.max(entry.bytes))?;
        if sha256_hex(&bytes) != entry.sha256 {
            log::error!("plugin directory: the bundle of {} {} is damaged on disk; not sending it", entry.id, entry.version);
            return Err("that plugin's bundle is damaged on the server; tell the server's owner".into());
        }
        Ok((entry, bytes))
    }

    /// Keep a bundle as a new version of its plugin, after checking all of it.
    // shortcut: the bundle is checked (its module compiled) on the hub thread, like team libraries;
    // move it to a worker if large plugins make other sessions wait noticeably.
    pub fn publish(&mut self, bytes: &[u8]) -> Result<PluginEntry, String> {
        let max = self.cfg.max_plugin_bytes;
        if bytes.len() as u64 > max {
            return Err(format!("the plugin is {} KiB; this server keeps plugins up to {} KiB", bytes.len() >> 10, max >> 10));
        }
        let bundle = Bundle::decode(bytes, usize::try_from(max).unwrap_or(usize::MAX))?;
        let m = bundle.check()?;
        if let Some(latest) = self.entries.iter().filter(|e| e.id == m.id).min_by(|a, b| by_version(a, b))
            && compare_versions(&m.version, &latest.version) != Some(Ordering::Greater)
        {
            return Err(format!("version {} of {} is on the server already: publish a newer version", latest.version, m.id));
        }
        if self.entries.len() >= self.cfg.max_plugins {
            return Err(format!("the directory keeps at most {} plugin versions: unpublish some first", self.cfg.max_plugins));
        }
        let sha = sha256_hex(bytes);
        let known = self.entries.iter().any(|e| e.sha256 == sha);
        if !known && self.total_bytes() + bytes.len() as u64 > self.cfg.max_total_bytes {
            return Err(format!("the directory is full ({} MiB): unpublish some plugins first", self.cfg.max_total_bytes >> 20));
        }
        let entry = PluginEntry {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            description: m.description.clone(),
            author: m.author.clone(),
            api: m.api,
            access: match m.permissions.access {
                Access::Read => "read".into(),
                Access::Write => "write".into(),
            },
            commands: m.permissions.commands.clone(),
            widget: m.widget,
            save_file: m.permissions.save_file,
            bytes: bytes.len() as u64,
            sha256: sha.clone(),
            published: now(),
            versions: Vec::new(),
        };
        fs::create_dir_all(&self.dir).map_err(|e| format!("couldn't keep the plugin: {e}"))?;
        let file = self.file(&sha);
        if !known {
            disk::write_atomic(&file, bytes).map_err(|e| format!("couldn't keep the plugin: {e}"))?;
        }
        self.entries.push(entry.clone());
        if let Err(e) = self.save() {
            self.entries.pop();
            return Err(format!("couldn't keep the plugin: {e}"));
        }
        log::info!("plugin directory: published {} {} ({} bytes)", entry.id, entry.version, entry.bytes);
        Ok(entry)
    }

    /// Remove version `version` (or every version) of plugin `id`; the versions removed.
    pub fn unpublish(&mut self, id: &str, version: Option<&str>) -> Result<Vec<String>, String> {
        let gone = |e: &PluginEntry| e.id == id && version.is_none_or(|v| compare_versions(&e.version, v) == Some(Ordering::Equal));
        let (removed, kept): (Vec<PluginEntry>, Vec<PluginEntry>) = self.entries.iter().cloned().partition(gone);
        if removed.is_empty() {
            return Err("the directory has no such plugin or version".into());
        }
        let before = std::mem::replace(&mut self.entries, kept);
        if let Err(e) = self.save() {
            self.entries = before;
            return Err(format!("couldn't unpublish: {e}"));
        }
        for r in &removed {
            if !self.entries.iter().any(|e| e.sha256 == r.sha256)
                && let Err(e) = fs::remove_file(self.file(&r.sha256))
            {
                log::warn!("plugin directory: couldn't delete the bundle of {} {}: {e}", r.id, r.version);
            }
        }
        let versions: Vec<String> = removed.into_iter().map(|e| e.version).collect();
        log::info!("plugin directory: unpublished {id} {}", versions.join(", "));
        Ok(versions)
    }

    fn save(&self) -> Result<(), String> {
        if self.broken {
            return Err(format!("the directory's {INDEX} is damaged; tell the server's owner"));
        }
        let text = serde_json::to_vec_pretty(&self.entries).map_err(|e| e.to_string())?;
        disk::write_atomic(&self.dir.join(INDEX), &text).map_err(|e| e.to_string())
    }
}
