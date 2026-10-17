//! Text behaviour suite, the tail, through commands: lists per paragraph (the list buttons and
//! shortcuts, typed list starts, Tab / Shift+Tab / Enter / Backspace at list items), list spacing,
//! hanging punctuation and lists, and decoration styles through `node.set`, inspect, CSS and the
//! REST import. Each case gives the rule and a one-line derivation from the public documentation;
//! "silent" marks a case where the documentation says nothing and today's behaviour is pinned.
//! Everything here is font-free.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::textedit::{begin, insert, key, set_caret};
use figcraft_doc::{DecorationLength, DecorationStyle, ListStyle, NodeId, ParaList, TextData, color::Color};

fn text(s: &mut Session, chars: &str) -> NodeId {
    id_of(&s.execute("text.create", &json!({"text": chars, "x": 0, "y": 0, "props": {"fontSize": 20}})).unwrap()["id"]).unwrap()
}

fn data(s: &Session, t: NodeId) -> TextData {
    s.doc().unwrap().node(t).unwrap().kind.text().unwrap().clone()
}

/// Each paragraph's (list, level).
fn lists(s: &Session, t: NodeId) -> Vec<(ListStyle, u8)> {
    let d = data(s, t);
    (0..d.paragraph_count()).map(|p| d.para_list(p)).map(|p| (p.list, p.indent)).collect()
}

use ListStyle::{Bulleted as B, None as P, Numbered as N};

#[test]
fn list_buttons_act_on_the_selected_paragraphs() {
    // The list buttons and shortcuts turn the paragraphs the selection touches into list items
    // (the whole layer when it's selected but not being edited); again turns them back.
    let mut s = Session::new();
    let t = text(&mut s, "one\ntwo\nthree");
    begin(&mut s, t, Some(5), false).unwrap();
    // (command, selection, lists after, derivation)
    let cases = [
        ("text.bulletList", (5, 5), [(P, 0), (B, 0), (P, 0)], "a caret: its paragraph"),
        ("text.bulletList", (5, 5), [(P, 0); 3], "again: back to plain"),
        ("text.numberList", (1, 8), [(N, 0), (N, 0), (P, 0)], "a selection: every paragraph it touches (not one it ends at the start of)"),
        ("text.bulletList", (0, 13), [(B, 0); 3], "mixed: all become bulleted"),
        ("text.numberList", (2, 2), [(N, 0), (B, 0), (B, 0)], "the other list kind replaces it"),
    ];
    for (cmd, (a, b), want, why) in cases {
        set_caret(&mut s, a, false).unwrap();
        set_caret(&mut s, b, true).unwrap();
        s.execute(cmd, &json!({})).unwrap();
        assert_eq!(lists(&s, t), want, "{why}");
    }
    crate::textedit::end(&mut s).unwrap();
    s.execute("text.listStyle", &json!({"ids": [t], "style": "bulleted"})).unwrap();
    assert_eq!(lists(&s, t), [(B, 0); 3], "not editing: the whole layer");
    assert!(data(&s, t).paragraphs.is_empty(), "one style for all folds into the layer's list style");
    s.execute("text.bulletList", &json!({"ids": [t]})).unwrap();
    assert_eq!(data(&s, t).list, P, "toggled off");
}

#[test]
fn a_list_with_a_start_number_toggles_off() {
    // An imported list that starts at 2 has the start on its first item only; it is still one
    // numbered list, so the button turns it back to plain (#686).
    let mut s = Session::new();
    let t = text(&mut s, "two\nthree\nfour");
    if let figcraft_doc::NodeKind::Text(td) = &mut s.doc_mut().unwrap().get_mut(t).unwrap().kind {
        let n = |start_offset| ParaList { list: N, indent: 0, start_offset };
        td.paragraphs = vec![n(1), n(0), n(0)];
    }
    let j = crate::inspect::node_json(s.doc().unwrap(), t, 0);
    assert_eq!(j["listStartOffsets"], json!([1, 0, 0]), "inspect reports the start");
    s.execute("text.numberList", &json!({"ids": [t]})).unwrap();
    assert_eq!(lists(&s, t), [(P, 0); 3]);
    assert_eq!(data(&s, t).para_list(0).start_offset, 0, "a plain paragraph has no start");
    assert!(crate::inspect::node_json(s.doc().unwrap(), t, 0).get("listStartOffsets").is_none());
}

