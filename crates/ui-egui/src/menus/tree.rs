use super::{Color32, CornerRadius, Icon, Id, Rect, Sense, Stroke, Tokens, Ui, a11y, i18n, icons, pos2, pretty_shortcut, regular, vec2};

/// A submenu row `label` in the menu being drawn; while it's open, `add` draws its items beside
/// it. `key` names the submenu (its menu path, unique in its menu tree). Returns what `add`
/// returned.
pub(crate) fn submenu<R>(ui: &mut Ui, key: &str, label: &str, t: &Tokens, add: impl FnOnce(&mut Ui) -> Option<R>) -> Option<R> {
    submenu_at(ui, SubmenuOptions { area: Id::unique(("submenu", key)), key, label, gap: 4.0, constrain: false, tokens: t }, add)
}

// ---------------------------------------------------------------------------------------------
// Which submenu is open: hover intent with a safe triangle.
//
// Every menu tree (the main menu, the context menu) keeps, per level, which of its submenus is
// open. Hovering another row closes the open submenu at once; resting on a submenu row for
// `HOVER_DELAY` opens it (sweeping across rows doesn't flash every submenu on the way). Except while the
// pointer is on its way from the open submenu's row to the submenu: while each move lands inside
// the triangle between where it was and the submenu's near edge (it keeps getting nearer), rows
// it crosses don't take over and the submenu's row stays highlighted. Resting (`AIM_REST`) or
// turning away ends that, and the row under the pointer takes over. Leaving the menus keeps the
// last submenu open.
// Closing a menu tree forgets all of it, so it reopens with no submenu open.

/// How long the pointer stays on a submenu row before its submenu opens.
pub(crate) const HOVER_DELAY: f64 = 0.15;

/// How long the pointer may rest inside the safe triangle before the row under it takes over.
pub(crate) const AIM_REST: f64 = 0.3;

/// A pointer that moved less than this is resting.
const AIM_STILL: f32 = 1.0;

/// The triangle reaches this far above and below the submenu.
const AIM_EDGE_SLOP: f32 = 8.0;

/// The hover state of one menu level.
#[derive(Clone, Default, Debug)]
struct Level {
    /// The open submenu's key.
    open: Option<String>,
    /// The open submenu's row and where its panel was drawn (last frame).
    row: Option<Rect>,
    panel: Option<Rect>,
    /// The pointer's last position counted as heading for the panel, and when it got there.
    apex: Option<egui::Pos2>,
    moved_at: f64,
    /// This pass's verdict: the pointer is aiming at the panel.
    pass: Option<u64>,
    aiming: bool,
    /// The row the pointer is on, waiting to take over (a submenu's key, or `None`: a plain row),
    /// and since when; whether a row asked this pass (unasked, it lapses).
    pending: Option<(Option<String>, f64)>,
    asked: bool,
    /// When the pointer came to rest in the triangle, if that's what ended aiming this pass.
    rested_at: Option<f64>,
}

/// One menu tree's levels, by menu path ("" for the top).
#[derive(Clone, Default, Debug)]
struct Tree {
    levels: std::collections::HashMap<String, Level>,
}

/// The menu level a menu frame's ui draws: its tree's root and its path.
#[derive(Clone, Debug)]
pub(super) struct At {
    pub(super) root: Id,
    pub(super) path: String,
}

/// Scoped egui id for the current menu level.
fn at_key(ui: &Ui) -> Id {
    ui.scope_id().with("menu_level")
}

/// Return the current menu root/path, defaulting to the main menu root.
fn level_at(ui: &Ui) -> At {
    ui.data(|d| d.get_temp::<At>(at_key(ui))).unwrap_or(At { root: Id::unique("menus"), path: String::new() })
}

fn with_level<R>(ctx: &egui::Context, at: &At, f: impl FnOnce(&mut Level) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Tree>(at.root).levels.entry(at.path.clone()).or_default()))
}

/// Forget everything a menu tree remembers (it closed or opened anew).
pub(crate) fn forget_tree(ctx: &egui::Context, root: Id) {
    ctx.data_mut(|d| d.remove::<Tree>(root));
}

