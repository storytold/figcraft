//! Join codes: what typing tolerates, both kinds round trip, and tampered local codes are caught.

use std::net::Ipv4Addr;

use super::*;

#[test]
fn typed_codes_are_normalised() {
    let table = [
        ("K7M-2QX-9TD", Ok("K7M2QX9TD")),
        ("  k7m 2qx 9td\n", Ok("K7M2QX9TD")),
        ("k7m2qx9td", Ok("K7M2QX9TD")),
        ("K7M–2QX—9TD", Ok("K7M2QX9TD")),
        ("K7M_2QX.9TD", Ok("K7M2QX9TD")),
        // Look-alikes are the digits they are mistaken for.
        ("O0o-IiL-l1X", Ok("00011111X")),
        ("", Err("type the join code")),
        ("  - ", Err("type the join code")),
        ("K7M-2QU-9TD", Err("no U")),
        ("K7M/2QX", Err("only letters and digits")),
        ("K7M2QX9T€", Err("only letters and digits")),
        ("figcraft://join?host=1.2.3.4:5&file=f&token=t", Err("only letters and digits")),
    ];
    for (typed, want) in table {
        match (normalize(typed), want) {
            (Ok(got), Ok(want)) => assert_eq!(got, want, "{typed:?}"),
            (Err(e), Err(part)) => assert!(e.contains(part), "{typed:?}: {e}"),
            (got, want) => panic!("{typed:?}: got {got:?}, wanted {want:?}"),
        }
    }
    assert!(normalize(&"A".repeat(500)).is_err(), "too long to be a code");
}

#[test]
fn errors_never_echo_what_was_typed() {
    for typed in ["s3cr3t!!", "SECRETSECRETSECRETSECRE", "figcraft://join?token=hunter2hunter2hunter2"] {
        let e = parse(typed).unwrap_err();
        assert!(!e.to_lowercase().contains(&typed.to_lowercase()[..6]), "{e}");
    }
}

#[test]
fn online_codes_are_nine_unambiguous_symbols() {
    for random in [0, 1, u64::MAX, 0x1234_5678_9ABC_DEF0] {
        let code = online(random);
        assert_eq!(code.len(), ONLINE_LEN);
        assert!(code.chars().all(|c| c.is_ascii_digit() || (c.is_ascii_uppercase() && !"ILOU".contains(c))), "{code}");
        assert_eq!(parse(&grouped(&code).to_lowercase()), Ok(Code::Online(code)));
    }
    assert_eq!(grouped("K7M2QX9TD"), "K7M-2QX-9TD");
    assert_ne!(online(1), online(2));
    // Only 45 bits count.
    assert_eq!(online(1 << 45), online(0));
}

#[test]
fn local_codes_round_trip() {
    for (ip, port, random) in [([192, 168, 1, 20], 47_900, 0x0ABC_DEF0_1234_u64), ([10, 0, 0, 1], 1, 0), ([127, 0, 0, 1], 65_535, u64::MAX)] {
        let local = Local::new(Ipv4Addr::from(ip), port, random);
        let code = local.code();
        assert_eq!(code.len(), LOCAL_LEN);
        let shown = grouped(&code);
        assert_eq!(shown.len(), LOCAL_LEN + 3, "four groups of five: {shown}");
        assert_eq!(parse(&shown), Ok(Code::Local(local)));
        assert_eq!(parse(&shown.to_lowercase().replace('-', " ")), Ok(Code::Local(local)));
        // The link a joiner connects with names the same session, and gives the code back.
        let link = local.link();
        assert_eq!(link, format!("figcraft://join?host={}:{port}&file=local&token={}", Ipv4Addr::from(ip), local.token()));
        assert_eq!(local.token().len(), 17);
        assert_eq!(Local::from_link(&link), Some(local));
    }
}

