//! Notice external saves without repainting idle windows. File contents are read only by the
//! engine's `file.refresh` command, on the session thread and inside the automation roots.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use figcraft_engine::{Session, file_access};
use figcraft_ui_egui::FigApp;
use serde_json::json;

type Stamp = Option<file_access::Stamp>;
type ShownError = (String, file_access::Stamp, String);

struct Watching {
    paths: Vec<String>,
    retry_at: Option<Instant>,
}

pub struct Watch {
    tx: Sender<Watching>,
    changed: Arc<AtomicBool>,
    paths: Vec<String>,
    revisions: Vec<(u64, u64)>,
    shown_errors: HashMap<u64, ShownError>,
    retry_at: Option<Instant>,
}

impl Watch {
    /// Start a confined metadata watcher. Dropping its sender ends the worker; stable files do
    /// not wake the UI, and file contents remain the engine's responsibility.
    pub fn start(wake: impl Fn() + Send + 'static, roots: Option<Arc<file_access::Roots>>) -> Option<Self> {
        let (tx, rx) = channel::<Watching>();
        let changed = Arc::new(AtomicBool::new(true));
        let flag = changed.clone();
        let thread = std::thread::Builder::new().name("figcraft-files".into()).spawn(move || {
            let mut paths = Vec::new();
            let mut stamps = HashMap::new();
            let mut retry_at: Option<Instant> = None;
            loop {
                let wait = retry_at.map_or(Duration::from_secs(1), |at| at.saturating_duration_since(Instant::now()).min(Duration::from_secs(1)));
                match rx.recv_timeout(wait) {
                    Ok(updated) => {
                        paths = updated.paths;
                        retry_at = updated.retry_at;
                        stamps.retain(|path, _| paths.contains(path));
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                let external = file_access::confined(roots.as_ref(), || scan(&paths, &mut stamps));
                let retry_due = retry_at.is_some_and(|at| at <= Instant::now());
                if retry_due {
                    // One wake per deadline. The next poll sends a new deadline if it failed.
                    retry_at = None;
                }
                if external || retry_due {
                    flag.store(true, Ordering::Release);
                    wake();
                }
            }
        });
        match thread {
            Ok(_) => Some(Self { tx, changed, paths: Vec::new(), revisions: Vec::new(), shown_errors: HashMap::new(), retry_at: None }),
            Err(e) => {
                log::warn!("file watcher: {e}");
                None
            }
        }
    }

    /// Refresh after a disk change, local revision or failed-read deadline. Each file/replacement
    /// gets one warning, while every actual failed attempt remains in the log.
    pub fn poll(&mut self, fig: &mut FigApp, ctx: &egui::Context) {
        // Retry after a gesture, preview or local edit ends, including undo back to the saved
        // state. A notification that arrived while editing must not be lost forever.
        let observed = revisions(&fig.session);
        let edited = observed != self.revisions;
        self.revisions = observed;
        let due = next_retry(&fig.session).is_some_and(|at| at <= Instant::now());
        if (self.changed.swap(false, Ordering::AcqRel) || edited || due)
            && let Some(result) = fig.run("file.refresh", json!({"automatic": true}))
        {
            if result["refreshed"].as_array().is_some_and(|v| !v.is_empty()) {
                ctx.request_repaint();
                for window in &fig.windows.list {
                    ctx.request_repaint_of(figcraft_ui_egui::windows::viewport(window.id));
                }
            }
            if let Some(errors) = result["errors"].as_array() {
                for error in errors.iter().filter_map(serde_json::Value::as_str) {
                    log::warn!("file refresh: {error}");
                }
            }
        }
        self.revisions = revisions(&fig.session);
        self.show_new_errors(fig);
        let paths: Vec<_> = fig.session.docs.iter().filter_map(|d| d.path.clone()).collect();
        let retry_at = next_retry(&fig.session);
        if paths != self.paths || retry_at != self.retry_at {
            // An idle/hidden viewport may not paint for a delayed repaint request. The metadata
            // worker owns this deadline and wakes the app independently of UI-frame scheduling.
            let _ = self.tx.send(Watching { paths: paths.clone(), retry_at });
            self.paths = paths;
            self.retry_at = retry_at;
        }
    }

    /// Deduplicate by tab, path, failed stamp and message; recovery, a new replacement or closing
    /// the tab removes the old warning's identity without suppressing another file's warning.
    fn show_new_errors(&mut self, fig: &mut FigApp) {
        let current: HashMap<_, _> = fig
            .session
            .docs
            .iter()
            .filter_map(|d| d.refresh_failure.as_ref().map(|f| (d.uid, (f.path.clone(), f.stamp.clone(), f.message.clone()))))
            .collect();
        let messages: Vec<_> = current.iter().filter(|(uid, error)| self.shown_errors.get(*uid) != Some(*error)).map(|(_, e)| e.2.clone()).collect();
        self.shown_errors = current;
        for message in messages {
            fig.toast(message);
        }
    }
}

/// The soonest failed replacement's deadline; none means the watcher needs no timed repaint.
fn next_retry(session: &Session) -> Option<Instant> {
    session.docs.iter().filter_map(|d| d.refresh_failure.as_ref().map(|f| f.retry_at)).min()
}

/// Observe local edits and transient-state changes without treating a metadata poll as an edit.
fn revisions(session: &Session) -> Vec<(u64, u64)> {
    session.docs.iter().map(|d| (d.uid, d.revision)).collect()
}

/// First observation and any later replacement wake the app; a stable file costs no UI frame.
fn scan(paths: &[String], stamps: &mut HashMap<String, Stamp>) -> bool {
    let mut changed = false;
    for path in paths {
        let now = file_access::stamp(path).ok();
        if stamps.get(path) != Some(&now) {
            stamps.insert(path.clone(), now);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A watcher without a background thread; tests inject the same disk-change notification.
    fn watcher() -> Watch {
        let (tx, _) = channel();
        Watch { tx, changed: Arc::new(AtomicBool::new(true)), paths: Vec::new(), revisions: Vec::new(), shown_errors: HashMap::new(), retry_at: None }
    }

    #[test]
    fn worker_wakes_once_for_a_retry_without_files_or_ui_activity() {
        let (wake, woken) = channel();
        let watch = Watch::start(
            move || {
                let _ = wake.send(());
            },
            None,
        )
        .unwrap();
        watch.tx.send(Watching { paths: Vec::new(), retry_at: Some(Instant::now() + Duration::from_millis(10)) }).unwrap();
        woken.recv_timeout(Duration::from_secs(5)).expect("the worker must wake the app for the deadline alone");
        assert!(woken.recv_timeout(Duration::from_millis(50)).is_err(), "the consumed deadline must not spin or keep waking the app");
    }

    struct FailedFile {
        dir: std::path::PathBuf,
        path: String,
        valid: Vec<u8>,
        modified: std::time::SystemTime,
        app: FigApp,
    }

    impl FailedFile {
        /// An external save replaced with damaged bytes, keeping its inode, size and timestamp.
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!("figcraft-refresh-retry-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("design.figcraft").to_string_lossy().into_owned();
            let mut writer = Session::new();
            writer.execute("file.save", &json!({"path": path})).unwrap();
            let mut reader = Session::empty();
            reader.execute("file.open", &json!({"path": path})).unwrap();
            writer.execute("page.rename", &json!({"name": "Recovered without an edit"})).unwrap();
            writer.execute("file.save", &json!({})).unwrap();
            let valid = std::fs::read(&path).unwrap();
            let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
            let file = Self { dir, path, valid, modified, app: FigApp::new(reader, Box::new(figcraft_ui_egui::NoServices)) };
            file.replace_in_place(&vec![b'!'; file.valid.len()]);
            file
        }

        /// Simulate a writer completing or repairing a same-stamp replacement in place.
        fn replace_in_place(&self, bytes: &[u8]) {
            std::fs::write(&self.path, bytes).unwrap();
            std::fs::OpenOptions::new()
                .write(true)
                .open(&self.path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(self.modified))
                .unwrap();
        }

        /// Advance only the retry deadline, so recovery tests need no sleeps or user edits.
        fn expire(&mut self) {
            self.app.session.ds_mut().unwrap().refresh_failure.as_mut().unwrap().retry_at = Instant::now() - Duration::from_secs(1);
        }
    }

    impl Drop for FailedFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn same_stamp_failure_retries_automatically_after_its_deadline() {
        let mut file = FailedFile::new();
        let mut watch = watcher();
        let ctx = egui::Context::default();
        watch.poll(&mut file.app, &ctx);
        let failure = file.app.session.ds().unwrap().refresh_failure.clone().unwrap();
        assert!(file.app.ui.toast.is_some());
        file.app.ui.toast = None;
        file.replace_in_place(&file.valid);
        assert_eq!(failure.stamp, file_access::stamp(&file.path).unwrap());
        let revision = file.app.session.ds().unwrap().revision;
        watch.poll(&mut file.app, &ctx);
        assert!(file.app.session.ds().unwrap().refresh_failure.is_some(), "same-stamp recovery waits for the bounded retry");
        assert_eq!(file.app.session.ds().unwrap().revision, revision);
        assert!(file.app.ui.toast.is_none(), "the old failure does not renew its toast");
        file.expire();
        watch.poll(&mut file.app, &ctx);
        let ds = file.app.session.ds().unwrap();
        assert!(ds.refresh_failure.is_none(), "the deadline alone triggers recovery, without a metadata notification or edit");
        assert_eq!(ds.doc.node(ds.page).unwrap().name, "Recovered without an edit");
        assert!(!ds.is_dirty());
        assert!(watch.shown_errors.is_empty(), "recovery clears the warning identity");
    }

    #[test]
    fn persistent_errors_warn_once_and_a_new_replacement_can_warn_again() {
        let mut file = FailedFile::new();
        let mut watch = watcher();
        let ctx = egui::Context::default();
        watch.poll(&mut file.app, &ctx);
        let failure = file.app.session.ds().unwrap().refresh_failure.clone().unwrap();
        file.app.ui.toast = None;
        file.app.run("view.zoomIn", json!({}));
        watch.poll(&mut file.app, &ctx);
        assert_eq!(file.app.session.ds().unwrap().refresh_failure.as_ref().unwrap().retry_at, failure.retry_at);
        assert!(file.app.ui.toast.is_none(), "using the tab must not repeat the same warning");
        file.expire();
        watch.poll(&mut file.app, &ctx);
        assert!(file.app.session.ds().unwrap().refresh_failure.as_ref().unwrap().retry_at > Instant::now());
        assert!(file.app.ui.toast.is_none(), "a failed retry logs the error without repeating its toast");
        file_access::write(&file.path, &vec![b'!'; file.valid.len()]).unwrap();
        watch.changed.store(true, Ordering::Release);
        watch.poll(&mut file.app, &ctx);
        assert_ne!(file.app.session.ds().unwrap().refresh_failure.as_ref().unwrap().stamp, failure.stamp);
        assert!(file.app.ui.toast.is_some(), "a new failed replacement gets its own warning");
        assert_eq!(watch.shown_errors.len(), 1);
    }

    #[test]
    fn external_replacements_wake_once_and_stable_files_stay_idle() {
        let dir = std::env::temp_dir().join(format!("figcraft-file-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("design.figcraft");
        let paths = vec![path.to_string_lossy().into_owned()];
        let mut stamps = HashMap::new();
        assert!(scan(&paths, &mut stamps));
        assert!(!scan(&paths, &mut stamps));
        std::fs::write(&path, b"first").unwrap();
        assert!(scan(&paths, &mut stamps));
        assert!(!scan(&paths, &mut stamps));
        let staged = dir.join("replacement");
        std::fs::write(&staged, b"second save").unwrap();
        std::fs::rename(&staged, &path).unwrap();
        assert!(scan(&paths, &mut stamps));
        assert!(!scan(&paths, &mut stamps));
        std::fs::remove_file(&path).unwrap();
        assert!(scan(&paths, &mut stamps));
        assert!(!scan(&paths, &mut stamps));
        let _ = std::fs::remove_dir_all(dir);
    }
}
