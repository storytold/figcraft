//! Image entries are checked as they apply: bytes carried under a content-hash key must be
//! exactly what that key hashes to (lowercase sha256, the form `figcraft_format::image_hash`
//! produces), a side stays within the 65 535 px bound the implementation enforces and decoded
//! bytes within its 32 MiB cap. Without this a collaborator could poison the shared
//! content-addressed image store or balloon memory (#1086).

use base64::Engine as _;
use figcraft_doc::Document;
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::change::{Change, Entry, Json};
use crate::docprops::apply as apply_doc;
use crate::docprops::{MAX_IMAGE_SIDE, image_within};

/// The side bound the implementation enforces.
const MAX_SIDE: u32 = MAX_IMAGE_SIDE;

/// sha256 hex, computed here independently of the implementation under test.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// An image entry putting `bytes` (`width` × `height`) under `key`.
pub(crate) fn put(key: &str, width: u32, height: u32, bytes: &[u8]) -> Change {
    Change::Put {
        entry: Entry::Image(key.into()),
        value: Json(json!({
            "width": width,
            "height": height,
            "bytes": base64::engine::general_purpose::STANDARD.encode(bytes),
        })),
    }
}

/// Bytes a collaborator hashed into the key, and different bytes they send under it.
pub(crate) fn forged() -> (String, &'static [u8]) {
    (sha256_hex(b"the image the key actually names"), b"someone else's bytes")
}

#[test]
fn bytes_that_do_not_hash_to_their_content_hash_key_are_refused() {
    let (key, bytes) = forged();
    let mut doc = Document::new("t");
    let e = match apply_doc(&mut doc, &put(&key, 2, 1, bytes)) {
        Ok(_) => panic!("bytes that don't hash to their key must be refused"),
        Err(e) => e,
    };
    assert!(e.to_string().contains("image"), "not an image refusal: {e}");
    assert!(doc.images.is_empty(), "the image store was poisoned under key {key}");
}

/// An uppercase key looks like a content hash but isn't one: producers store the lowercase
/// form, so bytes matching under the uppercase form would file a second copy under a name
/// no honest lookup reaches. Refused (#1086).
#[test]
fn an_uppercase_content_hash_shaped_key_is_refused() {
    let bytes = b"honestly keyed, uppercase claim".as_slice();
    let mut doc = Document::new("t");
    let upper = sha256_hex(bytes).to_uppercase();
    let e = match apply_doc(&mut doc, &put(&upper, 2, 1, bytes)) {
        Ok(_) => panic!("an uppercase key claims a content address it doesn't have"),
        Err(e) => e,
    };
    assert!(e.to_string().contains("image"), "not an image refusal: {e}");
    assert!(doc.images.is_empty(), "the store took an uppercase-keyed copy");
}

#[test]
fn an_image_that_hashes_to_its_key_applies() {
    let bytes = b"a real image, honestly keyed".as_slice();
    let mut doc = Document::new("t");
    apply_doc(&mut doc, &put(&sha256_hex(bytes), 2, 1, bytes)).expect("an honest image must apply unchanged");
    let img = doc.images.get(&sha256_hex(bytes)).expect("the image is stored");
    assert_eq!((img.width, img.height), (2, 1));
    assert_eq!(img.bytes.as_slice(), bytes);
}

#[test]
fn absurd_dimensions_are_refused() {
    let bytes = b"honest bytes, dishonest size".as_slice();
    let key = sha256_hex(bytes);
    for (w, h) in [(MAX_SIDE + 1, 10), (10, MAX_SIDE + 1), (u32::MAX, u32::MAX)] {
        let mut doc = Document::new("t");
        apply_doc(&mut doc, &put(&key, w, h, bytes))
            .err()
            .unwrap_or_else(|| panic!("a {w}×{h} image must be refused, but it applied to {:?} keys", doc.images.len()));
        assert!(doc.images.is_empty());
    }
}

/// Bytes past the cap are refused, and so is base64 text too long to decode within it (checked
/// before decoding). The cap is `figcraft_doc::ImageData::MAX_BYTES` (256 MiB); a small one keeps
/// the test light.
#[test]
fn oversized_image_bytes_are_refused() {
    let value = |bytes: &[u8]| json!({"width": 1, "height": 1, "bytes": base64::engine::general_purpose::STANDARD.encode(bytes)});
    let at = vec![0u8; 1024];
    image_within(&sha256_hex(&at), &Json(value(&at)), 1024).expect("exactly the cap applies");
    let past = vec![0u8; 1025];
    assert!(image_within(&sha256_hex(&past), &Json(value(&past)), 1024).is_err(), "one byte past the cap");
    let long_text = json!({"width": 1, "height": 1, "bytes": "A".repeat(4096)});
    let e = image_within("img1", &Json(long_text), 1024).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("base64 text longer"), "refused before decoding: {e:?}");
}

