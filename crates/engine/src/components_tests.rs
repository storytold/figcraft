//! Component / instance behaviour tests (overrides, structure, nesting, variants, properties).

use super::*;
use crate::{EngineError, Session, params::id_of};
use serde_json::{Value, json};

pub(crate) fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    match s.execute(cmd, &p) {
        Ok(v) => v,
        Err(e) => panic!("{cmd} {p}: {e}"),
    }
}

pub(crate) fn idv(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

pub(crate) fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().node(id).unwrap().clone()
}

pub(crate) fn kids(s: &Session, id: NodeId) -> Vec<NodeId> {
    node(s, id).children
}

pub(crate) fn fill(s: &Session, id: NodeId) -> String {
    node(s, id).fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

pub(crate) fn chars(s: &Session, id: NodeId) -> String {
    node(s, id).kind.text().map(|t| t.characters.clone()).unwrap_or_default()
}

pub(crate) fn healthy(s: &Session) {
    let doc = s.doc().unwrap();
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());
    assert!(mirror_issues(doc).is_empty(), "{:?}", mirror_issues(doc));
}

/// The instance layer mirroring main layer `src`.
pub(crate) fn copy_of(s: &Session, inst: NodeId, src: NodeId) -> NodeId {
    let doc = s.doc().unwrap();
    doc.subtree(inst).into_iter().find(|i| doc.node(*i).and_then(|n| n.source) == Some(src)).unwrap()
}

/// Card: 200×100 component with "Bg" rectangle, "Title" text and "Dot" ellipse.
pub(crate) fn card(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})));
    let bg =
        idv(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 100, "parent": f, "props": {"name": "Bg", "fill": "#eeeeee"}})));
    let title = idv(&ex(s, "text.create", json!({"text": "Hello", "x": 10, "y": 10, "parent": f, "props": {"name": "Title"}})));
    let dot =
        idv(&ex(s, "shape.ellipse", json!({"x": 170, "y": 10, "width": 20, "height": 20, "parent": f, "props": {"name": "Dot", "fill": "#ff0000"}})));
    let c = idv(&ex(s, "component.create", json!({"ids": [f]})));
    (c, bg, title, dot)
}

