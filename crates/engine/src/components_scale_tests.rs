//! Components at scale: a synthetic design system (every property kind, nesting four deep, styles
//! and light/dark variables) and a file using it with overrides at every nesting level. Edits of
//! deep mains, variant switches, swaps, resets, detach, push, library updates, copy/paste and
//! undo/redo keep every override and every instance mirroring its main.

use std::collections::BTreeMap;

use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
use crate::components::*;
use crate::synthetic_ds::*;

/// Instances in the test file (debug builds: the bench uses 2,000 with the full design system).
const COUNT: usize = 120;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    match s.execute(cmd, &p) {
        Ok(v) => v,
        Err(e) => panic!("{cmd} {p}: {e}"),
    }
}

fn session() -> (Session, DesignSystem, Vec<NodeId>) {
    // Other tests scan the system fonts; text measured before and after that scan would differ.
    figcraft_text::scan_system_fonts();
    let (doc, ds, inst) = design_system_with_instances(&DsSpec::SMALL, COUNT).unwrap();
    (Session::with_document(doc), ds, inst)
}

fn healthy(doc: &Document) {
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());
    let issues = mirror_issues(doc);
    assert!(issues.is_empty(), "{} mirror issues, first: {:?}", issues.len(), issues.first());
}

/// Every overridden layer inside an instance: its groups and the values they hold.
fn overrides(doc: &Document) -> BTreeMap<NodeId, (Vec<String>, String)> {
    doc.nodes()
        .filter(|n| !n.overrides.is_empty() && owning_instance(doc, n.id).is_some())
        .map(|n| {
            let mut v = Vec::new();
            for g in &n.overrides {
                v.push(match g.as_str() {
                    "fills" => format!("{:?}", n.fills),
                    "name" => n.name.clone(),
                    "characters" => n.kind.text().map(|t| t.characters.clone()).unwrap_or_default(),
                    "component" | "props" => format!("{:?}", n.kind),
                    other => other.to_string(),
                });
            }
            (n.id, (n.overrides.iter().cloned().collect(), v.join("|")))
        })
        .collect()
}

fn chars(doc: &Document, id: NodeId) -> String {
    doc.node(id).and_then(|n| n.kind.text()).map(|t| t.characters.clone()).unwrap_or_default()
}

fn comp_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    match doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Instance { component, .. }) => Some(*component),
        _ => None,
    }
}

fn main_of(doc: &Document, id: NodeId) -> NodeId {
    comp_of(doc, id).unwrap_or_else(|| panic!("{id} is not an instance"))
}

#[test]
fn the_generated_file_covers_every_property_kind_and_nesting_level() {
    let (s, ds, inst) = session();
    let doc = s.doc().unwrap();
    healthy(doc);
    assert_eq!(inst.len(), COUNT);
    assert!(ds.components >= 40, "{}", ds.components);
    let full = design_system_document(&DsSpec::FULL).unwrap().1;
    assert!((280..=330).contains(&full.components), "the full design system has {} components", full.components);
    // Property kinds on the button set: variant, text, boolean, instance swap (with preferred values).
    let props = ex(&mut Session::with_document(doc.clone()), "component.properties", json!({"id": ds.button}));
    let kinds: Vec<&str> = props["properties"].as_array().unwrap().iter().filter_map(|p| p["type"].as_str()).collect();
    for k in ["variant", "text", "boolean", "instanceSwap"] {
        assert!(kinds.contains(&k), "{kinds:?}");
    }
    let swap = props["properties"].as_array().unwrap().iter().find(|p| p["type"] == "instanceSwap").unwrap();
    assert_eq!(swap["preferred"].as_array().unwrap().len(), 4);
    // Slots on cards.
    let card = doc.node(ds.card).unwrap().children[0];
    assert_eq!(slots_of(doc, card).len(), 1);
    // Four levels deep: nav > profile card > footer button > icon > glyph.
    let nav = inst.iter().copied().find(|i| variant_set(doc, main_of(doc, *i)) == Some(ds.nav)).unwrap();
    let glyph = find_path(doc, nav, &["Profile", "Footer button", "Icon", "Glyph"]).unwrap();
    assert_eq!(scope_depth(doc, glyph), 4);
    // Overrides at every level, styles and variables in use.
    let o = overrides(doc);
    for g in ["fills", "characters", "component", "props", "name"] {
        assert!(o.values().any(|(gs, _)| gs.iter().any(|x| x == g)), "no {g} override");
    }
    assert!(doc.nodes().filter(|n| !n.bound.is_empty()).count() > 100);
    assert!(doc.nodes().any(|n| n.styles.text.is_some()));
    assert!(doc.nodes().any(|n| n.var_modes.values().any(|m| *m == ds.dark)));
}

/// Instances `id` is nested in (0 for a top-level instance's own layers).
fn scope_depth(doc: &Document, id: NodeId) -> usize {
    doc.ancestors(id).into_iter().filter(|a| matches!(doc.node(*a).map(|n| &n.kind), Some(NodeKind::Instance { .. }))).count()
}

