//! IME composition in a text layer, driven through egui headless with the events macOS sends for
//! Pinyin ("nihao", then a candidate picked): the platform keeps the IME on every frame of the
//! edit (an idle frame too) with the candidate window at the caret, the composition shows without
//! entering the document, keys pressed while composing are the IME's, and the commit lands once.

use egui::output::{IMEOutput, IMESurroundingText, IMESurroundingTextPayload, ImeChangeCause};
use egui::{Event, ImeEvent, Key, Modifiers, RawInput, Rect};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn key(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
}

fn preedit(text: &str) -> Event {
    let n = text.chars().count();
    Event::Ime(ImeEvent::Preedit { text: text.into(), active_range_chars: (n > 0).then_some(n..n) })
}

#[expect(deprecated, reason = "egui still delivers these, and the editor must ignore them")]
fn enabled_disabled() -> (Event, Event) {
    (Event::Ime(ImeEvent::Enabled), Event::Ime(ImeEvent::Disabled))
}

fn text(u: &Ui, id: NodeId) -> String {
    match &u.app.session.doc().unwrap().node(id).unwrap().kind {
        NodeKind::Text(t) => t.characters.clone(),
        _ => panic!("not text"),
    }
}

/// One frame; the IME output it leaves for the platform (`None` turns the IME off).
fn ime(u: &mut Ui, events: Vec<Event>) -> Rect {
    let ime = u.frame_raw(RawInput { events, ..Default::default() }).platform_output.ime.expect("the IME stays on while editing");
    assert_eq!(ime.rect, ime.cursor_rect, "the desktop places candidates by `rect`: the caret");
    ime.cursor_rect
}

fn composing(u: &Ui) -> Option<String> {
    u.app.session.ds().unwrap().text_edit.as_ref().expect("still editing").preedit.as_ref().map(|p| p.text.clone())
}

/// The whole IME descriptor one frame leaves for the platform.
fn ime_descriptor(u: &mut Ui, events: Vec<Event>) -> IMEOutput {
    u.frame_raw(RawInput { events, ..Default::default() }).platform_output.ime.expect("the IME stays on while editing")
}

/// Return the available surrounding-text payload of an IME output, panicking when unavailable.
fn available(ime: &IMEOutput) -> &IMESurroundingTextPayload {
    match &ime.surrounding_text {
        IMESurroundingText::Available(payload) => payload,
        IMESurroundingText::Unavailable => panic!("expected available surrounding text"),
    }
}

/// Create a text layer with `text` and return its node id.
fn text_layer(s: &mut Session, text: &str) -> NodeId {
    NodeId(s.execute("text.create", &json!({"text": text, "x": 40, "y": 40})).unwrap()["id"].as_u64().unwrap())
}

#[test]
fn pinyin_composes_inline_and_commits_once_with_the_ime_on_every_frame() {
    // The text is measured with a font: without craft-fonts a system one, scanned here rather than
    // by whichever test happens to run first (#333).
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = NodeId(s.execute("text.create", &json!({"text": "Hi", "x": 40, "y": 40})).unwrap()["id"].as_u64().unwrap());
    // On a machine without any font nothing lays out and every caret sits at the start.
    let fonts = figcraft_text::layout_node(s.doc().unwrap().node(t).unwrap()).is_some_and(|(l, _)| !l.glyphs.is_empty());
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();
    // An idle frame (the caret blinking) keeps the IME on, or macOS drops the composition.
    let caret = ime(&mut u, vec![]);
    let ds = u.app.session.ds().unwrap();
    let b = ds.doc.world_bounds(t).unwrap();
    let layer = Rect::from_two_pos(u.app.to_screen(b.x0, b.y0), u.app.to_screen(b.x1, b.y1)).expand(1.0);
    assert!(layer.contains(caret.center()), "candidates open at the caret ({caret:?} in {layer:?})");

    let (enabled, disabled) = enabled_disabled();
    ime(&mut u, vec![enabled, preedit("n")]);
    ime(&mut u, vec![]);
    let mid = ime(&mut u, vec![preedit("ni")]);
    assert!(!fonts || mid.min.x > caret.min.x, "the IME cursor follows the composition");
    // A key the platform leaks while composing (Backspace) belongs to the IME, not the text.
    ime(&mut u, vec![key(Key::Backspace, true), preedit("nih"), key(Key::Backspace, false)]);
    ime(&mut u, vec![preedit("nihao")]);
    assert_eq!(composing(&u).as_deref(), Some("nihao"));
    assert_eq!(text(&u, t), "Hi", "the composition isn't in the document yet");

    // Enter confirms the candidate: no newline, editing goes on.
    let after = ime(&mut u, vec![key(Key::Enter, true), preedit(""), Event::Ime(ImeEvent::Commit("你好".into())), disabled, key(Key::Enter, false)]);
    assert_eq!(text(&u, t), "Hi你好");
    assert_eq!(composing(&u), None);
    let te = u.app.session.ds().unwrap().text_edit.clone().unwrap();
    assert_eq!((te.caret, te.anchor), (4, 4));
    assert!(!fonts || after.min.x > caret.min.x, "the caret moved past the committed text");
    ime(&mut u, vec![]);
    assert_eq!(text(&u, t), "Hi你好", "committed once");

    // Keys are the editor's again once the composition is over.
    ime(&mut u, vec![key(Key::Enter, true), key(Key::Enter, false), Event::Text("!".into())]);
    assert_eq!(text(&u, t), "Hi你好\n!");
}

