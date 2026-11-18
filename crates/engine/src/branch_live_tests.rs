//! Branches of a file in a live session: the branch stays with its author and makes ids in their
//! own client space; merging is one synced edit (and one step of the author's undo history) that
//! everyone receives; peers' edits on main while the branch is open merge against the branch's own
//! base, and conflicts with them resolve as offline; update from main takes them in; a view link
//! still refuses merging; a branch made outside the session that adds layers is refused instead
//! of being sent for the server to refuse. Plus a fuzz test with random real commands.

use figcraft_collab::sim::Rng;
use figcraft_doc::ids::space_of;
use figcraft_doc::{Document, NodeId};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::branch_tests::{base_with_lists, random_edit};
use crate::collab_live_tests::{Room, frame_and_rect};
use crate::collab_tests::base;
use crate::live::{Access, Event, Role, VIEW_ONLY};
use crate::live_tests::{info, push, relay, serve_as, session};

fn ok(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn me(s: &Session) -> u32 {
    s.live().client().unwrap().id()
}

fn version_names(s: &mut Session) -> Vec<String> {
    ok(s, "version.list", json!({})).as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap().to_string()).collect()
}

/// The conflict keys of branch `id`'s review.
fn conflict_keys(s: &mut Session, id: &str) -> Vec<String> {
    let review = ok(s, "branch.review", json!({"id": id}));
    review["conflicts"].as_array().unwrap().iter().map(|c| c["key"].as_str().unwrap().to_string()).collect()
}

#[test]
fn a_branch_merges_into_the_session_as_one_synced_edit() {
    let mut room = Room::new(base(), 2, 7);
    let (f, r) = frame_and_rect(room.server.document());
    let you = me(room.s(0));
    let id = ok(room.s(0), "branch.create", json!({"name": "Explore"}))["id"].as_str().unwrap().to_string();
    // On the branch tab: a recolour, a new layer and a new style.
    ok(room.s(0), "fill.set", json!({"ids": [r], "color": "#FF0000"}));
    let made = NodeId(ok(room.s(0), "shape.rectangle", json!({"x": 900, "y": 900, "width": 10, "height": 10}))["id"].as_u64().unwrap());
    assert_eq!(space_of(made.0), you, "the branch makes ids in your own client space");
    ok(room.s(0), "style.create", json!({"type": "paint", "name": "Brand", "from": r}));
    room.settle();
    assert!(!room.server.document().contains(made), "the branch stays with its author");
    // Someone else edits the main file meanwhile.
    ok(room.s(1), "node.set", json!({"id": f, "props": {"name": "Renamed by a peer"}}));
    room.settle();
    assert_eq!(conflict_keys(room.s(0), &id), Vec::<String>::new());
    assert_eq!(ok(room.s(0), "branch.merge", json!({"id": id}))["merged"], true);
    room.settle();
    room.check();
    let doc = room.server.document();
    assert!(doc.contains(made), "everyone has the merge");
    assert_eq!(doc.node(f).unwrap().name, "Renamed by a peer", "the peer's edit stays");
    assert!(doc.styles.iter().any(|st| st.name == "Brand"));
    assert_ne!(doc.node(r).unwrap().fills, base().node(r).unwrap().fills);
    // The merge is one step of the author's own undo history.
    ok(room.s(0), "edit.undo", json!({}));
    room.settle();
    room.check();
    assert!(!room.server.document().contains(made));
    assert_eq!(room.server.document().node(f).unwrap().name, "Renamed by a peer");
}

