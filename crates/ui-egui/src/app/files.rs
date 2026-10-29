use super::{
    AUTO_SAVE_RETRY_SECS, AUTO_SAVE_SECS, AUTOSAVE_SECS, Arc, DialogFuture, FigApp, OPEN_EXTENSIONS, OsStr, Path, PathBuf, PendingDialog, PickFuture,
    Picked, Poll, RepaintWaker, Services, TaskContext, Value, Waker, dialogs, i18n, json, repaint_in, windows,
};

impl FigApp {
    pub fn save_as_dialog(&mut self) {
        if self.services.downloads() {
            if let Some(r) = self.run("file.saveAs", json!({"data": true}))
                && self.download(r["name"].as_str().unwrap_or_default(), &r)
            {
                self.run("file.markSaved", json!({}));
                dialogs::file_saved(self, &r);
            }
            return;
        }
        let Ok(name) = self.session.doc().map(|d| d.name.clone()) else { return };
        self.pick_save(i18n::tr("Save as"), &format!("{name}.figcraft"), &["figcraft", "fig"], |app, mut p| {
            if p.extension().is_none_or(OsStr::is_empty) {
                if !p.set_extension("figcraft") {
                    app.toast(i18n::tr("Use the .figcraft or .fig extension."));
                    return;
                }
                // This name wasn't confirmed by the system dialog: don't replace an existing file.
                match p.try_exists() {
                    Ok(false) => {}
                    Ok(true) => {
                        app.toast(i18n::tr("A .figcraft file with this name already exists. Choose its full name to confirm replacement."));
                        return;
                    }
                    Err(e) => {
                        app.toast(e.to_string());
                        return;
                    }
                }
            } else if !p.extension().and_then(OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("figcraft") || e.eq_ignore_ascii_case("fig")) {
                app.toast(i18n::tr("Use the .figcraft or .fig extension."));
                return;
            }
            if let Some(r) = app.run("file.saveAs", json!({"path": p.to_string_lossy()})) {
                dialogs::file_saved(app, &r);
            }
        });
    }

    pub fn save(&mut self) {
        // A branch tab saves its main file.
        let tab = figcraft_engine::branches::file_tab(&self.session);
        // A path ending in .fig is never saved over silently: a .fig destination is an export copy.
        let native_path = self
            .session
            .docs
            .get(tab)
            .and_then(|d| d.path.as_deref())
            .is_some_and(|path| !Path::new(path).extension().and_then(OsStr::to_str).is_some_and(|e| e.eq_ignore_ascii_case("fig")));
        if native_path {
            if self.run("file.save", json!({})).is_some() {
                self.toast(i18n::tr("Saved"));
            }
        } else {
            self.save_as_dialog();
        }
    }

    pub fn open_dialog(&mut self) {
        // Opening needs no file in front: it goes ahead even if that file closed meanwhile.
        self.pick_file(i18n::tr("Open"), OPEN_EXTENSIONS, false, |app, file| {
            if app.run_user("file.open", file).is_some() {
                app.ui.home = false;
            }
        });
    }

    pub fn import_dialog(&mut self, place: bool) {
        let exts: &[&str] = if place {
            &["png", "jpg", "jpeg", "webp", "gif", "tif", "tiff"]
        } else {
            &["penpot", "svg", "png", "jpg", "jpeg", "webp", "gif", "tif", "tiff", "figcraft", "fig", "sketch"]
        };
        let title = if place { i18n::tr("Place image") } else { i18n::tr("Import") };
        self.pick_file(title, exts, true, |app, file| {
            app.run_user("file.import", file);
        });
    }

    /// Pick a file to open or import; `done` gets it as `file.open` and `file.import` take it (a
    /// path, or the name and contents where the platform has no paths). `pin`: as
    /// [`FigApp::pick_open`]'s file in front.
    fn pick_file(&mut self, title: &str, exts: &[&str], pin: bool, done: impl FnOnce(&mut FigApp, Value) + 'static) {
        self.start_pick(
            |services| -> PickFuture {
                match services.pick_open_contents(title, exts) {
                    Some(read) => {
                        Box::pin(async move { read.await.map(|r| r.map_or_else(Picked::Unreadable, |(name, bytes)| Picked::Contents(name, bytes))) })
                    }
                    None => {
                        let pick = services.pick_open(title, exts);
                        Box::pin(async move { pick.await.map(Picked::Path) })
                    }
                }
            },
            pin,
            move |app, picked| match picked.file_params() {
                Ok(file) => done(app, file),
                Err(why) => app.show_error(why),
            },
        );
    }

