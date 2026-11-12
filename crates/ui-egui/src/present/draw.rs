use super::actions::texture_scale;
use super::layout::{Layout, Place, hotspots};
use super::transition::{smart_interpolate, transition_frames};
use super::{ACCENT, Color32, CornerRadius, FLASH_SECS, NodeId, OverlayState, Presenter, Rect, Stroke, StrokeKind, Transition, pos2, vec2};

pub(super) fn draw(p: &mut Presenter, painter: &egui::Painter, ctx: &egui::Context, l: &Layout, now: f64) {
    let ppp = f64::from(ctx.pixels_per_point());
    let px = l.scale * ppp;
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let to_tex = p.texture(ctx, &l.disp, p.current, px, now);
    // Transition progress.
    let mut drawn = false;
    if let Some(a) = &p.anim {
        let raw = ((now - a.start) / a.dur.max(1e-3)).clamp(0.0, 1.0);
        if raw >= 1.0 {
            p.anim = None;
        } else {
            ctx.request_repaint();
            let e = a.easing.eval(raw);
            let from_place = a.from_doc.world_bounds(a.from_frame).map(|fb| Place::centered(fb, l.screen.center(), l.scale));
            if matches!(a.transition, Transition::SmartAnimate) {
                #[cfg(not(target_arch = "wasm32"))]
                let t0 = std::time::Instant::now();
                let tmp = smart_interpolate(&a.from_doc, a.from_frame, &l.disp, p.current, e);
                let bounds = figcraft_render::export_bounds(&tmp, p.current);
                if let Some(b) = bounds
                    && let Some(pix) = figcraft_render::render_node(&tmp, p.current, texture_scale(ctx, b, px))
                {
                    let img = crate::render_worker::screen_image(&pix, tmp.color_profile);
                    let tex = match &mut p.anim_tex {
                        Some(t) => {
                            t.set(img, egui::TextureOptions::LINEAR);
                            t.clone()
                        }
                        None => {
                            let t = ctx.load_texture("present-anim", img, egui::TextureOptions::LINEAR);
                            p.anim_tex = Some(t.clone());
                            t
                        }
                    };
                    painter.with_clip_rect(l.screen).image(tex.id(), l.place.rect(b), uv, Color32::WHITE);
                    drawn = true;
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    p.render_ms = t0.elapsed().as_secs_f64() * 1000.0;
                }
            } else if let (Some(ft), Some(to), Some(fp)) = (&a.from_tex, &to_tex, from_place) {
                let f = transition_frames(a.transition, e as f32);
                let from_rect = fp.rect(ft.bounds);
                let from_frame_rect = a.from_doc.world_bounds(a.from_frame).map(|b| fp.rect(b)).unwrap_or(from_rect);
                let to_rect = l.place.rect(to.bounds);
                let off = |r: Rect, o: (f32, f32)| vec2(r.width() * o.0, r.height() * o.1);
                // Without a device a dissolve shows both frames whole; on one, the screen clips.
                let dissolve_both = l.device.is_none() && matches!(a.transition, Transition::Dissolve);
                let clip = painter.with_clip_rect(if dissolve_both { l.screen.union(from_frame_rect) } else { l.screen });
                let draw_from = |p: &egui::Painter| {
                    if f.from_alpha > 0.0 {
                        p.image(ft.tex.id(), from_rect.translate(off(from_frame_rect, f.from_off)), uv, Color32::WHITE.gamma_multiply(f.from_alpha));
                    }
                };
                let draw_to = |p: &egui::Painter| {
                    p.image(to.tex.id(), to_rect.translate(off(l.screen, f.to_off)), uv, Color32::WHITE.gamma_multiply(f.to_alpha));
                };
                if f.from_on_top {
                    draw_to(&clip);
                    draw_from(&clip);
                } else {
                    draw_from(&clip);
                    draw_to(&clip);
                }
                drawn = true;
            }
        }
    }
    if !drawn && let Some(to) = &to_tex {
        painter.with_clip_rect(l.clip).image(to.tex.id(), l.place.rect(to.bounds), uv, Color32::WHITE);
    }
    // Overlays (each with its background and opening transition); closing ones run it backwards.
    let mut ovs: Vec<(f64, OverlayState)> =
        p.overlays.iter().map(|o| (if o.dur > 0.0 { ((now - o.opened) / o.dur).clamp(0.0, 1.0) } else { 1.0 }, o.clone())).collect();
    let mut placed: Vec<(NodeId, Place, Rect)> = l.overlays.clone();
    for ((at, o), pl) in p.closing.iter().zip(l.closing.iter()) {
        if now - at >= o.dur {
            continue;
        }
        ovs.insert(0, (1.0 - ((now - at) / o.dur.max(1e-3)).clamp(0.0, 1.0), o.clone()));
        placed.insert(0, *pl);
    }
    let on_screen = painter.with_clip_rect(l.screen);
    for ((raw, o), (id, place, orect)) in ovs.iter().zip(placed.iter()) {
        let raw = *raw;
        if raw < 1.0 {
            ctx.request_repaint();
        }
        let e = o.easing.eval(raw) as f32;
        let f = if matches!(o.transition, Transition::Instant) {
            transition_frames(Transition::Dissolve, 1.0)
        } else {
            transition_frames(o.transition, e)
        };
        if let Some(bgc) = o.background {
            let c = crate::theme::doc_c32(ctx, bgc);
            on_screen.rect_filled(l.screen, CornerRadius::ZERO, c.gamma_multiply(raw as f32));
        }
        if let Some(tex) = p.texture(ctx, &l.disp, *id, px, now) {
            let r = place.rect(tex.bounds).translate(vec2(orect.width() * f.to_off.0, orect.height() * f.to_off.1));
            on_screen.image(tex.tex.id(), r, uv, Color32::WHITE.gamma_multiply(f.to_alpha));
        }
    }
    // Finished closing animations go (after drawing, so they stayed paired with the layout).
    p.closing.retain(|(at, o)| now - at < o.dur);
    // Hotspot flash after a click on nothing.
    if now < p.flash_until {
        let (root, place, clip) = l.active(p.current);
        let a = ((p.flash_until - now) / FLASH_SECS) as f32;
        for (_, r) in hotspots(&l.disp, root, &place, clip) {
            painter.rect_filled(r, CornerRadius::same(4), ACCENT.gamma_multiply(0.25 * a));
            painter.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, ACCENT.gamma_multiply(a)), StrokeKind::Outside);
        }
        ctx.request_repaint();
    }
}
