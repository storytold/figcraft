//! Shared branches in a live session: sync branch creation, edits, updates from main, and
//! merge/archive state changes across everyone connected to the same server file.

use std::sync::Arc;

use figcraft_collab::message::{BranchEvent, BranchReq, BranchStatus, ClientMsg, SharedBranch};
use figcraft_collab::{Access, Mode, Replica, diff};
use figcraft_doc::Document;
use figcraft_format::{Branch, BranchState};

use super::types::SessionId;
use super::{Session, busy};
use crate::branches;

/// Synced state of one shared branch on this client.
pub(super) struct SyncedBranch {
    pub(super) base: Replica,
    pub(super) head: Replica,
    pub(super) synced: Arc<Document>,
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
        if let Some(live) = self.live.sessions.get_mut(&session_id) {
            live.branch_replicas.insert(branch.id.clone(), SyncedBranch { base: base_rep, head: head_rep, synced });
        }
        let req = BranchReq::Create {
            id: branch.id.clone(),
            name: branch.name.clone(),
            time: branch.time,
            client: branch.client,
            base_seq,
            base_diff: base_diff.changes,
            head_diff: head_diff.changes,
        };
        self.live.send(session_id, [ClientMsg::Branch(Box::new(req))]);
    }

    /// Push local edits of shared branch `branch_id` (of main document `main_uid`) to the server.
    pub(crate) fn live_branch_edit(&mut self, main_uid: u64, branch_id: &str) {
        let Some(tab_idx) = branches::tab_of(self, main_uid, branch_id) else { return };
        let Some(tab) = self.docs.get(tab_idx) else { return };
        if busy(tab) {
            return;
        }
        let doc = tab.document_itself().clone();
        let Some(live) = self.live.doc_mut(main_uid) else { return };
        if !live.shared_branches || !live.client.as_ref().is_some_and(|c| c.access() == Access::Edit) {
            return;
        }
        let session_id = live.id;
        let Some(sb) = live.branch_replicas.get_mut(branch_id) else { return };
        if Arc::ptr_eq(&sb.synced, &doc) {
            return;
        }
        let Ok(d) = diff(&sb.head, &doc) else { return };
        sb.head = d.replica;
        sb.synced = doc.clone();
        if !d.changes.is_empty() {
            if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
                && let Some(b) = m.branches.iter_mut().find(|b| b.id == branch_id)
            {
                b.head = doc;
            }
            self.live.send(session_id, [ClientMsg::Branch(Box::new(BranchReq::Edit { id: branch_id.to_string(), changes: d.changes }))]);
        }
    }

    /// Sync edits on all open shared branch tabs belonging to live document `main_uid`.
    pub(super) fn live_branches_sync(&mut self, main_uid: u64) {
        let can_sync = self.live.doc(main_uid).is_some_and(|l| l.shared_branches && l.client.as_ref().is_some_and(|c| c.access() == Access::Edit));
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
        let Ok(hd) = diff(&sb.head, new_head) else { return };
        sb.base = bd.replica;
        sb.head = hd.replica;
        sb.synced = new_head.clone();
        let req = BranchReq::UpdateFromMain { id: branch_id.to_string(), base_changes: bd.changes, head_changes: hd.changes };
        self.live.send(session_id, [ClientMsg::Branch(Box::new(req))]);
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

    /// Initial list of shared branches sent by the server right after `Welcome`.
    pub(super) fn live_branches_welcome(&mut self, session_id: SessionId, list: Vec<SharedBranch>) {
        let Some((main_uid, confirmed)) = self.live.sessions.get_mut(&session_id).and_then(|l| {
            l.shared_branches = true;
            let uid = l.doc_uid?;
            let c = l.client.as_ref()?.confirmed().clone();
            Some((uid, c))
        }) else {
            return;
        };
        for wb in list {
            self.install_shared_branch(session_id, main_uid, &confirmed, wb, false);
        }
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
                if author == me {
                    return;
                }
                self.apply_remote_branch_edit(session_id, main_uid, &id, &changes);
            }
            BranchEvent::UpdatedFromMain { id, author, base_changes, head_changes, .. } => {
                if author == me {
                    return;
                }
                self.apply_remote_branch_update(session_id, main_uid, &id, &base_changes, &head_changes);
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
                if author != me
                    && let Some(name) = name
                {
                    let who = self.peer_name(session_id, author);
                    let verb = if state == BranchState::Merged { "merged" } else { "archived" };
                    self.message = Some(format!("{who} {verb} branch “{name}”"));
                }
            }
        }
    }

    fn peer_name(&self, session_id: SessionId, peer_id: u32) -> String {
        self.live.sessions.get(&session_id).and_then(|l| l.peers.get(&peer_id)).map_or_else(|| "Someone".to_string(), |p| p.name.clone())
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
        let head_doc = Arc::new(head_rep.document().clone());
        let id = wb.id.clone();
        if own_echo {
            // Only refresh base if main moved concurrently when we created the branch.
            if let Some(live) = self.live.sessions.get_mut(&session_id)
                && let Some(sb) = live.branch_replicas.get_mut(&id)
            {
                sb.base = base_rep;
            }
            if let Some(m) = self.docs.iter_mut().find(|d| d.uid == main_uid)
                && let Some(b) = m.branches.iter_mut().find(|b| b.id == id)
            {
                b.base = base_doc;
            }
            return;
        }
        if let Some(live) = self.live.sessions.get_mut(&session_id) {
            live.branch_replicas.insert(id.clone(), SyncedBranch { base: base_rep, head: head_rep, synced: head_doc.clone() });
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

    fn apply_remote_branch_edit(&mut self, session_id: SessionId, main_uid: u64, id: &str, changes: &[figcraft_collab::Change]) {
        let Some(live) = self.live.sessions.get_mut(&session_id) else { return };
        let Some(sb) = live.branch_replicas.get_mut(id) else { return };
        if sb.head.apply(changes, Mode::Lenient).is_err() {
            return;
        }
        let head_doc = Arc::new(sb.head.document().clone());
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
        base_changes: &[figcraft_collab::Change],
        head_changes: &[figcraft_collab::Change],
    ) {
        let Some(live) = self.live.sessions.get_mut(&session_id) else { return };
        let Some(sb) = live.branch_replicas.get_mut(id) else { return };
        if sb.base.apply(base_changes, Mode::Lenient).is_err() || sb.head.apply(head_changes, Mode::Lenient).is_err() {
            return;
        }
        let base_doc = Arc::new(sb.base.document().clone());
        let head_doc = Arc::new(sb.head.document().clone());
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
