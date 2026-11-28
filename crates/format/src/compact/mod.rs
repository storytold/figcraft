//! Compact `.figcraft` document encoding for mirrored instance layers and repeated vectors.
//!
//! Unchanged and partially overridden mirrored layers (`Node::source`) are written as delta
//! records (`"mirror": true` plus only the fields that differ from `source`), and repeated
//! non-empty `VectorNetwork`s are hoisted into a top-level `"vectors"` table referenced by
//! `"vectorRef"`. Documents without mirrors or repeated vectors serialize identically to legacy
//! `document.json`, and legacy v1 files always load unchanged.

mod decode;
mod encode;
mod fields;

pub(crate) use decode::parse_compact_document;
pub(crate) use encode::CompactDocument;

#[cfg(test)]
pub(crate) use decode::{materialized_budget, parse_compact_document_with_budget};
