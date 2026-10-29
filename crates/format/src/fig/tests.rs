//! `.fig` import tests on fixtures built in code: a small schema written in a compact text form,
//! a message encoded with our own kiwi encoder, compressed (deflate and zstd), framed and zipped.

use std::io::Write;

use figcraft_doc::{NodeKind, VariableType};
use serde_json::Value;

use super::kiwi::{Def, Field, Kind, Kv, Schema, Type, encode};
use super::*;

/// The fixture schema: `enum Name A B …`, `struct|message Name type field, type field[] …`.
/// Message field ids count from 1; enum values from 0.
const SCHEMA: &str = "
enum NodeType NONE DOCUMENT CANVAS GROUP FRAME BOOLEAN_OPERATION VECTOR STAR LINE ELLIPSE RECTANGLE REGULAR_POLYGON ROUNDED_RECTANGLE TEXT SLICE SYMBOL INSTANCE STICKY VARIABLE VARIABLE_SET SHAPE_WITH_TEXT CONNECTOR TABLE
enum PaintType SOLID GRADIENT_LINEAR IMAGE EMOJI
enum ImageScaleMode STRETCH FIT FILL TILE
enum WindingRule NONZERO ODD
enum StyleType NONE FILL TEXT
enum StackMode NONE HORIZONTAL VERTICAL
enum PropType BOOL TEXT
enum VariableField MISSING VISIBLE TEXT_DATA
enum DataType BOOLEAN FLOAT STRING ALIAS COLOR PROP_REF EXPRESSION
enum ResolvedType BOOLEAN FLOAT STRING COLOR
enum InteractionType ON_CLICK ON_HOVER
enum ConnectionType NONE INTERNAL_NODE URL BACK CLOSE SET_VARIABLE CONDITIONAL SET_VARIABLE_MODE
enum StrokeCap NONE ROUND SQUARE ARROW_LINES ARROW_EQUILATERAL
enum StrokeJoin MITER BEVEL ROUND
enum VectorMirror NONE ANGLE ANGLE_AND_LENGTH
enum ShapeWithTextType SQUARE ELLIPSE
enum ConnectorLineStyle ELBOWED STRAIGHT CURVED
enum ExpressionFunction ADDITION SUBTRACTION RESOLVE_VARIANT MULTIPLY DIVIDE EQUALS
enum NavigationType NAVIGATE OVERLAY
enum TransitionType INSTANT_TRANSITION DISSOLVE SMART_ANIMATE
enum EasingType IN_CUBIC OUT_CUBIC
enum ConstraintType MIN CENTER MAX STRETCH SCALE
enum LeadingTrim NONE CAP_HEIGHT
struct GUID uint sessionID, uint localID
struct Color float r, float g, float b, float a
struct Vector float x, float y
struct Matrix float m00, float m01, float m02, float m10, float m11, float m12
struct ColorStop Color color, float position
message Image byte[] hash, string name, uint dataBlob
message AssetRef string key, string version
message VariableID GUID guid, AssetRef assetRef
message PropRefValue GUID defId
message AnyValue bool boolValue, float floatValue, VariableID alias, Color colorValue, PropRefValue propRefValue, TextData textDataValue, string textValue, Expression expressionValue
message Expression ExpressionFunction expressionFunction, VariableData[] expressionArguments
message VariableTarget VariableID id
message VariableSetID GUID guid, AssetRef assetRef
message ConditionalActions Action[] actions, VariableData condition
message VectorData uint vectorNetworkBlob, Vector normalizedSize, NodeChange[] styleOverrideTable
message ConnectorEndpoint GUID endpointNodeID, Vector position, Vector relativePosition
message VariableData AnyValue value, DataType dataType
message Paint PaintType type, Color color, float opacity, bool visible, Image image, Matrix transform, ColorStop[] stops, VariableData colorVar, ImageScaleMode imageScaleMode
message ParentIndex GUID guid, string position
message FontName string family, string style, string postscript
message TextData string characters, uint[] characterStyleIDs, NodeChange[] styleOverrideTable
message Path WindingRule windingRule, uint commandsBlob
message GUIDPath GUID[] guids
message SymbolData GUID symbolID, NodeChange[] symbolOverrides
message StyleId GUID guid
message PropValue bool boolValue, TextData textValue
message PropDef GUID id, string name, PropValue initialValue, PropType type
message PropAssignment GUID defID, PropValue value
message MapEntry VariableData variableData, VariableField variableField
message VariableDataMap MapEntry[] entries
message SetMode GUID id, string name, string sortPosition
message ValuesEntry GUID modeID, VariableData variableData
message Values ValuesEntry[] entries
message ModeEntry VariableID variableSetID, GUID variableModeID
message ModeMap ModeEntry[] entries
message Event InteractionType interactionType
message Action GUID transitionNodeID, TransitionType transitionType, float transitionDuration, EasingType easingType, ConnectionType connectionType, NavigationType navigationType, VariableTarget targetVariable, VariableData targetVariableData, VariableSetID targetVariableSetID, GUID targetVariableModeID, ConditionalActions[] conditionalActions
message Interaction Event event, Action[] actions
message StartingPoint string name, string position
message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, bool visible, float opacity, Vector size, Matrix transform, Paint[] fillPaints, Paint[] strokePaints, float strokeWeight, float cornerRadius, bool resizeToFit, bool internalOnly, float fontSize, FontName fontName, TextData textData, uint styleID, Path[] fillGeometry, Path[] strokeGeometry, StyleType styleType, StyleId styleIdForFill, SymbolData symbolData, PropDef[] componentPropDefs, PropAssignment[] componentPropAssignments, VariableDataMap parameterConsumptionMap, SetMode[] variableSetModes, VariableID variableSetID, ResolvedType variableResolvedType, Values variableDataValues, ModeMap variableModeBySetMap, Interaction[] prototypeInteractions, StartingPoint prototypeStartingPoint, GUIDPath guidPath, StackMode stackMode, float stackSpacing, float stackHorizontalPadding, float stackVerticalPadding, uint count, float starInnerScale, VectorData vectorData, StrokeCap strokeCap, StrokeJoin strokeJoin, VectorMirror handleMirroring, ShapeWithTextType shapeWithTextType, ConnectorEndpoint connectorStart, ConnectorEndpoint connectorEnd, StrokeCap connectorStartCap, StrokeCap connectorEndCap, ConnectorLineStyle connectorLineStyle, ConstraintType horizontalConstraint, ConstraintType verticalConstraint, NodeChange[] derivedSymbolData, string key, string version, LeadingTrim leadingTrim, float[] dashPattern
message Blob byte[] bytes
message Message NodeChange[] nodeChanges, Blob[] blobs
";

fn schema() -> Schema {
    Schema::parse(SCHEMA).unwrap()
}

