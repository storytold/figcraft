//! File ▸ Print: the frames of the page (or the selected ones) on paper, to a printer of this
//! computer or to a network printer typed by address, both chosen from one Printer list.
//!
//! The dialog chooses *what* is printed: which frames, on which paper, the way up and how a frame is
//! scaled onto the sheet. The engine lays the frames out ([`figcraft_engine::cmd::file::print_plan`]: a
//! print-ready PDF, one sheet per frame) and the destination sends it:
//!
//! * **System printer**: the queues of the Printer list, craft-ui's queue settings over craft-print's spooler (Windows spooler, CUPS).
//!   Listing the queues and sending the job run on their own threads.
//! * **Network printer** (cfg `network_print`): the list's "Network printer…" entry (and the printers
//!   already queried in this session, which it remembers by their reported name); craft-ui's `printing::network` destination over
//!   [`crate::network_glue`], WS-Print or IPP by address, with the capability states of the printer, the
//!   per-job confirmation and a tracked job whose status lines name the printer.
//!
//! Programmatic calls never print: `file.print` only opens this dialog (AGENTS.md), and a network job
//! starts only from the Print button, after the person ticked the confirmation box there.
//! The web build has no spooler and no sockets: it says so and leaves printing to the browser's PDF
//! viewer (export the frames to PDF).

use egui::Context;
use figcraft_pdf::{Fit, FitOrientation, FitScale};

use crate::FigApp;
use crate::i18n::tr;
#[cfg(not(target_arch = "wasm32"))]
use crate::i18n::trf;

/// The unprinted margin on every sheet, in points (a quarter inch: printers cannot print to the edge).
pub const MARGIN: f64 = 18.0;

/// A paper size the dialog offers: its short and long side in points and the media name printers list.
pub struct Paper {
    pub name: &'static str,
    pub short: f64,
    pub long: f64,
    pub media: &'static str,
}

/// The sizes of the dialog (`name` is also the key in the catalogs).
pub const PAPERS: &[Paper] = &[
    Paper { name: "US Letter", short: 612.0, long: 792.0, media: "na_letter_8.5x11in" },
    Paper { name: "US Legal", short: 612.0, long: 1008.0, media: "na_legal_8.5x14in" },
    Paper { name: "Tabloid", short: 792.0, long: 1224.0, media: "na_ledger_11x17in" },
    Paper { name: "A3", short: 841.89, long: 1190.55, media: "iso_a3_297x420mm" },
    Paper { name: "A4", short: 595.28, long: 841.89, media: "iso_a4_210x297mm" },
    Paper { name: "A5", short: 419.53, long: 595.28, media: "iso_a5_148x210mm" },
];

/// Index of A4 in [`PAPERS`], the opening choice.
const A4: usize = 4;

/// Which frames are printed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Every visible frame of the page, in reading order.
    #[default]
    Page,
    /// The selected top-level frames.
    Selection,
}

/// Where the job goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Destination {
    #[default]
    System,
    Network,
}

/// One line of the Printer list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A queue of this computer.
    System(String),
    /// A network printer queried earlier in this session (an index into [`PrintUi::known`]).
    #[cfg(network_print)]
    Known(usize),
    /// "Network printer…": an address to type.
    #[cfg(network_print)]
    Network,
}

/// A network printer that answered a query: kept for the session only (never saved), so it can be
/// printed to again without retyping its address.
#[cfg(network_print)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownPrinter {
    /// The name the printer reported.
    pub name: String,
    /// The normalised address it was queried at.
    pub url: String,
}

/// Hands a print-ready PDF to a queue and says what the spooler answered.
#[cfg(not(target_arch = "wasm32"))]
pub type RunJob = fn(&[u8], &craft_print::Job) -> Result<String, String>;

/// The Print dialog's state.
pub struct PrintUi {
    pub open: bool,
    pub scope: Scope,
    /// An index into [`PAPERS`]; `None`: each sheet is its frame's own size (the printer fits it).
    pub paper: Option<usize>,
    pub orientation: FitOrientation,
    pub scale: FitScale,
    /// Where the chosen entry of the Printer list sends the job.
    pub destination: Destination,
    /// The person picked the paper themselves (not the opening A4): only then is a local queue made
    /// to have exactly that paper.
    pub paper_chosen: bool,
    /// The system printer queues and the last job sent to one.
    #[cfg(not(target_arch = "wasm32"))]
    pub spooler: craft_ui::printing::State,
    /// Lists the system queues (the spooler; tests give a fixed list).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) discover: craft_ui::printing::Discover,
    /// Hands a print-ready PDF to a queue (the spooler; tests record the job).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) run_job: RunJob,
    /// The queues were asked for since the dialog last opened.
    #[cfg(not(target_arch = "wasm32"))]
    listed: bool,
    /// The network printers that answered a query in this session.
    #[cfg(network_print)]
    pub known: Vec<KnownPrinter>,
    /// The network destination.
    #[cfg(network_print)]
    pub(crate) net: crate::network_glue::NetTasks,
}

