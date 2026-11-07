//! Per-user undo and redo in a live session. Every undoable batch of this client becomes a
//! [`Step`]: the values it wrote and the values they replaced, by [`Target`], plus the nodes it
//! created or deleted. Undoing a step writes the replaced values back, but only where nobody else
//! wrote after the step (the server ordered someone else's batch after it); redo goes the other
//! way. Batches from others never become steps.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::NodeId;

use crate::Error;
use crate::change::{Change, DocProp, Entry, Family, Json, Prop, Target};
use crate::replica::{Mode, Replica, create_change};

/// Index of the state before a step in its two-sided records.
pub(crate) const BACK: usize = 0;
/// Index of the state after a step.
pub(crate) const FWD: usize = 1;

/// What a batch of this client is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchKind {
    /// An edit (one command or gesture).
    Edit,
    /// Undoing an edit.
    Undo,
    /// Redoing an undone edit.
    Redo,
    /// Derived data brought up to date after other batches (never in the undo history).
    FollowUp,
}

/// Something to tell the user (a short toast).
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    /// The server refused one of this client's batches: what it changed is rolled back and it
    /// leaves the undo history.
    Rejected { batch_id: u64, kind: BatchKind, reason: String },
    /// The session restarted while this client was away: its unconfirmed batches are gone.
    Lost { batches: usize },
}

impl Notice {
    /// The text to show.
    pub fn message(&self) -> String {
        match self {
            Notice::Rejected { kind: BatchKind::Undo, reason, .. } => format!("Couldn't undo: {reason}"),
            Notice::Rejected { kind: BatchKind::Redo, reason, .. } => format!("Couldn't redo: {reason}"),
            Notice::Rejected { reason, .. } => format!("Your change couldn't be applied: {reason}"),
            Notice::Lost { batches } => {
                format!("The live session restarted: {batches} unsent change{} couldn't be kept", if *batches == 1 { "" } else { "s" })
            }
        }
    }
}

/// The two sides of a value: before and after the step.
type Sides<T> = [T; 2];
/// What a batch wrote, by target.
type Writes = BTreeMap<Target, Sides<Option<Json>>>;
/// Nodes that exist on one side only: the change creating them as they are there.
type Nodes = BTreeMap<NodeId, Sides<Option<Change>>>;

/// One entry of the undo history.
#[derive(Clone, Debug)]
pub(crate) struct Step {
    /// Unique within the client (batches name their step by it).
    pub id: u64,
    pub label: String,
    /// The batch that last applied this step: the edit, or its latest undo or redo (which may
    /// still wait to be sent).
    pub batch: u64,
    /// The server confirmed the step's last batch that was sent, and nothing of it is in flight:
    /// batches from others ordered after it conflict with it.
    pub confirmed: bool,
    /// That confirmed batch went towards the after side (the edit or a redo; false: an undo).
    pub settled_forward: bool,
    /// While an undo or redo of the step is in flight: what it writes, and what others wrote
    /// meanwhile (ordered before it, unless the connection drops and it is made again).
    in_flight: Option<(BTreeSet<Target>, Touches)>,
    writes: Writes,
    nodes: Nodes,
    /// Targets others wrote after this step: it never writes them again.
    taken: BTreeSet<Target>,
    /// Nodes others created again after this step (by their undo): it never deletes them.
    taken_nodes: BTreeSet<NodeId>,
}

