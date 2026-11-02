//! Range fills on text: set on a range, cleared back to the layer's fill; a layer-level fill edit
//! reaches every character.

use serde_json::json;

use crate::Session;
use crate::cmd::lists::character_fills;
use crate::params::id_of;

/// The fill each character of `t` shows, as hex.
fn fills_shown(s: &Session, t: figcraft_doc::NodeId) -> Vec<String> {
    let n = s.doc().unwrap().node(t).unwrap().clone();
    let tx = n.kind.text().unwrap().clone();
    (0..tx.char_count())
        .map(|i| {
            tx.style_at(i)
                .fills
                .clone()
                .unwrap_or_else(|| n.fills.clone())
                .iter()
                .map(|p| p.solid_color().unwrap().to_hex())
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect()
}

/// A text styled on a range, or on all of its characters, keeps those fills until the layer's
/// fill is edited: then every character takes the layer's fills, in one undo step (#1856).
#[test]
fn a_layer_fill_edit_reaches_every_character() {
    let mut s = Session::new();
    let t = id_of(&s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    s.execute("fill.set", &json!({"ids": [t], "color": "#FF0000"})).unwrap();
    let node = |s: &Session| s.doc().unwrap().node(t).unwrap().clone();
    assert_eq!(character_fills(&node(&s)).map(|f| f.len()), Some(1), "one shared list");
    // "ello" yellow: the characters differ, the layer's fill is mixed, the canvas keeps both.
    s.execute("text.style", &json!({"id": t, "start": 1, "end": 5, "textFills": ["#FFFF00"]})).unwrap();
    assert_eq!(fills_shown(&s, t), ["FF0000", "FFFF00", "FFFF00", "FFFF00", "FFFF00"]);
    assert_eq!(character_fills(&node(&s)), None, "mixed");
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    assert_eq!(fills_shown(&s, t), ["FF0000", "FFFF00", "FFFF00", "FFFF00", "FFFF00"], "reselecting changes nothing");
    // The layer's fill set: every character takes it, their own fills are gone.
    let steps = s.ds().unwrap().history.undo.len();
    s.execute("fill.set", &json!({"ids": [t], "color": "#0000FF"})).unwrap();
    assert_eq!(fills_shown(&s, t), ["0000FF"; 5]);
    assert!(node(&s).kind.text().unwrap().runs.is_empty(), "no character keeps fills of its own");
    assert_eq!(s.ds().unwrap().history.undo.len(), steps + 1, "one undo step");
    s.undo().unwrap();
    assert_eq!(fills_shown(&s, t), ["FF0000", "FFFF00", "FFFF00", "FFFF00", "FFFF00"], "undo puts the characters' fills back");
    s.redo().unwrap();
    // All of the characters styled while editing: they share a list the layer doesn't have yet.
    // A fill added at the layer goes on top of that list, not the layer's old one.
    s.execute("text.style", &json!({"id": t, "start": 0, "end": 5, "textFills": ["#00FF00"]})).unwrap();
    assert_eq!(fills_shown(&s, t), ["00FF00"; 5]);
    assert_eq!(character_fills(&node(&s)).map(|f| f.len()), Some(1), "one shared list, the characters'");
    assert_eq!(node(&s).fills[0].solid_color().unwrap().to_hex(), "0000FF", "the layer's own list is older");
    s.execute("fill.add", &json!({"ids": [t], "color": "#FF00FF"})).unwrap();
    assert_eq!(fills_shown(&s, t), ["00FF00+FF00FF"; 5]);
    assert!(node(&s).kind.text().unwrap().runs.is_empty());
    assert_eq!(node(&s).fills.len(), 2, "the layer's list is the characters' now");
    // `node.set` with a fill, and a mixed text again: both reach every character.
    s.execute("text.style", &json!({"id": t, "start": 1, "end": 5, "textFills": ["#FFFF00"]})).unwrap();
    s.execute("node.set", &json!({"id": t, "props": {"fill": "#123456"}})).unwrap();
    assert_eq!(fills_shown(&s, t), ["123456"; 5]);
    // A removal that names no paint leaves the characters' own fills alone; a copy takes the fill
    // the characters show, the list the Design panel's rows are.
    s.execute("text.style", &json!({"id": t, "start": 1, "end": 5, "textFills": ["#FFFF00"]})).unwrap();
    s.execute("fill.remove", &json!({"ids": [t], "index": 7})).unwrap();
    assert_eq!(fills_shown(&s, t), ["123456", "FFFF00", "FFFF00", "FFFF00", "FFFF00"], "nothing removed, nothing taken");
    // Characters coloured differently show no fill rows at all: a removal with or without an
    // index leaves the layer's hidden fill and the characters' own alone, and there is no row to
    // crop (the command is unavailable, as the menu has it).
    s.execute("fill.remove", &json!({"ids": [t], "index": 0})).unwrap();
    s.execute("fill.remove", &json!({"ids": [t]})).unwrap();
    assert_eq!(fills_shown(&s, t), ["123456", "FFFF00", "FFFF00", "FFFF00", "FFFF00"], "mixed: nothing to remove");
    assert_eq!(node(&s).fills.len(), 1, "the layer's own fill stays");
    assert!(matches!(s.execute("image.crop", &json!({"ids": [t]})), Err(crate::EngineError::Disabled(..))), "mixed: nothing to crop");
    assert_eq!(fills_shown(&s, t), ["123456", "FFFF00", "FFFF00", "FFFF00", "FFFF00"], "and nothing taken");
    assert_eq!(crate::cmd::image::top_image_fill(&node(&s)), None, "the menu finds no image to crop either");
    s.execute("text.style", &json!({"id": t, "start": 0, "end": 5, "textFills": ["#00FF00"]})).unwrap();
    s.execute("fill.copy", &json!({"id": t})).unwrap();
    let copied = s.clipboard.paint.clone().and_then(|(_, p)| p.solid_color()).map(|c| c.to_hex());
    assert_eq!(copied.as_deref(), Some("00FF00"), "the characters' fill is copied, not the layer's older one");
    // A rectangle is left alone by all of this.
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"]).unwrap();
    s.execute("fill.set", &json!({"ids": [r], "color": "#ABCDEF"})).unwrap();
    assert_eq!(character_fills(s.doc().unwrap().node(r).unwrap()).map(|f| f.len()), Some(1));
}

#[test]
fn range_fill_sets_and_clears() {
    let mut s = Session::new();
    let t = id_of(&s.execute("text.create", &json!({"text": "Hello brand world", "x": 0, "y": 0})).unwrap()["id"]).unwrap();
    s.execute("text.style", &json!({"id": t, "start": 6, "end": 11, "textFills": ["#0D99FF"]})).unwrap();
    let fill_at = |s: &Session, i: usize| {
        let n = s.doc().unwrap().node(t).unwrap().clone();
        let tx = n.kind.text().unwrap().clone();
        tx.style_at(i).fills.clone().unwrap_or(n.fills).first().and_then(|p| p.solid_color()).map(|c| c.to_hex())
    };
    assert_eq!(fill_at(&s, 7), Some("0D99FF".into()));
    assert_ne!(fill_at(&s, 1), Some("0D99FF".into()));
    s.execute("text.style", &json!({"id": t, "start": 6, "end": 11, "textFills": null})).unwrap();
    assert_eq!(fill_at(&s, 7), fill_at(&s, 1), "back to the layer's fill");
    s.undo().unwrap();
    assert_eq!(fill_at(&s, 7), Some("0D99FF".into()));
}