/// Asserts an empty IME commit ends the composition, inserts nothing, keeps the editor
/// focused, and the next pass observes the committed text.
#[test]
fn an_empty_ime_commit_ends_the_composition_without_inserting_text() {
    // Safari's egui web integration sends `Commit("")` (not `Preedit("")`) when the last
    // composition character is deleted. It ends the composition; it must not leave the pre-edit
    // rendering or insert anything.
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hello");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(5), false).unwrap();
    ime(&mut u, vec![preedit("にほ")]);
    assert_eq!(composing(&u).as_deref(), Some("にほ"));
    let after = ime_descriptor(&mut u, vec![Event::Ime(ImeEvent::Commit(String::new()))]);
    assert_eq!(composing(&u), None, "the empty commit ends the composition");
    assert_eq!(text(&u, t), "Hello", "and inserts nothing");
    assert!(matches!(after.surrounding_text, IMESurroundingText::Available(_)));
    // The editor is still focused, and the next pass observes committed text only.
    let next = ime_descriptor(&mut u, vec![]);
    assert_eq!(composing(&u), None);
    assert_eq!(available(&next).text, "Hello");
    assert_eq!(text(&u, t), "Hello");
}

/// Asserts byte deletion before the paired commit replaces the exact UTF-8 range, and one undo
/// restores the pre-IME text.
#[test]
fn an_ime_replacing_text_deletes_in_bytes_before_the_paired_commit() {
    // 0.37 sends `DeleteSurroundingBytes` immediately before the commit that replaces it. The
    // custom engine editor deletes the exact UTF-8 range (never display-bidi text) and keeps the
    // whole edit as one undo step.
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = NodeId(s.execute("text.create", &json!({"text": "ab\u{4F60}\u{597D}cd", "x": 40, "y": 40})).unwrap()["id"].as_u64().unwrap());
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(4), false).unwrap();
    ime(&mut u, vec![]);
    // Compose, then replace "你好c" with "X".
    ime(&mut u, vec![preedit("x")]);
    ime(&mut u, vec![Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 6, after_bytes: 1 }), Event::Ime(ImeEvent::Commit("X".into()))]);
    assert_eq!(text(&u, t), "abXd");
    figcraft_engine::textedit::end(&mut u.app.session).unwrap();
    u.app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&u, t), "ab\u{4F60}\u{597D}cd", "one undo restores the pre-IME text");
}

