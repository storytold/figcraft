//! Three-way merge, for branches of a file: what a branch and its main file each changed since
//! the branch was made (two [`diff`]s from their common base), where those changes conflict, and
//! one side's changes merged into the other side.
//!
//! A conflict is the same property (or document-level entry) changed on both sides to different
//! values, or a layer deleted on one side while the other side edited it or something inside it
//! (including adding layers to it or moving layers into or out of it). The layer lists whose items
//! stand on their own (fills, strokes, effects, layout grids, export settings, guides, prototype
//! interactions, accessibility annotations, Dev Mode resources and measurements) merge item by
//! item: both sides changing different items of the list (or one adding items at its end) is no
//! conflict, only the same item changed both ways or the list shortened on a side while the other
//! changed it. Auto layout settings (a frame's and a child's) merge field by field: each field
//! changed both ways is a conflict of its own, and the other fields merge. Values a file works out
//! for itself (a size or place auto layout gives a layer, text sizing to its characters, a value a
//! layer in an instance takes from its main component) never conflict: the merged file works them
//! out again from what they follow. Nothing that conflicts is merged until it is resolved: each
//! conflict is resolved for one side, and the other side's changes to it are left out. Everything
//! else from both sides is kept.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use figcraft_doc::{Document, Node, NodeId, NodeKind, Sizing, TextAutoResize};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::change::{Change, DocProp, Entry, Json, Prop};
use crate::replica::{Mode, Positions, Replica, create_change};
use crate::{Error, diff};

/// One side of a merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Branch,
    Main,
}

impl Side {
    /// The side that isn't `self`.
    pub fn other(self) -> Side {
        match self {
            Side::Branch => Side::Main,
            Side::Main => Side::Branch,
        }
    }

    /// `"branch"` or `"main"`.
    pub fn name(self) -> &'static str {
        match self {
            Side::Branch => "branch",
            Side::Main => "main",
        }
    }

    /// The side `"branch"` or `"main"` names (`None` for anything else).
    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "branch" => Some(Side::Branch),
            "main" => Some(Side::Main),
            _ => None,
        }
    }
}

/// What a conflict is about.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A layer property. A layer's parent and position are one property (where it is).
    Node(NodeId, Prop),
    /// One field of a layer property that merges field by field ([`PER_FIELD`]): `padding` of
    /// `layout`, `sizing_h` of `childLayout`…
    Field(NodeId, Prop, String),
    /// A layer deleted on one side and edited (it or what's inside it) on the other.
    Layer(NodeId),
    /// A document-level property.
    Doc(DocProp),
    /// A document-level entry (a style, variable, comment thread…).
    Entry(Entry),
}

/// The serde name of `v` (`fills`, `styleOrder`…).
fn camel<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// `12/fills`, `12/layout.padding`, `12/layer`, `doc/styleOrder`, `style/S:3`, `comment/7`… (the
/// keys resolutions use).
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Node(id, prop) => write!(f, "{id}/{}", camel(prop)),
            Key::Field(id, prop, field) => write!(f, "{id}/{}.{field}", camel(prop)),
            Key::Layer(id) => write!(f, "{id}/layer"),
            Key::Doc(prop) => write!(f, "doc/{}", camel(prop)),
            Key::Entry(e) => match e {
                Entry::Style(k) => write!(f, "style/{k}"),
                Entry::Comment(k) => write!(f, "comment/{k}"),
                Entry::Collection(k) => write!(f, "collection/{k}"),
                Entry::Variable(k) => write!(f, "variable/{k}"),
                Entry::DevSnapshot(k) => write!(f, "devSnapshot/{k}"),
                Entry::Image(k) => write!(f, "image/{k}"),
                Entry::Ballot(k) => write!(f, "ballot/{k}"),
            },
        }
    }
}

/// Changes that conflict, and which side's to keep.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub key: Key,
    /// Layer conflicts: the side that deleted the layer.
    pub deleted_by: Option<Side>,
    /// The branch's changes to it (indexes into [`Plan::branch`]).
    pub branch: Vec<usize>,
    /// The main file's changes to it (indexes into [`Plan::main`]).
    pub main: Vec<usize>,
}

