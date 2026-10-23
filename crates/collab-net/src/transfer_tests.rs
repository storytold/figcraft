//! Real sockets: the same document is too large for a legacy message budget, but a negotiated
//! connection receives bounded pieces and exposes exactly one complete document to the engine.

use std::net::SocketAddr;
use std::time::Duration;

use figcraft_collab::message::{self, ServerMsg};
use figcraft_collab::{Client, Limits};
use figcraft_doc::Document;
use figcraft_engine::live::Event;
use tungstenite::Message;

use crate::guest::{Guest, GuestConfig};
use crate::host::{Host, HostConfig};
use crate::invite::Invite;
use crate::loopback_tests::Probe;
use crate::net::{NetLimits, send_binary};
use crate::transfer::{CHUNK, Receiver};

const TOKEN: &str = "SyntheticTransferSecret";

#[test]
fn framing_fits_a_tight_websocket_budget_without_relaxing_the_logical_limit() {
    use std::net::{TcpListener, TcpStream};
    use tungstenite::protocol::{Role, WebSocket};

    let payload = vec![7; CHUNK + 1];
    let packets: Vec<_> = crate::transfer::packets(&payload, true).collect();
    assert_eq!(packets.len(), 2);
    assert!(packets[0].len() > payload.len());
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let sender = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut ws = WebSocket::from_raw_socket(stream, Role::Server, None);
        for packet in packets {
            ws.send(Message::Binary(packet.into())).unwrap();
        }
    });
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut ws = WebSocket::from_raw_socket(stream, Role::Client, Some(crate::net::config(payload.len())));
    let mut receiver = Receiver::default();
    for i in 0..2 {
        let Message::Binary(packet) = ws.read().unwrap() else { panic!("expected a chunk") };
        if i == 0 {
            assert_eq!(Receiver::default().push(&packet, true, CHUNK), Err(crate::transfer::Error::TooLarge));
        }
        let complete = receiver.push(&packet, true, payload.len()).unwrap();
        if i == 0 {
            assert!(complete.is_none());
        } else {
            assert_eq!(complete.unwrap(), payload);
        }
    }
    sender.join().unwrap();
}

fn host() -> std::io::Result<(Host, Invite, String)> {
    host_with_name("Synthetic document / ".repeat(80_000))
}

fn host_with_name(name: String) -> std::io::Result<(Host, Invite, String)> {
    let probe = Probe::new();
    let host = Host::start(
        HostConfig {
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            any_port_if_taken: false,
            file: "large-test".into(),
            token: TOKEN.into(),
            doc: Document::new(&name),
            name: "Host".into(),
            limits: NetLimits::default(),
            core: Limits::default(),
        },
        probe.inbox,
    )?;
    let invite = Invite::new(host.addr().to_string(), "large-test", TOKEN);
    Ok((host, invite, name))
}

#[test]
#[ignore = "explicit large-memory transport regression; run separately"]
fn sixty_five_mib_document_crosses_real_socket() {
    // Synthetic content only. This exercises transport above 64 MiB, not the separate
    // persistent server's file quota, which an operator must explicitly configure.
    let (_host, invite, name) = host_with_name("S".repeat(65 * CHUNK)).unwrap();
    let limits = NetLimits::default();
    let (mut ws, _, _) = crate::net::connect_websocket(&invite.server, Duration::from_secs(30), &limits).unwrap();
    ws.set_config(|c| *c = crate::net::config(CHUNK + 49));
    send_binary(&mut ws, &message::encode(&Client::hello("large-test", TOKEN, "Large test", "")).unwrap()).unwrap();
    let mut rx = Receiver::default();
    let mut pieces = 0;
    loop {
        let Message::Binary(bytes) = ws.read().unwrap() else { continue };
        pieces += 1;
        let Some(bytes) = rx.push(&bytes, true, limits.max_server_message).unwrap() else { continue };
        assert!(bytes.len() > 64 * CHUNK);
        let welcome = message::decode::<ServerMsg>(&bytes, limits.max_server_message).unwrap();
        assert_eq!(Client::join(welcome).unwrap().document().name, name);
        break;
    }
    assert!(pieces > 64);
    let _ = ws.close(None);
}

#[test]
fn large_welcome_crosses_real_socket_in_bounded_pieces() {
    let (_host, invite, name) = host().unwrap();
    let limits = NetLimits::default();
    let (mut ws, _, _) = crate::net::connect_websocket(&invite.server, Duration::from_secs(5), &limits).unwrap();
    assert!(ws.get_ref().chunked);
    ws.set_config(|c| *c = crate::net::config(CHUNK + 49));
    let hello = message::encode(&Client::hello("large-test", TOKEN, "Test", "")).unwrap();
    send_binary(&mut ws, &hello).unwrap();
    let mut rx = Receiver::default();
    let mut pieces = 0;
    loop {
        let Message::Binary(bytes) = ws.read().unwrap() else { continue };
        assert!(bytes.len() <= CHUNK + 49);
        pieces += 1;
        if let Some(bytes) = rx.push(&bytes, true, limits.max_server_message).unwrap() {
            let welcome = message::decode::<ServerMsg>(&bytes, limits.max_server_message).unwrap();
            assert_eq!(Client::join(welcome).unwrap().document().name, name);
            break;
        }
    }
    assert!(pieces > 1);
    let _ = ws.close(None);
}

