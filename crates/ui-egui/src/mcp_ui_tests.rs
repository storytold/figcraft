//! The Local MCP Server window and the Plugins window's MCP and Development tabs, driven headless
//! through egui at both desktop (1440×900) and narrow (340×600) viewports: every control stays
//! inside the dialog without horizontal overflow, toggling the server applies and saves preferences
//! at once, and the Plugin Dev Console displays and clears execution history.

use std::sync::{Arc, Mutex};

use egui::accesskit::Node;
use egui::{Id, Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::plugins_ui::PluginsTab;

#[derive(Default)]
struct McpLog {
    applied: Vec<(bool, u16)>,
    saved: Vec<figcraft_engine::Prefs>,
}

struct MockMcpServices(Arc<Mutex<McpLog>>);

impl crate::Services for MockMcpServices {
    fn mcp_server_status(&self) -> Option<(u16, usize, Option<String>)> {
        Some((47800, 1, None))
    }

    fn mcp_server_apply(&mut self, enabled: bool, port: u16) {
        self.0.lock().unwrap().applied.push((enabled, port));
    }

    fn save_prefs(&mut self, prefs: &figcraft_engine::Prefs) {
        self.0.lock().unwrap().saved.push(prefs.clone());
    }
}

/// Returns the center position of an AccessKit node's bounding box.
fn centre(n: &Node) -> Pos2 {
    let b = n.bounds().unwrap();
    Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

/// Runs one egui frame at `screen` resolution and collects all AccessKit nodes.
fn nodes_at(u: &mut Ui, screen: egui::Vec2) -> Vec<Node> {
    let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), ..Default::default() };
    let app = &mut u.app;
    let mut out = u.ctx.run_ui(raw, |ui| app.ui(ui));
    out.textures_delta.clear();
    out.platform_output.accesskit_update.unwrap().nodes.into_iter().map(|(_, n)| n).collect()
}

/// Finds all AccessKit nodes whose label equals `label` at 1440×900.
fn named(u: &mut Ui, label: &str) -> Vec<Node> {
    nodes_at(u, vec2(1440.0, 900.0)).into_iter().filter(|n| n.label() == Some(label)).collect()
}

/// Returns whether any AccessKit node label or value contains `text` at 1440×900.
fn shows(u: &mut Ui, text: &str) -> bool {
    nodes_at(u, vec2(1440.0, 900.0)).iter().any(|n| n.label().or(n.value()).is_some_and(|l| l.contains(text)))
}

/// Simulates a primary mouse press and release at `at`.
fn click(u: &mut Ui, at: Pos2) {
    u.button(at, true);
    u.button(at, false);
}

/// Verifies that the MCP tab and standalone MCP dialog fit within both standard and narrow viewports.
#[test]
fn mcp_tab_and_window_fit_standard_and_narrow_viewports() {
    for screen in [vec2(1440.0, 900.0), vec2(340.0, 600.0)] {
        let mut u = Ui::new(Session::new());
        u.ctx.enable_accesskit();
        u.app.ui.plugins_window = true;
        u.app.ui.plugins_manage.tab = PluginsTab::Mcp;
        for _ in 0..3 {
            let _ = nodes_at(&mut u, screen);
        }
        let nodes = nodes_at(&mut u, screen);
        let win = u.ctx.memory(|m| m.area_rect(Id::unique("Plugins"))).expect("Plugins window rect");
        assert!(win.min.x >= -1.0 && win.max.x <= screen.x + 1.0, "Plugins window {win:?} stays inside screen {screen:?}");

        for label in [
            "Enable local MCP control server",
            "Copy Command",
            "Configure All",
            "Copy Config",
            "Antigravity",
            "Claude Code",
            "Claude Desktop",
            "Codex",
        ] {
            let matching: Vec<&Node> = nodes.iter().filter(|n| n.label().or(n.value()) == Some(label) && n.bounds().is_some()).collect();
            assert!(!matching.is_empty(), "{label} is rendered on {screen:?}");
            for n in matching {
                let b = n.bounds().unwrap();
                assert!(
                    b.x0 as f32 >= win.min.x - 2.0 && b.x1 as f32 <= win.max.x + 2.0,
                    "{label} bounds {b:?} overflow window {win:?} on screen {screen:?}"
                );
            }
        }

        // Also verify the standalone Local MCP Server dialog on the same viewport.
        u.app.ui.plugins_window = false;
        u.app.ui.mcp_window = true;
        for _ in 0..3 {
            let _ = nodes_at(&mut u, screen);
        }
        let mcp_nodes = nodes_at(&mut u, screen);
        let mcp_win = u.ctx.memory(|m| m.area_rect(Id::unique("Local MCP Server"))).expect("Local MCP Server window rect");
        assert!(mcp_win.min.x >= -1.0 && mcp_win.max.x <= screen.x + 1.0, "Local MCP Server window {mcp_win:?} stays inside screen {screen:?}");
        for label in ["Copy Command", "Configure All", "Copy Config"] {
            let matching: Vec<&Node> = mcp_nodes.iter().filter(|n| n.label().or(n.value()) == Some(label) && n.bounds().is_some()).collect();
            assert!(!matching.is_empty(), "{label} is rendered in standalone dialog on {screen:?}");
            for n in matching {
                let b = n.bounds().unwrap();
                assert!(
                    b.x0 as f32 >= mcp_win.min.x - 2.0 && b.x1 as f32 <= mcp_win.max.x + 2.0,
                    "{label} bounds {b:?} overflow standalone window {mcp_win:?} on screen {screen:?}"
                );
            }
        }
    }
}

