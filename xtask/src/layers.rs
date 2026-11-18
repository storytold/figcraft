//! Crate layering: a crate may depend only on lower layers (or earlier crates of its own layer
//! listed in ORDER). Nothing below the UI depends on egui/eframe.

use std::collections::BTreeMap;
use std::process::Command;

/// (crate suffix, layer)
const TABLE: &[(&str, u8)] = &[
    ("geom", 0),
    ("color", 0),
    ("doc", 1),
    ("layout", 2),
    ("text", 2),
    ("pathops", 2),
    ("collab", 2),
    ("plugin", 2),
    ("render", 3),
    ("format", 3),
    ("svg", 4),
    ("pdf", 4),
    ("vectordrawable", 4),
    ("engine", 5),
    ("ui-egui", 6),
    ("mcp", 6),
    // Live sessions over the network (WebSockets): above the engine, beside the UI.
    ("collab-net", 6),
];

/// Networking crates: only L6 (`collab-net`) may use them.
const NETWORK: &[&str] = &["tungstenite", "tokio", "tokio-tungstenite", "mio", "socket2"];

pub fn check(root: &std::path::Path) -> bool {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output();
    let Ok(out) = out else { return false };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return false };
    let layer: BTreeMap<String, u8> = TABLE.iter().map(|(n, l)| (format!("figcraft-{n}"), *l)).collect();
    let mut ok = true;
    for pkg in v["packages"].as_array().cloned().unwrap_or_default() {
        let name = pkg["name"].as_str().unwrap_or("").to_string();
        let Some(&l) = layer.get(&name) else { continue };
        for dep in pkg["dependencies"].as_array().cloned().unwrap_or_default() {
            let dn = dep["name"].as_str().unwrap_or("");
            if dep["kind"].as_str() == Some("dev") {
                continue;
            }
            if l < 6 && (dn == "egui" || dn == "eframe" || dn == "rfd" || dn == "winit") {
                eprintln!("layers: {name} (L{l}) depends on UI crate {dn}");
                ok = false;
            }
            if l < 6 && NETWORK.contains(&dn) {
                eprintln!("layers: {name} (L{l}) depends on networking crate {dn}");
                ok = false;
            }
            if let Some(&dl) = layer.get(dn)
                && dl >= l
                && !(dl == l && same_layer_ok(&name, dn))
            {
                eprintln!("layers: {name} (L{l}) may not depend on {dn} (L{dl})");
                ok = false;
            }
        }
    }
    if ok {
        eprintln!("layers: ok");
    }
    ok
}

fn same_layer_ok(from: &str, to: &str) -> bool {
    // Within L0: color uses geom types; within L2: text/layout/pathops/collab are independent;
    // within L6 the MCP server joins live sessions through collab-net.
    matches!((from, to), ("figcraft-color", "figcraft-geom") | ("figcraft-render", "figcraft-format") | ("figcraft-mcp", "figcraft-collab-net"))
}
