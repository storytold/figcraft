//! The main menu (from the engine's menu catalog), the canvas/layer context menu and the
//! Actions palette (⌘K / ⌘/).

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_engine::catalog::{MENU, SEP};
use serde_json::json;

use crate::FigApp;
use crate::a11y;
use crate::i18n::{self, tr, tr_noop, trf, trn};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular};
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
/// Paste: the system clipboard wins when it holds something newer than our own copy (layers copied
/// from another design app, an image, SVG or text from another app); otherwise our copied layers.
/// `event_text` is the text the platform handed over with the paste event (the browser has no
/// other way to read it). `at`: the page point to paste at (Paste here); `None` pastes in the
/// middle of the view.
pub fn paste(app: &mut FigApp, event_text: Option<&str>, at: Option<(f64, f64)>) {
    // Only HTML that holds readable design data takes this path; other HTML pastes as before.
    if let Some(html) = app.services.clipboard_get_html().filter(|h| figcraft_engine::cmd::edit::design_clipboard_buffer(h).is_some()) {
        let mut params = json!({"html": html});
        if let Some((x, y)) = at {
            params["x"] = json!(x);
            params["y"] = json!(y);
        }
        if let Some(r) = app.run("edit.pasteDesignLayers", params) {
            // The pasted layers are now our copy: text another app puts on the clipboard later is newer.
            app.ui.last_copy = Some(app.services.clipboard_get_text().unwrap_or_default());
            let left_out = r["warnings"].as_array().map_or(0, Vec::len);
            if left_out > 0 {
                app.toast(trn("Pasted design layers; {n} kinds of content were left out", left_out, &[]));
            }
        }
        return;
    }
    let sys_text = app.services.clipboard_get_text().or_else(|| event_text.map(str::to_string)).filter(|t| !t.is_empty());
    let ours = sys_text.is_some() && sys_text == app.ui.last_copy;
    let (cx, cy) = at.or_else(|| app.session.ds().ok().map(|d| (d.view.cx, d.view.cy))).unwrap_or((0.0, 0.0));
    if !ours {
        if let Some((png, w, h)) = clipboard_png(app) {
            use base64::Engine as _;
            let b64 = base64::engine::general_purpose::STANDARD.encode(png);
            // Centred in the view; Paste here puts its top-left at the point, as it does layers.
            let (x, y) = if at.is_some() { (cx, cy) } else { ((cx - f64::from(w) / 2.0).round(), (cy - f64::from(h) / 2.0).round()) };
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
                app.run_user("file.import", json!({"data": b64, "name": "Pasted SVG.svg", "x": cx, "y": cy, "center": at.is_none()}));
            } else {
                app.run("text.create", json!({"text": text, "x": cx, "y": cy}));
            }
            return;
        }
    }
    app.run("edit.paste", at.map_or(json!({}), |(x, y)| json!({"x": x, "y": y})));
}

/// The system clipboard's image as PNG bytes, with its size.
fn clipboard_png(app: &mut FigApp) -> Option<(Vec<u8>, u32, u32)> {
    let (w, h, rgba) = app.services.clipboard_get_image()?;
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some((out.into_inner(), w, h))
}

/// The Actions palette's tab.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum PaletteTab {
    #[default]
    All,
    Assets,
    Plugins,
}

/// Remember an action run from the palette for its Recents (newest first, capped).
pub fn record_recent(app: &mut FigApp, key: &str) {
    let r = &mut app.session.prefs.recent_actions;
    if r.first().is_some_and(|f| f == key) {
        return;
    }
    r.retain(|k| k != key);
    r.insert(0, key.to_string());
    r.truncate(figcraft_engine::MAX_RECENT_ACTIONS);
}

