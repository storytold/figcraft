//! Sessions per document over the network: the link's session table, and two engine sessions in
//! one process that host and join several files at once through `collab.*` commands. Each file's
//! session has its own people, links, edits and presence; closing a file ends its session only.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use figcraft_doc::NodeId;
use figcraft_engine::Session;
use figcraft_engine::live::{Event, Live, Presence, Role, Status};
use serde_json::{Value, json};

use crate::NetLink;
use crate::loopback_tests::quick;
use crate::sessions::Sessions;

#[test]
fn each_session_has_its_own_mailbox() {
    let woken = Arc::new(AtomicUsize::new(0));
    let wake = woken.clone();
    let mut table: Sessions<&str> = Sessions::new(Arc::new(move || {
        wake.fetch_add(1, Ordering::SeqCst);
    }));
    let (a, b) = (table.inbox(), table.inbox());
    assert!(table.insert(1, "host", a.clone()).is_none());
    assert!(table.insert(2, "guest", b.clone()).is_none());
    a.push(Event::PeerLeft(7));
    b.push(Event::PeerLeft(9));
    b.presence(3, Presence::default());
    assert_eq!(woken.load(Ordering::SeqCst), 3, "every arrival wakes the UI");
    assert_eq!(table.poll(), vec![(1, Event::PeerLeft(7)), (2, Event::PeerLeft(9)), (2, Event::Presence(3, Presence::default()))]);
    assert!(table.poll().is_empty(), "taken once");
    // A session that ended: what its threads still send goes nowhere, the other goes on.
    assert_eq!(table.remove(1), Some("host"));
    a.push(Event::PeerLeft(8));
    b.push(Event::PeerLeft(10));
    assert_eq!(table.poll(), vec![(2, Event::PeerLeft(10))]);
    assert_eq!((table.ids(), table.get(1), table.get(2)), (vec![2], None, Some(&"guest")));
    // An id started again replaces what had it, mailbox included.
    assert_eq!(table.insert(2, "again", table.inbox()), Some("guest"));
    b.push(Event::PeerLeft(11));
    assert!(table.poll().is_empty());
    assert_eq!(table.remove(3), None);
}

fn session(name: &str) -> Session {
    let mut s = Session::new();
    s.live.install(Box::new(NetLink::new(Arc::new(|| {})).with_guest_config(quick)));
    s.execute("pref.userName", &json!({"name": name})).unwrap();
    s
}

/// Pump the sessions (as their UIs would every frame) until `f` holds.
fn pump(all: &mut [&mut Session], what: &str, f: impl Fn(&[&mut Session]) -> bool) {
    let start = Instant::now();
    loop {
        for s in all.iter_mut() {
            s.live_poll();
            s.live_sync();
        }
        if f(all) {
            return;
        }
        let states: Vec<String> = all.iter().map(|s| s.live_summary()["sessions"].to_string()).collect();
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}: {states:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn link_of(v: &Value) -> String {
    v["link"].as_str().unwrap().to_string()
}

fn uid_at(s: &Session, index: usize) -> u64 {
    s.docs[index].uid
}

/// The session syncing through the host at `address` (a guest's).
fn joined<'a>(s: &'a Session, address: &str) -> Option<&'a Live> {
    s.live.iter().find(|l| l.syncing() && l.info.as_ref().is_some_and(|i| i.role == Role::Guest && i.address == address))
}

fn has(s: &Session, uid: u64, node: u64) -> bool {
    s.docs.iter().find(|d| d.uid == uid).is_some_and(|d| d.doc.contains(NodeId(node)))
}

fn rect(s: &mut Session) -> u64 {
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap()
}

