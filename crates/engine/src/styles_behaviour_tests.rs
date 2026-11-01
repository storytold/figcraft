//! Styles behaviour suite, written from the public documentation of styles in design tools:
//! what a style holds, creating one from a selection, applying, editing (every user follows),
//! detaching (values stay), editing a styled property, organising (folders by "/" names, order,
//! duplicates), descriptions and deleting a used style. Each case gives the rule's source in one
//! line; "docs silent" marks behaviour the documentation doesn't describe (pinned as it is).

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{Node, NodeId};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn sid(v: Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    id(s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 40, "height": 40, "props": {"fill": "#FF0000"}})).unwrap())
}

fn text(s: &mut Session, x: f64) -> NodeId {
    id(s.execute("text.create", &json!({"text": "Hello", "x": x, "y": 100})).unwrap())
}

fn node(s: &Session, n: NodeId) -> Node {
    s.doc().unwrap().node(n).unwrap().clone()
}

fn hex(s: &Session, n: NodeId) -> Vec<String> {
    node(s, n).fills.iter().filter_map(|p| p.solid_color()).map(|c| c.to_hex()).collect()
}

fn font_size(s: &Session, n: NodeId) -> f64 {
    node(s, n).kind.text().unwrap().style.font_size
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

/// One layer of each style type linked to a style made from it: (layer, style id, kind).
fn styled(s: &mut Session) -> Vec<(NodeId, String, &'static str)> {
    let r = rect(s, 0.0);
    run(s, "effect.add", json!({"ids": [r], "type": "dropShadow"}));
    run(s, "grid.add", json!({"ids": [r]}));
    run(s, "node.set", json!({"id": r, "props": {"strokes": ["#00FF00"]}}));
    let t = text(s, 0.0);
    let mut out = Vec::new();
    for (ty, layer) in [("paint", r), ("stroke", r), ("effect", r), ("grid", r), ("text", t)] {
        let st = sid(run(s, "style.create", json!({"type": ty, "name": format!("S/{ty}"), "from": layer})));
        out.push((layer, st, ty));
    }
    out
}

/// What a layer shows for a style kind (comparable before/after).
fn shown(s: &Session, n: NodeId, kind: &str) -> String {
    let n = node(s, n);
    match kind {
        "paint" => format!("{:?}", n.fills),
        "stroke" => format!("{:?}", n.strokes),
        "effect" => format!("{:?}", n.effects),
        "grid" => format!("{:?}", n.grids),
        _ => format!("{:?}", n.kind.text().map(|t| t.style.as_text_style())),
    }
}

fn link(s: &Session, n: NodeId, kind: &str) -> Option<String> {
    let r = node(s, n).styles;
    match kind {
        "paint" => r.fill,
        "stroke" => r.stroke,
        "effect" => r.effect,
        "grid" => r.grid,
        _ => r.text,
    }
}

/// The key linking a style of `kind` (a paint style links fills).
fn style_key(kind: &str) -> String {
    if kind == "paint" { "fillStyle".into() } else { format!("{kind}Style") }
}

/// `node.create` params for an unselected layer that takes `kind` styles, with `props`.
fn new_layer(kind: &str, props: Value) -> Value {
    if kind == "text" {
        json!({"type": "text", "text": "Hej", "select": false, "props": props})
    } else {
        json!({"type": "frame", "select": false, "props": props})
    }
}

#[test]
fn a_style_holds_the_whole_property_and_links_its_source() {
    // Documented: colour styles can hold several fills (gradients, images); effect styles combine
    // several effects; creating a style from a selection uses (and links) its properties.
    let mut s = Session::new();
    let r = rect(&mut s, 0.0);
    run(&mut s, "node.set", json!({"id": r, "props": {"fills": ["#FF0000", "#0000FF80"]}}));
    run(&mut s, "effect.add", json!({"ids": [r], "type": "dropShadow"}));
    run(&mut s, "effect.add", json!({"ids": [r], "type": "layerBlur"}));
    let paint = sid(run(&mut s, "style.create", json!({"type": "paint", "name": "Brand/Duo", "from": r})));
    let fx = sid(run(&mut s, "style.create", json!({"type": "effect", "name": "Depth", "from": r})));
    let b = rect(&mut s, 100.0);
    run(&mut s, "style.apply", json!({"ids": [b], "style": paint}));
    run(&mut s, "style.apply", json!({"ids": [b], "style": fx}));
    let cases: [(&str, usize, usize); 2] =
        [("two fills in one colour style", node(&s, b).fills.len(), 2), ("two effects", node(&s, b).effects.len(), 2)];
    for (what, got, want) in cases {
        assert_eq!(got, want, "{what}");
    }
    assert_eq!(hex(&s, b), ["FF0000", "0000FF"]);
    assert_eq!(node(&s, r).styles.fill.as_deref(), Some(paint.as_str()), "the source layer is linked");
    assert_eq!(node(&s, r).styles.effect.as_deref(), Some(fx.as_str()));
    // A paint style applies to strokes too (documented: colour styles for fills and strokes).
    run(&mut s, "style.apply", json!({"ids": [b], "style": paint, "target": "stroke"}));
    assert_eq!(node(&s, b).strokes.len(), 2);
    assert_eq!(node(&s, b).styles.stroke.as_deref(), Some(paint.as_str()));
}

#[test]
fn editing_a_style_updates_every_layer_using_it() {
    // Documented: a style edit reaches every layer linked to it. Table: per kind, give the style a
    // new definition and check a second user follows.
    let mut s = Session::new();
    let rows = styled(&mut s);
    let other_rect = rect(&mut s, 300.0);
    let other_text = text(&mut s, 300.0);
    for (_, st, kind) in &rows {
        let user = if *kind == "text" { other_text } else { other_rect };
        let target = if *kind == "stroke" { json!("stroke") } else { Value::Null };
        run(&mut s, "style.apply", json!({"ids": [user], "style": st, "target": target}));
        assert_eq!(link(&s, user, kind).as_deref(), Some(st.as_str()), "{kind}: linked");
    }
    // New definitions.
    let edits: [(&str, Value); 5] = [
        ("paint", json!(["#123456"])),
        ("stroke", json!(["#654321"])),
        ("effect", json!([])),
        ("grid", json!([])),
        ("text", json!({"fontSize": 31})),
    ];
    for (kind, value) in edits {
        let (_, st, _) = rows.iter().find(|r| r.2 == kind).unwrap();
        run(&mut s, "style.update", json!({"style": st, "value": value}));
    }
    assert_eq!(hex(&s, other_rect), ["123456"], "paint: fills follow");
    assert_eq!(node(&s, other_rect).strokes.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).as_deref(), Some("654321"), "stroke");
    assert!(node(&s, other_rect).effects.is_empty(), "effect");
    assert!(node(&s, other_rect).grids.is_empty(), "grid");
    assert_eq!(font_size(&s, other_text), 31.0, "text");
    // The source layers follow too, and stay linked.
    for (layer, st, kind) in &rows {
        assert_eq!(link(&s, *layer, kind).as_deref(), Some(st.as_str()), "{kind}: source still linked");
    }
    assert_eq!(font_size(&s, rows[4].0), 31.0);
    // One undo step puts the style and its users back.
    s.undo().unwrap();
    assert_ne!(font_size(&s, other_text), 31.0);
    assert_eq!(hex(&s, other_rect), ["123456"], "earlier edits stay");
}

