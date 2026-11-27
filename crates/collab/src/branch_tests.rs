//! Server-side shared branch sync: capability gating, access control, hostile input and id-space
//! validation, undo of deleted peer layers, aggregate wire-size caps, journal replay, state
//! restore, and concurrent edits.

use figcraft_doc::ids::STYLE_ID_PREFIX;
use figcraft_doc::{Document, NodeId, NodeKind, Style, StyleKind};

use crate::change::{Change, Entry, Json, Prop};
use crate::message::{Access, BranchEvent, BranchReq, BranchStatus, ClientMsg, Hello, MAX_SHARED_BRANCHES, ServerMsg, SharedBranch};
use crate::{Client, Journal, Limits, Outgoing, Server};

fn doc() -> (Document, NodeId, NodeId, NodeId) {
    let mut d = Document::new("Shared");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "Frame", 0.0, 0.0, 200.0, 200.0).unwrap();
    let rect = d.create(frame, NodeKind::Rectangle, "Rect", 10.0, 10.0, 40.0, 40.0).unwrap();
    (d, page, frame, rect)
}

fn hello(name: &str) -> Hello {
    let ClientMsg::Hello(h) = Client::hello("f", "tok", name, "#3366FF") else { panic!("not a hello") };
    h
}

fn join_as(server: &mut Server, h: Hello, access: Access) -> (u32, Vec<Outgoing>) {
    server.join_as(h, access).unwrap()
}

fn join_editor(server: &mut Server, name: &str) -> u32 {
    join_as(server, hello(name), Access::Edit).0
}

fn create_req(id: &str, name: &str, base_seq: u64, base_diff: Vec<Change>, head_diff: Vec<Change>) -> ClientMsg {
    ClientMsg::Branch(Box::new(BranchReq::Create { id: id.into(), name: name.into(), time: 1_700_000_000, base_seq, base_diff, head_diff }))
}

fn make_rect_change(id: NodeId, parent: NodeId, name: &str) -> Change {
    Change::Create {
        id,
        kind: Json::of(&NodeKind::Rectangle).unwrap(),
        parent: Some(parent),
        position: "a0".into(),
        props: vec![(Prop::Name, Json::of(name).unwrap()), (Prop::Width, Json::of(&50.0f64).unwrap()), (Prop::Height, Json::of(&50.0f64).unwrap())],
    }
}

fn put_style(id: String) -> Change {
    let style = Style {
        id: id.clone(),
        name: "Forged".into(),
        description: String::new(),
        kind: StyleKind::Paint { paints: Vec::new() },
        bound: Default::default(),
    };
    Change::Put { entry: Entry::Style(id), value: Json::of(&style).unwrap() }
}

fn is_rejected(outs: &[Outgoing], sender: u32) -> bool {
    matches!(
        outs,
        [Outgoing::To(to, ServerMsg::Branch(ev))] if *to == sender && matches!(ev.as_ref(), BranchEvent::Rejected { .. })
    )
}

#[test]
fn shared_branches_capability_gates_older_clients() {
    let (d, _, _, _) = doc();
    let mut server = Server::new(d, "tok", Limits::default());
    // Older client does not announce the `branches` capability.
    let (old_id, old_out) = join_as(&mut server, Hello { client_version: "0.15.0 signals".into(), ..hello("Old") }, Access::Edit);
    assert!(!old_out.iter().any(|o| matches!(o, Outgoing::To(_, ServerMsg::Branches(_)))));
    // Modern clients receive the initial branch list after Welcome.
    let (a_id, a_out) = join_as(&mut server, hello("Ada"), Access::Edit);
    assert!(a_out.iter().any(|o| matches!(o, Outgoing::To(to, ServerMsg::Branches(list)) if *to == a_id && list.is_empty())));
    let (b_id, _) = join_as(&mut server, hello("Bo"), Access::Edit);
    // Creating a branch broadcasts to branch-capable peers (including sender confirmation) and
    // never to older clients; `client` is stamped from `from`.
    let outs = server.receive(a_id, create_req("b1", "Header", server.seq(), Vec::new(), Vec::new()));
    assert!(!outs.iter().any(|o| matches!(o, Outgoing::To(to, _) if *to == old_id)));
    assert!(outs.iter().any(|o| matches!(
        o,
        Outgoing::To(to, ServerMsg::Branch(ev))
            if *to == a_id && matches!(ev.as_ref(), BranchEvent::Created { author, branch } if *author == a_id && branch.client == a_id)
    )));
    assert!(outs.iter().any(|o| matches!(
        o,
        Outgoing::To(to, ServerMsg::Branch(ev))
            if *to == b_id && matches!(ev.as_ref(), BranchEvent::Created { author, branch } if *author == a_id && branch.client == a_id)
    )));
}

