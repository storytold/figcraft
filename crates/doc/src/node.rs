//! Nodes: one struct with the properties every node can have, and a kind with per-type data.

use crate::NodeId;
use crate::props::{
    AutoLayout, ChildLayout, Constraints, Corners, Effect, EffectKind, ExportSetting, LayoutGrid, SPREAD_MITER_LIMIT, StrokeAlign, StrokeCap,
    StrokeProps,
};
use crate::proto::{Interaction, Overflow, ScrollPosition};
use crate::styles::{CollectionId, ModeId, StyleRefs, VariableId};
use crate::text::TextData;
use crate::vector::VectorNetwork;
use figcraft_color::{BlendMode, Color, Paint};
use figcraft_geom::Affine;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Boolean operation of a boolean group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BoolOp {
    #[default]
    Union,
    Subtract,
    Intersect,
    Exclude,
}

impl BoolOp {
    pub fn label(self) -> &'static str {
        match self {
            BoolOp::Union => "Union",
            BoolOp::Subtract => "Subtract",
            BoolOp::Intersect => "Intersect",
            BoolOp::Exclude => "Exclude",
        }
    }
}

/// A component property definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PropDef {
    Boolean {
        default: bool,
    },
    Text {
        default: String,
    },
    /// A nested instance's main; `preferred` components (or sets) are offered first when
    /// swapping it.
    InstanceSwap {
        default: NodeId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        preferred: Vec<NodeId>,
    },
    Variant {
        options: Vec<String>,
        default: String,
    },
    /// A frame (bound through `prop_refs["slot"]`) whose instances take any content of their own;
    /// `preferred` components are offered first when filling it.
    Slot {
        #[serde(default)]
        preferred: Vec<NodeId>,
    },
}

/// What a node is, with per-type data.
// Text sets the size; the lint counts the gap to the next largest variant, which grew only because
// Page's guides moved to the node (#1476).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NodeKind {
    Page {
        #[serde(default = "page_bg")]
        background: Color,
        #[serde(default)]
        flows: Vec<crate::proto::Flow>,
        #[serde(default)]
        prototype: crate::proto::PrototypeSettings,
    },
    Frame,
    Group,
    Section {
        #[serde(default)]
        contents_hidden: bool,
    },
    Rectangle,
    Ellipse {
        #[serde(default)]
        start: f64,
        #[serde(default = "tau")]
        sweep: f64,
        #[serde(default)]
        inner: f64,
    },
    Polygon {
        count: u32,
    },
    Star {
        count: u32,
        inner_ratio: f64,
    },
    Line,
    Vector {
        network: VectorNetwork,
    },
    Text(TextData),
    BooleanOp {
        op: BoolOp,
    },
    Component {
        #[serde(default)]
        description: String,
        #[serde(default)]
        props: BTreeMap<String, PropDef>,
    },
    ComponentSet {
        #[serde(default)]
        description: String,
    },
    Instance {
        component: NodeId,
        /// Values for the component's properties.
        #[serde(default)]
        props: BTreeMap<String, serde_json::Value>,
    },
    Slice,
}

fn page_bg() -> Color {
    Color::DEFAULT_PAGE
}
fn tau() -> f64 {
    std::f64::consts::TAU
}

/// How a mask layer masks the layers above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MaskType {
    /// The mask's rendered alpha (soft edges, gradients, opacity).
    #[default]
    Alpha,
    /// The mask's outline geometry, fully opaque (fills and opacity don't matter).
    Vector,
    /// The mask's brightness: white shows, black hides.
    Luminance,
}

impl MaskType {
    pub const ALL: [MaskType; 3] = [MaskType::Alpha, MaskType::Vector, MaskType::Luminance];
    pub fn is_alpha(&self) -> bool {
        *self == MaskType::Alpha
    }
    pub fn id(self) -> &'static str {
        match self {
            MaskType::Alpha => "alpha",
            MaskType::Vector => "vector",
            MaskType::Luminance => "luminance",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            MaskType::Alpha => "Alpha",
            MaskType::Vector => "Vector",
            MaskType::Luminance => "Luminance",
        }
    }
}

/// Handoff status of a frame or section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DevStatus {
    #[default]
    None,
    ReadyForDev,
    Completed,
}

