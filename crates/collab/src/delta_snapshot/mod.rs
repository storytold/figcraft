//! Direct mirror-delta and deduplicated-vector snapshots (`CatchUp::DeltaSnapshot`).
//!
//! Used for clients whose hello announces `delta-snapshot` and `design-review`.
//! Nodes are ordered topologically over `(parent, source)` edges so parents precede
//! children and acyclic sources precede their mirrors. Mirrors diff field values
//! structurally instead of trusting `Node::overrides`. Decoding streams records
//! one at a time under inflated-byte, exact declared-length, node-count and
//! materialized-memory budgets.

mod decode;
mod encode;
mod fields;

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::color::{Color, ColorProfile};
use figcraft_doc::{
    AnnotationCategory, Ballots, BoardTimer, Comment, DesignReview, DevSnapshot, FileKind, ImagePreset, Libraries, Node, NodeId, VectorNetwork, Vote,
    VoteRecord,
};
use serde::{Deserialize, Serialize};

use crate::Error;
use fields::FieldDelta;

pub use decode::{decode_delta_snapshot, decode_delta_snapshot_with_budget};
pub use encode::encode_delta_snapshot;

/// Wire header magic for [`CatchUp::DeltaSnapshot`](crate::CatchUp::DeltaSnapshot).
pub const DELTA_MAGIC: &[u8; 4] = b"FCDS";
/// Wire format version for [`CatchUp::DeltaSnapshot`](crate::CatchUp::DeltaSnapshot).
pub const DELTA_VERSION: u8 = 1;
/// Fixed header size: 4-byte magic, 1-byte version, 4-byte little-endian uncompressed length.
const HEADER_LEN: usize = 9;
/// Default ceiling on estimated materialized bytes during decode.
pub const DEFAULT_MAX_MATERIALIZED_BYTES: usize = 512 << 20;

/// Check the fixed header and declared uncompressed length in O(1).
///
/// Combined with [`decode_delta_snapshot_with_budget`] requiring `inflated.len() == declared`,
/// this prevents a client from inflating past `max_bytes`.
pub fn check_header_limit(bytes: &[u8], max_bytes: usize) -> Result<(), Error> {
    let declared = parse_header(bytes)?;
    if declared > max_bytes {
        return Err(Error::TooLarge(max_bytes));
    }
    Ok(())
}

fn parse_header(bytes: &[u8]) -> Result<usize, Error> {
    let header = bytes.get(..HEADER_LEN).ok_or_else(|| Error::Decode("delta snapshot header is truncated".into()))?;
    if header.get(..4) != Some(DELTA_MAGIC.as_slice()) {
        return Err(Error::Decode("delta snapshot magic mismatch".into()));
    }
    let version = header.get(4).copied().ok_or_else(|| Error::Decode("delta snapshot version missing".into()))?;
    if version != DELTA_VERSION {
        return Err(Error::Decode(format!("unsupported delta snapshot version {version}")));
    }
    let len_slice = header.get(5..9).ok_or_else(|| Error::Decode("delta snapshot length missing".into()))?;
    let len_bytes: [u8; 4] = len_slice.try_into().map_err(|_| Error::Decode("delta snapshot length".into()))?;
    usize::try_from(u32::from_le_bytes(len_bytes)).map_err(|_| Error::TooLarge(usize::MAX))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "r", rename_all = "camelCase")]
enum WireRecord {
    Full {
        pos: String,
        node: Box<Node>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vref: Option<u32>,
    },
    Mirror(Box<MirrorWire>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MirrorWire {
    id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<NodeId>,
    pos: String,
    source: NodeId,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    overrides: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    deltas: Vec<FieldDelta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WireImage {
    key: String,
    width: u32,
    height: u32,
    bytes: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireMeta {
    name: String,
    #[serde(default)]
    kind: FileKind,
    #[serde(default)]
    styles: Vec<figcraft_doc::Style>,
    #[serde(default)]
    variables: figcraft_doc::Variables,
    #[serde(default)]
    current_page: Option<NodeId>,
    #[serde(default)]
    document_colors: Vec<Color>,
    #[serde(default)]
    comments: Vec<Comment>,
    #[serde(default)]
    libraries: Libraries,
    #[serde(default)]
    color_profile: ColorProfile,
    #[serde(default)]
    image_presets: Vec<ImagePreset>,
    #[serde(default)]
    design_review: DesignReview,
    #[serde(default)]
    annotation_categories: Option<Vec<AnnotationCategory>>,
    #[serde(default)]
    dev_snapshots: BTreeMap<NodeId, DevSnapshot>,
    #[serde(default)]
    timer: Option<BoardTimer>,
    #[serde(default)]
    vote: Option<Vote>,
    #[serde(default)]
    votes: Ballots,
    #[serde(default)]
    vote_history: Vec<VoteRecord>,
    #[serde(default)]
    images: Vec<WireImage>,
}

/// Serialization envelope written in canonical order (`meta`, `vectors`, `records`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeltaWireRef<'a> {
    meta: &'a WireMeta,
    #[serde(skip_serializing_if = "<[VectorNetwork]>::is_empty")]
    vectors: &'a [VectorNetwork],
    records: &'a [WireRecord],
}
