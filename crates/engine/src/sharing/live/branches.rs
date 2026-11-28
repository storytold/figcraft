//! Shared branches in a live session: sync branch creation, edits, updates from main, and
//! merge/archive state changes across everyone connected to the same server file.

use std::sync::Arc;

use figcraft_collab::message::{BranchEvent, BranchReq, BranchStatus, ClientMsg, SharedBranch};
use figcraft_collab::{Access, Change, Mode, Replica, diff};
use figcraft_doc::Document;
use figcraft_format::{Branch, BranchState};

use super::types::SessionId;
use super::{Session, busy};
use crate::branches;

/// Synced state of one shared branch on this client.
pub(super) struct SyncedBranch {
    /// Confirmed base on the server.
    pub(super) base: Replica,
    /// Confirmed head on the server (updated only from server events).
    pub(super) confirmed: Replica,
    /// Optimistic local view (`confirmed` with `pending` applied on top).
    pub(super) view: Replica,
    /// Local edits awaiting server echo (or reconnect).
    pub(super) pending: Vec<Vec<Change>>,
    /// The tab document as last diffed or installed.
    pub(super) synced: Arc<Document>,
    /// Whether the server has confirmed this branch exists.
    pub(super) confirmed_on_server: bool,
    /// Original create request while awaiting server confirmation.
    pub(super) pending_create: Option<BranchReq>,
}

impl SyncedBranch {
    fn rebuild_view(&mut self) {
        let mut next = self.confirmed.clone();
        for batch in &self.pending {
            let _ = next.apply(batch, Mode::Lenient);
        }
        self.view = next;
    }
}

fn to_status(s: BranchState) -> BranchStatus {
    match s {
        BranchState::Active => BranchStatus::Active,
        BranchState::Merged => BranchStatus::Merged,
        BranchState::Archived => BranchStatus::Archived,
    }
}

fn from_status(s: BranchStatus) -> BranchState {
    match s {
        BranchStatus::Active => BranchState::Active,
        BranchStatus::Merged => BranchState::Merged,
        BranchStatus::Archived => BranchState::Archived,
    }
}

impl Session {
    /// Whether branch `id` of document `uid` is synced with the live session's server.
    pub(crate) fn live_branch_shared(&self, uid: u64, id: &str) -> bool {
        self.live.doc(uid).is_some_and(|l| l.syncing() && l.shared_branches && l.branch_replicas.contains_key(id))
    }

    /// Share a newly created branch of live document `uid` with the server.
    pub(crate) fn live_branch_create(&mut self, uid: u64, branch: &Branch) {
        let Some((session_id, base_seq, confirmed)) = self
            .live
            .doc(uid)
            .filter(|l| l.shared_branches && l.client.as_ref().is_some_and(|c| c.access() == Access::Edit))
            .and_then(|l| l.client.as_ref().map(|c| (l.id, c.seq(), c.confirmed().clone())))
        else {
            return;
        };
        let Ok(base_diff) = diff(&confirmed, &branch.base) else { return };
        let base_rep = base_diff.replica;
        let Ok(head_diff) = diff(&base_rep, &branch.head) else { return };
        let head_rep = head_diff.replica;
        let synced = branch.head.clone();
        let req = BranchReq::Create {
            id: branch.id.clone(),
            name: branch.name.clone(),
            time: branch.time,
            base_seq,
            base_diff: base_diff.changes,
            head_diff: head_diff.changes,
        };
        if let Some(live) = self.live.sessions.get_mut(&session_id) {
            live.branch_replicas.insert(
                branch.id.clone(),
                SyncedBranch {
                    base: base_rep,
                    confirmed: head_rep.clone(),
                    view: head_rep,
                    pending: Vec::new(),
                    synced,
                    confirmed_on_server: false,
                    pending_create: Some(req.clone()),
                },
            );
        }
        self.live.send(session_id, [ClientMsg::Branch(Box::new(req))]);
    }