#[test]
fn conflicts_with_peers_resolve_as_offline() {
    let mut room = Room::new(base(), 2, 3);
    let (f, r) = frame_and_rect(room.server.document());
    let r2 = room.server.document().node(f).unwrap().children[1];
    let id = ok(room.s(0), "branch.create", json!({}))["id"].as_str().unwrap().to_string();
    ok(room.s(0), "node.set", json!({"id": r, "props": {"name": "Kept on the branch"}}));
    ok(room.s(0), "node.set", json!({"id": r2, "props": {"opacity": 0.5}}));
    // A peer deletes the layer the branch edited and sets the same property another way.
    ok(room.s(1), "edit.delete", json!({"ids": [r]}));
    ok(room.s(1), "node.set", json!({"id": r2, "props": {"opacity": 0.2}}));
    room.settle();
    let keys = conflict_keys(room.s(0), &id);
    let mut want = vec![format!("{r}/layer"), format!("{r2}/opacity")];
    want.sort();
    let mut got = keys.clone();
    got.sort();
    assert_eq!(got, want);
    assert!(room.s(0).execute("branch.merge", &json!({"id": id})).is_err(), "conflicts first");
    // The branch's edits win the deletion, main wins the opacity.
    let picks: Map<String, Value> = keys.iter().map(|k| (k.clone(), json!(if k.ends_with("/layer") { "branch" } else { "main" }))).collect();
    ok(room.s(0), "branch.merge", json!({"id": id, "resolutions": picks}));
    room.settle();
    room.check();
    let doc = room.server.document();
    assert_eq!(doc.node(r).map(|n| n.name.as_str()), Some("Kept on the branch"), "the deleted layer is back, with the branch's edit");
    assert_eq!(doc.node(r2).unwrap().opacity, 0.2);
}

#[test]
fn update_from_main_takes_in_peers_edits_and_shows_in_the_branch_history() {
    let mut room = Room::new(base(), 2, 5);
    let (f, r) = frame_and_rect(room.server.document());
    let id = ok(room.s(0), "branch.create", json!({}))["id"].as_str().unwrap().to_string();
    ok(room.s(0), "node.set", json!({"id": r, "props": {"name": "Branch"}}));
    ok(room.s(1), "node.set", json!({"id": f, "props": {"name": "Peer 1"}}));
    room.settle();
    assert_eq!(ok(room.s(0), "branch.updateFromMain", json!({}))["updated"], true);
    assert_eq!(room.s(0).doc().unwrap().node(f).unwrap().name, "Peer 1", "the branch tab has the peer's edit");
    assert_eq!(version_names(room.s(0)), ["Branch created", "Updated from main"]);
    // The new base is main as it was then: a later peer edit is main's change, not a conflict.
    ok(room.s(1), "node.set", json!({"id": f, "props": {"name": "Peer 2"}}));
    room.settle();
    assert_eq!(conflict_keys(room.s(0), &id), Vec::<String>::new());
    ok(room.s(0), "branch.merge", json!({}));
    room.settle();
    room.check();
    let doc = room.server.document();
    assert_eq!((doc.node(f).unwrap().name.as_str(), doc.node(r).unwrap().name.as_str()), ("Peer 2", "Branch"));
}

