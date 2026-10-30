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
    loop {
        let tungstenite::Message::Binary(bytes) = ws.read()? else { continue };
        return Ok(message::decode(&bytes, 1 << 20)?);
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
    send_binary(&mut ws, &crate::assets_wire::client(&message::ClientMsg::FetchAsset(foreign.clone()), true).unwrap()).unwrap();
    loop {
        if let ServerMsg::Asset { bytes, error, .. } = receive(&mut ws).unwrap() {
            assert!(bytes.is_none());
            assert!(error.is_some());
            break;
        }
    }
    let request = figcraft_assets::preview::Request { asset: foreign, side: 128 };
    send_binary(&mut ws, &crate::assets_wire::client(&message::ClientMsg::FetchPreview(request), true).unwrap()).unwrap();
    assert!(matches!(receive(&mut ws).unwrap(), ServerMsg::Refused { .. }), "previews must enforce the original file authorization too");
    let _ = ws.close(None);
}

#[test]
fn small_view_downloads_a_preview_and_zoom_and_save_keep_the_original() {
    use figcraft_assets::preview::{self, Request};
    let mut source = Document::new("Synthetic preview image");
    let mut pix = figcraft_render::Pixmap::new(1600, 2400).unwrap();
    for (i, p) in pix.data_mut().chunks_exact_mut(4).enumerate() {
        let x = (i % 1600) as u32;
        let y = (i / 1600) as u32;
        let n = x.wrapping_mul(1664525).wrapping_add(y.wrapping_mul(1013904223));
        let n = (n ^ (n >> 16)).wrapping_mul(2246822519);
        p.copy_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8, 255]);
    }
    let bytes = Arc::new(figcraft_render::encode_png(&pix).unwrap());
    source.images.insert("portrait".into(), ImageData { asset: None, bytes: bytes.clone(), width: 1600, height: 2400 });
    let id = source.create(source.first_page().unwrap(), NodeKind::Rectangle, "Portrait", 0.0, 0.0, 100.0, 150.0).unwrap();
    source.get_mut(id).unwrap().fills = vec![Paint::image("portrait", ImageScaleMode::Fill)];
    let (_host, invite) = host(source).unwrap();
    let (_guest, mut probe, client) = joined(invite).unwrap();
    let doc = client.document();
    let asset = doc.images["portrait"].asset.as_ref().unwrap();
    let request = Request { asset: asset.clone(), side: 256 };
    assert!(cache::load(asset).is_err());
    let rendered = Arc::new(doc.clone());
    let mut retained = figcraft_render::Retained::default();
    let view = figcraft_render::View { origin: (0.0, 0.0), scale: 1.0, width: 128, height: 180 };
    let opts = figcraft_render::Options { lod: true, magnified_image_pixels: true, ..Default::default() };
    retained.render(&rendered, doc.first_page().unwrap(), &view, &opts).unwrap();
    probe.wait("small image preview from real canvas rendering", |_| preview::load(&request).is_some());
    let (_, canvas, _) = retained.render(&rendered, doc.first_page().unwrap(), &view, &opts).unwrap();
    assert!(canvas.pixel(20, 20).unwrap().alpha() > 0);

    let small = preview::load(&request).unwrap();
    assert!(small.len() < bytes.len() / 4, "preview {} vs original {}", small.len(), bytes.len());
    assert_eq!(figcraft_render::images::dimensions(&small), Some((170, 256)));
    assert!(cache::load(asset).is_err(), "general view must not silently download the full image");
    let overview =
        figcraft_render::images::get_sized("portrait", Some(&doc.images["portrait"]), &Default::default(), Default::default(), 256).unwrap();
    assert_eq!(overview.height(), 256);
    let closer = figcraft_render::images::get_sized("portrait", Some(&doc.images["portrait"]), &Default::default(), Default::default(), 512).unwrap();
    assert_eq!(closer.height(), 256, "keep overview pixels until the sharper preview arrives");
    let sharper = Request { asset: asset.clone(), side: 512 };
    probe.wait("sharper preview", |_| preview::load(&sharper).is_some());
    let closer = figcraft_render::images::get_sized("portrait", Some(&doc.images["portrait"]), &Default::default(), Default::default(), 512).unwrap();
    assert_eq!(closer.height(), 512, "a new zoom level must not reuse the blurry cache entry");
    let original = cache::require(asset).unwrap();
    assert_eq!(*original, *bytes);
    assert_eq!(*figcraft_format::load_bytes(&figcraft_format::save_bytes(doc, None).unwrap()).unwrap().0.images["portrait"].bytes, *bytes);
}

#[test]
fn hundreds_of_warm_previews_do_not_trip_the_host_message_rate_limit() {
    use figcraft_assets::preview::{self, Request};
    let mut doc = Document::new("Synthetic warm preview gallery");
    for i in 0..501u32 {
        let mut p = figcraft_render::Pixmap::new(256, 256).unwrap();
        for pixel in p.data_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[(i >> 8) as u8, i as u8, 171, 255]);
        }
        let bytes = Arc::new(figcraft_render::encode_png(&p).unwrap());
        let request = Request { asset: AssetRef::of_arc(&bytes), side: 128 };
        let small = Arc::new(figcraft_render::images::make_preview(&bytes, 128).unwrap());
        preview::store(&request, small).unwrap();
        doc.images.insert(format!("warm-{i}"), ImageData { asset: None, bytes, width: 256, height: 256 });
    }
    let (_host, invite) = host(doc).unwrap();
    let (_guest, mut probe, client) = joined(invite).unwrap();
    let requests: Vec<_> = client.document().images.values().map(|i| Request { asset: i.asset.clone().unwrap(), side: 128 }).collect();
    probe.wait("warm gallery without rate refusal", |events| {
        assert!(
            !events.iter().any(|e| matches!(e, Event::Status(figcraft_engine::live::Status::Failed { .. }))),
            "a cached gallery must not disconnect"
        );
        for r in &requests {
            let _ = preview::resolve(r);
        }
        requests.iter().all(|r| preview::load(r).is_some())
    });
    assert!(requests.iter().all(|r| cache::load(&r.asset).is_err()), "warm previews must not fall back to downloading originals");
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