impl Conflict {
    /// `side`'s changes to it.
    pub fn changes(&self, side: Side) -> &[usize] {
        match side {
            Side::Branch => &self.branch,
            Side::Main => &self.main,
        }
    }
}

/// What each side changed since the base, and where they conflict.
#[derive(Clone, Debug)]
pub struct Plan {
    base: Replica,
    /// The branch's changes since the base.
    pub branch: Vec<Change>,
    /// The main file's changes since the base.
    pub main: Vec<Change>,
    pub conflicts: Vec<Conflict>,
    /// Layer properties changed both ways to values both files work out for themselves
    /// ([`computed`]): no conflict, and a merge leaves the file merged into its own value.
    derived: BTreeSet<Key>,
    /// Layer properties ([`Key::Node`]) merged field by field with fields in conflict.
    fields: BTreeMap<Key, Fields>,
    branch_positions: Positions,
    main_positions: Positions,
}

/// A layer property merged field by field ([`merge_fields`]): the value with every field changed
/// one way (or the same way on both sides) merged, and each side's values of the fields changed
/// both ways (`Null` for a field a side left out).
#[derive(Clone, Debug, Default, PartialEq)]
struct Fields {
    merged: Map<String, Value>,
    branch: Map<String, Value>,
    main: Map<String, Value>,
}

/// Document-level properties that belong to each copy, not to the work: the file name (a branch
/// is named after itself) and the page it opens on.
fn merges(c: &Change) -> bool {
    !matches!(c, Change::SetDoc { prop: DocProp::Name | DocProp::CurrentPage, .. })
}

/// What a change writes, for finding the same write on both sides (creations: nothing; ids are
/// never made twice, see [`Document::set_client_id`]).
fn key_of(c: &Change) -> Option<Key> {
    Some(match c {
        Change::Set { id, prop: Prop::Position, .. } => Key::Node(*id, Prop::Parent),
        Change::Set { id, prop, .. } => Key::Node(*id, *prop),
        Change::Delete { id } => Key::Layer(*id),
        Change::SetDoc { prop, .. } => Key::Doc(*prop),
        Change::Put { entry, .. } | Change::Remove { entry } => Key::Entry(entry.clone()),
        Change::Create { .. } => return None,
    })
}

/// The changes (indexes) in `changes` grouped by what they write ([`key_of`]).
fn by_key(changes: &[Change]) -> BTreeMap<Key, Vec<usize>> {
    let mut out: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
    for (i, c) in changes.iter().enumerate() {
        if let Some(k) = key_of(c) {
            out.entry(k).or_default().push(i);
        }
    }
    out
}

/// List properties whose items merge one by one: the items stand on their own, so both sides
/// changing different ones (or one side adding at its end) is no conflict. Sets and keyed maps
/// (`bound`, `varModes`, `overrides`, `propRefs`, `pluginData`) are not here: their items line up
/// by key, not by position.
const PER_ITEM: &[Prop] = &[
    Prop::Fills,
    Prop::Strokes,
    Prop::Effects,
    Prop::Grids,
    Prop::Exports,
    Prop::Guides,
    Prop::Interactions,
    Prop::Annotations,
    // Not `CodeLinks`: there is one per language, and lining them up by position could leave two
    // for one language (the second hidden), so they merge whole.
    Prop::DevResources,
    Prop::Measurements,
];

/// Both sides' writes `b` and `m` of list property `key` merged item by item against the base
/// (see [`merge_items`]); `None` when `key` isn't such a list or the writes conflict.
fn merged_list(base: &Document, key: &Key, b: Option<&Change>, m: Option<&Change>) -> Option<Json> {
    let Key::Node(id, prop) = key else { return None };
    let (Some(Change::Set { value: bv, .. }), Some(Change::Set { value: mv, .. })) = (b, m) else { return None };
    if !PER_ITEM.contains(prop) {
        return None;
    }
    let o = prop.get(base.node(*id)?).ok()?;
    merge_items(o.0.as_array()?, bv.0.as_array()?, mv.0.as_array()?).map(|v| Json(Value::Array(v)))
}