#[test]
fn tampered_local_codes_are_caught() {
    let code = Local::new(Ipv4Addr::new(192, 168, 1, 20), 47_900, 0x123_4567_89AB).code();
    let symbols = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let (mut tried, mut missed) = (0, 0);
    for at in 0..code.len() {
        for s in symbols.chars().filter(|s| code.chars().nth(at) != Some(*s)) {
            let mut changed: Vec<char> = code.chars().collect();
            changed[at] = s;
            tried += 1;
            missed += usize::from(parse(&changed.iter().collect::<String>()).is_ok());
        }
    }
    // Eight check bits: about one changed symbol in 256 slips through (and then fails to join).
    assert!(missed * 50 < tried, "{missed} of {tried} single-symbol changes went unnoticed");
    // Two symbols swapped.
    let mut swapped: Vec<char> = code.chars().collect();
    let at = (0..19).find(|i| swapped[*i] != swapped[*i + 1]).unwrap();
    swapped.swap(at, at + 1);
    assert!(parse(&swapped.iter().collect::<String>()).unwrap_err().contains("typing mistake"));
    // Wrong lengths say what a code looks like.
    assert!(parse(&code[..19]).unwrap_err().contains("9 characters"));
    assert!(parse("K7M2QX9T").unwrap_err().contains("that one has 8"));
}

#[test]
fn only_local_session_links_have_a_code() {
    for link in [
        "figcraft://join?host=192.168.1.20:47900&file=abc&token=local-0123456789a",
        "figcraft://join?host=192.168.1.20:47900&file=local&token=AbCdEfGhIjKlMnOpQrStUv",
        "figcraft://join?host=[::1]:47900&file=local&token=local-0123456789a",
        "figcraft://join?host=192.168.1.20:0&file=local&token=local-0123456789a",
        "figcraft://join?host=192.168.1.20:47900&file=local&token=local-0123456789az",
        "figcraft://join?host=10.0.0.1:47900&file=local&token=local-+0000000001",
        "figcraft://join?server=wss://example.org/&file=local&token=local-0123456789a",
        "ws://192.168.1.20:47900/?file=local&token=local-0123456789a",
        "",
    ] {
        assert_eq!(Local::from_link(link), None, "{link}");
    }
}

#[test]
fn the_new_messages_go_last_on_the_wire() {
    use crate::message::{Admin, AdminReply, ClientMsg, Resolve, ServerMsg, encode};
    // postcard writes a variant's index: an older peer reads everything it knew unchanged, and
    // fails only on the variants added for join codes (which is how the app tells).
    let tag = |bytes: Vec<u8>| bytes[1];
    let ask = Resolve { protocol: 2, client_version: String::new(), code: "K7M2QX9TD".into() };
    assert_eq!(tag(encode(&ClientMsg::Ping).unwrap()), 4);
    assert_eq!(tag(encode(&ClientMsg::Report { reason: String::new() }).unwrap()), 13);
    assert_eq!(tag(encode(&ClientMsg::Resolve(ask)).unwrap()), 14);
    assert_eq!(tag(encode(&ServerMsg::Pong).unwrap()), 7);
    assert_eq!(tag(encode(&ServerMsg::Reported { error: None }).unwrap()), 13);
    assert_eq!(tag(encode(&ServerMsg::Resolved { file: String::new(), token: String::new() }).unwrap()), 14);
    assert_eq!(tag(encode(&Admin::Blocked).unwrap()), 11);
    assert_eq!(tag(encode(&Admin::Code { file: String::new(), token: String::new() }).unwrap()), 12);
    assert_eq!(tag(encode(&AdminReply::Blocked(Vec::new())).unwrap()), 10);
    assert_eq!(tag(encode(&AdminReply::Code { code: String::new() }).unwrap()), 11);
}

#[test]
fn junk_never_panics() {
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    for _ in 0..20_000 {
        let len = (seed % 40) as usize;
        let text: String = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                char::from_u32((seed >> 40) as u32 % 0x250).unwrap_or('-')
            })
            .collect();
        let _ = parse(&text);
        let _ = Local::from_link(&text);
        let _ = grouped(&text);
    }
}
