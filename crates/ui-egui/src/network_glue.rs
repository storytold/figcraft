//! Print ▸ Network printer (WS-Print and IPP): the app's part of `craft_ui::printing::network`. The
//! model, its rules and the widgets are craft-ui's; the printing is craft-print's WS-Print and IPP
//! backends through [`crate::netprint`]. This file supplies what a host supplies: the address check,
//! the worker that asks a printer what it lists, the worker that makes the tracked print and drives
//! it, the mapping of craft-print's answers onto craft-ui's light types, and the wording.
//!
//! Built only with the `network-print` feature on a target with sockets (the `network_print` cfg).
//! All network work runs on its own threads; the UI thread only starts it and drains receivers.
//!
//! The document is the print-ready PDF (one sheet per frame, laid out on the chosen paper). A printer
//! that lists PDF gets it as it is; one that lists only PWG raster gets the same frames rendered by
//! FigCraft's own renderer ([`crate::netprint::raster::FramesToRaster`], registered for this print on
//! the station; the registry converts, at the resolution the printer lists nearest 300 dpi).
//!
//! One press of Print is one tracked [`crate::netprint::PrintJob`] ([`tracked`]), so retries,
//! reconciling and "check the printer first" advice belong to that print, and every status line names
//! the printer by what it reported about itself. A document is sent once: a send whose outcome is
//! unknown is shown with the request to check the printer before sending again, and only the
//! person's Retry sends it again.

use std::sync::{Arc, Mutex, PoisonError, mpsc};

use craft_ui::printing::network::{self as net, Failure, FailureKind, JobHandle, Labels, PrinterSummary, QueryPrinter, Request, SendJob};
use figcraft_doc::Document as Doc;
use figcraft_engine::cmd::file::PrintPlan;

use crate::i18n::{fill, tr};
use crate::netprint::ipp::PrinterAttrs;
use crate::netprint::raster::FramesToRaster;
use crate::netprint::station::{self, Station};
use crate::netprint::wsd::PrinterElements;
use crate::netprint::{Duplex, Job, JobOptions, Orientation, PrintError, Quality, UserConfirmed};

mod caps;
mod tracked;
use tracked::{Command, Slot};

/// Any transport that can be shared with the worker threads (tests give an in-memory printer).
pub type Transport = crate::netprint::Transport;

/// The media types the dialog can produce: the print-ready PDF, or its sheets as PWG raster (the
/// registry converts through the station's converter when the printer lists no PDF).
pub(crate) const FORMATS: &[&str] = &["application/pdf", "image/pwg-raster"];

/// The printer last addressed: its registry (WS-Print and IPP backends) and, with the test env var, its
/// WS-Eventing listener, kept for the query and for the jobs that follow, keyed by the typed address.
pub(crate) type Stations = Arc<Mutex<Option<(String, Arc<Station>)>>>;

