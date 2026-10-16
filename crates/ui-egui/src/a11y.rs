//! Accessibility: names, roles and states for the custom-painted widgets (the AccessKit tree a
//! screen reader reads), and the keyboard navigation mode.
//!
//! Keyboard navigation: F6 / Shift+F6 move the keyboard between the regions of the window (the
//! toolbar, the layers panel, the canvas and the properties panel); Tab / Shift+Tab move between
//! the controls of a region (wrapping at its ends); Enter or Space activate the control; the
//! arrows move along the toolbar and through the layer rows; a menu or flyout opened from the
//! keyboard takes it, and gives it back to its button when it closes; Escape goes back to the
//! canvas, where every shortcut works as usual. A focus ring shows the control with the keyboard
//! until the next pointer press.
//!
//! The regions' controls come from what the last frame drew (egui keeps every widget's rectangle
//! and sense), so panels don't register their controls. Widget names are only built while a screen
//! reader asks for the tree (egui runs the node writers only then).
//!
//! Locking: egui runs the node writers (`accesskit_node_builder`) and the `ctx.data`, `ctx.input`,
//! `ctx.memory` and `ctx.viewport` closures with its context locked. Nothing in them may call the
//! context again (a nested call deadlocks; debug builds panic after 10 s): read what they need
//! first, as everything here does (`a11y_tests` runs every path with the tree on).

use egui::accesskit::{self, Role, Toggled};
use egui::{Context, CornerRadius, Event, FocusDirection, Id, Key, LayerId, Modifiers, Order, Pos2, Rect, Response, Stroke, StrokeKind, Ui};
use figcraft_engine::keymap;

use crate::FigApp;
use crate::i18n::tr;
use crate::theme::Tokens;

/// The commands F6 and Shift+F6 are bound to.
pub const NEXT_REGION: &str = "view.nextRegion";
pub const PREVIOUS_REGION: &str = "view.previousRegion";

/// A part of the window F6 moves the keyboard to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Region {
    Toolbar,
    Layers,
    #[default]
    Canvas,
    Properties,
}

impl Region {
    /// In F6 order.
    pub const ALL: [Region; 4] = [Region::Toolbar, Region::Layers, Region::Canvas, Region::Properties];

    pub fn id(self) -> &'static str {
        match self {
            Region::Toolbar => "toolbar",
            Region::Layers => "layers",
            Region::Canvas => "canvas",
            Region::Properties => "properties",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.id().eq_ignore_ascii_case(s))
    }

    fn index(self) -> usize {
        self as usize
    }

    /// This region's entry of a per-region array.
    fn of<T>(self, a: &[T; 4]) -> &T {
        let [t, l, c, p] = a;
        match self {
            Region::Toolbar => t,
            Region::Layers => l,
            Region::Canvas => c,
            Region::Properties => p,
        }
    }

    fn of_mut<T>(self, a: &mut [T; 4]) -> &mut T {
        let [t, l, c, p] = a;
        match self {
            Region::Toolbar => t,
            Region::Layers => l,
            Region::Canvas => c,
            Region::Properties => p,
        }
    }
}

/// What a frame drew that the next frame's keyboard navigation needs.
#[derive(Clone, Copy, Default)]
struct Drawn {
    /// Where each region is (None: not shown).
    rects: [Option<Rect>; 4],
    /// The control F6 lands on in each region (the active tool, the selected layer's row).
    preferred: [Option<Id>; 4],
    /// The canvas widget.
    canvas: Option<Id>,
    /// An open popup the app draws itself: (the control that opened it, the popup's area id).
    popup: Option<(Id, Id)>,
    /// A layer row had the keyboard.
    row_focused: bool,
}

/// Where the keyboard goes on the next frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Move {
    /// By this many regions (F6, Shift+F6).
    Step(i32),
    /// To this region.
    To(Region),
}

/// Keyboard navigation state (in [`crate::UiState`]).
#[derive(Default)]
pub struct Nav {
    /// The region with the keyboard.
    pub region: Region,
    /// Keyboard mode: the keyboard moved the focus since the last pointer press (the ring shows).
    pub keyboard: bool,
    last: Drawn,
    next: Drawn,
    pending: Option<Move>,
    /// Frames left to give the selected layer's row the keyboard (the arrows moved the selection,
    /// and its row may only be laid out a frame later).
    refocus: u8,
    /// The control that opened the popup the keyboard went into, and the popup's layer.
    opener: Option<(Id, LayerId)>,
    /// A control the keyboard moved to: the properties panel scrolls it into view.
    scroll_to: Option<Id>,
}

impl Nav {
    /// Move the keyboard `step` regions on (`view.nextRegion` / `view.previousRegion`).
    pub fn step(&mut self, step: i32) {
        self.pending = Some(match self.pending {
            Some(Move::Step(s)) => Move::Step(s.saturating_add(step)),
            _ => Move::Step(step),
        });
    }

