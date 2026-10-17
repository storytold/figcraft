//! Bounded native video decode worker used only by the prototype presenter.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
pub(crate) struct FrameRequest {
    pub(crate) instance: figcraft_render::VideoInstanceId,
    pub(crate) seek_epoch: u64,
    pub(crate) hash: String,
    pub(crate) bytes: Arc<Vec<u8>>,
    pub(crate) seconds: f64,
}

pub(crate) struct AudioRequest {
    pub(crate) hash: String,
    pub(crate) bytes: Arc<Vec<u8>>,
}

pub(crate) struct Request {
    pub(crate) uid: u64,
    pub(crate) revision: u64,
    pub(crate) generation: u64,
    pub(crate) frames: Vec<FrameRequest>,
    pub(crate) audio: Vec<AudioRequest>,
    pub(crate) ctx: egui::Context,
}

pub(crate) struct Decoded {
    pub instance: figcraft_render::VideoInstanceId,
    pub seek_epoch: u64,
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub next: f64,
    pub duration: f64,
    pub is_last: bool,
    pub frame_seconds: f64,
}

pub(crate) struct AudioDecoded {
    pub hash: String,
    pub clip: Option<Arc<figcraft_media::AudioClip>>,
    pub error: Option<String>,
}

fn is_last_frame(frame_index: u32, frame_count: u32) -> bool {
    frame_count > 0 && frame_index.saturating_add(1) >= frame_count
}

pub(crate) struct ResultSet {
    pub uid: u64,
    pub revision: u64,
    pub generation: u64,
    pub frames: Vec<Decoded>,
    pub audio: Vec<AudioDecoded>,
    pub failures: Vec<(figcraft_render::VideoInstanceId, u64, String, String)>,
}

struct State {
    closed: bool,
    pending: Option<Request>,
    result: Option<ResultSet>,
    identity: Option<(u64, u64, u64)>,
    seek_epochs: HashMap<figcraft_render::VideoInstanceId, u64>,
}

pub struct Worker {
    state: Arc<(Mutex<State>, Condvar)>,
}

