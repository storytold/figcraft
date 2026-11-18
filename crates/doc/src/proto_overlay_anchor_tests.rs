use serde_json::json;

use crate::NodeId;
use crate::proto::*;

const TRIGGER: [f64; 2] = [100.0, 40.0];
const OVERLAY: [f64; 2] = [60.0, 200.0];

fn at(anchor: OverlayAnchor, p: OverlayPosition) -> Option<[f64; 2]> {
    anchor.offset(p, TRIGGER, OVERLAY)
}

#[test]
fn outside_sits_above_or_below_with_edges_lined_up() {
    use OverlayPosition as P;
    let out = OverlayAnchor::TriggerOutside;
    assert_eq!(at(out, P::BottomLeft), Some([0.0, 40.0]));
    assert_eq!(at(out, P::BottomCenter), Some([20.0, 40.0]));
    assert_eq!(at(out, P::BottomRight), Some([40.0, 40.0]));
    assert_eq!(at(out, P::TopLeft), Some([0.0, -200.0]));
    assert_eq!(at(out, P::TopCenter), Some([20.0, -200.0]));
    assert_eq!(at(out, P::TopRight), Some([40.0, -200.0]));
    assert_eq!(at(out, P::Center), Some([20.0, -80.0]));
}

#[test]
fn inside_uses_the_presets_on_the_trigger() {
    use OverlayPosition as P;
    let ins = OverlayAnchor::TriggerInside;
    assert_eq!(at(ins, P::TopLeft), Some([0.0, 0.0]));
    assert_eq!(at(ins, P::TopRight), Some([40.0, 0.0]));
    assert_eq!(at(ins, P::BottomLeft), Some([0.0, -160.0]));
    assert_eq!(at(ins, P::BottomCenter), Some([20.0, -160.0]));
    assert_eq!(at(ins, P::Center), Some([20.0, -80.0]));
}

#[test]
fn the_screen_and_manual_positions_have_no_preset_offset() {
    assert_eq!(at(OverlayAnchor::Screen, OverlayPosition::BottomLeft), None);
    assert_eq!(at(OverlayAnchor::TriggerOutside, OverlayPosition::Manual), None);
    assert_eq!(at(OverlayAnchor::TriggerInside, OverlayPosition::Manual), None);
}

#[test]
fn json_keeps_the_anchor_and_old_actions_stay_on_the_screen() {
    let a = Action::OpenOverlay {
        dest: NodeId(7),
        position: OverlayPosition::BottomLeft,
        close_on_outside: true,
        background: None,
        offset: None,
        anchor: OverlayAnchor::TriggerOutside,
    };
    let v = serde_json::to_value(&a).unwrap();
    assert_eq!(v["anchor"], "triggerOutside");
    assert_eq!(serde_json::from_value::<Action>(v).unwrap(), a);

    let old: Action = serde_json::from_value(json!({"type": "openOverlay", "dest": 7, "position": "bottomLeft"})).unwrap();
    assert!(matches!(old, Action::OpenOverlay { anchor: OverlayAnchor::Screen, .. }));
    assert!(serde_json::to_value(&old).unwrap().get("anchor").is_none());
}