    /// Give `region` the keyboard (control channel `ui.set {focusRegion}`).
    pub fn go(&mut self, region: Region) {
        self.pending = Some(Move::To(region));
    }

    /// The region the point `p` is in (the toolbar floats over the canvas; the side panels scroll,
    /// so a control scrolled below a panel's bottom still belongs to it).
    fn region_at(&self, p: Pos2) -> Option<Region> {
        let rect = |r: Region| *r.of(&self.last.rects);
        if rect(Region::Toolbar).is_some_and(|r| r.contains(p)) {
            return Some(Region::Toolbar);
        }
        for r in [Region::Layers, Region::Properties] {
            if rect(r).is_some_and(|rect| rect.x_range().contains(p.x) && p.y >= rect.min.y) {
                return Some(r);
            }
        }
        rect(Region::Canvas).filter(|r| r.contains(p)).map(|_| Region::Canvas)
    }

    fn shown(&self, r: Region) -> bool {
        r == Region::Canvas || r.of(&self.last.rects).is_some()
    }
}

// ---------------------------------------------------------------------------------------------
// What the panels tell navigation while they draw.

/// `ui` draws (part of) `region` this frame; it is also a container named `label` for screen
/// readers. Call it before the region's widgets.
pub fn region(nav: &mut Nav, ui: &Ui, region: Region, role: Role, label: &str) {
    area(nav, region, ui.max_rect());
    container(ui, role, label);
}

/// `region` covers `rect` this frame (added to what it already covers).
pub fn area(nav: &mut Nav, region: Region, rect: Rect) {
    let slot = region.of_mut(&mut nav.next.rects);
    *slot = Some(slot.map_or(rect, |s| s.union(rect)));
}

/// A container node named `label` holding `ui`'s widgets (call before them).
pub fn container(ui: &Ui, role: Role, label: &str) {
    let r = ui.max_rect();
    ui.ctx().accesskit_node_builder(ui.unique_id(), |n| {
        n.set_role(role);
        n.set_label(label);
        n.set_bounds(bounds(r));
    });
}

/// `ui`'s container node covers `rect` (a container smaller than its `Ui`: the toolbar).
pub fn container_bounds(ui: &Ui, rect: Rect) {
    ui.ctx().accesskit_node_builder(ui.unique_id(), |n| n.set_bounds(bounds(rect)));
}

/// F6 into `region` lands on `id` (the active tool, the selected layer's row).
pub fn prefer(nav: &mut Nav, region: Region, id: Id) {
    *region.of_mut(&mut nav.next.preferred) = Some(id);
}

/// The canvas widget: F6 and Escape give it the keyboard (shortcuts keep working while it has it);
/// screen readers read it as "Canvas" with `describe()` (what is selected) as its description.
pub fn canvas(nav: &mut Nav, resp: &Response, describe: impl FnOnce() -> String) {
    nav.next.canvas = Some(resp.id);
    area(nav, Region::Canvas, resp.rect);
    shared(&resp.ctx, |s| s.canvas = Some(resp.id));
    node(resp, tr("Canvas"), |n| {
        n.set_role(Role::Group);
        n.set_description(describe());
    });
}

/// An open popup drawn by the app itself (egui `Area` `area`), opened from `opener`: a keyboard
/// user who opened it moves into it.
pub fn popup(nav: &mut Nav, opener: Id, area: Id) {
    nav.next.popup = Some((opener, area));
}

/// A layer row: while it has the keyboard, the arrows move through the layers.
pub fn layer_row(nav: &mut Nav, resp: &Response) {
    if resp.has_focus() {
        nav.next.row_focused = true;
    }
}

/// The properties panel's scroll area: scroll the control the keyboard just moved to into view.
/// Call at the end of the scroll area's contents.
pub fn scroll_focus(nav: &mut Nav, ui: &Ui) {
    if let Some(id) = nav.scroll_to.take()
        && let Some(r) = ui.ctx().read_response(id)
        && nav.region_at(r.rect.center()) == Some(Region::Properties)
        && !ui.clip_rect().contains_rect(r.rect)
    {
        r.scroll_to_me(None);
    }
}

// ---------------------------------------------------------------------------------------------
// State the widget helpers read from the context (they only see a `Ui` or a `Response`).

#[derive(Clone, Default)]
struct Shared {
    canvas: Option<Id>,
    keyboard: bool,
    /// The focused widget showed its focus itself this frame (no ring needed).
    ring: Option<Id>,
    /// The focused widget's ring goes round this rectangle (clipped to the second) instead of the
    /// widget: a control made of several widgets, a row whose visible part is inset.
    ring_rect: Option<(Id, Rect, Rect)>,
    /// The focused control's name (`ui.inspect` › `focus.control`).
    name: String,
}