impl DevStatus {
    pub const ALL: [DevStatus; 3] = [DevStatus::None, DevStatus::ReadyForDev, DevStatus::Completed];
    pub fn is_none(&self) -> bool {
        *self == DevStatus::None
    }
    pub fn id(self) -> &'static str {
        match self {
            DevStatus::None => "none",
            DevStatus::ReadyForDev => "readyForDev",
            DevStatus::Completed => "completed",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            DevStatus::None => "No status",
            DevStatus::ReadyForDev => "Ready for dev",
            DevStatus::Completed => "Completed",
        }
    }
}

/// A note for developers on a layer (a Dev Mode annotation): its text and the properties it pins,
/// which Dev Mode shows with their current values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Pinned property ids ("width", "fill", "fontSize"…; `figcraft_engine::annotations` lists
    /// them). Unknown ids are kept and ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<String>,
    /// The id of its category ([`AnnotationCategory`]; `None`: uncategorised). An id the file
    /// doesn't define reads as uncategorised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// A main component (or component set) linked to the code component it is built as, in one
/// language: Dev Mode shows the snippet its template makes instead of generated code
/// (`figcraft_engine::code_connect`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeLink {
    /// `react`, `leptos`, `vue`, `swiftui`, `compose`, `arkui` or `flutter`.
    pub language: String,
    /// What the snippet imports from (`@acme/ui`, a Swift module, a Kotlin name).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub import_path: String,
    /// The code component's name.
    pub name: String,
    /// The snippet with `{property}` and `{property ? "a" : "b"}` placeholders.
    pub template: String,
}

/// A dev resource: a link for developers attached to a layer. The URL is data only: it is shown
/// and opened when someone clicks it, never fetched.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevResource {
    pub url: String,
    pub name: String,
}

/// A measurement kept on the canvas for developers, stored on the layer it starts from: to another
/// layer (`to`), or across one of the layer's own sides (`side`). Dev Mode draws it from the layers'
/// current bounds, so it follows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measurement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<crate::Side>,
}

/// A kind of annotation (Development, Accessibility…): its stable id, name and the colour its
/// callouts are drawn in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnotationCategory {
    pub id: String,
    pub name: String,
    pub color: Color,
}

impl AnnotationCategory {
    /// The colours offered for categories (the defaults take the first four).
    pub const COLORS: [Color; 8] = [
        Color::from_rgb8_const(0x2F, 0x6F, 0xEB),
        Color::from_rgb8_const(0x8A, 0x4F, 0xE0),
        Color::from_rgb8_const(0x1E, 0x9E, 0x6A),
        Color::from_rgb8_const(0xD9, 0x7A, 0x06),
        Color::from_rgb8_const(0xD6, 0x3A, 0x3A),
        Color::from_rgb8_const(0xD1, 0x3F, 0x9B),
        Color::from_rgb8_const(0x11, 0x92, 0xA8),
        Color::from_rgb8_const(0x6B, 0x72, 0x80),
    ];

    pub fn new(id: &str, name: &str, color: Color) -> Self {
        Self { id: id.to_string(), name: name.to_string(), color }
    }

    /// The categories a file starts with.
    pub fn defaults() -> Vec<AnnotationCategory> {
        let [dev, interaction, a11y, content, ..] = Self::COLORS;
        vec![
            Self::new("development", "Development", dev),
            Self::new("interaction", "Interaction", interaction),
            Self::new("accessibility", "Accessibility", a11y),
            Self::new("content", "Content", content),
        ]
    }
}

/// A ruler guide on a page or frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    /// `true` for a vertical guide (x position), `false` for horizontal (y).
    pub vertical: bool,
    pub offset: f64,
}

