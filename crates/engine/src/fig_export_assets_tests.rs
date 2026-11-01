//! `.fig` export through `file.export` and back through `file.open`: variables with their modes,
//! the layers bound to them (also inside instances shown in another mode), shared styles, and
//! instances that are resized, fill an auto layout or were scaled come back as they were. The
//! format's own tests (written fields, damaged files) are in `figcraft_format::fig`.

use figcraft_doc::{Document, NodeKind};
use serde_json::{Value, json};

use crate::Session;

fn id(v: &Value) -> u64 {
    v["id"].as_u64().unwrap()
}

fn run(s: &mut Session, cmd: &str, params: Value) -> Value {
    s.execute(cmd, &params).unwrap_or_else(|e| panic!("{cmd}: {e:?}"))
}

/// A card component (a box that scales with it, a label), instances of it as it is, resized,
/// filling an auto layout frame and scaled; a theme with two modes bound to two frames, one of
/// them dark and holding another instance; a paint style on a rectangle.
fn build(s: &mut Session) {
    let card = id(&run(s, "frame.create", json!({"x": 40, "y": 40, "width": 100, "height": 60, "name": "Card"})));
    let boxed = json!({"fill": "#3B82F6", "name": "Box", "constraints": {"horizontal": "scale", "vertical": "scale"}});
    run(s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40, "parent": card, "props": boxed}));
    let label = id(&run(s, "text.create", json!({"text": "Card", "x": 56, "y": 20, "parent": card, "props": {"name": "Label"}})));
    run(s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    run(s, "variable.create", json!({"collection": "Theme", "name": "bg", "type": "color", "values": {"Light": "#FFFFFF", "Dark": "#111827"}}));
    run(s, "variable.create", json!({"collection": "Theme", "name": "fg", "type": "color", "values": {"Light": "#111827", "Dark": "#F9FAFB"}}));
    run(s, "variable.create", json!({"collection": "Theme", "name": "gap", "type": "number", "values": {"Light": 8, "Dark": 16}}));
    run(s, "variable.bind", json!({"ids": [card], "field": "fill", "variable": "bg"}));
    run(s, "variable.bind", json!({"ids": [label], "field": "fill", "variable": "fg"}));
    let comp = id(&run(s, "component.create", json!({"ids": [card]})));
    let same = id(&run(s, "instance.create", json!({"component": comp, "x": 200, "y": 40})));
    run(s, "node.set", json!({"ids": [same], "name": "Same"}));
    let resized = id(&run(s, "instance.create", json!({"component": comp, "x": 360, "y": 40})));
    run(s, "node.set", json!({"ids": [resized], "name": "Resized", "width": 200, "height": 120}));
    let row = id(&run(s, "frame.create", json!({"x": 40, "y": 220, "width": 400, "height": 100, "name": "Row"})));
    run(s, "layout.add", json!({"ids": [row], "direction": "horizontal"}));
    run(s, "node.set", json!({"ids": [row], "width": 400, "height": 100, "sizingH": "fixed", "sizingV": "fixed"}));
    run(s, "variable.bind", json!({"ids": [row], "field": "gap", "variable": "gap"}));
    let filling = id(&run(s, "instance.create", json!({"component": comp, "parent": row})));
    run(s, "node.set", json!({"ids": [filling], "name": "Filling", "sizingH": "fill", "sizingV": "fill"}));
    let scaled = id(&run(s, "instance.create", json!({"component": comp, "x": 620, "y": 40})));
    run(s, "node.set", json!({"ids": [scaled], "name": "Scaled"}));
    run(s, "transform.scale", json!({"ids": [scaled], "factor": 1.5}));
    let dark = id(&run(s, "frame.create", json!({"x": 40, "y": 360, "width": 300, "height": 120, "name": "Dark"})));
    run(s, "variable.bind", json!({"ids": [dark], "field": "fill", "variable": "bg"}));
    run(s, "variable.setMode", json!({"ids": [dark], "collection": "Theme", "mode": "Dark"}));
    let inside = id(&run(s, "instance.create", json!({"component": comp, "parent": dark, "x": 20, "y": 20})));
    run(s, "node.set", json!({"ids": [inside], "name": "In the dark"}));
    let swatch =
        id(&run(s, "shape.rectangle", json!({"x": 520, "y": 360, "width": 100, "height": 100, "props": {"fill": "#F59E0B", "name": "Swatch"}})));
    run(s, "style.create", json!({"type": "paint", "name": "Brand/Accent", "from": swatch}));
    run(s, "style.apply", json!({"ids": [swatch], "style": "Brand/Accent"}));
}

