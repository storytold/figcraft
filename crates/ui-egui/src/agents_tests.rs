//! Preferences ▸ AI agents and the agent parts of the control channel, driven through egui
//! headless with a stand-in for the app's server: the empty state turns MCP on, the client list
//! installs, a busy port offers a free one, the tab strip shows who's connected, agents' changes
//! are listed and saved, and a hidden window still answers (#269).

use std::cell::RefCell;
use std::rc::Rc;

use egui::accesskit::{Node, Role};
use egui::{Event, PointerButton, Pos2};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::agents::{AgentHost, Client, ClientState, ServerStatus, pretty_client};
use crate::dialogs::PrefsTab;
use crate::layers_drag_tests::Ui;
use crate::{ControlReply, ControlRequest, Services};

#[derive(Default)]
struct Host {
    status: ServerStatus,
    clients: Vec<Client>,
    calls: Vec<String>,
    /// Times the clients' settings were read.
    reads: usize,
}

#[derive(Clone, Default)]
struct Fake(Rc<RefCell<Host>>);

impl AgentHost for Fake {
    fn status(&self) -> ServerStatus {
        self.0.borrow().status.clone()
    }
    fn start(&mut self, port: u16) -> Result<(), String> {
        let mut h = self.0.borrow_mut();
        h.calls.push(format!("start {port}"));
        h.status = ServerStatus { listening: Some(port), ..Default::default() };
        Ok(())
    }
    fn stop(&mut self) {
        let mut h = self.0.borrow_mut();
        h.calls.push("stop".into());
        h.status = ServerStatus::default();
    }
    fn disconnect(&mut self) {
        self.0.borrow_mut().calls.push("disconnect".into());
    }
    fn free_port(&self, from: u16) -> Option<u16> {
        Some(from)
    }
    fn clients(&mut self, _port: u16) -> Vec<Client> {
        self.0.borrow_mut().reads += 1;
        self.0.borrow().clients.clone()
    }
    fn install(&mut self, id: &str, port: u16) -> Result<String, String> {
        self.0.borrow_mut().calls.push(format!("install {id} {port}"));
        Ok("installed".into())
    }
    fn uninstall(&mut self, id: &str) -> Result<(), String> {
        self.0.borrow_mut().calls.push(format!("uninstall {id}"));
        Ok(())
    }
    fn config_json(&self, port: u16) -> String {
        format!("{{\"port\": {port}}}")
    }
}

struct WithAgents {
    host: Fake,
    copied: Rc<RefCell<String>>,
}

impl Services for WithAgents {
    fn agents(&mut self) -> Option<&mut dyn AgentHost> {
        Some(&mut self.host)
    }
    fn clipboard_set_text(&mut self, s: &str) {
        *self.copied.borrow_mut() = s.to_string();
    }
}

fn setup(host: Host) -> (Ui, Fake, Rc<RefCell<String>>) {
    let fake = Fake(Rc::new(RefCell::new(host)));
    let copied = Rc::new(RefCell::new(String::new()));
    let mut u = Ui::new(Session::new());
    u.app.services = Box::new(WithAgents { host: fake.clone(), copied: copied.clone() });
    u.app.ui.preferences = true;
    u.app.ui.prefs_tab = PrefsTab::Agents;
    u.frame(vec![]);
    (u, fake, copied)
}

fn tree(u: &mut Ui) -> Vec<Node> {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    out.platform_output.accesskit_update.expect("an AccessKit update").nodes.into_iter().map(|(_, n)| n).collect()
}

fn named(u: &mut Ui, label: &str) -> Option<Node> {
    tree(u).into_iter().find(|n| n.label() == Some(label) || n.value() == Some(label))
}

/// Click the control named `label`, as a pointer would.
fn click(u: &mut Ui, label: &str) {
    let node = named(u, label).unwrap_or_else(|| panic!("nothing named {label:?} on screen"));
    let b = node.bounds().unwrap_or_else(|| panic!("{label:?} has no bounds"));
    let at = Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    u.frame(vec![Event::PointerMoved(at)]);
    u.frame(vec![button(true)]);
    u.frame(vec![button(false)]);
    u.frame(vec![]);
}

fn client(id: &'static str, name: &'static str, state: ClientState) -> Client {
    Client { id, name, state, detail: format!("~/.{id}") }
}

