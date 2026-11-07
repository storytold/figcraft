//! Slots: frames of a main component whose instances take any content of their own.

use crate::Session;
use crate::components::tests::{copy_of, ex, fill, healthy, idv, kids, node};
use crate::components::*;
use crate::tools::{PointerEvent, PointerKind, ToolId};
use figcraft_doc::*;
use serde_json::{Value, json};

/// A "Card" component (300×200 at the origin) with a "Title" text and a "Content" slot (280×140 at
/// 10,50) holding a default "Placeholder" rectangle. Returns (card, title, content, placeholder).
fn slot_card(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 200, "name": "Card"})));
    let title = idv(&ex(s, "text.create", json!({"text": "Title", "x": 10, "y": 10, "parent": f, "props": {"name": "Title"}})));
    let content = idv(&ex(s, "frame.create", json!({"x": 10, "y": 50, "width": 280, "height": 140, "parent": f, "name": "Content"})));
    let ph = idv(&ex(
        s,
        "shape.rectangle",
        json!({"x": 0, "y": 0, "width": 100, "height": 40, "parent": content, "props": {"name": "Placeholder", "fill": "#cccccc"}}),
    ));
    let c = idv(&ex(s, "component.create", json!({"ids": [f]})));
    ex(s, "component.makeSlot", json!({"id": content}));
    (c, title, content, ph)
}

/// A 40×20 "Chip" component at (0, 400) with a red "Dot". Returns (chip, dot).
fn chip(s: &mut Session) -> (NodeId, NodeId) {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 400, "width": 40, "height": 20, "name": "Chip"})));
    let dot =
        idv(&ex(s, "shape.ellipse", json!({"x": 2, "y": 2, "width": 16, "height": 16, "parent": f, "props": {"name": "Dot", "fill": "#ff0000"}})));
    (idv(&ex(s, "component.create", json!({"ids": [f]}))), dot)
}

fn refused(s: &mut Session, cmd: &str, p: Value) {
    let before = s.doc().unwrap().clone();
    assert!(s.execute(cmd, &p).is_err(), "{cmd} {p} should be refused");
    assert_eq!(*s.doc().unwrap(), before, "{cmd} rolled back");
}

fn props_of(s: &mut Session, id: NodeId) -> Vec<Value> {
    ex(s, "component.properties", json!({"id": id}))["properties"].as_array().cloned().unwrap_or_default()
}

/// A refused slot is reported by `component.makeSlot`, the command that was run, not by the
/// property command it adds its slot property through.
#[test]
fn a_refused_slot_names_make_slot() {
    let mut s = Session::new();
    let (c, ..) = slot_card(&mut s);
    let other = idv(&ex(&mut s, "frame.create", json!({"x": 150, "y": 10, "width": 40, "height": 30, "parent": c, "name": "Other"})));
    let err = s.execute("component.makeSlot", &json!({"id": other, "name": "Content"})).unwrap_err().to_string();
    assert!(err.contains("`component.makeSlot`") && err.contains("a property named `Content` already exists"), "{err}");
}

