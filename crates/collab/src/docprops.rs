//! Document-level objects: diffing and applying the data that isn't a node (file name, styles,
//! variables, comments, images, dev snapshots, library data…).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use base64::Engine as _;
use figcraft_doc::{Collection, Comment, DevSnapshot, Document, ImageData, Style, Variable};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::Error;
use crate::change::{Change, DocProp, Entry, Family, Json};

/// An entry of a document-level list that has an id.
trait Keyed: Clone + PartialEq + Serialize + DeserializeOwned {
    type Key: Ord + Clone + Serialize + DeserializeOwned;
    fn key(&self) -> Self::Key;
}

impl Keyed for Style {
    type Key = String;
    fn key(&self) -> String {
        self.id.clone()
    }
}

impl Keyed for Comment {
    type Key = u64;
    fn key(&self) -> u64 {
        self.id
    }
}

impl Keyed for Collection {
    type Key = String;
    fn key(&self) -> String {
        self.id.clone()
    }
}

impl Keyed for Variable {
    type Key = String;
    fn key(&self) -> String {
        self.id.clone()
    }
}

/// How one keyed list of the document syncs.
struct List<T: Keyed> {
    whole: DocProp,
    order: DocProp,
    entry: fn(T::Key) -> Entry,
    get: fn(&Document) -> &Vec<T>,
    get_mut: fn(&mut Document) -> &mut Vec<T>,
}

const STYLES: List<Style> =
    List { whole: DocProp::Styles, order: DocProp::StyleOrder, entry: Entry::Style, get: |d| &d.styles, get_mut: |d| &mut d.styles };
const COMMENTS: List<Comment> =
    List { whole: DocProp::Comments, order: DocProp::CommentOrder, entry: Entry::Comment, get: |d| &d.comments, get_mut: |d| &mut d.comments };
const COLLECTIONS: List<Collection> = List {
    whole: DocProp::Collections,
    order: DocProp::CollectionOrder,
    entry: Entry::Collection,
    get: |d| &d.variables.collections,
    get_mut: |d| &mut d.variables.collections,
};
const VARIABLES: List<Variable> = List {
    whole: DocProp::Variables,
    order: DocProp::VariableOrder,
    entry: Entry::Variable,
    get: |d| &d.variables.variables,
    get_mut: |d| &mut d.variables.variables,
};

impl<T: Keyed> List<T> {
    fn diff(&self, a: &Document, b: &Document, out: &mut Vec<Change>) -> Result<(), Error> {
        let (la, lb) = ((self.get)(a), (self.get)(b));
        if la == lb {
            return Ok(());
        }
        let unique = |v: &[T]| {
            let mut seen = BTreeSet::new();
            v.iter().all(|t| seen.insert(t.key()))
        };
        if !unique(la) || !unique(lb) {
            out.push(Change::SetDoc { prop: self.whole, value: Json::of(lb)? });
            return Ok(());
        }
        let old: BTreeMap<T::Key, &T> = la.iter().map(|t| (t.key(), t)).collect();
        let new: BTreeSet<T::Key> = lb.iter().map(T::key).collect();
        for t in lb {
            if old.get(&t.key()).is_none_or(|o| *o != t) {
                out.push(Change::Put { entry: (self.entry)(t.key()), value: Json::of(t)? });
            }
        }
        for t in la {
            if !new.contains(&t.key()) {
                out.push(Change::Remove { entry: (self.entry)(t.key()) });
            }
        }
        // The order the puts and removes leave on `a` (survivors in place, new ones appended).
        let kb: Vec<T::Key> = lb.iter().map(T::key).collect();
        let left: Vec<T::Key> =
            la.iter().map(T::key).filter(|k| new.contains(k)).chain(kb.iter().filter(|k| !old.contains_key(*k)).cloned()).collect();
        if left != kb {
            out.push(Change::SetDoc { prop: self.order, value: Json::of(&kb)? });
        }
        Ok(())
    }