/// Two documents hold the same layers with the same values (the first difference otherwise).
/// Layers created by the edit may have other ids on each side: matched by their place in the
/// tree. Where a hidden layer sits inside an auto layout frame is nobody's concern: not compared.
fn assert_same(a: &Document, b: &Document, what: &str) {
    assert_eq!(a.node_count(), b.node_count(), "{what}: layer counts differ");
    // b's ids → a's, pairing layers by position from the roots down.
    let mut map: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut stack: Vec<(NodeId, NodeId)> = a.nodes().filter(|n| n.parent.is_none()).map(|n| (n.id, n.id)).collect();
    while let Some((x, y)) = stack.pop() {
        map.insert(y, x);
        let (Some(xn), Some(yn)) = (a.node(x), b.node(y)) else { panic!("{what}: layer {x} is missing on one side") };
        assert_eq!(xn.children.len(), yn.children.len(), "{what}: layer {x} has other layers");
        stack.extend(xn.children.iter().copied().zip(yn.children.iter().copied()));
    }
    let hidden = |d: &Document, id: NodeId| std::iter::once(id).chain(d.ancestors(id)).any(|x| d.node(x).is_some_and(|n| !n.visible));
    for (yid, id) in map.clone() {
        let (Some(x), Some(y)) = (a.node(id), b.node(yid)) else { panic!("{what}: layer {id} is missing on one side") };
        let m = |i: NodeId| map.get(&i).copied().unwrap_or(i);
        let mut y = y.clone();
        y.id = m(y.id);
        y.parent = y.parent.map(m);
        y.source = y.source.map(m);
        y.children = y.children.iter().map(|c| m(*c)).collect();
        if hidden(a, id) && hidden(b, yid) {
            y.transform = x.transform;
        }
        let (x, y) = (Some(x), Some(&y));
        assert!(x == y, "{what}: layer {id} differs:\n{x:?}\n{y:?}");
    }
}

/// The first top-level instance of set `set` with a layer at name path `path`: (instance, layer).
fn first_of(doc: &Document, inst: &[NodeId], set: NodeId, path: &[&str]) -> (NodeId, NodeId) {
    inst.iter()
        .copied()
        .filter(|i| variant_set(doc, main_of(doc, *i)) == Some(set))
        .find_map(|i| find_path(doc, i, path).map(|l| (i, l)))
        .unwrap_or_else(|| panic!("no instance of {set} has {path:?}"))
}

fn rgb_of(doc: &Document, var: &str, mode: &str) -> String {
    match doc.variables.get(var).and_then(|v| v.values.get(mode)) {
        Some(VariableValue::Color(c)) => c.to_hex(),
        other => panic!("{var}: {other:?}"),
    }
}

/// The edits of deep mains the propagation tests run (each one undoable command).
fn main_edits(doc: &Document, ds: &DesignSystem) -> Vec<(&'static str, Value)> {
    let glyph = doc.node(ds.icons[0]).unwrap().children[0];
    let button = doc.node(ds.button).unwrap().children[0];
    let label = find_path(doc, button, &["Label"]).unwrap();
    let card = doc.node(ds.card).unwrap().children[0];
    let title = find_path(doc, card, &["Title"]).unwrap();
    let body = find_path(doc, card, &["Body"]).unwrap();
    let nav = doc.node(ds.nav).unwrap().children[0];
    vec![
        ("node.set", json!({"ids": [glyph], "props": {"boundVariables": {"fill": null}, "fill": "#ff0000"}})),
        ("node.set", json!({"ids": [glyph], "props": {"cornerRadius": 3, "opacity": 0.8}})),
        ("node.set", json!({"ids": [glyph], "props": {"width": 18, "height": 18}})),
        ("node.set", json!({"ids": [glyph], "props": {"x": 2, "y": 3}})),
        ("node.set", json!({"ids": [label], "props": {"fontSize": 17}})),
        ("node.set", json!({"ids": [label], "props": {"visible": false}})),
        ("node.set", json!({"ids": [title], "props": {"name": "Heading"}})),
        ("arrange.back", json!({"ids": [body]})),
        ("edit.delete", json!({"ids": [body]})),
        ("node.set", json!({"ids": [card], "props": {"strokeWeight": 3}})),
        ("node.set", json!({"ids": [nav], "props": {"effects": [{"type": "dropShadow"}]}})),
        ("shape.rectangle", json!({"x": 0, "y": 0, "width": 4, "height": 4, "parent": ds.icons[0], "props": {"name": "Badge dot"}})),
        ("node.set", json!({"ids": [ds.icons[0]], "props": {"width": 30}})),
    ]
}

#[test]
fn editing_deep_mains_keeps_every_override_and_matches_a_full_rebuild() {
    let (mut a, ds, _) = session();
    let mut b = Session::with_document(a.doc().unwrap().clone());
    let edits = main_edits(a.doc().unwrap(), &ds);
    let start = overrides(a.doc().unwrap());
    let mut patched = Vec::new();
    for (cmd, p) in &edits {
        let was = PATCHED.with(std::cell::Cell::get);
        ex(&mut a, cmd, p.clone());
        patched.push(PATCHED.with(std::cell::Cell::get) > was);
        FULL_REBUILD.with(|f| f.set(true));
        ex(&mut b, cmd, p.clone());
        FULL_REBUILD.with(|f| f.set(false));
        let doc = a.doc().unwrap();
        healthy(doc);
        assert_same(doc, b.doc().unwrap(), &format!("{cmd} {p}"));
        // Every override is still there with its value (the edits touch no overridden group).
        let now = overrides(doc);
        for (id, o) in &start {
            assert_eq!(now.get(id), Some(o), "after {cmd} {p}: override of {id}");
        }
    }
    // Every one of these edits was carried to the copies without rebuilding instances.
    let rebuilt: Vec<String> = edits.iter().zip(&patched).filter(|(_, p)| !**p).map(|((c, p), _)| format!("{c} {p}")).collect();
    assert!(rebuilt.is_empty(), "rebuilt for {rebuilt:?}");
    // The copies that don't override it followed: buttons' labels are hidden.
    let doc = a.doc().unwrap();
    let hidden = doc.nodes().filter(|n| n.name == "Label" && owning_instance(doc, n.id).is_some() && !n.visible).count();
    assert!(hidden > COUNT / 4, "{hidden} labels hidden");
    // Undo walks every edit back; redo replays them.
    for _ in &edits {
        a.undo().unwrap();
    }
    assert_eq!(overrides(a.doc().unwrap()), start);
    for _ in &edits {
        a.redo().unwrap();
    }
    assert_same(a.doc().unwrap(), b.doc().unwrap(), "after redo");
}