/// What `changes` (a batch that turned `pre` into `post`) wrote, oriented `[pre, post]`; with
/// `same`, also the targets it wrote that kept their value.
fn capture(pre: &Replica, post: &Replica, changes: &[Change], same: bool) -> Result<(Writes, Nodes), Error> {
    let (a, b) = (pre.document(), post.document());
    let mut writes = Writes::new();
    let mut nodes = Nodes::new();
    for c in changes {
        match c {
            Change::Create { id, .. } => {
                if let Some(n) = b.node(*id)
                    && !a.contains(*id)
                {
                    nodes.insert(*id, [None, Some(create_change(n, post.key(*id))?)]);
                }
            }
            Change::Delete { id } => {
                for d in a.subtree(*id) {
                    if let Some(n) = a.node(d)
                        && !b.contains(d)
                    {
                        nodes.insert(d, [Some(create_change(n, pre.key(d))?), None]);
                    }
                }
            }
            other => {
                let Some(t) = Target::of(other) else { continue };
                if writes.contains_key(&t) {
                    continue;
                }
                let (x, y) = (pre.value(&t)?, post.value(&t)?);
                // A node created or deleted by the batch is recorded whole above.
                let missing = matches!(t, Target::Node(..)) && (x.is_none() || y.is_none());
                if !missing && (same || x != y) {
                    writes.insert(t, [x, y]);
                }
            }
        }
    }
    Ok((writes, nodes))
}

impl Step {
    /// The step for batch `batch` that turned `pre` into `post` with `changes`.
    pub fn new(id: u64, label: &str, batch: u64, pre: &Replica, post: &Replica, changes: &[Change]) -> Result<Step, Error> {
        let (writes, nodes) = capture(pre, post, changes, false)?;
        Ok(Step {
            id,
            label: label.to_string(),
            batch,
            confirmed: false,
            settled_forward: true,
            writes,
            nodes,
            taken: BTreeSet::new(),
            taken_nodes: BTreeSet::new(),
            in_flight: None,
        })
    }

    /// A batch of the step (its edit, an undo or a redo: `kind`) was applied again, now on top of
    /// `pre` (giving `post`): what it replaced is now taken from `pre` (others' batches were
    /// ordered before it, or an earlier batch of this client was refused) and what it wrote from
    /// `post`. The edit's nodes become those it now creates and deletes (a layer someone put
    /// inside a layer it deletes comes back with an undo); an undo or redo only updates the nodes
    /// of the step (what it takes with it beyond them was someone else's).
    pub fn refresh(&mut self, pre: &Replica, post: &Replica, changes: &[Change], kind: BatchKind) -> Result<(), Error> {
        let (writes, nodes) = capture(pre, post, changes, true)?;
        let (p, q) = if kind == BatchKind::Undo { (FWD, BACK) } else { (BACK, FWD) };
        for (t, [x, y]) in writes {
            if !self.taken.contains(&t)
                && let Some(s) = self.writes.get_mut(&t)
            {
                s[p] = x;
                s[q] = y;
            }
        }
        let oriented = nodes.into_iter().map(|(id, [x, y])| {
            let mut s = [None, None];
            s[p] = x;
            s[q] = y;
            (id, s)
        });
        if kind == BatchKind::Edit {
            self.nodes = oriented.collect();
        } else {
            for (id, s) in oriented {
                if let Some(e) = self.nodes.get_mut(&id) {
                    *e = s;
                }
            }
        }
        Ok(())
    }

    /// An undo or redo of the step (`changes`) goes out: until the server confirms it, others'
    /// batches are ordered before it.
    pub fn send(&mut self, changes: &[Change]) {
        self.confirmed = false;
        self.in_flight = Some((changes.iter().filter_map(Target::of).collect(), Touches::default()));
    }

    /// The server confirmed the step's batch of `kind`.
    pub fn settle(&mut self, kind: BatchKind) {
        self.confirmed = true;
        self.in_flight = None;
        self.settled_forward = kind != BatchKind::Undo;
    }

    /// Its undo or redo in flight was lost with the connection: it will be made again, so what
    /// others wrote meanwhile came after the step's last confirmed batch.
    pub fn lost(&mut self) {
        self.confirmed = true;
        if let Some((_, seen)) = self.in_flight.take() {
            self.taint(&seen, &BTreeSet::new());
        }
    }

