//! The main menu (from the engine's menu catalog), the canvas/layer context menu and the
//! Actions palette (⌘K / ⌘/).

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::catalog::{MENU, SEP};
use serde_json::json;

use crate::FigApp;
use crate::a11y;
use crate::i18n::{self, tr, tr_noop, trf};
use crate::icons::{self, Icon};
use crate::theme::{ACCENT, Tokens, regular};
use crate::toolbar::pretty_shortcut;

/// Commands the UI layer implements itself (dialogs and platform actions).
pub const UI_COMMANDS: &[&str] = &[
    "plugin.import",
    "file.open",
    "file.import",
    "file.placeImage",
    "file.save",
    "file.saveAs",
    "file.export",
    "file.exportSvg",
    "edit.copyAsText",
    "edit.copyAsCss",
    "edit.copyAsSvg",
    "edit.copyAsPng",
    "edit.copyLink",
];

/// Run a menu item: some open dialogs or use platform services, the rest go to the engine.
/// Paste: the system clipboard wins when it holds something newer than our own copy (an image, SVG
/// or text from another app); otherwise our copied layers. `event_text` is the text the platform
/// handed over with the paste event (the browser has no other way to read it).
pub fn paste(app: &mut FigApp, event_text: Option<&str>) {
    let sys_text = app.services.clipboard_get_text().or_else(|| event_text.map(str::to_string)).filter(|t| !t.is_empty());
    let ours = sys_text.is_some() && sys_text == app.ui.last_copy;
    let (cx, cy) = app.session.ds().map(|d| (d.view.cx, d.view.cy)).unwrap_or((0.0, 0.0));
    if !ours {
        if let Some((png, w, h)) = clipboard_png(app) {
            use base64::Engine as _;
            let b64 = base64::engine::general_purpose::STANDARD.encode(png);
            // Centred in the view.
            let (x, y) = ((cx - f64::from(w) / 2.0).round(), (cy - f64::from(h) / 2.0).round());
            app.run("file.import", json!({"data": b64, "name": "Pasted image.png", "x": x, "y": y}));
            return;
        }
        // Text from elsewhere when it's newer than our copy, or when we have nothing to paste.
        if let Some(text) = sys_text.filter(|_| app.ui.last_copy.is_some() || app.session.clipboard.nodes.is_empty()) {
            // A comment's link opens its thread.
            if text.trim().starts_with("figcraft://comment/") {
                app.run("comment.open", json!({"link": text.trim()}));
            } else if figcraft_engine::looks_like_svg(&text) {
                use base64::Engine as _;
                let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
                app.run("file.import", json!({"data": b64, "name": "Pasted SVG.svg", "x": cx, "y": cy, "center": true}));
            } else {
                app.run("text.create", json!({"text": text, "x": cx, "y": cy}));
            }
            return;
        }
    }
    app.run("edit.paste", json!({}));
}

/// The system clipboard's image as PNG bytes, with its size.
fn clipboard_png(app: &mut FigApp) -> Option<(Vec<u8>, u32, u32)> {
    let (w, h, rgba) = app.services.clipboard_get_image()?;
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some((out.into_inner(), w, h))
}

