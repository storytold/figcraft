//! Text styles on chars (the text style browser's commands): apply to layers, ranges or the
//! selected chars being edited, edits unlink what they change, style edits follow the links,
//! detach, delete, create from selected chars, old files; live previews of styles and axis values.

use figcraft_doc::{NodeId, StyleKind, TextData};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn text(s: &mut Session, chars: &str) -> NodeId {
    id_of(&s.execute("text.create", &json!({"text": chars, "x": 0, "y": 0})).unwrap()["id"]).unwrap()
}

fn data(s: &Session, id: NodeId) -> TextData {
    s.doc().unwrap().node(id).unwrap().kind.text().unwrap().clone()
}

fn layer_link(s: &Session, id: NodeId) -> Option<String> {
    s.doc().unwrap().node(id).unwrap().styles.text.clone()
}

/// The style link of each char.
fn links(s: &Session, id: NodeId) -> Vec<Option<String>> {
    let t = data(s, id);
    (0..t.char_count()).map(|i| t.style_at(i).style_id.clone()).collect()
}

/// A text style "Heading" (Inter Bold 32) made from a layer.
fn heading(s: &mut Session) -> String {
    let src = text(s, "Heading");
    s.execute("node.set", &json!({"ids": [src], "props": {"fontSize": 32, "fontStyle": "Bold"}})).unwrap();
    let id = s.execute("style.create", &json!({"type": "text", "name": "Type/Heading", "from": src})).unwrap()["id"].as_str().unwrap().to_string();
    s.execute("edit.delete", &json!({"ids": [src]})).unwrap();
    id
}

fn new_session() -> Session {
    Session::new()
}

#[test]
fn styles_apply_to_layers_and_ranges() {
    let mut s = new_session();
    let h = heading(&mut s);
    let t = text(&mut s, "Hello world");
    // A red word keeps its colour when the whole layer takes the style.
    s.execute("text.style", &json!({"id": t, "start": 6, "end": 11, "textFills": [{"type": "solid", "color": "#ff0000"}]})).unwrap();
    s.execute("style.apply", &json!({"ids": [t], "style": h})).unwrap();
    assert_eq!(layer_link(&s, t).as_deref(), Some(h.as_str()));
    assert!(links(&s, t).iter().all(|l| l.as_deref() == Some(h.as_str())));
    let d = data(&s, t);
    assert_eq!(d.style_at(0).font_size, 32.0);
    assert_eq!(d.style_at(8).font_style, "Bold");
    assert!(d.style_at(8).fills.is_some(), "range fills stay");
    // A range takes another style: the layer as a whole is linked to none.
    let b = s
        .execute(
            "style.create",
            &json!({"type": "text", "name": "Type/Body", "value": {"fontFamily": "Inter", "fontStyle": "Regular", "fontSize": 14}}),
        )
        .unwrap();
    let b = b["id"].as_str().unwrap().to_string();
    s.execute("style.apply", &json!({"ids": [t], "style": b, "start": 0, "end": 5})).unwrap();
    assert_eq!(layer_link(&s, t), None);
    let l = links(&s, t);
    assert_eq!(l[0].as_deref(), Some(b.as_str()));
    assert_eq!(l[6].as_deref(), Some(h.as_str()));
    assert_eq!(data(&s, t).style_at(0).font_size, 14.0);
}

#[test]
fn editing_text_applies_to_the_selected_chars() {
    let mut s = new_session();
    let h = heading(&mut s);
    let t = text(&mut s, "Hello world");
    s.execute("text.edit.begin", &json!({"id": t, "caret": 0})).unwrap();
    s.execute("text.edit.caret", &json!({"index": 5, "extend": true})).unwrap();
    s.execute("style.apply", &json!({"style": h})).unwrap();
    let l = links(&s, t);
    assert!(l[..5].iter().all(|x| x.as_deref() == Some(h.as_str())));
    assert!(l[5..].iter().all(Option::is_none));
    // A new style from the selected chars links them.
    s.execute("text.edit.caret", &json!({"index": 6})).unwrap();
    s.execute("text.edit.caret", &json!({"index": 11, "extend": true})).unwrap();
    s.execute("text.style", &json!({"id": t, "start": 6, "end": 11, "fontSize": 20})).unwrap();
    let c = s.execute("style.create", &json!({"type": "text", "name": "Type/Caption"})).unwrap()["id"].as_str().unwrap().to_string();
    let style = s.doc().unwrap().style(&c).cloned().unwrap();
    let StyleKind::Text { style: def } = style.kind else { panic!("not a text style") };
    assert_eq!(def.font_size, 20.0);
    assert_eq!(def.style_id, None, "definitions link to nothing");
    assert_eq!(links(&s, t)[8].as_deref(), Some(c.as_str()));
    // Detach the selected chars only.
    s.execute("style.detach", &json!({"which": "text"})).unwrap();
    assert_eq!(links(&s, t)[8], None);
    assert_eq!(links(&s, t)[0].as_deref(), Some(h.as_str()));
    s.execute("text.edit.end", &json!({})).unwrap();
    // The whole edit is one undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(links(&s, t).iter().all(Option::is_none));
}