#[test]
fn overrides_survive_main_edits() {
    let mut s = Session::new();
    let (c, bg, title, dot) = card(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    healthy(&s);
    let (ibg, ititle, idot) = (copy_of(&s, i, bg), copy_of(&s, i, title), copy_of(&s, i, dot));
    // Overrides on the instance.
    ex(&mut s, "node.set", json!({"id": ibg, "props": {"fill": "#00ff00", "stroke": "#0000ff", "strokeWeight": 2, "name": "Background"}}));
    ex(&mut s, "node.set", json!({"id": ibg, "props": {"effects": [{"type": "dropShadow"}]}}));
    ex(&mut s, "text.set", json!({"id": ititle, "characters": "Overridden"}));
    ex(&mut s, "node.set", json!({"id": idot, "props": {"visible": false}}));
    let ov = node(&s, ibg).overrides;
    for g in ["fills", "strokes", "effects", "name"] {
        assert!(ov.contains(g), "{g} in {ov:?}");
    }
    // Main edits: overridden groups stay, the rest follows.
    ex(&mut s, "node.set", json!({"id": bg, "props": {"fill": "#123456", "cornerRadius": 8, "name": "Bg2", "strokeWeight": 5}}));
    ex(&mut s, "text.set", json!({"id": title, "characters": "Main text"}));
    ex(&mut s, "node.set", json!({"id": title, "props": {"fontSize": 30}}));
    ex(&mut s, "node.set", json!({"id": dot, "props": {"fill": "#00ffff", "width": 10, "visible": true}}));
    ex(&mut s, "node.set", json!({"id": c, "props": {"fill": "#fafafa"}}));
    healthy(&s);
    assert_eq!(copy_of(&s, i, bg), ibg, "ids are stable across syncs");
    let n = node(&s, ibg);
    assert_eq!(fill(&s, ibg), "00FF00");
    assert_eq!(n.name, "Background");
    assert_eq!(n.corners.radii, [8.0; 4], "non-overridden corners follow");
    assert_eq!(n.stroke.weight, 2.0, "overridden stroke keeps its weight");
    assert_eq!(n.effects.len(), 1);
    assert_eq!(chars(&s, ititle), "Overridden");
    assert_eq!(node(&s, ititle).kind.text().unwrap().style.font_size, 30.0, "text style follows the main");
    let d = node(&s, idot);
    assert!(!d.visible);
    assert_eq!(d.width, 10.0, "the main's size reaches the layer");
    assert_eq!(fill(&s, idot), "00FFFF");
    assert_eq!(fill(&s, i), "FAFAFA");
    // Per-group reset.
    ex(&mut s, "component.resetOverride", json!({"id": ibg, "group": "fills"}));
    assert_eq!(fill(&s, ibg), "123456");
    assert_eq!(node(&s, ibg).name, "Background", "other overrides stay");
    assert!(s.execute("component.resetOverride", &json!({"id": ibg, "group": "bogus"})).is_err());
    // Reset all.
    ex(&mut s, "component.resetInstance", json!({"ids": [i]}));
    assert_eq!(node(&s, ibg).name, "Bg2");
    assert_eq!(chars(&s, ititle), "Main text");
    assert!(node(&s, idot).visible);
    assert_eq!(node(&s, idot).width, 10.0);
    assert!(s.doc().unwrap().subtree(i).iter().all(|d| node(&s, *d).overrides.is_empty()));
    healthy(&s);
}

#[test]
fn main_structure_edits_propagate() {
    let mut s = Session::new();
    let (c, bg, title, dot) = card(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    let idot = copy_of(&s, i, dot);
    ex(&mut s, "node.set", json!({"id": idot, "props": {"fill": "#000000"}}));
    // Add a layer.
    let star = idv(&ex(&mut s, "shape.star", json!({"x": 100, "y": 50, "width": 20, "height": 20, "parent": c})));
    healthy(&s);
    assert_eq!(kids(&s, i).len(), 4);
    // Reorder: dot to the bottom.
    ex(&mut s, "node.reorder", json!({"id": dot, "index": 0}));
    healthy(&s);
    assert_eq!(kids(&s, i)[0], idot);
    assert_eq!(fill(&s, idot), "000000");
    // Remove layers.
    ex(&mut s, "edit.delete", json!({"ids": [title, star]}));
    healthy(&s);
    assert_eq!(kids(&s, i).len(), 2);
    assert_eq!(fill(&s, idot), "000000", "overrides on surviving layers stay");
    assert_eq!(copy_of(&s, i, bg), kids(&s, i)[1]);
    // Undo restores the layer in the instance too.
    s.undo().unwrap();
    assert_eq!(kids(&s, i).len(), 4);
    healthy(&s);
}

#[test]
fn instances_refuse_structure_edits() {
    let mut s = Session::new();
    let (c, bg, _title, dot) = card(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    let (ibg, idot) = (copy_of(&s, i, bg), copy_of(&s, i, dot));
    let before = s.doc().unwrap().clone();
    let undo = s.ds().unwrap().history.undo.len();
    let refused = [
        ("edit.delete", json!({"ids": [ibg]})),
        ("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5, "parent": i})),
        ("node.reorder", json!({"id": idot, "index": 0})),
        ("object.group", json!({"ids": [ibg, idot]})),
        ("node.reparent", json!({"ids": [idot], "parent": s.page().unwrap()})),
        ("edit.duplicate", json!({"ids": [idot]})),
        ("component.create", json!({"ids": [idot]})),
    ];
    for (cmd, p) in refused {
        let r = s.execute(cmd, &p);
        assert!(matches!(r, Err(EngineError::Disabled(..)) | Err(EngineError::BadParams { .. })), "{cmd} should be refused: {r:?}");
        assert_eq!(*s.doc().unwrap(), before, "{cmd} rolled back");
    }
    assert_eq!(s.ds().unwrap().history.undo.len(), undo);
    // Deleting the whole instance is fine.
    ex(&mut s, "edit.delete", json!({"ids": [i]}));
    healthy(&s);
}

/// Icon component (one "Shape" rectangle) and a Button component holding an Icon instance.
pub(crate) fn button(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId) {
    let f = idv(&ex(s, "frame.create", json!({"x": 0, "y": 300, "width": 24, "height": 24, "name": "Icon"})));
    let shape = idv(&ex(
        s,
        "shape.rectangle",
        json!({"x": 2, "y": 2, "width": 20, "height": 20, "parent": f, "props": {"name": "Shape", "fill": "#111111"}}),
    ));
    let icon = idv(&ex(s, "component.create", json!({"ids": [f]})));
    let b = idv(&ex(s, "frame.create", json!({"x": 0, "y": 400, "width": 120, "height": 40, "name": "Button"})));
    let nested = idv(&ex(s, "instance.create", json!({"component": icon, "x": 8, "y": 8, "parent": b})));
    ex(s, "text.create", json!({"text": "Go", "x": 40, "y": 10, "parent": b, "props": {"name": "Label"}}));
    let button = idv(&ex(s, "component.create", json!({"ids": [b]})));
    (icon, shape, button, nested)
}

#[test]
fn nested_instances_and_swaps() {
    let mut s = Session::new();
    let (icon, shape, button, nested) = button(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    healthy(&s);
    // The button instance's icon copy mirrors the main's nested instance.
    let inested = copy_of(&s, i, nested);
    let nested_shape = copy_of(&s, nested, shape);
    let ishape = copy_of(&s, i, nested_shape);
    assert!(matches!(node(&s, inested).kind, NodeKind::Instance { component, .. } if component == icon));
    // Override inside the nested instance, then edit the Icon main.
    ex(&mut s, "node.set", json!({"id": ishape, "props": {"fill": "#ff00ff"}}));
    ex(&mut s, "node.set", json!({"id": shape, "props": {"fill": "#222222", "cornerRadius": 6}}));
    healthy(&s);
    assert_eq!(fill(&s, nested_shape), "222222", "main's nested instance follows the icon");
    assert_eq!(fill(&s, ishape), "FF00FF", "override inside the nested copy kept");
    assert_eq!(node(&s, ishape).corners.radii, [6.0; 4], "icon edit reaches the button instance");
    // Main-level override on the nested instance reaches instances.
    ex(&mut s, "node.set", json!({"id": nested_shape, "props": {"strokeWeight": 3, "stroke": "#00ff00"}}));
    assert!(node(&s, nested_shape).overrides.contains("strokes"));
    assert_eq!(node(&s, ishape).stroke.weight, 3.0);
    // Swap the nested instance through an instance-swap property.
    let f2 = idv(&ex(&mut s, "frame.create", json!({"x": 100, "y": 300, "width": 24, "height": 24, "name": "Star icon"})));
    ex(&mut s, "shape.star", json!({"x": 0, "y": 0, "width": 24, "height": 24, "parent": f2, "props": {"name": "Glyph"}}));
    ex(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 4, "height": 4, "parent": f2, "props": {"name": "Spark"}}));
    let icon2 = idv(&ex(&mut s, "component.create", json!({"ids": [f2]})));
    ex(&mut s, "component.addProperty", json!({"id": button, "name": "Icon", "type": "instanceSwap", "default": icon, "layer": nested}));
    healthy(&s);
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Icon": icon2}}));
    healthy(&s);
    let inested = copy_of(&s, i, nested);
    assert!(matches!(node(&s, inested).kind, NodeKind::Instance { component, .. } if component == icon2));
    assert_eq!(kids(&s, inested).len(), 2, "swapped nested instance shows the new main's layers");
    // Main edits keep the swap; the swapped icon follows its own main.
    ex(&mut s, "node.set", json!({"id": button, "props": {"fill": "#0000ff"}}));
    ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 4, "height": 4, "parent": icon2}));
    healthy(&s);
    let inested = copy_of(&s, i, nested);
    assert!(matches!(node(&s, inested).kind, NodeKind::Instance { component, .. } if component == icon2));
    assert_eq!(kids(&s, inested).len(), 3);
    // Swapping directly (instance.swap on the nested copy) is an override that survives main edits.
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Icon": icon}}));
    let inested = copy_of(&s, i, nested);
    ex(&mut s, "instance.swap", json!({"ids": [inested], "component": icon2}));
    assert!(node(&s, inested).overrides.contains("component"));
    ex(&mut s, "node.set", json!({"id": button, "props": {"fill": "#00ff00"}}));
    healthy(&s);
    assert!(matches!(node(&s, inested).kind, NodeKind::Instance { component, .. } if component == icon2));
    ex(&mut s, "component.resetOverride", json!({"id": inested, "group": "component"}));
    assert!(matches!(node(&s, inested).kind, NodeKind::Instance { component, .. } if component == icon));
    healthy(&s);
    // A nested instance can't be swapped into a component that contains it.
    assert!(s.execute("instance.swap", &json!({"ids": [nested], "component": button})).is_err());
}

