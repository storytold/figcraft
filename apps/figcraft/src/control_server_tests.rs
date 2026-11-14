#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::io::Read;
use std::net::Shutdown;
use std::sync::mpsc;
use std::time::Instant;

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    client.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    (client, listener.accept().unwrap().0)
}

fn queue() -> Queue {
    Arc::new(Mutex::new(Vec::new()))
}

fn context() -> CtxSlot {
    Arc::new(Mutex::new(None))
}

fn reply(reader: &mut BufReader<TcpStream>) -> Value {
    let mut line = String::new();
    assert!(reader.read_line(&mut line).unwrap() > 0);
    serde_json::from_str(&line).unwrap()
}

/// Build a `ui.inspect` control request with a throwaway reply channel.
fn request() -> ControlRequest {
    ControlRequest { id: json!(0), method: "ui.inspect".into(), params: json!({}), reply: mpsc::channel().0, state: Default::default() }
}

#[test]
fn rejects_an_oversized_unterminated_frame_without_dispatch() {
    let (mut client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(5)));
    // Generate bytes as they are written; no giant input string and no terminating newline.
    std::io::copy(&mut std::io::repeat(b' ').take((MAX_LINE + 1) as u64), &mut client).unwrap();
    let mut reader = BufReader::new(client);
    assert_eq!(reply(&mut reader), json!({"id": null, "ok": false, "error": "message too large"}));
    assert_eq!(reader.read(&mut [0]).unwrap(), 0);
    worker.join().unwrap().unwrap();
    assert!(pending.lock().unwrap().is_empty());
}

#[test]
fn valid_frames_keep_their_order_ids_and_parameters() {
    let (mut client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(5)));
    client.write_all(b"\r\n{\"id\":\"test\",\"method\":\"engine.execute\",\"params\":{\"command\":\"shape.rectangle\"}}\r\n").unwrap();
    let mut reader = BufReader::new(client);
    let start = Instant::now();
    let r = loop {
        if let Some(r) = pending.lock().unwrap().pop() {
            break r;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(r.id, "test");
    assert_eq!(r.method, "engine.execute");
    assert_eq!(r.params, json!({"command": "shape.rectangle"}));
    r.reply.send(figcraft_ui_egui::ControlReply { id: r.id, result: Ok(json!({"id": 42})) }).unwrap();
    assert_eq!(reply(&mut reader), json!({"id": "test", "ok": true, "result": {"id": 42}}));
    reader.get_ref().shutdown(Shutdown::Write).unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn a_line_that_is_not_json_ends_the_connection_before_anything_after_it_runs() {
    // What a browser sends when a web page posts to the port: the JSON line in its body never runs.
    let (mut client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(5)));
    let body = "{\"id\":1,\"method\":\"engine.execute\",\"params\":{\"command\":\"file.save\"}}\n";
    let http = format!("POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    client.write_all(http.as_bytes()).unwrap();
    let mut reader = BufReader::new(client);
    assert_eq!(reply(&mut reader)["ok"], false);
    assert_eq!(reader.read(&mut [0]).unwrap(), 0, "the connection is closed");
    worker.join().unwrap().unwrap();
    assert!(pending.lock().unwrap().is_empty(), "nothing was dispatched");
}

#[test]
fn full_queue_rejects_before_dispatch_and_ui_queue_stays_bounded() {
    let (mut client, server) = pair();
    let pending = queue();
    pending.lock().unwrap().extend((0..MAX_QUEUED_REQUESTS).map(|_| request()));
    let mut ui_queue = (0..MAX_QUEUED_REQUESTS - 1).map(|_| request()).collect();
    drain_queue(&pending, &mut ui_queue);
    assert_eq!(ui_queue.len(), MAX_QUEUED_REQUESTS);
    assert_eq!(pending.lock().unwrap().len(), MAX_QUEUED_REQUESTS - 1);
    pending.lock().unwrap().push(request());
    drain_queue(&pending, &mut ui_queue);
    assert_eq!(pending.lock().unwrap().len(), MAX_QUEUED_REQUESTS);
    let q = pending.clone();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(5)));
    client.write_all(b"{\"id\":1,\"method\":\"engine.execute\"}\n").unwrap();
    let mut reader = BufReader::new(client);
    assert_eq!(reply(&mut reader), json!({"id": 1, "ok": false, "error": "control request queue is full"}));
    assert_eq!(pending.lock().unwrap().len(), MAX_QUEUED_REQUESTS);
    ui_queue.clear();
    drain_queue(&pending, &mut ui_queue);
    assert_eq!(ui_queue.len(), MAX_QUEUED_REQUESTS);
    assert!(pending.lock().unwrap().is_empty());
    reader.get_ref().shutdown(Shutdown::Write).unwrap();
    worker.join().unwrap().unwrap();
}