#[test]
fn typed_list_starts() {
    // Typing "- " or "* " at the start of a paragraph starts a bulleted list, "1. " a numbered one
    // (the typed marker goes); elsewhere they're plain text.
    // (typed, chars after, list, derivation)
    let cases = [
        ("- ", "", B, "hyphen and space: bullets"),
        ("* ", "", B, "asterisk and space: bullets"),
        ("1. ", "", N, "one, full stop, space: numbers"),
        ("2. ", "2. ", P, "silent: only 1. starts a numbered list"),
        ("a- ", "a- ", P, "not at the paragraph's start: text"),
        ("-x ", "-x ", P, "the space must follow the marker"),
    ];
    for (typed, want, list, why) in cases {
        let mut s = Session::new();
        let t = text(&mut s, "x");
        s.execute("node.set", &json!({"id": t, "props": {"characters": ""}})).unwrap();
        begin(&mut s, t, Some(0), false).unwrap();
        for c in typed.chars() {
            insert(&mut s, &c.to_string()).unwrap();
        }
        assert_eq!((data(&s, t).characters.as_str(), lists(&s, t)[0].0), (want, list), "{why}");
    }
    // In the second paragraph, and not again inside a list item.
    let mut s = Session::new();
    let t = text(&mut s, "Intro\n");
    begin(&mut s, t, Some(6), false).unwrap();
    for c in "- - ".chars() {
        insert(&mut s, &c.to_string()).unwrap();
    }
    assert_eq!((data(&s, t).characters.as_str(), lists(&s, t)), ("Intro\n- ", vec![(P, 0), (B, 0)]), "an item's own text stays text");
}

#[test]
fn list_keys_at_list_items() {
    // (chars, lists before, caret, key, shift, chars after, lists after, derivation)
    let cases = [
        ("a\nb", vec![(B, 0), (B, 0)], 3, "Tab", false, "a\nb", vec![(B, 0), (B, 1)], "Tab nests the item one level"),
        ("a\nb", vec![(B, 0), (B, 1)], 3, "Tab", true, "a\nb", vec![(B, 0), (B, 0)], "Shift+Tab moves it out a level"),
        ("a\nb", vec![(B, 0), (B, 0)], 3, "Tab", true, "a\nb", vec![(B, 0), (P, 0)], "silent: Shift+Tab at the top leaves the list"),
        ("a\nb", vec![(N, 0), (N, 0)], 1, "Enter", false, "a\n\nb", vec![(N, 0), (N, 0), (N, 0)], "Enter in an item starts the next item"),
        ("a\n", vec![(N, 0), (N, 0)], 2, "Enter", false, "a\n", vec![(N, 0), (P, 0)], "silent: Enter on an empty item ends the list"),
        ("a\n", vec![(N, 0), (N, 1)], 2, "Enter", false, "a\n", vec![(N, 0), (N, 0)], "silent: ...a nested one moves out a level first"),
        (
            "a\nb",
            vec![(B, 0), (B, 0)],
            2,
            "Backspace",
            false,
            "a\nb",
            vec![(B, 0), (P, 0)],
            "silent: Backspace at an item's start removes its bullet",
        ),
        ("a\nbc", vec![(B, 0), (B, 0)], 4, "Backspace", false, "a\nb", vec![(B, 0), (B, 0)], "Backspace inside an item deletes text"),
        ("a\nb", vec![(P, 0), (P, 0)], 3, "Tab", false, "a\nb\t", vec![(P, 0), (P, 0)], "outside a list Tab types a tab"),
        ("a", vec![(B, 8)], 1, "Tab", false, "a", vec![(B, 8)], "silent: nesting stops at 8 levels"),
    ];
    for (chars, before, caret, k, shift, after, want, why) in cases {
        let mut s = Session::new();
        let t = text(&mut s, chars);
        let ps: Vec<ParaList> = before.iter().map(|(list, indent)| ParaList { list: *list, indent: *indent, start_offset: 0 }).collect();
        if let figcraft_doc::NodeKind::Text(td) = &mut s.doc_mut().unwrap().get_mut(t).unwrap().kind {
            td.paragraphs = ps;
        }
        begin(&mut s, t, Some(caret), false).unwrap();
        key(&mut s, k, shift, false, false).unwrap();
        assert_eq!((data(&s, t).characters.as_str(), lists(&s, t)), (after, want), "{why}");
    }
    // A whole editing session is one undo step.
    let mut s = Session::new();
    let t = text(&mut s, "a\nb");
    s.execute("text.listStyle", &json!({"ids": [t], "style": "bulleted"})).unwrap();
    begin(&mut s, t, Some(3), false).unwrap();
    key(&mut s, "Tab", false, false, false).unwrap();
    crate::textedit::end(&mut s).unwrap();
    s.undo().unwrap();
    assert_eq!(lists(&s, t), [(B, 0), (B, 0)]);
}