/// The station for `address`: the one already open for that text, else a new one.
/// `PDFCRAFT_TEST_EVENTS_BIND` (test builds) turns the WS-Eventing listener on for a WS-Print printer.
fn station_for(stations: &Stations, address: &str, transport: Option<Transport>) -> Result<Arc<Station>, PrintError> {
    let mut slot = lock(stations);
    if let Some((typed, station)) = slot.as_ref()
        && typed == address
    {
        return Ok(Arc::clone(station));
    }
    let events = match station::events_from_env() {
        Some(Ok(config)) => Some(config),
        Some(Err(e)) => return Err(e),
        None => None,
    };
    let station = Arc::new(Station::open(address, transport, events)?);
    *slot = Some((address.to_owned(), Arc::clone(&station)));
    Ok(station)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
const MAX_JOB_NAME: usize = 100;

/// The destination's state and the closures its workers are made from.
#[derive(Default)]
pub(crate) struct NetTasks {
    transport: Option<Transport>,
    pub(crate) state: net::State,
    query: Option<QueryPrinter>,
    stations: Stations,
    /// The tracked print of the last job: its status line and buttons.
    pub(crate) tracked: Slot,
}

impl NetTasks {
    /// Use `transport` instead of the built-in HTTP one (tests; embedding).
    #[allow(dead_code)] // used by the tests
    pub(crate) fn set_transport(&mut self, transport: Transport) {
        self.transport = Some(transport);
        self.query = None;
        *lock(&self.stations) = None;
    }

    #[allow(dead_code)] // used by the tests
    pub(crate) fn transport(&self) -> Option<Transport> {
        self.transport.clone()
    }

    /// Whether the printer's Color setting is Grayscale (as it will be sent).
    pub(crate) fn grayscale(&self) -> bool {
        self.state.draft.request(None).color == Some(net::ColorMode::Monochrome)
    }

    /// Ask the printer at the typed address what it lists, as the Query button does. `false` when the
    /// address is refused or another query or job is running.
    pub(crate) fn query_now(&mut self, ctx: &egui::Context) -> bool {
        let (transport, stations) = (self.transport.clone(), Arc::clone(&self.stations));
        let query = self.query.get_or_insert_with(|| query_printer(transport, Some(ctx.clone()), stations));
        let address_ok = validate(&self.state.draft.normalized_url()).is_ok();
        self.state.start_query(query, address_ok)
    }

    /// Draw the section; `sheets_ok`: the dialog's settings give something to print.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, sheets_ok: bool) -> net::Response {
        let (transport, ctx, stations) = (self.transport.clone(), ui.ctx().clone(), Arc::clone(&self.stations));
        let query = self.query.get_or_insert_with(|| query_printer(transport, Some(ctx), stations));
        let host = net::Host { validate: &validate, query, sheets_ok, formats: FORMATS };
        let response = self.state.show(ui, egui::Id::new("network-printer"), &host, &labels());
        status_block(ui, &self.tracked, false);
        response
    }

    /// Make the tracked print of `plan` (laid out as `plan.sheets`, whose count is the job's page count) for
    /// the queried printer and start following it, off the UI thread. It needs the person's confirmation,
    /// which it spends. `doc` is the document the plan was made from (sRGB), for a printer that wants raster.
    /// Returns `true` when a job started.
    pub(crate) fn send(&mut self, ctx: Option<egui::Context>, name: &str, doc: Arc<Doc>, plan: PrintPlan, media: Option<&str>) -> bool {
        let address = self.state.draft.normalized_url();
        // The attestation comes from the ticked box and nothing else.
        let Ok(confirmed) = UserConfirmed::attest(Some(self.state.draft.confirmed)) else { return false };
        let pages = plan.sheets.len();
        let send = send_job(
            self.transport.clone(),
            ctx,
            name.to_owned(),
            (doc, Arc::new(plan)),
            (Arc::clone(&self.stations), Arc::clone(&self.tracked)),
            confirmed,
        );
        // The sheets are the pages of the document sent: the total that progress shows until the printer reports one.
        self.state.draft.expected_pages = u32::try_from(pages).ok();
        self.state.start_job(&send, validate(&address).is_ok(), true, FORMATS, media)
    }

    /// Collect finished work (each frame), also while the dialog is closed. With the dialog gone the person
    /// still hears how a job went: the one-line notice, or `None` while nothing ended.
    pub(crate) fn poll(&mut self, ctx: &egui::Context, dialog_open: bool) -> Option<String> {
        let was_running = self.state.draft.running();
        self.state.poll(ctx);
        if !was_running || self.state.draft.running() || dialog_open {
            return None;
        }
        let name = self.state.draft.printer().map(|p| p.name.clone()).unwrap_or_default();
        notice_text(&labels(), &self.state.draft.job, &name)
    }
}

/// The address checked against the network policy (a numeric loopback, link-local or private
/// address). The error is what the dialog shows under the field.
pub(crate) fn validate(text: &str) -> Result<(), Failure> {
    station::validate(text).map(|_| ()).map_err(|e| Failure::new(FailureKind::Policy, e.to_string()))
}

/// What went wrong, as craft-ui words it. `open`: the failure is the printer's address being refused.
fn failure(e: &PrintError, open: bool) -> Failure {
    tracked::failure_of(&crate::netprint::AttemptError::from(e), open)
}

