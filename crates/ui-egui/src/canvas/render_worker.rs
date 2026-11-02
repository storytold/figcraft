//! Canvas rendering off the UI thread (desktop) or inline (web). The latest request wins (queued
//! older ones are skipped); the UI shows each finished image, re-projected to the current view,
//! until the next one arrives.
//! The renderer keeps its last image ([`figcraft_render::Retained`]): a pan renders only the
//! uncovered strips and an edit only the areas of the layers that changed.
//! A new zoom can't reuse that image, and the old one, re-projected, covers little of the view
//! after a zoom out (an eighth of the width from 800% to 100%). When full renders are slow, the
//! whole view is first rendered coarsely ([`PREVIEW_FACTOR`] times fewer pixels each way, kept
//! apart from the full image so pans and edits still build on it), then refined unless a newer
//! request is already waiting.
//! Without a measure of the new render (the first one, another page or file, a zoom after quick
//! renders) nothing is spent up front: on the desktop a watchdog thread renders the preview once
//! the render from scratch has run past [`PREVIEW_AFTER_MS`], beside it (the full render keeps the
//! other cores), and drops it if the full image is there first.
//! The browser renders on the UI thread, so a zoom's full render blocks the page: there a preview
//! is refined only once no newer request has come for [`SETTLE_MS`] (the zoom has paused), or for
//! as long as the last full render took when that is shorter (a wait never costs more than the
//! render it may save), and the full render is then drawn a band at a time
//! ([`figcraft_render::Steps`]), at least a band a frame and more while they fit in [`STEP_MS`],
//! so input is handled while it draws (on a page whose render takes seconds, a band is a
//! fraction of that: eight at most).

use std::sync::Arc;

use figcraft_color::ColorProfile;
use figcraft_doc::{Document, NodeId};
use figcraft_render::{Begun, Pixmap, Retained, Steps, View, Work};
// A monotonic clock in the browser too: preview decisions rest on how long renders take there.
use web_time::Instant;

/// A preview has this many times fewer pixels each way than the view it stands in for.
pub const PREVIEW_FACTOR: f64 = 4.0;
/// Full renders quicker than this (about a frame) show without a preview.
pub const PREVIEW_AFTER_MS: f64 = 12.0;
/// The browser refines a zoom's preview only once no newer request came for this long (or for as
/// long as the last full render took, when shorter): a full render there blocks the page, and one
/// between every two wheel steps of a zoom held it up for seconds.
pub const SETTLE_MS: u64 = 120;
/// The browser draws a full render a band at a time ([`figcraft_render::Steps`]): at least one
/// band a frame, and more while they fit in this long, so input is handled while it draws. A
/// band of a render that takes seconds takes a fraction of that (see [`MAX_BANDS`]).
pub const STEP_MS: f64 = 24.0;
/// Bands are at least this many rows ...
const MIN_BAND_ROWS: u32 = 64;
/// ... and a view has at most this many: a frame draws at least one band, so the longest frame of
/// a render drawn in steps is about a band's time, a part of the render rather than the whole of it
/// (0.7–1.4 s where the render took 3.7–4.0 s, measured for #1496; more bands would repeat more of
/// the overlap and of the per-band work).
const MAX_BANDS: u32 = 8;

#[derive(Clone)]
pub struct Job {
    pub doc: Arc<Document>,
    pub page: NodeId,
    pub view: View,
    pub outline: bool,
    /// Outline mode's options (hidden layers, object bounds).
    pub outline_extras: figcraft_render::Outline,
    /// Layers drawn as outlines (outlines for the selection).
    pub outlined: Vec<NodeId>,
    pub skip: Vec<NodeId>,
    /// Only this layer (the focus view).
    pub solo: Option<NodeId>,
    /// Soft-proof sRGB: stripe the colours sRGB can't show.
    pub proof: bool,
    /// A board's dot grid under the layers.
    pub dots: Option<figcraft_render::Dots>,
    /// The document's [`figcraft_engine::DocState::uid`]: images of other files are never shown.
    pub uid: u64,
    /// A coarse preview may come first when the zoom changes (not in pixel preview, whose pixels
    /// are the point).
    pub zoom_preview: bool,
    /// When the UI asked for it: how long the image took to reach the screen.
    pub at: Instant,
}