#[test]
fn the_empty_state_turns_mcp_on_and_remembers_it() {
    let (mut u, fake, _) = setup(Host::default());
    assert!(named(&mut u, "Let an AI agent work on your file").is_some(), "the empty state explains it first");
    click(&mut u, "Turn on MCP");
    assert_eq!(fake.0.borrow().calls, vec!["start 7979"]);
    assert!(u.app.session.prefs.agents.enabled, "and turns on with FigCraft from now on");
    assert!(named(&mut u, "MCP is on").is_some());
    assert!(named(&mut u, "Waiting for an agent. After installing, open the client and ask: “Use FigCraft to create a 1440 × 900 frame.”").is_some());
}

#[test]
fn clients_install_remove_and_copy_their_settings() {
    let host = Host {
        status: ServerStatus { listening: Some(7979), ..Default::default() },
        clients: vec![
            client("claude-code", "Claude Code", ClientState::Available),
            client("claude-desktop", "Claude Desktop", ClientState::Missing),
            client("cursor", "Cursor", ClientState::Installed { same_port: true }),
        ],
        ..Default::default()
    };
    let (mut u, fake, copied) = setup(host);
    click(&mut u, "Install");
    assert_eq!(fake.0.borrow().calls.last().map(String::as_str), Some("install claude-code 7979"));
    click(&mut u, "Remove");
    assert_eq!(fake.0.borrow().calls.last().map(String::as_str), Some("uninstall cursor"));
    click(&mut u, "Copy settings");
    assert_eq!(*copied.borrow(), "{\"port\": 7979}");
    // A client set up for another port (seen once the tab reads the settings again) asks for an update.
    fake.0.borrow_mut().clients = vec![client("cursor", "Cursor", ClientState::Installed { same_port: false })];
    u.app.ui.agents.clients = None;
    u.frame(vec![]);
    click(&mut u, "Update port");
    assert_eq!(fake.0.borrow().calls.last().map(String::as_str), Some("install cursor 7979"));
}

#[test]
fn the_clients_settings_are_read_when_the_tab_opens_and_after_a_change_not_every_frame() {
    let host = Host {
        status: ServerStatus { listening: Some(7979), ..Default::default() },
        clients: vec![client("cursor", "Cursor", ClientState::Available)],
        ..Default::default()
    };
    let (mut u, fake, _) = setup(host);
    let reads = || fake.0.borrow().reads;
    let first = reads();
    for _ in 0..5 {
        u.frame(vec![]);
    }
    assert_eq!(reads(), first, "open and idle: read once");
    click(&mut u, "Install");
    u.frame(vec![]);
    assert_eq!(reads(), first + 1, "an install reads them again");
    // Closed and opened again: read again (another app may have changed them meanwhile).
    u.app.ui.preferences = false;
    u.frame(vec![]);
    u.app.ui.preferences = true;
    u.frame(vec![]);
    assert_eq!(reads(), first + 2);
}

#[test]
fn the_options_set_the_preferences_and_the_switch_stops_the_server() {
    let host = Host { status: ServerStatus { listening: Some(7979), ..Default::default() }, ..Default::default() };
    let (mut u, fake, _) = setup(host);
    click(&mut u, "Save files automatically");
    assert!(u.app.session.prefs.auto_save, "the same auto-save as General's");
    click(&mut u, "Turn on when FigCraft starts");
    assert!(u.app.session.prefs.agents.enabled);
    click(&mut u, "MCP");
    assert_eq!(fake.0.borrow().calls.last().map(String::as_str), Some("stop"));
}

#[test]
fn a_busy_port_says_so_and_offers_a_free_one() {
    let host = Host {
        status: ServerStatus { error: Some("Address already in use (os error 48)".into()), error_port: Some(7979), ..Default::default() },
        ..Default::default()
    };
    let (mut u, fake, _) = setup(host);
    assert!(named(&mut u, "Port 7979 is already in use by another app on this computer.").is_some());
    click(&mut u, "Use port 7980");
    assert_eq!(fake.0.borrow().calls, vec!["start 7980"]);
    assert_eq!(u.app.session.prefs.agents.port, 7980);
}