/// Asserts a commit that writes the file and queues its reply wins the gate: the transport's
/// timeout branch consumes that reply and leaves the state `Replied`.
#[test]
fn a_committed_reply_wins_over_the_transport_timeout() {
    let dir = std::env::temp_dir().join(format!("figcraft-control-commit-{}", std::process::id()));
    let path = dir.join("shot.png");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let gate = RequestGate::new();
    let (tx, rx) = mpsc::channel();
    // The UI's final commit gets the gate first: the file and reply are both committed before the
    // transport's timeout branch can look at the gate.
    let committed = gate.commit(|| {
        std::fs::write(&path, b"png").unwrap();
        tx.send(ControlReply { id: json!(7), result: Ok(json!({"path": path.to_string_lossy()})) }).unwrap();
    });
    assert!(committed.is_some());
    assert!(path.is_file());
    let reply = after_timeout(&rx, &gate).expect("the queued commit is consumed");
    assert_eq!(reply.id, json!(7));
    assert!(reply.result.is_ok());
    assert_eq!(gate.state(), RequestState::Replied, "the timeout must not overwrite a commit");
    assert!(path.is_file(), "a winning commit keeps its file");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Asserts a zero-duration wait reports `Timeout` and cancels the open gate.
#[test]
fn a_timeout_cancels_an_open_request() {
    let gate = RequestGate::new();
    let (_tx, rx) = mpsc::channel::<ControlReply>();
    assert!(matches!(wait_for_reply(&rx, &gate, Duration::ZERO), Err(WaitFailure::Timeout)));
    assert_eq!(gate.state(), RequestState::Cancelled, "nothing may write after the timeout");
}

/// Asserts a closed reply channel reports `Disconnected` and leaves the gate `Open`, not
/// cancelled.
#[test]
fn a_disconnected_reply_channel_is_not_reported_as_a_timeout() {
    let gate = RequestGate::new();
    let (tx, rx) = mpsc::channel::<ControlReply>();
    drop(tx);
    assert!(matches!(wait_for_reply(&rx, &gate, Duration::from_secs(1)), Err(WaitFailure::Disconnected)));
    assert_eq!(gate.state(), RequestState::Open, "a closed app is not a cancel decision");
}

/// #1673: a request the UI already started can't be cancelled any more. Its timeout says it is
/// still running and will take effect, not that the window is hidden; one that finished just as
/// the timeout fired gets its real reply.
#[test]
fn a_started_request_times_out_as_still_running() {
    let gate = RequestGate::new();
    assert!(gate.start(), "an open request starts");
    let (tx, rx) = mpsc::channel::<ControlReply>();
    let Err(failure) = wait_for_reply(&rx, &gate, Duration::ZERO) else { panic!("no reply was sent") };
    assert_eq!(failure, WaitFailure::StillRunning);
    assert_eq!(gate.state(), RequestState::Running, "the timeout doesn't cancel a running request");
    let message = timeout_message(&failure, false, Duration::from_secs(60));
    assert!(message.contains("will take effect") && !message.contains("window"), "{message}");
    tx.send(ControlReply { id: json!(3), result: Ok(json!({"done": true})) }).unwrap();
    assert_eq!(wait_for_reply(&rx, &gate, Duration::ZERO).unwrap().id, json!(3), "a reply that made it is the truth");
}

/// #1673: a request the transport gave up on before the UI started it never starts, and its
/// timeout says it wasn't applied; only requests that need a drawn frame blame the window.
#[test]
fn a_request_cancelled_before_it_started_never_runs() {
    let gate = RequestGate::new();
    let (_tx, rx) = mpsc::channel::<ControlReply>();
    let Err(failure) = wait_for_reply(&rx, &gate, Duration::ZERO) else { panic!("no reply was sent") };
    assert_eq!(failure, WaitFailure::Timeout);
    assert!(!gate.start(), "the UI must not run a cancelled request");
    assert_eq!(gate.state(), RequestState::Cancelled);
    let message = |needs_frame| timeout_message(&failure, needs_frame, Duration::from_secs(60));
    assert!(message(false).contains("not applied"));
    assert!(!message(false).contains("window on screen"));
    assert!(message(true).contains("window on screen"), "screenshots and input still need the window");
}

/// #1696: a request's `timeoutMs` sets its wait, capped; anything but a whole number leaves the
/// default.
#[test]
fn a_request_sets_its_own_timeout_within_the_cap() {
    let default = Duration::from_secs(60);
    let wait = |v: Value| request_timeout(&v, default);
    assert_eq!(wait(json!({"id": 1, "method": "engine.execute"})), default, "older clients keep the default");
    assert_eq!(wait(json!({"timeoutMs": 300_000})), Duration::from_secs(300));
    assert_eq!(wait(json!({"timeoutMs": 500})), Duration::from_millis(500), "a shorter wait too");
    assert_eq!(wait(json!({"timeoutMs": u64::MAX})), MAX_TIMEOUT, "never unbounded");
    for junk in [json!({"timeoutMs": "600000"}), json!({"timeoutMs": -1}), json!({"timeoutMs": 1.5}), json!({"timeoutMs": null})] {
        assert_eq!(wait(junk), default);
    }
}

/// #1696: over a connection, a raised `timeoutMs` outlasts the connection's default.
#[test]
fn a_raised_timeout_outlasts_the_default() {
    let (mut client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    // Written first: the connection's 50 ms is also how long it waits for a line.
    client.write_all(b"{\"id\":1,\"method\":\"ui.inspect\",\"timeoutMs\":5000}\n").unwrap();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_millis(50)));
    let mut reader = BufReader::new(client);
    let start = Instant::now();
    let r = loop {
        if let Some(r) = pending.lock().unwrap().pop() {
            break r;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    };
    // Well past the connection's 50 ms: the request's own wait still holds.
    std::thread::sleep(Duration::from_millis(200));
    r.reply.send(ControlReply { id: r.id, result: Ok(json!("late")) }).unwrap();
    assert_eq!(reply(&mut reader), json!({"id": 1, "ok": true, "result": "late"}));
    drop(reader);
    worker.join().unwrap().ok();
}

/// #1696: the timeout's message says how long the request waited, not always 60 s.
#[test]
fn a_timeout_names_the_wait_the_request_asked_for() {
    let (mut client, server) = pair();
    let worker = std::thread::spawn(move || serve(server, queue(), context(), Duration::from_secs(5)));
    client.write_all(b"{\"id\":2,\"method\":\"ui.inspect\",\"timeoutMs\":100}\n").unwrap();
    let mut reader = BufReader::new(client);
    let error = reply(&mut reader)["error"].as_str().unwrap().to_string();
    assert!(error.contains("after 1 s") && error.contains("not applied"), "{error}");
    reader.get_ref().shutdown(Shutdown::Write).unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn idle_and_partial_frame_connections_expire() {
    for bytes in [b"".as_slice(), b"{\"id\":"] {
        let (mut client, server) = pair();
        let (done, finished) = mpsc::channel();
        let pending = queue();
        let q = pending.clone();
        let worker = std::thread::spawn(move || done.send(serve(server, q, context(), Duration::from_millis(50))));
        client.write_all(bytes).unwrap();
        let err = finished.recv_timeout(Duration::from_secs(5)).unwrap().unwrap_err();
        assert!(matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut));
        assert!(pending.lock().unwrap().is_empty());
        worker.join().unwrap().unwrap();
    }
}

/// A failed accept is followed by a wait before the next one, so a listener that keeps failing
/// does not spin, and the connections around the failures still come through.
#[test]
fn a_failed_accept_waits_before_the_next() {
    let failed = || Err(std::io::Error::other("no file descriptors left"));
    let incoming = vec![failed(), Ok(1), failed(), failed(), Ok(2)];
    let mut waits = 0;
    let connections: Vec<i32> = accepted(incoming.into_iter(), || waits += 1).collect();
    assert_eq!(connections, [1, 2]);
    assert_eq!(waits, 3, "one wait per failed accept");
}

#[test]
fn connection_limit_rejects_excess_clients_and_reuses_finished_slots() {
    let pending = queue();
    let ctx = context();
    let mut workers = Vec::new();
    let mut clients = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        let (client, server) = pair();
        spawn_connection(
            server,
            pending.clone(),
            ctx.clone(),
            Arc::new(figcraft_engine::events::EventLog::default()),
            &mut workers,
            Duration::from_secs(5),
        );
        clients.push(client);
    }
    assert_eq!(workers.len(), MAX_CONNECTIONS);
    let (mut extra, server) = pair();
    spawn_connection(
        server,
        pending.clone(),
        ctx.clone(),
        Arc::new(figcraft_engine::events::EventLog::default()),
        &mut workers,
        Duration::from_secs(5),
    );
    // The refused client is told why, then its connection closes: never a bare reset.
    let mut refusal = String::new();
    extra.read_to_string(&mut refusal).unwrap();
    let refusal: serde_json::Value = serde_json::from_str(refusal.trim()).unwrap();
    assert_eq!(refusal["ok"], false);
    assert!(refusal["error"].as_str().is_some_and(|e| e.contains("too many control connections")), "{refusal}");
    assert_eq!(workers.len(), MAX_CONNECTIONS);
    for client in clients {
        client.shutdown(Shutdown::Both).unwrap();
    }
    let start = Instant::now();
    while workers.iter().any(|worker| !worker.is_finished()) {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    }
    let (mut client, server) = pair();
    spawn_connection(server, pending, ctx, Arc::new(figcraft_engine::events::EventLog::default()), &mut workers, Duration::from_secs(5));
    assert_eq!(workers.len(), 1);
    client.write_all(b"invalid\n").unwrap();
    assert_eq!(reply(&mut BufReader::new(client.try_clone().unwrap()))["ok"], false);
    // The server closes this connection after answering an invalid request, and shutting down
    // a socket whose peer has already closed is ENOTCONN on macOS and the BSDs where Linux
    // returns Ok. Closing the client is what the worker is waiting for, so the result is moot:
    // the other shutdowns in this file run while the peer is still open and stay checked.
    let _ = client.shutdown(Shutdown::Both);
    workers.pop().unwrap().join().unwrap();
}

