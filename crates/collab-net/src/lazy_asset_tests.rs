//! Cold-cache, real-socket regressions. No private artwork or external server is involved.
use crate::guest::{Guest, GuestConfig};
use crate::host::{Host, HostConfig};
use crate::invite::Invite;
use crate::loopback_tests::Probe;
use crate::net::{Duplex, NetLimits, send_binary};
use figcraft_assets::{AssetRef, cache};
use figcraft_collab::message::{self, CatchUp, ServerMsg};
use figcraft_collab::{Client, Limits};
use figcraft_doc::{
    Document, ImageData, NodeKind,
    color::{ImageScaleMode, Paint},
};
use figcraft_engine::live::Event;
use std::sync::Arc;
use std::time::Duration;

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

fn picture(red: u8) -> TestResult<Vec<u8>> {
    let mut pix = figcraft_render::Pixmap::new(32, 32).ok_or("no pixmap")?;
    for pixel in pix.data_mut().chunks_exact_mut(4) {
        pixel.copy_from_slice(&[red, 17, 211, 255]);
    }
    Ok(figcraft_render::encode_png(&pix)?)
}

fn document() -> TestResult<Document> {
    let mut doc = Document::new("Synthetic lazy images");
    let page = doc.first_page().ok_or("no page")?;
    for (key, red, x) in [("left", 231, 0.0), ("right", 19, 10_000.0)] {
        let bytes = Arc::new(picture(red)?);
        doc.images.insert(key.into(), ImageData { asset: None, bytes, width: 32, height: 32 });
        let node = doc.create(page, NodeKind::Rectangle, key, x, 0.0, 32.0, 32.0)?;
        doc.get_mut(node)?.fills = vec![Paint::image(key, ImageScaleMode::Fill)];
    }
    Ok(doc)
}

fn host(doc: Document) -> std::io::Result<(Host, Invite)> {
    let file = format!("synthetic-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(std::io::Error::other)?.as_nanos());
    let token = "SyntheticAssetSessionSecret";
    let h = Host::start(
        HostConfig {
            addr: "127.0.0.1:0".parse().map_err(std::io::Error::other)?,
            any_port_if_taken: false,
            file: file.clone(),
            token: token.into(),
            doc,
            name: "Host".into(),
            limits: NetLimits::default(),
            core: Limits::default(),
        },
        Probe::new().inbox,
    )?;
    let invite = Invite::new(h.addr().to_string(), file, token);
    Ok((h, invite))
}

fn joined(invite: Invite) -> TestResult<(Guest, Probe, Client)> {
    let mut probe = Probe::new();
    let guest = Guest::start(GuestConfig::new(invite, "Guest".into()), probe.inbox.clone())?;
    probe.wait("manifest welcome", |events| events.iter().any(|e| matches!(e, Event::Sync(ServerMsg::Welcome { .. }))));
    let at = probe.events.iter().position(|event| matches!(event, Event::Sync(ServerMsg::Welcome { .. }))).ok_or("no welcome")?;
    let Event::Sync(welcome) = probe.events.remove(at) else {
        return Err("no welcome".into());
    };
    Ok((guest, probe, Client::join(welcome)?))
}

#[test]
fn viewport_fetches_only_visible_images_and_save_is_self_contained() {
    let source = document().unwrap();
    let (_host, invite) = host(source.clone()).unwrap();
    let (_guest, mut probe, client) = joined(invite).unwrap();
    let doc = client.document();
    assert!(doc.images.values().all(|image| image.bytes.is_empty() && image.asset.is_some()));
    let left = doc.images["left"].asset.as_ref().unwrap();
    let right = doc.images["right"].asset.as_ref().unwrap();
    assert!(cache::load(left).is_err());
    assert!(cache::load(right).is_err());
    let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 64, height: 64 };
    let rendered = Arc::new(doc.clone());
    let mut retained = figcraft_render::Retained::default();
    retained.render(&rendered, doc.first_page().unwrap(), &view, &Default::default()).unwrap();
    probe.wait("visible image", |events| events.iter().any(|e| matches!(e, Event::AssetsChanged)));
    assert_eq!(*cache::load(left).unwrap(), *source.images["left"].bytes);
    assert!(cache::load(right).is_err(), "off-screen data must not be requested");
    let (_, pix, work) = retained.render(&rendered, doc.first_page().unwrap(), &view, &Default::default()).unwrap();
    assert_ne!(work, figcraft_render::Work::Reused, "downloads must invalidate retained pixels without document edits");
    assert_eq!(pix.pixel(8, 8).unwrap().red(), 231);
    let saved = figcraft_format::save_bytes(doc, None).unwrap();
    let (reopened, issues) = figcraft_format::load_bytes(&saved).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(*reopened.images["left"].bytes, *source.images["left"].bytes);
    assert_eq!(*reopened.images["right"].bytes, *source.images["right"].bytes);
    assert!(reopened.images.values().all(|image| image.asset.is_none()));
    assert!(doc.images.values().all(|image| image.bytes.is_empty()), "downloads live outside undo/document clones");
}