impl Default for PrintUi {
    fn default() -> Self {
        Self {
            open: false,
            scope: Scope::Page,
            paper: Some(A4),
            orientation: FitOrientation::Auto,
            scale: FitScale::Shrink,
            destination: Destination::System,
            paper_chosen: false,
            #[cfg(not(target_arch = "wasm32"))]
            spooler: craft_ui::printing::State::default(),
            #[cfg(not(target_arch = "wasm32"))]
            discover: spooler_discover(),
            #[cfg(not(target_arch = "wasm32"))]
            run_job: spooler_run,
            #[cfg(not(target_arch = "wasm32"))]
            listed: false,
            #[cfg(network_print)]
            known: Vec::new(),
            #[cfg(network_print)]
            net: crate::network_glue::NetTasks::default(),
        }
    }
}

impl PrintUi {
    /// The layout the settings ask for: `None` when sheets are frame-sized.
    pub fn fit(&self) -> Option<Fit> {
        let paper = PAPERS.get(self.paper?)?;
        Some(Fit { paper: (paper.short, paper.long), orientation: self.orientation, scale: self.scale, margin: MARGIN })
    }

    /// The media name of the chosen paper, for printers that list it.
    pub fn media(&self) -> Option<&'static str> {
        self.paper.and_then(|i| PAPERS.get(i)).map(|p| p.media)
    }

    /// Whether the job comes out gray: the local queue's Grayscale box, or the network printer's Color
    /// setting. The PDF (and the raster made from the frames) follows it, so what is sent is gray
    /// exactly when gray is what the person asked for; the driver is asked for monochrome as well.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn grayscale(&self) -> bool {
        match self.destination {
            Destination::System => self.spooler.settings.grayscale,
            #[cfg(network_print)]
            Destination::Network => self.net.grayscale(),
            #[cfg(not(network_print))]
            Destination::Network => false,
        }
    }

    /// How a local queue takes the sheets: exactly on the paper the person picked (a queue that lacks
    /// it refuses the job), else on the queue's own paper with the sheet shrunk to fit (frame-sized
    /// sheets, and the opening A4 nobody chose).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn local_scaling(&self) -> craft_print::Scaling {
        if self.paper.is_some() && self.paper_chosen { craft_print::Scaling::Exact } else { craft_print::Scaling::ShrinkToFit }
    }

    /// The lines of the Printer list, in order: the queues of this computer (the default first), the
    /// network printers queried in this session, then "Network printer…".
    #[cfg(not(target_arch = "wasm32"))]
    pub fn entries(&self) -> Vec<Entry> {
        let mut queues: Vec<&craft_ui::printing::Printer> = self.spooler.printers.iter().collect();
        queues.sort_by_key(|q| !q.default);
        #[allow(unused_mut)]
        let mut entries: Vec<Entry> = queues.into_iter().map(|q| Entry::System(q.name.clone())).collect();
        #[cfg(network_print)]
        {
            entries.extend((0..self.known.len()).map(Entry::Known));
            entries.push(Entry::Network);
        }
        entries
    }

    /// The line the current choice is, if any.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn selected(&self) -> Option<Entry> {
        match self.destination {
            Destination::System => {
                let name = &self.spooler.settings.printer;
                self.spooler.printers.iter().any(|q| q.name == *name).then(|| Entry::System(name.clone()))
            }
            #[cfg(network_print)]
            Destination::Network => {
                let url = self.net.state.draft.normalized_url();
                Some(self.known.iter().position(|k| k.url == url).map_or(Entry::Network, Entry::Known))
            }
            #[cfg(not(network_print))]
            Destination::Network => None,
        }
    }

    /// The text of a line of the list. The default queue says so.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn entry_label(&self, entry: &Entry) -> String {
        match entry {
            Entry::System(name) if self.spooler.printers.iter().any(|q| q.name == *name && q.default) => trf("{name} (default)", &[("name", name)]),
            Entry::System(name) => name.clone(),
            #[cfg(network_print)]
            Entry::Known(i) => self.known.get(*i).map_or_else(String::new, |k| k.name.clone()),
            #[cfg(network_print)]
            Entry::Network => tr("Network printer…").to_owned(),
        }
    }

    /// Pick a line of the list. A printer queried before comes back with its address filled in (and
    /// is asked again if the dialog has forgotten what it said).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn choose(&mut self, entry: &Entry, ctx: &Context) {
        let _ = ctx;
        match entry {
            Entry::System(name) => {
                self.destination = Destination::System;
                self.spooler.settings.printer.clone_from(name);
            }
            #[cfg(network_print)]
            Entry::Network => self.destination = Destination::Network,
            #[cfg(network_print)]
            Entry::Known(i) => {
                let Some(known) = self.known.get(*i).cloned() else { return };
                self.destination = Destination::Network;
                if self.net.state.draft.normalized_url() != known.url {
                    self.net.state.draft.url = known.url;
                    self.net.state.draft.address_edited();
                }
                if self.net.state.draft.printer().is_none() {
                    self.net.query_now(ctx);
                }
            }
        }
    }

    /// Remember the network printer the last query reached, by the name it reported.
    #[cfg(network_print)]
    pub fn remember_network_printer(&mut self) {
        let Some(name) = self.net.state.draft.printer().map(|p| p.name.trim().to_owned()).filter(|n| !n.is_empty()) else { return };
        let url = self.net.state.draft.normalized_url();
        match self.known.iter_mut().find(|k| k.url == url) {
            Some(k) => k.name = name,
            None => self.known.push(KnownPrinter { name, url }),
        }
    }
}

