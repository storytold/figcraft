//! Accepting a library update that changes only a paint style refreshes the layers linked to it.

use figcraft_doc::{NodeId, StyleKind};
use figcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

fn layer_paint(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().fills[0].solid_color().unwrap().to_hex()
}

fn style_paint(s: &Session, id: &str) -> String {
    match &s.doc().unwrap().style(id).unwrap().kind {
        StyleKind::Paint { paints } => paints[0].solid_color().unwrap().to_hex(),
        _ => panic!("expected paint style"),
    }
}

struct Fixture(std::path::PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn accepted_library_paint_reaches_linked_layer() {
    let fixture = Fixture(std::env::temp_dir().join(format!("figcraft-style-accept-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&fixture.0);
    std::fs::create_dir(&fixture.0).unwrap();
    let path = fixture.0.join("library.figcraft");
    let mut publisher = Session::new();
    run(&mut publisher, "style.create", json!({"type":"paint","name":"Brand","value":"#0000ff"}));
    let publication = run(&mut publisher, "library.publish", json!({}));
    assert_eq!(publication["components"], 0);
    assert_eq!(publication["styles"], 1);
    run(&mut publisher, "file.saveAs", json!({"path":path}));
    let mut consumer = Session::new();
    let rectangle = run(&mut consumer, "shape.rectangle", json!({"width":40,"height":40}));
    let layer = NodeId(rectangle["id"].as_u64().unwrap());
    run(&mut consumer, "library.add", json!({"path":path}));
    let applied = run(&mut consumer, "library.applyStyle", json!({"library":publication["id"],"style":"Brand","id":layer}));
    let style = applied["style"].as_str().unwrap();
    assert_eq!(layer_paint(&consumer, layer), "0000FF");
    run(&mut publisher, "style.update", json!({"style":"Brand","value":"#00ff00"}));
    run(&mut publisher, "library.publish", json!({}));
    run(&mut publisher, "file.save", json!({}));
    assert_eq!(run(&mut consumer, "library.updates", json!({}))["count"], 1);
    let accepted = run(&mut consumer, "library.acceptUpdates", json!({}));
    assert_eq!(accepted["styles"], 1);
    assert_eq!(accepted["components"], 0);
    assert_eq!(accepted["variables"], 0);
    assert_eq!(style_paint(&consumer, style), "00FF00");
    assert_eq!(consumer.doc().unwrap().node(layer).unwrap().styles.fill.as_deref(), Some(style));
    // Captured before any reapply so a later action can't hide a missed refresh.
    let after_accept = layer_paint(&consumer, layer);
    assert_eq!(after_accept, "00FF00");
    assert_eq!(run(&mut consumer, "library.updates", json!({}))["count"], 0);
    consumer.undo().unwrap();
    assert_eq!(style_paint(&consumer, style), "0000FF");
    assert_eq!(layer_paint(&consumer, layer), "0000FF");
    consumer.redo().unwrap();
    assert_eq!(style_paint(&consumer, style), "00FF00");
    assert_eq!(layer_paint(&consumer, layer), "00FF00");
    run(&mut consumer, "style.apply", json!({"style":style,"id":layer}));
    assert_eq!(layer_paint(&consumer, layer), "00FF00");
}
