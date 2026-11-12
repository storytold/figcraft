//! Styles, variables, collections and comments created by several people at once: ids from each
//! creator's space, checked by the server, so concurrent creations never overwrite each other.

use figcraft_doc::ids::{STYLE_ID_PREFIX, id_space};
use figcraft_doc::{Collection, Comment, Document, Message, Mode, Style, StyleKind, Variable, VariableType};

use crate::change::{Change, Entry, Json};
use crate::message::{ClientMsg, ServerMsg};
use crate::server::NO_SESSION_SPACE;
use crate::sim::Sim;
use crate::{Client, Limits, NoRecompute, Outgoing, Server};

fn style(id: String, name: &str) -> Style {
    Style { id, name: name.into(), description: String::new(), kind: StyleKind::Paint { paints: Vec::new() }, bound: Default::default() }
}

fn comment(id: u64, doc: &Document) -> Comment {
    let page = doc.first_page().unwrap();
    Comment { id, page, x: 1.0, y: 2.0, anchor: None, first: Message::new("A", "hi", 0), resolved: false, replies: Vec::new(), read_by: None }
}

fn join(server: &mut Server) -> Client {
    let ClientMsg::Hello(h) = Client::hello("f", "", "Ada", "#FF8800") else { panic!("not a hello") };
    let (_, out) = server.join(h).unwrap();
    let Some(Outgoing::To(_, welcome)) = out.into_iter().next() else { panic!("no welcome") };
    Client::join(welcome).unwrap()
}

/// Send `changes` from client `from` as batch `batch_id`; the server's answer to it.
fn send(server: &mut Server, from: u32, batch_id: u64, changes: Vec<Change>) -> ServerMsg {
    let out = server.receive(from, ClientMsg::Changes { base_seq: server.seq(), batch_id, changes });
    match out.into_iter().next() {
        Some(Outgoing::All(m) | Outgoing::To(_, m)) => m,
        other => panic!("unexpected {other:?}"),
    }
}

fn put_style(id: String) -> Change {
    let value = Json::of(&style(id.clone(), "Forged")).unwrap();
    Change::Put { entry: Entry::Style(id), value }
}

#[test]
fn new_entries_must_come_from_the_senders_space() {
    let mut base = Document::new("Shared");
    base.styles.push(style("S:1".into(), "Old"));
    let mut server = Server::new(base, "", Limits::default());
    let mut a = join(&mut server);
    let b = join(&mut server);
    // A style made in a session takes its id from its creator's space.
    let mut doc = a.document().clone();
    let id = doc.new_style_id();
    assert_eq!(id_space(&id, STYLE_ID_PREFIX), a.id(), "{id}");
    doc.styles.push(style(id.clone(), "Brand"));
    let msg = a.edit(&doc).unwrap().unwrap();
    assert!(matches!(server.receive(a.id(), msg).first(), Some(Outgoing::All(ServerMsg::Changes(_)))));
    // Someone else can change it (last writer wins)…
    assert!(matches!(send(&mut server, b.id(), 1, vec![put_style(id.clone())]), ServerMsg::Changes(_)));
    // …but not make a new one in another person's space.
    let theirs = format!("{STYLE_ID_PREFIX}{}", (u64::from(a.id()) << 32) | 999);
    let ServerMsg::Reject { reason, .. } = send(&mut server, b.id(), 2, vec![put_style(theirs)]) else { panic!("accepted") };
    assert!(reason.contains("id space"), "{reason}");
    // Ids from before the session (or of other forms, such as library copies) are anyone's.
    for (k, old) in ["S:77", "library:S:5"].into_iter().enumerate() {
        assert!(matches!(send(&mut server, b.id(), 3 + k as u64, vec![put_style(old.into())]), ServerMsg::Changes(_)), "{old}");
    }
    // A removed entry can be put back by anyone (undo).
    assert!(matches!(send(&mut server, a.id(), 100, vec![Change::Remove { entry: Entry::Style(id.clone()) }]), ServerMsg::Changes(_)));
    assert!(matches!(send(&mut server, b.id(), 10, vec![put_style(id.clone())]), ServerMsg::Changes(_)));
    // The whole-list fallback is checked the same way.
    let mut list = server.document().styles.clone();
    list.push(style(format!("{STYLE_ID_PREFIX}{}", (u64::from(a.id()) << 32) | 1000), "Sneaky"));
    let set = Change::SetDoc { prop: crate::DocProp::Styles, value: Json::of(&list).unwrap() };
    assert!(matches!(send(&mut server, b.id(), 11, vec![set]), ServerMsg::Reject { .. }));
    // Comments too.
    let forged = comment((u64::from(a.id()) << 32) | 5, server.document());
    let put = Change::Put { entry: Entry::Comment(forged.id), value: Json::of(&forged).unwrap() };
    assert!(matches!(send(&mut server, b.id(), 12, vec![put]), ServerMsg::Reject { .. }));
    // Removed entries are part of the saved state.
    let state = server.state().unwrap();
    assert!(state.removed.is_empty(), "put back since");
    let restored = Server::restore(state.clone(), "", Limits::default()).unwrap();
    assert_eq!(restored.state().unwrap(), state);
}

#[test]
fn client_ids_skip_spaces_used_by_styles_and_comments() {
    let mut d = Document::new("a");
    d.styles.push(style(format!("{STYLE_ID_PREFIX}{}", (4u64 << 32) | 1), "From session 4"));
    let c = comment((9u64 << 32) | 1, &d);
    d.comments.push(c);
    let mut server = Server::new(d, "", Limits::default());
    assert_eq!(join(&mut server).id(), 10);
}