impl NodeKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            NodeKind::Page { .. } => "PAGE",
            NodeKind::Frame => "FRAME",
            NodeKind::Group => "GROUP",
            NodeKind::Section { .. } => "SECTION",
            NodeKind::Rectangle => "RECTANGLE",
            NodeKind::Ellipse { .. } => "ELLIPSE",
            NodeKind::Polygon { .. } => "POLYGON",
            NodeKind::Star { .. } => "STAR",
            NodeKind::Line => "LINE",
            NodeKind::Vector { .. } => "VECTOR",
            NodeKind::Text(_) => "TEXT",
            NodeKind::BooleanOp { .. } => "BOOLEAN_OPERATION",
            NodeKind::Component { .. } => "COMPONENT",
            NodeKind::ComponentSet { .. } => "COMPONENT_SET",
            NodeKind::Instance { .. } => "INSTANCE",
            NodeKind::Slice => "SLICE",
        }
    }

    /// The label the design panel's type dropdown shows.
    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::Page { .. } => "Page",
            NodeKind::Frame => "Frame",
            NodeKind::Group => "Group",
            NodeKind::Section { .. } => "Section",
            NodeKind::Rectangle => "Rectangle",
            NodeKind::Ellipse { .. } => "Ellipse",
            NodeKind::Polygon { .. } => "Polygon",
            NodeKind::Star { .. } => "Star",
            NodeKind::Line => "Line",
            NodeKind::Vector { .. } => "Vector",
            NodeKind::Text(_) => "Text",
            NodeKind::BooleanOp { op } => op.label(),
            NodeKind::Component { .. } => "Component",
            NodeKind::ComponentSet { .. } => "Component set",
            NodeKind::Instance { .. } => "Instance",
            NodeKind::Slice => "Slice",
        }
    }

    /// Frame-like nodes: can have auto layout, clip content, layout grids, and act as artboards.
    pub fn is_frame_like(&self) -> bool {
        matches!(self, NodeKind::Frame | NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. })
    }

    /// Nodes that may contain children.
    pub fn is_container(&self) -> bool {
        matches!(
            self,
            NodeKind::Page { .. }
                | NodeKind::Frame
                | NodeKind::Group
                | NodeKind::Section { .. }
                | NodeKind::BooleanOp { .. }
                | NodeKind::Component { .. }
                | NodeKind::ComponentSet { .. }
                | NodeKind::Instance { .. }
        )
    }

    /// Group-like containers whose bounds follow their children.
    pub fn bounds_follow_children(&self) -> bool {
        matches!(self, NodeKind::Group | NodeKind::BooleanOp { .. })
    }

    /// Shapes with a geometric outline of their own.
    pub fn is_shape(&self) -> bool {
        matches!(
            self,
            NodeKind::Rectangle
                | NodeKind::Ellipse { .. }
                | NodeKind::Polygon { .. }
                | NodeKind::Star { .. }
                | NodeKind::Line
                | NodeKind::Vector { .. }
        )
    }

    /// Kinds whose corners can be rounded from the panel.
    pub fn has_corner_radius(&self) -> bool {
        matches!(
            self,
            NodeKind::Rectangle
                | NodeKind::Frame
                | NodeKind::Component { .. }
                | NodeKind::ComponentSet { .. }
                | NodeKind::Instance { .. }
                | NodeKind::Polygon { .. }
                | NodeKind::Star { .. }
                | NodeKind::Vector { .. }
                | NodeKind::Section { .. }
        )
    }

    /// Kinds with a radius per corner (the others round every corner alike).
    pub fn has_independent_corners(&self) -> bool {
        self.has_corner_radius() && !matches!(self, NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::Vector { .. })
    }

    /// Kinds whose rounded corners are drawn with corner smoothing (a vector's rounded points are drawn without it).
    pub fn has_corner_smoothing(&self) -> bool {
        self.has_corner_radius() && !matches!(self, NodeKind::Vector { .. })
    }

    pub fn text(&self) -> Option<&TextData> {
        match self {
            NodeKind::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn text_mut(&mut self) -> Option<&mut TextData> {
        match self {
            NodeKind::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// One node of the scene graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    #[serde(flatten)]
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<NodeId>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub children: Vec<NodeId>,
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub locked: bool,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f32,
    #[serde(default = "pass", skip_serializing_if = "is_pass")]
    pub blend: BlendMode,
    /// Transform relative to the parent (the page for top-level nodes).
    #[serde(default = "identity", skip_serializing_if = "is_identity")]
    pub transform: Affine,
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub fills: Vec<Paint>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub strokes: Vec<Paint>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub stroke: StrokeProps,
    #[serde(default, skip_serializing_if = "is_default")]
    pub effects: Vec<Effect>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub corners: Corners,
    #[serde(default, skip_serializing_if = "is_default")]
    pub constraints: Constraints,
    /// Auto layout (frame-like nodes only).
    #[serde(default, skip_serializing_if = "is_default")]
    pub layout: Option<AutoLayout>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub child_layout: ChildLayout,
    /// Clip content (frames).
    #[serde(default, skip_serializing_if = "is_default")]
    pub clip: bool,
    /// This node masks the siblings above it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub is_mask: bool,
    /// How a mask masks: by its alpha, its outline, or its luminance.
    #[serde(default, skip_serializing_if = "MaskType::is_alpha")]
    pub mask_type: MaskType,
    #[serde(default, skip_serializing_if = "is_default")]
    pub grids: Vec<LayoutGrid>,
    /// A page's or frame's ruler guides, in its own coordinates (see [`guides_key`]).
    #[serde(flatten, with = "guides_key")]
    pub guides: Vec<Guide>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub exports: Vec<ExportSetting>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub interactions: Vec<Interaction>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub overflow: Overflow,
    #[serde(default, skip_serializing_if = "is_default")]
    pub scroll_position: ScrollPosition,
    #[serde(default, skip_serializing_if = "is_default")]
    pub styles: StyleRefs,
    /// Variable bindings: field name ("fills/0/color", "width", "cornerRadius", "characters", …) → id.
    #[serde(default, skip_serializing_if = "is_default")]
    pub bound: BTreeMap<String, VariableId>,
    /// Explicit variable modes (collection → mode) set on this container.
    #[serde(default, skip_serializing_if = "is_default")]
    pub var_modes: BTreeMap<CollectionId, ModeId>,
    /// In a boolean group: how this operand combines with the operands below it (`None`: the
    /// group's operation). Ignored for the bottom operand and outside boolean groups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bool_op: Option<BoolOp>,
    /// Lock aspect ratio when resizing from the panel.
    #[serde(default, skip_serializing_if = "is_default")]
    pub constrain_proportions: bool,
    /// Inside an instance: the node of the main component this one mirrors.
    #[serde(default, skip_serializing_if = "is_default")]
    pub source: Option<NodeId>,
    /// A nested instance inside a main component whose properties show on the main's instances.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exposed: bool,

    /// Handoff status of a frame or section (shown as a badge next to its name).
    #[serde(default, skip_serializing_if = "DevStatus::is_none")]
    pub dev_status: DevStatus,
    /// Ready for dev, but its contents changed since it was marked (shown as "Changed" until the
    /// change is resolved or the layer is marked again).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dev_changed: bool,
    /// Why the last change to a ready-for-dev layer was made (entered when resolving it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dev_note: String,
    /// Inside an instance: property groups overridden locally ("fills", "characters", …; "slot":
    /// this slot holds the instance's own content).
    #[serde(default, skip_serializing_if = "is_default")]
    pub overrides: BTreeSet<String>,
    /// Component property references: which property controls this node's
    /// "visible" (boolean), "characters" (text) or "mainComponent" (instance swap), or which slot
    /// property this frame is ("slot").
    #[serde(default, skip_serializing_if = "is_default")]
    pub prop_refs: BTreeMap<String, String>,
    /// Free-form plugin / agent data.
    #[serde(default, skip_serializing_if = "is_default")]
    pub plugin_data: BTreeMap<String, String>,
    /// Dev Mode annotations, in the order they were added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Main components and component sets: the code components they are linked to, one per
    /// language.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code_links: Vec<CodeLink>,
    /// Dev resources: links for developers (docs, code, tickets…), in the order they were added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dev_resources: Vec<DevResource>,
    /// Dev Mode measurements that start from this layer, in the order they were added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measurements: Vec<Measurement>,
    /// On a board: what this object is (a sticky note, a connector, a stamp…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<crate::BoardItem>,
}

pub(crate) fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}
fn pass() -> BlendMode {
    BlendMode::PassThrough
}
fn identity() -> Affine {
    Affine::IDENTITY
}

// Fields at their default value are left out of the JSON (the file is several times smaller and
// faster to write and read); reading fills them back in from the same defaults.

/// Whether `v` is its type's default (what `#[serde(default)]` reads back for a missing field).
pub(crate) fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
fn is_yes(v: &bool) -> bool {
    *v == yes()
}
fn is_one(v: &f32) -> bool {
    *v == one()
}
fn is_pass(v: &BlendMode) -> bool {
    *v == pass()
}
fn is_identity(v: &Affine) -> bool {
    *v == identity()
}

/// A layer's `guides` key: left out when there are none, and every `guides` list the layer has is
/// read. Pages used to keep their guides in [`NodeKind::Page`] while the layer field took the
/// `guides` key on reading, so a reopened page lost its guides, and saving it then wrote two
/// `guides` keys, which a plain field refuses (#1476). Both lists are kept, in file order.
mod guides_key {
    use super::Guide;
    use serde::de::{IgnoredAny, MapAccess, Visitor};
    use serde::ser::SerializeMap;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::fmt;

    const KEY: &str = "guides";

    /// Write `guides` as the layer's `guides` key, or nothing when there are none.
    pub fn serialize<S: Serializer>(guides: &[Guide], s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        if !guides.is_empty() {
            map.serialize_entry(KEY, guides)?;
        }
        map.end()
    }

    /// Read every `guides` list among the layer's fields, joined in file order.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Guide>, D::Error> {
        d.deserialize_map(Lists)
    }

    /// Reads the `guides` lists of a layer's fields and skips the other fields.
    struct Lists;

    impl<'de> Visitor<'de> for Lists {
        type Value = Vec<Guide>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a layer's fields")
        }
        /// Every `guides` value joined, in order; a malformed one is an error.
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vec<Guide>, A::Error> {
            let mut guides = Vec::new();
            while let Some(key) = map.next_key::<Key>()? {
                if key.0 {
                    guides.extend(map.next_value::<Vec<Guide>>()?);
                } else {
                    map.next_value::<IgnoredAny>()?;
                }
            }
            Ok(guides)
        }
    }

    /// Whether a field name is `guides` (without copying the name).
    struct Key(bool);

    impl<'de> Deserialize<'de> for Key {
        /// Compare a field name with `guides` as it is read.
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Name;
            impl Visitor<'_> for Name {
                type Value = Key;
                fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                    f.write_str("a field name")
                }
                /// `guides` or another field.
                fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Key, E> {
                    Ok(Key(s == KEY))
                }
            }
            d.deserialize_identifier(Name)
        }
    }
}

