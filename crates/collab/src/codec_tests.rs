//! Message encodings: round trips, hostile input, and the sizes of typical messages.

use figcraft_doc::{Document, NodeId, NodeKind};

use crate::change::{Change, Json, Prop};
use crate::message::{
    Accepted, Access, Admin, AdminReply, CatchUp, ClientMsg, EditingText, FileInfo, FileRow, FileUsage, Hello, HostInfo, LibraryReply, LibraryReq,
    LinkInfo, Manage, NewLink, OpenHosting, OpenLibrary, OpenPlugins, PROTOCOL, Peer, PluginEntry, PluginReply, PluginReq, Presence, Processed,
    ReportInfo, Resume, ServerMsg, Signal, Usage, WIRE_VERSION, decode, decode_json, encode, encode_json,
};
use crate::sim::Rng;
use crate::{Error, Replica, diff};

/// A typical edit: one layer dragged (its transform).
fn moved_layer() -> ClientMsg {
    let mut doc = Document::new("f");
    let page = doc.first_page().unwrap();
    doc.set_client_id(3);
    let r = doc.create(page, NodeKind::Rectangle, "Rectangle 1", 120.0, 80.0, 100.0, 60.0).unwrap();
    let a = Replica::new(doc);
    let mut b = a.document().clone();
    b.get_mut(r).unwrap().transform = figcraft_geom::Affine::translate((164.5, 92.25));
    ClientMsg::Changes { base_seq: 1234, batch_id: 56, changes: diff(&a, &b).unwrap().changes }
}

fn cursor() -> ClientMsg {
    ClientMsg::Presence(Presence { cursor: Some((NodeId(1), 512.5, 384.25)), ..Default::default() })
}

