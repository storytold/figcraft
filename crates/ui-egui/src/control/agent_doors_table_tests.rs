//! The doors of the engine's table (`figcraft_engine::agent::DOORS`) against the rules of
//! `agent_doors_tests.rs`: a door added to the table without being classified by every rule breaks
//! here, and so does a method the window handles that the table does not know.

use figcraft_engine::agent::{DOORS, Door};

use super::agent_doors_tests::{
    METHODS, admission_through, document_front_through, emplacement_through, gesture_through, raw_input_through, undo_through,
};

#[test]
fn every_door_of_the_table_is_classified_by_every_rule() {
    for (method, _) in DOORS {
        // Each function panics on a method it has no arm for.
        admission_through(method);
        emplacement_through(method);
        undo_through(method, false);
        undo_through(method, true);
        gesture_through(method);
        document_front_through(method);
        raw_input_through(method);
    }
}

#[test]
fn every_method_the_window_handles_is_a_door() {
    for method in METHODS {
        assert!(DOORS.iter().any(|(m, _)| m == method), "`{method}` is handled by the window but is not in DOORS");
    }
}

#[test]
fn every_door_is_a_method_the_window_lists() {
    for (method, _) in DOORS {
        assert!(METHODS.contains(method), "`{method}` is in DOORS but the window's list of methods does not name it");
    }
}

#[test]
fn only_membership_escapes_the_admission_rule() {
    for (method, door) in DOORS {
        // The log is the same for every client and its reply carries nobody's name: nothing to admit.
        if *method == "events.poll" {
            assert_eq!(*door, Door::Events, "`events.poll` waits on the shared log");
            continue;
        }
        // Queued for a frame, not run by `handle`: its admission is the window's, when it takes the capture.
        if *method == "ui.screenshot" {
            continue;
        }
        assert_eq!(admission_through(method).is_none(), *door == Door::Membership, "`{method}`: admitted by its door");
    }
}
