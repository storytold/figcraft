use super::scroll::{clamp_scroll, page_settings};
use super::{
    Device, DeviceColor, Document, NodeId, OverlayState, Point, Pos2, PresentScale, PresentView, Presenter, Rect, Scroller, Vec2,
    anchored_overlay_rect, pos2, vec2,
};

/// Page → screen mapping for one presented frame.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub page_min: (f64, f64),
    pub origin: Pos2,
    pub scale: f64,
}

impl Place {
    pub(super) fn centered(fb: figcraft_geom::Rect, center: Pos2, scale: f64) -> Self {
        let size = vec2((fb.width() * scale) as f32, (fb.height() * scale) as f32);
        Place { page_min: (fb.x0, fb.y0), origin: center - size / 2.0, scale }
    }
    pub fn pt(&self, x: f64, y: f64) -> Pos2 {
        pos2(self.origin.x + ((x - self.page_min.0) * self.scale) as f32, self.origin.y + ((y - self.page_min.1) * self.scale) as f32)
    }
    pub fn rect(&self, r: figcraft_geom::Rect) -> Rect {
        Rect::from_min_max(self.pt(r.x0, r.y0), self.pt(r.x1, r.y1))
    }
    /// The page point under screen point `q`.
    pub fn page(&self, q: Pos2) -> Point {
        Point::new(self.page_min.0 + f64::from(q.x - self.origin.x) / self.scale, self.page_min.1 + f64::from(q.y - self.origin.y) / self.scale)
    }
}

