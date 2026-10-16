//! On-canvas text editing: caret, selection, typing. A whole editing session is one undo step.

use figcraft_doc::{Document, ListStyle, NodeId, NodeKind, TextData};

use crate::{EngineError, Preedit, Result, Session, TextEdit};

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
    let (caret, anchor) = match caret {
        Some(c) => (c.min(n), c.min(n)),
        None => (n, 0),
    };
    ds.text_edit = Some(TextEdit { id, caret, anchor, created, preedit: None });
    ds.selection = vec![id];
    ds.revision += 1;
    s.live_text_notice(id);
    Ok(())
}

/// Finish editing: an empty new text layer is removed; the session becomes one undo step.
pub fn end(s: &mut Session) -> Result<()> {
    let Some(te) = s.ds()?.text_edit.clone() else { return Ok(()) };
    s.ds_mut()?.text_edit = None;
    // A widget's editable text isn't the user's to change: what was typed goes to the widget's
    // plugin, which redraws it (one undo step), and the layer itself is put back.
    if let Some(input) = crate::widgets::input_of(s.doc()?, te.id) {
        let typed = text_of(s, te.id)?.characters;
        s.cancel_interaction()?;
        let changed = text_of(s, te.id).is_ok_and(|was| was.characters != typed);
        let widget = crate::widgets::widget_of(s.doc()?, te.id);
        if changed && let Err(e) = s.execute("widget.setText", &serde_json::json!({"id": te.id, "input": input.name, "text": typed})) {
            s.message = Some(e.to_string());
        }
        let ds = s.ds_mut()?;
        ds.selection = widget.into_iter().collect();
        ds.revision += 1;
        return Ok(());
    }
    let empty = text_of(s, te.id).map(|t| t.characters.trim().is_empty()).unwrap_or(false);
    if empty {
        let doc = s.doc_mut()?;
        let _ = doc.remove(te.id);
        s.ds_mut()?.selection.clear();
    }
    if empty && te.created {
        // Nothing to undo for a text box that was never filled. The gesture snapshot was taken
        // after the layer was added, so it still holds it: re-base it on the document without the
        // layer, and committing then records no step (and leaves the redo stack alone).
        let ds = s.ds_mut()?;
        let doc = ds.doc.clone();
        if let Some(it) = ds.interaction.as_mut() {
            it.doc = doc;
        }
    }
    s.commit_interaction()?;
    let ds = s.ds_mut()?;
    ds.revision += 1;
    Ok(())
}

fn range(te: &TextEdit) -> (usize, usize) {
    (te.caret.min(te.anchor), te.caret.max(te.anchor))
}

fn edit(s: &mut Session, f: impl FnOnce(&mut TextData, &mut TextEdit)) -> Result<()> {
    let ds = s.ds_mut()?;
    let Some(mut te) = ds.text_edit.clone() else { return Err(EngineError::Other("not editing text".into())) };
    let before = ds.doc.clone();
    let doc = ds.doc_mut();
    let n = doc.get_mut(te.id)?;
    let NodeKind::Text(t) = &mut n.kind else { return Err(EngineError::Other("not a text layer".into())) };
    f(t, &mut te);
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
    Ok(())
}