/// Every layer, depth-first: name, kind, place, size, first fill colour, what it is bound to and
/// the styles and modes it names (by name: a round trip gives new ids).
fn outline(doc: &Document) -> Vec<String> {
    let var = |v: &String| doc.variables.get(v).map_or("?", |x| x.name.as_str()).to_string();
    let style = |r: &Option<String>| r.as_ref().and_then(|i| doc.style(i)).map(|x| x.name.clone());
    doc.pages
        .iter()
        .flat_map(|p| doc.subtree(*p))
        .filter_map(|i| doc.node(i))
        .map(|n| {
            let t = n.transform.translation();
            let fill = n.fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex());
            let bound: Vec<(&str, String)> = n.bound.iter().map(|(k, v)| (k.as_str(), var(v))).collect();
            let modes: Vec<(String, String)> = n
                .var_modes
                .iter()
                .filter_map(|(c, m)| doc.variables.collection(c).map(|c| (c.name.clone(), c.find_mode(m).map_or("?".into(), |m| m.name.clone()))))
                .collect();
            let size = n.kind.text().map(|t| t.style.font_size);
            format!(
                "{} {:?} ({:.1}, {:.1}) {:.1}×{:.1} weight {:.2} font {size:?} fill {fill:?} bound {bound:?} modes {modes:?} style {:?} gap {:?}",
                n.kind.type_name(),
                n.name,
                t.x,
                t.y,
                n.width,
                n.height,
                n.stroke.weight,
                style(&n.styles.fill),
                n.layout.as_ref().map(|l| l.gap),
            )
        })
        .collect()
}

