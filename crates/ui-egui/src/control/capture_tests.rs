//! Screenshot capture (the local completion-slot adapter): per-request identity and reverse
//! completion, cancellation/late delivery, budget admission, straight-alpha round-trips and zero
//! retained bytes after draining. Driven headless through the real UI pass, so geometry comes from
//! the frame that issues the capture, not from a stale logic tick.

use std::sync::Arc;

use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::control::{CaptureSpec, ControlReply, ControlRequest};
use crate::layers_drag_tests::Ui;

/// Queue a `ui.screenshot` request in the test app and return its reply receiver.
fn push(u: &mut Ui, id: u64, params: serde_json::Value) -> std::sync::mpsc::Receiver<ControlReply> {
    let (tx, rx) = std::sync::mpsc::channel();
    u.app.control_queue.push(ControlRequest { id: json!(id), method: "ui.screenshot".into(), params, reply: tx, state: Default::default() });
    rx
}

/// Run UI passes until every admitted capture has been issued (settled on real UI passes).
fn settle_to_issue(u: &mut Ui, n: usize) {
    for _ in 0..12 {
        u.frame(vec![]);
        if u.app.capture.shots.len() == n && u.app.capture.shots.iter().all(|r| r.issued) {
            return;
        }
    }
    panic!("{n} captures were not issued: {} admitted", u.app.capture.shots.len());
}

/// A real changed viewport size, without allocating a readback image of that size.
fn resized_frame(u: &mut Ui, size: egui::Vec2) -> egui::FullOutput {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
    let app = &mut u.app;
    let mut out = u.ctx.run_ui(input, |ui| app.ui(ui));
    out.textures_delta.clear();
    out
}

/// Whether any viewport in the frame output issues a `Screenshot` command.
fn has_screenshot(out: &egui::FullOutput) -> bool {
    out.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Screenshot(_))))
}

/// Asserts reverse capture completion replies only to the matching request, then drains both
/// requests.
#[test]
fn two_captures_correlate_by_request_across_reverse_completion() {
    let mut u = Ui::new(Session::empty());
    let first = push(&mut u, 1, json!({}));
    let second = push(&mut u, 2, json!({}));
    settle_to_issue(&mut u, 2);
    let [a, b] = [0, 1].map(|i| u.app.capture.shots[i].spec.expect("geometry"));
    assert_eq!(a.viewport, b.viewport);
    assert!(a.size_px[0] > 0 && a.size_px[1] > 0);
    // The second capture arrives first; only its own request is completed.
    let slot_b = u.app.capture.shots[1].slot.clone().expect("issued");
    slot_b.complete(Arc::new(egui::ColorImage::filled(b.size_px, egui::Color32::GREEN)));
    u.frame(vec![]);
    let got = second.try_recv().expect("the second capture's reply");
    assert_eq!(got.id, json!(2));
    assert!(first.try_recv().is_err(), "the first is still in flight");
    assert_eq!(u.app.capture.shots.len(), 1);
    let slot_a = u.app.capture.shots[0].slot.clone().expect("issued");
    slot_a.complete(Arc::new(egui::ColorImage::filled(a.size_px, egui::Color32::RED)));
    u.frame(vec![]);
    let got = first.try_recv().expect("the first capture's reply");
    assert_eq!(got.id, json!(1));
    assert!(u.app.capture.shots.is_empty(), "no retained requests");
}

/// Asserts a cancelled capture gets no reply and its slot is released.
#[test]
fn a_cancelled_capture_never_replies_or_writes() {
    let mut u = Ui::new(Session::empty());
    let rx = push(&mut u, 3, json!({}));
    settle_to_issue(&mut u, 1);
    let spec = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    u.app.capture.shots[0].state.cancel_if_open();
    slot.complete(Arc::new(egui::ColorImage::filled(spec.size_px, egui::Color32::BLUE)));
    u.frame(vec![]);
    assert!(rx.try_recv().is_err(), "a cancelled transport gets no reply");
    assert!(u.app.capture.shots.is_empty(), "the slot is released");
}