pub struct Done {
    /// The image, ready for a texture (converted off the UI thread).
    pub image: egui::ColorImage,
    /// The view the image shows (the requested one snapped outwards to whole device pixels).
    pub view: View,
    pub ms: f64,
    pub work: Work,
    /// What it is an image of ([`Job::uid`], [`Job::page`]).
    pub uid: u64,
    pub page: NodeId,
    /// A coarse preview, refined by a later image unless a newer request replaced it.
    pub coarse: bool,
    /// [`Job::at`].
    pub at: Instant,
    /// The request it answers, numbered by the worker in order (0 outside a worker).
    pub seq: u64,
}

/// A render of a document in `profile` as a texture image for the screen (sRGB).
pub fn screen_image(pix: &Pixmap, profile: ColorProfile) -> egui::ColorImage {
    let size = [pix.width() as usize, pix.height() as usize];
    if profile.is_srgb() {
        return egui::ColorImage::from_rgba_premultiplied(size, pix.data());
    }
    let mut data = pix.data().to_vec();
    profile.conversion_to(ColorProfile::Srgb).apply_pixels(&mut data);
    egui::ColorImage::from_rgba_premultiplied(size, &data)
}

/// Soft proof: grey diagonal stripes over the pixels whose colours sRGB can't show.
fn mark_outside_srgb(image: &mut egui::ColorImage, pix: &Pixmap, profile: ColorProfile) {
    let w = image.size[0].max(1);
    let clipped = profile.conversion_to(ColorProfile::Srgb).clipped_pixels(pix.data());
    for (i, (px, out)) in image.pixels.iter_mut().zip(clipped).enumerate() {
        if out && ((i % w) + (i / w)) % 8 < 4 {
            *px = egui::Color32::from_gray(128);
        }
    }
}

/// The canvas is a preview: what's too small to see in detail at this zoom is approximated.
fn options(job: &Job) -> figcraft_render::Options {
    figcraft_render::Options {
        outline_mode: job.outline,
        outline: job.outline_extras,
        outlined: job.outlined.clone(),
        skip: job.skip.clone(),
        solo: job.solo,
        lod: true,
        magnified_image_pixels: true,
        dots: job.dots,
        ..Default::default()
    }
}

/// Render `view` of the job (its own view, or the coarse one standing in for it).
fn run(job: &Job, view: &View, retained: &mut Retained, coarse: bool) -> Option<Done> {
    let t0 = Instant::now();
    let (view, pix, work) = retained.render(&job.doc, job.page, view, &options(job))?;
    Some(done(job, view, pix, work, t0.elapsed().as_secs_f64() * 1000.0, coarse))
}

/// The image `pix` of `view`, rendered for `job` in `ms`.
fn done(job: &Job, view: View, pix: &Pixmap, work: Work, ms: f64, coarse: bool) -> Done {
    let mut image = screen_image(pix, job.doc.color_profile);
    if job.proof {
        mark_outside_srgb(&mut image, pix, job.doc.color_profile);
    }
    Done { image, view, ms, work, uid: job.uid, page: job.page, coarse, at: job.at, seq: 0 }
}

/// `view` with [`PREVIEW_FACTOR`] times fewer pixels each way, showing at least the same area.
pub fn coarse_view(view: &View) -> View {
    let shrink = |n: u32| (f64::from(n) / PREVIEW_FACTOR).ceil().max(1.0) as u32;
    View { origin: view.origin, scale: view.scale / PREVIEW_FACTOR, width: shrink(view.width), height: shrink(view.height) }
}

/// What the render thread keeps between jobs: the canvas image, the previews standing in for it
/// while the zoom changes, and what the last full render drew and took.
pub struct Renders {
    full: Retained,
    preview: Retained,
    /// The last full render's document, page and scale.
    last: Option<(u64, NodeId, f64)>,
    /// How long the last render from scratch took.
    full_ms: f64,
    /// Previews come first only when renders from scratch take at least this long.
    preview_after_ms: f64,
    /// A full render being drawn band by band (the browser): the job it was started for, the
    /// font generation then, the bands and the time they took so far.
    steps: Option<Stepping>,
}

/// A full render drawn in steps and what it is of: a newer job of the same file, page, zoom, size
/// and render options keeps it going (its image is then patched to the job, as a pan or an edit
/// is), any other drops it.
struct Stepping {
    job: Job,
    fonts: u64,
    steps: Box<Steps>,
    spent: f64,
}

