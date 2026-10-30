//! Changes: the unit of synchronisation. A document is a set of objects (nodes, and document-level
//! data such as styles, variables and comments) with properties; a [`Change`] creates, writes or
//! deletes one of them, and a batch of changes is one user action.

use figcraft_doc::{Node, NodeId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Error;

/// A property value. Messages carry it as JSON: the JSON form of a message shows it as is, the
/// binary form as compact JSON text (the document types use tagged and flattened serde
/// representations that only self-describing formats can read back).
#[derive(Clone, Debug, PartialEq)]
pub struct Json(pub serde_json::Value);

impl Json {
    /// The JSON value of `v`.
    pub fn of<T: Serialize + ?Sized>(v: &T) -> Result<Json, Error> {
        serde_json::to_value(v).map(Json).map_err(|e| Error::Encode(e.to_string()))
    }

    /// Read the value as a `T` (what [`apply`](crate::Replica::apply) does to check a value's type).
    pub fn read<'a, T: Deserialize<'a>>(&'a self, what: &str) -> Result<T, Error> {
        T::deserialize(&self.0).map_err(|e| Error::Invalid(format!("{what}: {e}")))
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            self.0.serialize(s)
        } else {
            let text = serde_json::to_string(&self.0).map_err(serde::ser::Error::custom)?;
            s.serialize_str(&text)
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Text;
        impl serde::de::Visitor<'_> for Text {
            type Value = Json;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON text")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Json, E> {
                // serde_json caps nesting depth, so hostile input can't overflow the stack.
                serde_json::from_str(v).map(Json).map_err(E::custom)
            }
        }
        if d.is_human_readable() { serde_json::Value::deserialize(d).map(Json) } else { d.deserialize_str(Text) }
    }
}

/// One change to the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Change {
    /// A new node (or one brought back, e.g. by undo): its type (the JSON of
    /// [`NodeKind`](figcraft_doc::NodeKind)), place in the tree (pages have no parent) and every
    /// property that differs from a new node of that type.
    Create { id: NodeId, kind: Json, parent: Option<NodeId>, position: String, props: Vec<(Prop, Json)> },
    /// Property `prop` of node `id` is now `value`.
    Set { id: NodeId, prop: Prop, value: Json },
    /// Delete a node and everything inside it (deleting one that is already gone does nothing).
    Delete { id: NodeId },
    /// A document-level property is now `value`.
    SetDoc { prop: DocProp, value: Json },
    /// A document-level entry (a style, comment, variable…) is now `value` (added or replaced).
    Put { entry: Entry, value: Json },
    /// Remove a document-level entry (removing one that is already gone does nothing).
    Remove { entry: Entry },
}

impl Change {
    /// The node this change is about, if any.
    pub fn node(&self) -> Option<NodeId> {
        match self {
            Change::Create { id, .. } | Change::Set { id, .. } | Change::Delete { id } => Some(*id),
            _ => None,
        }
    }
}

macro_rules! node_props {
    ($($variant:ident => $field:ident),* $(,)?) => {
        /// A node property: the tree position (`parent`, `position`) or one saved field of
        /// [`Node`]. List-valued fields (fills, text runs, vector networks…) are one property each,
        /// replaced whole (three-way merges line the lists whose items stand on their own, such as
        /// fills, up item by item, see [`merge`](crate::merge)). New properties go at the end (the
        /// binary form numbers them).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub enum Prop {
            /// The parent node (`null` for pages).
            Parent,
            /// The fractional position key among the parent's children ([`frac`](crate::frac)).
            Position,
            $($variant),*
        }

        impl Prop {
            /// The properties that are plain fields of [`Node`] (everything but the tree position).
            pub const FIELDS: &'static [Prop] = &[$(Prop::$variant),*];

            /// Whether field property `self` differs between `a` and `b` (tree properties: false).
            pub(crate) fn differs(self, a: &Node, b: &Node) -> bool {
                match self {
                    Prop::Parent | Prop::Position => false,
                    $(Prop::$variant => a.$field != b.$field),*
                }
            }

            /// Field property `self` of `n`.
            pub(crate) fn get(self, n: &Node) -> Result<Json, Error> {
                match self {
                    Prop::Parent | Prop::Position => Err(Error::Invalid(format!("{self:?} is not a field"))),
                    $(Prop::$variant => Json::of(&n.$field)),*
                }
            }

            /// Write field property `self` of `n` (unchanged when `v` has the wrong type).
            pub(crate) fn set(self, n: &mut Node, v: &Json) -> Result<(), Error> {
                match self {
                    Prop::Parent | Prop::Position => return Err(Error::Invalid(format!("{self:?} is not a field"))),
                    $(Prop::$variant => n.$field = v.read(stringify!($field))?),*
                }
                Ok(())
            }
        }

        /// Fails to build when `Node` gains a field that isn't listed as a property above.
        #[allow(dead_code)]
        fn every_node_field_is_a_property(n: &Node) {
            let Node { id: _, parent: _, children: _, $($field: _),* } = n;
        }
    };
}

