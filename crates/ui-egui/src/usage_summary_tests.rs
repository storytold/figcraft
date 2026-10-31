//! Show usage's period line sums the list the Type choice shows, so it never reads 0 above a list
//! with counts, and a server that doesn't count a kind says so for that kind alone.

use serde_json::json;

use crate::panels::assets_panel::period_totals;

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