/// Whether `a` and `b` are drawn the same way: the same file, page, zoom, size and options (the
/// view may be elsewhere and the document edited: a render of one is patched to the other).
fn same_settings(a: &Job, b: &Job) -> bool {
    a.uid == b.uid
        && a.page == b.page
        && a.view.scale.to_bits() == b.view.scale.to_bits()
        && (a.view.width, a.view.height) == (b.view.width, b.view.height)
        && a.outline == b.outline
        && a.outline_extras == b.outline_extras
        && a.outlined == b.outlined
        && a.skip == b.skip
        && a.solo == b.solo
        && a.proof == b.proof
        && a.dots == b.dots
}

/// Whether a render drawn for `a` (with fonts of generation `fonts`) is `b`'s image as it is:
/// the same settings, document and view, and no fonts arrived meanwhile.
fn same_request(a: &Job, fonts: u64, b: &Job) -> bool {
    same_settings(a, b)
        && Arc::ptr_eq(&a.doc, &b.doc)
        && a.view.origin.0.to_bits() == b.view.origin.0.to_bits()
        && a.view.origin.1.to_bits() == b.view.origin.1.to_bits()
        && figcraft_text::layout_generation() == fonts
}

/// What [`Renders::refine_for`] got to.
pub enum Refined {
    /// The image (`None`: nothing could be drawn).
    Done(Option<Done>),
    /// Bands remain: call again (next frame).
    More,
    /// A full render drawn for what has changed since (an edit, a pan or images that came while
    /// it was drawn), shown meanwhile; it is drawn again: call again.
    Shown(Done),
}

impl Default for Renders {
    fn default() -> Self {
        Self::with_threshold(PREVIEW_AFTER_MS)
    }
}

impl Renders {
    /// Previews when renders from scratch take at least `ms` (tests: 0, always).
    pub fn with_threshold(ms: f64) -> Self {
        Self { full: Retained::default(), preview: Retained::default(), last: None, full_ms: 0.0, preview_after_ms: ms, steps: None }
    }

    /// Whether `job` is a render from scratch: the kept image is of another zoom, page or file, or
    /// there is none.
    fn starts_over(&self, job: &Job) -> bool {
        self.last.is_none_or(|(uid, page, scale)| uid != job.uid || page != job.page || scale.to_bits() != job.view.scale.to_bits())
    }

    /// Whether `job` gets a preview first: it renders from scratch and those have been slow on a
    /// page already drawn (the first render has no measure).
    fn wants_preview(&self, job: &Job) -> bool {
        job.zoom_preview && self.last.is_some() && self.full_ms_so_far() >= self.preview_after_ms && self.starts_over(job)
    }

    /// How long the last render from scratch took, or what the one being drawn in steps has taken
    /// so far when that is more: a render dropped mid-way (a zoom going on) leaves a measure too.
    pub fn full_ms_so_far(&self) -> f64 {
        self.full_ms.max(self.steps.as_ref().map_or(0.0, |s| s.spent))
    }

    /// Whether `job` should get a preview should its render run long: from scratch, with none first.
    pub fn wants_late_preview(&self, job: &Job) -> bool {
        job.zoom_preview && self.starts_over(job) && !self.wants_preview(job)
    }

    /// The coarse preview of the whole of `job`'s view, when one should come first.
    pub fn preview(&mut self, job: &Job) -> Option<Done> {
        if !self.wants_preview(job) {
            return None;
        }
        preview_with(job, &mut self.preview)
    }

    /// The image of `job`'s view, built on the kept one where it can be.
    pub fn refine(&mut self, job: &Job) -> Option<Done> {
        let done = guarded(&mut self.full, |r| run(job, &job.view, r, false));
        if let Some(d) = &done {
            self.last = Some((job.uid, job.page, job.view.scale));
            if d.work == Work::Full {
                self.full_ms = d.ms;
            }
        }
        done
    }

