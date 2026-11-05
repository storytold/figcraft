//! Local MCP server & LLM harness configuration dialog (`File ▸ Local MCP server…` and
//! `Plugins ▸ Manage plugins… ▸ MCP`).

use egui::{Context, RichText};
use figcraft_engine::harness::{HarnessKind, attach_command, detect_cli_binary, inspect_all, install};
use serde_json::json;

use crate::FigApp;
use crate::i18n::{tr, trf};
use crate::theme::{Tokens, regular, semibold};

/// Default width of the standalone Local MCP Server window on desktop viewports.
pub const MCP_WINDOW_W: f32 = 440.0;

pub fn show(app: &mut FigApp, ctx: &Context) {
    if !app.ui.mcp_window {
        return;
    }
    let t = Tokens::get(ctx);
    let mut open = true;
    let w = (ctx.content_rect().width() - 48.0).clamp(260.0, MCP_WINDOW_W);
    super::floating(tr("Local MCP Server"), ctx)
        .id(egui::Id::unique("Local MCP Server"))
        .default_pos(ctx.content_rect().center())
        .pivot(egui::Align2::CENTER_CENTER)
        .constrain(true)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(w);
            ui.set_max_width(w);
            body(app, ui, &t);
        });
    if !open {
        app.ui.mcp_window = false;
    }
}

