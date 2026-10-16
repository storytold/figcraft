//! Export to `.fig` (experimental, best effort), from the same public format knowledge as the
//! import: the inverse of its mapping ([`super::map`]).
//!
//! The file is the zip the import reads: `canvas.fig` (the fig-kiwi header, then the binary schema
//! and the message, each a raw-deflate chunk), `meta.json`, an optional `thumbnail.png` and
//! `images/<hash>`. The schema travels in the file, so it describes only the fields written here
//! ([`SCHEMA`]). The message lists one node change per layer (its id, its parent and its position
//! among the siblings, then its fields), parents first, plus blobs (vector networks and outlines).
//! Instances are written as their main, the fields they and their layers override (by id path) and
//! what follows from those (derived data: sizes, places, outlines), like the files the import
//! reads. Shared styles, variable collections and variables are nodes of their own on a hidden
//! page; layers, paints, effects and layout grids name the ones they use. What the format can't
//! carry yet (Dev Mode data, comments, …) is left out and listed in the warnings. Round trips
//! through our own importer are tested, and written files were opened in other software
//! (`docs/file-format-parity.md` says what was checked); other apps may not open every file.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;

use figcraft_doc::color::{BlendMode, Color, GradientKind, ImageScaleMode, Paint, PaintKind};
use figcraft_doc::geom::kurbo::PathEl;
use figcraft_doc::geom::{Affine, BezPath};
use figcraft_doc::proto::{
    Action, Direction, Easing, GAMEPAD_BUTTONS, Interaction, KeyMods, Overflow, OverlayPosition, ScrollPosition, Transition, Trigger,
};
use figcraft_doc::{
    AutoLayout, BoolOp, ContentAlign, CrossAlign, Document, EffectKind, ExportSize, GridAlign, GridKind, HConstraint, LayoutMode, LetterSpacing,
    LineHeight, ListStyle, MainAlign, MaskType, Node, NodeId, NodeKind, NoiseMode, PropDef, Sizing, StrokeAlign, StrokeCap, StrokeJoin, StyleKind,
    TextAlignH, TextAlignV, TextAutoResize, TextCase, TextData, TextDecoration, TextStyle, VConstraint, VariableType, VariableValue, VectorNetwork,
    WindingRule,
};
use serde_json::Value;

use super::kiwi::{Kv, Schema, encode};
use super::network;
use crate::FormatError;

/// Most layers written, and deepest nesting followed (the import's limits).
const MAX_NODES: usize = crate::rest::MAX_NODES;
const MAX_DEPTH: usize = 256;
/// Largest `canvas.fig` and image written (what the import reads).
const MAX_CANVAS: u64 = super::MAX_CANVAS;
const MAX_IMAGE: u64 = super::MAX_IMAGE;
/// Largest whole file written.
const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
/// Most characters of one text layer, and vertices + segments + loop entries of one vector
/// network (what the import reads).
const MAX_CHARS: usize = 1 << 20;
const MAX_NETWORK: usize = 200_000;
/// Largest magnitude written for a float (the format stores 32-bit floats).
const MAX_FLOAT: f64 = 1e30;
/// The version number in the `canvas.fig` header.
const VERSION: u32 = 101;
/// Session ids of the GUIDs: layers, and grid tracks and component properties.
const LAYERS: u32 = 1;
const AUX: u32 = 2;

/// The schema written into every file: `enum Name A=value …`, `struct Name type field, …` (every
/// field, in order) and `message Name type field=id, …`, with the names, kinds and numbers the
/// format gives them. Only what the export writes.
pub(super) const SCHEMA: &str = "
enum MessageType JOIN_START=0 NODE_CHANGES=1
enum NodePhase CREATED=0 REMOVED=1
enum NodeType NONE=0 DOCUMENT=1 CANVAS=2 GROUP=3 FRAME=4 BOOLEAN_OPERATION=5 VECTOR=6 STAR=7 LINE=8 ELLIPSE=9 RECTANGLE=10 REGULAR_POLYGON=11 ROUNDED_RECTANGLE=12 TEXT=13 SLICE=14 SYMBOL=15 INSTANCE=16 SECTION=25 VARIABLE=28 VARIABLE_SET=31
enum BlendMode PASS_THROUGH=0 NORMAL=1 DARKEN=2 MULTIPLY=3 LINEAR_BURN=4 COLOR_BURN=5 LIGHTEN=6 SCREEN=7 LINEAR_DODGE=8 COLOR_DODGE=9 OVERLAY=10 SOFT_LIGHT=11 HARD_LIGHT=12 DIFFERENCE=13 EXCLUSION=14 HUE=15 SATURATION=16 COLOR=17 LUMINOSITY=18
enum PaintType SOLID=0 GRADIENT_LINEAR=1 GRADIENT_RADIAL=2 GRADIENT_ANGULAR=3 GRADIENT_DIAMOND=4 IMAGE=5
enum ImageScaleMode STRETCH=0 FIT=1 FILL=2 TILE=3
enum WindingRule NONZERO=0 ODD=1
enum StrokeAlign CENTER=0 INSIDE=1 OUTSIDE=2
enum StrokeCap NONE=0 ROUND=1 SQUARE=2 ARROW_LINES=3 ARROW_EQUILATERAL=4 TRIANGLE_FILLED=6 DIAMOND_FILLED=5 CIRCLE_FILLED=14
enum StrokeJoin MITER=0 BEVEL=1 ROUND=2
enum ConstraintType MIN=0 CENTER=1 MAX=2 STRETCH=3 SCALE=4
enum StackMode NONE=0 HORIZONTAL=1 VERTICAL=2 GRID=3
enum StackAlign MIN=0 CENTER=1 MAX=2 BASELINE=3
enum StackJustify MIN=0 CENTER=1 MAX=2 SPACE_EVENLY=3 SPACE_BETWEEN=4 SPACE_AROUND=5
enum StackCounterAlign MIN=0 CENTER=1 MAX=2 STRETCH=3 AUTO=4 BASELINE=5
enum StackSize FIXED=0 RESIZE_TO_FIT=1
enum StackWrap NO_WRAP=0 WRAP=1
enum StackCounterAlignContent AUTO=0 SPACE_BETWEEN=1
enum StackPositioning AUTO=0 ABSOLUTE=1
enum EffectType INNER_SHADOW=0 DROP_SHADOW=1 FOREGROUND_BLUR=2 BACKGROUND_BLUR=3 NOISE=7 GRAIN=6 GLASS=8
enum BlurOpType NORMAL=0 PROGRESSIVE=1
enum NoiseType MONOTONE=1 DUOTONE=2 MULTITONE=0
enum TextAlignHorizontal LEFT=0 CENTER=1 RIGHT=2 JUSTIFIED=3
enum TextAlignVertical TOP=0 CENTER=1 BOTTOM=2
enum TextAutoResize NONE=0 WIDTH_AND_HEIGHT=1 HEIGHT=2
enum TextTruncation DISABLED=0 ENDING=1
enum TextCase ORIGINAL=0 UPPER=1 LOWER=2 TITLE=3 SMALL_CAPS=4
enum TextDecoration NONE=0 UNDERLINE=1 STRIKETHROUGH=2
enum NumberUnits RAW=0 PIXELS=1 PERCENT=2
enum LineType PLAIN=0 ORDERED_LIST=1 UNORDERED_LIST=2
enum SourceDirectionality AUTO=0 LTR=1 RTL=2
enum DocumentColorProfile LEGACY=0 SRGB=1 DISPLAY_P3=2
enum EmojiImageSet APPLE=0 NOTO=1
enum MaskType ALPHA=0 OUTLINE=1 LUMINANCE=2
enum VariableResolvedDataType BOOLEAN=0 FLOAT=1 STRING=2 COLOR=4 SYMBOL_ID=6 TEXT_DATA=8
enum VectorMirror NONE=0 ANGLE=1 ANGLE_AND_LENGTH=2
enum BooleanOperation UNION=0 INTERSECT=1 SUBTRACT=2 XOR=3
enum LayoutGridType MIN=0 CENTER=1 STRETCH=2 MAX=3
enum LayoutGridPattern STRIPES=0 GRID=1
enum Axis X=0 Y=1
enum ImageType PNG=0 JPEG=1 SVG=2 PDF=3
enum ExportConstraintType CONTENT_SCALE=0 CONTENT_WIDTH=1 CONTENT_HEIGHT=2
enum ComponentPropType BOOL=0 TEXT=1 INSTANCE_SWAP=3 VARIANT=4
enum VariableField MISSING=0 CORNER_RADIUS=1 PARAGRAPH_SPACING=2 PARAGRAPH_INDENT=3 STROKE_WEIGHT=4 STACK_SPACING=5 STACK_PADDING_LEFT=6 STACK_PADDING_TOP=7 STACK_PADDING_RIGHT=8 STACK_PADDING_BOTTOM=9 VISIBLE=10 TEXT_DATA=11 WIDTH=12 HEIGHT=13 RECTANGLE_TOP_LEFT_CORNER_RADIUS=14 RECTANGLE_TOP_RIGHT_CORNER_RADIUS=15 RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS=16 RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS=17 BORDER_TOP_WEIGHT=18 BORDER_BOTTOM_WEIGHT=19 BORDER_LEFT_WEIGHT=20 BORDER_RIGHT_WEIGHT=21 STACK_COUNTER_SPACING=23 MIN_WIDTH=24 MAX_WIDTH=25 MIN_HEIGHT=26 MAX_HEIGHT=27 FONT_FAMILY=28 FONT_STYLE=29 OPACITY=31 FONT_SIZE=32 LETTER_SPACING=34 LINE_HEIGHT=36 OVERRIDDEN_SYMBOL_ID=37
enum VariableDataType BOOLEAN=0 FLOAT=1 STRING=2 ALIAS=3 COLOR=4 PROP_REF=13
enum VariableScope ALL_SCOPES=0 TEXT_CONTENT=1 CORNER_RADIUS=2 WIDTH_HEIGHT=3 GAP=4 ALL_FILLS=5 FRAME_FILL=6 SHAPE_FILL=7 TEXT_FILL=8 STROKE=9 STROKE_FLOAT=10 EFFECT_FLOAT=11 EFFECT_COLOR=12 OPACITY=13 FONT_STYLE=14 FONT_FAMILY=15 FONT_SIZE=16 LINE_HEIGHT=17 LETTER_SPACING=18 PARAGRAPH_SPACING=19 PARAGRAPH_INDENT=20
enum StyleType NONE=0 FILL=1 STROKE=2 TEXT=3 EFFECT=4 EXPORT=5 GRID=6
enum InteractionType ON_CLICK=0 DRAG=9 ON_HOVER=4 ON_PRESS=7 MOUSE_IN=2 MOUSE_OUT=3 MOUSE_DOWN=5 MOUSE_UP=6 AFTER_TIMEOUT=1 ON_KEY_DOWN=10
enum TriggerDevice KEYBOARD=0 UNKNOWN_CONTROLLER=1
enum ConnectionType NONE=0 INTERNAL_NODE=1 URL=2 BACK=3 CLOSE=4
enum NavigationType NAVIGATE=0 OVERLAY=1 SWAP=2 SWAP_STATE=3 SCROLL_TO=4
enum TransitionType INSTANT_TRANSITION=0 DISSOLVE=1 SMART_ANIMATE=24 SLIDE_FROM_LEFT=3 SLIDE_FROM_RIGHT=4 SLIDE_FROM_TOP=5 SLIDE_FROM_BOTTOM=6 PUSH_FROM_LEFT=7 PUSH_FROM_RIGHT=8 PUSH_FROM_TOP=9 PUSH_FROM_BOTTOM=10 MOVE_FROM_LEFT=11 MOVE_FROM_RIGHT=12 MOVE_FROM_TOP=13 MOVE_FROM_BOTTOM=14 SLIDE_OUT_TO_LEFT=15 SLIDE_OUT_TO_RIGHT=16 SLIDE_OUT_TO_TOP=17 SLIDE_OUT_TO_BOTTOM=18 MOVE_OUT_TO_LEFT=19 MOVE_OUT_TO_RIGHT=20 MOVE_OUT_TO_TOP=21 MOVE_OUT_TO_BOTTOM=22
enum EasingType IN_CUBIC=0 OUT_CUBIC=1 INOUT_CUBIC=2 LINEAR=3 IN_BACK_CUBIC=4 OUT_BACK_CUBIC=5 INOUT_BACK_CUBIC=6 CUSTOM_CUBIC=7 GENTLE_SPRING=9
enum ScrollDirection NONE=0 HORIZONTAL=1 VERTICAL=2 BOTH=3
enum ScrollBehavior SCROLLS=0 FIXED_WHEN_CHILD_OF_SCROLLING_FRAME=1 STICKY_SCROLLS=2
enum OverlayPositionType CENTER=0 TOP_LEFT=1 TOP_CENTER=2 TOP_RIGHT=3 BOTTOM_LEFT=4 BOTTOM_CENTER=5 BOTTOM_RIGHT=6 MANUAL=7
enum OverlayBackgroundInteraction NONE=0 CLOSE_ON_CLICK_OUTSIDE=1
enum OverlayBackgroundType NONE=0 SOLID_COLOR=1
struct GUID uint sessionID, uint localID
struct Color float r, float g, float b, float a
struct Vector float x, float y
struct Matrix float m00, float m01, float m02, float m10, float m11, float m12
struct ColorStop Color color, float position
struct ParentIndex GUID guid, string position
message Image byte[] hash=1, string name=2
message VariableWidthPoint float position=1, float ascent=2, float descent=3
message PluginData string pluginID=1, string value=2, string key=3
message StyleId GUID guid=1
message VariableID GUID guid=1
message VariableSetID GUID guid=1
message ColorStopVar Color color=1, VariableData colorVar=2, float position=3
message Paint PaintType type=1, Color color=2, float opacity=3, bool visible=4, BlendMode blendMode=5, ColorStop[] stops=6, Matrix transform=7, Image image=8, ImageScaleMode imageScaleMode=10, float rotation=11, float scale=12, VariableData colorVar=21, ColorStopVar[] stopsVar=23
message Effect EffectType type=1, Color color=2, Vector offset=3, float radius=4, bool visible=5, BlendMode blendMode=6, float spread=7, bool showShadowBehindNode=8, BlurOpType blurOpType=19, float startRadius=21, Vector startOffset=20, Vector endOffset=28, NoiseType noiseType=26, Vector noiseSize=22, float density=25, float opacity=27, Color secondaryColor=24, bool clipToShape=23, VariableData radiusVar=9, VariableData colorVar=10, VariableData spreadVar=11, VariableData xVar=12, VariableData yVar=13, VariableData startRadiusVar=45
struct FontName string family, string style, string postscript
struct Number float value, NumberUnits units
message Hyperlink string url=1
message TextLineData LineType lineType=1, int styleId=10, int indentationLevel=2, SourceDirectionality sourceDirectionality=9, int listStartOffset=7, bool isFirstLineOfList=8
message TextData string characters=1, uint[] characterStyleIDs=2, NodeChange[] styleOverrideTable=3, TextLineData[] lines=12
message Path WindingRule windingRule=1, uint commandsBlob=2, uint styleID=3
message VectorData uint vectorNetworkBlob=1, Vector normalizedSize=2, NodeChange[] styleOverrideTable=3
message ArcData float startingAngle=1, float endingAngle=2, float innerRadius=3
message OptionalVector Vector value=1
message LayoutGrid LayoutGridType type=1, Axis axis=2, bool visible=3, int numSections=4, float offset=5, float sectionSize=6, float gutterSize=7, Color color=8, LayoutGridPattern pattern=9, VariableData numSectionsVar=10, VariableData offsetVar=11, VariableData sectionSizeVar=12, VariableData gutterSizeVar=13
struct ExportConstraint ExportConstraintType type, float value
message ExportSettings string suffix=1, ImageType imageType=2, ExportConstraint constraint=3
message GUIDPath GUID[] guids=1
message SymbolData GUID symbolID=1, NodeChange[] symbolOverrides=2, float uniformScaleFactor=3
message ComponentPropValue bool boolValue=1, TextData textValue=2, GUID guidValue=3
message ComponentPropDef GUID id=1, string name=2, ComponentPropValue initialValue=3, ComponentPropType type=6
message ComponentPropAssignment GUID defID=1, ComponentPropValue value=2
message PropRefValue GUID defId=1
message VariableAnyValue bool boolValue=1, string textValue=2, float floatValue=3, VariableID alias=4, Color colorValue=5, PropRefValue propRefValue=13
message VariableData VariableAnyValue value=1, VariableDataType dataType=2, VariableResolvedDataType resolvedDataType=3
message VariableDataMapEntry uint nodeField=1, VariableData variableData=2, VariableField variableField=3
message VariableDataMap VariableDataMapEntry[] entries=1
message VariableModeBySetMapEntry VariableSetID variableSetID=1, GUID variableModeID=2
message VariableModeBySetMap VariableModeBySetMapEntry[] entries=1
message VariableSetMode GUID id=1, string name=2, string sortPosition=3
message VariableDataValuesEntry GUID modeID=1, VariableData variableData=2
message VariableDataValues VariableDataValuesEntry[] entries=1
message KeyTrigger int[] keyCodes=1, TriggerDevice triggerDevice=2
message PrototypeEvent InteractionType interactionType=1, float transitionTimeout=6, KeyTrigger keyTrigger=4
message PrototypeAction GUID transitionNodeID=1, TransitionType transitionType=2, float transitionDuration=3, EasingType easingType=4, float[] easingFunction=12, ConnectionType connectionType=7, Vector overlayRelativePosition=9, NavigationType navigationType=10, string connectionURL=8, bool transitionPreserveScroll=11
message PrototypeInteraction PrototypeEvent event=2, PrototypeAction[] actions=3
message PrototypeStartingPoint string name=1, string position=3
message OverlayBackgroundAppearance OverlayBackgroundType backgroundType=1, Color backgroundColor=2
message GUIDPositionMapEntry GUID id=1, string position=2
message GUIDPositionMap GUIDPositionMapEntry[] entries=1
message NodeChange GUID guid=1, NodePhase phase=2, ParentIndex parentIndex=3, NodeType type=4, string name=5, bool visible=6, bool locked=7, float opacity=8, BlendMode blendMode=9, Vector size=11, Matrix transform=12, bool mask=16, MaskType maskType=317, float backgroundOpacity=19, bool backgroundEnabled=15, bool internalOnly=142, string description=318, StyleType styleType=163, string sortPosition=320, StyleId styleIdForFill=332, StyleId styleIdForStrokeFill=333, StyleId styleIdForText=334, StyleId styleIdForEffect=335, StyleId styleIdForGrid=336, VariableDataMap variableConsumptionMap=307, VariableModeBySetMap variableModeBySetMap=316, VariableSetMode[] variableSetModes=312, VariableSetID variableSetID=313, VariableResolvedDataType variableResolvedType=314, VariableDataValues variableDataValues=315, VariableScope[] variableScopes=353, DocumentColorProfile documentColorProfile=349, Paint[] fillPaints=38, Paint[] strokePaints=39, float strokeWeight=26, StrokeAlign strokeAlign=29, StrokeJoin strokeJoin=31, StrokeCap strokeCap=30, float[] dashPattern=13, float miterLimit=25, bool borderStrokeWeightsIndependent=299, float borderTopWeight=295, float borderRightWeight=298, float borderBottomWeight=296, float borderLeftWeight=297, Effect[] effects=43, float cornerRadius=20, float cornerSmoothing=160, bool rectangleCornerRadiiIndependent=149, float rectangleTopLeftCornerRadius=145, float rectangleTopRightCornerRadius=146, float rectangleBottomRightCornerRadius=148, float rectangleBottomLeftCornerRadius=147, ConstraintType horizontalConstraint=28, ConstraintType verticalConstraint=37, bool frameMaskDisabled=115, bool resizeToFit=117, bool isStateGroup=225, bool sectionContentsHidden=283, Color backgroundColor=50, StackMode stackMode=105, float stackSpacing=107, float stackCounterSpacing=324, StackWrap stackWrap=323, StackCounterAlignContent stackCounterAlignContent=343, float stackHorizontalPadding=209, float stackVerticalPadding=210, float stackPaddingRight=233, float stackPaddingBottom=234, StackJustify stackPrimaryAlignItems=230, StackAlign stackCounterAlignItems=231, StackSize stackPrimarySizing=229, StackSize stackCounterSizing=221, bool bordersTakeSpace=294, bool stackReverseZIndex=271, float gridColumnGap=438, float gridRowGap=437, GUIDPositionMap gridColumns=436, GUIDPositionMap gridRows=435, float stackChildPrimaryGrow=232, StackCounterAlign stackChildAlignSelf=236, StackPositioning stackPositioning=269, OptionalVector minSize=325, OptionalVector maxSize=326, GUID gridColumnAnchor=440, GUID gridRowAnchor=439, uint gridColumnSpan=442, uint gridRowSpan=441, LayoutGrid[] layoutGrids=47, ExportSettings[] exportSettings=45, TextData textData=42, FontName fontName=41, float fontSize=21, Number lineHeight=40, Number letterSpacing=165, TextCase textCase=34, TextDecoration textDecoration=35, TextAlignHorizontal textAlignHorizontal=32, TextAlignVertical textAlignVertical=33, TextAutoResize textAutoResize=46, TextTruncation textTruncation=280, int maxLines=351, float paragraphSpacing=23, float paragraphIndent=22, Hyperlink hyperlink=223, bool autoRename=14, bool fontVariantCommonLigatures=127, bool fontVariantContextualLigatures=128, string fontVersion=202, uint textUserLayoutVersion=203, uint textExplicitLayoutVersion=396, uint textBidiVersion=279, float textTracking=27, bool textDecorationSkipInk=412, EmojiImageSet emojiImageSet=391, uint styleID=49, Path[] fillGeometry=51, Path[] strokeGeometry=52, VectorData vectorData=48, VectorMirror handleMirroring=44, uint count=10, float starInnerScale=24, ArcData arcData=195, BooleanOperation booleanOperation=36, string symbolDescription=144, VariableWidthPoint[] variableWidthPoints=447, PluginData[] pluginData=204, SymbolData symbolData=113, NodeChange[] derivedSymbolData=125, GUID overriddenSymbolID=143, ComponentPropDef[] componentPropDefs=266, ComponentPropAssignment[] componentPropAssignments=268, VariableDataMap parameterConsumptionMap=445, GUIDPath guidPath=111, PrototypeInteraction[] prototypeInteractions=226, PrototypeStartingPoint prototypeStartingPoint=249, ScrollDirection scrollDirection=159, ScrollBehavior scrollBehavior=186, OverlayPositionType overlayPositionType=198, OverlayBackgroundInteraction overlayBackgroundInteraction=200, OverlayBackgroundAppearance overlayBackgroundAppearance=201
struct Blob byte[] bytes
message Message MessageType type=1, uint sessionID=2, NodeChange[] nodeChanges=4, Blob[] blobs=6
";

