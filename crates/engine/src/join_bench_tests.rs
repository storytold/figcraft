//! Join timings for a large shared file (#2041): the server's welcome, the sync client built from
//! it and the engine opening it as a tab, for the current catch-up and the older full snapshot.
//! Run in release:
//! `cargo test --release -p figcraft-engine --lib join_timings -- --ignored --nocapture`.
//!
//! `FIGCRAFT_JOIN_BENCH_LAYERS` sets the layer count (default 120000) and
//! `FIGCRAFT_JOIN_BENCH_CATCH_UP` which catch-ups to time (`delta,legacy` by default).

use std::time::Instant;

use figcraft_collab::message::{ClientMsg, ServerMsg};
use figcraft_collab::{Client, Limits, Outgoing, Server};
use figcraft_doc::color::{Color, Paint};
use figcraft_doc::{Document, NodeKind, TextData};
use serde_json::json;

use crate::live::{Event, Role};
use crate::live_tests::{info, push, session};

/// What a current client announces, minus the delta snapshot: a server sends it the full snapshot.
const LEGACY_CLIENT_VERSION: &str = "0.18.1 signals hosting comments packed design-review branches";

/// A document of about `layers` layers over 15 pages: frames of eight children (text, rectangles
/// and ellipses with fills), like a large product file.
fn big_doc(layers: usize) -> Document {
    let mut doc = Document::new("Big file");
    let mut pages = vec![doc.first_page().unwrap()];
    for p in 1..15 {
        pages.push(doc.add_page(&format!("Page {p}")));
    }
    let per_page = layers / pages.len();
    for (pi, page) in pages.into_iter().enumerate() {
        let mut made = 0;
        let mut f = 0;
        while made < per_page {
            let x = (f % 40) as f64 * 400.0;
            let y = (f / 40) as f64 * 300.0;
            let frame = doc.create(page, NodeKind::Frame, &format!("Card {pi}.{f}"), x, y, 360.0, 260.0).unwrap();
            for c in 0..8 {
                let kind = match c % 3 {
                    0 => NodeKind::Text(TextData::new(format!("Label {f} {c}")).into()),
                    1 => NodeKind::Rectangle,
                    _ => NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 },
                };
                let id = doc.create(frame, kind, &format!("Layer {c}"), 8.0 + c as f64 * 40.0, 8.0, 32.0, 32.0).unwrap();
                doc.node_mut(id).unwrap().fills = vec![Paint::solid(Color::new(0.2, 0.4, c as f32 / 8.0, 1.0))];
            }
            made += 9;
            f += 1;
        }
    }
    doc.repair();
    doc
}

/// The welcome `server` gives a newcomer announcing `client_version`, and its id.
fn welcome(server: &mut Server, client_version: &str) -> (u32, ServerMsg) {
    let ClientMsg::Hello(mut h) = Client::hello("f", "", "Ada", "") else { panic!("no hello") };
    h.client_version = client_version.to_string();
    let (id, outs) = server.join(h).unwrap();
    let w = outs.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
    (id, w)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "timing: run in release with --ignored --nocapture"]
fn join_timings() {
    let layers: usize = std::env::var("FIGCRAFT_JOIN_BENCH_LAYERS").ok().and_then(|v| v.parse().ok()).unwrap_or(120_000);
    let which = std::env::var("FIGCRAFT_JOIN_BENCH_CATCH_UP").unwrap_or_else(|_| "delta,legacy".into());
    let doc = big_doc(layers);
    println!("{} layers", doc.node_count());
    let mut server = Server::new(doc, "", Limits::default());
    for name in which.split(',').map(str::trim) {
        let version = if name == "legacy" { LEGACY_CLIENT_VERSION.to_string() } else { figcraft_collab::message::client_version() };
        let t = Instant::now();
        let (id, w) = welcome(&mut server, &version);
        let welcome_ms = ms(t);
        let bytes = figcraft_collab::message::encode(&w).unwrap().len();
        let t = Instant::now();
        let client = Client::join(w.clone()).unwrap();
        let client_ms = ms(t);
        drop(client);
        let (mut s, script) = session();
        s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
        push(&script, vec![Event::Welcome { info: info(Role::Guest, id), peers: vec![] }, Event::Sync(w)]);
        let t = Instant::now();
        s.live_poll();
        let engine_ms = ms(t);
        assert_eq!(s.doc().unwrap().name, "Big file");
        println!("{name}: welcome {bytes} bytes in {welcome_ms:.0} ms; Client::join {client_ms:.0} ms; engine join (live_poll) {engine_ms:.0} ms");
    }
}
