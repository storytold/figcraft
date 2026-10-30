use super::*;
use figcraft_color::{Color, ColorStop, Gradient, GradientKind, Paint};
use figcraft_doc::{Effect, EffectKind, NodeKind, Region, Segment, Shadow, StrokeAlign, TextData, VectorNetwork, Vertex, WindingRule};

fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = &p.data()[i..i + 4];
    [d[0], d[1], d[2], d[3]]
}

fn doc_with_rect(fill: Color) -> (figcraft_doc::Document, figcraft_doc::NodeId, figcraft_doc::NodeId) {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let r = d.create(p, NodeKind::Rectangle, "R", 10.0, 10.0, 50.0, 30.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(fill)];
    (d, p, r)
}

fn view(w: u32, h: u32) -> View {
    View { origin: (0.0, 0.0), scale: 1.0, width: w, height: h }
}

#[test]
fn solid_rect_and_page_background() {
    let (d, p, _) = doc_with_rect(Color::from_hex("FF0000").unwrap());
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 30, 20), [255, 0, 0, 255]);
    assert_eq!(px(&pix, 5, 5), [0xF5, 0xF5, 0xF5, 255]);
}

#[test]
fn vector_regions_use_their_own_paints_and_empty_regions_do_not_erase() {
    let mut d = figcraft_doc::Document::new("regions");
    let page = d.first_page().unwrap();
    let network = VectorNetwork {
        vertices: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (10.0, 0.0), (20.0, 0.0), (20.0, 10.0), (10.0, 10.0)]
            .into_iter()
            .map(|(x, y)| Vertex::new(x, y))
            .collect(),
        segments: (0..8).map(|i| Segment::line(i, (i + 1) % 4 + (i / 4) * 4)).collect(),
        regions: vec![
            Region {
                loops: vec![vec![0, 1, 2, 3]],
                winding: WindingRule::NonZero,
                fills: Some(vec![Paint::solid(Color::from_hex("FF0000").unwrap())]),
                ..Default::default()
            },
            Region {
                loops: vec![vec![4, 5, 6, 7]],
                winding: WindingRule::NonZero,
                fills: Some(vec![Paint::solid(Color::from_hex("0000FF").unwrap())]),
                ..Default::default()
            },
            Region { loops: vec![vec![0, 1, 2, 3]], winding: WindingRule::NonZero, fills: Some(vec![]), ..Default::default() },
        ],
    };
    let id = d.create(page, NodeKind::Vector { network }, "V", 10.0, 10.0, 20.0, 10.0).unwrap();
    d.get_mut(id).unwrap().fills.clear();
    let pix = render_view(&d, page, &view(40, 30), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    assert_eq!(px(&pix, 15, 15), [255, 0, 0, 255]);
    assert_eq!(px(&pix, 25, 15), [0, 0, 255, 255]);
    assert_eq!(d.get(id).unwrap().fills.len(), 0);
}