#[test]
fn an_id_in_the_top_space_does_not_block_joins() {
    let mut d = Document::new("a");
    d.styles.push(style(format!("{STYLE_ID_PREFIX}{}", u64::MAX), "Out of range"));
    let c = comment(u64::from(u32::MAX) << 32, &d);
    d.comments.push(c);
    let mut server = Server::new(d, "", Limits::default());
    let a = join(&mut server);
    assert!(a.id() > 0 && a.id() < u32::MAX, "{}", a.id());
    // After a reload the session still takes newcomers and keeps the document as it was.
    let state = server.state().unwrap();
    let mut restored = Server::restore(state.clone(), "", Limits::default()).unwrap();
    let b = join(&mut restored);
    assert!(b.id() > a.id() && b.id() < u32::MAX, "{}", b.id());
    assert_eq!(restored.document().styles.len(), 1);
    assert_eq!(restored.document().comments.len(), 1);
    // A state saved while the counter was stuck at the top recovers too.
    let stuck = crate::SessionState { next_client: u32::MAX, ..state };
    let mut restored = Server::restore(stuck, "", Limits::default()).unwrap();
    let c = join(&mut restored);
    assert!(c.id() > a.id() && c.id() < u32::MAX, "{}", c.id());
    // A new entry may not take an id from the reserved top half of the spaces.
    for (k, space) in [NO_SESSION_SPACE, u32::MAX].into_iter().enumerate() {
        let id = format!("{STYLE_ID_PREFIX}{}", (u64::from(space) << 32) | 1);
        let ServerMsg::Reject { reason, .. } = send(&mut restored, c.id(), 1 + k as u64, vec![put_style(id.clone())]) else {
            panic!("accepted {id}")
        };
        assert!(reason.contains("id space"), "{reason}");
    }
}

#[test]
fn styles_imported_from_another_session_are_accepted_and_joins_go_on() {
    let mut server = Server::new(Document::new("a"), "", Limits::default());
    let a = join(&mut server);
    // `file.import` copies styles with their ids, made by clients this session hasn't given out
    // (up to the highest space a member may use).
    for (k, space) in [a.id() + 1, a.id() + 1000, NO_SESSION_SPACE - 1].into_iter().enumerate() {
        let id = format!("{STYLE_ID_PREFIX}{}", (u64::from(space) << 32) | 1);
        assert!(matches!(send(&mut server, a.id(), 1 + k as u64, vec![put_style(id.clone())]), ServerMsg::Changes(_)), "refused {id}");
    }
    // After a restart client ids start above them, and there are plenty left.
    let mut restored = Server::restore(server.state().unwrap(), "", Limits::default()).unwrap();
    assert_eq!(join(&mut restored).id(), NO_SESSION_SPACE);
    assert_eq!(join(&mut Server::new(restored.document().clone(), "", Limits::default())).id(), NO_SESSION_SPACE);
}

/// Each client makes `rounds` styles, collections (with an extra mode), variables and comments
/// before seeing anyone else's.
fn create_everything(doc: &mut Document, who: usize, round: usize) {
    let name = format!("p{who} r{round}");
    let id = doc.new_style_id();
    doc.styles.push(style(id, &name));
    let cid = doc.new_collection_id();
    doc.variables.collections.push(Collection {
        id: cid.clone(),
        name: name.clone(),
        modes: vec![Mode { id: format!("{cid}/mode1"), name: "Light".into() }],
        extends: None,
    });
    if let Some(m) = doc.new_mode_id(&cid)
        && let Some(c) = doc.variables.collections.iter_mut().find(|c| c.id == cid)
    {
        c.modes.push(Mode { id: m, name: "Dark".into() });
    }
    let vid = doc.new_variable_id();
    doc.variables.variables.push(Variable {
        id: vid,
        name,
        collection: cid,
        kind: VariableType::Number,
        values: Default::default(),
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    });
    let k = doc.new_comment_id();
    let c = comment(k, doc);
    doc.comments.push(c);
}

#[test]
fn concurrent_creations_all_survive() {
    for seed in 0..20 {
        let n = 2 + (seed as usize % 4);
        let rounds = 3;
        let mut sim = Sim::new(Document::new("Shared"), n, seed, 12).unwrap();
        for round in 0..rounds {
            // Everyone creates before anything is delivered: the worst case for colliding ids.
            for i in 0..n {
                let mut doc = sim.document(i).unwrap().clone();
                create_everything(&mut doc, i, round);
                assert!(sim.edit(i, &doc).unwrap());
            }
            for _ in 0..sim.rng().below(6) {
                sim.step(&NoRecompute).unwrap();
            }
        }
        sim.settle(&NoRecompute, 1_000_000).unwrap();
        sim.converged().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(sim.rejected, 0, "seed {seed}: nothing is refused");
        let d = sim.server.document();
        let made = n * rounds;
        assert_eq!(d.styles.len(), made, "seed {seed}: every style survives");
        assert_eq!(d.variables.collections.len(), made, "seed {seed}");
        assert!(d.variables.collections.iter().all(|c| c.modes.len() == 2), "seed {seed}: both modes of each");
        assert_eq!(d.variables.variables.len(), made, "seed {seed}");
        assert_eq!(d.comments.len(), made, "seed {seed}");
        for who in 0..n {
            for round in 0..rounds {
                let name = format!("p{who} r{round}");
                assert!(d.styles.iter().any(|s| s.name == name), "seed {seed}: {name}'s style");
                assert!(d.variables.variables.iter().any(|v| v.name == name), "seed {seed}: {name}'s variable");
            }
        }
    }
}
