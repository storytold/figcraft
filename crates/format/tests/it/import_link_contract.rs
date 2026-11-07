//! A text hyperlink's destination survives a REST import whole, however long it is (#459).
//!
//! The assertions are the reporter's; the helper returns its findings instead of unwrapping, so
//! that the crate's `unwrap_used`/`panic` lints hold outside a `#[test]` function.

use figcraft_doc::NodeKind;
use figcraft_format::rest;
use serde_json::json;

/// The link of the imported text, and the warnings the import reported. `None` for a link means
/// the layer was missing, was not text, or carried no link at all.
fn imported_link(length: usize, override_style: bool) -> (String, Option<String>, Vec<String>) {
    let url = format!("https://example.com/{}", "a".repeat(length - 20));
    assert_eq!(url.len(), length);
    let mut text = json!({"id":"1:2","type":"TEXT","name":"Link","characters":"A","relativeTransform":[[1,0,0],[0,1,0]]});
    let style = json!({"hyperlink":{"type":"URL","url":url}});
    if override_style {
        text["characterStyleOverrides"] = json!([1]);
        text["styleOverrideTable"] = json!({"1":style});
    } else {
        text["style"] = style;
    }
    let input = json!({"document":{"type":"DOCUMENT","children":[{"id":"1:1","type":"CANVAS","children":[text]}]}});
    assert!(rest::is_rest_json(&input));
    let Ok((doc, warnings)) = rest::import(&input, &rest::no_images) else {
        return (url, None, vec!["import failed".into()]);
    };
    let link = match doc.nodes().find(|n| n.name == "Link").map(|n| &n.kind) {
        Some(NodeKind::Text(data)) if override_style => data.runs.first().and_then(|r| r.style.link.clone()),
        Some(NodeKind::Text(data)) => data.style.link.clone(),
        _ => None,
    };
    (url, link, warnings)
}

/// Every length in turn: the destination comes back exactly, and nothing is reported.
fn check(override_style: bool) {
    let mut mismatches = Vec::new();
    for length in [200, 201, 240] {
        let (url, link, warnings) = imported_link(length, override_style);
        assert!(warnings.is_empty(), "import warned at {length}: {warnings:?}");
        if link.as_deref() != Some(url.as_str()) {
            mismatches.push((length, link.map_or(0, |l| l.len())));
        }
    }
    assert!(mismatches.is_empty(), "URLs changed, (input length, output length): {mismatches:?}");
}

#[test]
fn base_hyperlink_preserves_url() {
    check(false);
}

#[test]
fn override_hyperlink_preserves_url() {
    check(true);
}