#[test]
fn variant_switch_keeps_compatible_overrides() {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for (k, (state, color)) in [("Default", "#ff0000"), ("Hover", "#0000ff")].iter().enumerate() {
        let f =
            idv(&ex(&mut s, "frame.create", json!({"x": k as f64 * 150.0, "y": 0, "width": 100, "height": 40, "name": format!("State={state}")})));
        ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 40, "parent": f, "props": {"name": "Bg", "fill": color}}));
        ex(&mut s, "text.create", json!({"text": *state, "x": 10, "y": 10, "parent": f, "props": {"name": "Label"}}));
        variants.push(idv(&ex(&mut s, "component.create", json!({"ids": [f]}))));
    }
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": variants.clone()})));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 0, "y": 200})));
    assert_eq!(node(&s, i).name, node(&s, set).name, "instances of variants are named after the set");
    let label = kids(&s, i)[1];
    ex(&mut s, "text.set", json!({"id": label, "characters": "Buy"}));
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"State": "Hover"}}));
    healthy(&s);
    assert!(matches!(node(&s, i).kind, NodeKind::Instance { component, .. } if component == variants[1]));
    let k = kids(&s, i);
    assert_eq!(fill(&s, k[0]), "0000FF");
    assert_eq!(chars(&s, k[1]), "Buy", "text override survives the variant switch");
    assert!(s.execute("instance.setProperties", &json!({"ids": [i], "properties": {"State": "Pressed"}})).is_err());
    assert!(s.execute("instance.setProperties", &json!({"ids": [i], "properties": {"Nope": 1}})).is_err());
    // Adding a variant value names it Prop=Value and grows the set.
    let h0 = node(&s, set).height;
    let v3 = idv(&ex(&mut s, "component.addVariant", json!({"id": set, "property": "State", "value": "Pressed"})));
    assert_eq!(node(&s, v3).name, "State=Pressed");
    assert!(node(&s, set).height > h0);
    assert!(s.execute("component.addVariant", &json!({"id": set, "property": "State", "value": "Pressed"})).is_err());
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"State": "Pressed"}}));
    assert!(matches!(node(&s, i).kind, NodeKind::Instance { component, .. } if component == v3));
    // A second variant property, renamed, then a value renamed.
    ex(&mut s, "component.addProperty", json!({"id": set, "name": "Size", "type": "variant", "default": "Large"}));
    assert_eq!(node(&s, variants[0]).name, "State=Default, Size=Large");
    ex(&mut s, "component.editProperty", json!({"id": set, "name": "Size", "newName": "Scale"}));
    ex(&mut s, "component.editProperty", json!({"id": set, "name": "State", "value": "Hover", "newValue": "Over"}));
    assert_eq!(node(&s, variants[1]).name, "State=Over, Scale=Large");
    ex(&mut s, "component.deleteProperty", json!({"id": set, "name": "Scale"}));
    assert_eq!(node(&s, variants[1]).name, "State=Over");
    healthy(&s);
}

