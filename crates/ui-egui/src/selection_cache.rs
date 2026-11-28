//! Values worked out from the document and the selection that several frames show unchanged
//! (#1571): a pointer move draws about three frames, and a large selection's outlines, slots and
//! mixed values cost milliseconds each time. A value is kept with the document snapshot it was
//! made from (compared by pointer: an edit is a new snapshot), the selection and whatever else it
//! depends on (`extra`: the view's mapping to the screen, a colour…), and made again when any
//! differs.

use std::sync::{Arc, Weak};

use figcraft_doc::{Document, Node, NodeId};

/// One kept value and what it was made from. The document is held weakly, so a kept value never
/// keeps an old snapshot alive; the weak reference keeps its address from being reused.
#[derive(Clone)]
struct Kept<T> {
    doc: Weak<Document>,
    sel: Vec<NodeId>,
    extra: Vec<u64>,
    value: T,
}

/// The value `name` for `doc`, `sel` and `extra`: the kept one when they are what it was made
/// from, else `make()`, kept for next time.
pub(crate) fn cached<T: Clone + Send + Sync + 'static>(
    ctx: &egui::Context,
    name: &'static str,
    doc: &Arc<Document>,
    sel: &[NodeId],
    extra: &[u64],
    make: impl FnOnce() -> T,
) -> T {
    let id = egui::Id::unique(("selection_cache", name));
    if let Some(k) = ctx.data(|d| d.get_temp::<Kept<T>>(id))
        && std::ptr::eq(k.doc.as_ptr(), Arc::as_ptr(doc))
        && k.sel == sel
        && k.extra == extra
    {
        return k.value;
    }
    let value = make();
    ctx.data_mut(|d| d.insert_temp(id, Kept { doc: Arc::downgrade(doc), sel: sel.to_vec(), extra: extra.to_vec(), value: value.clone() }));
    value
}

/// While a gesture has only moved the selection (`Interaction::moved_by`): the document it
/// started from and how far it moved them (page units). What depends only on the selected layers'
/// shapes, not where they are, is worked out from that document, so it is kept for the whole
/// gesture instead of made again on every move (#1571).
pub(crate) fn moved(ds: &figcraft_engine::DocState) -> Option<(Arc<Document>, (f64, f64))> {
    let it = ds.interaction.as_ref()?;
    let d = it.moved_by?;
    (it.selection == ds.selection).then(|| (it.doc.clone(), d))
}

/// The document to work out from what a move doesn't change (which layers, their kinds and
/// parents): the one a gesture that only moved the selection started from, kept for the whole
/// gesture ([`moved`]), else `doc`.
pub(crate) fn unmoved(ds: &figcraft_engine::DocState, doc: &Arc<Document>) -> Arc<Document> {
    moved(ds).map_or_else(|| doc.clone(), |(base, _)| base)
}

/// The selected layers of the document the Design panel shows, looked up once: its sections go
/// over the whole selection many times a frame (mixed values, which kinds are selected), and with
/// hundreds of layers selected the lookups cost more than the rest of the panel (#1571).
struct Shown {
    doc: Weak<Document>,
    sel: Vec<NodeId>,
    nodes: Vec<Option<Arc<Node>>>,
}

thread_local! {
    static SHOWN: std::cell::RefCell<Option<Shown>> = const { std::cell::RefCell::new(None) };
}

/// The panel is about to show `sel` of `doc`: [`with_nodes`] over exactly these reads the layers
/// looked up here, kept while the document snapshot and the selection stay the same.
pub(crate) fn show(doc: &Arc<Document>, sel: &[NodeId]) {
    SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        if s.as_ref().is_some_and(|k| std::ptr::eq(k.doc.as_ptr(), Arc::as_ptr(doc)) && k.sel == sel) {
            return;
        }
        *s = Some(Shown { doc: Arc::downgrade(doc), sel: sel.to_vec(), nodes: sel.iter().map(|i| doc.node_arc(*i).cloned()).collect() });
    });
}

/// `f` over the layers of `ids` in `doc`, in order (`None` for one not in it): the ones [`show`]
/// kept when `doc` and `ids` are what it was given, else looked up. The kept document is held
/// weakly, so its address can't be another document's and its contents can't change.
pub(crate) fn with_nodes<R>(doc: &Document, ids: &[NodeId], f: impl FnOnce(&mut dyn Iterator<Item = Option<&Node>>) -> R) -> R {
    SHOWN.with(|s| {
        let s = s.borrow();
        match s.as_ref().filter(|k| std::ptr::eq(k.doc.as_ptr(), doc) && k.sel == ids) {
            Some(k) => f(&mut k.nodes.iter().map(Option::as_deref)),
            None => f(&mut ids.iter().map(|i| doc.node(*i))),
        }
    })
}

/// Whether every one of `ids` is in `doc` and passes `f`.
pub(crate) fn every(doc: &Document, ids: &[NodeId], f: impl Fn(&Node) -> bool) -> bool {
    // `Iterator::all` on the `&mut dyn Iterator` itself (sized), not on the trait object.
    with_nodes(doc, ids, |mut it| Iterator::all(&mut it, |n| n.is_some_and(&f)))
}

/// A screen point as key material.
pub(crate) fn bits(p: egui::Pos2) -> [u64; 2] {
    [u64::from(p.x.to_bits()), u64::from(p.y.to_bits())]
}
