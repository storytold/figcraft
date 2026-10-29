//! Comments: pinned notes with reply threads, stored in the file.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::{Document, NodeId};

/// An emoji reaction on a message and who left it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<String>,
}

/// One message in a comment thread (the first one, or a reply).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub author: String,
    pub text: String,
    /// Unix seconds.
    #[serde(default)]
    pub created: u64,
    /// When the text was last edited (Unix seconds; 0: never).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub edited: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
    /// Written by an AI agent (a command run for an MCP client), not by a person in the app.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub agent: bool,
}

/// A reply in a comment thread.
pub type Reply = Message;

fn is_zero(v: &u64) -> bool {
    *v == 0
}

impl Message {
    pub fn new(author: impl Into<String>, text: impl Into<String>, created: u64) -> Self {
        Self { author: author.into(), text: text.into(), created, edited: 0, reactions: Vec::new(), agent: false }
    }

    /// `user` left the `emoji` reaction.
    pub fn reacted(&self, emoji: &str, user: &str) -> bool {
        self.reactions.iter().any(|r| r.emoji == emoji && r.users.iter().any(|u| u == user))
    }

    /// Add `user`'s `emoji` reaction, or take it back (`on`: None toggles). True when it changed.
    pub fn react(&mut self, emoji: &str, user: &str, on: Option<bool>) -> bool {
        let want = on.unwrap_or(!self.reacted(emoji, user));
        if want == self.reacted(emoji, user) {
            return false;
        }
        match self.reactions.iter().position(|r| r.emoji == emoji) {
            Some(i) if want => {
                if let Some(r) = self.reactions.get_mut(i) {
                    r.users.push(user.to_string());
                }
            }
            Some(i) => {
                if let Some(r) = self.reactions.get_mut(i) {
                    r.users.retain(|u| u != user);
                    if r.users.is_empty() {
                        self.reactions.remove(i);
                    }
                }
            }
            None => self.reactions.push(Reaction { emoji: emoji.to_string(), users: vec![user.to_string()] }),
        }
        true
    }
}

/// A comment pinned to a point on a page, optionally anchored to a node so it follows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub page: NodeId,
    /// Page coordinates, or the offset in the anchor's local space when `anchor` is set.
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub anchor: Option<NodeId>,
    /// The message that starts the thread (its fields sit on the comment itself in the file).
    #[serde(flatten)]
    pub first: Message,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub replies: Vec<Message>,
    /// Who has read the thread as it is now. `None` (files from before read state was kept):
    /// read by everyone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_by: Option<Vec<String>>,
}

impl Comment {
    /// Where the pin sits on its page (anchored comments follow their node).
    pub fn position(&self, doc: &Document) -> (f64, f64) {
        if let Some(a) = self.anchor
            && doc.node(a).is_some()
        {
            let p = doc.world_transform(a) * crate::geom::Point::new(self.x, self.y);
            return (p.x, p.y);
        }
        (self.x, self.y)
    }

    /// The messages in order: the first one, then the replies.
    pub fn messages(&self) -> impl Iterator<Item = &Message> {
        std::iter::once(&self.first).chain(self.replies.iter())
    }

    /// A message by thread index (0: the first message, 1…: the replies).
    pub fn message(&self, index: usize) -> Option<&Message> {
        match index.checked_sub(1) {
            None => Some(&self.first),
            Some(i) => self.replies.get(i),
        }
    }

    /// A message by thread index, for editing.
    pub fn message_mut(&mut self, index: usize) -> Option<&mut Message> {
        match index.checked_sub(1) {
            None => Some(&mut self.first),
            Some(i) => self.replies.get_mut(i),
        }
    }

    /// When the thread last had a new message (Unix seconds).
    pub fn last_activity(&self) -> u64 {
        self.messages().map(|m| m.created).max().unwrap_or(0)
    }

    /// Everyone in the thread, in order of first appearance.
    pub fn participants(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for m in self.messages() {
            if !out.contains(&m.author.as_str()) {
                out.push(m.author.as_str());
            }
        }
        out
    }

    /// `user` wrote in the thread or is @mentioned in it.
    pub fn involves(&self, user: &str) -> bool {
        self.messages().any(|m| m.author == user || !mention_ranges(&m.text, &[user]).is_empty())
    }

    /// The thread has something `user` hasn't read.
    pub fn is_unread(&self, user: &str) -> bool {
        self.read_by.as_ref().is_some_and(|r| !r.iter().any(|u| u == user))
    }

    /// Mark the thread read or unread for `user`. True when it changed.
    pub fn set_read(&mut self, user: &str, read: bool) -> bool {
        if read != self.is_unread(user) {
            return false;
        }
        if read {
            if let Some(r) = self.read_by.as_mut() {
                r.push(user.to_string());
            }
            return true;
        }
        // Older files count as read by everyone: the thread's other participants stay so.
        if self.read_by.is_none() {
            self.read_by = Some(self.participants().into_iter().map(String::from).collect());
        }
        if let Some(r) = self.read_by.as_mut() {
            r.retain(|u| u != user);
        }
        true
    }

    /// A new message from `user`: the thread is unread for everyone else.
    pub fn touched_by(&mut self, user: &str) {
        self.read_by = Some(vec![user.to_string()]);
    }
}

/// The byte ranges of `@Name` mentions of `people` in `text` (names match ignoring ASCII case; the
/// longest name wins; a mention starts the text or follows a non-alphanumeric character and ends
/// before one).
pub fn mention_ranges(text: &str, people: &[&str]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(off) = text.get(from..).and_then(|t| t.find('@')) {
        let at = from + off;
        from = at + 1;
        let starts = text.get(..at).and_then(|t| t.chars().next_back()).is_none_or(|c| !c.is_alphanumeric());
        if !starts {
            continue;
        }
        let rest = text.get(at + 1..).unwrap_or("");
        let best = people
            .iter()
            .filter(|n| !n.is_empty())
            .filter(|n| rest.get(..n.len()).is_some_and(|h| h.eq_ignore_ascii_case(n)))
            .filter(|n| rest.get(n.len()..).and_then(|t| t.chars().next()).is_none_or(|c| !c.is_alphanumeric()))
            .map(|n| n.len())
            .max();
        if let Some(len) = best {
            out.push(at..at + 1 + len);
            from = at + 1 + len;
        }
    }
    out
}

impl Document {
    pub fn comment(&self, id: u64) -> Option<&Comment> {
        self.comments.iter().find(|c| c.id == id)
    }

    pub fn comment_mut(&mut self, id: u64) -> Option<&mut Comment> {
        self.comments.iter_mut().find(|c| c.id == id)
    }

    /// The next free comment id outside a live session (one above the highest; see
    /// [`new_comment_id`](Self::new_comment_id)).
    pub fn next_comment_id(&self) -> u64 {
        self.comments.iter().map(|c| c.id).max().unwrap_or(0).saturating_add(1)
    }

    /// Everyone named in the file's comments (authors and those who reacted), sorted.
    pub fn comment_people(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .comments
            .iter()
            .flat_map(|c| c.messages())
            .flat_map(|m| std::iter::once(&m.author).chain(m.reactions.iter().flat_map(|r| r.users.iter())))
            .filter(|n| !n.trim().is_empty())
            .cloned()
            .collect();
        out.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
        out.dedup();
        out
    }
}