/// Layer properties whose fields merge one by one: auto layout settings, a frame's (`layout`) and
/// a child's (`childLayout`). Changing the padding on one side and the gap on the other is no
/// conflict, and each field changed both ways is a conflict of its own ([`Key::Field`]).
const PER_FIELD: &[Prop] = &[Prop::Layout, Prop::ChildLayout];

/// Three-way merge of object `o` changed to `b` on one side and `m` on the other, field by field:
/// a field changed on one side takes that side's value, and one changed both ways (differently)
/// is in conflict. `None` when any of them isn't an object (auto layout added or removed).
fn merge_fields(o: &Value, b: &Value, m: &Value) -> Option<Fields> {
    let (o, b, m) = (o.as_object()?, b.as_object()?, m.as_object()?);
    let names: BTreeSet<&String> = o.keys().chain(b.keys()).chain(m.keys()).collect();
    let mut out = Fields::default();
    for name in names {
        let (base, x, y) = (o.get(name), b.get(name), m.get(name));
        let kept = if x == base {
            y
        } else if y == base || x == y {
            x
        } else {
            out.branch.insert(name.clone(), x.cloned().unwrap_or(Value::Null));
            out.main.insert(name.clone(), y.cloned().unwrap_or(Value::Null));
            base
        };
        if let Some(v) = kept {
            out.merged.insert(name.clone(), v.clone());
        }
    }
    Some(out)
}

/// Both sides' writes `b` and `m` of property `key` (one of [`PER_FIELD`]) merged field by field
/// against the base; `None` when `key` isn't such a property or a value isn't an object.
fn merged_fields(base: &Document, key: &Key, b: Option<&Change>, m: Option<&Change>) -> Option<Fields> {
    let Key::Node(id, prop) = key else { return None };
    let (Some(Change::Set { value: bv, .. }), Some(Change::Set { value: mv, .. })) = (b, m) else { return None };
    if !PER_FIELD.contains(prop) {
        return None;
    }
    let o = prop.get(base.node(*id)?).ok()?;
    merge_fields(&o.0, &bv.0, &mv.0)
}

/// Whether layer `n` of `doc` is placed by its parent's auto layout (not absolute).
fn in_flow(doc: &Document, n: &Node) -> bool {
    !n.child_layout.absolute && n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some())
}

/// Whether auto layout or text works out layer `n`'s width (`horizontal`) or height: an auto layout
/// frame hugging its contents, a child filling its auto layout parent, or text sizing to its
/// characters.
fn sized_for_itself(doc: &Document, n: &Node, horizontal: bool) -> bool {
    let sizing = if horizontal { n.child_layout.sizing_h } else { n.child_layout.sizing_v };
    let by_layout = match sizing {
        Sizing::Hug => n.layout.is_some(),
        Sizing::Fill => in_flow(doc, n),
        Sizing::Fixed => false,
    };
    let by_text = match &n.kind {
        NodeKind::Text(t) => t.auto_resize == TextAutoResize::WidthAndHeight || (!horizontal && t.auto_resize == TextAutoResize::Height),
        _ => false,
    };
    by_layout || by_text
}

/// Whether layer `n` of `doc` (an instance, or a layer inside one) has property `prop` as its main
/// component's layer has it: it follows the main rather than overriding it.
fn follows_main(doc: &Document, n: &Node, prop: Prop) -> bool {
    let main = match &n.kind {
        NodeKind::Instance { component, .. } => Some(*component),
        _ => n.source,
    };
    Prop::FIELDS.contains(&prop) && main.and_then(|m| doc.node(m)).is_some_and(|m| !prop.differs(n, m))
}

/// Whether layer property `key` holds a value `doc` works out for itself rather than one set on
/// the layer: a size auto layout or text gives it ([`sized_for_itself`]), its place in an auto
/// layout parent, or a value it takes from its main component ([`follows_main`]).
fn computed(doc: &Document, key: &Key) -> bool {
    let Key::Node(id, prop) = key else { return false };
    let Some(n) = doc.node(*id) else { return false };
    let by_layout = match prop {
        Prop::Width => sized_for_itself(doc, n, true),
        Prop::Height => sized_for_itself(doc, n, false),
        Prop::Transform => in_flow(doc, n),
        _ => false,
    };
    by_layout || follows_main(doc, n, *prop)
}