#[test]
fn paragraph_lists_follow_edits() {
    // A paragraph's list stays with it as text is typed, pasted and deleted: new paragraphs copy
    // the one they split from; joined paragraphs keep the first one's.
    let mut t = TextData::new("ab\ncd\nef");
    t.paragraphs = vec![ParaList { list: B, indent: 1, start_offset: 0 }, ParaList::default(), ParaList { list: N, indent: 0, start_offset: 0 }];
    let get = |t: &TextData| (0..t.paragraph_count()).map(|p| (t.para_list(p).list, t.para_list(p).indent)).collect::<Vec<_>>();
    t.replace_range(1, 1, "x\ny");
    assert_eq!(get(&t), [(B, 1), (B, 1), (P, 0), (N, 0)], "a pasted newline: a new item like it");
    t.replace_range(2, 7, "");
    assert_eq!((t.characters.as_str(), get(&t)), ("axd\nef", vec![(B, 1), (N, 0)]), "joined: the first paragraph's list");
    t.replace_range(0, t.char_count(), "z");
    assert_eq!(get(&t), [(B, 1)]);
    // A deterministic edit sweep: the per-paragraph list always matches the paragraph count.
    let mut seed = 7u64;
    let mut rnd = |n: usize| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize % n.max(1)
    };
    let mut t = TextData::new("a\nb\nc\nd");
    t.paragraphs = vec![ParaList { list: B, indent: 2, start_offset: 0 }; 4];
    for _ in 0..500 {
        let n = t.char_count();
        let (a, b) = (rnd(n + 2), rnd(n + 2));
        let ins = ["", "\n", "q", "\n\n", "x\ny"][rnd(5)];
        t.replace_range(a.min(b), a.max(b), ins);
        if !t.paragraphs.is_empty() {
            assert_eq!(t.paragraphs.len(), t.paragraph_count(), "{:?}", t.characters);
        }
        t.set_para_lists(rnd(n + 2), rnd(n + 2), |p| p.indent = 200);
        assert!((0..t.paragraph_count()).all(|p| t.para_list(p).indent <= figcraft_doc::MAX_LIST_INDENT));
    }
}

