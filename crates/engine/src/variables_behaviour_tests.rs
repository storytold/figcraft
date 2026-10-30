//! Variables behaviour suite, written from the public documentation of variables in design
//! tools: types and their default values, collections and modes (new and duplicated modes, the
//! default mode), aliases (same type, chains, no cycles), scopes (pickers only), mode inheritance
//! down the tree, bindings by field and type, deleting bound variables, code syntax and the
//! variables JSON. Each case gives the rule's source in one line; "docs silent" marks behaviour the
//! documentation doesn't describe (pinned as it is).

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::{LineHeight, Node, NodeId, VariableType, VariableValue};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

pub(crate) fn vid(v: Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn node(s: &Session, n: NodeId) -> Node {
    s.doc().unwrap().node(n).unwrap().clone()
}

fn fill(s: &Session, n: NodeId) -> String {
    node(s, n).fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

fn frame(s: &mut Session, parent: Option<NodeId>) -> NodeId {
    id(run(s, "frame.create", json!({"parent": parent, "x": 0, "y": 0, "width": 100, "height": 100})))
}

fn rect(s: &mut Session, parent: Option<NodeId>) -> NodeId {
    id(run(s, "shape.rectangle", json!({"parent": parent, "x": 0, "y": 0, "width": 10, "height": 10})))
}

/// A "Theme" collection with Light and Dark modes and a colour variable `bg` (white / black).
fn theme(s: &mut Session) {
    run(s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    run(s, "variable.create", json!({"collection": "Theme", "name": "bg", "type": "color", "values": {"Light": "#FFFFFF", "Dark": "#000000"}}));
}

#[test]
fn types_and_their_starting_values() {
    // Documented: four types (colour, number, string, boolean). Docs silent on a new variable's
    // value; pinned: black, 0, empty, true. Every mode of the collection gets a value.
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "C", "modes": ["A", "B"]}));
    let cases: [(&str, VariableType, VariableValue); 4] = [
        ("color", VariableType::Color, VariableValue::Color(figcraft_color::Color::BLACK)),
        ("number", VariableType::Number, VariableValue::Number(0.0)),
        ("string", VariableType::String, VariableValue::String(String::new())),
        ("boolean", VariableType::Boolean, VariableValue::Boolean(true)),
    ];
    for (ty, kind, want) in cases {
        let v = vid(run(&mut s, "variable.create", json!({"collection": "C", "name": ty, "type": ty})));
        let var = s.doc().unwrap().variables.get(&v).unwrap().clone();
        assert_eq!(var.kind, kind, "{ty}");
        assert_eq!(var.values.len(), 2, "{ty}: a value per mode");
        assert!(var.values.values().all(|x| *x == want), "{ty}: {:?}", var.values);
    }
    assert!(s.execute("variable.create", &json!({"collection": "C", "name": "x", "type": "date"})).is_err());
    assert!(s.execute("variable.create", &json!({"collection": "C", "name": "y", "type": "number", "value": "wide"})).is_err());
}