#[test]
fn view_and_comment_clients_cannot_mutate_shared_branches() {
    let (d, _, _, rect) = doc();
    let mut server = Server::new(d, "tok", Limits::default());
    let editor = join_editor(&mut server, "Ada");
    let (viewer, _) = join_as(&mut server, hello("Val"), Access::View);
    let (commenter, _) = join_as(&mut server, hello("Cam"), Access::Comment);
    server.receive(editor, create_req("b1", "Feature", server.seq(), Vec::new(), Vec::new()));
    let before = server.state().unwrap();
    for limited in [viewer, commenter] {
        assert!(is_rejected(&server.receive(limited, create_req("b2", "Sneaky", server.seq(), Vec::new(), Vec::new())), limited));
        assert!(is_rejected(
            &server.receive(
                limited,
                ClientMsg::Branch(Box::new(BranchReq::Edit {
                    id: "b1".into(),
                    changes: vec![Change::Set { id: rect, prop: Prop::Name, value: Json::of("Hacked").unwrap() }],
                }))
            ),
            limited
        ));
        assert!(is_rejected(
            &server.receive(
                limited,
                ClientMsg::Branch(Box::new(BranchReq::UpdateFromMain {
                    id: "b1".into(),
                    base_changes: Vec::new(),
                    head_changes: vec![Change::Set { id: rect, prop: Prop::Name, value: Json::of("Hacked").unwrap() }],
                }))
            ),
            limited
        ));
        assert!(is_rejected(
            &server.receive(limited, ClientMsg::Branch(Box::new(BranchReq::SetState { id: "b1".into(), status: BranchStatus::Archived }))),
            limited
        ));
    }
    assert_eq!(server.state().unwrap(), before);
}