#[test]
fn region_paint_space_preserves_gradient_image_and_pattern_placement() {
    use figcraft_color::{ImageScaleMode, PaintKind, PatternPaint};
    use figcraft_doc::RegionPaintSpace;
    use figcraft_geom::{Affine, Rect, Shape};

    let mut d = Document::new("paint frames");
    let page = d.first_page().unwrap();
    let tile = d.create(page, NodeKind::Rectangle, "Tile", 500.0, 500.0, 8.0, 6.0).unwrap();
    d.get_mut(tile).unwrap().fills = vec![Paint::solid(Color::from_hex("F05020").unwrap())];
    let mut image = Pixmap::new(8, 4).unwrap();
    for (i, pixel) in image.pixels_mut().iter_mut().enumerate() {
        *pixel = tiny_skia::PremultipliedColorU8::from_rgba((i % 8 * 30) as u8, (i / 8 * 60) as u8, 30, 255).unwrap();
    }
    d.images.insert(
        "region-frame-image".into(),
        figcraft_doc::ImageData { bytes: std::sync::Arc::new(encode_png(&image).unwrap()), width: 8, height: 4 },
    );
    let mut pattern = PatternPaint::new(tile.0);
    pattern.spacing = (3.0, 4.0);
    let paints = [
        Paint::gradient(Gradient::default_for(GradientKind::Linear, Color::BLACK)),
        Paint::image("region-frame-image", ImageScaleMode::Fill),
        Paint::image("region-frame-image", ImageScaleMode::Tile),
        Paint { kind: PaintKind::Pattern(pattern), ..Paint::solid(Color::WHITE) },
    ];
    let frame = Affine::translate((35.0, 25.0)) * Affine::rotate(0.3);
    let id = d.create(page, NodeKind::Rectangle, "Source", 0.0, 0.0, 60.0, 30.0).unwrap();
    for paint in paints {
        let n = d.get_mut(id).unwrap();
        n.kind = NodeKind::Rectangle;
        n.transform = frame;
        n.width = 60.0;
        n.height = 30.0;
        n.fills = vec![paint.clone()];
        let before = render_view(&d, page, &view(120, 100), &Options::default()).unwrap();
        let mut path = Rect::new(0.0, 0.0, 60.0, 30.0).to_path(0.1);
        path.apply_affine(frame);
        let mut network = VectorNetwork::from_path(&path);
        for region in &mut network.regions {
            region.fills = Some(vec![paint.clone()]);
            region.paint_space = Some(RegionPaintSpace { width: 60.0, height: 30.0, transform: frame });
        }
        let n = d.get_mut(id).unwrap();
        n.kind = NodeKind::Vector { network };
        n.transform = Affine::IDENTITY;
        n.width = 120.0;
        n.height = 100.0;
        n.fills.clear();
        let after = render_view(&d, page, &view(120, 100), &Options::default()).unwrap();
        for (a, b) in before.data().iter().zip(after.data()) {
            assert!(a.abs_diff(*b) <= 1, "paint moved after reboxing: {:?}", paint.kind);
        }
    }
}