#[test]
fn a_view_link_still_refuses_merging() {
    let (mut s, script) = session();
    s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
    let mut doc = Document::new("Shared");
    let page = doc.first_page().unwrap();
    let card = doc.create(page, figcraft_doc::NodeKind::Rectangle, "Card", 0.0, 0.0, 10.0, 10.0).unwrap();
    let (_server, id, welcome) = serve_as(&doc, Access::View);
    let mut you = info(Role::Guest, id);
    you.access = Access::View;
    push(&script, vec![Event::Welcome { info: you, peers: vec![] }, welcome]);
    s.live_poll();
    assert_eq!(s.active_read_only(), Some(VIEW_ONLY));
    // The branch is yours to edit; merging it would edit the file you may only view.
    ok(&mut s, "branch.create", json!({}));
    ok(&mut s, "node.set", json!({"id": card, "props": {"name": "Mine"}}));
    ok(&mut s, "branch.updateFromMain", json!({}));
    match s.execute("branch.merge", &json!({})) {
        Err(crate::EngineError::Disabled(_, why)) => assert_eq!(why, VIEW_ONLY),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(script.lock().unwrap().msgs.is_empty(), "nothing was sent");
}

#[test]
fn a_branch_made_outside_the_session_merges_edits_but_not_new_layers() {
    let (mut s, script) = session();
    let r = NodeId(ok(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap());
    let edits = ok(&mut s, "branch.create", json!({"name": "Edits"}))["id"].as_str().unwrap().to_string();
    ok(&mut s, "node.set", json!({"id": r, "props": {"name": "Edited before the session"}}));
    ok(&mut s, "doc.switch", json!({"index": 0}));
    let adds = ok(&mut s, "branch.create", json!({"name": "Adds"}))["id"].as_str().unwrap().to_string();
    ok(&mut s, "shape.ellipse", json!({"x": 90, "y": 0, "width": 20, "height": 20}));
    ok(&mut s, "doc.switch", json!({"index": 0}));
    s.execute("collab.host", &json!({})).unwrap();
    let (mut server, id, welcome) = serve_as(s.doc().unwrap(), Access::Edit);
    push(&script, vec![welcome]);
    s.live_poll();
    assert!(s.live().syncing());
    // Its new layer is in another id space: refused here rather than by the server.
    match s.execute("branch.merge", &json!({"id": adds})) {
        Err(crate::EngineError::Disabled(_, why)) => assert!(why.contains("outside this live session"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(s.doc().unwrap().node_count(), server.document().node_count(), "nothing merged");
    // Edits of layers everyone has go through.
    ok(&mut s, "branch.merge", json!({"id": edits}));
    let answers = relay(&script, &mut server, id);
    push(&script, answers);
    s.live_poll();
    assert_eq!(server.document().node(r).unwrap().name, "Edited before the session");
    assert!(*s.doc().unwrap() == *server.document());
}

/// The live fuzz test: three people edit with random real commands (export settings and guides
/// included) while one of them works on a branch; every conflict resolved for a random side, the
/// merge goes through the server as it is and everyone converges on it.
#[test]
fn random_edits_on_a_branch_merge_into_a_live_session() {
    let (mut created, mut conflicted) = (0, 0);
    for seed in 0..8 {
        let mut room = Room::new(base_with_lists(), 3, seed);
        let mut rng = Rng::new(seed + 50);
        let id = ok(room.s(0), "branch.create", json!({}))["id"].as_str().unwrap().to_string();
        for _ in 0..30 {
            let i = rng.below(3);
            let (cmd, params) = random_edit(room.s(i).doc().unwrap(), &mut rng, true);
            let _ = room.s(i).execute(cmd, &params);
            if rng.chance(0.3) {
                room.settle();
            }
        }
        room.settle();
        let keys = conflict_keys(room.s(0), &id);
        conflicted += usize::from(!keys.is_empty());
        let picks: Map<String, Value> = keys.into_iter().map(|k| (k, json!(if rng.chance(0.5) { "branch" } else { "main" }))).collect();
        let before = room.server.document().clone();
        ok(room.s(0), "branch.merge", json!({"id": id, "resolutions": picks}));
        let merged = room.s(0).doc().unwrap().clone();
        assert!(merged.validate().is_empty(), "seed {seed}: {:?}", merged.validate());
        created += usize::from(merged.ids().any(|n| !before.contains(n)));
        room.settle();
        room.check();
        assert!(*room.server.document() == merged, "seed {seed}: the server took the merge as it is");
    }
    assert!(created >= 3 && conflicted >= 2, "{created} merges added layers, {conflicted} had conflicts");
}

/// No branch command panics on junk parameters in a live session, from the main file's tab or
/// the branch's, and the session stays in sync.
#[test]
fn branch_commands_survive_junk_params_in_a_live_session() {
    let junk = [
        json!({}),
        json!(null),
        json!([1, 2]),
        json!({"id": 5, "name": 1e308, "resolutions": [1]}),
        json!({"id": "nope", "resolutions": {"1/layer": "both", "doc/name": 7}}),
        json!({"name": "\u{0}".repeat(500), "resolutions": {"x": "branch"}}),
    ];
    let mut room = Room::new(base(), 2, 11);
    ok(room.s(0), "branch.create", json!({}));
    let ids: Vec<&str> = crate::branches::specs().iter().map(|s| s.id).collect();
    for tab in [0, 1] {
        for id in &ids {
            for j in &junk {
                let s = room.s(0);
                if s.docs.len() > tab {
                    let _ = s.execute("doc.switch", &json!({"index": tab}));
                }
                if let Err(crate::EngineError::Internal { cmd, msg }) = s.execute(id, j) {
                    panic!("{cmd} panicked on {j}: {msg}");
                }
                room.settle();
            }
        }
    }
    room.check();
}