#[test]
fn detaching_keeps_the_current_values() {
    // Documented: a detached layer keeps its current values and stops following the style.
    let mut s = Session::new();
    let rows = styled(&mut s);
    for (layer, st, kind) in &rows {
        let before = shown(&s, *layer, kind);
        let which = if *kind == "paint" { "fill" } else { kind };
        run(&mut s, "style.detach", json!({"ids": [layer], "which": which}));
        assert_eq!(link(&s, *layer, kind), None, "{kind}: unlinked");
        assert_eq!(shown(&s, *layer, kind), before, "{kind}: values kept");
        let value = match *kind {
            "text" => json!({"fontSize": 50}),
            "paint" | "stroke" => json!(["#ABCDEF"]),
            _ => json!([]),
        };
        run(&mut s, "style.update", json!({"style": st, "value": value}));
        assert_eq!(shown(&s, *layer, kind), before, "{kind}: style edits no longer reach it");
    }
}

#[test]
fn deleting_a_used_style_keeps_the_values() {
    // Documented: deleting a style unlinks the layers using it; they keep what they show.
    let mut s = Session::new();
    let rows = styled(&mut s);
    for (layer, st, kind) in &rows {
        let before = shown(&s, *layer, kind);
        run(&mut s, "style.delete", json!({"style": st}));
        assert_eq!(link(&s, *layer, kind), None, "{kind}: unlinked");
        assert_eq!(shown(&s, *layer, kind), before, "{kind}: values kept");
    }
    assert!(s.doc().unwrap().styles.is_empty());
    // Undo brings the style and the link back.
    s.undo().unwrap();
    assert_eq!(link(&s, rows[4].0, "text").as_deref(), Some(rows[4].1.as_str()));
}

