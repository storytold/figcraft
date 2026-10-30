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
    AutoLayout, BoolOp, ContentAlign, CrossAlign, Document, EffectKind, ExportSize, GridAlign, GridKind, GridTrack, HConstraint, LayoutMode,
    LetterSpacing, LineHeight, ListStyle, MainAlign, MaskType, Node, NodeId, NodeKind, NoiseMode, PropDef, Sizing, StrokeAlign, StrokeCap,
    StrokeJoin, StyleKind, TextAlignH, TextAlignV, TextAutoResize, TextCase, TextData, TextDecoration, TextStyle, VConstraint, VariableType,
    VariableValue, VectorNetwork, WindingRule,
};
use serde_json::Value;
use sha1::Digest;

use super::kiwi::{Kv, Schema, encode};
use super::network;
use crate::FormatError;

mod assets;
mod common;
mod component;
mod layer;
mod paint;
mod prototype;
mod shape;

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
enum StackJustify MIN=0 CENTER=1 MAX=2 SPACE_EVENLY=3 SPACE_BETWEEN=4 SPACE_AROUND=5 SPACE_EVENLY_CSS=6
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
enum VariableDataType BOOLEAN=0 FLOAT=1 STRING=2 ALIAS=3 COLOR=4 SYMBOL_ID=7 TEXT_DATA=9 PROP_REF=13
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
message ComponentPropPreferredValues string[] stringValues=1
message ComponentPropDef GUID id=1, string name=2, ComponentPropValue initialValue=3, string sortPosition=4, GUID parentPropDefId=5, ComponentPropType type=6, ComponentPropPreferredValues preferredValues=8, VariableData varValue=9
message ComponentPropAssignment GUID defID=1, ComponentPropValue value=2, VariableData varValue=3
enum ComponentPropNodeField VISIBLE=0 TEXT_DATA=1 OVERRIDDEN_SYMBOL_ID=2
message ComponentPropRef GUID defID=2, ComponentPropNodeField componentPropNodeField=4
message VariantPropSpec GUID propDefId=1, string value=2
message StateGroupPropertyValueOrder string property=1, string[] values=2
message PropRefValue GUID defId=1
message SymbolId GUID guid=1
message VariableAnyValue bool boolValue=1, string textValue=2, float floatValue=3, VariableID alias=4, Color colorValue=5, SymbolId symbolIdValue=8, TextData textDataValue=10, PropRefValue propRefValue=13
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
enum GridTrackSizingType FLEX=0 FIXED=1 HUG=2 UNKNOWN=3
message GridTrackSizingFunction GridTrackSizingType type=1, float value=2
message GridTrackSize GridTrackSizingFunction minSizing=1, GridTrackSizingFunction maxSizing=2
message GUIDGridTrackSizeMapEntry GUID id=1, GridTrackSize trackSize=2
message GUIDGridTrackSizeMap GUIDGridTrackSizeMapEntry[] entries=1
message NodeChange GUID guid=1, NodePhase phase=2, ParentIndex parentIndex=3, NodeType type=4, string name=5, bool visible=6, bool locked=7, float opacity=8, BlendMode blendMode=9, Vector size=11, Matrix transform=12, bool mask=16, MaskType maskType=317, float backgroundOpacity=19, bool backgroundEnabled=15, bool internalOnly=142, string description=318, StyleType styleType=163, string sortPosition=320, StyleId styleIdForFill=332, StyleId styleIdForStrokeFill=333, StyleId styleIdForText=334, StyleId styleIdForEffect=335, StyleId styleIdForGrid=336, VariableDataMap variableConsumptionMap=307, VariableModeBySetMap variableModeBySetMap=316, VariableSetMode[] variableSetModes=312, VariableSetID variableSetID=313, VariableResolvedDataType variableResolvedType=314, VariableDataValues variableDataValues=315, VariableScope[] variableScopes=353, DocumentColorProfile documentColorProfile=349, Paint[] fillPaints=38, Paint[] strokePaints=39, float strokeWeight=26, StrokeAlign strokeAlign=29, StrokeJoin strokeJoin=31, StrokeCap strokeCap=30, float[] dashPattern=13, float miterLimit=25, bool borderStrokeWeightsIndependent=299, float borderTopWeight=295, float borderRightWeight=298, float borderBottomWeight=296, float borderLeftWeight=297, Effect[] effects=43, float cornerRadius=20, float cornerSmoothing=160, bool rectangleCornerRadiiIndependent=149, float rectangleTopLeftCornerRadius=145, float rectangleTopRightCornerRadius=146, float rectangleBottomRightCornerRadius=148, float rectangleBottomLeftCornerRadius=147, ConstraintType horizontalConstraint=28, ConstraintType verticalConstraint=37, bool frameMaskDisabled=115, bool resizeToFit=117, bool isStateGroup=225, StateGroupPropertyValueOrder[] stateGroupPropertyValueOrders=238, bool sectionContentsHidden=283, Color backgroundColor=50, StackMode stackMode=105, float stackSpacing=107, float stackCounterSpacing=324, StackWrap stackWrap=323, StackCounterAlignContent stackCounterAlignContent=343, float stackHorizontalPadding=209, float stackVerticalPadding=210, float stackPaddingRight=233, float stackPaddingBottom=234, StackJustify stackPrimaryAlignItems=230, StackAlign stackCounterAlignItems=231, StackSize stackPrimarySizing=229, StackSize stackCounterSizing=221, bool bordersTakeSpace=294, bool stackReverseZIndex=271, float gridColumnGap=438, float gridRowGap=437, GUIDPositionMap gridColumns=436, GUIDPositionMap gridRows=435, GUIDGridTrackSizeMap gridColumnsSizing=474, GUIDGridTrackSizeMap gridRowsSizing=475, float stackChildPrimaryGrow=232, StackCounterAlign stackChildAlignSelf=236, StackPositioning stackPositioning=269, OptionalVector minSize=325, OptionalVector maxSize=326, GUID gridColumnAnchor=440, GUID gridRowAnchor=439, uint gridColumnSpan=442, uint gridRowSpan=441, LayoutGrid[] layoutGrids=47, ExportSettings[] exportSettings=45, TextData textData=42, FontName fontName=41, float fontSize=21, Number lineHeight=40, Number letterSpacing=165, TextCase textCase=34, TextDecoration textDecoration=35, TextAlignHorizontal textAlignHorizontal=32, TextAlignVertical textAlignVertical=33, TextAutoResize textAutoResize=46, TextTruncation textTruncation=280, int maxLines=351, float paragraphSpacing=23, float paragraphIndent=22, Hyperlink hyperlink=223, bool autoRename=14, bool fontVariantCommonLigatures=127, bool fontVariantContextualLigatures=128, string fontVersion=202, uint textUserLayoutVersion=203, uint textExplicitLayoutVersion=396, uint textBidiVersion=279, float textTracking=27, bool textDecorationSkipInk=412, EmojiImageSet emojiImageSet=391, uint styleID=49, Path[] fillGeometry=51, Path[] strokeGeometry=52, VectorData vectorData=48, VectorMirror handleMirroring=44, uint count=10, float starInnerScale=24, ArcData arcData=195, BooleanOperation booleanOperation=36, string symbolDescription=144, VariableWidthPoint[] variableWidthPoints=447, PluginData[] pluginData=204, SymbolData symbolData=113, NodeChange[] derivedSymbolData=125, GUID overriddenSymbolID=143, ComponentPropDef[] componentPropDefs=266, ComponentPropRef[] componentPropRefs=267, ComponentPropAssignment[] componentPropAssignments=268, VariantPropSpec[] variantPropSpecs=483, VariableDataMap parameterConsumptionMap=445, GUIDPath guidPath=111, PrototypeInteraction[] prototypeInteractions=226, PrototypeStartingPoint prototypeStartingPoint=249, ScrollDirection scrollDirection=159, ScrollBehavior scrollBehavior=186, OverlayPositionType overlayPositionType=198, OverlayBackgroundInteraction overlayBackgroundInteraction=200, OverlayBackgroundAppearance overlayBackgroundAppearance=201
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
pub(super) fn canvas(schema: &[u8], message: &[u8]) -> Result<Vec<u8>, FormatError> {
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

/// A track's size as the format's enum name and value, when both sizing functions agree (the
/// importer keeps only those). `None`: the format has no name for it, so an equal track stands in.
fn grid_track_size(track: &GridTrack) -> Option<(&'static str, f64)> {
    let value = track.value;
    let fits = value.is_finite() && (0.0..=crate::fig::map::MAX_GRID_TRACK_VALUE).contains(&value);
    match track.sizing {
        Sizing::Fixed if fits => Some(("FIXED", value)),
        Sizing::Fill if fits && value > 0.0 => Some(("FLEX", value)),
        Sizing::Hug => Some(("HUG", 0.0)),
        _ => None,
    }
}

/// A grid axis's track sizes, keyed by track id like the order map. A track the document doesn't
/// set is left out, and one the format can't hold keeps its id (the import equalises it) and sets
/// `dropped`.
fn grid_sizing(tracks: &[GridTrack], ids: &[u32], dropped: &mut bool) -> Option<Kv<'static>> {
    let mut entries = Vec::new();
    for (track, id) in tracks.iter().zip(ids) {
        match grid_track_size(track) {
            Some((kind, value)) => {
                let size = || o(vec![("type", e(kind)), ("value", fl(value))]);
                entries.push(o(vec![("id", guid(AUX, *id)), ("trackSize", o(vec![("minSizing", size()), ("maxSizing", size())]))]));
            }
            None => *dropped = true,
        }
    }
    (!entries.is_empty()).then(|| o(vec![("entries", arr(entries))]))
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
    /// Component property (owner, name, type) → local id of its definition's GUID (session [`AUX`]).
    /// The owner is the component, or the set for a variant (variant properties included, type
    /// `VARIANT`).
    defs: BTreeMap<(NodeId, String, &'static str), u32>,
    /// A variant's own definition of each of its set's properties (variant, name, type) → local
    /// id (session [`AUX`]): what its layers and its instances refer to.
    child_defs: BTreeMap<(NodeId, String, &'static str), u32>,
    /// Grid tracks of written grid frames: (columns, rows) local ids (session [`AUX`]).
    tracks: BTreeMap<NodeId, (Vec<u32>, Vec<u32>)>,
    aux: u32,
    overlays: BTreeMap<NodeId, Overlay>,
    trigger_size: [f64; 2],
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
            child_defs: BTreeMap::new(),
            tracks: BTreeMap::new(),
            aux: 0,
            overlays: BTreeMap::new(),
            trigger_size: [0.0, 0.0],
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

    /// The component set component `id` is a variant of.
    fn set_of(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.doc.node(id)?.parent.and_then(|p| self.doc.node(p))?;
        matches!(parent.kind, NodeKind::ComponentSet { .. }).then_some(parent.id)
    }

    /// The local id a layer of component `comp` (or an instance of it) uses for its property
    /// `name` of type `ty`: a variant's own definition, else the component's.
    fn def_for(&self, comp: NodeId, name: &str, ty: &'static str) -> Option<u32> {
        let key = (comp, name.to_string(), ty);
        let defs = if self.set_of(comp).is_some() { &self.child_defs } else { &self.defs };
        defs.get(&key).copied()
    }

    /// The main component a layer belongs to (itself when it is one). A layer inside an instance
    /// mirrors its main's layer (`source`, through nested instances): that layer's component, so
    /// the instance's layers refer to what the main's do. An instance placed in a component is the
    /// component's own layer (its `source` is its main, which it doesn't belong to).
    fn component_of(&self, n: &Node) -> Option<NodeId> {
        let in_instance = |x: &Node| {
            let mut p = x.parent.and_then(|p| self.doc.node(p));
            for _ in 0..MAX_DEPTH {
                match p {
                    Some(a) if matches!(a.kind, NodeKind::Instance { .. }) => return true,
                    Some(a) => p = a.parent.and_then(|q| self.doc.node(q)),
                    None => return false,
                }
            }
            false
        };
        let mut n = n;
        for _ in 0..MAX_DEPTH {
            match n.source.and_then(|s| self.doc.node(s)) {
                Some(src) if in_instance(n) => n = src,
                _ => break,
            }
        }
        let mut cur = Some(n);
        for _ in 0..MAX_DEPTH {
            let c = cur?;
            if matches!(c.kind, NodeKind::Component { .. }) {
                return Some(c.id);
            }
            cur = c.parent.and_then(|p| self.doc.node(p));
        }
        None
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
}

/// The plugin id and key under which a layer written as an outline keeps what it was.
pub(super) const STASH: (&str, &str) = ("figcraft", "stroke");

/// An image's name in the file: the SHA-1 of its bytes (lowercase hex). Paints refer to images by
/// these 20 bytes, and readers reject other hash lengths (the document's own SHA-256 key is not
/// used here).
pub(super) fn sha1_hex(bytes: &[u8]) -> String {
    sha1::Sha1::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A style name as readers look the face up: the standard name ([`super::style_name`]) with the
/// compound weights spelt the way most font families name their faces ("SemiBold", "ExtraBold",
/// "ExtraLight Italic"), except Inter, whose faces are named with a space ("Semi Bold"). A name
/// the family doesn't have reads as a missing font, and its text isn't drawn.
pub(super) fn face_style(family: &str, style: &str) -> String {
    let standard = super::style_name(style);
    if family == "Inter" || family.starts_with("Inter ") {
        return standard;
    }
    ["Extra", "Ultra", "Semi", "Demi"].iter().fold(standard, |acc, p| acc.replace(&format!("{p} "), p))
}
