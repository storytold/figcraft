//! The import's tail: editable vector networks, the closest layers for kinds the editor doesn't
//! have (sticky notes, shapes with text, connectors, tables) and variable prototype actions, on
//! fixtures built in code with the parent module's helpers.

use figcraft_doc::proto::Action;
use figcraft_doc::{HandleMirroring, StrokeCap, StrokeJoin, WindingRule};

use super::super::network::{self, Network, Region, Segment, Vertex};
use super::*;

fn seg(a: usize, b: usize, ta: (f64, f64), tb: (f64, f64)) -> Segment {
    Segment { style: 0, a, ta, b, tb }
}

fn vtx(style: u32, x: f64, y: f64) -> Vertex {
    Vertex { style, x, y }
}

/// A square with one curved side and a rounded, mirrored corner, filled even-odd.
fn square_blob() -> Vec<u8> {
    network::encode(&Network {
        vertices: vec![vtx(0, 0.0, 0.0), vtx(1, 50.0, 0.0), vtx(2, 50.0, 50.0), vtx(3, 0.0, 50.0)],
        segments: vec![
            seg(0, 1, (10.0, 0.0), (-10.0, 0.0)),
            seg(1, 2, (0.0, 0.0), (0.0, 0.0)),
            seg(2, 3, (0.0, 0.0), (0.0, 0.0)),
            seg(3, 0, (0.0, 0.0), (0.0, 0.0)),
        ],
        regions: vec![Region { style: 0, non_zero: false, loops: vec![vec![0, 1, 2, 3]] }],
    })
}

/// An open line whose end point has its own (arrow) cap.
fn arrow_blob() -> Vec<u8> {
    network::encode(&Network {
        vertices: vec![vtx(0, 0.0, 0.0), vtx(2, 100.0, 0.0)],
        segments: vec![seg(0, 1, (0.0, 0.0), (0.0, 0.0))],
        regions: vec![],
    })
}

/// An open path with one missing style ID and one unsupported point join.
fn join_fallback_blob() -> Vec<u8> {
    network::encode(&Network {
        vertices: vec![vtx(4, 0.0, 0.0), vtx(5, 50.0, 0.0), vtx(0, 100.0, 0.0)],
        segments: vec![seg(0, 1, (0.0, 0.0), (0.0, 0.0)), seg(1, 2, (0.0, 0.0), (0.0, 0.0))],
        regions: vec![],
    })
}

fn vector_data(blob: u64, nx: f64, ny: f64, styles: Vec<Kv<'static>>) -> Kv<'static> {
    o(vec![("vectorNetworkBlob", u(blob)), ("normalizedSize", xy(nx, ny)), ("styleOverrideTable", arr(styles))])
}

fn outline(blob: u64) -> Kv<'static> {
    arr(vec![o(vec![("windingRule", e("NONZERO")), ("commandsBlob", u(blob))])])
}

fn text_data(t: &str) -> Kv<'static> {
    o(vec![("characters", s(t))])
}

fn data(ty: &'static str, value: Kv<'static>) -> Kv<'static> {
    o(vec![("value", value), ("dataType", e(ty))])
}

fn action(ty: &'static str, mut more: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    more.push(("connectionType", e(ty)));
    o(more)
}