/// Write `doc` as a `.fig` file, with `thumbnail` (a PNG) when given. Returns the bytes and the
/// warnings about what was left out or approximated.
pub fn export(doc: &Document, thumbnail: Option<&[u8]>) -> Result<(Vec<u8>, Vec<String>), FormatError> {
    let mut x = Exporter::new(doc);
    let message = x.message();
    let fail = |e: super::kiwi::KiwiError| FormatError::Io(format!(".fig export: {e}"));
    let schema = Schema::parse(SCHEMA).map_err(fail)?;
    let root = schema.find("Message").ok_or_else(|| FormatError::Io(".fig export: the schema has no Message".into()))?;
    let body = encode::message(&schema, root, &message).map_err(fail)?;
    let canvas = canvas(&encode::schema(&schema), &body)?;
    let background = doc.pages.first().and_then(|p| doc.node(*p)).and_then(|p| match &p.kind {
        NodeKind::Page { background, .. } => Some(*background),
        _ => None,
    });
    let bytes = archive(&doc.name, background.unwrap_or(Color::WHITE), &canvas, &x.files, thumbnail)?;
    Ok((bytes, x.warnings))
}

fn too_large() -> FormatError {
    FormatError::Io("the document is too large for a .fig file".into())
}

/// `canvas.fig`: the magic, the version and the two deflated chunks, each after its length.
fn canvas(schema: &[u8], message: &[u8]) -> Result<Vec<u8>, FormatError> {
    let mut out = b"fig-kiwi".to_vec();
    out.extend_from_slice(&VERSION.to_le_bytes());
    for chunk in [schema, message] {
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(chunk).map_err(crate::io_err)?;
        let c = z.finish().map_err(crate::io_err)?;
        let len = u32::try_from(c.len()).map_err(|_| too_large())?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&c);
    }
    if out.len() as u64 > MAX_CANVAS {
        return Err(too_large());
    }
    Ok(out)
}

/// The zip around `canvas.fig`.
fn archive(
    name: &str,
    background: Color,
    canvas: &[u8],
    images: &[(String, Arc<Vec<u8>>)],
    thumbnail: Option<&[u8]>,
) -> Result<Vec<u8>, FormatError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        crate::put(&mut z, "canvas.fig", stored, canvas)?;
        if let Some(t) = thumbnail {
            crate::put(&mut z, "thumbnail.png", stored, t)?;
        }
        let mut client = serde_json::json!({"background_color": {"r": background.r, "g": background.g, "b": background.b, "a": background.a}});
        // A PNG says its size right after its signature and header chunk name.
        let be = |at: usize| thumbnail.and_then(|t| t.get(at..at + 4)).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes);
        if let (Some(w), Some(h)) = (be(16), be(20)) {
            client["thumbnail_size"] = serde_json::json!({"width": w, "height": h});
        }
        let meta = serde_json::to_vec(&serde_json::json!({"client_meta": client, "file_name": name})).map_err(crate::io_err)?;
        crate::put(&mut z, "meta.json", crate::json_options(), &meta)?;
        let mut total = canvas.len() as u64;
        for (hash, bytes) in images {
            total += bytes.len() as u64;
            if total > MAX_FILE {
                return Err(too_large());
            }
            crate::put(&mut z, &format!("images/{hash}"), stored, bytes)?;
        }
        z.finish().map_err(crate::io_err)?;
    }
    Ok(buf.into_inner())
}

type Fields = Vec<(&'static str, Kv<'static>)>;

fn o(f: Fields) -> Kv<'static> {
    Kv::Object(f)
}
fn arr(v: Vec<Kv<'static>>) -> Kv<'static> {
    Kv::Array(v)
}
/// A float the format can hold: finite (non-finite values become 0) and within range.
fn fl(x: f64) -> Kv<'static> {
    Kv::Float(if x.is_finite() { x.clamp(-MAX_FLOAT, MAX_FLOAT) as f32 } else { 0.0 })
}
fn u(x: u32) -> Kv<'static> {
    Kv::Uint(u64::from(x))
}
fn int(x: i64) -> Kv<'static> {
    Kv::Int(x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)))
}
fn e(name: &'static str) -> Kv<'static> {
    Kv::Enum(name)
}
fn s(x: &str) -> Kv<'static> {
    Kv::Str(x.to_string())
}
fn b(x: bool) -> Kv<'static> {
    Kv::Bool(x)
}
fn guid(session: u32, local: u32) -> Kv<'static> {
    o(vec![("sessionID", u(session)), ("localID", u(local))])
}
fn xy(x: f64, y: f64) -> Kv<'static> {
    o(vec![("x", fl(x)), ("y", fl(y))])
}
fn color(c: Color) -> Kv<'static> {
    o(vec![("r", fl(c.r.into())), ("g", fl(c.g.into())), ("b", fl(c.b.into())), ("a", fl(c.a.into()))])
}
fn matrix(m: Affine) -> Kv<'static> {
    let [a, b, c, d, e, f] = m.as_coeffs();
    o(vec![("m00", fl(a)), ("m01", fl(c)), ("m02", fl(e)), ("m10", fl(b)), ("m11", fl(d)), ("m12", fl(f))])
}
/// What the document and its pages say besides their names: left out, a reader takes them as
/// hidden and fully transparent, and draws nothing below them.
fn root_fields() -> Fields {
    vec![
        ("visible", b(true)),
        ("opacity", fl(1.0)),
        ("transform", matrix(Affine::IDENTITY)),
        ("strokeWeight", fl(0.0)),
        ("strokeAlign", e("CENTER")),
        ("strokeJoin", e("BEVEL")),
    ]
}
/// A finite coordinate (vector network blobs store raw floats).
fn finite(x: f64) -> f64 {
    if x.is_finite() { x.clamp(-MAX_FLOAT, MAX_FLOAT) } else { 0.0 }
}

/// A position string that sorts as child `i` does among its siblings (fixed width, printable).
fn position(i: usize) -> String {
    let mut out = [0u8; 4];
    let mut v = i;
    for c in out.iter_mut().rev() {
        *c = b'!' + (v % 94) as u8;
        v /= 94;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The format's name of a blend mode: the one the import reads back as `m`.
fn blend_name(m: BlendMode) -> &'static str {
    const NAMES: [&str; 19] = [
        "PASS_THROUGH",
        "NORMAL",
        "DARKEN",
        "MULTIPLY",
        "LINEAR_BURN",
        "COLOR_BURN",
        "LIGHTEN",
        "SCREEN",
        "LINEAR_DODGE",
        "COLOR_DODGE",
        "OVERLAY",
        "SOFT_LIGHT",
        "HARD_LIGHT",
        "DIFFERENCE",
        "EXCLUSION",
        "HUE",
        "SATURATION",
        "COLOR",
        "LUMINOSITY",
    ];
    NAMES.into_iter().find(|n| crate::rest::props::blend_mode(n) == Some(m)).unwrap_or("NORMAL")
}

fn cap_name(c: StrokeCap) -> &'static str {
    match c {
        StrokeCap::None => "NONE",
        StrokeCap::Round => "ROUND",
        StrokeCap::Square => "SQUARE",
        StrokeCap::LineArrow => "ARROW_LINES",
        StrokeCap::TriangleArrow | StrokeCap::ReversedTriangle => "ARROW_EQUILATERAL",
        StrokeCap::CircleFilled => "CIRCLE_FILLED",
        StrokeCap::DiamondFilled => "DIAMOND_FILLED",
    }
}

fn join_name(j: StrokeJoin) -> &'static str {
    match j {
        StrokeJoin::Miter => "MITER",
        StrokeJoin::Bevel => "BEVEL",
        StrokeJoin::Round => "ROUND",
    }
}

/// A geometry blob (the import's `svg_path` reads it): commands `0` close, `1` move, `2` line,
/// `3` quadratic, `4` cubic, each followed by its points as little-endian `f32` pairs.
fn path_blob(p: &BezPath) -> Vec<u8> {
    let mut out = Vec::new();
    let mut put = |cmd: u8, pts: &[figcraft_doc::geom::Point]| {
        out.push(cmd);
        for q in pts {
            out.extend_from_slice(&(finite(q.x) as f32).to_le_bytes());
            out.extend_from_slice(&(finite(q.y) as f32).to_le_bytes());
        }
    };
    for el in p.elements() {
        match *el {
            PathEl::MoveTo(a) => put(1, &[a]),
            PathEl::LineTo(a) => put(2, &[a]),
            PathEl::QuadTo(a, c) => put(3, &[a, c]),
            PathEl::CurveTo(a, c, d) => put(4, &[a, c, d]),
            PathEl::ClosePath => put(0, &[]),
        }
    }
    out
}

/// Settings an overlay's destination carries.
struct Overlay {
    position: OverlayPosition,
    close_on_outside: bool,
    background: Option<Color>,
}

struct Exporter<'d> {
    doc: &'d Document,
    /// Layer → local id of its GUID (session [`LAYERS`]).
    ids: BTreeMap<NodeId, u32>,
    nodes: Vec<Kv<'static>>,
    /// Blob bytes → index (identical blobs are written once).
    blobs: BTreeMap<Vec<u8>, u32>,
    blob_bytes: u64,
    /// Image key → its hash (lowercase hex) once written, `None` when its bytes aren't there.
    images: BTreeMap<String, Option<String>>,
    /// `images/<hash>` members.
    files: Vec<(String, Arc<Vec<u8>>)>,
    /// Component property (name, type) → local id of its definition's GUID (session [`AUX`]).
    defs: BTreeMap<(String, &'static str), u32>,
    /// Grid tracks of written grid frames: (columns, rows) local ids (session [`AUX`]).
    tracks: BTreeMap<NodeId, (Vec<u32>, Vec<u32>)>,
    aux: u32,
    overlays: BTreeMap<NodeId, Overlay>,
    /// Flow starting points: layer → (order, name).
    starts: BTreeMap<NodeId, (usize, String)>,
    /// Shared styles, variable collections and variables → local id of their node's GUID (session
    /// [`LAYERS`], after the layers'), and (collection, mode) → local id of the mode's GUID
    /// (session [`AUX`]).
    style_ids: BTreeMap<&'d str, u32>,
    set_ids: BTreeMap<&'d str, u32>,
    var_ids: BTreeMap<&'d str, u32>,
    mode_ids: BTreeMap<(&'d str, &'d str), u32>,
    warnings: Vec<String>,
}

