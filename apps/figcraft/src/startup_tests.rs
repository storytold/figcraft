//! The message shown when the window can't open: it carries the error, a next step and where to
//! report it, instead of the app quitting silently (#261).

use figcraft_ui_egui::dialogs::window_failed as message;
use figcraft_ui_egui::i18n::tr;

const ERROR: &str = "WGPU error: Failed to create surface for any enabled backend: {}";
const BACKEND_HINT: &str = "WGPU_BACKEND is set and names the graphics API to use; unsetting it lets FigCraft choose.";
const DRIVER_HINT: &str = if cfg!(windows) {
    "Updating the graphics driver often helps. To use OpenGL only, start FigCraft from a Command Prompt with:"
} else {
    "Updating the graphics driver often helps. WGPU_BACKEND (vulkan, gl, metal) names another graphics API to try."
};

#[test]
fn the_message_carries_the_error_and_where_to_report_it() {
    let text = message(ERROR, false);
    assert!(text.contains(ERROR), "the error itself: {text}");
    assert!(text.contains("https://github.com/storytold/figcraft/issues"), "where to report it: {text}");
}

#[test]
fn without_wgpu_backend_it_suggests_the_driver_and_another_graphics_api() {
    let text = message(ERROR, false);
    assert!(text.contains(tr(DRIVER_HINT)), "{text}");
    assert!(text.contains("WGPU_BACKEND"), "another graphics API to try: {text}");
    assert!(!text.contains(tr(BACKEND_HINT)), "{text}");
}

#[cfg(windows)]
#[test]
fn on_windows_it_shows_how_to_start_with_opengl() {
    let text = message(ERROR, false);
    assert!(text.contains("set WGPU_BACKEND=gl"), "a release build has no console to look this up from: {text}");
}

#[test]
fn with_wgpu_backend_set_it_suggests_unsetting_it() {
    let text = message(ERROR, true);
    assert!(text.contains(tr(BACKEND_HINT)), "{text}");
    assert!(!text.contains(tr(DRIVER_HINT)), "the chosen backend is the likelier cause: {text}");
}
