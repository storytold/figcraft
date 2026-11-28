use crate::common;

use common::Client;
use serde_json::json;

#[test]
fn an_agent_anchors_a_dropdown_to_its_button() {
    let mut c = Client::new();
    let listed = c.ok("list_commands", json!({"filter": "proto.interaction.add"}));
    assert!(listed.as_str().unwrap().contains("triggerOutside"), "{listed}");

    let screen = c.create(json!({"type": "frame", "name": "Screen", "x": 0, "y": 0, "width": 400, "height": 800}));
    let menu = c.create(json!({"type": "frame", "name": "Menu", "x": 600, "y": 0, "width": 160, "height": 200}));
    let button = c.create(json!({"type": "frame", "parent": screen, "name": "Button", "x": 20, "y": 40, "width": 160, "height": 40}));
    let open = json!({"type": "openOverlay", "dest": menu, "position": "bottomLeft", "anchor": "triggerOutside"});
    c.run("proto.interaction.add", json!({"id": button, "action": open}));

    let action = &c.node(&button)["interactions"][0]["actions"][0];
    assert_eq!((action["position"].as_str(), action["anchor"].as_str()), (Some("bottomLeft"), Some("triggerOutside")), "{action}");

    let bad = json!({"type": "openOverlay", "dest": menu, "anchor": "nextToIt"});
    let err = c.err("run_command", json!({"command": "proto.interaction.add", "params": {"id": button, "action": bad}}));
    assert!(!err.is_empty());
    assert_eq!(c.node(&button)["interactions"].as_array().unwrap().len(), 1);
}
