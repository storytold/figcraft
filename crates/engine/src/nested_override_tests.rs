//! Overrides on and inside nested instances across swaps, variant switches, main updates, detach
//! and copy/paste.

use crate::Session;
use crate::components::tests::{button, copy_of, ex, fill, healthy, idv, kids, node};
use crate::components::*;
use figcraft_doc::*;
use serde_json::json;

/// A 24×24 icon component at (`x`, 300) with one 20×20 rectangle named `layer`.
fn icon(s: &mut Session, x: f64, name: &str, layer: &str) -> NodeId {
    let f = idv(&ex(s, "frame.create", json!({"x": x, "y": 300, "width": 24, "height": 24, "name": name})));
    ex(s, "shape.rectangle", json!({"x": 2, "y": 2, "width": 20, "height": 20, "parent": f, "props": {"name": layer, "fill": "#111111"}}));
    idv(&ex(s, "component.create", json!({"ids": [f]})))
}

fn main_of(s: &Session, id: NodeId) -> NodeId {
    match node(s, id).kind {
        NodeKind::Instance { component, .. } => component,
        _ => panic!("{id} is not an instance"),
    }
}

#[test]
fn swapping_a_nested_instance_keeps_overrides_on_matching_layers() {
    let mut s = Session::new();
    let (_icon, shape, button, nested) = button(&mut s);
    let other = icon(&mut s, 200.0, "Other", "Shape");
    let unrelated = icon(&mut s, 300.0, "Unrelated", "Glyph");
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    let inested = copy_of(&s, i, nested);
    let ishape = kids(&s, inested)[0];
    ex(&mut s, "node.set", json!({"id": ishape, "props": {"fill": "#ff00ff", "opacity": 0.5}}));
    // A main with a same-named layer: the overrides carry over.
    ex(&mut s, "instance.swap", json!({"ids": [inested], "component": other}));
    healthy(&s);
    let ishape = kids(&s, inested)[0];
    assert_eq!(main_of(&s, inested), other);
    assert_eq!(fill(&s, ishape), "FF00FF");
    assert_eq!(node(&s, ishape).opacity, 0.5);
    assert!(node(&s, ishape).overrides.contains("fills"));
    // ... and survive edits of the outer main and of the swapped-in main.
    ex(&mut s, "node.set", json!({"id": button, "props": {"fill": "#00ff00"}}));
    let other_shape = kids(&s, other)[0];
    ex(&mut s, "node.set", json!({"id": other_shape, "props": {"cornerRadius": 5, "fill": "#222222"}}));
    healthy(&s);
    let ishape = kids(&s, copy_of(&s, i, nested))[0];
    assert_eq!(fill(&s, ishape), "FF00FF");
    assert_eq!(node(&s, ishape).corners.radii, [5.0; 4]);
    // No matching layer: the overrides go.
    ex(&mut s, "instance.swap", json!({"ids": [inested], "component": unrelated}));
    healthy(&s);
    assert_eq!(fill(&s, kids(&s, inested)[0]), "111111");
    // Back to the original: it shows its main again.
    ex(&mut s, "component.resetOverride", json!({"id": inested, "group": "component"}));
    healthy(&s);
    assert_eq!(fill(&s, kids(&s, inested)[0]), fill(&s, copy_of(&s, nested, shape)));
}

#[test]
fn variant_switch_keeps_nested_overrides_and_swaps() {
    let mut s = Session::new();
    let ic = icon(&mut s, 0.0, "Icon", "Shape");
    let star = icon(&mut s, 100.0, "Star", "Shape");
    let mut variants = Vec::new();
    for (k, state) in ["Default", "Hover"].iter().enumerate() {
        let f =
            idv(&ex(&mut s, "frame.create", json!({"x": k as f64 * 200.0, "y": 0, "width": 120, "height": 40, "name": format!("State={state}")})));
        ex(
            &mut s,
            "shape.rectangle",
            json!({"x": 0, "y": 0, "width": 120, "height": 40, "parent": f, "props": {"name": "Bg", "fill": if k == 0 { "#ffffff" } else { "#eeeeee" }}}),
        );
        ex(&mut s, "instance.create", json!({"component": ic, "x": 8, "y": 8, "parent": f}));
        variants.push(idv(&ex(&mut s, "component.create", json!({"ids": [f]}))));
    }
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": variants.clone()})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 200})));
    let inested = kids(&s, i)[1];
    let ishape = kids(&s, inested)[0];
    ex(&mut s, "node.set", json!({"id": ishape, "props": {"fill": "#ff8800"}}));
    ex(&mut s, "node.set", json!({"id": inested, "props": {"opacity": 0.4}}));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"State": "Hover"}}));
    healthy(&s);
    let inested = kids(&s, i)[1];
    assert_eq!(fill(&s, kids(&s, i)[0]), "EEEEEE", "the new variant shows");
    assert_eq!(fill(&s, kids(&s, inested)[0]), "FF8800", "override inside the nested instance kept");
    assert_eq!(node(&s, inested).opacity, 0.4, "override on the nested instance kept");
    // A swap override on the nested instance survives switching back.
    ex(&mut s, "instance.swap", json!({"ids": [inested], "component": star}));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"State": "Default"}}));
    healthy(&s);
    let inested = kids(&s, i)[1];
    assert_eq!(main_of(&s, inested), star);
    assert_eq!(fill(&s, kids(&s, inested)[0]), "FF8800");
}