/// The variables bound to one layer, as the fields that carry them: paints by (fills?, index),
/// gradient stops by (fills?, paint, stop), effect and layout grid fields by (index, field), and
/// the entries of the layer's own map.
#[derive(Default)]
struct Bound {
    paints: BTreeMap<(bool, usize), Kv<'static>>,
    stops: BTreeMap<(bool, usize, usize), Kv<'static>>,
    effects: BTreeMap<(usize, &'static str), Kv<'static>>,
    grids: BTreeMap<(usize, &'static str), Kv<'static>>,
    entries: Vec<Kv<'static>>,
}

impl<'d> Exporter<'d> {
    fn new(doc: &'d Document) -> Self {
        Self {
            doc,
            ids: BTreeMap::new(),
            nodes: Vec::new(),
            blobs: BTreeMap::new(),
            blob_bytes: 0,
            images: BTreeMap::new(),
            files: Vec::new(),
            defs: BTreeMap::new(),
            tracks: BTreeMap::new(),
            aux: 0,
            overlays: BTreeMap::new(),
            starts: BTreeMap::new(),
            style_ids: BTreeMap::new(),
            set_ids: BTreeMap::new(),
            var_ids: BTreeMap::new(),
            mode_ids: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn next_aux(&mut self) -> u32 {
        self.aux += 1;
        self.aux
    }

    fn guid_of(&self, id: NodeId) -> Option<Kv<'static>> {
        self.ids.get(&id).map(|l| guid(LAYERS, *l))
    }

    /// The message: the document, its pages and their layers, and the blobs.
    fn message(&mut self) -> Kv<'static> {
        let doc = self.doc;
        self.assign();
        let profile = if doc.color_profile == figcraft_doc::color::ColorProfile::DisplayP3 { "DISPLAY_P3" } else { "SRGB" };
        let mut root: Fields = vec![("guid", guid(0, 0)), ("phase", e("CREATED")), ("type", e("DOCUMENT")), ("name", s(&doc.name))];
        root.extend(root_fields());
        root.push(("documentColorProfile", e(profile)));
        self.nodes.push(o(root));
        for (i, p) in doc.pages.iter().enumerate() {
            self.emit(*p, guid(0, 0), i, None, 0);
        }
        self.assets(doc.pages.len());
        if !doc.comments.is_empty() {
            self.warn("comments were left out");
        }
        let mut blobs: Vec<(u32, Vec<u8>)> = std::mem::take(&mut self.blobs).into_iter().map(|(bytes, i)| (i, bytes)).collect();
        blobs.sort_by_key(|x| x.0);
        o(vec![
            ("type", e("NODE_CHANGES")),
            ("sessionID", u(0)),
            ("nodeChanges", arr(std::mem::take(&mut self.nodes))),
            ("blobs", arr(blobs.into_iter().map(|(_, bytes)| o(vec![("bytes", Kv::Bytes(bytes))])).collect())),
        ])
    }

    /// GUIDs for every layer (pages first, then depth-first), component property definitions,
    /// flow starting points and overlay settings, before anything refers to them.
    fn assign(&mut self) {
        let doc = self.doc;
        let mut stack: Vec<NodeId> = doc.pages.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(n) = doc.node(id) else { continue };
            if self.ids.len() >= MAX_NODES {
                self.warn("the document has more layers than a .fig file is read with; the rest were left out");
                break;
            }
            let local = u32::try_from(self.ids.len() + 1).unwrap_or(u32::MAX);
            self.ids.insert(id, local);
            // An instance's layers aren't written (its main says them), so nothing may refer to them.
            let mirrored = matches!(&n.kind, NodeKind::Instance { component, .. } if doc.node(*component).is_some_and(|c| matches!(c.kind, NodeKind::Component { .. })));
            if !mirrored {
                stack.extend(n.children.iter().rev());
            }
            match &n.kind {
                NodeKind::Component { props, .. } => {
                    for (name, d) in props {
                        if let Some(ty) = prop_type(d) {
                            let key = (name.clone(), ty);
                            if !self.defs.contains_key(&key) {
                                let id = self.next_aux();
                                self.defs.insert(key, id);
                            }
                        }
                    }
                }
                NodeKind::Page { flows, .. } => {
                    for (i, f) in flows.iter().enumerate() {
                        self.starts.entry(f.start).or_insert((i, f.name.clone()));
                    }
                }
                _ => {}
            }
            for a in n.interactions.iter().flat_map(|it| figcraft_doc::proto::walk(&it.actions)) {
                if let Action::OpenOverlay { dest, position, close_on_outside, background, .. } = a {
                    let o = Overlay { position: *position, close_on_outside: *close_on_outside, background: *background };
                    match self.overlays.get(dest) {
                        Some(old) if (old.position, old.close_on_outside, old.background) != (o.position, o.close_on_outside, o.background) => {
                            self.warn("overlays opened with different settings keep the first one's");
                        }
                        Some(_) => {}
                        None => {
                            self.overlays.insert(*dest, o);
                        }
                    }
                }
            }
        }
        // The page that holds styles and variables takes the next id, then their nodes.
        let mut next = u32::try_from(self.ids.len() + 1).unwrap_or(u32::MAX);
        let mut node = || {
            next = next.saturating_add(1);
            next
        };
        for st in &doc.styles {
            self.style_ids.entry(st.id.as_str()).or_insert_with(&mut node);
        }
        for c in &doc.variables.collections {
            self.set_ids.entry(c.id.as_str()).or_insert_with(&mut node);
        }
        for v in &doc.variables.variables {
            if self.set_ids.contains_key(v.collection.as_str()) {
                self.var_ids.entry(v.id.as_str()).or_insert_with(&mut node);
            }
        }
        for c in &doc.variables.collections {
            for m in &c.modes {
                if !self.mode_ids.contains_key(&(c.id.as_str(), m.id.as_str())) {
                    let id = self.next_aux();
                    self.mode_ids.insert((c.id.as_str(), m.id.as_str()), id);
                }
            }
        }
    }

    /// The hidden page after the `pages` real ones that holds the shared styles, the variable
    /// collections and the variables, each a node of its own.
    fn assets(&mut self, pages: usize) {
        let doc = self.doc;
        if self.style_ids.is_empty() && self.set_ids.is_empty() {
            return;
        }
        let page = u32::try_from(self.ids.len() + 1).unwrap_or(u32::MAX);
        let mut f: Fields = vec![
            ("guid", guid(LAYERS, page)),
            ("phase", e("CREATED")),
            ("parentIndex", o(vec![("guid", guid(0, 0)), ("position", s(&position(pages)))])),
            ("type", e("CANVAS")),
            ("name", s("Internal Only Canvas")),
        ];
        f.extend(root_fields().into_iter().map(|(k, v)| if k == "visible" { (k, b(false)) } else { (k, v) }));
        f.push(("internalOnly", b(true)));
        self.nodes.push(o(f));
        let mut index = 0usize;
        let mut head = |local: u32, ty: &'static str, name: &str| -> Fields {
            index += 1;
            vec![
                ("guid", guid(LAYERS, local)),
                ("phase", e("CREATED")),
                ("parentIndex", o(vec![("guid", guid(LAYERS, page)), ("position", s(&position(index)))])),
                ("type", e(ty)),
                ("name", s(name)),
                ("sortPosition", s(&position(index))),
            ]
        };
        for st in &doc.styles {
            let Some(&local) = self.style_ids.get(st.id.as_str()) else { continue };
            // A sample layer that shows the style: a square for paints and effects, a frame for
            // grids, two letters for text.
            let (ty, kind) = match &st.kind {
                StyleKind::Paint { .. } => ("ROUNDED_RECTANGLE", "FILL"),
                StyleKind::Effect { .. } => ("ROUNDED_RECTANGLE", "EFFECT"),
                StyleKind::Grid { .. } => ("FRAME", "GRID"),
                StyleKind::Text { .. } => ("TEXT", "TEXT"),
            };
            let mut f = head(local, ty, &st.name);
            f.extend([
                ("styleType", e(kind)),
                ("visible", b(true)),
                ("opacity", fl(1.0)),
                ("size", xy(100.0, 100.0)),
                ("transform", matrix(Affine::IDENTITY)),
                ("strokeWeight", fl(0.0)),
                ("strokeAlign", e("INSIDE")),
                ("strokeJoin", e("MITER")),
            ]);
            if !st.description.is_empty() {
                f.push(("description", s(&st.description)));
            }
            // The style's variable bindings ride on its sample layer, as a layer's do.
            let none = if st.bound.is_empty() {
                Bound::default()
            } else {
                let mut sample = Node::new(NodeId(0), NodeKind::Rectangle, "", 0.0, 0.0);
                if let StyleKind::Text { style } = &st.kind {
                    sample.kind = NodeKind::Text(TextData { style: style.as_text_style(), ..TextData::new("Ag") });
                }
                sample.bound.clone_from(&st.bound);
                self.bound(&sample)
            };
            let entries = none.entries.clone();
            if !entries.is_empty() {
                f.push(("variableConsumptionMap", o(vec![("entries", arr(entries))])));
            }
            match &st.kind {
                StyleKind::Paint { paints } => f.push(("fillPaints", self.paints(paints, true, &none))),
                StyleKind::Effect { effects } => {
                    let list = effects.iter().enumerate().map(|(i, x)| self.effect(x, i, &none)).collect();
                    f.extend([("fillPaints", arr(Vec::new())), ("effects", arr(list))]);
                }
                StyleKind::Grid { grids } => {
                    let list = grids.iter().enumerate().map(|(i, g)| layout_grid(g, i, &none)).collect();
                    f.extend([("fillPaints", arr(Vec::new())), ("layoutGrids", arr(list))]);
                }
                StyleKind::Text { style } => {
                    let td = TextData { style: style.as_text_style(), ..TextData::new("Ag") };
                    let fills = [Paint::solid(Color::BLACK)];
                    f.push(("fillPaints", self.paints(style.fills.as_deref().unwrap_or(&fills), true, &none)));
                    self.text(&td, &fills, &mut f);
                }
            }
            self.nodes.push(o(f));
        }
        for c in &doc.variables.collections {
            let Some(&local) = self.set_ids.get(c.id.as_str()) else { continue };
            let mut f = head(local, "VARIABLE_SET", &c.name);
            let modes = c.modes.iter().enumerate().filter_map(|(i, m)| {
                let id = self.mode_ids.get(&(c.id.as_str(), m.id.as_str()))?;
                Some(o(vec![("id", guid(AUX, *id)), ("name", s(&m.name)), ("sortPosition", s(&position(i)))]))
            });
            f.push(("variableSetModes", arr(modes.collect())));
            self.nodes.push(o(f));
            if c.extends.is_some() {
                self.warn("extended variable collections were written as collections of their own");
            }
        }
        for v in &doc.variables.variables {
            let ids = (self.var_ids.get(v.id.as_str()), self.set_ids.get(v.collection.as_str()));
            let (Some(&local), Some(&set), Some(c)) = (ids.0, ids.1, doc.variables.collections.iter().find(|c| c.id == v.collection)) else {
                self.warn("variables whose collection isn't in the document were left out");
                continue;
            };
            let resolved = resolved_type(v.kind);
            let mut values = Vec::new();
            for m in &c.modes {
                let (Some(mode), Some(val)) = (self.mode_ids.get(&(c.id.as_str(), m.id.as_str())), v.values.get(&m.id)) else { continue };
                let (ty, value) = match val {
                    VariableValue::Color(x) => ("COLOR", ("colorValue", color(*x))),
                    VariableValue::Number(x) => ("FLOAT", ("floatValue", fl(*x))),
                    // NULs can't be stored in a string.
                    VariableValue::String(x) => ("STRING", ("textValue", s(&x.replace('\0', "")))),
                    VariableValue::Boolean(x) => ("BOOLEAN", ("boolValue", b(*x))),
                    VariableValue::Alias(a) => match self.var_ids.get(a.as_str()) {
                        Some(t) => ("ALIAS", ("alias", o(vec![("guid", guid(LAYERS, *t))]))),
                        None => {
                            self.warn("variable aliases to variables that aren't in the document were left out");
                            continue;
                        }
                    },
                };
                let data = o(vec![("value", o(vec![value])), ("dataType", e(ty)), ("resolvedDataType", e(resolved))]);
                values.push(o(vec![("modeID", guid(AUX, *mode)), ("variableData", data)]));
            }
            let mut f = head(local, "VARIABLE", &v.name);
            f.extend([
                ("variableSetID", o(vec![("guid", guid(LAYERS, set))])),
                ("variableResolvedType", e(resolved)),
                ("variableDataValues", o(vec![("entries", arr(values))])),
            ]);
            if !v.description.is_empty() {
                f.push(("description", s(&v.description)));
            }
            if let Some(scopes) = &v.scopes {
                let known: Vec<Kv> = scopes.iter().filter_map(|sc| scope_name(sc)).map(e).collect();
                if known.len() < scopes.len() {
                    self.warn("variable scopes the format has no name for were left out");
                }
                f.push(("variableScopes", arr(known)));
            }
            if v.hidden_from_publishing || !v.code_syntax.is_empty() || v.extends.is_some() {
                self.warn("variables' publishing, code syntax and inheritance settings were left out");
            }
            self.nodes.push(o(f));
        }
    }

