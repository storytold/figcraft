use super::{CanvasState, Done, FigApp, Job, NodeId, Worker};

#[derive(Clone, PartialEq)]
pub(super) struct JobKey {
    /// The document and its revision, by value: a drag replaces the document every frame, and the
    /// allocator hands the freed block back — keyed by address, the frame after a drag step looked
    /// like the frame before it (ABA), the canvas kept its texture and the card froze for as long
    /// as the drag lasted. The revision (bumped by every command) is what says it changed.
    doc: (u64, u64),
    /// The IME composition on screen, while one is being typed: it changes without a command.
    preedit: Option<(figcraft_engine::Preedit, usize)>,
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
    /// The GPU canvas preference ([`figcraft_engine::Prefs::gpu_canvas`]): turning it on asks for
    /// a new frame like any other change the renderer sees.
    gpu: bool,
}

/// Whether a finished render goes on screen: an image of the file and page shown (`shown`), and
/// not a coarse preview over a refined image of the same or a newer request, or one that already
/// shows the whole view asked for at its zoom (the same zoom elsewhere on the page is no such
/// image).
pub(crate) fn shows(c: &CanvasState, done: &Done, shown: Option<(u64, NodeId)>) -> bool {
    if shown != Some((done.uid, done.page)) {
        return false;
    }
    let refined = !c.tex_coarse && c.tex_of == shown && c.tex_revision.is_some();
    // A newer request's preview over the refined image of an older one is only a step back when the
    // content is the same: a drag's next frame is newer content, whatever its request number.
    let newer_content = c.tex_revision.is_some_and(|shown| done.revision > shown);
    if done.coarse && refined && !newer_content && done.seq <= c.tex_seq {
        return false;
    }
    // A preview of *newer* content does go on over a refined image: while a drag moves a layer the
    // view does not change, and the frame that follows the pointer is the preview.
    let refined_here = refined
        && c.tex_revision == Some(done.revision)
        && c.tex_view
            .as_ref()
            .zip(c.want_view.as_ref())
            .is_some_and(|(have, want)| have.scale.to_bits() == want.scale.to_bits() && covers(have, want));
    // A preview of content the screen has moved past (a frame that landed late) is a step back.
    let behind = refined && c.tex_revision.is_some_and(|shown| done.revision < shown);
    !(done.coarse && (refined_here || behind))
}

/// Whether `outer` shows all of the page that `inner` does.
fn covers(outer: &figcraft_render::View, inner: &figcraft_render::View) -> bool {
    let (o, i) = (outer.page_rect(), inner.page_rect());
    // Rounding in the snapped origin, well under a device pixel.
    let eps = 1e-6 / inner.scale.max(1e-9);
    o.x0 <= i.x0 + eps && o.y0 <= i.y0 + eps && o.x1 >= i.x1 - eps && o.y1 >= i.y1 - eps
}

/// Ask the render thread for the frame the canvas needs, when it needs a new one. Returns how
/// long until a preview on screen should ask for its sharp frame (the UI wakes for it); `None`
/// when there is nothing to wait for.
#[allow(clippy::too_many_arguments)]
pub(super) fn request_render(
    app: &mut FigApp,
    worker: &mut Worker,
    ppp: f32,
    max_texture_side: u32,
    dots: Option<figcraft_render::Dots>,
    holding: bool,
) -> Option<std::time::Duration> {
    let Ok(ds) = app.session.ds() else { return None };
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
    let preedit = app.preedit_view().map(|p| (p.key.1.clone(), p.key.2));
    let doc = app.preedit_view().map(|p| p.doc.clone()).unwrap_or_else(|| ds.doc.clone());
    let outlined = if ds.view.outlines_selection { ds.selection.clone() } else { vec![] };
    let proof = ds.view.proof_srgb && !doc.color_profile.is_srgb();
    // The GPU rasterizer is a desktop feature and off unless the preferences ask for it.
    let gpu = cfg!(not(target_arch = "wasm32")) && app.session.prefs.gpu_canvas;
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
        doc: (ds.uid, ds.revision),
        preedit,
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
        gpu,
    };
    let view = figcraft_render::View { origin: requested, scale, width: size.0, height: size.1 };
    let mut nothing_new = app.canvas.submitted.as_ref() == Some(&key);
    if let Some(previous) = &app.canvas.submitted {
        let previous_origin = (previous.origin.0 as f64 / 1000.0, previous.origin.1 as f64 / 1000.0);
        let same_content = JobKey { origin: previous.origin, size: previous.size, ..key.clone() } == *previous;
        let covers = cached_render_covers(previous_origin, previous.size, origin, visible, scale);
        nothing_new = nothing_new || (same_content && covers);
    }
    // What is on screen is a preview from an interaction (the zoom or a drag was moving faster
    // than a full frame): once nothing new is asked for — the pointer is up and nothing has been
    // asked for a while — draw the sharp frame, even though this job's key matches the preview's.
    // A full frame takes a few previews' worth of time, so a drag that steps slowly keeps its
    // previews instead.
    let quiet = app.canvas.last_submitted.is_none_or(|at| at.elapsed().as_secs_f64() * 1000.0 >= crate::render_worker::GPU_REFINE_AFTER_MS);
    let sharpen = nothing_new && !holding && !worker.in_flight() && app.canvas.tex_coarse;
    let refine = sharpen && quiet && app.canvas.refined.as_ref() != Some(&key);
    if nothing_new && !refine {
        // A preview waits for the quiet time: wake the UI when it is over, and this frame asks again.
        if sharpen && !quiet {
            let waited = app.canvas.last_submitted.map_or(0.0, |at| at.elapsed().as_secs_f64() * 1000.0);
            let left = (crate::render_worker::GPU_REFINE_AFTER_MS - waited).max(1.0);
            return Some(std::time::Duration::from_millis(left.ceil() as u64));
        }
        return None;
    }
    let job = Job {
        doc,
        page: ds.page,
        view: view.clone(),
        outline: ds.view.outlines,
        outline_extras,
        outlined,
        skip: vec![],
        solo: key.solo,
        proof,
        dots,
        uid: ds.uid,
        revision: ds.revision,
        zoom_preview: ds.view.pixel_preview_scale().is_none(),
        refine,
        gpu,
        at: web_time::Instant::now(),
    };
    app.canvas.want_view = Some(job.view.clone());
    worker.submit(job);
    app.canvas.last_submitted = Some(web_time::Instant::now());
    if refine {
        app.canvas.refined = Some(key);
    } else {
        app.canvas.submitted = Some(key);
        app.canvas.refined = None;
    }
    None
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

