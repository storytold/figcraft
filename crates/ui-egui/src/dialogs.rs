//! Dialogs: About, Preferences, keyboard shortcuts, find & replace, rename layers, variables,
//! export.

use std::path::Path;

use egui::{Align2, Context, CornerRadius, RichText, Sense, Stroke, vec2};
use figcraft_color::ColorProfile;
use figcraft_doc::{ExportFormat, ExportSetting, ExportSize, NodeId};
use figcraft_engine::cmd::file::{export_jobs, exportable_layers};
use serde_json::{Value, json};

use crate::FigApp;
use crate::export_panel::{export_layers, save_one, suggested_name};
use crate::i18n::{self, tr, tr_noop, trf, trn};
use crate::theme::{Tokens, regular, semibold};

mod batch_rename;
mod color_profile;
mod export;
mod menu_input;

pub use batch_rename::BatchRenameUi;
use batch_rename::batch_rename_dialog;
#[cfg(test)]
pub(crate) use batch_rename::batch_rename_params;
pub(crate) use batch_rename::open_batch_rename;
use color_profile::color_profile_dialog;
use export::export_dialog;
pub(crate) use export::file_saved;
pub use menu_input::MenuInput;
use menu_input::menu_input_dialog;
pub mod branch_ui;
pub mod credits;
pub mod import_report;
pub mod missing_fonts;
pub mod plugins_ui;
pub mod sampled_color;
pub mod updates_ui;
pub mod variables_panel;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where a report goes.
const ISSUES: &str = "https://github.com/storytold/figcraft/issues";

/// What the desktop app says when its window couldn't open with `error` (no frame was drawn: the
/// language is the one [`i18n::set`] picked from the preferences). `backend_set`: `WGPU_BACKEND`
/// named the graphics API, so the hint is to unset it rather than to try another one. `tried`: the
/// graphics APIs the app already started again with, so naming another one wouldn't help.
pub fn window_failed(error: &str, backend_set: bool, tried: Option<&str>) -> String {
    let hint = if let Some(apis) = tried {
        trf(
            "FigCraft tried {apis}, then again with lower requirements on the power-saving GPU. Updating the graphics driver often helps.",
            &[("apis", &apis)],
        )
    } else if backend_set {
        tr("WGPU_BACKEND is set and names the graphics API to use; unsetting it lets FigCraft choose.").to_string()
    } else if cfg!(windows) {
        // Direct3D 12 and OpenGL are both offered, but wgpu keeps the Direct3D 12 adapter it picked
        // even when its device then fails, so naming OpenGL alone can still help.
        format!(
            "{}

set WGPU_BACKEND=gl
figcraft.exe",
            tr("Updating the graphics driver often helps. To use OpenGL only, start FigCraft from a Command Prompt with:")
        )
    } else {
        tr("Updating the graphics driver often helps. WGPU_BACKEND (vulkan, gl, metal) names another graphics API to try.").to_string()
    };
    format!(
        "{}

{hint}

{}",
        trf("FigCraft couldn't open its window: {error}", &[("error", &error)]),
        trf("If it keeps happening, please report it at {link} with this message.", &[("link", &ISSUES)])
    )
}

/// A dialog window, centred.
pub(crate) fn window<'a>(title: &'a str, ctx: &Context) -> egui::Window<'a> {
    floating(title, ctx).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
}

/// A window styled like the dialogs that the user can move.
pub(crate) fn floating<'a>(title: &'a str, ctx: &Context) -> egui::Window<'a> {
    let t = Tokens::get(ctx);
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .frame(egui::Frame::window(&ctx.global_style()).fill(t.panel).inner_margin(egui::Margin::same(20)))
}

/// The About window's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AboutTab {
    #[default]
    About,
    Contributors,
    Models,
}

impl AboutTab {
    pub const ALL: [AboutTab; 3] = [AboutTab::About, AboutTab::Contributors, AboutTab::Models];