fn summary(elements: PrinterElements) -> PrinterSummary {
    let caps = caps::wsd(&elements);
    let description = elements.description.unwrap_or_default();
    let status = elements.status.unwrap_or_default();
    PrinterSummary {
        name: description.name,
        color: description.color_supported,
        state: status.state,
        queued_jobs: status.queued_jobs,
        reasons: status.reasons,
        caps,
    }
}

fn repaint(ctx: &Option<egui::Context>) {
    if let Some(ctx) = ctx {
        ctx.request_repaint();
    }
}

/// What an IPP printer's attributes say, as craft-ui's light types (the keywords are the dialog's tokens).
fn ipp_summary(attrs: &PrinterAttrs) -> PrinterSummary {
    let state = match attrs.state {
        Some(3) => "Idle",
        Some(4) => "Processing",
        Some(5) => "Stopped",
        _ => "Unknown",
    };
    let mut reasons: Vec<String> = attrs.state_reasons.iter().filter(|r| r.as_str() != "none").cloned().collect();
    if attrs.accepting_jobs == Some(false) {
        reasons.push("not-accepting-jobs".to_owned());
    }
    PrinterSummary {
        name: attrs.name.clone().or_else(|| attrs.info.clone()).or_else(|| attrs.make_and_model.clone()).unwrap_or_default(),
        color: attrs.color_supported,
        state: state.to_owned(),
        queued_jobs: None,
        reasons,
        caps: caps::ipp(attrs),
    }
}

/// The worker that asks the printer at the (normalised) address what it is and what it lists, over the
/// protocol its address names (WS-Print or IPP). It sends no job.
pub(crate) fn query_printer(transport: Option<Transport>, ctx: Option<egui::Context>, stations: Stations) -> QueryPrinter {
    Box::new(move |address: &str| {
        let (tx, rx) = mpsc::channel();
        let (address, transport, ctx, stations) = (address.to_owned(), transport.clone(), ctx.clone(), Arc::clone(&stations));
        std::thread::spawn(move || {
            let result = match station_for(&stations, &address, transport) {
                Ok(station) => match station.protocol() {
                    station::Protocol::Wsd => station.wsd_info().map(summary).map_err(|e| failure(&e, false)),
                    station::Protocol::Ipp => station.ipp_info().map(|attrs| ipp_summary(&attrs)).map_err(|e| failure(&e, false)),
                }
                .inspect(|summary| station.set_display_name(&summary.name)),
                Err(e) => Err(failure(&e, true)),
            };
            let _ = tx.send(result);
            repaint(&ctx);
        });
        rx
    })
}

/// The job and its per-job options for the registry. `Photo` is craft-print's `Quality::Photo`: the
/// backend maps it to the printer's own quality when the printer lists one and says so when it does not.
fn options(request: &Request, job_name: &str) -> (Job, JobOptions) {
    let name: String = job_name.chars().filter(|c| !c.is_control()).take(MAX_JOB_NAME).collect();
    let mut job = Job::default();
    if !name.trim().is_empty() {
        job.title = name;
    }
    job.copies = request.copies.unwrap_or(1);
    job.duplex = request.sides.map(|s| match s {
        net::Sides::OneSided => Duplex::Off,
        net::Sides::LongEdge => Duplex::LongEdge,
        net::Sides::ShortEdge => Duplex::ShortEdge,
    });
    let mut options = JobOptions::new();
    if let Some(o) = request.orientation {
        options = options.with_orientation(match o {
            net::Orientation::Portrait => Orientation::Portrait,
            net::Orientation::Landscape => Orientation::Landscape,
            net::Orientation::ReversePortrait => Orientation::ReversePortrait,
            net::Orientation::ReverseLandscape => Orientation::ReverseLandscape,
        });
    }
    match request.quality {
        Some(net::Quality::Draft) => options = options.with_quality(Quality::Draft),
        Some(net::Quality::Normal) => options = options.with_quality(Quality::Normal),
        Some(net::Quality::High) => options = options.with_quality(Quality::High),
        Some(net::Quality::Photo) => options = options.with_quality(Quality::Photo),
        None => {}
    }
    options = caps::apply_request(options, request);
    if let Some(media) = &request.media_size {
        options = options.with_media(media.as_str());
    }
    if let Some(pages) = request.expected_pages {
        options = options.with_expected_pages(pages);
    }
    (job, options)
}