impl Worker {
    pub fn new() -> Result<Self, String> {
        let state =
            Arc::new((Mutex::new(State { closed: false, pending: None, result: None, identity: None, seek_epochs: HashMap::new() }), Condvar::new()));
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
            state.seek_epochs = request.frames.iter().map(|frame| (frame.instance, frame.seek_epoch)).collect();
            state.pending = Some(request);
            wake.notify_one();
        }
    }

    pub fn invalidate(&self) {
        let (lock, wake) = &*self.state;
        if let Ok(mut state) = lock.lock() {
            state.identity = None;
            state.seek_epochs.clear();
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
    let mut audio_identity = None;
    let mut audio_cache: HashMap<String, Result<Option<Arc<figcraft_media::AudioClip>>, String>> = HashMap::new();
    let mut audio_lru = VecDeque::new();
    let mut prefetch_errors: HashMap<(u64, u64, String), String> = HashMap::new();
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
        let document_identity = (request.uid, request.revision);
        if source_identity != Some(document_identity) {
            sources.clear();
            lru.clear();
            prefetch_errors.clear();
            source_identity = Some(document_identity);
        }
        if audio_identity != Some(document_identity) {
            audio_cache.clear();
            audio_lru.clear();
            audio_identity = Some(document_identity);
        }
        let mut output = ResultSet {
            uid: request.uid,
            revision: request.revision,
            generation: request.generation,
            frames: Vec::new(),
            audio: Vec::new(),
            failures: Vec::new(),
        };
        let mut prefetch_candidates = Vec::new();
        for task in request.frames {
            if !is_identity_current(&shared, identity) {
                break;
            }
            if let Some(error) = take_prefetch_error(&mut prefetch_errors, request.uid, request.revision, &task.hash) {
                sources.remove(&task.hash);
                lru.retain(|hash| hash != &task.hash);
                output.failures.push((task.instance, task.seek_epoch, task.hash, error));
                continue;
            }
            if !sources.contains_key(&task.hash) {
                match catch_unwind(AssertUnwindSafe(|| figcraft_media::VideoSource::open(task.bytes.clone()))) {
                    Ok(Ok(source)) => {
                        if sources.len() == 4
                            && let Some(oldest) = lru.pop_front()
                        {
                            sources.remove(&oldest);
                        }
                        sources.insert(task.hash.clone(), source);
                    }
                    Ok(Err(error)) => {
                        output.failures.push((task.instance, task.seek_epoch, task.hash, error.to_string()));
                        continue;
                    }
                    Err(_) => {
                        output.failures.push((task.instance, task.seek_epoch, task.hash, "Video decoder failed while opening this source".into()));
                        continue;
                    }
                }
            }
            lru.retain(|hash| hash != &task.hash);
            lru.push_back(task.hash.clone());
            let Some(source) = sources.get_mut(&task.hash) else { continue };
            let duration = source.metadata().duration_seconds;
            let time = task.seconds.min((duration - 1e-6).max(0.0));
            let decoded = catch_unwind(AssertUnwindSafe(|| {
                source.frame_at_cancellable(time, || !is_frame_current(&shared, identity, task.instance, task.seek_epoch))
            }));
            let frame_count = source.metadata().frame_count;
            match decoded {
                Ok(Ok(frame)) => {
                    prefetch_candidates.push(PrefetchCandidate {
                        hash: task.hash.clone(),
                        anchor: frame.frame_index,
                        interval: source.metadata().frame_interval_seconds,
                    });
                    output.frames.push(Decoded {
                        instance: task.instance,
                        seek_epoch: task.seek_epoch,
                        hash: task.hash,
                        width: frame.width,
                        height: frame.height,
                        rgba: frame.rgba,
                        next: frame.next_frame_seconds,
                        duration,
                        is_last: is_last_frame(frame.frame_index, frame_count),
                        frame_seconds: f64::from(frame.frame_index) * source.metadata().frame_interval_seconds,
                    });
                }
                Ok(Err(figcraft_media::VideoError::Cancelled)) => break,
                Ok(Err(error)) => output.failures.push((task.instance, task.seek_epoch, task.hash, error.to_string())),
                Err(_) => {
                    sources.remove(&task.hash);
                    output.failures.push((task.instance, task.seek_epoch, task.hash, "Video decoder failed while decoding this source".into()));
                }
            }
        }
        for task in request.audio {
            if !is_identity_current(&shared, identity) {
                break;
            }
            if let Some(cached) = audio_cache.get(&task.hash) {
                let cached = cached.clone();
                audio_lru.retain(|hash| hash != &task.hash);
                audio_lru.push_back(task.hash.clone());
                output.audio.push(audio_result(&task.hash, cached));
                continue;
            }
            if !sources.contains_key(&task.hash) {
                match catch_unwind(AssertUnwindSafe(|| figcraft_media::VideoSource::open(task.bytes.clone()))) {
                    Ok(Ok(source)) => {
                        if sources.len() == 4
                            && let Some(oldest) = lru.pop_front()
                        {
                            sources.remove(&oldest);
                        }
                        sources.insert(task.hash.clone(), source);
                    }
                    Ok(Err(error)) => {
                        let cached = Err(error.to_string());
                        cache_audio(&mut audio_cache, &mut audio_lru, &task.hash, cached.clone());
                        output.audio.push(audio_result(&task.hash, cached));
                        continue;
                    }
                    Err(_) => {
                        let cached = Err("Video decoder failed while opening this audio source".into());
                        cache_audio(&mut audio_cache, &mut audio_lru, &task.hash, cached.clone());
                        output.audio.push(audio_result(&task.hash, cached));
                        continue;
                    }
                }
            }
            let Some(source) = sources.get_mut(&task.hash) else { continue };
            let decoded = if source.metadata().audio.is_none() {
                Ok(Ok(None))
            } else {
                catch_unwind(AssertUnwindSafe(|| source.decode_audio_cancellable(|| !is_identity_current(&shared, identity))))
            };
            match decoded {
                Ok(Ok(clip)) => {
                    let cached = Ok(clip.map(Arc::new));
                    cache_audio(&mut audio_cache, &mut audio_lru, &task.hash, cached.clone());
                    output.audio.push(audio_result(&task.hash, cached));
                }
                Ok(Err(figcraft_media::VideoError::Cancelled)) => break,
                Ok(Err(error)) => {
                    let cached = Err(error.to_string());
                    cache_audio(&mut audio_cache, &mut audio_lru, &task.hash, cached.clone());
                    output.audio.push(audio_result(&task.hash, cached));
                }
                Err(_) => {
                    let cached = Err("Video decoder failed while decoding audio".into());
                    cache_audio(&mut audio_cache, &mut audio_lru, &task.hash, cached.clone());
                    output.audio.push(audio_result(&task.hash, cached));
                }
            }
        }
        let initial_warm = should_warm_initial(&prefetch_candidates);
        if initial_warm {
            prefetch_sources(&shared, identity, &prefetch_candidates, &mut sources, &mut lru, &mut prefetch_errors);
        }
        let (lock, _) = &*shared;
        let ctx = request.ctx;
        let mut published = false;
        if let Ok(mut state) = lock.lock()
            && is_identity_current_locked(&state, identity)
        {
            output.frames.retain(|frame| state.seek_epochs.get(&frame.instance) == Some(&frame.seek_epoch));
            output.failures.retain(|(instance, epoch, _, _)| state.seek_epochs.get(instance) == Some(epoch));
            if let Some(previous) = state.result.take()
                && (previous.uid, previous.revision, previous.generation) == identity
            {
                output = merge_results(previous, output);
            }
            output.frames.retain(|frame| state.seek_epochs.get(&frame.instance) == Some(&frame.seek_epoch));
            output.failures.retain(|(instance, epoch, _, _)| state.seek_epochs.get(instance) == Some(epoch));
            state.result = Some(output);
            ctx.request_repaint();
            published = true;
        }
        if published && !initial_warm {
            prefetch_sources(&shared, identity, &prefetch_candidates, &mut sources, &mut lru, &mut prefetch_errors);
        }
    }
}

