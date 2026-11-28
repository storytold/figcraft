//! List markers of right-to-left paragraphs on the right, in what the canvas and PNG export draw
//! and in the outlines the SVG export writes: the bullet ends at the box's right edge with a clear
//! gap between it and the text on its left. (The PDF export draws the same laid-out glyphs.)

use figcraft_doc::{Document, ListStyle, NodeId, NodeKind, TextAlignH, TextAutoResize, TextData};

use super::*;

const W: f64 = 300.0;

/// A 300-wide right-aligned bulleted Hebrew paragraph at the origin.
fn doc() -> (Document, NodeId, TextData) {
    let mut td = TextData::new("\u{05E9}\u{05DC}\u{05D5}\u{05DD} \u{05E2}\u{05D5}\u{05DC}\u{05DD}");
    td.style.font_size = 20.0;
    td.list = ListStyle::Bulleted;
    td.align_h = TextAlignH::Right;
    td.auto_resize = TextAutoResize::Height;
    let mut d = Document::new("rtl");
    let p = d.first_page().unwrap();
    let id = d.create(p, NodeKind::Text(td.clone().into()), "T", 0.0, 0.0, W, 40.0).unwrap();
    (d, id, td)
}

/// The bullet's span and where the text ends, from the layout; None without Hebrew fonts.
fn spans(td: &TextData) -> Option<((f64, f64), f64)> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(figcraft_text::scan_system_fonts);
    let l = figcraft_text::layout(td, Some(W), None);
    if l.glyphs.is_empty() || l.glyphs.iter().any(|g| g.gid == 0) {
        eprintln!("no font for Hebrew; skipping");
        return None;
    }
    let m = l.glyphs.iter().find(|g| g.char_start == g.char_end)?;
    let text_right = l.glyphs.iter().filter(|g| g.char_end > g.char_start).map(|g| g.x + g.advance).fold(f64::MIN, f64::max);
    Some(((m.x, m.x + m.advance), text_right))
}

#[test]
fn the_bullet_of_a_right_to_left_paragraph_draws_on_the_right() {
    let (d, id, td) = doc();
    let Some(((m0, m1), text_right)) = spans(&td) else { return };
    assert!(m0 > text_right + 8.0 && (m1 - W).abs() < 0.5, "{m0}..{m1}, text to {text_right}");
    let mut pix = Pixmap::new(W as u32, 40).unwrap();
    render_nodes_into(&d, &[id], &View { origin: (0.0, 0.0), scale: 1.0, width: W as u32, height: 40 }, &mut pix);
    let inked = |x: u32| (0..40).any(|y| pix.pixel(x, y).is_some_and(|p| p.alpha() > 64));
    assert!((m0.ceil() as u32..m1.floor() as u32).any(inked), "the bullet is drawn");
    assert!(!(text_right.ceil() as u32 + 2..m0.floor() as u32 - 1).any(inked), "nothing between the text and the bullet");
    assert!((0..text_right as u32).any(inked), "the text is drawn");
    // The outline the SVG export writes: the same gap.
    let (path, _) = draw::node_outline(&d, d.node(id).unwrap()).unwrap();
    let xs: Vec<f64> = path
        .elements()
        .iter()
        .flat_map(|e| match e {
            figcraft_geom::kurbo::PathEl::MoveTo(p) | figcraft_geom::kurbo::PathEl::LineTo(p) => vec![p.x],
            figcraft_geom::kurbo::PathEl::QuadTo(a, p) => vec![a.x, p.x],
            figcraft_geom::kurbo::PathEl::CurveTo(a, b, p) => vec![a.x, b.x, p.x],
            figcraft_geom::kurbo::PathEl::ClosePath => vec![],
        })
        .collect();
    assert!(xs.iter().any(|x| (m0..=m1).contains(x)));
    assert!(!xs.iter().any(|x| *x > text_right + 1.0 && *x < m0 - 1.0), "{xs:?}");
}