/// Asserts a caret click invalidates the whole stale IME batch (both byte and character
/// deletions) while a later direct commit still works.
#[test]
fn a_caret_click_rejects_the_stale_delete_and_commit_in_the_same_input_batch() {
    figcraft_text::scan_system_fonts();
    for delete in
        [ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 1 }, ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 1 }]
    {
        let mut s = Session::new();
        let t = text_layer(&mut s, "abcdef");
        let mut u = Ui::new(s);
        figcraft_engine::textedit::begin(&mut u.app.session, t, Some(6), false).unwrap();
        ime(&mut u, vec![preedit("z")]);
        let p = u.app.to_screen(42.0, 45.0);
        // Real canvas pointer handling moves the caret and cancels preedit before text_keys.
        ime(
            &mut u,
            vec![
                Event::PointerMoved(p),
                Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                Event::Ime(delete),
                Event::Ime(ImeEvent::Commit("Q".into())),
            ],
        );
        let edit = u.app.session.ds().unwrap().text_edit.as_ref().unwrap();
        assert!(edit.caret < 6, "the production canvas click actually moved the caret");
        assert!(edit.preedit.is_none(), "the click ended the composition");
        assert_eq!(text(&u, t), "abcdef", "neither stale deletion nor its Commit may mutate the clicked caret");
        u.frame(vec![Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        // A later direct commit without an active composition is legitimate.
        let caret = u.app.session.ds().unwrap().text_edit.as_ref().unwrap().caret;
        ime(&mut u, vec![Event::Ime(ImeEvent::Commit("X".into()))]);
        let mut expected = "abcdef".to_string();
        expected.insert(caret, 'X');
        assert_eq!(text(&u, t), expected, "the interrupted batch does not poison a later direct commit");
        figcraft_engine::textedit::end(&mut u.app.session).unwrap();
        u.app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(text(&u, t), "abcdef", "the valid edit retains one-session undo");
    }
}

/// Asserts a control-channel document switch rejects the old composition's delete and commit
/// batch, while a direct commit to the new target remains valid.
#[test]
fn a_document_switch_rejects_the_old_compositions_delete_and_commit_batch() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let a = text_layer(&mut s, "Alpha");
    s.execute("file.new", &json!({"name": "B"})).unwrap();
    let b = text_layer(&mut s, "Bravo");
    figcraft_engine::textedit::begin(&mut s, b, Some(5), false).unwrap();
    s.execute("doc.switch", &json!({"index": 0})).unwrap();
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, a, Some(5), false).unwrap();
    ime(&mut u, vec![preedit("z")]);
    let (tx, rx) = std::sync::mpsc::channel();
    u.app.control_queue.push(crate::control::ControlRequest {
        id: json!(91),
        method: "engine.execute".into(),
        params: json!({"window": 0, "command": "doc.switch", "params": {"index": 1}}),
        reply: tx,
        state: Default::default(),
    });
    ime(&mut u, vec![Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }), Event::Ime(ImeEvent::Commit("Q".into()))]);
    assert!(rx.try_recv().unwrap().result.is_ok(), "production control drain switched the document");
    assert_eq!(u.app.session.active, 1);
    assert_eq!(text(&u, b), "Bravo", "the new target cannot inherit the old composition's mutations");
    assert_eq!(u.app.session.docs[0].doc.node(a).unwrap().kind.text().unwrap().characters, "Alpha");
    ime(&mut u, vec![Event::Ime(ImeEvent::Commit("X".into()))]);
    assert_eq!(text(&u, b), "BravoX", "a direct commit to the new noncomposing target remains valid");
}

/// Asserts the first observation and ordinary typing are `Other`, while an accepted commit is
/// `InputMethod` with UTF-8 offsets, the same owner and generation.
#[test]
fn ime_cause_is_input_method_only_for_an_accepted_commit() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hi");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();

    let first = ime_descriptor(&mut u, vec![]);
    assert_eq!(first.change_cause, ImeChangeCause::Other, "the first focused observation starts a context");
    assert_eq!(available(&first).text, "Hi");
    assert_eq!((available(&first).cursor, available(&first).anchor), (2, 2));

    let commit = ime_descriptor(&mut u, vec![Event::Ime(ImeEvent::Commit("你好".into()))]);
    assert_eq!(commit.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(available(&commit).text, "Hi你好");
    assert_eq!(available(&commit).cursor, 8, "UTF-8 bytes, not characters");
    assert_eq!(commit.context_generation, first.context_generation, "a commit is not a context change");
    assert_eq!(commit.owner, first.owner);

    let typed = ime_descriptor(&mut u, vec![Event::Text("x".into())]);
    assert_eq!(typed.change_cause, ImeChangeCause::Other, "ordinary typing is an application edit");
    assert_eq!(available(&typed).text, "Hi你好x");
    assert_eq!(text(&u, t), "Hi你好x");
}