/// The media name of a sheet size name of the dialog (the catalog key), for the sizes printers list.
pub fn media_for_paper(name: &str) -> Option<&'static str> {
    PAPERS.iter().find(|p| p.name == name).map(|p| p.media)
}

/// What the dialog would print: the frames and how many sheets they make.
#[cfg(not(target_arch = "wasm32"))]
fn frames(app: &FigApp, scope: Scope) -> Vec<figcraft_doc::NodeId> {
    let Ok(ds) = app.session.ds() else { return Vec::new() };
    use figcraft_engine::cmd::file::printable_frames;
    let selection = app.session.selection();
    match scope {
        Scope::Page => printable_frames(&ds.doc, ds.page, None),
        Scope::Selection => printable_frames(&ds.doc, ds.page, Some(&selection)),
    }
}

/// The print job for the dialog's settings, or why there is none.
#[cfg(not(target_arch = "wasm32"))]
fn plan(app: &FigApp) -> Result<(std::sync::Arc<figcraft_doc::Document>, figcraft_engine::cmd::file::PrintPlan), String> {
    use figcraft_engine::cmd::file::print_plan;
    let frames = frames(app, app.ui.print.scope);
    let ds = app.session.ds().map_err(|e| e.to_string())?;
    let plan = print_plan(&ds.doc, &frames, app.ui.print.fit(), app.ui.print.grayscale()).map_err(|e| e.to_string())?;
    // The document the raster path renders from: its colours are sRGB, like the PDF's.
    let doc = std::sync::Arc::new(ds.doc.in_srgb().into_owned());
    Ok((doc, plan))
}

/// File ▸ Print was asked for (`dialog.print`): open the dialog, or, in the web build, say why not.
pub fn request(app: &mut FigApp) {
    #[cfg(target_arch = "wasm32")]
    app.toast(tr("Printing needs the desktop app: export the frames to PDF and print that."));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let p = &mut app.ui.print;
        p.open = true;
        p.listed = false;
        #[cfg(network_print)]
        p.net.state.dialog_opened();
    }
}

