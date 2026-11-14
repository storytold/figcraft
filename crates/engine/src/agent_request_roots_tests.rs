//! A request for the interface carries the folders its caller was confined to: the window handles
//! it later, when the caller's roots no longer apply, and must run it under the same ones. A request
//! made by the user carries none.

use std::sync::Arc;

use serde_json::json;

use crate::Session;
use crate::file_access::{Roots, confined};

fn roots() -> Arc<Roots> {
    let dir = std::env::temp_dir().to_string_lossy().to_string();
    Arc::new(Roots::new(&[], &[dir]).unwrap())
}

#[test]
fn a_request_made_under_roots_carries_them() {
    let roots = roots();
    let mut s = Session::new();
    confined(Some(&roots), || s.request_ui("test.dialog", json!({})));
    let r = s.ui_requests.last().expect("the request was queued");
    assert_eq!(r.roots.as_deref(), Some(&*roots), "the request holds the caller's folders");
}

#[test]
fn a_request_made_without_roots_carries_none() {
    let mut s = Session::new();
    s.request_ui("test.dialog", json!({}));
    // Calibration: the request is queued, and the roots of an earlier confined call did not stay.
    let roots = roots();
    confined(Some(&roots), || ());
    s.request_ui("test.dialog", json!({}));
    assert_eq!(s.ui_requests.len(), 2);
    assert!(s.ui_requests.iter().all(|r| r.roots.is_none()));
}