    /// A reference to variable `var` (its id or name) as variable data.
    fn alias(&mut self, var: &str) -> Option<Kv<'static>> {
        let found = self.doc.variables.get(var).and_then(|v| Some((v.kind, *self.var_ids.get(v.id.as_str())?)));
        let Some((kind, local)) = found else {
            self.warn("bindings to variables that aren't in the document were written as their values");
            return None;
        };
        Some(o(vec![
            ("value", o(vec![("alias", o(vec![("guid", guid(LAYERS, local))]))])),
            ("dataType", e("ALIAS")),
            ("resolvedDataType", e(resolved_type(kind))),
        ]))
    }

    /// Layer `n`'s variable bindings, sorted into the fields that carry them. What the format has
    /// no field for stays as the value the layer already shows, with a warning.
    fn bound(&mut self, n: &Node) -> Bound {
        const BAKED: &str = "variables bound to properties the format has no field for were written as their values";
        let mut out = Bound::default();
        for (key, var) in &n.bound {
            let parts: Vec<&str> = key.split('/').collect();
            let index = |i: &str| i.parse::<usize>().ok().filter(|_| i.bytes().all(|c| c.is_ascii_digit()));
            let fields: &[&'static str] = match (parts.as_slice(), key.as_str()) {
                (_, "fill" | "fills" | "fills/0" | "stroke") => {
                    if let Some(a) = self.alias(var) {
                        out.paints.insert((key != "stroke", 0), a);
                    }
                    continue;
                }
                ([list @ ("fills" | "strokes"), i, "color"], _) => {
                    if let (Some(i), Some(a)) = (index(i), self.alias(var)) {
                        out.paints.insert((*list == "fills", i), a);
                    }
                    continue;
                }
                ([list @ ("fills" | "strokes"), i, "stops", j, "color"], _) => {
                    if let (Some(i), Some(j), Some(a)) = (index(i), index(j), self.alias(var)) {
                        out.stops.insert((*list == "fills", i, j), a);
                    }
                    continue;
                }
                ([list @ ("effects" | "layoutGrids"), i, field], _) => {
                    let names: &[&'static str] = if *list == "effects" {
                        &["color", "radius", "spread", "offsetX", "offsetY", "startRadius"]
                    } else {
                        &["size", "count", "gutter", "offset"]
                    };
                    match (index(i), names.iter().find(|f| *f == field)) {
                        (Some(i), Some(name)) => {
                            if let Some(a) = self.alias(var) {
                                if *list == "effects" { &mut out.effects } else { &mut out.grids }.insert((i, *name), a);
                            }
                        }
                        _ => self.warn(BAKED),
                    }
                    continue;
                }
                (_, "width") => &["WIDTH"],
                (_, "height") => &["HEIGHT"],
                (_, "gap") => &["STACK_SPACING"],
                (_, "crossGap") => &["STACK_COUNTER_SPACING"],
                (_, "paddingLeft") => &["STACK_PADDING_LEFT"],
                (_, "paddingTop") => &["STACK_PADDING_TOP"],
                (_, "paddingRight") => &["STACK_PADDING_RIGHT"],
                (_, "paddingBottom") => &["STACK_PADDING_BOTTOM"],
                (_, "paddingHorizontal") => &["STACK_PADDING_LEFT", "STACK_PADDING_RIGHT"],
                (_, "paddingVertical") => &["STACK_PADDING_TOP", "STACK_PADDING_BOTTOM"],
                (_, "padding") => &["STACK_PADDING_LEFT", "STACK_PADDING_TOP", "STACK_PADDING_RIGHT", "STACK_PADDING_BOTTOM"],
                (_, "cornerRadius") => &[
                    "CORNER_RADIUS",
                    "RECTANGLE_TOP_LEFT_CORNER_RADIUS",
                    "RECTANGLE_TOP_RIGHT_CORNER_RADIUS",
                    "RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS",
                    "RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS",
                ],
                (_, "topLeftRadius") => &["RECTANGLE_TOP_LEFT_CORNER_RADIUS"],
                (_, "topRightRadius") => &["RECTANGLE_TOP_RIGHT_CORNER_RADIUS"],
                (_, "bottomLeftRadius") => &["RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS"],
                (_, "bottomRightRadius") => &["RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS"],
                (_, "opacity") => &["OPACITY"],
                (_, "visible") => &["VISIBLE"],
                (_, "characters") => &["TEXT_DATA"],
                (_, "strokeWeight") => &["STROKE_WEIGHT"],
                (_, "strokeTopWeight") => &["BORDER_TOP_WEIGHT"],
                (_, "strokeRightWeight") => &["BORDER_RIGHT_WEIGHT"],
                (_, "strokeBottomWeight") => &["BORDER_BOTTOM_WEIGHT"],
                (_, "strokeLeftWeight") => &["BORDER_LEFT_WEIGHT"],
                (_, "minWidth") => &["MIN_WIDTH"],
                (_, "maxWidth") => &["MAX_WIDTH"],
                (_, "minHeight") => &["MIN_HEIGHT"],
                (_, "maxHeight") => &["MAX_HEIGHT"],
                (_, "fontSize") => &["FONT_SIZE"],
                (_, "fontFamily") => &["FONT_FAMILY"],
                (_, "fontStyle") => &["FONT_STYLE"],
                (_, "letterSpacing") => &["LETTER_SPACING"],
                (_, "lineHeight") => &["LINE_HEIGHT"],
                (_, "paragraphSpacing") => &["PARAGRAPH_SPACING"],
                (_, "paragraphIndent") => &["PARAGRAPH_INDENT"],
                _ => {
                    self.warn(BAKED);
                    continue;
                }
            };
            // Each entry names the bound property and the number of the layer field that holds it.
            for field in fields {
                if let Some(a) = self.alias(var) {
                    let mut entry: Fields = node_field(field).into_iter().map(|id| ("nodeField", u(id))).collect();
                    entry.extend([("variableData", a), ("variableField", e(field))]);
                    out.entries.push(o(entry));
                }
            }
        }
        out
    }

    /// Write layer `id` (child `index` of the layer with GUID `parent_guid`) and its subtree.
    fn emit(&mut self, id: NodeId, parent_guid: Kv<'static>, index: usize, parent: Option<&Node>, depth: usize) {
        let doc = self.doc;
        let (Some(&local), Some(n)) = (self.ids.get(&id), doc.node(id)) else { return };
        if depth > MAX_DEPTH {
            self.warn("layers nested deeper than 256 levels were left out");
            return;
        }
        let mut f: Fields = vec![
            ("guid", guid(LAYERS, local)),
            ("phase", e("CREATED")),
            ("parentIndex", o(vec![("guid", parent_guid), ("position", s(&position(index)))])),
        ];
        let children = if let NodeKind::Page { background, guides, prototype, .. } = &n.kind {
            f.extend([("type", e("CANVAS")), ("name", s(&n.name))]);
            f.extend(root_fields());
            f.extend([("backgroundColor", color(*background)), ("backgroundOpacity", fl(background.a.into())), ("backgroundEnabled", b(true))]);
            if !guides.is_empty() {
                self.warn("ruler guides were left out");
            }
            if *prototype != Default::default() {
                self.warn("prototype device settings were left out");
            }
            self.modes(n, &mut f);
            true
        } else {
            self.layer(n, parent, &mut f, false)
        };
        self.nodes.push(o(f));
        if children {
            for (i, c) in n.children.iter().enumerate() {
                self.emit(*c, guid(LAYERS, local), i, Some(n), depth + 1);
            }
        }
    }

    /// The fields of layer `n` (whose parent is `parent`). `nested`: for an instance's override
    /// (no type, no instance or component data). Returns whether its children are written as
    /// layers (an instance's come from its main).
    fn layer(&mut self, n: &Node, parent: Option<&Node>, f: &mut Fields, nested: bool) -> bool {
        let doc = self.doc;
        if let Some((outline, original)) = outlined(n) {
            // The format's variable width is the same on both sides of the path: a stroke wider on
            // one side is written as the area it paints, with the stroke itself kept as plugin data
            // (which the import puts back).
            self.warn("strokes wider on one side of their path were written as filled outlines (the stroke itself is kept for FigCraft)");
            let children = self.layer(&outline, parent, f, nested);
            f.push(("pluginData", arr(vec![o(vec![("pluginID", s(STASH.0)), ("value", s(&original)), ("key", s(STASH.1))])])));
            return children;
        }
        let main = match &n.kind {
            NodeKind::Instance { component, .. } => {
                let found = self.ids.contains_key(component) && doc.node(*component).is_some_and(|c| matches!(c.kind, NodeKind::Component { .. }));
                if !found && !nested {
                    self.warn("instances of components that aren't in the file were written as frames");
                }
                found.then_some(*component)
            }
            _ => None,
        };
        let ty = match &n.kind {
            NodeKind::Page { .. } | NodeKind::Frame | NodeKind::Group | NodeKind::ComponentSet { .. } => "FRAME",
            NodeKind::Instance { .. } if main.is_none() => "FRAME",
            NodeKind::Section { .. } => "SECTION",
            NodeKind::Rectangle => "ROUNDED_RECTANGLE",
            NodeKind::Ellipse { .. } => "ELLIPSE",
            NodeKind::Polygon { .. } => "REGULAR_POLYGON",
            NodeKind::Star { .. } => "STAR",
            NodeKind::Line => "LINE",
            NodeKind::Vector { .. } => "VECTOR",
            NodeKind::Text(_) => "TEXT",
            NodeKind::BooleanOp { .. } => "BOOLEAN_OPERATION",
            NodeKind::Component { .. } => "SYMBOL",
            NodeKind::Instance { .. } => "INSTANCE",
            NodeKind::Slice => "SLICE",
        };
        if !nested {
            f.push(("type", e(ty)));
        }
        f.push(("name", s(&n.name)));
        self.common(n, parent, f);
        match &n.kind {
            NodeKind::Group => f.push(("resizeToFit", b(true))),
            NodeKind::Section { contents_hidden } => f.push(("sectionContentsHidden", b(*contents_hidden))),
            NodeKind::Ellipse { start, sweep, inner } => {
                f.push(("arcData", o(vec![("startingAngle", fl(*start)), ("endingAngle", fl(start + sweep)), ("innerRadius", fl(*inner))])))
            }
            NodeKind::Polygon { count } => f.push(("count", u(*count))),
            NodeKind::Star { count, inner_ratio } => f.extend([("count", u(*count)), ("starInnerScale", fl(*inner_ratio))]),
            NodeKind::Vector { network } => self.vector(n, network, f),
            NodeKind::Text(td) => self.text(td, &n.fills, f),
            NodeKind::BooleanOp { op } => {
                let name = match op {
                    BoolOp::Union => "UNION",
                    BoolOp::Subtract => "SUBTRACT",
                    BoolOp::Intersect => "INTERSECT",
                    BoolOp::Exclude => "XOR",
                };
                f.push(("booleanOperation", e(name)));
                if n.children.iter().filter_map(|c| doc.node(*c)).any(|c| c.bool_op.is_some()) {
                    self.warn("boolean groups whose layers combine in different ways use the group's operation");
                }
            }
            NodeKind::Component { description, props } if !nested => {
                let defs: Vec<Kv> = props.iter().filter_map(|(name, d)| self.prop_def(name, d)).collect();
                if !defs.is_empty() {
                    f.push(("componentPropDefs", arr(defs)));
                }
                if props.values().any(|d| matches!(d, PropDef::Slot { .. })) {
                    self.warn("slot properties were left out");
                }
                if !description.is_empty() {
                    f.push(("symbolDescription", s(description)));
                }
            }
            NodeKind::ComponentSet { description } if !nested => {
                f.push(("isStateGroup", b(true)));
                if !description.is_empty() {
                    f.push(("symbolDescription", s(description)));
                }
            }
            NodeKind::Instance { props, .. } if !nested => {
                if let Some(main) = main {
                    self.instance(n, main, props, f);
                }
            }
            _ => {}
        }
        let boxy = n.kind.is_frame_like() || matches!(n.kind, NodeKind::Rectangle | NodeKind::Section { .. });
        if (boxy || matches!(n.kind, NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. }))
            && let Some(p) = figcraft_doc::geometry::shape_path(n)
        {
            self.geometry("fillGeometry", &p, WindingRule::NonZero, f);
        }
        self.stroke_geometry(n, boxy, f);
        main.is_none()
    }

    /// Fields every layer kind shares.
    fn common(&mut self, n: &Node, parent: Option<&Node>, f: &mut Fields) {
        f.extend([
            ("visible", b(n.visible)),
            ("locked", b(n.locked)),
            ("opacity", fl(n.opacity.into())),
            ("blendMode", e(blend_name(n.blend))),
            ("size", xy(n.width, n.height)),
            ("transform", matrix(n.transform)),
        ]);
        if n.is_mask {
            let ty = match n.mask_type {
                MaskType::Alpha => "ALPHA",
                MaskType::Vector => "OUTLINE",
                MaskType::Luminance => "LUMINANCE",
            };
            f.extend([("mask", b(true)), ("maskType", e(ty))]);
        }
        let bound = self.bound(n);
        // Text whose own style sets fills paints with them (the layer's fills only show through
        // runs); a variable bound to the layer's fill colours the layer's.
        let fills = match &n.kind {
            NodeKind::Text(TextData { style: TextStyle { fills: Some(own), .. }, .. }) => self.paints(own, true, &Bound::default()),
            _ => self.paints(&n.fills, true, &bound),
        };
        let strokes = self.paints(&n.strokes, false, &bound);
        f.extend([("fillPaints", fills), ("strokePaints", strokes)]);
        self.stroke(n, f);
        if !n.effects.is_empty() {
            let effects = n.effects.iter().enumerate().map(|(i, x)| self.effect(x, i, &bound)).collect();
            f.push(("effects", arr(effects)));
        }
        if n.kind.has_corner_radius() {
            let r = n.corners.radii;
            f.push(("cornerRadius", fl(r[0])));
            let differ = r.iter().any(|x| *x != r[0]);
            if n.kind.has_independent_corners() && (differ || r[0] > 0.0) {
                f.extend(differ.then(|| ("rectangleCornerRadiiIndependent", b(true))));
                f.extend([
                    ("rectangleTopLeftCornerRadius", fl(r[0])),
                    ("rectangleTopRightCornerRadius", fl(r[1])),
                    ("rectangleBottomRightCornerRadius", fl(r[2])),
                    ("rectangleBottomLeftCornerRadius", fl(r[3])),
                ]);
            }
            if n.corners.smoothing > 0.0 {
                f.push(("cornerSmoothing", fl(n.corners.smoothing)));
            }
        }
        let h = match n.constraints.horizontal {
            HConstraint::Left => "MIN",
            HConstraint::Right => "MAX",
            HConstraint::LeftRight => "STRETCH",
            HConstraint::Center => "CENTER",
            HConstraint::Scale => "SCALE",
        };
        let v = match n.constraints.vertical {
            VConstraint::Top => "MIN",
            VConstraint::Bottom => "MAX",
            VConstraint::TopBottom => "STRETCH",
            VConstraint::Center => "CENTER",
            VConstraint::Scale => "SCALE",
        };
        f.extend([("horizontalConstraint", e(h)), ("verticalConstraint", e(v))]);
        if n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. }) {
            f.push(("frameMaskDisabled", b(!n.clip)));
        }
        if n.kind.is_frame_like() {
            if let Some(l) = &n.layout {
                self.auto_layout(n, l, f);
            }
            if !n.grids.is_empty() {
                f.push(("layoutGrids", arr(n.grids.iter().enumerate().map(|(i, g)| layout_grid(g, i, &bound)).collect())));
            }
        }
        self.child_layout(n, parent, f);
        if !n.exports.is_empty() {
            if n.exports.iter().any(|x| x.format == figcraft_doc::ExportFormat::Webp) {
                self.warn("WebP export settings were written as PNG");
            }
            f.push(("exportSettings", arr(n.exports.iter().map(export_setting).collect())));
        }
        self.prototype(n, f);
        let refs: Vec<Kv> = n.prop_refs.iter().filter_map(|(field, prop)| self.prop_ref(field, prop)).collect();
        if !refs.is_empty() {
            f.push(("parameterConsumptionMap", o(vec![("entries", arr(refs))])));
        }
        let text = matches!(n.kind, NodeKind::Text(_));
        let links = [
            ("styleIdForFill", &n.styles.fill, true),
            ("styleIdForStrokeFill", &n.styles.stroke, true),
            ("styleIdForEffect", &n.styles.effect, true),
            ("styleIdForGrid", &n.styles.grid, n.kind.is_frame_like()),
            ("styleIdForText", &n.styles.text, text),
        ];
        for (key, id, fits) in links {
            let Some(id) = id else { continue };
            match self.style_ids.get(id.as_str()).filter(|_| fits) {
                Some(local) => f.push((key, o(vec![("guid", guid(LAYERS, *local))]))),
                None => self.warn("links to shared styles that aren't in the document were left out (their layers keep the values)"),
            }
        }
        if let NodeKind::Text(td) = &n.kind
            && n.styles.text.is_none()
            && std::iter::once(&td.style).chain(td.runs.iter().map(|r| &r.style)).any(|st| st.style_id.is_some())
        {
            self.warn("text styles applied to part of a text were written as their values");
        }
        if !bound.entries.is_empty() {
            f.push(("variableConsumptionMap", o(vec![("entries", arr(bound.entries))])));
        }
        self.modes(n, f);
        if !n.guides.is_empty() {
            self.warn("ruler guides were left out");
        }
        if !n.annotations.is_empty() || !n.dev_resources.is_empty() || !n.code_links.is_empty() || !n.dev_status.is_none() {
            self.warn("Dev Mode annotations, statuses, resources and code links were left out");
        }
        if !n.plugin_data.is_empty() {
            self.warn("plugin data was left out");
        }
    }

    /// The variable modes layer (or page) `n` sets for what is in it.
    fn modes(&mut self, n: &Node, f: &mut Fields) {
        let mut entries = Vec::new();
        for (coll, mode) in &n.var_modes {
            match (self.set_ids.get(coll.as_str()), self.mode_ids.get(&(coll.as_str(), mode.as_str()))) {
                (Some(set), Some(mode)) => {
                    entries.push(o(vec![("variableSetID", o(vec![("guid", guid(LAYERS, *set))])), ("variableModeID", guid(AUX, *mode))]));
                }
                _ => self.warn("variable modes of collections that aren't in the document were left out"),
            }
        }
        if !entries.is_empty() {
            f.push(("variableModeBySetMap", o(vec![("entries", arr(entries))])));
        }
    }

    /// A paint list: a layer's fills (`fills`) or strokes with the variables `bound` to them.
    fn paints(&mut self, list: &[Paint], fills: bool, bound: &Bound) -> Kv<'static> {
        arr(list.iter().enumerate().filter_map(|(i, p)| self.paint(p, (fills, i), bound)).collect())
    }

    fn paint(&mut self, p: &Paint, at: (bool, usize), bound: &Bound) -> Option<Kv<'static>> {
        let mut f: Fields = vec![("visible", b(p.visible)), ("blendMode", e(blend_name(p.blend)))];
        let mut opacity = f64::from(p.opacity);
        match &p.kind {
            PaintKind::Solid { color: c } => {
                opacity *= f64::from(c.a);
                f.extend([("type", e("SOLID")), ("color", color(c.with_alpha(1.0)))]);
                f.extend(bound.paints.get(&at).map(|a| ("colorVar", a.clone())));
            }
            PaintKind::Gradient(g) => {
                let ty = match g.kind {
                    GradientKind::Linear => "GRADIENT_LINEAR",
                    GradientKind::Radial => "GRADIENT_RADIAL",
                    GradientKind::Angular => "GRADIENT_ANGULAR",
                    GradientKind::Diamond => "GRADIENT_DIAMOND",
                };
                // The handles in the unit box come from the inverse of the paint transform: it maps
                // (0, ½) → start, (1, ½) → end, (0, 1) → width for linear gradients, and the centre
                // (½, ½) and the points half a box away from it for the others.
                let (from, to, width) = (g.from, g.to, g.width);
                let (ax, ay) =
                    if g.kind == GradientKind::Linear { (to.0 - from.0, to.1 - from.1) } else { (2.0 * (to.0 - from.0), 2.0 * (to.1 - from.1)) };
                let (cx, cy) = (2.0 * (width.0 - from.0), 2.0 * (width.1 - from.1));
                let (ex, ey) = if g.kind == GradientKind::Linear {
                    (from.0 - 0.5 * cx, from.1 - 0.5 * cy)
                } else {
                    (from.0 - 0.5 * (ax + cx), from.1 - 0.5 * (ay + cy))
                };
                let handles = Affine::new([ax, ay, cx, cy, ex, ey]);
                let t = if handles.determinant().abs() > 1e-12 && handles.as_coeffs().iter().all(|x| x.is_finite()) {
                    handles.inverse()
                } else {
                    self.warn("gradients with handles on one line were written with the default direction");
                    Affine::IDENTITY
                };
                let stops = g.stops.iter().map(|st| o(vec![("color", color(st.color)), ("position", fl(st.position.into()))])).collect();
                f.extend([("type", e(ty)), ("stops", arr(stops)), ("transform", matrix(t))]);
                // Stops bound to variables: every stop again, the bound ones with their variable.
                if bound.stops.range((at.0, at.1, 0)..=(at.0, at.1, usize::MAX)).next().is_some() {
                    let vars = g.stops.iter().enumerate().map(|(j, st)| {
                        let mut sf: Fields = vec![("color", color(st.color))];
                        sf.extend(bound.stops.get(&(at.0, at.1, j)).map(|a| ("colorVar", a.clone())));
                        sf.push(("position", fl(st.position.into())));
                        o(sf)
                    });
                    f.push(("stopsVar", arr(vars.collect())));
                }
            }
            PaintKind::Image(img) => {
                let hash = self.image(&img.image);
                let mode = match img.scale_mode {
                    ImageScaleMode::Fill => "FILL",
                    ImageScaleMode::Fit => "FIT",
                    ImageScaleMode::Crop => "STRETCH",
                    ImageScaleMode::Tile => "TILE",
                };
                f.extend([
                    ("type", e("IMAGE")),
                    ("image", o(vec![("hash", Kv::Bytes(hash)), ("name", s(""))])),
                    ("imageScaleMode", e(mode)),
                    ("rotation", fl(f64::from(img.rotation.rem_euclid(4)) * 90.0)),
                    ("scale", fl(img.scale)),
                ]);
                if let (ImageScaleMode::Crop, Some(c)) = (img.scale_mode, img.crop) {
                    f.push(("transform", matrix(Affine::new(c))));
                }
                if img.filters != Default::default() {
                    self.warn("image adjustments (exposure, contrast, …) were left out");
                }
            }
            PaintKind::Pattern(_) => {
                self.warn("pattern fills were left out");
                return None;
            }
        }
        f.push(("opacity", fl(opacity)));
        Some(o(f))
    }

    /// The hash bytes of image `key`, writing its file once (empty when its bytes aren't there).
    fn image(&mut self, key: &str) -> Vec<u8> {
        let hex = match self.images.get(key) {
            Some(h) => h.clone(),
            None => {
                let h = match self.doc.images.get(key) {
                    Some(img) if img.bytes.len() as u64 <= MAX_IMAGE => {
                        let h = crate::image_hash(&img.bytes);
                        if !self.files.iter().any(|(x, _)| *x == h) {
                            self.files.push((h.clone(), img.bytes.clone()));
                        }
                        Some(h)
                    }
                    Some(_) => {
                        self.warn("images larger than 256 MB were left out");
                        None
                    }
                    None => {
                        self.warn("image fills whose image isn't in the document were written without one");
                        None
                    }
                };
                self.images.insert(key.to_string(), h.clone());
                h
            }
        };
        let hex = hex.unwrap_or_default();
        (0..hex.len() / 2).filter_map(|i| hex.get(2 * i..2 * i + 2).and_then(|d| u8::from_str_radix(d, 16).ok())).collect()
    }

    fn stroke(&mut self, n: &Node, f: &mut Fields) {
        let sp = &n.stroke;
        let align = match sp.align {
            StrokeAlign::Inside => "INSIDE",
            StrokeAlign::Center => "CENTER",
            StrokeAlign::Outside => "OUTSIDE",
        };
        f.extend([
            ("strokeWeight", fl(sp.weight)),
            ("strokeAlign", e(align)),
            ("strokeJoin", e(join_name(sp.join))),
            ("strokeCap", e(cap_name(sp.cap_start))),
            ("miterLimit", fl(sp.miter_limit)),
        ]);
        if !sp.dashes.is_empty() {
            f.push(("dashPattern", arr(sp.dashes.iter().map(|d| fl(*d)).collect())));
        }
        if let Some([t, r, bo, l]) = sp.sides {
            f.extend([
                ("borderStrokeWeightsIndependent", b(true)),
                ("borderTopWeight", fl(t)),
                ("borderRightWeight", fl(r)),
                ("borderBottomWeight", fl(bo)),
                ("borderLeftWeight", fl(l)),
            ]);
        }
        // A variable width: how far the stroke reaches on each side of the path at positions along
        // its length, as a share of the stroke weight (half of it each for the full weight). The
        // two sides are the same: a reader draws nothing for a point with unequal sides.
        if let Some(p) = sp.profile().filter(|_| sp.weight.is_finite() && sp.weight > 0.0) {
            let point = |q: &figcraft_doc::WidthPoint| o(vec![("position", fl(q.t)), ("ascent", fl(q.scale / 2.0)), ("descent", fl(q.scale / 2.0))]);
            let p = p.sanitized();
            if one_sided(&p) {
                self.warn("strokes wider on one side of their path were written with that width on both sides");
            }
            f.push(("variableWidthPoints", arr(p.points.iter().map(point).collect())));
        }
        if sp.brush.is_some() {
            self.warn("brush strokes were written as plain strokes");
        }
        if [sp.cap_start, sp.cap_end].contains(&StrokeCap::ReversedTriangle) {
            self.warn("reversed triangle stroke ends were written as triangle arrows");
        }
        // Vectors keep each end's cap on their end points; other layers have one cap.
        if sp.cap_start != sp.cap_end && !matches!(n.kind, NodeKind::Vector { .. }) {
            self.warn("lines with different stroke ends use the start's at both ends");
        }
    }

    /// Effect `index` of a layer, with the variables `bound` to its fields.
    fn effect(&mut self, x: &figcraft_doc::Effect, index: usize, bound: &Bound) -> Kv<'static> {
        let mut f: Fields = vec![("visible", b(x.visible))];
        let vars = [
            ("color", "colorVar"),
            ("radius", "radiusVar"),
            ("spread", "spreadVar"),
            ("offsetX", "xVar"),
            ("offsetY", "yVar"),
            ("startRadius", "startRadiusVar"),
        ];
        f.extend(vars.into_iter().filter_map(|(ours, key)| Some((key, bound.effects.get(&(index, ours))?.clone()))));
        let blur = |f: &mut Fields, ty: &'static str, bl: &figcraft_doc::Blur| {
            f.extend([("type", e(ty)), ("radius", fl(bl.radius))]);
            if bl.blur_type == figcraft_doc::BlurType::Progressive {
                f.extend([
                    ("blurOpType", e("PROGRESSIVE")),
                    ("startRadius", fl(bl.start_radius)),
                    ("startOffset", xy(bl.start.0, bl.start.1)),
                    ("endOffset", xy(bl.end.0, bl.end.1)),
                ]);
            }
        };
        let grain = |size: f64, size_y: Option<f64>| xy(size, size_y.unwrap_or(size));
        match &x.kind {
            EffectKind::DropShadow(sh) | EffectKind::InnerShadow(sh) => {
                let ty = if matches!(x.kind, EffectKind::DropShadow(_)) { "DROP_SHADOW" } else { "INNER_SHADOW" };
                f.extend([
                    ("type", e(ty)),
                    ("color", color(sh.color)),
                    ("offset", xy(sh.offset.0, sh.offset.1)),
                    ("radius", fl(sh.radius)),
                    ("spread", fl(sh.spread)),
                    ("blendMode", e(blend_name(sh.blend))),
                    ("showShadowBehindNode", b(sh.show_behind)),
                ]);
            }
            EffectKind::LayerBlur(bl) => blur(&mut f, "FOREGROUND_BLUR", bl),
            EffectKind::BackgroundBlur(bl) => blur(&mut f, "BACKGROUND_BLUR", bl),
            EffectKind::Noise(nz) => {
                let mode = match nz.mode {
                    NoiseMode::Mono => "MONOTONE",
                    NoiseMode::Duo => "DUOTONE",
                    NoiseMode::Multi => "MULTITONE",
                };
                f.extend([
                    ("type", e("NOISE")),
                    ("noiseType", e(mode)),
                    ("noiseSize", grain(nz.size, nz.size_y)),
                    ("density", fl(nz.density)),
                    ("color", color(nz.color)),
                    ("secondaryColor", color(nz.secondary)),
                    ("opacity", fl(nz.opacity.into())),
                    ("blendMode", e(blend_name(nz.blend))),
                ]);
            }
            EffectKind::Texture(t) => {
                f.extend([
                    ("type", e("GRAIN")),
                    ("noiseSize", grain(t.size, t.size_y)),
                    ("radius", fl(t.radius)),
                    ("clipToShape", b(t.clip_to_shape)),
                ]);
            }
            EffectKind::Glass(g) => {
                self.warn("glass effects keep only their frost");
                f.extend([("type", e("GLASS")), ("radius", fl(g.frost))]);
            }
        }
        o(f)
    }

    fn auto_layout(&mut self, n: &Node, l: &AutoLayout, f: &mut Fields) {
        let grid = l.mode == LayoutMode::Grid;
        let mode = match l.mode {
            LayoutMode::Horizontal => "HORIZONTAL",
            LayoutMode::Vertical => "VERTICAL",
            LayoutMode::Grid => "GRID",
        };
        let [top, right, bottom, left] = l.padding;
        let main = match l.main_align {
            MainAlign::Min => "MIN",
            MainAlign::Center => "CENTER",
            MainAlign::Max => "MAX",
            MainAlign::SpaceBetween => "SPACE_BETWEEN",
            MainAlign::SpaceAround => "SPACE_AROUND",
            MainAlign::SpaceEvenly => "SPACE_EVENLY",
        };
        let cross = match l.cross_align {
            CrossAlign::Min => "MIN",
            CrossAlign::Center => "CENTER",
            CrossAlign::Max => "MAX",
            CrossAlign::Baseline => "BASELINE",
        };
        // The main axis is the width for horizontal and grid layouts.
        let (main_hug, cross_hug) = {
            let (h, v) = (n.child_layout.sizing_h == Sizing::Hug, n.child_layout.sizing_v == Sizing::Hug);
            if l.mode == LayoutMode::Vertical { (v, h) } else { (h, v) }
        };
        let size = |hug: bool| e(if hug { "RESIZE_TO_FIT" } else { "FIXED" });
        f.extend([
            ("stackMode", e(mode)),
            ("stackWrap", e(if l.wrap { "WRAP" } else { "NO_WRAP" })),
            ("stackHorizontalPadding", fl(left)),
            ("stackVerticalPadding", fl(top)),
            ("stackPaddingRight", fl(right)),
            ("stackPaddingBottom", fl(bottom)),
            ("stackPrimaryAlignItems", e(main)),
            ("stackCounterAlignItems", e(cross)),
            ("stackPrimarySizing", size(main_hug)),
            ("stackCounterSizing", size(cross_hug)),
            ("bordersTakeSpace", b(l.strokes_included)),
            ("stackReverseZIndex", b(l.reverse_z)),
        ]);
        if grid {
            // Instance layers share their main layer's track ids, so unchanged tracks and the cells
            // their layers sit in still match.
            let key = self.main_layer(n.id);
            let (want_c, want_r) = (l.grid_columns.min(1000) as usize, l.grid_rows.min(1000) as usize);
            let (cols, rows) = match self.tracks.get(&key) {
                Some((c, r)) if (c.len(), r.len()) == (want_c, want_r) => (c.clone(), r.clone()),
                _ => ((0..want_c).map(|_| self.next_aux()).collect(), (0..want_r).map(|_| self.next_aux()).collect()),
            };
            let map = |ids: &[u32]| {
                o(vec![(
                    "entries",
                    arr(ids.iter().enumerate().map(|(i, t)| o(vec![("id", guid(AUX, *t)), ("position", s(&position(i)))])).collect()),
                )])
            };
            f.extend([("gridColumnGap", fl(l.gap)), ("gridRowGap", fl(l.cross_gap)), ("gridColumns", map(&cols)), ("gridRows", map(&rows))]);
            if l.column_tracks.iter().chain(&l.row_tracks).any(|t| t.sizing != Sizing::Fill) {
                self.warn("grid tracks of fixed or content size were written as equal tracks");
            }
            self.tracks.insert(key, (cols, rows));
        } else {
            let spread = l.content_align == ContentAlign::SpaceBetween;
            f.extend([
                ("stackSpacing", fl(l.gap)),
                ("stackCounterSpacing", fl(l.cross_gap)),
                ("stackCounterAlignContent", e(if spread { "SPACE_BETWEEN" } else { "AUTO" })),
            ]);
        }
    }

    /// How a layer sizes and sits in its auto layout parent. Min/max sizes are the layer's own
    /// and are written whatever its parent is.
    fn child_layout(&mut self, n: &Node, parent: Option<&Node>, f: &mut Fields) {
        let cl = &n.child_layout;
        for (key, w, h) in [("minSize", cl.min_w, cl.min_h), ("maxSize", cl.max_w, cl.max_h)] {
            if w.is_some() || h.is_some() {
                f.push((key, o(vec![("value", xy(w.unwrap_or(0.0), h.unwrap_or(0.0)))])));
            }
        }
        let Some(pl) = parent.and_then(|p| p.layout.as_ref()) else { return };
        let (fill_h, fill_v) = (cl.sizing_h == Sizing::Fill, cl.sizing_v == Sizing::Fill);
        let (grow, stretch) = if pl.mode == LayoutMode::Vertical { (fill_v, fill_h) } else { (fill_h, fill_v) };
        if grow {
            f.push(("stackChildPrimaryGrow", fl(1.0)));
        }
        if stretch {
            f.push(("stackChildAlignSelf", e("STRETCH")));
        }
        if cl.absolute {
            f.push(("stackPositioning", e("ABSOLUTE")));
        }
        if pl.mode == LayoutMode::Grid
            && let Some((cols, rows)) = parent.and_then(|p| self.tracks.get(&self.main_layer(p.id)))
        {
            let anchor = |ids: &[u32], i: Option<u32>| i.and_then(|i| ids.get(i as usize)).map(|t| guid(AUX, *t));
            let (c, r) = (anchor(cols, cl.grid_column), anchor(rows, cl.grid_row));
            f.extend(c.map(|g| ("gridColumnAnchor", g)));
            f.extend(r.map(|g| ("gridRowAnchor", g)));
            if cl.grid_column_span > 0 {
                f.push(("gridColumnSpan", int(cl.grid_column_span.into())));
            }
            if cl.grid_row_span > 0 {
                f.push(("gridRowSpan", int(cl.grid_row_span.into())));
            }
        }
    }

    /// Text: its style, box settings, and characters with their styled runs. `fills`: the layer's
    /// (what runs without fills of their own show when the text's own style sets other fills).
    fn text(&mut self, td: &TextData, fills: &[Paint], f: &mut Fields) {
        style_fields(&td.style, f);
        let (ha, va) = (
            match td.align_h {
                TextAlignH::Left => "LEFT",
                TextAlignH::Center => "CENTER",
                TextAlignH::Right => "RIGHT",
                TextAlignH::Justified => "JUSTIFIED",
            },
            match td.align_v {
                TextAlignV::Top => "TOP",
                TextAlignV::Middle => "CENTER",
                TextAlignV::Bottom => "BOTTOM",
            },
        );
        let resize = match td.auto_resize {
            TextAutoResize::WidthAndHeight => "WIDTH_AND_HEIGHT",
            TextAutoResize::Height => "HEIGHT",
            TextAutoResize::None | TextAutoResize::Truncate => "NONE",
        };
        f.extend([
            ("textAlignHorizontal", e(ha)),
            ("textAlignVertical", e(va)),
            ("textAutoResize", e(resize)),
            ("paragraphSpacing", fl(td.paragraph_spacing)),
            ("paragraphIndent", fl(td.paragraph_indent)),
            ("autoRename", b(false)),
            ("fontVariantCommonLigatures", b(true)),
            ("fontVariantContextualLigatures", b(true)),
            ("fontVersion", s("")),
            ("textUserLayoutVersion", u(5)),
            ("textExplicitLayoutVersion", u(1)),
            ("textBidiVersion", u(1)),
            ("textTracking", fl(0.0)),
            ("textDecorationSkipInk", b(true)),
            ("emojiImageSet", e("APPLE")),
        ]);
        if td.auto_resize == TextAutoResize::Truncate {
            f.push(("textTruncation", e("ENDING")));
        }
        if td.max_lines > 0 {
            f.push(("maxLines", int(td.max_lines.into())));
        }
        // Styles per UTF-16 unit: 0 for the base style, else an entry of the override table.
        if td.char_count() > MAX_CHARS {
            self.warn("text longer than 1,048,576 characters was cut");
        }
        let spans = td.spans();
        let mut table: Vec<&TextStyle> = Vec::new();
        let mut span_ids = Vec::with_capacity(spans.len());
        for (_, _, st) in &spans {
            span_ids.push(if **st == td.style {
                0
            } else {
                let i = table.iter().position(|t| *t == *st).unwrap_or_else(|| {
                    table.push(st);
                    table.len() - 1
                });
                i as u32 + 1
            });
        }
        let mut chars = String::new();
        let mut ids: Vec<Kv> = Vec::new();
        let mut span = 0;
        for (i, ch) in td.characters.chars().take(MAX_CHARS).enumerate() {
            while spans.get(span).is_some_and(|sp| i >= sp.1) {
                span += 1;
            }
            // NULs can't be stored in a string.
            if ch == '\0' {
                continue;
            }
            chars.push(ch);
            let id = span_ids.get(span).copied().unwrap_or(0);
            ids.extend((0..ch.len_utf16()).map(|_| u(id)));
        }
        let mut data: Fields = vec![("characters", Kv::Str(chars))];
        if !table.is_empty() {
            let entries = table
                .iter()
                .zip(1u32..)
                .map(|(st, id)| {
                    let mut ef: Fields = vec![("styleID", u(id))];
                    style_fields(st, &mut ef);
                    // An empty link clears the text's own (a missing one would inherit it).
                    if st.link.is_none() && td.style.link.is_some() {
                        ef.push(("hyperlink", o(vec![("url", s(""))])));
                    }
                    if let Some(p) = st.fills.as_deref().or(td.style.fills.as_ref().map(|_| fills)) {
                        ef.push(("fillPaints", self.paints(p, true, &Bound::default())));
                    }
                    o(ef)
                })
                .collect();
            data.extend([("characterStyleIDs", arr(ids)), ("styleOverrideTable", arr(entries))]);
        }
        let lists: Vec<ListStyle> = (0..td.paragraph_count()).map(|p| td.para_list(p).list).collect();
        // One entry per paragraph, each with every field a reader expects.
        let line = |(i, l): (usize, &ListStyle)| {
            let ty = match l {
                ListStyle::None => "PLAIN",
                ListStyle::Numbered => "ORDERED_LIST",
                ListStyle::Bulleted => "UNORDERED_LIST",
            };
            let listed = *l != ListStyle::None;
            let first = listed && (i == 0 || lists.get(i - 1) != Some(l));
            o(vec![
                ("lineType", e(ty)),
                ("styleId", int(0)),
                ("indentationLevel", int(i64::from(listed))),
                ("sourceDirectionality", e("AUTO")),
                ("listStartOffset", int(0)),
                ("isFirstLineOfList", b(first)),
            ])
        };
        data.push(("lines", arr(lists.iter().enumerate().map(line).collect())));
        f.push(("textData", o(data)));
        if (0..td.paragraph_count()).any(|p| td.para_list(p).indent > 0) {
            self.warn("nested list levels were written as the first level");
        }
        let styles = std::iter::once(&td.style).chain(td.runs.iter().map(|r| &r.style));
        if styles.clone().any(|st| st.decoration_look.is_some() || !st.variations.is_empty()) {
            self.warn("text decoration looks and variable font settings were left out");
        }
        if td.path.is_some() {
            self.warn("text on a path was written as plain text");
        }
        if td.list_spacing != 0.0 || td.hanging_punctuation || td.hanging_list || td.leading_trim != Default::default() || !td.features.is_empty() {
            self.warn("list spacing, hanging punctuation, leading trim and OpenType features were left out");
        }
    }

    /// The editable network (with per-point corner radii, handle mirroring and end caps) and,
    /// when it has filled areas, their outline.
    fn vector(&mut self, n: &Node, net: &VectorNetwork, f: &mut Fields) {
        let entries: usize = net.regions.iter().flat_map(|r| &r.loops).map(Vec::len).sum();
        if net.vertices.len() + net.segments.len() + entries > MAX_NETWORK {
            self.warn("vector networks too large for the format were written as outlines");
        } else {
            // The stroke starts at the first end point and stops at the last: the layer's end caps go
            // there (where the point has none of its own).
            let mut degree = vec![0usize; net.vertices.len()];
            for sg in &net.segments {
                for i in [sg.a, sg.b] {
                    if let Some(d) = degree.get_mut(i) {
                        *d += 1;
                    }
                }
            }
            let ends: Vec<usize> = (0..degree.len()).filter(|i| degree.get(*i) == Some(&1)).collect();
            let (first, last) = (ends.first().copied(), ends.last().copied());
            let caps_differ = n.stroke.cap_start != n.stroke.cap_end || net.vertices.iter().any(|v| v.cap.is_some());
            let mut styles: Vec<Fields> = Vec::new();
            let mut style_of = |fields: Fields| -> u32 {
                if fields.is_empty() {
                    return 0;
                }
                let i = styles.iter().position(|x| *x == fields).unwrap_or_else(|| {
                    styles.push(fields);
                    styles.len() - 1
                });
                i as u32 + 1
            };
            let vertices = net
                .vertices
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let mut st: Fields = Vec::new();
                    if v.radius > 0.0 {
                        st.push(("cornerRadius", fl(v.radius)));
                    }
                    if let Some(m) = v.mirror {
                        st.push((
                            "handleMirroring",
                            e(match m {
                                figcraft_doc::HandleMirroring::None => "NONE",
                                figcraft_doc::HandleMirroring::Angle => "ANGLE",
                                figcraft_doc::HandleMirroring::AngleAndLength => "ANGLE_AND_LENGTH",
                            }),
                        ));
                    }
                    let end_cap = if Some(i) == first {
                        Some(n.stroke.cap_start)
                    } else if Some(i) == last {
                        Some(n.stroke.cap_end)
                    } else {
                        None
                    };
                    if let Some(c) = v.cap.or(end_cap.filter(|_| caps_differ)) {
                        st.push(("strokeCap", e(cap_name(c))));
                    }
                    if let Some(j) = v.join {
                        st.push(("strokeJoin", e(join_name(j))));
                    }
                    network::Vertex { style: style_of(st), x: finite(v.x), y: finite(v.y) }
                })
                .collect();
            let segments = net
                .segments
                .iter()
                .map(|sg| network::Segment { a: sg.a, ta: (finite(sg.ta.0), finite(sg.ta.1)), b: sg.b, tb: (finite(sg.tb.0), finite(sg.tb.1)) })
                .collect();
            let regions = net
                .regions
                .iter()
                .map(|r| network::Region { style: 0, non_zero: r.winding == WindingRule::NonZero, loops: r.loops.clone() })
                .collect();
            let blob = network::encode(&network::Network { vertices, segments, regions });
            let table = styles.into_iter().zip(1u32..).map(|(mut st, id)| {
                st.insert(0, ("styleID", u(id)));
                o(st)
            });
            let table: Vec<Kv> = table.collect();
            if let Some(i) = self.blob(blob) {
                f.push((
                    "vectorData",
                    o(vec![("vectorNetworkBlob", u(i)), ("normalizedSize", xy(n.width, n.height)), ("styleOverrideTable", arr(table))]),
                ));
            }
        }
        if !net.regions.is_empty() {
            let winding = net.regions.first().map(|r| r.winding).unwrap_or_default();
            if net.regions.iter().any(|r| r.winding != winding) {
                self.warn("vectors whose filled areas use different winding rules use the first one's in their outline");
            }
            self.geometry("fillGeometry", &net.fill_path(), winding, f);
        }
    }

    /// `fillGeometry` or `strokeGeometry` (`key`) from a local path.
    fn geometry(&mut self, key: &'static str, p: &BezPath, winding: WindingRule, f: &mut Fields) {
        if p.elements().is_empty() {
            return;
        }
        if let Some(i) = self.blob(path_blob(p)) {
            let rule = if winding == WindingRule::EvenOdd { "ODD" } else { "NONZERO" };
            f.push((key, arr(vec![o(vec![("windingRule", e(rule)), ("commandsBlob", u(i)), ("styleID", u(0))])])));
        }
    }

    /// `strokeGeometry`, the outline of a plain stroke: a ring around boxes and whole ellipses, the
    /// widened path of other centred strokes. Dashed, per-side and other strokes get none (a reader
    /// builds theirs from the stroke settings).
    fn stroke_geometry(&mut self, n: &Node, boxy: bool, f: &mut Fields) {
        let sp = &n.stroke;
        let (w, h, weight) = (n.width, n.height, sp.weight);
        let plain = n.strokes.iter().any(|p| p.visible) && sp.dashes.is_empty() && sp.sides.is_none() && sp.profile().is_none() && sp.brush.is_none();
        if !plain || !(weight.is_finite() && weight > 0.0 && w.is_finite() && h.is_finite()) {
            return;
        }
        let whole = matches!(n.kind, NodeKind::Ellipse { sweep, inner, .. } if inner == 0.0 && sweep.abs() >= std::f64::consts::TAU - 1e-9);
        if boxy || whole {
            let (out, inset) = match sp.align {
                StrokeAlign::Inside => (0.0, weight),
                StrokeAlign::Center => (weight / 2.0, weight / 2.0),
                StrokeAlign::Outside => (weight, 0.0),
            };
            // The shape grown by `d` (shrunk when negative), or nothing when it shrinks away.
            let grown = |d: f64| -> Option<BezPath> {
                let (gw, gh) = (w + 2.0 * d, h + 2.0 * d);
                if gw <= 0.0 || gh <= 0.0 {
                    return None;
                }
                let mut p = if whole {
                    figcraft_doc::geom::ellipse(gw, gh, 0.0, std::f64::consts::TAU, 0.0)
                } else {
                    let radii = n.corners.radii.map(|r| if r > 0.0 { (r + d).max(0.0) } else { 0.0 });
                    figcraft_doc::geom::rounded_rect(gw, gh, radii, n.corners.smoothing)
                };
                p.apply_affine(Affine::translate((-d, -d)));
                Some(p)
            };
            let Some(mut ring) = grown(out) else { return };
            ring.extend(grown(-inset).iter().flat_map(|p| p.elements().iter().copied()));
            self.geometry("strokeGeometry", &ring, WindingRule::EvenOdd, f);
        } else if sp.align == StrokeAlign::Center {
            let path = match &n.kind {
                NodeKind::Vector { network } => Some(network.stroke_path()),
                NodeKind::Text(_) => None,
                _ => figcraft_doc::geometry::shape_path(n),
            };
            let Some(path) = path.filter(|p| !p.elements().is_empty()) else { return };
            use figcraft_doc::geom::kurbo::{Cap, Join, Stroke, StrokeOpts, stroke};
            let cap = match sp.cap_start {
                StrokeCap::Round => Cap::Round,
                StrokeCap::Square => Cap::Square,
                _ => Cap::Butt,
            };
            let join = match sp.join {
                StrokeJoin::Miter => Join::Miter,
                StrokeJoin::Bevel => Join::Bevel,
                StrokeJoin::Round => Join::Round,
            };
            let limit = if sp.miter_limit.is_finite() { sp.miter_limit.max(1.0) } else { 4.0 };
            let style = Stroke::new(weight).with_caps(cap).with_join(join).with_miter_limit(limit);
            self.geometry("strokeGeometry", &stroke(path, &style, &StrokeOpts::default(), 0.05), WindingRule::NonZero, f);
        }
    }

    /// Add a blob; its index (`None` once the blobs would make the file too large).
    fn blob(&mut self, bytes: Vec<u8>) -> Option<u32> {
        if let Some(i) = self.blobs.get(&bytes) {
            return Some(*i);
        }
        self.blob_bytes += bytes.len() as u64;
        if self.blob_bytes > MAX_CANVAS {
            self.warn("vector data too large for the format was left out");
            return None;
        }
        let i = u32::try_from(self.blobs.len()).ok()?;
        self.blobs.insert(bytes, i);
        Some(i)
    }

    /// A component property definition (variant properties are implied by the variants' names).
    fn prop_def(&mut self, name: &str, d: &PropDef) -> Option<Kv<'static>> {
        let ty = prop_type(d)?;
        let id = *self.defs.get(&(name.to_string(), ty))?;
        let value = match d {
            PropDef::Boolean { default } => o(vec![("boolValue", b(*default))]),
            PropDef::Text { default } => o(vec![("textValue", o(vec![("characters", s(default))]))]),
            PropDef::InstanceSwap { default, preferred } => {
                if !preferred.is_empty() {
                    self.warn("preferred values of instance swap properties were left out");
                }
                o(vec![("guidValue", self.guid_of(*default)?)])
            }
            PropDef::Variant { .. } | PropDef::Slot { .. } => return None,
        };
        Some(o(vec![("id", guid(AUX, id)), ("name", s(name)), ("initialValue", value), ("type", e(ty))]))
    }

    /// Which property controls a layer's `field`.
    fn prop_ref(&mut self, field: &str, prop: &str) -> Option<Kv<'static>> {
        let (var_field, ty, resolved) = match field {
            "visible" => ("VISIBLE", "BOOL", "BOOLEAN"),
            "characters" => ("TEXT_DATA", "TEXT", "TEXT_DATA"),
            "mainComponent" => ("OVERRIDDEN_SYMBOL_ID", "INSTANCE_SWAP", "SYMBOL_ID"),
            _ => {
                self.warn("slot layers were written as plain layers");
                return None;
            }
        };
        let id = *self.defs.get(&(prop.to_string(), ty))?;
        let data = o(vec![
            ("value", o(vec![("propRefValue", o(vec![("defId", guid(AUX, id))]))])),
            ("dataType", e("PROP_REF")),
            ("resolvedDataType", e(resolved)),
        ]);
        Some(o(vec![("variableData", data), ("variableField", e(var_field))]))
    }

    /// An instance: its main, the fields it and its layers override, the size, place and outline
    /// its layers get from that (derived data), its property values. `f`: its own fields so far.
    fn instance(&mut self, n: &Node, main: NodeId, props: &BTreeMap<String, Value>, f: &mut Fields) {
        const OWN: [&str; 7] = ["guid", "phase", "parentIndex", "type", "transform", "fillGeometry", "strokeGeometry"];
        let doc = self.doc;
        let (Some(m), Some(symbol)) = (doc.node(main), self.guid_of(main)) else { return };
        // What the instance itself changes from its main (its size, its paints, …) is an override
        // addressed by the main's id: a reader takes everything else from the main. Where it sits
        // is its own.
        let mut root_theirs = Vec::new();
        self.layer(m, m.parent.and_then(|p| doc.node(p)), &mut root_theirs, true);
        root_theirs.retain(|(k, _)| !OWN.contains(k));
        // An instance is named after its component (the set, for a variant) until renamed.
        let set = m.parent.and_then(|p| doc.node(p)).filter(|p| matches!(p.kind, NodeKind::ComponentSet { .. }));
        let named = set.is_none_or(|s| s.name != n.name);
        let root_mine: Fields = f.iter().filter(|(k, _)| !OWN.contains(k) && (named || *k != "name")).cloned().collect();
        // Each layer below: its id path, the fields that differ from the main's layer, that layer's
        // fields, and the component a nested instance was swapped for.
        struct Row {
            path: Kv<'static>,
            changed: Fields,
            theirs: Fields,
            nested: bool,
            swap: Option<Kv<'static>>,
        }
        let mut rows: Vec<Row> = Vec::new();
        let mut stack: Vec<(NodeId, NodeId)> = n.children.iter().rev().map(|c| (*c, n.id)).collect();
        let mut seen = 0usize;
        while let Some((id, parent)) = stack.pop() {
            let Some(d) = doc.node(id) else { continue };
            seen += 1;
            if seen > MAX_NODES {
                break;
            }
            stack.extend(d.children.iter().rev().map(|c| (*c, id)));
            let path = self.layer_path(d, n.id, main, 0);
            let guids: Option<Vec<Kv>> = path.and_then(|p| p.iter().map(|x| self.guid_of(*x)).collect());
            let (Some(guids), Some(src)) = (guids, d.source.and_then(|src| doc.node(src))) else {
                self.warn("instance layers that don't come from their main component were left out");
                continue;
            };
            let (mut mine, mut theirs) = (Vec::new(), Vec::new());
            self.layer(d, doc.node(parent), &mut mine, true);
            self.layer(src, src.parent.and_then(|p| doc.node(p)), &mut theirs, true);
            // A nested instance swapped for another component.
            let swap = match (&d.kind, &src.kind) {
                (NodeKind::Instance { component, .. }, NodeKind::Instance { component: was, .. }) if component == was => None,
                (NodeKind::Instance { component, .. }, _) => self.guid_of(*component),
                _ => None,
            };
            let nested = matches!(d.kind, NodeKind::Instance { .. });
            rows.push(Row { path: o(vec![("guids", arr(guids))]), changed: changed(mine, &theirs), theirs, nested, swap });
        }
        // An instance scaled as a whole (the Scale tool) is `factor` times its main in every length:
        // sizes, places and stroke weights, its own and its layers'. A resized one keeps its weights.
        let field = |fields: &Fields, key: &str| fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone());
        let factor = Some(n.width / m.width).filter(|k| k.is_finite() && *k > 0.0 && (k - 1.0).abs() > 1e-6).filter(|k| {
            let grown = |mine: &Fields, theirs: &Fields| {
                let weight = field(theirs, "strokeWeight").and_then(|w| w.f64()).unwrap_or(0.0);
                (weight == 0.0 || field(mine, "strokeWeight").is_some_and(|w| scaled(&w, &fl(weight), *k, 0.0)))
                    && field(mine, "size").zip(field(theirs, "size")).is_some_and(|(a, b)| scaled(&a, &b, *k, 1.0))
                    && field(mine, "transform").zip(field(theirs, "transform")).is_none_or(|(a, b)| scaled(&a, &b, *k, 1.0))
            };
            (n.height / m.height - k).abs() <= 1e-6 * k.max(1.0)
                && grown(&changed(root_mine.clone(), &root_theirs), &root_theirs)
                && rows.iter().all(|r| grown(&r.changed, &r.theirs))
        });
        // What follows from the overrides goes with the derived data: a layer's size, place and
        // outline (a vector's network is the main's, drawn at the layer's size), the colours its
        // variables give here, and what the scale factor makes of the main's values.
        let follows = |k: &str, v: &Kv<'static>, theirs: &Fields| {
            ["size", "transform", "fillGeometry", "strokeGeometry", "vectorData"].contains(&k)
                || resolved_alike(k, v, theirs)
                || factor.is_some_and(|x| field(theirs, k).is_some_and(|t| scaled(v, &t, x, 0.0)))
        };
        let mut overrides = Vec::new();
        let mut derived = Vec::new();
        let mut of: Fields = vec![("guidPath", o(vec![("guids", arr(vec![symbol.clone()]))]))];
        of.extend(changed(root_mine, &root_theirs).into_iter().filter(|(k, v)| *k == "size" || !follows(k, v, &root_theirs)));
        if of.len() > 1 {
            overrides.push(o(of));
        }
        for r in rows {
            let (mut of, mut df): (Fields, Fields) = (vec![("guidPath", r.path.clone())], vec![("guidPath", r.path)]);
            for (k, v) in r.changed {
                let follows = follows(k, &v, &r.theirs);
                // A nested instance's size is its own to set.
                if follows && r.nested && k == "size" {
                    of.push((k, v.clone()));
                }
                if follows { &mut df } else { &mut of }.push((k, v));
            }
            if df.len() > 1 {
                derived.push(o(df));
            }
            of.extend(r.swap.map(|g| ("overriddenSymbolID", g)));
            if of.len() > 1 {
                overrides.push(o(of));
            }
        }
        f.push(("symbolData", o(vec![("symbolID", symbol), ("symbolOverrides", arr(overrides)), ("uniformScaleFactor", fl(factor.unwrap_or(1.0)))])));
        if !derived.is_empty() {
            f.push(("derivedSymbolData", arr(derived)));
        }
        let mut assignments = Vec::new();
        for (name, v) in props {
            let (ty, value) = match v {
                Value::Bool(x) => ("BOOL", Some(o(vec![("boolValue", b(*x))]))),
                Value::String(t) => ("TEXT", Some(o(vec![("textValue", o(vec![("characters", s(t))]))]))),
                Value::Number(c) => ("INSTANCE_SWAP", c.as_u64().and_then(|c| self.guid_of(NodeId(c))).map(|g| o(vec![("guidValue", g)]))),
                _ => continue,
            };
            // Variant values have no definition of their own: the main they show says them.
            if let (Some(&id), Some(value)) = (self.defs.get(&(name.clone(), ty)), value) {
                assignments.push(o(vec![("defID", guid(AUX, id)), ("value", value)]));
            }
        }
        if !assignments.is_empty() {
            f.push(("componentPropAssignments", arr(assignments)));
        }
        if n.children.iter().filter_map(|c| doc.node(*c)).any(|c| c.source.is_none()) {
            self.warn("instance layers that don't come from their main component were left out");
        }
    }

    /// The main component layer an instance or instance layer mirrors (through nested instances),
    /// or `id` itself.
    fn main_layer(&self, id: NodeId) -> NodeId {
        let mut cur = id;
        for _ in 0..32 {
            let next = self.doc.node(cur).and_then(|n| match &n.kind {
                _ if n.source.is_some() => n.source,
                NodeKind::Instance { component, .. } => Some(*component),
                _ => None,
            });
            match next.filter(|s| self.doc.contains(*s)) {
                Some(s) => cur = s,
                None => break,
            }
        }
        cur
    }

    /// The id path of layer `d` of instance `inst` (of `main`): that of the main's layer it mirrors,
    /// or, inside a nested instance swapped for another component, the path of that instance and
    /// then the one inside its new main.
    fn layer_path(&self, d: &Node, inst: NodeId, main: NodeId, depth: usize) -> Option<Vec<NodeId>> {
        let src = d.source?;
        if let Some(p) = self.path_in(src, main, 0) {
            return Some(p);
        }
        if depth > 32 {
            return None;
        }
        let doc = self.doc;
        let nested = doc.ancestors(d.id).into_iter().take_while(|a| *a != inst).filter_map(|a| doc.node(a)).find_map(|a| match &a.kind {
            NodeKind::Instance { component, .. } => Some((a, *component)),
            _ => None,
        });
        let (outer, component) = nested?;
        let mut p = self.layer_path(outer, inst, main, depth + 1)?;
        p.extend(self.path_in(src, component, 0)?);
        Some(p)
    }

    /// The id path of layer `s` inside component `comp`: `s` itself, or through the nested
    /// instance it is expanded in, then on inside that instance's main.
    fn path_in(&self, s: NodeId, comp: NodeId, depth: usize) -> Option<Vec<NodeId>> {
        if depth > 32 {
            return None;
        }
        let doc = self.doc;
        let up = doc.ancestors(s);
        let below: Vec<NodeId> = up.iter().take_while(|a| **a != comp).copied().collect();
        if below.len() == up.len() {
            return None;
        }
        // The outermost nested instance between the component and the layer.
        match below.iter().rev().find_map(|a| match doc.node(*a).map(|n| &n.kind) {
            Some(NodeKind::Instance { component, .. }) => Some((*a, *component)),
            _ => None,
        }) {
            None => Some(vec![s]),
            Some((inst, main)) => {
                let mut p = vec![inst];
                p.extend(self.path_in(doc.node(s)?.source?, main, depth + 1)?);
                Some(p)
            }
        }
    }

    /// Interactions, scrolling, flow starts and the overlay settings this layer is opened with.
    fn prototype(&mut self, n: &Node, f: &mut Fields) {
        let list: Vec<Kv> = n.interactions.iter().filter_map(|it| self.interaction(it)).collect();
        if !list.is_empty() {
            f.push(("prototypeInteractions", arr(list)));
        }
        let dir = match n.overflow {
            Overflow::None => None,
            Overflow::Horizontal => Some("HORIZONTAL"),
            Overflow::Vertical => Some("VERTICAL"),
            Overflow::Both => Some("BOTH"),
        };
        f.extend(dir.map(|d| ("scrollDirection", e(d))));
        let behavior = match n.scroll_position {
            ScrollPosition::Scrolls => None,
            ScrollPosition::Fixed => Some("FIXED_WHEN_CHILD_OF_SCROLLING_FRAME"),
            ScrollPosition::Sticky => Some("STICKY_SCROLLS"),
        };
        f.extend(behavior.map(|x| ("scrollBehavior", e(x))));
        if let Some((i, name)) = self.starts.get(&n.id) {
            f.push(("prototypeStartingPoint", o(vec![("name", s(name)), ("position", s(&position(*i)))])));
        }
        if let Some(ov) = self.overlays.get(&n.id) {
            const POSITIONS: [&str; 8] = ["CENTER", "TOP_LEFT", "TOP_CENTER", "TOP_RIGHT", "BOTTOM_LEFT", "BOTTOM_CENTER", "BOTTOM_RIGHT", "MANUAL"];
            let pos = POSITIONS.into_iter().find(|p| crate::rest::proto::overlay_position(p) == Some(ov.position)).unwrap_or("CENTER");
            let bg = match ov.background {
                Some(c) => o(vec![("backgroundType", e("SOLID_COLOR")), ("backgroundColor", color(c))]),
                None => o(vec![("backgroundType", e("NONE"))]),
            };
            f.extend([
                ("overlayPositionType", e(pos)),
                ("overlayBackgroundInteraction", e(if ov.close_on_outside { "CLOSE_ON_CLICK_OUTSIDE" } else { "NONE" })),
                ("overlayBackgroundAppearance", bg),
            ]);
        }
    }

    fn interaction(&mut self, it: &Interaction) -> Option<Kv<'static>> {
        let mut event: Fields = Vec::new();
        let ty = match &it.trigger {
            Trigger::OnClick => "ON_CLICK",
            Trigger::OnHover => "ON_HOVER",
            Trigger::OnPress => "ON_PRESS",
            Trigger::OnDrag => "DRAG",
            Trigger::MouseEnter => "MOUSE_IN",
            Trigger::MouseLeave => "MOUSE_OUT",
            Trigger::MouseDown => "MOUSE_DOWN",
            Trigger::MouseUp => "MOUSE_UP",
            Trigger::AfterDelay { ms } => {
                event.push(("transitionTimeout", fl(f64::from(*ms) / 1000.0)));
                "AFTER_TIMEOUT"
            }
            Trigger::KeyPress { key } => {
                let Some((codes, pad)) = key_codes(key) else {
                    self.warn("key triggers on keys the format has no code for were left out");
                    return None;
                };
                let device = e(if pad { "UNKNOWN_CONTROLLER" } else { "KEYBOARD" });
                event.push(("keyTrigger", o(vec![("keyCodes", arr(codes.into_iter().map(int).collect())), ("triggerDevice", device)])));
                "ON_KEY_DOWN"
            }
        };
        // A mouse trigger's delay is the event's timeout.
        if it.delay_secs() > 0.0 {
            event.push(("transitionTimeout", fl(it.delay_secs())));
        }
        event.insert(0, ("interactionType", e(ty)));
        let actions: Vec<Kv> = it.actions.iter().filter_map(|a| self.action(a, it)).collect();
        (!actions.is_empty()).then(|| o(vec![("event", o(event)), ("actions", arr(actions))]))
    }

    fn action(&mut self, a: &Action, it: &Interaction) -> Option<Kv<'static>> {
        let node = |x: &mut Self, dest: &NodeId, nav: &'static str| -> Option<Kv<'static>> {
            let Some(g) = x.guid_of(*dest) else {
                x.warn("prototype actions whose destination isn't written were left out");
                return None;
            };
            let mut f: Fields = vec![("transitionNodeID", g), ("connectionType", e("INTERNAL_NODE")), ("navigationType", e(nav))];
            x.transition(it, &mut f);
            Some(o(f))
        };
        let plain = |ty: &'static str| Some(o(vec![("connectionType", e(ty))]));
        match a {
            Action::Navigate { dest, preserve_scroll } => {
                let mut act = node(self, dest, "NAVIGATE")?;
                if let Kv::Object(f) = &mut act {
                    f.push(("transitionPreserveScroll", b(*preserve_scroll)));
                }
                Some(act)
            }
            Action::OpenOverlay { dest, position, offset, .. } => {
                let mut act = node(self, dest, "OVERLAY")?;
                if let (Kv::Object(f), OverlayPosition::Manual, Some([x, y])) = (&mut act, position, offset) {
                    f.push(("overlayRelativePosition", o(vec![("x", fl(*x)), ("y", fl(*y))])));
                }
                Some(act)
            }
            Action::SwapOverlay { dest } => node(self, dest, "SWAP"),
            Action::ChangeTo { dest } => node(self, dest, "SWAP_STATE"),
            Action::ScrollTo { dest, offset } => {
                if *offset != [0.0, 0.0] {
                    self.warn("scroll-to offsets were left out");
                }
                node(self, dest, "SCROLL_TO")
            }
            Action::Back => plain("BACK"),
            Action::CloseOverlay => plain("CLOSE"),
            Action::OpenUrl { url } => Some(o(vec![("connectionType", e("URL")), ("connectionURL", s(url))])),
            Action::SetVariable { .. } | Action::SetVariableMode { .. } | Action::Conditional { .. } => {
                self.warn("variable and conditional prototype actions were left out");
                None
            }
        }
    }

    /// An interaction's transition on one of its actions.
    fn transition(&mut self, it: &Interaction, f: &mut Fields) {
        let side_from = |d: Direction, l: &'static str, r: &'static str, t: &'static str, bo: &'static str| match d {
            // The direction names where the incoming screen moves: it comes from the other side.
            Direction::Right => l,
            Direction::Left => r,
            Direction::Bottom => t,
            Direction::Top => bo,
        };
        let side_to = |d: Direction, l: &'static str, r: &'static str, t: &'static str, bo: &'static str| match d {
            Direction::Left => l,
            Direction::Right => r,
            Direction::Top => t,
            Direction::Bottom => bo,
        };
        let ty = match it.transition {
            Transition::Instant => "INSTANT_TRANSITION",
            Transition::Dissolve => "DISSOLVE",
            Transition::SmartAnimate => "SMART_ANIMATE",
            Transition::SlideIn { direction } => side_from(direction, "SLIDE_FROM_LEFT", "SLIDE_FROM_RIGHT", "SLIDE_FROM_TOP", "SLIDE_FROM_BOTTOM"),
            Transition::Push { direction } => side_from(direction, "PUSH_FROM_LEFT", "PUSH_FROM_RIGHT", "PUSH_FROM_TOP", "PUSH_FROM_BOTTOM"),
            Transition::MoveIn { direction } => side_from(direction, "MOVE_FROM_LEFT", "MOVE_FROM_RIGHT", "MOVE_FROM_TOP", "MOVE_FROM_BOTTOM"),
            Transition::SlideOut { direction } => {
                side_to(direction, "SLIDE_OUT_TO_LEFT", "SLIDE_OUT_TO_RIGHT", "SLIDE_OUT_TO_TOP", "SLIDE_OUT_TO_BOTTOM")
            }
            Transition::MoveOut { direction } => side_to(direction, "MOVE_OUT_TO_LEFT", "MOVE_OUT_TO_RIGHT", "MOVE_OUT_TO_TOP", "MOVE_OUT_TO_BOTTOM"),
        };
        let easing = match it.easing {
            Easing::Linear => "LINEAR",
            Easing::EaseIn => "IN_CUBIC",
            Easing::EaseOut => "OUT_CUBIC",
            Easing::EaseInOut => "INOUT_CUBIC",
            Easing::EaseInBack => "IN_BACK_CUBIC",
            Easing::EaseOutBack => "OUT_BACK_CUBIC",
            Easing::EaseInOutBack => "INOUT_BACK_CUBIC",
            Easing::Gentle => "GENTLE_SPRING",
            Easing::CustomBezier { .. } => "CUSTOM_CUBIC",
            Easing::Quick | Easing::Bouncy | Easing::Slow | Easing::CustomSpring { .. } => {
                self.warn("spring animations other than Gentle were written as ease out");
                "OUT_CUBIC"
            }
        };
        f.extend([("transitionType", e(ty)), ("transitionDuration", fl(f64::from(it.duration_ms) / 1000.0)), ("easingType", e(easing))]);
        if let Easing::CustomBezier { x1, y1, x2, y2 } = it.easing {
            f.push(("easingFunction", arr(vec![fl(x1), fl(y1), fl(x2), fl(y2)])));
        }
    }
}