#[test]
fn bound_copies_follow_their_own_theme_after_main_edits() {
    let (mut s, ds, _) = session();
    let doc = s.doc().unwrap();
    let dark = rgb_of(doc, "ds:icon", &ds.dark);
    let light = rgb_of(doc, "ds:icon", &ds.light);
    // Edit something else on the icon main: copies in dark screens keep their dark colour.
    let glyph = doc.node(ds.icons[0]).unwrap().children[0];
    ex(&mut s, "node.set", json!({"ids": [glyph], "props": {"width": 15}}));
    ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 2, "height": 2, "parent": ds.icons[0]}));
    let doc = s.doc().unwrap();
    let (mut seen_dark, mut seen_light) = (0, 0);
    for n in doc.nodes().filter(|n| n.name == "Glyph" && n.overrides.is_empty() && owning_instance(doc, n.id).is_some()) {
        let is_dark = doc.modes_for(n.id).get(&ds.theme) == Some(&ds.dark);
        let want = if is_dark { &dark } else { &light };
        assert_eq!(n.fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).as_ref(), Some(want), "glyph {}", n.id);
        if is_dark {
            seen_dark += 1;
        } else {
            seen_light += 1;
        }
    }
    assert!(seen_dark > 5 && seen_light > 5, "{seen_dark} dark, {seen_light} light");
}

#[test]
fn variant_switches_keep_nested_overrides() {
    let (mut s, ds, inst) = session();
    let doc = s.doc().unwrap();
    let (card, glyph) = first_of(doc, &inst, ds.card, &["Footer button", "Icon", "Glyph"]);
    let footer = find_path(doc, card, &["Footer button"]).unwrap();
    let icon = find_path(doc, card, &["Footer button", "Icon"]).unwrap();
    let (fill, swapped, props) = (doc.node(glyph).unwrap().fills.clone(), main_of(doc, icon), doc.node(footer).unwrap().kind.clone());
    let title = chars(doc, find_path(doc, card, &["Title"]).unwrap());
    let before = overrides(doc);
    for (size, ty) in [("Medium", "Outlined"), ("Small", "Elevated"), ("Medium", "Elevated")] {
        ex(&mut s, "instance.setProperties", json!({"ids": [card], "properties": {"Size": size, "Type": ty}}));
        let doc = s.doc().unwrap();
        healthy(doc);
        assert!(doc.node(main_of(doc, card)).unwrap().name.contains(&format!("Type={ty}")));
        // Same layers, same overrides: the exposed footer button's label, its swapped icon and that
        // icon's recoloured glyph, four levels down; the title property.
        assert_eq!(find_path(doc, card, &["Footer button", "Icon", "Glyph"]), Some(glyph));
        assert_eq!(doc.node(glyph).unwrap().fills, fill);
        assert_eq!(main_of(doc, icon), swapped);
        assert_eq!(doc.node(footer).unwrap().kind, props);
        assert_eq!(chars(doc, find_path(doc, card, &["Title"]).unwrap()), title);
        let now = overrides(doc);
        for (id, o) in before.iter().filter(|(id, _)| doc.is_ancestor(card, **id)) {
            assert_eq!(now.get(id), Some(o), "override of {id}");
        }
    }
    // The exposed nested instance's properties are offered on the card.
    let exposed = ex(&mut s, "instance.exposed", json!({"id": card}));
    assert!(exposed["instances"].as_array().unwrap().iter().any(|e| e["id"] == json!(footer.0)));
    for _ in 0..3 {
        s.undo().unwrap();
    }
    assert_eq!(overrides(s.doc().unwrap()), before);
}

#[test]
fn instance_swaps_offer_preferred_values_and_keep_matching_overrides() {
    let (mut s, ds, inst) = session();
    // The button's icon property prefers the first icons; the list can be edited.
    let props = ex(&mut s, "component.properties", json!({"id": ds.button}));
    let swap = props["properties"].as_array().unwrap().iter().find(|p| p["name"] == "Icon").unwrap().clone();
    assert_eq!(swap["preferred"], json!(ds.icons.iter().take(4).map(|i| i.0).collect::<Vec<_>>()));
    ex(&mut s, "component.editProperty", json!({"id": ds.button, "name": "Icon", "preferred": [ds.icons[5].0, ds.icons[4].0]}));
    let props = ex(&mut s, "component.properties", json!({"id": ds.button}));
    let swap = props["properties"].as_array().unwrap().iter().find(|p| p["name"] == "Icon").unwrap().clone();
    assert_eq!(swap["preferred"], json!([ds.icons[5].0, ds.icons[4].0]));
    assert_eq!(swap["default"], json!(ds.icons[0].0), "the default stays");
    assert!(s.execute("component.editProperty", &json!({"id": ds.button, "name": "Label", "preferred": [ds.icons[1].0]})).is_err());
    // Swapping through the property: the recoloured glyph keeps its colour (same layer name).
    let doc = s.doc().unwrap();
    let (button, glyph) = first_of(doc, &inst, ds.button, &["Icon", "Glyph"]);
    let fill = doc.node(glyph).unwrap().fills.clone();
    ex(&mut s, "instance.setProperties", json!({"ids": [button], "properties": {"Icon": ds.icons[5].0}}));
    let doc = s.doc().unwrap();
    healthy(doc);
    let icon = find_path(doc, button, &["Icon"]).unwrap();
    assert_eq!(main_of(doc, icon), ds.icons[5]);
    assert_eq!(doc.node(find_path(doc, icon, &["Glyph"]).unwrap()).unwrap().fills, fill);
    // Swapping a top-level icon instance: same.
    let (top, tglyph) =
        inst.iter().copied().filter(|i| ds.icons.contains(&main_of(doc, *i))).find_map(|i| find_path(doc, i, &["Glyph"]).map(|g| (i, g))).unwrap();
    let tfill = doc.node(tglyph).unwrap().fills.clone();
    let other = ds.icons.iter().copied().find(|c| *c != main_of(doc, top)).unwrap();
    ex(&mut s, "instance.swap", json!({"ids": [top], "component": other}));
    let doc = s.doc().unwrap();
    healthy(doc);
    assert_eq!(main_of(doc, top), other);
    assert_eq!(doc.node(find_path(doc, top, &["Glyph"]).unwrap()).unwrap().fills, tfill);
}

