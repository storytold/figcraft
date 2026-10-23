//! Resident image edits retain binary undo payloads even when the replica's opportunistic
//! old-image pool has evicted them. Lazy viewport downloads never enter this history.
use crate::history::{BACK, Step};
use crate::{Replica, diff};
use figcraft_doc::{Document, ImageData};
use std::sync::Arc;

#[test]
fn undo_a_removed_large_image_does_not_turn_it_into_a_missing_reference() {
    let bytes = Arc::new(vec![7; (128 << 20) + 1]);
    let mut doc = Document::new("Synthetic image undo");
    doc.images.insert("large".into(), ImageData { asset: None, bytes: bytes.clone(), width: 10, height: 10 });
    let before = Replica::new(doc.clone());
    doc.images.clear();
    let change = diff(&before, &doc).unwrap();
    let step = Step::new(1, "Remove image", 1, &before, &change.replica, &change.changes).unwrap();
    assert!(change.replica.asset_payload(&figcraft_assets::AssetRef::of_arc(&bytes)).is_none());
    let undo = step.toward(BACK, &change.replica).unwrap();
    let mut restored = change.replica;
    restored.apply(&undo, crate::replica::Mode::Strict).unwrap();
    assert_eq!(*restored.document().images["large"].bytes, *bytes);
    assert!(restored.document().images["large"].asset.is_none());
}
