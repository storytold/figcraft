//! On-canvas text editing: caret, selection, typing. A whole editing session is one undo step.

use std::sync::{LazyLock, Mutex};

use figcraft_doc::{Document, ListStyle, NodeId, NodeKind, TextData};

use crate::{EngineError, Preedit, Result, Session, TextEdit};

/// Most committed UTF-8 bytes an outgoing IME snapshot may describe.
///
/// This mirrors the fork's `egui::output::MAX_IME_SURROUNDING_TEXT_BYTES`; the engine keeps its
/// own constant so this layer has no egui dependency. The query below returns only this bounded
/// window around the selection, never a copy of the whole document.
pub const MAX_IME_SURROUNDING_BYTES: usize = 4000;

/// Stable identity of the on-canvas text edit an IME context belongs to.
///
/// `document` and `node` do not change through ordinary text or caret edits. `editing_session`
/// changes when [`begin`] starts a new session for an equal `(document, node)` pair (leaving and
/// re-entering the same layer), so a replaced target never inherits an old composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImeEditOwner {
    /// The document tab's uid.
    pub document: u64,
    /// The edited text node.
    pub node: NodeId,
    /// [`TextEdit::editing_session`] of the current edit.
    pub editing_session: u64,
}

/// Why the committed text around the IME caret cannot be described.
///
/// These are the only reasons this custom editor reports
/// `egui::output::IMESurroundingText::Unavailable`; an empty text layer is available with empty
/// text and a collapsed selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImeSurroundingUnavailable {
    /// No text layer is being edited.
    NotEditing,
    /// The editing target is not a text node any more.
    NotText,
    /// The selection alone is wider than [`MAX_IME_SURROUNDING_BYTES`], so no bounded snapshot
    /// can contain both selection ends.
    SelectionTooWide,
    /// Defensive: the computed window was not a character-boundary slice. The rounding above
    /// makes this unreachable for a well-formed text model.
    InvalidWindow,
}

/// One bounded, committed logical-text observation for the outgoing IME snapshot.
///
/// `text` is never display/bidi order and never contains the pre-edit (which is UI-only in this
/// editor). `cursor` and `anchor` are UTF-8 byte offsets into `text`; `anchor > cursor` is a
/// valid reversed selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImeSurrounding {
    /// Identity of the edit this observation belongs to.
    pub owner: ImeEditOwner,
    /// At most [`MAX_IME_SURROUNDING_BYTES`] bytes of committed text around the selection.
    pub text: String,
    /// UTF-8 byte offset of the primary caret in [`Self::text`].
    pub cursor: usize,
    /// UTF-8 byte offset of the selection anchor in [`Self::text`].
    pub anchor: usize,
}

/// Every editing session gets its own token; zero is valid and never reused deliberately.
///
/// A mutex (rather than an atomic) also works on wasm32 targets without 64-bit atomics.
static EDITING_SESSION: LazyLock<Mutex<u64>> = LazyLock::new(|| Mutex::new(0));

/// Return the next editing-session token from a mutex-protected counter (`wrapping_add`).
/// Zero is a valid token; the mutex also works on wasm32 without 64-bit atomics.
fn next_editing_session() -> u64 {
    let mut session = match EDITING_SESSION.lock() {
        Ok(session) => session,
        Err(poisoned) => poisoned.into_inner(),
    };
    *session = session.wrapping_add(1);
    *session
}

fn text_of(s: &Session, id: NodeId) -> Result<TextData> {
    match &s.doc()?.get(id)?.kind {
        NodeKind::Text(t) => Ok(t.clone()),
        _ => Err(EngineError::Other(format!("{id} is not a text layer"))),
    }
}

/// Start editing text node `id` with the caret at `caret` (None = select all).
pub fn begin(s: &mut Session, id: NodeId, caret: Option<usize>, created: bool) -> Result<()> {
    let t = text_of(s, id)?;
    let n = t.char_count();
    s.begin_interaction("Edit text")?;
    let ds = s.ds_mut()?;
    // Editing the same layer again (a double-click into a text just made) keeps it new.
    let created = created || ds.text_edit.as_ref().is_some_and(|te| te.id == id && te.created);
    let (caret, anchor) = match caret {
        Some(c) => (c.min(n), c.min(n)),
        None => (n, 0),
    };
    ds.text_edit = Some(TextEdit { id, caret, anchor, created, editing_session: next_editing_session(), preedit: None });
    ds.selection = vec![id];
    ds.revision += 1;
    s.live_text_notice(id);
    Ok(())
}

