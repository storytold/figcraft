//! Components and variants behaviour suite: each documented rule of instances, overrides, variants,
//! swaps, detach / reset / push and resizing is a table row with a one-line derivation. Where the
//! documentation is silent the row says so and pins today's behaviour. Nested instance overrides
//! are covered in `nested_override_tests`, slots in `components_tests`, accepting library updates
//! with overrides kept in `library_tests` and `components_scale_tests`.

use serde_json::{Value, json};

use crate::Session;
use crate::components::tests::{chars, copy_of, ex, fill, healthy, idv, kids, node};
use figcraft_doc::*;

/// The card: a 200×100 component with a "Bg" rectangle, a "Title" text and a "Row" auto layout
/// frame (padding 4, gap 2) holding a "Chip" rectangle; and one instance of it.
struct Kit {
    card: NodeId,
    bg: NodeId,
    title: NodeId,
    row: NodeId,
    chip: NodeId,
    inst: NodeId,
}

#[derive(Clone, Copy, Debug)]
enum Layer {
    Root,
    Bg,
    Title,
    Row,
    Chip,
}

impl Kit {
    fn main(&self, l: Layer) -> NodeId {
        match l {
            Layer::Root => self.card,
            Layer::Bg => self.bg,
            Layer::Title => self.title,
            Layer::Row => self.row,
            Layer::Chip => self.chip,
        }
    }

    /// The instance's copy of main layer `l`.
    fn copy(&self, s: &Session, l: Layer) -> NodeId {
        match l {
            Layer::Root => self.inst,
            _ => copy_of(s, self.inst, self.main(l)),
        }
    }
}

fn kit(s: &mut Session) -> Kit {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    let bg =
        idv(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 100, "parent": f, "props": {"name": "Bg", "fill": "#eeeeee"}})));
    let title = idv(&ex(s, "text.create", json!({"text": "Hello", "x": 10, "y": 10, "parent": f, "props": {"name": "Title"}})));
    let row = idv(&ex(s, "frame.create", json!({"x": 10, "y": 50, "width": 120, "height": 30, "parent": f, "name": "Row"})));
    ex(s, "node.set", json!({"id": row, "props": {"layout": "horizontal", "padding": 4, "gap": 2}}));
    let chip = idv(&ex(
        s,
        "shape.rectangle",
        json!({"x": 0, "y": 0, "width": 20, "height": 20, "parent": row, "props": {"name": "Chip", "fill": "#333333"}}),
    ));
    let card = idv(&ex(s, "component.create", json!({"ids": [f]})));
    let inst = idv(&ex(s, "instance.create", json!({"component": card, "x": 300, "y": 0})));
    Kit { card, bg, title, row, chip, inst }
}

fn undo_len(s: &Session) -> usize {
    s.ds().unwrap().history.undo.len()
}

fn component_of(s: &Session, id: NodeId) -> NodeId {
    match node(s, id).kind {
        NodeKind::Instance { component, .. } => component,
        _ => panic!("{id} is not an instance"),
    }
}

fn text(n: &Node) -> &TextData {
    n.kind.text().unwrap()
}

#[test]
fn copying_a_main_component_pastes_a_linked_instance() {
    for command in ["edit.paste", "edit.pasteOver", "edit.pasteReplace"] {
        let mut s = Session::new();
        let k = kit(&mut s);
        ex(&mut s, "select.set", json!({"ids": [k.card]}));
        ex(&mut s, "edit.copy", json!({}));
        if command != "edit.paste" {
            ex(&mut s, "select.set", json!({"ids": [k.inst]}));
        }
        let before = undo_len(&s);
        let pasted = crate::params::id_of(&ex(&mut s, command, json!({}))["ids"][0]).unwrap();
        assert_eq!(component_of(&s, pasted), k.card, "{command}");
        assert_eq!(node(&s, pasted).parent, Some(s.page().unwrap()));
        assert_eq!(node(&s, pasted).source, Some(k.card));
        assert_eq!(undo_len(&s), before + 1);
        ex(&mut s, "edit.undo", json!({}));
        assert!(s.doc().unwrap().node(pasted).is_none());
        ex(&mut s, "edit.redo", json!({}));
        assert_eq!(component_of(&s, pasted), k.card);
        ex(&mut s, "node.set", json!({"id": k.title, "props": {"characters": "Updated"}}));
        assert_eq!(chars(&s, copy_of(&s, pasted, k.title)), "Updated");
        healthy(&s);
    }
}

#[test]
fn copying_a_variant_pastes_an_instance_but_copying_its_set_keeps_the_components() {
    let mut s = Session::new();
    let (set, _, hover) = button_set(&mut s);
    ex(&mut s, "select.set", json!({"ids": [hover]}));
    ex(&mut s, "edit.copy", json!({}));
    let pasted = crate::params::id_of(&ex(&mut s, "edit.paste", json!({}))["ids"][0]).unwrap();
    assert_eq!(component_of(&s, pasted), hover);
    assert_eq!(node(&s, pasted).name, node(&s, set).name);
    let NodeKind::Instance { props, .. } = &node(&s, pasted).kind else { panic!("pasted variant is not an instance") };
    assert_eq!(chars(&s, layer(&s, pasted, "Label").unwrap()), "Hover");
    assert_eq!(props["Show dot"], true);
    assert_eq!(node(&s, pasted).parent, Some(s.page().unwrap()));

    ex(&mut s, "select.set", json!({"ids": [set]}));
    ex(&mut s, "edit.copy", json!({}));
    let copied_set = crate::params::id_of(&ex(&mut s, "edit.paste", json!({}))["ids"][0]).unwrap();
    assert!(matches!(node(&s, copied_set).kind, NodeKind::ComponentSet { .. }));
    assert_eq!(node(&s, copied_set).parent, Some(s.page().unwrap()));
    assert_eq!(kids(&s, copied_set).len(), 2);
    for variant in kids(&s, copied_set) {
        assert!(matches!(node(&s, variant).kind, NodeKind::Component { .. }));
        assert!(!kids(&s, set).contains(&variant));
    }
    healthy(&s);
}

