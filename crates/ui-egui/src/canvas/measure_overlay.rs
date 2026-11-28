//! Smart guides and measurements on the canvas: red lines (dashed extension lines), a marker on a
//! point snapped onto, the outline of the layer measured to, and distance labels placed so they
//! never overlap one another and stay the same size at every zoom.

use egui::{Color32, CornerRadius, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use crate::FigApp;
use crate::theme::{Tokens, semibold};

/// Space between a label and the line it labels (or another label), in screen pixels.
const GAP: f32 = 4.0;
/// Padding inside a label's pill.
const PAD: Vec2 = vec2(4.0, 2.0);

/// Draw the tool overlay's guides, measurement labels and measured box, and in Dev Mode the
/// measurements kept on the canvas; labels keep clear of `avoid` (the selection's size label) and
/// of one another.
pub fn draw(app: &FigApp, painter: &egui::Painter, t: &Tokens, avoid: Option<Rect>) {
    let ov = &app.session.tool.overlay;
    let line = Stroke::new(1.0, t.guide);
    if let Some([x0, y0, x1, y1]) = ov.measure_box {
        painter.rect_stroke(Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1)), CornerRadius::ZERO, line, StrokeKind::Middle);
    }
    let mut labels: Vec<(Pos2, Pos2, String)> = Vec::new();
    for g in ov.guides.iter().chain(crate::dev_mode::saved_lines(app)) {
        let (a, b) = (app.to_screen(g.x0, g.y0), app.to_screen(g.x1, g.y1));
        if g.x0 == g.x1 && g.y0 == g.y1 {
            for d in [vec2(4.0, 4.0), vec2(4.0, -4.0)] {
                painter.line_segment([a - d, a + d], Stroke::new(1.5, t.guide));
            }
        } else if g.dashed {
            painter.extend(Shape::dashed_line(&[a, b], line, 3.0, 2.0));
        } else {
            painter.line_segment([a, b], line);
        }
        if let Some(l) = &g.label {
            labels.push((a, b, l.clone()));
        }
    }
    let galleys: Vec<_> = labels.iter().map(|(_, _, l)| painter.layout_no_wrap(l.clone(), semibold(10.0), Color32::WHITE)).collect();
    let lines: Vec<(Pos2, Pos2, Vec2)> = labels.iter().zip(&galleys).map(|((a, b, _), g)| (*a, *b, g.size() + PAD * 2.0)).collect();
    for (r, g) in place_labels(&lines, avoid.as_slice()).into_iter().zip(galleys) {
        painter.rect_filled(r, CornerRadius::same(3), t.guide);
        painter.galley(r.min + PAD, g, Color32::WHITE);
    }
}

