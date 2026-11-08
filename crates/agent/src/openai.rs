//! OpenAI-compatible endpoints (`/chat/completions` with function calling, `/models`): Ollama,
//! LM Studio, OpenRouter, vLLM and others. The loop calls the model, runs the tools it asks for
//! through [`crate::Guarded`], gives back the results and repeats until it answers without tools.

use std::sync::atomic::Ordering;
use std::time::Duration;

use figcraft_mcp::{Backend, call_tool, tool_definitions};
use serde_json::{Map, Value, json};
use ureq::http::Uri;

use crate::{Cancel, Event, TOOLS, clip, system_prompt, tool_detail};

/// How many times one run may call the model.
const MAX_TURNS: usize = 40;
/// How many tools one run may call.
const MAX_TOOL_CALLS: usize = 400;
/// Most text of one tool result sent back: the document's JSON can be huge.
const MAX_TOOL_TEXT: usize = 64 * 1024;
const MAX_RESPONSE: u64 = 16 * 1024 * 1024;
const MAX_ERROR_BODY: usize = 800;
/// How much of a chat is sent again with each message: older exchanges are forgotten.
const MAX_HISTORY: usize = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(300);

/// One endpoint and model.
#[derive(Clone, Debug)]
pub struct Config {
    pub url: String,
    pub api_key: Option<String>,
    pub model: String,
    /// The model takes images: it gets the screenshot tool, and its pictures as a message.
    pub vision: bool,
}

/// The endpoint's base URL without a trailing `/`, or why it can't be used. A key and a design must
/// not travel in the clear, so plain `http` is only for this computer.
pub fn check_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    let uri: Uri = url.parse().map_err(|_| format!("`{url}` is not a URL"))?;
    let host = uri.host().ok_or_else(|| format!("`{url}` has no host"))?;
    if uri.query().is_some() || uri.authority().is_some_and(|a| a.as_str().contains('@')) {
        return Err("the URL can't carry a query or a password".into());
    }
    let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1");
    match uri.scheme_str() {
        Some("https") => Ok(url.to_string()),
        Some("http") if local => Ok(url.to_string()),
        Some("http") => Err("plain http is only allowed for this computer (localhost): use https".into()),
        _ => Err("the URL must start with https:// (or http:// for localhost)".into()),
    }
}

/// Where a run sends its requests (the tests give a scripted model).
pub trait Transport {
    fn post(&mut self, body: &Value) -> Result<Value, String>;
}

/// Requests over HTTP.
pub struct Http {
    url: String,
    api_key: Option<String>,
    agent: ureq::Agent,
}

impl Http {
    pub fn new(url: &str, api_key: Option<String>) -> Result<Self, String> {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            // A redirect could carry the key somewhere else.
            .max_redirects(0)
            .build()
            .new_agent();
        Ok(Self { url: check_url(url)?, api_key, agent })
    }

    fn finish(mut response: ureq::http::Response<ureq::Body>) -> Result<Value, String> {
        let status = response.status();
        let text = response.body_mut().with_config().limit(MAX_RESPONSE).read_to_string().map_err(|e| format!("reading the reply: {e}"))?;
        if !status.is_success() {
            return Err(format!("the endpoint answered {status}: {}", clip(&text, MAX_ERROR_BODY)));
        }
        serde_json::from_str(&text).map_err(|e| format!("the endpoint's reply isn't JSON: {e}"))
    }

    /// The models the endpoint offers.
    pub fn models(&self) -> Result<Vec<String>, String> {
        let mut request = self.agent.get(format!("{}/models", self.url));
        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        let reply = Self::finish(request.call().map_err(|e| format!("can't reach the endpoint: {e}"))?)?;
        Ok(model_ids(&reply))
    }
}

impl Transport for Http {
    fn post(&mut self, body: &Value) -> Result<Value, String> {
        let mut request = self.agent.post(format!("{}/chat/completions", self.url)).header("Content-Type", "application/json");
        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        Self::finish(request.send(body.to_string()).map_err(|e| format!("can't reach the endpoint: {e}"))?)
    }
}

/// The ids in a `/models` reply, sorted.
fn model_ids(reply: &Value) -> Vec<String> {
    let mut ids: Vec<String> = reply
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    ids.sort();
    ids
}