fn shared<R>(ctx: &Context, f: impl FnOnce(&mut Shared) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Shared>(Id::new("figcraft_a11y"))))
}

/// Whether a widget other than the canvas has the keyboard (a field, a focused button): the
/// canvas shortcuts stay off then.
pub fn field_focused(ctx: &Context) -> bool {
    let canvas = shared(ctx, |s| s.canvas);
    ctx.memory(|m| m.focused()).is_some_and(|f| Some(f) != canvas)
}

/// The name of the control with the keyboard (empty when unnamed or nothing has it).
pub fn focused_name(ctx: &Context) -> String {
    if ctx.memory(|m| m.focused()).is_none() {
        return String::new();
    }
    shared(ctx, |s| s.name.clone())
}

/// Whether a screen reader reads the tree this frame: spelled-out values (colours, zoom) are only
/// built then.
pub fn active(ctx: &Context) -> bool {
    ctx.viewport(|v| v.this_pass.accesskit_state.is_some())
}

/// Keyboard mode is on (the focus ring shows).
pub fn keyboard(ctx: &Context) -> bool {
    shared(ctx, |s| s.keyboard)
}

// ---------------------------------------------------------------------------------------------
// Frame hooks.

/// Before the panels draw: pointer presses, the focused control's region, and the navigation
/// keys (F6, Tab, arrows, Escape), which are taken out of the input when they move the keyboard.
pub fn begin(app: &mut FigApp, ctx: &Context) {
    app.ui.nav.last = std::mem::take(&mut app.ui.nav.next);
    keys(app, ctx);
    let keyboard = app.ui.nav.keyboard;
    shared(ctx, |s| {
        s.keyboard = keyboard;
        s.ring = None;
        s.ring_rect = None;
        // Written again by the focused control as it draws.
        s.name.clear();
    });
}