/// Once a frame: collect finished work, also while the dialog is closed, and tell the person how a
/// network print went when the dialog is not there to show it.
pub fn poll(app: &mut FigApp, ctx: &Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let open = app.ui.print.open;
        #[cfg(network_print)]
        app.ui.print.remember_network_printer();
        #[cfg(network_print)]
        if let Some(notice) = app.ui.print.net.poll(ctx, open) {
            app.toast(notice);
        }
        let _ = open;
        if !open {
            app.ui.print.spooler.poll(ctx);
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

#[cfg(target_arch = "wasm32")]
pub fn show(_app: &mut FigApp, _ctx: &Context) {}

/// The dialog.
#[cfg(not(target_arch = "wasm32"))]
pub fn show(app: &mut FigApp, ctx: &Context) {
    if !app.ui.print.open {
        return;
    }
    let all = frames(app, Scope::Page).len();
    let selected = frames(app, Scope::Selection).len();
    // Nothing selected: only the page's frames can be chosen.
    if selected == 0 && app.ui.print.scope == Scope::Selection {
        app.ui.print.scope = Scope::Page;
    }
    let t = crate::theme::Tokens::get(ctx);
    let mut open = true;
    let mut print_now = false;
    crate::dialogs::window(tr("Print"), ctx).open(&mut open).show(ctx, |ui| {
        ui.set_width(380.0);
        let p = &mut app.ui.print;
        crate::widgets::label(ui, tr("What to print"), t.text_secondary);
        if crate::widgets::radio(ui, p.scope == Scope::Page, &trf("Every frame on this page ({n})", &[("n", &all)])) {
            p.scope = Scope::Page;
        }
        ui.add_enabled_ui(selected > 0, |ui| {
            if crate::widgets::radio(ui, p.scope == Scope::Selection, &trf("The selected frames ({n})", &[("n", &selected)])) {
                p.scope = Scope::Selection;
            }
        });
        ui.add_space(8.0);
        egui::Grid::new("print_layout").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(tr("Paper:"));
            let paper_text = p.paper.and_then(|i| PAPERS.get(i)).map_or_else(|| tr("Frame size").to_owned(), |paper| tr(paper.name).to_owned());
            let paper_before = p.paper;
            egui::ComboBox::from_id_salt("print_paper").selected_text(paper_text).show_ui(ui, |ui| {
                ui.selectable_value(&mut p.paper, None, tr("Frame size"));
                for (i, paper) in PAPERS.iter().enumerate() {
                    ui.selectable_value(&mut p.paper, Some(i), tr(paper.name));
                }
            });
            if p.paper != paper_before {
                p.paper_chosen = true;
            }
            ui.end_row();
            // Orientation and scale only mean something on a paper.
            ui.add_enabled_ui(p.paper.is_some(), |ui| {
                ui.label(tr("Orientation:"));
            });
            let orientation = match p.orientation {
                FitOrientation::Auto => tr("Automatic (by the frame's shape)"),
                FitOrientation::Portrait => tr("Portrait"),
                FitOrientation::Landscape => tr("Landscape"),
            };
            ui.add_enabled_ui(p.paper.is_some(), |ui| {
                egui::ComboBox::from_id_salt("print_orientation").selected_text(orientation).show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.orientation, FitOrientation::Auto, tr("Automatic (by the frame's shape)"));
                    ui.selectable_value(&mut p.orientation, FitOrientation::Portrait, tr("Portrait"));
                    ui.selectable_value(&mut p.orientation, FitOrientation::Landscape, tr("Landscape"));
                });
            });
            ui.end_row();
            ui.add_enabled_ui(p.paper.is_some(), |ui| {
                ui.label(tr("Scale:"));
            });
            let scale = match p.scale {
                FitScale::Shrink => tr("Actual size, shrunk to fit when larger"),
                FitScale::Fill => tr("Fit to the page"),
            };
            ui.add_enabled_ui(p.paper.is_some(), |ui| {
                egui::ComboBox::from_id_salt("print_scale").selected_text(scale).show_ui(ui, |ui| {
                    ui.selectable_value(&mut p.scale, FitScale::Shrink, tr("Actual size, shrunk to fit when larger"));
                    ui.selectable_value(&mut p.scale, FitScale::Fill, tr("Fit to the page"));
                });
            });
            ui.end_row();
        });
        if p.paper.is_none() {
            crate::widgets::label(ui, tr("Each sheet is the size of its frame: the printer decides how it fits."), t.text_secondary);
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
        printer_picker(ui, p);
        ui.add_space(6.0);
        let sheets_ok = match p.scope {
            Scope::Page => all > 0,
            Scope::Selection => selected > 0,
        };
        match p.destination {
            Destination::System => print_now = system_section(ui, p, &t, sheets_ok),
            #[cfg(network_print)]
            Destination::Network => {
                let response = p.net.show(ui, sheets_ok);
                ui.add_space(6.0);
                if crate::widgets::button(ui, tr("Print"), true, response.block.is_none()).clicked() {
                    print_now = true;
                }
            }
            #[cfg(not(network_print))]
            Destination::Network => p.destination = Destination::System,
        }
    });
    if print_now {
        send(app, ctx);
    }
    if !open {
        app.ui.print.open = false;
    }
}

/// The Printer list: the queues of this computer, the network printers asked this session and
/// "Network printer…", with Refresh beside it.
#[cfg(not(target_arch = "wasm32"))]
fn printer_picker(ui: &mut egui::Ui, p: &mut PrintUi) {
    let ctx = ui.ctx().clone();
    if !p.listed {
        p.listed = true;
        p.spooler.refresh(&p.discover);
    }
    p.spooler.poll(&ctx);
    #[cfg(network_print)]
    p.remember_network_printer();
    let entries = p.entries();
    let selected = p.selected();
    let shown = match &selected {
        Some(entry) => p.entry_label(entry),
        None => tr("Select a printer").to_owned(),
    };
    let mut picked = None;
    ui.add_enabled_ui(!p.spooler.busy(), |ui| {
        ui.horizontal(|ui| {
            let label = ui.label(tr("Printer:"));
            egui::ComboBox::from_id_salt("print_printer")
                .width(230.0)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for entry in &entries {
                        if ui.selectable_label(selected.as_ref() == Some(entry), p.entry_label(entry)).clicked() {
                            picked = Some(entry.clone());
                        }
                    }
                })
                .response
                .labelled_by(label.id);
            if ui.add_enabled(!p.spooler.discovering(), egui::Button::new(tr("Refresh"))).clicked() {
                p.spooler.refresh(&p.discover);
            }
        });
    });
    if let Some(entry) = picked {
        p.choose(&entry, &ctx);
    }
}

