//! FigCraft's half of Open and Save against cloud storage.
//!
//! The pages themselves are `craft_ui::cloud`. What is here is what only
//! FigCraft can answer: `.figcraft` bytes through `file.open` / `save_bytes`.
//! FigCraft holds no provider credential. Every call goes to CollabCraft.

use std::collections::HashMap;
use std::sync::Arc;

pub use craft_cloud::{Capability, Entry, EntryKind};
pub use craft_ui::cloud::{
    Account, Busy, CloudApp, CloudBackend, CloudFiles, Conflict, CopyFormat, Destination, DocumentKind, Event, Fault, Folder, Job, Link, Origin,
    Post, Source, UNAVAILABLE, available, browse, can_converse, conflict_modal, destination_name, has_target, link, load_shared, open_entry,
    open_page, origin, parent_of, prefers_cloud, progress, pump, reload_active, save, save_as, save_page, share_button, shares, sign_in, sign_out,
    start, who, work_together,
};
use egui::{Color32, Painter, Rect, RichText, Ui};
use figcraft_doc::Document;
use serde_json::json;

use crate::FigApp;
use crate::icons::{self, Icon};
use crate::theme::{Tokens, semibold};

const OPENS: [&str; 2] = ["figcraft", "fig"];
const MIMES: [&str; 1] = ["application/vnd.figcraft+zip"];

/// What a FigCraft document is, as a cloud file list needs to know.
pub const DESIGN: DocumentKind = DocumentKind { extensions: &OPENS, mimes: &MIMES, noun: "file", default_extension: "figcraft" };

const COPIES: [(&str, &str); 2] = [("file.export", "Export…"), ("file.exportPdf", "Export as PDF…")];

/// The document snapshot each save in flight encoded.
pub type Sent = HashMap<u64, Arc<Document>>;

/// Whether FigCraft can open this entry.
pub fn opens(entry: &Entry) -> bool {
    craft_ui::cloud::opens(entry, DESIGN)
}

/// The Share menu's export half.
pub fn copies() -> Vec<CopyFormat> {
    COPIES.iter().map(|(id, label)| CopyFormat::new(*id, *label)).collect()
}

fn service_host() -> String {
    let from_env = ["FIGCRAFT_CLOUD_URL", "CRAFT_CLOUD_URL", "COLLABCRAFT_URL"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .find(|value| !value.is_empty())
        .map(|value| value.trim_start_matches("https://").trim_start_matches("http://").to_owned())
        .unwrap_or_default();
    if !from_env.is_empty() {
        return from_env;
    }
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(host) = web_sys::window().and_then(|w| w.location().host().ok()).filter(|h| !h.is_empty()) {
            return host;
        }
    }
    String::new()
}

impl CloudApp for FigApp {
    fn cloud(&self) -> &CloudFiles {
        &self.cloud
    }

    fn cloud_mut(&mut self) -> &mut CloudFiles {
        &mut self.cloud
    }

    fn kind(&self) -> DocumentKind {
        DESIGN
    }

    fn active(&self) -> Option<u64> {
        self.session.docs.get(self.session.active).map(|d| d.uid)
    }

    fn title(&self, uid: u64) -> String {
        self.session.docs.iter().find(|d| d.uid == uid).map(|d| d.doc.name.clone()).unwrap_or_default()
    }

    fn open_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<Option<u64>, String> {
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        let title = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        let file_name = if name.contains('.') { name.to_owned() } else { format!("{title}.figcraft") };
        self.run("file.open", json!({"name": file_name, "data": encoded})).ok_or_else(|| format!("could not open {name}"))?;
        // A cloud open has no path on this computer.
        if let Some(ds) = self.session.docs.get_mut(self.session.active) {
            ds.path = None;
        }
        self.ui.home = false;
        Ok(self.session.docs.get(self.session.active).map(|d| d.uid))
    }

    fn encode(&mut self, uid: u64, into: Destination<'_>) -> Result<Vec<u8>, String> {
        let extension = into.extension();
        if !extension.is_empty() && extension != DESIGN.default_extension && extension != "fig" {
            return Err(format!("FigCraft writes .{}. Save a copy instead.", DESIGN.default_extension));
        }
        let doc = self.session.docs.iter().find(|d| d.uid == uid).ok_or("no file open")?;
        let snapshot = doc.doc.clone();
        let bytes = figcraft_format::save_bytes(&snapshot, None).map_err(|e| e.to_string())?;
        self.cloud_sent.insert(uid, snapshot);
        Ok(bytes)
    }