#[test]
fn slots_are_frames_of_a_main_with_a_slot_property() {
    let mut s = Session::new();
    let (c, title, content, ph) = slot_card(&mut s);
    assert_eq!(slot_name(s.doc().unwrap(), content).as_deref(), Some("Content"));
    let p = props_of(&mut s, c);
    assert_eq!(p[0]["type"], "slot");
    assert_eq!(p[0]["layers"], json!([content.0]));
    assert_eq!(slots_of(s.doc().unwrap(), c), vec![(content, "Content".to_string())]);
    // A second frame named "Content" gets a fresh property name.
    let other = idv(&ex(&mut s, "frame.create", json!({"x": 150, "y": 10, "width": 40, "height": 30, "parent": c, "name": "Content"})));
    let r = ex(&mut s, "component.makeSlot", json!({"id": other}));
    assert_eq!(r["name"], "Content 2");
    ex(&mut s, "component.removeSlot", json!({"id": other}));
    assert!(!is_slot(s.doc().unwrap(), other));
    // Preferred components.
    let (chip, _) = chip(&mut s);
    ex(&mut s, "component.editProperty", json!({"id": c, "name": "Content", "preferred": [chip]}));
    assert_eq!(props_of(&mut s, c)[0]["preferred"], json!([chip.0]));
    // What isn't a slot, or can't become one.
    let loose = idv(&ex(&mut s, "frame.create", json!({"x": 900, "y": 0, "width": 50, "height": 50})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 400, "y": 0})));
    let islot = copy_of(&s, i, content);
    for (cmd, p) in [
        ("component.makeSlot", json!({"id": c})),
        ("component.makeSlot", json!({"id": ph})),
        ("component.makeSlot", json!({"id": loose})),
        ("component.makeSlot", json!({"id": content})),
        ("component.makeSlot", json!({"id": islot})),
        ("component.removeSlot", json!({"id": islot})),
        ("component.removeSlot", json!({"id": title})),
        ("component.editProperty", json!({"id": c, "name": "Content", "preferred": [title]})),
        ("component.editProperty", json!({"id": c, "name": "Content", "default": "x"})),
        ("component.bindProperty", json!({"layer": ph, "property": "Content", "field": "slot"})),
        ("component.bindProperty", json!({"layer": other, "property": "Content"})),
        ("instance.setProperties", json!({"ids": [i], "properties": {"Content": 1}})),
        ("instance.slotInsert", json!({"id": content, "component": chip})),
        ("instance.slotInsert", json!({"id": i, "slot": "Nope", "component": chip})),
        ("instance.resetSlot", json!({"id": i})),
    ] {
        refused(&mut s, cmd, p);
    }
    // Slots hold content, not values: instances store none.
    assert!(matches!(&node(&s, i).kind, NodeKind::Instance { props, .. } if props.is_empty()));
    healthy(&s);
}

/// `instances_fill_slots_with_their_own_content`: resetting a slot, or the whole instance, restores the default content.
fn reset_slot_content(s: &mut Session, [a, aslot, ph, ichip, chip]: [NodeId; 5]) {
    // Reset: back to the main's default content.
    ex(s, "instance.resetSlot", json!({"id": aslot}));
    healthy(s);
    assert!(!slot_edited(s.doc().unwrap(), aslot));
    assert_eq!(kids(s, aslot).len(), 2);
    assert_eq!(fill(s, copy_of(s, a, ph)), "0000FF");
    assert!(!s.doc().unwrap().contains(ichip));
    s.undo().unwrap();
    assert!(s.doc().unwrap().contains(ichip));
    healthy(s);
    // Reset all changes resets slots too; `replace` clears the default content first.
    ex(s, "component.resetInstance", json!({"ids": [a]}));
    assert!(!slot_edited(s.doc().unwrap(), aslot));
    let only = idv(&ex(s, "instance.slotInsert", json!({"id": aslot, "component": chip, "replace": true})));
    assert_eq!(kids(s, aslot), vec![only]);
    ex(s, "component.resetOverride", json!({"id": aslot, "group": "slot"}));
    assert_eq!(kids(s, aslot).len(), 2);
    healthy(s);
}

