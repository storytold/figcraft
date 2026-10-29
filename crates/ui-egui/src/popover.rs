//! The chrome every detached panel shares: a card with a header — the title 16 points in from its
//! left edge, a 24-point X 8 in from its right — a hairline under the header and the theme's popup
//! shadow, around one slot.
//!
//! The measurements are our own: they are the header the colour picker has always drawn, and the
//! card is as wide as it. The shadow comes from the theme, so the dark theme gets its own. Nothing
//! here is taken from another product's interface.
//!
//! It opens against an anchor (its right edge on it, its top on it, lifted so its bottom keeps off
//! the screen's bottom) or at a point, the header drags it, and it closes itself on the X, on
//! Escape and on a press outside it — except in the rects the caller holds out (the button that
//! opened it, a popover it opened in turn), so that button's own click can toggle it shut instead
//! of the press closing it and the release opening it again.

use egui::epaint::RectShape;
use egui::{Align2, Color32, Context, CornerRadius, Id, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};

use crate::a11y::Tip;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, semibold};
use crate::widgets;

/// The default width: the colour picker's, and the one every detached panel shares.
pub const WIDTH: f32 = 240.0;
/// The header's height: the title's line and the X sit in it.
pub const HEADER_H: f32 = 40.0;
/// The title starts this far from the panel's left edge, the X ends this far from its right one.
const TITLE_LEFT: f32 = 16.0;
const CLOSE_RIGHT: f32 = 8.0;
/// The X's button (the icon is inset in it).
const CLOSE: f32 = 24.0;
/// The slot's default margin inside the panel.
const SLOT_PAD: f32 = 16.0;
/// A lifted popover keeps this much screen under it; it stays [`EDGE`] inside the others.
const SCREEN_GAP: f32 = 16.0;
const EDGE: f32 = 8.0;
/// The panel's corners.
const RADIUS: u8 = 13;

/// The surface a popover draws on: the panels' own, or the dark one the style picker uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Panel,
    Menu,
}

/// Where a popover opens when the caller has not put it down itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Open {
    /// Its right edge on `x` (the right panel's left edge), its top on `y` (the button's top),
    /// lifted when that would run its bottom past the screen's bottom.
    Anchor(Pos2),
    /// Its top-left exactly here.
    At(Pos2),
}

/// What keeps a popover open while a press or Escape happens somewhere else.
#[derive(Clone, Copy, Default)]
pub struct Hold<'a> {
    /// Presses in these rects do not close it: the button that opened it, the popovers it opened.
    pub rects: &'a [Rect],
    /// Escape belongs to something above it (a field being typed in, a picker of its own).
    pub escape: bool,
}

/// Where the popover shows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    /// From [`Open`], until the header drags it.
    Auto,
    /// Here: put down by the caller or by the drag.
    At(Pos2),
}

/// An open popover.
pub struct Popover {
    id: Id,
    /// The header's line.
    pub title: String,
    width: f32,
    pad: f32,
    surface: Surface,
    /// Whether a press outside it closes it.
    outside: bool,
    place: Place,
    /// While the header is held down: where in the panel the pointer grabbed it.
    grab: Option<Vec2>,
    /// The panel, its header, its slot and the X, as drawn last frame.
    pub rect: Rect,
    pub header: Rect,
    pub slot: Rect,
    pub close: Rect,
    /// The panel's size as laid out last frame.
    size: Vec2,
}

