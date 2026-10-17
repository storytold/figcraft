//! Network printing on craft-print's WS-Print and IPP backends (cfg `network_print`: the
//! `network-print` feature on a target with sockets; never the web build).
//!
//! The protocols, the print ticket and the job lifecycle are craft-print's. This
//! module is the thin layer between it and the Print dialog:
//!
//! * [`station::Station`]: one printer the person typed, behind a private [`Registry`] that holds both
//!   network backends; it asks the printer what it is and makes the tracked [`PrintJob`] for a document.
//! * [`raster`]: FigCraft's [`DocumentConverter`], the one part that is the app's own: the print-ready
//!   PDF is drawn from the page's frames, so for a printer that lists PWG raster but no PDF the same
//!   frames are rendered with FigCraft's renderer instead of reading the PDF back.
//!
//! Everything here is generic except `raster` (and the `FIGCRAFT_`-free names are deliberate: the
//! environment variable is the Craft family's: `PDFCRAFT_TEST_EVENTS_BIND`).
//! Every job needs a [`UserConfirmed`], and nothing is sent twice by this layer.

use std::sync::{Arc, PoisonError, RwLock};

pub use craft_print::ipp;
pub use craft_print::wsd;
pub use craft_print::{
    CancelToken, ColourMode, ConvertContext, Document, DocumentConverter, DocumentFormat, Duplex, ErrorKind, Job, JobHint, JobOptions, JobState,
    Orientation, OwnedDocument, Phase, PrintError, PrinterDetails, Quality, Registry, Resolution, RetrySafety, UserConfirmed, pwg,
};
// The tracked print (one `PrintJob` per press of Print) and the shapes the dialog shows from it.
pub use craft_print::{
    AttemptError, JobView, LifecycleState, Outcome as PrintOutcome, PrintJob, Reconciled as TrackedReconciled, RetryAdvice, SimpleState, Snapshot,
    StaleReason,
};

pub mod raster;
pub mod station;

/// A transport shared with worker threads: the plain-HTTP one of craft-wsd, or an in-memory fake.
pub type Transport = Arc<dyn wsd::adapter::Transport + Send + Sync>;

/// The converter every [`station::Station`] opened afterwards registers on its registry.
static CONVERTER: RwLock<Option<Arc<dyn DocumentConverter>>> = RwLock::new(None);

/// Register a converter the app brings for printers opened from now on. Replaces an earlier one.
/// FigCraft sets its converter per print instead ([`station::Station::set_converter`]), because it
/// depends on the document being printed.
#[allow(dead_code)]
pub fn register_converter(converter: Arc<dyn DocumentConverter>) {
    *CONVERTER.write().unwrap_or_else(PoisonError::into_inner) = Some(converter);
}

pub(crate) fn registered_converter() -> Option<Arc<dyn DocumentConverter>> {
    CONVERTER.read().unwrap_or_else(PoisonError::into_inner).clone()
}