fn samples() -> (Vec<ClientMsg>, Vec<ServerMsg>) {
    let peer = Peer { client_id: 4, name: "Ada".into(), color: "#FF8800".into() };
    let set = Change::Set { id: NodeId((4 << 32) | 9), prop: Prop::Name, value: Json::of("Café ☕").unwrap() };
    let client = vec![
        ClientMsg::Hello(Hello {
            protocol: PROTOCOL,
            file: "f".into(),
            token: "t".into(),
            name: "Ada".into(),
            color: "#FF8800".into(),
            client_version: "0.2.0".into(),
            resume: Some(Resume { session: 77, client_id: 4, seq: 10 }),
        }),
        moved_layer(),
        cursor(),
        ClientMsg::Presence(Presence {
            cursor: None,
            selection: Some(vec![NodeId(2), NodeId(3)]),
            viewport: Some((NodeId(1), 0.0, 0.0, 1200.0, 800.0)),
            editing_text: Some(Box::new(EditingText { node: NodeId(7), caret: 3, selection: Some((1, 3)) })),
        }),
        ClientMsg::Ack { seq: 9 },
        ClientMsg::Ping,
        ClientMsg::Manage(Manage { protocol: PROTOCOL, key: "k".into(), client_version: "0.3.1".into() }),
        ClientMsg::Admin(Admin::Publish { name: "Café".into(), snapshot: Replica::new(Document::new("Up")).snapshot().unwrap() }),
        ClientMsg::Admin(Admin::Files),
        ClientMsg::Admin(Admin::Links { file: "f".into() }),
        ClientMsg::Admin(Admin::CreateLink { file: "f".into(), access: Access::View, label: "Review".into() }),
        ClientMsg::Admin(Admin::Revoke { file: "f".into(), link: "l".into() }),
        ClientMsg::Admin(Admin::Remove { file: "f".into() }),
        ClientMsg::OpenLibrary(OpenLibrary { protocol: PROTOCOL, file: "f".into(), token: "t".into(), client_version: "0.3.1".into() }),
        ClientMsg::Library(LibraryReq::Fetch { have: Some(3), usage: Some(Usage { file: "Checkout".into(), components: vec![(NodeId(7), 12)] }) }),
        ClientMsg::Library(LibraryReq::Publish { archive: vec![0x50, 0x4B, 3, 4] }),
        ClientMsg::Library(LibraryReq::Usage),
        ClientMsg::Signal(Signal::chat("Hi ☕")),
        ClientMsg::Signal(Signal::spotlight(true)),
        ClientMsg::OpenPlugins(OpenPlugins { protocol: PROTOCOL, key: String::new(), client_version: "0.3.1".into() }),
        ClientMsg::Plugins(PluginReq::List { query: "grid".into() }),
        ClientMsg::Plugins(PluginReq::Fetch { id: "org.example.p".into(), version: Some("1.0.0".into()) }),
        ClientMsg::Plugins(PluginReq::Publish { bundle: vec![b'F', b'C'] }),
        ClientMsg::Plugins(PluginReq::Unpublish { id: "org.example.p".into(), version: None }),
        ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: "0.3.2 hosting".into(), file: "f".into(), owner: "o".into() }),
        ClientMsg::Admin(Admin::Rename { file: "f".into(), name: "Café 2".into() }),
        ClientMsg::Admin(Admin::Listing),
        ClientMsg::Admin(Admin::Reports),
        ClientMsg::Admin(Admin::Block { ip: "203.0.113.9".into() }),
        ClientMsg::Admin(Admin::Unblock { ip: "2001:db8::".into() }),
        ClientMsg::Admin(Admin::Blocked),
        ClientMsg::Admin(Admin::RemovePeer { file: "f".into(), client: 7 }),
        ClientMsg::Report { reason: "Spam ☕".into() },
    ];
    let snapshot = Replica::new(Document::new("Snap")).snapshot().unwrap();
    let file = FileInfo { id: "f".into(), name: "Café".into(), created: 1, updated: 2, seq: 3, bytes: 4, people: 5 };
    let link = LinkInfo { id: "l".into(), file: "f".into(), access: Access::Edit, label: String::new(), created: 6 };
    let new_link = NewLink { link: link.clone(), token: "t".into() };
    let entry = PluginEntry {
        id: "org.example.p".into(),
        name: "Grille ☕".into(),
        version: "1.2.0".into(),
        access: "write".into(),
        commands: vec!["node.*".into()],
        bytes: 9,
        sha256: "ab".into(),
        versions: vec!["1.2.0".into(), "1.0.0".into()],
        ..PluginEntry::default()
    };
    let server = vec![
        ServerMsg::Welcome {
            session: 77,
            client_id: 4,
            seq: 10,
            catch_up: CatchUp::Snapshot(snapshot),
            processed: Processed::default(),
            peers: vec![peer.clone()],
            access: Access::Edit,
        },
        ServerMsg::Welcome {
            session: 77,
            client_id: 4,
            seq: 11,
            catch_up: CatchUp::Delta(vec![Accepted { seq: 11, author: 5, batch_id: 1, changes: vec![set.clone()] }]),
            processed: Processed { last_batch: 7, refused: vec![3, 6] },
            peers: vec![],
            access: Access::View,
        },
        ServerMsg::Changes(Accepted { seq: 11, author: 4, batch_id: 2, changes: vec![set.clone(), Change::Delete { id: NodeId(5) }] }),
        ServerMsg::Reject { batch_id: 3, reason: "no".into(), fixes: vec![set] },
        ServerMsg::Presence { client_id: 4, presence: Presence::default() },
        ServerMsg::PeerJoined(peer),
        ServerMsg::PeerLeft { client_id: 4 },
        ServerMsg::Refused { reason: "full".into() },
        ServerMsg::Pong,
        ServerMsg::Admin(Box::new(AdminReply::Ready)),
        ServerMsg::Admin(Box::new(AdminReply::Published { file: file.clone(), edit: new_link.clone(), view: new_link.clone() })),
        ServerMsg::Admin(Box::new(AdminReply::Files(vec![file.clone()]))),
        ServerMsg::Admin(Box::new(AdminReply::Links(vec![link]))),
        ServerMsg::Admin(Box::new(AdminReply::Link(new_link.clone()))),
        ServerMsg::Admin(Box::new(AdminReply::Done)),
        ServerMsg::Admin(Box::new(AdminReply::Failed { reason: "no".into() })),
        ServerMsg::Library(Box::new(LibraryReply::Ready { access: Access::View })),
        ServerMsg::Library(Box::new(LibraryReply::Library { version: 2, archive: vec![1, 2, 3] })),
        ServerMsg::Library(Box::new(LibraryReply::Unchanged { version: 2 })),
        ServerMsg::Library(Box::new(LibraryReply::Empty)),
        ServerMsg::Library(Box::new(LibraryReply::Published { version: 3 })),
        ServerMsg::Library(Box::new(LibraryReply::Usage(vec![FileUsage {
            usage: Usage { file: "Checkout".into(), components: vec![(NodeId(7), 12)] },
            updated: 9,
        }]))),
        ServerMsg::Library(Box::new(LibraryReply::Failed { reason: "no".into() })),
        ServerMsg::Signal { client_id: 4, signal: Signal::chat("Hi ☕") },
        ServerMsg::Signal { client_id: 4, signal: Signal::ready() },
        ServerMsg::Plugins(Box::new(PluginReply::Ready { publisher: true })),
        ServerMsg::Plugins(Box::new(PluginReply::List(vec![entry.clone()]))),
        ServerMsg::Plugins(Box::new(PluginReply::Bundle { entry: entry.clone(), bundle: vec![1, 2] })),
        ServerMsg::Plugins(Box::new(PluginReply::Published(entry))),
        ServerMsg::Plugins(Box::new(PluginReply::Removed(vec!["1.0.0".into()]))),
        ServerMsg::Plugins(Box::new(PluginReply::Failed { reason: "no".into() })),
        ServerMsg::Admin(Box::new(AdminReply::Hosted {
            file: file.clone(),
            edit: new_link.clone(),
            view: new_link.clone(),
            owner: "o".into(),
            expires: Some(99),
        })),
        ServerMsg::Admin(Box::new(AdminReply::Listing(vec![FileRow {
            info: file.clone(),
            opened: 7,
            expires: None,
            open: true,
            ip: "203.0.113.9".into(),
            reports: 2,
        }]))),
        ServerMsg::Admin(Box::new(AdminReply::Reports(vec![ReportInfo {
            at: 8,
            file: "f".into(),
            name: "Café".into(),
            link: "l".into(),
            ip: "203.0.113.9".into(),
            reason: "Spam".into(),
        }]))),
        ServerMsg::Admin(Box::new(AdminReply::Blocked(vec!["203.0.113.9".into()]))),
        ServerMsg::Hosting(Box::new(HostInfo {
            open: true,
            operator: "abuse@example.com".into(),
            terms: "https://example.com/terms".into(),
            idle_secs: 86_400,
            max_file_mb: 16,
            expires: Some(1_800_000_000),
        })),
        ServerMsg::Reported { error: None },
        ServerMsg::Reported { error: Some("too many reports".into()) },
    ];
    (client, server)
}