    fn put(&self, doc: &mut Document, key: T::Key, value: &Json) -> Result<(), Error> {
        let t: T = value.read("entry")?;
        if t.key() != key {
            return Err(Error::Invalid("an entry's id must match the entry it is put at".into()));
        }
        let list = (self.get_mut)(doc);
        match list.iter_mut().find(|e| e.key() == key) {
            Some(e) => *e = t,
            None => list.push(t),
        }
        Ok(())
    }

    fn remove(&self, doc: &mut Document, key: &T::Key) {
        (self.get_mut)(doc).retain(|e| e.key() != *key);
    }

    /// Entries in `keys` order first, the rest after in their current order.
    fn reorder(&self, doc: &mut Document, value: &Json) -> Result<(), Error> {
        let keys: Vec<T::Key> = value.read("order")?;
        let list = (self.get_mut)(doc);
        let mut slots: Vec<Option<T>> = std::mem::take(list).into_iter().map(Some).collect();
        let mut at: BTreeMap<T::Key, std::collections::VecDeque<usize>> = BTreeMap::new();
        for (i, e) in slots.iter().enumerate() {
            if let Some(e) = e {
                at.entry(e.key()).or_default().push_back(i);
            }
        }
        for k in keys {
            let Some(idx) = at.get_mut(&k) else { continue };
            let Some(i) = idx.pop_front() else { continue };
            if let Some(e) = slots.get_mut(i).and_then(Option::take) {
                list.push(e);
            }
        }
        list.extend(slots.into_iter().flatten());
        Ok(())
    }

    fn set_whole(&self, doc: &mut Document, value: &Json) -> Result<(), Error> {
        *(self.get_mut)(doc) = value.read("list")?;
        Ok(())
    }
}

/// An image as it travels in a change.
#[derive(Serialize, Deserialize)]
struct ImageValue {
    width: u32,
    height: u32,
    /// The encoded file (PNG, JPEG…), base64.
    bytes: String,
}

/// Largest image side: JPEG and GIF size fields are 16-bit; PNG can claim more, but no
/// image this app realistically opens reaches it — an entry claiming a bigger side is
/// refused rather than trusted. Zero is allowed: the format crate stores 0×0 for bytes it
/// couldn't probe (`image_data`).
pub(crate) const MAX_IMAGE_SIDE: u32 = 65_535;

/// Lowercase sha256 hex, the same form `figcraft_format::image_hash` produces.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The image an entry puts under `key`, which now holds `held`: [`image_of`], and a key that
/// already holds an image can't be given other bytes. A key always names the same bytes
/// (producers key images by their bytes; the REST importer's placeholder keys always hold the
/// same grey picture), so nobody can swap the picture under a key someone else added.
fn image_for(held: Option<&ImageData>, key: &str, value: &Json) -> Result<ImageData, Error> {
    let img = image_of(key, value)?;
    if held.is_some_and(|old| old.bytes != img.bytes) {
        return Err(Error::Invalid(format!("image: {key} already holds other bytes")));
    }
    Ok(img)
}

/// The JSON value an image entry carries: its size and its encoded bytes in base64.
fn image_json(img: &ImageData) -> Result<Json, Error> {
    Json::of(&ImageValue { width: img.width, height: img.height, bytes: base64::engine::general_purpose::STANDARD.encode(img.bytes.as_slice()) })
}

/// The image a change carries, validated as a collaborator's claim (an invalid entry
/// rejects its batch, leaving every replica's store clean): a key that can name an archive
/// member ([`ImageData::is_valid_key`]: a saved file writes it as `images/<key>`), dimensions within
/// [`MAX_IMAGE_SIDE`], decoded bytes within [`ImageData::MAX_BYTES`] (what the importers read;
/// the encoded length is capped before decoding so hostile text can't force a large
/// allocation), and a key that claims a content hash ([`ImageData::claims_content_hash`]) must
/// be exactly the lowercase sha256 of these bytes, the form `figcraft_format::image_hash`
/// writes — a poisoned content-addressed store would show the wrong image everywhere at once
/// (#1086). Other keys (the svg importer's FNV-128 keys, the REST importer's placeholders)
/// claim no content address, so only the name and size rules apply to them.
fn image_of(key: &str, value: &Json) -> Result<ImageData, Error> {
    image_within(key, value, ImageData::MAX_BYTES)
}

