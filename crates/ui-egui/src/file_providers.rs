//! Optional same-origin storage capability. No provider is enabled by default.
//!
//! Requests own immutable native project captures. A host result never selects a live tab,
//! revision, filesystem path or account. This module contains no storage or authentication.
use figcraft_engine::DocState;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

const MAX_BYTES: usize = 80 * 1024 * 1024;
const MAX_PENDING: usize = 16;
const MAX_EVENTS: usize = 64;
const MAX_RESOURCES: usize = 100;
const EXTENSION: &str = ".figcraft";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resource {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub updated_at: String,
}
pub struct Event {
    pub metadata: Value,
    pub bytes: Option<Vec<u8>>,
}
#[derive(Clone, PartialEq, Eq)]
struct Identity {
    provider_id: String,
    label: String,
    namespace: String,
}
/// The exact states encoded at Save, including a main document's branch tabs where applicable.
pub(crate) struct Capture {
    pub doc: u64,
    pub revision: u64,
    pub snapshots: Vec<DocState>,
    pub name: String,
    intent: String,
    destination: String,
    bytes: Option<Vec<u8>>,
}
pub(crate) enum Action {
    Save { request: String, capture: Capture, status: String, resource: String },
    Open { request: String, resource: String, name: String, bytes: Vec<u8> },
}
#[derive(Clone, Copy)]
enum TerminalOwner {
    Open,
    Save,
}
#[derive(Clone)]
struct Terminal {
    request: String,
    metadata: Value,
    owner: TerminalOwner,
}
#[derive(Default)]
struct State {
    provider: Option<Identity>,
    enabled: bool,
    nonce: String,
    serial: u64,
    connection: String,
    resources: Vec<Resource>,
    bindings: HashMap<u64, String>,
    names: HashMap<u64, String>,
    dialogs: HashMap<String, Capture>,
    pending: HashMap<String, Capture>,
    // Keep every captured document alias owned until its terminal event can be queued.
    finishing_saves: HashMap<String, Vec<u64>>,
    opens: HashMap<String, String>,
    queued_opens: HashSet<String>,
    actions: Vec<Action>,
    events: Vec<Event>,
    // At most MAX_PENDING saves plus MAX_PENDING reads; never stores transfer bytes.
    finished: VecDeque<Terminal>,
    documents: Value,
    open_error: Option<String>,
}
impl State {
    fn busy(&self) -> bool {
        !self.dialogs.is_empty()
            || !self.pending.is_empty()
            || !self.finishing_saves.is_empty()
            || !self.opens.is_empty()
            || !self.actions.is_empty()
            || !self.finished.is_empty()
    }
    fn next(&mut self, kind: &str) -> Result<String, String> {
        if self.nonce.is_empty() {
            return Err("file provider session is not initialized".into());
        }
        self.serial = self.serial.checked_add(1).ok_or("file provider request counter exhausted")?;
        Ok(format!("{}-{kind}-{}", self.nonce, self.serial))
    }
    fn event(&mut self, mut metadata: Value, bytes: Option<Vec<u8>>) {
        if let (Some(provider), Some(fields)) = (&self.provider, metadata.as_object_mut()) {
            fields.insert("providerId".into(), provider.provider_id.clone().into());
            fields.insert("namespace".into(), provider.namespace.clone().into());
            fields.insert("connectionId".into(), self.connection.clone().into());
        }
        self.events.push(Event { metadata, bytes });
    }
    fn retained_bytes(&self) -> usize {
        let captures = self.dialogs.values().chain(self.pending.values()).map(|p| p.bytes.as_ref().map_or(0, Vec::len));
        let events = self.events.iter().map(|e| e.bytes.as_ref().map_or(0, Vec::len));
        let actions = self.actions.iter().map(|a| match a {
            Action::Open { bytes, .. } => bytes.len(),
            _ => 0,
        });
        captures.chain(events).chain(actions).fold(0, usize::saturating_add)
    }
    fn pending_for(&self, doc: u64) -> bool {
        self.dialogs.values().chain(self.pending.values()).any(|c| c.snapshots.iter().any(|d| d.uid == doc))
            || self.finishing_saves.values().any(|aliases| aliases.contains(&doc))
            || self.actions.iter().any(|a| matches!(a, Action::Save { capture, .. } if capture.snapshots.iter().any(|d| d.uid == doc)))
    }
    fn flush_finished(&mut self) {
        while self.events.len() < MAX_EVENTS {
            let Some(terminal) = self.finished.pop_front() else { break };
            self.event(terminal.metadata, None);
            match terminal.owner {
                TerminalOwner::Open => {
                    self.opens.remove(&terminal.request);
                    self.queued_opens.remove(&terminal.request);
                }
                TerminalOwner::Save => {
                    self.finishing_saves.remove(&terminal.request);
                }
            }
        }
    }
}
#[derive(Clone, Default)]
pub struct Host(Arc<Mutex<State>>);
impl Host {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn set_session(&self, nonce: String) -> Result<(), String> {
        let mut s = self.state();
        if !s.nonce.is_empty() || s.provider.is_some() || !valid_id(&nonce) || nonce.len() < 16 {
            return Err("invalid or already initialized file provider session".into());
        }
        s.nonce = nonce;
        Ok(())
    }
    pub fn connect(&self, provider_id: String, label: String, namespace: String) -> Result<String, String> {
        if !valid_id(&provider_id) || !valid_label(&label) || !valid_id(&namespace) {
            return Err("invalid file provider identity".into());
        }
        let mut s = self.state();
        let identity = Identity { provider_id, label, namespace };
        if let Some(current) = &s.provider {
            return if *current == identity { Ok(s.connection.clone()) } else { Err("unregister before changing provider or account".into()) };
        }
        s.connection = s.next("connection")?;
        s.provider = Some(identity);
        s.enabled = true;
        Ok(s.connection.clone())
    }
    pub fn disconnect(&self) -> Result<(), String> {
        let mut s = self.state();
        if s.busy() {
            return Err("finish or cancel pending provider operations before unregistering".into());
        }
        s.provider = None;
        s.enabled = false;
        s.connection.clear();
        s.resources.clear();
        s.bindings.clear();
        s.names.clear();
        s.events.clear();
        s.documents = Value::Null;
        s.open_error = None;
        Ok(())
    }
    pub fn connected(&self) -> bool {
        self.state().provider.is_some()
    }
    pub fn enabled(&self) -> bool {
        let s = self.state();
        s.provider.is_some() && s.enabled
    }
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let mut s = self.state();
        if s.provider.is_none() {
            return Err("file provider is not connected".into());
        }
        s.enabled = enabled;
        Ok(())
    }
    pub fn busy(&self) -> bool {
        self.state().busy()
    }
    pub fn pending_for(&self, doc: u64) -> bool {
        self.state().pending_for(doc)
    }
    /// Pausing new choices does not release an external checkpoint or an in-flight save.
    pub(crate) fn owns_document(&self, doc: u64) -> bool {
        let s = self.state();
        s.bindings.contains_key(&doc) || s.pending_for(doc)
    }
    /// A durable overwrite supersedes every previous open owner of this resource.
    pub(crate) fn unbind_resource_aliases(&self, doc: u64, resource: &str) -> Vec<u64> {
        let mut s = self.state();
        let aliases: Vec<_> = s.bindings.iter().filter(|(id, bound)| **id != doc && bound.as_str() == resource).map(|(id, _)| *id).collect();
        for alias in &aliases {
            s.bindings.remove(alias);
            s.names.remove(alias);
        }
        aliases
    }
    pub fn bound_document(&self, resource: &str) -> Option<u64> {
        self.state().bindings.iter().find(|(_, r)| r.as_str() == resource).map(|(d, _)| *d)
    }
    pub fn bound_filename(&self, doc: u64) -> Option<String> {
        self.state().names.get(&doc).cloned()
    }
    /// Captures are produced synchronously by the editor before this event is issued.
    pub(crate) fn begin(
        &self,
        snapshots: Vec<DocState>,
        doc: u64,
        revision: u64,
        name: String,
        bytes: Vec<u8>,
        save_as: bool,
    ) -> Result<String, String> {
        let mut s = self.state();
        if s.provider.is_none()
            || !s.enabled
            || !valid_native_name(&name)
            || bytes.is_empty()
            || bytes.len() > MAX_BYTES
            || s.retained_bytes().saturating_add(bytes.len()) > MAX_BYTES
            || s.pending_for(doc)
            || s.dialogs.len() + s.pending.len() + s.finishing_saves.len() >= MAX_PENDING
            || s.events.len() >= MAX_EVENTS
            || !snapshots.iter().any(|snapshot| snapshot.uid == doc)
        {
            return Err("provider unavailable, a save is pending, or capture exceeds 80 MiB".into());
        }
        let request = s.next("dialog")?;
        let intent = if save_as { "saveAs" } else { "save" }.to_string();
        s.event(json!({"type":"saveDialog","requestId":request,"documentId":doc.to_string(),"revision":revision.to_string(),"intent":intent,"filename":name}), None);
        s.dialogs.insert(request.clone(), Capture { doc, revision, snapshots, name, intent, destination: String::new(), bytes: Some(bytes) });
        Ok(request)
    }
    pub fn resolve_dialog(&self, request: &str, destination: &str, name: &str) -> Result<(), String> {
        if !matches!(destination, "provider" | "device" | "cancel") || (destination != "cancel" && !valid_native_name(name)) {
            return Err("choose provider, device or cancel and a native project filename".into());
        }
        let mut s = self.state();
        if s.events.len() >= MAX_EVENTS {
            return Err("provider event queue is full".into());
        }
        let mut capture = s.dialogs.remove(request).ok_or("unknown or completed save dialog")?;
        if destination == "cancel" {
            capture.bytes = None;
            s.finishing_saves.insert(request.into(), capture.snapshots.iter().map(|doc| doc.uid).collect());
            s.actions.push(Action::Save { request: request.into(), capture, status: "cancel".into(), resource: String::new() });
            return Ok(());
        }
        capture.name = name.into();
        capture.destination = destination.into();
        let resource = if capture.intent == "save" { s.bindings.get(&capture.doc).cloned() } else { None };
        let bytes = capture.bytes.take();
        s.event(json!({"type":"save","requestId":request,"documentId":capture.doc.to_string(),"revision":capture.revision.to_string(),"intent":capture.intent,"filename":name,"resourceId":resource,"destination":destination}), bytes);
        s.pending.insert(request.into(), capture);
        Ok(())
    }
    pub fn result(&self, request: String, status: String, resource: String) -> Result<(), String> {
        if !matches!(status.as_str(), "saved" | "download" | "failed" | "cancel") || (!resource.is_empty() && !valid_id(&resource)) {
            return Err("invalid provider result".into());
        }
        let mut s = self.state();
        let capture = s.pending.get(&request).ok_or("unknown or completed save request")?;
        if (status == "saved" && (capture.destination != "provider" || !valid_id(&resource)))
            || (status == "download" && capture.destination != "device")
        {
            return Err("result does not match the chosen destination".into());
        }
        if status == "saved" && capture.intent == "saveAs" && s.bindings.get(&capture.doc) == Some(&resource) {
            return Err("Save As must produce a distinct resource".into());
        }
        if s.actions.len() >= MAX_PENDING {
            return Err("provider command queue is full".into());
        }
        let capture = s.pending.remove(&request).ok_or("unknown save request")?;
        s.finishing_saves.insert(request.clone(), capture.snapshots.iter().map(|doc| doc.uid).collect());
        s.actions.push(Action::Save { request, capture, status, resource });
        Ok(())
    }
    pub fn set_resources(&self, connection: &str, resources: Vec<Resource>) -> Result<(), String> {
        let mut seen = HashSet::new();
        if resources.len() > MAX_RESOURCES
            || resources.iter().any(|r| !valid_id(&r.id) || !valid_native_name(&r.name) || r.updated_at.len() > 100 || !seen.insert(r.id.clone()))
        {
            return Err("invalid native project resource list".into());
        }
        let mut s = self.state();
        if s.provider.is_none() || s.connection != connection {
            return Err("resource list belongs to another connection".into());
        }
        s.resources = resources;
        Ok(())
    }
    pub fn request_open(&self, resource: &str) -> Result<String, String> {
        let mut s = self.state();
        if !s.enabled
            || s.provider.is_none()
            || s.opens.len() >= MAX_PENDING
            || s.events.len() >= MAX_EVENTS
            || !s.resources.iter().any(|r| r.id == resource)
        {
            return Err("resource is unavailable in the current provider account".into());
        }
        let request = s.next("open")?;
        s.opens.insert(request.clone(), resource.into());
        s.event(json!({"type":"openRequested","requestId":request,"resourceId":resource}), None);
        Ok(request)
    }
    pub fn open(&self, request: String, name: String, bytes: Vec<u8>) -> Result<(), String> {
        if !valid_native_name(&name) || bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("invalid native project resource".into());
        }
        let mut s = self.state();
        if s.actions.len() >= MAX_PENDING || s.retained_bytes().saturating_add(bytes.len()) > MAX_BYTES {
            return Err("provider command queue is full".into());
        }
        if s.queued_opens.contains(&request) {
            return Err("open response already queued".into());
        }
        let resource = s.opens.get(&request).cloned().ok_or("unknown or completed open request")?;
        s.queued_opens.insert(request.clone());
        s.actions.push(Action::Open { request, resource, name, bytes });
        Ok(())
    }
    pub fn cancel_open(&self, request: String, error: String) -> Result<(), String> {
        if error.len() > 1024 {
            return Err("invalid open result".into());
        }
        let mut s = self.state();
        if s.queued_opens.contains(&request) {
            return Err("open response already queued".into());
        }
        if s.events.len() >= MAX_EVENTS {
            return Err("provider event queue is full".into());
        }
        let resource = s.opens.remove(&request).ok_or("unknown or completed open request")?;
        s.event(json!({"type":if error.is_empty(){"openCancelled"}else{"openFailed"},"requestId":request,"resourceId":resource,"error":error}), None);
        Ok(())
    }
    pub(crate) fn take_actions(&self) -> Vec<Action> {
        std::mem::take(&mut self.state().actions)
    }
    pub fn take_events(&self) -> Vec<Event> {
        let mut s = self.state();
        let events = std::mem::take(&mut s.events);
        s.flush_finished();
        if events.is_empty() { std::mem::take(&mut s.events) } else { events }
    }
    pub fn snapshot(&self) -> Value {
        let s = self.state();
        json!({"protocolVersion":1,"enabled":s.enabled && s.provider.is_some(),"provider":s.provider.as_ref().map(|p|json!({"id":p.provider_id,"label":p.label,"namespace":p.namespace})),"connectionId":s.connection,"resources":s.resources,"documents":s.documents,"pending":s.busy()})
    }
    pub(crate) fn refresh(&self, docs: Vec<(u64, u64, String, bool)>) {
        let mut s = self.state();
        let live: HashSet<_> = docs.iter().map(|d| d.0).collect();
        s.bindings.retain(|id, _| live.contains(id));
        s.names.retain(|id, _| live.contains(id));
        s.documents = json!(docs.into_iter().map(|(id,rev,name,dirty)|json!({"id":id.to_string(),"revision":rev.to_string(),"name":name,"dirty":dirty,"resourceId":s.bindings.get(&id)})).collect::<Vec<_>>());
    }
    pub(crate) fn finish_save(&self, request: &str, capture: &Capture, status: &str, resource: &str, alive: bool) {
        let mut s = self.state();
        if !s.finishing_saves.get(request).is_some_and(|aliases| aliases.contains(&capture.doc))
            || s.finished.iter().any(|terminal| terminal.request == request)
        {
            return;
        }
        if status == "saved" {
            if alive {
                s.bindings.insert(capture.doc, resource.into());
                s.names.insert(capture.doc, capture.name.clone());
            }
            s.resources.retain(|r| r.id != resource);
            s.resources.insert(0, Resource { id: resource.into(), name: capture.name.clone(), updated_at: String::new() });
            s.resources.truncate(MAX_RESOURCES);
        }
        let metadata = json!({"type":match status { "saved"=>"saved", "download"=>"downloaded", "cancel"=>"saveCancelled", _=>"saveFailed" },"requestId":request,"documentId":capture.doc.to_string(),"revision":capture.revision.to_string(),"resourceId":resource,"filename":capture.name,"documentOpen":alive});
        s.finished.push_back(Terminal { request: request.into(), metadata, owner: TerminalOwner::Save });
        s.flush_finished();
    }
    pub(crate) fn finish_open(&self, request: &str, resource: &str, name: &str, result: Result<u64, String>) {
        let mut s = self.state();
        if !s.opens.get(request).is_some_and(|owned| owned == resource)
            || !s.queued_opens.contains(request)
            || s.finished.iter().any(|terminal| terminal.request == request)
        {
            return;
        }
        let metadata = match result {
            Ok(doc) => {
                s.bindings.insert(doc, resource.into());
                s.names.insert(doc, name.into());
                json!({"type":"opened","requestId":request,"resourceId":resource,"documentId":doc.to_string()})
            }
            Err(error) => json!({"type":"openFailed","requestId":request,"resourceId":resource,"error":bounded_error(&error)}),
        };
        s.finished.push_back(Terminal { request: request.into(), metadata, owner: TerminalOwner::Open });
        s.flush_finished();
    }
    pub fn recent_ui(&self, ui: &mut egui::Ui) -> bool {
        let (label, resources, enabled) = {
            let s = self.state();
            (s.provider.as_ref().map(|p| p.label.clone()), s.resources.clone(), s.enabled)
        };
        let Some(label) = label.filter(|_| enabled) else { return false };
        ui.add_space(12.0);
        ui.strong(format!("Recent files — {label}"));
        let mut selected = false;
        egui::ScrollArea::vertical().id_salt("file-provider-recents").max_height(ui.available_height().clamp(80.0, 240.0)).show(ui, |ui| {
            if resources.is_empty() {
                ui.weak("No recent files from this provider");
            }
            for resource in resources {
                if ui.button(&resource.name).on_hover_text(&resource.updated_at).clicked() {
                    match self.request_open(&resource.id) {
                        Ok(_) => {
                            self.state().open_error = None;
                            selected = true;
                        }
                        Err(error) => self.state().open_error = Some(error),
                    }
                    ui.ctx().request_repaint();
                }
            }
        });
        let error = self.state().open_error.clone();
        if let Some(error) = error {
            ui.label(error);
        }
        ui.separator();
        ui.weak("Recent files on this device");
        selected
    }
}
fn bounded_error(message: &str) -> String {
    let mut error = String::new();
    for ch in message.chars().filter(|ch| !ch.is_control()) {
        if error.len().saturating_add(ch.len_utf8()) > 1024 {
            break;
        }
        error.push(ch);
    }
    error
}
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 160 && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}
fn valid_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 100 && !value.chars().any(char::is_control)
}
fn valid_native_name(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 255
        && !value.chars().any(|c| c.is_control() || matches!(c, '/' | '\\'))
        && value.to_ascii_lowercase().ends_with(EXTENSION)
}
#[cfg(test)]
mod tests;
