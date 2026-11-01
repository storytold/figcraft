//! The interface size (Preferences ▸ Interface size, `pref.uiScale`): how the saved choice becomes
//! egui's zoom factor.
//!
//! egui draws at `pixels_per_point = zoom factor × the system's scale`. The size is relative to the
//! system's scale, so it is the zoom itself: 100 % is the system's own size on every screen (native
//! on a Retina Mac too), 125 % a quarter larger. A size above 100 % is held back while the window is
//! too small to lay the interface out at it; the saved choice never changes.

/// The smallest window, in points, the interface is laid out for: the main window's minimum size.
pub const FIT_POINTS: (f32, f32) = (800.0, 500.0);

/// The zoom factors egui's own settings window allows.
const ZOOM_RANGE: (f32, f32) = (0.1, 10.0);

/// What to give egui for a saved choice on this screen and window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// For `Context::set_zoom_factor`: the size drawn, relative to the system's scale.
    pub zoom: f32,
    /// Physical pixels per point once drawn (the zoom times the system's scale).
    pub drawn: f32,
    /// The window is too small for the size chosen, so a smaller one is drawn.
    pub limited: bool,
}

/// The plan for `choice` (relative to the system's scale), the system's scale `native` and the
/// window's size in physical pixels `window_px`. An unknown, zero or not finite value reads as no
/// information: a size of 1, a system scale of 1 and a window of any size.
pub fn plan(choice: f64, native: Option<f32>, window_px: Option<(f32, f32)>) -> Plan {
    let native = native.filter(|n| n.is_finite() && *n > 0.0).unwrap_or(1.0);
    let want = Some(choice as f32).filter(|c| c.is_finite() && *c > 0.0).unwrap_or(1.0);
    // The largest zoom at which the window still holds the smallest layout.
    let fit = window_px
        .filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0)
        .map(|(w, h)| (w / FIT_POINTS.0).min(h / FIT_POINTS.1) / native);
    // Never below 100 % (or the size asked, when that is smaller): a tiny window doesn't shrink the interface further.
    let zoom = match fit {
        Some(fit) if want > fit => fit.max(want.min(1.0)),
        _ => want,
    }
    .clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
    Plan { zoom, drawn: zoom * native, limited: zoom < want - 0.005 }
}

/// A size as the percentage Preferences shows (1.25 → 125).
pub fn percent(scale: f32) -> u32 {
    (scale * 100.0).round().clamp(0.0, 100_000.0) as u32
}

/// The sizes Preferences offers, relative to the system's scale (`pref.uiScale` takes up to 3).
pub const SIZES: [f64; 8] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

#[cfg(test)]
mod tests {
    use super::*;

    const BIG: Option<(f32, f32)> = Some((20000.0, 12000.0));

    #[test]
    fn a_size_is_relative_to_the_systems_scale_on_every_screen() {
        // Windows 100/125/150/175 %, Retina 2×, a bare X11 window manager's guessed 2×, a 3× screen.
        for native in [1.0, 1.25, 1.5, 1.75, 2.0, 3.0] {
            let p = plan(1.0, Some(native), BIG);
            assert_eq!((p.zoom, p.drawn, p.limited), (1.0, native, false), "100 % is the system's own size on {native}");
            for size in SIZES {
                let p = plan(size, Some(native), BIG);
                assert!((p.zoom - size as f32).abs() < 1e-5, "{size} on {native}: zoom {}", p.zoom);
                assert!((p.drawn - size as f32 * native).abs() < 1e-4, "{size} on {native}: drawn {}", p.drawn);
                assert!(!p.limited, "{size} on {native}");
            }
        }
    }

    #[test]
    fn a_missing_or_unusable_scale_reads_as_1_and_never_gives_egui_a_bad_zoom() {
        for native in [None, Some(0.0), Some(-2.0), Some(f32::NAN), Some(f32::INFINITY)] {
            for choice in [1.0, 1.5, f64::NAN, f64::INFINITY, -1.0, 0.0, 1e300] {
                let p = plan(choice, native, Some((f32::NAN, 0.0)));
                assert!(p.zoom.is_finite() && (ZOOM_RANGE.0..=ZOOM_RANGE.1).contains(&p.zoom), "{choice:?} {native:?}: {p:?}");
                assert!(p.drawn.is_finite() && p.drawn > 0.0, "{choice:?} {native:?}: {p:?}");
            }
        }
        // A choice that isn't a usable number is the system's own size.
        assert_eq!(plan(f64::NAN, Some(2.0), BIG), plan(1.0, Some(2.0), BIG));
    }

    #[test]
    fn a_small_window_holds_a_big_size_back_without_going_below_100() {
        // The minimum window on a Retina Mac: 800 × 500 points is 1600 × 1000 pixels, which holds 100 % only.
        let min = Some((1600.0, 1000.0));
        let p = plan(1.5, Some(2.0), min);
        assert_eq!((p.zoom, p.drawn, p.limited), (1.0, 2.0, true));
        assert!(!plan(1.0, Some(2.0), min).limited);
        // A larger window on a 1× screen: 1200 × 750 pixels hold 150 %.
        let p = plan(2.0, Some(1.0), Some((1200.0, 750.0)));
        assert_eq!((p.zoom, p.limited), (1.5, true));
        // A window smaller than the minimum (a tiling window manager): never below 100 %, and never
        // below a smaller size the person chose.
        assert_eq!(plan(2.0, Some(1.0), Some((400.0, 250.0))).zoom, 1.0);
        let p = plan(0.75, Some(1.0), Some((400.0, 250.0)));
        assert_eq!((p.zoom, p.limited), (0.75, false));
    }

    #[test]
    fn the_limit_does_not_depend_on_the_zoom_so_it_cannot_flicker() {
        // The window's size in pixels is the same whatever zoom is drawn now, so the plan for it is too.
        let px = Some((1700.0, 1100.0));
        assert_eq!(plan(2.5, Some(2.0), px), plan(2.5, Some(2.0), px));
    }

    #[test]
    fn percentages_round_like_the_menu_shows_them() {
        assert_eq!([percent(0.75), percent(0.9), percent(1.1), percent(1.25), percent(2.0), percent(1.7999)], [75, 90, 110, 125, 200, 180]);
        assert_eq!([percent(f32::NAN), percent(-1.0)], [0, 0]);
    }
}
