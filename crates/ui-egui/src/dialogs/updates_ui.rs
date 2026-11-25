//! Software updates, background check at startup, and startup prompt modal.

use crate::FigApp;

#[cfg(target_arch = "wasm32")]
use crate::i18n::tr;
#[cfg(not(target_arch = "wasm32"))]
use craft_update::updater::{Action, Status, Updater};
#[cfg(not(target_arch = "wasm32"))]
use craft_update::{CheckResultDialog, CheckResultResponse};

pub(crate) struct Runtime {
    /// The updater; `None` for the Launcher-managed Nightly, which never updates itself.
    #[cfg(not(target_arch = "wasm32"))]
    updater: Option<Updater>,
    /// The desktop app turns the check at start on; tests and the web build leave it off.
    #[cfg(not(target_arch = "wasm32"))]
    startup_check: bool,
    #[cfg(not(target_arch = "wasm32"))]
    check_requested: bool,
    /// The context of the last frame, to wake the UI when a check is queued from a menu callback.
    #[cfg(not(target_arch = "wasm32"))]
    repaint: Option<egui::Context>,
    /// The result dialog of a manual check is open.
    #[cfg(target_arch = "wasm32")]
    pub(crate) manual_window_open: bool,
    /// The web build has no updater; its only result is that checks are unsupported.
    #[cfg(target_arch = "wasm32")]
    pub(crate) status_message: Option<String>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            updater: Some(Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION"))),
            #[cfg(not(target_arch = "wasm32"))]
            startup_check: false,
            #[cfg(not(target_arch = "wasm32"))]
            check_requested: false,
            #[cfg(not(target_arch = "wasm32"))]
            repaint: None,
            #[cfg(target_arch = "wasm32")]
            manual_window_open: false,
            #[cfg(target_arch = "wasm32")]
            status_message: None,
        }
    }
}

impl FigApp {
    /// Whether the app asks for a newer release when it starts (off until the desktop app turns it on).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_startup_update_check(&mut self, on: bool) {
        self.updates_rt.startup_check = on;
    }

    /// Start the install of an update that waits for the app to close. The desktop app calls this
    /// when it exits, before it ends the process; later calls do nothing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_pending_update(&mut self) {
        if let Some(updater) = self.updates_rt.updater.as_mut() {
            updater.start_pending_update();
        }
    }

    /// Never check for or install an update: the Launcher owns the Nightly's version. The desktop
    /// app calls this before the first frame of a Nightly build, so nothing here reads or writes
    /// the update pins.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn disable_updates(&mut self) {
        self.updates_rt.updater = None;
        self.updates_rt.startup_check = false;
        self.updates_rt.check_requested = false;
    }
}

pub(crate) fn check(app: &mut FigApp) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // A build without an updater has no check to run (the Nightly is updated by the Launcher).
        if app.updates_rt.updater.is_some() {
            app.updates_rt.check_requested = true;
            if let Some(ctx) = &app.updates_rt.repaint {
                ctx.request_repaint();
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        app.updates_rt.manual_window_open = true;
        app.updates_rt.status_message = Some(tr("Update check is not supported in the web browser.").to_string());
    }
}

pub(crate) fn tick(app: &mut FigApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let rt = &mut app.updates_rt;
        rt.repaint = Some(ctx.clone());
        let check_requested = std::mem::take(&mut rt.check_requested);
        let startup_check = rt.startup_check;
        let Some(updater) = rt.updater.as_mut() else { return };
        if check_requested {
            updater.check_now(ctx);
        } else if startup_check {
            updater.check_at_start(ctx);
        }
        updater.tick(ctx);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

/// Whether the check result window shows: for every status but idle (a requested check, and the
/// first start after an update, which reports that it installed or why it failed), and never over
/// the startup prompt.
#[cfg(not(target_arch = "wasm32"))]
fn result_window_shows(status: &Status, prompt_open: bool) -> bool {
    !prompt_open && !matches!(status, Status::Idle)
}

pub(crate) fn show(app: &mut FigApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The updater asks to close only when no document has unsaved changes; the install starts when the app exits.
        let clean = !app.session.docs.iter().any(|d| d.is_dirty());
        let mut check_again = false;
        let mut restart_to_update = false;
        if let Some(updater) = app.updates_rt.updater.as_mut() {
            match updater.show_if(ctx, clean) {
                Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
                // The whole application, not just the window that drew the prompt: the root window's guarded close.
                Action::Close => ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close),
                Action::None => {}
            }
            if result_window_shows(updater.status(), updater.prompt_open()) {
                match CheckResultDialog::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")).show(ctx, updater.status()) {
                    CheckResultResponse::None => {}
                    CheckResultResponse::Close => updater.dismiss_status(),
                    CheckResultResponse::CheckAgain => check_again = true,
                    // The next `show_if` asks the window to close once no document has unsaved changes.
                    CheckResultResponse::RestartToUpdate => restart_to_update = true,
                }
            }
        }
        app.updates_rt.check_requested |= check_again;
        if restart_to_update && let Some(updater) = app.updates_rt.updater.as_mut() {
            updater.restart_to_update();
        }
    }

    #[cfg(target_arch = "wasm32")]
    if app.updates_rt.manual_window_open {
        let mut open = true;
        let mut close = false;
        egui::Window::new(tr("Check for Updates"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                if let Some(msg) = &app.updates_rt.status_message {
                    ui.label(msg);
                }
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("Close")).clicked() {
                        close = true;
                    }
                });
            });
        if !open || close {
            app.updates_rt.manual_window_open = false;
            app.updates_rt.status_message = None;
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn result_window_shows_every_status_but_idle() {
        let shown = [
            Status::Checking,
            Status::UpToDate,
            Status::Failed("no network".into()),
            Status::Installing,
            Status::Ready,
            Status::Managed,
            Status::Updated("0.2.0".into()),
            Status::InstallFailed("disk full".into()),
        ];
        for status in &shown {
            assert!(result_window_shows(status, false), "{status:?}");
            assert!(!result_window_shows(status, true), "{status:?} over the prompt");
        }
        assert!(!result_window_shows(&Status::Idle, false));
    }
}
