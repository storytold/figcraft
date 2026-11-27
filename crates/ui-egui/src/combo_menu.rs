//! The searchable drop-down menus under a field FigCraft draws itself (the font family picker, the
//! component swap menu): a menu that offers its contents the full height on every pass, so a list
//! that shrank while it was open (a search with no matches) grows back (egui#5225), with a header
//! (the search field) that stays in place above the scrolled list.

use std::ops::Range;

use egui::accesskit::Role;
use egui::style::StyleModifier;
use egui::{Align, Context, Frame, Popup, PopupCloseBehavior, Rect, RectAlign, Response, ScrollArea, TextWrapMode, Ui};

/// A menu under a field. It opens below the field, or above it when there is more room there, and
/// stays on that side while it is open. It is never taller than the room on that side: a long list
/// scrolls.
#[must_use = "You should call .show*"]
pub(crate) struct Menu<'a> {
    popup: Popup<'a>,
    ctx: Context,
    field: Rect,
    height: f32,
    style: StyleModifier,
    close_behavior: PopupCloseBehavior,
}

impl<'a> Menu<'a> {
    /// The menu under `field`, a widget in `ui`.
    pub(crate) fn new(ui: &Ui, field: &Response) -> Self {
        // Below the field or above it, never beside it.
        let flipped: &'static [RectAlign] = if ui.ctx().global_style().right_to_left { &[RectAlign::TOP_END] } else { &[RectAlign::TOP_START] };
        Self {
            popup: Popup::menu(field).align_alternatives(flipped),
            ctx: field.ctx.clone(),
            field: field.rect,
            height: ui.spacing().combo_height,
            style: StyleModifier::from(egui::containers::menu::menu_style),
            close_behavior: PopupCloseBehavior::CloseOnClick,
        }
    }

    /// The maximum height of the menu's contents, inside its frame, kept to the room on its side
    /// of the field.
    pub(crate) fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// When the menu closes.
    pub(crate) fn close_behavior(mut self, close_behavior: PopupCloseBehavior) -> Self {
        self.close_behavior = close_behavior;
        self
    }

    /// The menu's style.
    pub(crate) fn style(mut self, style: impl Into<StyleModifier>) -> Self {
        self.style = style.into();
        self
    }

    /// Shows `header` at the top of the menu and `list` under it while the menu is open, and
    /// returns what `list` returned. `list` gets what `header` returned, such as the rows a search
    /// field's text matches. Only `list` scrolls; the header stays in place. `list` lays out in a
    /// vertical scroll area: give it rows, not a scroll area of its own.
    pub(crate) fn show_ui_with_header<H, R>(self, header: impl FnOnce(&mut Ui) -> H, list: impl FnOnce(&mut Ui, H) -> R) -> Option<R> {
        self.show(header, |scroll, ui, h| scroll.show(ui, |ui| list(ui, h)).inner)
    }

    /// Like [`Self::show_ui_with_header`], with a list of `total_rows(&header's)` rows, each
    /// `row_height` tall without the item spacing, that only lays out the rows in view: `rows`
    /// gets the range to add, as in [`ScrollArea::show_rows`], and what `header` returned.
    pub(crate) fn show_rows_with_header<H, R>(
        self,
        header: impl FnOnce(&mut Ui) -> H,
        row_height: f32,
        total_rows: impl FnOnce(&H) -> usize,
        rows: impl FnOnce(&mut Ui, Range<usize>, H) -> R,
    ) -> Option<R> {
        self.show(header, move |scroll, ui, h| {
            let total = total_rows(&h);
            scroll.show_rows(ui, row_height, total, |ui, range| rows(ui, range, h)).inner
        })
    }

    fn show<H, R>(self, header: impl FnOnce(&mut Ui) -> H, list: impl FnOnce(ScrollArea, &mut Ui, H) -> R) -> Option<R> {
        let Self { popup, ctx, field, height, style, close_behavior } = self;
        let id = popup.get_id();
        let (align, settled) = side(&popup);
        // Until the side is settled, the menu is measured against the whole screen.
        let room = match popup.get_anchor_rect() {
            Some(anchor) if settled => room(align, anchor, ctx.content_rect()),
            _ => ctx.content_rect().height(),
        };
        let mut popup_style = (*ctx.global_style()).clone();
        style.apply(&mut popup_style);
        let frame = Frame::popup(&popup_style).total_margin().sum().y;
        let height = height.min(room - frame).max(0.0);
        let inner = popup
            .align(align)
            .align_alternatives(&[])
            .width(field.width())
            .style(style)
            .close_behavior(close_behavior)
            .show(|ui| {
                ui.set_min_width(ui.available_width());
                // The popup offers last pass's height. Offer the full height instead: the scroll
                // area takes what the header leaves of it, then shrinks to the list.
                ui.set_max_height(height);
                let header = header(ui);
                // Labels widen a narrow menu rather than wrap early, as in egui's combo box. A
                // list left with no room keeps ScrollArea's minimum scrolled height.
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                list(ScrollArea::vertical().max_height(ui.available_height().max(0.0)), ui, header)
            })
            .map(|r| r.inner);
        if inner.is_some() {
            ctx.accesskit_node_builder(id, |node| node.set_role(Role::ListBox));
        }
        inner
    }
}

/// The side of its field `popup` shows on, and whether it is settled. The side is settled on the
/// first pass after the menu was shown and measured: the side egui picks when the menu fits there,
/// else the one with more room. It is kept while the menu stays open, so a menu that shrinks or
/// grows while open doesn't jump to the other side.
fn side(popup: &Popup<'_>) -> (RectAlign, bool) {
    let ctx = popup.ctx();
    let key = popup.get_id().with("figcraft-menu-side");
    if !popup.is_open() || ctx.read_response(popup.get_id()).is_none() {
        ctx.data_mut(|d| d.remove::<RectAlign>(key));
        return (popup.get_best_align(), false);
    }
    if let Some(align) = ctx.data(|d| d.get_temp::<RectAlign>(key)) {
        return (align, true);
    }
    // Outside the data lock: these read the popup's state from memory.
    let mut align = popup.get_best_align();
    if let (Some(size), Some(anchor)) = (popup.get_expected_size(), popup.get_anchor_rect()) {
        let screen = ctx.content_rect();
        let flipped = align.flipped_y();
        if room(align, anchor, screen) < size.y && room(flipped, anchor, screen) > room(align, anchor, screen) {
            align = flipped;
        }
    }
    ctx.data_mut(|d| d.insert_temp(key, align));
    (align, true)
}

/// The height a menu on the `align` side of `anchor` has inside `screen`.
fn room(align: RectAlign, anchor: Rect, screen: Rect) -> f32 {
    match (align.parent.y(), align.child.y()) {
        (Align::Max, Align::Min) => screen.bottom() - anchor.bottom(),
        (Align::Min, Align::Max) => anchor.top() - screen.top(),
        _ => screen.height(),
    }
}