/// Asserts two files are hosted and joined independently: each has its own port, session and
/// edits, presence stays in its file's session, and closing one file's session leaves the
/// other live.
#[test]
fn two_files_are_hosted_and_joined_independently() {
    let mut host = session("Hana");
    let mut guest = session("Ada");
    // File A is hosted; a new file B isn't in a session until it is hosted too.
    let a = host.execute("collab.host", &json!({"port": 0})).unwrap();
    host.execute("file.new", &json!({"name": "Second"})).unwrap();
    let off = host.execute("collab.peers", &json!({})).unwrap();
    assert_eq!((&off["status"], &off["session"], &off["link"], &off["document"]), (&json!("off"), &Value::Null, &Value::Null, &Value::Null));
    assert_eq!(off["sessions"].as_array().unwrap().len(), 1, "the first file's session goes on: {off}");
    assert_eq!(off["sessions"][0]["document"], 0);
    let b = host.execute("collab.host", &json!({"port": 0})).unwrap();
    assert_ne!(a["address"], b["address"], "each file listens on its own port");
    assert_ne!(a["session"], b["session"]);
    assert_eq!(b["sessions"].as_array().unwrap().len(), 2);
    let (addr_a, addr_b) = (a["address"].as_str().unwrap(), b["address"].as_str().unwrap());
    let (ha, hb) = (uid_at(&host, 0), uid_at(&host, 1));
    // The guest joins both, one right after the other: two more tabs, each with its session.
    for l in [&a, &b] {
        assert_eq!(guest.execute("collab.join", &json!({"link": link_of(l)})).unwrap()["status"], "connecting");
    }
    pump(&mut [&mut host, &mut guest], "both joins", |s| {
        joined(&*s[1], addr_a).is_some() && joined(&*s[1], addr_b).is_some() && s[0].live_of(ha).peers.len() == 1 && s[0].live_of(hb).peers.len() == 1
    });
    assert_eq!(guest.docs.len(), 3, "the guest's own file and the two it joined");
    let (ga, gb) = (joined(&guest, addr_a).unwrap().doc_uid.unwrap(), joined(&guest, addr_b).unwrap().doc_uid.unwrap());
    assert_ne!(ga, gb);
    assert_eq!(guest.live_of(uid_at(&guest, 0)).status, Status::Off, "the file the joins were started from is in no session");
    assert_eq!(guest.docs.iter().find(|d| d.uid == gb).unwrap().doc.name, "Second");
    // `collab.peers` is the file in front's: switching files switches it.
    assert_eq!(host.live_summary()["link"], b["link"]);
    host.execute("doc.switch", &json!({"index": 0})).unwrap();
    assert_eq!(host.live_summary()["link"], a["link"]);
    assert_eq!(host.live_summary()["document"], 0);
    // Each joined file opens in front when its welcome arrives, and the two joins race: the file
    // whose welcome came last is in front.
    assert_eq!(guest.ds().unwrap().uid, guest.docs[2].uid, "the file that arrived last is in front");
    // Concurrent joins may complete in either order. Put B in front explicitly so the following
    // assertions exercise A's background session independent of network timing.
    guest.execute("doc.switch", &json!({"index": guest.docs.iter().position(|d| d.uid == gb).unwrap()})).unwrap();
    assert_eq!(guest.ds().unwrap().uid, gb, "B is in front while A receives edits");
    // An edit in A reaches the guest's copy of A only, while B is in front at the guest.
    let in_a = rect(&mut host);
    host.execute("doc.switch", &json!({"index": 1})).unwrap();
    let in_b = rect(&mut host);
    host.execute("node.set", &json!({"ids": [in_b], "props": {"name": "Only in B"}})).unwrap();
    let name = |s: &Session, uid: u64, node: u64| s.docs.iter().find(|d| d.uid == uid).and_then(|d| d.doc.node(NodeId(node))).map(|n| n.name.clone());
    // The rename is its own batch, behind the one that creates the layer: wait for it too.
    pump(&mut [&mut host, &mut guest], "the edits at the guest", |s| has(&*s[1], ga, in_a) && name(&*s[1], gb, in_b).as_deref() == Some("Only in B"));
    assert_ne!(name(&guest, ga, in_b).as_deref(), Some("Only in B"), "B's edit never reaches A");
    assert_ne!(name(&host, ha, in_b).as_deref(), Some("Only in B"));
    // And back: the guest edits its copy of A in the background tab's session.
    guest.execute("doc.switch", &json!({"index": guest.docs.iter().position(|d| d.uid == ga).unwrap()})).unwrap();
    let from_guest = rect(&mut guest);
    guest.execute("node.set", &json!({"ids": [from_guest], "props": {"name": "From Ada"}})).unwrap();
    pump(&mut [&mut host, &mut guest], "the guest's edit at the host", |s| name(&*s[0], ha, from_guest).as_deref() == Some("From Ada"));
    assert_ne!(name(&host, hb, from_guest).as_deref(), Some("From Ada"));
    // Presence stays in its session: the guest points in A (in front there), not in B.
    let ada = |s: &Session, uid: u64| s.live_of(uid).peers.keys().next().copied().unwrap();
    guest.execute("collab.cursor", &json!({"x": 64, "y": 48})).unwrap();
    pump(&mut [&mut host, &mut guest], "the guest's cursor in A", |s| {
        s[0].live_of(ha).presence.get(&ada(&*s[0], ha)).and_then(|p| p.cursor).is_some_and(|c| c.1 == 64.0 && c.2 == 48.0)
    });
    assert!(host.live_of(hb).presence.get(&ada(&host, hb)).is_none_or(|p| p.cursor.is_none()), "no cursor in the file that isn't in front");
    // The host closes B: its session ends for everyone, A's goes on.
    host.execute("file.close", &json!({"index": 1, "force": true})).unwrap();
    pump(&mut [&mut host, &mut guest], "B's session ending", |s| s[0].live.iter().count() == 1 && s[1].live.iter().count() == 1);
    assert_eq!(guest.live_of(gb).status, Status::Off);
    assert_eq!(guest.docs.len(), 3, "the guest keeps its copy of B");
    assert_eq!((host.live_of(ha).status.clone(), host.live_of(ha).peers.len()), (Status::Live, 1));
    assert_eq!(guest.live_of(ga).status, Status::Live);
    let later = rect(&mut host);
    pump(&mut [&mut host, &mut guest], "an edit after B closed", |s| has(&*s[1], ga, later));
    assert!(!has(&guest, gb, later));
}