#[test]
fn shared_branch_rejects_hostile_ids_names_limits_images_and_foreign_spaces() {
    let (d, _, frame, rect) = doc();
    let mut server = Server::new(d, "tok", Limits::default());
    let a = join_editor(&mut server, "Ada");
    let b = join_editor(&mut server, "Bo");
    // Bad ids and empty/control-only names are rejected with an explicit Rejected event.
    for bad_id in ["", "../escape", "has space", &"x".repeat(65)] {
        assert!(is_rejected(&server.receive(a, create_req(bad_id, "Valid", server.seq(), Vec::new(), Vec::new())), a));
    }
    assert!(is_rejected(&server.receive(a, create_req("ok-id", " \n\t ", server.seq(), Vec::new(), Vec::new())), a));
    // Create one valid branch to test mutations against.
    assert!(!is_rejected(&server.receive(a, create_req("b1", "Clean", server.seq(), Vec::new(), Vec::new())), a));
    // Invalid geometry (opacity out of 0..=1) and invalid image keys are rejected on branch edit.
    let bad_opacity = Change::Set { id: rect, prop: Prop::Opacity, value: Json::of(&2.5f64).unwrap() };
    assert!(is_rejected(&server.receive(a, ClientMsg::Branch(Box::new(BranchReq::Edit { id: "b1".into(), changes: vec![bad_opacity] }))), a));
    let bad_image = Change::Put {
        entry: Entry::Image("../evil".into()),
        value: Json::of(&serde_json::json!({"width": 1, "height": 1, "bytes": "AA=="})).unwrap(),
    };
    assert!(is_rejected(&server.receive(a, ClientMsg::Branch(Box::new(BranchReq::Edit { id: "b1".into(), changes: vec![bad_image] }))), a));
    // Client B cannot smuggle a node or style in client A's id space via create, edit or updateFromMain.
    let foreign_node = NodeId((u64::from(a) << 32) | 500);
    let foreign_style = format!("{STYLE_ID_PREFIX}{}", (u64::from(a) << 32) | 501);
    assert!(is_rejected(
        &server.receive(b, create_req("smuggle-node", "S", server.seq(), Vec::new(), vec![make_rect_change(foreign_node, frame, "S")])),
        b
    ));
    assert!(is_rejected(&server.receive(b, create_req("smuggle-style", "S", server.seq(), Vec::new(), vec![put_style(foreign_style.clone())])), b));
    assert!(is_rejected(
        &server
            .receive(b, ClientMsg::Branch(Box::new(BranchReq::Edit { id: "b1".into(), changes: vec![make_rect_change(foreign_node, frame, "S")] }))),
        b
    ));
    assert!(is_rejected(
        &server.receive(b, ClientMsg::Branch(Box::new(BranchReq::Edit { id: "b1".into(), changes: vec![put_style(foreign_style.clone())] }))),
        b
    ));
    assert!(is_rejected(
        &server.receive(
            b,
            ClientMsg::Branch(Box::new(BranchReq::UpdateFromMain {
                id: "b1".into(),
                base_changes: Vec::new(),
                head_changes: vec![make_rect_change(foreign_node, frame, "S")],
            }))
        ),
        b
    ));
    assert!(is_rejected(
        &server.receive(
            b,
            ClientMsg::Branch(Box::new(BranchReq::UpdateFromMain {
                id: "b1".into(),
                base_changes: Vec::new(),
                head_changes: vec![put_style(foreign_style.clone())],
            }))
        ),
        b
    ));
    // Because the branch rejected them, main still rejects client B creating client A's ids.
    let reject_node =
        server.receive(b, ClientMsg::Changes { base_seq: server.seq(), batch_id: 1, changes: vec![make_rect_change(foreign_node, frame, "S")] });
    assert!(matches!(reject_node.as_slice(), [Outgoing::To(to, ServerMsg::Reject { .. })] if *to == b));
    let reject_style = server.receive(b, ClientMsg::Changes { base_seq: server.seq(), batch_id: 2, changes: vec![put_style(foreign_style)] });
    assert!(matches!(reject_style.as_slice(), [Outgoing::To(to, ServerMsg::Reject { .. })] if *to == b));
    // Branch count is capped at MAX_SHARED_BRANCHES and the 65th gets an explicit Rejected event.
    for i in 1..MAX_SHARED_BRANCHES {
        assert!(!is_rejected(&server.receive(a, create_req(&format!("cap-{i}"), "B", server.seq(), Vec::new(), Vec::new())), a));
    }
    assert!(is_rejected(&server.receive(a, create_req("over-cap", "Overflow", server.seq(), Vec::new(), Vec::new())), a));
}

