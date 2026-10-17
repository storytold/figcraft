//! Contextual import sources survive rebasing; detached and swapped subtrees use their current mains.

use super::*;

fn add(doc: &mut Document, parent: NodeId, kind: NodeKind, name: &str) -> NodeId {
    doc.create(parent, kind, name, 0.0, 0.0, 20.0, 20.0).unwrap()
}

fn component(doc: &mut Document, parent: NodeId, name: &str) -> NodeId {
    add(doc, parent, NodeKind::Component { description: String::new(), props: BTreeMap::new() }, name)
}

fn mirror(doc: &mut Document, source: NodeId, parent: NodeId) -> NodeId {
    let mut n = doc.node(source).unwrap().clone();
    let children = std::mem::take(&mut n.children);
    n.id = doc.alloc_id();
    n.parent = None;
    n.source = Some(source);
    let id = n.id;
    doc.insert(n, parent, None).unwrap();
    for child in children {
        mirror(doc, child, id);
    }
    id
}

fn instance(doc: &mut Document, main: NodeId, parent: NodeId, name: &str) -> NodeId {
    let mut n = doc.node(main).unwrap().clone();
    let children = std::mem::take(&mut n.children);
    n.id = doc.alloc_id();
    n.kind = NodeKind::Instance { component: main, props: BTreeMap::new() };
    n.name = name.into();
    n.parent = None;
    n.source = None;
    let id = n.id;
    doc.insert(n, parent, None).unwrap();
    for child in children {
        mirror(doc, child, id);
    }
    id
}

struct Scene {
    doc: Document,
    inner: NodeId,
    base: NodeId,
    slot: NodeId,
    context: NodeId,
    root: NodeId,
    nested: NodeId,
    child: NodeId,
}

fn scene() -> Scene {
    let mut doc = Document::new("Source context");
    let page = doc.first_page().unwrap();
    let inner = component(&mut doc, page, "Inner");
    let base = add(&mut doc, inner, NodeKind::Rectangle, "Body");
    doc.node_mut(base).unwrap().transform = Affine::translate((2.0, 0.0));
    let outer = component(&mut doc, page, "Outer");
    let slot = instance(&mut doc, inner, outer, "Nested");
    doc.node_mut(slot).unwrap().source = Some(inner);
    let context = doc.node(slot).unwrap().children[0];
    doc.node_mut(context).unwrap().transform = Affine::translate((3.0, 0.0));
    doc.node_mut(context).unwrap().opacity = 0.3;
    let root = instance(&mut doc, outer, page, "Outer copy");
    let nested = doc.node(root).unwrap().children[0];
    let child = doc.node(nested).unwrap().children[0];
    doc.node_mut(child).unwrap().transform = Affine::translate((2.0, 0.0));
    doc.node_mut(child).unwrap().opacity = 0.2;
    Scene { doc, inner, base, slot, context, root, nested, child }
}

fn both(s: &mut Scene) {
    rebase_instance(&mut s.doc, s.nested);
    rebase_instance(&mut s.doc, s.root);
}

#[test]
fn rebase_default_only_sources_promote_to_the_actual_outer_context() {
    let mut s = scene();
    s.doc.node_mut(s.nested).unwrap().source = None;
    s.doc.node_mut(s.child).unwrap().source = None;
    both(&mut s);
    assert_eq!(s.doc.node(s.nested).unwrap().source, Some(s.slot));
    assert_eq!(s.doc.node(s.child).unwrap().source, Some(s.context));
}

#[test]
fn rebase_rejects_a_context_source_from_another_nested_slot() {
    let mut s = scene();
    let outer = s.doc.node(s.slot).unwrap().parent.unwrap();
    let sibling = instance(&mut s.doc, s.inner, outer, "Other nested slot");
    s.doc.node_mut(sibling).unwrap().source = Some(s.inner);
    let sibling_child = s.doc.node(sibling).unwrap().children[0];
    mirror(&mut s.doc, sibling, s.root);
    s.doc.node_mut(s.child).unwrap().source = Some(sibling_child);
    both(&mut s);
    assert_eq!(s.doc.node(s.nested).unwrap().source, Some(s.slot));
    assert_eq!(s.doc.node(s.child).unwrap().source, Some(s.context));
    assert_eq!(s.doc.node(sibling_child).unwrap().transform.translation().x, 2.0);
    assert_eq!(s.doc.node(s.context).unwrap().transform.translation().x, 3.0);
}