#[test]
fn detach_keeps_the_look() {
    let mut s = Session::new();
    let (icon, shape, button, nested) = button(&mut s);
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    let inested = copy_of(&s, i, nested);
    let ishape = kids(&s, inested)[0];
    let ilabel = kids(&s, i)[1];
    ex(&mut s, "node.set", json!({"id": ishape, "props": {"fill": "#abcdef"}}));
    ex(&mut s, "text.set", json!({"id": ilabel, "characters": "Detached"}));
    ex(&mut s, "node.set", json!({"id": i, "props": {"fill": "#fedcba"}}));
    // A nested instance alone can't be detached.
    assert!(s.execute("component.detach", &json!({"ids": [inested]})).is_err());
    ex(&mut s, "component.detach", json!({"ids": [i]}));
    healthy(&s);
    assert_eq!(node(&s, i).kind.type_name(), "FRAME");
    assert_eq!(fill(&s, i), "FEDCBA");
    assert_eq!(chars(&s, ilabel), "Detached");
    assert!(node(&s, ilabel).source.is_none());
    // The nested instance stays an instance of Icon, re-linked to it with its override kept.
    assert!(matches!(node(&s, inested).kind, NodeKind::Instance { component, .. } if component == icon));
    assert_eq!(node(&s, ishape).source, Some(shape));
    assert_eq!(fill(&s, ishape), "ABCDEF");
    ex(&mut s, "node.set", json!({"id": shape, "props": {"cornerRadius": 4, "fill": "#000000"}}));
    assert_eq!(fill(&s, ishape), "ABCDEF");
    assert_eq!(node(&s, ishape).corners.radii, [4.0; 4]);
    // Detached layers are ordinary layers now.
    ex(&mut s, "edit.delete", json!({"ids": [ilabel]}));
    healthy(&s);
}