#[test]
fn reset_detach_and_push_at_scale() {
    let (mut s, ds, inst) = session();
    let doc = s.doc().unwrap();
    let navs: Vec<NodeId> = inst.iter().copied().filter(|i| variant_set(doc, main_of(doc, *i)) == Some(ds.nav)).collect();
    assert!(navs.len() >= 3);
    // Reset one override four levels down, then everything on a nav.
    let glyph = find_path(doc, navs[0], &["Profile", "Footer button", "Icon", "Glyph"]).unwrap();
    assert!(doc.node(glyph).unwrap().overrides.contains("fills"));
    ex(&mut s, "component.resetOverride", json!({"id": glyph, "group": "fills"}));
    let doc = s.doc().unwrap();
    let src = doc.node(glyph).unwrap().source.unwrap();
    assert_eq!(doc.node(glyph).unwrap().fills, doc.node(src).unwrap().fills);
    ex(&mut s, "component.resetInstance", json!({"ids": [navs[0]]}));
    let doc = s.doc().unwrap();
    healthy(doc);
    let left: Vec<(NodeId, String, Vec<String>)> = doc
        .subtree(navs[0])
        .into_iter()
        .filter_map(|i| doc.node(i))
        .filter(|n| n.overrides.iter().any(|g| g != "size"))
        .map(|n| (n.id, n.name.clone(), n.overrides.iter().cloned().collect()))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    // Detach another: a frame; its nested instances keep their look and stay linked.
    let look = |doc: &Document, root: NodeId| -> Vec<(String, String, String)> {
        doc.subtree(root)
            .into_iter()
            .skip(1)
            .filter_map(|i| doc.node(i))
            .map(|n| (n.name.clone(), format!("{:?}", n.fills), n.kind.text().map(|t| t.characters.clone()).unwrap_or_default()))
            .collect()
    };
    let was = look(doc, navs[1]);
    ex(&mut s, "component.detach", json!({"ids": [navs[1]]}));
    let doc = s.doc().unwrap();
    healthy(doc);
    assert!(matches!(doc.node(navs[1]).unwrap().kind, NodeKind::Frame));
    assert_eq!(look(doc, navs[1]), was);
    let profile = find_path(doc, navs[1], &["Profile"]).unwrap();
    assert!(matches!(doc.node(profile).unwrap().kind, NodeKind::Instance { .. }));
    // Push a recoloured glyph from a button to its main: the other instances of that variant
    // showing the same icon follow unless they recolour the glyph themselves.
    let (button, bglyph) = first_of(doc, &inst, ds.button, &["Icon", "Glyph"]);
    let variant = main_of(doc, button);
    let fill = doc.node(bglyph).unwrap().fills.clone();
    let bicon = main_of(doc, find_path(doc, button, &["Icon"]).unwrap());
    let r = ex(&mut s, "component.pushChanges", json!({"id": button}));
    assert!(r["layers"].as_u64().unwrap() >= 1);
    let doc = s.doc().unwrap();
    healthy(doc);
    assert!(!doc.node(bglyph).unwrap().overrides.contains("fills"));

    for i in inst.iter().copied().filter(|i| *i != button && comp_of(doc, *i) == Some(variant)) {
        let (Some(g), Some(icon)) = (find_path(doc, i, &["Icon", "Glyph"]), find_path(doc, i, &["Icon"])) else { continue };
        let n = doc.node(g).unwrap();
        if !n.overrides.contains("fills") && main_of(doc, icon) == bicon {
            assert_eq!(n.fills, fill, "glyph {g} of {i}");
        }
    }

    for _ in 0..4 {
        s.undo().unwrap();
    }
    healthy(s.doc().unwrap());
}

#[test]
fn copy_paste_of_instances_keeps_overrides() {
    let (mut s, ds, inst) = session();
    let doc = s.doc().unwrap();
    let (nav, _) = first_of(doc, &inst, ds.nav, &["Profile"]);
    let o = overrides(doc);
    let mine: Vec<(Vec<String>, String)> = doc.subtree(nav).iter().filter_map(|i| o.get(i).cloned()).collect();
    assert!(mine.len() >= 4);
    ex(&mut s, "edit.copy", json!({"ids": [nav]}));
    let r = ex(&mut s, "edit.paste", json!({}));
    let pasted = crate::params::id_of(&r["ids"][0]).unwrap();
    let doc = s.doc().unwrap();
    healthy(doc);
    let o = overrides(doc);
    let theirs: Vec<(Vec<String>, String)> = doc.subtree(pasted).iter().filter_map(|i| o.get(i).cloned()).collect();
    assert_eq!(theirs, mine);
    assert_eq!(main_of(doc, pasted), main_of(doc, nav));
}