#[test]
fn node_set_keys_for_agents() {
    let mut s = Session::new();
    let t = text(&mut s, "one\ntwo\nthree");
    let set = |s: &mut Session, props: Value| s.execute("node.set", &json!({"id": t, "props": props}));
    // (props, derivation)
    set(&mut s, json!({"listStyle": "numbered", "start": 4, "end": 13})).unwrap();
    assert_eq!(lists(&s, t), [(P, 0), (N, 0), (N, 0)], "start/end: the paragraphs those chars touch");
    set(&mut s, json!({"indentation": 2, "start": 9, "end": 9})).unwrap();
    assert_eq!(lists(&s, t), [(P, 0), (N, 0), (N, 2)], "indentation: the list level");
    set(&mut s, json!({"indentation": 3})).unwrap();
    assert_eq!(lists(&s, t), [(P, 0), (N, 3), (N, 3)], "plain paragraphs don't nest");
    set(&mut s, json!({"listSpacing": 6, "hangingPunctuation": true, "hangingList": true})).unwrap();
    let d = data(&s, t);
    assert_eq!((d.list_spacing, d.hanging_punctuation, d.hanging_list), (6.0, true, true));
    set(
        &mut s,
        json!({"textDecoration": "underline", "textDecorationStyle": "wavy", "textDecorationThickness": 2, "textDecorationOffset": "10%",
               "textDecorationSkipInk": true, "textDecorationColor": "#FF0000", "start": 0, "end": 3}),
    )
    .unwrap();
    let d = data(&s, t);
    let look = d.style_at(0).look();
    assert_eq!(
        (look.style, look.thickness, look.offset, look.skip_ink, look.color),
        (DecorationStyle::Wavy, DecorationLength::Pixels(2.0), DecorationLength::Percent(10.0), true, Some(Color::new(1.0, 0.0, 0.0, 1.0)))
    );
    assert!(d.style_at(5).decoration_look.is_none(), "only the range");
    set(&mut s, json!({"textDecorationThickness": "auto", "textDecorationColor": "auto", "textDecorationStyle": "solid", "textDecorationSkipInk": false, "textDecorationOffset": "auto"})).unwrap();
    assert!(data(&s, t).spans().iter().all(|(_, _, st)| st.decoration_look.is_none()), "auto everywhere: the default look, stored as nothing");
    // Junk is refused, nothing changes.
    let before = data(&s, t);
    for junk in [
        json!({"listStyle": "zigzag"}),
        json!({"indentation": "deep"}),
        json!({"hangingList": 3}),
        json!({"textDecorationStyle": "dashed"}),
        json!({"textDecorationThickness": "thick"}),
        json!({"textDecorationOffset": 1e300}),
        json!({"textDecorationSkipInk": "yes"}),
        json!({"textDecorationColor": "nope"}),
    ] {
        assert!(set(&mut s, junk.clone()).is_err(), "{junk}");
        assert_eq!(data(&s, t), before, "{junk}");
    }
    // The keys are documented for agents.
    for k in
        ["textDecorationStyle", "textDecorationSkipInk", "textDecorationColor", "indentation", "listSpacing", "hangingPunctuation", "hangingList"]
    {
        assert!(crate::cmd::props::KEYS.contains(k), "{k}");
    }
}