impl Node {
    /// A node with Figma's defaults for its kind.
    pub fn new(id: NodeId, kind: NodeKind, name: impl Into<String>, width: f64, height: f64) -> Self {
        let fills = match &kind {
            NodeKind::Frame | NodeKind::Component { .. } => vec![Paint::solid(Color::WHITE)],
            NodeKind::Section { .. } => vec![Paint::solid(Color::WHITE)],
            NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::Vector { .. } => {
                vec![Paint::solid(Color::DEFAULT_SHAPE)]
            }
            NodeKind::Text(_) => vec![Paint::solid(Color::BLACK)],
            _ => vec![],
        };
        let strokes = match &kind {
            NodeKind::Line => vec![Paint::solid(Color::BLACK)],
            NodeKind::ComponentSet { .. } => vec![Paint::solid(Color::from_rgb8_const(0x97, 0x47, 0xFF))],
            _ => vec![],
        };
        let stroke = match &kind {
            NodeKind::Line | NodeKind::Vector { .. } => StrokeProps { align: StrokeAlign::Center, ..Default::default() },
            NodeKind::ComponentSet { .. } => StrokeProps { dashes: vec![10.0, 5.0], ..Default::default() },
            _ => StrokeProps::default(),
        };
        let blend = if kind.is_container() { BlendMode::PassThrough } else { BlendMode::Normal };
        let clip = matches!(kind, NodeKind::Frame | NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. });
        let corners = match &kind {
            NodeKind::ComponentSet { .. } => Corners { radii: [5.0; 4], smoothing: 0.0 },
            _ => Corners::default(),
        };
        Self {
            id,
            name: name.into(),
            kind,
            parent: None,
            children: Vec::new(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend,
            transform: Affine::IDENTITY,
            width: width.max(0.0),
            height: height.max(0.0),
            fills,
            strokes,
            stroke,
            effects: Vec::new(),
            corners,
            constraints: Constraints::default(),
            layout: None,
            child_layout: ChildLayout::default(),
            clip,
            is_mask: false,
            mask_type: MaskType::Alpha,
            grids: Vec::new(),
            guides: Vec::new(),
            exports: Vec::new(),
            interactions: Vec::new(),
            overflow: Default::default(),
            scroll_position: Default::default(),
            styles: StyleRefs::default(),
            bound: BTreeMap::new(),
            var_modes: BTreeMap::new(),
            bool_op: None,
            constrain_proportions: false,
            source: None,
            exposed: false,

            dev_status: DevStatus::None,
            dev_changed: false,
            dev_note: String::new(),
            overrides: BTreeSet::new(),
            prop_refs: BTreeMap::new(),
            plugin_data: BTreeMap::new(),
            annotations: Vec::new(),
            code_links: Vec::new(),
            dev_resources: Vec::new(),
            measurements: Vec::new(),
            board: None,
        }
    }