    /// Someone else's batch (`touches`) was ordered after the step's last batch, or before its
    /// batch in flight: the targets it writes are theirs from now on, except those the batch in
    /// flight writes again after them.
    pub fn others_wrote(&mut self, touches: &Touches) {
        if self.confirmed {
            self.taint(touches, &BTreeSet::new());
        } else if let Some((mine, seen)) = self.in_flight.take() {
            self.taint(touches, &mine);
            let mut seen = seen;
            seen.absorb(touches);
            self.in_flight = Some((mine, seen));
        }
    }

    /// Leave the targets `touches` writes (but those in `except`) alone from now on.
    fn taint(&mut self, touches: &Touches, except: &BTreeSet<Target>) {
        for t in self.writes.keys() {
            if touches.hits(t) && !except.contains(t) {
                self.taken.insert(t.clone());
            }
        }
        for id in self.nodes.keys() {
            if touches.nodes.contains(id) {
                self.taken_nodes.insert(*id);
            }
        }
    }

    /// The batches since this step are unknown (a reconnect brought a snapshot): every target
    /// whose value in `state` isn't the one the step's confirmed batch wrote was changed by others.
    pub fn verify(&mut self, state: &Replica) -> Result<(), Error> {
        let side = if self.settled_forward { FWD } else { BACK };
        for (t, v) in &self.writes {
            if !self.taken.contains(t) && state.value(t)? != v[side] {
                self.taken.insert(t.clone());
            }
        }
        Ok(())
    }

    /// The batch that takes `view` to this step's `side` (BACK: undo, FWD: redo): nodes of that
    /// side created again (parents first), the values of that side written back, then nodes of
    /// the other side deleted. Leaves out targets others wrote since, nodes that no longer fit
    /// (a parent gone) and created nodes others have put layers into. Every change of the result
    /// applies (validated in order).
    pub fn toward(&self, side: usize, view: &Replica) -> Result<Vec<Change>, Error> {
        let other = 1 - side;
        let mut work = view.clone();
        let mut out = Vec::new();
        let doc = view.document();
        // Nodes to bring back, shallowest first (a parent before its children).
        let back: BTreeMap<NodeId, &Change> = self
            .nodes
            .iter()
            .filter(|(id, s)| s[other].is_none() && !self.taken_nodes.contains(*id) && !doc.contains(**id))
            .filter_map(|(id, s)| s[side].as_ref().map(|c| (*id, c)))
            .collect();
        let depth = |mut id: NodeId| {
            let mut d = 0usize;
            while let Some(Change::Create { parent: Some(p), .. }) = back.get(&id).copied() {
                d += 1;
                id = *p;
                if d > back.len() {
                    break;
                }
            }
            d
        };
        let mut todo: Vec<(usize, Change)> = back.iter().map(|(id, c)| (depth(*id), (*c).clone())).collect();
        todo.sort_by_key(|(d, _)| *d);
        let mut todo: Vec<Change> = todo.into_iter().map(|(_, c)| c).collect();
        for (t, v) in &self.writes {
            let moved_away = matches!(t, Target::Node(id, Prop::Position) if self.taken.contains(&Target::Node(*id, Prop::Parent)));
            if self.taken.contains(t) || moved_away {
                continue;
            }
            let now = work.value(t)?;
            if matches!(t, Target::Node(..)) && now.is_none() {
                continue;
            }
            let want = v[side].as_ref();
            if now.as_ref() == want {
                continue;
            }
            todo.extend(t.write(want));
        }
        apply_all(&mut work, &mut out, todo);
        // Nodes to remove: those that exist only on the other side, when nothing else is inside.
        let gone: BTreeSet<NodeId> = self
            .nodes
            .iter()
            .filter(|(id, s)| s[side].is_none() && s[other].is_some() && !self.taken_nodes.contains(*id))
            .map(|(id, _)| *id)
            .collect();
        let wdoc = work.document();
        let mut stack: Vec<NodeId> =
            gone.iter().copied().filter(|id| wdoc.node(*id).is_some_and(|n| n.parent.is_none_or(|p| !gone.contains(&p)))).collect();
        let mut deletes = Vec::new();
        while let Some(id) = stack.pop() {
            if wdoc.subtree(id).iter().all(|d| gone.contains(d)) {
                deletes.push(Change::Delete { id });
            } else if let Some(n) = wdoc.node(id) {
                stack.extend(n.children.iter().copied().filter(|c| gone.contains(c)));
            }
        }
        apply_all(&mut work, &mut out, deletes);
        // Changes that apply one by one can still fail the batch's checks together (rare): keep
        // only those that pass them in order.
        view.clone().apply_kept(&mut out, |_| true)?;
        Ok(out)
    }
}