    pub fn label(self) -> &'static str {
        match self {
            AboutTab::About => tr("About"),
            AboutTab::Contributors => tr("Contributors"),
            AboutTab::Models => tr("Models"),
        }
    }
}

/// The About window's width, the same on every tab (the Contributors table needs the room).
pub const ABOUT_WIDTH: f32 = 660.0;

/// The About window's tab row, drawn like the Design / Prototype tabs of the right panel.
/// A row of tabs (the About window's, the shortcuts panel's): the current one highlighted.
pub(crate) fn tab_strip<T: Copy + PartialEq>(ui: &mut egui::Ui, t: &Tokens, tabs: &[T], label: fn(T) -> &'static str, current: &mut T) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for &tab in tabs {
            let label = label(tab);
            let active = *current == tab;
            let galley = ui.painter().layout_no_wrap(label.into(), semibold(11.0), if active { t.text } else { t.text_secondary });
            let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, 24.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
            if active || resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(5), t.hover);
            }
            ui.painter().galley(egui::pos2(r.min.x + 8.0, r.center().y - galley.size().y / 2.0), galley, t.text);
            if resp.clicked() {
                *current = tab;
            }
        }
    });
}

/// About ▸ About: the logo, version, blurb and links.
fn about_body(ui: &mut egui::Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
        crate::theme::paint_app_icon(ui, r);
        ui.vertical(|ui| {
            let (wordmark, _) = ui.allocate_exact_size(vec2(130.0, 23.0), Sense::hover());
            crate::theme::paint_wordmark(ui.painter(), egui::pos2(wordmark.min.x, wordmark.center().y), 18.0, t);
            ui.label(RichText::new(trf("Version {version}", &[("version", &VERSION)])).color(t.text_secondary));
        });
    });
    ui.add_space(12.0);
    ui.label(tr(
        "Interface design and prototyping. A free, open-source, clean-room design tool written from scratch in Rust, part of the ArtCraft Crafting Apps.",
    ));
    ui.add_space(8.0);
    ui.hyperlink_to("getartcraft.com/apps/figcraft", "https://getartcraft.com/apps/figcraft");
    ui.hyperlink_to(tr("Join us on Discord"), "https://discord.gg/artcraft");
    ui.add_space(8.0);
    ui.label(
        RichText::new(tr("Dual-licensed MIT OR Apache-2.0. Made by the ArtCraft team and community.")).color(t.text_secondary).font(regular(11.0)),
    );
}

/// Escape closes the topmost simple dialog (one without menus of its own), before the canvas's
/// shortcuts see it (it doesn't deselect), like a system dialog: from one of its fields too. A menu
/// open over the dialog (Preferences ▸ Interface size) takes the first Escape and closes alone.
pub fn keys(app: &mut FigApp, ctx: &Context) {
    if egui::Popup::is_any_open(ctx) {
        return;
    }
    let u = &mut app.ui;
    let open = u.color_profile.is_some()
        || u.menu_input.is_some()
        || u.find.is_some()
        || u.batch_rename.is_some()
        || u.import_report.is_some()
        || u.missing_fonts.is_some()
        || u.confirm_close
        || u.export_preview
        || u.shortcuts_panel
        || u.preferences
        || u.about;
    if !open || !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        return;
    }
    if u.menu_input.take().is_some()
        || u.color_profile.take().is_some()
        || u.find.take().is_some()
        || u.batch_rename.take().is_some()
        || u.import_report.take().is_some()
        || u.missing_fonts.take().is_some()
    {
        return;
    }
    for flag in [&mut u.confirm_close, &mut u.export_preview, &mut u.shortcuts_panel, &mut u.preferences, &mut u.about] {
        if std::mem::take(flag) {
            return;
        }
    }
}