#[test]
fn instances_fill_slots_with_their_own_content() {
    let mut s = Session::new();
    let (c, title, content, ph) = slot_card(&mut s);
    let (chip, dot) = chip(&mut s);
    let a = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 400, "y": 0})));
    let b = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 800, "y": 0})));
    let (aslot, aph) = (copy_of(&s, a, content), copy_of(&s, a, ph));
    // The default content shows (and follows the main) until the slot is edited.
    assert!(!slot_edited(s.doc().unwrap(), aslot));
    ex(&mut s, "node.set", json!({"id": ph, "props": {"fill": "#00ff00"}}));
    assert_eq!(fill(&s, aph), "00FF00");
    // Filling the slot: one undo step; the default content becomes the instance's own.
    let undo = s.ds().unwrap().history.undo.len();
    let r = ex(&mut s, "instance.slotInsert", json!({"id": a, "slot": "Content", "component": chip}));
    let ichip = idv(&r);
    assert_eq!(s.ds().unwrap().history.undo.len(), undo + 1);
    assert_eq!(kids(&s, aslot), vec![aph, ichip]);
    assert!(slot_edited(s.doc().unwrap(), aslot));
    assert!(node(&s, aph).source.is_none() && node(&s, aph).overrides.is_empty());
    assert!(!inside_instance(s.doc().unwrap(), ichip), "an instance in a slot stands alone");
    assert_eq!(owning_instance(s.doc().unwrap(), aph), None);
    let v = props_of(&mut s, a);
    assert_eq!(v[0]["value"], json!({"layer": aslot.0, "edited": true, "layers": 2}));
    healthy(&s);
    // Main edits: the rest of the instance follows, the slot content stays as it is.
    ex(&mut s, "node.set", json!({"id": ph, "props": {"fill": "#0000ff"}}));
    let extra = idv(&ex(&mut s, "shape.ellipse", json!({"x": 150, "y": 0, "width": 20, "height": 20, "parent": content})));
    ex(&mut s, "text.set", json!({"id": title, "characters": "New title"}));
    ex(&mut s, "node.set", json!({"id": c, "props": {"fill": "#fafafa"}}));
    healthy(&s);
    assert_eq!(kids(&s, aslot), vec![aph, ichip], "content and its ids survive main updates");
    assert_eq!(fill(&s, aph), "00FF00");
    assert_eq!(fill(&s, a), "FAFAFA");
    assert_eq!(node(&s, copy_of(&s, a, title)).kind.text().unwrap().characters, "New title");
    // The untouched instance still shows the main's default content.
    let bslot = copy_of(&s, b, content);
    assert_eq!(kids(&s, bslot).len(), 2);
    assert_eq!(fill(&s, copy_of(&s, b, ph)), "0000FF");
    assert!(s.doc().unwrap().contains(copy_of(&s, b, extra)));
    // An instance in a slot follows its own main.
    ex(&mut s, "node.set", json!({"id": dot, "props": {"fill": "#123456"}}));
    assert_eq!(fill(&s, copy_of(&s, ichip, dot)), "123456");
    // Inside the slot, layers come and go freely; outside it the instance is locked.
    let r2 = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10, "parent": aslot})));
    ex(&mut s, "node.reorder", json!({"id": r2, "index": 0}));
    ex(&mut s, "edit.delete", json!({"ids": [aph]}));
    assert_eq!(kids(&s, aslot), vec![r2, ichip]);
    ex(&mut s, "edit.duplicate", json!({"ids": [ichip]}));
    assert_eq!(kids(&s, aslot).len(), 3);
    refused(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5, "parent": a}));
    refused(&mut s, "node.reparent", json!({"ids": [r2], "parent": a}));
    refused(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5, "parent": ichip}));
    let idot = copy_of(&s, ichip, dot);
    refused(&mut s, "node.reparent", json!({"ids": [idot], "parent": aslot}));
    healthy(&s);
    // Layers of the instance can't move into its slot either.
    let atitle = copy_of(&s, a, title);
    refused(&mut s, "node.reparent", json!({"ids": [atitle], "parent": aslot}));
    reset_slot_content(&mut s, [a, aslot, ph, ichip, chip]);
    healthy(&s);
}

#[test]
fn slot_content_survives_variants_copies_and_detach() {
    let mut s = Session::new();
    let (chip, _) = chip(&mut s);
    // Two variants, each with a "Content" slot (the same property, shared by the set).
    let mut variants = Vec::new();
    let mut slots = Vec::new();
    for (k, state) in ["Default", "Hover"].iter().enumerate() {
        let f =
            idv(&ex(&mut s, "frame.create", json!({"x": k as f64 * 400.0, "y": 0, "width": 300, "height": 200, "name": format!("State={state}")})));
        let content = idv(&ex(&mut s, "frame.create", json!({"x": 10, "y": 50, "width": 280, "height": 140, "parent": f, "name": "Content"})));
        variants.push(idv(&ex(&mut s, "component.create", json!({"ids": [f]}))));
        slots.push(content);
    }
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": variants.clone()})));
    ex(&mut s, "component.addProperty", json!({"id": set, "name": "Body", "type": "slot", "layer": slots[0]}));
    ex(&mut s, "component.bindProperty", json!({"layer": slots[1], "property": "Body"}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 600})));
    let ichip = idv(&ex(&mut s, "instance.slotInsert", json!({"id": i, "slot": "Body", "component": chip})));
    ex(&mut s, "node.set", json!({"id": ichip, "props": {"fill": "#abcdef"}}));
    // Variant switch: the content moves along.
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"State": "Hover"}}));
    healthy(&s);
    let islot = copy_of(&s, i, slots[1]);
    assert_eq!(kids(&s, islot), vec![ichip]);
    assert_eq!(fill(&s, ichip), "ABCDEF");
    // Copy and paste: the copy has its own copy of the content.
    ex(&mut s, "edit.copy", json!({"ids": [i]}));
    s.set_selection(vec![]);
    let pasted = ex(&mut s, "edit.paste", json!({"x": 0, "y": 1000}))["ids"][0].as_u64().map(NodeId).unwrap();
    healthy(&s);
    let pslot = copy_of(&s, pasted, slots[1]);
    let pchip = kids(&s, pslot)[0];
    assert_ne!(pchip, ichip);
    assert!(slot_edited(s.doc().unwrap(), pslot));
    assert_eq!(fill(&s, pchip), "ABCDEF");
    ex(&mut s, "node.set", json!({"id": pchip, "props": {"fill": "#000000"}}));
    assert_eq!(fill(&s, ichip), "ABCDEF");
    // Detach: the content stays, the instance in it stays an instance of its own main.
    ex(&mut s, "component.detach", json!({"ids": [i]}));
    healthy(&s);
    assert_eq!(kids(&s, islot), vec![ichip]);
    assert!(matches!(node(&s, ichip).kind, NodeKind::Instance { component, .. } if component == chip));
    assert!(node(&s, islot).overrides.is_empty() && node(&s, islot).source.is_none());
}