#[test]
fn deleted_main_keeps_instances_and_restores() {
    let mut s = Session::new();
    let (c, bg, title, _dot) = card(&mut s);
    let a = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    let b = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 600, "y": 0})));
    let atitle = copy_of(&s, a, title);
    ex(&mut s, "text.set", json!({"id": atitle, "characters": "Instance A"}));
    assert!(s.execute("component.restoreMain", &json!({"id": a})).is_err(), "main still exists");
    ex(&mut s, "edit.delete", json!({"ids": [c]}));
    healthy(&s);
    assert_eq!(kids(&s, a).len(), 3, "instances keep working without their main");
    assert!(s.execute("component.goToMain", &json!({"id": a})).is_err());
    // Editing an orphan instance still works.
    let bbg = copy_of(&s, b, bg);
    ex(&mut s, "node.set", json!({"id": bbg, "props": {"fill": "#333333"}}));
    // Restore from B: the main comes back with its old id in B's place.
    let bx = s.doc().unwrap().world_bounds(b).unwrap();
    let r = idv(&ex(&mut s, "component.restoreMain", json!({"id": b})));
    assert_eq!(r, c);
    assert!(!s.doc().unwrap().contains(b));
    assert_eq!(s.doc().unwrap().world_bounds(c).unwrap(), bx);
    assert!(matches!(node(&s, c).kind, NodeKind::Component { .. }));
    healthy(&s);
    // A re-links: main edits reach it, its override stays.
    assert_eq!(node(&s, atitle).source, Some(title));
    ex(&mut s, "node.set", json!({"id": bg, "props": {"cornerRadius": 9}}));
    assert_eq!(node(&s, copy_of(&s, a, bg)).corners.radii, [9.0; 4]);
    assert_eq!(fill(&s, copy_of(&s, a, bg)), "333333", "the restored main has B's look");
    assert_eq!(chars(&s, atitle), "Instance A");
    healthy(&s);
}

#[test]
fn resized_instances_follow_constraints_and_auto_layout() {
    let mut s = Session::new();
    let (c, bg, _title, dot) = card(&mut s);
    ex(&mut s, "node.set", json!({"id": dot, "props": {"constraints": {"horizontal": "right"}}}));
    ex(&mut s, "node.set", json!({"id": bg, "props": {"constraints": {"horizontal": "leftRight", "vertical": "topBottom"}}}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    ex(&mut s, "node.set", json!({"id": i, "props": {"width": 300, "height": 150}}));
    let (ibg, idot) = (copy_of(&s, i, bg), copy_of(&s, i, dot));
    assert_eq!(node(&s, idot).transform.as_coeffs()[4], 270.0);
    assert_eq!((node(&s, ibg).width, node(&s, ibg).height), (300.0, 150.0));
    assert!(node(&s, idot).overrides.is_empty(), "constraint moves aren't overrides");
    assert!(node(&s, i).overrides.contains("size"));
    // A main edit keeps the instance's size and the constraint layout.
    ex(&mut s, "node.set", json!({"id": dot, "props": {"fill": "#00ff00"}}));
    assert_eq!(node(&s, i).width, 300.0);
    assert_eq!(node(&s, idot).transform.as_coeffs()[4], 270.0);
    assert_eq!(node(&s, ibg).width, 300.0);
    // Main resized: instances without a size override follow it.
    let j = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 700, "y": 0})));
    ex(&mut s, "node.set", json!({"id": c, "props": {"width": 220}}));
    assert_eq!(node(&s, j).width, 220.0);
    assert_eq!(node(&s, i).width, 300.0);
    // Auto layout: a longer text override makes a hugging instance grow.
    let row = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 600, "width": 50, "height": 30, "name": "Chip"})));
    let t = idv(&ex(&mut s, "text.create", json!({"text": "Hi", "x": 0, "y": 0, "parent": row})));
    ex(&mut s, "layout.add", json!({"ids": [row]}));
    let chip = idv(&ex(&mut s, "component.create", json!({"ids": [row]})));
    let ci = idv(&ex(&mut s, "instance.create", json!({"component": chip, "x": 300, "y": 600})));
    let w0 = node(&s, ci).width;
    let it = copy_of(&s, ci, t);
    let tw0 = node(&s, it).width;
    ex(&mut s, "text.set", json!({"id": it, "characters": "A much longer chip label"}));
    // Without a measurable font (CI has no craft-fonts) the text keeps its size; only check the
    // hug when the text actually grew.
    if node(&s, it).width > tw0 {
        assert!(node(&s, ci).width > w0, "auto layout instance hugs its overridden text");
    }
    healthy(&s);
}