pub fn invoke(app: &mut FigApp, id: &str) {
    match id {
        "file.open" => app.open_dialog(),
        "plugin.import" => crate::plugins_ui::import_dialog(app),
        "file.import" => app.import_dialog(false),
        "file.placeImage" => app.import_dialog(true),
        "file.save" => app.save(),
        "file.saveAs" => app.save_as_dialog(),
        "file.saveVersion" => crate::versions::start_save(app),
        "branch.create" => crate::versions::start_branch(app),
        "file.export" | "file.exportSvg" => app.ui.export_preview = true,
        "file.exportFramesPdf" => export_frames_pdf(app),
        "edit.copyAsCss" => {
            let ids = app.session.selection();
            if let (Some(first), Ok(doc)) = (ids.first(), app.session.doc()) {
                let css = figcraft_engine::codegen::code_for(doc, *first, figcraft_engine::codegen::CodeLang::Css);
                app.services.clipboard_set_text(&css);
                app.toast(tr("Copied as CSS"));
            }
        }
        "edit.copyAsSvg" => {
            let ids = app.session.selection();
            if let Some(svg) = app.session.doc().ok().and_then(|d| figcraft_svg_export(d, &ids)) {
                app.services.clipboard_set_text(&svg);
                app.toast(tr("Copied as SVG"));
            }
        }
        "edit.copyAsPng" => {
            let ids = app.session.selection();
            if let Some((mut pix, profile)) =
                app.session.doc().ok().and_then(|d| Some((figcraft_engine::cmd::file::copy_png(d, &ids)?, d.color_profile)))
            {
                // Clipboard images are read as sRGB.
                figcraft_render::to_srgb(&mut pix, profile);
                let rgba = figcraft_render::to_rgba8(&pix);
                app.services.clipboard_set_png(pix.width(), pix.height(), &rgba);
                app.toast(tr("Copied as PNG"));
            }
        }
        "edit.copyAsText" => {
            let doc = app.session.doc().ok();
            let text: Vec<String> = app
                .session
                .selection()
                .iter()
                .filter_map(|i| doc.and_then(|d| d.node(*i)).and_then(|n| n.kind.text().map(|t| t.characters.clone())))
                .collect();
            app.services.clipboard_set_text(&text.join("\n"));
            app.toast(tr("Copied as text"));
        }
        "edit.copyLink" => {
            let ids = app.session.selection();
            app.services.clipboard_set_text(&format!("figcraft://node/{}", ids.first().map(|i| i.0).unwrap_or(0)));
        }
        "edit.findReplace" => app.ui.find = Some((String::new(), String::new())),
        // From a menu the link is pasted into the Join dialog (`collab.join {link}` joins directly).
        "collab.join" => crate::live::open_join(app, ""),
        // So is the server address for publishing (it carries the server key).
        "collab.publish" => crate::live::open_publish(app),
        "edit.copy" | "edit.cut" => {
            let ids = app.session.selection();
            // SVG for other apps, taken before a cut removes the layers.
            let svg = app.session.doc().ok().and_then(|d| figcraft_svg_export(d, &ids));
            app.run(id, json!({}));
            // Mirror to the system clipboard (SVG text, else a marker) and remember it, so a later
            // paste knows whether the clipboard still holds our layers.
            let text = svg.unwrap_or_else(|| format!("figcraft://clipboard/{}", ids.len()));
            app.services.clipboard_set_text(&text);
            app.ui.last_copy = Some(text);
        }
        "edit.paste" => paste(app, None),
        "file.close" if app.session.ds().is_ok_and(|d| d.is_dirty() && d.branch_of.is_none()) => {
            let save = pretty_shortcut(crate::shortcuts::shortcut("file.save").unwrap_or(""));
            app.toast(trf(
                "Unsaved changes: save first ({save}), or close the tab with {alt}-click to discard",
                &[("save", &save), ("alt", &pretty_shortcut("Alt"))],
            ));
        }
        _ => {
            app.run(id, json!({}));
        }
    }
}

/// Every visible top-level frame of the page as one PDF page each, through a save dialog.
fn export_frames_pdf(app: &mut FigApp) {
    let name = app.session.doc().map(|d| d.name.clone()).unwrap_or_else(|_| "Untitled".into());
    app.pick_save(tr("Export"), &format!("{name}.pdf"), &["pdf"], |app, p| {
        if app.run("file.exportFramesPdf", json!({"path": p.to_string_lossy()})).is_some() {
            app.toast(trf("Exported {name}", &[("name", &p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default())]));
        }
    });
}

fn figcraft_svg_export(doc: &figcraft_doc::Document, ids: &[figcraft_doc::NodeId]) -> Option<String> {
    let bytes = figcraft_engine::cmd::file::export_bytes(doc, ids, figcraft_doc::ExportFormat::Svg, 1.0).ok()?;
    String::from_utf8(bytes).ok()
}

