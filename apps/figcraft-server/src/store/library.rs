use super::{
    Action, BUSY_DOWNLOADS, Cursor, FILES, FileUsage, Instant, IpAddr, LIBRARY, LibraryMeta, LibraryReply, MAX_FIRST_LIBRARY_VERSION, MAX_REPORTED,
    MAX_USAGE_FILE, MAX_USERS, META, NO_PUBLIC_LIBRARY, Path, PathBuf, Store, USAGE, USAGE_WRITE_EVERY, Usage, UsageCache, clean_name, disk, fs, now,
    write_json,
};

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
        let now = Instant::now();
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
    pub(super) fn write_usage(&mut self, all: bool) {
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
