use super::{BlendMode, Color32, CornerRadius, FIELD_H, FigApp, Id, NodeId, PAD, Rect, Sense, Tokens, Ui, a11y, json, pos2, regular, tr, vec2};

/// The smallest width a popup menu is measured in (see [`menu_popup`]).
const MENU_MIN_W: f32 = 48.0;

/// A menu row's inset either side, and the gap between its label and its shortcut or art.
const MENU_PAD: f32 = 8.0;

const MENU_GAP: f32 = 24.0;

/// The style of popup menus: egui's, with text kept on one line (a menu grows to its widest item).
pub(crate) fn menu_style(s: &mut egui::Style) {
    egui::containers::menu::menu_style(s);
    s.wrap_mode = Some(egui::TextWrapMode::Extend);
}

/// The blend modes as menu items in groups (pass-through only for layers); the one picked. Each
/// row `preview`s its mode on the canvas while hovered.
pub fn blend_menu(ui: &mut Ui, with_pass_through: bool, mut preview: impl FnMut(BlendMode)) -> Option<BlendMode> {
    let mut picked = None;
    for b in BlendMode::ALL.into_iter().filter(|b| with_pass_through || *b != BlendMode::PassThrough) {
        let resp = menu_row(ui, tr(b.label()), "", 0.0, |_, _, _| {});
        if resp.hovered() {
            preview(b);
        }
        if resp.clicked() {
            picked = Some(b);
            ui.close();
        }
        if matches!(b, BlendMode::Normal | BlendMode::ColorBurn | BlendMode::ColorDodge | BlendMode::HardLight | BlendMode::Exclusion) {
            ui.separator();
        }
    }
    picked
}

/// Show `blend` on `ids` on the canvas while its row in a blend mode menu is hovered.
pub(crate) fn preview_blend(app: &mut FigApp, ctx: &egui::Context, ids: &[NodeId], blend: BlendMode) {
    app.preview_text(ctx, "node.set", json!({"ids": ids, "props": {"blend": blend.label()}}));
}

/// The menu a dropdown or button opens: as wide as its widest row and at least as wide as
/// `anchor`. egui measures a popup in a first, invisible pass as wide as the width it is given
/// (else its 600 px default area width) and keeps the size measured; rows ([`menu_row`]) take the
/// larger of that width and their content, so the menu fits its content. Chain `.width(w)` for a
/// menu at least `w` wide.
pub fn menu_popup(anchor: &egui::Response) -> egui::Popup<'static> {
    egui::Popup::menu(anchor).width(anchor.rect.width().max(MENU_MIN_W)).style(menu_style)
}

/// A context menu (secondary click on `resp`) sized like [`menu_popup`].
pub fn context_popup(resp: &egui::Response) -> egui::Popup<'static> {
    egui::Popup::context_menu(resp).width(MENU_MIN_W).style(menu_style)
}

/// A context menu for a property row: a secondary click anywhere over `rect` opens it, even over
/// the row's fields (which would otherwise take the click), like the incumbent's row menus.
pub(super) fn row_context_menu(ui: &Ui, rect: Rect, id: Id) -> egui::Popup<'static> {
    let resp = ui.interact(rect, id, Sense::hover());
    let open = ui.rect_contains_pointer(rect) && ui.input(|i| i.pointer.secondary_clicked());
    let at_pointer = egui::Popup::menu(&resp).width(MENU_MIN_W).style(menu_style).at_pointer_fixed();
    at_pointer.open_memory(open.then_some(egui::SetOpenCommand::Bool(true)))
}

pub fn menu_item(ui: &mut Ui, label: &str, shortcut: &str) -> bool {
    menu_row(ui, label, shortcut, 0.0, |_, _, _| {}).clicked()
}

/// A menu row with art drawn into it after the label (`art(painter, row rect, text colour)`),
/// taking `art_w` from the row's right edge.
pub fn menu_item_with(ui: &mut Ui, label: &str, shortcut: &str, art_w: f32, art: impl FnOnce(&egui::Painter, Rect, Color32)) -> bool {
    menu_row(ui, label, shortcut, art_w, art).clicked()
}