/// Whether a menu id is available (registered in the engine or handled by the UI).
pub fn available(id: &str) -> bool {
    !id.is_empty() && (figcraft_engine::find_command(id).is_some() || UI_COMMANDS.contains(&id))
}

/// Draw one level of the main menu: `prefix` is the menu path ("File", "Edit/Copy as").
fn menu_level(app: &mut FigApp, ui: &mut Ui, prefix: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    // The installed plugins head the Plugins menu.
    if prefix == "Plugins" && crate::plugins_ui::menu(app, ui, &t) {
        return true;
    }
    let mut clicked = None;
    let mut subs: Vec<String> = Vec::new();
    let mut last_sep = true;
    for m in MENU.iter() {
        if m.menu == prefix {
            if m.label == SEP {
                if !last_sep {
                    separator(ui);
                    last_sep = true;
                }
                continue;
            }
            last_sep = false;
            if m.id == "file.openRecent" {
                if recent_menu(app, ui, prefix, m.label, &i18n::menu_item(m.id, m.label), &t) {
                    return true;
                }
                continue;
            }
            // One window in the browser.
            let enabled = available(m.id) && !(m.id == "window.new" && ui.ctx().embed_viewports());
            let checked = figcraft_engine::cmd::view::checked(&app.session, m.id).unwrap_or(false);
            if item(ui, &i18n::menu_item(m.id, m.label), crate::shortcuts::shortcut(m.id).unwrap_or(""), enabled, checked, &t) && enabled {
                clicked = Some(m.id);
            }
        } else if let Some(rest) = m.menu.strip_prefix(prefix).and_then(|r| r.strip_prefix('/')) {
            let sub = rest.split('/').next().unwrap_or(rest).to_string();
            if !subs.contains(&sub) {
                subs.push(sub.clone());
                last_sep = false;
                let full = format!("{prefix}/{sub}");
                if submenu(ui, prefix, &full, tr(&sub), &t, |ui| menu_level(app, ui, &full).then_some(())).is_some() {
                    return true;
                }
            }
        }
    }
    if let Some(id) = clicked {
        invoke(app, id);
        return true;
    }
    false
}

/// File ▸ Open recent: the recent files (greyed when the file is gone) and Clear recent. `key` is
/// the menu's English label, `label` the one shown. Returns whether an item ran.
fn recent_menu(app: &mut FigApp, ui: &mut Ui, prefix: &str, key: &str, label: &str, t: &Tokens) -> bool {
    let picked = submenu(ui, prefix, &format!("{prefix}/{key}"), label, t, |ui| {
        let recents = &app.session.prefs.recent_files;
        let mut picked = None;
        for (i, p) in recents.iter().enumerate() {
            let path = std::path::Path::new(p);
            let name = path.file_name().map_or_else(|| p.clone(), |f| f.to_string_lossy().into_owned());
            let exists = path.exists();
            if item(ui, &name, "", exists, false, t) && exists {
                picked = Some(json!({"index": i}));
            }
        }
        if !recents.is_empty() {
            separator(ui);
        }
        if item(ui, tr("Clear recent"), "", !recents.is_empty(), false, t) && !recents.is_empty() {
            picked = Some(json!({"clear": true}));
        }
        picked
    });
    let Some(params) = picked else { return false };
    if app.run("file.openRecent", params).is_some_and(|r| r.get("index").is_some()) {
        app.ui.home = false;
    }
    true
}

