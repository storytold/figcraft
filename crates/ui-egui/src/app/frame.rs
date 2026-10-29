use super::{
    Arc, CJK_CHANGES, FigApp, LEFT_PANEL, MobilePanel, PANEL_DEFAULT, RIGHT_PANEL, RightTab, TOAST_SECS, ToastClock, Value, a11y, cjk, i18n, icons,
    json, repaint_in, theme,
};

impl FigApp {
    /// Show what `command params` would do on the canvas as a live preview (`text.preview`) while
    /// a font family, font style, text style, named instance or axis slider is hovered or dragged. Call it every frame
    /// the preview should stay: it ends on the first frame nothing asks for it.
    pub fn preview_text(&mut self, ctx: &egui::Context, command: &str, params: Value) {
        self.ui.text_preview_wanted = true;
        let revision = self.session.ds().map_or(0, |d| d.revision);
        if let Some((c, p, rev)) = &self.ui.text_preview
            && c == command
            && *p == params
            && (*rev == revision || figcraft_engine::textpreview::active(&self.session))
        {
            return;
        }
        let shown = self.session.execute("text.preview", &json!({"command": command, "params": params})).is_ok();
        let revision = self.session.ds().map_or(0, |d| d.revision);
        self.ui.text_preview = shown.then(|| (command.to_string(), params, revision));
        // The canvas drew before the panels: show the preview next frame.
        ctx.request_repaint();
    }

    /// End the live text preview the panels show (Escape, or nothing hovered any more).
    pub fn end_text_preview(&mut self, ctx: &egui::Context) {
        if self.ui.text_preview.take().is_some() {
            let _ = self.session.execute("text.endPreview", &json!({}));
            ctx.request_repaint();
        }
    }

    /// Clicking the control that opened the colour picker closes it again; clicking another opens
    /// the picker for that one.
    pub fn toggle_picker(&mut self, target: crate::colorpicker::PickerTarget, button: Option<egui::Rect>) {
        if self.ui.picker.as_ref() == Some(&target) {
            crate::colorpicker::close(self);
        } else {
            self.open_picker(target, button);
        }
    }

    /// Start a fresh picker beside its control, committing edits to the previous target first.
    pub fn open_picker(&mut self, target: crate::colorpicker::PickerTarget, button: Option<egui::Rect>) {
        if self.ui.picker.is_some() {
            crate::colorpicker::close(self);
        }
        self.ui.picker_popover = None;
        self.ui.picker_button_id = None;
        self.ui.picker = Some(target);
        self.ui.picker_button = button;
    }

    /// Follow only the widget that opened this target, even when several controls edit it.
    pub fn refresh_picker_control(&mut self, target: &crate::colorpicker::PickerTarget, response: &egui::Response) {
        if self.ui.picker.as_ref() == Some(target) && self.ui.picker_button_id.is_none_or(|id| id == response.id) {
            self.ui.picker_button = response.interact_rect.is_positive().then_some(response.rect);
            self.ui.picker_button_id = Some(response.id);
        }
    }

    /// Drawn colour controls refresh their held rectangle and toggle the picker on a click.
    pub fn picker_control(&mut self, target: crate::colorpicker::PickerTarget, response: &egui::Response) {
        self.refresh_picker_control(&target, response);
        if response.clicked() {
            self.toggle_picker(target, Some(response.rect));
            if self.ui.picker.is_some() {
                self.ui.picker_button_id = Some(response.id);
            }
        }
    }

    /// Leaving Design ends a pen path as Escape does and goes back to Move: Prototype and Inspect
    /// don't draw (#307). Checked as the frame starts (the control channel) and after the panels
    /// and shortcuts (the mode switcher, the tabs, Shift+E/Shift+D), before the canvas sees more.
    pub(super) fn end_pen_outside_design(&mut self) {
        if self.ui.right_tab == self.ui.last_right_tab {
            return;
        }
        self.ui.last_right_tab = self.ui.right_tab;
        if self.ui.right_tab != RightTab::Design && self.session.tool.tool == figcraft_engine::tools::ToolId::Pen {
            self.run("tool.set", json!({"tool": "move"}));
        }
    }