/// A unique directory for a commit test; the caller removes it.
fn commit_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-capture-commit-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Asserts cancellation after encoding still prevents the final file write and reply.
#[test]
fn a_cancellation_after_encoding_prevents_the_final_write() {
    let dir = commit_dir("cancelled");
    let path = dir.join("shot.png");
    let gate = crate::control::RequestGate::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let clock = crate::FrameClock::default();
    let img = egui::ColorImage::filled([2, 2], egui::Color32::RED);
    // Encoding is complete before the transport cancels: the final gate must still refuse the file.
    let encoded = super::encode_shot(&img, Some(path.to_str().expect("utf-8 path"))).expect("encoded");
    assert!(!path.exists(), "encoding alone does not write");
    assert_eq!(gate.cancel_if_open(), crate::control::RequestState::Cancelled);
    assert!(!super::commit_shot(&gate, &json!(40), &tx, 100.0, &clock, Ok(encoded)), "a cancelled request does not commit");
    assert!(!path.exists(), "the cancellation landed before the final write");
    assert!(rx.try_recv().is_err(), "a cancelled transport gets no reply");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Asserts a capture that arrives past its deadline reports a timeout and writes no PNG.
#[test]
fn a_real_arrival_that_crosses_the_deadline_does_not_write_its_png() {
    let mut u = Ui::new(Session::empty());
    let dir = commit_dir("settle-expired");
    let path = dir.join("shot.png");
    let rx = push(&mut u, 60, json!({"path": path.to_string_lossy()}));
    settle_to_issue(&mut u, 1);
    let spec = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    let deadline = u.app.capture.shots[0].deadline;
    let epoch = u.app.capture.shots[0].epoch;
    slot.complete(Arc::new(egui::ColorImage::filled(spec.size_px, egui::Color32::RED)));
    // A frame's saved time may precede encoding; the final write must use a fresh clock.
    let stale_now = u.app.clock.now();
    u.app.clock.set_test_now(Some(deadline + 1.0));
    super::drain_completed(&mut u.app, &u.ctx, egui::ViewportId::ROOT, stale_now, epoch);
    let reply = rx.try_recv().expect("the late commit reports a timeout");
    assert!(reply.result.unwrap_err().contains("didn't deliver"));
    assert!(!path.exists(), "the final deadline check must precede the PNG write");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Asserts an expiry during encoding reports the timeout under the gate, marks the request
/// replied and writes no file.
#[test]
fn an_expiry_during_encoding_prevents_the_final_write_and_reports_timeout() {
    let dir = commit_dir("expired");
    let path = dir.join("shot.png");
    let gate = crate::control::RequestGate::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut clock = crate::FrameClock::default();
    clock.set_test_now(Some(5.0));
    let img = egui::ColorImage::filled([2, 2], egui::Color32::RED);
    let encoded = super::encode_shot(&img, Some(path.to_str().expect("utf-8 path"))).expect("encoded");
    // The gate is open, but the fresh clock read under it is already at the deadline.
    assert!(super::commit_shot(&gate, &json!(41), &tx, 5.0, &clock, Ok(encoded)), "the open request is settled once");
    let reply = rx.try_recv().expect("a timeout reply");
    assert_eq!(reply.id, json!(41));
    assert!(reply.result.unwrap_err().contains("didn't deliver"), "the late commit reports the same timeout");
    assert!(!path.exists(), "an expired request never writes its file");
    assert!(gate.is_replied(), "the timeout reply was committed under the gate");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Asserts a commit before the deadline writes the file and queues one reply, and a second
/// commit is refused.
#[test]
fn a_commit_before_the_deadline_writes_and_queues_its_reply_once() {
    let dir = commit_dir("committed");
    let path = dir.join("shot.png");
    let gate = crate::control::RequestGate::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut clock = crate::FrameClock::default();
    clock.set_test_now(Some(1.0));
    let img = egui::ColorImage::filled([2, 2], egui::Color32::GREEN);
    let encoded = super::encode_shot(&img, Some(path.to_str().expect("utf-8 path"))).expect("encoded");
    assert!(super::commit_shot(&gate, &json!(42), &tx, 10.0, &clock, Ok(encoded)), "an open request commits");
    let reply = rx.try_recv().expect("the committed reply");
    assert_eq!(reply.id, json!(42));
    assert_eq!(reply.result.expect("written")["path"], json!(path.to_string_lossy()));
    assert!(path.is_file(), "the file is written under the gate");
    assert!(gate.is_replied(), "a timeout now consumes the committed reply");
    // A second commit is refused: the gate is no longer open.
    let again = super::encode_shot(&img, Some(path.to_str().expect("utf-8 path"))).expect("encoded");
    assert!(!super::commit_shot(&gate, &json!(43), &tx, 10.0, &clock, Ok(again)));
    assert!(rx.try_recv().is_err(), "no duplicate reply");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Asserts a capture past its deadline sends one timeout reply, releases its slot, and late
/// pixels are ignored.
#[test]
fn a_capture_after_its_deadline_replies_once_and_late_pixels_are_inert() {
    let mut u = Ui::new(Session::empty());
    let rx = push(&mut u, 4, json!({}));
    settle_to_issue(&mut u, 1);
    let spec = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    u.advance_clock(crate::control::SHOT_TIMEOUT + 1.0);
    u.frame(vec![]);
    let reply = rx.try_recv().expect("timed out");
    assert_eq!(reply.id, json!(4));
    assert!(reply.result.unwrap_err().contains("didn't deliver"), "one timeout reply");
    assert!(u.app.capture.shots.is_empty(), "released");
    // The renderer finishing afterwards finds a closed slot: no reply, no write, no panic.
    slot.complete(Arc::new(egui::ColorImage::filled(spec.size_px, egui::Color32::RED)));
    u.frame(vec![]);
    assert!(rx.try_recv().is_err(), "a late capture is inert");
}

/// Asserts the snapshot count budget rejects an over-budget request before issuing, and every
/// admitted capture still completes under its own id.
#[test]
fn a_snapshot_batch_is_capped_before_issuing_over_the_count_budget() {
    let mut u = Ui::new(Session::empty());
    let mut rxs: Vec<_> = (0..crate::control::MAX_SHOTS as u64).map(|i| push(&mut u, 10 + i, json!({}))).collect();
    settle_to_issue(&mut u, crate::control::MAX_SHOTS);
    let over = push(&mut u, 99, json!({}));
    u.frame(vec![]);
    let reply = over.try_recv().expect("rejected at admission");
    assert!(reply.result.unwrap_err().contains("too many screenshots"), "budget error");
    assert_eq!(u.app.capture.shots.len(), crate::control::MAX_SHOTS);
    // Every admitted capture still completes with its own identity. Each drain removes its
    // request, so always complete the first remaining one: the vector shifts under the loop.
    for (i, rx) in rxs.iter_mut().enumerate() {
        let spec = u.app.capture.shots.first().expect("one left").spec.expect("geometry");
        let slot = u.app.capture.shots.first().and_then(|r| r.slot.clone()).expect("issued");
        slot.complete(Arc::new(egui::ColorImage::filled(spec.size_px, egui::Color32::RED)));
        u.frame(vec![]);
        assert_eq!(rx.try_recv().expect("answered").id, json!(10 + i as u64), "request {i} keeps its identity");
    }
    rxs.clear();
}

/// Asserts an undersized readback is not accepted but reissued with one retry and no reply.
#[test]
fn an_undersized_readback_is_reissued_instead_of_accepted() {
    let mut u = Ui::new(Session::empty());
    let rx = push(&mut u, 30, json!({}));
    settle_to_issue(&mut u, 1);
    let spec = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    assert!(spec.size_px[0] > 2, "room for a two-pixel undersized readback");
    // Smaller by two pixels on one axis: a scale mismatch, not the admitted surface.
    slot.complete(Arc::new(egui::ColorImage::filled([spec.size_px[0] - 2, spec.size_px[1]], egui::Color32::RED)));
    u.frame(vec![]);
    assert!(rx.try_recv().is_err(), "the undersized pixels are not accepted");
    let r = u.app.capture.shots.first().expect("reissued");
    assert_eq!(r.retries, 1, "one retry, not an error");
    assert!(!r.issued, "waits for the settled geometry again");
}

/// Asserts a one-pixel rounding difference is accepted as the same surface.
#[test]
fn a_one_pixel_rounding_difference_is_still_the_same_surface() {
    let mut u = Ui::new(Session::empty());
    let rx = push(&mut u, 31, json!({}));
    settle_to_issue(&mut u, 1);
    let spec = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    slot.complete(Arc::new(egui::ColorImage::filled([spec.size_px[0] - 1, spec.size_px[1] + 1], egui::Color32::RED)));
    u.frame(vec![]);
    let reply = rx.try_recv().expect("a pixel of rounding is accepted");
    assert_eq!(reply.id, json!(31));
    assert!(reply.result.is_ok());
    assert!(u.app.capture.shots.is_empty(), "released");
}

/// Asserts an A→B→A tab switch invalidates the admitted capture with "document changed".
#[test]
fn a_document_a_to_b_to_a_switch_is_not_the_admitted_capture() {
    let mut s = Session::empty();
    s.execute("file.new", &json!({"name": "A"})).unwrap();
    s.execute("file.new", &json!({"name": "B"})).unwrap();
    let (a, b) = (s.docs.iter().position(|d| d.doc.name == "A").expect("file A"), s.docs.iter().position(|d| d.doc.name == "B").expect("file B"));
    let mut u = Ui::new(s);
    u.app.session.active = a;
    let rx = push(&mut u, 32, json!({}));
    settle_to_issue(&mut u, 1);
    let spec = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    // A→B→A before the callback is drained: the final UID is A again, but its pixels were never
    // what this capture asked for.
    assert!(u.app.run("doc.switch", json!({"index": b})).is_some());
    assert!(u.app.run("doc.switch", json!({"index": a})).is_some());
    slot.complete(Arc::new(egui::ColorImage::filled(spec.size_px, egui::Color32::RED)));
    u.frame(vec![]);
    let reply = rx.try_recv().expect("answered");
    assert!(reply.result.unwrap_err().contains("document changed"), "the A→B→A switch invalidates the capture");
    assert!(u.app.capture.shots.is_empty(), "released");
}

/// Asserts a geometry change bumps the capture epoch and reissues instead of accepting old
/// pixels.
#[test]
fn a_geometry_change_reissues_instead_of_accepting_old_pixels() {
    let mut u = Ui::new(Session::empty());
    let rx = push(&mut u, 5, json!({}));
    settle_to_issue(&mut u, 1);
    let old = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    // A resize between issuance and arrival bumps the epoch.
    u.app
        .capture
        .geometry
        .insert(egui::ViewportId::ROOT, (CaptureSpec { size_px: [old.size_px[0] + 1, old.size_px[1]], ..old }, u.app.capture.epoch.wrapping_add(1)));
    slot.complete(Arc::new(egui::ColorImage::filled(old.size_px, egui::Color32::RED)));
    u.frame(vec![]);
    assert!(rx.try_recv().is_err(), "the old pixels are not accepted");
    let r = u.app.capture.shots.first().expect("reissued");
    assert_eq!(r.retries, 1, "one retry, not an error");
    assert!(!r.issued, "waits for the settled geometry again");
}

/// Asserts a translucent pixel round-trips through the PNG as straight RGBA without
/// darkening.
#[test]
fn a_translucent_pixel_round_trips_straight_rgba() {
    let img = egui::ColorImage::from_rgba_unmultiplied([1, 1], &[128, 64, 32, 128]);
    let reply = super::shot_reply(&img, None).expect("encoded");
    use base64::Engine as _;
    let png = base64::engine::general_purpose::STANDARD.decode(reply["data"].as_str().expect("data")).expect("base64");
    let decoded = image::load_from_memory(&png).expect("png").to_rgba8();
    assert_eq!(decoded.get_pixel(0, 0).0, [128, 64, 32, 128], "straight alpha is not darkened");
}

/// Asserts a capture is not issued before three completed real UI frames; the following
/// completed frame issues it.
#[test]
fn a_capture_is_not_issued_before_three_completed_real_ui_frames() {
    let mut u = Ui::new(Session::empty());
    let _rx = push(&mut u, 6, json!({}));
    u.frame(vec![]);
    assert_eq!(u.app.capture.shots.len(), 1, "admitted");
    assert!(!u.app.capture.shots[0].issued, "not on the frame that anchors the count");
    u.frame(vec![]);
    assert!(!u.app.capture.shots[0].issued, "not on the second frame");
    u.frame(vec![]);
    assert!(!u.app.capture.shots[0].issued, "the third frame has not completed when its pass 0 runs");
    u.frame(vec![]);
    assert!(u.app.capture.shots[0].issued, "issued once three frames have presented their layout");
}

/// Asserts a discarded sizing pass does not count toward capture settling, and the next
/// completed frame issues it.
#[test]
fn a_discarded_sizing_pass_does_not_count_as_a_settled_capture_frame() {
    let mut u = Ui::new(Session::empty());
    let _rx = push(&mut u, 33, json!({}));
    // Each `frame_raw_with_discard` presents a layout after rerunning a discarded first pass.
    // Only that completed frame counts; the discarded pass must not make the capture look settled.
    u.frame_raw_with_discard(egui::RawInput::default());
    u.frame_raw_with_discard(egui::RawInput::default());
    u.frame_raw_with_discard(egui::RawInput::default());
    assert!(!u.app.capture.shots[0].issued, "three multipass frames present only two prior layouts here; the third pass 0 is not settled");
    u.frame(vec![]);
    assert!(u.app.capture.shots[0].issued, "the next completed frame issues the capture");
}

/// Asserts pixels delivered while hidden force a draining paint and are answered rather than
/// timed out.
#[test]
fn a_hidden_window_that_delivers_pixels_is_asked_to_paint_the_draining_pass() {
    let mut u = Ui::new(Session::empty());
    let rx = push(&mut u, 7, json!({}));
    settle_to_issue(&mut u, 1);
    let spec = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    // The renderer completes the readback while the window is hidden; the screenshot command that
    // produced it is drained, so without a new forced paint the request would only time out.
    slot.complete(Arc::new(egui::ColorImage::filled(spec.size_px, egui::Color32::RED)));
    let mut input = egui::RawInput::default();
    input.viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo { minimized: Some(true), ..Default::default() });
    let ctx = u.ctx.clone();
    let out = ctx.run_logic(&input, |ctx| crate::control::background(&mut u.app, ctx));
    let asked = out
        .viewport_commands
        .get(&egui::ViewportId::ROOT)
        .is_some_and(|cmds| cmds.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaintWhileHidden)));
    assert!(asked, "the completed pixels keep the hidden window painting");
    // The forced paint drains the slot; the deadline is untouched.
    u.frame(vec![]);
    let reply = rx.try_recv().expect("the completed capture is answered, not timed out");
    assert_eq!(reply.id, json!(7));
    assert!(reply.result.unwrap()["width"].as_u64().is_some(), "written or returned");
    assert!(u.app.capture.shots.is_empty(), "released");
}

/// Asserts a capture for a window with unknown visibility is parked with its deadline and a
/// paint request, then times out once.
#[test]
fn a_capture_for_a_window_with_unknown_visibility_is_admitted_with_a_deadline() {
    let mut u = Ui::new(Session::empty());
    let rx = push(&mut u, 40, json!({}));
    // A minimised window may report no `visible` flag at all. The request must still be parked
    // with its deadline rather than left in `control_queue` waiting for a frame that never draws.
    let mut input = egui::RawInput::default();
    // Neither `visible` nor `minimized` is reported: the platform has not said whether the
    // window draws, so waiting for its frame would leave the request queued indefinitely.
    input.viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo::default());
    let ctx = u.ctx.clone();
    let out = ctx.run_logic(&input, |ctx| crate::control::background(&mut u.app, ctx));
    assert_eq!(u.app.capture.shots.len(), 1, "admitted, not left queued");
    assert!(u.app.control_queue.is_empty(), "the parked request no longer waits in the input queue");
    let r = &u.app.capture.shots[0];
    assert!(!r.issued);
    let now = u.app.clock.now();
    assert!((r.deadline - (now + crate::control::SHOT_TIMEOUT)).abs() < 1e-6, "the hidden request has its deadline");
    let asked = out
        .viewport_commands
        .get(&egui::ViewportId::ROOT)
        .is_some_and(|cmds| cmds.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaintWhileHidden)));
    assert!(asked, "the parked capture asks the window to paint");
    // The deadline still answers the transport when no frame draws.
    u.advance_clock(crate::control::SHOT_TIMEOUT + 2.0);
    u.frame(vec![]);
    let reply = rx.try_recv().expect("the parked capture times out");
    assert_eq!(reply.id, json!(40));
    assert!(reply.result.unwrap_err().contains("didn't deliver"));
    assert!(u.app.capture.shots.is_empty(), "released");
}