#[test]
fn component_properties_apply_to_instances() {
    let mut s = Session::new();
    let (c, _bg, title, dot) = card(&mut s);
    // Boolean bound to the dot's visibility (created from the selected layer).
    ex(&mut s, "component.addProperty", json!({"id": dot, "name": "Show dot", "type": "boolean", "default": true}));
    assert_eq!(node(&s, dot).prop_refs.get("visible").map(String::as_str), Some("Show dot"));
    ex(&mut s, "component.addProperty", json!({"id": c, "name": "Title", "type": "text", "layer": title}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    let (ititle, idot) = (copy_of(&s, i, title), copy_of(&s, i, dot));
    assert_eq!(chars(&s, ititle), "Hello", "text property defaults to the layer's text");
    ex(&mut s, "instance.setProperties", json!({"ids": [i], "properties": {"Show dot": false, "Title": "From prop"}}));
    assert!(!node(&s, idot).visible);
    assert_eq!(chars(&s, ititle), "From prop");
    assert!(node(&s, idot).overrides.is_empty(), "property values aren't overrides");
    // Main edits keep the property values.
    ex(&mut s, "node.set", json!({"id": dot, "props": {"fill": "#00ff00"}}));
    assert!(!node(&s, idot).visible);
    assert_eq!(chars(&s, ititle), "From prop");
    // Rename the property: bindings and instance values follow.
    ex(&mut s, "component.editProperty", json!({"id": c, "name": "Title", "newName": "Heading", "default": "Default heading"}));
    assert_eq!(node(&s, title).prop_refs.get("characters").map(String::as_str), Some("Heading"));
    assert!(matches!(&node(&s, i).kind, NodeKind::Instance { props, .. } if props.get("Heading") == Some(&json!("From prop"))));
    assert_eq!(chars(&s, ititle), "From prop");
    let j = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 600, "y": 0})));
    assert_eq!(chars(&s, copy_of(&s, j, title)), "Default heading");
    // Wrong types and names are refused.
    assert!(s.execute("instance.setProperties", &json!({"ids": [i], "properties": {"Show dot": "maybe"}})).is_err());
    assert!(s.execute("component.bindProperty", &json!({"layer": dot, "property": "Heading"})).is_err());
    assert!(s.execute("component.bindProperty", &json!({"layer": ititle, "property": "Heading"})).is_err());
    assert!(s.execute("component.addProperty", &json!({"id": c, "name": "Heading", "type": "text"})).is_err());
    // Unbind and delete.
    ex(&mut s, "component.bindProperty", json!({"layer": dot, "property": null, "field": "visible"}));
    assert!(node(&s, copy_of(&s, i, dot)).visible);
    ex(&mut s, "component.deleteProperty", json!({"id": c, "name": "Heading"}));
    assert!(node(&s, title).prop_refs.is_empty());
    assert!(matches!(&node(&s, i).kind, NodeKind::Instance { props, .. } if !props.contains_key("Heading")));
    let desc = ex(&mut s, "component.properties", json!({"id": i}));
    assert_eq!(desc["properties"].as_array().unwrap().len(), 1);
    // A variant property on a lone component makes it a component set.
    let set = idv(&ex(&mut s, "component.addProperty", json!({"id": c, "name": "Theme", "type": "variant", "default": "Light"})));
    assert!(matches!(node(&s, set).kind, NodeKind::ComponentSet { .. }));
    assert_eq!(node(&s, c).name, "Theme=Light");
    healthy(&s);
}

#[test]
fn push_changes_to_main() {
    let mut s = Session::new();
    let (c, bg, _title, _dot) = card(&mut s);
    let a = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 300, "y": 0})));
    let b = idv(&ex(&mut s, "instance.create", json!({"component": c, "x": 600, "y": 0})));
    let abg = copy_of(&s, a, bg);
    ex(&mut s, "node.set", json!({"id": abg, "props": {"fill": "#445566"}}));
    ex(&mut s, "component.pushChanges", json!({"id": a}));
    assert_eq!(fill(&s, bg), "445566");
    assert_eq!(fill(&s, copy_of(&s, b, bg)), "445566");
    assert!(node(&s, copy_of(&s, a, bg)).overrides.is_empty());
    healthy(&s);
}

