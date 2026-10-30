//! The window that makes a colour the eyedropper picked (with Cmd/Ctrl+Shift) into a colour style
//! or variable: it opens from the eyedropper's request, Create makes what it says, and a variable
//! goes into the chosen collection or, with none, a new one, in one undo step.

use serde_json::json;

use crate::export_preview_tests::{click, labelled};
use crate::layers_drag_tests::Ui;
use crate::sampled_color::{SampledColor, create_params};

fn opened(s: figcraft_engine::Session) -> Ui {
    let mut u = Ui::new(s);
    u.app.session.request_ui("dialog.createFromColor", json!({"color": "#ff8800"}));
    u.app.handle_ui_requests();
    u.frame(vec![]);
    u.frame(vec![]);
    u
}

fn press(u: &mut Ui, text: &str) {
    let shown = labelled(u);
    let at = shown.iter().find(|(t, _)| t == text).map(|x| x.1).unwrap_or_else(|| panic!("{text:?} in {shown:?}"));
    click(u, at);
}

#[test]
fn the_eyedroppers_request_opens_the_window_and_create_makes_a_style() {
    let mut u = opened(figcraft_engine::Session::new());
    let s = u.app.ui.sampled_color.clone().expect("the window is open");
    assert_eq!((s.color.as_str(), s.variable, s.name.as_str()), ("#FF8800", false, "Color 1"));
    let texts: Vec<String> = labelled(&mut u).into_iter().map(|(t, _)| t).collect();
    assert!(texts.iter().any(|t| t == "Create from color") && texts.iter().any(|t| t == "FF8800"), "{texts:?}");
    press(&mut u, "Create");
    let doc = u.app.session.doc().unwrap();
    let style = doc.styles.iter().find(|s| s.name == "Color 1").expect("a style");
    let colour = match &style.kind {
        figcraft_doc::StyleKind::Paint { paints } => paints.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()),
        _ => None,
    };
    assert_eq!(colour.as_deref(), Some("FF8800"), "a paint style of the colour: {style:?}");
    assert!(u.app.ui.sampled_color.is_none(), "it closes");
}

#[test]
fn a_variable_goes_into_a_new_or_the_chosen_collection() {
    // No collection yet: one is made with it, one undo step.
    let mut u = opened(figcraft_engine::Session::new());
    let undo = u.app.session.undo_depth().0;
    if let Some(s) = u.app.ui.sampled_color.as_mut() {
        s.variable = true;
        s.name = "Accent".into();
    }
    u.frame(vec![]);
    press(&mut u, "Create");
    let doc = u.app.session.doc().unwrap();
    let v = doc.variables.variables.iter().find(|v| v.name == "Accent").expect("a variable");
    let c = doc.variables.collection(&v.collection).expect("its collection");
    assert_eq!(c.name, "Collection");
    assert_eq!(u.app.session.undo_depth().0, undo + 1, "one step");
    // With collections: the first is chosen, and the variable goes there.
    let mut s = figcraft_engine::Session::new();
    s.execute("variable.createCollection", &json!({"name": "Brand"})).unwrap();
    let u = opened(s);
    let state = u.app.ui.sampled_color.clone().unwrap();
    let brand = u.app.session.doc().unwrap().variables.collections[0].id.clone();
    assert_eq!(state.collection.as_deref(), Some(brand.as_str()));
    let (cmd, params) = create_params(&u.app, &SampledColor { variable: true, name: "Accent".into(), ..state });
    assert_eq!((cmd, params["collection"].as_str()), ("variable.create", Some(brand.as_str())));
    assert_eq!((params["type"].as_str(), params["value"].as_str()), (Some("color"), Some("#FF8800")));
}
