use serde_json::json;

use super::*;
use figcraft_doc::NodeKind;

fn sess() -> Session {
    Session::new()
}

fn id(v: &Value) -> NodeId {
    params::id_of(&v["id"]).unwrap()
}

#[test]
fn create_select_undo_redo() {
    let mut s = sess();
    let r = s.execute("node.create", &json!({"type": "rectangle", "x": 10, "y": 20, "width": 30, "height": 40})).unwrap();
    let rid = id(&r);
    assert_eq!(s.selection(), vec![rid]);
    assert_eq!(s.doc().unwrap().world_bounds(rid).unwrap().x0, 10.0);
    assert!(s.undo().unwrap());
    assert!(!s.doc().unwrap().contains(rid));
    assert!(s.redo().unwrap());
    assert!(s.doc().unwrap().contains(rid));
}

#[test]
fn repeat_preserves_command_and_undo_steps() {
    let mut s = sess();
    let rid = id(&s.execute("shape.rectangle", &json!({"x": 10, "y": 20, "width": 10, "height": 10})).unwrap());
    let params = json!({"ids": [rid], "dx": 2, "dy": 3});
    s.execute("transform.nudge", &params).unwrap();
    let repeated = Some(("transform.nudge".to_string(), params));
    for count in 2..=4 {
        s.execute("edit.repeat", &json!({})).unwrap();
        assert_eq!(s.last_command, repeated);
        let bounds = s.doc().unwrap().world_bounds(rid).unwrap();
        assert_eq!((bounds.x0, bounds.y0), (10.0 + 2.0 * count as f64, 20.0 + 3.0 * count as f64));
        assert_eq!(s.undo_depth(), (count + 1, 0));
    }
    for count in (0..4).rev() {
        assert!(s.undo().unwrap());
        let bounds = s.doc().unwrap().world_bounds(rid).unwrap();
        assert_eq!((bounds.x0, bounds.y0), (10.0 + 2.0 * count as f64, 20.0 + 3.0 * count as f64));
    }
    assert_eq!(s.undo_depth(), (1, 4));
    assert!(s.undo().unwrap());
    assert!(!s.doc().unwrap().contains(rid));
}

#[test]
fn repeat_rejects_missing_command_and_self_reference() {
    for last in [None, Some(("edit.repeat".to_string(), json!({})))] {
        let mut s = sess();
        s.last_command = last.clone();
        assert!(matches!(s.execute("edit.repeat", &json!({})), Err(EngineError::Disabled(cmd, _)) if cmd == "edit.repeat"));
        assert_eq!(s.last_command, last);
        assert_eq!(s.undo_depth(), (0, 0));
        assert!(s.journal.is_empty());
    }
}

#[test]
fn node_set_and_errors_roll_back() {
    let mut s = sess();
    let rid = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    s.execute("node.set", &json!({"id": rid, "props": {"fill": "#ff0000", "cornerRadius": 4, "opacity": 0.5, "name": "Red"}})).unwrap();
    let n = s.doc().unwrap().node(rid).unwrap().clone();
    assert_eq!(n.name, "Red");
    assert_eq!(n.corners.radii, [4.0; 4]);
    assert_eq!(n.fills[0].solid_color().unwrap().to_hex(), "FF0000");
    let undo = s.ds().unwrap().history.undo.len();
    assert!(s.execute("node.set", &json!({"id": rid, "props": {"name": "X", "bogus": 1}})).is_err());
    assert_eq!(s.doc().unwrap().node(rid).unwrap().name, "Red");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo);
    assert!(s.execute("no.such", &json!({})).is_err());
}

#[test]
fn failed_batch_restores_all_documents_and_history() {
    let mut s = sess();
    let original = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    s.execute("file.new", &json!({"name": "Second"})).unwrap();
    s.execute("doc.nextTab", &json!({})).unwrap();
    let before_journal = s.journal.clone();
    let before_history: Vec<_> = s.docs.iter().map(|d| d.history.undo.len()).collect();

    let result = s.execute(
        "command.batch",
        &json!({
            "label": "Broken batch",
            "commands": [
                {"command": "doc.nextTab"},
                {"command": "shape.rectangle", "params": {"x": 20, "y": 20, "width": 10, "height": 10}},
                {"command": "node.set", "params": {"id": 999999, "props": {"name": "must fail"}}}
            ]
        }),
    );

    assert!(result.is_err());
    assert_eq!(s.active, 0, "the active document is part of the transaction");
    assert_eq!(s.docs.len(), 2, "a failed batch cannot leave a newly opened tab behind");
    assert_eq!(s.doc().unwrap().node(original).unwrap().name, "Rectangle");
    assert_eq!(s.docs[1].doc.node(s.docs[1].page).map_or(0, |page| page.children.len()), 0);
    assert_eq!(s.docs.iter().map(|d| d.history.undo.len()).collect::<Vec<_>>(), before_history);
    assert_eq!(s.journal, before_journal, "failed inner commands are not journaled");
}