/// Random edits on mains and instances keep the document valid and instances mirroring mains.
#[test]
fn random_edit_sequences_keep_instances_consistent() {
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
    for seed in 1..=std::env::var("FC_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(24u64) {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut s = Session::new();
        let (icon, _shape, button, _nested) = button(&mut s);
        let (card_c, ..) = card(&mut s);
        let comps = [icon, button, card_c];
        for c in comps {
            for k in 0..2 {
                ex(&mut s, "instance.create", json!({"component": c, "x": 500 + k * 200, "y": c.0 * 10}));
            }
        }
        for step in 0..60 {
            let doc = s.doc().unwrap().clone();
            let main_layers: Vec<NodeId> = comps.iter().flat_map(|c| doc.subtree(*c)).filter(|i| doc.contains(*i)).collect();
            let inst_layers: Vec<NodeId> =
                doc.nodes().filter(|n| n.source.is_some() && owning_instance(&doc, n.id).is_some()).map(|n| n.id).collect();
            let roots: Vec<NodeId> =
                doc.nodes().filter(|n| matches!(n.kind, NodeKind::Instance { .. }) && !inside_instance(&doc, n.id)).map(|n| n.id).collect();
            let containers: Vec<NodeId> = main_layers
                .iter()
                .copied()
                .filter(|i| {
                    doc.node(*i).is_some_and(|n| n.kind.is_container())
                        && !inside_instance(&doc, *i)
                        && !matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Instance { .. }))
                })
                .collect();
            let color = format!("#{:06x}", r.next() % 0xFF_FFFF);
            let (cmd, p) = match r.next() % 18 {
                14 => ("text.set", json!({"id": r.pick(&inst_layers), "characters": format!("T{}", r.next() % 9)})),
                15 => ("component.detach", json!({"ids": [r.pick(&roots)]})),
                16 => ("component.pushChanges", json!({"id": r.pick(&roots)})),
                17 => ("node.set", json!({"id": r.pick(&inst_layers), "props": {"name": format!("I{}", r.next() % 3)}})),
                0 => (
                    "shape.rectangle",
                    json!({"x": (r.next() % 50) as f64, "y": 5, "width": 8, "height": 8, "parent": r.pick(&containers), "props": {"name": format!("R{step}")}}),
                ),
                1 => ("edit.delete", json!({"ids": [r.pick(&main_layers).filter(|i| !comps.contains(i))]})),
                2 => ("node.reorder", json!({"id": r.pick(&main_layers), "index": r.next() % 3})),
                3 => ("node.set", json!({"id": r.pick(&main_layers), "props": {"fill": color}})),
                4 => ("node.set", json!({"id": r.pick(&main_layers), "props": {"name": format!("N{}", r.next() % 4)}})),
                5 => ("node.set", json!({"id": r.pick(&inst_layers), "props": {"fill": color}})),
                6 => ("node.set", json!({"id": r.pick(&inst_layers), "props": {"visible": r.next().is_multiple_of(2)}})),
                7 => ("node.set", json!({"id": r.pick(&inst_layers), "props": {"width": 5 + r.next() % 40}})),
                8 => ("node.set", json!({"id": r.pick(&roots), "props": {"width": 20 + r.next() % 200, "height": 20 + r.next() % 100}})),
                9 => ("edit.delete", json!({"ids": [r.pick(&inst_layers)]})),
                10 => ("component.resetInstance", json!({"ids": [r.pick(&roots)]})),
                11 => ("component.resetOverride", json!({"id": r.pick(&inst_layers)})),
                12 => (
                    "instance.swap",
                    json!({"ids": [r.pick(&inst_layers).filter(|i| matches!(doc.node(*i).map(|n| &n.kind), Some(NodeKind::Instance { .. })))], "component": r.pick(&[icon, card_c])}),
                ),
                _ => ("node.set", json!({"id": r.pick(&main_layers), "props": {"width": 5 + r.next() % 120}})),
            };
            if std::env::var("FC_DUMP").ok().as_deref() == Some(&format!("{seed}:{step}")) {
                for n in s.doc().unwrap().nodes() {
                    eprintln!(
                        "{} {:?} {} parent={:?} src={:?} kids={:?} ov={:?}",
                        n.id,
                        n.name,
                        match &n.kind {
                            NodeKind::Instance { component, .. } => format!("INST({component})"),
                            k => k.type_name().into(),
                        },
                        n.parent,
                        n.source,
                        n.children,
                        n.overrides
                    );
                }
            }
            let res = s.execute(cmd, &p);
            if let Err(EngineError::Internal { cmd, msg }) = &res {
                panic!("seed {seed} step {step}: {cmd} panicked: {msg}");
            }
            let doc = s.doc().unwrap();
            assert!(doc.validate().is_empty(), "seed {seed} step {step} {cmd} {p}: {:?}", doc.validate());
            assert!(mirror_issues(doc).is_empty(), "seed {seed} step {step} {cmd} {p} -> {res:?}: {:?}", mirror_issues(doc));
        }
        // Undo everything: still valid at every step.
        while s.undo().unwrap() {
            assert!(s.doc().unwrap().validate().is_empty());
        }
    }
}

