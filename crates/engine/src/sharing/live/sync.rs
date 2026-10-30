use super::host::hosting_json;
use super::types::{NO_SESSION, SessionId, Share, SharedLink, Status};
use super::{
    Access, Arc, Client, EditingText, EngineError, EngineRecompute, FollowedView, Live, Result, Session, Value, ViewState, bad, busy, close, code,
    editing_of, json,
};

impl Session {
    /// Your client id in document `uid`'s live session (`None`: it isn't in one). Refused for
    /// `cmd` while the session is on its way: there is no id yet.
    pub(crate) fn live_client_id(&self, cmd: &str, uid: u64) -> Result<Option<u32>> {
        let Some(live) = self.live.doc(uid) else { return Ok(None) };
        match &live.client {
            Some(c) => Ok(Some(c.id())),
            None if live.status.active() => Err(EngineError::Disabled(cmd.into(), "wait until the file's live session has started".into())),
            None => Ok(None),
        }
    }

    /// Branch `id` of document `uid` got its base from the session document just now.
    pub(crate) fn live_branch_base(&mut self, uid: u64, id: &str) {
        if let Some(live) = self.live.doc_mut(uid).filter(|l| l.syncing()) {
            live.branch_bases.insert(id.to_string());
        }
    }

    /// Whether branch `id` of document `uid` got its base in the live session it is in now.
    pub(crate) fn live_branch_in_session(&self, uid: u64, id: &str) -> bool {
        self.live.doc(uid).is_some_and(|l| l.syncing() && l.branch_bases.contains(id))
    }

    /// Whether document `uid` is a live session's synced document: its edits sync and its undo
    /// history is the user's own.
    pub fn live_doc(&self, uid: u64) -> bool {
        self.live.doc(uid).is_some_and(Live::syncing)
    }

    /// Sync the edits of session document `uid` as one batch: one step of the user's undo history
    /// called `label`, or not undoable (`None`: also what changed it outside a command). Nothing
    /// while a gesture or preview shows, or inside a group (it syncs when the group ends).
    pub(crate) fn live_edit(&mut self, uid: u64, label: Option<&str>) {
        let Some(ds) = self.docs.iter().find(|d| d.uid == uid) else { return };
        let Some(live) = self.live.doc(uid) else { return };
        if live.hold > 0 || busy(ds) || live.synced.as_ref().is_some_and(|s| Arc::ptr_eq(s, &ds.doc)) {
            return;
        }
        let (id, doc, synced) = (live.id, ds.doc.clone(), live.synced.clone());
        if self.live_read_only(uid).is_some() && !self.live_comment_edit(uid, &doc) {
            // Nothing goes out: what changed outside a command is put back.
            if let Some(s) = synced {
                self.apply_remote(uid, s);
            }
            return;
        }
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let Some(client) = live.client.as_mut() else { return };
        match client.edit_with(&doc, label) {
            Ok(msg) => {
                live.synced = Some(doc);
                self.live.send(id, msg);
            }
            Err(e) => self.live_fail(id, format!("Couldn't sync an edit: {e}")),
        }
    }

    /// A group of commands on document `uid` starts (`on`) or ends: in between its edits don't
    /// sync on their own.
    pub(crate) fn live_hold(&mut self, uid: u64, on: bool) {
        if let Some(live) = self.live.doc_mut(uid).filter(|l| l.syncing()) {
            live.hold = if on { live.hold.saturating_add(1) } else { live.hold.saturating_sub(1) };
        }
    }

    /// Undo (`back`) or redo the user's own last step on the active document, in its session:
    /// what it wrote goes back where nobody else wrote since. Returns whether there was a step to
    /// take back.
    pub(crate) fn live_travel(&mut self, back: bool) -> bool {
        let Some(uid) = self.tab() else { return false };
        // Edits not synced yet go first.
        self.live_edit(uid, None);
        let Some(live) = self.live.doc_mut(uid) else { return false };
        let Some(client) = live.client.as_mut() else { return false };
        let (undo, redo) = client.history_len();
        let msgs = match if back { client.undo(&EngineRecompute) } else { client.redo(&EngineRecompute) } {
            Ok(m) => m,
            Err(e) => {
                log::warn!("live session: {} failed: {e}", if back { "undo" } else { "redo" });
                return false;
            }
        };
        let (u, r) = client.history_len();
        let did = if back { r > redo } else { u > undo };
        let doc = Arc::new(client.document().clone());
        if did {
            live.synced = Some(doc.clone());
        }
        let id = live.id;
        self.live.send(id, msgs);
        if did {
            self.apply_remote(uid, doc);
        }
        did
    }