#[test]
fn modes_new_duplicated_reordered_and_the_default() {
    // Documented: a new mode starts with the default mode's values and a duplicated one with its
    // source's; the default is the left-most mode (set as default = move it there); layers on
    // Auto follow a new default, layers set to a mode keep theirs.
    let mut s = Session::new();
    theme(&mut s);
    let v = |s: &Session, mode: &str| {
        let doc = s.doc().unwrap();
        let c = doc.variables.collection("Theme").unwrap();
        let m = c.find_mode(mode).unwrap().id.clone();
        doc.variables.get("bg").unwrap().values.get(&m).cloned()
    };
    run(&mut s, "variable.addMode", json!({"collection": "Theme", "name": "Dim"}));
    assert_eq!(v(&s, "Dim"), v(&s, "Light"), "new mode: the first column's values");
    run(&mut s, "variable.addMode", json!({"collection": "Theme", "name": "Dark 2", "from": "Dark"}));
    assert_eq!(v(&s, "Dark 2"), v(&s, "Dark"), "duplicated mode: that mode's values");
    assert!(s.execute("variable.addMode", &json!({"collection": "Theme", "name": "X", "from": "Nope"})).is_err());
    let auto = rect(&mut s, None);
    let pinned_frame = frame(&mut s, None);
    let pinned = rect(&mut s, Some(pinned_frame));
    for r in [auto, pinned] {
        run(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "bg"}));
    }
    run(&mut s, "variable.setMode", json!({"ids": [pinned_frame], "collection": "Theme", "mode": "Light"}));
    assert_eq!((fill(&s, auto), fill(&s, pinned)), ("FFFFFF".into(), "FFFFFF".into()));
    // Dark becomes the default (moved to the first column).
    run(&mut s, "variable.moveMode", json!({"collection": "Theme", "mode": "Dark", "to": 0}));
    let order: Vec<String> = s.doc().unwrap().variables.collection("Theme").unwrap().modes.iter().map(|m| m.name.clone()).collect();
    assert_eq!(order, ["Dark", "Light", "Dim", "Dark 2"]);
    assert_eq!(fill(&s, auto), "000000", "Auto follows the new default");
    assert_eq!(fill(&s, pinned), "FFFFFF", "an explicit mode stays");
    // Moving right past the end clamps; one undo step per move.
    run(&mut s, "variable.moveMode", json!({"collection": "Theme", "mode": "Dark", "to": 99}));
    assert_eq!(s.doc().unwrap().variables.collection("Theme").unwrap().modes.last().unwrap().name, "Dark");
    assert_eq!(fill(&s, auto), "FFFFFF");
    s.undo().unwrap();
    assert_eq!(fill(&s, auto), "000000");
    assert!(s.execute("variable.moveMode", &json!({"collection": "Theme", "mode": "Nope", "to": 0})).is_err());
    // Deleting a mode a frame is set to: it goes back to Auto (docs silent; pinned).
    run(&mut s, "variable.removeMode", json!({"collection": "Theme", "mode": "Light"}));
    assert!(node(&s, pinned_frame).var_modes.is_empty());
    assert_eq!(fill(&s, pinned), "000000");
}

#[test]
fn aliases_same_type_chains_and_no_cycles() {
    // Documented: an alias points at a variable of the same type. Docs silent on loops; pinned:
    // refused (a value must resolve), also when creating with an alias.
    let mut s = Session::new();
    theme(&mut s);
    run(&mut s, "variable.createCollection", json!({"name": "Brand", "modes": ["Base"]}));
    run(&mut s, "variable.create", json!({"collection": "Brand", "name": "blue", "type": "color", "value": "#0000FF"}));
    run(&mut s, "variable.create", json!({"collection": "Brand", "name": "n", "type": "number", "value": 4}));
    run(&mut s, "variable.create", json!({"collection": "Theme", "name": "accent", "type": "color", "value": {"alias": "blue"}}));
    run(&mut s, "variable.create", json!({"collection": "Theme", "name": "link", "type": "color", "value": {"alias": "accent"}}));
    let r = rect(&mut s, None);
    run(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "link"}));
    assert_eq!(fill(&s, r), "0000FF", "a chain link → accent → blue resolves");
    run(&mut s, "variable.set", json!({"variable": "blue", "value": "#00FF00"}));
    assert_eq!(fill(&s, r), "00FF00", "the chain follows the end of it");
    let refused: [(&str, Value, &str); 5] = [
        ("variable.set", json!({"variable": "blue", "value": {"alias": "link"}}), "a loop through the chain"),
        ("variable.set", json!({"variable": "accent", "value": {"alias": "accent"}}), "itself"),
        ("variable.set", json!({"variable": "accent", "value": {"alias": "n"}}), "another type"),
        ("variable.create", json!({"collection": "Theme", "name": "x", "type": "color", "value": {"alias": "n"}}), "create: another type"),
        ("variable.create", json!({"collection": "Theme", "name": "y", "type": "color", "value": {"alias": "missing"}}), "create: no such variable"),
    ];
    for (cmd, p, what) in refused {
        assert!(s.execute(cmd, &p).is_err(), "{what}");
    }
    assert!(s.doc().unwrap().variables.get("x").is_none() && s.doc().unwrap().variables.get("y").is_none());
}