/// Open (`Some(key)`) or close (`None`) a submenu of `root`'s level `path`; the levels below the
/// one it replaces are forgotten (they reopen closed).
pub(super) fn set_open(ctx: &egui::Context, root: Id, path: &str, key: Option<&str>) {
    ctx.data_mut(|d| {
        let tree = d.get_temp_mut_or_default::<Tree>(root);
        let level = tree.levels.entry(path.to_string()).or_default();
        if level.open.as_deref() == key {
            return;
        }
        let old = std::mem::replace(&mut level.open, key.map(str::to_string));
        level.row = None;
        level.panel = None;
        level.apex = None;
        level.pending = None;
        if let Some(old) = old {
            let below = format!("{old}/");
            tree.levels.retain(|p, _| *p != old && !p.starts_with(&below));
        }
    });
}

/// Whether `p` is in the triangle `a`, `b`, `c` (either winding, edges included).
pub(crate) fn in_triangle(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> bool {
    let side = |u: egui::Pos2, v: egui::Pos2| (v.x - u.x) * (p.y - u.y) - (v.y - u.y) * (p.x - u.x);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Whether a pointer that moved from `from` to `p` is heading for `panel`: `p` is inside the
/// triangle from `from` to the panel's near edge, so it got nearer the panel without leaving the
/// rows' way to it. Straight up or down (or away) isn't.
pub(crate) fn heading_for(from: egui::Pos2, p: egui::Pos2, panel: Rect) -> bool {
    if ![from.x, from.y, p.x, p.y].iter().all(|v| v.is_finite()) {
        return false;
    }
    let edge = if panel.min.x >= from.x {
        panel.min.x
    } else if panel.max.x <= from.x {
        panel.max.x
    } else {
        return false;
    };
    in_triangle(p, from, pos2(edge, panel.min.y - AIM_EDGE_SLOP), pos2(edge, panel.max.y + AIM_EDGE_SLOP))
}

/// Whether the pointer is on its way to this level's open submenu (rows it crosses don't take
/// over). Worked out once per pass per level.
fn aiming(ui: &Ui) -> bool {
    let at = level_at(ui);
    let ctx = ui.ctx();
    let pass = ctx.cumulative_pass_nr();
    let (p, now) = ctx.input(|i| (i.pointer.hover_pos(), i.time));
    let (aiming, wait) = with_level(ctx, &at, |l| {
        if l.pass == Some(pass) {
            return (l.aiming, None);
        }
        l.pass = Some(pass);
        l.aiming = false;
        l.rested_at = None;
        if !l.asked {
            l.pending = None;
        }
        l.asked = false;
        let (Some(p), Some(_), Some(panel)) = (p, l.open.as_ref(), l.panel) else {
            l.apex = None;
            return (false, None);
        };
        if panel.contains(p) {
            l.apex = None;
            return (false, None);
        }
        // On the row (its bottom edge is the next row's).
        if l.row.is_some_and(|r| (r.min.x..r.max.x).contains(&p.x) && (r.min.y..r.max.y).contains(&p.y)) {
            l.apex = Some(p);
            l.moved_at = now;
            return (false, None);
        }
        let Some(from) = l.apex else { return (false, None) };
        if (p - from).length() <= AIM_STILL {
            // Resting: the submenu holds a moment longer.
            if !(0.0..AIM_REST).contains(&(now - l.moved_at)) {
                l.apex = None;
                // It has rested long enough: the row under it needn't wait again.
                l.rested_at = Some(l.moved_at);
                return (false, None);
            }
        } else if heading_for(from, p, panel) {
            l.apex = Some(p);
            l.moved_at = now;
        } else {
            l.apex = None;
            return (false, None);
        }
        l.aiming = true;
        (true, Some(AIM_REST - (now - l.moved_at)))
    });
    // Wake up when the rest runs out, so the row under a still pointer takes over.
    if let Some(w) = wait.filter(|w| w.is_finite() && *w > 0.0) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(w));
    }
    aiming
}