/// The controls of a printer of this computer, its feedback and its Print button. Returns whether
/// Print was clicked.
#[cfg(not(target_arch = "wasm32"))]
fn system_section(ui: &mut egui::Ui, p: &mut PrintUi, t: &crate::theme::Tokens, sheets_ok: bool) -> bool {
    use craft_ui::printing::{Duplex, Feedback};
    let busy = p.spooler.busy();
    ui.add_enabled_ui(!busy, |ui| {
        let settings = &mut p.spooler.settings;
        ui.horizontal(|ui| {
            let label = ui.label(tr("Copies:"));
            ui.add(egui::DragValue::new(&mut settings.copies).range(1..=999)).labelled_by(label.id);
        });
        let choices = [
            (None, tr("Printer default")),
            (Some(Duplex::Off), tr("Off")),
            (Some(Duplex::LongEdge), tr("Flip on long edge")),
            (Some(Duplex::ShortEdge), tr("Flip on short edge")),
        ];
        ui.horizontal(|ui| {
            let label = ui.label(tr("Two-sided:"));
            let shown = choices.iter().find(|(value, _)| *value == settings.duplex).map_or(tr("Printer default"), |(_, text)| *text);
            egui::ComboBox::from_id_salt("print_duplex")
                .width(230.0)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (value, text) in choices {
                        ui.selectable_value(&mut settings.duplex, value, text);
                    }
                })
                .response
                .labelled_by(label.id);
        });
        ui.checkbox(&mut settings.grayscale, tr("Grayscale"));
    });
    let message = match &p.spooler.message {
        None => None,
        Some(Feedback::FindingPrinters) => Some(tr("Looking for printers…").to_owned()),
        Some(Feedback::NoPrinters) => Some(tr("No printers were found on this computer.").to_owned()),
        Some(Feedback::DiscoveryFailed) => Some(tr("The printers couldn't be listed.").to_owned()),
        Some(Feedback::Submitting) => Some(tr("Sending to the printer…").to_owned()),
        Some(Feedback::Submitted(receipt)) => Some(trf("Sent to the printer: {receipt}", &[("receipt", receipt)])),
        Some(Feedback::Failed(error)) => Some(trf("The print job failed: {error}", &[("error", error)])),
        Some(Feedback::WorkerStopped) => Some(tr("The print job stopped unexpectedly.").to_owned()),
    };
    if let Some(message) = message {
        crate::widgets::label(ui, &message, t.text_secondary);
    }
    ui.add_space(6.0);
    crate::widgets::button(ui, tr("Print"), true, sheets_ok && p.spooler.ready()).clicked()
}

/// Start the job for the dialog's settings.
#[cfg(not(target_arch = "wasm32"))]
fn send(app: &mut FigApp, ctx: &Context) {
    let (doc, plan) = match plan(app) {
        Ok(planned) => planned,
        Err(why) => {
            app.toast(trf("Nothing was printed: {why}", &[("why", &why)]));
            return;
        }
    };
    let name = doc.name.clone();
    match app.ui.print.destination {
        Destination::System => {
            let submit = spooler_submit(name, app.ui.print.local_scaling(), app.ui.print.run_job);
            let settings = app.ui.print.spooler.settings.clone();
            if app.ui.print.spooler.begin(submit(plan.pdf, settings)).is_err() {
                app.toast(tr("A print job is already being sent."));
            }
        }
        #[cfg(network_print)]
        Destination::Network => {
            let media = app.ui.print.media();
            if !app.ui.print.net.send(Some(ctx.clone()), &name, doc, plan, media) {
                app.toast(tr("Query the printer, then tick the confirmation box before sending."));
            }
        }
        #[cfg(not(network_print))]
        Destination::Network => {}
    }
    let _ = ctx;
}

/// Lists the system's printer queues on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn spooler_discover() -> craft_ui::printing::Discover {
    Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new().name("print-discovery".into()).spawn(move || {
            let printers = craft_print::printers().into_iter().map(|p| craft_ui::printing::Printer { name: p.name, default: p.default }).collect();
            let _ = sender.send(printers);
        });
        receiver
    })
}

/// The job for a local queue: the person's settings, the title, and how the sheets are taken
/// (see [`PrintUi::local_scaling`]). `grayscale` is the driver hint; the PDF is gray itself.
#[cfg(not(target_arch = "wasm32"))]
pub fn local_job(settings: &craft_ui::printing::Settings, title: String, scaling: craft_print::Scaling) -> craft_print::Job {
    craft_print::Job {
        printer: Some(settings.printer.clone()),
        copies: settings.copies.clamp(1, 999),
        duplex: settings.duplex.map(|d| match d {
            craft_ui::printing::Duplex::Off => craft_print::Duplex::Off,
            craft_ui::printing::Duplex::LongEdge => craft_print::Duplex::LongEdge,
            craft_ui::printing::Duplex::ShortEdge => craft_print::Duplex::ShortEdge,
        }),
        grayscale: settings.grayscale,
        scaling,
        title,
        ..craft_print::Job::default()
    }
}