#[test]
fn scopes_limit_pickers_only() {
    // Documented: scoping only changes which variables pickers offer; the scope lists per type
    // (number: gap and padding, corner radius, font weight/size/line height/letter spacing/
    // paragraph spacing and indent, opacity, effects, stroke, text content, width and height;
    // colour: frame/shape/text fill, stroke, effects; string: font family, style, text content).
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "C"}));
    run(&mut s, "variable.create", json!({"collection": "C", "name": "radius", "type": "number", "value": 8}));
    run(&mut s, "variable.update", json!({"variable": "radius", "scopes": ["CORNER_RADIUS"]}));
    let r = rect(&mut s, None);
    let doc = s.doc().unwrap();
    let var = doc.variables.get("radius").unwrap();
    let picker = |field: &str| crate::cmd::variables::field_scope(doc, r, field).is_none_or(|sc| var.in_scope(sc));
    assert!(picker("cornerRadius"));
    assert!(!picker("width"), "out of scope: not offered for width");
    // Binding out of scope still works (scopes are about pickers).
    run(&mut s, "variable.bind", json!({"ids": [r], "field": "width", "variable": "radius"}));
    assert_eq!(node(&s, r).width, 8.0);
    let number = figcraft_doc::scopes_for(VariableType::Number).iter().map(|(k, _)| *k).collect::<Vec<_>>();
    for want in [
        "GAP",
        "CORNER_RADIUS",
        "FONT_WEIGHT",
        "FONT_SIZE",
        "LINE_HEIGHT",
        "LETTER_SPACING",
        "PARAGRAPH_SPACING",
        "PARAGRAPH_INDENT",
        "OPACITY",
        "EFFECT_FLOAT",
        "STROKE_FLOAT",
        "TEXT_CONTENT",
        "WIDTH_HEIGHT",
    ] {
        assert!(number.contains(&want), "number scope {want}");
    }
    let doc = s.doc().unwrap();
    let t = figcraft_doc::NodeId(0);
    let fields: [(&str, &str); 3] =
        [("fontWeight", "FONT_WEIGHT"), ("paragraphSpacing", "PARAGRAPH_SPACING"), ("paragraphIndent", "PARAGRAPH_INDENT")];
    for (field, scope) in fields {
        assert_eq!(crate::cmd::variables::field_scope(doc, t, field), Some(scope), "{field}");
    }
    // Unknown scope ids are refused.
    assert!(s.execute("variable.update", &json!({"variable": "radius", "scopes": ["TEXT_FILL"]})).is_err());
}

#[test]
fn modes_inherit_down_the_tree() {
    // Documented: layers are on Auto by default and take the nearest container's mode (frames,
    // groups, sections, components, pages); with none set, the collection's default mode.
    let mut s = Session::new();
    theme(&mut s);
    let page = s.page().unwrap();
    let outer = frame(&mut s, None);
    let inner = frame(&mut s, Some(outer));
    let r = rect(&mut s, Some(inner));
    let free = rect(&mut s, None);
    for x in [r, free] {
        run(&mut s, "variable.bind", json!({"ids": [x], "field": "fill", "variable": "bg"}));
    }
    fn set(s: &mut Session, ids: Value, mode: Value) {
        run(s, "variable.setMode", json!({"ids": ids, "collection": "Theme", "mode": mode}));
    }
    // A step: what it does to (page, outer, inner).
    type Step = fn(&mut Session, NodeId, NodeId, NodeId);
    // (step, expected nested, expected loose)
    let steps: [(&str, Step, &str, &str); 5] = [
        ("nothing set: default mode", |_, _, _, _| {}, "FFFFFF", "FFFFFF"),
        ("page Dark: everything on it", |s, p, _, _| set(s, json!([p]), json!("Dark")), "000000", "000000"),
        ("outer Light: its subtree", |s, _, o, _| set(s, json!([o]), json!("Light")), "FFFFFF", "000000"),
        ("inner Dark: the nearest wins", |s, _, _, i| set(s, json!([i]), json!("Dark")), "000000", "000000"),
        ("inner back to Auto: outer's again", |s, _, _, i| set(s, json!([i]), Value::Null), "FFFFFF", "000000"),
    ];
    for (what, step, nested, loose) in steps {
        step(&mut s, page, outer, inner);
        assert_eq!((fill(&s, r).as_str(), fill(&s, free).as_str()), (nested, loose), "{what}");
    }
    // Moving a layer into another container: it takes that one's mode.
    run(&mut s, "node.reparent", json!({"ids": [free], "parent": outer}));
    assert_eq!(fill(&s, free), "FFFFFF");
}