/// The allowed tools in the OpenAI function format.
pub fn tools(vision: bool) -> Vec<Value> {
    tool_definitions()
        .into_iter()
        .filter(|t| t.get("name").and_then(Value::as_str).is_some_and(|n| allowed(n, vision)))
        .map(|t| json!({"type": "function", "function": {"name": t["name"], "description": t["description"], "parameters": t["inputSchema"]}}))
        .collect()
}

fn allowed(name: &str, vision: bool) -> bool {
    TOOLS.contains(&name) && (vision || name != "screenshot")
}

/// Run one tool call. Returns the text for the model and the images it produced.
fn run_tool(backend: &mut dyn Backend, name: &str, arguments: &str, vision: bool) -> (String, Vec<String>, bool) {
    if !allowed(name, vision) {
        return (format!("`{name}` isn't available: use one of the tools listed"), Vec::new(), true);
    }
    // Some servers send no arguments for a tool without parameters.
    let parsed = if arguments.trim().is_empty() { Ok(json!({})) } else { serde_json::from_str::<Value>(arguments) };
    let args = match parsed {
        Ok(args) => args,
        Err(e) => return (format!("the arguments aren't JSON: {e}"), Vec::new(), true),
    };
    let result = call_tool(backend, name, &args);
    let text: Vec<&str> = result.content.iter().filter_map(|c| c.get("text").and_then(Value::as_str)).collect();
    let images = result
        .content
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("image"))
        .filter_map(|c| Some(format!("data:{};base64,{}", c.get("mimeType")?.as_str()?, c.get("data")?.as_str()?)))
        .collect();
    (clip(&text.join("\n"), MAX_TOOL_TEXT), images, result.is_error)
}

/// A message that only carries pictures: only the newest stays in the conversation.
fn is_picture(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("user") && message.get("content").is_some_and(Value::is_array)
}

/// A run that ended early can leave tool calls without their results, which endpoints refuse.
fn drop_unanswered_calls(messages: &mut Vec<Value>) {
    let Some(last) = messages.iter().rposition(|m| m.get("tool_calls").is_some()) else { return };
    let wanted = messages.get(last).and_then(|m| m.get("tool_calls")).and_then(Value::as_array).map_or(0, Vec::len);
    let answered = messages.iter().skip(last + 1).filter(|m| m.get("role").and_then(Value::as_str) == Some("tool")).count();
    if answered < wanted {
        messages.truncate(last);
    }
}

/// Forget the oldest exchanges of a long chat, whole, keeping the system prompt and the newest.
fn trim_history(messages: &mut Vec<Value>, max: usize) {
    let size = |messages: &[Value]| messages.iter().map(|m| m.to_string().len()).sum::<usize>();
    while size(messages) > max {
        let starts = |m: &Value| m.get("role").and_then(Value::as_str) == Some("user") && !is_picture(m);
        let Some(next) = messages.iter().skip(2).position(starts) else { return };
        messages.drain(1..next + 2);
    }
}

/// Send `prompt` after the chat in `history`, which keeps the exchange, reporting through `on`.
pub fn run(
    config: &Config,
    prompt: &str,
    history: &mut Vec<Value>,
    backend: &mut dyn Backend,
    transport: &mut dyn Transport,
    on: &mut dyn FnMut(Event),
    cancel: &Cancel,
) {
    if let Err(why) = drive(config, prompt, history, backend, transport, on, cancel) {
        on(Event::Error(why));
    }
    drop_unanswered_calls(history);
}