#[test]
fn crash_recovery_keeps_references_without_downloading_and_untrusted_files_cannot_use_them() {
    let (_host, invite) = host(document().unwrap()).unwrap();
    let (_guest, _probe, client) = joined(invite).unwrap();
    let bytes = figcraft_format::save_recovery_bytes_with_history(client.document(), &[], &[]).unwrap();
    assert!(bytes.len() < 10_000);
    assert!(client.document().images.values().all(|image| cache::load(image.asset.as_ref().unwrap()).is_err()));
    assert!(figcraft_format::load_bytes(&bytes).is_err(), "ordinary opened files cannot grant access to a cache scope");
    let recovered = figcraft_format::load_recovery_bytes_with_versions(&bytes).unwrap();
    assert!(recovered.issues.is_empty(), "{:?}", recovered.issues);
    assert_eq!(recovered.doc, *client.document());
    assert!(recovered.doc.images.values().all(|image| image.asset.is_some() && image.bytes.is_empty()));
}

#[test]
fn an_old_client_still_gets_base64_entries_not_new_image_variants() {
    let source = document().unwrap();
    let (_host, invite) = host(source.clone()).unwrap();
    let stream = std::net::TcpStream::connect(&invite.server.host).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let (mut ws, _) = tungstenite::client::client(invite.endpoint(), stream).unwrap();
    let hello = Client::hello(&invite.file, &invite.token, "Old client", "");
    ws.send(tungstenite::Message::Binary(message::encode(&hello).unwrap().into())).unwrap();
    let tungstenite::Message::Binary(bytes) = ws.read().unwrap() else { panic!("no welcome") };
    let welcome: ServerMsg = message::decode(&bytes, 1 << 20).unwrap();
    let ServerMsg::Welcome { catch_up: CatchUp::Snapshot(changes), .. } = &welcome else { panic!("no snapshot") };
    assert!(!changes.iter().any(|change| matches!(change, figcraft_collab::Change::Image(_))));
    assert_eq!(Client::join(welcome).unwrap().document().images, source.images);
    let _ = ws.close(None);
}

fn receive(ws: &mut tungstenite::WebSocket<Duplex>) -> TestResult<ServerMsg> {
    let mut transfer = crate::transfer::Receiver::default();
    loop {
        let tungstenite::Message::Binary(bytes) = ws.read()? else { continue };
        if let Some(bytes) = transfer.push(&bytes, true, 1 << 20)? {
            return Ok(message::decode(&bytes, 1 << 20)?);
        }
    }
}

#[test]
fn joining_one_file_does_not_grant_access_to_other_cached_assets() {
    let (_host, invite) = host(document().unwrap()).unwrap();
    let secret = b"other file synthetic bytes";
    let foreign = AssetRef::of(secret);
    cache::store(&foreign, secret).unwrap();
    let (mut ws, _, _) = crate::net::connect_websocket(&invite.server, Duration::from_secs(5), &NetLimits::default()).unwrap();
    let hello = Client::hello(&invite.file, &invite.token, "Test", "");
    send_binary(&mut ws, &crate::assets_wire::client(&hello, true).unwrap()).unwrap();
    assert!(matches!(receive(&mut ws).unwrap(), ServerMsg::Welcome { .. }));
    send_binary(&mut ws, &crate::assets_wire::client(&message::ClientMsg::FetchAsset(foreign), true).unwrap()).unwrap();
    loop {
        if let ServerMsg::Asset { bytes, error, .. } = receive(&mut ws).unwrap() {
            assert!(bytes.is_none());
            assert!(error.is_some());
            break;
        }
    }
    let _ = ws.close(None);
}