    /// Pick a file to open. `done` runs with the file in front now back in front (and not at all
    /// if that file closed meanwhile).
    pub fn pick_open(&mut self, title: &str, exts: &[&str], done: impl FnOnce(&mut FigApp, PathBuf) + 'static) {
        self.start_dialog(|services| services.pick_open(title, exts), true, done);
    }

    /// Pick where to save; `done` runs as [`FigApp::pick_open`]'s does.
    pub fn pick_save(&mut self, title: &str, name: &str, exts: &[&str], done: impl FnOnce(&mut FigApp, PathBuf) + 'static) {
        self.start_dialog(|services| services.pick_save(title, name, exts), true, done);
    }

    /// Hand a command result's base64 `data` over as a download named `name`, where the platform
    /// saves by downloads rather than save dialogs (the browser, #969). Says so when nothing arrived.
    pub(crate) fn download(&mut self, name: &str, result: &Value) -> bool {
        use base64::Engine as _;
        let bytes = result["data"].as_str().and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok());
        if bytes.is_some_and(|b| self.services.download(name, &b)) {
            return true;
        }
        self.toast(i18n::trf("Couldn't download {name}", &[("name", &name)]));
        false
    }

    /// Pick a folder; `done` runs as [`FigApp::pick_open`]'s does.
    pub fn pick_folder(&mut self, title: &str, done: impl FnOnce(&mut FigApp, PathBuf) + 'static) {
        self.start_dialog(|services| services.pick_folder(title), true, done);
    }

    /// Open a native dialog from the window drawing now, for the file in front (`pin`).
    fn start_dialog(&mut self, start: impl FnOnce(&mut dyn Services) -> DialogFuture, pin: bool, done: impl FnOnce(&mut FigApp, PathBuf) + 'static) {
        self.start_pick(
            |services| -> PickFuture {
                let pick = start(services);
                Box::pin(async move { pick.await.map(Picked::Path) })
            },
            pin,
            move |app, picked| {
                if let Picked::Path(p) = picked {
                    done(app, p);
                }
            },
        );
    }

    /// [`FigApp::start_dialog`] for any kind of pick.
    fn start_pick(&mut self, start: impl FnOnce(&mut dyn Services) -> PickFuture, pin: bool, done: impl FnOnce(&mut FigApp, Picked) + 'static) {
        if self.pending_dialog.is_some() {
            return;
        }
        let doc = if pin { self.session.ds().ok().filter(|d| self.shows(d.uid)).map(|d| d.uid) } else { None };
        let future = start(self.services.as_mut());
        self.pending_dialog = Some(PendingDialog { future, picked: None, done: Box::new(done), doc, window: self.windows.current });
        if let Some(ctx) = &self.ui_context {
            ctx.request_repaint();
        }
    }

    /// Once the dialog has a path, its continuation runs in the window that holds its file, with
    /// that file in front; then the tab in front before comes back (unless the dialog opened a file).
    pub(crate) fn poll_dialog(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.pending_dialog else { return };
        if d.picked.is_none() {
            let waker = Waker::from(Arc::new(RepaintWaker(ctx.clone())));
            let Poll::Ready(picked) = d.future.as_mut().poll(&mut TaskContext::from_waker(&waker)) else { return };
            d.picked = picked;
        }
        let (doc, origin, picked) = (d.doc, d.window, d.picked.is_some());
        let index = doc.map(|u| self.session.docs.iter().position(|ds| ds.uid == u));
        // Cancelled, or its file closed meanwhile.
        if !picked || index == Some(None) {
            self.pending_dialog = None;
            return;
        }
        let window = match doc {
            Some(u) => self.windows.of(u),
            None if self.windows.exists(origin) => origin,
            None => 0,
        };
        if window != self.windows.current {
            ctx.request_repaint_of(windows::viewport(window));
            return;
        }
        let Some(PendingDialog { picked: Some(picked), done, .. }) = self.pending_dialog.take() else { return };
        let before = self.session.ds().ok().map(|d| d.uid);
        if let Some(Some(i)) = index {
            self.session.active = i;
        }
        done(self, picked);
        if doc.is_some()
            && self.session.ds().ok().map(|d| d.uid) == doc
            && let Some(i) = before.and_then(|u| self.session.docs.iter().position(|d| d.uid == u))
        {
            self.session.active = i;
        }
    }

    /// Every [`AUTOSAVE_SECS`], each open document with unsaved changes since its last version
    /// gets an autosave checkpoint in its version history.
    pub(super) fn autosave_tick(&mut self, ctx: &egui::Context, now: f64) {
        if now < self.ui.next_autosave {
            // Wake up for it even if nothing else draws a frame by then.
            repaint_in(ctx, self.ui.next_autosave - now);
            return;
        }
        if self.ui.next_autosave > 0.0 {
            let active = self.session.active;
            for i in 0..self.session.docs.len() {
                self.session.active = i;
                let _ = self.session.execute("version.autosave", &json!({}));
            }
            self.session.active = active;
        }
        self.ui.next_autosave = now + AUTOSAVE_SECS;
        repaint_in(ctx, AUTOSAVE_SECS);
    }

    /// With auto-save on, files that have a path are saved [`AUTO_SAVE_SECS`] after they last
    /// changed (not during a gesture, a live preview or while a version is shown). The file is
    /// encoded on a thread of its own, so a big one doesn't hold up the window, and written here.
    pub(super) fn auto_save_tick(&mut self, ctx: &egui::Context, now: f64) {
        let mut failed = self.auto_save_done();
        // The browser has no files to save to.
        let on = !cfg!(target_arch = "wasm32") && self.session.prefs.auto_save;
        let saving = &self.ui.auto_saving;
        let due = |d: &&figcraft_engine::DocState| {
            d.path.as_deref().is_some_and(|p| d.images_lost_saving_to(p).is_none())
                && d.branch_of.is_none()
                && d.preview.is_none()
                && d.is_dirty()
                && !saving.iter().any(|(uid, _)| *uid == d.uid)
        };
        let changes = if on { self.session.docs.iter().filter(due).map(|d| d.revision).reduce(u64::wrapping_add) } else { None };
        match changes {
            None => self.ui.auto_save_at = None,
            Some(changes) => {
                let at = match self.ui.auto_save_at {
                    Some((at, c)) if c == changes && failed.is_none() => at,
                    _ => now + if failed.is_some() { AUTO_SAVE_RETRY_SECS } else { AUTO_SAVE_SECS },
                };
                self.ui.auto_save_at = Some((at, changes));
                if now < at || self.session.docs.iter().any(|d| d.interaction.is_some() || d.text_preview.is_some()) {
                    // Due but busy: look again in a second.
                    repaint_in(ctx, if now < at { at - now } else { 1.0 });
                } else {
                    self.ui.auto_save_at = None;
                    let files: Vec<_> =
                        self.session.docs.iter().filter(due).map(|d| (d.uid, figcraft_engine::cmd::file::FileSnapshot::of(d))).collect();
                    for (uid, file) in files {
                        let encode = move || {
                            let bytes = file.encode();
                            (file, bytes)
                        };
                        match std::thread::Builder::new().name("figcraft-save".into()).spawn(encode) {
                            Ok(h) => self.ui.auto_saving.push((uid, h)),
                            Err(e) => {
                                failed.get_or_insert(e.to_string());
                                self.ui.auto_save_at = Some((now + AUTO_SAVE_RETRY_SECS, changes));
                            }
                        }
                    }
                }
            }
        }
        if !self.ui.auto_saving.is_empty() {
            // Look again shortly for the files being encoded.
            repaint_in(ctx, 0.1);
        }
        if let Some(e) = failed {
            self.toast(i18n::trf("Couldn't save automatically: {error}", &[("error", &e)]));
        }
    }

    /// Write the files whose auto-save encoding finished; the first error, if one failed.
    fn auto_save_done(&mut self) -> Option<String> {
        if self.ui.auto_saving.is_empty() {
            return None;
        }
        let mut failed = None;
        let done: Vec<_> = self.ui.auto_saving.extract_if(.., |(_, h)| h.is_finished()).collect();
        for (_, h) in done {
            let saved = match h.join() {
                Ok((snapshot, Ok(bytes))) => snapshot.save(&mut self.session, &bytes).map(|_| ()).map_err(|e| e.to_string()),
                Ok((_, Err(e))) => Err(e.to_string()),
                Err(_) => Err("the file couldn't be encoded".to_string()),
            };
            if let Err(e) = saved {
                failed.get_or_insert(e);
            }
        }
        failed
    }
}
