//! Invite links and server addresses: every form reads back, bad links are refused without
//! echoing them, and the parsers survive anything.

use proptest::prelude::*;

use crate::invite::{Endpoint, Invite};

const TOKEN: &str = "AbCdEfGhIjKlMnOpQrStUv";

fn invite() -> Invite {
    Invite::new("192.168.1.20:47900", "0123456789abcdef", TOKEN)
}

#[test]
fn both_forms_read_back() {
    let i = invite();
    assert_eq!(i.link(), format!("figcraft://join?host=192.168.1.20:47900&file=0123456789abcdef&token={TOKEN}"));
    assert_eq!(Invite::parse(&i.link()), Ok(i.clone()));
    assert_eq!(Invite::parse(&i.url()), Ok(i.clone()));
    // Pasted with spaces, quotes or angle brackets, or with escapes.
    assert_eq!(Invite::parse(&format!("  <{}>\n", i.link())), Ok(i.clone()));
    assert_eq!(Invite::parse(&format!("\"{}\"", i.url())), Ok(i.clone()));
    assert_eq!(Invite::parse(&format!("figcraft://join?host=192.168.1.20%3A47900&file=0123456789abcdef&token={TOKEN}")), Ok(i.clone()));
    // The secret never travels in the WebSocket request.
    assert_eq!(i.endpoint(), "ws://192.168.1.20:47900/");
    assert_eq!(i.server.display(), "192.168.1.20:47900");
}

#[test]
fn hosts_can_be_v6_or_names() {
    let v6 = Invite::new("[::1]:47900", "0123456789abcdef", TOKEN);
    assert_eq!(Invite::parse(&v6.link()), Ok(v6.clone()));
    assert_eq!(Invite::parse(&v6.url()), Ok(v6.clone()));
    assert!(v6.loopback());
    let name = Invite::new("studio.local:47900", "0123456789abcdef", TOKEN);
    assert_eq!(Invite::parse(&name.link()), Ok(name));
}

#[test]
fn server_links_carry_tls_and_a_path() {
    // A server behind a TLS reverse proxy, under a path: the server= form.
    let i = Invite { server: Endpoint { host: "figcraft.example.com:443".into(), path: "/live/".into(), secure: true }, ..invite() };
    assert_eq!(i.link(), format!("figcraft://join?server=wss://figcraft.example.com:443/live/&file=0123456789abcdef&token={TOKEN}"));
    assert_eq!(i.url(), format!("wss://figcraft.example.com:443/live/?file=0123456789abcdef&token={TOKEN}"));
    assert_eq!(i.endpoint(), "wss://figcraft.example.com:443/live/");
    assert_eq!(i.server.display(), "wss://figcraft.example.com:443/live/");
    assert_eq!(Invite::parse(&i.link()), Ok(i.clone()));
    assert_eq!(Invite::parse(&i.url()), Ok(i.clone()));
    // The scheme's port when there's none, and a path without its last slash.
    assert_eq!(Invite::parse(&format!("wss://figcraft.example.com/live?file=0123456789abcdef&token={TOKEN}")), Ok(i.clone()));
    assert_eq!(
        Invite::parse(&format!("figcraft://join?server=wss%3A%2F%2Ffigcraft.example.com%2Flive%2F&file=0123456789abcdef&token={TOKEN}")),
        Ok(i)
    );
    // A plain server at / is the same as an app's invite.
    let plain = Invite::parse(&format!("figcraft://join?server=ws://10.0.0.5:47900&file=0123456789abcdef&token={TOKEN}")).unwrap();
    assert_eq!(plain, Invite::new("10.0.0.5:47900", "0123456789abcdef", TOKEN));
    assert!(plain.link().starts_with("figcraft://join?host=10.0.0.5:47900&"));
}