#[test]
fn a_zero_sized_image_still_applies() {
    // The format crate stores 0×0 for bytes it can't probe (`image_data` in `figcraft_format`),
    // so a shared document can legitimately carry one; the bytes still have to be honest.
    let bytes = b"undecodable bytes kept from a file load".as_slice();
    let mut doc = Document::new("t");
    apply_doc(&mut doc, &put(&sha256_hex(bytes), 0, 0, bytes)).expect("0×0 means the size is unknown, not absurd");
    assert!(doc.images.contains_key(&sha256_hex(bytes)));
}

#[test]
fn a_key_that_is_not_a_content_hash_shape_still_applies() {
    // Legacy producers key images without sha256: the svg importer stores FNV-128 keys (32 hex
    // chars, re-keyed by sha256 on save), and the REST importer keeps a placeholder under the
    // imageRef itself. They are not content-addressed references, so no hash is claimed; the
    // dimension and size bounds still hold.
    for key in ["rest-image-missing", "hash", &"f".repeat(32), &"ab".repeat(20)] {
        let mut doc = Document::new("t");
        apply_doc(&mut doc, &put(key, 2, 1, b"placeholder bytes")).unwrap_or_else(|e| panic!("legacy key {key:?} refused: {e}"));
        assert!(doc.images.contains_key(key));
        let mut big = Document::new("t");
        assert!(apply_doc(&mut big, &put(key, MAX_SIDE + 1, 1, b"placeholder bytes")).is_err(), "a legacy key must still bound dimensions");
    }
}

/// A key becomes an archive member name when the file is saved (`images/<key>`), so a key that
/// isn't plain letters and digits is refused as it applies: a collaborator could otherwise write
/// `images/../../x` into everyone's saved file, which the next load drops as damaged (#1086).
#[test]
fn a_key_that_cannot_name_an_archive_member_is_refused() {
    for key in ["../../evil", "a/b", "x.png", "", "with space", "\\server", &"a".repeat(129)] {
        let mut doc = Document::new("t");
        assert!(apply_doc(&mut doc, &put(key, 2, 1, b"placeholder bytes")).is_err(), "key {key:?} must be refused");
        assert!(doc.images.is_empty(), "key {key:?} reached the image store");
    }
}

/// The cap matches what the importers read (`figcraft_doc::ImageData::MAX_BYTES`): an image a
/// document took from a `.fig`, `.sketch` or REST import reaches everyone in a live session.
#[test]
fn an_image_as_large_as_an_import_takes_still_applies() {
    let bytes = vec![0u8; 40 << 20];
    let mut doc = Document::new("t");
    apply_doc(&mut doc, &put(&sha256_hex(&bytes), 0, 0, &bytes)).expect("a 40 MiB image is within what imports read");
    assert_eq!(doc.images.len(), 1);
}

/// An image key is content-derived by every producer, so its bytes never change: a collaborator
/// can't swap the picture under a key another one added.
#[test]
fn an_image_cannot_be_replaced_under_its_key() {
    let mut doc = Document::new("t");
    apply_doc(&mut doc, &put("img1", 2, 1, b"the first bytes")).unwrap();
    assert!(apply_doc(&mut doc, &put("img1", 2, 1, b"other bytes")).is_err(), "replaced under the same key");
    assert_eq!(doc.images.get("img1").map(|i| i.bytes.as_slice()), Some(b"the first bytes".as_slice()));
    apply_doc(&mut doc, &put("img1", 2, 1, b"the first bytes")).expect("the same image again is fine (undo, redo)");
}

/// A snapshot written before these checks may hold an image they refuse: the session still
/// opens, without that image, instead of not opening at all.
#[test]
fn a_snapshot_holding_an_image_the_checks_refuse_still_opens_without_it() {
    let mut snapshot = crate::Replica::new(crate::sim_tests::base()).snapshot().unwrap();
    let (key, bytes) = forged();
    snapshot.push(put(&key, 2, 1, bytes));
    let r = crate::Replica::from_snapshot(&snapshot).expect("the snapshot opens");
    assert!(!r.document().images.contains_key(&key));
}

