#![allow(clippy::unwrap_used)]
use super::*;
use figcraft_engine::Session;
fn host() -> Host {
    let h = Host::default();
    h.set_session("0123456789abcdef0123456789abcdef".into()).unwrap();
    h.connect("test".into(), "Test storage".into(), "account-a".into()).unwrap();
    h
}
fn capture(h: &Host, save_as: bool) -> (String, u64) {
    let mut session = Session::new();
    session.execute("file.new", &json!({})).unwrap();
    let doc = session.ds().unwrap().clone();
    let uid = doc.uid;
    (h.begin(vec![doc.clone()], uid, doc.revision, "example.figcraft".into(), vec![1, 2, 3], save_as).unwrap(), uid)
}
#[test]
fn off_until_explicit_registration_and_session_is_single_use() {
    let h = Host::default();
    assert!(!h.enabled());
    assert!(!h.connected());
    assert!(h.connect("test".into(), "Test".into(), "a".into()).is_err());
    assert!(host().set_session("0123456789abcdef".into()).is_err());
}
#[test]
fn each_save_has_a_choice_and_cancel_never_acknowledges_a_document() {
    let h = host();
    let (request, uid) = capture(&h, false);
    assert_eq!(h.take_events()[0].metadata["type"], "saveDialog");
    assert!(h.pending_for(uid));
    h.resolve_dialog(&request, "cancel", "").unwrap();
    let actions = h.take_actions();
    assert!(matches!(&actions[..],[Action::Save { status, .. }] if status == "cancel"));
    assert!(h.state().bindings.is_empty());
    assert!(h.result(request, "saved".into(), "file".into()).is_err());
}
#[test]
fn durable_ack_owns_its_capture_and_is_single_use() {
    let h = host();
    let (request, uid) = capture(&h, false);
    h.take_events();
    assert!(h.disconnect().is_err());
    h.resolve_dialog(&request, "provider", "durable.figcraft").unwrap();
    let event = h.take_events().remove(0);
    assert_eq!(event.metadata["type"], "save");
    assert_eq!(event.bytes, Some(vec![1, 2, 3]));
    h.result(request.clone(), "saved".into(), "file-1".into()).unwrap();
    assert!(h.result(request, "saved".into(), "file-1".into()).is_err());
    assert!(matches!(h.take_actions().as_slice(),[Action::Save { capture,resource,.. }] if capture.doc==uid && resource=="file-1"));
}
#[test]
fn replies_cannot_cross_the_chosen_destination_or_an_open_resource_alias() {
    let h = host();
    let (request, uid) = capture(&h, false);
    h.resolve_dialog(&request, "device", "local.figcraft").unwrap();
    assert!(h.result(request.clone(), "saved".into(), "remote".into()).is_err());
    h.result(request, "download".into(), String::new()).unwrap();
    h.take_actions();
    h.take_events();
    let (request, _) = capture(&h, false);
    h.state().bindings.insert(uid, "already-open".into());
    h.resolve_dialog(&request, "provider", "new.figcraft").unwrap();
    assert!(h.result(request, "saved".into(), "already-open".into()).is_err());
}
#[test]
fn unregister_clears_cached_account_documents_before_reconnect() {
    let h = host();
    h.state().bindings.insert(42, "account-a-resource".into());
    h.state().names.insert(42, "Account A.figcraft".into());
    h.refresh(vec![(42, 1, "Account A.figcraft".into(), false)]);
    assert_eq!(h.snapshot()["documents"][0]["resourceId"], "account-a-resource");
    h.disconnect().unwrap();
    let disconnected = h.snapshot();
    assert!(disconnected["documents"].is_null());
    assert!(disconnected["resources"].as_array().unwrap().is_empty());
    assert!(h.state().bindings.is_empty());
    assert!(h.state().names.is_empty());
    h.connect("test".into(), "Test storage".into(), "account-b".into()).unwrap();
    assert!(h.snapshot()["documents"].is_null());
}
#[test]
fn resource_reads_and_lists_are_scoped_to_current_account_connection() {
    let h = host();
    let connection = h.snapshot()["connectionId"].as_str().unwrap().to_string();
    let rows = vec![Resource { id: "file-1".into(), name: "remote.figcraft".into(), updated_at: String::new() }];
    assert!(h.set_resources("another-connection", rows.clone()).is_err());
    h.set_resources(&connection, rows).unwrap();
    assert!(h.request_open("unknown").is_err());
    let request = h.request_open("file-1").unwrap();
    assert!(h.open("forged".into(), "remote.figcraft".into(), vec![1]).is_err());
    h.open(request.clone(), "remote.figcraft".into(), vec![1]).unwrap();
    assert!(h.open(request.clone(), "remote.figcraft".into(), vec![1]).is_err());
    assert!(h.cancel_open(request.clone(), String::new()).is_err());
    assert!(h.disconnect().is_err());
    h.take_actions();
    h.finish_open(&request, "file-1", "remote.figcraft", Err("invalid archive".into()));
    h.disconnect().unwrap();
    let next = h.connect("test".into(), "Test storage".into(), "account-b".into()).unwrap();
    assert_ne!(connection, next);
    assert!(h.set_resources(&connection, Vec::new()).is_err());
    assert!(h.snapshot()["resources"].as_array().unwrap().is_empty());
}
#[test]
fn names_are_native_filenames_and_resource_ids_are_opaque() {
    assert!(valid_native_name("with accents áé.figcraft"));
    assert!(!valid_native_name("../project.figcraft"));
    assert!(!valid_native_name("project.svg"));
    assert!(!valid_id("../../project"));
    assert!(!valid_native_name("project.figcraft\0"));
}
#[test]
fn disabled_provider_does_not_discard_a_capture_already_awaiting_ack() {
    let h = host();
    let (request, uid) = capture(&h, false);
    h.set_enabled(false).unwrap();
    assert!(!h.enabled());
    assert!(h.pending_for(uid));
    h.resolve_dialog(&request, "provider", "in-flight.figcraft").unwrap();
    h.result(request, "saved".into(), "file-1".into()).unwrap();
    assert!(matches!(h.take_actions().as_slice(),[Action::Save { status,.. }] if status=="saved"));
}
