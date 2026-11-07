use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::import_report::Report;
use crate::layers_drag_tests::Ui;

/// A saved file with text in `families` (the first two layers share the first family).
fn saved(name: &str, families: &[&str]) -> String {
    let mut s = Session::new();
    for (i, f) in families.iter().chain(families.first()).enumerate() {
        s.execute("text.create", &json!({"text": "Hamburgefonstiv", "x": 0, "y": 40 * i, "props": {"fontFamily": f}})).unwrap();
    }
    let path = std::env::temp_dir().join(format!("figcraft-missing-fonts-{name}-{}.figcraft", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    s.execute("file.saveAs", &json!({"path": path})).unwrap();
    path
}

/// The installed fonts are indexed, as the desktop app does at startup.
fn fonts_indexed() -> bool {
    figcraft_text::scan_system_fonts();
    figcraft_text::installed_fonts_known()
}

fn listed(u: &Ui) -> Value {
    u.app.inspect()["missingFonts"].clone()
}

#[test]
fn opening_a_file_lists_its_missing_fonts_and_not_the_installed_ones() {
    if !fonts_indexed() {
        eprintln!("skipped: system fonts opted out");
        return;
    }
    let installed = figcraft_text::family_names().into_iter().next();
    let mut families = vec!["FigCraft Missing UI"];
    families.extend(installed.as_deref());
    let path = saved("open", &families);
    let mut u = Ui::new(Session::new());
    // Programmatic opens (control channel, MCP, scripts) never open the window.
    u.app.run("file.open", json!({"path": path}));
    u.frame(vec![]);
    assert_eq!(listed(&u), Value::Null);
    u.app.run("file.close", json!({}));

    u.app.run_user("file.open", json!({"path": path}));
    std::fs::remove_file(&path).unwrap();
    u.frame(vec![]);
    assert_eq!(listed(&u), json!({"families": [{"family": "FigCraft Missing UI", "layers": 2, "source": null}], "indexing": false, "consent": null}));
    u.frame(vec![]);
    let shown = u.frame_output(vec![]).shapes;
    assert!(crate::import_report_tests::shown_texts(shown).iter().any(|t| t == "2 text layers"));

    // It goes with its file.
    u.app.run("file.close", json!({}));
    u.frame(vec![]);
    assert_eq!(listed(&u), Value::Null);
}

#[test]
fn a_file_without_missing_fonts_opens_no_window() {
    if !fonts_indexed() {
        eprintln!("skipped: system fonts opted out");
        return;
    }
    let Some(installed) = figcraft_text::family_names().into_iter().next() else {
        eprintln!("skipped: no fonts on this machine");
        return;
    };
    let path = saved("clean", &[&installed]);
    let mut u = Ui::new(Session::new());
    u.app.run_user("file.open", json!({"path": path}));
    std::fs::remove_file(&path).unwrap();
    u.frame(vec![]);
    assert_eq!(listed(&u), Value::Null);
    assert!(u.app.ui.font_checks.is_empty(), "the check ran");
}

#[test]
fn it_waits_for_the_import_report_to_close() {
    if !fonts_indexed() {
        eprintln!("skipped: system fonts opted out");
        return;
    }
    let path = saved("report", &["FigCraft Missing Report"]);
    let mut u = Ui::new(Session::new());
    u.app.run_user("file.open", json!({"path": path}));
    std::fs::remove_file(&path).unwrap();
    let mut report =
        Report::from_result("file.open", &json!({}), &json!({"warnings": ["Synthetic note"], "summary": {"name": "Synthetic"}})).unwrap();
    report.uid = u.app.session.ds().unwrap().uid;
    u.app.ui.import_report = Some(report);
    u.frame(vec![]);
    assert_eq!(listed(&u), Value::Null, "the import report comes first");
    u.app.ui.import_report = None;
    u.frame(vec![]);
    assert_eq!(listed(&u)["families"][0]["family"], "FigCraft Missing Report");
}

#[test]
fn text_menu_shows_the_window_for_the_file_in_front_even_with_nothing_missing() {
    assert!(fonts_indexed());
    let mut u = Ui::new(Session::new());
    assert!(u.app.run("text.missingFonts", json!({})).is_some());
    assert_eq!(listed(&u), json!({"families": [], "indexing": false, "consent": null}));
    u.frame(vec![]);
    let shown = u.frame_output(vec![]).shapes;
    assert!(crate::import_report_tests::shown_texts(shown).iter().any(|t| t == "All the fonts this file uses are installed"));
    let escape =
        vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }];
    u.frame(escape);
    assert_eq!(listed(&u), Value::Null, "Escape closes it");
}

