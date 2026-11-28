//! Show usage's period line sums the list the Type choice shows, so it never reads 0 above a list
//! with counts, and a server that doesn't count a kind says so for that kind alone. The answer draws
//! its totals, its period and the rows of the list chosen.

use serde_json::json;

use crate::live_tests::{flat, has_text};
use crate::panels::assets_panel::{period_totals, usage_answer};

#[test]
fn the_period_line_sums_the_list_shown() {
    let v = json!({
        "actionsKnown": true,
        "assetActionsKnown": true,
        "insertions": 0,
        "detachments": 0,
        "components": [],
        "styles": [{"insertions": 3, "detachments": 1}, {"insertions": 2, "detachments": 0}],
        "variables": [{"insertions": 1, "detachments": 4}],
    });
    assert_eq!(period_totals(&v, "components"), Some((0, 0)));
    assert_eq!(period_totals(&v, "styles"), Some((5, 1)));
    assert_eq!(period_totals(&v, "variables"), Some((1, 4)));
}

#[test]
fn a_server_that_doesnt_count_a_kind_has_no_totals_for_it() {
    let v = json!({"actionsKnown": true, "components": [{"insertions": 2, "detachments": 1}], "styles": [{"layers": 3}]});
    assert_eq!(period_totals(&v, "components"), Some((2, 1)));
    assert_eq!(period_totals(&v, "styles"), None, "a server from before them");
    assert_eq!(period_totals(&json!({}), "components"), None);
}

/// The texts Show usage draws for the answer `v` with the Type choice at `kind` (0 components,
/// 1 styles, 2 variables).
fn drawn(v: &serde_json::Value, kind: usize) -> Vec<egui::Shape> {
    let ctx = egui::Context::default();
    let id = egui::Id::unique("usage_summary_tests");
    ctx.data_mut(|d| d.insert_temp(id.with("kind"), kind));
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| usage_answer(ui, 600.0, id, v));
    out.textures_delta.clear();
    flat(&out.shapes)
}

#[test]
fn show_usage_draws_the_totals_the_period_and_the_list_chosen() {
    let v = json!({
        "days": 90,
        "files": [{}, {}],
        "instances": 5,
        "detachesKnown": true,
        "detached": 3,
        "actionsKnown": true,
        "assetActionsKnown": true,
        "assetsKnown": true,
        "components": [
            {"name": "Button", "instances": 4, "files": 2, "detached": 2, "insertions": 6, "detachments": 2},
            {"name": "Gone", "instances": 0, "files": 0, "detached": 3},
        ],
        "styles": [{"name": "Brand", "layers": 7, "files": 3, "insertions": 2, "detachments": 0}],
        "variables": [],
    });
    let components = drawn(&v, 0);
    for text in [
        "Used in 2 files · 5 instances · 3 detached",
        "Past 90 days: 6 inserted · 2 detached",
        "4 instances in 2 files · 2 detached · 6 inserted",
        "3 detached",
    ] {
        assert!(has_text(&components, text), "{text}");
    }
    let styles = drawn(&v, 1);
    assert!(has_text(&styles, "Past 90 days: 2 inserted · 0 detached"));
    assert!(has_text(&styles, "7 layers in 3 files · 2 inserted"));
    assert!(has_text(&drawn(&v, 2), "Nothing counted yet"), "an empty list says so");
}

#[test]
fn show_usage_from_an_older_server_says_what_it_doesnt_count() {
    let shapes = drawn(&json!({"files": [{}, {}], "instances": 5, "components": []}), 0);
    for text in [
        "Used in 2 files · 5 instances",
        "This server doesn't count detached instances",
        "This server doesn't count styles and variables",
        "This server doesn't count insertions",
    ] {
        assert!(has_text(&shapes, text), "{text}");
    }
}
