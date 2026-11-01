//! Branches through commands: create, edit on both sides, review, resolve, merge as one undo step,
//! update from main, archive, saving with the file, and a fuzz test over random real commands.

use figcraft_collab::merge::{Key, Side, plan};
use figcraft_collab::sim::Rng;
use figcraft_collab::{Change, DocProp, Mode, Prop, Replica, Target};
use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::collab_tests::{base, random_command};

fn ok(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

/// The base file open, with a branch of it made and shown: (session, branch id, the rectangles).
fn branched() -> (Session, String, Vec<NodeId>) {
    let mut s = Session::with_document(base());
    let rects: Vec<NodeId> = s.doc().unwrap().nodes().filter(|n| matches!(n.kind, NodeKind::Rectangle)).map(|n| n.id).collect();
    let b = ok(&mut s, "branch.create", json!({"name": "Explore"}));
    assert_eq!(b["index"], 1);
    assert_eq!(s.active, 1);
    (s, b["id"].as_str().unwrap().to_string(), rects)
}

fn fill(s: &Session, id: NodeId) -> String {
    serde_json::to_string(&s.doc().unwrap().node(id).unwrap().fills).unwrap()
}

#[test]
fn edits_on_both_sides_merge_as_one_undo_step() {
    let (mut s, id, r) = branched();
    ok(&mut s, "fill.set", json!({"ids": [r[0]], "color": "#FF0000"}));
    let made = ok(&mut s, "shape.rectangle", json!({"x": 900, "y": 900, "width": 10, "height": 10}))["id"].clone();
    // Main is edited meanwhile; the branch edit made it unsaved.
    ok(&mut s, "doc.switch", json!({"index": 0}));
    assert!(s.ds().unwrap().is_dirty());
    ok(&mut s, "fill.set", json!({"ids": [r[1]], "color": "#00FF00"}));
    let review = ok(&mut s, "branch.review", json!({"id": id}));
    assert_eq!(review["conflicts"], json!([]));
    assert!(review["branch"]["layers"].as_array().unwrap().iter().any(|l| l["id"] == json!(r[0].0)), "{review}");
    assert!(review["main"]["layers"].as_array().unwrap().iter().any(|l| l["id"] == json!(r[1].0)));
    let main_before = fill(&s, r[1]);
    let merged = ok(&mut s, "branch.merge", json!({"id": id}));
    assert_eq!(merged["merged"], true);
    assert_eq!(s.docs.len(), 1, "the branch tab closes");
    let doc = s.doc().unwrap();
    assert!(doc.node(figcraft_doc::NodeId(made.as_u64().unwrap())).is_some());
    assert!(fill(&s, r[0]).contains("1.0") && fill(&s, r[1]) == main_before);
    assert_eq!(ok(&mut s, "branch.list", json!({}))[0]["state"], "merged");
    // One undo step takes the merge back.
    ok(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().node(figcraft_doc::NodeId(made.as_u64().unwrap())).is_none());
    assert_eq!(fill(&s, r[1]), main_before);
    assert!(s.execute("branch.merge", &json!({"id": id})).is_err(), "a merged branch can't merge again");
}

#[test]
fn conflicts_must_be_resolved_and_either_side_can_win() {
    for (pick, want) in [("branch", "#FF0000"), ("main", "#0000FF")] {
        let (mut s, id, r) = branched();
        ok(&mut s, "fill.set", json!({"ids": [r[0]], "color": "#FF0000"}));
        ok(&mut s, "doc.switch", json!({"index": 0}));
        ok(&mut s, "fill.set", json!({"ids": [r[0]], "color": "#0000FF"}));
        let review = ok(&mut s, "branch.review", json!({"id": id}));
        let c = &review["conflicts"][0];
        assert_eq!(review["conflicts"].as_array().unwrap().len(), 1, "{review}");
        let key = c["key"].as_str().unwrap().to_string();
        assert_eq!(key, format!("{}/fills", r[0]));
        assert_eq!(c["property"], "fills");
        assert!(c["branch"].as_str().unwrap().contains("FF0000") && c["main"].as_str().unwrap().contains("0000FF"), "{c}");
        let err = s.execute("branch.merge", &json!({"id": id})).unwrap_err().to_string();
        assert!(err.contains(&key), "{err}");
        assert!(s.execute("branch.merge", &json!({"id": id, "resolutions": {&key: "both"}})).is_err());
        ok(&mut s, "branch.merge", json!({"id": id, "resolutions": {&key: pick}}));
        let want = Session::with_document({
            let mut t = Session::with_document(s.doc().unwrap().clone());
            ok(&mut t, "fill.set", json!({"ids": [r[0]], "color": want}));
            t.doc().unwrap().clone()
        });
        assert_eq!(fill(&s, r[0]), fill(&want, r[0]), "{pick} wins");
    }
}

#[test]
fn deleting_what_main_edited_is_an_explicit_conflict() {
    let (mut s, id, r) = branched();
    ok(&mut s, "edit.delete", json!({"ids": [r[2]]}));
    ok(&mut s, "doc.switch", json!({"index": 0}));
    ok(&mut s, "node.set", json!({"id": r[2], "props": {"name": "Keep me"}}));
    let review = ok(&mut s, "branch.review", json!({"id": id}));
    let c = &review["conflicts"][0];
    assert_eq!(
        (c["key"].as_str().unwrap(), &c["deletedBy"], &c["branch"], &c["main"]),
        (format!("{}/layer", r[2]).as_str(), &json!("branch"), &json!("deleted"), &json!("edited"))
    );
    assert!(s.execute("branch.merge", &json!({"id": id})).is_err(), "never dropped silently");
    ok(&mut s, "branch.merge", json!({"id": id, "resolutions": {c["key"].as_str().unwrap(): "main"}}));
    assert_eq!(s.doc().unwrap().node(r[2]).unwrap().name, "Keep me");
}

#[test]
fn update_from_main_then_merge() {
    let (mut s, _, r) = branched();
    ok(&mut s, "node.set", json!({"id": r[0], "props": {"opacity": 0.5}}));
    ok(&mut s, "doc.switch", json!({"index": 0}));
    ok(&mut s, "node.set", json!({"id": r[1], "props": {"opacity": 0.25}}));
    ok(&mut s, "node.set", json!({"id": r[0], "props": {"name": "Main name"}}));
    ok(&mut s, "doc.switch", json!({"index": 1}));
    ok(&mut s, "branch.updateFromMain", json!({}));
    let tab = s.doc().unwrap();
    assert_eq!((tab.node(r[0]).unwrap().opacity, tab.node(r[1]).unwrap().opacity), (0.5, 0.25));
    assert_eq!(tab.node(r[0]).unwrap().name, "Main name");
    assert!(s.ds().unwrap().history.undo.is_empty(), "undo can't step past the new base");
    // Main's changes are now in the branch's base: nothing on main to review any more.
    let review = ok(&mut s, "branch.review", json!({}));
    assert_eq!(review["main"]["layers"], json!([]));
    // A conflicting update needs a resolution too.
    ok(&mut s, "node.set", json!({"id": r[2], "props": {"opacity": 0.1}}));
    ok(&mut s, "doc.switch", json!({"index": 0}));
    ok(&mut s, "node.set", json!({"id": r[2], "props": {"opacity": 0.9}}));
    ok(&mut s, "doc.switch", json!({"index": 1}));
    let key = format!("{}/opacity", r[2]);
    assert!(s.execute("branch.updateFromMain", &json!({})).is_err());
    ok(&mut s, "branch.updateFromMain", json!({"resolutions": {&key: "branch"}}));
    assert_eq!(s.doc().unwrap().node(r[2]).unwrap().opacity, 0.1);
    ok(&mut s, "branch.merge", json!({}));
    let doc = s.doc().unwrap();
    assert_eq!(s.active, 0);
    assert_eq!((doc.node(r[0]).unwrap().opacity, doc.node(r[2]).unwrap().opacity), (0.5, 0.1));
}

#[test]
fn styles_variables_and_comments_merge() {
    let (mut s, id, r) = branched();
    ok(&mut s, "fill.set", json!({"ids": [r[0]], "color": "#123456"}));
    ok(&mut s, "style.create", json!({"type": "paint", "name": "Brand", "from": r[0]}));
    let page = s.ds().unwrap().page;
    ok(&mut s, "comment.add", json!({"x": 1, "y": 2, "text": "On the branch", "page": page}));
    ok(&mut s, "doc.switch", json!({"index": 0}));
    ok(&mut s, "variable.create", json!({"collection": "Tokens", "name": "accent", "type": "color", "value": "#FF00FF"}));
    let review = ok(&mut s, "branch.review", json!({"id": id}));
    let what: Vec<&str> = review["branch"]["document"].as_array().unwrap().iter().map(|d| d["what"].as_str().unwrap()).collect();
    assert!(what.contains(&"Style “Brand”") && what.contains(&"Comment “On the branch”"), "{what:?}");
    assert!(review["main"]["document"].as_array().unwrap().iter().any(|d| d["what"] == "Variable “accent”" && d["change"] == "added"));
    ok(&mut s, "branch.merge", json!({"id": id}));
    let doc = s.doc().unwrap();
    assert!(doc.styles.iter().any(|st| st.name == "Brand"));
    assert!(doc.comments.iter().any(|c| c.first.text == "On the branch"));
    assert!(doc.variables.variables.iter().any(|v| v.name == "accent"));
}

#[test]
fn branch_tabs_save_with_the_main_file_and_come_back() {
    let dir = std::env::temp_dir().join(format!("figcraft-branches-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("file.figcraft").to_string_lossy().to_string();
    let (mut s, id, r) = branched();
    ok(&mut s, "node.set", json!({"id": r[0], "props": {"name": "Branch edit"}}));
    // Saving on the branch tab saves the main file, branch included.
    ok(&mut s, "file.saveAs", json!({"path": path}));
    assert_eq!(s.docs[0].path.as_deref(), Some(path.as_str()));
    assert!(!s.docs[0].is_dirty() && !s.docs[1].is_dirty());
    // Closing the main file closes its branch tabs.
    ok(&mut s, "file.close", json!({"index": 0}));
    assert!(s.docs.is_empty());
    let mut t = Session::empty();
    ok(&mut t, "file.open", json!({"path": path}));
    let list = ok(&mut t, "branch.list", json!({}));
    assert_eq!((list[0]["id"].as_str().unwrap(), &list[0]["name"], &list[0]["state"]), (id.as_str(), &json!("Explore"), &json!("active")));
    ok(&mut t, "branch.open", json!({"id": id}));
    assert_eq!(t.doc().unwrap().node(r[0]).unwrap().name, "Branch edit");
    assert!(!t.docs[0].is_dirty(), "opening a branch is no edit");
    // New layers on the reopened branch are made in its own id space.
    let made = ok(&mut t, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].as_u64().unwrap();
    assert!(made >> 32 > 0, "{made}");
    // Archiving keeps it in the file and closes its tab.
    ok(&mut t, "branch.archive", json!({}));
    assert_eq!(t.docs.len(), 1);
    assert_eq!(ok(&mut t, "branch.list", json!({}))[0]["state"], "archived");
    assert!(t.execute("branch.merge", &json!({"id": id})).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn branch_commands_refuse_what_makes_no_sense() {
    let (mut s, _, _) = branched();
    assert!(s.execute("branch.create", &json!({})).is_err(), "no branches of branches");
    assert!(s.execute("branch.open", &json!({"id": "nope"})).is_err());
    ok(&mut s, "doc.switch", json!({"index": 0}));
    assert!(s.execute("branch.review", &json!({})).is_err(), "main needs an id");
    assert!(s.execute("branch.merge", &json!({"id": "nope"})).is_err());
}

/// Every change a side made (that the other side didn't overwrite) shows in `doc`.
fn shows(changes: &[Change], doc: &Document) -> Result<(), String> {
    let r = Replica::new(doc.clone());
    for c in changes {
        let ok = match c {
            Change::Create { id, .. } => doc.contains(*id),
            Change::Delete { id } => !doc.contains(*id),
            Change::Set { prop: figcraft_collab::Prop::Position, .. } => true,
            Change::Set { id, prop, value } => r.value(&Target::Node(*id, *prop)).ok().flatten().as_ref() == Some(value),
            Change::SetDoc { prop: DocProp::StyleOrder | DocProp::CommentOrder | DocProp::CollectionOrder | DocProp::VariableOrder, .. } => true,
            Change::SetDoc { prop, value } => r.value(&Target::Doc(*prop)).ok().flatten().as_ref() == Some(value),
            Change::Put { entry, value } => r.value(&Target::Entry(entry.clone())).ok().flatten().as_ref() == Some(value),
            Change::Remove { entry: figcraft_collab::Entry::Image(_) } => true,
            Change::Remove { entry } => r.value(&Target::Entry(entry.clone())).ok().flatten().is_none(),
        };
        if !ok {
            return Err(format!("{c:?}"));
        }
    }
    Ok(())
}

/// [`base`] with two export settings and two guides on its first frame (lists that merge item by
/// item).
pub(crate) fn base_with_lists() -> Document {
    let mut doc = base();
    let f = doc.node(doc.first_page().unwrap()).unwrap().children[0];
    let n = doc.get_mut(f).unwrap();
    n.exports = ["@1", "@2"].map(|suffix| figcraft_doc::ExportSetting { suffix: suffix.into(), ..Default::default() }).to_vec();
    n.guides = vec![figcraft_doc::Guide { vertical: true, offset: 10.0 }, figcraft_doc::Guide { vertical: false, offset: 20.0 }];
    doc
}

/// A random real command: often an edit of a frame's export settings or guides, else
/// [`random_command`].
pub(crate) fn random_edit(doc: &Document, rng: &mut Rng, with_undo: bool) -> (&'static str, Value) {
    if rng.chance(0.6) {
        return random_command(doc, rng, with_undo);
    }
    let frames: Vec<NodeId> = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Frame)).map(|n| n.id).collect();
    let f = rng.pick(&frames).map_or(json!(0), |i| json!(i));
    let i = rng.below(3);
    match rng.below(6) {
        0 => ("export.add", json!({"ids": [f]})),
        1 => ("export.update", json!({"ids": [f], "index": i, "suffix": format!("@{}", rng.below(9))})),
        2 => ("export.remove", json!({"ids": [f], "index": i})),
        3 => ("guide.add", json!({"frame": f, "vertical": rng.chance(0.5), "offset": rng.below(300)})),
        4 => ("guide.move", json!({"frame": f, "index": i, "offset": rng.below(300)})),
        _ => ("guide.remove", json!({"frame": f, "index": i})),
    }
}

/// The fuzz test: random real commands on a branch and on main (export settings and guides
/// included). Without conflicts the merge (both ways) equals applying both change sets and shows
/// every change of either side; with conflicts, resolving them all for either side gives a valid
/// document.
#[test]
fn random_edits_on_both_sides_merge() {
    let (mut disjoint, mut conflicted, mut lists) = (0, 0, 0);
    for seed in 0..60 {
        let mut rng = Rng::new(seed);
        let start = base_with_lists();
        let mut on_branch = start.clone();
        on_branch.set_client_id(start.highest_id_space() + 1);
        let mut sides = [Session::with_document(on_branch), Session::with_document(start.clone())];
        for s in &mut sides {
            for _ in 0..1 + rng.below(5) {
                let (cmd, params) = random_edit(s.doc().unwrap(), &mut rng, false);
                let _ = s.execute(cmd, &params);
            }
        }
        let [b, m] = sides.map(|s| s.doc().unwrap().clone());
        let p = plan(&start, &b, &m).unwrap();
        // Export settings or guides of a layer changed on both sides that merged item by item.
        let list_writes = |changes: &[Change]| -> std::collections::BTreeSet<Key> {
            changes
                .iter()
                .filter_map(|c| match c {
                    Change::Set { id, prop: prop @ (Prop::Exports | Prop::Guides), .. } => Some(Key::Node(*id, *prop)),
                    _ => None,
                })
                .collect()
        };
        lists += list_writes(&p.branch).intersection(&list_writes(&p.main)).filter(|k| !p.conflicts.iter().any(|c| c.key == **k)).count();
        if p.conflicts.is_empty() {
            disjoint += 1;
            let none = |_: &figcraft_collab::merge::Key| None;
            let (into_main, _) = p.merge(&m, Side::Branch, &none).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            let (into_branch, _) = p.merge(&b, Side::Main, &none).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            let mut both = Replica::new(start.clone());
            both.apply(&p.main, Mode::Strict).unwrap();
            both.apply(&p.branch, Mode::Strict).unwrap();
            let mut want = both.document().clone();
            (want.name, want.current_page) = (m.name.clone(), m.current_page);
            assert_eq!(into_main, want, "seed {seed}");
            for doc in [&into_main, &into_branch] {
                assert!(doc.validate().is_empty(), "seed {seed}: {:?}", doc.validate());
                for side in [&p.branch, &p.main] {
                    shows(side, doc).unwrap_or_else(|c| panic!("seed {seed}: lost {c}"));
                }
            }
        } else {
            conflicted += 1;
            for side in [Side::Branch, Side::Main] {
                let (doc, _) = p.merge(&m, Side::Branch, &|_| Some(side)).unwrap_or_else(|e| panic!("seed {seed} ({side:?} wins): {e}"));
                assert!(doc.validate().is_empty(), "seed {seed}: {:?}", doc.validate());
            }
        }
    }
    assert!(disjoint >= 10 && conflicted >= 5 && lists >= 3, "{disjoint} disjoint, {conflicted} with conflicts, {lists} lists merged item by item");
}
