use super::layout::{bezel, upright_w};
use super::{Color32, CornerRadius, DeviceColor, Rect, Stroke, StrokeKind, pos2, vec2};

/// The black glass around a device's screen.
const GLASS: Color32 = Color32::from_rgb(0x16, 0x16, 0x18);

/// A device colour lighter (`k` > 0, towards white) or darker (`k` < 0, towards black).
fn shade([r, g, b]: [u8; 3], k: f32) -> Color32 {
    let k = k.clamp(-1.0, 1.0);
    let f = |c: u8| {
        let c = f32::from(c);
        (if k >= 0.0 { c + (255.0 - c) * k } else { c * (1.0 + k) }).round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(f(r), f(g), f(b))
}

/// The width of the coloured band round a device's glass (upright screen width `w`, bezel `b`).
fn rim(w: f32, b: f32) -> f32 {
    (w * 0.016).clamp(1.5, (b * 0.5).max(1.5))
}

/// Body of the presentation device behind the screen `s` (drawn in code: rounded slabs in the
/// device's colour with black glass round the screen). Turned to landscape (a quarter turn
/// counter-clockwise), a phone's left side is at the bottom.
pub(super) fn device_body(p: &egui::Painter, device: &str, s: Rect, landscape: bool, color: DeviceColor) {
    let (body, edge, metal) = (shade(color.rgb, 0.0), shade(color.rgb, -0.3), shade(color.rgb, 0.12));
    let w = upright_w(s.size(), landscape);
    let (b, screen_r) = bezel(device, w);
    let band = rim(w, b);
    let radius = |r: f32| CornerRadius::same(r.clamp(0.0, 255.0) as u8);
    match device {
        "MacBook Air" => {
            // The lid's coloured edge round black glass, then the base: a thin slab wider than
            // the lid, with a finger notch.
            let lid = s.expand(b);
            p.rect_filled(lid, radius(b * 1.2), body);
            p.rect_filled(lid.shrink(band), radius(b * 1.2 - band), GLASS);
            let base = Rect::from_min_max(pos2(lid.min.x - w * 0.08, lid.max.y), pos2(lid.max.x + w * 0.08, lid.max.y + (w * 0.022).max(6.0)));
            p.rect_filled(base, CornerRadius { nw: 2, ne: 2, sw: 10, se: 10 }, metal);
            let notch = Rect::from_center_size(pos2(base.center().x, base.min.y + 1.5), vec2(w * 0.12, 3.0));
            p.rect_filled(notch, CornerRadius::same(2), edge);
        }
        "Desktop" => {
            let screen = s.expand(b);
            p.rect_filled(screen, CornerRadius::same(8), body);
            p.rect_filled(screen.shrink(band), radius(8.0 - band), GLASS);
            let neck =
                Rect::from_min_max(pos2(screen.center().x - w * 0.06, screen.max.y), pos2(screen.center().x + w * 0.06, screen.max.y + w * 0.08));
            p.rect_filled(neck, CornerRadius::ZERO, metal);
            let foot = Rect::from_min_max(pos2(neck.min.x - w * 0.06, neck.max.y), pos2(neck.max.x + w * 0.06, neck.max.y + 6.0));
            p.rect_filled(foot, CornerRadius::same(3), edge);
        }
        _ => {
            // Phones and tablets: a rounded slab (phones with side buttons).
            let slab = s.expand(b);
            if !device.starts_with("iPad") {
                // (on the left side, from the top, length) as fractions of the upright side.
                for (left, f0, h) in [(true, 0.18, 0.06), (true, 0.27, 0.1), (false, 0.24, 0.13)] {
                    let btn = if landscape {
                        let y = if left { slab.max.y - 0.5 } else { slab.min.y - 2.5 };
                        Rect::from_min_size(pos2(slab.min.x + slab.width() * f0, y), vec2(slab.width() * h, 2.5))
                    } else {
                        let x = if left { slab.min.x - 2.0 } else { slab.max.x };
                        Rect::from_min_size(pos2(x, slab.min.y + slab.height() * f0), vec2(2.5, slab.height() * h))
                    };
                    p.rect_filled(btn, CornerRadius::same(1), edge);
                }
            }
            p.rect_filled(slab, radius(screen_r + b), body);
            p.rect_stroke(slab, radius(screen_r + b), Stroke::new(1.0, edge), StrokeKind::Inside);
            // The glass reaches round the screen's square corners (masked by `device_glass`).
            let glass = b - band;
            p.rect_filled(slab.shrink(band), radius((screen_r + glass).min(glass * 3.4)), GLASS);
        }
    }
}

/// What sits on top of the screen: rounded screen corners (glass masks the frame's square
/// corners) and a phone's camera pill (at the left when turned to landscape).
pub(super) fn device_glass(p: &egui::Painter, device: &str, s: Rect, landscape: bool) {
    let w = upright_w(s.size(), landscape);
    let (_, screen_r) = bezel(device, w);
    if screen_r <= 0.0 {
        return;
    }
    p.add(corner_masks(s, screen_r.min(s.width() / 2.0).min(s.height() / 2.0), GLASS));
    if device.starts_with("iPhone") {
        let pill = if landscape {
            Rect::from_center_size(pos2(s.min.x + w * 0.045, s.center().y), vec2(w * 0.085, w * 0.3))
        } else {
            Rect::from_center_size(pos2(s.center().x, s.min.y + w * 0.045), vec2(w * 0.3, w * 0.085))
        };
        p.rect_filled(pill, CornerRadius::same((w * 0.0425).min(255.0) as u8), Color32::BLACK);
    }
}

/// Fill what lies between each corner of `s` and its rounding of radius `r` (fans of triangles
/// from the corner to the arc).
fn corner_masks(s: Rect, r: f32, color: Color32) -> egui::Shape {
    const SEGMENTS: usize = 12;
    if !(r.is_finite() && r > 0.0) {
        return egui::Shape::Noop;
    }
    let mut mesh = egui::Mesh::default();
    // (corner, centre of its arc, angle the arc starts at).
    let corners = [
        (s.left_top(), s.left_top() + vec2(r, r), std::f32::consts::PI),
        (s.right_top(), s.right_top() + vec2(-r, r), std::f32::consts::FRAC_PI_2 * 3.0),
        (s.right_bottom(), s.right_bottom() + vec2(-r, -r), 0.0),
        (s.left_bottom(), s.left_bottom() + vec2(r, -r), std::f32::consts::FRAC_PI_2),
    ];
    for (corner, centre, start) in corners {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(corner, color);
        for k in 0..=SEGMENTS {
            let a = start + std::f32::consts::FRAC_PI_2 * k as f32 / SEGMENTS as f32;
            mesh.colored_vertex(centre + r * vec2(a.cos(), a.sin()), color);
        }
        for k in 0..SEGMENTS as u32 {
            mesh.add_triangle(base, base + 1 + k, base + 2 + k);
        }
    }
    egui::Shape::mesh(mesh)
}