/// The spooler: Windows' queues, CUPS.
#[cfg(not(target_arch = "wasm32"))]
fn spooler_run(pdf: &[u8], job: &craft_print::Job) -> Result<String, String> {
    craft_print::submit(pdf, job).map_err(|e| e.to_string())
}

/// Sends a print-ready PDF to a system queue on a thread of its own. Workers are kept until
/// [`finish_pending_jobs`]: dropping the handle at exit could kill the spooler hand-over half way.
#[cfg(not(target_arch = "wasm32"))]
fn spooler_submit(title: String, scaling: craft_print::Scaling, run: RunJob) -> craft_ui::printing::Submit {
    Box::new(move |pdf, settings| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let title = title.clone();
        let worker = std::thread::Builder::new().name("print-submit".into()).spawn(move || {
            let job = local_job(&settings, title, scaling);
            let _ = sender.send(run(&pdf, &job));
        });
        if let Ok(worker) = worker {
            let mut jobs = JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            jobs.retain(|j| !j.is_finished());
            jobs.push(worker);
        }
        receiver
    })
}

#[cfg(not(target_arch = "wasm32"))]
static JOBS: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

/// Wait for the print jobs being handed to the spooler (call before the process exits).
#[cfg(not(target_arch = "wasm32"))]
pub fn finish_pending_jobs() {
    let workers = std::mem::take(&mut *JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for worker in workers {
        if worker.join().is_err() {
            log::error!("the print worker stopped unexpectedly during shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_opens_on_a4_and_maps_its_choices_to_the_layout() {
        let mut ui = PrintUi::default();
        assert_eq!(ui.media(), Some("iso_a4_210x297mm"));
        let fit = ui.fit().expect("a paper");
        assert_eq!((fit.paper, fit.margin, fit.scale, fit.orientation), ((595.28, 841.89), MARGIN, FitScale::Shrink, FitOrientation::Auto));
        ui.paper = Some(0);
        ui.scale = FitScale::Fill;
        ui.orientation = FitOrientation::Landscape;
        assert_eq!(ui.media(), Some("na_letter_8.5x11in"));
        assert_eq!(ui.fit().map(|f| (f.paper, f.scale, f.orientation)), Some(((612.0, 792.0), FitScale::Fill, FitOrientation::Landscape)));
        // Frame size: no paper, no layout, no media.
        ui.paper = None;
        assert!(ui.fit().is_none() && ui.media().is_none());
        ui.paper = Some(99);
        assert!(ui.fit().is_none() && ui.media().is_none(), "an index outside the table is no paper");
    }

    #[test]
    fn paper_names_map_to_the_sizes_printers_list() {
        assert_eq!(media_for_paper("A4"), Some("iso_a4_210x297mm"));
        assert_eq!(media_for_paper("US Letter"), Some("na_letter_8.5x11in"));
        assert_eq!(media_for_paper("ARCH A (9 x 12 in)"), None);
    }

    #[test]
    fn every_paper_is_portrait_and_names_a_media() {
        for p in PAPERS {
            assert!(p.short < p.long, "{}", p.name);
            assert!(!p.media.is_empty());
        }
    }

    // ---- one Printer list, Grayscale, and the paper of a local job (in-process fakes only) ----

    #[cfg(not(target_arch = "wasm32"))]
    mod dialog {
        use std::sync::Mutex;

        use super::*;
        use crate::layers_drag_tests::Ui;
        use crate::live_tests::{flat, has_text};
        use craft_ui::printing::{Discover, Printer, Settings};
        use figcraft_engine::Session;

        /// Two queues (the second is the default), listed without asking the spooler.
        fn fake_queues() -> Discover {
            Box::new(|| {
                let (tx, rx) = std::sync::mpsc::channel();
                let _ = tx.send(vec![Printer { name: "Office Laser".into(), default: false }, Printer { name: "Front Desk".into(), default: true }]);
                rx
            })
        }

        /// What the fake spooler was handed.
        static GOT: Mutex<Vec<(Vec<u8>, craft_print::Job)>> = Mutex::new(Vec::new());

        fn fake_run(pdf: &[u8], job: &craft_print::Job) -> Result<String, String> {
            GOT.lock().unwrap().push((pdf.to_vec(), job.clone()));
            Ok("fake".into())
        }

        fn listed() -> PrintUi {
            let mut p = PrintUi { discover: fake_queues(), run_job: fake_run, ..PrintUi::default() };
            p.spooler.refresh(&p.discover);
            p.spooler.poll(&egui::Context::default());
            p.listed = true;
            p
        }

        fn shown(u: &mut Ui) -> Vec<egui::Shape> {
            flat(&u.frame_output(vec![]).shapes)
        }

        fn open_dialog() -> Ui {
            let mut u = Ui::new(Session::empty());
            u.app.ui.print.discover = fake_queues();
            u.app.ui.print.run_job = fake_run;
            u.app.ui.print.open = true;
            u.frame(vec![]);
            u.frame(vec![]);
            u
        }

        #[test]
        fn the_list_has_the_real_queues_first_the_default_marked_and_the_network_entry_last() {
            let p = listed();
            let entries = p.entries();
            assert_eq!(entries.first(), Some(&Entry::System("Front Desk".into())), "the default queue leads");
            assert_eq!(entries.get(1), Some(&Entry::System("Office Laser".into())));
            assert_eq!(p.entry_label(&entries[0]), "Front Desk (default)");
            assert_eq!(p.entry_label(&entries[1]), "Office Laser");
            #[cfg(network_print)]
            {
                assert_eq!(entries.last(), Some(&Entry::Network));
                assert_eq!(p.entry_label(&Entry::Network), "Network printer…");
                assert_eq!(entries.len(), 3);
            }
            // The default is chosen when the list arrives.
            assert_eq!(p.selected(), Some(Entry::System("Front Desk".into())));
        }

        #[test]
        fn the_dialog_has_one_printer_list_and_no_destination_switch() {
            let mut u = open_dialog();
            let s = shown(&mut u);
            assert!(has_text(&s, "Printer:") && has_text(&s, "Front Desk (default)"), "the default queue is the choice");
            assert!(has_text(&s, "Copies:") && has_text(&s, "Grayscale"));
            for old in ["Print to", "A printer on this computer", "A network printer", "Print in black and white"] {
                assert!(!has_text(&s, old), "{old}");
            }
            assert!(!has_text(&s, "Query printer"), "the network section waits for its entry");
        }

        #[cfg(network_print)]
        #[test]
        fn choosing_the_network_entry_shows_the_network_section() {
            let mut u = open_dialog();
            let ctx = u.ctx.clone();
            u.app.ui.print.choose(&Entry::Network, &ctx);
            let s = shown(&mut u);
            assert!(has_text(&s, "Query printer"), "the address field's Query printer button");
            assert!(has_text(&s, "Network printer…"), "the list shows the entry it is on");
            assert!(!has_text(&s, "Copies:"), "the queue's own settings are not shown for a network printer");
            // Back to a queue: the local settings return and the network section goes.
            u.app.ui.print.choose(&Entry::System("Office Laser".into()), &ctx);
            let s = shown(&mut u);
            assert!(!has_text(&s, "Query printer") && has_text(&s, "Grayscale") && has_text(&s, "Office Laser"));
        }

        #[cfg(network_print)]
        #[test]
        fn a_queried_network_printer_becomes_its_own_entry_by_its_reported_name() {
            use craft_print::wsd::testing::{FakeConfig, FakePrinter, MemTransport};
            let mut p = listed();
            let ctx = egui::Context::default();
            let printer = FakePrinter::new(FakeConfig::default());
            p.net.set_transport(std::sync::Arc::new(MemTransport(printer.clone())));
            p.net.state.draft = craft_ui::printing::network::Draft::for_address("127.0.0.1:5357/printer");
            p.choose(&Entry::Network, &ctx);
            assert!(p.net.query_now(&ctx));
            for _ in 0..400 {
                p.net.state.poll(&ctx);
                if !p.net.state.draft.querying() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            p.remember_network_printer();
            p.remember_network_printer();
            assert_eq!(p.known.len(), 1, "once, however often it is noted");
            let name = p.known[0].name.clone();
            assert!(!name.is_empty());
            let entries = p.entries();
            assert_eq!(entries[entries.len() - 2], Entry::Known(0), "the printer sits before 'Network printer…'");
            assert_eq!(entries.last(), Some(&Entry::Network));
            assert_eq!(p.entry_label(&Entry::Known(0)), name);
            assert_eq!(p.selected(), Some(Entry::Known(0)));
            // Pick a queue, then the remembered printer: no retyping, the address and its answer are back.
            p.choose(&Entry::System("Office Laser".into()), &ctx);
            assert_eq!(p.destination, Destination::System);
            p.choose(&Entry::Known(0), &ctx);
            assert_eq!(p.destination, Destination::Network);
            assert_eq!(p.net.state.draft.url, "127.0.0.1:5357/printer");
            assert!(p.net.state.draft.printer().is_some());
            assert_eq!(printer.jobs(), 0, "reading only; nothing was printed");
        }

        #[test]
        fn grayscale_is_the_one_word_and_the_pdf_follows_the_choice() {
            let mut p = listed();
            assert!(!p.grayscale());
            // The network colour choice says Grayscale too, with its reason text.
            #[cfg(network_print)]
            {
                let labels = crate::network_glue::labels();
                assert_eq!(labels.color_monochrome, "Grayscale");
                assert_eq!(labels.unsupported_color, "This printer only prints in grayscale.");
            }
            // A red frame.
            let mut doc = figcraft_doc::Document::new("Red");
            let page = doc.first_page().unwrap();
            let frame = doc.create(page, figcraft_doc::NodeKind::Frame, "F", 0.0, 0.0, 200.0, 100.0).unwrap();
            doc.get_mut(frame).unwrap().fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::from_hex("FF0000").unwrap())];
            let fit = p.fit();
            let plan = |gray| figcraft_engine::cmd::file::print_plan(&doc, &[frame], fit, gray).unwrap().pdf;
            let colour = plan(p.grayscale());
            p.spooler.settings.grayscale = true;
            assert!(p.grayscale());
            let gray = plan(p.grayscale());
            // (figcraft-pdf's tests read the colour operators; here: the dialog's choice reaches the PDF.)
            assert_ne!(colour, gray, "ticking Grayscale changes the PDF that is sent");
            assert_eq!(colour, plan(false));
            assert_eq!(gray, plan(true));
            // The queue is still asked for monochrome as the driver hint.
            assert!(local_job(&p.spooler.settings, "Red".into(), p.local_scaling()).grayscale);
            // A network printer's Color setting is the choice when that printer is chosen.
            #[cfg(network_print)]
            {
                p.spooler.settings.grayscale = false;
                p.destination = Destination::Network;
                assert!(!p.grayscale());
                p.net.state.draft.color = Some(craft_ui::printing::network::ColorMode::Monochrome);
                assert!(p.grayscale());
            }
        }

        #[test]
        fn a_slide_or_custom_size_job_goes_on_the_queues_own_paper_shrunk_to_fit() {
            let mut p = listed();
            // The opening A4 is nobody's choice: no paper is demanded of the queue.
            assert_eq!(p.local_scaling(), craft_print::Scaling::ShrinkToFit);
            p.paper = None; // Frame size: a 13.33 x 7.50 in slide, an artboard
            assert_eq!(p.local_scaling(), craft_print::Scaling::ShrinkToFit);
            let job = local_job(&Settings { printer: "Front Desk".into(), ..Settings::default() }, "Deck".into(), p.local_scaling());
            assert_eq!((job.scaling, job.printer.as_deref()), (craft_print::Scaling::ShrinkToFit, Some("Front Desk")));
            assert!(job.options.is_empty(), "no paper option is sent for the job");
        }

        #[test]
        fn a_paper_the_person_picked_is_demanded_exactly() {
            let mut p = listed();
            p.paper = Some(0);
            p.paper_chosen = true;
            assert_eq!(p.local_scaling(), craft_print::Scaling::Exact);
            // ... but not once they go back to the frame's own size.
            p.paper = None;
            assert_eq!(p.local_scaling(), craft_print::Scaling::ShrinkToFit);
        }

        #[test]
        fn the_job_the_spooler_gets_has_the_scaling_the_dialog_chose() {
            GOT.lock().unwrap().clear();
            let mut p = listed();
            p.paper = None;
            p.spooler.settings.grayscale = true;
            let submit = spooler_submit("Slides".into(), p.local_scaling(), p.run_job);
            let rx = submit(b"%PDF fake".to_vec(), p.spooler.settings.clone());
            assert_eq!(rx.recv().unwrap(), Ok("fake".into()));
            let got = GOT.lock().unwrap();
            let (pdf, job) = got.last().expect("the spooler was handed the job");
            assert_eq!(pdf, b"%PDF fake");
            assert_eq!((job.scaling, job.grayscale, job.title.as_str()), (craft_print::Scaling::ShrinkToFit, true, "Slides"));
            assert_eq!(job.printer.as_deref(), Some("Front Desk"));
            drop(got);
            finish_pending_jobs();
        }

        /// A frame with one hyperlinked word (`None` without a font on this machine).
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

        #[test]
        fn a_hyperlinked_design_reaches_the_spooler_without_links_on_the_queues_paper_and_on_a_chosen_one() {
            let Some((doc, frame)) = linked_doc() else { return };
            let mut p = listed();
            for (paper, chosen) in [(None, false), (Some(0), true)] {
                GOT.lock().unwrap().clear();
                p.paper = paper;
                p.paper_chosen = chosen;
                for gray in [false, true] {
                    p.spooler.settings.grayscale = gray;
                    let pdf = figcraft_engine::cmd::file::print_plan(&doc, &[frame], p.fit(), p.grayscale()).unwrap().pdf;
                    let submit = spooler_submit("Links".into(), p.local_scaling(), p.run_job);
                    assert_eq!(submit(pdf, p.spooler.settings.clone()).recv().unwrap(), Ok("fake".into()));
                }
                finish_pending_jobs();
                let got = GOT.lock().unwrap();
                assert_eq!(got.len(), 2);
                for (pdf, _) in got.iter() {
                    let s = String::from_utf8_lossy(pdf);
                    assert!(s.starts_with("%PDF"));
                    for k in ["/Annots", "/Link", "/URI"] {
                        assert!(!s.contains(k), "{k} in the PDF handed to the spooler (paper {paper:?})");
                    }
                }
            }
        }
    }
}