/// Blobs: 0 a triangle outline, 1 the square network, 2 the arrow network, 3 a truncated network.
fn tail_message() -> Kv<'static> {
    let modes = arr(vec![
        o(vec![("id", g(92)), ("name", s("Off")), ("sortPosition", s("!"))]),
        o(vec![("id", g(93)), ("name", s("On")), ("sortPosition", s("\""))]),
    ]);
    let bool_value = |b| data("BOOLEAN", o(vec![("boolValue", Kv::Bool(b))]));
    let values = o(vec![(
        "entries",
        arr(vec![o(vec![("modeID", g(92)), ("variableData", bool_value(false))]), o(vec![("modeID", g(93)), ("variableData", bool_value(true))])]),
    )]);
    let is_on = data(
        "EXPRESSION",
        o(vec![(
            "expressionValue",
            o(vec![("expressionFunction", e("EQUALS")), ("expressionArguments", arr(vec![alias_data(41), bool_value(true)]))]),
        )]),
    );
    let click = o(vec![
        ("event", o(vec![("interactionType", e("ON_CLICK"))])),
        (
            "actions",
            arr(vec![
                action("SET_VARIABLE", vec![("targetVariable", o(vec![("id", o(vec![("guid", g(41))]))])), ("targetVariableData", bool_value(true))]),
                action(
                    "CONDITIONAL",
                    vec![(
                        "conditionalActions",
                        arr(vec![
                            o(vec![("actions", arr(vec![action("BACK", vec![])])), ("condition", is_on)]),
                            o(vec![("actions", arr(vec![action("CLOSE", vec![])]))]),
                        ]),
                    )],
                ),
                action("SET_VARIABLE_MODE", vec![("targetVariableSetID", o(vec![("guid", g(40))])), ("targetVariableModeID", g(93))]),
            ]),
        ),
    ]);
    let stroke = || ("strokePaints", arr(vec![solid(0.0, 0.0, 0.0)]));
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Board", vec![]),
        node(2, Some((0, "~")), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))]),
        node(40, Some((2, "!")), "VARIABLE_SET", "Flags", vec![("variableSetModes", modes)]),
        node(
            41,
            Some((2, "\"")),
            "VARIABLE",
            "on",
            vec![("variableSetID", o(vec![("guid", g(40))])), ("variableResolvedType", e("BOOLEAN")), ("variableDataValues", values)],
        ),
        node(
            30,
            Some((1, "!")),
            "VECTOR",
            "Wave",
            vec![
                ("size", xy(200.0, 100.0)),
                ("transform", at(700.0, 0.0)),
                ("fillPaints", arr(vec![solid(1.0, 0.0, 0.0)])),
                ("fillGeometry", outline(0)),
                ("strokeJoin", e("MITER")),
                ("dashPattern", arr(vec![f(4.0), f(2.0)])),
                (
                    "vectorData",
                    vector_data(
                        1,
                        100.0,
                        50.0,
                        vec![
                            o(vec![
                                ("styleID", u(1)),
                                ("cornerRadius", f(4.0)),
                                ("handleMirroring", e("ANGLE_AND_LENGTH")),
                                ("strokeJoin", e("BEVEL")),
                            ]),
                            o(vec![("styleID", u(2)), ("strokeJoin", e("MITER"))]),
                            o(vec![("styleID", u(3)), ("strokeJoin", e("ROUND"))]),
                        ],
                    ),
                ),
            ],
        ),
        node(
            31,
            Some((1, "\"")),
            "VECTOR",
            "Arrow",
            vec![
                ("size", xy(300.0, 0.0)),
                ("transform", at(0.0, 230.0)),
                stroke(),
                ("strokeWeight", f(2.0)),
                ("strokeCap", e("NONE")),
                ("vectorData", vector_data(2, 100.0, 0.0, vec![o(vec![("styleID", u(2)), ("strokeCap", e("ARROW_LINES"))])])),
            ],
        ),
        node(
            42,
            Some((1, "#")),
            "VECTOR",
            "Join Fallbacks",
            vec![
                ("size", xy(100.0, 10.0)),
                ("transform", at(0.0, 330.0)),
                stroke(),
                ("strokeJoin", e("MITER")),
                ("vectorData", vector_data(4, 100.0, 10.0, vec![o(vec![("styleID", u(5)), ("strokeJoin", u(999))])])),
            ],
        ),
        node(
            32,
            Some((1, "#")),
            "VECTOR",
            "Broken",
            vec![
                ("size", xy(10.0, 8.0)),
                ("transform", at(700.0, 200.0)),
                ("fillPaints", arr(vec![solid(0.0, 0.0, 1.0)])),
                ("fillGeometry", outline(0)),
                ("vectorData", vector_data(3, 10.0, 8.0, vec![])),
            ],
        ),
        node(
            33,
            Some((1, "$")),
            "STICKY",
            "Idea",
            vec![
                ("size", xy(200.0, 200.0)),
                ("fillPaints", arr(vec![solid(1.0, 0.9, 0.4)])),
                ("fontSize", f(24.0)),
                ("textData", text_data("Ship it")),
            ],
        ),
        node(
            34,
            Some((1, "%")),
            "SHAPE_WITH_TEXT",
            "Bubble",
            vec![
                ("size", xy(100.0, 60.0)),
                ("transform", at(0.0, 400.0)),
                ("fillPaints", arr(vec![solid(0.2, 0.6, 1.0)])),
                ("shapeWithTextType", e("ELLIPSE")),
                ("fillGeometry", outline(0)),
                ("textData", text_data("Hi")),
            ],
        ),
        node(
            35,
            Some((1, "&")),
            "RECTANGLE",
            "Box",
            vec![("size", xy(100.0, 100.0)), ("transform", at(400.0, 0.0)), ("fillPaints", arr(vec![solid(0.8, 0.8, 0.8)]))],
        ),
        node(
            36,
            Some((1, "'")),
            "CONNECTOR",
            "Link",
            vec![
                stroke(),
                ("strokeWeight", f(2.0)),
                ("connectorStart", o(vec![("position", xy(0.0, 300.0))])),
                ("connectorEnd", o(vec![("endpointNodeID", g(35))])),
                ("connectorEndCap", e("ARROW_EQUILATERAL")),
                ("connectorLineStyle", e("ELBOWED")),
            ],
        ),
        node(
            37,
            Some((1, "(")),
            "TABLE",
            "Grid",
            vec![("size", xy(100.0, 50.0)), ("transform", at(300.0, 400.0)), ("strokePaints", arr(vec![solid(0.0, 0.0, 0.0)]))],
        ),
        node(
            39,
            Some((1, ")")),
            "FRAME",
            "Proto",
            vec![("size", xy(50.0, 50.0)), ("transform", at(500.0, 400.0)), ("prototypeInteractions", arr(vec![click]))],
        ),
    ];
    let mut broken = square_blob();
    broken.truncate(30);
    let blobs = [triangle_blob(), square_blob(), arrow_blob(), broken, join_fallback_blob()];
    o(vec![("nodeChanges", arr(nodes)), ("blobs", arr(blobs.into_iter().map(|b| o(vec![("bytes", Kv::Bytes(b))])).collect()))])
}

fn tail_canvas() -> Vec<u8> {
    canvas(&schema(), &tail_message())
}

fn region_canvas(fill_paints: Option<Vec<Kv<'static>>>, style_id: u32) -> Vec<u8> {
    let mut message = tail_message();
    let Kv::Array(nodes) = field_mut(&mut message, "nodeChanges").unwrap() else { panic!() };
    let wave = nodes.iter_mut().find(|node| node.get("name").and_then(Kv::str) == Some("Wave")).unwrap();
    let vector_data = field_mut(wave, "vectorData").unwrap();
    let Kv::Array(styles) = field_mut(vector_data, "styleOverrideTable").unwrap() else { panic!() };
    let Kv::Object(fields) = styles.first_mut().unwrap() else { panic!() };
    if let Some(fill_paints) = fill_paints {
        fields.push(("fillPaints", arr(fill_paints)));
    }
    let mut network = network::decode(&square_blob()).unwrap();
    network.regions[0].style = style_id;
    let Kv::Array(blobs) = field_mut(&mut message, "blobs").unwrap() else { panic!() };
    let bytes = field_mut(blobs.get_mut(1).unwrap(), "bytes").unwrap();
    *bytes = Kv::Bytes(network::encode(&network));
    canvas(&schema(), &message)
}

fn region_canvas_with_link(own: Option<Kv<'static>>, link_target: Option<u64>, linked_fill: Kv<'static>) -> Vec<u8> {
    region_canvas_with_typed_link(own, link_target, "FILL", linked_fill)
}

