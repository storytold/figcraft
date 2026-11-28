//! The Fill section on a text whose characters were coloured while editing (#1856), driven
//! through egui headless: characters with different fills show as mixed content and + replaces
//! them on every character; characters that all share fills the layer doesn't have show those.

use egui::accesskit::Role;
use egui::{Pos2, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

const NOTE: &str = "Click + to replace mixed content";

/// A red "Hello" with its characters from `from` on coloured yellow while editing, selected.
fn ui(from: usize) -> (Ui, NodeId) {
    let mut s = Session::new();
    let t = NodeId(s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap()["id"].as_u64().unwrap());
    s.execute("fill.set", &json!({"ids": [t], "color": "#FF0000"})).unwrap();
    s.execute("text.style", &json!({"id": t, "start": from, "end": 5, "textFills": ["#FFFF00"]})).unwrap();
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    (u, t)
}

/// The accessible controls after a frame: role, label, value and centre.
fn controls(u: &mut Ui) -> Vec<(Role, String, String, Pos2)> {
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    nodes
        .iter()
        .map(|(_, n)| {
            let at = n.bounds().map_or(pos2(0.0, 0.0), |b| pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32));
            (n.role(), n.label().unwrap_or_default().to_string(), n.value().unwrap_or_default().to_string(), at)
        })
        .collect()
}

/// What the Fill section shows: its hex rows top first, whether the mixed-content note is there
/// (only a paint section shows it, and the strokes here are never mixed), and its + button. The
/// section's rows lie between its + and the Stroke section's; Selection colors is further down.
fn fill_section(c: &[(Role, String, String, Pos2)]) -> (Vec<String>, bool, Pos2) {
    let button = |name: &str| {
        c.iter().find(|(r, l, _, _)| *r == Role::Button && l == name).map(|(_, _, _, p)| *p).unwrap_or_else(|| panic!("no {name} button: {c:?}"))
    };
    let (top, bottom) = (button("Add fill"), button("Add stroke"));
    let mut rows: Vec<(f32, String)> =
        c.iter().filter(|(r, _, _, p)| *r == Role::TextInput && p.y > top.y && p.y < bottom.y).map(|(_, _, v, p)| (p.y, v.clone())).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let note = c.iter().any(|(_, l, v, _)| l == NOTE || v == NOTE);
    (rows.into_iter().map(|(_, v)| v).collect(), note, top)
}

fn fills_shown(u: &Ui, t: NodeId) -> Vec<String> {
    let n = u.app.session.doc().unwrap().node(t).unwrap().clone();
    let tx = n.kind.text().unwrap().clone();
    (0..tx.char_count())
        .map(|i| {
            tx.style_at(i)
                .fills
                .clone()
                .unwrap_or_else(|| n.fills.clone())
                .iter()
                .map(|p| p.solid_color().unwrap().to_hex())
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect()
}

#[test]
fn differently_coloured_characters_show_as_mixed_content_and_plus_replaces_them() {
    let (mut u, t) = ui(1);
    let c = controls(&mut u);
    let (rows, note, plus) = fill_section(&c);
    assert!(note, "the Fill section shows the note: {c:?}");
    assert!(rows.is_empty(), "no row for the layer's own red: {rows:?}");
    // + adds a fill on top of the layer's: every character shows the layer's fills now.
    let steps = u.app.session.ds().unwrap().history.undo.len();
    u.button(plus, true);
    u.button(plus, false);
    u.frame(vec![]);
    let n = u.app.session.doc().unwrap().node(t).unwrap().clone();
    assert_eq!(n.fills.len(), 2, "the new fill on the layer's: {:?}", n.fills);
    assert!(n.kind.text().unwrap().runs.is_empty(), "no character keeps fills of its own");
    assert!(fills_shown(&u, t).iter().all(|f| f == "FF0000+D9D9D9"), "{:?}", fills_shown(&u, t));
    assert_eq!(u.app.session.ds().unwrap().history.undo.len(), steps + 1, "one undo step");
    let c = controls(&mut u);
    let (rows, note, _) = fill_section(&c);
    assert!(!note, "no note once the characters agree: {c:?}");
    assert_eq!(rows, ["D9D9D9", "FF0000"], "the new fill above the layer's red: {c:?}");
}

#[test]
fn characters_sharing_fills_the_layer_lacks_show_theirs() {
    let (mut u, t) = ui(0);
    let n = u.app.session.doc().unwrap().node(t).unwrap().clone();
    assert_eq!(n.fills[0].solid_color().unwrap().to_hex(), "FF0000", "the layer still has its old fill");
    assert_eq!(fills_shown(&u, t), ["FFFF00"; 5]);
    let c = controls(&mut u);
    let (rows, note, _) = fill_section(&c);
    assert_eq!(rows, ["FFFF00"], "the characters' yellow is the row, not the layer's red: {c:?}");
    assert!(!note, "{c:?}");
}
