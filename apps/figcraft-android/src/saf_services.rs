#![cfg(target_os = "android")]

//! FigCraft's `Services` implemented on top of the Storage Access Framework.
//!
//! FigCraft is the one Craft app whose `Services` is a trait *and* whose pickers return
//! `DialogFuture = Pin<Box<dyn Future<Output = Option<PathBuf>>>>`, polled across frames by the UI
//! (see `DialogFuture` and the `future` field in `figcraft_ui_egui`). That suits SAF far better
//! than the synchronous `FnMut(&str) -> Option<String>` the other apps use: nothing has to block
//! the frame.
//!
//! The future this module returns works like this. On its first poll it spawns a thread that runs
//! [`saf::pick_media`] -- which itself blocks for as long as the picker is up, so it must not be on
//! the render thread -- and sends the chosen path back over a oneshot channel. It then asks egui to
//! repaint once the result has arrived, because a cancelled dialog produces no input event of its
//! own and an idle app would otherwise not poll again. Every later poll takes the value if it is
//! there, and `Pending` until then.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use figcraft_ui_egui::{DialogFuture, Services};

use crate::saf;

/// `Services` for Android: the SAF picker for open, and FigCraft's own directory for save.
pub struct SafServices {
    /// The activity pointer (`AndroidApp::activity_as_ptr()`), which is what
    /// `startActivityForResult` has to be called on; `ndk_context` hands out the Application, which
    /// has no such method.
    activity: usize,
    /// `AndroidApp::internal_data_path()`: the same `filesDir` the Java side writes its manifest to.
    data_dir: Option<PathBuf>,
    /// Filled in once eframe has built its `egui::Context`, so a completed pick can wake the app.
    ctx: Arc<Mutex<Option<egui::Context>>>,
}

impl SafServices {
    pub fn new(activity: usize, data_dir: Option<PathBuf>, ctx: Arc<Mutex<Option<egui::Context>>>) -> Self {
        Self { activity, data_dir, ctx }
    }

    /// Start the picker on a thread and return a future that resolves once it has an answer.
    fn spawn_pick(&self) -> DialogFuture {
        let Some(dir) = self.data_dir.clone() else {
            log::warn!("SAF: no data dir; cannot open the picker");
            return Box::pin(std::future::ready(None));
        };
        let (tx, rx) = mpsc::channel::<Option<PathBuf>>();
        let activity = self.activity;
        let ctx = Arc::clone(&self.ctx);
        std::thread::spawn(move || {
            let picked = saf::pick_media(activity, &dir, std::time::Duration::from_secs(300)).into_iter().next().map(PathBuf::from);
            match &picked {
                Some(path) => log::info!("SAF: picked {}", path.display()),
                None => log::info!("SAF: nothing picked"),
            }
            let _ = tx.send(picked);
            // A cancelled picker delivers no input event, so an idle app would never poll again.
            if let Ok(guard) = ctx.lock() {
                if let Some(ctx) = guard.as_ref() {
                    ctx.request_repaint();
                }
            }
        });
        Box::pin(PickFuture { rx })
    }
}

/// Polls a channel; `Pending` until the picker thread sends its answer.
struct PickFuture {
    rx: Receiver<Option<PathBuf>>,
}

impl Future for PickFuture {
    type Output = Option<PathBuf>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.rx.try_recv() {
            Ok(answer) => Poll::Ready(answer),
            Err(TryRecvError::Empty) => {
                // The picker thread wakes us on completion, but a poll landing between the send and
                // the wakeup would otherwise spin; yielding is enough because the wakeup follows.
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(TryRecvError::Disconnected) => Poll::Ready(None),
        }
    }
}

impl Services for SafServices {
    fn pick_open(&mut self, _title: &str, _exts: &[&str]) -> DialogFuture {
        self.spawn_pick()
    }

    fn pick_save(&mut self, _title: &str, name: &str, _exts: &[&str]) -> DialogFuture {
        // Two-stage, like the other apps, but the future makes it simpler: it can resolve straight
        // away with the scratch path so FigCraft starts writing, while a worker asks the user where
        // the file should really go and copies it there once it has stopped growing.
        let Some(dir) = self.data_dir.clone() else {
            log::warn!("SAF: no data dir; cannot save");
            return Box::pin(std::future::ready(None));
        };
        let leaf: String = {
            let requested = if name.is_empty() { "export" } else { name };
            requested.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or("export").to_owned()
        };
        let export_dir = dir.join("export");
        if std::fs::create_dir_all(&export_dir).is_err() {
            log::warn!("SAF: could not create {}", export_dir.display());
            return Box::pin(std::future::ready(None));
        }
        let scratch = export_dir.join(&leaf);
        let scratch_string = scratch.to_string_lossy().into_owned();
        log::info!("SAF: save dialog -> {scratch_string}");

        let activity = self.activity;
        let publish_path = scratch_string.clone();
        let publish_leaf = leaf.clone();
        std::thread::spawn(move || {
            if !saf::pick_save(activity, &publish_leaf, std::time::Duration::from_secs(300)) {
                log::info!("SAF: no destination chosen; the file stays at {publish_path}");
                return;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(900);
            let (mut size, mut stable_since) = (0u64, std::time::Instant::now());
            loop {
                let now = std::fs::metadata(&publish_path).map(|m| m.len()).unwrap_or(0);
                if now != size {
                    size = now;
                    stable_since = std::time::Instant::now();
                } else if size > 0 && stable_since.elapsed() > std::time::Duration::from_millis(1500) {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    log::warn!("SAF: giving up on {publish_path} (nothing was written)");
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            if saf::publish(activity, &publish_path) {
                log::info!("SAF: published {publish_path} ({size} bytes)");
            } else {
                log::warn!("SAF: publishing {publish_path} failed; the file stays in the export directory");
            }
        });

        Box::pin(std::future::ready(Some(scratch)))
    }
}