#[test]
fn cut_components_and_components_copied_to_another_file_remain_components() {
    for (copy, undo_cut, new_file) in [("edit.cut", false, false), ("edit.cut", true, false), ("edit.copy", false, true)] {
        let mut s = Session::new();
        let k = kit(&mut s);
        ex(&mut s, copy, json!({"ids": [k.card]}));
        if undo_cut {
            ex(&mut s, "edit.undo", json!({}));
        }
        if new_file {
            ex(&mut s, "file.new", json!({}));
        }
        let page = s.page().unwrap();
        let pasted = crate::params::id_of(&ex(&mut s, "edit.paste", json!({"parent": page}))["ids"][0]).unwrap();
        assert!(matches!(node(&s, pasted).kind, NodeKind::Component { .. }), "{copy}, undo={undo_cut}, new_file={new_file}");
        assert_eq!(kids(&s, pasted).len(), 3);
        healthy(&s);
    }
}

#[test]
fn copying_a_component_does_not_link_to_an_id_reused_after_undo() {
    let mut s = Session::new();
    let frame = idv(&ex(&mut s, "frame.create", json!({"width": 120, "height": 60, "name": "Copied main"})));
    let main = idv(&ex(&mut s, "component.create", json!({"ids": [frame]})));
    ex(&mut s, "edit.copy", json!({"ids": [main]}));
    s.undo().unwrap();
    s.undo().unwrap();
    let other = idv(&ex(&mut s, "frame.create", json!({"width": 300, "height": 200, "name": "Unrelated main"})));
    ex(&mut s, "component.create", json!({"ids": [other]}));
    assert_eq!(other, main, "undo makes the original id available again");
    let page = s.page().unwrap();
    let pasted = crate::params::id_of(&ex(&mut s, "edit.paste", json!({"parent": page}))["ids"][0]).unwrap();
    assert_eq!(node(&s, pasted).name, "Copied main");
    assert_eq!((node(&s, pasted).width, node(&s, pasted).height), (120.0, 60.0));
    assert!(matches!(node(&s, pasted).kind, NodeKind::Component { .. }));
    healthy(&s);
}

#[test]
fn pasted_component_instances_keep_the_copied_transform_in_the_target_frame() {
    let mut s = Session::new();
    let k = kit(&mut s);
    ex(&mut s, "node.set", json!({"id": k.card, "props": {"rotation": 30}}));
    let original = s.doc().unwrap().world_transform(k.card);
    let frame = idv(&ex(&mut s, "frame.create", json!({"x": -100, "y": -100, "width": 1000, "height": 1000})));
    ex(&mut s, "edit.copy", json!({"ids": [k.card]}));
    let pasted = crate::params::id_of(&ex(&mut s, "edit.paste", json!({"parent": frame}))["ids"][0]).unwrap();
    assert_eq!(component_of(&s, pasted), k.card);
    for (actual, expected) in s.doc().unwrap().world_transform(pasted).as_coeffs().into_iter().zip(original.as_coeffs()) {
        assert!((actual - expected).abs() < 1e-9);
    }
    assert_eq!(node(&s, pasted).parent, Some(frame));
    let before = s.doc().unwrap().clone();
    assert!(s.execute("edit.paste", &json!({"parent": k.card})).is_err());
    assert_eq!(s.doc().unwrap(), &before, "pasting into the main is refused atomically");
    healthy(&s);
}

#[test]
fn duplicating_a_main_component_makes_a_linked_instance() {
    for rename in [false, true] {
        let mut s = Session::new();
        ex(&mut s, "pref.renameDuplicates", json!({"on": rename}));
        let k = kit(&mut s);
        let rect = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 300, "width": 50, "height": 50, "props": {"name": "Box"}})));
        let before = s.doc().unwrap().clone();
        let out = ex(&mut s, "edit.duplicate", json!({"ids": [k.card, rect]}));
        let ids: Vec<NodeId> = out["ids"].as_array().unwrap().iter().filter_map(crate::params::id_of).collect();
        assert_eq!(s.selection(), ids);
        let copy = *ids.iter().find(|i| matches!(node(&s, **i).kind, NodeKind::Instance { .. })).expect("the main's copy is an instance");
        assert_eq!(component_of(&s, copy), k.card);
        assert_eq!(node(&s, copy).name, "Card", "instances keep their default name (rename={rename})");
        assert_eq!(node(&s, copy).parent, Some(s.page().unwrap()));
        let (b, c) = (s.doc().unwrap().world_bounds(k.card).unwrap(), s.doc().unwrap().world_bounds(copy).unwrap());
        assert_eq!((c.x0, c.y0, c.width(), c.height()), (b.x1 + 100.0, b.y0, b.width(), b.height()), "a top-level frame's offset");
        let other = *ids.iter().find(|i| **i != copy).unwrap();
        assert!(matches!(node(&s, other).kind, NodeKind::Rectangle), "other layers are still copied");
        ex(&mut s, "node.set", json!({"id": k.title, "props": {"characters": "Updated"}}));
        assert_eq!(chars(&s, copy_of(&s, copy, k.title)), "Updated", "the instance follows its main");
        ex(&mut s, "edit.undo", json!({}));
        ex(&mut s, "edit.undo", json!({}));
        assert_eq!(*s.doc().unwrap(), before, "undo restores exactly");
        healthy(&s);
    }
}

#[test]
fn alt_dragging_a_main_component_copies_an_instance() {
    use crate::tools::{PointerEvent, PointerKind};
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "Tile"})));
    let main = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let ev = |kind, x| PointerEvent { alt: true, ..PointerEvent::at(kind, x, 50.0, 1.0) };
    s.pointer(&ev(PointerKind::Down, 50.0)).unwrap();
    s.pointer(&ev(PointerKind::Move, 150.0)).unwrap();
    s.pointer(&ev(PointerKind::Move, 250.0)).unwrap();
    s.pointer(&ev(PointerKind::Up, 250.0)).unwrap();
    let [copy] = s.selection()[..] else { panic!("one copy selected") };
    assert_eq!(component_of(&s, copy), main);
    assert_eq!(s.doc().unwrap().world_bounds(copy).unwrap().x0, 200.0);
    assert_eq!(s.doc().unwrap().world_bounds(main).unwrap().x0, 0.0, "the main stays");
    healthy(&s);
}

#[test]
fn duplicating_a_variant_adds_a_variant_to_its_set() {
    let mut s = Session::new();
    let (set, _, hover) = button_set(&mut s);
    let copy = crate::params::id_of(&ex(&mut s, "edit.duplicate", json!({"ids": [hover]}))["ids"][0]).unwrap();
    assert!(matches!(node(&s, copy).kind, NodeKind::Component { .. }));
    assert_eq!(node(&s, copy).parent, Some(set));
    assert_eq!(kids(&s, set).len(), 3);
    // A whole set duplicates as a new set of variants.
    let set2 = crate::params::id_of(&ex(&mut s, "edit.duplicate", json!({"ids": [set]}))["ids"][0]).unwrap();
    assert!(matches!(node(&s, set2).kind, NodeKind::ComponentSet { .. }));
    assert!(kids(&s, set2).iter().all(|v| matches!(node(&s, *v).kind, NodeKind::Component { .. })));
    healthy(&s);
}

