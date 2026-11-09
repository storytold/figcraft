//! Fill, stroke and effect rows of the Design panel as things to copy, as the help center's "Copy
//! and paste properties between layers" describes: click to highlight the property in the right
//! sidebar, press Cmd/Ctrl+C, select another layer, press Cmd/Ctrl+V.
//!
//! A click on a row's background (beside and between its controls, or on its reorder grip)
//! highlights it, one row at a time. The highlight belongs to the selection it was made on, and
//! goes with Escape or a press on the canvas or elsewhere in the Design panel. Cmd/Ctrl+C then
//! copies that one paint or effect (`fill.copy`, `stroke.copy`, `effect.copy`, as the row's menu
//! does) instead of the layers, and Cmd/Ctrl+V pastes it onto every selected layer (`fill.paste`,
//! `stroke.paste`, `effect.paste`) while it is still the last thing copied: on top of their list,
//! or in place of their highlighted row when it is of the same kind. A copy keeps its kind: a
//! stroke never lands in the fills, whatever row is highlighted.

use egui::{Color32, Context, Rect, Response};
use figcraft_doc::NodeId;
use serde_json::json;

use crate::{FigApp, RightTab};

/// Which list of the Design panel a row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyKind {
    Fill,
    Stroke,
    Effect,
}

impl PropertyKind {
    /// The command that copies row `index` of this list.
    fn copy_command(self) -> &'static str {
        match self {
            PropertyKind::Fill => "fill.copy",
            PropertyKind::Stroke => "stroke.copy",
            PropertyKind::Effect => "effect.copy",
        }
    }

    /// The command that pastes the copied paint or effect.
    fn paste_command(self) -> &'static str {
        match self {
            PropertyKind::Fill => "fill.paste",
            PropertyKind::Stroke => "stroke.paste",
            PropertyKind::Effect => "effect.paste",
        }
    }

    /// What the system clipboard gets when the copy has no text of its own: the marker that also
    /// stands for copied layers, so a paste never makes a text layer of it.
    fn marker(self) -> &'static str {
        match self {
            PropertyKind::Fill => "figcraft://clipboard/fill",
            PropertyKind::Stroke => "figcraft://clipboard/stroke",
            PropertyKind::Effect => "figcraft://clipboard/effect",
        }
    }

    /// Whether the engine clipboard still holds a copy of this kind.
    fn on_clipboard(self, app: &FigApp) -> bool {
        let clip = &app.session.clipboard;
        match self {
            PropertyKind::Fill => clip.paint.as_ref().is_some_and(|(fills, _)| *fills),
            PropertyKind::Stroke => clip.paint.as_ref().is_some_and(|(fills, _)| !*fills),
            PropertyKind::Effect => clip.effect.is_some(),
        }
    }
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

/// A paint or effect copied from its row: Cmd/Ctrl+V pastes it rather than the copied layers while
/// it is the last thing copied.
#[derive(Clone, Debug, PartialEq)]
pub struct Copied {
    pub kind: PropertyKind,
    /// The engine clipboard's layers generation at the copy: a layer copy since moves it on.
    pub layers_generation: u64,
    /// What the copy put on the system clipboard (`None`: a programmatic copy, which never writes
    /// it). A user's paste takes the property only while the system clipboard is still that.
    pub mirror: Option<String>,
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
/// or the toolbar, so Edit ▸ Copy still copies the highlighted property. Called each frame before
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

/// Cmd/Ctrl+C with a highlighted row: copy its paint or effect. `false` when no row is
/// highlighted (the layers are copied as usual).
pub(crate) fn copy_highlighted(app: &mut FigApp) -> bool {
    let Some(h) = current(app).cloned() else { return false };
    copy(app, h.kind, h.index, &h.ids);
    true
}

/// Copy paint or effect `index` of the `kind` list of `ids` (a highlighted row, or the row's menu)
/// and remember it as the last thing copied. A solid colour goes to the system clipboard as its
/// hex code, as other apps read colours; anything else as a marker of the copy.
pub(crate) fn copy(app: &mut FigApp, kind: PropertyKind, index: usize, ids: &[NodeId]) {
    let Some(copied) = app.run(kind.copy_command(), json!({"ids": ids, "index": index})) else { return };
    let layers_generation = app.session.clipboard.layers_generation;
    let mirror = if app.control_synthetic {
        // The engine's copy is no longer what the system clipboard mirrors: a user's paste must
        // not take that text for this copy (`menus::paste` would paste stale layers).
        if app.ui.property_copy.as_ref().is_some_and(|c| c.mirror.is_some() && c.mirror == app.ui.last_copy) {
            app.ui.last_copy = None;
        }
        None
    } else {
        let text = match kind {
            PropertyKind::Effect => None,
            PropertyKind::Fill | PropertyKind::Stroke => {
                serde_json::from_value::<figcraft_color::Paint>(copied).ok().and_then(|p| p.solid_color()).map(figcraft_color::Color::to_css_hex)
            }
        }
        .unwrap_or_else(|| kind.marker().to_string());
        app.clipboard_set_text(&text);
        // The system clipboard holds our copy now: a paste reads it as ours (`menus::paste`).
        app.ui.last_copy = Some(text.clone());
        app.ui.last_copy_generation = layers_generation;
        Some(text)
    };
    app.ui.property_copy = Some(Copied { kind, layers_generation, mirror });
}

/// Cmd/Ctrl+V when nothing newer than our own copy is on the system clipboard: when a paint or
/// effect copied from its row is the last thing copied, paste it onto the selected layers (in
/// place of their highlighted row of that kind, else on top) and return `true`; `false` leaves
/// the paste to the copied layers.
pub(crate) fn paste(app: &mut FigApp) -> bool {
    let Some(c) = app.ui.property_copy.clone() else { return false };
    let still_last = c.layers_generation == app.session.clipboard.layers_generation
        && c.kind.on_clipboard(app)
        // A user's paste: the system clipboard must still be what this copy put there. A
        // programmatic paste reads only the engine clipboard.
        && (app.control_synthetic || (c.mirror.is_some() && app.ui.last_copy == c.mirror));
    if !still_last {
        // The system clipboard still holds this copy's mirror, but the engine's copy of it is gone
        // (a command copied a paint or effect of another kind since): nothing to paste, rather
        // than the layers copied before it.
        return !app.control_synthetic
            && c.mirror.is_some()
            && app.ui.last_copy == c.mirror
            && app.ui.last_copy_generation == app.session.clipboard.layers_generation
            && c.layers_generation == app.session.clipboard.layers_generation;
    }
    let ids = app.session.selection();
    if ids.is_empty() {
        // Nothing to paste onto: a paste of the layers copied before would be surprising.
        return true;
    }
    let cmd = c.kind.paste_command();
    if crate::menus::dev_mode_refuses(app, cmd) {
        return true;
    }
    let mut params = json!({"ids": ids});
    if let Some(h) = current(app).filter(|h| h.kind == c.kind) {
        params["index"] = json!(h.index);
    }
    app.run(cmd, params);
    true
}
