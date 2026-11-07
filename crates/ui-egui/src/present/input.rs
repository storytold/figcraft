use super::actions::find_trigger;
use super::chrome::layer_at;
use super::layout::{Layout, Place, hotspots};
use super::navigate::next_delay;
use super::scroll::{COAST_MIN, DECAY_DRAG, DECAY_WHEEL, RUBBER_PTS, clamp_scroll, page_settings, rubber_band, scrollers_at, units_per_point};
use super::transition::{direction_vec, lerp};
use super::{Document, Easing, FLASH_SECS, Fling, KeyMods, NodeId, Pos2, Presenter, Rect, Scroller, Transition, Trigger, Vec2, vec2};

/// Keep a key the prototype used from reaching the editor's shortcuts.
fn consume_key(ctx: &egui::Context, key: egui::Key) {
    ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { key: k, .. } if *k == key)));
}

/// A transition a drag started follows the pointer while it stays down, then plays on from there.
fn follow_drag(p: &mut Presenter, pos: Option<egui::Pos2>, down: bool, now: f64) {
    if let Some(a) = p.anim.as_mut()
        && let Some((from, dir, len)) = a.follow
    {
        match pos.filter(|_| down) {
            Some(q) => a.start = now - f64::from(((q - from).dot(dir) / len).clamp(0.0, 1.0)) * a.dur,
            None => a.follow = None,
        }
    }
}

/// A drag that scrolled lets go: it carries on at the pointer's speed (and springs back from
/// past the ends).
fn release_drag_scroll(p: &mut Presenter, l: &Layout, down: bool, velocity: Vec2, now: f64) {
    if !down && let Some((t, _)) = p.drag_scroll.take() {
        let u = units_per_point(t, l.scale);
        let r = p.doc.as_ref().map_or((0.0, 0.0), |b| p.limits(b, t));
        let along = |v: f32, r: f64| if r > 0.0 { -f64::from(v) * u } else { 0.0 };
        p.fling = Some(Fling { target: t, vel: (along(velocity.x, r.0), along(velocity.y, r.1)), decay: DECAY_DRAG, bounce: true, at: now });
    }
}

/// Scrolls the first of `targets` the wheel can still move, and smooths the wheel's speed over
/// ~50 ms whatever the frame rate.
fn wheel_scroll(p: &mut Presenter, l: &Layout, base: &Document, targets: Vec<Scroller>, wheel: Vec2, dt: f64) {
    let mut moved = None;
    for t in targets {
        let u = units_per_point(t, l.scale);
        let cur = p.offset(t);
        let next = clamp_scroll((cur.0 - f64::from(wheel.x) * u, cur.1 - f64::from(wheel.y) * u), p.limits(base, t));
        if (next.0 - cur.0).abs() > 1e-6 || (next.1 - cur.1).abs() > 1e-6 {
            p.set_offset(t, next);
            p.scroll_anim = None;
            p.fling = None;
            moved = Some((t, ((next.0 - cur.0) / dt, (next.1 - cur.1) / dt)));
            break;
        }
    }
    // The wheel's speed, smoothed over ~50 ms whatever the frame rate.
    p.wheel = moved.map(|(t, inst)| {
        let prev = p.wheel.filter(|w| w.0 == t).map_or(inst, |w| w.1);
        let a = 1.0 - (-dt / 0.05).exp();
        (t, (lerp(prev.0, inst.0, a), lerp(prev.1, inst.1, a)))
    });
}

/// A drag begun inside a mobile screen moves its content, even if the pointer leaves the
/// screen. The device body stays in place even when the drag begins outside its screen.
/// Panning the whole view remains available only without a mobile device.
fn begin_drag_scroll(p: &mut Presenter, doc: &Document, root: NodeId, place: &Place, start: Pos2, d: Vec2) {
    if let Some(base) = p.doc.clone() {
        let horizontal = d.x.abs() >= d.y.abs();
        let scrolls = scrollers_at(p, &base, doc, root, place, start);
        let on_axis = |range: (f64, f64)| if horizontal { range.0 > 0.0 } else { range.1 > 0.0 };
        let mobile = page_settings(&base, root).device().is_some_and(|device| device.rotates);
        let target = scrolls
            .iter()
            .copied()
            .find(|f| on_axis(p.range(&base, *f)))
            .or_else(|| scrolls.first().copied())
            .map(Scroller::Frame)
            .or((!mobile && p.pan_range != (0.0, 0.0)).then_some(Scroller::View));
        if let Some(t) = target {
            p.fling = None;
            p.drag_scroll = Some((t, clamp_scroll(p.offset(t), p.limits(&base, t))));
        }
    }
}