/// Asserts a completed capture is rejected when current geometry is unavailable, failing
/// closed.
#[test]
fn a_completed_capture_is_rejected_when_current_geometry_is_unavailable() {
    let mut u = Ui::new(Session::empty());
    let rx = push(&mut u, 41, json!({}));
    settle_to_issue(&mut u, 1);
    let spec = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    slot.complete(Arc::new(egui::ColorImage::filled(spec.size_px, egui::Color32::RED)));
    // A surface that cannot represent geometry at all (smaller than one point here). The pixels
    // may still match the old admitted spec, but they cannot be proven current: fail closed.
    u.advance_clock(1.0);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(0.0, 0.0))),
        time: Some(1.0),
        ..Default::default()
    };
    let app = &mut u.app;
    let ctx = u.ctx.clone();
    let mut out = ctx.run_ui(input, |ui| crate::control::drain(app, ui.ctx()));
    out.textures_delta.clear();
    let reply = rx.try_recv().expect("answered instead of accepted");
    assert_eq!(reply.id, json!(41));
    assert!(reply.result.unwrap_err().contains("geometry is unavailable"), "fail closed");
    assert!(u.app.capture.shots.is_empty(), "released");
}

/// Asserts a tab switch while a capture settles invalidates it with "document changed".
#[test]
fn a_capture_belongs_to_the_document_admitted_not_the_one_showing_when_it_arrives() {
    let mut s = Session::empty();
    s.execute("file.new", &json!({"name": "A"})).unwrap();
    s.execute("file.new", &json!({"name": "B"})).unwrap();
    let (a, b) = (s.docs.iter().position(|d| d.doc.name == "A").expect("file A"), s.docs.iter().position(|d| d.doc.name == "B").expect("file B"));
    let mut u = Ui::new(s);
    u.app.session.active = a;
    let rx = push(&mut u, 8, json!({}));
    u.frame(vec![]);
    assert_eq!(u.app.capture.shots.len(), 1, "admitted against A");
    // The tab switches while the capture settles: the pixels that arrive are B's, not A's.
    u.app.session.active = b;
    settle_to_issue(&mut u, 1);
    let spec = u.app.capture.shots[0].spec.expect("geometry");
    let slot = u.app.capture.shots[0].slot.clone().expect("issued");
    slot.complete(Arc::new(egui::ColorImage::filled(spec.size_px, egui::Color32::RED)));
    u.frame(vec![]);
    let reply = rx.try_recv().expect("answered");
    assert!(reply.result.unwrap_err().contains("document changed"), "a tab switch invalidates the capture");
    assert!(u.app.capture.shots.is_empty(), "released");
}

