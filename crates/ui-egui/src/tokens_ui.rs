//! Binding design tokens to properties, for files worked on with Design tokens (Preferences): the
//! variable buttons of the Design panel open this picker instead of the variable one. It lists the
//! tokens (resolved in the selection's contexts) whose type the property takes, by group; picking
//! one runs `token.bind`. Bound fields show the token's path where a variable's name would be.

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::tokens::{self as dt, TokenType};
use figcraft_doc::{Document, NodeId, ResolvedToken};
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::theme::{Tokens, regular, semibold};

/// The open token picker: the layers and the fields it binds (a token goes to the first field
/// that takes its type), its title, where it opens.
#[derive(Clone)]
struct Picker {
    ids: Vec<NodeId>,
    fields: Vec<String>,
    title: Option<&'static str>,
    at: Pos2,
}

fn picker_id() -> Id {
    Id::new("token_picker")
}

fn mode_id() -> Id {
    Id::new("design_tokens_mode")
}

/// Note this frame whether the interface works with design tokens (call once per frame), for
/// widgets that only have a `Ui` (tooltips of the bind buttons).
pub fn set_mode(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(mode_id(), on));
}

/// Whether the interface works with design tokens rather than variables.
pub fn on(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(mode_id())).unwrap_or(false)
}

/// Open the picker for `field` of `ids` at `at`.
pub fn open(ctx: &egui::Context, ids: &[NodeId], field: &str, at: Pos2) {
    let p = Picker { ids: ids.to_vec(), fields: vec![field.to_string()], title: None, at };
    ctx.data_mut(|d| d.insert_temp(picker_id(), p));
}

/// Open the picker for several fields of `ids` under the pointer (a style slot's tokens: colours
/// and gradients for a fill…), titled `title`.
pub fn open_slot(ui: &Ui, ids: &[NodeId], fields: Vec<String>, title: &'static str) {
    let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or_default();
    let p = Picker { ids: ids.to_vec(), fields, title: Some(title), at };
    ui.ctx().data_mut(|d| d.insert_temp(picker_id(), p));
}

/// Whether the picker is open.
pub fn picker_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Picker>(picker_id()).is_some())
}

/// Where the open picker is on screen.
pub fn picker_rect(ctx: &egui::Context) -> Option<Rect> {
    picker_open(ctx).then(|| ctx.memory(|m| m.area_rect(Id::new("token_picker_area")))).flatten()
}

/// The token `field` is bound to on every one of `ids`, if they agree.
pub fn bound_token(doc: &Document, ids: &[NodeId], field: &str) -> Option<String> {
    let mut it = ids.iter().map(|i| doc.node(*i).and_then(|n| n.token_bound.get(field)).cloned());
    let first = it.next().flatten()?;
    it.all(|v| v.as_ref() == Some(&first)).then_some(first)
}

/// A small preview of a resolved token in `r`: its colour, gradient or a type letter.
pub fn swatch(ui: &Ui, r: Rect, tok: &ResolvedToken) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    let paint = |c: figcraft_color::Color| figcraft_color::Paint::solid(c);
    match tok.kind {
        Some(TokenType::Color) => {
            if let Some(c) = dt::color_of(&tok.value) {
                crate::widgets::paint_swatch(p, r, &paint(c));
                return;
            }
        }
        Some(TokenType::Gradient) => {
            let stops: Vec<figcraft_color::ColorStop> = tok
                .value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| {
                    Some(figcraft_color::ColorStop {
                        color: dt::color_of(s.get("color")?)?,
                        position: s.get("position")?.as_f64()?.clamp(0.0, 1.0) as f32,
                    })
                })
                .collect();
            if stops.len() >= 2 {
                let mut g = figcraft_color::Gradient::default_for(Default::default(), figcraft_color::Color::BLACK);
                g.stops = stops;
                crate::widgets::paint_swatch(p, r, &figcraft_color::Paint::gradient(g));
                return;
            }
        }
        _ => {}
    }
    let letter = match tok.kind {
        Some(TokenType::Dimension) => "D",
        Some(TokenType::Number) => "#",
        Some(TokenType::FontFamily) => "F",
        Some(TokenType::FontWeight) => "W",
        Some(TokenType::Duration) => "ms",
        Some(TokenType::CubicBezier) => "∿",
        Some(TokenType::StrokeStyle) => "--",
        Some(TokenType::Border) => "▢",
        Some(TokenType::Shadow) => "◐",
        Some(TokenType::Typography) => "Aa",
        Some(TokenType::Transition) => "→",
        Some(TokenType::Color | TokenType::Gradient) | None => "?",
    };
    p.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, letter, regular(8.5), t.text_secondary);
}

