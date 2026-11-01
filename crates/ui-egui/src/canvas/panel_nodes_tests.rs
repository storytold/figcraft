//! What a large selection's frames keep (#1571): the Design panel looks the selected layers up once
//! per document snapshot and selection, and the canvas keeps which layers its constraint guides and
//! boolean operand outlines go with. Each answers as the document does, after edits, another
//! selection, a deleted layer, and for any other document.

use std::sync::Arc;

use egui::{Color32, Context, Rect, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::{Value, json};

use super::selection_cache_tests::drawn;
use crate::right::common;
use crate::selection_cache::{every, show};
use crate::theme::set_canvas_page;
use crate::{FigApp, NoServices};

fn id(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: i32, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": 10, "width": 40, "height": 40});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(s.execute("shape.rectangle", &p).unwrap())
}

/// The panel's questions (the selection's common opacity, whether it is all rectangles), asked as
/// the panel asks them.
fn answers(doc: &Document, sel: &[NodeId]) -> (Option<u32>, bool) {
    (common(doc, sel, |n| n.opacity.to_bits()), every(doc, sel, |n| matches!(n.kind, NodeKind::Rectangle)))
}

/// The same questions asked of the document directly.
fn direct(doc: &Document, sel: &[NodeId]) -> (Option<u32>, bool) {
    let opacities: Vec<u32> = sel.iter().filter_map(|i| doc.node(*i)).map(|n| n.opacity.to_bits()).collect();
    let common = opacities.first().copied().filter(|o| opacities.iter().all(|x| x == o));
    (common, sel.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Rectangle))))
}

#[test]
fn the_panels_kept_layers_answer_as_the_document_does() {
    let mut s = Session::new();
    let (a, b) = (rect(&mut s, 0, None), rect(&mut s, 80, None));
    s.execute("node.set", &json!({"ids": [a], "props": {"opacity": 0.5}})).unwrap();
    let sel = vec![a, b];
    let doc = s.ds().unwrap().doc.clone();
    show(&doc, &sel);
    assert_eq!(answers(&doc, &sel), direct(&doc, &sel));
    assert_eq!(answers(&doc, &sel).0, None, "mixed");
    // An edit is a new snapshot: answered from itself, before it is shown and after.
    s.execute("node.set", &json!({"ids": [a, b], "props": {"opacity": 0.25}})).unwrap();
    let edited = s.ds().unwrap().doc.clone();
    assert!(!Arc::ptr_eq(&doc, &edited));
    assert_eq!(answers(&edited, &sel), (Some(0.25f32.to_bits()), true));
    show(&edited, &sel);
    assert_eq!(answers(&edited, &sel), (Some(0.25f32.to_bits()), true));
    assert_eq!(answers(&doc, &sel), direct(&doc, &sel), "the document shown before still answers for itself");
    // Another selection.
    s.execute("node.set", &json!({"ids": [b], "props": {"opacity": 1.0}})).unwrap();
    let doc = s.ds().unwrap().doc.clone();
    show(&doc, &sel);
    for ids in [vec![a], vec![b], vec![b, a]] {
        assert_eq!(answers(&doc, &ids), direct(&doc, &ids), "{ids:?}");
        show(&doc, &ids);
        assert_eq!(answers(&doc, &ids), direct(&doc, &ids), "{ids:?} shown");
    }
    // A layer that isn't in the document: `every` says no, `common` goes by the others.
    s.execute("edit.delete", &json!({"ids": [b]})).unwrap();
    let doc = s.ds().unwrap().doc.clone();
    show(&doc, &sel);
    assert_eq!(answers(&doc, &sel), (Some(0.25f32.to_bits()), false));
}

#[test]
fn a_kept_selection_never_answers_for_another_document() {
    let mut s = Session::new();
    let a = rect(&mut s, 0, None);
    let sel = vec![a];
    let base = s.doc().unwrap().clone();
    let first = Arc::new(base.clone());
    let kept_at = Arc::as_ptr(&first);
    show(&first, &sel);
    drop(first);
    let mut changed = base;
    changed.node_mut(a).unwrap().opacity = 0.1;
    let second = Arc::new(changed);
    assert_ne!(Arc::as_ptr(&second), kept_at, "the kept document's place stays taken while it is kept");
    assert_eq!(answers(&second, &sel), (Some(0.1f32.to_bits()), true));
}

fn app_of(s: Session) -> (FigApp, Context) {
    let mut app = FigApp::new(s, Box::new(NoServices));
    app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let ctx = Context::default();
    set_canvas_page(&ctx, Color32::WHITE);
    (app, ctx)
}

/// What one frame draws, and what it draws with nothing kept.
fn drawn_now(app: &FigApp, ctx: &Context) -> (usize, usize) {
    let fresh = Context::default();
    set_canvas_page(&fresh, Color32::WHITE);
    (drawn(app, ctx), drawn(app, &fresh))
}

#[test]
fn constraint_guides_follow_the_layers_frame() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 300, "height": 300})).unwrap());
    let r = rect(&mut s, 40, Some(f));
    let page = s.page().unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let (mut app, ctx) = app_of(s);
    let (pinned, fresh) = drawn_now(&app, &ctx);
    assert_eq!(pinned, fresh);
    for (parent, what) in [(page, "on the page: no frame to pin it to"), (f, "back in its frame")] {
        app.session.execute("node.reparent", &json!({"ids": [r], "parent": parent})).unwrap();
        app.session.execute("select.set", &json!({"ids": [r]})).unwrap();
        let (now, fresh) = drawn_now(&app, &ctx);
        assert_eq!(now, fresh, "{what}");
        assert_eq!(now < pinned, parent == page, "{what}");
    }
    // In an auto layout flow its constraints don't apply; ignoring the layout, they do.
    app.session.execute("layout.add", &json!({"ids": [f]})).unwrap();
    let (placed, fresh) = drawn_now(&app, &ctx);
    assert_eq!(placed, fresh, "auto layout places the layer");
    app.set_props(&[r], json!({"absolute": true}));
    let (absolute, fresh) = drawn_now(&app, &ctx);
    assert_eq!(absolute, fresh, "ignoring the auto layout");
}

#[test]
fn operand_outlines_follow_the_boolean_group() {
    let mut s = Session::new();
    let ops = [rect(&mut s, 0, None), rect(&mut s, 20, None), rect(&mut s, 40, None)];
    s.execute("bool.union", &json!({"ids": ops})).unwrap();
    s.execute("select.set", &json!({"ids": [ops[0]]})).unwrap();
    let (mut app, ctx) = app_of(s);
    let (three, fresh) = drawn_now(&app, &ctx);
    assert_eq!(three, fresh);
    app.session.execute("edit.delete", &json!({"ids": [ops[2]]})).unwrap();
    app.session.execute("select.set", &json!({"ids": [ops[0]]})).unwrap();
    let (two, fresh) = drawn_now(&app, &ctx);
    assert_eq!(two, fresh, "one operand fewer to outline");
    assert!(two < three);
}