/// A submenu row `label` in the menu at `prefix`; while it's open (hovered last), `add` draws its
/// items beside it. `key` is unique among the menu's submenus. Returns what `add` returned.
pub(crate) fn submenu<R>(ui: &mut Ui, prefix: &str, key: &str, label: &str, t: &Tokens, add: impl FnOnce(&mut Ui) -> Option<R>) -> Option<R> {
    let resp = submenu_item(ui, label, t);
    let slot = Id::new(("submenu_open", prefix));
    if resp.hovered() || resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(slot, key.to_string()));
    }
    let open: Option<String> = ui.data(|d| d.get_temp(slot));
    if open.as_deref() != Some(key) {
        return None;
    }
    let pos = pos2(resp.rect.max.x + 4.0, resp.rect.min.y - 8.0);
    egui::Area::new(Id::new(("submenu", key))).fixed_pos(pos).order(egui::Order::Foreground).show(ui.ctx(), |ui| menu_frame(ui, t, add)).inner
}

/// A thin line between groups of menu items.
pub(crate) fn separator(ui: &mut Ui) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.0, Color32::from_white_alpha(30)));
    ui.add_space(4.0);
}

fn menu_frame<R>(ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> R {
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
            add(ui)
        })
        .inner
}

/// A menu's narrowest width.
const MENU_W: f32 = 240.0;

/// Where a menu remembers the width its rows need (its frame's ui), per language.
fn width_key(ui: &Ui) -> Id {
    ui.id().with(("menu_w", i18n::current().map(|l| l.code)))
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
fn indented_item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool, checked: bool, indent: f32, t: &Tokens) -> bool {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    a11y::menu_item(&resp, label, shortcut, checked.then_some(true));
    if !enabled {
        a11y::disabled(&resp);
    }
    // The keyboard's item is highlighted like the hovered one.
    a11y::own_focus(&resp);
    let hov = (resp.hovered() || resp.has_focus()) && enabled;
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

fn submenu_item(ui: &mut Ui, label: &str, t: &Tokens) -> egui::Response {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    a11y::submenu(&resp, label);
    a11y::own_focus(&resp);
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(r, CornerRadius::same(5), t.menu_hover);
    }
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(12.0), t.menu_text);
    need_width(ui, 28.0 + galley.size().x + 28.0);
    ui.painter().galley(pos2(r.min.x + 28.0, r.center().y - galley.size().y / 2.0), galley, t.menu_text);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.max.x - 12.0, r.center().y), vec2(12.0, 12.0)), Icon::ChevronRight, t.menu_text);
    resp
}

/// Open the main menu at top-level menu `top` (control channel).
pub fn open_main_menu(ctx: &egui::Context, top: &str) {
    ctx.data_mut(|d| d.insert_temp(Id::new("main_open"), top.to_string()));
}

/// The main menu's area.
pub fn main_menu_id() -> Id {
    Id::new("main_menu")
}

/// Whether the pointer is over a menu (any foreground area).
fn over_menu(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.pointer.interact_pos()).and_then(|p| ctx.layer_id_at(p)).is_some_and(|l| l.order == egui::Order::Foreground)
}

const TOP_MENUS: &[&str] = &[
    tr_noop("File"),
    tr_noop("Edit"),
    tr_noop("View"),
    tr_noop("Object"),
    tr_noop("Text"),
    tr_noop("Arrange"),
    tr_noop("Vector"),
    tr_noop("Plugins"),
    tr_noop("Preferences"),
    tr_noop("Libraries"),
    tr_noop("Help"),
];