/// Whether property `key`, changed both ways, is one both files work out for themselves
/// ([`computed`]), so neither side's value is a choice to make. A place given by auto layout is
/// only that when both sides also turn and scale the layer the same way (a rotation is set on the
/// layer, not worked out).
fn derived_on_both(branch: &Document, main: &Document, key: &Key) -> bool {
    if !(computed(branch, key) && computed(main, key)) {
        return false;
    }
    let Key::Node(id, Prop::Transform) = key else { return true };
    let linear = |doc: &Document| doc.node(*id).map(|n| n.transform.as_coeffs()).map(|c| [c[0], c[1], c[2], c[3]]);
    linear(branch) == linear(main)
}

/// The change at the only index in `c`.
fn only<'a>(c: &[usize], all: &'a [Change]) -> Option<&'a Change> {
    match c {
        [i] => all.get(*i),
        _ => None,
    }
}

/// Three-way merge of list `o` changed to `b` on one side and `m` on the other. Items at the same
/// index merge one by one (changed on one side: that side's; on both: only the same way), and
/// items added after `o`'s on one side (or the same ones on both) are kept. `None` when the same
/// item changed both ways, both added different items, or a side removed items (lists without ids
/// can't be lined up then).
pub(crate) fn merge_items(o: &[Value], b: &[Value], m: &[Value]) -> Option<Vec<Value>> {
    let (tb, tm) = (b.get(o.len()..)?, m.get(o.len()..)?);
    let mut out = Vec::with_capacity(b.len().max(m.len()));
    for (i, base) in o.iter().enumerate() {
        let (x, y) = (b.get(i)?, m.get(i)?);
        let item = if x == base {
            y
        } else if y == base || x == y {
            x
        } else {
            return None;
        };
        out.push(item.clone());
    }
    if !tb.is_empty() && !tm.is_empty() && tb != tm {
        return None;
    }
    out.extend_from_slice(if tb.is_empty() { tm } else { tb });
    Some(out)
}

/// The changes (indexes) that edit what's in `gone`: write one of its layers, create a layer in it
/// (or in a layer created in it) or move a layer into it. Changes come in [`diff`] order, so
/// creations come parents first and before moves.
fn touching(gone: &BTreeSet<NodeId>, changes: &[Change]) -> Vec<usize> {
    let mut doomed = gone.clone();
    let mut out = Vec::new();
    for (i, c) in changes.iter().enumerate() {
        let hit = match c {
            Change::Set { id, prop: Prop::Parent, value } => {
                doomed.contains(id) || value.read::<Option<NodeId>>("parent").ok().flatten().is_some_and(|p| doomed.contains(&p))
            }
            Change::Set { id, .. } => doomed.contains(id),
            Change::Create { id, parent, .. } => {
                let hit = parent.is_some_and(|p| doomed.contains(&p));
                if hit {
                    doomed.insert(*id);
                }
                hit
            }
            _ => false,
        };
        if hit {
            out.push(i);
        }
    }
    out
}