#[test]
fn overridable_properties_stay_through_main_edits_and_reset() {
    // (what, layer, group, override on the instance, the same property edited on the main, read,
    // derivation)
    type Row = (&'static str, Layer, &'static str, Value, Option<Value>, fn(&Node) -> String, &'static str);
    let grid = serde_json::to_value(LayoutGrid::default()).unwrap();
    let export = serde_json::to_value(ExportSetting::next_after(None)).unwrap();
    let click = json!({"trigger": {"type": "onClick"}, "actions": [{"type": "back"}]});
    let rows: Vec<Row> = vec![
        (
            "fill",
            Layer::Bg,
            "fills",
            json!({"fill": "#00ff00"}),
            Some(json!({"fill": "#123456"})),
            |n| format!("{:?}", n.fills),
            "\"change the fill … of any layers\"",
        ),
        (
            "fill of the instance itself",
            Layer::Root,
            "fills",
            json!({"fill": "#abcdef"}),
            Some(json!({"fill": "#fedcba"})),
            |n| format!("{:?}", n.fills),
            "the instance is a layer too",
        ),
        (
            "stroke",
            Layer::Bg,
            "strokes",
            json!({"stroke": "#0000ff", "strokeWeight": 3}),
            Some(json!({"strokeWeight": 7})),
            |n| format!("{:?} {:?}", n.strokes, n.stroke),
            "\"change the … stroke of any layers\"",
        ),
        (
            "effects",
            Layer::Bg,
            "effects",
            json!({"effects": [{"type": "dropShadow"}]}),
            Some(json!({"effects": [{"type": "dropShadow"}, {"type": "dropShadow"}]})),
            |n| n.effects.len().to_string(),
            "\"add, edit, or remove any shadow or blur effects\"",
        ),
        ("visibility", Layer::Bg, "visible", json!({"visible": false}), None, |n| n.visible.to_string(), "hidden layers stand in for removed ones"),
        ("layer name", Layer::Bg, "name", json!({"name": "Back"}), Some(json!({"name": "Bg2"})), |n| n.name.clone(), "\"change the layer name\""),
        (
            "text content",
            Layer::Title,
            "characters",
            json!({"characters": "Override"}),
            Some(json!({"characters": "Main"})),
            |n| text(n).characters.clone(),
            "text content is overridable",
        ),
        (
            "font size",
            Layer::Title,
            "textStyle",
            json!({"fontSize": 30}),
            Some(json!({"fontSize": 18})),
            |n| text(n).style.font_size.to_string(),
            "\"change text properties, including font, weight, size …\"",
        ),
        (
            "text resizing",
            Layer::Title,
            "textStyle",
            json!({"textAutoResize": "height"}),
            Some(json!({"textAutoResize": "none"})),
            |n| format!("{:?}", text(n).auto_resize),
            "\"… letter spacing, and resizing\"",
        ),
        (
            "paragraph spacing",
            Layer::Title,
            "textStyle",
            json!({"paragraphSpacing": 8}),
            Some(json!({"paragraphSpacing": 2})),
            |n| text(n).paragraph_spacing.to_string(),
            "a text property like the others",
        ),
        (
            "padding",
            Layer::Row,
            "padding",
            json!({"padding": 9}),
            Some(json!({"padding": 1})),
            |n| format!("{:?}", n.layout.as_ref().map(|l| l.padding)),
            "\"adjust vertical and horizontal padding\" on instances",
        ),
        (
            "gap",
            Layer::Row,
            "gap",
            json!({"gap": 12}),
            Some(json!({"gap": 6})),
            |n| format!("{:?}", n.layout.as_ref().map(|l| l.gap)),
            "\"adjust gap between\" on instances",
        ),
        (
            "layout grids",
            Layer::Row,
            "grids",
            json!({"grids": [grid.clone()]}),
            Some(json!({"grids": [grid.clone(), grid.clone()]})),
            |n| n.grids.len().to_string(),
            "\"add, edit, or remove layout guides\"",
        ),
        (
            "layout grids of the instance",
            Layer::Root,
            "grids",
            json!({"grids": [grid.clone()]}),
            Some(json!({"grids": [grid.clone(), grid.clone()]})),
            |n| n.grids.len().to_string(),
            "\"add, edit, or remove layout guides\"",
        ),
        (
            "export settings",
            Layer::Bg,
            "exports",
            json!({"exports": [export.clone()]}),
            Some(json!({"exports": [export.clone(), export.clone()]})),
            |n| n.exports.len().to_string(),
            "\"add, edit, or remove export settings\"",
        ),
        (
            "prototype interactions",
            Layer::Bg,
            "interactions",
            json!({"interactions": [click.clone()]}),
            Some(json!({"interactions": [click.clone(), click.clone()]})),
            |n| n.interactions.len().to_string(),
            "interactions on an instance win over the inherited ones",
        ),
        (
            "opacity",
            Layer::Bg,
            "opacity",
            json!({"opacity": 0.5}),
            Some(json!({"opacity": 0.8})),
            |n| n.opacity.to_string(),
            "documentation silent (pinned)",
        ),
        (
            "corner radius",
            Layer::Bg,
            "corners",
            json!({"cornerRadius": 6}),
            Some(json!({"cornerRadius": 2})),
            |n| format!("{:?}", n.corners.radii),
            "documentation silent (pinned)",
        ),
    ];
    for (what, layer, group, over, main_edit, read, why) in rows {
        let mut s = Session::new();
        let k = kit(&mut s);
        let (main, copy) = (k.main(layer), k.copy(&s, layer));
        let steps = undo_len(&s);
        ex(&mut s, "node.set", json!({"id": copy, "props": over}));
        assert_eq!(undo_len(&s), steps + 1, "{what}: one undo step");
        assert!(node(&s, copy).overrides.contains(group), "{what}: a `{group}` override ({why}); has {:?}", node(&s, copy).overrides);
        let value = read(&node(&s, copy));
        assert_ne!(value, read(&node(&s, main)), "{what}: the override shows");
        if let Some(m) = main_edit {
            ex(&mut s, "node.set", json!({"id": main, "props": m}));
            assert_eq!(read(&node(&s, copy)), value, "{what}: kept when the main changes the same property ({why})");
        }
        // A structure edit of the main rebuilds every instance.
        ex(&mut s, "shape.rectangle", json!({"x": 150, "y": 60, "width": 10, "height": 10, "parent": k.card}));
        healthy(&s);
        assert_eq!(read(&node(&s, copy)), value, "{what}: kept through a rebuild");
        ex(&mut s, "component.resetOverride", json!({"id": copy, "group": group}));
        assert_eq!(read(&node(&s, copy)), read(&node(&s, main)), "{what}: reset shows the main's value");
        assert!(!node(&s, copy).overrides.contains(group), "{what}: reset clears the override");
        healthy(&s);
    }
}

#[test]
fn unrelated_main_edits_still_reach_overridden_layers() {
    // (what, layer, override, main edit of another property, read, expected, derivation)
    type Row = (&'static str, Layer, Value, Value, fn(&Node) -> String, &'static str, &'static str);
    let rows: Vec<Row> = vec![
        (
            "text content override, main font size",
            Layer::Title,
            json!({"characters": "Mine"}),
            json!({"fontSize": 24}),
            |n| format!("{} {}", text(n).characters, text(n).style.font_size),
            "Mine 24",
            "only the overridden property stays; the font follows the main",
        ),
        (
            "fill override, main corner radius",
            Layer::Bg,
            json!({"fill": "#00ff00"}),
            json!({"cornerRadius": 5}),
            |n| format!("{} {:?}", n.fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default(), n.corners.radii),
            "00FF00 [5.0, 5.0, 5.0, 5.0]",
            "changes to the main reach every property not overridden",
        ),
        (
            "padding override, main direction",
            Layer::Row,
            json!({"padding": 9}),
            json!({"layout": "vertical"}),
            |n| format!("{:?}", n.layout.as_ref().map(|l| (l.mode, l.padding))),
            "Some((Vertical, [9.0, 9.0, 9.0, 9.0]))",
            "the direction isn't an instance's to change, so it follows the main",
        ),
    ];
    for (what, layer, over, main_edit, read, expected, why) in rows {
        let mut s = Session::new();
        let k = kit(&mut s);
        let copy = k.copy(&s, layer);
        ex(&mut s, "node.set", json!({"id": copy, "props": over}));
        ex(&mut s, "node.set", json!({"id": k.main(layer), "props": main_edit}));
        healthy(&s);
        assert_eq!(read(&node(&s, copy)), expected, "{what}: {why}");
    }
}

#[test]
fn what_instances_cannot_override_is_refused() {
    // (what, command, params (`$` stands for the instance's copy of the layer), layer, derivation)
    type Row = (&'static str, &'static str, Value, Layer, &'static str);
    let rows: Vec<Row> = vec![
        ("move a layer", "node.set", json!({"id": "$", "props": {"x": 50}}), Layer::Bg, "\"position of any layers within the component\""),
        ("nudge a layer", "transform.nudge", json!({"ids": ["$"], "dx": 1, "dy": 0}), Layer::Bg, "position"),
        ("rotate a layer", "node.set", json!({"id": "$", "props": {"rotation": 15}}), Layer::Bg, "position and rotation come with the layer's place"),
        ("flip a layer", "object.flipH", json!({"ids": ["$"]}), Layer::Bg, "position"),
        (
            "change constraints",
            "node.set",
            json!({"id": "$", "props": {"constraints": {"horizontal": "right"}}}),
            Layer::Bg,
            "\"any constraints applied to the layers\"",
        ),
        (
            "take a layer out of auto layout",
            "node.set",
            json!({"id": "$", "props": {"absolute": true}}),
            Layer::Chip,
            "position, including items in an auto layout frame",
        ),
        (
            "change the auto layout direction",
            "node.set",
            json!({"id": "$", "props": {"layout": "vertical"}}),
            Layer::Row,
            "only padding and gap are adjustable on instances",
        ),
        ("remove auto layout", "layout.remove", json!({"ids": ["$"]}), Layer::Row, "structure of the instance"),
        ("reorder layers", "node.reorder", json!({"id": "$", "index": 3}), Layer::Bg, "\"the order, or z-index, of any layers within the instance\""),
        ("delete a layer", "edit.delete", json!({"ids": ["$"]}), Layer::Bg, "layers can only be hidden"),
        (
            "add a layer",
            "shape.rectangle",
            json!({"x": 0, "y": 0, "width": 5, "height": 5, "parent": "$"}),
            Layer::Root,
            "\"not possible to make changes to the underlying structure\"",
        ),
    ];
    for (what, cmd, params, layer, why) in rows {
        let mut s = Session::new();
        let k = kit(&mut s);
        let target = k.copy(&s, layer);
        let p: Value = serde_json::from_str(&params.to_string().replace("\"$\"", &target.0.to_string())).unwrap();
        let (before, steps) = (s.doc().unwrap().clone(), undo_len(&s));
        assert!(s.execute(cmd, &p).is_err(), "{what} is refused ({why})");
        assert_eq!(*s.doc().unwrap(), before, "{what}: nothing changed");
        assert_eq!(undo_len(&s), steps, "{what}: no undo step");
    }
    // The same edits on the main, and resizing the instance itself, are fine.
    let mut s = Session::new();
    let k = kit(&mut s);
    ex(&mut s, "node.set", json!({"id": k.bg, "props": {"x": 5, "width": 150, "constraints": {"horizontal": "leftRight"}}}));
    ex(&mut s, "node.set", json!({"id": k.inst, "props": {"width": 260}}));
    let ibg = k.copy(&s, Layer::Bg);
    assert_eq!((node(&s, ibg).transform.as_coeffs()[4], node(&s, ibg).width), (5.0, 210.0), "the main's constraints apply when the instance resizes");
    assert!(node(&s, ibg).overrides.is_empty(), "layout isn't an override");
    // Placing an auto layout item is the layout's job: the x is ignored, not refused.
    let ichip = k.copy(&s, Layer::Chip);
    let x0 = node(&s, ichip).transform.as_coeffs()[4];
    ex(&mut s, "node.set", json!({"id": ichip, "props": {"x": 60}}));
    assert_eq!(node(&s, ichip).transform.as_coeffs()[4], x0, "auto layout puts the item back");
    assert!(!node(&s, ichip).overrides.contains("position"));
    // The instance's own export settings are its own like its place (documentation silent; pinned).
    let export = serde_json::to_value(ExportSetting::next_after(None)).unwrap();
    ex(&mut s, "node.set", json!({"id": k.inst, "props": {"exports": [export]}}));
    ex(&mut s, "shape.rectangle", json!({"x": 150, "y": 60, "width": 10, "height": 10, "parent": k.card}));
    assert_eq!((node(&s, k.inst).exports.len(), node(&s, k.inst).overrides.contains("exports")), (1, false));
    healthy(&s);
}

/// A "Button" set: State=Default (Bg white, Label "Default" dark, Badge, Dot) and State=Hover (Bg
/// grey, Label "Hover" blue, no Badge, the same Dot), both with a black stroke on Bg and a
/// "Show dot" boolean bound to Dot. Returns (set, default, hover).
fn button_set(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let mut variants = Vec::new();
    for (k, (state, bg, label)) in [("Default", "#ffffff", "#222222"), ("Hover", "#eeeeee", "#0000ff")].into_iter().enumerate() {
        let f = idv(&ex(s, "frame.create", json!({"x": k as f64 * 200.0, "y": 0, "width": 120, "height": 40, "name": format!("State={state}")})));
        ex(s, "node.set", json!({"id": f, "props": {"fill": "#fafafa", "cornerRadius": 4 + 4 * k}}));
        let props = json!({"name": "Bg", "fill": bg, "stroke": "#000000", "strokeWeight": 1});
        ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 120, "height": 40, "parent": f, "props": props}));
        ex(s, "text.create", json!({"text": state, "x": 10, "y": 10, "parent": f, "props": {"name": "Label", "fill": label}}));
        if k == 0 {
            ex(s, "shape.ellipse", json!({"x": 90, "y": 4, "width": 8, "height": 8, "parent": f, "props": {"name": "Badge", "fill": "#ff0000"}}));
        }
        ex(s, "shape.ellipse", json!({"x": 104, "y": 4, "width": 8, "height": 8, "parent": f, "props": {"name": "Dot", "fill": "#00aa00"}}));
        variants.push(idv(&ex(s, "component.create", json!({"ids": [f]}))));
    }
    let set = idv(&ex(s, "component.combineVariants", json!({"ids": variants.clone()})));
    ex(s, "component.addProperty", json!({"id": set, "name": "Show dot", "type": "boolean", "default": true}));
    for v in &variants {
        let dot = kids(s, *v).into_iter().find(|c| node(s, *c).name == "Dot").unwrap();
        ex(s, "component.bindProperty", json!({"layer": dot, "property": "Show dot"}));
    }
    (set, variants[0], variants[1])
}