/// A client over the limit that had already sent its request still reads why it is refused, then
/// a clean end. Closing over the unread request resets the connection instead, and a reset can
/// take the refusal away before the client reads it.
#[test]
fn a_refused_client_that_already_wrote_reads_the_refusal_then_a_clean_end() {
    let (mut client, server) = pair();
    client.write_all(b"{\"id\":1,\"method\":\"document.inspect\"}\n").unwrap();
    // The request is in the server's socket before the server refuses.
    assert_eq!(server.peek(&mut [0]).unwrap(), 1);
    refuse(server);
    let mut reader = BufReader::new(client);
    let refusal = reply(&mut reader);
    assert_eq!(refusal["ok"], false);
    assert!(refusal["error"].as_str().is_some_and(|e| e.contains("too many control connections")), "{refusal}");
    let end = reader.read(&mut [0]);
    assert!(matches!(end, Ok(0)), "the connection ends cleanly after the refusal, not by a reset: {end:?}");
}

/// A client that sent more after a line that isn't JSON (a file of requests whose first line is
/// broken) still reads why the connection ends, then a clean end. Closing over what it sent and
/// nobody read resets the connection instead, and the reset takes the reason away.
#[test]
fn a_client_that_sent_more_after_a_broken_line_reads_why_then_a_clean_end() {
    let (mut client, server) = pair();
    // More than the server reads at once: the rest waits in its socket when it gives up.
    let mut sent = b"{broken\n".to_vec();
    sent.resize(32 * 1024, b' ');
    client.write_all(&sent).unwrap();
    assert_eq!(server.peek(&mut [0]).unwrap(), 1);
    let worker = std::thread::spawn(move || serve(server, queue(), context(), Duration::from_secs(5)));
    let mut reader = BufReader::new(client);
    let mut line = String::new();
    let read = reader.read_line(&mut line);
    assert!(matches!(read, Ok(n) if n > 0), "the reason is lost with the connection: {read:?}");
    let reason: Value = serde_json::from_str(&line).unwrap();
    assert!(reason["error"].as_str().is_some_and(|e| e.starts_with("bad JSON")), "{reason}");
    let end = reader.read(&mut [0]);
    assert!(matches!(end, Ok(0)), "the connection ends cleanly after the reason, not by a reset: {end:?}");
    worker.join().unwrap().unwrap();
}