fn o(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}
fn f(x: f64) -> Kv<'static> {
    Kv::Float(x as f32)
}
fn u(x: u64) -> Kv<'static> {
    Kv::Uint(x)
}
fn e(n: &'static str) -> Kv<'static> {
    Kv::Enum(n)
}
fn s(x: &str) -> Kv<'static> {
    Kv::Str(x.into())
}
fn g(l: u64) -> Kv<'static> {
    o(vec![("sessionID", u(1)), ("localID", u(l))])
}
fn arr(v: Vec<Kv<'static>>) -> Kv<'static> {
    Kv::Array(v)
}
fn xy(x: f64, y: f64) -> Kv<'static> {
    o(vec![("x", f(x)), ("y", f(y))])
}
fn at(x: f64, y: f64) -> Kv<'static> {
    o(vec![("m00", f(1.0)), ("m01", f(0.0)), ("m02", f(x)), ("m10", f(0.0)), ("m11", f(1.0)), ("m12", f(y))])
}
fn rgb(r: f64, gr: f64, b: f64) -> Kv<'static> {
    o(vec![("r", f(r)), ("g", f(gr)), ("b", f(b)), ("a", f(1.0))])
}
fn solid(r: f64, gr: f64, b: f64) -> Kv<'static> {
    o(vec![("type", e("SOLID")), ("color", rgb(r, gr, b)), ("opacity", f(1.0)), ("visible", Kv::Bool(true))])
}
fn root_guid() -> Kv<'static> {
    o(vec![("sessionID", u(0)), ("localID", u(0))])
}
fn alias_data(l: u64) -> Kv<'static> {
    o(vec![("value", o(vec![("alias", o(vec![("guid", g(l))]))])), ("dataType", e("ALIAS"))])
}
fn prop_ref(def: u64, field: &'static str) -> Kv<'static> {
    let data = o(vec![("value", o(vec![("propRefValue", o(vec![("defId", g(def))]))])), ("dataType", e("PROP_REF"))]);
    o(vec![("entries", arr(vec![o(vec![("variableData", data), ("variableField", e(field))])]))])
}

/// A node: id (0: the document), parent (id, position), type, name and further fields.
fn node(id: u64, par: Option<(u64, &str)>, ty: &'static str, name: &str, mut more: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    let mut v = vec![("guid", if id == 0 { root_guid() } else { g(id) })];
    if let Some((p, pos)) = par {
        v.push(("parentIndex", o(vec![("guid", if p == 0 { root_guid() } else { g(p) }), ("position", s(pos))])));
    }
    v.push(("type", e(ty)));
    v.push(("name", s(name)));
    v.append(&mut more);
    o(v)
}

/// A path blob: a closed triangle.
fn triangle_blob() -> Vec<u8> {
    let mut b = Vec::new();
    for (cmd, pts) in [(1u8, vec![0.0f32, 0.0]), (2, vec![10.0, 0.0]), (2, vec![5.0, 8.0]), (0, vec![])] {
        b.push(cmd);
        for p in pts {
            b.extend_from_slice(&p.to_le_bytes());
        }
    }
    b
}

/// A 2×1 PNG.
pub(super) fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

const HASH: [u8; 20] = [0xab; 20];

/// The fixture message: a page with a frame holding a rectangle (image fill, click interaction),
/// a two-style text, a group with a triangle vector and a star, a layer with a linked fill style,
/// one with a variable-bound fill, and an instance of a component with a boolean property
/// (property off, child renamed); a flow and an explicit variable mode; an unsupported sticky
/// note; styles and variables on the internal page.
fn message() -> Kv<'static> {
    let modes = arr(vec![
        o(vec![("id", g(90)), ("name", s("Light")), ("sortPosition", s("!"))]),
        o(vec![("id", g(91)), ("name", s("Dark")), ("sortPosition", s("\""))]),
    ]);
    let color_value = |c| o(vec![("value", o(vec![("colorValue", c)])), ("dataType", e("COLOR"))]);
    let values = o(vec![(
        "entries",
        arr(vec![
            o(vec![("modeID", g(90)), ("variableData", color_value(rgb(1.0, 1.0, 1.0)))]),
            o(vec![("modeID", g(91)), ("variableData", color_value(rgb(0.0, 0.0, 0.0)))]),
        ]),
    )]);
    let click = o(vec![
        ("event", o(vec![("interactionType", e("ON_CLICK"))])),
        (
            "actions",
            arr(vec![o(vec![
                ("transitionNodeID", g(20)),
                ("transitionType", e("SMART_ANIMATE")),
                ("transitionDuration", f(0.25)),
                ("easingType", e("OUT_CUBIC")),
                ("connectionType", e("INTERNAL_NODE")),
                ("navigationType", e("NAVIGATE")),
            ])]),
        ),
    ]);
    let font = |style: &str| o(vec![("family", s("Inter")), ("style", s(style)), ("postscript", s(""))]);
    let text = o(vec![
        ("characters", s("Hi there")),
        ("characterStyleIDs", arr(vec![u(0), u(0), u(0), u(5), u(5), u(5)])),
        ("styleOverrideTable", arr(vec![o(vec![("fontName", font("Bold")), ("styleID", u(5))])])),
    ]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page A", vec![]),
        node(2, Some((0, "~")), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))]),
        node(3, Some((2, "!")), "RECTANGLE", "Brand", vec![("styleType", e("FILL")), ("fillPaints", arr(vec![solid(0.0, 0.0, 1.0)]))]),
        node(4, Some((2, "\"")), "VARIABLE_SET", "Theme", vec![("variableSetModes", modes)]),
        node(
            5,
            Some((2, "#")),
            "VARIABLE",
            "surface",
            vec![("variableSetID", o(vec![("guid", g(4))])), ("variableResolvedType", e("COLOR")), ("variableDataValues", values)],
        ),
        // Listed before the frame, but its position puts it second.
        node(
            20,
            Some((1, "\"")),
            "SYMBOL",
            "Button",
            vec![
                ("size", xy(100.0, 40.0)),
                ("transform", at(500.0, 0.0)),
                ("fillPaints", arr(vec![solid(0.0, 1.0, 0.0)])),
                (
                    "componentPropDefs",
                    arr(vec![o(vec![
                        ("id", g(80)),
                        ("name", s("Show icon")),
                        ("initialValue", o(vec![("boolValue", Kv::Bool(true))])),
                        ("type", e("BOOL")),
                    ])]),
                ),
                ("stackMode", e("HORIZONTAL")),
                ("stackSpacing", f(8.0)),
                ("stackHorizontalPadding", f(12.0)),
                ("stackVerticalPadding", f(4.0)),
            ],
        ),
        node(
            21,
            Some((20, "!")),
            "ELLIPSE",
            "Icon",
            vec![("size", xy(16.0, 16.0)), ("transform", at(12.0, 12.0)), ("parameterConsumptionMap", prop_ref(80, "VISIBLE"))],
        ),
        node(
            10,
            Some((1, "!")),
            "FRAME",
            "Screen",
            vec![
                ("size", xy(400.0, 300.0)),
                ("fillPaints", arr(vec![solid(1.0, 1.0, 1.0)])),
                ("prototypeStartingPoint", o(vec![("name", s("Main flow")), ("position", s("!"))])),
                (
                    "variableModeBySetMap",
                    o(vec![("entries", arr(vec![o(vec![("variableSetID", o(vec![("guid", g(4))])), ("variableModeID", g(91))])]))]),
                ),
            ],
        ),
        node(
            11,
            Some((10, "!")),
            "ROUNDED_RECTANGLE",
            "Photo",
            vec![
                ("size", xy(50.0, 40.0)),
                ("transform", at(10.0, 20.0)),
                ("cornerRadius", f(6.0)),
                ("fillPaints", arr(vec![o(vec![("type", e("IMAGE")), ("image", o(vec![("hash", Kv::Bytes(HASH.to_vec()))]))])])),
                ("strokePaints", arr(vec![solid(0.0, 0.0, 0.0)])),
                ("strokeWeight", f(2.0)),
                ("prototypeInteractions", arr(vec![click])),
            ],
        ),
        node(
            12,
            Some((10, "\"")),
            "TEXT",
            "Title",
            vec![
                ("size", xy(120.0, 20.0)),
                ("transform", at(10.0, 80.0)),
                ("fillPaints", arr(vec![solid(0.0, 0.0, 0.0)])),
                ("fontSize", f(16.0)),
                ("fontName", font("Regular")),
                ("leadingTrim", e("CAP_HEIGHT")),
                ("textData", text),
            ],
        ),
        node(13, Some((10, "#")), "FRAME", "Group", vec![("size", xy(30.0, 30.0)), ("transform", at(100.0, 100.0)), ("resizeToFit", Kv::Bool(true))]),
        node(
            14,
            Some((13, "!")),
            "VECTOR",
            "Tri",
            vec![
                ("size", xy(10.0, 8.0)),
                ("transform", at(5.0, 6.0)),
                ("fillPaints", arr(vec![solid(1.0, 0.0, 0.0)])),
                ("fillGeometry", arr(vec![o(vec![("windingRule", e("NONZERO")), ("commandsBlob", u(0))])])),
            ],
        ),
        node(
            15,
            Some((13, "\"")),
            "STAR",
            "Star",
            vec![("size", xy(20.0, 20.0)), ("transform", at(10.0, 10.0)), ("count", u(6)), ("starInnerScale", f(0.5))],
        ),
        node(
            16,
            Some((10, "$")),
            "ROUNDED_RECTANGLE",
            "Styled",
            vec![
                ("size", xy(10.0, 10.0)),
                ("transform", at(0.0, 200.0)),
                // A stale copy of the style's paint: the style wins.
                ("fillPaints", arr(vec![solid(0.5, 0.5, 0.5)])),
                ("styleIdForFill", o(vec![("guid", g(3))])),
            ],
        ),
        node(
            17,
            Some((10, "%")),
            "ROUNDED_RECTANGLE",
            "Card",
            vec![
                ("size", xy(10.0, 10.0)),
                ("transform", at(20.0, 200.0)),
                ("fillPaints", arr(vec![o(vec![("type", e("SOLID")), ("color", rgb(0.0, 0.0, 0.0)), ("colorVar", alias_data(5))])])),
            ],
        ),
        node(
            18,
            Some((10, "&")),
            "INSTANCE",
            "Button copy",
            vec![
                ("size", xy(100.0, 40.0)),
                ("transform", at(200.0, 200.0)),
                ("fillPaints", arr(vec![solid(1.0, 0.0, 0.0)])),
                ("stackMode", e("HORIZONTAL")),
                (
                    "symbolData",
                    o(vec![
                        ("symbolID", g(20)),
                        ("symbolOverrides", arr(vec![o(vec![("guidPath", o(vec![("guids", arr(vec![g(21)]))])), ("name", s("Renamed icon"))])])),
                    ]),
                ),
                ("componentPropAssignments", arr(vec![o(vec![("defID", g(80)), ("value", o(vec![("boolValue", Kv::Bool(false))]))])])),
            ],
        ),
        node(19, Some((10, "'")), "STICKY", "Note", vec![]),
    ];
    o(vec![("nodeChanges", arr(nodes)), ("blobs", arr(vec![o(vec![("bytes", Kv::Bytes(triangle_blob()))])]))])
}

