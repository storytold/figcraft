//! Responsive context holds only what the designer stated.

use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s
}

fn frame(s: &mut Session, name: &str, width: u32) -> u64 {
    s.execute("node.create", &json!({"type": "frame", "name": name, "width": width, "height": 800})).unwrap()["id"].as_u64().unwrap()
}

/// What is stated about layer `id`.
fn context(s: &mut Session, id: u64) -> Value {
    s.execute("responsive.list", &json!({"ids": [id]})).unwrap()
}

#[test]
fn nothing_is_inferred_from_names_or_sizes() {
    let mut s = session();
    let phone = frame(&mut s, "Home / Mobile", 390);
    frame(&mut s, "Home / Desktop", 1440);
    let v = context(&mut s, phone);
    assert_eq!(v, json!({"explicit": false, "versions": [], "rules": []}));
}

#[test]
fn marked_frames_are_told_apart_and_come_with_their_alternates() {
    let mut s = session();
    let phone = frame(&mut s, "Home phone", 390);
    let tablet = frame(&mut s, "Home tablet", 768);
    let desk = frame(&mut s, "Home large", 1440);
    let other = frame(&mut s, "About", 1440);
    for (id, viewport) in [(desk, "desktop"), (phone, "mobile"), (tablet, "tablet")] {
        s.execute("responsive.set", &json!({"id": id, "viewport": viewport, "group": "Home"})).unwrap();
    }
    s.execute("responsive.set", &json!({"id": other, "viewport": "desktop"})).unwrap();
    let v = context(&mut s, desk);
    assert_eq!(v["explicit"], true);
    let versions = v["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 1, "other designs stay out: {versions:?}");
    let frames: Vec<&str> = versions[0]["frames"].as_array().unwrap().iter().map(|f| f["viewport"].as_str().unwrap()).collect();
    assert_eq!(frames, ["mobile", "tablet", "desktop"], "narrowest first");
    let all = s.execute("responsive.list", &json!({})).unwrap();
    assert_eq!(all["versions"].as_array().unwrap().len(), 2);
}

#[test]
fn a_rule_is_the_designers_words_in_their_category() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    let side = s.execute("node.create", &json!({"type": "frame", "parent": f, "name": "Sidebar", "width": 240, "height": 800})).unwrap()["id"]
        .as_u64()
        .unwrap();
    s.execute("responsive.rule", &json!({"id": side, "text": "Sidebar becomes a drawer below 768 px"})).unwrap();
    s.execute("annotation.add", &json!({"id": side, "label": "Ordinary note"})).unwrap();
    let v = context(&mut s, f);
    let rules = v["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!((rules[0]["layer"].as_str(), rules[0]["text"].as_str()), (Some("Sidebar"), Some("Sidebar becomes a drawer below 768 px")));
    let kept = s.execute("annotation.categories", &json!({})).unwrap();
    assert_eq!(kept["categories"].as_array().unwrap().iter().filter(|c| c["name"] == "Responsive").count(), 1);
    s.execute("responsive.rule", &json!({"id": f, "text": "Second"})).unwrap();
    let kept = s.execute("annotation.categories", &json!({})).unwrap();
    assert_eq!(kept["categories"].as_array().unwrap().iter().filter(|c| c["name"] == "Responsive").count(), 1, "not made twice");
}

#[test]
fn a_rule_on_the_frame_reaches_a_layer_read_on_its_own() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    let child = s.execute("node.create", &json!({"type": "rectangle", "parent": f, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("responsive.rule", &json!({"id": f, "text": "Cards stack on mobile"})).unwrap();
    assert_eq!(context(&mut s, child)["rules"][0]["text"], "Cards stack on mobile");
}

#[test]
fn marking_is_undoable_saved_and_clearable() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    s.execute("responsive.set", &json!({"id": f, "viewport": "desktop", "group": "Home"})).unwrap();
    let path = std::env::temp_dir().join(format!("figcraft-responsive-{}.figcraft", std::process::id()));
    s.execute("file.save", &json!({"path": path})).unwrap();
    let mut back = Session::new();
    back.execute("file.open", &json!({"path": path})).unwrap();
    assert_eq!(crate::responsive::groups(back.doc().unwrap()).len(), 1, "it survives saving");
    let _ = std::fs::remove_file(&path);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(crate::responsive::groups(s.doc().unwrap()).is_empty());
    s.execute("responsive.set", &json!({"id": f, "viewport": "mobile"})).unwrap();
    s.execute("responsive.set", &json!({"id": f, "viewport": null})).unwrap();
    assert!(crate::responsive::groups(s.doc().unwrap()).is_empty());
}

#[test]
fn bad_input_is_refused() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    let r = s.execute("node.create", &json!({"type": "rectangle", "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    for p in [
        json!({"id": f}),
        json!({"id": f, "viewport": "watch"}),
        json!({"id": r, "viewport": "mobile"}),
        json!({"id": f, "viewport": "mobile", "group": "  "}),
    ] {
        assert!(s.execute("responsive.set", &p).is_err(), "{p}");
    }
    assert!(s.execute("responsive.rule", &json!({"id": f, "text": " "})).is_err());
    assert!(s.execute("responsive.rule", &json!({"id": f})).is_err());
}

#[test]
fn malformed_targets_are_refused_rather_than_read_as_the_selection() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let calls = [
        ("responsive.set", json!({"ids": [f, "nope"], "viewport": "mobile"})),
        ("responsive.set", json!({"ids": [999_999], "viewport": "mobile"})),
        ("responsive.rule", json!({"id": "not-an-id", "text": "A rule"})),
        ("responsive.list", json!({"id": "not-an-id"})),
        ("responsive.list", json!({"ids": [f, "nope"]})),
        ("responsive.list", json!({"ids": [999_999]})),
    ];
    for (command, p) in calls {
        assert!(s.execute(command, &p).is_err(), "{command} {p}");
    }
    assert!(s.doc().unwrap().node(figcraft_doc::NodeId(f)).unwrap().annotations.is_empty(), "the selection wasn't used instead");
}

#[test]
fn a_rule_keeps_to_the_limits_annotations_have() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    let long = "x".repeat(crate::annotations::MAX_LABEL + 50);
    let at = s.execute("responsive.rule", &json!({"id": f, "text": long})).unwrap()["index"].as_u64().unwrap() as usize;
    let label = s.doc().unwrap().node(figcraft_doc::NodeId(f)).unwrap().annotations[at].label.chars().count();
    assert_eq!(label, crate::annotations::MAX_LABEL, "capped like annotation.add");
    while s.doc().unwrap().node(figcraft_doc::NodeId(f)).unwrap().annotations.len() < crate::annotations::MAX_PER_LAYER {
        s.execute("annotation.add", &json!({"id": f, "label": "note"})).unwrap();
    }
    assert!(s.execute("responsive.rule", &json!({"id": f, "text": "One too many"})).is_err(), "a layer's annotation limit holds");
}

#[test]
fn no_responsive_category_is_made_past_the_category_limit() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    let mut k = 0;
    while s.execute("annotation.categories", &json!({})).unwrap()["categories"].as_array().unwrap().len() < crate::annotations::MAX_CATEGORIES {
        s.execute("annotation.addCategory", &json!({"name": format!("Category {k}")})).unwrap();
        k += 1;
    }
    assert!(s.execute("responsive.rule", &json!({"id": f, "text": "A rule"})).is_err());
}

