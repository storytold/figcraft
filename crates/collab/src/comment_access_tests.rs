//! Comment links: a commenter sees everything and may change comment threads (add, reply, react,
//! resolve, remove, reorder), but a batch that changes anything else is refused with the reason
//! and rolled back; a client that can't read comment links joins with one as a viewer, managing
//! clients that can't read them get them listed as view links, and a level this build doesn't know
//! reads as view.

use figcraft_doc::{Comment, Document, Message, NodeKind};

use crate::change::{Change, DocProp, Entry, Json, Prop};
use crate::message::{
    self, Access, AdminReply, ClientMsg, Hello, LinkInfo, NewLink, ServerMsg, WIRE_VERSION, decode, decode_json, encode, encode_json,
};
use crate::sim_tests::base;
use crate::{COMMENT_ONLY, Client, Limits, NoRecompute, Outgoing, Server};

fn hello(client_version: &str) -> Hello {
    let ClientMsg::Hello(mut h) = Client::hello("f", "", "Ada", "#FF8800") else { panic!("not a hello") };
    h.client_version = client_version.into();
    h
}

fn join_as(s: &mut Server, access: Access, client_version: &str) -> Client {
    let (_, out) = s.join_as(hello(client_version), access).unwrap();
    let Some(Outgoing::To(_, welcome)) = out.into_iter().next() else { panic!("no welcome") };
    Client::join(welcome).unwrap()
}

/// `c` edits its copy with `f`; the server's answer, delivered to everyone in `all`.
fn send(s: &mut Server, c: &mut Client, f: impl FnOnce(&mut Document), all: &mut [&mut Client]) -> Vec<Outgoing> {
    let mut d = c.document().clone();
    f(&mut d);
    let m = c.edit(&d).unwrap().expect("a change");
    let out = s.receive(c.id(), m);
    for o in &out {
        let m = match o {
            Outgoing::All(m) => m,
            Outgoing::To(t, m) if *t == c.id() => m,
            _ => continue,
        };
        c.receive(m.clone(), &NoRecompute).unwrap();
        for other in all.iter_mut() {
            if matches!(o, Outgoing::All(_)) {
                other.receive(m.clone(), &NoRecompute).unwrap();
            }
        }
    }
    out
}

fn accepted(out: &[Outgoing]) -> bool {
    matches!(out, [Outgoing::All(ServerMsg::Changes(_))])
}

fn refused(out: &[Outgoing]) -> bool {
    matches!(out, [Outgoing::To(_, ServerMsg::Reject { reason, .. })] if reason == COMMENT_ONLY)
}

fn comment(id: u64, d: &Document, text: &str) -> Comment {
    let page = d.first_page().unwrap();
    Comment { id, page, x: 10.0, y: 20.0, anchor: None, first: Message::new("Cy", text, 0), resolved: false, replies: vec![], read_by: None }
}

#[test]
fn a_commenter_changes_comment_threads_and_nothing_else() {
    let mut s = Server::new(base(), "", Limits::default());
    let mut editor = join_as(&mut s, Access::Edit, &message::client_version());
    let mut cy = join_as(&mut s, Access::Comment, &message::client_version());
    assert_eq!(cy.access(), Access::Comment, "the welcome says what the client may do");
    let page = s.document().first_page().unwrap();
    // A new thread reaches everyone; so do a reply, a resolve and a reaction to it.
    let out = send(
        &mut s,
        &mut cy,
        |d| {
            let id = d.new_comment_id();
            let c = comment(id, d, "Bigger logo?");
            d.comments.push(c);
        },
        &mut [&mut editor],
    );
    assert!(accepted(&out), "{out:?}");
    assert_eq!(editor.document().comments.len(), 1);
    let out = send(
        &mut s,
        &mut cy,
        |d| {
            let c = &mut d.comments[0];
            c.replies.push(Message::new("Cy", "Or a darker one", 1));
            c.resolved = true;
            c.first.react("👍", "Cy", None);
        },
        &mut [&mut editor],
    );
    assert!(accepted(&out), "{out:?}");
    assert_eq!(s.document().comments[0].replies.len(), 1);
    assert!(s.document().comments[0].resolved);
    // A layer, a page or any other change is refused with the reason and rolled back.
    let seq = s.seq();
    assert!(refused(&send(&mut s, &mut cy, |d| d.node_mut(page).unwrap().name = "Mine".into(), &mut [])));
    assert!(refused(&send(
        &mut s,
        &mut cy,
        |d| {
            d.create(page, NodeKind::Rectangle, "Card", 0.0, 0.0, 10.0, 10.0).unwrap();
        },
        &mut []
    )));
    assert!(refused(&send(&mut s, &mut cy, |d| d.name = "Renamed".into(), &mut [])));
    // So is a batch that sneaks an edit in beside a comment.
    assert!(refused(&send(
        &mut s,
        &mut cy,
        |d| {
            d.comments[0].resolved = false;
            d.node_mut(page).unwrap().name = "Mine".into();
        },
        &mut []
    )));
    assert_eq!(s.seq(), seq, "nothing accepted");
    assert_eq!(cy.view(), s.replica(), "the commenter's copy is the server's again");
    assert!(s.document().comments[0].resolved);
    // Removing a thread is a comment change too.
    assert!(accepted(&send(&mut s, &mut cy, |d| d.comments.clear(), &mut [&mut editor])));
    assert!(editor.document().comments.is_empty());
    // The editor still edits.
    assert!(accepted(&send(&mut s, &mut editor, |d| d.node_mut(page).unwrap().name = "Home".into(), &mut [&mut cy])));
    assert_eq!(cy.document().node(page).unwrap().name, "Home");
}

