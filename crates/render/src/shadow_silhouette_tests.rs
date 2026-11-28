//! A sharp drop shadow under a stroked layer is one solid silhouette: where the layer's edge lies
//! over its own shadow, nothing lighter than the shadow shows between them (#2631).

use figcraft_color::Paint;
use figcraft_doc::{Document, Effect, EffectKind, NodeKind, Shadow, StrokeAlign, VectorNetwork};
use figcraft_geom::Point;

use super::paint_behaviour_tests::{hex, page, px, render};

const OFFSET: (i64, i64) = (5, 7);

/// The page with `kind` at a fractional position, filled pink with a black 3-unit `align` stroke
/// and, with `shadow`, a black unblurred drop shadow at [`OFFSET`]; `None` without a font for text.
fn scene(kind: &NodeKind, align: StrokeAlign, shadow: bool) -> Option<Document> {
    let (mut d, p) = page();
    let id = d.create(p, kind.clone(), "L", 40.3, 30.6, 60.0, 60.0).ok()?;
    let n = d.get_mut(id).ok()?;
    n.fills = vec![Paint::solid(hex("#EC4899"))];
    n.strokes = vec![Paint::solid(hex("#000000"))];
    (n.stroke.weight, n.stroke.align) = (3.0, align);
    if shadow {
        let offset = (OFFSET.0 as f64, OFFSET.1 as f64);
        n.effects =
            vec![Effect { kind: EffectKind::DropShadow(Shadow { color: hex("#000000"), offset, radius: 0.0, ..Shadow::default() }), visible: true }];
    }
    if matches!(kind, NodeKind::Text(_)) {
        (n.width, n.height) = figcraft_text::auto_size(n)?;
    }
    Some(d)
}

#[test]
fn a_sharp_shadow_under_a_stroked_layer_has_no_light_line_along_the_layers_edge() {
    let mut letter = figcraft_doc::TextData::new("B");
    letter.style.font_size = 80.0;
    // A letter-like outline with a notch (cast from the layer's pixels), an ellipse (cast from
    // its outline) and, where a font loads, a letter.
    let notch = [(0.0, 0.0), (60.0, 0.0), (60.0, 60.0), (40.0, 60.0), (30.0, 20.0), (20.0, 60.0), (0.0, 60.0)].map(|(x, y)| Point::new(x, y));
    let kinds = [
        ("notch", NodeKind::Vector { network: VectorNetwork::from_points(&notch, true) }),
        ("ellipse", NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }),
        ("letter", NodeKind::Text(letter)),
    ];
    let shadow = hex("#000000").to_rgba8();
    for (name, kind) in &kinds {
        for align in [StrokeAlign::Inside, StrokeAlign::Center, StrokeAlign::Outside] {
            let (Some(plain), Some(shadowed)) = (scene(kind, align, false), scene(kind, align, true)) else { continue };
            let (plain, shadowed) = (render(&plain), render(&shadowed));
            // The layer's paints cover a pixel fully when every neighbour is pink or black.
            let solid = |x: i64, y: i64| {
                (-1..=1).all(|i| {
                    (-1..=1).all(|j| {
                        u32::try_from(x + i)
                            .ok()
                            .zip(u32::try_from(y + j).ok())
                            .is_some_and(|(x, y)| x < 200 && y < 200 && px(&plain, x, y)[1] < 0x48 + 8)
                    })
                })
            };
            let mut checked = 0;
            for (x, y) in (0..200).flat_map(|y| (0..200).map(move |x| (x, y))) {
                // Inside the shadow, away from its edge, wherever the layer doesn't hide it whole.
                if solid(x - OFFSET.0, y - OFFSET.1) && !solid(x, y) {
                    checked += 1;
                    let c = px(&shadowed, x as u32, y as u32);
                    assert!(c.iter().zip(shadow).all(|(v, s)| *v <= s + 2), "{name} {align:?}: ({x}, {y}) is {c:?}, lighter than the shadow");
                }
            }
            assert!(checked > 100, "{name} {align:?}: the shadow shows past the layer");
        }
    }
}