#[test]
fn variables_styles_and_instance_sizes_come_back() {
    let mut s = Session::new();
    build(&mut s);
    let before = outline(s.doc().unwrap());
    // What the outline shows: the instance in the dark frame takes the dark values.
    assert!(before.iter().any(|l| l.contains("\"In the dark\"") && l.contains("111827")), "{before:#?}");
    assert!(before.iter().any(|l| l.contains("\"Resized\"") && l.contains("200.0×120.0")), "{before:#?}");
    let path = std::env::temp_dir().join(format!("figcraft-fig-assets-{}.fig", std::process::id()));
    let r = run(&mut s, "file.export", json!({"format": "fig", "path": path.to_string_lossy()}));
    assert_eq!(r["warnings"], json!([]));
    let r = run(&mut s, "file.open", json!({"path": path.to_string_lossy()}));
    assert_eq!(r["warnings"], json!([]));
    let _ = std::fs::remove_file(&path);
    let doc = s.doc().unwrap();
    assert_eq!(outline(doc), before);
    // Collections, modes, variables and styles themselves.
    let theme = doc.variables.collection("Theme").unwrap();
    assert_eq!(theme.modes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Light", "Dark"]);
    assert_eq!(doc.variables.variables.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["bg", "fg", "gap"]);
    assert_eq!(doc.styles.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Brand/Accent"]);
    // The instances are linked to the main again, layer by layer.
    for inst in doc.nodes().filter(|n| matches!(n.kind, NodeKind::Instance { .. })) {
        assert!(inst.children.iter().all(|c| doc.node(*c).is_some_and(|c| c.source.is_some())), "{}", inst.name);
    }
    // Switching the dark frame back to light recolours what is in it: the bindings are live.
    let dark = doc.nodes().find(|n| n.name == "Dark").unwrap().id.0;
    run(&mut s, "variable.setMode", json!({"ids": [dark], "collection": "Theme", "mode": "Light"}));
    let after = outline(s.doc().unwrap());
    assert!(after.iter().any(|l| l.contains("\"In the dark\"") && l.contains("FFFFFF")), "{after:#?}");
}

#[test]
fn every_example_keeps_its_variables_and_styles() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let Ok(files) = std::fs::read_dir(dir) else { return };
    for p in files.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "figcraft")) {
        let mut s = Session::new();
        run(&mut s, "file.open", json!({"path": p.to_string_lossy()}));
        let before = outline(s.doc().unwrap());
        let counts = |d: &Document| (d.variables.collections.len(), d.variables.variables.len(), d.styles.len());
        let had = counts(s.doc().unwrap());
        let out = std::env::temp_dir().join(format!("figcraft-fig-assets-{}-{}.fig", std::process::id(), p.file_stem().unwrap().to_string_lossy()));
        let r = run(&mut s, "file.export", json!({"format": "fig", "path": out.to_string_lossy()}));
        let warnings = r["warnings"].to_string();
        assert!(!warnings.contains("variable") && !warnings.contains("style"), "{}: {warnings}", p.display());
        run(&mut s, "file.open", json!({"path": out.to_string_lossy()}));
        let _ = std::fs::remove_file(&out);
        assert_eq!(counts(s.doc().unwrap()), had, "{}", p.display());
        // Names, paints, bindings, modes and style links (the format's tests compare the geometry,
        // which 32-bit floats round).
        let links = |lines: &[String]| -> Vec<String> {
            lines.iter().map(|l| format!("{} |{}", l.split(" (").next().unwrap_or(""), l.split(" fill ").nth(1).unwrap_or(""))).collect()
        };
        let after = outline(s.doc().unwrap());
        assert_eq!(after.len(), before.len(), "{}", p.display());
        for (a, b) in links(&after).iter().zip(&links(&before)) {
            assert_eq!(a, b, "{}", p.display());
        }
    }
}