#[test]
fn editing_a_styled_property_detaches_that_style_only() {
    // Docs silent on direct edits (the panel shows the style until it's detached); pinned: a value
    // of the layer's own is no longer the style's, so that link goes (text styles already unlink
    // the chars edited) and later style edits don't overwrite it. Other links and unrelated edits
    // keep the link.
    let mut s = Session::new();
    let rows = styled(&mut s);
    let r = rows[0].0;
    // Moving or renaming the layer keeps every link.
    run(&mut s, "node.set", json!({"id": r, "props": {"x": 25, "name": "Moved"}}));
    for (layer, st, kind) in &rows {
        assert_eq!(link(&s, *layer, kind).as_deref(), Some(st.as_str()), "{kind}: unrelated edit keeps the link");
    }
    let edits: [(&str, Value); 5] = [
        ("paint", json!({"fills": ["#00FFFF"]})),
        ("stroke", json!({"strokes": ["#FFFF00"]})),
        ("effect", json!({"effects": []})),
        ("grid", json!({"layoutGrids": []})),
        ("text", json!({"fontSize": 40})),
    ];
    for (kind, props) in edits {
        let (layer, st, _) = rows.iter().find(|x| x.2 == kind).unwrap().clone();
        run(&mut s, "node.set", json!({"id": layer, "props": props}));
        assert_eq!(link(&s, layer, kind), None, "{kind}: the edited property's link goes");
        let after = shown(&s, layer, kind);
        let value = if kind == "text" {
            json!({"fontSize": 9})
        } else if kind == "paint" || kind == "stroke" {
            json!(["#111111"])
        } else {
            json!(null)
        };
        if !value.is_null() {
            run(&mut s, "style.update", json!({"style": st, "value": value}));
            assert_eq!(shown(&s, layer, kind), after, "{kind}: the edit isn't overwritten by the style");
        }
    }
    // Editing one paint's colour through its row detaches the fill style too (it did already).
    let b = rect(&mut s, 200.0);
    let st = sid(run(&mut s, "style.create", json!({"type": "paint", "name": "P", "from": b})));
    run(&mut s, "fill.set", json!({"ids": [b], "index": 0, "color": "#222222"}));
    assert_eq!(node(&s, b).styles.fill, None);
    let _ = st;
}

#[test]
fn a_style_link_wins_over_values_in_the_same_call() {
    // Docs silent on one call setting both (a linked layer shows its style's values; editing one
    // detaches it); pinned: the link wins whatever the keys are called, on node.create and
    // node.set, and a deviation set in a later call unlinks as before.
    let mut s = Session::new();
    let rows = styled(&mut s);
    let pairs: [(&str, &str, Value); 12] = [
        ("paint", "fill", json!("#123456")),
        ("paint", "fills", json!(["#123456"])),
        ("stroke", "stroke", json!("#123456")),
        ("stroke", "strokes", json!(["#123456"])),
        ("effect", "effects", json!([])),
        ("grid", "grids", json!([])),
        ("grid", "layoutGrids", json!([])),
        ("text", "fontStyle", json!("Bold")),
        ("text", "lineHeight", json!(40)),
        ("text", "textDecoration", json!("underline")),
        ("text", "letterSpacing", json!(5)),
        ("text", "variationSettings", json!({"wght": 700})),
    ];
    for (kind, key, value) in pairs {
        let (source, st, _) = rows.iter().find(|r| r.2 == kind).unwrap().clone();
        let link_key = style_key(kind);
        let props = json!({key: value.clone(), link_key.as_str(): st.clone()});
        let made = id(run(&mut s, "node.create", new_layer(kind, props.clone())));
        let set = if kind == "text" { text(&mut s, 400.0) } else { rect(&mut s, 400.0) };
        run(&mut s, "node.set", json!({"id": set, "props": props}));
        for (how, layer) in [("node.create", made), ("node.set", set)] {
            assert_eq!(link(&s, layer, kind).as_deref(), Some(st.as_str()), "{how} `{key}` + `{link_key}`: linked");
            assert_eq!(shown(&s, layer, kind), shown(&s, source, kind), "{how} `{key}` + `{link_key}`: the style's values");
        }
        run(&mut s, "node.set", json!({"id": set, "props": {key: value}}));
        assert_eq!(link(&s, set, kind), None, "`{key}` in a later call unlinks");
        assert_ne!(shown(&s, set, kind), shown(&s, source, kind), "`{key}` in a later call applies");
    }
}