#[test]
fn group_ungroup_and_frame() {
    let mut s = sess();
    let a = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let b = id(&s.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 10, "height": 10})).unwrap());
    let g = id(&s.execute("object.group", &json!({"ids": [a, b]})).unwrap());
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(g).unwrap().kind.type_name(), "GROUP");
    assert_eq!(doc.world_bounds(g).unwrap().width(), 60.0);
    s.execute("object.ungroup", &json!({"ids": [g]})).unwrap();
    assert!(!s.doc().unwrap().contains(g));
    let f = id(&s.execute("object.frameSelection", &json!({"ids": [a, b]})).unwrap());
    assert_eq!(s.doc().unwrap().node(a).unwrap().parent, Some(f));
    assert_eq!(s.doc().unwrap().world_bounds(a).unwrap().x0, 0.0);
}

#[test]
fn auto_layout_add_hugs() {
    let mut s = sess();
    let a = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 20})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 60, "y": 0, "width": 40, "height": 20})).unwrap());
    let r = s.execute("layout.add", &json!({"ids": [a, b]})).unwrap();
    let f = params::id_of(&r["ids"][0]).unwrap();
    let doc = s.doc().unwrap();
    let fnode = doc.node(f).unwrap();
    assert!(fnode.layout.is_some());
    assert_eq!(fnode.width, 100.0);
    // Change gap: frame hugs.
    s.execute("node.set", &json!({"id": f, "props": {"gap": 0}})).unwrap();
    assert_eq!(s.doc().unwrap().node(f).unwrap().width, 80.0);
    // Padding.
    s.execute("node.set", &json!({"id": f, "props": {"padding": 10}})).unwrap();
    assert_eq!(s.doc().unwrap().node(f).unwrap().width, 100.0);
    assert_eq!(s.doc().unwrap().node(f).unwrap().height, 40.0);
}

#[test]
fn text_create_autosize_and_edit() {
    let mut s = sess();
    let t = id(&s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0, "props": {"fontSize": 24}})).unwrap());
    let w0 = s.doc().unwrap().node(t).unwrap().width;
    textedit::begin(&mut s, t, Some(5), false).unwrap();
    textedit::insert(&mut s, " world").unwrap();
    textedit::key(&mut s, "Backspace", false, false, false).unwrap();
    textedit::end(&mut s).unwrap();
    let n = s.doc().unwrap().node(t).unwrap().clone();
    assert_eq!(n.kind.text().unwrap().characters, "Hello worl");
    if figcraft_text::find_face("Inter", "Regular").is_some() {
        assert!(n.width > w0);
    }
    // One undo step for the whole edit session.
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap().node(t).unwrap().kind.text().unwrap().characters, "Hello");
}

#[test]
fn find_replace_replaces_every_occurrence_when_replacement_contains_find() {
    for (text, find, replace, want) in [
        ("cat and cat", "cat", "cats", "cats and cats"),
        ("cat and cat", "cat", "category", "category and category"),
        ("banana", "a", "aa", "baanaanaa"),
    ] {
        let mut s = sess();
        let t = id(&s.execute("text.create", &json!({"text": text, "x": 0, "y": 0})).unwrap());
        s.execute("edit.findReplace", &json!({"find": find, "replace": replace})).unwrap();
        assert_eq!(s.doc().unwrap().node(t).unwrap().kind.text().unwrap().characters, want, "{text}: {find} -> {replace}");
    }
}

#[test]
fn find_replace_rejects_empty_find() {
    let mut s = sess();
    let t = id(&s.execute("text.create", &json!({"text": "cat", "x": 0, "y": 0})).unwrap());
    assert!(s.execute("edit.findReplace", &json!({"find": "", "replace": "x"})).is_err());
    assert_eq!(s.doc().unwrap().node(t).unwrap().kind.text().unwrap().characters, "cat");
}

#[test]
fn components_and_instances_sync() {
    let mut s = sess();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 50, "name": "Button"})).unwrap());
    let label = id(&s.execute("text.create", &json!({"text": "OK", "x": 10, "y": 10, "parent": f})).unwrap());
    let c = id(&s.execute("component.create", &json!({"ids": [f]})).unwrap());
    assert_eq!(c, f);
    let i = id(&s.execute("instance.create", &json!({"component": c, "x": 200, "y": 0})).unwrap());
    let doc = s.doc().unwrap();
    assert_eq!(doc.node(i).unwrap().children.len(), 1);
    let inst_label = doc.node(i).unwrap().children[0];
    // Edit the main: instance follows.
    s.execute("node.set", &json!({"id": c, "props": {"fill": "#00ff00"}})).unwrap();
    assert_eq!(s.doc().unwrap().node(i).unwrap().fills[0].solid_color().unwrap().to_hex(), "00FF00");
    // Override text in the instance, then change main text: override wins.
    s.execute("text.set", &json!({"id": inst_label, "characters": "Cancel"})).unwrap();
    s.execute("text.set", &json!({"id": label, "characters": "Yes"})).unwrap();
    let doc = s.doc().unwrap();
    let kid = doc.node(i).unwrap().children[0];
    assert_eq!(kid, inst_label, "ids are stable across syncs");
    assert_eq!(doc.node(kid).unwrap().kind.text().unwrap().characters, "Cancel");
    // Reset clears overrides.
    s.execute("component.resetInstance", &json!({"ids": [i]})).unwrap();
    let doc = s.doc().unwrap();
    let kid = doc.node(i).unwrap().children[0];
    assert_eq!(doc.node(kid).unwrap().kind.text().unwrap().characters, "Yes");
    // Detach.
    s.execute("component.detach", &json!({"ids": [i]})).unwrap();
    assert_eq!(s.doc().unwrap().node(i).unwrap().kind.type_name(), "FRAME");
    assert!(s.doc().unwrap().validate().is_empty());
}