fn region_canvas_with_typed_link(own: Option<Kv<'static>>, link_target: Option<u64>, style_type: &'static str, linked_fill: Kv<'static>) -> Vec<u8> {
    let mut message = tail_message();
    let Kv::Array(nodes) = field_mut(&mut message, "nodeChanges").unwrap() else { panic!() };
    let wave = nodes.iter_mut().find(|node| node.get("name").and_then(Kv::str) == Some("Wave")).unwrap();
    let vector_data = field_mut(wave, "vectorData").unwrap();
    let Kv::Array(styles) = field_mut(vector_data, "styleOverrideTable").unwrap() else { panic!() };
    let Kv::Object(fields) = styles.first_mut().unwrap() else { panic!() };
    if let Some(own) = own {
        fields.push(("fillPaints", arr(vec![own])));
    }
    if let Some(target) = link_target {
        fields.push(("styleIdForFill", o(vec![("guid", g(target))])));
    }
    nodes.push(node(50, None, "RECTANGLE", "Linked Fill", vec![("styleType", e(style_type)), ("fillPaints", arr(vec![linked_fill]))]));
    let mut network = network::decode(&square_blob()).unwrap();
    network.regions[0].style = 1;
    let Kv::Array(blobs) = field_mut(&mut message, "blobs").unwrap() else { panic!() };
    *field_mut(blobs.get_mut(1).unwrap(), "bytes").unwrap() = Kv::Bytes(network::encode(&network));
    canvas(&schema(), &message)
}

fn field_mut<'a, 's>(value: &'a mut Kv<'s>, name: &str) -> Option<&'a mut Kv<'s>> {
    if let Kv::Object(fields) = value {
        return fields.iter_mut().find(|(key, _)| *key == name).map(|(_, field)| field);
    }
    None
}

#[test]
fn vector_region_fills_import_from_style_table() {
    let solid = o(vec![
        ("type", e("SOLID")),
        ("color", o(vec![("r", f(0.2)), ("g", f(0.4)), ("b", f(0.8)), ("a", f(0.5))])),
        ("opacity", f(0.5)),
        ("visible", Kv::Bool(false)),
    ]);
    let gradient = o(vec![
        ("type", e("GRADIENT_LINEAR")),
        (
            "stops",
            arr(vec![
                o(vec![("position", f(0.0)), ("color", o(vec![("r", f(1.0)), ("g", f(0.0)), ("b", f(0.0)), ("a", f(1.0))]))]),
                o(vec![("position", f(1.0)), ("color", o(vec![("r", f(0.0)), ("g", f(0.0)), ("b", f(1.0)), ("a", f(0.25))]))]),
            ]),
        ),
    ]);
    let image_hash = vec![0x12, 0x34, 0x56];
    let image = o(vec![("type", e("IMAGE")), ("image", o(vec![("hash", Kv::Bytes(image_hash.clone()))])), ("imageScaleMode", e("FILL"))]);
    let wire = region_canvas(Some(vec![solid, gradient, image]), 1);
    let (doc, warnings) = import(&wire, "Region fills").unwrap();
    let fills = network_of(&doc, "Wave").regions[0].fills.as_ref().unwrap();
    let figcraft_doc::color::PaintKind::Solid { color } = &fills[0].kind else { panic!("{fills:?}") };
    assert_eq!((color.r, color.g, color.b, fills[0].opacity, fills[0].visible), (0.2, 0.4, 0.8, 0.25, false));
    assert!(matches!(&fills[1].kind, figcraft_doc::color::PaintKind::Gradient(g) if g.stops.len() == 2 && g.stops[1].color.a == 0.25));
    let figcraft_doc::color::PaintKind::Image(imported_image) = &fills[2].kind else { panic!("{fills:?}") };
    let image_key = imported_image.image.clone();
    assert!(doc.images.contains_key(&image_key), "image reference must retain its image placeholder");
    assert!(!warnings.iter().any(|warning| warning.contains("their own fill use the layer's fill")), "{warnings:?}");

    let saved = crate::save_bytes(&doc, None).unwrap();
    let (reopened, _) = crate::load_bytes(&saved).unwrap();
    assert_eq!(network_of(&reopened, "Wave").regions[0].fills, Some(fills.clone()));
    assert!(reopened.images.contains_key(&image_key), "image reference must survive native save/reopen");
}

#[test]
fn vector_region_missing_and_empty_paints_remain_distinct() {
    let (inherited, _) = import(&region_canvas(None, 1), "Inherited region").unwrap();
    assert_eq!(network_of(&inherited, "Wave").regions[0].fills, None);
    let (empty, _) = import(&region_canvas(Some(Vec::new()), 1), "Empty region").unwrap();
    assert_eq!(network_of(&empty, "Wave").regions[0].fills, Some(Vec::new()));
}

#[test]
fn vector_region_linked_fill_uses_style_paints_without_recording_definition() {
    let linked = solid(0.9, 0.1, 0.2);
    let own = solid(0.1, 0.2, 0.9);
    let wire = region_canvas_with_link(Some(own), Some(50), linked);
    let (doc, _) = import(&wire, "Linked region style").unwrap();
    let fills = network_of(&doc, "Wave").regions[0].fills.as_ref().unwrap();
    let figcraft_doc::color::PaintKind::Solid { color } = &fills[0].kind else { panic!("{fills:?}") };
    assert_eq!((color.r, color.g, color.b), (0.9, 0.1, 0.2));
    assert!(doc.nodes().all(|node| node.name != "Linked Fill"), "style lookup must not add a style as a layer");
    let archive = Archive::open(&wire).unwrap();
    let (schema, message) = decode_canvas(&archive.canvas).unwrap();
    let message = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
    let (rest, _, _) = map::to_rest(&message, "Linked region style", crate::rest::MAX_NODES);
    assert!(rest.get("styles").is_none(), "regional link must not populate the layer styles map");
}