#[test]
fn switching_a_screen_theme_records_no_overrides() {
    let (mut s, ds, _) = session();
    let doc = s.doc().unwrap();
    let before = overrides(doc);
    let light_screen = doc
        .nodes()
        .find(|n| n.name.starts_with("Screen ") && matches!(n.kind, NodeKind::Frame) && n.var_modes.is_empty() && !n.children.is_empty())
        .map(|n| n.id)
        .unwrap();
    ex(&mut s, "variable.setMode", json!({"ids": [light_screen], "collection": ds.theme, "mode": ds.dark}));
    let doc = s.doc().unwrap();
    healthy(doc);
    let now = overrides(doc);
    let changed: Vec<_> =
        now.iter().filter(|(k, v)| before.get(*k) != Some(*v)).chain(before.iter().filter(|(k, _)| !now.contains_key(*k))).collect();
    assert!(changed.is_empty(), "a theme switch isn't an override: {changed:?}");
    let dark = rgb_of(doc, "ds:icon", &ds.dark);
    let glyphs: Vec<&Node> =
        doc.subtree(light_screen).into_iter().filter_map(|i| doc.node(i)).filter(|n| n.name == "Glyph" && n.overrides.is_empty()).collect();
    assert!(!glyphs.is_empty());
    for g in glyphs {
        assert_eq!(g.fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()), Some(dark.clone()), "glyph {}", g.id);
    }
    // Resetting an instance there keeps the theme's colours (they're no overrides to reset).
    let inst = doc.node(light_screen).unwrap().children[0];
    ex(&mut s, "component.resetInstance", json!({"ids": [inst]}));
    healthy(s.doc().unwrap());
}

/// A scratch directory for one test.
fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-scale-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The design system published and saved at `path`, and a consumer file whose instances use it
/// from the library (with overrides at every level). Returns (library session, its design system,
/// consumer session, the consumer's copies, the consumer's instances).
fn library_setup(path: &str) -> (Session, DesignSystem, Session, DesignSystem, Vec<NodeId>) {
    let (doc, ds) = design_system_document(&DsSpec::SMALL).unwrap();
    let mut lib_s = Session::with_document(doc);
    ex(&mut lib_s, "library.publish", json!({}));
    ex(&mut lib_s, "file.saveAs", json!({"path": path}));
    let lib = crate::library::Cache::default().load(path).unwrap();
    let mut doc = Document::new("Consumer");
    let m = &lib.manifest;
    doc.libraries_mut().enabled.push(LibraryLink { id: m.id.clone(), name: m.name.clone(), path: path.to_string(), version: m.version });
    let map = crate::library::import(&mut doc, &lib, &ds.assets(), false).unwrap();
    let mut local = ds.remap(&map);
    local.theme = doc.libraries.collections.iter().find(|r| r.key == ds.theme).map(|r| r.local.clone()).unwrap();
    crate::normalize(&mut doc, &Document::new(""));
    let inst = place_instances(&mut doc, &local, COUNT / 2).unwrap();
    crate::normalize(&mut doc, &Document::new(""));
    let consumer = Session::with_document(doc);
    healthy(consumer.doc().unwrap());
    (lib_s, ds, consumer, local, inst)
}