#[test]
fn a_client_that_stops_reading_cannot_hold_a_worker() {
    let (mut client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    let (done, finished) = mpsc::channel();
    let worker = std::thread::spawn(move || done.send(serve(server, q, context(), Duration::from_millis(100))));
    client.write_all(b"{\"id\":1,\"method\":\"document.inspect\"}\n").unwrap();
    let start = Instant::now();
    let r = loop {
        if let Some(r) = pending.lock().unwrap().pop() {
            break r;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    };
    r.reply.send(figcraft_ui_egui::ControlReply { id: r.id, result: Ok(json!("x".repeat(8 * 1024 * 1024))) }).unwrap();
    let err = finished.recv_timeout(Duration::from_secs(5)).unwrap().unwrap_err();
    assert!(matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut));
    worker.join().unwrap().unwrap();
}

/// A client that leaves while its request waits for the UI has given it up (killed, out of budget,
/// its own timeout): the request is cancelled then, not when the server's wait runs out, so the
/// UI does not run it later for nobody.
#[test]
fn a_request_whose_client_left_is_cancelled_long_before_its_timeout() {
    let (mut client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(60)));
    client.write_all(b"{\"id\":1,\"method\":\"engine.execute\",\"params\":{\"command\":\"edit.delete\"}}\n").unwrap();
    let gate = queued_gate(&pending);
    assert_eq!(gate.state(), RequestState::Open, "queued, not started");

    drop(client);
    assert_cancelled_within(&gate, Duration::from_secs(15), "the client left and its request still waits to run");
    assert!(!gate.start(), "the UI must not run a request nobody waits for");
    worker.join().unwrap().unwrap();
}

