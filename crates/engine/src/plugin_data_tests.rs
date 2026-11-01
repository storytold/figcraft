//! Layer data (`pluginData`) reads back through `node.get` and `document.inspect` (#879): what
//! `node.set` writes on a layer or a page comes back as `{key: value}`, a removed key goes, and a
//! layer without any reports none.

use crate::Session;
use crate::components::tests::{ex, idv};
use serde_json::json;

#[test]
fn node_get_reports_the_layer_data_node_set_wrote() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let r = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})));
    let page = s.ds().unwrap().page;
    assert!(ex(&mut s, "node.get", json!({"id": f})).get("pluginData").is_none(), "no data, no field");
    ex(&mut s, "node.set", json!({"ids": [f], "props": {"pluginData": {"com.example.sync": "{\"v\":1}", "n": 2}}}));
    ex(&mut s, "node.set", json!({"ids": [page], "props": {"pluginData": {"com.example.sync": "page"}}}));
    let got = ex(&mut s, "node.get", json!({"id": f}))["pluginData"].clone();
    assert_eq!(got, json!({"com.example.sync": "{\"v\":1}", "n": "2"}), "values read back as the strings stored");
    assert_eq!(ex(&mut s, "node.get", json!({"id": page}))["pluginData"], json!({"com.example.sync": "page"}), "pages keep data too");
    assert!(ex(&mut s, "node.get", json!({"id": r})).get("pluginData").is_none(), "other layers report none");
    // The document tree reports it as well (the page and its children).
    let tree = crate::inspect::document(&s, 2, false);
    assert_eq!(tree["pages"][0]["pluginData"], json!({"com.example.sync": "page"}));
    // A null removes a key; undo brings it back.
    ex(&mut s, "node.set", json!({"ids": [f], "props": {"pluginData": {"n": null}}}));
    assert_eq!(ex(&mut s, "node.get", json!({"id": f}))["pluginData"], json!({"com.example.sync": "{\"v\":1}"}));
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(ex(&mut s, "node.get", json!({"id": f}))["pluginData"]["n"], "2");
}