/// Finish editing: an empty new text layer is removed; the session becomes one undo step.
pub fn end(s: &mut Session) -> Result<()> {
    let Some(te) = s.ds()?.text_edit.clone() else { return Ok(()) };
    s.ds_mut()?.text_edit = None;
    // A widget's editable field: the typing is put back and the new value goes to the widget's
    // plugin through `widget.setText`, one undo step (or none when nothing changed).
    if let Some((w, field)) = s.doc().ok().and_then(|d| crate::widgets::field_of(d, te.id)) {
        let value = text_of(s, te.id).map(|t| t.characters).unwrap_or_default();
        s.cancel_interaction()?;
        let ds = s.ds_mut()?;
        ds.selection = vec![w];
        ds.revision += 1;
        if text_of(s, te.id).is_ok_and(|t| t.characters != value)
            && let Err(e) = s.execute("widget.setText", &serde_json::json!({"id": w, "field": field, "value": value}))
        {
            s.message = Some(e.to_string());
        }
        return Ok(());
    }
    // A board object's text (a sticky's note) stays when emptied; the object is selected again.
    let owner = s.doc().ok().and_then(|d| crate::board::owner_of_text(d, te.id));
    let empty = owner.is_none() && text_of(s, te.id).map(|t| t.characters.trim().is_empty()).unwrap_or(false);
    if empty {
        let doc = s.doc_mut()?;
        let _ = doc.remove(te.id);
        s.ds_mut()?.selection.clear();
    }
    if empty && te.created {
        // Nothing to undo for a text box that was never filled. The gesture snapshot was taken
        // after the layer was added, so it still holds it: re-base it on the document without the
        // layer, and committing then records no step (and leaves the redo stack alone).
        // A snapshot taken before the layer was added (the Text on a path tool, which also took
        // the path layer away) is put back whole instead.
        let ds = s.ds_mut()?;
        let doc = ds.doc.clone();
        if ds.interaction.as_ref().is_some_and(|it| it.doc.node(te.id).is_none()) {
            s.cancel_interaction()?;
        } else if let Some(it) = ds.interaction.as_mut() {
            it.doc = doc;
        }
    }
    s.commit_interaction()?;
    let ds = s.ds_mut()?;
    if let Some(o) = owner {
        ds.selection = vec![o];
    }
    ds.revision += 1;
    Ok(())
}

fn range(te: &TextEdit) -> (usize, usize) {
    (te.caret.min(te.anchor), te.caret.max(te.anchor))
}

/// Apply an unconditional text-model mutation through `edit_if`, whose success clears the
/// composition, clamps caret and anchor to the character count, marks a `characters` override
/// when the node is source-backed, normalizes the document and bumps the revision.
fn edit(s: &mut Session, f: impl FnOnce(&mut TextData, &mut TextEdit)) -> Result<()> {
    edit_if(s, |t, te| {
        f(t, te);
        true
    })
    .map(|_| ())
}

/// [`edit`] for a mutation that can still turn out to be a no-op.
///
/// The closure returns whether it changed the model; return `false` only before mutating (the
/// text is handed over directly). A no-op leaves the composition (`preedit`), the revision and the
/// document untouched, so an IME deletion that finds nothing to delete does not cancel an
/// in-progress composition.
fn edit_if(s: &mut Session, f: impl FnOnce(&mut TextData, &mut TextEdit) -> bool) -> Result<bool> {
    let ds = s.ds_mut()?;
    let Some(mut te) = ds.text_edit.clone() else { return Err(EngineError::Other("not editing text".into())) };
    let before = ds.doc.clone();
    let doc = ds.doc_mut();
    let n = doc.get_mut(te.id)?;
    let NodeKind::Text(t) = &mut n.kind else { return Err(EngineError::Other("not a text layer".into())) };
    if !f(t, &mut te) {
        return Ok(false);
    }
    // Any edit ends a composition (a commit replaces it with the final text).
    te.preedit = None;
    let len = t.char_count();
    te.caret = te.caret.min(len);
    te.anchor = te.anchor.min(len);
    if n.source.is_some() {
        n.overrides.insert("characters".into());
    }
    crate::normalize(doc, &before);
    ds.text_edit = Some(te);
    ds.revision += 1;
    Ok(true)
}

