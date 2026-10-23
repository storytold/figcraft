use super::*;
use crate::netprint::{ErrorKind, Phase, RetrySafety};
use craft_print::wsd::adapter::{DeviceUrl, Request as WireRequest, Response, TransportKind, WireError};
use craft_print::wsd::testing::{Ending, FakeConfig as Config, FakePrinter, MemTransport};
use craft_ui::printing::network::Outcome;
use figcraft_doc::{Document as Doc, NodeKind};
use figcraft_engine::cmd::file::{print_plan, printable_frames};
use figcraft_pdf::{Fit, FitOrientation, FitScale};
use net::{JobView, Query};
use std::time::Duration;

const ADDRESS: &str = "127.0.0.1:5357/printer";

/// What the Print dialog owns, without the window: the destination's tasks, the document that is printed, its
/// plan and the notices a finished print would have shown.
struct App {
    net: NetTasks,
    doc: Arc<Doc>,
    plan: PrintPlan,
    notices: Vec<String>,
}

fn app_with(transport: Transport) -> App {
    let mut doc = Doc::new("Report");
    let page = doc.first_page().expect("a page");
    let frame = doc.create(page, NodeKind::Frame, "Cover", 0.0, 0.0, 200.0, 100.0).expect("a frame");
    doc.get_mut(frame).expect("the frame").fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::from_hex("FF0000").expect("a colour"))];
    let fit = Fit { paper: (595.28, 841.89), orientation: FitOrientation::Auto, scale: FitScale::Shrink, margin: 18.0 };
    let plan = print_plan(&doc, &printable_frames(&doc, page, None), Some(fit), false).expect("a plan");
    let mut net = NetTasks::default();
    net.set_transport(transport);
    net.state.draft = net::Draft::for_address(ADDRESS);
    App { net, doc: Arc::new(doc), plan, notices: Vec::new() }
}

fn app(config: Config) -> (App, Arc<FakePrinter>) {
    let printer = FakePrinter::new(config);
    (app_with(Arc::new(MemTransport(printer.clone()))), printer)
}

impl App {
    fn poll_network(&mut self) {
        if let Some(notice) = self.net.poll(&egui::Context::default(), false) {
            self.notices.push(notice);
        }
    }
}