fn keys(app: &mut FigApp, ctx: &Context) {
    // A press hides the ring, and the keyboard goes to the region pressed in.
    let press = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
    if let Some(p) = press {
        let nav = &mut app.ui.nav;
        nav.keyboard = false;
        nav.opener = None;
        nav.refocus = 0;
        if ctx.layer_id_at(p).is_none_or(|l| l == LayerId::background())
            && let Some(r) = nav.region_at(p)
        {
            nav.region = r;
        }
    }
    let focused = ctx.memory(|m| m.focused());
    let widget = focused.and_then(|id| last_widget(ctx, id));
    let on_background = widget.is_none_or(|w| w.layer_id == LayerId::background());
    // The control with the keyboard decides the region (a click into a field, a Tab in a panel);
    // a press elsewhere takes the keyboard from it later this frame.
    if let Some(w) = widget.filter(|_| on_background && press.is_none())
        && let Some(r) = app.ui.nav.region_at(w.rect.center())
    {
        app.ui.nav.region = r;
    }
    // Dialogs, the Actions palette and the presenter keep egui's own keyboard handling.
    if blocked(app) {
        app.ui.nav.pending = None;
        return;
    }
    // F6 / Shift+F6 (and the commands they run).
    if let Some((id, key, m)) = binding_pressed(ctx, &[NEXT_REGION, PREVIOUS_REGION]) {
        ctx.input_mut(|i| i.consume_key(m, key));
        app.ui.nav.step(if id == NEXT_REGION { 1 } else { -1 });
    }
    if let Some(mv) = app.ui.nav.pending.take() {
        let to = match mv {
            Move::To(r) => r,
            Move::Step(s) => next_region(&app.ui.nav, s),
        };
        // egui's own Tab and arrow moves would undo the move.
        ctx.memory_mut(|m| m.move_focus(FocusDirection::None));
        enter(app, ctx, to);
        return;
    }
    popups(app, ctx, focused, widget);
    if !on_background {
        return;
    }
    let region = app.ui.nav.region;
    // The modifiers each key was pressed with (not the ones held when the frame began).
    let pressed = |k: Key, m: Modifiers| {
        ctx.input(|i| {
            i.events.iter().any(|e| matches!(e, Event::Key { key, pressed: true, modifiers, .. } if *key == k && modifiers.matches_exact(m)))
        })
    };
    // Tab moves between the region's controls (on the canvas it stays the select-next shortcut).
    // A layer rename takes Tab itself (on to the next row's name).
    let tab = if pressed(Key::Tab, Modifiers::NONE) {
        Some(1)
    } else if pressed(Key::Tab, Modifiers::SHIFT) {
        Some(-1)
    } else {
        None
    };
    if let Some(step) = tab
        && app.ui.rename.is_none()
    {
        ctx.memory_mut(|m| m.move_focus(FocusDirection::None));
        if region != Region::Canvas {
            ctx.input_mut(|i| {
                i.consume_key(Modifiers::SHIFT, Key::Tab);
                i.consume_key(Modifiers::NONE, Key::Tab);
            });
            app.ui.nav.keyboard = true;
            step_within(app, ctx, region, step);
        }
    }
    // egui moves the focus with the arrows too; here they belong to fields (steps), the toolbar
    // and the layer rows.
    let arrows = [Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown];
    if focused.is_some() && arrows.iter().any(|k| ctx.input(|i| i.key_pressed(*k))) {
        ctx.memory_mut(|m| m.move_focus(FocusDirection::None));
    }
    let in_region =
        focused.is_some_and(|f| widget.is_some_and(|w| app.ui.nav.region_at(w.rect.center()) == Some(region)) && Some(f) != app.ui.nav.last.canvas);
    match region {
        Region::Toolbar if in_region => {
            for (k, step) in [(Key::ArrowLeft, -1), (Key::ArrowRight, 1)] {
                if pressed(k, Modifiers::NONE) {
                    ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
                    app.ui.nav.keyboard = true;
                    step_within(app, ctx, region, step);
                }
            }
            for (k, last) in [(Key::Home, false), (Key::End, true)] {
                if pressed(k, Modifiers::NONE) {
                    ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
                    app.ui.nav.keyboard = true;
                    let list = focusables(ctx, &app.ui.nav, region);
                    if let Some(id) = if last { list.last() } else { list.first() } {
                        focus(app, ctx, id.0);
                    }
                }
            }
        }
        Region::Layers if in_region && app.ui.nav.last.row_focused => {
            // The canvas shortcuts are off while a row has the keyboard: rename (F2, Cmd/Ctrl+R)
            // and the context menu (Shift+F10) work on its layer here.
            if let Some((id, key, m)) = binding_pressed(ctx, &["object.rename", "view.contextMenu"]) {
                ctx.input_mut(|i| i.consume_key(m, key));
                crate::shortcuts::run_binding(app, id);
            }
            for k in arrows {
                let shift = matches!(k, Key::ArrowUp | Key::ArrowDown) && pressed(k, Modifiers::SHIFT);
                if (pressed(k, Modifiers::NONE) || shift) && crate::layers::key_nav(app, k, shift) {
                    ctx.input_mut(|i| i.consume_key(if shift { Modifiers::SHIFT } else { Modifiers::NONE }, k));
                    app.ui.nav.keyboard = true;
                    app.ui.nav.refocus = 3;
                }
            }
        }
        _ => {}
    }
    // Escape gives the keyboard back to the canvas (a popup it closes keeps it where it was; a layer
    // rename it cancels gives it back to the row).
    if region != Region::Canvas
        && app.ui.rename.is_none()
        && pressed(Key::Escape, Modifiers::NONE)
        && !egui::Popup::is_any_open(ctx)
        && app.ui.nav.opener.is_none()
        && !menus_open(app)
    {
        // From the keyboard the canvas takes it; after the pointer (Escape in a field) only the
        // region changes.
        if app.ui.nav.keyboard {
            enter(app, ctx, Region::Canvas);
        } else {
            app.ui.nav.region = Region::Canvas;
        }
        return;
    }
    // In keyboard mode the canvas keeps the keyboard (Escape on it deselects and lets egui drop it).
    if app.ui.nav.keyboard
        && region == Region::Canvas
        && ctx.memory(|m| m.focused()).is_none()
        && let Some(c) = app.ui.nav.last.canvas
    {
        ctx.memory_mut(|m| m.request_focus(c));
    }
}

/// The first of the commands `ids` a key pressed this frame is bound to (canvas keys), with the key.
fn binding_pressed(ctx: &Context, ids: &[&str]) -> Option<(&'static str, Key, Modifiers)> {
    ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key { key, physical_key, pressed: true, modifiers, .. } => {
                crate::shortcuts::lookup(keymap::Context::Canvas, *key, *physical_key, *modifiers)
                    .filter(|b| ids.contains(&b.id))
                    .map(|b| (b.id, *key, *modifiers))
            }
            _ => None,
        })
    })
}

/// A layer rename from the keyboard ended: the selected layer's row takes the keyboard again.
pub fn refocus_row(nav: &mut Nav) {
    if nav.keyboard {
        nav.region = Region::Layers;
        nav.refocus = 3;
    }
}

