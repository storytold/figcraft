//! The Manage plugins window's Browse tab, driven headless against a scripted directory: Install
//! only asks (it shows the permissions, nothing is downloaded), the confirmation's Install installs,
//! and the installed plugin's row then offers Remove. Against a directory that answers later (as the
//! network does), Browse and Install show that they're on their way and can be cancelled, the
//! window keeps working meanwhile, and the installed plugin is saved at once.

use std::sync::{Arc, Mutex};

use egui::Pos2;
use figcraft_engine::Session;
use figcraft_engine::figcraft_collab::message::{ClientMsg, PluginEntry, PluginReply, PluginReq};
use figcraft_engine::live::{Event, HostOptions, LiveLink, NetReply, NetRequest, Presence, SessionInfo};
use figcraft_engine::plugin_directory::bundle::{Bundle, sha256_hex};
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::plugins_ui::PluginsTab;

const ID: &str = "org.example.grid";

struct Link(Arc<Mutex<usize>>, Vec<u8>);

/// The scripted directory's answer to `req` for the bundle `bytes`.
fn reply(bytes: &[u8], req: PluginReq) -> PluginReply {
    let entry = PluginEntry {
        id: ID.into(),
        name: "Grid maker".into(),
        version: "1.0.0".into(),
        author: "Ada".into(),
        access: "write".into(),
        commands: vec!["shape.rectangle".into()],
        bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
        ..PluginEntry::default()
    };
    match req {
        PluginReq::List { .. } => PluginReply::List(vec![entry]),
        PluginReq::Fetch { .. } => PluginReply::Bundle { entry, bundle: bytes.to_vec() },
        _ => PluginReply::Failed { reason: "no".into() },
    }
}

impl LiveLink for Link {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("no".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Err("no".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, Event)> {
        Vec::new()
    }
    fn plugins(&mut self, _: &str, req: PluginReq) -> Result<PluginReply, String> {
        if matches!(req, PluginReq::Fetch { .. }) {
            *self.0.lock().unwrap() += 1;
        }
        Ok(reply(&self.1, req))
    }
}

/// A directory that answers later: the requests out and the answers handed back.
#[derive(Default)]
struct Wire {
    out: Vec<(u64, PluginReq)>,
    answers: Vec<Event>,
}

struct Later(Arc<Mutex<Wire>>);

impl LiveLink for Later {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("no".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Err("no".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, Event)> {
        std::mem::take(&mut self.0.lock().unwrap().answers).into_iter().map(|e| (0, e)).collect()
    }
    fn plugins(&mut self, _: &str, _: PluginReq) -> Result<PluginReply, String> {
        Err("the window waited for the directory".into())
    }
    fn background(&mut self, job: u64, req: NetRequest) -> bool {
        if let NetRequest::Plugins { req, .. } = req {
            self.0.lock().unwrap().out.push((job, req));
        }
        true
    }
}

/// Answer the requests out, as the network would; how many there were.
fn deliver(wire: &Arc<Mutex<Wire>>, bytes: &[u8]) -> usize {
    let mut w = wire.lock().unwrap();
    let out = std::mem::take(&mut w.out);
    for (job, req) in &out {
        w.answers.push(Event::Net { job: *job, reply: Box::new(NetReply::Plugins(Ok(reply(bytes, req.clone())))) });
    }
    out.len()
}

/// Services that keep what the app saved.
struct Saved(Arc<Mutex<Vec<figcraft_engine::Prefs>>>);

impl crate::Services for Saved {
    fn save_prefs(&mut self, prefs: &figcraft_engine::Prefs) {
        self.0.lock().unwrap().push(prefs.clone());
    }
}

fn centre(n: &egui::accesskit::Node) -> Pos2 {
    let b = n.bounds().unwrap();
    Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

/// Every node labelled `label`, in tree order.
fn named(u: &mut Ui, label: &str) -> Vec<egui::accesskit::Node> {
    let tree = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    tree.into_iter().map(|(_, n)| n).filter(|n| n.label() == Some(label)).collect()
}

/// Some node's label (a text's value) holds `text`.
fn shows(u: &mut Ui, text: &str) -> bool {
    let tree = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    tree.iter().any(|(_, n)| n.label().or(n.value()).is_some_and(|l| l.contains(text)))
}

fn click(u: &mut Ui, at: Pos2) {
    u.button(at, true);
    u.button(at, false);
}

fn bundle() -> Vec<u8> {
    let wasm =
        wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) i32.const 0))"#).unwrap();
    let manifest = json!({"id": ID, "name": "Grid maker", "version": "1.0.0", "main": "g.wasm", "permissions": {"access": "write", "commands": ["shape.rectangle"]}});
    Bundle { manifest: manifest.to_string(), wasm, assets: Vec::new() }.encode()
}

/// The Browse button: the lower of the two "Browse" (the tab is above it).
fn browse_button(u: &mut Ui) -> Pos2 {
    named(u, "Browse").iter().map(centre).max_by(|a, b| a.y.total_cmp(&b.y)).unwrap()
}

