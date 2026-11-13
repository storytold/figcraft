use super::{
    Action, Document, Easing, FrameTex, NodeId, NodeKind, OverlayAnchor, OverlayPosition, OverlayState, Presenter, Rect, ScrollAnim, Temp,
    Transition, Trigger, Waiting,
};

#[derive(Clone, Copy)]
struct ActionTiming {
    transition: Transition,
    duration: f64,
    easing: Easing,
    rect: Option<Rect>,
    now: f64,
    depth: usize,
}

impl Presenter {
    /// A mouse enter, leave, down or up on `node` (shown on `root`) met its interaction `k`: its
    /// actions run now, or after its delay while `root` is still what shows (again once only
    /// while it waits).
    pub(super) fn gesture(&mut self, node: NodeId, k: usize, rect: Option<Rect>, now: f64, root: NodeId) -> bool {
        let delay = self.doc.as_ref().and_then(|d| d.node(node)).and_then(|n| n.interactions.get(k)).map_or(0.0, |it| it.delay_secs());
        if delay <= 0.0 {
            return self.fire(node, k, rect, now);
        }
        if !self.waiting.iter().any(|w| (w.node, w.k) == (node, k)) {
            self.waiting.push(Waiting { node, k, rect, root, due: now + delay });
        }
        false
    }

