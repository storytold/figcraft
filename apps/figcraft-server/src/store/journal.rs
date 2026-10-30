use super::{
    FORCED_COMPACTION_EVERY, Instant, LOG, LOG_HEADER, Log, MAX_PROBLEMS, META, OpenOptions, RewriteError, SNAPSHOT, Server, Store, Write, disk, fs,
    now, write_json,
};

impl Store {
    /// Whether `file` has room for about `n` more bytes: its cap, and for a file published
    /// without the key its address's share and everyone's. A log that took it over its cap is
    /// compacted first (the state may be smaller than its history).
    pub(super) fn room_for(&mut self, file: &str, server: &Server, n: u64) -> Result<(), String> {
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
        for (times, _, used) in self.per_block(&ip) {
            let cap = max_ip.saturating_mul(u64::from(times));
            if used.saturating_add(n) > cap {
                return Err(format!("the files published from this file's address have reached their {} MiB: that change wasn't kept", cap >> 20));
            }
        }
        let used_all: u64 = self.files.keys().map(|id| self.bytes(id)).sum();
        if used_all.saturating_add(n) > max_all {
            return Err("this server is full: that change wasn't kept".into());
        }
        Ok(())
    }

    /// Load a file's session: its snapshot, then its log (a damaged end is dropped and reported).
    /// One everyone left lately is still loaded: it comes back as it was, nothing read (#1074).
    pub(super) fn load(&mut self, file: &str) -> Result<Server, String> {
        self.touch(file);
        if let Some((server, log)) = self.unpark(file) {
            self.entry(file)?.log = Some(log);
            return Ok(server);
        }
        let dir = self.dir(file);
        let (generation, state) = disk::decode_snapshot(&disk::read_capped(&dir.join(SNAPSHOT), self.snapshot_cap())?)?;
        let mut server = Server::restore(state, "", self.cfg.core).map_err(|e| format!("the snapshot can't be used: {e}"))?;
        let mut issues = Vec::new();
        let mut fresh_log = true;
        let mut kept = 0;
        let log_path = dir.join(LOG);
        // A log that's there but can't be read (an IO error, or over a lowered cap) may hold
        // confirmed edits: the file isn't opened, and the log is left as it is for another try.
        let read = match disk::read_capped(&log_path, self.cfg.max_file_bytes) {
            Err(why) if log_path.exists() => return Err(format!("its log can't be read ({why}); it was left as it is")),
            r => r,
        };
        // One that isn't a log is moved aside before a new one replaces it (or the file isn't opened).
        let read = match read.map(|b| disk::read_log(&b)) {
            Ok(Err(why)) => {
                let aside = dir.join(format!("{LOG}.unreadable-{generation}-{}", now()));
                fs::rename(&log_path, &aside).map_err(|e| format!("{why}, and it couldn't be moved aside ({e})"))?;
                Err(format!("{why}; it was moved aside to {}", aside.display()))
            }
            Ok(Ok(log)) => Ok(log),
            Err(why) => Err(why),
        };
        match read {
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
            _ => {
                let next = generation.checked_add(1).ok_or("the file's generation can't go any higher")?;
                self.rewrite(file, next, &server).map_err(|e| e.why)?
            }
        };
        self.entry(file)?.log = Some(log);
        Ok(server)
    }

    /// The largest snapshot read back: a file's cap plus room for what its state keeps beside the
    /// document (recent batches for reconnecting clients, deletions), which may take it over.
    pub(super) fn snapshot_cap(&self) -> u64 {
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
    pub(super) fn low_disk(&mut self) -> Option<String> {
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
    pub(super) fn note(&mut self, problem: String) {
        if self.problems.len() >= MAX_PROBLEMS {
            self.problems.remove(0);
        }
        self.problems.push(problem);
    }

    /// Compact an open file: its state as the next snapshot, an empty log.
    pub(super) fn compact(&mut self, file: &str, server: &Server) {
        self.compactions += 1;
        let Some(generation) = self.files.get(file).and_then(|e| e.log.as_ref()).map(|l| l.generation) else { return };
        let Some(next) = generation.checked_add(1) else {
            log::error!("file {file}: compaction failed: the file's generation can't go any higher");
            return;
        };
        match self.rewrite(file, next, server) {
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
    pub(super) fn flush_all(&mut self) -> Vec<String> {
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

/// Write and sync a log's pending records: the bytes written, or `Err` when that failed (the log
/// can't be trusted to hold them).
pub(super) fn flush_log(id: &str, log: &mut Log) -> Result<u64, ()> {
    if log.pending.is_empty() {
        return Ok(0);
    }
    disk::counted(true);
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