fn drive(
    config: &Config,
    prompt: &str,
    messages: &mut Vec<Value>,
    backend: &mut dyn Backend,
    transport: &mut dyn Transport,
    on: &mut dyn FnMut(Event),
    cancel: &Cancel,
) -> Result<(), String> {
    let tools = tools(config.vision);
    if messages.is_empty() {
        messages.push(json!({"role": "system", "content": system_prompt()}));
    }
    messages.retain(|m| !is_picture(m));
    trim_history(messages, MAX_HISTORY);
    messages.push(json!({"role": "user", "content": prompt}));
    let mut calls = 0;
    for _ in 0..MAX_TURNS {
        if cancel.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        let reply = transport.post(&json!({"model": config.model, "messages": messages, "tools": tools, "tool_choice": "auto", "stream": false}))?;
        if cancel.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        if let Some(usage) = reply.get("usage") {
            let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
            on(Event::Usage { input: count("prompt_tokens"), output: count("completion_tokens") });
        }
        let message = reply
            .pointer("/choices/0/message")
            .ok_or_else(|| format!("the endpoint's reply has no message: {}", clip(&reply.to_string(), MAX_ERROR_BODY)))?;
        if let Some(text) = message.get("content").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
            on(Event::Text(text.to_string()));
        }
        let wanted = message.get("tool_calls").and_then(Value::as_array).cloned().unwrap_or_default();
        if wanted.is_empty() {
            messages.push(json!({"role": "assistant", "content": message.get("content").cloned().unwrap_or(Value::Null)}));
            on(Event::Done);
            return Ok(());
        }
        messages.push(assistant_message(message, &wanted));
        // A picture is costly: only the newest stays in the conversation.
        messages.retain(|m| !is_picture(m));
        let mut images = Vec::new();
        for call in &wanted {
            calls += 1;
            if calls > MAX_TOOL_CALLS {
                return Err(format!("stopped after {MAX_TOOL_CALLS} tool calls"));
            }
            let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default();
            let arguments = call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or_default();
            on(Event::Tool(name.to_string(), tool_detail(name, &serde_json::from_str(arguments).unwrap_or(Value::Null))));
            let (text, pictures, failed) = run_tool(backend, name, arguments, config.vision);
            if failed {
                on(Event::ToolError(clip(&text, MAX_ERROR_BODY)));
            }
            images.extend(pictures);
            messages.push(json!({"role": "tool", "tool_call_id": call.get("id").cloned().unwrap_or(Value::Null), "content": text}));
        }
        // Tool messages are text only on many endpoints: pictures follow as a user message.
        if !images.is_empty() {
            let parts: Vec<Value> = images.iter().map(|url| json!({"type": "image_url", "image_url": {"url": url}})).collect();
            messages.push(json!({"role": "user", "content": parts}));
        }
    }
    Err(format!("stopped after {MAX_TURNS} turns"))
}

