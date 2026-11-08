//! The one gate every model's tool call passes, whichever backend made it. A document can carry
//! hostile text that a model reads, so the commands a run may use exclude files, other documents,
//! plugins and the network, and the app also runs each call confined to no folder at all.

use std::sync::atomic::Ordering;

use figcraft_engine::plugins;
use figcraft_mcp::Backend;
use serde_json::{Value, json};

use crate::Cancel;

/// The control-channel key that confines a call to no folder (`figcraft_ui_egui::control`).
const SCOPE: &str = "agentScope";
/// The methods that take `document`: the rest would refuse it or act on the tab in front.
const DOCUMENT_METHODS: &[&str] = &["engine.execute", "document.inspect", "node.get", "ui.render"];
/// Commands plugins may not call that a person's prompt may: the model works in the open file's history.
const ALSO_ALLOWED: &[&str] = &["edit.undo", "edit.redo"];

/// Why `command` may not run for the Agent tab's model, if it may not.
fn command_refusal(command: &str, params: &Value, nested: bool) -> Option<String> {
    let refused = || Some(format!("`{command}` isn't available in the Agent tab: files, other documents, plugins and the network are off"));
    if command == "command.batch" {
        if nested {
            return refused();
        }
        return params.get("commands").and_then(Value::as_array).into_iter().flatten().find_map(|step| {
            let inner = step.get("command").and_then(Value::as_str).unwrap_or_default();
            command_refusal(inner, step.get("params").unwrap_or(&Value::Null), true)
        });
    }
    if ALSO_ALLOWED.contains(&command) {
        return None;
    }
    if command.starts_with("font.") || command.starts_with("agent.") || plugins::denied(command, params).is_some() {
        return refused();
    }
    None
}

/// Why a control-channel call may not run, if it may not.
fn refusal(method: &str, params: &Value) -> Option<String> {
    match method {
        "engine.commands" | "document.inspect" | "node.get" => None,
        // Its `path` writes a file: without one the picture comes back as data.
        "ui.render" if params.get("path").is_none() => None,
        "engine.execute" => {
            let Some(command) = params.get("command").and_then(Value::as_str) else { return Some("`command` required".into()) };
            command_refusal(command, params.get("params").unwrap_or(&Value::Null), false)
        }
        _ => Some(format!("`{method}` isn't available in the Agent tab")),
    }
}

/// `inner` with every call checked, confined, and pinned to the file the prompt was given for.
pub struct Guarded<B> {
    inner: B,
    document: Option<u64>,
    agent: String,
    cancel: Cancel,
}

impl<B: Backend> Guarded<B> {
    /// `document` is a `doc.list` id: the run stays on that file when the user changes tabs.
    pub fn new(inner: B, document: Option<u64>, agent: impl Into<String>, cancel: Cancel) -> Self {
        Self { inner, document, agent: agent.into(), cancel }
    }
}