    /// Once a frame, after the panels: a preview nothing asked for this frame ends.
    pub(super) fn end_unwanted_text_preview(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.ui.text_preview_wanted) {
            self.end_text_preview(ctx);
        }
    }

    /// Finish the frame that applies an injected batch: acknowledge it once the pass is presented,
    /// and stop fencing the system clipboard. Only the viewport whose batch this frame took acts.
    pub(crate) fn end_input_frame(&mut self, ctx: &egui::Context) {
        if self.applied_frame != Some(ctx.viewport_id()) {
            return;
        }
        self.applied_input = None;
        if ctx.will_discard() {
            // Keep `applied_frame`/`applied_acks`: the rerun pass must present (and acknowledge)
            // the same frame without replaying the input.
            return;
        }
        self.applied_frame = None;
        for ack in self.applied_acks.drain(..) {
            let _ = ack.tx.send(ack.reply);
        }
        self.control_synthetic = false;
    }

    /// The app's monotonic clock in seconds, also usable from logic-only callbacks (recovery,
    /// control deadlines). Never `ctx.input(i.time)`, which is stale there.
    pub fn now_secs(&self) -> f64 {
        self.clock.now()
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.ui.toast = Some((msg.into(), ToastClock::Raised));
    }

    /// Say that `n` documents left unsaved by a crash were opened again from recovery copies.
    pub fn note_recovered(&mut self, n: usize) {
        if n > 0 {
            self.toast(i18n::trn("Recovered {n} unsaved files after the app quit unexpectedly: save them to keep them", n, &[]));
        }
    }

    /// Set the widths asked for this window's side panels (within their limits); the main
    /// window's are kept in the preferences.
    pub fn set_panel_widths(&mut self, left: f32, right: f32) {
        let within = |w: f32, range: std::ops::RangeInclusive<f32>| if w.is_finite() { w.clamp(*range.start(), *range.end()) } else { PANEL_DEFAULT };
        (self.ui.left_width, self.ui.right_width) = (within(left, LEFT_PANEL), within(right, RIGHT_PANEL));
        if self.windows.current == 0 {
            self.session.prefs.panel_widths = (self.ui.left_width, self.ui.right_width);
        }
    }

    /// The edge between a side panel and the canvas, at `x` down `rows`: dragging it sizes the
    /// panel (`grows`: +1 for the left panel, whose edge is on its right, −1 for the right one), a
    /// double-click puts back the default width. Returns the width asked for, when it changed.
    pub(super) fn panel_edge(ui: &mut egui::Ui, name: &str, x: f32, rows: egui::Rangef, width: f32, grows: f32) -> Option<f32> {
        let grab = ui.style().interaction.resize_grab_radius_side;
        let rect = egui::Rect::from_x_y_ranges(x - grab..=x + grab, rows);
        let id = egui::Id::unique(("panel_edge", name));
        // Not focusable: an edge is a pointer target, so it must not take Tab/F6 focus from the
        // panels' controls (see `a11y::focusables`).
        let resp = ui.interact(rect, id, egui::Sense::CLICK | egui::Sense::DRAG);
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().vline(x, rows, egui::Stroke::new(2.0, theme::ACCENT));
        }
        if resp.double_clicked() {
            return Some(PANEL_DEFAULT);
        }
        // The edge stays under the pointer where it was grabbed (also back from past a limit).
        let at = resp.interact_pointer_pos().filter(|_| resp.dragged())?;
        if resp.drag_started() {
            let pressed = ui.input(|i| i.pointer.press_origin()).unwrap_or(at);
            ui.data_mut(|d| d.insert_temp(id, pressed.x - x));
        }
        let grabbed: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
        let w = width + (at.x - grabbed - x) * grows;
        (w != width).then_some(w)
    }

    /// Reduced motion: the preference or the system's setting.
    pub fn reduce_motion(&self) -> bool {
        self.session.prefs.reduce_motion || self.services.reduce_motion()
    }

    /// The CJK, Bengali and Thai scripts the UI shows: the language's, the language picker's (Preferences open),
    /// typed text's, and those in the open file's names and comments (read when it changes, not
    /// mid-drag). Nothing is checked once every script has a face.
    pub(crate) fn cjk_needed(&mut self, ctx: &egui::Context, installed: u8) -> u8 {
        if installed == cjk::ALL {
            return installed;
        }
        let mut need = i18n::current().map_or(0, |l| cjk::language(l.code));
        if self.ui.preferences {
            need |= i18n::languages().iter().filter(|l| cjk::can_show(l.code)).fold(0, |m, l| m | cjk::language(l.code));
        }
        need |= ctx.input(|i| {
            i.events.iter().fold(0, |m, e| match e {
                egui::Event::Text(t) | egui::Event::Ime(egui::ImeEvent::Preedit { text: t, .. } | egui::ImeEvent::Commit(t)) => m | cjk::scripts(t),
                _ => m,
            })
        });
        let dragging = ctx.input(|i| i.pointer.any_down());
        // Pans and zooms change the revision too: only a new version of the document is looked
        // at, and only the layers it changed (a large file has hundreds of thousands of names).
        let current = self.session.docs.get(self.session.active).filter(|_| !dragging).map(|d| (d.uid, d.doc.clone()));
        if let Some((uid, doc)) = current {
            let before = self.cjk_checked.take().filter(|(u, _)| *u == uid).map(|(_, b)| b);
            if !before.as_ref().is_some_and(|b| Arc::ptr_eq(b, &doc)) {
                let mut scan = |s: &str| need |= cjk::scripts(s);
                scan(&doc.name);
                match before.as_ref().and_then(|b| doc.changed_ids(b, CJK_CHANGES)) {
                    Some(ids) => ids.into_iter().filter_map(|id| doc.node(id)).for_each(|n| scan(&n.name)),
                    None => doc.nodes().for_each(|n| scan(&n.name)),
                }
                doc.comments.iter().flat_map(|c| std::iter::once(&c.first).chain(&c.replies)).for_each(|m| {
                    scan(&m.author);
                    scan(&m.text);
                });
            }
            self.cjk_checked = Some((uid, doc));
        }
        need
    }

    /// Whether the right panel shows: never with the UI `hidden`; always on a design file; on a
    /// board, which has no prototypes or Dev Mode (its tab stays Design), only when asked for.
    pub(super) fn right_panel_shown(&mut self, hidden: bool) -> bool {
        let board = self.session.doc().is_ok_and(figcraft_engine::board::is_board);
        if board {
            self.ui.right_tab = RightTab::Design;
        }
        !hidden && (!board || self.ui.board_panel)
    }

    /// A narrow window's two buttons for the side panels (touch-sized); the open one closes again.
    pub(super) fn mobile_buttons(&mut self, ctx: &egui::Context, at: egui::Pos2) {
        use a11y::Tip as _;
        let t = theme::Tokens::get(ctx);
        egui::Area::new(egui::Id::unique("mobile_panel_buttons")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
            for (side, icon, name) in
                [(MobilePanel::Left, icons::Icon::Layers, i18n::tr("Layers")), (MobilePanel::Right, icons::Icon::Panels, i18n::tr("Design"))]
            {
                let open = self.ui.mobile_panel == Some(side);
                let (r, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                let resp = resp.tip(name);
                let fill = if open { t.accent } else { t.panel };
                ui.painter().rect(r, 8.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                icons::paint(ui.painter(), r.shrink(10.0), icon, if open { t.accent_on } else { t.icon });
                if resp.clicked() {
                    if side == MobilePanel::Left && !open {
                        self.ui.open_left_panel();
                    } else {
                        self.ui.mobile_panel = if open { None } else { Some(side) };
                    }
                }
                ui.add_space(4.0);
            }
        });
    }

    /// Draw the toast: start its lifetime on the frame **after** the first drawn frame, request
    /// that frame and the frame that hides it, expire and remove it at the deadline, and place it
    /// at the bottom centre.
    pub(super) fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((msg, clock)) = &mut self.ui.toast else { return };
        let now = ctx.input(|i| i.time);
        // The clock starts on the frame after the one that first draws the toast: that frame's
        // time was taken before whatever raised it ran (a file dialog, a long open), and would
        // count that wait against it, so a failed open's message expired before it was seen.
        let left = match *clock {
            ToastClock::Raised => {
                *clock = ToastClock::Drawn;
                ctx.request_repaint();
                TOAST_SECS
            }
            ToastClock::Drawn => {
                *clock = ToastClock::Since(now);
                TOAST_SECS
            }
            ToastClock::Since(t) => TOAST_SECS - (now - t),
        };
        if left <= 0.0 {
            self.ui.toast = None;
            return;
        }
        let msg = msg.clone();
        // Nothing moves while it shows: the next frame it needs is the one that hides it.
        if matches!(clock, ToastClock::Since(_)) {
            repaint_in(ctx, left);
        }
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("toast"))
            .fixed_pos(egui::pos2(screen.center().x, screen.max.y - 96.0))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(0x1E, 0x1E, 0x1E))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        // Wrapped at a readable width (the area alone would wrap at a sliver).
                        let width = (screen.width() - 64.0).clamp(120.0, 480.0);
                        let galley = ui.painter().layout(msg, theme::regular(12.0), egui::Color32::WHITE, width);
                        ui.label(galley);
                    });
            });
    }
}
