//! Fill, stroke and effect rows of the Design panel that highlight, the first step of the help
//! center's "Copy and paste properties between layers": click to highlight the property in the
//! right sidebar.
//!
//! A click on a row's background (beside and between its controls, or on its reorder grip)
//! highlights it, one row at a time. The highlight belongs to the selection it was made on, and
//! goes with Escape or a press on the canvas or elsewhere in the Design panel.

use egui::{Color32, Context, Rect, Response};
use figcraft_doc::NodeId;

use crate::{FigApp, RightTab};

/// Which list of the Design panel a row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyKind {
    Fill,
    Stroke,
    Effect,
}

/// The row highlighted in the Design panel. It belongs to the layers selected when it was clicked
/// and goes when they are no longer the selection.
#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
    pub kind: PropertyKind,
    /// The paint's or effect's index in its list (model order: the top one is the last).
    pub index: usize,
    /// The selection the row was highlighted on.
    pub ids: Vec<NodeId>,
    /// The file it was highlighted in.
    pub doc: u64,
    /// Where the row was drawn this frame (`None` until it draws): a press elsewhere lets it go,
    /// and so does a frame it isn't drawn in.
    pub rect: Option<Rect>,
}

/// The highlighted row, when it still belongs to the selection in the Design panel.
pub(crate) fn current(app: &FigApp) -> Option<&Highlight> {
    let h = app.ui.property_row.as_ref()?;
    let ds = app.session.ds().ok()?;
    (app.ui.right_tab == RightTab::Design && ds.uid == h.doc && ds.selection == h.ids).then_some(h)
}

/// Let the highlight go once it no longer belongs to what the Design panel shows (another file or
/// tab, another selection, the panel not `shown`), or when the pointer is pressed on the canvas or
/// elsewhere in the Design panel. A press on its own row keeps it; so does one on a menu, a popup
/// or the toolbar, so a menu command can still act on it. Called each frame before
/// the right panel draws its rows.
pub(crate) fn reconcile(app: &mut FigApp, ctx: &Context, shown: bool) {
    if app.ui.property_row.is_none() {
        return;
    }
    let pressed = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin() } else { None });
    // A menu or popup is a foreground area over whatever is under it.
    let lets_go = pressed.filter(|p| ctx.layer_id_at(*p).is_none_or(|l| l.order != egui::Order::Foreground)).is_some_and(|p| {
        let on_row = app.ui.property_row.as_ref().and_then(|h| h.rect).is_some_and(|r| r.contains(p));
        let in_panel = app.ui.right_panel_rect.is_some_and(|r| r.contains(p));
        !on_row && (in_panel || app.canvas.rect.contains(p))
    });
    // The row draws itself each frame (`row`): one that wasn't drawn last frame is gone (an undo
    // took its paint away, say), and its index may now name another row or none.
    let drawn = app.ui.property_row.as_ref().is_some_and(|h| h.rect.is_some());
    if lets_go || !shown || !drawn || current(app).is_none() {
        app.ui.property_row = None;
    } else if let Some(h) = app.ui.property_row.as_mut() {
        h.rect = None;
    }
}

/// Escape with a highlighted row: the highlight goes and the selection stays. `false` when no row
/// is highlighted (Escape does what it does otherwise).
pub(crate) fn escape(app: &mut FigApp) -> bool {
    let highlighted = current(app).is_some();
    app.ui.property_row = None;
    highlighted
}

/// Row `index` of the `kind` list, drawn as `resp` (a row target, see [`fill`]) for the selection
/// `ids`: a click on its background highlights it; the highlighted row keeps where it was drawn.
pub(crate) fn row(app: &mut FigApp, resp: &Response, kind: PropertyKind, index: usize, ids: &[NodeId]) {
    if resp.clicked() {
        click(app, kind, index, ids, resp.rect);
    } else if let Some(h) = app.ui.property_row.as_mut()
        && h.kind == kind
        && h.index == index
    {
        h.rect = Some(resp.rect);
    }
}

/// A click on row `index` of the `kind` list (drawn at `rect`) for the selection `ids`, on its
/// background or on its reorder grip, the strip left of its fields: it is the highlighted row now.
pub(crate) fn click(app: &mut FigApp, kind: PropertyKind, index: usize, ids: &[NodeId], rect: Rect) {
    let Some(doc) = app.session.ds().ok().map(|d| d.uid) else { return };
    app.ui.property_row = Some(Highlight { kind, index, ids: ids.to_vec(), doc, rect: Some(rect) });
}

/// The background of row `index` of the `kind` list: the selected-row colour when it is the
/// highlighted one.
pub(crate) fn fill(app: &FigApp, kind: PropertyKind, index: usize, selected: Color32) -> Option<Color32> {
    current(app).is_some_and(|h| h.kind == kind && h.index == index).then_some(selected)
}

/// The highlight follows its paint or effect when the rows of `kind` are reordered (`from` moved
/// to `to`) or one is removed (`to` is `None`); it goes with a removed one.
pub(crate) fn follow(app: &mut FigApp, kind: PropertyKind, from: usize, to: Option<usize>) {
    let Some(h) = app.ui.property_row.as_mut().filter(|h| h.kind == kind) else { return };
    let moved = match to {
        Some(to) => Some(figcraft_engine::cmd::lists::moved_index(h.index, from, to)),
        None => figcraft_engine::cmd::lists::removed_index(h.index, from),
    };
    match moved {
        Some(i) => h.index = i,
        None => app.ui.property_row = None,
    }
}
