use super::*;
use crate::model::{Grad, GradKind};
use crate::paint::{GradOut, color_at, gradient, svd, turn_stops};
use figcraft_color::{ColorStop, Gradient, GradientKind};
use figcraft_geom::{Affine, Point};

fn two_stop(kind: GradientKind) -> Gradient {
    let mut g = Gradient::default_for(kind, hex("FF0000"));
    g.stops = vec![
        ColorStop { position: 0.0, color: hex("FF0000") },
        ColorStop { position: 0.6, color: hex("00FF00") },
        ColorStop { position: 1.0, color: hex("0000FF") },
    ];
    g
}

/// Skewed, mirrored handles: nothing a plain gradient element could hold.
fn skewed(kind: GradientKind) -> Gradient {
    let mut g = two_stop(kind);
    g.from = (0.35, 0.4);
    g.to = (0.8, 0.55);
    g.width = (0.25, 0.95);
    g
}

/// Where Android samples a Vector Drawable gradient at viewport point `p` (sweep: from +x,
/// turning toward +y, a full turn 0 → 1).
fn vd_t(out: &GradOut, p: Point) -> Option<(f64, &Grad)> {
    let (q, grad) = match out {
        GradOut::Plain(g) => (p, g),
        GradOut::Grouped { inverse, grad, .. } => (*inverse * p, grad),
        _ => return None,
    };
    let t = match grad.kind {
        GradKind::Linear { start, end } => {
            let (dx, dy) = (end.0 - start.0, end.1 - start.1);
            ((q.x - start.0) * dx + (q.y - start.1) * dy) / (dx * dx + dy * dy)
        }
        GradKind::Radial { center, radius } => (q.x - center.0).hypot(q.y - center.1) / radius,
        GradKind::Sweep { center } => (q.y - center.1).atan2(q.x - center.0).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU,
    };
    Some((t, grad))
}

/// FigCraft's own parameter at `p` for a gradient with frame `f` (viewport ← unit).
fn figcraft_t(kind: GradientKind, f: Affine, p: Point) -> f64 {
    let u = f.inverse() * p;
    match kind {
        GradientKind::Linear => u.x,
        GradientKind::Radial => u.x.hypot(u.y),
        _ => u.y.atan2(u.x).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU,
    }
}

fn layer_xfs() -> Vec<Affine> {
    vec![
        Affine::IDENTITY,
        Affine::translate((40.0, 10.0)) * Affine::rotate(0.7),
        Affine::translate((90.0, 0.0)) * Affine::scale_non_uniform(-1.0, 1.0),
        Affine::new([1.2, 0.3, -0.5, 0.8, 30.0, 20.0]),
    ]
}