/// Apply `todo` to `work` change by change, keeping in `out` those that apply; changes that don't
/// are retried while others still go in (a move back can depend on another move back).
fn apply_all(work: &mut Replica, out: &mut Vec<Change>, mut todo: Vec<Change>) {
    loop {
        let before = todo.len();
        let mut left = Vec::new();
        for c in todo {
            match work.apply(std::slice::from_ref(&c), Mode::Lenient) {
                Ok(a) if a.skipped == 0 => out.push(c),
                _ => left.push(c),
            }
        }
        if left.is_empty() || left.len() == before {
            return;
        }
        todo = left;
    }
}

/// What a batch from someone else wrote, for marking the steps it conflicts with.
#[derive(Clone, Debug, Default)]
pub(crate) struct Touches {
    props: BTreeSet<(NodeId, Prop)>,
    /// Nodes created (or created again): all their properties are written.
    nodes: BTreeSet<NodeId>,
    docs: BTreeSet<DocProp>,
    entries: BTreeSet<Entry>,
    /// Lists written whole.
    whole: BTreeSet<Family>,
    /// Lists written in any way (whole, order or an entry).
    lists: BTreeSet<Family>,
}

impl Touches {
    /// Add what `other` writes.
    pub fn absorb(&mut self, other: &Touches) {
        self.props.extend(other.props.iter().copied());
        self.nodes.extend(other.nodes.iter().copied());
        self.docs.extend(other.docs.iter().copied());
        self.entries.extend(other.entries.iter().cloned());
        self.whole.extend(other.whole.iter().copied());
        self.lists.extend(other.lists.iter().copied());
    }

    pub fn of(changes: &[Change]) -> Touches {
        let mut t = Touches::default();
        for c in changes {
            match c {
                Change::Create { id, .. } => {
                    t.nodes.insert(*id);
                }
                Change::Set { id, prop, .. } => {
                    t.props.insert((*id, *prop));
                }
                Change::Delete { .. } => {}
                Change::SetDoc { prop, .. } => {
                    t.docs.insert(*prop);
                    match prop.list() {
                        Some(Ok(l)) => {
                            t.whole.insert(l);
                            t.lists.insert(l);
                        }
                        Some(Err(l)) => {
                            t.lists.insert(l);
                        }
                        None => {}
                    }
                }
                Change::Put { entry, .. } | Change::Remove { entry } => {
                    t.entries.insert(entry.clone());
                    if let Some(l) = entry.list() {
                        t.lists.insert(l);
                    }
                }
            }
        }
        t
    }

    /// Whether these writes change `target`.
    pub fn hits(&self, target: &Target) -> bool {
        match target {
            Target::Node(id, p) => self.props.contains(&(*id, *p)) || self.nodes.contains(id),
            Target::Doc(p) => {
                self.docs.contains(p)
                    || match p.list() {
                        Some(Ok(l)) => self.lists.contains(&l),
                        Some(Err(l)) => self.whole.contains(&l),
                        None => false,
                    }
            }
            Target::Entry(e) => self.entries.contains(e) || e.list().is_some_and(|l| self.whole.contains(&l)),
        }
    }

    /// Whether these writes create node `id`.
    pub fn creates(&self, id: NodeId) -> bool {
        self.nodes.contains(&id)
    }
}
