//! Outline stroke keeps the renderer's holes and existing property ownership.
#[cfg(test)]
mod tests {
    use figcraft_doc::{Document, NodeId, NodeKind};
    use figcraft_engine::Session;
    use figcraft_render::{Options, View, render_view};
    use serde_json::{Value, json};

    fn run(s: &mut Session, command: &str, params: Value) -> Value {
        s.execute(command, &params).unwrap()
    }

    fn rectangle(s: &mut Session, opacity: f64) -> NodeId {
        NodeId(
            run(
                s,
                "shape.rectangle",
                json!({
                    "x": 24, "y": 24, "width": 64, "height": 64,
                    "props": {"fill": "#FF0000", "stroke": "#0000FF", "strokeWeight": 8, "opacity": opacity}
                }),
            )["id"]
                .as_u64()
                .unwrap(),
        )
    }

    fn pixels(doc: &Document) -> Vec<u8> {
        render_view(doc, doc.first_page().unwrap(), &View { origin: (0.0, 0.0), scale: 1.0, width: 112, height: 112 }, &Options::default())
            .unwrap()
            .data()
            .to_vec()
    }

    fn outlined(s: &mut Session, id: NodeId) -> NodeId {
        NodeId(run(s, "vector.outlineStroke", json!({"ids": [id]}))["ids"][0].as_u64().unwrap())
    }

    fn same_pixels(actual: &[u8], expected: &[u8]) {
        assert_eq!(actual.len(), expected.len());
        let differences: Vec<_> = actual.iter().zip(expected).enumerate().filter(|(_, (a, b))| a != b).take(8).collect();
        assert!(differences.is_empty(), "rendered channels changed (byte, actual/expected): {differences:?}");
    }

    #[test]
    fn outline_keeps_rings_through_undo_and_native_reopen() {
        for vector in [false, true] {
            let mut s = Session::new();
            let mut id = rectangle(&mut s, 1.0);
            if vector {
                id = NodeId(run(&mut s, "vector.flatten", json!({"ids": [id]}))["id"].as_u64().unwrap());
            }
            let before = s.doc().unwrap().clone();
            let expected = pixels(&before);
            let depth = s.undo_depth();
            let group = outlined(&mut s, id);
            let after = s.doc().unwrap().clone();
            same_pixels(&pixels(&after), &expected);
            assert!(after.node(id).unwrap().strokes.is_empty());
            assert_eq!(s.selection(), vec![group]);
            assert_eq!(s.undo_depth(), (depth.0 + 1, 0));
            let (loaded, notes) = figcraft_format::load_bytes(&figcraft_format::save_bytes(&after, None).unwrap()).unwrap();
            assert!(notes.is_empty(), "{notes:?}");
            same_pixels(&pixels(&loaded), &expected);
            assert!(s.undo().unwrap());
            assert_eq!(serde_json::to_value(s.doc().unwrap()).unwrap(), serde_json::to_value(&before).unwrap());
            assert!(s.redo().unwrap());
            assert_eq!(serde_json::to_value(s.doc().unwrap()).unwrap(), serde_json::to_value(&after).unwrap());
        }
    }

    #[test]
    fn stroke_only_keeps_holes_at_each_opacity() {
        for opacity in [0.0, 0.25, 0.5, 1.0] {
            let mut s = Session::new();
            let id = rectangle(&mut s, opacity);
            run(&mut s, "object.removeFill", json!({"ids": [id]}));
            let before = pixels(s.doc().unwrap());
            let vector = outlined(&mut s, id);
            let doc = s.doc().unwrap();
            assert!(matches!(doc.node(vector).unwrap().kind, NodeKind::Vector { .. }));
            assert_eq!(doc.node(vector).unwrap().opacity, opacity as f32);
            same_pixels(&pixels(doc), &before);
        }
    }