pub fn invoke(app: &mut FigApp, id: &str) {
    match id {
        "file.open" => app.open_dialog(),
        "plugin.import" => crate::plugins_ui::import_dialog(app),
        "file.import" => app.import_dialog(false),
        "file.placeImage" => app.import_dialog(true),
        "file.save" => app.save(),
        "file.saveAs" => app.save_as_dialog(),
        "file.openRecent" => {
            app.ui.home = true;
            app.ui.home_view.section = crate::home::Section::Recents;
        }
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
        // The Actions palette invokes commands without parameters. These values are edited in
        // Preferences; sending an empty object would either fail or silently reset a preference.
        "pref.language" | "pref.userName" | "pref.publishServer" | "pref.nudge" => {
            app.run("app.preferences", json!({}));
        }
        // Downloading needs the user's say-so: the Missing fonts window asks for it.
        "text.downloadFonts" => {
            app.run("text.missingFonts", json!({}));
        }
        // The file name in the left panel's header turns into its editor, the name selected.
        "file.rename" => {
            app.ui.left_tab = crate::LeftTab::File;
            app.ui.left_collapsed = false;
            app.ui.rename_file = true;
        }
        "text.link" => app.ui.menu_input = Some(crate::dialogs::MenuInput::Link(String::new())),
        "vector.offset" => app.ui.menu_input = Some(crate::dialogs::MenuInput::Offset(0.0)),
        "object.moveToPage" => app.ui.menu_input = Some(crate::dialogs::MenuInput::MoveToPage),
        "object.batchRename" => {
            let ids = app.session.selection();
            if !ids.is_empty() {
                crate::dialogs::open_batch_rename(app, &ids);
            }
        }
        "tool.set" => app.ui.tool_flyout = Some("move"),
        "page.select" => {
            app.ui.left_tab = crate::LeftTab::File;
            app.ui.left_collapsed = false;
            app.ui.pages_open = true;
        }
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
        "edit.paste" => paste(app, None, None),
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
                if submenu(ui, &full, tr(&sub), &t, |ui| menu_level(app, ui, &full).then_some(())).is_some() {
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
    let picked = submenu(ui, &format!("{prefix}/{key}"), label, t, |ui| {
        let recents: Vec<&String> = crate::home_recents::listed(&app.session.prefs.recent_files).collect();
        let mut picked = None;
        for p in recents.iter().copied() {
            let path = std::path::Path::new(p);
            let name = path.file_name().map_or_else(|| p.clone(), |f| f.to_string_lossy().into_owned());
            let exists = path.exists();
            if item(ui, &name, "", exists, false, t) && exists {
                picked = Some(json!({"path": p}));
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
    // Through the user path, so an import report shows; `file.openRecent` also drops a file that is gone.
    if app.run_user("file.openRecent", params).is_some_and(|r| r.get("index").is_some()) {
        app.ui.home = false;
    }
    true
}

/// A submenu row `label` in the menu being drawn; while it's open, `add` draws its items beside
/// it. `key` names the submenu (its menu path, unique in its menu tree). Returns what `add`
/// returned.
pub(crate) fn submenu<R>(ui: &mut Ui, key: &str, label: &str, t: &Tokens, add: impl FnOnce(&mut Ui) -> Option<R>) -> Option<R> {
    submenu_at(ui, Id::new(("submenu", key)), key, label, 4.0, false, t, add)
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
struct At {
    root: Id,
    path: String,
}

fn at_key(ui: &Ui) -> Id {
    ui.id().with("menu_level")
}

fn level_at(ui: &Ui) -> At {
    ui.data(|d| d.get_temp::<At>(at_key(ui))).unwrap_or(At { root: Id::new("menus"), path: String::new() })
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
fn set_open(ctx: &egui::Context, root: Id, path: &str, key: Option<&str>) {
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
fn plain_row_hover(ui: &Ui, resp: &egui::Response) -> bool {
    let hovered = resp.hovered() && !aiming(ui);
    if hovered {
        hover_request(ui.ctx(), &level_at(ui), None);
    }
    hovered || resp.has_focus()
}

/// A submenu row whose panel (area `area`, `gap` right of the row) shows while it's open.
#[allow(clippy::too_many_arguments)]
fn submenu_at<R>(
    ui: &mut Ui,
    area: Id,
    key: &str,
    label: &str,
    gap: f32,
    constrain: bool,
    t: &Tokens,
    add: impl FnOnce(&mut Ui) -> Option<R>,
) -> Option<R> {
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
    let pos = pos2(r.max.x + gap, r.min.y - 8.0);
    let child = At { root: at.root, path: key.to_string() };
    let shown =
        egui::Area::new(area).fixed_pos(pos).order(egui::Order::Foreground).constrain(constrain).show(ui.ctx(), |ui| menu_frame(ui, t, child, add));
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
fn menu_frame<R>(ui: &mut Ui, t: &Tokens, at: At, add: impl FnOnce(&mut Ui) -> R) -> R {
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

/// Open the main menu at top-level menu `top` (control channel).
pub fn open_main_menu(ctx: &egui::Context, top: &str) {
    set_open(ctx, main_menu_id(), "", Some(top));
}

/// The context menu's area (and menu tree).
fn ctx_menu_id() -> Id {
    Id::new("ctx_menu")
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
    // A closed menu forgets its open submenus (and a context menu opened somewhere else starts
    // afresh), so neither reopens with a stale submenu showing.
    if !app.ui.main_menu {
        forget_tree(ctx, main_menu_id());
    }
    let ctx_at = app.ui.context_menu.as_ref().map(|(p, _)| *p);
    let ctx_seen = Id::new("ctx_menu_at");
    if ctx.data(|d| d.get_temp::<Option<egui::Pos2>>(ctx_seen)) != Some(ctx_at) {
        ctx.data_mut(|d| d.insert_temp(ctx_seen, ctx_at));
        forget_tree(ctx, ctx_menu_id());
    }
    // Select layer opened or closed from the control channel.
    if let Some(open) = ctx.data_mut(|d| d.remove_temp::<bool>(select_layer_ask_id())) {
        set_open(ctx, ctx_menu_id(), "", open.then_some(SELECT_LAYER));
    }
    // Main menu.
    if app.ui.main_menu {
        let mut close = false;
        let area = egui::Area::new(main_menu_id()).fixed_pos(pos2(8.0, crate::tabs::HEIGHT + 48.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            menu_frame(ui, &t, At { root: main_menu_id(), path: String::new() }, |ui| {
                // Quick actions search row.
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                plain_row_hover(ui, &resp);
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
                    set_palette(&mut app.ui, true);
                    close = true;
                }
                ui.add_space(6.0);
                for top in TOP_MENUS {
                    if submenu_at(ui, Id::new(("menu", *top)), top, tr(top), 12.0, false, &t, |ui| menu_level(app, ui, top).then_some(())).is_some() {
                        close = true;
                    }
                }
            });
        });
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && !over_menu(ctx) || ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if close || (clicked_outside && !area.response.hovered()) {
            app.ui.main_menu = false;
            forget_tree(ctx, main_menu_id());
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
        let area = egui::Area::new(ctx_menu_id()).fixed_pos(pos).order(egui::Order::Foreground).constrain(true).show(ctx, |ui| {
            menu_frame(ui, &t, At { root: ctx_menu_id(), path: String::new() }, |ui| {
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
                // Frames on the page (or in a section) convert to sections, and sections to frames.
                let convert = app.session.doc().ok().and_then(|doc| {
                    let nodes: Vec<_> = ids.iter().filter_map(|i| doc.node(*i)).collect();
                    let on_board = |n: &figcraft_doc::Node| {
                        n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.is_page() || matches!(p.kind, figcraft_doc::NodeKind::Section { .. }))
                    };
                    if nodes.is_empty() {
                        None
                    } else if nodes.iter().all(|n| matches!(n.kind, figcraft_doc::NodeKind::Section { .. })) {
                        Some((tr("Convert to frame"), "object.convertToFrame"))
                    } else if nodes.iter().all(|n| matches!(n.kind, figcraft_doc::NodeKind::Frame) && on_board(n)) {
                        Some((tr("Convert to section"), "object.convertToSection"))
                    } else {
                        None
                    }
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
                if let Some(entry) = convert
                    && let Some(at) = entries.iter().position(|e| e.1 == "object.wrapInSection")
                {
                    entries.insert(at + 1, entry);
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
                            paste(app, None, Some((p.x, p.y)));
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
            forget_tree(ctx, ctx_menu_id());
        }
    }
    crate::colorpicker::show(app, ctx);
    palette(app, ctx);
}

/// The context menu's Select layer submenu (its key in the context menu's tree).
const SELECT_LAYER: &str = "Select layer";

fn select_layer_ask_id() -> Id {
    Id::new("ctx_select_layer_ask")
}

/// Open (or close) the context menu's Select layer submenu (applied when the menu next draws).
pub fn open_select_layer(ctx: &egui::Context, open: bool) {
    ctx.data_mut(|d| d.insert_temp(select_layer_ask_id(), open));
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
    let picked = submenu_at(ui, Id::new("ctx_select_layer"), SELECT_LAYER, tr("Select layer"), 4.0, true, t, |ui| {
        let mut picked = None;
        for (id, depth, name) in &layers {
            if indented_item(ui, name, "", true, sel.contains(id), 12.0 * (*depth).min(8) as f32, t) {
                picked = Some(*id);
            }
        }
        picked
    });
    separator(ui);
    match picked {
        Some(id) => {
            app.run("select.set", json!({"ids": [id]}));
            true
        }
        None => false,
    }
}

/// The Actions palette's content height (the frame adds its own 8 px margins).
const PALETTE_H: f32 = 450.0;

/// Open the Actions palette afresh (empty search, All tab, first row highlighted), or close it.
pub fn set_palette(ui: &mut crate::UiState, open: bool) {
    ui.palette = open.then(String::new);
    ui.palette_tab = PaletteTab::All;
    ui.palette_hi = 0;
    ui.palette_armed = open;
}

/// Open the Actions palette afresh, or close it when it's open (⌘K, the toolbar's Actions button).
pub fn toggle_palette(ui: &mut crate::UiState) {
    let open = ui.palette.is_none();
    set_palette(ui, open);
}

/// The Actions palette: search over commands, assets and plugins, with tabs, recents and grouped
/// sections, floating above the toolbar. Rows run on click; Up and Down move the highlight and
/// Enter runs the highlighted row.
fn palette(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut q) = app.ui.palette.clone() else { return };
    let t = Tokens::get(ctx);
    // Same centre and width as the toolbar pill (it centres on the canvas, not the window),
    // floating 8 px above it. Fixed size: switching tabs never resizes the panel.
    let bar_w = crate::toolbar::main_width();
    let bar = crate::toolbar::pill_rect(app.canvas.rect, bar_w);
    let area = Rect::from_min_max(pos2(bar.min.x, bar.min.y - 8.0 - (PALETTE_H + 16.0)), pos2(bar.max.x, bar.min.y - 8.0));
    let mut run: Option<String> = None;
    let mut close = false;
    // A press anywhere outside closes the panel. The opening press is spared once, and the
    // spare expires the first frame without a press (so the next outside press always closes).
    let pressed = ctx.input(|i| i.pointer.any_pressed());
    if pressed && !std::mem::replace(&mut app.ui.palette_armed, false) {
        let on_toggle = app.ui.actions_rect.is_some_and(|b| ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| b.contains(p))));
        let outside = !on_toggle && ctx.input(|i| i.pointer.hover_pos().map(|p| !area.contains(p)).unwrap_or(false));
        if outside {
            close = true;
        }
    } else if !pressed {
        app.ui.palette_armed = false;
    }
    // Taken before the search field sees them (it would move its cursor).
    let step = ctx.input_mut(|i| {
        i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) as i64
            - i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) as i64
    });
    egui::Area::new(Id::new("palette")).fixed_pos(area.min).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .corner_radius(CornerRadius::same(13))
            .stroke(Stroke::new(1.0, t.border))
            .shadow(egui::Shadow { offset: [0, 8], blur: 32, spread: 0, color: t.shadow })
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                // Same outer width as the toolbar pill, and a fixed height: switching tabs
                // never resizes the panel.
                let inner = bar_w - 16.0;
                ui.set_width(inner);
                ui.set_height(PALETTE_H);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut q)
                        .hint_text(tr("Search actions, layers and tools"))
                        .desired_width(inner - 16.0)
                        .font(regular(14.0))
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(8))),
                );
                r.request_focus();
                crate::widgets::select_all_on_focus(ui, &r, &q);
                // A new search starts again at the first row.
                if r.changed() {
                    app.ui.palette_hi = 0;
                }
                ui.add_space(4.0);
                // Tabs: All, Assets, Plugins & widgets.
                let tabs = [(PaletteTab::All, tr("All")), (PaletteTab::Assets, tr("Assets")), (PaletteTab::Plugins, tr("Plugins & widgets"))];
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
                    for (tab, label) in tabs {
                        let active = app.ui.palette_tab == tab;
                        // The frame stays on for every tab (transparent when idle) so the
                        // active pill never moves its neighbours.
                        let resp = ui.add(
                            egui::Button::new(egui::RichText::new(label).size(12.0).color(if active { t.text } else { t.text_secondary }))
                                .frame(true)
                                .fill(if active { t.hover } else { Color32::TRANSPARENT })
                                .corner_radius(CornerRadius::same(6)),
                        );
                        if resp.clicked() {
                            app.ui.palette_tab = tab;
                            app.ui.palette_hi = 0;
                        }
                    }
                });
                ui.add_space(4.0);
                let sections = palette_sections(app, &q, app.ui.palette_tab);
                let count = sections.iter().map(|(_, rows)| rows.len()).sum::<usize>();
                // At either end it stays where it was.
                let hi = (app.ui.palette_hi as i64 + step).clamp(0, count.saturating_sub(1) as i64) as usize;
                app.ui.palette_hi = hi;
                // The pointer moving over a row moves the highlight there.
                let pointer_moved = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    let mut i = 0;
                    for (name, rows) in &sections {
                        section(ui, &t, name, |ui| {
                            for row in rows {
                                let resp = draw_row(ui, &t, row, i == hi);
                                if resp.clicked() {
                                    run = Some(row.key.clone());
                                }
                                if pointer_moved && resp.hovered() && i != hi {
                                    app.ui.palette_hi = i;
                                    // Drawn there on the next frame, even if the pointer stops now.
                                    ui.ctx().request_repaint();
                                }
                                if step != 0 && i == hi {
                                    resp.scroll_to_me(None);
                                }
                                i += 1;
                            }
                        });
                    }
                });
                if run.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    run = sections.iter().flat_map(|(_, rows)| rows).nth(hi).map(|row| row.key.clone());
                }
            });
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if let Some(key) = run {
        app.ui.palette = None;
        run_action(app, &key);
        return;
    }
    app.ui.palette = if close { None } else { Some(q) };
    let _ = StrokeKind::Inside;
}