/// The direct layer of instance `inst` mirroring a main layer named `name` (renames don't
/// matter); `None` when there is none.
fn layer(s: &Session, inst: NodeId, name: &str) -> Option<NodeId> {
    kids(s, inst).into_iter().find(|c| node(s, *c).source.is_some_and(|src| node(s, src).name == name))
}

#[test]
fn switching_variants_keeps_overrides_where_the_variants_agreed() {
    let mut s = Session::new();
    let (set, default, hover) = button_set(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 200})));
    assert_eq!(component_of(&s, i), default);
    // (what, layer ("" = the instance), override, kept after the switch, derivation)
    type Row = (&'static str, &'static str, Value, bool, &'static str);
    let rows: Vec<Row> = vec![
        (
            "Bg fill (the variants differ)",
            "Bg",
            json!({"fill": "#ff00ff"}),
            false,
            "kept only if the property \"originally matched between variants\"",
        ),
        ("Bg stroke (the variants agree)", "Bg", json!({"strokeWeight": 4}), true, "names match and the stroke matched"),
        ("Label text (the variants differ)", "Label", json!({"characters": "Buy"}), true, "text layers keep their changes when their names match"),
        ("Label colour (the variants differ)", "Label", json!({"fill": "#ff8800"}), true, "\"any changes you've made to text layers\""),
        ("Dot renamed and faded (the variants agree)", "Dot", json!({"name": "My dot", "opacity": 0.5}), true, "renaming doesn't break matching"),
        ("Badge (only in Default)", "Badge", json!({"fill": "#000000"}), false, "no layer of that name in the new variant"),
        ("the instance's fill (the variants agree)", "", json!({"fill": "#123456"}), true, "names match and the fill matched"),
        ("the instance's corners (the variants differ)", "", json!({"cornerRadius": 12}), false, "the corner radius didn't match"),
    ];
    let target = |s: &Session, name: &str| if name.is_empty() { Some(i) } else { layer(s, i, name) };
    let read = |x: &Node, k: &str| match k {
        "fill" => format!("{:?}", x.fills),
        "strokeWeight" => x.stroke.weight.to_string(),
        "characters" => text(x).characters.clone(),
        "name" => x.name.clone(),
        "opacity" => x.opacity.to_string(),
        "cornerRadius" => format!("{:?}", x.corners.radii),
        other => panic!("no reader for `{other}`"),
    };
    // What each override shows before the switch.
    let mut shown: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (what, name, over, _, _) in &rows {
        let id = target(&s, name).unwrap();
        ex(&mut s, "node.set", json!({"id": id, "props": over}));
        assert!(!node(&s, id).overrides.is_empty(), "{what}: an override");
        shown.insert(what.to_string(), over.as_object().unwrap().keys().map(|k| read(&node(&s, id), k)).collect());
    }
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Show dot": false}}));
    let steps = undo_len(&s);
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"State": "Hover"}}));
    assert_eq!(undo_len(&s), steps + 1, "one undo step");
    healthy(&s);
    assert_eq!(component_of(&s, i), hover);
    for (what, name, over, kept, why) in &rows {
        let Some(id) = target(&s, name) else {
            assert!(!kept, "{what}: gone ({why})");
            continue;
        };
        let (n, main) = (node(&s, id), node(&s, if id == i { hover } else { node(&s, id).source.unwrap() }));
        for (k, overridden) in over.as_object().unwrap().keys().zip(&shown[&what.to_string()]) {
            let expected = if *kept { overridden.clone() } else { read(&main, k) };
            assert_eq!(read(&n, k), expected, "{what}: {} ({why})", if *kept { "kept" } else { "shows the new variant" });
        }
    }
    let dot = layer(&s, i, "Dot").unwrap();
    assert!(!node(&s, dot).visible, "property values stay across variants (documentation silent; pinned)");
    // Switching back doesn't bring dropped overrides back.
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"State": "Default"}}));
    assert_eq!(fill(&s, layer(&s, i, "Bg").unwrap()), "FFFFFF");
    assert_eq!(chars(&s, layer(&s, i, "Label").unwrap()), "Buy");
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(component_of(&s, i), default, "undo restores the variant ...");
    assert_eq!(fill(&s, layer(&s, i, "Bg").unwrap()), "FF00FF", "... and its overrides");
    healthy(&s);
}