    fn close(&mut self, uid: u64) {
        if let Some(index) = self.session.docs.iter().position(|d| d.uid == uid) {
            let _ = self.session.execute("file.close", &json!({"index": index, "force": true}));
        }
    }

    fn mark_saved(&mut self, uid: u64) {
        let Some(snapshot) = self.cloud_sent.remove(&uid) else { return };
        if let Some(ds) = self.session.docs.iter_mut().find(|d| d.uid == uid) {
            // Keep in-flight edits. Mark clean only when the live document is still the
            // snapshot that was sent.
            if Arc::ptr_eq(&ds.doc, &snapshot) || *ds.doc == *snapshot {
                ds.mark_saved();
            }
        }
    }

    fn drop_pending(&mut self, uid: u64) {
        self.cloud_sent.remove(&uid);
        if self.cloud.sharing_save {
            self.cloud.sharing_save = false;
            self.share.save_failed("the file was not saved");
        }
    }

    fn sharing(&mut self) -> &mut craft_ui::share::ShareDialog {
        &mut self.share
    }

    fn presence(&mut self) -> &mut craft_ui::presence::PresenceBar {
        &mut self.presence
    }

    fn paint_icon(&self, painter: &Painter, rect: Rect, kind: EntryKind, colour: Color32) {
        let icon = if kind == EntryKind::Folder { Icon::Library } else { Icon::Page };
        icons::paint(painter, rect, icon, colour);
    }

    fn open_url(&self, url: &str) {
        if let Some(ctx) = &self.ui_context {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    fn say(&mut self, text: String) {
        self.show_error(text);
    }

    fn toast(&mut self, text: String) {
        FigApp::toast(self, text);
    }

    fn app_name(&self) -> &str {
        "FigCraft"
    }

    fn tr(&self, text: &str) -> String {
        crate::i18n::tr(text).to_owned()
    }

    fn service(&self) -> String {
        service_host()
    }

    fn save_note(&self) -> Option<String> {
        Some(self.tr("The file is saved as a FigCraft file (.figcraft) in the folder above."))
    }

    fn style(&self, ui: &Ui) -> craft_ui::cloud::CloudStyle {
        let t = Tokens::get(ui.ctx());
        craft_ui::cloud::CloudStyle {
            surface: t.panel,
            border: t.border,
            text: t.text,
            dim: t.text_secondary,
            hover: t.hover,
            soft: t.hover,
            family: egui::FontFamily::Proportional,
            bold: egui::FontFamily::Proportional,
        }
    }
}

/// The Cloud section of Home ▸ Online. The live-session list stays under it
/// when this computer actually has files on a FigCraft server.
pub fn open_section(app: &mut FigApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(crate::i18n::tr("Cloud")).font(semibold(15.0)).color(t.text));
    ui.add_space(8.0);
    start(app);
    open_page(app, ui);
}

/// Answers from the cloud worker. Called once a frame.
pub fn poll(app: &mut FigApp, ctx: &egui::Context) {
    pump(app, ctx);
    take_account_name(app);
}

/// Put the signed-in name on comments and the people bar, instead of "You".
fn take_account_name(app: &mut FigApp) {
    let Some(display) = app.cloud.account.as_ref().map(|a| a.display.trim().to_owned()).filter(|s| !s.is_empty()) else {
        return;
    };
    if app.session.prefs.user_name == display {
        return;
    }
    let _ = app.run("pref.userName", json!({"name": display}));
}

/// Carry out a save the share dialog asked for.
pub fn share_save(app: &mut FigApp) {
    if let Some(request) = app.share.take_save_request() {
        let _ = request.destination;
        match save_as(app, "/", &request.name) {
            Ok(_) => app.cloud.sharing_save = true,
            Err(reason) => app.share.save_failed(reason),
        }
    }
    if app.cloud.sharing_save && app.cloud.busy.is_none() {
        app.cloud.sharing_save = false;
        match origin(app) {
            Some(origin) => {
                let (id, name) = (origin.entry.id.clone(), origin.entry.name.clone());
                app.share.saved(id, name);
            }
            None => {
                let reason = app.cloud.status.clone().unwrap_or_else(|| "the file was not saved".to_owned());
                app.share.save_failed(reason);
            }
        }
    }
}
