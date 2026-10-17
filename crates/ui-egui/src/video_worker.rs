//! Bounded native video decode worker used only by the prototype presenter.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

pub(crate) struct FrameRequest {
    pub(crate) hash: String,
    pub(crate) bytes: Arc<Vec<u8>>,
    pub(crate) seconds: f64,
}

pub(crate) struct Request {
    pub(crate) uid: u64,
    pub(crate) revision: u64,
    pub(crate) generation: u64,
    pub(crate) frames: Vec<FrameRequest>,
    pub(crate) ctx: egui::Context,
}

pub(crate) struct Decoded {
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub next: f64,
    pub duration: f64,
    pub is_last: bool,
}

fn is_last_frame(frame_index: u32, frame_count: u32) -> bool {
    frame_count > 0 && frame_index.saturating_add(1) >= frame_count
}

pub(crate) struct ResultSet {
    pub uid: u64,
    pub revision: u64,
    pub generation: u64,
    pub frames: Vec<Decoded>,
    pub failures: Vec<(String, String)>,
}

struct State {
    closed: bool,
    pending: Option<Request>,
    result: Option<ResultSet>,
    identity: Option<(u64, u64, u64)>,
}

pub struct Worker {
    state: Arc<(Mutex<State>, Condvar)>,
}

impl Worker {
    pub fn new() -> Result<Self, String> {
        let state = Arc::new((Mutex::new(State { closed: false, pending: None, result: None, identity: None }), Condvar::new()));
        let shared = state.clone();
        thread::Builder::new()
            .name("figcraft-video-decode".into())
            .spawn(move || run(shared))
            .map(|_| Self { state })
            .map_err(|e| format!("Could not start video decoder: {e}"))
    }

    pub fn request(&self, request: Request) {
        let (lock, wake) = &*self.state;
        if let Ok(mut state) = lock.lock() {
            state.identity = Some((request.uid, request.revision, request.generation));
            state.pending = Some(request);
            wake.notify_one();
        }
    }

    pub fn invalidate(&self) {
        let (lock, wake) = &*self.state;
        if let Ok(mut state) = lock.lock() {
            state.identity = None;
            state.pending = None;
            state.result = None;
            wake.notify_one();
        }
    }

    pub fn take_result(&self) -> Option<ResultSet> {
        self.state.0.lock().ok()?.result.take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.state;
        if let Ok(mut state) = lock.lock() {
            state.closed = true;
            state.pending = None;
            wake.notify_one();
        }
    }
}

fn run(shared: Arc<(Mutex<State>, Condvar)>) {
    let mut sources: HashMap<String, figcraft_media::VideoSource> = HashMap::new();
    let mut lru = VecDeque::new();
    let mut source_identity = None;
    loop {
        let request = {
            let (lock, wake) = &*shared;
            let Ok(state) = lock.lock() else { return };
            let mut state = match wake.wait_while(state, |s| !s.closed && s.pending.is_none()) {
                Ok(state) => state,
                Err(_) => return,
            };
            if state.closed {
                return;
            }
            state.pending.take()
        };
        let Some(request) = request else { continue };
        let identity = (request.uid, request.revision, request.generation);
        if source_identity != Some(identity) {
            sources.clear();
            lru.clear();
            source_identity = Some(identity);
        }
        let mut output =
            ResultSet { uid: request.uid, revision: request.revision, generation: request.generation, frames: Vec::new(), failures: Vec::new() };
        for task in request.frames {
            if !is_identity_current(&shared, identity) {
                break;
            }
            if !sources.contains_key(&task.hash) {
                match catch_unwind(AssertUnwindSafe(|| figcraft_media::VideoSource::open(task.bytes))) {
                    Ok(Ok(source)) => {
                        if sources.len() == 4
                            && let Some(oldest) = lru.pop_front()
                        {
                            sources.remove(&oldest);
                        }
                        sources.insert(task.hash.clone(), source);
                    }
                    Ok(Err(error)) => {
                        output.failures.push((task.hash, error.to_string()));
                        continue;
                    }
                    Err(_) => {
                        output.failures.push((task.hash, "Video decoder failed while opening this source".into()));
                        continue;
                    }
                }
            }
            lru.retain(|hash| hash != &task.hash);
            lru.push_back(task.hash.clone());
            let Some(source) = sources.get_mut(&task.hash) else { continue };
            let duration = source.metadata().duration_seconds;
            let time = task.seconds.min((duration - 1e-6).max(0.0));
            let decoded = catch_unwind(AssertUnwindSafe(|| source.frame_at_cancellable(time, || !is_identity_current(&shared, identity))));
            let frame_count = source.metadata().frame_count;
            match decoded {
                Ok(Ok(frame)) => output.frames.push(Decoded {
                    hash: task.hash,
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.rgba,
                    next: frame.next_frame_seconds,
                    duration,
                    is_last: is_last_frame(frame.frame_index, frame_count),
                }),
                Ok(Err(figcraft_media::VideoError::Cancelled)) => break,
                Ok(Err(error)) => output.failures.push((task.hash, error.to_string())),
                Err(_) => {
                    sources.remove(&task.hash);
                    output.failures.push((task.hash, "Video decoder failed while decoding this source".into()));
                }
            }
        }
        let (lock, _) = &*shared;
        let ctx = request.ctx;
        if let Ok(mut state) = lock.lock()
            && is_identity_current_locked(&state, identity)
        {
            state.result = Some(output);
            ctx.request_repaint();
        }
    }
}

fn is_identity_current(shared: &Arc<(Mutex<State>, Condvar)>, identity: (u64, u64, u64)) -> bool {
    shared.0.lock().is_ok_and(|state| is_identity_current_locked(&state, identity))
}

fn is_identity_current_locked(state: &State, identity: (u64, u64, u64)) -> bool {
    !state.closed && state.identity == Some(identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_last_frame_from_backend_index_and_count() {
        assert!(!is_last_frame(0, 2));
        assert!(is_last_frame(1, 2));
        assert!(!is_last_frame(0, 0));
        assert!(is_last_frame(u32::MAX, 1));
    }

    #[test]
    fn time_updates_keep_decode_live_but_generation_changes_cancel_it() {
        let shared = Arc::new((Mutex::new(State { closed: false, pending: None, result: None, identity: Some((1, 2, 3)) }), Condvar::new()));
        assert!(is_identity_current(&shared, (1, 2, 3)));
        if let Ok(mut state) = shared.0.lock() {
            state.pending = Some(Request { uid: 1, revision: 2, generation: 3, frames: Vec::new(), ctx: egui::Context::default() });
        }
        assert!(is_identity_current(&shared, (1, 2, 3)), "a newer time request in the same generation must not cancel this decode");
        if let Ok(mut state) = shared.0.lock() {
            state.identity = Some((1, 2, 4));
        }
        assert!(!is_identity_current(&shared, (1, 2, 3)));
    }
}