/// A client that only stopped sending (`| nc -N`, a half-close) still reads: the look that finds
/// a client gone must not take this one for it, however long its request waits for the UI.
#[test]
fn a_client_that_only_stopped_sending_gets_its_answer_after_a_long_wait() {
    let (mut client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(10)));
    client.write_all(b"{\"id\":1,\"method\":\"ui.inspect\"}\n").unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let start = Instant::now();
    let r = loop {
        if let Some(r) = pending.lock().unwrap().pop() {
            break r;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    };
    // Long enough for the server to look at its client and to hear how the look was taken.
    std::thread::sleep(CLIENT_CHECK + PROBE_ANSWER * 3);
    assert_eq!(r.state.state(), RequestState::Open, "the request of a client that still reads was cancelled");
    r.reply.send(figcraft_ui_egui::ControlReply { id: r.id, result: Ok(json!({})) }).unwrap();
    assert_eq!(reply(&mut BufReader::new(client)), json!({"id": 1, "ok": true, "result": {}}));
    worker.join().unwrap().unwrap();
}

#[test]
fn events_poll_is_answered_without_the_ui_queue() {
    let (mut client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    let log = Arc::new(figcraft_engine::events::EventLog::default());
    log.push("comment.added", "Doc", None, json!({"commentId": 7}));
    let l = log.clone();
    let worker = std::thread::spawn(move || serve_with_events(server, q, context(), l, Duration::from_secs(5)));
    client.write_all(b"{\"id\":1,\"method\":\"events.poll\",\"params\":{\"after\":0,\"timeoutMs\":0}}\n").unwrap();
    let mut reader = BufReader::new(client);
    let r = reply(&mut reader);
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["events"].as_array().unwrap().len(), 1);
    assert_eq!(r["result"]["events"][0]["kind"], "comment.added");
    assert_eq!(r["result"]["events"][0]["data"]["commentId"], 7);
    assert_eq!(r["result"]["cursor"], 1);
    assert!(pending.lock().unwrap().is_empty());
    reader.get_ref().shutdown(Shutdown::Write).unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn a_comment_added_in_the_session_is_pollable() {
    let mut session = figcraft_engine::Session::new();
    let log = session.events.clone();
    let id = session.execute("comment.add", &json!({"x": 10, "y": 20, "text": "Fix the padding"})).unwrap()["id"].clone();
    let r = event_poll(&log, &json!({"after": 0, "timeoutMs": 0})).unwrap();
    assert_eq!(r["events"].as_array().unwrap().len(), 1);
    assert_eq!(r["events"][0]["kind"], "comment.added");
    assert_eq!(r["events"][0]["data"]["commentId"], id);
}

#[test]
fn a_limit_sized_frame_is_accepted_at_eof() {
    let (mut client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(5)));
    let message = b"{\"id\":1,\"method\":\"ui.inspect\"}";
    client.write_all(message).unwrap();
    std::io::copy(&mut std::io::repeat(b' ').take((MAX_LINE - message.len()) as u64), &mut client).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let start = Instant::now();
    let r = loop {
        if let Some(r) = pending.lock().unwrap().pop() {
            break r;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(r.id, 1);
    assert_eq!(r.method, "ui.inspect");
    r.reply.send(figcraft_ui_egui::ControlReply { id: r.id, result: Ok(json!({})) }).unwrap();
    assert_eq!(reply(&mut BufReader::new(client)), json!({"id": 1, "ok": true, "result": {}}));
    worker.join().unwrap().unwrap();
}

/// Asserts expired requests stop occupying the UI queue cap, a later request is admitted
/// and answered, and a window that finally draws discards its expired command before
/// dispatch.
#[test]
fn timed_out_requests_release_a_full_deferred_ui_queue() {
    use figcraft_ui_egui::{FigApp, NoServices, control, windows};

    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = FigApp::new(figcraft_engine::Session::new(), Box::new(NoServices));
    let window = windows::open(&mut app, &ctx, Some(0), None, None).unwrap();
    let pending = queue();
    let mut clients = Vec::new();
    let mut workers = Vec::new();
    for id in 0..MAX_QUEUED_REQUESTS {
        let (mut client, server) = pair();
        let q = pending.clone();
        workers.push(std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(1))));
        writeln!(client, "{}", json!({"id": id, "method": "engine.execute", "params": {"window": window, "command": "file.new"}})).unwrap();
        clients.push(BufReader::new(client));
    }
    let start = Instant::now();
    while pending.lock().unwrap().len() < MAX_QUEUED_REQUESTS {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    }
    drain_queue(&pending, &mut app.control_queue);
    control::drain(&mut app, &ctx);
    assert_eq!(app.control_queue.len(), MAX_QUEUED_REQUESTS, "the other window has not drawn");
    for client in &mut clients {
        assert!(reply(client)["error"].as_str().unwrap().contains("timed out"));
        client.get_ref().shutdown(Shutdown::Write).unwrap();
    }
    assert!(app.control_queue.iter().all(ControlRequest::is_cancelled));
    let expired = app.control_queue.pop().unwrap();
    // Include expired requests in the socket queue, too: admission must reclaim their slots.
    pending.lock().unwrap().extend((0..MAX_QUEUED_REQUESTS).map(|_| {
        let r = request();
        r.state.cancel_if_open();
        r
    }));
    let (mut client, server) = pair();
    let q = pending.clone();
    workers.push(std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(5))));
    client.write_all(b"{\"id\":100,\"method\":\"window.list\",\"params\":{\"window\":0}}\n").unwrap();
    let start = Instant::now();
    while !pending.lock().unwrap().iter().any(|r| r.id == 100) {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    }
    drain_queue(&pending, &mut app.control_queue);
    assert_eq!(app.control_queue.len(), 1, "expired UI requests no longer occupy the cap");
    control::drain(&mut app, &ctx);
    let mut client = BufReader::new(client);
    assert_eq!(reply(&mut client)["ok"], true);
    client.get_ref().shutdown(Shutdown::Write).unwrap();
    assert!(app.control_queue.is_empty());
    // A window that finally draws must discard its expired command before dispatch, too.
    let documents = app.session.docs.len();
    app.windows.current = window;
    app.control_queue.push(expired);
    control::drain(&mut app, &ctx);
    assert!(app.control_queue.is_empty());
    assert_eq!(app.session.docs.len(), documents);
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
}