/// The plugin id and key under which a layer written as an outline keeps what it was.
pub(super) const STASH: (&str, &str) = ("figcraft", "stroke");
/// Longest kept description of such a layer (bytes of JSON).
const MAX_STASH: usize = 1 << 20;

/// Whether a width profile reaches further on one side of the path than on the other.
fn one_sided(p: &figcraft_doc::WidthProfile) -> bool {
    p.points.iter().any(|q| q.bias.abs() > 1e-6)
}

/// A shape whose stroke is wider on one side of its path, as the area that stroke paints (a vector
/// filled with the stroke's paints), with what it was as JSON: its kind, paints and stroke. Only
/// for shapes that show nothing but that stroke; `None` for every other layer.
fn outlined(n: &Node) -> Option<(Node, String)> {
    let sp = &n.stroke;
    let shape = matches!(
        n.kind,
        NodeKind::Line | NodeKind::Vector { .. } | NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::Rectangle
    );
    let shown = |paints: &[Paint]| paints.iter().any(|p| p.visible);
    if !shape
        || sp.brush.is_some()
        || !sp.profile().is_some_and(one_sided)
        || !(sp.weight.is_finite() && sp.weight > 0.0)
        || !shown(&n.strokes)
        || shown(&n.fills)
    {
        return None;
    }
    let path = figcraft_doc::geometry::stroke_geometry(n)?;
    let net = match &n.kind {
        NodeKind::Vector { network } => Some(network),
        _ => None,
    };
    let area = figcraft_pathops::stroke_outline_of(&path, sp, figcraft_doc::geometry::is_closed(n), net);
    if area.elements().is_empty() || area.elements().len() > MAX_NETWORK / 4 {
        return None;
    }
    let original = serde_json::json!({"kind": n.kind, "fills": n.fills, "strokes": n.strokes, "stroke": n.stroke}).to_string();
    if original.len() > MAX_STASH {
        return None;
    }
    let mut network = VectorNetwork::from_path(&area);
    for r in &mut network.regions {
        r.winding = WindingRule::NonZero;
    }
    let mut v = n.clone();
    v.kind = NodeKind::Vector { network };
    v.fills = std::mem::take(&mut v.strokes);
    v.stroke = Default::default();
    v.styles.fill = v.styles.stroke.take();
    // Bindings name the stroke's paints, which are the fills now: the values stay.
    v.bound.retain(|k, _| !(k == "stroke" || k == "fill" || k.starts_with("strokes/") || k.starts_with("fills/") || k.starts_with("stroke")));
    Some((v, original))
}

