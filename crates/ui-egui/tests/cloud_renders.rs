//! Offscreen renders of the CollabCraft pieces in FigCraft, for review.
//!
//! Skipped unless `FIGCRAFT_CLOUD_RENDERS` names a directory. Starting wgpu
//! in ordinary CI would buy nothing; the behaviour is covered elsewhere.

use std::sync::{Arc, Mutex};

use figcraft_engine::Session;
use figcraft_ui_egui::cloud::{self, Account, Capability, CloudBackend, Entry, EntryKind, Event, Fault, Job, Post};
use figcraft_ui_egui::home::Section;
use figcraft_ui_egui::{FigApp, NoServices};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, FigApp>;

const SETTLE: usize = 8;

struct State {
    files: Vec<Entry>,
    shared: Vec<Entry>,
}

struct Service(Arc<Mutex<State>>);

impl CloudBackend for Service {
    fn submit(&self, job: Job, post: Post) {
        let state = self.0.lock().expect("state");
        match job {
            Job::Account => post.send(Event::Account(Ok(Account {
                display: "Ada Lovelace".to_owned(),
                capabilities: vec![Capability::Files, Capability::SharedWithMe, Capability::Sharing, Capability::Etags],
                provider_linked: true,
            }))),
            Job::List(path) => {
                let entries = state.files.iter().filter(|e| parent(&e.path) == path).cloned().collect();
                post.send(Event::Listed { path, entries: Ok(entries) });
            }
            Job::SharedWithMe => post.send(Event::Shared(Ok(state.shared.clone()))),
            Job::SignOut => {}
            Job::LinkStart => post.send(Event::Browser { url: "https://cloud.example/link".to_owned(), handle: Some("h".to_owned()) }),
            Job::LinkPoll(_) => post.send(Event::Linked(Ok(true))),
            Job::Read { entry, into, .. } => post.send(Event::Read { entry, into, bytes: Err(Fault::Refused("not for a render".to_owned())) }),
            Job::Write { uid, .. } | Job::Create { uid, .. } => {
                post.send(Event::Saved { uid, entry: Err(Fault::Refused("not for a render".to_owned())) });
            }
            Job::Conversation(_) => post.send(Event::Conversation(Err(Fault::Refused("not for a render".to_owned())))),
        }
    }
}

struct Sharing;

impl craft_ui::share::ShareBackend for Sharing {
    fn submit(&self, request: craft_ui::share::Request, reply: craft_ui::share::Replier) {
        use craft_cloud::{Contact, Permission, Share, ShareTarget};
        use craft_ui::share::{Answer, Request};
        let person = |id: &str, name: &str, email: &str| Contact {
            id: id.to_owned(),
            target: ShareTarget::User(id.to_owned()),
            name: name.to_owned(),
            email: Some(email.to_owned()),
            avatar: None,
            is_group: false,
        };
        match request {
            Request::Capabilities => reply.answer(Answer::Capabilities {
                capabilities: vec![Capability::Sharing, Capability::Contacts, Capability::LinkShares],
                linked: true,
            }),
            Request::Destinations => reply.answer(Answer::Destinations(vec![craft_ui::share::Destination {
                id: "collabcraft".to_owned(),
                name: "CollabCraft".to_owned(),
                account: Some("Ada Lovelace".to_owned()),
                linked: true,
            }])),
            Request::Lookup { query, .. } => {
                let everybody = [person("dana", "Dana Okafor", "dana@example.org"), person("priya", "Priya Raman", "priya@example.org")];
                let want = query.to_lowercase();
                reply.answer(Answer::Lookup(everybody.into_iter().filter(|c| c.name.to_lowercase().contains(&want)).collect()));
            }
            Request::Access { .. } => reply.answer(Answer::Access(vec![
                Share {
                    id: "s1".to_owned(),
                    target: ShareTarget::User("dana".to_owned()),
                    display: Some("Dana Okafor".to_owned()),
                    permission: Permission::Edit,
                    url: None,
                    note: None,
                    created: None,
                },
                Share {
                    id: "s2".to_owned(),
                    target: ShareTarget::Link,
                    display: None,
                    permission: Permission::View,
                    url: Some("https://cloud.example/s/abc123".to_owned()),
                    note: None,
                    created: None,
                },
            ])),
            other => reply.refuse(format!("{other:?} is not offered here")),
        }
    }
}

fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) | None => String::new(),
        Some(cut) => path[..cut].to_owned(),
    }
}

fn file_entry(path: &str, shared_by: Option<&str>) -> Entry {
    Entry {
        id: path.to_owned(),
        room: format!("room:{path}"),
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        path: path.to_owned(),
        kind: EntryKind::File,
        size: Some(1024),
        modified: Some("2026-10-01T09:00:00Z".to_owned()),
        mime: Some("application/vnd.figcraft+zip".to_owned()),
        etag: Some("v1".to_owned()),
        writable: true,
        shared_by: shared_by.map(str::to_owned),
    }
}

fn folder_entry(path: &str) -> Entry {
    Entry { kind: EntryKind::Folder, size: None, etag: None, mime: None, ..file_entry(path, None) }
}

fn furnished() -> State {
    State {
        files: vec![
            folder_entry("/Campaigns"),
            file_entry("/Landing.figcraft", None),
            file_entry("/Icons.figcraft", None),
            file_entry("/Campaigns/Poster.figcraft", None),
        ],
        shared: vec![file_entry("/shared/Brand.figcraft", Some("Dana"))],
    }
}

fn window(home_online: bool) -> Harness {
    let store = Arc::new(Mutex::new(furnished()));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_pixels_per_point(2.0).wgpu().build_ui_state(
        |ui, app: &mut FigApp| {
            app.ui(ui);
        },
        {
            let mut session = Session::new();
            session.prefs.getting_started_dismissed = true;
            let mut app = FigApp::new(session, Box::new(NoServices));
            app.cloud.connect(Service(store));
            app.share.connect(Sharing);
            cloud::start(&mut app);
            if home_online {
                app.ui.home = true;
                app.ui.home_view.section = Section::Online;
            } else {
                let _ = app.run("file.new", json!({}));
                app.ui.home = false;
            }
            app
        },
    );
    h.run_steps(SETTLE);
    h
}

fn save(h: &mut Harness, into: &std::path::Path, name: &str) {
    h.run_steps(SETTLE);
    let image = h.render().expect("an offscreen renderer is required to write renders");
    let path = into.join(format!("figcraft-{name}.png"));
    image.save(&path).expect("the render directory must be writable");
    eprintln!("wrote {}", path.display());
}

#[test]
fn renders_for_review() {
    let Ok(into) = std::env::var("FIGCRAFT_CLOUD_RENDERS") else {
        eprintln!("set FIGCRAFT_CLOUD_RENDERS to a directory to write renders");
        return;
    };
    let into = std::path::PathBuf::from(into);
    std::fs::create_dir_all(&into).expect("the render directory must be writable");

    let mut h = window(true);
    save(&mut h, &into, "online-cloud");

    let mut h = window(false);
    save(&mut h, &into, "editor-account");

    let mut h = window(false);
    h.state_mut().share.open("Landing.figcraft", "Landing");
    save(&mut h, &into, "share-dialog");
}