#[test]
fn events_poll_rejects_an_unknown_topic_without_the_ui_queue() {
    let (client, server) = pair();
    let pending = queue();
    let q = pending.clone();
    let log = Arc::new(figcraft_engine::events::EventLog::default());
    let l = log.clone();
    let worker = std::thread::spawn(move || serve_with_events(server, q, context(), l, Duration::from_secs(5)));
    let mut reader = BufReader::new(client);
    reader.get_mut().write_all(b"{\"id\":1,\"method\":\"events.poll\",\"params\":{\"topics\":[\"nope\"],\"timeoutMs\":0}}\n").unwrap();
    let r = reply(&mut reader);
    assert_eq!(r["id"], 1);
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().is_some_and(|e| e.contains("unknown topic")), "{r}");
    assert!(pending.lock().unwrap().is_empty());
    reader.get_ref().shutdown(Shutdown::Write).unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn events_poll_reports_reset_for_a_cursor_ahead_of_the_log() {
    let (client, server) = pair();
    let log = Arc::new(figcraft_engine::events::EventLog::default());
    log.push("comment.added", "Doc", None, json!({"commentId": 1}));
    let q = queue();
    let l = log.clone();
    let worker = std::thread::spawn(move || serve_with_events(server, q, context(), l, Duration::from_secs(5)));
    let mut reader = BufReader::new(client);
    reader.get_mut().write_all(b"{\"id\":1,\"method\":\"events.poll\",\"params\":{\"after\":2,\"timeoutMs\":0}}\n").unwrap();
    let r = reply(&mut reader);
    assert_eq!(r["result"]["reset"], true, "{r}");
    assert!(r["result"]["events"].as_array().unwrap().is_empty(), "{r}");
    assert_eq!(r["result"]["cursor"], 1);
    reader.get_ref().shutdown(Shutdown::Write).unwrap();
    worker.join().unwrap().unwrap();
}

