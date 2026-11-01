//! Layers panel and Assets panel behaviour suite, written from the public documentation of design
//! tools: what the panels' gestures do to the document (reordering and moving layers, renaming
//! several at once, showing, hiding and locking, pages and dividers, the Assets panel's listing,
//! hidden names and drops). Each rule is a table row with a one-line derivation; where the
//! documentation is silent the row says so and pins today's behaviour. The gestures themselves
//! (clicks, Shift ranges, drags in the panel) are covered in the UI crate's
//! `layers_behaviour_tests`.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind};
use figcraft_doc::NodeId;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rect(s: &mut Session, name: &str, x: f64, y: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": 40, "height": 40, "props": {"name": name}});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&ex(s, "shape.rectangle", p))
}

fn frame(s: &mut Session, name: &str, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id(&ex(s, "frame.create", json!({"x": x, "y": y, "width": w, "height": h, "name": name})))
}

fn kids(s: &Session, id: NodeId) -> Vec<NodeId> {
    s.doc().unwrap().node(id).unwrap().children.clone()
}

fn name(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().name.clone()
}

fn world(s: &Session, id: NodeId) -> [f64; 6] {
    s.doc().unwrap().world_transform(id).as_coeffs()
}

fn same(a: [f64; 6], b: [f64; 6]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

fn click(s: &mut Session, x: f64, y: f64) {
    s.pointer(&PointerEvent::at(PointerKind::Down, x, y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, x, y, 1.0)).unwrap();
}

/// Frame `f` (0,0 400×300) holding `a`, `b`, `c`, `d` bottom to top; the group `g` of `e` on the
/// page; the auto layout frame `al` (500,0) of `x` and `y`; an instance `inst` (0,400) of a
/// component; a section `sec` (900,0).
#[allow(dead_code)]
struct Scene {
    f: NodeId,
    a: NodeId,
    b: NodeId,
    c: NodeId,
    d: NodeId,
    g: NodeId,
    e: NodeId,
    al: NodeId,
    x: NodeId,
    y: NodeId,
    inst: NodeId,
    sec: NodeId,
}

fn scene(s: &mut Session) -> Scene {
    let f = frame(s, "F", 0.0, 0.0, 400.0, 300.0);
    let a = rect(s, "a", 10.0, 10.0, Some(f));
    let b = rect(s, "b", 60.0, 10.0, Some(f));
    let c = rect(s, "c", 110.0, 10.0, Some(f));
    let d = rect(s, "d", 160.0, 10.0, Some(f));
    let e = rect(s, "e", 0.0, 600.0, None);
    let g = id(&ex(s, "object.group", json!({"ids": [e]})));
    let al = frame(s, "AL", 500.0, 0.0, 300.0, 100.0);
    let x = rect(s, "x", 0.0, 0.0, Some(al));
    let y = rect(s, "y", 50.0, 0.0, Some(al));
    ex(s, "layout.add", json!({"ids": [al], "direction": "horizontal"}));
    let main = frame(s, "Main", 0.0, 800.0, 100.0, 40.0);
    rect(s, "Bg", 0.0, 0.0, Some(main));
    let comp = id(&ex(s, "component.create", json!({"ids": [main]})));
    let inst = id(&ex(s, "instance.create", json!({"component": comp, "x": 0, "y": 400})));
    let sec = id(&ex(s, "node.create", json!({"type": "section", "x": 900, "y": 0, "width": 300, "height": 300})));
    ex(s, "select.none", json!({}));
    Scene { f, a, b, c, d, g, e, al, x, y, inst, sec }
}

#[test]
fn dragging_in_the_layers_panel_reorders_by_the_documented_rules() {
    // The panel drops layers with `node.reparent {ids, parent, index}`: `index` is where the first
    // moved layer ends up among the parent's children, bottom first, counted after the move.
    // (moved, index in F, F's children after (bottom first), derivation)
    let rows: [(&[&str], usize, [&str; 4], &str); 8] = [
        (&["a"], 3, ["b", "c", "d", "a"], "a layer dragged to the top of its parent's list comes to the front"),
        (&["a"], 9, ["b", "c", "d", "a"], "an index past the end is the front"),
        (&["d"], 0, ["d", "a", "b", "c"], "a layer dragged to the bottom of the list goes to the back"),
        (&["a"], 1, ["b", "a", "c", "d"], "a layer moved up one place goes up one place (#992)"),
        (&["a"], 0, ["a", "b", "c", "d"], "silent: a layer asked to stay where it is stays (pinned)"),
        (&["a", "b"], 2, ["c", "d", "a", "b"], "several selected layers move together, keeping their order"),
        (&["a", "d"], 1, ["b", "a", "d", "c"], "several layers from apart land together where they are asked to"),
        (&["b", "c"], 1, ["a", "b", "c", "d"], "silent: moving a block of layers to where it is changes nothing (pinned)"),
    ];
    for (moved, index, after, why) in rows {
        let mut s = Session::new();
        let k = scene(&mut s);
        let by = |n: &str| match n {
            "a" => k.a,
            "b" => k.b,
            "c" => k.c,
            _ => k.d,
        };
        let ids: Vec<NodeId> = moved.iter().map(|n| by(n)).collect();
        let before: Vec<[f64; 6]> = ids.iter().map(|i| world(&s, *i)).collect();
        ex(&mut s, "node.reparent", json!({"ids": ids, "parent": k.f, "index": index}));
        let names: Vec<String> = kids(&s, k.f).iter().map(|i| name(&s, *i)).collect();
        assert_eq!(names, after, "{why}");
        for (i, w) in ids.iter().zip(before) {
            assert!(same(world(&s, *i), w), "{why}: reordering leaves the layers in place on the canvas");
        }
    }
}

#[test]
fn dragging_into_frames_groups_and_auto_layout() {
    let mut s = Session::new();
    let k = scene(&mut s);
    // Into a group: the layer keeps its place on the canvas (silent, pinned).
    let w = world(&s, k.a);
    ex(&mut s, "node.reparent", json!({"ids": [k.a], "parent": k.g}));
    assert_eq!(kids(&s, k.g).last(), Some(&k.a), "a layer dropped on a group goes in it, on top");
    assert!(same(world(&s, k.a), w), "silent: a layer moved into a group keeps its place on the canvas (pinned)");
    // Out to the page.
    let page = s.page().unwrap();
    ex(&mut s, "node.reparent", json!({"ids": [k.b], "parent": page}));
    assert_eq!(s.doc().unwrap().node(k.b).unwrap().parent, Some(page), "a layer dragged out of a frame goes to the page");
    // Into auto layout: the layer joins the flow where it is dropped.
    ex(&mut s, "node.reparent", json!({"ids": [k.c], "parent": k.al, "index": 0}));
    assert_eq!(kids(&s, k.al).first(), Some(&k.c), "dropped first in an auto layout frame");
    let doc = s.doc().unwrap();
    let lx = |i: NodeId| doc.node(i).unwrap().transform.as_coeffs()[4];
    assert!(lx(k.c) < lx(k.x) && lx(k.x) < lx(k.y), "the auto layout frame places it first in its flow");
    // Refused drops.
    let refused: [(NodeId, NodeId, &str); 3] = [
        (k.f, k.d, "a layer can't go into its own child"),
        (k.d, k.inst, "an instance's layers come from its main: nothing can be dropped into it"),
        (k.sec, k.f, "sections go on the page or in other sections, not in frames"),
    ];
    for (what, into, why) in refused {
        let before = s.doc().unwrap().clone();
        assert!(s.execute("node.reparent", &json!({"ids": [what], "parent": into})).is_err(), "{why}");
        assert_eq!(*s.doc().unwrap(), before, "{why}: nothing changed");
    }
    // Silent (pinned): hidden and locked layers move like any other; a locked frame takes drops.
    ex(&mut s, "object.toggleVisible", json!({"ids": [k.d], "visible": false}));
    ex(&mut s, "object.toggleLock", json!({"ids": [k.f], "locked": true}));
    ex(&mut s, "node.reparent", json!({"ids": [k.b, k.d], "parent": k.f}));
    // `b` was above the frame on the page, so it stays above `d` (stacking order kept).
    assert_eq!(
        kids(&s, k.f).iter().rev().take(2).copied().collect::<Vec<_>>(),
        [k.b, k.d],
        "silent: hidden layers move and locked frames take drops (pinned)"
    );
}

#[test]
fn reparent_index_is_the_final_position_in_auto_layout_and_across_parents() {
    // #992: `index` is the layer's place among the parent's children after the move.
    let mut s = Session::new();
    let row = id(&ex(&mut s, "node.create", json!({"type": "frame", "name": "Row", "props": {"layout": "horizontal"}})));
    let text = |s: &mut Session, t: &str, parent: NodeId| id(&ex(s, "node.create", json!({"type": "text", "name": t, "text": t, "parent": parent})));
    let (first, second, third) = (text(&mut s, "first", row), text(&mut s, "second", row), text(&mut s, "third", row));
    let names = |s: &Session, p: NodeId| kids(s, p).iter().map(|i| name(s, *i)).collect::<Vec<_>>();
    ex(&mut s, "node.reparent", json!({"ids": [first], "parent": row, "index": 1}));
    assert_eq!(names(&s, row), ["second", "first", "third"], "moved down one place");
    ex(&mut s, "node.reparent", json!({"ids": [first], "parent": row, "index": 2}));
    assert_eq!(names(&s, row), ["second", "third", "first"], "moved to the end");
    ex(&mut s, "node.reparent", json!({"ids": [first], "parent": row, "index": 0}));
    assert_eq!(names(&s, row), ["first", "second", "third"], "moved to the start");
    let x = |s: &Session, i: NodeId| s.doc().unwrap().node(i).unwrap().transform.as_coeffs()[4];
    assert!(x(&s, first) < x(&s, second) && x(&s, second) < x(&s, third), "the flow follows the new order");
    // Into another parent, `index` means the same thing.
    let other = id(&ex(&mut s, "node.create", json!({"type": "frame", "name": "Other", "props": {"layout": "horizontal"}})));
    let (p, q) = (text(&mut s, "p", other), text(&mut s, "q", other));
    ex(&mut s, "node.reparent", json!({"ids": [second], "parent": other, "index": 1}));
    assert_eq!(kids(&s, other), [p, second, q], "lands at index 1 of the new parent");
    ex(&mut s, "node.reparent", json!({"ids": [first, third], "parent": other, "index": 3}));
    assert_eq!(kids(&s, other), [p, second, q, first, third], "several layers start at index 3, keeping their order");
    assert!(kids(&s, row).is_empty());
}

#[test]
fn renaming_several_layers_by_the_documented_codes() {
    // Three layers named `names`, bottom to top; (names, params, new names, derivation).
    let rows: Vec<([&str; 3], Value, [&str; 3], &str)> = vec![
        (["A", "B", "C"], json!({"name": "Icon"}), ["Icon", "Icon", "Icon"], "a plain new name goes to every layer"),
        (["A", "B", "C"], json!({"name": "Icon_$n"}), ["Icon_1", "Icon_2", "Icon_3"], "the up number counts up from layer to layer"),
        (["A", "B", "C"], json!({"name": "Icon_$N"}), ["Icon_3", "Icon_2", "Icon_1"], "the down number counts down"),
        (["A", "B", "C"], json!({"name": "Icon_$nnn"}), ["Icon_001", "Icon_002", "Icon_003"], "repeating the n pads the number (three digits)"),
        (["A", "B", "C"], json!({"name": "$NNN"}), ["003", "002", "001"], "padding works counting down too"),
        (["A", "B", "C"], json!({"name": "$n $&"}), ["1 A", "2 B", "3 C"], "the current name code keeps each name; the number can go first"),
        (["A", "B", "C"], json!({"name": "$&-$n"}), ["A-1", "B-2", "C-3"], "or the number goes at the end"),
        (["A", "B", "C"], json!({"name": "$n", "start": 5}), ["5", "6", "7"], "up numbers start from the value given"),
        (["A", "B", "C"], json!({"name": "$N", "start": 0}), ["2", "1", "0"], "down numbers stop at the value given"),
        (
            ["Old A", "Old B", "Old C"],
            json!({"find": "Old", "replace": "New"}),
            ["New A", "New B", "New C"],
            "a match replaces only the matched part",
        ),
        (["A", "B", "C"], json!({"find": "zz", "replace": "y"}), ["A", "B", "C"], "layers without the match keep their names"),
        (
            ["a_a", "b", "c"],
            json!({"find": "a", "replace": "<$&>"}),
            ["<a>_<a>", "b", "c"],
            "every occurrence is replaced; the current name code is the matched text",
        ),
        (
            ["Icon_003", "Logo_7", "x"],
            json!({"find": "([a-zA-Z]+)_(\\d+)", "regex": true, "replace": "$2_$1"}),
            ["003_Icon", "7_Logo", "x"],
            "a regular expression's groups swap parts of the name",
        ),
        (
            ["ab", "b", "c"],
            json!({"find": "b", "regex": true, "replace": "[$`|$']"}),
            ["a[a|]", "[|]", "c"],
            "the before and after codes are the text around the match",
        ),
        (["A", "B", "C"], json!({"find": "B", "replace": "$n"}), ["A", "2", "C"], "numbers count every layer, matched or not"),
        (["A", "B", "C"], json!({"name": "$$$n"}), ["$1", "$2", "$3"], "two dollar signs make one"),
        (
            ["A", "B", "C"],
            json!({"name": "$x $1"}),
            ["$x $1", "$x $1", "$x $1"],
            "unknown codes stay as typed (a group without a regular expression too)",
        ),
    ];
    for (names, mut params, after, why) in rows {
        let mut s = Session::new();
        let ids: Vec<NodeId> = names.iter().enumerate().map(|(k, n)| rect(&mut s, n, k as f64 * 50.0, 0.0, None)).collect();
        // Given top first, as a range picked in the panel: the numbering follows the stacking
        // order, bottom first (silent, pinned).
        params["ids"] = json!(ids.iter().rev().collect::<Vec<_>>());
        ex(&mut s, "object.batchRename", params);
        let got: Vec<String> = ids.iter().map(|i| name(&s, *i)).collect();
        assert_eq!(got, after, "{why}");
    }
    // One undo step; a bad regular expression changes nothing.
    let mut s = Session::new();
    let a = rect(&mut s, "A", 0.0, 0.0, None);
    let steps = s.undo_depth().0;
    ex(&mut s, "object.batchRename", json!({"ids": [a], "name": "$n"}));
    assert_eq!(s.undo_depth().0, steps + 1, "a batch rename is one undo step");
    assert!(
        s.execute("object.batchRename", &json!({"ids": [a], "find": "(", "regex": true, "replace": "x"})).is_err(),
        "an invalid pattern is refused"
    );
    assert_eq!(name(&s, a), "1");
    // Odd input stays bounded: a lone dollar sign, a very wide pad, a huge start, other scripts.
    let r = |find: Option<&str>, to: &str, start: Option<f64>, name: &str| {
        crate::rename::BatchRename::new(find, false, to, find.is_none().then_some(to), start).unwrap().apply(name, 0, 1)
    };
    assert_eq!(r(None, "x$", None, "a"), "x$");
    assert_eq!(r(None, &format!("${}", "n".repeat(40)), None, "a"), format!("{:012}", 1));
    assert_eq!(r(None, "$n", Some(1e30), "a"), "1000000000000");
    assert_eq!(r(None, "$n", Some(f64::NAN), "a"), "1");
    assert_eq!(r(Some("é"), "e", None, "café é"), "cafe e");
}

#[test]
fn showing_hiding_and_locking_by_the_documented_rules() {
    let mut s = Session::new();
    let k = scene(&mut s);
    // Hidden layers can still be moved and changed.
    ex(&mut s, "object.toggleVisible", json!({"ids": [k.a]}));
    assert!(!s.doc().unwrap().node(k.a).unwrap().visible, "the eye hides a layer");
    ex(&mut s, "node.set", json!({"id": k.a, "props": {"opacity": 0.5}}));
    ex(&mut s, "transform.move", json!({"ids": [k.a], "dx": 5, "dy": 0}));
    let n = s.doc().unwrap().node(k.a).unwrap().clone();
    assert!((n.opacity - 0.5).abs() < 1e-6 && (n.transform.as_coeffs()[4] - 15.0).abs() < 1e-9, "a hidden layer can still be moved and changed");
    // Mixed visibility (silent, pinned): one toggle shows them all.
    ex(&mut s, "object.toggleVisible", json!({"ids": [k.a, k.b]}));
    let doc = s.doc().unwrap();
    assert!(
        doc.node(k.a).unwrap().visible && doc.node(k.b).unwrap().visible,
        "silent: toggling hidden and shown layers together shows them all (pinned)"
    );
    // Locking a frame locks its layers on the canvas; a child can't be unlocked on its own.
    ex(&mut s, "object.toggleLock", json!({"ids": [k.f], "locked": true}));
    click(&mut s, 180.0, 30.0);
    assert!(s.selection().is_empty(), "a locked frame's layers can't be picked on the canvas");
    ex(&mut s, "object.toggleLock", json!({"ids": [k.d], "locked": false}));
    click(&mut s, 180.0, 30.0);
    assert!(s.selection().is_empty(), "a layer stays locked while its parent is");
    ex(&mut s, "object.toggleLock", json!({"ids": [k.f], "locked": false}));
    click(&mut s, 180.0, 30.0);
    assert_eq!(s.selection(), [k.d], "unlocking the parent frees its layers");
    // Silent (pinned): locking deselects; the panel can still select a locked layer.
    ex(&mut s, "object.toggleLock", json!({"ids": [k.d]}));
    assert!(s.selection().is_empty(), "silent: locking a selected layer deselects it (pinned)");
    ex(&mut s, "select.set", json!({"ids": [k.d]}));
    assert_eq!(s.selection(), [k.d], "silent: a locked layer can still be selected from the panel (pinned)");
}

fn pages(s: &mut Session) -> Vec<Value> {
    ex(s, "page.list", json!({})).as_array().unwrap().clone()
}

#[test]
fn pages_by_the_documented_rules() {
    let mut s = Session::new();
    let p1 = s.page().unwrap();
    rect(&mut s, "r", 30.0, 40.0, None);
    let p2 = id(&ex(&mut s, "page.add", json!({})));
    assert_eq!(s.page().unwrap(), p2, "silent: a new page opens (pinned)");
    assert_eq!(name(&s, p2), "Page 2", "silent: new pages are numbered (pinned)");
    ex(&mut s, "page.rename", json!({"id": p2, "name": "Flows"}));
    assert_eq!(name(&s, p2), "Flows", "a page is renamed in place");
    ex(&mut s, "page.move", json!({"id": p2, "index": 0}));
    assert_eq!(pages(&mut s)[0]["id"], json!(p2), "dragging a page up the list reorders the pages");
    // Duplicating: the copy is named "Copy of …", next to the source, with copies of its layers.
    let dup = id(&ex(&mut s, "page.duplicate", json!({"id": p1})));
    assert_eq!(name(&s, dup), format!("Copy of {}", name(&s, p1)), "a duplicated page's name starts with Copy of");
    let ids: Vec<Value> = pages(&mut s).iter().map(|p| p["id"].clone()).collect();
    assert_eq!(ids, [json!(p2), json!(p1), json!(dup)], "silent: the copy comes right after its source (pinned)");
    assert_eq!(kids(&s, dup).len(), 1, "the copy has the page's layers");
    // Moving layers to another page keeps their coordinates.
    let r = kids(&s, p1)[0];
    let w = world(&s, r);
    ex(&mut s, "object.moveToPage", json!({"ids": [r], "page": p2}));
    assert_eq!(s.doc().unwrap().page_of(r), Some(p2), "Move to page moves the layers");
    assert!(same(world(&s, r), w), "layers moved to a page land at the same coordinates");
    // Dividers: a blank page named with a leading hyphen.
    ex(&mut s, "page.select", json!({"id": p2}));
    let div = id(&ex(&mut s, "page.add", json!({"name": "-"})));
    let list = pages(&mut s);
    let row = list.iter().find(|p| p["id"] == json!(div)).unwrap();
    assert_eq!(row["divider"], json!(true), "a blank page named with a hyphen first is a divider");
    assert_eq!(s.page().unwrap(), p2, "adding a divider leaves the canvas where it was");
    assert!(s.execute("page.select", &json!({"id": div})).is_err(), "a divider isn't a page to open");
    assert!(s.execute("object.moveToPage", &json!({"ids": [r], "page": div})).is_err(), "a divider holds no layers");
    ex(&mut s, "page.rename", json!({"id": p2, "name": "-Flows"}));
    assert!(!s.doc().unwrap().is_divider(p2), "a page with content stays a page whatever its name");
    ex(&mut s, "page.rename", json!({"id": p2, "name": "Flows"}));
    // Deleting.
    let mut t = Session::new();
    let only = t.page().unwrap();
    assert!(t.execute("page.delete", &json!({"id": only})).is_err(), "silent: the only page can't be deleted (pinned)");
    let d = id(&ex(&mut t, "page.add", json!({"name": "- - -"})));
    assert!(t.execute("page.delete", &json!({"id": only})).is_err(), "the last page that isn't a divider can't be deleted");
    ex(&mut t, "page.delete", json!({"id": d}));
    ex(&mut s, "page.delete", json!({"id": p2}));
    assert_ne!(s.page().unwrap(), div, "deleting the open page opens a page, never a divider");
}

#[test]
fn page_move_refuses_what_is_not_a_page() {
    let mut s = Session::new();
    let r = rect(&mut s, "r", 30.0, 40.0, None);
    let before = pages(&mut s);
    assert!(s.execute("page.move", &json!({"id": r, "index": 0})).is_err(), "a layer id is not a page");
    assert!(s.execute("page.move", &json!({"id": 999999, "index": 0})).is_err(), "a page that isn't there is not a page");
    assert_eq!(pages(&mut s), before, "the page list is untouched");
    assert!(s.doc().unwrap().validate().is_empty(), "the document still checks out");
}

#[test]
fn page_select_and_doc_switch_refuse_negative_or_fractional_indexes() {
    let mut s = Session::new();
    let p2 = id(&ex(&mut s, "page.add", json!({})));
    for bad_index in [json!(-1), json!(0.5)] {
        assert!(s.execute("page.select", &json!({"index": bad_index})).is_err(), "page.select index {bad_index} must fail");
        assert_eq!(s.page().unwrap(), p2, "page.select index {bad_index} leaves the open page alone");
    }
    ex(&mut s, "file.new", json!({}));
    let active = s.active;
    assert_eq!(active, 1);
    for bad_index in [json!(-1), json!(0.5)] {
        assert!(s.execute("doc.switch", &json!({"index": bad_index})).is_err(), "doc.switch index {bad_index} must fail");
        assert_eq!(s.active, active, "doc.switch index {bad_index} leaves the active tab alone");
    }
}

#[test]
fn assets_panel_lists_files_and_drops_by_the_documented_rules() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    ex(&mut s, "page.rename", json!({"name": "Kit"}));
    let holder = frame(&mut s, "Controls", 0.0, 0.0, 600.0, 200.0);
    let b = id(&ex(&mut s, "frame.create", json!({"x": 10, "y": 10, "width": 80, "height": 30, "name": "Buttons / Primary", "parent": holder})));
    let comp = id(&ex(&mut s, "component.create", json!({"ids": [b]})));
    let loose = frame(&mut s, "Badge", 800.0, 0.0, 40.0, 20.0);
    let badge = id(&ex(&mut s, "component.create", json!({"ids": [loose]})));
    // (asset, folders, tile name, derivation)
    let rows: [(NodeId, &[&str], &str, &str); 2] = [
        (comp, &["Kit", "Controls", "Buttons"], "Primary", "assets file under their page and frame, then a folder per slash in the name"),
        (badge, &["Kit"], "Badge", "an asset on the page files under the page alone"),
    ];
    let listed = ex(&mut s, "component.list", json!({}));
    for (asset, folders, label, why) in rows {
        let want = (folders.iter().map(|f| f.to_string()).collect::<Vec<_>>(), label.to_string());
        assert_eq!(crate::library::asset_folder(s.doc().unwrap(), asset), want, "{why}");
        let row = listed.as_array().unwrap().iter().find(|r| r["id"] == json!(asset)).unwrap();
        assert_eq!((row["folder"].clone(), row["label"].clone()), (json!(folders), json!(label)), "{why} (as listed for agents)");
    }
    assert!(crate::library::local_components(s.doc().unwrap()).contains(&comp), "local components are listed");
    // Hidden from publishing: a leading period (or underscore) in the name, both ways.
    let rows: [(&str, bool, &str); 4] = [
        ("Badge", true, "a component is offered for publishing"),
        (".Badge", false, "a leading period keeps a component out of the library, even one published before"),
        ("_Badge", false, "silent beyond variable collections: a leading underscore does the same (pinned)"),
        ("Badge", true, "removing the prefix brings it back"),
    ];
    let first = crate::library::publish(s.doc().unwrap(), None, &crate::library::default_selection(s.doc().unwrap(), None), None, 1).unwrap();
    for (n, offered, why) in rows {
        ex(&mut s, "object.rename", json!({"ids": [badge], "name": n}));
        let sel = crate::library::default_selection(s.doc().unwrap(), Some(&first.manifest));
        assert_eq!(sel.components.contains(&badge), offered, "{why}");
    }
    // Dropping an asset: into the frame under the pointer, into an auto layout flow by position.
    let target = frame(&mut s, "Target", 0.0, 400.0, 300.0, 200.0);
    let placed = id(&ex(&mut s, "instance.place", json!({"component": comp, "x": 100, "y": 500})));
    assert_eq!(s.doc().unwrap().node(placed).unwrap().parent, Some(target), "an asset dropped on a frame goes into it");
    let row = frame(&mut s, "Row", 400.0, 400.0, 400.0, 60.0);
    rect(&mut s, "one", 0.0, 0.0, Some(row));
    rect(&mut s, "two", 50.0, 0.0, Some(row));
    ex(&mut s, "layout.add", json!({"ids": [row], "direction": "horizontal"}));
    let first_x = s.doc().unwrap().world_transform(kids(&s, row)[0]).as_coeffs()[4];
    let into = id(&ex(&mut s, "instance.place", json!({"component": comp, "x": first_x + 1.0, "y": 420})));
    assert_eq!(kids(&s, row).first(), Some(&into), "an asset dropped in an auto layout frame joins the flow where it is dropped");
    assert_eq!(s.doc().unwrap().page_of(into), Some(page));
}

