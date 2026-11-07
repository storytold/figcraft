//! Ids a file being imported brings that the document already uses for something else.
//!
//! Style, variable and collection ids are numbered per file (`S:1`, `VariableID:1`,
//! `VariableCollectionId:1`; see [`figcraft_doc::ids`]), so two files made apart use the same ones
//! for unrelated things. Kept as they were, the imported layers would show the document's
//! variables and styles instead of their own.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::ids::MODE_ID_INFIX;
use figcraft_doc::proto::{Action, Expr, MAX_EXPR_DEPTH, walk_mut};
use figcraft_doc::{Document, ModeId, NodeId, NodeKind, VariableValue, Variables};

/// A style, variable or collection of the file being imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Asset<'a> {
    Style(&'a str),
    Variable(&'a str),
    Collection(&'a str),
}

/// New ids for styles, variables and collections of a file being imported (its id → the new one).
#[derive(Debug, Default)]
pub(super) struct Renames {
    styles: BTreeMap<String, String>,
    variables: BTreeMap<String, String>,
    collections: BTreeMap<String, String>,
}

impl Renames {
    /// The new ids `other`'s styles, variables and collections take when appended to `doc`.
    ///
    /// One whose id `doc` doesn't use keeps it. One `doc` already has is shared and keeps its id:
    /// the same content under the same id, naming only things that are shared too (the same file
    /// imported again). A collection and its variables are shared only all together, so variables
    /// of one file never join a collection of another. Library copies keep their ids and are shared
    /// with the document's copies, as before (`merge_store` records them). Every other id `doc`
    /// already uses is replaced by a new one from `doc`'s allocators, clear of `other`'s ids too.
    pub(super) fn new(doc: &mut Document, other: &Document) -> Renames {
        let (theirs, libs) = (&other.variables, &other.libraries);
        // Each with whether `doc` has the same under its id (None: nothing there), and whether
        // it's a library copy.
        let styles = other.styles.iter().map(|s| {
            let here = doc.styles.iter().find(|m| m.id == s.id);
            (Asset::Style(&s.id), here.map(|m| m == s), libs.style_import(&s.id).is_some())
        });
        let colls = theirs.collections.iter().map(|c| {
            let here = doc.variables.collection_by_id(&c.id);
            (Asset::Collection(&c.id), here.map(|m| m == c), libs.collection_import(&c.id).is_some())
        });
        let vars = theirs.variables.iter().map(|v| {
            let here = doc.variables.get_by_id(&v.id);
            (Asset::Variable(&v.id), here.map(|m| m == v), libs.variable_import(&v.id).is_some())
        });
        // `taken`: ids `doc` uses; `differs`: what `doc` doesn't have the same of.
        let (mut taken, mut copies, mut differs) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for (a, same, copy) in styles.chain(colls).chain(vars) {
            match same {
                None => {
                    differs.insert(a);
                }
                Some(same) => {
                    taken.insert(a);
                    if copy {
                        copies.insert(a);
                    } else if !same {
                        differs.insert(a);
                    }
                }
            }
        }
        // What differs makes what names it differ (library copies aside), and a collection and its
        // variables differ together.
        let mut named_by: BTreeMap<Asset, Vec<Asset>> = BTreeMap::new();
        for v in &theirs.variables {
            let (var, coll) = (Asset::Variable(&v.id), Asset::Collection(&v.collection));
            named_by.entry(coll).or_default().push(var);
            named_by.entry(var).or_default().push(coll);
            for r in v.values.values().flat_map(VariableValue::references).chain(&v.extends) {
                named_by.entry(Asset::Variable(r)).or_default().push(var);
            }
        }
        for c in &theirs.collections {
            if let Some(e) = &c.extends {
                named_by.entry(Asset::Collection(e)).or_default().push(Asset::Collection(&c.id));
            }
        }
        for s in &other.styles {
            for v in s.bound.values() {
                named_by.entry(Asset::Variable(v)).or_default().push(Asset::Style(&s.id));
            }
        }
        let mut todo: Vec<Asset> = differs.iter().copied().collect();
        while let Some(a) = todo.pop() {
            for &n in named_by.get(&a).into_iter().flatten() {
                if !copies.contains(&n) && differs.insert(n) {
                    todo.push(n);
                }
            }
        }
        let renamed = |a: Asset| taken.contains(&a) && differs.contains(&a);
        Renames {
            collections: fresh(
                theirs.collections.iter().map(|c| &c.id),
                |id| renamed(Asset::Collection(id)),
                |avoid| doc.new_collection_id_avoiding(avoid),
            ),
            variables: fresh(theirs.variables.iter().map(|v| &v.id), |id| renamed(Asset::Variable(id)), |avoid| doc.new_variable_id_avoiding(avoid)),
            styles: fresh(other.styles.iter().map(|s| &s.id), |id| renamed(Asset::Style(id)), |avoid| doc.new_style_id_avoiding(avoid)),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.styles.is_empty() && self.variables.is_empty() && self.collections.is_empty()
    }

    /// `other` with the renamed styles, variables and collections under their new ids, and what
    /// names them following: layers' style links (text ranges too), bindings, explicit modes and
    /// prototype actions; styles' bindings; variables' collections, aliases and calculations; what
    /// extended collections and their variables extend; and the ids of the renamed collections'
    /// modes, which name their collection (`<collection>/mode<k>`).
    pub(super) fn applied(&self, other: &Document) -> Document {
        let theirs = &other.variables;
        let mut doc = other.clone();
        let (styles, vars, colls) = (&self.styles, &self.variables, &self.collections);
        for s in &mut doc.styles {
            rename(styles, &mut s.id);
            s.bound.values_mut().for_each(|v| rename(vars, v));
        }
        for c in &mut doc.variables.collections {
            for m in &mut c.modes {
                self.mode(theirs, &c.id, &mut m.id);
            }
            for id in std::iter::once(&mut c.id).chain(&mut c.extends) {
                rename(colls, id);
            }
        }
        for v in &mut doc.variables.variables {
            let values = std::mem::take(&mut v.values).into_iter().map(|(mut mode, mut val)| {
                self.mode(theirs, &v.collection, &mut mode);
                val.references_mut().into_iter().for_each(|r| rename(vars, r));
                (mode, val)
            });
            v.values = values.collect();
            let overrides = std::mem::take(&mut v.overrides).into_iter().map(|mut mode| {
                self.mode(theirs, &v.collection, &mut mode);
                mode
            });
            v.overrides = overrides.collect();
            rename(colls, &mut v.collection);
            for id in std::iter::once(&mut v.id).chain(&mut v.extends) {
                rename(vars, id);
            }
        }
        let ids: Vec<NodeId> = doc.ids().collect();
        for id in ids {
            let Some(n) = doc.node_mut(id) else { continue };
            let links = [&mut n.styles.fill, &mut n.styles.stroke, &mut n.styles.effect, &mut n.styles.grid, &mut n.styles.text];
            for s in links.into_iter().flatten() {
                rename(styles, s);
            }
            if let NodeKind::Text(t) = &mut n.kind {
                t.map_text_styles(|s| Some(styles.get(s).cloned().unwrap_or_else(|| s.to_string())));
            }
            n.bound.values_mut().for_each(|v| rename(vars, v));
            n.var_modes = std::mem::take(&mut n.var_modes)
                .into_iter()
                .map(|(mut c, mut mode)| {
                    self.mode(theirs, &c, &mut mode);
                    rename(colls, &mut c);
                    (c, mode)
                })
                .collect();
            for it in &mut n.interactions {
                walk_mut(&mut it.actions, &mut |a| self.action(theirs, a));
            }
        }
        doc
    }

    /// Rename mode `mode` of collection `of` (`theirs`: the imported file's variables as they were).
    /// A mode id names the collection it was made in: `of`, or one `of` extends (an extended
    /// collection has the modes of the one it extends). Library copies keep their library's mode
    /// ids, which may name one of the file's own collections.
    fn mode(&self, theirs: &Variables, of: &str, mode: &mut ModeId) {
        let mut at = Some(of);
        for _ in 0..=theirs.collections.len() {
            let Some(c) = at else { return };
            if let Some(k) = mode.strip_prefix(c).and_then(|rest| rest.strip_prefix(MODE_ID_INFIX)) {
                if let Some(new) = self.collections.get(c) {
                    *mode = format!("{new}{MODE_ID_INFIX}{k}");
                }
                return;
            }
            at = theirs.collection_by_id(c).and_then(|c| c.extends.as_deref());
        }
    }

    /// Rename what one prototype action names (not the actions nested in it).
    fn action(&self, theirs: &Variables, a: &mut Action) {
        match a {
            Action::SetVariable { variable, expr, .. } => {
                rename(&self.variables, variable);
                if let Some(e) = expr {
                    self.expr(theirs, e, 0);
                }
            }
            Action::SetVariableMode { collection, mode } => {
                self.mode(theirs, collection, mode);
                rename(&self.collections, collection);
            }
            Action::Conditional { blocks } => blocks.iter_mut().filter_map(|b| b.condition.as_mut()).for_each(|c| self.expr(theirs, c, 0)),
            _ => {}
        }
    }

    fn expr(&self, theirs: &Variables, e: &mut Expr, depth: usize) {
        match e {
            Expr::Value(v) => v.references_mut().into_iter().for_each(|r| rename(&self.variables, r)),
            Expr::Op { args, .. } if depth < MAX_EXPR_DEPTH => args.iter_mut().for_each(|a| self.expr(theirs, a, depth + 1)),
            Expr::Op { .. } => {}
            Expr::Mode { collection } => rename(&self.collections, collection),
            Expr::InMode { variable, mode } => {
                if let Some(v) = theirs.get_by_id(variable) {
                    self.mode(theirs, &v.collection, mode);
                }
                rename(&self.variables, variable);
            }
        }
    }
}

/// New ids, from `new_id`, for those of `ids` that `pick` picks; each clear of all of `ids`.
fn fresh<'a>(
    ids: impl Iterator<Item = &'a String> + Clone,
    pick: impl Fn(&str) -> bool,
    mut new_id: impl FnMut(&BTreeSet<String>) -> String,
) -> BTreeMap<String, String> {
    let mut avoid: BTreeSet<String> = ids.clone().cloned().collect();
    let mut out = BTreeMap::new();
    for id in ids.filter(|id| pick(id)) {
        let new = new_id(&avoid);
        avoid.insert(new.clone());
        out.insert(id.clone(), new);
    }
    out
}

/// `id` as `map` renames it.
fn rename(map: &BTreeMap<String, String>, id: &mut String) {
    if let Some(new) = map.get(id.as_str()) {
        *id = new.clone();
    }
}