/// Asserts the capture budget covers the encoding and reply peaks, not just pixels, and an
/// over-budget resize is rejected before readback.
#[test]
fn the_capture_budget_counts_the_encoding_peak_not_only_the_pixels() {
    let fits = CaptureSpec { viewport: egui::ViewportId::ROOT, size_px: [4096, 4096], pixels_per_point: 1.0, zoom: 1.0 };
    let over = CaptureSpec { viewport: egui::ViewportId::ROOT, size_px: [5000, 5000], pixels_per_point: 1.0, zoom: 1.0 };
    // The encode phase holds the readback and the straight-RGBA copy while the PNG is built; the
    // reply phase holds the readback and PNG plus the base64 string after that copy is dropped.
    let raw = fits.bytes().expect("pixels");
    let png = raw + super::SHOT_PNG_OVERHEAD;
    let encode_phase = raw * 2 + png;
    let reply_phase = raw + png + (png / 3 + 1) * 4;
    assert_eq!(fits.cost(), Some(encode_phase.max(reply_phase)), "cost covers both live encoding phases");
    // 5000² × 4 is under the per-image pixel budget but its PNG copy and base64 reply are not.
    assert!(over.bytes().expect("pixels") <= super::MAX_SHOT_IMAGE_BYTES);
    assert!(over.cost().expect("cost") > super::MAX_SHOT_TOTAL_BYTES, "the reply is part of the bound");
    assert!(fits.cost().expect("cost") <= super::MAX_SHOT_TOTAL_BYTES);
    // The real resize path enforces that bound, even after admission at a smaller size.
    let mut u = Ui::new(Session::empty());
    let rx = push(&mut u, 9, json!({}));
    u.frame(vec![]);
    assert_eq!(u.app.capture.shots.len(), 1, "admitted");
    let out = resized_frame(&mut u, egui::vec2(5000.0, 5000.0));
    let reply = rx.try_recv().expect("rejected");
    assert!(reply.result.unwrap_err().contains("capture budget"), "the encoding peak is refused");
    assert!(u.app.capture.shots.is_empty(), "nothing retained");
    assert!(!has_screenshot(&out), "over-budget growth is rejected before any readback command");
}