#[test]
fn styled_vector_mask_unions_region_windings_and_excludes_unpainted_regions() {
    let mut d = figcraft_doc::Document::new("region mask");
    let page = d.first_page().unwrap();
    let group = d.create(page, NodeKind::Group, "Mask group", 0.0, 0.0, 60.0, 20.0).unwrap();
    let mut network = VectorNetwork::default();
    let mut regions = Vec::new();
    for x in [0.0, 10.0, 30.0, 40.0, 50.0] {
        let v = network.vertices.len();
        network.vertices.extend([(x, 0.0), (x + 20.0, 0.0), (x + 20.0, 20.0), (x, 20.0)].map(|(x, y)| Vertex::new(x, y)));
        let s = network.segments.len();
        network.segments.extend((0..4).map(|i| Segment::line(v + i, v + (i + 1) % 4)));
        let fills = match regions.len() {
            0 => Some(vec![Paint::solid(Color::from_hex("FF0000").unwrap())]),
            1 => Some(vec![Paint::solid(Color::from_hex("0000FF").unwrap())]),
            2 => Some(vec![Paint { visible: false, ..Paint::solid(Color::from_hex("00FF00").unwrap()) }]),
            3 => None,
            _ => Some(Vec::new()),
        };
        regions.push(Region {
            loops: vec![(s..s + 4).collect()],
            winding: if regions.len() == 1 { WindingRule::EvenOdd } else { WindingRule::NonZero },
            fills,
            ..Default::default()
        });
    }
    network.regions = regions;
    let mask = d.create(group, NodeKind::Vector { network }, "Mask", 0.0, 0.0, 60.0, 20.0).unwrap();
    {
        let n = d.get_mut(mask).unwrap();
        n.fills.clear();
        n.is_mask = true;
        n.mask_type = figcraft_doc::MaskType::Vector;
    }
    let content = d.create(group, NodeKind::Rectangle, "Content", 0.0, 0.0, 60.0, 20.0).unwrap();
    d.get_mut(content).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let pix = render_view(&d, page, &view(60, 20), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    assert_eq!(px(&pix, 5, 10), [0, 0, 0, 255], "visible left region masks");
    assert_eq!(px(&pix, 15, 10), [0, 0, 0, 255], "overlap remains in union despite mixed winding");
    assert_eq!(px(&pix, 35, 10), [255, 255, 255, 255], "hidden region excluded from styled silhouette");
    assert_eq!(px(&pix, 45, 10), [255, 255, 255, 255], "unpainted inherited region excluded");
    assert_eq!(px(&pix, 55, 10), [255, 255, 255, 255], "explicit-empty region excluded");
}

#[test]
fn inherited_paint_frames_keep_overlapping_regions_in_vector_masks() {
    use figcraft_geom::{Affine, Point, Shape};

    let mut d = Document::new("inherited region mask");
    let page = d.first_page().unwrap();
    let group = d.create(page, NodeKind::Group, "Mask group", 0.0, 0.0, 40.0, 20.0).unwrap();
    let mut network = VectorNetwork::default();
    for x in [0.0, 10.0] {
        let v = network.vertices.len();
        let s = network.segments.len();
        network.vertices.extend([(x, 0.0), (x + 20.0, 0.0), (x + 20.0, 20.0), (x, 20.0)].map(|(x, y)| Vertex::new(x, y)));
        network.segments.extend((0..4).map(|i| Segment::line(v + i, v + (i + 1) % 4)));
        network.regions.push(Region {
            loops: vec![(s..s + 4).collect()],
            winding: WindingRule::EvenOdd,
            fills: None,
            paint_space: Some(figcraft_doc::RegionPaintSpace { width: 20.0, height: 20.0, transform: Affine::translate((x, 0.0)) }),
        });
    }
    let mask = d.create(group, NodeKind::Vector { network }, "Mask", 0.0, 0.0, 30.0, 20.0).unwrap();
    let n = d.get_mut(mask).unwrap();
    n.fills = vec![Paint::solid(Color::BLACK)];
    n.is_mask = true;
    n.mask_type = figcraft_doc::MaskType::Vector;
    let (outline, rule) = node_outline(&d, d.node(mask).unwrap()).unwrap();
    assert_eq!(rule, FillRule::Winding);
    assert_ne!(outline.winding(Point::new(15.0, 10.0)), 0, "independent inherited regions retain their overlap");
    let content = d.create(group, NodeKind::Rectangle, "Content", 0.0, 0.0, 40.0, 20.0).unwrap();
    d.get_mut(content).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let pix = render_view(&d, page, &view(40, 20), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    for x in [5, 15, 25] {
        assert_eq!(px(&pix, x, 10), [0, 0, 0, 255]);
    }
    assert_eq!(px(&pix, 35, 10), [255, 255, 255, 255]);
}

#[test]
fn zoom_scales() {
    let (d, p, _) = doc_with_rect(Color::BLACK);
    let v = View { origin: (0.0, 0.0), scale: 2.0, width: 200, height: 200 };
    let pix = render_view(&d, p, &v, &Options::default()).unwrap();
    assert_eq!(px(&pix, 100, 60)[3], 255);
    assert_eq!(px(&pix, 118, 78), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 125, 60)[0], 0xF5);
}

#[test]
fn opacity_and_layer() {
    let (mut d, p, r) = doc_with_rect(Color::BLACK);
    d.get_mut(r).unwrap().opacity = 0.5;
    let pix = render_view(&d, p, &view(100, 100), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    let v = px(&pix, 30, 20)[0];
    assert!((120..=135).contains(&v), "{v}");
}

#[test]
fn drop_shadow_paints_outside() {
    let (mut d, p, r) = doc_with_rect(Color::WHITE);
    d.get_mut(r).unwrap().effects.push(Effect {
        kind: EffectKind::DropShadow(Shadow { color: Color::BLACK, offset: (0.0, 10.0), radius: 0.0, ..Default::default() }),
        visible: true,
    });
    let pix = render_view(&d, p, &view(100, 100), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    // Below the rect (y 40..50) is shadow.
    assert!(px(&pix, 30, 45)[0] < 30, "{:?}", px(&pix, 30, 45));
    // The rect itself stays white.
    assert_eq!(px(&pix, 30, 20)[0], 255);
}

#[test]
fn inner_shadow_and_blur_dont_crash() {
    let (mut d, p, r) = doc_with_rect(Color::WHITE);
    let n = d.get_mut(r).unwrap();
    n.effects.push(Effect::inner_shadow());
    n.effects.push(Effect::layer_blur());
    n.effects.push(Effect::background_blur());
    n.effects.push(Effect {
        kind: EffectKind::Noise(figcraft_doc::Noise { size: 1.0, density: 0.5, color: Color::BLACK, ..Default::default() }),
        visible: true,
    });
    let _ = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
}

#[test]
fn gradients_render() {
    for kind in [GradientKind::Linear, GradientKind::Radial, GradientKind::Angular, GradientKind::Diamond] {
        let (mut d, p, r) = doc_with_rect(Color::BLACK);
        let g = Gradient {
            kind,
            stops: vec![
                ColorStop { position: 0.0, color: Color::from_hex("FF0000").unwrap() },
                ColorStop { position: 1.0, color: Color::from_hex("0000FF").unwrap() },
            ],
            from: (0.0, 0.5),
            to: (1.0, 0.5),
            width: (0.0, 1.0),
        };
        let g = if kind == GradientKind::Linear { g } else { Gradient { from: (0.5, 0.5), to: (1.0, 0.5), width: (0.5, 1.0), ..g } };
        d.get_mut(r).unwrap().fills = vec![Paint::gradient(g)];
        let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
        let left = px(&pix, if kind == GradientKind::Linear { 11 } else { 35 }, 25);
        let right = px(&pix, 58, 25);
        assert_ne!(left, right, "{kind:?}");
    }
}

#[test]
fn frame_clips_children() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 50.0, 50.0).unwrap();
    let r = d.create(f, NodeKind::Rectangle, "R", 25.0, 25.0, 50.0, 50.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 40, 40), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 60, 60)[0], 0xF5);
    d.get_mut(f).unwrap().clip = false;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 60, 60), [0, 0, 0, 255]);
}