#[test]
fn variants_switch() {
    let mut s = sess();
    let a = id(&s
        .execute(
            "node.create",
            &json!({"type": "component", "x": 0, "y": 0, "width": 40, "height": 40, "name": "State=Default", "props": {"fill": "#ff0000"}}),
        )
        .unwrap());
    let b = id(&s
        .execute(
            "node.create",
            &json!({"type": "component", "x": 60, "y": 0, "width": 40, "height": 40, "name": "State=Hover", "props": {"fill": "#0000ff"}}),
        )
        .unwrap());
    s.execute("component.combineVariants", &json!({"ids": [a, b]})).unwrap();
    let i = id(&s.execute("instance.create", &json!({"component": a, "x": 0, "y": 200})).unwrap());
    s.execute("instance.setProperties", &json!({"ids": [i], "properties": {"State": "Hover"}})).unwrap();
    let n = s.doc().unwrap().node(i).unwrap().clone();
    assert!(matches!(n.kind, NodeKind::Instance { component, .. } if component == b));
    assert_eq!(n.fills[0].solid_color().unwrap().to_hex(), "0000FF");
}

#[test]
fn styles_and_variables() {
    let mut s = sess();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10, "props": {"fill": "#123456"}})).unwrap());
    let st = s.execute("style.create", &json!({"type": "paint", "name": "Brand/Primary", "from": r})).unwrap();
    let sid = st["id"].as_str().unwrap().to_string();
    let r2 = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap());
    s.execute("style.apply", &json!({"ids": [r2], "style": "Brand/Primary"})).unwrap();
    assert_eq!(s.doc().unwrap().node(r2).unwrap().fills[0].solid_color().unwrap().to_hex(), "123456");
    s.execute("style.update", &json!({"style": sid, "value": ["#ff00ff"]})).unwrap();
    assert_eq!(s.doc().unwrap().node(r2).unwrap().fills[0].solid_color().unwrap().to_hex(), "FF00FF");
    // Variables with modes.
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "bg", "type": "color", "values": {"Light": "#ffffff", "Dark": "#000000"}}))
        .unwrap();
    s.execute("variable.bind", &json!({"ids": [r], "field": "fill", "variable": "bg"})).unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().fills[0].solid_color().unwrap().to_hex(), "FFFFFF");
    let page = s.page().unwrap();
    s.execute("variable.setMode", &json!({"ids": [page], "collection": "Theme", "mode": "Dark"})).unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().fills[0].solid_color().unwrap().to_hex(), "000000");
}

#[test]
fn align_distribute_zorder() {
    let mut s = sess();
    let a = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 30, "y": 20, "width": 10, "height": 10})).unwrap());
    let c = id(&s.execute("shape.rectangle", &json!({"x": 100, "y": 5, "width": 10, "height": 10})).unwrap());
    s.execute("align.top", &json!({"ids": [a, b, c]})).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(b).unwrap().y0, 0.0);
    s.execute("distribute.horizontal", &json!({"ids": [a, b, c]})).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(b).unwrap().x0, 50.0);
    s.execute("arrange.back", &json!({"ids": [c]})).unwrap();
    let page = s.page().unwrap();
    assert_eq!(s.doc().unwrap().node(page).unwrap().children[0], c);
}

#[test]
fn distribute_spans_the_outermost_edge() {
    let mut s = sess();
    let a = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 150, "y": 150, "width": 500, "height": 500})).unwrap());
    let c = id(&s.execute("shape.rectangle", &json!({"x": 160, "y": 160, "width": 10, "height": 10})).unwrap());
    s.execute("distribute.horizontal", &json!({"ids": [a, b, c]})).unwrap();
    let xs: Vec<f64> = [a, b, c].iter().map(|i| s.doc().unwrap().world_bounds(*i).unwrap().x0).collect();
    assert_eq!(xs, [0.0, 75.0, 640.0]);
    s.execute("distribute.vertical", &json!({"ids": [a, b, c]})).unwrap();
    let ys: Vec<f64> = [a, b, c].iter().map(|i| s.doc().unwrap().world_bounds(*i).unwrap().y0).collect();
    assert_eq!(ys, [0.0, 75.0, 640.0]);
}

#[test]
fn a_layer_alone_on_the_page_has_nothing_to_align_to() {
    use crate::cmd::arrange::can_align;
    let mut s = sess();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap());
    let inner = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 30, "width": 10, "height": 10, "parent": f})).unwrap());
    let loose = id(&s.execute("shape.rectangle", &json!({"x": 300, "y": 5, "width": 10, "height": 10})).unwrap());
    let doc = s.doc().unwrap();
    assert!(!can_align(doc, &[]));
    assert!(!can_align(doc, &[loose]), "alone on the page");
    assert!(!can_align(doc, &[f, inner]), "a frame with its own child is the frame alone");
    assert!(can_align(doc, &[inner]), "one in a frame aligns to it");
    assert!(can_align(doc, &[f, loose]), "two align to each other");
    // Where the buttons are off, the command still does nothing (and no harm).
    let before = doc.world_bounds(loose);
    s.execute("align.right", &json!({"ids": [loose]})).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(loose), before);
    s.execute("align.right", &json!({"ids": [inner]})).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(inner).unwrap().x1, 200.0);
}