/// `canvas.fig`: magic, version, deflated schema, zstd message.
pub(super) fn canvas(schema: &Schema, msg: &Kv) -> Vec<u8> {
    let root = schema.find("Message").unwrap();
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&encode::schema(schema)).unwrap();
    let schema_chunk = z.finish().unwrap();
    let msg_chunk = ruzstd::encoding::compress_to_vec(&encode::message(schema, root, msg).unwrap()[..], ruzstd::encoding::CompressionLevel::Fastest);
    let mut out = b"fig-kiwi".to_vec();
    out.extend_from_slice(&101u32.to_le_bytes());
    for c in [schema_chunk, msg_chunk] {
        out.extend_from_slice(&(c.len() as u32).to_le_bytes());
        out.extend_from_slice(&c);
    }
    out
}

fn zip_file(canvas: &[u8]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let image_name = format!("images/{}", HASH.iter().map(|b| format!("{b:02x}")).collect::<String>());
    for (name, bytes) in [("canvas.fig", canvas.to_vec()), ("meta.json", br#"{"file_name":"Fixture"}"#.to_vec()), (image_name.as_str(), png())] {
        z.start_file(name, stored).unwrap();
        z.write_all(&bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn fixture() -> Vec<u8> {
    zip_file(&canvas(&schema(), &message()))
}

fn named<'d>(doc: &'d figcraft_doc::Document, name: &str) -> &'d figcraft_doc::Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no layer {name}"))
}

/// The shared schema plus the grid sizing definitions only these tests use.
fn grid_schema() -> Schema {
    const NODE_FIELDS: &str = ", GridTrackMap gridColumns, GridTrackMap gridRows, GridTrackSizeMap gridColumnsSizing, GridTrackSizeMap gridRowsSizing, GUID gridColumnAnchor, GUID gridRowAnchor, uint gridColumnSpan, uint gridRowSpan, float gridColumnGap, float gridRowGap";
    const TYPES: &str = "
enum GridTrackSizingType FLEX FIXED HUG UNKNOWN
message GridTrackSizeFunction GridTrackSizingType type, float value
message GridTrackSize GridTrackSizeFunction minSizing, GridTrackSizeFunction maxSizing
message GridTrackSizeEntry GUID id, GridTrackSize trackSize
message GridTrackSizeMap GridTrackSizeEntry[] entries
message GridTrackEntry GUID id, string position
message GridTrackMap GridTrackEntry[] entries
message Blob ";
    let text = SCHEMA.replace("enum StackMode NONE HORIZONTAL VERTICAL\n", "enum StackMode NONE HORIZONTAL VERTICAL GRID\n").replacen(
        "\nmessage Blob ",
        &format!("{NODE_FIELDS}{TYPES}"),
        1,
    );
    Schema::parse(&text).unwrap()
}

fn track_size(kind: &'static str, value: f64) -> Kv<'static> {
    let sizing = || o(vec![("type", e(kind)), ("value", f(value))]);
    o(vec![("minSizing", sizing()), ("maxSizing", sizing())])
}

fn ranged_track_size(min: Kv<'static>, max: Kv<'static>) -> Kv<'static> {
    o(vec![("minSizing", min), ("maxSizing", max)])
}

fn track_map_entry(id: u64, size: Kv<'static>) -> Kv<'static> {
    o(vec![("id", g(id)), ("trackSize", size)])
}

fn positioned_track(id: u64, position: &'static str) -> Kv<'static> {
    o(vec![("id", g(id)), ("position", s(position))])
}

fn unpositioned_track(id: u64) -> Kv<'static> {
    o(vec![("id", g(id))])
}

#[test]
fn grid_tracks_import_by_guid_and_keep_track_order() {
    let cols = arr(vec![positioned_track(13, "$"), unpositioned_track(10), positioned_track(12, "#"), unpositioned_track(11)]);
    let rows = arr(vec![positioned_track(21, "\""), positioned_track(20, "!")]);
    // Sizing entries intentionally differ from track order. Their GUIDs define the join.
    let col_sizes = arr(vec![
        track_map_entry(12, track_size("FLEX", 1.0)),
        track_map_entry(10, track_size("FIXED", 120.0)),
        track_map_entry(13, track_size("FLEX", 2.0)),
        track_map_entry(11, track_size("HUG", 80.0)),
    ]);
    let row_sizes = arr(vec![track_map_entry(21, track_size("HUG", 30.0)), track_map_entry(20, track_size("FIXED", 50.0))]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(
            2,
            Some((1, "!")),
            "FRAME",
            "Grid",
            vec![
                ("size", xy(600.0, 300.0)),
                ("transform", at(0.0, 0.0)),
                ("stackMode", e("GRID")),
                ("gridColumns", o(vec![("entries", cols)])),
                ("gridRows", o(vec![("entries", rows)])),
                ("gridColumnsSizing", o(vec![("entries", col_sizes)])),
                ("gridRowsSizing", o(vec![("entries", row_sizes)])),
                ("gridColumnGap", f(10.0)),
                ("gridRowGap", f(12.0)),
            ],
        ),
        node(
            3,
            Some((2, "!")),
            "RECTANGLE",
            "Hug child",
            vec![
                ("size", xy(80.0, 30.0)),
                ("transform", at(0.0, 0.0)),
                ("gridColumnAnchor", g(11)),
                ("gridRowAnchor", g(21)),
                ("gridColumnSpan", u(2)),
                ("gridRowSpan", u(1)),
            ],
        ),
    ];
    let bytes = canvas(&grid_schema(), &o(vec![("nodeChanges", arr(nodes))]));
    let (doc, warnings) = import(&bytes, "Grid tracks").unwrap();
    let grid = named(&doc, "Grid").layout.as_ref().unwrap();
    assert_eq!((grid.mode, grid.grid_columns, grid.grid_rows, grid.gap, grid.cross_gap), (figcraft_doc::LayoutMode::Grid, 4, 2, 10.0, 12.0));
    assert_eq!(
        grid.column_tracks,
        vec![
            figcraft_doc::GridTrack::fixed(120.0),
            figcraft_doc::GridTrack::hug(),
            figcraft_doc::GridTrack::fill(1.0),
            figcraft_doc::GridTrack::fill(2.0),
        ]
    );
    assert_eq!(grid.row_tracks, vec![figcraft_doc::GridTrack::fixed(50.0), figcraft_doc::GridTrack::hug()]);
    let child = &named(&doc, "Hug child").child_layout;
    assert_eq!((child.grid_column, child.grid_row, child.grid_column_span, child.grid_row_span), (Some(1), Some(1), 2, 1));
    assert!(!warnings.iter().any(|w| w.contains("grid track")), "{warnings:?}");
}

#[test]
fn malformed_grid_tracks_warn_and_keep_equal_track_fallbacks() {
    let tracks = arr(vec![positioned_track(30, "!"), positioned_track(31, "\""), positioned_track(32, "#"), positioned_track(33, "$")]);
    let sizing = arr(vec![
        track_map_entry(30, track_size("FIXED", -1.0)),
        track_map_entry(31, ranged_track_size(o(vec![("type", e("FLEX")), ("value", f(1.0))]), o(vec![("type", e("FLEX")), ("value", f(2.0))]))),
        track_map_entry(33, track_size("UNKNOWN", 5.0)),
        track_map_entry(34, track_size("FLEX", 3.0)),
    ]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(
            2,
            Some((1, "!")),
            "FRAME",
            "Grid",
            vec![
                ("size", xy(100.0, 100.0)),
                ("transform", at(0.0, 0.0)),
                ("stackMode", e("GRID")),
                ("gridColumns", o(vec![("entries", tracks)])),
                ("gridColumnsSizing", o(vec![("entries", sizing)])),
            ],
        ),
    ];
    let bytes = canvas(&grid_schema(), &o(vec![("nodeChanges", arr(nodes))]));
    let (doc, warnings) = import(&bytes, "Malformed grid tracks").unwrap();
    let layout = named(&doc, "Grid").layout.as_ref().unwrap();
    assert_eq!(layout.column_tracks, vec![figcraft_doc::GridTrack::default(); 4]);
    assert!(warnings.iter().any(|w| w.contains("grid track sizing is unsupported or malformed")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("stale or invalid grid track sizing entries")), "{warnings:?}");
}

#[test]
fn oversized_grid_track_lists_warn_and_use_default_tracks() {
    let oversized: Vec<Kv<'static>> = (0..1_001).map(|i| unpositioned_track(100 + i)).collect();
    let sizing = vec![track_map_entry(100, track_size("FIXED", 40.0))];
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
        node(
            2,
            Some((1, "!")),
            "FRAME",
            "Grid",
            vec![
                ("size", xy(100.0, 100.0)),
                ("transform", at(0.0, 0.0)),
                ("stackMode", e("GRID")),
                ("gridColumns", o(vec![("entries", arr(oversized))])),
                ("gridColumnsSizing", o(vec![("entries", arr(sizing))])),
            ],
        ),
    ];
    let bytes = canvas(&grid_schema(), &o(vec![("nodeChanges", arr(nodes))]));
    let (doc, warnings) = import(&bytes, "Oversized grid tracks").unwrap();
    let layout = named(&doc, "Grid").layout.as_ref().unwrap();
    assert!(layout.column_tracks.is_empty(), "{:?}", layout.column_tracks.len());
    assert!(warnings.iter().any(|w| w.contains("grid track list exceeds 1000 entries")), "{warnings:?}");
}

#[test]
fn kiwi_round_trip() {
    let sc = schema();
    let back = Schema::decode(&encode::schema(&sc)).unwrap();
    assert_eq!(back.defs.len(), sc.defs.len());
    for (a, b) in sc.defs.iter().zip(&back.defs) {
        assert_eq!((a.name.as_str(), a.kind), (b.name.as_str(), b.kind));
        let fields = |d: &Def| d.fields.iter().map(|f| (f.name.clone(), f.ty, f.array, f.value)).collect::<Vec<_>>();
        assert_eq!(fields(a), fields(b));
    }
    let msg = message();
    let root = sc.find("Message").unwrap();
    assert_eq!(back.decode_message(root, &encode::message(&sc, root, &msg).unwrap()).unwrap(), msg);
}

#[test]
fn kiwi_scalars() {
    let names = ["f", "i", "u", "i64", "u64", "s", "b"];
    let types = [Type::Float, Type::Int, Type::Uint, Type::Int64, Type::Uint64, Type::Str, Type::Bool];
    let fields = names
        .iter()
        .zip(types)
        .enumerate()
        .map(|(i, (n, t))| Field { name: n.to_string(), ty: Some(t), array: false, value: i as u32 + 1 })
        .collect();
    let sc = Schema { defs: vec![Def { name: "M".into(), kind: Kind::Message, fields }] };
    let cases = [
        (0.0f32, 0i32, 0u32, 0i64, 0u64),
        (-1.5, -7, 300, i64::MIN, u64::MAX),
        (3.25e7, i32::MAX, u32::MAX, i64::MAX, 1 << 60),
        (1e-40, i32::MIN, 1, -1, 127),
    ];
    for (fv, iv, uv, i64v, u64v) in cases {
        // Written by hand: the generic encoder goes through f64, which can't hold every 64-bit value.
        let mut w = encode::Writer::default();
        w.var_uint(1);
        w.var_float(fv);
        w.var_uint(2);
        w.var_int(iv);
        w.var_uint(3);
        w.var_uint(uv);
        w.var_uint(4);
        w.var_int64(i64v);
        w.var_uint(5);
        w.var_uint64(u64v);
        w.var_uint(6);
        w.string("héllo");
        w.var_uint(7);
        w.byte(1);
        w.var_uint(0);
        let got = sc.decode_message(0, &w.0).unwrap();
        // Denormals are stored as zero.
        let want_f = if fv.abs() < f32::MIN_POSITIVE { 0.0 } else { fv };
        assert_eq!(got.get("f"), Some(&Kv::Float(want_f)));
        assert_eq!(got.get("i"), Some(&Kv::Int(i64::from(iv))));
        assert_eq!(got.get("u"), Some(&Kv::Uint(u64::from(uv))));
        assert_eq!(got.get("i64"), Some(&Kv::Int(i64v)));
        assert_eq!(got.get("u64"), Some(&Kv::Uint(u64v)));
        assert_eq!(got.get("s"), Some(&s("héllo")));
        assert_eq!(got.get("b"), Some(&Kv::Bool(true)));
    }
}

#[test]
fn imports_the_fixture() {
    let bytes = fixture();
    assert!(is_fig(&bytes));
    let (doc, warnings) = import(&bytes, "fallback").unwrap();
    assert_eq!(doc.name, "Fixture");
    assert_fixture_layers(&doc);
    assert_fixture_styles(&doc);
    assert_fixture_variables(&doc);
    assert_fixture_components(&doc);
    assert_fixture_prototype(&doc, &warnings);
}

fn assert_fixture_layers(doc: &Document) {
    // The internal page is skipped.
    assert_eq!(doc.pages.len(), 1);
    let page = doc.node(doc.pages[0]).unwrap();
    assert_eq!(page.name, "Page A");
    // Siblings by position: the frame comes first even though it's listed after the component.
    assert_eq!(doc.node(page.children[0]).unwrap().name, "Screen");
    let screen = named(doc, "Screen");
    let names: Vec<&str> = screen.children.iter().map(|c| doc.node(*c).unwrap().name.as_str()).collect();
    assert_eq!(names, ["Photo", "Title", "Group", "Styled", "Card", "Button copy", "Note"]);
    // Image fill from the archive.
    let photo = named(doc, "Photo");
    let figcraft_doc::color::PaintKind::Image(img) = &photo.fills[0].kind else { panic!("not an image fill") };
    assert_eq!(doc.images.get(&img.image).map(|i| (i.width, i.height)), Some((2, 1)));
    assert_eq!((photo.corners.radii[0], photo.stroke.weight), (6.0, 2.0));
    // Text with a bold run over "the".
    let NodeKind::Text(t) = &named(doc, "Title").kind else { panic!("not text") };
    assert_eq!(t.characters, "Hi there");
    assert_eq!(t.style.font_size, 16.0);
    assert_eq!(t.leading_trim, figcraft_doc::LeadingTrim::CapHeight, "vertical trim (#466)");
    assert_eq!(t.runs.len(), 1);
    assert_eq!((t.runs[0].start, t.runs[0].end, t.runs[0].style.font_style.as_str()), (3, 6, "Bold"));
    // The group's children keep their place on the page (group at 100, 100).
    assert!(matches!(named(doc, "Group").kind, NodeKind::Group));
    let tri = named(doc, "Tri");
    assert!(matches!(&tri.kind, NodeKind::Vector { network } if !network.regions.is_empty()));
    let abs = doc.world_transform(tri.id).translation();
    assert!((abs.x - 105.0).abs() < 1e-6 && (abs.y - 106.0).abs() < 1e-6, "{abs:?}");
    assert!(matches!(named(doc, "Star").kind, NodeKind::Star { count: 6, .. }));
}

fn assert_fixture_styles(doc: &Document) {
    let styled = named(doc, "Styled");
    assert_eq!(styled.styles.fill.as_deref(), Some("1:3"));
    assert!(doc.styles.iter().any(|s| s.name == "Brand"));
    let figcraft_doc::color::PaintKind::Solid { color } = styled.fills[0].kind else { panic!("not solid") };
    assert_eq!((color.r, color.g, color.b), (0.0, 0.0, 1.0));
}

fn assert_fixture_variables(doc: &Document) {
    let screen = named(doc, "Screen");
    assert_eq!(doc.variables.collections.len(), 1);
    assert_eq!(doc.variables.variables[0].kind, VariableType::Color);
    assert_eq!(named(doc, "Card").bound.get("fill").map(String::as_str), Some("1:5"));
    assert_eq!(screen.var_modes.get("1:4").map(String::as_str), Some("1:91"));
}

fn assert_fixture_components(doc: &Document) {
    let button = named(doc, "Button");
    assert!(matches!(&button.kind, NodeKind::Component { props, .. } if props.contains_key("Show icon")));
    let inst = named(doc, "Button copy");
    assert!(
        matches!(&inst.kind, NodeKind::Instance { component, props } if *component == button.id && props.get("Show icon") == Some(&Value::Bool(false)))
    );
    let icon = doc.node(inst.children[0]).unwrap();
    assert_eq!(icon.name, "Renamed icon");
    assert!(!icon.visible);
}

fn assert_fixture_prototype(doc: &Document, warnings: &[String]) {
    let button = named(doc, "Button");
    let screen = named(doc, "Screen");
    let it = &named(doc, "Photo").interactions[0];
    assert!(matches!(it.actions[0], figcraft_doc::proto::Action::Navigate { dest, .. } if dest == button.id));
    assert_eq!(it.duration_ms, 250);
    let Some(NodeKind::Page { flows, .. }) = doc.node(doc.pages[0]).map(|p| &p.kind) else { panic!("no page") };
    assert_eq!((flows[0].name.as_str(), flows[0].start), ("Main flow", screen.id));
    // The sticky note comes in as a frame, with a warning.
    assert!(matches!(named(doc, "Note").kind, NodeKind::Frame));
    assert!(warnings.iter().any(|w| w.contains("sticky layers import as frames")), "{warnings:?}");
}

#[test]
fn bare_canvas_and_zlib_chunks_open() {
    let sc = schema();
    let c = canvas(&sc, &message());
    let (doc, w) = import(&c, "Bare").unwrap();
    assert_eq!(doc.name, "Bare");
    // Without the archive the image is a placeholder, reported.
    assert!(w.iter().any(|w| w.contains("isn't in the file")), "{w:?}");
    // A zlib-wrapped schema chunk reads too.
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&encode::schema(&sc)).unwrap();
    let zl = z.finish().unwrap();
    let parts = chunks(&c).unwrap();
    let mut out = c[..12].to_vec();
    for ch in [zl.as_slice(), parts[1]] {
        out.extend_from_slice(&(ch.len() as u32).to_le_bytes());
        out.extend_from_slice(ch);
    }
    assert!(import(&out, "Z").is_ok());
}

/// The fixture message with the photo's image bytes in its own blobs, as copies written by plugins
/// and tools carry them: the PNG goes at the end of `blobs` and `image.dataBlob` points `shift`
/// places past it (0 is the PNG itself).
fn message_with_image_in_blobs(shift: u64) -> Kv<'static> {
    fn point_images_at(value: &mut Kv<'static>, blob: u64) {
        match value {
            Kv::Object(fields) => {
                for (name, field) in fields.iter_mut() {
                    match field {
                        Kv::Object(image) if *name == "image" => image.push(("dataBlob", u(blob))),
                        _ => point_images_at(field, blob),
                    }
                }
            }
            Kv::Array(items) => items.iter_mut().for_each(|item| point_images_at(item, blob)),
            _ => {}
        }
    }
    let mut msg = message();
    let index = {
        let Kv::Object(fields) = &mut msg else { panic!("the message is an object") };
        let Some((_, Kv::Array(blobs))) = fields.iter_mut().find(|(name, _)| *name == "blobs") else { panic!("no blobs") };
        blobs.push(o(vec![("bytes", Kv::Bytes(png()))]));
        blobs.len() as u64 - 1
    };
    point_images_at(&mut msg, index + shift);
    msg
}