/// [`image_of`] with `max_bytes` for the decoded size.
pub(crate) fn image_within(key: &str, value: &Json, max_bytes: u64) -> Result<ImageData, Error> {
    if !ImageData::is_valid_key(key) {
        return Err(Error::Invalid(format!("image: {key:?} can't name an image (1–128 letters, digits, - or _)")));
    }
    let v: ImageValue = value.read("image")?;
    if v.width > MAX_IMAGE_SIDE || v.height > MAX_IMAGE_SIDE {
        return Err(Error::Invalid(format!("image: {}×{} passes the {} a side may be", v.width, v.height, MAX_IMAGE_SIDE)));
    }
    let max_encoded = (max_bytes / 3).saturating_mul(4).saturating_add(4);
    if u64::try_from(v.bytes.len()).unwrap_or(u64::MAX) > max_encoded {
        return Err(Error::Invalid(format!("image: base64 text longer than {max_encoded} bytes for {max_bytes} of bytes")));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(v.bytes.as_bytes()).map_err(|e| Error::Invalid(format!("image: {e}")))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
        return Err(Error::Invalid(format!("image: {} bytes pass the {max_bytes} an image may be", bytes.len())));
    }
    if ImageData::claims_content_hash(key) && key != sha256_hex(&bytes) {
        return Err(Error::Invalid(format!("image: the bytes aren't what their key {key} hashes to")));
    }
    Ok(ImageData { bytes: Arc::new(bytes), width: v.width, height: v.height })
}

macro_rules! doc_scalars {
    ($($variant:ident => $field:ident),* $(,)?) => {
        fn diff_scalars(a: &Document, b: &Document, out: &mut Vec<Change>) -> Result<(), Error> {
            $(if a.$field != b.$field {
                out.push(Change::SetDoc { prop: DocProp::$variant, value: Json::of(&b.$field)? });
            })*
            Ok(())
        }

        /// A scalar document property's value; `None` for list properties.
        fn get_scalar(doc: &Document, prop: DocProp) -> Result<Option<Json>, Error> {
            match prop {
                $(DocProp::$variant => Json::of(&doc.$field).map(Some),)*
                _ => Ok(None),
            }
        }

        /// Write a scalar document property; false for list properties.
        fn set_scalar(doc: &mut Document, prop: DocProp, value: &Json) -> Result<bool, Error> {
            match prop {
                $(DocProp::$variant => doc.$field = value.read(stringify!($field))?,)*
                _ => return Ok(false),
            }
            Ok(true)
        }
    };
}

doc_scalars! {
    Name => name,
    CurrentPage => current_page,
    DocumentColors => document_colors,
    ColorProfile => color_profile,
    ImagePresets => image_presets,
    AnnotationCategories => annotation_categories,
    Libraries => libraries,
    Kind => kind,
    Timer => timer,
    Vote => vote,
    VoteHistory => vote_history,
}

/// Keyed maps (no order): changed and added entries are put, missing ones removed.
fn diff_map<K: Ord + Clone, V>(
    a: &BTreeMap<K, V>,
    b: &BTreeMap<K, V>,
    same: impl Fn(&V, &V) -> bool,
    entry: impl Fn(&K) -> Entry,
    json: impl Fn(&V) -> Result<Json, Error>,
    out: &mut Vec<Change>,
) -> Result<(), Error> {
    for (k, v) in b {
        if a.get(k).is_none_or(|o| !same(o, v)) {
            out.push(Change::Put { entry: entry(k), value: json(v)? });
        }
    }
    for k in a.keys() {
        if !b.contains_key(k) {
            out.push(Change::Remove { entry: entry(k) });
        }
    }
    Ok(())
}