/// The changes on `branch` and on `main` since `base` (the main file when the branch was made or
/// last updated), and their conflicts.
pub fn plan(base: &Document, branch: &Document, main: &Document) -> Result<Plan, Error> {
    let base_r = Replica::new(base.clone());
    let b = diff(&base_r, branch)?;
    let m = diff(&base_r, main)?;
    let mut bc: Vec<Change> = b.changes.into_iter().filter(merges).collect();
    let mut mc: Vec<Change> = m.changes.into_iter().filter(merges).collect();
    let mut conflicts = Vec::new();
    let mut lists = Vec::new();
    let mut derived = BTreeSet::new();
    let mut fields = BTreeMap::new();
    let km = by_key(&mc);
    for (k, bi) in by_key(&bc) {
        // Deletions conflict with edits (below); images are content-addressed, so never differ.
        if matches!(k, Key::Layer(_) | Key::Entry(Entry::Image(_))) {
            continue;
        }
        // shortcut: a layer inside an instance that follows its main conflicts along with the main's
        // layer (either side is fine: the engine re-derives it after merging); leave such layers
        // out once the plan can tell followed values from overrides.
        let Some(mi) = km.get(&k) else { continue };
        let same = bi.len() == mi.len() && bi.iter().zip(mi).all(|(x, y)| bc.get(*x) == mc.get(*y));
        if same {
            continue;
        }
        if derived_on_both(branch, main, &k) {
            derived.insert(k);
            continue;
        }
        if let Some(value) = merged_list(base, &k, only(&bi, &bc), only(mi, &mc)) {
            lists.push((bi, mi.clone(), value));
            continue;
        }
        match (&k, merged_fields(base, &k, only(&bi, &bc), only(mi, &mc))) {
            (_, Some(f)) if f.branch.is_empty() => lists.push((bi, mi.clone(), Json(Value::Object(f.merged)))),
            (Key::Node(id, prop), Some(f)) => {
                for name in f.branch.keys() {
                    let key = Key::Field(*id, *prop, name.clone());
                    conflicts.push(Conflict { key, deleted_by: None, branch: bi.clone(), main: mi.clone() });
                }
                fields.insert(k, f);
            }
            _ => conflicts.push(Conflict { key: k, deleted_by: None, branch: bi, main: mi.clone() }),
        }
    }
    // Both sides' writes of a list merged item by item (or of auto layout settings merged field by
    // field) become the merged value, so merging either way writes it.
    for (bi, mi, value) in lists {
        for (changes, at) in [(&mut bc, bi), (&mut mc, mi)] {
            for i in at {
                if let Some(Change::Set { value: v, .. }) = changes.get_mut(i) {
                    *v = value.clone();
                }
            }
        }
    }
    for (side, changes, doc, other) in [(Side::Branch, &bc, branch, &mc), (Side::Main, &mc, main, &bc)] {
        for (i, c) in changes.iter().enumerate() {
            let Change::Delete { id } = c else { continue };
            // Layers moved out before the deletion survive it.
            let gone: BTreeSet<NodeId> = base.subtree(*id).into_iter().filter(|n| !doc.contains(*n)).collect();
            let edits = touching(&gone, other);
            if edits.is_empty() {
                continue;
            }
            let (branch, main) = if side == Side::Branch { (vec![i], edits) } else { (edits, vec![i]) };
            conflicts.push(Conflict { key: Key::Layer(*id), deleted_by: Some(side), branch, main });
        }
    }
    Ok(Plan {
        base: base_r,
        branch: bc,
        main: mc,
        conflicts,
        derived,
        fields,
        branch_positions: b.replica.positions().clone(),
        main_positions: m.replica.positions().clone(),
    })
}

impl Plan {
    /// The base, as a replica.
    pub fn base(&self) -> &Replica {
        &self.base
    }

    /// `side`'s changes.
    pub fn changes(&self, side: Side) -> &[Change] {
        match side {
            Side::Branch => &self.branch,
            Side::Main => &self.main,
        }
    }

    /// Layer properties both sides changed to values both files work out for themselves (a size or
    /// place auto layout gives, a value an instance takes from its main): not conflicts, and
    /// merging leaves the file merged into its own value for the merged file to work out again.
    pub fn derived(&self) -> &BTreeSet<Key> {
        &self.derived
    }

    /// The branch's and main's values of field conflict `key` ([`Key::Field`]); `None` for other
    /// keys.
    pub fn field_values(&self, key: &Key) -> Option<(&Value, &Value)> {
        let Key::Field(id, prop, name) = key else { return None };
        let f = self.fields.get(&Key::Node(*id, *prop))?;
        Some((f.branch.get(name)?, f.main.get(name)?))
    }