pub(crate) fn body(app: &mut FigApp, ui: &mut egui::Ui, t: &Tokens) {
    let avail_w = ui.available_width();
    ui.set_max_width(avail_w);

    let enabled = app.session.prefs.mcp_enabled;
    let mut port = app.session.prefs.mcp_port;
    let mut changed = false;

    let toggle_checkbox = |ui: &mut egui::Ui, app: &mut FigApp, changed: &mut bool| {
        if crate::widgets::checkbox(ui, enabled, tr("Enable local MCP control server")) && app.run("pref.mcp", json!({"enabled": !enabled})).is_some()
        {
            *changed = true;
        }
    };

    let port_controls = |ui: &mut egui::Ui, app: &mut FigApp, port: &mut u16, changed: &mut bool| {
        let is_default = *port == figcraft_engine::DEFAULT_MCP_PORT;
        if crate::widgets::button(ui, tr("Reset"), false, !is_default).on_hover_text(tr("Reset to default port 47800")).clicked() {
            *port = figcraft_engine::DEFAULT_MCP_PORT;
            if app.run("pref.mcp", json!({"port": *port})).is_some() {
                *changed = true;
            }
        }
        let drag = ui.add(egui::DragValue::new(port).range(1024..=65535));
        if drag.changed() && app.run("pref.mcp", json!({"port": *port})).is_some() {
            *changed = true;
        }
        ui.label(RichText::new(tr("Port")).color(t.text_secondary).font(regular(11.0)));
    };

    if avail_w >= 380.0 {
        ui.horizontal(|ui| {
            toggle_checkbox(ui, app, &mut changed);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                port_controls(ui, app, &mut port, &mut changed);
            });
        });
    } else {
        toggle_checkbox(ui, app, &mut changed);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Port")).color(t.text_secondary).font(regular(11.0)));
            let drag = ui.add(egui::DragValue::new(&mut port).range(1024..=65535));
            if drag.changed() && app.run("pref.mcp", json!({"port": port})).is_some() {
                changed = true;
            }
            let is_default = port == figcraft_engine::DEFAULT_MCP_PORT;
            if crate::widgets::button(ui, tr("Reset"), false, !is_default).on_hover_text(tr("Reset to default port 47800")).clicked() {
                port = figcraft_engine::DEFAULT_MCP_PORT;
                if app.run("pref.mcp", json!({"port": port})).is_some() {
                    changed = true;
                }
            }
        });
    }

    if changed {
        app.services.mcp_server_apply(app.session.prefs.mcp_enabled, app.session.prefs.mcp_port);
        app.services.save_prefs(&app.session.prefs);
    }

    ui.add_space(6.0);
    if let Some((listening, active, err)) = app.services.mcp_server_status() {
        if listening != 0 {
            let status_text = trf("Listening on 127.0.0.1:{port} · {count} active connection(s)", &[("port", &listening), ("count", &active)]);
            ui.add(egui::Label::new(RichText::new(status_text).color(t.accent).font(semibold(11.0))).wrap());
        } else if let Some(err) = err {
            ui.add(egui::Label::new(RichText::new(trf("Error: {error}", &[("error", &err)])).color(t.danger).font(regular(11.0))).wrap());
        } else {
            ui.add(egui::Label::new(RichText::new(tr("Server stopped")).color(t.text_secondary).font(regular(11.0))).wrap());
        }
    } else {
        ui.add(
            egui::Label::new(
                RichText::new(tr("Local MCP control server is available in the desktop app.")).color(t.text_secondary).font(regular(11.0)),
            )
            .wrap(),
        );
    }

    let cli_bin = detect_cli_binary();
    let active_port = app.session.prefs.mcp_port;
    let attach_cmd = attach_command(&cli_bin, active_port);

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("CLI bridge")).color(t.text_secondary).font(semibold(11.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::button(ui, tr("Copy Command"), false, true).clicked() {
                ui.ctx().copy_text(attach_cmd.clone());
                app.toast(tr("Copied MCP command"));
            }
        });
    });
    ui.add_space(2.0);
    egui::Frame::NONE.fill(t.field).corner_radius(egui::CornerRadius::same(4)).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        let inner_w = (avail_w - 16.0).max(120.0);
        ui.set_width(inner_w);
        ui.set_max_width(inner_w);
        ui.add(egui::Label::new(RichText::new(&attach_cmd).color(t.text_secondary).font(egui::FontId::monospace(10.5))).wrap());
    });

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("LLM Harnesses")).color(t.text).font(semibold(12.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::widgets::button(ui, tr("Configure All"), true, true).clicked() {
                let mut ok_count = 0usize;
                let mut last_err = None;
                for kind in HarnessKind::ALL {
                    match install(kind, &cli_bin, Some(active_port)) {
                        Ok(_) => ok_count += 1,
                        Err(e) => last_err = Some(e),
                    }
                }
                if let Some(e) = last_err {
                    app.show_error(e);
                } else {
                    app.toast(trf("Configured {count} MCP harnesses", &[("count", &ok_count)]));
                }
            }
        });
    });
    ui.add(
        egui::Label::new(
            RichText::new(tr("Connect Antigravity, Claude Code, Claude Desktop or Codex directly to this live FigCraft window."))
                .color(t.text_secondary)
                .font(regular(11.0)),
        )
        .wrap(),
    );
    ui.add_space(8.0);

    let statuses = inspect_all(&cli_bin, Some(active_port));
    let scroll_max_h = (ui.ctx().content_rect().height() - 260.0).clamp(180.0, 380.0);
    egui::ScrollArea::vertical().id_salt("mcp_harnesses_scroll").max_height(scroll_max_h).show(ui, |ui| {
        let card_inner_w = (avail_w - 20.0).max(160.0);
        for h in statuses {
            egui::Frame::NONE.fill(t.field).corner_radius(egui::CornerRadius::same(6)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                ui.set_width(card_inner_w);
                ui.set_max_width(card_inner_w);
                let (badge, color) = if h.configured { (tr("Configured"), t.accent) } else { (tr("Not configured"), t.text_secondary) };
                let action_buttons = |ui: &mut egui::Ui, app: &mut FigApp| {
                    let btn_label = if h.configured { tr("Update") } else { tr("Configure") };
                    if crate::widgets::button(ui, btn_label, !h.configured, true).clicked()
                        && let Some(kind) = HarnessKind::parse(&h.id)
                    {
                        match install(kind, &cli_bin, Some(active_port)) {
                            Ok(res) => {
                                app.toast(trf("Configured {name} ({path})", &[("name", &h.label), ("path", &res.config_path.display())]));
                            }
                            Err(e) => app.show_error(e),
                        }
                    }
                    if crate::widgets::button(ui, tr("Copy Config"), false, true).clicked() {
                        ui.ctx().copy_text(h.snippet.clone());
                        app.toast(trf("Copied {name} config", &[("name", &h.label)]));
                    }
                };

                if card_inner_w >= 340.0 {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&h.label).color(t.text).font(semibold(12.0)));
                        ui.label(RichText::new(badge).color(color).font(regular(11.0)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            action_buttons(ui, app);
                        });
                    });
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&h.label).color(t.text).font(semibold(12.0)));
                        ui.label(RichText::new(badge).color(color).font(regular(11.0)));
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        action_buttons(ui, app);
                    });
                }

                ui.add(egui::Label::new(RichText::new(h.config_path.display().to_string()).color(t.text_secondary).font(regular(10.5))).wrap());
            });
            ui.add_space(6.0);
        }
    });
}