#[test]
fn asked_for_before_the_fonts_are_indexed_it_lists_them_once_they_are() {
    assert!(fonts_indexed());
    let path = saved("indexing", &["FigCraft Missing Indexing"]);
    let mut u = Ui::new(Session::new());
    u.app.run("file.open", json!({"path": path}));
    std::fs::remove_file(&path).unwrap();
    // As Text ▸ Missing fonts leaves it while the startup scan is still running.
    let uid = u.app.session.ds().unwrap().uid;
    u.app.ui.missing_fonts = Some(crate::missing_fonts::Shown::new(uid, Vec::new(), true));
    u.frame(vec![]);
    assert_eq!(
        listed(&u),
        json!({"families": [{"family": "FigCraft Missing Indexing", "layers": 2, "source": null}], "indexing": false, "consent": null})
    );
}

#[test]
fn placing_images_or_clearing_recents_does_not_bring_it_back() {
    assert!(fonts_indexed());
    let path = saved("place", &["FigCraft Missing Place"]);
    let svg = std::env::temp_dir().join(format!("figcraft-missing-fonts-place-{}.svg", std::process::id()));
    std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#).unwrap();
    let mut u = Ui::new(Session::new());
    u.app.run_user("file.open", json!({"path": path}));
    u.frame(vec![]);
    assert_eq!(listed(&u)["families"][0]["family"], "FigCraft Missing Place");
    u.app.ui.missing_fonts = None;
    // A placed SVG, the same file opened again (it only comes to the front), Clear recent.
    assert!(u.app.run_user("file.import", json!({"path": svg.to_string_lossy()})).is_some());
    assert!(u.app.run_user("file.open", json!({"path": path})).is_some());
    u.app.run_user("file.openRecent", json!({"clear": true}));
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(&svg).unwrap();
    u.frame(vec![]);
    assert!(u.app.ui.font_checks.is_empty());
    assert_eq!(listed(&u), Value::Null);
}

/// The middle of the button labelled `label` in the window, from the AccessKit tree.
fn click_button(u: &mut Ui, label: &str) {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let update = u.frame_output(vec![]).platform_output.accesskit_update.expect("an AccessKit update");
    let node = update.nodes.iter().map(|(_, n)| n).find(|n| n.label() == Some(label)).unwrap_or_else(|| panic!("no {label:?} button"));
    let b = node.bounds().unwrap();
    let at = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
    u.button(at, true);
    u.button(at, false);
    u.frame(vec![]);
}

#[test]
fn a_family_the_font_store_has_offers_download_and_asks_before_any_download() {
    // A store folder of the tests' own, where downloading was never turned on.
    figcraft_engine::fonts::download::use_store_dir(std::env::temp_dir().join(format!("figcraft-ui-font-store-{}", std::process::id())));
    assert!(fonts_indexed());
    let Some(store) = ["Lato", "Merriweather"].into_iter().find(|f| !figcraft_text::has_family(f)) else {
        eprintln!("skipped: these fonts are installed");
        return;
    };
    let path = saved("download", &["FigCraft Missing Download", store]);
    let mut u = Ui::new(Session::new());
    u.app.run_user("file.open", json!({"path": path}));
    std::fs::remove_file(&path).unwrap();
    u.frame(vec![]);
    assert_eq!(
        listed(&u)["families"],
        json!([{"family": "FigCraft Missing Download", "layers": 2, "source": null}, {"family": store, "layers": 1, "source": "Google Fonts"}])
    );
    let shown = crate::import_report_tests::shown_texts(u.frame_output(vec![]).shapes);
    assert_eq!(shown.iter().filter(|t| *t == "Download").count(), 1, "only the family the store has: {shown:?}");

    click_button(&mut u, "Download");
    assert_eq!(listed(&u)["consent"], store, "downloading is off: the user is asked first");
    let shown = crate::import_report_tests::shown_texts(u.frame_output(vec![]).shapes);
    assert!(shown.iter().any(|t| t == "Allow downloading fonts?"), "{shown:?}");
    assert!(figcraft_engine::fonts::downloads().is_empty(), "nothing downloads before the user allows it");

    click_button(&mut u, "Not now");
    assert_eq!(listed(&u)["consent"], Value::Null);
    assert!(!figcraft_engine::fonts::downloads_allowed() && figcraft_engine::fonts::downloads().is_empty());
}