/// Open the context menu from the keyboard (Shift+F10, `view.contextMenu`): for the selection, at
/// its layer row when the layers panel has the keyboard, else on the canvas below the selection
/// (the middle of the canvas with nothing selected). The menu takes the keyboard (keyboard mode),
/// and gives it back to the row or the canvas when it closes. Call after the panels drew.
pub fn context_menu(app: &mut FigApp, ctx: &Context) {
    let nav = &app.ui.nav;
    let rect_of = |id: Id| ctx.viewport(|v| v.this_pass.widgets.get(id).or_else(|| v.prev_pass.widgets.get(id)).map(|w| w.rect));
    let row = (app.ui.layers_focus || nav.region == Region::Layers)
        .then(|| Region::Layers.of(&nav.next.preferred).or(*Region::Layers.of(&nav.last.preferred)))
        .flatten()
        .and_then(|id| Some((id, rect_of(id)?)));
    let (opener, pos, region) = match row {
        Some((id, r)) => (Some(id), egui::pos2(r.min.x + 40.0, r.max.y), Region::Layers),
        None => {
            let canvas = app.canvas.rect;
            let below = app.session.ds().ok().and_then(|ds| {
                let b = ds.selection.iter().filter_map(|i| ds.doc.world_bounds(*i)).reduce(|a, b| a.union(b))?;
                Some(egui::pos2(app.to_screen(b.x0, b.y1).x, app.to_screen(b.x0, b.y1).y + 8.0))
            });
            let pos = below.filter(|p| canvas.contains(*p)).unwrap_or(canvas.center());
            (nav.next.canvas.or(nav.last.canvas), pos, Region::Canvas)
        }
    };
    app.ui.context_menu = Some((pos, app.session.selection()));
    let nav = &mut app.ui.nav;
    nav.keyboard = true;
    nav.region = region;
    nav.opener = None;
    if let Some(id) = opener {
        ctx.memory_mut(|m| m.request_focus(id));
    }
    ctx.request_repaint();
}

/// After everything drew: the layer row the arrows moved to takes the keyboard, and the focus ring.
pub fn end(app: &mut FigApp, ctx: &Context) {
    // A popup registered before it closed later in the frame (Escape) isn't open any more.
    if !menus_open(app) {
        app.ui.nav.next.popup = None;
    }
    let nav = &mut app.ui.nav;
    if nav.refocus > 0 {
        nav.refocus -= 1;
        if let Some(id) = *Region::Layers.of(&nav.next.preferred) {
            if ctx.memory(|m| m.focused()) != Some(id) {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            nav.refocus = 0;
        }
        ctx.request_repaint();
    }
    if nav.keyboard {
        ring(ctx);
    }
}

/// Dialogs, the palette, the presenter and the home screen have no regions.
fn blocked(app: &FigApp) -> bool {
    let u = &app.ui;
    u.palette.is_some()
        || u.about
        || u.preferences
        || u.variables.is_some()
        || u.export_preview
        || u.color_profile.is_some()
        || u.present.is_some()
        || u.home
        || u.find.is_some()
        || !app.has_files()
}

/// Menus the app draws itself (Escape closes them, and the keyboard stays put).
fn menus_open(app: &FigApp) -> bool {
    app.ui.main_menu || app.ui.tool_flyout.is_some() || app.ui.context_menu.is_some()
}

/// The region `step` regions on from the current one, skipping regions not shown.
fn next_region(nav: &Nav, step: i32) -> Region {
    // A full turn at most each way.
    let step = step.clamp(-4, 4);
    let n = Region::ALL.len() as i32;
    let cur = nav.region.index() as i32;
    let dir = step.signum();
    let mut i = cur;
    let mut left = step.abs();
    for _ in 0..(n * step.abs().max(1)) {
        if left == 0 {
            break;
        }
        i = (i + dir).rem_euclid(n);
        if Region::ALL.get(i as usize).is_some_and(|r| nav.shown(*r)) {
            left -= 1;
        }
    }
    Region::ALL.get(i as usize).copied().unwrap_or_default()
}

/// Give `region` the keyboard: the canvas, or the region's preferred control, else its first.
/// Open menus close.
fn enter(app: &mut FigApp, ctx: &Context, region: Region) {
    egui::Popup::close_all(ctx);
    app.ui.main_menu = false;
    app.ui.tool_flyout = None;
    app.ui.context_menu = None;
    let nav = &mut app.ui.nav;
    nav.region = region;
    nav.keyboard = true;
    nav.opener = None;
    nav.refocus = 0;
    if region == Region::Canvas {
        match nav.last.canvas {
            Some(c) => ctx.memory_mut(|m| m.request_focus(c)),
            None => drop_focus(ctx),
        }
        ctx.request_repaint();
        return;
    }
    let list = focusables(ctx, nav, region);
    let target = region.of(&nav.last.preferred).filter(|p| list.iter().any(|(id, _)| id == p)).or_else(|| list.first().map(|w| w.0));
    match target {
        Some(id) => focus(app, ctx, id),
        None => drop_focus(ctx),
    }
}

fn drop_focus(ctx: &Context) {
    ctx.memory_mut(|m| {
        if let Some(f) = m.focused() {
            m.surrender_focus(f);
        }
    });
}

fn focus(app: &mut FigApp, ctx: &Context, id: Id) {
    ctx.memory_mut(|m| m.request_focus(id));
    if app.ui.nav.region == Region::Properties {
        app.ui.nav.scroll_to = Some(id);
    }
    ctx.request_repaint();
}

/// Tab / Shift+Tab (and the toolbar's arrows): the next or previous control in the region,
/// wrapping. Going back from a field being edited skips the field it is part of.
fn step_within(app: &mut FigApp, ctx: &Context, region: Region, step: i32) {
    let list = focusables(ctx, &app.ui.nav, region);
    let n = list.len() as i32;
    if n == 0 {
        return;
    }
    let focused = ctx.memory(|m| m.focused());
    let cur = focused.and_then(|f| list.iter().position(|(id, _)| *id == f));
    let Some(cur) = cur else {
        let first = if step > 0 { list.first() } else { list.last() };
        if let Some((id, _)) = first {
            focus(app, ctx, *id);
        }
        return;
    };
    let here = list.get(cur).map(|w| w.1.center());
    let mut i = cur as i32;
    for _ in 0..n {
        i = (i + step.signum()).rem_euclid(n);
        let Some((id, rect)) = list.get(i as usize) else { break };
        // The control a focused field editor sits in.
        if here.is_some_and(|c| rect.contains(c)) && i as usize != cur {
            continue;
        }
        focus(app, ctx, *id);
        return;
    }
}

/// The controls of `region` the last frame drew, in drawing order (the panels' edges, which resize
/// them, are left out).
fn focusables(ctx: &Context, nav: &Nav, region: Region) -> Vec<(Id, Rect)> {
    let canvas = nav.last.canvas;
    let edges = [Id::new("left_panel").with("__resize"), Id::new("right_panel").with("__resize")];
    ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .get_layer(LayerId::background())
            .filter(|w| {
                w.enabled
                    && w.sense.is_focusable()
                    && w.rect.is_positive()
                    && w.rect.is_finite()
                    && Some(w.id) != canvas
                    && !edges.contains(&w.id)
                    && nav.region_at(w.rect.center()) == Some(region)
            })
            .map(|w| (w.id, w.rect))
            .collect()
    })
}

