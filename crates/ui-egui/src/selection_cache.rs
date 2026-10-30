//! Values worked out from the document and the selection that several frames show unchanged
//! (#1571): a pointer move draws about three frames, and a large selection's outlines, slots and
//! mixed values cost milliseconds each time. A value is kept with the document snapshot it was
//! made from (compared by pointer: an edit is a new snapshot), the selection and whatever else it
//! depends on (`extra`: the view's mapping to the screen, a colour…), and made again when any
//! differs.

use std::sync::{Arc, Weak};

use figcraft_doc::{Document, NodeId};

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

/// A screen point as key material.
pub(crate) fn bits(p: egui::Pos2) -> [u64; 2] {
    [u64::from(p.x.to_bits()), u64::from(p.y.to_bits())]
}