/// One token in a list (`width` wide): its preview, last name and value; true when clicked.
pub fn token_row(ui: &mut Ui, tok: &ResolvedToken, width: f32, on: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    if on || resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    swatch(ui, Rect::from_center_size(pos2(r.min.x + 13.0, r.center().y), vec2(14.0, 14.0)), tok);
    let name = tok.path.rsplit('.').next().unwrap_or(&tok.path);
    let name_r = Rect::from_min_max(pos2(r.min.x + 26.0, r.min.y), pos2(r.max.x - 84.0, r.max.y));
    ui.painter().with_clip_rect(name_r).text(name_r.left_center(), Align2::LEFT_CENTER, name, regular(11.0), t.menu_text);
    let val_r = Rect::from_min_max(pos2(r.max.x - 82.0, r.min.y), pos2(r.max.x - 6.0, r.max.y));
    let shown = dt::display(tok.kind, &tok.value);
    ui.painter().with_clip_rect(val_r).text(val_r.right_center(), Align2::RIGHT_CENTER, &shown, regular(10.0), t.menu_dim);
    resp.on_hover_text(format!("{}\n{shown}", tok.path)).clicked()
}

/// The group a token path is in ("color.brand" for "color.brand.primary").
pub fn group_of(path: &str) -> &str {
    path.rsplit_once('.').map_or("", |(g, _)| g)
}

/// The picker popover (call once per frame).
pub fn picker(app: &mut FigApp, ctx: &egui::Context) {
    let Some(p) = ctx.data(|d| d.get_temp::<Picker>(picker_id())) else { return };
    let Ok(doc) = app.session.ds().map(|d| d.doc.clone()) else { return };
    let t = Tokens::get(ctx);
    let query_id = Id::new("token_picker_query");
    let mut query: String = ctx.data(|d| d.get_temp(query_id)).unwrap_or_default();
    let q = query.trim().to_lowercase();
    let first = p.ids.first().copied();
    // The field each type goes to: the first of the picker's fields that takes it.
    let field_for = |k: TokenType| -> Option<&String> {
        let id = first?;
        p.fields.iter().find(|f| figcraft_engine::cmd::tokens::field_types(&doc, id, f).contains(&k))
    };
    let input = first.map(|i| doc.token_input_for(i)).unwrap_or_default();
    let resolved = doc.tokens.resolve(&input);
    let current: Vec<String> = p.fields.iter().filter_map(|f| bound_token(&doc, &p.ids, f)).collect();
    let fits: Vec<&ResolvedToken> = resolved
        .tokens
        .values()
        .filter(|x| x.error.is_none() && x.kind.is_some_and(|k| field_for(k).is_some()))
        .filter(|x| q.is_empty() || x.path.to_lowercase().contains(&q))
        .collect();
    let takes_any = TokenType::ALL.iter().any(|k| field_for(*k).is_some());
    let mut chosen: Option<(String, String)> = None;
    let mut close = false;
    let area = egui::Area::new(Id::new("token_picker_area")).order(egui::Order::Foreground).fixed_pos(p.at + vec2(0.0, 4.0)).show(ctx, |ui| {
        egui::Frame::NONE.fill(t.menu).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(260.0);
            ui.label(egui::RichText::new(p.title.map_or(tr("Design tokens"), tr)).font(semibold(11.0)).color(t.menu_text));
            ui.add_space(4.0);
            let r = ui.add(egui::TextEdit::singleline(&mut query).hint_text(tr("Search")).font(regular(11.0)).desired_width(244.0));
            crate::widgets::select_all_on_focus(ui, &r, &query);
            if !r.has_focus() && !crate::a11y::field_focused(ui.ctx()) {
                r.request_focus();
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut group: Option<&str> = None;
                for tok in &fits {
                    let g = group_of(&tok.path);
                    if group != Some(g) {
                        group = Some(g);
                        if !g.is_empty() {
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(g.replace('.', " / ")).font(regular(10.0)).color(t.menu_dim));
                        }
                    }
                    if token_row(ui, tok, 244.0, current.contains(&tok.path))
                        && let Some(f) = tok.kind.and_then(field_for)
                    {
                        chosen = Some((f.clone(), tok.path.clone()));
                    }
                }
            });
            if fits.is_empty() {
                let msg = if !takes_any {
                    tr("No token type fits this property.")
                } else if resolved.tokens.is_empty() {
                    tr("No design tokens yet. Create them in Design tokens.")
                } else {
                    tr("No tokens fit here.")
                };
                ui.label(egui::RichText::new(msg).font(regular(11.0)).color(t.menu_dim));
            }
        });
    });
    let rect = area.response.rect;
    ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("token_picker_border"))).rect_stroke(
        rect,
        CornerRadius::same(10),
        Stroke::new(1.0, Color32::from_black_alpha(40)),
        egui::StrokeKind::Inside,
    );
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|pos| !rect.contains(pos)));
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || clicked_outside {
        close = true;
    }
    if let Some((field, path)) = chosen {
        // One token per slot: a colour replaces a gradient token on the same paint and back.
        for other in p.fields.iter().filter(|f| **f != field) {
            if bound_token(&doc, &p.ids, other).is_some() {
                app.run("token.bind", json!({"ids": p.ids, "field": other, "token": null}));
            }
        }
        app.run("token.bind", json!({"ids": p.ids, "field": field, "token": path}));
        close = true;
    }
    if close {
        ctx.data_mut(|d| {
            d.remove::<Picker>(picker_id());
            d.remove::<String>(query_id);
        });
    } else {
        ctx.data_mut(|d| d.insert_temp(query_id, query));
    }
}