fn last_widget(ctx: &Context, id: Id) -> Option<egui::WidgetRect> {
    ctx.viewport(|v| v.prev_pass.widgets.get(id).copied())
}

/// The first control of `layer` the last frame drew (not the area itself: egui's `Area` senses
/// clicks on its background as `<area id>.with("move")`).
fn first_in(ctx: &Context, layer: LayerId) -> Option<Id> {
    let background = layer.id.with("move");
    ctx.viewport(|v| {
        v.prev_pass.widgets.get_layer(layer).find(|w| w.enabled && w.sense.is_focusable() && w.rect.is_positive() && w.id != background).map(|w| w.id)
    })
}

/// A menu opened from the keyboard takes it (its first item); when the keyboard leaves the menu
/// (Escape, or a pick that closes it), the control that opened it gets it back.
fn popups(app: &mut FigApp, ctx: &Context, focused: Option<Id>, widget: Option<egui::WidgetRect>) {
    let nav = &mut app.ui.nav;
    if let Some((opener, layer)) = nav.opener {
        let in_menu = widget.is_some_and(|w| w.layer_id == layer);
        if focused.is_none() {
            nav.opener = None;
            if last_widget(ctx, opener).is_some() {
                ctx.memory_mut(|m| m.request_focus(opener));
                ctx.request_repaint();
            }
        } else if !in_menu && first_in(ctx, layer).is_none() {
            nav.opener = None;
        }
        return;
    }
    if !nav.keyboard {
        return;
    }
    let Some(f) = focused.filter(|_| widget.is_some_and(|w| w.layer_id == LayerId::background())) else { return };
    let layer = match nav.last.popup {
        Some((opener, area)) if opener == f => LayerId::new(Order::Foreground, area),
        _ => {
            // egui's `Popup::default_response_id` of the control (the menus `menu_popup` opens).
            let id = f.with("popup");
            if !egui::Popup::is_id_open(ctx, id) {
                return;
            }
            LayerId::new(Order::Foreground, id)
        }
    };
    if let Some(first) = first_in(ctx, layer) {
        ctx.memory_mut(|m| m.request_focus(first));
        nav.opener = Some((f, layer));
        ctx.request_repaint();
    }
}

/// The focus ring around the focused control (unless it showed its focus itself).
fn ring(ctx: &Context) {
    let Some(id) = ctx.memory(|m| m.focused()) else { return };
    let (own, canvas, custom) = shared(ctx, |s| (s.ring == Some(id), s.canvas == Some(id), s.ring_rect.filter(|r| r.0 == id)));
    if own {
        return;
    }
    let Some(w) = ctx.viewport(|v| v.this_pass.widgets.get(id).copied()) else { return };
    let painter = ctx.layer_painter(w.layer_id);
    let (rect, painter) = match custom {
        Some((_, rect, clip)) => (rect.expand(1.0), painter.with_clip_rect(clip)),
        None if canvas => (w.rect.shrink(3.0), painter),
        None => (w.interact_rect.expand(1.0), painter),
    };
    if rect.is_positive() {
        paint_ring(&painter, rect);
    }
}