#[test]
fn a_client_that_cant_read_comment_links_joins_with_one_as_a_viewer() {
    let mut s = Server::new(base(), "", Limits::default());
    let mut old = join_as(&mut s, Access::Comment, "0.12.1 signals hosting");
    assert_eq!(old.access(), Access::View);
    let page = s.document().first_page().unwrap();
    let out = send(
        &mut s,
        &mut old,
        |d| {
            let id = d.new_comment_id();
            let c = comment(id, d, "Hi");
            d.comments.push(c);
        },
        &mut [],
    );
    assert!(matches!(out.as_slice(), [Outgoing::To(_, ServerMsg::Reject { reason, .. })] if reason == crate::VIEW_ONLY), "{out:?}");
    // Edit and view links are as they were for it.
    assert_eq!(join_as(&mut s, Access::Edit, "0.12.1").access(), Access::Edit);
    assert_eq!(join_as(&mut s, Access::View, "0.12.1").access(), Access::View);
    assert!(s.document().node(page).is_some());
}

#[test]
fn only_comment_thread_changes_count_as_comments() {
    let d = base();
    let c = comment(7, &d, "x");
    let value = Json::of(&c).unwrap();
    let page = d.first_page().unwrap();
    let comments = [
        Change::Put { entry: Entry::Comment(7), value: value.clone() },
        Change::Remove { entry: Entry::Comment(7) },
        Change::SetDoc { prop: DocProp::CommentOrder, value: Json::of(&[7u64]).unwrap() },
        Change::SetDoc { prop: DocProp::Comments, value: Json::of(&[c]).unwrap() },
    ];
    assert!(comments.iter().all(Change::is_comment));
    let others = [
        Change::Put { entry: Entry::Style("S:1".into()), value: value.clone() },
        Change::Remove { entry: Entry::Image("a".into()) },
        Change::SetDoc { prop: DocProp::Name, value: Json::of("x").unwrap() },
        Change::SetDoc { prop: DocProp::StyleOrder, value: Json::of(&["S:1"]).unwrap() },
        Change::Set { id: page, prop: Prop::Name, value: Json::of("x").unwrap() },
        Change::Delete { id: page },
    ];
    assert!(!others.iter().any(Change::is_comment));
}

#[test]
fn comment_access_reads_back_and_an_unknown_level_reads_as_view() {
    assert_eq!(Access::parse(" Comment "), Some(Access::Comment));
    assert_eq!(Access::Comment.id(), "comment");
    for a in [Access::Edit, Access::View, Access::Comment] {
        assert_eq!(decode::<Access>(&encode(&a).unwrap(), 64).unwrap(), a);
        assert_eq!(decode_json::<Access>(&encode_json(&a).unwrap(), 64).unwrap(), a);
    }
    // A level from a newer build.
    assert_eq!(decode::<Access>(&[WIRE_VERSION, 9], 64).unwrap(), Access::View);
    assert_eq!(decode_json::<Access>("\"owner\"", 64).unwrap(), Access::View);
}

#[test]
fn managing_clients_that_cant_read_comment_links_get_view_links() {
    let link = |access| LinkInfo { id: "l".into(), file: "f".into(), access, label: String::new(), created: 0 };
    let listing = AdminReply::Links(vec![link(Access::Edit), link(Access::Comment), link(Access::View)]);
    let AdminReply::Links(old) = listing.clone().for_reader(false) else { panic!() };
    assert_eq!(old.iter().map(|l| l.access).collect::<Vec<_>>(), [Access::Edit, Access::View, Access::View]);
    assert_eq!(listing.clone().for_reader(true), listing);
    let made = AdminReply::Link(NewLink { link: link(Access::Comment), token: "t".into() });
    assert!(matches!(made.for_reader(false), AdminReply::Link(l) if l.link.access == Access::View));
    assert!(message::reads_comments(&message::client_version()) && !message::reads_comments("0.12.1 signals hosting"));
}
