//! `inspect_document` must not emit a message larger than a client can read in one piece: MCP
//! stdio is newline-delimited JSON, so one result is one message, and a client whose read buffer
//! it exceeds closes the pipe and kills the server for the rest of the session (#912).
use crate::common;

use common::Client;
use serde_json::{Value, json};

/// The allowance `inspect_document` keeps for the content object, the JSON-RPC envelope and the
/// request id (`ENVELOPE_RESERVE`).
const ENVELOPE: usize = 1024;

/// A document whose tree is far larger than the limits used below.
fn busy() -> Client {
    let mut c = Client::new();
    for i in 0..40 {
        let frame = c.create(json!({"type": "frame", "x": i * 10, "y": 0, "width": 200, "height": 200}));
        for j in 0..12 {
            c.create(json!({"type": "rectangle", "parent": frame, "x": j * 5, "y": 5, "width": 20, "height": 20}));
        }
    }
    c
}

/// The response and what its text block takes on the wire, escaped as the transport sends it.
fn inspect(c: &mut Client, args: Value) -> (Value, usize) {
    let r = c.call("inspect_document", args.clone());
    assert_eq!(r["isError"], false, "{args}: {}", r["content"][0]["text"]);
    let text = r["content"][0]["text"].as_str().expect("a text block");
    let wire = serde_json::to_string(&text).unwrap().len();
    (serde_json::from_str(text).expect("the block is JSON"), wire)
}

#[test]
fn a_tree_too_large_for_the_client_is_reported_shallower_and_says_so() {
    let mut c = busy();
    let (full, full_wire) = inspect(&mut c, json!({"depth": 6, "allPages": true}));
    assert!(full["truncated"].is_null(), "a tree within the limit is not truncated: {}", full["truncated"]);
    assert!(full_wire > 30_000, "the fixture has to outgrow the limits below: {full_wire}");

    // Asked for more than the caller can read: a shallower tree, inside the limit, and a marker.
    let (cut, cut_wire) = inspect(&mut c, json!({"depth": 6, "allPages": true, "maxBytes": 30_000}));
    assert!(cut_wire <= 30_000, "the response still exceeds maxBytes: {cut_wire}");
    let t = &cut["truncated"];
    assert_eq!(t["requestedDepth"], 6, "{t}");
    assert!(t["depth"].as_u64().unwrap() < 6, "the depth was reduced: {t}");
    assert_eq!(t["maxBytes"], 30_000, "{t}");
    assert_eq!(t["requestedDepthBytes"], full_wire, "the marker says what the full tree would have taken: {t}");
    // The marker points at the way to read the rest.
    assert!(t["hint"].as_str().unwrap_or("").contains("get_node"), "{t}");
    // Still a usable document, not just a marker: the page and its top-level layers are there.
    assert!(!cut["currentPage"].is_null(), "{cut}");
    assert!(cut["pages"][0]["children"].as_array().is_some_and(|c| !c.is_empty()), "{cut}");

    // The deepest tree that fits is the one sent: a limit that admits the whole tree is not cut.
    // maxBytes is the whole message, so the tree fits under it with room for the envelope, not
    // at it: a text block of exactly maxBytes would be sent inside a larger message than that.
    let (whole, _) = inspect(&mut c, json!({"depth": 6, "allPages": true, "maxBytes": full_wire + ENVELOPE}));
    assert!(whole["truncated"].is_null(), "a limit that leaves room for the envelope is not a truncation: {}", whole["truncated"]);
    let (tight, _) = inspect(&mut c, json!({"depth": 6, "allPages": true, "maxBytes": full_wire}));
    assert_eq!(tight["truncated"]["requestedDepth"], 6, "a limit at the exact size of the tree leaves none for the envelope: {tight}");
}

/// Every over-limit response carries the marker, however the depth was arrived at: a caller that
/// asked for depth 0 has no other way to learn that its limit was passed.
#[test]
fn a_depth_0_response_over_the_limit_says_so_too() {
    let mut c = busy();
    let (deep, _) = inspect(&mut c, json!({"depth": 0, "allPages": true}));
    assert!(deep["truncated"].is_null(), "within the default limit: {}", deep["truncated"]);
    let (v, wire) = inspect(&mut c, json!({"depth": 0, "allPages": true, "maxBytes": 1024}));
    let t = &v["truncated"];
    assert_eq!((t["requestedDepth"].as_u64(), t["depth"].as_u64()), (Some(0), Some(0)), "{t}");
    // `bytes` is the tree, measured before the marker was added to it, so the message that goes
    // out is larger still -- there being nothing smaller to send.
    assert_eq!(t["bytes"], t["requestedDepthBytes"], "{t}");
    assert!(wire > 1024, "the depth-0 tree is sent over the limit, with the marker saying so: {wire}");
    assert!(!v["pages"].is_null(), "the tree is still sent, there being nothing smaller: {v}");
}