/// Asserts a frame mixing IME and application edits is `Other`, and a later pure commit is
/// `InputMethod` without lingering state.
#[test]
fn ime_cause_reports_mixed_application_and_ime_mutations_as_other() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hi");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();
    ime(&mut u, vec![]);

    let mixed = ime_descriptor(&mut u, vec![Event::Ime(ImeEvent::Commit("あ".into())), Event::Text("b".into())]);
    assert_eq!(mixed.change_cause, ImeChangeCause::Other, "an application edit in the same frame dominates");
    assert_eq!(available(&mixed).text, "Hiあb");

    // The next frame is only an accepted commit: the frame-scoped `Other` does not linger.
    let pure = ime_descriptor(&mut u, vec![Event::Ime(ImeEvent::Commit("う".into()))]);
    assert_eq!(pure.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(available(&pure).text, "Hiあbう");

    // An application edit before the IME receipt, and IME → application → IME, are both Other.
    let before = ime_descriptor(&mut u, vec![Event::Text("c".into()), Event::Ime(ImeEvent::Commit("え".into()))]);
    assert_eq!(before.change_cause, ImeChangeCause::Other);
    let sandwiched =
        ime_descriptor(&mut u, vec![Event::Ime(ImeEvent::Commit("お".into())), Event::Text("d".into()), Event::Ime(ImeEvent::Commit("か".into()))]);
    assert_eq!(sandwiched.change_cause, ImeChangeCause::Other);
    assert_eq!(text(&u, t), "Hiあbうcえおdか");
}

/// Asserts a control-channel text edit is an application change (`Other`).
#[test]
fn a_control_channel_text_edit_is_an_application_change() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hi");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();
    ime(&mut u, vec![]);

    // A command from the control channel (not an ImeEvent) changes committed text.
    u.app.session.execute("text.edit.insert", &json!({"text": "z"})).unwrap();
    let out = ime_descriptor(&mut u, vec![]);
    assert_eq!(out.change_cause, ImeChangeCause::Other);
    assert_eq!(available(&out).text, "Hiz");
}

/// Asserts a byte delete followed by a commit is `InputMethod` and describes the
/// post-mutation text and caret.
#[test]
fn ime_delete_then_commit_is_input_method_and_describes_the_post_mutation_text() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "ab\u{4F60}\u{597D}cd");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(4), false).unwrap();
    ime(&mut u, vec![]);

    let out = ime_descriptor(
        &mut u,
        vec![Event::Ime(ImeEvent::DeleteSurroundingBytes { before_bytes: 6, after_bytes: 1 }), Event::Ime(ImeEvent::Commit("X".into()))],
    );
    assert_eq!(out.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(available(&out).text, "abXd", "the snapshot is observed after both mutations");
    assert_eq!(available(&out).cursor, 3);
    assert_eq!(text(&u, t), "abXd");
}

/// Asserts preedit text is excluded from surrounding text and earns no receipt, while the
/// following commit is `InputMethod`.
#[test]
fn preedit_is_not_surrounding_text_but_the_commit_that_follows_is_input_method() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hi");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();
    ime(&mut u, vec![]);

    let composing = ime_descriptor(&mut u, vec![preedit("にほ")]);
    assert_eq!(available(&composing).text, "Hi", "the composition is excluded");
    assert_eq!((available(&composing).cursor, available(&composing).anchor), (2, 2));
    assert_eq!(composing.change_cause, ImeChangeCause::Other, "preedit alone earns no receipt");

    let committed = ime_descriptor(&mut u, vec![preedit(""), Event::Ime(ImeEvent::Commit("にほ".into()))]);
    assert_eq!(committed.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(available(&committed).text, "Hiにほ");
    assert_eq!(available(&committed).cursor, 8);
}

/// Asserts surrounding text is logical UTF-8 with byte offsets and keeps a reversed
/// selection.
#[test]
fn ime_surrounding_text_is_logical_utf8_and_keeps_a_reversed_selection() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "café😀e\u{301}x");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(8), false).unwrap();
    figcraft_engine::textedit::set_caret(&mut u.app.session, 8, false).unwrap();
    figcraft_engine::textedit::set_caret(&mut u.app.session, 5, true).unwrap();

    let out = ime_descriptor(&mut u, vec![]);
    let payload = available(&out);
    assert_eq!(payload.text, "café😀e\u{301}x");
    assert_eq!((payload.cursor, payload.anchor), (9, 13));
    assert!(payload.cursor < payload.anchor, "the reversed selection is not sorted");
}