node_props! {
    Name => name,
    Kind => kind,
    Visible => visible,
    Locked => locked,
    Opacity => opacity,
    Blend => blend,
    Transform => transform,
    Width => width,
    Height => height,
    Fills => fills,
    Strokes => strokes,
    Stroke => stroke,
    Effects => effects,
    Corners => corners,
    Constraints => constraints,
    Layout => layout,
    ChildLayout => child_layout,
    Clip => clip,
    IsMask => is_mask,
    MaskType => mask_type,
    Grids => grids,
    Guides => guides,
    Exports => exports,
    Interactions => interactions,
    Overflow => overflow,
    ScrollPosition => scroll_position,
    Styles => styles,
    Bound => bound,
    VarModes => var_modes,
    BoolOp => bool_op,
    ConstrainProportions => constrain_proportions,
    Source => source,
    Exposed => exposed,
    DevStatus => dev_status,
    DevChanged => dev_changed,
    DevNote => dev_note,
    Overrides => overrides,
    PropRefs => prop_refs,
    PluginData => plugin_data,
    Annotations => annotations,
    CodeLinks => code_links,
    DevResources => dev_resources,
    Measurements => measurements,
    Board => board,
}

/// A document-level property. Lists of entries with ids (styles, comments, variable collections,
/// variables) sync per entry ([`Entry`]) plus their order; the whole-list properties are the
/// fallback for lists whose ids aren't unique.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DocProp {
    /// The file name.
    Name,
    /// The page shown when the file opens.
    CurrentPage,
    /// Colours saved as document colours.
    DocumentColors,
    /// The colour space colour values are in.
    ColorProfile,
    /// Saved image adjustment presets.
    ImagePresets,
    /// Annotation categories (`null`: the defaults).
    AnnotationCategories,
    /// Enabled libraries and the assets copied from them (whole).
    Libraries,
    /// Every style, in order (fallback).
    Styles,
    /// Style ids in panel order.
    StyleOrder,
    /// Every comment thread, in order (fallback).
    Comments,
    /// Comment ids in order.
    CommentOrder,
    /// Every variable collection, in order (fallback).
    Collections,
    /// Collection ids in order.
    CollectionOrder,
    /// Every variable, in order (fallback).
    Variables,
    /// Variable ids in order.
    VariableOrder,
    /// A design file or a board.
    Kind,
    /// A board's countdown (`null`: none).
    Timer,
    /// A board's vote (`null`: none); the ballots are [`Entry::Ballot`]s.
    Vote,
}

/// A document-level entry, by id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Entry {
    /// A style (value: [`Style`](figcraft_doc::Style)).
    Style(String),
    /// A comment thread (value: [`Comment`](figcraft_doc::Comment)).
    Comment(u64),
    /// A variable collection (value: [`Collection`](figcraft_doc::Collection)).
    Collection(String),
    /// A variable (value: [`Variable`](figcraft_doc::Variable)).
    Variable(String),
    /// What a ready-for-dev layer looked like when marked (value:
    /// [`DevSnapshot`](figcraft_doc::DevSnapshot)).
    DevSnapshot(NodeId),
    /// An image by content hash (value: `{width, height, bytes}` with the encoded bytes in base64).
    /// A key with the shape of a content hash (64 hex chars) must be exactly the lowercase sha256
    /// hex `figcraft_format::image_hash` produces for the bytes; every key must be a plain
    /// archive member name ([`figcraft_doc::ImageData::is_valid_key`]), and dimensions and byte
    /// size are bounded as the entry applies (`image_of` in `docprops.rs`).
    Image(String),
    /// A voter's ballot in a board's vote (value: the layer ids voted on).
    Ballot(String),
}

/// One synced value a change writes: a node property, a document-level property or a
/// document-level entry. Per-user undo tracks what a batch wrote by target.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    Node(NodeId, Prop),
    Doc(DocProp),
    Entry(Entry),
}

impl Target {
    /// What `change` writes (`None` for creations and deletions, which change whole nodes).
    pub fn of(change: &Change) -> Option<Target> {
        match change {
            Change::Set { id, prop, .. } => Some(Target::Node(*id, *prop)),
            Change::SetDoc { prop, .. } => Some(Target::Doc(*prop)),
            Change::Put { entry, .. } | Change::Remove { entry } => Some(Target::Entry(entry.clone())),
            Change::Create { .. } | Change::Delete { .. } => None,
        }
    }

