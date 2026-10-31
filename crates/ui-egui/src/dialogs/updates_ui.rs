//! Software updates, background check at startup, and startup prompt modal.

use crate::FigApp;
use crate::i18n::tr;

#[cfg(not(target_arch = "wasm32"))]
use craft_update::updater::{Action, Status, Updater};

const RELEASES_PAGE: &str = "https://github.com/storytold/figcraft/releases";

pub(crate) struct Runtime {
    #[cfg(not(target_arch = "wasm32"))]
    updater: Updater,
    /// The desktop app turns the check at start on; tests and the web build leave it off.
    #[cfg(not(target_arch = "wasm32"))]
    startup_check: bool,
    #[cfg(not(target_arch = "wasm32"))]
    check_requested: bool,
    /// The context of the last frame, to wake the UI when a check is queued from a menu callback.
    #[cfg(not(target_arch = "wasm32"))]
    repaint: Option<egui::Context>,
    pub(crate) manual_window_open: bool,
    pub(crate) status_message: Option<String>,
    /// The last check failed, so the dialog also names where the releases are listed.
    failed: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            updater: Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")),
            #[cfg(not(target_arch = "wasm32"))]
            startup_check: false,
            #[cfg(not(target_arch = "wasm32"))]
            check_requested: false,
            #[cfg(not(target_arch = "wasm32"))]
            repaint: None,
            manual_window_open: false,
            status_message: None,
            failed: false,
        }
    }
}

impl FigApp {
    /// Whether the app asks for a newer release when it starts (off until the desktop app turns it on).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_startup_update_check(&mut self, on: bool) {
        self.updates_rt.startup_check = on;
    }
}

pub(crate) fn check(app: &mut FigApp) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.updates_rt.check_requested = true;
        if let Some(ctx) = &app.updates_rt.repaint {
            ctx.request_repaint();
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
        if std::mem::take(&mut rt.check_requested) {
            rt.updater.check_now(ctx);
        } else if rt.startup_check {
            rt.updater.check_at_start(ctx);
        }
        rt.updater.tick(ctx);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

pub(crate) fn show(app: &mut FigApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The restart helper is scheduled before the app closes, so no update starts over unsaved work.
        let clean = !app.session.docs.iter().any(|d| d.is_dirty());
        match app.updates_rt.updater.show_if(ctx, clean) {
            Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            // The whole application, not just the window that drew the prompt: the root window's guarded close.
            Action::Close => ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close),
            Action::None => {}
        }
        // The window answers Help ▸ Check for Updates, so it follows the updater's status.
        let rt = &mut app.updates_rt;
        (rt.manual_window_open, rt.status_message, rt.failed) = match rt.updater.status() {
            Status::Idle => (false, None, false),
            Status::Checking => (true, None, false),
            Status::UpToDate => (true, Some(tr("FigCraft {version} is up to date.").replace("{version}", env!("CARGO_PKG_VERSION"))), false),
            Status::Failed(e) => (true, Some(tr("Could not check for updates:\n{error}").replace("{error}", e)), true),
        };
    }

    if app.updates_rt.manual_window_open {
        let t = crate::theme::Tokens::get(ctx);
        let version = env!("CARGO_PKG_VERSION");
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("check_for_updates")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(egui::RichText::new(tr("Check for Updates")).font(crate::theme::semibold(16.0)));
            ui.add_space(8.0);
            match &app.updates_rt.status_message {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("Checking for updates…"));
                    });
                }
                Some(msg) => {
                    ui.label(msg);
                    if app.updates_rt.failed {
                        ui.label(
                            egui::RichText::new(
                                tr("You have version {version}. All releases are listed at {page}.")
                                    .replace("{version}", version)
                                    .replace("{page}", RELEASES_PAGE),
                            )
                            .color(t.text_secondary),
                        );
                    }
                }
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::widgets::button(ui, tr("Close"), true, true).clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            app.updates_rt.manual_window_open = false;
            app.updates_rt.status_message = None;
            app.updates_rt.failed = false;
        }
    }

    // Closing the window dismisses the outcome it showed.
    #[cfg(not(target_arch = "wasm32"))]
    if !app.updates_rt.manual_window_open {
        app.updates_rt.updater.dismiss_status();
    }
}