fn wait(app: &mut App, done: impl Fn(&App) -> bool) {
    for _ in 0..400 {
        app.poll_network();
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting");
}

fn state(app: &App) -> &net::State {
    &app.net.state
}

/// Ask the printer, as the Query button does, and wait for the answer.
fn query(app: &mut App) {
    let address_ok = validate(&app.net.state.draft.normalized_url()).is_ok();
    let worker = query_printer(app.net.transport(), None, Arc::clone(&app.net.stations));
    let started = app.net.state.start_query(&worker, address_ok);
    assert_eq!(started, address_ok);
    wait(app, |a| !state(a).draft.querying());
}

fn send(app: &mut App) -> bool {
    app.net.send(None, "report.figcraft", Arc::clone(&app.doc), app.plan.clone(), Some("iso_a4_210x297mm"))
}

#[test]
fn the_policy_refuses_what_it_should_before_anything_is_sent() {
    for bad in ["printer.local", "http://8.8.8.8/", "https://10.0.0.1/", "http://user@10.0.0.1/", "http://0.0.0.0/"] {
        assert_eq!(validate(&net::normalize_url(bad)).map_err(|f| f.kind), Err(FailureKind::Policy), "{bad}");
    }
    assert!(validate(&net::normalize_url("192.168.1.50:5357/service")).is_ok());
    assert!(validate("http://[::1]:5357/x").is_ok());
}

#[test]
fn errors_map_onto_the_failures_the_dialog_words_and_keep_whether_the_outcome_is_unknown() {
    for (kind, want) in [
        (ErrorKind::InvalidRequest, FailureKind::Invalid),
        (ErrorKind::UnsupportedFormat, FailureKind::NoCommonFormat),
        (ErrorKind::Unsupported, FailureKind::Unsupported),
        (ErrorKind::Unavailable, FailureKind::Transport),
        (ErrorKind::Timeout, FailureKind::Transport),
        (ErrorKind::Failed, FailureKind::Protocol),
        (ErrorKind::Rejected, FailureKind::Fault),
        (ErrorKind::Cancelled, FailureKind::Stopped),
    ] {
        assert_eq!(failure(&PrintError::new(kind, "x"), false).kind, want, "{kind:?}");
    }
    assert_eq!(failure(&PrintError::new(ErrorKind::InvalidRequest, "x"), true).kind, FailureKind::Policy);
    // A document that could not be made into a format the printer takes is "couldn't be prepared".
    assert_eq!(failure(&PrintError::new(ErrorKind::Failed, "x").with_phase(Phase::Prepare), false).kind, FailureKind::Encode);
    assert!(failure(&PrintError::new(ErrorKind::Timeout, "x").with_safety(RetrySafety::OutcomeUnknown), false).outcome_unknown);
    assert!(!failure(&PrintError::new(ErrorKind::Busy, "x").with_safety(RetrySafety::Safe), false).outcome_unknown);
}

#[test]
fn a_query_fills_in_the_printer_and_what_it_lists() {
    let (mut app, printer) = app(Config::default());
    query(&mut app);
    let d = &state(&app).draft;
    assert!(matches!(d.query, Query::Done(_)), "{:?}", d.query);
    assert_eq!(d.offered_sides().len(), 2);
    assert_eq!(d.max_copies(), 10);
    // Reading is all that happened: no job exists.
    assert_eq!(printer.jobs(), 0);
}

#[test]
fn a_refused_address_is_never_contacted_and_a_failed_query_is_shown() {
    let (mut app, printer) = app(Config { http_error: Some(503), ..Default::default() });
    app.net.state.draft.url = "8.8.8.8".into();
    query(&mut app);
    assert!(!state(&app).draft.querying() && printer.actions().is_empty());
    app.net.state.draft.url = "127.0.0.1:1/x".into();
    query(&mut app);
    assert!(matches!(&state(&app).draft.query, Query::Failed(f) if f.kind == FailureKind::Transport), "{:?}", state(&app).draft.query);
}

#[test]
fn nothing_is_sent_without_the_confirmation_and_a_query() {
    let (mut app, printer) = app(Config::default());
    assert!(!send(&mut app), "not queried");
    query(&mut app);
    assert!(!send(&mut app), "not confirmed");
    assert_eq!(printer.jobs(), 0);
}

#[test]
fn a_confirmed_job_runs_to_the_end_and_spends_the_confirmation() {
    let (mut app, printer) = app(Config::default());
    query(&mut app);
    app.net.state.draft.confirmed = true;
    app.net.state.draft.copies = 2;
    assert!(send(&mut app));
    assert!(!state(&app).draft.confirmed && state(&app).draft.running());
    wait(&mut app, |a| !state(a).draft.running());
    match &state(&app).draft.job {
        JobView::Finished(r) => assert_eq!(r.outcome, Outcome::Completed),
        other => panic!("{other:?}"),
    }
    assert_eq!(printer.jobs(), 1);
    // The next print needs a new yes.
    assert!(!send(&mut app));
    assert_eq!(printer.jobs(), 1);
}

#[test]
fn the_job_is_accepted_first_and_only_then_followed_to_its_end() {
    let (mut app, printer) = app(Config { ending: Ending::Hangs, ..Default::default() });
    query(&mut app);
    app.net.state.draft.confirmed = true;
    assert!(send(&mut app));
    // The printer has the document and the job is still going: accepted, not finished.
    wait(&mut app, |a| matches!(state(a).draft.job, JobView::Accepted { .. }));
    assert_eq!(printer.jobs(), 1);
    assert!(state(&app).draft.running());
    // It carries on at the printer; the dialog can cancel it, once.
    assert!(app.net.state.cancel_job());
    wait(&mut app, |a| !state(a).draft.running());
    assert!(matches!(&state(&app).draft.job, JobView::Finished(r) if r.outcome == Outcome::Canceled), "{:?}", state(&app).draft.job);
    assert_eq!(printer.actions().iter().filter(|a| *a == "SendDocument").count(), 1);
    assert_eq!(printer.actions().iter().filter(|a| *a == "CancelJob").count(), 1);
}

#[test]
fn the_documents_page_count_gives_the_progress_a_total_the_printer_does_not_report() {
    let (mut app, _printer) = app(Config { ending: Ending::Hangs, ..Default::default() });
    query(&mut app);
    let draft = &mut app.net.state.draft;
    draft.confirmed = true;
    draft.copies = 2;
    assert!(send(&mut app));
    // One sheet, two copies: the app supplied the page count, craft-ui the estimate; WS-Print reports no total.
    assert_eq!(state(&app).draft.expected_pages, Some(1));
    wait(&mut app, |a| matches!(&state(a).draft.job, JobView::Accepted { progress, .. } if progress.sheets_total.is_some()));
    match &state(&app).draft.job {
        JobView::Accepted { progress, .. } => assert_eq!(progress.sheets_total, Some(2), "{progress:?}"),
        other => panic!("{other:?}"),
    }
    assert!(app.net.state.cancel_job());
    wait(&mut app, |a| !state(a).draft.running());
}

#[test]
fn a_job_the_printer_aborts_is_reported_as_such() {
    let (mut app, _printer) = app(Config { ending: Ending::Aborts("DocumentFormatError"), ..Default::default() });
    query(&mut app);
    app.net.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    assert!(matches!(&state(&app).draft.job, JobView::Finished(r) if r.outcome == Outcome::Aborted), "{:?}", state(&app).draft.job);
}

/// Delivers every request to the fake and loses the reply to `SendDocument`: the printer did the
/// work and the client saw the connection close with no answer.
struct LoseSendReply(Arc<FakePrinter>);

impl craft_print::wsd::adapter::Transport for LoseSendReply {
    fn post(&self, _: &DeviceUrl, request: WireRequest) -> Result<Response, WireError> {
        let (status, content_type, body) = self.0.handle(&request.content_type, &request.body);
        if request.soap_action.ends_with("SendDocument") {
            return Err(WireError::transport(TransportKind::ConnectionClosed, "closed without a reply"));
        }
        Ok(Response::new(status, Some(content_type), body))
    }
}

#[test]
fn a_send_whose_outcome_is_unknown_asks_to_check_the_printer_and_is_never_repeated() {
    let printer = FakePrinter::new(Config::default());
    let mut app = app_with(Arc::new(LoseSendReply(printer.clone())));
    query(&mut app);
    app.net.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    match &state(&app).draft.job {
        JobView::Failed(f) => assert!(f.outcome_unknown, "{f:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(printer.actions().iter().filter(|a| *a == "SendDocument").count(), 1, "{:?}", printer.actions());
    assert!(!printer.actions().iter().any(|a| a == "CancelJob"), "a job that may be printing is left alone");
    // The confirmation was spent: sending again needs a new yes.
    assert!(!send(&mut app));
}

#[test]
fn options_carry_only_what_was_requested() {
    let r = Request {
        copies: Some(2),
        sides: Some(net::Sides::LongEdge),
        orientation: Some(net::Orientation::Landscape),
        quality: Some(net::Quality::High),
        color: Some(net::ColorMode::Monochrome),
        resolution: Some(net::Resolution::new(600, 300)),
        media_size: Some("iso_a4_210x297mm".into()),
        expected_pages: Some(5),
    };
    let (job, o) = options(&r, "a\u{7}b.pdf");
    assert_eq!((job.copies, job.title.as_str(), job.duplex), (2, "ab.pdf", Some(Duplex::LongEdge)));
    assert_eq!(o.orientation, Some(Orientation::Landscape));
    assert_eq!(o.quality, Some(Quality::High));
    assert_eq!((o.colour, o.resolution), (craft_print::ColourMode::Mono, Some(craft_print::Resolution::new(600, 300))));
    assert_eq!((o.media.as_deref(), o.expected_pages), (Some("iso_a4_210x297mm"), Some(5)));
    let (job, o) = options(&Request::default(), "");
    assert_eq!((job.title, job.copies, job.duplex), (Job::default().title, 1, None));
    assert_eq!(o, JobOptions::new());
    // Photo is craft-print's own quality now: it goes to the registry like the others.
    let (_, o) = options(&Request { quality: Some(net::Quality::Photo), ..Request::default() }, "x");
    assert_eq!(o.quality, Some(Quality::Photo));
}

fn reads(printer: &FakePrinter) -> usize {
    printer.actions().iter().filter(|a| *a == "GetPrinterElements").count()
}

#[test]
fn photo_quality_chosen_in_the_dialog_reaches_the_ticket() {
    let (mut app, printer) = app(Config { qualities: vec!["Normal", "Photo"], ..Default::default() });
    query(&mut app);
    assert!(state(&app).draft.offered_qualities().contains(&net::Quality::Photo), "the printer lists it, so the dialog offers it");
    app.net.state.draft.confirmed = true;
    app.net.state.draft.quality = Some(net::Quality::Photo);
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    assert!(matches!(&state(&app).draft.job, JobView::Finished(r) if r.outcome == Outcome::Completed), "{:?}", state(&app).draft.job);
    let ticket = printer.job(1).unwrap().ticket;
    assert!(ticket.contains("Photo"), "Photo missing from {ticket}");
}

#[test]
fn a_printer_without_pdf_gets_the_pages_as_raster_and_is_read_twice_for_the_tracked_job() {
    let (mut app, printer) = app(Config { formats: vec!["image/pwg-raster"], resolutions: vec![(150, 150), (600, 600)], ..Default::default() });
    query(&mut app);
    assert_eq!(reads(&printer), 1, "the query");
    app.net.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    assert!(matches!(&state(&app).draft.job, JobView::Finished(r) if r.outcome == Outcome::Completed), "{:?}", state(&app).draft.job);
    let sent = printer.job(1).unwrap().document.unwrap();
    assert_eq!(&sent[..4], b"RaS2");
    // The listed resolution nearest 300 dpi (150), in the raster header's HWResolution.
    assert_eq!(u32::from_be_bytes(sent[4 + 276..4 + 280].try_into().unwrap()), 150);
    // The tracked `PrintJob` reads the printer's details itself (the registry's choice of format), then
    // the backend reads its capabilities to plan the ticket: two reads during the job after the query's.
    assert_eq!(reads(&printer), 3, "{:?}", printer.actions());
}

#[test]
fn a_printer_that_lists_pdf_gets_the_print_ready_pdf_unchanged() {
    let (mut app, printer) = app(Config::default());
    query(&mut app);
    app.net.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    assert_eq!(printer.job(1).unwrap().document.as_deref(), Some(&app.plan.pdf[..]));
    assert_eq!(reads(&printer), 3, "the query, the registry's and the backend's own: {:?}", printer.actions());
}

#[test]
fn the_notice_for_a_print_that_ended_names_the_printer_and_no_address() {
    let labels = labels();
    let finished =
        |outcome| JobView::Finished(net::Report { outcome, printer: "http://127.0.0.1:5357/printer".to_owned(), sheets: 1, notes: Vec::new() });
    let aborted = notice_text(&labels, &finished(Outcome::Aborted), "Office Laser").unwrap();
    assert_eq!(aborted, format!("Office Laser: {}", labels.outcome_aborted));
    let unnamed = notice_text(&labels, &finished(Outcome::Aborted), "").unwrap();
    assert_eq!(unnamed, labels.outcome_aborted);
    // A report that carries an address instead of a name never shows it.
    let sent = notice_text(&labels, &finished(Outcome::Completed), "").unwrap();
    assert!(!sent.contains("127.0.0.1") && !sent.contains("http"), "{sent}");
    let failure = net::Failure::new(net::FailureKind::Transport, "no answer from http://127.0.0.1:5357/printer").with_outcome_unknown(true);
    let failed = notice_text(&labels, &JobView::Failed(failure), "Office Laser").unwrap();
    assert!(failed.starts_with("Office Laser: "), "{failed}");
    assert!(!failed.contains("127.0.0.1") && !failed.contains("http"), "{failed}");
    assert_eq!(notice_text(&labels, &JobView::Idle, "Office Laser"), None);
}

/// A frame with one hyperlinked word of text (`None` when this machine has no font to lay it out).
fn linked_doc() -> Option<(figcraft_doc::Document, figcraft_doc::NodeId)> {
    use figcraft_doc::{NodeKind, StyleRun, TextData};
    let mut d = figcraft_doc::Document::new("Links");
    let page = d.first_page()?;
    let f = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 300.0, 100.0).ok()?;
    let mut td = TextData::new("Read the docs here");
    td.style.font_size = 20.0;
    let mut linked = td.style.clone();
    linked.link = Some("https://example.com/figcraft".into());
    td.runs = vec![StyleRun { start: 9, end: 13, style: linked }];
    let t = d.create(f, NodeKind::Text(td), "T", 10.0, 10.0, 280.0, 40.0).ok()?;
    d.get_mut(t).ok()?.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)];
    let (layout, _) = figcraft_text::layout_node(d.node(t)?)?;
    (!layout.glyphs.is_empty()).then_some((d, f))
}

/// Whether the PDF bytes carry any link: an annotation array, a link annotation or a URI action.
fn has_links(pdf: &[u8]) -> bool {
    let s = String::from_utf8_lossy(pdf);
    ["/Annots", "/Link", "/URI"].iter().any(|k| s.contains(k))
}

#[test]
fn a_hyperlinked_design_goes_to_a_network_printer_without_links() {
    let Some((doc, _)) = linked_doc() else { return };
    let page = doc.first_page().unwrap();
    let fit = Fit { paper: (595.28, 841.89), orientation: FitOrientation::Auto, scale: FitScale::Shrink, margin: 18.0 };
    let plan = print_plan(&doc, &printable_frames(&doc, page, None), Some(fit), false).expect("a plan");
    let (mut app, printer) = app(Config::default());
    app.doc = Arc::new(doc);
    app.plan = plan;
    query(&mut app);
    app.net.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    let sent = printer.job(1).unwrap().document.expect("the printer got the PDF");
    assert!(sent.starts_with(b"%PDF"));
    assert!(!has_links(&sent), "no /Annots, /Link or /URI in the PDF the printer receives");
}
