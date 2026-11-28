//! The team list (`team_files`): everyone who reaches the server sees the files shared from it now
//! and opens them with a team link, one per file, that the server makes the first time someone
//! asks for it and keeps (its token readable, unlike other links') while the file stays shared. A
//! file is shared while a link of its own (not a team link) has a join code: Share ▸ Start sharing
//! makes such links, Stop sharing revokes them, and with the last of them the team links go too
//! (whoever joined with one is disconnected, as with the links themselves).

use figcraft_collab::message::{MAX_TEAM, TeamFile};

use super::{Access, AdminReply, Entry, LINKS, NewLink, OpenFile, Store, TeamToken, new_link, now, write_json};

/// What a server without a team list answers.
pub const NO_TEAM: &str = "this server keeps no team list (its operator can turn one on with team_files)";

/// What asking for the link of a file that isn't shared now answers.
pub const NOT_SHARED: &str = "this file isn't shared now (its owner stopped sharing it, or removed it)";

/// The label of a team link (as `figcraft-server links list` shows it).
fn label(access: Access) -> String {
    format!("Team ({})", access.id())
}

/// Whether `entry` is shared now: a link of its own has a join code.
fn shared(entry: &Entry) -> bool {
    entry.links.iter().any(|l| l.team.is_none() && l.code.is_some())
}

impl Store {
    /// The files shared now, most recently changed first (at most [`MAX_TEAM`]).
    pub(super) fn team(&self, open: &dyn Fn(&str) -> Option<OpenFile>) -> AdminReply {
        let Some(access) = self.cfg.hosting.team else { return AdminReply::Failed { reason: NO_TEAM.into() } };
        let mut files: Vec<TeamFile> =
            self.files.iter().filter(|(_, e)| shared(e)).map(|(id, e)| TeamFile { file: self.info(id, e, open(id)), access }).collect();
        files.sort_by(|a, b| b.file.updated.cmp(&a.file.updated).then_with(|| a.file.name.cmp(&b.file.name)));
        files.truncate(MAX_TEAM);
        AdminReply::Team { web: self.cfg.hosting.web.clone(), files }
    }

    /// The link the team joins `file` with: made now when it has none with the team list's access.
    /// It replaces a team link made with another access before the operator changed the setting
    /// (that took a restart, so nobody is in the file with it).
    pub(super) fn team_link(&mut self, file: &str) -> Result<NewLink, String> {
        let access = self.cfg.hosting.team.ok_or(NO_TEAM)?;
        let max = self.cfg.max_links;
        let path = self.dir(file).join(LINKS);
        let entry = self.files.get_mut(file).filter(|e| shared(e)).ok_or(NOT_SHARED)?;
        let made = entry.links.iter().find_map(|l| match &l.team {
            Some(TeamToken(token)) if l.access == access => Some(NewLink { link: l.info(file), token: token.clone() }),
            _ => None,
        });
        if let Some(made) = made {
            return Ok(made);
        }
        // Made before the list changes: a failure leaves the links as they were.
        let (mut link, token) = new_link(access, &label(access), now())?;
        let before = entry.links.clone();
        entry.links.retain(|l| l.team.is_none());
        if entry.links.len() >= max {
            entry.links = before;
            return Err(format!("a file has at most {max} links: revoke one first"));
        }
        link.team = Some(TeamToken(token.clone()));
        entry.links.push(link.clone());
        if let Err(e) = write_json(&path, &entry.links) {
            entry.links = before;
            return Err(format!("couldn't keep the team link: {e}"));
        }
        log::info!("event=team_link file={file} link={}", link.id);
        Ok(NewLink { link: link.info(file), token })
    }

    /// At startup: the team links the list doesn't hand out now (it is off, or gives another
    /// access) are removed for good, so setting `team_files` back doesn't bring them back. Nobody
    /// keeps a team link's token for later (a team list link names none), so nothing is lost: the
    /// next join makes a new one. One that can't be removed stays refused at the door, and the
    /// operator is told.
    pub(super) fn drop_stale_team_links(&mut self) {
        let team = self.cfg.hosting.team;
        let stale = |l: &super::Link| l.team.is_some() && team != Some(l.access);
        let files: Vec<String> = self.files.iter().filter(|(_, e)| e.links.iter().any(stale)).map(|(id, _)| id.clone()).collect();
        for file in files {
            let path = self.dir(&file).join(LINKS);
            let Some(entry) = self.files.get_mut(&file) else { continue };
            entry.links.retain(|l| !stale(l));
            if let Err(e) = write_json(&path, &entry.links) {
                self.note(format!(
                    "file {file}: its old team links couldn't be removed ({e}); they are refused, but would work again if team_files were set back to their access"
                ));
            }
        }
    }

    /// `file` stopped being shared (a link was revoked): revoke its team links too. The ids
    /// revoked (whoever joined with them is disconnected); none while it is still shared.
    pub(super) fn team_unshared(&mut self, file: &str) -> Vec<String> {
        let path = self.dir(file).join(LINKS);
        let Some(entry) = self.files.get_mut(file).filter(|e| !shared(e)) else { return Vec::new() };
        let gone: Vec<String> = entry.links.iter().filter(|l| l.team.is_some()).map(|l| l.id.clone()).collect();
        if gone.is_empty() {
            return gone;
        }
        let before = entry.links.clone();
        entry.links.retain(|l| l.team.is_none());
        if let Err(e) = write_json(&path, &entry.links) {
            // Still refused at the door ([`team_admits`]) while the file isn't shared.
            log::error!("file {file}: its team links couldn't be revoked: {e}");
            entry.links = before;
            return Vec::new();
        }
        log::info!("revoked the team links of file {file}: it isn't shared now");
        gone
    }
}

/// Whether link `at` of `entry` lets people in, on a server whose team list gives `team`: a team
/// link only while the list is on with its access and the file is shared; any other link always.
pub(super) fn team_admits(team: Option<Access>, entry: &Entry, at: usize) -> bool {
    entry.links.get(at).is_some_and(|l| l.team.is_none() || (team == Some(l.access) && shared(entry)))
}