/// The document-level changes that turn `a` into `b`.
pub(crate) fn diff(a: &Document, b: &Document, out: &mut Vec<Change>) -> Result<(), Error> {
    diff_scalars(a, b, out)?;
    STYLES.diff(a, b, out)?;
    COMMENTS.diff(a, b, out)?;
    COLLECTIONS.diff(a, b, out)?;
    VARIABLES.diff(a, b, out)?;
    diff_map(&a.dev_snapshots, &b.dev_snapshots, |x, y| Arc::ptr_eq(x, y) || x == y, |k| Entry::DevSnapshot(*k), Json::of, out)?;
    diff_map(
        &a.images,
        &b.images,
        |x, y| x.width == y.width && x.height == y.height && (Arc::ptr_eq(&x.bytes, &y.bytes) || x.bytes == y.bytes),
        |k| Entry::Image(k.clone()),
        image_json,
        out,
    )?;
    diff_map(&a.votes, &b.votes, |x, y| x == y, |k| Entry::Ballot(k.clone()), Json::of, out)
}

/// Apply a document-level change. Returns whether it can change which pages are listed (the
/// library store page is never listed).
pub(crate) fn apply(doc: &mut Document, change: &Change) -> Result<bool, Error> {
    match change {
        Change::SetDoc { prop, value } => {
            if set_scalar(doc, *prop, value)? {
                return Ok(*prop == DocProp::Libraries);
            }
            match prop {
                DocProp::Styles => STYLES.set_whole(doc, value),
                DocProp::StyleOrder => STYLES.reorder(doc, value),
                DocProp::Comments => COMMENTS.set_whole(doc, value),
                DocProp::CommentOrder => COMMENTS.reorder(doc, value),
                DocProp::Collections => COLLECTIONS.set_whole(doc, value),
                DocProp::CollectionOrder => COLLECTIONS.reorder(doc, value),
                DocProp::Variables => VARIABLES.set_whole(doc, value),
                DocProp::VariableOrder => VARIABLES.reorder(doc, value),
                _ => Err(Error::Invalid(format!("{prop:?} is not a list"))),
            }?;
        }
        Change::Put { entry, value } => match entry {
            Entry::Style(k) => STYLES.put(doc, k.clone(), value)?,
            Entry::Comment(k) => COMMENTS.put(doc, *k, value)?,
            Entry::Collection(k) => COLLECTIONS.put(doc, k.clone(), value)?,
            Entry::Variable(k) => VARIABLES.put(doc, k.clone(), value)?,
            Entry::DevSnapshot(k) => {
                let snap: DevSnapshot = value.read("dev snapshot")?;
                doc.dev_snapshots.insert(*k, Arc::new(snap));
            }
            Entry::Image(k) => {
                let img = image_for(doc.images.get(k), k, value)?;
                doc.images.insert(k.clone(), img);
            }
            Entry::Ballot(k) => {
                doc.votes.insert(k.clone(), value.read("ballot")?);
            }
        },
        Change::Remove { entry } => match entry {
            Entry::Style(k) => STYLES.remove(doc, k),
            Entry::Comment(k) => COMMENTS.remove(doc, k),
            Entry::Collection(k) => COLLECTIONS.remove(doc, k),
            Entry::Variable(k) => VARIABLES.remove(doc, k),
            Entry::DevSnapshot(k) => {
                doc.dev_snapshots.remove(k);
            }
            Entry::Image(k) => {
                // An image a layer or style shows stays: removing it would let other bytes go
                // under its key, swapping the picture that layer shows ([`image_for`]).
                if figcraft_doc::used_images(doc).contains(k) {
                    return Err(Error::Invalid(format!("image: {k} is still in use")));
                }
                doc.images.remove(k);
            }
            Entry::Ballot(k) => {
                doc.votes.remove(k);
            }
        },
        _ => return Err(Error::Invalid("not a document-level change".into())),
    }
    Ok(false)
}