/// A drag step replaces the document, and the allocator hands the freed block back: a job keyed by
/// the document's address then matches the frame before the drag, and the canvas never asks for the
/// moved one. The key is the document and its revision by value, so a step is a new job.
#[cfg(test)]
#[test]
fn a_moved_document_asks_for_a_new_frame() {
    let mut doc = figcraft_doc::Document::new("drag");
    let Some(page) = doc.first_page() else { return };
    let Ok(card) = doc.create(page, figcraft_doc::NodeKind::Rectangle, "card", 10.0, 10.0, 40.0, 30.0) else { return };
    let mut app = FigApp::new(figcraft_engine::Session::with_document(doc), Box::new(crate::NoServices));
    app.canvas.rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(96.0, 72.0));
    let mut worker = crate::render_worker::Worker::new(egui::Context::default());
    let _ = request_render(&mut app, &mut worker, 1.0, 16_383, None, false);
    let Some(first) = app.canvas.submitted.clone() else { return };
    // Where a drag's step lands: one command, one new revision.
    app.session.set_selection(vec![card]);
    assert!(app.run("node.set", serde_json::json!({"props": {"x": 20.0}})).is_some(), "the move");
    let _ = request_render(&mut app, &mut worker, 1.0, 16_383, None, false);
    let Some(second) = app.canvas.submitted.clone() else { return };
    assert!(first != second, "the moved frame was the job before it: the canvas keeps its texture");
}

/// Frames until the worker has nothing left (a few seconds at most).
#[cfg(test)]
fn render_idle(worker: &mut crate::render_worker::Worker) {
    for _ in 0..600 {
        if !worker.in_flight() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// An interaction on screen (a preview drawn at fewer pixels while the pointer moves) gets one
/// sharp frame once nothing new is asked for — and only one, until the interaction moves again.
#[cfg(test)]
#[test]
fn a_preview_on_screen_asks_for_the_sharp_frame_once() {
    let doc = figcraft_doc::Document::new("preview");
    let mut app = FigApp::new(figcraft_engine::Session::with_document(doc), Box::new(crate::NoServices));
    app.canvas.rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(96.0, 72.0));
    let mut worker = crate::render_worker::Worker::new(egui::Context::default());
    let _ = request_render(&mut app, &mut worker, 1.0, 16_383, None, false);
    let first = worker.submitted_count();
    assert_eq!(first, 1, "the first frame is asked for");
    render_idle(&mut worker);
    app.canvas.tex_coarse = true;
    // A pointer held down is still interacting (a drag): its previews are not sharpened.
    let _ = request_render(&mut app, &mut worker, 1.0, 16_383, None, true);
    assert_eq!(worker.submitted_count(), first, "a drag in progress keeps its previews");
    // Nothing new to draw for a while, and a preview on screen: the sharp frame is asked for.
    std::thread::sleep(std::time::Duration::from_millis(70));
    let _ = request_render(&mut app, &mut worker, 1.0, 16_383, None, false);
    assert_eq!(worker.submitted_count(), first + 1, "the preview asks for the sharp frame");
    let _ = request_render(&mut app, &mut worker, 1.0, 16_383, None, false);
    assert_eq!(worker.submitted_count(), first + 1, "and only once");
    // The sharp frame is on screen: nothing more is asked for.
    render_idle(&mut worker);
    app.canvas.tex_coarse = false;
    let _ = request_render(&mut app, &mut worker, 1.0, 16_383, None, false);
    assert_eq!(worker.submitted_count(), first + 1);
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