#[test]
fn bindings_by_field_and_type() {
    // Documented: which properties each type applies to (number: sizes, gaps, padding, radius,
    // opacity, font properties, effects, strokes, text content; colour: fills, strokes, effects;
    // string: text content, font family/style; boolean: visibility). Opacity numbers are percent
    // (as the field shows); other numbers are pixels. A variable of a type the field doesn't take
    // is refused (docs: a variable applies only to properties of its type).
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "C"}));
    for (name, ty, value) in
        [("n", "number", json!(12)), ("c", "color", json!("#336699")), ("t", "string", json!("Hi")), ("b", "boolean", json!(false))]
    {
        run(&mut s, "variable.create", json!({"collection": "C", "name": name, "type": ty, "value": value}));
    }
    let f = frame(&mut s, None);
    run(&mut s, "layout.add", json!({"ids": [f]}));
    let r = rect(&mut s, None);
    run(&mut s, "node.set", json!({"id": r, "props": {"strokes": ["#000000"], "strokeWeight": 1}}));
    let t = id(run(&mut s, "text.create", json!({"text": "Hello", "x": 0, "y": 200})));
    type Check = fn(&Node) -> bool;
    let ok: [(NodeId, &str, &str, Check); 14] = [
        (r, "width", "n", |n| n.width == 12.0),
        (r, "height", "n", |n| n.height == 12.0),
        (r, "cornerRadius", "n", |n| n.corners.uniform() == Some(12.0)),
        (r, "opacity", "n", |n| (n.opacity - 0.12).abs() < 1e-5),
        (r, "strokeWeight", "n", |n| n.stroke.weight == 12.0),
        (f, "gap", "n", |n| n.layout.as_ref().is_some_and(|l| l.gap == 12.0)),
        (f, "paddingTop", "n", |n| n.layout.as_ref().is_some_and(|l| l.padding[0] == 12.0)),
        (t, "fontSize", "n", |n| n.kind.text().is_some_and(|x| x.style.font_size == 12.0)),
        (t, "lineHeight", "n", |n| n.kind.text().is_some_and(|x| x.style.line_height == LineHeight::Pixels(12.0))),
        (t, "paragraphSpacing", "n", |n| n.kind.text().is_some_and(|x| x.paragraph_spacing == 12.0)),
        (r, "fill", "c", |n| n.fills.first().and_then(|p| p.solid_color()).is_some_and(|c| c.to_hex() == "336699")),
        (r, "strokes/0/color", "c", |n| n.strokes.first().and_then(|p| p.solid_color()).is_some_and(|c| c.to_hex() == "336699")),
        (t, "characters", "t", |n| n.kind.text().is_some_and(|x| x.characters == "Hi")),
        (t, "visible", "b", |n| !n.visible),
    ];
    for (layer, field, var, check) in ok {
        run(&mut s, "variable.bind", json!({"ids": [layer], "field": field, "variable": var}));
        assert!(check(&node(&s, layer)), "{field} ← {var}");
    }
    // Number variables in text content show as text (documented: number → text content).
    let t2 = id(run(&mut s, "text.create", json!({"text": "x", "x": 0, "y": 300})));
    run(&mut s, "variable.bind", json!({"ids": [t2], "field": "characters", "variable": "n"}));
    assert_eq!(node(&s, t2).kind.text().map(|x| x.characters.clone()).as_deref(), Some("12"));
    let refused: [(NodeId, &str, &str); 6] =
        [(r, "width", "c"), (r, "fill", "n"), (r, "opacity", "t"), (t, "visible", "n"), (t, "characters", "c"), (t, "fontFamily", "n")];
    for (layer, field, var) in refused {
        let before = node(&s, layer).bound.get(field).cloned();
        assert!(s.execute("variable.bind", &json!({"ids": [layer], "field": field, "variable": var})).is_err(), "{field} ← {var} refused");
        assert_eq!(node(&s, layer).bound.get(field).cloned(), before, "{field}: binding unchanged");
    }
}

#[test]
fn auto_layout_bindings_go_with_the_auto_layout() {
    // Docs silent; pinned (#810): removing a frame's auto layout drops its gap and padding bindings,
    // and a binding never brings an auto layout back. One made without auto layout waits for it.
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "C"}));
    run(&mut s, "variable.create", json!({"collection": "C", "name": "g", "type": "number", "value": 12}));
    let removals =
        [("layout.remove", json!({})), ("node.set", json!({"props": {"layout": null}})), ("node.set", json!({"props": {"layout": "none"}}))];
    for (cmd, mut p) in removals {
        let f = frame(&mut s, None);
        run(&mut s, "layout.add", json!({"ids": [f]}));
        for field in ["gap", "crossGap", "paddingTop"] {
            run(&mut s, "variable.bind", json!({"ids": [f], "field": field, "variable": "g"}));
        }
        p["ids"] = json!([f]);
        run(&mut s, cmd, p.clone());
        let n = node(&s, f);
        assert!(n.layout.is_none(), "{cmd} {p}: auto layout stays removed");
        assert!(n.bound.is_empty(), "{cmd} {p}: layout bindings dropped");
    }
    let f = frame(&mut s, None);
    run(&mut s, "variable.bind", json!({"ids": [f], "field": "gap", "variable": "g"}));
    assert!(node(&s, f).layout.is_none(), "binding gap adds no auto layout");
    run(&mut s, "layout.add", json!({"ids": [f]}));
    assert_eq!(node(&s, f).layout.map(|l| l.gap), Some(12.0), "the waiting binding applies to the new auto layout");
}

