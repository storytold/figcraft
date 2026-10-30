use super::clock::{AUTO_SAVE_RETRY_SECS, AUTO_SAVE_SECS, AUTOSAVE_SECS, CJK_CHANGES, FrameClock, TOAST_SECS, ToastClock, repaint_in};
use super::open::OPEN_EXTENSIONS;
use super::services::{DialogFuture, PickFuture, Picked, RepaintWaker, Services};
use super::state::{LEFT_PANEL, LeftTab, MOBILE_STRIP, MobilePanel, NARROW_WIDTH, PANEL_DEFAULT, RIGHT_PANEL, RightTab, TitleBar, fit_panels};
use super::{
    Arc, Done, FigApp, NodeId, OsStr, Path, PathBuf, PendingAck, PendingDialog, Poll, Session, TaskContext, UiState, Value, Waker, a11y, branch_ui,
    canvas, cjk, comments, cursors, dev_compare, dialogs, frame_stats, home, home_recents, i18n, icons, import_report, json, layers, live, menus,
    missing_fonts, online, plugins_ui, present, render_worker, right, selection_spacing, shortcuts, style_picker, tabs, theme, toolbar, ui_scale,
    updates_ui, variable_ui, versions, widgets, windows,
};

mod clipboard;
mod commands;
mod control;
mod files;
mod frame;
mod ime;

impl FigApp {
    /// Construct `FigApp` from a session and services, taking panel widths and pages height
    /// from preferences and initializing UI, canvas, clocks, capture/IME and transient channel
    /// state.
    pub fn new(session: Session, services: Box<dyn Services>) -> Self {
        // The main window's panels open as wide as they were left.
        let (left_width, right_width) = session.prefs.panel_widths;
        let pages_height = session.prefs.pages_height;
        Self {
            ui: UiState { left_width, right_width, pages_height, ..UiState::default() },
            session,
            canvas: canvas::CanvasState::default(),
            services,
            worker: None,
            fonts_installed: false,
            script_fonts: None,
            cjk_checked: None,
            language: None,
            field_focus: false,
            panels_unsaved: false,
            control_queue: Vec::new(),
            control_input: Vec::new(),
            frame_stats: frame_stats::FrameStats::default(),
            windows: windows::Windows::default(),
            window_requests: Vec::new(),
            waiting: Vec::new(),
            pending_dialog: None,
            ui_context: None,
            automation_roots: None,
            control_input_doc: None,
            control_synthetic: false,
            input_acks: Vec::new(),
            applied_acks: Vec::new(),
            applied_input: None,
            control_input_queued: false,
            applied_frame: None,
            clock: FrameClock::default(),
            capture: crate::control::CaptureState::default(),
            logic_ticked: false,
            ui_frame_stamp: None,
            ime: egui::ime_context::ImeContextTracker::new(),
            updates_rt: updates_ui::Runtime::default(),
            #[cfg(test)]
            ime_observations: 0,
        }
    }

