//! A team library's insertions and detachments week by week (library analytics over time): files'
//! apps report their totals, the server counts what grew since in the current week (an undo and a
//! redo count once, a detach total from before weeks were kept starts from that report), keeps a
//! year of weeks across restarts, and the library's owner sees them per component and in all.

use std::time::{Duration, Instant};

use figcraft_collab::message::{Access, Detached, LibraryReply, LibraryReq, WeekActions};
use figcraft_collab_net::hub::Backend;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::hardening_tests::archive;
use crate::store::{FILES, Store};
use crate::test_util::{TempDir, config, ip, publish, session, snapshot};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// `a`'s analytics of its file's library once `done` holds for them.
fn usage_when(a: &mut Session, p: Value, done: impl Fn(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        let u = ex(a, "library.usage", p.clone());
        if done(&u) {
            return u;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "the reports never arrived: {u}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn placing_pasting_and_duplicating_count_as_insertions_this_week() {
    let dir = TempDir::new("library-actions");
    let cfg = config(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let mut a = session("Ada");
    let f = id(&ex(&mut a, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 40, "name": "Button"})));
    ex(&mut a, "component.create", json!({"ids": [f]}));
    let p = publish(&mut a, dir.path(), addr);
    ex(&mut a, "library.publishToServer", json!({}));

    // Bo places a Button, duplicates it, copies and pastes it inside a frame, and detaches one.
    let mut b = session("Bo");
    let lid = ex(&mut b, "library.add", json!({"path": p.view}))["id"].as_str().unwrap().to_string();
    let one = id(&ex(&mut b, "library.place", json!({"library": lid, "component": "Button", "x": 0, "y": 0})));
    let two = ex(&mut b, "edit.duplicate", json!({"ids": [one], "map": true}))["ids"][0].clone();
    let frame = id(&ex(&mut b, "frame.create", json!({"x": 400, "y": 0, "width": 300, "height": 300})));
    ex(&mut b, "edit.copy", json!({"ids": [one]}));
    ex(&mut b, "select.set", json!({"ids": [frame]}));
    ex(&mut b, "edit.paste", json!({}));
    ex(&mut b, "component.detach", json!({"ids": [two]}));
    let counted: Vec<u32> = b.doc().unwrap().libraries.inserted.iter().map(|c| c.count).collect();
    assert_eq!(counted, [3], "placed, duplicated and pasted");
    b.refresh_server_libraries();
    let u = usage_when(&mut a, json!({}), |u| u["insertions"] == 3 && u["detachments"] == 1);
    assert_eq!((u["actionsKnown"].clone(), u["days"].clone()), (json!(true), json!(30)), "{u}");
    let c = &u["components"][0];
    assert_eq!((c["name"].clone(), c["insertions"].clone(), c["detachments"].clone()), (json!("Button"), json!(3), json!(1)), "{u}");
    let weeks = u["weeks"].as_array().unwrap();
    assert_eq!(weeks.len(), 1, "{u}");
    assert_eq!((weeks[0]["insertions"].clone(), weeks[0]["detachments"].clone()), (json!(3), json!(1)));
    let monday = weeks[0]["week"].as_u64().unwrap();
    assert_eq!((monday / 86_400 + 3) % 7, 0, "weeks start on a Monday");
    // A shorter period still holds the current week; reporting again counts nothing twice.
    b.refresh_server_libraries();
    let u = usage_when(&mut a, json!({"days": 1}), |u| u["days"] == 1);
    assert_eq!(u["insertions"], 3, "{u}");
    server.stop();
}

#[test]
fn only_what_grows_past_a_files_highest_totals_counts_and_weeks_are_kept() {
    let dir = TempDir::new("library-actions-store");
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let (file, ..) = store.publish("Library", &snapshot("Library")).unwrap();
    store.publish_library(&file.id, &archive(0, 0)).unwrap();
    // A detach report from before weeks were kept: 2 detached.
    let old = json!([{"usage": {"file": "Shop", "components": []}, "detached": [[7, 2]], "detached_updated": 1_700_000_000}]);
    std::fs::write(dir.path().join(FILES).join(&file.id).join(crate::disk::USAGE), serde_json::to_vec(&old).unwrap()).unwrap();
    drop(store);
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let report = |store: &mut Store, req: fn(Detached) -> LibraryReq, n: u32| {
        let r = store.library(ip("198.51.100.1"), &file.id, Access::View, req(Detached { file: "Shop".into(), components: vec![(NodeId(7), n)] }));
        assert_eq!(r, LibraryReply::Noted);
    };
    // Inserted 5, then an undo (4) and a redo (5): 5 in all; then one more: 6.
    for n in [5, 4, 5, 6] {
        report(&mut store, LibraryReq::ReportInserted, n);
    }
    // Detached 3: one more than the old report.
    report(&mut store, LibraryReq::ReportDetached, 3);
    let weeks = store.library_actions(&file.id).unwrap();
    let only = |w: &[WeekActions]| w.iter().map(|w| (w.component, w.insertions, w.detachments)).collect::<Vec<_>>();
    assert_eq!(only(&weeks), [(NodeId(7), 6, 1)]);
    assert_eq!(store.library_detached(&file.id).unwrap()[0].detached.components, [(NodeId(7), 3)], "the detach total shown");
    // Kept across a restart.
    Backend::stop(&mut store, Vec::new());
    drop(store);
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    assert_eq!(store.library_actions(&file.id).unwrap(), weeks);
    // A damaged weeks file starts over instead of failing.
    std::fs::write(dir.path().join(FILES).join(&file.id).join(crate::disk::ACTIONS), b"{not json").unwrap();
    drop(store);
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    assert!(store.library_actions(&file.id).unwrap().is_empty());
}
