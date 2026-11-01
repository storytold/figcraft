use super::{FigApp, shortcuts};

impl FigApp {
    /// The pass's one IME observation: the focused on-canvas editor, or an explicit idle.
    ///
    /// Calling `ImeContextTracker::observe` more than once in a pass would treat an idle followed
    /// by a focused observation as a context change (dropping the frame's receipts) and bump the
    /// generation. Keeping this in one place makes the fork contract structural.
    pub(super) fn observe_ime(&mut self, ctx: &egui::Context, focused: bool) {
        #[cfg(test)]
        {
            self.ime_observations = self.ime_observations.saturating_add(1);
        }
        if focused {
            shortcuts::ime_output(self, ctx);
        } else {
            let _ = self.ime.observe(ctx.viewport_id(), None);
        }
    }
}