/// What an accepted IME `Commit` did to the committed model.
///
/// An IME receipt may label the snapshot `InputMethod` only for [`Self::Plain`]. AutoFormat
/// (typing "- " at a line start turns the paragraph into a list) is an application
/// transformation in the same command, so the frame's change must be reported as `Other`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImeInsertion {
    /// Nothing was inserted (an empty, control-only, or bare-newline commit).
    Nothing,
    /// The committed text replaced the selection exactly.
    Plain,
    /// The editor transformed the commit before applying it: paragraph AutoFormat, or line-ending
    /// normalisation/control-character removal. The frame's change is application-origin.
    Transformed,
}

/// Type (or paste) `text` replacing the selection. Line endings are normalised and other control
/// characters dropped, so pasted plain text from any platform lands cleanly.
pub fn insert(s: &mut Session, text: &str) -> Result<()> {
    insert_model(s, text, false).map(|_| ())
}

/// End the composition without changing the document, for an empty IME commit.
///
/// Returns without touching the session when no composition is active (or nothing is being
/// edited): an empty commit is otherwise a no-op, never an error.
fn clear_composition(s: &mut Session) -> Result<()> {
    let composing = s.ds().ok().and_then(|d| d.text_edit.as_ref()).is_some_and(|te| te.preedit.is_some());
    if !composing {
        return Ok(());
    }
    let ds = s.ds_mut()?;
    if let Some(te) = ds.text_edit.as_mut() {
        te.preedit = None;
    }
    ds.revision += 1;
    Ok(())
}

/// Apply an IME `Commit`, reporting whether it was a plain insertion.
///
/// The UI calls this instead of [`insert`] so only a commit that the editor applied as a plain
/// insertion can earn an `InputMethod` receipt. AutoFormat, line-ending normalisation and
/// control-character removal are application transformations and are reported as
/// [`ImeInsertion::Transformed`]. Like stock `TextEdit`, a bare `"\n"`/`"\r"` commit is ignored.
pub fn insert_ime_commit(s: &mut Session, text: &str) -> Result<ImeInsertion> {
    insert_model(s, text, true)
}

/// Insert committed text, optionally as an IME commit: normalize line endings, drop control
/// characters except `\n`/`\t`, ignore a bare newline/CR IME commit, clear an active
/// composition for an empty or control-only IME commit, and turn a committed space after a
/// line-start `-`, `*` or `1.` into a bulleted/numbered list. Reports whether the insertion was
/// plain, transformed, or a no-op.
fn insert_model(s: &mut Session, text: &str, ime_commit: bool) -> Result<ImeInsertion> {
    // Stock `TextEdit` deliberately ignores these two commits (they are not a mutation); only a
    // real commit may do so, since typing Enter goes through `insert`.
    if ime_commit && matches!(text, "\n" | "\r") {
        return Ok(ImeInsertion::Nothing);
    }
    let normalized: String =
        crate::cmd::props::normalize_line_endings(text).chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect();
    if normalized.is_empty() {
        // An empty commit is how some integrations (Safari's egui web integration when the last
        // composition character is deleted) end the composition. It inserts nothing, but it must
        // still clear the pre-edit; stock `TextEdit` does the same in its non-ignored `Commit` arm.
        if ime_commit {
            clear_composition(s)?;
        }
        return Ok(ImeInsertion::Nothing);
    }
    // A commit the editor had to transform is an application edit, not a plain insertion.
    let mut outcome = if normalized == text { ImeInsertion::Plain } else { ImeInsertion::Transformed };
    let text = normalized;
    edit(s, |t, te| {
        let (a, b) = range(te);
        // "- ", "* " or "1. " typed at the start of a plain paragraph starts a list.
        if text == " " && a == b && t.para_list(t.para_of(a)).list == ListStyle::None {
            let chars: Vec<char> = t.characters.chars().collect();
            let start = line_start(&chars, a);
            let typed: String = chars.get(start..a).unwrap_or(&[]).iter().collect();
            let list = match typed.as_str() {
                "-" | "*" => ListStyle::Bulleted,
                "1." => ListStyle::Numbered,
                _ => ListStyle::None,
            };
            if list != ListStyle::None {
                t.replace_range(start, a, "");
                t.set_para_lists(start, start, |pl| pl.list = list);
                (te.caret, te.anchor) = (start, start);
                outcome = ImeInsertion::Transformed;
                return;
            }
        }
        t.replace_range(a, b, &text);
        te.caret = a + text.chars().count();
        te.anchor = te.caret;
    })?;
    Ok(outcome)
}