#[test]
fn vector_region_variable_bound_own_fill_wins_over_linked_style() {
    let own =
        o(vec![("type", e("SOLID")), ("color", o(vec![("r", f(0.1)), ("g", f(0.2)), ("b", f(0.9)), ("a", f(1.0))])), ("colorVar", alias_data(41))]);
    let (doc, _) = import(&region_canvas_with_link(Some(own), Some(50), solid(0.9, 0.1, 0.2)), "Bound region style").unwrap();
    let fills = network_of(&doc, "Wave").regions[0].fills.as_ref().unwrap();
    let figcraft_doc::color::PaintKind::Solid { color } = &fills[0].kind else { panic!("{fills:?}") };
    assert_eq!((color.r, color.g, color.b), (0.1, 0.2, 0.9));
}

#[test]
fn vector_region_missing_fills_inherit_and_unresolved_link_keeps_own_fill() {
    let (inherited, _) = import(&region_canvas_with_link(None, Some(50), solid(0.9, 0.1, 0.2)), "Inherited linked region").unwrap();
    assert_eq!(network_of(&inherited, "Wave").regions[0].fills, None);

    let own = solid(0.1, 0.2, 0.9);
    let (fallback, _) = import(&region_canvas_with_link(Some(own), Some(999), solid(0.9, 0.1, 0.2)), "Unresolved linked region").unwrap();
    let fills = network_of(&fallback, "Wave").regions[0].fills.as_ref().unwrap();
    let figcraft_doc::color::PaintKind::Solid { color } = &fills[0].kind else { panic!("{fills:?}") };
    assert_eq!((color.r, color.g, color.b), (0.1, 0.2, 0.9));
}

#[test]
fn vector_region_link_to_non_fill_style_keeps_own_fill() {
    let own = solid(0.1, 0.2, 0.9);
    let (doc, _) = import(&region_canvas_with_typed_link(Some(own), Some(50), "TEXT", solid(0.9, 0.1, 0.2)), "Text-linked region").unwrap();
    let fills = network_of(&doc, "Wave").regions[0].fills.as_ref().unwrap();
    let figcraft_doc::color::PaintKind::Solid { color } = &fills[0].kind else { panic!("{fills:?}") };
    assert_eq!((color.r, color.g, color.b), (0.1, 0.2, 0.9));
}

#[test]
fn vector_region_paint_budget_covers_the_whole_file() {
    // 190 gradients of 255 stops: 48,640 elements per layer, so four layers fit and a fifth doesn't.
    let stop = o(vec![("position", f(0.0)), ("color", o(vec![("r", f(1.0)), ("g", f(0.0)), ("b", f(0.0)), ("a", f(1.0))]))]);
    let gradient = o(vec![("type", e("GRADIENT_LINEAR")), ("stops", arr(vec![stop; 255]))]);
    let mut message = tail_message();
    let Kv::Array(nodes) = field_mut(&mut message, "nodeChanges").unwrap() else { panic!() };
    let wave = nodes.iter_mut().find(|node| node.get("name").and_then(Kv::str) == Some("Wave")).unwrap();
    let vector_data = field_mut(wave, "vectorData").unwrap();
    let Kv::Array(styles) = field_mut(vector_data, "styleOverrideTable").unwrap() else { panic!() };
    let Kv::Object(style_fields) = styles.first_mut().unwrap() else { panic!() };
    style_fields.push(("fillPaints", arr(vec![gradient; 190])));
    let wave = wave.clone();
    for (index, position) in ["w", "x", "y", "z"].into_iter().enumerate() {
        let mut copy = wave.clone();
        let Kv::Object(fields) = &mut copy else { panic!() };
        for (key, value) in fields.iter_mut() {
            match *key {
                "guid" => *value = g(60 + index as u64),
                "name" => *value = Kv::Str(format!("Wave {index}")),
                "parentIndex" => *value = o(vec![("guid", g(1)), ("position", s(position))]),
                _ => {}
            }
        }
        nodes.push(copy);
    }
    let mut network = network::decode(&square_blob()).unwrap();
    network.regions[0].style = 1;
    let Kv::Array(blobs) = field_mut(&mut message, "blobs").unwrap() else { panic!() };
    *field_mut(blobs.get_mut(1).unwrap(), "bytes").unwrap() = Kv::Bytes(network::encode(&network));

    let (doc, warnings) = import(&canvas(&schema(), &message), "Region paint budget").unwrap();
    let painted = ["Wave", "Wave 0", "Wave 1", "Wave 2", "Wave 3"].iter().filter(|name| network_of(&doc, name).regions[0].fills.is_some()).count();
    assert_eq!(painted, 4);
    assert!(warnings.iter().any(|warning| warning.contains("region paint data exceeded import limits")), "{warnings:?}");
}

#[test]
fn vector_regions_keep_distinct_solid_paints() {
    let mut message = tail_message();
    let Kv::Array(nodes) = field_mut(&mut message, "nodeChanges").unwrap() else { panic!() };
    let wave = nodes.iter_mut().find(|node| node.get("name").and_then(Kv::str) == Some("Wave")).unwrap();
    let vector_data = field_mut(wave, "vectorData").unwrap();
    let Kv::Array(styles) = field_mut(vector_data, "styleOverrideTable").unwrap() else { panic!() };
    let Kv::Object(first_style) = styles.first_mut().unwrap() else { panic!() };
    first_style.push(("fillPaints", arr(vec![solid(0.1, 0.2, 0.9)])));
    let Kv::Object(second_style) = o(vec![("styleID", u(2)), ("fillPaints", arr(vec![solid(0.9, 0.1, 0.2)]))]) else { panic!() };
    styles.push(Kv::Object(second_style));
    let mut network = network::decode(&square_blob()).unwrap();
    network.regions[0].style = 1;
    network.regions.push(network::Region { style: 2, non_zero: network.regions[0].non_zero, loops: network.regions[0].loops.clone() });
    let Kv::Array(blobs) = field_mut(&mut message, "blobs").unwrap() else { panic!() };
    let bytes = field_mut(blobs.get_mut(1).unwrap(), "bytes").unwrap();
    *bytes = Kv::Bytes(network::encode(&network));

    let (doc, _) = import(&canvas(&schema(), &message), "Two region fills").unwrap();
    let regions = &network_of(&doc, "Wave").regions;
    assert_eq!(regions.len(), 2);
    let color = |index: usize| match &regions[index].fills.as_ref().unwrap()[0].kind {
        figcraft_doc::color::PaintKind::Solid { color } => (color.r, color.g, color.b),
        paint => panic!("{paint:?}"),
    };
    assert_eq!(color(0), (0.1, 0.2, 0.9));
    assert_eq!(color(1), (0.9, 0.1, 0.2));
}