/// A "Layout" set: Row (horizontal, hugging, a 40 wide "A") and Stacked (vertical, fixed 300 wide,
/// "A" filling it). Returns (set, row, stacked).
fn layout_set(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let mut variants = Vec::new();
    for (k, (name, props, child)) in [
        ("Layout=Row", json!({"layout": "horizontal", "gap": 8, "sizingH": "hug", "sizingV": "hug", "fills": []}), json!({})),
        (
            "Layout=Stacked",
            json!({"layout": "vertical", "gap": 8, "sizingH": "fixed", "width": 300, "sizingV": "hug", "fills": []}),
            json!({"sizingH": "fill"}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let c = idv(&ex(s, "node.create", json!({"type": "component", "name": name, "x": k as f64 * 400.0, "props": props})));
        ex(s, "node.create", json!({"type": "rectangle", "parent": c, "name": "A", "width": 40, "height": 20, "props": child}));
        variants.push(c);
    }
    let set = idv(&ex(s, "component.combineVariants", json!({"ids": variants.clone()})));
    (set, variants[0], variants[1])
}

#[test]
fn switching_variants_takes_the_new_variants_resizing_unless_the_instance_set_its_own() {
    let mut s = Session::new();
    let (set, row, stacked) = layout_set(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 200})));
    assert_eq!(component_of(&s, i), row);
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Layout": "Stacked"}}));
    assert_eq!(component_of(&s, i), stacked);
    let direct = idv(&ex(&mut s, "instance.create", json!({"component": stacked, "x": 0, "y": 400})));
    let (n, d) = (node(&s, i), node(&s, direct));
    assert_eq!(n.child_layout.sizing_h, Sizing::Fixed, "Row's Hug was never the instance's own: it takes Stacked's Fixed");
    assert_eq!((n.width, n.child_layout.sizing_h), (d.width, d.child_layout.sizing_h), "the same as an instance made from Stacked");
    let a = layer(&s, i, "A").unwrap();
    assert_eq!((node(&s, a).child_layout.sizing_h, node(&s, a).width), (Sizing::Fill, 300.0), "A fills the 300 wide instance");
    // Filling an auto layout parent afterwards leaves A filling, with nothing turned into an override.
    let col = idv(&ex(
        &mut s,
        "node.create",
        json!({"type": "frame", "name": "Column", "x": 600, "y": 0, "width": 500, "props": {"layout": "vertical", "sizingH": "fixed", "sizingV": "hug", "fills": []}}),
    ));
    ex(&mut s, "node.reparent", json!({"ids": [i], "parent": col}));
    ex(&mut s, "node.set", json!({"ids": [i], "props": {"sizingH": "fill"}}));
    let a = layer(&s, i, "A").unwrap();
    assert_eq!(node(&s, a).child_layout.sizing_h, Sizing::Fill, "A still fills");
    assert!(!node(&s, a).overrides.contains("resizing"), "A's resizing is still the main's");
    assert_eq!(node(&s, a).width, node(&s, i).width);
    // Resizing the instance set itself stays across a switch.
    let own = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 600})));
    ex(&mut s, "node.set", json!({"ids": [own], "props": {"minWidth": 120}}));
    ex(&mut s, "instance.setProperties", json!({"ids": [own], "properties": {"Layout": "Stacked"}}));
    assert_eq!(node(&s, own).child_layout.min_w, Some(120.0), "its own min width stays");
    assert_eq!(node(&s, own).child_layout.sizing_h, Sizing::Fixed, "its sizing, still Row's, follows Stacked");
    healthy(&s);
}