#[test]
fn strokes_align() {
    let (mut d, p, r) = doc_with_rect(Color::WHITE);
    let n = d.get_mut(r).unwrap();
    n.strokes = vec![Paint::solid(Color::BLACK)];
    n.stroke.weight = 4.0;
    n.stroke.align = StrokeAlign::Outside;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 30, 8), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 30, 12), [255, 255, 255, 255]);
    d.get_mut(r).unwrap().stroke.align = StrokeAlign::Inside;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 30, 12), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 30, 8)[0], 0xF5);
}

#[test]
fn side_strokes_follow_rounded_corners() {
    // border-l-4 rounded-r-lg: a straight 4 px bar on the left, nothing on the other edges.
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let f = d.create(p, NodeKind::Frame, "F", 10.0, 10.0, 120.0, 60.0).unwrap();
    let n = d.get_mut(f).unwrap();
    n.fills = vec![Paint::solid(Color::WHITE)];
    n.corners.radii = [0.0, 8.0, 8.0, 0.0];
    n.strokes = vec![Paint::solid(Color::BLACK)];
    n.stroke.align = StrokeAlign::Inside;
    n.stroke.sides = Some([0.0, 0.0, 0.0, 4.0]);
    let pix = render_view(&d, p, &view(150, 80), &Options::default()).unwrap();
    for x in 10..14 {
        assert_eq!(px(&pix, x, 40), [0, 0, 0, 255], "x {x}");
    }
    for (x, y) in [(14, 40), (129, 40), (126, 12), (126, 67), (70, 10), (70, 69)] {
        assert_eq!(px(&pix, x, y), [255, 255, 255, 255], "({x}, {y})");
    }
}

