//! Layout extensions cannot overwrite unrelated or subsequently edited native properties.

use super::*;

#[test]
fn layout_stash_filters_keys_and_preserves_native_edits() {
    for (cross, content, expected_cross, expected_content) in
        [("MIN", "AUTO", "STRETCH", "SPACE_EVENLY"), ("CENTER", "SPACE_BETWEEN", "CENTER", "SPACE_BETWEEN")]
    {
        let stash = json!({
            "id": "injected", "type": "TEXT", "layoutMode": "VERTICAL",
            "layoutReverse": true, "counterAxisAlignItems": "STRETCH",
            "counterAxisAlignContent": "SPACE_EVENLY", "columnTracks": [{"value": 99}],
        });
        let node = Kv::Object(vec![
            ("stackMode", Kv::Enum("GRID")),
            ("stackCounterAlignItems", Kv::Str(cross.into())),
            ("stackCounterAlignContent", Kv::Str(content.into())),
            (
                "pluginData",
                Kv::Array(vec![Kv::Object(vec![
                    ("pluginID", Kv::Str("figcraft".into())),
                    ("key", Kv::Str("layout".into())),
                    ("value", Kv::Str(stash.to_string())),
                ])]),
            ),
        ]);
        let message = Kv::Object(vec![]);
        let mut mapper = Mapper::new(&message, 100);
        let mut mapped = Obj::new();
        mapper.auto_layout(&mut mapped, &View::of(&node));
        assert!(!mapped.contains_key("id") && !mapped.contains_key("type"));
        assert_eq!(mapped["layoutMode"], "GRID");
        assert_eq!(mapped["layoutReverse"], true);
        assert_eq!(mapped["counterAxisAlignItems"], expected_cross);
        assert_eq!(mapped["counterAxisAlignContent"], expected_content);
        assert!(!mapped.contains_key("columnTracks"), "stale tracks cannot replace a changed native count");
    }
}