/// The worker that makes the tracked print of the plan's PDF and drives it (see [`tracked`]). Each call
/// makes a new print, with the person's `confirmed` (never `interactive()`: it is the attestation of the
/// ticked box, taken when Print was pressed).
pub(crate) fn send_job(
    transport: Option<Transport>,
    ctx: Option<egui::Context>,
    name: String,
    (doc, plan): (Arc<Doc>, Arc<PrintPlan>),
    (stations, slot): (Stations, Slot),
    confirmed: UserConfirmed,
) -> SendJob {
    Box::new(move |address: &str, request: Request| {
        let (job, options) = options(&request, &name);
        match station_for(&stations, address, transport.clone()) {
            Ok(station) => {
                // A printer that lists no PDF is sent the same sheets as raster.
                station.set_converter(Arc::new(FramesToRaster::new(Arc::clone(&doc), plan.sheets.clone())));
                let print = station.job(plan.pdf.clone(), job, options, confirmed);
                tracked::spawn(station, print, &slot, ctx.clone())
            }
            Err(e) => {
                let (tx, rx) = mpsc::channel();
                let _ = tx.send(Err(failure(&e, true)));
                JobHandle::new(rx, Box::new(|| {}))
            }
        }
    })
}

/// The status of the tracked print (a line built from its `JobView`, the retry advice, the last answer of
/// a command) and the buttons that make sense for the state. `row`: the small row shown with the dialog
/// closed (adds Dismiss). Returns whether there was anything to show.
pub(crate) fn status_block(ui: &mut egui::Ui, slot: &Slot, row: bool) -> bool {
    let Some(view) = lock(slot).as_ref().and_then(|t| t.view()) else { return false };
    let mut dismiss = false;
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("{}: {}", view.view.title, tracked::status_text(&view.view, &view.name))).strong());
    if let Some(advice) = tracked::advice_text(view.advice, view.lifecycle, &view.name) {
        ui.label(advice);
    }
    if let Some(stale) = tracked::stale_text(view.stale, &view.name) {
        ui.label(stale);
    }
    if let Some(note) = &view.note {
        ui.label(egui::RichText::new(note).small());
    }
    if let Some(events) = &view.events {
        ui.label(egui::RichText::new(fill(tr("WS-Eventing listener: {events}"), &[("events", events)])).small().weak());
    }
    let buttons = tracked::buttons(&view);
    ui.horizontal_wrapped(|ui| {
        let guard = lock(slot);
        let Some(tracked) = guard.as_ref() else { return };
        let mut ask = |enabled: bool, label: &str, tip: &str, command: Command| {
            if ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tip).clicked() {
                tracked.send(command);
            }
        };
        ask(buttons.cancel, tr("Cancel print"), tr("Ask the printer to cancel this print"), Command::Cancel);
        ask(buttons.retry, tr("Retry"), tr("Send this print again as a new attempt (check the printer first if it says so)"), Command::Retry);
        ask(
            buttons.reconcile,
            tr("Reconcile"),
            tr("Look at the printer's job lists (reads only) for a job from the earlier attempt"),
            Command::Reconcile,
        );
        ask(buttons.accept, tr("Accept found job"), tr("Follow the job the printer already has instead of sending again"), Command::AcceptFound);
        ask(buttons.refresh, tr("Check now"), tr("Read the job's status now"), Command::Refresh);
        if row && ui.button(tr("Dismiss")).clicked() {
            dismiss = true;
        }
    });
    if dismiss {
        *lock(slot) = None;
    }
    true
}

