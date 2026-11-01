//! A client renamed in a session (a `name` signal): the server keeps the new name and tells
//! everyone with the client's `PeerJoined`; newcomers' welcomes list it (#1380).

use figcraft_doc::Document;

use crate::message::{ClientMsg, Hello, Peer, ServerMsg, Signal, client_version};
use crate::{Client, Limits, Outgoing, Server};

fn hello(name: &str) -> Hello {
    let ClientMsg::Hello(h) = Client::hello("f", "", name, "#FF8800") else { panic!("not a hello") };
    Hello { client_version: client_version(), ..h }
}

fn rename(s: &mut Server, from: u32, name: &str) -> Vec<Outgoing> {
    s.receive(from, ClientMsg::Signal(Signal::name(name)))
}

#[test]
fn the_new_name_goes_to_everyone_and_to_newcomers() {
    let mut s = Server::new(Document::new("Shared"), "", Limits::default());
    let (ada, _) = s.join(hello("Ada")).unwrap();
    s.join(hello("Hana")).unwrap();
    let out = rename(&mut s, ada, "  Bob\u{7} ");
    let bob = Peer { client_id: ada, name: "Bob".into(), color: "#FF8800".into() };
    assert_eq!(out, vec![Outgoing::All(ServerMsg::PeerJoined(bob))], "cleaned, to everyone, Ada too");
    let (_, out) = s.join(hello("Lin")).unwrap();
    let Some(Outgoing::To(_, ServerMsg::Welcome { peers, .. })) = out.first() else { panic!("no welcome") };
    assert!(peers.iter().any(|p| p.client_id == ada && p.name == "Bob"));
    // The same name, a blank one, a stranger's and a name past the server's limit change nothing.
    assert!(rename(&mut s, ada, "Bob").is_empty());
    assert!(rename(&mut s, ada, " \n ").is_empty());
    assert!(!rename(&mut s, 999, "Eve").iter().any(|o| matches!(o, Outgoing::All(ServerMsg::PeerJoined(_)))));
    let mut tight = Server::new(Document::new("Shared"), "", Limits { max_name: 7, ..Limits::default() });
    let (id, _) = tight.join(hello("Ada")).unwrap();
    assert!(rename(&mut tight, id, "Long name").is_empty());
    assert_eq!(rename(&mut tight, id, "Bo").len(), 1);
}

#[test]
fn a_name_is_cut_to_the_longest_shown() {
    let long = "é".repeat(Signal::MAX_NAME + 10);
    assert_eq!(Signal::name(&long).clean().map(|s| s.text.chars().count()), Some(Signal::MAX_NAME));
    assert_eq!(Signal::name("\t").clean(), None);
}
