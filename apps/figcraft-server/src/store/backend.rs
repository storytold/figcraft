use super::journal::flush_log;
use super::{
    Access, Action, Admin, AdminReply, Admission, BUSY_ADMIN, BUSY_DOWNLOADS, Backend, ClientMsg, Closing, Handled, HostInfo, Instant, IpAddr,
    LibraryReply, LibraryReq, OpenFile, PluginReply, PluginReq, Server, Store, disk, now, same_secret,
};

/// Whether `req` changes the server's files (a write, a removal or a join code's stretching):
/// such requests count against the address's budget ([`Action::Admin`]); looking doesn't.
fn changes_files(req: &Admin) -> bool {
    !matches!(
        req,
        Admin::Files | Admin::Links { .. } | Admin::Listing | Admin::Reports | Admin::Blocked | Admin::Owned { .. } | Admin::RemovePeer { .. }
    )
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
        let compact = self.files.get_mut(file).is_some_and(|entry| {
            let snapshot_seq = entry.meta.seq;
            entry.log.as_mut().is_some_and(|log| {
                // Issued client ids and refusals are durable in the log; only edits need a
                // new image-bearing snapshot. Still try a snapshot if syncing the log failed.
                flush_log(file, log).is_err() || (log.dirty && server.seq() > snapshot_seq)
            })
        });
        if compact {
            self.compact(file, server);
        }
        // It closes anyway, with its state in the snapshot and durable log.
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
        self.admin(ip, req, open)
    }

    fn report(&mut self, file: &str, link: Option<&str>, ip: IpAddr, reason: &str) -> Result<(), String> {
        self.keep_report(file, link, ip, reason)
    }

    fn admin(&mut self, ip: IpAddr, req: Admin, open: &dyn Fn(&str) -> Option<OpenFile>) -> Handled {
        let failed = |reason: String| Handled::from(AdminReply::Failed { reason });
        if changes_files(&req) && !self.abuse.allow(ip, Action::Admin, Instant::now()) {
            log::info!("event=refused ip={ip} reason=admin_per_address");
            return failed(BUSY_ADMIN.into());
        }
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
