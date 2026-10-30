use super::{
    FigApp, PendingAck, Value, cursors, dev_compare, home_recents, import_report, json, live, missing_fonts, online, present, tabs, windows,
};

impl FigApp {
    /// Queue raw input injected through the control channel for the window drawing next, tagged
    /// with the document it belongs to (a batch is dropped if another file is active by then).
    pub(crate) fn push_control_input(&mut self, events: Vec<egui::Event>) -> bool {
        if events.is_empty() {
            return false;
        }
        let doc = self.session.docs.get(self.session.active).map(|d| d.uid);
        if !self.control_input.is_empty() && self.control_input_doc != doc {
            // A queued batch belongs to the file that was open when it was injected; it cannot be
            // merged with one for the file showing now. Its acknowledgement says so.
            self.control_input.clear();
            let viewport = crate::windows::viewport(self.windows.current);
            crate::windows::fail_input_acks(self, viewport);
        }
        // Always the live pane vector: while a child window draws, `windows::enter` has swapped
        // its pane into the app, and `leave` swaps these events back into that pane.
        self.control_input.extend(events);
        self.control_input_doc = doc;
        self.control_synthetic = true;
        true
    }

    /// The acknowledgements waiting for `viewport`'s input batch (other windows keep theirs).
    pub(crate) fn take_input_acks(&mut self, viewport: egui::ViewportId) -> Vec<PendingAck> {
        let mut mine = Vec::new();
        for ack in std::mem::take(&mut self.input_acks) {
            if ack.viewport == viewport {
                mine.push(ack);
            } else {
                self.input_acks.push(ack);
            }
        }
        mine
    }

    /// UI state as JSON (control channel `ui.inspect`).
    pub fn inspect(&self) -> Value {
        let mut state = json!({
            "rightTab": self.ui.right_tab,
            "leftTab": self.ui.left_tab,
            "home": self.ui.home,
            "online": online::inspect(self),
            "tabs": self.session.docs.iter().map(|d| json!({"name": d.doc.name, "live": tabs::live_mark(self, d.uid).map(|shared| if shared { "shared" } else { "private" })})).collect::<Vec<_>>(),
            "homeSection": self.ui.home_view.section.id(),
            "homeRecents": home_recents::inspect(self),
            "dark": self.ui.dark,
            "focus": {"region": self.ui.nav.region.id(), "keyboard": self.ui.nav.keyboard},
            "reduceMotion": self.reduce_motion(),
            "presenting": self.ui.present.is_some(),
            "presenter": tabs::shown_or_paused(self).map(present::Presenter::inspect),
            "presentOptions": {"scale": self.ui.present_view.scale.id(), "deviceFrame": self.ui.present_view.device_frame, "comments": self.ui.present_view.comments, "hotspotHints": self.ui.present_view.hotspot_hints, "inspect": self.ui.present_view.inspect},
            "preview": self.ui.preview.as_ref().map(|p| p.player.inspect()),
            "mainMenu": self.ui.main_menu,
            "palette": self.ui.palette.is_some(),
            "variables": self.ui.variables.is_some(),
            "versionHistory": self.ui.versions.open,
            "branchReview": self.ui.branch_review.as_ref().map(|r| json!({"id": r.id, "picks": r.picks, "mode": if r.mode == dev_compare::Mode::Overlay { "overlay" } else { "sideBySide" }})),
            "devCompare": self.ui.dev.compare.as_ref().map(|c| json!({"id": c.id, "mode": if c.mode == dev_compare::Mode::Overlay { "overlay" } else { "sideBySide" }, "split": c.split, "version": c.version})),
            "devFocus": self.session.ds().ok().and_then(figcraft_engine::DocState::focused),
            "annotationFilter": self.ui.dev.categories.id(),
            "colorProfileDialog": self.ui.color_profile.map(|p| p.id()),
            "stylePicker": self.ui.style_picker.as_ref().map(|p| p.slot().key()),
            "stylePickerView": if self.ui.style_grid { "grid" } else { "list" },
            "textPreview": figcraft_engine::textpreview::active(&self.session),
            "comments": {
                "open": self.ui.comments.open,
                "draft": self.ui.comments.draft.as_ref().map(|d| [d.0, d.1]),
                "filter": self.ui.comments.filter.id(),
                "sort": self.ui.comments.sort.id(),
                "search": self.ui.comments.search,
                "editing": self.ui.comments.editing.as_ref().map(|e| [e.0, e.1 as u64]),
                "mentions": self.ui.comments.mention.names(),
            },
            "frameMs": self.ui.frame_ms,
            "renderMs": self.ui.render_ms,
            "framesDrawn": self.frame_stats.frames,
            "repaintCauses": self.frame_stats.causes(),
            "canvas": {
                "rect": [self.canvas.rect.min.x, self.canvas.rect.min.y, self.canvas.rect.max.x, self.canvas.rect.max.y],
                "renderWork": self.canvas.render_work.map(|w| match w {
                    figcraft_render::Work::Reused => json!({"kind": "reused"}),
                    figcraft_render::Work::Partial { pixels } => json!({"kind": "partial", "pixels": pixels}),
                    figcraft_render::Work::Full => json!({"kind": "full"}),
                }),
                "quality": if self.canvas.tex_coarse { "preview" } else { "refined" },
                "previewLatencyMs": self.canvas.preview_latency_ms,
                "refineLatencyMs": self.canvas.refine_latency_ms,
            },
            "live": live::inspect(self),
            "window": self.windows.current,
            "windows": windows::list(self),
            "tool": self.session.tool_info(),
            "cursor": self.canvas.cursor.map(cursors::Shown::name),
            "panels": {"left": self.ui.left_width, "right": self.ui.right_width, "leftCollapsed": self.ui.left_collapsed},
            "status": self.session.status(),
            "lastError": self.ui.last_error,
            "importReport": self.ui.import_report.as_ref().map(import_report::Report::inspect),
        });
        state["homeSearch"] = json!(self.ui.home_view.file_query);
        // Set apart from the literal above, which is at the macro recursion limit.
        state["uiScale"] = json!({"choice": self.session.prefs.ui_scale, "zoom": self.ui.ui_scale.zoom, "drawn": self.ui.ui_scale.drawn, "limited": self.ui.ui_scale.limited});
        state["pendingOpens"] = json!(self.ui.pending_opens.len());
        state["missingFonts"] = json!(self.ui.missing_fonts.as_ref().map(missing_fonts::Shown::inspect));
        state["presentTabs"] = tabs::present_tabs_inspect(self);
        state["frameTimes"] = self.frame_stats.times();
        state
    }
}