pub fn popups(app: &mut FigApp, ctx: &egui::Context) {
    let t = Tokens::get(ctx);
    // Main menu.
    if app.ui.main_menu {
        let mut close = false;
        let area = egui::Area::new(main_menu_id()).fixed_pos(pos2(8.0, crate::tabs::HEIGHT + 48.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            menu_frame(ui, &t, |ui| {
                // Quick actions search row.
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                a11y::menu_item(&resp, tr("Actions…"), crate::shortcuts::shortcut("app.actions").unwrap_or(""), None);
                ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(5), Color32::from_white_alpha(18));
                icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 16.0, r.center().y), vec2(14.0, 14.0)), Icon::Search, t.menu_dim);
                ui.painter().text(pos2(r.min.x + 32.0, r.center().y), Align2::LEFT_CENTER, tr("Actions…"), regular(12.0), t.menu_dim);
                ui.painter().text(
                    pos2(r.max.x - 10.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    pretty_shortcut(crate::shortcuts::shortcut("app.actions").unwrap_or("")),
                    regular(12.0),
                    t.menu_dim,
                );
                if resp.clicked() {
                    app.ui.palette = Some(String::new());
                    close = true;
                }
                ui.add_space(6.0);
                for top in TOP_MENUS {
                    let resp = submenu_item(ui, tr(top), &t);
                    if resp.hovered() || resp.has_focus() {
                        ui.data_mut(|d| d.insert_temp(Id::new("main_open"), top.to_string()));
                    }
                    let open: Option<String> = ui.data(|d| d.get_temp(Id::new("main_open")));
                    if open.as_deref() == Some(*top) {
                        let pos = pos2(resp.rect.max.x + 12.0, resp.rect.min.y - 8.0);
                        egui::Area::new(Id::new(("menu", *top))).fixed_pos(pos).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
                            menu_frame(ui, &t, |ui| {
                                if menu_level(app, ui, top) {
                                    close = true;
                                }
                            });
                        });
                    }
                }
            });
        });
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && !over_menu(ctx) || ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if close || (clicked_outside && !area.response.hovered()) {
            app.ui.main_menu = false;
        }
    }
    // Context menu (from the keyboard: at the focused layer row or the canvas selection, taking the
    // keyboard from the control with it).
    if std::mem::take(&mut app.ui.key_menu) {
        a11y::context_menu(app, ctx);
    }
    if let Some((pos, ids)) = app.ui.context_menu.clone() {
        if let Some(f) = ctx.memory(|m| m.focused()) {
            a11y::popup(&mut app.ui.nav, f, Id::new("ctx_menu"));
        }
        let mut close = false;
        let area = egui::Area::new(Id::new("ctx_menu")).fixed_pos(pos).order(egui::Order::Foreground).constrain(true).show(ctx, |ui| {
            menu_frame(ui, &t, |ui| {
                if select_layer_menu(app, ui, pos, &t) {
                    close = true;
                }
                let has_sel = !ids.is_empty();
                // Sections in the selection add a contents toggle naming what it will do.
                let section_toggle = app.session.doc().ok().and_then(|doc| {
                    let secs: Vec<bool> = ids
                        .iter()
                        .filter_map(|i| doc.node(*i))
                        .filter(|n| matches!(n.kind, figcraft_doc::NodeKind::Section { .. }))
                        .map(|n| n.hides_contents())
                        .collect();
                    (!secs.is_empty()).then(|| if secs.iter().all(|h| *h) { tr("Show section contents") } else { tr("Hide section contents") })
                });
                let mut entries: Vec<(&str, &str)> = if has_sel {
                    vec![
                        (tr("Copy"), "edit.copy"),
                        (tr("Paste here"), "edit.paste"),
                        (tr("Paste to replace"), "edit.pasteReplace"),
                        ("-", ""),
                        (tr("Bring to front"), "arrange.front"),
                        (tr("Bring forward"), "arrange.forward"),
                        (tr("Send backward"), "arrange.backward"),
                        (tr("Send to back"), "arrange.back"),
                        ("-", ""),
                        (tr("Group selection"), "object.group"),
                        (tr("Frame selection"), "object.frameSelection"),
                        (tr("Wrap in new section"), "object.wrapInSection"),
                        (tr("Ungroup"), "object.ungroup"),
                        (tr("Flatten"), "vector.flatten"),
                        (tr("Outline stroke"), "vector.outlineStroke"),
                        (tr("Use as mask"), "object.mask"),
                        ("-", ""),
                        (tr("Add auto layout"), "layout.add"),
                        (tr("Create component"), "component.create"),
                        (tr("Detach instance"), "component.detach"),
                        ("-", ""),
                        (tr("Show/Hide"), "object.toggleVisible"),
                        (tr("Lock/Unlock"), "object.toggleLock"),
                        ("-", ""),
                        (tr("Flip horizontal"), "object.flipH"),
                        (tr("Flip vertical"), "object.flipV"),
                        ("-", ""),
                        (tr("Copy as CSS"), "edit.copyAsCss"),
                        (tr("Copy as SVG"), "edit.copyAsSvg"),
                        (tr("Copy as PNG"), "edit.copyAsPng"),
                        ("-", ""),
                        (tr("Delete"), "edit.delete"),
                    ]
                } else {
                    vec![
                        (tr("Paste here"), "edit.paste"),
                        ("-", ""),
                        (tr("Select all"), "select.all"),
                        ("-", ""),
                        (tr("Show/Hide UI"), "view.toggleUi"),
                        (tr("Zoom to fit"), "view.zoomFit"),
                        (tr("Rulers"), "view.rulers"),
                        (tr("Pixel grid"), "view.pixelGrid"),
                    ]
                };
                if let Some(label) = section_toggle
                    && let Some(at) = entries.iter().position(|e| e.1 == "object.toggleLock")
                {
                    entries.insert(at + 1, (label, "section.toggleContents"));
                }
                // A right-clicked ruler guide (selected, with no layers) can be removed.
                if !has_sel && app.session.ds().is_ok_and(|d| d.selected_guide().is_some()) {
                    entries.splice(0..0, [(tr("Remove guide"), "guide.remove"), ("-", "")]);
                }
                for (label, id) in &entries {
                    if *label == "-" {
                        separator(ui);
                        continue;
                    }
                    let sc = crate::shortcuts::shortcut(id).unwrap_or("");
                    if item(ui, label, sc, available(id), false, &t) {
                        if *id == "edit.paste" {
                            let p = app.to_page(pos);
                            if app.session.clipboard.nodes.is_empty() {
                                invoke(app, id);
                            } else {
                                app.run("edit.paste", json!({"x": p.x, "y": p.y}));
                            }
                        } else {
                            invoke(app, id);
                        }
                        close = true;
                    }
                }
            });
        });
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && !area.response.hovered() && !over_menu(ctx);
        if close || clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            app.ui.context_menu = None;
            ctx.data_mut(|d| d.remove::<bool>(select_layer_id()));
        }
    }
    crate::colorpicker::show(app, ctx);
    palette(app, ctx);
}