#[test]
fn send_to_back_across_frames_counts_per_parent() {
    let mut s = sess();
    let make = |s: &mut Session, p: Value| id(&s.execute("node.create", &p).unwrap());
    let fa = make(&mut s, json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 200}));
    let fb = make(&mut s, json!({"type": "frame", "x": 300, "y": 0, "width": 200, "height": 200}));
    let rect = |parent: NodeId| json!({"type": "rectangle", "parent": parent, "x": 10, "y": 10, "width": 20, "height": 20});
    let a: Vec<NodeId> = (0..2).map(|_| make(&mut s, rect(fa))).collect();
    let b: Vec<NodeId> = (0..3).map(|_| make(&mut s, rect(fb))).collect();
    let idx = |s: &Session, n: NodeId| s.doc().unwrap().index_in_parent(n);
    assert_eq!(idx(&s, a[1]), Some(1));
    assert_eq!(idx(&s, b[2]), Some(2));
    s.execute("arrange.back", &json!({"ids": [a[1], b[2]]})).unwrap();
    assert_eq!(idx(&s, a[1]), Some(0), "top of frame A at the back");
    assert_eq!(idx(&s, b[2]), Some(0), "top of frame B at the back");
    // Two from B plus one from A: each parent's selection ends at 0..n in its original order.
    s.execute("arrange.back", &json!({"ids": [b[1], a[0], b[0]]})).unwrap();
    let kids = |s: &Session, f: NodeId| s.doc().unwrap().node(f).unwrap().children.clone();
    assert_eq!(kids(&s, fa), vec![a[0], a[1]]);
    assert_eq!(kids(&s, fb), vec![b[0], b[1], b[2]]);
}

#[test]
fn copy_paste_and_duplicate() {
    let mut s = sess();
    let a = id(&s.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap());
    s.execute("edit.copy", &json!({"ids": [a]})).unwrap();
    let r = s.execute("edit.paste", &json!({})).unwrap();
    let pid = params::id_of(&r["ids"][0]).unwrap();
    assert_ne!(pid, a);
    assert_eq!(s.doc().unwrap().world_bounds(pid).unwrap().x0, 5.0);
    let d = s.execute("edit.duplicate", &json!({"ids": [a]})).unwrap();
    assert_eq!(d["ids"].as_array().unwrap().len(), 1);
}

#[test]
fn boolean_and_flatten() {
    let mut s = sess();
    let a = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap());
    let g = id(&s.execute("bool.union", &json!({"ids": [a, b]})).unwrap());
    assert_eq!(s.doc().unwrap().node(g).unwrap().kind.type_name(), "BOOLEAN_OPERATION");
    let v = id(&s.execute("vector.flatten", &json!({"ids": [g]})).unwrap());
    let n = s.doc().unwrap().node(v).unwrap().clone();
    assert_eq!(n.kind.type_name(), "VECTOR");
    assert_eq!((n.width, n.height), (30.0, 30.0));
}

#[test]
fn pointer_draw_move_resize() {
    let mut s = sess();
    s.set_tool(ToolId::Rectangle);
    let z = 1.0;
    s.pointer(&PointerEvent::at(PointerKind::Down, 10.0, 10.0, z)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 60.0, 40.0, z)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 60.0, 40.0, z)).unwrap();
    let r = s.selection()[0];
    assert_eq!(s.doc().unwrap().world_bounds(r).unwrap(), figcraft_geom::Rect::new(10.0, 10.0, 60.0, 40.0));
    assert_eq!(s.tool.tool, ToolId::Move);
    let undo_len = s.ds().unwrap().history.undo.len();
    // Move it.
    s.pointer(&PointerEvent::at(PointerKind::Down, 30.0, 20.0, z)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 130.0, 120.0, z)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 130.0, 120.0, z)).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(r).unwrap().x0, 110.0);
    assert_eq!(s.ds().unwrap().history.undo.len(), undo_len + 1);
    // Resize from the bottom-right corner.
    s.pointer(&PointerEvent::at(PointerKind::Down, 160.0, 140.0, z)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 210.0, 190.0, z)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 210.0, 190.0, z)).unwrap();
    let b = s.doc().unwrap().world_bounds(r).unwrap();
    assert_eq!((b.width(), b.height()), (100.0, 80.0));
    // Marquee empty space deselects then selects.
    s.pointer(&PointerEvent::at(PointerKind::Down, 500.0, 500.0, z)).unwrap();
    assert!(s.selection().is_empty());
    s.pointer(&PointerEvent::at(PointerKind::Move, 100.0, 100.0, z)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 100.0, 100.0, z)).unwrap();
    assert_eq!(s.selection(), vec![r]);
}