    /// A new section with the look sections are created with (white, a light grey border, slightly
    /// rounded, contents unclipped).
    pub fn new_section(id: NodeId, name: impl Into<String>, width: f64, height: f64) -> Self {
        let mut n = Self::new(id, NodeKind::Section { contents_hidden: false }, name, width, height);
        n.strokes = vec![Paint::solid(Color::from_rgb8_const(0xE6, 0xE6, 0xE6))];
        n.corners.radii = [2.0; 4];
        n.clip = false;
        n
    }

    pub fn size(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    pub fn local_rect(&self) -> figcraft_geom::Rect {
        figcraft_geom::Rect::new(0.0, 0.0, self.width, self.height)
    }

    pub fn is_page(&self) -> bool {
        matches!(self.kind, NodeKind::Page { .. })
    }

    /// A board object selected and moved as a whole (a sticky, a shape with text, a stamp…).
    pub fn is_board_atom(&self) -> bool {
        self.board.as_ref().is_some_and(crate::BoardItem::is_atom)
    }

    /// A section whose contents are hidden: only its name and outline show; its children are
    /// skipped when rendering and hit testing.
    pub fn hides_contents(&self) -> bool {
        matches!(self.kind, NodeKind::Section { contents_hidden: true })
    }

    /// Whether `self` and `other` differ only in handoff status and whether a section's contents
    /// are hidden (neither counts as a change to the design).
    pub fn same_design(&self, other: &Node) -> bool {
        if self == other {
            return true;
        }
        let plain = |n: &Node| {
            let mut n = n.clone();
            n.dev_status = DevStatus::None;
            n.dev_changed = false;
            n.dev_note.clear();
            if let NodeKind::Section { contents_hidden } = &mut n.kind {
                *contents_hidden = false;
            }
            n
        };
        plain(self) == plain(other)
    }

    /// The ruler guides on this page or frame, in its own coordinates.
    pub fn guides(&self) -> &[Guide] {
        &self.guides
    }

    /// The ruler guides on this page or frame, to edit.
    pub fn guides_mut(&mut self) -> &mut Vec<Guide> {
        &mut self.guides
    }

    /// Children in paint order, bottom to top: the child list, reversed in an auto layout frame
    /// whose first child paints on top (canvas stacking; the layer order stays as it is).
    pub fn paint_order(&self) -> impl DoubleEndedIterator<Item = NodeId> + '_ {
        let rev = self.layout.as_ref().is_some_and(|l| l.reverse_z);
        let n = self.children.len();
        (0..n).filter_map(move |i| self.children.get(if rev { n - 1 - i } else { i }).copied())
    }