impl Popover {
    /// A popover titled `title`; `id` names its egui area, so it must be stable and unique.
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, title: impl Into<String>) -> Self {
        Self::with_id(Id::unique(id), title)
    }

    /// A popover whose egui area is `id` itself — not a hash of it — so a caller that publishes the
    /// id can find the area again (the colour picker's [`crate::colorpicker::window_id`]).
    pub fn with_id(id: Id, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            width: WIDTH,
            pad: SLOT_PAD,
            surface: Surface::Panel,
            outside: true,
            place: Place::Auto,
            grab: None,
            rect: Rect::NOTHING,
            header: Rect::NOTHING,
            slot: Rect::NOTHING,
            close: Rect::NOTHING,
            size: vec2(WIDTH, 0.0),
        }
    }

    /// `width` points wide instead of [`WIDTH`].
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(1.0);
        self.size.x = self.width;
        self
    }

    /// `pad` points of margin around the slot instead of the default 16.
    pub fn pad(mut self, pad: f32) -> Self {
        self.pad = pad.max(0.0);
        self
    }

    /// Drawn on the dark surface the style browser uses.
    pub fn surface(mut self, surface: Surface) -> Self {
        self.surface = surface;
        self
    }

    /// Whether a press outside it closes it (the default). The colour picker stays open, so a
    /// press on a row beside it keeps editing the paint it is on.
    pub fn close_on_outside(mut self, yes: bool) -> Self {
        self.outside = yes;
        self
    }

    /// Its top-left at `pos`, instead of where [`Open`] would put it.
    pub fn at(mut self, pos: Pos2) -> Self {
        self.place = Place::At(pos);
        self
    }

    /// Draws the panel and fills its slot with `body`; true when it asked to close — the X,
    /// Escape or a press outside it (outside [`Hold`]'s rects) on this frame.
    pub fn show(&mut self, ctx: &Context, open: Open, hold: Hold<'_>, body: impl FnOnce(&mut Ui)) -> bool {
        let t = Tokens::get(ctx);
        let dark = self.surface == Surface::Menu;
        let screen = ctx.content_rect();
        // The anchor rule needs the panel's height, which only a layout gives: the first frame
        // lays it out invisibly and the next one places it — a jump would show.
        let first = self.size.y <= 0.0;
        let pos = match self.place {
            Place::At(p) => clamp(p, self.size, screen),
            Place::Auto => {
                let p = match open {
                    Open::At(p) => p,
                    Open::Anchor(a) => open_pos(a, self.size, screen),
                };
                clamp(p, self.size, screen)
            }
        };
        let mut close = false;
        egui::Area::new(self.id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            if first {
                ui.set_invisible();
                ctx.request_repaint();
            }
            // The panel is painted into a place kept before the content: the shadow and the fill
            // go under it, though its size is only known once the slot is laid out.
            let bg = ui.painter().add(egui::Shape::Noop);
            let spacing = ui.spacing().item_spacing;
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            // 1. The header: the title, then the X at the right.
            let (header, _) = ui.allocate_exact_size(vec2(self.width, HEADER_H), Sense::hover());
            let close_rect = Rect::from_min_size(pos2(header.max.x - CLOSE_RIGHT - CLOSE, header.center().y - CLOSE / 2.0), Vec2::splat(CLOSE));
            let handle = Rect::from_min_max(header.min, pos2(close_rect.min.x - 6.0, header.max.y));
            let drag = ui.interact(handle, self.id.with("drag"), Sense::drag());
            let font = semibold(11.0);
            let title = widgets::elide_font(ui, &self.title, (handle.width() - TITLE_LEFT - 8.0).max(0.0), font.clone());
            ui.painter().text(pos2(header.min.x + TITLE_LEFT, header.center().y), Align2::LEFT_CENTER, title, font, header_text(t, dark));
            close = close_button(ui, self.id.with("close"), close_rect, dark, t);
            // 2. The header drags the panel (grabbed where the press landed, so the first frame of
            // a fast drag counts too); the X keeps its own clicks.
            if drag.hovered() && ctx.input(|i| i.pointer.primary_pressed()) {
                self.grab = ctx.input(|i| i.pointer.interact_pos()).map(|at| at - pos);
            }
            if drag.dragged()
                && let (Some(grab), Some(at)) = (self.grab, drag.interact_pointer_pos())
            {
                self.place = Place::At(clamp(at - grab, self.size, screen));
            }
            if ctx.input(|i| i.pointer.any_released()) {
                self.grab = None;
            }
            if drag.dragged() || drag.hovered() {
                ui.ctx().set_cursor_icon(if drag.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            }
            // 3. The slot, under a hairline.
            let frame = egui::Frame::NONE.inner_margin(egui::Margin::same(self.pad as i8)).show(ui, |ui| {
                ui.spacing_mut().item_spacing = spacing;
                ui.set_width(self.width - 2.0 * self.pad);
                body(ui);
                ui.min_rect()
            });
            let slot = frame.inner;
            let panel = Rect::from_min_size(header.min, vec2(self.width, frame.response.rect.max.y - header.min.y));
            let shadow = ctx.global_style().visuals.popup_shadow;
            let fill = if dark { t.menu } else { t.panel };
            let shapes = vec![
                egui::Shape::from(shadow.as_shape(panel, RADIUS)),
                RectShape::filled(panel, CornerRadius::same(RADIUS), fill).into(),
                // A point of the theme's border colour under the header.
                RectShape::filled(
                    Rect::from_min_max(pos2(panel.min.x, header.max.y - 1.0), pos2(panel.max.x, header.max.y)),
                    CornerRadius::ZERO,
                    if dark { Color32::from_white_alpha(24) } else { t.border },
                )
                .into(),
            ];
            ui.painter().set(bg, egui::Shape::Vec(shapes));
            self.rect = panel;
            self.header = header;
            self.slot = slot;
            self.close = close_rect;
            self.size = panel.size();
        });
        // Closing is the widget's: the X above, Escape and a press outside (a child popup or a
        // held-out rect keeps it open, so the button that opened it can click it shut).
        let popup_open = egui::Popup::is_any_open(ctx);
        let escape = !hold.escape && !popup_open && ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let outside = self.outside
            && !popup_open
            && ctx.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer.interact_pos().is_some_and(|at| !self.rect.contains(at) && !hold.rects.iter().any(|r| r.contains(at)))
            });
        close || escape || outside
    }
}

fn header_text(t: Tokens, dark: bool) -> Color32 {
    if dark { t.menu_text } else { t.text }
}

/// The X: a 24-point button, its icon inset 6 (the colour picker's header).
fn close_button(ui: &mut Ui, id: Id, rect: Rect, dark: bool, t: Tokens) -> bool {
    let resp = ui.interact(rect, id, Sense::click()).tip(tr("Close"));
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), if dark { Color32::from_white_alpha(20) } else { t.hover });
    }
    icons::paint(ui.painter(), rect.shrink(6.0), Icon::Close, if dark { t.menu_text } else { t.icon });
    resp.clicked()
}

/// The opening position: the right edge on `anchor.x` (the right panel's left edge), the top on
/// `anchor.y` (the button's top) — lifted, when that runs its bottom past the screen's bottom, to
/// keep [`SCREEN_GAP`] under it.
fn open_pos(anchor: Pos2, size: Vec2, screen: Rect) -> Pos2 {
    let bottom = screen.max.y - SCREEN_GAP;
    let y = if anchor.y + size.y > bottom { bottom - size.y } else { anchor.y };
    pos2(anchor.x - size.x, y)
}

/// Keeps the panel inside the screen: whole when it fits, its top-left corner when it doesn't.
fn clamp(pos: Pos2, size: Vec2, screen: Rect) -> Pos2 {
    let (x0, y0) = (screen.min.x + EDGE, screen.min.y + EDGE);
    let x = pos.x.clamp(x0, (screen.max.x - size.x - EDGE).max(x0));
    let y = pos.y.clamp(y0, (screen.max.y - size.y - EDGE).max(y0));
    pos2(x, y)
}