/// #1351: a stroke on one side of a rectangle or frame with rounded corners stays on that side, for
/// every alignment, with no stroke on the other edges.
#[test]
fn a_rounded_shape_strokes_only_the_side_asked_for() {
    // On each edge, top to left: a point of an outside stroke and one of an inside stroke, which a
    // centred stroke covers too.
    let edges = [[(70, 18), (70, 21)], [(131, 50), (128, 50)], [(70, 81), (70, 78)], [(8, 50), (11, 50)]];
    for kind in [NodeKind::Rectangle, NodeKind::Frame] {
        for align in [StrokeAlign::Inside, StrokeAlign::Center, StrokeAlign::Outside] {
            for side in 0..4 {
                let mut d = figcraft_doc::Document::new("t");
                let p = d.first_page().unwrap();
                let f = d.create(p, kind.clone(), "F", 10.0, 20.0, 120.0, 60.0).unwrap();
                let n = d.get_mut(f).unwrap();
                n.corners.radii = [16.0; 4];
                n.fills = vec![Paint::solid(Color::WHITE)];
                n.strokes = vec![Paint::solid(Color::BLACK)];
                n.stroke.align = align;
                n.stroke.sides = Some(std::array::from_fn(|i| if i == side { 4.0 } else { 0.0 }));
                let pix = render_view(&d, p, &view(150, 100), &Options::default()).unwrap();
                let black = |(x, y): (u32, u32)| px(&pix, x, y) == [0, 0, 0, 255];
                let at = edges[side][usize::from(align != StrokeAlign::Outside)];
                assert!(black(at), "{kind:?} {align:?} side {side} has its stroke at {at:?}");
                for (i, edge) in edges.iter().enumerate().filter(|(i, _)| *i != side) {
                    assert!(!edge.iter().any(|at| black(*at)), "{kind:?} {align:?} side {side} leaves edge {i} alone");
                }
            }
        }
    }
}

#[test]
fn mask_clips_siblings() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
    let m = d.create(g, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "M", 0.0, 0.0, 50.0, 50.0).unwrap();
    d.get_mut(m).unwrap().is_mask = true;
    let r = d.create(g, NodeKind::Rectangle, "R", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 25, 25), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 2, 2)[0], 0xF5);
    assert_eq!(px(&pix, 80, 80)[0], 0xF5);
}

#[test]
fn luminance_and_vector_masks() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
    // The mask: white on the left half, black on the right.
    let m = d.create(g, NodeKind::Rectangle, "M", 0.0, 0.0, 100.0, 100.0).unwrap();
    let grad = figcraft_color::Gradient {
        kind: figcraft_color::GradientKind::Linear,
        stops: vec![
            figcraft_color::ColorStop { position: 0.0, color: Color::WHITE },
            figcraft_color::ColorStop { position: 0.49, color: Color::WHITE },
            figcraft_color::ColorStop { position: 0.51, color: Color::BLACK },
            figcraft_color::ColorStop { position: 1.0, color: Color::BLACK },
        ],
        from: (0.0, 0.5),
        to: (1.0, 0.5),
        width: (0.0, 1.0),
    };
    d.get_mut(m).unwrap().fills = vec![Paint::gradient(grad)];
    d.get_mut(m).unwrap().is_mask = true;
    let r = d.create(g, NodeKind::Rectangle, "R", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(r).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    // Alpha: the mask is opaque everywhere, so everything shows.
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 80, 50), [0, 0, 0, 255]);
    // Luminance: only the white half shows.
    d.get_mut(m).unwrap().mask_type = figcraft_doc::MaskType::Luminance;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 20, 50), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 80, 50)[0], 0xF5, "black hides");
    // Vector: a fully transparent ellipse still masks by its outline.
    let e = d.create(g, NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "E", 0.0, 0.0, 50.0, 50.0).unwrap();
    d.get_mut(e).unwrap().fills = vec![Paint::solid(Color::new(0.0, 0.0, 0.0, 0.0))];
    d.get_mut(e).unwrap().is_mask = true;
    d.get_mut(e).unwrap().mask_type = figcraft_doc::MaskType::Vector;
    d.get_mut(g).unwrap().children = vec![e, r];
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 25, 25), [0, 0, 0, 255], "inside the outline");
    assert_eq!(px(&pix, 80, 80)[0], 0xF5, "outside");
    // As an alpha mask the transparent ellipse hides everything.
    d.get_mut(e).unwrap().mask_type = figcraft_doc::MaskType::Alpha;
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 25, 25)[0], 0xF5);
}