#[test]
fn the_tab_strip_shows_the_connected_client_and_opens_its_settings() {
    let host = Host { status: ServerStatus { listening: Some(7979), connections: 1, ..Default::default() }, ..Default::default() };
    let (mut u, fake, _) = setup(host);
    u.app.ui.preferences = false;
    let ctx = u.ctx.clone();
    crate::control::handle(&mut u.app, &ctx, "app.hello", &json!({"client": "claude-code", "version": "2.1.0"})).unwrap();
    // The pill opens a popover: what the agent did, its settings, Disconnect.
    click(&mut u, "Claude Code");
    assert!(u.app.ui.agents.popover);
    assert!(named(&mut u, "Claude Code is editing this file").is_some());
    click(&mut u, "Open AI agents");
    assert!(u.app.ui.preferences && u.app.ui.prefs_tab == PrefsTab::Agents && !u.app.ui.agents.popover);
    assert!(named(&mut u, "Claude Code is connected").is_some());
    click(&mut u, "Disconnect");
    assert_eq!(fake.0.borrow().calls.last().map(String::as_str), Some("disconnect"));
}

#[test]
fn agent_commands_that_change_the_file_are_listed_and_reads_are_not() {
    let (mut u, _, _) = setup(Host { status: ServerStatus { listening: Some(7979), connections: 1, ..Default::default() }, ..Default::default() });
    let ctx = u.ctx.clone();
    let exec = |u: &mut Ui, command: &str, params: Value| {
        crate::control::handle(&mut u.app, &ctx, "engine.execute", &json!({"command": command, "params": params}))
    };
    exec(&mut u, "node.create", json!({"type": "frame", "name": "Hero"})).unwrap();
    assert_eq!(u.app.ui.agents.commands, 1);
    let a = u.app.ui.agents.recent.front().unwrap();
    assert_eq!(a.command, "node.create");
    assert!(a.detail.contains("“Hero”"), "{}", a.detail);
    // Reading changes nothing, so it isn't listed.
    exec(&mut u, "inspect.outline", json!({})).unwrap();
    exec(&mut u, "inspect.lint", json!({})).unwrap();
    assert_eq!(u.app.ui.agents.commands, 1);
}

#[test]
fn without_a_server_the_tab_says_why() {
    let mut u = Ui::new(Session::new());
    u.app.ui.preferences = true;
    u.app.ui.prefs_tab = PrefsTab::Agents;
    let t = tree(&mut u);
    assert!(t.iter().any(|n| n.label().or(n.value()).is_some_and(|l| l.contains("can't listen"))), "the browser build explains it");
    let role = Role::Button;
    assert!(!t.iter().any(|n| n.role() == role && n.label() == Some("Turn on MCP")));
}

#[test]
fn client_names_read_as_people_know_them() {
    assert_eq!(pretty_client("claude-code").as_deref(), Some("Claude Code"));
    assert_eq!(pretty_client("cursor-vscode").as_deref(), Some("Cursor"));
    assert_eq!(pretty_client("my-agent").as_deref(), Some("my-agent"));
    assert_eq!(pretty_client("  "), None);
}

#[test]
fn only_a_client_that_said_hello_counts_as_an_agent_and_is_forgotten_when_it_leaves() {
    let host = Host { status: ServerStatus { listening: Some(7979), connections: 1, ..Default::default() }, ..Default::default() };
    let (mut u, fake, _) = setup(host);
    // A script on the control channel: connected, but no agent to show.
    assert!(named(&mut u, "MCP is on").is_some(), "a connection without hello isn't an agent");
    u.app.ui.preferences = false;
    assert!(named(&mut u, "Claude Code").is_none());
    let ctx = u.ctx.clone();
    crate::control::handle(&mut u.app, &ctx, "app.hello", &json!({"client": "claude-code", "version": "2"})).unwrap();
    assert!(named(&mut u, "Claude Code").is_some(), "the tab strip shows it once it says hello");
    fake.0.borrow_mut().status.connections = 0;
    u.frame(vec![]);
    assert!(named(&mut u, "Claude Code").is_none());
    assert!(u.app.ui.agents.client.is_none(), "forgotten: the next connection may be someone else");
}

#[test]
fn a_delayed_screenshot_is_only_asked_for_when_its_time_comes() {
    let (mut u, _, _) = setup(Host::default());
    let (tx, _rx) = std::sync::mpsc::channel::<ControlReply>();
    u.app.control_queue.push(ControlRequest {
        id: json!(1),
        method: "ui.screenshot".into(),
        params: json!({"path": "x.png", "after": 0.5}),
        reply: tx,
        cancelled: Default::default(),
    });
    u.frame(vec![]);
    let asked = |u: &Ui| crate::control::pending(&u.ctx).lock().unwrap().iter().map(|p| p.asked).collect::<Vec<_>>();
    assert_eq!(asked(&u), vec![false], "not yet");
    for _ in 0..40 {
        u.frame(vec![]);
    }
    assert_eq!(asked(&u), vec![true], "asked once half a second has passed");
}