impl<B: Backend> Backend for Guarded<B> {
    fn call(&mut self, method: &str, mut params: Value) -> Result<Value, String> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err("the run was stopped".into());
        }
        if let Some(why) = refusal(method, &params) {
            return Err(why);
        }
        let Some(o) = params.as_object_mut() else { return Err("params must be an object".into()) };
        o.insert(SCOPE.into(), json!(true));
        if method == "engine.execute" {
            o.entry("agent").or_insert_with(|| json!(self.agent));
        }
        if let (Some(document), true) = (self.document, DOCUMENT_METHODS.contains(&method)) {
            o.insert("document".into(), json!(document));
        }
        let result = self.inner.call(method, params)?;
        // A model is only told about what it can use.
        Ok(match (method, result) {
            ("engine.commands", Value::Array(all)) => Value::Array(
                all.into_iter()
                    .filter(|c| {
                        c.get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| id == "command.batch" || command_refusal(id, &Value::Null, false).is_none())
                    })
                    .collect(),
            ),
            (_, other) => other,
        })
    }

    fn has_ui(&self) -> bool {
        // A screenshot is rendered from the document, not captured from the window.
        false
    }

    fn describe(&self) -> String {
        format!("{} (Agent tab)", self.inner.describe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recording(Vec<(String, Value)>);

    impl Backend for &mut Recording {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.0.push((method.to_string(), params));
            Ok(json!([{"id": "shape.rectangle"}, {"id": "file.open"}, {"id": "command.batch"}, {"id": "collab.host"}]))
        }
        fn has_ui(&self) -> bool {
            true
        }
        fn describe(&self) -> String {
            "test".into()
        }
    }

    fn guard(inner: &mut Recording, document: Option<u64>) -> Guarded<&mut Recording> {
        Guarded::new(inner, document, "Model", Cancel::default())
    }

    fn execute(command: &str, params: Value) -> Result<Value, String> {
        guard(&mut Recording::default(), None).call("engine.execute", json!({"command": command, "params": params}))
    }

    #[test]
    fn commands_that_reach_files_or_the_network_are_refused() {
        for command in [
            "file.open",
            "file.export",
            "collab.host",
            "plugin.run",
            "app.quit",
            "library.add",
            "font.download",
            "agent.prompt",
            "doc.switch",
            "data.fill",
            "window.new",
        ] {
            assert!(execute(command, json!({})).is_err(), "{command}");
        }
        for (command, params) in [
            ("image.place", json!({"path": "/etc/passwd"})),
            ("variable.export", json!({"path": "/tmp/x"})),
            ("codeConnect.import", json!({"path": "/x"})),
        ] {
            assert!(execute(command, params).is_err(), "{command}");
        }
    }

    #[test]
    fn design_commands_and_the_history_run() {
        for command in ["shape.rectangle", "node.set", "variable.list", "style.list", "component.list", "edit.undo", "edit.redo", "doc.list"] {
            assert!(execute(command, json!({})).is_ok(), "{command}");
        }
    }

    #[test]
    fn a_batch_cant_carry_a_refused_command() {
        let bad = json!({"commands": [{"command": "shape.rectangle"}, {"command": "file.save", "params": {"path": "/x"}}]});
        assert!(execute("command.batch", bad).is_err());
        let nested = json!({"commands": [{"command": "command.batch", "params": {"commands": [{"command": "file.open"}]}}]});
        assert!(execute("command.batch", nested).is_err());
        assert!(execute("command.batch", json!({"commands": [{"command": "shape.rectangle"}]})).is_ok());
    }

    #[test]
    fn methods_outside_the_document_are_refused() {
        for method in ["ui.key", "ui.pointer", "ui.input", "ui.menu.invoke", "ui.set", "ui.screenshot", "window.new", "app.quit", "events.poll"] {
            assert!(guard(&mut Recording::default(), None).call(method, json!({})).is_err(), "{method}");
        }
        assert!(guard(&mut Recording::default(), None).call("ui.render", json!({"path": "/tmp/x.png"})).is_err());
        assert!(guard(&mut Recording::default(), None).call("ui.render", json!({"scale": 1})).is_ok());
    }

    #[test]
    fn calls_are_confined_signed_and_pinned_to_the_file() {
        let mut seen = Recording::default();
        guard(&mut seen, Some(7)).call("engine.execute", json!({"command": "shape.rectangle"})).unwrap();
        let (_, p) = &seen.0[0];
        assert_eq!((&p["agentScope"], &p["agent"], &p["document"]), (&json!(true), &json!("Model"), &json!(7)));
    }

    #[test]
    fn the_command_list_only_shows_what_a_model_can_use() {
        let listed = guard(&mut Recording::default(), None).call("engine.commands", json!({})).unwrap();
        assert_eq!(listed, json!([{"id": "shape.rectangle"}, {"id": "command.batch"}]));
    }

    #[test]
    fn a_stopped_run_makes_no_more_calls() {
        let mut seen = Recording::default();
        let cancel = Cancel::default();
        cancel.store(true, Ordering::Relaxed);
        assert!(Guarded::new(&mut seen, None, "Model", cancel).call("node.get", json!({"id": 1})).is_err());
        assert!(seen.0.is_empty());
    }
}
