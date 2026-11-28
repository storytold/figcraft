//! Keeps the custom (undecorated) title bar windows truly borderless on Windows (#1343).
//!
//! winit 0.30 gives an undecorated window the `WS_CAPTION | WS_BORDER` styles and hides the
//! non-client area by spreading the client rect over it (`WM_NCCALCSIZE`). On some systems the
//! desktop compositor still reserves that frame for the window surface: the content is drawn about
//! 30 px low (a black band above it) while the pointer keeps the un-offset client coordinates, so
//! clicks land on something other than what is drawn under them.
//!
//! winit 0.27 omitted those styles for undecorated windows and had no offset. [`WindowFrame`]
//! restores that: it strips `WS_CAPTION` and `WS_BORDER` from every window of this process that
//! draws its own title bar, and has Windows recalculate the frame. winit puts the styles back
//! whenever it changes the window flags (maximize, restore, ...), so the step runs every frame; it
//! reads each style and writes only when it differs, so it costs next to nothing.
//!
//! The same fix is in DesignCraft (#2139) and PhotoCraft (#2401). The long-term fix is in winit
//! (storytold/winit#17); this module goes when the pinned winit has it.
//!
//! The Windows calls go through safe `winsafe` wrappers (the workspace forbids `unsafe`). Which
//! windows are ours is found by enumerating this process's top-level windows of winit's window
//! class, so the main window and every other window opened later are covered alike, and a file
//! dialog or message box of the same process is left alone.

/// `WS_BORDER`.
#[cfg(any(windows, test))]
const WS_BORDER: u32 = 0x0080_0000;
/// `WS_CAPTION`, which already includes `WS_BORDER`.
#[cfg(any(windows, test))]
const WS_CAPTION: u32 = 0x00C0_0000;

/// `style` without the OS title bar and frame (`WS_CAPTION`, `WS_BORDER`). The sizing border
/// (`WS_THICKFRAME`) and the window buttons stay, so edge resizing, Aero snap and the system menu
/// keep working, as they did with winit 0.27's undecorated windows.
#[cfg(any(windows, test))]
pub fn borderless_style(style: u32) -> u32 {
    style & !(WS_CAPTION | WS_BORDER)
}

/// The per-frame step that keeps the windows borderless; a no-op where the OS draws the frame
/// (macOS, Linux, the BSDs), where the app uses the system title bar, and in the browser.
pub struct WindowFrame {
    step: Option<Box<dyn FnMut()>>,
}

impl WindowFrame {
    /// The step for an app whose windows draw their own title bar when `custom_titlebar`.
    pub fn new(custom_titlebar: bool) -> Self {
        Self { step: platform::step(custom_titlebar) }
    }

    /// Run the step; call it once per frame, before the frame is laid out.
    pub fn run(&mut self) {
        if let Some(step) = self.step.as_mut() {
            step();
        }
    }

    #[cfg(test)]
    fn with_step(step: Box<dyn FnMut()>) -> Self {
        Self { step: Some(step) }
    }
}

#[cfg(windows)]
mod platform {
    use winsafe::{HWND, HwndPlace, POINT, SIZE, co};

    /// winit's window class (`WindowAttributes`' default `class_name`).
    const WINIT_CLASS: &str = "Window Class";

    /// A step that strips the frame styles from this process's windows each call, or `None` when
    /// the app uses the system title bar (the windows are decorated).
    pub fn step(custom_titlebar: bool) -> Option<Box<dyn FnMut()>> {
        if !custom_titlebar {
            return None;
        }
        let pid = std::process::id();
        Some(Box::new(move || {
            // The windows are handed over as real `HWND`s; nothing is wrapped from a raw handle.
            let _ = winsafe::EnumWindows(|hwnd| {
                strip_if_ours(&hwnd, pid);
                true
            });
        }))
    }

    /// Strip `hwnd` when it is one of this process's winit windows.
    fn strip_if_ours(hwnd: &HWND, pid: u32) {
        if hwnd.GetWindowThreadProcessId().1 != pid || !matches!(hwnd.GetClassName().as_deref(), Ok(WINIT_CLASS)) {
            return;
        }
        strip(hwnd);
    }

    /// Removes `WS_CAPTION`/`WS_BORDER` and has Windows recalculate the frame; a no-op once they
    /// are gone.
    fn strip(hwnd: &HWND) {
        let style = hwnd.style();
        // The decision is the tested arithmetic; the write uses the same bits as winsafe's flags.
        if super::borderless_style(style.raw()) == style.raw() {
            return;
        }
        hwnd.set_style(style & !(co::WS::CAPTION | co::WS::BORDER));
        let _ = hwnd.SetWindowPos(
            HwndPlace::None,
            POINT::new(),
            SIZE::new(),
            co::SWP::FRAMECHANGED | co::SWP::NOMOVE | co::SWP::NOSIZE | co::SWP::NOZORDER | co::SWP::NOACTIVATE,
        );
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn step(_custom_titlebar: bool) -> Option<Box<dyn FnMut()>> {
        None
    }
}

#[cfg(test)]
mod window_frame_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{WS_BORDER, WS_CAPTION, WindowFrame, borderless_style};

    #[cfg(windows)]
    #[test]
    fn the_style_bits_match_winsafes() {
        use winsafe::co;
        assert_eq!(co::WS::CAPTION.raw(), WS_CAPTION);
        assert_eq!(co::WS::BORDER.raw(), WS_BORDER);
    }

    const WS_SYSMENU: u32 = 0x0008_0000;
    const WS_THICKFRAME: u32 = 0x0004_0000;
    const WS_MINIMIZEBOX: u32 = 0x0002_0000;
    const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
    const WS_VISIBLE: u32 = 0x1000_0000;

    #[test]
    fn the_borderless_style_drops_the_caption_and_border_only() {
        let decorated = WS_CAPTION | WS_BORDER | WS_SYSMENU | WS_THICKFRAME | WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_VISIBLE;
        let bare = borderless_style(decorated);
        assert_eq!(bare & WS_CAPTION, 0, "caption gone");
        assert_eq!(bare & WS_BORDER, 0, "border gone");
        assert_eq!(bare & WS_THICKFRAME, WS_THICKFRAME, "edge resizing kept");
        assert_eq!(bare & (WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_SYSMENU), WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_SYSMENU, "window buttons kept");
        assert_eq!(bare & WS_VISIBLE, WS_VISIBLE, "unrelated bits kept");
    }

    #[test]
    fn an_already_borderless_style_is_unchanged_so_no_frame_change_is_made() {
        let bare = WS_THICKFRAME | WS_SYSMENU | WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_VISIBLE;
        assert_eq!(borderless_style(bare), bare);
        assert_eq!(borderless_style(borderless_style(bare | WS_CAPTION)), bare);
    }

    #[test]
    fn the_step_runs_each_time_the_frame_runs_it() {
        let calls = Arc::new(AtomicUsize::new(0));
        let n = calls.clone();
        let mut frame = WindowFrame::with_step(Box::new(move || {
            n.fetch_add(1, Ordering::SeqCst);
        }));
        for _ in 0..3 {
            frame.run();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn no_step_is_fine() {
        // Decorated windows and other platforms have no step; running must do nothing.
        let mut frame = WindowFrame::new(false);
        frame.run();
    }
}