    /// Apply the server's batches that waited in session `id` (none while a gesture or preview
    /// shows on its document).
    pub(super) fn live_deferred(&mut self, id: SessionId) {
        let Some(live) = self.live.sessions.get(&id).filter(|l| !l.deferred.is_empty() && l.syncing()) else { return };
        let Some(uid) = live.doc_uid else { return };
        if self.docs.iter().find(|d| d.uid == uid).is_none_or(busy) {
            return;
        }
        // Your own edits first: the client replays them over what others did.
        self.live_edit(uid, None);
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let Some(client) = live.client.as_mut() else { return };
        let mut out = Vec::new();
        for m in std::mem::take(&mut live.deferred) {
            match client.receive(m, &EngineRecompute) {
                Ok(more) => out.extend(more),
                Err(e) => return self.live_fail(id, format!("The live session got out of step ({e}); join again")),
            }
        }
        let doc = Arc::new(client.document().clone());
        // Edits the server refused were rolled back: say why (the UI's toast).
        let notices: Vec<String> = client.take_notices().iter().map(figcraft_collab::Notice::message).collect();
        live.synced = Some(doc.clone());
        if !notices.is_empty() {
            self.message = Some(notices.join("; "));
        }
        self.live.send(id, out);
        self.apply_remote(uid, doc);
    }

    /// Show the peer followed in session `id`: their viewport (and their page, in the session's
    /// document) on the session's tab.
    pub(super) fn live_follow_apply(&mut self, id: SessionId) {
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let Some(peer) = live.follow else { return };
        if !live.peers.contains_key(&peer) {
            live.unfollow();
            return;
        }
        let Some((page, x, y, w, h)) = live.presence.get(&peer).and_then(|p| p.viewport) else { return };
        let Some(ds) = self.docs.iter_mut().find(|d| Some(d.uid) == live.tab) else { return };
        if live.syncing() && page != ds.page && ds.doc.pages.contains(&page) {
            ds.page = page;
            ds.selection.clear();
            ds.text_edit = None;
        }
        if !(w > 0.0 && h > 0.0 && x.is_finite() && y.is_finite()) {
            return;
        }
        ds.view.cx = x + w / 2.0;
        ds.view.cy = y + h / 2.0;
        let zoom = (ds.view.vw / w).min(ds.view.vh / h);
        if zoom.is_finite() {
            ds.view.zoom = zoom.clamp(ViewState::MIN_ZOOM, ViewState::MAX_ZOOM);
        }
        ds.revision += 1;
        live.followed = Some(FollowedView { page: ds.page, cx: ds.view.cx, cy: ds.view.cy, zoom: ds.view.zoom });
    }

    /// Sessions whose tab was closed end (hosts stop hosting, guests leave); the others go on. A
    /// join still on its way from a closed tab shows on the one in front.
    pub(super) fn live_closed(&mut self) {
        let (docs, tab) = (&self.docs, self.tab());
        let mut gone = Vec::new();
        for l in self.live.sessions.values_mut().filter(|l| l.tab != tab && !docs.iter().any(|d| Some(d.uid) == l.tab)) {
            if l.doc_uid.is_none() && l.status.active() {
                l.tab = tab;
            } else {
                gone.push((l.id, l.status == Status::Live));
            }
        }
        for (id, was_live) in gone {
            self.live.end(id);
            if was_live {
                self.message = Some("The live session's file was closed, so you left the session".into());
            }
        }
    }

    /// Send your edits and presence in every session when they changed (headless sessions call
    /// it after each command). A view you moved yourself stops following.
    pub fn live_sync(&mut self) {
        self.live_sync_in(|_| true);
    }

    /// [`live_sync`](Self::live_sync) for the sessions whose document (`None`: a join on its way)
    /// `mine` picks: a window calls it every frame it draws, after input, for the documents it
    /// holds.
    pub fn live_sync_in(&mut self, mine: impl Fn(Option<u64>) -> bool) {
        // A recovery copy whose session was left has its outcome too.
        if !self.live.recovering.is_empty() {
            self.online_recoveries();
        }
        if self.live.sessions.is_empty() {
            return;
        }
        self.live_closed();
        let ids: Vec<SessionId> = self.live.sessions.values().filter(|l| l.status == Status::Live && mine(l.doc_uid)).map(|l| l.id).collect();
        for id in ids {
            self.live_deferred(id);
            if let Some(uid) = self.live.get(id).and_then(|l| l.doc_uid) {
                self.live_edit(uid, None);
            }
            self.live_present(id);
        }
    }

    /// Hand your presence in session `id` to the link when it changed.
    fn live_present(&mut self, id: SessionId) {
        let Some(live) = self.live.get(id).filter(|l| l.status == Status::Live) else { return };
        let moved = live.followed.is_some_and(|f| {
            self.docs
                .iter()
                .find(|d| Some(d.uid) == live.tab)
                .is_some_and(|ds| ds.page != f.page || !close(ds.view.cx, f.cx) || !close(ds.view.cy, f.cy) || !close(ds.view.zoom, f.zoom))
        });
        let p = self.presence_in(live);
        let changed = live.sent.as_ref() != Some(&p);
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        if moved {
            live.unfollow();
        }
        if changed {
            if let Some(link) = self.live.link.as_mut() {
                link.presence(id, &p);
            }
            live.sent = Some(p);
        }
    }