/// The rows a palette tab shows for the query, by section: All has Recents, the commands by menu,
/// then plugins and widgets; Assets this file's components and styles; Plugins & widgets those.
pub(crate) fn palette_sections(app: &FigApp, q: &str, tab: PaletteTab) -> Vec<(String, Vec<ActionRow>)> {
    let mut out = match tab {
        PaletteTab::All => {
            let recents = app.session.prefs.recent_actions.iter().filter_map(|key| recent_row(app, key, q)).collect();
            let mut out = vec![(tr("Recents").to_string(), recents)];
            out.extend(command_sections(q));
            out
        }
        PaletteTab::Assets => asset_sections(app, q),
        PaletteTab::Plugins => Vec::new(),
    };
    if tab != PaletteTab::Assets {
        out.push((tr("Plugins").to_string(), plugin_rows(app, q)));
        out.push((tr("Widgets").to_string(), widget_rows(app, q)));
    }
    out.retain(|(_, rows)| !rows.is_empty());
    out
}

/// Run a palette row: `command:<id>`, `plugin:<plugin>/<command>`, `widget:<plugin>`,
/// `component:<node>` or `style:<id>`.
pub(crate) fn run_action(app: &mut FigApp, key: &str) {
    if let Some(id) = key.strip_prefix("command:") {
        record_recent(app, key);
        invoke(app, id);
    } else if let Some(rest) = key.strip_prefix("plugin:") {
        if let Some((plugin, command)) = rest.split_once('/') {
            record_recent(app, key);
            crate::plugins_ui::start(app, plugin, command);
        }
    } else if let Some(plugin) = key.strip_prefix("widget:") {
        record_recent(app, key);
        app.run("widget.insert", json!({ "plugin": plugin }));
    } else if let Some(node) = key.strip_prefix("component:") {
        if let Ok(n) = node.parse::<u64>() {
            let (cx, cy) = app.session.ds().map(|d| (d.view.cx, d.view.cy)).unwrap_or((0.0, 0.0));
            app.run("instance.place", json!({ "component": n, "x": cx, "y": cy }));
        }
    } else if let Some(style) = key.strip_prefix("style:") {
        if app.session.selection().is_empty() {
            app.toast(tr("Select layers to apply the style to"));
        } else {
            app.run("style.apply", json!({ "style": style }));
        }
    }
}

