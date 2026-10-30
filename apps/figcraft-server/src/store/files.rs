use super::{
    Access, Action, AdminReply, BLOCKED, Change, Entry, FILES, FileInfo, FileRow, HostInfo, Instant, IpAddr, KEY_ONLY, LINKS, LOG, LOG_HEADER,
    MAX_BLOCKED, MAX_OWNED, MAX_PARKED, MAX_PARKED_BYTES, MAX_REPORTS, META, Meta, NewLink, OpenFile, OwnedFile, PARK_FOR, Parked, REPORTS, Replica,
    ReportInfo, SNAPSHOT, SWEEP_EVERY, Server, Store, TOUCH_EVERY, clean_name, clean_reason, disk, fs, hash, ip_blocks, ip_key, new_link, now,
    same_secret, token, write_json,
};

impl Store {
    /// Keep a document (its snapshot) as a new file with an edit link and a view link.
    pub fn publish(&mut self, name: &str, snapshot: &[Change]) -> Result<(FileInfo, NewLink, NewLink), String> {
        self.keep(name, snapshot, None, &|_| Ok(())).map(|(info, edit, view, _)| (info, edit, view))
    }

    /// For each block the address `ip` counts in ([`ip_blocks`]), how many times one address's
    /// caps it gets, and the files and bytes kept by the files published from addresses in it. An
    /// address that doesn't parse counts as itself only, ×1, by its exact text.
    pub(super) fn per_block(&self, ip: &str) -> Vec<(u32, usize, u64)> {
        let blocks_of = |text: &str| text.parse::<IpAddr>().map(|a| ip_blocks(ip_key(a))).unwrap_or_default();
        let kept: Vec<(&str, Vec<_>, u64)> = self
            .files
            .iter()
            .filter_map(|(id, e)| {
                let from = e.meta.ip.as_deref()?;
                Some((from, blocks_of(from).into_iter().map(|(b, _)| b).collect(), self.bytes(id)))
            })
            .collect();
        let sum = |mine: &mut dyn Iterator<Item = &(&str, Vec<_>, u64)>| mine.fold((0, 0u64), |(f, b), (_, _, n)| (f + 1, b.saturating_add(*n)));
        let blocks = blocks_of(ip);
        if blocks.is_empty() {
            let (files, bytes) = sum(&mut kept.iter().filter(|(from, _, _)| *from == ip));
            return vec![(1, files, bytes)];
        }
        blocks
            .into_iter()
            .map(|(block, times)| {
                let (files, bytes) = sum(&mut kept.iter().filter(|(_, blocks, _)| blocks.contains(&block)));
                (times, files, bytes)
            })
            .collect()
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
        let blocks = self.per_block(&ip.to_string());
        for &(times, files, _) in &blocks {
            if files >= max_files.saturating_mul(times as usize) {
                return Err(format!(
                    "your address keeps {files} files on this server already, the most it may: remove one (with its owner token) or wait until one expires"
                ));
            }
        }
        let used_all: u64 = self.files.keys().map(|id| self.bytes(id)).sum();
        let fits = |n: u64| {
            for &(times, _, used) in &blocks {
                let cap = max_ip.saturating_mul(u64::from(times));
                if used.saturating_add(n) > cap {
                    return Err(format!("your address's files would take more than the {} MiB it may keep here: remove one first", cap >> 20));
                }
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
            .and_then(|()| {
                disk::sync_dir(Some(&self.root.join(FILES)));
                disk::write_atomic(&dir.join(SNAPSHOT), &bytes).map_err(|e| e.to_string())
            })
            .and_then(|()| disk::write_atomic(&dir.join(LOG), &disk::log_header(1)).map_err(|e| e.to_string()))
            .and_then(|()| write_json(&dir.join(LINKS), &[edit.clone(), view.clone()]))
            // Last: a folder without its description is an upload cut short.
            .and_then(|()| write_json(&dir.join(META), &meta));
        if let Err(e) = written {
            let _ = fs::remove_dir_all(&dir);
            return Err(format!("couldn't keep the file: {e}"));
        }
        let opened_written = meta.opened;
        let (links, bytes_kept) = (vec![edit.clone(), view.clone()], (bytes.len() + LOG_HEADER) as u64);
        let entry = Entry { meta, links, log: None, bytes: bytes_kept, forced: 0, parked: None, opened_written };
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
    pub(super) fn owns(&self, file: &str, owner: &str) -> bool {
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
        // Files kept loaded after everyone left let go once their time is up.
        for e in self.files.values_mut() {
            if e.parked.as_ref().is_some_and(|p| p.at.elapsed() > PARK_FOR) {
                e.parked = None;
            }
        }
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
            e.opened_written = m.opened;
            let _ = write_json(&path, m);
        }
    }

    /// Make an open file's log refuse writes (tests: a full or failing disk).
    #[cfg(test)]
    pub fn break_log(&mut self, file: &str) {
        let path = self.dir(file).join(LOG);
        if let Some(log) = self.files.get_mut(file).and_then(|e| e.log.as_mut())
            && let Ok(f) = std::fs::File::open(path)
        {
            log.file = f;
        }
    }

    /// Note that a file was opened now (it is used: it doesn't expire). Expiry counts in days, so
    /// `meta.json` gets the time at most every [`TOUCH_EVERY`] seconds (and when the server stops),
    /// not at every join and leave (#1074).
    pub(super) fn touch(&mut self, file: &str) {
        let t = now();
        let due = self.files.get_mut(file).is_some_and(|e| {
            e.meta.opened = t;
            t >= e.opened_written.saturating_add(TOUCH_EVERY)
        });
        if due {
            self.write_opened(file);
        }
    }

    /// Write `file`'s `opened` time to its `meta.json`.
    pub(super) fn write_opened(&mut self, file: &str) {
        let path = self.dir(file).join(META);
        if let Some(e) = self.files.get_mut(file) {
            match write_json(&path, &e.meta) {
                Ok(()) => e.opened_written = e.meta.opened,
                Err(err) => log::warn!("file {file}: couldn't update {META}: {err}"),
            }
        }
    }

    /// Keep `file`'s session loaded for a while after everyone left ([`PARK_FOR`]), at most
    /// [`MAX_PARKED`] files of at most [`MAX_PARKED_BYTES`] together (the ones parked longest let
    /// go first; a file larger than that isn't kept).
    pub(super) fn park(&mut self, file: &str, server: Server, log: super::Log) {
        let size = self.bytes(file);
        if size > MAX_PARKED_BYTES {
            return;
        }
        let mut parked: Vec<(String, Instant, u64)> =
            self.files.iter().filter_map(|(id, e)| e.parked.as_ref().map(|p| (id.clone(), p.at, e.bytes))).collect();
        parked.sort_by_key(|(_, at, _)| *at);
        let mut held: u64 = parked.iter().map(|(_, _, b)| b).sum();
        let mut n = parked.len();
        for (id, _, b) in &parked {
            if n < MAX_PARKED && held.saturating_add(size) <= MAX_PARKED_BYTES {
                break;
            }
            if let Some(e) = self.files.get_mut(id) {
                e.parked = None;
            }
            n -= 1;
            held = held.saturating_sub(*b);
        }
        if let Some(e) = self.files.get_mut(file) {
            e.parked = Some(Parked { server, log, at: Instant::now() });
        }
    }

    /// `file`'s session as it was left, while it is still kept loaded ([`PARK_FOR`]).
    pub(super) fn unpark(&mut self, file: &str) -> Option<(Server, super::Log)> {
        let p = self.files.get_mut(file)?.parked.take()?;
        (p.at.elapsed() <= PARK_FOR).then_some((p.server, p.log))
    }

    /// What the server says about its hosting (with `file`'s expiry).
    pub(super) fn host_info(&self, file: Option<&str>) -> HostInfo {
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
        let reason = clean_reason(reason);
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
}