#[test]
fn removing_a_slot_takes_the_instances_content_away() {
    let mut s = Session::new();
    let (c, _title, content, _ph) = slot_card(&mut s);
    let (chip, _) = chip(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 400, "y": 0})));
    let islot = copy_of(&s, i, content);
    let ichip = idv(&ex(&mut s, "instance.slotInsert", json!({"id": islot, "component": chip})));
    ex(&mut s, "component.removeSlot", json!({"id": content}));
    healthy(&s);
    assert!(!is_slot(s.doc().unwrap(), islot));
    assert!(!s.doc().unwrap().contains(ichip));
    assert_eq!(kids(&s, islot).len(), 1, "mirrors the main again");
    assert!(props_of(&mut s, c).is_empty());
    s.undo().unwrap();
    assert!(s.doc().unwrap().contains(ichip));
    healthy(&s);
}

#[test]
fn drops_land_in_slots() {
    let mut s = Session::new();
    let (c, title, content, _ph) = slot_card(&mut s);
    let (chip, _) = chip(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 400, "y": 0})));
    let islot = copy_of(&s, i, content);
    let page = s.page().unwrap();
    let doc = s.doc().unwrap().clone();
    // The slot (at 410..690, 50..190) takes drops; the rest of the instance doesn't.
    assert_eq!(crate::drop::slot(&doc, page, figcraft_geom::Point::new(600.0, 150.0)).parent, islot);
    assert_eq!(crate::drop::slot(&doc, page, figcraft_geom::Point::new(420.0, 20.0)).parent, page);
    let r = ex(&mut s, "instance.place", json!({"component": chip, "x": 600, "y": 150}));
    assert_eq!(r["parent"], json!(islot.0));
    assert!(slot_edited(s.doc().unwrap(), islot));
    healthy(&s);
    // Dragging a layer onto the slot moves it in; dragging a layer of the instance out of it
    // only moves it.
    let rect = idv(&ex(&mut s, "shape.rectangle", json!({"x": 1000, "y": 0, "width": 20, "height": 20})));
    s.set_tool(ToolId::Move);
    let drag = |s: &mut Session, from: (f64, f64), to: (f64, f64), cmd: bool| {
        let mut down = PointerEvent::at(PointerKind::Down, from.0, from.1, 1.0);
        down.cmd = cmd;
        s.pointer(&down).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Move, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, 1.0)).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Move, to.0, to.1, 1.0)).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Up, to.0, to.1, 1.0)).unwrap();
    };
    drag(&mut s, (1010.0, 10.0), (650.0, 170.0), false);
    assert_eq!(node(&s, rect).parent, Some(islot));
    healthy(&s);
    // Into a slot still showing the main's default content: it becomes the instance's own.
    let j = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 400, "y": 300})));
    let jslot = copy_of(&s, j, content);
    let dot = idv(&ex(&mut s, "shape.ellipse", json!({"x": 1000, "y": 100, "width": 20, "height": 20})));
    s.set_selection(vec![]);
    drag(&mut s, (1010.0, 110.0), (650.0, 470.0), false);
    assert_eq!(node(&s, dot).parent, Some(jslot));
    assert!(slot_edited(s.doc().unwrap(), jslot));
    assert_eq!(kids(&s, jslot).len(), 2);
    healthy(&s);
    s.set_selection(vec![islot]);
    drag(&mut s, (450.0, 120.0), (1450.0, 620.0), true);
    assert_eq!(node(&s, islot).parent, Some(i), "instance layers stay in the instance");
    assert_eq!(node(&s, title).parent, Some(c));
    healthy(&s);
}