#[test]
fn every_message_round_trips_in_both_forms() {
    let (client, server) = samples();
    for m in &client {
        assert_eq!(&decode::<ClientMsg>(&encode(m).unwrap(), usize::MAX).unwrap(), m);
        assert_eq!(&decode_json::<ClientMsg>(&encode_json(m).unwrap(), usize::MAX).unwrap(), m);
    }
    for m in &server {
        assert_eq!(&decode::<ServerMsg>(&encode(m).unwrap(), usize::MAX).unwrap(), m);
        assert_eq!(&decode_json::<ServerMsg>(&encode_json(m).unwrap(), usize::MAX).unwrap(), m);
    }
    // The JSON form is readable: the type, then its fields; values as JSON.
    let json = encode_json(&moved_layer()).unwrap();
    assert!(json.starts_with(r#"{"changes":{"base_seq":1234,"batch_id":56,"changes":[{"set":"#), "{json}");
    assert!(json.contains(r#""prop":"transform","value":[1.0,0.0,0.0,1.0,164.5,92.25]"#), "{json}");
    let chat = encode_json(&ClientMsg::Signal(Signal::chat("hi"))).unwrap();
    assert_eq!(chat, r#"{"signal":{"kind":"chat","text":"hi"}}"#);
    // The binary form starts with the wire version and carries values as JSON text.
    let bin = encode(&moved_layer()).unwrap();
    assert_eq!(bin[0], WIRE_VERSION);
    assert!(bin.windows(5).any(|w| w == b"164.5"));
}

#[test]
fn hostile_bytes_are_refused_without_panicking() {
    assert!(matches!(decode::<ClientMsg>(&[], 100), Err(Error::Decode(_))));
    assert!(matches!(decode::<ClientMsg>(&[WIRE_VERSION + 1, 4], 100), Err(Error::Decode(_))));
    let ping = encode(&ClientMsg::Ping).unwrap();
    let mut trailing = ping.clone();
    trailing.push(0);
    assert!(decode::<ClientMsg>(&trailing, 100).is_err(), "trailing bytes");
    assert!(decode::<ClientMsg>(&ping, 1).is_err(), "over the size limit");
    assert!(decode_json::<ClientMsg>("{\"ping\":null}", 3).is_err(), "over the size limit");
    // A huge length prefix doesn't allocate it.
    let mut huge = vec![WIRE_VERSION, 1];
    huge.extend([0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    assert!(decode::<ClientMsg>(&huge, 1 << 20).is_err());
    // Deeply nested JSON inside a value is refused by the JSON parser's depth limit.
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    let evil = format!(r#"{{"changes":{{"base_seq":0,"batch_id":0,"changes":[{{"set":{{"id":1,"prop":"name","value":{deep}}}}}]}}}}"#);
    assert!(decode_json::<ClientMsg>(&evil, usize::MAX).is_err());
    // The same value in a binary message (JSON text inside).
    let mut bin = encode(&ClientMsg::Changes {
        base_seq: 0,
        batch_id: 0,
        changes: vec![Change::Set { id: NodeId(1), prop: Prop::Name, value: Json::of("x").unwrap() }],
    })
    .unwrap();
    let at = bin.windows(3).position(|w| w == b"\"x\"").unwrap();
    bin.truncate(at - 1);
    let mut len = deep.len();
    while len >= 0x80 {
        bin.push((len as u8 & 0x7F) | 0x80);
        len >>= 7;
    }
    bin.push(len as u8);
    bin.extend(deep.as_bytes());
    assert!(matches!(decode::<ClientMsg>(&bin, usize::MAX), Err(Error::Decode(_))));
}

#[test]
fn fuzzed_messages_never_panic() {
    let (client, server) = samples();
    let valid: Vec<Vec<u8>> = client.iter().map(|m| encode(m).unwrap()).chain(server.iter().map(|m| encode(m).unwrap())).collect();
    let mut rng = Rng::new(42);
    for round in 0..20_000 {
        let mut bytes =
            if round % 4 == 0 { (0..rng.below(64)).map(|_| rng.next_u64() as u8).collect() } else { rng.pick(&valid).cloned().unwrap_or_default() };
        // Flip, insert or cut a few bytes.
        for _ in 0..1 + rng.below(4) {
            if bytes.is_empty() {
                break;
            }
            let i = rng.below(bytes.len());
            match rng.below(3) {
                0 => bytes[i] = rng.next_u64() as u8,
                1 => bytes.insert(i, rng.next_u64() as u8),
                _ => bytes.truncate(i.max(1)),
            }
        }
        let _ = decode::<ClientMsg>(&bytes, 1 << 20);
        let _ = decode::<ServerMsg>(&bytes, 1 << 20);
        if let Ok(text) = std::str::from_utf8(&bytes) {
            let _ = decode_json::<ClientMsg>(text, 1 << 20);
        }
    }
}

#[test]
fn message_sizes() {
    // Recorded in docs/collab-protocol.md (Encoding). Binary stays well under JSON.
    let edit = moved_layer();
    let (eb, ej) = (encode(&edit).unwrap().len(), encode_json(&edit).unwrap().len());
    let cur = cursor();
    let (cb, cj) = (encode(&cur).unwrap().len(), encode_json(&cur).unwrap().len());
    println!("typical edit (move a layer): binary {eb} B, JSON {ej} B; cursor update: binary {cb} B, JSON {cj} B");
    assert!(eb < ej && cb < cj);
    assert!(cb <= 32, "a cursor update is tiny: {cb}");
    assert!(eb <= 96, "a move is small: {eb}");
}
