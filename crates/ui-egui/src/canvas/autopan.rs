//! Auto-pan: a drag that holds the pointer near the canvas's edge, or past it, pans the view
//! toward it, steadily while the pointer stays still (#2015). The rule lives in
//! [`figcraft_engine::ViewState::autopan_velocity`]; this applies it once a frame.

use egui::{Context, Pos2};

use crate::FigApp;

/// The longest frame the pan counts, so a stall doesn't jump the view.
const MAX_DT: f64 = 0.1;

/// Pan the view for this frame with a drag's pointer at screen point `pos`. True when it panned,
/// so the caller feeds the engine the pointer again: the dragged layer, box, shape or connection
/// follows the document point now under it. Asks for another frame while the pointer is in the
/// zone, so the pan goes on when nothing else moves.
pub(crate) fn pan(app: &mut FigApp, ctx: &Context, pos: Pos2) -> bool {
    let dt = f64::from(ctx.input(|i| i.unstable_dt)).clamp(0.0, MAX_DT);
    let min = app.canvas.rect.min;
    let (px, py) = (f64::from(pos.x - min.x), f64::from(pos.y - min.y));
    let Ok(ds) = app.session.ds_mut() else { return false };
    let (vx, vy) = figcraft_engine::ViewState::autopan_velocity(px, py, ds.view.vw, ds.view.vh);
    if vx == 0.0 && vy == 0.0 {
        return false;
    }
    // In the zone: the next frame pans too, even if the pointer never moves.
    ctx.request_repaint();
    if !ds.view.autopan(px, py, dt) {
        return false;
    }
    ds.revision += 1;
    true
}

/// The drag under way pans at the edge: a primary drag of the Move or a drawing tool that
/// [`figcraft_engine::tools::ToolState::autopans`] allows, with no text being edited.
pub(super) fn wanted(app: &FigApp, ctx: &Context) -> bool {
    let Ok(ds) = app.session.ds() else { return false };
    app.canvas.dragging
        && ds.text_edit.is_none()
        && ds.vector_edit.is_none()
        && ctx.input(|i| i.pointer.primary_down())
        && app.session.tool.autopans(ds.view.zoom)
}