#[test]
fn drags_into_auto_layout_slots_take_their_place_in_the_flow() {
    let mut s = Session::new();
    let (c, _title, content, ph) = slot_card(&mut s);
    ex(&mut s, "node.set", json!({"id": content, "props": {"layout": "vertical", "gap": 8, "padding": 10, "sizingH": "fixed", "sizingV": "fixed"}}));
    ex(&mut s, "node.set", json!({"id": c, "props": {"layout": "vertical", "padding": 10, "gap": 10}}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 400, "y": 0})));
    let (islot, iph) = (copy_of(&s, i, content), copy_of(&s, i, ph));
    let rect = idv(&ex(&mut s, "shape.rectangle", json!({"x": 1000, "y": 0, "width": 30, "height": 30})));
    s.set_tool(ToolId::Move);
    // Below the placeholder copy, inside the slot.
    let b = s.doc().unwrap().world_bounds(islot).unwrap();
    let to = (b.center().x, b.y1 - 5.0);
    s.pointer(&PointerEvent::at(PointerKind::Down, 1015.0, 15.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, 800.0, 100.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, to.0, to.1, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, to.0, to.1, 1.0)).unwrap();
    assert_eq!(kids(&s, islot), vec![iph, rect]);
    assert!(slot_edited(s.doc().unwrap(), islot));
    healthy(&s);
    // Its own content reorders by dragging within the slot, like any auto layout frame; the
    // instance's own layers don't.
    let first = s.doc().unwrap().world_bounds(kids(&s, islot)[0]).unwrap();
    let r = s.doc().unwrap().world_bounds(rect).unwrap().center();
    s.set_selection(vec![rect]);
    s.pointer(&PointerEvent::at(PointerKind::Down, r.x, r.y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, r.x, first.y0 + 6.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, r.x, first.y0 + 2.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, r.x, first.y0 + 2.0, 1.0)).unwrap();
    assert_eq!(kids(&s, islot)[0], rect);
    let order = kids(&s, i);
    let at = s.doc().unwrap().world_bounds(islot).unwrap().center();
    s.set_selection(vec![islot]);
    let mut down = PointerEvent::at(PointerKind::Down, at.x, at.y, 1.0);
    down.cmd = true;
    s.pointer(&down).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, at.x, at.y - 60.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, at.x, at.y - 120.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, at.x, at.y - 120.0, 1.0)).unwrap();
    assert_eq!(kids(&s, i), order, "the instance's layers keep their order");
    healthy(&s);
}

#[test]
fn slots_of_nested_instances_fill_from_the_outer_main_and_its_instances() {
    let mut s = Session::new();
    let (container, _title, content, ph) = slot_card(&mut s);
    let (chip, _) = chip(&mut s);
    // A "Page" component holding a Card instance whose slot the Page main fills.
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 800, "width": 400, "height": 300, "name": "Page"})));
    let nested = idv(&ex(&mut s, "instance.create", json!({"component": container, "x": 20, "y": 20, "parent": f})));
    let pagec = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let nslot = copy_of(&s, nested, content);
    let mchip = idv(&ex(&mut s, "instance.slotInsert", json!({"id": nslot, "component": chip})));
    assert_eq!(owning_component(s.doc().unwrap(), mchip), Some(pagec));
    // Page instances mirror that content (it's the Page main's) ...
    let i = idv(&ex(&mut s, "instance.create", json!({"component": pagec, "x": 600, "y": 800})));
    healthy(&s);
    let inested = copy_of(&s, i, nested);
    let islot = copy_of(&s, inested, nslot);
    assert!(!slot_edited(s.doc().unwrap(), islot));
    assert_eq!(kids(&s, islot).len(), 2);
    // ... and a main edit of that content reaches them.
    ex(&mut s, "node.set", json!({"id": mchip, "props": {"opacity": 0.5}}));
    assert_eq!(node(&s, copy_of(&s, i, mchip)).opacity, 0.5);
    // The instance fills the nested slot with its own content, kept across both mains' edits.
    let own = idv(&ex(&mut s, "shape.star", json!({"x": 0, "y": 0, "width": 30, "height": 30, "parent": islot})));
    assert!(slot_edited(s.doc().unwrap(), islot));
    ex(&mut s, "node.set", json!({"id": ph, "props": {"fill": "#00ff00"}}));
    ex(&mut s, "node.set", json!({"id": pagec, "props": {"fill": "#eeeeee"}}));
    ex(&mut s, "node.set", json!({"id": container, "props": {"fill": "#dddddd"}}));
    healthy(&s);
    assert_eq!(kids(&s, islot).len(), 3);
    assert_eq!(*kids(&s, islot).last().unwrap(), own);
    assert_eq!(fill(&s, nested), "DDDDDD");
    assert_eq!(kids(&s, nslot).len(), 2, "the Page main keeps its own content in the nested slot");
    assert_eq!(fill(&s, *kids(&s, nslot).first().unwrap()), "CCCCCC", "content no longer follows the Card main");
}