    /// Run the delayed mouse triggers that are due (those whose screen or overlay went away are
    /// dropped). True when one changed what shows.
    pub(super) fn fire_waiting(&mut self, root: NodeId, now: f64) -> bool {
        let (due, later): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut self.waiting).into_iter().partition(|w| w.due <= now);
        self.waiting = later;
        self.waiting.retain(|w| w.root == root);
        let mut did = false;
        for w in due.into_iter().filter(|w| w.root == root) {
            // An earlier one may have left the screen: the rest are dropped with it. The gesture
            // is long over: what these open is not the viewer's doing (#1096).
            did = did || self.fire_from(w.node, w.k, w.rect, now, false);
        }
        did
    }

    /// Run the actions of interaction `k` on `node`. `rect` is the layer's screen rect.
    pub(crate) fn fire(&mut self, node: NodeId, k: usize, rect: Option<Rect>, now: f64) -> bool {
        self.fire_from(node, k, rect, now, true)
    }

    /// [`fire`](Self::fire); `live`: the viewer's input fired it just now (not a delay it waited
    /// out). The links it opens are the viewer's own only when live from a click, press, drag or
    /// key: a delay, the pointer passing over a layer and a timed mouse trigger fire without the
    /// viewer asking for anything ([`Presenter::urls`], #1096).
    fn fire_from(&mut self, node: NodeId, k: usize, rect: Option<Rect>, now: f64, live: bool) -> bool {
        let Some(it) = self.doc.as_ref().and_then(|d| d.node(node)).and_then(|n| n.interactions.get(k)).cloned() else { return false };
        let dur = it.easing.seconds(it.duration_ms);
        let temp = match it.trigger {
            Trigger::OnHover => Some(false),
            Trigger::OnPress => Some(true),
            _ => None,
        };
        let snap = if temp.is_some() && self.temp.is_none() { self.snapshot() } else { None };
        let mut did = false;
        let first_link = self.urls.len();
        for a in &it.actions {
            did |= self.perform(a, node, it.transition, dur, it.easing, rect, now);
        }
        let asked = live
            && matches!(
                it.trigger,
                Trigger::OnClick | Trigger::OnPress | Trigger::OnDrag | Trigger::MouseDown | Trigger::MouseUp | Trigger::KeyPress { .. }
            );
        for link in self.urls.iter_mut().skip(first_link) {
            link.1 = asked;
        }
        if did && let (Some(press), Some(snap), Some(rect)) = (temp, snap, rect) {
            self.temp = Some(Temp { rect, press, snap, transition: it.transition, dur, easing: it.easing });
        }
        did
    }

    /// Run the widget clicks the player took: each calls its plugin back on the player's copy (the
    /// prototype's own state of the widget), as on the canvas but without touching the file.
    /// Returns what went wrong, for a toast.
    pub fn click_widgets(&mut self, s: &figcraft_engine::Session) -> Option<String> {
        let mut error = None;
        for (id, target) in std::mem::take(&mut self.widget_clicks) {
            let Some(doc) = self.design.clone() else { break };
            match figcraft_engine::widgets::click_on(s, doc, id, &target) {
                Ok((d, _)) => {
                    self.design = Some(d);
                    self.relayout();
                }
                Err(e) => error = Some(e.to_string()),
            }
        }
        error
    }

    pub(crate) fn perform(&mut self, a: &Action, node: NodeId, tr: Transition, dur: f64, easing: Easing, rect: Option<Rect>, now: f64) -> bool {
        self.perform_at(a, node, ActionTiming { transition: tr, duration: dur, easing, rect, now, depth: 0 })
    }

    /// One action; `depth` counts the conditional actions it is inside.
    fn perform_at(&mut self, a: &Action, node: NodeId, timing: ActionTiming) -> bool {
        let ActionTiming { transition: tr, duration: dur, easing, rect, now, depth } = timing;
        let Some(doc) = self.doc.clone() else { return false };
        match a {
            Action::Navigate { dest, preserve_scroll } => {
                let Some(top) = figcraft_engine::proto_run::navigate_target(&doc, *dest, &self.sections) else { return false };
                if !doc.contains(top) || (top == self.current && self.overlays.is_empty()) {
                    return false;
                }
                self.go(top, tr, dur, easing, now, true, *preserve_scroll);
                true
            }
            Action::Back => {
                if self.close_overlay(now) {
                    return true;
                }
                match self.history.pop() {
                    Some(v) => {
                        self.go(v.frame, v.transition.reversed(), v.dur, v.easing, now, false, false);
                        true
                    }
                    None => false,
                }
            }
            Action::ScrollTo { dest, offset } => {
                let frames = self.scroll_to_layer(&doc, *dest, *offset);
                if frames.is_empty() {
                    return false;
                }
                if matches!(tr, Transition::Instant) || dur <= 0.0 {
                    self.scroll.extend(frames.iter().map(|(f, _, to)| (*f, *to)));
                    self.rev += 1;
                } else {
                    self.scroll_anim = Some(ScrollAnim { frames, start: now, dur, easing });
                }
                true
            }
            Action::OpenOverlay { dest, position, close_on_outside, background, offset, anchor } => {
                let at = self.page_rect(rect);
                if !doc.contains(*dest) {
                    return false;
                }
                self.delays_fired.retain(|(id, _)| !doc.is_ancestor(*dest, *id));
                self.overlays.push(OverlayState {
                    id: *dest,
                    position: *position,
                    close_on_outside: *close_on_outside,
                    background: *background,
                    anchor: at,
                    offset: *offset,
                    anchor_to: *anchor,
                    opened: now,
                    transition: tr,
                    dur,
                    easing,
                });
                self.hovered.clear();
                true
            }
            Action::SwapOverlay { dest } => {
                let at = self.page_rect(rect);
                if !doc.contains(*dest) {
                    return false;
                }
                self.delays_fired.retain(|(id, _)| !doc.is_ancestor(*dest, *id));
                match self.overlays.last_mut() {
                    Some(o) => {
                        o.id = *dest;
                        o.opened = now;
                        o.transition = tr;
                        o.dur = dur;
                        o.easing = easing;
                    }
                    None => self.overlays.push(OverlayState {
                        id: *dest,
                        position: OverlayPosition::Center,
                        close_on_outside: true,
                        background: None,
                        anchor: at,
                        offset: None,
                        anchor_to: OverlayAnchor::Screen,
                        opened: now,
                        transition: tr,
                        dur,
                        easing,
                    }),
                }
                self.hovered.clear();
                true
            }
            Action::CloseOverlay => self.close_overlay(now),
            Action::OpenUrl { url, new_tab } => {
                // The URL comes from the document and this action fires from whatever trigger the
                // prototype sets, including a delay, so no click is implied: only a web or mail
                // link is kept, as not the viewer's doing until `fire_from` says otherwise.
                if figcraft_engine::doc_urls::openable(url) {
                    self.urls.push((url.clone(), false, *new_tab));
                }
                true
            }
            Action::ChangeTo { dest } => {
                let Some(inst) = figcraft_engine::proto_run::change_to_target(&doc, node, *dest) else { return false };
                if !doc.contains(*dest) {
                    return false;
                }
                self.begin(tr, dur, easing, now);
                let Some(d) = self.design.as_mut() else { return false };
                let before = d.clone();
                if let Some(n) = d.node_mut(inst)
                    && let NodeKind::Instance { component, .. } = &mut n.kind
                {
                    *component = *dest;
                }
                figcraft_engine::components::sync_instance(d, inst);
                self.settle(&before);
                true
            }
            Action::SetVariable { variable, value, expr } => {
                let Some(d) = self.design.as_mut() else { return false };
                let before = d.clone();
                let set = figcraft_engine::proto_run::set_variable(d, node, variable, value, expr.as_ref());
                if set {
                    self.settle(&before);
                }
                set
            }
            Action::SetVariableMode { collection, mode } => {
                let Some(d) = self.design.as_mut() else { return false };
                let before = d.clone();
                let set = figcraft_engine::proto_run::set_mode_by_id(d, self.current, collection, mode);
                if set {
                    self.settle(&before);
                }
                set
            }
            Action::Conditional { blocks } => {
                if depth >= figcraft_doc::proto::MAX_ACTION_DEPTH {
                    return false;
                }
                // Evaluated now: earlier actions of the interaction may have changed variables.
                let mut did = false;
                for inner in figcraft_engine::proto_run::branch(&doc, node, blocks) {
                    did |= self.perform_at(&inner, node, ActionTiming { depth: depth + 1, ..timing });
                }
                did
            }
        }
    }

    /// Seconds into the frame's animated GIFs, and the frame of them showing (see [`Self::texture`]).
    fn frame_clock(&self, ctx: &egui::Context, doc: &Document, id: NodeId, now: f64) -> (f64, Option<(u64, f64)>) {
        // Animated GIFs play from when the frame arrived: a new texture when one of them moves on,
        // and a wake-up for that moment (nothing else draws frames while it shows). egui wakes a
        // display frame before the time asked for, a little earlier still at times: ask that much
        // later, and count a frame due within a few milliseconds as due.
        let t = (now - self.arrived).max(0.0) + 0.005;
        let clock = figcraft_render::images::animation_clock(doc, id, t);
        if let Some((_, next)) = clock {
            crate::repaint_in(ctx, next + f64::from(ctx.input(|i| i.predicted_dt)));
        }
        (t, clock)
    }

    /// The part of the frame `id` (its whole texture covering `whole`, page units) in view
    /// (`visible`) at the full pixel scale `px`, for a frame whose whole texture is capped below it
    /// (see [`detail_plan`]). While the view moves (`moving`) it only returns a detail already
    /// rendered, the capped texture showing under it.
    pub(super) fn detail_texture(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        id: NodeId,
        px: f64,
        whole: figcraft_geom::Rect,
        visible: figcraft_geom::Rect,
        moving: bool,
        now: f64,
    ) -> Option<FrameTex> {
        // Converted back from the screen, the view can stand a rounding error outside the frame,
        // which a detail (always inside the frame) never covers: a still view would then render a
        // new detail every frame.
        let visible = visible.intersect(whole);
        let (t, clock) = self.frame_clock(ctx, doc, id, now);
        let key = (self.rev, px.to_bits(), clock.map_or(0, |c| c.0));
        let covers = |b: &figcraft_geom::Rect| b.x0 <= visible.x0 && b.y0 <= visible.y0 && b.x1 >= visible.x1 && b.y1 >= visible.y1;
        if let Some((did, d)) = &self.detail
            && *did == id
            && d.key == key
            && (moving || covers(&d.bounds))
        {
            return Some(d.clone());
        }
        if moving {
            return None;
        }
        let side = ctx.input(|i| i.max_texture_side) as f64 - 2.0;
        let rect = detail_plan(whole, visible, px, side)?;
        let pix = figcraft_render::render_area_at(doc, &[id], rect, texture_scale_for(rect, px, side), t)?;
        let img = crate::render_worker::screen_image(&pix, doc.color_profile);
        let tex = ctx.load_texture(format!("present-detail-{}", id.0), img, egui::TextureOptions::LINEAR);
        let ft = FrameTex { key, tex, bounds: rect };
        self.detail = Some((id, ft.clone()));
        Some(ft)
    }

    pub(super) fn texture(&mut self, ctx: &egui::Context, doc: &Document, id: NodeId, px_scale: f64, now: f64) -> Option<FrameTex> {
        let (t, clock) = self.frame_clock(ctx, doc, id, now);
        let key = (self.rev, px_scale.to_bits(), clock.map_or(0, |c| c.0));
        if let Some(t) = self.textures.get(&id)
            && t.key == key
        {
            return Some(t.clone());
        }
        let bounds = figcraft_render::export_bounds(doc, id)?;
        let pix = figcraft_render::render_node_at(doc, id, texture_scale(ctx, bounds, px_scale), t)?;
        let img = crate::render_worker::screen_image(&pix, doc.color_profile);
        let tex = ctx.load_texture(format!("present-{}", id.0), img, egui::TextureOptions::LINEAR);
        let ft = FrameTex { key, tex, bounds };
        self.textures.insert(id, ft.clone());
        Some(ft)
    }
}

