//! Variables, shared styles and instance sizes in exported `.fig` files: collections with their
//! modes, variables with a value per mode, the bindings on layers, paints, gradient stops, effects
//! and layout grids, the modes frames and pages set, style definitions and the links to them, and
//! instances that keep their own size (resized, filling an auto layout, scaled) with their layers
//! laid out for it. Everything written reads back; what the format can't say is written as its
//! value with a warning; damaged files never panic.

use figcraft_doc::{Collection, Mode, Style, StyleKind, Variable, VariableType, VariableValue};
use serde_json::json;

use super::*;

fn solid(r: f32, g: f32, b: f32) -> Paint {
    Paint::solid(Color::new(r, g, b, 1.0))
}

fn var(id: &str, name: &str, collection: &str, kind: VariableType, values: &[(&str, VariableValue)]) -> Variable {
    Variable {
        id: id.into(),
        name: name.into(),
        collection: collection.into(),
        kind,
        values: values.iter().map(|(m, v)| (m.to_string(), v.clone())).collect(),
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    }
}

fn add(doc: &mut Document, parent: NodeId, kind: NodeKind, name: &str, at: (f64, f64), size: (f64, f64), edit: &dyn Fn(&mut Node)) -> NodeId {
    let id = doc.create(parent, kind, name, at.0, at.1, size.0, size.1).unwrap();
    let n = doc.node_mut(id).unwrap();
    n.fills = vec![solid(1.0, 1.0, 1.0)];
    n.strokes.clear();
    edit(n);
    id
}

/// Two collections (one with two modes), variables of every type (one an alias, some with scopes
/// and a description), and layers bound to them in every way the format has a field for.
fn themed() -> Document {
    use VariableValue::{Alias, Boolean, Number};
    let mut doc = Document::new("Themed");
    let page = doc.first_page().unwrap();
    let mode = |id: &str, name: &str| Mode { id: id.into(), name: name.into() };
    doc.variables.collections = vec![
        Collection { id: "c-theme".into(), name: "Theme".into(), modes: vec![mode("m-light", "Light"), mode("m-dark", "Dark")], extends: None },
        Collection { id: "c-space".into(), name: "Spacing".into(), modes: vec![mode("m-only", "Default")], extends: None },
    ];
    let color = |r: f32, g: f32, b: f32| VariableValue::Color(Color::new(r, g, b, 1.0));
    let mut bg = var("v-bg", "color/bg", "c-theme", VariableType::Color, &[("m-light", color(1.0, 1.0, 1.0)), ("m-dark", color(0.0, 0.125, 0.25))]);
    bg.description = "Behind everything".into();
    bg.scopes = Some(vec!["FRAME_FILL".into(), "SHAPE_FILL".into()]);
    let mut fg = var("v-fg", "color/fg", "c-theme", VariableType::Color, &[("m-light", color(0.0, 0.0, 0.0)), ("m-dark", color(0.75, 0.75, 0.75))]);
    fg.scopes = Some(vec!["TEXT_FILL".into(), "STROKE_COLOR".into(), "EFFECT_COLOR".into()]);
    let mut shown = var("v-shown", "shown", "c-theme", VariableType::Boolean, &[("m-light", Boolean(true)), ("m-dark", Boolean(false))]);
    shown.scopes = Some(Vec::new());
    doc.variables.variables = vec![
        // Out of collection order on purpose: the order of variables is the document's.
        var("v-gap", "space/gap", "c-space", VariableType::Number, &[("m-only", Number(8.0))]),
        bg,
        fg,
        var("v-accent", "color/accent", "c-theme", VariableType::Color, &[("m-light", Alias("v-fg".into())), ("m-dark", color(1.0, 0.5, 0.0))]),
        var("v-radius", "radius", "c-space", VariableType::Number, &[("m-only", Number(12.5))]),
        var(
            "v-title",
            "title",
            "c-theme",
            VariableType::String,
            &[("m-light", VariableValue::String("Hello".into())), ("m-dark", VariableValue::String("Grüße 😀".into()))],
        ),
        shown,
    ];
    doc.node_mut(page).unwrap().var_modes.insert("c-space".into(), "m-only".into());
    let bind = |n: &mut Node, pairs: &[(&str, &str)]| n.bound.extend(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let frame = add(&mut doc, page, NodeKind::Frame, "Card", (0.0, 0.0), (300.0, 200.0), &|n| {
        n.layout = Some(AutoLayout::default());
        n.strokes = vec![solid(0.0, 0.0, 0.0)];
        n.stroke.sides = Some([2.0, 1.0, 1.0, 1.0]);
        n.grids = vec![LayoutGrid { kind: GridKind::Columns, ..LayoutGrid::default() }];
        n.var_modes.insert("c-theme".into(), "m-dark".into());
        bind(
            n,
            &[
                ("fill", "v-bg"),
                ("stroke", "v-fg"),
                ("gap", "v-gap"),
                ("crossGap", "v-gap"),
                ("paddingHorizontal", "v-gap"),
                ("paddingTop", "v-radius"),
                ("cornerRadius", "v-radius"),
                ("width", "v-gap"),
                ("minHeight", "v-gap"),
                ("opacity", "v-gap"),
                ("strokeTopWeight", "v-gap"),
                ("layoutGrids/0/gutter", "v-gap"),
                ("layoutGrids/0/count", "v-radius"),
            ],
        );
    });
    add(&mut doc, frame, NodeKind::Rectangle, "Swatch", (10.0, 10.0), (80.0, 60.0), &|n| {
        let mut g = Gradient::default_for(GradientKind::Linear, Color::BLACK);
        g.stops = vec![ColorStop { position: 0.0, color: Color::BLACK }, ColorStop { position: 1.0, color: Color::WHITE }];
        n.fills = vec![
            solid(0.5, 0.5, 0.5),
            Paint { kind: PaintKind::Gradient(g), opacity: 1.0, visible: true, blend: BlendMode::Normal, color_alpha: 1.0 },
        ];
        n.strokes = vec![solid(0.0, 0.0, 0.0), solid(1.0, 0.0, 0.0)];
        n.effects = vec![Effect {
            kind: EffectKind::DropShadow(Shadow {
                color: Color::new(0.0, 0.0, 0.0, 0.25),
                offset: (0.0, 4.0),
                radius: 8.0,
                spread: 0.0,
                blend: BlendMode::Normal,
                show_behind: false,
            }),
            visible: true,
        }];
        bind(
            n,
            &[
                ("fills/0/color", "v-accent"),
                ("fills/1/stops/1/color", "v-bg"),
                ("strokes/1/color", "v-fg"),
                ("effects/0/color", "v-fg"),
                ("effects/0/radius", "v-gap"),
                ("effects/0/spread", "v-gap"),
                ("effects/0/offsetX", "v-gap"),
                ("effects/0/offsetY", "v-radius"),
                ("visible", "v-shown"),
                ("topLeftRadius", "v-radius"),
            ],
        );
    });
    add(&mut doc, frame, NodeKind::Text(TextData::new("Hello")), "Title", (10.0, 80.0), (60.0, 20.0), &|n| {
        n.fills = vec![solid(0.0, 0.0, 0.0)];
        bind(
            n,
            &[
                ("fill", "v-fg"),
                ("characters", "v-title"),
                ("fontSize", "v-radius"),
                ("lineHeight", "v-radius"),
                ("letterSpacing", "v-gap"),
                ("paragraphSpacing", "v-gap"),
                ("paragraphIndent", "v-gap"),
                ("fontFamily", "v-title"),
                ("fontStyle", "v-title"),
            ],
        );
    });
    doc
}

/// Ids of collections, modes, variables and styles replaced by names (a round trip gives them new
/// ids), wherever they are used.
fn by_name(doc: &mut Document) {
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    for c in &doc.variables.collections {
        ids.insert(c.id.clone(), format!("collection {}", c.name));
        for m in &c.modes {
            ids.insert(m.id.clone(), format!("mode {}/{}", c.name, m.name));
        }
    }
    ids.extend(doc.variables.variables.iter().map(|v| (v.id.clone(), format!("variable {}", v.name))));
    ids.extend(doc.styles.iter().map(|s| (s.id.clone(), format!("style {}", s.name))));
    let name = |id: &String| ids.get(id).cloned().unwrap_or_else(|| format!("unknown {id}"));
    for c in &mut doc.variables.collections {
        c.id = name(&c.id);
        for m in &mut c.modes {
            m.id = name(&m.id);
        }
    }
    for v in &mut doc.variables.variables {
        (v.id, v.collection) = (name(&v.id), name(&v.collection));
        v.values = std::mem::take(&mut v.values)
            .into_iter()
            .map(|(m, val)| {
                let val = match val {
                    VariableValue::Alias(a) => VariableValue::Alias(name(&a)),
                    other => other,
                };
                (name(&m), val)
            })
            .collect();
    }
    for s in &mut doc.styles {
        s.id = name(&s.id);
    }
    let nodes: Vec<NodeId> = doc.ids().collect();
    for id in nodes {
        let Some(n) = doc.node_mut(id) else { continue };
        for v in n.bound.values_mut() {
            *v = name(v);
        }
        n.var_modes = std::mem::take(&mut n.var_modes).into_iter().map(|(c, m)| (name(&c), name(&m))).collect();
        for r in [&mut n.styles.fill, &mut n.styles.stroke, &mut n.styles.effect, &mut n.styles.grid, &mut n.styles.text] {
            *r = r.as_ref().map(name);
        }
    }
}

/// Export `doc`, import the file and compare everything: layers with their bindings, modes and
/// style links, variables and styles. Returns the export's warnings.
fn same_after_round_trip(doc: &Document, what: &str) -> Vec<String> {
    let (bytes, warnings) = export(doc, None).unwrap();
    let (mut back, notes) = import(&bytes, "x").unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(notes.is_empty(), "{what}: the import warned: {notes:?}");
    let mut doc = doc.clone();
    by_name(&mut doc);
    by_name(&mut back);
    assert_same(&doc, &back, what);
    let assets = |d: &Document| {
        let mut styles = d.styles.clone();
        for s in &mut styles {
            // Held the way a layer's paints and text are.
            let mut n = Node::new(NodeId(0), NodeKind::Text(TextData::new("")), "", 0.0, 0.0);
            match &mut s.kind {
                StyleKind::Paint { paints } => {
                    n.fills = std::mem::take(paints);
                    normalize(&mut n, &|i| i);
                    *paints = n.fills;
                }
                StyleKind::Text { style } => {
                    if let NodeKind::Text(td) = &mut n.kind {
                        td.style = style.clone();
                    }
                    n.fills.clear();
                    normalize(&mut n, &|i| i);
                    if let NodeKind::Text(td) = n.kind {
                        *style = td.style;
                    }
                }
                _ => {}
            }
        }
        json!({"variables": d.variables, "styles": styles})
    };
    if let Err(e) = close(&assets(&doc), &assets(&back), "assets") {
        panic!("{what}: {e}\nwritten:   {}\nread back: {}", assets(&doc), assets(&back));
    }
    warnings
}

/// The node changes of the written file as JSON.
fn written(doc: &Document) -> Vec<Value> {
    let (bytes, _) = export(doc, None).unwrap();
    let (schema, message) = decode_canvas(&Archive::open(&bytes).unwrap().canvas).unwrap();
    let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    msg.get("nodeChanges").unwrap().list().iter().map(debug_json).collect()
}

fn named<'v>(nodes: &'v [Value], name: &str) -> &'v Value {
    nodes.iter().find(|n| n["name"] == name).unwrap_or_else(|| panic!("no {name} was written"))
}