fn select_layer_id() -> Id {
    Id::new("ctx_select_layer_open")
}

/// Open (or close) the context menu's Select layer submenu.
pub fn open_select_layer(ctx: &egui::Context, open: bool) {
    ctx.data_mut(|d| d.insert_temp(select_layer_id(), open));
}

/// The context menu's "Select layer" submenu (right-click on the canvas): every layer under the
/// click, nested ones indented, the selected ones checked. Returns whether one was picked.
fn select_layer_menu(app: &mut FigApp, ui: &mut Ui, pos: egui::Pos2, t: &Tokens) -> bool {
    if !app.canvas.rect.contains(pos) {
        return false;
    }
    let p = app.to_page(pos);
    let Ok(ds) = app.session.ds() else { return false };
    let tol = 2.0 / ds.view.zoom.max(1e-6);
    let layers: Vec<(figcraft_doc::NodeId, usize, String)> =
        figcraft_doc::geometry::layers_at(&ds.doc, ds.page, p, tol, Some(&figcraft_engine::booleans::result_outline(&ds.doc, &ds.selection)))
            .into_iter()
            .take(40)
            .filter_map(|(id, depth)| {
                ds.doc.node(id).map(|n| (id, depth, if n.name.is_empty() { n.kind.type_name().to_lowercase() } else { n.name.clone() }))
            })
            .collect();
    if layers.is_empty() {
        return false;
    }
    let sel = ds.selection.clone();
    let resp = submenu_item(ui, tr("Select layer"), t);
    if resp.hovered() {
        ui.data_mut(|d| d.insert_temp(select_layer_id(), true));
    }
    let mut picked = None;
    if ui.data(|d| d.get_temp::<bool>(select_layer_id())).unwrap_or(false) {
        let at = pos2(resp.rect.max.x + 4.0, resp.rect.min.y - 8.0);
        egui::Area::new(Id::new("ctx_select_layer")).fixed_pos(at).order(egui::Order::Foreground).constrain(true).show(ui.ctx(), |ui| {
            menu_frame(ui, t, |ui| {
                for (id, depth, name) in &layers {
                    if indented_item(ui, name, "", true, sel.contains(id), 12.0 * (*depth).min(8) as f32, t) {
                        picked = Some(*id);
                    }
                }
            });
        });
    }
    separator(ui);
    match picked {
        Some(id) => {
            app.run("select.set", json!({"ids": [id]}));
            true
        }
        None => false,
    }
}