/// Type (or paste) `text` replacing the selection. Line endings are normalised and other control
/// characters dropped, so pasted plain text from any platform lands cleanly.
pub fn insert(s: &mut Session, text: &str) -> Result<()> {
    let text: String = crate::cmd::props::normalize_line_endings(text).chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect();
    // A widget's editable text takes what its plugin allows: one line unless it's multiline, and
    // no more characters than its maximum.
    let input = s.ds()?.text_edit.as_ref().and_then(|te| crate::widgets::input_of(&s.ds().ok()?.doc, te.id));
    let text = match &input {
        Some(input) => {
            let te = s.ds()?.text_edit.clone();
            let (kept, replaced) = te.map_or((0, 0), |te| (text_of(s, te.id).map_or(0, |t| t.char_count()), range(&te).1 - range(&te).0));
            input.clean(&text).chars().take(input.max.saturating_sub(kept.saturating_sub(replaced))).collect()
        }
        None => text,
    };
    if text.is_empty() {
        return Ok(());
    }
    edit(s, |t, te| {
        let (a, b) = range(te);
        // "- ", "* " or "1. " typed at the start of a plain paragraph starts a list.
        if input.is_none() && text == " " && a == b && t.para_list(t.para_of(a)).list == ListStyle::None {
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
                return;
            }
        }
        t.replace_range(a, b, &text);
        te.caret = a + text.chars().count();
        te.anchor = te.caret;
    })
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
    // In a widget's one-line text, Enter and Tab commit.
    let input = crate::widgets::input_of(s.doc()?, te.id);
    if matches!(key, "Enter" | "Tab") && input.as_ref().is_some_and(|i| !i.multiline || key == "Tab") {
        return end(s);
    }
    if input.is_none() && matches!(key, "Tab" | "Backspace" | "Enter") && !word && !line && list_key(s, key, shift)? {
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
                        figcraft_text::grapheme_step(&t.characters, a, false)
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
mod tests {
    use super::*;
    use crate::tools::{PointerEvent, PointerKind};
    use figcraft_doc::{ListStyle, TextAutoResize, TextCase, TextDecoration};
    use serde_json::json;

    fn setup(text: &str) -> (Session, NodeId) {
        let mut s = Session::new();
        let r = s.execute("text.create", &json!({"text": text, "x": 0, "y": 0, "props": {"fontSize": 24}})).unwrap();
        let id = crate::params::id_of(&r["id"]).unwrap();
        (s, id)
    }

    fn data(s: &Session, id: NodeId) -> TextData {
        s.doc().unwrap().node(id).unwrap().kind.text().unwrap().clone()
    }

    fn sel(s: &Session) -> (usize, usize) {
        let te = s.ds().unwrap().text_edit.clone().unwrap();
        (te.anchor, te.caret)
    }

    #[test]
    fn whole_editing_session_is_one_undo_step() {
        let (mut s, id) = setup("Hello");
        let undo_before = s.ds().unwrap().history.undo.len();
        begin(&mut s, id, Some(5), false).unwrap();
        insert(&mut s, " big").unwrap();
        insert(&mut s, " world").unwrap();
        key(&mut s, "Backspace", false, false, false).unwrap();
        // Styling a range mid-session joins the same step.
        set_caret(&mut s, 0, false).unwrap();
        set_caret(&mut s, 5, true).unwrap();
        s.execute("text.bold", &json!({})).unwrap();
        s.execute("text.alignCenter", &json!({})).unwrap();
        end(&mut s).unwrap();
        let t = data(&s, id);
        assert_eq!(t.characters, "Hello big worl");
        assert_eq!(t.style_at(0).font_style, "Bold");
        assert_eq!(t.style_at(6).font_style, "Regular");
        assert_eq!(s.ds().unwrap().history.undo.len(), undo_before + 1);
        s.undo().unwrap();
        let t = data(&s, id);
        assert_eq!(t.characters, "Hello");
        assert!(t.runs.is_empty());
        assert_eq!(t.align_h, figcraft_doc::TextAlignH::Left);
    }

    #[test]
    fn typing_replaces_selection_and_paste_is_plain() {
        let (mut s, id) = setup("Hello world");
        begin(&mut s, id, None, false).unwrap();
        assert_eq!(sel(&s), (0, 11), "begin without caret selects all");
        select_word(&mut s, 7).unwrap();
        assert_eq!(sel(&s), (6, 11));
        insert(&mut s, "there").unwrap();
        assert_eq!(data(&s, id).characters, "Hello there");
        assert_eq!(sel(&s), (11, 11));
        // Pasted text: CRLF / CR / Unicode separators become newlines, other controls are dropped.
        insert(&mut s, "\r\na\rb\u{2029}c\u{0}\u{7}\td").unwrap();
        assert_eq!(data(&s, id).characters, "Hello there\na\nb\nc\td");
        insert(&mut s, "\u{1}").unwrap();
        assert_eq!(data(&s, id).characters, "Hello there\na\nb\nc\td");
        // Select the line (paragraph) then type over it.
        select_line(&mut s, 0).unwrap();
        assert_eq!(sel(&s), (0, 11));
        insert(&mut s, "Hi").unwrap();
        assert_eq!(data(&s, id).characters, "Hi\na\nb\nc\td");
        assert_eq!(selected_text(&s).as_deref(), Some(""));
        end(&mut s).unwrap();
    }

    #[test]
    fn word_selection_classes() {
        let (mut s, id) = setup("foo,  bar_baz!\nnext");
        begin(&mut s, id, Some(0), false).unwrap();
        select_word(&mut s, 1).unwrap();
        assert_eq!(sel(&s), (0, 3));
        // Right after a word's last char selects the word.
        select_word(&mut s, 13).unwrap();
        assert_eq!(sel(&s), (6, 13));
        // A run of spaces.
        select_word(&mut s, 4).unwrap();
        assert_eq!(sel(&s), (4, 6));
        select_line(&mut s, 16).unwrap();
        assert_eq!(sel(&s), (15, 19));
        select_line(&mut s, 999).unwrap();
        assert_eq!(sel(&s), (15, 19));
        select_word(&mut s, 999).unwrap();
        assert_eq!(sel(&s), (15, 19));
        set_caret(&mut s, 999, false).unwrap();
        assert_eq!(sel(&s), (19, 19), "carets clamp to the text");
    }

    #[test]
    fn keyboard_navigation_and_extension() {
        let (mut s, id) = setup("one two\nthree");
        begin(&mut s, id, Some(0), false).unwrap();
        key(&mut s, "Right", false, true, false).unwrap(); // Alt+Right: end of word
        assert_eq!(sel(&s), (3, 3));
        key(&mut s, "Right", true, true, false).unwrap(); // Shift+Alt+Right extends
        assert_eq!(sel(&s), (3, 7));
        key(&mut s, "Left", false, false, false).unwrap(); // collapses to the start
        assert_eq!(sel(&s), (3, 3));
        key(&mut s, "Right", false, false, true).unwrap(); // Cmd+Right: line end
        assert_eq!(sel(&s), (7, 7));
        key(&mut s, "Down", true, false, true).unwrap(); // Shift+Cmd+Down: to the end
        assert_eq!(sel(&s), (7, 13));
        key(&mut s, "Left", false, false, true).unwrap(); // Cmd+Left from the collapsed start
        assert_eq!(sel(&s), (0, 0));
        key(&mut s, "Delete", false, true, false).unwrap(); // Alt+Delete: word forward
        assert_eq!(data(&s, id).characters, " two\nthree");
        key(&mut s, "SelectAll", false, false, false).unwrap();
        assert_eq!(sel(&s), (0, 10));
        end(&mut s).unwrap();
    }

    fn ev(kind: PointerKind, x: f64, y: f64, shift: bool) -> PointerEvent {
        let mut e = PointerEvent::at(kind, x, y, 1.0);
        e.shift = shift;
        e
    }

    #[test]
    fn pointer_double_click_drag_and_shift_click() {
        let (mut s, id) = setup("Hello world");
        let Some((l, _)) = figcraft_text::layout_node(s.doc().unwrap().node(id).unwrap()) else { return };
        if l.glyphs.is_empty() {
            return; // no fonts
        }
        let y = l.lines[0].top + l.lines[0].height / 2.0;
        let x_of = |i: usize| l.caret(i).0;
        // Double-click inside "world" edits with all the text selected; another one there
        // selects the word.
        let mid = (x_of(7) + x_of(8)) / 2.0;
        s.pointer(&ev(PointerKind::Down, mid, y, false)).unwrap();
        s.pointer(&ev(PointerKind::Up, mid, y, false)).unwrap();
        s.pointer(&ev(PointerKind::DoubleClick, mid, y, false)).unwrap();
        assert_eq!(sel(&s), (0, 11));
        s.pointer(&ev(PointerKind::Down, mid, y, false)).unwrap();
        s.pointer(&ev(PointerKind::DoubleClick, mid, y, false)).unwrap();
        s.pointer(&ev(PointerKind::Up, mid, y, false)).unwrap();
        assert_eq!(sel(&s), (6, 11));
        // Drag-select from the start of "Hello" to the middle.
        s.pointer(&ev(PointerKind::Down, x_of(0) + 0.5, y, false)).unwrap();
        assert_eq!(sel(&s), (0, 0));
        s.pointer(&ev(PointerKind::Move, (x_of(3) + x_of(4)) / 2.0 - 1.0, y, false)).unwrap();
        s.pointer(&ev(PointerKind::Up, (x_of(3) + x_of(4)) / 2.0 - 1.0, y, false)).unwrap();
        assert_eq!(sel(&s), (0, 3));
        // Shift+click extends.
        s.pointer(&ev(PointerKind::Down, x_of(9) + 0.5, y, true)).unwrap();
        s.pointer(&ev(PointerKind::Up, x_of(9) + 0.5, y, true)).unwrap();
        assert_eq!(sel(&s), (0, 9));
        insert(&mut s, "J").unwrap();
        assert_eq!(data(&s, id).characters, "Jld");
    }

    #[test]
    fn text_commands_apply_to_edited_range() {
        let (mut s, id) = setup("Hello world");
        begin(&mut s, id, Some(0), false).unwrap();
        set_caret(&mut s, 6, true).unwrap(); // "Hello "
        for (cmd, p) in [
            ("text.underline", json!({})),
            ("text.italic", json!({})),
            ("text.sizeUp", json!({})),
            ("text.trackingUp", json!({})),
            ("text.lineHeightUp", json!({})),
            ("text.caseSmallCaps", json!({})),
            ("text.link", json!({"url": "https://example.com"})),
        ] {
            s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        }
        let t = data(&s, id);
        let (a, b) = (t.style_at(0), t.style_at(8));
        assert_eq!(a.decoration, TextDecoration::Underline);
        assert!(a.italic() && !b.italic());
        assert_eq!(a.font_size, 25.0);
        assert_eq!(b.font_size, 24.0);
        assert_eq!(a.case, TextCase::SmallCaps);
        assert_eq!(a.link.as_deref(), Some("https://example.com"));
        assert!(b.link.is_none());
        // text.style with an explicit range (what the panel sends).
        s.execute("text.style", &json!({"id": id, "start": 6, "end": 11, "fontSize": 40, "textCase": "upper"})).unwrap();
        let t = data(&s, id);
        assert_eq!(t.style_at(8).font_size, 40.0);
        assert_eq!(t.style_at(8).case, TextCase::Upper);
        // Paragraph commands hit the edited node.
        s.execute("text.listStyle", &json!({"style": "numbered"})).unwrap();
        assert_eq!(data(&s, id).list, ListStyle::Numbered);
        // Every Text-menu command runs while editing.
        for item in crate::catalog::MENU.iter().filter(|i| i.id.starts_with("text.")) {
            let p = if item.id == "text.link" { json!({"url": "u"}) } else { json!({}) };
            if crate::find_command(item.id).is_some() {
                s.execute(item.id, &p).unwrap_or_else(|e| panic!("{}: {e}", item.id));
            }
        }
        end(&mut s).unwrap();
    }

    #[test]
    fn paragraph_and_feature_commands_on_nodes() {
        let (mut s, id) = setup("Hello");
        let ids = json!([id]);
        s.execute("text.setFeatures", &json!({"ids": ids, "features": {"liga": false, "tnum": true}})).unwrap();
        assert_eq!(data(&s, id).features.get("liga"), Some(&false));
        s.execute("text.setFeatures", &json!({"ids": ids, "features": {"liga": null, "ss01": 1}})).unwrap();
        let f = data(&s, id).features;
        assert_eq!(f.get("liga"), None);
        assert_eq!(f.get("ss01"), Some(&true));
        assert_eq!(f.get("tnum"), Some(&true));
        s.execute("text.setFeatures", &json!({"ids": ids, "features": {"zero": true}, "replace": true})).unwrap();
        assert_eq!(data(&s, id).features.len(), 1);
        assert!(s.execute("text.setFeatures", &json!({"ids": ids, "features": {"ligatures": true}})).is_err());
        assert!(s.execute("text.setFeatures", &json!({"ids": ids, "features": {"liga": "yes"}})).is_err());
        assert!(s.execute("text.setFeatures", &json!({"ids": ids})).is_err());
        s.execute("node.set", &json!({"id": id, "props": {"features": {"calt": false}}})).unwrap();
        assert_eq!(data(&s, id).features.len(), 1);
        s.execute("text.paragraphIndent", &json!({"ids": ids, "indent": 16})).unwrap();
        s.execute("text.paragraphSpacing", &json!({"ids": ids, "spacing": 8})).unwrap();
        s.execute("text.maxLines", &json!({"ids": ids, "lines": 2})).unwrap();
        s.execute("text.decoration", &json!({"ids": ids, "decoration": "strikethrough"})).unwrap();
        s.execute("text.case", &json!({"ids": ids, "case": "smallCaps"})).unwrap();
        s.execute("text.listStyle", &json!({"ids": ids, "style": "bulleted"})).unwrap();
        let t = data(&s, id);
        assert_eq!((t.paragraph_indent, t.paragraph_spacing, t.max_lines), (16.0, 8.0, 2));
        assert_eq!(t.auto_resize, TextAutoResize::Truncate);
        assert_eq!(t.style.decoration, TextDecoration::Strikethrough);
        assert_eq!(t.style.case, TextCase::SmallCaps);
        assert_eq!(t.list, ListStyle::Bulleted);
        assert!(s.execute("text.listStyle", &json!({"ids": ids, "style": "zigzag"})).is_err());
        assert!(s.execute("text.paragraphIndent", &json!({"ids": ids, "indent": "NaN"})).is_err());
        s.execute("node.set", &json!({"id": id, "props": {"textCase": "smallCaps", "paragraphIndent": 4, "maxLines": 0}})).unwrap();
        assert_eq!(data(&s, id).paragraph_indent, 4.0);
    }

    fn preedit_state(s: &Session) -> Option<Preedit> {
        s.ds().unwrap().text_edit.clone().unwrap().preedit
    }

    #[test]
    fn ime_composition_is_shown_but_not_in_the_document_until_committed() {
        let (mut s, id) = setup("Hello world");
        let undo_before = s.ds().unwrap().history.undo.len();
        begin(&mut s, id, Some(5), false).unwrap();
        let doc_before = s.ds().unwrap().doc.clone();
        s.execute("text.edit.preedit", &json!({"text": "にほ", "cursor": 1})).unwrap();
        assert_eq!(preedit_state(&s), Some(Preedit { text: "にほ".into(), cursor: 1 }));
        assert!(std::sync::Arc::ptr_eq(&doc_before, &s.ds().unwrap().doc), "composing leaves the document alone");
        let (shown, range, cursor) = preedit_doc(&s).unwrap();
        assert_eq!(shown.node(id).unwrap().kind.text().unwrap().characters, "Helloにほ world");
        assert_eq!((range, cursor), ((5, 7), 6));
        let info = s.execute("text.edit.selection", &json!({})).unwrap();
        assert_eq!(info["preedit"]["text"], "にほ");
        // Updating the composition replaces it; an empty one cancels it.
        preedit(&mut s, "にほん", None).unwrap();
        assert_eq!(preedit_doc(&s).unwrap().2, 8);
        preedit(&mut s, "", None).unwrap();
        assert!(preedit_state(&s).is_none() && preedit_doc(&s).is_none());
        assert_eq!(data(&s, id).characters, "Hello world");
        // Commit puts the final text at the caret.
        preedit(&mut s, "にほん", None).unwrap();
        s.execute("text.edit.insert", &json!({"text": "日本"})).unwrap();
        assert!(preedit_state(&s).is_none());
        assert_eq!(data(&s, id).characters, "Hello日本 world");
        assert_eq!(sel(&s), (7, 7));
        // Composing over a selection replaces it; moving the caret abandons the composition.
        set_caret(&mut s, 7, false).unwrap();
        set_caret(&mut s, 13, true).unwrap();
        preedit(&mut s, "x", None).unwrap();
        assert_eq!(data(&s, id).characters, "Hello日本");
        set_caret(&mut s, 0, false).unwrap();
        assert!(preedit_state(&s).is_none());
        assert_eq!(data(&s, id).characters, "Hello日本");
        // Ending the edit drops an uncommitted composition; the session stays one undo step.
        preedit(&mut s, "zz", None).unwrap();
        end(&mut s).unwrap();
        assert_eq!(data(&s, id).characters, "Hello日本");
        assert_eq!(s.ds().unwrap().history.undo.len(), undo_before + 1);
        // Not editing: an error, not a crash.
        assert!(preedit(&mut s, "a", None).is_err());
    }

    #[test]
    fn variable_font_axes_as_commands_and_props() {
        let (mut s, id) = setup("Hello world");
        let ids = json!([id]);
        s.execute("text.setVariations", &json!({"ids": ids, "variations": {"wght": 550, "wdth": 85}})).unwrap();
        let st = data(&s, id).style;
        assert_eq!((st.variations.get("wght"), st.variations.get("wdth")), (Some(&550.0), Some(&85.0)));
        assert_eq!(st.weight(), 550);
        // null removes one axis; replace clears the rest.
        s.execute("text.setVariations", &json!({"ids": ids, "variations": {"wdth": null}})).unwrap();
        assert_eq!(data(&s, id).style.variations.len(), 1);
        s.execute("text.setVariations", &json!({"ids": ids, "variations": {"opsz": 14}, "replace": true})).unwrap();
        assert_eq!(data(&s, id).style.variations.keys().collect::<Vec<_>>(), ["opsz"]);
        // Undo restores the previous values.
        s.undo().unwrap();
        assert_eq!(data(&s, id).style.variations.keys().collect::<Vec<_>>(), ["wght"]);
        // Junk is rejected.
        assert!(s.execute("text.setVariations", &json!({"ids": ids, "variations": {"weight": 500}})).is_err());
        assert!(s.execute("text.setVariations", &json!({"ids": ids, "variations": {"wght": "bold"}})).is_err());
        assert!(s.execute("text.setVariations", &json!({"ids": ids})).is_err());
        // node.set props (whole text or a range) replace the custom values.
        s.execute("node.set", &json!({"id": id, "props": {"fontVariations": {"wght": 300}, "start": 6, "end": 11}})).unwrap();
        let t = data(&s, id);
        assert_eq!(t.style_at(0).variations.get("wght"), Some(&550.0));
        assert_eq!(t.style_at(7).variations.get("wght"), Some(&300.0));
        s.execute("node.set", &json!({"id": id, "props": {"variationSettings": null}})).unwrap();
        assert!(data(&s, id).runs.is_empty() && data(&s, id).style.variations.is_empty());
        // Picking a named style (Bold, weight steps) drops the custom weight but keeps width.
        s.execute("text.setVariations", &json!({"ids": ids, "variations": {"wght": 650, "wdth": 90}})).unwrap();
        s.execute("text.bold", &json!({"ids": ids})).unwrap();
        let st = data(&s, id).style;
        assert_eq!(st.font_style, "Bold", "650 is lighter than bold, so Bold applies");
        assert!(!st.variations.contains_key("wght"));
        assert_eq!(st.variations.get("wdth"), Some(&90.0));
        // While editing, the command styles the selected range only.
        begin(&mut s, id, Some(0), false).unwrap();
        set_caret(&mut s, 5, true).unwrap();
        s.execute("text.setVariations", &json!({"variations": {"wdth": 75}})).unwrap();
        end(&mut s).unwrap();
        let t = data(&s, id);
        assert_eq!(t.style_at(0).variations.get("wdth"), Some(&75.0));
        assert_eq!(t.style_at(8).variations.get("wdth"), Some(&90.0));
        // The axes query answers without fonts too.
        let r = s.execute("text.fonts", &json!({"family": "Inter", "axes": true})).unwrap();
        assert!(r.is_null() || r["axes"].is_array());
    }

    #[test]
    fn text_commands_survive_junk_while_editing() {
        let junk = [
            json!({}),
            json!(null),
            json!({"index": -5, "features": {"": 1}}),
            json!({"index": 1e300, "style": 3, "case": null}),
            json!([1]),
            json!({"variations": {"wght": "heavy", "toolong": 1}, "text": "\u{0}\u{7}", "cursor": 1e300}),
            json!({"variations": {"wght": 1e300, "opsz": -1e300}, "replace": "yes", "text": "日本語", "cursor": -1}),
        ];
        for spec in crate::cmd::all_commands().iter().filter(|c| c.id.starts_with("text.")) {
            for j in &junk {
                let (mut s, id) = setup("Hi there\nyou");
                begin(&mut s, id, Some(2), false).unwrap();
                set_caret(&mut s, 5, true).unwrap();
                if let Err(EngineError::Internal { cmd, msg }) = s.execute(spec.id, j) {
                    panic!("{cmd} panicked on {j}: {msg}");
                }
            }
        }
    }
}
