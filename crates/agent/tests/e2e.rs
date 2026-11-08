//! Both backends against a real engine session: a real HTTP exchange for the endpoint and, when
//! asked for with `--ignored`, the real Claude Code CLI with the real MCP child process.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::Ordering;

use figcraft_agent::{Cancel, Event, Guarded, bridge, claude, openai};
use figcraft_mcp::{Backend, Headless};
use serde_json::{Value, json};

/// The engine as the app answers: the app removes the scope key before a call runs.
struct Engine<'a>(&'a mut Headless);

impl Backend for Engine<'_> {
    fn call(&mut self, method: &str, mut params: Value) -> Result<Value, String> {
        if let Some(o) = params.as_object_mut() {
            o.remove("agentScope");
        }
        self.0.call(method, params)
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "engine".into()
    }
}

fn layer_names(session: &mut Headless) -> String {
    session.call("document.inspect", json!({"depth": 8})).unwrap().to_string()
}

/// Answers `replies` in order on `listener`, and gives back each request's first line, headers and body.
fn serve_http(listener: TcpListener, replies: Vec<Value>) -> std::thread::JoinHandle<Vec<String>> {
    std::thread::spawn(move || {
        let mut seen = Vec::new();
        for reply in replies {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut request, mut length) = (String::new(), 0);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().unwrap();
                }
                let end = line == "\r\n";
                request.push_str(&line);
                if end {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            request.push_str(&String::from_utf8(body).unwrap());
            seen.push(request);
            let text = reply.to_string();
            write!(&stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len())
                .unwrap();
        }
        seen
    })
}

#[test]
fn an_endpoint_over_http_edits_a_real_document_and_cant_reach_a_file() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
    let tool = |id: &str, name: &str, arguments: Value| json!({"id": id, "type": "function", "function": {"name": name, "arguments": arguments.to_string()}});
    let server = serve_http(
        listener,
        vec![
            json!({"data": [{"id": "test-model"}]}),
            json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
                tool("a", "create_node", json!({"type": "rectangle", "name": "Made by the model", "x": 1, "y": 2, "width": 30, "height": 40})),
                tool("b", "run_command", json!({"command": "file.save", "params": {"path": "/tmp/figcraft-must-not-exist.figcraft"}})),
            ]}}]}),
            json!({"choices": [{"message": {"role": "assistant", "content": "Added it."}}]}),
        ],
    );
    let mut http = openai::Http::new(&url, Some("the-key".into())).unwrap();
    assert_eq!(http.models().unwrap(), ["test-model"]);
    let mut session = Headless::new();
    let cancel = Cancel::default();
    let mut events = Vec::new();
    {
        let mut guarded = Guarded::new(Engine(&mut session), None, "test-model", cancel.clone());
        let config = openai::Config { url, api_key: None, model: "test-model".into(), vision: false };
        openai::run(&config, "make a rectangle", &mut Vec::new(), &mut guarded, &mut http, &mut |e| events.push(e), &cancel);
    }
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /v1/models"), "{}", requests[0]);
    assert!(requests[1].starts_with("POST /v1/chat/completions"), "{}", requests[1]);
    assert!(requests[1].to_lowercase().contains("authorization: bearer the-key"));
    assert!(requests[1].contains("\"tools\"") && !requests[1].contains("save_file"));
    assert!(requests[1].contains("like Figma"), "the system prompt goes with the message");
    assert!(requests[2].contains("isn't available in the Agent tab"), "the model hears why the file call failed");
    assert!(layer_names(&mut session).contains("Made by the model"));
    assert!(!std::path::Path::new("/tmp/figcraft-must-not-exist.figcraft").exists());
    assert_eq!(events.last(), Some(&Event::Done), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::ToolError(why) if why.contains("file.save"))), "{events:?}");
}

/// One message through the real CLI: its events, and the layers of a fresh document afterwards.
fn claude_turn(prompt: &str, resume: Option<String>) -> (Vec<Event>, String) {
    let mcp_command = std::env::var_os("FIGCRAFT_AGENT_MCP").expect("FIGCRAFT_AGENT_MCP names the figcraft program").into();
    let program = claude::detect().expect("a claude program");
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let cancel = Cancel::default();
    let config = claude::Config {
        program,
        model: "haiku".into(),
        mcp_command,
        mcp_args: vec!["--agent-mcp".into()],
        bridge: format!("{port}:the-key"),
        resume,
    };
    let mut events = Vec::new();
    let names = std::thread::scope(|scope| {
        let serving = scope.spawn(|| {
            let mut session = Headless::new();
            {
                let mut guarded = Guarded::new(Engine(&mut session), None, "Claude Code", cancel.clone());
                bridge::serve(&listener, "the-key", &mut guarded, &cancel).unwrap();
            }
            layer_names(&mut session)
        });
        claude::run(&config, prompt, &mut |e| events.push(e), &cancel);
        cancel.store(true, Ordering::Relaxed);
        serving.join().unwrap()
    });
    (events, names)
}

fn replies(events: &[Event]) -> String {
    events.iter().filter_map(|e| if let Event::Text(t) = e { Some(t.as_str()) } else { None }).collect::<Vec<_>>().join(" ")
}

/// `FIGCRAFT_AGENT_MCP=$PWD/target/debug/figcraft cargo test -p figcraft-agent --test e2e -- --ignored --nocapture`
/// This uses the person's Claude Code login and a little of its quota.
#[test]
#[ignore = "calls the real Claude Code CLI"]
fn the_real_claude_cli_chats_and_edits_through_the_real_mcp_child() {
    let models = claude::list_models(&claude::detect().expect("a claude program")).unwrap();
    println!("models: {:?}", models.iter().map(|m| (&m.value, &m.label)).collect::<Vec<_>>());
    assert!(models.iter().any(|m| m.value == "haiku"), "{models:?}");

    let first = "Use create_node once to add a rectangle named Made by Claude at x 10, y 10, 100 wide and 50 high. Then try run_command with command file.save and params {\"path\": \"/tmp/figcraft-must-not-exist.figcraft\"} once. Then name the app you are designing in and stop.";
    let (events, names) = claude_turn(first, None);
    println!("first: {events:#?}");
    assert!(names.contains("Made by Claude"), "{names}\n{events:?}");
    assert!(!std::path::Path::new("/tmp/figcraft-must-not-exist.figcraft").exists());
    assert_eq!(events.last(), Some(&Event::Done));
    assert!(replies(&events).contains("FigCraft"), "the system prompt told it where it is: {events:?}");
    assert!(events.iter().any(|e| matches!(e, Event::Usage { input, output } if *input > 0 && *output > 0)), "tokens are reported: {events:?}");
    let session = events.iter().find_map(|e| if let Event::Session(id) = e { Some(id.clone()) } else { None }).expect("a session");

    let (events, _) = claude_turn("What was the name of the rectangle you just added? Answer with the name only, and use no tools.", Some(session));
    println!("second: {events:#?}");
    assert!(replies(&events).contains("Made by Claude"), "the chat goes on: {events:?}");

    let selected = "How wide is this? Answer with the number only, and use no tools.\n\n<selection count=\"1\" ids=\"7\">\n{\"id\":7,\"name\":\"Hero\",\"type\":\"FRAME\",\"width\":321,\"height\":90}\n</selection>";
    let (events, _) = claude_turn(selected, None);
    println!("selection: {events:#?}");
    assert!(replies(&events).contains("321"), "\"this\" means the selection: {events:?}");
}