/// List keys: Tab nests the selected list items a level deeper; Shift+Tab, Backspace at an item's
/// start and Enter on an empty item move it a level out (at the top: out of the list). Returns
/// whether the key was taken.
fn list_key(s: &mut Session, key: &str, shift: bool) -> Result<bool> {
    let Some(te) = s.ds()?.text_edit.clone() else { return Ok(false) };
    let t = text_of(s, te.id)?;
    let (a, b) = range(&te);
    let chars: Vec<char> = t.characters.chars().collect();
    let at_start = a == b && line_start(&chars, a) == a;
    let item = t.para_list(t.para_of(a)).list != ListStyle::None;
    let (pa, pb) = t.para_span(a, b);
    let any_item = (pa..=pb).any(|p| t.para_list(p).list != ListStyle::None);
    let deeper = match key {
        "Tab" if any_item => !shift,
        "Backspace" if item && at_start => false,
        "Enter" if item && at_start && chars.get(a).is_none_or(|c| *c == '\n') => false,
        _ => return Ok(false),
    };
    edit(s, |t, _| {
        t.set_para_lists(a, b, |pl| match pl.list {
            ListStyle::None => {}
            _ if deeper => pl.indent = pl.indent.saturating_add(1),
            _ if pl.indent > 0 => pl.indent -= 1,
            _ => pl.list = ListStyle::None,
        })
    })?;
    Ok(true)
}

/// Cmd/Ctrl+] and Cmd/Ctrl+[ while editing: the selected list items a level deeper or out, like
/// Tab and Shift+Tab. Returns whether anything changed (text that isn't a list and items already
/// at the deepest level stay as they are).
pub fn list_indent(s: &mut Session, out: bool) -> Result<bool> {
    let Some(id) = s.ds()?.text_edit.as_ref().map(|te| te.id) else { return Ok(false) };
    let before = text_of(s, id)?;
    if !list_key(s, "Tab", out)? {
        return Ok(false);
    }
    Ok(text_of(s, id)? != before)
}

/// The kind of word char `i` belongs to: 0 for spaces and punctuation, else its script, so a word
/// ends where the script changes (Han, hiragana, katakana and Hangul runs are words of their own in
/// text without spaces). Combining marks and voicing marks belong to the char before them.
fn word_class(chars: &[char], mut i: usize) -> u8 {
    while i > 0
        && chars.get(i).is_some_and(
            |c| matches!(*c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0x3099..=0x309A | 0xFE20..=0xFE2F),
        )
    {
        i -= 1;
    }
    match chars.get(i).copied() {
        Some(
            '\u{3005}'
            | '\u{3006}'
            | '\u{3007}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{3FFFF}',
        ) => 2,
        Some('\u{3041}'..='\u{309F}') => 3,
        Some('\u{30A0}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}') => 4,
        Some('\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{AC00}'..='\u{D7AF}') => 5,
        Some(c) if c.is_alphanumeric() || c == '_' => 1,
        _ => 0,
    }
}

fn is_word(chars: &[char], i: usize) -> bool {
    word_class(chars, i) != 0
}

/// The start of the word before `i` (skipping spaces and punctuation first).
fn word_left(chars: &[char], mut i: usize) -> usize {
    while i > 0 && !is_word(chars, i - 1) {
        i -= 1;
    }
    let class = word_class(chars, i.saturating_sub(1));
    while i > 0 && word_class(chars, i - 1) == class && class != 0 {
        i -= 1;
    }
    i
}

/// The end of the word after `i` (skipping spaces and punctuation first).
fn word_right(chars: &[char], mut i: usize) -> usize {
    let n = chars.len();
    while i < n && !is_word(chars, i) {
        i += 1;
    }
    let class = word_class(chars, i);
    while i < n && word_class(chars, i) == class && class != 0 {
        i += 1;
    }
    i
}

