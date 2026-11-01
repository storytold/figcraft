//! Team libraries end to end over loopback, with the server in-process on a temporary data
//! directory: a file publishes its library to the server, another file enables it with a view
//! link, places a component and overrides it; the library is republished with a change, the other
//! file's app notices in the background and accepts the update (the override stays); usage counts;
//! view links can't publish; junk library requests are refused and the server keeps serving.

use figcraft_collab::message::{Access, ClientMsg, LibraryReply, LibraryReq, OpenLibrary, PROTOCOL, ServerMsg, Usage};
use figcraft_collab::sim::Rng;
use figcraft_collab_net::Invite;
use figcraft_collab_net::hub::Backend;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::store::Store;
use crate::test_util::{Raw, TempDir, config, publish, pump, session};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    match s.execute(cmd, &p) {
        Ok(v) => v,
        Err(e) => panic!("{cmd} {p}: {e}"),
    }
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn fill(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

fn open(link: &Invite) -> ClientMsg {
    ClientMsg::OpenLibrary(OpenLibrary { protocol: PROTOCOL, file: link.file.clone(), token: link.token.clone(), client_version: String::new() })
}

fn answer(raw: &mut Raw) -> LibraryReply {
    match raw.next() {
        ServerMsg::Library(r) => *r,
        other => panic!("not a library answer: {other:?}"),
    }
}

/// Junk from an edit link is refused and the library unharmed; a wrong token isn't let in.
fn junk_is_refused(addr: std::net::SocketAddr, edit: &Invite) {
    let mut raw = Raw::connect(addr);
    raw.send(&open(edit));
    assert_eq!(answer(&mut raw), LibraryReply::Ready { access: Access::Edit });
    raw.send(&ClientMsg::Library(LibraryReq::Fetch { have: None, usage: None }));
    let LibraryReply::Library { version: 2, archive } = answer(&mut raw) else { panic!("no library") };
    let mut rng = Rng::new(5);
    for round in 0..12 {
        let mut junk = if round % 3 == 0 { vec![0x5A; 1 + rng.below(4096)] } else { archive.clone() };
        for _ in 0..1 + rng.below(8) {
            let i = rng.below(junk.len());
            junk[i] = rng.next_u64() as u8;
        }
        raw.send(&ClientMsg::Library(LibraryReq::Publish { archive: junk }));
        assert!(matches!(answer(&mut raw), LibraryReply::Failed { .. }), "round {round}");
    }
    let flood = Usage { file: "\u{7}Spam\n".repeat(100), components: (0..5_000).map(|i| (NodeId(i), 1)).collect() };
    raw.send(&ClientMsg::Library(LibraryReq::Fetch { have: Some(2), usage: Some(flood) }));
    assert_eq!(answer(&mut raw), LibraryReply::Unchanged { version: 2 });
    raw.send(&ClientMsg::Library(LibraryReq::Usage));
    let LibraryReply::Usage(files) = answer(&mut raw) else { panic!("no usage") };
    let spam = files.iter().find(|f| f.usage.file.starts_with("Spam")).unwrap();
    assert!(spam.usage.file.chars().count() <= 256 && !spam.usage.file.contains('\n'), "names are cleaned and capped");
    assert_eq!(spam.usage.components.len(), 1_000, "reports are capped");
    let mut raw = Raw::connect(addr);
    raw.send(&open(&Invite { token: "nope".into(), ..edit.clone() }));
    assert!(matches!(raw.next(), ServerMsg::Refused { .. }));
}

#[test]
fn publish_enable_update_and_usage() {
    let dir = TempDir::new("library");
    let cfg = config(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    // Ada's library file, kept on the server: a "Button" with a "Label".
    let mut a = session("Ada");
    let f = id(&ex(&mut a, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 40, "name": "Button"})));
    ex(&mut a, "text.create", json!({"text": "OK", "x": 10, "y": 10, "parent": f, "props": {"name": "Label"}}));
    ex(&mut a, "node.set", json!({"id": f, "props": {"fill": "#0000ff"}}));
    let button = id(&ex(&mut a, "component.create", json!({"ids": [f]})));
    let p = publish(&mut a, dir.path(), addr);
    let mut b = session("Bo");
    let e = b.execute("library.add", &json!({"path": p.view})).unwrap_err().to_string();
    assert!(e.contains("no library on the server"), "{e}");
    let r = ex(&mut a, "library.publishToServer", json!({}));
    assert_eq!(r["version"], 1);

    // Bo enables it with the view link and places the Button, overriding its label.
    let r = ex(&mut b, "library.add", json!({"path": p.view}));
    let lid = r["id"].as_str().unwrap().to_string();
    assert_eq!(r["components"].as_array().map(Vec::len), Some(1));
    let inst = id(&ex(&mut b, "library.place", json!({"library": lid, "component": "Button", "x": 0, "y": 0})));
    let label = b.doc().unwrap().node(inst).unwrap().children[0];
    ex(&mut b, "text.set", json!({"id": label, "characters": "Buy"}));
    assert_eq!(fill(&b, inst), "0000FF");
    assert_eq!(ex(&mut b, "library.list", json!({}))["libraries"][0]["server"], true);

    // Ada changes the Button and publishes again; nothing more to publish after that.
    ex(&mut a, "node.set", json!({"id": button, "props": {"fill": "#00ff00"}}));
    assert_eq!(ex(&mut a, "library.publishToServer", json!({}))["version"], 2);
    let e = a.execute("library.publishToServer", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no changes"), "{e}");

    // Bo's app notices in the background ("Library updates available") and takes the update.
    b.refresh_server_libraries();
    pump(&mut [&mut b], "the new version", |s| s[0].libraries.cached(&p.view).is_some_and(|l| l.is_ok_and(|l| l.manifest.version == 2)));
    assert_eq!(ex(&mut b, "library.updates", json!({}))["count"], 1);
    assert_eq!(ex(&mut b, "library.acceptUpdates", json!({}))["components"], 1);
    assert_eq!(fill(&b, inst), "00FF00", "the change arrived");
    let label = b.doc().unwrap().node(inst).unwrap().children[0];
    assert_eq!(b.doc().unwrap().node(label).unwrap().kind.text().map(|t| t.characters.as_str()), Some("Buy"), "the override stayed");

    // Usage: what Bo's app reported, seen by Ada (her file's library) and by Bo (by the library).
    for (s, args) in [(&mut a, json!({})), (&mut b, json!({"library": lid}))] {
        let u = ex(s, "library.usage", args);
        assert_eq!(u["instances"], 1, "{u}");
        assert_eq!(u["files"].as_array().map(Vec::len), Some(1), "{u}");
        assert_eq!(u["components"][0]["name"], "Button", "{u}");
        assert_eq!(u["components"][0]["files"], 1, "{u}");
    }

    // A view link can't publish, whatever the app does.
    let mine = id(&ex(&mut b, "frame.create", json!({"x": 300, "y": 0, "width": 10, "height": 10, "name": "Mine"})));
    ex(&mut b, "component.create", json!({"ids": [mine]}));
    let e = b.execute("library.publishToServer", &json!({"server": p.view})).unwrap_err().to_string();
    assert!(e.contains("only view"), "{e}");
    let view = Invite::parse(&p.view).unwrap();
    let mut raw = Raw::connect(addr);
    raw.send(&open(&view));
    assert_eq!(answer(&mut raw), LibraryReply::Ready { access: Access::View });
    raw.send(&ClientMsg::Library(LibraryReq::Publish { archive: vec![1, 2, 3] }));
    assert!(matches!(answer(&mut raw), LibraryReply::Failed { reason } if reason.contains("only view")));

    let edit = Invite::parse(&p.edit).unwrap();
    junk_is_refused(addr, &edit);
    assert_eq!(ex(&mut b, "library.list", json!({}))["libraries"][0]["version"], 2);

    // Kept on disk: the library is there after the server stops.
    server.stop();
    drop(server);
    let mut store = Store::open(dir.path(), cfg.store()).unwrap();
    assert!(matches!(
        store.library(std::net::Ipv4Addr::LOCALHOST.into(), &edit.file, Access::View, LibraryReq::Fetch { have: None, usage: None }),
        LibraryReply::Library { version: 2, .. }
    ));
}