/// The wording of the destination, in the interface language. Templates keep their `{...}` holes.
pub(crate) fn labels() -> Labels<'static> {
    Labels {
        address: tr("Printer address:"),
        address_hint: "http://192.168.1.50:5357/service  or  ipp://192.168.1.50:631/ipp/print",
        address_note: tr("Only IP addresses on this network are accepted (private, link-local or loopback), not host names."),
        query_printer: tr("Query printer"),
        querying: tr("Asking the printer…"),
        query_first: tr("Query the printer first to see what it supports."),
        printer_status: tr("Printer status: {state}"),
        reasons: tr("Reasons: {reasons}"),
        no_common_format: tr("The printer accepts none of the formats this document can be sent in"),
        copies: tr("Copies:"),
        sides: tr("Two-sided:"),
        sides_off: tr("Off"),
        sides_long_edge: tr("Flip on long edge"),
        sides_short_edge: tr("Flip on short edge"),
        orientation: tr("Printer orientation:"),
        portrait: tr("Portrait"),
        landscape: tr("Landscape"),
        reverse_portrait: tr("Reverse portrait"),
        reverse_landscape: tr("Reverse landscape"),
        quality: tr("Quality:"),
        draft: tr("Draft"),
        normal: tr("Normal"),
        high: tr("High"),
        photo: tr("Photo"),
        color: tr("Color:"),
        color_color: tr("Color"),
        color_monochrome: tr("Grayscale"),
        resolution: tr("Resolution:"),
        resolution_dpi: "{dpi} dpi",
        resolution_dpi_xy: "{x} x {y} dpi",
        auto: tr("Auto"),
        unsupported_copies: tr("This printer prints one copy at a time."),
        unsupported_sides: tr("This printer only prints one-sided."),
        unsupported_orientation: tr("This printer does not take an orientation setting."),
        unsupported_quality: tr("This printer does not take a quality setting."),
        unsupported_color: tr("This printer only prints in grayscale."),
        unsupported_resolution: tr("This printer does not take a resolution setting."),
        unconfirmed_note: tr("Not confirmed by the printer"),
        confirm: tr("I confirm sending this document to this printer"),
        sending: tr("Sending to {printer}…"),
        canceling: tr("Canceling…"),
        cancel: tr("Cancel"),
        sent: tr("Sent to {printer}"),
        accepted: tr("Accepted by {printer}"),
        stage_waiting: tr("Waiting in the printer's queue"),
        stage_held: tr("Held by the printer"),
        stage_printing: tr("The printer is printing it"),
        stage_stopped: tr("The printer paused the job; it needs attention"),
        sheets: tr("Sheets printed: {n}"),
        sheets_progress: tr("Sheets printed: {done} of {total}"),
        outcome_warnings: tr("Printed with warnings"),
        outcome_errors: tr("Printed with errors"),
        outcome_canceled: tr("The job was canceled"),
        outcome_aborted: tr("The printer stopped the job"),
        outcome_forgotten: tr("The printer no longer lists the job; it most likely finished"),
        outcome_still_running: tr("Still printing; the job continues on the printer"),
        outcome_accepted_unknown: tr("Sent. The printer can't say whether it printed."),
        outcome_unknown_warning: tr("The printer may have received the document. Check the printer before sending again."),
        failure_policy: tr("The printer address was refused"),
        failure_transport: tr("Couldn't reach the printer"),
        failure_protocol: tr("The printer's reply couldn't be understood"),
        failure_fault: tr("The printer refused the request"),
        failure_no_common_format: tr("The printer accepts none of the formats this document can be sent in"),
        failure_unsupported: tr("The printer doesn't support this setting"),
        failure_encode: tr("The document couldn't be prepared"),
        failure_invalid: tr("A setting isn't valid"),
        failure_stopped: tr("The print job stopped unexpectedly"),
        failure_other: tr("The print job stopped unexpectedly"),
    }
}

/// The one-line notice for a print that ended with the dialog closed, naming the printer by its display name
/// (nothing when it has none, nor an address): "<name>: The printer stopped the job".
fn notice_text(labels: &Labels<'_>, job: &net::JobView, name: &str) -> Option<String> {
    let who = tracked::printer_label(name);
    let named = tracked::is_named(&who);
    let prefix = |text: String| if named { format!("{who}: {text}") } else { text };
    match job {
        net::JobView::Finished(r) => Some(match labels.outcome_text(r.outcome) {
            Some(text) => prefix(text.to_owned()),
            None => fill(labels.sent, &[("printer", &tracked::printer_label(&r.printer))]),
        }),
        net::JobView::Failed(f) => {
            let mut text = prefix(format!("{}: {}", labels.failure_heading(f.kind), tracked::scrubbed(&f.detail, &who)));
            if f.outcome_unknown {
                text = format!("{text} {}", labels.outcome_unknown_warning);
            }
            Some(text)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
