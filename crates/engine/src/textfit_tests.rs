//! `text.fit`: the largest size that fits the box and a line limit, orphans avoided, and
//! data.fill sizing each copy's text.

use crate::Session;
use crate::components::tests::{ex, idv};
use figcraft_doc::{NodeId, NodeKind};
use serde_json::json;

/// A fixed-width (auto-height) headline of `width` px at 64px. Returns its id.
fn headline(s: &mut Session, text: &str, width: f64) -> NodeId {
    figcraft_text::scan_system_fonts();
    let t = idv(&ex(s, "text.create", json!({"text": text, "x": 0, "y": 0, "props": {"name": "Headline", "fontSize": 64}})));
    ex(s, "node.set", json!({"id": t, "props": {"width": width, "textAutoResize": "height"}}));
    t
}

fn size(s: &Session, id: NodeId) -> f64 {
    match &s.doc().unwrap().node(id).unwrap().kind {
        NodeKind::Text(t) => t.style.font_size,
        _ => 0.0,
    }
}

#[test]
fn text_shrinks_to_its_line_limit_and_no_further() {
    let mut s = Session::new();
    let t = headline(&mut s, "Seu site e seu WhatsApp com IA, prontos em 7 dias", 600.0);
    let r = ex(&mut s, "text.fit", json!({"ids": [t], "maxLines": 2}));
    let f = &r["fitted"][0];
    assert_eq!(f["lines"], 2, "{r}");
    assert!(f["fits"].as_bool().unwrap());
    let fitted = size(&s, t);
    assert!((64.0 * 0.6..64.0).contains(&fitted), "{fitted}");
    // The largest that fits: a pixel more breaks the limit.
    ex(&mut s, "node.set", json!({"id": t, "props": {"fontSize": fitted + 1.0}}));
    let again = ex(&mut s, "text.fit", json!({"ids": [t], "maxLines": 2}));
    assert!((again["fitted"][0]["fontSize"].as_f64().unwrap() - fitted).abs() <= 0.2, "{again}");
    // Text that already fits keeps its size; one undo restores the original.
    let short = headline(&mut s, "7 dias", 600.0);
    ex(&mut s, "text.fit", json!({"ids": [short], "maxLines": 1}));
    assert_eq!(size(&s, short), 64.0);
    // A floor that can't fit reports it instead of going below (fit never grows text).
    ex(&mut s, "node.set", json!({"id": t, "props": {"fontSize": 64}}));
    let r = ex(&mut s, "text.fit", json!({"ids": [t], "maxLines": 1, "minSize": 50}));
    assert_eq!(r["fitted"][0]["fits"], false);
    assert_eq!(size(&s, t), 50.0);
    // Text that grows to its content has no box to fit.
    let auto = idv(&ex(&mut s, "text.create", json!({"text": "x", "x": 0, "y": 0})));
    assert!(s.execute("text.fit", &json!({"ids": [auto]})).is_err());
}

#[test]
fn a_one_word_last_line_is_avoided() {
    let mut s = Session::new();
    // At 64px in 520px this breaks with "1s." alone on the last line.
    let t = headline(&mut s, "Lighthouse 100, abre em menos de 1s.", 520.0);
    let r = ex(&mut s, "text.fit", json!({"ids": [t]}));
    assert_eq!(r["fitted"][0]["orphan"], false, "{r}");
    let doc = s.doc().unwrap();
    let NodeKind::Text(tx) = &doc.node(t).unwrap().kind else { panic!() };
    let l = figcraft_text::layout(tx, Some(520.0), None);
    let last = l.lines.last().unwrap();
    let words = tx.characters.chars().skip(last.char_start).take(last.char_end - last.char_start).collect::<String>();
    assert!(l.lines.len() == 1 || words.split_whitespace().count() > 1 || words.contains('\u{a0}'), "{words:?}");
}