/// One pickable palette row.
pub(crate) struct ActionRow {
    pub(crate) key: String,
    pub(crate) label: String,
    hint: String,
    /// A letter tile instead of an icon (plugins carry no artwork).
    tile: Option<char>,
}

/// A section header with its rows.
fn section(ui: &mut Ui, t: &Tokens, name: &str, rows: impl FnOnce(&mut Ui)) {
    ui.add_space(10.0);
    ui.painter().text(pos2(ui.min_rect().min.x + 10.0, ui.cursor().min.y + 8.0), Align2::LEFT_TOP, name, regular(12.5), t.text_secondary);
    ui.add_space(22.0);
    rows(ui);
}

/// Draw one row (a letter tile for plugins, else just the label) with its right-side hint,
/// `highlighted` when Enter runs it.
fn draw_row(ui: &mut Ui, t: &Tokens, row: &ActionRow, highlighted: bool) -> egui::Response {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
    // The highlight pill stays inside the row: it never touches the panel's edges.
    let pill = r.shrink2(vec2(4.0, 1.0));
    if highlighted {
        ui.painter().rect_filled(pill, CornerRadius::same(6), t.hover);
    }
    let mut label_x = r.min.x + 10.0;
    if let Some(letter) = row.tile {
        let tile = Rect::from_center_size(pos2(r.min.x + 20.0, r.center().y), vec2(18.0, 18.0));
        ui.painter().rect_filled(tile, CornerRadius::same(5), t.hover);
        ui.painter().text(tile.center(), Align2::CENTER_CENTER, letter.to_string(), regular(11.0), t.text);
        label_x = r.min.x + 38.0;
    }
    ui.painter().text(pos2(label_x, r.center().y), Align2::LEFT_CENTER, &row.label, regular(13.0), t.text);
    ui.painter().text(pos2(r.max.x - 10.0, r.center().y), Align2::RIGHT_CENTER, &row.hint, regular(11.0), t.text_secondary);
    resp
}