#[test]
fn a_style_link_leaves_values_its_style_does_not_set() {
    // Docs silent; pinned: alignment given beside a text style applies, on node.create and node.set.
    let mut s = Session::new();
    let st = styled(&mut s).into_iter().find(|r| r.2 == "text").unwrap().1;
    let props = json!({"textStyle": st, "textAlign": "center"});
    let made = id(run(&mut s, "node.create", new_layer("text", props.clone())));
    let set = text(&mut s, 400.0);
    run(&mut s, "node.set", json!({"id": set, "props": props}));
    for (how, layer) in [("node.create", made), ("node.set", set)] {
        assert_eq!(link(&s, layer, "text").as_deref(), Some(st.as_str()), "{how}: linked");
        assert_eq!(node(&s, layer).kind.text().unwrap().align_h, figcraft_doc::TextAlignH::Center, "{how}: `textAlign` applies");
    }
}

#[test]
fn applying_a_style_replaces_variables_bound_to_that_property() {
    // Documented: applying a style sets the property to the style's values. Docs silent on bound
    // variables; pinned: the style's paints replace the bound ones (else the variable would win on
    // the next edit), and binding a variable to a styled paint is an edit (the style detaches).
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "C"}));
    run(&mut s, "variable.create", json!({"collection": "C", "name": "brand", "type": "color", "value": "#0D99FF"}));
    let r = rect(&mut s, 0.0);
    let st = sid(run(&mut s, "style.create", json!({"type": "paint", "name": "Red", "value": ["#FF0000"]})));
    run(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "brand"}));
    assert_eq!(hex(&s, r), ["0D99FF"]);
    run(&mut s, "style.apply", json!({"ids": [r], "style": st}));
    assert_eq!(hex(&s, r), ["FF0000"], "the style's paint shows");
    assert!(node(&s, r).bound.is_empty(), "the fill binding went");
    assert_eq!(node(&s, r).styles.fill.as_deref(), Some(st.as_str()));
    // A later variable edit doesn't reach it.
    run(&mut s, "variable.set", json!({"variable": "brand", "value": "#00FF00"}));
    assert_eq!(hex(&s, r), ["FF0000"]);
    // Binding the variable to the styled fill: the variable's colour shows and the style detaches.
    run(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "brand"}));
    assert_eq!(hex(&s, r), ["00FF00"]);
    assert_eq!(node(&s, r).styles.fill, None);
    // Bindings of other properties stay when a paint style is applied.
    run(&mut s, "variable.create", json!({"collection": "C", "name": "w", "type": "number", "value": 64}));
    run(&mut s, "variable.bind", json!({"ids": [r], "field": "width", "variable": "w"}));
    run(&mut s, "style.apply", json!({"ids": [r], "style": st}));
    assert!(node(&s, r).bound.contains_key("width"));
    assert_eq!(node(&s, r).width, 64.0);
}

#[test]
fn text_styles_go_on_text_layers_and_chars() {
    // Documented: text styles apply to whole text layers or selected text. A text style is no
    // property of other layers: they're left alone; a selection without text is refused.
    let mut s = Session::new();
    let t = text(&mut s, 0.0);
    let r = rect(&mut s, 200.0);
    let st = sid(run(&mut s, "style.create", json!({"type": "text", "name": "Body", "value": {"fontSize": 22}})));
    run(&mut s, "style.apply", json!({"ids": [t, r], "style": st}));
    assert_eq!(font_size(&s, t), 22.0);
    assert_eq!(node(&s, t).styles.text.as_deref(), Some(st.as_str()));
    assert_eq!(node(&s, r).styles.text, None, "a rectangle takes no text style");
    assert!(s.execute("style.apply", &json!({"ids": [r], "style": st})).is_err(), "no text layer to take it");
    // On some chars (#72): only those take it, and the layer shows no single link.
    let t2 = text(&mut s, 300.0);
    run(&mut s, "style.apply", json!({"ids": [t2], "style": st, "start": 0, "end": 2}));
    let tx = node(&s, t2);
    let td = tx.kind.text().unwrap();
    assert_eq!(td.style_at(0).font_size, 22.0);
    assert_ne!(td.style_at(4).font_size, 22.0);
    assert_eq!(tx.styles.text, None, "mixed: the layer isn't linked as a whole");
}