/// Random slot edits, main edits and resets keep every instance mirroring its main.
#[test]
fn random_slot_edits_keep_instances_consistent() {
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<T: Copy>(&mut self, v: &[T]) -> Option<T> {
            if v.is_empty() { None } else { v.get((self.next() % v.len() as u64) as usize).copied() }
        }
    }
    for seed in 1..=std::env::var("FC_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(16u64) {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut s = Session::new();
        let (card, _title, content, _ph) = slot_card(&mut s);
        let (chip, _) = chip(&mut s);
        for k in 0..3 {
            ex(&mut s, "instance.create", json!({"component": card, "x": 400 + k * 350, "y": 0}));
        }
        for step in 0..50 {
            let doc = s.doc().unwrap().clone();
            let slots: Vec<NodeId> = doc.nodes().filter(|n| owning_instance(&doc, n.id).is_some() && is_slot(&doc, n.id)).map(|n| n.id).collect();
            let content_layers: Vec<NodeId> = slots.iter().flat_map(|sl| doc.node(*sl).map(|n| n.children.clone()).unwrap_or_default()).collect();
            let main_layers: Vec<NodeId> = doc.subtree(card).into_iter().filter(|i| *i != card).collect();
            let color = format!("#{:06x}", r.next() % 0xFF_FFFF);
            let (cmd, p) = match r.next() % 10 {
                0 | 1 => ("instance.slotInsert", json!({"id": r.pick(&slots), "component": chip, "index": r.next() % 3})),
                2 => ("shape.rectangle", json!({"x": 5, "y": 5, "width": 8, "height": 8, "parent": r.pick(&slots)})),
                3 => ("edit.delete", json!({"ids": [r.pick(&content_layers)]})),
                4 => ("node.reorder", json!({"id": r.pick(&content_layers), "index": r.next() % 3})),
                5 => ("instance.resetSlot", json!({"id": r.pick(&slots)})),
                6 => (
                    "shape.ellipse",
                    json!({"x": 5, "y": 5, "width": 8, "height": 8, "parent": if r.next().is_multiple_of(2) { content } else { card }}),
                ),
                7 => ("node.set", json!({"id": r.pick(&main_layers), "props": {"fill": color}})),
                8 => ("edit.delete", json!({"ids": [r.pick(&main_layers).filter(|i| *i != content)]})),
                _ => ("node.set", json!({"id": r.pick(&content_layers), "props": {"fill": color, "width": 5 + r.next() % 40}})),
            };
            let res = s.execute(cmd, &p);
            if let Err(crate::EngineError::Internal { cmd, msg }) = &res {
                panic!("seed {seed} step {step}: {cmd} panicked: {msg}");
            }
            let doc = s.doc().unwrap();
            assert!(doc.validate().is_empty(), "seed {seed} step {step} {cmd} {p}: {:?}", doc.validate());
            assert!(mirror_issues(doc).is_empty(), "seed {seed} step {step} {cmd} {p} -> {res:?}: {:?}", mirror_issues(doc));
        }
        while s.undo().unwrap() {
            assert!(s.doc().unwrap().validate().is_empty());
        }
    }
}