/// A named key: Backspace, Delete, Left, Right, Up, Down, Home, End, Enter, Escape, SelectAll.
/// `word` moves or deletes by word, `line` to the line's ends (Up/Down, Home/End: the text's ends);
/// [`crate::keymap::text_motion`] gives them for the platform's keys. `ArrowLeft` etc. also work.
pub fn key(s: &mut Session, key: &str, shift: bool, word: bool, line: bool) -> Result<()> {
    let key = key.strip_prefix("Arrow").unwrap_or(key);
    let Some(te) = s.ds()?.text_edit.clone() else { return Err(EngineError::Other("not editing text".into())) };
    let t = text_of(s, te.id)?;
    let chars: Vec<char> = t.characters.chars().collect();
    let n = chars.len();
    if matches!(key, "Tab" | "Backspace" | "Enter") && !word && !line && list_key(s, key, shift)? {
        return Ok(());
    }
    match key {
        "Escape" => return end(s),
        "Enter" => return insert(s, "\n"),
        "Tab" => return insert(s, "\t"),
        "Backspace" | "Delete" => {
            return edit(s, |t, te| {
                let (a, b) = range(te);
                let (a, b) = if a != b {
                    (a, b)
                } else if key == "Backspace" {
                    let from = if line {
                        line_start(&chars, a)
                    } else if word {
                        word_left(&chars, a)
                    } else {
                        figcraft_text::backspace_step(&t.characters, a)
                    };
                    (from, a)
                } else {
                    let to = if line {
                        line_end(&chars, a)
                    } else if word {
                        word_right(&chars, a)
                    } else {
                        figcraft_text::grapheme_step(&t.characters, a, true).min(n)
                    };
                    (a, to)
                };
                t.replace_range(a, b, "");
                te.caret = a;
                te.anchor = a;
            });
        }
        _ => {}
    }
    // Caret movement.
    let layout = {
        let doc = s.doc()?;
        let node = doc.get(te.id)?;
        figcraft_text::layout_node(node).map(|(l, _)| l)
    };
    let (a, b) = range(&te);
    // Without Shift, a selection collapses first: plain arrows stop at its edge, word/line moves
    // continue from that edge.
    let cur = match key {
        "Left" | "Up" | "Home" if !shift && a != b => a,
        "Right" | "Down" | "End" if !shift && a != b => b,
        _ => te.caret,
    };
    let next = match key {
        "Left" if !shift && a != b && !word && !line => a,
        "Right" if !shift && a != b && !word && !line => b,
        "Left" if line => line_start(&chars, cur),
        "Right" if line => line_end(&chars, cur),
        "Left" if word => word_left(&chars, cur),
        "Right" if word => word_right(&chars, cur),
        // Plain arrows move on screen through right-to-left runs, by char elsewhere.
        "Left" | "Right" => match layout.as_ref().and_then(|l| l.visual_step(cur, key == "Right")) {
            Some(i) => i.min(n),
            None => figcraft_text::grapheme_step(&t.characters, cur, key == "Right").min(n),
        },
        "Home" if line => 0,
        "End" if line => n,
        "Home" => line_start(&chars, cur),
        "End" => line_end(&chars, cur),
        "Up" | "Down" => match &layout {
            _ if line => {
                if key == "Up" {
                    0
                } else {
                    n
                }
            }
            Some(l) => {
                let (x, li) = l.caret(cur);
                let target = if key == "Up" { li.checked_sub(1) } else { Some(li + 1).filter(|k| *k < l.lines.len()) };
                match target.and_then(|k| l.lines.get(k)) {
                    Some(line) => l.hit(x, line.top + line.height / 2.0),
                    None => {
                        if key == "Up" {
                            0
                        } else {
                            n
                        }
                    }
                }
            }
            None => cur,
        },
        "SelectAll" => return select(s, 0, n),
        _ => return Ok(()),
    };
    let anchor = if shift { te.anchor } else { next };
    select(s, anchor, next)
}

/// Set the selection (`anchor`..`caret`, clamped to the text). Moving the caret abandons any
/// composition in progress.
fn select(s: &mut Session, anchor: usize, caret: usize) -> Result<()> {
    let Some(id) = s.ds()?.text_edit.as_ref().map(|te| te.id) else { return Ok(()) };
    let len = s.doc()?.node(id).and_then(|n| n.kind.text()).map_or(0, TextData::char_count);
    let ds = s.ds_mut()?;
    if let Some(te) = ds.text_edit.as_mut() {
        te.anchor = anchor.min(len);
        te.caret = caret.min(len);
        te.preedit = None;
    }
    ds.revision += 1;
    Ok(())
}

