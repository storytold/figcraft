use super::canvas::presented_image_rect;
use egui::{Pos2, Rect, pos2, vec2};
use figcraft_render::View;

#[test]
fn native_density_rect_aligns_origin_and_preserves_pixel_extent() {
    for ppp in [1.0_f32, 1.25, 1.5, 2.0] {
        for zoom in [0.37_f64, 0.67, 1.0, 1.3, 2.0] {
            let view = View { origin: (-31.25, 18.75), scale: zoom * f64::from(ppp), width: 375, height: 240 };
            let tl = pos2(149.5, 91.25);
            let rect = presented_image_rect(tl, &view, zoom, ppp, false);
            assert!(((rect.min.x * ppp) - (rect.min.x * ppp).round()).abs() < 1e-4, "left edge: ppp={ppp}, zoom={zoom}, rect={rect:?}");
            assert!(((rect.min.y * ppp) - (rect.min.y * ppp).round()).abs() < 1e-4, "top edge: ppp={ppp}, zoom={zoom}, rect={rect:?}");
            assert!((rect.min.x - tl.x).abs() * ppp <= 0.5001, "x shift exceeds half a device pixel: ppp={ppp}, rect={rect:?}");
            assert!((rect.min.y - tl.y).abs() * ppp <= 0.5001, "y shift exceeds half a device pixel: ppp={ppp}, rect={rect:?}");
            assert!((rect.width() * ppp - view.width as f32).abs() < 1e-3, "width changed: ppp={ppp}, rect={rect:?}");
            assert!((rect.height() * ppp - view.height as f32).abs() < 1e-3, "height changed: ppp={ppp}, rect={rect:?}");
        }
    }
}

#[test]
fn stale_pixel_preview_and_invalid_density_keep_unsnapped_origin() {
    let tl = Pos2::new(149.5, 91.25);
    let view = View { origin: (0.0, 0.0), scale: 1.95, width: 300, height: 180 };
    for (zoom, ppp, pixel_preview) in [(1.2, 1.5, false), (1.3, 1.5, true)] {
        let rect = presented_image_rect(tl, &view, zoom, ppp, pixel_preview);
        let original =
            Rect::from_min_size(tl, vec2((f64::from(view.width) / view.scale * zoom) as f32, (f64::from(view.height) / view.scale * zoom) as f32));
        assert_eq!(rect, original);
    }
    for ppp in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(presented_image_rect(tl, &view, 1.3, ppp, false).min, tl);
    }
}
