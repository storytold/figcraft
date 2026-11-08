//! `view.memoryUsage` asks the UI to open the Memory usage window.

use serde_json::json;

use crate::Session;

#[test]
fn memory_usage_opens_the_window() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("view.memoryUsage", &json!({})).unwrap();
    assert!(s.ui_requests.iter().any(|r| r.kind == "dialog.memoryUsage"), "{:?}", s.ui_requests.iter().map(|r| r.kind.clone()).collect::<Vec<_>>());
}