#[test]
fn duplicating_a_layer_named_past_the_counter_does_not_overflow() {
    let mut s = Session::new();
    ex(&mut s, "pref.renameDuplicates", json!({"on": true}));
    let r = rect(&mut s, "r", 30.0, 40.0, None);
    ex(&mut s, "node.set", json!({"id": r, "props": {"name": "Layer 18446744073709551615"}}));
    let dup = id_of(&ex(&mut s, "edit.duplicate", json!({"ids": [r]}))["ids"][0]).unwrap();
    assert_eq!(name(&s, dup), "Layer 18446744073709551615 2");
    assert!(s.doc().unwrap().validate().is_empty(), "the document still checks out");
}

#[test]
fn page_navigation_skips_dividers() {
    // A trailing divider: Next page stays on the ordinary page.
    let mut s = Session::new();
    let first = s.page().unwrap();
    let div = id(&ex(&mut s, "page.add", json!({"name": "-"})));
    assert!(s.execute("page.select", &json!({"id": div})).is_err(), "a divider isn't a page to open");
    let r = ex(&mut s, "view.nextPage", json!({}));
    assert_eq!(s.page().unwrap(), first, "Next page doesn't open a trailing divider");
    assert_eq!(id_of(&r["page"]), Some(first), "the result names the current page");
    // A middle divider: Previous page crosses it, Next page crosses it back.
    let later = id(&ex(&mut s, "page.add", json!({"name": "Later"})));
    assert_eq!(s.page().unwrap(), later);
    let r = ex(&mut s, "view.prevPage", json!({}));
    assert_eq!(s.page().unwrap(), first, "Previous page crosses a divider");
    assert_eq!(id_of(&r["page"]), Some(first));
    ex(&mut s, "view.nextPage", json!({}));
    assert_eq!(s.page().unwrap(), later, "Next page crosses a divider");
    ex(&mut s, "view.nextPage", json!({}));
    assert_eq!(s.page().unwrap(), later, "Next page stops on the last page");
}

