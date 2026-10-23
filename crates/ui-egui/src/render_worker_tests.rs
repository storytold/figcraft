//! Scheduling regressions independent of machine render speed.

use super::*;

fn job(doc: &Arc<Document>) -> Job {
    Job {
        doc: doc.clone(),
        page: doc.pages[0],
        view: View { origin: (0.0, 0.0), scale: 1.0, width: 64, height: 64 },
        outline: false,
        outline_extras: Default::default(),
        outlined: vec![],
        skip: vec![],
        solo: None,
        proof: false,
        uid: 1,
        zoom_preview: true,
        at: Instant::now(),
    }
}

#[test]
fn slow_edits_preview_but_quick_partial_edits_and_pans_do_not() {
    let first = Arc::new(Document::default());
    let mut renders = Renders::with_threshold(12.0);
    let initial = job(&first);
    renders.refine(&initial).unwrap();
    // Model a slow initial render without relying on the speed of the test machine.
    renders.full_ms = 30.0;
    let changed = Arc::new((*first).clone());
    let edit = job(&changed);
    assert!(renders.wants_preview(&edit), "the first edit can use the measured full render's cost");
    renders.edit_ms = Some(1.0);
    assert!(!renders.wants_preview(&edit), "cheap retained edits do not pay for a preview");
    renders.edit_ms = Some(30.0);
    assert!(renders.wants_preview(&edit), "slow edits preview even at the same zoom");
    let mut pan = initial;
    pan.view.origin.0 += 10.0;
    assert!(!renders.wants_preview(&pan), "same-snapshot pans use retained strips");
    let mut pixels = edit;
    pixels.zoom_preview = false;
    assert!(!renders.wants_preview(&pixels), "pixel preview always shows actual pixels");
}

#[test]
fn the_inline_worker_keeps_showing_new_edit_previews_then_settles_to_full_resolution() {
    let ctx = egui::Context::default();
    let mut worker = Inline { ctx, pending: None, previewed: false, renders: Renders::with_threshold(0.0) };
    let initial = Arc::new(Document::default());
    worker.submit(job(&initial));
    assert!(!worker.poll().unwrap().coarse);
    for _ in 0..10 {
        let snapshot = Arc::new((*initial).clone());
        worker.submit(job(&snapshot));
        let image = worker.poll().expect("each edit shows a preview without waiting for refinement");
        assert!(image.coarse && Arc::ptr_eq(&image.doc, &snapshot));
        assert!(worker.in_flight());
    }
    let full = worker.poll().expect("the last edit refines when input stops");
    assert!(!full.coarse && full.view.scale == 1.0 && !worker.in_flight());
    assert!(worker.poll().is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn latest_mailboxes_release_superseded_snapshots_and_deliver_only_the_newest() {
    let pending = Latest::default();
    let old = Arc::new(Document::default());
    let weak = Arc::downgrade(&old);
    drop(pending.replace((1, job(&old))));
    drop(old);
    assert!(weak.upgrade().is_some());
    let newest = Arc::new(Document::default());
    for seq in 2..=1000 {
        drop(pending.replace((seq, job(&newest))));
    }
    assert!(weak.upgrade().is_none(), "queued drag snapshots are released immediately");
    assert_eq!(Arc::strong_count(&newest), 2, "one queued snapshot regardless of input rate");
    let (seq, latest) = pending.take().unwrap();
    assert_eq!(seq, 1000);
    assert!(Arc::ptr_eq(&latest.doc, &newest) && pending.take().is_none());
    // Results use the same bounded slot, so a paused UI cannot accumulate texture images.
    let ready = Latest::default();
    for seq in 1..=1000 {
        let _ = ready.replace(seq);
    }
    assert_eq!(ready.take(), Some(1000));
    assert!(ready.take().is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_desktop_worker_settles_to_the_last_job_after_an_unpolled_burst() {
    let mut worker = Worker::new(egui::Context::default());
    for _ in 0..500 {
        worker.submit(job(&Arc::new(Document::default())));
    }
    let last = Arc::new(Document::default());
    worker.submit(job(&last));
    let started = Instant::now();
    while worker.in_flight() {
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "the newest job never settled");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let finished = worker.poll().expect("the latest result survives until the UI polls");
    assert!(!finished.coarse && Arc::ptr_eq(&finished.doc, &last));
    assert!(worker.poll().is_none());
}
