//! A connection dragged out of a variant (or a layer in it) onto a sibling variant makes a Change
//! to it (On click, Instant), as in Figma (#2158); onto the set's background or its own variant it
//! makes nothing, and onto a frame outside the set it still navigates.

use egui::{Pos2, pos2};
use figcraft_doc::NodeId;
use figcraft_doc::proto::{Action, Transition, Trigger};
use figcraft_engine::Session;
use serde_json::json;

use crate::FigApp;
use crate::RightTab;
use crate::canvas::proto_canvas::handles;
use crate::present_tests::{Ui, id};

fn screen_centre(app: &FigApp, n: NodeId) -> Pos2 {
    let b = app.session.doc().unwrap().world_bounds(n).unwrap();
    app.to_screen((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0)
}

struct Setup {
    u: Ui,
    set: NodeId,
    default: NodeId,
    hover: NodeId,
    layer: NodeId,
    outside: NodeId,
}

fn setup() -> Setup {
    setup_in(false)
}

/// `grouped`: the set is put in a group, inside a section around everything.
fn setup_in(grouped: bool) -> Setup {
    let mut s = Session::new();
    let mut variant = |name: &str, x: f64| {
        let f = id(&s.execute("frame.create", &json!({"x": x, "y": 0, "width": 100, "height": 80, "name": name})).unwrap());
        id(&s.execute("component.create", &json!({"ids": [f]})).unwrap())
    };
    let default = variant("Property 1=Default", 0.0);
    let hover = variant("Property 1=Hover", 300.0);
    let layer = id(&s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20, "parent": default})).unwrap());
    let outside = id(&s.execute("frame.create", &json!({"x": 0, "y": 500, "width": 200, "height": 150, "name": "Outside"})).unwrap());
    let set = id(&s.execute("component.combineVariants", &json!({"ids": [default, hover]})).unwrap());
    if grouped {
        let group = id(&s.execute("object.group", &json!({"ids": [set]})).unwrap());
        let section = id(&s.execute("node.create", &json!({"type": "section", "x": -200, "y": -200, "width": 1000, "height": 1000})).unwrap());
        s.execute("node.reparent", &json!({"ids": [group], "parent": section})).unwrap();
        assert_eq!(s.doc().unwrap().top_level_frame(default), None, "a set in a group has no top-level frame");
    }
    let mut u = Ui::new(s);
    u.app.ui.right_tab = RightTab::Prototype;
    Setup { u, set, default, hover, layer, outside }
}

/// Select `source`, then drag its connection handle (found inside) to the screen point `to` and release.
fn drag(t: &mut Setup, source: NodeId, to: Pos2) {
    t.u.run("select.set", json!({"ids": [source]}));
    t.u.run("view.zoomSelection", json!({}));
    t.u.run("view.zoomFit", json!({}));
    t.u.frames(2, Vec::new);
    let doc = t.u.app.session.doc().unwrap().clone();
    let (_, at) = *handles(&t.u.app, &doc, &[source]).first().unwrap();
    t.u.button(at, true);
    t.u.button(pos2((at.x + to.x) / 2.0, (at.y + to.y) / 2.0), true);
    t.u.button(to, true);
    t.u.button(to, false);
    t.u.frames(2, Vec::new);
}

fn interactions(t: &Setup, n: NodeId) -> Vec<figcraft_doc::proto::Interaction> {
    t.u.app.session.doc().unwrap().node(n).unwrap().interactions.clone()
}

fn assert_change_to(t: &Setup, source: NodeId, dest: NodeId) {
    let its = interactions(t, source);
    assert_eq!(its.len(), 1, "{its:?}");
    assert_eq!(its[0].trigger, Trigger::OnClick);
    assert_eq!(its[0].transition, Transition::Instant);
    assert!(matches!(its[0].actions.as_slice(), [Action::ChangeTo { dest: d }] if *d == dest), "{:?}", its[0].actions);
    assert_eq!(crate::proto_panel::editor(&t.u.ctx), Some((source, 0)));
}

#[test]
fn a_variants_connection_dropped_on_a_sibling_variant_changes_to_it() {
    let mut t = setup();
    let to = screen_centre(&t.u.app, t.hover);
    let src = t.default;
    drag(&mut t, src, to);
    assert_change_to(&t, t.default, t.hover);
}

#[test]
fn the_reverse_connection_changes_back() {
    let mut t = setup();
    let to = screen_centre(&t.u.app, t.default);
    let src = t.hover;
    drag(&mut t, src, to);
    assert_change_to(&t, t.hover, t.default);
}

#[test]
fn a_layer_inside_a_variant_changes_its_set_to_the_sibling() {
    let mut t = setup();
    let to = screen_centre(&t.u.app, t.hover);
    let src = t.layer;
    drag(&mut t, src, to);
    assert_change_to(&t, t.layer, t.hover);
}

#[test]
fn dropped_on_the_sets_background_or_its_own_variant_it_adds_nothing() {
    let mut t = setup();
    // Between the two variants (x 100..300) and inside the set's padding.
    let (a, b) = (screen_centre(&t.u.app, t.default), screen_centre(&t.u.app, t.hover));
    let gap = pos2((a.x + b.x) / 2.0, a.y);
    let src = t.default;
    drag(&mut t, src, gap);
    assert!(interactions(&t, t.default).is_empty());
    let own = screen_centre(&t.u.app, t.default);
    let src = t.default;
    drag(&mut t, src, own);
    assert!(interactions(&t, t.default).is_empty());
    let _ = t.set;
}

/// A set in a group in a section (CodeRabbit, #2161): its background still gives nothing, not a
/// Navigate to the section, and a sibling still gives Change to.
#[test]
fn in_a_grouped_set_the_background_adds_nothing_and_a_sibling_changes_to() {
    let mut t = setup_in(true);
    let (a, b) = (screen_centre(&t.u.app, t.default), screen_centre(&t.u.app, t.hover));
    let gap = pos2((a.x + b.x) / 2.0, a.y);
    let src = t.default;
    drag(&mut t, src, gap);
    assert!(interactions(&t, t.default).is_empty(), "{:?}", interactions(&t, t.default));
    let to = screen_centre(&t.u.app, t.hover);
    drag(&mut t, src, to);
    assert_change_to(&t, t.default, t.hover);
}

#[test]
fn a_variant_dropped_on_a_frame_outside_the_set_still_navigates() {
    let mut t = setup();
    let to = screen_centre(&t.u.app, t.outside);
    let src = t.default;
    drag(&mut t, src, to);
    let its = interactions(&t, t.default);
    assert_eq!(its.len(), 1, "{its:?}");
    assert!(matches!(its[0].actions.as_slice(), [Action::Navigate { dest, .. }] if *dest == t.outside), "{:?}", its[0].actions);
}
