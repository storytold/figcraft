use super::FigApp;

impl FigApp {
    /// Put `text` on the system clipboard. Input injected through the control channel never
    /// replaces what the user copied: this is the one write path every copy command goes through.
    pub(crate) fn clipboard_set_text(&mut self, text: &str) {
        if self.control_synthetic {
            return;
        }
        self.services.clipboard_set_text(text);
    }

    /// [`Self::clipboard_set_text`] for an image (straight RGBA).
    pub(crate) fn clipboard_set_png(&mut self, width: u32, height: u32, rgba: &[u8]) {
        if self.control_synthetic {
            return;
        }
        self.services.clipboard_set_png(width, height, rgba);
    }
}