#[test]
fn nested_overrides_survive_main_restructuring() {
    let mut s = Session::new();
    let (_icon, shape, button, nested) = button(&mut s);
    let other = icon(&mut s, 200.0, "Other", "Shape");
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    let ishape = copy_of(&s, i, copy_of(&s, nested, shape));
    ex(&mut s, "node.set", json!({"id": ishape, "props": {"fill": "#ff00ff"}}));
    // The main wraps its nested instance in a frame, then reorders it.
    let wrap = idv(&ex(&mut s, "object.frameSelection", json!({"ids": [nested]})));
    healthy(&s);
    let ishape = copy_of(&s, i, copy_of(&s, nested, shape));
    assert_eq!(fill(&s, ishape), "FF00FF", "kept when the main moves the nested instance into a frame");
    ex(&mut s, "node.reorder", json!({"id": wrap, "index": 5}));
    healthy(&s);
    assert_eq!(fill(&s, copy_of(&s, i, copy_of(&s, nested, shape))), "FF00FF");
    // The main swaps its nested instance to a main with the same layer.
    ex(&mut s, "instance.swap", json!({"ids": [nested], "component": other}));
    healthy(&s);
    let inested = copy_of(&s, i, nested);
    assert_eq!(main_of(&s, inested), other);
    assert_eq!(fill(&s, kids(&s, inested)[0]), "FF00FF");
}

#[test]
fn detach_keeps_swapped_nested_instances_and_their_overrides() {
    let mut s = Session::new();
    let (_icon, _shape, button, nested) = button(&mut s);
    let other = icon(&mut s, 200.0, "Other", "Glyph");
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    let inested = copy_of(&s, i, nested);
    ex(&mut s, "instance.swap", json!({"ids": [inested], "component": other}));
    let iglyph = kids(&s, inested)[0];
    ex(&mut s, "node.set", json!({"id": iglyph, "props": {"fill": "#00aaff"}}));
    ex(&mut s, "component.detach", json!({"ids": [i]}));
    healthy(&s);
    assert_eq!(main_of(&s, inested), other);
    assert_eq!(node(&s, inested).source, Some(other));
    assert!(!node(&s, inested).overrides.contains("component"));
    let glyph = kids(&s, other)[0];
    assert_eq!(node(&s, iglyph).source, Some(glyph));
    assert_eq!(fill(&s, iglyph), "00AAFF");
    ex(&mut s, "node.set", json!({"id": glyph, "props": {"fill": "#000000", "cornerRadius": 3}}));
    assert_eq!(fill(&s, iglyph), "00AAFF");
    assert_eq!(node(&s, iglyph).corners.radii, [3.0; 4]);
    healthy(&s);
}