#[test]
fn a_bare_canvas_reads_images_carried_in_its_blobs() {
    // The clipboard holds a bare canvas (no `images/`); the image comes from `image.dataBlob`.
    let c = canvas(&schema(), &message_with_image_in_blobs(0));
    let (doc, warnings) = import(&c, "Pasted").unwrap();
    let photo = named(&doc, "Photo");
    let figcraft_doc::color::PaintKind::Image(img) = &photo.fills[0].kind else { panic!("not an image fill") };
    assert_eq!(doc.images.get(&img.image).map(|i| (i.width, i.height)), Some((2, 1)));
    assert!(!warnings.iter().any(|w| w.contains("isn't in the file")), "{warnings:?}");
}

#[test]
fn a_data_blob_past_the_blobs_stays_a_placeholder() {
    let c = canvas(&schema(), &message_with_image_in_blobs(5));
    let (_, warnings) = import(&c, "Pasted").unwrap();
    assert!(warnings.iter().any(|w| w.contains("isn't in the file")), "{warnings:?}");
}

/// A file can point any number of image hashes at one large blob; each used to get its own copy
/// of it (#1059).
#[test]
fn hashes_naming_one_blob_share_one_copy_of_it() {
    let image = |hash: u8, blob: u64| o(vec![("image", o(vec![("hash", Kv::Bytes(vec![hash; 20])), ("dataBlob", u(blob))]))]);
    let msg = o(vec![
        ("blobs", Kv::Array(vec![o(vec![("bytes", Kv::Bytes(png()))]), o(vec![("bytes", Kv::Bytes(vec![7; 16]))])])),
        ("nodeChanges", Kv::Array((1..=50).map(|h| image(h, 0)).chain([image(99, 1), image(98, 2)]).collect())),
    ]);
    let copies = embedded_copies(&msg);
    assert_eq!(copies.len(), 51, "the hash naming no blob is left out");
    let first = &copies[&"01".repeat(20)];
    assert_eq!(**first, *png());
    assert_eq!(copies.values().filter(|c| Arc::ptr_eq(c, first)).count(), 50);
    assert_eq!(*copies[&"63".repeat(20)], [7; 16]);
}