    /// Push local edits of shared branch `branch_id` (of main document `main_uid`) to the server,
    /// or queue them in `pending` while disconnected.
    pub(crate) fn live_branch_edit(&mut self, main_uid: u64, branch_id: &str) {
        let Some(tab_idx) = branches::tab_of(self, main_uid, branch_id) else { return };
        let Some(tab) = self.docs.get(tab_idx) else { return };
        if busy(tab) {
            return;
        }
        let doc = tab.document_itself().clone();
        let Some(live) = self.live.doc_mut(main_uid) else { return };
        if !live.client.as_ref().is_some_and(|c| c.access() == Access::Edit) {
            return;
        }
        let session_id = live.id;
        let send_now = live.shared_branches;
        let Some(sb) = live.branch_replicas.get_mut(branch_id) else { return };
        if Arc::ptr_eq(&sb.synced, &doc) {
            return;
        }
        let Ok(d) = diff(&sb.view, &doc) else { return };
        sb.view = d.replica;
        sb.synced = doc.clone();
        if !d.changes.is_empty() {
            sb.pending.push(d.changes.clone());
            if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
                && let Some(b) = m.branches.iter_mut().find(|b| b.id == branch_id)
            {
                b.head = doc;
            }
            if send_now {
                self.live.send(session_id, [ClientMsg::Branch(Box::new(BranchReq::Edit { id: branch_id.to_string(), changes: d.changes }))]);
            }
        }
    }

    /// Sync edits on all open shared branch tabs belonging to live document `main_uid`.
    pub(super) fn live_branches_sync(&mut self, main_uid: u64) {
        let can_sync =
            self.live.doc(main_uid).is_some_and(|l| !l.branch_replicas.is_empty() && l.client.as_ref().is_some_and(|c| c.access() == Access::Edit));
        if !can_sync {
            return;
        }
        let ids: Vec<String> = branches::tabs(self, main_uid).filter_map(|t| t.branch_of.as_ref().map(|(_, id)| id.clone())).collect();
        for id in ids {
            self.live_branch_edit(main_uid, &id);
        }
    }

    /// Tell the server that shared branch `branch_id` was updated from main.
    pub(crate) fn live_branch_update_from_main(&mut self, main_uid: u64, branch_id: &str, new_base: &Arc<Document>, new_head: &Arc<Document>) {
        let Some(live) = self.live.doc_mut(main_uid) else { return };
        if !live.shared_branches || !live.client.as_ref().is_some_and(|c| c.access() == Access::Edit) {
            return;
        }
        let session_id = live.id;
        let Some(sb) = live.branch_replicas.get_mut(branch_id) else { return };
        let Ok(bd) = diff(&sb.base, new_base) else { return };
        let Ok(hd) = diff(&sb.view, new_head) else { return };
        sb.base = bd.replica;
        sb.view = hd.replica;
        sb.synced = new_head.clone();
        sb.pending.push(hd.changes.clone());
        let req = BranchReq::UpdateFromMain { id: branch_id.to_string(), base_changes: bd.changes, head_changes: hd.changes };
        self.live.send(session_id, [ClientMsg::Branch(Box::new(req))]);
    }

    /// Most recent main-document `ClientMsg::Changes` batch id sent for live document `main_uid`.
    pub(crate) fn live_last_batch(&self, main_uid: u64) -> Option<u64> {
        self.live.doc(main_uid).and_then(|l| l.last_batch)
    }

    /// Flush the main-document merge batch for shared branch `branch_id` and defer
    /// `SetState(Merged)` until the server confirms that batch (`before_batch`: the last batch id
    /// sent before the merge was recorded).
    pub(crate) fn live_branch_merge(&mut self, main_uid: u64, branch_id: &str, before_batch: Option<u64>) {
        if !self.live_branch_shared(main_uid, branch_id) {
            return;
        }
        let _ = self.live_edit_batch(main_uid, Some("Merge branch"));
        let sent = self.live_last_batch(main_uid).filter(|b| Some(*b) != before_batch);
        if let Some(batch_id) = sent {
            if let Some(live) = self.live.doc_mut(main_uid) {
                live.pending_merges.insert(batch_id, branch_id.to_string());
            }
        } else {
            self.live_branch_set_state(main_uid, branch_id, BranchState::Merged);
        }
    }

    /// Complete or roll back a deferred shared-branch merge once the server accepts or rejects its
    /// main-document batch.
    pub(super) fn live_branch_merge_finished(&mut self, main_uid: u64, branch_id: &str, accepted: bool) {
        if accepted {
            self.live_branch_set_state(main_uid, branch_id, BranchState::Merged);
        } else if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
            && let Some(b) = m.branches.iter_mut().find(|b| b.id == branch_id && b.state == BranchState::Merged)
        {
            b.state = BranchState::Active;
        }
    }

    /// Tell the server that shared branch `branch_id` was merged or archived.
    pub(crate) fn live_branch_set_state(&mut self, main_uid: u64, branch_id: &str, state: BranchState) {
        let Some(session_id) = self
            .live
            .doc(main_uid)
            .filter(|l| {
                l.shared_branches && l.branch_replicas.contains_key(branch_id) && l.client.as_ref().is_some_and(|c| c.access() == Access::Edit)
            })
            .map(|l| l.id)
        else {
            return;
        };
        let req = BranchReq::SetState { id: branch_id.to_string(), status: to_status(state) };
        self.live.send(session_id, [ClientMsg::Branch(Box::new(req))]);
    }

