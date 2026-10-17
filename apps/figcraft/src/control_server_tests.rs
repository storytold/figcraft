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

fn request() -> ControlRequest {
    ControlRequest { id: json!(0), method: "ui.inspect".into(), params: json!({}), reply: mpsc::channel().0, cancelled: Default::default() }
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

#[test]
fn connection_limit_rejects_excess_clients_and_reuses_finished_slots() {
    let pending = queue();
    let ctx = context();
    let mut workers = Vec::new();
    let mut clients = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        let (client, server) = pair();
        spawn_connection(server, pending.clone(), ctx.clone(), &mut workers, Duration::from_secs(5));
        clients.push(client);
    }
    assert_eq!(workers.len(), MAX_CONNECTIONS);
    let (mut extra, server) = pair();
    spawn_connection(server, pending.clone(), ctx.clone(), &mut workers, Duration::from_secs(5));
    assert_eq!(extra.read(&mut [0]).unwrap(), 0);
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
    spawn_connection(server, pending, ctx, &mut workers, Duration::from_secs(5));
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
        r.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
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