    /// Stop following in the active document's session (Escape, `collab.follow {}`). False when
    /// you weren't following anyone.
    pub fn live_unfollow(&mut self) -> bool {
        self.live_mut().is_some_and(Live::unfollow)
    }

    /// Follow `peer`'s viewport in the active document's session (shown at once when known).
    pub fn live_follow(&mut self, peer: u32) -> Result<()> {
        let Some(live) = self.live_mut().filter(|l| l.peers.contains_key(&peer)) else {
            return Err(bad("collab.follow", format!("nobody with client id {peer} is in the session")));
        };
        live.follow = Some(peer);
        live.followed = None;
        let id = live.id;
        self.live_follow_apply(id);
        Ok(())
    }

    /// The active document's session as JSON (`collab.peers`), with every session in brief.
    pub fn live_summary(&self) -> Value {
        self.summary(self.live())
    }

    /// The index of the open document session `l` syncs.
    fn live_index(&self, l: &Live) -> Option<usize> {
        l.doc_uid.and_then(|u| self.docs.iter().position(|d| d.uid == u))
    }

    pub(super) fn summary(&self, l: &Live) -> Value {
        let info = l.info.as_ref();
        let reason = |l: &Live| match &l.status {
            Status::Failed { reason } => Some(reason.clone()),
            _ => None,
        };
        let synced = self.live_index(l).and_then(|i| self.docs.get(i));
        // Who types in which layer of the session document.
        let editing = |e: &EditingText| {
            let layer = synced.and_then(|d| d.doc.node(e.node)).map(|n| n.name.as_str());
            json!({"node": e.node, "layer": layer, "caret": e.caret, "selection": e.selection})
        };
        let peers: Vec<Value> = l
            .peers
            .values()
            .map(|p| {
                let presence = l.presence.get(&p.client_id);
                let typing = presence.and_then(|pr| pr.editing_text.as_deref()).map(editing);
                let chat = l.chats.get(&p.client_id);
                json!({"id": p.client_id, "name": p.name, "color": p.color, "editing": typing, "chat": chat, "presence": presence})
            })
            .collect();
        let mine = synced.and_then(|d| d.text_edit.as_ref()).map(|t| editing(&editing_of(t)));
        let sessions: Vec<Value> = self
            .live
            .iter()
            .map(|o| {
                let info = o.info.as_ref();
                json!({
                    "session": o.id,
                    "document": self.live_index(o),
                    "status": o.status.id(),
                    "reason": reason(o),
                    "role": o.role(),
                    "fileName": info.map(|i| i.file_name.as_str()),
                    "link": info.map(|i| i.link.as_str()),
                    "peers": o.peers.len(),
                })
            })
            .collect();
        json!({
            "available": self.live.available(),
            "canHost": self.live.can_host(),
            "session": (l.id != NO_SESSION).then_some(l.id),
            "status": l.status.id(),
            "reason": reason(l),
            "attempt": match l.status { Status::Reconnecting { attempt } => Some(attempt), _ => None },
            "role": info.map(|i| i.role),
            "me": info.map(|i| i.me),
            "name": info.map(|i| i.name.as_str()),
            "color": info.map(|i| i.color.as_str()),
            "fileName": info.map(|i| i.file_name.as_str()),
            "pages": info.map(|i| &i.pages),
            "link": info.map(|i| i.link.as_str()),
            "url": info.map(|i| i.url.as_str()),
            "address": info.map(|i| i.address.as_str()),
            "network": info.map(|i| i.network),
            "access": l.client.as_ref().map(Client::access).or(info.map(|i| i.access)),
            "readOnly": l.doc_uid.is_some_and(|u| self.live_read_only(u).is_some()),
            "published": l.published,
            "code": l.share.as_ref().and_then(Share::code),
            "share": l.share.as_ref().map(|sh| {
                let of = |a| sh.links.as_ref().and_then(|s| s.of(a));
                let (edit, view, comment) = (of(Access::Edit), of(Access::View), of(Access::Comment));
                let code = |l: Option<&SharedLink>| l.and_then(|l| l.code.as_deref()).map(code::grouped);
                json!({
                    "via": sh.via,
                    "access": sh.access.id(),
                    "shared": sh.links.is_some(),
                    "editCode": code(edit),
                    "viewCode": code(view),
                    "commentCode": code(comment),
                    "editLink": edit.map(|l| l.link.as_str()),
                    "viewLink": view.map(|l| l.link.as_str()),
                    "commentLink": comment.map(|l| l.link.as_str()),
                })
            }),
            "hosting": l.hosting.as_ref().map(hosting_json),
            "follow": l.follow,
            "signals": l.signals,
            "chat": l.chat,
            "spotlight": l.spotlight,
            "spotlighting": l.spotlighting,
            "syncing": l.syncing(),
            "document": self.live_index(l),
            "editing": mine,
            "peers": peers,
            "sessions": sessions,
        })
    }
}