    /// Initial list of shared branches sent by the server right after `Welcome` (or after a
    /// reconnect). Preserves and resends any pending or offline edits made on shared branches.
    pub(super) fn live_branches_welcome(&mut self, session_id: SessionId, list: Vec<SharedBranch>) {
        let Some((main_uid, can_edit, confirmed)) = self.live.sessions.get(&session_id).and_then(|l| {
            let uid = l.doc_uid?;
            let c = l.client.as_ref()?;
            Some((uid, c.access() == Access::Edit, c.confirmed().clone()))
        }) else {
            return;
        };
        // Capture any edits made on open branch tabs while disconnected before reconciling.
        self.live_branches_sync(main_uid);
        let mut previous = match self.live.sessions.get_mut(&session_id) {
            Some(l) => {
                l.shared_branches = true;
                std::mem::take(&mut l.branch_replicas)
            }
            None => return,
        };
        let mut resend = Vec::new();
        for wb in list {
            let old = previous.remove(&wb.id);
            if let Some(msg) = self.install_welcome_branch(session_id, main_uid, &confirmed, wb, old, can_edit) {
                resend.push(msg);
            }
        }
        // Re-send any branch created right before a disconnect that never reached the server.
        if can_edit {
            for (id, old) in previous {
                if !old.confirmed_on_server
                    && let Some(create_req) = old.pending_create.clone()
                {
                    resend.push(ClientMsg::Branch(Box::new(create_req)));
                    for batch in &old.pending {
                        if !batch.is_empty() {
                            resend.push(ClientMsg::Branch(Box::new(BranchReq::Edit { id: id.clone(), changes: batch.clone() })));
                        }
                    }
                    if let Some(live) = self.live.sessions.get_mut(&session_id) {
                        live.branch_replicas.insert(id, old);
                    }
                }
            }
        }
        if !resend.is_empty() {
            self.live.send(session_id, resend);
        }
    }