#[test]
fn variant_combinations_order_default_and_conflicts() {
    let mut s = Session::new();
    // Two variants created right to left: the layer order and the top-left corner disagree.
    let a = idv(&ex(&mut s, "frame.create", json!({"x": 200, "y": 0, "width": 80, "height": 40, "name": "Size=Large, State=Default"})));
    let b = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 80, "height": 40, "name": "Size=Small, State=Default"})));
    let c = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 80, "width": 80, "height": 40, "name": "Size=Small, State=Hover"})));
    let mut v = Vec::new();
    for f in [a, b, c] {
        v.push(idv(&ex(&mut s, "component.create", json!({"ids": [f]}))));
    }
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": v.clone()})));
    // Default: "the variant in the top-left corner".
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 300})));
    assert_eq!(component_of(&s, i), v[1], "the top-left variant is the default");
    let props = ex(&mut s, "component.properties", json!({"id": set}));
    assert_eq!(props["properties"][0]["default"], json!("Small"), "the default value is the top-left variant's");
    // Making Large the default moves its variant to the top-left corner (one undo step).
    let steps = undo_len(&s);
    ex(&mut s, "component.editProperty", json!({"id": set, "name": "Size", "default": "Large"}));
    assert_eq!(undo_len(&s), steps + 1);
    let j = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 400})));
    assert_eq!(component_of(&s, j), v[0], "the new default is the top-left variant");
    // Reordering properties rewrites every variant's name to the new order.
    ex(&mut s, "component.editProperty", json!({"id": set, "name": "State", "index": 0}));
    assert_eq!(node(&s, v[2]).name, "State=Hover, Size=Small");
    let props = ex(&mut s, "component.properties", json!({"id": i}));
    assert_eq!(props["properties"][0]["name"], json!("State"), "the order shows in the properties");
    // A missing combination picks the variant matching the most values (documentation silent; pinned).
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Size": "Large", "State": "Hover"}}));
    assert!(v.contains(&component_of(&s, i)));
    // Two variants with the same values are a conflict, reported until resolved.
    assert_eq!(ex(&mut s, "component.properties", json!({"id": set}))["conflicts"], json!([]));
    ex(&mut s, "component.editProperty", json!({"id": set, "name": "State", "value": "Hover", "newValue": "Default"}));
    let conflicts = ex(&mut s, "component.properties", json!({"id": set}))["conflicts"].clone();
    assert_eq!(conflicts, json!([[v[1].0, v[2].0]]), "duplicate variant combinations are reported as a conflict");
    healthy(&s);
}