/// No command panics on junk parameters.
#[test]
fn every_command_survives_junk_params() {
    // A junk-parameter sweep: no command may panic (guard turns panics into Internal errors,
    // which would fail this test).
    let junk = [
        json!({}),
        json!(null),
        json!({"ids": ["x", -1, 99999]}),
        json!({"id": "nope", "x": "NaN", "props": 5}),
        json!([1, 2, 3]),
        json!({"variations": {"wght": 1e308, "": "x", "slnt": null}, "props": {"fontVariations": [1], "variationSettings": {"wdth": "-1e9"}}, "text": 7, "cursor": -3}),
        json!({"style": 3, "props": {"listStyle": 5, "indentation": 1e308, "listSpacing": "NaN", "hangingList": "yes", "start": -5, "end": 1e12,
               "textDecorationThickness": {"unit": "pixels", "value": "x"}, "textDecorationOffset": -1e308, "textDecorationColor": [1]}}),
        // Paint variable props: malformed wrappers, unsupported fields and mixed references.
        json!({"id": 2, "type": "rectangle", "props": {"fill": {"$var": null}, "stroke": {"$var": 7}}}),
        json!({"id": 2, "type": "rectangle", "props": {"fill": {"$var": ""}, "stroke": {"$var": []}}}),
        json!({"id": 2, "type": "rectangle", "props": {"fill": {"$var": "x", "extra": true}, "width": {"$var": "x"}}}),
        json!({"id": 2, "type": "rectangle", "props": {"fill": {"$var": {"$ref": "x"}}, "stroke": {"$var": "x", "$ref": "x"}}}),
        // Templates and the Getting started tips as junk.
        json!({"template": {"id": 1}, "name": [1e308], "show": "yes"}),
        // Links (dev resources) and indexes as junk.
        json!({"id": 2, "url": "javascript:alert(1)", "index": 1e308, "name": {"x": 1}}),
        json!({"id": 2, "url": "https://\u{0}x", "index": -1, "name": ""}),
        // Dev Mode measurements: layers, sides, pages and indexes as junk.
        json!({"from": "x", "to": -1, "side": [1], "index": 1e308, "page": {"x": 1}}),
        json!({"from": 2, "to": 2, "side": "middle", "index": -1, "page": 2, "id": "nope"}),
        json!({"from": 2, "to": 1, "side": "auto"}),
        // Widgets: plugins, targets and fuel as junk.
        json!({"id": 1, "plugin": {"x": 1}, "target": [1], "fuel": -1e308, "x": 1e308}),
        // Widget fields and menus: fields, items and values as junk.
        json!({"id": 1, "field": {"x": 1}, "item": [1], "value": {"x": "\u{0}"}, "fuel": 1e308}),
        json!({"id": [1], "field": "label\u{0}", "item": "", "value": "\u{7}".repeat(5000)}),
        // Plugin directories: ids, versions, servers and confirmations as junk.
        json!({"id": "../../x", "version": {"x": 1}, "server": "ws://\u{0}/?key=", "confirm": "yes", "path": [5], "plugin": [1], "query": 7, "sha256": 3}),
        json!({"id": "org.example.x", "version": "1.0.0", "server": "wss://[::1", "confirm": true, "plugin": "nope"}),
        // Prototype expressions and gamepad keys as junk.
        json!({"id": -5, "expression": "x[\u{0}", "expr": {"op": {"op": "add", "args": [1]}}, "trigger": {"type": "keyPress", "key": "Gamepad \u{0}"}}),
        json!({"id": 1, "expr": {"inMode": {"variable": [], "mode": 1e308}}, "trigger": {"type": "keyPress", "key": 7}}),
        // Keyboard steps (view.pan) and toggles as junk.
        json!({"dx": 1e308, "dy": "NaN", "big": "yes", "on": 3}),
        json!({"dx": -1e308, "dy": [1], "big": null, "on": null}),
        // Open hosting: servers, owner tokens, actions, reports and the publish server as junk.
        json!({"server": "wss://\u{0}", "file": [1], "owner": {"x": 1}, "action": "remove", "reason": 5, "link": -1, "access": "admin", "url": 7}),
        json!({"server": "ws://127.0.0.1:1/", "file": "../x", "owner": "", "action": "createLink", "reason": "\u{7}\u{0}", "name": "", "url": "ws://x/?key=k"}),
        // Sharing with a join code: ways, access, codes, servers, ports and interfaces as junk.
        json!({"via": 7, "access": [1], "code": {"x": 1}, "link": -1, "server": 5, "port": -1, "bind": 3, "name": [1]}),
        json!({"via": "carrier pigeon", "access": "admin", "code": "K7M-2QX-9T\u{0}", "server": "wss://\u{0}", "port": 1e308, "bind": "::::"}),
        json!({"via": "local", "access": "view", "code": "\u{1F4A5}".repeat(64), "port": 70000, "bind": "lan"}),
        // Online files: ids, servers, refreshes, actions and names as junk.
        json!({"file": [1], "server": 7, "refresh": "maybe", "action": "share", "name": {"x": 1}}),
        json!({"file": "../x", "server": "wss://\u{0}", "refresh": true, "action": "unshare", "name": 5}),
        // Files as base64 data: a damaged `.sketch` (zip magic, then junk) and an old one (SQLite).
        json!({"data": "UEsDBBQAAAAIAP////8=", "name": "x.sketch"}),
        json!({"data": "U1FMaXRlIGZvcm1hdCAzAA==", "name": "x.sketch", "path": null}),
        // Selection color slots: malformed indexes, ranges and paints.
        json!({"targets": [{"id": 2, "fills": true, "index": -1, "range": [0, 1e308]}], "paint": {"type": "solid", "color": "invalid"}}),
        json!({"targets": [{"id": 2, "fills": "yes", "index": 0, "range": [4, 1]}], "style": 5}),
        json!({"targets": [{"id": 2, "fills": false, "index": 99999999999u64, "range": null}], "variable": ["x"]}),
        // Smart-selection reorder targets and modes as junk.
        json!({"ids": [2, 3], "source": -1, "target": {"id": 3}, "swap": [], "fromGesture": true}),
        json!({"source": 1e308, "target": "NaN", "swap": "yes", "fromGesture": []}),
        // Rotation origins and placing with a tool as junk.
        json!({"tool": [1], "x": 1e308, "y": "NaN", "reset": "maybe", "on": {"x": 1}}),
        json!({"tool": "frame", "x": -1e308, "y": 1e308, "reset": true, "on": "yes"}),
        // Vector paths (`node.set {path}`) as junk. (Not a string `path` at the top: file commands
        // would write to it.)
        json!({"ids": [2, 3], "type": "vector", "path": [5], "width": -1e308, "props": {"path": "M0 0 L1e308 0 L-1e308 1e308", "height": "NaN", "x": 1e308}}),
        json!({"id": 2, "type": "vector", "path": {"d": 1}, "props": {"path": "M NaN 0 Z", "width": 1e-308}}),
        // Compact inspection: fields and outline with malformed types and unknown keys.
        json!({"id": 2, "fields": "name", "outline": true}),
        json!({"id": 2, "fields": ["name", 5]}),
        json!({"id": 2, "fields": ["Name", "noSuchField"]}),
        json!({"id": 2, "fields": null, "outline": []}),
        json!({"id": 2, "outline": "yes"}),
        // Layer search roots, filters and limits as junk.
        json!({"root": {"id": 2}, "name": [1], "type": "nope", "text": true, "component": [], "limit": -1}),
        json!({"root": 999999, "limit": 1e308}),
        // Batch step names and reference paths as junk.
        json!({"commands": [{"command": "select.none", "as": "a"}, {"command": "select.none", "as": [1]}, {"command": "select.none", "as": "a"}]}),
        json!({"commands": [{"command": "select.all", "as": "\u{0}"}, {"command": "node.set", "params": {"ids": [
            {"$ref": "\u{0}", "field": "ids/18446744073709551616/x"}, {"$ref": {"x": 1}}, {"$step": 0, "field": "//"}, {"$ref": "\u{0}", "field": "/"}],
            "props": {"name": {"$ref": "nope", "field": 5}}}}]}),
        // Font downloads: family names as junk and as paths (a family is only ever looked up in
        // the font index, never used as a path).
        json!({"family": "../../\u{0}Lato"}),
        json!({"family": ["Lato"], "style": 1e308}),
    ];
    for spec in all_commands() {
        if matches!(
            spec.id,
            "file.open"
                | "file.save"
                | "file.saveAs"
                | "file.import"
                | "file.export"
                | "file.exportSvg"
                | "file.placeImage"
                | "image.place"
                | "file.revert"
                // Takes no params and walks the font folders: one walk per junk is slow.
                | "text.rescanFonts"
        ) {
            continue;
        }
        for j in &junk {
            // Each junk object as given (names the command doesn't take are refused) and cut down
            // to the names it takes, so the junk values still reach its parsers.
            let mut taken = j.clone();
            if let (Some(names), Some(o)) = (crate::cmd::param_names(spec.id), taken.as_object_mut()) {
                o.retain(|k, _| matches!(k.as_str(), "ids" | "id") || names.contains(&k.as_str()));
            }
            for params in if taken == *j { vec![j] } else { vec![j, &taken] } {
                let mut s = sess();
                let _ = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10}));
                let _ = s.execute("text.create", &json!({"text": "t"}));
                if let Err(EngineError::Internal { cmd, msg }) = s.execute(spec.id, params) {
                    panic!("{cmd} panicked on {params}: {msg}");
                }
            }
        }
    }
}