#[test]
fn other_files_are_not_fig() {
    assert!(try_import(b"{\"document\": {}}", "x").is_none());
    assert!(try_import(b"", "x").is_none());
    let native = crate::save_bytes(&figcraft_doc::Document::new("N"), None).unwrap();
    assert!(try_import(&native, "x").is_none());
}

/// A cheap deterministic generator for the mangle tests.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn mangled_files_never_panic() {
    let bytes = fixture();
    let c = canvas(&schema(), &message());
    let mut r = Lcg(7);
    for src in [&bytes, &c] {
        for cut in (0..src.len()).step_by(37) {
            let _ = import(&src[..cut], "x");
        }
        for _ in 0..300 {
            let mut m = src.clone();
            for _ in 0..1 + r.next() % 8 {
                let i = (r.next() as usize) % m.len();
                m[i] ^= 1 << (r.next() % 8);
            }
            let _ = import(&m, "x");
        }
    }
}

#[test]
fn mangled_kiwi_never_panics() {
    let sc = schema();
    let root = sc.find("Message").unwrap();
    let schema_bytes = encode::schema(&sc);
    let msg = encode::message(&sc, root, &message()).unwrap();
    let mut r = Lcg(11);
    for cut in 0..schema_bytes.len() {
        let _ = Schema::decode(&schema_bytes[..cut]);
    }
    for cut in (0..msg.len()).step_by(3) {
        if let Ok(kv) = sc.decode_message(root, &msg[..cut]) {
            let _ = map::to_rest(&kv, "x", crate::rest::MAX_NODES);
        }
    }
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
        let mut sbytes = schema_bytes.clone();
        let i = (r.next() as usize) % sbytes.len();
        sbytes[i] = r.next() as u8;
        if let Ok(s2) = Schema::decode(&sbytes)
            && let Some(rt) = s2.find("Message")
        {
            let _ = s2.decode_message(rt, &msg);
        }
    }
    // Pure junk, bare and framed.
    for _ in 0..500 {
        let junk: Vec<u8> = (0..(r.next() % 200)).map(|_| r.next() as u8).collect();
        let _ = Schema::decode(&junk);
        let _ = sc.decode_message(root, &junk);
        let mut framed = b"fig-kiwi\0\0\0\0".to_vec();
        framed.extend_from_slice(&junk);
        let _ = import(&framed, "x");
    }
}