/// The width a menu row needs for a label `label_w` wide and, at its right end, a shortcut or
/// art `trail` wide (0 for neither).
pub(crate) fn menu_row_width(label_w: f32, trail: f32) -> f32 {
    MENU_PAD + label_w + if trail > 0.0 { MENU_GAP + trail } else { MENU_PAD }
}

/// A menu row (see [`menu_item_with`]); its response tells hovering too. It spans the menu and
/// is at least as wide as its label, shortcut and art.
pub fn menu_row(ui: &mut Ui, label: &str, shortcut: &str, art_w: f32, art: impl FnOnce(&egui::Painter, Rect, Color32)) -> egui::Response {
    lit_row(ui, label, shortcut, art_w, art, false)
}

/// A plain menu row highlighted when `lit`, as if hovered: the row Up and Down are on in a menu
/// whose search field keeps the keyboard.
pub(crate) fn menu_row_lit(ui: &mut Ui, label: &str, lit: bool) -> egui::Response {
    lit_row(ui, label, "", 0.0, |_, _, _| {}, lit)
}

fn lit_row(ui: &mut Ui, label: &str, shortcut: &str, art_w: f32, art: impl FnOnce(&egui::Painter, Rect, Color32), lit: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let text = ui.painter().layout_no_wrap(label.to_owned(), regular(11.5), Color32::PLACEHOLDER);
    let keys =
        (!shortcut.is_empty()).then(|| ui.painter().layout_no_wrap(crate::toolbar::pretty_shortcut(shortcut), regular(11.0), Color32::PLACEHOLDER));
    let trail = keys.as_ref().map_or(0.0, |k| k.size().x + MENU_PAD).max(art_w);
    let w = ui.available_width().max(menu_row_width(text.size().x, trail).ceil());
    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    a11y::menu_item(&resp, label, shortcut, None);
    // The keyboard's item is highlighted like the hovered one.
    a11y::own_focus(&resp);
    let hovered = lit || resp.hovered() || resp.has_focus();
    if hovered {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    // These menus sit on the theme's popup fill (white in the light theme), not on the dark
    // `t.menu` of the menu bar: the theme's text, white only on the green highlight (#972).
    let col = if hovered { t.menu_text } else { t.text };
    ui.painter().galley(pos2(r.min.x + MENU_PAD, r.center().y - text.size().y / 2.0), text, col);
    if let Some(k) = keys {
        let at = pos2(r.max.x - MENU_PAD - k.size().x, r.center().y - k.size().y / 2.0);
        ui.painter().galley(at, k, if hovered { t.menu_text } else { t.text_secondary });
    }
    art(ui.painter(), r, col);
    resp
}

/// A menu item with a check mark when `on` (the unchecked ones line up with it).
pub fn menu_check(ui: &mut Ui, on: bool, label: &str, shortcut: &str) -> bool {
    marked_row(ui, Some(on), label, shortcut)
}

/// A plain menu item lined up with the check items above or below it.
pub fn menu_item_beside_checks(ui: &mut Ui, label: &str, shortcut: &str) -> bool {
    marked_row(ui, None, label, shortcut)
}

fn marked_row(ui: &mut Ui, on: Option<bool>, label: &str, shortcut: &str) -> bool {
    let resp = menu_row(ui, &format!("{}{label}", if on == Some(true) { "✓ " } else { "    " }), shortcut, 0.0, |_, _, _| {});
    a11y::menu_item(&resp, label, shortcut, on);
    resp.clicked()
}

pub(crate) fn row(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui)) {
    row_rect(ui, h, add);
}

/// [`row`], returning the area the row takes.
pub(crate) fn row_rect(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui)) -> Rect {
    block(ui, h, (h - FIELD_H) / 2.0, add)
}

/// A panel row `h` tall whose content is inset `inset_y` from its top and bottom (for content
/// taller than one field); returns the area the row takes.
pub(crate) fn block(ui: &mut Ui, h: f32, inset_y: f32, add: impl FnOnce(&mut Ui)) -> Rect {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    // The section's grid: 16 in from the left, 8 from the right (not 16 on both sides).
    let inner = Rect::from_min_max(pos2(r.min.x + PAD, r.min.y + inset_y), pos2(r.max.x - crate::widgets::PAD_RIGHT, r.max.y - inset_y));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing = vec2(crate::widgets::GUTTER, 0.0);
    add(&mut child);
    r
}