/// Draw the app's dialogs and cards: pending opens/imports/missing fonts, About,
/// Preferences, shortcuts, find/replace, batch rename, menu input, variables, dev compare,
/// branch, export preview, colour profile, plugin, live join/publish, online dialogs and
/// close confirmation.
pub fn show(app: &mut FigApp, ctx: &Context) {
    // Dropped files waiting to open: their card, then (once it has shown) the files.
    crate::pending_open::pending_opens(app, ctx);
    crate::import_report::show(app, ctx);
    crate::missing_fonts::show(app, ctx);
    crate::sampled_color::show(app, ctx);
    let t = Tokens::get(ctx);
    if app.ui.about {
        let mut open = true;
        window(tr("About FigCraft"), ctx).open(&mut open).show(ctx, |ui| {
            ui.set_width(ABOUT_WIDTH);
            tab_strip(ui, &t, &AboutTab::ALL, AboutTab::label, &mut app.ui.about_tab);
            ui.add_space(12.0);
            match app.ui.about_tab {
                AboutTab::About => about_body(ui, &t),
                AboutTab::Contributors => crate::credits::contributors_ui(ui),
                AboutTab::Models => crate::credits::models_ui(ui),
            }
        });
        if !open {
            app.ui.about = false;
        }
    }
    if app.ui.preferences {
        let mut open = true;
        window(tr("Preferences"), ctx).open(&mut open).show(ctx, |ui| {
            ui.set_width(360.0);
            let prefs = app.session.prefs.clone();
            let mut toggles = vec![];
            for (label, val, cmd) in [
                (tr("Keep tool selected after use"), prefs.keep_tool, "pref.keepTool"),
                (tr("Highlight layers on hover"), prefs.highlight_on_hover, "pref.highlightHover"),
                (tr("Rename duplicated layers"), prefs.rename_duplicates, "pref.renameDuplicates"),
                (tr("Show dimensions on objects"), prefs.show_dimensions, "pref.showDimensions"),
                (tr("Use number keys for opacity"), prefs.number_keys_opacity, "pref.numberKeysOpacity"),
                (tr("Scroll wheel zoom"), prefs.scroll_wheel_zoom, "pref.scrollWheelZoom"),
                (tr("Reduce motion"), prefs.reduce_motion, "pref.reduceMotion"),
            ] {
                if crate::widgets::checkbox(ui, val, label) {
                    toggles.push(cmd);
                }
            }
            if app.services.reduce_motion() {
                crate::widgets::label(ui, tr("Your system asks for reduced motion: animations stay off."), t.text_secondary);
            }
            // The browser has no files to save to.
            if !cfg!(target_arch = "wasm32") && crate::widgets::checkbox(ui, prefs.auto_save, tr("Save files automatically")) {
                toggles.push("pref.autoSave");
            }
            if !cfg!(target_arch = "wasm32") && crate::widgets::checkbox(ui, prefs.start_on_home, tr("Start on the Home screen")) {
                toggles.push("pref.startOnHome");
            }
            for c in toggles {
                app.run(c, json!({}));
            }
            // Where the app registers itself as the opener of invite links (Windows).
            if let Some(on) = app.session.invite_links_on() {
                if crate::widgets::checkbox(ui, on, tr("Open invite links in FigCraft")) {
                    app.run("collab.registerLinks", json!({"on": !on}));
                }
                crate::widgets::label(
                    ui,
                    tr("figcraft:// links from a browser, chat or email open this app and join the session."),
                    t.text_secondary,
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tr("Your name"));
                crate::live::name_field(app, ui, egui::Id::unique("pref_user_name"), 160.0);
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tr("Server for online sharing"));
                // Where Share ▸ Online keeps files and join codes are asked; no key here
                // (preferences are saved as text).
                let name = tr("Server for online sharing");
                if let Some(url) = crate::widgets::text_field_named(ui, egui::Id::unique("pref_publish_server"), &prefs.publish_server, 220.0, name) {
                    app.run("pref.publishServer", json!({"url": url}));
                }
                let usual = prefs.publish_server == figcraft_engine::live::DEFAULT_SERVER;
                if crate::widgets::button(ui, tr("Reset"), false, !usual).on_hover_text(tr("Back to the FigCraft server")).clicked() {
                    app.run("pref.publishServer", json!({}));
                }
            });
            if prefs.publish_server.is_empty() {
                crate::widgets::label(ui, tr("Online sharing is off: files are shared on the local network only."), t.text_secondary);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tr("Nudge amount"));
                let mut small = prefs.nudge_small;
                let mut big = prefs.nudge_big;
                let a = ui.add(egui::DragValue::new(&mut small).range(0.1..=100.0));
                let b = ui.add(egui::DragValue::new(&mut big).range(1.0..=1000.0));
                if a.changed() || b.changed() {
                    app.run("pref.nudge", json!({"small": small, "big": big}));
                }
            });
            ui.add_space(8.0);
            interface_size(app, ui, prefs.ui_scale);
            ui.add_space(8.0);
            crate::board_bar::grid_settings(app, ui);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tr("Theme"));
                for (l, c) in [(tr_noop("Light"), "view.themeLight"), (tr_noop("Dark"), "view.themeDark"), (tr_noop("System"), "view.themeSystem")] {
                    if ui.selectable_label(app.session.prefs.theme.eq_ignore_ascii_case(l), tr(l)).clicked() {
                        app.run(c, json!({}));
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tr("Language"));
                // Each language by its own name; System follows the OS (the browser on the web).
                let langs = || {
                    [("system", tr("System"))]
                        .into_iter()
                        .chain(i18n::languages().iter().filter(|l| crate::cjk::can_show(l.code)).map(|l| (l.code, l.name)))
                };
                let on = |code: &str| app.session.prefs.language.eq_ignore_ascii_case(code);
                let selected = langs().find(|(code, _)| on(code)).map_or(tr("System"), |(_, name)| name);
                let resp = crate::widgets::dropdown(ui, selected, 220.0, None);
                let mut pick = None;
                crate::right::menu_popup(&resp).show(|ui| {
                    for (code, name) in langs() {
                        if crate::right::menu_check(ui, on(code), name, "") {
                            pick = Some(code);
                            ui.close();
                        }
                    }
                });
                if let Some(code) = pick {
                    app.run("pref.language", json!({"language": code}));
                    ui.ctx().request_repaint();
                }
            });
            if let Some(lang) = i18n::current().filter(|l| l.draft) {
                let note = trf("{language}: draft translation, waiting for a native speaker's review.", &[("language", &lang.name)]);
                crate::widgets::label(ui, &note, t.text_secondary);
            }
        });
        if !open {
            app.ui.preferences = false;
        }
    }
    crate::shortcuts_panel::show(app, ctx);
    if let Some((mut find, mut replace)) = app.ui.find.clone() {
        let mut open = true;
        let mut action = None;
        egui::Window::new(tr("Find and replace"))
            .id(egui::Id::unique("Find and replace"))
            .open(&mut open)
            .anchor(Align2::LEFT_TOP, vec2(260.0, 120.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(280.0);
                let r = ui.add(egui::TextEdit::singleline(&mut find).hint_text(tr("Find")).desired_width(260.0));
                crate::widgets::select_all_on_focus(ui, &r, &find);
                let r = ui.add(egui::TextEdit::singleline(&mut replace).hint_text(tr("Replace with")).desired_width(260.0));
                crate::widgets::select_all_on_focus(ui, &r, &replace);
                ui.horizontal(|ui| {
                    if ui.button(tr("Find")).clicked() {
                        action = Some(false);
                    }
                    if ui.button(tr("Replace all")).clicked() {
                        action = Some(true);
                    }
                });
            });
        if let Some(rep) = action
            && !find.is_empty()
        {
            let r = if rep {
                app.run("edit.findReplace", json!({"find": find, "replace": replace, "layerNames": false}))
            } else {
                app.run("edit.findReplace", json!({"find": find, "layerNames": true}))
            };
            if let Some(v) = r {
                let n = v["matches"].as_array().map(|a| a.len()).unwrap_or(0);
                app.toast(trn("{n} matches", n, &[]));
            }
        }
        app.ui.find = if open { Some((find, replace)) } else { None };
    }
    batch_rename_dialog(app, ctx);
    menu_input_dialog(app, ctx);
    crate::variables_panel::show(app, ctx);
    crate::dev_compare::show(app, ctx);
    crate::branch_ui::show(app, ctx);
    if app.ui.export_preview {
        export_dialog(app, ctx);
    }
    color_profile_dialog(app, ctx);
    crate::plugins_ui::show(app, ctx);
    crate::live::join_dialog(app, ctx);
    crate::live::publish_dialog(app, ctx);
    crate::online::dialogs(app, ctx);
    crate::windows::confirm_close(app, ctx);
}