#[test]
fn a_finished_connection_leaves_no_handle_behind() {
    // Each accepted connection used to keep a file descriptor for the life of the server: after
    // about a thousand requests the app could serve none.
    let stop = start(0, Duration::ZERO, queue(), context(), Arc::new(figcraft_engine::events::EventLog::default())).unwrap();
    for _ in 0..3 {
        drop(TcpStream::connect(("127.0.0.1", stop.port())).unwrap());
    }
    let begun = Instant::now();
    while !stop.open.lock().unwrap().is_empty() {
        assert!(begun.elapsed() < Duration::from_secs(5), "{} handles are still kept", stop.open.lock().unwrap().len());
        std::thread::sleep(Duration::from_millis(10));
    }
    stop.stop();
}

// ---- Departures the server must see (N1, N2, N2 bis) and the end of the server (N3). ----

/// The gate of the first request waiting in `pending`, without taking it out of the queue.
fn queued_gate(pending: &Queue) -> RequestGate {
    let start = Instant::now();
    loop {
        if let Some(r) = pending.lock().unwrap().first() {
            return r.state.clone();
        }
        assert!(start.elapsed() < Duration::from_secs(5), "the request never reached the queue");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Wait (polling, at most `within`) until `gate` is `Cancelled`; panics with `what` otherwise.
fn assert_cancelled_within(gate: &RequestGate, within: Duration, what: &str) {
    let left = Instant::now();
    while gate.state() != RequestState::Cancelled {
        assert!(left.elapsed() < within, "{what}: still {:?} after {:?}", gate.state(), left.elapsed());
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// N1. Bytes the client sent beyond the server's read buffer stay in the socket. `look_at` must
/// not take them for a client still there, or the request runs for nobody: it has to see the
/// departure whatever is still unread.
#[test]
fn a_client_that_left_with_unread_bytes_in_the_socket_does_not_leave_its_request_to_run() {
    let (mut client, server) = pair();
    let probe = server.try_clone().unwrap();
    let pending = queue();
    let q = pending.clone();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(60)));
    // The request, then far more than the 8 KiB the server reads at once: the rest waits in its socket.
    let mut sent = b"{\"id\":1,\"method\":\"ui.inspect\"}\n".to_vec();
    sent.resize(64 * 1024, b' ');
    client.write_all(&sent).unwrap();
    let gate = queued_gate(&pending);
    // Calibration 1: bytes really remain in the server's socket (else this proves nothing).
    let start = Instant::now();
    loop {
        match probe.peek(&mut [0]) {
            Ok(n) if n > 0 => break,
            other => assert!(start.elapsed() < Duration::from_secs(2), "no byte left in the socket: {other:?}"),
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    // Calibration 2: with the client still there, several looks pass and the request stays open
    // (a window of observation, not a synchronisation: a wrong cancel would show in it).
    std::thread::sleep(CLIENT_CHECK + PROBE_ANSWER * 2);
    assert_eq!(gate.state(), RequestState::Open, "the request of a present client was cancelled");
    drop(client);
    assert_cancelled_within(&gate, Duration::from_secs(15), "the client left with bytes unread and its request still waits to run");
    assert!(!gate.start(), "the UI must not run a request nobody waits for");
    worker.join().unwrap().unwrap();
}

/// N2. A client that closed only its sending side (FIN) is `Listening` once it took the probe
/// byte; the wait must keep looking at it until the reply, or its later full departure goes
/// unseen. The neighbour test
/// `a_client_that_only_stopped_sending_gets_its_answer_after_a_long_wait` keeps the other half.
#[test]
fn a_half_closed_client_that_then_leaves_entirely_does_not_leave_its_request_to_run() {
    let (mut client, server) = pair();
    let probe = server.try_clone().unwrap();
    let pending = queue();
    let q = pending.clone();
    let worker = std::thread::spawn(move || serve(server, q, context(), Duration::from_secs(60)));
    client.write_all(b"{\"id\":1,\"method\":\"ui.inspect\"}\n").unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let gate = queued_gate(&pending);
    // Calibration 1: the server sees the end of what it sent (EOF), not a byte.
    assert_eq!(probe.peek(&mut [0]).unwrap(), 0);
    // Calibration 2: the server asked, and the client took the byte (the read blocks until it comes).
    let mut byte = [0];
    assert_eq!(client.read(&mut byte).unwrap(), 1);
    assert_eq!(byte[0], b' ');
    // The server's next look (PROBE_ANSWER later) finds it `Listening`: a margin, the look is not observable.
    std::thread::sleep(PROBE_ANSWER * 5);
    // Calibration 3: half-closed and still open, the request waits.
    assert_eq!(gate.state(), RequestState::Open, "the request of a client that still reads was cancelled");
    drop(client);
    assert_cancelled_within(&gate, Duration::from_secs(15), "the half-closed client left entirely and its request still waits to run");
    worker.join().unwrap().unwrap();
}

/// N2 bis. `events.poll` waits up to `timeoutMs` on the log. It must look at its client
/// meanwhile, or a client that left holds the connection thread for the whole wait. The poll's
/// timeout is 20 s and the test's deadline 12 s; a client that left is seen within about 2 s, so
/// the deadline has a wide margin. The poll itself is light to test: the log is empty, so it
/// waits for its whole timeout.
#[test]
fn an_events_poll_wait_ends_when_its_client_leaves() {
    let (mut client, server) = pair();
    let log = Arc::new(figcraft_engine::events::EventLog::default());
    let (done, ended) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let r = serve_with_events(server, queue(), context(), log, Duration::from_secs(30));
        let _ = done.send(());
        r
    });
    client.write_all(b"{\"id\":1,\"method\":\"events.poll\",\"params\":{\"after\":0,\"timeoutMs\":20000}}\n").unwrap();
    // Calibration: the poll is waiting, no answer comes while the client stays.
    client.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    let early = client.read(&mut [0]);
    assert!(matches!(&early, Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)), "{early:?}");
    assert!(ended.try_recv().is_err(), "the connection ended while its client stayed");
    drop(client);
    assert!(ended.recv_timeout(Duration::from_secs(12)).is_ok(), "the client left and its events.poll still holds the connection");
    worker.join().unwrap().unwrap_or_default();
}

/// A started server whose connection has a request queued in `pending`, for the stop tests.
fn started(pending: &Queue) -> (Stop, TcpStream) {
    let server = start(0, Duration::ZERO, pending.clone(), context(), Arc::new(EventLog::default())).unwrap();
    let client = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    (server, client)
}

/// Wait until the accepting thread tracks one connection (it pushes it just before it spawns the
/// connection thread), so `stop` cannot beat the connection it is meant to find.
fn wait_tracked(server: &Stop) {
    let start = Instant::now();
    while server.open.lock().unwrap().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(5), "the connection was never accepted");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// N3 (agent_control's sequence: `stop`, then the queue is drained). A request queued by a client
/// still connected must not be live once `stop` returned: `stop` has to join the connection
/// threads, not only the accepting one. A thread left running only cancels at its next look
/// (>= CLIENT_CHECK later), and the drain below would still find the request `Open`.
#[test]
fn a_request_queued_before_stop_is_not_live_when_stop_returns() {
    let pending = queue();
    let (server, mut client) = started(&pending);
    client.write_all(b"{\"id\":1,\"method\":\"ui.inspect\"}\n").unwrap();
    let _gate = queued_gate(&pending);
    server.stop();
    let mut ui = Vec::new();
    drain_queue(&pending, &mut ui);
    assert!(ui.is_empty(), "{} request(s) still run after stop", ui.len());
    assert!(pending.lock().unwrap().iter().all(ControlRequest::is_cancelled));
}

/// N3, the late push, made deterministic without a seam: the test holds the queue's lock, so the
/// connection thread is held right before its push (at its `queue.lock()`); `stop` runs
/// meanwhile; the lock is then released. Nothing live may be in the queue once `stop` has
/// returned. A `stop` that neither waits for that thread nor makes its push refuse lets the
/// request land after `stop`. Either way passes: `stop` joins the thread (it then returns only
/// after the release), or the push refuses once stopping.
#[test]
fn a_connection_held_before_its_push_cannot_queue_a_request_after_stop() {
    let pending = queue();
    let (server, mut client) = started(&pending);
    let hold = pending.lock().unwrap();
    client.write_all(b"{\"id\":1,\"method\":\"ui.inspect\"}\n").unwrap();
    wait_tracked(&server);
    let (returned, stopped) = mpsc::channel();
    let stopper = std::thread::spawn(move || {
        server.stop();
        let _ = returned.send(());
    });
    // A `stop` that joins waits for the lock; one that only refuses the push returns at once: both
    // are allowed here.
    let returned_early = stopped.recv_timeout(Duration::from_millis(300)).is_ok();
    drop(hold);
    if !returned_early {
        stopped.recv_timeout(Duration::from_secs(5)).expect("stop never returned after the queue was released");
    }
    stopper.join().unwrap();
    // A `stop` that joined the thread leaves nothing more to come: one look is conclusive. A `stop`
    // that returned early leaves the held thread to push right after the release: wait (up to 5 s,
    // ending at once on the push) for it to land, then look at what it queued.
    let waited = Instant::now();
    while returned_early && pending.lock().unwrap().is_empty() && waited.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        pending.lock().unwrap().iter().all(ControlRequest::is_cancelled),
        "a request reached the queue live after stop returned (returned_early: {returned_early})"
    );
    drop(client);
}
