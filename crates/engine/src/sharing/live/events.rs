use super::host::{clean_hosting, clean_shared};
use super::types::{Event, MAX_PEERS, PageInfo, Role, SessionId, SessionInfo, Status};
use super::{
    Access, Arc, Client, ClientMsg, DocState, Document, EngineError, Live, NodeId, PUBLISHING, Peer, Presence, Result, ServerMsg, Session, Signal,
    ToolId, display_name, editing_hint, editing_of, limited_by, sanitize, tool_left,
};

/// What a live session's document holding components that contain each other's instances shows
/// (`names`: the loop): those instances stop expanding where they would show themselves again.
fn loop_notice(names: &str) -> String {
    format!("Components contain each other's instances ({names}): those instances stop where they would show themselves")
}

/// A session that starts reconnecting forgets who's here (that comes again with the next welcome;
/// edits wait for it), and the host's signal support until it says it reads them again (a
/// spotlight of yours goes out again then).
fn reset_for_reconnect(live: &mut Live) {
    live.peers.clear();
    live.presence.clear();
    live.sent = None;
    live.signals = false;
    live.chats.clear();
    live.chat.clear();
    live.spotlight = None;
    if let Some(c) = live.client.as_mut() {
        c.disconnect();
    }
}

/// Joined, or joined again: who you are and who else is here (at most [`MAX_PEERS`] of them).
fn welcome_in(live: &mut Live, mut info: SessionInfo, peers: Vec<Peer>) {
    // The shared file's name and pages come with the document (the sync welcome).
    if let Some(old) = live.info.as_ref().filter(|_| info.file_name.is_empty()) {
        info.file_name = old.file_name.clone();
        info.pages = old.pages.clone();
    }
    live.status = Status::Live;
    live.info = Some(info);
    live.peers = peers.into_iter().take(MAX_PEERS).map(|p| (p.client_id, p)).collect();
    live.presence.retain(|id, _| live.peers.contains_key(id));
    if live.follow.is_some_and(|f| !live.peers.contains_key(&f)) {
        live.unfollow();
    }
    // Others need your presence again.
    live.sent = None;
}

impl Session {
    /// The active document (`None`: no document is open).
    pub(super) fn tab(&self) -> Option<u64> {
        self.ds().ok().map(|d| d.uid)
    }

    /// The active document's live session (status off when it isn't in one). Everything the
    /// window shows and the `collab.*` commands act on is this one.
    pub fn live(&self) -> &Live {
        self.live.on(self.tab())
    }

    /// The active document's live session, when it has one (running, on its way or failed).
    pub fn live_mut(&mut self) -> Option<&mut Live> {
        let id = self.live.shown(self.tab())?;
        self.live.sessions.get_mut(&id)
    }

    /// Document `uid`'s live session (status off when it isn't in one).
    pub fn live_of(&self, uid: u64) -> &Live {
        self.live.on(Some(uid))
    }

    /// The active document is its session's synced document: page and layer ids in others'
    /// presence refer to it.
    pub fn live_shared(&self) -> bool {
        self.tab().is_some_and(|uid| self.live_doc(uid))
    }