#[test]
fn copied_instances_keep_their_overrides() {
    let mut s = Session::new();
    let (icon, shape, button, nested) = button(&mut s);
    ex(&mut s, "component.addProperty", json!({"id": icon, "name": "Filled", "type": "boolean", "default": true, "layer": shape}));
    ex(&mut s, "component.exposeInstance", json!({"id": nested}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    let inested = copy_of(&s, i, nested);
    let ishape = kids(&s, inested)[0];
    let ilabel = kids(&s, i)[1];
    ex(&mut s, "node.set", json!({"id": ishape, "props": {"fill": "#ff00ff"}}));
    ex(&mut s, "text.set", json!({"id": ilabel, "characters": "Copied"}));
    ex(&mut s, "instance.setProperties", json!({"ids": [inested], "properties": {"Filled": false}}));
    // The whole instance: overrides and property values come along, and survive main edits.
    ex(&mut s, "edit.copy", json!({"ids": [i]}));
    s.set_selection(vec![]);
    let j = ex(&mut s, "edit.paste", json!({"x": 300, "y": 800}))["ids"][0].as_u64().map(NodeId).unwrap();
    healthy(&s);
    ex(&mut s, "node.set", json!({"id": shape, "props": {"cornerRadius": 4}}));
    ex(&mut s, "node.set", json!({"id": button, "props": {"fill": "#eeeeee"}}));
    healthy(&s);
    let jnested = copy_of(&s, j, nested);
    let jshape = kids(&s, jnested)[0];
    assert_eq!(fill(&s, jshape), "FF00FF");
    assert!(!node(&s, jshape).visible, "property value of the nested instance kept");
    assert_eq!(node(&s, jshape).corners.radii, [4.0; 4]);
    assert_eq!(node(&s, kids(&s, j)[1]).kind.text().unwrap().characters, "Copied");
    assert_eq!(fill(&s, j), "EEEEEE");
    // A nested instance copied out on its own becomes an instance of its own main, same look.
    let inested = copy_of(&s, i, nested);
    ex(&mut s, "edit.copy", json!({"ids": [inested]}));
    s.set_selection(vec![]);
    let k = ex(&mut s, "edit.paste", json!({"x": 900, "y": 800}))["ids"][0].as_u64().map(NodeId).unwrap();
    healthy(&s);
    assert!(!inside_instance(s.doc().unwrap(), k));
    assert_eq!(node(&s, k).source, Some(icon));
    let kshape = kids(&s, k)[0];
    assert_eq!(node(&s, kshape).source, Some(shape));
    assert_eq!(fill(&s, kshape), "FF00FF");
    assert!(node(&s, kshape).overrides.contains("fills"));
    assert!(!node(&s, kshape).visible);
    assert!(!node(&s, kshape).overrides.contains("visible"), "set by its property, not an override");
    ex(&mut s, "instance.setProperties", json!({"ids": [k], "properties": {"Filled": true}}));
    assert!(node(&s, kids(&s, k)[0]).visible);
    ex(&mut s, "node.set", json!({"id": shape, "props": {"cornerRadius": 7}}));
    assert_eq!(node(&s, kids(&s, k)[0]).corners.radii, [7.0; 4]);
    assert_eq!(fill(&s, kids(&s, k)[0]), "FF00FF");
    // A plain layer copied out of an instance is a plain layer.
    let ilabel = kids(&s, i)[1];
    ex(&mut s, "edit.copy", json!({"ids": [ilabel]}));
    s.set_selection(vec![]);
    let t = ex(&mut s, "edit.paste", json!({"x": 900, "y": 1000}))["ids"][0].as_u64().map(NodeId).unwrap();
    assert!(node(&s, t).source.is_none() && node(&s, t).overrides.is_empty());
    healthy(&s);
}

/// `button()` with the main's copy of the icon's Shape filled red, and an instance-swap property
/// `Icon` on the nested icon: (icon, shape, button, nested).
fn button_with_icon_override(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let (icon_main, shape, button, nested) = button(s);
    let main_shape = copy_of(s, nested, shape);
    ex(s, "node.set", json!({"id": main_shape, "props": {"fill": "#ff0000"}}));
    ex(s, "component.addProperty", json!({"id": button, "name": "Icon", "type": "instanceSwap", "default": icon_main, "layer": nested}));
    (icon_main, shape, button, nested)
}

/// Swap instance `i`'s icon to `to` by its instance-swap property `Icon`.
fn swap_icon(s: &mut Session, i: NodeId, to: NodeId) {
    ex(s, "instance.setProperties", json!({"ids": [i], "properties": {"Icon": to}}));
    healthy(s);
}

#[test]
fn a_main_override_carries_onto_a_swapped_nested_instance() {
    // Documented: overrides survive a swap when layer names match, and a swapped icon takes the
    // colour the main component set (forum thread with staff replies). Rows: by the instance-swap
    // property and by instance.swap, onto a matching and an unmatched name. The main's size
    // override is geometry and stays out; the instance's own override wins over the main's.
    let mut s = Session::new();
    let (_icon, shape, button, nested) = button_with_icon_override(&mut s);
    let main_shape = copy_of(&s, nested, shape);
    ex(&mut s, "node.set", json!({"id": main_shape, "props": {"width": 10}}));
    let other = icon(&mut s, 200.0, "Other", "Shape");
    let unmatched = icon(&mut s, 300.0, "Unmatched", "Glyph");
    for (by_property, to, want) in [(true, other, "FF0000"), (false, other, "FF0000"), (true, unmatched, "111111"), (false, unmatched, "111111")] {
        let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
        if by_property {
            swap_icon(&mut s, i, to);
        } else {
            let inested = copy_of(&s, i, nested);
            ex(&mut s, "instance.swap", json!({"ids": [inested], "component": to}));
            healthy(&s);
        }
        let glyph = kids(&s, copy_of(&s, i, nested))[0];
        assert_eq!(fill(&s, glyph), want, "by property {by_property}, to {to}");
        assert_eq!(node(&s, glyph).width, 20.0, "the main's size override stays out");
    }
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 600})));
    let glyph = kids(&s, copy_of(&s, i, nested))[0];
    ex(&mut s, "node.set", json!({"id": glyph, "props": {"fill": "#0000ff"}}));
    swap_icon(&mut s, i, other);
    assert_eq!(fill(&s, kids(&s, copy_of(&s, i, nested))[0]), "0000FF", "the instance's own override wins");
}