/// A server's log written before these checks may hold an image they refuse: a restart replays
/// the edits after it instead of dropping them with the rest of the log.
#[test]
fn a_restart_keeps_the_edits_after_an_image_the_checks_refuse() {
    let s = crate::Server::new(crate::sim_tests::base(), "", crate::Limits::default());
    let mut r = crate::Server::restore(s.state().unwrap(), "", crate::Limits::default()).unwrap();
    let (key, bytes) = forged();
    let good = b"an honest image after it".as_slice();
    let accepted = |seq, changes| crate::Journal::Accepted(crate::message::Accepted { seq, author: 1, batch_id: seq, changes });
    for j in [crate::Journal::Issued { client: 1 }, accepted(1, vec![put(&key, 2, 1, bytes)]), accepted(2, vec![put(&sha256_hex(good), 2, 1, good)])]
    {
        r.replay(j).expect("the log replays");
    }
    assert_eq!(r.seq(), 2, "both batches count");
    assert!(!r.document().images.contains_key(&key), "the refused image is left out");
    assert!(r.document().images.contains_key(&sha256_hex(good)), "the edit after it is kept");
}

/// A server's log written before these checks may give an image other bytes under its key (the
/// apply step allowed it then): a restart replays past that batch, keeping the first bytes and
/// every edit after it, instead of stopping there and dropping the rest of the log.
#[test]
fn a_restart_replays_past_a_logged_image_replacement() {
    let s = crate::Server::new(crate::sim_tests::base(), "", crate::Limits::default());
    let mut r = crate::Server::restore(s.state().unwrap(), "", crate::Limits::default()).unwrap();
    let good = b"an honest image after it".as_slice();
    let accepted = |seq, changes| crate::Journal::Accepted(crate::message::Accepted { seq, author: 1, batch_id: seq, changes });
    let journal = [
        crate::Journal::Issued { client: 1 },
        accepted(1, vec![put("img1", 2, 1, b"the first bytes")]),
        accepted(2, vec![put("img1", 2, 1, b"other bytes")]),
        accepted(3, vec![put(&sha256_hex(good), 2, 1, good)]),
    ];
    for j in journal {
        r.replay(j).expect("the log replays");
    }
    assert_eq!(r.seq(), 3, "every batch counts");
    assert_eq!(r.document().images.get("img1").map(|i| i.bytes.as_slice()), Some(b"the first bytes".as_slice()), "the replacement is left out");
    assert!(r.document().images.contains_key(&sha256_hex(good)), "the edit after it is kept");
}

/// A document whose rectangle shows `img1` (the first bytes), and that rectangle.
fn in_use() -> (Document, figcraft_doc::NodeId) {
    let mut doc = Document::new("t");
    let page = doc.first_page().unwrap();
    let id = doc.create(page, figcraft_doc::NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    doc.get_mut(id).unwrap().fills = vec![figcraft_color::Paint::image("img1", figcraft_color::ImageScaleMode::Fill)];
    apply_doc(&mut doc, &put("img1", 2, 1, b"the first bytes")).unwrap();
    (doc, id)
}

/// An image a layer or style still shows can't be removed: removing it and putting other bytes
/// under its key would swap the picture that layer shows, which giving the key other bytes
/// directly can't do (#1086).
#[test]
fn an_image_in_use_cannot_be_removed() {
    let (mut doc, _) = in_use();
    let remove = Change::Remove { entry: Entry::Image("img1".into()) };
    assert!(apply_doc(&mut doc, &remove).is_err(), "removed while a layer shows it");
    assert!(apply_doc(&mut doc, &put("img1", 2, 1, b"other bytes")).is_err());
    assert_eq!(doc.images.get("img1").map(|i| i.bytes.as_slice()), Some(b"the first bytes".as_slice()));
}

/// A batch that first stops using an image, then removes it (the order a diff makes), applies.
#[test]
fn an_image_no_longer_in_use_can_be_removed() {
    let (doc, id) = in_use();
    let mut r = crate::Replica::new(doc);
    let batch = [Change::Delete { id }, Change::Remove { entry: Entry::Image("img1".into()) }];
    r.apply(&batch, crate::Mode::Strict).expect("the layer goes first, then its image");
    assert!(r.document().images.is_empty());
}

/// Only image entries are left out of saved history: a snapshot whose other changes don't apply
/// is still an error, not a document missing them.
#[test]
fn a_snapshot_with_a_broken_layer_change_still_fails_to_open() {
    let mut snapshot = crate::Replica::new(crate::sim_tests::base()).snapshot().unwrap();
    // A property of a layer that isn't there: an error when applied (unlike deleting it).
    let missing = figcraft_doc::NodeId(u64::MAX);
    snapshot.push(Change::Set { id: missing, prop: crate::change::Prop::Name, value: Json(json!("x")) });
    assert!(crate::Replica::from_snapshot(&snapshot).is_err());
}