/// Asserts resizing settling captures rechecks aggregate reserved bytes, rejects the request
/// that no longer fits, and makes the survivor reserve its real cost.
#[test]
fn resizing_settling_captures_rechecks_the_aggregate_reserved_bytes() {
    let mut u = Ui::new(Session::empty());
    let a = push(&mut u, 81, json!({}));
    let b = push(&mut u, 82, json!({}));
    u.frame(vec![]);
    assert_eq!(u.app.capture.shots.len(), 2, "both fit at the original size");
    let out = resized_frame(&mut u, egui::vec2(3200.0, 3200.0));
    let replies: Vec<_> = [a.try_recv(), b.try_recv()].into_iter().filter_map(Result::ok).collect();
    assert_eq!(replies.len(), 1, "each resized request fits alone, but not both together");
    assert!(replies[0].result.as_ref().unwrap_err().contains("capture budget"));
    assert_eq!(u.app.capture.shots.len(), 1);
    let remaining = &u.app.capture.shots[0];
    assert_eq!(remaining.spec.unwrap().size_px, [3200, 3200]);
    assert_eq!(remaining.reserved, remaining.spec.unwrap().cost().unwrap(), "the survivor reserves its real current cost");
    assert!(!remaining.issued);
    assert!(!has_screenshot(&out), "neither request was read back using the old small reservation");
}