    /// Whether this group or frame blends its children among themselves before the result meets
    /// what is behind it: set to Normal rather than pass-through (with children to blend).
    pub fn isolates_children(&self) -> bool {
        self.blend == BlendMode::Normal && self.kind.is_container() && !matches!(self.kind, NodeKind::BooleanOp { .. }) && !self.children.is_empty()
    }

    /// Whether painted strokes are present.
    pub fn has_visible_stroke(&self) -> bool {
        self.strokes.iter().any(|p| p.visible) && self.stroke.weight > 0.0
    }

    /// Maximum distance (in local units) painting can reach outside the node box.
    pub fn paint_reach(&self) -> f64 {
        let fx = self.effects.iter().map(Effect::reach).fold(0.0, f64::max);
        self.outline_reach() + fx + self.spread_miter_reach()
    }

    /// The page-space area this node paints itself under `world` (its children aside): its box
    /// grown by its strokes and glyphs, then by its effects (see [`Node::grow_by_effects`]).
    pub fn own_paint_bounds(&self, world: Affine) -> figcraft_geom::Rect {
        let o = self.outline_reach();
        self.grow_by_effects(figcraft_geom::transform_rect_bbox(world, self.local_rect().inflate(o, o)))
    }

    /// Page-space area `b` grown by what this node's visible effects paint around it: blurs all
    /// round, drop shadows towards their offset (in page units, as the renderer draws them), so a
    /// shadow falling down adds room below and none above.
    pub fn grow_by_effects(&self, b: figcraft_geom::Rect) -> figcraft_geom::Rect {
        let miter = self.spread_miter_reach();
        self.effects.iter().filter(|e| e.visible).fold(b, |r, e| match &e.kind {
            EffectKind::DropShadow(s) => {
                let g = s.radius.abs() * 1.5 + s.spread.max(0.0) + miter;
                let (dx, dy) = s.offset;
                if g.is_finite() && dx.is_finite() && dy.is_finite() { r.union(b.inflate(g, g) + figcraft_geom::kurbo::Vec2::new(dx, dy)) } else { r }
            }
            _ => {
                let g = e.reach();
                if g.is_finite() && g > 0.0 { r.union(b.inflate(g, g)) } else { r }
            }
        })
    }