#[test]
fn rebase_does_not_mark_inherited_context_values_as_local_overrides() {
    let mut s = scene();
    s.doc.node_mut(s.child).unwrap().opacity = 0.3;
    both(&mut s);
    assert_eq!(s.doc.node(s.child).unwrap().source, Some(s.context));
    assert!(!s.doc.node(s.child).unwrap().overrides.contains("opacity"));
}

#[test]
fn rebase_preserves_multilevel_context_without_shortening_source_chains() {
    let mut doc = Document::new("Nested source contexts");
    let page = doc.first_page().unwrap();
    let inner = component(&mut doc, page, "Inner");
    let base = add(&mut doc, inner, NodeKind::Rectangle, "Body");
    doc.node_mut(base).unwrap().transform = Affine::translate((2.0, 0.0));
    let wrapper = component(&mut doc, page, "Wrapper");
    let default_slot = instance(&mut doc, inner, wrapper, "Nested");
    doc.node_mut(default_slot).unwrap().source = Some(inner);
    let outer = component(&mut doc, page, "Outer");
    let wrapper_slot = instance(&mut doc, wrapper, outer, "Wrapper slot");
    doc.node_mut(wrapper_slot).unwrap().source = Some(wrapper);
    let source_slot = doc.node(wrapper_slot).unwrap().children[0];
    let context = doc.node(source_slot).unwrap().children[0];
    doc.node_mut(context).unwrap().transform = Affine::translate((3.0, 0.0));
    let root = instance(&mut doc, outer, page, "Outer copy");
    let wrapper_copy = doc.node(root).unwrap().children[0];
    let nested = doc.node(wrapper_copy).unwrap().children[0];
    let child = doc.node(nested).unwrap().children[0];
    doc.node_mut(nested).unwrap().name = "Renamed nested".into();
    doc.node_mut(child).unwrap().transform = Affine::translate((2.0, 0.0));
    for id in [nested, wrapper_copy, root] {
        rebase_instance(&mut doc, id);
        assert_eq!(doc.node(nested).unwrap().source, Some(source_slot));
        assert_eq!(doc.node(child).unwrap().source, Some(context));
    }
    assert_eq!(doc.node(context).unwrap().source, Some(doc.node(default_slot).unwrap().children[0]));
    assert_eq!(doc.node(context).unwrap().transform.translation().x, 3.0);
    assert_eq!(doc.node(child).unwrap().transform.translation().x, 2.0);
}

#[test]
fn rebase_preserves_placed_frame_to_instance_and_current_main_children() {
    let mut doc = Document::new("Typed nested source");
    let page = doc.first_page().unwrap();
    let inner = component(&mut doc, page, "Inner");
    let body = add(&mut doc, inner, NodeKind::Rectangle, "Body");
    let outer = component(&mut doc, page, "Outer");
    let slot = add(&mut doc, outer, NodeKind::Frame, "Empty slot");
    let root = instance(&mut doc, outer, page, "Outer copy");
    let nested = doc.node(root).unwrap().children[0];
    doc.node_mut(nested).unwrap().kind = NodeKind::Instance { component: inner, props: BTreeMap::new() };
    let child = mirror(&mut doc, body, nested);
    rebase_instance(&mut doc, nested);
    rebase_instance(&mut doc, root);
    assert_eq!(doc.node(nested).unwrap().source, Some(slot));
    assert_eq!(doc.node(child).unwrap().source, Some(body));
    assert!(matches!(doc.node(nested).unwrap().kind, NodeKind::Instance { component, .. } if component == inner));
}