    /// [`Self::refine`] for about `budget_ms` at a time (the browser): a full render is drawn band by
    /// band over several calls, the kept image (re-projected) or the preview showing meanwhile.
    pub fn refine_for(&mut self, job: &Job, budget_ms: f64) -> Refined {
        let t0 = Instant::now();
        if let Some(s) = self.steps.as_ref()
            && !same_settings(&s.job, job)
        {
            // Dropped for a render of the new view. What its bands took is at least what a full
            // render takes: enough for the next zoom to get its preview and its wait.
            self.full_ms = self.full_ms.max(s.spent);
            self.steps = None;
        }
        if self.steps.is_none()
            && let Some(r) = self.begin(job, t0, None)
        {
            return r;
        }
        let Some(s) = self.steps.as_mut() else { return Refined::Done(None) };
        let steps = &mut s.steps;
        let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            loop {
                if steps.step() {
                    return true;
                }
                if t0.elapsed().as_secs_f64() * 1000.0 >= budget_ms {
                    return false;
                }
            }
        }));
        s.spent += t0.elapsed().as_secs_f64() * 1000.0;
        match stepped {
            Ok(false) => Refined::More,
            Ok(true) => {
                let Some(Stepping { job: drawn_for, fonts, steps, spent }) = self.steps.take() else { return Refined::Done(None) };
                let (view, pix) = self.full.finish(*steps);
                self.full_ms = spent;
                // The image of the job it was started for: its document's colour profile, its
                // proof setting and its request time.
                let drawn = done(&drawn_for, view, pix, Work::Full, spent, false);
                // Drawn for this very request: shown, done. Otherwise kept, then started on like
                // any render: an edit, a pan or images that came meanwhile are patched in, or it
                // is drawn again in steps (never all at once here), this one shown till then.
                if same_request(&drawn_for, fonts, job) {
                    return Refined::Done(self.finished(job, Some(drawn)));
                }
                self.begin(job, t0, Some(drawn)).unwrap_or(Refined::More)
            }
            Err(_) => {
                self.steps = None;
                self.full.clear();
                Refined::Done(None)
            }
        }
    }

    /// Start `job` on the kept image: done when it is reused or patched, `None` when a full render
    /// is now drawn in steps. `drawn` is the kept image just drawn in steps: what a reuse shows,
    /// else shown while it is drawn again.
    fn begin(&mut self, job: &Job, t0: Instant, mut drawn: Option<Done>) -> Option<Refined> {
        // The snapped view may be a row taller.
        let rows = job.view.height.saturating_add(1).div_ceil(MAX_BANDS).max(MIN_BAND_ROWS);
        let begun =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match self.full.begin(&job.doc, job.page, &job.view, &options(job), rows) {
                Some(Begun::Done(view, pix, work)) => Err(Box::new(match drawn.take() {
                    Some(d) if work == Work::Reused => Some(d),
                    _ => Some(done(job, view, pix, work, t0.elapsed().as_secs_f64() * 1000.0, false)),
                })),
                Some(Begun::Steps(steps)) => Ok(steps),
                None => Err(Box::new(None)),
            }));
        match begun {
            Ok(Ok(steps)) => {
                // A pan or an edit at this zoom builds on this render: no preview for it.
                self.last = Some((job.uid, job.page, job.view.scale));
                self.steps = Some(Stepping { job: job.clone(), fonts: figcraft_text::layout_generation(), steps, spent: 0.0 });
                drawn.map(Refined::Shown)
            }
            Ok(Err(d)) => Some(Refined::Done(self.finished(job, *d))),
            Err(_) => {
                self.full.clear();
                Some(Refined::Done(None))
            }
        }
    }

    /// Record a render done at once in [`Self::refine_for`].
    fn finished(&mut self, job: &Job, d: Option<Done>) -> Option<Done> {
        if let Some(d) = &d {
            self.last = Some((job.uid, job.page, job.view.scale));
            if d.work == Work::Full {
                self.full_ms = d.ms;
            }
        }
        d
    }

    /// Whether a full render is being drawn band by band.
    pub fn stepping(&self) -> bool {
        self.steps.is_some()
    }
}

/// The coarse preview of the whole of `job`'s view, kept in `retained`.
pub fn preview_with(job: &Job, retained: &mut Retained) -> Option<Done> {
    let view = coarse_view(&job.view);
    guarded(retained, |r| run(job, &view, r, true))
}

/// `d` numbered as the answer to request `seq`.
fn answering(d: Done, seq: u64) -> Done {
    Done { seq, ..d }
}

/// Of two finished renders, the one to show: the newer request's, and of one request's the refined
/// image (the watchdog's preview can arrive after it).
pub fn better(a: Option<Done>, b: Done) -> Done {
    match a {
        Some(a) if a.seq > b.seq || (a.seq == b.seq && !a.coarse && b.coarse) => a,
        _ => b,
    }
}