struct PrefetchCandidate {
    hash: String,
    anchor: u32,
    interval: f64,
}

fn merge_results(mut previous: ResultSet, mut current: ResultSet) -> ResultSet {
    for frame in current.frames.drain(..) {
        previous.frames.retain(|existing| existing.instance != frame.instance);
        previous.frames.push(frame);
    }
    for audio in current.audio.drain(..) {
        if let Some(existing) = previous.audio.iter_mut().find(|existing| existing.hash == audio.hash) {
            *existing = audio;
        } else {
            previous.audio.push(audio);
        }
    }
    for (instance, epoch, hash, error) in current.failures.drain(..) {
        previous.failures.retain(|existing| existing.0 != instance);
        previous.failures.push((instance, epoch, hash, error));
    }
    previous
}

fn should_warm_initial(candidates: &[PrefetchCandidate]) -> bool {
    !candidates.is_empty() && candidates.iter().all(|candidate| candidate.anchor == 0)
}

fn prefetch_allowed(shared: &Arc<(Mutex<State>, Condvar)>, identity: (u64, u64, u64), candidates: &[PrefetchCandidate]) -> bool {
    let Ok(state) = shared.0.lock() else { return false };
    if !is_identity_current_locked(&state, identity) {
        return false;
    }
    let Some(pending) = state.pending.as_ref() else { return true };
    if pending.uid != identity.0 || pending.revision != identity.1 || pending.generation != identity.2 {
        return false;
    }
    !pending.frames.is_empty()
        && pending.frames.iter().all(|frame| {
            candidates.iter().any(|candidate| {
                candidate.anchor == 0 && candidate.hash == frame.hash && frame_index(frame.seconds, candidate.interval) == Some(candidate.anchor)
            })
        })
}