#[test]
fn shared_branch_undo_can_restore_a_peer_created_layer_and_style() {
    let (d, _, frame, _) = doc();
    let mut server = Server::new(d, "tok", Limits::default());
    let a = join_editor(&mut server, "Ada");
    let b = join_editor(&mut server, "Bo");
    server.receive(a, create_req("b1", "Undo", server.seq(), Vec::new(), Vec::new()));
    // Ada creates a layer and a style on the branch.
    let ada_node = NodeId((u64::from(a) << 32) | 42);
    let ada_style = format!("{STYLE_ID_PREFIX}{}", (u64::from(a) << 32) | 43);
    assert!(!is_rejected(
        &server.receive(
            a,
            ClientMsg::Branch(Box::new(BranchReq::Edit {
                id: "b1".into(),
                changes: vec![make_rect_change(ada_node, frame, "AdaCard"), put_style(ada_style.clone())],
            }))
        ),
        a
    ));
    // Bo deletes both on the branch.
    assert!(!is_rejected(
        &server.receive(
            b,
            ClientMsg::Branch(Box::new(BranchReq::Edit {
                id: "b1".into(),
                changes: vec![Change::Delete { id: ada_node }, Change::Remove { entry: Entry::Style(ada_style.clone()) }],
            }))
        ),
        b
    ));
    // Bo undoes the deletion (re-creating Ada's ids): server accepts both restorations.
    let outs = server.receive(
        b,
        ClientMsg::Branch(Box::new(BranchReq::Edit {
            id: "b1".into(),
            changes: vec![make_rect_change(ada_node, frame, "AdaCard"), put_style(ada_style)],
        })),
    );
    assert!(!is_rejected(&outs, b));
    assert!(outs.iter().any(|o| matches!(
        o,
        Outgoing::To(to, ServerMsg::Branch(ev)) if *to == a && matches!(ev.as_ref(), BranchEvent::Edited { id, changes, .. } if id == "b1" && !changes.is_empty())
    )));
}

#[test]
fn shared_branch_aggregate_wire_size_is_bounded() {
    let (d, _, frame, _) = doc();
    let mut server = Server::new(d, "tok", Limits { max_message_bytes: 512, ..Limits::default() });
    let a = join_editor(&mut server, "Ada");
    assert!(!is_rejected(&server.receive(a, create_req("b1", "Small", server.seq(), Vec::new(), Vec::new())), a));
    let huge_name = "X".repeat(600);
    let huge_node = NodeId((u64::from(a) << 32) | 1);
    let outs = server
        .receive(a, ClientMsg::Branch(Box::new(BranchReq::Edit { id: "b1".into(), changes: vec![make_rect_change(huge_node, frame, &huge_name)] })));
    assert!(is_rejected(&outs, a), "oversized branch edit must be rejected before bloating ServerMsg::Branches");
    // Joining client receives a ServerMsg::Branches frame that fits within max_message_bytes.
    let (_, join_out) = join_as(&mut server, hello("Bo"), Access::Edit);
    let branches_msg = join_out.into_iter().find_map(|o| match o {
        Outgoing::To(_, m @ ServerMsg::Branches(_)) => Some(m),
        _ => None,
    });
    let encoded = crate::message::encode(&branches_msg.unwrap()).unwrap();
    assert!(encoded.len() <= 512, "encoded branches frame {} > 512", encoded.len());
}