#[test]
fn vector_region_gradient_limit_warns_instead_of_silently_truncating() {
    for count in [256, 257] {
        let stops = (0..count)
            .map(|i| {
                let value = if i + 1 == count { 1.0 } else { 0.0 };
                o(vec![
                    ("position", f(i as f64 / (count - 1) as f64)),
                    ("color", o(vec![("r", f(value)), ("g", f(value)), ("b", f(value)), ("a", f(1.0))])),
                ])
            })
            .collect();
        let gradient = o(vec![("type", e("GRADIENT_LINEAR")), ("stops", arr(stops))]);
        let (doc, warnings) = import(&region_canvas(Some(vec![gradient]), 1), "Bounded region gradient").unwrap();
        let fills = &network_of(&doc, "Wave").regions[0].fills;
        if count == 256 {
            let figcraft_doc::color::PaintKind::Gradient(g) = &fills.as_ref().unwrap()[0].kind else { panic!() };
            assert_eq!(g.stops.len(), 256);
            assert_eq!(g.stops.last().unwrap().color.r, 1.0);
            assert!(!warnings.iter().any(|w| w.contains("vector region paint data exceeded")));
        } else {
            assert!(fills.is_none());
            assert!(warnings.iter().any(|w| w.contains("vector region paint data exceeded import limits")), "{warnings:?}");
        }
    }
}

#[test]
fn vector_region_invalid_paints_and_style_refs_warn() {
    let malformed = o(vec![("opacity", f(0.5))]);
    let unsupported = o(vec![("type", e("EMOJI"))]);
    let (doc, warnings) = import(&region_canvas(Some(vec![malformed, unsupported]), 1), "Invalid region paints").unwrap();
    assert_eq!(network_of(&doc, "Wave").regions[0].fills, Some(Vec::new()));
    assert!(warnings.iter().any(|warning| warning.contains("invalid vector region paints were skipped")), "{warnings:?}");
    assert!(warnings.iter().any(|warning| warning.contains("EMOJI paints are not supported")), "{warnings:?}");

    let (doc, warnings) = import(&region_canvas(None, 9), "Missing region style").unwrap();
    assert_eq!(network_of(&doc, "Wave").regions[0].fills, None);
    assert!(warnings.iter().any(|warning| warning.contains("vector region references a missing fill style")), "{warnings:?}");
}

fn network_of<'d>(doc: &'d figcraft_doc::Document, name: &str) -> &'d figcraft_doc::VectorNetwork {
    match &named(doc, name).kind {
        NodeKind::Vector { network } => network,
        k => panic!("{name} is {k:?}"),
    }
}

#[test]
fn vectors_import_as_editable_networks() {
    let (doc, warnings) = import(&tail_canvas(), "Tail").unwrap();
    // Scaled from the normalized size (100 × 50) to the layer's (200 × 100), corner and handles kept.
    let wave = network_of(&doc, "Wave");
    let pts: Vec<(f64, f64)> = wave.vertices.iter().map(|v| (v.x, v.y)).collect();
    assert_eq!(pts, [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
    assert_eq!((wave.vertices[1].radius, wave.vertices[1].mirror), (4.0, Some(HandleMirroring::AngleAndLength)));
    assert_eq!(wave.vertices[0].mirror, None);
    assert_eq!(
        wave.vertices.iter().map(|v| v.join).collect::<Vec<_>>(),
        [None, Some(StrokeJoin::Bevel), Some(StrokeJoin::Miter), Some(StrokeJoin::Round)]
    );
    assert_eq!(named(&doc, "Wave").stroke.join, StrokeJoin::Miter);
    assert_eq!((wave.segments[0].ta, wave.segments[0].tb), ((20.0, 0.0), (-20.0, 0.0)));
    assert_eq!(wave.regions.len(), 1);
    assert_eq!((wave.regions[0].loops.clone(), wave.regions[0].winding), (vec![vec![0, 1, 2, 3]], WindingRule::EvenOdd));
    assert_eq!(named(&doc, "Wave").fills.len(), 1);
    // An open line stays a stroke; its end point's own cap is the end cap.
    let arrow = named(&doc, "Arrow");
    assert_eq!(network_of(&doc, "Arrow").vertices.iter().map(|v| v.x).collect::<Vec<_>>(), [0.0, 300.0]);
    assert!(network_of(&doc, "Arrow").regions.is_empty());
    assert_eq!((arrow.stroke.cap_start, arrow.stroke.cap_end), (StrokeCap::None, StrokeCap::LineArrow));
    assert_eq!((arrow.strokes.len(), arrow.fills.len()), (1, 0));
    // A damaged network falls back to the outline.
    assert_eq!(network_of(&doc, "Broken").regions.len(), 1);
    assert!(warnings.iter().any(|w| w.contains("damaged vector networks import as outlines")), "{warnings:?}");
    let fallback = network_of(&doc, "Join Fallbacks");
    assert!(fallback.vertices.iter().all(|v| v.join.is_none()));
    assert_eq!(named(&doc, "Join Fallbacks").stroke.join, StrokeJoin::Miter);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("vector points refer to point settings missing from the file; they use the layer's joins and plain corners")),
        "{warnings:?}"
    );
    assert!(warnings.iter().any(|w| w.contains("unknown stroke joins use the layer join")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("dashed or variable-width strokes render with the layer join")), "{warnings:?}");
}