    /// How far (local units) strokes and glyphs reach outside the box.
    fn outline_reach(&self) -> f64 {
        let stroke = if self.has_visible_stroke() {
            let w = self.stroke.sides.map(|s| s.iter().copied().fold(0.0, f64::max)).unwrap_or_else(|| self.stroke.max_weight());
            let base = match self.stroke.align {
                StrokeAlign::Inside => 0.0,
                StrokeAlign::Center => w / 2.0,
                StrokeAlign::Outside => w,
            };
            // Arrowheads are sized by the weight, whatever a width profile does.
            let point_arrows =
                matches!(&self.kind, NodeKind::Vector { network } if network.vertices.iter().any(|v| v.cap.is_some_and(StrokeCap::is_arrow)));
            let caps =
                if self.stroke.cap_start.is_arrow() || self.stroke.cap_end.is_arrow() || point_arrows { w.max(self.stroke.weight) * 3.0 } else { w };
            // Right-angled or smooth outlines (boxes, ellipses) end their mitres on the grown box.
            let boxy = matches!(self.kind, NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Section { .. }) || self.kind.is_frame_like();
            let miter = if boxy { 1.0 } else { self.stroke.miter_limit.clamp(1.0, 4.0) };
            base.max(if matches!(self.kind, NodeKind::Line | NodeKind::Vector { .. }) { caps } else { 0.0 }) * miter
        } else {
            0.0
        };
        let glyphs = if let NodeKind::Text(t) = &self.kind { t.path_reach() + t.trim_reach() } else { 0.0 };
        stroke + glyphs
    }

    /// How much further than its spread a drop shadow can reach at the outline's sharp corners:
    /// spread moves the edges out in parallel and they meet in a mitre (see
    /// [`SPREAD_MITER_LIMIT`]). Rectangles, frames and ellipses grow by exactly the spread.
    fn spread_miter_reach(&self) -> f64 {
        if matches!(self.kind, NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Section { .. }) || self.kind.is_frame_like() {
            return 0.0;
        }
        self.effects
            .iter()
            .filter(|e| e.visible)
            .filter_map(|e| match &e.kind {
                EffectKind::DropShadow(s) if s.spread.is_finite() => Some(s.spread.max(0.0) * (SPREAD_MITER_LIMIT - 1.0)),
                _ => None,
            })
            .fold(0.0, f64::max)
    }
}