#[test]
fn large_files_get_a_larger_value_budget() {
    // Small files keep the fixed floor: a few bytes can't expand into a huge tree.
    assert_eq!(kiwi::value_budget(0), 20_000_000);
    assert_eq!(kiwi::value_budget(1 << 20), 20_000_000);
    // A real 400 000-layer file: an 83.5 MB canvas whose message unpacks to 334 MB and holds
    // 30.5 million values was refused as "too many values".
    assert!(kiwi::value_budget(83_491_232) > 30_514_899);
    // A real 194 MB design-system file (UX4G): a 39.5 MB canvas holding 29.7 million values.
    assert!(kiwi::value_budget(39_528_420) > 29_703_730);
    // Still bounded by the largest canvas read.
    assert!(kiwi::value_budget(MAX_CANVAS as usize) <= 128 << 20);
}

#[test]
fn the_layer_cap_follows_the_file_size() {
    // A small (or broken) file keeps the REST import's cap: instances expand into a layer per
    // layer of their component, and a few bytes mustn't claim millions of layers.
    assert_eq!(layer_budget(0), crate::rest::MAX_NODES);
    assert_eq!(layer_budget(1 << 20), crate::rest::MAX_NODES);
    // A real 894 004-layer file (an 83.5 MB canvas) stopped at 200 000 and came in with 27 of
    // its 30 pages empty.
    assert!(layer_budget(83_491_232) > 894_004);
    // No file gets more than the ceiling.
    assert_eq!(layer_budget(MAX_CANVAS as usize), MAX_FIG_NODES);
}

#[test]
fn a_small_file_that_unpacks_large_gets_no_larger_budgets() {
    // 40 MB of zeros in a blob pack into a few KB. Keyed to the unpacked message, they lifted the
    // layer cap above the 231 231 layers its instances expand into; keyed to the file, the cap
    // stays at the REST import's.
    let mut nodes = vec![node(0, None, "DOCUMENT", "D", vec![]), node(1, Some((0, "!")), "CANVAS", "P", vec![])];
    nodes.push(node(5, Some((1, "!")), "SYMBOL", "Main", vec![("size", xy(10.0, 10.0))]));
    nodes.extend((10..1010).map(|i| node(i, Some((5, "!")), "RECTANGLE", "R", vec![("size", xy(1.0, 1.0))])));
    nodes.extend((2000..2230).map(|i| node(i, Some((1, "!")), "INSTANCE", "I", vec![("symbolData", o(vec![("symbolID", g(5))]))])));
    let blob = o(vec![("bytes", Kv::Bytes(vec![0; 40 << 20]))]);
    let file = canvas(&schema(), &o(vec![("nodeChanges", arr(nodes)), ("blobs", arr(vec![blob]))]));
    assert!(file.len() < 1 << 20, "{} bytes", file.len());
    let (doc, warnings) = import(&file, "x").unwrap();
    assert!(warnings.iter().any(|w| w.contains("more than 200000 layers")), "{warnings:?}");
    assert!(doc.node_count() <= crate::rest::MAX_NODES + 10);
}

#[test]
fn the_mapper_stops_at_the_layer_cap_it_is_given() {
    let mut nodes = vec![node(0, None, "DOCUMENT", "D", vec![]), node(1, Some((0, "!")), "CANVAS", "P", vec![])];
    nodes.extend((2..12).map(|i| node(i, Some((1, "!")), "RECTANGLE", "R", vec![("size", xy(1.0, 1.0))])));
    let msg = o(vec![("nodeChanges", arr(nodes))]);
    let (_, warnings, _) = map::to_rest(&msg, "x", 5);
    assert!(warnings.iter().any(|w| w.contains("more than 5 layers")), "{warnings:?}");
    let (_, warnings, _) = map::to_rest(&msg, "x", crate::rest::MAX_NODES);
    assert!(!warnings.iter().any(|w| w.contains("more than")), "{warnings:?}");
}

#[test]
fn scalar_fields_count_against_the_value_budget() {
    // A message of 1000 scalar fields (id 1, `true`) is 1001 values: a budget of 10 refuses it.
    let fields = vec![Field { name: "b".into(), ty: Some(Type::Bool), array: false, value: 1 }];
    let sc = Schema { defs: vec![Def { name: "Message".into(), kind: Kind::Message, fields }] };
    let mut data = [1u8, 1].repeat(1000);
    data.push(0);
    let e = sc.decode_message_within(0, &data, 10).unwrap_err();
    assert_eq!(e.0, "message: too many values");
    assert!(sc.decode_message_within(0, &data, 1001).is_ok());
}