/// Whether `label` (or `extra`) matches the query, in any case.
fn matches(q: &str, label: &str, extra: &str) -> bool {
    q.is_empty() || label.to_lowercase().contains(q) || extra.to_lowercase().contains(q)
}

/// A command's row: its label, and its shortcut as the hint.
fn command_row(id: &str, label: String) -> ActionRow {
    ActionRow { key: format!("command:{id}"), label, hint: crate::shortcuts::shortcut(id).map(pretty_shortcut).unwrap_or_default(), tile: None }
}

/// Most rows a search shows (the best matches).
const MAX_HITS: usize = 60;

/// Commands grouped by top-level menu, filtered by the query. Without one, the menus' items in
/// catalog order; a search covers every command (the engine's and the UI's own, "Other" for those
/// no menu lists), best matches first (labels starting with the query, then the shortest), each
/// group placed by its best row. Labels match in the UI language and in English. The Plugins
/// menu's items come with the plugins ([`plugin_rows`]).
pub(crate) fn command_sections(q: &str) -> Vec<(String, Vec<ActionRow>)> {
    let ql = q.trim().to_lowercase();
    let mut menu_of = std::collections::HashMap::new();
    for m in MENU.iter().filter(|m| m.label != SEP) {
        menu_of.entry(m.id).or_insert(m);
    }
    let menu_items = MENU.iter().filter(|m| m.label != SEP).map(|m| (m.id, m.label));
    let engine = figcraft_engine::all_commands().iter().map(|c| (c.id, c.label));
    let candidates: Box<dyn Iterator<Item = (&str, &str)>> = if ql.is_empty() { Box::new(menu_items) } else { Box::new(menu_items.chain(engine)) };
    let mut seen = std::collections::HashSet::new();
    // By top-level menu; `None`: no menu lists it.
    let mut hits: Vec<(Option<&str>, ActionRow)> = Vec::new();
    for (id, english) in candidates {
        if !available(id) || !seen.insert(id) {
            continue;
        }
        let menu = menu_of.get(id);
        let group = menu.map(|m| m.menu.split('/').next().unwrap_or(m.menu));
        if group == Some("Plugins") {
            continue;
        }
        let label = match menu {
            Some(m) => i18n::menu_item(id, m.label),
            None => i18n::command(id, english),
        };
        if matches(&ql, &label, &format!("{english} {id}")) {
            hits.push((group, command_row(id, label.into_owned())));
        }
    }
    if !ql.is_empty() {
        hits.sort_by_cached_key(|(_, row)| (!row.label.to_lowercase().starts_with(&ql), row.label.chars().count()));
        hits.truncate(MAX_HITS);
    }
    let mut groups: Vec<(String, Vec<ActionRow>)> = Vec::new();
    for (group, row) in hits {
        let group = group.map_or_else(|| tr("Other"), tr).to_string();
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, rows)) => rows.push(row),
            None => groups.push((group, vec![row])),
        }
    }
    groups
}