#[test]
fn edits_unlink_and_style_edits_follow() {
    let mut s = new_session();
    let h = heading(&mut s);
    let a = text(&mut s, "One");
    let b = text(&mut s, "Two");
    s.execute("style.apply", &json!({"ids": [a, b], "style": h})).unwrap();
    // Changing what the style sets detaches the layer (its values stay).
    s.execute("node.set", &json!({"ids": [b], "props": {"fontSize": 50}})).unwrap();
    assert_eq!(layer_link(&s, b), None);
    assert!(links(&s, b).iter().all(Option::is_none));
    assert_eq!(layer_link(&s, a).as_deref(), Some(h.as_str()));
    // A colour isn't part of a text style: the link stays.
    s.execute("text.style", &json!({"id": a, "start": 0, "end": 1, "textFills": [{"type": "solid", "color": "#00ff00"}]})).unwrap();
    assert_eq!(layer_link(&s, a).as_deref(), Some(h.as_str()));
    // Redefining the style moves linked chars only.
    let mut def = match &s.doc().unwrap().style(&h).unwrap().kind {
        StyleKind::Text { style } => style.clone(),
        _ => panic!("not a text style"),
    };
    def.font_size = 40.0;
    s.execute("style.update", &json!({"style": h, "value": serde_json::to_value(&def).unwrap()})).unwrap();
    assert_eq!(data(&s, a).style_at(2).font_size, 40.0);
    assert_eq!(data(&s, b).style_at(0).font_size, 50.0);
    assert_eq!(layer_link(&s, a).as_deref(), Some(h.as_str()), "following the style keeps the link");
    // Deleting the style unlinks every char.
    s.execute("style.delete", &json!({"style": h})).unwrap();
    assert!(links(&s, a).iter().all(Option::is_none));
    assert_eq!(layer_link(&s, a), None);
    assert_eq!(data(&s, a).style_at(0).font_size, 40.0, "values stay");
}

#[test]
fn files_from_before_char_links_link_their_chars() {
    let mut s = new_session();
    let h = heading(&mut s);
    let t = text(&mut s, "Old");
    s.execute("style.apply", &json!({"ids": [t], "style": h})).unwrap();
    // An old file: the layer is linked, its chars carry nothing.
    let mut doc = s.doc().unwrap().clone();
    let n = doc.get_mut(t).unwrap();
    if let figcraft_doc::NodeKind::Text(td) = &mut n.kind {
        let len = td.char_count();
        td.unlink_text_style(0, len);
    }
    n.styles.text = Some(h.clone());
    let s2 = Session::with_document(doc);
    assert!(links(&s2, t).iter().all(|l| l.as_deref() == Some(h.as_str())));
    assert_eq!(layer_link(&s2, t).as_deref(), Some(h.as_str()));
    // Saved and read back, the links survive.
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (back, _) = figcraft_format::load_bytes(&bytes).unwrap();
    assert_eq!(back.node(t).unwrap().kind.text().unwrap().style.style_id.as_deref(), Some(h.as_str()));
}

#[test]
fn previews_show_without_an_undo_step() {
    let mut s = new_session();
    let h = heading(&mut s);
    let t = text(&mut s, "Preview me");
    s.ds_mut().unwrap().mark_saved();
    let undo = s.ds().unwrap().history.undo.len();
    let before = s.doc().unwrap().clone();
    s.execute("text.preview", &json!({"command": "style.apply", "params": {"ids": [t], "style": h}})).unwrap();
    assert_eq!(data(&s, t).style_at(0).font_size, 32.0, "the canvas shows the style");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo);
    assert!(!s.ds().unwrap().is_dirty(), "a preview isn't an edit");
    // Another preview replaces it (from the document, not on top of the first).
    s.execute("text.preview", &json!({"command": "text.setVariations", "params": {"ids": [t], "variations": {"wght": 650}}})).unwrap();
    assert_eq!(data(&s, t).style_at(0).font_size, before.node(t).unwrap().kind.text().unwrap().style.font_size);
    assert_eq!(data(&s, t).style_at(0).variations.get("wght"), Some(&650.0));
    // Ending it puts the document back as it was.
    assert_eq!(s.execute("text.endPreview", &json!({})).unwrap()["ended"], json!(true));
    assert_eq!(*s.doc().unwrap(), before);
    assert!(!s.ds().unwrap().is_dirty());
    assert_eq!(s.execute("text.endPreview", &json!({})).unwrap()["ended"], json!(false));
    // Committing after a preview is one undo step on the document itself.
    s.execute("text.preview", &json!({"command": "style.apply", "params": {"ids": [t], "style": h}})).unwrap();
    s.execute("style.apply", &json!({"ids": [t], "style": h})).unwrap();
    assert_eq!(s.ds().unwrap().history.undo.len(), undo + 1);
    assert!(s.ds().unwrap().text_preview.is_none());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap(), before);
    // Any other command ends a preview first.
    s.execute("text.preview", &json!({"command": "style.apply", "params": {"ids": [t], "style": h}})).unwrap();
    s.execute("node.set", &json!({"ids": [t], "props": {"name": "Renamed"}})).unwrap();
    assert_eq!(data(&s, t).style_at(0).font_size, before.node(t).unwrap().kind.text().unwrap().style.font_size);
    assert!(s.ds().unwrap().text_preview.is_none());
}