#[test]
fn a_main_override_carries_onto_a_swap_one_level_deeper() {
    // As above for a Button instance inside a Toolbar main, swapped in a Toolbar instance.
    let mut s = Session::new();
    let (_icon, _shape, button, nested) = button_with_icon_override(&mut s);
    let other = icon(&mut s, 200.0, "Other", "Shape");
    let bar = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 600, "width": 300, "height": 60, "name": "Toolbar"})));
    let inner = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 10, "y": 10, "parent": bar})));
    let toolbar = idv(&ex(&mut s, "component.create", json!({"ids": [bar]})));
    let t = idv(&ex(&mut s, "instance.create", json!({"component": toolbar, "x": 0, "y": 800})));
    let icon_in_t = copy_of(&s, t, copy_of(&s, inner, nested));
    ex(&mut s, "instance.swap", json!({"ids": [icon_in_t], "component": other}));
    healthy(&s);
    assert_eq!(fill(&s, kids(&s, icon_in_t)[0]), "FF0000");
}

#[test]
fn a_swapped_nested_instance_keeps_the_main_override_it_took() {
    // Documented (staff reply): later edits of the main's icon colour don't reach instances that
    // swapped, and do reach the rest. Undo and redo follow; resetting the override shows the icon
    // main's own fill.
    let mut s = Session::new();
    let (_icon, shape, button, nested) = button_with_icon_override(&mut s);
    let other = icon(&mut s, 200.0, "Other", "Shape");
    let swapped = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    let kept = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 500})));
    swap_icon(&mut s, swapped, other);
    let glyph = |s: &Session, i: NodeId| fill(s, kids(s, copy_of(s, i, nested))[0]);
    let main_shape = copy_of(&s, nested, shape);
    ex(&mut s, "node.set", json!({"id": main_shape, "props": {"fill": "#00ff00"}}));
    healthy(&s);
    assert_eq!((glyph(&s, swapped), glyph(&s, kept)), ("FF0000".into(), "00FF00".into()));
    s.undo().unwrap();
    assert_eq!((glyph(&s, swapped), glyph(&s, kept)), ("FF0000".into(), "FF0000".into()));
    s.redo().unwrap();
    assert_eq!((glyph(&s, swapped), glyph(&s, kept)), ("FF0000".into(), "00FF00".into()));
    let swapped_glyph = kids(&s, copy_of(&s, swapped, nested))[0];
    ex(&mut s, "component.resetOverride", json!({"id": swapped_glyph, "group": "fills"}));
    healthy(&s);
    assert_eq!(glyph(&s, swapped), "111111");
}

