//! `component_mappings` and `sync_mappings` against a code repository's connect file.

use serde_json::{Value, json};

use crate::common::Client;

fn repo(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-mcp-connect-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

#[test]
fn mappings_travel_through_a_repositorys_file() {
    let dir = repo("travel");
    let mut c = Client::new();
    let frame = c.create(json!({"type": "frame", "name": "Tag", "width": 80, "height": 24}));
    let main = c.run("component.create", json!({"ids": [frame]}))["id"].clone();
    let file = dir.join("figcraft.connect.json");
    std::fs::write(&file, r#"{"version":1,"project":{"name":"shop"},"links":[{"component":"Tag","language":"react","importPath":"@/ui/tag","name":"Tag","template":"<Tag />","source":"src/tag.tsx"}]}"#).unwrap();
    let before = c.ok("component_mappings", json!({"unmapped": true, "root": dir}));
    assert_eq!(before["unmapped"], 1);
    c.ok("sync_mappings", json!({"direction": "import", "path": file}));
    let after = c.ok("component_mappings", json!({"ids": [main], "root": dir}));
    let link = &after["components"][0]["links"][0];
    assert_eq!(link["import"], r#"import { Tag } from "@/ui/tag";"#);
    assert_eq!(link["sourceFound"], false, "src/tag.tsx isn't there yet");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/tag.tsx"), "").unwrap();
    assert_eq!(c.ok("component_mappings", json!({"ids": [main], "root": dir}))["components"][0]["links"][0]["sourceFound"], true);
    c.ok("sync_mappings", json!({"direction": "export", "path": file}));
    let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(written["project"]["name"], "shop", "keys the export doesn't own stay");
    assert_eq!(written["links"][0]["source"], "src/tag.tsx");
    assert!(c.err("sync_mappings", json!({"direction": "sideways"})).contains("import or export"));
}