/// Installed plugins' commands, filtered by the query.
fn plugin_rows(app: &FigApp, q: &str) -> Vec<ActionRow> {
    let ql = q.to_lowercase();
    let mut rows = Vec::new();
    for p in &app.session.prefs.plugins {
        let m = &p.manifest;
        for c in &m.commands {
            let label = if m.commands.len() == 1 { m.name.clone() } else { format!("{} — {}", m.name, c.label) };
            if !matches(&ql, &label, &m.id) {
                continue;
            }
            rows.push(ActionRow {
                key: format!("plugin:{}/{}", m.id, c.id),
                label,
                hint: if c.params.is_empty() { tr("Plugin").to_string() } else { c.label.clone() },
                tile: m.name.chars().next(),
            });
        }
    }
    // The Plugins menu's own items (run last, manage, import).
    for m in MENU.iter().filter(|m| m.label != SEP && m.menu.split('/').next() == Some("Plugins") && available(m.id)) {
        let label = i18n::menu_item(m.id, m.label);
        if matches(&ql, &label, &format!("{} {}", m.label, m.id)) {
            rows.push(command_row(m.id, label.into_owned()));
        }
    }
    rows
}

/// Installed widgets, filtered by the query.
fn widget_rows(app: &FigApp, q: &str) -> Vec<ActionRow> {
    let ql = q.to_lowercase();
    app.session
        .prefs
        .plugins
        .iter()
        .filter(|p| p.manifest.widget)
        .filter(|p| matches(&ql, &p.manifest.name, &p.manifest.id))
        .map(|p| ActionRow {
            key: format!("widget:{}", p.manifest.id),
            label: p.manifest.name.clone(),
            hint: tr("Widget").to_string(),
            tile: p.manifest.name.chars().next(),
        })
        .collect()
}