/// The pointer is on a row of level `at` (`target`: the row's submenu, or `None` for a plain
/// row): another open submenu closes at once, and `target` opens once the pointer has stayed
/// there `HOVER_DELAY`.
fn hover_request(ctx: &egui::Context, at: &At, target: Option<&str>) {
    let now = ctx.input(|i| i.time);
    if with_level(ctx, at, |l| l.open.is_some() && l.open.as_deref() != target) {
        set_open(ctx, at.root, &at.path, None);
    }
    let left = with_level(ctx, at, |l| {
        l.asked = true;
        if l.open.as_deref() == target {
            l.pending = None;
            return None;
        }
        let since = match &l.pending {
            Some((t, since)) if t.as_deref() == target => *since,
            _ => {
                let since = l.rested_at.unwrap_or(now);
                l.pending = Some((target.map(str::to_string), since));
                since
            }
        };
        Some(HOVER_DELAY - (now - since))
    });
    match left {
        Some(left) if left <= 0.0 => set_open(ctx, at.root, &at.path, target),
        // Wake up when it's due, so a still pointer gets there.
        Some(left) => ctx.request_repaint_after(std::time::Duration::from_secs_f64(left.min(HOVER_DELAY))),
        None => {}
    }
}

/// Whether the pointer is on a row of level `at` other than the open submenu's (it shows as
/// hovered instead).
fn pending_elsewhere(ctx: &egui::Context, at: &At) -> bool {
    with_level(ctx, at, |l| l.pending.as_ref().is_some_and(|(t, _)| *t != l.open))
}

/// A plain row of the level being drawn: hovering it (not while aiming at a submenu) closes the
/// level's open submenu. Returns whether it shows as hovered.
pub(super) fn plain_row_hover(ui: &Ui, resp: &egui::Response) -> bool {
    let hovered = resp.hovered() && !aiming(ui);
    if hovered {
        hover_request(ui.ctx(), &level_at(ui), None);
    }
    hovered || resp.has_focus()
}

/// A submenu row whose panel (area `area`, `gap` right of the row) shows while it's open.
pub(super) struct SubmenuOptions<'a> {
    pub(super) area: Id,
    pub(super) key: &'a str,
    pub(super) label: &'a str,
    pub(super) gap: f32,
    pub(super) constrain: bool,
    pub(super) tokens: &'a Tokens,
}

pub(super) fn submenu_at<R>(ui: &mut Ui, options: SubmenuOptions<'_>, add: impl FnOnce(&mut Ui) -> Option<R>) -> Option<R> {
    let SubmenuOptions { area, key, label, gap, constrain, tokens: t } = options;
    let at = level_at(ui);
    let aim = aiming(ui);
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    // The keyboard and clicks open it at once; hovering after a moment.
    if resp.has_focus() || resp.clicked() {
        set_open(ui.ctx(), at.root, &at.path, Some(key));
    } else if resp.hovered() && !aim {
        hover_request(ui.ctx(), &at, Some(key));
    }
    let open = with_level(ui.ctx(), &at, |l| {
        let open = l.open.as_deref() == Some(key);
        if open {
            l.row = Some(r);
        }
        open
    });
    a11y::submenu(&resp, label, open);
    a11y::own_focus(&resp);
    // The open submenu's row stays highlighted while the pointer is in the submenu (not while it
    // rests on another row of this menu).
    let keep = open && !pending_elsewhere(ui.ctx(), &at);
    paint_submenu_row(ui, r, label, keep || (resp.hovered() && !aim) || resp.has_focus(), t);
    if !open {
        return None;
    }
    let mut pos = pos2(r.max.x + gap, r.min.y - 8.0);
    // No room beside the row (a phone): the submenu covers its parent instead of running off screen.
    let screen = ui.ctx().content_rect();
    let cramped = pos.x + MENU_W > screen.max.x;
    if cramped {
        pos.x = (screen.max.x - MENU_W - 8.0).max(screen.min.x + 4.0);
    }
    let child = At { root: at.root, path: key.to_string() };
    let shown = egui::Area::new(area)
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .constrain(constrain || cramped)
        .show(ui.ctx(), |ui| menu_frame(ui, t, child, add));
    let panel = shown.response.rect;
    with_level(ui.ctx(), &at, |l| l.panel = Some(panel));
    shown.inner
}