fn prefetch_sources(
    shared: &Arc<(Mutex<State>, Condvar)>,
    identity: (u64, u64, u64),
    candidates: &[PrefetchCandidate],
    sources: &mut HashMap<String, figcraft_media::VideoSource>,
    lru: &mut VecDeque<String>,
    errors: &mut HashMap<(u64, u64, String), String>,
) {
    for candidate in candidates {
        if !prefetch_allowed(shared, identity, candidates) {
            break;
        }
        let decoded = match sources.get_mut(&candidate.hash) {
            Some(source) => catch_unwind(AssertUnwindSafe(|| {
                source.prefetch_after_cancellable(candidate.anchor, || !prefetch_allowed(shared, identity, candidates))
            })),
            None => continue,
        };
        match decoded {
            Ok(Ok(())) | Ok(Err(figcraft_media::VideoError::Cancelled)) => {}
            Ok(Err(error)) => {
                errors.insert((identity.0, identity.1, candidate.hash.clone()), error.to_string());
                sources.remove(&candidate.hash);
                lru.retain(|hash| hash != &candidate.hash);
            }
            Err(_) => {
                errors.insert((identity.0, identity.1, candidate.hash.clone()), "Video decoder failed while prefetching this source".into());
                sources.remove(&candidate.hash);
                lru.retain(|hash| hash != &candidate.hash);
            }
        }
    }
}

fn frame_index(seconds: f64, interval: f64) -> Option<u32> {
    if !seconds.is_finite() || seconds < 0.0 || !interval.is_finite() || interval <= 0.0 {
        return None;
    }
    let position = seconds / interval;
    let roundoff = position.abs().max(1.0) * f64::EPSILON * 4.0;
    let index = (position + roundoff).floor();
    (index <= f64::from(u32::MAX)).then_some(index as u32)
}

fn take_prefetch_error(errors: &mut HashMap<(u64, u64, String), String>, uid: u64, revision: u64, hash: &str) -> Option<String> {
    errors.remove(&(uid, revision, hash.into()))
}

fn cache_audio(
    cache: &mut HashMap<String, Result<Option<Arc<figcraft_media::AudioClip>>, String>>,
    lru: &mut VecDeque<String>,
    hash: &str,
    value: Result<Option<Arc<figcraft_media::AudioClip>>, String>,
) {
    if !cache.contains_key(hash)
        && cache.len() == 4
        && let Some(oldest) = lru.pop_front()
    {
        cache.remove(&oldest);
    }
    lru.retain(|entry| entry != hash);
    lru.push_back(hash.to_string());
    cache.insert(hash.to_string(), value);
}

fn audio_result(hash: &str, decoded: Result<Option<Arc<figcraft_media::AudioClip>>, String>) -> AudioDecoded {
    match decoded {
        Ok(clip) => AudioDecoded { hash: hash.into(), clip, error: None },
        Err(error) => AudioDecoded { hash: hash.into(), clip: None, error: Some(error) },
    }
}

fn is_identity_current(shared: &Arc<(Mutex<State>, Condvar)>, identity: (u64, u64, u64)) -> bool {
    shared.0.lock().is_ok_and(|state| is_identity_current_locked(&state, identity))
}

fn is_identity_current_locked(state: &State, identity: (u64, u64, u64)) -> bool {
    !state.closed && state.identity == Some(identity)
}

