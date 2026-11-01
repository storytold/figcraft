use super::{Deserialize, Serialize};

/// The canvas view of a document: what's at the centre of the viewport and the zoom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    /// Page coordinate at the viewport centre.
    pub cx: f64,
    pub cy: f64,
    /// Zoom (1 = 100%).
    pub zoom: f64,
    /// Viewport size in logical pixels (set by the UI; used by zoom-to-fit).
    pub vw: f64,
    pub vh: f64,
    pub pixel_grid: bool,
    pub layout_grids: bool,
    pub rulers: bool,
    pub outlines: bool,
    /// Outline mode for the selected layers only (the rest draws normally).
    pub outlines_selection: bool,
    /// Outline mode also outlines hidden layers (View ▸ Outlines ▸ Include hidden layers).
    pub outline_hidden: bool,
    /// Outline mode also draws every layer's box (View ▸ Outlines ▸ Include object bounds).
    pub outline_bounds: bool,
    /// Outline the masks (they don't draw themselves).
    pub mask_outlines: bool,
    pub pixel_preview: bool,
    /// Pixel preview resolution: rendered pixels per page pixel (1 or 2).
    pub pixel_scale: u8,
    pub show_slices: bool,
    pub comments: bool,
    /// Others' cursors in a live session (View ▸ Multiplayer cursors).
    pub multiplayer_cursors: bool,
    pub frame_outlines: bool,
    /// Soft-proof sRGB: the canvas marks the colours of a Display P3 file that sRGB can't show.
    pub proof_srgb: bool,
    pub snap_geometry: bool,
    pub snap_objects: bool,
    pub snap_pixel: bool,
    pub ui_hidden: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            cx: 0.0,
            cy: 0.0,
            zoom: 1.0,
            vw: 1200.0,
            vh: 800.0,
            pixel_grid: true,
            layout_grids: true,
            rulers: false,
            outlines: false,
            outlines_selection: false,
            outline_hidden: false,
            outline_bounds: false,
            mask_outlines: false,
            pixel_preview: false,
            pixel_scale: 1,
            show_slices: false,
            comments: true,
            multiplayer_cursors: true,
            frame_outlines: false,
            proof_srgb: false,
            snap_geometry: true,
            snap_objects: true,
            snap_pixel: true,
            ui_hidden: false,
        }
    }
}

impl ViewState {
    pub const MIN_ZOOM: f64 = 0.02;
    pub const MAX_ZOOM: f64 = 256.0;
    /// Page coordinate at the viewport's top-left.
    pub fn origin(&self) -> (f64, f64) {
        let z = self.zoom.max(1e-9);
        (self.cx - self.vw / 2.0 / z, self.cy - self.vh / 2.0 / z)
    }
    pub fn set_zoom_at(&mut self, zoom: f64, anchor: (f64, f64)) {
        let z = if zoom.is_finite() { zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM) } else { self.zoom };
        let old = self.zoom.max(1e-9);
        // Keep the page point under `anchor` fixed.
        self.cx = anchor.0 + (self.cx - anchor.0) * old / z;
        self.cy = anchor.1 + (self.cy - anchor.1) * old / z;
        self.zoom = z;
    }
    /// Fit a page rectangle into the viewport with margin.
    pub fn fit(&mut self, r: figcraft_geom::Rect, max_zoom: f64) {
        if !(r.width() > 0.0 || r.height() > 0.0) || !r.x0.is_finite() {
            self.cx = r.center().x;
            self.cy = r.center().y;
            return;
        }
        let margin = 64.0;
        let zx = (self.vw - margin * 2.0).max(50.0) / r.width().max(1e-6);
        let zy = (self.vh - margin * 2.0).max(50.0) / r.height().max(1e-6);
        self.zoom = zx.min(zy).clamp(Self::MIN_ZOOM, max_zoom);
        self.cx = r.center().x;
        self.cy = r.center().y;
    }
    /// The zoom from which the pixel grid shows (400%).
    pub const PIXEL_GRID_ZOOM: f64 = 4.0;
    /// Whether the pixel grid draws: it's on and the canvas is zoomed in to 400% or more.
    pub fn pixel_grid_shown(&self) -> bool {
        self.pixel_grid && self.zoom >= Self::PIXEL_GRID_ZOOM - 1e-9
    }
    /// Pixel preview's resolution (rendered pixels per page pixel) when it's on and zoomed in past
    /// it; below that the canvas is already coarser than the preview.
    pub fn pixel_preview_scale(&self) -> Option<f64> {
        let k = f64::from(self.pixel_scale.clamp(1, 2));
        (self.pixel_preview && self.zoom > k).then_some(k)
    }
    /// A wheel or trackpad scroll of (`dx`, `dy`) screen points: the canvas pans with it (content
    /// follows the fingers); with Shift a plain vertical wheel pans sideways.
    pub fn scroll(&mut self, dx: f64, dy: f64, shift: bool) {
        let (dx, dy) = if shift && dx == 0.0 { (dy, 0.0) } else { (dx, dy) };
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let z = self.zoom.max(1e-6);
        self.cx -= dx / z;
        self.cy -= dy / z;
    }
    pub fn page_to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        let (ox, oy) = self.origin();
        ((x - ox) * self.zoom, (y - oy) * self.zoom)
    }
    pub fn screen_to_page(&self, x: f64, y: f64) -> (f64, f64) {
        let (ox, oy) = self.origin();
        let z = self.zoom.max(1e-9);
        (ox + x / z, oy + y / z)
    }
}