/// Asserts empty text is available while a selection wider than the 4000-byte budget is
/// explicitly unavailable but still has a nonzero IME context generation.
#[test]
fn empty_text_is_available_while_a_too_wide_selection_is_explicitly_unavailable() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(0), false).unwrap();
    let empty = ime_descriptor(&mut u, vec![]);
    let payload = available(&empty);
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("", 0, 0));

    // The same editor over a selection wider than the 4000-byte budget cannot describe it.
    let mut s = Session::new();
    let long = "a".repeat(5000);
    let t = text_layer(&mut s, &long);
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, None, false).unwrap();
    let wide = ime_descriptor(&mut u, vec![]);
    assert_eq!(wide.surrounding_text, IMESurroundingText::Unavailable);
    assert!(wide.context_generation > 0, "the owner stays known when only the text is unavailable");
}

/// Asserts replacing a too-wide selection with a commit is still an `InputMethod` receipt.
#[test]
fn a_commit_replacing_a_too_wide_selection_is_still_an_input_method_receipt() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let long = "a".repeat(5000);
    let t = text_layer(&mut s, &long);
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, None, false).unwrap();
    let wide = ime_descriptor(&mut u, vec![]);
    assert_eq!(wide.surrounding_text, IMESurroundingText::Unavailable);

    // The unavailable baseline and the pre-mutation stamp carry the engine's document revision,
    // so replacing the over-wide selection with a normal commit is a provable IME receipt rather
    // than an unproven `Other` change.
    let out = ime_descriptor(&mut u, vec![Event::Ime(ImeEvent::Commit("X".into()))]);
    assert_eq!(out.change_cause, ImeChangeCause::InputMethod);
    assert_eq!(text(&u, t), "X");
    assert_eq!(available(&out).text, "X");
    assert_eq!((available(&out).cursor, available(&out).anchor), (1, 1));
}

/// Asserts another text node is another owner, and re-entering the same node changes the
/// context generation while keeping the owner key.
#[test]
fn a_new_edit_session_and_another_text_node_are_new_ime_owners() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let a = text_layer(&mut s, "A");
    let b = text_layer(&mut s, "B");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, a, Some(1), false).unwrap();
    let first = ime_descriptor(&mut u, vec![]);

    figcraft_engine::textedit::end(&mut u.app.session).unwrap();
    figcraft_engine::textedit::begin(&mut u.app.session, b, Some(1), false).unwrap();
    let other = ime_descriptor(&mut u, vec![]);
    assert_ne!(other.owner, first.owner, "another text node is another owner");
    assert_ne!(other.context_generation, first.context_generation);

    figcraft_engine::textedit::end(&mut u.app.session).unwrap();
    figcraft_engine::textedit::begin(&mut u.app.session, a, Some(1), false).unwrap();
    let again = ime_descriptor(&mut u, vec![]);
    assert_eq!(again.owner, first.owner, "the owner key is the document and node");
    assert_ne!(again.context_generation, first.context_generation, "re-entering is a replaced target");
}

/// Asserts an accepted commit survives a discarded rerun without inserting twice and keeps
/// its `InputMethod` cause, while a mixed rerun is `Other`.
#[test]
fn an_accepted_commit_survives_a_discarded_pass_without_reapplying_or_losing_its_cause() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hi");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();
    let first = ime_descriptor(&mut u, vec![]);

    let out = u.frame_raw_with_discard(RawInput { events: vec![Event::Ime(ImeEvent::Commit("あ".into()))], ..Default::default() });
    let ime = out.platform_output.ime.expect("the presented pass keeps the IME on");
    assert_eq!(text(&u, t), "Hiあ", "the discarded rerun must not insert twice");
    assert_eq!(available(&ime).text, "Hiあ");
    assert_eq!(ime.change_cause, ImeChangeCause::InputMethod, "the unchanged second pass keeps the first pass's cause");
    assert_eq!(ime.context_generation, first.context_generation);

    let mixed = u
        .frame_raw_with_discard(RawInput { events: vec![Event::Ime(ImeEvent::Commit("い".into())), Event::Text("x".into())], ..Default::default() });
    let mixed = mixed.platform_output.ime.expect("still editing");
    assert_eq!(text(&u, t), "Hiあいx");
    assert_eq!(mixed.change_cause, ImeChangeCause::Other);
}

