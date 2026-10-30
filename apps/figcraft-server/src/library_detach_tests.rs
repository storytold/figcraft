//! Detach counts in a team library's analytics, end to end over loopback: a file that uses the
//! library detaches instances of its components and its app reports them with its usage; the
//! library's owner sees them beside the instances. Reports are cleaned and capped like usage, kept
//! per file, and the usage list a server wrote before detach counts still reads.

use std::time::{Duration, Instant};

use figcraft_collab::message::{Access, ClientMsg, Detached, FileUsage, LibraryReply, LibraryReq, OpenLibrary, PROTOCOL, ServerMsg, Usage};
use figcraft_collab_net::Invite;
use figcraft_collab_net::hub::Backend;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::hardening_tests::archive;
use crate::store::{FILES, Store};
use crate::test_util::{Raw, TempDir, config, ip, publish, session, snapshot};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// `a`'s analytics of its file's library once `done` holds for them (the reports arrive from
/// another app's background requests).
fn usage_when(a: &mut Session, mut done: impl FnMut(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        let u = ex(a, "library.usage", json!({}));
        if done(&u) {
            return u;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "the reports never arrived: {u}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn answer(raw: &mut Raw) -> LibraryReply {
    match raw.next() {
        ServerMsg::Library(r) => *r,
        other => panic!("not a library answer: {other:?}"),
    }
}

#[test]
fn detaches_are_reported_and_counted_beside_instances() {
    let dir = TempDir::new("library-detach");
    let cfg = config(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    // Ada's library on the server: a "Button".
    let mut a = session("Ada");
    let f = id(&ex(&mut a, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 40, "name": "Button"})));
    ex(&mut a, "component.create", json!({"ids": [f]}));
    let p = publish(&mut a, dir.path(), addr);
    ex(&mut a, "library.publishToServer", json!({}));

    // Bo places two Buttons and detaches one.
    let mut b = session("Bo");
    let lid = ex(&mut b, "library.add", json!({"path": p.view}))["id"].as_str().unwrap().to_string();
    let place = |b: &mut Session, x: i32| id(&ex(b, "library.place", json!({"library": lid, "component": "Button", "x": x, "y": 0})));
    let (one, two) = (place(&mut b, 0), place(&mut b, 200));
    ex(&mut b, "component.detach", json!({"ids": [two]}));
    b.refresh_server_libraries();
    let u = usage_when(&mut a, |u| u["detached"] == 1 && u["instances"] == 1);
    assert_eq!((u["instances"].clone(), u["detachesKnown"].clone()), (json!(1), json!(true)), "{u}");
    assert_eq!(u["components"][0]["name"], "Button", "{u}");
    assert_eq!((u["components"][0]["instances"].clone(), u["components"][0]["detached"].clone()), (json!(1), json!(1)), "{u}");
    assert_eq!(u["files"][0]["detached"], 1, "{u}");

    // Detaching the other one too: no instances left, still listed with its detaches.
    ex(&mut b, "component.detach", json!({"ids": [one]}));
    // Again each time: a check asked for while the last one still runs is dropped.
    let u = usage_when(&mut a, |u| {
        b.refresh_server_libraries();
        u["detached"] == 2 && u["instances"] == 0
    });
    assert_eq!(u["instances"], 0, "{u}");
    assert_eq!((u["components"][0]["name"].clone(), u["components"][0]["detached"].clone()), (json!("Button"), json!(2)), "{u}");
    server.stop();
}

/// Reports from the wire: names cleaned and counts capped as usage is, kept apart per file; a
/// file that only reported detaches isn't listed as using the library; the requests from before
/// detach counts are answered as they were.
#[test]
fn detach_reports_are_capped_per_file_and_older_requests_answer_as_before() {
    let dir = TempDir::new("library-detach-wire");
    let cfg = config(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let mut a = session("Ada");
    let f = id(&ex(&mut a, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 40, "name": "Button"})));
    ex(&mut a, "component.create", json!({"ids": [f]}));
    let p = publish(&mut a, dir.path(), addr);
    ex(&mut a, "library.publishToServer", json!({}));
    let view = Invite::parse(&p.view).unwrap();
    let mut raw = Raw::connect(addr);
    raw.send(&ClientMsg::OpenLibrary(OpenLibrary {
        protocol: PROTOCOL,
        file: view.file.clone(),
        token: view.token.clone(),
        client_version: String::new(),
    }));
    assert_eq!(answer(&mut raw), LibraryReply::Ready { access: Access::View });
    let flood = Detached { file: "\u{7}Spam\n".repeat(100), components: (0..5_000).map(|i| (NodeId(i), 1)).collect() };
    raw.send(&ClientMsg::Library(LibraryReq::ReportDetached(flood)));
    assert_eq!(answer(&mut raw), LibraryReply::Noted);
    raw.send(&ClientMsg::Library(LibraryReq::ReportDetached(Detached {
        file: "Shop".into(),
        components: vec![(NodeId(3), 4), (NodeId(3), 9), (NodeId(5), 0)],
    })));
    assert_eq!(answer(&mut raw), LibraryReply::Noted);
    raw.send(&ClientMsg::Library(LibraryReq::Detached));
    let LibraryReply::Detached(files) = answer(&mut raw) else { panic!("no detaches") };
    let spam = files.iter().find(|f| f.detached.file.starts_with("Spam")).unwrap();
    assert!(spam.detached.file.chars().count() <= 256 && !spam.detached.file.contains('\n'), "names are cleaned and capped");
    assert_eq!(spam.detached.components.len(), 1_000, "reports are capped");
    let shop = files.iter().find(|f| f.detached.file == "Shop").unwrap();
    assert_eq!(shop.detached.components, [(NodeId(3), 4)], "one count per component, none of 0");
    // The usage list (what older apps ask for) is as it was: nobody reported using the library.
    raw.send(&ClientMsg::Library(LibraryReq::Usage));
    assert_eq!(answer(&mut raw), LibraryReply::Usage(Vec::new()));
    server.stop();
}

/// A usage list written before detach counts reads as it did, and gains them beside it.
#[test]
fn a_usage_list_from_before_detach_counts_still_reads() {
    let dir = TempDir::new("library-detach-old");
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    let (file, ..) = store.publish("Library", &snapshot("Library")).unwrap();
    store.publish_library(&file.id, &archive(0, 0)).unwrap();
    let old = vec![FileUsage { usage: Usage { file: "Checkout".into(), components: vec![(NodeId(7), 3)] }, updated: 1_700_000_000 }];
    std::fs::write(dir.path().join(FILES).join(&file.id).join(crate::disk::USAGE), serde_json::to_vec(&old).unwrap()).unwrap();
    drop(store);
    let mut store = Store::open(dir.path(), config(dir.path()).store()).unwrap();
    assert_eq!(store.library_usage(&file.id).unwrap(), old);
    let report = Detached { file: "Checkout".into(), components: vec![(NodeId(7), 2)] };
    let r = store.library(ip("198.51.100.1"), &file.id, Access::View, LibraryReq::ReportDetached(report));
    assert_eq!(r, LibraryReply::Noted);
    assert_eq!(store.library_usage(&file.id).unwrap(), old, "the usage report is untouched");
    let detached = store.library_detached(&file.id).unwrap();
    assert_eq!(detached.len(), 1);
    assert_eq!(detached[0].detached.components, [(NodeId(7), 2)]);
}