    /// The side `pick` chose for each field in conflict of every property merged field by field,
    /// written into the value merged from the other fields: the writes that replace both sides'
    /// own writes of those properties.
    fn field_writes(&self, pick: &dyn Fn(&Key) -> Option<Side>) -> Vec<Change> {
        let mut out = Vec::new();
        for (key, f) in &self.fields {
            let Key::Node(id, prop) = key else { continue };
            let mut value = f.merged.clone();
            for name in f.branch.keys() {
                let won = if pick(&Key::Field(*id, *prop, name.clone())) == Some(Side::Branch) { &f.branch } else { &f.main };
                match won.get(name) {
                    Some(Value::Null) | None => value.remove(name),
                    Some(v) => value.insert(name.clone(), v.clone()),
                };
            }
            out.push(Change::Set { id: *id, prop: *prop, value: Json(Value::Object(value)) });
        }
        out
    }

    /// `side`'s positions (fractional keys) of the layers, for applying its changes elsewhere.
    fn positions(&self, side: Side) -> &Positions {
        match side {
            Side::Branch => &self.branch_positions,
            Side::Main => &self.main_positions,
        }
    }

    /// The conflicts `pick` has no side for.
    pub fn unresolved(&self, pick: &dyn Fn(&Key) -> Option<Side>) -> Vec<&Conflict> {
        self.conflicts.iter().filter(|c| pick(&c.key).is_none()).collect()
    }

    /// Merge `from`'s changes into `into` (the other side's document, as the plan was made from),
    /// each conflict resolved by `pick`. Fails when a conflict has no side, or when the result is
    /// not a valid document. Returns the merged document and the layers it touched.
    pub fn merge(&self, into: &Document, from: Side, pick: &dyn Fn(&Key) -> Option<Side>) -> Result<(Document, BTreeSet<NodeId>), Error> {
        let unresolved: Vec<String> = self.unresolved(pick).iter().map(|c| c.key.to_string()).collect();
        if !unresolved.is_empty() {
            return Err(Error::Invalid(format!("unresolved conflicts: {}", unresolved.join(", "))));
        }
        let mut skip: BTreeSet<usize> = BTreeSet::new();
        let mut batch = Vec::new();
        for c in &self.conflicts {
            if matches!(c.key, Key::Field(..)) {
                // Written whole below, each field from the side picked for it.
                skip.extend(c.changes(from));
            } else if pick(&c.key) != Some(from) {
                skip.extend(c.changes(from));
            } else if let (Key::Layer(id), Some(by)) = (&c.key, c.deleted_by)
                && by != from
            {
                // `into` deleted the layer and `from`'s edits win: bring back what it deleted (as
                // it was in the base), then `from`'s edits apply to it.
                for n in self.base.document().subtree(*id) {
                    if let Some(node) = self.base.document().node(n).filter(|_| !into.contains(n)) {
                        batch.push(create_change(node, self.base.key(n))?);
                    }
                }
            }
        }
        // shortcut: images a side stopped using are kept (unused ones cost file size only); drop
        // them if files grow.
        let kept = |c: &Change| !matches!(c, Change::Remove { entry: Entry::Image(_) });
        // Values both files work out for themselves stay as `into` has them; normalising the merged
        // file works them out again from what changed.
        let own = |c: &Change| key_of(c).is_some_and(|k| self.derived.contains(&k));
        batch.extend(self.changes(from).iter().enumerate().filter(|(i, c)| !skip.contains(i) && kept(c) && !own(c)).map(|(_, c)| c.clone()));
        batch.extend(self.field_writes(pick));
        let mut r = Replica::from_parts(into.clone(), self.positions(from.other()).clone());
        let applied = r.apply(&batch, Mode::Strict)?;
        Ok((r.document().clone(), applied.touched))
    }

    /// `into` (the other side's document) with only `from`'s changes at `indexes` applied, those
    /// that apply (for showing what one side's changes to a conflict look like).
    pub fn with_changes(&self, into: &Document, from: Side, indexes: &[usize]) -> Document {
        let changes: Vec<Change> = indexes.iter().filter_map(|i| self.changes(from).get(*i).cloned()).collect();
        let mut r = Replica::from_parts(into.clone(), self.positions(from.other()).clone());
        let _ = r.apply(&changes, Mode::Lenient);
        r.document().clone()
    }
}
