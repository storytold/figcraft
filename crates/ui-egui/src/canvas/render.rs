use super::{Arc, CanvasState, Done, FigApp, Job, NodeId, Worker};

#[derive(Clone, PartialEq)]
pub(super) struct JobKey {
    doc: usize,
    page: NodeId,
    origin: (i64, i64),
    scale: i64,
    size: (u32, u32),
    outline: bool,
    outline_extras: figcraft_render::Outline,
    outlined: Vec<NodeId>,
    skip: Vec<NodeId>,
    solo: Option<NodeId>,
    /// Text lays out anew when fonts arrive (system fonts load in the background).
    fonts: u64,
    proof: bool,
    dots: Option<figcraft_render::Dots>,
}

/// Whether a finished render goes on screen: an image of the file and page shown (`shown`), and
/// not a coarse preview over a refined image of the same or a newer request, or one that already
/// shows the whole view asked for at its zoom (the same zoom elsewhere on the page is no such
/// image).
pub(crate) fn shows(c: &CanvasState, done: &Done, shown: Option<(u64, NodeId)>) -> bool {
    if shown != Some((done.uid, done.page)) {
        return false;
    }
    let refined = !c.tex_coarse && c.tex_of == shown;
    if done.coarse && refined && done.seq <= c.tex_seq {
        return false;
    }
    let refined_here = refined
        && c.tex_view
            .as_ref()
            .zip(c.want_view.as_ref())
            .is_some_and(|(have, want)| have.scale.to_bits() == want.scale.to_bits() && covers(have, want));
    !(done.coarse && refined_here)
}

/// Whether `outer` shows all of the page that `inner` does.
fn covers(outer: &figcraft_render::View, inner: &figcraft_render::View) -> bool {
    let (o, i) = (outer.page_rect(), inner.page_rect());
    // Rounding in the snapped origin, well under a device pixel.
    let eps = 1e-6 / inner.scale.max(1e-9);
    o.x0 <= i.x0 + eps && o.y0 <= i.y0 + eps && o.x1 >= i.x1 - eps && o.y1 >= i.y1 - eps
}

pub(super) fn request_render(app: &mut FigApp, worker: &mut Worker, ppp: f32, max_texture_side: u32, dots: Option<figcraft_render::Dots>) {
    let Ok(ds) = app.session.ds() else { return };
    let rect = app.canvas.rect;
    let (mut ox, mut oy) = ds.view.origin();
    let (scale, visible_w, visible_h) = if let Some(k) = ds.view.pixel_preview_scale() {
        // Pixel preview: render `k` pixels per page unit from a whole-pixel origin, shown magnified
        // without smoothing.
        (ox, oy) = ((ox * k).floor() / k, (oy * k).floor() / k);
        let z = ds.view.zoom.max(1e-9) / k;
        (k, (f64::from(rect.width()) / z).ceil() as u32 + 2, (f64::from(rect.height()) / z).ceil() as u32 + 2)
    } else {
        (ds.view.zoom * f64::from(ppp), (rect.width() * ppp).ceil().max(1.0) as u32, (rect.height() * ppp).ceil().max(1.0) as u32)
    };
    // The edited text is drawn live (an IME composition included); the caret is an overlay.
    let doc = app.preedit_view().map(|p| p.doc.clone()).unwrap_or_else(|| ds.doc.clone());
    let outlined = if ds.view.outlines_selection { ds.selection.clone() } else { vec![] };
    let proof = ds.view.proof_srgb && !doc.color_profile.is_srgb();
    let outline_extras = figcraft_render::Outline { hidden: ds.view.outline_hidden, bounds: ds.view.outline_bounds, light: false };
    // Render beyond the visible viewport. During a pan the previous texture then still covers
    // the screen while the worker renders newly exposed content. Request a new buffer before the
    // visible area reaches its edge, rather than exposing an empty strip each pointer move.
    let margin = overscan_margin(visible_w, visible_h, max_texture_side);
    let origin = (ox, oy);
    let visible = (visible_w, visible_h);
    let requested = (ox - f64::from(margin) / scale, oy - f64::from(margin) / scale);
    let size = (visible_w.saturating_add(margin * 2).min(max_texture_side), visible_h.saturating_add(margin * 2).min(max_texture_side));
    let key = JobKey {
        doc: Arc::as_ptr(&doc) as usize,
        page: ds.page,
        origin: ((requested.0 * 1000.0) as i64, (requested.1 * 1000.0) as i64),
        scale: (scale * 10000.0) as i64,
        size,
        outline: ds.view.outlines,
        outline_extras,
        outlined: outlined.clone(),
        skip: vec![],
        solo: ds.focused(),
        fonts: figcraft_text::layout_generation(),
        proof,
        dots,
    };
    if let Some(previous) = &app.canvas.submitted {
        let previous_origin = (previous.origin.0 as f64 / 1000.0, previous.origin.1 as f64 / 1000.0);
        let same_content = JobKey { origin: previous.origin, size: previous.size, ..key.clone() } == *previous;
        let covers = cached_render_covers(previous_origin, previous.size, origin, visible, scale);
        if same_content && covers {
            return;
        }
    }
    if app.canvas.submitted.as_ref() == Some(&key) {
        return;
    }
    let job = Job {
        doc,
        page: ds.page,
        view: figcraft_render::View { origin: requested, scale, width: size.0, height: size.1 },
        outline: ds.view.outlines,
        outline_extras,
        outlined,
        skip: vec![],
        solo: key.solo,
        proof,
        dots,
        uid: ds.uid,
        zoom_preview: ds.view.pixel_preview_scale().is_none(),
        at: web_time::Instant::now(),
    };
    app.canvas.want_view = Some(job.view.clone());
    worker.submit(job);
    app.canvas.submitted = Some(key);
}