/// Label boxes for lines `(a, b, label size)` in screen space: centred on the line when it is long
/// enough to show around the label, else (or when that spot is taken) slid along it, then beside it
/// (below a horizontal line, right of a vertical one, then the other side, further out each time)
/// until it clears `avoid` and every label placed before it.
pub fn place_labels(lines: &[(Pos2, Pos2, Vec2)], avoid: &[Rect]) -> Vec<Rect> {
    let mut placed: Vec<Rect> = avoid.to_vec();
    for (a, b, size) in lines {
        let mid = pos2((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        let d = *b - *a;
        let horizontal = d.x.abs() >= d.y.abs();
        // Along the line and out from it: the unit direction and one label's step.
        let (dir, out) = if horizontal { (vec2(1.0, 0.0), vec2(0.0, 1.0)) } else { (vec2(0.0, 1.0), vec2(1.0, 0.0)) };
        let (along, across) = if horizontal { (size.x, size.y) } else { (size.y, size.x) };
        let room = (d.length() - along) / 2.0 - GAP;
        let mut centres: Vec<Pos2> = Vec::new();
        if room >= 0.0 {
            centres.push(mid);
            // Slid along the line, both ways, while it still lies on the line.
            let mut n = 1.0;
            while n * (along + GAP) <= room && centres.len() < 32 {
                centres.extend([mid + dir * n * (along + GAP), mid - dir * n * (along + GAP)]);
                n += 1.0;
            }
        }
        for n in 0..32 {
            let off = across / 2.0 + GAP + n as f32 * (across + GAP);
            centres.extend([mid + out * off, mid - out * off]);
        }
        let free = |r: &Rect| !placed.iter().any(|p| p.expand(GAP / 2.0).intersects(*r));
        // Past every candidate (dozens of labels on one spot): stack below everything placed.
        let r = centres.into_iter().map(|c| Rect::from_center_size(c, *size)).find(free).unwrap_or_else(|| {
            let bottom = placed.iter().map(|p| p.max.y).fold(mid.y, f32::max);
            Rect::from_min_size(pos2(mid.x - size.x / 2.0, bottom + GAP), *size)
        });
        placed.push(r);
    }
    placed.split_off(avoid.len())
}

#[cfg(test)]
mod tests {
    use super::place_labels;
    use egui::{Pos2, Rect, pos2, vec2};

    const SIZE: egui::Vec2 = vec2(24.0, 14.0);

    fn no_overlaps(r: &[Rect]) -> bool {
        r.iter().enumerate().all(|(i, a)| r.iter().skip(i + 1).all(|b| !a.intersects(*b)))
    }

    #[test]
    fn long_lines_carry_their_label_in_the_middle() {
        let r = place_labels(&[(pos2(0.0, 50.0), pos2(200.0, 50.0), SIZE), (pos2(50.0, 0.0), pos2(50.0, 300.0), SIZE)], &[]);
        assert_eq!(r[0].center(), pos2(100.0, 50.0));
        assert_eq!(r[1].center(), pos2(50.0, 150.0));
    }

    #[test]
    fn short_lines_put_the_label_beside_them() {
        let r = place_labels(&[(pos2(0.0, 50.0), pos2(10.0, 50.0), SIZE), (pos2(50.0, 0.0), pos2(50.0, 6.0), SIZE)], &[]);
        assert!(r[0].min.y > 50.0, "below a short horizontal line: {:?}", r[0]);
        assert!(r[1].min.x > 50.0, "right of a short vertical line: {:?}", r[1]);
    }

    #[test]
    fn labels_never_overlap_even_when_crowded() {
        // Zoomed far out: every line collapses onto one spot.
        let crowd: Vec<(Pos2, Pos2, egui::Vec2)> =
            (0..100).map(|i| (pos2(10.0, 10.0), pos2(10.0 + (i % 2) as f32 * 0.5, 10.0 + (i % 3) as f32 * 0.5), SIZE)).collect();
        assert!(no_overlaps(&place_labels(&crowd, &[])));
        // Parallel lines a few pixels apart.
        let close: Vec<(Pos2, Pos2, egui::Vec2)> = (0..6).map(|i| (pos2(0.0, i as f32 * 3.0), pos2(100.0, i as f32 * 3.0), SIZE)).collect();
        let r = place_labels(&close, &[]);
        assert!(no_overlaps(&r));
        assert_eq!(r[0].center(), pos2(50.0, 0.0), "the first keeps its place");
    }

    #[test]
    fn labels_keep_clear_of_the_size_label() {
        let size = Rect::from_center_size(pos2(50.0, 100.0), vec2(60.0, 16.0));
        // A long line slides its label along itself, past the size label.
        let r = place_labels(&[(pos2(50.0, 80.0), pos2(50.0, 200.0), SIZE)], &[size]);
        assert_eq!(r.len(), 1);
        assert!(!r[0].intersects(size) && r[0].center().x == 50.0, "{:?}", r[0]);
        // A short one moves beside it.
        let r = place_labels(&[(pos2(50.0, 90.0), pos2(50.0, 110.0), SIZE)], &[size]);
        assert!(!r[0].intersects(size), "{:?}", r[0]);
    }
}
