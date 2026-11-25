//! Software updates, background check at startup, and startup prompt modal. The flow itself is
//! `craft_update::updater::Updater`; its dialogs run on the app's
//! [`DialogHost`](craft_ui::dialog::DialogHost) ([`FigApp::framework_dialogs`]).

use crate::FigApp;

#[cfg(target_arch = "wasm32")]
use crate::i18n::tr;
#[cfg(not(target_arch = "wasm32"))]
use craft_ui::dialog::DialogResult;
#[cfg(not(target_arch = "wasm32"))]
use craft_update::stack_dialogs::{CheckChoice, UpdateCheckDialog};
#[cfg(not(target_arch = "wasm32"))]
use craft_update::updater::{Action, Status, Updater};

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
            updater.check_now();
        } else if startup_check {
            updater.check_at_start();
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

/// One frame of the update flow, in the main window only: answers the result dialog closed in the
/// last pass, drives the startup prompt, keeps the result dialog open and current while the updater
/// has something to report, then runs the dialog host.
pub(crate) fn show(app: &mut FigApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    #[cfg(not(target_arch = "wasm32"))]
    {
        if ctx.viewport_id() != egui::ViewportId::ROOT {
            return;
        }
        // The updater asks to close only when no document has unsaved changes; the install starts when the app exits.
        let clean = !app.session.docs.iter().any(|d| d.is_dirty());
        // A build without an updater (the Launcher-managed Nightly) has no update flow and no update dialog.
        if let Some(updater) = app.updates_rt.updater.as_mut() {
            // Answered before the dialog is kept open below, so a dismissed outcome does not open again at once.
            match app.framework_dialogs.take::<UpdateCheckDialog>() {
                Some(DialogResult::Ok(CheckChoice::CheckNow | CheckChoice::CheckAgain)) => app.updates_rt.check_requested = true,
                // The next `run` asks the window to close once no document has unsaved changes.
                Some(DialogResult::Ok(CheckChoice::RestartToUpdate)) => updater.restart_to_update(),
                Some(DialogResult::Ok(CheckChoice::Update)) => {
                    updater.request_install(clean);
                }
                Some(DialogResult::Ok(CheckChoice::Download)) => ctx.open_url(egui::OpenUrl::new_tab(crate::UPDATE_APP.releases_url())),
                // Closing the dialog dismisses the outcome it showed.
                Some(_) => updater.dismiss_status(),
                None => {}
            }
            match updater.run(&mut app.framework_dialogs, clean) {
                Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
                // The whole application, not just the window that drew the prompt: the root window's guarded close.
                Action::Close => ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close),
                _ => {}
            }
            let shows = result_window_shows(updater.status(), updater.prompt_open());
            let (status, installing, message) = (updater.status().clone(), updater.is_installing(), updater.status_message());
            let dialogs = &mut app.framework_dialogs;
            if shows && !dialogs.is_open::<UpdateCheckDialog>() {
                dialogs.open(UpdateCheckDialog::new(env!("CARGO_PKG_VERSION")));
            } else if !shows && dialogs.is_open::<UpdateCheckDialog>() {
                dialogs.cancel::<UpdateCheckDialog>(&ctx);
            }
            if let Some(dialog) = dialogs.get_mut::<UpdateCheckDialog>() {
                dialog.set_status(status);
                dialog.set_installing(installing);
                dialog.set_message(message);
            }
            // The outcome of the last update is a modeless notice: Escape closes it wherever the focus is, after FigCraft's
            // own readers of the window's Escape. A modal result reads its Escape when the host runs.
            if dialogs.is_open::<UpdateCheckDialog>() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                dialogs.cancel::<UpdateCheckDialog>(&ctx);
            }
        }
        app.framework_dialogs.run(ui);
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
            .show(&ctx, |ui| {
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
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};

    use craft_update::Paths;
    use craft_update::updater::CheckSource;
    use figcraft_engine::Session;
    use serde_json::json;

    use super::*;
    use crate::layers_drag_tests::Ui;

    /// Escape closes the result of a requested check, which is modal, and the notice of the last update, which is not
    /// and need not have the focus. The modal result's Escape is its own: the layers stay selected.
    #[test]
    fn escape_closes_the_update_dialogs() {
        let dir = std::env::temp_dir().join(format!("figcraft-update-escape-{}", std::process::id()));
        let paths = Paths::under(&dir);
        std::fs::create_dir_all(&paths.data).unwrap();
        std::fs::write(paths.data.join("self-update-done.txt"), "v0.2.0").unwrap();
        // A check runs until `running` goes away at the end of the test.
        let (running, wait) = channel::<()>();
        let wait = Mutex::new(wait);
        let source: CheckSource = Arc::new(move || {
            if let Ok(wait) = wait.lock() {
                let _ = wait.recv();
            }
            Ok(None)
        });
        let updater = || {
            let updater = Updater::new(&crate::UPDATE_APP, "0.2.0").with_source(source.clone());
            Some(updater.with_pins_path(dir.join("pins.json")).with_paths(paths.clone()))
        };
        let status = |u: &Ui| u.app.updates_rt.updater.as_ref().map(|updater| updater.status().clone());
        let open = |u: &Ui| u.app.framework_dialogs.is_open::<UpdateCheckDialog>();
        let frames = |u: &mut Ui| (0..4).for_each(|_| u.frame(vec![]));
        let escape = |u: &mut Ui| {
            let key =
                |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
            u.frame(vec![key(true), key(false)]);
            // The frames that read the dialog's answer.
            u.frame(vec![]);
            u.frame(vec![]);
        };

        let mut session = Session::new();
        let layer = session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap()["id"].clone();
        session.execute("select.set", &json!({"ids": [layer]})).unwrap();
        let mut u = Ui::new(session);
        let selected = u.app.session.selection();
        assert!(!selected.is_empty());

        u.app.updates_rt.updater = updater();
        check(&mut u.app);
        frames(&mut u);
        assert!(open(&u) && status(&u) == Some(Status::Checking));
        escape(&mut u);
        assert!(!open(&u), "Escape closes the check");
        assert_eq!(status(&u), Some(Status::Idle));
        assert_eq!(u.app.session.selection(), selected);

        u.app.updates_rt.updater = updater();
        u.app.set_startup_update_check(true);
        frames(&mut u);
        assert!(open(&u) && status(&u) == Some(Status::Updated("0.2.0".into())));
        if let Some(focused) = u.ctx.memory(|m| m.focused()) {
            u.ctx.memory_mut(|m| m.surrender_focus(focused));
        }
        escape(&mut u);
        assert!(!open(&u), "Escape closes the notice");
        assert_eq!(status(&u), Some(Status::Idle));
        drop(running);
        let _ = std::fs::remove_dir_all(dir);
    }

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