/// A thin line between groups of menu items.
pub(crate) fn separator(ui: &mut Ui) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.0, Color32::from_white_alpha(30)));
    ui.add_space(4.0);
}

/// A menu panel drawing menu level `at`.
pub(super) fn menu_frame<R>(ui: &mut Ui, t: &Tokens, at: At, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(t.menu)
        .corner_radius(CornerRadius::same(13))
        .inner_margin(egui::Margin::same(8))
        .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(60) })
        .show(ui, |ui| {
            // As wide as its widest row asked for (labels run longer in some languages).
            let w = ui.data(|d| d.get_temp::<f32>(width_key(ui))).unwrap_or(0.0).max(MENU_W);
            ui.set_width(w);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let key = at_key(ui);
            ui.data_mut(|d| d.insert_temp(key, at));
            add(ui)
        })
        .inner
}

/// A menu's narrowest width.
const MENU_W: f32 = 240.0;

/// Where a menu remembers the width its rows need (its frame's ui), per language.
fn width_key(ui: &Ui) -> Id {
    ui.scope_id().with(("menu_w", i18n::current().map(|l| l.code)))
}

/// A row needs `w`: the menu widens to it from the next frame on.
fn need_width(ui: &Ui, w: f32) {
    let key = width_key(ui);
    if ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(0.0) < w {
        ui.data_mut(|d| d.insert_temp(key, w));
        ui.ctx().request_repaint();
    }
}

pub(crate) fn item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool, checked: bool, t: &Tokens) -> bool {
    indented_item(ui, label, shortcut, enabled, checked, 0.0, t)
}

/// A menu row whose label starts `indent` pixels further in (nested layers).
pub(super) fn indented_item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool, checked: bool, indent: f32, t: &Tokens) -> bool {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    a11y::menu_item(&resp, label, shortcut, checked.then_some(true));
    if !enabled {
        a11y::disabled(&resp);
    }
    // The keyboard's item is highlighted like the hovered one.
    a11y::own_focus(&resp);
    let hov = plain_row_hover(ui, &resp) && enabled;
    if hov {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    let col = if enabled { t.menu_text } else { t.menu_dim };
    if checked {
        icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(12.0, 12.0)), Icon::Check, col);
    }
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(12.0), col);
    let keys = (!shortcut.is_empty())
        .then(|| ui.painter().layout_no_wrap(pretty_shortcut(shortcut), regular(12.0), if hov { t.menu_text } else { t.menu_dim }));
    need_width(ui, 28.0 + indent + galley.size().x + keys.as_ref().map_or(8.0, |k| 16.0 + k.size().x + 10.0));
    let text_rect = Rect::from_min_max(pos2(r.min.x + 28.0 + indent, r.min.y), pos2(r.max.x - 8.0, r.max.y));
    ui.painter().with_clip_rect(text_rect).galley(pos2(text_rect.min.x, r.center().y - galley.size().y / 2.0), galley, col);
    if let Some(k) = keys {
        ui.painter().galley(pos2(r.max.x - 10.0 - k.size().x, r.center().y - k.size().y / 2.0), k, col);
    }
    resp.clicked()
}

fn paint_submenu_row(ui: &mut Ui, r: Rect, label: &str, highlight: bool, t: &Tokens) {
    if highlight {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(12.0), t.menu_text);
    need_width(ui, 28.0 + galley.size().x + 28.0);
    ui.painter().galley(pos2(r.min.x + 28.0, r.center().y - galley.size().y / 2.0), galley, t.menu_text);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 12.0, r.center().y), vec2(12.0, 12.0)), Icon::ChevronRight, t.menu_text);
}
