//! Your name changed in Preferences while in a live session: everyone there sees the new one,
//! people who join later too, and a rename made while joining arrives once the host reads it
//! (#1380).

use serde_json::json;

use crate::session_tests::{host_and_join, pump, session};

#[test]
fn a_rename_reaches_everyone_in_the_session() {
    let (mut host, mut guest, hana, ada) = host_and_join();
    pump(&mut host, &mut guest, "both told signals work", |h, g| h.live().signals && g.live().signals);
    host.message = None;
    guest.execute("pref.userName", &json!({"name": "Bob"})).unwrap();
    assert_eq!(guest.live().info.as_ref().unwrap().name, "Bob", "you see it at once");
    pump(&mut host, &mut guest, "the guest's new name at the host", |h, _| h.live().peers[&ada].name == "Bob");
    assert_eq!(host.message, None, "a rename isn't announced as someone joining");
    host.execute("pref.userName", &json!({"name": "Kim"})).unwrap();
    pump(&mut host, &mut guest, "the host's new name at the guest", |_, g| g.live().peers[&hana].name == "Kim");
    assert_eq!(guest.live().info.as_ref().unwrap().name, "Bob", "the server confirmed the guest's");
    // Renamed while joining: the hello had the old name, the new one follows once signals work.
    let mut late = session("Lin");
    late.execute("collab.join", &json!({"link": host.live().info.as_ref().unwrap().link.clone()})).unwrap();
    late.execute("pref.userName", &json!({"name": "Lou"})).unwrap();
    pump(&mut host, &mut late, "the late joiner's name", |h, l| {
        h.live().peers.values().any(|p| p.name == "Lou") && l.live().info.as_ref().is_some_and(|i| i.name == "Lou")
    });
    let names: Vec<&str> = late.live().peers.values().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"Kim") && names.contains(&"Bob"), "newcomers get the new names: {names:?}");
    // Blank goes back to the default for comments; the session keeps the name it shows.
    guest.execute("pref.userName", &json!({"name": " "})).unwrap();
    assert_eq!(guest.live().info.as_ref().unwrap().name, "Bob");
    host.execute("collab.stop", &json!({})).unwrap();
}