#[test]
fn boolean_subtract_renders_hole() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let b = d.create(p, NodeKind::BooleanOp { op: figcraft_doc::BoolOp::Subtract }, "B", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(b).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    d.create(b, NodeKind::Rectangle, "A", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.create(b, NodeKind::Rectangle, "Hole", 25.0, 25.0, 50.0, 50.0).unwrap();
    let pix = render_view(&d, p, &view(100, 100), &Options::default()).unwrap();
    assert_eq!(px(&pix, 10, 10), [0, 0, 0, 255]);
    assert_eq!(px(&pix, 50, 50)[0], 0xF5);
}

#[test]
fn text_renders_something() {
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let mut td = TextData::new("Hello");
    td.style.font_size = 32.0;
    d.create(p, NodeKind::Text(td), "T", 0.0, 0.0, 200.0, 50.0).unwrap();
    let pix = render_view(&d, p, &view(200, 60), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    let dark = pix.data().as_chunks::<4>().0.iter().filter(|c| c[0] < 100).count();
    if figcraft_text::find_face("Inter", "Regular").is_some() {
        assert!(dark > 50, "{dark}");
    }
}

/// The machine's colour emoji face, if it has one (this test skips without: no font ships here).
fn emoji_face() -> Option<(String, String)> {
    figcraft_text::scan_system_fonts();
    let mut db = figcraft_text::db().write().unwrap();
    db.emoji_fallbacks().into_iter().find(|f| f.has_color()).map(|f| (f.family.clone(), f.style.clone()))
}

#[test]
fn colour_emoji_draw_their_pixels() {
    let Some((family, style)) = emoji_face() else { return };
    let mut d = figcraft_doc::Document::new("t");
    let p = d.first_page().unwrap();
    let mut td = TextData::new("\u{1F600}");
    (td.style.font_family, td.style.font_style) = (family.clone(), style.clone());
    td.style.font_size = 48.0;
    d.create(p, NodeKind::Text(td), "T", 0.0, 0.0, 200.0, 64.0).unwrap();
    let pix = render_view(&d, p, &view(100, 80), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    // The emoji is the only thing in this document that paints saturated colour.
    let coloured = pix
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|c| {
            let (r, g, b) = (i16::from(c[0]), i16::from(c[1]), i16::from(c[2]));
            c[3] == 255 && (r.max(g).max(b) - r.min(g).min(b)) > 60
        })
        .count();
    assert!(coloured > 200, "{family} drew {coloured} coloured pixels");
}

#[test]
fn export_png_roundtrip() {
    let (d, _, r) = doc_with_rect(Color::from_hex("00FF00").unwrap());
    let pix = render_node(&d, r, 2.0).unwrap();
    assert_eq!((pix.width(), pix.height()), (100, 60));
    let png = encode_png(&pix).unwrap();
    let back = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(back.get_pixel(50, 30).0, [0, 255, 0, 255]);
    let _ = encode_jpeg(&pix, Color::WHITE, 90).unwrap();
}

#[test]
fn exports_carry_the_colour_profile() {
    use figcraft_color::ColorProfile;
    use figcraft_doc::ExportFormat;
    use image::ImageDecoder;
    let (d, _, r) = doc_with_rect(Color::from_hex("00FF00").unwrap());
    let pix = render_node(&d, r, 1.0).unwrap();
    for profile in ColorProfile::ALL {
        let png = encode_export(&pix, ExportFormat::Png, profile).unwrap();
        // The inserted chunk has a valid CRC: the decoder accepts the file and keeps the pixels.
        let mut dec = image::codecs::png::PngDecoder::new(std::io::Cursor::new(&png)).unwrap();
        let icc = dec.icc_profile().unwrap();
        assert_eq!(icc.is_some(), !profile.is_srgb(), "{profile:?}");
        assert_eq!(image::load_from_memory(&png).unwrap().to_rgba8().get_pixel(5, 5).0, [0, 255, 0, 255]);
        for f in [ExportFormat::Jpg, ExportFormat::Webp] {
            assert!(!encode_export(&pix, f, profile).unwrap().is_empty());
        }
        assert!(encode_export(&pix, ExportFormat::Svg, profile).is_err());
    }
    assert_eq!(png_insert_chunk(b"not a png".to_vec(), b"sRGB", &[0]), b"not a png");
}

#[test]
fn images_are_managed_into_the_file_profile() {
    use figcraft_color::{ColorProfile, ImageScaleMode, Paint};
    let (mut d, _, r) = doc_with_rect(Color::WHITE);
    let mut red = Pixmap::new(4, 4).unwrap();
    red.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    d.images.insert("red".into(), figcraft_doc::ImageData { bytes: std::sync::Arc::new(encode_png(&red).unwrap()), width: 4, height: 4 });
    d.get_mut(r).unwrap().fills = vec![Paint::image("red", ImageScaleMode::Fill)];
    let center = |d: &Document| {
        let pix = render_node(d, r, 1.0).unwrap();
        let (x, y) = (pix.width() / 2, pix.height() / 2);
        let px = pix.pixel(x, y).unwrap();
        (pix, [px.red(), px.green(), px.blue()])
    };
    assert_eq!(center(&d).1, [255, 0, 0]);
    // In a Display P3 file the (sRGB) image is converted into P3 values…
    d.color_profile = ColorProfile::DisplayP3;
    let (mut pix, rgb) = center(&d);
    assert!(rgb[0].abs_diff(234) <= 1 && rgb[1].abs_diff(51) <= 1 && rgb[2].abs_diff(35) <= 1, "{rgb:?}");
    // …and comes back to the screen unchanged.
    to_srgb(&mut pix, d.color_profile);
    let px = pix.pixel(pix.width() / 2, pix.height() / 2).unwrap();
    assert!(px.red() >= 254 && px.green() <= 1 && px.blue() <= 1, "{px:?}");
}

#[test]
fn hostile_sizes() {
    let (mut d, p, r) = doc_with_rect(Color::BLACK);
    let n = d.get_mut(r).unwrap();
    n.width = f64::NAN;
    n.height = 1e12;
    n.effects.push(Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(1e9)), visible: true });
    let _ = render_view(&d, p, &view(64, 64), &Options::default());
    let _ = render_node(&d, r, f64::NAN);
    let _ = render_view(&d, p, &View { origin: (f64::NAN, 0.0), scale: 0.0, width: 0, height: 0 }, &Options::default());
}