/// Asserts an allowed resize restarts settling, issues only after three new-size frames, and
/// reserves the current cost.
#[test]
fn an_allowed_resize_restarts_settling_and_issues_with_the_current_reservation() {
    let mut u = Ui::new(Session::empty());
    let _rx = push(&mut u, 83, json!({}));
    u.frame(vec![]);
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(u.app.capture.shots[0].settle, 2);
    let size = egui::vec2(2000.0, 1200.0);
    let out = resized_frame(&mut u, size);
    assert!(!has_screenshot(&out), "the third old-size frame cannot issue a newly resized surface");
    assert_eq!(u.app.capture.shots[0].settle, 0);
    for _ in 0..2 {
        assert!(!has_screenshot(&resized_frame(&mut u, size)), "wait for three completed new-size frames");
    }
    let out = resized_frame(&mut u, size);
    assert!(has_screenshot(&out), "a valid settled capture still issues");
    let request = &u.app.capture.shots[0];
    assert!(request.issued);
    assert_eq!(request.spec.unwrap().size_px, [2000, 1200]);
    assert_eq!(request.reserved, request.spec.unwrap().cost().unwrap());
}

/// Queue an injected-input request (the desktop app's `raw_input_hook` is simulated by
/// `windows::take_input` in these tests).
fn push_input(u: &mut Ui, id: u64) -> std::sync::mpsc::Receiver<ControlReply> {
    let (tx, rx) = std::sync::mpsc::channel();
    u.app.control_queue.push(ControlRequest {
        id: json!(id),
        method: "ui.input".into(),
        params: json!({"events": [{"move": [10.0, 10.0]}]}),
        reply: tx,
        state: Default::default(),
    });
    rx
}