#[test]
fn parity_catalog_mostly_covered() {
    let (done, total, missing) = catalog::parity(&[]);
    assert!(total > 150);
    assert!(done as f64 / total as f64 > 0.8, "{done}/{total} missing: {missing:?}");
}

#[test]
fn export_frames_to_pdf() {
    let mut s = sess();
    let a = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap());
    let b = id(&s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 120, "height": 80})).unwrap());
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let r = s.execute("file.exportFramesPdf", &json!({})).unwrap();
    assert_eq!(r["pages"], 2);
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(text.matches("/Type/Page").count() - text.matches("/Type/Pages").count(), 2);
    assert!(text.contains("/MediaBox[0 0 200 100]") && text.contains("/MediaBox[0 0 120 80]"));
    // file.export with format pdf: frames get a page each, other nodes share one.
    let r = s.execute("file.export", &json!({"ids": [a, b], "format": "pdf"})).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(text.matches("/Type/Page").count() - text.matches("/Type/Pages").count(), 2);
    // A page without frames is refused, not panicking.
    s.execute("page.add", &json!({})).ok();
    let empty = s.doc().unwrap().pages.last().copied().unwrap();
    assert!(s.execute("file.exportFramesPdf", &json!({"page": empty})).is_err());
    assert!(s.execute("file.exportFramesPdf", &json!({"page": a})).is_err());
}