/// Delete the text surrounding the caret/selection that an IME reports with
/// [`egui::ImeEvent::DeleteSurroundingBytes`]: `before_bytes` UTF-8 bytes end where the selection
/// starts and `after_bytes` start where it ends. As in stock `TextEdit`, the selected text itself
/// is kept for the paired `Commit`; only whole characters covered by the byte counts beyond the
/// selection are removed (a character the range only partly covers stays). Returns whether
/// anything was deleted.
pub fn delete_surrounding_bytes(s: &mut Session, before_bytes: usize, after_bytes: usize) -> Result<bool> {
    let Some(te) = s.ds()?.text_edit.clone() else { return Ok(false) };
    let t = text_of(s, te.id)?;
    let (a, b) = range(&te);
    let (before, after) = surrounding_chars(&t.characters, a, b, before_bytes, after_bytes);
    if before == 0 && after == 0 {
        return Ok(false);
    }
    edit_if(s, |t, te| {
        let (a, b) = range(te);
        let (before, after) = surrounding_chars(&t.characters, a, b, before_bytes, after_bytes);
        if before == 0 && after == 0 {
            return false;
        }
        // Delete after the selection first so the before range's indices stay valid.
        if after > 0 {
            t.replace_range(b, b + after, "");
        }
        if before > 0 {
            t.replace_range(a - before, a, "");
        }
        // Keep the selection: both ends move left by the deleted prefix, reversed or not.
        te.caret = te.caret.saturating_sub(before);
        te.anchor = te.anchor.saturating_sub(before);
        true
    })
}

/// Delete surrounding text counted in characters (the older `DeleteSurrounding` event). The
/// selection is kept, as in stock `TextEdit`; only text before and after it is removed.
pub fn delete_surrounding_chars(s: &mut Session, before_chars: usize, after_chars: usize) -> Result<bool> {
    let Some(te) = s.ds()?.text_edit.clone() else { return Ok(false) };
    let t = text_of(s, te.id)?;
    let (a, b) = range(&te);
    let (_, b_byte) = char_byte_offsets(&t.characters, a, b);
    let before = before_chars.min(a);
    let after = t.characters.get(b_byte..).map_or(0, |rest| rest.chars().take(after_chars).count());
    if before == 0 && after == 0 {
        return Ok(false);
    }
    edit_if(s, |t, te| {
        let (a, b) = range(te);
        let (_, b_byte) = char_byte_offsets(&t.characters, a, b);
        let before = before_chars.min(a);
        let after = t.characters.get(b_byte..).map_or(0, |rest| rest.chars().take(after_chars).count());
        if before == 0 && after == 0 {
            return false;
        }
        if after > 0 {
            t.replace_range(b, b + after, "");
        }
        if before > 0 {
            t.replace_range(a - before, a, "");
        }
        te.caret = te.caret.saturating_sub(before);
        te.anchor = te.anchor.saturating_sub(before);
        true
    })
}

/// The UTF-8 byte offsets of character indices `a` and `b` in one forward pass.
///
/// An index at or past the end maps to `text.len()`. Walking once (instead of twice) matters on
/// every IME event over a large text layer; the walk is still O(max(a, b)) characters, and callers
/// that need the end still pay O(n). See [`ime_surrounding`].
fn char_byte_offsets(text: &str, a: usize, b: usize) -> (usize, usize) {
    let (mut a_byte, mut b_byte) = (None, None);
    for (i, (byte, _)) in text.char_indices().enumerate() {
        if a_byte.is_none() && i == a {
            a_byte = Some(byte);
        }
        if b_byte.is_none() && i == b {
            b_byte = Some(byte);
        }
        if a_byte.is_some() && b_byte.is_some() {
            break;
        }
    }
    (a_byte.unwrap_or(text.len()), b_byte.unwrap_or(text.len()))
}

/// `(before, after)` characters covered by the IME's byte counts around the selection `[a, b)`
/// (character indices): mirror of egui's own `surrounding_bytes_to_chars`.
fn surrounding_chars(text: &str, a: usize, b: usize, before_bytes: usize, after_bytes: usize) -> (usize, usize) {
    let (a_byte, b_byte) = char_byte_offsets(text, a, b);
    let from = a_byte.saturating_sub(before_bytes);
    let before = text[..a_byte].char_indices().rev().take_while(|(i, _)| *i >= from).count();
    let after = text[b_byte..].char_indices().take_while(|(i, c)| i + c.len_utf8() <= after_bytes).count();
    (before, after)
}