#[test]
fn per_segment_stroke_styles_are_reported() {
    let styled_import = |segment_style: u32| {
        let mut blob = square_blob();
        // The first segment's style id follows the counts and the vertices (12 bytes each).
        blob[12 + 4 * 12..12 + 4 * 12 + 4].copy_from_slice(&segment_style.to_le_bytes());
        let net = network::decode(&blob).unwrap();
        assert_eq!((net.segments.len(), net.segments[0].style), (4, segment_style));
        let mut message = tail_message();
        let Kv::Object(fields) = &mut message else { panic!() };
        let Kv::Array(blobs) = &mut fields.iter_mut().find(|(key, _)| *key == "blobs").unwrap().1 else { panic!() };
        blobs[1] = o(vec![("bytes", Kv::Bytes(blob))]);
        let (doc, warnings) = import(&canvas(&schema(), &message), "Tail").unwrap();
        assert_eq!(network_of(&doc, "Wave").segments.len(), 4);
        warnings
    };
    assert!(network::decode(&square_blob()).unwrap().segments.iter().all(|s| s.style == 0));
    let plain = import(&tail_canvas(), "Tail").unwrap().1;
    assert!(!plain.iter().any(|w| w.contains("vector segments")), "{plain:?}");
    let uninterpreted = |w: &String| w.contains("style settings of some vector segments aren't supported; those segments use the layer's stroke");
    let missing = |w: &String| w.contains("some vector segments refer to style settings missing from the file; they use the layer's stroke");
    // Style 1 has a row in the layer's override table; its contents are not applied.
    let known = styled_import(1);
    assert_eq!(known.iter().filter(|w| uninterpreted(w)).count(), 1, "{known:?}");
    assert!(!known.iter().any(missing), "{known:?}");
    // Style 7 names no row at all.
    let unknown = styled_import(7);
    assert_eq!(unknown.iter().filter(|w| missing(w)).count(), 1, "{unknown:?}");
    assert!(!unknown.iter().any(uninterpreted), "{unknown:?}");
}

#[test]
fn other_layer_kinds_import_as_the_closest_layers() {
    let (doc, warnings) = import(&tail_canvas(), "Tail").unwrap();
    let text_of = |id| match &doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.characters.clone(),
        k => panic!("{k:?}"),
    };
    // A sticky note: a frame with its paint, holding its text.
    let idea = named(&doc, "Idea");
    assert!(matches!(idea.kind, NodeKind::Frame));
    assert_eq!(idea.fills.len(), 1);
    assert_eq!(idea.children.len(), 1);
    assert_eq!(text_of(idea.children[0]), "Ship it");
    let NodeKind::Text(t) = &doc.node(idea.children[0]).unwrap().kind else { panic!() };
    assert_eq!(t.style.font_size, 24.0);
    // A shape with text: its outline as a vector (with the paint), then the text.
    let bubble = named(&doc, "Bubble");
    assert!(bubble.fills.is_empty());
    let kids: Vec<&figcraft_doc::Node> = bubble.children.iter().map(|c| doc.node(*c).unwrap()).collect();
    assert!(matches!(kids[0].kind, NodeKind::Vector { .. }) && kids[0].fills.len() == 1);
    assert_eq!(text_of(bubble.children[1]), "Hi");
    // An elbowed connector from a free point to a sibling: in right angles, into the side of the
    // box facing the point, with its end cap.
    let link = named(&doc, "Link");
    let world = doc.world_transform(link.id);
    let ends: Vec<(f64, f64)> =
        network_of(&doc, "Link").vertices.iter().map(|v| world * figcraft_doc::geom::Point::new(v.x, v.y)).map(|p| (p.x, p.y)).collect();
    assert_eq!(ends, [(0.0, 300.0), (200.0, 300.0), (200.0, 50.0), (400.0, 50.0)]);
    assert_eq!(link.stroke.cap_end, StrokeCap::TriangleArrow);
    // A table: a frame.
    assert!(matches!(named(&doc, "Grid").kind, NodeKind::Frame));
    for w in ["sticky layers import as frames", "shape with text layers", "connector layers import as vectors", "table layers", "elbowed"] {
        assert!(warnings.iter().any(|x| x.contains(w)), "{w}: {warnings:?}");
    }
}

/// Connectors saved without their path: straight to the point on the layer, elbowed and curved
/// from the side the end's point lies on, as a board connector would run.
#[test]
fn connectors_without_a_saved_path_follow_their_line_style() {
    let mut message = tail_message();
    let Kv::Array(nodes) = field_mut(&mut message, "nodeChanges").unwrap() else { panic!() };
    let box_at = |id, name: &str, x| node(id, Some((1, "*")), "RECTANGLE", name, vec![("size", xy(100.0, 100.0)), ("transform", at(x, 600.0))]);
    let joined = |id, name: &str, style: &'static str| {
        let side = |node, x| o(vec![("endpointNodeID", g(node)), ("relativePosition", xy(x, 0.5))]);
        node(
            id,
            Some((1, "+")),
            "CONNECTOR",
            name,
            vec![
                ("strokePaints", arr(vec![solid(0.0, 0.0, 0.0)])),
                ("connectorStart", side(60, 1.0)),
                ("connectorEnd", side(61, 0.0)),
                ("connectorLineStyle", e(style)),
            ],
        )
    };
    nodes.extend([box_at(60, "A", 0.0), box_at(61, "B", 300.0), joined(62, "Curve", "CURVED"), joined(63, "Line", "STRAIGHT")]);
    let (doc, warnings) = import(&canvas(&schema(), &message), "Routes").unwrap();
    let world = |name: &str| doc.world_transform(named(&doc, name).id);
    // Curved: one curve from A's right side to B's left, leaving and arriving level.
    let curve = network_of(&doc, "Curve");
    let ends: Vec<_> = curve.vertices.iter().map(|v| world("Curve") * figcraft_doc::geom::Point::new(v.x, v.y)).collect();
    assert_eq!(ends.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), [(100.0, 650.0), (300.0, 650.0)]);
    assert_eq!(curve.segments.len(), 1);
    assert_eq!((curve.segments[0].ta, curve.segments[0].tb), ((100.0, 0.0), (-100.0, 0.0)), "the curve leaves and arrives along the sides");
    // Straight: unchanged, to the points the file gives on the layers.
    let line: Vec<_> =
        network_of(&doc, "Line").vertices.iter().map(|v| world("Line") * figcraft_doc::geom::Point::new(v.x, v.y)).map(|p| (p.x, p.y)).collect();
    assert_eq!(line, [(100.0, 650.0), (300.0, 650.0)]);
    assert!(warnings.iter().any(|w| w.contains("FigCraft's own routing")), "{warnings:?}");
}