#[test]
fn reproduces_the_legacy_whole_message_limit_on_the_same_document() {
    let (_host, invite, _) = host().unwrap();
    let stream = std::net::TcpStream::connect(&invite.server.host).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let (mut ws, _) = tungstenite::client::client_with_config(invite.endpoint(), stream, Some(crate::net::config(CHUNK + 49))).unwrap();
    ws.send(Message::Binary(message::encode(&Client::hello("large-test", TOKEN, "Legacy", "")).unwrap().into())).unwrap();
    assert!(matches!(ws.read(), Err(tungstenite::Error::Capacity(_))), "the legacy welcome cannot fit the very same per-message budget");
}

#[test]
fn large_edit_uploads_and_is_confirmed_as_one_operation() {
    let (_host, invite, _) = host().unwrap();
    let limits = NetLimits::default();
    let (mut ws, _, _) = crate::net::connect_websocket(&invite.server, Duration::from_secs(5), &limits).unwrap();
    send_binary(&mut ws, &message::encode(&Client::hello("large-test", TOKEN, "Edit", "")).unwrap()).unwrap();
    let mut rx = Receiver::default();
    let mut client = loop {
        let Message::Binary(b) = ws.read().unwrap() else { continue };
        let Some(b) = rx.push(&b, true, limits.max_server_message).unwrap() else { continue };
        let m = message::decode::<ServerMsg>(&b, limits.max_server_message).unwrap();
        if matches!(m, ServerMsg::Welcome { .. }) {
            break Client::join(m).unwrap();
        }
    };
    let mut doc = client.document().clone();
    doc.name = "Changed document / ".repeat(90_000);
    let edit = client.edit(&doc).unwrap().unwrap();
    let bytes = message::encode(&edit).unwrap();
    assert!(bytes.len() > CHUNK);
    send_binary(&mut ws, &bytes).unwrap();
    loop {
        let Message::Binary(b) = ws.read().unwrap() else { continue };
        let Some(b) = rx.push(&b, true, limits.max_server_message).unwrap() else { continue };
        if let ServerMsg::Changes(accepted) = message::decode::<ServerMsg>(&b, limits.max_server_message).unwrap() {
            assert_eq!(accepted.seq, 1, "the entire large edit is one accepted batch");
            break;
        }
    }
    let _ = ws.close(None);
}

#[test]
fn native_guest_receives_the_complete_large_document() {
    let (_host, invite, name) = host().unwrap();
    let mut probe = Probe::new();
    let _guest = Guest::start(GuestConfig::new(invite, "Test".into()), probe.inbox.clone()).unwrap();
    probe.wait("complete chunked welcome", |events| events.iter().any(|e| matches!(e, Event::Sync(ServerMsg::Welcome { .. }))));
    let welcome = probe
        .events
        .into_iter()
        .find_map(|e| match e {
            Event::Sync(m @ ServerMsg::Welcome { .. }) => Some(m),
            _ => None,
        })
        .unwrap();
    assert_eq!(Client::join(welcome).unwrap().document().name, name);
}

#[test]
fn a_chunked_welcome_above_the_budget_is_a_local_size_failure_not_a_host_refusal() {
    let (_host, invite, _) = host().unwrap();
    let mut cfg = GuestConfig::new(invite, "Budget test".into());
    cfg.limits.max_server_message = CHUNK + 128;
    let expected = figcraft_engine::live::TOO_LARGE_TO_JOIN.replace("{mb}", "1");
    let mut probe = Probe::new();
    let _guest = Guest::start(cfg, probe.inbox.clone()).unwrap();
    probe.wait("local receive-budget failure", |events| {
        events.iter().any(|e| matches!(e, Event::Status(figcraft_engine::live::Status::Failed { reason }) if reason == &expected))
    });
    assert!(!probe.events.iter().any(|e| matches!(e, Event::Sync(ServerMsg::Welcome { .. }))));
}

#[test]
fn legacy_client_is_not_sent_chunk_envelopes() {
    let (_host, invite, _) = host().unwrap();
    let stream = std::net::TcpStream::connect(&invite.server.host).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let (mut ws, _) = tungstenite::client::client(invite.endpoint(), stream).unwrap();
    let hello = message::encode(&Client::hello("large-test", TOKEN, "Legacy", "")).unwrap();
    ws.send(Message::Binary(hello.into())).unwrap();
    let Message::Binary(bytes) = ws.read().unwrap() else { panic!("expected welcome") };
    assert_eq!(bytes.first(), Some(&message::WIRE_VERSION));
    assert!(bytes.len() > CHUNK);
    assert!(matches!(message::decode::<ServerMsg>(&bytes, 8 * CHUNK).unwrap(), ServerMsg::Welcome { .. }));
    let _ = ws.close(None);
}

#[test]
fn new_client_uses_legacy_on_the_same_connection_when_the_extension_is_not_selected() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        // The old server ignores the optional header; no second handshake is necessary.
        for _ in 0..1 {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let _ = ws.read();
        }
    });
    let endpoint = Invite::new(address.to_string(), "test", TOKEN).server;
    let (ws, _, _) = crate::net::connect_websocket(&endpoint, Duration::from_secs(5), &NetLimits::default()).unwrap();
    assert!(!ws.get_ref().chunked);
    drop(ws);
    server.join().unwrap();
}