/// Asserts injected input is queued, applied by the presenting pass, and acknowledged once
/// after that pass.
#[test]
fn injected_input_is_acknowledged_only_after_the_pass_that_applies_it() {
    let mut u = Ui::new(Session::empty());
    let rx = push_input(&mut u, 20);
    u.frame(vec![]);
    assert!(rx.try_recv().is_err(), "queued, not applied yet");
    let events = crate::windows::take_input(&mut u.app, egui::ViewportId::ROOT);
    assert!(!events.is_empty(), "the batch the window draws with");
    assert!(u.app.control_synthetic, "the applying frame is fenced from the system clipboard");
    u.frame(vec![]);
    let reply = rx.try_recv().expect("acknowledged by the presented pass");
    assert_eq!(reply.id, json!(20));
    assert!(reply.result.is_ok());
    assert!(!u.app.control_synthetic, "the fence ends with the frame");
}

/// Asserts a discarded pass neither acknowledges nor replays an injected batch; the presented
/// pass acknowledges once.
#[test]
fn a_discarded_pass_does_not_acknowledge_or_lose_an_injected_batch() {
    let mut u = Ui::new(Session::empty());
    let rx = push_input(&mut u, 21);
    u.frame(vec![]);
    let events = crate::windows::take_input(&mut u.app, egui::ViewportId::ROOT);
    assert!(!events.is_empty());
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))),
        time: Some(1.0),
        ..Default::default()
    };
    let app = &mut u.app;
    let ctx = u.ctx.clone();
    let mut passes = 0;
    let mut out = ctx.run_ui(input, |ui| {
        passes += 1;
        app.ui(ui);
        if passes == 1 {
            ui.ctx().request_discard("test sizing pass");
        }
    });
    out.textures_delta.clear();
    assert_eq!(passes, 2);
    // The batch's side effects already happened; it is never replayed. Its acknowledgement waits
    // for the pass whose output is actually presented.
    assert_eq!(rx.try_recv().expect("acknowledged by the presented pass").id, json!(21));
    assert!(app.control_input.is_empty(), "the batch is not replayed");
    assert!(app.input_acks.is_empty() && app.applied_acks.is_empty());
}