/// Dragging scrolls with the pointer; past the ends it stretches.
fn drag_scrolls(p: &mut Presenter, ctx: &egui::Context, l: &Layout, down: bool, pos: Option<Pos2>) {
    if down
        && let (Some((t, from)), Some(start), Some(q)) = (p.drag_scroll, p.press, pos)
        && let Some(base) = p.doc.as_ref()
    {
        let (d, u) = (q - start, units_per_point(t, l.scale));
        let r = p.limits(base, t);
        let limit = RUBBER_PTS * u;
        let to = (rubber_band(from.0 - f64::from(d.x) * u, r.0, limit), rubber_band(from.1 - f64::from(d.y) * u, r.1, limit));
        p.set_offset(t, to);
        p.scroll_anim = None;
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    }
}

/// The player's own keys, when the prototype doesn't use them: R restarts, Z cycles the
/// full-window presenter's size options. True when `key` was used.
fn player_key(p: &mut Presenter, ctx: &egui::Context, l: &Layout, key: &egui::Key, mods: &egui::Modifiers, pane: bool, now: f64) -> bool {
    if mods.is_none() {
        match key {
            egui::Key::R => {
                p.restart(p.start, now);
                consume_key(ctx, *key);
                return true;
            }
            // (A responsive frame filling the window has no size options.)
            egui::Key::Z if !pane && (l.device.is_some() || p.fit.is_none()) => {
                p.set_scale(p.view.scale.next(l.device.is_some()));
                consume_key(ctx, *key);
                return true;
            }
            _ => {}
        }
    }
    false
}

/// The arrows (and Space or N forward) step through top-level frames when nothing else uses
/// them. True when `key` stepped.
fn step_frames(p: &mut Presenter, ctx: &egui::Context, key: &egui::Key, mods: &egui::Modifiers, now: f64) -> bool {
    let forward = match key {
        egui::Key::ArrowRight => Some(true),
        egui::Key::Space | egui::Key::N if mods.is_none() => Some(true),
        egui::Key::ArrowLeft => Some(false),
        _ => None,
    };
    if let Some(forward) = forward
        && p.overlays.is_empty()
        && let Some(base) = p.doc.as_ref()
    {
        let frames = base.top_frames(base.page_of(p.current).unwrap_or(p.current));
        if let Some(i) = frames.iter().position(|f| *f == p.current) {
            let next = if forward { frames.get(i + 1) } else { i.checked_sub(1).and_then(|j| frames.get(j)) };
            if let Some(n) = next.copied() {
                p.go(n, Transition::Instant, 0.0, Easing::Linear, now, true, false);
                consume_key(ctx, *key);
                return true;
            }
        }
    }
    false
}

/// Controller buttons: pressed alone, anywhere on the screen, like keys. True when one fired.
fn fire_pads(p: &mut Presenter, doc: &Document, root: NodeId, pads: &[String], now: f64) -> bool {
    for name in pads {
        let found = doc.subtree(root).into_iter().find_map(|id| find_trigger(doc, id, |t| t.fires_on_key(name, KeyMods::default())).map(|k| (id, k)));
        if let Some((id, k)) = found
            && p.fire(id, k, None, now)
        {
            return true;
        }
    }
    false
}

/// Pointer, wheel, keys and timers. `over_area`: the pointer is over the player; `keys_on`: the
/// player takes the keyboard; `pane`: it plays in the preview pane (which always fits).
pub(super) struct InputMode {
    pub over_area: bool,
    pub keys_on: bool,
    pub pane: bool,
    pub now: f64,
}