/// This file's components and styles, filtered by the query.
fn asset_sections(app: &FigApp, q: &str) -> Vec<(String, Vec<ActionRow>)> {
    let ql = q.to_lowercase();
    let Ok(doc) = app.session.doc() else { return Vec::new() };
    let components: Vec<ActionRow> = figcraft_engine::library::local_components(doc)
        .iter()
        .filter_map(|id| doc.node(*id))
        .filter(|n| matches(&ql, &n.name, ""))
        .map(|n| ActionRow { key: format!("component:{}", n.id.0), label: n.name.clone(), hint: tr("Component").to_string(), tile: None })
        .collect();
    let styles: Vec<ActionRow> = figcraft_engine::library::local_styles(doc)
        .filter(|s| matches(&ql, &s.name, ""))
        .map(|s| ActionRow { key: format!("style:{}", s.id), label: s.name.clone(), hint: tr("Style").to_string(), tile: None })
        .collect();
    let mut out = Vec::new();
    if !components.is_empty() {
        out.push((tr("Components").to_string(), components));
    }
    if !styles.is_empty() {
        out.push((tr("Styles").to_string(), styles));
    }
    out
}

/// A recent action back as a row (gone commands, plugins or assets drop out).
pub(crate) fn recent_row(app: &FigApp, key: &str, q: &str) -> Option<ActionRow> {
    let ql = q.to_lowercase();
    if let Some(id) = key.strip_prefix("command:") {
        let spec = figcraft_engine::find_command(id);
        let menu_label = figcraft_engine::catalog::MENU.iter().find(|m| m.id == id);
        let english = menu_label.map(|m| m.label).or_else(|| spec.map(|s| s.label)).unwrap_or(id);
        let label = i18n::command(id, english);
        if !available(id) || !matches(&ql, &label, &format!("{english} {id}")) {
            return None;
        }
        return Some(command_row(id, label.into_owned()));
    }
    if let Some(rest) = key.strip_prefix("plugin:") {
        let (pid, cid) = rest.split_once('/')?;
        let p = app.session.prefs.plugins.iter().find(|p| p.manifest.id == pid)?;
        let c = p.manifest.commands.iter().find(|c| c.id == cid)?;
        let label = if p.manifest.commands.len() == 1 { p.manifest.name.clone() } else { format!("{} — {}", p.manifest.name, c.label) };
        if !matches(&ql, &label, pid) {
            return None;
        }
        return Some(ActionRow { key: key.to_string(), label, hint: tr("Plugin").to_string(), tile: p.manifest.name.chars().next() });
    }
    if let Some(pid) = key.strip_prefix("widget:") {
        let p = app.session.prefs.plugins.iter().find(|p| p.manifest.id == pid && p.manifest.widget)?;
        if !matches(&ql, &p.manifest.name, pid) {
            return None;
        }
        return Some(ActionRow {
            key: key.to_string(),
            label: p.manifest.name.clone(),
            hint: tr("Widget").to_string(),
            tile: p.manifest.name.chars().next(),
        });
    }
    None
}