/// Asserts injected text is applied once across a discarded sizing pass and acknowledged
/// once.
#[test]
fn injected_text_lands_once_across_a_discarded_sizing_pass() {
    // egui reruns a discarded pass with the same `RawInput`. The custom editor applied the text in
    // the first pass; the rerun must not insert it a second time.
    let mut s = Session::new();
    let id = NodeId(s.execute("text.create", &json!({"text": "X", "x": 40, "y": 40})).unwrap()["id"].as_u64().unwrap());
    let mut u = Ui::new(s);
    figcraft_engine::textedit::begin(&mut u.app.session, id, Some(1), false).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    u.app.control_queue.push(ControlRequest {
        id: json!(22),
        method: "ui.input".into(),
        params: json!({"events": [{"text": "abc"}]}),
        reply: tx,
        state: Default::default(),
    });
    u.frame(vec![]);
    let events = crate::windows::take_input(&mut u.app, egui::ViewportId::ROOT);
    assert!(!events.is_empty(), "the applying frame's batch");
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))),
        time: Some(1.0),
        events,
        ..Default::default()
    };
    let app = &mut u.app;
    let ctx = u.ctx.clone();
    let mut passes = 0;
    let mut out = ctx.run_ui(input, |ui| {
        passes += 1;
        app.ui(ui);
        if passes == 1 {
            ui.ctx().request_discard("a grid-like sizing pass");
        }
    });
    out.textures_delta.clear();
    assert_eq!(passes, 2, "egui reran the frame");
    let typed = match &app.session.doc().expect("a file").node(id).expect("the text layer").kind {
        figcraft_doc::NodeKind::Text(t) => t.characters.clone(),
        _ => panic!("not text"),
    };
    assert_eq!(typed, "Xabc", "the discarded pass's text was not applied twice");
    assert_eq!(rx.try_recv().expect("acknowledged once").id, json!(22));
    assert!(app.input_acks.is_empty() && app.applied_acks.is_empty());
}