/// Asserts IME context state is scoped per viewport: each viewport has its own generation,
/// the root keeps its context across an aux pass, and a closed aux viewport does not inherit
/// the old generation.
#[test]
fn ime_context_state_is_scoped_per_viewport() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hi");
    figcraft_engine::textedit::begin(&mut s, t, Some(2), false).unwrap();
    let mut app = crate::FigApp::new(s, Box::new(crate::NoServices));
    // No window drew here, so give the canvas a finite rect for the caret geometry.
    app.canvas.rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let aux = egui::ViewportId::from_hash_of("figcraft-ime-aux");

    let mut raw = RawInput { viewport_id: egui::ViewportId::ROOT, ..Default::default() };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default();
    let root_out = ctx.run_ui(raw, |ui| {
        ui.ctx().show_viewport_deferred(aux, egui::ViewportBuilder::default(), |_, _| {});
        crate::shortcuts::ime_output(&mut app, ui.ctx());
    });
    let root = root_out.platform_output.ime.clone().expect("root descriptor");
    root_out.drop_without_applying_deltas();

    let mut raw = RawInput { viewport_id: aux, ..Default::default() };
    raw.viewports.entry(aux).or_default();
    let aux_out = ctx.run_ui(raw, |ui| crate::shortcuts::ime_output(&mut app, ui.ctx()));
    let aux_ime = aux_out.platform_output.ime.clone().expect("aux descriptor");
    aux_out.drop_without_applying_deltas();
    assert_eq!(aux_ime.owner, root.owner, "the same edit is the same owner");
    assert_ne!(aux_ime.context_generation, root.context_generation, "each viewport gets its own generation");
    assert_eq!(available(&aux_ime).text, "Hi");

    // The root's next consecutive pass keeps its context even though the aux pass ran between.
    let mut raw = RawInput { viewport_id: egui::ViewportId::ROOT, ..Default::default() };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default();
    let root_out = ctx.run_ui(raw, |ui| {
        ui.ctx().show_viewport_deferred(aux, egui::ViewportBuilder::default(), |_, _| {});
        crate::shortcuts::ime_output(&mut app, ui.ctx());
    });
    let root_again = root_out.platform_output.ime.clone().expect("root descriptor");
    root_out.drop_without_applying_deltas();
    assert_eq!(root_again.context_generation, root.context_generation);

    // Closing the aux viewport releases its state; it never inherits the old context.
    app.ime.end_viewport(aux);
    let mut raw = RawInput { viewport_id: aux, ..Default::default() };
    raw.viewports.entry(aux).or_default();
    let aux_out = ctx.run_ui(raw, |ui| crate::shortcuts::ime_output(&mut app, ui.ctx()));
    let aux_again = aux_out.platform_output.ime.clone().expect("aux descriptor");
    aux_out.drop_without_applying_deltas();
    assert_ne!(aux_again.context_generation, aux_ime.context_generation);
}

/// Asserts the engine's IME surrounding-text byte limit equals egui's wire limit.
#[test]
fn the_engine_surrounding_limit_matches_the_wire_limit() {
    // A divergence would truncate or overrun the bounded payload silently.
    let engine = figcraft_engine::textedit::MAX_IME_SURROUNDING_BYTES;
    let wire = egui::output::MAX_IME_SURROUNDING_TEXT_BYTES;
    assert_eq!(engine, wire);
}

/// Asserts a frame observes the IME exactly once in both the editor pass and the Home
/// early-return pass.
#[test]
fn a_frame_observes_the_ime_once_even_when_it_ends_early() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hi");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();
    let before = u.app.ime_observations;
    u.frame(vec![]);
    assert_eq!(u.app.ime_observations - before, 1, "one final observation for the editor pass");

    // The home/present early returns make the same single idle observation.
    u.app.ui.home = true;
    let before = u.app.ime_observations;
    u.frame(vec![]);
    assert_eq!(u.app.ime_observations - before, 1, "one idle observation on the home screen");
}

