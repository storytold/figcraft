use super::{
    Access, Action, CODE_GONE, CODES_BUSY, Entry, FileInfo, Instant, IpAddr, LINKS, LinkInfo, MAX_GONE_CODES, NO_CODE, NewLink, OpenFile, Sealed,
    Store, clean_name, codes, fs, hash, new_link, now, same_secret, write_json,
};

impl Store {
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
    pub(super) fn link_of(entry: &Entry, token: &str) -> Option<usize> {
        let presented = hash(token);
        let mut found = None;
        for (i, l) in entry.links.iter().enumerate() {
            if same_secret(&presented, &l.hash) {
                found = Some(i);
            }
        }
        found
    }

    /// These join codes of `file` no longer open anything: typing one says so. A code is dropped
    /// from the index once none of `file`'s links holds it.
    fn forget_codes(&mut self, file: &str, codes: impl IntoIterator<Item = Sealed>) {
        for c in codes {
            let held = self.files.get(file).is_some_and(|e| e.links.iter().any(|l| l.code.as_ref().is_some_and(|k| k.hash == c.hash)));
            if !held && let Some(files) = self.codes.get_mut(&c.hash) {
                files.remove(file);
                if files.is_empty() {
                    self.codes.remove(&c.hash);
                }
            }
            self.gone_codes.push_back(c.hash);
        }
        let extra = self.gone_codes.len().saturating_sub(MAX_GONE_CODES);
        self.gone_codes.drain(..extra);
    }

    /// Where the link with the join code `key` is: its file, its place among the file's links
    /// and its token. Only the files the index names for the code's hash are looked at; their
    /// links are each compared, in constant time.
    fn find_code(&self, key: &codes::Key) -> Option<(String, usize, String)> {
        let mut found = None;
        for id in self.codes.get(key.hash())? {
            let Some(e) = self.files.get(id) else { continue };
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
        let found_by = sealed.hash.clone();
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
        self.codes.entry(found_by).or_default().insert(file.to_string());
        self.forget_codes(file, before);
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
        self.forget_codes(file, gone.code);
        Ok(())
    }

    /// Remove a file and its links for good. `Err` when its folder couldn't be deleted (it is no
    /// longer served either way).
    pub fn remove(&mut self, file: &str) -> Result<(), String> {
        // Closed first: an open log would keep the folder from being deleted on Windows.
        let entry = self.files.remove(file).ok_or("this file isn't on the server (it may have been removed)")?;
        self.forget_codes(file, entry.links.iter().filter_map(|l| l.code.clone()));
        drop(entry);
        let dir = self.dir(file);
        fs::remove_dir_all(&dir)
            .map_err(|e| format!("it is no longer served, but its folder {} couldn't be deleted ({e}); delete it by hand", dir.display()))?;
        log::info!("removed file {file}");
        Ok(())
    }
}