#[test]
fn install_asks_first_and_then_installs() {
    let bytes = bundle();
    let fetched = Arc::new(Mutex::new(0));
    let mut s = Session::new();
    s.live.install(Box::new(Link(fetched.clone(), bytes)));
    s.execute("plugin.directory.list", &json!({"server": "ws://127.0.0.1:9/"})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.plugins_window = true;
    u.app.ui.plugins_manage.tab = PluginsTab::Browse;
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let row = named(&mut u, "Install");
    assert_eq!(row.len(), 1, "the plugin's row offers Install");
    let row_at = centre(&row[0]);
    click(&mut u, row_at);
    assert_eq!(u.app.ui.plugins_manage.confirm.as_deref(), Some(ID), "Install asks first");
    assert!(u.app.session.prefs.plugins.is_empty());
    assert_eq!(*fetched.lock().unwrap(), 0, "nothing is downloaded before the user confirms");
    u.frame(vec![]);
    for shown in ["read and edit the document", "call only: shape.rectangle", "From ws://127.0.0.1:9/"] {
        assert!(shows(&mut u, shown), "the confirmation shows {shown:?}");
    }
    // An API 4 plugin may ask to save files: the confirmation says so.
    let save = "offer files to save";
    assert!(!shows(&mut u, save));
    u.app.session.plugins.directory.as_mut().unwrap().entries[0].api = 4;
    assert!(shows(&mut u, save), "an API 4 plugin's confirmation mentions saving files");
    let confirm = named(&mut u, "Install").into_iter().map(|n| centre(&n)).find(|p| p.distance(row_at) > 4.0).unwrap();
    click(&mut u, confirm);
    assert_eq!(*fetched.lock().unwrap(), 1);
    assert_eq!(u.app.session.prefs.plugins.len(), 1, "installed");
    assert_eq!(u.app.ui.plugins_manage.confirm, None);
    u.frame(vec![]);
    assert!(named(&mut u, "Install").is_empty() && !named(&mut u, "Remove").is_empty(), "the row now offers Remove");
}

#[test]
fn browsing_and_installing_show_progress_and_never_wait() {
    let (wire, bytes) = (Arc::new(Mutex::new(Wire::default())), bundle());
    let mut s = Session::new();
    s.live.install(Box::new(Later(wire.clone())));
    let mut u = Ui::new(s);
    let saved = Arc::new(Mutex::new(Vec::new()));
    u.app.services = Box::new(Saved(saved.clone()));
    u.app.ui.plugins_window = true;
    u.app.ui.plugins_manage.tab = PluginsTab::Browse;
    u.app.ui.plugins_manage.server = Some("ws://127.0.0.1:9/".into());
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let browse = browse_button(&mut u);
    click(&mut u, browse);
    assert!(u.app.session.waiting_for("plugin.directory.list"));
    assert!(shows(&mut u, "Fetching from the server…") && !named(&mut u, "Cancel").is_empty(), "Browse shows it's on its way");
    // Cancel stops waiting: the late answer is dropped.
    let cancel = centre(&named(&mut u, "Cancel")[0]);
    click(&mut u, cancel);
    assert_eq!(deliver(&wire, &bytes), 1);
    u.frame(vec![]);
    assert!(u.app.session.plugins.directory.is_none() && !u.app.session.waiting_for("plugin.directory.list"));
    // Browse again; the answer arrives and the listing shows.
    let browse = browse_button(&mut u);
    click(&mut u, browse);
    assert_eq!(deliver(&wire, &bytes), 1);
    u.frame(vec![]);
    let row = named(&mut u, "Install");
    assert_eq!(row.len(), 1, "the listing arrived");
    let row_at = centre(&row[0]);
    click(&mut u, row_at);
    u.frame(vec![]);
    let confirm = named(&mut u, "Install").into_iter().map(|n| centre(&n)).find(|p| p.distance(row_at) > 4.0).unwrap();
    click(&mut u, confirm);
    assert!(u.app.session.waiting_for("plugin.directory.install"));
    assert!(shows(&mut u, "Installing…"), "the confirmation shows the install on its way");
    assert!(u.app.ui.plugins_manage.confirm.is_some() && u.app.session.prefs.plugins.is_empty());
    assert!(saved.lock().unwrap().is_empty());
    assert_eq!(deliver(&wire, &bytes), 1);
    u.frame(vec![]);
    assert_eq!(u.app.session.prefs.plugins.len(), 1, "installed when the download arrived");
    assert_eq!(u.app.ui.plugins_manage.confirm, None);
    assert!(u.app.ui.toast.as_ref().is_some_and(|(t, _)| t.contains("Installed Grid maker")), "{:?}", u.app.ui.toast);
    let kept = saved.lock().unwrap();
    assert_eq!(kept.last().map(|p| p.plugins.len()), Some(1), "the plugin list was saved at once, not at quit");
}