#[test]
fn hostile_input_is_refused() {
    // A message that contains itself, nested past the depth cap.
    let field = |ty, array| vec![Field { name: "m".into(), ty: Some(ty), array, value: 1 }];
    let nested = Schema { defs: vec![Def { name: "Message".into(), kind: Kind::Message, fields: field(Type::Def(0), false) }] };
    assert!(nested.decode_message(0, &[1u8; 10_000]).is_err());
    // An array count larger than the data.
    let array = Schema { defs: vec![Def { name: "Message".into(), kind: Kind::Message, fields: field(Type::Uint, true) }] };
    assert!(array.decode_message(0, &[1, 0xff, 0xff, 0xff, 0xff, 0x0f]).is_err());
    // A field type naming a missing definition.
    let mut w = encode::Writer::default();
    w.var_uint(1);
    w.string("Message");
    w.byte(2);
    w.var_uint(1);
    w.string("x");
    w.var_int(5);
    w.byte(0);
    w.var_uint(1);
    assert!(Schema::decode(&w.0).is_err());
    // A schema without a Message definition.
    let c = canvas(&Schema { defs: vec![Def { name: "Message".into(), kind: Kind::Message, fields: vec![] }] }, &Kv::Object(vec![]));
    let renamed: Vec<u8> = {
        let sc = Schema { defs: vec![Def { name: "Other".into(), kind: Kind::Message, fields: vec![] }] };
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&encode::schema(&sc)).unwrap();
        let sch = z.finish().unwrap();
        let mut out = c[..12].to_vec();
        out.extend_from_slice(&(sch.len() as u32).to_le_bytes());
        out.extend_from_slice(&sch);
        out.extend_from_slice(&c[12 + 4 + chunks(&c).unwrap()[0].len()..]);
        out
    };
    assert!(import(&c, "x").is_ok());
    assert!(import(&renamed, "x").is_err());
}

#[test]
fn instances_inside_their_own_main_stop() {
    let sym = node(5, Some((1, "!")), "SYMBOL", "Loop", vec![("size", xy(10.0, 10.0))]);
    let inner = node(6, Some((5, "!")), "INSTANCE", "Self", vec![("symbolData", o(vec![("symbolID", g(5))]))]);
    let doc_nodes = vec![node(0, None, "DOCUMENT", "D", vec![]), node(1, Some((0, "!")), "CANVAS", "P", vec![]), sym, inner];
    let (doc, _) = import(&canvas(&schema(), &o(vec![("nodeChanges", arr(doc_nodes))])), "x").unwrap();
    assert!(doc.nodes().any(|n| n.name == "Self"));
}

/// A collection from a library is named by its asset key, not a guid — also in the explicit mode
/// map of the frames, components and instances set it on, and in the variables' collection link.
#[test]
fn modes_of_collections_named_by_asset_import() {
    let modes = arr(vec![
        o(vec![("id", g(90)), ("name", s("Dark")), ("sortPosition", s("!"))]),
        o(vec![("id", g(91)), ("name", s("Light")), ("sortPosition", s("\""))]),
    ]);
    let color_value = |c| o(vec![("value", o(vec![("colorValue", c)])), ("dataType", e("COLOR"))]);
    let values = o(vec![(
        "entries",
        arr(vec![
            o(vec![("modeID", g(90)), ("variableData", color_value(rgb(0.0, 0.0, 0.0)))]),
            o(vec![("modeID", g(91)), ("variableData", color_value(rgb(1.0, 1.0, 1.0)))]),
        ]),
    )]);
    let by_asset = |key: &'static str, version: Option<&'static str>| {
        let mut r = vec![("key", s(key))];
        r.extend(version.map(|v| ("version", s(v))));
        o(vec![("assetRef", o(r))])
    };
    let mode_map = |set: Kv<'static>| o(vec![("entries", arr(vec![o(vec![("variableSetID", set), ("variableModeID", g(91))])]))]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page A", vec![]),
        node(2, Some((0, "~")), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))]),
        // The collection itself says the asset key its references name.
        node(4, Some((2, "!")), "VARIABLE_SET", "Schema", vec![("variableSetModes", modes), ("key", s("schema-collection")), ("version", s("7"))]),
        node(
            5,
            Some((2, "\"")),
            "VARIABLE",
            "surface",
            vec![("variableSetID", by_asset("schema-collection", Some("7"))), ("variableResolvedType", e("COLOR")), ("variableDataValues", values)],
        ),
        node(
            10,
            Some((1, "!")),
            "FRAME",
            "Light section",
            vec![("size", xy(400.0, 300.0)), ("variableModeBySetMap", mode_map(by_asset("schema-collection", Some("7"))))],
        ),
        node(
            20,
            Some((1, "\"")),
            "SYMBOL",
            "Button",
            vec![("size", xy(100.0, 40.0)), ("variableModeBySetMap", mode_map(by_asset("schema-collection", None)))],
        ),
        node(
            21,
            Some((1, "#")),
            "INSTANCE",
            "Button copy",
            vec![
                ("size", xy(100.0, 40.0)),
                ("symbolData", o(vec![("symbolID", g(20))])),
                ("variableModeBySetMap", mode_map(by_asset("schema-collection", Some("7")))),
            ],
        ),
        // One names a collection that isn't in the file: its mode is skipped, with a warning.
        node(11, Some((1, "$")), "FRAME", "Gone", vec![("size", xy(400.0, 300.0)), ("variableModeBySetMap", mode_map(by_asset("gone", None)))]),
    ];
    let (doc, warnings) = import(&canvas(&schema(), &o(vec![("nodeChanges", arr(nodes))])), "x").unwrap();
    assert_eq!(doc.variables.collections.len(), 1);
    assert_eq!(doc.variables.variables.len(), 1);
    assert_eq!(doc.variables.variables[0].collection, "1:4");
    for name in ["Light section", "Button", "Button copy"] {
        let n = named(&doc, name);
        assert_eq!(n.var_modes.get("1:4").map(String::as_str), Some("1:91"), "{name}");
    }
    assert!(named(&doc, "Gone").var_modes.is_empty());
    assert!(warnings.iter().any(|w| w.contains("variable modes of collections that aren't in the file")), "{warnings:?}");
}