    /// The change that writes `value` here (`None`: an entry that isn't there, or no value for a
    /// property, which writes nothing).
    pub fn write(&self, value: Option<&Json>) -> Option<Change> {
        match (self, value) {
            (Target::Node(id, prop), Some(v)) => Some(Change::Set { id: *id, prop: *prop, value: v.clone() }),
            (Target::Doc(prop), Some(v)) => Some(Change::SetDoc { prop: *prop, value: v.clone() }),
            (Target::Entry(entry), Some(v)) => Some(Change::Put { entry: entry.clone(), value: v.clone() }),
            (Target::Entry(entry), None) => Some(Change::Remove { entry: entry.clone() }),
            (Target::Node(..) | Target::Doc(_), None) => None,
        }
    }
}

/// A document-level list that syncs per entry and as a whole (see [`DocProp`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Family {
    Styles,
    Comments,
    Collections,
    Variables,
}

impl DocProp {
    /// The list this property is the whole of (`Ok`) or the order of (`Err`).
    pub(crate) fn list(self) -> Option<Result<Family, Family>> {
        Some(match self {
            DocProp::Styles => Ok(Family::Styles),
            DocProp::StyleOrder => Err(Family::Styles),
            DocProp::Comments => Ok(Family::Comments),
            DocProp::CommentOrder => Err(Family::Comments),
            DocProp::Collections => Ok(Family::Collections),
            DocProp::CollectionOrder => Err(Family::Collections),
            DocProp::Variables => Ok(Family::Variables),
            DocProp::VariableOrder => Err(Family::Variables),
            _ => return None,
        })
    }
}

impl Change {
    /// Whether this puts or removes an image entry.
    pub(crate) fn is_image(&self) -> bool {
        matches!(self, Change::Put { entry: Entry::Image(_), .. } | Change::Remove { entry: Entry::Image(_) })
    }

    /// Whether this only changes comment threads (a thread, their order, or all of them): what a
    /// comment link may send.
    pub fn is_comment(&self) -> bool {
        match self {
            Change::Put { entry, .. } | Change::Remove { entry } => entry.list() == Some(Family::Comments),
            Change::SetDoc { prop, .. } => matches!(prop.list(), Some(Ok(Family::Comments) | Err(Family::Comments))),
            Change::Create { .. } | Change::Set { .. } | Change::Delete { .. } => false,
        }
    }
}

impl Entry {
    /// The client id space the entry's id lies in ([`figcraft_doc::ids`]): ids made during a live
    /// session carry their creator's client id, ids from before and ids of other forms are in
    /// space 0. `None` for entries keyed by something else (dev snapshots by layer, images by
    /// content).
    pub fn id_space(&self) -> Option<u32> {
        use figcraft_doc::ids::{COLLECTION_ID_PREFIX, STYLE_ID_PREFIX, VARIABLE_ID_PREFIX, id_space, space_of};
        match self {
            Entry::Style(id) => Some(id_space(id, STYLE_ID_PREFIX)),
            Entry::Variable(id) => Some(id_space(id, VARIABLE_ID_PREFIX)),
            Entry::Collection(id) => Some(id_space(id, COLLECTION_ID_PREFIX)),
            Entry::Comment(id) => Some(space_of(*id)),
            Entry::DevSnapshot(_) | Entry::Image(_) | Entry::Ballot(_) => None,
        }
    }

    /// What the entry is, for messages ("style S:3").
    pub fn describe(&self) -> String {
        match self {
            Entry::Style(id) => format!("style {id}"),
            Entry::Comment(id) => format!("comment thread {id}"),
            Entry::Collection(id) => format!("variable collection {id}"),
            Entry::Variable(id) => format!("variable {id}"),
            Entry::DevSnapshot(id) => format!("ready-for-dev snapshot of {id}"),
            Entry::Image(hash) => format!("image {hash}"),
            Entry::Ballot(voter) => format!("ballot of {voter}"),
        }
    }

    /// The list this entry belongs to (dev snapshots and images aren't ordered lists).
    pub(crate) fn list(&self) -> Option<Family> {
        match self {
            Entry::Style(_) => Some(Family::Styles),
            Entry::Comment(_) => Some(Family::Comments),
            Entry::Collection(_) => Some(Family::Collections),
            Entry::Variable(_) => Some(Family::Variables),
            Entry::DevSnapshot(_) | Entry::Image(_) | Entry::Ballot(_) => None,
        }
    }
}