#[test]
fn deleting_a_bound_variable_keeps_the_last_value() {
    // Documented: deleting a variable unbinds what used it, which keeps showing its last value.
    // Docs silent on variables aliasing it; pinned: they take its values too.
    let mut s = Session::new();
    theme(&mut s);
    run(&mut s, "variable.create", json!({"collection": "Theme", "name": "surface", "type": "color", "value": {"alias": "bg"}}));
    let f = frame(&mut s, None);
    let r = rect(&mut s, Some(f));
    let r2 = rect(&mut s, Some(f));
    run(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "bg"}));
    run(&mut s, "variable.bind", json!({"ids": [r2], "field": "fill", "variable": "surface"}));
    run(&mut s, "variable.setMode", json!({"ids": [f], "collection": "Theme", "mode": "Dark"}));
    run(&mut s, "variable.delete", json!({"variable": "bg"}));
    assert_eq!(fill(&s, r), "000000", "the last value stays");
    assert!(node(&s, r).bound.is_empty(), "no longer connected");
    let doc = s.doc().unwrap();
    let surface = doc.variables.get("surface").unwrap();
    let (light, dark) = (&doc.variables.collection("Theme").unwrap().modes[0].id, &doc.variables.collection("Theme").unwrap().modes[1].id);
    assert_eq!(surface.values.get(light).map(VariableValue::display).as_deref(), Some("FFFFFF"), "alias → its value per mode");
    assert_eq!(surface.values.get(dark).map(VariableValue::display).as_deref(), Some("000000"));
    assert_eq!(fill(&s, r2), "000000");
    // Undo brings the variable and the binding back.
    s.undo().unwrap();
    assert!(node(&s, r).bound.contains_key("fill"));
    // Deleting the collection: the same for layers, and aliases from other collections into it.
    run(&mut s, "variable.createCollection", json!({"name": "Other"}));
    run(&mut s, "variable.create", json!({"collection": "Other", "name": "o", "type": "color", "value": {"alias": "bg"}}));
    run(&mut s, "variable.deleteCollection", json!({"collection": "Theme"}));
    assert_eq!(fill(&s, r), "000000");
    assert!(node(&s, r).bound.is_empty());
    let o = s.doc().unwrap().variables.get("o").unwrap().clone();
    assert!(matches!(o.values.values().next(), Some(VariableValue::Color(c)) if c.to_hex() == "FFFFFF"), "{:?}", o.values);
}

