use super::{
    ACTIONS, Action, BUSY_DOWNLOADS, Cursor, Detached, FILES, FileDetached, FileUsage, IpAddr, LIBRARY, LibraryMeta, LibraryReply,
    MAX_FIRST_LIBRARY_VERSION, MAX_REPORTED, MAX_USAGE_FILE, MAX_USERS, MAX_WEEK_ROWS, MAX_WEEKS, META, NO_PUBLIC_LIBRARY, NodeId, Path, PathBuf,
    Store, USAGE, USAGE_WRITE_EVERY, Usage, UsageCache, UsageEntry, WeekActions, clean_name, disk, fs, now, write_json,
};

/// Which of a file's totals a report brings.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Count {
    Inserted,
    Detached,
}

const WEEK: u64 = 7 * 86_400;

/// The start of the week `t` falls in (seconds since 1970 UTC, a Monday at 00:00).
fn week_of(t: u64) -> u64 {
    let Some(at) = i64::try_from(t).ok().and_then(|t| chrono::DateTime::from_timestamp(t, 0)) else { return 0 };
    let monday = at.date_naive().week(chrono::Weekday::Mon).first_day();
    u64::try_from(monday.and_time(chrono::NaiveTime::MIN).and_utc().timestamp()).unwrap_or(0)
}

/// A kept list of statistics, or an empty one when it is missing or damaged (it starts over).
fn read_kept<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    if !path.exists() {
        return Vec::new();
    }
    disk::read_capped(path, MAX_USAGE_FILE).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

impl UsageEntry {
    /// Raise this file's highest `kind` totals to `counts`; what grew past them, by how much.
    /// A detach total reported before peaks were kept starts from that report.
    fn grow_peaks(&mut self, counts: &[(NodeId, u32)], kind: Count) -> Vec<(NodeId, u32)> {
        let mut grown = Vec::new();
        for &(c, n) in counts {
            let last = if kind == Count::Detached { self.detached.iter().find(|d| d.0 == c).map_or(0, |d| d.1) } else { 0 };
            let at = match self.peaks.iter().position(|p| p.0 == c) {
                Some(at) => at,
                None if self.peaks.len() < MAX_REPORTED => {
                    self.peaks.push((c, 0, 0));
                    self.peaks.len() - 1
                }
                None => continue,
            };
            let Some(p) = self.peaks.get_mut(at) else { continue };
            let peak = if kind == Count::Inserted { &mut p.1 } else { &mut p.2 };
            let base = (*peak).max(last);
            if n > base {
                grown.push((c, n - base));
            }
            *peak = base.max(n);
        }
        grown
    }
}

/// Add `grown` to this week's `kind` counts in `weeks`, dropping weeks older than [`MAX_WEEKS`]
/// and, past [`MAX_WEEK_ROWS`], the oldest rows.
fn add_to_week(weeks: &mut Vec<WeekActions>, grown: &[(NodeId, u32)], kind: Count, t: u64) {
    let week = week_of(t);
    weeks.retain(|w| w.week.saturating_add(MAX_WEEKS * WEEK) > week);
    for &(component, n) in grown {
        let at = match weeks.iter().position(|w| w.week == week && w.component == component) {
            Some(at) => at,
            None => {
                weeks.push(WeekActions { week, component, insertions: 0, detachments: 0 });
                weeks.len() - 1
            }
        };
        if let Some(w) = weeks.get_mut(at) {
            let slot = if kind == Count::Inserted { &mut w.insertions } else { &mut w.detachments };
            *slot = slot.saturating_add(n);
        }
    }
    if weeks.len() > MAX_WEEK_ROWS {
        weeks.sort_by_key(|w| std::cmp::Reverse(w.week));
        weeks.truncate(MAX_WEEK_ROWS);
    }
}

impl Store {
    /// A file's team library for `ip`, unless its version is `have`; within what one address and
    /// everyone may download (the size is known before anything is read).
    pub(super) fn fetch_library(&mut self, ip: IpAddr, file: &str, have: Option<u64>) -> Result<LibraryReply, String> {
        let (dir, max) = (self.dir(file), self.cfg.max_file_bytes);
        let Some(lib) = self.entry(file)?.meta.library.clone() else { return Ok(LibraryReply::Empty) };
        if have == Some(lib.version) {
            return Ok(LibraryReply::Unchanged { version: lib.version });
        }
        let path = library_file(&dir, lib.version);
        let size = fs::metadata(&path).map_or(0, |m| m.len());
        let now = self.clock();
        if !self.abuse.allow(ip, Action::Download(usize::try_from(size).unwrap_or(usize::MAX)), now) || !self.served.take_spread(now, size as f64) {
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
            // A jump would let a client use up the numbers and leave no newer version to publish.
            if m.version > old.version.saturating_add(1) {
                return Err(format!("version {} is too high: the next one after {} is {}", m.version, old.version, old.version.saturating_add(1)));
            }
        } else if m.version > MAX_FIRST_LIBRARY_VERSION {
            return Err(format!("version {} is too high: a first publish goes up to {MAX_FIRST_LIBRARY_VERSION}", m.version));
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
            let (list, weeks) = (read_kept(&path), read_kept(&self.dir(file).join(ACTIONS)));
            self.usage.insert(file.to_string(), UsageCache { list, weeks, dirty: false, written: now() });
        }
        Ok(self.usage.get_mut(file))
    }