/// The assistant's message as sent back: only the fields every endpoint accepts.
fn assistant_message(message: &Value, tool_calls: &[Value]) -> Value {
    let mut out = Map::new();
    out.insert("role".into(), json!("assistant"));
    out.insert("content".into(), message.get("content").cloned().unwrap_or(Value::Null));
    out.insert("tool_calls".into(), Value::Array(tool_calls.to_vec()));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gives the replies in order and keeps every request.
    struct Script {
        replies: Vec<Value>,
        seen: Vec<Value>,
    }

    impl Script {
        fn new(replies: Vec<Value>) -> Self {
            Self { replies, seen: Vec::new() }
        }
    }

    impl Transport for Script {
        fn post(&mut self, body: &Value) -> Result<Value, String> {
            self.seen.push(body.clone());
            if self.replies.is_empty() { Err("no more replies".into()) } else { Ok(self.replies.remove(0)) }
        }
    }

    #[derive(Default)]
    struct Doc(Vec<String>);

    impl Backend for Doc {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.0.push(format!("{method} {}", params.get("command").and_then(Value::as_str).unwrap_or("")));
            Ok(json!({"id": 1}))
        }
        fn has_ui(&self) -> bool {
            false
        }
        fn describe(&self) -> String {
            "doc".into()
        }
    }

    fn config(vision: bool) -> Config {
        Config { url: "http://localhost:11434/v1".into(), api_key: None, model: "m".into(), vision }
    }

    fn text(t: &str) -> Value {
        json!({"choices": [{"message": {"role": "assistant", "content": t}}]})
    }

    fn call(name: &str, arguments: &str) -> Value {
        json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
            {"id": "c1", "type": "function", "function": {"name": name, "arguments": arguments}}
        ]}}]})
    }

    fn drive_script(replies: Vec<Value>, vision: bool) -> (Vec<Event>, Doc, Script) {
        let (mut doc, mut script, mut events) = (Doc::default(), Script::new(replies), Vec::new());
        run(&config(vision), "make a card", &mut Vec::new(), &mut doc, &mut script, &mut |e| events.push(e), &Cancel::default());
        (events, doc, script)
    }

    #[test]
    fn a_second_message_continues_the_chat_with_one_system_prompt() {
        let (mut doc, mut history, mut events) = (Doc::default(), Vec::new(), Vec::new());
        let mut script = Script::new(vec![call("undo", "{}"), text("Undone."), text("You asked me to undo.")]);
        for message in ["undo that", "what did I ask?"] {
            run(&config(false), message, &mut history, &mut doc, &mut script, &mut |e| events.push(e), &Cancel::default());
        }
        let sent = script.seen[2]["messages"].as_array().unwrap();
        let roles: Vec<&str> = sent.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool", "assistant", "user"]);
        assert_eq!(sent[1]["content"], "undo that");
        assert_eq!(sent[4]["content"], "Undone.", "the model's own answer stays in the chat");
        assert!(sent[0]["content"].as_str().unwrap().contains("like Figma"), "the system prompt comes before the person's first message");
        assert_eq!(history.last().unwrap()["content"], "You asked me to undo.");
    }

    #[test]
    fn tokens_are_reported_for_each_request() {
        let mut reply = text("ok");
        reply["usage"] = json!({"prompt_tokens": 11, "completion_tokens": 4});
        let (events, ..) = drive_script(vec![reply], false);
        assert_eq!(events, [Event::Usage { input: 11, output: 4 }, Event::Text("ok".into()), Event::Done]);
    }

    #[test]
    fn a_stopped_exchange_leaves_nothing_an_endpoint_would_refuse() {
        let mut history = vec![
            json!({"role": "system", "content": "s"}),
            json!({"role": "user", "content": "u"}),
            json!({"role": "assistant", "tool_calls": [{"id": "a"}, {"id": "b"}]}),
            json!({"role": "tool", "tool_call_id": "a", "content": "x"}),
        ];
        drop_unanswered_calls(&mut history);
        assert_eq!(history.len(), 2, "the half-answered calls go");
        history.extend([json!({"role": "assistant", "tool_calls": [{"id": "a"}]}), json!({"role": "tool", "tool_call_id": "a", "content": "x"})]);
        drop_unanswered_calls(&mut history);
        assert_eq!(history.len(), 4, "answered calls stay");
    }

    #[test]
    fn a_long_chat_forgets_its_oldest_exchanges_whole() {
        let long = "x".repeat(100);
        let mut history = vec![
            json!({"role": "system", "content": "s"}),
            json!({"role": "user", "content": long}),
            json!({"role": "assistant", "content": long}),
            json!({"role": "user", "content": "newest"}),
            json!({"role": "assistant", "content": "reply"}),
        ];
        trim_history(&mut history, 150);
        assert_eq!(history.iter().map(|m| m["role"].as_str().unwrap()).collect::<Vec<_>>(), ["system", "user", "assistant"]);
        assert_eq!(history[1]["content"], "newest");
        trim_history(&mut history, 1);
        assert_eq!(history.len(), 3, "the newest exchange always stays");
    }

    #[test]
    fn pictures_go_as_plain_messages_and_only_the_newest_stays() {
        let picture = json!({"role": "user", "content": [{"type": "image_url", "image_url": {"url": "data:image/png;base64,AA"}}]});
        let mut history = vec![json!({"role": "system", "content": "s"}), json!({"role": "user", "content": "u"}), picture];
        let (mut doc, mut script, mut events) = (Doc::default(), Script::new(vec![text("ok")]), Vec::new());
        run(&config(true), "next", &mut history, &mut doc, &mut script, &mut |e| events.push(e), &Cancel::default());
        let sent = script.seen[0]["messages"].as_array().unwrap();
        assert!(!sent.iter().any(is_picture), "an old picture isn't sent again");
    }

    #[test]
    fn a_tool_call_runs_and_its_result_goes_back() {
        let (events, doc, script) = drive_script(vec![call("run_command", r##"{"command":"variable.list"}"##), text("Done.")], false);
        assert_eq!(events, [Event::Tool("run_command".into(), "variable.list".into()), Event::Text("Done.".into()), Event::Done]);
        assert_eq!(doc.0, ["engine.execute variable.list"]);
        let messages = script.seen[1]["messages"].as_array().unwrap();
        let (assistant, tool) = (&messages[2], &messages[3]);
        assert_eq!(assistant["tool_calls"][0]["id"], "c1");
        assert_eq!((tool["role"].as_str(), tool["tool_call_id"].as_str()), (Some("tool"), Some("c1")));
    }

    #[test]
    fn tools_outside_the_list_never_run_and_the_model_is_told() {
        let (events, doc, script) = drive_script(vec![call("save_file", "{}"), text("ok")], false);
        assert!(doc.0.is_empty());
        assert!(matches!(&events[1], Event::ToolError(e) if e.contains("save_file")));
        assert!(script.seen[1]["messages"][3]["content"].as_str().unwrap().contains("isn't available"));
    }

    #[test]
    fn broken_arguments_are_told_to_the_model_not_a_crash() {
        let (events, doc, _) = drive_script(vec![call("run_command", "{not json"), text("ok")], false);
        assert!(doc.0.is_empty());
        assert!(matches!(&events[1], Event::ToolError(e) if e.contains("JSON")));
    }

    #[test]
    fn missing_arguments_count_as_empty() {
        let (events, doc, _) = drive_script(vec![call("undo", ""), text("ok")], false);
        assert_eq!(doc.0, ["engine.execute edit.undo"]);
        assert_eq!(events.last(), Some(&Event::Done));
    }

    #[test]
    fn the_screenshot_tool_needs_vision() {
        let names = |vision| tools(vision).iter().map(|t| t["function"]["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert!(names(true).contains(&"screenshot".to_string()));
        assert!(!names(false).contains(&"screenshot".to_string()));
        assert!(names(false).iter().all(|n| TOOLS.contains(&n.as_str())));
        let (events, doc, _) = drive_script(vec![call("screenshot", "{}"), text("ok")], false);
        assert!(doc.0.is_empty());
        assert!(matches!(&events[1], Event::ToolError(_)));
    }

    #[test]
    fn endless_tool_calls_stop_at_the_limit() {
        let (events, doc, _) = drive_script(vec![call("undo", "{}"); MAX_TURNS + 5], false);
        assert_eq!(doc.0.len(), MAX_TURNS);
        assert!(matches!(events.last(), Some(Event::Error(e)) if e.contains("turns")));
    }

    #[test]
    fn a_bad_reply_is_an_error_not_a_crash() {
        for reply in [json!({}), json!({"choices": []}), json!("text"), json!({"choices": [{"message": 3}]})] {
            let (events, ..) = drive_script(vec![reply.clone()], false);
            assert!(matches!(events.last(), Some(Event::Error(_)) | Some(Event::Done)), "{reply}");
        }
        let (events, ..) = drive_script(Vec::new(), false);
        assert_eq!(events, [Event::Error("no more replies".into())]);
    }

    #[test]
    fn stopping_ends_the_run_before_the_next_request() {
        let (mut doc, mut script, mut events) = (Doc::default(), Script::new(vec![text("late")]), Vec::new());
        let cancel = Cancel::default();
        cancel.store(true, Ordering::Relaxed);
        run(&config(false), "x", &mut Vec::new(), &mut doc, &mut script, &mut |e| events.push(e), &cancel);
        assert!(script.seen.is_empty());
        assert_eq!(events, [Event::Error("stopped".into())]);
    }

    #[test]
    fn urls_are_checked() {
        assert_eq!(check_url(" https://api.example.com/v1/ ").unwrap(), "https://api.example.com/v1");
        for ok in ["http://localhost:11434/v1", "http://127.0.0.1:1234/v1", "http://[::1]:8080/v1"] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.com/v1",
            "http://localhost.example.com/v1",
            "ftp://x/v1",
            "",
            "not a url",
            "https://u:p@x.example/v1",
            "https://x.example/v1?a=b",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn model_lists_read_only_what_they_understand() {
        assert_eq!(model_ids(&json!({"data": [{"id": "b"}, {"id": "a"}, {"x": 1}, 3]})), ["a", "b"]);
        assert!(model_ids(&json!({"data": "no"})).is_empty());
    }
}