/// The number of the layer field a bound property (`VariableField`) is held in.
fn node_field(field: &str) -> Option<u32> {
    let name = match field {
        "WIDTH" | "HEIGHT" => "size",
        "MIN_WIDTH" | "MIN_HEIGHT" => "minSize",
        "MAX_WIDTH" | "MAX_HEIGHT" => "maxSize",
        "STACK_SPACING" => "stackSpacing",
        "STACK_COUNTER_SPACING" => "stackCounterSpacing",
        "STACK_PADDING_LEFT" => "stackHorizontalPadding",
        "STACK_PADDING_TOP" => "stackVerticalPadding",
        "STACK_PADDING_RIGHT" => "stackPaddingRight",
        "STACK_PADDING_BOTTOM" => "stackPaddingBottom",
        "CORNER_RADIUS" => "cornerRadius",
        "RECTANGLE_TOP_LEFT_CORNER_RADIUS" => "rectangleTopLeftCornerRadius",
        "RECTANGLE_TOP_RIGHT_CORNER_RADIUS" => "rectangleTopRightCornerRadius",
        "RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS" => "rectangleBottomLeftCornerRadius",
        "RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS" => "rectangleBottomRightCornerRadius",
        "OPACITY" => "opacity",
        "VISIBLE" => "visible",
        "TEXT_DATA" => "textData",
        "STROKE_WEIGHT" => "strokeWeight",
        "BORDER_TOP_WEIGHT" => "borderTopWeight",
        "BORDER_RIGHT_WEIGHT" => "borderRightWeight",
        "BORDER_BOTTOM_WEIGHT" => "borderBottomWeight",
        "BORDER_LEFT_WEIGHT" => "borderLeftWeight",
        "FONT_SIZE" => "fontSize",
        "FONT_FAMILY" | "FONT_STYLE" => "fontName",
        "LETTER_SPACING" => "letterSpacing",
        "LINE_HEIGHT" => "lineHeight",
        "PARAGRAPH_SPACING" => "paragraphSpacing",
        "PARAGRAPH_INDENT" => "paragraphIndent",
        _ => return None,
    };
    // The numbers are the schema's.
    let layer = SCHEMA.lines().find(|l| l.starts_with("message NodeChange "))?;
    layer.split(", ").find_map(|f| f.rsplit_once(' ')?.1.strip_prefix(name)?.strip_prefix('=')?.parse().ok())
}