/// The most pixels a presented frame's texture holds (bigger prototypes, at 100% or filling the
/// window, are drawn from a smaller texture scaled up).
const MAX_TEXTURE_PIXELS: f64 = 16_777_216.0;

/// The pixel scale, at most `px`, at which `bounds` makes a texture the GPU takes (no side past
/// its limit) of at most [`MAX_TEXTURE_PIXELS`].
pub(super) fn texture_scale(ctx: &egui::Context, bounds: figcraft_geom::Rect, px: f64) -> f64 {
    texture_scale_for(bounds, px, ctx.input(|i| i.max_texture_side) as f64 - 2.0)
}

/// [`texture_scale`] for a GPU taking textures up to `side` pixels a side.
pub(super) fn texture_scale_for(bounds: figcraft_geom::Rect, px: f64, side: f64) -> f64 {
    let (w, h) = (bounds.width().max(1.0), bounds.height().max(1.0));
    let s = px.min(side / w.max(h)).min((MAX_TEXTURE_PIXELS / (w * h)).sqrt());
    if s.is_finite() && s > 0.0 { s } else { 1e-3 }
}

/// The page area to draw at the full pixel scale `px` over the whole frame's capped texture, when
/// that texture is below `px`: the part in `visible` with a margin (a quarter of its size each
/// way, shrunk until the area makes a texture the GPU takes), inside `bounds`. `None` when the
/// whole frame is already at `px`, or nothing of it is in view. When the view alone is past the
/// caps it is returned all the same, and drawn at the scale [`texture_scale_for`] gives it.
pub(super) fn detail_plan(bounds: figcraft_geom::Rect, visible: figcraft_geom::Rect, px: f64, side: f64) -> Option<figcraft_geom::Rect> {
    if texture_scale_for(bounds, px, side) >= px || !(px.is_finite() && px > 0.0) {
        return None;
    }
    let view = visible.intersect(bounds);
    if !(view.width() > 0.0 && view.height() > 0.0 && view.width().is_finite() && view.height().is_finite()) {
        return None;
    }
    let fits = |r: &figcraft_geom::Rect| r.width() * px <= side && r.height() * px <= side && r.area() * px * px <= MAX_TEXTURE_PIXELS;
    let mut margin = 0.25;
    for _ in 0..8 {
        let r = view.inflate(view.width() * margin, view.height() * margin).intersect(bounds);
        if fits(&r) {
            return Some(r);
        }
        margin /= 2.0;
    }
    Some(view)
}

/// The first interaction index on `id` whose trigger satisfies `want`.
pub(super) fn find_trigger(doc: &Document, id: NodeId, want: impl Fn(&Trigger) -> bool) -> Option<usize> {
    doc.node(id)?.interactions.iter().position(|it| want(&it.trigger))
}