/// Verifies toggling the MCP server, resetting the port, and copying the CLI command from the MCP tab.
#[test]
fn mcp_tab_toggles_server_resets_port_and_copies_command() {
    let log = Arc::new(Mutex::new(McpLog::default()));
    let mut u = Ui::new(Session::new());
    u.app.services = Box::new(MockMcpServices(log.clone()));
    u.app.ui.plugins_window = true;
    u.app.ui.plugins_manage.tab = PluginsTab::Installed;
    u.ctx.enable_accesskit();
    u.frame(vec![]);

    // Switch to the MCP tab via the tab strip.
    let mcp_tab = centre(&named(&mut u, "MCP")[0]);
    click(&mut u, mcp_tab);
    assert_eq!(u.app.ui.plugins_manage.tab, PluginsTab::Mcp);
    assert!(shows(&mut u, "Listening on 127.0.0.1:47800"));

    // Toggle the local MCP server on (from its opt-in `false` default) and verify Services::mcp_server_apply and save_prefs run.
    assert!(!u.app.session.prefs.mcp_enabled);
    let toggle = centre(&named(&mut u, "Enable local MCP control server")[0]);
    click(&mut u, toggle);
    assert!(u.app.session.prefs.mcp_enabled);
    assert_eq!(log.lock().unwrap().applied.last(), Some(&(true, 47800)));
    assert_eq!(log.lock().unwrap().saved.last().map(|p| p.mcp_enabled), Some(true));

    // Change the port, then click Reset to restore 47800.
    u.app.run("pref.mcp", json!({"port": 47911}));
    u.frame(vec![]);
    assert_eq!(u.app.session.prefs.mcp_port, 47911);
    let reset = centre(&named(&mut u, "Reset")[0]);
    click(&mut u, reset);
    assert_eq!(u.app.session.prefs.mcp_port, figcraft_engine::DEFAULT_MCP_PORT);

    // Click Copy Command and verify the toast confirmation.
    let copy_cmd = centre(&named(&mut u, "Copy Command")[0]);
    click(&mut u, copy_cmd);
    assert!(u.app.ui.toast.as_ref().is_some_and(|(t, _)| t.contains("Copied MCP command")), "{:?}", u.app.ui.toast);
}

/// Verifies that the Development tab displays and clears plugin execution console logs.
#[test]
fn development_tab_shows_and_clears_plugin_console_logs() {
    use base64::Engine as _;
    let mut s = Session::new();
    let wasm = wat::parse_str(
        r#"(module
            (import "figcraft" "log" (func $log (param i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "hello from dev console")
            (func (export "figcraft_run") (param i32) (result i32)
                i32.const 0
                i32.const 22
                call $log
                i32.const 0))"#,
    )
    .unwrap();
    let manifest = json!({
        "id": "org.example.logger",
        "name": "Logger",
        "version": "1.0.0",
        "main": "l.wasm"
    });
    s.execute(
        "plugin.import",
        &json!({
            "manifest": manifest,
            "wasm": base64::engine::general_purpose::STANDARD.encode(&wasm),
        }),
    )
    .unwrap();
    s.execute("plugin.run", &json!({"id": "org.example.logger"})).unwrap();
    assert_eq!(s.plugins.history.len(), 1);

    let mut u = Ui::new(s);
    u.app.ui.plugins_window = true;
    u.app.ui.plugins_manage.tab = PluginsTab::Development;
    u.ctx.enable_accesskit();
    u.frame(vec![]);

    assert!(shows(&mut u, "hello from dev console"));
    let clear = centre(&named(&mut u, "Clear")[0]);
    click(&mut u, clear);
    assert!(u.app.session.plugins.history.is_empty());
}
