//! `widgets.selectAll`: every widget on the current page becomes the selection; `widgets.manage`
//! asks the UI to open the widgets window.

use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

/// A widget is a Frame layer carrying its plugin's id in the plugin data.
fn widget_frame(s: &mut Session, plugin: &str) -> NodeId {
    let id = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 80, "height": 40})).unwrap());
    let doc = s.doc_mut().unwrap();
    let n = doc.get_mut(id).unwrap();
    n.kind = NodeKind::Frame;
    n.plugin_data.insert("figcraft.widget".into(), plugin.into());
    id
}

fn rect(s: &mut Session) -> NodeId {
    id(&s.execute("shape.rectangle", &json!({"x": 200, "y": 0, "width": 10, "height": 10})).unwrap())
}

#[test]
fn select_all_widgets_picks_every_widget_on_the_page() {
    let mut s = Session::new();
    let w1 = widget_frame(&mut s, "acme.counter");
    let w2 = widget_frame(&mut s, "acme.timer");
    let plain = rect(&mut s);
    let r = s.execute("widgets.selectAll", &json!({})).unwrap();
    assert_eq!(r["count"], 2);
    let sel: Vec<NodeId> = r["selection"].as_array().unwrap().iter().map(|v| id_of(v).unwrap()).collect();
    assert!(sel.contains(&w1) && sel.contains(&w2));
    assert!(!sel.contains(&plain), "a layer that isn't a widget isn't selected");
    assert_eq!(s.selection(), sel);
}

#[test]
fn select_all_widgets_with_none_toasts_and_selects_nothing() {
    let mut s = Session::new();
    let plain = rect(&mut s);
    s.set_selection(vec![plain]);
    let r = s.execute("widgets.selectAll", &json!({})).unwrap();
    assert_eq!(r["count"], 0);
    assert!(s.selection().is_empty(), "nothing is selected");
    assert_eq!(s.message.as_deref(), Some("No widgets on this page"));
}

#[test]
fn manage_widgets_opens_the_widgets_window() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("widgets.manage", &json!({})).unwrap();
    assert!(s.ui_requests.iter().any(|r| r.kind == "dialog.widgets"));
}