/// The hotspots of the screen `root` (its visible layers with interactions, bottom-most first)
/// placed on screen and clipped by `screen`.
pub(super) fn hotspots(doc: &Document, root: NodeId, place: &Place, screen: Rect) -> Vec<(NodeId, Rect)> {
    figcraft_engine::proto_run::hotspots(doc, root)
        .into_iter()
        .map(|(id, b)| (id, place.rect(b).intersect(screen)))
        .filter(|(_, r)| r.is_positive())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The player UI, shared by the full-window presenter and the preview pane.

pub(super) struct Layout {
    pub(super) disp: Document,
    pub(super) scale: f64,
    pub(super) place: Place,
    /// What shows the prototype: the device's screen, else the presented frame itself.
    pub(super) screen: Rect,
    /// What the presented frame may draw into: the device's screen, else the whole area (shadows
    /// and overflowing content show around a frame presented without a device).
    pub(super) clip: Rect,
    pub(super) device: Option<Device>,
    /// The device is turned to landscape.
    pub(super) landscape: bool,
    /// The colour of the device's body, when it is drawn around its screen.
    pub(super) color: Option<DeviceColor>,
    pub(super) overlays: Vec<(NodeId, Place, Rect)>,
    /// Overlays animating out (same order as `Presenter::closing`).
    pub(super) closing: Vec<(NodeId, Place, Rect)>,
}

impl Layout {
    /// The active layer (the top overlay, else the presented frame `current`), where it's placed
    /// and what it may draw into.
    pub(super) fn active(&self, current: NodeId) -> (NodeId, Place, Rect) {
        match self.overlays.last() {
            Some((id, pl, _)) => (*id, *pl, self.screen),
            None => (current, self.place, self.clip),
        }
    }
}

/// The width of a device's screen of `size` held upright (its short side when turned).
pub(super) fn upright_w(size: Vec2, landscape: bool) -> f32 {
    if landscape { size.y } else { size.x }
}

/// Bezel width around a device's screen of upright width `w`, and the screen's corner radius.
pub(super) fn bezel(device: &str, w: f32) -> (f32, f32) {
    match device {
        "MacBook Air" => ((w * 0.025).max(6.0), 0.0),
        "Desktop" => ((w * 0.02).max(6.0), 0.0),
        d if d.starts_with("iPad") => ((w * 0.05).max(8.0), w * 0.04),
        _ => ((w * 0.06).max(6.0), w * 0.13),
    }
}

/// How far a device's body reaches past its screen of `size`: (each side, top, bottom). A phone
/// turned to landscape has its side buttons above and below.
fn body_outset(device: &str, size: Vec2, landscape: bool) -> (f32, f32, f32) {
    let w = upright_w(size, landscape);
    let (b, _) = bezel(device, w);
    match device {
        "MacBook Air" => (b + w * 0.08, b, b + (w * 0.022).max(6.0)),
        "Desktop" => (b, b, b + w * 0.08 + 6.0),
        _ if landscape => (b, b + 3.0, b + 3.0),
        _ => (b + 3.0, b, b),
    }
}

/// Place the presented frame in `area`: on its device's screen (the whole device sized by the
/// view's scale option), else the frame itself. Fit and 100% keep the chrome's padding from the
/// edges; Fit never goes above 100%. What doesn't fit the window pans (`Presenter::pan`). The
/// preview pane always fits. A responsive prototype's frame is first resized to the device's
/// screen, or without a device to the whole area (shown at 100%).
pub(super) fn layout(p: &mut Presenter, area: Rect, chrome: Chrome) -> Option<Layout> {
    let settings = page_settings(p.doc.as_ref()?, p.current);
    let (device, landscape) = (settings.device(), settings.landscape());
    let fills = settings.responsive && device.is_none();
    let whole_area = (f64::from(area.width().round()), f64::from(area.height().round()));
    p.respond_to(settings.responsive.then(|| device.map_or(whole_area, |d| (d.width, d.height))));
    let disp = p.display()?;
    let fb = disp.world_bounds(p.current)?;
    let view = if chrome.pane { PresentView::default() } else { p.view };
    // Custom sizes are just a screen: no body.
    let body = device.filter(|d| view.device_frame && !d.colors.is_empty());
    let pad = if !fills && matches!(view.scale, PresentScale::Fit | PresentScale::Actual) { chrome.pad } else { 0.0 };
    let avail = area.shrink(pad);
    let (sw, sh) = device.map_or((fb.width(), fb.height()), |d| (d.width, d.height));
    // The screen at `scale`, the body's outsets around it and the whole size.
    let outer = |scale: f64| {
        let size = vec2((sw * scale) as f32, (sh * scale) as f32);
        let (x, top, bottom) = body.map_or((0.0, 0.0, 0.0), |d| body_outset(d.name, size, landscape));
        (size, x, top, vec2(size.x + 2.0 * x, size.y + top + bottom))
    };
    // The factor that makes something of size `o` fit, fill… the available space.
    let factor = |o: Vec2| {
        let (kx, ky) = (avail.width() / o.x.max(1.0), avail.height() / o.y.max(1.0));
        f64::from(match view.scale {
            PresentScale::Fit => kx.min(ky),
            PresentScale::FitWidth => kx,
            PresentScale::Fill => kx.max(ky),
            PresentScale::Actual => 1.0,
        })
    };
    let mut scale = if fills { 1.0 } else { factor(vec2(sw as f32, sh as f32)) };
    // The body doesn't scale exactly with the screen (its edges have a minimum): size it once more.
    if body.is_some() {
        scale *= factor(outer(scale).3);
    }
    let max = if view.scale == PresentScale::Fit { 1.0 } else { 16.0 };
    let scale = if scale.is_finite() { scale.clamp(0.01, max) } else { 0.01 };
    let (size, x, top, whole) = outer(scale);
    p.zoom = scale;
    // What's bigger than the window pans: starting centred across and at the top.
    let over = |d: f32| if d > 0.5 { d } else { 0.0 };
    let excess = vec2(over(whole.x - avail.width()), over(whole.y - avail.height()));
    p.pan_range = (f64::from(excess.x), f64::from(excess.y));
    // While a drag or fling moves the view it may stretch past the ends; otherwise it stays in range.
    let live = matches!(p.drag_scroll, Some((Scroller::View, _))) || p.fling.is_some_and(|f| f.target == Scroller::View);
    if !live {
        p.pan = p.pan.map(|v| clamp_scroll(v, p.pan_range));
    }
    let pan = p.offset(Scroller::View);
    let along = |excess: f32, min: f32, centre: f32, len: f32, pan: f64| if excess > 0.0 { min - pan as f32 } else { centre - len / 2.0 };
    let min = pos2(along(excess.x, avail.min.x, area.center().x, whole.x, pan.0), along(excess.y, avail.min.y, area.center().y, whole.y, pan.1));
    let screen = Rect::from_min_size(min + vec2(x, top), size);
    let place = Place::centered(fb, screen.center(), scale);
    let place_overlay = |o: &OverlayState| {
        let ob = disp.world_bounds(o.id)?;
        let size = vec2((ob.width() * scale) as f32, (ob.height() * scale) as f32);
        let offset = o.offset.map(|[x, y]| vec2((x * scale) as f32, (y * scale) as f32));
        let r = anchored_overlay_rect(o.position, o.anchor_to, screen, size, o.anchor.map(|a| place.rect(a)), offset);
        Some((o.id, Place { page_min: (ob.x0, ob.y0), origin: r.min, scale }, r))
    };
    let overlays = p.overlays.iter().filter_map(place_overlay).collect();
    let closing = p.closing.iter().filter_map(|(_, o)| place_overlay(o)).collect();
    let clip = if device.is_some() { screen } else { area };
    let color = body.and(settings.device_color());
    Some(Layout { disp, scale, place, screen, clip, device, landscape, color, overlays, closing })
}

/// Where the player sits: the whole window or the preview pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Chrome {
    pub(super) pane: bool,
    pub(super) bar_h: f32,
    /// Space kept around the device or frame.
    pub(super) pad: f32,
    /// Corner radius of the window around the player (the pane's).
    pub(super) round: u8,
    /// The player may take keys (the pane only while hovered, and never while the editor is
    /// typing: text on the canvas or in a field).
    pub(super) keys: bool,
}

pub(super) const FULL: Chrome = Chrome { pane: false, bar_h: 40.0, pad: 32.0, round: 0, keys: true };