#[test]
fn resource_negotiation_uses_one_complete_message_above_a_mebibyte_without_chunk_headers() {
    let mut source = document().unwrap();
    source.name = "synthetic title ".repeat(100_000);
    let (_host, invite) = host(source.clone()).unwrap();
    let mut request = crate::net::request(&invite.endpoint()).unwrap();
    assert!(!request.headers().contains_key("X-FigCraft-Transfer"));
    assert!(!request.headers().contains_key("Sec-WebSocket-Protocol"));
    request.headers_mut().insert("X-FigCraft-Assets", crate::assets_wire::PROTOCOL.parse().unwrap());
    let socket = std::net::TcpStream::connect(&invite.server.host).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let (mut ws, response) = tungstenite::client::client(request, socket).unwrap();
    assert!(response.headers().contains_key("X-FigCraft-Assets"));
    assert!(!response.headers().contains_key("X-FigCraft-Transfer"));
    ws.send(tungstenite::Message::Binary(message::encode(&Client::hello(&invite.file, &invite.token, "Guest", "")).unwrap().into())).unwrap();
    let tungstenite::Message::Binary(bytes) = ws.read().unwrap() else { panic!("no binary welcome") };
    assert!(bytes.len() > 1 << 20);
    assert_eq!(bytes[0], 1);
    let welcome: ServerMsg = message::decode(&bytes, 4 << 20).unwrap();
    let doc = Client::join(welcome).unwrap().document().clone();
    assert_eq!(doc.name, source.name);
    assert!(doc.images.values().all(|i| i.bytes.is_empty() && i.asset.is_some()));
    let _ = ws.close(None);
}

#[test]
#[allow(clippy::result_large_err)]
fn review_uploading_a_cold_reference_fetches_its_original_only_once() {
    use tungstenite::handshake::server::{Request, Response};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let invite = Invite::new(listener.local_addr().unwrap().to_string(), crate::token::file_id().unwrap(), "SyntheticAssetSessionSecret");
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut ws = tungstenite::accept_hdr(socket, |_: &Request, mut response: Response| {
            response.headers_mut().insert("X-FigCraft-Assets", crate::assets_wire::PROTOCOL.parse().unwrap());
            Ok(response)
        })
        .unwrap();
        let tungstenite::Message::Binary(bytes) = ws.read().unwrap() else { panic!("no hello") };
        let message::ClientMsg::Hello(hello) = message::decode(&bytes, 1 << 20).unwrap() else { panic!("no hello") };
        let source = document().unwrap();
        let payload = source.images["left"].bytes.clone();
        let asset = source.images["left"].reference();
        let mut core = figcraft_collab::Server::new(source, "SyntheticAssetSessionSecret", Limits::default());
        let (_, outs) = core.join_as(hello, message::Access::Edit).unwrap();
        for out in outs {
            if let figcraft_collab::Outgoing::To(_, welcome @ ServerMsg::Welcome { .. }) = out {
                ws.send(tungstenite::Message::Binary(crate::assets_wire::server(&welcome, true).unwrap().into())).unwrap();
            }
        }
        let mut fetches = 0;
        let mut uploaded = false;
        loop {
            let wire = match ws.read() {
                Ok(wire) => wire,
                Err(tungstenite::Error::Io(e)) if uploaded && matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    break;
                }
                Err(e) => panic!("{e}"),
            };
            let tungstenite::Message::Binary(bytes) = wire else { continue };
            match message::decode::<message::ClientMsg>(&bytes, 1 << 20).unwrap() {
                message::ClientMsg::FetchAsset(_) => {
                    fetches += 1;
                    let reply = ServerMsg::Asset { asset: asset.clone(), bytes: Some(figcraft_assets::Blob(payload.clone())), error: None };
                    ws.send(tungstenite::Message::Binary(message::encode(&reply).unwrap().into())).unwrap();
                }
                message::ClientMsg::Changes { changes, .. } => {
                    assert!(changes.iter().any(|c| matches!(c, figcraft_collab::Change::Image(i) if i.bytes.is_some())));
                    uploaded = true;
                    ws.get_mut().set_read_timeout(Some(Duration::from_millis(400))).unwrap();
                }
                _ => {}
            }
        }
        assert!(uploaded);
        fetches
    });
    let (guest, _probe, client) = joined(invite).unwrap();
    let image = &client.document().images["left"];
    assert!(cache::load(image.asset.as_ref().unwrap()).is_err());
    guest.send(message::ClientMsg::Changes {
        base_seq: 0,
        batch_id: 1,
        changes: vec![figcraft_collab::Change::Image(figcraft_collab::change::ImageChange::of("left", image))],
    });
    assert_eq!(server.join().unwrap(), 1, "queued self-fetch must not download an original twice");
    drop(guest);
}