#[test]
fn swiftui_and_compose_name_variables_by_their_code_syntax() {
    // Documented: Dev Mode's code snippets show a bound variable's code syntax for the platform,
    // in CSS, SwiftUI and Compose. A variable without one keeps the value: no made-up name.
    use crate::codegen::{CodeLang, code_for};
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "Tokens"}));
    let names = [("brand", "color", json!("#0d99ff"), "Color.brand", "AppColors.brand"), ("space", "number", json!(16), "Spacing.md", "Spacing.md")];
    for (name, kind, value, ios, android) in names {
        run(&mut s, "variable.create", json!({"collection": "Tokens", "name": name, "type": kind, "value": value}));
        run(&mut s, "variable.update", json!({"variable": name, "codeSyntax": {"iOS": ios, "ANDROID": android}}));
    }
    run(&mut s, "variable.create", json!({"collection": "Tokens", "name": "web only", "type": "number", "value": 9}));
    run(&mut s, "variable.update", json!({"variable": "web only", "codeSyntax": {"WEB": "--r", "iOS": "\u{7}"}}));

    let f = frame(&mut s, None);
    run(&mut s, "layout.add", json!({"ids": [f]}));
    let bind = |s: &mut Session, id: NodeId, field: &str, variable: &str| {
        run(s, "variable.bind", json!({"ids": [id], "field": field, "variable": variable}));
    };
    for (field, variable) in [("gap", "space"), ("padding", "space"), ("fill", "brand"), ("cornerRadius", "web only")] {
        bind(&mut s, f, field, variable);
    }
    let code = |s: &Session, lang| code_for(s.doc().unwrap(), f, lang);
    let swift = code(&s, CodeLang::SwiftUi);
    assert!(swift.contains("spacing: Spacing.md"), "{swift}");
    assert!(swift.contains("EdgeInsets(top: Spacing.md, leading: Spacing.md"), "{swift}");
    assert!(swift.contains(".background(Color.brand)"), "{swift}");
    assert!(swift.contains("RoundedRectangle(cornerRadius: 9)"), "a name of nothing keeps the value: {swift}");
    assert!(!swift.contains("Color(red"), "{swift}");
    let kotlin = code(&s, CodeLang::Compose);
    assert!(kotlin.contains("Arrangement.spacedBy(Spacing.md)"), "{kotlin}");
    assert!(kotlin.contains(".padding(start = Spacing.md, top = Spacing.md"), "{kotlin}");
    assert!(kotlin.contains(".background(AppColors.brand, "), "{kotlin}");
    assert!(kotlin.contains("RoundedCornerShape(9.dp)"), "no Android name: {kotlin}");

    // Shapes and text too: sizes, strokes, the font and its colour.
    let r = crate::params::id_of(&run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40, "parent": f}))["id"]).unwrap();
    run(&mut s, "stroke.add", json!({"ids": [r], "color": "#000000"}));
    for (field, variable) in [("width", "space"), ("height", "space"), ("stroke", "brand"), ("strokeWeight", "space")] {
        bind(&mut s, r, field, variable);
    }
    let t = crate::params::id_of(&run(&mut s, "text.create", json!({"text": "Hi", "x": 0, "y": 0, "parent": f}))["id"]).unwrap();
    bind(&mut s, t, "fontSize", "space");
    bind(&mut s, t, "fill", "brand");
    let swift = code(&s, CodeLang::SwiftUi);
    assert!(swift.contains(".frame(width: Spacing.md, height: Spacing.md)"), "{swift}");
    assert!(swift.contains(".stroke(Color.brand, lineWidth: Spacing.md)"), "{swift}");
    assert!(swift.contains("size: Spacing.md)") && swift.contains(".foregroundColor(Color.brand)"), "{swift}");
    let kotlin = code(&s, CodeLang::Compose);
    assert!(kotlin.contains("Modifier.size(Spacing.md, Spacing.md)"), "{kotlin}");
    assert!(kotlin.contains(".border(Spacing.md, AppColors.brand, "), "{kotlin}");
    assert!(kotlin.contains("fontSize = Spacing.md,") && kotlin.contains("color = AppColors.brand,"), "{kotlin}");
    // Code for another platform doesn't take iOS or Android names.
    for lang in [CodeLang::Flutter, CodeLang::Css] {
        let other = code(&s, lang);
        assert!(!other.contains("Spacing.md") && !other.contains("AppColors"), "{other}");
    }
}

#[test]
fn code_syntax_per_platform() {
    // Documented: a variable can have a code name per platform (Web, iOS, Android) for handoff
    // code; a Web one can be written as a CSS `var()` reference.
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "Spacing"}));
    run(&mut s, "variable.create", json!({"collection": "Spacing", "name": "space/xs", "type": "number", "value": 4}));
    run(
        &mut s,
        "variable.update",
        json!({"variable": "space/xs", "codeSyntax": {"WEB": "var(--space-tight)", "iOS": "Spacing.tight", "ANDROID": "spacing_xs"}}),
    );
    let var = s.doc().unwrap().variables.get("space/xs").unwrap().clone();
    assert_eq!(var.code_syntax.get("WEB").map(String::as_str), Some("var(--space-tight)"));
    assert_eq!(var.code_syntax.len(), 3);
    // Unknown platforms are refused; an empty name clears one.
    assert!(s.execute("variable.update", &json!({"variable": "space/xs", "codeSyntax": {"Windows": "x"}})).is_err());
    run(&mut s, "variable.update", json!({"variable": "space/xs", "codeSyntax": {"ANDROID": ""}}));
    assert_eq!(s.doc().unwrap().variables.get("space/xs").unwrap().code_syntax.len(), 2);
    // Generated CSS uses the Web name.
    let f = frame(&mut s, None);
    run(&mut s, "layout.add", json!({"ids": [f]}));
    run(&mut s, "variable.bind", json!({"ids": [f], "field": "gap", "variable": "space/xs"}));
    let css = crate::codegen::code_for(s.doc().unwrap(), f, crate::codegen::CodeLang::Css);
    assert!(css.contains("var(--space-tight, 4px)"), "{css}");
    // The variables JSON carries it both ways.
    let out = run(&mut s, "variable.export", json!({"format": "json"}));
    let text = out["text"].as_str().map(str::to_string).unwrap_or_else(|| out.to_string());
    assert!(text.contains("Spacing.tight"), "{text}");
    let mut s2 = Session::new();
    run(&mut s2, "variable.import", json!({"variables": text}));
    let back = s2.doc().unwrap().variables.get("space/xs").unwrap().code_syntax.clone();
    assert_eq!(back, var.code_syntax.into_iter().filter(|(k, _)| k != "ANDROID").collect());
}

