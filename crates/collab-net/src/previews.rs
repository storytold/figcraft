//! A bounded worker pool keeps image decoding off the collaboration hub thread.
use figcraft_assets::preview::Request;
use std::sync::{
    Arc, Mutex, OnceLock, PoisonError,
    atomic::{AtomicUsize, Ordering},
    mpsc::{SyncSender, sync_channel},
};

pub(crate) const PROTOCOL: &str = "figcraft.previews.v1";
type Done = Box<dyn FnOnce(Result<Arc<Vec<u8>>, String>) + Send>;
struct Job {
    request: Request,
    source: Arc<Vec<u8>>,
}
struct Pool {
    tx: SyncSender<Job>,
    held: Arc<AtomicUsize>,
    pending: Arc<Mutex<std::collections::BTreeMap<Request, Vec<Done>>>>,
}

fn pool() -> Option<&'static Pool> {
    static POOL: OnceLock<Option<Pool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (tx, rx) = sync_channel::<Job>(32);
        let rx = Arc::new(Mutex::new(rx));
        let held = Arc::new(AtomicUsize::new(0));
        let pending: Arc<Mutex<std::collections::BTreeMap<Request, Vec<Done>>>> = Arc::default();
        let mut workers = 0;
        for _ in 0..2 {
            let rx = rx.clone();
            let held = held.clone();
            let pending = pending.clone();
            if std::thread::Builder::new()
                .name("image-preview".into())
                .spawn(move || {
                    loop {
                        let job = rx.lock().unwrap_or_else(PoisonError::into_inner).recv();
                        let Ok(job) = job else { break };
                        let result = figcraft_render::images::make_preview(&job.source, u32::from(job.request.side)).map(Arc::new);
                        held.fetch_sub(job.source.len(), Ordering::Relaxed);
                        let result = result.and_then(|bytes| {
                            figcraft_assets::preview::store(&job.request, bytes.clone())?;
                            Ok(bytes)
                        });
                        let callbacks = pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&job.request).unwrap_or_default();
                        for done in callbacks {
                            done(result.clone());
                        }
                    }
                })
                .is_ok()
            {
                workers += 1;
            }
        }
        (workers > 0).then_some(Pool { tx, held, pending })
    })
    .as_ref()
}

pub(crate) fn submit(request: Request, source: Arc<Vec<u8>>, done: Done) -> bool {
    let Some(pool) = pool() else { return false };
    let mut pending = pool.pending.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(waiters) = pending.get_mut(&request) {
        if waiters.len() >= 32 {
            return false;
        }
        waiters.push(done);
        return true;
    }
    if pending.len() >= 32 {
        return false;
    }
    let n = source.len();
    let previous = pool.held.fetch_add(n, Ordering::Relaxed);
    if previous > 0 && previous.saturating_add(n) > 64 << 20 {
        pool.held.fetch_sub(n, Ordering::Relaxed);
        return false;
    }
    pending.insert(request.clone(), vec![done]);
    if pool.tx.try_send(Job { request: request.clone(), source }).is_err() {
        pending.remove(&request);
        pool.held.fetch_sub(n, Ordering::Relaxed);
        return false;
    }
    true
}
