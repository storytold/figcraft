//! The chrome every detached panel shares: a 15rem-wide card with a 40-point header — the title 16
//! in from its left edge, the X 8 in from its right — a hairline under the header and the
//! three-layer shadow of the design spec, around one slot.
//!
//! It opens against an anchor: its right edge on the anchor's x (the left edge of the right-hand
//! panel), its top on the anchor's y (the top of the button that opened it) — lifted, when that
//! would run its bottom past the screen's bottom, so its bottom keeps [`SCREEN_GAP`] from it. The
//! header drags it anywhere else, and a caller can put it down outright with [`Popover::at`].

use egui::epaint::RectShape;
use egui::{Align2, Color32, Context, CornerRadius, Id, Pos2, Rect, Sense, Shadow, Ui, Vec2, pos2, vec2};

use crate::i18n::tr;
use crate::icons::Icon;
use crate::theme::{Tokens, semibold};
use crate::widgets;

/// 15rem: the width of every popover.
pub const WIDTH: f32 = 240.0;
/// The header's height: the title's line and the X sit in it.
pub const HEADER_H: f32 = 40.0;
/// The title starts this far from the panel's left edge, the X ends this far from its right one.
const TITLE_LEFT: f32 = 16.0;
const CLOSE_RIGHT: f32 = 8.0;
/// The X's button (the icon is inset in it).
const CLOSE: f32 = 24.0;
/// The slot's margin inside the panel.
const SLOT_PAD: f32 = 16.0;
/// A lifted popover keeps this much screen under it; it stays [`EDGE`] inside the others.
const SCREEN_GAP: f32 = 16.0;
const EDGE: f32 = 8.0;
/// The panel's corners.
const RADIUS: u8 = 13;

/// The spec's three shadows — `0 2px 5px rgba(0,0,0,.15)`, `0 10px 16px rgba(0,0,0,.12)`, the
/// `0 0 .5px rgba(0,0,0,.12)` hairline — painted in reverse: the first of them ends up on top.
const SHADOW_NEAR: Shadow = Shadow { offset: [0, 2], blur: 5, spread: 0, color: Color32::from_black_alpha(38) };
const SHADOW_FAR: Shadow = Shadow { offset: [0, 10], blur: 16, spread: 0, color: Color32::from_black_alpha(31) };
const SHADOW_EDGE: Shadow = Shadow { offset: [0, 0], blur: 1, spread: 0, color: Color32::from_black_alpha(31) };

/// Where the popover shows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    /// Against the anchor, until the header drags it (the rule on opening).
    Anchor,
    /// Here: put down by the caller or by the drag.
    At(Pos2),
}

/// An open popover.
pub struct Popover {
    id: Id,
    /// The header's line.
    pub title: String,
    place: Place,
    /// While the header is held down: where in the panel the pointer grabbed it.
    grab: Option<Vec2>,
    /// The panel, its header, its slot and the X, as drawn last frame (0-sized until it has been).
    pub rect: Rect,
    pub header: Rect,
    pub slot: Rect,
    pub close: Rect,
    /// The panel's size as laid out last frame.
    size: Vec2,
}

impl Popover {
    /// A popover titled `title`, opening against an anchor (see [`Self::show`]).
    pub fn new(id: &'static str, title: impl Into<String>) -> Self {
        Self {
            id: Id::new(id),
            title: title.into(),
            place: Place::Anchor,
            grab: None,
            rect: Rect::NOTHING,
            header: Rect::NOTHING,
            slot: Rect::NOTHING,
            close: Rect::NOTHING,
            size: vec2(WIDTH, 0.0),
        }
    }

    /// Its top-left at `pos`, instead of against the anchor.
    pub fn at(mut self, pos: Pos2) -> Self {
        self.place = Place::At(pos);
        self
    }

    /// Draws the panel and fills its slot with `body`; true when the X was clicked.
    ///
    /// Opened against `anchor` — its right edge on `anchor.x` (the right panel's left edge), its top
    /// on `anchor.y` (the top of the button that opened it) — it is lifted when that would run its
    /// bottom past the screen's bottom, so its bottom keeps [`SCREEN_GAP`] from it. The header
    /// drags it; [`Self::at`] and the drag both overrule the anchor.
    pub fn show(&mut self, ctx: &Context, anchor: Pos2, body: impl FnOnce(&mut Ui)) -> bool {
        let t = Tokens::get(ctx);
        let screen = ctx.content_rect();
        // The rule needs the panel's height, which only a layout gives: the first frame lays it
        // out invisibly and the next one places it — a jump would show.
        let first = self.size.y <= 0.0;
        let pos = match self.place {
            Place::At(p) => clamp(p, self.size, screen),
            Place::Anchor => clamp(open_pos(anchor, self.size, screen), self.size, screen),
        };
        let mut closed = false;
        egui::Area::new(self.id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            if first {
                ui.set_invisible();
                ctx.request_repaint();
            }
            // The panel is painted into a place kept before the content: the shadows and the fill
            // go under it, though its size is only known once the slot is laid out.
            let bg = ui.painter().add(egui::Shape::Noop);
            let spacing = ui.spacing().item_spacing;
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            // 1. The header: the title, then the X at the right.
            let (header, _) = ui.allocate_exact_size(vec2(WIDTH, HEADER_H), Sense::hover());
            let close = Rect::from_min_size(pos2(header.max.x - CLOSE_RIGHT - CLOSE, header.center().y - CLOSE / 2.0), Vec2::splat(CLOSE));
            let handle = Rect::from_min_max(header.min, pos2(close.min.x - 6.0, header.max.y));
            let drag = ui.interact(handle, self.id.with("drag"), Sense::drag());
            let font = semibold(11.5);
            let title = widgets::elide_font(ui, &self.title, (handle.width() - TITLE_LEFT - 8.0).max(0.0), font.clone());
            ui.painter().text(pos2(header.min.x + TITLE_LEFT, header.center().y), Align2::LEFT_CENTER, title, font, t.text);
            closed = widgets::icon_button_at(ui, self.id.with("close"), close, Icon::Close, tr("Close"));
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
            let frame = egui::Frame::NONE.inner_margin(egui::Margin::same(SLOT_PAD as i8)).show(ui, |ui| {
                ui.spacing_mut().item_spacing = spacing;
                ui.set_width(WIDTH - 2.0 * SLOT_PAD);
                body(ui);
                ui.min_rect()
            });
            let slot = frame.inner;
            let panel = Rect::from_min_size(header.min, vec2(WIDTH, frame.response.rect.max.y - header.min.y));
            let shade = |s: Shadow| egui::Shape::from(s.as_shape(panel, RADIUS));
            // `inset 0 -1px #E6E6E6` under the header: the theme's border colour, a point thick.
            let hairline = Rect::from_min_max(pos2(panel.min.x, header.max.y - 1.0), pos2(panel.max.x, header.max.y));
            let shapes = vec![
                shade(SHADOW_EDGE),
                shade(SHADOW_FAR),
                shade(SHADOW_NEAR),
                RectShape::filled(panel, CornerRadius::same(RADIUS), t.panel).into(),
                RectShape::filled(hairline, CornerRadius::ZERO, t.border).into(),
            ];
            ui.painter().set(bg, egui::Shape::Vec(shapes));
            self.rect = panel;
            self.header = header;
            self.slot = slot;
            self.close = close;
            self.size = panel.size();
        });
        closed
    }
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