#[test]
fn place_instances_into_frames_and_auto_layout_slots() {
    let mut s = Session::new();
    let c = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 40, "height": 20, "name": "Chip"})));
    let c = idv(&ex(&mut s, "component.create", json!({"ids": [c]})));
    // A component made from one shape is named after it.
    let star = idv(&ex(&mut s, "shape.star", json!({"x": 0, "y": 200, "width": 24, "height": 24, "name": "Icon / Star"})));
    let sc = idv(&ex(&mut s, "component.create", json!({"ids": [star]})));
    assert_eq!(node(&s, sc).name, "Icon / Star");
    // On empty canvas: on the page, centred on the point.
    let a = idv(&ex(&mut s, "instance.place", json!({"component": c, "x": 500, "y": 500})));
    let b = s.doc().unwrap().world_bounds(a).unwrap();
    assert_eq!((b.x0, b.y0), (480.0, 490.0));
    assert_eq!(node(&s, a).parent, s.ds().unwrap().doc.first_page());
    // Into a plain frame under the point (frame-relative position).
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 1000, "y": 0, "width": 300, "height": 300})));
    let r = ex(&mut s, "instance.place", json!({"component": c, "x": 1100, "y": 100}));
    let i = idv(&r);
    assert_eq!(node(&s, i).parent, Some(f));
    assert_eq!(s.doc().unwrap().world_bounds(i).unwrap().x0, 1080.0);
    // Into a horizontal auto layout row between its two children.
    let row = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 400, "width": 300, "height": 40})));
    let k1 = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 20, "parent": row})));
    let k2 = idv(&ex(&mut s, "shape.rectangle", json!({"x": 100, "y": 0, "width": 50, "height": 20, "parent": row})));
    ex(&mut s, "layout.add", json!({"ids": [row]}));
    let b1 = s.doc().unwrap().world_bounds(k1).unwrap();
    let b2 = s.doc().unwrap().world_bounds(k2).unwrap();
    let mid = (b1.x1 + b2.x0) / 2.0;
    let j = idv(&ex(&mut s, "instance.place", json!({"component": c, "x": mid, "y": b1.center().y})));
    assert_eq!(kids(&s, row), vec![k1, j, k2], "inserted between");
    // One undo step.
    s.undo().unwrap();
    assert_eq!(kids(&s, row), vec![k1, k2]);
    // Never inside its own main component: lands on the page instead.
    let own = s.doc().unwrap().world_bounds(c).unwrap().center();
    let k = idv(&ex(&mut s, "instance.place", json!({"component": c, "x": own.x, "y": own.y})));
    assert_eq!(node(&s, k).parent, s.ds().unwrap().doc.first_page());
    // Junk.
    for j in [json!({}), json!({"component": c}), json!({"component": 99999, "x": 0, "y": 0}), json!({"component": c, "x": "NaN", "y": 1e308})] {
        assert!(s.execute("instance.place", &j).is_err(), "{j}");
    }
    healthy(&s);
}

#[test]
fn exposed_nested_instances_show_their_properties_on_instances() {
    let mut s = Session::new();
    let (icon, shape, button, nested) = button(&mut s);
    ex(&mut s, "component.addProperty", json!({"id": icon, "name": "Filled", "type": "boolean", "default": true, "layer": shape}));
    let i = idv(&ex(&mut s, "instance.create", json!({"component": button, "x": 300, "y": 400})));
    let exposed = |s: &mut Session| ex(s, "instance.exposed", json!({"id": i}))["instances"].as_array().unwrap().len();
    assert_eq!(exposed(&mut s), 0);
    ex(&mut s, "component.exposeInstance", json!({"id": nested}));
    assert!(node(&s, nested).exposed);
    healthy(&s);
    let inested = copy_of(&s, i, nested);
    let list = ex(&mut s, "instance.exposed", json!({"id": i}));
    assert_eq!(list["instances"][0]["id"], json!(inested.0));
    // The nested copy's property is set from the outer instance and survives main edits.
    ex(&mut s, "instance.setProperties", json!({"ids": [inested], "properties": {"Filled": false}}));
    let ishape = copy_of(&s, inested, copy_of(&s, nested, shape));
    assert!(!node(&s, ishape).visible);
    ex(&mut s, "node.set", json!({"id": button, "props": {"fill": "#00ff00"}}));
    let inested = copy_of(&s, i, nested);
    let ishape = copy_of(&s, inested, copy_of(&s, nested, shape));
    assert!(!node(&s, ishape).visible, "kept across main edits");
    healthy(&s);
    // Un-exposing hides it again; one undo step each.
    ex(&mut s, "component.exposeInstance", json!({"id": nested, "exposed": false}));
    assert_eq!(exposed(&mut s), 0);
    s.undo().unwrap();
    assert_eq!(exposed(&mut s), 1);
    // Only nested instances inside a main can be exposed.
    assert!(s.execute("component.exposeInstance", &json!({"id": i})).is_err());
    assert!(s.execute("component.exposeInstance", &json!({"id": inested})).is_err());
    assert!(s.execute("component.exposeInstance", &json!({"id": shape})).is_err());
}