    /// The usage reports kept for a file's library (none when it has no library).
    pub fn library_usage(&mut self, file: &str) -> Result<Vec<FileUsage>, String> {
        let list = self.usage_of(file)?.map(|u| u.list.as_slice()).unwrap_or_default();
        Ok(list.iter().filter(|e| e.updated > 0).map(|e| FileUsage { usage: e.usage.clone(), updated: e.updated }).collect())
    }

    /// The detach reports kept for a file's library (none when it has no library).
    pub fn library_detached(&mut self, file: &str) -> Result<Vec<FileDetached>, String> {
        let list = self.usage_of(file)?.map(|u| u.list.as_slice()).unwrap_or_default();
        Ok(list
            .iter()
            .filter(|e| !e.detached.is_empty())
            .map(|e| FileDetached { detached: Detached { file: e.usage.file.clone(), components: e.detached.clone() }, updated: e.detached_updated })
            .collect())
    }

    /// A file's library's components' insertions and detachments week by week, over the last
    /// [`MAX_WEEKS`] weeks (none when it has no library).
    pub fn library_actions(&mut self, file: &str) -> Result<Vec<WeekActions>, String> {
        let since = week_of(now()).saturating_sub((MAX_WEEKS - 1) * WEEK);
        let weeks = self.usage_of(file)?.map(|u| u.weeks.as_slice()).unwrap_or_default();
        Ok(weeks.iter().filter(|w| w.week >= since).cloned().collect())
    }

    /// Keep how one file uses a file's library (in memory; written when it changed, at most every
    /// [`USAGE_WRITE_EVERY`] seconds).
    pub fn report_usage(&mut self, file: &str, usage: Usage) -> Result<(), String> {
        self.report(file, &usage.file, usage.components, |e, components| {
            if e.updated > 0 && e.usage.components == components {
                return false;
            }
            e.usage.components = components;
            e.updated = now();
            true
        })
    }

    /// Keep how many instances of a file's library components one file has detached (as
    /// [`Self::report_usage`] keeps its usage, beside it); what it detached since its last report
    /// counts in this week.
    pub fn report_detached(&mut self, file: &str, detached: Detached) -> Result<(), String> {
        self.report_counts(file, detached, Count::Detached)
    }

    /// Note how many instances of a file's library components one file has inserted: what it
    /// inserted since its last report counts in this week (the totals themselves aren't shown).
    pub fn report_inserted(&mut self, file: &str, inserted: Detached) -> Result<(), String> {
        self.report_counts(file, inserted, Count::Inserted)
    }

    fn report_counts(&mut self, file: &str, d: Detached, kind: Count) -> Result<(), String> {
        let mut grown = Vec::new();
        self.report(file, &d.file, d.components, |e, components| {
            grown = e.grow_peaks(&components, kind);
            if kind == Count::Inserted || e.detached == components {
                return !grown.is_empty();
            }
            e.detached = components;
            e.detached_updated = now();
            true
        })?;
        if let (false, Some(u)) = (grown.is_empty(), self.usage.get_mut(file)) {
            add_to_week(&mut u.weeks, &grown, kind, now());
            // Written with the reports, at most every USAGE_WRITE_EVERY seconds and at shutdown.
            u.dirty = true;
        }
        Ok(())
    }

    /// Keep `counts` from the file called `name` (cleaned: positive counts, one per component, at
    /// most [`MAX_REPORTED`]) in its entry with `set` (which says whether anything changed),
    /// keeping at most [`MAX_USERS`] files, the ones heard from most recently.
    fn report(
        &mut self,
        file: &str,
        name: &str,
        counts: Vec<(NodeId, u32)>,
        set: impl FnOnce(&mut UsageEntry, Vec<(NodeId, u32)>) -> bool,
    ) -> Result<(), String> {
        let name = clean_name(name);
        if name.is_empty() {
            return Ok(());
        }
        let mut counts: Vec<_> = counts.into_iter().filter(|(_, n)| *n > 0).take(MAX_REPORTED * 4).collect();
        counts.sort();
        counts.dedup_by_key(|c| c.0);
        counts.truncate(MAX_REPORTED);
        let Some(u) = self.usage_of(file)? else { return Ok(()) };
        let at = match u.list.iter().position(|e| e.usage.file == name) {
            Some(at) => at,
            None => {
                u.list.push(UsageEntry { usage: Usage { file: name, components: Vec::new() }, ..UsageEntry::default() });
                u.list.len() - 1
            }
        };
        let Some(entry) = u.list.get_mut(at) else { return Ok(()) };
        if !set(entry, counts) {
            return Ok(());
        }
        if u.list.len() > MAX_USERS {
            u.list.sort_by_key(|e| std::cmp::Reverse(e.updated.max(e.detached_updated)));
            u.list.truncate(MAX_USERS);
        }
        u.dirty = true;
        if now() >= u.written.saturating_add(USAGE_WRITE_EVERY) {
            self.write_usage(false);
        }
        Ok(())
    }

    /// Write the usage reports that changed (`all`: whenever they were last written).
    pub(super) fn write_usage(&mut self, all: bool) {
        let t = now();
        let root = self.root.join(FILES);
        for (file, u) in &mut self.usage {
            if u.dirty && (all || t >= u.written.saturating_add(USAGE_WRITE_EVERY)) {
                if let Err(e) = write_json(&root.join(file).join(USAGE), &u.list) {
                    log::warn!("file {file}: library usage couldn't be kept: {e}");
                }
                if let Err(e) = write_json(&root.join(file).join(ACTIONS), &u.weeks) {
                    log::warn!("file {file}: library analytics couldn't be kept: {e}");
                }
                (u.dirty, u.written) = (false, t);
            }
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