#[test]
fn every_gradient_samples_where_figcraft_samples_it() {
    let (w, h) = (100.0, 80.0);
    for kind in [GradientKind::Linear, GradientKind::Radial, GradientKind::Angular] {
        for g in [two_stop(kind), skewed(kind)] {
            for xf in layer_xfs() {
                let out = gradient(&g, 1.0, xf, w, h, false);
                let f = xf * crate::paint::frame(&g, w, h);
                for i in 0..40 {
                    for j in 0..40 {
                        let p = xf * Point::new(w * (i as f64 + 0.5) / 40.0, h * (j as f64 + 0.5) / 40.0);
                        let want = figcraft_t(kind, f, p);
                        let (got, grad) = vd_t(&out, p).unwrap();
                        let stops: Vec<_> = g.sorted_stops().iter().map(|s| (f64::from(s.position), s.color)).collect();
                        let (a, b) = (color_at(&stops, want), color_at(&grad.stops, got));
                        // Away from the sweep's seam a turn's worth either side is the same colour.
                        let near_seam = kind == GradientKind::Angular && (want < 1e-3 || want > 1.0 - 1e-3);
                        if !near_seam {
                            assert!(
                                (a.r - b.r).abs() < 1e-3 && (a.g - b.g).abs() < 1e-3 && (a.b - b.b).abs() < 1e-3,
                                "{kind:?} {xf:?} at {p:?}: {want} vs {got}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn the_written_gradient_matches_the_rendered_pixels() {
    for kind in [GradientKind::Linear, GradientKind::Radial, GradientKind::Angular] {
        let (mut d, p) = doc();
        let r = rect(&mut d, p, "R", 0.0, 0.0, 100.0, 80.0, "000000");
        d.get_mut(r).unwrap().fills = vec![Paint::gradient(skewed(kind))];
        let area = figcraft_render::export_bounds(&d, r).unwrap();
        let pix = figcraft_render::render_area(&d, &[r], area, 1.0).unwrap();
        let rgba = figcraft_render::to_rgba8(&pix);
        let out = gradient(&skewed(kind), 1.0, Affine::translate((-area.x0, -area.y0)) * d.world_transform(r), 100.0, 80.0, false);
        let f = Affine::translate((-area.x0, -area.y0)) * d.world_transform(r) * crate::paint::frame(&skewed(kind), 100.0, 80.0);
        let mut checked = 0;
        for y in (2..78).step_by(3) {
            for x in (2..98).step_by(3) {
                let pt = Point::new(x as f64 + 0.5, y as f64 + 0.5);
                let ft = figcraft_t(kind, f, pt);
                if kind == GradientKind::Angular && (ft < 0.03 || ft > 0.97 || (ft - 0.6).abs() < 0.01) {
                    continue;
                }
                let (t, grad) = vd_t(&out, pt).unwrap();
                let c = color_at(&grad.stops, t);
                let i = (y * pix.width() as usize + x) * 4;
                let px = &rgba[i..i + 3];
                let want = [c.r, c.g, c.b].map(|v| (v * 255.0).round());
                for k in 0..3 {
                    assert!((f32::from(px[k]) - want[k]).abs() <= 6.0, "{kind:?} at ({x},{y}): rendered {px:?}, VD {want:?}");
                }
                checked += 1;
            }
        }
        assert!(checked > 500, "{kind:?}: {checked}");
    }
}

#[test]
fn svd_puts_the_matrix_back_together() {
    let ms = [[1.0, 0.0, 0.0, 1.0], [2.0, 0.5, -0.3, 0.7], [0.0, 3.0, -2.0, 0.0], [-1.0, 0.2, 0.4, 1.5], [0.3, -0.9, -1.1, -0.2]];
    for [a, b, c, d] in ms {
        let (phi, sx, sy, theta) = svd(a, b, c, d);
        assert!(sx >= sy.abs() - 1e-12, "{sx} {sy}");
        assert_eq!(sy < 0.0, a * d - b * c < 0.0, "a mirror shows in sy");
        let m = Affine::rotate(phi) * Affine::scale_non_uniform(sx, sy) * Affine::rotate(theta);
        for (x, y) in m.as_coeffs().iter().zip([a, b, c, d]) {
            assert!(close(*x, y), "{:?} vs {:?}", m.as_coeffs(), [a, b, c, d]);
        }
    }
}

#[test]
fn turned_stops_keep_every_colour_where_it_was() {
    let stops = vec![(0.0, hex("FF0000")), (0.25, hex("00FF00")), (0.7, hex("0000FF")), (1.0, hex("FFFF00"))];
    for shift in [0.1, 0.5, 0.95, -0.3, 1.25] {
        let turned = turn_stops(&stops, shift);
        assert!(turned.windows(2).all(|w| w[0].0 <= w[1].0), "sorted: {turned:?}");
        assert_eq!((turned[0].0, turned[turned.len() - 1].0), (0.0, 1.0));
        for i in 1..200 {
            let t = i as f64 / 200.0;
            let moved = (t + shift).rem_euclid(1.0);
            if moved < 1e-6 || moved > 1.0 - 1e-6 {
                continue;
            }
            let (a, b) = (color_at(&stops, t), color_at(&turned, moved));
            assert!((a.r - b.r).abs() < 1e-4 && (a.g - b.g).abs() < 1e-4 && (a.b - b.b).abs() < 1e-4, "{shift}: {t}");
        }
    }
    assert_eq!(turn_stops(&stops, 0.0), stops);
    assert_eq!(turn_stops(&stops, 2.0), stops);
}

#[test]
fn a_collapsed_frame_paints_nothing_and_one_stop_is_solid() {
    let mut g = two_stop(GradientKind::Radial);
    g.width = g.from;
    assert_eq!(gradient(&g, 1.0, Affine::IDENTITY, 10.0, 10.0, false), GradOut::Nothing);
    let mut one = two_stop(GradientKind::Linear);
    one.stops.truncate(1);
    assert_eq!(gradient(&one, 0.5, Affine::IDENTITY, 10.0, 10.0, false), GradOut::Solid(hex("FF000080").with_alpha(0.5)));
}

#[test]
fn a_linear_gradient_is_an_aapt_attr_with_paint_opacity_in_its_stops() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "R", 0.0, 0.0, 100.0, 50.0, "000000");
    let mut paint = Paint::gradient(two_stop(GradientKind::Linear));
    paint.opacity = 0.5;
    d.get_mut(r).unwrap().fills = vec![paint];
    let out = export(&d, &[r], Compat::Exact);
    assert!(out.xml.contains("xmlns:aapt=\"http://schemas.android.com/aapt\""), "{}", out.xml);
    let els = elements(&out.xml);
    let attr = tagged(&els, "attr");
    assert_eq!(attr[0].get("name"), Some("android:fillColor"));
    let g = tagged(&els, "gradient")[0];
    assert_eq!(g.get("type"), Some("linear"));
    assert!(g.get("tileMode").is_none(), "clamp is the default");
    // Default handles: top centre to bottom centre.
    assert_eq!([g.get("startX"), g.get("startY"), g.get("endX"), g.get("endY")], [Some("50"), Some("0"), Some("50"), Some("50")]);
    let items: Vec<_> = tagged(&els, "item").iter().map(|e| (e.get("offset").unwrap().to_string(), e.get("color").unwrap().to_string())).collect();
    assert_eq!(items, [("0".into(), "#80FF0000".into()), ("0.6".into(), "#8000FF00".into()), ("1".into(), "#800000FF".into())]);
    assert!(tagged(&els, "path")[0].get("fillColor").is_none());
}

#[test]
fn a_radial_gradient_sits_in_a_group_that_maps_the_unit_circle() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "R", 0.0, 0.0, 100.0, 50.0, "000000");
    d.get_mut(r).unwrap().fills = vec![Paint::gradient(two_stop(GradientKind::Radial))];
    let out = export(&d, &[r], Compat::Exact);
    let els = elements(&out.xml);
    let gr = tagged(&els, "group")[0];
    // Centre (50, 25), radii 50 × 25 (an axis-aligned frame: no rotation).
    assert_eq!([gr.get("translateX"), gr.get("translateY")], [Some("50"), Some("25")]);
    let (sx, sy) = (gr.get("scaleX").unwrap().parse::<f64>().unwrap(), gr.get("scaleY").unwrap().parse::<f64>().unwrap());
    assert!(close(sx.abs(), 50.0) && close(sy.abs(), 25.0), "{sx} {sy}");
    let g = tagged(&els, "gradient")[0];
    assert_eq!([g.get("type"), g.get("centerX"), g.get("centerY"), g.get("gradientRadius")], [Some("radial"), Some("0"), Some("0"), Some("1")]);
    // The path is in the group's space: mapped back it covers the rectangle.
    let (x0, y0, x1, y1) = bounds(tagged(&els, "path")[0].get("pathData").unwrap());
    let rot = gr.get("rotation").map_or(0.0, |v| v.parse::<f64>().unwrap()).to_radians();
    let m = Affine::translate((50.0, 25.0)) * Affine::rotate(rot) * Affine::scale_non_uniform(sx, sy);
    let b = m.transform_rect_bbox(figcraft_geom::Rect::new(x0, y0, x1, y1));
    assert!(close(b.x0, 0.0) && close(b.y0, 0.0) && (b.x1 - 100.0).abs() < 0.05 && (b.y1 - 50.0).abs() < 0.05, "{b:?}");
}

#[test]
fn diamond_gradients_per_mode() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "Gem", 0.0, 0.0, 40.0, 40.0, "000000");
    d.get_mut(r).unwrap().fills = vec![Paint::solid(hex("FFFFFF")), Paint::gradient(two_stop(GradientKind::Diamond))];
    let best = export(&d, &[r], Compat::BestEffort);
    assert!(best.xml.contains("android:type=\"radial\""), "{}", best.xml);
    assert_eq!(best.warnings, ["diamond gradients were drawn as radial gradients (Gem)"]);
    let exact = export(&d, &[r], Compat::Exact);
    assert!(!exact.xml.contains("gradient") && exact.xml.contains("#FFFFFF"), "only the gradient goes:\n{}", exact.xml);
    assert!(!exact.xml.contains("xmlns:aapt"));
    assert_eq!(exact.warnings, ["diamond gradients were left out (exact mode) (Gem)"]);
    let ignore = export(&d, &[r], Compat::Ignore);
    assert!(!ignore.xml.contains("gradient") && ignore.warnings.is_empty(), "{:?}", ignore.warnings);
}

#[test]
fn a_sweep_is_written_as_a_sweep_gradient_with_its_seam() {
    // Default angular handles start at 3 o'clock; red, yellow at a quarter, blue at the end.
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "Wheel", 0.0, 0.0, 100.0, 100.0, "000000");
    let mut g = two_stop(GradientKind::Angular);
    g.stops = vec![
        ColorStop { position: 0.0, color: hex("FF0000") },
        ColorStop { position: 0.25, color: hex("FFFF00") },
        ColorStop { position: 1.0, color: hex("0000FF") },
    ];
    d.get_mut(r).unwrap().fills = vec![figcraft_color::Paint::gradient(g)];
    let out = export(&d, &[r], Compat::BestEffort);
    let els = elements(&out.xml);
    let grads = tagged(&els, "gradient");
    assert_eq!(grads.len(), 1, "{}", out.xml);
    assert_eq!(grads[0].get("type"), Some("sweep"));
    assert_eq!((grads[0].get("centerX"), grads[0].get("centerY")), (Some("0"), Some("0")));
    // In a group that moves the unit gradient onto the layer.
    assert!(tagged(&els, "group").iter().any(|e| e.get("translateX").is_some()), "{}", out.xml);
    let items: Vec<(f64, String)> =
        tagged(&els, "item").iter().map(|e| (e.get("offset").unwrap().parse().unwrap(), e.get("color").unwrap().to_string())).collect();
    assert_eq!(items.first().map(|i| i.0), Some(0.0), "{items:?}");
    assert_eq!(items.last().map(|i| i.0), Some(1.0), "{items:?}");
    assert!(items.windows(2).all(|w| w[0].0 <= w[1].0), "offsets go up: {items:?}");
    assert!(items.windows(2).all(|w| w[0] != w[1]), "no repeated stops: {items:?}");
    // The seam: blue (the end) then red (the start) at the same offset.
    assert!(items.windows(2).any(|w| w[0].0 == w[1].0 && w[0].1 == "#0000FF" && w[1].1 == "#FF0000"), "{items:?}");
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn turned_stops_on_the_seam_are_not_repeated() {
    let (r, b) = (hex("FF0000"), hex("0000FF"));
    let out = turn_stops(&[(0.0, r), (0.5, b), (1.0, r)], 0.5);
    assert!(out.windows(2).all(|w| w[0] != w[1]), "{out:?}");
}