#[test]
fn vector_edit_mode() {
    let mut s = sess();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    s.execute("vector.edit.begin", &json!({"id": r})).unwrap();
    assert_eq!(s.doc().unwrap().node(r).unwrap().kind.type_name(), "VECTOR");
    // Move one corner out: the box grows.
    s.execute("vector.edit.select", &json!({"vertices": [2]})).unwrap();
    s.execute("vector.edit.moveVertices", &json!({"dx": 50, "dy": 50})).unwrap();
    let b = s.doc().unwrap().world_bounds(r).unwrap();
    assert_eq!((b.width(), b.height()), (150.0, 150.0));
    // Add a point on segment 0 and delete-heal it.
    let v = s.execute("vector.edit.addVertex", &json!({"segment": 0, "t": 0.5})).unwrap();
    assert!(v["vertex"].as_u64().is_some());
    let n0 = match &s.doc().unwrap().node(r).unwrap().kind {
        NodeKind::Vector { network } => network.vertices.len(),
        _ => 0,
    };
    s.execute("vector.deleteHeal", &json!({})).unwrap();
    let n1 = match &s.doc().unwrap().node(r).unwrap().kind {
        NodeKind::Vector { network } => (network.vertices.len(), network.regions.len()),
        _ => (0, 0),
    };
    assert_eq!(n1, (n0 - 1, 1), "healed and still closed");
    // Pointer: drag a vertex in edit mode.
    let before = s.doc().unwrap().world_bounds(r).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 0.0, 0.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, -20.0, 0.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, -20.0, 0.0, 1.0)).unwrap();
    let after = s.doc().unwrap().world_bounds(r).unwrap();
    assert_eq!(after.x0, before.x0 - 20.0);
    s.execute("vector.edit.end", &json!({})).unwrap();
    assert!(s.ds().unwrap().vector_edit.is_none());
}

#[test]
fn arrows_reorder_auto_layout_children_and_cmd_arrows_resize() {
    let mut s = sess();
    let row = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 40})).unwrap());
    let kids: Vec<NodeId> =
        (0..3).map(|i| id(&s.execute("shape.rectangle", &json!({"x": i * 60, "y": 0, "width": 40, "height": 20, "parent": row})).unwrap())).collect();
    s.execute("layout.add", &json!({"ids": [row]})).unwrap();
    let order = |s: &Session| s.doc().unwrap().node(row).unwrap().children.clone();
    assert_eq!(order(&s), kids);
    // Right moves the first child one place later; Down does nothing in a horizontal flow.
    s.execute("transform.nudge", &json!({"ids": [kids[0]], "dx": 1, "dy": 0})).unwrap();
    assert_eq!(order(&s), vec![kids[1], kids[0], kids[2]]);
    s.execute("transform.nudge", &json!({"ids": [kids[0]], "dx": 0, "dy": 1})).unwrap();
    assert_eq!(order(&s), vec![kids[1], kids[0], kids[2]]);
    // A selected block moves together and stops at the end.
    s.execute("transform.nudge", &json!({"ids": [kids[1], kids[0]], "dx": 1, "dy": 0, "big": true})).unwrap();
    assert_eq!(order(&s), vec![kids[2], kids[1], kids[0]]);
    s.execute("transform.nudge", &json!({"ids": [kids[0]], "dx": -1, "dy": 0})).unwrap();
    assert_eq!(order(&s), vec![kids[2], kids[0], kids[1]]);
    s.undo().unwrap();
    assert_eq!(order(&s), vec![kids[2], kids[1], kids[0]]);
    // Free layers still nudge.
    let free = id(&s.execute("shape.rectangle", &json!({"x": 500, "y": 500, "width": 10, "height": 10})).unwrap());
    s.execute("transform.nudge", &json!({"ids": [free], "dx": 1, "dy": 0, "big": true})).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(free).unwrap().x0, 510.0);
    // Cmd+arrows: resize by the nudge amount, top-left fixed, only the changed axis becomes fixed.
    s.execute("transform.nudgeSize", &json!({"ids": [free], "dw": 1, "dh": 0})).unwrap();
    let n = s.doc().unwrap().node(free).unwrap().clone();
    assert_eq!((n.width, n.height), (11.0, 10.0));
    assert_eq!(s.doc().unwrap().world_bounds(free).unwrap().x0, 510.0);
    s.execute("transform.nudgeSize", &json!({"ids": [free], "dw": 0, "dh": -1, "big": true})).unwrap();
    assert_eq!(s.doc().unwrap().node(free).unwrap().height, 0.0, "never negative");
    // A hugging auto layout frame keeps hugging vertically when only its width is nudged.
    s.execute("node.set", &json!({"id": row, "props": {"sizingH": "hug", "sizingV": "hug"}})).unwrap();
    s.execute("transform.nudgeSize", &json!({"ids": [row], "dw": 1, "dh": 0, "big": true})).unwrap();
    let r = s.doc().unwrap().node(row).unwrap().clone();
    assert_eq!(r.child_layout.sizing_v, figcraft_doc::Sizing::Hug);
}