/// Run `f` on `retained`; after a panic, forget the image it may have left half drawn.
fn guarded(retained: &mut Retained, f: impl FnOnce(&mut Retained) -> Option<Done>) -> Option<Done> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(retained))) {
        Ok(d) => d,
        Err(_) => {
            retained.clear();
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Worker {
    tx: std::sync::mpsc::Sender<(u64, Job)>,
    rx: std::sync::mpsc::Receiver<Done>,
    /// Jobs submitted so far.
    submitted: u64,
    /// The newest job the thread is done with: rendered (or skipped for a newer one) and the UI
    /// woken for it.
    finished: Arc<std::sync::atomic::AtomicU64>,
    /// The watchdog is drawing a late preview, or has one to hand over and wake the UI for.
    previewing: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Worker {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, jobs) = std::sync::mpsc::channel::<(u64, Job)>();
        let (done_tx, rx) = std::sync::mpsc::channel::<Done>();
        let finished = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let done_with = finished.clone();
        // The window that renders with it (each window has its own worker).
        let viewport = ctx.viewport_id();
        // The request being rendered from scratch without a preview, and when it started, for the
        // watchdog; the newest request refined so far.
        let (watch, watched) = std::sync::mpsc::channel::<(u64, Job, std::time::Instant)>();
        let refined = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let refined_seen = refined.clone();
        let previewing = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let busy = previewing.clone();
        let (preview_tx, preview_ctx) = (done_tx.clone(), ctx.clone());
        let watchdog =
            std::thread::Builder::new().name("figcraft-render-preview".into()).stack_size(figcraft_render::RENDER_STACK).spawn(move || {
                let mut retained = Retained::default();
                let after = std::time::Duration::from_secs_f64(PREVIEW_AFTER_MS / 1000.0);
                while let Ok((mut seq, mut job, mut started)) = watched.recv() {
                    // Wait out the threshold; a newer arming means that render is over.
                    loop {
                        match watched.recv_timeout(after.saturating_sub(started.elapsed())) {
                            Ok(newer) => (seq, job, started) = newer,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    // Busy from before it looks at the render until its preview is handed over and the
                    // UI woken, so `in_flight` holds until then: once the full render is done (stored
                    // before the render thread's `finished`), this sees it and hands over nothing.
                    busy.store(true, std::sync::atomic::Ordering::SeqCst);
                    let still_rendering = || refined_seen.load(std::sync::atomic::Ordering::SeqCst) < seq;
                    let mut sent = true;
                    if still_rendering()
                        && let Some(d) = preview_with(&job, &mut retained)
                        && still_rendering()
                    {
                        sent = preview_tx.send(answering(d, seq)).is_ok();
                        preview_ctx.request_repaint_of(viewport);
                    }
                    busy.store(false, std::sync::atomic::Ordering::SeqCst);
                    if !sent {
                        break;
                    }
                }
            });
        if let Err(e) = watchdog {
            log::error!("render preview thread: {e}");
        }
        let spawned = std::thread::Builder::new().name("figcraft-render".into()).stack_size(figcraft_render::RENDER_STACK).spawn(move || {
            let mut renders = Renders::default();
            let mut next = None;
            loop {
                let Some((mut seq, mut job)) = next.take().or_else(|| jobs.recv().ok()) else { break };
                // Skip to the newest queued job.
                while let Ok(newer) = jobs.try_recv() {
                    (seq, job) = newer;
                }
                if let Some(d) = renders.preview(&job) {
                    if done_tx.send(answering(d, seq)).is_err() {
                        break;
                    }
                    ctx.request_repaint_of(viewport);
                    // A newer request makes refining this one moot: it gets its own preview.
                    if let Ok(newer) = jobs.try_recv() {
                        next = Some(newer);
                        continue;
                    }
                }
                if renders.wants_late_preview(&job) {
                    // Without the watchdog (it failed to start) there is just no preview.
                    let _ = watch.send((seq, job.clone(), std::time::Instant::now()));
                }
                let done = renders.refine(&job);
                refined.store(seq, std::sync::atomic::Ordering::SeqCst);
                if let Some(d) = done
                    && done_tx.send(answering(d, seq)).is_err()
                {
                    break;
                }
                ctx.request_repaint_of(viewport);
                done_with.store(seq, std::sync::atomic::Ordering::SeqCst);
            }
        });
        if let Err(e) = spawned {
            log::error!("render thread: {e}");
        }
        Self { tx, rx, submitted: 0, finished, previewing }
    }

    pub fn submit(&mut self, job: Job) {
        if self.tx.send((self.submitted + 1, job)).is_ok() {
            self.submitted += 1;
        }
    }

    /// Whether a submitted job is still rendering or hasn't woken the UI yet, the watchdog's late
    /// preview included (a finished image may still wait for [`Self::poll`]).
    pub fn in_flight(&self) -> bool {
        self.finished.load(std::sync::atomic::Ordering::SeqCst) < self.submitted || self.previewing.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The best of the renders finished since the last poll ([`better`]).
    pub fn poll(&mut self) -> Option<Done> {
        let mut best = None;
        while let Ok(d) = self.rx.try_recv() {
            best = Some(better(best, d));
        }
        best
    }
}

/// The browser's worker: renders inline, one image a frame: a preview first when one is due, the
/// refined image once no newer request came for [`SETTLE_MS`] (a zoom has paused), right away
/// when no preview came first (a pan, an edit). Built everywhere so tests run it.
pub struct Inline {
    ctx: egui::Context,
    pending: Option<Job>,
    /// Requests submitted so far.
    submitted: u64,
    previewed: bool,
    /// Whether the pending job's preview was shown (its refinement then waits to settle).
    preview_shown: bool,
    /// When that preview was handed over: the wait counts from there, not from the request, since
    /// rendering the preview itself can take longer than the wait (the page is blocked meanwhile,
    /// so no newer request could have come).
    since: Instant,
    /// How long to wait after a preview, at most: `wait` is this capped by the last full render's
    /// time, so a page whose renders are quick is not held coarse for longer than one would take.
    settle: std::time::Duration,
    /// The wait for the pending job's preview.
    wait: std::time::Duration,
    renders: Renders,
}

#[cfg(target_arch = "wasm32")]
pub type Worker = Inline;

impl Inline {
    pub fn new(ctx: egui::Context) -> Self {
        Self::with_settle(ctx, std::time::Duration::from_millis(SETTLE_MS))
    }
    /// Refining a preview after `settle` without newer requests (tests: zero, the next frame).
    pub fn with_settle(ctx: egui::Context, settle: std::time::Duration) -> Self {
        Self {
            ctx,
            pending: None,
            submitted: 0,
            previewed: false,
            preview_shown: false,
            since: Instant::now(),
            settle,
            wait: settle,
            renders: Renders::default(),
        }
    }
    /// Previewing when renders from scratch take at least `ms` (tests: 0, whatever the machine).
    #[cfg(test)]
    pub fn with_threshold(mut self, ms: f64) -> Self {
        self.renders = Renders::with_threshold(ms);
        self
    }
    pub fn submit(&mut self, job: Job) {
        self.pending = Some(job);
        self.submitted += 1;
        self.previewed = false;
        self.preview_shown = false;
    }
    /// Whether a request is still to be refined (a preview's wait counts).
    pub fn in_flight(&self) -> bool {
        self.pending.is_some()
    }
    pub fn poll(&mut self) -> Option<Done> {
        self.poll_at(Instant::now())
    }
    /// [`Self::poll`] as of `now` (tests pass the time, so a busy machine can't stretch the wait).
    pub fn poll_at(&mut self, now: Instant) -> Option<Done> {
        if !self.previewed {
            self.previewed = true;
            if let Some(d) = self.pending.as_ref().and_then(|job| self.renders.preview(job)) {
                self.preview_shown = true;
                self.since = now;
                let full = std::time::Duration::try_from_secs_f64(self.renders.full_ms_so_far() / 1000.0);
                self.wait = full.map_or(self.settle, |full| full.min(self.settle));
                self.ctx.request_repaint_after(self.wait);
                return Some(answering(d, self.submitted));
            }
        }
        if self.pending.is_some() && self.preview_shown {
            let waited = now.saturating_duration_since(self.since);
            if waited < self.wait {
                self.ctx.request_repaint_after(self.wait - waited);
                return None;
            }
        }
        let job = self.pending.as_ref()?;
        match self.renders.refine_for(job, STEP_MS) {
            Refined::Done(d) => {
                self.pending = None;
                d.map(|d| answering(d, self.submitted))
            }
            Refined::More => {
                self.ctx.request_repaint();
                None
            }
            Refined::Shown(d) => {
                self.ctx.request_repaint();
                Some(answering(d, self.submitted))
            }
        }
    }
}

/// Render synchronously from scratch (tests, screenshots).
pub fn render_now(job: &Job) -> Option<Done> {
    run(job, &job.view, &mut Retained::default(), false)
}