pub(super) fn input(p: &mut Presenter, ctx: &egui::Context, l: &Layout, mode: InputMode) {
    let InputMode { over_area, keys_on, pane, now } = mode;
    let (pos, pressed, released, down, wheel, velocity, dt, keys) = ctx.input(|i| {
        let keys: Vec<(egui::Key, egui::Modifiers)> = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect();
        (
            i.pointer.hover_pos(),
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.button_released(egui::PointerButton::Primary),
            i.pointer.primary_down(),
            i.smooth_scroll_delta,
            i.pointer.velocity(),
            f64::from(i.unstable_dt).clamp(1.0 / 500.0, 0.1),
            keys,
        )
    });
    let keys = if keys_on { keys } else { Vec::new() };
    // Controller buttons pressed while a link notice shows are dropped, as keys are.
    let pads = std::mem::take(&mut p.pads);
    let pads = if p.link_notice.is_none() { pads } else { Vec::new() };
    follow_drag(p, pos, down, now);
    let pos = pos.filter(|_| over_area || p.press.is_some());
    release_drag_scroll(p, l, down, velocity, now);
    // The active layer: the top overlay, else the frame.
    let (root, place, clip) = l.active(p.current);
    let doc = &l.disp;
    let hot = hotspots(doc, root, &place, clip);
    p.hot.clone_from(&hot);
    if p.fire_waiting(root, now) {
        return;
    }
    // "While hovering / pressing" reverts.
    if let Some(tmp) = &p.temp {
        let leave = if tmp.press { released || !down } else { pos.is_none_or(|q| !tmp.rect.contains(q)) };
        if leave {
            p.revert(now);
            return;
        }
    }
    // The wheel scrolls the innermost frame under the pointer that can still move that way, else
    // pans a view bigger than the window. When it stops it coasts on, slowing down.
    if wheel != Vec2::ZERO
        && let Some(q) = pos
        && let Some(base) = p.doc.clone()
    {
        let mut targets: Vec<Scroller> = scrollers_at(p, &base, doc, root, &place, q).into_iter().map(Scroller::Frame).collect();
        if p.pan_range != (0.0, 0.0) {
            targets.push(Scroller::View);
        }
        wheel_scroll(p, l, &base, targets, wheel, dt);
    } else if wheel == Vec2::ZERO
        && let Some((t, v)) = p.wheel.take()
        && v.0.hypot(v.1) > COAST_MIN * units_per_point(t, l.scale)
    {
        p.fling = Some(Fling { target: t, vel: v, decay: DECAY_WHEEL, bounce: false, at: now });
    }
    let hits: Vec<(NodeId, Rect)> = hot.iter().rev().filter(|(_, r)| pos.is_some_and(|q| r.contains(q))).copied().collect();
    if !hits.is_empty() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // First (top-most) hit that has a matching trigger.
    let pick = |want: &dyn Fn(&Trigger) -> bool| -> Option<(NodeId, usize, Rect)> {
        hits.iter().find_map(|(id, r)| find_trigger(doc, *id, want).map(|k| (*id, k, *r)))
    };
    // Enter / leave.
    let ids: Vec<NodeId> = hits.iter().map(|h| h.0).collect();
    let entered: Vec<(NodeId, Rect)> = hits.iter().filter(|(id, _)| !p.hovered.contains(id)).copied().collect();
    let left: Vec<NodeId> = p.hovered.iter().filter(|id| !ids.contains(id)).copied().collect();
    p.hovered = ids;
    for id in left {
        if let Some(k) = find_trigger(doc, id, |t| *t == Trigger::MouseLeave)
            && p.gesture(id, k, None, now, root)
        {
            return;
        }
    }
    for (id, r) in &entered {
        if let Some(k) = find_trigger(doc, *id, |t| *t == Trigger::MouseEnter)
            && p.gesture(*id, k, Some(*r), now, root)
        {
            return;
        }
    }
    if p.temp.is_none()
        && !entered.is_empty()
        && let Some((id, k, r)) = pick(&|t| *t == Trigger::OnHover)
        && entered.iter().any(|e| e.0 == id)
        && p.fire(id, k, Some(r), now)
    {
        return;
    }
    // Press / drag / release.
    if pressed && pos.is_some() && over_area {
        p.press = pos;
        p.dragged = false;
        // Pressing catches a fling (one springing back from past an end carries on).
        let overscrolled = |f: &Fling| p.doc.as_ref().is_some_and(|b| clamp_scroll(p.offset(f.target), p.limits(b, f.target)) != p.offset(f.target));
        p.fling = p.fling.filter(overscrolled);
        p.wheel = None;
        if let (Some((_, _, orect)), Some(o), Some(q)) = (l.overlays.last(), p.overlays.last(), pos)
            && !orect.contains(q)
        {
            if o.close_on_outside {
                p.close_overlay(now);
                p.press = None;
                p.hovered.clear();
            }
            return;
        }
        if let Some((id, k, r)) = pick(&|t| *t == Trigger::MouseDown)
            && p.gesture(id, k, Some(r), now, root)
        {
            return;
        }
        if let Some((id, k, r)) = pick(&|t| *t == Trigger::OnPress)
            && p.fire(id, k, Some(r), now)
        {
            return;
        }
    }
    if down
        && !p.dragged
        && let (Some(start), Some(q)) = (p.press, pos)
        && start.distance(q) > 6.0
    {
        p.dragged = true;
        let at_start: Vec<(NodeId, Rect)> = hot.iter().rev().filter(|(_, r)| r.contains(start)).copied().collect();
        // A directional transition starts only on a drag its way (else the drag may scroll).
        let d = q - start;
        let drags =
            |id: NodeId| doc.node(id)?.interactions.iter().position(|it| it.trigger == Trigger::OnDrag && it.transition.accepts_drag(d.x, d.y));
        if let Some((id, k, r)) = at_start.iter().find_map(|(id, r)| drags(*id).map(|k| (*id, k, *r)))
            && p.fire(id, k, Some(r), now)
        {
            p.press = None;
            // The transition it started follows the pointer: along its direction, else the drag's
            // main axis, completing over the screen's size that way.
            if let Some(a) = p.anim.as_mut().filter(|a| a.start == now) {
                let (x, y) = a
                    .transition
                    .direction()
                    .map_or_else(|| if d.x.abs() >= d.y.abs() { (d.x.signum(), 0.0) } else { (0.0, d.y.signum()) }, direction_vec);
                let len = if x != 0.0 { l.screen.width() } else { l.screen.height() };
                a.follow = Some((start, vec2(x, y), len.max(1.0)));
            }
            return;
        }
        // A drag begun inside a mobile screen moves its content, even if the pointer leaves the
        // screen. The device body stays in place even when the drag begins outside its screen.
        // Panning the whole view remains available only without a mobile device.
        begin_drag_scroll(p, doc, root, &place, start, d);
    }
    // Dragging scrolls with the pointer; past the ends it stretches.
    drag_scrolls(p, ctx, l, down, pos);
    if released && p.press.is_some() {
        let dragged = p.dragged;
        p.press = None;
        p.dragged = false;
        p.drag_scroll = None;
        if let Some((id, k, r)) = pick(&|t| *t == Trigger::MouseUp)
            && p.gesture(id, k, Some(r), now, root)
        {
            return;
        }
        if !dragged {
            if let Some((id, k, r)) = pick(&|t| *t == Trigger::OnClick)
                && p.fire(id, k, Some(r), now)
            {
                return;
            }
            // A widget's clickable part calls its plugin back (after the frame, with the session).
            if let Some(q) = pos
                && let Some(t) = layer_at(doc, root, &place, q).and_then(|hit| figcraft_engine::widgets::target_at(doc, hit, place.page(q)))
            {
                p.widget_clicks.push(t);
                return;
            }
            // Clicking where nothing reacts flashes the hotspots.
            if p.view.hotspot_hints {
                p.flash_until = now + FLASH_SECS;
            }
        }
    }
    if fire_pads(p, doc, root, &pads, now) {
        return;
    }
    // Keys.
    for (key, mods) in &keys {
        let (name, held) = (key.name(), KeyMods { ctrl: mods.ctrl, alt: mods.alt, shift: mods.shift, cmd: mods.mac_cmd });
        let found = doc.subtree(root).into_iter().find_map(|id| find_trigger(doc, id, |t| t.fires_on_key(name, held)).map(|k| (id, k)));
        if let Some((id, k)) = found {
            if p.fire(id, k, None, now) {
                consume_key(ctx, *key);
                return;
            }
            continue;
        }
        if player_key(p, ctx, l, key, mods, pane, now) || step_frames(p, ctx, key, mods, now) {
            return;
        }
    }
    // After delay (timers start when the frame appears; not while animating).
    if p.anim.is_none()
        && let Some(((id, k), due)) = next_delay(p, doc, root, now)
        && due <= 0.0
    {
        p.delays_fired.insert((id, k));
        p.fire(id, k, None, now);
    }
}