#[test]
fn document_colors_are_counted_per_page() {
    let mut s = sess();
    for (i, c) in ["#FF0000", "#FF0000", "#00FF00"].iter().enumerate() {
        let r = id(&s.execute("shape.rectangle", &json!({"x": i * 20, "y": 0, "width": 10, "height": 10})).unwrap());
        s.execute("node.set", &json!({"id": r, "props": {"fills": [c], "strokes": ["#0000FF"]}})).unwrap();
    }
    let v = s.execute("paint.documentColors", &json!({})).unwrap();
    let list: Vec<(String, u64)> =
        v["colors"].as_array().unwrap().iter().map(|c| (c["color"].as_str().unwrap().to_string(), c["count"].as_u64().unwrap())).collect();
    assert_eq!(list.first(), Some(&("0000FF".to_string(), 3)));
    assert!(list.contains(&("FF0000".to_string(), 2)));
    assert!(list.contains(&("00FF00".to_string(), 1)));
    let v = s.execute("paint.documentColors", &json!({"limit": 1})).unwrap();
    assert_eq!(v["colors"].as_array().unwrap().len(), 1);
    // Another page is separate.
    s.execute("page.add", &json!({})).unwrap();
    let v = s.execute("paint.documentColors", &json!({})).unwrap();
    assert!(v["colors"].as_array().unwrap().is_empty());
}

/// `guide.add`, `guide.move` and `guide.remove` edit the current page's guides by index.
#[test]
fn guides_add_move_remove() {
    let mut s = sess();
    let guides = |s: &Session| s.doc().unwrap().node(s.page().unwrap()).unwrap().guides.clone();
    let r = s.execute("guide.add", &json!({"vertical": true, "offset": 120})).unwrap();
    assert_eq!(r["index"], json!(0));
    s.execute("guide.add", &json!({"vertical": false, "offset": 40.5})).unwrap();
    s.execute("guide.move", &json!({"index": 0, "offset": 150})).unwrap();
    assert_eq!(guides(&s)[0].offset, 150.0);
    s.undo().unwrap();
    assert_eq!(guides(&s)[0].offset, 120.0);
    s.execute("guide.remove", &json!({"index": 1})).unwrap();
    assert_eq!(guides(&s).len(), 1);
    for j in [json!({"index": -1}), json!({"index": 5, "offset": 1}), json!({"index": "x"}), json!({"offset": "NaN"})] {
        assert!(s.execute("guide.remove", &j).is_err() || guides(&s).len() == 1, "{j}");
        let _ = s.execute("guide.move", &j);
    }
    assert_eq!(guides(&s).len(), 1, "junk never removes the first guide");
    s.execute("guide.clear", &json!({})).unwrap();
    assert!(guides(&s).is_empty());
}

#[test]
fn find_replace_matches_and_replaces_case_insensitively() {
    let mut s = sess();
    let t = id(&s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap());
    let u = id(&s.execute("text.create", &json!({"text": "İ HELLO hello", "x": 0, "y": 40})).unwrap());
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 80, "width": 50, "height": 50, "name": "HELLO box"})).unwrap());
    let chars = |s: &Session, i| s.doc().unwrap().node(i).unwrap().kind.text().unwrap().characters.clone();
    let r = s.execute("edit.findReplace", &json!({"find": "hello", "replace": "bye", "layerNames": true})).unwrap();
    assert_eq!(r["matches"], json!([t, u, f]));
    assert_eq!(chars(&s, t), "bye");
    assert_eq!(chars(&s, u), "İ bye bye");
    assert_eq!(s.doc().unwrap().node(f).unwrap().name, "bye box");
}

#[test]
fn find_replace_changing_only_the_case_replaces_every_occurrence() {
    let mut s = sess();
    let t = id(&s.execute("text.create", &json!({"text": "figcraft and FIGCRAFT, figcraft", "x": 0, "y": 0})).unwrap());
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 80, "width": 50, "height": 50, "name": "figcraft / figcraft"})).unwrap());
    s.execute("edit.findReplace", &json!({"find": "figcraft", "replace": "FigCraft", "layerNames": true})).unwrap();
    assert_eq!(s.doc().unwrap().node(t).unwrap().kind.text().unwrap().characters, "FigCraft and FigCraft, FigCraft");
    assert_eq!(s.doc().unwrap().node(f).unwrap().name, "FigCraft / FigCraft");
}

/// Damaged and too-old `.sketch` data sent to the file commands (which the junk sweep skips) fail
/// with an ordinary error, not a caught panic.
#[test]
fn damaged_sketch_data_fails_without_panicking() {
    for data in ["UEsDBBQAAAAIAP////8=", "U1FMaXRlIGZvcm1hdCAzAA=="] {
        for cmd in ["file.open", "file.import"] {
            let mut s = sess();
            let r = s.execute(cmd, &json!({"data": data, "name": "x.sketch"}));
            assert!(matches!(r, Err(ref e) if !matches!(e, EngineError::Internal { .. })), "{cmd}: {r:?}");
        }
    }
}
