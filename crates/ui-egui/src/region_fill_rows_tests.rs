//! The Fill section with a region of a vector selected in edit mode (#1875), driven through egui
//! headless: the region's own fill shows, and what belongs to the layer (its fill style, a
//! variable bound to its fill) stays off the region's rows.

use egui::accesskit::Role;
use egui::{Pos2, pos2};
use figcraft_doc::{NodeId, NodeKind, Region, Segment, VectorNetwork, Vertex};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// Four points and four straight segments of a square, appended to `n`.
fn square(n: &mut VectorNetwork, x0: f64, y0: f64, size: f64) {
    let v0 = n.vertices.len();
    for (x, y) in [(x0, y0), (x0 + size, y0), (x0 + size, y0 + size), (x0, y0 + size)] {
        n.vertices.push(Vertex::new(x, y));
    }
    for k in 0..4 {
        n.segments.push(Segment::line(v0 + k, v0 + (k + 1) % 4));
    }
}

/// A vector of two squares on the page, the outer one's region then the inner one's, both
/// inheriting the layer's default grey fill (also used by the canvas handle and picker tests).
pub(crate) fn two_region_vector(s: &mut Session) -> NodeId {
    let r = s.execute("vector.create", &json!({"points": [[0, 0], [10, 0], [10, 10]], "closed": true})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    {
        let doc = s.doc_mut().unwrap();
        let node = doc.get_mut(id).unwrap();
        let mut n = VectorNetwork::default();
        square(&mut n, 0.0, 0.0, 200.0);
        square(&mut n, 60.0, 60.0, 80.0);
        n.regions =
            vec![Region { loops: vec![vec![0, 1, 2, 3]], ..Default::default() }, Region { loops: vec![vec![4, 5, 6, 7]], ..Default::default() }];
        node.kind = NodeKind::Vector { network: n };
        node.transform = figcraft_geom::Affine::IDENTITY;
        node.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::DEFAULT_SHAPE)];
        figcraft_engine::vectoredit::refit(doc, id).unwrap();
    }
    id
}

/// [`two_region_vector`] with its fill bound to a blue colour variable; in edit mode with the
/// inner region selected and given a red fill of its own.
fn ui() -> Ui {
    let mut s = Session::new();
    let id = two_region_vector(&mut s);
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "brand", "type": "color", "value": "#0000FF"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [id.0], "field": "fill", "variable": "brand"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("vector.edit.begin", &json!({"id": id.0})).unwrap();
    s.execute("vector.edit.select", &json!({"regions": [1]})).unwrap();
    s.execute("fill.set", &json!({"ids": [id.0], "color": "#FF0000"})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    u
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

fn button(c: &[(Role, String, String, Pos2)], name: &str) -> Pos2 {
    c.iter().find(|(r, l, _, _)| *r == Role::Button && l == name).map(|(_, _, _, p)| *p).unwrap_or_else(|| panic!("no {name} button: {c:?}"))
}

/// What the Fill section shows, between its + and the Stroke section's: its hex rows, whether a
/// Detach variable button is there, and whether its header has a Styles button.
fn fill_section(c: &[(Role, String, String, Pos2)]) -> (Vec<String>, bool, bool) {
    let (top, bottom) = (button(c, "Add fill"), button(c, "Add stroke"));
    let inside = |p: &Pos2| p.y > top.y + 2.0 && p.y < bottom.y - 2.0;
    let rows = c.iter().filter(|(r, _, _, p)| *r == Role::TextInput && inside(p)).map(|(_, _, v, _)| v.clone()).collect();
    let detach = c.iter().any(|(_, l, _, p)| l == "Detach variable" && inside(p));
    let styles = c.iter().any(|(r, l, _, p)| *r == Role::Button && l == "Styles" && (p.y - top.y).abs() < 2.0);
    (rows, detach, styles)
}

#[test]
fn a_selected_regions_rows_leave_out_the_layers_style_and_variable() {
    let mut u = ui();
    let red = figcraft_color::Color::from_hex("#FF0000").unwrap().to_hex();
    let (rows, detach, styles) = fill_section(&controls(&mut u));
    assert_eq!(rows, [red], "the region's own red, as a hex field");
    assert!(!detach, "the layer's variable binding is not the region's");
    assert!(!styles, "no fill style for a region's fills");
    // No region selected: the layer's row again, with its bound variable and the Styles button.
    u.app.run("vector.edit.select", json!({}));
    let (rows, detach, styles) = fill_section(&controls(&mut u));
    assert!(rows.is_empty(), "the layer's paint shows as its variable, not a hex field: {rows:?}");
    assert!(detach && styles);
}