/// The token binding key on paint `index`'s colour of `n` (`fills/<i>/color`, `fill`…), if any.
pub fn paint_key(n: &figcraft_doc::Node, fills: bool, index: usize) -> Option<&str> {
    use figcraft_engine::cmd::lists::{BoundList, list_binding};
    let list = if fills { BoundList::Fills } else { BoundList::Strokes };
    n.token_bound.keys().map(String::as_str).find(|k| list_binding(k) == Some((list, index, "color")))
}

/// The token contexts of the selected frames (sections, components, instances): a row per
/// modifier, Auto (the context the layer inherits) or a context of its own (`token.setContext`).
/// Returns whether it showed.
pub fn contexts_section(app: &mut FigApp, ui: &mut Ui, doc: &Document, sel: &[NodeId]) -> bool {
    use crate::right::{menu_check, menu_popup, row};
    if sel.is_empty() || !sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like())) || doc.tokens.modifiers.is_empty() {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    crate::widgets::section_header(ui, tr("Token contexts"), &[], false);
    let col = crate::widgets::col_width(ui.available_width());
    let (name_w, pick_w) = ((col * 0.7).round(), (col * 1.3).round());
    // Auto resolves from the parent of the first selected layer.
    let inherited = sel.first().and_then(|i| doc.node(*i)).and_then(|n| n.parent).map(|p| doc.token_input_for(p)).unwrap_or_default();
    for m in &doc.tokens.modifiers {
        let own = crate::right::common(doc, sel, |n| n.token_contexts.get(&m.name).cloned());
        let auto = crate::i18n::trf("Auto ({mode})", &[("mode", &inherited.get(&m.name).cloned().unwrap_or_default())]);
        let label = match &own {
            Some(Some(c)) => c.clone(),
            Some(None) => auto.clone(),
            None => tr("Mixed").to_string(),
        };
        row(ui, 32.0, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(name_w, crate::widgets::FIELD_H), Sense::hover());
            let name = crate::widgets::elide(ui, &m.name, name_w);
            ui.painter().text(pos2(rect.min.x, rect.center().y), Align2::LEFT_CENTER, &name, regular(11.0), t.text);
            let resp = crate::widgets::dropdown(ui, &label, pick_w, None);
            menu_popup(&resp).width(180.0).show(|ui| {
                let mut pick = None;
                if menu_check(ui, own == Some(None), &auto, "") {
                    pick = Some(serde_json::Value::Null);
                }
                ui.separator();
                for c in &m.contexts {
                    if menu_check(ui, own.as_ref().is_some_and(|o| o.as_deref() == Some(c.name.as_str())), &c.name, "") {
                        pick = Some(json!(c.name));
                    }
                }
                if let Some(c) = pick {
                    app.run("token.setContext", json!({"ids": sel, "modifier": m.name, "context": c}));
                    ui.close();
                }
            });
        });
    }
    ui.add_space(8.0);
    true
}