    fn install_welcome_branch(
        &mut self,
        session_id: SessionId,
        main_uid: u64,
        confirmed: &Replica,
        wb: SharedBranch,
        previous: Option<SyncedBranch>,
        can_edit: bool,
    ) -> Option<ClientMsg> {
        let mut base_rep = confirmed.clone();
        if base_rep.apply(&wb.base_diff, Mode::Lenient).is_err() {
            return None;
        }
        let mut head_rep = base_rep.clone();
        if head_rep.apply(&wb.head_diff, Mode::Lenient).is_err() {
            return None;
        }
        let mut pending = Vec::new();
        let mut view_rep = head_rep.clone();
        let mut to_send = None;
        if can_edit
            && wb.status == BranchStatus::Active
            && let Some(old) = previous
            && !old.pending.is_empty()
        {
            let mut replayed = head_rep.clone();
            for batch in &old.pending {
                let _ = replayed.apply(batch, Mode::Lenient);
            }
            if let Ok(net) = diff(&head_rep, replayed.document())
                && !net.changes.is_empty()
            {
                view_rep = net.replica;
                pending.push(net.changes.clone());
                to_send = Some(ClientMsg::Branch(Box::new(BranchReq::Edit { id: wb.id.clone(), changes: net.changes })));
            }
        }
        let base_doc = Arc::new(base_rep.document().clone());
        let head_doc = Arc::new(view_rep.document().clone());
        let id = wb.id.clone();
        let state = from_status(wb.status);
        if state == BranchState::Active
            && let Some(live) = self.live.sessions.get_mut(&session_id)
        {
            live.branch_replicas.insert(
                id.clone(),
                SyncedBranch {
                    base: base_rep,
                    confirmed: head_rep,
                    view: view_rep,
                    pending,
                    synced: head_doc.clone(),
                    confirmed_on_server: true,
                    pending_create: None,
                },
            );
        }
        if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid) {
            if let Some(existing) = m.branches.iter_mut().find(|b| b.id == id) {
                existing.name = wb.name;
                existing.state = state;
                existing.base = base_doc;
                existing.head = head_doc.clone();
            } else {
                m.branches.push(Branch {
                    id: wb.id,
                    name: wb.name,
                    time: wb.time,
                    client: wb.client,
                    state,
                    base: base_doc,
                    head: head_doc.clone(),
                    versions: Vec::new(),
                });
            }
        }
        self.update_open_branch_tab(session_id, main_uid, &id, head_doc);
        to_send
    }

    /// A live shared-branch update from the server.
    pub(super) fn live_branch_event(&mut self, session_id: SessionId, ev: BranchEvent) {
        let Some((main_uid, me, confirmed)) =
            self.live.sessions.get(&session_id).and_then(|l| Some((l.doc_uid?, l.client.as_ref()?.id(), l.client.as_ref()?.confirmed().clone())))
        else {
            return;
        };
        match ev {
            BranchEvent::Created { author, branch } => {
                let name = branch.name.clone();
                self.install_shared_branch(session_id, main_uid, &confirmed, branch, author == me);
                if author != me {
                    let who = self.peer_name(session_id, author);
                    self.message = Some(format!("{who} created branch “{name}”"));
                }
            }
            BranchEvent::Edited { id, author, changes, .. } => {
                self.apply_remote_branch_edit(session_id, main_uid, &id, author == me, &changes);
            }
            BranchEvent::UpdatedFromMain { id, author, base_changes, head_changes, .. } => {
                self.apply_remote_branch_update(session_id, main_uid, &id, author == me, &base_changes, &head_changes);
            }
            BranchEvent::State { id, author, status, .. } => {
                let state = from_status(status);
                let mut name = None;
                if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
                    && let Some(b) = m.branches.iter_mut().find(|b| b.id == id)
                {
                    b.state = state;
                    name = Some(b.name.clone());
                }
                if state != BranchState::Active {
                    if let Some(live) = self.live.sessions.get_mut(&session_id) {
                        live.branch_replicas.remove(&id);
                    }
                    if author != me {
                        let was_active = branches::tab_of(self, main_uid, &id) == Some(self.active);
                        branches::close_tabs(self, main_uid, Some(&id));
                        if was_active && let Some(mi) = self.docs.iter().position(|d| d.uid == main_uid) {
                            self.active = mi;
                        }
                    }
                }
                if author != me
                    && let Some(name) = name
                {
                    let who = self.peer_name(session_id, author);
                    let verb = if state == BranchState::Merged { "merged" } else { "archived" };
                    self.message = Some(format!("{who} {verb} branch “{name}”"));
                }
            }
            BranchEvent::Rejected { id, reason } => {
                self.handle_branch_rejected(session_id, main_uid, &id, &reason);
            }
        }
    }

    fn peer_name(&self, session_id: SessionId, peer_id: u32) -> String {
        self.live.sessions.get(&session_id).and_then(|l| l.peers.get(&peer_id)).map_or_else(|| "Someone".to_string(), |p| p.name.clone())
    }

    fn handle_branch_rejected(&mut self, session_id: SessionId, main_uid: u64, id: &str, reason: &str) {
        let Some(live) = self.live.sessions.get_mut(&session_id) else { return };
        let Some(sb) = live.branch_replicas.get_mut(id) else {
            self.message = Some(format!("Shared branch change refused: {reason}"));
            return;
        };
        if !sb.confirmed_on_server {
            live.branch_replicas.remove(id);
            self.message = Some(format!("Shared branch wasn't synced: {reason}"));
            return;
        }
        sb.pending.clear();
        sb.view = sb.confirmed.clone();
        let rollback = Arc::new(sb.confirmed.document().clone());
        sb.synced = rollback.clone();
        if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
            && let Some(b) = m.branches.iter_mut().find(|b| b.id == id)
        {
            b.head = rollback.clone();
        }
        self.update_open_branch_tab(session_id, main_uid, id, rollback);
        self.message = Some(format!("Shared branch change refused: {reason}"));
    }

    fn install_shared_branch(&mut self, session_id: SessionId, main_uid: u64, confirmed: &Replica, wb: SharedBranch, own_echo: bool) {
        let mut base_rep = confirmed.clone();
        if base_rep.apply(&wb.base_diff, Mode::Lenient).is_err() {
            return;
        }
        let mut head_rep = base_rep.clone();
        if head_rep.apply(&wb.head_diff, Mode::Lenient).is_err() {
            return;
        }
        let base_doc = Arc::new(base_rep.document().clone());
        let id = wb.id.clone();
        if own_echo {
            // Confirm server creation and keep both base and confirmed head in step with the server
            // (replaying any local edits the author already made on top).
            let head_doc = if let Some(live) = self.live.sessions.get_mut(&session_id)
                && let Some(sb) = live.branch_replicas.get_mut(&id)
            {
                sb.confirmed_on_server = true;
                sb.pending_create = None;
                sb.base = base_rep;
                sb.confirmed = head_rep;
                sb.rebuild_view();
                let doc = Arc::new(sb.view.document().clone());
                sb.synced = doc.clone();
                Some(doc)
            } else {
                None
            };
            if let Some(head_doc) = head_doc {
                if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
                    && let Some(b) = m.branches.iter_mut().find(|b| b.id == id)
                {
                    b.base = base_doc;
                    b.head = head_doc.clone();
                }
                self.update_open_branch_tab(session_id, main_uid, &id, head_doc);
            }
            return;
        }
        let head_doc = Arc::new(head_rep.document().clone());
        if let Some(live) = self.live.sessions.get_mut(&session_id) {
            live.branch_replicas.insert(
                id.clone(),
                SyncedBranch {
                    base: base_rep,
                    confirmed: head_rep.clone(),
                    view: head_rep,
                    pending: Vec::new(),
                    synced: head_doc.clone(),
                    confirmed_on_server: true,
                    pending_create: None,
                },
            );
        }
        let state = from_status(wb.status);
        if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid) {
            if let Some(existing) = m.branches.iter_mut().find(|b| b.id == id) {
                existing.name = wb.name;
                existing.state = state;
                existing.base = base_doc;
                existing.head = head_doc.clone();
            } else {
                m.branches.push(Branch {
                    id: wb.id,
                    name: wb.name,
                    time: wb.time,
                    client: wb.client,
                    state,
                    base: base_doc,
                    head: head_doc.clone(),
                    versions: Vec::new(),
                });
            }
        }
        self.update_open_branch_tab(session_id, main_uid, &id, head_doc);
    }

    fn apply_remote_branch_edit(&mut self, session_id: SessionId, main_uid: u64, id: &str, own_echo: bool, changes: &[Change]) {
        let Some(live) = self.live.sessions.get_mut(&session_id) else { return };
        let Some(sb) = live.branch_replicas.get_mut(id) else { return };
        if !changes.is_empty() && sb.confirmed.apply(changes, Mode::Lenient).is_err() {
            return;
        }
        if own_echo && !sb.pending.is_empty() {
            sb.pending.remove(0);
        }
        sb.rebuild_view();
        let head_doc = Arc::new(sb.view.document().clone());
        sb.synced = head_doc.clone();
        if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
            && let Some(b) = m.branches.iter_mut().find(|b| b.id == id)
        {
            b.head = head_doc.clone();
        }
        self.update_open_branch_tab(session_id, main_uid, id, head_doc);
    }

    fn apply_remote_branch_update(
        &mut self,
        session_id: SessionId,
        main_uid: u64,
        id: &str,
        own_echo: bool,
        base_changes: &[Change],
        head_changes: &[Change],
    ) {
        let Some(live) = self.live.sessions.get_mut(&session_id) else { return };
        let Some(sb) = live.branch_replicas.get_mut(id) else { return };
        if sb.base.apply(base_changes, Mode::Lenient).is_err() || sb.confirmed.apply(head_changes, Mode::Lenient).is_err() {
            return;
        }
        if own_echo && !sb.pending.is_empty() {
            sb.pending.remove(0);
        }
        sb.rebuild_view();
        let base_doc = Arc::new(sb.base.document().clone());
        let head_doc = Arc::new(sb.view.document().clone());
        sb.synced = head_doc.clone();
        if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
            && let Some(b) = m.branches.iter_mut().find(|b| b.id == id)
        {
            b.base = base_doc;
            b.head = head_doc.clone();
        }
        self.update_open_branch_tab(session_id, main_uid, id, head_doc);
    }

    fn update_open_branch_tab(&mut self, session_id: SessionId, main_uid: u64, id: &str, head_doc: Arc<Document>) {
        if let Some(ti) = branches::tab_of(self, main_uid, id)
            && let Some(tab) = self.docs.get(ti)
            && !busy(tab)
        {
            let tab_uid = tab.uid;
            let client_id = tab.doc.client_id();
            let mut doc = (*head_doc).clone();
            doc.set_client_id(client_id);
            let next_id = tab.doc.next_id();
            if next_id.0 > doc.next_id().0 {
                doc.reserve_ids(figcraft_doc::NodeId(next_id.0 - 1));
            }
            let doc = Arc::new(doc);
            if let Some(live) = self.live.sessions.get_mut(&session_id)
                && let Some(sb) = live.branch_replicas.get_mut(id)
            {
                sb.synced = doc.clone();
            }
            if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
                && let Some(b) = m.branches.iter_mut().find(|b| b.id == id)
            {
                b.head = doc.clone();
            }
            self.apply_remote(tab_uid, doc);
        }
    }
}