#[test]
fn number_variables_are_pixels_and_edits_are_one_undo_step() {
    // Documented: number variables hold plain numbers; sizes, gaps and type sizes take them as
    // pixels. Each edit in the editor is one undo step.
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "C"}));
    run(&mut s, "variable.create", json!({"collection": "C", "name": "lh", "type": "number", "value": 20}));
    let t = id(run(&mut s, "text.create", json!({"text": "Hello", "x": 0, "y": 0})));
    run(&mut s, "variable.bind", json!({"ids": [t], "field": "lineHeight", "variable": "lh"}));
    let lh = |s: &Session| node(s, t).kind.text().unwrap().style.line_height;
    assert_eq!(lh(&s), LineHeight::Pixels(20.0));
    run(&mut s, "variable.set", json!({"variable": "lh", "value": 28}));
    assert_eq!(lh(&s), LineHeight::Pixels(28.0));
    s.undo().unwrap();
    assert_eq!(lh(&s), LineHeight::Pixels(20.0), "the value and the bound layer go back together");
}

#[test]
fn create_collection_keeps_at_least_one_named_mode() {
    // A collection keeps at least one mode (removeMode) and mode names can't be empty (renameMode),
    // so createCollection refuses an empty mode list, blank mode names and a blank collection name.
    let mut s = Session::new();
    for p in [
        json!({"name": "C", "modes": []}),
        json!({"name": "C", "modes": [1]}),
        json!({"name": "C", "modes": ["", "  "]}),
        json!({"name": "C", "modes": ["A", " "]}),
        json!({"name": "  "}),
        json!({"name": ""}),
    ] {
        assert!(s.execute("variable.createCollection", &p).is_err(), "{p} should be refused");
    }
    assert!(s.doc().unwrap().variables.collections.is_empty(), "a refused call creates nothing");
    run(&mut s, "variable.createCollection", json!({"name": "C", "modes": [" Light ", "Dark"]}));
    let names: Vec<String> = s.doc().unwrap().variables.collections[0].modes.iter().map(|m| m.name.clone()).collect();
    assert_eq!(names, ["Light", "Dark"]);
}

#[test]
fn explicit_edits_of_bound_fields_stick_and_detach_that_binding() {
    // #809: an edit of a bound field detaches its binding (as a paint edit does) instead of
    // the variable's value coming back; bindings whose value the edit kept stay.
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "C"}));
    let w = vid(run(&mut s, "variable.create", json!({"collection": "C", "name": "w", "type": "number", "value": 48})));
    let o = vid(run(&mut s, "variable.create", json!({"collection": "C", "name": "o", "type": "number", "value": 40})));
    let r = rect(&mut s, None);
    for (field, v) in [("width", &w), ("height", &w), ("opacity", &o)] {
        run(&mut s, "variable.bind", json!({"ids": [r], "field": field, "variable": v}));
    }
    assert_eq!((node(&s, r).width, node(&s, r).opacity), (48.0, 0.4));

    run(&mut s, "node.set", json!({"ids": [r], "width": 100, "opacity": 0.4}));
    let n = node(&s, r);
    assert_eq!(n.width, 100.0);
    assert!(!n.bound.contains_key("width"));
    assert!(n.bound.contains_key("height") && n.bound.contains_key("opacity"), "unchanged values keep their bindings");

    run(&mut s, "transform.resize", json!({"ids": [r], "height": 30}));
    assert_eq!(node(&s, r).height, 30.0);
    assert!(!node(&s, r).bound.contains_key("height"));

    run(&mut s, "object.opacityKeys", json!({"ids": [r], "digits": "9"}));
    assert_eq!(node(&s, r).opacity, 0.9);
    assert!(node(&s, r).bound.is_empty());

    // A value set with its binding in the same call keeps the new binding.
    run(&mut s, "node.set", json!({"ids": [r], "width": 5, "boundVariables": {"width": w}}));
    assert_eq!((node(&s, r).width, node(&s, r).bound.get("width")), (48.0, Some(&w)));
}