fn is_frame_current(
    shared: &Arc<(Mutex<State>, Condvar)>,
    identity: (u64, u64, u64),
    instance: figcraft_render::VideoInstanceId,
    seek_epoch: u64,
) -> bool {
    shared.0.lock().is_ok_and(|state| is_identity_current_locked(&state, identity) && state.seek_epochs.get(&instance) == Some(&seek_epoch))
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
        let shared = Arc::new((
            Mutex::new(State { closed: false, pending: None, result: None, identity: Some((1, 2, 3)), seek_epochs: HashMap::new() }),
            Condvar::new(),
        ));
        assert!(is_identity_current(&shared, (1, 2, 3)));
        if let Ok(mut state) = shared.0.lock() {
            state.pending =
                Some(Request { uid: 1, revision: 2, generation: 3, frames: Vec::new(), audio: Vec::new(), ctx: egui::Context::default() });
        }
        assert!(is_identity_current(&shared, (1, 2, 3)), "a newer time request in the same generation must not cancel this decode");
        if let Ok(mut state) = shared.0.lock() {
            state.identity = Some((1, 2, 4));
        }
        assert!(!is_identity_current(&shared, (1, 2, 3)));
    }

    #[test]
    fn pending_seek_preempts_prefetch_but_same_frame_zero_does_not() {
        let shared = Arc::new((
            Mutex::new(State { closed: false, pending: None, result: None, identity: Some((1, 2, 3)), seek_epochs: HashMap::new() }),
            Condvar::new(),
        ));
        let candidates = [PrefetchCandidate { hash: "video".into(), anchor: 0, interval: 0.04 }];
        if let Ok(mut state) = shared.0.lock() {
            state.pending = Some(Request {
                uid: 1,
                revision: 2,
                generation: 3,
                frames: vec![FrameRequest {
                    instance: figcraft_render::VideoInstanceId {
                        scope: figcraft_render::VideoScopeId(1),
                        occurrence: figcraft_render::VideoOccurrenceId {
                            paint: figcraft_render::PaintId { node: figcraft_doc::NodeId(1), domain: figcraft_render::PaintDomain::Fill, index: 0 },
                            patterns: Default::default(),
                        },
                    },
                    seek_epoch: 0,
                    hash: "video".into(),
                    bytes: Arc::new(Vec::new()),
                    seconds: 0.0,
                }],
                audio: Vec::new(),
                ctx: egui::Context::default(),
            });
        }
        assert!(prefetch_allowed(&shared, (1, 2, 3), &candidates));
        if let Ok(mut state) = shared.0.lock()
            && let Some(pending) = state.pending.as_mut()
        {
            pending.frames[0].seconds = 0.04;
        }
        assert!(!prefetch_allowed(&shared, (1, 2, 3), &candidates));
    }

    #[test]
    fn mixed_initial_and_later_frames_publish_before_prefetch() {
        let candidates = [
            PrefetchCandidate { hash: "first".into(), anchor: 0, interval: 0.04 },
            PrefetchCandidate { hash: "later".into(), anchor: 15, interval: 0.04 },
        ];
        assert!(!should_warm_initial(&candidates));
        assert!(should_warm_initial(&candidates[..1]));
        assert!(!should_warm_initial(&[]));
    }

    #[test]
    fn deferred_prefetch_error_is_delivered_once_for_its_document() {
        let mut errors = HashMap::new();
        errors.insert((1, 2, "video".into()), "prefetch failed".into());
        assert_eq!(take_prefetch_error(&mut errors, 1, 3, "video"), None);
        assert_eq!(take_prefetch_error(&mut errors, 1, 2, "video").as_deref(), Some("prefetch failed"));
        assert_eq!(take_prefetch_error(&mut errors, 1, 2, "video"), None);
    }

    #[test]
    fn merged_result_keeps_unconsumed_frames_and_audio_before_new_failures() {
        let previous = ResultSet {
            uid: 1,
            revision: 2,
            generation: 3,
            frames: vec![Decoded {
                instance: figcraft_render::VideoInstanceId {
                    scope: figcraft_render::VideoScopeId(1),
                    occurrence: figcraft_render::VideoOccurrenceId {
                        paint: figcraft_render::PaintId { node: figcraft_doc::NodeId(1), domain: figcraft_render::PaintDomain::Fill, index: 0 },
                        patterns: Default::default(),
                    },
                },
                seek_epoch: 0,
                hash: "video".into(),
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 255],
                next: 0.04,
                duration: 1.0,
                is_last: false,
                frame_seconds: 0.0,
            }],
            audio: vec![AudioDecoded { hash: "video".into(), clip: None, error: None }],
            failures: Vec::new(),
        };
        let current = ResultSet {
            uid: 1,
            revision: 2,
            generation: 3,
            frames: Vec::new(),
            audio: Vec::new(),
            failures: vec![(
                figcraft_render::VideoInstanceId {
                    scope: figcraft_render::VideoScopeId(1),
                    occurrence: figcraft_render::VideoOccurrenceId {
                        paint: figcraft_render::PaintId { node: figcraft_doc::NodeId(1), domain: figcraft_render::PaintDomain::Fill, index: 0 },
                        patterns: Default::default(),
                    },
                },
                0,
                "video".into(),
                "prefetch failed".into(),
            )],
        };
        let merged = merge_results(previous, current);
        assert_eq!(merged.frames.len(), 1);
        assert_eq!(merged.audio.len(), 1);
        assert_eq!(merged.failures.len(), 1);
        assert_eq!(merged.failures[0].2, "video");
        assert_eq!(merged.failures[0].3, "prefetch failed");
    }
}