fn paint_ring(painter: &egui::Painter, rect: Rect) {
    let t = Tokens::get(painter.ctx());
    painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(2.0, t.focus), StrokeKind::Outside);
}

/// The focus ring goes round `rect` (inside `ui`'s clip) while `resp` has the keyboard: a control
/// made of several widgets (a number field's label and value), a row whose visible part is inset.
pub fn ring_around(ui: &Ui, rect: Rect, resp: &Response) {
    if resp.has_focus() {
        let clip = ui.clip_rect().expand(4.0);
        shared(&resp.ctx, |s| s.ring_rect = Some((resp.id, rect, clip)));
    }
}

/// `resp` shows its focus itself (a highlighted menu row): no ring around it.
pub fn own_focus(resp: &Response) {
    if resp.has_focus() {
        shared(&resp.ctx, |s| s.ring = Some(resp.id));
    }
}

// ---------------------------------------------------------------------------------------------
// Names, roles and states.

fn bounds(r: Rect) -> accesskit::Rect {
    accesskit::Rect { x0: r.min.x.into(), y0: r.min.y.into(), x1: r.max.x.into(), y1: r.max.y.into() }
}

fn toggled(on: bool) -> Toggled {
    if on { Toggled::True } else { Toggled::False }
}

/// Write `resp`'s node: its bounds, `label`, then `fill`. Also remembers the focused control's name.
fn node(resp: &Response, label: &str, fill: impl FnOnce(&mut accesskit::Node)) {
    remember(resp, label);
    let r = resp.rect;
    resp.ctx.accesskit_node_builder(resp.id, |n| {
        n.set_bounds(bounds(r));
        if !label.is_empty() {
            n.set_label(label);
        }
        fill(n);
    });
}

/// Keep the focused control's name for `ui.inspect`.
fn remember(resp: &Response, name: &str) {
    if resp.has_focus() && !name.is_empty() {
        shared(&resp.ctx, |s| {
            if s.name != name {
                s.name.clear();
                s.name.push_str(name);
            }
        });
    }
}

/// A tooltip's name and key: "Rectangle  R" is the name "Rectangle" and the key "R" (tooltips
/// put two spaces before the key).
pub fn split_tip(tip: &str) -> (&str, Option<&str>) {
    match tip.split_once("  ") {
        Some((name, key)) if !key.trim().is_empty() => (name.trim_end(), Some(key.trim())),
        _ => (tip, None),
    }
}

/// A button named by its tooltip ("Align left  ⌥A": the name and the key).
pub fn button(resp: &Response, tip: &str) {
    let (name, key) = split_tip(tip);
    node(resp, name, |n| {
        n.set_role(Role::Button);
        if let Some(k) = key {
            n.set_keyboard_shortcut(k);
        }
    });
}

/// A button named by `label` as it is: no key is split off (a file name may hold two spaces).
pub fn named_button(resp: &Response, label: &str) {
    node(resp, label, |n| n.set_role(Role::Button));
}

/// A link, named by its text.
pub fn link(resp: &Response, label: &str) {
    node(resp, label, |n| n.set_role(Role::Link));
}

/// A button opening a menu or flyout, `expanded` while it is open.
pub fn menu_button(resp: &Response, label: &str, expanded: bool) {
    node(resp, label, |n| {
        n.set_role(Role::Button);
        n.set_has_popup(accesskit::HasPopup::Menu);
        n.set_expanded(expanded);
    });
}

/// A button that shows or hides what is under it, `expanded` while that shows.
pub fn disclosure(resp: &Response, label: &str, expanded: bool) {
    node(resp, label, |n| {
        n.set_role(Role::Button);
        n.set_expanded(expanded);
    });
}

/// A toggle button (a tool, a segment), pressed when `on`.
pub fn toggle(resp: &Response, tip: &str, on: bool) {
    let (name, key) = split_tip(tip);
    node(resp, name, |n| {
        n.set_role(Role::Button);
        n.set_toggled(toggled(on));
        if let Some(k) = key {
            n.set_keyboard_shortcut(k);
        }
    });
}

/// A status shown in place (a badge), with what it means.
pub fn status(resp: &Response, label: &str, description: &str) {
    node(resp, label, |n| {
        n.set_role(Role::Status);
        n.set_description(description);
    });
}

/// A tab, selected when it's the one shown.
pub fn tab(resp: &Response, label: &str, selected: bool) {
    node(resp, label, |n| {
        n.set_role(Role::Tab);
        n.set_selected(selected);
    });
}