/// The Actions palette: fuzzy search over every command.
fn palette(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut q) = app.ui.palette.clone() else { return };
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let mut run: Option<String> = None;
    let mut close = false;
    egui::Area::new(Id::new("palette")).fixed_pos(pos2(screen.center().x - 260.0, screen.min.y + 120.0)).order(egui::Order::Foreground).show(
        ctx,
        |ui| {
            egui::Frame::NONE
                .fill(t.panel)
                .corner_radius(CornerRadius::same(13))
                .stroke(Stroke::new(1.0, t.border))
                .shadow(egui::Shadow { offset: [0, 8], blur: 32, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(520.0);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut q)
                            .hint_text(tr("Search actions, layers and tools"))
                            .desired_width(500.0)
                            .font(regular(14.0))
                            .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(8))),
                    );
                    r.request_focus();
                    crate::widgets::select_all_on_focus(ui, &r, &q);
                    ui.add_space(4.0);
                    let ql = q.to_lowercase();
                    // Labels match in the UI language and in English.
                    let matches =
                        |label: &str, english: &str, id: &str| ql.is_empty() || [label, english, id].iter().any(|s| s.to_lowercase().contains(&ql));
                    let mut hits: Vec<(&str, std::borrow::Cow<str>, &str)> = Vec::new();
                    let menu_items = MENU.iter().filter(|m| UI_COMMANDS.contains(&m.id)).map(|m| (m.id, m.label));
                    for (id, english) in figcraft_engine::all_commands().iter().map(|c| (c.id, c.label)).chain(menu_items) {
                        let label = i18n::command(id, english);
                        if matches(&label, english, id) {
                            hits.push((id, label, crate::shortcuts::shortcut(id).unwrap_or("")));
                        }
                    }
                    hits.sort_by_key(|h| (!h.1.to_lowercase().starts_with(&ql), h.1.len()));
                    hits.dedup_by(|a, b| a.0 == b.0);
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for (i, (id, label, sc)) in hits.iter().take(60).enumerate() {
                            let w = ui.available_width();
                            let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                            if resp.hovered() || i == 0 {
                                ui.painter().rect_filled(r, CornerRadius::same(5), if resp.hovered() { ACCENT } else { t.hover });
                            }
                            let col = if resp.hovered() { Color32::WHITE } else { t.text };
                            ui.painter().text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, label.as_ref(), regular(12.0), col);
                            ui.painter().text(
                                pos2(r.max.x - 10.0, r.center().y),
                                Align2::RIGHT_CENTER,
                                pretty_shortcut(sc),
                                regular(11.0),
                                if resp.hovered() { Color32::WHITE } else { t.text_secondary },
                            );
                            if resp.clicked() || (enter && i == 0) {
                                run = Some(id.to_string());
                            }
                        }
                    });
                });
        },
    );
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if let Some(id) = run {
        app.ui.palette = None;
        invoke(app, &id);
        return;
    }
    app.ui.palette = if close { None } else { Some(q) };
    let _ = StrokeKind::Inside;
}
