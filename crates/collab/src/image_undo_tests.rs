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
    assert_eq!(
        before.asset_payload(&figcraft_assets::AssetRef::of_arc(&bytes)).unwrap().as_slice(),
        bytes.as_slice(),
        "current originals remain available independently of disk cache eviction"
    );
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

#[test]
fn typed_image_operations_preserve_key_dimension_and_immutability_checks() {
    use crate::change::ImageChange;
    use crate::{Change, Replica, replica::Mode};
    let bytes = std::sync::Arc::new(b"synthetic typed image".to_vec());
    let image = figcraft_doc::ImageData { bytes, width: 1, height: 1, asset: None };
    let mut r = Replica::new(figcraft_doc::Document::new("typed image validation"));
    let valid = ImageChange::of("valid-key", &image);
    r.apply(&[Change::Image(valid.clone())], Mode::Strict).unwrap();
    for key in ["../escape", &"a".repeat(64)] {
        let mut bad = valid.clone();
        bad.key = key.into();
        assert!(r.clone().apply(&[Change::Image(bad)], Mode::Strict).is_err());
    }
    let mut bad = valid.clone();
    bad.width = 65_536;
    assert!(r.clone().apply(&[Change::Image(bad)], Mode::Strict).is_err());
    let mut changed = image.clone();
    changed.bytes = std::sync::Arc::new(b"other content".to_vec());
    assert!(r.apply(&[Change::Image(ImageChange::of("valid-key", &changed))], Mode::Strict).is_err());
}