#[test]
fn library_updates_reach_consumers_with_every_override_kept() {
    let dir = scratch("updates");
    let path = dir.join("ds.figcraft").to_string_lossy().into_owned();
    let (mut lib_s, ds, mut s, local, _) = library_setup(&path);
    let before = overrides(s.doc().unwrap());
    assert!(before.len() > 50);
    // The library changes deep mains: the icon's glyph colour (unbinding it), a button label's
    // size, a new layer in the icon.
    let doc = lib_s.doc().unwrap();
    let glyph = doc.node(ds.icons[0]).unwrap().children[0];
    let label = find_path(doc, doc.node(ds.button).unwrap().children[0], &["Label"]).unwrap();
    ex(&mut lib_s, "variable.bind", json!({"ids": [glyph], "field": "fill", "variable": null}));
    ex(&mut lib_s, "node.set", json!({"ids": [glyph], "props": {"fill": "#ff0000"}}));
    ex(&mut lib_s, "node.set", json!({"ids": [label], "props": {"fontSize": 16}}));
    ex(&mut lib_s, "shape.rectangle", json!({"x": 1, "y": 1, "width": 3, "height": 3, "parent": ds.icons[0], "props": {"name": "Dot"}}));
    ex(&mut lib_s, "library.publish", json!({}));
    ex(&mut lib_s, "file.save", json!({}));
    let u = ex(&mut s, "library.updates", json!({}));
    assert!(u["count"].as_u64().unwrap() >= 2, "{u}");
    let r = ex(&mut s, "library.acceptUpdates", json!({}));
    assert!(r["components"].as_u64().unwrap() >= 2, "{r}");
    let doc = s.doc().unwrap();
    healthy(doc);
    // Same layers, same overrides.
    let now = overrides(doc);
    for (id, o) in &before {
        assert_eq!(now.get(id), Some(o), "override of {id}");
    }
    // The copies that don't override the glyph's colour show the new one; the new layer is there.
    let main_glyph = doc.node(local.icons[0]).unwrap().children[0];
    assert_eq!(doc.node(main_glyph).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).as_deref(), Some("FF0000"));
    let red = doc
        .nodes()
        .filter(|n| n.name == "Glyph" && n.overrides.is_empty() && owning_instance(doc, n.id).is_some() && !doc.is_remote(n.id))
        .filter(|n| {
            let icon = owning_instance(doc, n.id).unwrap();
            comp_of(doc, icon) == Some(local.icons[0])
        })
        .inspect(|n| assert_eq!(n.fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).as_deref(), Some("FF0000"), "glyph {}", n.id))
        .count();
    assert!(red > 0);
    assert!(doc.nodes().any(|n| n.name == "Dot" && owning_instance(doc, n.id).is_some() && !doc.is_remote(n.id)));
    assert_eq!(ex(&mut s, "library.updates", json!({}))["count"], 0);
    // Undo takes the update back.
    s.undo().unwrap();
    assert_eq!(overrides(s.doc().unwrap()), before);
    healthy(s.doc().unwrap());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pasting_instances_into_other_files_follows_their_mains() {
    let dir = scratch("paste");
    let path = dir.join("ds.figcraft").to_string_lossy().into_owned();
    let (mut lib_s, ds, mut s, local, inst) = library_setup(&path);
    // From the consumer (library mains) into a new file: the library copies come along.
    let doc = s.doc().unwrap();
    let (nav, _) = first_of(doc, &inst, local.nav, &["Profile"]);
    let o = overrides(doc);
    let mine: Vec<(Vec<String>, String)> = doc.subtree(nav).iter().filter_map(|i| o.get(i).cloned()).collect();
    ex(&mut s, "edit.copy", json!({"ids": [nav]}));
    ex(&mut s, "file.new", json!({"name": "Other"}));
    let r = ex(&mut s, "edit.paste", json!({}));
    let pasted = crate::params::id_of(&r["ids"][0]).unwrap();
    let doc = s.doc().unwrap();
    healthy(doc);
    assert!(doc.is_remote(main_of(doc, pasted)), "linked to a library copy");
    assert_eq!(doc.libraries.enabled.len(), 1);
    let o = overrides(doc);
    let theirs: Vec<(Vec<String>, String)> = doc.subtree(pasted).iter().filter_map(|i| o.get(i).cloned()).collect();
    assert_eq!(theirs.len(), mine.len());
    // Pasting again shares the copies.
    let copies = doc.libraries.components.len();
    ex(&mut s, "edit.paste", json!({}));
    assert_eq!(s.doc().unwrap().libraries.components.len(), copies);
    healthy(s.doc().unwrap());
    // From the library file itself (its published components) into a new file: used from the
    // library, which gets enabled.
    let b = crate::params::id_of(&ex(&mut lib_s, "instance.create", json!({"component": ds.button, "x": 0, "y": -400}))["id"]).unwrap();
    ex(&mut lib_s, "edit.copy", json!({"ids": [b]}));
    ex(&mut lib_s, "file.new", json!({"name": "Third"}));
    let r = ex(&mut lib_s, "edit.paste", json!({}));
    let pasted = crate::params::id_of(&r["ids"][0]).unwrap();
    let doc = lib_s.doc().unwrap();
    healthy(doc);
    assert!(doc.is_remote(main_of(doc, pasted)));
    assert_eq!(doc.libraries.enabled.first().map(|l| l.path.as_str()), Some(path.as_str()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pasting_an_instance_of_an_unpublished_component_into_another_file_leaves_its_main_behind() {
    let mut s = Session::new();
    let f = crate::params::id_of(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 50, "height": 50, "name": "Tile"}))["id"]).unwrap();
    ex(&mut s, "shape.rectangle", json!({"x": 5, "y": 5, "width": 10, "height": 10, "parent": f, "props": {"name": "Dot"}}));
    let c = crate::params::id_of(&ex(&mut s, "component.create", json!({"ids": [f]}))["id"]).unwrap();
    let i = crate::params::id_of(&ex(&mut s, "instance.create", json!({"component": c, "x": 100, "y": 0}))["id"]).unwrap();
    ex(&mut s, "edit.copy", json!({"ids": [i]}));
    ex(&mut s, "file.new", json!({}));
    // Layers whose ids collide with the copied mains' ids in the other file.
    for k in 0..6 {
        ex(&mut s, "shape.rectangle", json!({"x": k * 20, "y": 300, "width": 10, "height": 10}));
    }
    let r = ex(&mut s, "edit.paste", json!({}));
    let pasted = crate::params::id_of(&r["ids"][0]).unwrap();
    let doc = s.doc().unwrap();
    healthy(doc);
    let main = main_of(doc, pasted);
    assert!(!doc.contains(main), "the main stays in its own file");
    for d in doc.subtree(pasted) {
        assert!(doc.node(d).unwrap().source.is_none_or(|src| !doc.contains(src) || doc.is_ancestor(pasted, src)), "{d} mirrors a layer of this file");
    }
    // The main can be restored here.
    ex(&mut s, "component.restoreMain", json!({"id": pasted}));
    let doc = s.doc().unwrap();
    healthy(doc);
    assert!(matches!(doc.node(main).unwrap().kind, NodeKind::Component { .. }));
}

/// A "Chip" component whose fill and corner radius are bound to variables of a light/dark
/// collection, holding an "Icon" instance (renamed, exposed) of a "Dot" component. Returns
/// (chip, icon instance in the main, the colour variable's light and dark hex).
fn bound_chip(s: &mut Session) -> (NodeId, NodeId) {
    let id = |v: Value| crate::params::id_of(&v["id"]).unwrap();
    ex(s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "Chip", "type": "color", "value": "#112233"}));
    ex(s, "variable.set", json!({"variable": "Chip", "mode": "Dark", "value": "#aabbcc"}));
    ex(s, "variable.create", json!({"collection": "Theme", "name": "Round", "type": "number", "value": 4}));
    ex(s, "variable.set", json!({"variable": "Round", "mode": "Dark", "value": 9}));
    let dot = id(ex(s, "frame.create", json!({"x": 0, "y": 300, "width": 10, "height": 10, "name": "Dot"})));
    let dot = id(ex(s, "component.create", json!({"ids": [dot]})));
    let ring = id(ex(s, "frame.create", json!({"x": 40, "y": 300, "width": 10, "height": 10, "name": "Ring"})));
    ex(s, "component.create", json!({"ids": [ring]}));
    let f = id(ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 80, "height": 30, "name": "Chip"})));
    let bg = id(ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 80, "height": 30, "parent": f, "props": {"name": "Bg"}})));
    ex(s, "variable.bind", json!({"ids": [bg], "field": "fill", "variable": "Chip"}));
    ex(s, "variable.bind", json!({"ids": [bg], "field": "cornerRadius", "variable": "Round"}));
    let icon = id(ex(s, "instance.create", json!({"component": dot, "x": 5, "y": 5, "parent": f})));
    ex(s, "node.set", json!({"ids": [icon], "props": {"name": "Icon"}}));
    let chip = id(ex(s, "component.create", json!({"ids": [f]})));
    ex(s, "component.exposeInstance", json!({"id": icon}));
    (chip, icon)
}