/// The committed text around the IME caret for the outgoing surrounding-text snapshot.
///
/// This is the edited node's logical model text (never the bidi display order), with the
/// pre-edit excluded because a composition is UI-only in this editor until it is committed.
/// Character caret/anchor indices become independent UTF-8 byte offsets, so a reversed
/// selection stays reversed. At most [`MAX_IME_SURROUNDING_BYTES`] bytes are copied: the window
/// is centred on the selection and rounded inward to character boundaries. An empty text layer
/// is `Ok` with `text == ""` and both offsets zero; the result is [`ImeSurroundingUnavailable`]
/// only when there is no text edit, the target is not a text node, or the selection alone is
/// wider than the budget.
///
/// Cost: one character walk up to the later of the two cursor indices (to turn char indices into
/// UTF-8 bytes) plus the bounded copy. The caret is a character index, so reaching a caret near
/// the end of a very large layer is still O(n); the UI calls this once per pass and twice around
/// each text-affecting event (`ime_stamp`), so such a layer pays O(caret/anchor) per event. The
/// document is never cloned here.
pub fn ime_surrounding(s: &Session) -> std::result::Result<ImeSurrounding, ImeSurroundingUnavailable> {
    let ds = s.ds().map_err(|_| ImeSurroundingUnavailable::NotEditing)?;
    let te = ds.text_edit.as_ref().ok_or(ImeSurroundingUnavailable::NotEditing)?;
    let node = ds.doc.node(te.id).ok_or(ImeSurroundingUnavailable::NotText)?;
    let NodeKind::Text(t) = &node.kind else { return Err(ImeSurroundingUnavailable::NotText) };
    let text = t.characters.as_str();
    // One pass for both offsets (the caret and anchor are already clamped by `edit`/`select`).
    let (cursor, anchor) = char_byte_offsets(text, te.caret, te.anchor);
    let (selection_start, selection_end) = (cursor.min(anchor), cursor.max(anchor));
    let selection_len = selection_end - selection_start;
    if selection_len > MAX_IME_SURROUNDING_BYTES {
        return Err(ImeSurroundingUnavailable::SelectionTooWide);
    }
    // Spend the budget on both sides, giving room unused on one side to the other.
    let budget = MAX_IME_SURROUNDING_BYTES - selection_len;
    let before_available = selection_start;
    let after_available = text.len() - selection_end;
    let mut before = (budget / 2).min(before_available);
    let mut after = (budget - before).min(after_available);
    before = (budget - after).min(before_available);
    after = (budget - before).min(after_available);
    let mut window_start = selection_start - before;
    let mut window_end = selection_end + after;
    // Round inward so the window stays within the byte budget and on character boundaries.
    while window_start < window_end && !text.is_char_boundary(window_start) {
        window_start += 1;
    }
    while window_end > window_start && !text.is_char_boundary(window_end) {
        window_end -= 1;
    }
    let Some(window) = text.get(window_start..window_end) else {
        return Err(ImeSurroundingUnavailable::InvalidWindow);
    };
    Ok(ImeSurrounding {
        owner: ImeEditOwner { document: ds.uid, node: te.id, editing_session: te.editing_session },
        text: window.to_owned(),
        cursor: cursor.saturating_sub(window_start),
        anchor: anchor.saturating_sub(window_start),
    })
}

/// Show an IME composition inline at the caret (`cursor` = the IME's cursor inside it, in chars;
/// default its end). The document doesn't change until the composition is committed with
/// [`insert`]; an empty `text` cancels it. Starting to compose over a selection deletes it, as
/// typing would.
pub fn preedit(s: &mut Session, text: &str, cursor: Option<usize>) -> Result<()> {
    let Some(te) = s.ds()?.text_edit.clone() else { return Err(EngineError::Other("not editing text".into())) };
    let text: String = text.chars().filter(|c| !c.is_control()).take(MAX_PREEDIT).collect();
    if text.is_empty() && te.preedit.is_none() {
        return Ok(()); // IMEs send empty compositions when idle
    }
    if !text.is_empty() && te.preedit.is_none() && te.caret != te.anchor {
        edit(s, |t, te| {
            let (a, b) = range(te);
            t.replace_range(a, b, "");
            te.caret = a;
            te.anchor = a;
        })?;
    }
    let n = text.chars().count();
    let ds = s.ds_mut()?;
    if let Some(te) = ds.text_edit.as_mut() {
        te.preedit = (n > 0).then(|| Preedit { text, cursor: cursor.unwrap_or(n).min(n) });
    }
    ds.revision += 1;
    Ok(())
}