pub fn checkbox(resp: &Response, label: &str, on: bool) {
    node(resp, label, |n| {
        n.set_role(Role::CheckBox);
        n.set_toggled(toggled(on));
    });
}

pub fn radio(resp: &Response, label: &str, on: bool) {
    node(resp, label, |n| {
        n.set_role(Role::RadioButton);
        n.set_toggled(toggled(on));
    });
}

/// A number field: its value (`suffix` after it), or `empty` ("Mixed", "Auto") without one.
pub fn number(resp: &Response, label: &str, value: Option<f64>, suffix: &str, empty: &str) {
    node(resp, label, |n| {
        n.set_role(Role::SpinButton);
        match value {
            Some(v) => {
                n.set_numeric_value(v);
                n.set_value(format!("{}{suffix}", crate::widgets::fmt_num(v)));
            }
            None => n.set_value(empty),
        }
    });
}

/// A text field showing `value`.
pub fn text(resp: &Response, label: &str, value: &str) {
    node(resp, label, |n| {
        n.set_role(Role::TextInput);
        n.set_value(value);
    });
}

/// A dropdown showing `value`, open when `expanded`.
pub fn combo(resp: &Response, label: &str, value: &str, expanded: bool) {
    // An unlabelled dropdown goes by what it shows.
    if label.is_empty() {
        remember(resp, value);
    }
    node(resp, label, |n| {
        n.set_role(Role::ComboBox);
        n.set_value(value);
        n.set_expanded(expanded);
    });
}

/// Whether the menu `resp` opens ([`crate::right::menu_popup`]) is open.
pub fn menu_open(resp: &Response) -> bool {
    egui::Popup::is_id_open(&resp.ctx, egui::Popup::default_response_id(resp))
}

/// A menu item, a checkable one when `checked` is set. `trail` is what the row shows at its right:
/// a key (`Shift+Cmd+K`, said the platform's way) or other text (a size), its description.
pub fn menu_item(resp: &Response, label: &str, trail: &str, checked: Option<bool>) {
    node(resp, label, |n| {
        match checked {
            Some(on) => {
                n.set_role(Role::MenuItemCheckBox);
                n.set_toggled(toggled(on));
            }
            None => n.set_role(Role::MenuItem),
        }
        if keymap::parse(trail).is_some() {
            n.set_keyboard_shortcut(keymap::display(trail, crate::shortcuts::platform()));
        } else if !trail.is_empty() {
            n.set_description(trail);
        }
    });
}

/// A menu item opening a submenu.
pub fn submenu(resp: &Response, label: &str) {
    node(resp, label, |n| {
        n.set_role(Role::MenuItem);
        n.set_has_popup(accesskit::HasPopup::Menu);
    });
}

/// `resp` can't be used now (a menu item whose command doesn't apply).
pub fn disabled(resp: &Response) {
    node(resp, "", |n| n.set_disabled());
}

/// What a layer row says besides its name.
pub struct RowInfo<'a> {
    /// The layer's type ("Frame", "Text").
    pub kind: &'a str,
    /// Nesting depth (0: on the page).
    pub depth: usize,
    pub selected: bool,
    /// Open or closed when the layer has children.
    pub expanded: Option<bool>,
    pub hidden: bool,
    pub locked: bool,
}

/// A layer row in the layers tree.
pub fn tree_item(resp: &Response, name: &str, info: &RowInfo) {
    node(resp, name, |n| {
        n.set_role(Role::TreeItem);
        n.set_level(info.depth + 1);
        n.set_selected(info.selected);
        if let Some(open) = info.expanded {
            n.set_expanded(open);
        }
        let mut d = info.kind.to_string();
        for (on, word) in [(info.hidden, tr("hidden")), (info.locked, tr("locked"))] {
            if on {
                d.push_str(", ");
                d.push_str(word);
            }
        }
        n.set_description(d);
    });
}

/// An item of a list with one current item (a page).
pub fn list_item(resp: &Response, label: &str, selected: bool) {
    node(resp, label, |n| {
        n.set_role(Role::ListBoxOption);
        n.set_selected(selected);
    });
}

/// A colour swatch showing `value` ("#FF0000", "Linear gradient").
pub fn color(resp: &Response, label: &str, value: &str) {
    node(resp, label, |n| {
        n.set_role(Role::ColorWell);
        n.set_value(value);
    });
}

/// Tooltips that also name the control for screen readers.
pub trait Tip {
    /// The tooltip, and for a clickable control its name (and key: "Align left  ⌥A").
    fn tip(self, tip: impl AsRef<str>) -> Self;
}

impl Tip for Response {
    fn tip(self, tip: impl AsRef<str>) -> Self {
        if self.sense.senses_click() {
            button(&self, tip.as_ref());
        }
        self.on_hover_ui(move |ui| {
            ui.label(tip.as_ref());
        })
    }
}