/// Asserts a double-click inside an existing edit selects the word without changing the IME
/// owner or generation.
#[test]
fn a_double_click_inside_the_edit_keeps_the_ime_generation() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hello world");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();
    let (p, zoom) = {
        let doc = u.app.session.doc().unwrap();
        let node = doc.node(t).unwrap();
        let Some((layout, dy)) = figcraft_text::layout_node(node) else { return };
        if layout.glyphs.is_empty() || layout.lines.is_empty() {
            return; // no fonts
        }
        let (x0, x1) = (layout.caret(7).0, layout.caret(8).0);
        let line = &layout.lines[0];
        let world = doc.world_transform(t);
        let p = world * figcraft_geom::Point::new((x0 + x1) / 2.0, line.top + line.height / 2.0 + dy);
        (p, u.app.session.ds().unwrap().view.zoom)
    };
    let first = ime_descriptor(&mut u, vec![]);
    let ev = figcraft_engine::PointerEvent::at(figcraft_engine::PointerKind::DoubleClick, p.x, p.y, zoom);
    u.app.session.pointer(&ev).unwrap();

    let after = ime_descriptor(&mut u, vec![]);
    assert_eq!(after.owner, first.owner, "the same text node is the same owner");
    assert_eq!(after.context_generation, first.context_generation, "a word-select inside the edit is not a replaced target");
    assert_eq!(available(&after).text, "Hello world");
}

/// Asserts a `window.new` that moves the edited file leaves the root window emitting no IME
/// descriptor.
#[test]
fn a_window_new_that_moves_the_edited_file_leaves_the_root_ime_idle() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::empty();
    s.execute("file.new", &json!({"name": "A"})).unwrap();
    let uid = s.ds().unwrap().uid;
    let t = text_layer(&mut s, "Hi");
    figcraft_engine::textedit::begin(&mut s, t, Some(2), false).unwrap();
    s.execute("file.new", &json!({"name": "B"})).unwrap();
    let a = s.docs.iter().position(|d| d.uid == uid).unwrap();
    s.execute("doc.switch", &json!({"index": a})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.set_embed_viewports(false);
    let first = ime_descriptor(&mut u, vec![]);
    assert_eq!(available(&first).text, "Hi");

    // A `window.new` queued before the pass is applied after `windows::begin` picked the file
    // for the root: the root no longer shows the edited file when the pass ends.
    u.app.window_requests.push(figcraft_engine::UiRequest { kind: "window.new".into(), data: json!({"index": a}) });
    let out = u.frame_raw(RawInput::default());
    assert_ne!(u.app.windows.of(uid), 0, "the file moved to its own window");
    assert!(out.platform_output.ime.is_none(), "the root must not emit a descriptor for a file it no longer shows");
}

/// Asserts a CRLF-normalized commit is application-origin (`Other`), and bare-newline and
/// control-only commits insert nothing.
#[test]
fn a_transformed_commit_is_application_origin_and_a_bare_newline_does_nothing() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "Hi");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();
    ime(&mut u, vec![]);

    let normalized = ime_descriptor(&mut u, vec![Event::Ime(ImeEvent::Commit("a\r\nb".into()))]);
    assert_eq!(normalized.change_cause, ImeChangeCause::Other, "CRLF normalisation is an application transformation");
    assert_eq!(text(&u, t), "Hia\nb");
    assert_eq!(available(&normalized).text, "Hia\nb");

    let newline = ime_descriptor(&mut u, vec![Event::Ime(ImeEvent::Commit("\n".into()))]);
    assert_eq!(text(&u, t), "Hia\nb", "stock TextEdit ignores a bare newline commit");
    assert!(matches!(newline.surrounding_text, IMESurroundingText::Available(_)));
    let control = ime_descriptor(&mut u, vec![Event::Ime(ImeEvent::Commit("\u{0}\u{7}".into()))]);
    assert_eq!(text(&u, t), "Hia\nb", "a control-only commit inserts nothing");
    assert!(matches!(control.surrounding_text, IMESurroundingText::Available(_)));
}

/// Asserts character deletion keeps the selection for the paired commit, which replaces it
/// and reports `InputMethod`.
#[test]
fn ime_delete_surrounding_chars_keeps_the_selection_for_the_paired_commit() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let t = text_layer(&mut s, "ab\u{4F60}\u{597D}cd");
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, t, Some(2), false).unwrap();
    figcraft_engine::textedit::set_caret(&mut u.app.session, 4, true).unwrap(); // "你好"
    ime(&mut u, vec![]);

    let out = ime_descriptor(
        &mut u,
        vec![Event::Ime(ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 1 }), Event::Ime(ImeEvent::Commit("X".into()))],
    );
    // The delete removes "b" and "c"; the selected "你好" stays for the commit, which replaces it.
    assert_eq!(text(&u, t), "aXd");
    assert_eq!(available(&out).text, "aXd");
    assert_eq!(available(&out).cursor, 2);
    assert_eq!(out.change_cause, ImeChangeCause::InputMethod);
}