    /// Install a document that came from the session (others' edits): the selection, page and view
    /// stay where they can, and the undo history isn't touched.
    pub fn apply_remote(&mut self, uid: u64, doc: Arc<Document>) {
        let Some(old) = self.docs.iter().find(|d| d.uid == uid).map(|d| d.doc.clone()) else { return };
        // Edits merged from others can close a loop of components containing each other.
        let changed = crate::derived::changed_nodes(&doc, &old);
        if let Some(names) = crate::components::new_cycle(&old, &doc, changed.iter()) {
            self.message = Some(loop_notice(&names));
        }
        let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid) else { return };
        ds.doc = doc;
        if !ds.doc.pages.contains(&ds.page) {
            ds.page = ds.doc.first_page().unwrap_or(ds.page);
        }
        if ds.text_edit.as_ref().is_some_and(|t| !ds.doc.contains(t.id)) {
            ds.text_edit = None;
        }
        if ds.vector_edit.as_ref().is_some_and(|v| !ds.doc.contains(v.id)) {
            ds.vector_edit = None;
        }
        ds.clean_selection();
        ds.revision += 1;
    }

    /// Your presence in the active document's session now: its page, selection, viewport and
    /// edited text, and the pointer.
    pub fn live_presence(&self) -> Presence {
        self.presence_in(self.live())
    }

    /// Your presence in `live` (nothing while another document than its own is in front).
    pub(super) fn presence_in(&self, live: &Live) -> Presence {
        let Some(ds) = self.ds().ok().filter(|ds| live.tab == Some(ds.uid)) else { return Presence::default() };
        let (x0, y0) = ds.view.origin();
        let z = ds.view.zoom.max(1e-9);
        sanitize(Presence {
            cursor: live.cursor.map(|(x, y)| (ds.page, x, y)),
            selection: Some(ds.selection.clone()),
            viewport: Some((ds.page, x0, y0, ds.view.vw / z, ds.view.vh / z)),
            editing_text: ds.text_edit.as_ref().map(|t| Box::new(editing_of(t))),
        })
    }

    /// The others typing in text layer `id` of the active document, in its session.
    pub fn live_text_editors(&self, id: NodeId) -> Vec<&Peer> {
        if !self.live_shared() {
            return Vec::new();
        }
        let live = self.live();
        live.presence.iter().filter(|(_, p)| p.editing_text.as_ref().is_some_and(|e| e.node == id)).filter_map(|(c, _)| live.peers.get(c)).collect()
    }

    /// You started typing in text layer `id`: say so when someone else is typing in it too (both
    /// edits sync when they finish; the one finished last wins).
    pub(crate) fn live_text_notice(&mut self, id: NodeId) {
        let names: Vec<&str> = self.live_text_editors(id).iter().map(|p| p.name.as_str()).collect();
        if let Some(hint) = editing_hint(&names) {
            self.message = Some(format!("{hint} too: the version finished last is kept"));
        }
    }

    /// Take in what happened in every session (the UI calls it every frame; the network wakes the
    /// UI when something arrives). Returns whether anything changed.
    pub fn live_poll(&mut self) -> bool {
        let Some(link) = self.live.link.as_mut() else { return false };
        let events = link.poll();
        if events.is_empty() {
            return false;
        }
        for (id, e) in events {
            self.live_event(id, e);
        }
        self.live_closed();
        for id in self.live.sessions.keys().copied().collect::<Vec<_>>() {
            self.live_deferred(id);
            self.live_follow_apply(id);
        }
        self.online_recoveries();
        true
    }

    fn live_event(&mut self, id: SessionId, e: Event) {
        let e = match e {
            Event::Library { link, reply } => return self.library_answer(&link, reply),
            Event::Net { job, reply } => return self.background_answer(job, *reply),
            e => e,
        };
        // A session left meanwhile: what was still on its way is dropped.
        let front = self.tab();
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        // What happens in a file that isn't in front says which file (by its tab's name).
        let open = self.docs.iter().find(|d| Some(d.uid) == live.doc_uid).map(|d| d.doc.name.as_str());
        let behind = open.or(live.file_name()).filter(|_| live.tab != front).map(|n| format!(" {n}")).unwrap_or_default();
        match e {
            Event::Status(Status::Failed { reason }) => self.live_fail(id, reason),
            Event::Status(Status::Off) => {
                self.live.end(id);
            }
            Event::Status(st) => {
                if let Status::Reconnecting { .. } = st {
                    reset_for_reconnect(live);
                }
                live.status = st;
            }
            Event::Welcome { info, peers } => welcome_in(live, info, peers),
            Event::PeerJoined(p) => {
                // Someone already here was renamed: no newcomer to announce.
                if !live.peers.contains_key(&p.client_id) {
                    if live.peers.len() >= MAX_PEERS {
                        log::warn!("live session: {MAX_PEERS} people already here; leaving out another");
                        return;
                    }
                    self.message = Some(format!("{} joined{behind}", p.name));
                }
                live.peers.insert(p.client_id, p);
            }
            Event::PeerLeft(peer) => {
                live.presence.remove(&peer);
                live.chats.remove(&peer);
                if live.spotlight == Some(peer) {
                    live.spotlight = None;
                }
                if let Some(p) = live.peers.remove(&peer) {
                    self.message = Some(format!("{} left{behind}", p.name));
                }
                if live.follow == Some(peer) {
                    live.unfollow();
                }
            }
            Event::Presence(peer, p) => {
                if live.peers.contains_key(&peer) {
                    live.presence.insert(peer, p);
                }
            }
            Event::Signal(peer, sig) => self.live_signal_in(id, peer, sig),
            Event::Sync(welcome @ ServerMsg::Welcome { .. }) if live.client.is_none() => self.live_welcome(id, welcome),
            // Batches, and the welcome back after a dropped connection: the client catches up in
            // order.
            Event::Sync(msg) => live.deferred.push(msg),
            Event::Ended { reason } => {
                self.message = Some(if behind.is_empty() { reason } else { format!("{}: {reason}", behind.trim_start()) });
                self.live.end(id);
            }
            Event::Published(p) => {
                // Shared with a code: the popover shows it, nothing more to say.
                if live.share.is_none() {
                    self.message = Some(format!("Published {} to the server", p.name));
                }
                let server = live.server.clone();
                live.published = Some(p.clone());
                self.online_published(&server, &p);
            }
            Event::Shared(Ok(links)) => {
                let links = clean_shared(links);
                match live.server_file() {
                    Some((server, file)) => self.online_shared(&server, &file, Some(links)),
                    None => {
                        if let Some(share) = live.share.as_mut() {
                            share.links = Some(links);
                        }
                    }
                }
            }
            Event::Shared(Err(why)) => self.message = Some(format!("The file is on the server, but couldn't be shared: {why}")),
            Event::Hosting(info) => live.hosting = Some(clean_hosting(info)),
            Event::Reported(error) => {
                self.message =
                    Some(error.map_or_else(|| "Your report was sent to the server's operator".into(), |e| format!("The report wasn't sent: {e}")));
            }
            // Answered above.
            Event::Library { .. } | Event::Net { .. } => {}
        }
    }

    /// Someone's cursor chat or spotlight in session `id` (checked again: the host is untrusted
    /// too), or the host saying it reads them.
    fn live_signal_in(&mut self, id: SessionId, peer: u32, sig: Signal) {
        let Some(sig) = sig.clean() else { return };
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let name = live.peers.get(&peer).map(|p| p.name.clone());
        match (sig.kind.as_str(), name) {
            (Signal::READY, _) if live.info.as_ref().is_some_and(|i| i.me == peer) => {
                live.signals = true;
                if live.spotlighting {
                    self.live.send(id, [ClientMsg::Signal(Signal::spotlight(true))]);
                }
                // Renamed while away, or before the session read signals: the host's hello had
                // the old name.
                self.live_send_name(id);
            }
            (Signal::CHAT, Some(_)) if sig.text.is_empty() => {
                live.chats.remove(&peer);
            }
            (Signal::CHAT, Some(_)) => {
                live.chats.insert(peer, sig.text);
            }
            (Signal::SPOTLIGHT, Some(name)) if sig.on() => {
                // Theirs takes over from yours or anyone else's.
                live.spotlight = Some(peer);
                live.spotlighting = false;
                live.follow = Some(peer);
                live.followed = None;
                self.message = Some(format!("{name} is spotlighting: you follow their view (move it to stop)"));
            }
            (Signal::SPOTLIGHT, Some(name)) if live.spotlight == Some(peer) => {
                live.spotlight = None;
                if live.follow == Some(peer) {
                    live.unfollow();
                }
                self.message = Some(format!("{name} stopped spotlighting"));
            }
            _ => {}
        }
    }

    /// Your name from Preferences changed: every session you're in shows you by it.
    pub(crate) fn live_rename(&mut self) {
        let ids: Vec<SessionId> = self.live.sessions.keys().copied().collect();
        for id in ids {
            self.live_send_name(id);
        }
    }

    /// Tell session `id` your name from Preferences when it shows you by another one. The default
    /// name isn't sent: the name the session gave you ("Guest N") stays. A host older than
    /// renaming ignores it.
    fn live_send_name(&mut self, id: SessionId) {
        let name = display_name(&self.prefs);
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        if name.is_empty() || !live.signals {
            return;
        }
        let Some(info) = live.info.as_mut().filter(|i| i.name != name) else { return };
        info.name.clone_from(&name);
        self.live.send(id, [ClientMsg::Signal(Signal::name(&name))]);
    }

    /// Why cursor chat or a spotlight can't go out now, if it can't.
    pub(super) fn signal_blocker(&self) -> Option<&'static str> {
        let live = self.live();
        if live.status != Status::Live {
            Some("cursor chat and spotlight need a live session")
        } else if !live.signals {
            Some("the session's host runs an older version without cursor chat and spotlight")
        } else {
            None
        }
    }

    /// Show `text` next to your pointer for everyone (empty closes it); returns it as sent
    /// (control characters out, at most [`Signal::MAX_CHAT`] characters).
    pub fn live_chat(&mut self, text: &str) -> Result<String> {
        let sig = Signal::chat(text).clean().unwrap_or_default();
        if sig.text.is_empty() && self.live().chat.is_empty() {
            return Ok(String::new());
        }
        if let Some(why) = self.signal_blocker() {
            return Err(EngineError::Disabled("collab.cursorChat".into(), why.into()));
        }
        let Some(live) = self.live_mut() else { return Ok(String::new()) };
        if sig.text != live.chat {
            live.chat.clone_from(&sig.text);
            let id = live.id;
            self.live.send(id, [ClientMsg::Signal(sig.clone())]);
        }
        Ok(sig.text)
    }

    /// Start (`on`) or stop asking everyone to follow your view.
    pub fn live_spotlight(&mut self, on: bool) -> Result<()> {
        if on == self.live().spotlighting {
            return Ok(());
        }
        if on && let Some(why) = self.signal_blocker() {
            return Err(EngineError::Disabled("collab.spotlight".into(), why.into()));
        }
        let Some(live) = self.live_mut() else { return Ok(()) };
        if on {
            live.unfollow();
            live.spotlight = None;
        }
        live.spotlighting = on;
        let id = live.id;
        self.live.send(id, [ClientMsg::Signal(Signal::spotlight(on))]);
        Ok(())
    }

    /// Session `id`'s welcome: start the sync client; the host keeps editing the document it
    /// shared, a guest opens the host's file as a new tab (or gets it again in that tab after a
    /// reconnect).
    fn live_welcome(&mut self, id: SessionId, welcome: ServerMsg) {
        let client = match Client::join(welcome) {
            Ok(c) => c,
            Err(e) => return self.live_fail(id, format!("Couldn't open the shared file: {e}")),
        };
        let Some(live) = self.live.sessions.get(&id) else { return };
        let existing = live.doc_uid.filter(|u| self.docs.iter().any(|d| d.uid == *u));
        let (uid, synced) = match (live.role(), existing) {
            (Some(Role::Host), Some(uid)) => {
                if let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid) {
                    let was_dirty = ds.is_dirty();
                    // Undo is per user from now on: earlier whole-document steps would revert
                    // others' edits.
                    ds.history.undo.clear();
                    ds.history.redo.clear();
                    ds.doc_mut().set_client_id(client.id());
                    if !was_dirty {
                        ds.mark_saved();
                    }
                }
                // Edits made since hosting started go out with the first sync.
                (uid, None)
            }
            (Some(Role::Guest), Some(uid)) => {
                // Joining again in this tab, or the file you published arriving from the server.
                let doc = Arc::new(client.document().clone());
                let was_dirty = self.docs.iter().find(|d| d.uid == uid).is_some_and(DocState::is_dirty);
                self.apply_remote(uid, doc.clone());
                if let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid) {
                    // Undo is per user from now on: earlier whole-document steps would revert
                    // others' edits.
                    ds.history.undo.clear();
                    ds.history.redo.clear();
                    if !was_dirty {
                        ds.mark_saved();
                    }
                }
                (uid, Some(doc))
            }
            (Some(Role::Guest), None) => {
                // The session's file gets a tab of its own, in front.
                let ds = DocState::new(client.document().clone(), None);
                self.message = Some(match crate::components::cycle_text(&ds.doc) {
                    Some(names) => format!("Joined {}. {}", ds.doc.name, loop_notice(&names)),
                    None => format!("Joined {}", ds.doc.name),
                });
                let opened = (ds.uid, Some(ds.doc.clone()));
                self.active = self.docs.len();
                self.docs.push(ds);
                opened
            }
            _ => return,
        };
        // A view or comment link: the editing tools are off.
        if let Some(why) = limited_by(client.access())
            && self.tab() == Some(uid)
            && !tool_left(why, self.tool.tool)
        {
            self.set_tool(ToolId::Move);
        }
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let doc = client.document();
        if let Some(info) = live.info.as_mut() {
            info.file_name = doc.name.clone();
            info.pages = doc.pages.iter().map(|p| PageInfo { id: p.0, name: doc.node(*p).map(|n| n.name.clone()).unwrap_or_default() }).collect();
        }
        live.doc_uid = Some(uid);
        live.tab = Some(uid);
        live.synced = synced;
        live.client = Some(client);
        live.deferred.clear();
    }

    /// Session `id` is over with a message: it couldn't be joined, the connection was lost for
    /// good or sync broke. Its tab shows why.
    pub(super) fn live_fail(&mut self, id: SessionId, reason: String) {
        log::warn!("live session: {reason}");
        self.live.leave(id);
        if let Some(live) = self.live.sessions.get_mut(&id) {
            live.reset();
            live.status = Status::Failed { reason: reason.clone() };
        }
        self.message = Some(reason);
    }

    /// Why document `uid` can't be edited now, if it can't: you may only view it (or view and
    /// comment on it: see [`live_comment_edit`](Self::live_comment_edit)) in its session, or it is
    /// on its way to a server.
    pub fn live_read_only(&self, uid: u64) -> Option<&'static str> {
        let live = self.live.doc(uid)?;
        match &live.client {
            Some(c) => limited_by(c.access()),
            None if live.status.active() && live.role() != Some(Role::Host) => Some(PUBLISHING),
            None => None,
        }
    }

    /// Whether `doc` is session document `uid` with nothing but comment threads changed, and the
    /// link it was joined with lets you comment: such an edit goes through where others are refused.
    pub(crate) fn live_comment_edit(&self, uid: u64, doc: &Document) -> bool {
        let client = self.live.doc(uid).and_then(|l| l.client.as_ref());
        client.is_some_and(|c| c.access() == Access::Comment && c.changes_only_comments(doc))
    }

    /// The active document can't be edited now (see [`live_read_only`](Self::live_read_only)).
    pub fn active_read_only(&self) -> Option<&'static str> {
        self.tab().and_then(|uid| self.live_read_only(uid))
    }
}