#[test]
fn variables_modes_and_bindings_round_trip() {
    let doc = themed();
    let warnings = same_after_round_trip(&doc, "themed");
    assert!(warnings.is_empty(), "{warnings:?}");
    // Read back in the document's order, the default mode first.
    let (back, _) = import(&export(&doc, None).unwrap().0, "x").unwrap();
    let names = |d: &Document| d.variables.variables.iter().map(|v| v.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&back), names(&doc));
    assert_eq!(back.variables.collections.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Theme", "Spacing"]);
    assert_eq!(back.variables.collections[0].modes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Light", "Dark"]);
}

#[test]
fn variables_are_written_as_nodes_on_a_hidden_page() {
    let nodes = written(&themed());
    let hidden = named(&nodes, "Internal Only Canvas");
    assert_eq!((hidden["type"].as_str(), hidden["internalOnly"].as_bool(), hidden["visible"].as_bool()), (Some("CANVAS"), Some(true), Some(false)));
    let set = named(&nodes, "Theme");
    assert_eq!(set["type"], "VARIABLE_SET");
    assert_eq!(set["parentIndex"]["guid"], hidden["guid"]);
    let modes = set["variableSetModes"].as_array().unwrap();
    assert_eq!(modes.iter().map(|m| m["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Light", "Dark"]);
    let bg = named(&nodes, "color/bg");
    assert_eq!((bg["type"].as_str(), bg["variableResolvedType"].as_str()), (Some("VARIABLE"), Some("COLOR")));
    assert_eq!(bg["variableSetID"]["guid"], set["guid"]);
    assert_eq!(bg["description"], "Behind everything");
    assert_eq!(bg["variableScopes"], json!(["FRAME_FILL", "SHAPE_FILL"]));
    assert_eq!(named(&nodes, "color/fg")["variableScopes"], json!(["TEXT_FILL", "STROKE", "EFFECT_COLOR"]));
    let values = bg["variableDataValues"]["entries"].as_array().unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[1]["modeID"], modes[1]["id"]);
    assert_eq!(
        (values[1]["variableData"]["dataType"].as_str(), values[1]["variableData"]["value"]["colorValue"]["b"].as_f64()),
        (Some("COLOR"), Some(0.25))
    );
    // An alias names the variable's node; every value says the type it resolves to.
    let accent = &named(&nodes, "color/accent")["variableDataValues"]["entries"];
    assert_eq!(accent[0]["variableData"]["dataType"], "ALIAS");
    assert_eq!(accent[0]["variableData"]["resolvedDataType"], "COLOR");
    assert_eq!(accent[0]["variableData"]["value"]["alias"]["guid"], named(&nodes, "color/fg")["guid"]);
    assert_eq!(named(&nodes, "title")["variableDataValues"]["entries"][1]["variableData"]["value"]["textValue"], "Grüße 😀");
    assert_eq!(named(&nodes, "shown")["variableDataValues"]["entries"][1]["variableData"]["value"]["boolValue"], false);
    assert_eq!(named(&nodes, "space/gap")["variableDataValues"]["entries"][0]["variableData"]["value"]["floatValue"], 8.0);
    // Bindings: on the paint, the gradient stop, the effect and the grid, and in the layer's map.
    let card = named(&nodes, "Card");
    assert_eq!(card["fillPaints"][0]["colorVar"]["value"]["alias"]["guid"], bg["guid"]);
    assert_eq!(card["variableModeBySetMap"]["entries"][0]["variableSetID"]["guid"], set["guid"]);
    assert_eq!(card["variableModeBySetMap"]["entries"][0]["variableModeID"], modes[1]["id"]);
    assert_eq!(card["layoutGrids"][0]["gutterSizeVar"]["value"]["alias"]["guid"], named(&nodes, "space/gap")["guid"]);
    let fields: Vec<&str> =
        card["variableConsumptionMap"]["entries"].as_array().unwrap().iter().map(|e| e["variableField"].as_str().unwrap()).collect();
    for f in
        ["STACK_SPACING", "STACK_PADDING_LEFT", "STACK_PADDING_RIGHT", "RECTANGLE_TOP_LEFT_CORNER_RADIUS", "WIDTH", "OPACITY", "BORDER_TOP_WEIGHT"]
    {
        assert!(fields.contains(&f), "{f} in {fields:?}");
    }
    let swatch = named(&nodes, "Swatch");
    assert!(swatch["fillPaints"][1]["stopsVar"][0].get("colorVar").is_none());
    assert_eq!(swatch["fillPaints"][1]["stopsVar"][1]["colorVar"]["value"]["alias"]["guid"], bg["guid"]);
    assert_eq!(swatch["fillPaints"][1]["stopsVar"].as_array().unwrap().len(), 2);
    assert!(swatch["strokePaints"][0].get("colorVar").is_none() && swatch["strokePaints"][1].get("colorVar").is_some());
    for key in ["colorVar", "radiusVar", "spreadVar", "xVar", "yVar"] {
        assert_eq!(swatch["effects"][0][key]["dataType"], "ALIAS", "{key}");
    }
    // The page sets a mode too.
    assert_eq!(named(&nodes, "Page 1")["variableModeBySetMap"]["entries"][0]["variableSetID"]["guid"], named(&nodes, "Spacing")["guid"]);
}

#[test]
fn bindings_the_format_cant_say_keep_their_value_with_a_warning() {
    let mut doc = themed();
    let card = doc.nodes().find(|n| n.name == "Card").unwrap().id;
    let n = doc.node_mut(card).unwrap();
    n.bound.insert("componentProperties/Label".into(), "v-title".into());
    n.bound.insert("effects/x/radius".into(), "v-gap".into());
    n.bound.insert("fills/0/stops/zero/color".into(), "v-bg".into());
    n.bound.insert("height".into(), "v-gone".into());
    n.var_modes.insert("c-gone".into(), "m-light".into());
    n.styles.fill = Some("s-gone".into());
    doc.variables.variables[0].scopes = Some(vec!["GAP".into(), "FONT_WEIGHT".into()]);
    doc.variables.variables.push(var("v-lost", "lost", "c-gone", VariableType::Number, &[("m", VariableValue::Number(1.0))]));
    doc.variables.variables.push(var(
        "v-dangling",
        "dangling",
        "c-space",
        VariableType::Number,
        &[("m-only", VariableValue::Alias("v-gone".into()))],
    ));
    let (bytes, warnings) = export(&doc, None).unwrap();
    for part in [
        "properties the format has no field for",
        "variables that aren't in the document were written as their values",
        "modes of collections that aren't in the document",
        "styles that aren't in the document",
        "scopes the format has no name for",
        "whose collection isn't in the document",
        "aliases to variables that aren't in the document",
    ] {
        assert!(warnings.iter().any(|w| w.contains(part)), "{part} in {warnings:?}");
    }
    // The rest still comes across.
    let (back, _) = import(&bytes, "x").unwrap();
    let card = back.nodes().find(|n| n.name == "Card").unwrap();
    assert!(card.bound.contains_key("fill") && !card.bound.contains_key("height"));
    assert!(back.variables.get("lost").is_none());
}

/// A style of each kind linked from a layer, and one that nothing uses.
fn styled() -> Document {
    let mut doc = Document::new("Styled");
    let page = doc.first_page().unwrap();
    let shadow = Effect {
        kind: EffectKind::DropShadow(Shadow {
            color: Color::new(0.0, 0.0, 0.0, 0.25),
            offset: (0.0, 2.0),
            radius: 6.0,
            spread: 1.0,
            blend: BlendMode::Normal,
            show_behind: false,
        }),
        visible: true,
    };
    let grid = LayoutGrid { kind: GridKind::Columns, count: 12, gutter: 16.0, ..LayoutGrid::default() };
    let heading = TextStyle {
        font_family: "Inter".into(),
        font_style: "Bold".into(),
        font_size: 24.0,
        line_height: LineHeight::Pixels(32.0),
        ..TextStyle::default()
    };
    let style = |id: &str, name: &str, kind: StyleKind| Style {
        id: id.into(),
        name: name.into(),
        description: String::new(),
        kind,
        bound: Default::default(),
    };
    doc.styles = vec![
        style("s-text", "Heading", StyleKind::Text { style: heading.clone() }),
        style("s-brand", "Brand/Primary", StyleKind::Paint { paints: vec![solid(0.25, 0.5, 1.0)] }),
        style("s-unused", "Brand/Unused", StyleKind::Paint { paints: vec![solid(1.0, 0.0, 0.0), solid(0.0, 1.0, 0.0)] }),
        style("s-shadow", "Elevation/1", StyleKind::Effect { effects: vec![shadow.clone()] }),
        style("s-grid", "Desktop/12 columns", StyleKind::Grid { grids: vec![grid.clone()] }),
        style("s-line", "Line", StyleKind::Paint { paints: vec![solid(0.0, 0.0, 0.0)] }),
    ];
    doc.styles[1].description = "The brand colour".into();
    let frame = add(&mut doc, page, NodeKind::Frame, "Screen", (0.0, 0.0), (400.0, 300.0), &|n| {
        n.grids = vec![grid.clone()];
        n.effects = vec![shadow.clone()];
        n.styles.grid = Some("s-grid".into());
        n.styles.effect = Some("s-shadow".into());
    });
    add(&mut doc, frame, NodeKind::Rectangle, "Button", (10.0, 10.0), (120.0, 40.0), &|n| {
        n.fills = vec![solid(0.25, 0.5, 1.0)];
        n.strokes = vec![solid(0.0, 0.0, 0.0)];
        n.styles.fill = Some("s-brand".into());
        n.styles.stroke = Some("s-line".into());
    });
    let mut td = TextData::new("Title");
    td.style = heading;
    add(&mut doc, frame, NodeKind::Text(td), "Title", (10.0, 80.0), (100.0, 32.0), &|n| {
        n.fills = vec![solid(0.0, 0.0, 0.0)];
        n.styles.text = Some("s-text".into());
    });
    doc
}

#[test]
fn shared_styles_and_their_links_round_trip() {
    let doc = styled();
    let warnings = same_after_round_trip(&doc, "styled");
    assert!(warnings.is_empty(), "{warnings:?}");
    let (back, _) = import(&export(&doc, None).unwrap().0, "x").unwrap();
    // In the document's order, the unused one included, each with its values.
    let names = |d: &Document| d.styles.iter().map(|s| (s.name.clone(), s.kind.type_name())).collect::<Vec<_>>();
    assert_eq!(names(&back), names(&doc));
    assert_eq!(back.styles[1].description, "The brand colour");
    assert!(matches!(&back.styles[2].kind, StyleKind::Paint { paints } if paints.len() == 2));
    let nodes = written(&doc);
    let style = named(&nodes, "Brand/Primary");
    assert_eq!((style["styleType"].as_str(), style["type"].as_str()), (Some("FILL"), Some("ROUNDED_RECTANGLE")));
    assert_eq!(style["parentIndex"]["guid"], named(&nodes, "Internal Only Canvas")["guid"]);
    assert_eq!(named(&nodes, "Heading")["styleType"], "TEXT");
    assert_eq!(named(&nodes, "Heading")["fontSize"], 24.0);
    assert_eq!(named(&nodes, "Elevation/1")["styleType"], "EFFECT");
    assert_eq!(named(&nodes, "Desktop/12 columns")["styleType"], "GRID");
    let button = named(&nodes, "Button");
    assert_eq!(button["styleIdForFill"]["guid"], style["guid"]);
    assert_eq!(button["styleIdForStrokeFill"]["guid"], named(&nodes, "Line")["guid"]);
    assert_eq!(named(&nodes, "Screen")["styleIdForEffect"]["guid"], named(&nodes, "Elevation/1")["guid"]);
    assert_eq!(named(&nodes, "Screen")["styleIdForGrid"]["guid"], named(&nodes, "Desktop/12 columns")["guid"]);
    assert_eq!(named(&nodes, "Title")["styleIdForText"]["guid"], named(&nodes, "Heading")["guid"]);
}

#[test]
fn a_styled_layers_fill_variable_survives_a_round_trip() {
    let mut doc = Document::new("Overlap");
    let page = doc.first_page().unwrap();
    let mode = |id: &str, name: &str| Mode { id: id.into(), name: name.into() };
    doc.variables.collections =
        vec![Collection { id: "c-theme".into(), name: "Theme".into(), modes: vec![mode("m-light", "Light"), mode("m-dark", "Dark")], extends: None }];
    let light = Color::new(0.1, 0.3, 0.7, 1.0);
    let dark = VariableValue::Color(Color::new(0.8, 0.9, 1.0, 1.0));
    doc.variables.variables =
        vec![var("v-accent", "Accent color", "c-theme", VariableType::Color, &[("m-light", VariableValue::Color(light)), ("m-dark", dark)])];
    doc.styles = vec![Style {
        id: "s-accent".into(),
        name: "Accent".into(),
        description: String::new(),
        kind: StyleKind::Paint { paints: vec![Paint::solid(light)] },
        bound: Default::default(),
    }];
    add(&mut doc, page, NodeKind::Rectangle, "Swatch", (0.0, 0.0), (120.0, 80.0), &|n| {
        n.fills = vec![Paint::solid(light)];
        n.styles.fill = Some("s-accent".into());
        n.bound.insert("fill".into(), "v-accent".into());
    });
    let (back, _) = import(&export(&doc, None).unwrap().0, "x").unwrap();
    let var = back.variables.variables.iter().find(|v| v.name == "Accent color").unwrap().id.clone();
    let swatch = back.nodes().find(|n| n.name == "Swatch").unwrap();
    let on_layer = swatch.bound.get("fill") == Some(&var) || swatch.bound.get("fills/0/color") == Some(&var);
    let style = back.styles.iter().find(|s| Some(&s.id) == swatch.styles.fill.as_ref()).unwrap();
    let on_style = style.bound.get("fills/0/color") == Some(&var);
    assert!(on_layer || on_style, "the fill binding was lost: layer {:?}, style {:?}", swatch.bound, style.bound);
}

#[test]
fn text_styles_on_part_of_a_text_are_written_as_values() {
    let mut doc = styled();
    let title = doc.nodes().find(|n| n.name == "Title").unwrap().id;
    let n = doc.node_mut(title).unwrap();
    n.styles.text = None;
    if let NodeKind::Text(td) = &mut n.kind {
        let mut run = td.style.clone();
        run.style_id = Some("s-text".into());
        run.font_size = 12.0;
        td.runs = vec![StyleRun { start: 0, end: 2, style: run }];
    }
    let (bytes, warnings) = export(&doc, None).unwrap();
    assert!(warnings.iter().any(|w| w.contains("part of a text")), "{warnings:?}");
    let (back, _) = import(&bytes, "x").unwrap();
    let td = back.nodes().find_map(|n| n.kind.text().filter(|_| n.name == "Title")).unwrap();
    assert_eq!(td.runs.iter().map(|r| r.style.font_size).collect::<Vec<_>>(), [12.0]);
}

/// A component (a box that scales with it, a label that stays put) and instances of it: as it
/// is, resized, and scaled 1.5 times as a whole. Returns the document and the instances.
fn sized() -> (Document, [NodeId; 3]) {
    let mut doc = Document::new("Sized");
    let page = doc.first_page().unwrap();
    let main =
        add(&mut doc, page, NodeKind::Component { description: String::new(), props: BTreeMap::new() }, "Card", (0.0, 0.0), (100.0, 60.0), &|_| {});
    add(&mut doc, main, NodeKind::Rectangle, "Box", (10.0, 10.0), (40.0, 40.0), &|n| {
        n.constraints = Constraints { horizontal: HConstraint::Scale, vertical: VConstraint::Scale };
    });
    add(&mut doc, main, NodeKind::Text(TextData::new("Card")), "Label", (56.0, 20.0), (32.0, 16.0), &|_| {});
    let mut g = Gen { r: Lcg(1), doc, image: String::new(), frames: Vec::new(), layers: Vec::new() };
    let insts = [(); 3].map(|_| g.instance(main, page, BTreeMap::new()));
    let mut doc = g.doc;
    let kids = |doc: &Document, i: NodeId| doc.node(i).unwrap().children.clone();
    for (i, name) in insts.iter().zip(["Same", "Resized", "Scaled"]) {
        doc.node_mut(*i).unwrap().name = name.into();
    }
    // Resized to twice the size: the box follows, the label stays.
    let n = doc.node_mut(insts[1]).unwrap();
    (n.width, n.height) = (200.0, 120.0);
    let b = doc.node_mut(kids(&doc, insts[1])[0]).unwrap();
    (b.width, b.height, b.transform) = (80.0, 80.0, Affine::translate((20.0, 20.0)));
    // Scaled: every length grows, stroke weights and the font size too.
    for id in std::iter::once(insts[2]).chain(kids(&doc, insts[2])) {
        let n = doc.node_mut(id).unwrap();
        (n.width, n.height) = (n.width * 1.5, n.height * 1.5);
        n.stroke.weight *= 1.5;
        if id != insts[2] {
            let t = n.transform.translation();
            n.transform = Affine::translate((t.x * 1.5, t.y * 1.5));
        }
        if let NodeKind::Text(td) = &mut n.kind {
            td.style.font_size *= 1.5;
        }
    }
    (doc, insts)
}

/// The overrides (by the last id of their path) of a written instance: (overridden, derived).
fn overrides(inst: &Value) -> (BTreeMap<u64, Vec<String>>, BTreeMap<u64, Vec<String>>) {
    let list = |v: &Value| -> BTreeMap<u64, Vec<String>> {
        let entry = |e: &Value| {
            let id = e["guidPath"]["guids"].as_array().unwrap().last().unwrap()["localID"].as_u64().unwrap();
            (id, e.as_object().unwrap().keys().filter(|k| *k != "guidPath").cloned().collect())
        };
        v.as_array().map(|a| a.iter().map(entry).collect()).unwrap_or_default()
    };
    (list(&inst["symbolData"]["symbolOverrides"]), list(&inst["derivedSymbolData"]))
}

#[test]
fn instances_keep_their_own_size_and_lay_their_layers_out_for_it() {
    let (doc, _) = sized();
    let nodes = written(&doc);
    let id = |name: &str| named(&nodes, name)["guid"]["localID"].as_u64().unwrap();
    let (main, boxed, label) = (id("Card"), id("Box"), id("Label"));
    // As its main: only its name is its own.
    let same = named(&nodes, "Same");
    assert_eq!(overrides(same), (BTreeMap::from([(main, vec!["name".to_string()])]), BTreeMap::new()));
    assert_eq!(same["symbolData"]["uniformScaleFactor"], 1.0);
    // Resized: the size is the instance's override (addressed by the main's id); where the box
    // lands follows from it.
    let resized = named(&nodes, "Resized");
    assert_eq!(resized["size"], json!({"x": 200.0, "y": 120.0}));
    let (over, derived) = overrides(resized);
    assert_eq!(over, BTreeMap::from([(main, vec!["name".to_string(), "size".to_string()])]));
    assert_eq!(derived, BTreeMap::from([(boxed, vec!["fillGeometry".to_string(), "size".to_string(), "transform".to_string()])]));
    assert_eq!(resized["derivedSymbolData"][0]["size"], json!({"x": 80.0, "y": 80.0}));
    assert_eq!(resized["symbolData"]["uniformScaleFactor"], 1.0);
    // Scaled: the factor says it; what it makes of the main's values is derived, not overridden.
    let scaled = named(&nodes, "Scaled");
    assert_eq!(scaled["symbolData"]["uniformScaleFactor"], 1.5);
    let (over, derived) = overrides(scaled);
    assert_eq!(over, BTreeMap::from([(main, vec!["name".to_string(), "size".to_string()])]));
    assert!(derived[&boxed].contains(&"strokeWeight".to_string()) && derived[&label].contains(&"fontSize".to_string()), "{derived:?}");
    // All of it reads back.
    let warnings = same_after_round_trip(&doc, "sized");
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn an_instance_named_after_its_set_has_no_name_of_its_own() {
    let (mut doc, insts) = sized();
    let page = doc.first_page().unwrap();
    let main = doc.nodes().find(|n| n.name == "Card").unwrap().id;
    let set = doc.create(page, NodeKind::ComponentSet { description: String::new() }, "Chip", 0.0, 200.0, 300.0, 100.0).unwrap();
    doc.reparent(main, set, None).unwrap();
    doc.node_mut(main).unwrap().name = "State=On".into();
    doc.node_mut(insts[0]).unwrap().name = "Chip".into();
    let nodes = written(&doc);
    let chip = nodes.iter().find(|n| n["type"] == "INSTANCE" && n["name"] == "Chip").unwrap();
    assert!(overrides(chip).0.values().all(|keys| !keys.contains(&"name".to_string())));
    // Renamed, the name is its own.
    let other = nodes.iter().find(|n| n["type"] == "INSTANCE" && n["name"] == "Resized").unwrap();
    assert!(overrides(other).0.values().any(|keys| keys.contains(&"name".to_string())));
}

#[test]
fn layers_bound_like_their_mains_are_no_overrides_in_another_mode() {
    let mut doc = themed();
    let page = doc.first_page().unwrap();
    let main =
        add(&mut doc, page, NodeKind::Component { description: String::new(), props: BTreeMap::new() }, "Chip", (0.0, 300.0), (80.0, 30.0), &|n| {
            n.bound.insert("fill".into(), "v-bg".into());
        });
    add(&mut doc, main, NodeKind::Text(TextData::new("Chip")), "Chip label", (8.0, 6.0), (40.0, 16.0), &|n| {
        n.fills = vec![solid(0.0, 0.0, 0.0)];
        n.bound.insert("fill".into(), "v-fg".into());
    });
    let dark = add(&mut doc, page, NodeKind::Frame, "Dark", (200.0, 300.0), (200.0, 100.0), &|n| {
        n.var_modes.insert("c-theme".into(), "m-dark".into());
    });
    let mut g = Gen { r: Lcg(2), doc, image: String::new(), frames: Vec::new(), layers: Vec::new() };
    let inst = g.instance(main, dark, BTreeMap::new());
    let mut doc = g.doc;
    // What the variables give in the dark mode.
    doc.node_mut(inst).unwrap().name = "Dark chip".into();
    doc.node_mut(inst).unwrap().fills = vec![solid(0.0, 0.125, 0.25)];
    let label = doc.node(inst).unwrap().children[0];
    doc.node_mut(label).unwrap().fills = vec![solid(0.75, 0.75, 0.75)];
    let nodes = written(&doc);
    let chip = named(&nodes, "Dark chip");
    let (over, derived) = overrides(chip);
    assert!(over.values().all(|keys| !keys.contains(&"fillPaints".to_string())), "{over:?}");
    assert_eq!(derived.values().next().unwrap(), &["fillPaints".to_string()]);
    assert_eq!(chip["fillPaints"][0]["color"]["b"], 0.25);
    assert!(chip["fillPaints"][0]["colorVar"].is_object());
    let warnings = same_after_round_trip(&doc, "dark chip");
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn layers_of_a_swapped_nested_instance_keep_what_they_change() {
    let (mut doc, insts) = sized();
    let page = doc.first_page().unwrap();
    let card = doc.nodes().find(|n| n.name == "Card").unwrap().id;
    // Another component like the card, and a row holding a card instance.
    let other = add(
        &mut doc,
        page,
        NodeKind::Component { description: String::new(), props: BTreeMap::new() },
        "Other",
        (0.0, 400.0),
        (100.0, 60.0),
        &|_| {},
    );
    add(&mut doc, other, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Dot", (10.0, 10.0), (40.0, 40.0), &|_| {});
    let row =
        add(&mut doc, page, NodeKind::Component { description: String::new(), props: BTreeMap::new() }, "Row", (0.0, 500.0), (300.0, 80.0), &|_| {});
    let mut g = Gen { r: Lcg(3), doc, image: String::new(), frames: Vec::new(), layers: Vec::new() };
    let thumb = g.instance(card, row, BTreeMap::new());
    g.doc.node_mut(thumb).unwrap().name = "Thumb".into();
    let outer = g.instance(row, page, BTreeMap::new());
    let mut doc = g.doc;
    doc.node_mut(outer).unwrap().name = "Row instance".into();
    // In the row's instance the thumb shows the other component, its dot made smaller.
    let nested = doc.node(outer).unwrap().children[0];
    let old: Vec<NodeId> = doc.node(nested).unwrap().children.clone();
    for k in old {
        doc.remove(k).unwrap();
    }
    if let Some(NodeKind::Instance { component, .. }) = doc.node_mut(nested).map(|n| &mut n.kind) {
        *component = other;
    }
    let dot_main = doc.node(other).unwrap().children[0];
    let mut dot = doc.node(dot_main).unwrap().clone();
    (dot.id, dot.source, dot.width, dot.height) = (doc.alloc_id(), Some(dot_main), 20.0, 20.0);
    dot.fills = vec![solid(1.0, 0.0, 0.0)];
    doc.insert(dot, nested, None).unwrap();
    let (bytes, warnings) = export(&doc, None).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let nodes = written(&doc);
    let (over, derived) = overrides(named(&nodes, "Row instance"));
    let dot_id = named(&nodes, "Dot")["guid"]["localID"].as_u64().unwrap();
    assert!(over[&dot_id].contains(&"fillPaints".to_string()) && derived[&dot_id].contains(&"size".to_string()), "{over:?} {derived:?}");
    let (back, _) = import(&bytes, "x").unwrap();
    let dots: Vec<(f64, usize)> = back.nodes().filter(|n| n.name == "Dot").map(|n| (n.width, n.fills.len())).collect();
    assert_eq!(dots, [(40.0, 1), (20.0, 1)]);
    let _ = insts;
}

/// A line with a stroke of weight 8 and `profile` (`[t, scale, bias]` points).
fn tapered(profile: &[(f64, f64, f64)]) -> Document {
    let mut doc = Document::new("Widths");
    let page = doc.first_page().unwrap();
    add(&mut doc, page, NodeKind::Line, "Tapered", (20.0, 20.0), (100.0, 0.0), &|n| {
        n.fills.clear();
        n.strokes = vec![solid(0.0, 0.0, 0.0)];
        n.stroke.weight = 8.0;
        n.stroke.align = StrokeAlign::Center;
        let mut p = WidthProfile::new(profile.iter().map(|q| (q.0, q.1)));
        for (point, q) in p.points.iter_mut().zip(profile) {
            point.bias = q.2;
        }
        n.stroke.width_profile = Some(p);
    });
    doc
}

#[test]
fn variable_width_strokes_say_their_reach_as_a_share_of_the_weight() {
    let mut doc = tapered(&[(0.0, 0.25, 0.0), (0.5, 1.0, 0.0), (1.0, 0.0, 0.0)]);
    let nodes = written(&doc);
    let line = named(&nodes, "Tapered");
    let points = line["variableWidthPoints"].as_array().unwrap();
    assert_eq!(points.len(), 3);
    assert_eq!(points[0], json!({"position": 0.0, "ascent": 0.125, "descent": 0.125}));
    // The full weight is half of it on each side.
    assert_eq!(points[1], json!({"position": 0.5, "ascent": 0.5, "descent": 0.5}));
    // The outline is left to the reader: it follows from the points.
    assert!(line.get("strokeGeometry").is_none() && line.get("pluginData").is_none());
    let warnings = same_after_round_trip(&doc, "widths");
    assert!(warnings.is_empty(), "{warnings:?}");
    // A stroke without a weight has no reach to say.
    let line = doc.nodes().find(|n| n.name == "Tapered").unwrap().id;
    doc.node_mut(line).unwrap().stroke.weight = 0.0;
    assert!(named(&written(&doc), "Tapered").get("variableWidthPoints").is_none());
}

#[test]
fn strokes_wider_on_one_side_are_written_as_their_outline_and_come_back_as_strokes() {
    let doc = tapered(&[(0.0, 0.0, 1.0), (0.5, 1.0, 1.0), (1.0, 0.0, 1.0)]);
    let nodes = written(&doc);
    let line = named(&nodes, "Tapered");
    // A vector filled with the stroke's paint, in the line's place; no stroke left to draw.
    assert_eq!(line["type"], "VECTOR");
    assert!(line.get("variableWidthPoints").is_none());
    assert_eq!((line["fillPaints"].as_array().unwrap().len(), line["strokePaints"].as_array().unwrap().len()), (1, 0));
    assert!(line["fillGeometry"].is_array() && line["vectorData"].is_object());
    assert_eq!(line["transform"]["m02"], 20.0);
    let kept = &line["pluginData"][0];
    assert_eq!((kept["pluginID"].as_str(), kept["key"].as_str()), (Some("figcraft"), Some("stroke")));
    assert!(kept["value"].as_str().unwrap().contains("width_profile"));
    // Our own import gets the line and its stroke back.
    let (bytes, warnings) = export(&doc, None).unwrap();
    assert!(warnings.iter().any(|w| w.contains("filled outlines")), "{warnings:?}");
    let (back, _) = import(&bytes, "x").unwrap();
    let (a, b) = (doc.nodes().find(|n| n.name == "Tapered").unwrap(), back.nodes().find(|n| n.name == "Tapered").unwrap());
    assert_eq!((&b.kind, &b.stroke, &b.strokes, &b.fills), (&a.kind, &a.stroke, &a.strokes, &a.fills));
    assert_eq!((b.width, b.height, b.transform), (a.width, a.height, a.transform));
    // A filled shape keeps its stroke, with the width on both sides and a warning.
    let mut filled = doc.clone();
    let id = a.id;
    let n = filled.node_mut(id).unwrap();
    n.kind = NodeKind::Rectangle;
    (n.height, n.fills) = (40.0, vec![solid(1.0, 0.0, 0.0)]);
    let (_, warnings) = export(&filled, None).unwrap();
    assert!(warnings.iter().any(|w| w.contains("on both sides")), "{warnings:?}");
    let shape = written(&filled);
    let shape = named(&shape, "Tapered");
    assert_eq!(shape["type"], "ROUNDED_RECTANGLE");
    assert_eq!(shape["variableWidthPoints"][1], json!({"position": 0.5, "ascent": 0.5, "descent": 0.5}));
}

#[test]
fn junk_kept_strokes_are_ignored_or_made_safe() {
    let node = |kept: Value| {
        json!({"name": "x", "document": {"id": "0:0", "type": "DOCUMENT", "children": [{"id": "0:1", "type": "CANVAS", "name": "P", "children": [
            {"id": "1:2", "type": "VECTOR", "name": "V", "size": {"x": 10, "y": 10}, "fills": [], "outlinedStroke": kept}]}]}})
    };
    let stroke = json!({"weight": 1e300, "dashes": vec![1.0; 500], "width_profile": {"points": [{"t": 9.0, "scale": -4.0, "bias": 77.0}]}});
    for kept in [
        json!(null),
        json!("stroke"),
        json!({"kind": {"type": "page"}, "fills": [], "strokes": [], "stroke": {}}),
        json!({"kind": {"type": "instance", "component": 5}, "fills": [], "strokes": [], "stroke": {}}),
        json!({"kind": {"type": "line"}, "fills": 5, "strokes": [], "stroke": {}}),
        json!({"kind": {"type": "line"}, "fills": [], "strokes": [], "stroke": stroke}),
        json!({"kind": {"type": "vector", "network": {"vertices": [{"x": 0.0, "y": 0.0}], "segments": [{"a": 0, "b": 99, "ta": [0.0, 0.0], "tb": [0.0, 0.0]}], "regions": [{"loops": [[7]], "winding": "nonZero"}]}},
               "fills": [], "strokes": [], "stroke": {}}),
    ] {
        let (doc, _) = crate::rest::import(&node(kept), &|_| None).unwrap();
        let v = doc.nodes().find(|n| n.name == "V").unwrap();
        assert!(matches!(v.kind, NodeKind::Vector { .. } | NodeKind::Line));
        assert!(v.stroke.weight <= 1e6 && v.stroke.dashes.len() <= 64);
        if let NodeKind::Vector { network } = &v.kind {
            assert!(network.segments.iter().all(|s| s.a < network.vertices.len() && s.b < network.vertices.len()));
        }
        export(&doc, None).unwrap();
    }
}

#[test]
fn bound_layer_fields_name_their_property_and_the_field_that_holds_it() {
    let nodes = written(&themed());
    let entries = |name: &str| -> BTreeMap<String, u64> {
        let list = named(&nodes, name)["variableConsumptionMap"]["entries"].as_array().unwrap();
        list.iter().map(|e| (e["variableField"].as_str().unwrap().to_string(), e["nodeField"].as_u64().unwrap())).collect()
    };
    let card = entries("Card");
    // The numbers are those of the layer's own fields in the schema.
    for (field, id) in [
        ("STACK_SPACING", 107),
        ("STACK_PADDING_LEFT", 209),
        ("STACK_PADDING_RIGHT", 233),
        ("CORNER_RADIUS", 20),
        ("WIDTH", 11),
        ("OPACITY", 8),
        ("BORDER_TOP_WEIGHT", 295),
    ] {
        assert_eq!(card.get(field), Some(&id), "{field}");
    }
    let title = entries("Title");
    for (field, id) in [("TEXT_DATA", 42), ("FONT_SIZE", 21), ("FONT_FAMILY", 41), ("LINE_HEIGHT", 40)] {
        assert_eq!(title.get(field), Some(&id), "{field}");
    }
    assert_eq!(entries("Swatch").get("VISIBLE"), Some(&6));
}

#[test]
fn the_schema_numbers_variables_styles_and_instance_data_as_the_format_does() {
    let s = Schema::parse(export::SCHEMA).unwrap();
    let def = |n: &str| s.defs.iter().find(|d| d.name == n).unwrap_or_else(|| panic!("no {n}"));
    let id = |d: &str, f: &str| def(d).fields.iter().find(|x| x.name == f).unwrap_or_else(|| panic!("no {d}.{f}")).value;
    let node = |f: &str| id("NodeChange", f);
    assert_eq!((id("NodeType", "VARIABLE"), id("NodeType", "VARIABLE_SET")), (28, 31));
    assert_eq!((node("variableConsumptionMap"), node("variableModeBySetMap"), node("variableSetModes")), (307, 316, 312));
    assert_eq!((node("variableSetID"), node("variableResolvedType"), node("variableDataValues"), node("variableScopes")), (313, 314, 315, 353));
    assert_eq!((node("styleType"), node("sortPosition"), node("description")), (163, 320, 318));
    assert_eq!(
        (node("styleIdForFill"), node("styleIdForStrokeFill"), node("styleIdForText"), node("styleIdForEffect"), node("styleIdForGrid")),
        (332, 333, 334, 335, 336)
    );
    assert_eq!((node("derivedSymbolData"), id("SymbolData", "uniformScaleFactor"), node("variableWidthPoints")), (125, 3, 447));
    assert_eq!((id("Paint", "colorVar"), id("Paint", "stopsVar"), id("Effect", "colorVar"), id("Effect", "startRadiusVar")), (21, 23, 10, 45));
    assert_eq!((id("LayoutGrid", "numSectionsVar"), id("LayoutGrid", "gutterSizeVar")), (10, 13));
    assert_eq!((id("VariableAnyValue", "alias"), id("VariableAnyValue", "colorValue"), id("VariableField", "BORDER_RIGHT_WEIGHT")), (4, 5, 21));
    assert_eq!((id("VariableScope", "STROKE"), id("StyleType", "GRID"), id("VariableDataType", "ALIAS")), (9, 6, 3));
    for n in [
        "StyleId",
        "VariableID",
        "VariableSetID",
        "VariableSetMode",
        "VariableDataValues",
        "VariableModeBySetMap",
        "ColorStopVar",
        "VariableWidthPoint",
    ] {
        assert_eq!(def(n).kind, Kind::Message, "{n}");
    }
}

// ---- damaged input ----

/// Change values all over a decoded message, keeping each value's kind and its numbers finite (so
/// it still encodes).
fn scramble(v: &mut Kv, r: &mut Lcg) {
    match v {
        Kv::Float(f) if r.chance(8) => *f = r.pick(&[f32::MAX, -1e30, 1e30, 0.0, -1.0, 1e-30]),
        Kv::Uint(u) if r.chance(12) => *u = r.pick(&[0, 1, 2, 7, 4_294_967_295]),
        Kv::Int(i) if r.chance(12) => *i = r.pick(&[0, -1, 2_147_483_647, -2_147_483_648]),
        Kv::Str(s) if r.chance(8) => *s = r.pick(&["", "!", "~~~~~~", "\u{FFFF}", "ALIAS"]).to_string(),
        Kv::Bool(b) if r.chance(8) => *b = !*b,
        Kv::Array(a) => {
            match r.below(12) {
                0 => a.clear(),
                1 => a.truncate(a.len() / 2),
                2 if a.len() < 64 => a.extend(a.clone()),
                _ => {}
            }
            a.iter_mut().for_each(|x| scramble(x, r));
        }
        Kv::Object(f) => {
            // Messages may lose fields (structs must keep theirs to encode).
            let message = f.iter().any(|(k, _)| matches!(*k, "type" | "guidPath" | "entries" | "variableData" | "symbolID" | "dataType"));
            if message && f.len() > 2 && r.chance(6) {
                let i = r.below(f.len() as u64) as usize;
                f.remove(i);
            }
            f.iter_mut().for_each(|(_, x)| scramble(x, r));
        }
        _ => {}
    }
}

#[test]
fn damaged_variables_styles_and_instances_never_panic() {
    let docs = [themed(), styled(), sized().0];
    let mut read = 0;
    for (d, doc) in docs.iter().enumerate() {
        let (bytes, _) = export(doc, None).unwrap();
        let (schema, message) = decode_canvas(&Archive::open(&bytes).unwrap().canvas).unwrap();
        let root = schema.find("Message").unwrap();
        for seed in 0..120 {
            let mut msg = schema.decode_message(root, &message).unwrap();
            scramble(&mut msg, &mut Lcg(seed * 3 + d as u64));
            let Ok(body) = encode::message(&schema, root, &msg) else { continue };
            let mut canvas = b"fig-kiwi".to_vec();
            canvas.extend_from_slice(&101u32.to_le_bytes());
            for chunk in [encode::schema(&schema), body] {
                let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
                std::io::Write::write_all(&mut z, &chunk).unwrap();
                let c = z.finish().unwrap();
                canvas.extend_from_slice(&(c.len() as u32).to_le_bytes());
                canvas.extend_from_slice(&c);
            }
            if let Ok((back, _)) = import(&canvas, "x") {
                read += 1;
                // What came back can be written again.
                export(&back, None).unwrap();
            }
        }
    }
    assert!(read > 100, "only {read} damaged files were read");
}

#[test]
fn junk_style_definitions_and_orders_never_panic() {
    let page = json!({"id": "0:1", "type": "CANVAS", "name": "P", "explicitVariableModes": {"c": 5, "": "m", "x": null},
        "children": [{"id": "1:2", "type": "FRAME", "name": "R", "styles": {"fill": "1:9", "text": 4},
            "fills": [{"type": "SOLID", "color": {"r": 1, "g": 0, "b": 0, "a": 1}, "boundVariables": {"color": {"type": "VARIABLE_ALIAS", "id": "v"}}},
                      {"type": "GRADIENT_LINEAR", "gradientStops": [{"position": 0, "color": {"r": 0, "g": 0, "b": 0, "a": 1}, "boundVariables": {"color": 7}},
                                                                    {"boundVariables": {"color": {"type": "VARIABLE_ALIAS", "id": "v"}}}]}],
            "effects": [{"type": "DROP_SHADOW", "boundVariables": {"radius": {"type": "VARIABLE_ALIAS", "id": "n"}, "nope": 1, "color": []}}],
            "layoutGrids": [{"pattern": "GRID", "boundVariables": {"count": {"type": "VARIABLE_ALIAS", "id": "n"}, "offset": "x"}}]}]});
    let meta = json!({"variableCollections": {"c": {"id": "c", "name": "C", "modes": [{"modeId": "m", "name": "M"}]}},
        "variables": {"v": {"id": "v", "name": "V", "variableCollectionId": "c", "resolvedType": "COLOR", "valuesByMode": {"m": {"r": 1, "g": 1, "b": 1, "a": 1}}},
                      "n": {"id": "n", "name": "N", "variableCollectionId": "c", "resolvedType": "FLOAT", "valuesByMode": {"m": 4}}},
        "collectionOrder": [5, "c", "c", null, {"x": 1}], "variableOrder": "v"});
    let definitions = [
        json!([]),
        json!("styles"),
        json!([5, null, {}, {"id": ""}, {"id": "1:9"}, {"id": "1:9", "node": 5, "styleType": "FILL"}]),
        json!([{"id": "1:9", "styleType": "FILL", "node": {"fills": "red"}}, {"id": "1:9", "styleType": "TEXT", "node": {"style": []}}]),
        json!([{"id": "a", "styleType": "EFFECT", "node": {"effects": [1, {"type": "?"}]}}, {"id": "b", "styleType": "GRID", "node": {"layoutGrids": {}}},
               {"id": "c", "styleType": "TEXT", "node": {"type": "TEXT", "characters": 7, "style": {"fontSize": "big"}}}, {"id": "d", "styleType": "NOPE", "node": {}}]),
        Value::Array((0..20_000).map(|i| json!({"id": format!("s{i}"), "styleType": "FILL", "node": {"fills": []}})).collect()),
    ];
    for d in definitions {
        let file = json!({"name": "x", "document": {"id": "0:0", "type": "DOCUMENT", "children": [page.clone()]}, "meta": meta.clone(), "styleDefinitions": d});
        let (doc, _) = crate::rest::import(&file, &|_| None).unwrap();
        assert!(doc.styles.len() <= 10_001);
        // The bound paint, stop, effect and grid fields come across: on the layer, or (the fill it
        // takes from a shared style) on that style.
        let r = doc.nodes().find(|n| n.name == "R").unwrap();
        let mut keys: Vec<&str> = r.bound.keys().map(String::as_str).collect();
        keys.extend(doc.styles.iter().filter(|s| s.id == "1:9").flat_map(|s| s.bound.keys().map(String::as_str)));
        keys.sort_unstable();
        assert_eq!(keys, ["effects/0/radius", "fills/0/color", "layoutGrids/0/count"]);
        export(&doc, None).unwrap();
    }
}

/// Local only (`FIGCRAFT_FIG_SAMPLES`): each sample's variables, modes, bindings, styles and style
/// links come back the same after an export and another import.
#[test]
fn local_samples_keep_their_variables_and_styles() {
    for p in crate::fig::tests::samples() {
        let bytes = std::fs::read(&p).unwrap();
        let (mut a, _) = import(&bytes, "sample").unwrap();
        link(&mut a);
        let (out, warnings) = export(&a, None).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let (mut b, _) = import(&out, "sample").unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert!(!warnings.iter().any(|w| w.contains("variable") || w.contains("style")), "{}: {warnings:?}", p.display());
        by_name(&mut a);
        by_name(&mut b);
        assert_same(&a, &b, &p.display().to_string());
        assert_eq!(json!(a.variables), json!(b.variables), "{}", p.display());
        let styles = |d: &Document| d.styles.iter().map(|s| (s.name.clone(), s.kind.type_name())).collect::<Vec<_>>();
        assert_eq!(styles(&a), styles(&b), "{}", p.display());
    }
}