#[test]
fn opacity_percent_strings_always_divide_by_100() {
    let mut s = Session::new();
    let r = rect(&mut s, "R", 0.0, 0.0, None);
    for (v, want) in [(json!("1%"), 0.01), (json!("0.5%"), 0.005), (json!("50%"), 0.5), (json!(1), 1.0), (json!(50), 0.5), (json!(0.25), 0.25)] {
        ex(&mut s, "node.set", json!({"id": r, "props": {"opacity": v.clone()}}));
        let o = s.doc().unwrap().node(r).unwrap().opacity;
        assert!((o - want).abs() < 1e-6, "opacity {v} gave {o}, want {want}");
    }
}

#[test]
fn fill_opacity_star_ratio_and_arc_ratio_read_a_percent_suffix() {
    let mut s = Session::new();
    let r = rect(&mut s, "R", 0.0, 0.0, None);
    ex(&mut s, "fill.set", json!({"ids": [r], "color": "#ff0000", "opacity": "1%"}));
    let fill = s.doc().unwrap().node(r).unwrap().fills[0].opacity;
    assert!((fill - 0.01).abs() < 1e-6, "fill opacity \"1%\" gave {fill}");
    let star = id(&ex(&mut s, "shape.star", json!({"x": 0, "y": 0, "width": 100, "height": 100, "props": {"innerRadius": "1%"}})));
    let figcraft_doc::NodeKind::Star { inner_ratio, .. } = s.doc().unwrap().node(star).unwrap().kind else { panic!("star") };
    assert!((inner_ratio - 0.01).abs() < 1e-9, "innerRadius \"1%\" gave {inner_ratio}");
    let arc = id(&ex(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 100, "height": 100, "props": {"arcInner": "1%"}})));
    let figcraft_doc::NodeKind::Ellipse { inner, .. } = s.doc().unwrap().node(arc).unwrap().kind else { panic!("ellipse") };
    assert!((inner - 0.01).abs() < 1e-9, "arcInner \"1%\" gave {inner}");
}