#[test]
fn shared_branch_journal_replay_and_state_round_trip() {
    let (d, _, frame, rect) = doc();
    let mut server = Server::new(d, "tok", Limits::default());
    let a = join_editor(&mut server, "Ada");
    let _b = join_editor(&mut server, "Bo");
    let initial = server.state().unwrap();
    server.journaling(true);
    server.receive(a, create_req("b1", "Replay", server.seq(), Vec::new(), Vec::new()));
    let added = NodeId((u64::from(a) << 32) | 10);
    server.receive(a, ClientMsg::Branch(Box::new(BranchReq::Edit { id: "b1".into(), changes: vec![make_rect_change(added, frame, "Card")] })));
    server.receive(
        a,
        ClientMsg::Branch(Box::new(BranchReq::UpdateFromMain {
            id: "b1".into(),
            base_changes: vec![Change::Set { id: rect, prop: Prop::Name, value: Json::of("MainRect").unwrap() }],
            head_changes: vec![Change::Set { id: rect, prop: Prop::Name, value: Json::of("MainRect").unwrap() }],
        })),
    );
    server.receive(a, ClientMsg::Branch(Box::new(BranchReq::SetState { id: "b1".into(), status: BranchStatus::Archived })));
    let journal = server.take_journal();
    // Verify each branch operation actually produced a journal record and updated server state.
    assert!(matches!(journal.first(), Some(Journal::BranchCreate(_))));
    assert!(journal.iter().any(|j| matches!(j, Journal::BranchEdit { id, rev: 1, .. } if id == "b1")));
    assert!(journal.iter().any(|j| matches!(j, Journal::BranchUpdateFromMain { id, rev: 2, .. } if id == "b1")));
    assert!(journal.iter().any(|j| matches!(j, Journal::BranchState { id, rev: 3, status: BranchStatus::Archived } if id == "b1")));
    let source = server.state().unwrap();
    assert_eq!(source.branches.len(), 1);
    assert_eq!(source.branches[0].rev, 3);
    assert_eq!(source.branches[0].status, BranchStatus::Archived);
    assert!(!source.branches[0].base_diff.is_empty());
    assert!(!source.branches[0].head_diff.is_empty());
    let mut replayed = Server::restore(initial, "tok", Limits::default()).unwrap();
    for entry in journal {
        replayed.replay(entry).unwrap();
    }
    assert_eq!(replayed.state().unwrap(), source);
    // Restoring a snapshot with a u32::MAX branch client id does not saturate next_client.
    let mut state = source;
    state.branches.push(SharedBranch {
        id: "max-client".into(),
        name: "Max".into(),
        time: 0,
        client: u32::MAX,
        status: BranchStatus::Active,
        rev: 0,
        base_diff: Vec::new(),
        head_diff: Vec::new(),
    });
    let mut restored = Server::restore(state, "tok", Limits::default()).unwrap();
    let next_id = join_editor(&mut restored, "Cy");
    assert!(next_id < u32::MAX);
}

#[test]
fn concurrent_edits_on_a_shared_branch_converge_and_echo_to_sender() {
    let (d, _, frame, rect) = doc();
    let mut server = Server::new(d, "tok", Limits::default());
    let a = join_editor(&mut server, "Ada");
    let b = join_editor(&mut server, "Bo");
    server.receive(a, create_req("b1", "CoEdit", server.seq(), Vec::new(), Vec::new()));
    // Ada deletes rect on the branch; Bo concurrently edits the deleted rect AND renames frame.
    let del_outs = server.receive(a, ClientMsg::Branch(Box::new(BranchReq::Edit { id: "b1".into(), changes: vec![Change::Delete { id: rect }] })));
    assert!(!is_rejected(&del_outs, a));
    let outs = server.receive(
        b,
        ClientMsg::Branch(Box::new(BranchReq::Edit {
            id: "b1".into(),
            changes: vec![
                Change::Set { id: rect, prop: Prop::Name, value: Json::of("Stale").unwrap() },
                Change::Set { id: frame, prop: Prop::Name, value: Json::of("Kept").unwrap() },
            ],
        })),
    );
    // The stale edit to the deleted node is skipped while the valid edit to frame echoes to BOTH
    // Ada and Bo in server order.
    for peer in [a, b] {
        assert!(outs.iter().any(|o| matches!(
            o,
            Outgoing::To(to, ServerMsg::Branch(ev))
                if *to == peer && matches!(ev.as_ref(), BranchEvent::Edited { id, changes, .. } if id == "b1" && changes.len() == 1)
        )));
    }
    // A newcomer gets the converged branch (rect gone, frame renamed to "Kept").
    let (_, c_out) = join_as(&mut server, hello("Cy"), Access::Edit);
    let branches = c_out
        .into_iter()
        .find_map(|o| match o {
            Outgoing::To(_, ServerMsg::Branches(list)) => Some(list),
            _ => None,
        })
        .unwrap();
    assert_eq!(branches.len(), 1);
    let mut rep = server.replica().clone();
    rep.apply(&branches[0].base_diff, crate::Mode::Lenient).unwrap();
    rep.apply(&branches[0].head_diff, crate::Mode::Lenient).unwrap();
    assert!(!rep.document().contains(rect));
    assert_eq!(rep.document().node(frame).unwrap().name, "Kept");
}