#[test]
fn inner_shadow_reaches_every_edge() {
    let (mut d, p, r) = doc_with_rect(Color::WHITE);
    d.get_mut(r).unwrap().effects.push(Effect {
        kind: EffectKind::InnerShadow(Shadow { color: Color::BLACK, offset: (0.0, 4.0), radius: 8.0, ..Default::default() }),
        visible: true,
    });
    let pix = render_view(&d, p, &view(100, 100), &Options { background: Some(Color::WHITE), ..Default::default() }).unwrap();
    // Left edge (no offset that way) is still shaded; the centre is not.
    assert!(px(&pix, 11, 25)[0] < 235, "{:?}", px(&pix, 11, 25));
    assert!(px(&pix, 35, 30)[0] > 245);
}

#[test]
fn rect_clip_frame_matches_same_rect_fill_on_fractional_edges() {
    let white = Options { background: Some(Color::WHITE), ..Default::default() };
    for (x, y, w, h) in [(0.5, 0.5, 10.0, 10.0), (0.0, 0.0, 10.5, 10.5), (0.0, 0.0, 10.0, 10.0)] {
        let mut clipped = figcraft_doc::Document::new("clipped");
        let p = clipped.first_page().unwrap();
        let f = clipped.create(p, NodeKind::Frame, "F", x, y, w, h).unwrap();
        clipped.get_mut(f).unwrap().fills.clear();
        clipped.get_mut(f).unwrap().clip = true;
        let c = clipped.create(f, NodeKind::Rectangle, "C", -5.0, -5.0, w + 10.0, h + 10.0).unwrap();
        clipped.get_mut(c).unwrap().fills = vec![Paint::solid(Color::BLACK)];

        let mut direct = figcraft_doc::Document::new("direct");
        let dp = direct.first_page().unwrap();
        let r = direct.create(dp, NodeKind::Rectangle, "R", x, y, w, h).unwrap();
        direct.get_mut(r).unwrap().fills = vec![Paint::solid(Color::BLACK)];

        let a = render_view(&clipped, p, &view(12, 12), &white).unwrap();
        let e = render_view(&direct, dp, &view(12, 12), &white).unwrap();
        for (px_x, px_y) in [(0, 0), (0, 5), (5, 0), (5, 5), (10, 5), (5, 10), (10, 10), (11, 11)] {
            assert_eq!(px(&a, px_x, px_y), px(&e, px_x, px_y), "frame {x},{y} {w}x{h} at ({px_x},{px_y})");
        }
    }
}
