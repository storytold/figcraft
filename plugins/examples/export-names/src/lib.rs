//! Export names: saves the names of the selected layers (optionally the layers inside them too) as
//! the text file `layer-names.txt`, through the host's own Save dialog (`save_file`).

use figcraft_plugin_sdk::{Input, Result, Value, call, entry, json, notify, save_file};

/// Layers looked at in one run, at most.
const MAX_LAYERS: usize = 10_000;

/// The names as the file's text: one per line, a line break at the end, line breaks inside a name
/// turned into spaces.
fn names_text(names: &[String]) -> String {
    let mut text = String::new();
    for name in names {
        text.extend(name.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }));
        text.push('\n');
    }
    text
}

fn run(input: Input) -> Result<Value> {
    if input.selection.is_empty() {
        return Err("Select the layers to export first".into());
    }
    let inside = input.bool("children").unwrap_or(true);
    // Depth first, in layer order: the stack takes the children reversed.
    let mut stack: Vec<u64> = input.selection.iter().rev().copied().collect();
    let mut names = Vec::new();
    while let Some(id) = stack.pop() {
        if names.len() >= MAX_LAYERS {
            break;
        }
        let node = call("node.get", &json!({"id": id}))?;
        names.push(node["name"].as_str().unwrap_or_default().to_string());
        if inside && let Some(children) = node["children"].as_array() {
            stack.extend(children.iter().rev().filter_map(|c| c["id"].as_u64()));
        }
    }
    save_file("layer-names.txt", "text/plain", names_text(&names).as_bytes())?;
    notify(&format!("Exporting {} names", names.len()));
    Ok(json!({"names": names.len()}))
}

entry!(run);

#[cfg(test)]
mod tests {
    use super::names_text;

    #[test]
    fn names_become_lines() {
        let names = ["Card".to_string(), "Title\nline".to_string(), String::new()];
        assert_eq!(names_text(&names), "Card\nTitle line\n\n");
        assert_eq!(names_text(&[]), "");
    }
}