/// Whether `a` is `b` with its lengths scaled by `k`: every number is `k` times the other's (within
/// `slack`, for sizes that snap to whole pixels) or the same, and the rest is equal.
fn scaled(a: &Kv<'static>, b: &Kv<'static>, k: f64, slack: f64) -> bool {
    match (a, b) {
        (Kv::Float(x), Kv::Float(y)) => {
            let (x, y) = (f64::from(*x), f64::from(*y));
            x == y || (x - y * k).abs() <= slack + 1e-3 * x.abs().max(1.0)
        }
        (Kv::Array(x), Kv::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| scaled(p, q, k, slack)),
        (Kv::Object(x), Kv::Object(y)) => x.len() == y.len() && x.iter().zip(y).all(|((kp, p), (kq, q))| kp == kq && scaled(p, q, k, slack)),
        _ => a == b,
    }
}

/// The fields of `mine` that differ from `theirs` (a layer and the main's layer it mirrors), and
/// the ones only `theirs` has at their empty values.
fn changed(mine: Fields, theirs: &Fields) -> Fields {
    let cleared: Fields = theirs.iter().filter(|(k, _)| !mine.iter().any(|(mk, _)| mk == k)).filter_map(|(k, _)| Some((*k, unset(k)?))).collect();
    let mut out: Fields = mine.into_iter().filter(|(k, v)| !theirs.iter().any(|(tk, tv)| tk == k && tv == v)).collect();
    out.extend(cleared);
    out
}