/// Longest composition shown (IMEs compose a phrase at a time).
const MAX_PREEDIT: usize = 1000;

/// The document as it looks while composing: the composition inserted at the caret, with text
/// auto-size and layout applied. Also returns the composition's char range and the IME cursor's
/// index in that document. `None` when nothing is being composed.
pub fn preedit_doc(s: &Session) -> Option<(Document, (usize, usize), usize)> {
    let ds = s.ds().ok()?;
    let te = ds.text_edit.as_ref()?;
    let pe = te.preedit.as_ref()?;
    let mut doc = (*ds.doc).clone();
    let NodeKind::Text(t) = &mut doc.get_mut(te.id).ok()?.kind else { return None };
    let at = te.caret.min(t.char_count());
    t.replace_range(at, at, &pe.text);
    crate::normalize(&mut doc, &ds.doc);
    let n = pe.text.chars().count();
    Some((doc, (at, at + n), at + pe.cursor.min(n)))
}

fn line_start(chars: &[char], mut i: usize) -> usize {
    while i > 0 && chars.get(i - 1).is_some_and(|c| *c != '\n') {
        i -= 1;
    }
    i
}

fn line_end(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars.get(i).is_some_and(|c| *c != '\n') {
        i += 1;
    }
    i
}

/// Place the caret (and optionally extend the selection) at char `index`.
pub fn set_caret(s: &mut Session, index: usize, extend: bool) -> Result<()> {
    let Some(te) = s.ds()?.text_edit.clone() else { return Ok(()) };
    select(s, if extend { te.anchor } else { index }, index)
}

/// Select the word at `index` (double-click).
pub fn select_word(s: &mut Session, index: usize) -> Result<()> {
    let Some(te) = s.ds()?.text_edit.clone() else { return Ok(()) };
    let chars: Vec<char> = text_of(s, te.id)?.characters.chars().collect();
    let mut a = index.min(chars.len());
    // Double-clicking just after a word's last char (or at the end) selects that word.
    if a > 0 && !is_word(&chars, a) && is_word(&chars, a - 1) {
        a -= 1;
    }
    let mut b = a;
    // Words by script (see `word_class`), then runs of spaces (10) and of punctuation (11).
    let class = |i: usize| match word_class(&chars, i) {
        0 if chars.get(i).is_some_and(|c| c.is_whitespace()) => 10,
        0 => 11,
        k => k,
    };
    // A word, a run of spaces or a run of punctuation, like the incumbent; newlines stay unselected.
    if let Some(k) = chars.get(a).filter(|c| **c != '\n').map(|_| class(a)) {
        while a > 0 && chars.get(a - 1).is_some_and(|c| *c != '\n') && class(a - 1) == k {
            a -= 1;
        }
        while b < chars.len() && chars.get(b).is_some_and(|c| *c != '\n') && class(b) == k {
            b += 1;
        }
    }
    select(s, a, b)
}

/// Select the line (paragraph) containing `index` (triple-click).
pub fn select_line(s: &mut Session, index: usize) -> Result<()> {
    let Some(te) = s.ds()?.text_edit.clone() else { return Ok(()) };
    let chars: Vec<char> = text_of(s, te.id)?.characters.chars().collect();
    let i = index.min(chars.len());
    select(s, line_start(&chars, i), line_end(&chars, i))
}

/// The selected text while editing (for copy).
pub fn selected_text(s: &Session) -> Option<String> {
    let (id, a, b) = selected_range(s)?;
    let doc = s.doc().ok()?;
    let t = doc.node(id)?.kind.text()?;
    Some(t.characters.chars().skip(a).take(b.saturating_sub(a)).collect())
}

/// The selected range while editing (for styling commands), or None.
pub fn selected_range(s: &Session) -> Option<(NodeId, usize, usize)> {
    let te = s.ds().ok()?.text_edit.clone()?;
    let (a, b) = range(&te);
    Some((te.id, a, b))
}

#[cfg(test)]
mod tests;
