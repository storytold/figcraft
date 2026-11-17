//! The table of doors of the engine against the rules of `agent_doors`: a door added to
//! `figcraft_engine::agent::DOORS` without being placed in every rule, or a method the backend
//! answers that the table does not list, fails here.

use std::path::{Path, PathBuf};

use super::agent_doors::{METHODS, admitted_through, front_through, gesture_through, placed_through, raw_input_through, undo_through};
use figcraft_engine::agent::{DOORS, Door, door};

/// Every method of the table is placed by every rule (each rule function fails on a method it
/// does not know), and the rules list nothing the table does not have.
#[test]
fn every_door_is_placed_in_every_rule() {
    for (method, d) in DOORS {
        let _ = admitted_through(method);
        let _ = placed_through(method);
        let _ = undo_through(method);
        let _ = gesture_through(method);
        let _ = front_through(method);
        let _ = raw_input_through(method);
        assert_eq!(door(method), Some(*d), "{method}: `door` answers what the table says");
    }
    for method in METHODS {
        assert!(door(method).is_some(), "{method}: the rules list a method the table does not");
    }
}

/// A door that is neither membership nor the event log is admitted by name: the rule of admission
/// has a request for every such method `Headless` answers. Membership admits by its own rule and
/// the event log admits nobody, so neither has one.
#[test]
fn only_membership_and_the_event_log_are_left_out_of_admission() {
    let answered = backend_methods();
    for (method, d) in DOORS {
        let left_out = matches!(d, Door::Membership | Door::Events);
        if left_out {
            assert!(admitted_through(method).is_none(), "{method}: admitted by its own rule, or by nobody");
        } else if answered.iter().any(|m| m == method) {
            assert!(admitted_through(method).is_some(), "{method}: a door admitted by name has no request in the rule");
        }
    }
}

/// The methods the backend can answer: those `Headless` matches, and those the tools, the server
/// and the command-line app send through `call`.
fn backend_methods() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    let headless = std::fs::read_to_string(root.join("src/headless.rs")).unwrap();
    let from = headless.find("fn dispatch(").unwrap();
    let to = headless[from..].find("impl Backend for Headless").map_or(headless.len(), |i| from + i);
    for line in headless[from..to].lines() {
        let line = line.trim();
        if line.starts_with('"')
            && let Some((arms, _)) = line.split_once("=>")
        {
            found.extend(arms.split('|').filter_map(|a| a.trim().strip_prefix('"')?.strip_suffix('"')).map(str::to_owned));
        }
    }
    let mut files: Vec<PathBuf> = ["src/backend.rs", "src/tools.rs", "src/server.rs"].iter().map(|f| root.join(f)).collect();
    collect_rs(&root.join("../../apps/figcraft-cli/src"), &mut files);
    for file in files {
        let src = std::fs::read_to_string(&file).unwrap();
        for part in src.split("call(\"").skip(1) {
            if let Some((name, _)) = part.split_once('"') {
                found.push(name.to_owned());
            }
        }
    }
    found.retain(|m| m.contains('.') && m.chars().all(|c| c.is_ascii_alphanumeric() || c == '.'));
    found.sort();
    found.dedup();
    found
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Calibration: the scan finds the methods `Headless` is known to answer. Then every one is in the
/// table of doors.
#[test]
fn every_method_the_backend_answers_is_in_the_table() {
    let found = backend_methods();
    for known in ["engine.execute", "document.inspect", "ui.key", "agent.join"] {
        assert!(found.iter().any(|m| m == known), "the scan misses {known}: {found:?}");
    }
    for method in &found {
        assert!(door(method).is_some(), "{method}: the backend answers it, the table of doors does not list it");
    }
}

/// Every request the server makes for its agent, whatever builds it (a tool, a resource, a render),
/// is a method of the table and carries the name and key, with the same key throughout its server (each server has its own).
#[test]
fn every_request_is_named_for_its_agent() {
    let servers = super::agent_named_requests::sweep();
    let calls: Vec<_> = servers.iter().flatten().collect();
    let methods: std::collections::BTreeSet<&str> = calls.iter().map(|(m, _)| m.as_str()).collect();
    for known in ["engine.execute", "document.inspect", "ui.render", "ui.screenshot", "node.get"] {
        assert!(methods.contains(known), "the sweep misses {known}: {methods:?}");
    }
    for calls in &servers {
        let mut key = None;
        for (method, p) in calls {
            assert!(door(method).is_some(), "{method}: sent by the server, not in the table of doors");
            assert_eq!(p["agent"].as_str(), Some("a"), "{method}: {p}");
            let k = p["agentKey"].as_str().filter(|k| !k.is_empty());
            assert!(k.is_some(), "{method}: no key: {p}");
            assert_eq!(*key.get_or_insert(k), k, "{method}: another key");
        }
    }
}