/// A component holding an instance of a variant, under a name of its own: opened from a `.fig`
/// file, instances of that component link every layer to the main's, the nested instance and its
/// layers included (a nested instance is first linked to its own component, whose name is not the
/// layer's).
#[test]
fn nested_instances_of_variants_link_to_their_mains_layers() {
    let mut s = Session::new();
    let variant = |s: &mut Session, y: i64, fill: &str| {
        let f = id(&run(s, "frame.create", json!({"x": 0, "y": y, "width": 120, "height": 80, "name": "Scene"})));
        run(s, "node.set", json!({"ids": [f], "fill": fill}));
        run(s, "shape.ellipse", json!({"x": 70, "y": 10, "width": 30, "height": 30, "parent": f, "props": {"name": "Sun"}}));
        id(&run(s, "component.create", json!({"ids": [f]})))
    };
    let (day, dusk) = (variant(&mut s, 0, "#8FCDF2"), variant(&mut s, 120, "#4A3A8E"));
    run(&mut s, "node.set", json!({"ids": [day], "name": "Time=Day"}));
    run(&mut s, "node.set", json!({"ids": [dusk], "name": "Time=Dusk"}));
    run(&mut s, "component.combineVariants", json!({"ids": [day, dusk]}));
    let card = id(&run(&mut s, "frame.create", json!({"x": 300, "y": 0, "width": 200, "height": 140, "name": "Card"})));
    let photo = id(&run(&mut s, "instance.create", json!({"component": day, "parent": card, "x": 10, "y": 10})));
    run(&mut s, "node.set", json!({"ids": [photo], "name": "Photo", "width": 180, "height": 120}));
    let card = id(&run(&mut s, "component.create", json!({"ids": [card]})));
    let first = id(&run(&mut s, "instance.create", json!({"component": card, "x": 300, "y": 200})));
    let second = id(&run(&mut s, "instance.create", json!({"component": card, "x": 600, "y": 200})));
    run(&mut s, "node.set", json!({"ids": [first], "name": "First"}));
    run(&mut s, "node.set", json!({"ids": [second], "name": "Second"}));
    // The second one shows the other variant.
    let nested = s.doc().unwrap().node(figcraft_doc::NodeId(second)).unwrap().children[0].0;
    run(&mut s, "instance.swap", json!({"ids": [nested], "component": dusk}));
    let before = outline(s.doc().unwrap());
    let path = std::env::temp_dir().join(format!("figcraft-fig-nested-{}.fig", std::process::id()));
    run(&mut s, "file.export", json!({"format": "fig", "path": path.to_string_lossy()}));
    let r = run(&mut s, "file.open", json!({"path": path.to_string_lossy()}));
    assert_eq!(r["warnings"], json!([]));
    let doc = s.doc().unwrap();
    assert_eq!(outline(doc), before);
    let mut nested = 0;
    for inst in doc.nodes().filter(|n| matches!(n.kind, NodeKind::Instance { .. })) {
        let inside = doc.ancestors(inst.id).iter().any(|a| doc.node(*a).is_some_and(|x| matches!(x.kind, NodeKind::Instance { .. })));
        nested += usize::from(inside);
        for layer in doc.subtree(inst.id).into_iter().skip(1).filter_map(|i| doc.node(i)) {
            assert!(layer.source.is_some(), "{} in {}", layer.name, inst.name);
        }
        // Inside another instance, a nested instance mirrors the main's layer, not its component.
        if let (true, NodeKind::Instance { component, .. }) = (inside, &inst.kind) {
            assert_ne!(inst.source, Some(*component), "{}", inst.name);
        }
    }
    assert_eq!(nested, 2);
    // Exported and opened once more, nothing moves: the links were what the export needed.
    run(&mut s, "file.export", json!({"format": "fig", "path": path.to_string_lossy()}));
    run(&mut s, "file.open", json!({"path": path.to_string_lossy()}));
    let _ = std::fs::remove_file(&path);
    assert_eq!(outline(s.doc().unwrap()), before);
}

#[test]
fn stored_variable_ids_keep_exported_binding_identity() {
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "Theme"}));
    let shadow = run(&mut s, "variable.create", json!({"collection": "Theme", "name": "shadow", "type": "color", "value": "#FF0000"}));
    let surface = run(&mut s, "variable.create", json!({"collection": "Theme", "name": "surface", "type": "color", "value": "#0D99FF"}));
    let r = id(&run(&mut s, "node.create", json!({"type": "rectangle", "props": {"name": "Bound", "fill": {"$var": surface["id"]}}})));
    run(&mut s, "select.set", json!({"ids": [r]}));
    run(&mut s, "style.create", json!({"type": "paint", "name": "Surface"}));
    run(&mut s, "node.set", json!({"id": r, "props": {"name": "Styled"}}));
    run(&mut s, "node.create", json!({"type": "rectangle", "props": {"name": "Bound", "fill": {"$var": surface["id"]}}}));
    run(&mut s, "variable.update", json!({"variable": shadow["id"], "name": surface["id"]}));
    let (bytes, _) = figcraft_format::fig::export(s.doc().unwrap(), None).unwrap();
    let (back, _) = figcraft_format::fig::import(&bytes, "identity").unwrap();
    let bound = back.nodes().find(|n| n.name == "Bound").unwrap().bound.values().next().unwrap();
    assert_eq!(back.variables.get_by_id(bound).unwrap().name, "surface");
    let style = back.styles.iter().find(|s| s.name == "Surface").unwrap();
    assert!(!style.bound.is_empty());
    for bound in style.bound.values() {
        assert_eq!(back.variables.get_by_id(bound).unwrap().name, "surface");
    }
}
