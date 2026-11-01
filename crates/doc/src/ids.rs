//! Ids of document-level objects (styles, variables, variable collections and modes, comment
//! threads) that stay unique when several people create them at the same time.
//!
//! Outside a live session (no client id, see [`Document::set_client_id`]) new ids are made exactly
//! as before: `S:<k>` with the first free `k` from the number of styles, `VariableID:<n>` and
//! `VariableCollectionId:<n>` one above the highest, `<collection>/mode<k>` with the first free `k`
//! from the number of modes, and comment ids one above the highest.
//!
//! During a session every new one takes its number from the document's id allocator, which hands
//! out `client_id << 32 | counter` like node ids and never hands a number out twice: `S:<n>`,
//! `VariableID:<n>`, `VariableCollectionId:<n>`, `<collection>/mode<n>` and comment id `n`. The
//! number's high 32 bits are the client whose space it lies in ([`space_of`]); ids from before
//! (small numbers) and ids of other forms (library copies `library:key`, imported files) are in
//! space 0. The session server accepts a new object only with an id in its creator's space or
//! space 0, so two people never make the same id.

use crate::{CollectionId, Document, ModeId, NodeId, StyleId, VariableId};

/// What style ids start with.
pub const STYLE_ID_PREFIX: &str = "S:";
/// What variable ids start with.
pub const VARIABLE_ID_PREFIX: &str = "VariableID:";
/// What variable collection ids start with.
pub const COLLECTION_ID_PREFIX: &str = "VariableCollectionId:";
/// What separates a mode id's collection from its number.
pub const MODE_ID_INFIX: &str = "/mode";

/// The client id space number `n` lies in (its high 32 bits; 0: made outside a live session).
pub fn space_of(n: u64) -> u32 {
    u32::try_from(n >> 32).unwrap_or(u32::MAX)
}

/// The number of id `<prefix><digits>`, if it has that form.
pub fn id_number(id: &str, prefix: &str) -> Option<u64> {
    digits(id.strip_prefix(prefix)?)
}

/// The client space of id `<prefix><n>` (0 for ids of any other form).
pub fn id_space(id: &str, prefix: &str) -> u32 {
    id_number(id, prefix).map_or(0, space_of)
}

/// The number at the end of mode id `<collection>/mode<n>`.
pub fn mode_number(id: &str) -> Option<u64> {
    digits(id.rsplit_once(MODE_ID_INFIX)?.1)
}

/// `s` as a number when it is only ASCII digits.
fn digits(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

impl Document {
    /// A number for a new document-level id in this document's client space, above every number
    /// of `taken` in that space. `None` outside a live session (ids are then made as before) or
    /// when the space is full.
    fn session_number(&mut self, taken: impl IntoIterator<Item = u64>) -> Option<u64> {
        let client = self.client_id();
        if client == 0 {
            return None;
        }
        // Numbers of this space the allocator hasn't seen (a document rebuilt from a snapshot)
        // stay taken.
        let highest = taken.into_iter().filter(|n| space_of(*n) == client).max();
        if let Some(h) = highest {
            self.reserve_ids(NodeId(h));
        }
        let n = self.alloc_id().0;
        (space_of(n) == client && highest.is_none_or(|h| n > h)).then_some(n)
    }

    /// A new style id.
    pub fn new_style_id(&mut self) -> StyleId {
        let taken: Vec<u64> = self.styles.iter().filter_map(|s| id_number(&s.id, STYLE_ID_PREFIX)).collect();
        if let Some(n) = self.session_number(taken) {
            return format!("{STYLE_ID_PREFIX}{n}");
        }
        (self.styles.len() + 1..)
            .map(|k| format!("{STYLE_ID_PREFIX}{k}"))
            .find(|id| self.style(id).is_none())
            .unwrap_or_else(|| format!("{STYLE_ID_PREFIX}0"))
    }

    /// A new variable id.
    pub fn new_variable_id(&mut self) -> VariableId {
        let taken: Vec<u64> = self.variables.variables.iter().filter_map(|v| id_number(&v.id, VARIABLE_ID_PREFIX)).collect();
        self.new_numbered_id(VARIABLE_ID_PREFIX, taken)
    }

    /// A new variable collection id.
    pub fn new_collection_id(&mut self) -> CollectionId {
        let taken: Vec<u64> = self.variables.collections.iter().filter_map(|c| id_number(&c.id, COLLECTION_ID_PREFIX)).collect();
        self.new_numbered_id(COLLECTION_ID_PREFIX, taken)
    }

    /// `<prefix><n>`: from this client's space in a session, else one above the highest of `taken`.
    fn new_numbered_id(&mut self, prefix: &str, taken: Vec<u64>) -> String {
        let n = match self.session_number(taken.iter().copied()) {
            Some(n) => n,
            None => taken.iter().max().map_or(1, |m| m.saturating_add(1)),
        };
        format!("{prefix}{n}")
    }

    /// A new id for a mode of collection `collection` (`None`: no such collection).
    pub fn new_mode_id(&mut self, collection: &str) -> Option<ModeId> {
        let modes = &self.variables.collections.iter().find(|c| c.id == collection)?.modes;
        let count = modes.len();
        let free = |doc: &Document, id: &str| doc.variables.collections.iter().any(|c| c.id == collection && c.modes.iter().all(|m| m.id != id));
        let taken: Vec<u64> = self.variables.collections.iter().flat_map(|c| c.modes.iter().filter_map(|m| mode_number(&m.id))).collect();
        if let Some(n) = self.session_number(taken) {
            let id = format!("{collection}{MODE_ID_INFIX}{n}");
            if free(self, &id) {
                return Some(id);
            }
        }
        (count + 1..).map(|k| format!("{collection}{MODE_ID_INFIX}{k}")).find(|id| free(self, id))
    }

    /// A new comment thread id.
    pub fn new_comment_id(&mut self) -> u64 {
        let taken: Vec<u64> = self.comments.iter().map(|c| c.id).collect();
        self.session_number(taken).unwrap_or_else(|| self.next_comment_id())
    }

    /// The highest client id space any id in the document lies in (layers, comments, styles,
    /// variables, collections, modes): a session hands out client ids above it, so ids made in
    /// earlier sessions are never made again. Space `u32::MAX` doesn't count: numbers too big for
    /// any client's space land there ([`space_of`]) and no session hands that client id out.
    pub fn highest_id_space(&self) -> u32 {
        let nodes = self.ids().map(|id| id.0);
        let comments = self.comments.iter().map(|c| c.id);
        let styles = self.styles.iter().filter_map(|s| id_number(&s.id, STYLE_ID_PREFIX));
        let vars = self.variables.variables.iter().filter_map(|v| id_number(&v.id, VARIABLE_ID_PREFIX));
        let colls = self.variables.collections.iter().filter_map(|c| id_number(&c.id, COLLECTION_ID_PREFIX));
        let modes = self.variables.collections.iter().flat_map(|c| c.modes.iter().filter_map(|m| mode_number(&m.id)));
        nodes.chain(comments).chain(styles).chain(vars).chain(colls).chain(modes).map(space_of).filter(|s| *s != u32::MAX).max().unwrap_or(0)
    }
}