#[test]
fn a_swapped_in_icon_keeps_its_own_overrides_to_itself() {
    // Docs silent; pinned: what an icon main overrides inside itself stays that main's. After a
    // swap to it, its edits still reach the instance; a second swap doesn't carry them on.
    let mut s = Session::new();
    let (_icon, _shape, button, nested) = button_with_icon_override(&mut s);
    let dot = icon(&mut s, 200.0, "Dot", "Shape");
    let with_dot = |s: &mut Session, x: f64, name: &str, fill: Option<&str>| {
        let f = idv(&ex(s, "frame.create", json!({"x": x, "y": 300, "width": 24, "height": 24, "name": name})));
        let d = idv(&ex(s, "instance.create", json!({"component": dot, "x": 0, "y": 0, "parent": f})));
        if let Some(c) = fill {
            let shape = kids(s, d)[0];
            ex(s, "node.set", json!({"id": shape, "props": {"fill": c}}));
        }
        (idv(&ex(s, "component.create", json!({"ids": [f]}))), d)
    };
    let (other2, dot2) = with_dot(&mut s, 300.0, "Other2", Some("#0000ff"));
    let (other3, _) = with_dot(&mut s, 400.0, "Other3", None);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    swap_icon(&mut s, i, other2);
    let shape_in = |s: &Session| kids(s, kids(s, copy_of(s, i, nested))[0])[0];
    assert_eq!(fill(&s, shape_in(&s)), "0000FF");
    let shape2 = kids(&s, dot2)[0];
    ex(&mut s, "node.set", json!({"id": shape2, "props": {"fill": "#00ffff"}}));
    healthy(&s);
    assert_eq!(fill(&s, shape_in(&s)), "00FFFF", "Other2's own edit reaches it");
    swap_icon(&mut s, i, other3);
    assert_eq!(fill(&s, shape_in(&s)), "111111", "Other2's override doesn't carry onto Other3");
}

#[test]
fn a_swapped_out_icons_own_overrides_stay_with_it() {
    // Docs silent; pinned: what the Button's default icon main overrides inside itself isn't the
    // Button main's to carry. Icon A colours its Base's Shape green; swapping to icon B, also built
    // on Base, shows Base's own fill.
    let mut s = Session::new();
    let base = icon(&mut s, 0.0, "Base", "Shape");
    let on_base = |s: &mut Session, x: f64, name: &str| {
        let f = idv(&ex(s, "frame.create", json!({"x": x, "y": 500, "width": 24, "height": 24, "name": name})));
        let b = idv(&ex(s, "instance.create", json!({"component": base, "x": 0, "y": 0, "parent": f})));
        (idv(&ex(s, "component.create", json!({"ids": [f]}))), b)
    };
    let (a, a_base) = on_base(&mut s, 100.0, "A");
    let (b, _) = on_base(&mut s, 200.0, "B");
    let a_shape = kids(&s, a_base)[0];
    ex(&mut s, "node.set", json!({"id": a_shape, "props": {"fill": "#00ff00"}}));
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 700, "width": 40, "height": 40, "name": "Button"})));
    let nested = idv(&ex(&mut s, "instance.create", json!({"component": a, "x": 4, "y": 4, "parent": f})));
    let button = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "component.addProperty", json!({"id": button, "name": "Icon", "type": "instanceSwap", "default": a, "layer": nested}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 0, "y": 800})));
    swap_icon(&mut s, i, b);
    let shape = kids(&s, kids(&s, copy_of(&s, i, nested))[0])[0];
    assert_eq!(fill(&s, shape), "111111");
    assert!(node(&s, shape).overrides.is_empty());
}

#[test]
fn a_slot_the_main_filled_shows_the_swapped_in_mains_content() {
    // Docs silent on slot content across a swap; pinned: a Chip the Panel main put in its card's
    // slot doesn't carry onto the swapped-in card, which shows its own default content.
    let mut s = Session::new();
    let card = |s: &mut Session, x: f64, name: &str| {
        let f = idv(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 100, "height": 60, "name": name})));
        let content = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 60, "parent": f, "name": "Content"})));
        ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20, "parent": content, "props": {"name": format!("{name} default")}}));
        let c = idv(&ex(s, "component.create", json!({"ids": [f]})));
        ex(s, "component.makeSlot", json!({"id": content}));
        c
    };
    let (a, b) = (card(&mut s, 0.0, "A"), card(&mut s, 200.0, "B"));
    let chip = icon(&mut s, 400.0, "Chip", "Dot");
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 400, "width": 200, "height": 100, "name": "Panel"})));
    let nested = idv(&ex(&mut s, "instance.create", json!({"component": a, "x": 0, "y": 0, "parent": f})));
    let panel = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    ex(&mut s, "instance.slotInsert", json!({"id": nested, "slot": "Content", "component": chip, "replace": true}));
    ex(&mut s, "component.addProperty", json!({"id": panel, "name": "Card", "type": "instanceSwap", "default": a, "layer": nested}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": panel, "x": 0, "y": 700})));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Card": b}}));
    healthy(&s);
    let content = kids(&s, copy_of(&s, i, nested))[0];
    let names: Vec<String> = kids(&s, content).into_iter().map(|k| node(&s, k).name.clone()).collect();
    assert_eq!(names, ["B default"]);
    assert!(!node(&s, content).overrides.contains(SLOT));
}