#[test]
fn resized_instances_lay_their_layers_out_by_their_constraints() {
    let pin = |h: &'static str, v: &'static str| vec![("horizontalConstraint", e(h)), ("verticalConstraint", e(v))];
    let layer = |id, pos, ty, name, w, h, x, y, c: Vec<(&'static str, Kv<'static>)>| {
        node(id, Some((5, pos)), ty, name, [vec![("size", xy(w, h)), ("transform", at(x, y))], c].concat())
    };
    let nodes = vec![
        node(0, None, "DOCUMENT", "D", vec![]),
        node(1, Some((0, "!")), "CANVAS", "P", vec![]),
        node(5, Some((1, "!")), "SYMBOL", "Logo", vec![("size", xy(132.0, 22.0)), ("transform", at(0.0, 0.0))]),
        layer(6, "!", "ROUNDED_RECTANGLE", "Mark", 132.0, 22.0, 0.0, 0.0, pin("SCALE", "SCALE")),
        layer(7, "\"", "ROUNDED_RECTANGLE", "Badge", 32.0, 10.0, 100.0, 12.0, pin("MAX", "MAX")),
        layer(8, "#", "ROUNDED_RECTANGLE", "Bar", 112.0, 2.0, 10.0, 0.0, pin("STRETCH", "MIN")),
        // A group scales its layers with it.
        layer(11, "$", "FRAME", "Wordmark", 66.0, 22.0, 66.0, 0.0, [pin("SCALE", "SCALE"), vec![("resizeToFit", Kv::Bool(true))]].concat()),
        node(12, Some((11, "!")), "ROUNDED_RECTANGLE", "Letter", vec![("size", xy(66.0, 22.0)), ("transform", at(0.0, 0.0))]),
        node(
            9,
            Some((1, "\"")),
            "INSTANCE",
            "Small logo",
            vec![("size", xy(90.0, 15.0)), ("transform", at(0.0, 100.0)), ("symbolData", o(vec![("symbolID", g(5))]))],
        ),
        // Derived layout the file gives wins over the constraints.
        node(
            10,
            Some((1, "#")),
            "INSTANCE",
            "Derived logo",
            vec![
                ("size", xy(90.0, 15.0)),
                ("transform", at(0.0, 200.0)),
                ("symbolData", o(vec![("symbolID", g(5))])),
                ("derivedSymbolData", arr(vec![o(vec![("guidPath", o(vec![("guids", arr(vec![g(6)]))])), ("size", xy(50.0, 5.0))])])),
            ],
        ),
        // A size the instance overrides: the place still follows the constraints.
        node(
            13,
            Some((1, "$")),
            "INSTANCE",
            "Resized badge",
            vec![
                ("size", xy(90.0, 15.0)),
                ("transform", at(0.0, 300.0)),
                (
                    "symbolData",
                    o(vec![
                        ("symbolID", g(5)),
                        ("symbolOverrides", arr(vec![o(vec![("guidPath", o(vec![("guids", arr(vec![g(7)]))])), ("size", xy(20.0, 10.0))])])),
                    ]),
                ),
            ],
        ),
    ];
    let (doc, _) = import(&canvas(&schema(), &o(vec![("nodeChanges", arr(nodes))])), "x").unwrap();
    let box_of = |inst: &str, name: &str| {
        let i = named(&doc, inst);
        let n = i.children.iter().map(|c| doc.node(*c).unwrap()).find(|n| n.name == name).unwrap();
        let t = n.transform.translation();
        [t.x, t.y, n.width, n.height].map(|x| (x * 1e3).round() / 1e3)
    };
    assert_eq!(box_of("Small logo", "Mark"), [0.0, 0.0, 90.0, 15.0]);
    assert_eq!(box_of("Small logo", "Badge"), [58.0, 5.0, 32.0, 10.0]);
    assert_eq!(box_of("Small logo", "Bar"), [10.0, 0.0, 70.0, 2.0]);
    // The main's own, then the instances'.
    let mut letters: Vec<[f64; 2]> =
        doc.nodes().filter(|n| n.name == "Letter").map(|n| [n.width, n.height].map(|x| (x * 1e3).round() / 1e3)).collect();
    letters.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(letters, [[45.0, 15.0], [45.0, 15.0], [45.0, 15.0], [66.0, 22.0]]);
    assert_eq!(box_of("Derived logo", "Mark")[2..], [50.0, 5.0]);
    assert_eq!(box_of("Resized badge", "Badge"), [58.0, 5.0, 20.0, 10.0]);
}

#[test]
fn nested_instances_resized_again_drop_the_layout_derived_for_their_old_size() {
    let stretch = || vec![("horizontalConstraint", e("STRETCH")), ("verticalConstraint", e("MIN"))];
    let nodes = vec![
        node(0, None, "DOCUMENT", "D", vec![]),
        node(1, Some((0, "!")), "CANVAS", "P", vec![]),
        node(20, Some((1, "!")), "SYMBOL", "Track", vec![("size", xy(100.0, 10.0)), ("transform", at(0.0, 0.0))]),
        node(21, Some((20, "!")), "ROUNDED_RECTANGLE", "Fill", [vec![("size", xy(100.0, 10.0)), ("transform", at(0.0, 0.0))], stretch()].concat()),
        node(30, Some((1, "\"")), "SYMBOL", "Slider", vec![("size", xy(200.0, 50.0)), ("transform", at(0.0, 100.0))]),
        // The nested instance is 150 wide in its main, with the layout derived for that.
        node(
            31,
            Some((30, "!")),
            "INSTANCE",
            "Inner",
            [
                vec![
                    ("size", xy(150.0, 10.0)),
                    ("transform", at(0.0, 0.0)),
                    ("symbolData", o(vec![("symbolID", g(20))])),
                    (
                        "derivedSymbolData",
                        arr(vec![o(vec![
                            ("guidPath", o(vec![("guids", arr(vec![g(21)]))])),
                            ("size", xy(150.0, 10.0)),
                            ("fillPaints", arr(vec![solid(1.0, 0.0, 0.0)])),
                        ])]),
                    ),
                ],
                stretch(),
            ]
            .concat(),
        ),
        node(
            40,
            Some((1, "#")),
            "INSTANCE",
            "Wide slider",
            vec![("size", xy(300.0, 50.0)), ("transform", at(0.0, 200.0)), ("symbolData", o(vec![("symbolID", g(30))]))],
        ),
    ];
    let (doc, _) = import(&canvas(&schema(), &o(vec![("nodeChanges", arr(nodes))])), "x").unwrap();
    let width = |outer: &str| {
        let inner = doc.node(named(&doc, outer).children[0]).unwrap();
        (inner.width, doc.node(inner.children[0]).unwrap().width)
    };
    // In its main the derived layout holds; stretched to 250 it no longer does.
    assert_eq!(width("Slider"), (150.0, 150.0));
    assert_eq!(width("Wide slider"), (250.0, 250.0));
    // What else the derived data gives (the colour a variable resolves to, say) still holds.
    let fill = doc.node(doc.node(named(&doc, "Wide slider").children[0]).unwrap().children[0]).unwrap();
    assert!(matches!(fill.fills.first().map(|p| &p.kind), Some(figcraft_doc::color::PaintKind::Solid { color }) if color.r == 1.0 && color.g == 0.0));
}

#[path = "tail_tests.rs"]
mod tail;

#[path = "image_filter_tests.rs"]
mod image_filters;

#[path = "text_list_tests.rs"]
mod text_list;

/// `.fig` files in the folder `FIGCRAFT_FIG_SAMPLES` names (none when it's unset).
pub(super) fn samples() -> Vec<std::path::PathBuf> {
    let Some(dir) = std::env::var_os("FIGCRAFT_FIG_SAMPLES") else { return Vec::new() };
    let dir = std::fs::read_dir(dir).into_iter().flatten().flatten();
    dir.map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "fig")).collect()
}

/// Local only (`FIGCRAFT_FIG_SAMPLES`): every file imports without warnings.
#[test]
fn local_samples_import() {
    for p in samples() {
        let bytes = std::fs::read(&p).unwrap();
        let (doc, warnings) = import(&bytes, "sample").unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        assert!(warnings.is_empty(), "{}: {warnings:?}", p.display());
        assert!(doc.node_count() > 5, "{}", p.display());
    }
}

/// Local only: dumps each sample's schema, decoded message and REST JSON when `FIG_DUMP` names a
/// folder (for working on the importer).
#[test]
fn dump_samples() {
    let Ok(out) = std::env::var("FIG_DUMP") else { return };
    for p in samples() {
        let bytes = std::fs::read(&p).unwrap();
        let a = Archive::open(&bytes).unwrap();
        let (schema, message) = decode_canvas(&a.canvas).unwrap();
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        let mut txt = String::new();
        for d in &schema.defs {
            txt += &format!("{:?} {}\n", d.kind, d.name);
            for f in &d.fields {
                let ty = match f.ty {
                    Some(Type::Def(i)) => schema.defs[i].name.clone(),
                    Some(t) => format!("{t:?}"),
                    None => String::new(),
                };
                txt += &format!("  {} {}{} = {}\n", f.name, ty, if f.array { "[]" } else { "" }, f.value);
            }
        }
        std::fs::write(format!("{out}/{stem}.schema.txt"), txt).unwrap();
        let msg = schema.decode_message(schema.find("Message").unwrap(), &message).unwrap();
        std::fs::write(format!("{out}/{stem}.json"), serde_json::to_string_pretty(&debug_json(&msg)).unwrap()).unwrap();
        let (rest, ..) = map::to_rest(&msg, &stem, super::layer_budget(message.len()));
        std::fs::write(format!("{out}/{stem}.rest.json"), serde_json::to_string_pretty(&rest).unwrap()).unwrap();
    }
}

#[path = "board_import_tests.rs"]
mod board;