    #[test]
    fn outline_keeps_stroke_holes_and_alignment() {
        for props in [
            json!({"strokeAlign": "inside"}),
            json!({"strokeAlign": "outside"}),
            json!({"strokeAlign": "center"}),
            json!({"strokeSides": [2, 4, 8, 16]}),
            json!({"strokeAlign": "center", "dashes": [8, 4]}),
        ] {
            for filled in [false, true] {
                let mut s = Session::new();
                let id = rectangle(&mut s, 1.0);
                run(&mut s, "node.set", json!({"ids": [id], "props": props}));
                if !filled {
                    run(&mut s, "object.removeFill", json!({"ids": [id]}));
                }
                let before = pixels(s.doc().unwrap());
                outlined(&mut s, id);
                same_pixels(&pixels(s.doc().unwrap()), &before);
            }
        }
    }

    #[test]
    fn outlining_an_instance_keeps_its_inherited_opacity_link() {
        let mut s = Session::new();
        let component = NodeId(
            run(
                &mut s,
                "frame.create",
                json!({
                    "width": 64, "height": 64,
                    "props": {"fill": "#FF0000", "stroke": "#0000FF", "strokeWeight": 8, "opacity": 0.5}
                }),
            )["id"]
                .as_u64()
                .unwrap(),
        );
        run(&mut s, "component.create", json!({"ids": [component]}));
        let instance = NodeId(run(&mut s, "instance.create", json!({"component": component, "x": 200}))["id"].as_u64().unwrap());
        outlined(&mut s, instance);
        assert_eq!(s.doc().unwrap().node(instance).unwrap().opacity, 0.5);
        assert!(!s.doc().unwrap().node(instance).unwrap().overrides.contains("opacity"));
        run(&mut s, "node.set", json!({"id": component, "props": {"opacity": 0.25}}));
        let n = s.doc().unwrap().node(instance).unwrap();
        assert_eq!(n.opacity, 0.25, "the instance still follows its main");
        assert!(!n.overrides.contains("opacity"));
    }

    #[test]
    fn outlining_a_main_layer_keeps_its_instance_override() {
        let mut s = Session::new();
        let frame = NodeId(run(&mut s, "frame.create", json!({"width": 100, "height": 100}))["id"].as_u64().unwrap());
        let layer = NodeId(
            run(
                &mut s,
                "shape.rectangle",
                json!({
                    "parent": frame, "x": 24, "y": 24, "width": 64, "height": 64,
                    "props": {"fill": "#FF0000", "stroke": "#0000FF", "strokeWeight": 8}
                }),
            )["id"]
                .as_u64()
                .unwrap(),
        );
        let component = run(&mut s, "component.create", json!({"ids": [frame]}))["id"].clone();
        run(&mut s, "instance.create", json!({"component": component, "x": 200}));
        let copy = s.doc().unwrap().nodes().find(|n| n.source == Some(layer)).unwrap().id;
        run(&mut s, "node.set", json!({"id": copy, "props": {"opacity": 0.25}}));
        let before = s.doc().unwrap().clone();
        let view = View { origin: (0.0, 0.0), scale: 1.0, width: 312, height: 112 };
        let expected = render_view(&before, s.page().unwrap(), &view, &Options::default()).unwrap();
        let group = outlined(&mut s, layer);
        let doc = s.doc().unwrap();
        let actual = render_view(doc, s.page().unwrap(), &view, &Options::default()).unwrap();
        same_pixels(actual.data(), expected.data());
        let instance_group = doc.node(copy).unwrap();
        assert!(matches!(instance_group.kind, NodeKind::Group));
        assert_eq!(instance_group.source, Some(group));
        assert_eq!(instance_group.opacity, 0.25);
        assert!(instance_group.overrides.contains("opacity"));
        assert!(s.undo().unwrap());
        assert_eq!(serde_json::to_value(s.doc().unwrap()).unwrap(), serde_json::to_value(&before).unwrap());
        assert!(s.redo().unwrap());
        let actual = render_view(s.doc().unwrap(), s.page().unwrap(), &view, &Options::default()).unwrap();
        same_pixels(actual.data(), expected.data());
    }
}