/// A connector with a label but no saved snapshot: a frame holding its line and its label,
/// centred on the line's middle, in black when the connector gives no paint.
#[test]
fn a_labelled_connector_without_a_snapshot_keeps_its_label() {
    let mut message = tail_message();
    let Kv::Array(nodes) = field_mut(&mut message, "nodeChanges").unwrap() else { panic!() };
    let box_at = |id, name: &str, x| node(id, Some((1, "*")), "RECTANGLE", name, vec![("size", xy(100.0, 100.0)), ("transform", at(x, 600.0))]);
    let side = |node, x| o(vec![("endpointNodeID", g(node)), ("relativePosition", xy(x, 0.5))]);
    let labelled = node(
        62,
        Some((1, "+")),
        "CONNECTOR",
        "Says",
        vec![
            ("strokePaints", arr(vec![solid(0.0, 0.0, 0.0)])),
            ("connectorStart", side(60, 1.0)),
            ("connectorEnd", side(61, 0.0)),
            ("connectorLineStyle", e("STRAIGHT")),
            ("textData", text_data("yes")),
            ("fontSize", f(20.0)),
        ],
    );
    nodes.extend([box_at(60, "A", 0.0), box_at(61, "B", 300.0), labelled]);
    let (doc, warnings) = import(&canvas(&schema(), &message), "Labels").unwrap();
    let says = named(&doc, "Says");
    assert!(matches!(says.kind, NodeKind::Frame), "{:?}", says.kind);
    assert!(says.fills.is_empty() && says.strokes.is_empty(), "the frame only holds the line and the label");
    let kids: Vec<&figcraft_doc::Node> = says.children.iter().map(|c| doc.node(*c).unwrap()).collect();
    assert_eq!(kids.len(), 2);
    assert!(matches!(kids[0].kind, NodeKind::Vector { .. }) && kids[0].strokes.len() == 1, "the line keeps its stroke");
    assert!(matches!(&kids[1].kind, NodeKind::Text(t) if t.characters == "yes"), "{:?}", kids[1].kind);
    assert_eq!(kids[1].fills.len(), 1, "black when the connector gives no paint");
    // Centred on the line's middle, (200, 650) on the page.
    let label = doc.world_transform(kids[1].id) * figcraft_doc::geom::Point::new(kids[1].width / 2.0, kids[1].height / 2.0);
    assert!((label.x - 200.0).abs() < 1e-6 && (label.y - 650.0).abs() < 1e-6, "{label:?}");
    assert!(warnings.iter().any(|w| w.contains("centred on the line")), "{warnings:?}");
    assert!(!warnings.iter().any(|w| w.contains("connector labels were skipped")), "{warnings:?}");
}

#[test]
fn variable_prototype_actions_import() {
    let (doc, warnings) = import(&tail_canvas(), "Tail").unwrap();
    let actions = &named(&doc, "Proto").interactions[0].actions;
    assert_eq!(actions.len(), 3, "{actions:?} {warnings:?}");
    assert!(matches!(&actions[0], Action::SetVariable { variable, value, .. } if variable == "1:41" && *value == Value::Bool(true)));
    let Action::Conditional { blocks } = &actions[1] else { panic!("{:?}", actions[1]) };
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].condition.is_some() && matches!(blocks[0].actions[..], [Action::Back]));
    assert!(blocks[1].condition.is_none() && matches!(blocks[1].actions[..], [Action::CloseOverlay]));
    assert!(matches!(&actions[2], Action::SetVariableMode { collection, mode } if collection == "1:40" && mode == "1:93"));
    assert!(!warnings.iter().any(|w| w.contains("prototype")), "{warnings:?}");
}

#[test]
fn unreadable_conditions_skip_the_conditional() {
    // A condition of a kind that can't be read must not become an "else".
    let bad = data("MAP", o(vec![]));
    let a =
        action("CONDITIONAL", vec![("conditionalActions", arr(vec![o(vec![("actions", arr(vec![action("BACK", vec![])])), ("condition", bad)])]))]);
    let it = o(vec![("event", o(vec![("interactionType", e("ON_CLICK"))])), ("actions", arr(vec![a]))]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "D", vec![]),
        node(1, Some((0, "!")), "CANVAS", "P", vec![]),
        node(2, Some((1, "!")), "FRAME", "F", vec![("size", xy(10.0, 10.0)), ("prototypeInteractions", arr(vec![it]))]),
    ];
    let mut sc = schema();
    // `MAP` isn't in the fixture's enum: add it.
    let dt = sc.defs.iter_mut().find(|d| d.name == "DataType").unwrap();
    let value = dt.fields.len() as u32;
    dt.fields.push(Field { name: "MAP".into(), ty: None, array: false, value });
    let (doc, warnings) = import(&canvas(&sc, &o(vec![("nodeChanges", arr(nodes))])), "x").unwrap();
    assert!(named(&doc, "F").interactions.is_empty());
    assert!(warnings.iter().any(|w| w.contains("conditions that can't be evaluated")), "{warnings:?}");
}