#[test]
fn styles_are_organised_by_slash_names_order_and_duplicates() {
    // Documented: "/" in a name groups styles in folders; moving a style to another folder renames
    // it to match ("Brand/Teal" into Brand/Site → "Brand/Site/Teal"); dragging changes the
    // order; a duplicate lands right under its original. Renaming a folder renames what's in it
    // (the folder is part of each name). Docs silent on the duplicate's name: "<name> copy", as
    // for variables.
    let mut s = Session::new();
    let mk = |s: &mut Session, name: &str| sid(run(s, "style.create", json!({"type": "paint", "name": name, "value": ["#FF00FF"]})));
    let a = mk(&mut s, "Brand/Teal");
    let b = mk(&mut s, "Brand/Site/Ink");
    let c = mk(&mut s, "Gray/100");
    let names = |s: &Session| s.doc().unwrap().styles.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
    // Into a folder: renamed, placed where dropped (before `c`).
    run(&mut s, "style.move", json!({"styles": [a], "folder": "Brand/Site", "before": c}));
    assert_eq!(names(&s), ["Brand/Site/Ink", "Brand/Site/Teal", "Gray/100"]);
    // Reorder only (no folder given): to the top.
    run(&mut s, "style.move", json!({"styles": [c], "before": b}));
    assert_eq!(names(&s), ["Gray/100", "Brand/Site/Ink", "Brand/Site/Teal"]);
    // Out of every folder (folder ""), to the end.
    run(&mut s, "style.move", json!({"styles": [b], "folder": ""}));
    assert_eq!(names(&s), ["Gray/100", "Brand/Site/Teal", "Ink"]);
    // Renaming a folder renames what's in it (and only that: "Brand/Siteplan" isn't inside).
    mk(&mut s, "Brand/Siteplan/X");
    run(&mut s, "style.renameFolder", json!({"folder": "Brand/Site", "name": "Brand/News"}));
    assert_eq!(names(&s), ["Gray/100", "Brand/News/Teal", "Ink", "Brand/Siteplan/X"]);
    // A duplicate sits right below its original, with the same definition and description.
    run(&mut s, "style.update", json!({"style": c, "description": "Lightest"}));
    let d = sid(run(&mut s, "style.duplicate", json!({"style": c})));
    assert_eq!(names(&s)[..2], ["Gray/100".to_string(), "Gray/100 copy".to_string()]);
    let doc = s.doc().unwrap();
    let (orig, copy) = (doc.style(&c).unwrap(), doc.style(&d).unwrap());
    assert_eq!((copy.kind.clone(), copy.description.as_str()), (orig.kind.clone(), "Lightest"));
    // Names are trimmed per folder part; empty names are refused; each move is one undo step.
    run(&mut s, "style.update", json!({"style": d, "name": " Gray / 200 "}));
    assert_eq!(s.doc().unwrap().style(&d).unwrap().name, "Gray/200");
    assert!(s.execute("style.update", &json!({"style": d, "name": " / "})).is_err());
    assert!(s.execute("style.create", &json!({"type": "paint", "name": "", "value": ["#000000"]})).is_err());
    assert!(s.execute("style.move", &json!({"styles": ["nope"]})).is_err());
    let before = names(&s);
    run(&mut s, "style.move", json!({"styles": [d], "before": c}));
    s.undo().unwrap();
    assert_eq!(names(&s), before);
}

#[test]
fn descriptions_and_library_styles() {
    // Documented: styles have descriptions (shown in the picker). Library styles are read-only in
    // files using them (edits happen in the library file; covered by `library_tests`).
    let mut s = Session::new();
    let st = sid(run(&mut s, "style.create", json!({"type": "paint", "name": "A", "value": ["#000000"], "description": "Primary text"})));
    assert_eq!(s.doc().unwrap().style(&st).unwrap().description, "Primary text");
    run(&mut s, "style.update", json!({"style": st, "description": "Body text"}));
    assert_eq!(s.doc().unwrap().style(&st).unwrap().description, "Body text");
}