/// Document-level property `prop` of `doc` (lists whole; orders as the ids in order).
pub(crate) fn get(doc: &Document, prop: DocProp) -> Result<Json, Error> {
    if let Some(v) = get_scalar(doc, prop)? {
        return Ok(v);
    }
    fn order<T: Keyed>(list: &[T]) -> Result<Json, Error> {
        Json::of(&list.iter().map(T::key).collect::<Vec<_>>())
    }
    match prop {
        DocProp::Styles => Json::of(&doc.styles),
        DocProp::StyleOrder => order(&doc.styles),
        DocProp::Comments => Json::of(&doc.comments),
        DocProp::CommentOrder => order(&doc.comments),
        DocProp::Collections => Json::of(&doc.variables.collections),
        DocProp::CollectionOrder => order(&doc.variables.collections),
        DocProp::Variables => Json::of(&doc.variables.variables),
        DocProp::VariableOrder => order(&doc.variables.variables),
        _ => Err(Error::Invalid(format!("{prop:?} has no value"))),
    }
}

/// Document-level entry `entry` of `doc` (`None`: there is none).
pub(crate) fn entry_value(doc: &Document, entry: &Entry) -> Result<Option<Json>, Error> {
    let value = match entry {
        Entry::Style(k) => doc.styles.iter().find(|s| s.id == *k).map(Json::of),
        Entry::Comment(k) => doc.comments.iter().find(|c| c.id == *k).map(Json::of),
        Entry::Collection(k) => doc.variables.collections.iter().find(|c| c.id == *k).map(Json::of),
        Entry::Variable(k) => doc.variables.variables.iter().find(|v| v.id == *k).map(Json::of),
        Entry::DevSnapshot(k) => doc.dev_snapshots.get(k).map(|s| Json::of(s.as_ref())),
        Entry::Image(k) => doc.images.get(k).map(image_json),
        Entry::Ballot(k) => doc.votes.get(k).map(Json::of),
    };
    value.transpose()
}

/// Whether `doc` has entry `entry`.
pub(crate) fn has(doc: &Document, entry: &Entry) -> bool {
    match entry {
        Entry::Style(k) => doc.styles.iter().any(|s| s.id == *k),
        Entry::Comment(k) => doc.comments.iter().any(|c| c.id == *k),
        Entry::Collection(k) => doc.variables.collections.iter().any(|c| c.id == *k),
        Entry::Variable(k) => doc.variables.variables.iter().any(|v| v.id == *k),
        Entry::DevSnapshot(k) => doc.dev_snapshots.contains_key(k),
        Entry::Image(k) => doc.images.contains_key(k),
        Entry::Ballot(k) => doc.votes.contains_key(k),
    }
}

/// The entries of list `family` in `doc`.
pub(crate) fn entries(doc: &Document, family: Family) -> Vec<Entry> {
    fn keys<T: Keyed>(list: &List<T>, doc: &Document) -> Vec<Entry> {
        (list.get)(doc).iter().map(|t| (list.entry)(t.key())).collect()
    }
    match family {
        Family::Styles => keys(&STYLES, doc),
        Family::Comments => keys(&COMMENTS, doc),
        Family::Collections => keys(&COLLECTIONS, doc),
        Family::Variables => keys(&VARIABLES, doc),
    }
}

/// The current value of what `change` writes, as a change (for a rejected batch's fixes): the
/// property or entry as it is now, or the entry's removal. List properties revert whole.
pub(crate) fn current(doc: &Document, change: &Change) -> Result<Option<Change>, Error> {
    match change {
        Change::SetDoc { prop, .. } => {
            // An order reverts as the whole list (the entries may have changed too).
            let prop = match prop.list() {
                Some(Ok(_)) | None => *prop,
                Some(Err(l)) => match l {
                    Family::Styles => DocProp::Styles,
                    Family::Comments => DocProp::Comments,
                    Family::Collections => DocProp::Collections,
                    Family::Variables => DocProp::Variables,
                },
            };
            Ok(Some(Change::SetDoc { prop, value: get(doc, prop)? }))
        }
        Change::Put { entry, .. } | Change::Remove { entry } => {
            Ok(crate::change::Target::Entry(entry.clone()).write(entry_value(doc, entry)?.as_ref()))
        }
        _ => Ok(None),
    }
}