/// Asserts `data.fill` keeps a short copy's font size, shrinks a long copy, and leaves the
/// source text layer untouched.
#[test]
fn data_fill_sizes_each_copys_text() {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 1080, "height": 1350, "name": "Post"})));
    let t = idv(&ex(&mut s, "text.create", json!({"text": "Curto", "x": 80, "y": 80, "parent": f, "props": {"name": "Headline", "fontSize": 96}})));
    ex(&mut s, "node.set", json!({"id": t, "props": {"width": 920, "textAutoResize": "height"}}));
    let r = ex(
        &mut s,
        "data.fill",
        json!({"id": f, "rows": [
            {"Headline": {"text": "Curto", "fit": {"maxLines": 2}}},
            {"Headline": {"text": "Landing page e atendente de WhatsApp com IA, prontos em sete dias úteis", "fit": {"maxLines": 2}}}
        ]}),
    );
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    let head = |s: &Session, frame: NodeId| s.doc().unwrap().node(frame).unwrap().children[0];
    assert_eq!(size(&s, head(&s, ids[0])), 96.0, "short copy keeps its size");
    assert!(size(&s, head(&s, ids[1])) < 96.0, "long copy shrinks");
    assert_eq!(size(&s, t), 96.0, "the source is untouched");
}

#[test]
fn review_fits_is_reported_from_the_final_text() {
    let mut s = Session::new();
    for (text, width, lines) in [
        ("Seu site e seu WhatsApp com IA, prontos em 7 dias", 600.0, 2.0),
        ("Lighthouse 100, abre em menos de 1s.", 520.0, 2.0),
        ("Curto", 300.0, 1.0),
    ] {
        let t = headline(&mut s, text, width);
        let r = ex(&mut s, "text.fit", json!({"ids": [t], "maxLines": lines}));
        let f = &r["fitted"][0];
        let doc = s.doc().unwrap();
        let NodeKind::Text(tx) = &doc.node(t).unwrap().kind else { panic!() };
        let real = figcraft_text::layout(tx, Some(width), None).lines.len();
        assert_eq!(f["lines"].as_u64().unwrap() as usize, real, "{r}");
        assert_eq!(f["fits"].as_bool().unwrap(), real as f64 <= lines, "{r}");
    }
}

#[test]
fn review_sizes_off_the_tenth_grid_keep_their_size_and_floor() {
    let mut s = Session::new();
    // Fits already: 14.25px stays 14.25px, not 14.2 or 14.3.
    let t = headline(&mut s, "Curto", 600.0);
    ex(&mut s, "node.set", json!({"id": t, "props": {"fontSize": 14.25}}));
    ex(&mut s, "text.fit", json!({"ids": [t], "maxLines": 1}));
    assert_eq!(size(&s, t), 14.25);
    // Can't fit: it stops at the floor, not a tenth below it.
    let long = headline(&mut s, "Seu site e seu WhatsApp com IA, prontos em 7 dias", 200.0);
    let r = ex(&mut s, "text.fit", json!({"ids": [long], "maxLines": 1, "minSize": 12.34}));
    assert_eq!(r["fitted"][0]["fits"], false, "{r}");
    assert!(size(&s, long) >= 12.34, "{}", size(&s, long));
}

#[test]
fn review_a_one_word_paragraph_is_not_an_orphan() {
    let mut s = Session::new();
    let t = headline(&mut s, "Two words\nOne", 600.0);
    let r = ex(&mut s, "text.fit", json!({"ids": [t]}));
    assert_eq!(r["fitted"][0]["orphan"], false, "{r}");
    assert_eq!(size(&s, t), 64.0, "nothing to fix, so nothing shrinks");
    let doc = s.doc().unwrap();
    let NodeKind::Text(tx) = &doc.node(t).unwrap().kind else { panic!() };
    assert_eq!(tx.characters, "Two words\nOne", "no no-break space in an earlier paragraph");
}