/// Whether paint list `v` (field `key`) is `theirs` but for the colours variables give: two layers
/// bound alike are the same, whatever mode each one shows.
fn resolved_alike(key: &str, v: &Kv<'static>, theirs: &Fields) -> bool {
    fn unresolved(v: &Kv<'static>) -> Vec<Kv<'static>> {
        let bare = |p: &Kv<'static>| match p {
            Kv::Object(f) if f.iter().any(|(k, _)| *k == "colorVar") => o(f.iter().filter(|(k, _)| *k != "color").cloned().collect()),
            other => other.clone(),
        };
        v.list().iter().map(bare).collect()
    }
    matches!(key, "fillPaints" | "strokePaints") && theirs.iter().any(|(k, t)| *k == key && unresolved(t) == unresolved(v))
}

/// The format's name of a variable type.
fn resolved_type(kind: VariableType) -> &'static str {
    match kind {
        VariableType::Color => "COLOR",
        VariableType::Number => "FLOAT",
        VariableType::String => "STRING",
        VariableType::Boolean => "BOOLEAN",
    }
}

/// The format's name of a variable scope (`None`: it has none).
fn scope_name(scope: &str) -> Option<&'static str> {
    const NAMES: [&str; 20] = [
        "ALL_SCOPES",
        "TEXT_CONTENT",
        "CORNER_RADIUS",
        "WIDTH_HEIGHT",
        "GAP",
        "ALL_FILLS",
        "FRAME_FILL",
        "SHAPE_FILL",
        "TEXT_FILL",
        "STROKE_FLOAT",
        "EFFECT_FLOAT",
        "EFFECT_COLOR",
        "OPACITY",
        "FONT_STYLE",
        "FONT_FAMILY",
        "FONT_SIZE",
        "LINE_HEIGHT",
        "LETTER_SPACING",
        "PARAGRAPH_SPACING",
        "PARAGRAPH_INDENT",
    ];
    if scope == "STROKE_COLOR" {
        return Some("STROKE");
    }
    NAMES.into_iter().find(|n| *n == scope)
}

/// The empty value of a field only written when set (an override clears it with this).
fn unset(key: &str) -> Option<Kv<'static>> {
    Some(match key {
        "styleIdForFill" | "styleIdForStrokeFill" | "styleIdForEffect" | "styleIdForGrid" | "styleIdForText" => {
            o(vec![("guid", guid(u32::MAX, u32::MAX))])
        }
        "variableConsumptionMap" | "variableModeBySetMap" => o(vec![("entries", arr(Vec::new()))]),
        "mask" | "rectangleCornerRadiiIndependent" | "borderStrokeWeightsIndependent" => b(false),
        "effects" | "dashPattern" | "layoutGrids" | "exportSettings" | "prototypeInteractions" | "fillGeometry" | "variableWidthPoints" => {
            arr(Vec::new())
        }
        "cornerSmoothing" | "stackChildPrimaryGrow" => fl(0.0),
        "stackMode" => e("NONE"),
        "stackChildAlignSelf" | "stackPositioning" => e("AUTO"),
        "minSize" | "maxSize" => o(vec![("value", xy(0.0, 0.0))]),
        "gridColumnSpan" | "gridRowSpan" | "maxLines" => int(0),
        "scrollDirection" => e("NONE"),
        "scrollBehavior" => e("SCROLLS"),
        "parameterConsumptionMap" => o(vec![("entries", arr(Vec::new()))]),
        "textTruncation" => e("DISABLED"),
        "hyperlink" => o(vec![("url", s(""))]),
        _ => return None,
    })
}

/// The format's type of a component property (`None`: not written).
fn prop_type(d: &PropDef) -> Option<&'static str> {
    match d {
        PropDef::Boolean { .. } => Some("BOOL"),
        PropDef::Text { .. } => Some("TEXT"),
        PropDef::InstanceSwap { .. } => Some("INSTANCE_SWAP"),
        PropDef::Variant { .. } | PropDef::Slot { .. } => None,
    }
}

/// The key codes of a key trigger (modifiers first), and whether it's a controller button.
fn key_codes(key: &str) -> Option<(Vec<i64>, bool)> {
    if let Some(i) = GAMEPAD_BUTTONS.iter().position(|g| g.eq_ignore_ascii_case(key.trim())) {
        return Some((vec![i as i64], true));
    }
    let (mods, k): (KeyMods, &str) = KeyMods::split(key);
    let code = (0..=255).find(|c| crate::rest::proto::key_name(f64::from(*c)).is_some_and(|n| n.eq_ignore_ascii_case(k)))?;
    let mut codes: Vec<i64> = [(mods.shift, 16), (mods.ctrl, 17), (mods.alt, 18), (mods.cmd, 91)].into_iter().filter(|m| m.0).map(|m| m.1).collect();
    codes.push(i64::from(code));
    Some((codes, false))
}

/// The text style fields of a layer or of an override table entry.
fn style_fields(st: &TextStyle, f: &mut Fields) {
    let line_height = match st.line_height {
        // Percent units count from the font's own line height (100 % is "auto"); a share of the
        // font size is a raw multiplier.
        LineHeight::Auto => o(vec![("value", fl(100.0)), ("units", e("PERCENT"))]),
        LineHeight::Percent(p) => o(vec![("value", fl(p / 100.0)), ("units", e("RAW"))]),
        LineHeight::Pixels(v) => o(vec![("value", fl(v)), ("units", e("PIXELS"))]),
    };
    let letter_spacing = match st.letter_spacing {
        LetterSpacing::Percent(p) => o(vec![("value", fl(p)), ("units", e("PERCENT"))]),
        LetterSpacing::Pixels(v) => o(vec![("value", fl(v)), ("units", e("PIXELS"))]),
    };
    let case = match st.case {
        TextCase::Original => "ORIGINAL",
        TextCase::Upper => "UPPER",
        TextCase::Lower => "LOWER",
        TextCase::Title => "TITLE",
        TextCase::SmallCaps => "SMALL_CAPS",
    };
    let decoration = match st.decoration {
        TextDecoration::None => "NONE",
        TextDecoration::Underline => "UNDERLINE",
        TextDecoration::Strikethrough => "STRIKETHROUGH",
    };
    f.extend([
        ("fontName", o(vec![("family", s(&st.font_family)), ("style", s(&super::style_name(&st.font_style))), ("postscript", s(""))])),
        ("fontSize", fl(st.font_size)),
        ("lineHeight", line_height),
        ("letterSpacing", letter_spacing),
        ("textCase", e(case)),
        ("textDecoration", e(decoration)),
    ]);
    if let Some(url) = &st.link {
        f.push(("hyperlink", o(vec![("url", s(url))])));
    }
}

/// Layout grid `index` of a layer, with the variables `bound` to its fields.
fn layout_grid(g: &figcraft_doc::LayoutGrid, index: usize, bound: &Bound) -> Kv<'static> {
    let (pattern, axis) = match g.kind {
        GridKind::Grid => ("GRID", "X"),
        GridKind::Columns => ("STRIPES", "X"),
        GridKind::Rows => ("STRIPES", "Y"),
    };
    let align = match g.align {
        GridAlign::Stretch => "STRETCH",
        GridAlign::Min => "MIN",
        GridAlign::Center => "CENTER",
        GridAlign::Max => "MAX",
    };
    let mut f: Fields = vec![
        ("type", e(align)),
        ("axis", e(axis)),
        ("visible", b(g.visible)),
        ("numSections", int(g.count.into())),
        ("offset", fl(g.offset)),
        ("sectionSize", fl(g.size)),
        ("gutterSize", fl(g.gutter)),
        ("color", color(g.color)),
        ("pattern", e(pattern)),
    ];
    let vars = [("count", "numSectionsVar"), ("offset", "offsetVar"), ("size", "sectionSizeVar"), ("gutter", "gutterSizeVar")];
    f.extend(vars.into_iter().filter_map(|(ours, key)| Some((key, bound.grids.get(&(index, ours))?.clone()))));
    o(f)
}

fn export_setting(x: &figcraft_doc::ExportSetting) -> Kv<'static> {
    let format = match x.format {
        // The format has no WebP export setting.
        figcraft_doc::ExportFormat::Png | figcraft_doc::ExportFormat::Webp => "PNG",
        figcraft_doc::ExportFormat::Jpg => "JPEG",
        figcraft_doc::ExportFormat::Svg => "SVG",
        figcraft_doc::ExportFormat::Pdf => "PDF",
    };
    let (ty, value) = match x.size {
        ExportSize::Scale { value } => ("CONTENT_SCALE", value),
        ExportSize::Width { value } => ("CONTENT_WIDTH", value),
        ExportSize::Height { value } => ("CONTENT_HEIGHT", value),
    };
    o(vec![("suffix", s(&x.suffix)), ("imageType", e(format)), ("constraint", o(vec![("type", e(ty)), ("value", fl(value))]))])
}
