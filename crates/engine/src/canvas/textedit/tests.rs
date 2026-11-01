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

#[test]
fn backspace_removes_only_the_final_thai_mark() {
    let (mut s, id) = setup("น้ำ");
    begin(&mut s, id, Some(3), false).unwrap();

    key(&mut s, "Backspace", false, false, false).unwrap();

    assert_eq!(data(&s, id).characters, "น้");
}

fn ev(kind: PointerKind, x: f64, y: f64, shift: bool) -> PointerEvent {
    let mut e = PointerEvent::at(kind, x, y, 1.0);
    e.shift = shift;
    e
}

/// Asserts double-clicking a text layer selects all text, a second double-click selects the
/// word without changing the editing session, and drag and Shift+click extend the
/// selection.
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
    let epoch = s.ds().unwrap().text_edit.as_ref().unwrap().editing_session;
    s.pointer(&ev(PointerKind::Down, mid, y, false)).unwrap();
    s.pointer(&ev(PointerKind::DoubleClick, mid, y, false)).unwrap();
    s.pointer(&ev(PointerKind::Up, mid, y, false)).unwrap();
    assert_eq!(sel(&s), (6, 11));
    assert_eq!(
        s.ds().unwrap().text_edit.as_ref().unwrap().editing_session,
        epoch,
        "a double-click inside the edit is an ordinary edit: same IME replacement epoch"
    );
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