/// Components "From" and "To" with layers named alike ("Bg" rectangle, "Title" text) and one only
/// "From" has ("Mark"); an instance of From with overrides on all three. Returns (from, to, inst).
fn swap_kit(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let mut comps = Vec::new();
    for (k, name) in ["From", "To"].iter().enumerate() {
        let f = idv(&ex(s, "frame.create", json!({"x": k as f64 * 200.0, "y": 0, "width": 100, "height": 40, "name": name})));
        ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 40, "parent": f, "props": {"name": "Bg", "fill": "#eeeeee"}}));
        ex(s, "text.create", json!({"text": name, "x": 4, "y": 4, "parent": f, "props": {"name": "Title"}}));
        if k == 0 {
            ex(s, "shape.ellipse", json!({"x": 80, "y": 4, "width": 8, "height": 8, "parent": f, "props": {"name": "Mark"}}));
        }
        comps.push(idv(&ex(s, "component.create", json!({"ids": [f]}))));
    }
    let i = idv(&ex(s, "instance.create", json!({"component": comps[0], "x": 0, "y": 200})));
    let ks = kids(s, i);
    ex(s, "node.set", json!({"id": ks[0], "props": {"fill": "#ff00ff"}}));
    ex(s, "node.set", json!({"id": ks[1], "props": {"characters": "Mine", "fill": "#00ff00"}}));
    ex(s, "node.set", json!({"id": ks[2], "props": {"opacity": 0.3}}));
    (comps[0], comps[1], i)
}

#[test]
fn swaps_keep_overrides_by_layer_name() {
    // (how, command, kept on Bg, derivation)
    let rows: [(&str, &str, bool, &str); 2] = [
        ("the instance menu", "instance.swap", true, "the layer names of the current instance and the instance you're selecting must match"),
        ("the Assets panel", "instance.swapAt", false, "an Assets-panel swap keeps only text overrides"),
    ];
    for (how, cmd, bg_kept, why) in rows {
        let mut s = Session::new();
        let (_from, to, i) = swap_kit(&mut s);
        let steps = undo_len(&s);
        let p = if cmd == "instance.swap" { json!({"ids": [i], "component": to}) } else { json!({"component": to, "x": 50, "y": 220}) };
        ex(&mut s, cmd, p);
        assert_eq!(undo_len(&s), steps + 1, "{how}: one undo step");
        healthy(&s);
        assert_eq!(component_of(&s, i), to, "{how}: swapped");
        let (bg, title) = (layer(&s, i, "Bg").unwrap(), layer(&s, i, "Title").unwrap());
        assert_eq!(fill(&s, bg) == "FF00FF", bg_kept, "{how}: Bg fill ({why})");
        assert_eq!((chars(&s, title), fill(&s, title)), ("Mine".to_string(), "00FF00".to_string()), "{how}: text layers keep their changes");
        assert!(layer(&s, i, "Mark").is_none(), "{how}: a layer the new main lacks goes with its overrides");
    }
    // Instance swap properties: preferred values are offered first, any component is allowed.
    let mut s = Session::new();
    let (from, to, _) = swap_kit(&mut s);
    let (icon, shape, button, nested) = crate::components::tests::button(&mut s);
    let _ = (shape, from);
    ex(&mut s, "component.addProperty", json!({"id": button, "name": "Icon", "type": "instanceSwap", "layer": nested, "preferred": [icon]}));
    let props = ex(&mut s, "component.properties", json!({"id": button}));
    assert_eq!(props["properties"][0]["preferred"], json!([icon.0]));
    let b = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 600, "y": 600})));
    ex(&mut s, "instance.setProperties", json!({"ids": [b], "properties": {"Icon": to}}));
    assert_eq!(component_of(&s, copy_of(&s, b, nested)), to, "a component outside the preferred values can still be picked");
    healthy(&s);
}

#[test]
fn swapping_to_a_component_of_another_set_takes_its_properties() {
    // Documentation silent beyond name matching (pinned): the instance shows the other set's
    // default variant's layers and offers that set's properties.
    let mut s = Session::new();
    let (set, _default, _hover) = button_set(&mut s);
    let a = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 500, "width": 60, "height": 20, "name": "Kind=Solid"})));
    ex(&mut s, "text.create", json!({"text": "Tag", "x": 2, "y": 2, "parent": a, "props": {"name": "Label"}}));
    let tag = idv(&ex(&mut s, "component.create", json!({"ids": [a]})));
    let tags = idv(&ex(&mut s, "component.combineVariants", json!({"ids": [tag]})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 200})));
    let t = layer(&s, i, "Label").unwrap();
    ex(&mut s, "node.set", json!({"id": t, "props": {"characters": "Kept"}}));
    ex(&mut s, "instance.swap", json!({"ids": [i], "component": tags}));
    healthy(&s);
    assert_eq!(component_of(&s, i), tag);
    assert_eq!(chars(&s, layer(&s, i, "Label").unwrap()), "Kept");
    let props = ex(&mut s, "component.properties", json!({"id": i}));
    assert_eq!(props["set"], json!(tags.0));
    assert_eq!(props["properties"][0]["name"], json!("Kind"));
}