#[test]
fn variables_on_instance_layers_are_no_overrides_and_detached_ones_stay_overridden() {
    let mut s = Session::new();
    let (chip, _) = bound_chip(&mut s);
    let i = crate::params::id_of(&ex(&mut s, "instance.create", json!({"component": chip, "x": 200, "y": 0}))["id"]).unwrap();
    let doc = s.doc().unwrap();
    let bg = find_path(doc, i, &["Bg"]).unwrap();
    ex(&mut s, "variable.setMode", json!({"ids": [i], "collection": "Theme", "mode": "Dark"}));
    let n = s.doc().unwrap().node(bg).unwrap().clone();
    assert_eq!(n.fills[0].solid_color().unwrap().to_hex(), "AABBCC");
    assert_eq!(n.corners.radii, [9.0; 4]);
    assert!(n.overrides.is_empty(), "values from variables aren't overrides: {:?}", n.overrides);
    // Detach the colour on the instance and pick another: an override that survives main edits.
    ex(&mut s, "variable.bind", json!({"ids": [bg], "field": "fill", "variable": null}));
    ex(&mut s, "node.set", json!({"ids": [bg], "props": {"fill": "#ff00ff"}}));
    assert!(s.doc().unwrap().node(bg).unwrap().overrides.contains("fills"));
    let main_bg = find_path(s.doc().unwrap(), chip, &["Bg"]).unwrap();
    ex(&mut s, "node.set", json!({"ids": [main_bg], "props": {"height": 28}}));
    ex(&mut s, "shape.ellipse", json!({"x": 60, "y": 5, "width": 5, "height": 5, "parent": chip}));
    let doc = s.doc().unwrap();
    healthy(doc);
    let n = doc.node(bg).unwrap();
    assert_eq!(n.fills[0].solid_color().unwrap().to_hex(), "FF00FF");
    assert!(!n.bound.contains_key("fill"));
    assert_eq!(n.corners.radii, [9.0; 4], "the corner radius still follows its variable");
    assert_eq!(n.height, 28.0);
    // Reset brings the variable back.
    ex(&mut s, "component.resetOverride", json!({"id": bg, "group": "fills"}));
    let n = s.doc().unwrap().node(bg).unwrap().clone();
    assert_eq!(n.bound.get("fill").map(String::as_str), s.doc().unwrap().node(main_bg).unwrap().bound.get("fill").map(String::as_str));
    assert_eq!(n.fills[0].solid_color().unwrap().to_hex(), "AABBCC");
}

#[test]
fn nested_instances_keep_exposure_and_names_through_main_edits_and_swaps() {
    let mut s = Session::new();
    let (chip, icon) = bound_chip(&mut s);
    let ring = s.doc().unwrap().nodes().find(|n| n.name == "Ring" && matches!(n.kind, NodeKind::Component { .. })).unwrap().id;
    // Editing the nested instance's own main rebuilds it inside the chip main.
    let dot = comp_of(s.doc().unwrap(), icon).unwrap();
    ex(&mut s, "node.set", json!({"ids": [dot], "props": {"fill": "#00ff00", "width": 12}}));
    ex(&mut s, "shape.rectangle", json!({"x": 1, "y": 1, "width": 2, "height": 2, "parent": dot}));
    let doc = s.doc().unwrap();
    assert!(doc.node(icon).unwrap().exposed, "still exposed");
    assert_eq!(doc.node(icon).unwrap().name, "Icon");
    let i = crate::params::id_of(&ex(&mut s, "instance.create", json!({"component": chip, "x": 200, "y": 0}))["id"]).unwrap();
    let nested = find_path(s.doc().unwrap(), i, &["Icon"]).unwrap();
    assert_eq!(ex(&mut s, "instance.exposed", json!({"id": i}))["instances"][0]["id"], json!(nested.0));
    // Swapped, the nested instance keeps the name its layer has in the main.
    ex(&mut s, "instance.swap", json!({"ids": [nested], "component": ring}));
    let doc = s.doc().unwrap();
    healthy(doc);
    assert_eq!(doc.node(nested).unwrap().name, "Icon");
    assert_eq!(comp_of(doc, nested), Some(ring));
}

fn centre(doc: &Document, id: NodeId) -> (f64, f64) {
    let b = doc.world_bounds(id).unwrap();
    (b.center().x, b.center().y)
}

