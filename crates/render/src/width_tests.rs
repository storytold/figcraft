//! Variable width strokes render along the path, past the weight where the profile widens them.

use super::*;
use figcraft_color::{Color, Paint};
use figcraft_doc::{NodeKind, WidthPreset, WidthProfile};

fn dark(p: &Pixmap, x: u32, y: u32) -> bool {
    let i = ((y * p.width() + x) * 4) as usize;
    p.data().get(i).is_some_and(|r| *r < 40)
}

/// A horizontal line from (10, 50) to (90, 50) with a 20 wide black stroke.
fn line_doc(profile: Option<WidthProfile>) -> (figcraft_doc::Document, figcraft_doc::NodeId) {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let l = d.create(p, NodeKind::Line, "L", 10.0, 50.0, 80.0, 0.0).unwrap();
    let n = d.get_mut(l).unwrap();
    n.strokes = vec![Paint::solid(Color::BLACK)];
    n.stroke.weight = 20.0;
    n.stroke.width_profile = profile;
    (d, p)
}

fn render(d: &figcraft_doc::Document, p: figcraft_doc::NodeId) -> Pixmap {
    render_view(d, p, &View { origin: (0.0, 0.0), scale: 1.0, width: 100, height: 100 }, &Options::default()).unwrap()
}

/// The stroke outline is cached: changing the dash cap alone has to redraw it.
#[test]
fn changing_the_dash_cap_redraws_the_dashes() {
    let (mut d, p) = line_doc(None);
    let line = d.node(p).unwrap().children[0];
    let n = d.get_mut(line).unwrap();
    (n.stroke.weight, n.stroke.dashes) = (10.0, vec![20.0, 20.0]);
    // Two 40-long patterns fit the 80-long line, so a whole dash ends at x = 60 on the page.
    assert!(!dark(&render(&d, p), 62, 50), "butt: a gap");
    d.get_mut(line).unwrap().stroke.dash_cap = Some(figcraft_doc::StrokeCap::Square);
    assert!(dark(&render(&d, p), 62, 50), "square: 5 further");
    d.get_mut(line).unwrap().stroke.dash_cap = Some(figcraft_doc::StrokeCap::None);
    assert!(!dark(&render(&d, p), 62, 50));
}

#[test]
fn wedge_profile_tapers_the_stroke() {
    let (d, p) = line_doc(WidthPreset::Wedge.profile());
    let pix = render(&d, p);
    assert!(dark(&pix, 15, 58) && dark(&pix, 15, 42), "wide at the start");
    assert!(dark(&pix, 70, 50), "a quarter of the width three quarters along");
    assert!(!dark(&pix, 85, 54) && !dark(&pix, 85, 46), "thin at the end");
    // Uniform for comparison: full width at both ends.
    let (d, p) = line_doc(None);
    let pix = render(&d, p);
    assert!(dark(&pix, 85, 58));
}

#[test]
fn widened_profile_paints_past_the_weight() {
    // Three times the weight in the middle: 30 either side of the line.
    let (d, p) = line_doc(Some(WidthProfile::new([(0.0, 1.0), (0.5, 3.0), (1.0, 1.0)])));
    let pix = render(&d, p);
    assert!(dark(&pix, 50, 77) && dark(&pix, 50, 23), "not culled at the weight's reach");
    assert!(!dark(&pix, 15, 70));
}