/// No text command panics on junk parameters while a text is being edited.
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
    // `text.rescanFonts` takes no params and walks the font folders: one walk per junk is slow.
    for spec in crate::cmd::all_commands().iter().filter(|c| c.id.starts_with("text.") && c.id != "text.rescanFonts") {
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

/// Asserts surrounding text is logical UTF-8 text with byte offsets and keeps a reversed
/// selection reversed.
#[test]
fn ime_surrounding_uses_logical_utf8_bytes_and_keeps_selection_direction() {
    let (mut s, id) = setup("café😀e\u{301}x");
    begin(&mut s, id, Some(8), false).unwrap();
    set_caret(&mut s, 8, false).unwrap();
    set_caret(&mut s, 5, true).unwrap(); // anchor after the emoji, caret before it
    let ime = ime_surrounding(&s).unwrap();
    assert_eq!(ime.text, "café😀e\u{301}x");
    assert_eq!((ime.cursor, ime.anchor), (9, 13), "char 5 and char 8 as UTF-8 bytes");
    assert!(ime.cursor < ime.anchor, "the reversed selection stays reversed");
    assert_eq!(ime.owner.node, id);
    assert_eq!(ime.owner.document, s.ds().unwrap().uid);
}

/// Asserts an active composition is excluded from surrounding text and an empty text layer
/// reports an available, empty payload.
#[test]
fn ime_surrounding_excludes_the_composition_and_empty_text_is_available() {
    let (mut s, id) = setup("Hi");
    begin(&mut s, id, Some(2), false).unwrap();
    preedit(&mut s, "にほ", Some(1)).unwrap();
    let ime = ime_surrounding(&s).unwrap();
    assert_eq!(ime.text, "Hi", "the composition is not committed text");
    assert_eq!((ime.cursor, ime.anchor), (2, 2));
    end(&mut s).unwrap();

    let (mut s, id) = setup("");
    begin(&mut s, id, Some(0), false).unwrap();
    let ime = ime_surrounding(&s).unwrap();
    assert_eq!((ime.text.as_str(), ime.cursor, ime.anchor), ("", 0, 0), "an empty layer is available, not unavailable");
}

/// Asserts surrounding text is capped at `MAX_IME_SURROUNDING_BYTES`, an exactly budget-sized
/// selection is describable, and one byte more is `SelectionTooWide`.
#[test]
fn ime_surrounding_is_bounded_and_a_too_wide_selection_is_explicitly_unavailable() {
    let long = "a".repeat(5000);
    let (mut s, id) = setup(&long);
    begin(&mut s, id, Some(5000), false).unwrap();
    let ime = ime_surrounding(&s).unwrap();
    assert_eq!(ime.text.len(), MAX_IME_SURROUNDING_BYTES);
    assert_eq!((ime.cursor, ime.anchor), (MAX_IME_SURROUNDING_BYTES, MAX_IME_SURROUNDING_BYTES));

    // Exactly the budget is still describable; one more byte is not.
    set_caret(&mut s, 1000, false).unwrap();
    set_caret(&mut s, 5000, true).unwrap();
    let ime = ime_surrounding(&s).unwrap();
    assert_eq!(ime.text.len(), 4000);
    assert_eq!((ime.cursor, ime.anchor), (4000, 0));
    set_caret(&mut s, 999, false).unwrap();
    set_caret(&mut s, 5000, true).unwrap();
    assert_eq!(ime_surrounding(&s), Err(ImeSurroundingUnavailable::SelectionTooWide));
}

/// Asserts edits and caret moves keep the same IME owner, while ending and beginning a new
/// edit session changes the generation.
#[test]
fn ime_owner_is_stable_through_edits_and_the_session_changes_on_begin() {
    let (mut s, id) = setup("Hello");
    begin(&mut s, id, Some(0), false).unwrap();
    let first = ime_surrounding(&s).unwrap().owner;
    insert(&mut s, "x").unwrap();
    set_caret(&mut s, 2, false).unwrap();
    assert_eq!(ime_surrounding(&s).unwrap().owner, first, "edits and caret moves keep the owner");
    end(&mut s).unwrap();
    begin(&mut s, id, Some(0), false).unwrap();
    let replaced = ime_surrounding(&s).unwrap().owner;
    assert_eq!(replaced.node, id);
    assert_ne!(replaced.editing_session, first.editing_session, "a new edit session is a replacement epoch");
}

/// Asserts byte deletion around a reversed selection removes whole characters on both
/// sides, keeps the selected text and direction, and a final no-op leaves the text alone.
#[test]
fn ime_delete_surrounding_bytes_keeps_the_reversed_selection_and_deletes_around_it() {
    let (mut s, id) = setup("ab你好cd");
    begin(&mut s, id, Some(4), false).unwrap();
    set_caret(&mut s, 4, false).unwrap();
    set_caret(&mut s, 2, true).unwrap(); // reversed: caret before 你, anchor after 好
    assert!(delete_surrounding_bytes(&mut s, 1, 1).unwrap());
    // "b" (whole character before the selection) and "c" (whole character after it) go; the
    // selected "你好" stays for the paired commit, and the reversed direction is preserved.
    assert_eq!(data(&s, id).characters, "a你好d");
    assert_eq!(sel(&s), (3, 1)); // (anchor, caret), still reversed
    assert!(delete_surrounding_bytes(&mut s, 1, 1).unwrap());
    assert_eq!(data(&s, id).characters, "你好");
    assert_eq!(sel(&s), (2, 0)); // (anchor, caret), still reversed
    assert!(!delete_surrounding_bytes(&mut s, 1, 1).unwrap(), "nothing is left around the selection");
    assert_eq!(data(&s, id).characters, "你好", "a no-op delete does not touch the text");
}

/// Asserts character deletion keeps and shifts the selection, and a no-op delete leaves an
/// active composition and the revision untouched.
#[test]
fn ime_delete_surrounding_chars_keeps_the_selection_and_ignores_a_no_op() {
    let (mut s, id) = setup("ab你好cd");
    begin(&mut s, id, Some(2), false).unwrap();
    set_caret(&mut s, 4, true).unwrap(); // "你好"
    assert!(delete_surrounding_chars(&mut s, 1, 1).unwrap());
    assert_eq!(data(&s, id).characters, "a你好d");
    assert_eq!(sel(&s), (1, 3), "the selection is kept, shifted by the deleted prefix");
    set_caret(&mut s, 0, false).unwrap();
    // A composition in progress survives a delete that finds nothing to remove.
    preedit(&mut s, "x", None).unwrap();
    let revision = s.ds().unwrap().revision;
    assert!(!delete_surrounding_chars(&mut s, 1, 0).unwrap());
    assert_eq!(s.ds().unwrap().text_edit.as_ref().and_then(|te| te.preedit.as_ref()).map(|p| p.text.as_str()), Some("x"));
    assert_eq!(s.ds().unwrap().revision, revision, "a no-op does not bump the revision");
    assert!(!delete_surrounding_bytes(&mut s, 1, 0).unwrap());
    assert_eq!(s.ds().unwrap().text_edit.as_ref().and_then(|te| te.preedit.as_ref()).map(|p| p.text.as_str()), Some("x"));
    assert_eq!(s.ds().unwrap().revision, revision);
}

/// Asserts a text layer in another document has a different IME owner.
#[test]
fn ime_owner_identity_changes_with_the_document() {
    let (mut s, id) = setup("Hi");
    begin(&mut s, id, Some(2), false).unwrap();
    let first = ime_surrounding(&s).unwrap().owner;
    end(&mut s).unwrap();
    s.execute("file.new", &json!({"name": "Second"})).unwrap();
    let r = s.execute("text.create", &json!({"text": "There", "x": 0, "y": 0})).unwrap();
    let other = crate::params::id_of(&r["id"]).unwrap();
    begin(&mut s, other, Some(5), false).unwrap();
    let second = ime_surrounding(&s).unwrap().owner;
    assert_ne!(second.document, first.document, "another document is another IME owner");
    assert_eq!(second.node, other);
}

/// Asserts a space committed after a line-start `-` becomes a bulleted list and reports
/// `Transformed`; a plain `"x"` commit reports `Plain`, and an empty commit reports `Nothing`.
#[test]
fn ime_commit_reports_list_autoformat_as_an_application_transformation() {
    let (mut s, id) = setup("-");
    begin(&mut s, id, Some(1), false).unwrap();
    assert_eq!(insert_ime_commit(&mut s, " ").unwrap(), ImeInsertion::Transformed);
    assert_eq!(data(&s, id).characters, "");
    assert_eq!(data(&s, id).para_list(0).list, ListStyle::Bulleted);
    assert_eq!(insert_ime_commit(&mut s, "x").unwrap(), ImeInsertion::Plain);
    assert_eq!(insert_ime_commit(&mut s, "").unwrap(), ImeInsertion::Nothing);
}

/// Asserts CRLF normalization reports `Transformed`, control-only input reports `Nothing`,
/// and bare newline/CR commits are ignored while a live composition remains.
#[test]
fn ime_commit_normalisation_is_a_transformation_and_a_bare_newline_is_ignored() {
    let (mut s, id) = setup("Hi");
    begin(&mut s, id, Some(2), false).unwrap();
    // CRLF is inserted as LF, but the editor changed the commit: application-origin.
    assert_eq!(insert_ime_commit(&mut s, "a\r\nb").unwrap(), ImeInsertion::Transformed);
    assert_eq!(data(&s, id).characters, "Hia\nb");
    // Control characters are dropped; the commit itself is a no-op.
    assert_eq!(insert_ime_commit(&mut s, "\u{0}\u{7}").unwrap(), ImeInsertion::Nothing);
    assert_eq!(data(&s, id).characters, "Hia\nb");
    // Stock `TextEdit` ignores a bare newline/CR commit; typing Enter still inserts one.
    // (It also leaves a live composition alone, unlike the non-ignored commit below.)
    preedit(&mut s, "x", None).unwrap();
    assert_eq!(insert_ime_commit(&mut s, "\n").unwrap(), ImeInsertion::Nothing);
    assert_eq!(insert_ime_commit(&mut s, "\r").unwrap(), ImeInsertion::Nothing);
    assert_eq!(preedit_state(&s).map(|p| p.text), Some("x".into()));
    preedit(&mut s, "", None).unwrap();
    assert_eq!(data(&s, id).characters, "Hia\nb");
    insert(&mut s, "\n").unwrap();
    assert_eq!(data(&s, id).characters, "Hia\nb\n");
}

/// Asserts an empty or control-only IME commit ends a live composition (bumping the revision
/// once), inserts nothing, and is a harmless no-op when no composition is active.
#[test]
fn an_empty_ime_commit_ends_the_composition_without_inserting_text() {
    let (mut s, id) = setup("Hello");
    begin(&mut s, id, Some(5), false).unwrap();
    preedit(&mut s, "にほ", None).unwrap();
    assert_eq!(preedit_state(&s).map(|p| p.text), Some("にほ".into()));
    let revision = s.ds().unwrap().revision;
    assert_eq!(insert_ime_commit(&mut s, "").unwrap(), ImeInsertion::Nothing);
    assert_eq!(s.ds().unwrap().revision, revision + 1, "ending a live composition bumps the revision");
    assert!(preedit_state(&s).is_none(), "an empty commit ends the composition");
    assert!(preedit_doc(&s).is_none());
    assert_eq!(data(&s, id).characters, "Hello", "and inserts nothing");

    // A control-only commit normalises to empty and ends the composition too.
    preedit(&mut s, "x", None).unwrap();
    let revision = s.ds().unwrap().revision;
    assert_eq!(insert_ime_commit(&mut s, "\u{0}\u{7}").unwrap(), ImeInsertion::Nothing);
    assert_eq!(s.ds().unwrap().revision, revision + 1, "clearing a live composition bumps the revision");
    assert!(preedit_state(&s).is_none());
    assert_eq!(data(&s, id).characters, "Hello");

    // With no composition (and not even while editing) an empty commit stays a harmless no-op.
    let revision = s.ds().unwrap().revision;
    assert_eq!(insert_ime_commit(&mut s, "").unwrap(), ImeInsertion::Nothing);
    assert_eq!(s.ds().unwrap().revision, revision);
    end(&mut s).unwrap();
    assert_eq!(insert_ime_commit(&mut s, "").unwrap(), ImeInsertion::Nothing);
}