#[test]
fn a_gesture_or_a_text_style_on_a_bound_field_detaches_it() {
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "C"}));
    let v = vid(run(&mut s, "variable.create", json!({"collection": "C", "name": "r", "type": "number", "value": 8})));
    let r = rect(&mut s, None);
    run(&mut s, "variable.bind", json!({"ids": [r], "field": "cornerRadius", "variable": v}));
    s.begin_interaction("Edit").unwrap();
    let props = serde_json::Map::from_iter([("cornerRadius".to_string(), json!(2))]);
    s.preview(|d, _| crate::cmd::props::apply("node.set", d, r, &props)).unwrap();
    s.commit_interaction().unwrap();
    assert_eq!(node(&s, r).corners.radii, [2.0; 4]);
    assert!(node(&s, r).bound.is_empty());

    let t = id(run(&mut s, "text.create", json!({"x": 0, "y": 0, "text": "Hi"})));
    run(&mut s, "variable.bind", json!({"ids": [t], "field": "fontSize", "variable": v}));
    run(&mut s, "text.style", json!({"id": t, "fontSize": 20}));
    assert!(node(&s, t).bound.is_empty());
    assert_eq!(node(&s, t).kind.text().unwrap().style.font_size, 20.0);

    // The font size/weight/tracking shortcuts and text content edits detach too.
    run(&mut s, "variable.bind", json!({"ids": [t], "field": "fontSize", "variable": v}));
    assert_eq!(node(&s, t).kind.text().unwrap().style.font_size, 8.0);
    run(&mut s, "text.sizeUp", json!({"ids": [t]}));
    assert_eq!(node(&s, t).kind.text().unwrap().style.font_size, 9.0);
    assert!(node(&s, t).bound.is_empty());
    let c = vid(run(&mut s, "variable.create", json!({"collection": "C", "name": "c", "type": "string", "value": "Hello"})));
    run(&mut s, "variable.bind", json!({"ids": [t], "field": "characters", "variable": c}));
    run(&mut s, "text.set", json!({"id": t, "characters": "Bye"}));
    assert_eq!(node(&s, t).kind.text().unwrap().characters, "Bye");
    assert!(node(&s, t).bound.is_empty());
}

#[test]
fn a_translucent_variable_bound_to_an_empty_fill_applies_its_alpha_once() {
    // #1514: the first paint added by a binding must not take the colour's alpha as opacity too.
    let mut s = Session::new();
    run(&mut s, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    run(
        &mut s,
        "variable.create",
        json!({"collection": "Theme", "name": "tint", "type": "color", "values": {"Light": "#FF000080", "Dark": "#00FF0040"}}),
    );
    let r = id(run(&mut s, "node.create", json!({"type": "rectangle", "props": {"fills": []}})));
    run(&mut s, "variable.bind", json!({"ids": [r], "field": "fill", "variable": "tint"}));
    assert_eq!(node(&s, r).fills[0].opacity, 1.0);
    run(&mut s, "variable.setMode", json!({"collection": "Theme", "mode": "Dark", "ids": [r]}));
    assert_eq!(node(&s, r).fills[0].solid_color().unwrap().to_css_hex(), "#00FF0040");
    assert_eq!(node(&s, r).fills[0].opacity, 1.0);
}

#[test]
fn stored_variable_ids_cannot_hide_an_alias_cycle() {
    for link in ["a", "b"] {
        let mut s = Session::new();
        run(&mut s, "variable.createCollection", json!({"name": "Theme"}));
        let mut ids = std::collections::BTreeMap::new();
        for name in ["shadow", "a", "b"] {
            let v = run(&mut s, "variable.create", json!({"collection": "Theme", "name": name, "type": "number", "value": 1}));
            ids.insert(name, v["id"].clone());
        }
        run(&mut s, "variable.set", json!({"variable": "b", "value": {"alias": "a"}}));
        run(&mut s, "variable.update", json!({"variable": ids["shadow"], "name": ids[link]}));
        let before = s.doc().unwrap().clone();
        let undo = s.ds().unwrap().history.undo.len();
        let error = s.execute("variable.set", &json!({"variable": "a", "value": {"alias": "b"}})).unwrap_err();
        assert!(error.to_string().contains("refer back to itself"), "{link}: {error}");
        assert_eq!(s.doc().unwrap(), &before);
        assert_eq!(s.ds().unwrap().history.undo.len(), undo);
    }
}