#[test]
fn alt_dropping_a_component_onto_an_instance_swaps_it() {
    let (mut s, ds, inst) = session();
    let doc = &s.doc().unwrap().clone();
    // Over a button's icon, a button variant swaps the button (same component set) ...
    let (button, glyph) = inst
        .iter()
        .copied()
        .filter(|i| variant_set(doc, main_of(doc, *i)) == Some(ds.button))
        .find_map(|i| {
            find_path(doc, i, &["Icon", "Glyph"])
                .filter(|_| find_path(doc, i, &["Icon"]).is_some_and(|c| doc.node(c).unwrap().visible))
                .map(|g| (i, g))
        })
        .unwrap();
    s.ds_mut().unwrap().page = doc.page_of(button).unwrap();
    let label = chars(doc, find_path(doc, button, &["Label"]).unwrap());
    let other = doc.node(ds.button).unwrap().children.iter().copied().find(|v| *v != main_of(doc, button)).unwrap();
    let (x, y) = centre(doc, glyph);
    assert_eq!(crate::drop::swap_target(doc, s.page().unwrap(), figcraft_geom::Point::new(x, y), Some("Button"), Some(other)), Some(button));
    let r = ex(&mut s, "instance.swapAt", json!({"component": other, "x": x, "y": y}));
    assert_eq!(r["id"], json!(button.0));
    let doc = s.doc().unwrap();
    healthy(doc);
    assert_eq!(main_of(doc, button), other);
    assert_eq!(s.selection(), vec![button]);
    // ... keeping only text layers' overrides (by matching names) and the property values.
    let new_glyph = find_path(doc, button, &["Icon", "Glyph"]).unwrap();
    assert!(doc.node(new_glyph).unwrap().overrides.is_empty());
    assert_eq!(chars(doc, find_path(doc, button, &["Label"]).unwrap()), label);
    // An icon dropped there swaps the button's icon (a nested swap, an override).
    let icon = find_path(doc, button, &["Icon"]).unwrap();
    let to = ds.icons.iter().copied().skip(1).find(|c| Some(*c) != comp_of(doc, icon)).unwrap();
    let (x, y) = centre(doc, find_path(doc, button, &["Icon", "Glyph"]).unwrap());
    ex(&mut s, "instance.swapAt", json!({"component": to, "x": x, "y": y}));
    let doc = s.doc().unwrap();
    healthy(doc);
    assert_eq!(comp_of(doc, icon), Some(to));
    assert!(doc.node(icon).unwrap().overrides.contains("component"));
    let glyph = find_path(doc, icon, &["Glyph"]).unwrap();
    assert!(doc.node(glyph).unwrap().overrides.is_empty());
    assert_eq!(doc.node(glyph).unwrap().fills, doc.node(doc.node(glyph).unwrap().source.unwrap()).unwrap().fills);
    // Nothing there: refused, nothing changes.
    let before = s.doc().unwrap().clone();
    assert!(s.execute("instance.swapAt", &json!({"component": to, "x": -50_000, "y": -50_000})).is_err());
    assert_eq!(*s.doc().unwrap(), before);
    // A component never swaps an instance inside itself.
    let main_icon = find_path(s.doc().unwrap(), other, &["Icon"]).unwrap();
    let (x, y) = centre(s.doc().unwrap(), main_icon);
    let r = s.execute("instance.swapAt", &json!({"component": other, "x": x, "y": y}));
    assert!(r.is_err() || r.unwrap()["id"] != json!(main_icon.0));
    s.undo().unwrap();
    s.undo().unwrap();
    healthy(s.doc().unwrap());
}

#[test]
fn alt_dropping_a_library_component_swaps_to_it() {
    let dir = scratch("swap");
    let path = dir.join("ds.figcraft").to_string_lossy().into_owned();
    let (_lib, ds, mut s, local, inst) = library_setup(&path);
    let doc = &s.doc().unwrap().clone();
    let (top, glyph) =
        inst.iter().copied().filter(|i| local.icons.contains(&main_of(doc, *i))).find_map(|i| find_path(doc, i, &["Glyph"]).map(|g| (i, g))).unwrap();
    s.ds_mut().unwrap().page = doc.page_of(top).unwrap();
    let k = ds.icons.len() - 1;
    let (x, y) = centre(doc, glyph);
    ex(&mut s, "library.swapAt", json!({"library": "Design system", "component": ds.icons[k].0, "x": x, "y": y}));
    let doc = s.doc().unwrap();
    healthy(doc);
    assert_eq!(main_of(doc, top), local.icons[k]);
    // Swapped from the Assets panel: only text layers keep their overrides.
    let glyph = find_path(doc, top, &["Glyph"]).unwrap();
    assert!(doc.node(glyph).unwrap().overrides.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_fill_set_on_a_bound_instance_layer_detaches_it_as_an_override() {
    // Without unbinding first: the explicit colour detaches the variable on the instance layer only,
    // and stays through main edits until the override is reset.
    let mut s = Session::new();
    let (chip, _) = bound_chip(&mut s);
    let i = crate::params::id_of(&ex(&mut s, "instance.create", json!({"component": chip, "x": 200, "y": 0}))["id"]).unwrap();
    let bg = find_path(s.doc().unwrap(), i, &["Bg"]).unwrap();
    ex(&mut s, "node.set", json!({"ids": [bg], "props": {"fill": "#ff00ff"}}));
    let n = s.doc().unwrap().node(bg).unwrap().clone();
    assert_eq!(n.fills[0].solid_color().unwrap().to_hex(), "FF00FF");
    assert!(!n.bound.contains_key("fill"));
    assert!(n.overrides.contains("fills"));
    let main_bg = find_path(s.doc().unwrap(), chip, &["Bg"]).unwrap();
    ex(&mut s, "node.set", json!({"ids": [main_bg], "props": {"height": 28}}));
    ex(&mut s, "variable.set", json!({"variable": "Chip", "value": "#445566"}));
    let doc = s.doc().unwrap();
    healthy(doc);
    assert_eq!(doc.node(bg).unwrap().fills[0].solid_color().unwrap().to_hex(), "FF00FF");
    assert!(doc.node(main_bg).unwrap().bound.contains_key("fill"), "the main keeps its binding");
    ex(&mut s, "component.resetOverride", json!({"id": bg, "group": "fills"}));
    let n = s.doc().unwrap().node(bg).unwrap();
    assert!(n.bound.contains_key("fill"));
    assert_eq!(n.fills[0].solid_color().unwrap().to_hex(), "445566");
}