/// Leave a 96-pixel reserve so the next render starts before a pan reaches a cached edge.
fn cached_render_covers(buffer_origin: (f64, f64), buffer_size: (u32, u32), view_origin: (f64, f64), view_size: (u32, u32), scale: f64) -> bool {
    if !scale.is_finite() || scale <= 0.0 {
        return false;
    }
    let guard_x = 96.0_f64.min(f64::from(buffer_size.0.saturating_sub(view_size.0)) / 4.0) / scale;
    let guard_y = 96.0_f64.min(f64::from(buffer_size.1.saturating_sub(view_size.1)) / 4.0) / scale;
    view_origin.0 >= buffer_origin.0 + guard_x
        && view_origin.1 >= buffer_origin.1 + guard_y
        && view_origin.0 + f64::from(view_size.0) / scale <= buffer_origin.0 + f64::from(buffer_size.0) / scale - guard_x
        && view_origin.1 + f64::from(view_size.1) / scale <= buffer_origin.1 + f64::from(buffer_size.1) / scale - guard_y
}

/// Bound the extra rendered area to about 1.6 times the visible area. A fixed large margin can
/// multiply the pixel count several-fold on a narrow or small viewport.
fn overscan_margin(width: u32, height: u32, max_texture_side: u32) -> u32 {
    width.min(height).saturating_div(8).min(128).min(max_texture_side.saturating_sub(width.max(height)) / 2)
}

#[cfg(test)]
#[test]
fn pan_uses_cached_pixels_until_the_refill_boundary() {
    let margin = overscan_margin(1000, 800, 16_383);
    assert_eq!(margin, 100);
    let buffer = (-f64::from(margin), -f64::from(margin));
    let size = (1000 + 2 * margin, 800 + 2 * margin);
    assert!(cached_render_covers(buffer, size, (0.0, 0.0), (1000, 800), 1.0));
    assert!(cached_render_covers(buffer, size, (50.0, 0.0), (1000, 800), 1.0));
    assert!(!cached_render_covers(buffer, size, (80.0, 0.0), (1000, 800), 1.0));
    assert!(!cached_render_covers(buffer, size, (-80.0, 0.0), (1000, 800), 1.0));
    assert!(cached_render_covers((-24.0, -24.0), (1048, 848), (10.0, 0.0), (1000, 800), 1.0));
    assert!(!cached_render_covers((-24.0, -24.0), (1048, 848), (19.0, 0.0), (1000, 800), 1.0));
}

#[cfg(test)]
#[test]
fn overscan_stays_within_texture_and_pixel_budget() {
    for (width, height, limit) in [(1000, 800, 16_383), (390, 844, 16_383), (2560, 1440, 16_383), (1000, 800, 1060)] {
        let margin = overscan_margin(width, height, limit);
        assert!(width + 2 * margin <= limit || width > limit);
        assert!(height + 2 * margin <= limit || height > limit);
        let rendered = u64::from(width + 2 * margin) * u64::from(height + 2 * margin);
        let visible = u64::from(width) * u64::from(height);
        assert!(rendered * 5 <= visible * 8); // At most 1.6× the visible pixels.
    }
}