#[test]
fn hidden_exposed_instances_hide_their_properties() {
    let mut s = Session::new();
    let (_icon, _shape, button, nested) = crate::components::tests::button(&mut s);
    ex(&mut s, "component.exposeInstance", json!({"id": nested}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    assert_eq!(ex(&mut s, "instance.exposed", json!({"id": i}))["instances"].as_array().unwrap().len(), 1);
    // "Hidden nested instances hide their attached properties."
    let t = copy_of(&s, i, nested);
    ex(&mut s, "node.set", json!({"id": t, "props": {"visible": false}}));
    assert_eq!(ex(&mut s, "instance.exposed", json!({"id": i}))["instances"], json!([]));
}

#[test]
fn detach_reset_push_and_restore() {
    // Detach: "a regular frame, while keeping the current layers and properties".
    let mut s = Session::new();
    let k = kit(&mut s);
    let row = k.copy(&s, Layer::Row);
    ex(&mut s, "node.set", json!({"id": row, "props": {"padding": 9}}));
    let t = k.copy(&s, Layer::Bg);
    ex(&mut s, "node.set", json!({"id": t, "props": {"fill": "#00ff00"}}));
    let look = figcraft_render::render_node(s.doc().unwrap(), k.inst, 1.0).unwrap();
    let steps = undo_len(&s);
    ex(&mut s, "component.detach", json!({"ids": [k.inst]}));
    assert_eq!(undo_len(&s), steps + 1);
    assert_eq!(node(&s, k.inst).kind.type_name(), "FRAME");
    assert_eq!(figcraft_render::render_node(s.doc().unwrap(), k.inst, 1.0).unwrap().data(), look.data(), "the look is kept");
    assert_eq!(node(&s, row).layout.as_ref().map(|l| l.padding), Some([9.0; 4]));
    healthy(&s);

    // Reset all: every layer shows its main again, property values go back to their defaults;
    // the instance keeps its size (documentation silent; pinned).
    let mut s = Session::new();
    let k = kit(&mut s);
    let grid = serde_json::to_value(LayoutGrid::default()).unwrap();
    let t = k.copy(&s, Layer::Row);
    ex(&mut s, "node.set", json!({"id": t, "props": {"padding": 9, "grids": [grid]}}));
    let t = k.copy(&s, Layer::Title);
    ex(&mut s, "node.set", json!({"id": t, "props": {"characters": "Mine", "paragraphSpacing": 6}}));
    ex(&mut s, "node.set", json!({"id": k.inst, "props": {"width": 240}}));
    ex(&mut s, "component.resetInstance", json!({"ids": [k.inst]}));
    healthy(&s);
    let (row, title) = (node(&s, k.copy(&s, Layer::Row)), node(&s, k.copy(&s, Layer::Title)));
    assert_eq!((row.layout.as_ref().map(|l| l.padding), row.grids.len()), (Some([4.0; 4]), 0));
    assert_eq!((text(&title).characters.as_str(), text(&title).paragraph_spacing), ("Hello", 0.0));
    assert_eq!(node(&s, k.inst).width, 240.0);

    // Push changes: the instance's overrides become the main's ("cannot push changes to nested
    // components": from a nested layer they still go to the outer main).
    let mut s = Session::new();
    let (icon, shape, button, nested) = crate::components::tests::button(&mut s);
    let a = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    let b = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 600, "y": 400})));
    let ishape = copy_of(&s, a, copy_of(&s, nested, shape));
    ex(&mut s, "node.set", json!({"id": ishape, "props": {"fill": "#ff00ff"}}));
    let label = kids(&s, a)[1];
    ex(&mut s, "node.set", json!({"id": label, "props": {"paragraphSpacing": 5}}));
    ex(&mut s, "component.pushChanges", json!({"id": ishape}));
    healthy(&s);
    assert_eq!(fill(&s, copy_of(&s, nested, shape)), "FF00FF", "the outer main's nested instance takes it");
    assert_eq!(fill(&s, shape), "111111", "the nested instance's own main doesn't");
    assert_eq!(fill(&s, copy_of(&s, b, copy_of(&s, nested, shape))), "FF00FF", "other instances follow");
    assert_eq!(text(&node(&s, kids(&s, button)[1])).paragraph_spacing, 5.0, "text properties are pushed");
    assert!(node(&s, ishape).overrides.is_empty() && node(&s, label).overrides.is_empty());
    let _ = icon;

    // Go to main selects it; restore main brings a deleted main back from an instance
    // (`components_tests::deleted_main_keeps_instances_and_restores`).
    let mut s = Session::new();
    let k = kit(&mut s);
    let t = k.copy(&s, Layer::Title);
    ex(&mut s, "component.goToMain", json!({"id": t}));
    assert_eq!(s.selection(), vec![k.card]);
}

#[test]
fn auto_layout_instances_follow_their_main() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 40, "name": "Chip"})));
    for _ in 0..2 {
        ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20, "parent": f}));
    }
    ex(&mut s, "node.set", json!({"id": f, "props": {"layout": "horizontal", "padding": 8, "gap": 4, "sizingH": "hug", "sizingV": "hug"}}));
    let chip = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": chip, "x": 0, "y": 100})));
    assert_eq!((node(&s, i).width, node(&s, i).height), (60.0, 36.0), "8 + 20 + 4 + 20 + 8 by 8 + 20 + 8");
    // Padding on the instance is an override; the main's gap and direction still reach it.
    ex(&mut s, "node.set", json!({"id": i, "props": {"padding": 2}}));
    ex(&mut s, "node.set", json!({"id": chip, "props": {"gap": 10}}));
    assert_eq!(node(&s, i).width, 2.0 + 20.0 + 10.0 + 20.0 + 2.0, "the hug follows the main's gap and the instance's padding");
    ex(&mut s, "node.set", json!({"id": chip, "props": {"layout": "vertical"}}));
    let l = node(&s, i).layout.unwrap();
    assert_eq!((l.mode, l.padding, l.gap), (LayoutMode::Vertical, [2.0; 4], 10.0));
    assert_eq!((node(&s, i).width, node(&s, i).height), (24.0, 2.0 + 20.0 + 10.0 + 20.0 + 2.0));
    // The direction is the main's: refused on the instance.
    assert!(s.execute("node.set", &json!({"id": i, "props": {"layout": "horizontal"}})).is_err());
    assert!(s.execute("layout.remove", &json!({"ids": [i]})).is_err());
    // Min / max width on the instance is its own (documentation silent; pinned).
    ex(&mut s, "node.set", json!({"id": i, "props": {"minWidth": 80}}));
    assert_eq!(node(&s, i).width, 80.0);
    ex(&mut s, "node.set", json!({"id": chip, "props": {"gap": 12}}));
    assert_eq!(node(&s, i).child_layout.min_w, Some(80.0));
    healthy(&s);
}

#[test]
fn a_new_component_set_is_named_after_its_first_variant() {
    // "Button/State=Hover" groups under "Button": the set takes the path before the last '/'.
    // A bare property name ("State=Default, Size=Small") names no set, and neither does an empty
    // name: the set gets the next numbered default, like a new component. A plain name is kept.
    let rows = [
        ("Button/State=Hover", "Button", "slash-prefixed: the path before the last '/'"),
        ("Icons/Arrow/Dir=Up", "Icons/Arrow", "nested path: everything before the last '/'"),
        ("State=Default, Size=Small", "Component 1", "bare prop=value: not \"State\""),
        ("", "Component 1", "empty name: the numbered default"),
    ];
    for (name, expected, why) in rows {
        let mut s = Session::new();
        let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 40, "height": 20})));
        ex(&mut s, "node.set", json!({"id": f, "props": {"name": name}}));
        let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
        assert_eq!(node(&s, c).name, name, "{why}: the variant keeps its name");
        let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": [c]})));
        assert_eq!(node(&s, set).name, expected, "{why}");
        healthy(&s);
    }

    // Add variant on a lone component (a variant property on it) wraps it in a set named after it.
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 40, "height": 20, "name": "Button"})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let set = idv(&ex(&mut s, "component.addProperty", json!({"id": c, "name": "State", "type": "variant"})));
    assert_eq!(node(&s, set).name, "Button");
    assert_eq!(node(&s, c).name, "State=Default");
    healthy(&s);
}
