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

#[cfg(test)]
mod tests {
    use egui::{Context, Pos2, RawInput, pos2};
    use figcraft_engine::Session;

    use crate::layers_drag_tests::Ui;

    /// Whether [`super::pan`] alone, with the pointer at `pos`, asks for the next frame. Run in a
    /// context of its own, after a few settling passes, so nothing else in the app asks for one.
    fn asks(u: &mut Ui, pos: Pos2) -> bool {
        let ctx = Context::default();
        for _ in 0..3 {
            ctx.run_ui(RawInput::default(), |_| {}).textures_delta.clear();
        }
        let mut out = ctx.run_ui(RawInput::default(), |ui| {
            super::pan(&mut u.app, ui.ctx(), pos);
        });
        out.textures_delta.clear();
        out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay.is_zero())
    }

    #[test]
    fn in_the_zone_it_asks_for_the_next_frame_and_outside_it_does_not() {
        let mut u = Ui::new(Session::new());
        u.frame(vec![]);
        u.frame(vec![]);
        let r = u.app.canvas.rect;
        assert!(asks(&mut u, pos2(r.max.x - 2.0, r.center().y)), "2 px inside the right edge");
        assert!(asks(&mut u, pos2(r.min.x - 20.0, r.max.y + 20.0)), "past the bottom-left corner");
        assert!(!asks(&mut u, r.center()), "the middle of the canvas");
        assert!(!asks(&mut u, pos2(r.max.x - 40.0, r.center().y)), "40 px inside the edge");
    }
}