#[test]
fn inspect_css_and_files() {
    let mut s = Session::new();
    let t = text(&mut s, "one\ntwo");
    s.execute("node.set", &json!({"id": t, "props": {"listStyle": "bulleted", "start": 4, "end": 4, "listSpacing": 4, "hangingList": true}}))
        .unwrap();
    s.execute(
        "node.set",
        &json!({"id": t, "props": {"textDecoration": "underline", "textDecorationStyle": "dotted", "textDecorationThickness": 2, "textDecorationOffset": 3, "textDecorationColor": "#00FF00"}}),
    )
    .unwrap();
    let doc = s.doc().unwrap();
    let j = crate::inspect::node_json(doc, t, 0);
    // (key, value, derivation)
    for (k, v, why) in [
        ("lineTypes", json!(["none", "bulleted"]), "each paragraph's list"),
        ("lineIndentations", json!([0, 0]), "and its level"),
        ("listSpacing", json!(4.0), "list spacing"),
        ("hangingList", json!(true), "hanging list"),
        ("textDecorationStyle", json!("dotted"), "decoration style"),
        ("textDecorationThickness", json!("2"), "thickness"),
        ("textDecorationOffset", json!("3"), "offset"),
        ("textDecorationColor", json!("#00FF00"), "colour"),
    ] {
        assert_eq!(j[k], v, "{why}");
    }
    let n = doc.node(t).unwrap();
    let css = crate::codegen::css_decls(doc, n, &mut Default::default(), (true, true));
    for want in [
        "text-decoration-style: dotted",
        "text-decoration-thickness: 2px",
        "text-underline-offset: 3px",
        "text-decoration-color: #00ff00",
        "text-decoration-skip-ink: none",
    ] {
        assert!(css.to_lowercase().contains(want), "{want} in {css}");
    }
    // Saved files keep it all and stay free of it when unused.
    let td = doc.node(t).unwrap().kind.text().unwrap();
    let back: TextData = serde_json::from_str(&serde_json::to_string(td).unwrap()).unwrap();
    assert_eq!(&back, td);
    let plain = serde_json::to_string(&TextData::new("x")).unwrap();
    for k in ["paragraphs", "list_spacing", "hanging", "decoration_look"] {
        assert!(!plain.contains(k), "{k} in {plain}");
    }
    // Renders and exports draw dotted, wavy and skipped-ink lines without trouble.
    for style in ["dotted", "wavy"] {
        s.execute("node.set", &json!({"id": t, "props": {"textDecorationStyle": style, "textDecorationSkipInk": true}})).unwrap();
        let doc = s.doc().unwrap();
        assert!(figcraft_render::render_node(doc, t, 1.0).is_some());
        assert!(figcraft_svg::export_svg(doc, &[t], &figcraft_svg::SvgOptions::default()).is_some_and(|svg| !svg.contains("NaN")));
        assert!(figcraft_pdf::export_nodes_pdf(doc, &[t]).is_ok());
    }
}

#[test]
fn rest_import_lists_and_decorations() {
    let file = json!({"document": {"type": "DOCUMENT", "children": [{"type": "CANVAS", "name": "P", "children": [{
        "type": "TEXT", "name": "T", "characters": "a\nb\nc",
        "absoluteBoundingBox": {"x": 0, "y": 0, "width": 100, "height": 60},
        "style": {"fontFamily": "Inter", "fontSize": 16, "listSpacing": 5, "hangingPunctuation": true, "hangingList": true,
                  "textDecoration": "UNDERLINE", "textDecorationStyle": "WAVY", "textDecorationSkipInk": true,
                  "textDecorationThickness": {"value": 2, "unit": "PIXELS"}, "textDecorationOffset": {"unit": "AUTO"},
                  "textDecorationColor": {"value": {"type": "SOLID", "color": {"r": 0, "g": 0, "b": 1, "a": 1}}}},
        "lineTypes": ["ORDERED", "ORDERED", "UNORDERED"], "lineIndentations": [0, 1, 0]
    }]}]}});
    let (doc, _) = figcraft_format::rest::import(&file, &figcraft_format::rest::no_images).unwrap();
    let t = doc.nodes().find_map(|n| n.kind.text()).unwrap();
    let got: Vec<(ListStyle, u8)> = (0..3).map(|p| (t.para_list(p).list, t.para_list(p).indent)).collect();
    assert_eq!(got, [(N, 0), (N, 1), (B, 0)], "each paragraph's list type and level");
    assert_eq!((t.list_spacing, t.hanging_punctuation, t.hanging_list), (5.0, true, true));
    let look = t.style.look();
    assert_eq!(
        (look.style, look.thickness, look.offset, look.skip_ink, look.color),
        (DecorationStyle::Wavy, DecorationLength::Pixels(2.0), DecorationLength::Auto, true, Some(Color::new(0.0, 0.0, 1.0, 1.0)))
    );
}