#[test]
fn a_limit_below_the_shallowest_tree_still_answers() {
    // Nothing smaller than depth 0 exists, so it is sent with the marker rather than an error:
    // an error would leave the caller with nothing at all.
    let mut c = busy();
    let (v, _) = inspect(&mut c, json!({"depth": 6, "allPages": true, "maxBytes": 1024}));
    assert_eq!(v["truncated"]["depth"], 0, "{}", v["truncated"]);
    assert_eq!(v["truncated"]["requestedDepth"], 6, "{}", v["truncated"]);
}

#[test]
fn maxbytes_is_checked() {
    let mut c = Client::new();
    let r = c.call("inspect_document", json!({"depth": 1, "maxBytes": 10}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["content"][0]["text"].as_str().unwrap_or("").contains("at least 1024"), "{r}");
}

/// `inspect_document` sends a shallower tree and points at `get_node` for the rest, so `get_node`
/// must keep to the same limit -- it was unbounded. Measured on `main`: a frame with 15,000
/// children answers with a 10.5 MiB message at depth 1, over the 10 MiB buffer the report is
/// about, and no depth makes that smaller because the width is at depth 1. The children are
/// windowed instead, with the offset to ask for next.
#[test]
fn a_node_with_more_children_than_fit_is_windowed() {
    let mut c = Client::new();
    let frame = c.create(json!({"type": "frame", "x": 0, "y": 0, "width": 400, "height": 400}));
    for i in 0..40 {
        c.create(json!({"type": "rectangle", "parent": frame, "x": i * 5, "y": 5, "width": 20, "height": 20}));
    }

    // Whole, to measure against: every child, no marker.
    let (whole, whole_wire) = get_node(&mut c, json!({"id": frame, "depth": 1}));
    let total = whole["children"].as_array().expect("children").len();
    assert_eq!(total, 40, "{whole}");
    assert!(whole["truncated"].is_null(), "within the default limit: {}", whole["truncated"]);

    // A limit that holds only some of them: a window, and the marker says how many and what next.
    let limit = whole_wire / 3;
    assert!(limit >= 1024, "the fixture has to outgrow the floor: {whole_wire}");
    let (cut, cut_wire) = get_node(&mut c, json!({"id": frame, "depth": 1, "maxBytes": limit}));
    assert!(cut_wire <= limit, "the response still exceeds maxBytes: {cut_wire} > {limit}");
    let window = &cut["truncated"]["children"];
    let shown = cut["children"].as_array().expect("children").len();
    assert_eq!(window["from"], json!(0), "{window}");
    assert_eq!(window["shown"], json!(shown), "{window}");
    assert_eq!(window["total"], json!(total), "{window}");
    assert!(shown > 0 && shown < total, "some children, not all: {shown} of {total}");
    assert_eq!(window["nextOffset"], json!(shown), "the offset to ask for next: {window}");
    assert!(cut["truncated"]["reason"].as_str().unwrap_or("").contains("over the"), "cut by size: {}", cut["truncated"]);
    // The layer itself is still there, not just a marker.
    assert_eq!(cut["id"], whole["id"]);

    // Paging through with that offset reaches every child exactly once, in order.
    let mut seen: Vec<Value> = Vec::new();
    let mut offset = Some(0u64);
    while let Some(at) = offset {
        let (page, wire) = get_node(&mut c, json!({"id": frame, "depth": 1, "maxBytes": limit, "offset": at}));
        assert!(wire <= limit, "page at {at} exceeds maxBytes: {wire}");
        seen.extend(page["children"].as_array().cloned().unwrap_or_default());
        offset = page["truncated"]["children"]["nextOffset"].as_u64();
        assert!(seen.len() <= total, "a page repeated a child: {} of {total}", seen.len());
    }
    assert_eq!(seen.len(), total, "every child was reported once");
    assert_eq!(seen, whole["children"].as_array().cloned().unwrap_or_default(), "in the same order");
}

/// The response and what its text block takes on the wire, for one `get_node` call.
fn get_node(c: &mut Client, args: Value) -> (Value, usize) {
    let r = c.call("get_node", args.clone());
    assert_eq!(r["isError"], false, "{args}: {}", r["content"][0]["text"]);
    let text = r["content"][0]["text"].as_str().expect("a text block");
    let wire = serde_json::to_string(&text).unwrap().len() + ENVELOPE;
    (serde_json::from_str(text).expect("the block is JSON"), wire)
}

/// A child too large for the limit on its own is passed over, one per page of just the marker,
/// so paging moves on and every page stays within the limit.
#[test]
fn children_too_large_to_fit_page_one_at_a_time() {
    let mut c = Client::new();
    let frame = c.create(json!({"type": "frame", "x": 0, "y": 0, "width": 400, "height": 400}));
    for i in 0..3 {
        c.create(json!({"type": "rectangle", "parent": frame, "x": i * 5, "y": 5, "width": 20, "height": 20, "name": "n".repeat(3000)}));
    }
    let mut offset = Some(0u64);
    let mut pages = 0;
    while let Some(at) = offset {
        let (page, wire) = get_node(&mut c, json!({"id": frame, "depth": 1, "maxBytes": 2500, "offset": at}));
        assert!(wire <= 2500, "page at {at} exceeds maxBytes: {wire}");
        assert_eq!(page["children"].as_array().map_or(0, Vec::len), 0, "{}", page["truncated"]);
        assert!(!page["truncated"].is_null(), "the marker stays: page at {at}");
        offset = page["truncated"]["children"]["nextOffset"].as_u64();
        assert!(offset.is_none_or(|o| o > at), "paging moves on from {at}");
        pages += 1;
        assert!(pages <= 3);
    }
    assert_eq!(pages, 3);

    // A sole child passed over still says the answer was cut.
    let single = c.create(json!({"type": "frame", "x": 500, "y": 0, "width": 40, "height": 40}));
    c.create(json!({"type": "rectangle", "parent": single, "x": 0, "y": 0, "width": 20, "height": 20, "name": "n".repeat(3000)}));
    let (page, _) = get_node(&mut c, json!({"id": single, "depth": 1, "maxBytes": 2500}));
    assert_eq!(page["children"].as_array().map_or(0, Vec::len), 0);
    assert_eq!(page["truncated"]["children"]["total"], json!(1), "{}", page["truncated"]);
}

/// A layer whose own fields pass the limit with every child left out is refused: an answer would
/// be over maxBytes, and the marker would only make it larger.
#[test]
fn a_layer_too_large_on_its_own_is_refused() {
    let mut c = Client::new();
    let rect = c.create(json!({"type": "rectangle", "x": 0, "y": 0, "width": 20, "height": 20, "name": "n".repeat(3000)}));
    let r = c.call("get_node", json!({"id": rect, "maxBytes": 2500}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["content"][0]["text"].as_str().unwrap_or("").contains("maxBytes"), "{r}");
    // A layer of ordinary size answers within the same limit.
    let small = c.create(json!({"type": "rectangle", "x": 0, "y": 0, "width": 20, "height": 20}));
    let (v, wire) = get_node(&mut c, json!({"id": small, "maxBytes": 2500}));
    assert!(wire <= 2500 && v["truncated"].is_null(), "{wire}: {v}");
}

/// A window that only an `offset` made, of a subtree within the limit, is paging: the marker
/// doesn't say the limit was passed.
#[test]
fn an_offset_alone_pages_without_claiming_the_limit() {
    let mut c = Client::new();
    let frame = c.create(json!({"type": "frame", "x": 0, "y": 0, "width": 400, "height": 400}));
    for i in 0..3 {
        c.create(json!({"type": "rectangle", "parent": frame, "x": i * 5, "y": 5, "width": 20, "height": 20}));
    }
    let (page, _) = get_node(&mut c, json!({"id": frame, "depth": 1, "offset": 1}));
    let t = &page["truncated"];
    assert_eq!(page["children"].as_array().map_or(0, Vec::len), 2, "{t}");
    assert!(t["children"]["nextOffset"].is_null(), "{t}");
    let reason = t["reason"].as_str().unwrap_or("");
    assert!(reason.contains("offset") && !reason.contains("over the"), "{reason}");
}