#[test]
fn damaged_network_blobs_never_panic() {
    let good = square_blob();
    assert!(network::decode(&good).is_some());
    for cut in 0..good.len() {
        assert!(network::decode(&good[..cut]).is_none(), "{cut}");
    }
    let mut r = Lcg(5);
    for _ in 0..5000 {
        let mut m = good.clone();
        for _ in 0..1 + r.next() % 4 {
            let i = (r.next() as usize) % m.len();
            m[i] ^= 1 << (r.next() % 8);
        }
        if let Some(net) = network::decode(&m) {
            assert!(net.segments.iter().all(|s| s.a < net.vertices.len() && s.b < net.vertices.len()));
            assert!(net.regions.iter().flat_map(|r| &r.loops).flatten().all(|i| *i < net.segments.len()));
            assert!(net.vertices.iter().all(|v| v.x.is_finite() && v.y.is_finite()));
        }
    }
    // Counts larger than the blob, and indices out of range.
    let mut huge = vec![0xff; 12];
    huge.extend_from_slice(&[0; 64]);
    assert!(network::decode(&huge).is_none());
    let mut bad = arrow_blob();
    bad[12 + 24 + 4..12 + 24 + 8].copy_from_slice(&7u32.to_le_bytes());
    assert!(network::decode(&bad).is_none());
    for _ in 0..2000 {
        let junk: Vec<u8> = (0..(r.next() % 120)).map(|_| r.next() as u8).collect();
        let _ = network::decode(&junk);
    }
}

#[test]
fn segment_styles_round_trip_without_changing_geometry() {
    let original = Network {
        vertices: vec![vtx(0, 2.0, 3.0), vtx(0, 10.0, 20.0), vtx(0, 40.0, 50.0), vtx(0, 60.0, 70.0)],
        segments: vec![
            seg(0, 1, (1.0, 2.0), (3.0, 4.0)),
            Segment { style: 17, a: 1, ta: (5.0, 6.0), b: 2, tb: (7.0, 8.0) },
            Segment { style: 27, a: 2, ta: (9.0, 10.0), b: 3, tb: (11.0, 12.0) },
        ],
        regions: vec![],
    };
    let bytes = network::encode(&original);
    let decoded = network::decode(&bytes).unwrap();
    assert_eq!(decoded.segments.len(), 3);
    for (got, want) in decoded.segments.iter().zip(&original.segments) {
        assert_eq!(got.style, want.style);
        assert_eq!((got.a, got.ta, got.b, got.tb), (want.a, want.ta, want.b, want.tb));
    }
    for cut in 0..bytes.len() {
        assert!(network::decode(&bytes[..cut]).is_none(), "truncated at {cut}");
    }
}

#[test]
fn mangled_tail_files_never_panic() {
    let c = tail_canvas();
    let mut r = Lcg(13);
    for cut in (0..c.len()).step_by(29) {
        let _ = import(&c[..cut], "x");
    }
    for _ in 0..300 {
        let mut m = c.clone();
        for _ in 0..1 + r.next() % 8 {
            let i = (r.next() as usize) % m.len();
            m[i] ^= 1 << (r.next() % 8);
        }
        let _ = import(&m, "x");
    }
    // Mangled messages decode and map without panicking.
    let sc = schema();
    let root = sc.find("Message").unwrap();
    let msg = encode::message(&sc, root, &tail_message()).unwrap();
    for _ in 0..2000 {
        let mut m = msg.clone();
        for _ in 0..1 + r.next() % 4 {
            let i = (r.next() as usize) % m.len();
            m[i] = r.next() as u8;
        }
        if let Ok(kv) = sc.decode_message(root, &m) {
            let (json, _, networks) = map::to_rest(&kv, "x", crate::rest::MAX_NODES);
            let _ = crate::rest::import_with_networks(&json, &crate::rest::no_images, networks, crate::rest::MAX_NODES);
        }
    }
}

#[test]
fn junk_vector_networks_in_json_fall_back() {
    let vector = |net: Value| {
        serde_json::json!({"document": {"id": "0:0", "type": "DOCUMENT", "children": [{"id": "0:1", "type": "CANVAS", "children": [{
            "id": "1:1", "type": "VECTOR", "name": "V", "size": {"x": 10, "y": 10}, "relativeTransform": [[1, 0, 0], [0, 1, 0]],
            "fills": [], "strokes": [], "fillGeometry": [{"path": "M0 0L10 0L10 10Z"}], "vectorNetwork": net}]}]}})
    };
    let cases = [
        serde_json::json!({"vertices": [{"x": 0, "y": 0}], "segments": [{"a": 0, "b": 5}]}),
        serde_json::json!({"vertices": [{"x": 1e300, "y": 0}, {"x": 0, "y": 0}], "segments": [{"a": 0, "b": 1}]}),
        serde_json::json!({"vertices": "no"}),
        serde_json::json!({"vertices": [{"x": 0, "y": 0}, {"x": 1, "y": 1}], "segments": [{"a": 0, "b": 1}], "regions": [{"loops": [[9]]}]}),
    ];
    for (i, net) in cases.into_iter().enumerate() {
        let (doc, w) = crate::rest::import(&vector(net), &crate::rest::no_images).unwrap();
        let NodeKind::Vector { network } = &named(&doc, "V").kind else { panic!() };
        assert!(!network.segments.is_empty(), "{i}");
        if i == 3 {
            // Usable apart from a loop that refers to nothing: kept, the loop dropped.
            assert!(network.regions.iter().all(|r| r.loops.is_empty()));
        } else {
            assert!(w.iter().any(|w| w.contains("vector networks import as outlines")), "{i}: {w:?}");
        }
    }
}

/// Local only: writes this module's fixture as a `.fig` file to the path `FIG_FIXTURE_OUT` names
/// (for opening it in the app).
#[test]
fn write_fixture() {
    if let Ok(out) = std::env::var("FIG_FIXTURE_OUT") {
        std::fs::write(out, zip_file(&tail_canvas())).unwrap();
    }
}