#[test]
fn previews_while_editing_keep_the_edit() {
    let mut s = new_session();
    let h = heading(&mut s);
    let t = text(&mut s, "Hello world");
    s.execute("text.edit.begin", &json!({"id": t, "caret": 0})).unwrap();
    s.execute("text.edit.caret", &json!({"index": 5, "extend": true})).unwrap();
    let edit = s.ds().unwrap().text_edit.clone();
    s.execute("text.preview", &json!({"command": "style.apply", "params": {"style": h}})).unwrap();
    assert_eq!(links(&s, t)[0].as_deref(), Some(h.as_str()));
    assert_eq!(links(&s, t)[6], None);
    assert_eq!(s.ds().unwrap().text_edit, edit);
    s.execute("text.endPreview", &json!({})).unwrap();
    assert_eq!(s.ds().unwrap().text_edit, edit);
    assert!(links(&s, t).iter().all(Option::is_none));
    // Typing ends it too, and the edit goes on.
    s.execute("text.preview", &json!({"command": "style.apply", "params": {"style": h}})).unwrap();
    s.execute("text.edit.insert", &json!({"text": "Bye"})).unwrap();
    assert_eq!(data(&s, t).characters, "Bye world");
    assert!(links(&s, t).iter().all(Option::is_none));
}

#[test]
fn previews_show_a_hovered_font_family() {
    let mut s = new_session();
    let t = text(&mut s, "Preview me");
    s.ds_mut().unwrap().mark_saved();
    let before = s.doc().unwrap().clone();
    let undo = s.ds().unwrap().history.undo.len();
    // Hovering a font family shows it on the canvas: no undo step, not dirty.
    s.execute("text.preview", &json!({"command": "text.style", "params": {"ids": [t], "fontFamily": "Arial"}})).unwrap();
    assert_eq!(data(&s, t).style_at(0).font_family, "Arial");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo);
    assert!(!s.ds().unwrap().is_dirty());
    // Moving to another family replaces the preview, from the document itself.
    s.execute("text.preview", &json!({"command": "text.style", "params": {"ids": [t], "fontFamily": "Georgia"}})).unwrap();
    assert_eq!(data(&s, t).style_at(0).font_family, "Georgia");
    // Closing without picking puts the document back as it was.
    assert_eq!(s.execute("text.endPreview", &json!({})).unwrap()["ended"], json!(true));
    assert_eq!(*s.doc().unwrap(), before);
    // Clicking commits it as one undo step.
    s.execute("text.style", &json!({"ids": [t], "fontFamily": "Georgia"})).unwrap();
    assert_eq!(data(&s, t).style_at(0).font_family, "Georgia");
    assert_eq!(s.ds().unwrap().history.undo.len(), undo + 1);
    assert!(s.ds().unwrap().text_preview.is_none());
}

#[test]
fn previews_refuse_what_they_cant_show() {
    let mut s = new_session();
    let t = text(&mut s, "x");
    for p in [
        json!({}),
        json!({"command": 5}),
        json!({"command": "edit.delete", "params": {"ids": [t]}}),
        json!({"command": "text.preview", "params": {}}),
        json!({"command": "style.apply", "params": {"ids": [t], "style": "nope"}}),
        json!({"command": "text.setVariations", "params": {"ids": [t], "variations": "junk"}}),
        json!({"command": "style.apply", "params": Value::Null}),
    ] {
        let doc = s.doc().unwrap().clone();
        assert!(s.execute("text.preview", &p).is_err(), "{p}");
        assert_eq!(*s.doc().unwrap(), doc);
        assert!(s.ds().unwrap().text_preview.is_none());
    }
    assert!(s.doc().unwrap().contains(t));
}