#[test]
fn hosting_one_file_while_joined_to_another() {
    let mut x = session("Hana");
    let mut y = session("Ada");
    let from_x = link_of(&x.execute("collab.host", &json!({"port": 0})).unwrap());
    let from_y = link_of(&y.execute("collab.host", &json!({"port": 0})).unwrap());
    let (xu, yu) = (uid_at(&x, 0), uid_at(&y, 0));
    // Each joins the other's file from the tab it hosts: the hosting goes on.
    y.execute("collab.join", &json!({"link": from_x})).unwrap();
    x.execute("collab.join", &json!({"link": from_y})).unwrap();
    pump(&mut [&mut x, &mut y], "both joined", |s| {
        s.iter().all(|s| s.docs.len() == 2 && s.live().syncing() && s.live().role() == Some(Role::Guest))
            && s[0].live_of(xu).peers.len() == 1
            && s[1].live_of(yu).peers.len() == 1
    });
    for (s, own) in [(&x, xu), (&y, yu)] {
        let p = s.live_summary();
        assert_eq!((&p["role"], &p["document"]), (&json!("guest"), &json!(1)), "the joined file is in front: {p}");
        assert_eq!(p["sessions"].as_array().unwrap().len(), 2);
        assert_eq!(s.live_of(own).role(), Some(Role::Host));
        assert_eq!(s.live_of(own).status, Status::Live);
    }
    // Y edits X's file (its tab in front) and its own; each lands in its own file at X.
    let in_x = rect(&mut y);
    y.execute("doc.switch", &json!({"index": 0})).unwrap();
    let in_y = rect(&mut y);
    y.execute("node.set", &json!({"ids": [in_y], "props": {"name": "Hosted by Ada"}})).unwrap();
    let named = |s: &Session, uid: u64| {
        s.docs.iter().find(|d| d.uid == uid).is_some_and(|d| d.doc.node(NodeId(in_y)).is_some_and(|n| n.name == "Hosted by Ada"))
    };
    let x_copy_of_y = x.live().doc_uid.unwrap();
    pump(&mut [&mut x, &mut y], "Y's edits at X", |s| has(&*s[0], xu, in_x) && named(&*s[0], x_copy_of_y));
    assert!(!named(&x, xu), "the edit of Y's file isn't in X's");
    // Y leaves X's file: Y still hosts its own, with X in it.
    y.execute("doc.switch", &json!({"index": 1})).unwrap();
    assert_eq!(y.execute("collab.leave", &json!({})).unwrap()["ended"], true);
    pump(&mut [&mut x, &mut y], "the leave", |s| s[0].live_of(xu).peers.is_empty());
    assert_eq!((y.live().status.clone(), y.live.iter().count()), (Status::Off, 1));
    assert_eq!((y.live_of(yu).status.clone(), y.live_of(yu).peers.len()), (Status::Live, 1));
    assert_eq!(x.live_of(x_copy_of_y).status, Status::Live);
}