#[test]
fn rebase_keeps_current_main_sources_and_component_delta_beneath_swap() {
    let mut s = scene();
    let page = s.doc.first_page().unwrap();
    let replacement = component(&mut s.doc, page, "Replacement");
    let body = add(&mut s.doc, replacement, NodeKind::Rectangle, "Body");
    s.doc.node_mut(body).unwrap().transform = Affine::translate((7.0, 0.0));
    s.doc.node_mut(s.nested).unwrap().kind = NodeKind::Instance { component: replacement, props: BTreeMap::new() };
    s.doc.remove(s.child).unwrap();
    s.child = mirror(&mut s.doc, body, s.nested);
    s.doc.node_mut(s.child).unwrap().transform = Affine::translate((8.0, 0.0));
    s.doc.node_mut(s.nested).unwrap().name = "Renamed swap".into();
    rebase_instance(&mut s.doc, s.nested);
    assert_eq!(s.doc.node(s.nested).unwrap().source, Some(s.slot));
    assert!(s.doc.node(s.nested).unwrap().overrides.contains("component"));
    rebase_instance(&mut s.doc, s.root);
    assert_eq!(s.doc.node(s.nested).unwrap().source, Some(s.slot));
    assert_eq!(s.doc.node(s.child).unwrap().source, Some(body));
    assert!(s.doc.node(s.nested).unwrap().overrides.contains("component"));
    assert_eq!(s.doc.node(s.child).unwrap().transform.translation().x, 8.0);
}

#[test]
fn rebase_detached_nested_uses_own_main_and_keeps_inherited_geometry() {
    let mut s = scene();
    let page = s.doc.first_page().unwrap();
    let detached = mirror(&mut s.doc, s.nested, page);
    let child = s.doc.node(detached).unwrap().children[0];
    // The old source references still exist, but their Outer context is absent here.
    s.doc.node_mut(detached).unwrap().source = Some(s.slot);
    s.doc.node_mut(child).unwrap().source = Some(s.context);
    s.doc.node_mut(child).unwrap().transform = Affine::translate((3.0, 0.0));
    rebase_instance(&mut s.doc, detached);
    assert_eq!(s.doc.node(detached).unwrap().source, Some(s.inner));
    assert_eq!(s.doc.node(child).unwrap().source, Some(s.base));
    assert_eq!(s.doc.node(child).unwrap().transform.translation().x, 3.0);
    assert_eq!(s.doc.node(s.base).unwrap().transform.translation().x, 2.0);
}

#[test]
fn rebase_does_not_preserve_existing_foreign_or_stale_sources() {
    let mut s = scene();
    let page = s.doc.first_page().unwrap();
    let foreign = component(&mut s.doc, page, "Foreign");
    let foreign_body = add(&mut s.doc, foreign, NodeKind::Rectangle, "Body");
    s.doc.node_mut(s.child).unwrap().source = Some(foreign_body);
    both(&mut s);
    assert_ne!(s.doc.node(s.child).unwrap().source, Some(foreign_body));
    assert_eq!(s.doc.node(s.child).unwrap().source, Some(s.context));
    let mut s = scene();
    let page = s.doc.first_page().unwrap();
    let replacement = component(&mut s.doc, page, "Replacement");
    let body = add(&mut s.doc, replacement, NodeKind::Rectangle, "Body");
    s.doc.node_mut(s.nested).unwrap().kind = NodeKind::Instance { component: replacement, props: BTreeMap::new() };
    s.doc.node_mut(s.child).unwrap().source = Some(s.context);
    both(&mut s);
    assert_ne!(s.doc.node(s.child).unwrap().source, Some(s.context), "the old default-main child is stale after a swap");
    assert!(s.doc.node(s.child).unwrap().source.is_none() || s.doc.node(s.child).unwrap().source == Some(body));
}

#[test]
fn rebase_rejects_source_and_parent_cycles_with_bounded_lookups() {
    let mut s = scene();
    s.doc.node_mut(s.base).unwrap().source = Some(s.context);
    both(&mut s);
    assert_eq!(s.doc.node(s.child).unwrap().source, None);
    let mut s = scene();
    s.doc.node_mut(s.context).unwrap().parent = Some(s.context);
    both(&mut s);
    assert_ne!(s.doc.node(s.child).unwrap().source, Some(s.context));
    let mut s = scene();
    s.doc.node_mut(s.nested).unwrap().parent = Some(s.child);
    s.doc.node_mut(s.child).unwrap().parent = Some(s.nested);
    let sources = RebaseSources::new(&s.doc);
    assert!(sources.owners(s.child).is_none());
    let mut s = scene();
    s.doc.node_mut(s.root).unwrap().kind = NodeKind::Instance { component: s.root, props: BTreeMap::new() };
    rebase_instance(&mut s.doc, s.root);
    assert_ne!(s.doc.node(s.root).unwrap().source, Some(s.root));
}