#[test]
fn a_host_without_asset_negotiation_cannot_introduce_foreign_cache_references() {
    for legacy_json in [false, true] {
        let mut foreign = AssetRef::of(&picture(73).unwrap());
        foreign.scope = AssetRef::of(b"synthetic foreign session").sha256;
        cache::store(&foreign, &picture(73).unwrap()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let invite = Invite::new(listener.local_addr().unwrap().to_string(), "unnegotiated-assets", "SyntheticAssetSessionSecret");
        let server = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            // Deliberately do not echo the resource extension.
            let mut ws = tungstenite::accept(socket).unwrap();
            let tungstenite::Message::Binary(bytes) = ws.read().unwrap() else { panic!("no hello") };
            let message::ClientMsg::Hello(hello) = message::decode(&bytes, 1 << 20).unwrap() else { panic!("no hello") };
            let mut doc = document().unwrap();
            for image in doc.images.values_mut() {
                image.asset = Some(foreign.clone());
                image.bytes = Arc::new(Vec::new());
            }
            let mut core = figcraft_collab::Server::new(doc, "SyntheticAssetSessionSecret", Limits::default());
            let (_, outs) = core.join_as(hello, message::Access::Edit).unwrap();
            for out in outs {
                if let figcraft_collab::Outgoing::To(_, mut welcome @ ServerMsg::Welcome { .. }) = out {
                    if legacy_json && let ServerMsg::Welcome { catch_up: CatchUp::Snapshot(changes), .. } = &mut welcome {
                        for change in changes {
                            if let figcraft_collab::Change::Image(image) = change {
                                *change = figcraft_collab::Change::Put {
                                    entry: figcraft_collab::Entry::Image(image.key.clone()),
                                    value: figcraft_collab::change::Json(serde_json::to_value(image).unwrap()),
                                };
                            }
                        }
                    }
                    assert!(crate::assets_wire::manifests(&welcome).unwrap().iter().any(|a| a.scope == foreign.scope));
                    assert!(crate::assets_wire::validate_legacy(&welcome).is_err());
                    ws.send(tungstenite::Message::Binary(message::encode(&welcome).unwrap().into())).unwrap();
                }
            }
            let _ = ws.read();
        });
        let mut probe = Probe::new();
        let _guest = Guest::start(GuestConfig::new(invite, "Guest".into()), probe.inbox.clone()).unwrap();
        probe.wait("refusal before importing cache capabilities", |events| {
            events
                .iter()
                .any(|e| matches!(e, Event::Status(figcraft_engine::live::Status::Failed { reason }) if reason.contains("without negotiating")))
        });
        assert!(!probe.events.iter().any(|e| matches!(e, Event::Sync(ServerMsg::Welcome { .. }) | Event::Welcome { .. })));
        server.join().unwrap();
    }
}

#[test]
#[allow(clippy::result_large_err)] // tungstenite's server handshake callback has this fixed error type.
fn corrupt_and_unsolicited_asset_responses_are_refused() {
    use figcraft_engine::live::Status;
    use tungstenite::handshake::server::{Request, Response};
    for requested in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let invite = Invite::new(listener.local_addr().unwrap().to_string(), format!("bad-assets-{requested}"), "SyntheticAssetSessionSecret");
        let server = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept_hdr(socket, |_: &Request, mut response: Response| {
                response.headers_mut().insert("X-FigCraft-Transfer", crate::transfer::PROTOCOL.parse().unwrap());
                response.headers_mut().insert("X-FigCraft-Assets", crate::assets_wire::PROTOCOL.parse().unwrap());
                Ok(response)
            })
            .unwrap();
            let tungstenite::Message::Binary(bytes) = ws.read().unwrap() else { panic!("no hello") };
            let message::ClientMsg::Hello(hello) = message::decode(&bytes, 1 << 20).unwrap() else { panic!("no hello") };
            let doc = document().unwrap();
            let asset = doc.images["left"].reference();
            let mut core = figcraft_collab::Server::new(doc, "SyntheticAssetSessionSecret", Limits::default());
            let (_, outs) = core.join_as(hello, message::Access::Edit).unwrap();
            for out in outs {
                if let figcraft_collab::Outgoing::To(_, welcome @ ServerMsg::Welcome { .. }) = out {
                    ws.send(tungstenite::Message::Binary(crate::assets_wire::server(&welcome, true).unwrap().into())).unwrap();
                }
            }
            if requested {
                loop {
                    let tungstenite::Message::Binary(bytes) = ws.read().unwrap() else { continue };
                    let msg: message::ClientMsg = message::decode(&bytes, 1 << 20).unwrap();
                    if matches!(msg, message::ClientMsg::FetchAsset(_)) {
                        break;
                    }
                }
            }
            let bad = ServerMsg::Asset { asset, bytes: Some(figcraft_assets::Blob(Arc::new(b"wrong checksum".to_vec()))), error: None };
            ws.send(tungstenite::Message::Binary(message::encode(&bad).unwrap().into())).unwrap();
            let _ = ws.read();
        });
        let (_guest, mut probe, client) = joined(invite).unwrap();
        let asset = client.document().images["left"].asset.as_ref().unwrap();
        if requested {
            assert!(cache::request(asset));
        }
        probe.wait("bad asset refused", |events| events.iter().any(|e| matches!(e, Event::Status(Status::Failed { .. }))));
        assert!(cache::load(asset).is_err());
        assert!(!probe.events.iter().any(|e| matches!(e, Event::AssetsChanged)));
        server.join().unwrap();
    }
}