/// Preferences ▸ Interface size: a size relative to the system's display scale (100 % is the
/// system's own), as a drop-down that takes effect at once. The browser has its own zoom, so there
/// the row only says so.
fn interface_size(app: &mut FigApp, ui: &mut egui::Ui, chosen: f64) {
    let t = Tokens::get(ui.ctx());
    if cfg!(target_arch = "wasm32") {
        ui.label(tr("Interface size"));
        crate::widgets::label(ui, tr("Use your browser's zoom to change the interface size."), t.text_secondary);
        return;
    }
    let percent = |size: f64| trf("{percent}%", &[("percent", &crate::ui_scale::percent(size as f32))]);
    let shown = percent(chosen);
    ui.horizontal(|ui| {
        ui.label(tr("Interface size"));
        let resp = crate::widgets::dropdown(ui, &shown, 120.0, None);
        crate::a11y::combo(&resp, tr("Interface size"), &shown, crate::a11y::menu_open(&resp));
        crate::right::menu_popup(&resp).show(|ui| {
            let focused = ui.memory(|memory| memory.focused());
            let step = ui.input(|input| {
                if input.key_pressed(egui::Key::ArrowDown) || (input.key_pressed(egui::Key::Tab) && !input.modifiers.shift) {
                    1
                } else if input.key_pressed(egui::Key::ArrowUp) || (input.key_pressed(egui::Key::Tab) && input.modifiers.shift) {
                    -1
                } else {
                    0
                }
            });
            let mut rows = Vec::with_capacity(crate::ui_scale::SIZES.len());
            for size in crate::ui_scale::SIZES {
                let label = percent(size);
                let selected = (chosen - size).abs() < 1e-6;
                let row = crate::right::menu_row(ui, &format!("{}{label}", if selected { "✓ " } else { "    " }), "", 0.0, |_, _, _| {});
                crate::a11y::menu_item(&row, &label, "", Some(selected));
                if row.has_focus() {
                    ui.memory_mut(|memory| memory.set_focus_lock_filter(row.id, egui::EventFilter { tab: true, ..Default::default() }));
                }
                rows.push(row.id);
                if row.clicked() {
                    app.run("pref.uiScale", json!({"scale": size}));
                    ui.close();
                }
            }
            // Spatial focus can jump to a control behind the popup when the menu overlaps the
            // Preferences window. Once an item is focused, walk the menu's own rows instead.
            if step != 0
                && let Some(index) = rows.iter().position(|id| Some(*id) == focused)
            {
                let next = index.saturating_add_signed(step).min(rows.len() - 1);
                ui.memory_mut(|memory| {
                    memory.request_focus(rows[next]);
                    memory.move_focus(egui::FocusDirection::None);
                });
            }
        });
    });
    crate::widgets::label(
        ui,
        tr("Relative to your system's display scaling (100% is the system's own size). Document zoom is separate."),
        t.text_secondary,
    );
    // Said in words, not only by the menu's value: the window is too small for the size chosen.
    if app.ui.ui_scale.limited {
        let note = trf(
            "The window is too small for {size}%, so the interface is drawn at {drawn}%. Make the window larger to use {size}%.",
            &[("size", &crate::ui_scale::percent(chosen as f32)), ("drawn", &crate::ui_scale::percent(app.ui.ui_scale.zoom))],
        );
        crate::widgets::label(ui, &note, t.text_secondary);
    }
}