#[test]
fn server_addresses_with_a_key() {
    let (e, q) = Endpoint::parse("ws://127.0.0.1:47900/?key=KeyKeyKeyKeyKeyKeyKey0").unwrap();
    assert_eq!(e, Endpoint::plain("127.0.0.1:47900"));
    assert_eq!(q, vec![("key".to_string(), "KeyKeyKeyKeyKeyKeyKey0".to_string())]);
    let (e, q) = Endpoint::parse(" wss://example.org ").unwrap();
    assert_eq!(e, Endpoint { host: "example.org:443".into(), path: "/".into(), secure: true });
    assert!(q.is_empty());
    let secret = "SuperSecretToken123456";
    for bad in [
        format!("http://example.org/?key={secret}"),
        format!("ws://exa mple.org/?key={secret}"),
        format!("ws://example.org:0/?key={secret}"),
        format!("ws://example.org/a/?key={secret}%zz"),
        format!("ws://example.org/<x>/?key={secret}"),
        format!("example.org?key={secret}"),
    ] {
        let e = Endpoint::parse(&bad).expect_err(&bad);
        assert!(!e.contains(secret), "{e}");
    }
}

#[test]
fn bad_links_are_refused_without_echoing_them() {
    let secret = "SuperSecretToken123456";
    for bad in [
        String::new(),
        "hello".into(),
        format!("https://example.com/?token={secret}"),
        format!("figcraft://node/12?token={secret}"),
        format!("figcraft://join?file=abc&token={secret}"),
        format!("figcraft://join?host=10.0.0.1&file=abc&token={secret}"),
        format!("figcraft://join?host=10.0.0.1:0&file=abc&token={secret}"),
        format!("figcraft://join?host=10.0.0.1:47900&token={secret}"),
        "figcraft://join?host=10.0.0.1:47900&file=abc&token=short".into(),
        format!("figcraft://join?host=10.0.0.1:47900&file=a/b&token={secret}"),
        format!("figcraft://join?host=10.0.0.1:47900&file=abc&token={secret}%zz"),
        format!("figcraft://join?server=http://10.0.0.1:47900/&file=abc&token={secret}"),
        format!("wss://10.0.0.1:47900/?file=abc&token=short{secret}!"),
        format!("figcraft://join?host=10.0.0.1:47900&file=abc&token={}", "x".repeat(3000)),
    ] {
        let e = Invite::parse(&bad).expect_err(&bad);
        assert!(!e.contains(secret), "{e}");
    }
}

proptest! {
    #[test]
    fn the_parsers_survive_anything(s in ".{0,300}") {
        let _ = Invite::parse(&s);
        let _ = Invite::parse(&format!("figcraft://join?{s}"));
        let _ = Invite::parse(&format!("figcraft://join?server={s}"));
        let _ = Invite::parse(&format!("ws://{s}"));
        let _ = Invite::parse(&format!("wss://{s}"));
        let _ = Endpoint::parse(&s);
        let _ = Endpoint::parse(&format!("ws://{s}"));
    }

    #[test]
    fn valid_invites_round_trip(a in 1u8..255, port in 1u16.., file in "[a-f0-9]{1,64}", token in "[A-Za-z0-9_-]{16,128}", path in "(/[a-z0-9]{1,8}){0,3}", secure: bool) {
        let i = Invite { server: Endpoint { host: format!("10.0.0.{a}:{port}"), path: format!("{path}/"), secure }, file, token };
        prop_assert_eq!(Invite::parse(&i.link()), Ok(i.clone()));
        prop_assert_eq!(Invite::parse(&i.url()), Ok(i));
    }
}

#[test]
fn plus_or_sign_inside_a_percent_escape_is_broken() {
    assert!(Endpoint::parse("ws://example.org/a/?key=%41").is_ok());
    for bad in ["%+1", "%-1", "%1", "%+a"] {
        assert!(Endpoint::parse(&format!("ws://example.org/a/?key={bad}")).is_err(), "{bad}");
    }
}

#[test]
fn port_must_be_all_digits() {
    assert!(Endpoint::parse("ws://localhost:80/").is_ok());
    for bad in ["ws://localhost:+80/", "ws://localhost:-80/", "ws://localhost:/"] {
        assert!(Endpoint::parse(bad).is_err(), "accepted {bad}");
    }
}