#[test]
fn a_walk_stopped_at_its_cap_says_so() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    for _ in 0..10 {
        s.execute("node.create", &json!({"type": "rectangle", "parent": f, "width": 4, "height": 4})).unwrap();
    }
    let doc = s.doc().unwrap();
    let id = figcraft_doc::NodeId(f);
    assert!(crate::responsive::context_within(doc, &[id], 100).get("truncated").is_none());
    assert_eq!(crate::responsive::context_within(doc, &[id], 5)["truncated"], true);
}

#[test]
fn overlapping_targets_count_each_layer_once() {
    let mut s = session();
    let a = frame(&mut s, "A", 1440);
    let c = s.execute("node.create", &json!({"type": "frame", "parent": a, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("node.create", &json!({"type": "rectangle", "parent": c, "width": 4, "height": 4})).unwrap();
    let b = frame(&mut s, "B", 390);
    s.execute("responsive.rule", &json!({"id": b, "text": "B stacks"})).unwrap();
    let doc = s.doc().unwrap();
    let ids = [figcraft_doc::NodeId(a), figcraft_doc::NodeId(c), figcraft_doc::NodeId(b)];
    let distinct = {
        let mut all: Vec<_> = ids.iter().flat_map(|i| doc.ancestors(*i).into_iter().chain(doc.subtree(*i))).collect();
        all.sort_unstable();
        all.dedup();
        all.len()
    };
    let v = crate::responsive::context_within(doc, &ids, distinct);
    assert!(v.get("truncated").is_none(), "{v}");
    assert_eq!(v["rules"][0]["text"], "B stacks", "{v}");
    assert_eq!(crate::responsive::context_within(doc, &ids, distinct - 1)["truncated"], true);
}

#[test]
fn a_page_takes_no_rule() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    let page = s.doc().unwrap().node(figcraft_doc::NodeId(f)).unwrap().parent.unwrap();
    let before = s.execute("annotation.categories", &json!({})).unwrap();
    assert!(s.execute("responsive.rule", &json!({"id": page.0, "text": "A rule"})).is_err());
    assert!(s.doc().unwrap().node(page).unwrap().annotations.is_empty());
    assert_eq!(s.execute("annotation.categories", &json!({})).unwrap(), before, "no category made");
}

#[test]
fn frames_and_sections_are_marked_and_components_are_not() {
    let mut s = session();
    let f = frame(&mut s, "Card", 300);
    let c = s.execute("component.create", &json!({"ids": [f]})).unwrap()["id"].as_u64().unwrap();
    let i = s.execute("instance.create", &json!({"component": c})).unwrap()["id"].as_u64().unwrap();
    for id in [c, i] {
        assert!(s.execute("responsive.set", &json!({"ids": [id], "viewport": "mobile"})).is_err(), "{id}");
    }
    let section =
        s.execute("node.create", &json!({"type": "section", "x": 2000, "y": 0, "width": 400, "height": 400})).unwrap()["id"].as_u64().unwrap();
    let home = frame(&mut s, "Home", 1440);
    s.execute("responsive.set", &json!({"ids": [section, home], "viewport": "desktop"})).unwrap();
}

#[test]
fn stored_values_that_arent_marks_are_left_out() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    let odd = frame(&mut s, "Odd", 800);
    s.execute("node.set", &json!({"ids": [odd], "props": {"pluginData": {"figcraft.viewport": "watch"}}})).unwrap();
    let long = "g".repeat(500);
    s.execute("node.set", &json!({"ids": [f], "props": {"pluginData": {"figcraft.viewport": "desktop", "figcraft.viewportGroup": long}}})).unwrap();
    let v = s.execute("responsive.list", &json!({})).unwrap();
    let versions = v["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 1, "{v}");
    assert_eq!(versions[0]["group"], "Home", "an over-long group reads as none: {v}");
    assert_eq!(versions[0]["frames"][0]["id"], f);
}

#[test]
fn a_group_can_go_back_to_the_frames_name() {
    let mut s = session();
    let f = frame(&mut s, "Home", 1440);
    s.execute("responsive.set", &json!({"ids": [f], "viewport": "desktop", "group": "Landing"})).unwrap();
    assert_eq!(s.execute("responsive.list", &json!({})).unwrap()["versions"][0]["group"], "Landing");
    s.execute("responsive.set", &json!({"ids": [f], "viewport": "desktop", "group": null})).unwrap();
    assert_eq!(s.execute("responsive.list", &json!({})).unwrap()["versions"][0]["group"], "Home");
}

#[test]
fn developer_mode_makes_its_own_responsive_edits() {
    for id in ["responsive.set", "responsive.rule", "responsive.list"] {
        assert!(crate::devinspect::dev_mode_allows(id), "{id}");
    }
}