    /// The window's wgpu state, for showing the GPU canvas's texture (desktop; the app sets it
    /// each frame).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_render_state(&mut self, state: egui_wgpu::RenderState) {
        self.canvas.render_state = Some(state);
    }

    /// UI-phase setup that needs a real current `Ui`: fonts/language/theme, interface zoom and the
    /// render worker. Runs once per frame (pass 0) for the main window, and once per frame for
    /// every child window (`windows::draw`), never on a logic-only tick.
    fn ui_setup(&mut self, ctx: &egui::Context) {
        self.viewport_setup(ctx);
    }

    /// The app-global part of [`Self::ui_setup`] that every window's frame needs: language, fonts,
    /// theme and `ui.dark`, the render worker, and the interface zoom plan. eframe only calls
    /// `App::ui` for the root window, so a child must run this itself before it draws; a child that
    /// only runs while the root is minimised still gets theme switches, CJK faces for its own
    /// document, and the right `ui.dark` for its tabs and dialogs.
    pub(crate) fn viewport_setup(&mut self, ctx: &egui::Context) {
        if self.language.as_ref() != Some(&self.session.prefs.language) {
            i18n::set(&self.session.prefs.language, self.services.system_language().as_deref());
            self.language = Some(self.session.prefs.language.clone());
        }
        // Screen-reader and bidi language of the UI (fork E17).
        ctx.set_language(i18n::current().map(|l| l.code.to_string()));
        if !self.fonts_installed {
            // Cmd/Ctrl+= / - / 0 zoom the canvas (view.zoomIn, view.zoomOut, view.zoom100): egui's
            // own keyboard zoom would scale the whole interface on the same keys.
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
            // A double-click's window is the platform's (0.5 s, Windows' default), not egui's
            // stricter 0.3 s: a double-click the OS takes must enter a shape's points too.
            ctx.memory_mut(|m| m.options.input_options.max_double_click_delay = 0.5);
            theme::install_fonts(ctx, false, 0);
            self.fonts_installed = true;
            // Installed fonts apply from the next frame; a child window draws on here, so ask for
            // that frame (the root draws nothing on the frame it installs them).
            ctx.request_repaint();
        } else if figcraft_text::system_fonts_scanned() {
            // Once the system fonts are indexed: faces for names in scripts the UI fonts lack, and
            // for each CJK script once something shows it.
            let installed = self.script_fonts.map_or(0, |(s, _)| s);
            let cjk = installed | self.cjk_needed(ctx, installed);
            // The UI language's script leads the CJK faces, so another language lays them out again.
            let lang = i18n::current().map_or("en", |l| l.code);
            if self.script_fonts != Some((cjk, lang)) {
                self.script_fonts = Some((cjk, lang));
                theme::install_fonts(ctx, true, cjk);
            }
        }
        // egui's zoom is for the whole app and multiplies each window's own system scale, so the
        // size is the same relative to every screen; the main window, whose size can hold a size
        // back, sets it. The browser keeps its own zoom. Every window records its own plan (the
        // Preferences warning and `ui.inspect` read it), from its own size.
        {
            let window_px = ctx.content_rect().size() * ctx.pixels_per_point();
            let size = if cfg!(target_arch = "wasm32") { 1.0 } else { self.session.prefs.ui_scale };
            let plan = ui_scale::plan(size, ctx.native_pixels_per_point(), Some((window_px.x, window_px.y)));
            if ctx.viewport_id() == egui::ViewportId::ROOT {
                ctx.set_zoom_factor(plan.zoom);
            }
            self.ui.ui_scale = plan;
        }
        let dark = match self.session.prefs.theme.as_str() {
            "dark" => true,
            // The active visuals were set by us last frame (possibly for a manual theme). Read the
            // integration's OS theme; winit reports none on Linux and the BSDs, where the platform
            // services' colour scheme answers instead. Without either, keep the light fallback.
            "system" => ctx.system_theme().map(|t| t == egui::Theme::Dark).or_else(|| self.services.system_dark()).unwrap_or(false),
            _ => false,
        };
        self.ui.dark = dark;
        // New files take the theme's page colour (existing pages keep theirs).
        self.session.set_ui_dark(dark);
        theme::apply_with(ctx, if dark { theme::Tokens::dark() } else { theme::Tokens::light() }, self.reduce_motion());
        if self.worker.is_none() {
            self.worker = Some(render_worker::Worker::new(ctx.clone()));
        }
    }

    /// Draw the whole UI: the main window (the other windows draw themselves).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        let t0 = std::time::Instant::now();
        let ctx = ui.ctx().clone();
        self.ui_context = Some(ctx.clone());
        // `App::logic` already ticked this pass; direct harnesses do not call it. The marker only
        // covers the tick that shares this pass: clearing it here keeps a direct `App::ui` harness
        // ticking on every frame (it used to tick every other frame).
        if !std::mem::take(&mut self.logic_ticked) {
            self.background_tick(&ctx);
            self.logic_ticked = false;
        }
        // Setup once per frame (pass 0), not once per pass and never on a logic-only tick.
        let stamp = (ctx.viewport_id(), ctx.cumulative_frame_nr());
        let first = !self.fonts_installed;
        if ctx.current_pass_index() == 0 && self.ui_frame_stamp != Some(stamp) {
            self.ui_frame_stamp = Some(stamp);
            self.frame_stats.record(&ctx);
            self.ui_setup(&ctx);
        }
        if first && self.fonts_installed {
            // New fonts apply from the next frame; draw nothing until then. Anything taken into
            // this frame's (undrawn) input waits for the first drawn frame.
            ctx.request_repaint();
            if let Some(events) = self.applied_input.take() {
                self.push_control_input(events);
            }
            self.input_acks.append(&mut self.applied_acks);
            self.applied_frame = None;
            self.control_synthetic = false;
            // The next (drawn) frame ticks for itself in a direct harness.
            self.logic_ticked = false;
            self.observe_ime(&ctx, false);
            return;
        }
        windows::begin(self, &ctx);
        let (files, prefs) = (self.session.docs.len(), self.session.prefs.clone());
        self.frame(ui);
        windows::settle(self, &ctx, files);
        if self.session.prefs != prefs {
            windows::repaint_others(self, &ctx);
        }
        windows::show(self, &ctx);
        // The injected batch was applied by the frame that took it. A discarded sizing pass keeps
        // its acknowledgement for the presented pass but never replays the input (its side effects
        // already happened and would otherwise duplicate).
        self.end_input_frame(&ctx);
        // Plugins installed, removed or set up are kept at once: a crash doesn't lose them.
        // So are the owner tokens of files put online.
        if std::mem::take(&mut self.session.plugins.unsaved) | std::mem::take(&mut self.session.live.unsaved) {
            self.services.save_prefs(&self.session.prefs);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.ui.frame_ms = t0.elapsed().as_secs_f64() * 1000.0;
            self.frame_stats.timed(self.ui.frame_ms);
        }
    }

    /// One frame of the window drawing now.
    pub(crate) fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // Bind this input batch before controls or pointer input can replace its composing
        // target/caret. Outgoing IME receipts run after mutation and cannot authorize it.
        let ime_input = shortcuts::ImeInputBaseline::capture(self);
        let t = theme::Tokens::get(&ctx);
        // A field that had the keyboard when the frame began keeps its keys from the shortcuts
        // even if it lets go during the frame (Enter commits it), or as it began (egui drops the
        // focus on Escape before the frame, which cancels the field's edit and nothing else).
        let typing = std::mem::take(&mut self.field_focus) || a11y::field_focused(&ctx);
        // Bind the originating field/file before tabs can switch. Its widget commits only after
        // processing this batch; delivery remains deferred until the following frame.
        let field_save = shortcuts::prepare_field_save(self, &ctx);
        let swallowed = shortcuts::swallowed_paste(self, &ctx);
        // A dialog that closed acts first, in the window it was opened from.
        self.poll_dialog(&ctx);
        // Control requests run first so their results reflect this frame.
        crate::control::drain(self, &ctx);
        windows::requests(self, &ctx);
        selection_spacing::sync(self, &ctx);
        // Before any widget: the navigation keys (F6, Tab, arrows, Escape) move the keyboard.
        a11y::begin(self, &ctx, ui.scope_id());
        if self.ui.title_bar == TitleBar::Custom {
            tabs::resize_edges(&ctx);
        }
        if self.ui.icon_gallery {
            icons::gallery(&ctx);
        }
        theme::set_doc_profile(&ctx, self.session.doc().map_or(figcraft_color::ColorProfile::Srgb, |d| d.color_profile));
        widgets::set_nudge(&ctx, self.session.prefs.nudge_small, self.session.prefs.nudge_big);
        self.sync_dev_mode();
        self.end_pen_outside_design();
        tabs::relink_presents(self);
        tabs::tab_bar(self, ui);
        // Control/window work and this pass's physical tab click may have replaced the owner.
        // A shared field ID in that replacement must never consume the old commit marker.
        let field_save = shortcuts::revalidate_field_save(self, &ctx, field_save);
        if self.ui.present.is_some() {
            // A close request can arrive while the presenter fills the window. Let its unsaved
            // changes dialog take Escape and draw above the player just as it does in the editor.
            dialogs::keys(self, &ctx);
            present::show(self, ui);
            dialogs::show(self, &ctx);
            updates_ui::show(self, &ctx);
            // What went wrong while presenting (a widget's plugin failing) shows over it.
            self.toast_ui(&ctx);
            windows::requests(self, &ctx);
            shortcuts::clear_field_save_marker(&ctx);
            self.observe_ime(&ctx, false);
            return;
        }
        if !self.has_files() || self.ui.home {
            home::show(self, ui);
            // The shortcuts panel belongs to the canvas; it doesn't hang over the home screen
            // (cleared after the home widgets, which may have just asked for it).
            self.ui.shortcuts_panel = false;
            dialogs::keys(self, &ctx);
            dialogs::show(self, &ctx);
            updates_ui::show(self, &ctx);
            self.toast_ui(&ctx);
            windows::requests(self, &ctx);
            shortcuts::clear_field_save_marker(&ctx);
            self.observe_ime(&ctx, false);
            return;
        }
        let hidden = self.session.ds().map(|d| d.view.ui_hidden).unwrap_or(false);
        let narrow = !hidden && ui.available_width() < NARROW_WIDTH;
        let mobile = self.ui.mobile_panel.filter(|_| narrow);
        self.ui.narrow = narrow;
        // A narrow window shows the right panel when its button asks for it, a board's too.
        let right_open = (self.right_panel_shown(hidden) && !narrow) || mobile == Some(MobilePanel::Right);
        if !hidden && (!narrow || mobile == Some(MobilePanel::Left)) {
            egui::Panel::left("left_rail")
                .exact_size(layers::RAIL_WIDTH)
                .resizable(false)
                .frame(egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)))
                .show(ui, |ui| {
                    a11y::region(&mut self.ui.nav, ui, a11y::Region::Layers, egui::accesskit::Role::Navigation, i18n::tr("Sidebar"));
                    layers::rail(self, ui)
                });
        }
        // The panels are as wide as asked, less in a window too small for them and the canvas.
        let left_open = !hidden && if narrow { mobile == Some(MobilePanel::Left) } else { !self.ui.left_collapsed };
        let (left_w, right_w) = if narrow {
            let w = (ui.available_width() - MOBILE_STRIP).max(0.0);
            (if left_open { w } else { 0.0 }, w)
        } else {
            fit_panels(ui.available_width(), left_open.then_some(self.ui.left_width), right_open.then_some(self.ui.right_width))
        };
        let rows = ui.available_rect_before_wrap().y_range();
        let mut edges: Vec<(&str, f32, f32, f32)> = Vec::new();
        if left_open {
            if !narrow {
                edges.push(("left", ui.available_rect_before_wrap().min.x + left_w, left_w, 1.0));
            }
            egui::Panel::left("left_panel")
                .resizable(false)
                .exact_size(left_w)
                .frame(egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)))
                .show(ui, |ui| {
                    let label = match self.ui.left_tab {
                        _ if self.ui.versions.open => i18n::tr("Version history"),
                        LeftTab::File => i18n::tr("Layers"),
                        LeftTab::Assets => i18n::tr("Assets"),
                    };
                    a11y::region(&mut self.ui.nav, ui, a11y::Region::Layers, egui::accesskit::Role::Pane, label);
                    layers::left_panel(self, ui)
                });
        }
        self.ui.right_panel_rect = None;
        if right_open {
            if !narrow {
                edges.push(("right", ui.available_rect_before_wrap().max.x - right_w, right_w, -1.0));
            }
            egui::Panel::right("right_panel")
                .resizable(false)
                .exact_size(right_w)
                // No stroke: a frame's stroke is part of its margin, which would take 1 pt off
                // each side of the panel (240 pt by default). The edge is drawn in `right_panel`.
                .frame(egui::Frame::NONE.fill(t.panel))
                .show(ui, |ui| {
                    let label = match self.ui.right_tab {
                        RightTab::Design => i18n::tr("Design"),
                        RightTab::Prototype => i18n::tr("Prototype"),
                        RightTab::Inspect => i18n::tr("Dev Mode"),
                    };
                    a11y::region(&mut self.ui.nav, ui, a11y::Region::Properties, egui::accesskit::Role::Pane, label);
                    right::right_panel(self, ui)
                });
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| {
            let worker = self.worker.take();
            let mut worker = worker.unwrap_or_else(|| render_worker::Worker::new(ctx.clone()));
            canvas::show(self, ui, &mut worker);
            self.worker = Some(worker);
            if !hidden && mobile.is_none() {
                ui.scope_builder(egui::UiBuilder::new().id_salt("toolbar"), |ui| {
                    a11y::container(ui, egui::accesskit::Role::Toolbar, i18n::tr("Tools"));
                    toolbar::show(self, ui);
                });
            }
        });
        if narrow {
            let x = if mobile == Some(MobilePanel::Left) { ctx.content_rect().max.x - MOBILE_STRIP + 4.0 } else { ctx.content_rect().min.x + 4.0 };
            self.mobile_buttons(&ctx, egui::pos2(x, rows.min + 8.0));
        }
        // The panels' edges, over the canvas and the panels (drawn after them: they take the press).
        for (name, x, shown, grows) in edges {
            if let Some(w) = Self::panel_edge(ui, name, x, rows, shown, grows) {
                let (l, r) = if name == "left" { (w, self.ui.right_width) } else { (self.ui.left_width, w) };
                self.set_panel_widths(l, r);
                self.panels_unsaved = true;
            }
        }
        // Kept once the drag is over (not on every pointer move).
        if self.panels_unsaved && !ctx.input(|i| i.pointer.any_down()) {
            self.panels_unsaved = false;
            self.services.save_prefs(&self.session.prefs);
        }
        // Before the shortcuts: keys the hovered pane's prototype uses don't reach the editor, and
        // the arrows step through comment threads with the Comment tool.
        present::show_pane(self, &ctx);
        comments::keys(self, &ctx);
        dev_compare::keys(self, &ctx);
        branch_ui::keys(self, &ctx);
        live::dialog_keys(self, &ctx);
        dialogs::keys(self, &ctx);
        shortcuts::handle_input(self, &ctx, typing, swallowed, ime_input, field_save);
        menus::popups(self, &ctx);
        variable_ui::picker(self, &ctx);
        style_picker::show(self, &ctx);
        self.end_unwanted_text_preview(&ctx);
        self.end_pen_outside_design();
        dialogs::show(self, &ctx);
        updates_ui::show(self, &ctx);
        self.toast_ui(&ctx);
        // A rename started after the layers panel drew (Cmd/Ctrl+R, a double-click) opens its field
        // next frame.
        if self.ui.rename.is_some_and(|r| r.start) && !hidden && !self.ui.left_collapsed && self.ui.left_tab == LeftTab::File {
            ctx.request_repaint();
        }
        a11y::end(self, &ctx);
        self.field_focus = a11y::field_focused(&ctx);
        windows::requests(self, &ctx);
        // Your pointer, selection and view to the others in each live session (when they
        // changed), from the window with its file.
        let windows = &self.windows;
        self.session.live_sync_in(|uid| uid.map_or(0, |u| windows.of(u)) == windows.current);
        // The canvas editor's IME context is observed exactly once per pass, after every command
        // this pass ran (shortcuts, menus, dialogs, window requests and live sync). A focused panel
        // field or an open palette owns the platform IME instead; the early `home`/present paths
        // above make the same single idle observation.
        self.observe_ime(&ctx, !typing && !self.field_focus && self.ui.palette.is_none());
    }
}
